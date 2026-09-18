//! Emitted first-party OAuth 2.0 lifecycle runtime for generated Go clients.
//!
//! The shared planner decides WHICH source schemes carry executable flows and
//! what configuration supplements them. This module lowers that selection into
//! one generated `go/oauth.go`: the compiled scheme descriptors embedded as
//! constants, an instance-owned token store, client-credentials acquisition
//! with skew-aware caching and per-client single-flight, explicit refresh and
//! — only when the compiled scheme carries them — authorization-code with PKCE
//! S256, RFC 8628 device authorization, RFC 7009 revocation and RFC 7662
//! introspection. Implicit and password flows are represented by the plan for
//! documentation only; they are never executed, so schemes with only those
//! flows emit nothing. A scheme carrying a discovery URL is lowered even with
//! no executable flows: OpenID Connect schemes have their endpoints defined by
//! the discovery document at runtime.
//!
//! Emission is strictly conditional and byte-identical for plans without a
//! discovery URL: without a configured `sdk_defaults` policy, or without any
//! usable scheme, the backend emits nothing new, and every emitted section has
//! a plain variant (exactly the pre-discovery bytes) assembled when no scheme
//! carries a discovery URL. Discovery-aware sections resolve the endpoints the
//! compiled plan omits through RFC 8414 / OpenID Connect discovery. Credential
//! values never enter emitted bytes: client identity comes from explicit call
//! options or the compiled environment variable names, read at call time.
//! Token endpoint requests use the client's own transport, so caller transport
//! policy (including test relays) covers the lifecycle too.

use super::emit::q;
use super::*;
use crate::http_protocol::{
    OAuthClientAuth, OAuthFlowDescriptor, OAuthFlowDescriptorKind, OAuthMode, OAuthPlan,
    OAuthSchemePlan,
};
use std::collections::{BTreeMap, BTreeSet};

/// Whether one compiled flow can be executed by the generated runtime.
fn executable(flow: &OAuthFlowDescriptor) -> bool {
    if flow.deprecated_flow {
        return false;
    }
    match flow.kind {
        OAuthFlowDescriptorKind::ClientCredentials => flow.token_url.is_some(),
        OAuthFlowDescriptorKind::AuthorizationCode => {
            flow.authorization_url.is_some() && flow.token_url.is_some()
        }
        OAuthFlowDescriptorKind::DeviceAuthorization => {
            flow.device_authorization_url.is_some() && flow.token_url.is_some()
        }
        // Implicit and password flows are never executed by generated code.
        OAuthFlowDescriptorKind::Implicit | OAuthFlowDescriptorKind::Password => false,
    }
}

/// The executable flow subset of one scheme, in declaration order.
fn executable_flows(scheme: &OAuthSchemePlan) -> Vec<&OAuthFlowDescriptor> {
    scheme
        .flows
        .iter()
        .filter(|flow| executable(flow))
        .collect()
}

/// Whether the compiled plan admits at least one usable scheme, so the runtime
/// file carries content. A scheme carrying a discovery URL is usable even
/// with no executable flows: the discovery document defines its endpoints.
#[must_use]
pub(super) fn emits(oauth: &OAuthPlan) -> bool {
    oauth.mode != OAuthMode::Off
        && oauth
            .schemes
            .iter()
            .any(|scheme| !executable_flows(scheme).is_empty() || scheme.discovery.is_some())
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials endpoint, so the replaying credential wrapper
/// participates. The wrapper serves exactly that provider, so schemes
/// without one compile exactly the pre-replay bytes.
#[must_use]
pub(super) fn has_client_credentials(oauth: &OAuthPlan) -> bool {
    oauth.mode != OAuthMode::Off
        && oauth.schemes.iter().any(|scheme| {
            executable_flows(scheme)
                .iter()
                .any(|flow| flow.kind == OAuthFlowDescriptorKind::ClientCredentials)
        })
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
pub(super) fn no_replay_requirements(
    oauth: &OAuthPlan,
    operations: &[super::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = oauth.schemes.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire().responses().iter().any(|response| {
            response.media().iter().any(|media| {
                matches!(
                    media.representation(),
                    crate::http_protocol::Representation::Stream { .. }
                )
            })
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire().security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    crate::http_protocol::CredentialHook::OAuth2 { .. }
                        | crate::http_protocol::CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if names.contains(requirement.name()) {
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(requirement.source().source().pointer().to_owned());
                }
            }
        }
    }
    pointers
}

/// Compiled OAuth planning carried by the plan: only a configured policy is
/// lowered, mirroring pagination helpers, so no-policy output stays
/// byte-identical for every document. Planning errors are the shared
/// planner's [`crate::http_protocol::HttpDiagnostic`]s.
pub(super) fn plan(
    contract: &Contract,
    wire: &protocol::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
) -> Result<Option<OAuthPlan>, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    if defaults.oauth.mode == OAuthMode::Off {
        return Ok(None);
    }
    let plan = protocol::plan_oauth(contract, wire, Some(defaults))?;
    Ok((!plan.schemes.is_empty()).then_some(plan))
}

/// Package-level declarations `go/oauth.go` owns when emitted. Reserved
/// against model symbols only while the file is emitted, so no-policy
/// allocation behavior is unchanged.
pub(super) const PACKAGE_NAMES: &[&str] = &[
    "AuthError",
    "AuthorizationTransaction",
    "DeviceAuthorization",
    "Introspection",
    "MemoryTokenStore",
    "NewMemoryTokenStore",
    "TokenOption",
    "TokenSet",
    "TokenStore",
    "WithAuthorizationScope",
    "WithRedirectURI",
    "WithTokenClientCredentials",
    "WithTokenStore",
];

/// Package-level declarations the replaying credential wrapper owns when its
/// section emits (an executable client-credentials endpoint exists).
pub(super) const REPLAY_PACKAGE_NAMES: &[&str] = &["NewReplayCredentials", "ReplayCredentials"];

/// Methods the emitted file adds to `*Client` (and the device transaction's
/// poll method). Reserved against operation method allocation only while the
/// file is emitted.
pub(super) const CLIENT_METHODS: &[&str] = &[
    "BeginAuthorization",
    "BeginDeviceAuthorization",
    "ClientCredentialsToken",
    "CompleteAuthorization",
    "IntrospectToken",
    "RefreshToken",
    "RevokeToken",
];

/// One usable scheme lowered into its compiled descriptor fields. A scheme
/// with no executable flow is lowered only when its discovery URL defines the
/// endpoints at runtime (OpenID Connect): every endpoint stays empty and the
/// discovery URL drives resolution.
struct Scheme {
    name: String,
    client_auth: &'static str,
    client_id_env: String,
    client_secret_env: String,
    skew: u32,
    token_url: String,
    refresh_url: String,
    client_credentials: String,
    authorization_url: String,
    code_token_url: String,
    device_url: String,
    device_token_url: String,
    revocation_url: String,
    introspection_url: String,
    discovery_url: String,
}

fn lower(scheme: &OAuthSchemePlan) -> Option<Scheme> {
    let flows = executable_flows(scheme);
    let discovery_url = scheme.discovery.clone().unwrap_or_default();
    let first = match flows.first() {
        Some(first) => first,
        None if !discovery_url.is_empty() => {
            return Some(Scheme {
                name: scheme.name.clone(),
                // A discovery-defined scheme has no compiled flow to derive
                // the client authentication from, so the compiled
                // configuration decides: client-secret-basic when a client
                // secret variable is configured, else the public profile.
                client_auth: if scheme.client_secret_env.is_some() {
                    "client-secret-basic"
                } else {
                    "none"
                },
                client_id_env: scheme.client_id_env.clone().unwrap_or_default(),
                client_secret_env: scheme.client_secret_env.clone().unwrap_or_default(),
                skew: scheme.refresh_skew_seconds,
                token_url: String::new(),
                refresh_url: String::new(),
                client_credentials: String::new(),
                authorization_url: String::new(),
                code_token_url: String::new(),
                device_url: String::new(),
                device_token_url: String::new(),
                revocation_url: scheme.revocation_endpoint.clone().unwrap_or_default(),
                introspection_url: scheme.introspection_endpoint.clone().unwrap_or_default(),
                discovery_url,
            });
        }
        None => return None,
    };
    let mut lowered = Scheme {
        name: scheme.name.clone(),
        client_auth: match first.client_auth {
            OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
            OAuthClientAuth::None => "none",
        },
        client_id_env: scheme.client_id_env.clone().unwrap_or_default(),
        client_secret_env: scheme.client_secret_env.clone().unwrap_or_default(),
        skew: scheme.refresh_skew_seconds,
        token_url: first.token_url.clone().unwrap_or_default(),
        refresh_url: first
            .refresh_url
            .clone()
            .or_else(|| first.token_url.clone())
            .unwrap_or_default(),
        client_credentials: String::new(),
        authorization_url: String::new(),
        code_token_url: String::new(),
        device_url: String::new(),
        device_token_url: String::new(),
        revocation_url: scheme.revocation_endpoint.clone().unwrap_or_default(),
        introspection_url: scheme.introspection_endpoint.clone().unwrap_or_default(),
        discovery_url,
    };
    for flow in flows {
        match flow.kind {
            OAuthFlowDescriptorKind::ClientCredentials if lowered.client_credentials.is_empty() => {
                lowered.client_credentials = flow.token_url.clone().unwrap_or_default();
            }
            OAuthFlowDescriptorKind::AuthorizationCode if lowered.authorization_url.is_empty() => {
                lowered.authorization_url = flow.authorization_url.clone().unwrap_or_default();
                lowered.code_token_url = flow.token_url.clone().unwrap_or_default();
            }
            OAuthFlowDescriptorKind::DeviceAuthorization if lowered.device_url.is_empty() => {
                lowered.device_url = flow.device_authorization_url.clone().unwrap_or_default();
                lowered.device_token_url = flow.token_url.clone().unwrap_or_default();
            }
            _ => {}
        }
    }
    Some(lowered)
}

/// Render `go/oauth.go`: the lifecycle runtime plus one compiled descriptor per
/// usable scheme. Called only when at least one usable scheme exists. Plans
/// without a discovery URL assemble byte-identically to the pre-discovery
/// emission. The replaying credential wrapper joins only when a compiled
/// scheme carries an executable client-credentials endpoint; everything else
/// stays byte-identical.
pub(super) fn emit(oauth: &OAuthPlan, no_replay: &BTreeMap<String, BTreeSet<String>>) -> String {
    let schemes: Vec<Scheme> = oauth.schemes.iter().filter_map(lower).collect();
    let has_authorization = schemes.iter().any(|s| !s.authorization_url.is_empty());
    let has_device = schemes.iter().any(|s| !s.device_url.is_empty());
    let has_revocation = schemes.iter().any(|s| !s.revocation_url.is_empty());
    let has_introspection = schemes.iter().any(|s| !s.introspection_url.is_empty());
    let discovery = schemes.iter().any(|s| !s.discovery_url.is_empty());

    let mut imports = BTreeSet::from([
        "context",
        "encoding/json",
        "errors",
        "io",
        "net/http",
        "net/url",
        "os",
        "strings",
        "sync",
        "time",
    ]);
    if has_client_credentials(oauth) {
        imports.insert("bytes");
    }
    if has_authorization {
        imports.extend([
            "crypto/rand",
            "crypto/sha256",
            "encoding/base64",
            "crypto/subtle",
        ]);
    }

    let mut code = String::from(
        "// Code generated by suspect. DO NOT EDIT.\n//\n// First-party OAuth 2.0 lifecycle for the source-declared schemes compiled into\n// oauthSchemes below. Every endpoint, environment variable name and policy\n// value is a generation-time constant: the runtime never reads OpenAPI, never\n// invents endpoints and never embeds credential values. Client identity comes\n// from explicit call options or the compiled environment variable names, read\n// at call time.\n//\n// Implemented here: client-credentials acquisition with skew-aware caching and\n// per-client single-flight, explicit refresh, authorization-code with PKCE\n// S256, RFC 8628 device authorization with interval, authorization_pending,\n// slow_down and expiry polling, RFC 7009 revocation and RFC 7662\n// introspection — exactly for the schemes compiled below. Implicit and\n// password flows are never executed: generated code does not perform\n// interactive resource-owner credential handling. ",
    );
    code.push_str(if discovery {
        "A compiled discovery URL resolves the endpoints the compiled plan\n// omits at call time, through RFC 8414 / OpenID Connect discovery.\n"
    } else {
        "OpenID Connect discovery\n// remains caller-owned for schemes without an executable declared flow.\n"
    });
    code.push_str("//\n// Token sets live only in the token store of the owning client instance,\n// under keys partitioned by scheme, token-endpoint issuer and client\n// identity; there is no process-global token cache. Error values never carry\n// token or client-secret material. Token endpoint requests use the client's\n// own transport, so caller transport policy covers the lifecycle too.\n//\n// Compiled source schemes: ");
    let names = schemes
        .iter()
        .map(|scheme| scheme.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    code.push_str(&names);
    code.push_str(".\npackage sdk\n\nimport (\n");
    for import in &imports {
        code.push_str(&format!("\t\"{import}\"\n"));
    }
    code.push_str(")\n\n");
    code.push_str(CORE_A);
    code.push_str(if discovery {
        CORE_STATE_DISCOVERY
    } else {
        CORE_STATE_PLAIN
    });
    code.push_str(CORE_B);
    if discovery {
        code.push_str(CLIENT_CREDENTIALS_TOKEN_DISCOVERY);
        code.push_str(REFRESH_TOKEN_DISCOVERY);
    } else {
        code.push_str(CLIENT_CREDENTIALS_TOKEN);
        code.push_str(REFRESH_TOKEN);
    }
    code.push('\n');
    code.push_str(&descriptors(&schemes));
    if has_revocation {
        code.push_str(if discovery {
            REVOCATION_DISCOVERY
        } else {
            REVOCATION
        });
    }
    if has_introspection {
        code.push_str(if discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION
        });
    }
    if has_authorization {
        code.push_str(AUTHORIZATION_CODE);
    }
    if has_device {
        code.push_str(DEVICE);
    }
    if discovery {
        code.push_str(DISCOVERY);
    }
    if has_client_credentials(oauth) {
        code.push_str(&replay_section(oauth, no_replay));
    }
    code
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations.
fn replay_section(oauth: &OAuthPlan, no_replay: &BTreeMap<String, BTreeSet<String>>) -> String {
    let discovery = oauth
        .schemes
        .iter()
        .any(|scheme| scheme.discovery.is_some());
    let mut code = String::from(
        "\n// oauthNoReplayRequirements carries the security-requirement source pointers\n// whose attaches are never replayed: requirements that name the scheme on an\n// operation whose responses carry a stream representation. Delivered stream\n// data prevents a transparent restart.\nvar oauthNoReplayRequirements = map[string][]string{\n",
    );
    for scheme in &oauth.schemes {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| q(pointer))
            .collect::<Vec<_>>()
            .join(", ");
        code.push_str(&format!("\t{}: {{{}}},\n", q(&scheme.name), rendered));
    }
    code.push_str("}\n\n");
    code.push_str(REPLAY_CORE_A);
    code.push_str(if discovery {
        REPLAY_LIFECYCLE_DISCOVERY
    } else {
        REPLAY_LIFECYCLE_PLAIN
    });
    code.push_str(REPLAY_CORE_B);
    code
}

/// The compiled descriptor map: one entry per usable source scheme. The head
/// carries the descriptor struct: byte-exact without discovery, with the
/// discovery URL field when one compiles.
fn descriptors(schemes: &[Scheme]) -> String {
    let mut code = String::from(
        if schemes
            .iter()
            .any(|scheme| !scheme.discovery_url.is_empty())
        {
            DESCRIPTORS_HEAD_DISCOVERY
        } else {
            DESCRIPTORS_HEAD_PLAIN
        },
    );
    for scheme in schemes {
        code.push_str(&format!("\t{}: {{\n", q(&scheme.name)));
        code.push_str(&format!("\t\tName: {},\n", q(&scheme.name)));
        code.push_str(&format!("\t\tClientAuth: {},\n", q(scheme.client_auth)));
        code.push_str(&format!(
            "\t\tClientIDEnv: {}, ClientSecretEnv: {},\n",
            q(&scheme.client_id_env),
            q(&scheme.client_secret_env)
        ));
        code.push_str(&format!("\t\tSkew: {} * time.Second,\n", scheme.skew));
        code.push_str(&format!(
            "\t\tTokenURL: {}, RefreshURL: {},\n",
            q(&scheme.token_url),
            q(&scheme.refresh_url)
        ));
        code.push_str(&format!(
            "\t\tClientCredentials: {}, AuthorizationURL: {}, CodeTokenURL: {},\n",
            q(&scheme.client_credentials),
            q(&scheme.authorization_url),
            q(&scheme.code_token_url)
        ));
        code.push_str(&format!(
            "\t\tDeviceURL: {}, DeviceTokenURL: {},\n",
            q(&scheme.device_url),
            q(&scheme.device_token_url)
        ));
        code.push_str(&format!(
            "\t\tRevocationURL: {}, IntrospectionURL: {},\n",
            q(&scheme.revocation_url),
            q(&scheme.introspection_url)
        ));
        if !scheme.discovery_url.is_empty() {
            code.push_str(&format!(
                "\t\tDiscoveryURL: {},\n",
                q(&scheme.discovery_url)
            ));
        }
        code.push_str("\t},\n");
    }
    code.push_str("}\n\n");
    code
}

/// The descriptor struct head: byte-exact without discovery.
const DESCRIPTORS_HEAD_PLAIN: &str = "// oauthSchemeDescriptor is the compiled lifecycle descriptor of one\n// source scheme. Zero fields mean \"not compiled\": endpoints are never\n// invented and supplemental endpoints exist only when configuration supplied\n// them.\ntype oauthSchemeDescriptor struct {\n\t// Name is the source Security Scheme name used by every lifecycle call.\n\tName string\n\t// ClientAuth is \"client-secret-basic\" when the compiled configuration\n\t// supplies a client secret variable, else \"none\" (a public client).\n\tClientAuth string\n\t// ClientIDEnv and ClientSecretEnv name the environment variables read at\n\t// call time; generation embeds no credential values.\n\tClientIDEnv, ClientSecretEnv string\n\t// Skew is the refresh-before-expiry clock skew.\n\tSkew time.Duration\n\t// TokenURL is the first executable flow's token endpoint; RefreshURL is\n\t// the declared refresh URL, else the token endpoint.\n\tTokenURL, RefreshURL string\n\t// Per-flow endpoints, empty when the flow is absent or not executable.\n\tClientCredentials, AuthorizationURL, CodeTokenURL string\n\tDeviceURL, DeviceTokenURL string\n\t// Supplemental endpoints, compiled from configuration only.\n\tRevocationURL, IntrospectionURL string\n}\n\n// oauthSchemes carries the usable source schemes. Schemes whose only declared\n// flows are implicit or password are never compiled here, so they emit\n// nothing.\nvar oauthSchemes = map[string]*oauthSchemeDescriptor{\n";

/// The descriptor struct head with the discovery URL field, emitted when at
/// least one scheme compiles a discovery URL.
const DESCRIPTORS_HEAD_DISCOVERY: &str = "// oauthSchemeDescriptor is the compiled lifecycle descriptor of one\n// source scheme. Zero fields mean \"not compiled\": endpoints are never\n// invented and supplemental endpoints exist only when configuration supplied\n// them.\ntype oauthSchemeDescriptor struct {\n\t// Name is the source Security Scheme name used by every lifecycle call.\n\tName string\n\t// ClientAuth is \"client-secret-basic\" when the compiled configuration\n\t// supplies a client secret variable, else \"none\" (a public client).\n\tClientAuth string\n\t// ClientIDEnv and ClientSecretEnv name the environment variables read at\n\t// call time; generation embeds no credential values.\n\tClientIDEnv, ClientSecretEnv string\n\t// Skew is the refresh-before-expiry clock skew.\n\tSkew time.Duration\n\t// TokenURL is the first executable flow's token endpoint; RefreshURL is\n\t// the declared refresh URL, else the token endpoint.\n\tTokenURL, RefreshURL string\n\t// Per-flow endpoints, empty when the flow is absent or not executable.\n\tClientCredentials, AuthorizationURL, CodeTokenURL string\n\tDeviceURL, DeviceTokenURL string\n\t// Supplemental endpoints, compiled from configuration only.\n\tRevocationURL, IntrospectionURL string\n\t// DiscoveryURL is the compiled metadata document (RFC 8414 / OpenID\n\t// Connect discovery); endpoints the fields above leave empty resolve\n\t// through it at call time.\n\tDiscoveryURL string\n}\n\n// oauthSchemes carries the usable source schemes. Schemes whose only declared\n// flows are implicit or password are never compiled here, so they emit\n// nothing; a scheme carrying a discovery URL compiles with empty endpoints\n// and resolves them through the document at call time.\nvar oauthSchemes = map[string]*oauthSchemeDescriptor{\n";

/// Store, error, client-state, option and request plumbing shared by every
/// compiled flow. The constants concatenate into exactly the pre-discovery
/// core bytes for plans without a discovery URL.
const CORE_A: &str = r#"
// TokenSet is one decoded token-endpoint response (RFC 6749 section 5.1).
// ExpiresAt is derived from expires_in; a zero ExpiresAt means the server
// declared no lifetime, so the set never expires locally. RefreshToken holds
// the server's rotated token, or the previous set's token when the server
// returned none.
type TokenSet struct {
	AccessToken  string
	TokenType    string
	ExpiresAt    time.Time
	RefreshToken string
	Scope        string
	hasRefresh   bool
}

// Authorization converts the set into the explicit credential value consumed
// by source-declared oauth2 and openid-connect operations: the server's token
// type, or the conventional Bearer when the response omitted one.
func (t *TokenSet) Authorization() Authorization {
	if t == nil {
		return Authorization{}
	}
	scheme := t.TokenType
	if scheme == "" {
		scheme = "Bearer"
	}
	return Authorization{Scheme: scheme, Value: t.AccessToken}
}

// oauthExpired reports whether the set is stale at the skew-adjusted expiry.
func (t *TokenSet) oauthExpired(skew time.Duration) bool {
	if t == nil || t.ExpiresAt.IsZero() {
		return false
	}
	return !t.ExpiresAt.Add(-skew).After(time.Now())
}

// TokenStore persists token sets under opaque keys. Implementations must be
// safe for concurrent use. Keys are partitioned by scheme, token-endpoint
// issuer and client identity; treat them as read-only routing information.
type TokenStore interface {
	Load(ctx context.Context, key string) (*TokenSet, error)
	Replace(ctx context.Context, key string, set *TokenSet) error
	Clear(ctx context.Context, key string) error
}

// MemoryTokenStore is an instance-owned in-process token store. Each instance
// guards its own keys; generated clients never share a store implicitly and
// there is no package-level token cache.
type MemoryTokenStore struct {
	mu   sync.Mutex
	sets map[string]*TokenSet
}

// NewMemoryTokenStore returns a fresh, independent in-process store. The
// caller owns the instance: share it only by passing it explicitly.
func NewMemoryTokenStore() *MemoryTokenStore {
	return &MemoryTokenStore{sets: make(map[string]*TokenSet)}
}

// Load returns a copy of the stored set, or nil when the key holds none.
func (s *MemoryTokenStore) Load(_ context.Context, key string) (*TokenSet, error) {
	if s == nil {
		return nil, nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	stored := s.sets[key]
	if stored == nil {
		return nil, nil
	}
	copied := *stored
	return &copied, nil
}

// Replace stores a copy of set under key, replacing any previous value
// atomically.
func (s *MemoryTokenStore) Replace(_ context.Context, key string, set *TokenSet) error {
	if s == nil {
		return errors.New("nil token store")
	}
	if set == nil {
		return errors.New("token store replacement requires a set")
	}
	copied := *set
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.sets == nil {
		s.sets = make(map[string]*TokenSet)
	}
	s.sets[key] = &copied
	return nil
}

// Clear drops the key; clearing an absent key succeeds.
func (s *MemoryTokenStore) Clear(_ context.Context, key string) error {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.sets, key)
	return nil
}

// AuthError reports an OAuth lifecycle failure. Kind and Scheme classify it;
// Status carries the endpoint HTTP status when one was reached; Code and
// Description carry the server's declared error. Error text and fields never
// contain token or client-secret values.
type AuthError struct {
	Kind        string
	Scheme      string
	Status      int
	Code        string
	Description string
	Cause       error
}

func (e *AuthError) Error() string {
	text := "oauth " + e.Kind + " failure"
	if e.Scheme != "" {
		text += " for scheme " + e.Scheme
	}
	if e.Code != "" {
		text += ": " + e.Code
	}
	return text
}

func (e *AuthError) Unwrap() error { return e.Cause }

"#;

/// The per-client OAuth state struct: byte-exact without discovery, with the
/// per-scheme discovery cache added when one compiles.
const CORE_STATE_PLAIN: &str = r#"// oauthClientState is one client instance's generated OAuth state: its default
// token store and the per-key single-flight gates. State is created lazily,
// belongs to exactly one client and is never shared between clients.
type oauthClientState struct {
	mu    sync.Mutex
	gates map[string]*oauthGate
	store TokenStore
}

"#;
const CORE_STATE_DISCOVERY: &str = r#"// oauthClientState is one client instance's generated OAuth state: its default
// token store, the per-key single-flight gates and the per-scheme discovery
// documents. State is created lazily, belongs to exactly one client and is
// never shared between clients.
type oauthClientState struct {
	mu    sync.Mutex
	gates map[string]*oauthGate
	store TokenStore
	// discovery caches one fetched metadata document per scheme for this
	// client's lifetime; failed fetches are never cached.
	discovery map[string]*oauthDiscoveredEndpoints
}

"#;

const CORE_B: &str = r#"// oauthGate serializes one store key's acquisition. The buffered channel is
// the mutex; waiting callers give up when their context completes first.
type oauthGate struct{ slot chan struct{} }

func newOAuthGate() *oauthGate { return &oauthGate{slot: make(chan struct{}, 1)} }

// lock holds the gate, or reports the caller's context completion first.
func (g *oauthGate) lock(ctx context.Context) error {
	select {
	case g.slot <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (g *oauthGate) unlock() { <-g.slot }

// oauthStates locates each client's own state. It is ownership bookkeeping for
// clients using their default store, not a token cache: token values live only
// inside each client's own store.
var oauthStates sync.Map

func (c *Client) oauthState() *oauthClientState {
	if state, found := oauthStates.Load(c); found {
		return state.(*oauthClientState)
	}
	created := &oauthClientState{gates: make(map[string]*oauthGate)}
	state, _ := oauthStates.LoadOrStore(c, created)
	return state.(*oauthClientState)
}

func (s *oauthClientState) gate(key string) *oauthGate {
	s.mu.Lock()
	defer s.mu.Unlock()
	gate := s.gates[key]
	if gate == nil {
		gate = newOAuthGate()
		s.gates[key] = gate
	}
	return gate
}

// store returns the client's default store, creating it on first use.
func (s *oauthClientState) defaultStore() TokenStore {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.store == nil {
		s.store = NewMemoryTokenStore()
	}
	return s.store
}

// TokenOption supplies explicit inputs for one token call. Explicit values win
// over the compiled environment variable names; neither generation nor this
// runtime ever bakes credential values into emitted bytes.
type TokenOption func(*oauthTokenOptions)

type oauthTokenOptions struct {
	clientID     string
	clientSecret string
	store        TokenStore
}

// WithTokenClientCredentials supplies the client identity and secret for this
// call, overriding the compiled environment variable names.
func WithTokenClientCredentials(clientID, clientSecret string) TokenOption {
	return func(options *oauthTokenOptions) {
		options.clientID, options.clientSecret = clientID, clientSecret
	}
}

// WithTokenStore supplies the store for this call, overriding the client's
// default store.
func WithTokenStore(store TokenStore) TokenOption {
	return func(options *oauthTokenOptions) { options.store = store }
}

// oauthClientID resolves the client identity for calls without explicit
// options: the compiled environment variable name, read at call time.
func (d *oauthSchemeDescriptor) oauthClientID() string {
	if d.ClientIDEnv == "" {
		return ""
	}
	return os.Getenv(d.ClientIDEnv)
}

// oauthCredentials resolves the call's client identity: explicit options
// first, then the compiled variable names read at call time. Empty values mean
// absent; callers may legitimately configure only one.
func (d *oauthSchemeDescriptor) oauthCredentials(options *oauthTokenOptions) (string, string) {
	id, secret := options.clientID, options.clientSecret
	if id == "" {
		id = d.oauthClientID()
	}
	if secret == "" && d.ClientSecretEnv != "" {
		secret = os.Getenv(d.ClientSecretEnv)
	}
	return id, secret
}

// oauthStoreKey partitions tokens by scheme, token-endpoint issuer and client
// identity, so distinct clients and endpoints never share a set.
func oauthStoreKey(scheme, tokenURL, clientID string) string {
	return scheme + "\x1f" + tokenURL + "\x1f" + clientID
}

// oauthMaxResponseBytes bounds one token/device/revocation/introspection
// response.
const oauthMaxResponseBytes = 1 << 20

type oauthTokenResponse struct {
	AccessToken      string `json:"access_token"`
	TokenType        string `json:"token_type"`
	ExpiresIn        int64  `json:"expires_in"`
	RefreshToken     string `json:"refresh_token"`
	Scope            string `json:"scope"`
	Error            string `json:"error"`
	ErrorDescription string `json:"error_description"`
}

// oauthEndpointRequest posts one form-encoded request to a compiled OAuth
// endpoint and returns the status and bounded body. It applies the scheme's
// compiled client authentication: HTTP Basic for confidential clients, the
// client_id form member for public ones.
func (c *Client) oauthEndpointRequest(ctx context.Context, scheme, endpoint, clientID, clientSecret string, form url.Values) (int, []byte, *AuthError) {
	if ctx == nil {
		return 0, nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	if endpoint == "" {
		return 0, nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	if err := ctx.Err(); err != nil {
		return 0, nil, &AuthError{Kind: "cancelled", Scheme: scheme, Cause: err}
	}
	basic := false
	if descriptor := oauthSchemes[scheme]; descriptor != nil && descriptor.ClientAuth == "client-secret-basic" {
		basic = true
	} else if clientID != "" {
		form.Set("client_id", clientID)
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, strings.NewReader(form.Encode()))
	if err != nil {
		return 0, nil, &AuthError{Kind: "request-representation", Scheme: scheme, Cause: err}
	}
	if basic {
		request.SetBasicAuth(url.QueryEscape(clientID), url.QueryEscape(clientSecret))
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Accept", "application/json")
	response, err := c.transport.Do(request)
	if err != nil {
		if response != nil && response.Body != nil {
			_ = response.Body.Close()
		}
		return 0, nil, &AuthError{Kind: "transport", Scheme: scheme, Cause: httpCause(ctx, err)}
	}
	body, readErr := io.ReadAll(io.LimitReader(response.Body, oauthMaxResponseBytes+1))
	_ = response.Body.Close()
	if readErr != nil {
		return 0, nil, &AuthError{Kind: "transport", Scheme: scheme, Status: response.StatusCode, Cause: httpCause(ctx, readErr)}
	}
	if len(body) > oauthMaxResponseBytes {
		return 0, nil, &AuthError{Kind: "resource-limit", Scheme: scheme, Status: response.StatusCode}
	}
	return response.StatusCode, body, nil
}

// oauthTokenRequest executes one RFC 6749 token-endpoint request and decodes
// the response. previous retains its refresh token when the server returns no
// rotated one.
func (c *Client) oauthTokenRequest(ctx context.Context, scheme, endpoint, clientID, clientSecret string, form url.Values, previous *TokenSet) (*TokenSet, error) {
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, endpoint, clientID, clientSecret, form)
	if failure != nil {
		return nil, failure
	}
	var decoded oauthTokenResponse
	if err := json.Unmarshal(body, &decoded); err != nil {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status, Cause: err}
	}
	if decoded.Error != "" {
		return nil, &AuthError{Kind: "authorization-error", Scheme: scheme, Status: status, Code: decoded.Error, Description: decoded.ErrorDescription}
	}
	if status < 200 || status >= 300 {
		return nil, &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	if decoded.AccessToken == "" {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status}
	}
	set := &TokenSet{
		AccessToken:  decoded.AccessToken,
		TokenType:    decoded.TokenType,
		RefreshToken: decoded.RefreshToken,
		Scope:        decoded.Scope,
		hasRefresh:   decoded.RefreshToken != "",
	}
	if decoded.ExpiresIn > 0 {
		set.ExpiresAt = time.Now().Add(time.Duration(decoded.ExpiresIn) * time.Second)
	}
	// Adopt a rotated refresh token; retain the previous one otherwise.
	if !set.hasRefresh && previous != nil && previous.hasRefresh {
		set.RefreshToken = previous.RefreshToken
		set.hasRefresh = true
	}
	return set, nil
}

"#;

/// The compiled client-credentials acquisition: byte-exact without discovery.
const CLIENT_CREDENTIALS_TOKEN: &str = r#"// ClientCredentialsToken returns the scheme's cached token set, acquiring one
// from the compiled client-credentials endpoint when the stored set is absent
// or expired beyond the compiled skew. Concurrent callers on one client share
// a single acquisition: callers waiting on the gate re-read the store instead
// of acquiring again. The returned set is a copy; callers may keep it.
func (c *Client) ClientCredentialsToken(ctx context.Context, scheme string, opts ...TokenOption) (*TokenSet, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.ClientCredentials == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	if ctx == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	var options oauthTokenOptions
	for _, apply := range opts {
		apply(&options)
	}
	clientID, clientSecret := descriptor.oauthCredentials(&options)
	store := options.store
	if store == nil {
		store = c.oauthState().defaultStore()
	}
	key := oauthStoreKey(scheme, descriptor.ClientCredentials, clientID)
	stored, err := store.Load(ctx, key)
	if err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	if stored != nil && !stored.oauthExpired(descriptor.Skew) {
		return stored, nil
	}
	gate := c.oauthState().gate(key)
	if err := gate.lock(ctx); err != nil {
		return nil, &AuthError{Kind: "cancelled", Scheme: scheme, Cause: err}
	}
	defer gate.unlock()
	// A caller that waited on the gate re-checks the store before acquiring:
	// the holder may have populated it already.
	stored, err = store.Load(ctx, key)
	if err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	if stored != nil && !stored.oauthExpired(descriptor.Skew) {
		return stored, nil
	}
	set, err := c.oauthTokenRequest(ctx, scheme, descriptor.ClientCredentials, clientID, clientSecret, url.Values{"grant_type": {"client_credentials"}}, stored)
	if err != nil {
		return nil, err
	}
	if err := store.Replace(ctx, key, set); err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	return set, nil
}

"#;

const REFRESH_TOKEN: &str = r#"// RefreshToken exchanges the set's refresh token (RFC 6749 section 6) at the
// scheme's declared refresh URL, or its token endpoint when no refresh URL is
// declared. The returned set adopts a rotated refresh token and retains the
// given one otherwise. The store is neither read nor updated: callers decide
// which set to keep.
func (c *Client) RefreshToken(ctx context.Context, scheme string, set *TokenSet) (*TokenSet, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if set == nil || !set.hasRefresh || set.RefreshToken == "" {
		return nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	if descriptor.RefreshURL == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	form := url.Values{"grant_type": {"refresh_token"}, "refresh_token": {set.RefreshToken}}
	return c.oauthTokenRequest(ctx, scheme, descriptor.RefreshURL, clientID, clientSecret, form, set)
}
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries a
/// revocation endpoint.
const REVOCATION: &str = r#"
// RevokeToken posts the token value to the scheme's compiled revocation
// endpoint (RFC 7009). Any 2xx response is success: RFC 7009 declares the
// token revoked even when the server reports an unsupported-token error.
func (c *Client) RevokeToken(ctx context.Context, scheme, tokenValue string) error {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.RevocationURL == "" {
		return &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, descriptor.RevocationURL, clientID, clientSecret, url.Values{"token": {tokenValue}})
	if failure != nil {
		return failure
	}
	var decoded oauthTokenResponse
	if err := json.Unmarshal(body, &decoded); err == nil && decoded.Error != "" {
		return &AuthError{Kind: "authorization-error", Scheme: scheme, Status: status, Code: decoded.Error, Description: decoded.ErrorDescription}
	}
	if status < 200 || status >= 300 {
		return &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	return nil
}
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries an
/// introspection endpoint.
const INTROSPECTION: &str = r#"
// Introspection is one RFC 7662 introspection response. Timestamps derive
// from the numeric epoch fields; zero means the server omitted them. The
// response describes the token without returning it.
type Introspection struct {
	Active    bool
	Scope     string
	ClientID  string
	TokenType string
	Username  string
	ExpiresAt time.Time
	IssuedAt  time.Time
	NotBefore time.Time
	Subject   string
	Audience  []string
	Issuer    string
	JWTID     string
}

type oauthIntrospectionResponse struct {
	Active    bool     `json:"active"`
	Scope     string   `json:"scope"`
	ClientID  string   `json:"client_id"`
	TokenType string   `json:"token_type"`
	Username  string   `json:"username"`
	ExpiresAt int64    `json:"exp"`
	IssuedAt  int64    `json:"iat"`
	NotBefore int64    `json:"nbf"`
	Subject   string   `json:"sub"`
	Audience  []string `json:"aud"`
	Issuer    string   `json:"iss"`
	JWTID     string   `json:"jti"`
}

// IntrospectToken queries the scheme's compiled introspection endpoint
// (RFC 7662) with the token value.
func (c *Client) IntrospectToken(ctx context.Context, scheme, tokenValue string) (*Introspection, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.IntrospectionURL == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, descriptor.IntrospectionURL, clientID, clientSecret, url.Values{"token": {tokenValue}})
	if failure != nil {
		return nil, failure
	}
	if status < 200 || status >= 300 {
		return nil, &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	var decoded oauthIntrospectionResponse
	if err := json.Unmarshal(body, &decoded); err != nil {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status, Cause: err}
	}
	converted := &Introspection{
		Active:    decoded.Active,
		Scope:     decoded.Scope,
		ClientID:  decoded.ClientID,
		TokenType: decoded.TokenType,
		Username:  decoded.Username,
		Subject:   decoded.Subject,
		Audience:  decoded.Audience,
		Issuer:    decoded.Issuer,
		JWTID:     decoded.JWTID,
	}
	if decoded.ExpiresAt > 0 {
		converted.ExpiresAt = time.Unix(decoded.ExpiresAt, 0)
	}
	if decoded.IssuedAt > 0 {
		converted.IssuedAt = time.Unix(decoded.IssuedAt, 0)
	}
	if decoded.NotBefore > 0 {
		converted.NotBefore = time.Unix(decoded.NotBefore, 0)
	}
	return converted, nil
}
"#;

/// Authorization code with PKCE S256, emitted only when a compiled scheme
/// carries an executable authorization-code flow.
const AUTHORIZATION_CODE: &str = r#"
// AuthorizationOption supplies explicit authorization-code inputs.
type AuthorizationOption func(*oauthAuthorizationOptions)

type oauthAuthorizationOptions struct {
	redirectURI string
	scopes      []string
}

// WithRedirectURI supplies the callback redirect URI. When the authorization
// request carries one, the code exchange must repeat it exactly.
func WithRedirectURI(uri string) AuthorizationOption {
	return func(options *oauthAuthorizationOptions) { options.redirectURI = uri }
}

// WithAuthorizationScope requests the given source-declared scopes for this
// transaction, joined into the RFC 6749 space-separated form.
func WithAuthorizationScope(scopes ...string) AuthorizationOption {
	return func(options *oauthAuthorizationOptions) { options.scopes = scopes }
}

// AuthorizationTransaction is one started authorization-code transaction with
// a bound PKCE verifier and a one-time state. The state and verifier leave
// this process only through AuthorizationURL and the code exchange.
type AuthorizationTransaction struct {
	// Scheme names the source security scheme.
	Scheme string
	// AuthorizationURL directs the resource owner to the compiled
	// authorization endpoint with client identity, scope, state and the S256
	// code challenge.
	AuthorizationURL string
	// State must match the callback's state parameter exactly; it is consumed
	// by the first CompleteAuthorization call.
	State string
	// CreatedAt marks transaction start; lifetime policy belongs to the
	// authorization server.
	CreatedAt time.Time

	secret *oauthAuthorizationSecret
}

type oauthAuthorizationSecret struct {
	mu          sync.Mutex
	consumed    bool
	verifier    string
	tokenURL    string
	redirectURI string
}

// oauthRandomValue returns 256 bits of crypto/rand entropy in the RFC 7636
// base64url alphabet: 43 characters, no padding.
func oauthRandomValue() (string, error) {
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(raw), nil
}

// oauthChallenge derives the S256 code challenge from the verifier.
func oauthChallenge(verifier string) []byte {
	digest := sha256.Sum256([]byte(verifier))
	return digest[:]
}

// BeginAuthorization starts an authorization-code transaction with PKCE S256:
// it allocates the state and verifier, binds them to the returned transaction
// and renders the complete authorization URL. It performs no network call; the
// caller directs the user to AuthorizationURL and completes the transaction
// with the callback parameters.
func (c *Client) BeginAuthorization(scheme string, opts ...AuthorizationOption) (*AuthorizationTransaction, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.AuthorizationURL == "" || descriptor.CodeTokenURL == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	var options oauthAuthorizationOptions
	for _, apply := range opts {
		apply(&options)
	}
	verifier, err := oauthRandomValue()
	if err != nil {
		return nil, &AuthError{Kind: "random", Scheme: scheme, Cause: err}
	}
	state, err := oauthRandomValue()
	if err != nil {
		return nil, &AuthError{Kind: "random", Scheme: scheme, Cause: err}
	}
	values := url.Values{
		"response_type":         {"code"},
		"state":                 {state},
		"code_challenge":        {base64.RawURLEncoding.EncodeToString(oauthChallenge(verifier))},
		"code_challenge_method": {"S256"},
	}
	if clientID := descriptor.oauthClientID(); clientID != "" {
		values.Set("client_id", clientID)
	}
	if options.redirectURI != "" {
		values.Set("redirect_uri", options.redirectURI)
	}
	if scope := strings.Join(options.scopes, " "); scope != "" {
		values.Set("scope", scope)
	}
	separator := "?"
	if strings.Contains(descriptor.AuthorizationURL, "?") {
		separator = "&"
	}
	return &AuthorizationTransaction{
		Scheme:           scheme,
		AuthorizationURL: descriptor.AuthorizationURL + separator + values.Encode(),
		State:            state,
		CreatedAt:        time.Now(),
		secret: &oauthAuthorizationSecret{
			verifier:    verifier,
			tokenURL:    descriptor.CodeTokenURL,
			redirectURI: options.redirectURI,
		},
	}, nil
}

// CompleteAuthorization validates the callback's state against the
// transaction's bound state, exchanges the code with the retained PKCE
// verifier and returns the resulting token set. The transaction is consumed by
// the first call, whatever its outcome: a second call is a typed state
// failure.
func (c *Client) CompleteAuthorization(ctx context.Context, txn *AuthorizationTransaction, callbackParams map[string]string) (*TokenSet, error) {
	if txn == nil || txn.secret == nil {
		return nil, &AuthError{Kind: "request-validation"}
	}
	secret := txn.secret
	secret.mu.Lock()
	consumed := secret.consumed
	secret.consumed = true
	verifier, redirectURI, tokenURL := secret.verifier, secret.redirectURI, secret.tokenURL
	secret.mu.Unlock()
	if consumed {
		return nil, &AuthError{Kind: "state-consumed", Scheme: txn.Scheme}
	}
	if subtle.ConstantTimeCompare([]byte(callbackParams["state"]), []byte(txn.State)) != 1 {
		return nil, &AuthError{Kind: "state-mismatch", Scheme: txn.Scheme}
	}
	if declared := callbackParams["error"]; declared != "" {
		return nil, &AuthError{Kind: "authorization-error", Scheme: txn.Scheme, Code: declared, Description: callbackParams["error_description"]}
	}
	code := callbackParams["code"]
	if code == "" {
		return nil, &AuthError{Kind: "invalid-response", Scheme: txn.Scheme}
	}
	descriptor := oauthSchemes[txn.Scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: txn.Scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	form := url.Values{
		"grant_type":    {"authorization_code"},
		"code":          {code},
		"code_verifier": {verifier},
	}
	if redirectURI != "" {
		form.Set("redirect_uri", redirectURI)
	}
	return c.oauthTokenRequest(ctx, txn.Scheme, tokenURL, clientID, clientSecret, form, nil)
}
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
// DeviceAuthorization is one started RFC 8628 device-authorization
// transaction. Present UserCode and VerificationURI to the user; Token polls
// the compiled token endpoint for the granted set.
type DeviceAuthorization struct {
	// Scheme names the source security scheme.
	Scheme string
	// UserCode is the code the user enters at VerificationURI.
	UserCode string
	// VerificationURI is where the user approves the device grant.
	VerificationURI string
	// VerificationURIComplete, when declared, carries the user code in the URL.
	VerificationURIComplete string
	// ExpiresAt is the server-declared transaction expiry; zero means the
	// server declared no lifetime and only its own expired_token answer
	// bounds polling.
	ExpiresAt time.Time
	// Interval is the polling floor; the server's slow_down answers extend it.
	Interval time.Duration

	secret *oauthDeviceSecret
}

type oauthDeviceSecret struct {
	client     *Client
	deviceCode string
	tokenURL   string
}

type oauthDeviceResponse struct {
	DeviceCode              string `json:"device_code"`
	UserCode                string `json:"user_code"`
	VerificationURI         string `json:"verification_uri"`
	VerificationURIComplete string `json:"verification_uri_complete"`
	ExpiresIn               int64  `json:"expires_in"`
	Interval                int64  `json:"interval"`
	Error            string `json:"error"`
	ErrorDescription string `json:"error_description"`
}

// BeginDeviceAuthorization starts a device-authorization transaction against
// the compiled device endpoint (RFC 8628 sections 3.1-3.2). It performs one
// network call and returns the user-facing code and verification URL.
func (c *Client) BeginDeviceAuthorization(ctx context.Context, scheme string) (*DeviceAuthorization, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.DeviceURL == "" || descriptor.DeviceTokenURL == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	if ctx == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	if err := ctx.Err(); err != nil {
		return nil, &AuthError{Kind: "cancelled", Scheme: scheme, Cause: err}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	form := url.Values{}
	if clientID != "" {
		form.Set("client_id", clientID)
	}
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, descriptor.DeviceURL, clientID, clientSecret, form)
	if failure != nil {
		return nil, failure
	}
	var decoded oauthDeviceResponse
	if err := json.Unmarshal(body, &decoded); err != nil {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status, Cause: err}
	}
	if decoded.Error != "" {
		return nil, &AuthError{Kind: "authorization-error", Scheme: scheme, Status: status, Code: decoded.Error, Description: decoded.ErrorDescription}
	}
	if status < 200 || status >= 300 {
		return nil, &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	if decoded.DeviceCode == "" || decoded.UserCode == "" || decoded.VerificationURI == "" {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status}
	}
	interval := time.Duration(decoded.Interval) * time.Second
	if interval <= 0 {
		interval = 5 * time.Second
	}
	transaction := &DeviceAuthorization{
		Scheme:                  scheme,
		UserCode:                decoded.UserCode,
		VerificationURI:         decoded.VerificationURI,
		VerificationURIComplete: decoded.VerificationURIComplete,
		Interval:                interval,
		secret:                  &oauthDeviceSecret{client: c, deviceCode: decoded.DeviceCode, tokenURL: descriptor.DeviceTokenURL},
	}
	if decoded.ExpiresIn > 0 {
		transaction.ExpiresAt = time.Now().Add(time.Duration(decoded.ExpiresIn) * time.Second)
	}
	return transaction, nil
}

// Token polls the compiled token endpoint until the user approves the device
// grant, the transaction expires or the context completes (RFC 8628 section
// 3.5). The server's interval is honored; authorization_pending keeps polling
// and slow_down extends the interval by five seconds per answer.
func (d *DeviceAuthorization) Token(ctx context.Context) (*TokenSet, error) {
	if d == nil || d.secret == nil {
		return nil, &AuthError{Kind: "request-validation"}
	}
	if ctx == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: d.Scheme}
	}
	descriptor := oauthSchemes[d.Scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: d.Scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	interval := d.Interval
	if interval <= 0 {
		interval = 5 * time.Second
	}
	for {
		if !d.ExpiresAt.IsZero() && !d.ExpiresAt.After(time.Now()) {
			return nil, &AuthError{Kind: "expired", Scheme: d.Scheme}
		}
		timer := time.NewTimer(interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil, &AuthError{Kind: "cancelled", Scheme: d.Scheme, Cause: ctx.Err()}
		case <-timer.C:
		}
		form := url.Values{
			"grant_type":  {"urn:ietf:params:oauth:grant-type:device_code"},
			"device_code": {d.secret.deviceCode},
		}
		set, err := d.secret.client.oauthTokenRequest(ctx, d.Scheme, d.secret.tokenURL, clientID, clientSecret, form, nil)
		if err == nil {
			return set, nil
		}
		var authError *AuthError
		if !errors.As(err, &authError) {
			return nil, err
		}
		switch authError.Code {
		case "authorization_pending":
			// The user has not approved yet; keep polling.
		case "slow_down":
			interval += 5 * time.Second
		default:
			return nil, err
		}
	}
}
"#;

/// The discovery-aware client-credentials acquisition: the compiled endpoint
/// always wins; otherwise the client's cached discovery document.
const CLIENT_CREDENTIALS_TOKEN_DISCOVERY: &str = r#"// ClientCredentialsToken returns the scheme's cached token set, acquiring one
// from the resolved client-credentials endpoint when the stored set is absent
// or expired beyond the compiled skew. Concurrent callers on one client share
// a single acquisition: callers waiting on the gate re-read the store instead
// of acquiring again. The returned set is a copy; callers may keep it.
//
// Endpoint resolution follows the compiled precedence: the compiled
// client-credentials endpoint always wins; otherwise, when the scheme
// compiles a discovery URL, the discovery document's token_endpoint resolves
// the acquisition (fetched once per client, single-flighted, retried after a
// failure); otherwise the compiled-only refusal stands.
func (c *Client) ClientCredentialsToken(ctx context.Context, scheme string, opts ...TokenOption) (*TokenSet, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if ctx == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	var options oauthTokenOptions
	for _, apply := range opts {
		apply(&options)
	}
	clientID, clientSecret := descriptor.oauthCredentials(&options)
	store := options.store
	if store == nil {
		store = c.oauthState().defaultStore()
	}
	endpoint, key := descriptor.ClientCredentials, ""
	if endpoint != "" {
		key = oauthStoreKey(scheme, endpoint, clientID)
	} else {
		discovered, err := c.oauthDiscovery(ctx, descriptor)
		if err != nil {
			return nil, err
		}
		if discovered.TokenEndpoint == "" {
			return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
		}
		endpoint = discovered.TokenEndpoint
		// Tokens acquired through a discovered endpoint partition under the
		// compiled discovery URL, which stays stable while metadata changes.
		key = oauthStoreKey(scheme, descriptor.DiscoveryURL, clientID)
	}
	stored, err := store.Load(ctx, key)
	if err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	if stored != nil && !stored.oauthExpired(descriptor.Skew) {
		return stored, nil
	}
	gate := c.oauthState().gate(key)
	if err := gate.lock(ctx); err != nil {
		return nil, &AuthError{Kind: "cancelled", Scheme: scheme, Cause: err}
	}
	defer gate.unlock()
	// A caller that waited on the gate re-checks the store before acquiring:
	// the holder may have populated it already.
	stored, err = store.Load(ctx, key)
	if err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	if stored != nil && !stored.oauthExpired(descriptor.Skew) {
		return stored, nil
	}
	set, err := c.oauthTokenRequest(ctx, scheme, endpoint, clientID, clientSecret, url.Values{"grant_type": {"client_credentials"}}, stored)
	if err != nil {
		return nil, err
	}
	if err := store.Replace(ctx, key, set); err != nil {
		return nil, &AuthError{Kind: "token-store", Scheme: scheme, Cause: err}
	}
	return set, nil
}

"#;

/// The discovery-aware explicit refresh: the compiled refresh URL always
/// wins; otherwise the client's cached discovery document.
const REFRESH_TOKEN_DISCOVERY: &str = r#"// RefreshToken exchanges the set's refresh token (RFC 6749 section 6) at the
// resolved refresh endpoint: the scheme's declared refresh URL, or its token
// endpoint when no refresh URL is declared. The returned set adopts a rotated
// refresh token and retains the given one otherwise. The store is neither
// read nor updated: callers decide which set to keep.
//
// Endpoint resolution follows the compiled precedence: the compiled refresh
// URL always wins; a scheme whose compiled plan declares neither a refresh
// URL nor a token endpoint resolves the discovery document's token_endpoint
// (fetched once per client and cached).
func (c *Client) RefreshToken(ctx context.Context, scheme string, set *TokenSet) (*TokenSet, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if set == nil || !set.hasRefresh || set.RefreshToken == "" {
		return nil, &AuthError{Kind: "request-validation", Scheme: scheme}
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	form := url.Values{"grant_type": {"refresh_token"}, "refresh_token": {set.RefreshToken}}
	if descriptor.RefreshURL != "" {
		return c.oauthTokenRequest(ctx, scheme, descriptor.RefreshURL, clientID, clientSecret, form, set)
	}
	discovered, err := c.oauthDiscovery(ctx, descriptor)
	if err != nil {
		return nil, err
	}
	if discovered.TokenEndpoint == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	return c.oauthTokenRequest(ctx, scheme, discovered.TokenEndpoint, clientID, clientSecret, form, set)
}

"#;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the client's cached discovery document.
const REVOCATION_DISCOVERY: &str = r#"
// RevokeToken posts the token value to the scheme's resolved revocation
// endpoint (RFC 7009). Any 2xx response is success: RFC 7009 declares the
// token revoked even when the server reports an unsupported-token error.
// Endpoint resolution follows the compiled precedence: the configured
// revocation endpoint always wins; a scheme without one resolves the
// discovery document's revocation_endpoint (fetched once per client and
// cached).
func (c *Client) RevokeToken(ctx context.Context, scheme, tokenValue string) error {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	endpoint := descriptor.RevocationURL
	if endpoint == "" {
		discovered, err := c.oauthDiscovery(ctx, descriptor)
		if err != nil {
			return err
		}
		if discovered.RevocationEndpoint == "" {
			return &AuthError{Kind: "unsupported-flow", Scheme: scheme}
		}
		endpoint = discovered.RevocationEndpoint
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, endpoint, clientID, clientSecret, url.Values{"token": {tokenValue}})
	if failure != nil {
		return failure
	}
	var decoded oauthTokenResponse
	if err := json.Unmarshal(body, &decoded); err == nil && decoded.Error != "" {
		return &AuthError{Kind: "authorization-error", Scheme: scheme, Status: status, Code: decoded.Error, Description: decoded.ErrorDescription}
	}
	if status < 200 || status >= 300 {
		return &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	return nil
}
"#;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the client's cached discovery document.
const INTROSPECTION_DISCOVERY: &str = r#"
// Introspection is one RFC 7662 introspection response. Timestamps derive
// from the numeric epoch fields; zero means the server omitted them. The
// response describes the token without returning it.
type Introspection struct {
	Active    bool
	Scope     string
	ClientID  string
	TokenType string
	Username  string
	ExpiresAt time.Time
	IssuedAt  time.Time
	NotBefore time.Time
	Subject   string
	Audience  []string
	Issuer    string
	JWTID     string
}

type oauthIntrospectionResponse struct {
	Active    bool     `json:"active"`
	Scope     string   `json:"scope"`
	ClientID  string   `json:"client_id"`
	TokenType string   `json:"token_type"`
	Username  string   `json:"username"`
	ExpiresAt int64    `json:"exp"`
	IssuedAt  int64    `json:"iat"`
	NotBefore int64    `json:"nbf"`
	Subject   string   `json:"sub"`
	Audience  []string `json:"aud"`
	Issuer    string   `json:"iss"`
	JWTID     string   `json:"jti"`
}

// IntrospectToken queries the scheme's resolved introspection endpoint (RFC
// 7662) with the token value. Endpoint resolution follows the compiled
// precedence: the configured introspection endpoint always wins; a scheme
// without one resolves the discovery document's introspection_endpoint
// (fetched once per client and cached).
func (c *Client) IntrospectToken(ctx context.Context, scheme, tokenValue string) (*Introspection, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	endpoint := descriptor.IntrospectionURL
	if endpoint == "" {
		discovered, err := c.oauthDiscovery(ctx, descriptor)
		if err != nil {
			return nil, err
		}
		if discovered.IntrospectionEndpoint == "" {
			return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
		}
		endpoint = discovered.IntrospectionEndpoint
	}
	clientID, clientSecret := descriptor.oauthCredentials(&oauthTokenOptions{})
	status, body, failure := c.oauthEndpointRequest(ctx, scheme, endpoint, clientID, clientSecret, url.Values{"token": {tokenValue}})
	if failure != nil {
		return nil, failure
	}
	if status < 200 || status >= 300 {
		return nil, &AuthError{Kind: "server-error", Scheme: scheme, Status: status}
	}
	var decoded oauthIntrospectionResponse
	if err := json.Unmarshal(body, &decoded); err != nil {
		return nil, &AuthError{Kind: "invalid-response", Scheme: scheme, Status: status, Cause: err}
	}
	converted := &Introspection{
		Active:    decoded.Active,
		Scope:     decoded.Scope,
		ClientID:  decoded.ClientID,
		TokenType: decoded.TokenType,
		Username:  decoded.Username,
		Subject:   decoded.Subject,
		Audience:  decoded.Audience,
		Issuer:    decoded.Issuer,
		JWTID:     decoded.JWTID,
	}
	if decoded.ExpiresAt > 0 {
		converted.ExpiresAt = time.Unix(decoded.ExpiresAt, 0)
	}
	if decoded.IssuedAt > 0 {
		converted.IssuedAt = time.Unix(decoded.IssuedAt, 0)
	}
	if decoded.NotBefore > 0 {
		converted.NotBefore = time.Unix(decoded.NotBefore, 0)
	}
	return converted, nil
}
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, and the per-client cache with single-flight.
const DISCOVERY: &str = r#"
// oauthDiscoveredEndpoints is one RFC 8414 / OpenID Connect discovery
// document reduced to the endpoints this runtime resolves. Unknown members
// are ignored; a present member must be an absolute http(s) URL.
type oauthDiscoveredEndpoints struct {
	TokenEndpoint         string
	RevocationEndpoint    string
	IntrospectionEndpoint string
}

// oauthDiscoveryMaxBytes bounds one discovery document response (about a
// mebibyte).
const oauthDiscoveryMaxBytes = 1 << 20

// oauthDiscoveryDocument is the typed discovery parse; json.Unmarshal ignores
// unknown members.
type oauthDiscoveryDocument struct {
	Issuer                string `json:"issuer"`
	TokenEndpoint         string `json:"token_endpoint"`
	RevocationEndpoint    string `json:"revocation_endpoint"`
	IntrospectionEndpoint string `json:"introspection_endpoint"`
}

// oauthDiscoveryOrigin returns the URL's origin with the scheme's default
// port made explicit, or false when the value is not an absolute http(s) URL.
func oauthDiscoveryOrigin(raw string) (string, bool) {
	parsed, err := url.Parse(raw)
	if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" {
		return "", false
	}
	port := parsed.Port()
	if port == "" {
		if parsed.Scheme == "http" {
			port = "80"
		} else {
			port = "443"
		}
	}
	return parsed.Scheme + "://" + parsed.Hostname() + ":" + port, true
}

// oauthDiscoveryGateKey namespaces the per-client single-flight gates, so a
// discovery round never shares a gate with a token acquisition.
func oauthDiscoveryGateKey(scheme string) string {
	return "\x00discovery\x1f" + scheme
}

// oauthDiscovery fetches the scheme's discovery document, returning the
// client's cached copy when one exists. Successful fetches are cached for the
// client's lifetime, so repeated calls never re-fetch; failed fetches are
// never cached, so the next call retries. Concurrent callers share the one
// in-flight fetch through the per-client single-flight gate.
func (c *Client) oauthDiscovery(ctx context.Context, descriptor *oauthSchemeDescriptor) (*oauthDiscoveredEndpoints, error) {
	if ctx == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: descriptor.Name}
	}
	if descriptor.DiscoveryURL == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: descriptor.Name}
	}
	state := c.oauthState()
	state.mu.Lock()
	if state.discovery == nil {
		state.discovery = make(map[string]*oauthDiscoveredEndpoints)
	}
	cached := state.discovery[descriptor.Name]
	state.mu.Unlock()
	if cached != nil {
		return cached, nil
	}
	gate := state.gate(oauthDiscoveryGateKey(descriptor.Name))
	if err := gate.lock(ctx); err != nil {
		return nil, &AuthError{Kind: "cancelled", Scheme: descriptor.Name, Cause: err}
	}
	defer gate.unlock()
	state.mu.Lock()
	cached = state.discovery[descriptor.Name]
	state.mu.Unlock()
	if cached != nil {
		return cached, nil
	}
	endpoints, failure := c.oauthDiscoveryFetch(ctx, descriptor)
	if failure != nil {
		return nil, failure
	}
	state.mu.Lock()
	state.discovery[descriptor.Name] = endpoints
	state.mu.Unlock()
	return endpoints, nil
}

// oauthDiscoveryFetch performs one GET for the discovery document with
// `accept: application/json` and the bounded response ceiling. The exact
// issuer rule: when the document carries an `issuer` claim, it must be an
// absolute http(s) URL whose origin (scheme, host and the port with the
// scheme default made explicit) equals the discovery URL's origin; OpenID
// Connect openIdConnectUrl documents are validated against their `issuer`
// claim exactly this way, as are RFC 8414 authorization-server metadata
// documents. A missing claim is tolerated. Failures never carry response
// body text.
func (c *Client) oauthDiscoveryFetch(ctx context.Context, descriptor *oauthSchemeDescriptor) (*oauthDiscoveredEndpoints, *AuthError) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, descriptor.DiscoveryURL, nil)
	if err != nil {
		return nil, &AuthError{Kind: "request-representation", Scheme: descriptor.Name, Cause: err}
	}
	request.Header.Set("Accept", "application/json")
	response, err := c.transport.Do(request)
	if err != nil {
		if response != nil && response.Body != nil {
			_ = response.Body.Close()
		}
		return nil, &AuthError{Kind: "transport", Scheme: descriptor.Name, Cause: httpCause(ctx, err)}
	}
	body, readErr := io.ReadAll(io.LimitReader(response.Body, oauthDiscoveryMaxBytes+1))
	_ = response.Body.Close()
	if readErr != nil {
		return nil, &AuthError{Kind: "transport", Scheme: descriptor.Name, Status: response.StatusCode, Cause: httpCause(ctx, readErr)}
	}
	if len(body) > oauthDiscoveryMaxBytes {
		return nil, &AuthError{Kind: "resource-limit", Scheme: descriptor.Name, Status: response.StatusCode}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, &AuthError{Kind: "discovery-failed", Scheme: descriptor.Name, Status: response.StatusCode}
	}
	var document oauthDiscoveryDocument
	if err := json.Unmarshal(body, &document); err != nil {
		return nil, &AuthError{Kind: "discovery-failed", Scheme: descriptor.Name, Status: response.StatusCode, Cause: err}
	}
	if issuerOrigin, ok := oauthDiscoveryOrigin(document.Issuer); ok {
		discoveryOrigin, _ := oauthDiscoveryOrigin(descriptor.DiscoveryURL)
		if issuerOrigin != discoveryOrigin {
			return nil, &AuthError{Kind: "discovery-failed", Scheme: descriptor.Name}
		}
	}
	endpoints := &oauthDiscoveredEndpoints{}
	if failure := oauthDiscoveryMember(descriptor.Name, document.TokenEndpoint, &endpoints.TokenEndpoint); failure != nil {
		return nil, failure
	}
	if failure := oauthDiscoveryMember(descriptor.Name, document.RevocationEndpoint, &endpoints.RevocationEndpoint); failure != nil {
		return nil, failure
	}
	if failure := oauthDiscoveryMember(descriptor.Name, document.IntrospectionEndpoint, &endpoints.IntrospectionEndpoint); failure != nil {
		return nil, failure
	}
	return endpoints, nil
}

// oauthDiscoveryMember copies one present discovery member, refusing
// non-absolute http(s) values as a typed discovery failure. Absent members
// stay empty and unknown members are ignored by the typed parse.
func oauthDiscoveryMember(scheme, value string, target *string) *AuthError {
	if value == "" {
		return nil
	}
	if _, ok := oauthDiscoveryOrigin(value); !ok {
		return &AuthError{Kind: "discovery-failed", Scheme: scheme}
	}
	*target = value
	return nil
}
"#;

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. Serving reuses the compiled
/// client-credentials lifecycle through an internal client whose transport is
/// the caller's own, so no token plumbing is duplicated.
const REPLAY_CORE_A: &str = r#"
// ReplayCredentials couples one scheme's compiled client-credentials
// lifecycle with the unified 401 replay policy. Pass Hook() as the scheme's
// credential hook and Transport() as the client's transport:
//
//	replay, err := NewReplayCredentials("service")
//	client, err := NewClient(
//		Credentials{}.WithHook("service", replay.Hook()),
//		ClientOptions{Transport: mustTransport(replay.Transport(inner))},
//	)
//
// A 401 (and only a 401) on a request whose Authorization value this
// provider attached triggers exactly one coordinated refresh — concurrent
// 401s share one token request round — and exactly one replay of the request
// with the fresh token. The second response is surfaced whatever it is: a
// second 401 reaches the caller as the declared error. The overall budget is
// one refresh plus one replay, never nested with other retry policies
// (requests are not retried today). Attaches for stream-protected
// requirements are never replayed, because delivered stream data prevents a
// transparent restart. A refresh failure surfaces as the typed *AuthError
// instead of a replay. The plain lifecycle keeps today's semantics: replay
// is this wrapper's opt-in only.
type ReplayCredentials struct {
	scheme     string
	descriptor *oauthSchemeDescriptor
	base       []TokenOption
	store      TokenStore
	mu         sync.Mutex
	served     []oauthServedAttach
	roundsMu   sync.Mutex
	rounds     map[string]*oauthReplayRound
	// inner and client are wired once by Transport before first use.
	inner  Doer
	client *Client
}

type oauthServedAttach struct {
	value    string
	eligible bool
}

// oauthReplayRound carries one coordinated refresh round: exactly one forced
// acquisition, shared by every concurrent 401 that presented the same stale
// token.
type oauthReplayRound struct {
	done chan struct{}
	set  *TokenSet
	err  error
}

func (g *oauthReplayRound) finish(set *TokenSet, err error) {
	g.set, g.err = set, err
	close(g.done)
}

// NewReplayCredentials prepares the replaying variant of one compiled
// scheme's client-credentials provider. Every TokenOption behaves exactly as
// in ClientCredentialsToken; the replay semantics are strictly additive and
// the plain lifecycle keeps today's attach-only semantics.
func NewReplayCredentials(scheme string, opts ...TokenOption) (*ReplayCredentials, error) {
	descriptor := oauthSchemes[scheme]
	if descriptor == nil {
		return nil, &AuthError{Kind: "unknown-scheme", Scheme: scheme}
	}
	if descriptor.ClientCredentials == "" {
		return nil, &AuthError{Kind: "unsupported-flow", Scheme: scheme}
	}
	return &ReplayCredentials{
		scheme:     scheme,
		descriptor: descriptor,
		base:       opts,
		store:      NewMemoryTokenStore(),
		rounds:     make(map[string]*oauthReplayRound),
	}, nil
}

// Hook returns the credential hook serving this scheme's tokens and
// remembering which Authorization values its attaches produced.
func (r *ReplayCredentials) Hook() CredentialHook {
	return func(ctx context.Context, request CredentialRequest) (Authorization, error) {
		if ctx == nil {
			return Authorization{}, &AuthError{Kind: "request-validation", Scheme: r.scheme}
		}
		if r.client == nil {
			return Authorization{}, &AuthError{Kind: "request-validation", Scheme: r.scheme}
		}
		set, err := r.client.ClientCredentialsToken(ctx, r.scheme, r.callOptions()...)
		if err != nil {
			return Authorization{}, err
		}
		r.record(oauthReplayValue(set), r.eligible(request))
		return set.Authorization(), nil
	}
}

// Transport wires the replaying Doer around inner and returns it; call it
// once before first use. The hook's token requests keep traveling through
// inner directly.
func (r *ReplayCredentials) Transport(inner Doer) (Doer, error) {
	if inner == nil {
		return nil, &AuthError{Kind: "request-validation", Scheme: r.scheme}
	}
	client, err := NewClient(Credentials{}, ClientOptions{Transport: inner})
	if err != nil {
		return nil, err
	}
	r.inner = inner
	r.client = client
	return &oauthReplayTransport{replay: r, inner: inner}, nil
}

func (r *ReplayCredentials) callOptions() []TokenOption {
	options := make([]TokenOption, 0, len(r.base)+1)
	options = append(options, r.base...)
	return append(options, WithTokenStore(r.store))
}

// oauthReplayValue renders the complete Authorization header value of a set.
func oauthReplayValue(set *TokenSet) string {
	authorization := set.Authorization()
	return authorization.Scheme + " " + authorization.Value
}
"#;

/// The plain variant's lifecycle-endpoint exclusion: the compiled
/// client-credentials and refresh endpoints.
const REPLAY_LIFECYCLE_PLAIN: &str = r#"
func (r *ReplayCredentials) oauthReplayLifecycleTarget(target string) bool {
	return target == r.descriptor.ClientCredentials ||
		(r.descriptor.RefreshURL != "" && target == r.descriptor.RefreshURL)
}
"#;

/// The discovery variant's lifecycle-endpoint exclusion: the compiled
/// discovery URL joins the compiled endpoints; the resolved token endpoint
/// rides the exact-token match.
const REPLAY_LIFECYCLE_DISCOVERY: &str = r#"
func (r *ReplayCredentials) oauthReplayLifecycleTarget(target string) bool {
	return target == r.descriptor.ClientCredentials ||
		(r.descriptor.RefreshURL != "" && target == r.descriptor.RefreshURL) ||
		(r.descriptor.DiscoveryURL != "" && target == r.descriptor.DiscoveryURL)
}
"#;

/// The replaying credential wrapper's second half: the served-attach record,
/// the coordinated refresh and the replaying Doer.
const REPLAY_CORE_B: &str = r#"
func (r *ReplayCredentials) record(value string, eligible bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.served = append([]oauthServedAttach{{value: value, eligible: eligible}}, r.served...)
	if len(r.served) > 8 {
		r.served = r.served[:8]
	}
}

func (r *ReplayCredentials) eligible(request CredentialRequest) bool {
	for _, pointer := range oauthNoReplayRequirements[r.scheme] {
		if pointer == request.Requirement.Source.Pointer {
			return false
		}
	}
	return true
}

func (r *ReplayCredentials) replayable(request *http.Request, presented string) bool {
	if presented == "" {
		return false
	}
	if r.oauthReplayLifecycleTarget(request.URL.String()) {
		return false
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, entry := range r.served {
		if entry.value == presented && entry.eligible {
			return true
		}
	}
	return false
}

// oauthReplayRefresh runs the coordinated refresh: a newer stored set wins
// over a stale re-refresh, concurrent 401s share one round, and a failed
// round fails every waiter exactly once.
func (r *ReplayCredentials) oauthReplayRefresh(ctx context.Context, presented string) (string, error) {
	if ctx == nil {
		return "", &AuthError{Kind: "request-validation", Scheme: r.scheme}
	}
	var options oauthTokenOptions
	for _, apply := range r.callOptions() {
		apply(&options)
	}
	clientID, _ := r.descriptor.oauthCredentials(&options)
	key := oauthStoreKey(r.scheme, r.descriptor.ClientCredentials, clientID)
	for {
		r.roundsMu.Lock()
		stored, err := r.store.Load(ctx, key)
		if err != nil {
			r.roundsMu.Unlock()
			return "", &AuthError{Kind: "token-store", Scheme: r.scheme, Cause: err}
		}
		if stored != nil && oauthReplayValue(stored) != presented {
			fresh := oauthReplayValue(stored)
			r.roundsMu.Unlock()
			return fresh, nil
		}
		round := r.rounds[key]
		leader := false
		if round == nil {
			round = &oauthReplayRound{done: make(chan struct{})}
			r.rounds[key] = round
			leader = true
		}
		r.roundsMu.Unlock()
		if !leader {
			select {
			case <-round.done:
				if round.err != nil {
					return "", round.err
				}
				return oauthReplayValue(round.set), nil
			case <-ctx.Done():
				return "", &AuthError{Kind: "cancelled", Scheme: r.scheme, Cause: ctx.Err()}
			}
		}
		if err := r.store.Clear(ctx, key); err != nil {
			failure := &AuthError{Kind: "token-store", Scheme: r.scheme, Cause: err}
			round.finish(nil, failure)
			r.removeRound(key)
			return "", failure
		}
		set, err := r.client.ClientCredentialsToken(ctx, r.scheme, r.callOptions()...)
		if err != nil {
			round.finish(nil, err)
			r.removeRound(key)
			return "", err
		}
		r.removeRound(key)
		round.finish(set, nil)
		return oauthReplayValue(set), nil
	}
}

func (r *ReplayCredentials) removeRound(key string) {
	r.roundsMu.Lock()
	defer r.roundsMu.Unlock()
	delete(r.rounds, key)
}

// oauthReplayMaxBodyBytes bounds the request body buffered for replay.
const oauthReplayMaxBodyBytes = 1 << 26

// oauthReplayTransport applies the one-refresh-one-replay 401 policy around
// the caller's transport. Lifecycle endpoint requests are never replayed:
// they carry no bearer token of this provider, and the exact-target guard in
// replayable is defense in depth.
type oauthReplayTransport struct {
	replay *ReplayCredentials
	inner  Doer
}

func (t *oauthReplayTransport) Do(request *http.Request) (*http.Response, error) {
	var body []byte
	if request.Body != nil {
		buffered, err := io.ReadAll(io.LimitReader(request.Body, oauthReplayMaxBodyBytes+1))
		_ = request.Body.Close()
		if err != nil || len(buffered) > oauthReplayMaxBodyBytes {
			request.Body = io.NopCloser(bytes.NewReader(buffered))
			return t.inner.Do(request)
		}
		body = buffered
		request.Body = io.NopCloser(bytes.NewReader(body))
	}
	response, err := t.inner.Do(request)
	if err != nil || response == nil || response.StatusCode != http.StatusUnauthorized {
		return response, err
	}
	presented := request.Header.Get("Authorization")
	if !t.replay.replayable(request, presented) {
		return response, nil
	}
	fresh, refreshErr := t.replay.oauthReplayRefresh(request.Context(), presented)
	if refreshErr != nil {
		return nil, refreshErr
	}
	replayed := request.Clone(request.Context())
	if body != nil {
		replayed.Body = io.NopCloser(bytes.NewReader(body))
	}
	replayed.Header.Set("Authorization", fresh)
	return t.inner.Do(replayed)
}
"#;
