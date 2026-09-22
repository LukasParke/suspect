//! Emitted-only OAuth 2.0 token lifecycle for the native C++ client.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when SDK defaults are configured, exactly like pagination) lowers
//! into one generated, header-only `include/<package>/oauth.hpp`: the compiled
//! scheme descriptors as constants, an instance-owned mutex-guarded token
//! store, client-credentials acquisition with skew-aware caching, per-key
//! single-flight rounds and atomic replacement, explicit refresh with
//! rotated-refresh adoption, and — only when the compiled schemes carry them —
//! RFC 8628 device authorization with injectable pacing, RFC 7009 revocation
//! and RFC 7662 introspection. Implicit and password flows are represented by
//! the plan for documentation only; they are never executed, so schemes with
//! only those flows emit nothing.
//!
//! On top of the plain lifecycle, and only when a compiled scheme carries an
//! executable client-credentials flow, one opt-in replaying credential wrapper
//! joins the emitted header: it records which Authorization values its
//! attaches served (with per-attach stream-protected eligibility), wraps a
//! caller transport so a 401 (and only a 401) carrying its token triggers
//! exactly one coordinated refresh through the wrapped session's own
//! single-flight machinery plus exactly one replay through the inner
//! transport, and never replays lifecycle endpoints or stream-protected
//! attaches. Plans without an executable client-credentials flow assemble
//! byte-identically to the pre-replay emission.
//!
//! Emission is strictly conditional: without configured defaults, or without
//! any usable scheme, the backend emits nothing new, so no-policy output stays
//! byte-identical. The helper type stands alone (no generated `Client` member
//! changes), token endpoint requests run through the caller-supplied
//! transport, credential values are read from the compiled environment
//! variable names at call time, and typed auth errors never carry token or
//! client-secret material. Authorization-code with PKCE S256 draws its
//! verifiers and states from `std::random_device` (the standard routes it to
//! the operating system's entropy source on the toolchains this runtime
//! targets), detects and refuses deterministic implementations at runtime,
//! and hashes the S256 challenge with a local FIPS 180-4 SHA-256 carrying a
//! first-use self-test vector.

use std::collections::{BTreeMap, BTreeSet};

use super::models::allocate;
use super::{PlannedOperation, SdkPlan};
use crate::http_protocol as wire;

/// Whether one compiled flow can be executed by the generated runtime.
fn executable(flow: &wire::OAuthFlowDescriptor) -> bool {
    if flow.deprecated_flow {
        return false;
    }
    match flow.kind {
        wire::OAuthFlowDescriptorKind::ClientCredentials => flow.token_url.is_some(),
        wire::OAuthFlowDescriptorKind::AuthorizationCode => {
            flow.authorization_url.is_some() && flow.token_url.is_some()
        }
        wire::OAuthFlowDescriptorKind::DeviceAuthorization => {
            flow.device_authorization_url.is_some() && flow.token_url.is_some()
        }
        // Implicit and password flows are never executed by generated code.
        wire::OAuthFlowDescriptorKind::Implicit | wire::OAuthFlowDescriptorKind::Password => false,
    }
}

/// The executable flow subset of one scheme, in declaration order.
fn executable_flows(scheme: &wire::OAuthSchemePlan) -> Vec<&wire::OAuthFlowDescriptor> {
    scheme
        .flows
        .iter()
        .filter(|flow| executable(flow))
        .collect()
}

/// One source scheme compiled down to executable endpoints.
#[derive(Debug, Clone)]
pub struct OAuthScheme {
    pub name: String,
    /// Refresh-before-expiry clock skew in seconds.
    pub skew_seconds: u32,
    /// Token-endpoint requests authenticate with the configured secret
    /// variable; otherwise the client is public.
    pub client_secret_basic: bool,
    pub client_id_env: Option<String>,
    pub client_secret_env: Option<String>,
    /// The issuing authority origin that store keys partition on.
    pub issuer: String,
    /// The declared refresh endpoint, else the token endpoint.
    pub refresh_url: Option<String>,
    pub client_credentials_url: Option<String>,
    /// The declared authorization-request URL, carried whole: it is rendered
    /// into the returned transaction, never sent through the transport.
    pub authorization_url: Option<String>,
    /// The authorization-code flow's token endpoint.
    pub code_token_url: Option<String>,
    pub device_url: Option<String>,
    pub device_token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub introspection_url: Option<String>,
    /// The known discovery/metadata document. A scheme with no executable
    /// flow is lowered only when its discovery URL defines the endpoints at
    /// runtime (OpenID Connect); a compiled endpoint always wins over the
    /// discovered one.
    pub discovery_url: Option<String>,
}

/// The allocated native type names of one emitted lifecycle surface.
#[derive(Debug, Clone)]
pub struct OAuthNames {
    pub sessions: String,
    pub token_set: String,
    pub token_store: String,
    pub memory_store: String,
    pub error: String,
    pub options: String,
    pub authorization: String,
    pub device: String,
    pub introspection: String,
    /// The opt-in replaying credential wrapper and its transport, allocated
    /// only when a compiled scheme carries an executable client-credentials
    /// flow, the provider the wrapper wraps.
    pub replay_credentials: Option<String>,
    pub replay_transport: Option<String>,
}

/// The compiled OAuth emission carried by one plan.
#[derive(Debug, Clone)]
pub struct OAuthPlan {
    /// The shared compiled selection this emission follows.
    pub outcome: wire::OAuthPlan,
    pub schemes: Vec<OAuthScheme>,
    pub names: OAuthNames,
}

impl OAuthPlan {
    /// Whether this plan emits OAuth support at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.schemes.is_empty()
    }
}

/// The `http(s)://authority` origin of one compiled absolute URL, or an empty
/// string when the URL cannot be parsed (the planner already validated
/// declared flow URLs).
fn origin(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return String::new();
    };
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
        return String::new();
    }
    let Some(host) = parsed.host().map(|host| host.to_string()) else {
        return String::new();
    };
    match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    }
}

/// Lower the compiled plan into its executable emission, allocating the fixed
/// native type names against the package namespace. `None` means no usable
/// scheme, so nothing is emitted and no name is reserved.
pub(super) fn lower(outcome: &wire::OAuthPlan, names: &mut BTreeSet<String>) -> Option<OAuthPlan> {
    let mut schemes = Vec::new();
    for scheme in &outcome.schemes {
        let flows = executable_flows(scheme);
        let discovery_url = scheme.discovery.clone();
        let Some(first) = flows.first() else {
            // A scheme with no executable flow is lowered only when its
            // discovery URL defines the endpoints at runtime (OpenID
            // Connect): every endpoint stays absent and the discovery URL
            // drives resolution. The client authentication has no compiled
            // flow to derive from, so the compiled configuration decides:
            // client-secret-basic when a client secret variable is
            // configured, else the public profile.
            if discovery_url.is_none() {
                continue;
            }
            schemes.push(OAuthScheme {
                name: scheme.name.clone(),
                skew_seconds: scheme.refresh_skew_seconds,
                client_secret_basic: scheme.client_secret_env.is_some(),
                client_id_env: scheme.client_id_env.clone(),
                client_secret_env: scheme.client_secret_env.clone(),
                issuer: origin(scheme.discovery.as_deref().unwrap_or_default()),
                refresh_url: None,
                client_credentials_url: None,
                authorization_url: None,
                code_token_url: None,
                device_url: None,
                device_token_url: None,
                revocation_url: scheme.revocation_endpoint.clone(),
                introspection_url: scheme.introspection_endpoint.clone(),
                discovery_url,
            });
            continue;
        };
        let issuer = first.token_url.as_deref().map_or_else(String::new, origin);
        let mut client_credentials_url = None;
        let mut authorization_url = None;
        let mut code_token_url = None;
        let mut device_url = None;
        let mut device_token_url = None;
        for flow in &flows {
            match flow.kind {
                wire::OAuthFlowDescriptorKind::ClientCredentials
                    if client_credentials_url.is_none() =>
                {
                    client_credentials_url = flow.token_url.clone();
                }
                wire::OAuthFlowDescriptorKind::AuthorizationCode if authorization_url.is_none() => {
                    // The authorization URL is a browser redirect, not a
                    // compiled transport endpoint: it is carried whole and
                    // extended with the request parameters at begin time. The
                    // planner already validated it as an absolute http(s) URL.
                    authorization_url = flow.authorization_url.clone();
                    code_token_url = flow.token_url.clone();
                }
                wire::OAuthFlowDescriptorKind::DeviceAuthorization if device_url.is_none() => {
                    device_url = flow.device_authorization_url.clone();
                    device_token_url = flow.token_url.clone();
                }
                _ => {}
            }
        }
        schemes.push(OAuthScheme {
            name: scheme.name.clone(),
            skew_seconds: scheme.refresh_skew_seconds,
            client_secret_basic: first.client_auth == wire::OAuthClientAuth::ClientSecretBasic,
            client_id_env: scheme.client_id_env.clone(),
            client_secret_env: scheme.client_secret_env.clone(),
            issuer,
            refresh_url: first
                .refresh_url
                .clone()
                .or_else(|| first.token_url.clone()),
            client_credentials_url,
            authorization_url,
            code_token_url,
            device_url,
            device_token_url,
            revocation_url: scheme.revocation_endpoint.clone(),
            introspection_url: scheme.introspection_endpoint.clone(),
            discovery_url,
        });
    }
    if schemes.is_empty() {
        return None;
    }
    let names = OAuthNames {
        sessions: allocate("OAuthSessions", names),
        token_set: allocate("OAuthTokenSet", names),
        token_store: allocate("OAuthTokenStore", names),
        memory_store: allocate("OAuthMemoryTokenStore", names),
        error: allocate("OAuthError", names),
        options: allocate("OAuthTokenOptions", names),
        authorization: allocate("OAuthAuthorization", names),
        device: allocate("OAuthDeviceGrant", names),
        introspection: allocate("OAuthIntrospection", names),
        // The replaying credential wrapper joins only when a compiled scheme
        // carries an executable client-credentials flow, the provider it
        // wraps; plans without one reserve nothing and assemble
        // byte-identically to the pre-replay emission.
        replay_credentials: schemes
            .iter()
            .any(|scheme| scheme.client_credentials_url.is_some())
            .then(|| allocate("OAuthReplayCredentials", names)),
        replay_transport: schemes
            .iter()
            .any(|scheme| scheme.client_credentials_url.is_some())
            .then(|| allocate("OAuthReplayTransport", names)),
    };
    Some(OAuthPlan {
        outcome: outcome.clone(),
        schemes,
        names,
    })
}

/// A `std::string_view` literal with an exact byte length; three-digit octal
/// escapes cannot absorb adjacent digits, and embedded NUL and hostile source
/// prose are data.
fn sv(value: &str) -> String {
    let mut literal = String::new();
    for byte in value.bytes() {
        match byte {
            b'"' => literal.push_str("\\\""),
            b'\\' => literal.push_str("\\\\"),
            32..=126 => literal.push(byte as char),
            byte => literal.push_str(&format!("\\{byte:03o}")),
        }
    }
    format!("std::string_view(\"{literal}\", {})", value.len())
}

/// The empty `std::string_view` member literal.
const SV_NONE: &str = "std::string_view()";

fn optional_sv(value: &Option<String>) -> String {
    value.as_deref().map_or_else(|| SV_NONE.to_owned(), sv)
}

/// Whether any compiled scheme carries the device grant, so the device
/// sections join the emitted header.
fn has_device(plan: &OAuthPlan) -> bool {
    plan.schemes
        .iter()
        .any(|scheme| scheme.device_url.is_some() && scheme.device_token_url.is_some())
}

/// Whether any compiled scheme carries the authorization-code grant, so the
/// PKCE sections join the emitted header.
fn has_authorization(plan: &OAuthPlan) -> bool {
    plan.schemes
        .iter()
        .any(|scheme| scheme.authorization_url.is_some() && scheme.code_token_url.is_some())
}

fn has_revocation(plan: &OAuthPlan) -> bool {
    plan.schemes
        .iter()
        .any(|scheme| scheme.revocation_url.is_some())
}

fn has_introspection(plan: &OAuthPlan) -> bool {
    plan.schemes
        .iter()
        .any(|scheme| scheme.introspection_url.is_some())
}

/// Whether any compiled scheme carries a discovery URL, so the discovery
/// engine, its typed failure kind and the endpoint-resolution precedence join
/// the emitted header. Plans without one assemble byte-identically to the
/// pre-discovery emission.
fn has_discovery(plan: &OAuthPlan) -> bool {
    plan.schemes
        .iter()
        .any(|scheme| scheme.discovery_url.is_some())
}

/// Render `include/<package>/oauth.hpp`. Called only when at least one usable
/// scheme exists.
pub(super) fn header(plan: &SdkPlan, oauth: &OAuthPlan) -> String {
    let mut out = String::from("#pragma once\n");
    out.push_str(&format!(
        "/** @file oauth.hpp Generated first-party OAuth 2.0 token lifecycle.\n{}\n",
        prose_header(oauth)
    ));
    out.push_str(&format!("#include \"{}/http.hpp\"\n", plan.config.name));
    out.push_str(
        "#include <algorithm>\n#include <array>\n#include <chrono>\n#include <condition_variable>\n#include <cstdint>\n#include <cstdlib>\n#include <functional>\n#include <map>\n#include <memory>\n#include <mutex>\n#include <optional>\n#include <random>\n#include <string>\n#include <string_view>\n#include <thread>\n#include <utility>\n#include <vector>\n\n",
    );
    out.push_str(&format!("namespace {} {{\n\n", plan.config.namespace));
    out.push_str(&descriptors(oauth));
    out.push_str(&core(oauth));
    if oauth
        .names
        .replay_credentials
        .as_ref()
        .is_some_and(|name| !name.is_empty())
    {
        out.push_str(&replay_section(plan, oauth));
    }
    out.push_str(&format!("}} // namespace {}\n", plan.config.namespace));
    expand_names(&out, &oauth.names)
}

/// The `@NAME@` substitutions for one emission's allocated identifiers.
fn expand_names(source: &str, names: &OAuthNames) -> String {
    let mut expanded = source
        .replace("@SESSIONS@", &names.sessions)
        .replace("@TOKEN_SET@", &names.token_set)
        .replace("@TOKEN_STORE@", &names.token_store)
        .replace("@MEMORY_STORE@", &names.memory_store)
        .replace("@ERROR@", &names.error)
        .replace("@OPTIONS@", &names.options)
        .replace("@AUTHORIZATION@", &names.authorization)
        .replace("@DEVICE@", &names.device)
        .replace("@INTROSPECTION@", &names.introspection);
    if let Some(name) = &names.replay_credentials {
        expanded = expanded.replace("@REPLAY@", name);
    }
    if let Some(name) = &names.replay_transport {
        expanded = expanded.replace("@REPLAY_TRANSPORT@", name);
    }
    expanded
}

/// The generated file's ownership and safety documentation. Byte-exact
/// without a discovery URL, with the discovery paragraphs for plans with one.
fn prose_header(oauth: &OAuthPlan) -> String {
    let scheme_list = oauth
        .schemes
        .iter()
        .map(|scheme| scheme.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let (never_paragraph, discovery_paragraph) = if has_discovery(oauth) {
        (
            "The runtime never parses OpenAPI\n\
             * and never invents an endpoint: endpoint URLs the compiled flows omit\n\
             * resolve at call time through RFC 8414 / OpenID Connect discovery when\n\
             * the scheme compiles a discovery URL. Client",
            "A compiled discovery URL also makes a scheme usable\n\
             * with no declared executable flows: the discovery document defines\n\
             * its endpoints at runtime.",
        )
    } else {
        (
            "The runtime never parses OpenAPI,\n\
             * never performs discovery and never invents an endpoint. Client",
            "OpenID Connect discovery remains caller-owned.",
        )
    };
    // The replaying wrapper's paragraph joins only when a compiled scheme
    // carries an executable client-credentials flow, the provider it wraps.
    let replay_paragraph = if oauth
        .names
        .replay_credentials
        .as_ref()
        .is_some_and(|name| !name.is_empty())
    {
        "\n\
         *\n\
         * The opt-in replaying credential wrapper extends exactly the compiled\n\
         * client-credentials providers with the unified 401 request policy: one\n\
         * coordinated refresh plus one eligible request replay per qualifying\n\
         * 401. Attaches served on stream-protected operations are never\n\
         * replayed, and lifecycle endpoints are never replayed. The plain\n\
         * lifecycle keeps today's attach-only semantics.\n\
         * Deprecated implicit and password flows are\n\
         * never executed. "
    } else {
        " Deprecated implicit and password flows are\n\
         * never executed. "
    };
    format!(
        " *\n\
         * Every endpoint, environment variable name and policy constant below\n\
         * is a generation-time constant compiled from the used security schemes\n\
         * of this package's source document. {never_paragraph}\n\
         * identity comes from explicit call options or the compiled environment\n\
         * variable names, read at call time: credential values are never\n\
         * embedded in emitted bytes, and token or client-secret values never\n\
         * enter error metadata.\n\
         *\n\
         * Implemented here, exactly for the schemes compiled below:\n\
         * client-credentials acquisition with skew-aware caching, per-key\n\
         * single-flight rounds and atomic store replacement, explicit refresh\n\
         * with rotated-refresh adoption, authorization-code with PKCE S256\n\
         * (RFC 7636) bound to single-use transactions, RFC 8628 device\n\
         * authorization with injectable pacing, RFC 7009 revocation and\n\
         * RFC 7662 introspection.{replay_paragraph}{discovery_paragraph}\n\
         *\n\
         * Entropy: PKCE verifiers and states draw 32 bytes from\n\
         * std::random_device. The C++ standard routes random_device to the\n\
         * operating system's entropy source on the toolchains this runtime\n\
         * targets (libstdc++ reads /dev/urandom or getrandom(2), libc++ uses\n\
         * getrandom/arc4random, MSVC uses the process RTL CSPRNG); a conforming\n\
         * implementation is a cryptographic generator or refuses to run.\n\
         * Known deterministic implementations (historically some MinGW\n\
         * builds) are detected at runtime — successive tokens of a real\n\
         * generator never repeat, so a repeated draw is refused with a typed\n\
         * Entropy error rather than minting predictable PKCE material — and\n\
         * the S256 challenge is hashed by a local FIPS 180-4 SHA-256 whose\n\
         * first use checks the standard 'abc' test vector and refuses on any\n\
         * mismatch. No third-party dependency is introduced.\n\
         *\n\
         * Token sets live only in the token store owned by each @SESSIONS@\n\
         * instance, under keys partitioned by source scheme, token-endpoint\n\
         * issuer and client identity; there is no process-global token cache.\n\
         *\n\
         * Compiled source schemes: {scheme_list}.\n\
         */\n",
        scheme_list = scheme_list,
        never_paragraph = never_paragraph,
        discovery_paragraph = discovery_paragraph,
        replay_paragraph = replay_paragraph,
    )
}

/// The compiled descriptor table and its JSON/form helpers, in the
/// runtime-owned `detail` namespace so no model symbol can collide with them.
fn descriptors(oauth: &OAuthPlan) -> String {
    let discovery = has_discovery(oauth);
    let mut out = String::from("namespace detail {\n\n");
    out.push_str(if discovery {
        DESCRIPTOR_TYPE_DISCOVERY
    } else {
        DESCRIPTOR_TYPE
    });
    out.push_str("inline const OAuthSchemeDescriptor oauth_schemes[] = {\n");
    for scheme in &oauth.schemes {
        if discovery {
            out.push_str(&format!(
                "    {{{}, {}, {}, {}, std::chrono::seconds{{{}}}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}}},\n",
                sv(&scheme.name),
                scheme.client_secret_basic,
                optional_sv(&scheme.client_id_env),
                optional_sv(&scheme.client_secret_env),
                scheme.skew_seconds,
                sv(&scheme.issuer),
                optional_sv(&scheme.refresh_url),
                optional_sv(&scheme.client_credentials_url),
                optional_sv(&scheme.authorization_url),
                optional_sv(&scheme.code_token_url),
                optional_sv(&scheme.device_url),
                optional_sv(&scheme.device_token_url),
                optional_sv(&scheme.revocation_url),
                optional_sv(&scheme.introspection_url),
                optional_sv(&scheme.discovery_url),
            ));
        } else {
            out.push_str(&format!(
                "    {{{}, {}, {}, {}, std::chrono::seconds{{{}}}, {}, {}, {}, {}, {}, {}, {}, {}, {}}},\n",
                sv(&scheme.name),
                scheme.client_secret_basic,
                optional_sv(&scheme.client_id_env),
                optional_sv(&scheme.client_secret_env),
                scheme.skew_seconds,
                sv(&scheme.issuer),
                optional_sv(&scheme.refresh_url),
                optional_sv(&scheme.client_credentials_url),
                optional_sv(&scheme.authorization_url),
                optional_sv(&scheme.code_token_url),
                optional_sv(&scheme.device_url),
                optional_sv(&scheme.device_token_url),
                optional_sv(&scheme.revocation_url),
                optional_sv(&scheme.introspection_url),
            ));
        }
    }
    out.push_str(DESCRIPTOR_HELPERS);
    out.push_str(PKCE_HELPERS);
    if discovery {
        out.push_str(DISCOVERY_HELPERS);
    }
    out.push_str("} // namespace detail\n\n");
    out
}

const DESCRIPTOR_TYPE: &str = r#"/// The compiled lifecycle descriptor of one source scheme. Empty members
/// mean "not compiled": endpoints are never invented and supplemental
/// endpoints exist only when configuration supplied them.
struct OAuthSchemeDescriptor {
    std::string_view name;
    /// Token-endpoint requests authenticate with the configured secret
    /// variable (client_secret_basic); otherwise the client is public.
    bool client_secret_basic = false;
    std::string_view client_id_env;
    std::string_view client_secret_env;
    /// Refresh-before-expiry clock skew.
    std::chrono::seconds skew{30};
    /// The issuing authority origin that store keys partition on.
    std::string_view issuer;
    /// The declared refresh endpoint, else the token endpoint.
    std::string_view refresh_url;
    std::string_view client_credentials_url;
    /// The declared authorization-request URL and the authorization-code
    /// flow's token endpoint; empty when the flow is absent.
    std::string_view authorization_url;
    std::string_view code_token_url;
    std::string_view device_url;
    std::string_view device_token_url;
    /// Configuration-supplied supplemental endpoints.
    std::string_view revocation_url;
    std::string_view introspection_url;
};

/// The usable source schemes. Schemes whose only declared flows are implicit
/// or password are never compiled here, so they emit nothing.
"#;

/// The descriptor struct head with the discovery URL field, emitted when at
/// least one compiled scheme carries a discovery URL.
const DESCRIPTOR_TYPE_DISCOVERY: &str = r#"/// The compiled lifecycle descriptor of one source scheme. Empty members
/// mean "not compiled": endpoints are never invented and supplemental
/// endpoints exist only when configuration supplied them. A compiled
/// discovery URL resolves the endpoint URLs the compiled plan omits at call
/// time, through RFC 8414 / OpenID Connect discovery.
struct OAuthSchemeDescriptor {
    std::string_view name;
    /// Token-endpoint requests authenticate with the configured secret
    /// variable (client_secret_basic); otherwise the client is public.
    bool client_secret_basic = false;
    std::string_view client_id_env;
    std::string_view client_secret_env;
    /// Refresh-before-expiry clock skew.
    std::chrono::seconds skew{30};
    /// The issuing authority origin that store keys partition on.
    std::string_view issuer;
    /// The declared refresh endpoint, else the token endpoint.
    std::string_view refresh_url;
    std::string_view client_credentials_url;
    /// The declared authorization-request URL and the authorization-code
    /// flow's token endpoint; empty when the flow is absent.
    std::string_view authorization_url;
    std::string_view code_token_url;
    std::string_view device_url;
    std::string_view device_token_url;
    /// Configuration-supplied supplemental endpoints.
    std::string_view revocation_url;
    std::string_view introspection_url;
    /// The known discovery/metadata document; empty when the compiled plan
    /// carries none.
    std::string_view discovery_url;
};

/// The usable source schemes. Schemes whose only declared flows are implicit
/// or password are never compiled here, so they emit nothing. A scheme with
/// no executable flow is compiled only when its discovery URL defines the
/// endpoints at runtime.
"#;

/// The RFC 8414 / OpenID Connect discovery engine's detail half: the reduced
/// document type, the bounded ceiling, the exact origin rule and the member
/// usability check. The typed failures live on the session type, which owns
/// the transport and the per-instance cache.
const DISCOVERY_HELPERS: &str = r#"
/// One RFC 8414 / OpenID Connect discovery document reduced to the endpoints
/// this runtime resolves. Unknown members are ignored; a present member must
/// be a nonempty string without control characters.
struct OAuthDiscoveredEndpoints {
    std::string token_endpoint;
    std::string revocation_endpoint;
    std::string introspection_endpoint;
};

/// The compiled ceiling for one discovery document response (about a
/// mebibyte).
inline constexpr std::size_t oauth_discovery_max_bytes = 1 << 20;

/// Whether one discovered endpoint value is unusable: empty or carrying
/// control characters.
inline bool oauth_discovery_unusable(std::string_view value) {
    if (value.empty()) return true;
    for (const char character : value) {
        const auto byte = static_cast<unsigned char>(character);
        if (byte <= 0x1f || byte == 0x7f) return true;
    }
    return false;
}

/// The origin of one absolute http(s) URL: scheme, host and the port with the
/// scheme default made explicit. Empty when the value is not an absolute
/// http(s) URL. Both sides of the issuer comparison pass through this exact
/// rule.
inline std::string oauth_url_origin(std::string_view url) {
    const auto scheme_end = url.find("://");
    if (scheme_end == std::string_view::npos || scheme_end == 0) return {};
    const auto lower = [](const std::string_view text) {
        std::string out;
        out.reserve(text.size());
        for (const char character : text) {
            out.push_back(character >= 'A' && character <= 'Z'
                ? static_cast<char>(character - 'A' + 'a')
                : character);
        }
        return out;
    };
    const std::string scheme = lower(url.substr(0, scheme_end));
    if (scheme != "http" && scheme != "https") return {};
    std::string_view authority = url.substr(scheme_end + 3);
    if (const auto stop = authority.find_first_of("/?#"); stop != std::string_view::npos) {
        authority = authority.substr(0, stop);
    }
    if (const auto at = authority.rfind('@'); at != std::string_view::npos) {
        authority = authority.substr(at + 1);
    }
    std::string_view host = authority;
    std::string_view port;
    if (!host.empty() && host.front() == '[') {
        // An RFC 3986 IP-literal host runs to the closing bracket.
        const auto close = host.find(']');
        if (close == std::string_view::npos) return {};
        const auto after = host.substr(close + 1);
        if (!after.empty()) {
            if (after.front() != ':') return {};
            port = after.substr(1);
        }
        host = host.substr(0, close + 1);
    } else if (const auto colon = host.rfind(':'); colon != std::string_view::npos) {
        port = host.substr(colon + 1);
        host = host.substr(0, colon);
    }
    if (host.empty()) return {};
    for (const char character : port) {
        if (character < '0' || character > '9') return {};
    }
    if (port.empty()) port = scheme == "http" ? std::string_view("80") : std::string_view("443");
    std::string origin = scheme;
    origin += "://";
    origin += lower(host);
    origin.push_back(':');
    origin += port;
    return origin;
}
"#;

const DESCRIPTOR_HELPERS: &str = r#"};

/// The compiled descriptor for one source scheme name; nullptr when the name
/// is not compiled into this package.
inline const OAuthSchemeDescriptor* oauth_scheme(std::string_view name) {
    for (const auto& scheme : oauth_schemes) {
        if (scheme.name == name) return &scheme;
    }
    return nullptr;
}

/// Bounded response ceiling for one token/device/revocation/introspection
/// response body.
inline constexpr std::size_t oauth_max_response_bytes = 1 << 20;

/// Decodes one bounded JSON object response body; anything else is an invalid
/// lifecycle response.
inline std::optional<JsonValue::Object> oauth_object(std::string_view body) {
    auto parsed = parse_json(body, JsonLimits{});
    if (!parsed) return std::nullopt;
    JsonValue value = std::move(parsed).value();
    if (!value.is<JsonValue::Object>()) return std::nullopt;
    return std::move(value.as<JsonValue::Object>());
}

/// The string member `key`, when present and a string.
inline std::optional<std::string> oauth_text(const JsonValue::Object& object,
    std::string_view key) {
    const auto it = object.find(key);
    if (it == object.end() || !it->second.is<std::string>()) return std::nullopt;
    return it->second.as<std::string>();
}

/// The integral member `key`, when present and exactly integral.
inline std::optional<std::int64_t> oauth_integer(const JsonValue::Object& object,
    std::string_view key) {
    const auto it = object.find(key);
    if (it == object.end() || !it->second.is<JsonNumber>()) return std::nullopt;
    const auto& number = it->second.as<JsonNumber>();
    if (!number.is_integer()) return std::nullopt;
    const auto converted = JsonInteger::from_number(number);
    if (!converted) return std::nullopt;
    const auto value = converted.value().to_int64();
    if (!value) return std::nullopt;
    return value.value();
}

/// application/x-www-form-urlencoded with RFC 3986 unreserved bytes kept
/// literal and spaces as %20.
inline std::string oauth_form_value(std::string_view value) {
    static constexpr char hex[] = "0123456789ABCDEF";
    std::string out;
    out.reserve(value.size());
    for (const char raw : value) {
        const auto byte = static_cast<unsigned char>(raw);
        const bool unreserved = (byte >= 'A' && byte <= 'Z')
            || (byte >= 'a' && byte <= 'z') || (byte >= '0' && byte <= '9')
            || byte == '-' || byte == '.' || byte == '_' || byte == '~';
        if (unreserved) {
            out.push_back(raw);
        } else {
            out.push_back('%');
            out.push_back(hex[byte >> 4]);
            out.push_back(hex[byte & 0x0F]);
        }
    }
    return out;
}

"#;

/// SHA-256 self-test, entropy draws, base64url encoding, the S256 challenge
/// derivation and the constant-time comparison behind the PKCE sections.
const PKCE_HELPERS: &str = r#"
/// FIPS 180-4 SHA-256, dependency-free, used only for the PKCE S256 code
/// challenge and its first-use self-test. No third-party dependency.
inline std::array<unsigned char, 32> oauth_sha256(std::string_view message) {
    static const std::uint32_t k[64] = {
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
    };
    std::uint32_t state[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
    std::string padded(message);
    padded.push_back(static_cast<char>(0x80));
    while (padded.size() % 64 != 56) padded.push_back('\0');
    const std::uint64_t bits = static_cast<std::uint64_t>(message.size()) * 8;
    for (int shift = 56; shift >= 0; shift -= 8) {
        padded.push_back(static_cast<char>((bits >> shift) & 0xFFU));
    }
    for (std::size_t at = 0; at < padded.size(); at += 64) {
        std::uint32_t w[64];
        for (int i = 0; i < 16; ++i) {
            w[i] = (static_cast<std::uint32_t>(static_cast<unsigned char>(padded[at + i * 4])) << 24)
                | (static_cast<std::uint32_t>(static_cast<unsigned char>(padded[at + i * 4 + 1])) << 16)
                | (static_cast<std::uint32_t>(static_cast<unsigned char>(padded[at + i * 4 + 2])) << 8)
                | static_cast<std::uint32_t>(static_cast<unsigned char>(padded[at + i * 4 + 3]));
        }
        for (int i = 16; i < 64; ++i) {
            const std::uint32_t s0 = ((w[i - 15] >> 7) | (w[i - 15] << 25))
                ^ ((w[i - 15] >> 18) | (w[i - 15] << 14)) ^ (w[i - 15] >> 3);
            const std::uint32_t s1 = ((w[i - 2] >> 17) | (w[i - 2] << 15))
                ^ ((w[i - 2] >> 19) | (w[i - 2] << 13)) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        std::uint32_t a = state[0], b = state[1], c = state[2], d = state[3];
        std::uint32_t e = state[4], f = state[5], g = state[6], h = state[7];
        for (int i = 0; i < 64; ++i) {
            const std::uint32_t s1 = ((e >> 6) | (e << 26)) ^ ((e >> 11) | (e << 21))
                ^ ((e >> 25) | (e << 7));
            const std::uint32_t choose = (e & f) ^ (~e & g);
            const std::uint32_t temp1 = h + s1 + choose + k[i] + w[i];
            const std::uint32_t s0 = ((a >> 2) | (a << 30)) ^ ((a >> 13) | (a << 19))
                ^ ((a >> 22) | (a << 10));
            const std::uint32_t majority = (a & b) ^ (a & c) ^ (b & c);
            const std::uint32_t temp2 = s0 + majority;
            h = g; g = f; f = e; e = d + temp1;
            d = c; c = b; b = a; a = temp1 + temp2;
        }
        state[0] += a; state[1] += b; state[2] += c; state[3] += d;
        state[4] += e; state[5] += f; state[6] += g; state[7] += h;
    }
    std::array<unsigned char, 32> digest{};
    for (int i = 0; i < 8; ++i) {
        digest[i * 4] = static_cast<unsigned char>((state[i] >> 24) & 0xFFU);
        digest[i * 4 + 1] = static_cast<unsigned char>((state[i] >> 16) & 0xFFU);
        digest[i * 4 + 2] = static_cast<unsigned char>((state[i] >> 8) & 0xFFU);
        digest[i * 4 + 3] = static_cast<unsigned char>(state[i] & 0xFFU);
    }
    return digest;
}

/// Runs the FIPS 180-4 'abc' test vector once per process; a mismatched
/// digest means the compiled hash is unusable and PKCE is refused rather
/// than minting challenges with broken hashing.
inline bool oauth_sha256_self_test() {
    static const bool ok = [] {
        static constexpr unsigned char expected[32] = {
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde,
            0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
            0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
        };
        const auto digest = oauth_sha256("abc");
        return std::equal(digest.begin(), digest.end(), expected);
    }();
    return ok;
}

/// Draws `count` bytes from std::random_device. The C++ standard routes
/// random_device to the operating system's entropy source on the toolchains
/// this runtime targets (libstdc++ reads /dev/urandom or getrandom(2), libc++
/// uses getrandom/arc4random, MSVC uses the process RTL CSPRNG); a conforming
/// implementation is a cryptographic generator or refuses to run. A known
/// deterministic implementation (historically some MinGW builds) is detected
/// here: two successive tokens of a real generator essentially never repeat,
/// so any repeated draw is refused instead of minting predictable PKCE
/// material.
inline bool oauth_random_bytes(unsigned char* out, std::size_t count) {
    std::size_t filled = 0;
    std::uint32_t previous = 0;
    bool has_previous = false;
    try {
        std::random_device device;
        while (filled < count) {
            const std::uint32_t token = device();
            if (has_previous && token == previous) return false;
            previous = token;
            has_previous = true;
            for (int shift = 24; shift >= 0 && filled < count; shift -= 8) {
                out[filled++] = static_cast<unsigned char>((token >> shift) & 0xFFU);
            }
        }
    } catch (...) {
        // A random_device that cannot deliver entropy refuses to run.
        return false;
    }
    return true;
}

/// RFC 4648 base64url without padding: 32 bytes encode to 43 characters,
/// every one an RFC 7636 unreserved byte.
inline std::string oauth_base64url(const unsigned char* bytes, std::size_t count) {
    static constexpr char alphabet[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    std::string out;
    out.reserve((count * 8 + 5) / 6);
    std::size_t at = 0;
    while (at + 3 <= count) {
        const std::uint32_t group = (static_cast<std::uint32_t>(bytes[at]) << 16)
            | (static_cast<std::uint32_t>(bytes[at + 1]) << 8) | bytes[at + 2];
        out.push_back(alphabet[(group >> 18) & 0x3FU]);
        out.push_back(alphabet[(group >> 12) & 0x3FU]);
        out.push_back(alphabet[(group >> 6) & 0x3FU]);
        out.push_back(alphabet[group & 0x3FU]);
        at += 3;
    }
    if (count - at == 1) {
        const std::uint32_t group = static_cast<std::uint32_t>(bytes[at]) << 16;
        out.push_back(alphabet[(group >> 18) & 0x3FU]);
        out.push_back(alphabet[(group >> 12) & 0x3FU]);
    } else if (count - at == 2) {
        const std::uint32_t group = (static_cast<std::uint32_t>(bytes[at]) << 16)
            | (static_cast<std::uint32_t>(bytes[at + 1]) << 8);
        out.push_back(alphabet[(group >> 18) & 0x3FU]);
        out.push_back(alphabet[(group >> 12) & 0x3FU]);
        out.push_back(alphabet[(group >> 6) & 0x3FU]);
    }
    return out;
}

/// 32 bytes of entropy in the RFC 7636 base64url alphabet: 43 characters.
/// nullopt means the platform entropy source is unusable or deterministic.
inline std::optional<std::string> oauth_random_value() {
    unsigned char bytes[32];
    if (!oauth_random_bytes(bytes, sizeof(bytes))) return std::nullopt;
    return oauth_base64url(bytes, sizeof(bytes));
}

/// Constant-time byte equality: every difference folds into one accumulator,
/// so the comparison cost does not depend on where a mismatch sits. Lengths
/// are not secret (both states are 43-character values), so unequal lengths
/// return early.
inline bool oauth_constant_time_equal(std::string_view left, std::string_view right) {
    if (left.size() != right.size()) return false;
    unsigned char difference = 0;
    for (std::size_t at = 0; at < left.size(); ++at) {
        difference |= static_cast<unsigned char>(left[at])
            ^ static_cast<unsigned char>(right[at]);
    }
    return difference == 0;
}

/// The RFC 7636 S256 PKCE pair: a random 43-character verifier and its
/// base64url SHA-256 challenge. nullopt means the platform entropy source or
/// the hash self-test failed and no verifier may be minted.
inline std::optional<std::pair<std::string, std::string>> oauth_pkce() {
    if (!oauth_sha256_self_test()) return std::nullopt;
    auto verifier = oauth_random_value();
    if (!verifier) return std::nullopt;
    const auto digest = oauth_sha256(*verifier);
    return std::make_pair(std::move(*verifier), oauth_base64url(digest.data(), digest.size()));
}
"#;

/// Token set, store, typed error and options, the conditional value types,
/// and the session type carrying acquisition, refresh and the shared request
/// plumbing. Each method is emitted exactly when some compiled scheme can
/// execute it. Plans without a discovery URL assemble byte-identically to the
/// pre-discovery bytes; plans with one emit the discovery-aware endpoint
/// resolution and the discovery engine.
fn core(oauth: &OAuthPlan) -> String {
    let discovery = has_discovery(oauth);
    let mut out = String::from(CORE_VALUE_TYPES);
    out.push_str(if discovery {
        ERROR_TYPE_DISCOVERY
    } else {
        ERROR_TYPE_PLAIN
    });
    out.push_str(CORE_OPTIONS);
    if has_authorization(oauth) {
        out.push_str(AUTHORIZATION_TYPE);
    }
    if has_device(oauth) {
        out.push_str(DEVICE_TYPE);
    }
    if has_introspection(oauth) || discovery {
        out.push_str(INTROSPECTION_TYPE);
    }
    out.push_str(SESSIONS_HEAD);
    if oauth
        .names
        .replay_credentials
        .as_ref()
        .is_some_and(|name| !name.is_empty())
    {
        out.push_str(REPLAY_STORE_KEY);
    }
    if discovery {
        out.push_str(CLIENT_CREDENTIALS_METHOD_DISCOVERY);
        out.push_str(REFRESH_METHOD_DISCOVERY);
    } else {
        if oauth
            .schemes
            .iter()
            .any(|scheme| scheme.client_credentials_url.is_some())
        {
            out.push_str(CLIENT_CREDENTIALS_METHOD);
        }
        if oauth.schemes.iter().any(|scheme| {
            scheme
                .refresh_url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
        }) {
            out.push_str(REFRESH_METHOD);
        }
    }
    if has_authorization(oauth) {
        out.push_str(AUTHORIZATION_METHODS);
    }
    if has_device(oauth) {
        out.push_str(DEVICE_METHODS);
    }
    if discovery {
        out.push_str(REVOCATION_METHOD_DISCOVERY);
        out.push_str(INTROSPECTION_METHOD_DISCOVERY);
    } else {
        if has_revocation(oauth) {
            out.push_str(REVOCATION_METHOD);
        }
        if has_introspection(oauth) {
            out.push_str(INTROSPECTION_METHOD);
        }
    }
    out.push_str(SESSIONS_TAIL);
    if discovery {
        out.push_str(DISCOVERY_STATE);
    }
    out.push_str(SESSIONS_TAIL_CLOSE);
    out
}

const CORE_VALUE_TYPES: &str = r#"/// One issued token and its metadata, exactly as the compiled token endpoint
/// returned it (RFC 6749 section 5.1). Every member is credential material:
/// never print, log or stream token values; `describe()` is the safe summary.
struct @TOKEN_SET@ {
    /// The access token.
    std::string access_token;
    /// The server's token type, or the conventional Bearer.
    std::string token_type = "Bearer";
    /// Issue time plus the returned expires_in. A default-constructed value
    /// means the server declared no lifetime, so the set never expires
    /// locally.
    std::chrono::system_clock::time_point expires_at{};
    /// The server's rotated refresh token, or the previous set's token when
    /// the server returned none.
    std::string refresh_token;
    /// Whether a refresh token is present (an empty refresh token value is a
    /// legal token).
    bool has_refresh = false;
    /// The granted scope, as the server returned it.
    std::string scope;

    /// Skew-aware staleness: whether now has reached the expiry minus the
    /// compiled clock skew.
    [[nodiscard]] bool expired(std::chrono::seconds skew) const {
        if (expires_at == std::chrono::system_clock::time_point{}) return false;
        return std::chrono::system_clock::now() >= expires_at - skew;
    }

    /// The explicit credential value carried on requests: the server's token
    /// type, or Bearer when the response omitted one.
    [[nodiscard]] Authorization authorization() const {
        return Authorization(token_type.empty() ? std::string("Bearer") : token_type, access_token);
    }

    /// Safe single-line summary carrying no credential material.
    [[nodiscard]] std::string describe() const {
        std::string text = "TokenSet(has_refresh=";
        text += has_refresh ? "true" : "false";
        text += ", scope=";
        text += scope;
        text += ")";
        return text;
    }
};

/// Caller-owned token storage behind the generated lifecycle. Keys are
/// partitioned by source scheme, token-endpoint issuer and client identity;
/// treat them as read-only routing information. `replace` is atomic per key.
/// v1 admits in-process store semantics only: loading simply finds nothing
/// when a key is absent.
class @TOKEN_STORE@ {
public:
    virtual ~@TOKEN_STORE@() = default;
    @TOKEN_STORE@(const @TOKEN_STORE@&) = delete;
    @TOKEN_STORE@& operator=(const @TOKEN_STORE@&) = delete;
    /// The stored token for `key`, or an empty optional.
    virtual std::optional<@TOKEN_SET@> load(const std::string& key) = 0;
    /// Atomically replace the stored token for `key`.
    virtual void replace(const std::string& key, @TOKEN_SET@ set) = 0;
    /// Drop the stored token for `key`; clearing an absent key succeeds.
    virtual void clear(const std::string& key) = 0;
protected:
    @TOKEN_STORE@() = default;
};

/// Instance-owned in-process token store guarded by a mutex. Each instance
/// guards its own keys; generated code never shares a store implicitly and
/// there is no process-global token cache.
class @MEMORY_STORE@ final : public @TOKEN_STORE@ {
public:
    @MEMORY_STORE@() = default;
    std::optional<@TOKEN_SET@> load(const std::string& key) override {
        std::lock_guard<std::mutex> guard(mutex_);
        const auto it = sets_.find(key);
        if (it == sets_.end()) return std::nullopt;
        return it->second;
    }
    void replace(const std::string& key, @TOKEN_SET@ set) override {
        std::lock_guard<std::mutex> guard(mutex_);
        sets_.insert_or_assign(key, std::move(set));
    }
    void clear(const std::string& key) override {
        std::lock_guard<std::mutex> guard(mutex_);
        sets_.erase(key);
    }
private:
    mutable std::mutex mutex_;
    std::map<std::string, @TOKEN_SET@> sets_;
};

"#;

const ERROR_TYPE_PLAIN: &str = r#"/// Typed OAuth lifecycle failure. `kind` and `scheme` classify it, `code`
/// carries the authorization server's declared error code and `status` the
/// endpoint HTTP status when one was reached. Messages and fields never
/// contain token or client-secret values, and never response bodies.
class @ERROR@ {
public:
    enum class Kind {
        /// The scheme name is not compiled into this package.
        UnknownScheme,
        /// The compiled plan has no executable endpoint for this step.
        UnsupportedFlow,
        /// Required client credentials are absent or empty.
        MissingCredentials,
        /// The given token set carries no refresh token to exchange.
        MissingRefreshToken,
        /// A token value argument is empty.
        InvalidToken,
        /// Transport failure, deadlines or cancellation.
        Transport,
        /// The endpoint response could not be decoded as RFC 6749 requires.
        InvalidResponse,
        /// The endpoint rejected the grant; `code` carries its declared error.
        ServerRejected,
        /// The endpoint answered outside the accepted status range.
        ServerError,
        /// A response body exceeded the bounded ceiling.
        ResourceLimit,
        /// The device grant's declared lifetime passed while polling.
        Expired,
        /// PKCE entropy was refused: the platform entropy source failed or
        /// behaved deterministically.
        Entropy,
        /// The callback state does not match the transaction's bound state.
        StateMismatch,
        /// The transaction was already consumed by an earlier completion
        /// attempt.
        TransactionUsed,
        /// The resource owner (or server) declared an authorization error;
        /// `code` carries its declared error.
        AuthorizationDenied,
    };
    Kind kind = Kind::Transport;
    std::string scheme;
    std::string code;
    int status = 0;

    /// Stable kind name for callers that classify failures textually.
    [[nodiscard]] static std::string_view kind_name(Kind kind) {
        switch (kind) {
            case Kind::UnknownScheme: return "unknown-scheme";
            case Kind::UnsupportedFlow: return "unsupported-flow";
            case Kind::MissingCredentials: return "missing-client-credentials";
            case Kind::MissingRefreshToken: return "no-refresh-token";
            case Kind::InvalidToken: return "invalid-token";
            case Kind::Transport: return "transport";
            case Kind::InvalidResponse: return "invalid-response";
            case Kind::ServerRejected: return "server-rejected";
            case Kind::ServerError: return "server-error";
            case Kind::ResourceLimit: return "resource-limit";
            case Kind::Expired: return "device-flow-expired";
            case Kind::Entropy: return "entropy-source";
            case Kind::StateMismatch: return "state-mismatch";
            case Kind::TransactionUsed: return "transaction-used";
            case Kind::AuthorizationDenied: return "authorization-denied";
        }
        return "transport";
    }

    /// Safe message carrying only the classification metadata.
    [[nodiscard]] std::string message() const {
        std::string text = "oauth ";
        text += kind_name(kind);
        text += " failure";
        if (!scheme.empty()) {
            text += " for scheme ";
            text += scheme;
        }
        if (!code.empty()) {
            text += ": ";
            text += code;
        }
        return text;
    }
};

"#;

/// The typed failure with the discovery-failed kind, emitted only when at
/// least one compiled scheme carries a discovery URL.
const ERROR_TYPE_DISCOVERY: &str = r#"/// Typed OAuth lifecycle failure. `kind` and `scheme` classify it, `code`
/// carries the authorization server's declared error code and `status` the
/// endpoint HTTP status when one was reached. Messages and fields never
/// contain token or client-secret values, and never response bodies; a
/// discovery failure never carries response body text either.
class @ERROR@ {
public:
    enum class Kind {
        /// The scheme name is not compiled into this package.
        UnknownScheme,
        /// The compiled plan has no executable endpoint for this step and the
        /// discovery document (when the scheme compiles one) declares none.
        UnsupportedFlow,
        /// Required client credentials are absent or empty.
        MissingCredentials,
        /// The given token set carries no refresh token to exchange.
        MissingRefreshToken,
        /// A token value argument is empty.
        InvalidToken,
        /// Transport failure, deadlines or cancellation.
        Transport,
        /// The endpoint response could not be decoded as RFC 6749 requires.
        InvalidResponse,
        /// The endpoint rejected the grant; `code` carries its declared error.
        ServerRejected,
        /// The endpoint answered outside the accepted status range.
        ServerError,
        /// A response body exceeded the bounded ceiling.
        ResourceLimit,
        /// The device grant's declared lifetime passed while polling.
        Expired,
        /// PKCE entropy was refused: the platform entropy source failed or
        /// behaved deterministically.
        Entropy,
        /// The callback state does not match the transaction's bound state.
        StateMismatch,
        /// The transaction was already consumed by an earlier completion
        /// attempt.
        TransactionUsed,
        /// The resource owner (or server) declared an authorization error;
        /// `code` carries its declared error.
        AuthorizationDenied,
        /// The discovery document request failed, the document was unusable,
        /// or its `issuer` claim did not share the discovery URL's origin.
        DiscoveryFailed,
    };
    Kind kind = Kind::Transport;
    std::string scheme;
    std::string code;
    int status = 0;

    /// Stable kind name for callers that classify failures textually.
    [[nodiscard]] static std::string_view kind_name(Kind kind) {
        switch (kind) {
            case Kind::UnknownScheme: return "unknown-scheme";
            case Kind::UnsupportedFlow: return "unsupported-flow";
            case Kind::MissingCredentials: return "missing-client-credentials";
            case Kind::MissingRefreshToken: return "no-refresh-token";
            case Kind::InvalidToken: return "invalid-token";
            case Kind::Transport: return "transport";
            case Kind::InvalidResponse: return "invalid-response";
            case Kind::ServerRejected: return "server-rejected";
            case Kind::ServerError: return "server-error";
            case Kind::ResourceLimit: return "resource-limit";
            case Kind::Expired: return "device-flow-expired";
            case Kind::Entropy: return "entropy-source";
            case Kind::StateMismatch: return "state-mismatch";
            case Kind::TransactionUsed: return "transaction-used";
            case Kind::AuthorizationDenied: return "authorization-denied";
            case Kind::DiscoveryFailed: return "discovery-failed";
        }
        return "transport";
    }

    /// Safe message carrying only the classification metadata.
    [[nodiscard]] std::string message() const {
        std::string text = "oauth ";
        text += kind_name(kind);
        text += " failure";
        if (!scheme.empty()) {
            text += " for scheme ";
            text += scheme;
        }
        if (!code.empty()) {
            text += ": ";
            text += code;
        }
        return text;
    }
};

"#;

const CORE_OPTIONS: &str = r#"/// Explicit inputs for one token call. Explicit values win over the compiled
/// environment variable names; neither generation nor this runtime ever bakes
/// credential values into emitted bytes.
struct @OPTIONS@ {
    /// Explicit client identity, overriding the compiled variable.
    std::string client_id;
    /// Explicit client secret, overriding the compiled variable.
    std::string client_secret;
    /// Explicit store, overriding the owning session's instance store.
    std::shared_ptr<@TOKEN_STORE@> store;
    /// Total per-endpoint deadline.
    std::chrono::milliseconds timeout{30000};
    /// Cancellation observed before each request and between polling rounds.
    std::stop_token stop;
    /// Replaces the sleeper for device polling; tests pass a no-op.
    std::function<void(std::chrono::milliseconds)> wait;
};

"#;

const AUTHORIZATION_TYPE: &str = r#"/// One started authorization-code + PKCE S256 transaction. Direct the
/// resource owner to `authorization_url`, then complete the transaction with
/// the callback's query parameters exactly once. The verifier and the
/// one-time consumption gate are bound to the transaction value itself, so a
/// replay through any copy is refused whatever the first attempt returned.
struct @AUTHORIZATION@ {
    /// The source scheme this transaction belongs to.
    std::string scheme;
    /// The complete authorization request: response type, client id,
    /// redirect URI, scope, the one-time state and the S256 challenge.
    std::string authorization_url;
    /// The one-time state the callback must repeat exactly.
    std::string state;
    /// The RFC 7636 S256 challenge derived from the verifier.
    std::string code_challenge;
    std::string code_challenge_method = "S256";
    /// The redirect URI bound at start; repeated exactly in the exchange.
    std::string redirect_uri;
    /// The requested scopes, in the order given.
    std::vector<std::string> scopes;
    /// When the transaction started. Lifetime policy belongs to the
    /// authorization server.
    std::chrono::system_clock::time_point created_at{};
    /// The PKCE verifier. Credential material: never print, log or stream it.
    std::string code_verifier;

    /// Whether this transaction was consumed by an earlier completion attempt
    /// (successful or not). Copies share the gate.
    [[nodiscard]] bool consumed() const {
        std::lock_guard<std::mutex> guard(gate_->mutex);
        return gate_->consumed;
    }

private:
    /// The shared one-time consumption gate. Copies of the transaction share
    /// it, so completing any copy burns the transaction.
    struct Gate {
        mutable std::mutex mutex;
        bool consumed = false;
    };
    friend class @SESSIONS@;
    std::shared_ptr<Gate> gate_ = std::make_shared<Gate>();
};

"#;

const DEVICE_TYPE: &str = r#"/// One started RFC 8628 device-authorization transaction. Present the user
/// code and verification URI, then poll the token endpoint for the granted
/// set.
struct @DEVICE@ {
    /// The source scheme this grant belongs to.
    std::string scheme;
    /// The code the user enters at the verification URI.
    std::string user_code;
    /// Where the user approves the device grant.
    std::string verification_uri;
    /// When declared, carries the user code in the URL.
    std::string verification_uri_complete;
    /// The server-declared transaction expiry; a default value means the
    /// server declared no lifetime and only its own expired_token answer
    /// bounds polling.
    std::chrono::system_clock::time_point expires_at{};
    /// The polling floor; the server's slow_down answers extend it.
    std::chrono::milliseconds interval{5000};
    /// Credential material: never print, log or stream.
    std::string device_code;
};

"#;

const INTROSPECTION_TYPE: &str = r#"/// One RFC 7662 introspection response. It describes the token without
/// returning it. Numeric fields are server epochs; zero means the server
/// omitted the claim.
struct @INTROSPECTION@ {
    bool active = false;
    std::string scope;
    std::string client_id;
    std::string token_type;
    std::string username;
    std::int64_t expires_at = 0;
    std::int64_t issued_at = 0;
    std::int64_t not_before = 0;
    std::string subject;
    std::vector<std::string> audience;
    std::string issuer;
    std::string jwt_id;
};

"#;

const SESSIONS_HEAD: &str = r#"/// One owner's generated OAuth lifecycle: an instance-owned default token
/// store, per-key single-flight acquisition gates and the compiled endpoints.
/// Every token value lives only inside a store instance; there is no
/// process-global token cache and no global mutable state.
class @SESSIONS@ {
public:
    explicit @SESSIONS@(std::shared_ptr<const Transport> transport,
        std::shared_ptr<@TOKEN_STORE@> store = {})
        : transport_(std::move(transport)), store_(std::move(store)) {}

    /// Whether the scheme name is compiled into this package.
    [[nodiscard]] static bool compiled(std::string_view scheme) {
        return detail::oauth_scheme(scheme) != nullptr;
    }
"#;

const CLIENT_CREDENTIALS_METHOD: &str = r#"
    /// The scheme's cached token set, acquiring one from the compiled
    /// client-credentials endpoint when the stored set is absent or expired
    /// beyond the compiled skew. Concurrent callers on one owner share a
    /// single acquisition: callers waiting on the gate re-read the store
    /// instead of acquiring again.
    Result<@TOKEN_SET@, @ERROR@> client_credentials_token(const std::string& scheme,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->client_credentials_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        const auto tokens = store_for(options);
        const std::string key = store_key(*descriptor, options);
        for (;;) {
            if (auto cached = fresh(tokens, key, *descriptor)) {
                return Result<@TOKEN_SET@, @ERROR@>::success(std::move(*cached));
            }
            std::shared_ptr<Round> round;
            bool proceed = false;
            {
                std::lock_guard<std::mutex> guard(gates_mutex_);
                auto& slot = gates_[key];
                if (!slot) {
                    slot = std::make_shared<Round>();
                    slot->claimed = true;
                    proceed = true;
                }
                round = slot;
            }
            if (!proceed) {
                // The round holder owns this acquisition: wait for its
                // completion, then re-probe the store for the set it stored.
                std::unique_lock<std::mutex> waiter(round->mutex);
                round->signal.wait(waiter, [&round] { return round->done; });
                continue;
            }
            // This caller owns the round: re-check the store once more (the
            // fast path ran before this caller was selected), acquire, store
            // and complete.
            if (auto cached = fresh(tokens, key, *descriptor)) {
                finish(key, round);
                return Result<@TOKEN_SET@, @ERROR@>::success(std::move(*cached));
            }
            const auto stored = tokens->load(key);
            std::map<std::string, std::string, std::less<>> fields{
                {"grant_type", "client_credentials"}};
            auto acquired = token_request(*descriptor,
                std::string(descriptor->client_credentials_url), std::move(fields),
                stored ? &stored.value() : nullptr, options);
            if (acquired) tokens->replace(key, acquired.value());
            finish(key, round);
            if (!acquired) return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(acquired).error());
            return Result<@TOKEN_SET@, @ERROR@>::success(std::move(acquired).value());
        }
    }
"#;

/// The discovery-aware client-credentials acquisition: the compiled
/// client-credentials endpoint always wins; otherwise the cached discovery
/// document's `token_endpoint` resolves the acquisition.
const CLIENT_CREDENTIALS_METHOD_DISCOVERY: &str = r#"
    /// The scheme's cached token set, acquiring one from the compiled
    /// client-credentials endpoint when the stored set is absent or expired
    /// beyond the compiled skew. The compiled endpoint always wins; when the
    /// compiled plan declares none, the scheme's cached discovery document's
    /// `token_endpoint` resolves the acquisition. Concurrent callers on one
    /// owner share a single acquisition: callers waiting on the gate re-read
    /// the store instead of acquiring again.
    Result<@TOKEN_SET@, @ERROR@> client_credentials_token(const std::string& scheme,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->client_credentials_url.empty() && descriptor->discovery_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        auto endpoint = resolve_endpoint(*descriptor, EndpointKind::Token, options);
        if (!endpoint) return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(endpoint).error());
        const auto tokens = store_for(options);
        const std::string key = store_key(*descriptor, options);
        for (;;) {
            if (auto cached = fresh(tokens, key, *descriptor)) {
                return Result<@TOKEN_SET@, @ERROR@>::success(std::move(*cached));
            }
            std::shared_ptr<Round> round;
            bool proceed = false;
            {
                std::lock_guard<std::mutex> guard(gates_mutex_);
                auto& slot = gates_[key];
                if (!slot) {
                    slot = std::make_shared<Round>();
                    slot->claimed = true;
                    proceed = true;
                }
                round = slot;
            }
            if (!proceed) {
                // The round holder owns this acquisition: wait for its
                // completion, then re-probe the store for the set it stored.
                std::unique_lock<std::mutex> waiter(round->mutex);
                round->signal.wait(waiter, [&round] { return round->done; });
                continue;
            }
            // This caller owns the round: re-check the store once more (the
            // fast path ran before this caller was selected), acquire, store
            // and complete.
            if (auto cached = fresh(tokens, key, *descriptor)) {
                finish(key, round);
                return Result<@TOKEN_SET@, @ERROR@>::success(std::move(*cached));
            }
            const auto stored = tokens->load(key);
            std::map<std::string, std::string, std::less<>> fields{
                {"grant_type", "client_credentials"}};
            auto acquired = token_request(*descriptor, endpoint.value(), std::move(fields),
                stored ? &stored.value() : nullptr, options);
            if (acquired) tokens->replace(key, acquired.value());
            finish(key, round);
            if (!acquired) return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(acquired).error());
            return Result<@TOKEN_SET@, @ERROR@>::success(std::move(acquired).value());
        }
    }
"#;

const REFRESH_METHOD: &str = r#"
    /// Exchange the set's refresh token (RFC 6749 section 6) at the scheme's
    /// declared refresh endpoint, or its token endpoint when none is
    /// declared. The returned set adopts a rotated refresh token and retains
    /// the given one otherwise. The store is neither read nor updated:
    /// callers decide which set to keep.
    Result<@TOKEN_SET@, @ERROR@> refresh_token(const std::string& scheme, const @TOKEN_SET@& set,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (!set.has_refresh || set.refresh_token.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::MissingRefreshToken, scheme);
        }
        if (descriptor->refresh_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (options.stop.stop_requested()) return failure<@TOKEN_SET@>(@ERROR@::Kind::Transport, scheme);
        std::map<std::string, std::string, std::less<>> fields{
            {"grant_type", "refresh_token"}, {"refresh_token", set.refresh_token}};
        return token_request(*descriptor, std::string(descriptor->refresh_url),
            std::move(fields), &set, options);
    }
"#;

/// The discovery-aware refresh: the compiled refresh URL (else the compiled
/// token URL, already folded in at generation time) always wins; otherwise
/// the cached discovery document's `token_endpoint` resolves the exchange.
const REFRESH_METHOD_DISCOVERY: &str = r#"
    /// Exchange the set's refresh token (RFC 6749 section 6) at the scheme's
    /// declared refresh endpoint, or its token endpoint when none is
    /// declared. The compiled endpoint always wins; when the compiled plan
    /// declares none, the scheme's cached discovery document's
    /// `token_endpoint` resolves the exchange. The returned set adopts a
    /// rotated refresh token and retains the given one otherwise. The store
    /// is neither read nor updated: callers decide which set to keep.
    Result<@TOKEN_SET@, @ERROR@> refresh_token(const std::string& scheme, const @TOKEN_SET@& set,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (!set.has_refresh || set.refresh_token.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::MissingRefreshToken, scheme);
        }
        if (descriptor->refresh_url.empty() && descriptor->discovery_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (options.stop.stop_requested()) return failure<@TOKEN_SET@>(@ERROR@::Kind::Transport, scheme);
        auto endpoint = resolve_endpoint(*descriptor, EndpointKind::Refresh, options);
        if (!endpoint) return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(endpoint).error());
        std::map<std::string, std::string, std::less<>> fields{
            {"grant_type", "refresh_token"}, {"refresh_token", set.refresh_token}};
        return token_request(*descriptor, endpoint.value(),
            std::move(fields), &set, options);
    }
"#;

const DEVICE_METHODS: &str = r#"
    /// Starts a device-authorization transaction against the compiled device
    /// endpoint (RFC 8628 sections 3.1-3.2). One network call; present the
    /// returned user code and verification URI to the user.
    Result<@DEVICE@, @ERROR@> begin_device_authorization(const std::string& scheme,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@DEVICE@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->device_url.empty() || descriptor->device_token_url.empty()) {
            return failure<@DEVICE@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (options.stop.stop_requested()) return failure<@DEVICE@>(@ERROR@::Kind::Transport, scheme);
        const auto resolved = credentials(*descriptor, options);
        if (resolved.first.empty()) {
            return failure<@DEVICE@>(@ERROR@::Kind::MissingCredentials, scheme);
        }
        std::map<std::string, std::string, std::less<>> fields{{"client_id", resolved.first}};
        auto outcome = endpoint_request(*descriptor, descriptor->device_url,
            std::move(fields), options);
        if (!outcome) return Result<@DEVICE@, @ERROR@>::failure(std::move(outcome).error());
        const auto [status, body] = std::move(outcome).value();
        const auto object = detail::oauth_object(body);
        if (!object) {
            return failure<@DEVICE@>(@ERROR@::Kind::InvalidResponse, scheme, {}, status);
        }
        if (auto declared = detail::oauth_text(*object, "error");
            declared && !declared->empty()) {
            return failure<@DEVICE@>(@ERROR@::Kind::ServerRejected, scheme, std::move(*declared), status);
        }
        if (status < 200 || status >= 300) {
            return failure<@DEVICE@>(@ERROR@::Kind::ServerError, scheme, {}, status);
        }
        auto device_code = detail::oauth_text(*object, "device_code");
        auto user_code = detail::oauth_text(*object, "user_code");
        auto verification_uri = detail::oauth_text(*object, "verification_uri");
        auto expires_in = detail::oauth_integer(*object, "expires_in");
        if (!device_code || device_code->empty() || !user_code || user_code->empty()
            || !verification_uri || verification_uri->empty() || !expires_in || *expires_in <= 0) {
            return failure<@DEVICE@>(@ERROR@::Kind::InvalidResponse, scheme, {}, status);
        }
        @DEVICE@ grant;
        grant.scheme = scheme;
        grant.device_code = std::move(*device_code);
        grant.user_code = std::move(*user_code);
        grant.verification_uri = std::move(*verification_uri);
        if (auto complete = detail::oauth_text(*object, "verification_uri_complete")) {
            grant.verification_uri_complete = std::move(*complete);
        }
        grant.expires_at = std::chrono::system_clock::now() + std::chrono::seconds(*expires_in);
        if (auto interval = detail::oauth_integer(*object, "interval"); interval && *interval > 0) {
            grant.interval = std::chrono::milliseconds(*interval * 1000);
        }
        return Result<@DEVICE@, @ERROR@>::success(std::move(grant));
    }

    /// Polls the compiled token endpoint until the user approves the device
    /// grant, the transaction's declared expiry passes or cancellation is
    /// observed (RFC 8628 section 3.5). `authorization_pending` waits the
    /// declared interval and retries, `slow_down` extends the interval by
    /// five seconds per answer, and the granted set atomically replaces the
    /// resolved store's entry. `options.wait` replaces the sleeper.
    Result<@TOKEN_SET@, @ERROR@> poll_device_token(const @DEVICE@& grant,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(grant.scheme);
        if (descriptor == nullptr) return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, grant.scheme);
        if (descriptor->device_token_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, grant.scheme);
        }
        const auto tokens = store_for(options);
        const std::string key = store_key(*descriptor, options);
        std::chrono::milliseconds interval =
            grant.interval > std::chrono::milliseconds{0}
                ? grant.interval
                : std::chrono::milliseconds{5000};
        for (;;) {
            if (options.stop.stop_requested()) {
                return failure<@TOKEN_SET@>(@ERROR@::Kind::Transport, grant.scheme);
            }
            if (grant.expires_at != std::chrono::system_clock::time_point{}
                && std::chrono::system_clock::now() >= grant.expires_at) {
                return failure<@TOKEN_SET@>(@ERROR@::Kind::Expired, grant.scheme);
            }
            std::map<std::string, std::string, std::less<>> fields{
                {"grant_type", "urn:ietf:params:oauth:grant-type:device_code"},
                {"device_code", grant.device_code}};
            auto outcome = token_request(*descriptor, std::string(descriptor->device_token_url),
                std::move(fields), nullptr, options);
            if (outcome) {
                tokens->replace(key, outcome.value());
                return outcome;
            }
            @ERROR@ error = std::move(outcome).error();
            if (error.code == "authorization_pending") {
                wait_interval(interval, options);
                continue;
            }
            if (error.code == "slow_down") {
                interval += std::chrono::milliseconds{5000};
                wait_interval(interval, options);
                continue;
            }
            return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(error));
        }
    }
"#;

const AUTHORIZATION_METHODS: &str = r#"
    /// Starts an authorization-code + PKCE S256 transaction (RFC 6749
    /// section 4.1, RFC 7636 section 4). No network call: direct the resource
    /// owner to the returned authorization_url, then pass the callback's
    /// query parameters to complete_authorization. The client id resolves
    /// like every other client input — explicit options first, then the
    /// compiled variable read now — and a missing id is a typed failure
    /// because the authorization request cannot be built without one.
    Result<@AUTHORIZATION@, @ERROR@> begin_authorization(const std::string& scheme,
        std::string redirect_uri = {}, std::vector<std::string> scopes = {},
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) {
            return failure<@AUTHORIZATION@>(@ERROR@::Kind::UnknownScheme, scheme);
        }
        if (descriptor->authorization_url.empty() || descriptor->code_token_url.empty()) {
            return failure<@AUTHORIZATION@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        const auto resolved = credentials(*descriptor, options);
        if (resolved.first.empty()) {
            return failure<@AUTHORIZATION@>(@ERROR@::Kind::MissingCredentials, scheme);
        }
        // The authorization request is a browser redirect: it always carries
        // the client id, whatever the token-endpoint authentication method.
        auto pkce = detail::oauth_pkce();
        auto state = detail::oauth_random_value();
        if (!pkce || !state) {
            return failure<@AUTHORIZATION@>(@ERROR@::Kind::Entropy, scheme);
        }
        @AUTHORIZATION@ transaction;
        transaction.scheme = scheme;
        transaction.code_verifier = std::move(pkce->first);
        transaction.code_challenge = std::move(pkce->second);
        transaction.state = std::move(*state);
        transaction.redirect_uri = std::move(redirect_uri);
        transaction.scopes = std::move(scopes);
        transaction.created_at = std::chrono::system_clock::now();
        std::string url(descriptor->authorization_url);
        url.push_back(url.find('?') == std::string::npos ? '?' : '&');
        url += "response_type=code";
        url += "&client_id=" + detail::oauth_form_value(resolved.first);
        if (!transaction.redirect_uri.empty()) {
            url += "&redirect_uri=" + detail::oauth_form_value(transaction.redirect_uri);
        }
        url += "&state=" + detail::oauth_form_value(transaction.state);
        url += "&code_challenge=" + detail::oauth_form_value(transaction.code_challenge);
        url += "&code_challenge_method=S256";
        if (!transaction.scopes.empty()) {
            std::string scope;
            for (const auto& name : transaction.scopes) {
                if (!scope.empty()) scope.push_back(' ');
                scope += name;
            }
            url += "&scope=" + detail::oauth_form_value(scope);
        }
        transaction.authorization_url = std::move(url);
        return Result<@AUTHORIZATION@, @ERROR@>::success(std::move(transaction));
    }

    /// Completes the transaction exactly once with the callback's query
    /// parameters: the one-time state is compared in constant time, a
    /// server-declared error surfaces as a typed refusal, and the code is
    /// exchanged with the retained PKCE verifier at the compiled token
    /// endpoint under the scheme's compiled client authentication. The
    /// resulting set replaces the resolved store's entry. The first call
    /// consumes the transaction whatever its outcome; a second attempt is
    /// Kind::TransactionUsed.
    Result<@TOKEN_SET@, @ERROR@> complete_authorization(const @AUTHORIZATION@& transaction,
        const std::map<std::string, std::string, std::less<>>& callback,
        const @OPTIONS@& options = {}) {
        {
            std::lock_guard<std::mutex> guard(transaction.gate_->mutex);
            if (transaction.gate_->consumed) {
                return failure<@TOKEN_SET@>(@ERROR@::Kind::TransactionUsed, transaction.scheme);
            }
            transaction.gate_->consumed = true;
        }
        const auto declared = callback.find("state");
        const std::string state = declared == callback.end() ? std::string() : declared->second;
        if (!detail::oauth_constant_time_equal(state, transaction.state)) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::StateMismatch, transaction.scheme);
        }
        if (const auto denied = callback.find("error");
            denied != callback.end() && !denied->second.empty()) {
            @ERROR@ error;
            error.kind = @ERROR@::Kind::AuthorizationDenied;
            error.scheme = transaction.scheme;
            error.code = denied->second;
            return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(error));
        }
        const auto code = callback.find("code");
        if (code == callback.end() || code->second.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::InvalidResponse, transaction.scheme);
        }
        const auto* descriptor = detail::oauth_scheme(transaction.scheme);
        if (descriptor == nullptr) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnknownScheme, transaction.scheme);
        }
        if (descriptor->code_token_url.empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::UnsupportedFlow, transaction.scheme);
        }
        if (options.stop.stop_requested()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::Transport, transaction.scheme);
        }
        std::map<std::string, std::string, std::less<>> fields{
            {"grant_type", "authorization_code"},
            {"code", code->second},
            {"code_verifier", transaction.code_verifier}};
        if (!transaction.redirect_uri.empty()) {
            fields.emplace("redirect_uri", transaction.redirect_uri);
        }
        auto outcome = token_request(*descriptor, std::string(descriptor->code_token_url),
            std::move(fields), nullptr, options);
        if (outcome) {
            store_for(options)->replace(store_key(*descriptor, options), outcome.value());
        }
        return outcome;
    }
"#;

const REVOCATION_METHOD: &str = r#"
    /// Posts the token value to the scheme's compiled revocation endpoint
    /// (RFC 7009). Any 2xx response is success: RFC 7009 declares the token
    /// revoked even when the server reports an unsupported-token error. A
    /// successful revocation clears the resolved store's partition for this
    /// client identity.
    Result<Unit, @ERROR@> revoke_token(const std::string& scheme, const std::string& token,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<Unit>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->revocation_url.empty()) {
            return failure<Unit>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (token.empty()) return failure<Unit>(@ERROR@::Kind::InvalidToken, scheme);
        if (options.stop.stop_requested()) return failure<Unit>(@ERROR@::Kind::Transport, scheme);
        std::map<std::string, std::string, std::less<>> fields{{"token", token}};
        auto outcome = endpoint_request(*descriptor, descriptor->revocation_url,
            std::move(fields), options);
        if (!outcome) return Result<Unit, @ERROR@>::failure(std::move(outcome).error());
        const auto [status, body] = std::move(outcome).value();
        if (status < 200 || status >= 300) {
            if (auto object = detail::oauth_object(body);
                object && detail::oauth_text(*object, "error").has_value())
            {
                return failure<Unit>(@ERROR@::Kind::ServerRejected, scheme,
                    detail::oauth_text(*object, "error").value_or(""), status);
            }
            return failure<Unit>(@ERROR@::Kind::ServerError, scheme, {}, status);
        }
        store_for(options)->clear(store_key(*descriptor, options));
        return Result<Unit, @ERROR@>::success(Unit{});
    }
"#;

/// The discovery-aware revocation: the compiled endpoint always wins; when
/// the compiled plan declares none, the scheme's cached discovery document's
/// `revocation_endpoint` resolves the request.
const REVOCATION_METHOD_DISCOVERY: &str = r#"
    /// Posts the token value to the scheme's compiled revocation endpoint
    /// (RFC 7009). Any 2xx response is success: RFC 7009 declares the token
    /// revoked even when the server reports an unsupported-token error. A
    /// successful revocation clears the resolved store's partition for this
    /// client identity. The compiled endpoint always wins; when the compiled
    /// plan declares none, the scheme's cached discovery document's
    /// `revocation_endpoint` resolves the request.
    Result<Unit, @ERROR@> revoke_token(const std::string& scheme, const std::string& token,
        const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<Unit>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->revocation_url.empty() && descriptor->discovery_url.empty()) {
            return failure<Unit>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (token.empty()) return failure<Unit>(@ERROR@::Kind::InvalidToken, scheme);
        if (options.stop.stop_requested()) return failure<Unit>(@ERROR@::Kind::Transport, scheme);
        auto endpoint = resolve_endpoint(*descriptor, EndpointKind::Revocation, options);
        if (!endpoint) return Result<Unit, @ERROR@>::failure(std::move(endpoint).error());
        std::map<std::string, std::string, std::less<>> fields{{"token", token}};
        auto outcome = endpoint_request(*descriptor, endpoint.value(),
            std::move(fields), options);
        if (!outcome) return Result<Unit, @ERROR@>::failure(std::move(outcome).error());
        const auto [status, body] = std::move(outcome).value();
        if (status < 200 || status >= 300) {
            if (auto object = detail::oauth_object(body);
                object && detail::oauth_text(*object, "error").has_value())
            {
                return failure<Unit>(@ERROR@::Kind::ServerRejected, scheme,
                    detail::oauth_text(*object, "error").value_or(""), status);
            }
            return failure<Unit>(@ERROR@::Kind::ServerError, scheme, {}, status);
        }
        store_for(options)->clear(store_key(*descriptor, options));
        return Result<Unit, @ERROR@>::success(Unit{});
    }
"#;

const INTROSPECTION_METHOD: &str = r#"
    /// Queries the scheme's compiled introspection endpoint (RFC 7662) with
    /// the token value. The response describes the token without returning
    /// it; any non-2xx answer is a typed failure carrying no token values.
    Result<@INTROSPECTION@, @ERROR@> introspect_token(const std::string& scheme,
        const std::string& token, const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@INTROSPECTION@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->introspection_url.empty()) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (token.empty()) return failure<@INTROSPECTION@>(@ERROR@::Kind::InvalidToken, scheme);
        if (options.stop.stop_requested()) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::Transport, scheme);
        }
        std::map<std::string, std::string, std::less<>> fields{{"token", token}};
        auto outcome = endpoint_request(*descriptor, descriptor->introspection_url,
            std::move(fields), options);
        if (!outcome) return Result<@INTROSPECTION@, @ERROR@>::failure(std::move(outcome).error());
        const auto [status, body] = std::move(outcome).value();
        if (status < 200 || status >= 300) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::ServerError, scheme, {}, status);
        }
        const auto object = detail::oauth_object(body);
        if (!object) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::InvalidResponse, scheme, {}, status);
        }
        @INTROSPECTION@ report;
        report.active = object->find("active") != object->end()
            && object->at("active").is<bool>() && object->at("active").as<bool>();
        if (auto scope = detail::oauth_text(*object, "scope")) report.scope = std::move(*scope);
        if (auto id = detail::oauth_text(*object, "client_id")) {
            report.client_id = std::move(*id);
        }
        if (auto type = detail::oauth_text(*object, "token_type")) {
            report.token_type = std::move(*type);
        }
        if (auto username = detail::oauth_text(*object, "username")) {
            report.username = std::move(*username);
        }
        if (auto value = detail::oauth_integer(*object, "exp")) report.expires_at = *value;
        if (auto value = detail::oauth_integer(*object, "iat")) report.issued_at = *value;
        if (auto value = detail::oauth_integer(*object, "nbf")) report.not_before = *value;
        if (auto subject = detail::oauth_text(*object, "sub")) report.subject = std::move(*subject);
        if (auto issuer = detail::oauth_text(*object, "iss")) report.issuer = std::move(*issuer);
        if (auto jwt_id = detail::oauth_text(*object, "jti")) report.jwt_id = std::move(*jwt_id);
        if (auto audience = object->find("aud"); audience != object->end()) {
            if (audience->second.is<std::string>()) {
                report.audience.push_back(audience->second.as<std::string>());
            } else if (audience->second.is<JsonValue::Array>()) {
                for (const auto& entry : audience->second.as<JsonValue::Array>()) {
                    if (entry.is<std::string>()) report.audience.push_back(entry.as<std::string>());
                }
            }
        }
        return Result<@INTROSPECTION@, @ERROR@>::success(std::move(report));
    }
"#;

/// The discovery-aware introspection: the compiled endpoint always wins; when
/// the compiled plan declares none, the scheme's cached discovery document's
/// `introspection_endpoint` resolves the request.
const INTROSPECTION_METHOD_DISCOVERY: &str = r#"
    /// Queries the scheme's compiled introspection endpoint (RFC 7662) with
    /// the token value. The response describes the token without returning
    /// it; any non-2xx answer is a typed failure carrying no token values.
    /// The compiled endpoint always wins; when the compiled plan declares
    /// none, the scheme's cached discovery document's `introspection_endpoint`
    /// resolves the request.
    Result<@INTROSPECTION@, @ERROR@> introspect_token(const std::string& scheme,
        const std::string& token, const @OPTIONS@& options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) return failure<@INTROSPECTION@>(@ERROR@::Kind::UnknownScheme, scheme);
        if (descriptor->introspection_url.empty() && descriptor->discovery_url.empty()) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (token.empty()) return failure<@INTROSPECTION@>(@ERROR@::Kind::InvalidToken, scheme);
        if (options.stop.stop_requested()) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::Transport, scheme);
        }
        auto endpoint = resolve_endpoint(*descriptor, EndpointKind::Introspection, options);
        if (!endpoint) return Result<@INTROSPECTION@, @ERROR@>::failure(std::move(endpoint).error());
        std::map<std::string, std::string, std::less<>> fields{{"token", token}};
        auto outcome = endpoint_request(*descriptor, endpoint.value(),
            std::move(fields), options);
        if (!outcome) return Result<@INTROSPECTION@, @ERROR@>::failure(std::move(outcome).error());
        const auto [status, body] = std::move(outcome).value();
        if (status < 200 || status >= 300) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::ServerError, scheme, {}, status);
        }
        const auto object = detail::oauth_object(body);
        if (!object) {
            return failure<@INTROSPECTION@>(@ERROR@::Kind::InvalidResponse, scheme, {}, status);
        }
        @INTROSPECTION@ report;
        report.active = object->find("active") != object->end()
            && object->at("active").is<bool>() && object->at("active").as<bool>();
        if (auto scope = detail::oauth_text(*object, "scope")) report.scope = std::move(*scope);
        if (auto id = detail::oauth_text(*object, "client_id")) {
            report.client_id = std::move(*id);
        }
        if (auto type = detail::oauth_text(*object, "token_type")) {
            report.token_type = std::move(*type);
        }
        if (auto username = detail::oauth_text(*object, "username")) {
            report.username = std::move(*username);
        }
        if (auto value = detail::oauth_integer(*object, "exp")) report.expires_at = *value;
        if (auto value = detail::oauth_integer(*object, "iat")) report.issued_at = *value;
        if (auto value = detail::oauth_integer(*object, "nbf")) report.not_before = *value;
        if (auto subject = detail::oauth_text(*object, "sub")) report.subject = std::move(*subject);
        if (auto issuer = detail::oauth_text(*object, "iss")) report.issuer = std::move(*issuer);
        if (auto jwt_id = detail::oauth_text(*object, "jti")) report.jwt_id = std::move(*jwt_id);
        if (auto audience = object->find("aud"); audience != object->end()) {
            if (audience->second.is<std::string>()) {
                report.audience.push_back(audience->second.as<std::string>());
            } else if (audience->second.is<JsonValue::Array>()) {
                for (const auto& entry : audience->second.as<JsonValue::Array>()) {
                    if (entry.is<std::string>()) report.audience.push_back(entry.as<std::string>());
                }
            }
        }
        return Result<@INTROSPECTION@, @ERROR@>::success(std::move(report));
    }
"#;

const SESSIONS_TAIL: &str = r#"
private:
    /// One single-flight acquisition round. The first caller claims it;
    /// later callers wait for `done` and then re-probe the store.
    struct Round {
        std::mutex mutex;
        std::condition_variable signal;
        bool claimed = false;
        bool done = false;
    };

    /// The explicit store, or the instance-owned default created on first
    /// use. Instance-owned: generated code never shares a store implicitly.
    [[nodiscard]] std::shared_ptr<@TOKEN_STORE@> store_for(const @OPTIONS@& options) {
        if (options.store) return options.store;
        std::lock_guard<std::mutex> guard(store_mutex_);
        if (!store_) store_ = std::make_shared<@MEMORY_STORE@>();
        return store_;
    }

    /// Explicit options first, then the compiled variable names read at call
    /// time. Empty values mean absent; callers may legitimately configure
    /// only one.
    [[nodiscard]] static std::pair<std::string, std::string> credentials(
        const detail::OAuthSchemeDescriptor& descriptor, const @OPTIONS@& options) {
        std::string id = options.client_id;
        if (id.empty() && !descriptor.client_id_env.empty()) {
            const std::string variable(descriptor.client_id_env);
            if (const char* value = std::getenv(variable.c_str())) id = value;
        }
        std::string secret = options.client_secret;
        if (secret.empty() && !descriptor.client_secret_env.empty()) {
            const std::string variable(descriptor.client_secret_env);
            if (const char* value = std::getenv(variable.c_str())) secret = value;
        }
        return {std::move(id), std::move(secret)};
    }

    /// Store keys partition by scheme, token-endpoint issuer and client
    /// identity, so distinct clients and endpoints never share a set.
    [[nodiscard]] static std::string store_key(const detail::OAuthSchemeDescriptor& descriptor,
        const @OPTIONS@& options) {
        const auto resolved = credentials(descriptor, options);
        std::string key(descriptor.name);
        key.push_back('\x1f');
        key += descriptor.issuer;
        key.push_back('\x1f');
        key += resolved.first;
        return key;
    }

    /// The stored set when it still outlives its compiled clock skew.
    [[nodiscard]] static std::optional<@TOKEN_SET@> fresh(
        const std::shared_ptr<@TOKEN_STORE@>& store, const std::string& key,
        const detail::OAuthSchemeDescriptor& descriptor) {
        auto stored = store->load(key);
        if (!stored || stored->expired(descriptor.skew)) return std::nullopt;
        return stored;
    }

    /// Release one finished round: waiters wake and re-probe the store
    /// instead of acquiring again.
    void finish(const std::string& key, const std::shared_ptr<Round>& round) {
        {
            std::lock_guard<std::mutex> guard(gates_mutex_);
            gates_.erase(key);
        }
        {
            std::lock_guard<std::mutex> guard(round->mutex);
            round->done = true;
        }
        round->signal.notify_all();
    }

    /// Typed failure construction carrying only safe metadata.
    template<class T>
    [[nodiscard]] static Result<T, @ERROR@> failure(@ERROR@::Kind kind, std::string scheme,
        std::string code = {}, const int status = 0) {
        @ERROR@ error;
        error.kind = kind;
        error.scheme = std::move(scheme);
        error.code = std::move(code);
        error.status = status;
        return Result<T, @ERROR@>::failure(std::move(error));
    }

    /// RFC 4648 base64 with padding, for Basic credentials only.
    [[nodiscard]] static std::string base64(std::string_view bytes) {
        static constexpr char alphabet[] =
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        std::string out;
        out.reserve(((bytes.size() + 2) / 3) * 4);
        std::size_t at = 0;
        while (at + 3 <= bytes.size()) {
            const std::uint32_t group =
                (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[at])) << 16)
                | (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[at + 1])) << 8)
                | static_cast<unsigned char>(bytes[at + 2]);
            out.push_back(alphabet[(group >> 18) & 0x3F]);
            out.push_back(alphabet[(group >> 12) & 0x3F]);
            out.push_back(alphabet[(group >> 6) & 0x3F]);
            out.push_back(alphabet[group & 0x3F]);
            at += 3;
        }
        const std::size_t rest = bytes.size() - at;
        if (rest == 1) {
            const std::uint32_t group =
                static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[at])) << 16;
            out.push_back(alphabet[(group >> 18) & 0x3F]);
            out.push_back(alphabet[(group >> 12) & 0x3F]);
            out.push_back('=');
            out.push_back('=');
        } else if (rest == 2) {
            const std::uint32_t group =
                (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[at])) << 16)
                | (static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[at + 1])) << 8);
            out.push_back(alphabet[(group >> 18) & 0x3F]);
            out.push_back(alphabet[(group >> 12) & 0x3F]);
            out.push_back(alphabet[(group >> 6) & 0x3F]);
            out.push_back('=');
        }
        return out;
    }

    /// RFC 6749 2.3.1 Basic credentials: form-encoded id and secret.
    [[nodiscard]] static std::string basic_credentials(std::string_view id,
        std::string_view secret) {
        std::string pair = detail::oauth_form_value(id);
        pair.push_back(':');
        pair += detail::oauth_form_value(secret);
        return base64(pair);
    }

    /// Sorted deterministic form encoding; absent optional entries are simply
    /// not sent.
    [[nodiscard]] static std::string form_encode(
        const std::map<std::string, std::string, std::less<>>& fields) {
        std::string out;
        for (const auto& field : fields) {
            if (!out.empty()) out.push_back('&');
            out += detail::oauth_form_value(field.first);
            out.push_back('=');
            out += detail::oauth_form_value(field.second);
        }
        return out;
    }

    /// Posts one form-encoded request to a compiled endpoint, applying the
    /// scheme's compiled client authentication (HTTP Basic for confidential
    /// clients, the client_id form member for public ones) and returns the
    /// status and the bounded body.
    [[nodiscard]] Result<std::pair<int, std::string>, @ERROR@> endpoint_request(
        const detail::OAuthSchemeDescriptor& descriptor, std::string_view endpoint,
        std::map<std::string, std::string, std::less<>> fields, const @OPTIONS@& options) const {
        if (endpoint.empty()) {
            return failure<std::pair<int, std::string>>(@ERROR@::Kind::UnsupportedFlow,
                std::string(descriptor.name));
        }
        if (options.stop.stop_requested()) {
            return failure<std::pair<int, std::string>>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        const auto resolved = credentials(descriptor, options);
        if (descriptor.client_secret_basic) {
            if (resolved.first.empty() || resolved.second.empty()) {
                return failure<std::pair<int, std::string>>(@ERROR@::Kind::MissingCredentials,
                    std::string(descriptor.name));
            }
        } else if (!resolved.first.empty()) {
            fields.emplace("client_id", resolved.first);
        }
        HttpRequest request;
        request.method = "POST";
        request.url = std::string(endpoint);
        request.headers = Headers{{"Accept", "application/json"},
            {"Content-Type", "application/x-www-form-urlencoded"}};
        if (descriptor.client_secret_basic) {
            request.headers.emplace_back("Authorization",
                "Basic " + basic_credentials(resolved.first, resolved.second));
        }
        request.body = form_encode(fields);
        TransportOptions transport_options;
        transport_options.stop = options.stop;
        transport_options.deadline = std::chrono::steady_clock::now() + options.timeout;
        if (!transport_) {
            return failure<std::pair<int, std::string>>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        auto outcome = transport_->send(request, transport_options);
        if (!outcome) {
            return failure<std::pair<int, std::string>>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        auto response = std::move(outcome).value();
        if (response.body.size() > detail::oauth_max_response_bytes) {
            return failure<std::pair<int, std::string>>(@ERROR@::Kind::ResourceLimit,
                std::string(descriptor.name), {}, response.status);
        }
        return Result<std::pair<int, std::string>, @ERROR@>::success(
            std::make_pair(response.status, std::move(response.body)));
    }

    /// Executes one RFC 6749 token-endpoint request and decodes the response.
    /// A server-declared error member wins; a 2xx answer without an access
    /// token is an invalid response. `previous` retains its refresh token
    /// when the server returns no rotated one.
    [[nodiscard]] Result<@TOKEN_SET@, @ERROR@> token_request(
        const detail::OAuthSchemeDescriptor& descriptor, std::string_view endpoint,
        std::map<std::string, std::string, std::less<>> fields, const @TOKEN_SET@* previous,
        const @OPTIONS@& options) const {
        auto outcome = endpoint_request(descriptor, endpoint, std::move(fields), options);
        if (!outcome) return Result<@TOKEN_SET@, @ERROR@>::failure(std::move(outcome).error());
        const auto answered = std::move(outcome).value();
        const int status = answered.first;
        const auto& body = answered.second;
        const auto object = detail::oauth_object(body);
        if (!object) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::InvalidResponse, std::string(descriptor.name), {},
                status);
        }
        if (auto declared = detail::oauth_text(*object, "error");
            declared && !declared->empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::ServerRejected, std::string(descriptor.name),
                std::move(*declared), status);
        }
        if (status < 200 || status >= 300) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::ServerError, std::string(descriptor.name), {},
                status);
        }
        auto access = detail::oauth_text(*object, "access_token");
        if (!access || access->empty()) {
            return failure<@TOKEN_SET@>(@ERROR@::Kind::InvalidResponse, std::string(descriptor.name), {},
                status);
        }
        @TOKEN_SET@ set;
        set.access_token = std::move(*access);
        if (auto type = detail::oauth_text(*object, "token_type"); type && !type->empty()) {
            // The conventional Bearer spelling per RFC 6750's case-insensitive
            // scheme matching; other token types keep the server's spelling.
            std::string normalized = std::move(*type);
            std::string lowered;
            lowered.reserve(normalized.size());
            for (const char c : normalized) {
                lowered.push_back(c >= 'A' && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c);
            }
            set.token_type = lowered == "bearer" ? std::string("Bearer") : std::move(normalized);
        }
        if (auto expires_in = detail::oauth_integer(*object, "expires_in");
            expires_in && *expires_in > 0) {
            set.expires_at = std::chrono::system_clock::now() + std::chrono::seconds(*expires_in);
        }
        if (auto refresh = detail::oauth_text(*object, "refresh_token");
            refresh && !refresh->empty()) {
            set.refresh_token = std::move(*refresh);
            set.has_refresh = true;
        } else if (previous != nullptr && previous->has_refresh) {
            // Adopt a rotated refresh token; retain the previous one otherwise.
            set.refresh_token = previous->refresh_token;
            set.has_refresh = true;
        }
        if (auto scope = detail::oauth_text(*object, "scope")) set.scope = std::move(*scope);
        return Result<@TOKEN_SET@, @ERROR@>::success(std::move(set));
    }

    /// Paces device polling: the injected waiter, else the thread sleeper.
    static void wait_interval(std::chrono::milliseconds interval, const @OPTIONS@& options) {
        if (options.wait) {
            options.wait(interval);
            return;
        }
        std::this_thread::sleep_for(interval);
    }

    std::shared_ptr<const Transport> transport_;
    std::mutex store_mutex_;
    std::shared_ptr<@TOKEN_STORE@> store_;
    std::mutex gates_mutex_;
    std::map<std::string, std::shared_ptr<Round>> gates_;
"#;

/// The session's private discovery engine, emitted only when at least one
/// compiled scheme carries a discovery URL: the per-instance cache keyed by
/// scheme, the single-flight fetch through the shared round gates, the
/// bounded and issuer-validated document fetch, and the endpoint-resolution
/// precedence the conditional methods consume. Placed inside the class's
/// private section, before the closing brace.
const DISCOVERY_STATE: &str = r#"
    /// Instance-owned discovery cache keyed by scheme name. Successful
    /// documents live here for the session's lifetime, so repeated calls
    /// never re-fetch; failed fetches are never cached, so the next call
    /// retries.
    mutable std::mutex discovery_mutex_;
    std::map<std::string, detail::OAuthDiscoveredEndpoints, std::less<>> discovery_;

    /// Which compiled endpoint a resolution serves.
    enum class EndpointKind { Token, Refresh, Revocation, Introspection };

    /// Fetches the scheme's discovery document (RFC 8414 / OpenID Connect),
    /// returning the session's cached copy when one exists. Concurrent
    /// callers share the one in-flight fetch through the shared round gates:
    /// callers waiting on the gate re-probe the cache instead of fetching
    /// again.
    Result<detail::OAuthDiscoveredEndpoints, @ERROR@> discover(
        const detail::OAuthSchemeDescriptor& descriptor, const @OPTIONS@& options) {
        if (descriptor.discovery_url.empty()) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::UnsupportedFlow,
                std::string(descriptor.name));
        }
        const std::string name(descriptor.name);
        for (;;) {
            {
                std::lock_guard<std::mutex> guard(discovery_mutex_);
                if (const auto cached = discovery_.find(name); cached != discovery_.end()) {
                    return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::success(
                        cached->second);
                }
            }
            // Discovery rounds share the acquisition gates under a key no
            // store key can produce, so a discovery round never blocks a
            // token acquisition.
            const std::string gate_key = std::string("\x1f", 1) + std::string("discovery\x1f") + name;
            std::shared_ptr<Round> round;
            bool proceed = false;
            {
                std::lock_guard<std::mutex> guard(gates_mutex_);
                auto& slot = gates_[gate_key];
                if (!slot) {
                    slot = std::make_shared<Round>();
                    slot->claimed = true;
                    proceed = true;
                }
                round = slot;
            }
            if (!proceed) {
                // The round holder owns this fetch: wait for its completion,
                // then re-probe the cache instead of fetching again.
                std::unique_lock<std::mutex> waiter(round->mutex);
                round->signal.wait(waiter, [&round] { return round->done; });
                continue;
            }
            // This caller owns the round: re-check the cache once more (the
            // fast path ran before this caller was selected), fetch and
            // store on success. A failed fetch is never cached, so the next
            // call retries.
            {
                std::lock_guard<std::mutex> guard(discovery_mutex_);
                if (const auto cached = discovery_.find(name); cached != discovery_.end()) {
                    finish(gate_key, round);
                    return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::success(
                        cached->second);
                }
            }
            auto fetched = discovery_fetch(descriptor, options);
            if (fetched) {
                std::lock_guard<std::mutex> guard(discovery_mutex_);
                discovery_.insert_or_assign(name, fetched.value());
            }
            finish(gate_key, round);
            if (!fetched) {
                return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::failure(
                    std::move(fetched).error());
            }
            return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::success(
                std::move(fetched).value());
        }
    }

    /// Performs one GET for the discovery document with
    /// `accept: application/json` and the bounded response ceiling. The
    /// exact issuer rule: when the document carries an `issuer` claim, it
    /// must be an absolute http(s) URL whose origin (scheme, host and the
    /// port with the scheme default made explicit) equals the discovery
    /// URL's origin; OpenID Connect openIdConnectUrl documents are validated
    /// against their `issuer` claim exactly this way, as are RFC 8414
    /// authorization-server metadata documents. A missing claim is
    /// tolerated. Failure messages carry only safe metadata, never response
    /// body text.
    Result<detail::OAuthDiscoveredEndpoints, @ERROR@> discovery_fetch(
        const detail::OAuthSchemeDescriptor& descriptor, const @OPTIONS@& options) const {
        if (!transport_) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        if (options.stop.stop_requested()) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        HttpRequest request;
        request.method = "GET";
        request.url = std::string(descriptor.discovery_url);
        request.headers = Headers{{"Accept", "application/json"}};
        TransportOptions transport_options;
        transport_options.stop = options.stop;
        transport_options.deadline = std::chrono::steady_clock::now() + options.timeout;
        auto outcome = transport_->send(request, transport_options);
        if (!outcome) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::Transport,
                std::string(descriptor.name));
        }
        auto response = std::move(outcome).value();
        if (response.body.size() > detail::oauth_discovery_max_bytes) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::DiscoveryFailed,
                std::string(descriptor.name), {}, response.status);
        }
        if (response.status < 200 || response.status >= 300) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::DiscoveryFailed,
                std::string(descriptor.name), {}, response.status);
        }
        const auto parsed = detail::oauth_object(response.body);
        if (!parsed) {
            return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::DiscoveryFailed,
                std::string(descriptor.name), {}, response.status);
        }
        if (auto claimed = detail::oauth_text(*parsed, "issuer"); claimed && !claimed->empty()) {
            const auto issuer_origin = detail::oauth_url_origin(*claimed);
            const auto discovery_origin = detail::oauth_url_origin(descriptor.discovery_url);
            if (discovery_origin.empty() || issuer_origin != discovery_origin) {
                return failure<detail::OAuthDiscoveredEndpoints>(@ERROR@::Kind::DiscoveryFailed,
                    std::string(descriptor.name), {}, response.status);
            }
        }
        auto member = [&](std::string_view key)
            -> Result<Presence<std::string>, @ERROR@> {
            return discovery_member(*parsed, key, response.status, descriptor);
        };
        detail::OAuthDiscoveredEndpoints endpoints;
        if (auto token = member("token_endpoint")) {
            if (auto value = std::move(token).value()) endpoints.token_endpoint = *value;
        } else {
            return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::failure(
                std::move(token).error());
        }
        if (auto revocation = member("revocation_endpoint")) {
            if (auto value = std::move(revocation).value()) {
                endpoints.revocation_endpoint = std::move(*value);
            }
        } else {
            return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::failure(
                std::move(revocation).error());
        }
        if (auto introspection = member("introspection_endpoint")) {
            if (auto value = std::move(introspection).value()) {
                endpoints.introspection_endpoint = std::move(*value);
            }
        } else {
            return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::failure(
                std::move(introspection).error());
        }
        return Result<detail::OAuthDiscoveredEndpoints, @ERROR@>::success(
            std::move(endpoints));
    }

    /// Reads one present discovery member: absent members stay disengaged,
    /// a non-string or unusable value is a typed discovery failure, and
    /// unknown members are ignored.
    Result<Presence<std::string>, @ERROR@> discovery_member(const JsonValue::Object& document,
        std::string_view key, const int status, const detail::OAuthSchemeDescriptor& descriptor)
        const {
        const auto found = document.find(key);
        if (found == document.end()) {
            return Result<Presence<std::string>, @ERROR@>::success(std::nullopt);
        }
        if (!found->second.is<std::string>()) {
            return failure<Presence<std::string>>(@ERROR@::Kind::DiscoveryFailed,
                std::string(descriptor.name), {}, status);
        }
        const auto& value = found->second.as<std::string>();
        if (detail::oauth_discovery_unusable(value)) {
            return failure<Presence<std::string>>(@ERROR@::Kind::DiscoveryFailed,
                std::string(descriptor.name), {}, status);
        }
        return Result<Presence<std::string>, @ERROR@>::success(value);
    }

    /// Resolves one lifecycle endpoint through the compiled precedence: an
    /// explicit compiled endpoint always wins; otherwise the cached
    /// discovery document's endpoint when the scheme compiles a discovery
    /// URL; otherwise the typed refusal the compiled plan alone would
    /// produce.
    Result<std::string, @ERROR@> resolve_endpoint(const detail::OAuthSchemeDescriptor& descriptor,
        const EndpointKind kind, const @OPTIONS@& options) {
        const std::string_view compiled = kind == EndpointKind::Token
            ? descriptor.client_credentials_url
            : kind == EndpointKind::Refresh
              ? descriptor.refresh_url
              : kind == EndpointKind::Revocation
                ? descriptor.revocation_url
                : descriptor.introspection_url;
        if (!compiled.empty()) {
            return Result<std::string, @ERROR@>::success(std::string(compiled));
        }
        auto discovered = discover(descriptor, options);
        if (!discovered) {
            return Result<std::string, @ERROR@>::failure(std::move(discovered).error());
        }
        const std::string& found = kind == EndpointKind::Revocation
            ? discovered.value().revocation_endpoint
            : kind == EndpointKind::Introspection
              ? discovered.value().introspection_endpoint
              : discovered.value().token_endpoint;
        if (found.empty()) {
            return failure<std::string>(@ERROR@::Kind::UnsupportedFlow,
                std::string(descriptor.name));
        }
        return Result<std::string, @ERROR@>::success(found);
    }
"#;

const SESSIONS_TAIL_CLOSE: &str = r#"};
"#;

/// The store-key derivation the opt-in replaying wrapper shares with the
/// session's partitioning, emitted only when the wrapper participates so plans
/// without an executable client-credentials flow keep the pre-replay bytes.
const REPLAY_STORE_KEY: &str = r#"
    /// The store key the opt-in replaying credential wrapper shares with this
    /// session's partitioning: it derives the same key from the same compiled
    /// inputs, so a replay refresh coordinates over the exact partition the
    /// plain lifecycle serves.
    [[nodiscard]] static std::string replay_store_key(
        const detail::OAuthSchemeDescriptor& descriptor, const @OPTIONS@& options) {
        return store_key(descriptor, options);
    }
"#;

/// The stream-protected operations whose attaches must never be replayed, per
/// compiled scheme: security requirements that name a compiled scheme on an
/// operation whose responses carry a stream representation. Delivered stream
/// data prevents a transparent restart, so those operations are excluded from
/// the replay wrapper's one-replay budget. The C++ attach request carries the
/// operation source (the requirement's own source pointer is not exposed to
/// credential hooks), so the compiled set names the stream-protected
/// operations per scheme.
pub(super) fn no_replay_requirements(
    schemes: &[OAuthScheme],
    operations: &[PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = schemes.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire.responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), wire::Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire.security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    wire::CredentialHook::OAuth2 { .. }
                        | wire::CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if names.contains(requirement.name()) {
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(operation.source.pointer().to_owned());
                }
            }
        }
    }
    pointers
}

/// The replaying credential wrapper: one coordinated refresh plus one eligible
/// request replay per qualifying 401, opt-in per provider, and never for
/// stream-protected operations. The section emits only when a compiled scheme
/// carries an executable client-credentials flow, the provider the wrapper
/// wraps; plans without one assemble byte-identically to the pre-replay bytes.
/// The plain and discovery variants resolve the lifecycle-endpoint exclusion
/// through the same compiled precedence as the lifecycle they wrap.
fn replay_section(plan: &SdkPlan, oauth: &OAuthPlan) -> String {
    let discovery = has_discovery(oauth);
    let no_replay = no_replay_requirements(&oauth.schemes, plan.operations());
    let mut out = String::from("namespace detail {\n\n");
    out.push_str(REPLAY_PROTECTED_TABLE);
    for scheme in &oauth.schemes {
        let Some(operations) = no_replay.get(&scheme.name) else {
            continue;
        };
        if operations.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "    {{{}, {{{}}}}},\n",
            sv(&scheme.name),
            operations
                .iter()
                .map(|op| sv(op))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    out.push_str("};\n\n} // namespace detail\n\n");
    out.push_str(REPLAY_LOWER);
    out.push_str(REPLAY_TRANSPORT_TYPE);
    out.push_str(REPLAY_WRAPPER_HEAD);
    out.push_str(if discovery {
        REPLAY_CREATE_DISCOVERY
    } else {
        REPLAY_CREATE_PLAIN
    });
    out.push_str(REPLAY_WRAPPER_CORE);
    out.push_str(if discovery {
        REPLAY_LIFECYCLE_DISCOVERY
    } else {
        REPLAY_LIFECYCLE_PLAIN
    });
    out.push_str(REPLAY_WRAPPER_TAIL);
    out
}

/// The compiled stream-protected operations table and the replaying
/// credential wrapper's public surface, shared by the plain and discovery
/// variants.
const REPLAY_PROTECTED_TABLE: &str = r#"/// Compiled stream-protected operations per source scheme: operations whose
/// responses carry a stream representation and whose security requirements
/// name the scheme. Attaches served on these operations are never replayed,
/// because delivered stream data prevents a transparent restart; the attach
/// request carries the operation source, so the compiled set names operations.
inline const std::vector<std::pair<std::string_view, std::vector<std::string_view>>> oauth_replay_protected = {
"#;

/// The replaying transport: the 401 interception half of the wrapper, shared
/// by the plain and discovery variants. The send body is defined after the
/// wrapper class, which it consults through the owning shared instance.
const REPLAY_TRANSPORT_TYPE: &str = r#"class @REPLAY@;

/// The replaying credential's transport: one coordinated refresh and, when
/// the request carried the provider's token and no stream is protected on it,
/// exactly one replay with the fresh token. Lifecycle endpoint requests are
/// never replayed: they carry no bearer token of this provider, and the
/// exact-target guard in the wrapper is defense in depth.
class @REPLAY_TRANSPORT@ final : public Transport {
public:
    @REPLAY_TRANSPORT@(std::shared_ptr<@REPLAY@> owner, std::shared_ptr<const Transport> inner)
        : owner_(std::move(owner)), inner_(std::move(inner)) {}
    Result<HttpResponse, TransportError> send(const HttpRequest& request,
        const TransportOptions& options) const override;

private:
    std::shared_ptr<@REPLAY@> owner_;
    std::shared_ptr<const Transport> inner_;
};

"#;

/// The replaying credential wrapper's head: the documented class, its
/// creation and the shared surface. Creation refuses a compiled scheme whose
/// client-credentials endpoint (or, with discovery, the discovery document)
/// cannot serve one.
const REPLAY_WRAPPER_HEAD: &str = r#"/// The replaying client-credentials credential plus its transport policy: the
/// plain lifecycle's attach behavior plus the unified 401 request policy.
/// Wire it in two places — pass `credential()` as the scheme's credentials
/// member and `transport(inner)` as the client's transport:
///
///     auto replay = OAuthReplayCredentials::create(inner, "scheme", options);
///     Client client(replay->transport(inner), credentials.with(replay->credential()));
///
/// A 401 (and only a 401) on a request whose Authorization value this wrapper
/// attached triggers exactly one coordinated refresh — concurrent 401s share
/// one token request round — and exactly one replay of the request with the
/// fresh token, preserving method, URL and body while regenerating the
/// authorization header. The second response is surfaced whatever it is: a
/// second 401 reaches the caller as the declared error. The overall budget is
/// one refresh plus one replay, never nested with other retry policies
/// (requests are not retried today). Attaches served on stream-protected
/// operations are never replayed, because delivered stream data prevents a
/// transparent restart. A refresh failure surfaces as the typed transport
/// failure instead of a replay. The plain lifecycle keeps today's semantics:
/// replay is this wrapper's opt-in only.
class @REPLAY@ final : public std::enable_shared_from_this<@REPLAY@> {
public:
    @REPLAY@(const @REPLAY@&) = delete;
    @REPLAY@& operator=(const @REPLAY@&) = delete;
"#;

/// The plain variant's creation: the compiled client-credentials endpoint
/// must exist, exactly like the plain lifecycle's first attach.
const REPLAY_CREATE_PLAIN: &str = r#"
    /// Creates the replaying variant of one compiled scheme's
    /// client-credentials provider. `inner` is the transport token requests
    /// travel through directly; every option behaves exactly as in
    /// @SESSIONS@::client_credentials_token. Creation refuses a compiled
    /// scheme whose client-credentials flow declares no token endpoint,
    /// exactly like the plain lifecycle's first attach.
    [[nodiscard]] static Result<std::shared_ptr<@REPLAY@>, @ERROR@> create(
        std::shared_ptr<const Transport> inner, std::string scheme, @OPTIONS@ options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) {
            return failure<std::shared_ptr<@REPLAY@>>(@ERROR@::Kind::UnknownScheme, scheme);
        }
        if (descriptor->client_credentials_url.empty()) {
            return failure<std::shared_ptr<@REPLAY@>>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (!options.store) options.store = std::make_shared<@MEMORY_STORE@>();
        auto sessions = std::make_shared<@SESSIONS@>(inner, options.store);
        return Result<std::shared_ptr<@REPLAY@>, @ERROR@>::success(
            std::shared_ptr<@REPLAY@>(new @REPLAY@(std::move(scheme), std::move(options),
                std::move(sessions), std::move(inner))));
    }
"#;

/// The discovery variant's creation: a compiled client-credentials endpoint
/// or a discovery URL makes the scheme replayable, exactly like the compiled
/// acquisition precedence.
const REPLAY_CREATE_DISCOVERY: &str = r#"
    /// Creates the replaying variant of one compiled scheme's
    /// client-credentials provider. `inner` is the transport token requests
    /// travel through directly; every option behaves exactly as in
    /// @SESSIONS@::client_credentials_token. Creation refuses a compiled
    /// scheme with neither a client-credentials endpoint nor a discovery URL,
    /// exactly like the compiled acquisition precedence; a discovery URL
    /// resolves the token endpoint at refresh time.
    [[nodiscard]] static Result<std::shared_ptr<@REPLAY@>, @ERROR@> create(
        std::shared_ptr<const Transport> inner, std::string scheme, @OPTIONS@ options = {}) {
        const auto* descriptor = detail::oauth_scheme(scheme);
        if (descriptor == nullptr) {
            return failure<std::shared_ptr<@REPLAY@>>(@ERROR@::Kind::UnknownScheme, scheme);
        }
        if (descriptor->client_credentials_url.empty() && descriptor->discovery_url.empty()) {
            return failure<std::shared_ptr<@REPLAY@>>(@ERROR@::Kind::UnsupportedFlow, scheme);
        }
        if (!options.store) options.store = std::make_shared<@MEMORY_STORE@>();
        auto sessions = std::make_shared<@SESSIONS@>(inner, options.store);
        return Result<std::shared_ptr<@REPLAY@>, @ERROR@>::success(
            std::shared_ptr<@REPLAY@>(new @REPLAY@(std::move(scheme), std::move(options),
                std::move(sessions), std::move(inner))));
    }
"#;

/// The replaying credential wrapper's core: the credential hook with its
/// served-attach record, the eligibility decision, the served-attach lookup,
/// the coordinated refresh round and the failure mapping. Shared by the plain
/// and discovery variants.
const REPLAY_WRAPPER_CORE: &str = r#"
    /// The credential hook: serves this scheme's tokens through the wrapped
    /// session's plain lifecycle and remembers which Authorization values its
    /// attaches produced. Pass it as the scheme's credentials member.
    [[nodiscard]] CredentialProvider credential() {
        std::shared_ptr<@REPLAY@> owner = shared_from_this();
        return [owner = std::move(owner)](const CredentialRequest& request)
            -> Result<Authorization, TransportError> {
            auto acquired = owner->sessions_->client_credentials_token(owner->scheme_, owner->options_);
            if (!acquired) {
                return Result<Authorization, TransportError>::failure(
                    transport_failure(std::move(acquired).error()));
            }
            owner->record(replay_value(acquired.value()), eligible(request));
            const auto& set = acquired.value();
            return Result<Authorization, TransportError>::success(Authorization(
                set.token_type.empty() ? std::string("Bearer") : set.token_type,
                set.access_token));
        };
    }

    /// Wraps `inner` with the one-refresh-one-replay 401 policy; call once per
    /// client. Token requests keep traveling through `inner` directly.
    [[nodiscard]] std::shared_ptr<const Transport> transport(
        std::shared_ptr<const Transport> inner) {
        return std::make_shared<@REPLAY_TRANSPORT@>(shared_from_this(), std::move(inner));
    }

private:
    friend class @REPLAY_TRANSPORT@;

    /// Typed failure construction carrying only safe metadata.
    template<class T>
    [[nodiscard]] static Result<T, @ERROR@> failure(@ERROR@::Kind kind, std::string scheme) {
        @ERROR@ error;
        error.kind = kind;
        error.scheme = std::move(scheme);
        return Result<T, @ERROR@>::failure(std::move(error));
    }

    @REPLAY@(std::string scheme, @OPTIONS@ options, std::shared_ptr<@SESSIONS@> sessions,
        std::shared_ptr<const Transport> inner)
        : scheme_(std::move(scheme)), options_(std::move(options)),
          sessions_(std::move(sessions)), inner_(std::move(inner)) {}

    /// One attach this wrapper served, remembered so the wrapped transport can
    /// tell which requests carried this wrapper's token. The record keeps only
    /// safe metadata plus the eligibility decision; token values already
    /// traveled on the wire.
    struct ServedAttach {
        std::string value;
        bool eligible = false;
    };

    /// Records one served attach, newest first, bounded to eight entries.
    void record(std::string value, const bool eligible) {
        std::lock_guard<std::mutex> guard(served_mutex_);
        served_.insert(served_.begin(), ServedAttach{std::move(value), eligible});
        if (served_.size() > 8) served_.pop_back();
    }

    /// Whether a qualifying 401 on this attach may be replayed: attaches for
    /// stream-protected requirements never are, because delivered stream data
    /// prevents a transparent restart.
    [[nodiscard]] static bool eligible(const CredentialRequest& request) {
        for (const auto& [scheme, operations] : detail::oauth_replay_protected) {
            if (scheme != std::string_view(request.scheme_name)) continue;
            return std::find(operations.begin(), operations.end(),
                       request.operation_source.pointer) == operations.end();
        }
        return true;
    }

    /// Whether the presented Authorization value carries this wrapper's token
    /// from an eligible attach.
    [[nodiscard]] bool replayable(const std::string& presented) const {
        std::lock_guard<std::mutex> guard(served_mutex_);
        for (const auto& entry : served_) {
            if (entry.value == presented && entry.eligible) return true;
        }
        return false;
    }

    /// One coordinated refresh: a newer stored set wins over a stale
    /// re-refresh, concurrent 401s share one round, and a failed round fails
    /// every waiter exactly once. The round resolves to the fresh complete
    /// Authorization value.
    [[nodiscard]] Result<std::string, @ERROR@> refresh(const std::string& presented) {
        const auto* descriptor = detail::oauth_scheme(scheme_);
        if (descriptor == nullptr) {
            return failure<std::string>(@ERROR@::Kind::UnknownScheme, scheme_);
        }
        const std::string key = @SESSIONS@::replay_store_key(*descriptor, options_);
        std::shared_ptr<ReplayRound> round;
        bool leader = false;
        {
            std::lock_guard<std::mutex> guard(rounds_mutex_);
            if (auto stored = options_.store->load(key)) {
                const std::string value = replay_value(*stored);
                if (value != presented) return Result<std::string, @ERROR@>::success(value);
            }
            auto& slot = rounds_[key];
            if (!slot) {
                slot = std::make_shared<ReplayRound>();
                leader = true;
            }
            round = slot;
        }
        if (!leader) {
            std::unique_lock<std::mutex> waiter(round->mutex);
            round->signal.wait(waiter, [&round] { return round->done; });
            if (round->value) return Result<std::string, @ERROR@>::success(*round->value);
            return Result<std::string, @ERROR@>::failure(round->error);
        }
        // This caller owns the round: clear the stale entry and force one
        // acquisition through the wrapped session's own single-flight
        // machinery, then complete the round for every waiter.
        options_.store->clear(key);
        auto acquired = sessions_->client_credentials_token(scheme_, options_);
        {
            std::lock_guard<std::mutex> guard(round->mutex);
            if (acquired) {
                round->value = replay_value(acquired.value());
            } else {
                round->error = std::move(acquired).error();
            }
            round->done = true;
        }
        round->signal.notify_all();
        {
            std::lock_guard<std::mutex> guard(rounds_mutex_);
            rounds_.erase(key);
        }
        if (round->value) return Result<std::string, @ERROR@>::success(*round->value);
        return Result<std::string, @ERROR@>::failure(round->error);
    }

    /// The complete Authorization header value of a stored set.
    [[nodiscard]] static std::string replay_value(const @TOKEN_SET@& set) {
        std::string value = set.token_type.empty() ? std::string("Bearer") : set.token_type;
        value.push_back(' ');
        value += set.access_token;
        return value;
    }

    /// Maps one typed lifecycle failure onto the transport error surface the
    /// credential hook and the transport wrapper share. The message carries
    /// only the safe classification metadata; never token or secret values.
    [[nodiscard]] static TransportError transport_failure(const @ERROR@& error) {
        TransportError mapped;
        switch (error.kind) {
            case @ERROR@::Kind::Transport:
                mapped.kind = TransportError::Kind::Network;
                break;
            case @ERROR@::Kind::ResourceLimit:
                mapped.kind = TransportError::Kind::ResourceLimit;
                break;
            default:
                mapped.kind = TransportError::Kind::Configuration;
                break;
        }
        mapped.message = error.message();
        return mapped;
    }

    /// One coordinated refresh round: exactly one forced acquisition, shared
    /// by every concurrent 401 that presented the same stale token. Waiters
    /// block on the round's completion and surface its outcome exactly once.
    struct ReplayRound {
        std::mutex mutex;
        std::condition_variable signal;
        bool done = false;
        std::optional<std::string> value;
        @ERROR@ error;
    };

    std::string scheme_;
    @OPTIONS@ options_;
    std::shared_ptr<@SESSIONS@> sessions_;
    std::shared_ptr<const Transport> inner_;
    mutable std::mutex served_mutex_;
    std::vector<ServedAttach> served_;
    std::mutex rounds_mutex_;
    std::map<std::string, std::shared_ptr<ReplayRound>> rounds_;
"#;

/// The plain variant's lifecycle-endpoint exclusion: the compiled
/// token, refresh, code-token, device, revocation and introspection
/// endpoints.
const REPLAY_LIFECYCLE_PLAIN: &str = r#"
    /// The exact-target lifecycle guard: lifecycle endpoint requests are never
    /// replayed. They carry no bearer token of this wrapper, so this guard is
    /// defense in depth against loops.
    [[nodiscard]] bool lifecycle(const std::string& url) const {
        const auto* descriptor = detail::oauth_scheme(scheme_);
        if (descriptor == nullptr) return false;
        return (!descriptor->client_credentials_url.empty()
                && url == descriptor->client_credentials_url)
            || (!descriptor->refresh_url.empty() && url == descriptor->refresh_url)
            || (!descriptor->code_token_url.empty() && url == descriptor->code_token_url)
            || (!descriptor->device_url.empty() && url == descriptor->device_url)
            || (!descriptor->device_token_url.empty() && url == descriptor->device_token_url)
            || (!descriptor->revocation_url.empty() && url == descriptor->revocation_url)
            || (!descriptor->introspection_url.empty() && url == descriptor->introspection_url);
    }
"#;

/// The discovery variant's lifecycle-endpoint exclusion: the compiled
/// discovery URL joins the compiled endpoints; the discovery-resolved token
/// endpoint rides the exact-token match.
const REPLAY_LIFECYCLE_DISCOVERY: &str = r#"
    /// The exact-target lifecycle guard: lifecycle endpoint requests are never
    /// replayed. They carry no bearer token of this wrapper, so this guard is
    /// defense in depth against loops; the discovery-resolved token endpoint
    /// rides the exact-token match.
    [[nodiscard]] bool lifecycle(const std::string& url) const {
        const auto* descriptor = detail::oauth_scheme(scheme_);
        if (descriptor == nullptr) return false;
        return (!descriptor->client_credentials_url.empty()
                && url == descriptor->client_credentials_url)
            || (!descriptor->refresh_url.empty() && url == descriptor->refresh_url)
            || (!descriptor->code_token_url.empty() && url == descriptor->code_token_url)
            || (!descriptor->device_url.empty() && url == descriptor->device_url)
            || (!descriptor->device_token_url.empty() && url == descriptor->device_token_url)
            || (!descriptor->revocation_url.empty() && url == descriptor->revocation_url)
            || (!descriptor->introspection_url.empty() && url == descriptor->introspection_url)
            || (!descriptor->discovery_url.empty() && url == descriptor->discovery_url);
    }
"#;

/// The replaying transport's send body: defined out of line, after the
/// wrapper class it consults through the owning shared instance.
const REPLAY_WRAPPER_TAIL: &str = r#"};

inline Result<HttpResponse, TransportError> @REPLAY_TRANSPORT@::send(const HttpRequest& request,
    const TransportOptions& options) const {
    auto response = inner_->send(request, options);
    if (!response || response.value().status != 401) return response;
    std::string presented;
    for (const auto& [name, value] : request.headers) {
        if (oauth_replay_lower(name) == "authorization") presented = value;
    }
    if (presented.empty()) return response;
    if (!owner_->replayable(presented)) return response;
    // Lifecycle endpoint requests carry no bearer token of this provider, so
    // this exact-target guard is defense in depth against loops.
    if (owner_->lifecycle(request.url)) return response;
    auto fresh = owner_->refresh(presented);
    if (!fresh) {
        return Result<HttpResponse, TransportError>::failure(
            @REPLAY@::transport_failure(std::move(fresh).error()));
    }
    HttpRequest replayed = request;
    for (auto& [name, value] : replayed.headers) {
        if (oauth_replay_lower(name) == "authorization") value = fresh.value();
    }
    return inner_->send(replayed, options);
}
"#;

/// Case-folds one header name for the replaying transport's authorization
/// lookup, emitted ahead of the replaying transport so the send body can call
/// it. Header names are ASCII; the comparison never sees anything else.
const REPLAY_LOWER: &str = r#"/// Case-folds one header name for the replaying transport's authorization
/// lookup. Header names are ASCII; the comparison never sees anything else.
inline std::string oauth_replay_lower(std::string_view text) {
    std::string out;
    out.reserve(text.size());
    for (const char character : text) {
        out.push_back(character >= 'A' && character <= 'Z'
            ? static_cast<char>(character - 'A' + 'a')
            : character);
    }
    return out;
}

"#;
