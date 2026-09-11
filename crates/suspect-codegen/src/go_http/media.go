package sdk

import (
	"errors"
	"net/http"
	"strings"
	"unicode/utf8"
)

func httpSplitQuoted(value string, delimiter byte) ([]string, error) {
	var result []string
	start := 0
	quoted, escaped := false, false
	for i := 0; i < len(value); i++ {
		c := value[i]
		if c < 32 && c != '\t' || c == 127 {
			return nil, errors.New("control character in media type")
		}
		if escaped {
			escaped = false
		} else if quoted && c == '\\' {
			escaped = true
		} else if c == '"' {
			quoted = !quoted
		} else if !quoted && c == delimiter {
			result = append(result, value[start:i])
			start = i + 1
		}
	}
	if quoted || escaped {
		return nil, errors.New("unterminated media quote")
	}
	return append(result, value[start:]), nil
}
func httpParseMedia(value string, ranges bool) (httpMediaType, error) {
	result := httpMediaType{Declared: value, Parameters: map[string]string{}}
	pieces, err := httpSplitQuoted(value, ';')
	if err != nil {
		return result, err
	}
	kind, subtype, ok := strings.Cut(strings.TrimSpace(pieces[0]), "/")
	if !ok || !httpToken(kind) || !httpToken(subtype) {
		return result, errors.New("invalid media type")
	}
	kind, subtype = strings.ToLower(kind), strings.ToLower(subtype)
	result.Type = kind
	result.Subtype = subtype
	result.Rank = 2
	if kind == "*" && subtype == "*" && ranges {
		result.Rank = 0
	} else if subtype == "*" && !strings.Contains(kind, "*") && ranges {
		result.Rank = 1
	} else if strings.Contains(kind, "*") || strings.Contains(subtype, "*") {
		return result, errors.New("a concrete Content-Type is required")
	}
	for _, piece := range pieces[1:] {
		key, raw, ok := strings.Cut(strings.TrimSpace(piece), "=")
		key = strings.ToLower(key)
		if !ok || !httpToken(key) {
			return result, errors.New("invalid media parameter")
		}
		if _, exists := result.Parameters[key]; exists {
			return result, errors.New("duplicate media parameter")
		}
		text := raw
		if strings.HasPrefix(raw, "\"") {
			if len(raw) < 2 || !strings.HasSuffix(raw, "\"") {
				return result, errors.New("invalid media quote")
			}
			var out strings.Builder
			escaped := false
			for _, c := range raw[1 : len(raw)-1] {
				if escaped {
					out.WriteRune(c)
					escaped = false
				} else if c == '\\' {
					escaped = true
				} else if c == '"' {
					return result, errors.New("unescaped media quote")
				} else {
					out.WriteRune(c)
				}
			}
			if escaped {
				return result, errors.New("unfinished media escape")
			}
			text = out.String()
		} else if !httpToken(raw) {
			return result, errors.New("invalid media parameter value")
		}
		result.Parameters[key] = text
	}
	return result, nil
}
func (m httpMediaType) matches(actual httpMediaType) bool {
	if m.Type != "*" && m.Type != actual.Type || m.Subtype != "*" && m.Subtype != actual.Subtype {
		return false
	}
	for k, v := range m.Parameters {
		a, ok := actual.Parameters[k]
		if !ok {
			return false
		}
		if k == "charset" {
			if !strings.EqualFold(v, a) {
				return false
			}
		} else if v != a {
			return false
		}
	}
	return true
}
func httpSelectMedia(media []httpMediaPlan, value string) (int, error) {
	actual, err := httpParseMedia(value, false)
	if err != nil {
		return -1, err
	}
	selected := -1
	for i, m := range media {
		if m.Media.matches(actual) && (selected < 0 || m.Media.Rank > media[selected].Media.Rank || m.Media.Rank == media[selected].Media.Rank && len(m.Media.Parameters) > len(media[selected].Media.Parameters)) {
			selected = i
		}
	}
	if selected < 0 {
		return -1, errors.New("undeclared Content-Type")
	}
	kind := media[selected].Kind
	if charset, ok := actual.Parameters["charset"]; ok && (kind == "text" || kind == "stream" || kind == "form") && !strings.EqualFold(charset, "utf-8") {
		return -1, errors.New("unsupported charset")
	}
	return selected, nil
}
func (r *rawHTTPResponse) selectResponse(op httpOperation) error {
	r.forbidden = op.Method == "HEAD" || r.status >= 100 && r.status < 200 || r.status == 204 || r.status == 205 || r.status == 304
	if r.status < 100 || r.status > 599 {
		return errors.New("invalid HTTP status")
	}
	rank := 0
	for i, p := range op.Responses {
		candidate := 0
		if p.Status == "default" {
			candidate = 1
		} else if len(p.Status) == 3 && p.Status[1:] == "XX" && int(p.Status[0]-'0') == r.status/100 {
			candidate = 2
		} else if len(p.Status) == 3 && int(p.Status[0]-'0')*100+int(p.Status[1]-'0')*10+int(p.Status[2]-'0') == r.status {
			candidate = 3
		}
		if candidate > rank {
			rank = candidate
			r.response = i
		}
	}
	if r.response < 0 {
		return errors.New("undeclared HTTP status")
	}
	if r.forbidden || len(op.Responses[r.response].Media) == 0 {
		return nil
	}
	values := httpHeaderValues(r.headers, "Content-Type")
	if len(values) != 1 {
		return errors.New("one Content-Type is required")
	}
	r.contentType = values[0]
	var err error
	r.media, err = httpSelectMedia(op.Responses[r.response].Media, r.contentType)
	return err
}
func httpDecodeHeader(h httpHeaderPlan, headers http.Header, limit int) (any, bool, error) {
	values := httpHeaderValues(headers, h.Name)
	if len(values) == 0 {
		if h.Required {
			return nil, false, httpLocated(h.Source, errors.New("required response or part header is missing"))
		}
		return nil, false, nil
	}
	if strings.EqualFold(h.Name, "Set-Cookie") && len(values) > 1 {
		return nil, false, httpLocated(h.Source, errors.New("Set-Cookie fields cannot be combined into one typed header value"))
	}
	text := strings.Join(values, ",")
	value, err := httpParseHeader(text, h.Serial, limit)
	if err != nil {
		return nil, false, httpLocated(h.Source, err)
	}
	native, err := httpCodecs[h.Codec].decode(value)
	if err != nil {
		return nil, false, httpLocated(h.Source, err)
	}
	return native, true, nil
}
func httpLocated(at HTTPSource, cause error) error {
	var existing *SDKError
	if errors.As(cause, &existing) && existing.Source.Document != "" {
		return cause
	}
	return &SDKError{Kind: httpFailureKind("request-representation", cause), Source: at, Cause: cause}
}
func httpEncodeBody(op httpOperation, index int, contentType string, value any, partLimit int) (httpBody, error) {
	m := op.Body[index]
	if contentType == "" {
		if m.Media.Rank < 2 {
			return httpBody{}, errors.New("wildcard body requires a concrete Content-Type")
		}
		contentType = m.Media.Declared
	}
	selected, err := httpSelectMedia(op.Body, contentType)
	if err != nil {
		return httpBody{}, err
	}
	if selected != index {
		return httpBody{}, errors.New("Content-Type selects a different typed request representation")
	}
	if partLimit == 0 {
		partLimit = op.MaxPart
	}
	if partLimit > op.MaxPart {
		return httpBody{}, httpProblem("resource-limit")
	}
	data, contentType, err := httpEncodeMedia(m, value, contentType, op.MaxRequest, partLimit)
	if err != nil {
		return httpBody{}, err
	}
	if len(data) > op.MaxRequest {
		return httpBody{}, httpProblem("resource-limit")
	}
	return httpBody{data: data, contentType: contentType, present: true}, nil
}
func httpEncodeMedia(m httpMediaPlan, value any, contentType string, limit, partLimit int) ([]byte, string, error) {
	switch m.Kind {
	case "json", "text":
		var v Value
		var err error
		if m.Codec != "" {
			v, err = httpCodecs[m.Codec].encode(value)
		} else {
			v = value
		}
		if err != nil {
			return nil, contentType, err
		}
		if m.Kind == "json" {
			data, err := httpJSONEncode(v, limit)
			return data, contentType, err
		}
		text, err := httpScalar(v, m.Scalar)
		if err != nil {
			return nil, contentType, err
		}
		if !utf8.ValidString(text) {
			return nil, contentType, errors.New("invalid UTF-8 text")
		}
		if len(text) > limit {
			return nil, contentType, httpProblem("resource-limit")
		}
		return []byte(text), contentType, nil
	case "binary":
		data, ok := value.([]byte)
		if !ok {
			return nil, contentType, errors.New("byte body requires []byte")
		}
		if len(data) > limit || len(data) > m.MaxBytes {
			return nil, contentType, httpProblem("resource-limit")
		}
		return append([]byte(nil), data...), contentType, nil
	case "form", "multipart":
		return httpEncodeAggregate(m, value, contentType, limit, partLimit)
	case "stream":
		data, err := httpEncodeStream(m, value, limit)
		return data, contentType, err
	}
	return nil, contentType, errors.New("unsupported request representation")
}
func httpDecodeMedia(m httpMediaPlan, r *rawHTTPResponse) (any, error) {
	switch m.Kind {
	case "json", "text":
		var value Value
		var err error
		if m.Kind == "json" {
			value, err = httpJSONParse(r.body, r.maximum)
		} else {
			value, err = httpTextValue(string(r.body), m.Scalar)
		}
		if err != nil {
			return nil, err
		}
		if m.Codec != "" {
			return httpCodecs[m.Codec].decode(value)
		}
		return value, nil
	case "binary":
		if len(r.body) > m.MaxBytes {
			return nil, httpProblem("resource-limit")
		}
		return r.body, nil
	case "form", "multipart":
		return httpDecodeAggregate(m, r)
	}
	return nil, errors.New("unsupported buffered response representation")
}
