//! Emitted-only OAuth 2.0 / OpenID Connect token lifecycle for the Java HTTP
//! SDK.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when client defaults are configured) compiles into one generated
//! `OAuth.java`: compiled per-scheme descriptors as constants, an immutable
//! `TokenSet`, the caller-implementable `TokenStore` interface with a
//! synchronized instance-owned `MemoryTokenStore`, client-credentials
//! acquisition with skew-aware caching and per-instance single-flight, explicit
//! refresh, and — only when the compiled schemes carry them — authorization
//! code with PKCE S256, RFC 8628 device polling, RFC 7009 revocation and RFC
//! 7662 introspection. Implicit and password flows are represented by the plan
//! but never executed, so schemes with only those flows emit nothing.
//!
//! When at least one compiled scheme carries a discovery URL (OpenID Connect
//! schemes, whose flows a discovery document defines at runtime, included), the
//! discovery engine emits alongside the compiled descriptors: the typed RFC
//! 8414 / OpenID Connect decode with the documented issuer-origin rule, a
//! per-instance cache keyed by scheme with per-scheme single-flight gates, and
//! endpoint resolution following the compiled precedence (an explicit compiled
//! endpoint always wins, then the cached discovery document, then the typed
//! refusal the compiled plan alone would produce). A discovery URL makes a
//! scheme usable even with no declared flows. Emission is byte-identical for
//! plans without a discovery URL.
//!
//! Emission is strictly conditional: without a configured `sdk_defaults`
//! policy, or without any usable scheme, the backend emits no new file and
//! every other artifact stays byte-identical. The runtime never parses OpenAPI;
//! client identity comes from explicit arguments or the compiled environment
//! variable names, read at call time, and token or secret values never enter
//! exception messages or `toString` output. Token and discovery requests ride
//! an injectable `java.net.http.HttpClient`, so caller transport policy covers
//! the lifecycle too.

use super::models::q;
use super::{http::JavaOperation, SdkPlan};
use crate::http_protocol::{
    CredentialHook, OAuthClientAuth, OAuthFlowDescriptor, OAuthFlowDescriptorKind, OAuthPlan,
    OAuthSchemeKind, OAuthSchemePlan, Representation,
};
use std::collections::{BTreeMap, BTreeSet};

/// Top-level type names `OAuth.java` owns in the package. Reserved against
/// model symbols only while the file is emitted, so no-policy allocation
/// behavior is unchanged; every other emitted type is nested inside `OAuth`.
pub(crate) const TOP_LEVEL_NAMES: &[&str] = &["OAuth"];

/// Whether one compiled flow can be executed by the emitted runtime.
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

/// Whether one compiled scheme contributes anything executable. A discovery
/// URL makes a scheme usable even with no declared flows: OpenID Connect
/// schemes have their endpoints defined by the discovery document at runtime.
fn usable_scheme(scheme: &OAuthSchemePlan) -> bool {
    scheme.discovery.is_some() || scheme.flows.iter().any(executable)
}

/// Whether the compiled plan justifies emitting `OAuth.java` at all.
pub(crate) fn emittable(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(usable_scheme)
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so plans without one compile
/// exactly the pre-replay bytes.
fn replaying(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        scheme
            .flows
            .iter()
            .any(|flow| executable(flow) && flow.kind == OAuthFlowDescriptorKind::ClientCredentials)
    })
}

/// The security-requirement source addresses whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget. Java credential hooks receive the address as
/// the requirement's document URI plus RFC 6901 pointer, exactly the value
/// the runtime's `Protocol.source` serves.
fn no_replay_requirements(
    oauth: &OAuthPlan,
    operations: &[JavaOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = oauth.schemes.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire.responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire.security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if names.contains(requirement.name()) {
                    let source = requirement.source().source();
                    let address =
                        format!("{}#{}", source.document().as_str(), source.pointer());
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(address);
                }
            }
        }
    }
    pointers
}

fn flow_kind_name(kind: OAuthFlowDescriptorKind) -> &'static str {
    match kind {
        OAuthFlowDescriptorKind::Implicit => "implicit",
        OAuthFlowDescriptorKind::Password => "password",
        OAuthFlowDescriptorKind::ClientCredentials => "client-credentials",
        OAuthFlowDescriptorKind::AuthorizationCode => "authorization-code",
        OAuthFlowDescriptorKind::DeviceAuthorization => "device-authorization",
    }
}

fn client_auth_name(client_auth: OAuthClientAuth) -> &'static str {
    match client_auth {
        OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
        OAuthClientAuth::None => "none",
    }
}

fn scheme_kind_name(kind: OAuthSchemeKind) -> &'static str {
    match kind {
        OAuthSchemeKind::OAuth2 => "oauth2",
        OAuthSchemeKind::OpenIdConnect => "open-id-connect",
    }
}

/// One compiled flow as the arguments of a `Flow` record constant.
fn flow_arguments(flow: &OAuthFlowDescriptor) -> String {
    let url = |value: &Option<String>| match value {
        Some(url) => q(url),
        None => "null".to_owned(),
    };
    let scopes = flow
        .scopes
        .iter()
        .flat_map(|(name, description)| [q(name), q(description)])
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "new Flow({}, {}, {}, {}, {}, {}, {}, scopes({}))",
        q(flow_kind_name(flow.kind)),
        url(&flow.authorization_url),
        url(&flow.token_url),
        url(&flow.refresh_url),
        url(&flow.device_authorization_url),
        q(client_auth_name(flow.client_auth)),
        flow.deprecated_flow,
        scopes,
    )
}

/// One compiled scheme as the arguments of a `Scheme` record constant. The
/// discovery-aware variant carries the compiled discovery URL; the plain
/// variant assembles byte-identically to the pre-discovery emission.
fn scheme_arguments(scheme: &OAuthSchemePlan, discovery: bool) -> String {
    let optional = |value: &Option<String>| match value {
        Some(value) => q(value),
        None => "null".to_owned(),
    };
    let flows = scheme
        .flows
        .iter()
        .map(flow_arguments)
        .collect::<Vec<_>>()
        .join(", ");
    if discovery {
        format!(
            "new Scheme({}, {}, {}, {}, {}, {}, {}, {}, flows({}))",
            q(&scheme.name),
            q(scheme_kind_name(scheme.kind)),
            scheme.refresh_skew_seconds,
            optional(&scheme.discovery),
            optional(&scheme.client_id_env),
            optional(&scheme.client_secret_env),
            optional(&scheme.revocation_endpoint),
            optional(&scheme.introspection_endpoint),
            flows,
        )
    } else {
        format!(
            "new Scheme({}, {}, {}, {}, {}, {}, {}, flows({}))",
            q(&scheme.name),
            q(scheme_kind_name(scheme.kind)),
            scheme.refresh_skew_seconds,
            optional(&scheme.client_id_env),
            optional(&scheme.client_secret_env),
            optional(&scheme.revocation_endpoint),
            optional(&scheme.introspection_endpoint),
            flows,
        )
    }
}

fn has_authorization_code(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            executable(flow) && flow.kind == OAuthFlowDescriptorKind::AuthorizationCode
        })
    })
}

fn has_device(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            executable(flow) && flow.kind == OAuthFlowDescriptorKind::DeviceAuthorization
        })
    })
}

fn has_revocation(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| scheme.revocation_endpoint.is_some())
}

fn has_introspection(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| scheme.introspection_endpoint.is_some())
}

/// The complete `OAuth.java` source for a plan with usable schemes, or `None`
/// when nothing is executable. Plans without a discovery URL assemble
/// byte-identically to the pre-discovery emission; plans with one emit the
/// discovery-aware credential methods and the discovery engine.
pub(crate) fn source(plan: &SdkPlan) -> Option<String> {
    let oauth = plan.oauth()?;
    let schemes: Vec<&OAuthSchemePlan> = oauth
        .schemes
        .iter()
        .filter(|scheme| usable_scheme(scheme))
        .collect();
    if schemes.is_empty() {
        return None;
    }
    let authorization_code = has_authorization_code(&schemes);
    let device = has_device(&schemes);
    let discovery = schemes.iter().any(|scheme| scheme.discovery.is_some());
    // The replaying credential wrapper joins only when a compiled scheme
    // carries an executable client-credentials flow, and compiles the
    // stream-protected requirement addresses of that plan's operations.
    let replay = replaying(oauth);
    let no_replay = if replay {
        no_replay_requirements(oauth, plan.operations())
    } else {
        BTreeMap::new()
    };
    let table = schemes
        .iter()
        .map(|scheme| {
            format!(
                "\n        schemes.put({}, {});",
                q(&scheme.name),
                scheme_arguments(scheme, discovery)
            )
        })
        .collect::<String>();
    let mut out = String::new();
    out.push_str(IMPORTS);
    // The replaying credential wrapper's imports join the import block only
    // when the wrapper participates, so non-replaying plans stay
    // byte-identical. Every import still precedes the emitted types.
    if replay {
        out.push_str(REPLAY_IMPORTS);
    }
    out.push_str("/**\n");
    for line in header_paragraph(discovery) {
        out.push_str(&format!(" * {line}\n"));
    }
    out.push_str(" */\npublic final class OAuth {\n\n");
    out.push_str(&format!(
        "    /** Compiled flow descriptor: exactly what the source declared. */\n    private record Flow(String kind, String authorizationUrl, String tokenUrl, String refreshUrl,\n            String deviceAuthorizationUrl, String clientAuth, boolean deprecated, Map<String, String> scopes) {{}}\n\n    /** Compiled scheme descriptor: exactly what the source declared plus the explicitly configured supplements{}. */\n    private record Scheme({}) {{}}\n\n    private static Map<String, String> scopes(String... pairs) {{\n        Map<String, String> scopes = new LinkedHashMap<>();\n        for (int at = 0; at + 1 < pairs.length; at += 2) {{ scopes.put(pairs[at], pairs[at + 1]); }}\n        return Map.copyOf(scopes);\n    }}\n\n    private static Map<String, Flow> flows(Flow... entries) {{\n        Map<String, Flow> flows = new LinkedHashMap<>();\n        for (Flow entry : entries) {{ flows.put(entry.kind(), entry); }}\n        return Map.copyOf(flows);\n    }}\n\n    /** Compiled scheme descriptors: generation-time constants, never parsed source documents. */\n    private static final Map<String, Scheme> SCHEMES;\n    static {{\n        Map<String, Scheme> schemes = new LinkedHashMap<>();{table}\n        SCHEMES = Collections.unmodifiableMap(schemes);\n    }}\n\n",
        if discovery {
            "; a compiled discovery URL resolves the endpoint URLs the compiled flows omit at call time"
        } else {
            ""
        },
        if discovery {
            "String name, String kind, int skew, String discovery, String clientIdEnv, String clientSecretEnv,\n            String revocation, String introspection, Map<String, Flow> flows"
        } else {
            "String name, String kind, int skew, String clientIdEnv, String clientSecretEnv,\n            String revocation, String introspection, Map<String, Flow> flows"
        },
        table = table,
    ));
    out.push_str(TYPES);
    if authorization_code {
        out.push_str(TRANSACTION);
    }
    if device {
        out.push_str(GRANT);
    }
    out.push_str(CORE);
    out.push_str(if discovery {
        CREDENTIALS_DISCOVERY
    } else {
        CREDENTIALS
    });
    out.push_str("\n    // Conditional lifecycle helpers: exactly the flows and configured\n    // supplemental endpoints compiled above participate.\n\n");
    if authorization_code {
        out.push_str(AUTHORIZATION_CODE);
    }
    if device {
        out.push_str(DEVICE);
    }
    if has_revocation(&schemes) {
        out.push_str(if discovery {
            REVOKE_DISCOVERY
        } else {
            REVOCATION
        });
    }
    if has_introspection(&schemes) {
        out.push_str(if discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION
        });
    }
    if discovery {
        out.push_str(DISCOVERY);
    }
    // The replaying credential wrapper: opt-in, so plans without an
    // executable client-credentials flow assemble exactly the pre-replay
    // bytes.
    if replay {
        out.push_str(&replay_section(oauth, &no_replay, discovery));
    }
    out.push_str("}\n");
    Some(out)
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow; its plain
/// and discovery variants resolve the wrapped token endpoint through the
/// same compiled precedence as the provider they wrap.
fn replay_section(
    oauth: &OAuthPlan,
    no_replay: &BTreeMap<String, BTreeSet<String>>,
    discovery: bool,
) -> String {
    let mut code = String::from(
        "\n    // The replaying credential wrapper: opt-in per provider, one\n    // coordinated refresh plus one eligible replay per qualifying 401, and\n    // never for stream-protected operations.\n\n    /** Compiled stream-protected requirements: security-requirement source addresses whose attaches are never replayed, because delivered stream data prevents a transparent restart. Each address is the requirement's document URI plus RFC 6901 pointer, exactly the value credential hooks receive. */\n    private static final Map<String, Set<String>> REPLAY_NO_REPLAY = ",
    );
    let entries = oauth
        .schemes
        .iter()
        .filter_map(|scheme| {
            let pointers = no_replay.get(&scheme.name)?;
            if pointers.is_empty() {
                return None;
            }
            let rendered = pointers
                .iter()
                .map(|pointer| q(pointer))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("Map.entry({}, Set.of({}))", q(&scheme.name), rendered))
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        code.push_str("Map.<String, Set<String>>ofEntries()");
    } else {
        code.push_str("Map.ofEntries(");
        code.push_str(&entries.join(", "));
        code.push(')');
    }
    code.push_str(";\n\n");
    code.push_str(REPLAY_FACTORIES);
    code.push_str(if discovery {
        REPLAY_CREDENTIALS_DISCOVERY
    } else {
        REPLAY_CREDENTIALS_PLAIN
    });
    code.push_str(REPLAY_TRANSPORT);
    code
}

/// The class documentation's opening paragraph: byte-exact for plans without
/// a discovery URL, discovery-aware otherwise.
fn header_paragraph(discovery: bool) -> Vec<&'static str> {
    let mut lines = vec![
        "Generated OAuth 2.0 lifecycle for this package's compiled OAuth schemes.",
        "",
        "Every endpoint, client-authentication style, scope set and policy constant in",
        "{@code SCHEMES} is a generation-time compilation of the used security schemes in",
        "the source document plus the explicitly configured supplements. This class never",
    ];
    if discovery {
        lines.extend([
            "parses OpenAPI and never invents an endpoint; endpoint URLs that the compiled",
            "flows omit resolve through RFC 8414 / OpenID Connect discovery when the scheme",
            "compiles a discovery URL.",
        ]);
    } else {
        lines.push("parses OpenAPI, never fetches discovery documents and never invents an endpoint.");
    }
    lines.extend([
        "The deprecated implicit and password flows stay described in the compiled",
        "descriptors but are never executed.",
        "",
        "Token requests are form-encoded (RFC 6749) over an injectable",
        "{@link java.net.http.HttpClient}, so caller transport policy covers the lifecycle",
        "too. Client identity resolves from explicit arguments first and otherwise from",
        "the compiled environment variable names, read at call time; credential values",
        "are never baked into generated bytes and never appear in exception messages or",
        "{@code toString()} output. Token sets live only in the token store owned by the",
        "{@code OAuth} instance (or a store explicitly supplied by the caller), under keys",
        "partitioned by scheme, token-endpoint issuer and client identity; there is no",
        "process-global token cache.",
    ]);
    lines
}

/// The emitted file's import block, following the package header.
const IMPORTS: &str = r#"import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.Collections;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

"#;

/// The always-emitted value types: token set, store interface, memory store
/// and the typed failure.
const TYPES: &str = r#"    /** One acquired token set. Values never appear in messages or {@code toString()} output. */
    public static final class TokenSet {
        /** Epoch millisecond at which the access token expires; {@code null} never expires. Freshness checks apply the compiled skew. */
        public final String accessToken, tokenType, refreshToken, scope;
        public final Long expiresAt;

        public TokenSet(String accessToken) { this(accessToken, "Bearer", null, null, null); }

        public TokenSet(String accessToken, String tokenType, Long expiresAt, String refreshToken, String scope) {
            Objects.requireNonNull(accessToken);
            if (accessToken.isEmpty() || accessToken.length() > 8192 || control(accessToken)) throw new IllegalArgumentException("TokenSet requires a nonempty access token without control characters");
            this.accessToken = accessToken;
            this.tokenType = tokenType == null ? "Bearer" : tokenType;
            this.expiresAt = expiresAt;
            this.refreshToken = refreshToken;
            this.scope = scope;
        }

        /** Whether the set outlives {@code now} by more than the compiled skew. */
        public boolean fresh(long now, int skewSeconds) { return expiresAt == null || expiresAt - skewSeconds * 1000L > now; }

        @Override public String toString() { return "TokenSet"; }
    }

    /** Caller-implementable token persistence. Keys partition stored token sets by scheme, token-endpoint issuer and client identity; values are whole token sets replaced atomically. */
    public interface TokenStore {
        /** Return the stored token set for {@code key}, or {@code null}. */
        TokenSet load(String key);

        /** Atomically replace the stored token set for {@code key}. */
        void replace(String key, TokenSet tokenSet);

        /** Drop the stored token set for {@code key}; clearing an absent key succeeds. */
        void clear(String key);
    }

    /** In-process token store owned by the {@code OAuth} instance or caller that created it; nothing keeps a global store. */
    public static final class MemoryTokenStore implements TokenStore {
        private final Map<String, TokenSet> tokens = new HashMap<>();

        @Override public synchronized TokenSet load(String key) { return tokens.get(key); }
        @Override public synchronized void replace(String key, TokenSet tokenSet) { tokens.put(key, tokenSet); }
        @Override public synchronized void clear(String key) { tokens.remove(key); }
    }

    /** Typed OAuth lifecycle failure. Messages and fields carry only safe metadata: kind, scheme, status, the server's machine error code and a retry hint; never token or client-secret values. */
    public static final class AuthException extends RuntimeException {
        private static final long serialVersionUID = 1L;
        public final String kind, scheme, serverError;
        public final Integer status, retryAfterSeconds;

        public AuthException(String kind, String scheme, String message) { this(kind, scheme, message, null, null, null); }

        public AuthException(String kind, String scheme, String message, Integer status, String serverError, Integer retryAfterSeconds) {
            super(message);
            this.kind = Objects.requireNonNull(kind);
            this.scheme = Objects.requireNonNull(scheme);
            this.status = status;
            this.serverError = serverError;
            this.retryAfterSeconds = retryAfterSeconds;
        }
    }

"#;

/// One bound authorization-code transaction.
const TRANSACTION: &str = r#"    /** One bound authorization-code transaction: session-scoped and consumed exactly once by {@link #completeAuthorization}, whether the exchange succeeds or fails. */
    public static final class AuthorizationTransaction {
        /** The exact redirect target, carrying response type, client id, redirect URI, state and the PKCE S256 challenge. */
        public final String scheme, authorizationUrl, state, codeVerifier, codeChallenge, redirectUri, tokenUrl;
        public final long createdAt;

        public AuthorizationTransaction(String scheme, String authorizationUrl, String state, String codeVerifier,
                String codeChallenge, String redirectUri, String tokenUrl, long createdAt) {
            this.scheme = scheme;
            this.authorizationUrl = authorizationUrl;
            this.state = state;
            this.codeVerifier = codeVerifier;
            this.codeChallenge = codeChallenge;
            this.redirectUri = redirectUri;
            this.tokenUrl = tokenUrl;
            this.createdAt = createdAt;
        }

        @Override public String toString() { return "AuthorizationTransaction[" + scheme + "]"; }
    }

"#;

/// One device grant value.
const GRANT: &str = r#"    /** One device-authorization grant from the declared endpoint (RFC 8628). The device code never appears in {@code toString()} output. */
    public static final class DeviceGrant {
        public final String scheme, deviceCode, userCode, verificationUri, verificationUriComplete;
        public final Long expiresAt;
        public final long intervalSeconds;

        public DeviceGrant(String scheme, String deviceCode, String userCode, String verificationUri,
                String verificationUriComplete, Long expiresAt, long intervalSeconds) {
            this.scheme = scheme;
            this.deviceCode = deviceCode;
            this.userCode = userCode;
            this.verificationUri = verificationUri;
            this.verificationUriComplete = verificationUriComplete;
            this.expiresAt = expiresAt;
            this.intervalSeconds = intervalSeconds;
        }

        @Override public String toString() { return "DeviceGrant[" + scheme + "]"; }
    }

"#;

/// The always-emitted OAuth class plumbing: descriptor lookup, request
/// plumbing and client-authentication helpers.
const CORE: &str = r#"    private final TokenStore store;
    private final HttpClient http;
    private final java.util.function.LongSupplier clock;
    /** Millisecond sleeper, injectable for tests; device polling and single-flight waits use it. */
    private final java.util.function.LongConsumer sleeper;
    private final java.time.Duration timeout;
    /** Per-instance single-flight gates, keyed by exact store key. */
    private final Map<String, Object> gates = new ConcurrentHashMap<>();

    public OAuth() { this(null, null, null, null); }

    /** @param store instance-owned by default; pass an explicit store to share it deliberately */
    public OAuth(TokenStore store) { this(store, null, null, null); }

    /** @param store instance-owned by default; pass an explicit store to share it deliberately
     * @param http injectable HTTP transport for token endpoint requests
     */
    public OAuth(TokenStore store, HttpClient http) { this(store, http, null, null); }

    /** @param store instance-owned by default; pass an explicit store to share it deliberately
     * @param http injectable HTTP transport for token endpoint requests
     * @param clock epoch-millisecond clock; injectable for tests
     * @param sleeper millisecond sleeper; injectable for tests
     */
    public OAuth(TokenStore store, HttpClient http, java.util.function.LongSupplier clock, java.util.function.LongConsumer sleeper) {
        this.store = store == null ? new MemoryTokenStore() : store;
        HttpClient client = http;
        if (client == null) {
            client = HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NEVER)
                .connectTimeout(java.time.Duration.ofSeconds(10)).build();
        }
        this.http = client;
        this.clock = clock == null ? System::currentTimeMillis : clock;
        this.sleeper = sleeper == null ? OAuth::defaultSleep : sleeper;
        this.timeout = java.time.Duration.ofSeconds(30);
    }

    private static void defaultSleep(long milliseconds) {
        try {
            Thread.sleep(milliseconds);
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            throw new AuthException("aborted", "", "the lifecycle wait was interrupted");
        }
    }

    private Scheme scheme(String name) {
        Scheme scheme = SCHEMES.get(name);
        if (scheme == null) throw new AuthException("unknown-scheme", name, "no compiled OAuth scheme carries that name; OAuth compiles exactly the source-declared schemes with executable flows");
        return scheme;
    }

    private Flow executableFlow(Scheme scheme, String kind) {
        Flow flow = scheme.flows().get(kind);
        if (flow == null || flow.deprecated()) throw new AuthException("unsupported-flow", scheme.name(), "scheme " + scheme.name() + " has no executable " + kind + " flow in its source declaration");
        return flow;
    }

    /** The refresh flow: the first executable flow carrying a declared refresh URL, else the first executable flow carrying a token URL. */
    private Flow refreshFlow(Scheme scheme) {
        for (String field : List.of("refreshUrl", "tokenUrl")) {
            for (String kind : List.of("authorization-code", "client-credentials", "device-authorization")) {
                Flow flow = scheme.flows().get(kind);
                if (flow == null || flow.deprecated()) continue;
                String url = "refreshUrl".equals(field) ? flow.refreshUrl() : flow.tokenUrl();
                if (url != null) return flow;
            }
        }
        throw new AuthException("endpoint-unavailable", scheme.name(), "the compiled scheme carries no executable flow with a refresh or token URL");
    }

    /** The endpoint that serves refreshes: the flow's declared refresh URL, else its token URL. */
    private String refreshEndpoint(Flow flow, Scheme scheme) {
        String endpoint = flow.refreshUrl() != null ? flow.refreshUrl() : flow.tokenUrl();
        if (endpoint == null) throw new AuthException("endpoint-unavailable", scheme.name(), "the compiled flow declares neither a refresh URL nor a token URL");
        return endpoint;
    }

    /** Reads one compiled environment variable name; values are read at request time, never at generation time. */
    private static String environment(String variable) {
        if (variable == null) return null;
        try {
            String value = System.getenv(variable);
            return value == null || value.isEmpty() ? null : value;
        } catch (RuntimeException error) {
            return null;
        }
    }

    /** Explicit arguments win; compiled environment variable names resolve at call time. */
    private String[] identity(Scheme scheme, String clientId, String clientSecret) {
        return new String[] {
            clientId != null ? clientId : environment(scheme.clientIdEnv()),
            clientSecret != null ? clientSecret : environment(scheme.clientSecretEnv()),
        };
    }

    /** RFC 6749 2.3.1 Basic credentials for confidential clients; the public profile never sends a secret. */
    private String basicAuth(Flow flow, String scheme, String[] identity) {
        if (!"client-secret-basic".equals(flow.clientAuth())) return null;
        if (identity[0] == null || identity[0].isEmpty() || identity[1] == null || identity[1].isEmpty()) {
            throw new AuthException("missing-client-credentials", scheme, "the compiled client authentication is client-secret-basic and no complete client identity is available");
        }
        String pair = urlEncode(identity[0]) + ":" + urlEncode(identity[1]);
        return Base64.getEncoder().encodeToString(pair.getBytes(StandardCharsets.US_ASCII));
    }

    /** The exact token-store key for one scheme, token-endpoint issuer and client identity. */
    private static String storeKey(String scheme, String issuer, String clientId) {
        return scheme + "|" + issuer + "|" + (clientId == null ? "public" : clientId);
    }

    private static boolean control(String value) {
        for (int at = 0; at < value.length(); at++) {
            char character = value.charAt(at);
            if (character < 0x20 || character == 0x7f) return true;
        }
        return false;
    }

    private static String urlEncode(String value) {
        return URLEncoder.encode(value, StandardCharsets.UTF_8).replace("+", "%20");
    }

    /** RFC 7636 base64url: unpadded URL-safe base64 over exact bytes. */
    private static String base64Url(byte[] bytes) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
    }

    /** Maps one rejected endpoint response to the typed error; messages carry only safe metadata. */
    private static AuthException serverFailure(String scheme, String serverError, int status, Integer retryAfterSeconds) {
        String kind = switch (serverError == null ? "" : serverError) {
            case "invalid_request" -> "invalid-request";
            case "invalid_client" -> "invalid-client";
            case "invalid_grant" -> "invalid-grant";
            case "unauthorized_client" -> "unauthorized-client";
            case "unsupported_grant_type" -> "unsupported-grant-type";
            case "invalid_scope" -> "invalid-scope";
            case "authorization_pending" -> "authorization-pending";
            case "slow_down" -> "slow-down";
            case "expired_token" -> "device-code-expired";
            default -> "server-error";
        };
        String label = serverError == null ? "HTTP " + status : serverError + ", HTTP " + status;
        return new AuthException(kind, scheme, "the authorization server rejected the request (" + label + ")", status, serverError, retryAfterSeconds);
    }

    /** One bounded, form-encoded endpoint POST with the compiled client authentication; a non-2xx response becomes the typed error. */
    private JsonRuntime.JsonObject postForm(String scheme, String endpoint, Flow flow, String[] identity, Map<String, String> fields) {
        Map<String, String> sent = new LinkedHashMap<>(fields);
        if (!"client-secret-basic".equals(flow.clientAuth()) && identity[0] != null) sent.put("client_id", identity[0]);
        StringBuilder form = new StringBuilder();
        for (Map.Entry<String, String> entry : sent.entrySet()) {
            if (entry.getValue() == null) continue;
            if (form.length() > 0) form.append('&');
            form.append(urlEncode(entry.getKey())).append('=').append(urlEncode(entry.getValue()));
        }
        HttpRequest.Builder builder = HttpRequest.newBuilder(URI.create(endpoint)).timeout(timeout)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(form.toString(), StandardCharsets.UTF_8));
        String basic = basicAuth(flow, scheme, identity);
        if (basic != null) builder.header("authorization", "Basic " + basic);
        HttpResponse<byte[]> response;
        try {
            response = http.send(builder.build(), HttpResponse.BodyHandlers.ofByteArray());
        } catch (java.io.IOException error) {
            throw new AuthException("transport-failure", scheme, "the endpoint request failed before a response arrived");
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            throw new AuthException("aborted", scheme, "the endpoint request was interrupted");
        }
        if (response.statusCode() < 200 || response.statusCode() > 299) {
            Integer retry = null;
            String raw = response.headers().firstValue("retry-after").orElse(null);
            if (raw != null && raw.trim().matches("[0-9]+")) retry = Integer.valueOf(raw.trim());
            String serverError = null;
            try {
                JsonRuntime.JsonValue decoded = parse(new String(response.body(), StandardCharsets.UTF_8));
                if (decoded instanceof JsonRuntime.JsonObject object) {
                    JsonRuntime.JsonValue error = object.values().get("error");
                    if (error instanceof JsonRuntime.JsonString message && !message.value().isEmpty()) serverError = message.value();
                }
            } catch (JsonRuntime.JsonError ignored) {
                // An unreadable error body is safe metadata loss.
            }
            throw serverFailure(scheme, serverError, response.statusCode(), retry);
        }
        JsonRuntime.JsonValue decoded;
        try {
            decoded = parse(new String(response.body(), StandardCharsets.UTF_8));
        } catch (JsonRuntime.JsonError error) {
            throw new AuthException("invalid-response", scheme, "the endpoint response is not readable JSON");
        }
        if (!(decoded instanceof JsonRuntime.JsonObject object)) throw new AuthException("invalid-response", scheme, "the endpoint response is not a JSON object");
        return object;
    }

    /** Decodes one RFC 6749 token response; a rotated refresh token is adopted, otherwise the previous one is retained. */
    private TokenSet tokenSetFrom(String scheme, JsonRuntime.JsonObject body, TokenSet previous) {
        JsonRuntime.JsonValue access = body.values().get("access_token");
        if (!(access instanceof JsonRuntime.JsonString token) || token.value().isEmpty() || control(token.value())) {
            throw new AuthException("invalid-response", scheme, "the token response carries no usable access token");
        }
        String type = "Bearer";
        JsonRuntime.JsonValue declared = body.values().get("token_type");
        if (declared instanceof JsonRuntime.JsonString text && !text.value().isBlank()) type = text.value().trim();
        if (!type.matches("[A-Za-z0-9!#$%&'*+.^_`|~-]+")) throw new AuthException("invalid-response", scheme, "the token type is not a usable authorization scheme");
        Long expiresAt = null;
        JsonRuntime.JsonValue expiresIn = body.values().get("expires_in");
        if (expiresIn instanceof JsonRuntime.JsonNumber seconds && seconds.isInteger() && seconds.signum() > 0
                && seconds.exactIntegerValue().bitLength() < 53) {
            expiresAt = Long.valueOf(clock.getAsLong() + seconds.exactIntegerValue().longValue() * 1000L);
        }
        String refresh = null;
        JsonRuntime.JsonValue declaredRefresh = body.values().get("refresh_token");
        if (declaredRefresh instanceof JsonRuntime.JsonString text && !text.value().isEmpty()) refresh = text.value();
        if (refresh == null && previous != null) refresh = previous.refreshToken;
        String scope = null;
        JsonRuntime.JsonValue declaredScope = body.values().get("scope");
        if (declaredScope instanceof JsonRuntime.JsonString text && !text.value().isEmpty()) scope = text.value();
        return new TokenSet(token.value(), type, expiresAt, refresh, scope);
    }

    /** Builds the complete Authorization attachment for one stored token set. Values never appear in errors. */
    private static HttpRuntime.Authorization authorization(TokenSet tokenSet, String scheme) {
        String type = tokenSet.tokenType.trim();
        if (type.isEmpty() || !type.matches("[A-Za-z0-9!#$%&'*+.^_`|~-]+")) {
            throw new AuthException("invalid-response", scheme, "the token type is not a usable authorization scheme");
        }
        return HttpRuntime.Authorization.of(type, tokenSet.accessToken);
    }

"#;

/// The plain credential methods: skew-aware client-credentials acquisition
/// with per-key single-flight and explicit refresh, exactly the pre-discovery
/// bytes.
const CREDENTIALS: &str = r#"    /**
     * Returns the scheme's cached token set, acquiring one from the compiled
     * client-credentials endpoint when the stored set is absent or expired
     * beyond the compiled skew. One acquisition runs per store key at a time
     * (single-flight): a caller waiting on the gate re-checks the store
     * instead of issuing a duplicate token request, and the store entry is
     * replaced atomically. A rotated refresh token from the response is
     * adopted; when the response carries none and a stored set exists, the
     * previous refresh token is retained.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet clientCredentialsToken(String scheme) { return clientCredentialsToken(scheme, null, null, null); }

    /**
     * Returns the scheme's cached token set, acquiring one from the compiled
     * client-credentials endpoint when the stored set is absent or expired
     * beyond the compiled skew. One acquisition runs per store key at a time
     * (single-flight), and the store entry is replaced atomically.
     *
     * @param scheme compiled source scheme name
     * @param clientId explicit client identity; defaults to the compiled environment variable
     * @param clientSecret explicit secret; defaults to the compiled environment variable
     * @param scope explicit scope string sent with the token request; nothing is inferred from operations
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet clientCredentialsToken(String scheme, String clientId, String clientSecret, String scope) {
        Scheme compiled = scheme(scheme);
        Flow flow = executableFlow(compiled, "client-credentials");
        if (flow.tokenUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled client-credentials flow declares no token URL");
        String[] identity = identity(compiled, clientId, clientSecret);
        String key = storeKey(scheme, flow.tokenUrl(), identity[0]);
        TokenSet stored = store.load(key);
        if (stored != null && stored.fresh(clock.getAsLong(), compiled.skew())) return stored;
        Object gate = gates.computeIfAbsent(key, unused -> new Object());
        synchronized (gate) {
            // Re-check the store after acquiring the per-key gate: another
            // holder may have populated it already.
            stored = store.load(key);
            if (stored != null && stored.fresh(clock.getAsLong(), compiled.skew())) return stored;
            Map<String, String> fields = new LinkedHashMap<>();
            fields.put("grant_type", "client_credentials");
            fields.put("scope", scope);
            TokenSet token = tokenSetFrom(scheme, postForm(scheme, flow.tokenUrl(), flow, identity, fields), stored);
            store.replace(key, token);
            return token;
        }
    }

    /**
     * The generated client's credential attach path: pass the returned
     * provider to {@code HttpRuntime.Options.Builder.authorization} under the
     * scheme's source name. Each protected call serves a fresh stored token,
     * otherwise it acquires one through {@link #clientCredentialsToken}.
     */
    public HttpRuntime.CredentialProvider clientCredentialsProvider(String scheme) {
        return context -> authorization(clientCredentialsToken(scheme), scheme);
    }

    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL. A rotated refresh token from the response is
     * adopted; the given one is retained otherwise. The store entry is
     * replaced atomically when {@code store} is supplied.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet refreshToken(String scheme, TokenSet tokenSet) { return refreshToken(scheme, tokenSet, null, null, null); }

    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL, adopting a rotated refresh token from the
     * response and retaining the given one otherwise.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet refreshToken(String scheme, TokenSet tokenSet, String clientId, String clientSecret, TokenStore store) {
        Objects.requireNonNull(tokenSet);
        if (tokenSet.refreshToken == null || tokenSet.refreshToken.isEmpty() || control(tokenSet.refreshToken)) {
            throw new AuthException("no-refresh-token", scheme, "the given token set carries no usable refresh token");
        }
        Scheme compiled = scheme(scheme);
        Flow flow = refreshFlow(compiled);
        String endpoint = refreshEndpoint(flow, compiled);
        String[] identity = identity(compiled, clientId, clientSecret);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("grant_type", "refresh_token");
        fields.put("refresh_token", tokenSet.refreshToken);
        TokenSet refreshed = tokenSetFrom(scheme, postForm(scheme, endpoint, flow, identity, fields), tokenSet);
        if (store != null) store.replace(storeKey(scheme, endpoint, identity[0]), refreshed);
        return refreshed;
    }
"#;

/// The discovery-aware credential methods: identical acquisition semantics,
/// with endpoint resolution following the compiled precedence (an explicit
/// compiled endpoint always wins, then the cached discovery document, then
/// the typed refusal the compiled plan alone would produce).
const CREDENTIALS_DISCOVERY: &str = r#"    /**
     * Returns the scheme's cached token set, acquiring one from the scheme's
     * client-credentials token endpoint when the stored set is absent or
     * expired beyond the compiled skew. One acquisition runs per store key at
     * a time (single-flight): a caller waiting on the gate re-checks the
     * store instead of issuing a duplicate token request, and the store entry
     * is replaced atomically. A rotated refresh token from the response is
     * adopted; when the response carries none and a stored set exists, the
     * previous refresh token is retained.
     *
     * <p>Endpoint resolution follows the compiled precedence: the compiled
     * client-credentials flow's token URL always wins; otherwise, when the
     * scheme compiles a discovery URL, the discovery document's
     * {@code token_endpoint} resolves the request (fetched once per scheme
     * and cached for this instance's lifetime, single-flighted across
     * concurrent callers, with a failed fetch retried on the next call);
     * otherwise the typed refusal stands.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet clientCredentialsToken(String scheme) { return clientCredentialsToken(scheme, null, null, null); }

    /**
     * Returns the scheme's cached token set, acquiring one from the scheme's
     * client-credentials token endpoint when the stored set is absent or
     * expired beyond the compiled skew, with per-key single-flight.
     *
     * @param scheme compiled source scheme name
     * @param clientId explicit client identity; defaults to the compiled environment variable
     * @param clientSecret explicit secret; defaults to the compiled environment variable
     * @param scope explicit scope string sent with the token request; nothing is inferred from operations
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet clientCredentialsToken(String scheme, String clientId, String clientSecret, String scope) {
        Scheme compiled = scheme(scheme);
        Flow flow = compiledFlowOrNull(compiled, "client-credentials");
        String[] identity = identity(compiled, clientId, clientSecret);
        String basic = flow == null ? discoveryBasicAuth(compiled, identity) : basicAuth(flow, scheme, identity);
        String tokenUrl = resolveEndpoint(compiled, flow == null ? null : flow.tokenUrl(), "token_endpoint");
        String key = storeKey(scheme, tokenUrl, identity[0]);
        TokenSet stored = store.load(key);
        if (stored != null && stored.fresh(clock.getAsLong(), compiled.skew())) return stored;
        Object gate = gates.computeIfAbsent(key, unused -> new Object());
        synchronized (gate) {
            // Re-check the store after acquiring the per-key gate: another
            // holder may have populated it already.
            stored = store.load(key);
            if (stored != null && stored.fresh(clock.getAsLong(), compiled.skew())) return stored;
            Map<String, String> fields = new LinkedHashMap<>();
            fields.put("grant_type", "client_credentials");
            fields.put("scope", scope);
            TokenSet token = tokenSetFrom(scheme, postForm(scheme, tokenUrl, basic, identity, fields), stored);
            store.replace(key, token);
            return token;
        }
    }

    /**
     * The generated client's credential attach path: pass the returned
     * provider to {@code HttpRuntime.Options.Builder.authorization} under the
     * scheme's source name. Each protected call serves a fresh stored token,
     * otherwise it acquires one through {@link #clientCredentialsToken}.
     */
    public HttpRuntime.CredentialProvider clientCredentialsProvider(String scheme) {
        return context -> authorization(clientCredentialsToken(scheme), scheme);
    }

    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL. A rotated refresh token from the response is
     * adopted; the given one is retained otherwise. The store entry is
     * replaced atomically when {@code store} is supplied.
     *
     * <p>Endpoint resolution follows the compiled precedence: the declared
     * refresh URL, else the compiled flow's token URL, always wins;
     * otherwise, when the scheme compiles a discovery URL, the discovery
     * document's {@code token_endpoint} resolves the exchange.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet refreshToken(String scheme, TokenSet tokenSet) { return refreshToken(scheme, tokenSet, null, null, null); }

    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL, adopting a rotated refresh token from the
     * response and retaining the given one otherwise.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet refreshToken(String scheme, TokenSet tokenSet, String clientId, String clientSecret, TokenStore store) {
        Objects.requireNonNull(tokenSet);
        if (tokenSet.refreshToken == null || tokenSet.refreshToken.isEmpty() || control(tokenSet.refreshToken)) {
            throw new AuthException("no-refresh-token", scheme, "the given token set carries no usable refresh token");
        }
        Scheme compiled = scheme(scheme);
        Flow flow = refreshFlowOrNull(compiled);
        String[] identity = identity(compiled, clientId, clientSecret);
        String endpoint;
        String basic;
        if (flow != null) {
            endpoint = refreshEndpoint(flow, compiled);
            basic = basicAuth(flow, scheme, identity);
        } else {
            endpoint = resolveEndpoint(compiled, null, "token_endpoint");
            basic = discoveryBasicAuth(compiled, identity);
        }
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("grant_type", "refresh_token");
        fields.put("refresh_token", tokenSet.refreshToken);
        TokenSet refreshed = tokenSetFrom(scheme, postForm(scheme, endpoint, basic, identity, fields), tokenSet);
        if (store != null) store.replace(storeKey(scheme, endpoint, identity[0]), refreshed);
        return refreshed;
    }
"#;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the cached discovery document's {@code revocation_endpoint}.
const REVOKE_DISCOVERY: &str = r#"    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded). Authentication follows the compiled client policy.
     *
     * <p>Endpoint resolution follows the compiled precedence: the configured
     * revocation endpoint always wins; otherwise, when the scheme compiles a
     * discovery URL, the discovery document's {@code revocation_endpoint}
     * resolves the request.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public void revoke(String scheme, String token) { revoke(scheme, token, null, null, null); }

    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded).
     *
     * @param tokenTypeHint {@code access_token} or {@code refresh_token}
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public void revoke(String scheme, String token, String tokenTypeHint, String clientId, String clientSecret) {
        if (token == null || token.isEmpty() || control(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        Scheme compiled = scheme(scheme);
        String[] identity = identity(compiled, clientId, clientSecret);
        String endpoint = resolveEndpoint(compiled, compiled.revocation(), "revocation_endpoint");
        Flow flow = refreshFlowOrNull(compiled);
        String basic = flow == null ? discoveryBasicAuth(compiled, identity) : basicAuth(flow, scheme, identity);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("token", token);
        fields.put("token_type_hint", tokenTypeHint);
        postForm(scheme, endpoint, basic, identity, fields);
    }
"#;

/// Authorization code with PKCE S256, emitted only when a compiled scheme
/// carries an executable authorization-code flow.
const AUTHORIZATION_CODE: &str = r#"    /** Consumed authorization-code transactions, by identity; a failed exchange still consumes the attempt. */
    private final Set<AuthorizationTransaction> consumed =
        Collections.synchronizedSet(Collections.newSetFromMap(new IdentityHashMap<>()));

    /**
     * Starts one authorization-code flow with PKCE (RFC 7636 S256): generates
     * the random state and code verifier from cryptographically secure bytes,
     * computes the S256 challenge, and returns the exact authorization URL to
     * redirect to, bound to the returned transaction. No network request is
     * made.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public AuthorizationTransaction beginAuthorization(String scheme, URI redirectUri) {
        return beginAuthorization(scheme, redirectUri, null, null);
    }

    /**
     * Starts one authorization-code flow with PKCE (RFC 7636 S256).
     *
     * @param scopes requested scopes; nothing is inferred from operations
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public AuthorizationTransaction beginAuthorization(String scheme, URI redirectUri, List<String> scopes, String clientId) {
        Scheme compiled = scheme(scheme);
        Flow flow = executableFlow(compiled, "authorization-code");
        if (flow.authorizationUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled authorization-code flow declares no authorization URL");
        if (flow.tokenUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled authorization-code flow declares no token URL");
        if (redirectUri == null || redirectUri.getScheme() == null || redirectUri.getHost() == null || redirectUri.getFragment() != null) {
            throw new AuthException("invalid-request", scheme, "redirectUri must be an absolute URI without a fragment");
        }
        String id = identity(compiled, clientId, null)[0];
        if (id == null || id.isEmpty()) throw new AuthException("missing-client-credentials", scheme, "a client id is required for the authorization-code flow; pass one or set the compiled environment variable");
        SecureRandom random = new SecureRandom();
        byte[] stateBytes = new byte[16];
        byte[] verifierBytes = new byte[32];
        random.nextBytes(stateBytes);
        random.nextBytes(verifierBytes);
        String state = base64Url(stateBytes);
        String codeVerifier = base64Url(verifierBytes);
        String codeChallenge;
        try {
            codeChallenge = base64Url(MessageDigest.getInstance("SHA-256")
                .digest(codeVerifier.getBytes(StandardCharsets.US_ASCII)));
        } catch (java.security.NoSuchAlgorithmException error) {
            throw new AuthException("crypto-unavailable", scheme, "SHA-256 is required for the authorization-code PKCE flow");
        }
        Map<String, String> query = new LinkedHashMap<>();
        query.put("response_type", "code");
        query.put("client_id", id);
        query.put("redirect_uri", redirectUri.toString());
        query.put("state", state);
        query.put("code_challenge", codeChallenge);
        query.put("code_challenge_method", "S256");
        if (scopes != null && !scopes.isEmpty()) query.put("scope", String.join(" ", scopes));
        StringBuilder target = new StringBuilder(flow.authorizationUrl());
        target.append(flow.authorizationUrl().contains("?") ? '&' : '?');
        boolean first = true;
        for (Map.Entry<String, String> entry : query.entrySet()) {
            if (!first) target.append('&');
            first = false;
            target.append(urlEncode(entry.getKey())).append('=').append(urlEncode(entry.getValue()));
        }
        return new AuthorizationTransaction(scheme, target.toString(), state, codeVerifier, codeChallenge,
            redirectUri.toString(), flow.tokenUrl(), clock.getAsLong());
    }

    /**
     * Completes one authorization-code transaction: validates the redirect
     * state, consumes the transaction exactly once (by any attempt, whether
     * the exchange succeeds or fails), exchanges the code at the
     * transaction's token URL with the stored verifier, and replaces the
     * stored token set atomically when {@code store} is supplied. A failed
     * exchange requires beginning a new authorization.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet completeAuthorization(AuthorizationTransaction transaction, String code, String state) {
        return completeAuthorization(transaction, code, state, null, null, null);
    }

    /**
     * Completes one authorization-code transaction: validates the redirect
     * state, consumes the transaction exactly once (by any attempt, whether
     * the exchange succeeds or fails), exchanges the code at the
     * transaction's token URL with the stored verifier, and replaces the
     * stored token set atomically when {@code store} is supplied.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet completeAuthorization(AuthorizationTransaction transaction, String code, String state,
            String clientId, String clientSecret, TokenStore store) {
        Objects.requireNonNull(transaction);
        synchronized (consumed) {
            if (consumed.contains(transaction)) throw new AuthException("transaction-consumed", transaction.scheme, "this authorization transaction was already consumed; begin a new authorization");
            consumed.add(transaction);
        }
        if (!Objects.equals(state, transaction.state)) throw new AuthException("state-mismatch", transaction.scheme, "the redirect state does not match the authorization transaction");
        if (code.isEmpty() || control(code)) throw new AuthException("invalid-request", transaction.scheme, "code must be a nonempty string without control characters");
        Scheme compiled = scheme(transaction.scheme);
        Flow flow = executableFlow(compiled, "authorization-code");
        String[] identity = identity(compiled, clientId, clientSecret);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("grant_type", "authorization_code");
        fields.put("code", code);
        fields.put("redirect_uri", transaction.redirectUri);
        fields.put("code_verifier", transaction.codeVerifier);
        TokenSet token = tokenSetFrom(transaction.scheme, postForm(transaction.scheme, transaction.tokenUrl, flow, identity, fields), null);
        if (store != null) store.replace(storeKey(transaction.scheme, transaction.tokenUrl, identity[0]), token);
        return token;
    }
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"    /**
     * Requests one device grant from the compiled device-authorization URL
     * (RFC 8628). Show the user the returned user code and verification URI,
     * then poll with {@link #pollDeviceAuthorization}.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public DeviceGrant beginDeviceAuthorization(String scheme) { return beginDeviceAuthorization(scheme, null, null); }

    /**
     * Requests one device grant from the compiled device-authorization URL
     * (RFC 8628).
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public DeviceGrant beginDeviceAuthorization(String scheme, String clientId, String clientSecret) {
        Scheme compiled = scheme(scheme);
        Flow flow = executableFlow(compiled, "device-authorization");
        if (flow.deviceAuthorizationUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled device-authorization flow declares no device-authorization URL");
        String[] identity = identity(compiled, clientId, clientSecret);
        JsonRuntime.JsonObject body = postForm(scheme, flow.deviceAuthorizationUrl(), flow, identity, new LinkedHashMap<>());
        JsonRuntime.JsonValue deviceCode = body.values().get("device_code");
        JsonRuntime.JsonValue userCode = body.values().get("user_code");
        JsonRuntime.JsonValue verificationUri = body.values().get("verification_uri");
        if (!(deviceCode instanceof JsonRuntime.JsonString device) || device.value().isEmpty() || control(device.value())
                || !(userCode instanceof JsonRuntime.JsonString user) || user.value().isEmpty()
                || !(verificationUri instanceof JsonRuntime.JsonString uri) || uri.value().isEmpty()) {
            throw new AuthException("invalid-response", scheme, "the device-authorization response carries no usable grant");
        }
        JsonRuntime.JsonValue complete = body.values().get("verification_uri_complete");
        Long expiresAt = null;
        JsonRuntime.JsonValue expiresIn = body.values().get("expires_in");
        if (expiresIn instanceof JsonRuntime.JsonNumber seconds && seconds.isInteger() && seconds.signum() > 0
                && seconds.exactIntegerValue().bitLength() < 53) {
            expiresAt = Long.valueOf(clock.getAsLong() + seconds.exactIntegerValue().longValue() * 1000L);
        }
        long interval = 5;
        JsonRuntime.JsonValue declared = body.values().get("interval");
        if (declared instanceof JsonRuntime.JsonNumber seconds && seconds.isInteger() && seconds.signum() > 0
                && seconds.exactIntegerValue().bitLength() < 63) {
            interval = seconds.exactIntegerValue().longValue();
        }
        return new DeviceGrant(scheme, device.value(), user.value(), uri.value(),
            complete instanceof JsonRuntime.JsonString text ? text.value() : null, expiresAt, interval);
    }

    /**
     * Polls the compiled token URL for one device grant to completion (RFC
     * 8628 3.5): {@code authorization_pending} waits the declared interval
     * and retries, {@code slow_down} grows the interval by five seconds, any
     * other refusal is a typed failure, and polling stops once the grant's
     * declared expiry passes. The sleeper is injectable (constructor
     * {@code sleeper}); the resulting token set replaces the store entry
     * atomically when {@code store} is supplied.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet pollDeviceAuthorization(DeviceGrant grant) { return pollDeviceAuthorization(grant, null, null, null); }

    /**
     * Polls the compiled token URL for one device grant to completion (RFC
     * 8628 3.5), honoring {@code authorization_pending}, {@code slow_down}
     * and the grant's declared expiry.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public TokenSet pollDeviceAuthorization(DeviceGrant grant, String clientId, String clientSecret, TokenStore store) {
        Objects.requireNonNull(grant);
        Scheme compiled = scheme(grant.scheme);
        Flow flow = executableFlow(compiled, "device-authorization");
        if (flow.tokenUrl() == null) throw new AuthException("endpoint-unavailable", grant.scheme, "the compiled device-authorization flow declares no token URL");
        String[] identity = identity(compiled, clientId, clientSecret);
        long interval = grant.intervalSeconds * 1000L;
        while (true) {
            if (grant.expiresAt != null && clock.getAsLong() >= grant.expiresAt) {
                throw new AuthException("device-code-expired", grant.scheme, "the device code expired before authorization completed");
            }
            Map<String, String> fields = new LinkedHashMap<>();
            fields.put("grant_type", "urn:ietf:params:oauth:grant-type:device_code");
            fields.put("device_code", grant.deviceCode);
            try {
                TokenSet token = tokenSetFrom(grant.scheme, postForm(grant.scheme, flow.tokenUrl(), flow, identity, fields), null);
                if (store != null) store.replace(storeKey(grant.scheme, flow.tokenUrl(), identity[0]), token);
                return token;
            } catch (AuthException error) {
                if ("authorization_pending".equals(error.serverError)) { sleeper.accept(interval); continue; }
                if ("slow_down".equals(error.serverError)) { interval += 5000; sleeper.accept(interval); continue; }
                if ("expired_token".equals(error.serverError)) throw new AuthException("device-code-expired", grant.scheme, "the device code expired before authorization completed");
                throw error;
            }
        }
    }
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries the
/// configured endpoint.
const REVOCATION: &str = r#"    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded). Authentication follows the compiled client policy.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public void revoke(String scheme, String token) { revoke(scheme, token, null, null, null); }

    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded).
     *
     * @param tokenTypeHint {@code access_token} or {@code refresh_token}
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public void revoke(String scheme, String token, String tokenTypeHint, String clientId, String clientSecret) {
        if (token == null || token.isEmpty() || control(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        Scheme compiled = scheme(scheme);
        if (compiled.revocation() == null) throw new AuthException("endpoint-unavailable", scheme, "no revocation endpoint was configured for this scheme");
        Flow flow = refreshFlow(compiled);
        String[] identity = identity(compiled, clientId, clientSecret);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("token", token);
        fields.put("token_type_hint", tokenTypeHint);
        postForm(scheme, compiled.revocation(), flow, identity, fields);
    }
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries the
/// configured endpoint.
const INTROSPECTION: &str = r#"    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded) and returns the server's JSON response. Authentication
     * follows the compiled client policy.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public JsonRuntime.JsonObject introspect(String scheme, String token) { return introspect(scheme, token, null, null, null); }

    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded).
     *
     * @param tokenTypeHint {@code access_token} or {@code refresh_token}
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public JsonRuntime.JsonObject introspect(String scheme, String token, String tokenTypeHint, String clientId, String clientSecret) {
        if (token == null || token.isEmpty() || control(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        Scheme compiled = scheme(scheme);
        if (compiled.introspection() == null) throw new AuthException("endpoint-unavailable", scheme, "no introspection endpoint was configured for this scheme");
        Flow flow = refreshFlow(compiled);
        String[] identity = identity(compiled, clientId, clientSecret);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("token", token);
        fields.put("token_type_hint", tokenTypeHint);
        return postForm(scheme, compiled.introspection(), flow, identity, fields);
    }
"#;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the cached discovery document's
/// {@code introspection_endpoint}.
const INTROSPECTION_DISCOVERY: &str = r#"    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded) and returns the server's JSON response. Authentication
     * follows the compiled client policy.
     *
     * <p>Endpoint resolution follows the compiled precedence: the configured
     * introspection endpoint always wins; otherwise, when the scheme compiles
     * a discovery URL, the discovery document's {@code introspection_endpoint}
     * resolves the request.
     *
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public JsonRuntime.JsonObject introspect(String scheme, String token) { return introspect(scheme, token, null, null, null); }

    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded).
     *
     * @param tokenTypeHint {@code access_token} or {@code refresh_token}
     * @throws AuthException typed lifecycle failure; messages never carry token or secret values
     */
    public JsonRuntime.JsonObject introspect(String scheme, String token, String tokenTypeHint, String clientId, String clientSecret) {
        if (token == null || token.isEmpty() || control(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        Scheme compiled = scheme(scheme);
        String[] identity = identity(compiled, clientId, clientSecret);
        String endpoint = resolveEndpoint(compiled, compiled.introspection(), "introspection_endpoint");
        Flow flow = refreshFlowOrNull(compiled);
        String basic = flow == null ? discoveryBasicAuth(compiled, identity) : basicAuth(flow, scheme, identity);
        Map<String, String> fields = new LinkedHashMap<>();
        fields.put("token", token);
        fields.put("token_type_hint", tokenTypeHint);
        return postForm(scheme, endpoint, basic, identity, fields);
    }
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-instance cache with single-flight, and the
/// endpoint-resolution precedence.
const DISCOVERY: &str = r#"    /** Compiled ceiling for one discovery document response (~1 MiB). */
    private static final int DISCOVERY_MAX_BYTES = 1 << 20;

    /** Per-instance discovery cache: successful documents keyed by scheme name, so repeated calls never re-fetch; a failed fetch is never cached, so the next call retries. */
    private final Map<String, JsonRuntime.JsonObject> discovered = new ConcurrentHashMap<>();

    /** Per-instance single-flight gates for discovery fetches, keyed by scheme name. */
    private final Map<String, Object> discoveryGates = new ConcurrentHashMap<>();

    /** The origin of one absolute http(s) URL: scheme, host and the port with the scheme default made explicit; null when the value is not an absolute http(s) URL. */
    private static String discoveryOrigin(String value) {
        URI parsed;
        try {
            parsed = URI.create(value);
        } catch (RuntimeException unparseable) {
            return null;
        }
        String scheme = parsed.getScheme() == null ? null : parsed.getScheme().toLowerCase(java.util.Locale.ROOT);
        String host = parsed.getHost();
        if (scheme == null || host == null || host.isEmpty() || (!"http".equals(scheme) && !"https".equals(scheme))) return null;
        int port = parsed.getPort();
        if (port == -1) port = "http".equals(scheme) ? 80 : 443;
        return scheme + "://" + host.toLowerCase(java.util.Locale.ROOT) + ":" + port;
    }

    /** Reads one discovery document member: absent stays null; a non-string or unusable value is a typed discovery failure. Unknown members are ignored. */
    private static String discoveredEndpoint(String scheme, JsonRuntime.JsonObject document, String member) {
        JsonRuntime.JsonValue value = document.values().get(member);
        if (value == null) return null;
        if (!(value instanceof JsonRuntime.JsonString text) || text.value().isEmpty() || control(text.value())) {
            throw new AuthException("discovery-failed", scheme, "the discovery document carries an unusable " + member + " value");
        }
        return text.value();
    }

    /**
     * Decodes and validates one discovery response. The response must be a
     * 2xx bounded JSON object whose {@code issuer} claim, when present, is an
     * absolute http(s) URL sharing the discovery URL's origin (scheme, host
     * and the port with the scheme default made explicit): OpenID Connect
     * openIdConnectUrl documents are validated against their {@code issuer}
     * claim exactly this way, as are RFC 8414 OAuth2 authorization-server
     * metadata documents. Failure messages carry only safe metadata, never
     * response body text.
     */
    private JsonRuntime.JsonObject discoveryDocument(Scheme compiled, String url, HttpResponse<byte[]> response) {
        String scheme = compiled.name();
        if (response.statusCode() < 200 || response.statusCode() > 299) {
            throw new AuthException("discovery-failed", scheme, "the discovery document request answered HTTP " + response.statusCode(), response.statusCode(), null, null);
        }
        String declared = response.headers().firstValue("content-length").orElse(null);
        if (declared != null && declared.trim().matches("[0-9]+") && Long.parseLong(declared.trim()) > DISCOVERY_MAX_BYTES) {
            throw new AuthException("discovery-failed", scheme, "the discovery document exceeds the compiled response ceiling");
        }
        if (response.body().length > DISCOVERY_MAX_BYTES) {
            throw new AuthException("discovery-failed", scheme, "the discovery document exceeds the compiled response ceiling");
        }
        JsonRuntime.JsonValue decoded;
        try {
            decoded = JsonRuntime.parse(response.body());
        } catch (JsonRuntime.JsonError error) {
            throw new AuthException("discovery-failed", scheme, "the discovery document is not readable JSON");
        }
        if (!(decoded instanceof JsonRuntime.JsonObject document)) {
            throw new AuthException("discovery-failed", scheme, "the discovery document is not a JSON object");
        }
        JsonRuntime.JsonValue issuer = document.values().get("issuer");
        if (issuer instanceof JsonRuntime.JsonString claim && !claim.value().isEmpty()) {
            String issuerOrigin = discoveryOrigin(claim.value());
            String discoveryOrigin = discoveryOrigin(url);
            if (issuerOrigin == null || discoveryOrigin == null || !issuerOrigin.equals(discoveryOrigin)) {
                throw new AuthException("discovery-failed", scheme, "the discovery document issuer does not share the discovery URL origin");
            }
        }
        return document;
    }

    /**
     * Fetches the scheme's discovery document through the injected transport
     * (GET, {@code accept: application/json}), returning the cached document
     * when one exists for this instance. Concurrent callers share the one
     * in-flight fetch (single-flight); timeouts stay with the injected
     * transport, exactly like every other lifecycle request.
     *
     * @throws AuthException typed lifecycle failure; messages never carry response body text
     */
    private JsonRuntime.JsonObject discover(Scheme compiled) {
        String url = compiled.discovery();
        if (url == null) throw new AuthException("endpoint-unavailable", compiled.name(), "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery");
        Object gate = discoveryGates.computeIfAbsent(compiled.name(), unused -> new Object());
        synchronized (gate) {
            JsonRuntime.JsonObject cached = discovered.get(compiled.name());
            if (cached != null) return cached;
            HttpRequest request = HttpRequest.newBuilder(URI.create(url)).timeout(timeout)
                .header("accept", "application/json").GET().build();
            HttpResponse<byte[]> response;
            try {
                response = http.send(request, HttpResponse.BodyHandlers.ofByteArray());
            } catch (java.io.IOException error) {
                throw new AuthException("discovery-failed", compiled.name(), "the discovery document request failed before a response arrived");
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
                throw new AuthException("aborted", compiled.name(), "the discovery document request was interrupted");
            }
            JsonRuntime.JsonObject document = discoveryDocument(compiled, url, response);
            discovered.put(compiled.name(), document);
            return document;
        }
    }

    /**
     * Resolves one lifecycle endpoint through the compiled precedence: an
     * explicit compiled endpoint always wins; otherwise the cached discovery
     * document's endpoint when the scheme compiles a discovery URL; otherwise
     * the typed refusal the compiled plan alone would produce.
     */
    private String resolveEndpoint(Scheme compiled, String compiledEndpoint, String member) {
        if (compiledEndpoint != null) return compiledEndpoint;
        JsonRuntime.JsonObject document = discover(compiled);
        String found = discoveredEndpoint(compiled.name(), document, member);
        if (found == null) throw new AuthException("endpoint-unavailable", compiled.name(), "neither the compiled plan nor the discovery document carries a " + member + " for this scheme");
        return found;
    }

    /** Client authentication for endpoints the discovery document supplies (no compiled flow declares one): RFC 6749 2.3.1 Basic credentials when the compiled configuration carries a client secret variable — an unavailable value becomes the typed missing-client-credentials refusal — else the public profile, which sends the client id in the form. */
    private String discoveryBasicAuth(Scheme compiled, String[] identity) {
        if (compiled.clientSecretEnv() == null) return null;
        if (identity[0] == null || identity[0].isEmpty() || identity[1] == null || identity[1].isEmpty()) {
            throw new AuthException("missing-client-credentials", compiled.name(), "the compiled client authentication is client-secret-basic and no complete client identity is available");
        }
        String pair = urlEncode(identity[0]) + ":" + urlEncode(identity[1]);
        return Base64.getEncoder().encodeToString(pair.getBytes(StandardCharsets.US_ASCII));
    }

    /** Resolves one executable (non-deprecated) compiled flow, or null when the scheme compiles none: a discovery-defined scheme's flows live in the discovery document. */
    private static Flow compiledFlowOrNull(Scheme compiled, String kind) {
        Flow flow = compiled.flows().get(kind);
        return flow == null || flow.deprecated() ? null : flow;
    }

    /** The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL, else null for a scheme the discovery document defines. */
    private static Flow refreshFlowOrNull(Scheme compiled) {
        for (String field : List.of("refreshUrl", "tokenUrl")) {
            for (String kind : List.of("authorization-code", "client-credentials", "device-authorization")) {
                Flow flow = compiled.flows().get(kind);
                if (flow == null || flow.deprecated()) continue;
                String url = "refreshUrl".equals(field) ? flow.refreshUrl() : flow.tokenUrl();
                if (url != null) return flow;
            }
        }
        return null;
    }

    /** One bounded, form-encoded endpoint POST with explicit client authentication; discovery-resolved endpoints have no compiled flow to authenticate with. A non-2xx response becomes the typed error. */
    private JsonRuntime.JsonObject postForm(String scheme, String endpoint, String basic, String[] identity, Map<String, String> fields) {
        Map<String, String> sent = new LinkedHashMap<>(fields);
        if (basic == null && identity[0] != null) sent.put("client_id", identity[0]);
        StringBuilder form = new StringBuilder();
        for (Map.Entry<String, String> entry : sent.entrySet()) {
            if (entry.getValue() == null) continue;
            if (form.length() > 0) form.append('&');
            form.append(urlEncode(entry.getKey())).append('=').append(urlEncode(entry.getValue()));
        }
        HttpRequest.Builder builder = HttpRequest.newBuilder(URI.create(endpoint)).timeout(timeout)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(form.toString(), StandardCharsets.UTF_8));
        if (basic != null) builder.header("authorization", "Basic " + basic);
        HttpResponse<byte[]> response;
        try {
            response = http.send(builder.build(), HttpResponse.BodyHandlers.ofByteArray());
        } catch (java.io.IOException error) {
            throw new AuthException("transport-failure", scheme, "the endpoint request failed before a response arrived");
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            throw new AuthException("aborted", scheme, "the endpoint request was interrupted");
        }
        if (response.statusCode() < 200 || response.statusCode() > 299) {
            Integer retry = null;
            String raw = response.headers().firstValue("retry-after").orElse(null);
            if (raw != null && raw.trim().matches("[0-9]+")) retry = Integer.valueOf(raw.trim());
            String serverError = null;
            try {
                JsonRuntime.JsonValue decoded = parse(new String(response.body(), StandardCharsets.UTF_8));
                if (decoded instanceof JsonRuntime.JsonObject object) {
                    JsonRuntime.JsonValue error = object.values().get("error");
                    if (error instanceof JsonRuntime.JsonString message && !message.value().isEmpty()) serverError = message.value();
                }
            } catch (JsonRuntime.JsonError ignored) {
                // An unreadable error body is safe metadata loss.
            }
            throw serverFailure(scheme, serverError, response.statusCode(), retry);
        }
        JsonRuntime.JsonValue decoded;
        try {
            decoded = parse(new String(response.body(), StandardCharsets.UTF_8));
        } catch (JsonRuntime.JsonError error) {
            throw new AuthException("invalid-response", scheme, "the endpoint response is not readable JSON");
        }
        if (!(decoded instanceof JsonRuntime.JsonObject object)) throw new AuthException("invalid-response", scheme, "the endpoint response is not a JSON object");
        return object;
    }
"#;

/// The replaying credential wrapper's import block: joins the import block
/// only when the wrapper participates, so every import still precedes the
/// emitted types and non-replaying plans stay byte-identical.
const REPLAY_IMPORTS: &str = r#"import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.Authenticator;
import java.net.CookieHandler;
import java.net.ProxySelector;
import java.nio.ByteBuffer;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicReference;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;

"#;

/// The replaying provider's OAuth-level factory methods, shared by the plain
/// and discovery variants.
const REPLAY_FACTORIES: &str = r#"
    /**
     * Creates the replaying variant of the compiled client-credentials
     * provider. Every argument behaves exactly as in
     * {@code clientCredentialsToken}; the replay semantics are strictly
     * additive and the plain provider keeps today's attach-only semantics.
     *
     * @return the replaying credential wiring its provider and transport
     */
    public ReplayingCredentials replayingCredentials(String scheme) { return replayingCredentials(scheme, null, null, null); }

    /**
     * Creates the replaying variant of the compiled client-credentials
     * provider with explicit client identity and scope. Creation refuses a
     * compiled scheme whose client-credentials flow declares no token URL,
     * exactly like the plain provider's first attach.
     *
     * @param scheme compiled source scheme name
     * @param clientId explicit client identity; defaults to the compiled environment variable
     * @param clientSecret explicit secret; defaults to the compiled environment variable
     * @param scope explicit scope string sent with the token request; nothing is inferred from operations
     * @return the replaying credential wiring its provider and transport
     */
    public ReplayingCredentials replayingCredentials(String scheme, String clientId, String clientSecret, String scope) {
        return new ReplayingCredentials(this, scheme, clientId, clientSecret, scope);
    }
"#;

/// The plain-variant replaying credential: the compiled client-credentials
/// token endpoint serves both the store key and the lifecycle exclusion,
/// exactly the pre-discovery provider bytes it wraps.
const REPLAY_CREDENTIALS_PLAIN: &str = r#"
    /**
     * The replaying variant of the compiled client-credentials provider: the
     * plain provider's attach behavior plus the unified request policy. Use
     * it in two places — pass {@link #provider()} as the scheme's
     * {@code authorization} member, and pass {@link #transport(HttpClient)}
     * as the client's {@code httpClient}:
     *
     * <pre>{@code
     * OAuth.ReplayingCredentials replay = oauth.replayingCredentials("service");
     * Client client = new Client(HttpRuntime.Options.builder()
     *     .authorization("service", replay.provider())
     *     .httpClient(replay.transport(myHttpClient)).build());
     * }</pre>
     *
     * A 401 (and only a 401) on a request whose Authorization value this
     * provider attached triggers exactly one coordinated refresh — concurrent
     * 401s share one token request through the same single-flight store round
     * — and exactly one replay of the request with the fresh token,
     * preserving method, URL and body while regenerating headers through the
     * normal attach path. The second response is surfaced whatever it is: a
     * second 401 reaches the caller as the declared error. The overall budget
     * is one refresh plus one replay, never nested with other retry policies
     * (requests are not retried today). Attaches for stream-protected
     * requirements are never replayed, because delivered stream data prevents
     * a transparent restart. A refresh failure surfaces as the typed
     * {@link AuthException} instead of a replay. The plain provider keeps
     * today's semantics: replay is this wrapper's opt-in only.
     */
    public static final class ReplayingCredentials {
        private final OAuth owner;
        private final String scheme, clientId, clientSecret, scope;
        /** The attach values this provider served, most recent first. The record keeps only safe metadata; token values already traveled on the wire. */
        private final List<Served> served = new ArrayList<>();
        private final Object servedLock = new Object();
        /** Coordinated refresh rounds, keyed by exact store key. */
        private final Map<String, ReplayRound> rounds = new HashMap<>();
        private final Object roundsLock = new Object();

        ReplayingCredentials(OAuth owner, String scheme, String clientId, String clientSecret, String scope) {
            this.owner = owner;
            this.scheme = Objects.requireNonNull(scheme);
            Flow flow = owner.executableFlow(owner.scheme(scheme), "client-credentials");
            if (flow.tokenUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled client-credentials flow declares no token URL");
            this.clientId = clientId;
            this.clientSecret = clientSecret;
            this.scope = scope;
        }

        /**
         * The credential attach path: serves a fresh stored token, otherwise
         * acquires one through {@link OAuth#clientCredentialsToken}, and
         * remembers the complete Authorization value so the transport wrapper
         * can tell which requests carried this provider's token.
         *
         * @return the credential hook for the wrapped scheme
         */
        public HttpRuntime.CredentialProvider provider() {
            return context -> {
                TokenSet set = owner.clientCredentialsToken(scheme, clientId, clientSecret, scope);
                String value = replayValue(set);
                record(value, eligible(context));
                return authorization(set, scheme);
            };
        }

        /**
         * Wraps {@code inner} with the one-refresh-one-replay 401 policy;
         * call it once per client. Token requests keep traveling through
         * {@code inner} directly, so construct the owning OAuth value over
         * the same transport the client gets wrapped around.
         *
         * @param inner the caller's transport
         * @return the wrapped transport carrying the replay policy
         */
        public HttpClient transport(HttpClient inner) {
            Objects.requireNonNull(inner);
            return new ReplayHttpClient(this, inner);
        }

        /** Whether one 401 response qualifies for the replay policy: the presented value is one this provider attached on an eligible (non-stream-protected) requirement, and the target is not a compiled lifecycle endpoint. Lifecycle requests carry no bearer token of this provider, so the exact-target guard is defense in depth. */
        boolean replayable(String presented, String target) {
            if (presented == null || presented.isEmpty()) return false;
            if (lifecycleTarget(target)) return false;
            synchronized (servedLock) {
                for (Served entry : served) {
                    if (entry.value.equals(presented) && entry.eligible) return true;
                }
            }
            return false;
        }

        /**
         * One coordinated refresh: a newer stored set wins over a stale
         * re-refresh, concurrent 401s share one round through the store key's
         * gate, and a failed round fails every waiter exactly once. Resolves
         * to the fresh complete Authorization value.
         */
        String refresh(String presented) {
            Scheme compiled = owner.scheme(scheme);
            String key = storeKey(scheme, replayTokenUrl(compiled), owner.identity(compiled, clientId, clientSecret)[0]);
            String fresh;
            ReplayRound round;
            boolean leader;
            synchronized (roundsLock) {
                TokenSet stored = owner.store.load(key);
                if (stored != null) {
                    String value = replayValue(stored);
                    if (!value.equals(presented)) return value;
                }
                round = rounds.get(key);
                leader = round == null;
                if (leader) {
                    round = new ReplayRound();
                    rounds.put(key, round);
                }
            }
            if (!leader) return round.await();
            try {
                owner.store.clear(key);
                fresh = replayValue(owner.clientCredentialsToken(scheme, clientId, clientSecret, scope));
            } catch (RuntimeException failure) {
                synchronized (roundsLock) { rounds.remove(key); }
                round.fail(failure);
                throw failure;
            }
            synchronized (roundsLock) { rounds.remove(key); }
            round.complete(fresh);
            return fresh;
        }

        private void record(String value, boolean eligible) {
            synchronized (servedLock) {
                served.add(0, new Served(value, eligible));
                if (served.size() > 8) served.subList(8, served.size()).clear();
            }
        }

        /** Whether one attach may be replayed: attaches for stream-protected requirements never are, because delivered stream data prevents a transparent restart. */
        private boolean eligible(HttpRuntime.CredentialContext context) {
            for (String pointer : REPLAY_NO_REPLAY.getOrDefault(scheme, Set.of())) {
                if (pointer.equals(context.source())) return false;
            }
            return true;
        }

        /** Whether one request target is one of the compiled lifecycle endpoints this provider's token plumbing posts to; such requests carry no replayable bearer token, and the exact-target guard is defense in depth against replay loops. */
        private boolean lifecycleTarget(String target) {
            Scheme compiled = owner.scheme(scheme);
            Flow flow = compiled.flows().get("client-credentials");
            if (flow == null || flow.deprecated()) return false;
            return target.equals(flow.tokenUrl()) || (flow.refreshUrl() != null && flow.refreshUrl().equals(target));
        }

        /** The compiled client-credentials token URL: exactly the endpoint the wrapped acquisition posts to. */
        private String replayTokenUrl(Scheme compiled) {
            Flow flow = owner.executableFlow(compiled, "client-credentials");
            if (flow.tokenUrl() == null) throw new AuthException("endpoint-unavailable", scheme, "the compiled client-credentials flow declares no token URL");
            return flow.tokenUrl();
        }

        /** The complete Authorization header value of one token set. Values never appear in errors. */
        private String replayValue(TokenSet tokenSet) {
            String type = tokenSet.tokenType.trim();
            if (type.isEmpty() || !type.matches("[A-Za-z0-9!#$%&'*+.^_`|~-]+")) {
                throw new AuthException("invalid-response", scheme, "the token type is not a usable authorization scheme");
            }
            return type + " " + tokenSet.accessToken;
        }

        /** One attach this provider served, remembered so the transport can tell which requests carried this provider's token. The record keeps only safe metadata; token values already traveled on the wire. */
        private record Served(String value, boolean eligible) {}

        /** One coordinated refresh round: exactly one forced acquisition, shared by every concurrent 401 that presented the same stale token; a failed round fails every waiter exactly once. */
        private static final class ReplayRound {
            private final CountDownLatch done = new CountDownLatch(1);
            private String value;
            private RuntimeException failure;

            void complete(String fresh) { value = fresh; done.countDown(); }

            void fail(RuntimeException error) { failure = error; done.countDown(); }

            /** The round's fresh value, or the failure every waiter shares. */
            String await() {
                try {
                    done.await();
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new AuthException("aborted", "", "the replay refresh was interrupted");
                }
                if (failure != null) throw failure;
                if (value == null) throw new AuthException("replay-refresh", "", "the replay refresh round completed without a value");
                return value;
            }
        }
    }
"#;

/// The discovery-variant replaying credential: the refresh endpoint resolves
/// through the compiled precedence (compiled token URL, else the discovery
/// document's), cached per instance exactly like the provider it wraps, and
/// the compiled discovery URL joins the lifecycle-endpoint guard.
const REPLAY_CREDENTIALS_DISCOVERY: &str = r#"
    /**
     * The replaying variant of the compiled client-credentials provider: the
     * discovery-aware provider's attach behavior plus the unified request
     * policy. Use it in two places — pass {@link #provider()} as the
     * scheme's {@code authorization} member, and pass
     * {@link #transport(HttpClient)} as the client's {@code httpClient}.
     *
     * <p>Every provider argument behaves exactly as in
     * {@code clientCredentialsToken}; the replay semantics are strictly
     * additive and the plain provider keeps today's attach-only semantics.
     * The refresh endpoint resolves through the compiled precedence — the
     * compiled token URL when the client-credentials flow compiles one,
     * otherwise the discovery document's {@code token_endpoint}, fetched once
     * and cached for this instance's lifetime. See the plain variant for the
     * one-refresh-one-replay contract.
     */
    public static final class ReplayingCredentials {
        private final OAuth owner;
        private final String scheme, clientId, clientSecret, scope;
        /** The attach values this provider served, most recent first. The record keeps only safe metadata; token values already traveled on the wire. */
        private final List<Served> served = new ArrayList<>();
        private final Object servedLock = new Object();
        /** Coordinated refresh rounds, keyed by exact store key. */
        private final Map<String, ReplayRound> rounds = new HashMap<>();
        private final Object roundsLock = new Object();

        ReplayingCredentials(OAuth owner, String scheme, String clientId, String clientSecret, String scope) {
            this.owner = owner;
            this.scheme = Objects.requireNonNull(scheme);
            owner.scheme(scheme);
            this.clientId = clientId;
            this.clientSecret = clientSecret;
            this.scope = scope;
        }

        /**
         * The credential attach path: serves a fresh stored token, otherwise
         * acquires one through {@link OAuth#clientCredentialsToken}, and
         * remembers the complete Authorization value so the transport wrapper
         * can tell which requests carried this provider's token.
         *
         * @return provider credential hook for the wrapped scheme
         */
        public HttpRuntime.CredentialProvider provider() {
            return context -> {
                TokenSet set = owner.clientCredentialsToken(scheme, clientId, clientSecret, scope);
                String value = replayValue(set);
                record(value, eligible(context));
                return authorization(set, scheme);
            };
        }

        /**
         * Wraps {@code inner} with the one-refresh-one-replay 401 policy;
         * call it once per client. Token requests keep traveling through
         * {@code inner} directly, so construct the owning OAuth value over
         * the same transport the client gets wrapped around.
         *
         * @param inner the caller's transport
         * @return the wrapped transport carrying the replay policy
         */
        public HttpClient transport(HttpClient inner) {
            Objects.requireNonNull(inner);
            return new ReplayHttpClient(this, inner);
        }

        /** Whether one 401 response qualifies for the replay policy: the presented value is one this provider attached on an eligible (non-stream-protected) requirement, and the target is not a compiled lifecycle endpoint. Lifecycle requests carry no bearer token of this provider, so the exact-target guard is defense in depth. */
        boolean replayable(String presented, String target) {
            if (presented == null || presented.isEmpty()) return false;
            if (lifecycleTarget(target)) return false;
            synchronized (servedLock) {
                for (Served entry : served) {
                    if (entry.value.equals(presented) && entry.eligible) return true;
                }
            }
            return false;
        }

        /**
         * One coordinated refresh: a newer stored set wins over a stale
         * re-refresh, concurrent 401s share one round through the store key's
         * gate, and a failed round fails every waiter exactly once. Resolves
         * to the fresh complete Authorization value.
         */
        String refresh(String presented) {
            Scheme compiled = owner.scheme(scheme);
            String key = storeKey(scheme, replayTokenUrl(compiled), owner.identity(compiled, clientId, clientSecret)[0]);
            String fresh;
            ReplayRound round;
            boolean leader;
            synchronized (roundsLock) {
                TokenSet stored = owner.store.load(key);
                if (stored != null) {
                    String value = replayValue(stored);
                    if (!value.equals(presented)) return value;
                }
                round = rounds.get(key);
                leader = round == null;
                if (leader) {
                    round = new ReplayRound();
                    rounds.put(key, round);
                }
            }
            if (!leader) return round.await();
            try {
                owner.store.clear(key);
                fresh = replayValue(owner.clientCredentialsToken(scheme, clientId, clientSecret, scope));
            } catch (RuntimeException failure) {
                synchronized (roundsLock) { rounds.remove(key); }
                round.fail(failure);
                throw failure;
            }
            synchronized (roundsLock) { rounds.remove(key); }
            round.complete(fresh);
            return fresh;
        }

        private void record(String value, boolean eligible) {
            synchronized (servedLock) {
                served.add(0, new Served(value, eligible));
                if (served.size() > 8) served.subList(8, served.size()).clear();
            }
        }

        /** Whether one attach may be replayed: attaches for stream-protected requirements never are, because delivered stream data prevents a transparent restart. */
        private boolean eligible(HttpRuntime.CredentialContext context) {
            for (String pointer : REPLAY_NO_REPLAY.getOrDefault(scheme, Set.of())) {
                if (pointer.equals(context.source())) return false;
            }
            return true;
        }

        /** Whether one request target is one of the compiled lifecycle endpoints this provider's token plumbing posts to; the compiled discovery URL joins the compiled endpoints, and the resolved token endpoint rides the exact-token match. Such requests carry no replayable bearer token, so the exact-target guard is defense in depth against replay loops. */
        private boolean lifecycleTarget(String target) {
            Scheme compiled = owner.scheme(scheme);
            Flow flow = compiled.flows().get("client-credentials");
            if (flow != null && !flow.deprecated()
                    && (target.equals(flow.tokenUrl()) || (flow.refreshUrl() != null && flow.refreshUrl().equals(target)))) return true;
            return compiled.discovery() != null && compiled.discovery().equals(target);
        }

        /** The client-credentials token endpoint through the compiled precedence: the compiled token URL when the flow compiles one, otherwise the discovery document's. */
        private String replayTokenUrl(Scheme compiled) {
            Flow flow = compiledFlowOrNull(compiled, "client-credentials");
            return owner.resolveEndpoint(compiled, flow == null ? null : flow.tokenUrl(), "token_endpoint");
        }

        /** The complete Authorization header value of one token set. Values never appear in errors. */
        private String replayValue(TokenSet tokenSet) {
            String type = tokenSet.tokenType.trim();
            if (type.isEmpty() || !type.matches("[A-Za-z0-9!#$%&'*+.^_`|~-]+")) {
                throw new AuthException("invalid-response", scheme, "the token type is not a usable authorization scheme");
            }
            return type + " " + tokenSet.accessToken;
        }

        /** One attach this provider served, remembered so the transport can tell which requests carried this provider's token. The record keeps only safe metadata; token values already traveled on the wire. */
        private record Served(String value, boolean eligible) {}

        /** One coordinated refresh round: exactly one forced acquisition, shared by every concurrent 401 that presented the same stale token; a failed round fails every waiter exactly once. */
        private static final class ReplayRound {
            private final CountDownLatch done = new CountDownLatch(1);
            private String value;
            private RuntimeException failure;

            void complete(String fresh) { value = fresh; done.countDown(); }

            void fail(RuntimeException error) { failure = error; done.countDown(); }

            /** The round's fresh value, or the failure every waiter shares. */
            String await() {
                try {
                    done.await();
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new AuthException("aborted", "", "the replay refresh was interrupted");
                }
                if (failure != null) throw failure;
                if (value == null) throw new AuthException("replay-refresh", "", "the replay refresh round completed without a value");
                return value;
            }
        }
    }
"#;

/// The replaying transport: the 401 interception half of the wrapper, shared
/// by the plain and discovery variants over the synchronous and asynchronous
/// client paths. The replay leg travels through the caller's transport
/// exactly once, carrying the buffered request body.
const REPLAY_TRANSPORT: &str = r#"
    /** Compiled ceiling for one request body buffered for replay. */
    private static final int REPLAY_MAX_BODY_BYTES = 1 << 26;

    /**
     * The replaying credential's client transport: one coordinated refresh
     * and, when the request carried the provider's token and no stream is
     * protected on it, exactly one replay with the fresh token. Lifecycle
     * endpoint requests are never replayed: they carry no bearer token of
     * this provider, and the exact-target guard in replayable is defense in
     * depth. The synchronous and asynchronous paths apply the same policy.
     */
    public static final class ReplayHttpClient extends HttpClient {
        private final ReplayingCredentials replay;
        private final HttpClient inner;

        ReplayHttpClient(ReplayingCredentials replay, HttpClient inner) { this.replay = replay; this.inner = inner; }

        @Override public Optional<CookieHandler> cookieHandler() { return inner.cookieHandler(); }
        @Override public Optional<Duration> connectTimeout() { return inner.connectTimeout(); }
        @Override public Redirect followRedirects() { return inner.followRedirects(); }
        @Override public Optional<ProxySelector> proxy() { return inner.proxy(); }
        @Override public SSLContext sslContext() { return inner.sslContext(); }
        @Override public SSLParameters sslParameters() { return inner.sslParameters(); }
        @Override public Optional<Authenticator> authenticator() { return inner.authenticator(); }
        @Override public Version version() { return inner.version(); }
        @Override public Optional<Executor> executor() { return inner.executor(); }

        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) throws IOException, InterruptedException {
            byte[] body = replayBody(request);
            HttpRequest outgoing = body == null ? request : rebuild(request, body, null);
            HttpResponse<T> response = inner.send(outgoing, handler);
            String presented = outgoing.headers().firstValue("authorization").orElse(null);
            if (response.statusCode() != 401 || !replay.replayable(presented, outgoing.uri().toString())) return response;
            return inner.send(rebuild(outgoing, body, replay.refresh(presented)), handler);
        }

        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }

        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            byte[] body = replayBody(request);
            HttpRequest outgoing = body == null ? request : rebuild(request, body, null);
            return inner.sendAsync(outgoing, handler).thenCompose(response -> {
                String presented = outgoing.headers().firstValue("authorization").orElse(null);
                if (response.statusCode() != 401 || !replay.replayable(presented, outgoing.uri().toString())) {
                    return CompletableFuture.completedFuture(response);
                }
                try {
                    return CompletableFuture.completedFuture(inner.send(rebuild(outgoing, body, replay.refresh(presented)), handler));
                } catch (IOException | InterruptedException failure) {
                    CompletableFuture<HttpResponse<T>> failed = new CompletableFuture<>();
                    failed.completeExceptionally(failure);
                    return failed;
                }
            });
        }

        /** Reads one request body into memory, bounded for replay; null when the body is absent, oversized or unreadable, so the request travels unreplayable. */
        private static byte[] replayBody(HttpRequest request) {
            Optional<HttpRequest.BodyPublisher> publisher = request.bodyPublisher();
            if (publisher.isEmpty()) return null;
            long declared = publisher.get().contentLength();
            if (declared == 0 || declared > REPLAY_MAX_BODY_BYTES) return null;
            ByteArrayOutputStream out = new ByteArrayOutputStream(declared > 0 ? (int) declared : 512);
            CountDownLatch done = new CountDownLatch(1);
            AtomicReference<Throwable> failure = new AtomicReference<>();
            publisher.get().subscribe(new java.util.concurrent.Flow.Subscriber<>() {
                private java.util.concurrent.Flow.Subscription subscription;

                @Override public void onSubscribe(java.util.concurrent.Flow.Subscription value) { subscription = value; value.request(Long.MAX_VALUE); }
                @Override public void onNext(ByteBuffer item) {
                    byte[] bytes = new byte[item.remaining()];
                    item.get(bytes);
                    if (out.size() + bytes.length > REPLAY_MAX_BODY_BYTES) {
                        failure.compareAndSet(null, new IllegalStateException("request body exceeds the replay ceiling"));
                        subscription.cancel();
                        done.countDown();
                        return;
                    }
                    out.writeBytes(bytes);
                }
                @Override public void onError(Throwable error) { failure.compareAndSet(null, error); done.countDown(); }
                @Override public void onComplete() { done.countDown(); }
            });
            try {
                done.await();
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                return null;
            }
            return failure.get() == null ? out.toByteArray() : null;
        }

        /** One rebuilt request: identical method, URI, timeout, continuation and headers, with {@code fresh} replacing the Authorization value (null keeps the presented one). */
        private static HttpRequest rebuild(HttpRequest request, byte[] body, String fresh) {
            HttpRequest.Builder builder = HttpRequest.newBuilder(request.uri())
                .method(request.method(), body == null ? HttpRequest.BodyPublishers.noBody() : HttpRequest.BodyPublishers.ofByteArray(body));
            request.timeout().ifPresent(builder::timeout);
            if (request.expectContinue()) builder.expectContinue(true);
            request.headers().map().forEach((name, values) -> {
                if ("authorization".equalsIgnoreCase(name)) return;
                values.forEach(value -> builder.header(name, value));
            });
            String value = fresh != null ? fresh : request.headers().firstValue("authorization").orElse(null);
            if (value != null) builder.header("authorization", value);
            return builder.build();
        }
    }
"#;
