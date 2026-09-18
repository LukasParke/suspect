//! M5 part 2: the emitted-only OAuth runtime for the Rust backend. The
//! canonical generation path compiles the shared OAuth selection and emits a
//! dependency-free token lifecycle module, while unconfigured plans stay
//! byte-identical.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
    rust_http,
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.oauth-runtime.test/openapi.json";
const TOKEN_URL: &str = "https://auth.oauth-runtime.test/token";
const REFRESH_URL: &str = "https://auth.oauth-runtime.test/token-refresh";
const AUTHORIZE_URL: &str = "https://auth.oauth-runtime.test/authorize";
const DEVICE_URL: &str = "https://auth.oauth-runtime.test/device";
const REVOKE_URL: &str = "https://auth.oauth-runtime.test/revoke";
const INTROSPECT_URL: &str = "https://auth.oauth-runtime.test/introspect";

/// One confidential scheme carrying client-credentials, authorization-code
/// (PKCE S256) and device flows, plus one public client-credentials scheme,
/// each used by an operation. The device authorization and authorization-code
/// flows are OpenAPI 3.2 constructs.
fn oauth_document() -> Value {
    json!({
        "openapi":"3.2.0", "info":{"title":"OAuth runtime","version":"1"},
        "servers":[{"url":"https://api.oauth-runtime.test/v1"}],
        "components":{
            "securitySchemes":{
                "service":{
                    "type":"oauth2",
                    "flows":{
                        "clientCredentials":{
                            "tokenUrl":TOKEN_URL,
                            "refreshUrl":REFRESH_URL,
                            "scopes":{"read":"Read access"}
                        },
                        "authorizationCode":{
                            "authorizationUrl":AUTHORIZE_URL,
                            "tokenUrl":TOKEN_URL,
                            "scopes":{"read":"Read access"}
                        },
                        "deviceAuthorization":{
                            "deviceAuthorizationUrl":DEVICE_URL,
                            "tokenUrl":TOKEN_URL,
                            "scopes":{}
                        }
                    }
                },
                "public":{
                    "type":"oauth2",
                    "flows":{"clientCredentials":{"tokenUrl":TOKEN_URL,"scopes":{"read":"Read access"}}}
                }
            },
            "schemas":{}
        },
        "paths":{
            "/widgets":{"get":{"operationId":"listWidgets","security":[{"service":["read"]}],"responses":{"200":{"description":"Ok"}}}},
            "/public-widgets":{"get":{"operationId":"listPublicWidgets","security":[{"public":["read"]}],"responses":{"200":{"description":"Ok"}}}}
        }
    })
}

fn contract_with_document(value: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec_pretty(&value).unwrap(),
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

fn contract() -> Arc<Contract> {
    contract_with_document(oauth_document())
}

fn selection(contract: &Contract) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::RustHttp,
        package_name: "oauth-rust".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1","pagination":"auto",
                "oauth":{"schemes":{
                    "service":{
                        "client_id_env":"SDK_OAUTH_TEST_CLIENT_ID",
                        "client_secret_env":"SDK_OAUTH_TEST_CLIENT_SECRET",
                        "revocation_endpoint":REVOKE_URL,
                        "introspection_endpoint":INTROSPECT_URL
                    },
                    "public":{"client_id_env":"SDK_OAUTH_TEST_PUBLIC_ID"}
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn generate(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = selection(&contract);
    backend::generate_with_options(contract, &selected, &target(), options).unwrap()
}

/// One OpenID Connect scheme whose endpoints the discovery document defines at
/// runtime, plus one OAuth2 scheme with configured auxiliary endpoints.
fn discovery_document() -> Value {
    json!({
        "openapi":"3.1.0", "info":{"title":"OAuth discovery","version":"1"},
        "servers":[{"url":"https://api.oauth-runtime.test/v1"}],
        "components":{
            "securitySchemes":{
                "identity":{
                    "type":"openIdConnect",
                    "openIdConnectUrl":"https://authority.oauth-runtime.test/.well-known/openid-configuration"
                },
                "service":{
                    "type":"oauth2",
                    "flows":{"clientCredentials":{"tokenUrl":"https://auth.oauth-runtime.test/token","scopes":{"read":"Read access"}}}
                }
            },
            "schemas":{}
        },
        "paths":{
            "/widgets":{"get":{"operationId":"listWidgets","security":[{"identity":["openid"]}],"responses":{"200":{"description":"Ok"}}}},
            "/gadgets":{"get":{"operationId":"listGadgets","security":[{"service":["read"]}],"responses":{"200":{"description":"Ok"}}}}
        }
    })
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "identity":{
                        "client_id_env":"SDK_OAUTH_DISCOVERY_ID",
                        "client_secret_env":"SDK_OAUTH_DISCOVERY_SECRET"
                    },
                    "service":{
                        "client_id_env":"SDK_OAUTH_DISCOVERY_ID",
                        "client_secret_env":"SDK_OAUTH_DISCOVERY_SECRET",
                        "revocation_endpoint":"https://auth.oauth-runtime.test/revoke",
                        "introspection_endpoint":"https://auth.oauth-runtime.test/introspect"
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The discovery engine, its typed failure kind, the per-sessions cache and
/// the compiled discovery URL emit exactly when a scheme compiles a discovery
/// URL; the plain fixture (no discovery) emits none of them.
#[test]
fn discovery_schemes_emit_the_discovery_engine() {
    let configured = generate(discovery_document(), &discovery_options());
    let oauth = configured
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("discovery generation emits the oauth module");
    for expected in [
        "DiscoveryFailed",
        "struct DiscoveredEndpoints",
        "struct Discovered {",
        "struct DiscoveryCache",
        "const DISCOVERY_MAX_BYTES: usize = 1 << 20",
        "const DISCOVERY_LIMITS: crate::http::Limits",
        "async fn discovery_document<T: Transport>(",
        "async fn discovery_fetch<T: Transport>(",
        "fn discovered_form_operation(",
        "fn discovery_origin(",
        "fn discovery_split(",
        "async fn acquire_discovered<T: Transport>(",
        "the discovery document issuer does not share the discovery URL's origin",
        r#"discovery_url: std::option::Option::Some("https://authority.oauth-runtime.test/.well-known/openid-configuration")"#,
        "discovery_round: DISCOVERY_ROUND_0",
        // The discovery-aware providers replaced the compiled-only ones.
        "pub async fn client_credentials_token",
        "pub async fn refresh_token",
        "pub async fn revoke_token",
        "pub async fn introspect_token",
    ] {
        assert!(oauth.content.contains(expected), "missing {expected}");
    }
    // The lib.rs wiring is unchanged by discovery: no new client methods.
    let lib = configured
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap();
    assert!(
        lib.content
            .contains("#[cfg(feature=\"http\")]\npub mod oauth;\n")
    );

    // Control: the plain fixture carries no discovery URL, so the engine is
    // absent entirely.
    let plain = generate(oauth_document(), &configured_options());
    let plain_oauth = plain
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("the plain fixture still emits the oauth module");
    assert!(!plain_oauth.content.contains("DiscoveryFailed"));
    assert!(!plain_oauth.content.contains("DiscoveryCache"));
    assert!(!plain_oauth.content.contains("discovery_url"));
    assert!(!plain_oauth.content.contains("discovery_round"));
}

#[test]
fn sdk_defaults_compile_the_oauth_plan_into_the_v3_plan_only() {
    let contract = contract();
    let defaults: SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":"auto"})).unwrap();
    let plan = rust_http::plan_http_v3(
        contract.clone(),
        &selection(&contract),
        rust_http::HttpConfig {
            sdk_defaults: Some(defaults),
            ..Default::default()
        },
    )
    .unwrap();
    let compiled = plan.oauth().expect("the v3 plan carries the OAuth plan");
    assert_eq!(compiled.schemes.len(), 2);
    let service = compiled
        .schemes
        .iter()
        .find(|scheme| scheme.name == "service")
        .unwrap();
    assert_eq!(service.refresh_skew_seconds, 30);
    assert_eq!(
        service
            .flows
            .iter()
            .find(|flow| flow.token_url.as_deref() == Some(TOKEN_URL))
            .map(|flow| flow.client_auth),
        Some(suspect_codegen::http_protocol::OAuthClientAuth::None),
        "without configured secrets the compiled client stays public"
    );

    // The retained v1/v2 planning APIs never populate OAuth.
    let v2 = rust_http::plan_http_v2(
        contract.clone(),
        &selection(&contract),
        rust_http::HttpConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version":"v1","oauth":"auto"
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v2.oauth().is_none());
    // Explicitly disabled defaults keep the plan empty too.
    let off = rust_http::plan_http_v3(
        contract.clone(),
        &selection(&contract),
        rust_http::HttpConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version":"v1","oauth":"off"
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(off.oauth().is_none());
}

#[test]
fn configured_generation_emits_oauth_and_unconfigured_generation_emits_nothing() {
    let configured = generate(oauth_document(), &configured_options());
    let oauth = configured
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("configured generation emits the oauth module");
    for expected in [
        "pub struct TokenSet",
        "pub trait TokenStore",
        "pub struct MemoryTokenStore",
        "pub struct TokenSessions",
        "pub struct AuthError",
        "pub enum AuthErrorKind",
        "pub struct DeviceAuthorization",
        "pub enum DevicePoll",
        "pub struct AuthorizationTransaction",
        "pub async fn client_credentials_token",
        "pub async fn refresh_token",
        "pub async fn revoke_token",
        "pub async fn introspect_token",
        "pub async fn begin_device_authorization",
        "pub async fn poll_device_token",
        "pub async fn poll_device_token_until_complete",
        "pub fn begin_authorization",
        "pub async fn complete_authorization",
        "pub fn pkce_s256_challenge",
        "fn sha256(",
        "fn base64url(",
        "fn constant_time_eq(",
        // Entropy: kernel CSPRNG on unix, typed refusal elsewhere.
        "/dev/urandom",
        "#[cfg(unix)]\nfn random_bytes(",
        "#[cfg(not(unix))]\nfn random_bytes(",
        "UnsupportedPlatform",
        "std::env::var",
        r#"client_id_env: std::option::Option::Some("SDK_OAUTH_TEST_CLIENT_ID")"#,
        r#"client_id_env: std::option::Option::Some("SDK_OAUTH_TEST_PUBLIC_ID")"#,
        "application/x-www-form-urlencoded",
        "static SCHEMES: &[Scheme]",
        r#"authorization_url: std::option::Option::Some("https://auth.oauth-runtime.test/authorize")"#,
    ] {
        assert!(oauth.content.contains(expected), "missing {expected}");
    }
    // Compiled descriptors embed the declared endpoints as consts, split into
    // an origin server template and a path; no secret value ever appears in
    // the emitted bytes.
    assert!(oauth.content.contains("https://auth.oauth-runtime.test"));
    for path in [
        "/token",
        "/token-refresh",
        "/device",
        "/revoke",
        "/introspect",
    ] {
        assert!(
            oauth.content.contains(&format!("path_template: {path:?}")),
            "missing compiled path {path}"
        );
    }
    assert!(oauth.content.contains("SDK_OAUTH_TEST_CLIENT_SECRET"));
    let lib = configured
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap();
    assert!(
        lib.content
            .contains("#[cfg(feature=\"http\")]\npub mod oauth;\n")
    );
    for method in [
        "pub async fn client_credentials_token<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn refresh_token<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn revoke_token<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn introspect_token<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn begin_device_authorization<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn poll_device_token<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn poll_device_token_until_complete<S: oauth::TokenStore",
        "pub fn begin_authorization<S: oauth::TokenStore>(&self, sessions:",
        "pub async fn complete_authorization<S: oauth::TokenStore>(&self, sessions:",
    ] {
        assert!(
            lib.content.contains(method),
            "missing client method {method}"
        );
    }

    // Operations modules and every other artifact stay byte-identical to the
    // unconfigured emission; only lib.rs changes and oauth.rs is new.
    let unconfigured = generate(oauth_document(), &GenerationOptions::default());
    let configured_by_path = configured
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let unconfigured_by_path = unconfigured
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        configured_by_path.len(),
        unconfigured_by_path.len() + 1,
        "configured emission adds exactly the oauth module"
    );
    for (path, content) in &unconfigured_by_path {
        match *path {
            "rust/src/lib.rs" => assert_ne!(*content, configured_by_path[path]),
            other => assert_eq!(
                *content, configured_by_path[other],
                "{other} must not change under oauth emission"
            ),
        }
    }
    assert!(configured_by_path.contains_key("rust/src/oauth.rs"));
    let lib = unconfigured_by_path["rust/src/lib.rs"];
    assert!(!lib.contains("pub mod oauth"));
    assert!(!lib.contains("client_credentials_token"));
}

/// An operation whose native name collides with a generated OAuth method is
/// renamed instead of the generated method.
#[test]
fn oauth_method_names_are_reserved_against_operation_collisions() {
    let mut document = oauth_document();
    document["paths"]["/refresh"] = json!({
        "get":{"operationId":"refresh_token","responses":{"200":{"description":"Ok"}}}
    });
    let files = generate(document, &configured_options());
    let lib = files
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap();
    assert!(
        lib.content
            .contains("pub async fn refresh_token_2(&self,input:"),
        "the colliding operation is renamed: {}",
        lib.content
    );
    assert!(
        lib.content
            .contains("pub async fn refresh_token<S: oauth::TokenStore>(&self, sessions:"),
        "the generated OAuth method keeps its name"
    );
}

/// Schemes whose only flows are deprecated or discovery-defined compile to
/// nothing at all.
#[test]
fn schemes_without_executable_flows_emit_nothing() {
    for (name, flows) in [
        (
            "implicit-only",
            json!({"implicit":{"authorizationUrl":"https://auth.oauth-runtime.test/authorize","scopes":{"read":"Read"}}}),
        ),
        (
            "password-only",
            json!({"password":{"tokenUrl":TOKEN_URL,"scopes":{"read":"Read"}}}),
        ),
    ] {
        let mut document = oauth_document();
        document["components"]["securitySchemes"] = json!({"legacy":{
            "type":"oauth2","flows":flows
        }});
        document["paths"]["/widgets"]["get"]["security"] = json!([{"legacy":["read"]}]);
        document["paths"]
            .as_object_mut()
            .unwrap()
            .remove("/public-widgets");
        // Only the used scheme may be configured, and nothing about it.
        let options = GenerationOptions {
            sdk_defaults: Some(
                serde_json::from_value::<SdkDefaults>(json!({
                    "version":"v1",
                    "oauth":{"schemes":{"legacy":{}}}
                }))
                .unwrap(),
            ),
            ..Default::default()
        };
        let files = generate(document, &options);
        assert!(
            !files.iter().any(|file| file.path == "rust/src/oauth.rs"),
            "{name} must not emit the oauth module"
        );
        let lib = files
            .iter()
            .find(|file| file.path == "rust/src/lib.rs")
            .unwrap();
        assert!(!lib.content.contains("pub mod oauth"), "{name}");
        assert!(!lib.content.contains("client_credentials_token"), "{name}");
    }
}

/// An authorization-code-only scheme is now executable (PKCE S256 is
/// dependency-free), so it emits the module and compiles its endpoints.
#[test]
fn authorization_code_only_schemes_emit_the_pkce_runtime() {
    let mut document = oauth_document();
    document["components"]["securitySchemes"] = json!({"legacy":{
        "type":"oauth2","flows":{"authorizationCode":{
            "authorizationUrl":AUTHORIZE_URL,
            "tokenUrl":TOKEN_URL,
            "scopes":{"read":"Read"}
        }}
    }});
    document["paths"]["/widgets"]["get"]["security"] = json!([{"legacy":["read"]}]);
    document["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/public-widgets");
    let options = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1",
                "oauth":{"schemes":{"legacy":{"client_id_env":"SDK_OAUTH_TEST_PUBLIC_ID"}}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let files = generate(document, &options);
    let oauth = files
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("the authorization-code-only scheme now emits the oauth module");
    for expected in [
        "pub fn begin_authorization",
        "pub async fn complete_authorization",
        r#"authorization_url: std::option::Option::Some("https://auth.oauth-runtime.test/authorize")"#,
        "code_challenge_method=S256",
        "code_verifier",
    ] {
        assert!(oauth.content.contains(expected), "missing {expected}");
    }
    let lib = files
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap();
    assert!(
        lib.content
            .contains("pub async fn complete_authorization<S: oauth::TokenStore>")
    );
}

/// Per-user cargo target directory, so concurrent agents never contend on the
/// shared workspace lock while building emitted packages.
fn cargo_target() -> PathBuf {
    let user = std::env::var_os("USER")
        .or_else(|| std::env::var_os("LOGNAME"))
        .unwrap_or_else(|| format!("uid-{}", std::process::id()).into());
    std::env::temp_dir().join(format!(
        "suspect-rust-oauth-target-{}",
        user.to_string_lossy()
    ))
}

fn cargo(command: &str, manifest: &Path, args: &[&str]) -> (bool, String) {
    // Under a full-suite run the inner build contends with the outer cargo for
    // the shared package-cache lock; one bounded retry absorbs that load flake
    // while a genuine compile error still fails every attempt.
    let mut log = String::new();
    for attempt in 0..2 {
        let output = Command::new("cargo")
            .arg(command)
            .arg("--offline")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(cargo_target())
            .args(args)
            .env_remove("RUST_MIN_STACK")
            .env(
                "RUSTUP_TOOLCHAIN",
                std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").unwrap_or_default(),
            )
            .output()
            .expect("cargo is available");
        log = format!("{}", String::from_utf8_lossy(&output.stderr));
        if output.status.success() {
            return (true, log);
        }
        if attempt == 0 {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    (false, log)
}

fn registry_unavailable(log: &str) -> bool {
    log.contains("no matching package named")
        || log.contains("failed to download")
        || log.contains("error: failed to select a version")
        || log.contains("network disabled")
        || log.contains("could not download")
}

/// Behavioral verification: the emitted package and a dependency-free
/// consumer run the real token lifecycle against a fake transport.
#[test]
fn emitted_oauth_runtime_drives_the_token_lifecycle_in_a_compiled_package() {
    let configured = generate(oauth_document(), &configured_options());
    let directory = tempfile::Builder::new()
        .prefix("rust-oauth-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");

    // Model-only compilation needs no dependencies at all and must always
    // succeed offline.
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");

    // The http feature pins `url`; it compiles offline from the local cache.
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"oauth-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"oauth-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        // No registry access for the pinned `url` crate: degrade to the
        // model-only compile check plus the static assertions above and say so.
        eprintln!(
            "skipping behavioral oauth execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_OAUTH").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

/// Behavioral verification of the discovery runtime: the emitted package and
/// a dependency-free consumer run discovery-driven resolution against a fake
/// transport.
#[test]
fn discovery_oauth_runtime_drives_the_lifecycle_in_a_compiled_package() {
    let configured = generate(discovery_document(), &discovery_options());
    let directory = tempfile::Builder::new()
        .prefix("rust-oauth-discovery-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");

    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"oauth-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"oauth-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), DISCOVERY_CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping discovery behavioral execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "discovery consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_OAUTH").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

const DISCOVERY_CONSUMER: &str = r##"
//! Consumer-side behavioral proof for discovery-driven endpoint resolution.
#![allow(dead_code)]
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use sdk::oauth::{AuthError, AuthErrorKind, MemoryTokenStore, TokenSessions, TokenStore};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

fn response(status: u16, body: &str) -> TransportResponse<Body> {
    TransportResponse {
        status,
        headers: vec![("Content-Type".into(), b"application/json".to_vec())],
        body: Body(Some(body.as_bytes().to_vec())),
    }
}

const DISCOVERY_URL: &str = "https://authority.oauth-runtime.test/.well-known/openid-configuration";
const DISCOVERED_TOKEN_URL: &str = "https://authority.oauth-runtime.test/oauth/token";
const DISCOVERED_REVOKE_URL: &str = "https://authority.oauth-runtime.test/oauth/revoke";
const DISCOVERED_INTROSPECT_URL: &str =
    "https://authority.oauth-runtime.test/oauth/introspect";
const COMPILED_TOKEN_URL: &str = "https://auth.oauth-runtime.test/token";
const COMPILED_REVOKE_URL: &str = "https://auth.oauth-runtime.test/revoke";
const IDENTITY: &str = "identity";
const SERVICE: &str = "service";

/// The fake OpenID Connect provider: discovery, the discovered token
/// endpoint, revocation and introspection, plus the compiled scheme's
/// endpoints. `issuer` and `fail_status` are read at request time so tests
/// can flip behavior between calls.
#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    issuer: Arc<Mutex<&'static str>>,
    fail_status: Arc<Mutex<u16>>,
}

impl Server {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            issuer: Arc::new(Mutex::new("https://authority.oauth-runtime.test")),
            fail_status: Arc::new(Mutex::new(0)),
        }
    }
}

impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| String::from_utf8_lossy(value).to_string());
        let body_text = request
            .body
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        self.calls.lock().unwrap().push(format!(
            "{} {} auth={authorization:?} body={body_text}",
            request.method, request.url
        ));
        if request.url == DISCOVERY_URL {
            assert_eq!(request.method, "GET", "discovery is a GET");
            let accept = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
                .map(|(_, value)| String::from_utf8_lossy(value).to_string());
            assert_eq!(accept.as_deref(), Some("application/json"));
            if *self.fail_status.lock().unwrap() != 0 {
                return Ok(response(*self.fail_status.lock().unwrap(), "{}"));
            }
            return Ok(response(
                200,
                &format!(
                    r#"{{"issuer":"{}","token_endpoint":"{DISCOVERED_TOKEN_URL}","revocation_endpoint":"{DISCOVERED_REVOKE_URL}","introspection_endpoint":"{DISCOVERED_INTROSPECT_URL}","unknown_member":{{"nested":true}}}}"#,
                    *self.issuer.lock().unwrap()
                ),
            ));
        }
        if request.url == DISCOVERED_TOKEN_URL {
            let grant = body_text
                .split('&')
                .find_map(|pair| pair.split_once('='))
                .filter(|(key, _)| *key == "grant_type")
                .map(|(_, value)| value);
            assert!(
                grant == Some("client_credentials") || grant == Some("refresh_token"),
                "{body_text}"
            );
            return Ok(response(
                200,
                r#"{"access_token":"discovered-1","token_type":"Bearer","expires_in":3600,"refresh_token":"r1"}"#,
            ));
        }
        if request.url == DISCOVERED_REVOKE_URL {
            assert!(body_text.contains("token=at-1"));
            return Ok(response(200, "{}"));
        }
        if request.url == DISCOVERED_INTROSPECT_URL {
            return Ok(response(200, r#"{"active":true,"scope":"read"}"#));
        }
        if request.url == COMPILED_REVOKE_URL {
            return Ok(response(200, "{}"));
        }
        assert_eq!(request.url, COMPILED_TOKEN_URL, "{:?}", request.url);
        Ok(response(
            200,
            r#"{"access_token":"compiled","token_type":"Bearer","expires_in":3600}"#,
        ))
    }
}

fn sessions_and_client(
    server: &Server,
) -> (
    sdk::Client<Server>,
    TokenSessions<MemoryTokenStore>,
    Arc<MemoryTokenStore>,
) {
    let store = Arc::new(MemoryTokenStore::new());
    let sessions = TokenSessions::with_store(store.clone())
        .with_client_credentials("discovery-client", "discovery-secret");
    let client = sdk::Client::with_transport(server.clone(), sdk::Credentials::new());
    (client, sessions, store)
}

fn discovery_causes(error: &sdk::http::SdkError) -> Option<&AuthError> {
    error.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>())
}

#[test]
fn credentials_resolve_through_the_discovered_token_endpoint() {
    let server = Server::new();
    let (client, sessions, store) = sessions_and_client(&server);
    let first = block_on(sessions.client_credentials_token(&client, IDENTITY)).unwrap();
    assert_eq!(first.access_token, "discovered-1");
    let log = server.calls.lock().unwrap();
    assert_eq!(
        log.iter().filter(|call| call.starts_with(&format!("GET {DISCOVERY_URL}"))).count(),
        1,
        "exactly one discovery fetch: {log:?}"
    );
    assert_eq!(
        log.iter().filter(|call| call.starts_with(&format!("POST {DISCOVERED_TOKEN_URL}"))).count(),
        1,
        "the acquisition hit the discovered endpoint: {log:?}"
    );
    assert!(
        log.iter().all(|call| !call.contains(COMPILED_TOKEN_URL)),
        "the compiled fallback was never contacted: {log:?}"
    );
    assert!(
        log[1].contains("auth=Some(\"Basic "),
        "the discovery-defined client authenticates with basic: {log:?}"
    );
    drop(log);
    assert_eq!(
        block_on(store.load(IDENTITY)).unwrap().access_token,
        "discovered-1"
    );

    // The discovery document and the token set are both cached: the second
    // call fetches nothing.
    let second = block_on(sessions.client_credentials_token(&client, IDENTITY)).unwrap();
    assert_eq!(second.access_token, "discovered-1");
    assert_eq!(server.calls.lock().unwrap().len(), 2, "{calls:?}", calls = server.calls.lock().unwrap());

    // Two concurrent callers share one discovery fetch and one acquisition.
    let (fresh_client, fresh_sessions, _store) = sessions_and_client(&server);
    let first_caller = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            block_on(fresh_sessions.client_credentials_token(&fresh_client, IDENTITY)).unwrap()
        });
        let second = scope.spawn(|| {
            block_on(fresh_sessions.client_credentials_token(&fresh_client, IDENTITY)).unwrap()
        });
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        [first.access_token, second.access_token]
    });
    assert!(first_caller.iter().all(|token| token == "discovered-1"));
    let discovery_fetches = server
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with(&format!("GET {DISCOVERY_URL}")))
        .count();
    assert_eq!(discovery_fetches, 2, "concurrent callers single-flight discovery");
}

#[test]
fn issuer_mismatches_and_failures_are_typed_and_retried() {
    let server = Server::new();
    *server.issuer.lock().unwrap() = "https://elsewhere.oauth-runtime.test";
    let (client, sessions, _store) = sessions_and_client(&server);
    let failure = block_on(sessions.client_credentials_token(&client, IDENTITY))
        .err()
        .expect("an issuer mismatch must fail");
    let auth = discovery_causes(&failure).expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::DiscoveryFailed);
    assert_eq!(auth.scheme, IDENTITY);
    // The failed fetch is not cached: after the server is fixed the next call
    // retries and succeeds.
    *server.issuer.lock().unwrap() = "https://authority.oauth-runtime.test";
    let recovered = block_on(sessions.client_credentials_token(&client, IDENTITY)).unwrap();
    assert_eq!(recovered.access_token, "discovered-1");
    let discovery_fetches = server
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with(&format!("GET {DISCOVERY_URL}")))
        .count();
    assert_eq!(discovery_fetches, 2, "the failed fetch was retried");

    // A failing discovery request is a typed failure a fresh sessions value
    // retries on its next call.
    *server.fail_status.lock().unwrap() = 500;
    let (client, sessions, _store) = sessions_and_client(&server);
    let failure = block_on(sessions.client_credentials_token(&client, IDENTITY))
        .err()
        .expect("a failing request must fail");
    let auth = discovery_causes(&failure).expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::DiscoveryFailed);
    assert!(format!("{failure}").contains("discovery"));
    *server.fail_status.lock().unwrap() = 0;
    let recovered = block_on(sessions.client_credentials_token(&client, IDENTITY)).unwrap();
    assert_eq!(recovered.access_token, "discovered-1");
}

#[test]
fn revocation_introspection_and_refresh_follow_the_precedence() {
    let server = Server::new();
    let (client, sessions, store) = sessions_and_client(&server);
    // The discovery-defined scheme resolves revocation through discovery.
    block_on(sessions.revoke_token(
        &client,
        IDENTITY,
        &sdk::oauth::TokenSet {
            access_token: "at-1".into(),
            token_type: "Bearer".into(),
            expires_at: std::time::SystemTime::now(),
            refresh_token: None,
            scope: None,
            issuer_account: None,
        },
    ))
    .unwrap();
    let log = server.calls.lock().unwrap();
    assert!(
        log.last().unwrap().starts_with(&format!("POST {DISCOVERED_REVOKE_URL}")),
        "revocation resolved through discovery: {log:?}"
    );
    drop(log);
    assert!(block_on(store.load(IDENTITY)).is_none(), "revocation cleared the stored set");

    // The configured scheme keeps its compiled endpoint: it wins.
    block_on(sessions.revoke_token(
        &client,
        SERVICE,
        &sdk::oauth::TokenSet {
            access_token: "at-1".into(),
            token_type: "Bearer".into(),
            expires_at: std::time::SystemTime::now(),
            refresh_token: None,
            scope: None,
            issuer_account: None,
        },
    ))
    .unwrap();
    let log = server.calls.lock().unwrap();
    assert!(
        log.last().unwrap().starts_with(&format!("POST {COMPILED_REVOKE_URL}")),
        "the configured endpoint wins over discovery: {log:?}"
    );
    drop(log);

    // Introspection follows the same precedence.
    let introspection = block_on(sessions.introspect_token(
        &client,
        IDENTITY,
        &sdk::oauth::TokenSet {
            access_token: "at-1".into(),
            token_type: "Bearer".into(),
            expires_at: std::time::SystemTime::now(),
            refresh_token: None,
            scope: None,
            issuer_account: None,
        },
    ))
    .unwrap();
    let active = match &introspection {
        sdk::Nullable::Value(sdk::JsonNonNullValue::Object(map)) => matches!(
            map.get("active"),
            Some(sdk::Nullable::Value(sdk::JsonNonNullValue::Bool(true)))
        ),
        _ => false,
    };
    assert!(active, "unexpected introspection: {introspection:?}");
    let log = server.calls.lock().unwrap();
    assert!(
        log.last().unwrap().starts_with(&format!("POST {DISCOVERED_INTROSPECT_URL}")),
        "{log:?}"
    );
    drop(log);

    // Refresh resolves through the discovered token endpoint.
    let set = block_on(sessions.client_credentials_token(&client, IDENTITY)).unwrap();
    assert_eq!(set.refresh_token.as_deref(), Some("r1"));
    let refreshed = block_on(sessions.refresh_token(&client, IDENTITY, &set)).unwrap();
    assert_eq!(refreshed.access_token, "discovered-1");
    let log = server.calls.lock().unwrap();
    let refresh = log.last().unwrap();
    assert!(
        refresh.starts_with(&format!("POST {DISCOVERED_TOKEN_URL}")),
        "refresh resolved through discovery: {log:?}"
    );
    assert!(refresh.contains("grant_type=refresh_token"), "{refresh}");
    assert!(refresh.contains("refresh_token=r1"), "{refresh}");
}

/// Compiled endpoints win over discovery for schemes that carry both, and a
/// step with neither a compiled endpoint nor a discovery URL is a typed
/// refusal.
#[test]
fn compiled_endpoints_win_and_missing_steps_fail_typed() {
    let server = Server::new();
    let (client, sessions, _store) = sessions_and_client(&server);
    // The service scheme's client-credentials token endpoint is compiled: it
    // wins, and discovery is never contacted for it.
    let token = block_on(sessions.client_credentials_token(&client, SERVICE)).unwrap();
    assert_eq!(token.access_token, "compiled");
    let log = server.calls.lock().unwrap();
    assert!(
        log.last().unwrap().starts_with(&format!("POST {COMPILED_TOKEN_URL}")),
        "{log:?}"
    );
    assert!(
        log.iter().all(|call| !call.contains(DISCOVERY_URL)),
        "discovery was never fetched for the compiled scheme: {log:?}"
    );
    drop(log);

    // The identity scheme compiles no device flow and discovery supplies only
    // token, revocation and introspection endpoints: the device grant is a
    // typed refusal.
    let failure = block_on(sessions.begin_device_authorization(&client, IDENTITY))
        .err()
        .expect("the device grant has no endpoint");
    let auth = discovery_causes(&failure).expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::Unavailable);
    assert_eq!(auth.scheme, IDENTITY);
}
"##;

const CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the emitted OAuth runtime. All
//! environment mutation is confined to the single-threaded `env_credentials`
//! test; every other test uses explicit credentials, so no concurrent test
//! ever reads the process environment.
#![allow(dead_code)]
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use sdk::oauth::{AuthError, AuthErrorKind, DevicePoll, MemoryTokenStore, TokenSessions, TokenSet, TokenStore, pkce_s256_challenge};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

fn response(status: u16, body: &str) -> TransportResponse<Body> {
    TransportResponse {
        status,
        headers: vec![("Content-Type".into(), b"application/json".to_vec())],
        body: Body(Some(body.as_bytes().to_vec())),
    }
}

fn ok(token: &str, extra: &str) -> TransportResponse<Body> {
    response(
        200,
        &format!(
            r#"{{"access_token":"{token}","token_type":"Bearer","expires_in":3600,"scope":"read","iss":"https://auth.oauth-runtime.test"{extra}}}"#
        ),
    )
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        result.push(TABLE[usize::from(a >> 2)] as char);
        result.push(TABLE[usize::from((a & 3) << 4 | b >> 4)] as char);
        result.push(if part.len() > 1 {
            TABLE[usize::from((b & 15) << 2 | c >> 6)] as char
        } else {
            '='
        });
        result.push(if part.len() > 2 {
            TABLE[usize::from(c & 63)] as char
        } else {
            '='
        });
    }
    result
}

/// A fake OAuth token server. Token, refresh and device modes are read at
/// request time so tests can flip behavior between calls. `hold` parks token
/// requests after logging them, which lets a test guarantee a second caller
/// enters the single-flight round while the first is still in flight.
/// `device_script`, when armed, feeds the device token poll a scripted
/// sequence of responses one per request.
#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    token_mode: Arc<Mutex<&'static str>>,
    refresh_mode: Arc<Mutex<&'static str>>,
    device_mode: Arc<Mutex<&'static str>>,
    device_script: Arc<Mutex<Vec<&'static str>>>,
    hold: Arc<(Mutex<bool>, Condvar)>,
}

impl Server {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: calls.clone(),
                token_mode: Arc::new(Mutex::new("success")),
                refresh_mode: Arc::new(Mutex::new("rotate")),
                device_mode: Arc::new(Mutex::new("pending")),
                device_script: Arc::new(Mutex::new(Vec::new())),
                hold: Arc::new((Mutex::new(false), Condvar::new())),
            },
            calls,
        )
    }
}

fn release(server: &Server) {
    let (held, signal) = &*server.hold;
    *held.lock().unwrap() = false;
    signal.notify_all();
}

fn wait_for_token_request(server: &Server) {
    for _ in 0..500 {
        if server
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.contains("/token "))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the first token request never arrived");
}

impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        let body_text = request
            .body
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| String::from_utf8_lossy(value).to_string());
        self.calls.lock().unwrap().push(format!(
            "{} {} body={body_text} auth={authorization:?}",
            request.method, request.url
        ));
        if request.url.ends_with("/revoke") {
            assert!(body_text.contains("token=at-1"));
            assert!(body_text.contains("token_type_hint=access_token"));
            return Ok(response(200, "{}"));
        }
        if request.url.ends_with("/introspect") {
            return Ok(response(200, r#"{"active":true,"scope":"read"}"#));
        }
        if request.url.ends_with("/token-refresh") {
            return match *self.refresh_mode.lock().unwrap() {
                "retain" => Ok(ok("at-refreshed", "")),
                _ => Ok(ok("at-2", r#","refresh_token":"r2""#)),
            };
        }
        if request.url.ends_with("/device") {
            return Ok(response(
                200,
                r#"{"device_code":"dc-1","user_code":"ABCD-EFGH","verification_uri":"https://auth.oauth-runtime.test/activate","expires_in":300,"interval":5}"#,
            ));
        }
        assert_eq!(request.url, "https://auth.oauth-runtime.test/token");
        assert_eq!(request.method, "POST");
        // Hold after logging so the single-flight test can interleave a
        // second caller deterministically.
        let (held, signal) = &*self.hold;
        let mut guard = held.lock().unwrap();
        while *guard {
            let (next, _) = signal
                .wait_timeout(guard, std::time::Duration::from_secs(10))
                .unwrap();
            guard = next;
        }
        drop(guard);
        let mode = if body_text.contains("device_code=") {
            let mut script = self.device_script.lock().unwrap();
            if script.is_empty() {
                *self.device_mode.lock().unwrap()
            } else {
                script.remove(0)
            }
        } else {
            *self.token_mode.lock().unwrap()
        };
        match mode {
            "reject" => Ok(response(
                400,
                r#"{"error":"invalid_client","error_description":"client authentication failed"}"#,
            )),
            "expire" => Ok(response(
                200,
                r#"{"access_token":"at-expired","token_type":"Bearer","expires_in":0}"#,
            )),
            "pending" => Ok(response(400, r#"{"error":"authorization_pending"}"#)),
            "slow-down" => Ok(response(400, r#"{"error":"slow_down"}"#)),
            "expired-token" => Ok(response(400, r#"{"error":"expired_token"}"#)),
            _ => Ok(ok("at-1", r#","refresh_token":"r1""#)),
        }
    }
}

fn client_and_explicit_sessions(
    server: &Server,
) -> (
    sdk::Client<Server>,
    TokenSessions<MemoryTokenStore>,
    Arc<MemoryTokenStore>,
) {
    let store = Arc::new(MemoryTokenStore::new());
    let sessions = TokenSessions::with_store(store.clone())
        .with_client_credentials("profile-client", "profile-secret");
    let client = sdk::Client::with_transport(server.clone(), sdk::Credentials::new());
    (client, sessions, store)
}

fn token_set(access_token: &str, refresh_token: Option<&str>) -> TokenSet {
    TokenSet {
        access_token: access_token.into(),
        token_type: "Bearer".into(),
        expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
        refresh_token: refresh_token.map(str::to_owned),
        scope: None,
        issuer_account: None,
    }
}

fn form_value<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    body.split('&').find_map(|pair| {
        pair.split_once('=')
            .and_then(|(key, value)| (key == name).then_some(value))
    })
    .map(|value| value.split(' ').next().unwrap_or(value))
}

const SCHEME: &str = "service";
const PUBLIC: &str = "public";
const AUTHORIZE_URL: &str = "https://auth.oauth-runtime.test/authorize";

#[test]
fn acquire_then_cache_hit_and_single_flight_issue_one_request() {
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    let first = block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    assert_eq!(first.access_token, "at-1");
    assert_eq!(first.token_type, "Bearer");
    assert_eq!(first.refresh_token.as_deref(), Some("r1"));
    assert_eq!(first.scope.as_deref(), Some("read"));
    assert_eq!(
        first.issuer_account.as_deref(),
        Some("https://auth.oauth-runtime.test")
    );
    assert!(first.expires_at > std::time::SystemTime::now());
    assert_eq!(block_on(store.load(SCHEME)).unwrap().access_token, "at-1");

    // The cached set is returned without another request.
    let second = block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    assert_eq!(second.access_token, "at-1");
    assert_eq!(calls.lock().unwrap().len(), 1, "{calls:?}");
    assert!(
        calls.lock().unwrap()[0].starts_with(
            "POST https://auth.oauth-runtime.test/token body=grant_type=client_credentials auth=Some(\"Basic "
        ),
        "{calls:?}"
    );

    // Single-flight: empty the store, hold the first round's request in the
    // transport, start a second caller while it is in flight, and release.
    // The loser must wait on the completed round and re-read the store,
    // never re-request.
    block_on(store.clear(SCHEME));
    {
        let (held, _) = &*server.hold;
        *held.lock().unwrap() = true;
    }
    let results = std::thread::scope(|scope| {
        let first_caller =
            scope.spawn(|| block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap());
        wait_for_token_request(&server);
        let second_caller =
            scope.spawn(|| block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap());
        // Give the second caller time to enter the in-flight round before
        // releasing the held request.
        std::thread::sleep(std::time::Duration::from_millis(50));
        release(&server);
        [
            first_caller.join().unwrap().access_token,
            second_caller.join().unwrap().access_token,
        ]
    });
    assert!(results.iter().all(|token| token == "at-1"));
    let log = calls.lock().unwrap();
    assert_eq!(log.len(), 2, "single-flight must not re-request: {log:?}");
    assert!(
        log[1].starts_with("POST https://auth.oauth-runtime.test/token"),
        "{log:?}"
    );
}

#[test]
fn expired_sets_are_reacquired_and_replace_the_store() {
    let (server, calls) = Server::new();
    *server.token_mode.lock().unwrap() = "expire";
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    let first = block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    assert_eq!(first.access_token, "at-expired");
    let second = block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    assert_eq!(second.access_token, "at-expired");
    assert_eq!(
        calls.lock().unwrap().len(),
        2,
        "an immediately expired set must re-acquire"
    );
    assert_eq!(
        block_on(store.load(SCHEME)).unwrap().access_token,
        "at-expired"
    );
}

#[test]
fn refresh_adopts_rotated_tokens_and_retains_absent_ones() {
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();

    // Rotation: the server's new refresh token replaces the stored one.
    let refreshed =
        block_on(sessions.refresh_token(&client, SCHEME, &token_set("at-1", Some("r1"))))
            .unwrap();
    assert_eq!(refreshed.access_token, "at-2");
    assert_eq!(refreshed.refresh_token.as_deref(), Some("r2"));
    assert_eq!(
        block_on(store.load(SCHEME))
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("r2")
    );
    let log = calls.lock().unwrap();
    let refresh_call = log.last().unwrap();
    assert!(refresh_call.starts_with(&format!("POST https://auth.oauth-runtime.test/token-refresh")));
    assert!(refresh_call.contains("grant_type=refresh_token"));
    assert_eq!(
        form_value(refresh_call.split_once("body=").unwrap().1, "refresh_token"),
        Some("r1")
    );
    assert!(refresh_call.contains("auth=Some(\"Basic "), "{refresh_call}");
    drop(log);

    // Retention: a response without a refresh token keeps the previous one.
    *server.refresh_mode.lock().unwrap() = "retain";
    let retained = block_on(sessions.refresh_token(
        &client,
        SCHEME,
        &token_set("at-2", Some("keep-me")),
    ))
    .unwrap();
    assert_eq!(retained.access_token, "at-refreshed");
    assert_eq!(retained.refresh_token.as_deref(), Some("keep-me"));
}

#[test]
fn wrong_credentials_fail_with_typed_metadata_without_leaking_the_secret() {
    const SENTINEL: &str = "sdk-oauth-secret-sentinel";
    let (server, calls) = Server::new();
    *server.token_mode.lock().unwrap() = "reject";
    let store = Arc::new(MemoryTokenStore::new());
    let sessions =
        TokenSessions::with_store(store).with_client_credentials("profile-client", SENTINEL);
    let client = sdk::Client::with_transport(server.clone(), sdk::Credentials::new());
    let failure = block_on(sessions.client_credentials_token(&client, SCHEME))
        .err()
        .expect("wrong credentials must fail");
    assert_eq!(failure.kind, sdk::http::SdkErrorKind::UnexpectedResponse);
    assert_eq!(failure.status, Some(400));
    let auth = failure
        .cause
        .as_ref()
        .and_then(|cause| cause.downcast_ref::<AuthError>())
        .expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::ServerRejected);
    assert_eq!(auth.flow, sdk::oauth::AuthFlow::ClientCredentials);
    assert_eq!(auth.scheme, SCHEME);
    assert_eq!(auth.server_error.as_deref(), Some("invalid_client"));
    assert_eq!(
        auth.server_description.as_deref(),
        Some("client authentication failed")
    );
    // The secret travels to the server as the basic header, and never into
    // the error's display, debug or capture.
    let log = calls.lock().unwrap();
    assert!(
        log[0].contains("auth=Some(\"Basic "),
        "the basic header must reach the transport: {log:?}"
    );
    drop(log);
    let rendered = format!("{}{failure:?}", failure);
    assert!(
        !rendered.contains(SENTINEL),
        "the client secret leaked into the error: {rendered}"
    );
    assert!(
        !String::from_utf8_lossy(&failure.raw_capture).contains(SENTINEL),
        "the client secret leaked into the capture"
    );
}

#[test]
fn revocation_and_introspection_post_to_the_compiled_endpoints() {
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    block_on(sessions.revoke_token(&client, SCHEME, &token_set("at-1", None))).unwrap();
    let log = calls.lock().unwrap();
    let revoke = log.last().unwrap();
    assert!(revoke.starts_with(&format!("POST https://auth.oauth-runtime.test/revoke")));
    assert!(revoke.contains("token=at-1"));
    drop(log);
    assert!(
        block_on(store.load(SCHEME)).is_none(),
        "revocation clears the stored set"
    );

    // The public scheme has no revocation endpoint compiled: typed refusal.
    let failure = block_on(sessions.revoke_token(&client, PUBLIC, &token_set("at-1", None)))
        .err()
        .unwrap();
    let auth = failure
        .cause
        .as_ref()
        .and_then(|cause| cause.downcast_ref::<AuthError>())
        .expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::Unavailable);

    block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    let introspection =
        block_on(sessions.introspect_token(&client, SCHEME, &token_set("at-1", None))).unwrap();
    let active = match &introspection {
        sdk::Nullable::Value(sdk::JsonNonNullValue::Object(map)) => matches!(
            map.get("active"),
            Some(sdk::Nullable::Value(sdk::JsonNonNullValue::Bool(true)))
        ),
        _ => false,
    };
    assert!(active, "unexpected introspection document: {introspection:?}");
    assert!(
        calls.lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with(&format!("POST https://auth.oauth-runtime.test/introspect"))
    );
}

#[test]
fn public_clients_send_the_client_id_without_a_secret_header() {
    let (server, calls) = Server::new();
    let (client, sessions, _store) = client_and_explicit_sessions(&server);
    let token = block_on(sessions.client_credentials_token(&client, PUBLIC)).unwrap();
    assert_eq!(token.access_token, "at-1");
    let log = calls.lock().unwrap();
    assert_eq!(
        log[0],
        "POST https://auth.oauth-runtime.test/token body=grant_type=client_credentials&client_id=profile-client auth=None"
    );
}

#[test]
fn device_grant_polls_pending_then_completes_and_stores() {
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    let transaction = block_on(sessions.begin_device_authorization(&client, SCHEME)).unwrap();
    assert_eq!(transaction.user_code, "ABCD-EFGH");
    assert_eq!(
        transaction.verification_uri,
        "https://auth.oauth-runtime.test/activate"
    );
    assert_eq!(transaction.interval_seconds, 5);
    assert!(
        calls.lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with(&format!("POST https://auth.oauth-runtime.test/device"))
    );

    // The user has not approved yet.
    *server.device_mode.lock().unwrap() = "pending";
    match block_on(sessions.poll_device_token(&client, SCHEME, &transaction)).unwrap() {
        DevicePoll::Pending {
            retry_after_seconds,
        } => assert_eq!(retry_after_seconds, 5),
        other => panic!("unexpected poll result: {other:?}"),
    }
    // slow_down lengthens the retry interval by the RFC's five seconds.
    *server.device_mode.lock().unwrap() = "slow-down";
    match block_on(sessions.poll_device_token(&client, SCHEME, &transaction)).unwrap() {
        DevicePoll::Pending {
            retry_after_seconds,
        } => assert_eq!(retry_after_seconds, 10),
        other => panic!("unexpected poll result: {other:?}"),
    }
    // Approval completes the grant and stores the token.
    *server.device_mode.lock().unwrap() = "success";
    match block_on(sessions.poll_device_token(&client, SCHEME, &transaction)).unwrap() {
        DevicePoll::Complete(set) => {
            assert_eq!(set.access_token, "at-1");
            assert_eq!(set.refresh_token.as_deref(), Some("r1"));
        }
        other => panic!("unexpected poll result: {other:?}"),
    }
    assert_eq!(block_on(store.load(SCHEME)).unwrap().access_token, "at-1");

    // Expiry is reported instead of looping.
    *server.device_mode.lock().unwrap() = "expired-token";
    assert_eq!(
        block_on(sessions.poll_device_token(&client, SCHEME, &transaction)).unwrap(),
        DevicePoll::Expired
    );
}

#[test]
fn device_polling_loop_paces_slows_down_cumulatively_and_expires() {
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);
    let transaction = block_on(sessions.begin_device_authorization(&client, SCHEME)).unwrap();

    // pending, pending, slow_down, then approval: the first polls wait the
    // transaction interval, and slow_down grows the interval by five seconds
    // for every subsequent attempt.
    *server.device_mode.lock().unwrap() = "unused";
    *server.device_script.lock().unwrap() = vec!["pending", "pending", "slow-down", "success"];
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorded = waits.clone();
    let token = block_on(sessions.poll_device_token_until_complete(
        &client,
        SCHEME,
        &transaction,
        move |duration| recorded.lock().unwrap().push(duration.as_secs()),
    ))
    .unwrap();
    assert_eq!(token.access_token, "at-1");
    assert_eq!(token.refresh_token.as_deref(), Some("r1"));
    assert_eq!(*waits.lock().unwrap(), vec![5, 5, 10], "interval then +5s slow_down");
    assert_eq!(
        calls.lock().unwrap().iter().filter(|call| call.contains("/token")).count(),
        4,
        "four polls for pending, pending, slow_down and the approval"
    );
    assert_eq!(block_on(store.load(SCHEME)).unwrap().access_token, "at-1");

    // Slow-down pacing is cumulative: two slow_down answers stretch the
    // interval to 10 and then 15 seconds.
    block_on(store.clear(SCHEME));
    *server.device_script.lock().unwrap() = vec!["slow-down", "slow-down", "success"];
    waits.lock().unwrap().clear();
    let recorded = waits.clone();
    let token = block_on(sessions.poll_device_token_until_complete(
        &client,
        SCHEME,
        &transaction,
        move |duration| recorded.lock().unwrap().push(duration.as_secs()),
    ))
    .unwrap();
    assert_eq!(token.access_token, "at-1");
    assert_eq!(*waits.lock().unwrap(), vec![10, 15]);

    // The server's expired_token answer ends the loop with a typed failure
    // instead of returning.
    block_on(store.clear(SCHEME));
    *server.device_script.lock().unwrap() = vec!["expired-token"];
    let failure = block_on(sessions.poll_device_token_until_complete(
        &client,
        SCHEME,
        &transaction,
        |_| panic!("an expired poll must not be paced"),
    ))
    .err()
    .expect("expiry ends the loop");
    let auth = failure
        .cause
        .as_ref()
        .and_then(|cause| cause.downcast_ref::<AuthError>())
        .expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::Expired);
    assert_eq!(auth.flow, sdk::oauth::AuthFlow::DeviceToken);

    // A transaction whose declared lifetime has passed is refused before any
    // request or pacing happens.
    let mut expired = transaction.clone();
    expired.expires_at = std::time::SystemTime::now()
        - std::time::Duration::from_secs(1);
    let before = calls.lock().unwrap().len();
    let failure = block_on(sessions.poll_device_token_until_complete(
        &client,
        SCHEME,
        &expired,
        |_| panic!("an expired transaction must not poll"),
    ))
    .err()
    .unwrap();
    assert_eq!(
        failure
            .cause
            .and_then(|cause| cause.downcast_ref::<AuthError>().map(|auth| auth.kind)),
        Some(AuthErrorKind::Expired)
    );
    assert_eq!(calls.lock().unwrap().len(), before, "no poll fired after expiry");
}

#[test]
fn authorization_code_pkce_round_trip_binds_verifier_state_and_single_use() {
    const REDIRECT: &str = "https://app.oauth-runtime.test/callback";
    let (server, calls) = Server::new();
    let (client, sessions, store) = client_and_explicit_sessions(&server);

    // Begin: no network call, a bound verifier and one-time state.
    let transaction = sessions
        .begin_authorization(SCHEME, Some(REDIRECT), &["read", "write"])
        .unwrap();
    assert_eq!(transaction.scheme, SCHEME);
    assert_eq!(transaction.state.len(), 43, "32 bytes of entropy in base64url");
    assert_eq!(
        transaction.code_verifier.len(),
        43,
        "the verifier is 43 characters, inside the RFC 7636 range"
    );
    assert!(
        transaction
            .code_verifier
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~')),
        "the verifier uses only RFC 7636 unreserved characters"
    );
    assert_eq!(
        transaction.code_challenge,
        pkce_s256_challenge(&transaction.code_verifier),
        "the exposed derivation matches the bound challenge"
    );
    let url = &transaction.authorization_url;
    assert!(url.starts_with(&format!("{AUTHORIZE_URL}?")), "{url}");
    for expected in [
        "response_type=code",
        "client_id=profile-client",
        &format!(
            "redirect_uri={}",
            REDIRECT.replace(':', "%3A").replace('/', "%2F")
        ),
        &format!("state={}", transaction.state),
        &format!("code_challenge={}", transaction.code_challenge),
        "code_challenge_method=S256",
        // The query uses application/x-www-form-urlencoded spacing, matching
        // the other backends' authorization requests.
        "scope=read+write",
    ] {
        assert!(url.contains(expected), "the URL must carry {expected}: {url}");
    }
    assert!(!url.contains(&transaction.code_verifier), "the verifier never rides the URL");

    // Successive transactions draw fresh entropy.
    let second = sessions.begin_authorization(SCHEME, None, &[]).unwrap();
    assert_ne!(second.state, transaction.state);
    assert_ne!(second.code_verifier, transaction.code_verifier);
    assert!(!second.authorization_url.contains("redirect_uri="));
    assert!(!second.authorization_url.contains("scope="));
    assert!(second.authorization_url.contains("client_id=profile-client"));

    // Complete: the code is exchanged with the verifier on the wire and the
    // set is stored for the scheme.
    let set = block_on(sessions.complete_authorization(
        &client,
        &transaction,
        &[("state", transaction.state.as_str()), ("code", "auth-code-1")],
    ))
    .unwrap();
    assert_eq!(set.access_token, "at-1");
    let log = calls.lock().unwrap();
    let exchange = log.last().unwrap();
    assert!(
        exchange.starts_with("POST https://auth.oauth-runtime.test/token"),
        "{exchange}"
    );
    assert!(exchange.contains("grant_type=authorization_code"), "{exchange}");
    assert!(exchange.contains("code=auth-code-1"), "{exchange}");
    assert!(
        exchange.contains(&format!("code_verifier={}", transaction.code_verifier)),
        "the retained verifier travels to the exchange: {exchange}"
    );
    assert!(
        exchange.contains("redirect_uri=https%3A%2F%2Fapp.oauth-runtime.test%2Fcallback"),
        "the bound redirect URI is repeated exactly: {exchange}"
    );
    assert!(exchange.contains("auth=Some(\"Basic "), "{exchange}");
    drop(log);
    assert_eq!(block_on(store.load(SCHEME)).unwrap().access_token, "at-1");

    // Single use: a second completion is refused whatever the callback.
    let replay = block_on(sessions.complete_authorization(
        &client,
        &transaction,
        &[("state", transaction.state.as_str()), ("code", "auth-code-1")],
    ))
    .err()
    .unwrap();
    assert!(transaction.consumed());
    let auth = replay.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>()).unwrap();
    assert_eq!(auth.kind, AuthErrorKind::TransactionUsed);

    // A state mismatch is refused in constant time, and the first attempt
    // already consumed the transaction.
    let fresh = sessions.begin_authorization(SCHEME, None, &[]).unwrap();
    let forged = "X".repeat(43);
    let mismatch = block_on(sessions.complete_authorization(
        &client,
        &fresh,
        &[("state", forged.as_str()), ("code", "x")],
    ))
    .err()
    .unwrap();
    let auth = mismatch.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>()).unwrap();
    assert_eq!(auth.kind, AuthErrorKind::StateMismatch);
    assert_eq!(fresh.consumed(), true);
    let burned = block_on(sessions.complete_authorization(
        &client,
        &fresh,
        &[("state", fresh.state.as_str()), ("code", "x")],
    ))
    .err()
    .unwrap();
    assert_eq!(
        burned.cause.and_then(|cause| cause.downcast_ref::<AuthError>().map(|auth| auth.kind)),
        Some(AuthErrorKind::TransactionUsed)
    );

    // A server-declared error in the callback is a typed refusal carrying
    // the code and description, never a token.
    let denied = sessions.begin_authorization(SCHEME, None, &[]).unwrap();
    let failure = block_on(sessions.complete_authorization(
        &client,
        &denied,
        &[
            ("state", denied.state.as_str()),
            ("error", "access_denied"),
            ("error_description", "the user said no"),
        ],
    ))
    .err()
    .unwrap();
    let auth = failure.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>()).unwrap();
    assert_eq!(auth.kind, AuthErrorKind::AuthorizationDenied);
    assert_eq!(auth.server_error.as_deref(), Some("access_denied"));
    assert_eq!(auth.server_description.as_deref(), Some("the user said no"));

    // A callback without a code is a typed invalid callback.
    let missing = sessions.begin_authorization(SCHEME, None, &[]).unwrap();
    let failure = block_on(sessions.complete_authorization(
        &client,
        &missing,
        &[("state", missing.state.as_str())],
    ))
    .err()
    .unwrap();
    let auth = failure.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>()).unwrap();
    assert_eq!(auth.kind, AuthErrorKind::InvalidCallback);

    // An unknown scheme is refused before any entropy is drawn.
    let failure = sessions
        .begin_authorization("other", None, &[])
        .err()
        .unwrap();
    let auth = failure.cause.as_ref().and_then(|cause| cause.downcast_ref::<AuthError>()).unwrap();
    assert_eq!(auth.kind, AuthErrorKind::UnknownScheme);
}

#[test]
fn unknown_schemes_fail_typed() {
    let (server, _calls) = Server::new();
    let (client, sessions, _store) = client_and_explicit_sessions(&server);
    let unknown = block_on(sessions.client_credentials_token(&client, "other"))
        .err()
        .unwrap();
    let auth = unknown
        .cause
        .as_ref()
        .and_then(|cause| cause.downcast_ref::<AuthError>())
        .expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::UnknownScheme);
    assert_eq!(auth.scheme, "other");
}

/// The compiled environment variables are read at call time. Environment
/// mutation is confined to this single-threaded test; every other test uses
/// explicit credentials, so no concurrent test ever reads the environment.
#[test]
fn env_credentials_are_read_at_call_time() {
    let (server, calls) = Server::new();
    let store = Arc::new(MemoryTokenStore::new());
    let sessions = TokenSessions::with_store(store.clone());
    let client = sdk::Client::with_transport(server.clone(), sdk::Credentials::new());

    std::env::set_var("SDK_OAUTH_TEST_CLIENT_ID", "env-client");
    std::env::set_var("SDK_OAUTH_TEST_CLIENT_SECRET", "env-secret");
    std::env::set_var("SDK_OAUTH_TEST_PUBLIC_ID", "env-pub");
    let token = block_on(sessions.client_credentials_token(&client, SCHEME)).unwrap();
    assert_eq!(token.access_token, "at-1");
    let expected = format!(
        "auth={:?}",
        Some(format!("Basic {}", base64(b"env-client:env-secret")))
    );
    assert!(
        calls.lock().unwrap()[0].contains(&expected),
        "the basic header must come from the compiled variables: {expected} vs {calls:?}"
    );

    // Removing the secret fails with the variable named, never its value;
    // clear the cache so the check cannot be served a stored set.
    block_on(store.clear(SCHEME));
    std::env::remove_var("SDK_OAUTH_TEST_CLIENT_SECRET");
    let failure = block_on(sessions.client_credentials_token(&client, SCHEME))
        .err()
        .expect("the confidential scheme needs its secret");
    let auth = failure
        .cause
        .as_ref()
        .and_then(|cause| cause.downcast_ref::<AuthError>())
        .expect("typed AuthError cause");
    assert_eq!(auth.kind, AuthErrorKind::MissingClientCredentials);
    let message = format!("{failure}");
    assert!(
        message.contains("SDK_OAUTH_TEST_CLIENT_SECRET"),
        "the error names the variable: {message}"
    );

    // The public scheme reads only its client id and sends no secret header.
    let token = block_on(sessions.client_credentials_token(&client, PUBLIC)).unwrap();
    assert_eq!(token.access_token, "at-1");
    assert!(
        calls.lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("body=grant_type=client_credentials&client_id=env-pub auth=None")
    );
    std::env::remove_var("SDK_OAUTH_TEST_CLIENT_ID");
    std::env::remove_var("SDK_OAUTH_TEST_PUBLIC_ID");
}
"##;

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    json!({
        "openapi":"3.2.0","info":{"title":"OAuth replay","version":"1"},
        "servers":[{"url":"https://api.oauth-runtime.test/v1"}],
        "paths":{
            "/widgets":{"get":{"operationId":"listWidgets","security":[{"service":["read"]}],"responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}}},
            "/events":{"get":{"operationId":"streamEvents","security":[{"feed":["read"]}],"responses":{"200":{"description":"Events","content":{"application/x-ndjson":{"itemSchema":{"type":"string"}}}}}}}
        },
        "components":{"securitySchemes":{
            "service":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.oauth-runtime.test/token","scopes":{"read":"Read access"}}}},
            "feed":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.oauth-runtime.test/feed-token","scopes":{"read":"Read access"}}}}
        }}
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "service":{"client_id_env":"SDK_REPLAY_CLIENT_ID","client_secret_env":"SDK_REPLAY_CLIENT_SECRET"},
                    "feed":{"client_id_env":"SDK_REPLAY_FEED_ID","client_secret_env":"SDK_REPLAY_FEED_SECRET"}
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials grant, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate(replay_document(), &replay_options());
    let oauth = configured
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("OAuth runtime emitted");
    for expected in [
        "pub struct ReplayCredentials<S: TokenStore = MemoryTokenStore> {",
        "pub fn transport<T: Transport>(&self, inner: T) -> ReplayTransport<T, S>",
        "pub fn hook(&self) -> impl crate::http::CredentialProvider + 'static",
        "static REPLAY_NO_REPLAY: &[(&str, &[&str])] = &[(\"feed\", &[\"/paths/~1events/get/security/0/feed\"])];",
        "delivered stream data prevents a transparent",
    ] {
        assert!(
            oauth.content.contains(expected),
            "oauth.rs is missing:\n{expected}\n--- emitted: ---\n{}",
            oauth.content
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.content.contains("~1widgets/get/security/0/service"));
    // The authorization-code-only control compiles exactly the pre-replay
    // bytes: no wrapper, no stream-protection table.
    let code_only = generate(code_only_document(), &code_only_options());
    let plain_oauth = code_only
        .iter()
        .find(|file| file.path == "rust/src/oauth.rs")
        .expect("OAuth runtime emitted");
    assert!(!plain_oauth.content.contains("ReplayCredentials"));
    assert!(!plain_oauth.content.contains("REPLAY_NO_REPLAY"));
}

fn code_only_document() -> Value {
    json!({
        "openapi":"3.2.0","info":{"title":"OAuth code only","version":"1"},
        "servers":[{"url":"https://api.oauth-runtime.test/v1"}],
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","security":[{"userOAuth":["read"]}],"responses":{"200":{"description":"Ok"}}}}},
        "components":{"securitySchemes":{"userOAuth":{"type":"oauth2","flows":{"authorizationCode":{
            "authorizationUrl":"https://auth.oauth-runtime.test/authorize",
            "tokenUrl":"https://auth.oauth-runtime.test/token",
            "scopes":{"read":"Read access"}
        }}}}}
    })
}

fn code_only_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1",
                "oauth":{"schemes":{"userOAuth":{
                    "client_id_env":"SDK_REPLAY_CLIENT_ID",
                    "client_secret_env":"SDK_REPLAY_CLIENT_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

const REPLAY_CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the replaying credential wrapper.
#![allow(dead_code)]
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use sdk::oauth::{ReplayCredentials, TokenSessions};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

fn response(status: u16, body: &str) -> TransportResponse<Body> {
    TransportResponse {
        status,
        headers: vec![("Content-Type".into(), b"application/json".to_vec())],
        body: Body(Some(body.as_bytes().to_vec())),
    }
}

#[derive(Default)]
struct State {
    svc_tokens: u32,
    feed_tokens: u32,
    mode: &'static str,
    stale_next: bool,
    stale_token: String,
    fail_from: u32,
}

#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    state: Arc<Mutex<State>>,
}

impl Server {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(State {
                fail_from: u32::MAX,
                ..State::default()
            })),
        }
    }
    fn arm_stale(&self) {
        self.state.lock().unwrap().stale_next = true;
    }
    fn always_401(&self) {
        self.state.lock().unwrap().mode = "always401";
    }
    fn count(&self, needle: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains(needle))
            .count()
    }
    fn values(&self, needle: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains(needle))
            .cloned()
            .collect()
    }
}

impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| String::from_utf8_lossy(value).to_string())
            .unwrap_or_default();
        self.calls
            .lock()
            .unwrap()
            .push(format!("{} {} {}", request.method, request.url, authorization));
        let mut state = self.state.lock().unwrap();
        match request.url.as_str() {
            "https://auth.oauth-runtime.test/token" => {
                state.svc_tokens += 1;
                let token = format!("svc-{}", state.svc_tokens);
                if state.stale_next {
                    state.stale_next = false;
                    state.stale_token = token.clone();
                }
                let fail = state.svc_tokens >= state.fail_from;
                drop(state);
                if fail {
                    return Ok(response(500, r#"{"error":"server_error"}"#));
                }
                Ok(response(
                    200,
                    &format!(r#"{{"access_token":"{token}","token_type":"Bearer","expires_in":3600}}"#),
                ))
            }
            "https://auth.oauth-runtime.test/feed-token" => {
                state.feed_tokens += 1;
                let token = format!("feed-{}", state.feed_tokens);
                drop(state);
                Ok(response(
                    200,
                    &format!(r#"{{"access_token":"{token}","token_type":"Bearer","expires_in":3600}}"#),
                ))
            }
            "https://api.oauth-runtime.test/v1/widgets" => {
                let stale = state.stale_token.clone();
                let mode = state.mode;
                drop(state);
                if mode == "always401" || (!stale.is_empty() && authorization == format!("Bearer {stale}")) {
                    return Ok(response(401, r#"{"error":"stale"}"#));
                }
                Ok(response(200, r#""ok""#))
            }
            "https://api.oauth-runtime.test/v1/events" => {
                drop(state);
                Ok(response(401, r#"{"error":"stream-denied"}"#))
            }
            other => panic!("unexpected request {other}"),
        }
    }
}

// (a) 401 then success: one refresh, one replay, and the caller sees 200.
#[test]
fn refreshes_once_and_replays_once() {
    let server = Server::new();
    server.arm_stale();
    let replay = ReplayCredentials::new().with_client_credentials("consumer-client", "consumer-secret");
    let client = sdk::Client::with_transport(
        replay.transport(server.clone()),
        sdk::Credentials::new().with_hook(replay.hook()),
    );
    block_on(replay.token(&client, "service")).unwrap();
    block_on(client.list_widgets_default()).expect("the replayed call succeeds");
    let calls = server.values("/v1/widgets");
    assert_eq!(calls.len(), 2, "exactly one replay");
    assert_eq!(server.count("/token"), 2, "exactly one refresh");
    assert_ne!(calls[0], calls[1], "the replay carried the fresh token");
}

// (b) 401 then 401: the second 401 surfaces and exactly one refresh ran.
#[test]
fn surfaces_the_second_401() {
    let server = Server::new();
    server.always_401();
    let replay = ReplayCredentials::new().with_client_credentials("consumer-client", "consumer-secret");
    let client = sdk::Client::with_transport(
        replay.transport(server.clone()),
        sdk::Credentials::new().with_hook(replay.hook()),
    );
    block_on(replay.token(&client, "service")).unwrap();
    let error = block_on(client.list_widgets_default()).expect_err("the second 401 surfaces");
    match error {
        sdk::operations::list_widgets::ListWidgetsError::Sdk(failure) => {
            assert_eq!(failure.status, Some(401));
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(server.count("/v1/widgets"), 2, "one replay, no loops");
    assert_eq!(server.count("/token"), 2, "exactly one refresh");
}

// (c) concurrent 401s across two threads: ONE refresh, two replays.
#[test]
fn concurrent_401s_share_one_refresh() {
    let server = Server::new();
    server.arm_stale();
    let replay = Arc::new(ReplayCredentials::new().with_client_credentials("consumer-client", "consumer-secret"));
    let client = Arc::new(sdk::Client::with_transport(
        replay.transport(server.clone()),
        sdk::Credentials::new().with_hook(replay.hook()),
    ));
    block_on(replay.token(&client, "service")).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let client = Arc::clone(&client);
            scope.spawn(move || {
                block_on(client.list_widgets_default()).expect("the replayed call succeeds");
            });
        }
    });
    assert_eq!(server.count("/v1/widgets"), 4, "two replays");
    assert_eq!(server.count("/token"), 2, "one shared refresh");
    let calls = server.values("/v1/widgets");
    let stale = format!("Bearer {}", server.state.lock().unwrap().stale_token);
    assert_eq!(
        calls.iter().filter(|call| call.ends_with(&stale)).count(),
        2,
        "both original requests carried the stale token"
    );
}

// (d) a streaming operation is never replayed: the typed 401 surfaces.
#[test]
fn streaming_operations_are_never_replayed() {
    let server = Server::new();
    let replay = ReplayCredentials::new().with_client_credentials("consumer-client", "consumer-secret");
    let client = sdk::Client::with_transport(
        replay.transport(server.clone()),
        sdk::Credentials::new().with_hook(replay.hook()),
    );
    block_on(replay.token(&client, "feed")).unwrap();
    let error = block_on(client.stream_events_default()).expect_err("the stream 401 surfaces");
    match error {
        sdk::operations::stream_events::StreamEventsError::Sdk(failure) => {
            assert_eq!(failure.status, Some(401));
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(server.count("/v1/events"), 1, "no replay for the streaming operation");
    assert_eq!(server.count("/feed-token"), 1, "no refresh for the streaming operation");
}

// (e) replay disabled by default: the plain lifecycle surfaces the 401
// without any refresh.
#[test]
fn replay_disabled_by_default() {
    let server = Server::new();
    server.arm_stale();
    let sessions = TokenSessions::new().with_client_credentials("consumer-client", "consumer-secret");
    let warm = sdk::Client::with_transport(server.clone(), sdk::Credentials::new());
    let set = block_on(sessions.client_credentials_token(&warm, "service")).unwrap();
    let client = sdk::Client::with_transport(
        server.clone(),
        sdk::Credentials::new().with_authorization("service", format!("Bearer {}", set.access_token)),
    );
    let error = block_on(client.list_widgets_default()).expect_err("the 401 surfaces");
    match error {
        sdk::operations::list_widgets::ListWidgetsError::Sdk(failure) => {
            assert_eq!(failure.status, Some(401));
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(server.count("/v1/widgets"), 1, "no replay");
    assert_eq!(server.count("/token"), 1, "no refresh");
}

// (f) refresh failure: the typed auth failure surfaces instead of a replay.
#[test]
fn refresh_failure_is_typed_and_never_replays() {
    let server = Server::new();
    server.arm_stale();
    server.state.lock().unwrap().fail_from = 2;
    let replay = ReplayCredentials::new().with_client_credentials("consumer-client", "consumer-secret");
    let client = sdk::Client::with_transport(
        replay.transport(server.clone()),
        sdk::Credentials::new().with_hook(replay.hook()),
    );
    block_on(replay.token(&client, "service")).unwrap();
    let error = block_on(client.list_widgets_default()).expect_err("the refresh failure surfaces");
    match error {
        sdk::operations::list_widgets::ListWidgetsError::Sdk(failure) => {
            assert_eq!(failure.kind, sdk::http::SdkErrorKind::Transport);
            // The wrapper's typed rejection rides as the transport failure's
            // boxed SdkError cause, with the AuthError nested inside it.
            let cause = failure.cause.as_ref().expect("the transport failure carries a cause");
            let inner = cause
                .downcast_ref::<sdk::http::SdkError>()
                .expect("the wrapper's typed rejection");
            let auth = inner
                .cause
                .as_ref()
                .and_then(|cause| cause.downcast_ref::<sdk::oauth::AuthError>())
                .expect("the typed auth failure is the cause");
            assert_eq!(auth.kind, sdk::oauth::AuthErrorKind::ServerRejected);
            assert_eq!(auth.server_error.as_deref(), Some("server_error"));
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(server.count("/v1/widgets"), 1, "no replay after a failed refresh");
    assert_eq!(server.count("/token"), 2, "the refresh was attempted exactly once");
}
"##;

#[test]
fn replay_lifecycle_drives_the_wrapper_in_a_compiled_package() {
    let configured = generate(replay_document(), &replay_options());
    let directory = tempfile::Builder::new()
        .prefix("rust-replay-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");

    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"oauth-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"oauth-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), REPLAY_CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping behavioral replay execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_OAUTH").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}
