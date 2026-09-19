package sdk

import (
	"bytes"
	"errors"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"net/textproto"
	"net/url"
	"reflect"
	"sort"
	"strings"
	"unicode"
)

type httpPartInput struct {
	plan                  httpPartPlan
	name                  string
	value                 any
	contentType, filename string
	headers               http.Header
}

// A querystring form is an actual JSON object input. Its aggregate codec has
// already validated it; per-part codecs receive the real field/item values.
func httpEncodeQueryForm(a *httpAggregatePlan, value Value, limit, partLimit int) (string, error) {
	object, ok := value.(map[string]Value)
	if !ok {
		return "", errors.New("form querystring requires an object")
	}
	if len(object) > a.MaxParts {
		return "", httpProblem("resource-limit")
	}
	keys := make([]string, 0, len(object))
	for key := range object {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	buffer := &httpBuffer{limit: limit}
	separator := ""
	fieldCount := 0
	for _, name := range keys {
		var p *httpPartPlan
		for i := range a.Parts {
			if a.Parts[i].Name == name {
				p = &a.Parts[i]
				break
			}
		}
		if p == nil {
			p = a.Additional
		}
		if p == nil {
			return "", errors.New("undeclared querystring form field")
		}
		values := []Value{object[name]}
		if p.Repeated {
			var ok bool
			values, ok = object[name].([]Value)
			if !ok {
				return "", errors.New("repeated form field requires an array")
			}
			if err := httpCardinality(len(values), p.Min, p.Max); err != nil {
				return "", err
			}
			if len(values) == 0 {
				return "", errors.New("empty repeated querystring field cannot be serialized")
			}
		}
		if len(values) > a.MaxParts-fieldCount {
			return "", httpProblem("resource-limit")
		}
		for _, value := range values {
			native, err := httpCodecs[p.Codec].decode(value)
			if err != nil {
				return "", err
			}
			data, err := httpPartEncode(httpPartInput{plan: *p, name: name, value: native}, partLimit)
			if err != nil {
				return "", err
			}
			text := string(data)
			if p.Kind != "style" {
				key, err := httpPercent(name, "form", limit)
				if err != nil {
					return "", err
				}
				escaped, err := httpPercent(text, p.Encoding, limit)
				if err != nil {
					return "", err
				}
				text = key + "=" + escaped
			}
			if _, err = buffer.Write([]byte(separator + text)); err != nil {
				return "", err
			}
			separator = "&"
			fieldCount += strings.Count(text, "&") + 1
			if fieldCount > a.MaxParts {
				return "", httpProblem("resource-limit")
			}
		}
	}
	return buffer.String(), nil
}

type httpPartWire struct {
	name, rawName, rawValue, contentType, filename string
	body                                           []byte
	headers                                        http.Header
}
type httpBuffer struct {
	bytes.Buffer
	limit int
}

func (b *httpBuffer) Write(data []byte) (int, error) {
	if len(data) > b.limit-b.Len() {
		return 0, httpProblem("resource-limit")
	}
	return b.Buffer.Write(data)
}
func httpCardinality(count int, min, max *uint64) error {
	if min != nil && uint64(count) < *min || max != nil && uint64(count) > *max {
		return errors.New("aggregate cardinality constraint")
	}
	return nil
}
func httpField(v reflect.Value, required bool) (reflect.Value, bool, error) {
	if required {
		return v, true, nil
	}
	present := v.FieldByName("IsSet").Bool()
	value := v.FieldByName("Value")
	if !present && !value.IsZero() {
		return value, false, errors.New("inconsistent absent wrapper")
	}
	return value, present, nil
}
func httpPartInputs(a *httpAggregatePlan, value any) ([]httpPartInput, error) {
	native := reflect.ValueOf(value)
	if !native.IsValid() || native.Type() != httpAggregateTypes[a.Type] {
		return nil, errors.New("wrong aggregate native type")
	}
	var result []httpPartInput
	present := map[string]bool{}
	count := 0
	gap := false
	appendValue := func(p httpPartPlan, name string, v reflect.Value) error {
		count := 1
		if p.Repeated {
			count = v.Len()
		}
		if count > a.MaxParts-len(result) {
			return httpProblem("resource-limit")
		}
		values := []reflect.Value{v}
		if p.Repeated {
			if err := httpCardinality(v.Len(), p.Min, p.Max); err != nil {
				return err
			}
			if v.Len() == 0 {
				return errors.New("present repeated field has no wire representation; omit optional empty fields")
			}
			values = nil
			for i := 0; i < v.Len(); i++ {
				values = append(values, v.Index(i))
			}
		}
		for _, v := range values {
			input := httpPartInput{plan: p, name: name, value: v.Interface()}
			if a.Multipart {
				input.value = v.FieldByName("Data").Interface()
				input.contentType = v.FieldByName("ContentType").String()
				input.filename = v.FieldByName("Filename").String()
				input.headers = v.FieldByName("Headers").Interface().(http.Header)
			}
			result = append(result, input)
		}
		return nil
	}
	for _, p := range a.Parts {
		v, exists, err := httpField(native.FieldByName(p.Field), p.Required)
		if err != nil {
			return nil, httpLocated(p.Source, err)
		}
		if !exists {
			if a.Positional {
				gap = true
			}
			continue
		}
		if a.Positional && gap {
			return nil, httpLocated(p.Source, errors.New("positional multipart cannot skip a prefix part"))
		}
		present[p.Name] = true
		count++
		if err = appendValue(p, p.Name, v); err != nil {
			return nil, httpLocated(p.Source, err)
		}
	}
	if p := a.Additional; p != nil {
		values := native.FieldByName(p.Field)
		if values.Len() > a.MaxParts-len(result) {
			return nil, httpLocated(p.Source, httpProblem("resource-limit"))
		}
		if a.Positional {
			if gap && values.Len() > 0 {
				return nil, httpLocated(p.Source, errors.New("items cannot skip a prefix part"))
			}
			for i := 0; i < values.Len(); i++ {
				count++
				if err := appendValue(*p, "", values.Index(i)); err != nil {
					return nil, httpLocated(p.Source, err)
				}
			}
		} else {
			keys := values.MapKeys()
			sort.Slice(keys, func(i, j int) bool { return keys[i].String() < keys[j].String() })
			for _, key := range keys {
				name := key.String()
				for _, declared := range a.Parts {
					if name == declared.Name {
						return nil, httpLocated(p.Source, errors.New("extra part shadows a declared field"))
					}
				}
				present[name] = true
				count++
				if err := appendValue(*p, name, values.MapIndex(key)); err != nil {
					return nil, httpLocated(p.Source, err)
				}
			}
		}
	}
	for _, name := range a.Required {
		if !present[name] {
			return nil, httpLocated(a.Source, errors.New("required aggregate field missing"))
		}
	}
	if err := httpCardinality(count, a.Min, a.Max); err != nil {
		return nil, httpLocated(a.Source, err)
	}
	return result, nil
}
func httpPartMedia(p httpPartPlan, contentType string) (string, error) {
	if p.Kind == "style" {
		if contentType != "" {
			return "", errors.New("style-encoded part has no contentType selection")
		}
		return "", nil
	}
	if contentType == "" {
		if len(p.Media) != 1 || p.Media[0].Rank != 2 {
			return "", errors.New("part requires an explicit concrete Content-Type")
		}
		contentType = p.Media[0].Declared
	}
	actual, err := httpParseMedia(contentType, false)
	if err != nil {
		return "", err
	}
	matched := false
	for _, m := range p.Media {
		matched = matched || m.matches(actual)
	}
	if !matched {
		return "", errors.New("undeclared part Content-Type")
	}
	if charset, ok := actual.Parameters["charset"]; ok && p.Kind == "text" && !strings.EqualFold(charset, "utf-8") {
		return "", errors.New("unsupported part charset")
	}
	return contentType, nil
}
func httpPartEncode(input httpPartInput, limit int) ([]byte, error) {
	p := input.plan
	if p.MaxBytes < limit {
		limit = p.MaxBytes
	}
	if p.Kind == "binary" {
		data, ok := input.value.([]byte)
		if !ok {
			return nil, errors.New("byte part requires []byte")
		}
		if len(data) > limit {
			return nil, httpProblem("resource-limit")
		}
		return data, nil
	}
	v, err := httpCodecs[p.Codec].encode(input.value)
	if err != nil {
		return nil, err
	}
	if p.Kind == "json" {
		return httpJSONEncode(v, limit)
	}
	if p.Kind == "style" {
		if p.Serial.Encoding == "none" {
			// RFC6570 supplies the separators; unescaped data must not impersonate them.
			var check func(Value) bool
			check = func(v Value) bool {
				switch x := v.(type) {
				case string:
					return !strings.Contains(x, "&")
				case []Value:
					for _, i := range x {
						if !check(i) {
							return false
						}
					}
				case map[string]Value:
					for k, i := range x {
						if strings.ContainsAny(k, "&=") || !check(i) {
							return false
						}
					}
				}
				return true
			}
			if !check(v) {
				return nil, errors.New("ambiguous multipart style data")
			}
		}
		location := "query"
		if p.Serial.Encoding == "none" {
			location = "part"
		}
		text, err := httpSerialize(input.name, location, p.Serial, v, limit)
		return []byte(text), err
	}
	text, err := httpScalar(v, p.Scalar)
	if err != nil {
		return nil, err
	}
	if len(text) > limit {
		return nil, httpProblem("resource-limit")
	}
	return []byte(text), nil
}
func httpEncodeAggregate(m httpMediaPlan, value any, contentType string, limit, partLimit int) ([]byte, string, error) {
	a := m.Aggregate
	inputs, err := httpPartInputs(a, value)
	if err != nil {
		return nil, contentType, err
	}
	buffer := &httpBuffer{limit: limit}
	var writer *multipart.Writer
	if a.Multipart {
		writer = multipart.NewWriter(buffer)
		actual, err := httpParseMedia(contentType, false)
		if err != nil {
			return nil, contentType, err
		}
		if boundary, ok := actual.Parameters["boundary"]; ok {
			if err = writer.SetBoundary(boundary); err != nil {
				return nil, contentType, err
			}
		}
		actual.Parameters["boundary"] = writer.Boundary()
		contentType = mime.FormatMediaType(actual.Type+"/"+actual.Subtype, actual.Parameters)
	}
	separator := ""
	fieldCount := 0
	for _, input := range inputs {
		p := input.plan
		failure := func(err error) ([]byte, string, error) { return nil, contentType, httpLocated(p.Source, err) }
		bound := partLimit
		if limit-buffer.Len() < bound {
			bound = limit - buffer.Len()
		}
		var data []byte
		var pieces []httpPartWire
		if a.Multipart && p.Kind == "style" {
			pieces, err = httpMultipartStyle(input, bound, limit-buffer.Len(), a.MaxParts-fieldCount)
			if err != nil {
				return failure(err)
			}
			fieldCount += len(pieces)
		} else {
			data, err = httpPartEncode(input, bound)
			if err != nil {
				return failure(err)
			}
		}
		if p.Kind == "style" && !a.Multipart {
			count := bytes.Count(data, []byte("&")) + 1
			if count > a.MaxParts-fieldCount {
				return failure(httpProblem("resource-limit"))
			}
			fieldCount += count
			for _, pair := range strings.Split(string(data), "&") {
				name, value, ok := strings.Cut(pair, "=")
				if !ok {
					return failure(errors.New("invalid style field"))
				}
				pieces = append(pieces, httpPartWire{name: name, body: []byte(value)})
			}
		}
		if p.Kind != "style" {
			fieldCount++
			if fieldCount > a.MaxParts {
				return failure(httpProblem("resource-limit"))
			}
		}
		if !a.Multipart {
			var text string
			if p.Kind == "style" {
				text = string(data)
			} else {
				name, e := httpPercent(input.name, "form", bound)
				if e != nil {
					return failure(e)
				}
				text, e = httpPercent(string(data), p.Encoding, bound)
				if e != nil {
					return failure(e)
				}
				text = name + "=" + text
			}
			if _, err = buffer.Write([]byte(separator + text)); err != nil {
				return failure(err)
			}
			separator = "&"
			continue
		}
		headerLimit := limit - buffer.Len()
		if len(input.name) > headerLimit || len(input.filename) > headerLimit-len(input.name) {
			return failure(httpProblem("resource-limit"))
		}
		if strings.IndexFunc(input.name+input.filename, unicode.IsControl) >= 0 {
			return failure(errors.New("invalid part disposition value"))
		}
		selected, e := httpPartMedia(p, input.contentType)
		if e != nil {
			return failure(e)
		}
		if httpHeaderBytes(input.headers) > headerLimit {
			return failure(httpProblem("resource-limit"))
		}
		headers := input.headers.Clone()
		if headers == nil {
			headers = make(http.Header)
		}
		for key, values := range headers {
			if !httpToken(key) || strings.EqualFold(key, "Content-Transfer-Encoding") {
				return failure(errors.New("invalid or unsupported part header"))
			}
			for _, v := range values {
				if !httpHeaderValue(v) {
					return failure(errors.New("invalid part header value"))
				}
			}
		}
		if len(httpHeaderValues(headers, "Content-Type")) != 0 {
			return failure(errors.New("use Part.ContentType for part media selection"))
		}
		if selected != "" {
			headers.Set("Content-Type", selected)
		}
		if pieces == nil {
			pieces = []httpPartWire{{name: input.name, body: data}}
		}
		for _, piece := range pieces {
			h := headers.Clone()
			if !a.Positional {
				if values := httpHeaderValues(h, "Content-Disposition"); len(values) > 0 {
					if len(values) != 1 {
						return failure(errors.New("duplicate part disposition"))
					}
					kind, params, e := mime.ParseMediaType(values[0])
					if e != nil || kind != "form-data" || params["name"] != piece.name || input.filename != "" && params["filename"] != input.filename {
						return failure(errors.New("part disposition conflicts with its typed name or filename"))
					}
				} else {
					disposition := map[string]string{"name": piece.name}
					if input.filename != "" {
						disposition["filename"] = input.filename
					}
					h.Set("Content-Disposition", mime.FormatMediaType("form-data", disposition))
				}
			} else if input.filename != "" && len(httpHeaderValues(h, "Content-Disposition")) == 0 {
				h.Set("Content-Disposition", mime.FormatMediaType("attachment", map[string]string{"filename": input.filename}))
			}
			if httpHeaderBytes(h) > headerLimit {
				return failure(httpProblem("resource-limit"))
			}
			for _, declared := range p.Headers {
				if _, _, err := httpDecodeHeader(declared, h, headerLimit); err != nil {
					return failure(err)
				}
			}
			part, e := writer.CreatePart(textproto.MIMEHeader(h))
			if e != nil {
				return failure(e)
			}
			if _, e = part.Write(piece.body); e != nil {
				return failure(e)
			}
		}
	}
	if writer != nil {
		if err = writer.Close(); err != nil {
			return nil, contentType, err
		}
	}
	return buffer.Bytes(), contentType, nil
}

// Multipart RFC6570 output is a sequence of MIME names and bodies, not a query
// string to split. '&', '=' and non-ASCII names/data therefore remain literal.
func httpMultipartStyle(input httpPartInput, limit, headerLimit, remaining int) ([]httpPartWire, error) {
	p := input.plan
	s := p.Serial
	value, err := httpCodecs[p.Codec].encode(input.value)
	if err != nil {
		return nil, err
	}
	var result []httpPartWire
	put := func(name, text string) error {
		if len(result) >= remaining || len(name) > headerLimit || len(text) > limit || len(text) > p.MaxBytes {
			return httpProblem("resource-limit")
		}
		if strings.IndexFunc(name, unicode.IsControl) >= 0 {
			return errors.New("invalid MIME field name")
		}
		result = append(result, httpPartWire{name: name, body: []byte(text)})
		return nil
	}
	text := func(v Value, kind string) (string, error) {
		raw, err := httpScalar(v, kind)
		if err != nil {
			return "", err
		}
		return httpEncodeParameterValue(raw, "part", s, limit)
	}
	if s.Shape == "scalar" {
		v, err := text(value, s.Scalar)
		if err != nil {
			return nil, err
		}
		if err = put(input.name, v); err != nil {
			return nil, err
		}
		return result, nil
	}
	var fields []string
	delimiter := ","
	if s.Style == "spaceDelimited" {
		delimiter = " "
	}
	if s.Style == "pipeDelimited" {
		delimiter = "|"
	}
	add := func(v string) error {
		if strings.Contains(v, delimiter) {
			return errors.New("part data requires an API-defined delimiter escape")
		}
		fields = append(fields, v)
		return nil
	}
	if s.Shape == "array" {
		items, ok := value.([]Value)
		if !ok || len(items) == 0 {
			return nil, errors.New("nonempty scalar array required")
		}
		if !s.Explode && len(items)-1 > limit || s.Explode && len(items) > remaining {
			return nil, httpProblem("resource-limit")
		}
		for _, item := range items {
			v, err := text(item, s.Scalar)
			if err != nil {
				return nil, err
			}
			if s.Explode {
				err = put(input.name, v)
			} else {
				err = add(v)
			}
			if err != nil {
				return nil, err
			}
		}
	} else {
		object, ok := value.(map[string]Value)
		if !ok || len(object) == 0 {
			return nil, errors.New("nonempty flat object required")
		}
		if (s.Explode || s.Style == "deepObject") && len(object) > remaining {
			return nil, httpProblem("resource-limit")
		}
		keys := make([]string, 0, len(object))
		for key := range object {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		for _, key := range keys {
			kind, ok := s.Properties[key]
			if !ok {
				kind = s.Additional
				if kind == "" {
					return nil, errors.New("undeclared flat property")
				}
			}
			v, err := text(object[key], kind)
			if err != nil {
				return nil, err
			}
			k, err := httpEncodeParameterValue(key, "part", s, limit)
			if err != nil {
				return nil, err
			}
			if s.Style == "deepObject" {
				err = put(input.name+"["+k+"]", v)
			} else if s.Explode {
				err = put(k, v)
			} else {
				if err = add(k); err == nil {
					err = add(v)
				}
			}
			if err != nil {
				return nil, err
			}
		}
	}
	if len(fields) > 0 {
		size := 0
		for _, f := range fields {
			if len(f) > limit-size {
				return nil, httpProblem("resource-limit")
			}
			size += len(f)
		}
		if len(fields)-1 > limit-size {
			return nil, httpProblem("resource-limit")
		}
		if err = put(input.name, strings.Join(fields, delimiter)); err != nil {
			return nil, err
		}
	}
	return result, nil
}
func httpWireParts(m httpMediaPlan, r *rawHTTPResponse) ([]httpPartWire, error) {
	var result []httpPartWire
	if m.Kind == "form" {
		if len(r.body) == 0 {
			return result, nil
		}
		if bytes.Count(r.body, []byte("&"))+1 > m.Aggregate.MaxParts {
			return nil, httpProblem("resource-limit")
		}
		for _, pair := range strings.Split(string(r.body), "&") {
			name, value, ok := strings.Cut(pair, "=")
			if !ok {
				return nil, errors.New("form field requires =")
			}
			decodedName, err := url.QueryUnescape(name)
			if err != nil {
				return nil, err
			}
			decoded, err := url.QueryUnescape(value)
			if err != nil {
				return nil, err
			}
			if len(decoded) > r.partLimit {
				return nil, httpProblem("resource-limit")
			}
			result = append(result, httpPartWire{name: decodedName, rawName: name, rawValue: value, body: []byte(decoded)})
		}
		return result, nil
	}
	actual, err := httpParseMedia(r.contentType, false)
	if err != nil {
		return nil, err
	}
	boundary := actual.Parameters["boundary"]
	if boundary == "" {
		return nil, errors.New("multipart boundary is required")
	}
	reader := multipart.NewReader(bytes.NewReader(r.body), boundary)
	for {
		p, err := reader.NextRawPart()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		if len(result) >= m.Aggregate.MaxParts {
			_ = p.Close()
			return nil, httpProblem("resource-limit")
		}
		headers := http.Header(p.Header)
		if len(httpHeaderValues(headers, "Content-Transfer-Encoding")) > 0 {
			_ = p.Close()
			return nil, errors.New("unsupported part transfer encoding")
		}
		if httpHeaderBytes(headers) > r.maximum {
			_ = p.Close()
			return nil, httpProblem("resource-limit")
		}
		name, filename := "", ""
		if disposition := httpHeaderValues(headers, "Content-Disposition"); len(disposition) > 0 {
			if len(disposition) != 1 {
				_ = p.Close()
				return nil, errors.New("duplicate part disposition")
			}
			kind, params, e := mime.ParseMediaType(disposition[0])
			if e != nil {
				_ = p.Close()
				return nil, e
			}
			name = params["name"]
			filename = params["filename"]
			if !m.Aggregate.Positional {
				if _, exists := params["name"]; kind != "form-data" || !exists {
					_ = p.Close()
					return nil, errors.New("named part requires form-data name")
				}
			}
		} else if !m.Aggregate.Positional {
			_ = p.Close()
			return nil, errors.New("missing named part disposition")
		}
		data, e := io.ReadAll(io.LimitReader(p, int64(r.partLimit)+1))
		_ = p.Close()
		if e != nil {
			return nil, e
		}
		if len(data) > r.partLimit {
			return nil, httpProblem("resource-limit")
		}
		ct := httpHeaderValues(headers, "Content-Type")
		if len(ct) > 1 {
			return nil, errors.New("duplicate part Content-Type")
		}
		contentType := ""
		if len(ct) == 1 {
			contentType = ct[0]
		}
		result = append(result, httpPartWire{name: name, rawName: name, rawValue: string(data), body: data, headers: headers.Clone(), filename: filename, contentType: contentType})
	}
	return result, nil
}
func httpPartAccepts(p httpPartPlan, name string) bool {
	if p.Kind == "style" && p.Serial.Shape == "object" {
		if p.Serial.Style == "deepObject" {
			return strings.HasPrefix(name, p.Name+"[") && strings.HasSuffix(name, "]")
		}
		if p.Serial.Explode {
			_, ok := p.Serial.Properties[name]
			return ok || p.Serial.Additional != ""
		}
	}
	return name == p.Name
}
func httpDecodeStyle(p httpPartPlan, parts []httpPartWire, limit int) (Value, error) {
	s := p.Serial
	decode := func(text string) (string, error) {
		if s.Encoding == "none" {
			return text, nil
		}
		return url.PathUnescape(text)
	}
	if s.Shape == "scalar" {
		if len(parts) != 1 {
			return nil, errors.New("duplicate scalar style field")
		}
		text, e := decode(parts[0].rawValue)
		if e != nil {
			return nil, e
		}
		return httpTextValue(text, s.Scalar)
	}
	if s.Shape == "array" && s.Explode {
		var out []Value
		for _, part := range parts {
			text, e := decode(part.rawValue)
			if e != nil {
				return nil, e
			}
			v, e := httpTextValue(text, s.Scalar)
			if e != nil {
				return nil, e
			}
			out = append(out, v)
		}
		return out, nil
	}
	object := map[string]Value{}
	put := func(key, text string) error {
		key, e := decode(key)
		if e != nil {
			return e
		}
		text, e = decode(text)
		if e != nil {
			return e
		}
		if _, ok := object[key]; ok {
			return errors.New("duplicate style property")
		}
		kind, ok := s.Properties[key]
		if !ok {
			kind = s.Additional
			if kind == "" {
				return errors.New("undeclared style property")
			}
		}
		v, e := httpTextValue(text, kind)
		if e == nil {
			object[key] = v
		}
		return e
	}
	if s.Shape == "object" && (s.Explode || s.Style == "deepObject") {
		for _, part := range parts {
			key := part.rawName
			if s.Style == "deepObject" {
				decoded := key
				if s.Encoding != "none" {
					var e error
					decoded, e = url.PathUnescape(key)
					if e != nil {
						return nil, e
					}
				}
				key = decoded[len(p.Name)+1 : len(decoded)-1]
				if s.Encoding != "none" {
					key = url.PathEscape(key)
				}
			}
			if e := put(key, part.rawValue); e != nil {
				return nil, e
			}
		}
		return object, nil
	}
	if len(parts) != 1 {
		return nil, errors.New("duplicate delimited style field")
	}
	delimiter := ","
	if s.Style == "spaceDelimited" {
		delimiter = "%20"
	}
	if s.Style == "pipeDelimited" {
		delimiter = "%7C"
	}
	if s.Encoding == "none" {
		if s.Style == "spaceDelimited" {
			delimiter = " "
		}
		if s.Style == "pipeDelimited" {
			delimiter = "|"
		}
	}
	values := strings.Split(parts[0].rawValue, delimiter)
	if s.Shape == "array" {
		out := make([]Value, 0, len(values))
		for _, text := range values {
			text, e := decode(text)
			if e != nil {
				return nil, e
			}
			v, e := httpTextValue(text, s.Scalar)
			if e != nil {
				return nil, e
			}
			out = append(out, v)
		}
		return out, nil
	}
	if len(values)%2 != 0 {
		return nil, errors.New("odd delimited object length")
	}
	for i := 0; i < len(values); i += 2 {
		if e := put(values[i], values[i+1]); e != nil {
			return nil, e
		}
	}
	return object, nil
}
func httpDecodePart(p httpPartPlan, parts []httpPartWire, multipart bool, limit int, headerLimit int) (any, error) {
	if len(parts) == 0 {
		return nil, errors.New("missing part")
	}
	for _, part := range parts {
		if len(part.body) > limit || len(part.body) > p.MaxBytes {
			return nil, httpProblem("resource-limit")
		}
		if multipart {
			for _, h := range p.Headers {
				if _, _, err := httpDecodeHeader(h, part.headers, headerLimit); err != nil {
					return nil, err
				}
			}
		}
	}
	if p.Kind == "style" {
		value, err := httpDecodeStyle(p, parts, limit)
		if err != nil {
			return nil, err
		}
		return httpCodecs[p.Codec].decode(value)
	}
	if len(parts) != 1 {
		return nil, errors.New("duplicate non-repeated part")
	}
	part := parts[0]
	if len(part.body) > limit || len(part.body) > p.MaxBytes {
		return nil, httpProblem("resource-limit")
	}
	if multipart {
		contentType := part.contentType
		if contentType == "" {
			contentType = "text/plain"
		}
		if _, err := httpPartMedia(p, contentType); err != nil {
			return nil, err
		}
	}
	if p.Kind == "binary" {
		return part.body, nil
	}
	var value Value
	var err error
	if p.Kind == "json" {
		value, err = httpJSONParse(part.body, limit)
	} else {
		value, err = httpTextValue(string(part.body), p.Scalar)
	}
	if err != nil {
		return nil, err
	}
	return httpCodecs[p.Codec].decode(value)
}
func httpSetPart(target reflect.Value, p httpPartPlan, parts []httpPartWire, a *httpAggregatePlan, limit, headerLimit int) error {
	if p.Repeated {
		if err := httpCardinality(len(parts), p.Min, p.Max); err != nil {
			return err
		}
		out := reflect.MakeSlice(target.Type(), 0, len(parts))
		single := p
		single.Repeated = false
		for _, part := range parts {
			v := reflect.New(target.Type().Elem()).Elem()
			if err := httpSetPart(v, single, []httpPartWire{part}, a, limit, headerLimit); err != nil {
				return err
			}
			out = reflect.Append(out, v)
		}
		target.Set(out)
		return nil
	}
	value, err := httpDecodePart(p, parts, a.Multipart, limit, headerLimit)
	if err != nil {
		return err
	}
	destination := target
	if a.Multipart {
		destination = target.FieldByName("Data")
		first := parts[0]
		target.FieldByName("ContentType").SetString(first.contentType)
		target.FieldByName("Filename").SetString(first.filename)
		target.FieldByName("Headers").Set(reflect.ValueOf(first.headers.Clone()))
	}
	if value == nil {
		destination.SetZero()
	} else {
		v := reflect.ValueOf(value)
		if !v.Type().AssignableTo(destination.Type()) {
			return errors.New("part codec type mismatch")
		}
		destination.Set(v)
	}
	return nil
}
func httpDecodeAggregate(m httpMediaPlan, r *rawHTTPResponse) (any, error) {
	parts, err := httpWireParts(m, r)
	if err != nil {
		return nil, err
	}
	a := m.Aggregate
	native := reflect.New(httpAggregateTypes[a.Type]).Elem()
	groups := make([][]httpPartWire, len(a.Parts))
	extras := map[string][]httpPartWire{}
	var remaining []httpPartWire
	for i, part := range parts {
		if a.Positional {
			if i < len(groups) {
				groups[i] = []httpPartWire{part}
			} else {
				remaining = append(remaining, part)
			}
			continue
		}
		matched := -1
		for j, p := range a.Parts {
			if httpPartAccepts(p, part.name) {
				if matched >= 0 {
					return nil, errors.New("ambiguous exploded part name")
				}
				matched = j
			}
		}
		if matched >= 0 {
			groups[matched] = append(groups[matched], part)
		} else {
			extras[part.name] = append(extras[part.name], part)
		}
	}
	count := 0
	present := map[string]bool{}
	for i, p := range a.Parts {
		if len(groups[i]) == 0 {
			if p.Required {
				return nil, httpLocated(p.Source, errors.New("required part missing"))
			}
			continue
		}
		count++
		present[p.Name] = true
		field := native.FieldByName(p.Field)
		if !p.Required {
			field.FieldByName("IsSet").SetBool(true)
			field = field.FieldByName("Value")
		}
		if err = httpSetPart(field, p, groups[i], a, r.partLimit, r.maximum); err != nil {
			return nil, httpLocated(p.Source, err)
		}
	}
	if len(extras) > 0 || len(remaining) > 0 {
		if a.Additional == nil {
			return nil, httpLocated(a.Source, errors.New("undeclared part"))
		}
		p := *a.Additional
		field := native.FieldByName(p.Field)
		if a.Positional {
			field.Set(reflect.MakeSlice(field.Type(), 0, len(remaining)))
			for _, part := range remaining {
				v := reflect.New(field.Type().Elem()).Elem()
				if err = httpSetPart(v, p, []httpPartWire{part}, a, r.partLimit, r.maximum); err != nil {
					return nil, httpLocated(p.Source, err)
				}
				field.Set(reflect.Append(field, v))
				count++
			}
		}
		if !a.Positional {
			field.Set(reflect.MakeMapWithSize(field.Type(), len(extras)))
			for name, parts := range extras {
				v := reflect.New(field.Type().Elem()).Elem()
				p.Name = name
				if err = httpSetPart(v, p, parts, a, r.partLimit, r.maximum); err != nil {
					return nil, httpLocated(p.Source, err)
				}
				field.SetMapIndex(reflect.ValueOf(name), v)
				present[name] = true
				count++
			}
		}
	}
	for _, name := range a.Required {
		if !present[name] {
			return nil, httpLocated(a.Source, errors.New("required aggregate property missing"))
		}
	}
	if err = httpCardinality(count, a.Min, a.Max); err != nil {
		return nil, httpLocated(a.Source, err)
	}
	return native.Interface(), nil
}
