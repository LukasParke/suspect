package sdk

import (
	"context"
	"encoding/base64"
	"errors"
	"net/http"
	"net/url"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Server is one source-declared candidate; variables are literal substitutions.
type Server struct {
	URL, Name, Description string
	Source                 *HTTPProvenance
	DefaultFrom            *HTTPSource
	DocumentBase           HTTPSource
	Variables              []ServerVariable
}
type ServerVariable struct {
	Name, Default, Description string
	Values                     []string
	Source                     HTTPProvenance
}

// OAuthFlow is source metadata passed to a caller-owned credential hook.
type OAuthFlow struct {
	URLBase                                                              string
	Source                                                               HTTPSource
	Kind, AuthorizationURL, TokenURL, RefreshURL, DeviceAuthorizationURL string
	Scopes                                                               map[string]string
}
type SecurityRequirement struct {
	URLBase                                string
	Name, Kind, Location, WireName         string
	Source                                 HTTPSource
	Scheme                                 HTTPProvenance
	Scopes, Roles                          []string
	Flows                                  []OAuthFlow
	MetadataURL, DiscoveryURL, Description string
}
type SecurityAlternative struct {
	Source       HTTPSource
	Requirements []SecurityRequirement
}

// CredentialRequest preserves scope/role, flow and definition metadata.
type CredentialRequest struct {
	// ServerURL is the selected effective API server for relative OAuth/OIDC URLs.
	ServerURL   string
	Operation   HTTPSource
	Requirement SecurityRequirement
}

// Authorization is an explicit HTTP auth scheme and credential. No token type is inferred.
type Authorization struct{ Scheme, Value string }

// CredentialHook supplies caller-managed OAuth/OIDC credentials for this exchange.
// It performs no automatic discovery, acquisition, refresh or retry.
type CredentialHook func(context.Context, CredentialRequest) (Authorization, error)
type httpCredential struct {
	kind, value, user, password string
	authorization               Authorization
	hook                        CredentialHook
}

// Credentials is an immutable-by-construction collection keyed by source scheme name.
type Credentials struct{ values map[string]httpCredential }

func (c Credentials) with(scheme string, v httpCredential) Credentials {
	values := httpCloneMap(c.values)
	if values == nil {
		values = make(map[string]httpCredential)
	}
	values[scheme] = v
	return Credentials{values: values}
}
func (c Credentials) WithBearer(scheme, token string) Credentials {
	return c.with(scheme, httpCredential{kind: "bearer", value: token})
}
func (c Credentials) WithBasic(scheme, user, password string) Credentials {
	return c.with(scheme, httpCredential{kind: "basic", user: user, password: password})
}
func (c Credentials) WithAPIKey(scheme, value string) Credentials {
	return c.with(scheme, httpCredential{kind: "api-key", value: value})
}
func (c Credentials) WithAuthorization(scheme string, value Authorization) Credentials {
	return c.with(scheme, httpCredential{kind: "authorization", authorization: value})
}
func (c Credentials) WithHook(scheme string, hook CredentialHook) Credentials {
	return c.with(scheme, httpCredential{kind: "authorization", hook: hook})
}
func validBearer(token string) bool {
	if token == "" || token[0] == '=' {
		return false
	}
	padding := false
	for _, c := range token {
		if c == '=' {
			padding = true
			continue
		}
		if padding || !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("-._~+/", c)) {
			return false
		}
	}
	return true
}
func httpHeaderValue(v string) bool {
	if !utf8.ValidString(v) {
		return false
	}
	for _, c := range v {
		if c < 32 && c != '\t' || c == 127 {
			return false
		}
	}
	return true
}
func httpToken(v string) bool {
	if v == "" {
		return false
	}
	for i := 0; i < len(v); i++ {
		c := v[i]
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("!#$%&'*+-.^_`|~", rune(c))) {
			return false
		}
	}
	return true
}
func httpCookieValue(value string) bool {
	for i := 0; i < len(value); i++ {
		b := value[i]
		if b < 0x21 || b > 0x7e || strings.ContainsRune("\",;\\", rune(b)) {
			return false
		}
	}
	return true
}
func (c *Client) authorize(ctx context.Context, op httpOperation, r *http.Request, serverURL string) error {
	if len(op.Security) == 0 {
		return nil
	}
	selected := -1
	if c.options.SecurityAlternative != nil {
		selected = *c.options.SecurityAlternative
		if selected >= len(op.Security) {
			return httpError("request-validation", op, op.Source, errors.New("unknown security alternative"))
		}
	} else {
		for index, a := range op.Security {
			available := true
			for _, requirement := range a.Requirements {
				value, ok := c.credentials.values[requirement.Name]
				kind := requirement.Kind
				if kind == "oauth2" || kind == "openid-connect" {
					kind = "authorization"
				}
				available = available && ok && value.kind == kind
			}
			if available {
				selected = index
				break
			}
		}
	}
	if selected < 0 {
		at := op.Source
		if len(op.Security) > 0 && len(op.Security[0].Requirements) > 0 {
			at = op.Security[0].Requirements[0].Scheme.Definition
		}
		return httpError("request-validation", op, at, errors.New("no complete security alternative supplied"))
	}
	attachHeader := func(name, value string) error {
		if len(name) > op.MaxRequest || len(value) > op.MaxRequest {
			return httpProblem("resource-limit")
		}
		if !httpToken(name) || !httpHeaderValue(value) {
			return errors.New("invalid credential header")
		}
		if len(httpHeaderValues(r.Header, name)) != 0 {
			return errors.New("conflicting credential attachment")
		}
		r.Header.Set(name, value)
		return nil
	}
	for _, required := range op.Security[selected].Requirements {
		at := required.Scheme.Definition
		failure := func(err error) error { return httpError("request-validation", op, at, err) }
		value, ok := c.credentials.values[required.Name]
		if !ok {
			return failure(errors.New("missing credential"))
		}
		if len(value.value) > op.MaxRequest || len(value.user) > op.MaxRequest || len(value.password) > op.MaxRequest {
			return httpError("resource-limit", op, at, nil)
		}
		switch required.Kind {
		case "bearer":
			if value.kind != "bearer" || !validBearer(value.value) {
				return failure(errors.New("invalid bearer credential"))
			}
			if err := attachHeader("Authorization", "Bearer "+value.value); err != nil {
				return failure(err)
			}
		case "basic":
			if value.kind != "basic" || strings.Contains(value.user, ":") || !httpHeaderValue(value.user) || !httpHeaderValue(value.password) || strings.IndexFunc(value.user, unicode.IsControl) >= 0 || strings.IndexFunc(value.password, unicode.IsControl) >= 0 {
				return failure(errors.New("invalid basic credential"))
			}
			if len(value.user)+len(value.password) > op.MaxRequest {
				return httpError("resource-limit", op, at, nil)
			}
			if err := attachHeader("Authorization", "Basic "+base64.StdEncoding.EncodeToString([]byte(value.user+":"+value.password))); err != nil {
				return failure(err)
			}
		case "api-key":
			if value.kind != "api-key" || value.value == "" || !httpHeaderValue(value.value) {
				return failure(errors.New("invalid API key"))
			}
			switch required.Location {
			case "header":
				if err := attachHeader(required.WireName, value.value); err != nil {
					return failure(err)
				}
			case "query":
				query, err := url.ParseQuery(r.URL.RawQuery)
				if err != nil {
					return failure(err)
				}
				if query.Has(required.WireName) {
					return failure(errors.New("conflicting query credential"))
				}
				for _, p := range op.Parameters {
					if p.Location == "querystring" {
						return failure(errors.New("query credential conflicts with complete querystring parameter"))
					}
				}
				name, err := httpPercent(required.WireName, "uri", op.MaxRequest)
				if err != nil {
					return failure(err)
				}
				text, err := httpPercent(value.value, "uri", op.MaxRequest)
				if err != nil {
					return failure(err)
				}
				if r.URL.RawQuery != "" {
					r.URL.RawQuery += "&"
				}
				r.URL.RawQuery += name + "=" + text
			case "cookie":
				for _, existing := range r.Cookies() {
					if existing.Name == required.WireName {
						return failure(errors.New("conflicting cookie credential"))
					}
				}
				// API key cookie attachment uses percent-escaped cookie data, without
				// net/http's lossy sanitizer or treating a credential as a quoted header.
				name, err := httpPercent(required.WireName, "uri", op.MaxRequest)
				if err != nil {
					return failure(err)
				}
				text, err := httpPercent(value.value, "uri", op.MaxRequest)
				if err != nil {
					return failure(err)
				}
				cookie := r.Header.Get("Cookie")
				if cookie != "" {
					cookie += "; "
				}
				r.Header.Set("Cookie", cookie+name+"="+text)
			}
		case "oauth2", "openid-connect":
			if value.kind != "authorization" {
				return failure(errors.New("caller authorization required"))
			}
			authorization := value.authorization
			if value.hook != nil {
				var err error
				authorization, err = value.hook(ctx, CredentialRequest{Operation: op.Source, Requirement: httpCopy(required), ServerURL: serverURL})
				if err = httpCause(ctx, err); err != nil {
					return failure(err)
				}
			}
			if len(authorization.Scheme) > op.MaxRequest || len(authorization.Value) > op.MaxRequest {
				return httpError("resource-limit", op, at, nil)
			}
			if !httpToken(authorization.Scheme) || authorization.Value == "" || !httpHeaderValue(authorization.Value) {
				return failure(errors.New("invalid caller authorization"))
			}
			if err := attachHeader("Authorization", authorization.Scheme+" "+authorization.Value); err != nil {
				return failure(err)
			}
		default:
			return failure(errors.New("unsupported credential attachment"))
		}
	}
	return nil
}
func httpHeaderValues(h http.Header, name string) []string {
	var out []string
	for k, values := range h {
		if strings.EqualFold(k, name) {
			out = append(out, values...)
		}
	}
	return out
}
