package sdk

import (
	"bufio"
	"errors"
	"io"
	"reflect"
	"strings"
	"unicode/utf8"
)

// Stream is a finite, pull-based, context-owned item iterator. Next/Value/Err
// have a single consumer. Close is idempotent and may interrupt a blocked Next.
// Always defer Close when stopping before EOF. No background read or reconnect occurs.
type Stream[T any] struct {
	raw                 *rawHTTPResponse
	media               httpMediaPlan
	codec               Codec[T]
	reader              *bufio.Reader
	value               T
	err                 error
	done, skipLF, first bool
	itemLimit           int
}
type httpStreamReader struct {
	raw   *rawHTTPResponse
	count int
}

func (r *httpStreamReader) Read(p []byte) (int, error) {
	if err := r.raw.owned.ctx.Err(); err != nil {
		return 0, err
	}
	remaining := r.raw.maximum - r.count
	if len(p) > remaining+1 {
		p = p[:remaining+1]
	}
	n, err := r.raw.owned.body.Read(p)
	r.count += n
	capture := r.raw.maxCapture - len(r.raw.body)
	if capture > n {
		capture = n
	}
	if capture > 0 {
		r.raw.body = append(r.raw.body, p[:capture]...)
	}
	if r.count > r.raw.maximum {
		return 0, httpProblem("resource-limit")
	}
	return n, httpCause(r.raw.owned.ctx, err)
}
func httpNewStream[T any](raw *rawHTTPResponse, m httpMediaPlan, codec Codec[T]) *Stream[T] {
	limit := m.MaxItemBytes
	if raw.itemLimit < limit {
		limit = raw.itemLimit
	}
	raw.taken = true
	return &Stream[T]{raw: raw, media: m, codec: codec, reader: bufio.NewReaderSize(&httpStreamReader{raw: raw}, 4096), first: true, itemLimit: limit}
}
func (s *Stream[T]) Value() T {
	if s == nil {
		var zero T
		return zero
	}
	return s.value
}
func (s *Stream[T]) Err() error {
	if s == nil {
		return nil
	}
	return s.err
}
func (s *Stream[T]) Close() error {
	if s == nil || s.raw == nil {
		return nil
	}
	return s.raw.owned.finish()
}
func (s *Stream[T]) finish(err error) bool {
	s.done = true
	var zero T
	s.value = zero
	if err != nil && err != io.EOF {
		s.err = s.raw.fail("response-decoding", s.media.Source, err)
		s.err.(*SDKError).Truncated = true
	}
	_ = s.Close()
	return false
}
func (s *Stream[T]) line() (string, bool, error) {
	var data []byte
	for {
		b, err := s.reader.ReadByte()
		if err == io.EOF {
			return string(data), true, nil
		}
		if err != nil {
			return "", false, err
		}
		if s.skipLF {
			s.skipLF = false
			if b == '\n' {
				continue
			}
		}
		if b == '\r' && s.media.Framing != "json-lines" {
			s.skipLF = true
			return string(data), false, nil
		}
		if b == '\n' {
			return string(data), false, nil
		}
		if len(data) >= s.itemLimit {
			return "", false, httpProblem("resource-limit")
		}
		data = append(data, b)
	}
}
func (s *Stream[T]) Next() bool {
	if s == nil || s.raw == nil || s.done {
		return false
	}
	if err := s.raw.owned.ctx.Err(); err != nil {
		return s.finish(err)
	}
	if s.media.Framing == "json-lines" {
		line, eof, err := s.line()
		if err != nil {
			return s.finish(err)
		}
		if eof && line == "" {
			return s.finish(io.EOF)
		}
		value, err := s.codec.Decode([]byte(line))
		if err != nil {
			return s.finish(err)
		}
		s.value = value
		return true
	}
	event := map[string]Value{}
	var data strings.Builder
	hasData := false
	used := 0
	for {
		line, eof, err := s.line()
		if err != nil {
			return s.finish(err)
		}
		// HTML framing discards a final block without a terminating blank line.
		if eof {
			return s.finish(io.EOF)
		}
		line = httpSSEUTF8([]byte(line))
		if s.first {
			s.first = false
			line = strings.TrimPrefix(line, "\uFEFF")
		}
		used += len(line) + 1
		if used > s.itemLimit {
			return s.finish(httpProblem("resource-limit"))
		}
		if line == "" {
			if !hasData {
				event = map[string]Value{}
				used = 0
				continue
			}
			text := data.String()
			event["data"] = strings.TrimSuffix(text, "\n")
			value, err := s.codec.DecodeValue(event)
			if err != nil {
				return s.finish(err)
			}
			s.value = value
			return true
		}
		if line[0] == ':' {
			continue
		}
		name, value, _ := strings.Cut(line, ":")
		value = strings.TrimPrefix(value, " ")
		switch name {
		case "data":
			data.WriteString(value)
			data.WriteByte('\n')
			hasData = true
		case "event":
			event["event"] = value
		case "id":
			if !strings.ContainsRune(value, 0) {
				event["id"] = value
			}
		case "retry":
			if httpDigits(value) {
				integer, err := ParseInteger(value)
				if err != nil {
					// Leading zeros are valid SSE decimal digits but not JSON number tokens.
					normalized := strings.TrimLeft(value, "0")
					if normalized == "" {
						normalized = "0"
					}
					integer, err = ParseInteger(normalized)
				}
				if err != nil {
					return s.finish(err)
				}
				event["retry"] = integer
			}
		}
	}
}
func httpDigits(text string) bool {
	if text == "" {
		return false
	}
	for i := 0; i < len(text); i++ {
		if text[i] < '0' || text[i] > '9' {
			return false
		}
	}
	return true
}

// UTF-8 decoding with the HTML replacement-error policy, including incomplete
// multibyte sequences. Transport chunk boundaries do not affect decoded fields.
func httpSSEUTF8(data []byte) string {
	if utf8.Valid(data) {
		return string(data)
	}
	var out strings.Builder
	for i := 0; i < len(data); {
		b := data[i]
		if b < 128 {
			out.WriteByte(b)
			i++
			continue
		}
		needed := 0
		lo, hi := byte(0x80), byte(0xbf)
		switch {
		case b >= 0xc2 && b <= 0xdf:
			needed = 1
		case b >= 0xe0 && b <= 0xef:
			needed = 2
			if b == 0xe0 {
				lo = 0xa0
			}
			if b == 0xed {
				hi = 0x9f
			}
		case b >= 0xf0 && b <= 0xf4:
			needed = 3
			if b == 0xf0 {
				lo = 0x90
			}
			if b == 0xf4 {
				hi = 0x8f
			}
		}
		if needed == 0 {
			out.WriteRune(utf8.RuneError)
			i++
			continue
		}
		consumed := 1
		valid := true
		for n := 1; n <= needed; n++ {
			if i+n >= len(data) {
				valid = false
				break
			}
			c := data[i+n]
			min, max := byte(0x80), byte(0xbf)
			if n == 1 {
				min, max = lo, hi
			}
			if c < min || c > max {
				valid = false
				break
			}
			consumed++
		}
		if valid {
			out.Write(data[i : i+consumed])
		} else {
			out.WriteRune(utf8.RuneError)
		}
		i += consumed
	}
	return out.String()
}
func httpEncodeStream(m httpMediaPlan, value any, limit int) ([]byte, error) {
	items := reflect.ValueOf(value)
	if items.Kind() != reflect.Slice {
		return nil, errors.New("finite stream request requires a typed item slice")
	}
	buffer := &httpBuffer{limit: limit}
	for i := 0; i < items.Len(); i++ {
		v, err := httpCodecs[m.Codec].encode(items.Index(i).Interface())
		if err != nil {
			return nil, err
		}
		var data []byte
		if m.Framing == "json-lines" {
			data, err = httpJSONEncode(v, m.MaxItemBytes)
			if err == nil {
				data = append(data, '\n')
			}
		} else {
			event, ok := v.(map[string]Value)
			if !ok {
				return nil, errors.New("SSE item requires an event envelope")
			}
			frame := &httpBuffer{limit: m.MaxItemBytes}
			for name := range event {
				if name != "data" && name != "event" && name != "id" && name != "retry" {
					return nil, errors.New("SSE unknown fields cannot round trip")
				}
			}
			for _, name := range []string{"event", "id", "retry", "data"} {
				field, exists := event[name]
				if !exists {
					continue
				}
				text, e := httpScalar(field, "")
				if e != nil {
					return nil, e
				}
				if name == "retry" && !httpDigits(text) {
					return nil, errors.New("SSE retry requires nonnegative decimal digits")
				}
				if name == "id" && strings.ContainsRune(text, 0) {
					return nil, errors.New("SSE id must not contain NUL")
				}
				if name != "data" && strings.ContainsAny(text, "\r\n") {
					return nil, errors.New("SSE field contains a framing delimiter")
				}
				if strings.Contains(text, "\r") {
					return nil, errors.New("SSE data CR requires an API-defined representation")
				}
				for _, line := range strings.Split(text, "\n") {
					if _, e = frame.Write([]byte(name + ": " + line + "\n")); e != nil {
						return nil, e
					}
				}
			}
			if _, exists := event["data"]; !exists {
				return nil, errors.New("SSE request item requires data to dispatch")
			}
			if _, err = frame.Write([]byte("\n")); err != nil {
				return nil, err
			}
			data = frame.Bytes()
		}
		if err != nil {
			return nil, err
		}
		if len(data) > m.MaxItemBytes {
			return nil, httpProblem("resource-limit")
		}
		if _, err = buffer.Write(data); err != nil {
			return nil, err
		}
	}
	return buffer.Bytes(), nil
}
