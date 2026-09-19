// Package sdk contains a dependency-free, bounded, exact JSON runtime.
//
// Representation only: it models JSON values; it does not provide OpenAPI
// schema validation, model codecs, or an HTTP client, and it does not clear any
// model codec obligation. Every numeric value stays an exact opaque token:
// nothing rounds, expands an exponent, or substitutes zero for an out-of-range
// value. Per-call Limits bound input bytes, output bytes, depth, values, and
// token length; there is no global configuration.
//
// Unicode: strings are Unicode scalar values in valid UTF-8. Invalid UTF-8 and
// \u escapes for unpaired surrogates fail with kind "utf8"; replacement
// characters are never substituted. Noncharacters (U+FFFE, U+FFFD) are scalar
// values and pass through unchanged; only surrogates are excluded.
package sdk

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"maps"
	"reflect"
	"slices"
	"strconv"
	"unicode/utf8"
)

// Value is the exact JSON domain: nil, bool, string, Number, Integer, []Value,
// and map[string]Value. Parse produces exactly these kinds; Encode accepts
// exactly these kinds and rejects everything else instead of coercing it.
type Value = any

// Number is an opaque exact JSON number token, such as 1.0000000000000000001,
// -0, or 1e999999. Construct one with ParseNumber; Parse yields a Number for
// every numeric token. Zero-value policy: the zero Number and zero Integer are
// invalid and carry no token; String reports "", IsInteger reports false, and
// Encode rejects them with kind "type". Invalid input never becomes a zero
// number.
type Number struct {
	token string
}

// Integer is an opaque exact token for a number that is mathematically an
// integer, such as 9007199254740993 or 1e3. Construct one with ParseInteger;
// the Number zero-value policy applies unchanged.
type Integer struct {
	token string
}

// JSONErrorKind classifies every failure this package reports.
type JSONErrorKind string

const (
	JSONSyntax        JSONErrorKind = "syntax"         // grammar violation
	JSONDuplicateName JSONErrorKind = "duplicate-name" // same decoded name twice in one object ("a" and "\u0061" collide)
	JSONBadUnicode    JSONErrorKind = "utf8"           // invalid UTF-8 or an escaped lone surrogate
	JSONLimit         JSONErrorKind = "limit"          // a Limits budget was exceeded
	JSONCycle         JSONErrorKind = "cycle"          // Encode met a container on its own active path
	JSONType          JSONErrorKind = "type"           // Encode met a value outside the domain: floats, structs, pointers, functions, zero-value numbers
)

// JSONError is the classified error type; every exported function returns
// errors of concrete type *JSONError.
type JSONError struct {
	Kind    JSONErrorKind
	Offset  int // byte offset for parse errors; -1 for token and encode errors
	Message string
}

func (e *JSONError) Error() string {
	if e.Offset < 0 {
		return fmt.Sprintf("json %s: %s", e.Kind, e.Message)
	}
	return fmt.Sprintf("json %s at byte %d: %s", e.Kind, e.Offset, e.Message)
}

// Limits are per-call resource budgets bounding work and output, not schema
// constraints, and there is no global configuration.
type Limits struct {
	MaxBytes        int // Parse input bytes
	MaxOutputBytes  int // Encode output bytes; overflow beyond this is bounded by MaxDepth structural bytes
	MaxDepth        int // nesting depth, both directions; zero admits scalars only
	MaxNodes        int // JSON values per call; Encode also charges each object member name
	MaxNumberLength int // numeric token characters, both directions
}

// DefaultLimits returns the recommended budgets: 64 MiB input and output,
// depth 256, one million values, and 4096 numeric characters.
func DefaultLimits() Limits {
	return Limits{MaxBytes: 64 << 20, MaxOutputBytes: 64 << 20, MaxDepth: 256, MaxNodes: 1 << 20, MaxNumberLength: 4096}
}

// check validates shared budgets; parse selects which size budget applies.
func (l Limits) check(parse bool) *JSONError {
	switch {
	case l.MaxNodes <= 0:
		return limitError("JSON value budget must be positive")
	case l.MaxNumberLength <= 0:
		return limitError("numeric token budget must be positive")
	case l.MaxDepth < 0 || l.MaxDepth > 256:
		return limitError("JSON nesting budget must be between zero and 256")
	case parse && l.MaxBytes <= 0:
		return limitError("input byte budget must be positive")
	case !parse && l.MaxOutputBytes <= 0:
		return limitError("output byte budget must be positive")
	}
	return nil
}

func limitError(message string) *JSONError {
	return &JSONError{Kind: JSONLimit, Offset: -1, Message: message}
}

// budget tracks the per-call node charge shared by Parse and Encode.
type budget struct {
	limits Limits
	nodes  int
}

func (b *budget) charge() error {
	if b.nodes >= b.limits.MaxNodes {
		return limitError("JSON value budget exceeded")
	}
	b.nodes++
	return nil
}

// numberScan records the RFC 8259 grammar parts of one number token.
type numberScan struct {
	intStart, intEnd   int // integer digits, after any minus sign
	fracStart, fracEnd int // fraction digits; equal when there is no '.'
	exponent           int // decimal exponent, saturated to the token length
	end                int // exclusive end of the token
	ok                 bool
}

// scanNumber scans one JSON number at s[at:]. Exponent digits beyond nine
// saturate to the token length, which dominates all mantissa digit counts.
// Leading exponent zeroes never change its mathematical value. Scanning never
// expands an exponent or rounds; invalid prefixes return the zero scan.
func scanNumber(s string, at int) numberScan {
	scan := numberScan{intStart: at, fracStart: at, fracEnd: at, ok: true}
	i := at
	if i < len(s) && s[i] == '-' {
		i++
	}
	scan.intStart = i
	if i = scanDigits(s, i); i == scan.intStart || (i-scan.intStart > 1 && s[scan.intStart] == '0') {
		return numberScan{} // no digits, or a leading zero
	}
	scan.intEnd = i
	if i < len(s) && s[i] == '.' {
		scan.fracStart = i + 1
		if i = scanDigits(s, i+1); i == scan.fracStart {
			return numberScan{}
		}
		scan.fracEnd = i
	}
	if i < len(s) && (s[i] == 'e' || s[i] == 'E') {
		i++
		negative := false
		if i < len(s) && (s[i] == '+' || s[i] == '-') {
			negative = s[i] == '-'
			i++
		}
		expStart := i
		if i = scanDigits(s, i); i == expStart {
			return numberScan{}
		}
		for k := expStart; k < i; k++ {
			digit := int(s[k] - '0')
			if scan.exponent > (len(s)-digit)/10 {
				scan.exponent = len(s)
				break
			}
			scan.exponent = scan.exponent*10 + digit
		}
		if negative {
			scan.exponent = -scan.exponent
		}
	}
	scan.end = i
	return scan
}

func scanDigits(s string, i int) int {
	for i < len(s) && isDigit(s[i]) {
		i++
	}
	return i
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

// validNumberToken reports whether token is exactly one JSON number.
func validNumberToken(token string) bool {
	scan := scanNumber(token, 0)
	return scan.ok && scan.end == len(token)
}

// ParseNumber validates token as one exact JSON number, returned unchanged;
// huge exponents and negative zero stay tokens. Errors are *JSONError.
func ParseNumber(token string) (Number, error) {
	if !validNumberToken(token) {
		return Number{}, &JSONError{Kind: JSONSyntax, Offset: -1,
			Message: "not a JSON number"}
	}
	return Number{token: token}, nil
}

// ParseInteger validates token as one exact JSON number that is mathematically
// an integer: every fractional digit must be zero, so 1e3 and -10.00e-1
// qualify while 1.5 and 1e-3 do not. Integrality never expands an exponent;
// errors are *JSONError with kind "syntax".
func ParseInteger(token string) (Integer, error) {
	if _, err := ParseNumber(token); err != nil {
		return Integer{}, err
	}
	if !integral(token) {
		return Integer{}, &JSONError{Kind: JSONSyntax, Offset: -1,
			Message: "numeric token is not an integer"}
	}
	return Integer{token: token}, nil
}

// String returns the exact original token; the zero Number reports "".
func (n Number) String() string { return n.token }

// String returns the exact original integer token; a zero Integer is invalid.
func (n Integer) String() string { return n.token }

// IsInteger reports whether the token is a mathematical integer; zero is invalid.
func (n Number) IsInteger() bool { return integral(n.token) }

// integral reports whether the token is a mathematical integer: "-0",
// "0e-999999", and "-10.00e-1" are; "1.5" is not.
func integral(token string) bool {
	scan := scanNumber(token, 0)
	if !scan.ok || scan.end != len(token) {
		return false
	}
	fracDigits := scan.fracEnd - scan.fracStart
	if scan.exponent >= fracDigits {
		return true
	}
	allZero := func(chunk string) bool {
		for k := 0; k < len(chunk); k++ {
			if chunk[k] != '0' {
				return false
			}
		}
		return true
	}
	if scan.exponent <= -(scan.intEnd - scan.intStart) {
		return allZero(token[scan.intStart:scan.intEnd]) && allZero(token[scan.fracStart:scan.fracEnd])
	}
	fractional := fracDigits - scan.exponent
	switch {
	case fractional >= scan.intEnd-scan.intStart+fracDigits: // under 1: integral only when zero
		return allZero(token[scan.intStart:scan.intEnd]) && allZero(token[scan.fracStart:scan.fracEnd])
	case fractional <= fracDigits:
		return allZero(token[scan.fracEnd-fractional : scan.fracEnd])
	default:
		return allZero(token[scan.fracStart:scan.fracEnd]) &&
			allZero(token[scan.intEnd-(fractional-fracDigits):scan.intEnd])
	}
}

// parser tokenizes one input document with encoding/json under per-call budgets.
type parser struct {
	budget
	dec *json.Decoder
}

// Parse decodes one exact JSON document into the Value domain; every numeric
// token becomes a Number that keeps its exact characters (IsInteger and
// ParseInteger give the integer kind). The stdlib tokenizer is guarded so its
// silent-replacement behavior cannot trigger: input must be valid UTF-8, a raw
// scan rejects unpaired-surrogate \u escapes, and object names are checked
// after escape decoding, so "a" and "\u0061" collide. Errors are *JSONError.
func Parse(data []byte, limits Limits) (Value, error) {
	if err := limits.check(true); err != nil {
		return nil, err
	}
	if len(data) > limits.MaxBytes {
		return nil, limitError("input exceeds its byte budget")
	}
	if !utf8.Valid(data) {
		return nil, &JSONError{Kind: JSONBadUnicode, Offset: -1, Message: "input is not valid UTF-8"}
	}
	if at := loneSurrogateEscape(data); at >= 0 {
		return nil, &JSONError{Kind: JSONBadUnicode, Offset: at, Message: "escaped lone surrogate"}
	}
	p := &parser{budget: budget{limits: limits}, dec: json.NewDecoder(bytes.NewReader(data))}
	p.dec.UseNumber()
	v, err := p.value(0)
	if err != nil {
		return nil, err
	}
	if _, err := p.dec.Token(); !errors.Is(err, io.EOF) {
		if err != nil {
			return nil, p.fail(err)
		}
		return nil, p.failAt(int(p.dec.InputOffset()), JSONSyntax, "unexpected trailing bytes")
	}
	return v, nil
}

// loneSurrogateEscape returns the offset of the first \u escape for an
// unpaired surrogate, or -1. The scan walks strings escape-aware, so escaped
// backslashes are never misread as escape introducers; malformed escapes are
// left for the tokenizer to reject.
func loneSurrogateEscape(data []byte) int {
	for i := 0; i < len(data); i++ {
		if data[i] != '"' {
			continue
		}
		for i++; i < len(data) && data[i] != '"'; i++ {
			if data[i] != '\\' {
				continue
			}
			i++
			if i+5 > len(data) || data[i] != 'u' {
				continue
			}
			first, ok := hex4(data[i+1 : i+5])
			if !ok {
				continue
			}
			low, high := first >= 0xDC00 && first <= 0xDFFF, first >= 0xD800 && first <= 0xDBFF
			paired := false
			if high && i+11 <= len(data) && data[i+5] == '\\' && data[i+6] == 'u' {
				second, ok := hex4(data[i+7 : i+11])
				paired = ok && second >= 0xDC00 && second <= 0xDFFF
				if paired {
					i += 10 // skip the whole pair
					continue
				}
			}
			if low || (high && !paired) {
				return i - 1
			}
			i += 4 // skip the four hex digits
		}
	}
	return -1
}

// hex4 reads four hexadecimal digits.
func hex4(chunk []byte) (uint16, bool) {
	value, err := strconv.ParseUint(string(chunk), 16, 16)
	return uint16(value), err == nil
}

func (p *parser) fail(err error) error {
	var syntax *json.SyntaxError
	if errors.As(err, &syntax) {
		return &JSONError{Kind: JSONSyntax, Offset: int(syntax.Offset), Message: syntax.Error()}
	}
	return &JSONError{Kind: JSONSyntax, Offset: -1, Message: err.Error()}
}

func (p *parser) failAt(offset int, kind JSONErrorKind, message string) error {
	return &JSONError{Kind: kind, Offset: offset, Message: message}
}

func (p *parser) value(depth int) (Value, error) {
	if err := p.charge(); err != nil {
		return nil, err
	}
	if depth > p.limits.MaxDepth {
		return nil, limitError("JSON nesting budget exceeded")
	}
	tok, err := p.dec.Token()
	if err != nil {
		return nil, p.fail(err)
	}
	if delim, ok := tok.(json.Delim); ok {
		if (delim == '{' || delim == '[') && depth >= p.limits.MaxDepth {
			return nil, limitError("JSON nesting budget exceeded")
		}
		switch delim {
		case '{':
			return p.object(depth)
		case '[':
			return p.array(depth)
		}
		return nil, p.failAt(int(p.dec.InputOffset()), JSONSyntax, "unexpected delimiter")
	}
	if number, ok := tok.(json.Number); ok {
		if len(number) > p.limits.MaxNumberLength {
			return nil, limitError("numeric token exceeds its character budget")
		}
		return Number{token: string(number)}, nil
	}
	return tok, nil // bool, string, or nil
}

func (p *parser) object(depth int) (Value, error) {
	object := map[string]Value{}
	for p.dec.More() {
		tok, err := p.dec.Token()
		if err != nil {
			return nil, p.fail(err)
		}
		name, ok := tok.(string)
		if !ok {
			return nil, p.failAt(int(p.dec.InputOffset()), JSONSyntax, "expected an object name")
		}
		if _, seen := object[name]; seen {
			return nil, p.failAt(int(p.dec.InputOffset()), JSONDuplicateName, "duplicate object name")
		}
		value, err := p.value(depth + 1)
		if err != nil {
			return nil, err
		}
		object[name] = value
	}
	if tok, err := p.dec.Token(); err != nil {
		return nil, p.fail(err)
	} else if tok != json.Delim('}') {
		return nil, p.failAt(int(p.dec.InputOffset()), JSONSyntax, "unterminated object")
	}
	return object, nil
}

func (p *parser) array(depth int) (Value, error) {
	items := []Value{}
	for p.dec.More() {
		value, err := p.value(depth + 1)
		if err != nil {
			return nil, err
		}
		items = append(items, value)
	}
	if tok, err := p.dec.Token(); err != nil {
		return nil, p.fail(err)
	} else if tok != json.Delim(']') {
		return nil, p.failAt(int(p.dec.InputOffset()), JSONSyntax, "unterminated array")
	}
	return items, nil
}

// encoder writes one JSON document.
type encoder struct {
	budget
	out []byte
	// path holds the identities of containers on the active encode path.
	path map[containerIdentity]struct{}
}

type containerIdentity struct {
	kind    reflect.Kind
	pointer uintptr
	length  int
}

// Encode writes v in the exact JSON domain: Numbers and Integers keep their
// exact tokens, and floats, structs, pointers, functions, and zero-value
// numbers fail with kind "type" instead of being coerced. Object names are
// sorted for deterministic output; cycles fail with kind "cycle" while the
// same acyclic container may appear any number of times. Errors are *JSONError.
func Encode(v Value, limits Limits) ([]byte, error) {
	if err := limits.check(false); err != nil {
		return nil, err
	}
	e := &encoder{budget: budget{limits: limits}, path: map[containerIdentity]struct{}{}}
	if err := e.value(v, 0); err != nil {
		return nil, err
	}
	return e.out, nil
}

func (e *encoder) value(v Value, depth int) error {
	if err := e.charge(); err != nil {
		return err
	}
	if depth > e.limits.MaxDepth {
		return limitError("JSON nesting budget exceeded")
	}
	if len(e.out) > e.limits.MaxOutputBytes {
		return limitError("output exceeds its byte budget")
	}
	switch x := v.(type) {
	case nil:
		return e.append([]byte("null")...)
	case bool:
		return e.append([]byte(strconv.FormatBool(x))...)
	case string:
		return e.text(x)
	case Number:
		return e.number(x.token)
	case Integer:
		return e.number(x.token)
	case []Value:
		if depth >= e.limits.MaxDepth {
			return limitError("JSON nesting budget exceeded")
		}
		return e.slice(x, depth)
	case map[string]Value:
		if depth >= e.limits.MaxDepth {
			return limitError("JSON nesting budget exceeded")
		}
		return e.object(x, depth)
	default:
		return &JSONError{Kind: JSONType, Offset: -1,
			Message: fmt.Sprintf("unsupported Go type %T", v)}
	}
}

func (e *encoder) append(bytes ...byte) error {
	if len(bytes) > e.limits.MaxOutputBytes-len(e.out) {
		return limitError("output exceeds its byte budget")
	}
	e.out = append(e.out, bytes...)
	return nil
}

// escapeText maps every byte that needs a short JSON escape to its suffix.
var escapeText = func() (table [256]string) {
	for i, escaped := range []string{`\b`, `\f`, `\n`, `\r`, `\t`, `\\`, `\"`} {
		table["\b\f\n\r\t\\\""[i]] = escaped
	}
	return table
}()

// text writes one JSON string; non-UTF-8 fails with kind "utf8" rather than
// being replaced, and output growth is bounded by worst-case expansion.
func (e *encoder) text(s string) error {
	if !utf8.ValidString(s) {
		return &JSONError{Kind: JSONBadUnicode, Offset: -1, Message: "string is not valid UTF-8"}
	}
	remaining := e.limits.MaxOutputBytes - len(e.out)
	if remaining < 2 {
		return limitError("output exceeds its byte budget")
	}
	size := 2
	for i := 0; i < len(s); i++ {
		width := 1
		if escaped := escapeText[s[i]]; escaped != "" {
			width = len(escaped)
		} else if s[i] < 0x20 {
			width = 6
		}
		if width > remaining-size {
			return limitError("output exceeds its byte budget")
		}
		size += width
	}
	out := append(e.out, '"')
	for i := 0; i < len(s); i++ {
		if escaped := escapeText[s[i]]; escaped != "" {
			out = append(out, escaped...)
		} else if s[i] < 0x20 {
			const hex = "0123456789abcdef"
			out = append(out, '\\', 'u', '0', '0', hex[s[i]>>4], hex[s[i]&0xF])
		} else {
			out = append(out, s[i])
		}
	}
	e.out = append(out, '"')
	return nil
}

// number writes one exact numeric token; the zero-value policy and the
// numeric budget both apply.
func (e *encoder) number(token string) error {
	if !validNumberToken(token) { // covers the zero value, which carries no token
		return &JSONError{Kind: JSONType, Offset: -1, Message: "numeric token is invalid or a zero-value number"}
	}
	if len(token) > e.limits.MaxNumberLength || len(e.out)+len(token) > e.limits.MaxOutputBytes {
		return limitError("output or numeric token budget exceeded")
	}
	e.out = append(e.out, token...)
	return nil
}

// enter records a non-empty container on the active path and returns its
// cleanup; a container already on the path is a cycle. Empty containers cannot
// cycle and stay untracked, so aliasing an empty slice is never a false
// positive. Slice identity includes length: a smaller view sharing storage
// can be a finite value even while its enclosing larger slice is active.
func (e *encoder) enter(v Value) (func(), error) {
	rv := reflect.ValueOf(v)
	switch rv.Kind() {
	case reflect.Slice, reflect.Map:
		if rv.Len() == 0 {
			return func() {}, nil
		}
		id := containerIdentity{kind: rv.Kind(), pointer: rv.Pointer()}
		if rv.Kind() == reflect.Slice {
			id.length = rv.Len()
		}
		if _, cyclic := e.path[id]; cyclic {
			return nil, &JSONError{Kind: JSONCycle, Offset: -1, Message: "cyclic JSON value"}
		}
		e.path[id] = struct{}{}
		return func() { delete(e.path, id) }, nil
	}
	return func() {}, nil
}

func (e *encoder) slice(items []Value, depth int) error {
	pop, err := e.enter(items)
	if err != nil {
		return err
	}
	defer pop()
	if len(items) > e.limits.MaxNodes-e.nodes {
		return limitError("JSON value budget exceeded")
	}
	if err := e.append('['); err != nil {
		return err
	}
	for i, item := range items {
		if i > 0 {
			if err := e.append(','); err != nil {
				return err
			}
		}
		if err := e.value(item, depth+1); err != nil {
			return err
		}
	}
	return e.append(']')
}

// object writes map keys in sorted byte order; each member name is charged
// against the node budget.
func (e *encoder) object(m map[string]Value, depth int) error {
	pop, err := e.enter(m)
	if err != nil {
		return err
	}
	defer pop()
	if len(m) > (e.limits.MaxNodes-e.nodes)/2 {
		return limitError("JSON value budget exceeded")
	}
	names := slices.Sorted(maps.Keys(m))
	if err := e.append('{'); err != nil {
		return err
	}
	for i, name := range names {
		if i > 0 {
			if err := e.append(','); err != nil {
				return err
			}
		}
		if err := e.charge(); err != nil {
			return err
		}
		if err := e.text(name); err != nil {
			return err
		}
		if err := e.append(':'); err != nil {
			return err
		}
		if err := e.value(m[name], depth+1); err != nil {
			return err
		}
	}
	return e.append('}')
}
