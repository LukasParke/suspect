//! Emitted-only OAuth 2.0 / OpenID Connect token lifecycle for the Kotlin
//! coroutine client.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when SDK defaults are configured, exactly like pagination) lowers
//! into one generated `OAuth.kt`: the compiled scheme descriptors as frozen
//! constants, an instance-owned synchronized token store, client-credentials
//! acquisition with skew-aware caching, per-key `Mutex` single-flight and
//! atomic replacement, explicit refresh with rotated-refresh adoption,
//! authorization-code with PKCE S256 (`java.security.SecureRandom` +
//! `MessageDigest`), RFC 8628 device authorization with injectable pacing,
//! RFC 7009 revocation and RFC 7662 introspection — each exactly when the
//! compiled schemes carry it. Implicit and password flows are represented by
//! the plan for documentation only; they are never executed, so schemes with
//! only those flows emit nothing.
//!
//! A compiled discovery URL (the declared `openIdConnectUrl`, `oauth2MetadataUrl`
//! or the configured `discovery_url`) makes a scheme usable even with no
//! declared flows: OpenID Connect schemes have their endpoints defined by the
//! RFC 8414 / OpenID Connect discovery document at runtime. Emission is
//! byte-identical for plans without a discovery URL: every emitted section
//! keeps its pre-discovery bytes, and every discovery-aware section (the
//! header paragraph, the frozen discovery URL map, the discovery-aware flow
//! members and the discovery engine) is emitted only when at least one
//! compiled scheme carries a discovery URL.
//!
//! On top of the plain lifecycle, and only when a compiled scheme carries an
//! executable client-credentials flow, one opt-in replaying credential wrapper
//! joins the emitted module: it records which Authorization values its
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
//! byte-identical. The generated client gains one internal lifecycle accessor
//! (its token store is instance-owned) and every flow is also reachable as a
//! public extension function. Token endpoint requests run through the
//! client's own transport, credential values are read from the compiled
//! environment variable names at call time, and typed auth errors never carry
//! token or client-secret material.

use super::Plan;
use super::emit::quote;
use crate::http_protocol as wire;
use std::collections::{BTreeMap, BTreeSet};

/// The top-level identifiers `OAuth.kt` declares. Reserved against model and
/// operation symbol allocation while the file is emitted, so no-policy
/// allocation behavior is unchanged. `DiscoveryDocuments` joins the set only
/// when a compiled scheme carries a discovery URL, and the replaying
/// credential wrapper's surface joins only when a compiled scheme carries an
/// executable client-credentials flow, so the plain variants' allocation
/// behavior is unchanged.
pub(super) fn reserved_types(discovery: bool, replay: bool) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = RESERVED_TYPES.to_vec();
    if discovery {
        names.push("DiscoveryDocuments");
    }
    if replay {
        names.push("OAuthReplayCredentials");
        names.push("oauthNoReplayOperations");
        names.push("oauthReplayValue");
    }
    names
}

/// The base reserved identifiers, extended with the discovery map only for the
/// discovery-aware emission.
const RESERVED_TYPES: &[&str] = &[
    "AuthException",
    "AuthorizationTransaction",
    "DeviceAuthorization",
    "Introspection",
    "MemoryTokenStore",
    "OAuthDeviceGrant",
    "OAuthMaxResponseBytes",
    "OAuthSchemeDescriptor",
    "OAuthSessions",
    "OAuthSchemes",
    "TokenSet",
    "TokenStore",
    "oauthFormEncode",
    "oauthFormValue",
];

/// The generated client member `OAuth.kt`'s extensions delegate to. Reserved
/// against operation method allocation while the file is emitted.
pub(super) const RESERVED_MEMBER: &str = "oauth";

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

/// Whether the compiled plan admits at least one usable scheme, so `OAuth.kt`
/// carries content. A discovery URL makes a scheme usable even with no
/// declared flows: OpenID Connect schemes have their endpoints defined by the
/// discovery document at runtime.
pub(super) fn emittable(plan: &wire::OAuthPlan) -> bool {
    plan.mode != wire::OAuthMode::Off
        && plan
            .schemes
            .iter()
            .any(|scheme| has_discovery(scheme) || scheme.flows.iter().any(executable))
}

/// Whether one compiled scheme carries a discovery/metadata URL, which the
/// generated runtime resolves at call time.
pub(super) fn has_discovery(scheme: &wire::OAuthSchemePlan) -> bool {
    scheme.discovery.is_some()
}

/// Whether any compiled scheme carries a discovery URL, so the emitted module
/// carries the discovery engine and the conditional reserved names.
pub(super) fn has_discovery_among(plan: &wire::OAuthPlan) -> bool {
    plan.schemes.iter().any(has_discovery)
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so schemes without one compile
/// exactly the pre-replay bytes.
pub(super) fn has_client_credentials(plan: &wire::OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            flow.kind == wire::OAuthFlowDescriptorKind::ClientCredentials
                && !flow.deprecated_flow
                && flow.token_url.is_some()
        })
    })
}

/// One source scheme compiled down to executable endpoints.
#[derive(Debug, Clone)]
struct Lowered {
    name: String,
    client_secret_basic: bool,
    client_id_env: Option<String>,
    client_secret_env: Option<String>,
    skew_seconds: u32,
    issuer: String,
    discovery: Option<String>,
    refresh_url: Option<String>,
    client_credentials_url: Option<String>,
    authorization_url: Option<String>,
    code_token_url: Option<String>,
    device_url: Option<String>,
    device_token_url: Option<String>,
    revocation_url: Option<String>,
    introspection_url: Option<String>,
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

/// Lower every usable scheme. Schemes whose only declared flows are implicit
/// or password are never compiled here, so they emit nothing — unless a
/// discovery URL makes them usable, since OpenID Connect schemes define their
/// endpoints through the discovery document at runtime.
fn lower(oauth: &wire::OAuthPlan) -> Vec<Lowered> {
    let mut lowered = Vec::new();
    for scheme in &oauth.schemes {
        let flows: Vec<&wire::OAuthFlowDescriptor> = scheme
            .flows
            .iter()
            .filter(|flow| executable(flow))
            .collect();
        let discovery = scheme.discovery.clone();
        if flows.is_empty() && discovery.is_none() {
            continue;
        }
        let mut entry = Lowered {
            name: scheme.name.clone(),
            // A discovery-only scheme's client authentication follows the
            // compiled configuration; otherwise the first executable flow's
            // declared policy applies.
            client_secret_basic: flows.first().map_or_else(
                || scheme.client_secret_env.is_some(),
                |flow| flow.client_auth == wire::OAuthClientAuth::ClientSecretBasic,
            ),
            client_id_env: scheme.client_id_env.clone(),
            client_secret_env: scheme.client_secret_env.clone(),
            skew_seconds: scheme.refresh_skew_seconds,
            issuer: flows
                .first()
                .and_then(|flow| flow.token_url.as_deref())
                .map_or_else(
                    || discovery.as_deref().map_or_else(String::new, origin),
                    origin,
                ),
            discovery,
            refresh_url: flows
                .first()
                .and_then(|flow| flow.refresh_url.clone().or_else(|| flow.token_url.clone())),
            client_credentials_url: None,
            authorization_url: None,
            code_token_url: None,
            device_url: None,
            device_token_url: None,
            revocation_url: scheme.revocation_endpoint.clone(),
            introspection_url: scheme.introspection_endpoint.clone(),
        };
        for flow in flows {
            match flow.kind {
                wire::OAuthFlowDescriptorKind::ClientCredentials
                    if entry.client_credentials_url.is_none() =>
                {
                    entry.client_credentials_url = flow.token_url.clone();
                }
                wire::OAuthFlowDescriptorKind::AuthorizationCode
                    if entry.authorization_url.is_none() =>
                {
                    entry.authorization_url = flow.authorization_url.clone();
                    entry.code_token_url = flow.token_url.clone();
                }
                wire::OAuthFlowDescriptorKind::DeviceAuthorization
                    if entry.device_url.is_none() =>
                {
                    entry.device_url = flow.device_authorization_url.clone();
                    entry.device_token_url = flow.token_url.clone();
                }
                _ => {}
            }
        }
        lowered.push(entry);
    }
    lowered
}

fn has(lowered: &[Lowered], present: impl Fn(&Lowered) -> bool) -> bool {
    lowered.iter().any(present)
}

/// The generated client member `OAuth.kt`'s extensions delegate to. Its token
/// store is instance-owned; keys stay partitioned by scheme, token-endpoint
/// issuer and client identity.
pub(super) fn client_member() -> &'static str {
    "    /** Generated OAuth lifecycle for this client; see OAuth.kt. The token\n    * store is owned by this client instance, with keys partitioned by source\n    * scheme, token-endpoint issuer and client identity. */\n    internal val oauth: OAuthSessions by lazy { OAuthSessions(transport) }\n"
}

/// Render `OAuth.kt`. Called only when at least one usable scheme exists.
/// Plans without a discovery URL assemble byte-identically to the
/// pre-discovery emission; plans with one emit the discovery-aware flow
/// members and the discovery engine.
pub(super) fn runtime(plan: &Plan, oauth: &wire::OAuthPlan) -> String {
    let lowered = lower(oauth);
    debug_assert!(!lowered.is_empty(), "emission is gated on usable schemes");
    let discovery = lowered.iter().any(|scheme| scheme.discovery.is_some());
    // The replaying credential wrapper joins only when a compiled scheme
    // carries an executable client-credentials flow, the provider it wraps.
    let replay = lowered
        .iter()
        .any(|scheme| scheme.client_credentials_url.is_some());
    let credentials = has(&lowered, |scheme| {
        scheme.client_credentials_url.is_some() || scheme.discovery.is_some()
    });
    let refresh = has(&lowered, |scheme| {
        scheme.discovery.is_some()
            || scheme
                .refresh_url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
    });
    let device = has(&lowered, |scheme| {
        scheme.device_url.is_some() && scheme.device_token_url.is_some()
    });
    let authorization = has(&lowered, |scheme| {
        scheme.authorization_url.is_some() && scheme.code_token_url.is_some()
    });
    let revocation = has(&lowered, |scheme| scheme.revocation_url.is_some());
    let introspection = has(&lowered, |scheme| scheme.introspection_url.is_some());

    let mut out = super::emit::header(plan);
    out.push_str("import java.net.URI\nimport java.nio.charset.StandardCharsets\nimport java.time.Duration\nimport java.util.Base64\nimport kotlinx.coroutines.CancellationException\nimport kotlinx.coroutines.sync.Mutex\nimport kotlinx.coroutines.sync.withLock\n");
    if device {
        out.push_str("import kotlinx.coroutines.delay\n");
    }
    if replay {
        out.push_str("import kotlinx.coroutines.CompletableDeferred\n");
    }
    if authorization {
        out.push_str("import java.security.MessageDigest\nimport java.security.SecureRandom\n");
    }
    out.push('\n');
    out.push_str(if discovery { HEAD_DISCOVERY } else { HEAD });
    if replay {
        out.push_str(HEAD_REPLAY);
    }
    out.push_str(CORE_TYPES);
    if device {
        out.push_str(DEVICE_TYPE);
    }
    if introspection {
        out.push_str(INTROSPECTION_TYPE);
    }
    if authorization {
        out.push_str(AUTHORIZATION_TYPE);
    }
    out.push_str(&descriptors(&lowered, discovery));
    if discovery {
        out.push_str(&discovery_documents(&lowered));
    }
    out.push_str(SESSIONS_HEAD);
    if replay {
        out.push_str(REPLAY_STORE_KEY);
    }
    if discovery {
        out.push_str(DISCOVERY_FIELDS);
    }
    if credentials {
        out.push_str(if discovery {
            CLIENT_CREDENTIALS_DISCOVERY
        } else {
            CLIENT_CREDENTIALS_METHOD
        });
    }
    if refresh {
        out.push_str(if discovery {
            REFRESH_DISCOVERY
        } else {
            REFRESH_METHOD
        });
    }
    if device {
        out.push_str(DEVICE_METHODS);
    }
    if revocation {
        out.push_str(if discovery {
            REVOCATION_DISCOVERY
        } else {
            REVOCATION_METHOD
        });
    }
    if introspection {
        out.push_str(if discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION_METHOD
        });
    }
    if authorization {
        out.push_str(AUTHORIZATION_METHODS);
    }
    if discovery {
        out.push_str(DISCOVERY_ENGINE);
    }
    out.push_str(SESSIONS_PRIVATE);
    out.push_str(SESSIONS_TAIL);
    if replay {
        out.push_str(&replay_section(&lowered, plan.operations(), discovery));
    }
    out.push_str(CLIENT_EXTENSIONS_HEAD);
    if credentials {
        out.push_str(CLIENT_CREDENTIALS_EXTENSION);
    }
    if refresh {
        out.push_str(REFRESH_EXTENSION);
    }
    if device {
        out.push_str(DEVICE_EXTENSIONS);
    }
    if revocation {
        out.push_str(REVOCATION_EXTENSION);
    }
    if introspection {
        out.push_str(INTROSPECTION_EXTENSION);
    }
    if authorization {
        out.push_str(AUTHORIZATION_EXTENSIONS);
    }
    out
}

/// The compiled descriptor map: one entry per usable source scheme. Plans
/// without a discovery URL keep the pre-discovery bytes; the discovery-aware
/// variant's comment says so.
fn descriptors(lowered: &[Lowered], discovery: bool) -> String {
    let schemes_doc = if discovery {
        "/** The compiled descriptors for this package's usable source schemes. The\n * runtime never parses OpenAPI; a compiled discovery URL resolves the\n * endpoint URLs the compiled plan omits at call time. */\n"
    } else {
        "/** The compiled descriptors for this package's usable source schemes. The\n * runtime never parses OpenAPI and never performs discovery. */\n"
    };
    let mut out = String::from(
        "/** The compiled lifecycle descriptor of one source scheme. Empty endpoint\n * members mean \"not compiled\": endpoints are never invented and supplemental\n * endpoints exist only when configuration supplied them. */\ninternal class OAuthSchemeDescriptor(\n    internal val name: String,\n    /** Token-endpoint requests authenticate with the configured secret variable (client_secret_basic); otherwise the client is public. */\n    internal val clientSecretBasic: Boolean,\n    /** The compiled environment variable names, read at call time. */\n    internal val clientIDEnv: String,\n    internal val clientSecretEnv: String,\n    /** Refresh-before-expiry clock skew in seconds. */\n    internal val skewSeconds: Int,\n    /** The issuing authority origin that store keys partition on. */\n    internal val issuer: String,\n    /** The declared refresh endpoint, else the token endpoint. */\n    internal val refreshURL: String,\n    /** Per-flow endpoints, empty when the flow is absent or not executable. */\n    internal val clientCredentialsURL: String,\n    internal val authorizationURL: String,\n    internal val codeTokenURL: String,\n    internal val deviceURL: String,\n    internal val deviceTokenURL: String,\n    /** Configuration-supplied supplemental endpoints. */\n    internal val revocationURL: String,\n    internal val introspectionURL: String,\n)\n\n",
    );
    out.push_str(schemes_doc);
    out.push_str(
        "internal object OAuthSchemes {\n    internal val schemes: Map<String, OAuthSchemeDescriptor> = mapOf(\n",
    );
    for scheme in lowered {
        let text = |value: &Option<String>| match value {
            Some(url) => quote(url),
            None => String::from("\"\""),
        };
        out.push_str(&format!(
            "        {} to OAuthSchemeDescriptor({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}),\n",
            quote(&scheme.name),
            quote(&scheme.name),
            scheme.client_secret_basic,
            text(&scheme.client_id_env),
            text(&scheme.client_secret_env),
            scheme.skew_seconds,
            quote(&scheme.issuer),
            text(&scheme.refresh_url),
            text(&scheme.client_credentials_url),
            text(&scheme.authorization_url),
            text(&scheme.code_token_url),
            text(&scheme.device_url),
            text(&scheme.device_token_url),
            text(&scheme.revocation_url),
            text(&scheme.introspection_url),
        ));
    }
    out.push_str(
        "    )\n\n    /** The compiled descriptor for one source scheme name; a typed refusal\n     * otherwise. */\n    internal fun descriptor(scheme: String): OAuthSchemeDescriptor =\n        schemes[scheme] ?: throw AuthException(\"unknown-scheme\", scheme)\n}\n\n",
    );
    out
}

/// The compiled discovery/metadata document URLs, emitted only when at least
/// one usable scheme carries one. A separate frozen map keeps the plain
/// variant's descriptor class and entries byte-identical.
fn discovery_documents(lowered: &[Lowered]) -> String {
    let mut out = String::from(
        "/** The compiled RFC 8414 / OpenID Connect discovery document URLs, keyed by\n * source scheme name. Schemes absent here resolve only their compiled\n * endpoints. */\ninternal object DiscoveryDocuments {\n    internal val documents: Map<String, String> = mapOf(\n",
    );
    for scheme in lowered.iter().filter(|scheme| scheme.discovery.is_some()) {
        out.push_str(&format!(
            "        {} to {},\n",
            quote(&scheme.name),
            match &scheme.discovery {
                Some(url) => quote(url),
                None => String::from("\"\""),
            },
        ));
    }
    out.push_str("    )\n}\n\n");
    out
}

/// The generated module's ownership and safety documentation.
const HEAD: &str = "/** Generated first-party OAuth 2.0 / OpenID Connect token lifecycle.\n *\n * Every endpoint, environment variable name and policy constant in\n * [OAuthSchemes] is a generation-time constant compiled from the used security\n * schemes of this package's source document. This module never parses OpenAPI,\n * never performs discovery and never invents an endpoint. Client identity\n * comes from explicit call arguments or the compiled environment variable\n * names, read at call time: credential values are never embedded in emitted\n * bytes, and token or client-secret values never enter error metadata.\n *\n * Implemented here, exactly for the schemes compiled below:\n * client-credentials acquisition with skew-aware caching, per-key\n * single-flight rounds and atomic store replacement, explicit refresh with\n * rotated-refresh adoption, authorization-code with PKCE S256, RFC 8628 device\n * authorization with injectable pacing, RFC 7009 revocation and RFC 7662\n * introspection. Deprecated implicit and password flows are never executed.\n * OpenID Connect discovery remains caller-owned.\n *\n * Token sets live only in the token store owned by each [OAuthSessions]\n * instance (or one explicitly passed per call), under keys partitioned by\n * source scheme, token-endpoint issuer and client identity; there is no\n * process-global token cache. Token endpoint requests use the owning client's\n * transport, so caller transport policy covers the lifecycle too.\n */\n\n";

/// The generated module's ownership and safety documentation for plans with a
/// discovery URL: the discovery paragraphs replace the plain ones.
const HEAD_DISCOVERY: &str = "/** Generated first-party OAuth 2.0 / OpenID Connect token lifecycle.\n *\n * Every endpoint, environment variable name and policy constant in\n * [OAuthSchemes] is a generation-time constant compiled from the used security\n * schemes of this package's source document. This module never parses OpenAPI;\n * endpoint URLs that the compiled flows omit resolve through RFC 8414 /\n * OpenID Connect discovery when the scheme compiles a discovery URL, and no\n * endpoint is ever invented. Client identity\n * comes from explicit call arguments or the compiled environment variable\n * names, read at call time: credential values are never embedded in emitted\n * bytes, and token or client-secret values never enter error metadata.\n *\n * Implemented here, exactly for the schemes compiled below:\n * client-credentials acquisition with skew-aware caching, per-key\n * single-flight rounds and atomic store replacement, explicit refresh with\n * rotated-refresh adoption, authorization-code with PKCE S256, RFC 8628 device\n * authorization with injectable pacing, RFC 7009 revocation and RFC 7662\n * introspection. Deprecated implicit and password flows are never executed.\n * A compiled discovery URL resolves the token, revocation and introspection\n * endpoints the compiled plan omits; a compiled endpoint always wins.\n *\n * Token sets live only in the token store owned by each [OAuthSessions]\n * instance (or one explicitly passed per call), under keys partitioned by\n * source scheme, token-endpoint issuer and client identity; there is no\n * process-global token cache. Token and discovery requests use the owning\n * client's transport, so caller transport policy covers the lifecycle too.\n */\n\n";

const CORE_TYPES: &str = "/** One issued token and its metadata, exactly as the compiled token endpoint\n * returned it (RFC 6749 section 5.1). Every member is credential material:\n * never print, log or stream token values; `toString` is the safe summary. */\npublic data class TokenSet(\n    /** The access token. */\n    public val accessToken: String,\n    /** The server's token type, or the conventional Bearer. */\n    public val tokenType: String = \"Bearer\",\n    /** Issue time plus the returned `expires_in`, in epoch milliseconds. Zero\n     * means the server declared no lifetime, so the set never expires locally. */\n    public val expiresAt: Long = 0,\n    /** The server's rotated refresh token, or the previous set's token when the\n     * server returned none. */\n    public val refreshToken: String? = null,\n    /** The granted scope, as the server returned it. */\n    public val scope: String? = null,\n) {\n    /** Skew-aware staleness: whether `nowMillis` has reached the expiry minus\n     * the compiled clock skew. */\n    public fun expired(skewSeconds: Int, nowMillis: Long = System.currentTimeMillis()): Boolean =\n        expiresAt > 0 && nowMillis >= expiresAt - skewSeconds * 1000L\n\n    /** Safe summary carrying no credential material. */\n    override fun toString(): String = \"TokenSet(hasRefresh=${refreshToken != null}, scope=$scope)\"\n}\n\n/** Caller-owned token storage behind the generated lifecycle. Keys are\n * partitioned by source scheme, token-endpoint issuer and client identity;\n * treat them as read-only routing information, and make `replace` atomic per\n * key. */\npublic interface TokenStore {\n    /** The stored token for `key`, or null. */\n    public fun load(key: String): TokenSet?\n\n    /** Atomically replace the stored token for `key`. */\n    public fun replace(key: String, token: TokenSet)\n\n    /** Drop the stored token for `key`; clearing an absent key succeeds. */\n    public fun clear(key: String)\n}\n\n/** Instance-owned in-process token store guarded by a monitor. Each instance\n * guards its own keys; generated code never shares a store implicitly and\n * there is no process-global token cache. */\npublic class MemoryTokenStore : TokenStore {\n    private val tokens = HashMap<String, TokenSet>()\n\n    override fun load(key: String): TokenSet? = synchronized(this) { tokens[key] }\n\n    override fun replace(key: String, token: TokenSet): Unit = synchronized(this) {\n        tokens[key] = token\n    }\n\n    override fun clear(key: String): Unit = synchronized(this) { tokens.remove(key) }\n}\n\n/** Typed OAuth lifecycle failure. `kind` and `scheme` classify it, `code`\n * carries the authorization server's declared error code and `status` the\n * endpoint HTTP status when one was reached. Messages never contain token or\n * client-secret values, and never response bodies. */\npublic class AuthException(\n    /** Stable failure kind. */\n    public val kind: String,\n    /** The source security scheme the failure belongs to. */\n    public val scheme: String,\n    /** The server's declared `error` code, when one exists. */\n    public val code: String? = null,\n    /** The endpoint HTTP status, when one was reached. */\n    public val status: Int? = null,\n    cause: Throwable? = null,\n) : RuntimeException(\n    buildString {\n        append(\"OAuth \")\n        append(kind)\n        append(\" failure\")\n        if (scheme.isNotEmpty()) {\n            append(\" for scheme \")\n            append(scheme)\n        }\n        code?.let {\n            append(\" (server code: \")\n            append(it)\n            append(\")\")\n        }\n    },\n    cause,\n) {\n    override fun toString(): String = message ?: \"AuthException\"\n}\n\n/** Bounded response ceiling for one token/device/revocation/introspection\n * response body. */\ninternal const val OAuthMaxResponseBytes: Int = 1 shl 20\n\n/** The RFC 8628 device grant type. */\ninternal const val OAuthDeviceGrant: String = \"urn:ietf:params:oauth:grant-type:device_code\"\n\n/** application/x-www-form-urlencoded with RFC 3986 unreserved bytes kept\n * literal and spaces as %20. */\ninternal fun oauthFormValue(value: String): String {\n    val hex = \"0123456789ABCDEF\"\n    val out = StringBuilder(value.length)\n    for (byte in value.toByteArray(StandardCharsets.UTF_8)) {\n        val code = byte.toInt() and 0xFF\n        val unreserved = code in 0x41..0x5A || code in 0x61..0x7A || code in 0x30..0x39 ||\n            code == '-'.code || code == '.'.code || code == '_'.code || code == '~'.code\n        if (unreserved) {\n            out.append(code.toChar())\n        } else {\n            out.append('%').append(hex[code shr 4]).append(hex[code and 0xF])\n        }\n    }\n    return out.toString()\n}\n\n/** Sorted deterministic form encoding; absent optional entries are simply not\n * sent. */\ninternal fun oauthFormEncode(fields: Map<String, String>): String =\n    fields.entries.sortedBy { it.key }\n        .joinToString(\"&\") { entry -> oauthFormValue(entry.key) + \"=\" + oauthFormValue(entry.value) }\n\n";

const DEVICE_TYPE: &str = "/** One started RFC 8628 device-authorization transaction. Present the user\n * code and verification URI, then poll the token endpoint for the granted\n * set. The device code never appears in `toString`. */\npublic class DeviceAuthorization(\n    /** The source scheme this grant belongs to. */\n    public val scheme: String,\n    /** The code the user enters at the verification URI. */\n    public val userCode: String,\n    /** Where the user approves the device grant. */\n    public val verificationURI: String,\n    /** When declared, carries the user code in the URL. */\n    public val verificationURIComplete: String? = null,\n    /** The server-declared transaction expiry in epoch milliseconds; zero\n     * means the server declared no lifetime and only its own `expired_token`\n     * answer bounds polling. */\n    public val expiresAt: Long = 0,\n    /** The polling floor in milliseconds; the server's `slow_down` answers\n     * extend it. */\n    public val intervalMillis: Long = 5000,\n    /** Credential material: never print, log or stream. */\n    internal val deviceCode: String,\n) {\n    override fun toString(): String =\n        \"DeviceAuthorization(scheme=$scheme, userCode=$userCode, verificationURI=$verificationURI)\"\n}\n\n";

const INTROSPECTION_TYPE: &str = "/** One RFC 7662 introspection response. It describes the token without\n * returning it. Epoch fields are server-declared; null means the server\n * omitted the claim. */\npublic class Introspection(\n    public val active: Boolean,\n    public val scope: String? = null,\n    public val clientID: String? = null,\n    public val tokenType: String? = null,\n    public val username: String? = null,\n    public val expiresAt: Long? = null,\n    public val issuedAt: Long? = null,\n    public val notBefore: Long? = null,\n    public val subject: String? = null,\n    public val audience: List<String> = emptyList(),\n    public val issuer: String? = null,\n    public val jwtID: String? = null,\n)\n\n";

const AUTHORIZATION_TYPE: &str = "/** One bound, single-use authorization-code transaction with a PKCE S256\n * verifier. The verifier and state leave this process only through\n * `authorizationURL` and the code exchange. */\npublic class AuthorizationTransaction internal constructor(\n    /** The source security scheme. */\n    public val scheme: String,\n    /** Direct the resource owner here; it carries the response type, client\n     * id, redirect URI, state and the S256 code challenge. */\n    public val authorizationURL: String,\n    /** Must match the callback's `state` parameter exactly; consumed by the\n     * first `completeAuthorization` call. */\n    public val state: String,\n    /** The bound redirect URI, repeated exactly on the code exchange when\n     * non-empty. */\n    public val redirectURI: String,\n    /** The requested scopes. */\n    public val scopes: List<String>,\n    internal val codeVerifier: String,\n    internal val codeTokenURL: String,\n) {\n    @Volatile\n    internal var consumed: Boolean = false\n\n    override fun toString(): String = \"AuthorizationTransaction(scheme=$scheme, state=$state)\"\n}\n\n";

const SESSIONS_HEAD: &str = "/** One owner's generated OAuth lifecycle: an instance-owned default token\n * store, per-key single-flight acquisition gates and the compiled endpoints.\n * Every token value lives only inside a store instance; there is no\n * process-global token cache and no global mutable state. */\npublic class OAuthSessions(\n    /** The transport every token, device, revocation and introspection request\n     * uses; the owning client's transport covers the lifecycle too. */\n    private val transport: Transport,\n    /** The instance-owned default store; per-call `tokenStore` arguments\n     * override it. */\n    private val defaultStore: TokenStore = MemoryTokenStore(),\n) {\n    private val gateMutex = Mutex()\n    private val gates = HashMap<String, Mutex>()\n";

const CLIENT_CREDENTIALS_METHOD: &str = "\n    /** The scheme's cached token set, acquiring one from the compiled\n     * client-credentials endpoint when the stored set is absent or expired\n     * beyond the compiled skew. Concurrent callers on one owner share a single\n     * acquisition: callers waiting on the gate re-read the store instead of\n     * acquiring again. */\n    public suspend fun clientCredentialsToken(\n        scheme: String,\n        clientID: String? = null,\n        clientSecret: String? = null,\n        tokenStore: TokenStore? = null,\n    ): TokenSet {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        if (descriptor.clientCredentialsURL.isEmpty()) {\n            throw AuthException(\"unsupported-flow\", scheme)\n        }\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val resolvedStore = tokenStore ?: defaultStore\n        val key = storeKey(descriptor, resolved.first)\n        fresh(resolvedStore, key, descriptor)?.let { return it }\n        val gate = entryGate(key)\n        gate.withLock {\n            // A caller that waited on the gate re-checks the store before\n            // acquiring: the holder may have populated it already.\n            fresh(resolvedStore, key, descriptor)?.let { return it }\n            val held = resolvedStore.load(key)\n            val token = tokenRequest(\n                descriptor,\n                descriptor.clientCredentialsURL,\n                linkedMapOf(\"grant_type\" to \"client_credentials\"),\n                held,\n                resolved.first,\n                resolved.second,\n            )\n            resolvedStore.replace(key, token)\n            return token\n        }\n    }\n";

const REFRESH_METHOD: &str = "\n    /** Exchange the set's refresh token (RFC 6749 section 6) at the scheme's\n     * declared refresh endpoint, or its token endpoint when none is declared.\n     * The returned set adopts a rotated refresh token and retains the given\n     * one otherwise. The store is neither read nor updated: callers decide\n     * which set to keep. */\n    public suspend fun refreshToken(\n        scheme: String,\n        token: TokenSet,\n        clientID: String? = null,\n        clientSecret: String? = null,\n    ): TokenSet {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        val refresh = token.refreshToken ?: throw AuthException(\"no-refresh-token\", scheme)\n        if (descriptor.refreshURL.isEmpty()) throw AuthException(\"unsupported-flow\", scheme)\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        return tokenRequest(\n            descriptor,\n            descriptor.refreshURL,\n            linkedMapOf(\"grant_type\" to \"refresh_token\", \"refresh_token\" to refresh),\n            token,\n            resolved.first,\n            resolved.second,\n        )\n    }\n";

const DEVICE_METHODS: &str = "\n    /** Starts a device-authorization transaction against the compiled device\n     * endpoint (RFC 8628 sections 3.1-3.2). One network call; present the\n     * returned user code and verification URI to the user. */\n    public suspend fun beginDeviceAuthorization(\n        scheme: String,\n        clientID: String? = null,\n        clientSecret: String? = null,\n    ): DeviceAuthorization {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        if (descriptor.deviceURL.isEmpty() || descriptor.deviceTokenURL.isEmpty()) {\n            throw AuthException(\"unsupported-flow\", scheme)\n        }\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val id = resolved.first ?: throw AuthException(\"missing-client-credentials\", scheme)\n        val response = endpointRequest(\n            descriptor,\n            descriptor.deviceURL,\n            linkedMapOf(\"client_id\" to id),\n            resolved.second,\n            resolved.first,\n        )\n        val members = oauthPayload(response, descriptor)\n        val deviceCode = (members[\"device_code\"] as? JsonString)?.value\n        val userCode = (members[\"user_code\"] as? JsonString)?.value\n        val verificationURI = (members[\"verification_uri\"] as? JsonString)?.value\n        val expiresIn = (members[\"expires_in\"] as? JsonNumber)?.token?.toLongOrNull()\n        if (deviceCode.isNullOrEmpty() || userCode.isNullOrEmpty() || verificationURI.isNullOrEmpty() ||\n            expiresIn == null || expiresIn <= 0\n        ) {\n            throw AuthException(\"invalid-response\", scheme, status = response.status)\n        }\n        val interval = (members[\"interval\"] as? JsonNumber)?.token?.toLongOrNull()\n        return DeviceAuthorization(\n            scheme = scheme,\n            userCode = userCode,\n            verificationURI = verificationURI,\n            verificationURIComplete = (members[\"verification_uri_complete\"] as? JsonString)?.value,\n            expiresAt = System.currentTimeMillis() + expiresIn * 1000,\n            intervalMillis = interval?.takeIf { it > 0 }?.times(1000) ?: 5000L,\n            deviceCode = deviceCode,\n        )\n    }\n\n    /** Polls the compiled token endpoint until the user approves the device\n     * grant, the transaction's declared expiry passes or the caller cancels\n     * (RFC 8628 section 3.5). `authorization_pending` waits the declared\n     * interval and retries, `slow_down` extends the interval by five seconds\n     * per answer, and the granted set atomically replaces the resolved\n     * store's entry. `wait` replaces the sleeper (tests pass a no-op). */\n    public suspend fun pollDeviceToken(\n        device: DeviceAuthorization,\n        clientID: String? = null,\n        clientSecret: String? = null,\n        tokenStore: TokenStore? = null,\n        wait: suspend (Long) -> Unit = { delay(it) },\n    ): TokenSet {\n        val descriptor = OAuthSchemes.descriptor(device.scheme)\n        if (descriptor.deviceTokenURL.isEmpty()) {\n            throw AuthException(\"unsupported-flow\", device.scheme)\n        }\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val resolvedStore = tokenStore ?: defaultStore\n        val key = storeKey(descriptor, resolved.first)\n        var interval = if (device.intervalMillis > 0) device.intervalMillis else 5000L\n        val fields = linkedMapOf(\"grant_type\" to OAuthDeviceGrant, \"device_code\" to device.deviceCode)\n        while (true) {\n            if (device.expiresAt > 0 && System.currentTimeMillis() >= device.expiresAt) {\n                throw AuthException(\"device-flow-expired\", device.scheme)\n            }\n            try {\n                val token = tokenRequest(\n                    descriptor,\n                    descriptor.deviceTokenURL,\n                    fields,\n                    null,\n                    resolved.first,\n                    resolved.second,\n                )\n                resolvedStore.replace(key, token)\n                return token\n            } catch (error: AuthException) {\n                when (error.code) {\n                    \"authorization_pending\" -> Unit\n                    \"slow_down\" -> interval += 5000\n                    else -> throw error\n                }\n            }\n            wait(interval)\n        }\n    }\n";

const REVOCATION_METHOD: &str = "\n    /** Posts the token value to the scheme's compiled revocation endpoint\n     * (RFC 7009). Any 2xx response is success: RFC 7009 declares the token\n     * revoked even when the server reports an unsupported-token error. A\n     * successful revocation clears the resolved store's partition for this\n     * client identity. */\n    public suspend fun revokeToken(\n        scheme: String,\n        token: String,\n        tokenTypeHint: String? = null,\n        clientID: String? = null,\n        clientSecret: String? = null,\n        tokenStore: TokenStore? = null,\n    ) {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        if (descriptor.revocationURL.isEmpty()) throw AuthException(\"unsupported-flow\", scheme)\n        if (token.isEmpty()) throw AuthException(\"invalid-token\", scheme)\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val fields = linkedMapOf(\"token\" to token)\n        tokenTypeHint?.takeIf { it.isNotEmpty() }?.let { fields[\"token_type_hint\"] = it }\n        val response = endpointRequest(descriptor, descriptor.revocationURL, fields, resolved.second, resolved.first)\n        if (response.status in 200..299) {\n            (tokenStore ?: defaultStore).clear(storeKey(descriptor, resolved.first))\n            return\n        }\n        val declared = try {\n            (oauthPayload(response, descriptor)[\"error\"] as? JsonString)?.value?.takeIf { it.isNotEmpty() }\n        } catch (error: AuthException) {\n            null\n        }\n        if (declared != null) {\n            throw AuthException(\"server-rejected\", scheme, code = declared, status = response.status)\n        }\n        throw AuthException(\"server-error\", scheme, status = response.status)\n    }\n";

const INTROSPECTION_METHOD: &str = "\n    /** Queries the scheme's compiled introspection endpoint (RFC 7662) with\n     * the token value. The response describes the token without returning it;\n     * any non-2xx answer is a typed failure carrying no token values. */\n    public suspend fun introspectToken(\n        scheme: String,\n        token: String,\n        tokenTypeHint: String? = null,\n        clientID: String? = null,\n        clientSecret: String? = null,\n    ): Introspection {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        if (descriptor.introspectionURL.isEmpty()) throw AuthException(\"unsupported-flow\", scheme)\n        if (token.isEmpty()) throw AuthException(\"invalid-token\", scheme)\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val fields = linkedMapOf(\"token\" to token)\n        tokenTypeHint?.takeIf { it.isNotEmpty() }?.let { fields[\"token_type_hint\"] = it }\n        val response = endpointRequest(descriptor, descriptor.introspectionURL, fields, resolved.second, resolved.first)\n        if (response.status !in 200..299) {\n            throw AuthException(\"server-error\", scheme, status = response.status)\n        }\n        val members = oauthPayload(response, descriptor)\n        val audience = when (val declared = members[\"aud\"]) {\n            is JsonString -> listOf(declared.value)\n            is JsonArray -> declared.values.mapNotNull { value -> (value as? JsonString)?.value }\n            else -> emptyList()\n        }\n        return Introspection(\n            active = (members[\"active\"] as? JsonBoolean)?.value == true,\n            scope = (members[\"scope\"] as? JsonString)?.value,\n            clientID = (members[\"client_id\"] as? JsonString)?.value,\n            tokenType = (members[\"token_type\"] as? JsonString)?.value,\n            username = (members[\"username\"] as? JsonString)?.value,\n            expiresAt = (members[\"exp\"] as? JsonNumber)?.token?.toLongOrNull(),\n            issuedAt = (members[\"iat\"] as? JsonNumber)?.token?.toLongOrNull(),\n            notBefore = (members[\"nbf\"] as? JsonNumber)?.token?.toLongOrNull(),\n            subject = (members[\"sub\"] as? JsonString)?.value,\n            audience = audience,\n            issuer = (members[\"iss\"] as? JsonString)?.value,\n            jwtID = (members[\"jti\"] as? JsonString)?.value,\n        )\n    }\n";

const AUTHORIZATION_METHODS: &str = "\n    /** The shared `java.security.SecureRandom` source for PKCE verifiers and\n     * states. */\n    private fun oauthRandom(bytes: Int): String {\n        val raw = ByteArray(bytes)\n        SecureRandom().nextBytes(raw)\n        return Base64.getUrlEncoder().withoutPadding().encodeToString(raw)\n    }\n    /** Starts an authorization-code transaction with PKCE S256: it allocates\n     * the state and verifier from `java.security.SecureRandom`, binds them to\n     * the returned transaction and renders the complete authorization URL. It\n     * performs no network call; direct the user to `authorizationURL` and\n     * complete the transaction with the callback parameters. */\n    public fun beginAuthorization(\n        scheme: String,\n        redirectURI: String,\n        scopes: List<String> = emptyList(),\n        state: String? = null,\n        clientID: String? = null,\n    ): AuthorizationTransaction {\n        val descriptor = OAuthSchemes.descriptor(scheme)\n        if (descriptor.authorizationURL.isEmpty() || descriptor.codeTokenURL.isEmpty()) {\n            throw AuthException(\"unsupported-flow\", scheme)\n        }\n        val resolved = resolveCredentials(descriptor, clientID, null)\n        val id = resolved.first ?: throw AuthException(\"missing-client-credentials\", scheme)\n        val verifier = oauthRandom(64)\n        val challenge = Base64.getUrlEncoder().withoutPadding().encodeToString(\n            MessageDigest.getInstance(\"SHA-256\")\n                .digest(verifier.toByteArray(StandardCharsets.US_ASCII)),\n        )\n        val chosen = state?.takeIf { it.isNotEmpty() } ?: oauthRandom(16)\n        val query = linkedMapOf(\n            \"response_type\" to \"code\",\n            \"client_id\" to id,\n            \"redirect_uri\" to redirectURI,\n            \"state\" to chosen,\n            \"code_challenge\" to challenge,\n            \"code_challenge_method\" to \"S256\",\n        )\n        if (scopes.isNotEmpty()) query[\"scope\"] = scopes.joinToString(\" \")\n        val encoded = query.entries\n            .joinToString(\"&\") { entry -> oauthFormValue(entry.key) + \"=\" + oauthFormValue(entry.value) }\n        val separator = if (descriptor.authorizationURL.contains('?')) \"&\" else \"?\"\n        return AuthorizationTransaction(\n            scheme,\n            descriptor.authorizationURL + separator + encoded,\n            chosen,\n            redirectURI,\n            scopes.toList(),\n            verifier,\n            descriptor.codeTokenURL,\n        )\n    }\n\n    /** Validates the callback's `state` against the transaction's bound\n     * state, exchanges the code with the retained PKCE verifier over the\n     * compiled token URL and client authentication, and returns the resulting\n     * token set, which replaces the resolved store's entry atomically. The\n     * transaction is consumed by the first call, whatever its outcome: a\n     * second call is a typed state failure. */\n    public suspend fun completeAuthorization(\n        transaction: AuthorizationTransaction,\n        callbackParams: Map<String, String>,\n        clientID: String? = null,\n        clientSecret: String? = null,\n        tokenStore: TokenStore? = null,\n    ): TokenSet {\n        val consumed = synchronized(transaction) {\n            val was = transaction.consumed\n            transaction.consumed = true\n            was\n        }\n        if (consumed) throw AuthException(\"transaction-used\", transaction.scheme)\n        if (!MessageDigest.isEqual(\n                callbackParams[\"state\"]?.toByteArray(StandardCharsets.UTF_8),\n                transaction.state.toByteArray(StandardCharsets.UTF_8),\n            )\n        ) {\n            throw AuthException(\"state-mismatch\", transaction.scheme)\n        }\n        callbackParams[\"error\"]?.takeIf { it.isNotEmpty() }?.let {\n            throw AuthException(\"authorization-denied\", transaction.scheme, code = it)\n        }\n        val code = callbackParams[\"code\"]?.takeIf { it.isNotEmpty() }\n            ?: throw AuthException(\"invalid-callback\", transaction.scheme)\n        val descriptor = OAuthSchemes.descriptor(transaction.scheme)\n        val resolved = resolveCredentials(descriptor, clientID, clientSecret)\n        val fields = linkedMapOf(\n            \"grant_type\" to \"authorization_code\",\n            \"code\" to code,\n            \"code_verifier\" to transaction.codeVerifier,\n        )\n        if (transaction.redirectURI.isNotEmpty()) fields[\"redirect_uri\"] = transaction.redirectURI\n        val token = tokenRequest(\n            descriptor,\n            transaction.codeTokenURL,\n            fields,\n            null,\n            resolved.first,\n            resolved.second,\n        )\n        (tokenStore ?: defaultStore).replace(storeKey(descriptor, resolved.first), token)\n        return token\n    }\n";

const SESSIONS_PRIVATE: &str = "\n    /** The single-flight gate for one store key: the first caller acquires,\n     * later callers wait and then re-read the store. */\n    private suspend fun entryGate(key: String): Mutex = gateMutex.withLock {\n        gates.getOrPut(key) { Mutex() }\n    }\n";

/// The extra `OAuthSessions` fields for the discovery variant: the per-instance
/// cache keyed by scheme and its single-flight gates. Never emitted for plans
/// without a discovery URL.
const DISCOVERY_FIELDS: &str = "\n    /** The discovery cache and its single-flight gates, owned by this\n     * instance; successful documents are cached per scheme for its lifetime\n     * and a failed fetch is never cached, so the next call retries. */\n    private val discoveryMutex = Mutex()\n    private val discoveryGates = HashMap<String, Mutex>()\n    private val discovered = HashMap<String, DiscoveredEndpoints>()\n";

/// The discovery-aware client-credentials member: the compiled token URL wins;
/// otherwise the scheme's cached discovery document resolves the request.
const CLIENT_CREDENTIALS_DISCOVERY: &str = r#"

    /** The scheme's cached token set, acquiring one from the resolved
     * client-credentials endpoint when the stored set is absent or expired
     * beyond the compiled skew. Concurrent callers on one owner share a single
     * acquisition: callers waiting on the gate re-read the store instead of
     * acquiring again.
     *
     * Endpoint resolution follows the compiled precedence: the compiled
     * client-credentials token URL always wins; otherwise, when the scheme
     * compiles a discovery URL, the discovery document's `token_endpoint`
     * resolves the request (fetched once per owner and cached,
     * single-flighted across concurrent callers, with a failed fetch retried
     * on the next call); otherwise the typed unsupported-flow refusal stands. */
    public suspend fun clientCredentialsToken(
        scheme: String,
        clientID: String? = null,
        clientSecret: String? = null,
        tokenStore: TokenStore? = null,
    ): TokenSet {
        val descriptor = OAuthSchemes.descriptor(scheme)
        val compiled = descriptor.clientCredentialsURL
        val endpoint = compiled.ifEmpty { discover(descriptor).tokenEndpoint ?: "" }
        if (endpoint.isEmpty()) {
            throw AuthException("unsupported-flow", scheme)
        }
        // Discovery-resolved endpoints authenticate as client-secret-basic
        // exactly when the compiled configuration supplies a client secret
        // variable; otherwise the public profile sends the client id in the
        // form.
        val effective = if (compiled.isNotEmpty()) {
            descriptor
        } else {
            discoveryDescriptor(descriptor, descriptor.clientSecretEnv.isNotEmpty())
        }
        val resolved = resolveCredentials(descriptor, clientID, clientSecret)
        val resolvedStore = tokenStore ?: defaultStore
        val key = storeKey(descriptor, resolved.first)
        fresh(resolvedStore, key, descriptor)?.let { return it }
        val gate = entryGate(key)
        gate.withLock {
            // A caller that waited on the gate re-checks the store before
            // acquiring: the holder may have populated it already.
            fresh(resolvedStore, key, descriptor)?.let { return it }
            val held = resolvedStore.load(key)
            val token = tokenRequest(
                effective,
                endpoint,
                linkedMapOf("grant_type" to "client_credentials"),
                held,
                resolved.first,
                resolved.second,
            )
            resolvedStore.replace(key, token)
            return token
        }
    }
"#;

/// The discovery-aware explicit refresh: the compiled refresh URL, else the
/// compiled token URL, always wins; otherwise the cached discovery document's
/// token endpoint resolves the exchange.
const REFRESH_DISCOVERY: &str = r#"

    /** Exchange the set's refresh token (RFC 6749 section 6) at the resolved
     * refresh endpoint. The returned set adopts a rotated refresh token and
     * retains the given one otherwise. The store is neither read nor updated:
     * callers decide which set to keep.
     *
     * Endpoint resolution follows the compiled precedence: the declared
     * refresh URL, else the compiled flow's token URL, always wins;
     * otherwise, when the scheme compiles a discovery URL, the discovery
     * document's `token_endpoint` resolves the exchange; otherwise the typed
     * unsupported-flow refusal stands. */
    public suspend fun refreshToken(
        scheme: String,
        token: TokenSet,
        clientID: String? = null,
        clientSecret: String? = null,
    ): TokenSet {
        val descriptor = OAuthSchemes.descriptor(scheme)
        val refresh = token.refreshToken ?: throw AuthException("no-refresh-token", scheme)
        val compiled = descriptor.refreshURL
        val endpoint = compiled.ifEmpty { discover(descriptor).tokenEndpoint ?: "" }
        if (endpoint.isEmpty()) throw AuthException("unsupported-flow", scheme)
        val effective = if (compiled.isNotEmpty()) {
            descriptor
        } else {
            discoveryDescriptor(descriptor, descriptor.clientSecretEnv.isNotEmpty())
        }
        val resolved = resolveCredentials(descriptor, clientID, clientSecret)
        return tokenRequest(
            effective,
            endpoint,
            linkedMapOf("grant_type" to "refresh_token", "refresh_token" to refresh),
            token,
            resolved.first,
            resolved.second,
        )
    }
"#;

/// The discovery-aware RFC 7009 revocation: the compiled endpoint always wins;
/// otherwise the discovery document's `revocation_endpoint`.
const REVOCATION_DISCOVERY: &str = r#"

    /** Posts the token value to the resolved revocation endpoint (RFC 7009).
     * Any 2xx response is success: RFC 7009 declares the token revoked even
     * when the server reports an unsupported-token error. A successful
     * revocation clears the resolved store's partition for this client
     * identity. The configured revocation endpoint always wins; otherwise,
     * when the scheme compiles a discovery URL, the discovery document's
     * `revocation_endpoint` resolves the request. */
    public suspend fun revokeToken(
        scheme: String,
        token: String,
        tokenTypeHint: String? = null,
        clientID: String? = null,
        clientSecret: String? = null,
        tokenStore: TokenStore? = null,
    ) {
        val descriptor = OAuthSchemes.descriptor(scheme)
        val compiled = descriptor.revocationURL
        val endpoint = compiled.ifEmpty { discover(descriptor).revocationEndpoint ?: "" }
        if (endpoint.isEmpty()) throw AuthException("unsupported-flow", scheme)
        if (token.isEmpty()) throw AuthException("invalid-token", scheme)
        val effective = if (compiled.isNotEmpty()) {
            descriptor
        } else {
            discoveryDescriptor(descriptor, descriptor.clientSecretEnv.isNotEmpty())
        }
        val resolved = resolveCredentials(descriptor, clientID, clientSecret)
        val fields = linkedMapOf("token" to token)
        tokenTypeHint?.takeIf { it.isNotEmpty() }?.let { fields["token_type_hint"] = it }
        val response = endpointRequest(effective, endpoint, fields, resolved.second, resolved.first)
        if (response.status in 200..299) {
            (tokenStore ?: defaultStore).clear(storeKey(descriptor, resolved.first))
            return
        }
        val declared = try {
            (oauthPayload(response, descriptor)["error"] as? JsonString)?.value?.takeIf { it.isNotEmpty() }
        } catch (error: AuthException) {
            null
        }
        if (declared != null) {
            throw AuthException("server-rejected", scheme, code = declared, status = response.status)
        }
        throw AuthException("server-error", scheme, status = response.status)
    }
"#;

/// The discovery-aware RFC 7662 introspection: the compiled endpoint always
/// wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECTION_DISCOVERY: &str = r#"

    /** Queries the resolved introspection endpoint (RFC 7662) with the token
     * value. The response describes the token without returning it; any
     * non-2xx answer is a typed failure carrying no token values. The
     * configured introspection endpoint always wins; otherwise, when the
     * scheme compiles a discovery URL, the discovery document's
     * `introspection_endpoint` resolves the request. */
    public suspend fun introspectToken(
        scheme: String,
        token: String,
        tokenTypeHint: String? = null,
        clientID: String? = null,
        clientSecret: String? = null,
    ): Introspection {
        val descriptor = OAuthSchemes.descriptor(scheme)
        val compiled = descriptor.introspectionURL
        val endpoint = compiled.ifEmpty { discover(descriptor).introspectionEndpoint ?: "" }
        if (endpoint.isEmpty()) throw AuthException("unsupported-flow", scheme)
        if (token.isEmpty()) throw AuthException("invalid-token", scheme)
        val effective = if (compiled.isNotEmpty()) {
            descriptor
        } else {
            discoveryDescriptor(descriptor, descriptor.clientSecretEnv.isNotEmpty())
        }
        val resolved = resolveCredentials(descriptor, clientID, clientSecret)
        val fields = linkedMapOf("token" to token)
        tokenTypeHint?.takeIf { it.isNotEmpty() }?.let { fields["token_type_hint"] = it }
        val response = endpointRequest(effective, endpoint, fields, resolved.second, resolved.first)
        if (response.status !in 200..299) {
            throw AuthException("server-error", scheme, status = response.status)
        }
        val members = oauthPayload(response, descriptor)
        val audience = when (val declared = members["aud"]) {
            is JsonString -> listOf(declared.value)
            is JsonArray -> declared.values.mapNotNull { value -> (value as? JsonString)?.value }
            else -> emptyList()
        }
        return Introspection(
            active = (members["active"] as? JsonBoolean)?.value == true,
            scope = (members["scope"] as? JsonString)?.value,
            clientID = (members["client_id"] as? JsonString)?.value,
            tokenType = (members["token_type"] as? JsonString)?.value,
            username = (members["username"] as? JsonString)?.value,
            expiresAt = (members["exp"] as? JsonNumber)?.token?.toLongOrNull(),
            issuedAt = (members["iat"] as? JsonNumber)?.token?.toLongOrNull(),
            notBefore = (members["nbf"] as? JsonNumber)?.token?.toLongOrNull(),
            subject = (members["sub"] as? JsonString)?.value,
            audience = audience,
            issuer = (members["iss"] as? JsonString)?.value,
            jwtID = (members["jti"] as? JsonString)?.value,
        )
    }
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one usable scheme carries a discovery URL: the typed decode with the
/// documented issuer-origin rule, the per-instance cache with single-flight,
/// the bounded GET and the discovery client-authentication rule.
const DISCOVERY_ENGINE: &str = r#"

    /** One fetched RFC 8414 / OpenID Connect discovery document, reduced to
     * the endpoints this lifecycle resolves. Unknown members are ignored. */
    private class DiscoveredEndpoints(
        val tokenEndpoint: String?,
        val revocationEndpoint: String?,
        val introspectionEndpoint: String?,
    )

    /** The scheme's compiled discovery URL, or empty when the compiled plan
     * carries none. */
    private fun discoveryURL(descriptor: OAuthSchemeDescriptor): String =
        DiscoveryDocuments.documents[descriptor.name] ?: ""

    /** The single-flight gate for one scheme's discovery fetch: the first
     * caller fetches, later callers wait and then re-read the cache. */
    private suspend fun discoveryGate(scheme: String): Mutex = discoveryMutex.withLock {
        discoveryGates.getOrPut(scheme) { Mutex() }
    }

    /** The scheme's cached discovery document, fetched once per owner instance
     * and single-flighted across concurrent callers. A failed fetch is never
     * cached, so the next call retries. A scheme whose compiled plan carries
     * no discovery URL is a typed refusal. */
    private suspend fun discover(descriptor: OAuthSchemeDescriptor): DiscoveredEndpoints {
        val url = discoveryURL(descriptor)
        if (url.isEmpty()) throw AuthException("unsupported-flow", descriptor.name)
        discoveryMutex.withLock { discovered[descriptor.name] }?.let { return it }
        val gate = discoveryGate(descriptor.name)
        gate.withLock {
            // A caller that waited on the gate re-checks the cache before
            // fetching: the holder may have populated it already.
            discoveryMutex.withLock { discovered[descriptor.name] }?.let { return it }
            val fetched = fetchDiscovery(descriptor, url)
            discoveryMutex.withLock { discovered[descriptor.name] = fetched }
            return fetched
        }
    }

    /** Fetches the scheme's discovery document (GET, `accept: application/json`)
     * over the owning client's transport; the response ceiling matches every
     * other bounded response. Failures are typed discovery failures that
     * never carry response body text. */
    private suspend fun fetchDiscovery(
        descriptor: OAuthSchemeDescriptor,
        url: String,
    ): DiscoveredEndpoints {
        val request = HttpRequest(
            method = "GET",
            url = URI(url),
            headers = linkedMapOf("Accept" to "application/json"),
            body = null,
            timeout = Duration.ofSeconds(30),
            maxResponseBytes = OAuthMaxResponseBytes,
        )
        val response = try {
            transport.execute(request)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            throw AuthException("discovery-failed", descriptor.name, cause = error)
        }
        if (response.status !in 200..299) {
            throw AuthException("discovery-failed", descriptor.name, status = response.status)
        }
        if (response.body.size > OAuthMaxResponseBytes) {
            throw AuthException("discovery-failed", descriptor.name, status = response.status)
        }
        return discoveryDocument(descriptor, url, response.body)
    }

    /** Decodes and validates one discovery response body into the endpoints
     * this lifecycle resolves. The exact issuer rule: when the document
     * carries an `issuer` claim, it must be an absolute http(s) URL whose
     * origin (scheme, host and the port with the scheme default made
     * explicit) equals the discovery URL's origin; OpenID Connect
     * openIdConnectUrl documents are validated against their `issuer` claim
     * exactly this way, as are RFC 8414 OAuth2 authorization-server metadata
     * documents. A missing claim is tolerated; a mismatching or unparseable
     * one is a typed discovery failure. Failure messages carry only safe
     * metadata, never response body text. */
    private fun discoveryDocument(
        descriptor: OAuthSchemeDescriptor,
        url: String,
        body: ByteArray,
    ): DiscoveredEndpoints {
        val payload = try {
            Json.parse(body)
        } catch (_: JsonException) {
            throw AuthException("discovery-failed", descriptor.name)
        }
        if (payload !is JsonObject) {
            throw AuthException("discovery-failed", descriptor.name)
        }
        val issuer = (payload.values["issuer"] as? JsonString)?.value
        if (!issuer.isNullOrEmpty()) {
            val issuerOrigin = discoveryOrigin(issuer)
            val documentOrigin = discoveryOrigin(url)
            if (issuerOrigin == null || documentOrigin == null || issuerOrigin != documentOrigin) {
                throw AuthException("discovery-failed", descriptor.name)
            }
        }
        return DiscoveredEndpoints(
            tokenEndpoint = discoveredEndpoint(descriptor, payload.values, "token_endpoint"),
            revocationEndpoint = discoveredEndpoint(descriptor, payload.values, "revocation_endpoint"),
            introspectionEndpoint = discoveredEndpoint(descriptor, payload.values, "introspection_endpoint"),
        )
    }

    /** One discovery document endpoint: absent stays null, and an unusable
     * value is a typed discovery failure. Unknown members are ignored. */
    private fun discoveredEndpoint(
        descriptor: OAuthSchemeDescriptor,
        values: Map<String, JsonValue>,
        member: String,
    ): String? = when (val value = values[member]) {
        null, is JsonNull -> null
        is JsonString -> value.value.ifEmpty {
            throw AuthException("discovery-failed", descriptor.name)
        }
        else -> throw AuthException("discovery-failed", descriptor.name)
    }

    /** The origin of one absolute http(s) URL: scheme, host and the port with
     * the scheme default made explicit; null when the value is not an
     * absolute http(s) URL. */
    private fun discoveryOrigin(url: String): String? {
        val parsed = try {
            URI(url)
        } catch (_: Exception) {
            return null
        }
        val scheme = parsed.scheme?.lowercase() ?: return null
        if (scheme != "http" && scheme != "https") return null
        val host = parsed.host ?: return null
        val port = parsed.port
        val explicit = if (port != -1) port else if (scheme == "http") 80 else 443
        return "$scheme://$host:$explicit"
    }

    /** The descriptor for discovery-resolved endpoints: client-secret-basic
     * exactly when the compiled configuration supplies a client secret
     * variable (an unavailable value becomes the typed missing-credentials
     * refusal at request time); otherwise the public profile. */
    private fun discoveryDescriptor(
        descriptor: OAuthSchemeDescriptor,
        basic: Boolean,
    ): OAuthSchemeDescriptor = if (descriptor.clientSecretBasic == basic) {
        descriptor
    } else {
        OAuthSchemeDescriptor(
            descriptor.name,
            basic,
            descriptor.clientIDEnv,
            descriptor.clientSecretEnv,
            descriptor.skewSeconds,
            descriptor.issuer,
            descriptor.refreshURL,
            descriptor.clientCredentialsURL,
            descriptor.authorizationURL,
            descriptor.codeTokenURL,
            descriptor.deviceURL,
            descriptor.deviceTokenURL,
            descriptor.revocationURL,
            descriptor.introspectionURL,
        )
    }
"#;

const SESSIONS_TAIL: &str = "\n    /** Explicit arguments win; the compiled variable names are read now.\n     * Empty values mean absent; callers may legitimately configure only one. */\n    private fun resolveCredentials(\n        descriptor: OAuthSchemeDescriptor,\n        clientID: String?,\n        clientSecret: String?,\n    ): Pair<String?, String?> {\n        val id = clientID?.takeIf { it.isNotEmpty() } ?: descriptor.clientIDEnv\n            .takeIf { it.isNotEmpty() }\n            ?.let { System.getenv(it)?.takeIf { value -> value.isNotEmpty() } }\n        val secret = clientSecret?.takeIf { it.isNotEmpty() } ?: descriptor.clientSecretEnv\n            .takeIf { it.isNotEmpty() }\n            ?.let { System.getenv(it)?.takeIf { value -> value.isNotEmpty() } }\n        return id to secret\n    }\n\n    /** Store keys partition by scheme, token-endpoint issuer and client\n     * identity, so distinct clients and endpoints never share a set. */\n    private fun storeKey(descriptor: OAuthSchemeDescriptor, clientID: String?): String =\n        descriptor.name + \"\\u001f\" + descriptor.issuer + \"\\u001f\" + (clientID ?: \"\")\n\n    /** The stored set when it still outlives its compiled clock skew. */\n    private fun fresh(store: TokenStore, key: String, descriptor: OAuthSchemeDescriptor): TokenSet? {\n        val stored = store.load(key) ?: return null\n        return if (stored.expired(descriptor.skewSeconds)) null else stored\n    }\n\n    /** Posts one form-encoded request to a compiled endpoint, applying the\n     * scheme's compiled client authentication (HTTP Basic for confidential\n     * clients, the client_id form member for public ones). */\n    private suspend fun endpointRequest(\n        descriptor: OAuthSchemeDescriptor,\n        endpoint: String,\n        fields: Map<String, String>,\n        clientSecret: String?,\n        clientID: String?,\n    ): HttpResponse {\n        if (endpoint.isEmpty()) throw AuthException(\"unsupported-flow\", descriptor.name)\n        val headers = linkedMapOf(\n            \"Accept\" to \"application/json\",\n            \"Content-Type\" to \"application/x-www-form-urlencoded\",\n        )\n        val body: String\n        if (descriptor.clientSecretBasic) {\n            if (clientID.isNullOrEmpty() || clientSecret.isNullOrEmpty()) {\n                throw AuthException(\"missing-client-credentials\", descriptor.name)\n            }\n            headers[\"Authorization\"] = \"Basic \" + Base64.getEncoder().encodeToString(\n                (oauthFormValue(clientID) + \":\" + oauthFormValue(clientSecret))\n                    .toByteArray(StandardCharsets.UTF_8),\n            )\n            body = oauthFormEncode(fields)\n        } else {\n            body = oauthFormEncode(if (clientID.isNullOrEmpty()) fields else fields + (\"client_id\" to clientID))\n        }\n        val request = HttpRequest(\n            method = \"POST\",\n            url = URI(endpoint),\n            headers = headers,\n            body = body.toByteArray(StandardCharsets.UTF_8),\n            timeout = Duration.ofSeconds(30),\n            maxResponseBytes = OAuthMaxResponseBytes,\n        )\n        val response = try {\n            transport.execute(request)\n        } catch (cancelled: CancellationException) {\n            throw cancelled\n        } catch (error: Exception) {\n            throw AuthException(\"transport\", descriptor.name, cause = error)\n        }\n        if (response.body.size > OAuthMaxResponseBytes) {\n            throw AuthException(\"resource-limit\", descriptor.name, status = response.status)\n        }\n        return response\n    }\n\n    /** Decodes one bounded JSON object response and refuses a server-declared\n     * `error` member before any status classification. */\n    private fun oauthPayload(response: HttpResponse, descriptor: OAuthSchemeDescriptor): Map<String, JsonValue> {\n        val payload = try {\n            Json.parse(response.body)\n        } catch (error: JsonException) {\n            throw AuthException(\"invalid-response\", descriptor.name, status = response.status)\n        }\n        if (payload !is JsonObject) {\n            throw AuthException(\"invalid-response\", descriptor.name, status = response.status)\n        }\n        val declared = (payload.values[\"error\"] as? JsonString)?.value\n        if (!declared.isNullOrEmpty()) {\n            throw AuthException(\"server-rejected\", descriptor.name, code = declared, status = response.status)\n        }\n        return payload.values\n    }\n\n    /** Executes one RFC 6749 token-endpoint request and decodes the response.\n     * A server-declared error member wins; a 2xx answer without an access\n     * token is an invalid response. `previous` retains its refresh token when\n     * the server returns no rotated one. */\n    private suspend fun tokenRequest(\n        descriptor: OAuthSchemeDescriptor,\n        endpoint: String,\n        fields: Map<String, String>,\n        previous: TokenSet?,\n        clientID: String?,\n        clientSecret: String?,\n    ): TokenSet {\n        val response = endpointRequest(descriptor, endpoint, fields, clientSecret, clientID)\n        val members = oauthPayload(response, descriptor)\n        if (response.status !in 200..299) {\n            throw AuthException(\"server-error\", descriptor.name, status = response.status)\n        }\n        val access = (members[\"access_token\"] as? JsonString)?.value\n        if (access.isNullOrEmpty()) {\n            throw AuthException(\"invalid-response\", descriptor.name, status = response.status)\n        }\n        val type = (members[\"token_type\"] as? JsonString)?.value\n            ?.takeIf { value -> value.isNotEmpty() }\n            ?.let { value -> if (value.equals(\"bearer\", ignoreCase = true)) \"Bearer\" else value }\n            ?: \"Bearer\"\n        val expiresAt = (members[\"expires_in\"] as? JsonNumber)?.token?.toLongOrNull()\n            ?.takeIf { it > 0 }\n            ?.let { System.currentTimeMillis() + it * 1000 }\n            ?: 0L\n        val refresh = (members[\"refresh_token\"] as? JsonString)?.value?.takeIf { it.isNotEmpty() }\n            ?: previous?.refreshToken\n        return TokenSet(\n            accessToken = access,\n            tokenType = type,\n            expiresAt = expiresAt,\n            refreshToken = refresh,\n            scope = (members[\"scope\"] as? JsonString)?.value,\n        )\n    }\n\n}\n\n";

const CLIENT_EXTENSIONS_HEAD: &str = "/** Extension surface over the owning client's generated [OAuthSessions];\n * every flow resolves client identity from the explicit arguments or the\n * compiled environment variable names, read at call time. */\n";

const CLIENT_CREDENTIALS_EXTENSION: &str = "\n/** The scheme's cached token set, acquiring one from the compiled\n * client-credentials endpoint when the stored set is absent or expired beyond\n * the compiled skew. Single-flighted per client, scheme, issuer and client\n * identity. */\npublic suspend fun Client.clientCredentialsToken(\n    scheme: String,\n    clientID: String? = null,\n    clientSecret: String? = null,\n    tokenStore: TokenStore? = null,\n): TokenSet = oauth.clientCredentialsToken(scheme, clientID, clientSecret, tokenStore)\n";

const REFRESH_EXTENSION: &str = "\n/** Exchange the set's refresh token at the scheme's declared refresh\n * endpoint, or its token endpoint when none is declared; a rotated refresh\n * token is adopted, the given one retained otherwise. */\npublic suspend fun Client.refreshToken(\n    scheme: String,\n    token: TokenSet,\n    clientID: String? = null,\n    clientSecret: String? = null,\n): TokenSet = oauth.refreshToken(scheme, token, clientID, clientSecret)\n";

const DEVICE_EXTENSIONS: &str = "\n/** Starts a device-authorization transaction against the compiled device\n * endpoint (RFC 8628). */\npublic suspend fun Client.beginDeviceAuthorization(\n    scheme: String,\n    clientID: String? = null,\n    clientSecret: String? = null,\n): DeviceAuthorization = oauth.beginDeviceAuthorization(scheme, clientID, clientSecret)\n\n/** Polls the compiled token endpoint for the device grant; `wait` replaces\n * the sleeper (tests pass a no-op). */\npublic suspend fun Client.pollDeviceToken(\n    device: DeviceAuthorization,\n    clientID: String? = null,\n    clientSecret: String? = null,\n    tokenStore: TokenStore? = null,\n    wait: suspend (Long) -> Unit = { delay(it) },\n): TokenSet = oauth.pollDeviceToken(device, clientID, clientSecret, tokenStore, wait)\n";

const REVOCATION_EXTENSION: &str = "\n/** Posts the token value to the scheme's compiled revocation endpoint\n * (RFC 7009) and clears the owning store's partition on success. */\npublic suspend fun Client.revokeToken(\n    scheme: String,\n    token: String,\n    tokenTypeHint: String? = null,\n    clientID: String? = null,\n    clientSecret: String? = null,\n    tokenStore: TokenStore? = null,\n): Unit = oauth.revokeToken(scheme, token, tokenTypeHint, clientID, clientSecret, tokenStore)\n";

const INTROSPECTION_EXTENSION: &str = "\n/** Queries the scheme's compiled introspection endpoint (RFC 7662) with the\n * token value. */\npublic suspend fun Client.introspectToken(\n    scheme: String,\n    token: String,\n    tokenTypeHint: String? = null,\n    clientID: String? = null,\n    clientSecret: String? = null,\n): Introspection = oauth.introspectToken(scheme, token, tokenTypeHint, clientID, clientSecret)\n";

const AUTHORIZATION_EXTENSIONS: &str = "\n/** Starts an authorization-code transaction with PKCE S256 bound to this\n * client; no network call. */\npublic fun Client.beginAuthorization(\n    scheme: String,\n    redirectURI: String,\n    scopes: List<String> = emptyList(),\n    state: String? = null,\n    clientID: String? = null,\n): AuthorizationTransaction = oauth.beginAuthorization(scheme, redirectURI, scopes, state, clientID)\n\n/** Validates the callback's state, exchanges the code with the retained PKCE\n * verifier and replaces the owning store's entry atomically. Single-use per\n * transaction. */\npublic suspend fun Client.completeAuthorization(\n    transaction: AuthorizationTransaction,\n    callbackParams: Map<String, String>,\n    clientID: String? = null,\n    clientSecret: String? = null,\n    tokenStore: TokenStore? = null,\n): TokenSet = oauth.completeAuthorization(transaction, callbackParams, clientID, clientSecret, tokenStore)\n";

/// The replaying wrapper's head paragraph, emitted only when a compiled scheme
/// carries an executable client-credentials flow, the provider the wrapper
/// wraps. Plans without one keep the plain head bytes.
const HEAD_REPLAY: &str = "/** The opt-in replaying credential wrapper [OAuthReplayCredentials] extends\n * exactly the compiled client-credentials providers with the unified 401\n * request policy: one coordinated refresh plus one eligible request replay\n * per qualifying 401. Attaches served on stream-protected operations are\n * never replayed, and lifecycle endpoints are never replayed. The plain\n * [OAuthSessions] lifecycle keeps today's attach-only semantics.\n */\n\n";

/// The store-key derivation the opt-in replaying wrapper shares with the
/// session's partitioning, emitted only when the wrapper participates so plans
/// without an executable client-credentials flow keep the pre-replay bytes.
const REPLAY_STORE_KEY: &str = "\n    /** The store key the opt-in replaying credential wrapper shares with this\n     * session's partitioning: it derives the same key from the same compiled\n     * inputs, so a replay refresh coordinates over the exact partition the\n     * plain lifecycle serves. */\n    internal fun replayStoreKey(\n        descriptor: OAuthSchemeDescriptor,\n        clientID: String?,\n        clientSecret: String?,\n    ): String = storeKey(descriptor, resolveCredentials(descriptor, clientID, clientSecret).first)\n";

/// The stream-protected operations whose attaches must never be replayed, per
/// lowered scheme: security requirements that name a lowered scheme on an
/// operation whose responses carry a stream representation. Delivered stream
/// data prevents a transparent restart, so those operations are excluded from
/// the replay wrapper's one-replay budget. The attach context carries the
/// operation ID (the requirement's source pointer is not exposed to credential
/// hooks), so the compiled set names the stream-protected operations per
/// scheme.
fn no_replay_requirements(
    lowered: &[Lowered],
    operations: &[super::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = lowered.iter().map(|s| s.name.as_str()).collect();
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
                        .insert(operation.operation_id.clone());
                }
            }
        }
    }
    pointers
}

/// The replaying credential wrapper: one coordinated refresh plus one eligible
/// request replay per qualifying 401, opt-in per provider, and never for
/// stream-protected operations. The section emits only when a lowered scheme
/// carries an executable client-credentials flow, the provider the wrapper
/// wraps; plans without one assemble byte-identically to the pre-replay bytes.
/// The plain and discovery variants resolve the lifecycle-endpoint exclusion
/// through the same compiled precedence as the lifecycle they wrap.
fn replay_section(
    lowered: &[Lowered],
    operations: &[super::PlannedOperation],
    discovery: bool,
) -> String {
    let no_replay = no_replay_requirements(lowered, operations);
    let mut out = String::from(
        "/** Compiled stream-protected operations per source scheme: operations whose\n * responses carry a stream representation and whose security requirements\n * name the scheme. Attaches served on these operations are never replayed,\n * because delivered stream data prevents a transparent restart; the attach\n * context carries the operation ID, so the compiled set names operations. */\ninternal val oauthNoReplayOperations: Map<String, Set<String>> = mapOf(\n",
    );
    for scheme in lowered {
        let Some(operations) = no_replay.get(&scheme.name) else {
            continue;
        };
        if operations.is_empty() {
            continue;
        }
        let rendered = operations
            .iter()
            .map(|operation| super::emit::quote(operation))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "        {} to setOf({}),\n",
            super::emit::quote(&scheme.name),
            rendered,
        ));
    }
    out.push_str(")\n\n");
    out.push_str(REPLAY_VALUE_HELPER);
    out.push_str(REPLAY_WRAPPER_HEAD);
    out.push_str(if discovery {
        REPLAY_GUARD_DISCOVERY
    } else {
        REPLAY_GUARD_PLAIN
    });
    out.push_str(REPLAY_WRAPPER_CORE);
    out.push_str(if discovery {
        REPLAY_LIFECYCLE_DISCOVERY
    } else {
        REPLAY_LIFECYCLE_PLAIN
    });
    out
}

/** The complete Authorization header value of a stored set. */
const REPLAY_VALUE_HELPER: &str = "/** The complete Authorization header value of one stored set. */\ninternal fun oauthReplayValue(token: TokenSet): String =\n    token.tokenType.ifEmpty { \"Bearer\" } + \" \" + token.accessToken\n\n";

/// The replaying credential wrapper's head: the documented class and its
/// fields. Shared by the plain and discovery variants.
const REPLAY_WRAPPER_HEAD: &str = "/** A replaying client-credentials credential: the plain [OAuthSessions]\n * attach behavior plus the unified 401 request policy. Use it in two places —\n * pass [hook] as the scheme's credentials member and [asTransport] as the\n * client's transport:\n *\n * ```kotlin\n * val replay = OAuthReplayCredentials(\"serviceOAuth\", myTransport, clientID = \"id\", clientSecret = \"secret\")\n * val client = Client(Credentials(serviceOAuth = replay.hook), replay.asTransport(myTransport))\n * ```\n *\n * A 401 (and only a 401) on a request whose Authorization value this provider\n * attached triggers exactly one coordinated refresh — concurrent 401s share\n * one token request round — and exactly one replay of the request with the\n * fresh token, preserving method, URL and body while regenerating headers\n * through the normal attach path. The second response is surfaced whatever it\n * is: a second 401 reaches the caller as the declared error. The overall\n * budget is one refresh plus one replay, never nested with other retry\n * policies (requests are not retried today). Attaches for stream-protected\n * requirements are never replayed, because delivered stream data prevents a\n * transparent restart. A refresh failure surfaces as the typed\n * [AuthException] instead of a replay. The plain [OAuthSessions] lifecycle\n * keeps today's attach-only semantics: replay is this wrapper's opt-in only.\n */\npublic class OAuthReplayCredentials(\n    /** The compiled source scheme this credential serves. */\n    public val scheme: String,\n    /** The transport token requests travel through directly; pass the same\n     * transport to [asTransport]. */\n    private val tokenTransport: Transport,\n    /** Explicit client identity; the compiled variables resolve otherwise. */\n    private val clientID: String? = null,\n    private val clientSecret: String? = null,\n    /** The instance-owned store; the wrapped session shares it. */\n    private val store: TokenStore = MemoryTokenStore(),\n) {\n    private val descriptor: OAuthSchemeDescriptor = OAuthSchemes.descriptor(scheme)\n";

/// The plain variant's creation guard: the compiled client-credentials
/// endpoint must exist, exactly like the plain lifecycle's first attach.
const REPLAY_GUARD_PLAIN: &str = "\n    init {\n        if (descriptor.clientCredentialsURL.isEmpty()) {\n            throw AuthException(\"unsupported-flow\", scheme)\n        }\n    }\n";

/// The discovery variant's creation guard: a compiled client-credentials
/// endpoint or a discovery URL makes the scheme replayable, exactly like the
/// compiled acquisition precedence.
const REPLAY_GUARD_DISCOVERY: &str = "\n    init {\n        if (descriptor.clientCredentialsURL.isEmpty() &&\n            (DiscoveryDocuments.documents[scheme] ?: \"\").isEmpty()\n        ) {\n            throw AuthException(\"unsupported-flow\", scheme)\n        }\n    }\n";

/// The replaying credential wrapper's core: the wrapped session, the
/// credential hook with its served-attach record, the eligibility decision,
/// the served-attach lookup, the coordinated refresh round and the replaying
/// transport. Shared by the plain and discovery variants.
const REPLAY_WRAPPER_CORE: &str = "\n    private val sessions = OAuthSessions(tokenTransport, store)\n\n    /** One attach this provider served, remembered so the wrapped transport\n     * can tell which requests carried this provider's token. The record keeps\n     * only safe metadata; token values already traveled on the wire. */\n    private class ServedAttach(val value: String, val eligible: Boolean)\n\n    private val served = ArrayDeque<ServedAttach>()\n    private val roundsMutex = Mutex()\n    private val rounds = HashMap<String, CompletableDeferred<String>>()\n\n    /** The credential hook: serves this scheme's tokens through the wrapped\n     * session's plain lifecycle and remembers which Authorization values its\n     * attaches produced. Pass it as the scheme's credentials member. */\n    public val hook: CredentialProvider = CredentialProvider { context ->\n        val token = sessions.clientCredentialsToken(scheme, clientID, clientSecret, store)\n        record(oauthReplayValue(token), eligible(context))\n        oauthReplayValue(token)\n    }\n\n    /** Wraps `inner` with the one-refresh-one-replay 401 policy; call once per\n     * client. Token requests keep traveling through `inner` directly. */\n    public fun asTransport(inner: Transport): Transport = ReplayTransport(inner)\n\n    /** Records one served attach, newest first, bounded to eight entries. */\n    private fun record(value: String, eligible: Boolean): Unit = synchronized(served) {\n        served.addFirst(ServedAttach(value, eligible))\n        while (served.size > 8) {\n            served.removeLast()\n        }\n    }\n\n    /** Whether a qualifying 401 on this attach may be replayed: attaches for\n     * stream-protected requirements never are, because delivered stream data\n     * prevents a transparent restart. */\n    private fun eligible(context: CredentialContext): Boolean =\n        oauthNoReplayOperations[scheme]?.contains(context.operationId) != true\n\n    /** Whether the presented Authorization value carries this provider's\n     * token from an eligible attach. */\n    private fun replayable(presented: String): Boolean = synchronized(served) {\n        served.any { attach -> attach.value == presented && attach.eligible }\n    }\n\n    /** One coordinated refresh: a newer stored set wins over a stale\n     * re-refresh, concurrent 401s share one round, and a failed round fails\n     * every waiter exactly once. The round resolves to the fresh complete\n     * Authorization value. */\n    private suspend fun refresh(presented: String): String {\n        val key = sessions.replayStoreKey(descriptor, clientID, clientSecret)\n        val round: CompletableDeferred<String>\n        val leader: Boolean\n        roundsMutex.withLock {\n            store.load(key)?.let { stored ->\n                val value = oauthReplayValue(stored)\n                if (value != presented) return value\n            }\n            val existing = rounds[key]\n            if (existing == null) {\n                val created = CompletableDeferred<String>()\n                rounds[key] = created\n                round = created\n                leader = true\n            } else {\n                round = existing\n                leader = false\n            }\n        }\n        if (!leader) {\n            // The round's outcome surfaces once, whatever it is.\n            return round.await()\n        }\n        try {\n            store.clear(key)\n            val token = sessions.clientCredentialsToken(scheme, clientID, clientSecret, store)\n            val fresh = oauthReplayValue(token)\n            round.complete(fresh)\n            return fresh\n        } catch (cancelled: CancellationException) {\n            round.completeExceptionally(cancelled)\n            throw cancelled\n        } catch (failure: Exception) {\n            round.completeExceptionally(failure)\n            throw failure\n        } finally {\n            roundsMutex.withLock { rounds.remove(key) }\n        }\n    }\n\n    /** The replaying transport: the 401 interception half of the wrapper. One\n     * coordinated refresh and, when the request carried the provider's token\n     * and no stream is protected on it, exactly one replay with the fresh\n     * token. Lifecycle endpoint requests are never replayed: they carry no\n     * bearer token of this provider, and the exact-target guard is defense in\n     * depth. */\n    private inner class ReplayTransport(\n        /** The caller transport the wrapped client's requests ride and the\n         * replay rides; token requests keep traveling through it directly. */\n        private val inner: Transport,\n    ) : Transport {\n        override suspend fun execute(request: HttpRequest): HttpResponse {\n            val response = inner.execute(request)\n            if (response.status != 401) return response\n            val presented = request.headers.entries\n                .firstOrNull { it.key.equals(\"Authorization\", ignoreCase = true) }?.value\n            if (presented.isNullOrEmpty() || !replayable(presented)) return response\n            // Lifecycle endpoint requests carry no bearer token of this\n            // provider, so this exact-target guard is defense in depth.\n            if (lifecycle(request.url.toString())) return response\n            val fresh = refresh(presented)\n            val headers = LinkedHashMap<String, String>(request.headers.size)\n            request.headers.forEach { (name, value) ->\n                headers[name] = if (name.equals(\"Authorization\", ignoreCase = true)) fresh else value\n            }\n            return inner.execute(\n                HttpRequest(\n                    method = request.method,\n                    url = request.url,\n                    headers = headers,\n                    body = request.body,\n                    timeout = request.timeout,\n                    maxResponseBytes = request.maxResponseBytes,\n                ),\n            )\n        }\n    }\n";

/// The plain variant's lifecycle-endpoint exclusion: the compiled token,
/// refresh, code-token, device, revocation and introspection endpoints.
const REPLAY_LIFECYCLE_PLAIN: &str = "\n    /** The exact-target lifecycle guard: lifecycle endpoint requests are\n     * never replayed. They carry no bearer token of this provider, so this\n     * guard is defense in depth against loops. */\n    private fun lifecycle(url: String): Boolean =\n        (descriptor.clientCredentialsURL.isNotEmpty() && url == descriptor.clientCredentialsURL) ||\n            (descriptor.refreshURL.isNotEmpty() && url == descriptor.refreshURL) ||\n            (descriptor.codeTokenURL.isNotEmpty() && url == descriptor.codeTokenURL) ||\n            (descriptor.deviceURL.isNotEmpty() && url == descriptor.deviceURL) ||\n            (descriptor.deviceTokenURL.isNotEmpty() && url == descriptor.deviceTokenURL) ||\n            (descriptor.revocationURL.isNotEmpty() && url == descriptor.revocationURL) ||\n            (descriptor.introspectionURL.isNotEmpty() && url == descriptor.introspectionURL)\n}\n";

/// The discovery variant's lifecycle-endpoint exclusion: the compiled
/// discovery URL joins the compiled endpoints; the discovery-resolved token
/// endpoint rides the exact-token match.
const REPLAY_LIFECYCLE_DISCOVERY: &str = "\n    /** The exact-target lifecycle guard: lifecycle endpoint requests are\n     * never replayed. They carry no bearer token of this provider, so this\n     * guard is defense in depth against loops; the discovery-resolved token\n     * endpoint rides the exact-token match. */\n    private fun lifecycle(url: String): Boolean =\n        (descriptor.clientCredentialsURL.isNotEmpty() && url == descriptor.clientCredentialsURL) ||\n            (descriptor.refreshURL.isNotEmpty() && url == descriptor.refreshURL) ||\n            (descriptor.codeTokenURL.isNotEmpty() && url == descriptor.codeTokenURL) ||\n            (descriptor.deviceURL.isNotEmpty() && url == descriptor.deviceURL) ||\n            (descriptor.deviceTokenURL.isNotEmpty() && url == descriptor.deviceTokenURL) ||\n            (descriptor.revocationURL.isNotEmpty() && url == descriptor.revocationURL) ||\n            (descriptor.introspectionURL.isNotEmpty() && url == descriptor.introspectionURL) ||\n            ((DiscoveryDocuments.documents[scheme] ?: \"\").isNotEmpty() &&\n                url == DiscoveryDocuments.documents[scheme])\n}\n";
