//! Emitted-only OAuth 2.0 / OpenID Connect token lifecycle for the generated
//! PHP package.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when client defaults are configured) compiles into one generated
//! `src/OAuth.php` class: compiled per-scheme descriptors as class constants,
//! a `TokenSet` value, the caller-implementable `TokenStore` interface with an
//! instance-owned `MemoryTokenStore`, client-credentials acquisition with
//! skew-aware caching and per-instance single-flight, explicit refresh, and —
//! only when the compiled schemes carry them — authorization-code with PKCE
//! S256, RFC 8628 device polling, RFC 7009 revocation and RFC 7662
//! introspection. Implicit and password flows are represented by the plan but
//! never executed, so schemes with only those flows emit nothing.
//!
//! Emission is strictly conditional: without a configured `sdk_defaults`
//! policy, or without any usable scheme, the backend emits no new file and
//! every other artifact stays byte-identical. The runtime never parses OpenAPI;
//! client identity comes from explicit arguments or the compiled environment
//! variable names, read at call time, and token or secret values never enter
//! exception messages. Token requests ride the package's injectable
//! `Transport` seam, so caller transport policy covers the lifecycle too.

use super::{Operation, SdkPlan, emit::php};
use crate::http_protocol::{
    CredentialHook, OAuthClientAuth, OAuthFlowDescriptor, OAuthFlowDescriptorKind, OAuthPlan,
    OAuthSchemeKind, OAuthSchemePlan, Representation,
};
use std::collections::{BTreeMap, BTreeSet};

/// Class names `src/OAuth.php` owns in the package namespace. Reserved against
/// native model symbols only while the file is emitted, so no-policy
/// allocation behavior is unchanged.
pub(super) const CLASS_NAMES: &[&str] = &[
    "OAuth",
    "TokenSet",
    "TokenStore",
    "MemoryTokenStore",
    "AuthException",
    "AuthorizationTransaction",
    "DeviceGrant",
];

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

/// Whether one compiled scheme contributes anything executable. A compiled
/// discovery URL contributes on its own: the discovery document defines the
/// scheme's endpoints at runtime.
fn usable_scheme(scheme: &OAuthSchemePlan) -> bool {
    scheme.discovery.is_some() || scheme.flows.iter().any(executable)
}

/// Whether the compiled plan justifies emitting `src/OAuth.php` at all. A
/// discovery URL makes a scheme usable even with no declared flows: OpenID
/// Connect schemes have their endpoints defined by the discovery document at
/// runtime.
pub(super) fn emittable(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(usable_scheme)
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that attach path, so plans without one emit
/// exactly the pre-replay bytes.
pub(super) fn replaying(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        scheme
            .flows
            .iter()
            .any(|flow| executable(flow) && flow.kind == OAuthFlowDescriptorKind::ClientCredentials)
    })
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
fn no_replay_requirements(
    oauth: &OAuthPlan,
    operations: &[Operation],
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

/// One compiled flow as a PHP array-literal body, with exactly the compiled
/// values and `null` for every absent declaration.
fn flow_entry(flow: &OAuthFlowDescriptor) -> String {
    let url = |value: &Option<String>| match value {
        Some(url) => php(url),
        None => "null".to_owned(),
    };
    let scopes = flow
        .scopes
        .iter()
        .map(|(name, description)| format!("{} => {}, ", php(name), php(description)))
        .collect::<Vec<_>>()
        .join("");
    format!(
        "                {} => ['authorization_url' => {}, 'token_url' => {}, 'refresh_url' => {}, 'device_authorization_url' => {}, 'client_auth' => {}, 'deprecated' => {}, 'scopes' => [{}]],\n",
        php(flow_kind_name(flow.kind)),
        url(&flow.authorization_url),
        url(&flow.token_url),
        url(&flow.refresh_url),
        url(&flow.device_authorization_url),
        php(client_auth_name(flow.client_auth)),
        flow.deprecated_flow,
        scopes,
    )
}

/// One compiled scheme as a PHP array-literal entry, in plan order. The
/// `discovery` member appears only when at least one compiled scheme carries
/// a discovery URL, so discovery-less plans assemble byte-identically.
fn scheme_entry(scheme: &OAuthSchemePlan, discovery: bool) -> String {
    let optional = |value: &Option<String>| match value {
        Some(value) => php(value),
        None => "null".to_owned(),
    };
    let mut entry = format!("        {} => [\n", php(&scheme.name));
    entry.push_str(&format!("            'name' => {},\n", php(&scheme.name)));
    entry.push_str(&format!(
        "            'kind' => {},\n",
        php(scheme_kind_name(scheme.kind))
    ));
    entry.push_str(&format!(
        "            'skew' => {},\n",
        scheme.refresh_skew_seconds
    ));
    if discovery {
        entry.push_str(&format!(
            "            'discovery' => {},\n",
            optional(&scheme.discovery)
        ));
    }
    entry.push_str(&format!(
        "            'client_id_env' => {},\n",
        optional(&scheme.client_id_env)
    ));
    entry.push_str(&format!(
        "            'client_secret_env' => {},\n",
        optional(&scheme.client_secret_env)
    ));
    entry.push_str(&format!(
        "            'revocation' => {},\n",
        optional(&scheme.revocation_endpoint)
    ));
    entry.push_str(&format!(
        "            'introspection' => {},\n",
        optional(&scheme.introspection_endpoint)
    ));
    entry.push_str("            'flows' => [\n");
    for flow in &scheme.flows {
        entry.push_str(&flow_entry(flow));
    }
    entry.push_str("            ],\n");
    entry.push_str("        ],\n");
    entry
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

/// The complete `src/OAuth.php` source for a plan with usable schemes, or
/// `None` when nothing is executable. Plans without a discovery URL assemble
/// byte-identically to the pre-discovery emission; plans with one emit the
/// discovery-aware acquisition methods and the discovery engine.
pub(super) fn source(plan: &SdkPlan) -> Option<String> {
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
    // stream-protected requirement pointers of that plan's operations.
    let replay = replaying(oauth);
    let no_replay = if replay {
        no_replay_requirements(oauth, plan.operations())
    } else {
        BTreeMap::new()
    };
    let namespace = plan.config().namespace.clone();
    let table = schemes
        .iter()
        .map(|scheme| scheme_entry(scheme, discovery))
        .collect::<String>();
    // The compiled descriptor type carries the discovery member only when a
    // discovery URL participates, and the per-instance discovery cache joins
    // the class members only then.
    let descriptor_type = if discovery {
        "array<string, array{name: string, kind: string, skew: int, discovery: string|null, client_id_env: string|null, client_secret_env: string|null, revocation: string|null, introspection: string|null, flows: array<string, array{authorization_url: string|null, token_url: string|null, refresh_url: string|null, device_authorization_url: string|null, client_auth: string, deprecated: bool, scopes: array<string,string>}>}>"
    } else {
        "array<string, array{name: string, kind: string, skew: int, client_id_env: string|null, client_secret_env: string|null, revocation: string|null, introspection: string|null, flows: array<string, array{authorization_url: string|null, token_url: string|null, refresh_url: string|null, device_authorization_url: string|null, client_auth: string, deprecated: bool, scopes: array<string,string>}>}>"
    };
    let discovery_members = if discovery {
        "\n    /** The compiled ceiling for one discovery document response (about a mebibyte). */\n    private const DISCOVERY_MAX_BYTES = 1048576;\n\n    /** @var array<string, array{token_endpoint: string|null, revocation_endpoint: string|null, introspection_endpoint: string|null}> Successful discovery documents, cached per scheme for this instance's lifetime; a failed fetch is never cached, so the next call retries. */\n    private array $discovered = [];\n    /** @var array<string, bool> Single-flight gates for in-progress discovery fetches, keyed by scheme. */\n    private array $discoveryInflight = [];\n"
    } else {
        ""
    };
    // The replaying provider's served-attach record and coordinated-refresh
    // rounds join the instance members only when the wrapper participates.
    let replay_members = if replay {
        "\n    /** @var list<array{scheme: string, value: string, eligible: bool, client_id: string|null, client_secret: string|null, scope: string|null}> One replaying provider's served attaches, most recent first. The record keeps only safe metadata plus the attach identity; token values already traveled on the wire. */\n    private array $replayServed = [];\n    /** @var array<string, bool> Single-flight gates for coordinated replay refresh rounds, keyed by exact store key. */\n    private array $replayRounds = [];\n"
    } else {
        ""
    };
    let mut out = String::new();
    out.push_str("<?php\ndeclare(strict_types=1);\n\n");
    out.push_str(&format!("namespace {namespace};\n\n"));
    out.push_str(HEADER_PREFIX);
    out.push_str(if discovery {
        HEADER_DISCOVERY
    } else {
        HEADER_PLAIN
    });
    out.push_str(HEADER_SUFFIX);
    out.push_str(TYPES);
    out.push_str(if authorization_code {
        TRANSACTION
    } else {
        ""
    });
    out.push_str(if device { GRANT } else { "" });
    out.push_str(&format!(
        "final class OAuth\n{{\n    /** Compiled scheme descriptors: generation-time constants, never parsed source documents.\n     *\n     * @var {descriptor_type}\n     */\n    private const SCHEMES = [\n{table}    ];\n\n    private TokenStore $store;\n    private Transport $transport;\n    /** @var \\Closure(): int */\n    private \\Closure $clock;\n    /** @var \\Closure(int): void */\n    private \\Closure $sleep;\n    /** @var array<string, bool> Single-flight gates, keyed by exact store key. */\n    private array $inflight = [];\n    /** @var \\SplObjectStorage<AuthorizationTransaction, null>|null Consumed authorization-code transactions. */\n    private ?\\SplObjectStorage $consumed = null;{discovery_members}{replay_members}\n\n"
    ));
    out.push_str(CORE);
    out.push_str(if discovery { CC_DISCOVERY } else { CC });
    out.push_str(CREDENTIAL);
    out.push_str(if discovery {
        REFRESH_DISCOVERY
    } else {
        REFRESH
    });
    out.push_str("\n    // Conditional lifecycle helpers: exactly the flows and configured\n    // supplemental endpoints compiled above participate.\n");
    if authorization_code {
        out.push_str(AUTHORIZATION_CODE);
    }
    if device {
        out.push_str(DEVICE);
    }
    if has_revocation(&schemes) {
        out.push_str(if discovery {
            REVOCATION_DISCOVERY
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
    // bytes. The wrapper class closes the file after the lifecycle class.
    if replay {
        out.push_str(&replay_section(&schemes, &no_replay, discovery));
    }
    out.push_str("}\n");
    if replay {
        out.push_str(REPLAY_TRANSPORT_CLASS);
    }
    Some(out)
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow; its plain
/// and discovery variants resolve the wrapped token endpoint through the
/// same compiled precedence as the attach path they wrap.
fn replay_section(
    schemes: &[&OAuthSchemePlan],
    no_replay: &BTreeMap<String, BTreeSet<String>>,
    discovery: bool,
) -> String {
    let mut code = String::from(
        "\n    // The replaying credential wrapper: opt-in per provider, one\n    // coordinated refresh plus one eligible replay per qualifying 401, and\n    // never for stream-protected operations.\n\n    /** Compiled stream-protected requirements: security-requirement source pointers whose attaches are never replayed, because delivered stream data prevents a transparent restart. */\n    private const REPLAY_NO_REPLAY = [\n",
    );
    for scheme in schemes {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| format!("                {},\n", php(pointer)))
            .collect::<String>();
        code.push_str(&format!(
            "        {} => [\n{}        ],\n",
            php(&scheme.name),
            rendered
        ));
    }
    code.push_str("    ];\n\n");
    code.push_str(REPLAY_CREDENTIALS);
    code.push_str(REPLAY_REFRESH);
    code.push_str(if discovery {
        REPLAY_DISCOVERY
    } else {
        REPLAY_PLAIN
    });
    code
}

/// The emitted file header, preceding the value types: byte-exact for plans
/// without a discovery URL, with the discovery paragraph for plans with one.
const HEADER_PREFIX: &str = r#"/**
 * Generated OAuth 2.0 lifecycle for this package's compiled OAuth schemes.
 *
 * Every endpoint, client-authentication style, scope set and policy constant
 * below is a generation-time compilation of the used security schemes in the
 * source document plus the explicitly configured supplements. This class never
"#;
const HEADER_PLAIN: &str = r#" * parses OpenAPI, never fetches discovery documents and never invents an
 * endpoint. The deprecated implicit and password flows stay described in the
"#;
const HEADER_DISCOVERY: &str = r#" * parses OpenAPI; endpoint URLs the compiled flows omit resolve through RFC
 * 8414 / OpenID Connect discovery when the scheme compiles a discovery URL.
 * The deprecated implicit and password flows stay described in the
"#;
const HEADER_SUFFIX: &str = r#" * compiled descriptors but are never executed.
 *
 * Token requests are form-encoded (RFC 6749) over the package's injectable
 * Transport seam, so caller transport policy covers the lifecycle too. Client
 * identity resolves from explicit arguments first and otherwise from the
 * compiled environment variable names, read at call time; credential values
 * are never baked into generated bytes and never appear in exception
 * messages. Token sets live only in the token store owned by the OAuth
 * instance (or a store explicitly supplied by the caller), under keys
 * partitioned by scheme, token-endpoint issuer and client identity; there is
 * no process-global token cache.
 */
"#;

/// The always-emitted value types: token set, store interface, memory store
/// and the typed failure.
const TYPES: &str = r#"/** One acquired token set. Values never appear in messages, casts or debug output. */
final readonly class TokenSet
{
    public function __construct(
        #[\SensitiveParameter] public string $accessToken,
        public string $tokenType = 'Bearer',
        /** Epoch second at which the access token expires; null never expires. Freshness checks apply the compiled skew. */
        public ?int $expiresAt = null,
        #[\SensitiveParameter] public ?string $refreshToken = null,
        public ?string $scope = null,
    ) {
        if ($accessToken === '' || strlen($accessToken) > 8192 || preg_match('/[\x00-\x1f\x7f]/', $accessToken) === 1) {
            throw new \InvalidArgumentException('TokenSet requires a nonempty access token without control characters');
        }
    }

    /** Whether the set outlives `now` by more than the compiled skew. */
    public function fresh(int $now, int $skewSeconds): bool
    {
        return $this->expiresAt === null || $this->expiresAt - $skewSeconds > $now;
    }

    /** @return array<string, string> */
    public function __debugInfo(): array
    {
        return ['tokenSet' => '[redacted]'];
    }
}

/** Caller-implementable token persistence. Keys partition stored token sets by scheme, token-endpoint issuer and client identity; values are whole token sets replaced atomically. */
interface TokenStore
{
    public function load(string $key): ?TokenSet;
    public function replace(string $key, TokenSet $tokenSet): void;
    public function clear(string $key): void;
}

/** In-process token store owned by the OAuth instance or caller that created it; nothing keeps a global store. */
final class MemoryTokenStore implements TokenStore
{
    /** @var array<string, TokenSet> */
    private array $tokens = [];

    public function load(string $key): ?TokenSet
    {
        return $this->tokens[$key] ?? null;
    }

    public function replace(string $key, TokenSet $tokenSet): void
    {
        $this->tokens[$key] = $tokenSet;
    }

    public function clear(string $key): void
    {
        unset($this->tokens[$key]);
    }
}

/** Typed OAuth lifecycle failure. Messages and fields carry only safe metadata: kind, scheme, status, the server's machine error code and a retry hint; never token or client-secret values. */
final class AuthException extends \RuntimeException
{
    public function __construct(
        public readonly string $kind,
        public readonly string $scheme,
        string $message,
        public readonly ?int $status = null,
        public readonly ?string $serverError = null,
        public readonly ?int $retryAfterSeconds = null,
        ?\Throwable $previous = null,
    ) {
        parent::__construct($message, 0, $previous);
    }
}

"#;

/// One bound authorization-code transaction.
const TRANSACTION: &str = r#"/** One bound authorization-code transaction: session-scoped and consumed exactly once by completeAuthorization, whether the exchange succeeds or fails. */
final readonly class AuthorizationTransaction
{
    public function __construct(
        public string $scheme,
        /** The exact redirect target, carrying response type, client id, redirect URI, state and the PKCE S256 challenge. */
        public string $authorizationUrl,
        public string $state,
        #[\SensitiveParameter] public string $codeVerifier,
        public string $codeChallenge,
        public string $redirectUri,
        public string $tokenUrl,
        public int $createdAt,
    ) {}
}

"#;

/// One device grant value.
const GRANT: &str = r#"/** One device-authorization grant from the declared endpoint (RFC 8628). The device code never appears in debug output. */
final readonly class DeviceGrant
{
    public function __construct(
        public string $scheme,
        #[\SensitiveParameter] public string $deviceCode,
        public string $userCode,
        public string $verificationUri,
        public ?string $verificationUriComplete,
        public ?int $expiresAt,
        public int $intervalSeconds,
    ) {}
}

"#;

/// The always-emitted OAuth class core: descriptor lookup and the shared
/// request plumbing used by every lifecycle method.
const CORE: &str = r#"    /**
     * @param TokenStore|null $store Instance-owned by default; pass an explicit store to share it deliberately.
     * @param Transport|null $transport Injectable HTTP transport for token endpoint requests.
     * @param \Closure(): int|null $clock Epoch-seconds clock; injectable for tests.
     * @param \Closure(int): void|null $sleep Polling sleeper; injectable for tests.
     */
    public function __construct(
        ?TokenStore $store = null,
        ?Transport $transport = null,
        ?\Closure $clock = null,
        ?\Closure $sleep = null,
        private int $timeoutMilliseconds = 30000,
    ) {
        $this->store = $store ?? new MemoryTokenStore();
        $this->transport = $transport ?? new CurlTransport();
        $this->clock = $clock ?? static fn (): int => time();
        $this->sleep = $sleep ?? static function (int $seconds): void { sleep($seconds); };
        if ($this->timeoutMilliseconds < 1 || $this->timeoutMilliseconds > 2147483647) { throw new SdkError('configuration', 'invalid OAuth token timeout'); }
    }

    /** @return array<string, mixed> */
    private function scheme(string $name): array
    {
        $scheme = self::SCHEMES[$name] ?? null;
        if ($scheme === null) { throw new AuthException('unknown-scheme', $name, 'no compiled OAuth scheme carries that name; OAuth compiles exactly the source-declared schemes with executable flows'); }
        return $scheme;
    }

    /** @param array<string, mixed> $scheme
     * @return array<string, mixed>
     */
    private function executableFlow(array $scheme, string $kind): array
    {
        $flow = $scheme['flows'][$kind] ?? null;
        if (!is_array($flow) || $flow['deprecated'] === true) { throw new AuthException('unsupported-flow', is_string($scheme['name']) ? $scheme['name'] : $scheme, "scheme has no executable {$kind} flow in its source declaration"); }
        return $flow;
    }

    /**
     * The refresh pair: the first executable flow carrying a declared refresh
     * URL, else the first executable flow carrying a token URL, paired with
     * the endpoint that serves refreshes.
     *
     * @param array<string, mixed> $scheme
     * @return array{string, array<string, mixed>}
     */
    private function refreshEndpoint(array $scheme): array
    {
        foreach (['refresh_url', 'token_url'] as $field) {
            foreach (['authorization-code', 'client-credentials', 'device-authorization'] as $kind) {
                $flow = $scheme['flows'][$kind] ?? null;
                if (!is_array($flow) || $flow['deprecated'] === true) { continue; }
                $url = $flow[$field] ?? null;
                if (is_string($url) && $url !== '') { return [$url, $flow]; }
            }
        }
        throw new AuthException('endpoint-unavailable', is_string($scheme['name'] ?? null) ? $scheme['name'] : $scheme, 'the compiled scheme carries no executable flow with a refresh or token URL');
    }

    /** Reads one compiled environment variable name; values are read at request time, never at generation time. */
    private static function environment(?string $variable): ?string
    {
        if ($variable === null) { return null; }
        $value = getenv($variable);
        return is_string($value) && $value !== '' ? $value : null;
    }

    /** Explicit arguments win; compiled environment variable names resolve at call time.
     * @param array<string, mixed> $scheme
     * @return array{string|null, string|null}
     */
    private function identity(array $scheme, ?string $clientId, ?string $clientSecret): array
    {
        $idVariable = $scheme['client_id_env'] ?? null;
        $secretVariable = $scheme['client_secret_env'] ?? null;
        return [
            $clientId ?? self::environment(is_string($idVariable) ? $idVariable : null),
            $clientSecret ?? self::environment(is_string($secretVariable) ? $secretVariable : null),
        ];
    }

    /** RFC 6749 2.3.1 Basic credentials for confidential clients; the public profile never sends a secret.
     * @param array<string, mixed> $flow
     */
    private function basicAuth(array $flow, string $scheme, ?string $clientId, ?string $clientSecret): ?string
    {
        if (($flow['client_auth'] ?? 'none') !== 'client-secret-basic') { return null; }
        if ($clientId === null || $clientId === '' || $clientSecret === null || $clientSecret === '') {
            throw new AuthException('missing-client-credentials', $scheme, 'the compiled client authentication is client-secret-basic and no complete client identity is available');
        }
        return base64_encode(rawurlencode($clientId) . ':' . rawurlencode($clientSecret));
    }

    /** The exact token-store key for one scheme, token-endpoint issuer and client identity. */
    private static function storeKey(string $scheme, string $issuer, ?string $clientId): string
    {
        return $scheme . '|' . $issuer . '|' . ($clientId ?? 'public');
    }

    private static function controlCharacters(string $value): bool
    {
        return preg_match('/[\x00-\x1f\x7f]/', $value) === 1;
    }

    /** Maps one rejected endpoint response to the typed error; messages carry only safe metadata. */
    private static function serverFailure(string $scheme, ?string $serverError, int $status, ?int $retryAfterSeconds): AuthException
    {
        $kind = match ($serverError) {
            'invalid_request' => 'invalid-request',
            'invalid_client' => 'invalid-client',
            'invalid_grant' => 'invalid-grant',
            'unauthorized_client' => 'unauthorized-client',
            'unsupported_grant_type' => 'unsupported-grant-type',
            'invalid_scope' => 'invalid-scope',
            'authorization_pending' => 'authorization-pending',
            'slow_down' => 'slow-down',
            'expired_token' => 'device-code-expired',
            default => 'server-error',
        };
        $label = $serverError === null ? "HTTP {$status}" : "{$serverError}, HTTP {$status}";
        return new AuthException($kind, $scheme, "the authorization server rejected the request ({$label})", $status, $serverError, $retryAfterSeconds);
    }

    /** One bounded, form-encoded endpoint POST with the compiled client authentication; a non-2xx response becomes the typed error.
     * @param array<string, string|null> $fields
     */
    private function postForm(string $scheme, string $endpoint, ?string $basic, ?string $bodyClientId, array $fields): HttpResponse
    {
        $fields['client_id'] = $bodyClientId;
        $fields = array_filter($fields, static fn (string|null $value): bool => $value !== null);
        $headers = ['content-type' => 'application/x-www-form-urlencoded', 'accept' => 'application/json'];
        if ($basic !== null) { $headers['authorization'] = 'Basic ' . $basic; }
        $request = new HttpRequest('POST', $endpoint, $headers, http_build_query($fields, '', '&', PHP_QUERY_RFC3986), $this->timeoutMilliseconds, RuntimeConfig::MAX_RESPONSE_BYTES, RuntimeConfig::MAX_HEADER_BYTES, RuntimeConfig::MAX_CAPTURE_BYTES, new CallControl(static function (): void {}));
        try {
            $response = $this->transport->send($request);
        } catch (AuthException $error) {
            throw $error;
        } catch (\Throwable $previous) {
            throw new AuthException('transport-failure', $scheme, 'the endpoint request failed before a response arrived', previous: $previous);
        }
        if ($response->status < 200 || $response->status > 299) {
            $retry = null;
            $raw = $response->header('retry-after');
            if (is_string($raw) && preg_match('/\A[0-9]+\z/', trim($raw)) === 1) { $retry = (int) trim($raw); }
            try { $decoded = JsonValue::parse($response->body, new JsonLimits()); } catch (\Throwable) { $decoded = null; }
            $serverError = null;
            if ($decoded !== null && $decoded->kind === JsonKind::Object) {
                $error = $decoded->asObject()['error'] ?? null;
                if ($error !== null && $error->kind === JsonKind::String && $error->asString() !== '') { $serverError = $error->asString(); }
            }
            throw self::serverFailure($scheme, $serverError, $response->status, $retry);
        }
        return $response;
    }

    /** The JSON object carried by one successful endpoint response.
     * @param array<string, string> $what
     */
    private function postObject(string $scheme, HttpResponse $response, string $what): JsonValue
    {
        try { $decoded = JsonValue::parse($response->body, new JsonLimits()); }
        catch (\Throwable $previous) { throw new AuthException('invalid-response', $scheme, "the {$what} response is not readable JSON", previous: $previous); }
        if ($decoded->kind !== JsonKind::Object) { throw new AuthException('invalid-response', $scheme, "the {$what} response is not a JSON object"); }
        return $decoded;
    }

    /** Decodes one RFC 6749 token response; a rotated refresh token is adopted, otherwise the previous one is retained. */
    private function tokenSetFrom(string $scheme, JsonValue $body, ?TokenSet $previous): TokenSet
    {
        $members = $body->asObject();
        $access = $members['access_token'] ?? null;
        if (!$access instanceof JsonValue || $access->kind !== JsonKind::String) { throw new AuthException('invalid-response', $scheme, 'the token response carries no usable access token'); }
        $accessToken = $access->asString();
        if ($accessToken === '' || self::controlCharacters($accessToken)) { throw new AuthException('invalid-response', $scheme, 'the token response carries no usable access token'); }
        $type = 'Bearer';
        $declaredType = $members['token_type'] ?? null;
        if ($declaredType !== null && $declaredType->kind === JsonKind::String && trim($declaredType->asString()) !== '') { $type = trim($declaredType->asString()); }
        if (preg_match("/\A[A-Za-z0-9!#$%&'*+.^_`|~-]+\z/", $type) !== 1) { throw new AuthException('invalid-response', $scheme, 'the token type is not a usable authorization scheme'); }
        $expiresAt = null;
        $expiresIn = $members['expires_in'] ?? null;
        if ($expiresIn !== null && $expiresIn->kind === JsonKind::Number) {
            $seconds = $expiresIn->asNumber();
            if ($seconds->isInteger() && $seconds->compare(JsonNumber::fromInt(0)) > 0) { $expiresAt = ($this->clock)() + $seconds->toInt(); }
        }
        $refresh = null;
        $declaredRefresh = $members['refresh_token'] ?? null;
        if ($declaredRefresh !== null && $declaredRefresh->kind === JsonKind::String && $declaredRefresh->asString() !== '') { $refresh = $declaredRefresh->asString(); }
        if ($refresh === null) { $refresh = $previous?->refreshToken; }
        $scope = null;
        $declaredScope = $members['scope'] ?? null;
        if ($declaredScope !== null && $declaredScope->kind === JsonKind::String && $declaredScope->asString() !== '') { $scope = $declaredScope->asString(); }
        return new TokenSet($accessToken, $type, $expiresAt, $refresh, $scope);
    }

    /** Builds the complete Authorization value for one stored token set. Values never appear in errors. */
    private static function authorizationValue(TokenSet $tokenSet, string $scheme): string
    {
        $type = trim($tokenSet->tokenType);
        if ($type === '' || preg_match("/\A[A-Za-z0-9!#$%&'*+.^_`|~-]+\z/", $type) !== 1) {
            throw new AuthException('invalid-response', $scheme, 'the token type is not a usable authorization scheme');
        }
        return $type . ' ' . $tokenSet->accessToken;
    }

    /**
     * Single-flight waiter: a caller arriving while one acquisition for this
     * store key is pending waits for it, then serves whatever it stored.
     * Bounded so a re-entrant caller can never hang forever.
     */
    private function waitFor(string $scheme, string $key): void
    {
        $polls = 0;
        while (isset($this->inflight[$key])) {
            $polls += 1;
            if ($polls > 600) { throw new AuthException('oauth-single-flight', $scheme, 'another acquisition for this store key never completed'); }
            ($this->sleep)(1);
        }
    }
"#;

/// The plain client-credentials acquisition: every endpoint is a compiled
/// constant. Byte-exact with the pre-discovery emission.
const CC: &str = r#"
    /**
     * Returns the scheme's cached token set, acquiring one from the compiled
     * client-credentials endpoint when the stored set is absent or expired
     * beyond the compiled skew. One acquisition runs per store key at a time
     * (single-flight): a caller waiting on the gate re-checks the store
     * instead of issuing a duplicate token request, and the store entry is
     * replaced atomically. A rotated refresh token from the response is
     * adopted; when the response carries none and a stored set exists, the
     * previous refresh token is retained.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function clientCredentials(
        string $scheme,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?string $scope = null,
        ?TokenStore $store = null,
    ): TokenSet {
        $compiled = $this->scheme($scheme);
        $flow = $this->executableFlow($compiled, 'client-credentials');
        $tokenUrl = $flow['token_url'] ?? null;
        if (!is_string($tokenUrl) || $tokenUrl === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled client-credentials flow declares no token URL'); }
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $key = self::storeKey($scheme, $tokenUrl, $clientId);
        $store ??= $this->store;
        $stored = $store->load($key);
        if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
        if (isset($this->inflight[$key])) {
            $this->waitFor($scheme, $key);
            $stored = $store->load($key);
            if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
        }
        $this->inflight[$key] = true;
        try {
            // Re-check the store after acquiring the per-key gate.
            $stored = $store->load($key);
            if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
            $decoded = $this->postObject($scheme, $this->postForm($scheme, $tokenUrl, $this->basicAuth($flow, $scheme, $clientId, $clientSecret), ($flow['client_auth'] ?? 'none') === 'none' ? $clientId : null, [
                'grant_type' => 'client_credentials',
                'scope' => $scope,
            ]), 'token');
            $token = $this->tokenSetFrom($scheme, $decoded, $stored);
            $store->replace($key, $token);
            return $token;
        } finally {
            unset($this->inflight[$key]);
        }
    }
"#;

/// The credential attach path, byte-identical with and without discovery.
const CREDENTIAL: &str = r#"
    /**
     * The generated Client's credential attach path: pass the returned closure
     * under the scheme's source name in the `Credentials` map, e.g.
     * `new Credentials(['serviceOAuth' => $oauth->credential('serviceOAuth')])`.
     * Each protected call serves a fresh stored token, otherwise it acquires
     * one through clientCredentials().
     */
    public function credential(
        string $scheme,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?string $scope = null,
    ): \Closure {
        return fn (CredentialRequest $request): AuthorizationCredential => new AuthorizationCredential(self::authorizationValue($this->clientCredentials($scheme, $clientId, $clientSecret, $scope), $scheme));
    }
"#;

/// The plain explicit refresh: the declared refresh URL or token URL only.
/// Byte-exact with the pre-discovery emission.
const REFRESH: &str = r#"
    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL. A rotated refresh token from the response is
     * adopted; the given one is retained otherwise. The store entry is
     * replaced atomically when `$store` is supplied.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function refresh(
        string $scheme,
        TokenSet $tokenSet,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?TokenStore $store = null,
    ): TokenSet {
        if ($tokenSet->refreshToken === null || $tokenSet->refreshToken === '' || self::controlCharacters($tokenSet->refreshToken)) { throw new AuthException('no-refresh-token', $scheme, 'the given token set carries no usable refresh token'); }
        $compiled = $this->scheme($scheme);
        [$endpoint, $flow] = $this->refreshEndpoint($compiled);
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $decoded = $this->postObject($scheme, $this->postForm($scheme, $endpoint, $this->basicAuth($flow, $scheme, $clientId, $clientSecret), ($flow['client_auth'] ?? 'none') === 'none' ? $clientId : null, [
            'grant_type' => 'refresh_token',
            'refresh_token' => $tokenSet->refreshToken,
        ]), 'token');
        $refreshed = $this->tokenSetFrom($scheme, $decoded, $tokenSet);
        if ($store !== null) { $store->replace(self::storeKey($scheme, $endpoint, $clientId), $refreshed); }
        return $refreshed;
    }
"#;

/// The discovery-aware client-credentials acquisition: endpoint resolution
/// follows the compiled precedence (the compiled flow's token URL always
/// wins; otherwise the cached discovery document).
const CC_DISCOVERY: &str = r#"
    /**
     * Returns the scheme's cached token set, acquiring one from the compiled
     * client-credentials endpoint when the stored set is absent or expired
     * beyond the compiled skew. One acquisition runs per store key at a time
     * (single-flight): a caller waiting on the gate re-checks the store
     * instead of issuing a duplicate token request, and the store entry is
     * replaced atomically. A rotated refresh token from the response is
     * adopted; when the response carries none and a stored set exists, the
     * previous refresh token is retained.
     *
     * Endpoint resolution follows the compiled precedence: the compiled
     * client-credentials flow's token URL always wins; otherwise, when the
     * scheme compiles a discovery URL, the discovery document's
     * `token_endpoint` resolves the request (fetched once per scheme and
     * cached for this instance's lifetime, single-flighted across concurrent
     * callers, with a failed fetch retried on the next call); otherwise the
     * typed endpoint-unavailable refusal stands.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function clientCredentials(
        string $scheme,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?string $scope = null,
        ?TokenStore $store = null,
    ): TokenSet {
        $compiled = $this->scheme($scheme);
        $flow = $this->executableFlowOrNull($compiled, 'client-credentials');
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $basic = $flow === null
            ? $this->discoveryClientAuth($compiled, $scheme, $clientId, $clientSecret)
            : $this->basicAuth($flow, $scheme, $clientId, $clientSecret);
        $tokenUrl = $this->resolveEndpoint($scheme, $flow === null ? null : (is_string($flow['token_url'] ?? null) ? $flow['token_url'] : null), 'token_endpoint');
        $key = self::storeKey($scheme, $tokenUrl, $clientId);
        $store ??= $this->store;
        $stored = $store->load($key);
        if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
        if (isset($this->inflight[$key])) {
            $this->waitFor($scheme, $key);
            $stored = $store->load($key);
            if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
        }
        $this->inflight[$key] = true;
        try {
            // Re-check the store after acquiring the per-key gate.
            $stored = $store->load($key);
            if ($stored !== null && $stored->fresh(($this->clock)(), $compiled['skew'])) { return $stored; }
            $decoded = $this->postObject($scheme, $this->postForm($scheme, $tokenUrl, $basic, $basic === null ? $clientId : null, [
                'grant_type' => 'client_credentials',
                'scope' => $scope,
            ]), 'token');
            $token = $this->tokenSetFrom($scheme, $decoded, $stored);
            $store->replace($key, $token);
            return $token;
        } finally {
            unset($this->inflight[$key]);
        }
    }
"#;

/// The discovery-aware explicit refresh: the declared refresh URL, else the
/// compiled token URL, always wins; otherwise the cached discovery document's
/// token endpoint resolves the refresh.
const REFRESH_DISCOVERY: &str = r#"
    /**
     * Explicitly refreshes one token set (RFC 6749 section 6) at the declared
     * refresh URL or token URL. A rotated refresh token from the response is
     * adopted; the given one is retained otherwise. The store entry is
     * replaced atomically when `$store` is supplied.
     *
     * Endpoint resolution follows the compiled precedence: the declared
     * refresh URL, else the compiled flow's token URL, always wins; otherwise,
     * when the scheme compiles a discovery URL, the discovery document's
     * `token_endpoint` resolves the refresh.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function refresh(
        string $scheme,
        TokenSet $tokenSet,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?TokenStore $store = null,
    ): TokenSet {
        if ($tokenSet->refreshToken === null || $tokenSet->refreshToken === '' || self::controlCharacters($tokenSet->refreshToken)) { throw new AuthException('no-refresh-token', $scheme, 'the given token set carries no usable refresh token'); }
        $compiled = $this->scheme($scheme);
        [$endpoint, $flow] = $this->refreshEndpointOrNull($compiled) ?? [null, null];
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $basic = $flow === null
            ? $this->discoveryClientAuth($compiled, $scheme, $clientId, $clientSecret)
            : $this->basicAuth($flow, $scheme, $clientId, $clientSecret);
        if ($flow === null) { $endpoint = $this->resolveEndpoint($scheme, null, 'token_endpoint'); }
        $decoded = $this->postObject($scheme, $this->postForm($scheme, $endpoint, $basic, $basic === null ? $clientId : null, [
            'grant_type' => 'refresh_token',
            'refresh_token' => $tokenSet->refreshToken,
        ]), 'token');
        $refreshed = $this->tokenSetFrom($scheme, $decoded, $tokenSet);
        if ($store !== null) { $store->replace(self::storeKey($scheme, $endpoint, $clientId), $refreshed); }
        return $refreshed;
    }
"#;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the discovery document's `revocation_endpoint`.
const REVOCATION_DISCOVERY: &str = r#"
    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded). Authentication follows the compiled client policy.
     *
     * Endpoint resolution follows the compiled precedence: the configured
     * revocation endpoint always wins; otherwise, when the scheme compiles a
     * discovery URL, the discovery document's `revocation_endpoint` resolves
     * the request.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function revoke(string $scheme, string $token, ?string $tokenTypeHint = null, ?string $clientId = null, ?string $clientSecret = null): void
    {
        if ($token === '' || self::controlCharacters($token)) { throw new AuthException('invalid-request', $scheme, 'token must be a nonempty string without control characters'); }
        $compiled = $this->scheme($scheme);
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $flow = $this->refreshFlowOrNull($compiled);
        $basic = $flow === null
            ? $this->discoveryClientAuth($compiled, $scheme, $clientId, $clientSecret)
            : $this->basicAuth($flow, $scheme, $clientId, $clientSecret);
        $endpoint = $compiled['revocation'] ?? null;
        if (!is_string($endpoint) || $endpoint === '') {
            $document = $this->discover($scheme);
            $endpoint = $document['revocation_endpoint'] ?? null;
            if (!is_string($endpoint) || $endpoint === '') { throw new AuthException('endpoint-unavailable', $scheme, 'no revocation endpoint was configured for this scheme and the discovery document declares none'); }
        }
        $this->postForm($scheme, $endpoint, $basic, $basic === null ? $clientId : null, [
            'token' => $token,
            'token_type_hint' => $tokenTypeHint,
        ]);
    }
"#;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECTION_DISCOVERY: &str = r#"
    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded) and returns the server's JSON response. Authentication
     * follows the compiled client policy.
     *
     * Endpoint resolution follows the compiled precedence: the configured
     * introspection endpoint always wins; otherwise, when the scheme compiles
     * a discovery URL, the discovery document's `introspection_endpoint`
     * resolves the request.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function introspect(string $scheme, string $token, ?string $tokenTypeHint = null, ?string $clientId = null, ?string $clientSecret = null): JsonValue
    {
        if ($token === '' || self::controlCharacters($token)) { throw new AuthException('invalid-request', $scheme, 'token must be a nonempty string without control characters'); }
        $compiled = $this->scheme($scheme);
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $flow = $this->refreshFlowOrNull($compiled);
        $basic = $flow === null
            ? $this->discoveryClientAuth($compiled, $scheme, $clientId, $clientSecret)
            : $this->basicAuth($flow, $scheme, $clientId, $clientSecret);
        $endpoint = $compiled['introspection'] ?? null;
        if (!is_string($endpoint) || $endpoint === '') {
            $document = $this->discover($scheme);
            $endpoint = $document['introspection_endpoint'] ?? null;
            if (!is_string($endpoint) || $endpoint === '') { throw new AuthException('endpoint-unavailable', $scheme, 'no introspection endpoint was configured for this scheme and the discovery document declares none'); }
        }
        return $this->postObject($scheme, $this->postForm($scheme, $endpoint, $basic, $basic === null ? $clientId : null, [
            'token' => $token,
            'token_type_hint' => $tokenTypeHint,
        ]), 'introspection');
    }
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-instance cache with single-flight, and the
/// endpoint-resolution precedence.
const DISCOVERY: &str = r#"
    /** @return array<string, mixed>|null The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL; null for a scheme the discovery document defines.
     * @param array<string, mixed> $scheme
     */
    private function refreshFlowOrNull(array $scheme): ?array
    {
        foreach (['authorization-code', 'client-credentials', 'device-authorization'] as $kind) {
            $flow = $scheme['flows'][$kind] ?? null;
            if (!is_array($flow) || $flow['deprecated'] === true) { continue; }
            if (is_string($flow['token_url'] ?? null) && $flow['token_url'] !== '') { return $flow; }
        }
        return null;
    }

    /**
     * The refresh pair or null: the first executable flow carrying a declared
     * refresh URL, else the first executable flow carrying a token URL, paired
     * with the endpoint that serves refreshes; null when the scheme compiles
     * neither, so discovery may resolve the endpoint.
     *
     * @param array<string, mixed> $scheme
     * @return array{string, array<string, mixed>}|null
     */
    private function refreshEndpointOrNull(array $scheme): ?array
    {
        foreach (['refresh_url', 'token_url'] as $field) {
            foreach (['authorization-code', 'client-credentials', 'device-authorization'] as $kind) {
                $flow = $scheme['flows'][$kind] ?? null;
                if (!is_array($flow) || $flow['deprecated'] === true) { continue; }
                $url = $flow[$field] ?? null;
                if (is_string($url) && $url !== '') { return [$url, $flow]; }
            }
        }
        return null;
    }

    /**
     * Resolves one executable (non-deprecated) compiled flow, or null when the
     * scheme compiles none: a discovery-defined scheme's flows live in the
     * discovery document.
     *
     * @param array<string, mixed> $scheme
     * @return array<string, mixed>|null
     */
    private function executableFlowOrNull(array $scheme, string $kind): ?array
    {
        $flow = $scheme['flows'][$kind] ?? null;
        return is_array($flow) && $flow['deprecated'] !== true ? $flow : null;
    }

    /**
     * Client authentication for endpoints the discovery document supplies (no
     * compiled flow declares one): client-secret-basic when the compiled
     * configuration carries a client secret variable - an unavailable value
     * becomes the typed missing-client-credentials refusal - else the public
     * profile, which sends the client id in the form.
     *
     * @param array<string, mixed> $scheme
     */
    private function discoveryClientAuth(array $scheme, string $schemeName, ?string $clientId, ?string $clientSecret): ?string
    {
        if (($scheme['client_secret_env'] ?? null) === null) { return null; }
        if ($clientId === null || $clientId === '' || $clientSecret === null || $clientSecret === '') {
            throw new AuthException('missing-client-credentials', $schemeName, 'the compiled client authentication is client-secret-basic and no complete client identity is available');
        }
        return base64_encode(rawurlencode($clientId) . ':' . rawurlencode($clientSecret));
    }

    /** The origin of one absolute http(s) URL: scheme, host and the port with the scheme default made explicit; null when the value is not an absolute http(s) URL. */
    private static function discoveryOrigin(string $url): ?string
    {
        $parts = parse_url($url);
        if ($parts === false || !isset($parts['scheme'], $parts['host'])) { return null; }
        $scheme = strtolower($parts['scheme']);
        if ($scheme !== 'http' && $scheme !== 'https') { return null; }
        $port = $parts['port'] ?? ($scheme === 'http' ? 80 : 443);
        return $scheme . '://' . strtolower($parts['host']) . ':' . $port;
    }

    /** One discovery document member: absent stays null, and a non-string or unusable value is a typed discovery failure. Unknown members are ignored.
     * @param array<string, mixed> $document
     */
    private static function discoveredEndpoint(string $scheme, array $document, string $member): ?string
    {
        $value = $document[$member] ?? null;
        if ($value === null) { return null; }
        if (!$value instanceof JsonValue || $value->kind !== JsonKind::String) {
            throw new AuthException('discovery-failed', $scheme, "the discovery document carries an unusable {$member} value");
        }
        $text = $value->asString();
        if ($text === '' || self::controlCharacters($text)) {
            throw new AuthException('discovery-failed', $scheme, "the discovery document carries an unusable {$member} value");
        }
        return $text;
    }

    /**
     * Decodes and validates one discovery response body into the endpoints
     * this class resolves. The exact issuer rule: when the document carries an
     * `issuer` claim, it must be an absolute http(s) URL whose origin (scheme,
     * host and the port with the scheme default made explicit) equals the
     * discovery URL's origin; OpenID Connect openIdConnectUrl documents are
     * validated against their `issuer` claim exactly this way, as are RFC 8414
     * OAuth2 authorization-server metadata documents. A missing claim is
     * tolerated; a mismatching or unparseable one is a typed discovery
     * failure. Failure messages carry only safe metadata, never response body
     * text.
     *
     * @return array{token_endpoint: string|null, revocation_endpoint: string|null, introspection_endpoint: string|null}
     */
    private function discoveryDocument(string $scheme, string $url, string $body): array
    {
        try { $decoded = JsonValue::parse($body, new JsonLimits()); }
        catch (\Throwable $previous) { throw new AuthException('discovery-failed', $scheme, 'the discovery document is not readable JSON', previous: $previous); }
        if ($decoded->kind !== JsonKind::Object) { throw new AuthException('discovery-failed', $scheme, 'the discovery document is not a JSON object'); }
        $document = $decoded->asObject();
        $issuer = $document['issuer'] ?? null;
        if ($issuer !== null && $issuer->kind === JsonKind::String && $issuer->asString() !== '') {
            $issuerOrigin = self::discoveryOrigin($issuer->asString());
            $discoveryOrigin = self::discoveryOrigin($url);
            if ($issuerOrigin === null || $discoveryOrigin === null || $issuerOrigin !== $discoveryOrigin) {
                throw new AuthException('discovery-failed', $scheme, 'the discovery document issuer does not share the discovery URL origin');
            }
        }
        return [
            'token_endpoint' => self::discoveredEndpoint($scheme, $document, 'token_endpoint'),
            'revocation_endpoint' => self::discoveredEndpoint($scheme, $document, 'revocation_endpoint'),
            'introspection_endpoint' => self::discoveredEndpoint($scheme, $document, 'introspection_endpoint'),
        ];
    }

    /**
     * Single-flight waiter: a caller arriving while one discovery fetch for
     * this scheme is pending waits for it, then serves whatever it cached.
     * Bounded so a re-entrant caller can never hang forever.
     */
    private function waitForDiscovery(string $scheme): void
    {
        $polls = 0;
        while (isset($this->discoveryInflight[$scheme])) {
            $polls += 1;
            if ($polls > 600) { throw new AuthException('oauth-single-flight', $scheme, 'another discovery fetch for this scheme never completed'); }
            ($this->sleep)(1);
        }
    }

    /**
     * Fetches the scheme's discovery document (GET, `accept: application/json`)
     * over the package's injectable Transport seam, returning the cached
     * document when one exists for this instance. The response is bounded at
     * the compiled ceiling; a failed fetch is never cached, so the next call
     * retries, and concurrent callers share the one in-flight fetch
     * (single-flight).
     *
     * @return array{token_endpoint: string|null, revocation_endpoint: string|null, introspection_endpoint: string|null}
     * @throws AuthException Typed 'discovery-failed' failure; messages never carry response body text.
     */
    private function discover(string $scheme): array
    {
        $compiled = $this->scheme($scheme);
        $url = $compiled['discovery'] ?? null;
        if (!is_string($url) || $url === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery'); }
        if (isset($this->discovered[$scheme])) { return $this->discovered[$scheme]; }
        if (isset($this->discoveryInflight[$scheme])) {
            $this->waitForDiscovery($scheme);
            if (isset($this->discovered[$scheme])) { return $this->discovered[$scheme]; }
        }
        $this->discoveryInflight[$scheme] = true;
        try {
            // Fetch after acquiring the per-scheme gate.
            $document = $this->fetchDiscovery($scheme, $url);
            $this->discovered[$scheme] = $document;
            return $document;
        } finally {
            unset($this->discoveryInflight[$scheme]);
        }
    }

    /** One bounded discovery GET over the transport; a non-2xx or oversized response is a typed discovery failure.
     * @return array{token_endpoint: string|null, revocation_endpoint: string|null, introspection_endpoint: string|null}
     */
    private function fetchDiscovery(string $scheme, string $url): array
    {
        $request = new HttpRequest('GET', $url, ['accept' => 'application/json'], null, $this->timeoutMilliseconds, self::DISCOVERY_MAX_BYTES, RuntimeConfig::MAX_HEADER_BYTES, RuntimeConfig::MAX_CAPTURE_BYTES, new CallControl(static function (): void {}));
        try {
            $response = $this->transport->send($request);
        } catch (AuthException $error) {
            throw $error;
        } catch (\Throwable $previous) {
            throw new AuthException('discovery-failed', $scheme, 'the discovery document request failed before a response arrived', previous: $previous);
        }
        if ($response->status < 200 || $response->status > 299) {
            throw new AuthException('discovery-failed', $scheme, "the discovery document request answered HTTP {$response->status}", $response->status);
        }
        if (strlen($response->body) > self::DISCOVERY_MAX_BYTES) {
            throw new AuthException('discovery-failed', $scheme, 'the discovery document exceeds the compiled response ceiling');
        }
        return $this->discoveryDocument($scheme, $url, $response->body);
    }

    /**
     * Resolves one lifecycle endpoint through the compiled precedence: an
     * explicit compiled endpoint always wins; otherwise the cached discovery
     * document's endpoint when the scheme compiles a discovery URL; otherwise
     * the typed endpoint-unavailable refusal the compiled plan alone would
     * produce.
     */
    private function resolveEndpoint(string $scheme, ?string $compiled, string $member): string
    {
        if ($compiled !== null && $compiled !== '') { return $compiled; }
        $document = $this->discover($scheme);
        $found = $document[$member] ?? null;
        if (!is_string($found) || $found === '') { throw new AuthException('endpoint-unavailable', $scheme, "neither the compiled plan nor the discovery document carries a usable {$member} for this scheme"); }
        return $found;
    }
"#;

/// Authorization-code + PKCE S256, emitted only when a compiled scheme carries
/// an executable authorization-code flow.
const AUTHORIZATION_CODE: &str = r#"
    /**
     * Starts one authorization-code flow with PKCE (RFC 7636 S256): generates
     * the random state and code verifier from cryptographically secure bytes,
     * computes the S256 challenge, and returns the exact authorization URL to
     * redirect to, bound to the returned transaction. No network request is
     * made.
     *
     * @param list<string>|null $scopes Requested scopes; nothing is inferred from operations.
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function beginAuthorization(
        string $scheme,
        string $redirectUri,
        ?array $scopes = null,
        ?string $clientId = null,
    ): AuthorizationTransaction {
        $compiled = $this->scheme($scheme);
        $flow = $this->executableFlow($compiled, 'authorization-code');
        $authorizationUrl = $flow['authorization_url'] ?? null;
        if (!is_string($authorizationUrl) || $authorizationUrl === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled authorization-code flow declares no authorization URL'); }
        $tokenUrl = $flow['token_url'] ?? null;
        if (!is_string($tokenUrl) || $tokenUrl === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled authorization-code flow declares no token URL'); }
        $parts = parse_url($redirectUri);
        if ($parts === false || !isset($parts['scheme'], $parts['host']) || isset($parts['fragment'])) { throw new AuthException('invalid-request', $scheme, 'redirectUri must be an absolute URI without a fragment'); }
        [$clientId] = $this->identity($compiled, $clientId, null);
        if ($clientId === null || $clientId === '') { throw new AuthException('missing-client-credentials', $scheme, 'a client id is required for the authorization-code flow; pass one or set the compiled environment variable'); }
        $state = self::base64Url(random_bytes(16));
        $codeVerifier = self::base64Url(random_bytes(32));
        $codeChallenge = self::base64Url(hash('sha256', $codeVerifier, true));
        $query = array_filter([
            'response_type' => 'code',
            'client_id' => $clientId,
            'redirect_uri' => $redirectUri,
            'state' => $state,
            'code_challenge' => $codeChallenge,
            'code_challenge_method' => 'S256',
            'scope' => $scopes === null || $scopes === [] ? null : implode(' ', $scopes),
        ], static fn (string|null $value): bool => $value !== null);
        $separator = str_contains($authorizationUrl, '?') ? '&' : '?';
        return new AuthorizationTransaction($scheme, $authorizationUrl . $separator . http_build_query($query, '', '&', PHP_QUERY_RFC3986), $state, $codeVerifier, $codeChallenge, $redirectUri, $tokenUrl, ($this->clock)());
    }

    /**
     * Completes one authorization-code transaction: validates the redirect
     * state, consumes the transaction exactly once (by any attempt, whether
     * the exchange succeeds or fails), exchanges the code at the
     * transaction's token URL with the stored verifier, and replaces the
     * stored token set atomically when `$store` is supplied. A failed
     * exchange requires beginning a new authorization.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function completeAuthorization(
        AuthorizationTransaction $transaction,
        string $code,
        string $state,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?TokenStore $store = null,
    ): TokenSet {
        $this->consumed ??= new \SplObjectStorage();
        if (isset($this->consumed[$transaction])) { throw new AuthException('transaction-consumed', $transaction->scheme, 'this authorization transaction was already consumed; begin a new authorization'); }
        $this->consumed->attach($transaction);
        if ($state !== $transaction->state) { throw new AuthException('state-mismatch', $transaction->scheme, 'the redirect state does not match the authorization transaction'); }
        if ($code === '' || self::controlCharacters($code)) { throw new AuthException('invalid-request', $transaction->scheme, 'code must be a nonempty string without control characters'); }
        $compiled = $this->scheme($transaction->scheme);
        $flow = $this->executableFlow($compiled, 'authorization-code');
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $decoded = $this->postObject($transaction->scheme, $this->postForm($transaction->scheme, $transaction->tokenUrl, $this->basicAuth($flow, $transaction->scheme, $clientId, $clientSecret), ($flow['client_auth'] ?? 'none') === 'none' ? $clientId : null, [
            'grant_type' => 'authorization_code',
            'code' => $code,
            'redirect_uri' => $transaction->redirectUri,
            'code_verifier' => $transaction->codeVerifier,
        ]), 'token');
        $token = $this->tokenSetFrom($transaction->scheme, $decoded, null);
        if ($store !== null) { $store->replace(self::storeKey($transaction->scheme, $transaction->tokenUrl, $clientId), $token); }
        return $token;
    }

    /** RFC 7636 base64url: unpadded URL-safe base64 over exact bytes. */
    private static function base64Url(string $bytes): string
    {
        return rtrim(strtr(base64_encode($bytes), '+/', '-_'), '=');
    }
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
    /**
     * Requests one device grant from the compiled device-authorization URL
     * (RFC 8628). Show the user the returned user code and verification URI,
     * then poll with pollDeviceAuthorization().
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function beginDeviceAuthorization(string $scheme, ?string $clientId = null, ?string $clientSecret = null): DeviceGrant
    {
        $compiled = $this->scheme($scheme);
        $flow = $this->executableFlow($compiled, 'device-authorization');
        $deviceUrl = $flow['device_authorization_url'] ?? null;
        if (!is_string($deviceUrl) || $deviceUrl === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled device-authorization flow declares no device-authorization URL'); }
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $basic = $this->basicAuth($flow, $scheme, $clientId, $clientSecret);
        $decoded = $this->postObject($scheme, $this->postForm($scheme, $deviceUrl, $basic, $basic === null ? $clientId : null, []), 'device-authorization');
        $members = $decoded->asObject();
        $deviceCode = $members['device_code'] ?? null;
        $userCode = $members['user_code'] ?? null;
        $verificationUri = $members['verification_uri'] ?? null;
        if (!$deviceCode instanceof JsonValue || !$userCode instanceof JsonValue || !$verificationUri instanceof JsonValue
            || $deviceCode->kind !== JsonKind::String || $deviceCode->asString() === '' || self::controlCharacters($deviceCode->asString())
            || $userCode->kind !== JsonKind::String || $userCode->asString() === ''
            || $verificationUri->kind !== JsonKind::String || $verificationUri->asString() === '') {
            throw new AuthException('invalid-response', $scheme, 'the device-authorization response carries no usable grant');
        }
        $complete = $members['verification_uri_complete'] ?? null;
        $expiresAt = null;
        $expiresIn = $members['expires_in'] ?? null;
        if ($expiresIn !== null && $expiresIn->kind === JsonKind::Number) {
            $seconds = $expiresIn->asNumber();
            if ($seconds->isInteger() && $seconds->compare(JsonNumber::fromInt(0)) > 0) { $expiresAt = ($this->clock)() + $seconds->toInt(); }
        }
        $interval = 5;
        $declaredInterval = $members['interval'] ?? null;
        if ($declaredInterval !== null && $declaredInterval->kind === JsonKind::Number) {
            $declared = $declaredInterval->asNumber();
            if ($declared->isInteger() && $declared->compare(JsonNumber::fromInt(0)) > 0) { $interval = $declared->toInt(); }
        }
        return new DeviceGrant($scheme, $deviceCode->asString(), $userCode->asString(), $verificationUri->asString(), $complete !== null && $complete->kind === JsonKind::String ? $complete->asString() : null, $expiresAt, $interval);
    }

    /**
     * Polls the compiled token URL for one device grant to completion (RFC
     * 8628 3.5): `authorization_pending` waits the declared interval and
     * retries, `slow_down` grows the interval by five seconds, any other
     * refusal is a typed failure, and polling stops once the grant's declared
     * expiry passes. The sleeper is injectable (constructor `$sleep`); the
     * resulting token set replaces the store entry atomically when `$store`
     * is supplied.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function pollDeviceAuthorization(DeviceGrant $grant, ?string $clientId = null, ?string $clientSecret = null, ?TokenStore $store = null): TokenSet
    {
        $compiled = $this->scheme($grant->scheme);
        $flow = $this->executableFlow($compiled, 'device-authorization');
        $tokenUrl = $flow['token_url'] ?? null;
        if (!is_string($tokenUrl) || $tokenUrl === '') { throw new AuthException('endpoint-unavailable', $grant->scheme, 'the compiled device-authorization flow declares no token URL'); }
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $basic = $this->basicAuth($flow, $grant->scheme, $clientId, $clientSecret);
        $interval = $grant->intervalSeconds;
        while (true) {
            if ($grant->expiresAt !== null && ($this->clock)() >= $grant->expiresAt) { throw new AuthException('device-code-expired', $grant->scheme, 'the device code expired before authorization completed'); }
            try {
                $decoded = $this->postObject($grant->scheme, $this->postForm($grant->scheme, $tokenUrl, $basic, $basic === null ? $clientId : null, [
                    'grant_type' => 'urn:ietf:params:oauth:grant-type:device_code',
                    'device_code' => $grant->deviceCode,
                ]), 'token');
                $token = $this->tokenSetFrom($grant->scheme, $decoded, null);
                if ($store !== null) { $store->replace(self::storeKey($grant->scheme, $tokenUrl, $clientId), $token); }
                return $token;
            } catch (AuthException $error) {
                if ($error->serverError === 'authorization_pending') { ($this->sleep)($interval); continue; }
                if ($error->serverError === 'slow_down') { $interval += 5; ($this->sleep)($interval); continue; }
                if ($error->serverError === 'expired_token') { throw new AuthException('device-code-expired', $grant->scheme, 'the device code expired before authorization completed'); }
                throw $error;
            }
        }
    }
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries the
/// configured endpoint.
const REVOCATION: &str = r#"
    /**
     * Revokes one token at the compiled revocation endpoint (RFC 7009,
     * form-encoded). Authentication follows the compiled client policy.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function revoke(string $scheme, string $token, ?string $tokenTypeHint = null, ?string $clientId = null, ?string $clientSecret = null): void
    {
        if ($token === '' || self::controlCharacters($token)) { throw new AuthException('invalid-request', $scheme, 'token must be a nonempty string without control characters'); }
        $compiled = $this->scheme($scheme);
        $endpoint = $compiled['revocation'] ?? null;
        if (!is_string($endpoint) || $endpoint === '') { throw new AuthException('endpoint-unavailable', $scheme, 'no revocation endpoint was configured for this scheme'); }
        [, $flow] = $this->refreshEndpoint($compiled);
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        $this->postForm($scheme, $endpoint, $this->basicAuth($flow, $scheme, $clientId, $clientSecret), ($flow['client_auth'] ?? 'none') === 'none' ? $clientId : null, [
            'token' => $token,
            'token_type_hint' => $tokenTypeHint,
        ]);
    }
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries the
/// configured endpoint.
const INTROSPECTION: &str = r#"
    /**
     * Introspects one token at the compiled introspection endpoint (RFC 7662,
     * form-encoded) and returns the server's JSON response. Authentication
     * follows the compiled client policy.
     *
     * @throws AuthException Typed lifecycle failure; messages never carry token or secret values.
     */
    public function introspect(string $scheme, string $token, ?string $tokenTypeHint = null, ?string $clientId = null, ?string $clientSecret = null): JsonValue
    {
        if ($token === '' || self::controlCharacters($token)) { throw new AuthException('invalid-request', $scheme, 'token must be a nonempty string without control characters'); }
        $compiled = $this->scheme($scheme);
        $endpoint = $compiled['introspection'] ?? null;
        if (!is_string($endpoint) || $endpoint === '') { throw new AuthException('endpoint-unavailable', $scheme, 'no introspection endpoint was configured for this scheme'); }
        [, $flow] = $this->refreshEndpoint($compiled);
        [$clientId, $clientSecret] = $this->identity($compiled, $clientId, $clientSecret);
        return $this->postObject($scheme, $this->postForm($scheme, $endpoint, $this->basicAuth($flow, $scheme, $clientId, $clientSecret), ($flow['client_auth'] ?? 'none') === 'none' ? $clientId : null, [
            'token' => $token,
            'token_type_hint' => $tokenTypeHint,
        ]), 'introspection');
    }
"#;

/// The replaying provider's opt-in attach path and transport wrapper factory,
/// shared by the plain and discovery variants. Attach behavior is exactly
/// `credential()`'s; the record and the wrapper join on top of it.
const REPLAY_CREDENTIALS: &str = r#"
    /**
     * The replaying variant of the attach path: attach behavior is exactly
     * credential()'s — single-flighted acquisition, skew-aware cache, atomic
     * store replacement — plus the unified 401 replay policy. Pass the
     * returned closure under the scheme's source name in the `Credentials`
     * map, and pass replayTransport()'s wrapper as the client's transport,
     * e.g. `new Credentials(['serviceOAuth' => $oauth->replayingCredential('serviceOAuth')])`
     * with `new Client(..., $oauth->replayTransport($transport))`.
     *
     * A 401 (and only a 401) on a request whose Authorization value this
     * provider attached triggers exactly one coordinated refresh — concurrent
     * 401s share one token request round — and exactly one replay of the
     * request with the fresh token. The second response is surfaced whatever
     * it is: a second 401 reaches the caller as the declared error. The
     * overall budget is one refresh plus one replay, never nested with other
     * retry policies (requests are not retried today). Attaches for
     * stream-protected requirements are never replayed, because delivered
     * stream data prevents a transparent restart; a refresh failure surfaces
     * as the typed AuthException instead of a replay. The plain attach path
     * keeps today's semantics: replay is this provider's opt-in only.
     */
    public function replayingCredential(
        string $scheme,
        ?string $clientId = null,
        ?string $clientSecret = null,
        ?string $scope = null,
    ): \Closure {
        return function (CredentialRequest $request) use ($scheme, $clientId, $clientSecret, $scope): AuthorizationCredential {
            [$clientId, $clientSecret] = $this->identity($this->scheme($scheme), $clientId, $clientSecret);
            $value = self::authorizationValue($this->clientCredentials($scheme, $clientId, $clientSecret, $scope), $scheme);
            $this->replayServed = [['scheme' => $scheme, 'value' => $value, 'eligible' => $this->replayEligible($scheme, $request), 'client_id' => $clientId, 'client_secret' => $clientSecret, 'scope' => $scope], ...$this->replayServed];
            if (count($this->replayServed) > 8) { $this->replayServed = array_slice($this->replayServed, 0, 8); }
            return new AuthorizationCredential($value);
        };
    }

    /**
     * Wraps `inner` with the one-refresh-one-replay 401 policy; call it once
     * per client. Token requests keep traveling through `inner` directly, so
     * construct the OAuth instance over the same transport the client gets
     * wrapped around. Streaming operations open through the wrapper without
     * ever replaying: delivered stream data prevents a transparent restart.
     */
    public function replayTransport(Transport $inner): Transport
    {
        return new ReplayTransport($inner, fn (string $presented): ?array => $this->replayServedEntry($presented), fn (array $entry, string $presented): string => $this->replayRefresh($entry, $presented), fn (string $scheme, string $url): bool => self::replayLifecycleTarget($this->scheme($scheme), $url));
    }

    /** Whether one attach may be replayed: attaches for stream-protected requirements never are, because delivered stream data prevents a transparent restart. */
    private function replayEligible(string $scheme, CredentialRequest $request): bool
    {
        $pointer = null;
        if ($request->metadata->kind === JsonKind::Object) {
            $source = $request->metadata->asObject()['source'] ?? null;
            if ($source instanceof JsonValue && $source->kind === JsonKind::Object) {
                $identity = $source->asObject()['source'] ?? null;
                if ($identity instanceof JsonValue && $identity->kind === JsonKind::Object) {
                    $declared = $identity->asObject()['pointer'] ?? null;
                    if ($declared instanceof JsonValue && $declared->kind === JsonKind::String) { $pointer = $declared->asString(); }
                }
            }
        }
        return $pointer === null || !in_array($pointer, self::REPLAY_NO_REPLAY[$scheme] ?? [], true);
    }

    /** The first eligible served attach carrying exactly the presented Authorization value.
     * @return array{scheme: string, value: string, eligible: bool, client_id: string|null, client_secret: string|null, scope: string|null}|null
     */
    private function replayServedEntry(string $presented): ?array
    {
        foreach ($this->replayServed as $entry) {
            if ($entry['value'] === $presented && $entry['eligible']) { return $entry; }
        }
        return null;
    }
"#;

/// The coordinated refresh: a newer stored set wins over a stale re-refresh
/// and concurrent 401s share one round through the store key's gate, exactly
/// like the acquisition single-flight above.
const REPLAY_REFRESH: &str = r#"
    /**
     * One coordinated refresh: concurrent 401s share one store round, a
     * newer stored set wins over a stale re-refresh, and the round resolves
     * to the fresh complete Authorization value.
     *
     * @param array{scheme: string, value: string, eligible: bool, client_id: string|null, client_secret: string|null, scope: string|null} $entry
     */
    private function replayRefresh(array $entry, string $presented): string
    {
        $scheme = $entry['scheme'];
        $compiled = $this->scheme($scheme);
        $key = self::storeKey($scheme, $this->replayTokenUrl($compiled, $scheme), $entry['client_id']);
        while (true) {
            $stored = $this->store->load($key);
            if ($stored !== null && self::authorizationValue($stored, $scheme) !== $presented) { return self::authorizationValue($stored, $scheme); }
            if (isset($this->replayRounds[$key])) {
                $this->waitForRound($scheme, $key);
                continue;
            }
            $this->replayRounds[$key] = true;
            try {
                // Re-check the store after acquiring the per-key round gate.
                $stored = $this->store->load($key);
                if ($stored !== null && self::authorizationValue($stored, $scheme) !== $presented) { return self::authorizationValue($stored, $scheme); }
                $this->store->clear($key);
                return self::authorizationValue($this->clientCredentials($scheme, $entry['client_id'], $entry['client_secret'], $entry['scope']), $scheme);
            } finally {
                unset($this->replayRounds[$key]);
            }
        }
    }

    /**
     * Single-flight waiter: a caller arriving while one replay refresh round
     * for this store key is pending waits for it, then re-checks the store.
     * Bounded so a re-entrant caller can never hang forever.
     */
    private function waitForRound(string $scheme, string $key): void
    {
        $polls = 0;
        while (isset($this->replayRounds[$key])) {
            $polls += 1;
            if ($polls > 600) { throw new AuthException('oauth-single-flight', $scheme, 'another replay refresh round for this store key never completed'); }
            ($this->sleep)(1);
        }
    }
"#;

/// The plain variant's endpoint resolution and lifecycle-endpoint exclusion:
/// exactly the compiled client-credentials endpoints.
const REPLAY_PLAIN: &str = r#"
    /** The compiled client-credentials token URL: exactly the endpoint the wrapped acquisition posts to.
     * @param array<string, mixed> $compiled
     */
    private function replayTokenUrl(array $compiled, string $scheme): string
    {
        $flow = $this->executableFlow($compiled, 'client-credentials');
        $tokenUrl = $flow['token_url'] ?? null;
        if (!is_string($tokenUrl) || $tokenUrl === '') { throw new AuthException('endpoint-unavailable', $scheme, 'the compiled client-credentials flow declares no token URL'); }
        return $tokenUrl;
    }

    /** Whether one request target is one of the compiled lifecycle endpoints this provider's token plumbing posts to; such requests carry no replayable bearer token, and the exact-target guard is defense in depth against replay loops.
     * @param array<string, mixed> $compiled
     */
    private static function replayLifecycleTarget(array $compiled, string $url): bool
    {
        $flow = $compiled['flows']['client-credentials'] ?? null;
        if (!is_array($flow)) { return false; }
        foreach (['token_url', 'refresh_url'] as $field) {
            if (is_string($flow[$field] ?? null) && $flow[$field] === $url) { return true; }
        }
        return false;
    }
"#;

/// The discovery variant's endpoint resolution and lifecycle-endpoint
/// exclusion: the compiled precedence (compiled token URL, else the discovery
/// document's) plus the compiled discovery URL.
const REPLAY_DISCOVERY: &str = r#"
    /** The client-credentials token endpoint through the compiled precedence: the compiled token URL when the flow compiles one, otherwise the discovery document's.
     * @param array<string, mixed> $compiled
     */
    private function replayTokenUrl(array $compiled, string $scheme): string
    {
        $flow = $this->executableFlowOrNull($compiled, 'client-credentials');
        return $this->resolveEndpoint($compiled, $flow === null ? null : (is_string($flow['token_url'] ?? null) ? $flow['token_url'] : null), 'token_endpoint');
    }

    /** Whether one request target is one of the compiled lifecycle endpoints this provider's token plumbing posts to; the compiled discovery URL joins the compiled endpoints, and the resolved token endpoint rides the exact-token match. Such requests carry no replayable bearer token, so the exact-target guard is defense in depth against replay loops.
     * @param array<string, mixed> $compiled
     */
    private static function replayLifecycleTarget(array $compiled, string $url): bool
    {
        $flow = $compiled['flows']['client-credentials'] ?? null;
        if (is_array($flow)) {
            foreach (['token_url', 'refresh_url'] as $field) {
                if (is_string($flow[$field] ?? null) && $flow[$field] === $url) { return true; }
            }
        }
        $discovery = $compiled['discovery'] ?? null;
        return is_string($discovery) && $discovery === $url;
    }
"#;

/// The replaying transport: the 401 interception half of the wrapper. The
/// replay leg travels through the caller's transport exactly once, carrying
/// the original request's limits and remaining timeout like every lifecycle
/// request.
const REPLAY_TRANSPORT_CLASS: &str = r#"/** The replaying credential's client transport: one coordinated refresh and, when the request carried the provider's token and no stream is protected on it, exactly one replay with the fresh token. Lifecycle endpoint requests are never replayed: they carry no bearer token of this provider, and the exact-target guard is defense in depth. */
final class ReplayTransport implements StreamTransport
{
    /**
     * @param Transport $inner The caller's transport; the replay leg travels through it exactly once.
     * @param \Closure(string): ?array{scheme: string, value: string, eligible: bool, client_id: string|null, client_secret: string|null, scope: string|null} $servedEntry The owning provider's served-attach lookup.
     * @param \Closure(array{scheme: string, value: string, eligible: bool, client_id: string|null, client_secret: string|null, scope: string|null}, string): string $refresh One coordinated refresh.
     * @param \Closure(string, string): bool $lifecycleTarget The compiled lifecycle-endpoint guard.
     */
    public function __construct(
        private readonly Transport $inner,
        private readonly \Closure $servedEntry,
        private readonly \Closure $refresh,
        private readonly \Closure $lifecycleTarget,
    ) {}

    public function send(HttpRequest $request): HttpResponse
    {
        $response = $this->inner->send($request);
        if ($response->status !== 401) { return $response; }
        $presented = null;
        foreach ($request->headers as $name => $value) {
            if (strtolower((string) $name) === 'authorization') { $presented = $value; break; }
        }
        if ($presented === null || $presented === '') { return $response; }
        $entry = ($this->servedEntry)($presented);
        if ($entry === null || ($this->lifecycleTarget)($entry['scheme'], $request->url)) { return $response; }
        $fresh = ($this->refresh)($entry, $presented);
        $request->check();
        $headers = [];
        foreach ($request->headers as $name => $value) {
            $key = strtolower((string) $name);
            if ($key === 'authorization') { continue; }
            $headers[$key] = $value;
        }
        $headers['authorization'] = $fresh;
        return $this->inner->send(new HttpRequest($request->method, $request->url, $headers, $request->body, $request->timeoutMilliseconds, $request->maxResponseBytes, $request->maxHeaderBytes, $request->maxCaptureBytes, new CallControl(static function (): void {})));
    }

    public function open(HttpRequest $request): StreamResponse
    {
        if ($this->inner instanceof StreamTransport) { return $this->inner->open($request); }
        throw new SdkError('transport', 'the replaying transport wraps a plain Transport; streaming operations require a StreamTransport');
    }
}

"#;
