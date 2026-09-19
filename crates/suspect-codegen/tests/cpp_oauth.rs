//! M5 OAuth runtime emission for the C++ backend: generation-time emission
//! shape, the no-policy byte-identity control, and native behavioral
//! verification of the generated lifecycle against a scripted transport.
//! Static runtime files are never modified; the whole surface lives in the
//! emitted `include/<package>/oauth.hpp`.

#![cfg(feature = "cpp-sdk")]

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

const ENTRY: &str = "https://source.oauth.test/openapi.json";

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
            backend: Backend::CppHttp,
            package_name: "oauth_cpp".into(),
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

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn configured_emission_adds_only_the_oauth_header() {
    let mut configured = generate(&oauth_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    let oauth_path = "cpp/include/oauth_cpp/oauth.hpp";
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
        assert_eq!(
            emitted.content, file.content,
            "unrelated file {} changed",
            file.path
        );
    }
    let oauth = file(&configured, "include/oauth_cpp/oauth.hpp");
    for expected in [
        // The compiled descriptors are constants; credential values never are.
        "inline const OAuthSchemeDescriptor oauth_schemes[]",
        r#"std::string_view("serviceOAuth""#,
        r#"std::string_view("https://auth.oauth.test/token""#,
        r#"std::string_view("https://auth.oauth.test/token-refresh""#,
        r#"std::string_view("https://auth.oauth.test/device""#,
        r#"std::string_view("SUSPECT_OAUTH_CLIENT_ID""#,
        r#"std::string_view("https://auth.oauth.test/revoke""#,
        r#"std::string_view("https://auth.oauth.test/introspect""#,
        // Instance-owned store, typed error, single-flight acquisition.
        "class OAuthTokenStore",
        "class OAuthMemoryTokenStore final : public OAuthTokenStore",
        "std::lock_guard<std::mutex> guard(mutex_)",
        "class OAuthError",
        "client_credentials_token(const std::string& scheme",
        "std::condition_variable signal",
        "round->signal.wait(waiter",
        "re-check the store once more",
        // Explicit refresh with rotated-refresh adoption.
        "refresh_token(const std::string& scheme, const OAuthTokenSet& set",
        "Adopt a rotated refresh token; retain the previous one otherwise",
        // Skew-aware cache reads the compiled skew.
        "expired(std::chrono::seconds skew)",
        // Conditional sections: revocation/introspection/device all configured.
        "revoke_token(const std::string& scheme, const std::string& token",
        "introspect_token(const std::string& scheme",
        "struct OAuthDeviceGrant",
        "begin_device_authorization(const std::string& scheme",
        "poll_device_token(const OAuthDeviceGrant& grant",
        "slow_down",
        "authorization_pending",
        // Authorization-code with PKCE S256: entropy, hashing, single-use.
        "struct OAuthAuthorization",
        "begin_authorization(const std::string& scheme",
        "complete_authorization(const OAuthAuthorization& transaction",
        "code_challenge_method=S256",
        "code_verifier",
        "std::random_device device",
        "oauth_sha256_self_test",
        "0xba, 0x78, 0x16, 0xbf",
        "oauth_constant_time_equal",
        "Kind::TransactionUsed",
        "Kind::StateMismatch",
        "Kind::AuthorizationDenied",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.hpp is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // Secrets never enter emitted bytes.
    assert!(!oauth.contains("client-secret-value"));
}

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn policy_without_usable_schemes_or_off_mode_is_byte_identical() {
    for options in [&off_options()] {
        let mut disabled = generate(options);
        let mut control = generate(&GenerationOptions::default());
        sorted(&mut disabled);
        sorted(&mut control);
        assert_eq!(disabled.len(), control.len());
        for (disabled, control) in disabled.iter().zip(control.iter()) {
            assert_eq!(disabled.path, control.path);
            assert_eq!(disabled.content, control.content);
        }
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

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn plan_carries_the_compiled_selection_only_when_configured() {
    let contract = contract_with_document(oauth_document(), ENTRY);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = suspect_codegen::cpp_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::cpp_sdk::SdkConfig {
            name: "oauth_cpp".into(),
            namespace: "oauth_cpp".into(),
            sdk_defaults: Some(SdkDefaults {
                oauth: oauth_config(),
                ..SdkDefaults::v1()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = configured.oauth().expect("configured policy is carried");
    assert!(oauth.emits());
    assert_eq!(oauth.schemes.len(), 2);
    let service = oauth
        .schemes
        .iter()
        .find(|scheme| scheme.name == "serviceOAuth")
        .expect("service scheme");
    assert!(service.client_secret_basic);
    assert_eq!(
        service.client_credentials_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    // The authorization-code flow compiles: the whole authorization URL is
    // carried for transaction rendering, plus its token endpoint.
    assert_eq!(
        service.authorization_url.as_deref(),
        Some("https://auth.oauth.test/authorize")
    );
    assert_eq!(
        service.code_token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    assert_eq!(
        service.refresh_url.as_deref(),
        Some("https://auth.oauth.test/token-refresh")
    );
    assert_eq!(service.issuer, "https://auth.oauth.test");
    assert_eq!(service.skew_seconds, 30);
    assert_eq!(
        service.revocation_url.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(
        service.introspection_url.as_deref(),
        Some("https://auth.oauth.test/introspect")
    );
    let device = oauth
        .schemes
        .iter()
        .find(|scheme| scheme.name == "deviceOAuth")
        .expect("device scheme");
    assert!(!device.client_secret_basic);
    assert!(device.client_credentials_url.is_none());
    assert!(device.authorization_url.is_none());
    assert!(device.code_token_url.is_none());
    assert_eq!(
        device.device_url.as_deref(),
        Some("https://auth.oauth.test/device")
    );
    assert_eq!(
        device.device_token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    assert_eq!(oauth.names.sessions, "OAuthSessions");
    let control =
        suspect_codegen::cpp_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
    assert!(control.oauth().is_none());
}

const CONSUMER: &str = r#"// Scripted-transport consumer asserting the generated OAuth lifecycle.
#include <oauth_cpp/sdk.hpp>
#include <oauth_cpp/oauth.hpp>

#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>
#include <thread>
#include <utility>
#include <vector>

using namespace oauth_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

template<class T>
void expect_ok(const Result<T, OAuthError>& result, const std::string& message) {
    if (!result.ok()) {
        std::cerr << "failed: " << message << ": " << result.error().message() << " status "
            << result.error().status << "\n";
        ++failures;
    }
}

struct Step {
    std::string url_contains;
    int status = 200;
    std::string body;
};

class ScriptedTransport final : public Transport {
public:
    explicit ScriptedTransport(std::vector<Step> steps) : steps_(std::move(steps)) {}
    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions&) const override {
        urls_.push_back(request.url);
        bodies_.push_back(request.body ? *request.body : std::string());
        headers_.push_back(request.headers);
        const auto index = urls_.size() - 1;
        if (index >= steps_.size()) {
            TransportError error;
            error.kind = TransportError::Kind::Protocol;
            error.message = "unexpected request " + std::to_string(index) + " to " + request.url;
            return Result<HttpResponse, TransportError>::failure(std::move(error));
        }
        const auto& step = steps_[index];
        if (!step.url_contains.empty() && request.url.find(step.url_contains) == std::string::npos) {
            TransportError error;
            error.kind = TransportError::Kind::Protocol;
            error.message = "request " + std::to_string(index) + " hit " + request.url
                + ", expected it to contain " + step.url_contains;
            return Result<HttpResponse, TransportError>::failure(std::move(error));
        }
        HttpResponse response;
        response.status = step.status;
        response.headers = Headers{{"Content-Type", "application/json"}};
        response.body = step.body;
        return Result<HttpResponse, TransportError>::success(std::move(response));
    }
    const std::vector<std::string>& urls() const { return urls_; }
    const std::vector<std::string>& bodies() const { return bodies_; }
    const std::vector<Headers>& headers() const { return headers_; }

private:
    std::vector<Step> steps_;
    mutable std::vector<std::string> urls_;
    mutable std::vector<std::string> bodies_;
    mutable std::vector<Headers> headers_;
};

const std::string kToken = R"({"access_token":"at-1","token_type":"Bearer","expires_in":3600})";
const std::string kTokenRotated = R"({"access_token":"at-2","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-2"})";
const std::string kTokenNoRefresh = R"({"access_token":"at-2","token_type":"bearer","expires_in":3600})";

// Acquire once, then a cache hit: exactly one token request, and the compiled
// client_secret_basic authentication applied per RFC 6749 2.3.1.
void acquire_then_cache_hit() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";
    auto first = sessions.client_credentials_token("serviceOAuth", options);
    expect_ok(first, "first acquisition succeeded");
    expect(first.value().access_token == "at-1", "access token decoded");
    expect(first.value().token_type == "Bearer", "token type decoded");
    expect(!first.value().expired(std::chrono::seconds{30}), "fresh set is not expired");
    auto second = sessions.client_credentials_token("serviceOAuth", options);
    expect_ok(second, "cache hit succeeded");
    expect(second.value().access_token == "at-1", "cache hit returned the stored set");
    expect(transport->urls().size() == 1, "cache hit issued no second request: " + std::to_string(transport->urls().size()));
    // client_secret_basic: form-encoded id and secret, base64-encoded.
    const auto& headers = transport->headers()[0];
    bool basic = false;
    for (const auto& [name, value] : headers) {
        if (name == "Authorization" && value == "Basic aWQtMTpzZWNyZXQtMQ==") basic = true;
    }
    expect(basic, "confidential client used RFC 6749 2.3.1 Basic credentials");
    expect(transport->bodies()[0].find("grant_type=client_credentials") != std::string::npos,
        "client-credentials grant on the wire");
    expect(transport->bodies()[0].find("id-1") == std::string::npos,
        "the client id never joined the form for a confidential client");
}

// Environment names are compiled; values are read at call time.
void environment_credentials() {
    setenv("SUSPECT_OAUTH_CLIENT_ID", "env-id", 1);
    setenv("SUSPECT_OAUTH_CLIENT_SECRET", "env-secret", 1);
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    auto token = sessions.client_credentials_token("serviceOAuth");
    expect_ok(token, "environment-backed acquisition succeeded");
    bool basic = false;
    for (const auto& [name, value] : transport->headers()[0]) {
        if (name == "Authorization" && value == "Basic ZW52LWlkOmVudi1zZWNyZXQ=") basic = true;
    }
    expect(basic, "environment values resolved at call time");
    unsetenv("SUSPECT_OAUTH_CLIENT_ID");
    unsetenv("SUSPECT_OAUTH_CLIENT_SECRET");
}

// Concurrent callers share one acquisition: exactly one token request.
void single_flight() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "concurrent";
    options.client_secret = "secret";
    std::vector<std::string> results(4);
    std::vector<std::jthread> workers;
    for (int worker = 0; worker < 4; ++worker) {
        workers.emplace_back([&, worker] {
            auto token = sessions.client_credentials_token("serviceOAuth", options);
            if (token.ok()) results[static_cast<std::size_t>(worker)] = token.value().access_token;
        });
    }
    workers.clear();
    for (const auto& result : results) expect(result == "at-1", "every caller received the set");
    expect(transport->urls().size() == 1, "single-flight issued exactly one request: "
        + std::to_string(transport->urls().size()));
}

// Distinct client identities never share a store partition.
void partitioned_store() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions one;
    one.client_id = "client-one";
    one.client_secret = "secret";
    OAuthTokenOptions two;
    two.client_id = "client-two";
    two.client_secret = "secret";
    expect(sessions.client_credentials_token("serviceOAuth", one).ok(), "first partition acquired");
    expect(sessions.client_credentials_token("serviceOAuth", two).ok(), "second partition acquired");
    expect(transport->urls().size() == 2, "distinct client identities acquired separately");
}

// Explicit refresh over the declared refresh URL: a rotated refresh token is
// adopted, an absent one retains the previous, and the store is untouched.
void refresh_rotation() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token-refresh", 200, kTokenRotated},
        {"https://auth.oauth.test/token-refresh", 200, kTokenNoRefresh},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";
    OAuthTokenSet set;
    set.access_token = "at-1";
    set.refresh_token = "rt-1";
    set.has_refresh = true;
    auto rotated = sessions.refresh_token("serviceOAuth", set, options);
    expect_ok(rotated, "refresh over the declared refresh URL succeeded");
    expect(rotated.value().access_token == "at-2", "rotated access token decoded");
    expect(rotated.value().has_refresh && rotated.value().refresh_token == "rt-2",
        "a rotated refresh token is adopted");
    expect(transport->bodies()[0].find("grant_type=refresh_token") != std::string::npos,
        "refresh grant on the wire");
    expect(transport->bodies()[0].find("refresh_token=rt-1") != std::string::npos,
        "the previous refresh token travelled");
    expect(transport->urls()[0].find("token-refresh") != std::string::npos,
        "the declared refresh endpoint was used, not the token endpoint");
    auto retained = sessions.refresh_token("serviceOAuth", rotated.value(), options);
    expect_ok(retained, "second refresh succeeded");
    expect(retained.value().has_refresh && retained.value().refresh_token == "rt-2",
        "an absent rotated refresh token retains the previous one");
    expect(retained.value().token_type == "Bearer",
        "a lowercase bearer token type normalizes to the conventional Bearer");
}

// RFC 8628 polling with an injected no-op waiter: pending waits, slow_down
// extends the interval, and the granted set replaces the stored entry.
void device_polling() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/device", 200,
            R"({"device_code":"dc-1","user_code":"ABCD-EFGH","verification_uri":"https://auth.oauth.test/activate","expires_in":600,"interval":1})"},
        {"https://auth.oauth.test/token", 400, R"({"error":"authorization_pending"})"},
        {"https://auth.oauth.test/token", 400, R"({"error":"slow_down"})"},
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions begin_options;
    begin_options.client_id = "device-client";
    auto begin = sessions.begin_device_authorization("deviceOAuth", begin_options);
    expect_ok(begin, "device grant began");
    expect(begin.value().user_code == "ABCD-EFGH", "user code decoded");
    expect(begin.value().verification_uri == "https://auth.oauth.test/activate",
        "verification URI decoded");
    expect(begin.value().interval == std::chrono::milliseconds{1000}, "server interval honored");
    int waits = 0;
    OAuthTokenOptions options;
    options.client_id = "device-client";
    options.wait = [&waits](std::chrono::milliseconds) { ++waits; };
    auto token = sessions.poll_device_token(begin.value(), options);
    expect_ok(token, "device polling delivered the granted set");
    expect(token.value().access_token == "at-1", "device grant token decoded");
    expect(waits == 2, "each pending/slow_down answer waited once: " + std::to_string(waits));
    expect(transport->urls().size() == 4, "device flow made four requests: "
        + std::to_string(transport->urls().size()));
    bool grant_type = false;
    for (const auto& body : transport->bodies()) {
        if (body.find("urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code") != std::string::npos) grant_type = true;
    }
    expect(grant_type, "the device grant type travelled on the wire");
}

// A declared revocation endpoint revokes and clears the store partition; the
// introspection endpoint answers with the typed report.
void revoke_and_introspect() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
        {"https://auth.oauth.test/revoke", 200, ""},
        {"https://auth.oauth.test/token", 200, kToken},
        {"https://auth.oauth.test/introspect", 200,
            R"({"active":true,"scope":"read","client_id":"id-1","sub":"user-1","aud":["aud-1","aud-2"],"exp":1893456000})"},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";
    expect_ok(sessions.client_credentials_token("serviceOAuth", options), "acquired");
    expect_ok(sessions.revoke_token("serviceOAuth", "at-1", options), "revocation succeeded");
    expect(transport->urls()[1].find("/revoke") != std::string::npos, "revocation endpoint used");
    expect(transport->bodies()[1].find("token=at-1") != std::string::npos, "token value posted");
    // The store partition was cleared, so the next call acquires again.
    expect_ok(sessions.client_credentials_token("serviceOAuth", options), "re-acquired");
    expect(transport->urls().size() == 3, "revocation cleared the cached entry");
    auto report = sessions.introspect_token("serviceOAuth", "at-1", options);
    expect_ok(report, "introspection succeeded");
    expect(report.value().active, "introspection active claim decoded");
    expect(report.value().scope == "read", "introspection scope decoded");
    expect(report.value().subject == "user-1", "introspection subject decoded");
    expect(report.value().audience.size() == 2, "introspection audience decoded");
    expect(report.value().expires_at == 1893456000, "introspection epoch decoded");
    // A token set value can be revoked through its raw access token only.
    expect(sessions.revoke_token("serviceOAuth", "").error().kind == OAuthError::Kind::InvalidToken,
        "an empty token value is a typed refusal");
}

// Typed failures carry the server error code and status, never the token,
// the secret, or the server's error description.
void typed_failures_carry_no_secrets() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 400,
            R"({"error":"invalid_client","error_description":"secret-1 was rejected"})"},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";
    auto token = sessions.client_credentials_token("serviceOAuth", options);
    expect(!token.ok(), "a rejected grant fails");
    const auto& error = token.error();
    expect(error.kind == OAuthError::Kind::ServerRejected, "typed server rejection");
    expect(error.code == "invalid_client", "the server error code surfaced");
    expect(error.status == 400, "the endpoint status surfaced");
    const auto message = error.message();
    expect(message.find("invalid_client") != std::string::npos, "the message names the code");
    expect(message.find("secret-1") == std::string::npos, "the message never carries the secret");
    expect(message.find("was rejected") == std::string::npos,
        "the message never carries the server description");
    // Unknown scheme and missing credentials are typed refusals too.
    expect(sessions.client_credentials_token("unknown").error().kind == OAuthError::Kind::UnknownScheme,
        "unknown scheme is typed");
    OAuthTokenOptions missing;
    missing.client_id = "only-id";
    expect(sessions.client_credentials_token("serviceOAuth", missing).error().kind
            == OAuthError::Kind::MissingCredentials,
        "missing secret for a confidential client is typed");
    OAuthTokenSet no_refresh;
    no_refresh.access_token = "at-1";
    expect(sessions.refresh_token("serviceOAuth", no_refresh).error().kind
            == OAuthError::Kind::MissingRefreshToken,
        "a set without a refresh token is typed");
}

// PKCE round trip: the authorization URL carries the S256 challenge derived
// from the retained verifier, the exchange sends the verifier at the compiled
// token endpoint, a state mismatch is refused in constant time, the first
// completion attempt consumes the transaction, and a server-declared error in
// the callback is a typed refusal.
void authorization_code_pkce() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<Step>{
        {"https://auth.oauth.test/token", 200, kToken},
    });
    OAuthSessions sessions(transport);
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";

    // The SHA-256 self-test vector passes (observable through the detail
    // helper), and the challenge derivation matches the standard vector.
    const auto abc = detail::oauth_sha256("abc");
    std::string abc_hex;
    static constexpr char hex[] = "0123456789abcdef";
    for (const auto byte : abc) {
        abc_hex.push_back(hex[byte >> 4]);
        abc_hex.push_back(hex[byte & 0x0F]);
    }
    expect(abc_hex
            == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "the local SHA-256 matches the FIPS 180-4 test vector");

    auto begin = sessions.begin_authorization("serviceOAuth",
        "https://app.oauth.test/callback", {"read", "write"}, options);
    expect_ok(begin, "the authorization transaction started");
    auto transaction = std::move(begin).value();
    expect(transaction.state.size() == 43, "the state is 43 base64url characters");
    expect(transaction.code_verifier.size() == 43,
        "the verifier is 43 characters, inside the RFC 7636 range");
    bool unreserved = true;
    for (const char character : transaction.code_verifier) {
        const bool ok = (character >= 'A' && character <= 'Z')
            || (character >= 'a' && character <= 'z')
            || (character >= '0' && character <= '9')
            || character == '-' || character == '.' || character == '_'
            || character == '~';
        unreserved = unreserved && ok;
    }
    expect(unreserved, "the verifier uses only RFC 7636 unreserved characters");
    const auto digest = detail::oauth_sha256(transaction.code_verifier);
    expect(transaction.code_challenge == detail::oauth_base64url(digest.data(), digest.size()),
        "the bound challenge is base64url(SHA-256(verifier))");
    const auto& url = transaction.authorization_url;
    expect(url.starts_with("https://auth.oauth.test/authorize?"), "the URL opens the declared authorize endpoint: " + url);
    expect(url.find("response_type=code") != std::string::npos, "response_type=code on the URL");
    expect(url.find("client_id=id-1") != std::string::npos, "the client id rides the redirect");
    expect(url.find("redirect_uri=https%3A%2F%2Fapp.oauth.test%2Fcallback") != std::string::npos,
        "the redirect URI is form-encoded on the URL");
    expect(url.find("state=" + transaction.state) != std::string::npos, "the one-time state rides the URL");
    expect(url.find("code_challenge=" + transaction.code_challenge) != std::string::npos,
        "the S256 challenge rides the URL");
    expect(url.find("code_challenge_method=S256") != std::string::npos, "S256 declared");
    expect(url.find("scope=read%20write") != std::string::npos, "the scope joined space-separated");
    expect(url.find(transaction.code_verifier) == std::string::npos,
        "the verifier never rides the authorization URL");

    // Successive transactions draw fresh entropy.
    auto second = sessions.begin_authorization("serviceOAuth", "", {}, options);
    expect_ok(second, "a second transaction started");
    expect(second.value().state != transaction.state, "states are independently random");
    expect(second.value().code_verifier != transaction.code_verifier,
        "verifiers are independently random");
    expect(second.value().authorization_url.find("redirect_uri=") == std::string::npos,
        "an empty redirect URI is not sent");
    expect(second.value().authorization_url.find("scope=") == std::string::npos,
        "an empty scope is not sent");

    // Complete: the code is exchanged with the verifier on the wire.
    std::map<std::string, std::string, std::less<>> callback{
        {"state", transaction.state}, {"code", "ac-1"}};
    auto done = sessions.complete_authorization(transaction, callback, options);
    expect_ok(done, "the transaction completed");
    expect(done.value().access_token == "at-1", "the exchange returned the token");
    expect(transport->urls()[0] == "https://auth.oauth.test/token", "the exchange hit the compiled token endpoint");
    expect(transport->bodies()[0].find("grant_type=authorization_code") != std::string::npos,
        "the authorization-code grant travelled");
    expect(transport->bodies()[0].find("code=ac-1") != std::string::npos, "the code travelled");
    expect(transport->bodies()[0].find("code_verifier=" + transaction.code_verifier) != std::string::npos,
        "the retained verifier travelled to the exchange");
    expect(transport->bodies()[0].find("redirect_uri=https%3A%2F%2Fapp.oauth.test%2Fcallback") != std::string::npos,
        "the bound redirect URI is repeated exactly");

    // Single use: the consumed gate refuses a replay with the same callback.
    auto replay = sessions.complete_authorization(transaction, callback, options);
    expect(!replay.ok(), "a replay is refused");
    expect(replay.error().kind == OAuthError::Kind::TransactionUsed,
        "the replay is a typed TransactionUsed refusal");
    expect(transaction.consumed(), "the transaction reports itself consumed");

    // A state mismatch is refused, and it too consumed the transaction.
    auto mismatch_txn = sessions.begin_authorization("serviceOAuth", "", {}, options);
    expect_ok(mismatch_txn, "the mismatch transaction started");
    const std::string forged(43, 'X');
    std::map<std::string, std::string, std::less<>> wrong{{"state", forged}, {"code", "ac-2"}};
    auto mismatch = sessions.complete_authorization(mismatch_txn.value(), wrong, options);
    expect(!mismatch.ok(), "a forged state is refused");
    expect(mismatch.error().kind == OAuthError::Kind::StateMismatch,
        "the mismatch is a typed StateMismatch refusal");
    std::map<std::string, std::string, std::less<>> right{
        {"state", mismatch_txn.value().state}, {"code", "ac-2"}};
    auto burned = sessions.complete_authorization(mismatch_txn.value(), right, options);
    expect(!burned.ok(), "the consumed transaction is burned");
    expect(burned.error().kind == OAuthError::Kind::TransactionUsed,
        "even a valid callback cannot reuse the transaction");

    // A server-declared error in the callback is a typed refusal.
    auto denied_txn = sessions.begin_authorization("serviceOAuth", "", {}, options);
    expect_ok(denied_txn, "the denial transaction started");
    std::map<std::string, std::string, std::less<>> denial{
        {"state", denied_txn.value().state}, {"error", "access_denied"}};
    auto denied = sessions.complete_authorization(denied_txn.value(), denial, options);
    expect(!denied.ok(), "a declared denial is refused");
    expect(denied.error().kind == OAuthError::Kind::AuthorizationDenied,
        "the denial is typed");
    expect(denied.error().code == "access_denied", "the declared error code surfaced");
    expect(transport->urls().size() == 1, "no exchange fired for refused callbacks");

    // A callback without a code is a typed invalid response.
    auto missing_txn = sessions.begin_authorization("serviceOAuth", "", {}, options);
    expect_ok(missing_txn, "the missing-code transaction started");
    std::map<std::string, std::string, std::less<>> missing{{"state", missing_txn.value().state}};
    auto missing_code = sessions.complete_authorization(missing_txn.value(), missing, options);
    expect(!missing_code.ok(), "a callback without a code is refused");
    expect(missing_code.error().kind == OAuthError::Kind::InvalidResponse,
        "the missing code is typed");
}

} // namespace

int main() {
    acquire_then_cache_hit();
    environment_credentials();
    single_flight();
    partitioned_store();
    refresh_rotation();
    device_polling();
    authorization_code_pkce();
    revoke_and_introspect();
    typed_failures_carry_no_secrets();
    return failures == 0 ? 0 : 1;
}
"#;

/// An OpenID Connect scheme (whose endpoints the discovery document defines
/// at runtime) plus one OAuth2 scheme with a compiled client-credentials flow
/// and configured supplemental endpoints.
fn discovery_document() -> Value {
    json!({
        "openapi": "3.1.0",
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
            "identity": {
                "type": "openIdConnect",
                "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"
            },
            "service": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: std::collections::BTreeMap::from([
                    (
                        "identity".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "service".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
                            revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                            introspection_endpoint: Some(
                                "https://auth.oauth.test/introspect".into(),
                            ),
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

fn generate_named(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    generate_document(document, options)
}

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn discovery_schemes_emit_the_discovery_engine_and_precedence() {
    let configured = generate_named(discovery_document(), &discovery_options());
    let oauth = file(&configured, "include/oauth_cpp/oauth.hpp");
    for expected in [
        // The discovery engine's detail half and its bounded ceiling.
        "struct OAuthDiscoveredEndpoints {",
        "inline constexpr std::size_t oauth_discovery_max_bytes = 1 << 20;",
        "inline bool oauth_discovery_unusable(std::string_view value)",
        "inline std::string oauth_url_origin(std::string_view url)",
        // The typed discovery failure and the session's per-instance cache.
        "DiscoveryFailed,",
        "case Kind::DiscoveryFailed: return \"discovery-failed\";",
        "std::map<std::string, detail::OAuthDiscoveredEndpoints, std::less<>> discovery_;",
        "Result<detail::OAuthDiscoveredEndpoints, OAuthError> discover(",
        "Result<detail::OAuthDiscoveredEndpoints, OAuthError> discovery_fetch(",
        "Result<std::string, OAuthError> resolve_endpoint(const detail::OAuthSchemeDescriptor& descriptor,",
        // The exact issuer rule and the single-flight fetch.
        "issuer rule",
        "share the discovery URL's origin",
        "round->signal.wait(waiter",
        // The compiled discovery URL and the discovery-defined scheme, plus
        // the documented compiled-wins precedence.
        "std::string_view(\"https://authority.oauth.test/.well-known/openid-configuration\"",
        "resolve at call time through RFC 8414 / OpenID Connect discovery",
        // The discovery-aware acquisition resolves the token endpoint.
        "descriptor->client_credentials_url.empty() && descriptor->discovery_url.empty()",
        "resolve_endpoint(*descriptor, EndpointKind::Token, options)",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.hpp is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // Control: the pre-discovery fixture's emission stays byte-identical and
    // carries no discovery engine at all.
    let plain = generate(&oauth_options());
    let plain_oauth = file(&plain, "include/oauth_cpp/oauth.hpp");
    for absent in [
        "OAuthDiscoveredEndpoints",
        "oauth_url_origin",
        "DiscoveryFailed",
        "discovery-failed",
        "resolve_endpoint",
        "discovery_url",
    ] {
        assert!(
            !plain_oauth.contains(absent),
            "the plain emission gained {absent}"
        );
    }
    assert!(
        plain_oauth.contains("never performs discovery and never invents an endpoint"),
        "the plain emission keeps its exact paragraph"
    );
}

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn native_discovery_lifecycle_resolves_caches_single_flights_and_retries() {
    let Some(cmake) = tool("SUSPECT_CPP_CMAKE", "cmake") else {
        eprintln!("cpp_oauth: cmake not available; degrading to static assertions");
        return;
    };
    let Some(cxx) = tool("SUSPECT_CPP_CXX", "clang++") else {
        eprintln!("cpp_oauth: no C++20 compiler; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate_named(discovery_document(), &discovery_options()),
        root.path(),
    )
    .unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(consumer.join("main.cpp"), DISCOVERY_CONSUMER).unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
    let ctest = cmake.with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.path().join("build"))
            .arg("--output-on-failure"),
        root.path(),
    );
}

const DISCOVERY_CONSUMER: &str = r#"// Scripted-transport consumer asserting the generated discovery lifecycle.
#include <oauth_cpp/sdk.hpp>
#include <oauth_cpp/oauth.hpp>

#include <iostream>
#include <memory>
#include <string>
#include <thread>
#include <utility>
#include <vector>

using namespace oauth_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

const std::string kDiscoveryUrl = "https://authority.oauth.test/.well-known/openid-configuration";
const std::string kDiscoveredToken = "https://authority.oauth.test/oauth/token";
const std::string kDiscoveredRevoke = "https://authority.oauth.test/oauth/revoke";
const std::string kDiscoveredIntrospect = "https://authority.oauth.test/oauth/introspect";
const std::string kCompiledToken = "https://auth.oauth.test/token";
const std::string kCompiledRevoke = "https://auth.oauth.test/revoke";
const std::string kCompiledIntrospect = "https://auth.oauth.test/introspect";

// Serves the discovery document, the discovered endpoints and the compiled
// scheme's endpoints, recording every request. The mutable knobs flip the
// issuer claim and the discovery failure between rounds.
class DiscoveryTransport final : public Transport {
public:
    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions&) const override {
        urls_.push_back(request.url);
        methods_.push_back(request.method);
        bodies_.push_back(request.body ? *request.body : std::string());
        if (request.method == "GET") {
            for (const auto& [name, value] : request.headers) {
                if (name == "Accept") accepts_.push_back(value);
            }
        }
        if (request.url == kDiscoveryUrl) {
            ++discovery_hits_;
            if (discovery_transport_failure_) {
                TransportError error;
                error.kind = TransportError::Kind::Network;
                error.message = "discovery unreachable";
                return Result<HttpResponse, TransportError>::failure(std::move(error));
            }
            if (discovery_fail_status_ != 0) {
                return success(discovery_fail_status_, R"({"error":"boom"})");
            }
            return success(200, document());
        }
        if (request.url == kDiscoveredToken) {
            ++token_hits_;
            return success(200,
                R"({"access_token":"discovered-1","token_type":"Bearer","expires_in":3600,"refresh_token":"rotated-1"})");
        }
        if (request.url == kDiscoveredRevoke) return success(200, "");
        if (request.url == kDiscoveredIntrospect) {
            return success(200, R"({"active":true,"scope":"read"})");
        }
        if (request.url == kCompiledToken) {
            ++token_hits_;
            return success(200, R"({"access_token":"compiled-1","token_type":"Bearer","expires_in":3600})");
        }
        if (request.url == kCompiledRevoke) return success(200, "");
        if (request.url == kCompiledIntrospect) {
            return success(200, R"({"active":true,"scope":"read"})");
        }
        TransportError error;
        error.kind = TransportError::Kind::Protocol;
        error.message = "unexpected request to " + request.url;
        return Result<HttpResponse, TransportError>::failure(std::move(error));
    }

    void fail_discovery(const int status) const { discovery_fail_status_ = status; }
    void fail_discovery_transport(const bool failure) const { discovery_transport_failure_ = failure; }
    void set_issuer(std::string issuer) const { issuer_ = std::move(issuer); }
    int discovery_hits() const { return discovery_hits_; }
    int token_hits() const { return token_hits_; }
    const std::vector<std::string>& urls() const { return urls_; }
    const std::vector<std::string>& methods() const { return methods_; }
    const std::vector<std::string>& accepts() const { return accepts_; }
    const std::vector<std::string>& bodies() const { return bodies_; }

private:
    Result<HttpResponse, TransportError> success(const int status, std::string body) const {
        HttpResponse response;
        response.status = status;
        response.headers = Headers{{"Content-Type", "application/json"}};
        response.body = std::move(body);
        return Result<HttpResponse, TransportError>::success(std::move(response));
    }
    std::string document() const {
        return R"({"issuer":")" + issuer_ + R"(",)"
            R"("token_endpoint":")" + kDiscoveredToken + R"(",)"
            R"("revocation_endpoint":")" + kDiscoveredRevoke + R"(",)"
            R"("introspection_endpoint":")" + kDiscoveredIntrospect + R"(",)"
            R"("unknown_member":{"nested":true}})";
    }

    mutable int discovery_hits_ = 0;
    mutable int token_hits_ = 0;
    mutable int discovery_fail_status_ = 0;
    mutable bool discovery_transport_failure_ = false;
    mutable std::string issuer_ = "https://authority.oauth.test";
    mutable std::vector<std::string> urls_;
    mutable std::vector<std::string> methods_;
    mutable std::vector<std::string> accepts_;
    mutable std::vector<std::string> bodies_;
};

template<class T>
void expect_ok(const Result<T, OAuthError>& result, const std::string& message) {
    if (!result.ok()) {
        std::cerr << "failed: " << message << ": " << result.error().message() << " status "
            << result.error().status << "\n";
        ++failures;
    }
}

OAuthTokenOptions identity_options() {
    OAuthTokenOptions options;
    options.client_id = "id-1";
    options.client_secret = "secret-1";
    return options;
}

// The discovery-defined scheme resolves the token endpoint through discovery,
// authenticates with the compiled basic profile, and caches both the document
// and the token set.
void discovered_token_endpoint_used_and_cached() {
    auto transport = std::make_shared<DiscoveryTransport>();
    OAuthSessions sessions(transport);
    const auto options = identity_options();
    auto first = sessions.client_credentials_token("identity", options);
    expect_ok(first, "the discovered acquisition succeeded");
    expect(first.value().access_token == "discovered-1", "the discovered token decoded");
    expect(transport->discovery_hits() == 1, "one discovery fetch");
    expect(transport->methods()[0] == "GET", "discovery is a GET");
    expect(transport->accepts()[0] == "application/json", "discovery sends accept: application/json");
    expect(transport->urls()[0] == kDiscoveryUrl, "the compiled discovery URL was fetched");
    expect(transport->urls()[1] == kDiscoveredToken, "the acquisition hit the discovered endpoint");
    for (const auto& url : transport->urls()) {
        expect(url != kCompiledToken, "the compiled fallback endpoint was never contacted");
    }
    // The discovery document and the token set are both cached.
    auto second = sessions.client_credentials_token("identity", options);
    expect_ok(second, "the cached acquisition succeeded");
    expect(second.value().access_token == "discovered-1", "the cache hit returned the set");
    expect(transport->discovery_hits() == 1 && transport->token_hits() == 1,
        "cache hits issued no further requests");
}

// Concurrent callers share one discovery fetch and one acquisition.
void single_flight() {
    auto transport = std::make_shared<DiscoveryTransport>();
    OAuthSessions sessions(transport);
    const auto options = identity_options();
    std::vector<std::string> results(4);
    std::vector<std::jthread> workers;
    for (int worker = 0; worker < 4; ++worker) {
        workers.emplace_back([&, worker] {
            auto token = sessions.client_credentials_token("identity", options);
            if (token.ok()) results[static_cast<std::size_t>(worker)] = token.value().access_token;
        });
    }
    workers.clear();
    for (const auto& result : results) expect(result == "discovered-1", "every caller received the set");
    expect(transport->discovery_hits() == 1, "single-flight issued one discovery fetch: "
        + std::to_string(transport->discovery_hits()));
    expect(transport->token_hits() == 1, "single-flight issued one acquisition: "
        + std::to_string(transport->token_hits()));
}

// A mismatching issuer claim is a typed discovery failure, and the failed
// fetch is not cached: the next call retries and succeeds.
void issuer_mismatch_is_typed_and_retried() {
    auto transport = std::make_shared<DiscoveryTransport>();
    OAuthSessions sessions(transport);
    transport->set_issuer("https://elsewhere.oauth.test");
    auto refused = sessions.client_credentials_token("identity", identity_options());
    expect(!refused.ok(), "the issuer mismatch failed");
    expect(refused.error().kind == OAuthError::Kind::DiscoveryFailed, "the mismatch is typed");
    expect(refused.error().message().find("elsewhere") == std::string::npos,
        "the failure never carries document text");
    expect(transport->discovery_hits() == 1, "one failed fetch");
    transport->set_issuer("https://authority.oauth.test");
    auto healed = sessions.client_credentials_token("identity", identity_options());
    expect_ok(healed, "the retried fetch succeeded");
    expect(healed.value().access_token == "discovered-1", "the retried token decoded");
    expect(transport->discovery_hits() == 2, "the failed fetch was retried");
}

// HTTP and transport failures of the discovery request are typed failures
// the next call retries; none carry the response body.
void fetch_failure_is_typed_and_retried() {
    auto transport = std::make_shared<DiscoveryTransport>();
    OAuthSessions sessions(transport);
    transport->fail_discovery(500);
    auto failed = sessions.client_credentials_token("identity", identity_options());
    expect(!failed.ok(), "the HTTP failure failed");
    expect(failed.error().kind == OAuthError::Kind::DiscoveryFailed, "the HTTP failure is typed");
    expect(failed.error().status == 500, "the status surfaced");
    expect(failed.error().message().find("boom") == std::string::npos,
        "the failure never carries the response body");
    transport->fail_discovery(0);
    expect_ok(sessions.client_credentials_token("identity", identity_options()),
        "the retried fetch succeeded");
    expect(transport->discovery_hits() == 2, "the failed HTTP fetch was retried");
    // A transport-level failure is a typed Transport refusal the next call
    // retries. A fresh session owns a fresh discovery cache, so the retry
    // really fetches.
    OAuthSessions fresh(transport);
    transport->fail_discovery_transport(true);
    auto unreachable = fresh.client_credentials_token("identity", identity_options());
    expect(!unreachable.ok(), "the transport failure failed");
    expect(unreachable.error().kind == OAuthError::Kind::Transport, "the transport failure is typed");
    transport->fail_discovery_transport(false);
    expect_ok(fresh.client_credentials_token("identity", identity_options()),
        "the retried transport succeeded");
    expect(transport->discovery_hits() == 4, "the failed transport fetch was retried: "
        + std::to_string(transport->discovery_hits()));
}

// The compiled precedence: the configured scheme keeps its compiled
// endpoints everywhere, while the discovery-defined scheme resolves
// revocation, introspection and refresh through the discovery document.
void compiled_endpoints_win_over_discovery() {
    auto transport = std::make_shared<DiscoveryTransport>();
    OAuthSessions sessions(transport);
    const auto options = identity_options();
    // The compiled scheme never fetches discovery.
    expect_ok(sessions.client_credentials_token("service", options), "compiled acquisition");
    expect(transport->discovery_hits() == 0, "a compiled endpoint needs no discovery");
    expect(transport->urls()[0] == kCompiledToken, "the compiled token endpoint won");
    expect_ok(sessions.revoke_token("service", "the-token-value", options), "compiled revocation");
    expect(transport->urls().back() == kCompiledRevoke, "the compiled revocation endpoint won");
    expect_ok(sessions.introspect_token("service", "the-token-value", options), "compiled introspection");
    expect(transport->urls().back() == kCompiledIntrospect, "the compiled introspection endpoint won");
    // The discovery-defined scheme resolves everything through discovery.
    expect_ok(sessions.revoke_token("identity", "the-token-value", options), "discovered revocation");
    expect(transport->urls().back() == kDiscoveredRevoke, "revocation resolved through discovery");
    expect_ok(sessions.introspect_token("identity", "the-token-value", options), "discovered introspection");
    expect(transport->urls().back() == kDiscoveredIntrospect, "introspection resolved through discovery");
    // Refresh resolves through the discovered token endpoint.
    auto set = sessions.client_credentials_token("identity", options);
    expect_ok(set, "discovered acquisition");
    expect(set.value().has_refresh && set.value().refresh_token == "rotated-1",
        "the discovered token response carried a refresh token");
    auto rotated = sessions.refresh_token("identity", set.value(), options);
    expect_ok(rotated, "the discovered refresh succeeded");
    expect(transport->urls().back() == kDiscoveredToken, "refresh resolved through discovery");
    expect(transport->bodies().back().find("grant_type=refresh_token") != std::string::npos,
        "the refresh grant travelled");
    expect(rotated.value().access_token == "discovered-1", "the refresh returned the set");
    // A scheme with neither a compiled endpoint nor discovery stays refused.
    expect(sessions.refresh_token("unknown", set.value(), options).error().kind
            == OAuthError::Kind::UnknownScheme,
        "an unknown scheme stays typed");
}

} // namespace

int main() {
    discovered_token_endpoint_used_and_cached();
    single_flight();
    issuer_mismatch_is_typed_and_retried();
    fetch_failure_is_typed_and_retried();
    compiled_endpoints_win_over_discovery();
    return failures == 0 ? 0 : 1;
}
"#;

fn tool(variable: &str, fallback: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Some(std::path::PathBuf::from(path));
    }
    let probe = Command::new(fallback).arg("--version").output().ok()?;
    probe
        .status
        .success()
        .then(|| std::path::PathBuf::from(fallback))
}

/// Runs the command and fails the test with the retained log on failure.
fn checked(command: &mut Command, retained: &std::path::Path) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("required native tool {command:?}: {error}"));
    let log = retained.join("commands.log");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    let _ = writeln!(
        file,
        "\n{command:?}\nstatus: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "native oauth gate retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Native behavior of the generated lifecycle, when the C++ toolchain is
/// available. Degrades to the static assertions above otherwise.
#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn native_lifecycle_acquires_refreshes_polls_and_revokes() {
    let Some(cmake) = tool("SUSPECT_CPP_CMAKE", "cmake") else {
        eprintln!("cpp_oauth: cmake not available; degrading to static assertions");
        return;
    };
    let Some(cxx) = tool("SUSPECT_CPP_CXX", "clang++") else {
        eprintln!("cpp_oauth: no C++20 compiler; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&oauth_options()), root.path()).unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(consumer.join("main.cpp"), CONSUMER).unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
    let ctest = cmake.with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.path().join("build"))
            .arg("--output-on-failure"),
        root.path(),
    );
}

const CONSUMER_CMAKE: &str = r#"cmake_minimum_required(VERSION 3.24)
project(OAuthConsumer LANGUAGES CXX)
set(SUSPECT_SDK_WITH_CURL OFF CACHE BOOL "" FORCE)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../cpp oauth-build)
add_executable(consumer main.cpp)
target_link_libraries(consumer PRIVATE oauth_cpp::oauth_cpp)
set_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)
target_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)
enable_testing()
add_test(NAME oauth COMMAND consumer)
"#;

/// One JSON operation protected by a client-credentials scheme and one SSE
/// stream operation protected by a second scheme, exactly like the reference
/// replay fixtures: the stream-protected scheme's attaches are ineligible.
fn replay_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {
                    "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
                    "additionalProperties": false
                }}}}}
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
/// gate, whose emission must stay byte-identical to the pre-replay bytes.
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
/// client-credentials flow, reserves its two type names only then, and
/// compiles the stream-protected operations of its scheme.
#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate_document(replay_document(), &replay_options());
    let oauth = file(&configured, "include/oauth_cpp/oauth.hpp");
    for expected in [
        // The compiled stream-protected operations: the streaming operation's
        // pointer compiles for its scheme, the JSON operation's does not.
        "inline const std::vector<std::pair<std::string_view, std::vector<std::string_view>>> oauth_replay_protected = {",
        r#"std::string_view("feedOAuth""#,
        r#"std::string_view("/paths/~1events/get""#,
        // The replaying credential wrapper and its transport.
        "class OAuthReplayCredentials final : public std::enable_shared_from_this<OAuthReplayCredentials>",
        "OAuthReplayCredentials::create(",
        "class OAuthReplayTransport final : public Transport",
        "OAuthReplayTransport::send(const HttpRequest& request,",
        // The opt-in semantics and the lifecycle exclusion.
        "The opt-in replaying credential wrapper extends exactly the compiled",
        "one eligible request replay per qualifying",
        "Attaches served on stream-protected operations are never",
        "exact-target guard is defense in depth against loops",
        "a newer stored set wins over a stale",
        "concurrent 401s share one round",
        "every waiter exactly once",
        // The wrapper coordinates over the plain lifecycle's exact partition.
        "static std::string replay_store_key(",
        "return store_key(descriptor, options);",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.hpp is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("~1widgets/get"));
    // The replay budget: exactly one refresh plus one replay, never nested.
    assert!(oauth.contains("one refresh plus one replay, never nested"));
    assert!(oauth.contains("if (!owner_->replayable(presented)) return response;"));
    assert!(oauth.contains("if (owner_->lifecycle(request.url)) return response;"));

    // A plan without any executable client-credentials flow compiles exactly
    // the pre-replay bytes: the wrapper, its transport, the shared store-key
    // derivation and the protected-operations table are all absent.
    let code_only = generate_document(interactive_code_document(), &code_only_options());
    let plain_oauth = file(&code_only, "include/oauth_cpp/oauth.hpp");
    for absent in [
        "OAuthReplayCredentials",
        "OAuthReplayTransport",
        "oauth_replay_protected",
        "replay_store_key",
        "replay_value",
    ] {
        assert!(
            !plain_oauth.contains(absent),
            "the code-only emission gained {absent}"
        );
    }
    assert!(
        plain_oauth.contains("refresh_token(const std::string& scheme, const OAuthTokenSet& set"),
        "the code-only control still compiles the plain lifecycle"
    );
}

const REPLAY_CONSUMER: &str = r#"// Scripted-transport consumer asserting the replaying credential wrapper.
#include <oauth_cpp/sdk.hpp>
#include <oauth_cpp/oauth.hpp>

#include <chrono>
#include <iostream>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>

using namespace oauth_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

template<class T>
void expect_ok(const Result<T, OAuthError>& result, const std::string& message) {
    if (!result.ok()) {
        std::cerr << "failed: " << message << ": " << result.error().message() << "\n";
        ++failures;
    }
}

template<class T>
void expect_attached(const Result<T, TransportError>& result, const std::string& message) {
    if (!result.ok()) {
        std::cerr << "failed: " << message << ": " << result.error().message << "\n";
        ++failures;
    }
}

/// The client call surface: Result<Success, std::variant<SdkError, ...>>.
template<class R>
void expect_success(const R& result, const std::string& message) {
    if (!result.ok()) {
        const auto& error = std::get<SdkError>(result.error());
        std::cerr << "failed: " << message << ": " << error.message << "\n";
        ++failures;
    }
}

std::string lowered(std::string name) {
    for (auto& character : name) {
        if (character >= 'A' && character <= 'Z') character = static_cast<char>(character - 'A' + 'a');
    }
    return name;
}

std::string authorization_header(const Headers& headers) {
    for (const auto& [name, value] : headers) {
        if (lowered(name) == "authorization") return value;
    }
    return {};
}

// Fake API and token endpoints. `arm_stale` makes the next issued token
// answer 401 on the API, so the driver counts exactly one refresh.
class ReplayServer final : public Transport {
public:
    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions&) const override {
        std::lock_guard<std::mutex> guard(mutex_);
        const auto presented = authorization_header(request.headers);
        requests_.push_back({request.url, presented});
        if (request.url == "https://auth.oauth.test/token") {
            ++tokens_;
            if (token_401_) return respond(401, R"({"error":"stale"})");
            const auto token = "svc-" + std::to_string(tokens_);
            if (stale_next_) {
                stale_next_ = false;
                stale_token_ = token;
            }
            if (fail_from_ != 0 && tokens_ >= fail_from_) return respond(500, R"({"error":"server_error"})");
            return respond(200, R"({"access_token":")" + token + R"(","token_type":"Bearer","expires_in":3600})");
        }
        if (request.url == "https://auth.oauth.test/feed-token") {
            ++feed_tokens_;
            return respond(200, R"({"access_token":"feed-1","token_type":"Bearer","expires_in":3600})");
        }
        if (request.url == "https://api.oauth.test/v1/widgets") {
            if (always_401_ || (!stale_token_.empty() && presented == "Bearer " + stale_token_)) {
                return respond(401, R"({"error":"stale"})");
            }
            return respond(200, R"({"ok":true})");
        }
        if (request.url == "https://api.oauth.test/v1/events") return respond(401, R"({"error":"stream-denied"})");
        return respond(404, "");
    }
    void arm_stale() const { stale_next_ = true; }
    void always_401() const { always_401_ = true; }
    void fail_from(int from) const { fail_from_ = from; }
    void fail_token_once() const { token_401_ = true; }
    int tokens() const { return tokens_; }
    int feed_tokens() const { return feed_tokens_; }
    /// The presented Authorization values of the requests whose URL carries
    /// the suffix, in request order.
    std::vector<std::string> presented_to(const std::string& suffix) const {
        std::lock_guard<std::mutex> guard(mutex_);
        std::vector<std::string> found;
        for (const auto& [url, presented] : requests_) {
            if (url.find(suffix) != std::string::npos) found.push_back(presented);
        }
        return found;
    }
    std::vector<std::string> hits(const std::string& suffix) const {
        std::lock_guard<std::mutex> guard(mutex_);
        std::vector<std::string> found;
        for (const auto& [url, presented] : requests_) {
            (void)presented;
            if (url.find(suffix) != std::string::npos) found.push_back(url);
        }
        return found;
    }

private:
    Result<HttpResponse, TransportError> respond(int status, std::string body) const {
        HttpResponse response;
        response.status = status;
        response.headers = Headers{{"Content-Type", "application/json"}};
        response.body = std::move(body);
        return Result<HttpResponse, TransportError>::success(std::move(response));
    }
    mutable std::mutex mutex_;
    mutable std::vector<std::pair<std::string, std::string>> requests_;
    mutable int tokens_ = 0;
    mutable int feed_tokens_ = 0;
    mutable bool stale_next_ = false;
    mutable bool always_401_ = false;
    mutable bool token_401_ = false;
    mutable int fail_from_ = 0;
    mutable std::string stale_token_;
};

OAuthTokenOptions service_options() {
    OAuthTokenOptions options;
    options.client_id = "cid";
    options.client_secret = "csecret";
    return options;
}

OAuthTokenOptions feed_options() {
    OAuthTokenOptions options;
    options.client_id = "fid";
    options.client_secret = "fsecret";
    return options;
}

CredentialRequest widget_request() {
    CredentialRequest request;
    request.operation_source = Source{"https://source.oauth.test/openapi.json", "/paths/~1widgets/get", 0, 0};
    request.scheme_source = Source{"https://source.oauth.test/openapi.json", "/components/securitySchemes/serviceOAuth", 0, 0};
    request.operation_id = "listWidgets";
    request.scheme_name = "serviceOAuth";
    request.scopes = {"read"};
    return request;
}

CredentialRequest event_request() {
    CredentialRequest request;
    request.operation_source = Source{"https://source.oauth.test/openapi.json", "/paths/~1events/get", 0, 0};
    request.scheme_source = Source{"https://source.oauth.test/openapi.json", "/components/securitySchemes/feedOAuth", 0, 0};
    request.operation_id = "streamEvents";
    request.scheme_name = "feedOAuth";
    request.scopes = {"read"};
    return request;
}

// (a) 401 then success: one refresh, one replay, 200 surfaced.
void stale_then_replay() {
    auto server = std::make_shared<ReplayServer>();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options());
    expect_ok(replay, "the replaying credential was created");
    Credentials credentials;
    credentials.service_o_auth = replay.value()->credential();
    Client client(replay.value()->transport(server), credentials);
    server->arm_stale();
    auto result = client.list_widgets();
    expect_success(result, "the replay surfaced the 200");
    expect(server->hits("/v1/widgets").size() == 2, "one original plus one replay: "
        + std::to_string(server->hits("/v1/widgets").size()));
    expect(server->tokens() == 2, "one acquisition plus one refresh: " + std::to_string(server->tokens()));
    const auto presented = server->presented_to("/v1/widgets");
    expect(presented.size() == 2, "both API requests recorded: " + std::to_string(presented.size()));
    expect(presented[0] == "Bearer svc-1", "the original carried the stale token: " + presented[0]);
    expect(presented[1] == "Bearer svc-2", "the replay carried the fresh token: " + presented[1]);
}

// (b) the second 401 surfaces; exactly one refresh, one replay, no loops.
void second_401_surfaces() {
    auto server = std::make_shared<ReplayServer>();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    Credentials credentials;
    credentials.service_o_auth = replay->credential();
    Client client(replay->transport(server), credentials);
    server->always_401();
    auto result = client.list_widgets();
    expect(!result.ok(), "the second 401 surfaced");
    const auto& error = std::get<SdkError>(result.error());
    expect(error.kind == SdkError::Kind::UnexpectedResponse, "the second 401 surfaced as the declared error");
    expect(error.response && error.response->status == 401, "the surfaced status is 401");
    expect(server->hits("/v1/widgets").size() == 2, "one replay, no loops: "
        + std::to_string(server->hits("/v1/widgets").size()));
    expect(server->tokens() == 2, "exactly one refresh: " + std::to_string(server->tokens()));
}

// (c) two concurrent 401s across two threads: ONE refresh, two replays.
void concurrent_401s_share_one_refresh() {
    auto server = std::make_shared<ReplayServer>();
    server->arm_stale();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    auto attach = replay->credential();
    auto attached = attach(widget_request());
    expect_attached(attached, "the attach served the stale token");
    auto wrapped = replay->transport(server);
    const std::string presented = attached.value().scheme + " " + attached.value().value;
    std::vector<std::string> statuses(2, "failed");
    {
        std::vector<std::jthread> workers;
        for (int worker = 0; worker < 2; ++worker) {
            workers.emplace_back([&statuses, &wrapped, &presented, worker] {
                HttpRequest request;
                request.method = "GET";
                request.url = "https://api.oauth.test/v1/widgets";
                request.headers = Headers{{"Authorization", presented}};
                auto outcome = wrapped->send(request, TransportOptions{});
                if (outcome.ok() && outcome.value().status == 200) {
                    statuses[static_cast<std::size_t>(worker)] = "ok";
                }
            });
        }
    }
    for (const auto& status : statuses) expect(status == "ok", "every replay surfaced the 200");
    const auto recorded = server->presented_to("/v1/widgets");
    expect(recorded.size() == 4, "two originals plus two replays: " + std::to_string(recorded.size()));
    int fresh_replays = 0;
    for (const auto& value : recorded) {
        if (value != "Bearer svc-1") ++fresh_replays;
    }
    expect(fresh_replays == 2, "both replays carried the fresh token: " + std::to_string(fresh_replays));
    expect(server->tokens() == 2, "one shared refresh: " + std::to_string(server->tokens()));
}

// (d) a stream-protected attach is ineligible: the typed 401 surfaces with no
// replay and no refresh.
void streaming_operation_never_replays() {
    auto server = std::make_shared<ReplayServer>();
    auto replay = OAuthReplayCredentials::create(server, "feedOAuth", feed_options());
    expect_ok(replay, "the feed replaying credential was created");
    auto attach = replay.value()->credential();
    auto attached = attach(event_request());
    expect_attached(attached, "the stream attach served the token");
    auto wrapped = replay.value()->transport(server);
    HttpRequest request;
    request.method = "GET";
    request.url = "https://api.oauth.test/v1/events";
    request.headers = Headers{{"Authorization", attached.value().scheme + " " + attached.value().value}};
    auto outcome = wrapped->send(request, TransportOptions{});
    expect(outcome.ok() && outcome.value().status == 401, "the stream 401 surfaced");
    expect(server->hits("/v1/events").size() == 1, "no replay for the streaming operation: "
        + std::to_string(server->hits("/v1/events").size()));
    expect(server->feed_tokens() == 1, "no refresh for the streaming operation: "
        + std::to_string(server->feed_tokens()));
}

// (e) the plain provider surfaces the 401 without any refresh or replay.
void plain_provider_never_replays() {
    auto server = std::make_shared<ReplayServer>();
    server->arm_stale();
    const auto options = service_options();
    OAuthSessions sessions(server);
    auto token = sessions.client_credentials_token("serviceOAuth", options);
    expect_ok(token, "the plain acquisition succeeded");
    CredentialProvider plain = [&sessions, options](const CredentialRequest& request)
        -> Result<Authorization, TransportError> {
        auto acquired = sessions.client_credentials_token(request.scheme_name, options);
        if (!acquired) {
            TransportError failure;
            failure.kind = TransportError::Kind::Configuration;
            failure.message = std::move(acquired).error().message();
            return Result<Authorization, TransportError>::failure(std::move(failure));
        }
        return Result<Authorization, TransportError>::success(
            Authorization(acquired.value().token_type, acquired.value().access_token));
    };
    Credentials credentials;
    credentials.service_o_auth = plain;
    Client client(server, credentials);
    auto result = client.list_widgets();
    expect(!result.ok(), "the plain lifecycle surfaced the 401");
    const auto& error = std::get<SdkError>(result.error());
    expect(error.kind == SdkError::Kind::UnexpectedResponse, "the 401 surfaced");
    expect(server->hits("/v1/widgets").size() == 1, "no replay: "
        + std::to_string(server->hits("/v1/widgets").size()));
    expect(server->tokens() == 1, "no refresh: " + std::to_string(server->tokens()));
}

// (f) a failed refresh surfaces the typed auth failure, with no replay.
void refresh_failure_is_typed_and_never_replays() {
    auto server = std::make_shared<ReplayServer>();
    server->arm_stale();
    server->fail_from(2);
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    Credentials credentials;
    credentials.service_o_auth = replay->credential();
    Client client(replay->transport(server), credentials);
    auto result = client.list_widgets();
    expect(!result.ok(), "the refresh failure surfaced");
    const auto& error = std::get<SdkError>(result.error());
    expect(error.kind == SdkError::Kind::Configuration,
        "the typed auth failure surfaced instead of a replay");
    expect(error.transport && error.transport->message.find("server-rejected") != std::string::npos,
        "the typed failure names the OAuth kind");
    expect(error.message.find("csecret") == std::string::npos, "the failure never carries the secret");
    expect(server->hits("/v1/widgets").size() == 1, "no replay after a failed refresh: "
        + std::to_string(server->hits("/v1/widgets").size()));
    expect(server->tokens() == 2, "the refresh was attempted exactly once: " + std::to_string(server->tokens()));
}

// A token this provider never served never triggers a refresh or a replay.
void foreign_token_never_replays() {
    auto server = std::make_shared<ReplayServer>();
    server->always_401();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    auto wrapped = replay->transport(server);
    HttpRequest request;
    request.method = "GET";
    request.url = "https://api.oauth.test/v1/widgets";
    request.headers = Headers{{"Authorization", "Bearer someone-elses-token"}};
    auto outcome = wrapped->send(request, TransportOptions{});
    expect(outcome.ok() && outcome.value().status == 401, "the foreign 401 surfaced untouched");
    expect(server->hits("/v1/widgets").size() == 1, "no replay");
    expect(server->tokens() == 0, "no refresh");
}

// Lifecycle endpoint requests are never replayed: a 401 on the token
// endpoint itself surfaces untouched.
void lifecycle_endpoints_never_replay() {
    auto server = std::make_shared<ReplayServer>();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    auto attach = replay->credential();
    auto attached = attach(widget_request());
    expect_attached(attached, "the attach served the token");
    server->fail_token_once();
    auto wrapped = replay->transport(server);
    HttpRequest request;
    request.method = "POST";
    request.url = "https://auth.oauth.test/token";
    request.headers = Headers{{"Authorization", attached.value().scheme + " " + attached.value().value}};
    auto outcome = wrapped->send(request, TransportOptions{});
    expect(outcome.ok() && outcome.value().status == 401, "the lifecycle 401 surfaced untouched");
    expect(server->tokens() == 2, "no refresh for the lifecycle endpoint: "
        + std::to_string(server->tokens()));
    expect(server->hits("/token").size() == 2, "no replay for the lifecycle endpoint: "
        + std::to_string(server->hits("/token").size()));
}

// A single attach record stays bounded: a token outside the record never
// matches, so an unserved 401 surfaces untouched and triggers no refresh,
// while a recent served value still coordinates its one refresh and replay.
void served_record_stays_bounded() {
    auto server = std::make_shared<ReplayServer>();
    server->arm_stale();
    auto replay = OAuthReplayCredentials::create(server, "serviceOAuth", service_options()).value();
    auto attach = replay->credential();
    std::string twelfth;
    for (int round = 0; round < 12; ++round) {
        auto attached = attach(widget_request());
        expect_attached(attached, "attach succeeded");
        twelfth = attached.value().scheme + " " + attached.value().value;
    }
    auto wrapped = replay->transport(server);
    HttpRequest unserved;
    unserved.method = "GET";
    unserved.url = "https://api.oauth.test/v1/widgets";
    unserved.headers = Headers{{"Authorization", "Bearer at-never-served"}};
    auto outcome = wrapped->send(unserved, TransportOptions{});
    expect(outcome.ok() && outcome.value().status == 200, "an unserved token rides through untouched");
    expect(server->tokens() == 1, "no refresh outside the served record: "
        + std::to_string(server->tokens()));
    HttpRequest recent;
    recent.method = "GET";
    recent.url = "https://api.oauth.test/v1/widgets";
    recent.headers = Headers{{"Authorization", twelfth}};
    auto replayed = wrapped->send(recent, TransportOptions{});
    expect(replayed.ok() && replayed.value().status == 200, "the recent attach still replays");
    expect(server->tokens() == 2, "the recent attach refreshed exactly once: "
        + std::to_string(server->tokens()));
}

} // namespace

int main() {
    stale_then_replay();
    second_401_surfaces();
    concurrent_401s_share_one_refresh();
    streaming_operation_never_replays();
    plain_provider_never_replays();
    refresh_failure_is_typed_and_never_replays();
    foreign_token_never_replays();
    lifecycle_endpoints_never_replay();
    served_record_stays_bounded();
    return failures == 0 ? 0 : 1;
}
"#;

/// Native behavior of the replaying credential wrapper, when the C++ toolchain
/// is available. Degrades to the static assertions above otherwise.
#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn native_replay_lifecycle_over_a_stubbed_transport() {
    let Some(cmake) = tool("SUSPECT_CPP_CMAKE", "cmake") else {
        eprintln!("cpp_oauth: cmake not available; degrading to static assertions");
        return;
    };
    let Some(cxx) = tool("SUSPECT_CPP_CXX", "clang++") else {
        eprintln!("cpp_oauth: no C++20 compiler; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate_document(replay_document(), &replay_options()),
        root.path(),
    )
    .unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(consumer.join("main.cpp"), REPLAY_CONSUMER).unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
    let ctest = cmake.with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.path().join("build"))
            .arg("--output-on-failure"),
        root.path(),
    );
}
