// Standard-library transport over source-backed protocol descriptors.
package sdk

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"reflect"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"
)

// HTTPSource is a canonical retrieval document and decoded JSON Pointer.
type HTTPSource struct{ Document, Pointer string }

// HTTPProvenance keeps a declaration separate from its referenced definition.
type HTTPProvenance struct {
	UseSite, Definition HTTPSource
	References          []HTTPSource
}

// SDKError is a public transport, representation, codec or resource failure.
// Error/Format omit captures and causes; inspect fields and Unwrap explicitly.
type SDKError struct {
	Kind              string
	Operation, Source HTTPSource
	Status            int
	Headers           http.Header
	RawCapture        []byte
	Truncated         bool
	Cause             error
}

func (e *SDKError) Error() string                 { return "SDK " + e.Kind + " failure" }
func (e *SDKError) Unwrap() error                 { return e.Cause }
func (e *SDKError) ResourceLimited() bool         { return e.Kind == "resource-limit" }
func (e *SDKError) Format(s fmt.State, verb rune) { _, _ = io.WriteString(s, e.Error()) }

// Doer injects ordinary net/http transport policy.
type Doer interface {
	Do(*http.Request) (*http.Response, error)
}

// ClientOptions controls explicit server/security selection and finite transport policy.
// Zero limits select generated defaults; positive limits may only lower them.
type ClientOptions struct {
	Transport       Doer
	ServerURL       string
	ServerIndex     int
	ServerVariables map[string]string
	// DocumentURL supplies an HTTP retrieval base for locally loaded relative servers.
	DocumentURL string
	// SecurityAlternative selects a source OR member. Nil chooses the first
	// satisfiable member in declaration order (including an anonymous member).
	SecurityAlternative                                                 *int
	MaxResponseBytes, MaxCaptureBytes, MaxPartBytes, MaxStreamItemBytes int
	// Timeout includes headers, complete body reading and stream iteration.
	Timeout time.Duration
	// UserAgent overrides the automatic ua/v1 attribution header. A non-nil
	// empty string suppresses the header entirely.
	UserAgent *string
	// ApplicationID replaces the SDK identity token in the automatic
	// attribution header: "<name>" or "<name>/<version>" (RFC 9110 tokens).
	ApplicationID string
}

// Client is reusable concurrently when its supplied transport and hooks are.
type Client struct {
	transport   Doer
	credentials Credentials
	options     ClientOptions
}

// ua/v1 application identity: `<name>` or `<name>/<version>` of RFC 9110 tokens.
var userAgentIdentityPattern = regexp.MustCompile("^[A-Za-z0-9!#$%&'*+.^`|~-]+(?:/[A-Za-z0-9!#$%&'*+.^`|~-]+)?$")

// resolveUserAgent assembles the ua/v1 attribution header. Explicit overrides
// win; a non-nil empty string suppresses; the default identifies suspect as the
// generator and the SDK package or a caller-supplied application as the client.
func (c *Client) resolveUserAgent() (string, bool) {
	if c.options.UserAgent != nil {
		if *c.options.UserAgent == "" {
			return "", false
		}
		return *c.options.UserAgent, true
	}
	if userAgentSuspectVersion == "" {
		return "", false
	}
	identity := userAgentSDKName + "/" + userAgentSDKVersion
	if c.options.ApplicationID != "" {
		if len(c.options.ApplicationID) > 128 || !userAgentIdentityPattern.MatchString(c.options.ApplicationID) {
			return "", false
		}
		identity = c.options.ApplicationID
	}
	return "suspect/" + userAgentSuspectVersion + " " + identity + " (go/" + strings.TrimPrefix(runtime.Version(), "go") + "; openapi/" + userAgentSpecVersion + ")", true
}

func NewClient(credentials Credentials, options ClientOptions) (*Client, error) {
	if options.ServerIndex < 0 || options.MaxResponseBytes < 0 || options.MaxCaptureBytes < 0 || options.MaxPartBytes < 0 || options.MaxStreamItemBytes < 0 || options.Timeout < 0 || options.SecurityAlternative != nil && *options.SecurityAlternative < 0 {
		return nil, &SDKError{Kind: "request-representation"}
	}
	if options.ServerURL != "" {
		if _, err := httpBaseURL(options.ServerURL); err != nil {
			return nil, err
		}
		if options.ServerIndex != 0 || len(options.ServerVariables) != 0 {
			return nil, &SDKError{Kind: "request-representation", Cause: errors.New("server override conflicts with candidate selection")}
		}
	}
	if options.DocumentURL != "" {
		u, err := url.Parse(options.DocumentURL)
		if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil {
			return nil, &SDKError{Kind: "request-representation"}
		}
	}
	options.ServerVariables = httpCloneMap(options.ServerVariables)
	if options.SecurityAlternative != nil {
		index := *options.SecurityAlternative
		options.SecurityAlternative = &index
	}
	if options.Transport == nil {
		transport := &http.Transport{Proxy: nil, DialContext: (&net.Dialer{Timeout: 30 * time.Second, KeepAlive: 30 * time.Second}).DialContext, ForceAttemptHTTP2: true, MaxIdleConns: 100, IdleConnTimeout: 90 * time.Second, TLSHandshakeTimeout: 10 * time.Second, ExpectContinueTimeout: time.Second, DisableCompression: true}
		options.Transport = &http.Client{Transport: transport, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	}
	return &Client{transport: options.Transport, credentials: Credentials{values: httpCloneMap(credentials.values)}, options: options}, nil
}
func (c *Client) CloseIdleConnections() {
	if closer, ok := c.transport.(interface{ CloseIdleConnections() }); ok {
		closer.CloseIdleConnections()
	}
}
func httpCloneMap[K comparable, V any](v map[K]V) map[K]V {
	if v == nil {
		return nil
	}
	out := make(map[K]V, len(v))
	for k, v := range v {
		out[k] = v
	}
	return out
}

// NoContent represents content forbidden by HTTP (HEAD, 1xx, 204 and 304).
type NoContent struct{}

// Content supplies an explicit concrete Content-Type and a typed value.
type Content[T any] struct {
	ContentType string
	Data        T
}

// Part is a finite in-memory MIME part. Filename is metadata, never a path to read.
// Headers are checked against declared part-header codecs before transport.
type Part[T any] struct {
	Data                  T
	ContentType, Filename string
	Headers               http.Header
}

func NewPart[T any](data T) Part[T]                    { return Part[T]{Data: data} }
func (v Part[T]) WithContentType(value string) Part[T] { v.ContentType = value; return v }
func (v Part[T]) WithFilename(value string) Part[T]    { v.Filename = value; return v }
func (v Part[T]) WithHeader(name, value string) Part[T] {
	v.Headers = v.Headers.Clone()
	if v.Headers == nil {
		v.Headers = make(http.Header)
	}
	v.Headers.Set(name, value)
	return v
}

// LocatedValue retains a literal or runtime-expression JSON value as metadata.
type LocatedValue struct {
	Source HTTPSource
	JSON   json.RawMessage
}

// Link describes a source Link Object. It never invokes another operation.
type Link struct {
	Name                      string
	Source                    HTTPProvenance
	OperationID, OperationRef string
	Target                    HTTPSource
	Parameters                map[string]LocatedValue
	RequestBody               *LocatedValue
	Description               string
	Server                    *Server
}

// APIResponse exposes actual HTTP status, validated data and source link metadata.
type APIResponse[T any] struct {
	Status      int
	Headers     http.Header
	ContentType string
	Data        T
	Links       []Link
}

// Close releases an iterator carried by a success or declared-error response.
// Buffered responses have no remaining transport resource.
func (r APIResponse[T]) Close() error {
	value := reflect.ValueOf(r.Data)
	if !value.IsValid() || value.Kind() == reflect.Pointer && value.IsNil() {
		return nil
	}
	if closer, ok := any(r.Data).(io.Closer); ok {
		return closer.Close()
	}
	return nil
}

type httpOperation struct {
	ID, Method, Path                          string
	Source                                    HTTPSource
	MaxRequest, MaxResponse, MaxPart, MaxItem int
	Servers                                   []Server
	Security                                  []SecurityAlternative
	Parameters                                []httpParameterPlan
	Body                                      []httpMediaPlan
	Responses                                 []httpResponsePlan
}
type httpSerial struct {
	Content, Style, Shape, Scalar, Additional, Encoding string
	Explode                                             bool
	Properties                                          map[string]string
}
type httpParameterPlan struct {
	Name, Location, Codec string
	Required              bool
	Source                HTTPSource
	Serial                httpSerial
	QueryForm             *httpAggregatePlan
}
type httpHeaderPlan struct {
	Name, Codec string
	Required    bool
	Source      HTTPSource
	Serial      httpSerial
}
type httpMediaType struct {
	Declared, Type, Subtype string
	Rank                    int
	Parameters              map[string]string
}
type httpMediaPlan struct {
	Source                       HTTPSource
	Media                        httpMediaType
	Kind, Codec, Scalar, Framing string
	MaxBytes, MaxItemBytes       int
	Aggregate                    *httpAggregatePlan
}
type httpResponsePlan struct {
	Status  string
	Source  HTTPSource
	Media   []httpMediaPlan
	Headers []httpHeaderPlan
	Links   []Link
}
type httpAggregatePlan struct {
	MaxParts              int
	Type                  string
	Positional, Multipart bool
	Parts                 []httpPartPlan
	Additional            *httpPartPlan
	Min, Max              *uint64
	Required              []string
	Source                HTTPSource
}
type httpPartPlan struct {
	Name, Field, Kind, Codec, Scalar, Encoding string
	Required, Repeated                         bool
	Source                                     HTTPSource
	Min, Max                                   *uint64
	Media                                      []httpMediaType
	Headers                                    []httpHeaderPlan
	Serial                                     httpSerial
	MaxBytes                                   int
}
type httpParameter struct {
	index int
	value Value
}
type httpBody struct {
	data        []byte
	contentType string
	present     bool
}
type httpValueCodec struct {
	encode func(any) (Value, error)
	decode func(Value) (any, error)
}

func httpBind[T any](codec Codec[T]) httpValueCodec {
	return httpValueCodec{encode: func(v any) (Value, error) {
		value, ok := v.(T)
		if !ok {
			if v == nil {
				var zero T
				value = zero
			} else {
				return nil, errors.New("invalid native codec input")
			}
		}
		return codec.EncodeValue(value)
	}, decode: func(v Value) (any, error) { return codec.DecodeValue(v) }}
}
func httpLoadOperation(text string) httpOperation {
	var result httpOperation
	if err := json.Unmarshal([]byte(text), &result); err != nil {
		panic(err)
	}
	return result
}
func httpCopy[T any](value T) T {
	data, err := json.Marshal(value)
	if err != nil {
		panic(err)
	}
	var result T
	if err = json.Unmarshal(data, &result); err != nil {
		panic(err)
	}
	return result
}
func httpNative[T any](v any) (T, error) {
	var zero T
	if v == nil {
		return zero, nil
	}
	value, ok := v.(T)
	if !ok {
		return zero, errors.New("invalid native representation")
	}
	return value, nil
}
func httpOptional[T any](v Optional[T], op httpOperation, at HTTPSource) error {
	if !v.IsSet && !reflect.ValueOf(&v.Value).Elem().IsZero() {
		return httpError("request-validation", op, at, errors.New("inconsistent absent wrapper"))
	}
	return nil
}
func httpFailureKind(kind string, cause error) string {
	if errors.Is(cause, context.Canceled) || errors.Is(cause, context.DeadlineExceeded) {
		kind = "cancelled"
	}
	var j *JSONError
	if errors.As(cause, &j) && j.Kind == JSONLimit {
		kind = "resource-limit"
	}
	var resource interface{ ResourceLimited() bool }
	if errors.As(cause, &resource) && resource.ResourceLimited() {
		kind = "resource-limit"
	}
	return kind
}
func httpError(kind string, op httpOperation, at HTTPSource, cause error) *SDKError {
	var located *SDKError
	if errors.As(cause, &located) && located.Source.Document != "" {
		at = located.Source
	}
	return &SDKError{Kind: httpFailureKind(kind, cause), Operation: op.Source, Source: at, Cause: cause}
}
func httpProblem(kind string) error { return &SDKError{Kind: kind} }
func httpCause(ctx context.Context, err error) error {
	if cause := ctx.Err(); cause != nil && !errors.Is(err, cause) {
		return errors.Join(err, cause)
	}
	return err
}

// An owned body has one close/cancel owner. Context cancellation interrupts both
// the standard transport and injected ReadClosers that unblock on Close.
type httpOwnedBody struct {
	body   io.ReadCloser
	ctx    context.Context
	cancel context.CancelFunc
	stop   func() bool
	once   sync.Once
	err    error
}

func (o *httpOwnedBody) close() error {
	o.once.Do(func() { o.cancel(); o.err = o.body.Close() })
	return o.err
}
func (o *httpOwnedBody) finish() error {
	if o.stop != nil {
		o.stop()
	}
	return o.close()
}

type rawHTTPResponse struct {
	status, response, media                   int
	headers                                   http.Header
	body                                      []byte
	source                                    HTTPSource
	contentType                               string
	maxCapture, maximum, itemLimit, partLimit int
	forbidden, taken                          bool
	owned                                     *httpOwnedBody
}

func (r *rawHTTPResponse) release() {
	if !r.taken {
		_ = r.owned.finish()
	}
}
func (r *rawHTTPResponse) fail(kind string, at HTTPSource, cause error) *SDKError {
	var located *SDKError
	if errors.As(cause, &located) && located.Source.Document != "" {
		at = located.Source
	}
	limit := len(r.body)
	if limit > r.maxCapture {
		limit = r.maxCapture
	}
	return &SDKError{Kind: httpFailureKind(kind, cause), Operation: r.source, Source: at, Status: r.status, Headers: r.headers.Clone(), RawCapture: append([]byte(nil), r.body[:limit]...), Truncated: limit < len(r.body), Cause: cause}
}
func httpBaseURL(value string) (*url.URL, error) {
	if value == "" || strings.ContainsAny(value, "?#\\{}") || strings.IndexFunc(value, func(r rune) bool { return unicode.IsSpace(r) || unicode.IsControl(r) }) >= 0 || !utf8.ValidString(value) {
		return nil, httpProblem("request-representation")
	}
	u, err := url.Parse(value)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || u.User != nil || u.ForceQuery {
		return nil, httpProblem("request-representation")
	}
	return u, nil
}
func (c *Client) server(op httpOperation) (string, error) {
	if c.options.ServerURL != "" {
		return c.options.ServerURL, nil
	}
	if c.options.ServerIndex >= len(op.Servers) {
		return "", httpError("request-representation", op, op.Source, errors.New("unknown server candidate"))
	}
	s := op.Servers[c.options.ServerIndex]
	at := op.Source
	if s.Source != nil {
		at = s.Source.UseSite
	} else if s.DefaultFrom != nil {
		at = *s.DefaultFrom
	}
	variables := make(map[string]string, len(s.Variables))
	for _, v := range s.Variables {
		variables[v.Name] = v.Default
	}
	for name, value := range c.options.ServerVariables {
		if _, ok := variables[name]; !ok {
			return "", httpError("request-representation", op, at, errors.New("undeclared server variable"))
		}
		variables[name] = value
	}
	for _, v := range s.Variables {
		if v.Values != nil {
			found := false
			for _, allowed := range v.Values {
				found = found || allowed == variables[v.Name]
			}
			if !found {
				return "", httpError("request-representation", op, v.Source.UseSite, errors.New("server variable outside enum"))
			}
		}
	}
	var expanded strings.Builder
	rest := s.URL
	for {
		index := strings.IndexByte(rest, '{')
		if index < 0 {
			if expanded.Len()+len(rest) > op.MaxRequest {
				return "", httpError("resource-limit", op, at, nil)
			}
			expanded.WriteString(rest)
			break
		}
		tail := rest[index+1:]
		close := strings.IndexByte(tail, '}')
		if close < 0 {
			return "", httpError("request-representation", op, at, nil)
		}
		value, ok := variables[tail[:close]]
		if !ok {
			return "", httpError("request-representation", op, at, nil)
		}
		if expanded.Len()+index+len(value) > op.MaxRequest {
			return "", httpError("resource-limit", op, at, nil)
		}
		expanded.WriteString(rest[:index])
		expanded.WriteString(value)
		rest = tail[close+1:]
	}
	text := expanded.String()
	if strings.ContainsAny(text, "?#\\{}") || strings.IndexFunc(text, func(r rune) bool { return unicode.IsSpace(r) || unicode.IsControl(r) }) >= 0 {
		return "", httpError("request-representation", op, at, nil)
	}
	u, err := url.Parse(text)
	if err != nil {
		return "", httpError("request-representation", op, at, err)
	}
	if !u.IsAbs() {
		base := c.options.DocumentURL
		if base == "" {
			base = s.DocumentBase.Document
		}
		b, e := url.Parse(base)
		if e != nil || (b.Scheme != "http" && b.Scheme != "https") || b.Host == "" || b.User != nil {
			return "", httpError("request-representation", op, at, errors.New("relative server requires an HTTP document URL"))
		}
		u = b.ResolveReference(u)
	}
	if _, err = httpBaseURL(u.String()); err != nil {
		return "", httpError("request-representation", op, at, err)
	}
	return u.String(), nil
}
func httpURL(op httpOperation, base string, parameters []httpParameter) (string, http.Header, error) {
	u, err := httpBaseURL(base)
	if err != nil {
		return "", nil, httpError("request-representation", op, op.Source, err)
	}
	route := op.Path
	queries := []string{}
	cookies := []string{}
	headers := make(http.Header)
	for _, p := range parameters {
		descriptor := op.Parameters[p.index]
		var wire string
		var err error
		if descriptor.QueryForm != nil {
			wire, err = httpEncodeQueryForm(descriptor.QueryForm, p.value, op.MaxRequest, op.MaxPart)
		} else {
			wire, err = httpSerialize(descriptor.Name, descriptor.Location, descriptor.Serial, p.value, op.MaxRequest)
		}
		if err != nil {
			return "", nil, httpError("request-representation", op, descriptor.Source, err)
		}
		switch descriptor.Location {
		case "path":
			marker := "{" + descriptor.Name + "}"
			count := strings.Count(route, marker)
			if count == 0 {
				return "", nil, httpError("request-representation", op, descriptor.Source, nil)
			}
			remaining := op.MaxRequest - (len(route) - len(marker)*count)
			if remaining < 0 || len(wire) > remaining/count {
				return "", nil, httpError("resource-limit", op, descriptor.Source, nil)
			}
			route = strings.ReplaceAll(route, marker, wire)
		case "query":
			queries = append(queries, wire)
		case "querystring":
			queries = append(queries, wire)
		case "header":
			headers.Add(descriptor.Name, wire)
		case "cookie":
			cookies = append(cookies, wire)
		}
	}
	uri := u.Scheme + "://" + u.Host + strings.TrimSuffix(u.EscapedPath(), "/") + route
	if len(queries) > 0 {
		uri += "?" + strings.Join(queries, "&")
	}
	if len(cookies) > 0 {
		existing := httpHeaderValues(headers, "Cookie")
		combined := append(append([]string(nil), existing...), cookies...)
		names := map[string]int{}
		for group, fragment := range combined {
			request := &http.Request{Header: http.Header{"Cookie": {fragment}}}
			for _, cookie := range request.Cookies() {
				if previous, exists := names[cookie.Name]; exists && previous != group {
					return "", nil, httpError("request-representation", op, op.Source, errors.New("conflicting cookie parameters"))
				}
				names[cookie.Name] = group
			}
		}
		headers.Set("Cookie", strings.Join(combined, "; "))
	}
	if len(uri) > op.MaxRequest {
		return "", nil, httpError("resource-limit", op, op.Source, nil)
	}
	parsed, e := url.Parse(uri)
	if e != nil {
		return "", nil, httpError("request-representation", op, op.Source, e)
	}
	for _, segment := range strings.Split(parsed.EscapedPath(), "/") {
		if segment == "." || segment == ".." {
			return "", nil, httpError("request-representation", op, op.Source, errors.New("ambiguous path segment"))
		}
	}
	return uri, headers, nil
}
func (c *Client) exchange(ctx context.Context, op httpOperation, parameters []httpParameter, body httpBody) (raw *rawHTTPResponse, err error) {
	if ctx == nil {
		return nil, httpError("request-validation", op, op.Source, nil)
	}
	var cancel context.CancelFunc
	if c.options.Timeout > 0 {
		ctx, cancel = context.WithTimeout(ctx, c.options.Timeout)
	} else {
		ctx, cancel = context.WithCancel(ctx)
	}
	handed := false
	defer func() {
		if !handed {
			cancel()
		}
	}()
	if err = ctx.Err(); err != nil {
		return nil, httpError("cancelled", op, op.Source, err)
	}
	maximum := op.MaxResponse
	partLimit := op.MaxPart
	itemLimit := op.MaxItem
	for _, p := range []struct {
		value  int
		target *int
	}{{c.options.MaxResponseBytes, &maximum}, {c.options.MaxPartBytes, &partLimit}, {c.options.MaxStreamItemBytes, &itemLimit}} {
		if p.value > 0 {
			if p.value > *p.target {
				return nil, httpError("resource-limit", op, op.Source, nil)
			}
			*p.target = p.value
		}
	}
	if len(body.data) > op.MaxRequest {
		return nil, httpError("resource-limit", op, op.Source, nil)
	}
	capture := c.options.MaxCaptureBytes
	if capture == 0 {
		capture = 4096
	}
	if capture > maximum {
		capture = maximum
	}
	base, e := c.server(op)
	if e != nil {
		return nil, e
	}
	uri, headers, e := httpURL(op, base, parameters)
	if e != nil {
		return nil, e
	}
	var reader io.Reader
	if body.present {
		reader = bytes.NewReader(body.data)
	}
	request, e := http.NewRequestWithContext(ctx, op.Method, uri, reader)
	if e != nil {
		return nil, httpError("request-representation", op, op.Source, e)
	}
	request.GetBody = nil
	request.Header = headers
	if hosts := httpHeaderValues(headers, "Host"); len(hosts) > 0 {
		if len(hosts) != 1 {
			return nil, httpError("request-representation", op, op.Source, errors.New("one Host value is required"))
		}
		host, e := httpBaseURL("http://" + hosts[0])
		if e != nil || host.Host != hosts[0] || host.Path != "" {
			return nil, httpError("request-representation", op, op.Source, errors.New("invalid Host parameter"))
		}
		request.Host = hosts[0]
	}
	var accept []string
	seen := map[string]bool{}
	for _, r := range op.Responses {
		for _, m := range r.Media {
			if !seen[m.Media.Declared] {
				accept = append(accept, m.Media.Declared)
				seen[m.Media.Declared] = true
			}
		}
	}
	if len(accept) > 0 {
		request.Header.Set("Accept", strings.Join(accept, ", "))
	}
	request.Header.Set("Accept-Encoding", "identity")
	// ua/v1 attribution is applied after declared parameters so an explicit
	// caller-supplied User-Agent header keeps precedence over the default.
	if userAgent, ok := c.resolveUserAgent(); ok && len(httpHeaderValues(headers, "User-Agent")) == 0 {
		request.Header.Set("User-Agent", userAgent)
	}
	if body.present {
		request.Header.Set("Content-Type", body.contentType)
	}
	if e = c.authorize(ctx, op, request, base); e != nil {
		return nil, e
	}
	if len(request.URL.String())+httpHeaderBytes(request.Header) > op.MaxRequest {
		return nil, httpError("resource-limit", op, op.Source, nil)
	}
	response, e := c.transport.Do(request)
	e = httpCause(ctx, e)
	if e != nil {
		if response != nil && response.Body != nil {
			_ = response.Body.Close()
		}
		return nil, httpError("transport", op, op.Source, e)
	}
	if response == nil || response.Body == nil {
		return nil, httpError("transport", op, op.Source, errors.New("transport returned no response body"))
	}
	owned := &httpOwnedBody{body: response.Body, ctx: ctx, cancel: cancel}
	owned.stop = context.AfterFunc(ctx, func() { _ = owned.close() })
	raw = &rawHTTPResponse{status: response.StatusCode, headers: response.Header.Clone(), source: op.Source, maxCapture: capture, maximum: maximum, itemLimit: itemLimit, partLimit: partLimit, owned: owned, media: -1, response: -1}
	handed = true
	complete := false
	defer func() {
		if !complete {
			raw.release()
		}
	}()
	selectionErr := raw.selectResponse(op)
	if selectionErr == nil && !raw.forbidden && raw.media >= 0 && op.Responses[raw.response].Media[raw.media].Kind == "stream" {
		complete = true
		return raw, nil
	}
	if raw.forbidden {
		if selectionErr != nil {
			return raw, raw.fail("unexpected-response", op.Source, selectionErr)
		}
		complete = true
		_ = owned.finish()
		return raw, nil
	}
	raw.body, e = io.ReadAll(io.LimitReader(response.Body, int64(maximum)+1))
	e = httpCause(ctx, e)
	if len(raw.body) > maximum {
		failure := raw.fail("resource-limit", op.Source, nil)
		failure.Truncated = true
		return raw, failure
	}
	if e != nil {
		failure := raw.fail("transport", op.Source, e)
		failure.Truncated = true
		return raw, failure
	}
	if selectionErr != nil {
		return raw, raw.fail("unexpected-response", op.Source, selectionErr)
	}
	complete = true
	_ = owned.finish()
	return raw, nil
}
func httpHeaderBytes(h http.Header) int {
	n := 0
	for k, values := range h {
		for _, v := range values {
			n += len(k) + len(v) + 4
		}
	}
	return n
}
