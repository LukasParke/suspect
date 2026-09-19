//! Emitted-only first-party OAuth 2.0 / OpenID Connect lifecycle for the
//! Swift package.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when client defaults are configured) compiles into one generated
//! `OAuth.swift` file, registered beside `Pagination.swift`; the static runtime
//! files and the shared planner stay untouched, and plans without configured
//! defaults — or without at least one usable scheme — emit no new bytes at
//! all, so no-policy output stays byte-identical.
//!
//! What counts as usable mirrors every backend: a non-deprecated flow whose
//! declared endpoints satisfy its grant — `client-credentials` needs a token
//! URL, `authorization-code` needs authorization and token URLs, and
//! `device-authorization` needs device-authorization and token URLs. Implicit
//! and password flows are represented in the compiled descriptors but are
//! never executed, so schemes with only those flows emit nothing.
//!
//! The generated file embeds the compiled descriptors as constants — the
//! runtime never parses OpenAPI and never invents an endpoint — and owns:
//! `TokenSet`, the `TokenStore` protocol with an instance-owned
//! `MemoryTokenStore`, skew-aware client-credentials acquisition with
//! per-key single-flight inside `OAuthSession`, explicit refresh, and — only
//! when the compiled scheme carries them — revocation, introspection,
//! authorization-code with PKCE S256, and RFC 8628 device polling. A compiled
//! discovery URL extends the lifecycle with RFC 8414 / OpenID Connect
//! discovery: the document is fetched once per session through the client's
//! own transport, cached on the `OAuthSession` actor with single-flight and
//! failed-fetch retry, issuer-validated, and resolves only the endpoint URLs
//! the compiled plan omits — compiled endpoints always win. Discovery-aware
//! emission is byte-identical without a discovery URL: the plain sections
//! concatenate into exactly the pre-discovery bytes. The generated `Client`
//! is an immutable value type without identity, so per-client token state is
//! owned by the caller-constructed `OAuthSession` (one session per client);
//! stateless one-shot steps stay on `Client` extensions. Client identity
//! resolves from explicit arguments first and otherwise from the compiled
//! environment variable names, read at call time. Token endpoint requests use
//! the client's own transport, so caller transport policy covers the
//! lifecycle too. Error values never carry token or client-secret material.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::SdkPlan;
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use suspect_ir::contract::Contract;

/// Fixed public API of the generated `OAuth.swift`: reserved against model
/// symbols while the file is emitted, so an inconsistent namespace is
/// rejected at planning instead of emitting a package that cannot compile.
pub(super) const TYPE_NAMES: &[&str] = &[
    "AuthError",
    "AuthorizationTransaction",
    "DeviceAuthorization",
    "DiscoveredEndpoints",
    "Introspection",
    "MemoryTokenStore",
    "OAuthCatalog",
    "OAuthSchemeDescriptor",
    "OAuthSession",
    "TokenSet",
    "TokenStore",
];

/// Replay-surface type names. Reserved against model symbols only when a
/// compiled scheme carries an executable client-credentials flow — the
/// wrapper joins the emitted surface exactly then, so plans without one keep
/// today's namespace and bytes.
pub(super) const REPLAY_TYPE_NAMES: &[&str] = &["OAuthReplayCredentials"];

/// Fixed `Client` extension members the emitted file adds. Reserved against
/// operation method allocation only while the file is emitted, so source
/// operations that would collide allocate suffixed names instead.
pub(super) const CLIENT_METHODS: &[&str] = &[
    "authorizationProvider",
    "beginAuthorization",
    "beginDeviceAuthorization",
    "clientCredentialsToken",
    "completeAuthorization",
    "introspectToken",
    "refreshToken",
    "revokeToken",
];

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

/// Whether one compiled scheme carries an executable client-credentials flow,
/// so the replaying credential wrapper participates. The wrapper serves
/// exactly that provider, so schemes without one compile exactly the
/// pre-replay bytes.
fn has_client_credentials(scheme: &wire::OAuthSchemePlan) -> bool {
    scheme.flows.iter().any(|flow| {
        flow.kind == wire::OAuthFlowDescriptorKind::ClientCredentials && executable(flow)
    })
}

/// Whether the compiled plan admits at least one usable scheme, so the
/// generated package carries `OAuth.swift`. A discovery URL makes a scheme
/// usable even with no executable declared flow: the discovery document
/// defines its endpoints at runtime.
pub(super) fn emits(oauth: &wire::OAuthPlan) -> bool {
    oauth.mode != wire::OAuthMode::Off
        && oauth
            .schemes
            .iter()
            .any(|scheme| !executable_flows(scheme).is_empty() || scheme.discovery.is_some())
}

/// Compiled OAuth planning carried by the plan: only a configured policy is
/// lowered, mirroring pagination, so no-policy output stays byte-identical for
/// every document. Planning errors are the shared planner's
/// [`crate::http_protocol::HttpDiagnostic`]s; fixed-name collisions with model
/// symbols or client members are source-linked rejections, mirroring the Go
/// backend's namespace refusal.
pub(super) fn plan(
    contract: &Contract,
    wire_plan: &wire::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
    type_names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Result<Option<wire::OAuthPlan>, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    if defaults.oauth.mode == wire::OAuthMode::Off {
        return Ok(None);
    }
    let plan = wire::plan_oauth(contract, wire_plan, Some(defaults))?;
    if !emits(&plan) {
        return Ok(None);
    }
    let replay = plan.schemes.iter().any(has_client_credentials);
    let fallback =
        suspect_ir::contract::SourceId::new(contract.entry().clone(), Default::default());
    let mut errors = Vec::new();
    for name in TYPE_NAMES
        .iter()
        .chain(replay.then_some(REPLAY_TYPE_NAMES).into_iter().flatten())
    {
        if type_names.contains(*name) {
            errors.push(super::diagnostic(
                contract,
                fallback.clone(),
                "swift-oauth-name-collision",
                format!(
                    "the OAuth lifecycle API reserves {name:?}, which is already allocated by the generated model plan; the OAuth runtime must not shadow a planned declaration"
                ),
            ));
            continue;
        }
        type_names.insert((*name).to_owned());
    }
    for name in CLIENT_METHODS {
        if methods.contains(*name) {
            errors.push(super::diagnostic(
                contract,
                fallback.clone(),
                "swift-oauth-name-collision",
                format!(
                    "the OAuth lifecycle API reserves the client member {name:?}, which is already allocated by the generated client plan; the OAuth runtime must not shadow a planned declaration"
                ),
            ));
            continue;
        }
        methods.insert((*name).to_owned());
    }
    if errors.is_empty() {
        Ok(Some(plan))
    } else {
        Err(errors)
    }
}

/// One usable scheme lowered into its compiled descriptor fields. Empty
/// strings mean "not compiled": endpoints are never invented. A scheme with
/// no executable flow lowers only when it carries a discovery URL, which
/// defines its endpoints at runtime.
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

fn lower(scheme: &wire::OAuthSchemePlan) -> Option<Scheme> {
    let flows = executable_flows(scheme);
    if flows.is_empty() && scheme.discovery.is_none() {
        return None;
    }
    let first = flows.first();
    let mut lowered = Scheme {
        name: scheme.name.clone(),
        client_auth: match first {
            Some(flow) => match flow.client_auth {
                wire::OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
                wire::OAuthClientAuth::None => "none",
            },
            // A discovery-defined scheme carries no compiled flow: the client
            // is confidential exactly when configuration declares a secret
            // variable, mirroring the discovery-aware providers of every
            // other backend.
            None => {
                if scheme.client_secret_env.is_some() {
                    "client-secret-basic"
                } else {
                    "none"
                }
            }
        },
        client_id_env: scheme.client_id_env.clone().unwrap_or_default(),
        client_secret_env: scheme.client_secret_env.clone().unwrap_or_default(),
        skew: scheme.refresh_skew_seconds,
        token_url: first
            .and_then(|flow| flow.token_url.clone())
            .unwrap_or_default(),
        refresh_url: first
            .and_then(|flow| flow.refresh_url.clone().or_else(|| flow.token_url.clone()))
            .unwrap_or_default(),
        client_credentials: String::new(),
        authorization_url: String::new(),
        code_token_url: String::new(),
        device_url: String::new(),
        device_token_url: String::new(),
        revocation_url: scheme.revocation_endpoint.clone().unwrap_or_default(),
        introspection_url: scheme.introspection_endpoint.clone().unwrap_or_default(),
        discovery_url: scheme.discovery.clone().unwrap_or_default(),
    };
    for flow in flows {
        match flow.kind {
            wire::OAuthFlowDescriptorKind::ClientCredentials
                if lowered.client_credentials.is_empty() =>
            {
                lowered.client_credentials = flow.token_url.clone().unwrap_or_default();
            }
            wire::OAuthFlowDescriptorKind::AuthorizationCode
                if lowered.authorization_url.is_empty() =>
            {
                lowered.authorization_url = flow.authorization_url.clone().unwrap_or_default();
                lowered.code_token_url = flow.token_url.clone().unwrap_or_default();
            }
            wire::OAuthFlowDescriptorKind::DeviceAuthorization if lowered.device_url.is_empty() => {
                lowered.device_url = flow.device_authorization_url.clone().unwrap_or_default();
                lowered.device_token_url = flow.token_url.clone().unwrap_or_default();
            }
            _ => {}
        }
    }
    Some(lowered)
}

fn q(value: &str) -> String {
    super::validation::string(value)
}

/// Render the generated `OAuth.swift`: the lifecycle runtime plus one compiled
/// descriptor per usable scheme. Called only when at least one usable scheme
/// exists, so no-policy output stays byte-identical.
pub(super) fn emit(plan: &SdkPlan) -> Option<String> {
    let oauth = plan.oauth.as_ref()?;
    let schemes: Vec<Scheme> = oauth
        .schemes
        .iter()
        .filter(|scheme| !executable_flows(scheme).is_empty() || scheme.discovery.is_some())
        .filter_map(lower)
        .collect();
    if schemes.is_empty() {
        return None;
    }
    let has_revocation = schemes
        .iter()
        .any(|scheme| !scheme.revocation_url.is_empty());
    let has_introspection = schemes
        .iter()
        .any(|scheme| !scheme.introspection_url.is_empty());
    let has_authorization = schemes
        .iter()
        .any(|scheme| !scheme.authorization_url.is_empty());
    let has_device = schemes.iter().any(|scheme| !scheme.device_url.is_empty());
    // Emission stays byte-identical without a discovery URL: the plain
    // lifecycle concatenates into exactly the pre-discovery bytes, and every
    // discovery-aware section (the header paragraph, the discovery-failed
    // error kind, the discovery engine and the endpoint-resolution providers)
    // is emitted only when at least one compiled scheme carries a discovery
    // URL.
    let has_discovery = schemes
        .iter()
        .any(|scheme| !scheme.discovery_url.is_empty());
    // The replaying credential wrapper joins only when a compiled scheme
    // carries an executable client-credentials flow, the provider it wraps;
    // every other plan compiles exactly the pre-replay bytes.
    let has_client_credentials = schemes
        .iter()
        .any(|scheme| !scheme.client_credentials.is_empty());
    let no_replay = if has_client_credentials {
        no_replay_requirements(&schemes, plan.operations())
    } else {
        BTreeMap::new()
    };

    let mut out =
        String::from("import Foundation\n#if canImport(CryptoKit)\nimport CryptoKit\n#endif\n\n");
    out.push_str(
        "/// Generated OAuth 2.0 / OpenID Connect lifecycle for the source-declared\n/// schemes compiled into ``OAuthCatalog`` below. Every endpoint, environment\n/// variable name and policy value is a generation-time constant: the runtime\n/// never reads OpenAPI, never invents an endpoint and never embeds credential\n/// values. Client identity comes from explicit call arguments or the compiled\n/// environment variable names, read at call time.\n///\n/// Implemented here: client-credentials acquisition with skew-aware caching\n/// and per-key single-flight inside ``OAuthSession``, explicit refresh, and —\n/// exactly for the schemes compiled below — authorization-code with PKCE S256,\n/// RFC 8628 device authorization with interval, authorization_pending,\n/// slow_down and expiry polling, RFC 7009 revocation and RFC 7662\n/// introspection. Implicit and password flows are never executed: generated\n/// code does not perform interactive resource-owner credential handling.\n",
    );
    out.push_str(if has_discovery {
        "/// When a compiled scheme carries a discovery URL, the discovery document\n/// resolves the endpoint URLs the compiled flows omit: compiled endpoints\n/// always win, the document is fetched once per session and cached, and a\n/// failed fetch is retried on the next call.\n///\n"
    } else {
        "/// OpenID Connect discovery remains caller-owned for schemes without an\n/// executable declared flow.\n///\n"
    });
    out.push_str(
        "///\n/// Token sets live only in the token store owned by one ``OAuthSession``\n/// instance, under keys partitioned by scheme, token-endpoint issuer and\n/// client identity; there is no package-level token cache. Error values never\n/// carry token or client-secret material. Token endpoint requests use the\n/// client's own transport, so caller transport policy covers the lifecycle\n/// too.\n///\n/// Compiled source schemes: ",
    );
    let names = schemes
        .iter()
        .map(|scheme| scheme.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&names);
    out.push_str(".\n\n");
    out.push_str(&descriptors(&schemes));
    for section in if has_discovery { CORE_DISCOVERY } else { CORE } {
        out.push_str(section);
    }
    if has_revocation {
        out.push_str(if has_discovery {
            REVOCATION_DISCOVERY
        } else {
            REVOCATION
        });
    }
    if has_introspection {
        out.push_str(if has_discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION
        });
    }
    if has_authorization {
        out.push_str(AUTHORIZATION_CODE);
    }
    if has_device {
        out.push_str(DEVICE);
    }
    if has_client_credentials {
        out.push_str(&replay_section(&no_replay, has_discovery));
    }
    Some(out)
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
fn no_replay_requirements(
    schemes: &[Scheme],
    operations: &[super::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.protocol().responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), wire::Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.protocol().security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    wire::CredentialHook::OAuth2 { .. }
                        | wire::CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if schemes
                    .iter()
                    .any(|scheme| scheme.name == requirement.name())
                {
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

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow; its plain
/// and discovery variants resolve the refresh endpoint and the
/// lifecycle-endpoint exclusion through the same compiled precedence as the
/// provider they wrap.
fn replay_section(no_replay: &BTreeMap<String, BTreeSet<String>>, discovery: bool) -> String {
    let mut code = String::from(
        "\n/// Compiled stream-protected requirements: security-requirement source\n/// pointers whose attaches are never replayed, because delivered stream data\n/// prevents a transparent restart.\nenum OAuthReplayProtection {\n",
    );
    if no_replay.is_empty() {
        code.push_str("    static let noReplay: [String: Set<String>] = [:]\n");
    } else {
        code.push_str("    static let noReplay: [String: Set<String>] = [\n");
        for (name, pointers) in no_replay {
            if pointers.is_empty() {
                continue;
            }
            let rendered = pointers
                .iter()
                .map(|pointer| q(pointer))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(code, "        {}: [{}],", q(name), rendered);
        }
        code.push_str("    ]\n");
    }
    code.push_str("}\n");
    code.push_str(if discovery {
        REPLAY_DISCOVERY
    } else {
        REPLAY_PLAIN
    });
    code
}

/// The compiled descriptor type and catalog: one entry per usable scheme.
fn descriptors(schemes: &[Scheme]) -> String {
    let mut code = String::from(
        "/// The compiled lifecycle descriptor of one source scheme. Empty fields\n/// mean \"not compiled\": endpoints are never invented and supplemental\n/// endpoints exist only when configuration supplied them.\nstruct OAuthSchemeDescriptor: Sendable {\n    /// The source Security Scheme name used by every lifecycle call.\n    let name: String\n    /// \"client-secret-basic\" when the compiled configuration supplies a\n    /// client secret variable, else \"none\" (a public client).\n    let clientAuth: String\n    /// Environment variable names, read at call time; generation embeds no\n    /// credential values. Empty means absent.\n    let clientIDEnv: String\n    let clientSecretEnv: String\n    /// The refresh-before-expiry clock skew, in seconds.\n    let skew: TimeInterval\n    /// The first executable flow's token endpoint; `refreshURL` is the\n    /// declared refresh URL, else the token endpoint.\n    let tokenURL: String\n    let refreshURL: String\n    /// Per-flow endpoints, empty when the flow is absent or not executable.\n    let clientCredentialsURL: String\n    let authorizationURL: String\n    let codeTokenURL: String\n    let deviceURL: String\n    let deviceTokenURL: String\n    /// Supplemental endpoints, compiled from configuration only.\n    let revocationURL: String\n    let introspectionURL: String\n    /// The compiled discovery/metadata document URL; empty when the scheme\n    /// compiles none, so endpoint resolution never invents one. A compiled\n    /// flow endpoint always wins over a discovered one.\n    let discoveryURL: String\n}\n\n/// The compiled lifecycle catalog: one entry per usable source scheme.\n/// Schemes whose only declared flows are implicit or password are never\n/// compiled here, so they emit nothing.\nenum OAuthCatalog {\n    static let schemes: [String: OAuthSchemeDescriptor] = [\n",
    );
    for scheme in schemes {
        let _ = write!(
            code,
            "        {}: OAuthSchemeDescriptor(\n            name: {},\n            clientAuth: {},\n            clientIDEnv: {},\n            clientSecretEnv: {},\n            skew: {},\n            tokenURL: {},\n            refreshURL: {},\n            clientCredentialsURL: {},\n            authorizationURL: {},\n            codeTokenURL: {},\n            deviceURL: {},\n            deviceTokenURL: {},\n            revocationURL: {},\n            introspectionURL: {},\n            discoveryURL: {}),\n",
            q(&scheme.name),
            q(&scheme.name),
            q(scheme.client_auth),
            q(&scheme.client_id_env),
            q(&scheme.client_secret_env),
            scheme.skew,
            q(&scheme.token_url),
            q(&scheme.refresh_url),
            q(&scheme.client_credentials),
            q(&scheme.authorization_url),
            q(&scheme.code_token_url),
            q(&scheme.device_url),
            q(&scheme.device_token_url),
            q(&scheme.revocation_url),
            q(&scheme.introspection_url),
            q(&scheme.discovery_url),
        );
    }
    code.push_str("    ]\n}\n\n");
    code
}

/// The typed OAuth failure with the plain kind set: no compiled scheme
/// resolves endpoints through discovery.
const AUTH_PLAIN: &str = r#"
/// Typed OAuth lifecycle failure. `kind` and `scheme` classify it; `status`
/// carries the endpoint HTTP status when one was reached, and `code` carries
/// the server's declared error. Descriptions and this value never contain
/// token values, client secrets or response bodies.
public struct AuthError: Error, Sendable, CustomStringConvertible {
    public enum Kind: String, Sendable {
        case unknownScheme, unsupportedFlow, requestValidation, cancelled
        case transport, invalidResponse, serverError, authorizationError
        case tokenStore, random, stateMismatch, stateConsumed, expired, resourceLimit
    }
    public let kind: Kind
    public let scheme: String
    public let status: Int?
    public let code: String?
    public let cause: (any Error)?
    init(_ kind: Kind, scheme: String, status: Int? = nil, code: String? = nil, cause: (any Error)? = nil) {
        self.kind = kind; self.scheme = scheme; self.status = status
        self.code = code; self.cause = cause
    }
    public var description: String {
        var text = "oauth \(kind.rawValue) failure"
        if !scheme.isEmpty { text += " for scheme \(scheme)" }
        if let code, !code.isEmpty { text += ": \(code)" }
        return text
    }
}
"#;

/// The typed OAuth failure with the discovery kind added: a compiled scheme
/// with a discovery URL resolves endpoint URLs through the discovery
/// document, and every discovery failure is typed `discovery-failed` and
/// never carries response body text.
const AUTH_DISCOVERY: &str = r#"
/// Typed OAuth lifecycle failure. `kind` and `scheme` classify it; `status`
/// carries the endpoint HTTP status when one was reached, and `code` carries
/// the server's declared error. Descriptions and this value never contain
/// token values, client secrets or response bodies.
public struct AuthError: Error, Sendable, CustomStringConvertible {
    public enum Kind: String, Sendable {
        case unknownScheme = "unknownScheme"
        case unsupportedFlow = "unsupportedFlow"
        case requestValidation = "requestValidation"
        case cancelled = "cancelled"
        case transport = "transport"
        case invalidResponse = "invalidResponse"
        case serverError = "serverError"
        case authorizationError = "authorizationError"
        case tokenStore = "tokenStore"
        case random = "random"
        case stateMismatch = "stateMismatch"
        case stateConsumed = "stateConsumed"
        case expired = "expired"
        case resourceLimit = "resourceLimit"
        /// The discovery document was unreachable, unreadable, issuer
        /// mismatched, oversized or carried an unusable endpoint value.
        case discoveryFailed = "discovery-failed"
    }
    public let kind: Kind
    public let scheme: String
    public let status: Int?
    public let code: String?
    public let cause: (any Error)?
    init(_ kind: Kind, scheme: String, status: Int? = nil, code: String? = nil, cause: (any Error)? = nil) {
        self.kind = kind; self.scheme = scheme; self.status = status
        self.code = code; self.cause = cause
    }
    public var description: String {
        var text = "oauth \(kind.rawValue) failure"
        if !scheme.isEmpty { text += " for scheme \(scheme)" }
        if let code, !code.isEmpty { text += ": \(code)" }
        return text
    }
}
"#;

/// Token plumbing shared by every compiled flow.
const TOKEN_CORE: &str = r#"
/// One decoded token-endpoint response (RFC 6749 section 5.1). `expiresAt` is
/// derived from `expires_in`; nil means the server declared no lifetime, so
/// the set never expires locally. `refreshToken` holds the server's rotated
/// token, or the previous set's token when the server returned none.
public struct TokenSet: Sendable {
    public let accessToken: String
    public let tokenType: String
    public let expiresAt: Date?
    public let refreshToken: String?
    public let scope: String?
    /// Whether this set carries an issued or retained refresh token.
    public let hasRefresh: Bool

    public init(accessToken: String, tokenType: String = "Bearer", expiresAt: Date? = nil,
                refreshToken: String? = nil, scope: String? = nil) {
        self.accessToken = accessToken
        self.tokenType = tokenType.isEmpty ? "Bearer" : tokenType
        self.expiresAt = expiresAt
        self.refreshToken = refreshToken
        self.scope = scope
        self.hasRefresh = !(refreshToken ?? "").isEmpty
    }

    /// The complete Authorization field consumed by source-declared oauth2 and
    /// openid-connect operations: the server's token type, or the conventional
    /// Bearer when the response omitted one.
    public var authorization: String { "\(tokenType) \(accessToken)" }

    /// Whether the set is stale at the skew-adjusted expiry.
    func expired(skew: TimeInterval) -> Bool {
        guard let expiresAt else { return false }
        return Date().addingTimeInterval(skew) >= expiresAt
    }
}

/// Persists token sets under opaque keys. Implementations must be safe for
/// concurrent use. Keys are partitioned by scheme, token-endpoint issuer and
/// client identity; treat them as read-only routing information.
public protocol TokenStore: Sendable {
    func load(key: String) async throws -> TokenSet?
    func replace(key: String, set: TokenSet) async throws
    func clear(key: String) async throws
}

/// Instance-owned in-process token store guarded by a lock, following the
/// package transport's `NSLock` idiom. Each instance guards its own keys;
/// generated clients never share a store implicitly and there is no
/// package-level token cache. The caller owns the instance: share it only by
/// passing it explicitly.
public final class MemoryTokenStore: TokenStore, @unchecked Sendable {
    private let lock = NSLock()
    private var sets: [String: TokenSet] = [:]
    public init() {}
    public func load(key: String) throws -> TokenSet? {
        lock.withLock { sets[key] }
    }
    public func replace(key: String, set: TokenSet) throws {
        lock.withLock { sets[key] = set }
    }
    public func clear(key: String) throws {
        _ = lock.withLock { sets.removeValue(forKey: key) }
    }
}

/// Source marker for lifecycle failures, distinct from every operation source.
let oauthSource = SourceLocation(document: "suspect-oauth", pointer: "")
/// Bounded one-token/device/revocation/introspection response.
let oauthMaxResponseBytes = 1 << 20
/// Finite token-request deadline.
let oauthTokenTimeout: TimeInterval = 30
/// The RFC 8628 device grant's token-endpoint grant type.
let oauthDeviceGrant = "urn:ietf:params:oauth:grant-type:device_code"

/// Shared lifecycle helpers: compiled-scheme lookup, client-identity
/// resolution and exact form encoding. Never reads OpenAPI at runtime.
enum OAuthLifecycle {
    static let jsonLimits = JsonLimits(maxBytes: oauthMaxResponseBytes, maxDepth: 32,
                                       maxNodes: 10_000, maxNumberBytes: 256)

    static func scheme(_ name: String) throws -> OAuthSchemeDescriptor {
        guard let descriptor = OAuthCatalog.schemes[name] else {
            throw AuthError(.unknownScheme, scheme: name)
        }
        return descriptor
    }

    /// Resolves the call's client identity: explicit arguments first, then the
    /// compiled environment variable names, read at call time. Empty values
    /// mean absent; callers may legitimately configure only one.
    static func resolve(_ descriptor: OAuthSchemeDescriptor, clientID: String?, clientSecret: String?)
        -> (id: String, secret: String) {
        var id = clientID ?? ""
        var secret = clientSecret ?? ""
        if id.isEmpty, !descriptor.clientIDEnv.isEmpty {
            id = ProcessInfo.processInfo.environment[descriptor.clientIDEnv] ?? ""
        }
        if secret.isEmpty, !descriptor.clientSecretEnv.isEmpty {
            secret = ProcessInfo.processInfo.environment[descriptor.clientSecretEnv] ?? ""
        }
        return (id, secret)
    }

    /// Store keys partition by scheme, token-endpoint issuer and client
    /// identity, so distinct clients and endpoints never share a set.
    static func storeKey(scheme: String, issuer: String, clientID: String) -> String {
        "\(scheme)\u{1F}\(issuer)\u{1F}\(clientID)"
    }

    /// RFC 6749 request form encoding: application/x-www-form-urlencoded over
    /// the RFC 3986 unreserved alphabet, so spaces stay exactly encoded.
    static func form(_ fields: [(String, String)]) -> Data {
        Data(fields.map { "\(escape($0.0))=\(escape($0.1))" }.joined(separator: "&").utf8)
    }

    static func escape(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: .oauthFormUnreserved) ?? value
    }

    /// RFC 6749 2.3.1 Basic credentials: form-encoded id and secret.
    static func basic(_ clientID: String, _ clientSecret: String) -> String {
        Data("\(clientID):\(clientSecret)".utf8).base64EncodedString()
    }

    static func decodeObject(_ body: Data, scheme: String, status: Int) throws -> JsonObject<JsonValue> {
        let value: JsonValue
        do { value = try JsonValue.parse(body, limits: jsonLimits) }
        catch { throw AuthError(.invalidResponse, scheme: scheme, status: status) }
        guard case .object(let object) = value else {
            throw AuthError(.invalidResponse, scheme: scheme, status: status)
        }
        return object
    }

    static func string(_ object: JsonObject<JsonValue>, _ key: String) -> String? {
        guard case .string(let value)? = object[key] else { return nil }
        return value
    }

    static func integer(_ object: JsonObject<JsonValue>, _ key: String) -> Int64? {
        guard case .number(let number)? = object[key] else { return nil }
        return Int64(number.raw)
    }

    static func boolean(_ object: JsonObject<JsonValue>, _ key: String) -> Bool {
        if case .bool(let value)? = object[key] { return value }
        return false
    }
}

extension CharacterSet {
    /// The RFC 3986 unreserved set: the exact application/x-www-form-urlencoded
    /// alphabet after space handling.
    static let oauthFormUnreserved = CharacterSet(charactersIn:
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
}

extension Data {
    /// RFC 7636 base64url: 43 characters per 32 bytes, no padding.
    func oauthBase64URL() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// Returns 256 bits of system CSPRNG entropy in the RFC 7636 base64url
/// alphabet, via CryptoKit's system-seeded key generator.
func oauthRandomValue(scheme: String) throws -> String {
    #if canImport(CryptoKit)
    return SymmetricKey(size: .bits256).withUnsafeBytes { buffer in Data(buffer) }.oauthBase64URL()
    #else
    // TODO: this platform has no compiled cryptographic randomness source, so
    // transactions needing one-time values are refused instead of falling back
    // to weak entropy.
    throw AuthError(.unsupportedFlow, scheme: scheme)
    #endif
}
"#;

/// The shared form/token request helpers, unchanged by discovery.
const REQUEST_HELPERS: &str = r#"
extension Client {
    /// Posts one form-encoded request to a compiled OAuth endpoint and returns
    /// the status and bounded body. It applies the scheme's compiled client
    /// authentication: HTTP Basic for confidential clients, the client_id form
    /// member for public ones.
    func oauthEndpointRequest(_ endpoint: String, scheme: String, clientID: String,
                              clientSecret: String, form: [(String, String)])
        async throws -> (status: Int, body: Data) {
        guard let url = URL(string: endpoint), url.host != nil else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        var fields = form
        var basic = false
        if let descriptor = OAuthCatalog.schemes[scheme], descriptor.clientAuth == "client-secret-basic" {
            basic = true
        } else if !clientID.isEmpty {
            fields.append(("client_id", clientID))
        }
        var headers = [
            HTTPHeader("Content-Type", "application/x-www-form-urlencoded"),
            HTTPHeader("Accept", "application/json"),
        ]
        if basic {
            headers.append(HTTPHeader("Authorization", "Basic \(OAuthLifecycle.basic(clientID, clientSecret))"))
        }
        let request = HTTPRequest(method: "POST", url: url, headers: headers,
                                  body: OAuthLifecycle.form(fields), timeout: oauthTokenTimeout,
                                  maxResponseBytes: oauthMaxResponseBytes)
        let response: HTTPResponse
        do { response = try await send(request, source: oauthSource) }
        catch { throw AuthError(.transport, scheme: scheme, cause: error) }
        return (response.status, response.body)
    }

    /// Executes one RFC 6749 token-endpoint request and decodes the response.
    /// `retainedRefresh` keeps the previous set's refresh token when the server
    /// returns no rotated one.
    func oauthTokenRequest(_ endpoint: String, scheme: String, clientID: String,
                           clientSecret: String, form: [(String, String)],
                           retainedRefresh: String?) async throws -> TokenSet {
        let (status, body) = try await oauthEndpointRequest(
            endpoint, scheme: scheme, clientID: clientID, clientSecret: clientSecret, form: form)
        let object = try OAuthLifecycle.decodeObject(body, scheme: scheme, status: status)
        if let declared = OAuthLifecycle.string(object, "error"), !declared.isEmpty {
            throw AuthError(.authorizationError, scheme: scheme, status: status, code: declared)
        }
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: scheme, status: status)
        }
        guard let access = OAuthLifecycle.string(object, "access_token"), !access.isEmpty else {
            throw AuthError(.invalidResponse, scheme: scheme, status: status)
        }
        var tokenType = OAuthLifecycle.string(object, "token_type") ?? ""
        if tokenType.isEmpty { tokenType = "Bearer" }
        var expiresAt: Date?
        if let seconds = OAuthLifecycle.integer(object, "expires_in"), seconds > 0 {
            expiresAt = Date().addingTimeInterval(TimeInterval(seconds))
        }
        let set = TokenSet(accessToken: access, tokenType: tokenType, expiresAt: expiresAt,
                           refreshToken: OAuthLifecycle.string(object, "refresh_token"),
                           scope: OAuthLifecycle.string(object, "scope"))
        // Adopt a rotated refresh token; retain the previous one otherwise.
        if !set.hasRefresh, let retainedRefresh, !retainedRefresh.isEmpty {
            return TokenSet(accessToken: set.accessToken, tokenType: set.tokenType,
                            expiresAt: set.expiresAt, refreshToken: retainedRefresh, scope: set.scope)
        }
        return set
    }
"#;

/// Client-credentials acquisition and refresh at the plain compiled
/// endpoints: no scheme resolves endpoints through discovery.
const CREDENTIALS_PLAIN: &str = r#"
    /// Returns the scheme's cached token set from `store`, acquiring one from
    /// the compiled client-credentials endpoint when the stored set is absent
    /// or expired beyond the compiled skew. This primitive does not serialize
    /// concurrent callers; wrap it in an `OAuthSession` for per-key
    /// single-flight. The returned set is a value; callers may keep it.
    public func clientCredentialsToken(scheme: String, store: any TokenStore,
                                       clientID: String? = nil, clientSecret: String? = nil)
        async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        let key = OAuthLifecycle.storeKey(scheme: scheme, issuer: descriptor.tokenURL, clientID: identity.id)
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        if let stored, !stored.expired(skew: descriptor.skew) {
            return stored
        }
        let set = try await oauthTokenRequest(
            descriptor.clientCredentialsURL, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret, form: [("grant_type", "client_credentials")],
            retainedRefresh: stored?.refreshToken)
        do { try await store.replace(key: key, set: set) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        return set
    }

    /// Exchanges the set's refresh token (RFC 6749 section 6) at the scheme's
    /// declared refresh URL, or its token endpoint when no refresh URL is
    /// declared. The returned set adopts a rotated refresh token and retains
    /// the given one otherwise. The store is neither read nor updated: callers
    /// decide which set to keep.
    public func refreshToken(scheme: String, set: TokenSet) async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard set.hasRefresh, let refresh = set.refreshToken, !refresh.isEmpty else {
            throw AuthError(.requestValidation, scheme: scheme)
        }
        guard !descriptor.refreshURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        return try await oauthTokenRequest(
            descriptor.refreshURL, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret,
            form: [("grant_type", "refresh_token"), ("refresh_token", refresh)],
            retainedRefresh: refresh)
    }
}
"#;

/// The session with per-key single-flight over the plain compiled endpoints.
const SESSION_PLAIN: &str = r#"
/// One client's OAuth token state: an instance-owned token store plus per-key
/// single-flight. The generated `Client` is an immutable value type without
/// identity, so the caller constructs one session per client and owns it:
/// token sets live only inside the session's own store and there is no
/// package-level cache. One-shot steps that need no state stay on `Client`.
public actor OAuthSession {
    private let client: Client
    private let store: any TokenStore
    /// Per-key in-flight acquisition rounds; installed before any await and
    /// removed by the round that installed it.
    private var flights: [String: Task<TokenSet, any Error>] = [:]

    /// Creates the session's own state for one client. `store` defaults to a
    /// fresh instance-owned `MemoryTokenStore`; share stores only explicitly.
    public init(client: Client, store: (any TokenStore)? = nil) {
        self.client = client
        self.store = store ?? MemoryTokenStore()
    }

    /// The scheme's cached token set, acquiring one through the session's
    /// client when the stored set is absent or expired beyond the compiled
    /// skew. Concurrent callers on one session share a single acquisition:
    /// callers that arrive while an acquisition is in flight await its result
    /// instead of acquiring again.
    public func clientCredentialsToken(scheme: String, clientID: String? = nil,
                                       clientSecret: String? = nil) async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        let key = OAuthLifecycle.storeKey(scheme: scheme, issuer: descriptor.tokenURL, clientID: identity.id)
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        if let stored, !stored.expired(skew: descriptor.skew) {
            return stored
        }
        // Single-flight: the first caller for a key starts the acquisition;
        // later callers await the same round. A caller that waited re-reads the
        // store inside the acquisition, so the holder's populated set wins.
        var flight: Task<TokenSet, any Error>
        let installed: Bool
        if let running = flights[key] {
            flight = running
            installed = false
        } else {
            flight = Task { [client, store] in
                try await client.clientCredentialsToken(
                    scheme: scheme, store: store, clientID: identity.id, clientSecret: identity.secret)
            }
            flights[key] = flight
            installed = true
        }
        defer { if installed { flights[key] = nil } }
        return try await flight.value
    }

    /// Exchanges the set's refresh token at the scheme's declared refresh URL,
    /// or its token endpoint. The returned set adopts a rotated refresh token
    /// and retains the given one otherwise. The store is neither read nor
    /// updated: callers decide which set to keep.
    public func refreshToken(scheme: String, set: TokenSet) async throws -> TokenSet {
        try await client.refreshToken(scheme: scheme, set: set)
    }

    /// Builds the generated credential hook for one compiled scheme: the
    /// client's oauth2/openid-connect attach path calls it and receives the
    /// complete Authorization field from the session's cached or freshly
    /// acquired token set. The factory itself touches no session state, so it
    /// stays callable outside the actor.
    public nonisolated func authorizationProvider(_ scheme: String, clientID: String? = nil,
                                                  clientSecret: String? = nil) -> HTTPAuthorizationProvider {
        let session = self
        return { _ in
            try await session.clientCredentialsToken(scheme: scheme, clientID: clientID,
                                                     clientSecret: clientSecret).authorization
        }
    }
}
"#;

/// Store, error, token, request and session plumbing shared by every
/// compiled flow, in emission order: exactly the pre-discovery bytes.
const CORE: &[&str] = &[
    AUTH_PLAIN,
    TOKEN_CORE,
    REQUEST_HELPERS,
    CREDENTIALS_PLAIN,
    SESSION_PLAIN,
];

/// The discovery engine, emitted only when at least one compiled scheme
/// carries a discovery URL: the typed document decode with the issuer-origin
/// rule, the one-shot fetch through the client's own transport, and the
/// endpoint-resolution precedence.
const DISCOVERY_ENGINE: &str = r#"
/// One RFC 8414 / OpenID Connect discovery document reduced to the endpoints
/// this package resolves. Unknown members are ignored; a known member must be
/// a nonempty control-character-free string when present.
struct DiscoveredEndpoints: Sendable {
    var tokenEndpoint: String?
    var revocationEndpoint: String?
    var introspectionEndpoint: String?
}

extension OAuthLifecycle {
    /// The origin of one absolute http(s) URL: scheme, host and the port with
    /// the scheme default made explicit. nil when the value is not an
    /// absolute http(s) URL.
    static func endpointOrigin(_ value: String) -> String? {
        guard let components = URLComponents(string: value),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let host = components.host, !host.isEmpty else {
            return nil
        }
        let port = components.port ?? (scheme == "http" ? 80 : 443)
        return "\(scheme)://\(host):\(port)"
    }

    /// Reads one discovery document member: absent stays nil; a non-string or
    /// unusable value is a typed discovery failure. Unknown members are
    /// ignored.
    static func discoveredEndpoint(_ object: JsonObject<JsonValue>, _ member: String,
                                   scheme: String) throws -> String? {
        guard case .string(let value)? = object[member] else { return nil }
        guard !value.isEmpty,
              !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw AuthError(.discoveryFailed, scheme: scheme)
        }
        return value
    }

    /// Decodes and validates one discovery response body into the endpoints
    /// this package resolves. The exact issuer rule: when the document carries
    /// an `issuer` claim, it must be an absolute http(s) URL whose origin
    /// (scheme, host and the port with the scheme default made explicit)
    /// equals the discovery URL's origin. A missing claim is tolerated; a
    /// mismatching or unparseable one is a typed discovery failure. Failure
    /// values carry only safe metadata, never response body text.
    static func discoveryDocument(_ body: Data, scheme: String, url: String) throws -> DiscoveredEndpoints {
        let value: JsonValue
        do { value = try JsonValue.parse(body, limits: jsonLimits) }
        catch { throw AuthError(.discoveryFailed, scheme: scheme) }
        guard case .object(let object) = value else {
            throw AuthError(.discoveryFailed, scheme: scheme)
        }
        if let issuer = string(object, "issuer"), !issuer.isEmpty {
            guard let issuerOrigin = endpointOrigin(issuer),
                  let discoveryOrigin = endpointOrigin(url),
                  issuerOrigin == discoveryOrigin else {
                throw AuthError(.discoveryFailed, scheme: scheme)
            }
        }
        var endpoints = DiscoveredEndpoints()
        endpoints.tokenEndpoint = try discoveredEndpoint(object, "token_endpoint", scheme: scheme)
        endpoints.revocationEndpoint = try discoveredEndpoint(object, "revocation_endpoint", scheme: scheme)
        endpoints.introspectionEndpoint = try discoveredEndpoint(object, "introspection_endpoint", scheme: scheme)
        return endpoints
    }
}

extension Client {
    /// Fetches the scheme's discovery document (GET, `accept: application/json`)
    /// through the client's own transport, bounded at the compiled response
    /// ceiling. One-shot helpers fetch per call and keep no cache;
    /// ``OAuthSession`` caches per scheme instead.
    func discoveryDocument(_ descriptor: OAuthSchemeDescriptor) async throws -> DiscoveredEndpoints {
        guard let url = URL(string: descriptor.discoveryURL), url.host != nil else {
            throw AuthError(.discoveryFailed, scheme: descriptor.name)
        }
        let request = HTTPRequest(method: "GET", url: url,
                                  headers: [HTTPHeader("Accept", "application/json")],
                                  body: nil, timeout: oauthTokenTimeout,
                                  maxResponseBytes: oauthMaxResponseBytes)
        let response: HTTPResponse
        do { response = try await send(request, source: oauthSource) }
        catch { throw AuthError(.discoveryFailed, scheme: descriptor.name, cause: error) }
        guard (200..<300).contains(response.status) else {
            throw AuthError(.discoveryFailed, scheme: descriptor.name, status: response.status)
        }
        return try OAuthLifecycle.discoveryDocument(
            response.body, scheme: descriptor.name, url: descriptor.discoveryURL)
    }

    /// The scheme's discovery document, or nil when the compiled plan carries
    /// no discovery URL, so the compiled refusal stands.
    func discoveryEndpoints(_ descriptor: OAuthSchemeDescriptor) async throws -> DiscoveredEndpoints? {
        if descriptor.discoveryURL.isEmpty { return nil }
        return try await discoveryDocument(descriptor)
    }

    /// One lifecycle endpoint through the compiled precedence for one-shot
    /// callers: the compiled endpoint always wins; otherwise the discovery
    /// document (fetched per call, never cached here) resolves the member;
    /// otherwise the compiled refusal stands.
    func resolveEndpoint(_ descriptor: OAuthSchemeDescriptor, compiled: String,
                         member: KeyPath<DiscoveredEndpoints, String?>) async throws -> String {
        if !compiled.isEmpty { return compiled }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let resolved = discovered[keyPath: member] else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return resolved
    }
}
"#;

/// Client-credentials acquisition and refresh with the discovery-aware
/// endpoint precedence: the compiled endpoints always win; otherwise the
/// discovery document resolves the token endpoint.
const CREDENTIALS_DISCOVERY: &str = r#"
    /// The client-credentials endpoint and its store-key issuer through the
    /// compiled precedence: the compiled client-credentials token URL always
    /// wins; otherwise the discovery document resolves the token endpoint.
    /// One-shot callers fetch discovery per call; ``OAuthSession`` resolves
    /// against its cached document instead.
    func clientCredentialsEndpoint(_ descriptor: OAuthSchemeDescriptor) async throws -> (endpoint: String, issuer: String) {
        if !descriptor.clientCredentialsURL.isEmpty {
            return (descriptor.clientCredentialsURL, descriptor.tokenURL)
        }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let endpoint = discovered.tokenEndpoint else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return (endpoint, endpoint)
    }

    /// Acquires, or serves from the store, the scheme's client-credentials set
    /// at one resolved endpoint. Shared by the one-shot call and the
    /// single-flighted session round.
    func clientCredentials(_ descriptor: OAuthSchemeDescriptor, endpoint: String,
                           issuer: String, store: any TokenStore,
                           clientID: String, clientSecret: String) async throws -> TokenSet {
        let key = OAuthLifecycle.storeKey(scheme: descriptor.name, issuer: issuer, clientID: clientID)
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: descriptor.name, cause: error) }
        if let stored, !stored.expired(skew: descriptor.skew) {
            return stored
        }
        let set = try await oauthTokenRequest(
            endpoint, scheme: descriptor.name, clientID: clientID,
            clientSecret: clientSecret, form: [("grant_type", "client_credentials")],
            retainedRefresh: stored?.refreshToken)
        do { try await store.replace(key: key, set: set) }
        catch { throw AuthError(.tokenStore, scheme: descriptor.name, cause: error) }
        return set
    }

    /// Returns the scheme's cached token set from `store`, acquiring one from
    /// the compiled client-credentials endpoint — or, when the compiled plan
    /// carries no such flow, from the scheme's discovery document's
    /// `token_endpoint` — when the stored set is absent or expired beyond the
    /// compiled skew. This primitive does not serialize concurrent callers;
    /// wrap it in an `OAuthSession` for per-key single-flight. The returned
    /// set is a value; callers may keep it.
    public func clientCredentialsToken(scheme: String, store: any TokenStore,
                                       clientID: String? = nil, clientSecret: String? = nil)
        async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty || !descriptor.discoveryURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        let resolved = try await clientCredentialsEndpoint(descriptor)
        return try await clientCredentials(
            descriptor, endpoint: resolved.endpoint, issuer: resolved.issuer, store: store,
            clientID: identity.id, clientSecret: identity.secret)
    }

    /// The refresh endpoint through the compiled precedence: the declared
    /// refresh URL, else the compiled token URL, always win; otherwise the
    /// discovery document's token endpoint resolves the exchange.
    func refreshEndpoint(_ descriptor: OAuthSchemeDescriptor) async throws -> String {
        if !descriptor.refreshURL.isEmpty { return descriptor.refreshURL }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let endpoint = discovered.tokenEndpoint else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return endpoint
    }

    /// Exchanges the set's refresh token (RFC 6749 section 6) at the scheme's
    /// declared refresh URL, or its token endpoint when no refresh URL is
    /// declared — or, when the compiled plan carries neither, at the discovery
    /// document's token endpoint. The returned set adopts a rotated refresh
    /// token and retains the given one otherwise. The store is neither read
    /// nor updated: callers decide which set to keep.
    public func refreshToken(scheme: String, set: TokenSet) async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard set.hasRefresh, let refresh = set.refreshToken, !refresh.isEmpty else {
            throw AuthError(.requestValidation, scheme: scheme)
        }
        let endpoint = try await refreshEndpoint(descriptor)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        return try await oauthTokenRequest(
            endpoint, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret,
            form: [("grant_type", "refresh_token"), ("refresh_token", refresh)],
            retainedRefresh: refresh)
    }
}
"#;

/// The session with per-key single-flight and the per-scheme discovery cache:
/// successful documents are cached for the session's lifetime, concurrent
/// callers share one in-flight fetch, and a failed fetch is retried on the
/// next call.
const SESSION_DISCOVERY: &str = r#"
/// One client's OAuth token state: an instance-owned token store, per-key
/// single-flight and a per-scheme discovery cache. The generated `Client` is
/// an immutable value type without identity, so the caller constructs one
/// session per client and owns it: token sets live only inside the session's
/// own store and there is no package-level cache. One-shot steps that need no
/// state stay on `Client`.
public actor OAuthSession {
    private let client: Client
    private let store: any TokenStore
    /// Per-key in-flight acquisition rounds; installed before any await and
    /// removed by the round that installed it.
    private var flights: [String: Task<TokenSet, any Error>] = [:]
    /// Per-scheme successful discovery documents, cached for the session's
    /// lifetime so repeated calls never re-fetch.
    private var discovered: [String: DiscoveredEndpoints] = [:]
    /// Per-scheme in-flight discovery fetches; installed before any await and
    /// removed by the round that installed it, so a failed fetch is retried on
    /// the next call.
    private var discoveryFlights: [String: Task<DiscoveredEndpoints, any Error>] = [:]

    /// Creates the session's own state for one client. `store` defaults to a
    /// fresh instance-owned `MemoryTokenStore`; share stores only explicitly.
    public init(client: Client, store: (any TokenStore)? = nil) {
        self.client = client
        self.store = store ?? MemoryTokenStore()
    }

    /// The scheme's discovery document through the session's client. A
    /// successful document is cached for the session's lifetime; concurrent
    /// callers share the one in-flight fetch; a failed fetch is never cached,
    /// so the next call retries.
    func discoveryDocument(_ descriptor: OAuthSchemeDescriptor) async throws -> DiscoveredEndpoints {
        if let cached = discovered[descriptor.name] {
            return cached
        }
        var flight: Task<DiscoveredEndpoints, any Error>
        let installed: Bool
        if let running = discoveryFlights[descriptor.name] {
            flight = running
            installed = false
        } else {
            let client = self.client
            flight = Task { try await client.discoveryDocument(descriptor) }
            discoveryFlights[descriptor.name] = flight
            installed = true
        }
        defer { if installed { discoveryFlights[descriptor.name] = nil } }
        let document = try await flight.value
        if let cached = discovered[descriptor.name] { return cached }
        discovered[descriptor.name] = document
        return document
    }

    /// The scheme's discovery document, or nil when the compiled plan carries
    /// no discovery URL, so the compiled refusal stands.
    func discoveryEndpoints(_ descriptor: OAuthSchemeDescriptor) async throws -> DiscoveredEndpoints? {
        if descriptor.discoveryURL.isEmpty { return nil }
        return try await discoveryDocument(descriptor)
    }

    /// One lifecycle endpoint through the compiled precedence: the compiled
    /// endpoint always wins; otherwise the session's cached discovery document
    /// resolves the member; otherwise the compiled refusal stands.
    func resolveEndpoint(_ descriptor: OAuthSchemeDescriptor, compiled: String,
                         member: KeyPath<DiscoveredEndpoints, String?>) async throws -> String {
        if !compiled.isEmpty { return compiled }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let resolved = discovered[keyPath: member] else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return resolved
    }

    /// The client-credentials endpoint and its store-key issuer through the
    /// compiled precedence, resolved against the session's cached discovery
    /// document.
    func clientCredentialsEndpoint(_ descriptor: OAuthSchemeDescriptor) async throws -> (endpoint: String, issuer: String) {
        if !descriptor.clientCredentialsURL.isEmpty {
            return (descriptor.clientCredentialsURL, descriptor.tokenURL)
        }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let endpoint = discovered.tokenEndpoint else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return (endpoint, endpoint)
    }

    /// The scheme's cached token set, acquiring one through the session's
    /// client when the stored set is absent or expired beyond the compiled
    /// skew. The token endpoint follows the compiled precedence: the compiled
    /// client-credentials URL wins; otherwise the session's cached discovery
    /// document resolves it. Concurrent callers on one session share a single
    /// acquisition: callers that arrive while an acquisition is in flight
    /// await its result instead of acquiring again.
    public func clientCredentialsToken(scheme: String, clientID: String? = nil,
                                       clientSecret: String? = nil) async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty || !descriptor.discoveryURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        // Endpoint resolution happens before the acquisition flight, so
        // concurrent callers share one discovery fetch through the session's
        // own single-flight; the compiled URL short-circuits without a fetch.
        let resolved = try await clientCredentialsEndpoint(descriptor)
        let key = OAuthLifecycle.storeKey(scheme: scheme, issuer: resolved.issuer, clientID: identity.id)
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        if let stored, !stored.expired(skew: descriptor.skew) {
            return stored
        }
        // Single-flight: the first caller for a key starts the acquisition;
        // later callers await the same round. A caller that waited re-reads the
        // store inside the acquisition, so the holder's populated set wins.
        var flight: Task<TokenSet, any Error>
        let installed: Bool
        if let running = flights[key] {
            flight = running
            installed = false
        } else {
            let endpoint = resolved.endpoint
            flight = Task { [client, store] in
                try await client.clientCredentials(
                    descriptor, endpoint: endpoint, issuer: resolved.issuer, store: store,
                    clientID: identity.id, clientSecret: identity.secret)
            }
            flights[key] = flight
            installed = true
        }
        defer { if installed { flights[key] = nil } }
        return try await flight.value
    }

    /// Exchanges the set's refresh token at the scheme's declared refresh URL,
    /// or its token endpoint — or, when the compiled plan carries neither, at
    /// the session's cached discovery document's token endpoint. The returned
    /// set adopts a rotated refresh token and retains the given one otherwise.
    /// The store is neither read nor updated: callers decide which set to keep.
    public func refreshToken(scheme: String, set: TokenSet) async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard set.hasRefresh, let refresh = set.refreshToken, !refresh.isEmpty else {
            throw AuthError(.requestValidation, scheme: scheme)
        }
        let endpoint = try await refreshEndpoint(descriptor)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        return try await client.oauthTokenRequest(
            endpoint, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret,
            form: [("grant_type", "refresh_token"), ("refresh_token", refresh)],
            retainedRefresh: refresh)
    }

    /// The refresh endpoint through the compiled precedence: the declared
    /// refresh URL, else the compiled token URL, always win; otherwise the
    /// session's cached discovery document's token endpoint resolves the
    /// exchange.
    func refreshEndpoint(_ descriptor: OAuthSchemeDescriptor) async throws -> String {
        if !descriptor.refreshURL.isEmpty { return descriptor.refreshURL }
        guard let discovered = try await discoveryEndpoints(descriptor),
              let endpoint = discovered.tokenEndpoint else {
            throw AuthError(.unsupportedFlow, scheme: descriptor.name)
        }
        return endpoint
    }

    /// Builds the generated credential hook for one compiled scheme: the
    /// client's oauth2/openid-connect attach path calls it and receives the
    /// complete Authorization field from the session's cached or freshly
    /// acquired token set. The factory itself touches no session state, so it
    /// stays callable outside the actor.
    public nonisolated func authorizationProvider(_ scheme: String, clientID: String? = nil,
                                                  clientSecret: String? = nil) -> HTTPAuthorizationProvider {
        let session = self
        return { _ in
            try await session.clientCredentialsToken(scheme: scheme, clientID: clientID,
                                                     clientSecret: clientSecret).authorization
        }
    }
}
"#;

/// The lifecycle with discovery support: the discovery-aware failure kinds,
/// the discovery engine, and the discovery-aware acquisition and session, in
/// emission order. The engine stays outside the `extension Client` block the
/// request helpers open and the credential providers close.
const CORE_DISCOVERY: &[&str] = &[
    AUTH_DISCOVERY,
    TOKEN_CORE,
    DISCOVERY_ENGINE,
    REQUEST_HELPERS,
    CREDENTIALS_DISCOVERY,
    SESSION_DISCOVERY,
];

/// RFC 7009 revocation, emitted only when a compiled scheme carries a
/// revocation endpoint. The plain variant refuses every scheme without a
/// compiled endpoint.
const REVOCATION: &str = r#"
extension Client {
    /// Posts the token value to the scheme's compiled revocation endpoint
    /// (RFC 7009). Any 2xx response is success: RFC 7009 declares the token
    /// revoked even when the server reports an unsupported-token error.
    public func revokeToken(scheme: String, tokenValue: String) async throws {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.revocationURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let (status, body) = try await oauthEndpointRequest(
            descriptor.revocationURL, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret, form: [("token", tokenValue)])
        if let object = try? OAuthLifecycle.decodeObject(body, scheme: scheme, status: status),
           let declared = OAuthLifecycle.string(object, "error"), !declared.isEmpty {
            throw AuthError(.authorizationError, scheme: scheme, status: status, code: declared)
        }
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: scheme, status: status)
        }
    }
}

extension OAuthSession {
    /// Revokes the token value through the session's client. See
    /// ``Client/revokeToken(scheme:tokenValue:)``.
    public func revokeToken(scheme: String, tokenValue: String) async throws {
        try await client.revokeToken(scheme: scheme, tokenValue: tokenValue)
    }
}
"#;

/// RFC 7009 revocation with the discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `revocation_endpoint`
/// resolves the request.
const REVOCATION_DISCOVERY: &str = r#"
extension Client {
    /// Posts the token value to the scheme's revocation endpoint (RFC 7009).
    /// Any 2xx response is success: RFC 7009 declares the token revoked even
    /// when the server reports an unsupported-token error.
    ///
    /// Endpoint resolution follows the compiled precedence: the configured
    /// revocation endpoint always wins; otherwise the discovery document's
    /// `revocation_endpoint` resolves the request (fetched per call by this
    /// one-shot helper, with the session's cached document used by
    /// ``OAuthSession``).
    public func revokeToken(scheme: String, tokenValue: String) async throws {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let endpoint = try await resolveEndpoint(
            descriptor, compiled: descriptor.revocationURL, member: \.revocationEndpoint)
        try await revokeToken(descriptor, endpoint: endpoint, identity: identity, tokenValue: tokenValue)
    }

    /// Posts the token value to one resolved revocation endpoint.
    func revokeToken(_ descriptor: OAuthSchemeDescriptor, endpoint: String,
                     identity: (id: String, secret: String), tokenValue: String) async throws {
        let (status, body) = try await oauthEndpointRequest(
            endpoint, scheme: descriptor.name, clientID: identity.id,
            clientSecret: identity.secret, form: [("token", tokenValue)])
        if let object = try? OAuthLifecycle.decodeObject(body, scheme: descriptor.name, status: status),
           let declared = OAuthLifecycle.string(object, "error"), !declared.isEmpty {
            throw AuthError(.authorizationError, scheme: descriptor.name, status: status, code: declared)
        }
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: descriptor.name, status: status)
        }
    }
}

extension OAuthSession {
    /// Revokes the token value through the session's client, resolving the
    /// endpoint through the compiled precedence with the session's cached
    /// discovery document. See ``Client/revokeToken(scheme:tokenValue:)``.
    public func revokeToken(scheme: String, tokenValue: String) async throws {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let endpoint = try await resolveEndpoint(
            descriptor, compiled: descriptor.revocationURL, member: \.revocationEndpoint)
        try await client.revokeToken(descriptor, endpoint: endpoint, identity: identity, tokenValue: tokenValue)
    }
}
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries an
/// introspection endpoint.
const INTROSPECTION: &str = r#"
/// One RFC 7662 introspection response. Timestamps derive from the numeric
/// epoch fields; nil means the server omitted them. The response describes
/// the token without returning it.
public struct Introspection: Sendable, Equatable {
    public let active: Bool
    public let scope: String?
    public let clientID: String?
    public let tokenType: String?
    public let username: String?
    public let expiresAt: Date?
    public let issuedAt: Date?
    public let notBefore: Date?
    public let subject: String?
    public let audience: [String]
    public let issuer: String?
    public let jwtID: String?
}

extension OAuthLifecycle {
    static func epoch(_ object: JsonObject<JsonValue>, _ key: String) -> Date? {
        guard let seconds = integer(object, key), seconds > 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(seconds))
    }
}

extension Client {
    /// Queries the scheme's compiled introspection endpoint (RFC 7662) with
    /// the token value.
    public func introspectToken(scheme: String, tokenValue: String) async throws -> Introspection {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.introspectionURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let (status, body) = try await oauthEndpointRequest(
            descriptor.introspectionURL, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret, form: [("token", tokenValue)])
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: scheme, status: status)
        }
        let object = try OAuthLifecycle.decodeObject(body, scheme: scheme, status: status)
        var audience: [String] = []
        if case .array(let members)? = object["aud"] {
            audience = members.compactMap { member in
                if case .string(let value) = member { return value }
                return nil
            }
        }
        return Introspection(
            active: OAuthLifecycle.boolean(object, "active"),
            scope: OAuthLifecycle.string(object, "scope"),
            clientID: OAuthLifecycle.string(object, "client_id"),
            tokenType: OAuthLifecycle.string(object, "token_type"),
            username: OAuthLifecycle.string(object, "username"),
            expiresAt: OAuthLifecycle.epoch(object, "exp"),
            issuedAt: OAuthLifecycle.epoch(object, "iat"),
            notBefore: OAuthLifecycle.epoch(object, "nbf"),
            subject: OAuthLifecycle.string(object, "sub"),
            audience: audience,
            issuer: OAuthLifecycle.string(object, "iss"),
            jwtID: OAuthLifecycle.string(object, "jti"))
    }
}

extension OAuthSession {
    /// Introspects the token value through the session's client. See
    /// ``Client/introspectToken(scheme:tokenValue:)``.
    public func introspectToken(scheme: String, tokenValue: String) async throws -> Introspection {
        try await client.introspectToken(scheme: scheme, tokenValue: tokenValue)
    }
}
"#;

/// RFC 7662 introspection with the discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`
/// resolves the request.
const INTROSPECTION_DISCOVERY: &str = r#"
/// One RFC 7662 introspection response. Timestamps derive from the numeric
/// epoch fields; nil means the server omitted them. The response describes
/// the token without returning it.
public struct Introspection: Sendable, Equatable {
    public let active: Bool
    public let scope: String?
    public let clientID: String?
    public let tokenType: String?
    public let username: String?
    public let expiresAt: Date?
    public let issuedAt: Date?
    public let notBefore: Date?
    public let subject: String?
    public let audience: [String]
    public let issuer: String?
    public let jwtID: String?
}

extension OAuthLifecycle {
    static func epoch(_ object: JsonObject<JsonValue>, _ key: String) -> Date? {
        guard let seconds = integer(object, key), seconds > 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(seconds))
    }
}

extension Client {
    /// Introspects one token at the scheme's introspection endpoint (RFC 7662)
    /// with the token value.
    ///
    /// Endpoint resolution follows the compiled precedence: the configured
    /// introspection endpoint always wins; otherwise the discovery document's
    /// `introspection_endpoint` resolves the request (fetched per call by this
    /// one-shot helper, with the session's cached document used by
    /// ``OAuthSession``).
    public func introspectToken(scheme: String, tokenValue: String) async throws -> Introspection {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let endpoint = try await resolveEndpoint(
            descriptor, compiled: descriptor.introspectionURL, member: \.introspectionEndpoint)
        return try await introspectToken(descriptor, endpoint: endpoint, identity: identity, tokenValue: tokenValue)
    }

    /// Queries one resolved introspection endpoint with the token value.
    func introspectToken(_ descriptor: OAuthSchemeDescriptor, endpoint: String,
                         identity: (id: String, secret: String), tokenValue: String) async throws -> Introspection {
        let (status, body) = try await oauthEndpointRequest(
            endpoint, scheme: descriptor.name, clientID: identity.id,
            clientSecret: identity.secret, form: [("token", tokenValue)])
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: descriptor.name, status: status)
        }
        let object = try OAuthLifecycle.decodeObject(body, scheme: descriptor.name, status: status)
        var audience: [String] = []
        if case .array(let members)? = object["aud"] {
            audience = members.compactMap { member in
                if case .string(let value) = member { return value }
                return nil
            }
        }
        return Introspection(
            active: OAuthLifecycle.boolean(object, "active"),
            scope: OAuthLifecycle.string(object, "scope"),
            clientID: OAuthLifecycle.string(object, "client_id"),
            tokenType: OAuthLifecycle.string(object, "token_type"),
            username: OAuthLifecycle.string(object, "username"),
            expiresAt: OAuthLifecycle.epoch(object, "exp"),
            issuedAt: OAuthLifecycle.epoch(object, "iat"),
            notBefore: OAuthLifecycle.epoch(object, "nbf"),
            subject: OAuthLifecycle.string(object, "sub"),
            audience: audience,
            issuer: OAuthLifecycle.string(object, "iss"),
            jwtID: OAuthLifecycle.string(object, "jti"))
    }
}

extension OAuthSession {
    /// Introspects the token value through the session's client, resolving the
    /// endpoint through the compiled precedence with the session's cached
    /// discovery document. See ``Client/introspectToken(scheme:tokenValue:)``.
    public func introspectToken(scheme: String, tokenValue: String) async throws -> Introspection {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        let endpoint = try await resolveEndpoint(
            descriptor, compiled: descriptor.introspectionURL, member: \.introspectionEndpoint)
        return try await client.introspectToken(
            descriptor, endpoint: endpoint, identity: identity, tokenValue: tokenValue)
    }
}
"#;

/// Authorization code with PKCE S256, emitted only when a compiled scheme
/// carries an executable authorization-code flow.
const AUTHORIZATION_CODE: &str = r#"
/// One started authorization-code transaction with a bound PKCE verifier and
/// a one-time state. The state and verifier leave this process only through
/// `authorizationURL` and the code exchange.
public final class AuthorizationTransaction: @unchecked Sendable {
    /// The source security scheme name.
    public let scheme: String
    /// The complete authorization endpoint URL: client identity, redirect URI,
    /// scope, state and the S256 code challenge.
    public let authorizationURL: String
    /// Must match the callback's state parameter exactly; it is consumed by
    /// the first `completeAuthorization` call.
    public let state: String
    /// Marks transaction start; lifetime policy belongs to the authorization
    /// server.
    public let createdAt: Date
    public var description: String { "AuthorizationTransaction(scheme: \(scheme))" }

    private let lock = NSLock()
    private var consumed = false
    private let verifier: String
    private let tokenURL: String
    private let redirectURI: String

    init(scheme: String, authorizationURL: String, state: String, createdAt: Date,
         verifier: String, tokenURL: String, redirectURI: String) {
        self.scheme = scheme; self.authorizationURL = authorizationURL
        self.state = state; self.createdAt = createdAt
        self.verifier = verifier; self.tokenURL = tokenURL; self.redirectURI = redirectURI
    }

    /// Consumes the transaction exactly once, whatever the completion outcome:
    /// a second call is a typed state failure.
    func take() -> (verifier: String, tokenURL: String, redirectURI: String)? {
        lock.lock(); defer { lock.unlock() }
        guard !consumed else { return nil }
        consumed = true
        return (verifier, tokenURL, redirectURI)
    }
}

extension Client {
    /// Starts an authorization-code transaction with PKCE S256: it allocates
    /// the state and verifier, binds them to the returned transaction and
    /// renders the complete authorization URL. It performs no network call;
    /// direct the user to `authorizationURL` and complete the transaction with
    /// the callback parameters.
    public func beginAuthorization(scheme: String, redirectURI: String,
                                   clientID: String? = nil) throws -> AuthorizationTransaction {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.authorizationURL.isEmpty, !descriptor.codeTokenURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: nil)
        guard !identity.id.isEmpty else {
            throw AuthError(.requestValidation, scheme: scheme)
        }
        let verifier = try oauthRandomValue(scheme: scheme)
        let state = try oauthRandomValue(scheme: scheme)
        #if canImport(CryptoKit)
        let challenge = Data(SHA256.hash(data: Data(verifier.utf8))).oauthBase64URL()
        #else
        let challenge = ""
        #endif
        let fields: [(String, String)] = [
            ("response_type", "code"),
            ("client_id", identity.id),
            ("redirect_uri", redirectURI),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ]
        let query = fields.map { "\(OAuthLifecycle.escape($0.0))=\(OAuthLifecycle.escape($0.1))" }
            .joined(separator: "&")
        let separator = descriptor.authorizationURL.contains("?") ? "&" : "?"
        return AuthorizationTransaction(
            scheme: scheme,
            authorizationURL: descriptor.authorizationURL + separator + query,
            state: state, createdAt: Date(), verifier: verifier,
            tokenURL: descriptor.codeTokenURL, redirectURI: redirectURI)
    }

    /// Validates the callback's state against the transaction's bound state,
    /// exchanges the code with the retained PKCE verifier and returns the
    /// resulting token set. The transaction is consumed by the first call,
    /// whatever its outcome: a second call is a typed state failure.
    public func completeAuthorization(_ transaction: AuthorizationTransaction,
                                      callbackParameters: [String: String]) async throws -> TokenSet {
        guard let bound = transaction.take() else {
            throw AuthError(.stateConsumed, scheme: transaction.scheme)
        }
        guard callbackParameters["state"] == transaction.state else {
            throw AuthError(.stateMismatch, scheme: transaction.scheme)
        }
        if let declared = callbackParameters["error"], !declared.isEmpty {
            throw AuthError(.authorizationError, scheme: transaction.scheme, code: declared)
        }
        guard let code = callbackParameters["code"], !code.isEmpty else {
            throw AuthError(.invalidResponse, scheme: transaction.scheme)
        }
        let descriptor = try OAuthLifecycle.scheme(transaction.scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        var fields: [(String, String)] = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", bound.verifier),
        ]
        if !bound.redirectURI.isEmpty {
            fields.append(("redirect_uri", bound.redirectURI))
        }
        return try await oauthTokenRequest(
            bound.tokenURL, scheme: transaction.scheme, clientID: identity.id,
            clientSecret: identity.secret, form: fields, retainedRefresh: nil)
    }
}

extension OAuthSession {
    /// Starts an authorization-code transaction through the session's client.
    /// See ``Client/beginAuthorization(scheme:redirectURI:clientID:)``.
    public func beginAuthorization(scheme: String, redirectURI: String,
                                   clientID: String? = nil) throws -> AuthorizationTransaction {
        try client.beginAuthorization(scheme: scheme, redirectURI: redirectURI, clientID: clientID)
    }

    /// Completes an authorization-code transaction through the session's
    /// client. See ``Client/completeAuthorization(_:callbackParameters:)``.
    public func completeAuthorization(_ transaction: AuthorizationTransaction,
                                      callbackParameters: [String: String]) async throws -> TokenSet {
        try await client.completeAuthorization(transaction, callbackParameters: callbackParameters)
    }
}
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
/// One started RFC 8628 device-authorization transaction. Present `userCode`
/// and `verificationURI` to the user; ``token()`` polls the compiled token
/// endpoint for the granted set.
public struct DeviceAuthorization: Sendable {
    /// The source security scheme name.
    public let scheme: String
    /// The code the user enters at the verification URI.
    public let userCode: String
    /// Where the user approves the device grant.
    public let verificationURI: String
    /// When declared, carries the user code in the URL.
    public let verificationURIComplete: String?
    /// The server-declared transaction expiry; nil means the server declared
    /// no lifetime and only its own expired_token answer bounds polling.
    public let expiresAt: Date?
    /// The polling floor; the server's slow_down answers extend it.
    public let interval: TimeInterval

    let client: Client
    let deviceCode: String
    let tokenURL: String

    /// Polls the compiled token endpoint until the user approves the device
    /// grant, the transaction expires or the task is cancelled (RFC 8628
    /// section 3.5). The server's interval is honored; authorization_pending
    /// keeps polling and slow_down extends the interval by five seconds per
    /// answer. Each wait is a cancellable `Task.sleep`.
    public func token() async throws -> TokenSet {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        let identity = OAuthLifecycle.resolve(descriptor, clientID: nil, clientSecret: nil)
        var pause = min(max(interval, 0), 86_400)
        while true {
            if let expiresAt, expiresAt <= Date() {
                throw AuthError(.expired, scheme: scheme)
            }
            try await Task.sleep(nanoseconds: UInt64(pause * 1_000_000_000))
            do {
                return try await client.oauthTokenRequest(
                    tokenURL, scheme: scheme, clientID: identity.id, clientSecret: identity.secret,
                    form: [("grant_type", oauthDeviceGrant), ("device_code", deviceCode)],
                    retainedRefresh: nil)
            } catch let error as AuthError {
                switch error.code {
                case "authorization_pending":
                    // The user has not approved yet; keep polling.
                    continue
                case "slow_down":
                    pause += 5
                    continue
                default:
                    throw error
                }
            } catch {
                throw error
            }
        }
    }
}

extension Client {
    /// Starts a device-authorization transaction against the compiled device
    /// endpoint (RFC 8628 sections 3.1-3.2). It performs one network call and
    /// returns the user-facing code and verification URL.
    public func beginDeviceAuthorization(scheme: String, clientID: String? = nil) async throws -> DeviceAuthorization {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.deviceURL.isEmpty, !descriptor.deviceTokenURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: nil)
        guard !identity.id.isEmpty else {
            throw AuthError(.requestValidation, scheme: scheme)
        }
        let form: [(String, String)] = [("client_id", identity.id)]
        let (status, body) = try await oauthEndpointRequest(
            descriptor.deviceURL, scheme: scheme, clientID: identity.id,
            clientSecret: identity.secret, form: form)
        let object = try OAuthLifecycle.decodeObject(body, scheme: scheme, status: status)
        if let declared = OAuthLifecycle.string(object, "error"), !declared.isEmpty {
            throw AuthError(.authorizationError, scheme: scheme, status: status, code: declared)
        }
        guard (200..<300).contains(status) else {
            throw AuthError(.serverError, scheme: scheme, status: status)
        }
        guard let deviceCode = OAuthLifecycle.string(object, "device_code"), !deviceCode.isEmpty,
              let userCode = OAuthLifecycle.string(object, "user_code"), !userCode.isEmpty,
              let verification = OAuthLifecycle.string(object, "verification_uri"),
              !verification.isEmpty
        else {
            throw AuthError(.invalidResponse, scheme: scheme, status: status)
        }
        let seconds = OAuthLifecycle.integer(object, "interval") ?? 0
        var expiresAt: Date?
        if let lifetime = OAuthLifecycle.integer(object, "expires_in"), lifetime > 0 {
            expiresAt = Date().addingTimeInterval(TimeInterval(lifetime))
        }
        return DeviceAuthorization(
            scheme: scheme, userCode: userCode, verificationURI: verification,
            verificationURIComplete: OAuthLifecycle.string(object, "verification_uri_complete"),
            expiresAt: expiresAt, interval: seconds > 0 ? TimeInterval(seconds) : 5,
            client: self, deviceCode: deviceCode, tokenURL: descriptor.deviceTokenURL)
    }
}

extension OAuthSession {
    /// Starts a device-authorization transaction through the session's client.
    /// See ``Client/beginDeviceAuthorization(scheme:)``.
    public func beginDeviceAuthorization(scheme: String) async throws -> DeviceAuthorization {
        try await client.beginDeviceAuthorization(scheme: scheme)
    }
}
"#;

/// The plain-variant replaying credential: the coordinated-refresh rounds
/// actor, the wrapper class with its replaying transport, and the session
/// extension that serves the acquisition round at the compiled client
/// -credentials endpoints.
const REPLAY_PLAIN: &str = r#"
/// The replaying variant of one compiled scheme's client-credentials
/// credential: the plain provider's attach behavior plus the unified 401
/// replay policy. Wire it in two places — pass ``OAuthReplayCredentials/provider``
/// as the scheme's member of ``Credentials``, and pass the wrapper from
/// ``OAuthReplayCredentials/transport(_:)`` as the client's transport:
///
///     let replay = try session.replayingAuthorizationProvider("service")
///     let client = Client(credentials: Credentials(service: replay.provider),
///                         transport: replay.transport(inner))
///
/// A 401 (and only a 401) on a request whose Authorization value this
/// provider attached triggers exactly one coordinated refresh — concurrent
/// 401s share one token request round — and exactly one replay of the request
/// with the fresh token. The second response is surfaced whatever it is: a
/// second 401 reaches the caller as the declared error. The overall budget is
/// one refresh plus one replay, never nested with other retry policies
/// (requests are not retried today). Attaches for stream-protected
/// requirements are never replayed, because delivered stream data prevents a transparent restart.
/// A refresh failure surfaces as the typed `AuthError`
/// instead of a replay. The plain provider keeps today's semantics: replay is
/// this wrapper's opt-in only.
public final class OAuthReplayCredentials: @unchecked Sendable {
    /// One attach this provider served, remembered so the wrapped transport
    /// can tell which requests carried this provider's token. The record
    /// keeps only safe metadata; token values already traveled on the wire.
    private struct ServedAttach {
        let value: String
        let eligible: Bool
    }

    /// One replaying credential's coordinated refresh rounds: concurrent
    /// 401s that present the same stale token share exactly one acquisition
    /// round, and a failed round fails every waiter exactly once.
    private actor Rounds {
        private var rounds: [String: Task<String, any Error>] = [:]

        /// Joins the in-flight round for `key` or starts one: the round body
        /// re-reads the stored set (a newer stored set wins over a stale
        /// re-refresh), clears the stored set and forces one fresh
        /// acquisition.
        func refresh(key: String, round: @escaping @Sendable () async throws -> String) async throws -> String {
            if let running = rounds[key] { return try await running.value }
            let task = Task { try await round() }
            rounds[key] = task
            defer { rounds[key] = nil }
            return try await task.value
        }
    }

    private let session: OAuthSession
    private let scheme: String
    private let clientID: String?
    private let clientSecret: String?
    private let neverReplay: Set<String>
    private let lifecycleTargets: Set<String>
    private let rounds = Rounds()
    private let lock = NSLock()
    private var served: [ServedAttach] = []

    /// Prepares the replaying variant of the compiled client-credentials
    /// provider for one scheme. Every identity argument behaves exactly as in
    /// the plain provider; the replay semantics are strictly additive and the
    /// plain provider keeps today's attach-only semantics. Creation refuses a
    /// compiled scheme whose client-credentials flow declares no token URL,
    /// exactly like the plain provider's first attach.
    public init(session: OAuthSession, scheme: String, clientID: String? = nil,
                clientSecret: String? = nil) throws {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        self.session = session
        self.scheme = scheme
        self.clientID = clientID
        self.clientSecret = clientSecret
        self.neverReplay = OAuthReplayProtection.noReplay[scheme] ?? []
        self.lifecycleTargets = Set([descriptor.clientCredentialsURL, descriptor.refreshURL].filter { !$0.isEmpty })
    }

    /// The credential attach path: the plain provider's attach behavior plus
    /// the served-attach record the replaying transport matches against.
    /// Pass it as the scheme's member of ``Credentials``.
    public var provider: HTTPAuthorizationProvider {
        { [self] context in
            let credential = try await session.clientCredentialsToken(scheme: scheme,
                                                                     clientID: clientID,
                                                                     clientSecret: clientSecret)
            let value = credential.authorization
            record(value: value, eligible: !neverReplay.contains(context.source.pointer))
            return value
        }
    }

    /// Wraps `inner` with the one-refresh-one-replay 401 policy; call once per
    /// client. Token requests keep traveling through `inner` directly.
    public func transport(_ inner: any HTTPTransport) -> any HTTPTransport {
        Transport(replay: self, inner: inner)
    }

    private func record(value: String, eligible: Bool) {
        lock.lock(); defer { lock.unlock() }
        served.insert(ServedAttach(value: value, eligible: eligible), at: 0)
        if served.count > 8 { served.removeLast(served.count - 8) }
    }

    private func servedEntry(presented: String) -> ServedAttach? {
        lock.lock(); defer { lock.unlock() }
        return served.first { $0.value == presented && $0.eligible }
    }

    /// The fresh Authorization value for one qualifying 401, or nil when the
    /// response stays untouched: an unmatched or ineligible presented token,
    /// or a lifecycle-endpoint target (defense in depth against loops —
    /// lifecycle requests carry no bearer token of this provider).
    func freshAuthorization(request: HTTPRequest, presented: String) async throws -> String? {
        guard servedEntry(presented: presented) != nil else { return nil }
        guard !lifecycleTargets.contains(request.url.absoluteString) else { return nil }
        let key = try await session.replayKey(scheme: scheme, clientID: clientID, clientSecret: clientSecret)
        return try await rounds.refresh(key: key) { [session, scheme, clientID, clientSecret, key, presented] in
            try await session.replayRefreshRound(scheme: scheme, key: key, presented: presented,
                                                 clientID: clientID, clientSecret: clientSecret)
        }
    }

    /// The replaying transport: one coordinated refresh and, when the request
    /// carried this provider's token and no stream is protected on it,
    /// exactly one replay with the fresh token. Stream responses are never
    /// intercepted: their attaches are recorded ineligible, and delivered stream data prevents a transparent restart.
    private struct Transport: HTTPTransport {
        let replay: OAuthReplayCredentials
        let inner: any HTTPTransport

        func send(_ request: HTTPRequest) async throws -> HTTPResponse {
            let presented = request.headers.first { $0.name.lowercased() == "authorization" }?.value
            let response = try await inner.send(request)
            guard response.status == 401, let presented else { return response }
            guard let fresh = try await replay.freshAuthorization(request: request, presented: presented) else {
                return response
            }
            var headers = request.headers.filter { $0.name.lowercased() != "authorization" }
            headers.append(HTTPHeader("Authorization", fresh))
            let replayed = HTTPRequest(method: request.method, url: request.url, headers: headers,
                                       body: request.body, timeout: request.timeout,
                                       maxResponseBytes: request.maxResponseBytes)
            return try await inner.send(replayed)
        }

        func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
            try await inner.open(request, maxBufferedBytes: maxBufferedBytes)
        }
    }
}

extension OAuthSession {
    /// Builds the replaying variant of the scheme's client-credentials
    /// credential. See ``OAuthReplayCredentials`` for the one-refresh
    /// -one-replay contract and its two-place wiring.
    public nonisolated func replayingAuthorizationProvider(_ scheme: String, clientID: String? = nil,
                                                           clientSecret: String? = nil) throws -> OAuthReplayCredentials {
        try OAuthReplayCredentials(session: self, scheme: scheme, clientID: clientID, clientSecret: clientSecret)
    }

    /// The replaying credential's store key: exactly the key the plain attach
    /// path partitions, so a newer stored set wins over a stale re-refresh.
    func replayKey(scheme: String, clientID: String?, clientSecret: String?) async throws -> String {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        return OAuthLifecycle.storeKey(scheme: scheme, issuer: descriptor.tokenURL, clientID: identity.id)
    }

    /// One coordinated-refresh acquisition round: a newer stored set wins
    /// over a stale re-refresh; otherwise the stored set is cleared and one
    /// fresh acquisition runs through the session's own single-flighted path.
    /// A failed round fails every waiter exactly once.
    func replayRefreshRound(scheme: String, key: String, presented: String,
                            clientID: String?, clientSecret: String?) async throws -> String {
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        if let stored, stored.authorization != presented {
            return stored.authorization
        }
        do { try await store.clear(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        return try await clientCredentialsToken(scheme: scheme, clientID: clientID,
                                                clientSecret: clientSecret).authorization
    }
}
"#;

/// The discovery-variant replaying credential: the refresh endpoint and the
/// lifecycle-endpoint exclusion resolve through the compiled precedence (the
/// compiled token URL always wins; otherwise the session's cached discovery
/// document's token endpoint).
const REPLAY_DISCOVERY: &str = r#"
/// The replaying variant of one compiled scheme's client-credentials
/// credential: the plain provider's attach behavior plus the unified 401
/// replay policy. Wire it in two places — pass ``OAuthReplayCredentials/provider``
/// as the scheme's member of ``Credentials``, and pass the wrapper from
/// ``OAuthReplayCredentials/transport(_:)`` as the client's transport:
///
///     let replay = try session.replayingAuthorizationProvider("service")
///     let client = Client(credentials: Credentials(service: replay.provider),
///                         transport: replay.transport(inner))
///
/// A 401 (and only a 401) on a request whose Authorization value this
/// provider attached triggers exactly one coordinated refresh — concurrent
/// 401s share one token request round — and exactly one replay of the request
/// with the fresh token. The second response is surfaced whatever it is: a
/// second 401 reaches the caller as the declared error. The overall budget is
/// one refresh plus one replay, never nested with other retry policies
/// (requests are not retried today). Attaches for stream-protected
/// requirements are never replayed, because delivered stream data prevents a transparent restart.
/// A refresh failure surfaces as the typed `AuthError`
/// instead of a replay. The plain provider keeps today's semantics: replay is
/// this wrapper's opt-in only. The refresh endpoint resolves through the
/// compiled precedence: the compiled token URL when the client-credentials
/// flow compiles one, otherwise the discovery document's `token_endpoint`,
/// resolved against the session's cached document.
public final class OAuthReplayCredentials: @unchecked Sendable {
    /// One attach this provider served, remembered so the wrapped transport
    /// can tell which requests carried this provider's token. The record
    /// keeps only safe metadata; token values already traveled on the wire.
    private struct ServedAttach {
        let value: String
        let eligible: Bool
    }

    /// One replaying credential's coordinated refresh rounds: concurrent
    /// 401s that present the same stale token share exactly one acquisition
    /// round, and a failed round fails every waiter exactly once.
    private actor Rounds {
        private var rounds: [String: Task<String, any Error>] = [:]

        /// Joins the in-flight round for `key` or starts one: the round body
        /// re-reads the stored set (a newer stored set wins over a stale
        /// re-refresh), clears the stored set and forces one fresh
        /// acquisition.
        func refresh(key: String, round: @escaping @Sendable () async throws -> String) async throws -> String {
            if let running = rounds[key] { return try await running.value }
            let task = Task { try await round() }
            rounds[key] = task
            defer { rounds[key] = nil }
            return try await task.value
        }
    }

    private let session: OAuthSession
    private let scheme: String
    private let clientID: String?
    private let clientSecret: String?
    private let neverReplay: Set<String>
    private let lifecycleTargets: Set<String>
    private let rounds = Rounds()
    private let lock = NSLock()
    private var served: [ServedAttach] = []

    /// Prepares the replaying variant of the compiled client-credentials
    /// provider for one scheme. Every identity argument behaves exactly as in
    /// the plain provider; the replay semantics are strictly additive and the
    /// plain provider keeps today's attach-only semantics. Creation refuses a
    /// compiled scheme that declares neither a client-credentials token URL
    /// nor a discovery URL, exactly like the plain provider's first attach.
    public init(session: OAuthSession, scheme: String, clientID: String? = nil,
                clientSecret: String? = nil) throws {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty || !descriptor.discoveryURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        self.session = session
        self.scheme = scheme
        self.clientID = clientID
        self.clientSecret = clientSecret
        self.neverReplay = OAuthReplayProtection.noReplay[scheme] ?? []
        self.lifecycleTargets = Set([descriptor.clientCredentialsURL, descriptor.refreshURL,
                                     descriptor.discoveryURL].filter { !$0.isEmpty })
    }

    /// The credential attach path: the plain provider's attach behavior plus
    /// the served-attach record the replaying transport matches against.
    /// Pass it as the scheme's member of ``Credentials``.
    public var provider: HTTPAuthorizationProvider {
        { [self] context in
            let credential = try await session.clientCredentialsToken(scheme: scheme,
                                                                     clientID: clientID,
                                                                     clientSecret: clientSecret)
            let value = credential.authorization
            record(value: value, eligible: !neverReplay.contains(context.source.pointer))
            return value
        }
    }

    /// Wraps `inner` with the one-refresh-one-replay 401 policy; call once per
    /// client. Token requests keep traveling through `inner` directly.
    public func transport(_ inner: any HTTPTransport) -> any HTTPTransport {
        Transport(replay: self, inner: inner)
    }

    private func record(value: String, eligible: Bool) {
        lock.lock(); defer { lock.unlock() }
        served.insert(ServedAttach(value: value, eligible: eligible), at: 0)
        if served.count > 8 { served.removeLast(served.count - 8) }
    }

    private func servedEntry(presented: String) -> ServedAttach? {
        lock.lock(); defer { lock.unlock() }
        return served.first { $0.value == presented && $0.eligible }
    }

    /// The fresh Authorization value for one qualifying 401, or nil when the
    /// response stays untouched: an unmatched or ineligible presented token,
    /// or a lifecycle-endpoint target (defense in depth against loops —
    /// lifecycle requests carry no bearer token of this provider, and the
    /// discovery-resolved token endpoint rides the exact-token match).
    func freshAuthorization(request: HTTPRequest, presented: String) async throws -> String? {
        guard servedEntry(presented: presented) != nil else { return nil }
        guard !lifecycleTargets.contains(request.url.absoluteString) else { return nil }
        let key = try await session.replayKey(scheme: scheme, clientID: clientID, clientSecret: clientSecret)
        return try await rounds.refresh(key: key) { [session, scheme, clientID, clientSecret, key, presented] in
            try await session.replayRefreshRound(scheme: scheme, key: key, presented: presented,
                                                 clientID: clientID, clientSecret: clientSecret)
        }
    }

    /// The replaying transport: one coordinated refresh and, when the request
    /// carried this provider's token and no stream is protected on it,
    /// exactly one replay with the fresh token. Stream responses are never
    /// intercepted: their attaches are recorded ineligible, and delivered stream data prevents a transparent restart.
    private struct Transport: HTTPTransport {
        let replay: OAuthReplayCredentials
        let inner: any HTTPTransport

        func send(_ request: HTTPRequest) async throws -> HTTPResponse {
            let presented = request.headers.first { $0.name.lowercased() == "authorization" }?.value
            let response = try await inner.send(request)
            guard response.status == 401, let presented else { return response }
            guard let fresh = try await replay.freshAuthorization(request: request, presented: presented) else {
                return response
            }
            var headers = request.headers.filter { $0.name.lowercased() != "authorization" }
            headers.append(HTTPHeader("Authorization", fresh))
            let replayed = HTTPRequest(method: request.method, url: request.url, headers: headers,
                                       body: request.body, timeout: request.timeout,
                                       maxResponseBytes: request.maxResponseBytes)
            return try await inner.send(replayed)
        }

        func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
            try await inner.open(request, maxBufferedBytes: maxBufferedBytes)
        }
    }
}

extension OAuthSession {
    /// Builds the replaying variant of the scheme's client-credentials
    /// credential. See ``OAuthReplayCredentials`` for the one-refresh
    /// -one-replay contract and its two-place wiring.
    public nonisolated func replayingAuthorizationProvider(_ scheme: String, clientID: String? = nil,
                                                           clientSecret: String? = nil) throws -> OAuthReplayCredentials {
        try OAuthReplayCredentials(session: self, scheme: scheme, clientID: clientID, clientSecret: clientSecret)
    }

    /// The replaying credential's store key: exactly the key the plain attach
    /// path partitions — the compiled client-credentials token URL wins;
    /// otherwise the session's cached discovery document resolves the token
    /// endpoint — so a newer stored set wins over a stale re-refresh.
    func replayKey(scheme: String, clientID: String?, clientSecret: String?) async throws -> String {
        let descriptor = try OAuthLifecycle.scheme(scheme)
        guard !descriptor.clientCredentialsURL.isEmpty || !descriptor.discoveryURL.isEmpty else {
            throw AuthError(.unsupportedFlow, scheme: scheme)
        }
        let identity = OAuthLifecycle.resolve(descriptor, clientID: clientID, clientSecret: clientSecret)
        let resolved = try await clientCredentialsEndpoint(descriptor)
        return OAuthLifecycle.storeKey(scheme: scheme, issuer: resolved.issuer, clientID: identity.id)
    }

    /// One coordinated-refresh acquisition round: a newer stored set wins
    /// over a stale re-refresh; otherwise the stored set is cleared and one
    /// fresh acquisition runs through the session's own single-flighted path.
    /// A failed round fails every waiter exactly once.
    func replayRefreshRound(scheme: String, key: String, presented: String,
                            clientID: String?, clientSecret: String?) async throws -> String {
        let stored: TokenSet?
        do { stored = try await store.load(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        if let stored, stored.authorization != presented {
            return stored.authorization
        }
        do { try await store.clear(key: key) }
        catch { throw AuthError(.tokenStore, scheme: scheme, cause: error) }
        return try await clientCredentialsToken(scheme: scheme, clientID: clientID,
                                                clientSecret: clientSecret).authorization
    }
}
"#;
