//! Emitted-only OAuth runtime for the C# HTTP backend: the generated
//! `src/OAuth.g.cs` lifecycle, no-policy byte-identity, and native .NET
//! behavior against a stubbed HttpMessageHandler. Static runtime files are
//! never modified; the token lifecycle lives entirely in the generated package.

#![cfg(feature = "csharp-sdk")]

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.oauth.test/csharp-openapi.json";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
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

/// One client-credentials scheme and one device-authorization scheme, each used
/// by an operation, with declared token and refresh URLs.
fn oauth_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
            }},
            "/devices": {"get": {
                "operationId": "listDevices",
                "security": [{"deviceOAuth": []}],
                "responses": page
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {
                "authorizationCode": {
                    "authorizationUrl": "https://auth.oauth.test/authorize",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access", "write": "Write access"}
                },
                "clientCredentials": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access"}
                }
            }},
            "deviceOAuth": {"type": "oauth2", "flows": {"deviceAuthorization": {
                "deviceAuthorizationUrl": "https://auth.oauth.test/device",
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {}
            }}}
        }}
    })
}

/// The same operations with every credential removed: the no-OAuth control.
fn plain_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Plain", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": page}},
            "/devices": {"get": {"operationId": "listDevices", "responses": page}}
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "acme.oauth-sdk".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn oauth_config() -> OAuthDefaults {
    OAuthDefaults {
        schemes: [
            (
                "serviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_CLIENT_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_CLIENT_SECRET".into()),
                    revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                    introspection_endpoint: Some("https://auth.oauth.test/introspect".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
            (
                "deviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_DEVICE_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_DEVICE_SECRET".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        ..OAuthDefaults::default()
    }
}

fn oauth_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: oauth_config(),
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn off_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                mode: suspect_codegen::http_protocol::OAuthMode::Off,
                ..oauth_config()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn source(files: &[OutFile], path: &str) -> String {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} missing"))
        .content
        .clone()
}

fn files_map(files: &[OutFile]) -> std::collections::BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect()
}

#[test]
fn oauth_module_emits_exactly_for_executable_schemes() {
    let files = generate(oauth_document(), &oauth_options());
    let oauth = source(&files, "csharp/src/OAuth.g.cs");
    for expected in [
        // The token set, the instance-owned store contract and typed error.
        "public sealed record TokenSet",
        "public interface ITokenStore",
        "public sealed class MemoryTokenStore : ITokenStore",
        "public sealed class AuthException : Exception",
        // Frozen compiled descriptors carry the plan, including configuration
        // supplements and the compiled client authentication.
        "public static readonly IReadOnlyDictionary<string, OAuthSchemeDescriptor> Schemes",
        "[\"serviceOAuth\"] = new(\"serviceOAuth\", \"oauth2\", 30, null, \"https://auth.oauth.test/revoke\", \"https://auth.oauth.test/introspect\", \"SUSPECT_OAUTH_CLIENT_ID\", \"SUSPECT_OAUTH_CLIENT_SECRET\", new OAuthFlowDescriptor[]",
        "new(\"client-credentials\", null, \"https://auth.oauth.test/token\", \"https://auth.oauth.test/token-refresh\", null, \"client-secret-basic\", false,",
        "new(\"authorization-code\", \"https://auth.oauth.test/authorize\", \"https://auth.oauth.test/token\", \"https://auth.oauth.test/token-refresh\", null, \"client-secret-basic\", false,",
        "new(\"device-authorization\", null, \"https://auth.oauth.test/token\", null, \"https://auth.oauth.test/device\", \"client-secret-basic\", false,",
        "[\"deviceOAuth\"] = new(\"deviceOAuth\", \"oauth2\", 30, null, null, null, \"SUSPECT_OAUTH_DEVICE_ID\", \"SUSPECT_OAUTH_DEVICE_SECRET\", new OAuthFlowDescriptor[]",
        "[\"read\"] = \"Read access\"",
        // Acquisition, refresh, providers and the authorization-code flow.
        "public static async Task<TokenSet> ClientCredentialsTokenAsync(string scheme, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)",
        "public static AuthorizationProvider CreateClientCredentialsProvider(string scheme, OAuthClientOptions? options = null)",
        "public static AuthorizationProvider CreateRefreshProvider(string scheme, OAuthClientOptions? options = null)",
        "public static async Task<TokenSet> RefreshTokenAsync(string scheme, string refreshToken, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)",
        "public static AuthorizationBegin BeginAuthorization(string scheme, string redirectUri, OAuthClientOptions? options = null, IEnumerable<string>? scopes = null)",
        "public static async Task<TokenSet> CompleteAuthorizationAsync(AuthorizationTransaction transaction, string code, string state, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)",
        "public static async Task<TokenSet> BeginDeviceAuthorizationAsync(string scheme, OAuthClientOptions? options = null, CancellationToken cancellationToken = default)",
        "public static async Task RevokeTokenAsync(",
        "public static async Task<JsonElement> IntrospectTokenAsync(",
        "public static string TokenStoreKey(string scheme, string issuer, string? clientId)",
        "grant_type\"] = \"client_credentials\"",
        "grant_type\"] = \"authorization_code\"",
        "grant_type\"] = \"refresh_token\"",
        "grant_type\"] = \"urn:ietf:params:oauth:grant-type:device_code\"",
        "code_challenge_method\"] = \"S256\"",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.g.cs lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // Control A: the same options against a document without OAuth schemes
    // keep the pre-OAuth bytes identical, with no new file.
    let mut unbound = oauth_options();
    unbound.sdk_defaults = Some(SdkDefaults::v1());
    let control = files_map(&generate(plain_document(), &unbound));
    let baseline = files_map(&generate(plain_document(), &GenerationOptions::default()));
    let differing: Vec<&String> = control
        .keys()
        .chain(baseline.keys())
        .filter(|path| control.get(*path) != baseline.get(*path))
        .collect();
    assert!(
        differing.is_empty(),
        "no OAuth usable: configured client defaults changed {differing:?}"
    );
    assert!(!control.contains_key("csharp/src/OAuth.g.cs"));

    // Control B: oauth mode off emits nothing and stays byte-identical to the
    // pre-OAuth generation of the same document.
    let off = files_map(&generate(oauth_document(), &off_options()));
    let baseline_off = files_map(&generate(oauth_document(), &GenerationOptions::default()));
    let differing_off: Vec<&String> = off
        .keys()
        .chain(baseline_off.keys())
        .filter(|path| off.get(*path) != baseline_off.get(*path))
        .collect();
    assert!(
        differing_off.is_empty(),
        "oauth off changed {differing_off:?}"
    );
    assert!(!off.contains_key("csharp/src/OAuth.g.cs"));

    // Control C: a scheme carrying only a deprecated implicit flow emits
    // nothing, mirroring the planner's refusal to execute implicit grants.
    let implicit_only = json!({
        "openapi": "3.2.0",
        "info": {"title": "Implicit", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"implicitOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {
                    "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}
                }}}}}
            }}
        },
        "components": {"securitySchemes": {
            "implicitOAuth": {"type": "oauth2", "flows": {"implicit": {
                "authorizationUrl": "https://auth.oauth.test/authorize",
                "scopes": {"read": "Read access"}
            }}}
        }}
    });
    let implicit_files = files_map(&generate(implicit_only.clone(), &unbound));
    let implicit_baseline = files_map(&generate(implicit_only, &GenerationOptions::default()));
    let differing_implicit: Vec<&String> = implicit_files
        .keys()
        .chain(implicit_baseline.keys())
        .filter(|path| implicit_files.get(*path) != implicit_baseline.get(*path))
        .collect();
    assert!(
        differing_implicit.is_empty(),
        "implicit-only scheme changed {differing_implicit:?}"
    );
    assert!(!implicit_files.contains_key("csharp/src/OAuth.g.cs"));
}

#[test]
fn plan_carries_the_oauth_plan_only_when_configured() {
    use suspect_codegen::csharp_sdk;
    let contract = contract_with_document(oauth_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = csharp_sdk::plan_sdk_with_options(
        contract.clone(),
        &selected,
        csharp_sdk::SdkConfig::default(),
        csharp_sdk::protocol::ProtocolOptions {
            sdk_defaults: Some(SdkDefaults {
                oauth: oauth_config(),
                ..SdkDefaults::v1()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let plan = configured.oauth().expect("configured policy is carried");
    assert_eq!(plan.schemes.len(), 2);
    let service = plan
        .schemes
        .iter()
        .find(|scheme| scheme.name == "serviceOAuth")
        .expect("client-credentials scheme");
    assert_eq!(
        service.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(service.refresh_skew_seconds, 30);
    let control = csharp_sdk::plan_sdk_with_options(
        contract,
        &selected,
        csharp_sdk::SdkConfig::default(),
        csharp_sdk::protocol::ProtocolOptions::default(),
    )
    .unwrap();
    assert!(control.oauth().is_none());
}

fn dotnet() -> Option<String> {
    let path = std::env::var_os("SUSPECT_DOTNET_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/.local/share/mise/dotnet-root/dotnet"));
    let output = Command::new(&path).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| path.to_string_lossy().into_owned())
}

const DRIVER: &str = r#"
using System.Net;
using System.Text;
using Acme.OauthSdk;

var hits = new List<string>();
var counter = 0;
var devicePolls = 0;
var now = 1_000_000L;
Func<DateTimeOffset> clock = () => DateTimeOffset.UnixEpoch.AddMilliseconds(Interlocked.Read(ref now));
var stub = new Stub(request => OAuthDriver.Respond(request, hits, () => counter, value => counter = value, () => devicePolls, value => devicePolls = value));
using var http = new HttpClient(stub);

OAuthClientOptions Options(ITokenStore? store = null) => new()
{
    ClientId = "client-id",
    ClientSecret = "client-secret",
    Clock = clock,
    Transport = http,
    Store = store,
};

// First acquire authenticates with client-secret-basic and the form-encoded
// grant; the second call reuses the stored token.
hits.Clear();
var store = new MemoryTokenStore();
var first = await OAuth.ClientCredentialsTokenAsync("serviceOAuth", Options(store));
if (first.AccessToken != "access-token-1" || first.TokenType != "Bearer") throw new Exception($"acquisition differs: {first.AccessToken}");
var tokenHits = hits.Where(h => h.Contains("/token")).ToList();
if (tokenHits.Count != 1 || !tokenHits[0].Contains("grant_type=client_credentials")) throw new Exception($"token request differs: {tokenHits.Count}");
if (!tokenHits[0].Contains("Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=")) throw new Exception($"basic authentication differs: {tokenHits[0]}");
var second = await OAuth.ClientCredentialsTokenAsync("serviceOAuth", Options(store));
if (second.AccessToken != "access-token-1") throw new Exception("the second call must reuse the stored token");
if (hits.Count(h => h.Contains("/token")) != 1) throw new Exception("the cache hit must not reach the transport");

// Past expiry (beyond the compiled skew) the next call re-acquires.
Interlocked.Add(ref now, 4_000_000);
var third = await OAuth.ClientCredentialsTokenAsync("serviceOAuth", Options(store));
if (third.AccessToken != "access-token-2") throw new Exception("an expired token must re-acquire");
if (hits.Count(h => h.Contains("/token")) != 2) throw new Exception($"re-acquisition differs: {hits.Count(h => h.Contains("/token"))}");

// Concurrent callers share exactly one in-flight acquisition.
hits.Clear();
Interlocked.Exchange(ref now, 10_000_000);
var flightOptions = Options(new MemoryTokenStore());
var shared = await Task.WhenAll(
    OAuth.ClientCredentialsTokenAsync("serviceOAuth", flightOptions),
    OAuth.ClientCredentialsTokenAsync("serviceOAuth", flightOptions));
if (shared[0].AccessToken != shared[1].AccessToken) throw new Exception("single-flight callers received different tokens");
if (hits.Count(h => h.Contains("/token")) != 1) throw new Exception($"concurrent calls must single-flight: {hits.Count(h => h.Contains("/token"))}");

// Explicit refresh: the rotated refresh token is adopted and stored; a later
// response without one retains the stored refresh token.
hits.Clear();
var refreshStore = new MemoryTokenStore();
var refreshed = await OAuth.RefreshTokenAsync("serviceOAuth", "stale", Options(refreshStore));
if (refreshed.RefreshToken != "rotated") throw new Exception($"rotation differs: {refreshed.RefreshToken}");
var key = OAuth.TokenStoreKey("serviceOAuth", "https://auth.oauth.test/token-refresh", "client-id");
if ((await refreshStore.LoadAsync(key))?.RefreshToken != "rotated") throw new Exception("the rotated refresh token must be stored");
var retained = await OAuth.RefreshTokenAsync("serviceOAuth", "rotated", Options(refreshStore));
if (retained.AccessToken != "second-refresh-access" || retained.RefreshToken != "rotated") throw new Exception("a response without a refresh token must retain the stored one");
var refreshHits = hits.Where(h => h.Contains("/token-refresh")).ToList();
if (refreshHits.Count != 2 || !refreshHits[0].Contains("grant_type=refresh_token") || !refreshHits[0].Contains("refresh_token=stale")) throw new Exception($"refresh posts differ: {string.Join("|", refreshHits)}");

// Wrong credentials produce the typed error without any secret in the message.
hits.Clear();
var badOptions = Options() with { ClientSecret = "super-secret-value" };
try
{
    await OAuth.ClientCredentialsTokenAsync("serviceOAuth", badOptions);
    throw new Exception("bad credentials must fail");
}
catch (AuthException error)
{
    if (error.Kind != "invalid-client" || error.Status != 401 || error.Scheme != "serviceOAuth") throw new Exception($"typed metadata differs: {error.Kind} {error.Status} {error.Scheme}");
    if (error.Message.Contains("super-secret-value") || error.Message.Contains("client-secret") || error.Message.Contains("access-token")) throw new Exception($"the typed error leaked a credential: {error.Message}");
}

// Revocation posts to the configured endpoint with the compiled authentication;
// introspection returns the server's JSON object.
hits.Clear();
await OAuth.RevokeTokenAsync("serviceOAuth", "token-to-revoke", Options(), tokenTypeHint: "access_token");
var revokeHits = hits.Where(h => h.Contains("/revoke")).ToList();
if (revokeHits.Count != 1 || !revokeHits[0].Contains("token=token-to-revoke") || !revokeHits[0].Contains("token_type_hint=access_token")) throw new Exception($"revocation differs: {string.Join("|", revokeHits)}");
hits.Clear();
var introspected = await OAuth.IntrospectTokenAsync("serviceOAuth", "token-to-inspect", Options());
if (introspected.GetProperty("active").GetBoolean() != true || introspected.GetProperty("scope").GetString() != "read") throw new Exception($"introspection differs: {introspected}");

// Authorization code + PKCE: the begin URL carries the S256 challenge and
// random state; state mismatch is a typed error; the transaction is consumed
// exactly once; the happy path exchanges the code with the stored verifier.
hits.Clear();
var begin = OAuth.BeginAuthorization("serviceOAuth", "https://app.oauth.test/callback", Options(), ["read"]);
if (!begin.AuthorizationUrl.StartsWith("https://auth.oauth.test/authorize?")) throw new Exception($"authorization URL differs: {begin.AuthorizationUrl}");
var query = begin.AuthorizationUrl[(begin.AuthorizationUrl.IndexOf('?') + 1)..]
    .Split('&')
    .Select(pair => pair.Split('='))
    .ToDictionary(parts => Uri.UnescapeDataString(parts[0]), parts => Uri.UnescapeDataString(parts[1]), StringComparer.Ordinal);
foreach (var (name, expected) in new[]
{
    ("response_type", "code"),
    ("client_id", "client-id"),
    ("redirect_uri", "https://app.oauth.test/callback"),
    ("code_challenge_method", "S256"),
    ("scope", "read"),
})
{
    if (query.GetValueOrDefault(name) != expected) throw new Exception($"authorization URL {name} differs: {begin.AuthorizationUrl}");
}
if (query["code_challenge"].Length < 43) throw new Exception("S256 challenge is too short");
if (begin.State.Length < 22) throw new Exception("state is too short");
try
{
    await OAuth.CompleteAuthorizationAsync(begin.Transaction, "the-code", "wrong", Options());
    throw new Exception("a wrong state must fail");
}
catch (AuthException error) when (error.Kind == "state-mismatch") { }
try
{
    await OAuth.CompleteAuthorizationAsync(begin.Transaction, "the-code", begin.State, Options());
    throw new Exception("a replayed transaction must fail");
}
catch (AuthException error) when (error.Kind == "transaction-consumed") { }
var retry = OAuth.BeginAuthorization("serviceOAuth", "https://app.oauth.test/callback", Options());
var codeStore = new MemoryTokenStore();
var codeTokens = await OAuth.CompleteAuthorizationAsync(retry.Transaction, "the-code", retry.State, Options(codeStore));
if (codeTokens.AccessToken != "code-access") throw new Exception($"code exchange differs: {codeTokens.AccessToken}");
var codeKey = OAuth.TokenStoreKey("serviceOAuth", "https://auth.oauth.test/token", "client-id");
if ((await codeStore.LoadAsync(codeKey))?.AccessToken != "code-access") throw new Exception("the exchanged token set must be stored");
var codeHits = hits.Where(h => h.Contains("/token") && h.Contains("grant_type=authorization_code")).ToList();
if (codeHits.Count != 1 || !codeHits[0].Contains("code_verifier=")) throw new Exception($"code exchange request differs: {string.Join("|", codeHits)}");

// Device flow: authorization_pending polls again, then the typed token set
// arrives and lands in the store under the compiled identity.
hits.Clear();
Interlocked.Exchange(ref devicePolls, 0);
var deviceOptions = new OAuthClientOptions
{
    ClientId = "device-id",
    ClientSecret = "device-secret",
    Clock = clock,
    Transport = http,
    Store = new MemoryTokenStore(),
    Delay = (_, _) => Task.CompletedTask,
};
var deviceTokens = await OAuth.BeginDeviceAuthorizationAsync("deviceOAuth", deviceOptions);
if (deviceTokens.AccessToken != "device-access") throw new Exception($"device tokens differ: {deviceTokens.AccessToken}");
if (hits.Count(h => h.Contains("/device")) != 1) throw new Exception("the device code request differs");
var deviceHits = hits.Count(h => h.Contains("/token") && h.Contains("grant_type=urn"));
if (deviceHits != 2) throw new Exception($"authorization_pending must poll again: {deviceHits}");

// End to end through the generated operation runtime: the provider is an
// ordinary credential hook and the API request carries the acquired token.
hits.Clear();
Interlocked.Exchange(ref now, 20_000_000);
var credentials = new Credentials
{
    ServiceOAuth = OAuth.CreateClientCredentialsProvider("serviceOAuth", new OAuthClientOptions
    {
        ClientId = "client-id",
        ClientSecret = "client-secret",
        Clock = clock,
        Transport = http,
    }),
};
using var client = new Client(credentials, httpClient: http);
var page = await client.ListWidgetsAsync(new ListWidgetsInput());
if (page.Data.Ok != true) throw new Exception("the protected call decoded the wrong page");
var apiHits = hits.Where(h => h.Contains("/v1/widgets")).ToList();
if (apiHits.Count != 1) throw new Exception($"the protected call differs: {apiHits.Count}");
if (!apiHits[0].Contains("Bearer access-token-")) throw new Exception($"the API request must carry the acquired token: {apiHits[0]}");
if (hits.Count(h => h.Contains("/token")) != 1) throw new Exception("the runtime call must acquire exactly one token");

Console.WriteLine("oauth behavior verified");
return 0;

internal static class OAuthDriver
{
    // The fake authorization server: every hit is recorded as one
    // "method path|authorization|body" line and answered by grant.
    public static string Respond(HttpRequestMessage request, List<string> hits, Func<int> counter, Action<int> setCounter, Func<int> devicePolls, Action<int> setDevicePolls)
    {
        var path = request.RequestUri!.PathAndQuery;
        var authorization = request.Headers.TryGetValues("Authorization", out var values) ? string.Join(",", values) : "-";
        var body = request.Content is null ? "" : request.Content.ReadAsStringAsync().ConfigureAwait(false).GetAwaiter().GetResult();
        hits.Add($"{request.Method} {path}|{authorization}|{body}");
        string Json(string value, HttpStatusCode status = HttpStatusCode.OK) => $"{(int)status} application/json {value}";
        string Basic() => authorization.StartsWith("Basic ") ? Encoding.UTF8.GetString(Convert.FromBase64String(authorization[6..])) : "";
        var form = body.Split('&', StringSplitOptions.RemoveEmptyEntries)
            .Select(pair => pair.Split('='))
            .ToDictionary(parts => Uri.UnescapeDataString(parts[0]), parts => parts.Length > 1 ? Uri.UnescapeDataString(parts[1]) : "");
        var grant = form.GetValueOrDefault("grant_type");
        if (path.StartsWith("/token-refresh"))
        {
            if (Basic() != "client-id:client-secret") return Json("""{"error":"invalid_client"}""", HttpStatusCode.Unauthorized);
            return form.GetValueOrDefault("refresh_token") == "stale"
                ? Json("""{"access_token":"refreshed-access","token_type":"bearer","refresh_token":"rotated"}""")
                : Json("""{"access_token":"second-refresh-access","token_type":"bearer"}""");
        }
        if (path == "/token")
        {
            if (grant == "urn:ietf:params:oauth:grant-type:device_code")
            {
                if (Basic() != "device-id:device-secret") return Json("""{"error":"invalid_client"}""", HttpStatusCode.Unauthorized);
                setDevicePolls(devicePolls() + 1);
                if (devicePolls() == 1) return Json("""{"error":"authorization_pending"}""", HttpStatusCode.BadRequest);
                return Json("""{"access_token":"device-access","token_type":"Bearer","expires_in":3600}""");
            }
            if (Basic() != "client-id:client-secret") return Json("""{"error":"invalid_client"}""", HttpStatusCode.Unauthorized);
            if (grant == "authorization_code") return Json("""{"access_token":"code-access","token_type":"Bearer","expires_in":3600}""");
            setCounter(counter() + 1);
            return Json($$"""{"access_token":"access-token-{{counter()}}","token_type":"Bearer","expires_in":3600}""");
        }
        if (path.StartsWith("/device")) return Json("""{"device_code":"device-code-1","user_code":"ABCD-EFGH","verification_uri":"https://auth.oauth.test/activate","expires_in":30,"interval":0}""");
        if (path.StartsWith("/revoke")) return $"200 text/plain ";
        if (path.StartsWith("/introspect")) return Json("""{"active":true,"scope":"read"}""");
        if (path.StartsWith("/v1/widgets") || path.StartsWith("/v1/devices")) return Json("""{"ok":true}""");
        return $"404 text/plain missing {path}";
    }
}

sealed class Stub(Func<HttpRequestMessage, string> respond) : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var line = respond(request);
        var status = (HttpStatusCode)int.Parse(line[..3], System.Globalization.CultureInfo.InvariantCulture);
        var separator = line.IndexOf(' ');
        var media = line[(separator + 1)..line.IndexOf(' ', separator + 1)];
        var payload = line[(line.IndexOf(' ', separator + 1) + 1)..];
        var message = new HttpResponseMessage(status) { Content = new StringContent(payload, Encoding.UTF8, media) };
        return Task.FromResult(message);
    }
}
"#;

/// One OpenID Connect scheme (endpoints defined by the discovery document at
/// runtime) plus one OAuth2 scheme with a compiled token endpoint and
/// configured auxiliary endpoints, each used by an operation.
fn discovery_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"identity": ["openid"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }},
            "/gadgets": {"get": {
                "operationId": "listGadgets",
                "security": [{"service": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {
            "identity": {"type": "openIdConnect", "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"},
            "service": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn discovery_options() -> GenerationOptions {
    let identity = OAuthSchemeConfig {
        client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
        client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
        ..OAuthSchemeConfig::default()
    };
    let service = OAuthSchemeConfig {
        client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
        client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
        revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
        introspection_endpoint: Some("https://auth.oauth.test/introspect".into()),
        ..OAuthSchemeConfig::default()
    };
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: [
                    ("identity".to_owned(), identity),
                    ("service".to_owned(), service),
                ]
                .into_iter()
                .collect(),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

#[test]
fn discovery_schemes_emit_the_discovery_engine_and_per_provider_cache() {
    let configured = files_map(&generate(discovery_document(), &discovery_options()));
    let plain = files_map(&generate(
        discovery_document(),
        &GenerationOptions::default(),
    ));
    // The discovery policy adds exactly one file and changes no other byte.
    assert_eq!(
        configured.len(),
        plain.len() + 1,
        "the discovery plan may add exactly one file ({} vs {})",
        configured.len(),
        plain.len()
    );
    for (path, content) in &plain {
        assert_eq!(
            configured.get(path).map(String::as_str),
            Some(content.as_str()),
            "{path} changed under the discovery policy"
        );
    }
    let oauth = configured
        .get("csharp/src/OAuth.g.cs")
        .expect("OAuth lifecycle runtime emitted");
    for expected in [
        // The typed discovery document shape and the provider-owned cache.
        "internal sealed record DiscoveredEndpoints",
        "public string? TokenEndpoint { get; init; }",
        "internal sealed class DiscoveryCache",
        "private const int DiscoveryMaxBytes = 1 << 20;",
        // The engine: typed fetch with the issuer-origin rule, single-flight
        // gate and the endpoint-resolution precedence.
        "private static async Task<DiscoveredEndpoints> DiscoverAsync(OAuthSchemeDescriptor scheme, OAuthClientOptions clientOptions, DiscoveryCache? cache, CancellationToken cancellationToken)",
        "the discovery document issuer does not share the discovery URL origin",
        "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery",
        "private static async Task<string> ResolveEndpointAsync(OAuthSchemeDescriptor scheme, string? compiledEndpoint",
        "ExecutableFlowOrNull(compiled, \"client-credentials\")",
        "RefreshFlowOrNull(compiled)",
        "DiscoveryClientAuth(compiled, identity)",
        "var discovery = new DiscoveryCache();",
        // The compiled discovery URL lands in the frozen descriptor of the
        // discovery-defined scheme, whose endpoint fields stay empty.
        "[\"identity\"] = new(\"identity\", \"open-id-connect\", 30, \"https://authority.oauth.test/.well-known/openid-configuration\", null, null, \"SUSPECT_DISCOVERY_CLIENT_ID\", \"SUSPECT_DISCOVERY_CLIENT_SECRET\", new OAuthFlowDescriptor[]",
        // The compiled scheme keeps its endpoints; discovery only supplements.
        "[\"service\"] = new(\"service\", \"oauth2\", 30, null, \"https://auth.oauth.test/revoke\", \"https://auth.oauth.test/introspect\", \"SUSPECT_DISCOVERY_CLIENT_ID\", \"SUSPECT_DISCOVERY_CLIENT_SECRET\", new OAuthFlowDescriptor[]",
        "new(\"client-credentials\", null, \"https://auth.oauth.test/token\", null, null, \"client-secret-basic\", false,",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.g.cs lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // Control: a plan without any discovery URL emits no discovery engine.
    let undiscovered = source(
        &generate(oauth_document(), &oauth_options()),
        "csharp/src/OAuth.g.cs",
    );
    for absent in [
        "DiscoveredEndpoints",
        "DiscoveryCache",
        "discovery-failed",
        "DiscoverAsync",
        "DiscoveryClientAuth",
        "ExecutableFlowOrNull",
    ] {
        assert!(
            !undiscovered.contains(absent),
            "a plan without a discovery URL must not carry {absent}"
        );
    }
}

const DISCOVERY_DRIVER: &str = r#"
using System.Net;
using System.Text;
using Acme.OauthSdk;

var stub = new DiscoveryStub();
using var http = new HttpClient(stub);
var now = 1_000_000L;
Func<DateTimeOffset> clock = () => DateTimeOffset.UnixEpoch.AddMilliseconds(Interlocked.Read(ref now));

OAuthClientOptions Options(ITokenStore? store = null) => new()
{
    ClientId = "client-id",
    ClientSecret = "client-secret",
    Clock = clock,
    Transport = http,
    Store = store,
};

// 1. One-shot acquisition resolves the discovered token endpoint and
//    authenticates with basic; the compiled fallback endpoint is never
//    contacted and the discovery request carries accept: application/json.
var tokens = await OAuth.ClientCredentialsTokenAsync("identity", Options(new MemoryTokenStore()));
if (tokens.AccessToken != "discovered-1") throw new Exception($"acquisition differs: {tokens.AccessToken}");
if (stub.DiscoveryHits() != 1) throw new Exception($"discovery fetches differ: {stub.DiscoveryHits()}");
var discovery = stub.RequestsTo("/.well-known/openid-configuration");
if (discovery.Count != 1 || discovery[0].Method != "GET" || discovery[0].Accept != "application/json") throw new Exception($"the discovery request differs: {discovery.Count}");
if (!discovery[0].Url.StartsWith("https://authority.oauth.test/.well-known/openid-configuration")) throw new Exception($"the discovery URL differs: {discovery[0].Url}");
var tokenHits = stub.RequestsTo("/oauth/token");
if (tokenHits.Count != 1 || !tokenHits[0].Url.StartsWith("https://authority.oauth.test/oauth/token")) throw new Exception($"the acquisition did not target the discovered endpoint: {tokenHits.Count}");
if (tokenHits[0].Body != "grant_type=client_credentials") throw new Exception($"the token form differs: {tokenHits[0].Body}");
if (!tokenHits[0].Authorization.StartsWith("Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=")) throw new Exception($"the discovery-defined client did not authenticate with basic: {tokenHits[0].Authorization}");
if (stub.RequestsTo("/token").Count != 0) throw new Exception("the compiled fallback endpoint was contacted");

// 2. Provider endpoints are cached per provider instance: the first client
//    call fetches discovery once and acquires once; the second reuses both.
var store = new MemoryTokenStore();
var credentials = new Credentials
{
    Identity = OAuth.CreateClientCredentialsProvider("identity", Options(store)),
    Service = OAuth.CreateClientCredentialsProvider("service", Options(new MemoryTokenStore())),
};
using var client = new Client(credentials, httpClient: http);
var providerBefore = (stub.DiscoveryHits(), stub.TokenHits("/oauth/token"));
var page = await client.ListWidgetsAsync(new ListWidgetsInput());
if (page.Data != "widgets") throw new Exception($"the protected call decoded the wrong page: {page.Data}");
if (stub.DiscoveryHits() != providerBefore.Item1 + 1) throw new Exception($"the provider must fetch its own document: {providerBefore.Item1}->{stub.DiscoveryHits()}");
if (stub.TokenHits("/oauth/token") != providerBefore.Item2 + 1) throw new Exception($"the provider must acquire once: {providerBefore.Item2}->{stub.TokenHits("/oauth/token")}");
await client.ListWidgetsAsync(new ListWidgetsInput());
if (stub.DiscoveryHits() != providerBefore.Item1 + 1 || stub.TokenHits("/oauth/token") != providerBefore.Item2 + 1) throw new Exception("the cache hit re-fetched");

// 3. After expiry the provider re-acquires through the still-cached document.
Interlocked.Add(ref now, 4_000_000);
await client.ListWidgetsAsync(new ListWidgetsInput());
if (stub.DiscoveryHits() != providerBefore.Item1 + 1) throw new Exception("the cached document was discarded on re-acquisition");
if (stub.TokenHits("/oauth/token") != providerBefore.Item2 + 2) throw new Exception($"the expired token was not re-acquired: {providerBefore.Item2}->{stub.TokenHits("/oauth/token")}");

// 4. Concurrent callers share exactly one discovery fetch and one acquisition.
var before = (stub.DiscoveryHits(), stub.TokenHits("/oauth/token"));
var flightCredentials = new Credentials { Identity = OAuth.CreateClientCredentialsProvider("identity", Options(new MemoryTokenStore())) };
using var flightClient = new Client(flightCredentials, httpClient: http);
var shared = await Task.WhenAll(
    flightClient.ListWidgetsAsync(new ListWidgetsInput()),
    flightClient.ListWidgetsAsync(new ListWidgetsInput()));
if (shared[0].Data != "widgets" || shared[1].Data != "widgets") throw new Exception("the concurrent calls decoded the wrong pages");
var after = (stub.DiscoveryHits(), stub.TokenHits("/oauth/token"));
if (after.Item1 != before.Item1 + 1 || after.Item2 != before.Item2 + 1) throw new Exception($"single-flight differs: discovery {before.Item1}->{after.Item1}, tokens {before.Item2}->{after.Item2}");

// 5. An issuer from another origin is a typed discovery failure whose message
//    carries no response body text; the next call retries.
stub.Issuer = "https://elsewhere.oauth.test";
try
{
    await OAuth.ClientCredentialsTokenAsync("identity", Options(new MemoryTokenStore()));
    throw new Exception("an issuer from another origin must fail");
}
catch (AuthException error)
{
    if (error.Kind != "discovery-failed" || error.Scheme != "identity") throw new Exception($"typed metadata differs: {error.Kind} {error.Scheme}");
    if (error.Message.Contains("boom") || error.Message.Contains("elsewhere")) throw new Exception($"the typed error leaked body text: {error.Message}");
}
stub.Issuer = "https://authority.oauth.test";
var mismatchRetried = await OAuth.ClientCredentialsTokenAsync("identity", Options(new MemoryTokenStore()));
if (mismatchRetried.AccessToken != "discovered-1") throw new Exception("the issuer retry decoded the wrong token");

// 6. A failing discovery request is a typed failure with the status, and the
//    next call retries; through a provider the failed fetch is never cached,
//    so the same provider instance recovers on its next attach.
stub.FailStatus = 500;
try
{
    await OAuth.ClientCredentialsTokenAsync("identity", Options(new MemoryTokenStore()));
    throw new Exception("a failing discovery request must fail");
}
catch (AuthException error)
{
    if (error.Kind != "discovery-failed" || error.Status != 500) throw new Exception($"the failed request differs: {error.Kind} {error.Status}");
}
stub.FailStatus = 0;
var failedCredentials = new Credentials { Identity = OAuth.CreateClientCredentialsProvider("identity", Options(new MemoryTokenStore())) };
using var failedClient = new Client(failedCredentials, httpClient: http);
stub.FailStatus = 500;
try
{
    await failedClient.ListWidgetsAsync(new ListWidgetsInput());
    throw new Exception("a failing provider discovery request must fail");
}
catch (SdkException error) when (error.Kind == SdkErrorKind.Authentication) { }
var failedDiscovery = stub.DiscoveryHits();
stub.FailStatus = 0;
var repaired = await failedClient.ListWidgetsAsync(new ListWidgetsInput());
if (repaired.Data != "widgets") throw new Exception("the repaired fetch decoded the wrong page");
if (stub.DiscoveryHits() != failedDiscovery + 1) throw new Exception($"the failed fetch was cached or re-fetched twice: {failedDiscovery}->{stub.DiscoveryHits()}");

// 7. Compiled precedence: the service scheme's compiled token URL wins and
//    never fetches discovery.
var discoveryBefore = stub.DiscoveryHits();
var serviceBefore = stub.TokenHits("/token");
var gadgets = await client.ListGadgetsAsync(new ListGadgetsInput());
if (gadgets.Data != "gadgets") throw new Exception($"the compiled scheme decoded the wrong page: {gadgets.Data}");
if (stub.TokenHits("/token") != serviceBefore + 1) throw new Exception("the compiled token endpoint differs");
if (!stub.RequestsTo("/token")[^1].Body.Contains("grant_type=client_credentials")) throw new Exception("the compiled grant differs");
if (stub.DiscoveryHits() != discoveryBefore) throw new Exception("the compiled scheme must not fetch discovery");

// 8. Revocation and introspection resolve through the discovery document for
//    the discovery-defined scheme.
await OAuth.RevokeTokenAsync("identity", "the-token-value", Options());
var revocations = stub.RequestsTo("/oauth/revoke");
if (revocations.Count != 1 || !revocations[0].Body.Contains("token=the-token-value")) throw new Exception($"revocation differs: {revocations.Count}");
if (!revocations[0].Authorization.StartsWith("Basic ")) throw new Exception("revocation must authenticate with basic");
var introspected = await OAuth.IntrospectTokenAsync("identity", "the-token-value", Options());
if (introspected.GetProperty("active").GetBoolean() != true || introspected.GetProperty("scope").GetString() != "read") throw new Exception($"introspection differs: {introspected}");

// 9. Explicit refresh resolves the discovery-defined token endpoint and the
//    rotated (here: refreshed) set lands in the store.
var refreshStore = new MemoryTokenStore();
var seeded = await OAuth.ClientCredentialsTokenAsync("identity", Options(refreshStore));
if (seeded.RefreshToken != "rotated-1") throw new Exception($"the seeded set differs: {seeded.RefreshToken}");
var refreshed = await OAuth.RefreshTokenAsync("identity", seeded.RefreshToken!, Options(refreshStore));
if (refreshed.AccessToken != "refreshed-access") throw new Exception($"the refresh differs: {refreshed.AccessToken}");
var stored = await refreshStore.LoadAsync(OAuth.TokenStoreKey("identity", "https://authority.oauth.test/oauth/token", "client-id"));
if (stored?.AccessToken != "refreshed-access") throw new Exception("the refreshed set must be stored under the discovered endpoint key");
var refreshPosts = stub.RequestsTo("/oauth/token").Where(h => h.Body.Contains("grant_type=refresh_token")).ToList();
if (refreshPosts.Count != 1) throw new Exception($"the refresh posts differ: {refreshPosts.Count}");

Console.WriteLine("discovery behavior verified");
return 0;

internal sealed record Hit(string Method, string Url, string Path, string Authorization, string Accept, string Body);

sealed class DiscoveryStub : HttpMessageHandler
{
    private readonly object _gate = new();
    private readonly List<Hit> _hits = new();
    internal string Issuer = "https://authority.oauth.test";
    internal int FailStatus;

    internal int Count(Func<Hit, bool> match)
    {
        lock (_gate) { return _hits.Count(match); }
    }

    internal List<Hit> RequestsTo(string path)
    {
        lock (_gate) { return _hits.Where(hit => hit.Path == path).ToList(); }
    }

    internal int DiscoveryHits() => Count(hit => hit.Path == "/.well-known/openid-configuration");
    internal int TokenHits(string path) => Count(hit => hit.Path == path);

    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var path = request.RequestUri!.AbsolutePath;
        var authorization = request.Headers.TryGetValues("Authorization", out var values) ? string.Join(",", values) : "-";
        var accept = request.Headers.Accept.FirstOrDefault()?.ToString() ?? "-";
        var body = request.Content is null ? "" : request.Content.ReadAsStringAsync().ConfigureAwait(false).GetAwaiter().GetResult();
        lock (_gate) { _hits.Add(new Hit(request.Method.Method, request.RequestUri.ToString(), path, authorization, accept, body)); }
        HttpResponseMessage Answer(string payload, HttpStatusCode status = HttpStatusCode.OK) => new(status) { Content = new StringContent(payload, Encoding.UTF8, "application/json") };
        int fail;
        string issuer;
        lock (_gate) { fail = FailStatus; issuer = Issuer; }
        if (path == "/.well-known/openid-configuration")
        {
            if (fail != 0) return Task.FromResult(Answer("""{"error":"boom"}""", (HttpStatusCode)fail));
            return Task.FromResult(Answer($$$"""{"issuer":"{{{issuer}}}","token_endpoint":"https://authority.oauth.test/oauth/token","revocation_endpoint":"https://authority.oauth.test/oauth/revoke","introspection_endpoint":"https://authority.oauth.test/oauth/introspect","unknown_member":{"nested":true}}"""));
        }
        var form = body.Split('&', StringSplitOptions.RemoveEmptyEntries)
            .Select(pair => pair.Split('='))
            .ToDictionary(parts => Uri.UnescapeDataString(parts[0]), parts => parts.Length > 1 ? Uri.UnescapeDataString(parts[1]) : "");
        if (path == "/oauth/token")
        {
            if (form.GetValueOrDefault("grant_type") == "refresh_token") return Task.FromResult(Answer("""{"access_token":"refreshed-access","token_type":"Bearer","expires_in":3600}"""));
            return Task.FromResult(Answer("""{"access_token":"discovered-1","token_type":"Bearer","expires_in":3600,"refresh_token":"rotated-1"}"""));
        }
        if (path == "/token") return Task.FromResult(Answer("""{"access_token":"compiled-access","token_type":"Bearer","expires_in":3600}"""));
        if (path == "/oauth/revoke") return Task.FromResult(Answer(""));
        if (path == "/oauth/introspect") return Task.FromResult(Answer("""{"active":true,"scope":"read"}"""));
        if (path == "/v1/widgets") return Task.FromResult(Answer("\"widgets\""));
        if (path == "/v1/gadgets") return Task.FromResult(Answer("\"gadgets\""));
        return Task.FromResult(Answer($"missing {path}", HttpStatusCode.NotFound));
    }
}
"#;

fn discovery_run(
    label: &str,
    directory: &str,
    arguments: &[&str],
    root: &Path,
    dotnet: &str,
    logs: bool,
) -> std::process::Output {
    let output = Command::new(dotnet)
        .args(arguments)
        .current_dir(root.join(directory))
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_HOME", root.join("dotnet-home"))
        .env("NUGET_PACKAGES", root.join("nuget-cache"))
        .output()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    if logs {
        fs::write(
            root.join(format!("logs/{label}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            root.join(format!("logs/{label}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
    }
    assert!(
        output.status.success(),
        "{label}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn native_discovery_lifecycle_drives_a_stubbed_http_handler() {
    let Some(dotnet) = dotnet() else {
        eprintln!("csharp_oauth: dotnet is not installed; degrading to static assertions");
        return;
    };
    eprintln!("csharp_oauth: {dotnet}");
    let parent =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-oauth-discovery");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("oauth-discovery-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(discovery_document(), &discovery_options()), &root)
        .unwrap();
    fs::write(
        root.join("global.json"),
        b"{\"sdk\":{\"version\":\"8.0.424\",\"rollForward\":\"disable\"}}",
    )
    .unwrap();
    fs::create_dir_all(root.join("feed")).unwrap();
    fs::write(
        root.join("NuGet.Config"),
        b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    discovery_run(
        "restore",
        "csharp",
        &[
            "restore",
            "Suspect.csproj",
            "--configfile",
            "../NuGet.Config",
        ],
        &root,
        &dotnet,
        true,
    );
    discovery_run(
        "build",
        "csharp",
        &[
            "build",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-restore",
            "-m:1",
        ],
        &root,
        &dotnet,
        true,
    );
    discovery_run(
        "pack",
        "csharp",
        &[
            "pack",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-build",
            "-o",
            "../feed",
        ],
        &root,
        &dotnet,
        true,
    );
    let package = root.join("feed/acme.oauth-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.oauth-sdk", "net8.0"),
    )
    .unwrap();
    fs::write(root.join("consumer/Program.cs"), DISCOVERY_DRIVER).unwrap();
    discovery_run(
        "consumer-restore",
        "consumer",
        &["restore", "--configfile", "../NuGet.Config"],
        &root,
        &dotnet,
        true,
    );
    discovery_run(
        "consumer-build",
        "consumer",
        &["build", "-c", "Release", "--no-restore", "-m:1"],
        &root,
        &dotnet,
        true,
    );
    discovery_run(
        "consumer-run",
        "consumer",
        &["run", "-c", "Release", "--no-build"],
        &root,
        &dotnet,
        true,
    );
    let assets: Value =
        serde_json::from_slice(&fs::read(root.join("consumer/obj/project.assets.json")).unwrap())
            .unwrap();
    assert_eq!(
        assets["libraries"]["acme.oauth-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}

/// The replay fixture: one client-credentials scheme over a JSON operation and
/// one over a streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
            }},
            "/events": {"get": {
                "operationId": "streamEvents",
                "security": [{"feedOAuth": ["read"]}],
                "responses": {"200": {"description": "Events", "content": {"text/event-stream": {
                    "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                }}}}
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}},
            "feedOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/feed-token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_REPLAY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_REPLAY_CLIENT_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "feedOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_REPLAY_FEED_ID".into()),
                            client_secret_env: Some("SUSPECT_REPLAY_FEED_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations in the full
/// document-and-pointer form the generated runtime reads back from the attach
/// context.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(replay_document(), &replay_options());
    let oauth = source(&files, "csharp/src/OAuth.g.cs");
    for expected in [
        // The opt-in factory and the wrapper it returns, nested in the OAuth
        // class so the plain lifecycle stays byte-identical.
        "public static ReplayingCredentials CreateReplayingCredentialsProvider(string scheme, OAuthClientOptions? options = null)",
        "public sealed class ReplayingCredentials",
        "public async ValueTask<AuthorizationValue> Attach(CredentialContext context, CancellationToken cancellationToken)",
        "public HttpMessageHandler Transport(HttpMessageHandler inner)",
        "triggers exactly one coordinated refresh",
        "delivered stream data prevents a transparent restart",
        "A refresh failure surfaces as the typed AuthException instead of a replay",
        // The coordinated refresh: one round per store key, a newer stored set
        // wins, a failed round fails every waiter exactly once.
        "private readonly Dictionary<string, Task<AuthorizationValue>> _rounds = new(StringComparer.Ordinal);",
        "if (!string.Equals(current.Scheme + \" \" + current.Parameter, presented, StringComparison.Ordinal)) return current;",
        "await _store.ClearAsync(key, cancellationToken).ConfigureAwait(false);",
        // The one replay: a fresh Authorization header through the same inner
        // transport, and the second response surfaced whatever it is.
        "replayed.Headers.TryAddWithoutValidation(\"Authorization\", $\"{fresh.Scheme} {fresh.Parameter}\");",
        // The compiled stream-protection table, in the form the attach context
        // reports: the streaming operation's requirement pointer is compiled
        // into the feed scheme's no-replay set.
        "private static readonly IReadOnlyDictionary<string, IReadOnlyList<string>> NoReplayRequirements",
        "\"https://source.oauth.test/csharp-openapi.json#/paths/~1events/get/security/0/feedOAuth\"",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.g.cs lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("~1widgets/get/security/0/serviceOAuth"));

    // The replay machinery stays out of a package without any executable
    // client-credentials flow: an authorization-code-only scheme compiles
    // exactly the pre-replay bytes.
    let code_only_document = json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {"/widgets": {"get": {
            "operationId": "listWidgets",
            "security": [{"userOAuth": ["read"]}],
            "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {
                "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}
            }}}}}
        }}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    });
    let code_only_options = GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([(
                    "userOAuth".to_owned(),
                    OAuthSchemeConfig {
                        client_id_env: Some("SUSPECT_CODE_ONLY_ID".into()),
                        client_secret_env: Some("SUSPECT_CODE_ONLY_SECRET".into()),
                        ..OAuthSchemeConfig::default()
                    },
                )]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    };
    let code_only = generate(code_only_document, &code_only_options);
    let plain_oauth = source(&code_only, "csharp/src/OAuth.g.cs");
    assert!(!plain_oauth.contains("ReplayingCredentials"));
    assert!(!plain_oauth.contains("NoReplayRequirements"));
    assert!(!plain_oauth.contains("CreateReplayingCredentialsProvider"));
    // The plain lifecycle stays exactly the pre-replay emission: the wrapper
    // only ever appends, and the plain provider keeps attach-only semantics.
    assert!(plain_oauth.contains("public static AuthorizationProvider CreateClientCredentialsProvider(string scheme, OAuthClientOptions? options = null)"));
    assert!(plain_oauth.contains("When a protected call still fails with a declared 401, call RefreshTokenAsync explicitly and retry; the SDK itself never retries."));
}

const REPLAY_DRIVER: &str = r#"
using System.Net;
using System.Text;
using Acme.OauthSdk;

// The fake authorization and API server. The API answers 401 for the first
// token of a scheme and 200 afterwards, so the driver can count exactly one
// refresh and one replay; Mode and FailFrom flip per scenario. Every response
// yields once before it is answered, so concurrent attaches, sends and
// refreshes interleave like they do over a real transport.

// (a) 401 then success: one refresh, one replay, and the caller sees 200.
// (b) 401 then 401: the second 401 surfaces and exactly one refresh ran.
// (c) concurrent 401s: ONE shared refresh, two replays.
// (d) a streaming operation is never replayed: the typed 401 surfaces.
// (e) replay disabled by default: the plain provider surfaces the 401
//     without any refresh.
// (f) refresh failure: the failure surfaces instead of a replay.

var stub = new ReplayStub();
var now = 1_000_000L;
Func<DateTimeOffset> clock = () => DateTimeOffset.UnixEpoch.AddMilliseconds(Interlocked.Read(ref now));
using var tokenTransport = new HttpClient(stub);

OAuthClientOptions TokenOptions() => new()
{
    ClientId = "client-id",
    ClientSecret = "client-secret",
    Clock = clock,
    Transport = tokenTransport,
};
OAuthClientOptions FeedOptions() => new()
{
    ClientId = "feed-id",
    ClientSecret = "feed-secret",
    Clock = clock,
    Transport = tokenTransport,
};

// (a) one replay, one refresh, and the fresh token reached the wire.
stub.Reset();
stub.StaleNext = true;
var replaying = OAuth.CreateReplayingCredentialsProvider("serviceOAuth", TokenOptions());
var credentials = new Credentials { ServiceOAuth = replaying.Attach };
using var transport = new HttpClient(replaying.Transport(stub));
using (var client = new Client(credentials, httpClient: transport))
{
    var page = await client.ListWidgetsAsync(new ListWidgetsInput());
    if (page.Data.Ok != true) throw new Exception($"the replayed call decoded the wrong page: {page.Data.Ok}");
}
if (stub.Count("/v1/widgets") != 2) throw new Exception($"exactly one replay differs: {stub.Count("/v1/widgets")}");
if (stub.Count("/token") != 2) throw new Exception($"exactly one token refresh differs: {stub.Count("/token")}");
if (stub.Count("/feed-token") != 0) throw new Exception("the feed scheme must not be contacted");
var widgetHits = stub.AuthorizationsTo("/v1/widgets");
if (widgetHits.Count != 2) throw new Exception($"the API request count differs: {widgetHits.Count}");
if (widgetHits[0] is null || widgetHits[0] == widgetHits[1]) throw new Exception($"the replay must carry the fresh token: {string.Join("|", widgetHits)}");

// (b) the same provider: its stored fresh token attaches without a token
//     request, the forced refresh runs once, and the second 401 surfaces.
stub.Reset();
stub.Mode = ReplayMode.Always401;
try
{
    using (var client = new Client(credentials, httpClient: transport))
    {
        await client.ListWidgetsAsync(new ListWidgetsInput());
    }
    throw new Exception("a second 401 must surface");
}
catch (SdkException error) when (error.Kind == SdkErrorKind.UnexpectedResponse)
{
    if (error.Response?.Status != 401) throw new Exception($"the second 401 differs: {error.Response?.Status}");
}
if (stub.Count("/v1/widgets") != 2) throw new Exception($"one replay, no loops differs: {stub.Count("/v1/widgets")}");
if (stub.Count("/token") != 1) throw new Exception($"exactly one refresh differs: {stub.Count("/token")}");

// (c) concurrent 401s: both calls attach the same stale token, share exactly
//     one refresh round and each replays exactly once. The stub barrier holds
//     both answers until both calls have arrived, so the two attaches and the
//     two 401s are ordered before either refresh.
stub.Reset();
stub.Mode = ReplayMode.First401;
stub.StaleNext = true;
stub.WidgetsBarrier = 2;
var shared = OAuth.CreateReplayingCredentialsProvider("serviceOAuth", TokenOptions());
var sharedCredentials = new Credentials { ServiceOAuth = shared.Attach };
using var sharedTransport = new HttpClient(shared.Transport(stub));
using (var client = new Client(sharedCredentials, httpClient: sharedTransport))
{
    var pages = await Task.WhenAll(
        client.ListWidgetsAsync(new ListWidgetsInput()),
        client.ListWidgetsAsync(new ListWidgetsInput()));
    if (pages.Any(page => page.Data.Ok != true)) throw new Exception("the concurrent replays decoded the wrong pages");
}
if (stub.Count("/v1/widgets") != 4) throw new Exception($"two replays differ: {stub.Count("/v1/widgets")}");
if (stub.Count("/token") != 2) throw new Exception($"one shared refresh differs: {stub.Count("/token")}");
var sharedHits = stub.AuthorizationsTo("/v1/widgets");
if (sharedHits.Distinct().Count() != 2) throw new Exception($"the two stale attaches must share one token: {string.Join("|", sharedHits)}");

// (d) a streaming operation is never replayed: the feed attach is recorded
//     ineligible, so the typed 401 surfaces and no refresh runs.
stub.Reset();
var feed = OAuth.CreateReplayingCredentialsProvider("feedOAuth", FeedOptions());
var feedCredentials = new Credentials { FeedOAuth = feed.Attach };
using var feedTransport = new HttpClient(feed.Transport(stub));
using (var client = new Client(feedCredentials, httpClient: feedTransport))
{
    try
    {
        await client.StreamEventsAsync(new StreamEventsInput());
        throw new Exception("the streaming 401 must surface");
    }
    catch (SdkException error) when (error.Kind == SdkErrorKind.UnexpectedResponse)
    {
        if (error.Response?.Status != 401) throw new Exception($"the streaming 401 differs: {error.Response?.Status}");
    }
}
if (stub.Count("/v1/events") != 1) throw new Exception($"no replay for the streaming operation differs: {stub.Count("/v1/events")}");
if (stub.Count("/feed-token") != 1) throw new Exception($"no refresh for the streaming operation differs: {stub.Count("/feed-token")}");

// (e) replay disabled by default: the plain provider surfaces the 401
//     without any refresh.
stub.Reset();
stub.StaleNext = true;
var plain = OAuth.CreateClientCredentialsProvider("serviceOAuth", TokenOptions());
var plainCredentials = new Credentials { ServiceOAuth = plain };
using (var client = new Client(plainCredentials, httpClient: tokenTransport))
{
    try
    {
        await client.ListWidgetsAsync(new ListWidgetsInput());
        throw new Exception("the plain 401 must surface");
    }
    catch (SdkException error) when (error.Kind == SdkErrorKind.UnexpectedResponse)
    {
        if (error.Response?.Status != 401) throw new Exception($"the plain 401 differs: {error.Response?.Status}");
    }
}
if (stub.Count("/v1/widgets") != 1) throw new Exception($"no replay differs: {stub.Count("/v1/widgets")}");
if (stub.Count("/token") != 1) throw new Exception($"no refresh differs: {stub.Count("/token")}");

// (f) refresh failure: the failure surfaces instead of a replay, and the
//     budget still holds: exactly one refresh was attempted, exactly once.
stub.Reset();
stub.StaleNext = true;
stub.FailFrom = stub.ServiceTokens + 2;
var failing = OAuth.CreateReplayingCredentialsProvider("serviceOAuth", TokenOptions());
var failingCredentials = new Credentials { ServiceOAuth = failing.Attach };
using var failingTransport = new HttpClient(failing.Transport(stub));
using (var client = new Client(failingCredentials, httpClient: failingTransport))
{
    try
    {
        await client.ListWidgetsAsync(new ListWidgetsInput());
        throw new Exception("a failed refresh must surface");
    }
    catch (SdkException error)
    {
        // The generated runtime wraps the typed AuthException as a transport
        // failure; the replay budget is what this driver pins.
        if (error.Kind != SdkErrorKind.Transport) throw new Exception($"the failed refresh differs: {error.Kind}");
    }
}
if (stub.Count("/v1/widgets") != 1) throw new Exception($"no replay after a failed refresh differs: {stub.Count("/v1/widgets")}");
if (stub.Count("/token") != 2) throw new Exception($"the refresh was attempted exactly once differs: {stub.Count("/token")}");

Console.WriteLine("replay behavior verified");
return 0;

internal sealed record Hit(string Method, string Path, string Authorization, string Body);

internal enum ReplayMode { First401, Always401 }

sealed class ReplayStub : HttpMessageHandler
{
    internal readonly List<Hit> Hits = new();
    internal int ServiceTokens;
    internal int FeedTokens;
    internal int FailFrom = int.MaxValue;
    internal string? StaleToken;
    internal bool StaleNext;
    internal ReplayMode Mode = ReplayMode.First401;
    // When above zero, a /v1/widgets request is answered only after that many
    // have arrived, so the concurrent scenario can pin the order: both calls
    // attach and both receive their 401 before either refresh runs.
    internal int WidgetsBarrier;
    private int _arrived;
    private TaskCompletionSource _barrier = NewBarrier();
    private readonly object _gate = new();

    private static TaskCompletionSource NewBarrier() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    internal int Count(string path) { lock (_gate) { return Hits.Count(hit => hit.Path == path); } }
    internal List<string> AuthorizationsTo(string path) { lock (_gate) { return Hits.Where(hit => hit.Path == path).Select(hit => hit.Authorization).ToList(); } }
    internal void Reset()
    {
        lock (_gate)
        {
            Hits.Clear();
            WidgetsBarrier = 0;
            _arrived = 0;
            _barrier = NewBarrier();
        }
    }

    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var path = request.RequestUri!.AbsolutePath;
        var authorization = request.Headers.TryGetValues("Authorization", out var values) ? string.Join(",", values) : "-";
        var body = request.Content is null ? "" : request.Content.ReadAsStringAsync().ConfigureAwait(false).GetAwaiter().GetResult();
        var barrier = Task.CompletedTask;
        HttpResponseMessage answer;
        lock (_gate)
        {
            Hits.Add(new Hit(request.Method.Method, path, authorization, body));
            HttpResponseMessage Json(string payload, HttpStatusCode status = HttpStatusCode.OK) => new(status) { Content = new StringContent(payload, Encoding.UTF8, "application/json") };
            if (path == "/token")
            {
                ServiceTokens += 1;
                if (ServiceTokens >= FailFrom) answer = Json("""{"error":"server_error"}""", HttpStatusCode.InternalServerError);
                else
                {
                    var token = $"svc-{ServiceTokens}";
                    if (StaleNext) { StaleToken = token; StaleNext = false; }
                    answer = Json($$"""{"access_token":"{{token}}","token_type":"Bearer","expires_in":3600}""");
                }
            }
            else if (path == "/feed-token")
            {
                FeedTokens += 1;
                answer = Json($$"""{"access_token":"feed-{{FeedTokens}}","token_type":"Bearer","expires_in":3600}""");
            }
            else if (path == "/v1/widgets")
            {
                if (WidgetsBarrier > 0)
                {
                    _arrived += 1;
                    if (_arrived >= WidgetsBarrier) _barrier.TrySetResult();
                    barrier = _barrier.Task;
                }
                if (Mode == ReplayMode.Always401) answer = Json("""{"error":"unauthorized"}""", HttpStatusCode.Unauthorized);
                else if (StaleToken is not null && authorization == $"Bearer {StaleToken}") answer = Json("""{"error":"stale"}""", HttpStatusCode.Unauthorized);
                else answer = Json("""{"ok":true}""");
            }
            else if (path == "/v1/events") answer = Json("""{"error":"stream-denied"}""", HttpStatusCode.Unauthorized);
            else answer = new HttpResponseMessage(HttpStatusCode.NotFound) { Content = new StringContent($"missing {path}", Encoding.UTF8, "text/plain") };
        }
        // Token endpoint answers are synchronous, so attaches never suspend
        // inside the acquisition gate. API answers yield once, and the
        // concurrent scenario's barrier holds its answers until both calls
        // have arrived.
        if (!barrier.IsCompletedSuccessfully) return Waited(barrier, answer);
        if (path == "/token" || path == "/feed-token") return Task.FromResult(answer);
        return Yielded(answer);
    }

    private static async Task<HttpResponseMessage> Waited(Task barrier, HttpResponseMessage answer)
    {
        await barrier.ConfigureAwait(false);
        return answer;
    }

    private static async Task<HttpResponseMessage> Yielded(HttpResponseMessage answer)
    {
        await Task.Yield();
        return answer;
    }
}
"#;

#[test]
fn replay_lifecycle_drives_a_stubbed_http_handler() {
    let Some(dotnet) = dotnet() else {
        eprintln!("csharp_oauth: dotnet is not installed; degrading to static assertions");
        return;
    };
    eprintln!("csharp_oauth: {dotnet}");
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-oauth-replay");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("oauth-replay-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(replay_document(), &replay_options()), &root).unwrap();
    fs::write(
        root.join("global.json"),
        b"{\"sdk\":{\"version\":\"8.0.424\",\"rollForward\":\"disable\"}}",
    )
    .unwrap();
    fs::create_dir_all(root.join("feed")).unwrap();
    fs::write(
        root.join("NuGet.Config"),
        b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    let run = |label: &str, directory: &str, arguments: &[&str]| {
        let output = Command::new(&dotnet)
            .args(arguments)
            .current_dir(root.join(directory))
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_HOME", root.join("dotnet-home"))
            .env("NUGET_PACKAGES", root.join("nuget-cache"))
            .output()
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        fs::write(
            root.join(format!("logs/{label}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            root.join(format!("logs/{label}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{label}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(
        "restore",
        "csharp",
        &[
            "restore",
            "Suspect.csproj",
            "--configfile",
            "../NuGet.Config",
        ],
    );
    run(
        "build",
        "csharp",
        &[
            "build",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-restore",
            "-m:1",
        ],
    );
    run(
        "pack",
        "csharp",
        &[
            "pack",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-build",
            "-o",
            "../feed",
        ],
    );
    let package = root.join("feed/acme.oauth-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.oauth-sdk", "net8.0"),
    )
    .unwrap();
    fs::write(root.join("consumer/Program.cs"), REPLAY_DRIVER).unwrap();
    run(
        "consumer-restore",
        "consumer",
        &["restore", "--configfile", "../NuGet.Config"],
    );
    run(
        "consumer-build",
        "consumer",
        &["build", "-c", "Release", "--no-restore", "-m:1"],
    );
    run(
        "consumer-run",
        "consumer",
        &["run", "-c", "Release", "--no-build"],
    );
    let assets: Value =
        serde_json::from_slice(&fs::read(root.join("consumer/obj/project.assets.json")).unwrap())
            .unwrap();
    assert_eq!(
        assets["libraries"]["acme.oauth-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}

#[test]
fn native_oauth_lifecycle_drives_a_stubbed_http_handler() {
    let Some(dotnet) = dotnet() else {
        eprintln!("csharp_oauth: dotnet is not installed; degrading to static assertions");
        return;
    };
    eprintln!("csharp_oauth: {dotnet}");
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-oauth");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("oauth-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(oauth_document(), &oauth_options()), &root).unwrap();
    fs::write(
        root.join("global.json"),
        b"{\"sdk\":{\"version\":\"8.0.424\",\"rollForward\":\"disable\"}}",
    )
    .unwrap();
    fs::create_dir_all(root.join("feed")).unwrap();
    fs::write(
        root.join("NuGet.Config"),
        b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    let run = |label: &str, directory: &str, arguments: &[&str]| {
        let output = Command::new(&dotnet)
            .args(arguments)
            .current_dir(root.join(directory))
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_HOME", root.join("dotnet-home"))
            .env("NUGET_PACKAGES", root.join("nuget-cache"))
            .output()
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        fs::write(
            root.join(format!("logs/{label}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            root.join(format!("logs/{label}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{label}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(
        "restore",
        "csharp",
        &[
            "restore",
            "Suspect.csproj",
            "--configfile",
            "../NuGet.Config",
        ],
    );
    run(
        "build",
        "csharp",
        &[
            "build",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-restore",
            "-m:1",
        ],
    );
    run(
        "pack",
        "csharp",
        &[
            "pack",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-build",
            "-o",
            "../feed",
        ],
    );
    let package = root.join("feed/acme.oauth-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.oauth-sdk", "net8.0"),
    )
    .unwrap();
    fs::write(root.join("consumer/Program.cs"), DRIVER).unwrap();
    run(
        "consumer-restore",
        "consumer",
        &["restore", "--configfile", "../NuGet.Config"],
    );
    run(
        "consumer-build",
        "consumer",
        &["build", "-c", "Release", "--no-restore", "-m:1"],
    );
    run(
        "consumer-run",
        "consumer",
        &["run", "-c", "Release", "--no-build"],
    );
    let assets: Value =
        serde_json::from_slice(&fs::read(root.join("consumer/obj/project.assets.json")).unwrap())
            .unwrap();
    assert_eq!(
        assets["libraries"]["acme.oauth-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}

fn project(config: &str, framework: &str) -> String {
    format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{framework}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"{config}\" Version=\"[0.1.0]\" /></ItemGroup></Project>\n"
    )
}
