package sdk

import (
	"errors"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

func httpPercent(value, encoding string, limit int) (string, error) {
	if !utf8.ValidString(value) {
		return "", httpProblem("request-representation")
	}
	if len(value) > limit {
		return "", httpProblem("resource-limit")
	}
	if encoding == "none" {
		return value, nil
	}
	const hex = "0123456789ABCDEF"
	var out strings.Builder
	for i := 0; i < len(value); i++ {
		b := value[i]
		plain := b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || strings.ContainsRune("-._~", rune(b))
		if encoding == "reserved" && b == '%' && i+2 < len(value) && httpHex(value[i+1]) && httpHex(value[i+2]) {
			if limit-out.Len() < 3 {
				return "", httpProblem("resource-limit")
			}
			out.WriteString(value[i : i+3])
			i += 2
			continue
		}
		if encoding == "reserved" && strings.ContainsRune(":/?#[]@!$&'()*+,;=", rune(b)) {
			plain = true
		}
		if encoding == "form" {
			plain = b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || strings.ContainsRune("*-._", rune(b))
		}
		width := 3
		if plain || encoding == "form" && b == ' ' {
			width = 1
		}
		if width > limit-out.Len() {
			return "", httpProblem("resource-limit")
		}
		if plain {
			out.WriteByte(b)
		} else if encoding == "form" && b == ' ' {
			out.WriteByte('+')
		} else {
			out.WriteByte('%')
			out.WriteByte(hex[b>>4])
			out.WriteByte(hex[b&15])
		}
	}
	return out.String(), nil
}
func httpHex(b byte) bool {
	return b >= '0' && b <= '9' || b >= 'a' && b <= 'f' || b >= 'A' && b <= 'F'
}
func httpScalar(v Value, kind string) (string, error) {
	switch value := v.(type) {
	case string:
		if kind == "" || kind == "any" || kind == "string" {
			return value, nil
		}
	case bool:
		if kind == "" || kind == "any" || kind == "boolean" {
			if value {
				return "true", nil
			}
			return "false", nil
		}
	case Number:
		if value.String() != "" && (kind == "" || kind == "any" || kind == "number" || kind == "integer" && value.IsInteger()) {
			return value.String(), nil
		}
	case Integer:
		if value.String() != "" && (kind == "" || kind == "any" || kind == "number" || kind == "integer") {
			return value.String(), nil
		}
	}
	return "", httpProblem("request-validation")
}
func httpTextValue(text, kind string) (Value, error) {
	if !utf8.ValidString(text) {
		return nil, errors.New("invalid UTF-8 text")
	}
	switch kind {
	case "", "any", "string":
		return text, nil
	case "boolean":
		if text == "true" {
			return true, nil
		}
		if text == "false" {
			return false, nil
		}
		return nil, errors.New("invalid boolean text")
	case "integer":
		return ParseInteger(text)
	case "number":
		return ParseNumber(text)
	}
	return nil, errors.New("unsupported text scalar")
}
func httpJSONEncode(value Value, limit int) ([]byte, error) {
	l := DefaultLimits()
	if limit < l.MaxOutputBytes {
		l.MaxOutputBytes = limit
	}
	return Encode(value, l)
}
func httpJSONParse(data []byte, limit int) (Value, error) {
	l := DefaultLimits()
	if limit < l.MaxBytes {
		l.MaxBytes = limit
	}
	return Parse(data, l)
}
func httpEncodeParameterValue(value, location string, s httpSerial, limit int) (string, error) {
	if s.Encoding == "none" {
		for _, c := range value {
			if unicode.IsControl(c) && !(location == "header" && c == '\t') {
				return "", errors.New("control character in wire value")
			}
		}
	}
	if location == "cookie" && s.Encoding == "none" && !httpCookieValue(value) {
		return "", errors.New("cookie value requires caller escaping")
	}
	if s.Style == "spaceDelimited" && strings.Contains(value, " ") || s.Style == "pipeDelimited" && strings.Contains(value, "|") || s.Style == "deepObject" && strings.ContainsAny(value, "[]") {
		return "", errors.New("ambiguous encoded style delimiter")
	}
	if s.Encoding == "reserved" {
		hazards := ""
		switch location {
		case "path":
			hazards = "#[]/?"
		case "query", "querystring":
			hazards = "#[]&=+"
		case "cookie":
			hazards = ";,"
		}
		if strings.ContainsAny(value, hazards) {
			return "", errors.New("reserved value requires caller escaping")
		}
		if s.Shape != "scalar" {
			delimiter := ""
			switch s.Style {
			case "simple", "form", "cookie":
				delimiter = ","
			case "label":
				delimiter = ".,"
			case "matrix":
				delimiter = ";,"
			}
			if strings.ContainsAny(value, delimiter) {
				return "", errors.New("reserved active delimiter")
			}
		}
	}
	return httpPercent(value, s.Encoding, limit)
}
func httpSerialize(name, location string, s httpSerial, value Value, limit int) (string, error) {
	if s.Content != "" {
		var text string
		if s.Content == "json" {
			data, err := httpJSONEncode(value, limit)
			if err != nil {
				return "", err
			}
			text = string(data)
		} else {
			var err error
			text, err = httpScalar(value, "")
			if err != nil {
				return "", err
			}
		}
		text, err := httpEncodeParameterValue(text, location, s, limit)
		if err != nil {
			return "", err
		}
		if location == "query" || location == "cookie" {
			name, err = httpPercent(name, "uri", limit)
			if err != nil {
				return "", err
			}
			text = name + "=" + text
		}
		if len(text) > limit {
			return "", httpProblem("resource-limit")
		}
		return text, nil
	}
	encoding := "uri"
	if location == "header" || location == "part" || s.Style == "cookie" {
		encoding = "none"
	}
	name, err := httpPercent(name, encoding, limit)
	if err != nil {
		return "", err
	}
	var one string
	var items []string
	var props [][2]string
	used := 0
	encode := func(value string) (string, error) {
		text, err := httpEncodeParameterValue(value, location, s, limit-used)
		used += len(text)
		return text, err
	}
	switch s.Shape {
	case "scalar":
		one, err = httpScalar(value, s.Scalar)
		if err == nil {
			one, err = encode(one)
		}
	case "array":
		array, ok := value.([]Value)
		if !ok || len(array) == 0 {
			return "", errors.New("nonempty scalar array required")
		}
		for _, item := range array {
			v, e := httpScalar(item, s.Scalar)
			if e != nil {
				return "", e
			}
			v, e = encode(v)
			if e != nil {
				return "", e
			}
			items = append(items, v)
		}
	case "object":
		object, ok := value.(map[string]Value)
		if !ok || len(object) == 0 {
			return "", errors.New("nonempty flat object required")
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
					return "", errors.New("undeclared flat property")
				}
			}
			v, e := httpScalar(object[key], kind)
			if e != nil {
				return "", e
			}
			k, e := encode(key)
			if e != nil {
				return "", e
			}
			v, e = encode(v)
			if e != nil {
				return "", e
			}
			props = append(props, [2]string{k, v})
		}
	default:
		return "", errors.New("unsupported wire shape")
	}
	if err != nil {
		return "", err
	}
	if err = httpExpansionBudget(name, one, items, props, s, location, used, limit); err != nil {
		return "", err
	}
	flattened := func(delimiter string) string {
		var values []string
		for _, pair := range props {
			values = append(values, pair[0], pair[1])
		}
		return strings.Join(values, delimiter)
	}
	pairs := func(delimiter string) string {
		var values []string
		for _, pair := range props {
			values = append(values, pair[0]+"="+pair[1])
		}
		return strings.Join(values, delimiter)
	}
	named := func(k, v string) string {
		if s.Style == "matrix" && v == "" {
			return k
		}
		return k + "=" + v
	}
	var out string
	switch s.Style {
	case "simple", "label":
		delimiter := ","
		if s.Style == "label" && s.Explode {
			delimiter = "."
		}
		if s.Shape == "scalar" {
			out = one
		} else if s.Shape == "array" {
			out = strings.Join(items, delimiter)
		} else if s.Explode {
			out = pairs(delimiter)
		} else {
			out = flattened(",")
		}
		if s.Style == "label" {
			out = "." + out
		}
	case "matrix":
		if s.Shape == "scalar" {
			out = ";" + named(name, one)
		} else if s.Shape == "array" {
			if s.Explode {
				for _, v := range items {
					out += ";" + named(name, v)
				}
			} else {
				out = ";" + name + "=" + strings.Join(items, ",")
			}
		} else if s.Explode {
			for _, p := range props {
				out += ";" + named(p[0], p[1])
			}
		} else {
			out = ";" + name + "=" + flattened(",")
		}
	case "form", "cookie":
		delimiter := "&"
		if s.Style == "cookie" {
			delimiter = "; "
		}
		if s.Shape == "scalar" {
			out = name + "=" + one
		} else if s.Shape == "array" {
			if s.Explode {
				values := make([]string, len(items))
				for i, v := range items {
					values[i] = name + "=" + v
				}
				out = strings.Join(values, delimiter)
			} else {
				out = name + "=" + strings.Join(items, ",")
			}
		} else if s.Explode {
			out = pairs(delimiter)
		} else {
			out = name + "=" + flattened(",")
		}
	case "spaceDelimited", "pipeDelimited":
		delimiter := "%20"
		if s.Style == "pipeDelimited" {
			delimiter = "%7C"
		}
		if location == "part" {
			delimiter = " "
			if s.Style == "pipeDelimited" {
				delimiter = "|"
			}
		}
		if s.Shape == "array" {
			out = name + "=" + strings.Join(items, delimiter)
		} else {
			out = name + "=" + flattened(delimiter)
		}
	case "deepObject":
		var values []string
		for _, p := range props {
			if location == "part" {
				values = append(values, name+"["+p[0]+"]="+p[1])
			} else {
				values = append(values, name+"%5B"+p[0]+"%5D="+p[1])
			}
		}
		out = strings.Join(values, "&")
	default:
		return "", errors.New("unsupported parameter style")
	}
	if len(out) > limit {
		return "", httpProblem("resource-limit")
	}
	return out, nil
}

// Account for repeated names and separators before allocating their expansion.
func httpExpansionBudget(name, one string, items []string, props [][2]string, s httpSerial, location string, used, limit int) error {
	ok := true
	add := func(count, size int) {
		if !ok {
			return
		}
		if count < 0 || size < 0 || size > 0 && count > (limit-used)/size {
			ok = false
			return
		}
		used += count * size
	}
	n := len(items)
	if s.Shape == "object" {
		n = len(props)
	}
	switch s.Style {
	case "simple", "label":
		if s.Style == "label" {
			add(1, 1)
		}
		if s.Shape == "array" {
			add(n-1, 1)
		} else if s.Shape == "object" {
			add(2*n-1, 1)
		}
	case "matrix":
		if s.Shape == "scalar" {
			add(1, len(name)+1)
			if one != "" {
				add(1, 1)
			}
		} else if s.Explode {
			if s.Shape == "array" {
				add(n, len(name)+1)
				for _, v := range items {
					if v != "" {
						add(1, 1)
					}
				}
			} else {
				add(n, 1)
				for _, v := range props {
					if v[1] != "" {
						add(1, 1)
					}
				}
			}
		} else {
			add(1, len(name)+2)
			if s.Shape == "array" {
				add(n-1, 1)
			} else {
				add(2*n-1, 1)
			}
		}
	case "form", "cookie":
		separator := 1
		if s.Style == "cookie" {
			separator = 2
		}
		if s.Shape == "scalar" {
			add(1, len(name)+1)
		} else if s.Explode {
			if s.Shape == "array" {
				add(n, len(name)+1)
			} else {
				add(n, 1)
			}
			add(n-1, separator)
		} else {
			add(1, len(name)+1)
			if s.Shape == "array" {
				add(n-1, 1)
			} else {
				add(2*n-1, 1)
			}
		}
	case "deepObject":
		width := 7
		if location == "part" {
			width = 3
		}
		add(n, len(name)+width)
		add(n-1, 1)
	case "spaceDelimited", "pipeDelimited":
		width := 3
		if location == "part" {
			width = 1
		}
		add(1, len(name)+1)
		if s.Shape == "object" {
			n *= 2
		}
		add(n-1, width)
	}
	if !ok || used > limit {
		return httpProblem("resource-limit")
	}
	return nil
}

// Header parsing is the inverse of its declared scalar/array/flat-object codec
// input. Unknown scalar property types stay strings rather than being guessed.
func httpParseHeader(text string, s httpSerial, limit int) (Value, error) {
	if !httpHeaderValue(text) {
		return nil, errors.New("invalid header field value")
	}
	if len(text) > limit {
		return nil, httpProblem("resource-limit")
	}
	if s.Content == "json" {
		return httpJSONParse([]byte(text), limit)
	}
	if s.Content == "text" {
		return httpTextValue(text, s.Scalar)
	}
	if s.Shape == "scalar" {
		return httpTextValue(text, s.Scalar)
	}
	fields := strings.Split(text, ",")
	if s.Shape == "array" {
		out := make([]Value, 0, len(fields))
		for _, field := range fields {
			v, err := httpTextValue(strings.TrimSpace(field), s.Scalar)
			if err != nil {
				return nil, err
			}
			out = append(out, v)
		}
		return out, nil
	}
	out := make(map[string]Value)
	for i := 0; i < len(fields); i++ {
		key, value := "", ""
		if s.Explode {
			var ok bool
			key, value, ok = strings.Cut(fields[i], "=")
			if !ok {
				return nil, errors.New("invalid exploded object header")
			}
		} else {
			key = fields[i]
			i++
			if i == len(fields) {
				return nil, errors.New("odd object header field count")
			}
			value = fields[i]
		}
		key = strings.TrimSpace(key)
		value = strings.TrimSpace(value)
		if _, exists := out[key]; exists {
			return nil, errors.New("duplicate object header property")
		}
		kind, ok := s.Properties[key]
		if !ok {
			kind = s.Additional
			if kind == "" {
				return nil, errors.New("undeclared object header property")
			}
		}
		v, err := httpTextValue(value, kind)
		if err != nil {
			return nil, err
		}
		out[key] = v
	}
	return out, nil
}
