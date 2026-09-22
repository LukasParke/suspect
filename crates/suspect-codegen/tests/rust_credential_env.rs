//! Explicit credential environment policy through real Rust plans and installed clients.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::rust_http::{self, HttpConfig, HttpPlan, PackageConfig};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.example/credential-env.json";
fn document() -> Value {
    let mut paths = serde_json::Map::new();
    for (path, id, security) in [
        ("/key", "getCurrentKey", json!([{"apiKey":[]}])),
        ("/either", "either", json!([{"apiKey":[]},{"keyHeader":[]}])),
        ("/and", "both", json!([{"apiKey":[],"keyHeader":[]}])),
        ("/anonymous", "anonymous", json!([])),
        ("/optional", "optional", json!([{"apiKey":[]},{}])),
        ("/query", "queryKey", json!([{"keyQuery":[]}])),
        ("/cookie", "cookieKey", json!([{"keyCookie":[]}])),
        ("/from-env", "fromEnv", json!([{"fromEnv":[]}])),
        (
            "/transport-env",
            "withTransportFromEnv",
            json!([{"keyHeader":[]}]),
        ),
    ] {
        paths.insert(path.into(),json!({"get":{"operationId":id,"security":security,"responses":{"204":{"description":"Accepted"}}}}));
    }
    json!({"openapi":"3.2.0","info":{"title":"Credential env witness","version":"1"},"servers":[{"url":"https://api.example.test/v1"}],"paths":paths,"components":{"securitySchemes":{
        "apiKey":{"type":"http","scheme":"bearer"},"keyHeader":{"type":"apiKey","in":"header","name":"X-Key"},
        "keyQuery":{"type":"apiKey","in":"query","name":"key"},"keyCookie":{"type":"apiKey","in":"cookie","name":"sid"},
        "fromEnv":{"type":"http","scheme":"bearer"}
    }}})
}
fn contract(value: Value) -> Arc<Contract> {
    let uri = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
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
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}
fn selected(c: &Contract) -> Vec<SourceId> {
    c.operations().map(|op| op.source().clone()).collect()
}
fn plan(config: HttpConfig) -> HttpPlan {
    let c = contract(document());
    rust_http::plan_http_v3(c.clone(), &selected(&c), config).unwrap()
}
fn files(plan: &HttpPlan, name: &str) -> Vec<suspect_codegen::OutFile> {
    rust_http::emit_http(
        plan,
        &PackageConfig {
            name: name.into(),
            version: "0.1.0".into(),
        },
    )
    .unwrap()
}

#[test]
fn rust_no_policy_artifacts_match_pre_env_checkpoint() {
    let files = files(&plan(Default::default()), "credential-env-sdk");
    let hashes = files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                format!("{:x}", Sha256::digest(file.content.as_bytes())),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected: BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/rust-credential-env-no-policy.json")).unwrap();
    assert_eq!(
        hashes, expected,
        "unconfigured output must match the pre-env artifact checkpoint"
    );
}

fn policy() -> suspect_codegen::credential_env::CredentialEnv {
    suspect_codegen::credential_env::CredentialEnv::v1(
        [
            ("apiKey", "SDK_TEST_BEARER"),
            ("fromEnv", "SDK_TEST_BEARER"),
            ("keyHeader", "SDK_TEST_HEADER"),
            ("keyQuery", "SDK_TEST_QUERY"),
            ("keyCookie", "SDK_TEST_COOKIE"),
        ]
        .into_iter()
        .map(|(name, variable)| (name.into(), variable.into()))
        .collect(),
    )
}

#[test]
fn rust_credential_env_is_source_bound_and_reserves_only_configured_helpers() {
    use suspect_codegen::credential_env::CredentialEnvKind;
    let base = plan(Default::default());
    let configured = plan(HttpConfig {
        credential_env: Some(policy()),
        ..Default::default()
    });
    let bound = configured.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 5);
    let bearer = bound
        .bindings()
        .iter()
        .find(|b| b.name() == "apiKey")
        .unwrap();
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(
        bearer.scheme().use_site().source().document().as_str(),
        ENTRY
    );
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert!(!bearer.scheme().use_site().span().is_empty());
    let semantic = serde_json::to_string(&bound.semantic_descriptor()).unwrap();
    assert!(!semantic.contains(ENTRY));
    assert!(!semantic.contains("source"));
    for (id, name) in [
        ("fromEnv", "from_env"),
        ("withTransportFromEnv", "with_transport_from_env"),
    ] {
        assert_eq!(
            base.operations()
                .iter()
                .find(|op| op.operation_id == id)
                .unwrap()
                .function_name,
            name
        );
        assert_ne!(
            configured
                .operations()
                .iter()
                .find(|op| op.operation_id == id)
                .unwrap()
                .function_name,
            name
        );
    }
    assert_eq!(
        base.credentials()
            .values()
            .find(|c| c.requirement.name() == "fromEnv")
            .unwrap()
            .constructor,
        "from_env"
    );
    assert_ne!(
        configured
            .credentials()
            .values()
            .find(|c| c.requirement.name() == "fromEnv")
            .unwrap()
            .constructor,
        "from_env"
    );
    let files = files(&configured, "credential-env-sdk");
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "rust/http-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        manifest["credentialEnv"]["bindings"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(files.iter().any(|f| f.path == "rust/src/credential_env.rs"));
    for kind in [
        json!({"type":"http","scheme":"basic"}),
        json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.example.test/token","scopes":{}}}}),
        json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.example.test/discovery"}),
    ] {
        let mut value = document();
        value["components"]["securitySchemes"]["apiKey"] = kind;
        let c = contract(value);
        let error = rust_http::plan_http_v3(
            c.clone(),
            &selected(&c),
            HttpConfig {
                credential_env: Some(policy()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.iter().any(|d| d.code == "sdk-credential-env-kind"
                && d.source.document().as_str() == ENTRY
                && d.source.pointer() == "/components/securitySchemes/apiKey"
                && !d.at.is_empty()),
            "{error:?}"
        );
    }
}

#[test]
fn rust_credential_env_generation_does_not_capture_generator_values() {
    const CANARY: &str = "generator-secret-must-never-enter-artifacts";
    if std::env::var_os("SUSPECT_RUST_ENV_CANARY_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "rust_credential_env_generation_does_not_capture_generator_values",
                "--nocapture",
            ])
            .env("SUSPECT_RUST_ENV_CANARY_CHILD", "1")
            .env("SDK_TEST_BEARER", CANARY)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let plan = plan(HttpConfig {
        credential_env: Some(policy()),
        ..Default::default()
    });
    for file in files(&plan, "credential-env-sdk") {
        assert!(
            !file.content.contains(CANARY),
            "generator value leaked into {}",
            file.path
        );
    }
    assert!(
        !serde_json::to_string(plan.credential_env().unwrap())
            .unwrap()
            .contains(CANARY)
    );
}

fn target() -> PathBuf {
    std::env::var_os("SUSPECT_RUST_CREDENTIAL_ENV_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-credential-env-floor"
                } else {
                    "../../target/native-rust-credential-env"
                },
            )
        })
}
fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut c = Command::new("cargo");
    c.args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings")
        .env_remove("RUST_MIN_STACK");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        c.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    c
}
fn checked(command: &mut Command, root: &Path) {
    use std::io::Write;
    let output = command.output().unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        log,
        "{command:?}\nstatus: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "retained {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn installed(plan: &HttpPlan, name: &str, consumer_source: &str) -> (PathBuf, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("rust-credential-env-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&files(plan, name), &directory).unwrap();
    let target = target();
    let manifest = directory.join("rust/Cargo.toml");
    checked(
        cargo("test", &manifest, &target).args(["--doc", "--all-features"]),
        &directory,
    );
    checked(
        cargo("check", &manifest, &target).arg("--no-default-features"),
        &directory,
    );
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &directory,
    );
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join(format!("package/{name}-0.1.0.crate"));
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &directory,
    );
    // Environment mutation is confined to a standalone, single-threaded 2021
    // consumer before constructing any executor/reqwest client. SDK code is 2024
    // and contains no unsafe code or environment mutation.
    std::fs::write(consumer.join("Cargo.toml"),format!("[package]\nname=\"credential-env-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={{package={name:?},path=\"vendor/{name}-0.1.0\",features=[\"reqwest-rustls\"]}}\ntokio={{version=\"=1.53.1\",features=[\"rt\"]}}\n")).unwrap();
    std::fs::write(consumer.join("src/main.rs"), consumer_source).unwrap();
    std::fs::write(
        consumer.join("src/lib.rs"),
        r#"
/// Rust explicit credentials cannot be null or an omitted member sentinel.
/// ```compile_fail,E0308
/// let _ = sdk::Client::with_transport((), None::<sdk::Credentials>);
/// ```
/// ```compile_fail,E0308
/// let _ = sdk::Client::with_reqwest(None::<sdk::Credentials>);
/// ```
pub struct ExplicitNonNullCredentials;
"#,
    )
    .unwrap();
    let digest = format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()));
    let consumer_target = target.join("installed").join(digest);
    checked(
        &mut cargo("build", &consumer.join("Cargo.toml"), &consumer_target),
        &directory,
    );
    checked(
        cargo("test", &consumer.join("Cargo.toml"), &consumer_target).arg("--doc"),
        &directory,
    );
    (
        directory,
        consumer_target.join("debug/credential-env-consumer"),
    )
}

#[test]
#[ignore = "requires native Rust 1.88/stable and pinned Cargo dependencies"]
fn installed_rust_credential_env_snapshots_and_explicit_credentials_obey_source_auth() {
    let p = plan(HttpConfig {
        credential_env: Some(policy()),
        ..Default::default()
    });
    let mut text = include_str!("fixtures/rust-credential-env-consumer.rs").to_owned();
    for (key, value) in [
        (
            "__FROM_ENV_OP__",
            p.operations()
                .iter()
                .find(|op| op.operation_id == "fromEnv")
                .unwrap()
                .default_function_name
                .as_ref()
                .unwrap(),
        ),
        (
            "__TRANSPORT_ENV_OP__",
            p.operations()
                .iter()
                .find(|op| op.operation_id == "withTransportFromEnv")
                .unwrap()
                .default_function_name
                .as_ref()
                .unwrap(),
        ),
        (
            "__FROM_ENV_CTOR__",
            &p.credentials()
                .values()
                .find(|c| c.requirement.name() == "fromEnv")
                .unwrap()
                .constructor,
        ),
    ] {
        text = text.replace(key, value);
    }
    let (root, binary) = installed(&p, "credential-env-sdk", &text);
    for mode in [
        "positive",
        "missing",
        "empty",
        "alternate",
        "snapshot",
        "invalid",
    ] {
        let mut c = Command::new(&binary);
        c.arg(mode);
        for key in [
            "SDK_TEST_BEARER",
            "SDK_TEST_HEADER",
            "SDK_TEST_QUERY",
            "SDK_TEST_COOKIE",
        ] {
            c.env_remove(key);
        }
        if matches!(mode, "positive" | "snapshot") {
            c.env("SDK_TEST_BEARER", "early-token")
                .env("SDK_TEST_HEADER", "header-key")
                .env("SDK_TEST_QUERY", "query &+")
                .env("SDK_TEST_COOKIE", "cookie-key");
        }
        if mode == "empty" {
            c.env("SDK_TEST_BEARER", "").env("SDK_TEST_HEADER", "");
        }
        if mode == "alternate" {
            c.env("SDK_TEST_BEARER", "")
                .env("SDK_TEST_HEADER", "header-key");
        }
        if mode == "invalid" {
            c.env("SDK_TEST_BEARER", "secret\ninvalid");
        }
        checked(&mut c, &root);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut c = Command::new(&binary);
        c.arg("unavailable").env(
            "SDK_TEST_BEARER",
            std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
        );
        for key in ["SDK_TEST_HEADER", "SDK_TEST_QUERY", "SDK_TEST_COOKIE"] {
            c.env_remove(key);
        }
        checked(&mut c, &root);
    }
    eprintln!("native credential-env controls retained {}", root.display());
}

#[test]
#[ignore = "requires native Rust 1.88/stable, tracked OpenRouter source and pinned Cargo dependencies"]
fn installed_rust_openrouter_env_current_key_uses_source_default_https() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = root.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let c =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selection = c
        .operations()
        .filter(|op| matches!(op.operation_id(), Some("getCurrentKey" | "getCredits")))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selection.len(), 2);
    let policy = suspect_codegen::credential_env::CredentialEnv::v1(BTreeMap::from([(
        "apiKey".into(),
        "OPENROUTER_API_KEY".into(),
    )]));
    let p = rust_http::plan_http_v3(
        c.clone(),
        &selection,
        HttpConfig {
            credential_env: Some(policy),
            ..Default::default()
        },
    )
    .unwrap();
    let binding = &p.credential_env().unwrap().bindings()[0];
    assert_eq!(
        binding.kind(),
        suspect_codegen::credential_env::CredentialEnvKind::Bearer
    );
    assert_eq!(
        binding.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    let response = |id: &str| {
        let op = p
            .operations()
            .iter()
            .find(|op| op.operation_id == id)
            .unwrap();
        assert_eq!(
            op.wire().servers().candidates()[0].template(),
            "https://openrouter.ai/api/v1"
        );
        let response = op
            .responses()
            .iter()
            .find(|response| response.wire().status_key() == "200")
            .unwrap();
        let media = response.wire().media()[0].source().terminal().source();
        serde_json::to_string(c.source(&media.child("example")).unwrap()).unwrap()
    };
    let constants = format!(
        "const CURRENT:&str={:?};\nconst CREDITS:&str={:?};\n",
        response("getCurrentKey"),
        response("getCredits")
    );
    let (directory, binary) = installed(
        &p,
        "openrouter",
        &format!("{constants}\n{OPENROUTER_CONSUMER}"),
    );
    checked(
        Command::new(&binary)
            .arg("positive")
            .env("OPENROUTER_API_KEY", "controlled-openrouter-token"),
        &directory,
    );
    checked(
        Command::new(&binary)
            .arg("missing")
            .env_remove("OPENROUTER_API_KEY"),
        &directory,
    );
    std::fs::write(
        directory.join("source-sha256.txt"),
        format!(
            "{:x}  {}\n",
            Sha256::digest(std::fs::read(&path).unwrap()),
            path.display()
        ),
    )
    .unwrap();
    eprintln!(
        "native credential-env OpenRouter retained {}",
        directory.display()
    );
}

const OPENROUTER_CONSUMER: &str = r#"
use sdk::{Client,Credentials,http::{BoxError,Request,ResponseBody,Transport,TransportResponse,SdkErrorKind}};
use std::sync::{Arc,Mutex};
struct Body(Option<Vec<u8>>);
impl ResponseBody for Body{async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError>{Ok(self.0.take())}}
#[derive(Clone,Default)]struct Controlled(Arc<Mutex<Vec<String>>>);
impl Transport for Controlled {
    type Body=Body;
    async fn send(&self,request:Request)->Result<TransportResponse<Body>,BoxError>{
        assert_eq!(request.method,"GET");assert_eq!(request.headers.iter().find(|(key,_)|key.eq_ignore_ascii_case("authorization")).unwrap().1,b"Bearer controlled-openrouter-token");
        let body=match request.url.as_str(){"https://openrouter.ai/api/v1/key"=>CURRENT,"https://openrouter.ai/api/v1/credits"=>CREDITS,_=>panic!("source-default URL changed")};
        self.0.lock().unwrap().push(request.url);Ok(TransportResponse{status:200,headers:vec![("Content-Type".into(),b"application/json".to_vec())],body:Body(Some(body.as_bytes().to_vec()))})
    }
}
fn main(){
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let transport=Controlled::default();let client=Client::with_transport_from_env(transport.clone());
        let _:Client<sdk::reqwest_transport::ReqwestTransport>=Client::from_env().unwrap();
        let _:Client<sdk::reqwest_transport::ReqwestTransport>=Client::with_reqwest(Credentials::api_key("explicit-token")).unwrap();
        if std::env::args().nth(1).as_deref()==Some("missing") {
            let sdk::operations::get_current_key::GetCurrentKeyError::Sdk(error)=client.get_current_key_default().await.unwrap_err()else{panic!("missing local credentials")};
            assert_eq!(error.kind,SdkErrorKind::RequestValidation);assert!(transport.0.lock().unwrap().is_empty());return;
        }
        let response=client.get_current_key_default().await.unwrap().into_response();
        assert_eq!(response.status,200);assert_eq!(response.data.data.label,"sk-or-v1-au7...890");assert!(!response.data.data.is_management_key);
        let response=client.get_credits_default().await.unwrap().into_response();assert_eq!(response.status,200);assert!(!response.data.data.total_credits.as_str().is_empty());
        assert_eq!(*transport.0.lock().unwrap(),vec!["https://openrouter.ai/api/v1/key","https://openrouter.ai/api/v1/credits"]);
    });
}
"#;
