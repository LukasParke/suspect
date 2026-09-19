//! Emitted-only OAuth 2.0 lifecycle. The static runtime gains nothing: every
//! type, compiled endpoint and routine below is generated into the consumer
//! package only when the plan carries source schemes with executable flows.
//!
//! The generated module acquires, refreshes, revokes and introspects tokens
//! through the client's public `send` surface with synthesized static token
//! descriptors, so the runtime never learns about OAuth and unconfigured
//! packages stay byte-identical.

use super::*;
use crate::http_protocol::{OAuthClientAuth, OAuthFlowDescriptorKind, OAuthPlan, OAuthSchemePlan};
use std::collections::BTreeSet;

/// Everything `package` needs to emit OAuth support.
pub(super) struct Emission {
    /// The generated `rust/src/oauth.rs` module content.
    pub(super) module: String,
    /// Client-impl methods appended to the emitted root by `root`.
    pub(super) methods: String,
}

/// One executable grant's compiled token endpoints.
struct CompiledGrant {
    token_url: String,
    refresh_url: Option<String>,
}

/// One source scheme compiled down to executable endpoints.
struct CompiledScheme {
    name: String,
    source: Option<SourceId>,
    skew_seconds: u64,
    client_id_env: Option<String>,
    client_secret_env: Option<String>,
    secret_basic: bool,
    client_credentials: Option<CompiledGrant>,
    /// The declared authorization-request URL, carried whole: it is rendered
    /// into the returned transaction, never sent through the transport.
    authorization_url: Option<String>,
    authorization_code: Option<CompiledGrant>,
    device: Option<CompiledGrant>,
    device_authorization: Option<String>,
    revocation: Option<String>,
    introspection: Option<String>,
    /// The compiled discovery/metadata URL: supplies the endpoints the fields
    /// above leave empty at call time.
    discovery: Option<String>,
}

/// Compile the OAuth emission for one plan, or `None` when no source scheme
/// carries an executable flow or a discovery URL (the package then gains
/// nothing at all).
pub(super) fn emission(plan: &HttpPlan) -> Option<Emission> {
    let oauth: &OAuthPlan = plan.oauth()?;
    let mut compiled = Vec::new();
    for scheme in &oauth.schemes {
        if let Some(scheme) = compile_scheme(plan, scheme) {
            compiled.push(scheme);
        }
    }
    if compiled.is_empty() {
        return None;
    }
    let discovery = compiled.iter().any(|scheme| scheme.discovery.is_some());
    let mut module = String::new();
    push_header(&mut module, &compiled);
    push_types(&mut module, discovery);
    let mut literals = Vec::new();
    for (index, scheme) in compiled.iter().enumerate() {
        literals.push(push_scheme(&mut module, index, scheme, plan, discovery));
    }
    module.push_str("static SCHEMES: &[Scheme] = &[");
    for literal in &literals {
        module.push_str(literal);
        module.push(',');
    }
    module.push_str("];\n\n");
    push_limits(&mut module, plan);
    push_methods(&mut module, discovery);
    push_authorization_code_methods(&mut module);
    push_helpers(&mut module);
    if discovery {
        module.push_str(&discovery_section(plan));
    }
    // The replaying credential wrapper joins only when a compiled scheme
    // carries an executable client-credentials grant: it serves exactly that
    // provider, and plans without one stay byte-identical.
    if compiled
        .iter()
        .any(|scheme| scheme.client_credentials.is_some())
    {
        module.push_str(&replay_section(plan, &compiled, discovery));
    }
    Some(Emission {
        module,
        methods: client_methods(),
    })
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
fn no_replay_requirements(
    plan: &HttpPlan,
    compiled: &[CompiledScheme],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = compiled.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in &plan.operations {
        let streams =
            operation.responses().iter().any(|response| {
                response.wire().media().iter().any(|media| {
                    matches!(media.representation(), wire::Representation::Stream { .. })
                })
            });
        if !streams {
            continue;
        }
        for credential in operation.credentials() {
            let requirement = &credential.requirement;
            if !matches!(
                requirement.credential(),
                wire::CredentialHook::OAuth2 { .. } | wire::CredentialHook::OpenIdConnect { .. }
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
    pointers
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations.
fn replay_section(plan: &HttpPlan, compiled: &[CompiledScheme], discovery: bool) -> String {
    let no_replay = no_replay_requirements(plan, compiled);
    let mut code = String::from(
        "\n/// Stream-protected requirement pointers per compiled scheme: attaches\n/// for these requirements are never replayed, because delivered stream\n/// data prevents a transparent restart.\nstatic REPLAY_NO_REPLAY: &[(&str, &[&str])] = &[",
    );
    let mut entries = Vec::new();
    for scheme in compiled {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| format!("{pointer:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        entries.push(format!("({:?}, &[{}])", scheme.name, rendered));
    }
    code.push_str(&entries.join(", "));
    code.push_str("];\n\n");
    code.push_str(REPLAY_CORE);
    code.push_str(if discovery {
        REPLAY_LIFECYCLE_DISCOVERY
    } else {
        REPLAY_LIFECYCLE_PLAIN
    });
    code
}

fn compile_scheme(plan: &HttpPlan, scheme: &OAuthSchemePlan) -> Option<CompiledScheme> {
    let mut client_credentials = None;
    let mut authorization_url = None;
    let mut authorization_code = None;
    let mut device = None;
    let mut device_authorization = None;
    let mut secret_basic = None;
    for flow in &scheme.flows {
        match flow.kind {
            OAuthFlowDescriptorKind::ClientCredentials => {
                let token_url = emittable(flow.token_url.as_deref()?)?;
                let refresh_url = flow
                    .refresh_url
                    .as_deref()
                    .and_then(emittable)
                    .filter(|refresh| *refresh != token_url);
                client_credentials = Some(CompiledGrant {
                    token_url,
                    refresh_url,
                });
                secret_basic =
                    secret_basic.or(Some(flow.client_auth == OAuthClientAuth::ClientSecretBasic));
            }
            OAuthFlowDescriptorKind::AuthorizationCode => {
                let token_url = emittable(flow.token_url.as_deref()?)?;
                let refresh_url = flow
                    .refresh_url
                    .as_deref()
                    .and_then(emittable)
                    .filter(|refresh| *refresh != token_url);
                // The authorization URL is a browser redirect, not a compiled
                // transport endpoint: it is carried whole and extended with
                // the request parameters at begin time. The planner already
                // validated it as an absolute http(s) URL.
                authorization_url = flow.authorization_url.clone();
                authorization_code = Some(CompiledGrant {
                    token_url,
                    refresh_url,
                });
                secret_basic =
                    secret_basic.or(Some(flow.client_auth == OAuthClientAuth::ClientSecretBasic));
            }
            OAuthFlowDescriptorKind::DeviceAuthorization => {
                let token_url = emittable(flow.token_url.as_deref()?)?;
                let device_url = emittable(flow.device_authorization_url.as_deref()?)?;
                let refresh_url = flow
                    .refresh_url
                    .as_deref()
                    .and_then(emittable)
                    .filter(|refresh| *refresh != token_url);
                device_authorization = Some(device_url);
                device = Some(CompiledGrant {
                    token_url,
                    refresh_url,
                });
                secret_basic =
                    secret_basic.or(Some(flow.client_auth == OAuthClientAuth::ClientSecretBasic));
            }
            // Implicit and password flows are deprecated and never executed;
            // they compile to nothing here.
            OAuthFlowDescriptorKind::Implicit | OAuthFlowDescriptorKind::Password => {}
        }
    }
    if client_credentials.is_none()
        && authorization_code.is_none()
        && device.is_none()
        && scheme.discovery.is_none()
    {
        return None;
    }
    let source = plan
        .credentials()
        .values()
        .find(|credential| {
            credential.requirement.name() == scheme.name
                && matches!(
                    credential.requirement.credential(),
                    wire::CredentialHook::OAuth2 { .. }
                        | wire::CredentialHook::OpenIdConnect { .. }
                )
        })
        .map(|credential| credential.requirement.scheme().terminal().source().clone());
    Some(CompiledScheme {
        name: scheme.name.clone(),
        source,
        skew_seconds: u64::from(scheme.refresh_skew_seconds),
        client_id_env: scheme.client_id_env.clone(),
        client_secret_env: scheme.client_secret_env.clone(),
        secret_basic: secret_basic.unwrap_or_else(|| {
            // A scheme whose flows declare no client authentication (OpenID
            // Connect discovery-defined schemes) derives the same rule the
            // planner applies per flow: client-secret-basic when the compiled
            // configuration supplies a client secret variable, else the
            // public profile.
            scheme.discovery.is_some() && scheme.client_secret_env.is_some()
        }),
        client_credentials,
        authorization_url,
        authorization_code,
        device,
        device_authorization,
        revocation: scheme.revocation_endpoint.as_deref().and_then(emittable),
        introspection: scheme.introspection_endpoint.as_deref().and_then(emittable),
        discovery: scheme.discovery.clone(),
    })
}

/// Validate that a declared absolute http(s) URL can compile into an origin
/// server template plus a path the runtime's server resolution can assemble.
/// Endpoints carrying query, fragment or userinfo components are outside the
/// compiled form surface and narrow the compiled plan instead of being
/// invented.
fn emittable(url: &str) -> Option<String> {
    split_url(url).is_some().then(|| url.to_owned())
}

fn split_url(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
        return None;
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    let path = parsed.path().to_owned();
    if path.contains(['{', '}', '\\'])
        || path.split('/').any(dot_segment)
        || !percent_triples(&path)
    {
        return None;
    }
    let host = parsed.host().map(|host| host.to_string())?;
    let origin = match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    };
    Some((origin, path))
}

fn dot_segment(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "." | ".." | "%2e" | ".%2e" | "%2e." | "%2e%2e"
    )
}

fn percent_triples(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            if !bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                return false;
            }
            at += 3;
        } else {
            at += 1;
        }
    }
    true
}

/// Compile one URL into a `(origin, path, name)` entry; the same URL reuses
/// one synthetic operation static.
struct Endpoints {
    index: usize,
    declared: Vec<(String, String, String)>,
}

impl Endpoints {
    fn new(index: usize) -> Self {
        Self {
            index,
            declared: Vec::new(),
        }
    }

    fn name(&mut self, url: &str) -> Option<String> {
        let (origin, path) = split_url(url)?;
        if let Some((_, _, name)) = self
            .declared
            .iter()
            .find(|(known_origin, known_path, _)| known_origin == &origin && known_path == &path)
        {
            return Some(name.clone());
        }
        let name = format!("ENDPOINT_{}_{}", self.index, self.declared.len());
        self.declared.push((origin, path, name.clone()));
        Some(name)
    }
}

fn push_header(module: &mut String, compiled: &[CompiledScheme]) {
    module.push_str("//! Code generated by suspect. DO NOT EDIT.\n//!\n//! Generated OAuth 2.0 token lifecycle for the source-selected executable\n//! security schemes of this package: client-credentials acquisition with an\n//! in-process token store, explicit refresh, revocation, introspection,\n//! authorization-code with PKCE S256, and the RFC 8628 device grant with both\n//! caller-paced single polls and a paced completion loop. Implicit and\n//! password flows are deprecated and are never executed.\n//!\n//! Entropy: PKCE verifiers and states draw 32 bytes from the operating\n//! system's kernel CSPRNG, read through `std::fs` from `/dev/urandom` on\n//! Unix targets. There is deliberately no fallback: a platform with no\n//! dependency-free cryptographic randomness source fails with a typed\n//! `unsupported platform` error instead of receiving weak entropy. The S256\n//! code challenge is a local FIPS 180-4 SHA-256; no hashing crate is\n//! introduced.\n//!\n//! The static runtime knows nothing about OAuth: every type and routine below\n//! is emitted only for packages whose source uses an executable OAuth scheme.\n//! Token and client-secret values are never baked into this file; client\n//! credentials are read from the compiled environment variables at call time\n//! or supplied explicitly on the token sessions value.\n//!\n//! Concurrency simplification: client-credentials acquisition is\n//! single-flighted per scheme with a `std::sync::Mutex`/`Condvar` pair. A\n//! losing caller blocks its thread until the winning round completes, which\n//! presumes concurrent callers run on distinct threads (multi-threaded\n//! executors or separate `block_on` threads); a single-threaded executor must\n//! not poll two acquisitions of the same scheme concurrently. Store\n//! replacement itself stays atomic.\n//!\n");
    for scheme in compiled {
        let steps = [
            (scheme.client_credentials.is_some(), "client-credentials"),
            (
                scheme.authorization_code.is_some(),
                "authorization-code (PKCE S256)",
            ),
            (scheme.device.is_some(), "device grant"),
            (scheme.revocation.is_some(), "revocation"),
            (scheme.introspection.is_some(), "introspection"),
            (scheme.discovery.is_some(), "discovery-resolved endpoints"),
        ]
        .iter()
        .filter(|(present, _)| *present)
        .map(|(_, step)| *step)
        .collect::<Vec<_>>()
        .join(", ");
        module.push_str(&format!(
            "//! - Source scheme {:?}: {steps}.\n",
            scheme.name
        ));
    }
    module.push('\n');
}

/// The typed failure categories: byte-exact without discovery, with the
/// `DiscoveryFailed` kind added when any compiled scheme carries a
/// discovery URL.
const AUTH_ERROR_KIND_PLAIN: &str = "/// Stable categories for [`AuthError`].\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum AuthErrorKind {\n    /// The scheme name is not compiled into this package.\n    UnknownScheme,\n    /// The compiled plan carries no executable endpoint for the step.\n    Unavailable,\n    /// The compiled environment variables held no usable client credentials.\n    MissingClientCredentials,\n    /// The token set carries no refresh token to exchange.\n    MissingRefreshToken,\n    /// The platform has no dependency-free cryptographic randomness\n    /// source; weak entropy is refused, never substituted.\n    UnsupportedPlatform,\n    /// The kernel entropy source could not be read.\n    Entropy,\n    /// The callback state does not match the transaction.\n    StateMismatch,\n    /// The transaction was already consumed by an earlier completion.\n    TransactionUsed,\n    /// The resource owner (or server) declared an authorization error.\n    AuthorizationDenied,\n    /// The callback carries no usable authorization code.\n    InvalidCallback,\n    /// The device grant transaction expired.\n    Expired,\n    /// The response was not a usable token, device or error document.\n    InvalidResponse,\n    /// The server answered with an OAuth error response.\n    ServerRejected,\n    /// The transport failed before an OAuth response existed.\n    Transport,\n}\nimpl std::fmt::Display for AuthErrorKind {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.write_str(match self {\n            Self::UnknownScheme => \"unknown compiled scheme\",\n            Self::Unavailable => \"no executable compiled endpoint\",\n            Self::MissingClientCredentials => \"missing client credentials\",\n            Self::MissingRefreshToken => \"missing refresh token\",\n            Self::UnsupportedPlatform => \"unsupported platform\",\n            Self::Entropy => \"entropy source failure\",\n            Self::StateMismatch => \"callback state mismatch\",\n            Self::TransactionUsed => \"transaction already consumed\",\n            Self::AuthorizationDenied => \"authorization denied\",\n            Self::InvalidCallback => \"invalid callback\",\n            Self::Expired => \"device grant expired\",\n            Self::InvalidResponse => \"invalid response\",\n            Self::ServerRejected => \"server rejected the request\",\n            Self::Transport => \"transport failure\",\n        })\n    }\n}\n\n";
const AUTH_ERROR_KIND_DISCOVERY: &str = "/// Stable categories for [`AuthError`].\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum AuthErrorKind {\n    /// The scheme name is not compiled into this package.\n    UnknownScheme,\n    /// The compiled plan carries no executable endpoint for the step.\n    Unavailable,\n    /// The compiled environment variables held no usable client credentials.\n    MissingClientCredentials,\n    /// The token set carries no refresh token to exchange.\n    MissingRefreshToken,\n    /// The platform has no dependency-free cryptographic randomness\n    /// source; weak entropy is refused, never substituted.\n    UnsupportedPlatform,\n    /// The kernel entropy source could not be read.\n    Entropy,\n    /// The callback state does not match the transaction.\n    StateMismatch,\n    /// The transaction was already consumed by an earlier completion.\n    TransactionUsed,\n    /// The resource owner (or server) declared an authorization error.\n    AuthorizationDenied,\n    /// The callback carries no usable authorization code.\n    InvalidCallback,\n    /// The device grant transaction expired.\n    Expired,\n    /// The response was not a usable token, device or error document.\n    InvalidResponse,\n    /// The server answered with an OAuth error response.\n    ServerRejected,\n    /// The transport failed before an OAuth response existed.\n    Transport,\n    /// The discovery document could not be fetched, decoded or validated.\n    DiscoveryFailed,\n}\nimpl std::fmt::Display for AuthErrorKind {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.write_str(match self {\n            Self::UnknownScheme => \"unknown compiled scheme\",\n            Self::Unavailable => \"no executable compiled endpoint\",\n            Self::MissingClientCredentials => \"missing client credentials\",\n            Self::MissingRefreshToken => \"missing refresh token\",\n            Self::UnsupportedPlatform => \"unsupported platform\",\n            Self::Entropy => \"entropy source failure\",\n            Self::StateMismatch => \"callback state mismatch\",\n            Self::TransactionUsed => \"transaction already consumed\",\n            Self::AuthorizationDenied => \"authorization denied\",\n            Self::InvalidCallback => \"invalid callback\",\n            Self::Expired => \"device grant expired\",\n            Self::InvalidResponse => \"invalid response\",\n            Self::ServerRejected => \"server rejected the request\",\n            Self::Transport => \"transport failure\",\n            Self::DiscoveryFailed => \"discovery failed\",\n        })\n    }\n}\n\n";

/// The compiled grant and scheme structs: byte-exact without discovery,
/// with the discovery fields added when any scheme carries a discovery URL.
const TYPES_SCHEME_PLAIN: &str = "/// One executable grant's compiled token endpoints.\n#[derive(Debug)]\nstruct Grant {\n    token: &'static Operation,\n    refresh: std::option::Option<&'static Operation>,\n}\n/// One compiled executable scheme. Endpoints are declared absolute http(s)\n/// URLs split into an origin server template and a path; the runtime never\n/// parses OpenAPI and performs no discovery.\n#[derive(Debug)]\nstruct Scheme {\n    name: &'static str,\n    source: Source,\n    /// Refresh-before-expiry clock skew, compiled from configuration.\n    skew_seconds: u64,\n    /// Compiled client credential variables, read from the environment at\n    /// call time.\n    client_id_env: std::option::Option<&'static str>,\n    client_secret_env: std::option::Option<&'static str>,\n    /// Token-endpoint requests authenticate with the configured secret\n    /// variable (`client_secret_basic`); otherwise the client is public and\n    /// only the client id, when configured, joins the form.\n    secret_basic: bool,\n    client_credentials: std::option::Option<Grant>,\n    authorization_url: std::option::Option<&'static str>,\n    authorization_code: std::option::Option<Grant>,\n    device: std::option::Option<Grant>,\n    device_authorization: std::option::Option<&'static Operation>,\n    revocation: std::option::Option<&'static Operation>,\n    introspection: std::option::Option<&'static Operation>,\n}\n";
const TYPES_SCHEME_DISCOVERY: &str = "/// One executable grant's compiled token endpoints.\n#[derive(Debug)]\nstruct Grant {\n    token: &'static Operation,\n    refresh: std::option::Option<&'static Operation>,\n}\n/// One compiled executable scheme. Endpoints are declared absolute http(s)\n/// URLs split into an origin server template and a path; the runtime never\n/// parses OpenAPI and performs no discovery.\n#[derive(Debug)]\nstruct Scheme {\n    name: &'static str,\n    source: Source,\n    /// Refresh-before-expiry clock skew, compiled from configuration.\n    skew_seconds: u64,\n    /// Compiled client credential variables, read from the environment at\n    /// call time.\n    client_id_env: std::option::Option<&'static str>,\n    client_secret_env: std::option::Option<&'static str>,\n    /// Token-endpoint requests authenticate with the configured secret\n    /// variable (`client_secret_basic`); otherwise the client is public and\n    /// only the client id, when configured, joins the form.\n    secret_basic: bool,\n    client_credentials: std::option::Option<Grant>,\n    authorization_url: std::option::Option<&'static str>,\n    authorization_code: std::option::Option<Grant>,\n    device: std::option::Option<Grant>,\n    device_authorization: std::option::Option<&'static Operation>,\n    revocation: std::option::Option<&'static Operation>,\n    introspection: std::option::Option<&'static Operation>,\n    /// The compiled discovery URL, when the plan carries one.\n    discovery_url: std::option::Option<&'static str>,\n    /// The single-flight round key for this scheme's discovery fetches.\n    discovery_round: &'static str,\n}\n";

/// The token sessions value: byte-exact without discovery, with the
/// discovery cache field added when any scheme carries a discovery URL.
const TOKEN_SESSIONS_PLAIN: &str = "/// Per-client OAuth token lifecycle over a caller-owned store. Create one\n/// value per client: its default store and single-flight rounds are\n/// instance-owned, never process-wide.\npub struct TokenSessions<S: TokenStore = MemoryTokenStore> {\n    store: std::sync::Arc<S>,\n    rounds: std::sync::Mutex<std::collections::BTreeMap<&'static str, std::sync::Arc<Round>>>,\n    client_id: std::option::Option<std::string::String>,\n    client_secret: std::option::Option<std::string::String>,\n}\nimpl<S: TokenStore> std::fmt::Debug for TokenSessions<S> {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"TokenSessions\")\n            .field(\"client_id\", &self.client_id.is_some())\n            .field(\"client_secret\", &self.client_secret.is_some())\n            .finish_non_exhaustive()\n    }\n}\nimpl TokenSessions<MemoryTokenStore> {\n    /// A sessions value with its own fresh in-process store.\n    #[must_use]\n    pub fn new() -> Self {\n        Self::with_store(std::sync::Arc::new(MemoryTokenStore::new()))\n    }\n}\nimpl<S: TokenStore> TokenSessions<S> {\n    /// A sessions value over a caller-owned store.\n    #[must_use]\n    pub fn with_store(store: std::sync::Arc<S>) -> Self {\n        Self {\n            store,\n            rounds: std::sync::Mutex::new(std::collections::BTreeMap::new()),\n            client_id: std::option::Option::None,\n            client_secret: std::option::Option::None,\n        }\n    }\n    /// Explicit client credentials for every compiled scheme of this value,\n    /// taking precedence over the compiled environment variables.\n    #[must_use]\n    pub fn with_client_credentials(\n        mut self,\n        client_id: impl Into<std::string::String>,\n        client_secret: impl Into<std::string::String>,\n    ) -> Self {\n        self.client_id = std::option::Option::Some(client_id.into());\n        self.client_secret = std::option::Option::Some(client_secret.into());\n        self\n    }\n}\n";
const TOKEN_SESSIONS_DISCOVERY: &str = "/// Per-client OAuth token lifecycle over a caller-owned store. Create one\n/// value per client: its default store and single-flight rounds are\n/// instance-owned, never process-wide.\npub struct TokenSessions<S: TokenStore = MemoryTokenStore> {\n    store: std::sync::Arc<S>,\n    rounds: std::sync::Mutex<std::collections::BTreeMap<&'static str, std::sync::Arc<Round>>>,\n    client_id: std::option::Option<std::string::String>,\n    client_secret: std::option::Option<std::string::String>,\n    /// The per-scheme discovery documents this sessions value has fetched:\n    /// cached for its lifetime, never process-wide.\n    discovery: DiscoveryCache,\n}\nimpl<S: TokenStore> std::fmt::Debug for TokenSessions<S> {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"TokenSessions\")\n            .field(\"client_id\", &self.client_id.is_some())\n            .field(\"client_secret\", &self.client_secret.is_some())\n            .finish_non_exhaustive()\n    }\n}\nimpl TokenSessions<MemoryTokenStore> {\n    /// A sessions value with its own fresh in-process store.\n    #[must_use]\n    pub fn new() -> Self {\n        Self::with_store(std::sync::Arc::new(MemoryTokenStore::new()))\n    }\n}\nimpl<S: TokenStore> TokenSessions<S> {\n    /// A sessions value over a caller-owned store.\n    #[must_use]\n    pub fn with_store(store: std::sync::Arc<S>) -> Self {\n        Self {\n            store,\n            rounds: std::sync::Mutex::new(std::collections::BTreeMap::new()),\n            client_id: std::option::Option::None,\n            client_secret: std::option::Option::None,\n            discovery: DiscoveryCache::default(),\n        }\n    }\n    /// Explicit client credentials for every compiled scheme of this value,\n    /// taking precedence over the compiled environment variables.\n    #[must_use]\n    pub fn with_client_credentials(\n        mut self,\n        client_id: impl Into<std::string::String>,\n        client_secret: impl Into<std::string::String>,\n    ) -> Self {\n        self.client_id = std::option::Option::Some(client_id.into());\n        self.client_secret = std::option::Option::Some(client_secret.into());\n        self\n    }\n}\n";

fn push_types(module: &mut String, discovery: bool) {
    module.push_str(
        "use crate::http::{\n    BoxError, Client, Operation, ParameterValue, PreparedBody, RawResponse, SdkError,\n    SdkErrorKind, Source, Transport,\n};\nuse crate::{JsonInteger, JsonNonNullValue, JsonValue, Nullable};\n\n/// An issued OAuth token and its metadata, exactly as the compiled token\n/// endpoint returned it. Debug omits the token values.\n#[derive(Clone, PartialEq, Eq)]\npub struct TokenSet {\n    /// The access token.\n    pub access_token: std::string::String,\n    /// The server's token type hint; absent responses compile to `bearer`.\n    pub token_type: std::string::String,\n    /// Issue time plus the returned `expires_in`. An absent or unusable\n    /// `expires_in` compiles to a far-future expiry the skew gate treats as\n    /// valid.\n    pub expires_at: std::time::SystemTime,\n    /// The server's refresh token, when one was returned.\n    pub refresh_token: std::option::Option<std::string::String>,\n    /// The granted scope, as the server returned it.\n    pub scope: std::option::Option<std::string::String>,\n    /// The server's RFC 9207 `iss` value, when returned.\n    pub issuer_account: std::option::Option<std::string::String>,\n}\nimpl std::fmt::Debug for TokenSet {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"TokenSet\")\n            .field(\"access_token\", &\"<redacted>\")\n            .field(\"token_type\", &self.token_type)\n            .field(\"expires_at\", &self.expires_at)\n            .field(\"refresh_token\", &self.refresh_token.is_some())\n            .field(\"scope\", &self.scope)\n            .field(\"issuer_account\", &self.issuer_account)\n            .finish()\n    }\n}\n\n/// Caller-owned token storage behind the generated lifecycle. Implementations\n/// keep entries independent per `key` (one key per compiled source scheme) and\n/// make `replace` atomic. The trait mirrors the crate's dependency-free async\n/// style: plain futures, no runtime crate.\npub trait TokenStore: Send + Sync {\n    /// Load the token set stored for `key`, if any.\n    fn load(\n        &self,\n        key: &str,\n    ) -> impl std::future::Future<Output = std::option::Option<TokenSet>> + std::marker::Send;\n    /// Atomically replace the token set stored for `key`.\n    fn replace(\n        &self,\n        key: &str,\n        set: TokenSet,\n    ) -> impl std::future::Future<Output = ()> + std::marker::Send;\n    /// Remove any token set stored for `key`.\n    fn clear(&self, key: &str) -> impl std::future::Future<Output = ()> + std::marker::Send;\n}\n\n/// Instance-owned in-process token store. One instance per client keeps its\n/// tokens local to that client; entries are partitioned by key and every\n/// mutation is a single mutex-guarded map update. Never a process-wide global.\n#[derive(Default)]\npub struct MemoryTokenStore {\n    entries: std::sync::Mutex<std::collections::BTreeMap<std::string::String, TokenSet>>,\n}\nimpl MemoryTokenStore {\n    #[must_use]\n    pub fn new() -> Self {\n        Self::default()\n    }\n}\nimpl Default for TokenSessions<MemoryTokenStore> {\n    fn default() -> Self {\n        Self::new()\n    }\n}\nimpl std::fmt::Debug for MemoryTokenStore {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"MemoryTokenStore\").finish_non_exhaustive()\n    }\n}\nimpl TokenStore for MemoryTokenStore {\n    async fn load(&self, key: &str) -> std::option::Option<TokenSet> {\n        self.entries\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner)\n            .get(key)\n            .cloned()\n    }\n    async fn replace(&self, key: &str, set: TokenSet) {\n        self.entries\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner)\n            .insert(key.to_owned(), set);\n    }\n    async fn clear(&self, key: &str) {\n        self.entries\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner)\n            .remove(key);\n    }\n}\n\n/// Typed OAuth failure metadata carried as an [`SdkError`] cause. It names the\n/// scheme and lifecycle step and, for server rejections, the OAuth error code\n/// and description; it never carries token or client-secret values.\npub struct AuthError {\n    /// Source scheme name this failure belongs to.\n    pub scheme: std::string::String,\n    /// The lifecycle step that failed.\n    pub flow: AuthFlow,\n    /// Stable failure category.\n    pub kind: AuthErrorKind,\n    /// RFC 6749/8628 `error` code returned by the server, when any.\n    pub server_error: std::option::Option<std::string::String>,\n    /// Server-provided `error_description`, when returned.\n    pub server_description: std::option::Option<std::string::String>,\n    cause: std::option::Option<BoxError>,\n}\nimpl std::fmt::Debug for AuthError {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"AuthError\")\n            .field(\"scheme\", &self.scheme)\n            .field(\"flow\", &self.flow)\n            .field(\"kind\", &self.kind)\n            .field(\"server_error\", &self.server_error)\n            .field(\"server_description\", &self.server_description.is_some())\n            .field(\"has_cause\", &self.cause.is_some())\n            .finish()\n    }\n}\nimpl std::fmt::Display for AuthError {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        write!(\n            f,\n            \"oauth {} for scheme {:?} failed: {}\",\n            self.flow, self.scheme, self.kind\n        )?;\n        if let std::option::Option::Some(error) = &self.server_error {\n            write!(f, \" ({error}\")?;\n            if let std::option::Option::Some(description) = &self.server_description {\n                write!(f, \": {description}\")?;\n            }\n            f.write_str(\")\")?;\n        }\n        std::result::Result::Ok(())\n    }\n}\nimpl std::error::Error for AuthError {\n    fn source(&self) -> std::option::Option<&(dyn std::error::Error + 'static)> {\n        self.cause\n            .as_ref()\n            .map(|cause| cause.as_ref() as &(dyn std::error::Error + 'static))\n    }\n}\n\n/// The lifecycle step named by [`AuthError`].\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum AuthFlow {\n    ClientCredentials,\n    AuthorizationCode,\n    DeviceAuthorization,\n    DeviceToken,\n    Refresh,\n    Revocation,\n    Introspection,\n}\nimpl std::fmt::Display for AuthFlow {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.write_str(match self {\n            Self::ClientCredentials => \"client-credentials\",\n            Self::AuthorizationCode => \"authorization-code\",\n            Self::DeviceAuthorization => \"device authorization\",\n            Self::DeviceToken => \"device token poll\",\n            Self::Refresh => \"refresh\",\n            Self::Revocation => \"revocation\",\n            Self::Introspection => \"introspection\",\n        })\n    }\n}\n\n",
    );
    module.push_str(if discovery {
        AUTH_ERROR_KIND_DISCOVERY
    } else {
        AUTH_ERROR_KIND_PLAIN
    });
    module.push_str(
        "/// One started RFC 8628 device authorization transaction. Debug omits the\n/// device code, which is the polling credential.\n#[derive(Clone)]\npub struct DeviceAuthorization {\n    /// The device code from the server.\n    pub device_code: std::string::String,\n    /// The human-readable code to enter at the verification URI.\n    pub user_code: std::string::String,\n    /// Where the user completes the authorization.\n    pub verification_uri: std::string::String,\n    /// An optional URI that embeds the user code.\n    pub verification_uri_complete: std::option::Option<std::string::String>,\n    /// When the transaction expires; an absent `expires_in` compiles to the\n    /// far future.\n    pub expires_at: std::time::SystemTime,\n    /// The server's polling interval, or the RFC default of five seconds.\n    pub interval_seconds: u64,\n}\nimpl std::fmt::Debug for DeviceAuthorization {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"DeviceAuthorization\")\n            .field(\"device_code\", &\"<redacted>\")\n            .field(\"user_code\", &self.user_code)\n            .field(\"verification_uri\", &self.verification_uri)\n            .field(\n                \"verification_uri_complete\",\n                &self.verification_uri_complete,\n            )\n            .field(\"expires_at\", &self.expires_at)\n            .field(\"interval_seconds\", &self.interval_seconds)\n            .finish()\n    }\n}\n\n/// One device-grant token poll.\n#[derive(Debug, Clone, PartialEq, Eq)]\npub enum DevicePoll {\n    /// The user approved; the token is stored for the scheme and returned.\n    Complete(TokenSet),\n    /// The user has not approved yet; wait at least `retry_after_seconds`.\n    Pending {\n        retry_after_seconds: u64,\n    },\n    /// The server reported the transaction expired.\n    Expired,\n}\n\n/// One device-grant poll outcome before public pacing translation: the\n/// single-poll function and the paced completion loop share it so the loop\n/// can tell `authorization_pending` from `slow_down` and pace cumulatively.\n#[derive(Debug)]\nenum DevicePollStep {\n    /// The user approved; the set is stored for the scheme.\n    Complete(TokenSet),\n    /// The user has not approved yet.\n    Pending,\n    /// The server demands slower polling.\n    SlowDown,\n    /// The server reported the transaction expired.\n    Expired,\n}\n\n/// One started authorization-code + PKCE S256 transaction. Direct the\n/// resource owner to `authorization_url`; complete the transaction with the\n/// callback query parameters exactly once. The verifier and the one-time\n/// consumption gate are bound to the transaction value itself, so a replay\n/// through any copy is refused whatever the first attempt returned.\n#[derive(Clone)]\npub struct AuthorizationTransaction {\n    /// The source scheme this transaction belongs to.\n    pub scheme: std::string::String,\n    /// The complete authorization request: response type, client id,\n    /// redirect URI, scope, the one-time state and the S256 challenge.\n    pub authorization_url: std::string::String,\n    /// The one-time state the callback must repeat exactly.\n    pub state: std::string::String,\n    /// The RFC 7636 S256 challenge derived from the verifier.\n    pub code_challenge: std::string::String,\n    /// The PKCE verifier. Credential material: never print, log or\n    /// stream it.\n    pub code_verifier: std::string::String,\n    /// The redirect URI bound at start; repeated exactly in the exchange.\n    pub redirect_uri: std::option::Option<std::string::String>,\n    /// The requested scopes in RFC 6749 space-separated form.\n    pub scope: std::option::Option<std::string::String>,\n    /// When the transaction started. Lifetime policy belongs to the\n    /// authorization server.\n    pub created_at: std::time::SystemTime,\n    consumed: std::sync::Arc<std::sync::Mutex<bool>>,\n}\nimpl std::fmt::Debug for AuthorizationTransaction {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.debug_struct(\"AuthorizationTransaction\")\n            .field(\"scheme\", &self.scheme)\n            .field(\"authorization_url\", &self.authorization_url)\n            .field(\"state\", &self.state)\n            .field(\"code_challenge\", &self.code_challenge)\n            .field(\"code_verifier\", &\"<redacted>\")\n            .field(\"redirect_uri\", &self.redirect_uri)\n            .field(\"scope\", &self.scope)\n            .field(\"created_at\", &self.created_at)\n            .finish()\n    }\n}\nimpl AuthorizationTransaction {\n    /// Whether this transaction was consumed by an earlier completion\n    /// attempt (successful or not). Copies share the gate.\n    #[must_use]\n    pub fn consumed(&self) -> bool {\n        *self\n            .consumed\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner)\n    }\n}\n\n/// Derive the RFC 7636 S256 `code_challenge` of a verifier: the unpadded\n/// base64url of the SHA-256 digest of the ASCII verifier. Callers replaying\n/// a transaction against their own tooling can recompute the challenge\n/// exactly as the runtime did.\n#[must_use]\npub fn pkce_s256_challenge(verifier: &str) -> String {\n    base64url(&sha256(verifier.as_bytes()))\n}\n",
    );
    module.push_str(if discovery {
        TYPES_SCHEME_DISCOVERY
    } else {
        TYPES_SCHEME_PLAIN
    });
    module.push_str(
        "/// One single-flight acquisition round. A losing caller blocks its thread on\n/// the condition variable until the winning round completes; see the module\n/// documentation for the concurrency simplification.\nstruct Round {\n    done: std::sync::Mutex<bool>,\n    signal: std::sync::Condvar,\n}\nimpl Round {\n    fn new() -> Self {\n        Self {\n            done: std::sync::Mutex::new(false),\n            signal: std::sync::Condvar::new(),\n        }\n    }\n    fn complete(&self) {\n        let mut done = self\n            .done\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner);\n        *done = true;\n        drop(done);\n        self.signal.notify_all();\n    }\n    fn wait(&self) {\n        let mut done = self\n            .done\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner);\n        while !*done {\n            done = self\n                .signal\n                .wait(done)\n                .unwrap_or_else(std::sync::PoisonError::into_inner);\n        }\n    }\n}\nenum Entered {\n    /// Another caller owns the in-flight round for this scheme.\n    Wait(std::sync::Arc<Round>),\n    /// This caller owns the fresh round and performs the acquisition.\n    Proceed(std::sync::Arc<Round>),\n}\n\n",
    );
    module.push_str(if discovery {
        TOKEN_SESSIONS_DISCOVERY
    } else {
        TOKEN_SESSIONS_PLAIN
    });
}

/// Emit one scheme's source constant and synthetic endpoint statics, and
/// return the inline `Scheme` table literal that references them.
#[allow(clippy::too_many_lines)]
fn push_scheme(
    module: &mut String,
    index: usize,
    scheme: &CompiledScheme,
    plan: &HttpPlan,
    discovery: bool,
) -> String {
    let source = match &scheme.source {
        Some(source) => format!(
            "crate::http::Source {{ document: {:?}, pointer: {:?} }}",
            source.document().as_str(),
            source.pointer()
        ),
        None => "OAUTH".to_owned(),
    };
    module.push_str(&format!(
        "// ---- compiled source scheme {:?} ----\n\nconst SOURCE_{index}: Source = {source};\n",
        scheme.name,
    ));
    if discovery {
        module.push_str(&format!(
            "/// The single-flight round key for scheme {:?}'s discovery fetches.\nconst DISCOVERY_ROUND_{index}: &str = \"discovery\\x00{index}\";\n",
            scheme.name,
        ));
    }
    // Collect every endpoint URL first so shared URLs reuse one static.
    let mut endpoints = Endpoints::new(index);
    let cc = scheme.client_credentials.as_ref().map(|grant| {
        (
            endpoints.name(&grant.token_url),
            grant
                .refresh_url
                .as_deref()
                .and_then(|url| endpoints.name(url)),
        )
    });
    let device = scheme.device.as_ref().map(|grant| {
        (
            endpoints.name(&grant.token_url),
            grant
                .refresh_url
                .as_deref()
                .and_then(|url| endpoints.name(url)),
        )
    });
    let authorization_code = scheme.authorization_code.as_ref().map(|grant| {
        (
            endpoints.name(&grant.token_url),
            grant
                .refresh_url
                .as_deref()
                .and_then(|url| endpoints.name(url)),
        )
    });
    let device_authorization = scheme
        .device_authorization
        .as_deref()
        .and_then(|url| endpoints.name(url));
    let revocation = scheme
        .revocation
        .as_deref()
        .and_then(|url| endpoints.name(url));
    let introspection = scheme
        .introspection
        .as_deref()
        .and_then(|url| endpoints.name(url));
    for (origin, path, name) in &endpoints.declared {
        module.push_str(&operation_static(
            name,
            &source,
            origin,
            path,
            &format!("oauth: scheme {} token endpoint", scheme.name),
            plan,
        ));
    }
    // Grants and the scheme literal are inlined into the table: the endpoints
    // are referenced by address, so no static is ever read by value.
    let grant_literal = |(token, refresh): &(Option<String>, Option<String>)| match token {
        Some(token) => format!(
            "std::option::Option::Some(Grant {{ token: &{token}, refresh: {} }})",
            match refresh {
                Some(name) => format!("std::option::Option::Some(&{name})"),
                None => "std::option::Option::None".to_owned(),
            }
        ),
        None => "std::option::Option::None".to_owned(),
    };
    let endpoint_literal = |value: &Option<String>| match value {
        Some(name) => format!("std::option::Option::Some(&{name})"),
        None => "std::option::Option::None".to_owned(),
    };
    let url_literal = |value: &Option<String>| match value {
        Some(url) => format!("std::option::Option::Some({url:?})"),
        None => "std::option::Option::None".to_owned(),
    };
    let env = |value: &Option<String>| match value {
        Some(name) => format!("std::option::Option::Some({name:?})"),
        None => "std::option::Option::None".to_owned(),
    };
    let discovery_fields = if discovery {
        format!(
            "        discovery_url: {},\n        discovery_round: DISCOVERY_ROUND_{index},\n",
            url_literal(&scheme.discovery),
        )
    } else {
        String::new()
    };
    format!(
        "Scheme {{\n        name: {:?},\n        source: SOURCE_{index},\n        skew_seconds: {},\n        client_id_env: {},\n        client_secret_env: {},\n        secret_basic: {},\n        client_credentials: {},\n        authorization_url: {},\n        authorization_code: {},\n        device: {},\n        device_authorization: {},\n        revocation: {},\n        introspection: {},\n{discovery_fields}    }}",
        scheme.name,
        scheme.skew_seconds,
        env(&scheme.client_id_env),
        env(&scheme.client_secret_env),
        scheme.secret_basic,
        grant_literal(cc.as_ref().unwrap_or(&(None, None))),
        url_literal(&scheme.authorization_url),
        grant_literal(authorization_code.as_ref().unwrap_or(&(None, None))),
        grant_literal(device.as_ref().unwrap_or(&(None, None))),
        endpoint_literal(&device_authorization),
        endpoint_literal(&revocation),
        endpoint_literal(&introspection),
    )
}

#[allow(clippy::too_many_arguments)]
fn operation_static(
    name: &str,
    source: &str,
    origin: &str,
    path: &str,
    operation_id: &str,
    plan: &HttpPlan,
) -> String {
    let config = &plan.config;
    let json = &config.codecs.json_limits;
    format!(
        "static {name}: Operation = Operation {{\n    operation_id: {operation_id:?},\n    source: {source},\n    provenance: crate::http::Provenance {{ use_site: {source}, terminal: {source}, references: &[], use_site_resource: std::option::Option::None, terminal_resource: std::option::Option::None, reference_resources: &[] }},\n    method: \"POST\",\n    path_template: {path:?},\n    servers: &[crate::http::Server {{ source: {source}, document_base: {source}, provenance: std::option::Option::None, template: {origin:?}, name: std::option::Option::None, description: std::option::Option::None, variables: &[] }}],\n    security: crate::http::Security::NoAuth({source}),\n    parameters: &[crate::http::Parameter {{ name: \"authorization\", source: {source}, location: crate::http::ParameterLocation::Header, required: true, serialization: crate::http::Serialization::Style {{ style: crate::http::Style::Simple, explode: false, shape: crate::http::Shape::Scalar(crate::http::ScalarType::String), encoding: crate::http::PercentEncoding::None }} }}],\n    request_media: &[crate::http::Media {{ source: {source}, declared: \"application/x-www-form-urlencoded\", range: crate::http::MediaRange::Concrete(\"application\", \"x-www-form-urlencoded\"), parameters: &[], kind: crate::http::MediaKind::Form }}],\n    responses: &[],\n    accept: \"\",\n    limits: crate::http::Limits {{ request: {}, response: {}, part: {}, item: {}, chunk: {}, header: {} }},\n    json_limits: crate::JsonLimits {{ max_input_bytes: {}, max_output_bytes: {}, max_depth: {}, max_work: {}}},\n}};\n",
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_part_bytes,
        config.max_stream_item_bytes,
        config.max_chunk_bytes,
        config.max_header_bytes,
        json.max_input_bytes,
        json.max_output_bytes,
        json.max_depth,
        json.max_work,
    )
}

fn push_limits(module: &mut String, plan: &HttpPlan) {
    let json = &plan.config.codecs.json_limits;
    module.push_str(&format!(
        "/// Exact JSON limits for token, device and introspection responses.\nconst TOKEN_JSON_LIMITS: crate::JsonLimits = crate::JsonLimits {{\n    max_input_bytes: {},\n    max_output_bytes: {},\n    max_depth: {},\n    max_work: {},\n}};\n/// Bounded response capture attached to rejected token requests. Rejected\n/// bodies carry server error metadata, never issued tokens.\nconst CAPTURE_BYTES: usize = 1024;\n/// Source marker for failures before any compiled scheme is resolved.\nconst OAUTH: Source = Source {{ document: \"suspect-oauth\", pointer: \"\" }};\n\n",
        json.max_input_bytes,
        json.max_output_bytes,
        json.max_depth,
        json.max_work,
    ));
}

fn push_methods(module: &mut String, discovery: bool) {
    module.push_str(if discovery {
        METHODS_CC_DISCOVERY
    } else {
        METHODS_CC_PLAIN
    });
    module.push_str(if discovery {
        METHODS_REFRESH_DISCOVERY
    } else {
        METHODS_REFRESH_PLAIN
    });
    module.push_str(if discovery {
        METHODS_REVOKE_DISCOVERY
    } else {
        METHODS_REVOKE_PLAIN
    });
    module.push_str(if discovery {
        METHODS_INTROSPECT_DISCOVERY
    } else {
        METHODS_INTROSPECT_PLAIN
    });
    if discovery {
        // The plain slices concatenate into one impl block; the discovery
        // variants are complete impl blocks, so the shared rest opens its
        // own.
        module.push_str("impl<S: TokenStore> TokenSessions<S> {\n");
    }
    module.push_str(
        "    /// Start the RFC 8628 device grant at the compiled device authorization\n    /// endpoint. Poll with [`TokenSessions::poll_device_token`], waiting at\n    /// least the returned interval between attempts; pacing stays with the\n    /// caller so no request loop or sleep is embedded here.\n    ///\n    /// # Errors\n    /// Unknown schemes, unconfigured endpoints, missing client credentials,\n    /// transport failures, and rejected or unusable responses.\n    pub async fn begin_device_authorization<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n    ) -> std::result::Result<DeviceAuthorization, SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        if compiled.device_authorization.is_none() || compiled.device.is_none() {\n            return std::result::Result::Err(unavailable(\n                scheme,\n                AuthFlow::DeviceAuthorization,\n            ));\n        };\n        let operation = compiled.device_authorization.expect(\"checked above\");\n        let client_id = self.public_client_id_if_public(compiled);\n        let mut fields = std::vec::Vec::new();\n        if let std::option::Option::Some(id) = &client_id {\n            fields.push((\"client_id\", id.as_str()));\n        }\n        let authorization = self\n            .authorization(compiled, AuthFlow::DeviceAuthorization)?;\n        let raw = self\n            .post(\n                client,\n                compiled,\n                AuthFlow::DeviceAuthorization,\n                operation,\n                &fields,\n                authorization,\n            )\n            .await?;\n        if !(200..300).contains(&raw.status) {\n            return std::result::Result::Err(rejection(\n                compiled,\n                AuthFlow::DeviceAuthorization,\n                &raw,\n            ));\n        }\n        let value =\n            crate::parse_json_bytes(&raw.body, TOKEN_JSON_LIMITS).map_err(|cause| {\n                invalid_response(\n                    compiled,\n                    AuthFlow::DeviceAuthorization,\n                    std::option::Option::Some(std::boxed::Box::new(cause)),\n                )\n            })?;\n        let object = match &value {\n            Nullable::Value(JsonNonNullValue::Object(map)) => map,\n            _ => {\n                return std::result::Result::Err(invalid_response(\n                    compiled,\n                    AuthFlow::DeviceAuthorization,\n                    std::option::Option::None,\n                ));\n            }\n        };\n        let text = |name: &str| {\n            object.get(name).and_then(|value| match value {\n                Nullable::Value(JsonNonNullValue::String(text)) if !text.is_empty() => {\n                    std::option::Option::Some(text.clone())\n                }\n                _ => std::option::Option::None,\n            })\n        };\n        let number = |name: &str, default: u64| {\n            object\n                .get(name)\n                .and_then(|value| match value {\n                    Nullable::Value(JsonNonNullValue::Number(token)) => token\n                        .as_str()\n                        .parse::<JsonInteger>()\n                        .ok()\n                        .and_then(|token| token.to_u128()),\n                    _ => std::option::Option::None,\n                })\n                .and_then(|value| u64::try_from(value).ok())\n                .unwrap_or(default)\n        };\n        let (Some(device_code), Some(user_code), Some(verification_uri)) =\n            (text(\"device_code\"), text(\"user_code\"), text(\"verification_uri\"))\n        else {\n            return std::result::Result::Err(invalid_response(\n                compiled,\n                AuthFlow::DeviceAuthorization,\n                std::option::Option::None,\n            ));\n        };\n        let now = std::time::SystemTime::now();\n        let expires_at = now\n            .checked_add(std::time::Duration::from_secs(number(\n                \"expires_in\",\n                300,\n            )))\n            .unwrap_or_else(||\n                now.checked_add(std::time::Duration::from_secs(365 * 24 * 60 * 60))\n                    .expect(\"one year fits a SystemTime offset\"),\n            );\n        std::result::Result::Ok(DeviceAuthorization {\n            device_code,\n            user_code,\n            verification_uri,\n            verification_uri_complete: text(\"verification_uri_complete\"),\n            expires_at,\n            interval_seconds: number(\"interval\", 5),\n        })\n    }\n\n    /// One device-grant token poll against the compiled device token URL.\n    /// `Pending` carries the minimum interval to wait before the next attempt\n    /// (`slow_down` adds the RFC's five seconds); a complete token is stored\n    /// for the scheme. Pacing stays with the caller; see\n    /// [`TokenSessions::poll_device_token_until_complete`] for the paced\n    /// completion loop.\n    ///\n    /// # Errors\n    /// Unknown schemes, missing configuration or credentials, transport\n    /// failures, and unusable or rejected responses.\n    pub async fn poll_device_token<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n        transaction: &DeviceAuthorization,\n    ) -> std::result::Result<DevicePoll, SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        let Some(grant) = compiled.device.as_ref() else {\n            return std::result::Result::Err(unavailable(scheme, AuthFlow::DeviceToken));\n        };\n        match self\n            .poll_device_step(client, compiled, grant, transaction)\n            .await?\n        {\n            DevicePollStep::Complete(set) => {\n                std::result::Result::Ok(DevicePoll::Complete(set))\n            }\n            DevicePollStep::Pending => std::result::Result::Ok(DevicePoll::Pending {\n                retry_after_seconds: transaction.interval_seconds,\n            }),\n            DevicePollStep::SlowDown => std::result::Result::Ok(DevicePoll::Pending {\n                retry_after_seconds: transaction.interval_seconds.saturating_add(5),\n            }),\n            DevicePollStep::Expired => std::result::Result::Ok(DevicePoll::Expired),\n        }\n    }\n\n    async fn acquire<T: Transport>(\n        &self,\n        client: &Client<T>,\n        compiled: &'static Scheme,\n        grant: &Grant,\n    ) -> std::result::Result<TokenSet, SdkError> {\n        let client_id = self.public_client_id_if_public(compiled);\n        let mut fields = std::vec![(\"grant_type\", \"client_credentials\")];\n        if let std::option::Option::Some(id) = &client_id {\n            fields.push((\"client_id\", id.as_str()));\n        }\n        let authorization = self.authorization(compiled, AuthFlow::ClientCredentials)?;\n        let raw = self\n            .post(\n                client,\n                compiled,\n                AuthFlow::ClientCredentials,\n                grant.token,\n                &fields,\n                authorization,\n            )\n            .await?;\n        let set = token_set(compiled, AuthFlow::ClientCredentials, &raw)?;\n        self.store.replace(compiled.name, set.clone()).await;\n        std::result::Result::Ok(set)\n    }\n\n    fn enter(&self, scheme: &'static str) -> Entered {\n        let mut rounds = self\n            .rounds\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner);\n        match rounds.get_mut(scheme) {\n            std::option::Option::Some(occupied) => {\n                let done = *occupied\n                    .done\n                    .lock()\n                    .unwrap_or_else(std::sync::PoisonError::into_inner);\n                if done {\n                    // The previous round completed: start a new one and lead it.\n                    let round = std::sync::Arc::new(Round::new());\n                    *occupied = std::sync::Arc::clone(&round);\n                    Entered::Proceed(round)\n                } else {\n                    Entered::Wait(std::sync::Arc::clone(occupied))\n                }\n            }\n            std::option::Option::None => {\n                let round = std::sync::Arc::new(Round::new());\n                rounds.insert(scheme, std::sync::Arc::clone(&round));\n                Entered::Proceed(round)\n            }\n        }\n    }\n\n    fn public_client_id(&self, compiled: &'static Scheme) -> std::option::Option<std::string::String> {\n        self.client_credential(compiled.client_id_env, self.client_id.as_deref())\n    }\n\n    /// The client id for public clients only; confidential clients keep\n    /// their single `client_secret_basic` authentication method.\n    fn public_client_id_if_public(\n        &self,\n        compiled: &'static Scheme,\n    ) -> std::option::Option<std::string::String> {\n        if compiled.secret_basic {\n            return std::option::Option::None;\n        }\n        self.public_client_id(compiled)\n    }\n\n    fn client_credential(\n        &self,\n        env: std::option::Option<&'static str>,\n        explicit: std::option::Option<&str>,\n    ) -> std::option::Option<std::string::String> {\n        explicit\n            .map(str::to_owned)\n            .or_else(|| {\n                env.and_then(|name| {\n                    std::env::var(name)\n                        .ok()\n                        .filter(|value| !value.is_empty())\n                })\n            })\n    }\n\n    /// The `client_secret_basic` authorization header value, or `None` for\n    /// public clients. Values travel only into this request; they are never\n    /// stored, logged or attached to an error.\n    fn authorization(\n        &self,\n        compiled: &'static Scheme,\n        flow: AuthFlow,\n    ) -> std::result::Result<std::option::Option<std::string::String>, SdkError> {\n        if !compiled.secret_basic {\n            return std::result::Result::Ok(std::option::Option::None);\n        }\n        let id = self\n            .client_credential(compiled.client_id_env, self.client_id.as_deref())\n            .ok_or_else(|| {\n                missing_credentials(compiled, flow, compiled.client_id_env)\n            })?;\n        let secret = self\n            .client_credential(\n                compiled.client_secret_env,\n                self.client_secret.as_deref(),\n            )\n            .ok_or_else(|| {\n                missing_credentials(compiled, flow, compiled.client_secret_env)\n            })?;\n        std::result::Result::Ok(std::option::Option::Some(format!(\n            \"Basic {}\",\n            base64(format!(\n                \"{}:{}\",\n                form_component(&id),\n                form_component(&secret)\n            ).as_bytes())\n        )))\n    }\n\n    /// One form-encoded request through the client's public send surface. The\n    /// synthetic token descriptor declares the `authorization` header, so\n    /// `client_secret_basic` credentials ride the header like any other\n    /// source-bound credential; public clients pass none.\n    async fn post<T: Transport>(\n        &self,\n        client: &Client<T>,\n        compiled: &'static Scheme,\n        flow: AuthFlow,\n        operation: &'static Operation,\n        fields: &[(&str, &str)],\n        authorization: std::option::Option<std::string::String>,\n    ) -> std::result::Result<RawResponse, SdkError> {\n        let mut body = std::string::String::new();\n        for (index, (name, value)) in fields.iter().enumerate() {\n            if index != 0 {\n                body.push('&');\n            }\n            form_pair(&mut body, name, value);\n        }\n        let prepared = match PreparedBody::new(\n            operation,\n            0,\n            \"application/x-www-form-urlencoded\".to_owned(),\n            body.into_bytes(),\n        ) {\n            std::result::Result::Ok(prepared) => prepared,\n            std::result::Result::Err(error) => {\n                return std::result::Result::Err(transport_failure(\n                    error, compiled, flow,\n                ));\n            }\n        };\n        let parameters = match &authorization {\n            std::option::Option::Some(value) => std::vec![ParameterValue {\n                parameter: operation.parameters[0],\n                value: Nullable::Value(JsonNonNullValue::String(value.clone())),\n            }],\n            std::option::Option::None => std::vec::Vec::new(),\n        };\n        match client\n            .send(operation, &parameters, std::option::Option::Some(prepared))\n            .await\n        {\n            std::result::Result::Ok(raw) => std::result::Result::Ok(raw),\n            std::result::Result::Err(error) => {\n                std::result::Result::Err(transport_failure(error, compiled, flow))\n            }\n        }\n    }\n}\n",
    );
}

/// The compiled lifecycle methods: byte-exact without discovery.
const METHODS_CC_PLAIN: &str = "impl<S: TokenStore> TokenSessions<S> {\n    /// Acquire a client-credentials token for the compiled source scheme\n    /// `scheme`. A stored set still inside its compiled clock skew is returned\n    /// without a request; an absent or expired set is acquired once per\n    /// single-flight round and atomically replaced in the store. Only the\n    /// `grant_type` travels on the wire: no scope is requested and the server\n    /// issues its default.\n    ///\n    /// # Errors\n    /// Unknown scheme names, missing configuration or client credentials,\n    /// transport failures, and unusable or rejected token responses.\n    pub async fn client_credentials_token<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n    ) -> std::result::Result<TokenSet, SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        let Some(grant) = compiled.client_credentials.as_ref() else {\n            return std::result::Result::Err(unavailable(\n                scheme,\n                AuthFlow::ClientCredentials,\n            ));\n        };\n        loop {\n            if let std::option::Option::Some(set) = self.store.load(compiled.name).await\n                && fresh(&set, compiled.skew_seconds)\n            {\n                return std::result::Result::Ok(set);\n            }\n            match self.enter(compiled.name) {\n                Entered::Wait(round) => {\n                    // The winning round completes its store replacement before\n                    // releasing the waiters; re-probe the store, then either\n                    // return the fresh set or start a new round.\n                    round.wait();\n                }\n                Entered::Proceed(round) => {\n                    let result = self.acquire(client, compiled, grant).await;\n                    round.complete();\n                    return result;\n                }\n            }\n        }\n    }\n\n";
const METHODS_REFRESH_PLAIN: &str = "    /// Exchange `set`'s refresh token at the compiled refresh URL, or at the\n    /// flow's token URL when no refresh URL is declared. A rotated refresh\n    /// token is adopted; otherwise the previous one is retained. An\n    /// `invalid_grant` rejection clears the stored set for the scheme.\n    ///\n    /// # Errors\n    /// Unknown schemes, missing configuration or credentials, transport\n    /// failures, and unusable or rejected token responses.\n    pub async fn refresh_token<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n        set: &TokenSet,\n    ) -> std::result::Result<TokenSet, SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        let Some(grant) = compiled\n            .client_credentials\n            .as_ref()\n            .or(compiled.authorization_code.as_ref())\n            .or(compiled.device.as_ref())\n        else {\n            return std::result::Result::Err(unavailable(scheme, AuthFlow::Refresh));\n        };\n        let Some(refresh_token) = set.refresh_token.as_deref() else {\n            return std::result::Result::Err(missing_refresh(compiled));\n        };\n        let operation = grant.refresh.unwrap_or(grant.token);\n        let client_id = self.public_client_id_if_public(compiled);\n        let mut fields = std::vec![\n            (\"grant_type\", \"refresh_token\"),\n            (\"refresh_token\", refresh_token),\n        ];\n        if let std::option::Option::Some(id) = &client_id {\n            fields.push((\"client_id\", id.as_str()));\n        }\n        let authorization = self.authorization(compiled, AuthFlow::Refresh)?;\n        let raw = self\n            .post(client, compiled, AuthFlow::Refresh, operation, &fields, authorization)\n            .await?;\n        let mut refreshed = match token_set(compiled, AuthFlow::Refresh, &raw) {\n            std::result::Result::Ok(set) => set,\n            std::result::Result::Err(error) => {\n                if rejected(&error, \"invalid_grant\") {\n                    self.store.clear(compiled.name).await;\n                }\n                return std::result::Result::Err(error);\n            }\n        };\n        if refreshed.refresh_token.is_none() {\n            refreshed.refresh_token = set.refresh_token.clone();\n        }\n        self.store\n            .replace(compiled.name, refreshed.clone())\n            .await;\n        std::result::Result::Ok(refreshed)\n    }\n\n";
const METHODS_REVOKE_PLAIN: &str = "    /// Revoke `set`'s access token at the compiled revocation endpoint and\n    /// clear the stored set for the scheme. Refuses schemes whose compiled\n    /// plan carries no revocation endpoint.\n    ///\n    /// # Errors\n    /// Unknown schemes, unconfigured endpoints, missing client credentials,\n    /// transport failures, and rejected revocations.\n    pub async fn revoke_token<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n        set: &TokenSet,\n    ) -> std::result::Result<(), SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        let Some(operation) = compiled.revocation else {\n            return std::result::Result::Err(unavailable(scheme, AuthFlow::Revocation));\n        };\n        let client_id = self.public_client_id_if_public(compiled);\n        let mut fields = std::vec![\n            (\"token\", set.access_token.as_str()),\n            (\"token_type_hint\", \"access_token\"),\n        ];\n        if let std::option::Option::Some(id) = &client_id {\n            fields.push((\"client_id\", id.as_str()));\n        }\n        let authorization = self.authorization(compiled, AuthFlow::Revocation)?;\n        let raw = self\n            .post(\n                client,\n                compiled,\n                AuthFlow::Revocation,\n                operation,\n                &fields,\n                authorization,\n            )\n            .await?;\n        if !(200..300).contains(&raw.status) {\n            return std::result::Result::Err(rejection(\n                compiled,\n                AuthFlow::Revocation,\n                &raw,\n            ));\n        }\n        self.store.clear(compiled.name).await;\n        std::result::Result::Ok(())\n    }\n\n";
const METHODS_INTROSPECT_PLAIN: &str = "    /// Introspect `set`'s access token at the compiled introspection endpoint,\n    /// returning the RFC 7662 document as an exact JSON value. Refuses\n    /// schemes whose compiled plan carries no introspection endpoint.\n    ///\n    /// # Errors\n    /// Unknown schemes, unconfigured endpoints, missing client credentials,\n    /// transport failures, and rejected or unusable responses.\n    pub async fn introspect_token<T: Transport>(\n        &self,\n        client: &Client<T>,\n        scheme: &str,\n        set: &TokenSet,\n    ) -> std::result::Result<JsonValue, SdkError> {\n        let compiled = compiled_scheme(scheme)?;\n        let Some(operation) = compiled.introspection else {\n            return std::result::Result::Err(unavailable(scheme, AuthFlow::Introspection));\n        };\n        let client_id = self.public_client_id_if_public(compiled);\n        let mut fields = std::vec![(\"token\", set.access_token.as_str())];\n        if let std::option::Option::Some(id) = &client_id {\n            fields.push((\"client_id\", id.as_str()));\n        }\n        let authorization = self.authorization(compiled, AuthFlow::Introspection)?;\n        let raw = self\n            .post(\n                client,\n                compiled,\n                AuthFlow::Introspection,\n                operation,\n                &fields,\n                authorization,\n            )\n            .await?;\n        if !(200..300).contains(&raw.status) {\n            return std::result::Result::Err(rejection(\n                compiled,\n                AuthFlow::Introspection,\n                &raw,\n            ));\n        }\n        crate::parse_json_bytes(&raw.body, TOKEN_JSON_LIMITS).map_err(|cause| {\n            invalid_response(\n                compiled,\n                AuthFlow::Introspection,\n                std::option::Option::Some(std::boxed::Box::new(cause)),\n            )\n        })\n    }\n\n";

/// Authorization-code + PKCE S256 and the paced device-completion loop,
/// emitted as a second `impl` block over the same sessions value.
fn push_authorization_code_methods(module: &mut String) {
    module.push_str(AUTHORIZATION_CODE_METHODS);
}

const AUTHORIZATION_CODE_METHODS: &str = r#"impl<S: TokenStore> TokenSessions<S> {
    /// Start an authorization-code + PKCE S256 transaction (RFC 6749 section
    /// 4.1, RFC 7636 section 4). No network call: the returned
    /// [`AuthorizationTransaction`] carries the complete authorization URL,
    /// its one-time state and the bound PKCE verifier, and is consumed by
    /// [`TokenSessions::complete_authorization`]. The client id resolves like
    /// every other client input — explicit on the sessions value first, then
    /// the compiled environment variable read now — and a missing id is a
    /// typed failure because the authorization request cannot be built
    /// without one.
    ///
    /// # Errors
    /// Unknown scheme names, schemes whose compiled plan carries no
    /// executable authorization-code flow, missing client identity, and
    /// platforms or entropy sources that cannot supply cryptographic
    /// randomness.
    pub fn begin_authorization(
        &self,
        scheme: &str,
        redirect_uri: std::option::Option<&str>,
        scopes: &[&str],
    ) -> std::result::Result<AuthorizationTransaction, SdkError> {
        let compiled = compiled_scheme(scheme)?;
        if compiled.authorization_code.is_none() || compiled.authorization_url.is_none() {
            return std::result::Result::Err(unavailable(
                scheme,
                AuthFlow::AuthorizationCode,
            ));
        }
        let Some(authorization_url) = compiled.authorization_url else {
            return std::result::Result::Err(unavailable(
                scheme,
                AuthFlow::AuthorizationCode,
            ));
        };
        // The authorization request is a browser redirect: it always carries
        // the client id, whatever the token-endpoint authentication method.
        let client_id = self
            .client_credential(compiled.client_id_env, self.client_id.as_deref())
            .ok_or_else(|| {
                missing_credentials(
                    compiled,
                    AuthFlow::AuthorizationCode,
                    compiled.client_id_env,
                )
            })?;
        let code_verifier = random_value()?;
        let state = random_value()?;
        let code_challenge = pkce_s256_challenge(&code_verifier);
        let mut url = std::string::String::from(authorization_url);
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str("response_type=code");
        url.push_str("&client_id=");
        form_component_into(&mut url, &client_id);
        if let std::option::Option::Some(redirect) = redirect_uri {
            url.push_str("&redirect_uri=");
            form_component_into(&mut url, redirect);
        }
        url.push_str("&state=");
        form_component_into(&mut url, &state);
        url.push_str("&code_challenge=");
        form_component_into(&mut url, &code_challenge);
        url.push_str("&code_challenge_method=S256");
        let scope = if scopes.is_empty() {
            std::option::Option::None
        } else {
            std::option::Option::Some(scopes.join(" "))
        };
        if let std::option::Option::Some(joined) = &scope {
            url.push_str("&scope=");
            form_component_into(&mut url, joined);
        }
        std::result::Result::Ok(AuthorizationTransaction {
            scheme: scheme.to_owned(),
            authorization_url: url,
            state,
            code_challenge,
            code_verifier,
            redirect_uri: redirect_uri.map(str::to_owned),
            scope,
            created_at: std::time::SystemTime::now(),
            consumed: std::sync::Arc::new(std::sync::Mutex::new(false)),
        })
    }

    /// Complete the transaction exactly once with the callback's query
    /// parameters: the one-time state is compared in constant time, a
    /// server-declared error surfaces as a typed refusal, and the code is
    /// exchanged with the retained PKCE verifier at the compiled token
    /// endpoint under the scheme's compiled client authentication. The
    /// resulting set is stored for the scheme. The first call consumes the
    /// transaction whatever its outcome; a second attempt is a typed
    /// [`AuthErrorKind::TransactionUsed`] refusal.
    ///
    /// # Errors
    /// Consumed transactions, state mismatches, authorization denials,
    /// callbacks without a code, unknown schemes, missing configuration or
    /// credentials, transport failures, and unusable or rejected responses.
    pub async fn complete_authorization<T: Transport>(
        &self,
        client: &Client<T>,
        transaction: &AuthorizationTransaction,
        callback: &[(&str, &str)],
    ) -> std::result::Result<TokenSet, SdkError> {
        {
            let mut consumed = transaction
                .consumed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *consumed {
                return std::result::Result::Err(transaction_used(&transaction.scheme));
            }
            *consumed = true;
        }
        let state = callback_value(callback, "state").unwrap_or_default();
        if !constant_time_eq(state.as_bytes(), transaction.state.as_bytes()) {
            return std::result::Result::Err(state_mismatch(&transaction.scheme));
        }
        if let std::option::Option::Some(error) = callback_value(callback, "error")
            && !error.is_empty()
        {
            return std::result::Result::Err(authorization_denied(
                &transaction.scheme,
                &error,
                callback_value(callback, "error_description"),
            ));
        }
        let Some(code) =
            callback_value(callback, "code").filter(|code| !code.is_empty())
        else {
            return std::result::Result::Err(invalid_callback(&transaction.scheme));
        };
        let compiled = compiled_scheme(&transaction.scheme)?;
        let Some(grant) = compiled.authorization_code.as_ref() else {
            return std::result::Result::Err(unavailable(
                &transaction.scheme,
                AuthFlow::AuthorizationCode,
            ));
        };
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec::Vec::with_capacity(4);
        fields.push(("grant_type", "authorization_code"));
        fields.push(("code", code));
        fields.push(("code_verifier", transaction.code_verifier.as_str()));
        if let std::option::Option::Some(redirect) = &transaction.redirect_uri {
            fields.push(("redirect_uri", redirect.as_str()));
        }
        if let std::option::Option::Some(id) = &client_id {
            fields.push(("client_id", id.as_str()));
        }
        let authorization = self.authorization(compiled, AuthFlow::AuthorizationCode)?;
        let raw = self
            .post(
                client,
                compiled,
                AuthFlow::AuthorizationCode,
                grant.token,
                &fields,
                authorization,
            )
            .await?;
        let set = token_set(compiled, AuthFlow::AuthorizationCode, &raw)?;
        self.store.replace(compiled.name, set.clone()).await;
        std::result::Result::Ok(set)
    }

    /// Poll the device grant until it completes, pacing automatically (RFC
    /// 8628 section 3.5): `authorization_pending` waits the transaction's
    /// interval, `slow_down` grows the interval by five seconds cumulatively,
    /// and the transaction's declared expiry or the server's `expired_token`
    /// answer ends the loop with a typed
    /// [`AuthErrorKind::Expired`] failure. The first poll is immediate; every
    /// later attempt is paced through the injected `sleep`, which runs inline
    /// on the polling task's thread — pass `std::thread::sleep` (or a test
    /// double) or drive [`TokenSessions::poll_device_token`] on an executor
    /// timer instead.
    ///
    /// # Errors
    /// Unknown schemes, missing configuration or credentials, transport
    /// failures, unusable or rejected responses, and expiry.
    pub async fn poll_device_token_until_complete<T, F>(
        &self,
        client: &Client<T>,
        scheme: &str,
        transaction: &DeviceAuthorization,
        sleep: F,
    ) -> std::result::Result<TokenSet, SdkError>
    where
        T: Transport,
        F: Fn(std::time::Duration),
    {
        let compiled = compiled_scheme(scheme)?;
        let Some(grant) = compiled.device.as_ref() else {
            return std::result::Result::Err(unavailable(scheme, AuthFlow::DeviceToken));
        };
        let mut interval = transaction.interval_seconds;
        loop {
            if std::time::SystemTime::now()
                .duration_since(transaction.expires_at)
                .is_ok()
            {
                return std::result::Result::Err(device_expired(compiled));
            }
            match self
                .poll_device_step(client, compiled, grant, transaction)
                .await?
            {
                DevicePollStep::Complete(set) => {
                    return std::result::Result::Ok(set);
                }
                DevicePollStep::Pending => sleep(std::time::Duration::from_secs(interval)),
                DevicePollStep::SlowDown => {
                    interval = interval.saturating_add(5);
                    sleep(std::time::Duration::from_secs(interval));
                }
                DevicePollStep::Expired => {
                    return std::result::Result::Err(device_expired(compiled));
                }
            }
        }
    }

    /// One device poll: the request, the response decode and the RFC 8628
    /// error translation, shared by the single-poll function and the paced
    /// completion loop.
    async fn poll_device_step<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
        grant: &Grant,
        transaction: &DeviceAuthorization,
    ) -> std::result::Result<DevicePollStep, SdkError> {
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code",
            ),
            ("device_code", transaction.device_code.as_str()),
        ];
        if let std::option::Option::Some(id) = &client_id {
            fields.push(("client_id", id.as_str()));
        }
        let authorization = self.authorization(compiled, AuthFlow::DeviceToken)?;
        let raw = self
            .post(
                client,
                compiled,
                AuthFlow::DeviceToken,
                grant.token,
                &fields,
                authorization,
            )
            .await?;
        if (200..300).contains(&raw.status) {
            let set = token_set(compiled, AuthFlow::DeviceToken, &raw)?;
            self.store.replace(compiled.name, set.clone()).await;
            return std::result::Result::Ok(DevicePollStep::Complete(set));
        }
        let (server_error, _) = oauth_error(&raw.body);
        match server_error.as_deref() {
            std::option::Option::Some("authorization_pending") => {
                std::result::Result::Ok(DevicePollStep::Pending)
            }
            std::option::Option::Some("slow_down") => {
                std::result::Result::Ok(DevicePollStep::SlowDown)
            }
            std::option::Option::Some("expired_token") => {
                std::result::Result::Ok(DevicePollStep::Expired)
            }
            _ => std::result::Result::Err(rejection(
                compiled,
                AuthFlow::DeviceToken,
                &raw,
            )),
        }
    }
}
"#;

/// Entropy, hashing and comparison helpers shared by the authorization-code
/// and device sections, emitted after the form/base64 helpers.
const RUNTIME_HELPERS: &str = r#"/// RFC 4648 base64url without padding, for PKCE values and challenges:
/// 32 bytes encode to exactly 43 characters, every one an RFC 7636
/// unreserved byte.
fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        result.push(char::from(TABLE[usize::from(a >> 2)]));
        result.push(char::from(TABLE[usize::from((a & 3) << 4 | b >> 4)]));
        if part.len() > 1 {
            result.push(char::from(TABLE[usize::from((b & 15) << 2 | c >> 6)]));
        }
        if part.len() > 2 {
            result.push(char::from(TABLE[usize::from(c & 63)]));
        }
    }
    result
}

/// FIPS 180-4 SHA-256, dependency-free, used only for the PKCE S256 code
/// challenge: no hashing crate joins this package's dependency tree.
fn sha256(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
        0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
        0x1f83d9ab, 0x5be0cd19,
    ];
    let bits = u64::try_from(message.len())
        .ok()
        .and_then(|length| length.checked_mul(8))
        .expect("message length fits a SHA-256 input");
    let mut padded = std::vec::Vec::with_capacity(message.len() + 72);
    padded.extend_from_slice(message);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bits.to_be_bytes());
    for block in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (word, chunk) in w.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (word, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *word = word.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 32];
    for (word, chunk) in state.iter().zip(digest.chunks_exact_mut(4)) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Cryptographic entropy without dependencies: the operating system's kernel
/// CSPRNG. Unix targets read `/dev/urandom` through `std::fs`; other targets
/// have no dependency-free cryptographic randomness source here and are
/// refused with [`AuthErrorKind::UnsupportedPlatform`] rather than served
/// weak entropy.
#[cfg(unix)]
fn random_bytes(buffer: &mut [u8]) -> std::result::Result<(), SdkError> {
    use std::io::Read;
    let mut random = std::fs::File::open("/dev/urandom").map_err(|cause| {
        entropy_failure(
            AuthErrorKind::Entropy,
            std::option::Option::Some(std::boxed::Box::new(cause)),
        )
    })?;
    random.read_exact(buffer).map_err(|cause| {
        entropy_failure(
            AuthErrorKind::Entropy,
            std::option::Option::Some(std::boxed::Box::new(cause)),
        )
    })
}

/// The typed non-Unix refusal: weak entropy is never a substitute.
#[cfg(not(unix))]
fn random_bytes(_buffer: &mut [u8]) -> std::result::Result<(), SdkError> {
    std::result::Result::Err(entropy_failure(
        AuthErrorKind::UnsupportedPlatform,
        std::option::Option::None,
    ))
}

/// 32 bytes of kernel entropy in the RFC 7636 base64url alphabet: a
/// 43-character value of unreserved bytes, usable as the verifier or the
/// state.
fn random_value() -> std::result::Result<String, SdkError> {
    let mut bytes = [0u8; 32];
    random_bytes(&mut bytes)?;
    std::result::Result::Ok(base64url(&bytes))
}

/// Constant-time byte equality: every difference folds into one accumulator,
/// so the comparison cost does not depend on where a mismatch sits. Lengths
/// are not secret (both states are 43-character values), so unequal lengths
/// return early; `black_box` keeps the final comparison from being
/// short-circuited by value-dependent optimization.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        difference |= left ^ right;
    }
    std::hint::black_box(difference) == 0
}

/// The first value bound to `name` in one callback's query parameters.
fn callback_value<'a>(
    callback: &[(&'a str, &'a str)],
    name: &str,
) -> std::option::Option<&'a str> {
    callback
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

/// One lifecycle refusal before any compiled endpoint is touched: typed
/// metadata only, never token or credential values.
fn lifecycle_refusal(
    scheme: &str,
    kind: AuthErrorKind,
    server_error: std::option::Option<&str>,
    server_description: std::option::Option<&str>,
) -> SdkError {
    let mut error = crate::http::validation_error(
        OAUTH,
        OAUTH,
        "the authorization-code transaction was refused",
    );
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: scheme.to_owned(),
        flow: AuthFlow::AuthorizationCode,
        kind,
        server_error: server_error.map(str::to_owned),
        server_description: server_description.map(str::to_owned),
        cause: std::option::Option::None,
    }));
    error
}

fn transaction_used(scheme: &str) -> SdkError {
    lifecycle_refusal(scheme, AuthErrorKind::TransactionUsed, std::option::Option::None, std::option::Option::None)
}

fn state_mismatch(scheme: &str) -> SdkError {
    lifecycle_refusal(scheme, AuthErrorKind::StateMismatch, std::option::Option::None, std::option::Option::None)
}

fn invalid_callback(scheme: &str) -> SdkError {
    lifecycle_refusal(scheme, AuthErrorKind::InvalidCallback, std::option::Option::None, std::option::Option::None)
}

fn authorization_denied(
    scheme: &str,
    code: &str,
    description: std::option::Option<&str>,
) -> SdkError {
    lifecycle_refusal(
        scheme,
        AuthErrorKind::AuthorizationDenied,
        std::option::Option::Some(code),
        description,
    )
}

fn entropy_failure(kind: AuthErrorKind, cause: std::option::Option<BoxError>) -> SdkError {
    let mut error = crate::http::validation_error(
        OAUTH,
        OAUTH,
        match kind {
            AuthErrorKind::UnsupportedPlatform => {
                "this platform has no dependency-free cryptographic randomness source; refusing to mint weak PKCE material"
            }
            _ => "the entropy source failed",
        },
    );
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: std::string::String::new(),
        flow: AuthFlow::AuthorizationCode,
        kind,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause,
    }));
    error
}

fn device_expired(compiled: &'static Scheme) -> SdkError {
    let mut error = crate::http::validation_error(
        compiled.source,
        compiled.source,
        "the device grant transaction expired",
    );
    error.kind = SdkErrorKind::UnexpectedResponse;
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: compiled.name.to_owned(),
        flow: AuthFlow::DeviceToken,
        kind: AuthErrorKind::Expired,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause: std::option::Option::None,
    }));
    error
}
"#;

fn push_helpers(module: &mut String) {
    module.push_str(
        "fn compiled_scheme(scheme: &str) -> std::result::Result<&'static Scheme, SdkError> {\n    SCHEMES\n        .iter()\n        .find(|compiled| compiled.name == scheme)\n        .ok_or_else(|| unknown(scheme))\n}\n\n/// Whether a stored set still outlives its compiled clock skew.\nfn fresh(set: &TokenSet, skew_seconds: u64) -> bool {\n    match std::time::SystemTime::now()\n        .checked_add(std::time::Duration::from_secs(skew_seconds))\n    {\n        std::option::Option::Some(deadline) => {\n            set.expires_at.duration_since(deadline).is_ok()\n        }\n        std::option::Option::None => true,\n    }\n}\n\n/// Whether an error carries the named server rejection code.\nfn rejected(error: &SdkError, code: &str) -> bool {\n    error.cause.as_ref().is_some_and(|cause| {\n        cause\n            .downcast_ref::<AuthError>()\n            .is_some_and(|auth| {\n                auth.kind == AuthErrorKind::ServerRejected\n                    && auth.server_error.as_deref() == std::option::Option::Some(code)\n            })\n    })\n}\n\nfn unknown(scheme: &str) -> SdkError {\n    let mut error = crate::http::validation_error(\n        OAUTH,\n        OAUTH,\n        \"the scheme name is not compiled into this package\",\n    );\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: scheme.to_owned(),\n        flow: AuthFlow::ClientCredentials,\n        kind: AuthErrorKind::UnknownScheme,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause: std::option::Option::None,\n    }));\n    error\n}\n\nfn unavailable(scheme: &str, flow: AuthFlow) -> SdkError {\n    let mut error = crate::http::validation_error(\n        OAUTH,\n        OAUTH,\n        \"the compiled OAuth plan has no executable endpoint for this step\",\n    );\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: scheme.to_owned(),\n        flow,\n        kind: AuthErrorKind::Unavailable,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause: std::option::Option::None,\n    }));\n    error\n}\n\nfn missing_refresh(compiled: &'static Scheme) -> SdkError {\n    let mut error = crate::http::validation_error(\n        compiled.source,\n        compiled.source,\n        \"the token set carries no refresh token to exchange\",\n    );\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: compiled.name.to_owned(),\n        flow: AuthFlow::Refresh,\n        kind: AuthErrorKind::MissingRefreshToken,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause: std::option::Option::None,\n    }));\n    error\n}\n\nfn missing_credentials(\n    compiled: &'static Scheme,\n    flow: AuthFlow,\n    env: std::option::Option<&'static str>,\n) -> SdkError {\n    let message = match env {\n        std::option::Option::Some(name) => format!(\n            \"the compiled client credential variable {name:?} is unset or empty\"\n        ),\n        std::option::Option::None => {\n            \"the compiled plan carries no client credential variable and none were supplied\"\n                .to_owned()\n        }\n    };\n    let mut error = crate::http::validation_error(compiled.source, compiled.source, message);\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: compiled.name.to_owned(),\n        flow,\n        kind: AuthErrorKind::MissingClientCredentials,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause: std::option::Option::None,\n    }));\n    error\n}\n\nfn invalid_response(\n    compiled: &'static Scheme,\n    flow: AuthFlow,\n    cause: std::option::Option<BoxError>,\n) -> SdkError {\n    let mut error = crate::http::validation_error(\n        compiled.source,\n        compiled.source,\n        \"the response is not a usable OAuth document\",\n    );\n    error.kind = SdkErrorKind::UnexpectedResponse;\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: compiled.name.to_owned(),\n        flow,\n        kind: AuthErrorKind::InvalidResponse,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause,\n    }));\n    error\n}\n\nfn transport_failure(error: SdkError, compiled: &'static Scheme, flow: AuthFlow) -> SdkError {\n    let mut error = error;\n    let cause = error.cause.take();\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: compiled.name.to_owned(),\n        flow,\n        kind: AuthErrorKind::Transport,\n        server_error: std::option::Option::None,\n        server_description: std::option::Option::None,\n        cause,\n    }));\n    error\n}\n\nfn rejection(compiled: &'static Scheme, flow: AuthFlow, raw: &RawResponse) -> SdkError {\n    let mut error = crate::http::validation_error(\n        compiled.source,\n        compiled.source,\n        \"the OAuth endpoint rejected the request\",\n    );\n    error.kind = SdkErrorKind::UnexpectedResponse;\n    error.status = std::option::Option::Some(raw.status);\n    let capture = raw.body.len().min(CAPTURE_BYTES);\n    error.raw_capture = raw.body[..capture].to_vec();\n    error.truncated = raw.body.len() > capture;\n    let (server_error, server_description) = oauth_error(&raw.body);\n    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {\n        scheme: compiled.name.to_owned(),\n        flow,\n        kind: AuthErrorKind::ServerRejected,\n        server_error,\n        server_description,\n        cause: std::option::Option::None,\n    }));\n    error\n}\n\n/// Decode a rejected response's OAuth error code and description. Issued\n/// tokens never reach this path, and successful bodies are never captured.\nfn oauth_error(body: &[u8]) -> (std::option::Option<std::string::String>, std::option::Option<std::string::String>) {\n    let value = match crate::parse_json_bytes(body, TOKEN_JSON_LIMITS) {\n        std::result::Result::Ok(value) => value,\n        std::result::Result::Err(_) => {\n            return (std::option::Option::None, std::option::Option::None);\n        }\n    };\n    let object = match &value {\n        Nullable::Value(JsonNonNullValue::Object(map)) => map,\n        _ => return (std::option::Option::None, std::option::Option::None),\n    };\n    let text = |name: &str| {\n        object.get(name).and_then(|value| match value {\n            Nullable::Value(JsonNonNullValue::String(text)) if !text.is_empty() => {\n                std::option::Option::Some(text.clone())\n            }\n            _ => std::option::Option::None,\n        })\n    };\n    (text(\"error\"), text(\"error_description\"))\n}\n\n/// Decode one RFC 6749 token response into a token set.\nfn token_set(\n    compiled: &'static Scheme,\n    flow: AuthFlow,\n    raw: &RawResponse,\n) -> std::result::Result<TokenSet, SdkError> {\n    if !(200..300).contains(&raw.status) {\n        return std::result::Result::Err(rejection(compiled, flow, raw));\n    }\n    let value = crate::parse_json_bytes(&raw.body, TOKEN_JSON_LIMITS).map_err(|cause| {\n        invalid_response(\n            compiled,\n            flow,\n            std::option::Option::Some(std::boxed::Box::new(cause)),\n        )\n    })?;\n    let object = match &value {\n        Nullable::Value(JsonNonNullValue::Object(map)) => map,\n        _ => return std::result::Result::Err(invalid_response(compiled, flow, std::option::Option::None)),\n    };\n    let text = |name: &str| {\n        object.get(name).and_then(|value| match value {\n            Nullable::Value(JsonNonNullValue::String(text)) if !text.is_empty() => {\n                std::option::Option::Some(text.clone())\n            }\n            _ => std::option::Option::None,\n        })\n    };\n    let Some(access_token) = text(\"access_token\") else {\n        return std::result::Result::Err(invalid_response(\n            compiled,\n            flow,\n            std::option::Option::None,\n        ));\n    };\n    let token_type =\n        text(\"token_type\").unwrap_or_else(|| \"bearer\".to_owned());\n    let expires_in = object.get(\"expires_in\").and_then(|value| match value {\n        Nullable::Value(JsonNonNullValue::Number(token)) => token\n            .as_str()\n            .parse::<JsonInteger>()\n            .ok()\n            .and_then(|token| token.to_u128()),\n        _ => std::option::Option::None,\n    });\n    let now = std::time::SystemTime::now();\n    let expires_at = expires_in\n        .and_then(|seconds| u64::try_from(seconds).ok())\n        .and_then(|seconds| {\n            now.checked_add(std::time::Duration::from_secs(seconds))\n        })\n        .unwrap_or_else(|| {\n            now.checked_add(std::time::Duration::from_secs(365 * 24 * 60 * 60))\n                .expect(\"one year fits a SystemTime offset\")\n        });\n    std::result::Result::Ok(TokenSet {\n        access_token,\n        token_type,\n        expires_at,\n        refresh_token: text(\"refresh_token\"),\n        scope: text(\"scope\"),\n        issuer_account: text(\"iss\"),\n    })\n}\n\n/// Append one RFC 6749 form field (`application/x-www-form-urlencoded`).\nfn form_pair(body: &mut String, name: &str, value: &str) {\n    form_component_into(body, name);\n    body.push('=');\n    form_component_into(body, value);\n}\n\nfn form_component(value: &str) -> String {\n    let mut out = std::string::String::new();\n    form_component_into(&mut out, value);\n    out\n}\n\nfn form_component_into(out: &mut String, value: &str) {\n    for byte in value.as_bytes() {\n        match byte {\n            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {\n                out.push(char::from(*byte))\n            }\n            b' ' => out.push('+'),\n            _ => {\n                out.push('%');\n                out.push(char::from(HEX[usize::from(byte >> 4)]));\n                out.push(char::from(HEX[usize::from(byte & 15)]));\n            }\n        }\n    }\n}\n\nconst HEX: &[u8; 16] = b\"0123456789ABCDEF\";\n\n/// RFC 4648 standard base64 with padding, for `client_secret_basic` only.\nfn base64(bytes: &[u8]) -> String {\n    const TABLE: &[u8; 64] = b\"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/\";\n    let mut result = String::new();\n    for part in bytes.chunks(3) {\n        let a = part[0];\n        let b = part.get(1).copied().unwrap_or(0);\n        let c = part.get(2).copied().unwrap_or(0);\n        result.push(char::from(TABLE[usize::from(a >> 2)]));\n        result.push(char::from(TABLE[usize::from((a & 3) << 4 | b >> 4)]));\n        result.push(if part.len() > 1 {\n            char::from(TABLE[usize::from((b & 15) << 2 | c >> 6)])\n        } else {\n            '='\n        });\n        result.push(if part.len() > 2 {\n            char::from(TABLE[usize::from(c & 63)])\n        } else {\n            '='\n        });\n    }\n    result\n}\n",
    );
    module.push_str(RUNTIME_HELPERS);
}

/// Client-impl methods appended to the emitted root. The compiled store stays
/// explicit on the token sessions value; the client methods delegate to it.
fn client_methods() -> String {
    String::from(
        "#[cfg(feature=\"http\")]\nimpl<T: http::Transport> Client<T> {\n    /// Acquire (or reuse) a client-credentials token through `sessions`. See [`oauth::TokenSessions::client_credentials_token`].\n    pub async fn client_credentials_token<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str) -> std::result::Result<oauth::TokenSet, http::SdkError> { sessions.client_credentials_token(self, scheme).await }\n    /// Exchange a token set's refresh token through `sessions`. See [`oauth::TokenSessions::refresh_token`].\n    pub async fn refresh_token<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, set: &oauth::TokenSet) -> std::result::Result<oauth::TokenSet, http::SdkError> { sessions.refresh_token(self, scheme, set).await }\n    /// Revoke a token set through `sessions`. See [`oauth::TokenSessions::revoke_token`].\n    pub async fn revoke_token<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, set: &oauth::TokenSet) -> std::result::Result<(), http::SdkError> { sessions.revoke_token(self, scheme, set).await }\n    /// Introspect a token set through `sessions`. See [`oauth::TokenSessions::introspect_token`].\n    pub async fn introspect_token<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, set: &oauth::TokenSet) -> std::result::Result<crate::JsonValue, http::SdkError> { sessions.introspect_token(self, scheme, set).await }\n    /// Start the device grant through `sessions`. See [`oauth::TokenSessions::begin_device_authorization`].\n    pub async fn begin_device_authorization<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str) -> std::result::Result<oauth::DeviceAuthorization, http::SdkError> { sessions.begin_device_authorization(self, scheme).await }\n    /// Poll the device grant through `sessions`. See [`oauth::TokenSessions::poll_device_token`].\n    pub async fn poll_device_token<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, transaction: &oauth::DeviceAuthorization) -> std::result::Result<oauth::DevicePoll, http::SdkError> { sessions.poll_device_token(self, scheme, transaction).await }\n    /// Start an authorization-code transaction through `sessions`. See [`oauth::TokenSessions::begin_authorization`].\n    pub fn begin_authorization<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, redirect_uri: std::option::Option<&str>, scopes: &[&str]) -> std::result::Result<oauth::AuthorizationTransaction, http::SdkError> { sessions.begin_authorization(scheme, redirect_uri, scopes) }\n    /// Complete an authorization-code transaction through `sessions`. See [`oauth::TokenSessions::complete_authorization`].\n    pub async fn complete_authorization<S: oauth::TokenStore>(&self, sessions: &oauth::TokenSessions<S>, transaction: &oauth::AuthorizationTransaction, callback: &[(&str, &str)]) -> std::result::Result<oauth::TokenSet, http::SdkError> { sessions.complete_authorization(self, transaction, callback).await }\n    /// Poll the device grant to completion through `sessions`. See [`oauth::TokenSessions::poll_device_token_until_complete`].\n    pub async fn poll_device_token_until_complete<S: oauth::TokenStore, F: Fn(std::time::Duration)>(&self, sessions: &oauth::TokenSessions<S>, scheme: &str, transaction: &oauth::DeviceAuthorization, sleep: F) -> std::result::Result<oauth::TokenSet, http::SdkError> { sessions.poll_device_token_until_complete(self, scheme, transaction, sleep).await }\n}\n",
    )
}

/// The discovery-aware client-credentials acquisition: the compiled grant's
/// token endpoint always wins; otherwise the sessions value's cached discovery
/// document.
const METHODS_CC_DISCOVERY: &str = r#"impl<S: TokenStore> TokenSessions<S> {
    /// Acquire a client-credentials token for the compiled source scheme
    /// `scheme`. A stored set still inside its compiled clock skew is returned
    /// without a request; an absent or expired set is acquired once per
    /// single-flight round and atomically replaced in the store. Only the
    /// `grant_type` travels on the wire: no scope is requested and the server
    /// issues its default.
    ///
    /// Endpoint resolution follows the compiled precedence: the compiled
    /// client-credentials flow's token endpoint always wins; otherwise, when
    /// the scheme compiles a discovery URL, the discovery document's
    /// `token_endpoint` resolves the acquisition (fetched once per scheme,
    /// single-flighted, cached for the sessions value's lifetime, and retried
    /// on the next call after a failure); otherwise the compiled-only refusal
    /// stands.
    ///
    /// # Errors
    /// Unknown scheme names, missing configuration or client credentials,
    /// discovery failures, transport failures, and unusable or rejected token
    /// responses.
    pub async fn client_credentials_token<T: Transport>(
        &self,
        client: &Client<T>,
        scheme: &str,
    ) -> std::result::Result<TokenSet, SdkError> {
        let compiled = compiled_scheme(scheme)?;
        if compiled.client_credentials.is_none() {
            if compiled.discovery_url.is_none() {
                return std::result::Result::Err(unavailable(
                    scheme,
                    AuthFlow::ClientCredentials,
                ));
            }
            // The discovery document defines this scheme's token endpoint:
            // resolve (and cache) it before the acquisition round so a failed
            // discovery never enters one.
            self.discovery_document(client, compiled).await?;
        }
        loop {
            if let std::option::Option::Some(set) = self.store.load(compiled.name).await
                && fresh(&set, compiled.skew_seconds)
            {
                return std::result::Result::Ok(set);
            }
            match self.enter(compiled.name) {
                Entered::Wait(round) => {
                    // The winning round completes its store replacement before
                    // releasing the waiters; re-probe the store, then either
                    // return the fresh set or start a new round.
                    round.wait();
                }
                Entered::Proceed(round) => {
                    let result = match compiled.client_credentials.as_ref() {
                        std::option::Option::Some(grant) => {
                            self.acquire(client, compiled, grant).await
                        }
                        std::option::Option::None => {
                            self.acquire_discovered(client, compiled).await
                        }
                    };
                    round.complete();
                    return result;
                }
            }
        }
    }
}
"#;

/// The discovery-aware explicit refresh: the compiled refresh URL, else the
/// compiled flow's token URL, always wins; otherwise the cached discovery
/// document's token endpoint.
const METHODS_REFRESH_DISCOVERY: &str = r#"impl<S: TokenStore> TokenSessions<S> {
    /// Exchange `set`'s refresh token at the resolved refresh endpoint: the
    /// compiled refresh URL, else the compiled flow's token URL, always win;
    /// a scheme whose compiled plan declares neither (an OpenID Connect
    /// scheme whose endpoints the discovery document defines) refreshes
    /// through the discovery document's `token_endpoint`, cached for the
    /// sessions value's lifetime. A rotated refresh token is adopted;
    /// otherwise the previous one is retained. An `invalid_grant` rejection
    /// clears the stored set for the scheme.
    ///
    /// # Errors
    /// Unknown schemes, missing configuration or credentials, discovery
    /// failures, transport failures, and unusable or rejected token responses.
    pub async fn refresh_token<T: Transport>(
        &self,
        client: &Client<T>,
        scheme: &str,
        set: &TokenSet,
    ) -> std::result::Result<TokenSet, SdkError> {
        let compiled = compiled_scheme(scheme)?;
        let discovered_only = compiled.client_credentials.is_none()
            && compiled.authorization_code.is_none()
            && compiled.device.is_none();
        if discovered_only && compiled.discovery_url.is_none() {
            return std::result::Result::Err(unavailable(scheme, AuthFlow::Refresh));
        }
        let Some(refresh_token) = set.refresh_token.as_deref() else {
            return std::result::Result::Err(missing_refresh(compiled));
        };
        let operation = match compiled
            .client_credentials
            .as_ref()
            .or(compiled.authorization_code.as_ref())
            .or(compiled.device.as_ref())
        {
            std::option::Option::Some(grant) => grant.refresh.unwrap_or(grant.token),
            std::option::Option::None => {
                self.discovered_token_operation(client, compiled).await?
            }
        };
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];
        if let std::option::Option::Some(id) = &client_id {
            fields.push(("client_id", id.as_str()));
        }
        let authorization = self.authorization(compiled, AuthFlow::Refresh)?;
        let raw = self
            .post(client, compiled, AuthFlow::Refresh, operation, &fields, authorization)
            .await?;
        let mut refreshed = match token_set(compiled, AuthFlow::Refresh, &raw) {
            std::result::Result::Ok(set) => set,
            std::result::Result::Err(error) => {
                if rejected(&error, "invalid_grant") {
                    self.store.clear(compiled.name).await;
                }
                return std::result::Result::Err(error);
            }
        };
        if refreshed.refresh_token.is_none() {
            refreshed.refresh_token = set.refresh_token.clone();
        }
        self.store
            .replace(compiled.name, refreshed.clone())
            .await;
        std::result::Result::Ok(refreshed)
    }
}
"#;

/// The discovery-aware revocation: the compiled endpoint always wins;
/// otherwise the cached discovery document's `revocation_endpoint`.
const METHODS_REVOKE_DISCOVERY: &str = r#"impl<S: TokenStore> TokenSessions<S> {
    /// Revoke `set`'s access token at the resolved revocation endpoint and
    /// clear the stored set for the scheme. Endpoint resolution follows the
    /// compiled precedence: the configured revocation endpoint always wins;
    /// a scheme without one resolves the discovery document's
    /// `revocation_endpoint` (cached for the sessions value's lifetime).
    ///
    /// # Errors
    /// Unknown schemes, unconfigured and undiscovered endpoints, missing
    /// client credentials, discovery failures, transport failures, and
    /// rejected revocations.
    pub async fn revoke_token<T: Transport>(
        &self,
        client: &Client<T>,
        scheme: &str,
        set: &TokenSet,
    ) -> std::result::Result<(), SdkError> {
        let compiled = compiled_scheme(scheme)?;
        let operation = match compiled.revocation {
            std::option::Option::Some(operation) => operation,
            std::option::Option::None => {
                if compiled.discovery_url.is_none() {
                    return std::result::Result::Err(unavailable(
                        scheme,
                        AuthFlow::Revocation,
                    ));
                }
                self.discovered_revocation_operation(client, compiled)
                    .await?
            }
        };
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec![
            ("token", set.access_token.as_str()),
            ("token_type_hint", "access_token"),
        ];
        if let std::option::Option::Some(id) = &client_id {
            fields.push(("client_id", id.as_str()));
        }
        let authorization = self.authorization(compiled, AuthFlow::Revocation)?;
        let raw = self
            .post(
                client,
                compiled,
                AuthFlow::Revocation,
                operation,
                &fields,
                authorization,
            )
            .await?;
        if !(200..300).contains(&raw.status) {
            return std::result::Result::Err(rejection(
                compiled,
                AuthFlow::Revocation,
                &raw,
            ));
        }
        self.store.clear(compiled.name).await;
        std::result::Result::Ok(())
    }
}
"#;

/// The discovery-aware introspection: the compiled endpoint always wins;
/// otherwise the cached discovery document's `introspection_endpoint`.
const METHODS_INTROSPECT_DISCOVERY: &str = r#"impl<S: TokenStore> TokenSessions<S> {
    /// Introspect `set`'s access token at the resolved introspection endpoint,
    /// returning the RFC 7662 document as an exact JSON value. Endpoint
    /// resolution follows the compiled precedence: the configured
    /// introspection endpoint always wins; a scheme without one resolves the
    /// discovery document's `introspection_endpoint` (cached for the sessions
    /// value's lifetime).
    ///
    /// # Errors
    /// Unknown schemes, unconfigured and undiscovered endpoints, missing
    /// client credentials, discovery failures, transport failures, and
    /// rejected or unusable responses.
    pub async fn introspect_token<T: Transport>(
        &self,
        client: &Client<T>,
        scheme: &str,
        set: &TokenSet,
    ) -> std::result::Result<JsonValue, SdkError> {
        let compiled = compiled_scheme(scheme)?;
        let operation = match compiled.introspection {
            std::option::Option::Some(operation) => operation,
            std::option::Option::None => {
                if compiled.discovery_url.is_none() {
                    return std::result::Result::Err(unavailable(
                        scheme,
                        AuthFlow::Introspection,
                    ));
                }
                self.discovered_introspection_operation(client, compiled)
                    .await?
            }
        };
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec![("token", set.access_token.as_str())];
        if let std::option::Option::Some(id) = &client_id {
            fields.push(("client_id", id.as_str()));
        }
        let authorization = self.authorization(compiled, AuthFlow::Introspection)?;
        let raw = self
            .post(
                client,
                compiled,
                AuthFlow::Introspection,
                operation,
                &fields,
                authorization,
            )
            .await?;
        if !(200..300).contains(&raw.status) {
            return std::result::Result::Err(rejection(
                compiled,
                AuthFlow::Introspection,
                &raw,
            ));
        }
        crate::parse_json_bytes(&raw.body, TOKEN_JSON_LIMITS).map_err(|cause| {
            invalid_response(
                compiled,
                AuthFlow::Introspection,
                std::option::Option::Some(std::boxed::Box::new(cause)),
            )
        })
    }
}
"#;

/// The RFC 8414 / OpenID Connect discovery section, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-sessions cache with single-flight, the
/// synthetic endpoint operations and the typed failure helpers.
fn discovery_section(plan: &HttpPlan) -> String {
    let config = &plan.config;
    format!(
        r#"/// The compiled ceiling for one discovery document response (about a
/// mebibyte).
const DISCOVERY_MAX_BYTES: usize = 1 << 20;
/// Exact limits for discovery requests: the response ceiling bounds one
/// discovery document; the remaining ceilings mirror the compiled plan.
const DISCOVERY_LIMITS: crate::http::Limits = crate::http::Limits {{
    request: {},
    response: DISCOVERY_MAX_BYTES,
    part: {},
    item: {},
    chunk: {},
    header: {},
}};

/// One RFC 8414 / OpenID Connect discovery document reduced to the endpoints
/// this runtime resolves. Unknown members are ignored; a present member must
/// be an absolute http(s) URL under the compiled endpoint rules.
#[derive(Debug, Default)]
struct DiscoveredEndpoints {{
    token_endpoint: std::option::Option<std::string::String>,
    revocation_endpoint: std::option::Option<std::string::String>,
    introspection_endpoint: std::option::Option<std::string::String>,
}}

/// One cached discovery result: the endpoints plus the synthetic form-POST
/// operations for the discovered endpoint URLs, built once per fetch.
#[derive(Debug)]
struct Discovered {{
    token: std::option::Option<&'static Operation>,
    revocation: std::option::Option<&'static Operation>,
    introspection: std::option::Option<&'static Operation>,
}}

/// The per-sessions discovery cache. Successful documents are cached per
/// scheme for the sessions value's lifetime, so repeated calls never re-fetch;
/// failed fetches are never cached, so the next call retries. Single-flight
/// per scheme reuses the acquisition round mechanism under the scheme's
/// discovery round key. Never a process-wide cache.
#[derive(Default)]
struct DiscoveryCache {{
    documents:
        std::sync::Mutex<std::collections::BTreeMap<&'static str, std::sync::Arc<Discovered>>>,
}}
impl DiscoveryCache {{
    fn load(&self, scheme: &str) -> std::option::Option<std::sync::Arc<Discovered>> {{
        self.documents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(scheme)
            .cloned()
    }}
    fn store(&self, scheme: &'static str, discovered: std::sync::Arc<Discovered>) {{
        self.documents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(scheme, discovered);
    }}
}}

/// The discovery URL's origin (scheme, host and the port with the scheme
/// default made explicit), or `None` when the value is not an absolute
/// http(s) URL.
fn discovery_origin(url: &str) -> std::option::Option<std::string::String> {{
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {{
        return std::option::Option::None;
    }}
    let host = parsed.host()?.to_string();
    let port = match parsed.port() {{
        std::option::Option::Some(port) => port,
        std::option::Option::None if parsed.scheme() == "http" => 80,
        std::option::Option::None => 443,
    }};
    std::option::Option::Some(format!("{{}}://{{host}}:{{port}}", parsed.scheme()))
}}

/// Whether one path segment would be normalized away by the URL stack: such
/// endpoints are refused instead of approximated.
fn discovery_dot_segment(value: &str) -> bool {{
    matches!(
        value.to_ascii_lowercase().as_str(),
        "." | ".." | "%2e" | ".%2e" | "%2e." | "%2e%2e"
    )
}}

/// Splits an absolute http(s) URL into its origin server template and path
/// under the compiled endpoint rules: no query, fragment or userinfo, no dot
/// segments and only complete percent escapes. Endpoints outside these rules
/// are a typed discovery failure, never approximated.
fn discovery_split(url: &str) -> std::option::Option<(std::string::String, std::string::String)> {{
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {{
        return std::option::Option::None;
    }}
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {{
        return std::option::Option::None;
    }}
    let path = parsed.path().to_owned();
    if path.contains(['{{', '}}', '\\'])
        || path.split('/').any(discovery_dot_segment)
        || !percent_triples(&path)
    {{
        return std::option::Option::None;
    }}
    let host = parsed.host()?.to_string();
    let origin = match parsed.port() {{
        std::option::Option::Some(port) => format!("{{}}://{{host}}:{{port}}", parsed.scheme()),
        std::option::Option::None => format!("{{}}://{{host}}", parsed.scheme()),
    }};
    std::option::Option::Some((origin, path))
}}

fn percent_triples(value: &str) -> bool {{
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {{
        if bytes[at] == b'%' {{
            if !bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
            {{
                return false;
            }}
            at += 3;
        }} else {{
            at += 1;
        }}
    }}
    true
}}

/// One typed discovery failure: safe metadata only, never raw body text.
fn discovery_failure(compiled: &'static Scheme, flow: AuthFlow, message: &str) -> SdkError {{
    let mut error = crate::http::validation_error(compiled.source, compiled.source, message);
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
        scheme: compiled.name.to_owned(),
        flow,
        kind: AuthErrorKind::DiscoveryFailed,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause: std::option::Option::None,
    }}));
    error
}}

/// One synthetic form-POST operation for a discovered endpoint URL. The
/// operation leaks: discovery results are cached per sessions value, so the
/// count of leaked operations is bounded by the compiled discovery URLs and
/// their declared endpoints.
fn discovered_form_operation(
    compiled: &'static Scheme,
    operation_id: &'static str,
    url: &str,
    flow: AuthFlow,
) -> std::result::Result<&'static Operation, SdkError> {{
    let Some((origin, path)) = discovery_split(url) else {{
        return std::result::Result::Err(discovery_failure(
            compiled,
            flow,
            "the discovery document carries an endpoint the compiled request rules cannot send",
        ));
    }};
    let path: &'static str = std::boxed::Box::leak(path.into_boxed_str());
    let origin: &'static str = std::boxed::Box::leak(origin.into_boxed_str());
    let operation = Operation {{
        operation_id,
        source: compiled.source,
        provenance: crate::http::Provenance {{
            use_site: compiled.source,
            terminal: compiled.source,
            references: &[],
            use_site_resource: std::option::Option::None,
            terminal_resource: std::option::Option::None,
            reference_resources: &[],
        }},
        method: "POST",
        path_template: path,
        servers: std::boxed::Box::leak(
            std::vec![crate::http::Server {{
                source: compiled.source,
                document_base: compiled.source,
                provenance: std::option::Option::None,
                template: origin,
                name: std::option::Option::None,
                description: std::option::Option::None,
                variables: &[],
            }}]
            .into_boxed_slice(),
        ),
        security: crate::http::Security::NoAuth(compiled.source),
        parameters: std::boxed::Box::leak(
            std::vec![crate::http::Parameter {{
                name: "authorization",
                source: compiled.source,
                location: crate::http::ParameterLocation::Header,
                required: true,
                serialization: crate::http::Serialization::Style {{
                    style: crate::http::Style::Simple,
                    explode: false,
                    shape: crate::http::Shape::Scalar(crate::http::ScalarType::String),
                    encoding: crate::http::PercentEncoding::None,
                }},
            }}]
            .into_boxed_slice(),
        ),
        request_media: std::boxed::Box::leak(
            std::vec![crate::http::Media {{
                source: compiled.source,
                declared: "application/x-www-form-urlencoded",
                range: crate::http::MediaRange::Concrete("application", "x-www-form-urlencoded"),
                parameters: &[],
                kind: crate::http::MediaKind::Form,
            }}]
            .into_boxed_slice(),
        ),
        responses: &[],
        accept: "",
        limits: DISCOVERY_LIMITS,
        json_limits: TOKEN_JSON_LIMITS,
    }};
    std::result::Result::Ok(std::boxed::Box::leak(std::boxed::Box::new(operation)))
}}

impl<S: TokenStore> TokenSessions<S> {{
    /// Fetch the scheme's discovery document (GET, `accept: application/json`,
    /// bounded at one mebibyte, through the caller's transport), returning the
    /// cached result for the sessions value. Single-flight per scheme reuses
    /// the acquisition round mechanism under the scheme's discovery round key,
    /// so concurrent callers share one fetch; failed fetches are never cached,
    /// so the next call retries.
    ///
    /// # Errors
    /// Schemes without a compiled discovery URL, discovery transport
    /// failures, and documents that fail the typed decode or the issuer rule.
    async fn discovery_document<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<std::sync::Arc<Discovered>, SdkError> {{
        loop {{
            if let std::option::Option::Some(found) = self.discovery.load(compiled.name) {{
                return std::result::Result::Ok(found);
            }}
            match self.enter(compiled.discovery_round) {{
                Entered::Wait(round) => {{
                    // The winning round stores its result (or leaves the
                    // cache empty after a failure) before releasing the
                    // waiters.
                    round.wait();
                }}
                Entered::Proceed(round) => {{
                    let result = self.discovery_fetch(client, compiled).await;
                    if let std::result::Result::Ok(discovered) = &result {{
                        self.discovery
                            .store(compiled.name, std::sync::Arc::clone(discovered));
                    }}
                    round.complete();
                    return result;
                }}
            }}
        }}
    }}

    async fn discovery_fetch<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<std::sync::Arc<Discovered>, SdkError> {{
        let Some(url) = compiled.discovery_url else {{
            return std::result::Result::Err(discovery_failure(
                compiled,
                AuthFlow::ClientCredentials,
                "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery",
            ));
        }};
        let Some((origin, path)) = discovery_split(url) else {{
            return std::result::Result::Err(discovery_failure(
                compiled,
                AuthFlow::ClientCredentials,
                "the compiled discovery URL cannot compile into a request",
            ));
        }};
        let path: &'static str = std::boxed::Box::leak(path.into_boxed_str());
        let origin: &'static str = std::boxed::Box::leak(origin.into_boxed_str());
        let operation: &'static Operation = std::boxed::Box::leak(std::boxed::Box::new(Operation {{
            operation_id: "oauth: discovery document",
            source: compiled.source,
            provenance: crate::http::Provenance {{
                use_site: compiled.source,
                terminal: compiled.source,
                references: &[],
                use_site_resource: std::option::Option::None,
                terminal_resource: std::option::Option::None,
                reference_resources: &[],
            }},
            method: "GET",
            path_template: path,
            servers: std::boxed::Box::leak(
                std::vec![crate::http::Server {{
                    source: compiled.source,
                    document_base: compiled.source,
                    provenance: std::option::Option::None,
                    template: origin,
                    name: std::option::Option::None,
                    description: std::option::Option::None,
                    variables: &[],
                }}]
                .into_boxed_slice(),
            ),
            security: crate::http::Security::NoAuth(compiled.source),
            parameters: &[],
            request_media: &[],
            responses: &[],
            accept: "application/json",
            limits: DISCOVERY_LIMITS,
            json_limits: TOKEN_JSON_LIMITS,
        }}));
        let raw = client
            .send(operation, &[], std::option::Option::None)
            .await
            .map_err(|cause| {{
                let mut error = crate::http::validation_error(
                    compiled.source,
                    compiled.source,
                    "the discovery document request failed before a response arrived",
                );
                error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                    scheme: compiled.name.to_owned(),
                    flow: AuthFlow::ClientCredentials,
                    kind: AuthErrorKind::DiscoveryFailed,
                    server_error: std::option::Option::None,
                    server_description: std::option::Option::None,
                    cause: std::option::Option::Some(std::boxed::Box::new(cause)),
                }}));
                error
            }})?;
        if !(200..300).contains(&raw.status) {{
            let mut error = crate::http::validation_error(
                compiled.source,
                compiled.source,
                "the discovery document request was rejected",
            );
            error.kind = SdkErrorKind::UnexpectedResponse;
            error.status = std::option::Option::Some(raw.status);
            let capture = raw.body.len().min(CAPTURE_BYTES);
            error.raw_capture = raw.body[..capture].to_vec();
            error.truncated = raw.body.len() > capture;
            error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                scheme: compiled.name.to_owned(),
                flow: AuthFlow::ClientCredentials,
                kind: AuthErrorKind::DiscoveryFailed,
                server_error: std::option::Option::None,
                server_description: std::option::Option::None,
                cause: std::option::Option::None,
            }}));
            return std::result::Result::Err(error);
        }}
        let value = crate::parse_json_bytes(&raw.body, TOKEN_JSON_LIMITS).map_err(|cause| {{
            let mut error = crate::http::validation_error(
                compiled.source,
                compiled.source,
                "the discovery document is not readable JSON",
            );
            error.kind = SdkErrorKind::UnexpectedResponse;
            error.status = std::option::Option::Some(raw.status);
            error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                scheme: compiled.name.to_owned(),
                flow: AuthFlow::ClientCredentials,
                kind: AuthErrorKind::DiscoveryFailed,
                server_error: std::option::Option::None,
                server_description: std::option::Option::None,
                cause: std::option::Option::Some(std::boxed::Box::new(cause)),
            }}));
            error
        }})?;
        let object = match &value {{
            Nullable::Value(JsonNonNullValue::Object(map)) => map,
            _ => {{
                return std::result::Result::Err(discovery_failure(
                    compiled,
                    AuthFlow::ClientCredentials,
                    "the discovery document is not a JSON object",
                ));
            }}
        }};
        // One member reader: absent or empty members are treated as absent
        // and unknown members are ignored; any other non-string value is a
        // typed discovery failure.
        let text = |name: &str| {{
            object.get(name).and_then(|value| match value {{
                Nullable::Value(JsonNonNullValue::String(text)) if !text.is_empty() => {{
                    std::option::Option::Some(text.clone())
                }}
                _ => std::option::Option::None,
            }})
        }};
        // The exact issuer rule: when the document carries an `issuer` claim,
        // it must be an absolute http(s) URL whose origin (scheme, host and
        // the port with the scheme default made explicit) equals the
        // discovery URL's origin; OpenID Connect openIdConnectUrl documents
        // are validated against their `issuer` claim exactly this way, as are
        // RFC 8414 authorization-server metadata documents. A missing claim
        // is tolerated.
        if let std::option::Option::Some(issuer) = text("issuer") {{
            if discovery_origin(&issuer) != discovery_origin(url) {{
                return std::result::Result::Err(discovery_failure(
                    compiled,
                    AuthFlow::ClientCredentials,
                    "the discovery document issuer does not share the discovery URL's origin",
                ));
            }}
        }}
        let token_endpoint = text("token_endpoint");
        let revocation_endpoint = text("revocation_endpoint");
        let introspection_endpoint = text("introspection_endpoint");
        let endpoints = DiscoveredEndpoints {{
            token_endpoint,
            revocation_endpoint,
            introspection_endpoint,
        }};
        let token = endpoints
            .token_endpoint
            .as_deref()
            .map(|url| {{
                discovered_form_operation(
                    compiled,
                    "oauth: discovered token endpoint",
                    url,
                    AuthFlow::ClientCredentials,
                )
            }})
            .transpose()?;
        let revocation = endpoints
            .revocation_endpoint
            .as_deref()
            .map(|url| {{
                discovered_form_operation(
                    compiled,
                    "oauth: discovered revocation endpoint",
                    url,
                    AuthFlow::Revocation,
                )
            }})
            .transpose()?;
        let introspection = endpoints
            .introspection_endpoint
            .as_deref()
            .map(|url| {{
                discovered_form_operation(
                    compiled,
                    "oauth: discovered introspection endpoint",
                    url,
                    AuthFlow::Introspection,
                )
            }})
            .transpose()?;
        std::result::Result::Ok(std::sync::Arc::new(Discovered {{
            token,
            revocation,
            introspection,
        }}))
    }}

    /// One client-credentials acquisition whose token endpoint the discovery
    /// document supplies. The synthesized client-credentials request matches
    /// the compiled flow's: only the `grant_type` (and, for public clients,
    /// the client id) travels on the wire.
    async fn acquire_discovered<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<TokenSet, SdkError> {{
        let operation = self.discovered_token_operation(client, compiled).await?;
        let client_id = self.public_client_id_if_public(compiled);
        let mut fields = std::vec![("grant_type", "client_credentials")];
        if let std::option::Option::Some(id) = &client_id {{
            fields.push(("client_id", id.as_str()));
        }}
        let authorization = self.authorization(compiled, AuthFlow::ClientCredentials)?;
        let raw = self
            .post(
                client,
                compiled,
                AuthFlow::ClientCredentials,
                operation,
                &fields,
                authorization,
            )
            .await?;
        let set = token_set(compiled, AuthFlow::ClientCredentials, &raw)?;
        self.store.replace(compiled.name, set.clone()).await;
        std::result::Result::Ok(set)
    }}

    /// The synthetic operation for the discovery document's token endpoint.
    async fn discovered_token_operation<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<&'static Operation, SdkError> {{
        let discovered = self.discovery_document(client, compiled).await?;
        discovered.token.ok_or_else(|| {{
            let mut error = crate::http::validation_error(
                compiled.source,
                compiled.source,
                "neither the compiled plan nor the discovery document carries a token endpoint",
            );
            error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                scheme: compiled.name.to_owned(),
                flow: AuthFlow::ClientCredentials,
                kind: AuthErrorKind::Unavailable,
                server_error: std::option::Option::None,
                server_description: std::option::Option::None,
                cause: std::option::Option::None,
            }}));
            error
        }})
    }}

    /// The synthetic operation for the discovery document's revocation
    /// endpoint.
    async fn discovered_revocation_operation<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<&'static Operation, SdkError> {{
        let discovered = self.discovery_document(client, compiled).await?;
        discovered.revocation.ok_or_else(|| {{
            let mut error = crate::http::validation_error(
                compiled.source,
                compiled.source,
                "neither the compiled plan nor the discovery document carries a revocation endpoint",
            );
            error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                scheme: compiled.name.to_owned(),
                flow: AuthFlow::Revocation,
                kind: AuthErrorKind::Unavailable,
                server_error: std::option::Option::None,
                server_description: std::option::Option::None,
                cause: std::option::Option::None,
            }}));
            error
        }})
    }}

    /// The synthetic operation for the discovery document's introspection
    /// endpoint.
    async fn discovered_introspection_operation<T: Transport>(
        &self,
        client: &Client<T>,
        compiled: &'static Scheme,
    ) -> std::result::Result<&'static Operation, SdkError> {{
        let discovered = self.discovery_document(client, compiled).await?;
        discovered.introspection.ok_or_else(|| {{
            let mut error = crate::http::validation_error(
                compiled.source,
                compiled.source,
                "neither the compiled plan nor the discovery document carries an introspection endpoint",
            );
            error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {{
                scheme: compiled.name.to_owned(),
                flow: AuthFlow::Introspection,
                kind: AuthErrorKind::Unavailable,
                server_error: std::option::Option::None,
                server_description: std::option::Option::None,
                cause: std::option::Option::None,
            }}));
            error
        }})
    }}
}}
"#,
        config.max_request_bytes,
        config.max_part_bytes,
        config.max_stream_item_bytes,
        config.max_chunk_bytes,
        config.max_header_bytes,
    )
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations.
const REPLAY_CORE: &str = r#"// ---------------------------------------------------------------------------
// The replaying credential wrapper (unified request policy)
// ---------------------------------------------------------------------------

// One attach this wrapper served, remembered so the transport can tell which
// requests carried its token. The record keeps only safe metadata; token
// values already traveled on the wire.
#[derive(Clone)]
struct ReplayServed {
    scheme: &'static str,
    value: std::string::String,
    eligible: bool,
}

// One coordinated refresh round: exactly one forced acquisition, shared by
// every concurrent 401 that presented the same stale token. Losing callers
// block their thread until the winning round completes; see the module
// documentation for the concurrency simplification.
struct ReplayRound {
    round: Round,
    outcome: std::sync::Mutex<std::option::Option<std::result::Result<std::string::String, ()>>>,
}
impl ReplayRound {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            round: Round::new(),
            outcome: std::sync::Mutex::new(std::option::Option::None),
        })
    }
    fn complete(&self, value: std::string::String) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = std::option::Option::Some(std::result::Result::Ok(value));
        self.round.complete();
    }
    fn fail(&self) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = std::option::Option::Some(std::result::Result::Err(()));
        self.round.complete();
    }
    fn wait(&self) -> std::result::Result<std::string::String, ()> {
        self.round.wait();
        self.outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or(std::result::Result::Err(()))
    }
}

struct ReplayShared {
    served: std::sync::Mutex<std::collections::VecDeque<ReplayServed>>,
    rounds: std::sync::Mutex<std::collections::BTreeMap<&'static str, std::sync::Arc<ReplayRound>>>,
    current: std::sync::Mutex<std::collections::BTreeMap<&'static str, std::string::String>>,
}
enum ReplayEntered {
    /// Another caller owns the in-flight refresh round for this scheme.
    Wait(std::sync::Arc<ReplayRound>),
    /// This caller owns the fresh round and performs the refresh.
    Proceed(std::sync::Arc<ReplayRound>),
}

/// The replaying client-credentials credential plus its transport policy: the
/// plain lifecycle's attach behavior plus the unified 401 replay policy.
///
/// A 401 (and only a 401) on a request whose Authorization value this wrapper
/// attached triggers exactly one coordinated refresh — concurrent 401s share
/// one token request round — and exactly one replay of the request with the
/// fresh token. The second response is surfaced whatever it is: a second 401
/// reaches the caller as the declared error. The overall budget is one
/// refresh plus one replay, never nested with other retry policies (requests
/// are not retried today). Attaches for stream-protected requirements are
/// never replayed, because delivered stream data prevents a transparent
/// restart. A refresh failure surfaces as the typed auth failure instead of a
/// replay. The plain lifecycle keeps today's semantics: replay is this
/// wrapper's opt-in only.
///
/// Wire it in three places: warm the store through
/// [`ReplayCredentials::token`], pass [`ReplayCredentials::hook`] as the
/// client's credential hook, and pass [`ReplayCredentials::transport`] as the
/// client's transport:
///
/// ```ignore
/// let replay = oauth::ReplayCredentials::new().with_client_credentials("id", "secret");
/// let client = Client::with_transport(replay.transport(my_transport), Credentials::new().with_hook(replay.hook()));
/// replay.token(&client, "scheme").await?;
/// ```
pub struct ReplayCredentials<S: TokenStore = MemoryTokenStore> {
    store: std::sync::Arc<S>,
    sessions: TokenSessions<S>,
    shared: std::sync::Arc<ReplayShared>,
    client_id: std::option::Option<std::string::String>,
    client_secret: std::option::Option<std::string::String>,
}
impl<S: TokenStore> std::fmt::Debug for ReplayCredentials<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayCredentials")
            .field("client_id", &self.client_id.is_some())
            .field("client_secret", &self.client_secret.is_some())
            .finish_non_exhaustive()
    }
}
impl ReplayCredentials<MemoryTokenStore> {
    /// A replaying wrapper with its own fresh in-process store.
    #[must_use]
    pub fn new() -> Self {
        Self::with_store(std::sync::Arc::new(MemoryTokenStore::new()))
    }
}
impl<S: TokenStore> ReplayCredentials<S> {
    /// A replaying wrapper over a caller-owned store.
    #[must_use]
    pub fn with_store(store: std::sync::Arc<S>) -> Self {
        Self {
            sessions: TokenSessions::with_store(std::sync::Arc::clone(&store)),
            store,
            shared: std::sync::Arc::new(ReplayShared {
                served: std::sync::Mutex::new(std::collections::VecDeque::new()),
                rounds: std::sync::Mutex::new(std::collections::BTreeMap::new()),
                current: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            }),
            client_id: std::option::Option::None,
            client_secret: std::option::Option::None,
        }
    }
    /// Explicit client credentials for the coordinated refresh, taking
    /// precedence over the compiled environment variables.
    #[must_use]
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<std::string::String>,
        client_secret: impl Into<std::string::String>,
    ) -> Self {
        self.client_id = std::option::Option::Some(client_id.into());
        self.client_secret = std::option::Option::Some(client_secret.into());
        // The wrapped sessions share the explicit credentials for their
        // acquisition and refresh requests.
        self.sessions.client_id = self.client_id.clone();
        self.sessions.client_secret = self.client_secret.clone();
        self
    }
    /// The wrapped token sessions value.
    #[must_use]
    pub fn sessions(&self) -> &TokenSessions<S> {
        &self.sessions
    }
    /// Warm (or reuse) the scheme's stored set through the wrapped sessions
    /// and remember its attach value so the credential hook can serve it.
    ///
    /// # Errors
    /// Unknown schemes, missing configuration or credentials, and unusable or
    /// rejected token responses.
    pub async fn token<T: Transport>(
        &self,
        client: &Client<T>,
        scheme: &str,
    ) -> std::result::Result<TokenSet, SdkError> {
        let compiled = compiled_scheme(scheme)?;
        if compiled.client_credentials.is_none() {
            return std::result::Result::Err(unavailable(
                scheme,
                AuthFlow::ClientCredentials,
            ));
        }
        let set = self.sessions.client_credentials_token(client, scheme).await?;
        self.remember(compiled, &set);
        std::result::Result::Ok(set)
    }
    /// The credential hook: serves the remembered client-credentials value
    /// for any compiled scheme and records whether the attach may be
    /// replayed. Requirements without a warmed value fail typed; the caller
    /// warms them through [`ReplayCredentials::token`].
    #[must_use]
    pub fn hook(&self) -> impl crate::http::CredentialProvider + 'static {
        let shared = std::sync::Arc::clone(&self.shared);
        move |requirement: &crate::http::CredentialRequirement| {
            let compiled = match compiled_scheme(requirement.name) {
                std::result::Result::Ok(compiled)
                    if compiled.client_credentials.is_some() =>
                {
                    compiled
                }
                _ => return std::result::Result::Ok(std::option::Option::None),
            };
            let value = {
                let current = shared
                    .current
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match current.get(compiled.name) {
                    std::option::Option::Some(value) => value.clone(),
                    std::option::Option::None => {
                        return std::result::Result::Err(
                            std::boxed::Box::new(replay_cold(compiled))
                                as BoxError,
                        );
                    }
                }
            };
            let mut eligible = true;
            for (scheme, pointers) in REPLAY_NO_REPLAY {
                if *scheme == compiled.name && pointers.contains(&requirement.source.pointer) {
                    eligible = false;
                }
            }
            {
                let mut served = shared
                    .served
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                served.push_front(ReplayServed {
                    scheme: compiled.name,
                    value: value.clone(),
                    eligible,
                });
                served.truncate(8);
            }
            std::result::Result::Ok(std::option::Option::Some(
                crate::http::Credential::Authorization(value),
            ))
        }
    }
    /// Wrap `inner` with the one-refresh-one-replay 401 policy; call once per
    /// client. Token requests keep traveling through `inner` directly.
    #[must_use]
    pub fn transport<T: Transport>(&self, inner: T) -> ReplayTransport<T, S> {
        ReplayTransport {
            inner,
            shared: std::sync::Arc::clone(&self.shared),
            store: std::sync::Arc::clone(&self.store),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
        }
    }
    fn remember(&self, compiled: &'static Scheme, set: &TokenSet) {
        self.shared
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(compiled.name, replay_value(set));
    }
}

/// The complete Authorization header value of a stored set.
fn replay_value(set: &TokenSet) -> std::string::String {
    let token_type = if set.token_type.is_empty() {
        "bearer".to_owned()
    } else {
        set.token_type.clone()
    };
    format!("{token_type} {}", set.access_token)
}

/// One endpoint operation's absolute URL: the synthesized token descriptors
/// compile one origin server template and one path.
fn replay_operation_url(operation: &Operation) -> std::string::String {
    let server = operation
        .servers
        .first()
        .map(|server| server.template)
        .unwrap_or("");
    let mut url = server.trim_end_matches('/').to_owned();
    url.push_str(operation.path_template);
    url
}

fn replay_cold(compiled: &'static Scheme) -> SdkError {
    let mut error = crate::http::validation_error(
        compiled.source,
        compiled.source,
        "no client-credentials token is warmed for this scheme; call ReplayCredentials::token before attaching",
    );
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: compiled.name.to_owned(),
        flow: AuthFlow::ClientCredentials,
        kind: AuthErrorKind::MissingClientCredentials,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause: std::option::Option::None,
    }));
    error
}

fn replay_refresh_failed(compiled: &'static Scheme) -> SdkError {
    let mut error = crate::http::validation_error(
        compiled.source,
        compiled.source,
        "the coordinated refresh failed",
    );
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: compiled.name.to_owned(),
        flow: AuthFlow::ClientCredentials,
        kind: AuthErrorKind::Transport,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause: std::option::Option::None,
    }));
    error
}

fn replay_transport_failure(compiled: &'static Scheme, cause: BoxError) -> SdkError {
    let mut error = crate::http::validation_error(
        compiled.source,
        compiled.source,
        "the token request failed before a response existed",
    );
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: compiled.name.to_owned(),
        flow: AuthFlow::ClientCredentials,
        kind: AuthErrorKind::Transport,
        server_error: std::option::Option::None,
        server_description: std::option::Option::None,
        cause: std::option::Option::Some(cause),
    }));
    error
}

fn replay_rejection(compiled: &'static Scheme, status: u16, body: &[u8]) -> SdkError {
    let mut error = crate::http::validation_error(
        compiled.source,
        compiled.source,
        "the OAuth endpoint rejected the request",
    );
    error.kind = SdkErrorKind::UnexpectedResponse;
    error.status = std::option::Option::Some(status);
    let capture = body.len().min(CAPTURE_BYTES);
    error.raw_capture = body[..capture].to_vec();
    error.truncated = body.len() > capture;
    let (server_error, server_description) = oauth_error(body);
    error.cause = std::option::Option::Some(std::boxed::Box::new(AuthError {
        scheme: compiled.name.to_owned(),
        flow: AuthFlow::ClientCredentials,
        kind: AuthErrorKind::ServerRejected,
        server_error,
        server_description,
        cause: std::option::Option::None,
    }));
    error
}

/// Decode one RFC 6749 token response body into a token set.
fn replay_token_response(
    compiled: &'static Scheme,
    status: u16,
    body: &[u8],
) -> std::result::Result<TokenSet, SdkError> {
    if !(200..300).contains(&status) {
        return std::result::Result::Err(replay_rejection(compiled, status, body));
    }
    let value = crate::parse_json_bytes(body, TOKEN_JSON_LIMITS).map_err(|cause| {
        invalid_response(
            compiled,
            AuthFlow::ClientCredentials,
            std::option::Option::Some(std::boxed::Box::new(cause)),
        )
    })?;
    let object = match &value {
        Nullable::Value(JsonNonNullValue::Object(map)) => map,
        _ => {
            return std::result::Result::Err(invalid_response(
                compiled,
                AuthFlow::ClientCredentials,
                std::option::Option::None,
            ))
        }
    };
    let text = |name: &str| {
        object.get(name).and_then(|value| match value {
            Nullable::Value(JsonNonNullValue::String(text)) if !text.is_empty() => {
                std::option::Option::Some(text.clone())
            }
            _ => std::option::Option::None,
        })
    };
    let Some(access_token) = text("access_token") else {
        return std::result::Result::Err(invalid_response(
            compiled,
            AuthFlow::ClientCredentials,
            std::option::Option::None,
        ));
    };
    let token_type = text("token_type").unwrap_or_else(|| "bearer".to_owned());
    let expires_in = object.get("expires_in").and_then(|value| match value {
        Nullable::Value(JsonNonNullValue::Number(token)) => token
            .as_str()
            .parse::<JsonInteger>()
            .ok()
            .and_then(|token| token.to_u128()),
        _ => std::option::Option::None,
    });
    let now = std::time::SystemTime::now();
    let expires_at = expires_in
        .and_then(|seconds| u64::try_from(seconds).ok())
        .and_then(|seconds| now.checked_add(std::time::Duration::from_secs(seconds)))
        .unwrap_or_else(|| {
            now.checked_add(std::time::Duration::from_secs(365 * 24 * 60 * 60))
                .expect("one year fits a SystemTime offset")
        });
    std::result::Result::Ok(TokenSet {
        access_token,
        token_type,
        expires_at,
        refresh_token: text("refresh_token"),
        scope: text("scope"),
        issuer_account: text("iss"),
    })
}

/// The replaying transport: one coordinated refresh and, when the request
/// carried this wrapper's token and no stream is protected on it, exactly one
/// replay with the fresh token. Lifecycle endpoint requests are never
/// replayed: they carry no bearer token of this wrapper, and the exact-target
/// guard is defense in depth.
pub struct ReplayTransport<T: Transport, S: TokenStore = MemoryTokenStore> {
    inner: T,
    shared: std::sync::Arc<ReplayShared>,
    store: std::sync::Arc<S>,
    client_id: std::option::Option<std::string::String>,
    client_secret: std::option::Option<std::string::String>,
}
impl<T: Transport, S: TokenStore> Transport for ReplayTransport<T, S> {
    type Body = T::Body;
    async fn send(
        &self,
        request: crate::http::Request,
    ) -> std::result::Result<crate::http::TransportResponse<Self::Body>, BoxError> {
        let presented = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| String::from_utf8_lossy(value).to_string());
        let replay_request = crate::http::Request {
            method: request.method,
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
        };
        let response = self.inner.send(request).await?;
        if response.status != 401 {
            return std::result::Result::Ok(response);
        }
        let Some(presented) = presented else {
            return std::result::Result::Ok(response);
        };
        let Some(scheme) = self.replayable(&presented) else {
            return std::result::Result::Ok(response);
        };
        let Ok(compiled) = compiled_scheme(scheme) else {
            return std::result::Result::Ok(response);
        };
        if replay_lifecycle_target(compiled, &replay_request.url) {
            return std::result::Result::Ok(response);
        }
        let fresh = match self.refresh(compiled, &presented).await {
            std::result::Result::Ok(value) => value,
            std::result::Result::Err(error) => {
                return std::result::Result::Err(std::boxed::Box::new(error));
            }
        };
        let mut headers = replay_request.headers.clone();
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        headers.push(("authorization".to_owned(), fresh.into_bytes()));
        let replayed = crate::http::Request {
            method: replay_request.method,
            url: replay_request.url,
            headers,
            body: replay_request.body,
        };
        self.inner.send(replayed).await
    }
}
impl<T: Transport, S: TokenStore> ReplayTransport<T, S> {
    fn replayable(&self, presented: &str) -> std::option::Option<&'static str> {
        let served = self
            .shared
            .served
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matched: std::option::Option<&'static str> = std::option::Option::None;
        for entry in served.iter() {
            if entry.value != presented {
                continue;
            }
            if !entry.eligible {
                // A stream-protected attach served this token: never replay.
                return std::option::Option::None;
            }
            matched = std::option::Option::Some(entry.scheme);
        }
        matched
    }
    // The coordinated refresh: a newer stored set wins over a stale
    // re-refresh, concurrent 401s share one round, and a failed round fails
    // every waiter exactly once.
    async fn refresh(
        &self,
        compiled: &'static Scheme,
        presented: &str,
    ) -> std::result::Result<std::string::String, SdkError> {
        loop {
            if let Some(stored) = self.store.load(compiled.name).await {
                let value = replay_value(&stored);
                if value != presented {
                    return std::result::Result::Ok(value);
                }
            }
            let entered = {
                let mut rounds = self
                    .shared
                    .rounds
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match rounds.get(compiled.name) {
                    std::option::Option::Some(occupied) => {
                        let done = *occupied
                            .round
                            .done
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if done {
                            let round = ReplayRound::new();
                            rounds.insert(compiled.name, std::sync::Arc::clone(&round));
                            ReplayEntered::Proceed(round)
                        } else {
                            ReplayEntered::Wait(std::sync::Arc::clone(occupied))
                        }
                    }
                    std::option::Option::None => {
                        let round = ReplayRound::new();
                        rounds.insert(compiled.name, std::sync::Arc::clone(&round));
                        ReplayEntered::Proceed(round)
                    }
                }
            };
            match entered {
                ReplayEntered::Wait(round) => match round.wait() {
                    std::result::Result::Ok(value) => {
                        return std::result::Result::Ok(value);
                    }
                    std::result::Result::Err(()) => {
                        return std::result::Result::Err(replay_refresh_failed(compiled));
                    }
                },
                ReplayEntered::Proceed(round) => {
                    if let Some(stored) = self.store.load(compiled.name).await {
                        let value = replay_value(&stored);
                        if value != presented {
                            round.complete(value.clone());
                            return std::result::Result::Ok(value);
                        }
                    }
                    self.store.clear(compiled.name).await;
                    match self.acquire(compiled).await {
                        std::result::Result::Ok(set) => {
                            self.store
                                .replace(compiled.name, set.clone())
                                .await;
                            self.shared
                                .current
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .insert(compiled.name, replay_value(&set));
                            let value = replay_value(&set);
                            round.complete(value.clone());
                            return std::result::Result::Ok(value);
                        }
                        std::result::Result::Err(error) => {
                            round.fail();
                            return std::result::Result::Err(error);
                        }
                    }
                }
            }
        }
    }
    // One forced client-credentials acquisition over the wrapped transport.
    async fn acquire(&self, compiled: &'static Scheme) -> std::result::Result<TokenSet, SdkError> {
        let grant = match compiled.client_credentials.as_ref() {
            std::option::Option::Some(grant) => grant,
            std::option::Option::None => {
                return std::result::Result::Err(unavailable(
                    compiled.name,
                    AuthFlow::ClientCredentials,
                ));
            }
        };
        let mut body = std::string::String::new();
        form_pair(&mut body, "grant_type", "client_credentials");
        let client_id = self.client_id.clone().or_else(|| {
            compiled
                .client_id_env
                .and_then(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        });
        let mut headers: crate::http::Headers = std::vec![
            (
                "content-type".to_owned(),
                b"application/x-www-form-urlencoded".to_vec(),
            ),
            ("accept".to_owned(), b"application/json".to_vec()),
        ];
        if compiled.secret_basic {
            let id = client_id.ok_or_else(|| {
                missing_credentials(compiled, AuthFlow::ClientCredentials, compiled.client_id_env)
            })?;
            let secret = self
                .client_secret
                .clone()
                .or_else(|| {
                    compiled.client_secret_env.and_then(|name| {
                        std::env::var(name).ok().filter(|value| !value.is_empty())
                    })
                })
                .ok_or_else(|| {
                    missing_credentials(
                        compiled,
                        AuthFlow::ClientCredentials,
                        compiled.client_secret_env,
                    )
                })?;
            headers.push((
                "authorization".to_owned(),
                format!(
                    "Basic {}",
                    base64(
                        format!("{}:{}", form_component(&id), form_component(&secret)).as_bytes()
                    )
                )
                .into_bytes(),
            ));
        } else if let std::option::Option::Some(id) = &client_id {
            body.push('&');
            form_pair(&mut body, "client_id", id);
        }
        let request = crate::http::Request {
            method: "POST",
            url: replay_operation_url(grant.token),
            headers,
            body: std::option::Option::Some(body.into_bytes()),
        };
        let response = match self.inner.send(request).await {
            std::result::Result::Ok(response) => response,
            std::result::Result::Err(error) => {
                return std::result::Result::Err(replay_transport_failure(
                    compiled,
                    error,
                ));
            }
        };
        let status = response.status;
        let mut bytes: std::vec::Vec<u8> = std::vec::Vec::new();
        let mut body_reader = response.body;
        loop {
            match crate::http::ResponseBody::next_chunk(&mut body_reader).await {
                std::result::Result::Ok(std::option::Option::Some(chunk)) => {
                    if bytes.len() + chunk.len() > TOKEN_JSON_LIMITS.max_input_bytes {
                        return std::result::Result::Err(replay_rejection(
                            compiled,
                            status,
                            &bytes,
                        ));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                std::result::Result::Ok(std::option::Option::None) => break,
                std::result::Result::Err(error) => {
                    return std::result::Result::Err(replay_transport_failure(
                        compiled,
                        error,
                    ));
                }
            }
        }
        replay_token_response(compiled, status, &bytes)
    }
}
"#;

/// The plain variant's lifecycle-endpoint exclusion: the compiled
/// client-credentials token and refresh endpoints.
const REPLAY_LIFECYCLE_PLAIN: &str = r#"
fn replay_lifecycle_target(compiled: &'static Scheme, url: &str) -> bool {
    let token = compiled
        .client_credentials
        .as_ref()
        .map(|grant| replay_operation_url(grant.token));
    let refresh = compiled
        .client_credentials
        .as_ref()
        .and_then(|grant| grant.refresh)
        .map(replay_operation_url);
    token.as_deref() == std::option::Option::Some(url)
        || refresh.as_deref() == std::option::Option::Some(url)
}
"#;

/// The discovery variant's lifecycle-endpoint exclusion: the compiled
/// discovery URL joins the compiled endpoints; the resolved token endpoint
/// rides the exact-token match.
const REPLAY_LIFECYCLE_DISCOVERY: &str = r#"
fn replay_lifecycle_target(compiled: &'static Scheme, url: &str) -> bool {
    let token = compiled
        .client_credentials
        .as_ref()
        .map(|grant| replay_operation_url(grant.token));
    let refresh = compiled
        .client_credentials
        .as_ref()
        .and_then(|grant| grant.refresh)
        .map(replay_operation_url);
    token.as_deref() == std::option::Option::Some(url)
        || refresh.as_deref() == std::option::Option::Some(url)
        || compiled.discovery_url == std::option::Option::Some(url)
}
"#;
