//! M5 OAuth runtime emission for the Kotlin backend: generation-time emission
//! shape, the no-policy byte-identity control, and native compilation plus a
//! scripted-transport behavioral probe of the emitted package when a
//! JDK/Maven toolchain is available. Static runtime files are never modified;
//! the surface lives in the emitted `OAuth.kt` plus one conditional internal
//! client accessor.

#![cfg(feature = "kotlin-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthMode, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// One client-credentials scheme (with an authorization-code flow and a
/// declared refresh URL) and one device-authorization scheme, each used by an
/// operation.
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

const ENTRY: &str = "https://source.oauth.test/openapi.json";

fn contract_with_document(document: Value, entry: &str) -> Arc<Contract> {
    let entry = Uri::parse(entry).unwrap();
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

fn generate_document(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract_with_document(document, ENTRY);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::KotlinHttp,
            package_name: "test.suspect:oauth-kotlin".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    generate_document(oauth_document(), options)
}

fn oauth_config() -> OAuthDefaults {
    OAuthDefaults {
        schemes: std::collections::BTreeMap::from([
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
                    ..OAuthSchemeConfig::default()
                },
            ),
        ]),
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
                mode: OAuthMode::Off,
                ..oauth_config()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

fn file<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn configured_emission_adds_only_oauth_kt_and_the_client_accessor() {
    let mut configured = generate(&oauth_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    let oauth_path = "kotlin/src/main/kotlin/test/suspect/oauth_kotlin/OAuth.kt";
    assert!(
        !control.iter().any(|file| file.path == oauth_path),
        "no-policy output must not carry the OAuth lifecycle"
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if file.path.ends_with("oauth_kotlin/Client.kt") {
            assert_ne!(
                emitted.content, file.content,
                "the OAuth client gains the lifecycle accessor"
            );
        } else {
            assert_eq!(
                emitted.content, file.content,
                "unrelated file {} changed",
                file.path
            );
        }
    }
    let client = file(&configured, "oauth_kotlin/Client.kt");
    assert!(
        client.contains("internal val oauth: OAuthSessions by lazy { OAuthSessions(transport) }"),
        "Client.kt carries the internal lifecycle accessor:\n{client}"
    );
    let oauth = file(&configured, "oauth_kotlin/OAuth.kt");
    for expected in [
        // The compiled descriptors are constants; credential values never are.
        "internal val schemes: Map<String, OAuthSchemeDescriptor> = mapOf(",
        "\"serviceOAuth\" to OAuthSchemeDescriptor(",
        "\"https://auth.oauth.test/token\"",
        "\"https://auth.oauth.test/token-refresh\"",
        "\"https://auth.oauth.test/device\"",
        "\"SUSPECT_OAUTH_CLIENT_ID\"",
        "\"https://auth.oauth.test/revoke\"",
        "\"https://auth.oauth.test/introspect\"",
        // Instance-owned store, typed error, single-flight acquisition.
        "public interface TokenStore",
        "public class MemoryTokenStore : TokenStore",
        "synchronized(this)",
        "public class AuthException(",
        "private val gateMutex = Mutex()",
        "gate.withLock {",
        "A caller that waited on the gate re-checks the store before",
        // Skew-aware cache reads the compiled skew.
        "public fun expired(skewSeconds: Int, nowMillis: Long = System.currentTimeMillis())",
        // Explicit refresh with rotated-refresh adoption.
        "public suspend fun refreshToken(",
        // PKCE S256 with SecureRandom + MessageDigest.
        "public fun beginAuthorization(",
        "public suspend fun completeAuthorization(",
        "MessageDigest.getInstance(\"SHA-256\")",
        "SecureRandom().nextBytes(raw)",
        "\"code_challenge_method\" to \"S256\"",
        "MessageDigest.isEqual(",
        // Device polling with injectable wait.
        "public suspend fun beginDeviceAuthorization(",
        "public suspend fun pollDeviceToken(",
        "wait: suspend (Long) -> Unit = { delay(it) }",
        "\"authorization_pending\"",
        "\"slow_down\"",
        // Conditional revocation/introspection.
        "public suspend fun revokeToken(",
        "public suspend fun introspectToken(",
        // Redaction: token values never enter summaries or messages.
        "override fun toString(): String = \"TokenSet(hasRefresh=${refreshToken != null}, scope=$scope)\"",
        "never contain token or",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.kt is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The client extensions delegate to the internal accessor.
    for expected in [
        "public suspend fun Client.clientCredentialsToken(",
        "public suspend fun Client.refreshToken(",
        "public suspend fun Client.beginDeviceAuthorization(",
        "public suspend fun Client.pollDeviceToken(",
        "public suspend fun Client.revokeToken(",
        "public suspend fun Client.introspectToken(",
        "public fun Client.beginAuthorization(",
        "public suspend fun Client.completeAuthorization(",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.kt lacks extension {expected}"
        );
    }
    // Secrets never enter emitted bytes.
    assert!(!oauth.contains("client-secret-value"));
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn policy_without_usable_schemes_or_off_mode_is_byte_identical() {
    let mut disabled = generate(&off_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut disabled);
    sorted(&mut control);
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
    // A document without any OAuth security scheme emits nothing even under a
    // configured policy (whose scheme entries would bind nothing and are
    // refused by the shared planner).
    let plain = json!({
        "openapi": "3.1.0",
        "info": {"title": "Plain", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets",
            "responses": {"200": {"description": "Ok"}}}}}
    });
    let empty = GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults::default(),
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    };
    let mut configured = generate_document(plain.clone(), &empty);
    let mut control = generate_document(plain, &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    assert_eq!(configured.len(), control.len());
    for (configured, control) in configured.iter().zip(control.iter()) {
        assert_eq!(configured.path, control.path);
        assert_eq!(configured.content, control.content);
    }
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn plan_carries_the_compiled_selection_only_when_configured() {
    let contract = contract_with_document(oauth_document(), ENTRY);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = suspect_codegen::kotlin_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::kotlin_sdk::SdkConfig {
            group_id: "test.suspect".into(),
            artifact_id: "oauth-kotlin".into(),
            version: "0.1.0".into(),
            package_name: "test.suspect.oauth_kotlin".into(),
            sdk_defaults: Some(SdkDefaults {
                oauth: oauth_config(),
                ..SdkDefaults::v1()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = configured.oauth().expect("configured policy is carried");
    assert_eq!(oauth.schemes.len(), 2);
    assert_eq!(oauth.schemes[0].name, "deviceOAuth");
    assert_eq!(oauth.schemes[1].name, "serviceOAuth");
    let service = oauth
        .schemes
        .iter()
        .find(|scheme| scheme.name == "serviceOAuth")
        .expect("service scheme");
    let service_flow = service
        .flows
        .iter()
        .find(|flow| {
            flow.kind == suspect_codegen::http_protocol::OAuthFlowDescriptorKind::ClientCredentials
        })
        .expect("client-credentials flow");
    assert_eq!(
        service_flow.token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    assert_eq!(
        service_flow.refresh_url.as_deref(),
        Some("https://auth.oauth.test/token-refresh")
    );
    assert_eq!(
        service_flow.client_auth,
        suspect_codegen::http_protocol::OAuthClientAuth::ClientSecretBasic
    );
    assert_eq!(service.refresh_skew_seconds, 30);
    assert_eq!(
        service.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(
        service.introspection_endpoint.as_deref(),
        Some("https://auth.oauth.test/introspect")
    );
    let control =
        suspect_codegen::kotlin_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
    assert!(control.oauth().is_none());
}

const PROBE: &str = r#"// Scripted-transport probe asserting the generated OAuth lifecycle.
package test.suspect.oauth_kotlin

import java.nio.charset.StandardCharsets
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking

private var failures = 0

private fun expect(condition: Boolean, message: String) {
    if (!condition) {
        System.err.println("failed: $message")
        failures++
    }
}

private class Step(val urlContains: String, val status: Int, val body: String)

private class ScriptedTransport(private val steps: List<Step>) : Transport {
    private val requests = mutableListOf<HttpRequest>()
    override suspend fun execute(request: HttpRequest): HttpResponse {
        val index = synchronized(requests) {
            requests.add(request)
            requests.size - 1
        }
        val step = steps.getOrNull(index) ?: throw TransportException(
            null,
            IllegalStateException("unexpected request $index to ${request.url}"),
        )
        if (step.urlContains.isNotEmpty() && !request.url.toString().contains(step.urlContains)) {
            throw TransportException(
                null,
                IllegalStateException(
                    "request $index hit ${request.url}, expected it to contain ${step.urlContains}",
                ),
            )
        }
        return HttpResponse(
            step.status,
            mapOf("Content-Type" to listOf("application/json")),
            step.body.toByteArray(StandardCharsets.UTF_8),
        )
    }
    fun urls(): List<String> = synchronized(requests) { requests.map { it.url.toString() } }
    fun bodies(): List<String> = synchronized(requests) {
        requests.map { it.body?.toString(StandardCharsets.UTF_8) ?: "" }
    }
    fun headers(): List<Map<String, String>> = synchronized(requests) { requests.map { it.headers } }
}

private const val TOKEN = """{"access_token":"at-1","token_type":"Bearer","expires_in":3600}"""
private const val TOKEN_ROTATED = """{"access_token":"at-2","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-2"}"""
private const val TOKEN_NO_REFRESH = """{"access_token":"at-2","token_type":"bearer","expires_in":3600}"""

// Acquire once, then a cache hit: exactly one token request, with the
// compiled client_secret_basic authentication per RFC 6749 2.3.1.
private fun acquireThenCacheHit() = runBlocking {
    val transport = ScriptedTransport(listOf(Step("https://auth.oauth.test/token", 200, TOKEN)))
    val sessions = OAuthSessions(transport)
    val first = sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1")
    expect(first.accessToken == "at-1", "first acquisition decoded the access token")
    expect(first.tokenType == "Bearer", "token type decoded")
    expect(!first.expired(30), "fresh set is not expired")
    val second = sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1")
    expect(second.accessToken == "at-1", "cache hit returned the stored set")
    expect(transport.urls().size == 1, "cache hit issued no second request: ${transport.urls().size}")
    expect(
        transport.headers()[0]["Authorization"] == "Basic aWQtMTpzZWNyZXQtMQ==",
        "confidential client used RFC 6749 2.3.1 Basic credentials",
    )
    expect(
        transport.bodies()[0].contains("grant_type=client_credentials"),
        "client-credentials grant on the wire",
    )
    expect(
        !transport.bodies()[0].contains("id-1"),
        "the client id never joined the form for a confidential client",
    )
    Unit
}

// Concurrent callers share one acquisition: exactly one token request.
private fun singleFlight() = runBlocking {
    val transport = ScriptedTransport(listOf(Step("https://auth.oauth.test/token", 200, TOKEN)))
    val sessions = OAuthSessions(transport)
    val tokens = (1..4).map {
        async { sessions.clientCredentialsToken("serviceOAuth", "concurrent", "secret") }
    }.awaitAll()
    expect(tokens.all { it.accessToken == "at-1" }, "every caller received the set")
    expect(
        transport.urls().size == 1,
        "single-flight issued exactly one request: ${transport.urls().size}",
    )
}

// Distinct client identities never share a store partition.
private fun partitionedStore() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step("https://auth.oauth.test/token", 200, TOKEN),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    sessions.clientCredentialsToken("serviceOAuth", "client-one", "secret")
    sessions.clientCredentialsToken("serviceOAuth", "client-two", "secret")
    expect(
        transport.urls().size == 2,
        "distinct client identities acquired separately: ${transport.urls().size}",
    )
}

// Explicit refresh over the declared refresh URL: a rotated refresh token is
// adopted, an absent one retains the previous.
private fun refreshRotation() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step("https://auth.oauth.test/token-refresh", 200, TOKEN_ROTATED),
        Step("https://auth.oauth.test/token-refresh", 200, TOKEN_NO_REFRESH),
    ))
    val sessions = OAuthSessions(transport)
    val set = TokenSet(accessToken = "at-1", refreshToken = "rt-1")
    val rotated = sessions.refreshToken("serviceOAuth", set, "id-1", "secret-1")
    expect(rotated.accessToken == "at-2", "rotated access token decoded")
    expect(rotated.refreshToken == "rt-2", "a rotated refresh token is adopted")
    expect(
        transport.bodies()[0].contains("grant_type=refresh_token") &&
            transport.bodies()[0].contains("refresh_token=rt-1"),
        "refresh grant with the previous token on the wire",
    )
    expect(
        transport.urls()[0].contains("token-refresh"),
        "the declared refresh endpoint was used, not the token endpoint",
    )
    val retained = sessions.refreshToken("serviceOAuth", rotated, "id-1", "secret-1")
    expect(
        retained.refreshToken == "rt-2",
        "an absent rotated refresh token retains the previous one",
    )
    expect(
        retained.tokenType == "Bearer",
        "a lowercase bearer token type normalizes to the conventional Bearer",
    )
}

// RFC 8628 polling with an injected no-op waiter: pending waits, slow_down
// extends the interval, and the granted set replaces the stored entry.
private fun devicePolling() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(
            "https://auth.oauth.test/device",
            200,
            """{"device_code":"dc-1","user_code":"ABCD-EFGH","verification_uri":"https://auth.oauth.test/activate","expires_in":600,"interval":1}""",
        ),
        Step("https://auth.oauth.test/token", 400, """{"error":"authorization_pending"}"""),
        Step("https://auth.oauth.test/token", 400, """{"error":"slow_down"}"""),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    val begin = sessions.beginDeviceAuthorization("deviceOAuth", "device-client")
    expect(begin.userCode == "ABCD-EFGH", "user code decoded")
    expect(begin.verificationURI == "https://auth.oauth.test/activate", "verification URI decoded")
    expect(begin.intervalMillis == 1000L, "server interval honored")
    val waits = AtomicInteger(0)
    val token = sessions.pollDeviceToken(begin, "device-client", null, null) { _ -> waits.incrementAndGet() }
    expect(token.accessToken == "at-1", "device polling delivered the granted set")
    expect(waits.get() == 2, "each pending/slow_down answer waited once: ${waits.get()}")
    expect(
        transport.urls().size == 4,
        "device flow made four requests: ${transport.urls().size}",
    )
    expect(
        transport.bodies().any { it.contains("urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code") },
        "the device grant type travelled on the wire",
    )
}

// A declared revocation endpoint revokes and clears the store partition; the
// introspection endpoint answers with the typed report.
private fun revokeAndIntrospect() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step("https://auth.oauth.test/token", 200, TOKEN),
        Step("https://auth.oauth.test/revoke", 200, ""),
        Step("https://auth.oauth.test/token", 200, TOKEN),
        Step(
            "https://auth.oauth.test/introspect",
            200,
            """{"active":true,"scope":"read","client_id":"id-1","sub":"user-1","aud":["aud-1","aud-2"],"exp":1893456000}""",
        ),
    ))
    val sessions = OAuthSessions(transport)
    sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1")
    sessions.revokeToken("serviceOAuth", "at-1", null, "id-1", "secret-1")
    expect(transport.urls()[1].contains("/revoke"), "revocation endpoint used")
    expect(transport.bodies()[1].contains("token=at-1"), "token value posted")
    // The store partition was cleared, so the next call acquires again.
    sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1")
    expect(transport.urls().size == 3, "revocation cleared the cached entry")
    val report = sessions.introspectToken("serviceOAuth", "at-1", null, "id-1", "secret-1")
    expect(report.active, "introspection active claim decoded")
    expect(report.scope == "read", "introspection scope decoded")
    expect(report.subject == "user-1", "introspection subject decoded")
    expect(report.audience == listOf("aud-1", "aud-2"), "introspection audience decoded")
    expect(report.expiresAt == 1893456000L, "introspection epoch decoded")
}

// Authorization-code with PKCE S256: the rendered URL carries the challenge,
// the exchange sends the verifier, and the state check is enforced.
private fun authorizationCode() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    val transaction = sessions.beginAuthorization("serviceOAuth", "https://app.oauth.test/callback", listOf("read"), clientID = "id-1")
    expect(
        transaction.authorizationURL.startsWith("https://auth.oauth.test/authorize?"),
        "the compiled authorization endpoint was used",
    )
    for (expected in listOf(
        "response_type=code",
        "client_id=id-1",
        "redirect_uri=https%3A%2F%2Fapp.oauth.test%2Fcallback",
        "code_challenge_method=S256",
        "scope=read",
    )) {
        expect(transaction.authorizationURL.contains(expected), "authorization URL carries $expected")
    }
    // A state mismatch is refused before any network call.
    val mismatch = try {
        sessions.completeAuthorization(
            transaction,
            mapOf("state" to "other", "code" to "abc"),
            "id-1",
            "secret-1",
        )
        false
    } catch (error: AuthException) {
        error.kind == "state-mismatch"
    }
    expect(mismatch, "a state mismatch is a typed refusal")
    // The transaction was consumed by the first attempt.
    val consumed = try {
        sessions.completeAuthorization(
            transaction,
            mapOf("state" to transaction.state, "code" to "abc"),
            "id-1",
            "secret-1",
        )
        false
    } catch (error: AuthException) {
        error.kind == "transaction-used"
    }
    expect(consumed, "the transaction is single-use")
    val second = sessions.beginAuthorization("serviceOAuth", "https://app.oauth.test/callback", state = "fixed-state", clientID = "id-1")
    val token = sessions.completeAuthorization(
        second,
        mapOf("state" to "fixed-state", "code" to "abc"),
        "id-1",
        "secret-1",
    )
    expect(token.accessToken == "at-1", "the code exchange delivered the token set")
    expect(
        transport.bodies()[0].contains("grant_type=authorization_code") &&
            transport.bodies()[0].contains("code_verifier=") &&
            transport.bodies()[0].contains("redirect_uri=https%3A%2F%2Fapp.oauth.test%2Fcallback"),
        "the exchange carried the code, verifier and redirect URI",
    )
    // The granted set replaced the store partition.
    expect(
        sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1").accessToken == "at-1",
        "the stored set is reused without another request",
    )
    expect(transport.urls().size == 1, "the stored entry served the next call")
}

// Typed failures carry the server error code and status, never the token,
// the secret, or the server's error description.
private fun typedFailuresCarryNoSecrets() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step("https://auth.oauth.test/token", 400, """{"error":"invalid_client","error_description":"secret-1 was rejected"}"""),
    ))
    val sessions = OAuthSessions(transport)
    val error = try {
        sessions.clientCredentialsToken("serviceOAuth", "id-1", "secret-1")
        null
    } catch (thrown: AuthException) {
        thrown
    } ?: run { expect(false, "a rejected grant throws"); return@runBlocking }
    expect(error.kind == "server-rejected", "typed server rejection")
    expect(error.code == "invalid_client", "the server error code surfaced")
    expect(error.status == 400, "the endpoint status surfaced")
    val message = error.message ?: ""
    expect(message.contains("invalid_client"), "the message names the code")
    expect(!message.contains("secret-1"), "the message never carries the secret")
    expect(!message.contains("was rejected"), "the message never carries the server description")
    expect(
        try {
            sessions.clientCredentialsToken("unknown"); false
        } catch (thrown: AuthException) {
            thrown.kind == "unknown-scheme"
        },
        "unknown scheme is typed",
    )
    expect(
        try {
            sessions.refreshToken("serviceOAuth", TokenSet(accessToken = "at-1")); false
        } catch (thrown: AuthException) {
            thrown.kind == "no-refresh-token"
        },
        "a set without a refresh token is typed",
    )
}

fun main() {
    acquireThenCacheHit()
    singleFlight()
    partitionedStore()
    refreshRotation()
    devicePolling()
    revokeAndIntrospect()
    authorizationCode()
    typedFailuresCarryNoSecrets()
    if (failures > 0) {
        kotlin.system.exitProcess(1)
    }
}
"#;

/// The native compile and behavioral-probe gates for the emitted OAuth
/// lifecycle, when a JDK and Maven are available.
fn toolchain() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    Some((java_home()?, maven()?))
}

/// One OpenID Connect scheme (usable only through its discovery URL) plus one
/// plain client-credentials scheme with a configured revocation endpoint.
fn discovery_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"oidc": []}],
                "responses": page
            }},
            "/things": {"get": {
                "operationId": "listThings",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
            }}
        },
        "components": {"securitySchemes": {
            "oidc": {"type": "openIdConnect", "openIdConnectUrl": "https://auth.oauth.test/.well-known/openid-configuration"},
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn discovery_config() -> OAuthDefaults {
    OAuthDefaults {
        schemes: std::collections::BTreeMap::from([
            (
                "oidc".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_OIDC_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_OIDC_SECRET".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
            (
                "serviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
        ]),
        ..OAuthDefaults::default()
    }
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: discovery_config(),
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn discovery_emission_is_conditional_on_a_compiled_discovery_url() {
    let mut configured = generate_document(discovery_document(), &discovery_options());
    let mut control = generate_document(discovery_document(), &GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    // The discovery URL makes the OIDC scheme usable: the same file set gains
    // only the OAuth.kt lifecycle.
    assert!(
        !control.iter().any(|file| file.path.ends_with("OAuth.kt")),
        "no-policy output must not carry the OAuth lifecycle"
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the discovery-aware policy may add exactly one file"
    );
    let oauth = file(&configured, "oauth_kotlin/OAuth.kt");
    for expected in [
        // The frozen compiled discovery URL map and the discovery engine.
        "internal object DiscoveryDocuments {",
        "internal val documents: Map<String, String> = mapOf(",
        "\"oidc\" to \"https://auth.oauth.test/.well-known/openid-configuration\",",
        "RFC 8414 / OpenID Connect discovery",
        "private suspend fun discover(descriptor: OAuthSchemeDescriptor)",
        "private suspend fun fetchDiscovery(",
        "private fun discoveryDocument(",
        "private fun discoveryOrigin(url: String): String?",
        // The typed discovery failure and the issuer-origin rule.
        "\"discovery-failed\", descriptor.name",
        "equals the discovery URL's origin",
        "A failed fetch is never",
        "so the next call retries",
        // The per-instance single-flight gates.
        "private val discoveryMutex = Mutex()",
        "private suspend fun discoveryGate(",
        // The discovery-aware client-credentials member and the plain
        // compiled precedence for the flow-carrying scheme.
        "val endpoint = compiled.ifEmpty { discover(descriptor).tokenEndpoint ?: \"\" }",
        "\"serviceOAuth\" to OAuthSchemeDescriptor(",
        "\"https://auth.oauth.test/token\"",
        "\"https://auth.oauth.test/revoke\"",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.kt lacks:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain variant for a plan without any discovery URL emits no
    // discovery bytes at all: the discovery-aware sections are strictly
    // conditional.
    let plain_files = generate(&oauth_options());
    let plain = file(&plain_files, "oauth_kotlin/OAuth.kt");
    assert!(!plain.contains("DiscoveryDocuments"));
    assert!(!plain.contains("discovery-failed"));
    assert!(!plain.contains("discoveryMutex"));
    assert!(plain.contains("never performs discovery and never invents an endpoint"));
    // The discovery-aware variant names the resolution.
    assert!(oauth.contains("RFC 8414 /\n * OpenID Connect discovery"));
}

/// The discovery-aware emitted package compiles, and the discovery engine
/// behaves against a scripted transport: the discovered token endpoint is
/// used, cached per instance, single-flighted, the issuer rule is enforced,
/// and a failed fetch is retried on the next call.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn native_discovery_resolves_caches_single_flights_and_retries() {
    let Some((java_home, maven)) = toolchain() else {
        eprintln!("kotlin_oauth: no JDK 21 + Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate_document(discovery_document(), &discovery_options()),
        root.path(),
    )
    .unwrap();
    let probe = root
        .path()
        .join("kotlin/src/test/kotlin/test/suspect/oauth_kotlin");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(probe.join("DiscoveryProbe.kt"), DISCOVERY_PROBE).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the discovery probe\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&maven)
        .args([
            "-q",
            "-B",
            "org.codehaus.mojo:exec-maven-plugin:3.6.3:java",
            "-Dexec.mainClass=test.suspect.oauth_kotlin.DiscoveryProbeKt",
            "-Dexec.classpathScope=test",
        ])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "discovery probe failed\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    eprintln!("kotlin_oauth: discovery probe succeeded");
}

const DISCOVERY_PROBE: &str = r#"// Scripted-transport probe asserting the generated discovery engine.
package test.suspect.oauth_kotlin

import java.nio.charset.StandardCharsets
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking

private var failures = 0

private fun expect(condition: Boolean, message: String) {
    if (!condition) {
        System.err.println("failed: $message")
        failures++
    }
}

private class Step(val urlContains: String, val status: Int, val body: String)

private class ScriptedTransport(private val steps: List<Step>) : Transport {
    private val requests = mutableListOf<HttpRequest>()
    override suspend fun execute(request: HttpRequest): HttpResponse {
        val index = synchronized(requests) {
            requests.add(request)
            requests.size - 1
        }
        val step = steps.getOrNull(index) ?: throw TransportException(
            null,
            IllegalStateException("unexpected request $index to ${request.url}"),
        )
        if (!request.url.toString().contains(step.urlContains)) {
            throw TransportException(
                null,
                IllegalStateException(
                    "request $index hit ${request.url}, expected it to contain ${step.urlContains}",
                ),
            )
        }
        return HttpResponse(
            step.status,
            mapOf("Content-Type" to listOf("application/json")),
            step.body.toByteArray(StandardCharsets.UTF_8),
        )
    }
    fun urls(): List<String> = synchronized(requests) { requests.map { it.url.toString() } }
    fun bodies(): List<String> = synchronized(requests) {
        requests.map { it.body?.toString(StandardCharsets.UTF_8) ?: "" }
    }
    fun headers(): List<Map<String, String>> = synchronized(requests) { requests.map { it.headers } }
    fun methods(): List<String> = synchronized(requests) { requests.map { it.method } }
}

private const val DISCOVERY = """{"issuer":"https://auth.oauth.test","token_endpoint":"https://auth.oauth.test/token"}"""
private const val DISCOVERY_MISMATCH = """{"issuer":"https://evil.oauth.test","token_endpoint":"https://auth.oauth.test/token"}"""
private const val TOKEN = """{"access_token":"at-1","token_type":"Bearer","expires_in":3600}"""

// The discovery document supplies the token endpoint the compiled plan omits:
// one GET, then the token POST with the discovered endpoint and the compiled
// client-secret-basic authentication.
private fun discoveredEndpointUsed() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(".well-known/openid-configuration", 200, DISCOVERY),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    val token = sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
    expect(token.accessToken == "at-1", "the discovered endpoint delivered the token set")
    expect(transport.methods()[0] == "GET", "discovery fetches with GET: ${transport.methods()[0]}")
    expect(
        transport.urls()[0] == "https://auth.oauth.test/.well-known/openid-configuration",
        "the compiled discovery URL was fetched: ${transport.urls()[0]}",
    )
    expect(
        transport.headers()[0]["Accept"] == "application/json",
        "discovery fetches with accept: application/json",
    )
    expect(
        transport.urls()[1].startsWith("https://auth.oauth.test/token"),
        "the discovered token endpoint was used: ${transport.urls()[1]}",
    )
    expect(
        transport.headers()[1]["Authorization"] == "Basic aWQtMTpzZWNyZXQtMQ==",
        "the discovery-resolved endpoint used the compiled client-secret-basic policy",
    )
    expect(
        transport.bodies()[1].contains("grant_type=client_credentials"),
        "client-credentials grant on the wire",
    )
    Unit
}

// Successful documents are cached per instance: repeated calls re-read the
// cache, and a fresh instance fetches its own copy.
private fun discoveryCachedPerInstance() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(".well-known/openid-configuration", 200, DISCOVERY),
        Step("https://auth.oauth.test/token", 200, TOKEN),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
    // The token store serves the second call: no discovery fetch, no token POST.
    sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
    expect(transport.urls().size == 2, "the cached document served the second call: ${transport.urls().size}")
    Unit
}

// Concurrent callers share one discovery fetch and one acquisition.
private fun discoverySingleFlight() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(".well-known/openid-configuration", 200, DISCOVERY),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    val tokens = (1..4).map {
        async { sessions.clientCredentialsToken("oidc", "concurrent", "secret") }
    }.awaitAll()
    expect(tokens.all { it.accessToken == "at-1" }, "every caller received the set")
    expect(
        transport.urls().size == 2,
        "single-flight issued one discovery fetch and one token request: ${transport.urls().size}",
    )
}

// A mismatching issuer claim is the typed discovery failure.
private fun issuerMismatchIsTyped() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(".well-known/openid-configuration", 200, DISCOVERY_MISMATCH),
    ))
    val sessions = OAuthSessions(transport)
    val error = try {
        sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
        null
    } catch (thrown: AuthException) {
        thrown
    } ?: run { expect(false, "a mismatching issuer throws"); return@runBlocking }
    expect(error.kind == "discovery-failed", "typed discovery failure: ${error.kind}")
    expect(error.scheme == "oidc", "the failure carries the scheme")
    Unit
}

// A failed discovery fetch is never cached: the next call retries and
// succeeds. Non-2xx answers and transport failures are typed the same way and
// never carry response body text.
private fun failedFetchRetries() = runBlocking {
    val transport = ScriptedTransport(listOf(
        Step(".well-known/openid-configuration", 500, "server exploded"),
        Step(".well-known/openid-configuration", 200, DISCOVERY),
        Step("https://auth.oauth.test/token", 200, TOKEN),
    ))
    val sessions = OAuthSessions(transport)
    val error = try {
        sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
        null
    } catch (thrown: AuthException) {
        thrown
    } ?: run { expect(false, "a failed discovery fetch throws"); return@runBlocking }
    expect(error.kind == "discovery-failed", "typed discovery failure: ${error.kind}")
    expect(error.status == 500, "the endpoint status surfaced")
    expect(
        error.message?.contains("server exploded") != true,
        "the failure never carries response body text",
    )
    val token = sessions.clientCredentialsToken("oidc", "id-1", "secret-1")
    expect(token.accessToken == "at-1", "the retry delivered the token set")
    expect(
        transport.urls().size == 3,
        "the failed fetch was retried, not cached: ${transport.urls().size}",
    )
    Unit
}

fun main() {
    discoveredEndpointUsed()
    discoveryCachedPerInstance()
    discoverySingleFlight()
    issuerMismatchIsTyped()
    failedFetchRetries()
    if (failures > 0) {
        kotlin.system.exitProcess(1)
    }
}
"#;

fn java_home() -> Option<std::path::PathBuf> {
    if let Some(home) =
        std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME").or_else(|| std::env::var_os("JAVA_HOME"))
    {
        return Some(std::path::PathBuf::from(home));
    }
    // The mise-managed Temurin install used by this workspace.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/java/temurin-21");
    let executable = home.join("bin/java");
    Command::new(&executable)
        .arg("-version")
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(home))
}

fn maven() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_KOTLIN_MAVEN") {
        return Some(std::path::PathBuf::from(path));
    }
    if Command::new("mvn")
        .arg("--version")
        .output()
        .is_ok_and(|probe| probe.status.success())
    {
        return Some(std::path::PathBuf::from("mvn"));
    }
    // mise-managed Maven installs carry a nested distribution directory.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/maven");
    let mut candidates = match std::fs::read_dir(&home) {
        Ok(installs) => installs
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| {
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .filter_map(|distribution| distribution.ok())
                    .map(|distribution| distribution.path().join("bin/mvn"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    candidates.sort();
    candidates.into_iter().find(|path| path.is_file())
}

/// Native compile of the emitted package, when a JDK and Maven are available.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn emitted_package_compiles_with_a_jdk_toolchain() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_oauth: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_oauth: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&oauth_options()), root.path()).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("kotlin_oauth: maven test-compile succeeded");
}

/// The emitted package compiles and the generated lifecycle behaves against a
/// scripted transport, when a JDK and Maven are available.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn native_lifecycle_acquires_refreshes_polls_and_revokes() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_oauth: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_oauth: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&oauth_options()), root.path()).unwrap();
    let probe = root
        .path()
        .join("kotlin/src/test/kotlin/test/suspect/oauth_kotlin");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(probe.join("OAuthProbe.kt"), PROBE).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the probe\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&maven)
        .args([
            "-q",
            "-B",
            "org.codehaus.mojo:exec-maven-plugin:3.6.3:java",
            "-Dexec.mainClass=test.suspect.oauth_kotlin.OAuthProbeKt",
            "-Dexec.classpathScope=test",
        ])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "OAuth probe failed\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    eprintln!("kotlin_oauth: scripted-transport probe succeeded");
}

/// One JSON operation protected by a client-credentials scheme and one SSE
/// stream operation protected by a second scheme, exactly like the reference
/// replay fixtures: the stream-protected scheme's attaches are ineligible.
fn replay_document() -> Value {
    let page = json!({
        "200": {"description": "Ok", "content": {"application/json": {"schema": {
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

fn replay_config() -> OAuthDefaults {
    OAuthDefaults {
        schemes: std::collections::BTreeMap::from([
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
    }
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: replay_config(),
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

/// An authorization-code-only scheme: the control for the replay emission
/// gate, whose module must stay byte-identical to the pre-replay emission.
fn interactive_code_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"userOAuth": ["read"]}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets",
            "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}}}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    })
}

fn code_only_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: std::collections::BTreeMap::from([(
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
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protected operations of its scheme's operations.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate_document(replay_document(), &replay_options());
    let oauth = file(&configured, "oauth_kotlin/OAuth.kt");
    for expected in [
        // The compiled stream-protected operations: the streaming operation's
        // id compiles for its scheme, the JSON operation's does not.
        "internal val oauthNoReplayOperations: Map<String, Set<String>> = mapOf(",
        "\"feedOAuth\" to setOf(\"streamEvents\"),",
        // The replaying credential wrapper, its hook and its transport.
        "public class OAuthReplayCredentials(",
        "public val hook: CredentialProvider",
        "public fun asTransport(inner: Transport): Transport",
        "oauthReplayValue(token)",
        // The opt-in one-refresh-one-replay contract.
        "One coordinated refresh: a newer stored set wins over a stale",
        "concurrent 401s share one round",
        "a failed round fails",
        "round.completeExceptionally",
        // The coordinated refresh rides the wrapped session's own machinery.
        "internal fun replayStoreKey(",
        "storeKey(descriptor, resolveCredentials(descriptor, clientID, clientSecret).first)",
        "store.load(key)?.let { stored ->",
        "store.clear(key)",
        // The lifecycle guard and the stream protection.
        "never replayed. They carry no bearer token of this provider",
        "oauthNoReplayOperations[scheme]?.contains(context.operationId) != true",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.kt is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("listWidgets\" to setOf("));
    // A plan without any executable client-credentials flow compiles exactly
    // the pre-replay bytes.
    let code_only = generate_document(interactive_code_document(), &code_only_options());
    let plain_oauth = file(&code_only, "oauth_kotlin/OAuth.kt");
    for absent in [
        "OAuthReplayCredentials",
        "oauthNoReplayOperations",
        "oauthReplayValue",
        "replayStoreKey",
        "asTransport",
    ] {
        assert!(
            !plain_oauth.contains(absent),
            "the code-only emission gained {absent}"
        );
    }
    // The plain lifecycle keeps today's semantics; the wrapper's paragraph
    // joins only when the wrapper participates.
    assert!(plain_oauth.contains("public class OAuthSessions("));
}

const REPLAY_PROBE: &str = r#"// Scripted-transport probe asserting the replaying credential wrapper.
package test.suspect.oauth_kotlin

import java.net.URI
import java.nio.charset.StandardCharsets
import java.time.Duration
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking

private var failures = 0

private fun expect(condition: Boolean, message: String) {
    if (!condition) {
        System.err.println("failed: $message")
        failures++
    }
}

private const val TOKEN_URL = "https://auth.oauth.test/token"
private const val FEED_TOKEN_URL = "https://auth.oauth.test/feed-token"
private const val WIDGETS_URL = "https://api.oauth.test/v1/widgets"
private const val EVENTS_URL = "https://api.oauth.test/v1/events"

// Fake API and token endpoints. `armStale` makes the next issued token answer
// 401 on the API, so the driver counts exactly one refresh.
private class ReplayServer : Transport {
    private val lock = Any()
    private val urls = mutableListOf<String>()
    private val presented = mutableListOf<String>()
    private var tokens = 0
    private var feedTokens = 0
    private var staleNext = false
    private var always401 = false
    private var failFrom = 0
    private var token401 = false
    private var staleToken: String? = null

    override suspend fun execute(request: HttpRequest): HttpResponse {
        val authorization = request.headers.entries
            .firstOrNull { it.key.equals("Authorization", ignoreCase = true) }?.value ?: ""
        synchronized(lock) {
            urls.add(request.url.toString())
            presented.add(authorization)
            when (request.url.toString()) {
                TOKEN_URL -> {
                    tokens++
                    if (token401) return respond(401, """{"error":"stale"}""")
                    val token = "svc-$tokens"
                    if (staleNext) {
                        staleNext = false
                        staleToken = token
                    }
                    if (failFrom in 1..tokens) return respond(500, """{"error":"server_error"}""")
                    return respond(200, """{"access_token":"$token","token_type":"Bearer","expires_in":3600}""")
                }
                FEED_TOKEN_URL -> {
                    feedTokens++
                    return respond(200, """{"access_token":"feed-1","token_type":"Bearer","expires_in":3600}""")
                }
                WIDGETS_URL -> {
                    val stale = staleToken
                    if (always401 || (stale != null && presented.last() == "Bearer $stale")) {
                        return respond(401, """{"error":"stale"}""")
                    }
                    return respond(200, """{"ok":true}""")
                }
                EVENTS_URL -> return respond(401, """{"error":"stream-denied"}""")
                else -> return respond(404, "")
            }
        }
    }
    private fun respond(status: Int, body: String) = HttpResponse(
        status,
        mapOf("Content-Type" to listOf("application/json")),
        body.toByteArray(java.nio.charset.StandardCharsets.UTF_8),
    )
    fun armStale() = synchronized(lock) { staleNext = true }
    fun always401() = synchronized(lock) { always401 = true }
    fun failFrom(from: Int) = synchronized(lock) { failFrom = from }
    fun failTokenOnce() = synchronized(lock) { token401 = true }
    fun tokenCount() = synchronized(lock) { tokens }
    fun feedTokenCount() = synchronized(lock) { feedTokens }
    /** The presented Authorization values of the requests whose URL carries
     * the suffix, in request order. */
    fun presentedTo(suffix: String) = synchronized(lock) {
        urls.zip(presented).filter { it.first.contains(suffix) }.map { it.second }
    }
    fun hits(suffix: String) = synchronized(lock) { urls.filter { it.contains(suffix) } }
    fun staleValue() = synchronized(lock) { staleToken }
}

private fun widgetRequest(authorization: String) = HttpRequest(
    method = "GET",
    url = java.net.URI(WIDGETS_URL),
    headers = linkedMapOf("Authorization" to authorization),
    body = null,
    timeout = Duration.ofSeconds(30),
    maxResponseBytes = OAuthMaxResponseBytes,
)

private fun eventRequest(authorization: String) = HttpRequest(
    method = "GET",
    url = java.net.URI(EVENTS_URL),
    headers = linkedMapOf("Authorization" to authorization),
    body = null,
    timeout = Duration.ofSeconds(30),
    maxResponseBytes = OAuthMaxResponseBytes,
)

// (a) 401 then success: one refresh, one replay, 200 surfaced.
private fun staleThenReplay() = runBlocking {
    val server = ReplayServer()
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val client = Client(Credentials(serviceOAuth = replay.hook), replay.asTransport(server))
    server.armStale()
    val result = client.listWidgets()
    expect(result is ListWidgetsResult.Status200, "the replay surfaced the 200")
    expect(server.hits("/v1/widgets").size == 2, "one original plus one replay: ${server.hits("/v1/widgets").size}")
    expect(server.tokenCount() == 2, "one acquisition plus one refresh: ${server.tokenCount()}")
    val presented = server.presentedTo("/v1/widgets")
    expect(presented.size == 2, "both API requests recorded: ${presented.size}")
    expect(presented[0] == "Bearer svc-1", "the original carried the stale token: ${presented[0]}")
    expect(presented[1] == "Bearer svc-2", "the replay carried the fresh token: ${presented[1]}")
    Unit
}

// (b) the second 401 surfaces; exactly one refresh, one replay, no loops.
private fun second401Surfaces() = runBlocking {
    val server = ReplayServer()
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val client = Client(Credentials(serviceOAuth = replay.hook), replay.asTransport(server))
    server.always401()
    var surfaced = false
    try {
        client.listWidgets()
    } catch (error: SdkException) {
        surfaced = error.response?.status == 401
    }
    expect(surfaced, "the second 401 surfaced as the declared error")
    expect(server.hits("/v1/widgets").size == 2, "one replay, no loops: ${server.hits("/v1/widgets").size}")
    expect(server.tokenCount() == 2, "exactly one refresh: ${server.tokenCount()}")
    Unit
}

// (c) two concurrent 401s across two coroutines: ONE refresh, two replays.
private fun concurrent401sShareOneRefresh() = runBlocking {
    val server = ReplayServer()
    server.armStale()
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val wrapped = replay.asTransport(server)
    val context = CredentialContext(
        operationId = "listWidgets",
        scheme = "serviceOAuth",
        scopes = listOf("read"),
        roles = emptyList(),
        source = SourceLocation("https://source.oauth.test/openapi.json", "/components/securitySchemes/serviceOAuth"),
        metadata = Json.parse("{}".toByteArray()) as JsonObject,
    )
    val authorization = replay.hook.authorization(context)
    expect(authorization == "Bearer " + server.staleValue(), "the attach served the stale token: $authorization")
    val responses = (1..2).map {
        async { replay.asTransport(server).execute(widgetRequest(authorization)) }
    }.awaitAll()
    expect(responses.all { it.status == 200 }, "every replay surfaced the 200: ${responses.map { it.status }}")
    expect(
        server.hits("/v1/widgets").size == 4,
        "two originals plus two replays: ${server.hits("/v1/widgets").size}",
    )
    expect(
        server.presentedTo("/v1/widgets").count { it != "Bearer svc-1" } == 2,
        "both replays carried the fresh token",
    )
    expect(server.tokenCount() == 2, "one shared refresh: ${server.tokenCount()}")
    Unit
}

// (d) a stream-protected attach is ineligible: the typed 401 surfaces with no
// replay and no refresh.
private fun streamingOperationNeverReplays() = runBlocking {
    val server = ReplayServer()
    val replay = OAuthReplayCredentials("feedOAuth", server, clientID = "fid", clientSecret = "fsecret")
    val context = CredentialContext(
        operationId = "streamEvents",
        scheme = "feedOAuth",
        scopes = listOf("read"),
        roles = emptyList(),
        source = SourceLocation("https://source.oauth.test/openapi.json", "/components/securitySchemes/feedOAuth"),
        metadata = Json.parse("{}".toByteArray()) as JsonObject,
    )
    val authorization = replay.hook.authorization(context)
    expect(authorization == "Bearer feed-1", "the stream attach served the token: $authorization")
    val response = replay.asTransport(server).execute(eventRequest(authorization))
    expect(response.status == 401, "the stream 401 surfaced: ${response.status}")
    expect(server.hits("/v1/events").size == 1, "no replay for the streaming operation")
    expect(server.feedTokenCount() == 1, "no refresh for the streaming operation")
    Unit
}

// (e) the plain provider surfaces the 401 without any refresh or replay.
private fun plainProviderNeverReplays() = runBlocking {
    val server = ReplayServer()
    server.armStale()
    val sessions = OAuthSessions(server)
    val token = sessions.clientCredentialsToken("serviceOAuth", "cid", "csecret")
    expect(token.accessToken == server.staleValue(), "the plain acquisition served the stale token")
    val plain = CredentialProvider { context -> oauthReplayValue(sessions.clientCredentialsToken(context.scheme, "cid", "csecret")) }
    val client = Client(Credentials(serviceOAuth = plain), server)
    var surfaced = false
    try {
        client.listWidgets()
    } catch (error: SdkException) {
        surfaced = error.response?.status == 401
    }
    expect(surfaced, "the plain lifecycle surfaced the 401")
    expect(server.hits("/v1/widgets").size == 1, "no replay: ${server.hits("/v1/widgets").size}")
    expect(server.tokenCount() == 1, "no refresh: ${server.tokenCount()}")
    Unit
}

// (f) a failed refresh surfaces the typed auth failure, with no replay.
private fun refreshFailureIsTypedAndNeverReplays() = runBlocking {
    val server = ReplayServer()
    server.armStale()
    server.failFrom(2)
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val client = Client(Credentials(serviceOAuth = replay.hook), replay.asTransport(server))
    var surfaced = false
    try {
        client.listWidgets()
    } catch (error: SdkException) {
        val cause = error.cause
        surfaced = cause is AuthException && cause.kind == "server-rejected" &&
            !error.message.orEmpty().contains("csecret")
    }
    expect(surfaced, "the typed auth failure surfaced instead of a replay")
    expect(server.hits("/v1/widgets").size == 1, "no replay after a failed refresh")
    expect(server.tokenCount() == 2, "the refresh was attempted exactly once: ${server.tokenCount()}")
    Unit
}

// A token this provider never served never triggers a refresh or a replay,
// and lifecycle endpoint requests never do either.
private fun foreignAndLifecycleRequestsNeverReplay() = runBlocking {
    val server = ReplayServer()
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val context = CredentialContext(
        operationId = "listWidgets",
        scheme = "serviceOAuth",
        scopes = listOf("read"),
        roles = emptyList(),
        source = SourceLocation("https://source.oauth.test/openapi.json", "/components/securitySchemes/serviceOAuth"),
        metadata = Json.parse("{}".toByteArray()) as JsonObject,
    )
    val authorization = replay.hook.authorization(context)
    // A foreign token: the 401 surfaces untouched, with no refresh.
    server.always401()
    val foreign = replay.asTransport(server).execute(widgetRequest("Bearer someone-elses-token"))
    expect(foreign.status == 401, "the foreign 401 surfaced untouched: ${foreign.status}")
    expect(server.tokenCount() == 1, "no refresh for a foreign token: ${server.tokenCount()}")
    expect(server.hits("/v1/widgets").size == 1, "no replay for a foreign token")
    // A lifecycle endpoint: the exact-target guard keeps it untouched.
    server.failTokenOnce()
    val lifecycle = replay.asTransport(server).execute(
        HttpRequest(
            method = "POST",
            url = java.net.URI(TOKEN_URL),
            headers = linkedMapOf("Authorization" to authorization),
            body = null,
            timeout = Duration.ofSeconds(30),
            maxResponseBytes = OAuthMaxResponseBytes,
        ),
    )
    expect(lifecycle.status == 401, "the lifecycle 401 surfaced untouched")
    expect(server.tokenCount() == 2, "no refresh for the lifecycle endpoint: ${server.tokenCount()}")
    expect(server.hits("/token").size == 2, "no replay for the lifecycle endpoint: ${server.hits("/token").size}")
    Unit
}

// The coordinated refresh is single-flighted per scheme and a newer stored
// set wins: after the concurrent replay populated the store, a stale 401
// reuses the stored fresh set with no second token request.
private fun newerStoredSetWins() = runBlocking {
    val server = ReplayServer()
    server.armStale()
    val replay = OAuthReplayCredentials("serviceOAuth", server, clientID = "cid", clientSecret = "csecret")
    val context = CredentialContext(
        operationId = "listWidgets",
        scheme = "serviceOAuth",
        scopes = listOf("read"),
        roles = emptyList(),
        source = SourceLocation("https://source.oauth.test/openapi.json", "/components/securitySchemes/serviceOAuth"),
        metadata = Json.parse("{}".toByteArray()) as JsonObject,
    )
    val stale = replay.hook.authorization(context)
    val wrapped = replay.asTransport(server)
    val first = wrapped.execute(widgetRequest(stale))
    expect(first.status == 200, "the replay surfaced the 200")
    expect(server.tokenCount() == 2, "one acquisition plus one refresh: ${server.tokenCount()}")
    // A second 401 presenting the same stale token reuses the stored fresh set.
    val second = wrapped.execute(widgetRequest(stale))
    expect(second.status == 200, "the newer stored set served the replay: ${second.status}")
    expect(server.tokenCount() == 2, "no second refresh: ${server.tokenCount()}")
    Unit
}

fun main() {
    staleThenReplay()
    second401Surfaces()
    concurrent401sShareOneRefresh()
    streamingOperationNeverReplays()
    plainProviderNeverReplays()
    refreshFailureIsTypedAndNeverReplays()
    foreignAndLifecycleRequestsNeverReplay()
    newerStoredSetWins()
    if (failures > 0) {
        kotlin.system.exitProcess(1)
    }
}
"#;

/// The native compile and behavioral-probe gates for the replaying credential
/// wrapper, when a JDK and Maven are available.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn native_replay_lifecycle_over_a_stubbed_transport() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_oauth: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_oauth: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate_document(replay_document(), &replay_options()),
        root.path(),
    )
    .unwrap();
    let probe = root
        .path()
        .join("kotlin/src/test/kotlin/test/suspect/oauth_kotlin");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(probe.join("ReplayProbe.kt"), REPLAY_PROBE).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the replay probe\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&maven)
        .args([
            "-q",
            "-B",
            "org.codehaus.mojo:exec-maven-plugin:3.6.3:java",
            "-Dexec.mainClass=test.suspect.oauth_kotlin.ReplayProbeKt",
            "-Dexec.classpathScope=test",
        ])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "replay probe failed\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    eprintln!("kotlin_oauth: replay probe succeeded");
}
