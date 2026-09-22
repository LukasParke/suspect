//! Emitted first-party OAuth lifecycle: conditional generation plus native
//! behavior.
//!
//! The configured policy adds exactly one file (`go/oauth.go`) with the
//! compiled scheme descriptors; generation without the policy — or a scheme
//! whose only declared flows are the deprecated implicit and password grants —
//! adds nothing, so no-policy output stays byte-identical. The native test
//! builds the emitted module and drives the lifecycle through a real
//! `httptest.NewServer` behind a URL-rewriting `Doer`, because token requests
//! use the client's own transport.
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn service_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth service","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"service":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"service":{
            "type":"oauth2",
            "flows":{
                "clientCredentials":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "refreshUrl":"https://auth.oauth.test/token-refresh",
                    "scopes":{"read":"Read access"}
                }
            }
        }}}
    })
}

/// The same shape without the OAuth scheme: the no-scheme control.
fn control_document() -> Value {
    let mut document = service_document();
    document["paths"]["/widgets"]["get"]
        .as_object_mut()
        .unwrap()
        .remove("security");
    document["components"]
        .as_object_mut()
        .unwrap()
        .remove("securitySchemes");
    document
}

/// A scheme whose only declared flows are the deprecated implicit and password
/// grants: represented by the plan, never executed.
fn deprecated_only_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth deprecated","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"legacy":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"legacy":{
            "type":"oauth2",
            "flows":{
                "implicit":{
                    "authorizationUrl":"https://auth.oauth.test/implicit-authorize",
                    "scopes":{"read":"Read access"}
                },
                "password":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }
            }
        }}}
    })
}

/// Authorization-code, device and implicit flows on one scheme. The device
/// flow is an OAS 3.2 declaration, so this document is 3.2.
fn interactive_document() -> Value {
    json!({
        "openapi":"3.2.0","info":{"title":"OAuth interactive","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"userAuth":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"userAuth":{
            "type":"oauth2",
            "flows":{
                "authorizationCode":{
                    "authorizationUrl":"https://auth.oauth.test/authorize",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "refreshUrl":"https://auth.oauth.test/token-refresh",
                    "scopes":{"read":"Read access"}
                },
                "deviceAuthorization":{
                    "deviceAuthorizationUrl":"https://auth.oauth.test/device",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                },
                "implicit":{
                    "authorizationUrl":"https://auth.oauth.test/implicit-authorize",
                    "scopes":{"read":"Read access"}
                }
            }
        }}}
    })
}

fn contract(document: &Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.oauth.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn generate(document: &Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract(document);
    generate_with_options(
        contract.clone(),
        &contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>(),
        &TargetConfig {
            backend: Backend::GoHttp,
            package_name: "example.com/oauth-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"service":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                    "revocation_endpoint":"https://auth.oauth.test/revoke",
                    "introspection_endpoint":"https://auth.oauth.test/introspect"
                }}}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn interactive_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"userAuth":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET"
                }}}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn deprecated_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"legacy":{"client_id_env":"OAUTH_SDK_CLIENT_ID"}}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

#[test]
fn configured_policy_emits_only_oauth_go() {
    let mut configured = generate(&service_document(), &configured_options());
    let mut plain = generate(&service_document(), &GenerationOptions::default());
    let mut control = generate(&control_document(), &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut plain);
    sorted(&mut control);
    assert!(
        !plain.iter().any(|file| file.path == "go/oauth.go"),
        "no-policy output must not carry the OAuth lifecycle runtime"
    );
    assert!(
        !control.iter().any(|file| file.path == "go/oauth.go"),
        "the control document must not carry the OAuth lifecycle runtime"
    );
    assert_eq!(
        plain
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        control
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        "the control document must not add or remove artifact paths"
    );
    assert_eq!(
        configured.len(),
        plain.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let oauth = configured
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    for expected in [
        "type TokenSet struct",
        "func (t *TokenSet) Authorization() Authorization",
        "type TokenStore interface",
        "func NewMemoryTokenStore() *MemoryTokenStore",
        "func (s *MemoryTokenStore) Load(",
        "func (s *MemoryTokenStore) Replace(",
        "func (s *MemoryTokenStore) Clear(",
        "type AuthError struct",
        "func (e *AuthError) Unwrap() error",
        // The compiled scheme descriptor: constants, never runtime parsing.
        "var oauthSchemes = map[string]*oauthSchemeDescriptor{",
        "\"service\": {",
        "Name: \"service\"",
        "ClientAuth: \"client-secret-basic\"",
        "ClientIDEnv: \"OAUTH_SDK_CLIENT_ID\", ClientSecretEnv: \"OAUTH_SDK_CLIENT_SECRET\"",
        "Skew: 30 * time.Second",
        "TokenURL: \"https://auth.oauth.test/token\", RefreshURL: \"https://auth.oauth.test/token-refresh\"",
        "ClientCredentials: \"https://auth.oauth.test/token\"",
        "RevocationURL: \"https://auth.oauth.test/revoke\", IntrospectionURL: \"https://auth.oauth.test/introspect\"",
        // Client identity is read at call time, never baked in.
        "os.Getenv",
        // The lifecycle API.
        "func (c *Client) ClientCredentialsToken(ctx context.Context, scheme string, opts ...TokenOption) (*TokenSet, error)",
        "func (c *Client) RefreshToken(ctx context.Context, scheme string, set *TokenSet) (*TokenSet, error)",
        "func (c *Client) RevokeToken(ctx context.Context, scheme, tokenValue string) error",
        "func (c *Client) IntrospectToken(ctx context.Context, scheme, tokenValue string) (*Introspection, error)",
        "type Introspection struct",
        "func WithTokenClientCredentials(clientID, clientSecret string) TokenOption",
        "func WithTokenStore(store TokenStore) TokenOption",
        "grant_type",
        "refresh_token",
    ] {
        assert!(
            oauth.content.contains(expected),
            "oauth.go is missing:\n{expected}\n--- emitted: ---\n{}",
            oauth.content
        );
    }
    for absent in [
        "func (c *Client) BeginAuthorization",
        "func (c *Client) CompleteAuthorization",
        "BeginDeviceAuthorization",
        "WithRedirectURI",
    ] {
        assert!(
            !oauth.content.contains(absent),
            "oauth.go must not carry {absent} without the compiled flow"
        );
    }
}

/// An OpenID Connect scheme (endpoints defined by the discovery document at
/// runtime) plus one OAuth2 scheme with configured auxiliary endpoints.
fn discovery_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth discovery","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"identity":["openid"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }},
            "/gadgets":{"get":{
                "operationId":"listGadgets",
                "security":[{"service":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{
            "identity":{
                "type":"openIdConnect",
                "openIdConnectUrl":"https://authority.oauth.test/.well-known/openid-configuration"
            },
            "service":{
                "type":"oauth2",
                "flows":{
                    "clientCredentials":{
                        "tokenUrl":"https://auth.oauth.test/token",
                        "scopes":{"read":"Read access"}
                    }
                }
            }
        }}
    })
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "identity":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET"
                    },
                    "service":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                        "revocation_endpoint":"https://auth.oauth.test/revoke",
                        "introspection_endpoint":"https://auth.oauth.test/introspect"
                    }
                }}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

#[test]
fn discovery_schemes_emit_the_discovery_engine_and_cache() {
    let mut configured = generate(&discovery_document(), &discovery_options());
    let mut plain = generate(&discovery_document(), &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut plain);
    assert_eq!(configured.len(), plain.len() + 1);
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let oauth = configured
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    for expected in [
        // The discovery engine and its typed failure kind.
        "type oauthDiscoveredEndpoints struct",
        "const oauthDiscoveryMaxBytes = 1 << 20",
        "type oauthDiscoveryDocument struct",
        "func (c *Client) oauthDiscovery(ctx context.Context, descriptor *oauthSchemeDescriptor) (*oauthDiscoveredEndpoints, error)",
        "func (c *Client) oauthDiscoveryFetch(ctx context.Context, descriptor *oauthSchemeDescriptor)",
        "\"discovery-failed\"",
        // The per-client cache and single-flight gate.
        "discovery map[string]*oauthDiscoveredEndpoints",
        "func oauthDiscoveryGateKey(scheme string) string",
        // The compiled discovery URL and the discovery-defined scheme.
        "DiscoveryURL: \"https://authority.oauth.test/.well-known/openid-configuration\"",
        // The documented issuer rule and precedence.
        "`issuer` claim, it must be an\n// absolute http(s) URL whose origin",
        "the compiled\n// client-credentials endpoint always wins",
    ] {
        assert!(
            oauth.content.contains(expected),
            "oauth.go is missing:\n{expected}\n--- emitted: ---\n{}",
            oauth.content
        );
    }
    // The discovery-defined scheme compiles with empty endpoints: the
    // discovery document supplies them at call time.
    assert!(oauth.content.contains("\"identity\": {"));
    // Control: the plain fixture has no discovery URL, so the engine is
    // absent entirely and the plain bytes are unchanged.
    let plain_oauth = generate(&service_document(), &configured_options());
    let plain = plain_oauth
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    assert!(!plain.content.contains("oauthDiscoveredEndpoints"));
    assert!(!plain.content.contains("oauthDiscovery"));
    assert!(!plain.content.contains("DiscoveryURL"));
}

#[test]
fn deprecated_only_schemes_emit_nothing() {
    let mut configured = generate(&deprecated_only_document(), &deprecated_options());
    let mut plain = generate(&deprecated_only_document(), &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut plain);
    assert!(
        !configured.iter().any(|file| file.path == "go/oauth.go"),
        "implicit/password-only schemes must not emit the lifecycle runtime"
    );
    assert_eq!(configured.len(), plain.len());
    for (configured, plain) in configured.iter().zip(plain.iter()) {
        assert_eq!(configured.path, plain.path);
        assert_eq!(configured.content, plain.content);
    }
}

#[test]
fn interactive_flows_compile_only_their_own_api() {
    let mut configured = generate(&interactive_document(), &interactive_options());
    let mut plain = generate(&interactive_document(), &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut plain);
    assert_eq!(configured.len(), plain.len() + 1);
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let oauth = configured
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    for expected in [
        "func (c *Client) BeginAuthorization(scheme string, opts ...AuthorizationOption) (*AuthorizationTransaction, error)",
        "func (c *Client) CompleteAuthorization(ctx context.Context, txn *AuthorizationTransaction, callbackParams map[string]string) (*TokenSet, error)",
        "type AuthorizationTransaction struct",
        "code_challenge_method",
        "\"S256\"",
        "code_verifier",
        "state",
        "subtle.ConstantTimeCompare",
        "crypto/rand entropy",
        "func (c *Client) BeginDeviceAuthorization(ctx context.Context, scheme string) (*DeviceAuthorization, error)",
        "type DeviceAuthorization struct",
        "func (d *DeviceAuthorization) Token(ctx context.Context) (*TokenSet, error)",
        "urn:ietf:params:oauth:grant-type:device_code",
        "authorization_pending",
        "slow_down",
        "verification_uri",
        "user_code",
        "crypto/rand entropy",
        "AuthorizationURL: \"https://auth.oauth.test/authorize\"",
        "DeviceURL: \"https://auth.oauth.test/device\"",
    ] {
        assert!(
            oauth.content.contains(expected),
            "oauth.go is missing:\n{expected}\n--- emitted: ---\n{}",
            oauth.content
        );
    }
    // The implicit flow is represented by the plan but never compiled into the
    // runtime: its endpoint appears nowhere in the emitted file.
    assert!(
        !oauth.content.contains("implicit-authorize"),
        "the deprecated implicit flow must not be compiled into the runtime"
    );
    // Supplemental endpoints exist only when configuration supplied them.
    assert!(
        !oauth.content.contains("RevokeToken") && !oauth.content.contains("IntrospectToken"),
        "revocation and introspection must stay absent without configured endpoints"
    );
}

#[test]
fn plan_carries_the_compiled_oauth_plan_only_when_configured() {
    let document = service_document();
    let selected = contract(&document)
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = go_http::plan_http(
        contract(&document),
        &selected,
        go_http::HttpConfig {
            sdk_defaults: configured_options().sdk_defaults,
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = configured.oauth().expect("configured policy is carried");
    assert_eq!(oauth.mode, suspect_codegen::http_protocol::OAuthMode::Auto);
    assert_eq!(oauth.schemes.len(), 1);
    let scheme = &oauth.schemes[0];
    assert_eq!(scheme.name, "service");
    assert_eq!(scheme.client_id_env.as_deref(), Some("OAUTH_SDK_CLIENT_ID"));
    assert_eq!(
        scheme.client_secret_env.as_deref(),
        Some("OAUTH_SDK_CLIENT_SECRET")
    );
    assert_eq!(scheme.refresh_skew_seconds, 30);
    assert_eq!(
        scheme.flows[0].client_auth,
        suspect_codegen::http_protocol::OAuthClientAuth::ClientSecretBasic
    );
    assert_eq!(
        scheme.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    let control = go_http::plan_http(
        contract(&document),
        &selected,
        go_http::HttpConfig::default(),
    )
    .unwrap();
    assert!(control.oauth().is_none());
    // The deprecated-only plan still compiles the scheme; only execution is
    // refused, so emission stays empty.
    let deprecated = deprecated_only_document();
    let deprecated_selected = contract(&deprecated)
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let deprecated_plan = go_http::plan_http(
        contract(&deprecated),
        &deprecated_selected,
        go_http::HttpConfig {
            sdk_defaults: deprecated_options().sdk_defaults,
            ..Default::default()
        },
    )
    .unwrap();
    let deprecated_oauth = deprecated_plan.oauth().expect("deprecated plan is carried");
    assert_eq!(deprecated_oauth.schemes.len(), 1);
    assert!(
        deprecated_oauth.schemes[0]
            .flows
            .iter()
            .all(|flow| flow.deprecated_flow)
    );
}

fn go_toolchain() -> Option<String> {
    let output = Command::new("go").arg("version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    // "go version go1.27.1 darwin/arm64" — require at least the emitted go.mod
    // language version so the module builds with the installed toolchain.
    let version = text.split_whitespace().nth(2)?;
    let minor = version
        .strip_prefix("go1.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 23).then_some(text)
}

#[test]
fn native_module_builds_and_vets_with_the_oauth_file() {
    let Some(_) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&service_document(), &configured_options()),
        root.path(),
    )
    .unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn native_module_builds_with_interactive_flows() {
    let Some(_) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&interactive_document(), &interactive_options()),
        root.path(),
    )
    .unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn native_module_builds_and_vets_with_discovery_flows() {
    let Some(_) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&discovery_document(), &discovery_options()),
        root.path(),
    )
    .unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

const DISCOVERY_BEHAVIOR: &str = r#"package consumer

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"

	sdk "example.com/oauth-sdk"
)

const (
	discoveryURL          = "https://authority.oauth.test/.well-known/openid-configuration"
	discoveredTokenURL    = "https://authority.oauth.test/oauth/token"
	discoveredRevokeURL   = "https://authority.oauth.test/oauth/revoke"
	discoveredIntrospect  = "https://authority.oauth.test/oauth/introspect"
	compiledTokenURL      = "https://auth.oauth.test/token"
	compiledRevokeURL     = "https://auth.oauth.test/revoke"
	clientIDValue         = "consumer-client"
	clientSecretValue     = "consumer-secret-must-never-echo-9f2a7c"
)

// relayTransport redirects nothing: the lifecycle endpoints own their answers.
var relayTransport = &http.Client{
	Transport:     http.DefaultTransport,
	CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse },
}

// discoveryFake serves the discovery document, the discovered endpoints and
// the compiled scheme's endpoints behind one httptest server. The Doer below
// rewrites every request onto it, so the compiled https:// URLs and the
// discovered ones all travel through the client's own transport.
type discoveryRequest struct {
	Method        string
	URL           string
	Path          string
	Authorization string
	ContentType   string
	Accept        string
	Form          url.Values
}

type discoveryFake struct {
	mu         sync.Mutex
	hits       map[string]int
	requests   []discoveryRequest
	issuer     string
	failStatus int
	server     *httptest.Server
}

func newDiscoveryFake() *discoveryFake {
	fake := &discoveryFake{hits: map[string]int{}, issuer: "https://authority.oauth.test"}
	fake.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fake.mu.Lock()
		fake.hits[r.URL.Path]++
		issuer := fake.issuer
		failStatus := fake.failStatus
		fake.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/.well-known/openid-configuration":
			if failStatus != 0 {
				w.WriteHeader(failStatus)
				fmt.Fprintf(w, `{"error":"boom"}`)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{
				"issuer":                issuer,
				"token_endpoint":        discoveredTokenURL,
				"revocation_endpoint":   discoveredRevokeURL,
				"introspection_endpoint": discoveredIntrospect,
				"unknown_member":        map[string]any{"nested": true},
			})
		case "/oauth/token":
			w.WriteHeader(http.StatusOK)
			_ = json.NewEncoder(w).Encode(map[string]any{"access_token": "discovered-1", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "rotated-1"})
		case "/oauth/revoke":
			w.WriteHeader(http.StatusOK)
		case "/oauth/introspect":
			_ = json.NewEncoder(w).Encode(map[string]any{"active": true, "scope": "read"})
		case "/token":
			w.WriteHeader(http.StatusOK)
			_ = json.NewEncoder(w).Encode(map[string]any{"access_token": "compiled", "token_type": "Bearer", "expires_in": 3600})
		case "/revoke":
			w.WriteHeader(http.StatusOK)
		case "/introspect":
			_ = json.NewEncoder(w).Encode(map[string]any{"active": true, "scope": "read"})
		default:
			w.WriteHeader(http.StatusOK)
		}
	}))
	return fake
}

func (f *discoveryFake) discoveryHits() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.hits["/.well-known/openid-configuration"]
}

func (f *discoveryFake) tokenHits() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.hits["/oauth/token"]
}

func (f *discoveryFake) setIssuer(issuer string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.issuer = issuer
}

func (f *discoveryFake) fail(status int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.failStatus = status
}

func (f *discoveryFake) requestsTo(path string) []discoveryRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	var found []discoveryRequest
	for _, request := range f.requests {
		if request.Path == path {
			found = append(found, request)
		}
	}
	return found
}

func (f *discoveryFake) lastRequestTo(path string) discoveryRequest {
	found := f.requestsTo(path)
	if len(found) == 0 {
		return discoveryRequest{}
	}
	return found[len(found)-1]
}

// Do rewrites every request onto the fake server, recording the original.
func (f *discoveryFake) Do(request *http.Request) (*http.Response, error) {
	target, err := url.Parse(f.server.URL)
	if err != nil {
		return nil, err
	}
	record := discoveryRequest{
		Method:        request.Method,
		URL:           request.URL.String(),
		Path:          request.URL.Path,
		Authorization: request.Header.Get("Authorization"),
		ContentType:   request.Header.Get("Content-Type"),
		Accept:        request.Header.Get("Accept"),
	}
	if request.Body != nil {
		if body, readErr := io.ReadAll(request.Body); readErr == nil {
			_ = request.Body.Close()
			record.Form, _ = url.ParseQuery(string(body))
			request.Body = io.NopCloser(bytes.NewReader(body))
		}
	}
	f.mu.Lock()
	f.requests = append(f.requests, record)
	f.mu.Unlock()
	outbound := request.Clone(request.Context())
	outbound.URL = &(*request.URL)
	outbound.URL.Scheme = target.Scheme
	outbound.URL.Host = target.Host
	return relayTransport.Do(outbound)
}

func TestDiscoveryCredentialsResolveThroughTheDiscoveredTokenEndpoint(t *testing.T) {
	fake := newDiscoveryFake()
	defer fake.server.Close()
	t.Setenv("OAUTH_SDK_CLIENT_ID", clientIDValue)
	t.Setenv("OAUTH_SDK_CLIENT_SECRET", clientSecretValue)
	client, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	first, err := client.ClientCredentialsToken(ctx, "identity")
	if err != nil {
		t.Fatal(err)
	}
	if first.AccessToken != "discovered-1" {
		t.Fatalf("unexpected token: %q", first.AccessToken)
	}
	if fake.discoveryHits() != 1 {
		t.Fatalf("unexpected discovery fetch count: %d", fake.discoveryHits())
	}
	sent := fake.requestsTo("/.well-known/openid-configuration")[0]
	if sent.Method != http.MethodGet || sent.Accept != "application/json" || sent.URL != discoveryURL {
		t.Fatalf("unexpected discovery request: %+v", sent)
	}
	token := fake.requestsTo("/oauth/token")[0]
	if token.URL != discoveredTokenURL {
		t.Fatalf("the acquisition did not target the discovered endpoint: %s", token.URL)
	}
	if token.Form.Get("grant_type") != "client_credentials" {
		t.Fatalf("unexpected form: %v", token.Form)
	}
	decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(token.Authorization, "Basic "))
	if err != nil || string(decoded) != clientIDValue+":"+clientSecretValue {
		t.Fatalf("the discovery-defined client did not authenticate with basic: %q %v", token.Authorization, err)
	}
	for _, request := range fake.requests {
		if request.URL == compiledTokenURL {
			t.Fatal("the compiled fallback endpoint was contacted")
		}
	}
	// The discovery document and the token set are both cached: the second
	// call fetches nothing.
	second, err := client.ClientCredentialsToken(ctx, "identity")
	if err != nil {
		t.Fatal(err)
	}
	if second.AccessToken != "discovered-1" || fake.discoveryHits() != 1 || fake.tokenHits() != 1 {
		t.Fatalf("cache hit re-fetched: hits=%d tokens=%d", fake.discoveryHits(), fake.tokenHits())
	}
	// Concurrent callers share one discovery fetch and one acquisition.
	fresh, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	const callers = 2
	sets := make([]*sdk.TokenSet, callers)
	errs := make([]error, callers)
	var wait sync.WaitGroup
	start := make(chan struct{})
	for index := 0; index < callers; index++ {
		wait.Add(1)
		go func(index int) {
			defer wait.Done()
			<-start
			sets[index], errs[index] = fresh.ClientCredentialsToken(ctx, "identity")
		}(index)
	}
	close(start)
	wait.Wait()
	for index, err := range errs {
		if err != nil {
			t.Fatalf("caller %d: %v", index, err)
		}
	}
	if fake.discoveryHits() != 2 {
		t.Fatalf("concurrent callers fetched discovery %d times", fake.discoveryHits())
	}
}

func TestDiscoveryIssuerMismatchAndFailuresAreTypedAndRetried(t *testing.T) {
	fake := newDiscoveryFake()
	defer fake.server.Close()
	fake.issuer = "https://elsewhere.oauth.test"
	t.Setenv("OAUTH_SDK_CLIENT_ID", clientIDValue)
	t.Setenv("OAUTH_SDK_CLIENT_SECRET", clientSecretValue)
	client, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	var authError *sdk.AuthError
	if _, err := client.ClientCredentialsToken(context.Background(), "identity"); !errors.As(err, &authError) || authError.Kind != "discovery-failed" {
		t.Fatalf("issuer mismatch did not fail typed: %v", err)
	}
	// The failed fetch is not cached: after the server is fixed the next call
	// retries and succeeds.
	fake.issuer = "https://authority.oauth.test"
	set, err := client.ClientCredentialsToken(context.Background(), "identity")
	if err != nil {
		t.Fatal(err)
	}
	if set.AccessToken != "discovered-1" || fake.discoveryHits() != 2 {
		t.Fatalf("the failed fetch was not retried: hits=%d", fake.discoveryHits())
	}
	// A failing discovery request is a typed failure the next call retries.
	// A fresh client owns a fresh discovery cache, so the retry really fetches.
	freshClient, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	fake.fail(500)
	if _, err := freshClient.ClientCredentialsToken(context.Background(), "identity"); !errors.As(err, &authError) || authError.Kind != "discovery-failed" || authError.Status != 500 {
		t.Fatalf("the failed request did not fail typed: %v", err)
	}
	fake.fail(0)
	set, err = freshClient.ClientCredentialsToken(context.Background(), "identity")
	if err != nil {
		t.Fatal(err)
	}
	if set.AccessToken != "discovered-1" || fake.discoveryHits() != 4 {
		t.Fatalf("the failed request was not retried: hits=%d", fake.discoveryHits())
	}
}

func TestDiscoveryRevocationIntrospectionAndRefreshFollowPrecedence(t *testing.T) {
	fake := newDiscoveryFake()
	defer fake.server.Close()
	fake.issuer = "https://authority.oauth.test"
	t.Setenv("OAUTH_SDK_CLIENT_ID", clientIDValue)
	t.Setenv("OAUTH_SDK_CLIENT_SECRET", clientSecretValue)
	client, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	// The discovery-defined scheme resolves revocation through discovery.
	if err := client.RevokeToken(ctx, "identity", "the-token-value"); err != nil {
		t.Fatal(err)
	}
	revoke := fake.lastRequestTo("/oauth/revoke")
	if revoke.URL != discoveredRevokeURL {
		t.Fatalf("revocation did not target the discovered endpoint: %s", revoke.URL)
	}
	// The configured scheme keeps its compiled endpoint: it wins.
	if err := client.RevokeToken(ctx, "service", "the-token-value"); err != nil {
		t.Fatal(err)
	}
	if compiled := fake.lastRequestTo("/revoke"); compiled.URL != compiledRevokeURL {
		t.Fatalf("the configured endpoint did not win: %s", compiled.URL)
	}
	// Introspection follows the same precedence.
	claims, err := client.IntrospectToken(ctx, "identity", "the-token-value")
	if err != nil {
		t.Fatal(err)
	}
	if !claims.Active || claims.Scope != "read" {
		t.Fatalf("unexpected introspection: %+v", claims)
	}
	if introspect := fake.lastRequestTo("/oauth/introspect"); introspect.URL != discoveredIntrospect {
		t.Fatalf("introspection did not target the discovered endpoint: %s", introspect.URL)
	}
	// Refresh resolves through the discovered token endpoint: acquire the
	// set first (the discovered token response carries a refresh token),
	// then exchange it.
	set, err := client.ClientCredentialsToken(ctx, "identity")
	if err != nil {
		t.Fatal(err)
	}
	if set.RefreshToken == "" {
		t.Fatalf("the discovered token response carried no refresh token: %+v", set)
	}
	refreshed, err := client.RefreshToken(ctx, "identity", set)
	if err != nil {
		t.Fatal(err)
	}
	if refreshed.AccessToken != "discovered-1" {
		t.Fatalf("unexpected refresh result: %+v", refreshed)
	}
	refresh := fake.lastRequestTo("/oauth/token")
	if refresh.Form.Get("grant_type") != "refresh_token" || refresh.Form.Get("refresh_token") != set.RefreshToken {
		t.Fatalf("unexpected refresh form: %v", refresh.Form)
	}
}

var _ = json.Marshal
var _ = fmt.Sprintf
var _ = errors.New
"#;

#[test]
fn discovery_lifecycle_runs_against_a_fake_discovery_server() {
    let Some(_) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&discovery_document(), &discovery_options()),
        root.path(),
    )
    .unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("go.mod"),
        "module example.com/oauth-consumer\n\ngo 1.23.0\nrequire example.com/oauth-sdk v0.1.0\nreplace example.com/oauth-sdk => ../go\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("discovery_behavior_test.go"),
        DISCOVERY_BEHAVIOR,
    )
    .unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "-v", "."])
        .current_dir(&consumer)
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "discovery OAuth lifecycle failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_oauth discovery: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}

const BEHAVIOR: &str = r#"package consumer

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	sdk "example.com/oauth-sdk"
)

const (
	compiledTokenURL      = "https://auth.oauth.test/token"
	compiledRefreshURL    = "https://auth.oauth.test/token-refresh"
	compiledRevokeURL     = "https://auth.oauth.test/revoke"
	compiledIntrospectURL = "https://auth.oauth.test/introspect"
	clientIDValue         = "consumer-client"
	clientSecretValue     = "consumer-secret-must-never-echo-9f2a7c"
)

type recordedRequest struct {
	Method        string
	URL           string
	Path          string
	Authorization string
	ContentType   string
	Form          url.Values
}

// fakeOAuth is the fake token/revocation/introspection endpoint suite behind a
// real httptest server, plus the URL-rewriting Doer that points the compiled
// https://auth.oauth.test endpoints at it through the client's own transport.
type fakeOAuth struct {
	mu              sync.Mutex
	hits            map[string]int
	requests        []recordedRequest
	expiresIn       int64
	access          string
	refresh         string
	failStatus      int
	failCode        string
	failDescription string
	delay           time.Duration
	server          *httptest.Server
}

func newFakeOAuth() *fakeOAuth {
	fake := &fakeOAuth{hits: map[string]int{}, expiresIn: 3600, access: "token-1"}
	fake.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fake.mu.Lock()
		fake.hits[r.URL.Path]++
		delay, failStatus, failCode, failDescription := fake.delay, fake.failStatus, fake.failCode, fake.failDescription
		access, refresh, expiresIn := fake.access, fake.refresh, fake.expiresIn
		fake.mu.Unlock()
		if delay > 0 {
			time.Sleep(delay)
		}
		if failStatus != 0 {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(failStatus)
			fmt.Fprintf(w, `{"error":%q,"error_description":%q}`, failCode, failDescription)
			return
		}
		switch r.URL.Path {
		case "/token", "/token-refresh":
			w.Header().Set("Content-Type", "application/json")
			body := map[string]any{"access_token": access, "token_type": "Bearer", "expires_in": expiresIn}
			if refresh != "" {
				body["refresh_token"] = refresh
			}
			_ = json.NewEncoder(w).Encode(body)
		case "/revoke":
			w.WriteHeader(http.StatusOK)
		case "/introspect":
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{
				"active": true, "scope": "read", "client_id": clientIDValue,
				"sub": "user-1", "exp": time.Now().Add(time.Hour).Unix(),
			})
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	return fake
}

// relayTransport redirects nothing: the lifecycle endpoints own their answers.
var relayTransport = &http.Client{
	Transport:     http.DefaultTransport,
	CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse },
}

// Do rewrites the compiled endpoint hosts onto the fake server, recording the
// original request before transport.
func (f *fakeOAuth) Do(request *http.Request) (*http.Response, error) {
	target, err := url.Parse(f.server.URL)
	if err != nil {
		return nil, err
	}
	record := recordedRequest{
		Method:        request.Method,
		URL:           request.URL.String(),
		Path:          request.URL.Path,
		Authorization: request.Header.Get("Authorization"),
		ContentType:   request.Header.Get("Content-Type"),
	}
	if body, readErr := io.ReadAll(request.Body); readErr == nil {
		_ = request.Body.Close()
		record.Form, _ = url.ParseQuery(string(body))
		request.Body = io.NopCloser(bytes.NewReader(body))
	}
	f.mu.Lock()
	f.requests = append(f.requests, record)
	f.mu.Unlock()
	outbound := request.Clone(request.Context())
	outbound.URL = &(*request.URL)
	outbound.URL.Scheme = target.Scheme
	outbound.URL.Host = target.Host
	return relayTransport.Do(outbound)
}

func (f *fakeOAuth) tokenHits() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.hits["/token"]
}

func (f *fakeOAuth) mode(access, refresh string, expiresIn int64) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.access, f.refresh, f.expiresIn = access, refresh, expiresIn
}

func (f *fakeOAuth) fail(status int, code, description string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.failStatus, f.failCode, f.failDescription = status, code, description
}

func (f *fakeOAuth) reset() {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.failStatus, f.failCode, f.failDescription = 0, "", ""
}

func (f *fakeOAuth) setDelay(delay time.Duration) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.delay = delay
}

func (f *fakeOAuth) requestsTo(path string) []recordedRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	var found []recordedRequest
	for _, request := range f.requests {
		if request.Path == path {
			found = append(found, request)
		}
	}
	return found
}

func (f *fakeOAuth) lastRequestTo(path string) recordedRequest {
	found := f.requestsTo(path)
	if len(found) == 0 {
		return recordedRequest{}
	}
	return found[len(found)-1]
}

func newOAuthClient(t *testing.T, fake *fakeOAuth) *sdk.Client {
	t.Helper()
	t.Setenv("OAUTH_SDK_CLIENT_ID", clientIDValue)
	t.Setenv("OAUTH_SDK_CLIENT_SECRET", clientSecretValue)
	client, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	return client
}

func TestAcquireThenCacheHitIssuesOneTokenRequest(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	client := newOAuthClient(t, fake)
	ctx := context.Background()
	first, err := client.ClientCredentialsToken(ctx, "service")
	if err != nil {
		t.Fatal(err)
	}
	second, err := client.ClientCredentialsToken(ctx, "service")
	if err != nil {
		t.Fatal(err)
	}
	if first.AccessToken != "token-1" || second.AccessToken != "token-1" {
		t.Fatalf("unexpected tokens: %q %q", first.AccessToken, second.AccessToken)
	}
	if *first != *second {
		t.Fatal("cache hit returned a different set")
	}
	if fake.tokenHits() != 1 {
		t.Fatalf("cache hit issued another acquisition: %d", fake.tokenHits())
	}
	sent := fake.lastRequestTo("/token")
	if sent.URL != compiledTokenURL {
		t.Fatalf("token request did not target the compiled endpoint: %s", sent.URL)
	}
	if sent.Method != http.MethodPost || sent.ContentType != "application/x-www-form-urlencoded" {
		t.Fatalf("unexpected token request: %+v", sent)
	}
	if sent.Form.Get("grant_type") != "client_credentials" {
		t.Fatalf("unexpected form: %v", sent.Form)
	}
	if sent.Form.Get("client_id") != "" {
		t.Fatal("a confidential client leaked its identity into the form", sent.Form)
	}
	decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(sent.Authorization, "Basic "))
	if err != nil {
		t.Fatal(err)
	}
	if string(decoded) != clientIDValue+":"+clientSecretValue {
		t.Fatalf("basic credential was not client_secret_basic: %q", decoded)
	}
	if strings.Contains(sent.URL, clientSecretValue) || strings.Contains(sent.Form.Encode(), clientSecretValue) {
		t.Fatal("the secret traveled outside the authorization header")
	}
}

func TestExpiryBeyondSkewReacquires(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	// expires_in=1s sits inside the compiled 30s skew, so the stored set is
	// immediately stale.
	fake.mode("token-1", "", 1)
	client := newOAuthClient(t, fake)
	ctx := context.Background()
	first, err := client.ClientCredentialsToken(ctx, "service")
	if err != nil {
		t.Fatal(err)
	}
	fake.mode("token-2", "", 3600)
	second, err := client.ClientCredentialsToken(ctx, "service")
	if err != nil {
		t.Fatal(err)
	}
	if second.AccessToken != "token-2" {
		t.Fatal("the stale set was not reacquired", second.AccessToken)
	}
	if fake.tokenHits() != 2 {
		t.Fatalf("unexpected acquisition count: %d", fake.tokenHits())
	}
	if first.AccessToken != "token-1" {
		t.Fatal("the first set changed", first.AccessToken)
	}
}

func TestSingleFlightSharesOneAcquisition(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	fake.setDelay(150 * time.Millisecond)
	client := newOAuthClient(t, fake)
	ctx := context.Background()
	const callers = 2
	sets := make([]*sdk.TokenSet, callers)
	errs := make([]error, callers)
	var wait sync.WaitGroup
	start := make(chan struct{})
	for index := 0; index < callers; index++ {
		wait.Add(1)
		go func(index int) {
			defer wait.Done()
			<-start
			sets[index], errs[index] = client.ClientCredentialsToken(ctx, "service")
		}(index)
	}
	close(start)
	wait.Wait()
	for index, err := range errs {
		if err != nil {
			t.Fatalf("caller %d: %v", index, err)
		}
		if sets[index].AccessToken != "token-1" {
			t.Fatalf("caller %d token: %q", index, sets[index].AccessToken)
		}
	}
	if *sets[0] != *sets[1] {
		t.Fatal("concurrent callers received different sets")
	}
	if fake.tokenHits() != 1 {
		t.Fatalf("concurrent callers issued %d acquisitions", fake.tokenHits())
	}
}

func TestRefreshAdoptsRotatedAndRetainsUnrotated(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	fake.mode("token-1", "rotated-1", 3600)
	client := newOAuthClient(t, fake)
	ctx := context.Background()
	set, err := client.ClientCredentialsToken(ctx, "service")
	if err != nil {
		t.Fatal(err)
	}
	if set.RefreshToken != "rotated-1" {
		t.Fatal("the declared refresh token was not kept", set.RefreshToken)
	}
	fake.mode("refreshed-1", "rotated-2", 3600)
	rotated, err := client.RefreshToken(ctx, "service", set)
	if err != nil {
		t.Fatal(err)
	}
	if rotated.AccessToken != "refreshed-1" {
		t.Fatal("refresh did not return the new access token", rotated.AccessToken)
	}
	if rotated.RefreshToken != "rotated-2" {
		t.Fatal("the rotated refresh token was not adopted", rotated.RefreshToken)
	}
	refreshes := fake.requestsTo("/token-refresh")
	if len(refreshes) != 1 {
		t.Fatalf("refresh did not use the declared refresh URL exactly once: %d", len(refreshes))
	}
	if refreshes[0].URL != compiledRefreshURL {
		t.Fatalf("refresh did not target the declared refresh URL: %s", refreshes[0].URL)
	}
	if refreshes[0].Form.Get("grant_type") != "refresh_token" || refreshes[0].Form.Get("refresh_token") != "rotated-1" {
		t.Fatalf("unexpected refresh form: %v", refreshes[0].Form)
	}
	// A server that stops rotating: the previous refresh token is retained.
	fake.mode("refreshed-2", "", 3600)
	retained, err := client.RefreshToken(ctx, "service", rotated)
	if err != nil {
		t.Fatal(err)
	}
	if retained.AccessToken != "refreshed-2" {
		t.Fatal("retention lost the new access token", retained.AccessToken)
	}
	if retained.RefreshToken != "rotated-2" {
		t.Fatal("the previous refresh token was not retained", retained.RefreshToken)
	}
}

func TestWrongCredentialsProduceTypedErrorWithoutTheSecret(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	fake.fail(401, "invalid_client", "client authentication failed")
	client := newOAuthClient(t, fake)
	_, err := client.ClientCredentialsToken(context.Background(), "service")
	var authError *sdk.AuthError
	if !errors.As(err, &authError) {
		t.Fatalf("expected *sdk.AuthError, got %T: %v", err, err)
	}
	if authError.Kind != "authorization-error" || authError.Code != "invalid_client" || authError.Scheme != "service" || authError.Status != 401 {
		t.Fatalf("unexpected typed failure: %+v", authError)
	}
	if authError.Description != "client authentication failed" {
		t.Fatalf("server context was lost: %+v", authError)
	}
	if strings.Contains(err.Error(), clientSecretValue) || strings.Contains(fmt.Sprintf("%+v", authError), clientSecretValue) {
		t.Fatal("the error leaked the client secret")
	}
	// The credentials were in fact sent as HTTP Basic: the failure is the
	// server's rejection, not a missing credential.
	sent := fake.lastRequestTo("/token")
	expected := "Basic " + base64.StdEncoding.EncodeToString([]byte(clientIDValue+":"+clientSecretValue))
	if sent.Authorization != expected {
		t.Fatalf("credentials were not sent as client_secret_basic: %q", sent.Authorization)
	}
	// The failed acquisition is not cached: a working server acquires fresh.
	fake.reset()
	fake.mode("token-after-failure", "", 3600)
	set, err := client.ClientCredentialsToken(context.Background(), "service")
	if err != nil {
		t.Fatal(err)
	}
	if set.AccessToken != "token-after-failure" {
		t.Fatal("the failed acquisition was cached", set.AccessToken)
	}
	if fake.tokenHits() != 2 {
		t.Fatalf("unexpected acquisition count: %d", fake.tokenHits())
	}
}

func TestRevocationAndIntrospectionPostToCompiledEndpoints(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	client := newOAuthClient(t, fake)
	ctx := context.Background()
	if err := client.RevokeToken(ctx, "service", "the-token-value"); err != nil {
		t.Fatal(err)
	}
	revoke := fake.lastRequestTo("/revoke")
	if revoke.URL != compiledRevokeURL {
		t.Fatalf("revocation did not target the compiled endpoint: %s", revoke.URL)
	}
	if revoke.Method != http.MethodPost || revoke.Form.Get("token") != "the-token-value" {
		t.Fatalf("unexpected revocation request: %+v", revoke)
	}
	if revoke.Form.Get("client_id") != "" {
		t.Fatal("confidential revocation leaked its identity into the form")
	}
	if revoke.Authorization == "" {
		t.Fatal("revocation lost client authentication")
	}
	introspection, err := client.IntrospectToken(ctx, "service", "the-token-value")
	if err != nil {
		t.Fatal(err)
	}
	if !introspection.Active || introspection.Scope != "read" || introspection.ClientID != clientIDValue || introspection.Subject != "user-1" {
		t.Fatalf("unexpected introspection: %+v", introspection)
	}
	if introspection.ExpiresAt.IsZero() {
		t.Fatal("exp was not converted")
	}
	introspect := fake.lastRequestTo("/introspect")
	if introspect.URL != compiledIntrospectURL {
		t.Fatalf("introspection did not target the compiled endpoint: %s", introspect.URL)
	}
}

func TestContextCancellationStopsAcquisition(t *testing.T) {
	fake := newFakeOAuth()
	defer fake.server.Close()
	fake.setDelay(time.Second)
	client := newOAuthClient(t, fake)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() {
		for fake.tokenHits() == 0 {
			time.Sleep(time.Millisecond)
		}
		cancel()
	}()
	started := time.Now()
	_, err := client.ClientCredentialsToken(ctx, "service")
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("expected cancellation, got %v", err)
	}
	if elapsed := time.Since(started); elapsed > 900*time.Millisecond {
		t.Fatal("cancellation did not stop the acquisition", elapsed)
	}
	fake.setDelay(0)
	// A pre-cancelled context never reaches the endpoint.
	cancelled, stop := context.WithCancel(context.Background())
	stop()
	if _, err := client.ClientCredentialsToken(cancelled, "service"); !errors.Is(err, context.Canceled) {
		t.Fatalf("the pre-cancelled context did not fail fast: %v", err)
	}
	if fake.tokenHits() != 1 {
		t.Fatalf("cancelled acquisitions reached the endpoint: %d", fake.tokenHits())
	}
}

func TestUnknownSchemeFailsTypedAndStoreStaysInstanceOwned(t *testing.T) {
	client, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{})
	if err != nil {
		t.Fatal(err)
	}
	var authError *sdk.AuthError
	if _, err := client.ClientCredentialsToken(context.Background(), "missing"); !errors.As(err, &authError) || authError.Kind != "unknown-scheme" {
		t.Fatalf("unknown scheme did not fail typed: %v", err)
	}
	if err := client.RevokeToken(context.Background(), "missing", "x"); !errors.As(err, &authError) || authError.Kind != "unknown-scheme" {
		t.Fatalf("unknown scheme revocation did not fail typed: %v", err)
	}
	// An explicitly created store is independent and copies on the way
	// through, so caller and store never alias a set.
	store := sdk.NewMemoryTokenStore()
	ctx := context.Background()
	set := &sdk.TokenSet{AccessToken: "a"}
	if err := store.Replace(ctx, "k1", set); err != nil {
		t.Fatal(err)
	}
	set.AccessToken = "mutated-after-store"
	loaded, err := store.Load(ctx, "k1")
	if err != nil {
		t.Fatal(err)
	}
	if loaded == nil || loaded.AccessToken != "a" {
		t.Fatal("the store aliased the caller's set")
	}
	loaded.AccessToken = "mutated-after-load"
	again, _ := store.Load(ctx, "k1")
	if again.AccessToken != "a" {
		t.Fatal("the loaded set aliases the store")
	}
	if other, _ := store.Load(ctx, "k2"); other != nil {
		t.Fatal("an unrelated key returned a set")
	}
	if err := store.Clear(ctx, "k1"); err != nil {
		t.Fatal(err)
	}
	if cleared, _ := store.Load(ctx, "k1"); cleared != nil {
		t.Fatal("clear failed")
	}
}
"#;

#[test]
fn native_lifecycle_counts_caches_rotates_and_cancels() {
    let Some(version) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    eprintln!("go_oauth: {version}");
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&service_document(), &configured_options()),
        root.path(),
    )
    .unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("go.mod"),
        "module example.com/oauth-consumer\n\ngo 1.23.0\nrequire example.com/oauth-sdk v0.1.0\nreplace example.com/oauth-sdk => ../go\n",
    )
    .unwrap();
    std::fs::write(consumer.join("oauth_behavior_test.go"), BEHAVIOR).unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "-v", "."])
        .current_dir(&consumer)
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native OAuth lifecycle failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_oauth: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    json!({
        "openapi":"3.2.0","info":{"title":"OAuth replay","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"service":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }},
            "/events":{"get":{
                "operationId":"streamEvents",
                "security":[{"feed":["read"]}],
                "responses":{"200":{"description":"Events","content":{"application/x-ndjson":{"itemSchema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{
            "service":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/token","scopes":{"read":"Read access"}
            }}},
            "feed":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/feed-token","scopes":{"read":"Read access"}
            }}}
        }}
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "service":{"client_id_env":"OAUTH_SDK_CLIENT_ID","client_secret_env":"OAUTH_SDK_CLIENT_SECRET"},
                    "feed":{"client_id_env":"OAUTH_SDK_FEED_ID","client_secret_env":"OAUTH_SDK_FEED_SECRET"}
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials endpoint, wraps exactly that provider, and compiles
/// the stream-protection pointers of its scheme's operations.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate(&replay_document(), &replay_options());
    let oauth = configured
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    for expected in [
        "func NewReplayCredentials(scheme string, opts ...TokenOption) (*ReplayCredentials, error)",
        "func (r *ReplayCredentials) Hook() CredentialHook",
        "func (r *ReplayCredentials) Transport(inner Doer) (Doer, error)",
        "var oauthNoReplayRequirements = map[string][]string{",
        "\"/paths/~1events/get/security/0/feed\"",
        "one coordinated refresh",
    ] {
        assert!(
            oauth.content.contains(expected),
            "oauth.go is missing:\n{expected}\n--- emitted: ---\n{}",
            oauth.content
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.content.contains("~1widgets/get/security/0/service"));
    // The authorization-code-only control compiles exactly the pre-replay
    // bytes: no wrapper, no stream-protection table.
    let code_only_document = json!({
        "openapi":"3.2.0","info":{"title":"OAuth code only","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{"/widgets":{"get":{
            "operationId":"listWidgets",
            "security":[{"userOAuth":["read"]}],
            "responses":{"200":{"description":"Ok"}}
        }}},
        "components":{"securitySchemes":{"userOAuth":{"type":"oauth2","flows":{"authorizationCode":{
            "authorizationUrl":"https://auth.oauth.test/authorize",
            "tokenUrl":"https://auth.oauth.test/token",
            "scopes":{"read":"Read access"}
        }}}}}
    });
    let code_only_options = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"userOAuth":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let code_only = generate(&code_only_document, &code_only_options);
    let plain_oauth = code_only
        .iter()
        .find(|file| file.path == "go/oauth.go")
        .expect("OAuth lifecycle runtime emitted");
    assert!(!plain_oauth.content.contains("ReplayCredentials"));
    assert!(!plain_oauth.content.contains("oauthNoReplayRequirements"));
}

const REPLAY_BEHAVIOR: &str = r#"package consumer

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"

	sdk "example.com/oauth-sdk"
)

const (
	widgetsPath       = "/api/widgets"
	eventsPath        = "/api/events"
	clientIDValue     = "consumer-client"
	clientSecretValue = "consumer-secret-must-never-echo-9f2a7c"
)

type fakeReplay struct {
	mu          sync.Mutex
	requests    []recordedReplay
	svcTokens   int
	feedTokens  int
	mode        string // "ok" | "always-401"
	staleNext   bool
	staleToken  string
	failFrom    int
	server      *httptest.Server
}

type recordedReplay struct {
	Path          string
	Authorization string
}

func newFakeReplay() *fakeReplay {
	fake := &fakeReplay{failFrom: int(^uint(0) >> 1)}
	fake.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fake.mu.Lock()
		fake.requests = append(fake.requests, recordedReplay{Path: r.URL.Path, Authorization: r.Header.Get("Authorization")})
		mode, stale, failFrom := fake.mode, fake.staleToken, fake.failFrom
		svcTokens := fake.svcTokens + 1
		feedTokens := fake.feedTokens + 1
		fake.mu.Unlock()
		switch r.URL.Path {
		case "/token":
			token := "svc-" + itoa(svcTokens)
			fake.mu.Lock()
			fake.svcTokens = svcTokens
			if fake.staleNext {
				fake.staleNext = false
				fake.staleToken = token
			}
			fake.mu.Unlock()
			if svcTokens >= failFrom {
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(http.StatusInternalServerError)
				_, _ = w.Write([]byte(`{"error":"server_error"}`))
				return
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"access_token":"` + token + `","token_type":"Bearer","expires_in":3600}`))
		case "/feed-token":
			fake.mu.Lock()
			fake.feedTokens = feedTokens
			fake.mu.Unlock()
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"access_token":"feed-"` + `,"token_type":"Bearer","expires_in":3600}`))
		case widgetsPath:
			if mode == "always-401" || (stale != "" && r.Header.Get("Authorization") == "Bearer "+stale) {
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(http.StatusUnauthorized)
				_, _ = w.Write([]byte(`{"error":"stale"}`))
				return
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`"ok"`))
		case eventsPath:
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusUnauthorized)
			_, _ = w.Write([]byte(`{"error":"stream-denied"}`))
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	return fake
}

func itoa(value int) string {
	if value == 0 {
		return "0"
	}
	digits := ""
	for value > 0 {
		digits = string(rune('0'+value%10)) + digits
		value /= 10
	}
	return digits
}

// Do rewrites the compiled endpoint hosts onto the fake server.
func (f *fakeReplay) Do(request *http.Request) (*http.Response, error) {
	target, err := url.Parse(f.server.URL)
	if err != nil {
		return nil, err
	}
	if strings.HasPrefix(request.URL.Path, "/v1/") {
		request.URL.Path = "/api" + strings.TrimPrefix(request.URL.Path, "/v1")
	}
	outbound := request.Clone(request.Context())
	outbound.URL.Scheme = target.Scheme
	outbound.URL.Host = target.Host
	response, err := http.DefaultTransport.RoundTrip(outbound)
	if err != nil {
		return nil, err
	}
	return response, nil
}

func (f *fakeReplay) armStale() {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.staleNext = true
	f.mode = "ok"
}

func (f *fakeReplay) always401() {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.mode = "always-401"
}

func (f *fakeReplay) count(path string) int {
	f.mu.Lock()
	defer f.mu.Unlock()
	total := 0
	for _, request := range f.requests {
		if request.Path == path {
			total++
		}
	}
	return total
}

func (f *fakeReplay) tokensFor(path string) []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	var values []string
	for _, request := range f.requests {
		if request.Path == path {
			values = append(values, request.Authorization)
		}
	}
	return values
}

func newReplayClient(t *testing.T, fake *fakeReplay, scheme string) *sdk.Client {
	t.Helper()
	replay, err := sdk.NewReplayCredentials(scheme)
	if err != nil {
		t.Fatal(err)
	}
	transport, err := replay.Transport(fake)
	if err != nil {
		t.Fatal(err)
	}
	client, err := sdk.NewClient(sdk.Credentials{}.WithHook(scheme, replay.Hook()), sdk.ClientOptions{Transport: transport})
	if err != nil {
		t.Fatal(err)
	}
	return client
}

// (a) 401 then success: one refresh, one replay, and the caller sees 200.
func TestReplayRefreshesOnceAndReplaysOnce(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	fake.armStale()
	client := newReplayClient(t, fake, "service")
	ctx := context.Background()
	if _, err := client.ListWidgets(ctx); err != nil {
		t.Fatal(err)
	}
	if got := fake.count(widgetsPath); got != 2 {
		t.Fatalf("expected exactly one replay, got %d API requests", got)
	}
	if got := fake.count("/token"); got != 2 {
		t.Fatalf("expected exactly one refresh, got %d token requests", got)
	}
	sent := fake.tokensFor(widgetsPath)
	if sent[0] == sent[1] {
		t.Fatal("the replay did not carry the fresh token")
	}
}

// (b) 401 then 401: the second 401 surfaces and exactly one refresh ran.
func TestReplaySurfacesTheSecond401(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	fake.always401()
	client := newReplayClient(t, fake, "service")
	var sdkError *sdk.SDKError
	if _, err := client.ListWidgets(context.Background()); !errors.As(err, &sdkError) || sdkError.Status != 401 {
		t.Fatalf("expected the second 401 to surface, got %v", err)
	}
	if got := fake.count(widgetsPath); got != 2 {
		t.Fatalf("one replay, no loops: %d", got)
	}
	if got := fake.count("/token"); got != 2 {
		t.Fatalf("exactly one refresh: %d", got)
	}
}

// (c) concurrent 401s across two goroutines: ONE refresh, two replays.
func TestConcurrent401sShareOneRefresh(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	fake.armStale()
	replay, err := sdk.NewReplayCredentials("service")
	if err != nil {
		t.Fatal(err)
	}
	transport, err := replay.Transport(fake)
	if err != nil {
		t.Fatal(err)
	}
	client, err := sdk.NewClient(sdk.Credentials{}.WithHook("service", replay.Hook()), sdk.ClientOptions{Transport: transport})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	var wait sync.WaitGroup
	start := make(chan struct{})
	errs := make([]error, 2)
	for index := 0; index < 2; index++ {
		wait.Add(1)
		go func(index int) {
			defer wait.Done()
			<-start
			_, errs[index] = client.ListWidgets(ctx)
		}(index)
	}
	close(start)
	wait.Wait()
	for index, err := range errs {
		if err != nil {
			t.Fatalf("caller %d: %v", index, err)
		}
	}
	if got := fake.count(widgetsPath); got != 4 {
		t.Fatalf("expected two replays, got %d API requests", got)
	}
	if got := fake.count("/token"); got != 2 {
		t.Fatalf("expected one shared refresh, got %d token requests", got)
	}
}

// (d) a streaming operation is never replayed: the typed 401 surfaces.
func TestStreamingOperationsAreNeverReplayed(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	client := newReplayClient(t, fake, "feed")
	var sdkError *sdk.SDKError
	if _, err := client.StreamEvents(context.Background()); !errors.As(err, &sdkError) || sdkError.Status != 401 {
		t.Fatalf("expected the stream 401 to surface, got %v", err)
	}
	if got := fake.count(eventsPath); got != 1 {
		t.Fatalf("no replay for the streaming operation: %d", got)
	}
	if got := fake.count("/feed-token"); got != 1 {
		t.Fatalf("no refresh for the streaming operation: %d", got)
	}
}

// (e) replay disabled by default: the plain lifecycle surfaces the 401
// without any refresh. The hook serves through the lifecycle token method.
func TestReplayDisabledByDefault(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	fake.armStale()
	t.Setenv("OAUTH_SDK_CLIENT_ID", clientIDValue)
	t.Setenv("OAUTH_SDK_CLIENT_SECRET", clientSecretValue)
	var lifecycle *sdk.Client
	hook := func(ctx context.Context, request sdk.CredentialRequest) (sdk.Authorization, error) {
		set, err := lifecycle.ClientCredentialsToken(ctx, "service")
		if err != nil {
			return sdk.Authorization{}, err
		}
		return set.Authorization(), nil
	}
	client, err := sdk.NewClient(sdk.Credentials{}.WithHook("service", hook), sdk.ClientOptions{Transport: fake})
	if err != nil {
		t.Fatal(err)
	}
	lifecycle = client
	var sdkError *sdk.SDKError
	if _, err := client.ListWidgets(context.Background()); !errors.As(err, &sdkError) || sdkError.Status != 401 {
		t.Fatalf("expected the 401 to surface, got %v", err)
	}
	if got := fake.count(widgetsPath); got != 1 {
		t.Fatalf("no replay: %d", got)
	}
	if got := fake.count("/token"); got != 1 {
		t.Fatalf("no refresh: %d", got)
	}
}

// (f) refresh failure: the typed auth error surfaces instead of a replay.
func TestRefreshFailureIsTypedAndNeverReplays(t *testing.T) {
	fake := newFakeReplay()
	defer fake.server.Close()
	fake.armStale()
	fake.mu.Lock()
	fake.failFrom = fake.svcTokens + 2
	fake.mu.Unlock()
	client := newReplayClient(t, fake, "service")
	_, err := client.ListWidgets(context.Background())
	var authError *sdk.AuthError
	if !errors.As(err, &authError) {
		t.Fatalf("expected the typed auth failure, got %v", err)
	}
	if authError.Status != 500 {
		t.Fatalf("unexpected typed failure: %+v", authError)
	}
	if got := fake.count(widgetsPath); got != 1 {
		t.Fatalf("no replay after a failed refresh: %d", got)
	}
	if got := fake.count("/token"); got != 2 {
		t.Fatalf("the refresh was attempted exactly once: %d", got)
	}
}

var _ = io.Discard
var _ = strings.TrimSpace
"#;

#[test]
fn replay_lifecycle_runs_against_a_fake_server() {
    let Some(_) = go_toolchain() else {
        eprintln!("go_oauth: Go toolchain (>= 1.23) not installed; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&replay_document(), &replay_options()),
        root.path(),
    )
    .unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("go.mod"),
        "module example.com/oauth-consumer\n\ngo 1.23.0\nrequire example.com/oauth-sdk v0.1.0\nreplace example.com/oauth-sdk => ../go\n",
    )
    .unwrap();
    std::fs::write(consumer.join("replay_behavior_test.go"), REPLAY_BEHAVIOR).unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "-v", "."])
        .current_dir(&consumer)
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "replay OAuth lifecycle failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_oauth replay: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}
