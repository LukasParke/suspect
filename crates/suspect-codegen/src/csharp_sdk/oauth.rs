//! Emitted-only OAuth 2.0 lifecycle for the C# HTTP adapter.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome lowers into
//! one generated `src/OAuth.g.cs` file holding the `TokenSet` record, the
//! `ITokenStore` contract with its instance-owned `MemoryTokenStore`, typed
//! `AuthException` metadata, frozen per-scheme descriptors and the flow
//! helpers. Static runtime files gain nothing: with no usable compiled scheme
//! the package stays byte-identical, and schemes carrying only deprecated
//! implicit/password flows — like OpenID Connect schemes, whose flows a
//! discovery document would define — emit nothing.
//!
//! The runtime never parses OpenAPI: every endpoint is a frozen compiled
//! constant and the environment carries compiled variable names only, with
//! values read at call time. Client-credentials acquisition is skew-aware,
//! single-flight per scheme/token-endpoint/client identity with a re-check of
//! the store after the gate, and atomically replaces the stored set, adopting
//! a server-rotated refresh token. Explicit refresh targets the declared
//! refresh URL, else the flow's token URL. Revocation and introspection are
//! emitted only when the compiled scheme carries those configured endpoints.
//! Typed `AuthException` values never carry token or secret values.

use super::{
    SdkPlan,
    emit::quote,
};
use crate::http_protocol::{
    CredentialHook, OAuthClientAuth, OAuthFlowDescriptorKind, OAuthPlan, OAuthSchemeKind,
    OAuthSchemePlan, Representation,
};
use std::collections::{BTreeMap, BTreeSet};

/// The compiled flow kind names emitted into the frozen descriptors.
fn flow_kind(kind: OAuthFlowDescriptorKind) -> &'static str {
    match kind {
        OAuthFlowDescriptorKind::Implicit => "implicit",
        OAuthFlowDescriptorKind::Password => "password",
        OAuthFlowDescriptorKind::ClientCredentials => "client-credentials",
        OAuthFlowDescriptorKind::AuthorizationCode => "authorization-code",
        OAuthFlowDescriptorKind::DeviceAuthorization => "device-authorization",
    }
}

/// The compiled client-authentication names emitted into the frozen descriptors.
fn client_auth(kind: OAuthClientAuth) -> &'static str {
    match kind {
        OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
        OAuthClientAuth::None => "none",
    }
}

/// The compiled scheme kinds emitted into the frozen descriptors.
fn scheme_kind(kind: OAuthSchemeKind) -> &'static str {
    match kind {
        OAuthSchemeKind::OAuth2 => "oauth2",
        OAuthSchemeKind::OpenIdConnect => "open-id-connect",
    }
}

fn executable(scheme: &OAuthSchemePlan, kind: OAuthFlowDescriptorKind) -> bool {
    scheme
        .flows
        .iter()
        .any(|flow| flow.kind == kind && !flow.deprecated_flow)
}

/// The usable subset: schemes carrying at least one executable (non-deprecated)
/// flow or a discovery URL. Deprecated implicit/password flows are represented
/// in their scheme's frozen descriptor but never execute; a scheme with only
/// those flows and no discovery URL contributes nothing. A discovery URL makes
/// a scheme usable even with no declared flows: OpenID Connect schemes have
/// their endpoints defined by the discovery document at runtime.
pub(super) fn usable(plan: &OAuthPlan) -> Vec<&OAuthSchemePlan> {
    plan.schemes
        .iter()
        .filter(|scheme| {
            scheme.discovery.is_some() || scheme.flows.iter().any(|flow| !flow.deprecated_flow)
        })
        .collect()
}

/// Whether the plan compiles at least one scheme with an executable flow, so
/// `src/OAuth.g.cs` participates in the package.
pub(super) fn has_usable(plan: &OAuthPlan) -> bool {
    !usable(plan).is_empty()
}

/// Whether any compiled scheme lowers into an emitted `src/OAuth.g.cs`.
pub(super) fn emits(plan: &SdkPlan) -> bool {
    plan.oauth().is_some_and(has_usable)
}

fn has_revocation(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| scheme.revocation_endpoint.is_some())
}

fn has_introspection(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| scheme.introspection_endpoint.is_some())
}

fn has_device(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| executable(scheme, OAuthFlowDescriptorKind::DeviceAuthorization))
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so schemes without one compile
/// exactly the pre-replay bytes.
fn has_client_credentials(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| executable(scheme, OAuthFlowDescriptorKind::ClientCredentials))
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget. The compiled values carry the requirement's
/// full document-and-pointer source, exactly what the generated runtime reads
/// back from the attach context.
fn no_replay_requirements(
    operations: &[super::PlannedOperation],
    schemes: &[&OAuthSchemePlan],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = schemes.iter().map(|scheme| scheme.name.as_str()).collect();
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
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(format!("{}#{}", source.document().as_str(), source.pointer()));
                }
            }
        }
    }
    pointers
}

fn optional(value: &Option<String>) -> String {
    value.as_ref().map_or_else(|| "null".to_owned(), |value| quote(value))
}

/// One scheme's frozen descriptor entry, carrying exactly the compiled plan.
fn scheme_entry(scheme: &OAuthSchemePlan) -> String {
    let flows = scheme
        .flows
        .iter()
        .map(|flow| {
            let scopes = flow
                .scopes
                .iter()
                .map(|(name, description)| format!("[{}] = {}, ", quote(name), quote(description)))
                .collect::<String>();
            format!(
                "            new({}, {}, {}, {}, {}, {}, {}, new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, string>(new Dictionary<string, string>(StringComparer.Ordinal) {{ {} }})),\n",
                quote(flow_kind(flow.kind)),
                optional(&flow.authorization_url),
                optional(&flow.token_url),
                optional(&flow.refresh_url),
                optional(&flow.device_authorization_url),
                quote(client_auth(flow.client_auth)),
                flow.deprecated_flow,
                scopes,
            )
        })
        .collect::<String>();
    format!(
        "        [{}] = new({}, {}, {}, {}, {}, {}, {}, {}, new OAuthFlowDescriptor[]\n        {{\n{}        }}),\n",
        quote(&scheme.name),
        quote(&scheme.name),
        quote(scheme_kind(scheme.kind)),
        scheme.refresh_skew_seconds,
        optional(&scheme.discovery),
        optional(&scheme.revocation_endpoint),
        optional(&scheme.introspection_endpoint),
        optional(&scheme.client_id_env),
        optional(&scheme.client_secret_env),
        flows,
    )
}

/// The generated `src/OAuth.g.cs`: frozen compiled descriptors plus the native
/// lifecycle library. Called only when at least one usable scheme compiles.
/// Plans without a discovery URL assemble byte-identically to the
/// pre-discovery emission; plans with one emit the discovery-aware providers
/// and the discovery engine.
pub(super) fn module(plan: &SdkPlan) -> String {
    let Some(oauth) = plan.oauth() else {
        unreachable!("OAuth emission requires a compiled plan");
    };
    let schemes = usable(oauth);
    let discovery = schemes.iter().any(|scheme| scheme.discovery.is_some());
    let mut out = super::emit::header(plan);
    out.push_str(CORE);
    if discovery {
        out.push_str(DISCOVERY_TYPES);
    }
    out.push_str(if discovery {
        SCHEMES_DISCOVERY
    } else {
        SCHEMES_PLAIN
    });
    for scheme in &schemes {
        out.push_str(&scheme_entry(scheme));
    }
    out.push_str("    });\n\n");
    out.push_str(API);
    out.push_str(if discovery {
        CC_DISCOVERY
    } else {
        CC_PLAIN
    });
    out.push_str(if discovery {
        REFRESH_DISCOVERY
    } else {
        REFRESH_PLAIN
    });
    out.push_str(API_TAIL);
    if has_revocation(&schemes) {
        out.push_str(if discovery {
            REVOKE_DISCOVERY
        } else {
            REVOKE
        });
    }
    if has_introspection(&schemes) {
        out.push_str(if discovery {
            INTROSPECT_DISCOVERY
        } else {
            INTROSPECT
        });
    }
    if has_device(&schemes) {
        out.push_str(DEVICE);
    }
    if discovery {
        out.push_str(DISCOVERY);
    }
    if has_client_credentials(&schemes) {
        out.push_str(&replay_section(plan, &schemes));
    }
    out.push_str("}\n");
    out
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow, and its
/// plain and discovery variants resolve the refresh endpoint and the
/// lifecycle-endpoint exclusion through the same compiled precedence as the
/// provider they wrap.
fn replay_section(plan: &SdkPlan, schemes: &[&OAuthSchemePlan]) -> String {
    let no_replay = no_replay_requirements(plan.operations(), schemes);
    let discovery = schemes.iter().any(|scheme| scheme.discovery.is_some());
    let mut code = String::from(
        "\n    /// <summary>Compiled stream-protected requirements: security-requirement source pointers whose attaches are never replayed, because delivered stream data prevents a transparent restart.</summary>\n    private static readonly IReadOnlyDictionary<string, IReadOnlyList<string>> NoReplayRequirements = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, IReadOnlyList<string>>(new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal)\n    {\n",
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
            .map(|pointer| quote(pointer))
            .collect::<Vec<_>>()
            .join(", ");
        code.push_str(&format!(
            "        [{}] = new string[] {{{}}},\n",
            quote(&scheme.name),
            rendered
        ));
    }
    code.push_str("    });\n");
    code.push_str(if discovery {
        REPLAY_DISCOVERY
    } else {
        REPLAY_PLAIN
    });
    code
}

/// The frozen-descriptor header paragraph and the `OAuth` class opening:
/// byte-exact without discovery, discovery-aware otherwise.
const SCHEMES_PLAIN: &str = "/// <summary>Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; OpenID Connect schemes compile no executable flows in v1 and emit nothing. The runtime never parses OpenAPI and never fetches discovery documents.</summary>\npublic static class OAuth\n{\n    /// <summary>Compiled descriptors keyed by exact source scheme name.</summary>\n    public static readonly IReadOnlyDictionary<string, OAuthSchemeDescriptor> Schemes = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, OAuthSchemeDescriptor>(new Dictionary<string, OAuthSchemeDescriptor>(StringComparer.Ordinal)\n    {\n";
const SCHEMES_DISCOVERY: &str = "/// <summary>Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; a compiled discovery URL resolves the endpoint URLs the compiled flows omit at call time. The runtime never parses OpenAPI.</summary>\npublic static class OAuth\n{\n    /// <summary>Compiled descriptors keyed by exact source scheme name.</summary>\n    public static readonly IReadOnlyDictionary<string, OAuthSchemeDescriptor> Schemes = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, OAuthSchemeDescriptor>(new Dictionary<string, OAuthSchemeDescriptor>(StringComparer.Ordinal)\n    {\n";

/// The static core half of the generated file: the types, the typed error, the
/// token store and the request plumbing. The `OAuth` static class opens after
/// this constant and closes at the end of `module`.
const CORE: &str = r#"using global::System.Collections.Concurrent;
using global::System.Globalization;
using global::System.Net.Http.Headers;
using global::System.Security.Cryptography;
using global::System.Text;

/// <summary>One acquired token set. ExpiresAt is the epoch millisecond at which the access token expires; a response without expires_in never expires. Freshness checks additionally apply the compiled refresh skew.</summary>
public sealed record TokenSet
{
    /// <summary>The access token value.</summary>
    public required string AccessToken { get; init; }
    /// <summary>The authorization scheme token; defaults to Bearer.</summary>
    public string TokenType { get; init; } = "Bearer";
    /// <summary>Epoch milliseconds of expiry; long.MaxValue when the response declared none.</summary>
    public long ExpiresAt { get; init; } = long.MaxValue;
    /// <summary>The server-rotated refresh token, when the response carried one.</summary>
    public string? RefreshToken { get; init; }
    /// <summary>The scope string the server granted, when echoed.</summary>
    public string? Scope { get; init; }
}

/// <summary>Caller-implementable token persistence. Keys partition stored token sets by scheme, token-endpoint issuer and client identity (see OAuth.TokenStoreKey); values are whole token sets replaced atomically.</summary>
public interface ITokenStore
{
    /// <summary>Load the stored token set for one key, or null.</summary>
    Task<TokenSet?> LoadAsync(string key, CancellationToken cancellationToken = default);
    /// <summary>Atomically replace the stored token set for one key.</summary>
    Task ReplaceAsync(string key, TokenSet tokenSet, CancellationToken cancellationToken = default);
    /// <summary>Clear the stored token set for one key.</summary>
    Task ClearAsync(string key, CancellationToken cancellationToken = default);
}

/// <summary>In-process token store owned by the provider or caller that created it; the runtime never keeps a global store.</summary>
public sealed class MemoryTokenStore : ITokenStore
{
    private readonly object _gate = new();
    private readonly Dictionary<string, TokenSet> _tokens = new(StringComparer.Ordinal);
    /// <summary>Load the stored token set for one key, or null.</summary>
    public Task<TokenSet?> LoadAsync(string key, CancellationToken cancellationToken = default)
    {
        lock (_gate) { return Task.FromResult(_tokens.TryGetValue(key, out var found) ? found : null); }
    }
    /// <summary>Atomically replace the stored token set for one key.</summary>
    public Task ReplaceAsync(string key, TokenSet tokenSet, CancellationToken cancellationToken = default)
    {
        lock (_gate) { _tokens[key] = tokenSet; }
        return Task.CompletedTask;
    }
    /// <summary>Clear the stored token set for one key.</summary>
    public Task ClearAsync(string key, CancellationToken cancellationToken = default)
    {
        lock (_gate) { _tokens.Remove(key); }
        return Task.CompletedTask;
    }
}

/// <summary>Typed OAuth lifecycle failure. Messages never contain token or secret values; fields carry only safe metadata (kind, scheme, status, server error code, retry hint).</summary>
public sealed class AuthException : Exception
{
    /// <summary>The machine failure kind, mirroring RFC 6749 error codes where one applies.</summary>
    public string Kind { get; }
    /// <summary>The source scheme the failure belongs to.</summary>
    public string Scheme { get; }
    /// <summary>The rejecting HTTP status, when a response arrived.</summary>
    public int? Status { get; }
    /// <summary>The server's machine error code, when the body declared one.</summary>
    public string? ServerError { get; }
    /// <summary>The server's Retry-After hint in seconds, when declared.</summary>
    public int? RetryAfterSeconds { get; }
    /// <summary>Construct the typed failure; callers pass only safe metadata.</summary>
    public AuthException(string kind, string scheme, string message, int? status = null, string? serverError = null, int? retryAfterSeconds = null) : base(message)
    {
        Kind = kind;
        Scheme = scheme;
        Status = status;
        ServerError = serverError;
        RetryAfterSeconds = retryAfterSeconds;
    }
}

/// <summary>One compiled flow descriptor: exactly what the source declared.</summary>
public sealed record OAuthFlowDescriptor(string Kind, string? AuthorizationUrl, string? TokenUrl, string? RefreshUrl, string? DeviceAuthorizationUrl, string ClientAuth, bool Deprecated, IReadOnlyDictionary<string, string> Scopes);

/// <summary>One compiled scheme descriptor: exactly what the source declared plus the explicitly configured supplements.</summary>
public sealed record OAuthSchemeDescriptor(string Name, string Kind, int RefreshSkewSeconds, string? Discovery, string? RevocationEndpoint, string? IntrospectionEndpoint, string? ClientIdEnv, string? ClientSecretEnv, IReadOnlyList<OAuthFlowDescriptor> Flows);

/// <summary>Caller options for the OAuth flow helpers. Client identity defaults to the compiled environment variable names when arguments are omitted; names are compiled, values are read at call time.</summary>
public sealed record OAuthClientOptions
{
    /// <summary>Explicit client identifier; wins over the compiled environment variable.</summary>
    public string? ClientId { get; init; }
    /// <summary>Explicit client secret; wins over the compiled environment variable.</summary>
    public string? ClientSecret { get; init; }
    /// <summary>Explicit scope string sent with the token request; nothing is inferred from operations.</summary>
    public string? Scope { get; init; }
    /// <summary>Explicit token persistence; omitted helpers acquire without persistence because no global store exists.</summary>
    public ITokenStore? Store { get; init; }
    /// <summary>Explicit clock for freshness and expiry; defaults to the system UTC clock.</summary>
    public Func<DateTimeOffset>? Clock { get; init; }
    /// <summary>Explicit endpoint transport; defaults to a shared cookie-free, non-redirecting invoker. HttpClient and stubbed HttpMessageHandler instances stay caller-owned.</summary>
    public HttpMessageInvoker? Transport { get; init; }
    /// <summary>Explicit polling delay for the device flow; defaults to Task.Delay. Tests inject a no-op.</summary>
    public Func<int, CancellationToken, Task>? Delay { get; init; }
}

/// <summary>A bound authorization-code transaction: session-scoped and consumed exactly once by CompleteAuthorizationAsync, whether the exchange succeeds or fails.</summary>
public sealed record AuthorizationTransaction(string Scheme, string State, string CodeVerifier, string RedirectUri, string TokenUrl, long CreatedAt);

/// <summary>The authorization redirect target plus the bound transaction.</summary>
public sealed record AuthorizationBegin(string AuthorizationUrl, string State, AuthorizationTransaction Transaction);
"#;

/// The public API half of the generated file: credential providers, explicit
/// refresh and the authorization-code flow, all inside the `OAuth` static
/// class. Conditional helpers are appended after this constant by `module`.
/// The shared plumbing before the per-flow sections: store keys, gates,
/// transport, typed refusals, request plumbing and the authorization-code
/// flow. The per-flow sections append after this constant and the plain
/// variants concatenate into exactly the pre-discovery bytes.
const API: &str = r#"
    /// <summary>The exact token-store key for one scheme, token-endpoint issuer and client identity. Identical inputs always yield identical keys; stored token sets are partitioned by all three.</summary>
    public static string TokenStoreKey(string scheme, string issuer, string? clientId) => $"{scheme}|{issuer}|{clientId ?? "public"}";

    private static readonly ConcurrentDictionary<string, SemaphoreSlim> Gates = new(StringComparer.Ordinal);
    private static readonly Lazy<HttpMessageInvoker> SharedTransport = new(static () => new HttpMessageInvoker(new SocketsHttpHandler
    {
        UseCookies = false,
        AllowAutoRedirect = false,
        UseProxy = false,
        PooledConnectionLifetime = TimeSpan.FromMinutes(10),
    }), LazyThreadSafetyMode.ExecutionAndPublication);
    private static readonly HashSet<AuthorizationTransaction> ConsumedTransactions = new(ReferenceEqualityComparer.Instance);

    private static SemaphoreSlim Gate(string key) => Gates.GetOrAdd(key, static _ => new SemaphoreSlim(1, 1));
    private static HttpMessageInvoker Transport(OAuthClientOptions options) => options.Transport ?? SharedTransport.Value;
    private static long Now(OAuthClientOptions options)
    {
        Func<DateTimeOffset> clock = options.Clock ?? (static () => DateTimeOffset.UtcNow);
        return clock().ToUnixTimeMilliseconds();
    }

    private static OAuthSchemeDescriptor Compiled(string scheme) => Schemes.TryGetValue(scheme, out var found) ? found : throw new AuthException("endpoint-unavailable", scheme, "no compiled OAuth scheme carries that name; OAuth compiles exactly the source-declared schemes with executable flows");

    private static OAuthFlowDescriptor ExecutableFlow(OAuthSchemeDescriptor scheme, string kind)
    {
        foreach (var candidate in scheme.Flows)
        {
            if (candidate.Kind == kind && !candidate.Deprecated) return candidate;
        }
        throw new AuthException("endpoint-unavailable", scheme.Name, $"scheme {scheme.Name} has no executable {kind} flow in its source declaration");
    }

    /// <summary>The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL.</summary>
    private static OAuthFlowDescriptor RefreshFlow(OAuthSchemeDescriptor scheme)
    {
        OAuthFlowDescriptor? fallback = null;
        foreach (var candidate in scheme.Flows)
        {
            if (candidate.Deprecated) continue;
            if (candidate.Kind == "authorization-code") return candidate;
            if (fallback is null && candidate.TokenUrl is not null) fallback = candidate;
        }
        return fallback ?? throw new AuthException("endpoint-unavailable", scheme.Name, "the compiled scheme carries no executable flow with a token URL");
    }

    /// <summary>The refresh endpoint: the declared refresh URL, else the flow's token URL.</summary>
    private static string RefreshEndpoint(OAuthFlowDescriptor grant, OAuthSchemeDescriptor scheme) => grant.RefreshUrl ?? grant.TokenUrl ?? throw new AuthException("endpoint-unavailable", scheme.Name, "the compiled flow declares neither a refresh URL nor a token URL");

    /// <summary>Reads one compiled environment variable name at call time; generation supplied the name only.</summary>
    private static string? EnvironmentValue(string? variable)
    {
        if (variable is null) return null;
        try { return Environment.GetEnvironmentVariable(variable) is { Length: > 0 } value ? value : null; }
        catch { return null; }
    }

    /// <summary>Client identity for one request: explicit arguments win, then the compiled environment variable names.</summary>
    private static (string? Id, string? Secret) ResolveIdentity(OAuthSchemeDescriptor scheme, OAuthClientOptions options) => (options.ClientId ?? EnvironmentValue(scheme.ClientIdEnv), options.ClientSecret ?? EnvironmentValue(scheme.ClientSecretEnv));

    /// <summary>Client authentication follows the compiled per-flow policy: client-secret-basic sends HTTP Basic; none is the public profile and never sends a secret.</summary>
    private static (bool Basic, string? Id, string? Secret) ClientAuthFor(OAuthFlowDescriptor grant, (string? Id, string? Secret) identity) => (grant.ClientAuth == "client-secret-basic", identity.Id, grant.ClientAuth == "client-secret-basic" ? identity.Secret : null);

    /// <summary>Freshness applies the compiled refresh skew: a token is fresh when it outlives now by more than the skew.</summary>
    private static bool IsFresh(TokenSet tokenSet, long skewMilliseconds, long now) => tokenSet.ExpiresAt - skewMilliseconds > now;

    private static bool IsToken(string value) => value.All(static c => c is >= 'a' and <= 'z' or >= 'A' and <= 'Z' or >= '0' and <= '9' || "!#$%&'*+-.^_`|~".Contains(c));
    private static bool IsUsableSecret(string value) => value.Length > 0 && value.Length <= 8192 && !value.Any(static c => c < 32 || c == 127);

    /// <summary>Builds the complete AuthorizationValue for one stored token set. Values never appear in errors.</summary>
    private static AuthorizationValue Authorization(TokenSet tokenSet, string scheme)
    {
        var type = tokenSet.TokenType.Trim();
        if (type.Length == 0 || !IsToken(type)) throw new AuthException("invalid-response", scheme, "the token type is not a usable authorization scheme");
        if (!IsUsableSecret(tokenSet.AccessToken)) throw new AuthException("invalid-response", scheme, "the access token is not a usable credential value");
        return new AuthorizationValue(type, tokenSet.AccessToken);
    }

    /// <summary>Atomic store replacement: a server-rotated refresh token is adopted; when the response carries none, the previous refresh token is retained.</summary>
    private static TokenSet Adopt(TokenSet? previous, TokenSet next)
    {
        if (next.RefreshToken is not null || previous is null || previous.RefreshToken is null) return next;
        return next with { RefreshToken = previous.RefreshToken };
    }

    private static string ServerErrorKind(string? serverError) => serverError switch
    {
        "invalid_request" => "invalid-request",
        "invalid_client" => "invalid-client",
        "invalid_grant" => "invalid-grant",
        "unauthorized_client" => "unauthorized-client",
        "unsupported_grant_type" => "unsupported-grant-type",
        "invalid_scope" => "invalid-scope",
        "authorization_pending" => "authorization-pending",
        "slow_down" => "slow-down",
        "expired_token" => "device-code-expired",
        _ => "server-error",
    };

    /// <summary>Maps one rejected endpoint response to the typed error; the message carries only the machine error code, status and scheme.</summary>
    private static AuthException ServerFailure(string scheme, string body, HttpResponseMessage response, int? retryAfterSeconds)
    {
        string? serverError = null;
        try
        {
            using var document = JsonDocument.Parse(body);
            if (document.RootElement.ValueKind == JsonValueKind.Object && document.RootElement.TryGetProperty("error", out var error) && error.ValueKind == JsonValueKind.String && error.GetString() is { Length: > 0 } code) serverError = code;
        }
        catch { /* An unreadable error body is safe metadata loss. */ }
        var status = (int)response.StatusCode;
        var label = serverError is null ? $"HTTP {status}" : $"{serverError}, HTTP {status}";
        return new AuthException(ServerErrorKind(serverError), scheme, $"the authorization server rejected the request ({label})", status, serverError, retryAfterSeconds);
    }

    /// <summary>One form-encoded endpoint POST with the compiled client authentication; a non-2xx response becomes the typed error.</summary>
    private static async Task<string> PostFormAsync(string scheme, string endpoint, (bool Basic, string? Id, string? Secret) auth, IReadOnlyDictionary<string, string> fields, HttpMessageInvoker transport, CancellationToken cancellationToken)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, endpoint);
        request.Headers.Accept.ParseAdd("application/json");
        if (auth.Basic)
        {
            if (auth.Secret is null) throw new AuthException("missing-credential", scheme, "the compiled client authentication is client-secret-basic and no client secret is available");
            request.Headers.Authorization = new AuthenticationHeaderValue("Basic", Convert.ToBase64String(Encoding.UTF8.GetBytes($"{auth.Id ?? string.Empty}:{auth.Secret}")));
        }
        request.Content = new FormUrlEncodedContent(fields);
        HttpResponseMessage response;
        try { response = await transport.SendAsync(request, cancellationToken).ConfigureAwait(false); }
        catch (Exception error) when (error is not OperationCanceledException) { throw new AuthException("transport-failure", scheme, "the endpoint request failed before a response arrived"); }
        using (response)
        {
            var body = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
            if ((int)response.StatusCode < 200 || (int)response.StatusCode > 299)
            {
                throw ServerFailure(scheme, body, response, response.Headers.RetryAfter?.Delta is { } delta ? (int)delta.TotalSeconds : null);
            }
            return body;
        }
    }

    private static JsonElement JsonObject(string scheme, string body, string what)
    {
        try
        {
            using var document = JsonDocument.Parse(body);
            if (document.RootElement.ValueKind == JsonValueKind.Object) return document.RootElement.Clone();
            throw new AuthException("invalid-response", scheme, $"the {what} response is not a JSON object");
        }
        catch (AuthException) { throw; }
        catch { throw new AuthException("invalid-response", scheme, $"the {what} response is not readable JSON"); }
    }

    private static TokenSet TokenSetFrom(string scheme, JsonElement body, long now)
    {
        var accessToken = body.ValueKind == JsonValueKind.Object && body.TryGetProperty("access_token", out var access) && access.ValueKind == JsonValueKind.String ? access.GetString() : null;
        if (accessToken is null || !IsUsableSecret(accessToken)) throw new AuthException("invalid-response", scheme, "the token response carries no usable access token");
        var tokenType = body.TryGetProperty("token_type", out var type) && type.ValueKind == JsonValueKind.String && type.GetString() is { Length: > 0 } declared ? declared.Trim() : "Bearer";
        var expiresAt = body.TryGetProperty("expires_in", out var expires) && expires.ValueKind == JsonValueKind.Number && expires.TryGetInt64(out var seconds) && seconds > 0 ? now + seconds * 1000 : long.MaxValue;
        var refreshToken = body.TryGetProperty("refresh_token", out var refresh) && refresh.ValueKind == JsonValueKind.String && refresh.GetString() is { Length: > 0 } rotated ? rotated : null;
        var scope = body.TryGetProperty("scope", out var scopeElement) && scopeElement.ValueKind == JsonValueKind.String && scopeElement.GetString() is { Length: > 0 } granted ? granted : null;
        return new TokenSet { AccessToken = accessToken, TokenType = tokenType, ExpiresAt = expiresAt, RefreshToken = refreshToken, Scope = scope };
    }

    private static async Task<TokenSet> TokenRequestAsync(string scheme, string endpoint, (bool Basic, string? Id, string? Secret) auth, IReadOnlyDictionary<string, string> fields, HttpMessageInvoker transport, CancellationToken cancellationToken, long now)
    {
        var body = JsonObject(scheme, await PostFormAsync(scheme, endpoint, auth, fields, transport, cancellationToken).ConfigureAwait(false), "token");
        return TokenSetFrom(scheme, body, now);
    }

    private static async Task<TokenSet> PerformRefreshAsync(OAuthSchemeDescriptor scheme, OAuthFlowDescriptor grant, string endpoint, (string? Id, string? Secret) identity, string refreshToken, TokenSet? previous, ITokenStore? store, HttpMessageInvoker transport, CancellationToken cancellationToken, long now)
    {
        var fields = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = refreshToken,
        };
        if (grant.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
        var acquired = await TokenRequestAsync(scheme.Name, endpoint, ClientAuthFor(grant, identity), fields, transport, cancellationToken, now).ConfigureAwait(false);
        var adopted = Adopt(previous, acquired);
        if (store is not null) await store.ReplaceAsync(TokenStoreKey(scheme.Name, endpoint, identity.Id), adopted, cancellationToken).ConfigureAwait(false);
        return adopted;
    }
"#;

/// The plain client-credentials acquisition and provider factory, byte-exact
/// with the pre-discovery emission.
const CC_PLAIN: &str = r#"
    /// <summary>Acquires a token with the compiled client-credentials flow: a fresh stored token is served, otherwise one form-encoded token request runs. Concurrent callers share one in-flight acquisition per scheme, token endpoint and client identity (the store is re-checked after the gate); the store is replaced atomically and a response refresh token is adopted, else a previous one retained. Without an explicit Store the call acquires without persistence, because the runtime never keeps a global store; the provider factories own one store per provider.</summary>
    public static async Task<TokenSet> ClientCredentialsTokenAsync(string scheme, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        var compiled = Compiled(scheme);
        var grant = ExecutableFlow(compiled, "client-credentials");
        var tokenUrl = grant.TokenUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled client-credentials flow declares no token URL");
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var transport = Transport(clientOptions);
        var store = clientOptions.Store;
        var key = TokenStoreKey(compiled.Name, tokenUrl, identity.Id);
        var gate = Gate(key);
        await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var stored = store is null ? null : await store.LoadAsync(key, cancellationToken).ConfigureAwait(false);
            if (stored is not null && IsFresh(stored, compiled.RefreshSkewSeconds * 1000L, Now(clientOptions))) return stored;
            var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["grant_type"] = "client_credentials" };
            if (clientOptions.Scope is not null) fields["scope"] = clientOptions.Scope;
            if (grant.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
            var acquired = await TokenRequestAsync(compiled.Name, tokenUrl, ClientAuthFor(grant, identity), fields, transport, cancellationToken, Now(clientOptions)).ConfigureAwait(false);
            var adopted = Adopt(stored, acquired);
            if (store is not null) await store.ReplaceAsync(key, adopted, cancellationToken).ConfigureAwait(false);
            return adopted;
        }
        finally
        {
            gate.Release();
        }
    }

    /// <summary>Creates an AuthorizationProvider for the compiled client-credentials flow; assign it to the scheme's Credentials property. The provider owns one token store for its lifetime (the explicit Store wins) and concurrent calls share one in-flight acquisition.</summary>
    public static AuthorizationProvider CreateClientCredentialsProvider(string scheme, OAuthClientOptions? options = null)
    {
        var name = scheme;
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        return async (context, cancellationToken) =>
        {
            _ = context;
            var tokenSet = await ClientCredentialsTokenAsync(name, clientOptions, cancellationToken).ConfigureAwait(false);
            return Authorization(tokenSet, name);
        };
    }
"#;

/// The plain refresh provider, stored-token accessor and explicit refresh,
/// byte-exact with the pre-discovery emission.
const REFRESH_PLAIN: &str = r#"
    /// <summary>Creates an AuthorizationProvider that serves stored tokens and refreshes them on demand: a fresh stored token is returned; an expired one is refreshed exactly once with its stored refresh token before serving. The provider owns one token store for its lifetime (the explicit Store wins). There is no token at all until an authorization or device flow has completed. When a protected call still fails with a declared 401, call RefreshTokenAsync explicitly and retry; the SDK itself never retries.</summary>
    public static AuthorizationProvider CreateRefreshProvider(string scheme, OAuthClientOptions? options = null)
    {
        var name = scheme;
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        return async (context, cancellationToken) =>
        {
            _ = context;
            var tokenSet = await StoredTokenAsync(name, clientOptions, cancellationToken).ConfigureAwait(false);
            return Authorization(tokenSet, name);
        };
    }

    private static async Task<TokenSet> StoredTokenAsync(string scheme, OAuthClientOptions clientOptions, CancellationToken cancellationToken)
    {
        var compiled = Compiled(scheme);
        var grant = RefreshFlow(compiled);
        var endpoint = RefreshEndpoint(grant, compiled);
        var identity = ResolveIdentity(compiled, clientOptions);
        var store = clientOptions.Store ?? new MemoryTokenStore();
        var stored = await store.LoadAsync(TokenStoreKey(compiled.Name, endpoint, identity.Id), cancellationToken).ConfigureAwait(false);
        if (stored is null) throw new AuthException("missing-credential", compiled.Name, "no stored token set exists for this scheme; complete an authorization or device flow first");
        if (IsFresh(stored, compiled.RefreshSkewSeconds * 1000L, Now(clientOptions))) return stored;
        if (stored.RefreshToken is null) throw new AuthException("missing-credential", compiled.Name, "the stored token set carries no refresh token");
        return await PerformRefreshAsync(compiled, grant, endpoint, identity, stored.RefreshToken, stored, store, Transport(clientOptions), cancellationToken, Now(clientOptions)).ConfigureAwait(false);
    }

    /// <summary>Exchanges one refresh token at the declared refresh URL or token URL (grant_type=refresh_token) and returns the token set. A rotated refresh token from the response is adopted; when the response carries none and a previous stored set exists, the previous refresh token is retained.</summary>
    public static async Task<TokenSet> RefreshTokenAsync(string scheme, string refreshToken, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(refreshToken)) throw new AuthException("invalid-request", scheme, "refreshToken must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var grant = RefreshFlow(compiled);
        var endpoint = RefreshEndpoint(grant, compiled);
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var previous = clientOptions.Store is null ? null : await clientOptions.Store.LoadAsync(TokenStoreKey(compiled.Name, endpoint, identity.Id), cancellationToken).ConfigureAwait(false);
        return await PerformRefreshAsync(compiled, grant, endpoint, identity, refreshToken, previous, clientOptions.Store, Transport(clientOptions), cancellationToken, Now(clientOptions)).ConfigureAwait(false);
    }
"#;

/// The plain shared plumbing after the refresh section, byte-exact with the
/// pre-discovery emission.
const API_TAIL: &str = r#"
    private static string Base64UrlEncode(byte[] bytes) => Convert.ToBase64String(bytes).Replace('+', '-').Replace('/', '_').TrimEnd('=');
    private static string RandomToken(int byteLength) => Base64UrlEncode(RandomNumberGenerator.GetBytes(byteLength));

    private static string QueryEscape(string value)
    {
        var builder = new StringBuilder(value.Length);
        foreach (var byteValue in Encoding.UTF8.GetBytes(value))
        {
            if (byteValue is >= (byte)'a' and <= (byte)'z' or >= (byte)'A' and <= (byte)'Z' or >= (byte)'0' and <= (byte)'9' || byteValue is (byte)'-' or (byte)'.' or (byte)'_' or (byte)'~')
            {
                builder.Append((char)byteValue);
            }
            else
            {
                builder.Append('%').Append(byteValue.ToString("X2", CultureInfo.InvariantCulture));
            }
        }
        return builder.ToString();
    }

    /// <summary>Appends form parameters to a compiled absolute endpoint without re-escaping the declared URL text.</summary>
    private static string WithQuery(string scheme, string endpoint, IReadOnlyDictionary<string, string> parameters)
    {
        var additions = string.Join("&", parameters.Select(static pair => $"{QueryEscape(pair.Key)}={QueryEscape(pair.Value)}"));
        var merged = endpoint.Contains('?') ? $"{endpoint}&{additions}" : $"{endpoint}?{additions}";
        return Uri.TryCreate(merged, UriKind.Absolute, out var parsed) ? parsed.ToString() : throw new AuthException("endpoint-unavailable", scheme, "the compiled authorization URL cannot be parsed");
    }

    /// <summary>Starts an authorization-code flow with PKCE (S256): generates the random state and code verifier with RandomNumberGenerator, computes the S256 challenge with SHA-256 and returns the exact authorization URL to redirect to plus the bound transaction. No network request is made.</summary>
    public static AuthorizationBegin BeginAuthorization(string scheme, string redirectUri, OAuthClientOptions? options = null, IEnumerable<string>? scopes = null)
    {
        var compiled = Compiled(scheme);
        var grant = ExecutableFlow(compiled, "authorization-code");
        var authorizationEndpoint = grant.AuthorizationUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled authorization-code flow declares no authorization URL");
        var tokenUrl = grant.TokenUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled authorization-code flow declares no token URL");
        if (redirectUri.Length == 0 || !Uri.TryCreate(redirectUri, UriKind.Absolute, out var redirect) || redirect.Fragment.Length > 0) throw new AuthException("invalid-request", compiled.Name, "redirectUri must be an absolute URI without a fragment");
        var identity = ResolveIdentity(compiled, options ?? new OAuthClientOptions());
        var clientId = identity.Id ?? throw new AuthException("missing-credential", compiled.Name, "a client id is required for the authorization-code flow; pass ClientId or set the compiled environment variable");
        var state = RandomToken(16);
        var codeVerifier = RandomToken(32);
        var codeChallenge = Base64UrlEncode(SHA256.HashData(Encoding.ASCII.GetBytes(codeVerifier)));
        var query = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["response_type"] = "code",
            ["client_id"] = clientId,
            ["redirect_uri"] = redirectUri,
            ["state"] = state,
            ["code_challenge"] = codeChallenge,
            ["code_challenge_method"] = "S256",
        };
        var requested = scopes is null ? string.Empty : string.Join(" ", scopes);
        if (requested.Length > 0) query["scope"] = requested;
        return new AuthorizationBegin(WithQuery(compiled.Name, authorizationEndpoint, query), state, new AuthorizationTransaction(compiled.Name, state, codeVerifier, redirectUri, tokenUrl, Now(options ?? new OAuthClientOptions())));
    }

    /// <summary>Completes one authorization-code transaction: validates the redirect state, exchanges the code at the transaction's token URL with the stored code verifier, and replaces the stored token set atomically. The transaction is consumed exactly once by any attempt; a failed exchange requires beginning a new authorization.</summary>
    public static async Task<TokenSet> CompleteAuthorizationAsync(AuthorizationTransaction transaction, string code, string state, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        lock (ConsumedTransactions)
        {
            if (!ConsumedTransactions.Add(transaction)) throw new AuthException("transaction-consumed", transaction.Scheme, "this authorization transaction was already consumed; begin a new authorization");
        }
        if (state != transaction.State) throw new AuthException("state-mismatch", transaction.Scheme, "the redirect state does not match the authorization transaction");
        if (!IsUsableSecret(code)) throw new AuthException("invalid-request", transaction.Scheme, "code must be a nonempty string without control characters");
        var compiled = Compiled(transaction.Scheme);
        var grant = ExecutableFlow(compiled, "authorization-code");
        var tokenUrl = transaction.TokenUrl.Length > 0 ? transaction.TokenUrl : throw new AuthException("endpoint-unavailable", compiled.Name, "the authorization transaction carries no token URL");
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var fields = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "authorization_code",
            ["code"] = code,
            ["redirect_uri"] = transaction.RedirectUri,
            ["code_verifier"] = transaction.CodeVerifier,
        };
        if (grant.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
        var acquired = await TokenRequestAsync(compiled.Name, tokenUrl, ClientAuthFor(grant, identity), fields, Transport(clientOptions), cancellationToken, Now(clientOptions)).ConfigureAwait(false);
        if (clientOptions.Store is not null) await clientOptions.Store.ReplaceAsync(TokenStoreKey(compiled.Name, tokenUrl, identity.Id), acquired, cancellationToken).ConfigureAwait(false);
        return acquired;
    }
"#;

/// The namespace-level discovery types, emitted after `CORE` and only when at
/// least one compiled scheme carries a discovery URL.
const DISCOVERY_TYPES: &str = r#"
/// <summary>One RFC 8414 / OpenID Connect discovery document reduced to the endpoints this module resolves. Unknown members are ignored; a present member must be a usable string.</summary>
internal sealed record DiscoveredEndpoints
{
    /// <summary>The discovered token endpoint, when the document declared one.</summary>
    public string? TokenEndpoint { get; init; }
    /// <summary>The discovered revocation endpoint, when the document declared one.</summary>
    public string? RevocationEndpoint { get; init; }
    /// <summary>The discovered introspection endpoint, when the document declared one.</summary>
    public string? IntrospectionEndpoint { get; init; }
}

/// <summary>One provider-owned discovery cache: successful documents are cached per scheme for the provider's lifetime, so repeated attaches never re-fetch; a failed fetch is never cached, so the next call retries; concurrent callers share the one in-flight fetch through a per-scheme gate (single-flight).</summary>
internal sealed class DiscoveryCache
{
    private readonly object _gate = new();
    private readonly Dictionary<string, DiscoveredEndpoints> _documents = new(StringComparer.Ordinal);
    private readonly Dictionary<string, SemaphoreSlim> _locks = new(StringComparer.Ordinal);

    internal SemaphoreSlim Gate(string scheme)
    {
        lock (_gate)
        {
            if (!_locks.TryGetValue(scheme, out var gate))
            {
                gate = new SemaphoreSlim(1, 1);
                _locks[scheme] = gate;
            }
            return gate;
        }
    }

    internal bool TryGet(string scheme, out DiscoveredEndpoints document)
    {
        lock (_gate)
        {
            if (_documents.TryGetValue(scheme, out var found))
            {
                document = found;
                return true;
            }
            document = null!;
            return false;
        }
    }

    internal void Store(string scheme, DiscoveredEndpoints document)
    {
        lock (_gate) { _documents[scheme] = document; }
    }
}
"#;

/// The discovery-aware client-credentials section: endpoint resolution follows
/// the compiled precedence (the compiled flow's token URL always wins;
/// otherwise the provider's cached discovery document).
const CC_DISCOVERY: &str = r#"
    /// <summary>Acquires a token with the compiled client-credentials flow: a fresh stored token is served, otherwise one form-encoded token request runs. Concurrent callers share one in-flight acquisition per scheme, token endpoint and client identity (the store is re-checked after the gate); the store is replaced atomically and a response refresh token is adopted, else a previous one retained. Without an explicit Store the call acquires without persistence, because the runtime never keeps a global store; the provider factories own one store per provider. Endpoint resolution follows the compiled precedence: the compiled flow's token URL wins; otherwise, when the scheme compiles a discovery URL, the discovery document's token endpoint resolves the request. This one-shot helper fetches discovery per call and keeps no cache.</summary>
    public static Task<TokenSet> ClientCredentialsTokenAsync(string scheme, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        var compiled = Compiled(scheme);
        var clientOptions = options ?? new OAuthClientOptions();
        return ClientCredentialsAsync(compiled, clientOptions, null, cancellationToken);
    }

    /// <summary>Creates an AuthorizationProvider for the compiled client-credentials flow; assign it to the scheme's Credentials property. The provider owns one token store for its lifetime (the explicit Store wins) and concurrent calls share one in-flight acquisition. Endpoint resolution follows the compiled precedence: the compiled flow's token URL wins; otherwise, when the scheme compiles a discovery URL, the discovery document cached on this provider resolves the request (fetched once per scheme for the provider's lifetime, single-flighted across concurrent callers, with a failed fetch retried on the next call).</summary>
    public static AuthorizationProvider CreateClientCredentialsProvider(string scheme, OAuthClientOptions? options = null)
    {
        var name = scheme;
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        var discovery = new DiscoveryCache();
        return async (context, cancellationToken) =>
        {
            _ = context;
            var tokenSet = await ClientCredentialsAsync(Compiled(name), clientOptions, discovery, cancellationToken).ConfigureAwait(false);
            return Authorization(tokenSet, name);
        };
    }

    /// <summary>One client-credentials acquisition over the resolved endpoint; see ClientCredentialsTokenAsync for the store and single-flight semantics and CreateClientCredentialsProvider for the provider-owned discovery cache.</summary>
    private static async Task<TokenSet> ClientCredentialsAsync(OAuthSchemeDescriptor compiled, OAuthClientOptions clientOptions, DiscoveryCache? discovery, CancellationToken cancellationToken)
    {
        var grant = ExecutableFlowOrNull(compiled, "client-credentials");
        var identity = ResolveIdentity(compiled, clientOptions);
        var auth = grant is null ? DiscoveryClientAuth(compiled, identity) : ClientAuthFor(grant, identity);
        var transport = Transport(clientOptions);
        var now = Now(clientOptions);
        var tokenUrl = await ResolveEndpointAsync(compiled, grant is null ? null : grant.TokenUrl, "token", static document => document.TokenEndpoint, clientOptions, discovery, cancellationToken).ConfigureAwait(false);
        var store = clientOptions.Store;
        var key = TokenStoreKey(compiled.Name, tokenUrl, identity.Id);
        var gate = Gate(key);
        await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var stored = store is null ? null : await store.LoadAsync(key, cancellationToken).ConfigureAwait(false);
            if (stored is not null && IsFresh(stored, compiled.RefreshSkewSeconds * 1000L, now)) return stored;
            var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["grant_type"] = "client_credentials" };
            if (clientOptions.Scope is not null) fields["scope"] = clientOptions.Scope;
            if (!auth.Basic && identity.Id is not null) fields["client_id"] = identity.Id;
            var acquired = await TokenRequestAsync(compiled.Name, tokenUrl, auth, fields, transport, cancellationToken, now).ConfigureAwait(false);
            var adopted = Adopt(stored, acquired);
            if (store is not null) await store.ReplaceAsync(key, adopted, cancellationToken).ConfigureAwait(false);
            return adopted;
        }
        finally
        {
            gate.Release();
        }
    }
"#;

/// The discovery-aware refresh section: the compiled refresh URL, else the
/// compiled token URL, always wins; otherwise the provider's cached discovery
/// document resolves the refresh.
const REFRESH_DISCOVERY: &str = r#"
    /// <summary>Creates an AuthorizationProvider that serves stored tokens and refreshes them on demand: a fresh stored token is returned; an expired one is refreshed exactly once with its stored refresh token before serving. The provider owns one token store for its lifetime (the explicit Store wins). There is no token at all until an authorization or device flow has completed. When a protected call still fails with a declared 401, call RefreshTokenAsync explicitly and retry; the SDK itself never retries. Endpoint resolution follows the compiled precedence: the declared refresh URL, else the compiled flow's token URL, always wins; otherwise, when the scheme compiles a discovery URL, the discovery document cached on this provider resolves the refresh (fetched once per scheme for the provider's lifetime, single-flighted across concurrent callers, with a failed fetch retried on the next call).</summary>
    public static AuthorizationProvider CreateRefreshProvider(string scheme, OAuthClientOptions? options = null)
    {
        var name = scheme;
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        var discovery = new DiscoveryCache();
        return async (context, cancellationToken) =>
        {
            _ = context;
            var tokenSet = await StoredTokenAsync(Compiled(name), clientOptions, discovery, cancellationToken).ConfigureAwait(false);
            return Authorization(tokenSet, name);
        };
    }

    /// <summary>Exchanges one refresh token at the declared refresh URL or token URL (grant_type=refresh_token) and returns the token set. A rotated refresh token from the response is adopted; when the response carries none and a previous stored set exists, the previous refresh token is retained. Endpoint resolution follows the compiled precedence: the declared refresh URL, else the compiled flow's token URL, always wins; otherwise, when the scheme compiles a discovery URL, the discovery document's token endpoint resolves the exchange. This one-shot helper fetches discovery per call and keeps no cache.</summary>
    public static async Task<TokenSet> RefreshTokenAsync(string scheme, string refreshToken, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(refreshToken)) throw new AuthException("invalid-request", scheme, "refreshToken must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var grant = RefreshFlowOrNull(compiled);
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var endpoint = grant is null
            ? await ResolveEndpointAsync(compiled, null, "token", static document => document.TokenEndpoint, clientOptions, null, cancellationToken).ConfigureAwait(false)
            : RefreshEndpoint(grant, compiled);
        var previous = clientOptions.Store is null ? null : await clientOptions.Store.LoadAsync(TokenStoreKey(compiled.Name, endpoint, identity.Id), cancellationToken).ConfigureAwait(false);
        return await RefreshGrantAsync(compiled, grant, endpoint, identity, refreshToken, previous, clientOptions.Store, Transport(clientOptions), cancellationToken, Now(clientOptions)).ConfigureAwait(false);
    }

    private static async Task<TokenSet> StoredTokenAsync(OAuthSchemeDescriptor compiled, OAuthClientOptions clientOptions, DiscoveryCache? discovery, CancellationToken cancellationToken)
    {
        var grant = RefreshFlowOrNull(compiled);
        var identity = ResolveIdentity(compiled, clientOptions);
        var store = clientOptions.Store ?? new MemoryTokenStore();
        var endpoint = grant is null
            ? await ResolveEndpointAsync(compiled, null, "token", static document => document.TokenEndpoint, clientOptions, discovery, cancellationToken).ConfigureAwait(false)
            : RefreshEndpoint(grant, compiled);
        var stored = await store.LoadAsync(TokenStoreKey(compiled.Name, endpoint, identity.Id), cancellationToken).ConfigureAwait(false);
        if (stored is null) throw new AuthException("missing-credential", compiled.Name, "no stored token set exists for this scheme; complete an authorization or device flow first");
        if (IsFresh(stored, compiled.RefreshSkewSeconds * 1000L, Now(clientOptions))) return stored;
        if (stored.RefreshToken is null) throw new AuthException("missing-credential", compiled.Name, "the stored token set carries no refresh token");
        return await RefreshGrantAsync(compiled, grant, endpoint, identity, stored.RefreshToken, stored, store, Transport(clientOptions), cancellationToken, Now(clientOptions)).ConfigureAwait(false);
    }

    /// <summary>One refresh grant over the resolved endpoint: a compiled flow carries its declared client authentication; a discovery-defined scheme uses the discovery client authentication.</summary>
    private static async Task<TokenSet> RefreshGrantAsync(OAuthSchemeDescriptor compiled, OAuthFlowDescriptor? grant, string endpoint, (string? Id, string? Secret) identity, string refreshToken, TokenSet? previous, ITokenStore? store, HttpMessageInvoker transport, CancellationToken cancellationToken, long now)
    {
        var fields = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = refreshToken,
        };
        var auth = grant is null ? DiscoveryClientAuth(compiled, identity) : ClientAuthFor(grant, identity);
        if (!auth.Basic && identity.Id is not null) fields["client_id"] = identity.Id;
        var acquired = await TokenRequestAsync(compiled.Name, endpoint, auth, fields, transport, cancellationToken, now).ConfigureAwait(false);
        var adopted = Adopt(previous, acquired);
        if (store is not null) await store.ReplaceAsync(TokenStoreKey(compiled.Name, endpoint, identity.Id), adopted, cancellationToken).ConfigureAwait(false);
        return adopted;
    }
"#;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the discovery document's `revocation_endpoint`.
const REVOKE_DISCOVERY: &str = r#"
    /// <summary>Revokes one token at the compiled revocation endpoint (RFC 7009, form-encoded). Authentication follows the compiled client policy. Endpoint resolution follows the compiled precedence: the configured revocation endpoint always wins; otherwise, when the scheme compiles a discovery URL, the discovery document's revocation_endpoint resolves the request. This one-shot helper fetches discovery per call and keeps no cache.</summary>
    public static async Task RevokeTokenAsync(string scheme, string token, OAuthClientOptions? options = null, string? tokenTypeHint = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var clientOptions = options ?? new OAuthClientOptions();
        var grant = RefreshFlowOrNull(compiled);
        var identity = ResolveIdentity(compiled, clientOptions);
        var endpoint = await ResolveEndpointAsync(compiled, compiled.RevocationEndpoint, "revocation", static document => document.RevocationEndpoint, clientOptions, null, cancellationToken).ConfigureAwait(false);
        var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["token"] = token };
        if (tokenTypeHint is not null) fields["token_type_hint"] = tokenTypeHint;
        var auth = grant is null ? DiscoveryClientAuth(compiled, identity) : ClientAuthFor(grant, identity);
        if (!auth.Basic && identity.Id is not null) fields["client_id"] = identity.Id;
        await PostFormAsync(compiled.Name, endpoint, auth, fields, Transport(clientOptions), cancellationToken).ConfigureAwait(false);
    }
"#;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECT_DISCOVERY: &str = r#"
    /// <summary>Introspects one token at the compiled introspection endpoint (RFC 7662, form-encoded) and returns the server's JSON response. Authentication follows the compiled client policy. Endpoint resolution follows the compiled precedence: the configured introspection endpoint always wins; otherwise, when the scheme compiles a discovery URL, the discovery document's introspection_endpoint resolves the request. This one-shot helper fetches discovery per call and keeps no cache.</summary>
    public static async Task<JsonElement> IntrospectTokenAsync(string scheme, string token, OAuthClientOptions? options = null, string? tokenTypeHint = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var clientOptions = options ?? new OAuthClientOptions();
        var grant = RefreshFlowOrNull(compiled);
        var identity = ResolveIdentity(compiled, clientOptions);
        var endpoint = await ResolveEndpointAsync(compiled, compiled.IntrospectionEndpoint, "introspection", static document => document.IntrospectionEndpoint, clientOptions, null, cancellationToken).ConfigureAwait(false);
        var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["token"] = token };
        if (tokenTypeHint is not null) fields["token_type_hint"] = tokenTypeHint;
        var auth = grant is null ? DiscoveryClientAuth(compiled, identity) : ClientAuthFor(grant, identity);
        if (!auth.Basic && identity.Id is not null) fields["client_id"] = identity.Id;
        return JsonObject(compiled.Name, await PostFormAsync(compiled.Name, endpoint, auth, fields, Transport(clientOptions), cancellationToken).ConfigureAwait(false), "introspection");
    }
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-provider cache with SemaphoreSlim
/// single-flight, and the endpoint-resolution precedence.
const DISCOVERY: &str = r#"
    private const int DiscoveryMaxBytes = 1 << 20;

    /// <summary>Client authentication for endpoints the discovery document supplies (no compiled flow declares one): client-secret-basic when the compiled configuration carries a client secret variable — an unavailable value becomes the typed missing-credential refusal — else the public profile, which sends the client id in the form.</summary>
    private static (bool Basic, string? Id, string? Secret) DiscoveryClientAuth(OAuthSchemeDescriptor scheme, (string? Id, string? Secret) identity) => (scheme.ClientSecretEnv is not null, identity.Id, scheme.ClientSecretEnv is not null ? identity.Secret : null);

    /// <summary>Resolves one executable (non-deprecated) compiled flow, or null when the scheme compiles none: a discovery-defined scheme's flows live in the discovery document.</summary>
    private static OAuthFlowDescriptor? ExecutableFlowOrNull(OAuthSchemeDescriptor scheme, string kind)
    {
        foreach (var candidate in scheme.Flows)
        {
            if (candidate.Kind == kind && !candidate.Deprecated) return candidate;
        }
        return null;
    }

    /// <summary>The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL, else null for a scheme the discovery document defines.</summary>
    private static OAuthFlowDescriptor? RefreshFlowOrNull(OAuthSchemeDescriptor scheme)
    {
        OAuthFlowDescriptor? fallback = null;
        foreach (var candidate in scheme.Flows)
        {
            if (candidate.Deprecated) continue;
            if (candidate.Kind == "authorization-code") return candidate;
            if (fallback is null && candidate.TokenUrl is not null) fallback = candidate;
        }
        return fallback;
    }

    /// <summary>Resolves one lifecycle endpoint through the compiled precedence: an explicit compiled endpoint always wins; otherwise the cached discovery document's endpoint when the scheme compiles a discovery URL; otherwise the typed endpoint-unavailable refusal the compiled plan alone would produce.</summary>
    private static async Task<string> ResolveEndpointAsync(OAuthSchemeDescriptor scheme, string? compiledEndpoint, string member, Func<DiscoveredEndpoints, string?> discovered, OAuthClientOptions clientOptions, DiscoveryCache? cache, CancellationToken cancellationToken)
    {
        if (compiledEndpoint is not null) return compiledEndpoint;
        var document = await DiscoverAsync(scheme, clientOptions, cache, cancellationToken).ConfigureAwait(false);
        return discovered(document) ?? throw new AuthException("endpoint-unavailable", scheme.Name, $"neither the compiled plan nor the discovery document carries a {member} endpoint for this scheme");
    }

    /// <summary>Fetches the scheme's discovery document (GET, `accept: application/json`), returning the cache's document when one exists. The response is bounded at DiscoveryMaxBytes; timeouts stay with the caller's transport, exactly like every other request in this module.</summary>
    private static async Task<DiscoveredEndpoints> DiscoverAsync(OAuthSchemeDescriptor scheme, OAuthClientOptions clientOptions, DiscoveryCache? cache, CancellationToken cancellationToken)
    {
        var url = scheme.Discovery ?? throw new AuthException("endpoint-unavailable", scheme.Name, "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery");
        if (cache is null) return await DiscoverFetchAsync(scheme, url, Transport(clientOptions), cancellationToken).ConfigureAwait(false);
        if (cache.TryGet(scheme.Name, out var cached)) return cached;
        var gate = cache.Gate(scheme.Name);
        await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (cache.TryGet(scheme.Name, out var rechecked)) return rechecked;
            var document = await DiscoverFetchAsync(scheme, url, Transport(clientOptions), cancellationToken).ConfigureAwait(false);
            cache.Store(scheme.Name, document);
            return document;
        }
        finally
        {
            gate.Release();
        }
    }

    /// <summary>One GET for the discovery document with the bounded response ceiling and the typed decode. The exact issuer rule: when the document carries an `issuer` claim, it must be an absolute http(s) URL whose origin (scheme, host and the port with the scheme default made explicit) equals the discovery URL's origin; OpenID Connect openIdConnectUrl documents are validated against their `issuer` claim exactly this way, as are RFC 8414 authorization-server metadata documents. A missing claim is tolerated. Failures never carry response body text.</summary>
    private static async Task<DiscoveredEndpoints> DiscoverFetchAsync(OAuthSchemeDescriptor scheme, string url, HttpMessageInvoker transport, CancellationToken cancellationToken)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, url);
        request.Headers.Accept.ParseAdd("application/json");
        HttpResponseMessage response;
        try { response = await transport.SendAsync(request, cancellationToken).ConfigureAwait(false); }
        catch (Exception error) when (error is not OperationCanceledException) { throw new AuthException("discovery-failed", scheme.Name, "the discovery document request failed before a response arrived"); }
        using (response)
        {
            if ((int)response.StatusCode < 200 || (int)response.StatusCode > 299)
            {
                throw new AuthException("discovery-failed", scheme.Name, $"the discovery document request answered HTTP {(int)response.StatusCode}", (int)response.StatusCode);
            }
            var body = await response.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
            if (body.Length > DiscoveryMaxBytes) throw new AuthException("discovery-failed", scheme.Name, "the discovery document exceeds the compiled response ceiling");
            return DiscoveryDocument(scheme.Name, url, body);
        }
    }

    /// <summary>Decodes and validates one discovery response body into the endpoints this module resolves. The issuer rule follows the discovery document claim; failures carry only safe metadata, never response body text.</summary>
    private static DiscoveredEndpoints DiscoveryDocument(string scheme, string url, byte[] body)
    {
        JsonElement document;
        try
        {
            using var parsed = JsonDocument.Parse(body);
            document = parsed.RootElement.Clone();
        }
        catch (Exception error) when (error is not AuthException)
        {
            throw new AuthException("discovery-failed", scheme, "the discovery document is not readable JSON");
        }
        if (document.ValueKind != JsonValueKind.Object) throw new AuthException("discovery-failed", scheme, "the discovery document is not a JSON object");
        var issuer = document.TryGetProperty("issuer", out var claim) && claim.ValueKind == JsonValueKind.String ? claim.GetString() : null;
        if (!string.IsNullOrEmpty(issuer))
        {
            var issuerOrigin = UrlOrigin(issuer);
            var discoveryOrigin = UrlOrigin(url);
            if (issuerOrigin is null || discoveryOrigin is null || !string.Equals(issuerOrigin, discoveryOrigin, StringComparison.Ordinal))
            {
                throw new AuthException("discovery-failed", scheme, "the discovery document issuer does not share the discovery URL origin");
            }
        }
        return new DiscoveredEndpoints
        {
            TokenEndpoint = DiscoveredEndpoint(scheme, document, "token_endpoint"),
            RevocationEndpoint = DiscoveredEndpoint(scheme, document, "revocation_endpoint"),
            IntrospectionEndpoint = DiscoveredEndpoint(scheme, document, "introspection_endpoint"),
        };
    }

    /// <summary>Reads one discovery document member: absent stays null; a non-string or unusable value is a typed discovery failure. Unknown members are ignored.</summary>
    private static string? DiscoveredEndpoint(string scheme, JsonElement document, string member)
    {
        if (!document.TryGetProperty(member, out var value) || value.ValueKind == JsonValueKind.Null) return null;
        var found = value.ValueKind == JsonValueKind.String ? value.GetString() : null;
        if (string.IsNullOrEmpty(found) || found!.Any(static character => character < ' ' || character == '\u007f'))
        {
            throw new AuthException("discovery-failed", scheme, $"the discovery document carries an unusable {member} value");
        }
        return found;
    }

    /// <summary>The origin of one absolute http(s) URL: scheme, host and the port with the scheme default made explicit. Returns null when the value is not an absolute http(s) URL.</summary>
    private static string? UrlOrigin(string value)
    {
        if (!Uri.TryCreate(value, UriKind.Absolute, out var parsed) || (parsed.Scheme != "http" && parsed.Scheme != "https")) return null;
        return $"{parsed.Scheme}://{parsed.Host}:{parsed.Port}";
    }
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries the
/// configured endpoint.
const REVOKE: &str = r#"
    /// <summary>Revokes one token at the compiled revocation endpoint (RFC 7009, form-encoded). Authentication follows the compiled client policy.</summary>
    public static async Task RevokeTokenAsync(string scheme, string token, OAuthClientOptions? options = null, string? tokenTypeHint = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var endpoint = compiled.RevocationEndpoint ?? throw new AuthException("endpoint-unavailable", compiled.Name, "no revocation endpoint was configured for this scheme");
        var grant = RefreshFlow(compiled);
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["token"] = token };
        if (tokenTypeHint is not null) fields["token_type_hint"] = tokenTypeHint;
        if (grant.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
        await PostFormAsync(compiled.Name, endpoint, ClientAuthFor(grant, identity), fields, Transport(clientOptions), cancellationToken).ConfigureAwait(false);
    }
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries the
/// configured endpoint.
const INTROSPECT: &str = r#"
    /// <summary>Introspects one token at the compiled introspection endpoint (RFC 7662, form-encoded) and returns the server's JSON response. Authentication follows the compiled client policy.</summary>
    public static async Task<JsonElement> IntrospectTokenAsync(string scheme, string token, OAuthClientOptions? options = null, string? tokenTypeHint = null, CancellationToken cancellationToken = default)
    {
        if (!IsUsableSecret(token)) throw new AuthException("invalid-request", scheme, "token must be a nonempty string without control characters");
        var compiled = Compiled(scheme);
        var endpoint = compiled.IntrospectionEndpoint ?? throw new AuthException("endpoint-unavailable", compiled.Name, "no introspection endpoint was configured for this scheme");
        var grant = RefreshFlow(compiled);
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var fields = new Dictionary<string, string>(StringComparer.Ordinal) { ["token"] = token };
        if (tokenTypeHint is not null) fields["token_type_hint"] = tokenTypeHint;
        if (grant.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
        return JsonObject(compiled.Name, await PostFormAsync(compiled.Name, endpoint, ClientAuthFor(grant, identity), fields, Transport(clientOptions), cancellationToken).ConfigureAwait(false), "introspection");
    }
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
    /// <summary>Runs the device authorization grant to completion (RFC 8628): requests the device code at the compiled device-authorization URL, then polls the token endpoint, honoring authorization_pending, slow_down (backing off five seconds per occurrence), the server interval and the code's expiry, and returns the typed token set. Returns only after the user completes authorization at the verification URI the server returned.</summary>
    public static async Task<TokenSet> BeginDeviceAuthorizationAsync(string scheme, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)
    {
        var compiled = Compiled(scheme);
        var deviceFlow = ExecutableFlow(compiled, "device-authorization");
        var deviceEndpoint = deviceFlow.DeviceAuthorizationUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled device-authorization flow declares no device-authorization URL");
        var tokenUrl = deviceFlow.TokenUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled device-authorization flow declares no token URL");
        var clientOptions = options ?? new OAuthClientOptions();
        var identity = ResolveIdentity(compiled, clientOptions);
        var transport = Transport(clientOptions);
        var requestFields = new Dictionary<string, string>(StringComparer.Ordinal);
        if (identity.Id is not null) requestFields["client_id"] = identity.Id;
        var body = JsonObject(compiled.Name, await PostFormAsync(compiled.Name, deviceEndpoint, (false, identity.Id, null), requestFields, transport, cancellationToken).ConfigureAwait(false), "device-authorization");
        var deviceCode = body.TryGetProperty("device_code", out var deviceElement) && deviceElement.ValueKind == JsonValueKind.String ? deviceElement.GetString() : null;
        if (deviceCode is null || !IsUsableSecret(deviceCode)) throw new AuthException("invalid-response", compiled.Name, "the device-authorization response carries no usable device code");
        var expiresInSeconds = body.TryGetProperty("expires_in", out var expiresElement) && expiresElement.ValueKind == JsonValueKind.Number && expiresElement.TryGetInt64(out var declared) && declared > 0 ? declared : 600;
        var intervalSeconds = body.TryGetProperty("interval", out var intervalElement) && intervalElement.ValueKind == JsonValueKind.Number && intervalElement.TryGetInt64(out var interval) && interval > 0 ? interval : 5;
        var expiresAt = Now(clientOptions) + expiresInSeconds * 1000;
        Func<int, CancellationToken, Task> delayFor = clientOptions.Delay ?? (static (milliseconds, token) => Task.Delay(milliseconds, token));
        var intervalMilliseconds = Math.Max(1, intervalSeconds) * 1000;
        for (;;)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (Now(clientOptions) >= expiresAt) throw new AuthException("device-code-expired", compiled.Name, "the device code expired before authorization completed");
            await delayFor((int)intervalMilliseconds, cancellationToken).ConfigureAwait(false);
            cancellationToken.ThrowIfCancellationRequested();
            if (Now(clientOptions) >= expiresAt) throw new AuthException("device-code-expired", compiled.Name, "the device code expired before authorization completed");
            var fields = new Dictionary<string, string>(StringComparer.Ordinal)
            {
                ["grant_type"] = "urn:ietf:params:oauth:grant-type:device_code",
                ["device_code"] = deviceCode,
            };
            if (deviceFlow.ClientAuth == "none" && identity.Id is not null) fields["client_id"] = identity.Id;
            TokenSet acquired;
            try
            {
                acquired = await TokenRequestAsync(compiled.Name, tokenUrl, ClientAuthFor(deviceFlow, identity), fields, transport, cancellationToken, Now(clientOptions)).ConfigureAwait(false);
            }
            catch (AuthException error) when (error.ServerError is "authorization_pending" or "slow_down" or "expired_token")
            {
                if (error.ServerError == "expired_token") throw new AuthException("device-code-expired", compiled.Name, "the device code expired before authorization completed");
                if (error.ServerError == "slow_down") intervalMilliseconds += 5000;
                continue;
            }
            if (clientOptions.Store is not null) await clientOptions.Store.ReplaceAsync(TokenStoreKey(compiled.Name, tokenUrl, identity.Id), acquired, cancellationToken).ConfigureAwait(false);
            return acquired;
        }
    }
"#;

/// The plain-variant replaying section: the compiled client-credentials token
/// endpoint serves both the store key and the lifecycle-endpoint exclusion.
const REPLAY_PLAIN: &str = r#"
    /// <summary>Creates the replaying variant of the compiled client-credentials provider. Every provider option behaves exactly as in CreateClientCredentialsProvider; the replay semantics are strictly additive and the plain provider keeps today's attach-only semantics. Creation refuses a compiled scheme whose client-credentials flow declares no token URL, exactly like the plain provider's first attach.</summary>
    public static ReplayingCredentials CreateReplayingCredentialsProvider(string scheme, OAuthClientOptions? options = null)
    {
        var compiled = Compiled(scheme);
        var grant = ExecutableFlow(compiled, "client-credentials");
        _ = grant.TokenUrl ?? throw new AuthException("endpoint-unavailable", compiled.Name, "the compiled client-credentials flow declares no token URL");
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        return new ReplayingCredentials(compiled, clientOptions, grant.TokenUrl!);
    }

    /// <summary>One attach this provider served, remembered so the wrapped transport can tell which requests carried this provider's token. The record keeps only safe metadata; token values already traveled on the wire.</summary>
    private sealed record ServedAttach(string Value, bool Eligible);

    /// <summary>A replaying client-credentials credential: the plain provider's attach behavior plus the unified request policy. Use it in two places — pass Attach as the scheme's Credentials member, and pass Transport(handler) as the client's HttpClient handler:
    /// <code>
    /// var replaying = OAuth.CreateReplayingCredentialsProvider("name", options);
    /// var credentials = new Credentials { Name = replaying.Attach };
    /// using var http = new HttpClient(replaying.Transport(innerHandler));
    /// using var client = new Client(credentials, httpClient: http);
    /// </code>
    /// A 401 (and only a 401) on a request whose Authorization value this provider attached triggers exactly one coordinated refresh — concurrent 401s share one token request through the same single-flight store round — and exactly one replay of the request with the fresh token. The second response is surfaced whatever it is: a second 401 reaches the caller as the declared error. The overall budget is one refresh plus one replay, never nested with other retry policies (requests are not retried today). Attaches for stream-protected requirements are never replayed, because delivered stream data prevents a transparent restart. A refresh failure surfaces as the typed AuthException instead of a replay. The plain provider keeps today's semantics: replay is this wrapper's opt-in only.</summary>
    public sealed class ReplayingCredentials
    {
        private const int ReplayMaxBodyBytes = 1 << 26;
        private readonly OAuthSchemeDescriptor _compiled;
        private readonly OAuthClientOptions _options;
        private readonly ITokenStore _store;
        private readonly string _tokenUrl;
        private readonly IReadOnlyList<string> _neverReplay;
        private readonly object _gate = new();
        private readonly List<ServedAttach> _served = new();
        private readonly SemaphoreSlim _roundsGate = new(1, 1);
        private readonly Dictionary<string, Task<AuthorizationValue>> _rounds = new(StringComparer.Ordinal);

        internal ReplayingCredentials(OAuthSchemeDescriptor compiled, OAuthClientOptions options, string tokenUrl)
        {
            _compiled = compiled;
            _options = options;
            _store = options.Store ?? new MemoryTokenStore();
            _tokenUrl = tokenUrl;
            _neverReplay = NoReplayRequirements.TryGetValue(compiled.Name, out var pointers) ? pointers : Array.Empty<string>();
        }

        /// <summary>The replaying credential itself: serves the scheme's tokens exactly like the plain provider and remembers which Authorization values its attaches produced.</summary>
        public async ValueTask<AuthorizationValue> Attach(CredentialContext context, CancellationToken cancellationToken)
        {
            var credential = Authorization(await AcquireAsync(cancellationToken).ConfigureAwait(false), _compiled.Name);
            Record(credential, Eligible(context));
            return credential;
        }

        /// <summary>Wraps inner with the one-refresh-one-replay 401 policy; call once per client and pass the returned handler to the client's HttpClient. Token requests keep traveling through inner directly.</summary>
        public HttpMessageHandler Transport(HttpMessageHandler inner)
        {
            if (inner is null) throw new ArgumentNullException(nameof(inner));
            return new ReplayTransportHandler(this, new HttpMessageInvoker(inner, disposeHandler: false));
        }

        private async Task<TokenSet> AcquireAsync(CancellationToken cancellationToken) => await ClientCredentialsTokenAsync(_compiled.Name, _options, cancellationToken).ConfigureAwait(false);

        private bool Eligible(CredentialContext context)
        {
            foreach (var pointer in _neverReplay)
            {
                if (string.Equals(pointer, context.RequirementSource, StringComparison.Ordinal)) return false;
            }
            return true;
        }

        private void Record(AuthorizationValue credential, bool eligible)
        {
            lock (_gate)
            {
                _served.Insert(0, new ServedAttach(credential.Scheme + " " + credential.Parameter, eligible));
                if (_served.Count > 8) _served.RemoveRange(8, _served.Count - 8);
            }
        }

        private ServedAttach? FindServed(string? presented)
        {
            if (string.IsNullOrEmpty(presented)) return null;
            lock (_gate)
            {
                foreach (var entry in _served)
                {
                    if (entry.Value == presented && entry.Eligible) return entry;
                }
            }
            return null;
        }

        private string StoreKey() => TokenStoreKey(_compiled.Name, _tokenUrl, ResolveIdentity(_compiled, _options).Id);

        /// <summary>One coordinated refresh: concurrent 401s share one store round, a newer stored set wins over a stale re-refresh, and a failed round fails every waiter exactly once. The round resolves to the fresh complete Authorization value.</summary>
        private async Task<AuthorizationValue> RefreshAsync(string presented, CancellationToken cancellationToken)
        {
            var key = StoreKey();
            await _roundsGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            Task<AuthorizationValue> round;
            try
            {
                if (!_rounds.TryGetValue(key, out var found))
                {
                    round = RefreshRoundAsync(presented, key, cancellationToken);
                    _rounds[key] = round;
                }
                else
                {
                    round = found;
                }
            }
            finally
            {
                _roundsGate.Release();
            }
            try
            {
                return await round.ConfigureAwait(false);
            }
            finally
            {
                await _roundsGate.WaitAsync(CancellationToken.None).ConfigureAwait(false);
                try
                {
                    if (_rounds.TryGetValue(key, out var settled) && ReferenceEquals(settled, round)) _rounds.Remove(key);
                }
                finally
                {
                    _roundsGate.Release();
                }
            }
        }

        private async Task<AuthorizationValue> RefreshRoundAsync(string presented, string key, CancellationToken cancellationToken)
        {
            var stored = await _store.LoadAsync(key, cancellationToken).ConfigureAwait(false);
            if (stored is not null)
            {
                var current = Authorization(stored, _compiled.Name);
                if (!string.Equals(current.Scheme + " " + current.Parameter, presented, StringComparison.Ordinal)) return current;
            }
            await _store.ClearAsync(key, cancellationToken).ConfigureAwait(false);
            return Authorization(await AcquireAsync(cancellationToken).ConfigureAwait(false), _compiled.Name);
        }

        /// <summary>The replaying transport: the 401 interception half of the wrapper. Lifecycle endpoint requests are never replayed: they carry no bearer token of this provider, and the exact-target guard is defense in depth.</summary>
        private sealed class ReplayTransportHandler : HttpMessageHandler
        {
            internal ReplayTransportHandler(ReplayingCredentials owner, HttpMessageInvoker inner) { _owner = owner; _inner = inner; }
            private readonly ReplayingCredentials _owner;
            private readonly HttpMessageInvoker _inner;

            protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
            {
                // Buffer the request content so the one replay can resend the
                // exact bytes; an unbufferable or over-ceiling body still sends
                // once and simply never replays.
                var buffered = await BufferAsync(request, cancellationToken).ConfigureAwait(false);
                var response = await _inner.SendAsync(request, cancellationToken).ConfigureAwait(false);
                if ((int)response.StatusCode != 401) return response;
                var presented = AuthorizationHeader(request);
                if (presented is null) return response;
                if (_owner.FindServed(presented) is null) return response;
                // Lifecycle endpoint requests carry no bearer token of this
                // provider, so this exact-target guard is defense in depth
                // against loops.
                if (IsLifecycleTarget(request)) return response;
                if (request.Content is not null && buffered is null) return response;
                AuthorizationValue fresh;
                try
                {
                    fresh = await _owner.RefreshAsync(presented, cancellationToken).ConfigureAwait(false);
                }
                catch
                {
                    response.Dispose();
                    throw;
                }
                var replayed = ReplayedRequest(request, buffered, fresh);
                response.Dispose();
                HttpResponseMessage answered;
                try
                {
                    answered = await _inner.SendAsync(replayed, cancellationToken).ConfigureAwait(false);
                }
                catch
                {
                    replayed.Dispose();
                    throw;
                }
                return answered;
            }

            private bool IsLifecycleTarget(HttpRequestMessage request) => string.Equals(request.RequestUri?.ToString(), _owner._tokenUrl, StringComparison.Ordinal);

            /// <summary>Buffers the request content so the one replay can resend the exact bytes; an unbufferable or over-ceiling body still sends once and simply never replays.</summary>
            private static async Task<byte[]?> BufferAsync(HttpRequestMessage request, CancellationToken cancellationToken)
            {
                if (request.Content is null) return Array.Empty<byte>();
                try
                {
                    using var buffer = new global::System.IO.MemoryStream();
                    await request.Content.CopyToAsync(buffer, cancellationToken).ConfigureAwait(false);
                    if (buffer.Length > ReplayMaxBodyBytes) return null;
                    var bytes = buffer.ToArray();
                    var headers = request.Content.Headers;
                    request.Content = new ByteArrayContent(bytes);
                    foreach (var header in headers)
                    {
                        foreach (var value in header.Value) request.Content.Headers.TryAddWithoutValidation(header.Key, value);
                    }
                    return bytes;
                }
                catch (Exception error) when (error is not OperationCanceledException)
                {
                    return null;
                }
            }

            private static string? AuthorizationHeader(HttpRequestMessage request) => request.Headers.NonValidated.TryGetValues("Authorization", out var values) && values.FirstOrDefault() is { Length: > 0 } presented ? presented : null;

            private static HttpRequestMessage ReplayedRequest(HttpRequestMessage request, byte[]? buffered, AuthorizationValue fresh)
            {
                var replayed = new HttpRequestMessage(request.Method, request.RequestUri)
                {
                    Version = request.Version,
                    VersionPolicy = request.VersionPolicy,
                    Content = request.Content is null ? null : new ByteArrayContent(buffered!),
                };
                foreach (var header in request.Headers)
                {
                    if (string.Equals(header.Key, "Authorization", StringComparison.OrdinalIgnoreCase)) continue;
                    foreach (var value in header.Value) replayed.Headers.TryAddWithoutValidation(header.Key, value);
                }
                replayed.Headers.TryAddWithoutValidation("Authorization", $"{fresh.Scheme} {fresh.Parameter}");
                if (replayed.Content is not null)
                {
                    foreach (var header in request.Content!.Headers)
                    {
                        foreach (var value in header.Value) replayed.Content.Headers.TryAddWithoutValidation(header.Key, value);
                    }
                }
                return replayed;
            }
        }
    }
"#;

/// The discovery-variant replaying section: the refresh endpoint and the
/// lifecycle-endpoint exclusion resolve through the compiled precedence
/// (compiled token URL, else the discovery document's), cached per provider
/// exactly like the plain provider's discovery resolution.
const REPLAY_DISCOVERY: &str = r#"
    /// <summary>Creates the replaying variant of the compiled client-credentials provider. Every provider option behaves exactly as in CreateClientCredentialsProvider; the replay semantics are strictly additive and the plain provider keeps today's attach-only semantics. The refresh endpoint resolves through the compiled precedence — the compiled token URL when the client-credentials flow compiles one, otherwise the discovery document's token endpoint, fetched once and cached for this provider's lifetime.</summary>
    public static ReplayingCredentials CreateReplayingCredentialsProvider(string scheme, OAuthClientOptions? options = null)
    {
        var compiled = Compiled(scheme);
        var grant = ExecutableFlowOrNull(compiled, "client-credentials");
        var clientOptions = (options ?? new OAuthClientOptions()).Store is null
            ? (options ?? new OAuthClientOptions()) with { Store = new MemoryTokenStore() }
            : options!;
        return new ReplayingCredentials(compiled, clientOptions, grant, new DiscoveryCache());
    }

    /// <summary>One attach this provider served, remembered so the wrapped transport can tell which requests carried this provider's token. The record keeps only safe metadata; token values already traveled on the wire.</summary>
    private sealed record ServedAttach(string Value, bool Eligible);

    /// <summary>A replaying client-credentials credential: the plain provider's attach behavior plus the unified request policy. Use it in two places — pass Attach as the scheme's Credentials member, and pass Transport(handler) as the client's HttpClient handler:
    /// <code>
    /// var replaying = OAuth.CreateReplayingCredentialsProvider("name", options);
    /// var credentials = new Credentials { Name = replaying.Attach };
    /// using var http = new HttpClient(replaying.Transport(innerHandler));
    /// using var client = new Client(credentials, httpClient: http);
    /// </code>
    /// A 401 (and only a 401) on a request whose Authorization value this provider attached triggers exactly one coordinated refresh — concurrent 401s share one token request through the same single-flight store round — and exactly one replay of the request with the fresh token. The second response is surfaced whatever it is: a second 401 reaches the caller as the declared error. The overall budget is one refresh plus one replay, never nested with other retry policies (requests are not retried today). Attaches for stream-protected requirements are never replayed, because delivered stream data prevents a transparent restart. A refresh failure surfaces as the typed AuthException instead of a replay. The plain provider keeps today's semantics: replay is this wrapper's opt-in only.</summary>
    public sealed class ReplayingCredentials
    {
        private const int ReplayMaxBodyBytes = 1 << 26;
        private readonly OAuthSchemeDescriptor _compiled;
        private readonly OAuthClientOptions _options;
        private readonly ITokenStore _store;
        private readonly OAuthFlowDescriptor? _grant;
        private readonly DiscoveryCache? _discovery;
        private readonly IReadOnlyList<string> _neverReplay;
        private readonly object _gate = new();
        private readonly List<ServedAttach> _served = new();
        private readonly SemaphoreSlim _roundsGate = new(1, 1);
        private readonly Dictionary<string, Task<AuthorizationValue>> _rounds = new(StringComparer.Ordinal);

        internal ReplayingCredentials(OAuthSchemeDescriptor compiled, OAuthClientOptions options, OAuthFlowDescriptor? grant, DiscoveryCache? discovery)
        {
            _compiled = compiled;
            _options = options;
            _store = options.Store ?? new MemoryTokenStore();
            _grant = grant;
            _discovery = discovery;
            _neverReplay = NoReplayRequirements.TryGetValue(compiled.Name, out var pointers) ? pointers : Array.Empty<string>();
        }

        /// <summary>The replaying credential itself: serves the scheme's tokens exactly like the plain provider and remembers which Authorization values its attaches produced.</summary>
        public async ValueTask<AuthorizationValue> Attach(CredentialContext context, CancellationToken cancellationToken)
        {
            var credential = Authorization(await AcquireAsync(cancellationToken).ConfigureAwait(false), _compiled.Name);
            Record(credential, Eligible(context));
            return credential;
        }

        /// <summary>Wraps inner with the one-refresh-one-replay 401 policy; call once per client and pass the returned handler to the client's HttpClient. Token requests keep traveling through inner directly.</summary>
        public HttpMessageHandler Transport(HttpMessageHandler inner)
        {
            if (inner is null) throw new ArgumentNullException(nameof(inner));
            return new ReplayTransportHandler(this, new HttpMessageInvoker(inner, disposeHandler: false));
        }

        /// <summary>The acquisition through the compiled precedence, cached per provider exactly like every other discovery resolution.</summary>
        private async Task<TokenSet> AcquireAsync(CancellationToken cancellationToken) => await ClientCredentialsAsync(_compiled, _options, _discovery, cancellationToken).ConfigureAwait(false);

        private bool Eligible(CredentialContext context)
        {
            foreach (var pointer in _neverReplay)
            {
                if (string.Equals(pointer, context.RequirementSource, StringComparison.Ordinal)) return false;
            }
            return true;
        }

        private void Record(AuthorizationValue credential, bool eligible)
        {
            lock (_gate)
            {
                _served.Insert(0, new ServedAttach(credential.Scheme + " " + credential.Parameter, eligible));
                if (_served.Count > 8) _served.RemoveRange(8, _served.Count - 8);
            }
        }

        private ServedAttach? FindServed(string? presented)
        {
            if (string.IsNullOrEmpty(presented)) return null;
            lock (_gate)
            {
                foreach (var entry in _served)
                {
                    if (entry.Value == presented && entry.Eligible) return entry;
                }
            }
            return null;
        }

        private async Task<string> StoreKeyAsync(CancellationToken cancellationToken) => TokenStoreKey(_compiled.Name, await ResolveEndpointAsync(_compiled, _grant is null ? null : _grant.TokenUrl, "token", static document => document.TokenEndpoint, _options, _discovery, cancellationToken).ConfigureAwait(false), ResolveIdentity(_compiled, _options).Id);

        /// <summary>One coordinated refresh: concurrent 401s share one store round, a newer stored set wins over a stale re-refresh, and a failed round fails every waiter exactly once. The round resolves to the fresh complete Authorization value.</summary>
        private async Task<AuthorizationValue> RefreshAsync(string presented, CancellationToken cancellationToken)
        {
            var key = await StoreKeyAsync(cancellationToken).ConfigureAwait(false);
            await _roundsGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            Task<AuthorizationValue> round;
            try
            {
                if (!_rounds.TryGetValue(key, out var found))
                {
                    round = RefreshRoundAsync(presented, key, cancellationToken);
                    _rounds[key] = round;
                }
                else
                {
                    round = found;
                }
            }
            finally
            {
                _roundsGate.Release();
            }
            try
            {
                return await round.ConfigureAwait(false);
            }
            finally
            {
                await _roundsGate.WaitAsync(CancellationToken.None).ConfigureAwait(false);
                try
                {
                    if (_rounds.TryGetValue(key, out var settled) && ReferenceEquals(settled, round)) _rounds.Remove(key);
                }
                finally
                {
                    _roundsGate.Release();
                }
            }
        }

        private async Task<AuthorizationValue> RefreshRoundAsync(string presented, string key, CancellationToken cancellationToken)
        {
            var stored = await _store.LoadAsync(key, cancellationToken).ConfigureAwait(false);
            if (stored is not null)
            {
                var current = Authorization(stored, _compiled.Name);
                if (!string.Equals(current.Scheme + " " + current.Parameter, presented, StringComparison.Ordinal)) return current;
            }
            await _store.ClearAsync(key, cancellationToken).ConfigureAwait(false);
            return Authorization(await AcquireAsync(cancellationToken).ConfigureAwait(false), _compiled.Name);
        }

        /// <summary>The replaying transport: the 401 interception half of the wrapper. Lifecycle endpoint requests are never replayed: they carry no bearer token of this provider, and the exact-target guard is defense in depth; the discovery-resolved token endpoint rides the exact-token match.</summary>
        private sealed class ReplayTransportHandler : HttpMessageHandler
        {
            internal ReplayTransportHandler(ReplayingCredentials owner, HttpMessageInvoker inner) { _owner = owner; _inner = inner; }
            private readonly ReplayingCredentials _owner;
            private readonly HttpMessageInvoker _inner;

            protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
            {
                // Buffer the request content so the one replay can resend the
                // exact bytes; an unbufferable or over-ceiling body still sends
                // once and simply never replays.
                var buffered = await BufferAsync(request, cancellationToken).ConfigureAwait(false);
                var response = await _inner.SendAsync(request, cancellationToken).ConfigureAwait(false);
                if ((int)response.StatusCode != 401) return response;
                var presented = AuthorizationHeader(request);
                if (presented is null) return response;
                if (_owner.FindServed(presented) is null) return response;
                // Lifecycle endpoint requests carry no bearer token of this
                // provider, so this exact-target guard is defense in depth
                // against loops; the discovery-resolved token endpoint rides
                // the exact-token match.
                if (IsLifecycleTarget(request)) return response;
                if (request.Content is not null && buffered is null) return response;
                AuthorizationValue fresh;
                try
                {
                    fresh = await _owner.RefreshAsync(presented, cancellationToken).ConfigureAwait(false);
                }
                catch
                {
                    response.Dispose();
                    throw;
                }
                var replayed = ReplayedRequest(request, buffered, fresh);
                response.Dispose();
                HttpResponseMessage answered;
                try
                {
                    answered = await _inner.SendAsync(replayed, cancellationToken).ConfigureAwait(false);
                }
                catch
                {
                    replayed.Dispose();
                    throw;
                }
                return answered;
            }

            private bool IsLifecycleTarget(HttpRequestMessage request) => _owner._compiled.Discovery is { } discoveryUrl && string.Equals(request.RequestUri?.ToString(), discoveryUrl, StringComparison.Ordinal);

            /// <summary>Buffers the request content so the one replay can resend the exact bytes; an unbufferable or over-ceiling body still sends once and simply never replays.</summary>
            private static async Task<byte[]?> BufferAsync(HttpRequestMessage request, CancellationToken cancellationToken)
            {
                if (request.Content is null) return Array.Empty<byte>();
                try
                {
                    using var buffer = new global::System.IO.MemoryStream();
                    await request.Content.CopyToAsync(buffer, cancellationToken).ConfigureAwait(false);
                    if (buffer.Length > ReplayMaxBodyBytes) return null;
                    var bytes = buffer.ToArray();
                    var headers = request.Content.Headers;
                    request.Content = new ByteArrayContent(bytes);
                    foreach (var header in headers)
                    {
                        foreach (var value in header.Value) request.Content.Headers.TryAddWithoutValidation(header.Key, value);
                    }
                    return bytes;
                }
                catch (Exception error) when (error is not OperationCanceledException)
                {
                    return null;
                }
            }

            private static string? AuthorizationHeader(HttpRequestMessage request) => request.Headers.NonValidated.TryGetValues("Authorization", out var values) && values.FirstOrDefault() is { Length: > 0 } presented ? presented : null;

            private static HttpRequestMessage ReplayedRequest(HttpRequestMessage request, byte[]? buffered, AuthorizationValue fresh)
            {
                var replayed = new HttpRequestMessage(request.Method, request.RequestUri)
                {
                    Version = request.Version,
                    VersionPolicy = request.VersionPolicy,
                    Content = request.Content is null ? null : new ByteArrayContent(buffered!),
                };
                foreach (var header in request.Headers)
                {
                    if (string.Equals(header.Key, "Authorization", StringComparison.OrdinalIgnoreCase)) continue;
                    foreach (var value in header.Value) replayed.Headers.TryAddWithoutValidation(header.Key, value);
                }
                replayed.Headers.TryAddWithoutValidation("Authorization", $"{fresh.Scheme} {fresh.Parameter}");
                if (replayed.Content is not null)
                {
                    foreach (var header in request.Content!.Headers)
                    {
                        foreach (var value in header.Value) replayed.Content.Headers.TryAddWithoutValidation(header.Key, value);
                    }
                }
                return replayed;
            }
        }
    }
"#;
