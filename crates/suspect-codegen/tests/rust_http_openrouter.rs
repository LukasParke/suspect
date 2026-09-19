//! Installed native OpenRouter Rust consumer acceptance through the public
//! generator API (`plan_http`/`emit_http`), a real `cargo package` tarball and
//! an independent recording TCP server.

use sha2::{Digest, Sha256};
use std::{path::Path, process::Command, sync::Arc};

use suspect_codegen::rust_http::{
    HttpConfig, HttpPlan, PackageConfig, PlannedOperation, emit_http, plan_http,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const WANTED: [&str; 5] = [
    "getCredits",
    "createKeys",
    "updateKeys",
    "listContainerFiles",
    "getContainerFile",
];

#[test]
#[ignore = "requires native Cargo, the tracked OpenRouter checkout and pinned crates"]
fn installed_expanded_openrouter_bytes_anonymous_and_delete_operations() {
    use suspect_codegen::http_protocol::{CompatibilityProfile, SecurityPlan};
    let contract = contract();
    let wanted = [
        "downloadContainerFileContent",
        "downloadFileContent",
        "createCoinbaseCharge",
        "deleteFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let strict = plan_http(contract.clone(), &selected, HttpConfig::default()).unwrap_err();
    assert!(strict.iter().any(|d| d.code == "http-binary-legacy-marker"));
    let config = HttpConfig {
        compatibility_profiles: vec![CompatibilityProfile::LegacyBinaryStringV1],
        ..Default::default()
    };
    let plan = plan_http(contract.clone(), &selected, config.clone()).unwrap();
    assert!(matches!(
        operation(&plan, "createCoinbaseCharge").wire().security(),
        SecurityPlan::NoAuth { .. }
    ));
    for id in ["downloadContainerFileContent", "downloadFileContent"] {
        let op = operation(&plan, id);
        assert!(op.responses().iter().any(|r| r.wire().status_key() == "200"
            && matches!(
                r.variants()[0].payload,
                suspect_codegen::rust_http::Payload::Bytes
            )));
        assert!(plan.protocol().codec_roots().iter().all(|root| {
            !root
                .pointer()
                .starts_with(&format!("{}/responses/200/", op.source.pointer()))
        }));
    }
    // The actual corpus upload leaves additional multipart properties untyped.
    // Its refusal is preserved; no closed-object or binary-null policy is invented.
    let upload = contract
        .operations()
        .find(|op| op.operation_id() == Some("uploadFile"))
        .unwrap();
    let errors = plan_http(contract.clone(), &[upload.source().clone()], config).unwrap_err();
    assert!(errors.iter().any(|d| d.code == "http-form-untyped-extras"));
    let directory = tempfile::Builder::new()
        .prefix("openrouter-rust-protocol-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(
        &emit_http(
            &plan,
            &PackageConfig {
                name: "openrouter-protocol-sdk".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        &directory,
    )
    .unwrap();
    let target = std::env::var_os("SUSPECT_RUST_PROTOCOL_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-openrouter-protocol-msrv"
                } else {
                    "../../target/native-rust-openrouter-protocol"
                },
            )
        });
    let manifest = directory.join("rust/Cargo.toml");
    for args in [
        vec!["test", "--doc", "--features", "reqwest-rustls"],
        vec!["doc", "--no-deps", "--features", "reqwest-rustls"],
        vec!["run", "--example", "validated", "--features", "http"],
        vec!["package", "--allow-dirty", "--no-verify"],
    ] {
        checked(
            native_env(
                Command::new("cargo")
                    .args(args)
                    .args(["--offline", "--quiet", "--manifest-path"])
                    .arg(&manifest),
            )
            .arg("--target-dir")
            .arg(&target),
            &directory,
        );
    }
    let archive = target.join("package/openrouter-protocol-sdk-0.0.0.crate");
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&archive).unwrap()));
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &directory,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"openrouter-protocol-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"openrouter-protocol-sdk\",path=\"vendor/openrouter-protocol-sdk-0.0.0\",features=[\"reqwest-rustls\"]}\ntokio={version=\"=1.53.1\",features=[\"macros\",\"rt\"]}\n").unwrap();
    std::fs::write(consumer.join("src/lib.rs"), EXPANDED_CONSUMER).unwrap();
    checked(
        native_env(
            Command::new("cargo")
                .args(["test", "--offline", "--quiet", "--manifest-path"])
                .arg(consumer.join("Cargo.toml")),
        )
        .arg("--target-dir")
        .arg(target.join("installed").join(digest)),
        &directory,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn checked(command: &mut Command, retained: &Path) {
    let output = command.output().expect("required native tool missing");
    assert!(
        output.status.success(),
        "native fixture retained at {}\ncommand: {command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn native_env(command: &mut Command) -> &mut Command {
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings")
}

fn contract() -> Arc<Contract> {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = root.join("projects/docs/openapi/openapi.yaml");
    assert!(
        path.is_file(),
        "tracked OpenRouter source checkout (corpus) is required at {}",
        path.display()
    );
    let workspace = WorkspaceBuilder::new()
        .root(path.parent().unwrap())
        .build()
        .unwrap();
    Arc::new(
        Contract::from_workspace(&Arc::new(workspace), &Uri::from_path(&path).unwrap()).unwrap(),
    )
}

fn selected(contract: &Arc<Contract>) -> Vec<SourceId> {
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| WANTED.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        WANTED.len(),
        "five distinct tracked operations are required"
    );
    selected
}

fn operation<'a>(plan: &'a HttpPlan, id: &str) -> &'a PlannedOperation {
    plan.operations()
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap_or_else(|| panic!("operation {id} was not planned"))
}

/// The outer promoted model for an inline request-body schema.
fn body_symbol(plan: &HttpPlan, op: &PlannedOperation) -> String {
    let pointer = format!(
        "{}/requestBody/content/application~1json/schema",
        op.source.pointer()
    );
    let symbols = plan.codecs().models().symbols();
    let matches = symbols
        .iter()
        .filter(|symbol| symbol.source().pointer() == pointer)
        .map(|symbol| symbol.name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one body model at {pointer}; planned pointers: {:#?}",
        symbols
            .iter()
            .filter(|symbol| symbol.source().pointer().starts_with(op.source.pointer()))
            .map(|symbol| symbol.source().pointer().to_string())
            .collect::<Vec<_>>()
    );
    matches.into_iter().next().unwrap()
}

#[test]
#[ignore = "requires native Cargo, the tracked OpenRouter checkout and the pinned crate cache"]
fn installed_openrouter_native_consumer_preserves_source_wire_values() {
    assert!(
        Command::new("cargo").arg("--version").output().is_ok(),
        "native Cargo is required by this opt-in gate"
    );
    assert!(
        Command::new("tar").arg("--version").output().is_ok(),
        "external tar is required to extract the packaged .crate"
    );

    let contract = contract();
    let operations = selected(&contract);
    let plan = plan_http(contract, &operations, HttpConfig::default()).unwrap();
    assert_eq!(plan.operations().len(), WANTED.len());
    // The consumer source below relies on the deterministic native names.
    for (id, module, input) in [
        ("getCredits", "get_credits", "GetCredits"),
        ("createKeys", "create_keys", "CreateKeys"),
        ("updateKeys", "update_keys", "UpdateKeys"),
        (
            "listContainerFiles",
            "list_container_files",
            "ListContainerFiles",
        ),
        ("getContainerFile", "get_container_file", "GetContainerFile"),
    ] {
        let op = operation(&plan, id);
        assert_eq!(
            (
                op.module_name.as_str(),
                op.function_name.as_str(),
                op.input_type.as_str()
            ),
            (module, module, input),
            "unexpected native naming for {id}"
        );
        assert_eq!(op.success_type, format!("{input}Success"));
        assert_eq!(op.error_type, format!("{input}Error"));
        assert_eq!(op.api_error_type, format!("{input}ApiError"));
    }

    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.keep();
    let generated = directory.join("rust");
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: "openrouter-native-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &directory).unwrap();

    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
            "../../target/native-rust-http-openrouter-msrv"
        } else {
            "../../target/native-rust-http-openrouter"
        },
    );
    checked(
        native_env(
            Command::new("cargo")
                .args(["test", "--doc", "--offline", "--quiet", "--manifest-path"])
                .arg(generated.join("Cargo.toml")),
        )
        .arg("--target-dir")
        .arg(&target)
        .args(["--features", "reqwest-rustls"]),
        &directory,
    );
    checked(
        native_env(
            Command::new("cargo")
                .args([
                    "doc",
                    "--no-deps",
                    "--offline",
                    "--quiet",
                    "--manifest-path",
                ])
                .arg(generated.join("Cargo.toml")),
        )
        .arg("--target-dir")
        .arg(&target)
        .args(["--features", "reqwest-rustls"]),
        &directory,
    );
    checked(
        native_env(
            Command::new("cargo")
                .args([
                    "package",
                    "--offline",
                    "--allow-dirty",
                    "--no-verify",
                    "--quiet",
                    "--manifest-path",
                ])
                .arg(generated.join("Cargo.toml")),
        )
        .arg("--target-dir")
        .arg(&target),
        &directory,
    );

    let crate_path = target.join("package/openrouter-native-sdk-0.0.0.crate");
    assert!(
        crate_path.is_file(),
        "packaged .crate missing at {}",
        crate_path.display()
    );
    // Cargo package normalizes archive mtimes. A shared target can otherwise
    // reuse a prior same-name/version path dependency after extraction even
    // when its bytes changed. Isolate verification by the installed bytes.
    let archive_digest = format!("{:x}", Sha256::digest(std::fs::read(&crate_path).unwrap()));
    let vendor = directory.join("consumer/vendor");
    std::fs::create_dir_all(&vendor).unwrap();
    checked(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&crate_path)
            .arg("-C")
            .arg(&vendor),
        &directory,
    );
    let installed = vendor.join("openrouter-native-sdk-0.0.0");
    assert!(
        installed.join("Cargo.toml").is_file(),
        "extracted installed package missing at {}",
        installed.display()
    );

    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(consumer.join("Cargo.toml"), consumer_toml()).unwrap();
    std::fs::write(
        consumer.join("src/lib.rs"),
        consumer_lib(
            &body_symbol(&plan, operation(&plan, "createKeys")),
            &body_symbol(&plan, operation(&plan, "updateKeys")),
        ),
    )
    .unwrap();
    checked(
        native_env(
            Command::new("cargo")
                .args(["test", "--offline", "--quiet", "--manifest-path"])
                .arg(consumer.join("Cargo.toml")),
        )
        .arg("--target-dir")
        .arg(target.join("consumer").join(archive_digest)),
        &directory,
    );

    std::fs::remove_dir_all(&directory).unwrap();
}

fn consumer_toml() -> String {
    "[package]\nname = \"openrouter-native-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n\n[dependencies]\nopenrouter-native-sdk = { path = \"vendor/openrouter-native-sdk-0.0.0\", features = [\"reqwest-rustls\"] }\nreqwest = { version = \"=0.12.28\", default-features = false }\ntokio = { version = \"=1.53.1\", features = [\"macros\", \"rt\"] }\n".into()
}
fn consumer_lib(create_body: &str, update_body: &str) -> String {
    let mut source = String::from(
        "//! Independent consumer acceptance for the installed openrouter-native-sdk package.\n#![cfg(test)]\n\n",
    );
    source.push_str("use openrouter_native_sdk::models::");
    source.push_str(create_body);
    source.push_str(" as CreateBody;\nuse openrouter_native_sdk::models::");
    source.push_str(update_body);
    source.push_str(" as UpdateBody;\n");
    source.push_str(CONSUMER_BODY);
    source
}
const CONSUMER_BODY: &str = r##"use std::net::SocketAddr;

use openrouter_native_sdk::{
    Client, ClientOptions, Credentials, JsonNumber, Nullable, Presence,
    http::SdkErrorKind,
    operations::create_keys::{CreateKeys, CreateKeysError, CreateKeysSuccess},
    operations::get_container_file::{GetContainerFile, GetContainerFileSuccess},
    operations::get_credits::{GetCredits, GetCreditsApiError, GetCreditsError, GetCreditsSuccess},
    operations::list_container_files::{ListContainerFiles, ListContainerFilesSuccess},
    operations::update_keys::{UpdateKeys, UpdateKeysSuccess},
};

mod recording {
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    pub struct Recorded {
        pub method: String,
        pub target: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Recorded {
        pub fn header(&self, name: &str) -> &str {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
                .unwrap_or("")
        }

        pub fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }
    }

    pub struct Server {
        records: Mutex<Vec<Recorded>>,
        fixtures: Mutex<VecDeque<(u16, &'static str)>>,
    }

    impl Server {
        pub fn records(&self) -> std::sync::MutexGuard<'_, Vec<Recorded>> {
            self.records.lock().unwrap()
        }

        fn serve(&self, mut stream: TcpStream) -> Option<()> {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
            stream.set_write_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut buffer).ok()?;
                if read == 0 {
                    return None;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(end) = find(&bytes, b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let mut lines = head.lines();
            let mut request_line = lines.next()?.split(' ');
            let method = request_line.next()?.to_string();
            let target = request_line.next()?.to_string();
            let mut content_length = 0usize;
            let mut headers = Vec::new();
            for line in lines {
                if let Some((name, value)) = line.split_once(':') {
                    let name = name.trim();
                    let value = value.trim();
                    if name.eq_ignore_ascii_case("content-length") {
                        content_length = value.parse().ok()?;
                    }
                    headers.push((name.to_string(), value.to_string()));
                }
            }
            let mut body = bytes[header_end..].to_vec();
            while body.len() < content_length {
                let read = stream.read(&mut buffer).ok()?;
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&buffer[..read]);
            }
            self.records.lock().unwrap().push(Recorded {
                method,
                target,
                headers,
                body,
            });
            let (status, fixture) = self.fixtures.lock().unwrap().pop_front()?;
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{fixture}",
                fixture.len()
            );
            stream.write_all(response.as_bytes()).ok()?;
            stream.flush().ok()?;
            Some(())
        }
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|window| window == needle)
    }

    pub fn start(fixtures: Vec<(u16, &'static str)>) -> (SocketAddr, Arc<Server>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local recording listener");
        let address = listener.local_addr().unwrap();
        let server = Arc::new(Server {
            records: Mutex::new(Vec::new()),
            fixtures: Mutex::new(fixtures.into_iter().collect()),
        });
        let loop_server = Arc::clone(&server);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    break;
                };
                if loop_server.serve(stream).is_none() {
                    break;
                }
            }
        });
        (address, server)
    }
}
const MANAGEMENT_TOKEN: &str = "sk-or-management-native-test";
const HASH: &str = "f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943";

/// Independently authored from the tracked OpenAPI response schemas; the
/// recorded request bytes below are authored by hand, not codec round-trips.
const CREATE_KEYS_RESPONSE: &str = r#"{"data":{"hash":"f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943","name":"Native Test Key","label":"Native Test Key","disabled":false,"limit":50.250,"limit_remaining":50.250,"limit_reset":"monthly","include_byok_in_limit":true,"usage":0,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"created_at":"2026-09-08T10:30:00Z","updated_at":null,"external_user":null,"creator_user_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"},"key":"sk-or-v1-fixture-only"}"#;
const UPDATE_KEYS_RESPONSE: &str = r#"{"data":{"hash":"f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943","name":"Updated Native Key","label":"Updated Native Key","disabled":true,"limit":75.50,"limit_remaining":49.5,"limit_reset":"daily","include_byok_in_limit":true,"usage":25.5,"usage_daily":25.5,"usage_weekly":25.5,"usage_monthly":25.5,"byok_usage":17.38,"byok_usage_daily":17.38,"byok_usage_weekly":17.38,"byok_usage_monthly":17.38,"created_at":"2025-08-24T10:30:00Z","updated_at":"2025-08-24T16:00:00Z","external_user":null,"creator_user_id":"user_2dHFtVWx2n56w6HkM0000000000","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#;
const CONTAINER_FILE_RESPONSE: &str = r#"{"id":"cfile_b3V0L3JlcG9ydC5jc3Y","object":"container.file","container_id":"sess_abc123","bytes":123,"created_at":1755640000,"path":"out/report.csv","source":"assistant"}"#;

fn client(address: SocketAddr) -> Client<openrouter_native_sdk::reqwest_transport::ReqwestTransport> {
    Client::with_reqwest(Credentials::api_key(MANAGEMENT_TOKEN))
        .unwrap()
        .with_options(ClientOptions {
            server_url: Some(format!("http://{address}/api/v1")),
            ..ClientOptions::default()
        })
}

fn recorded(server: &recording::Server) -> recording::Recorded {
    let mut records = server.records();
    assert!(!records.is_empty(), "recording server has no pending request");
    records.remove(0)
}
#[tokio::test]
async fn exact_wire_values_typed_declared_errors_and_escaping() {
    let (address, server) = recording::start(vec![
        (200, r#"{"data":{"total_credits":100.50000000000000001,"total_usage":25.75}}"#),
        (401, r#"{"error":{"code":401,"message":"Missing Authentication header"}}"#),
        (201, CREATE_KEYS_RESPONSE),
        (200, UPDATE_KEYS_RESPONSE),
        (200, CONTAINER_FILE_RESPONSE),
        (200, r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#),
    ]);
    let client = client(address);

    // getCredits: GET path, bearer + profile headers, exact numeric tokens.
    let GetCreditsSuccess::Status200(credits) = client.get_credits(GetCredits::new()).await.unwrap();
    assert_eq!(credits.status, 200);
    assert_eq!(credits.data.data.total_credits.as_str(), "100.50000000000000001");
    assert_eq!(credits.data.data.total_usage.as_str(), "25.75");
    let request = recorded(&server);
    assert_eq!(request.method, "GET");
    assert_eq!(request.path(), "/api/v1/credits");
    assert_eq!(request.header("authorization"), "Bearer sk-or-management-native-test");
    assert_eq!(request.header("accept"), "application/json");
    assert!(request.body.is_empty());

    // The declared 401 stays a typed API error with its source-schema payload.
    match client.get_credits(GetCredits::new()).await.unwrap_err() {
        GetCreditsError::Api(error) => match *error {
            GetCreditsApiError::Status401(response) => {
                assert_eq!(response.status, 401);
                assert_eq!(response.data.error.code.as_str(), "401");
                assert_eq!(response.data.error.message, "Missing Authentication header");
                assert_eq!(response.data.user_id, Presence::Absent);
            }
            other => panic!("unexpected declared error: {other:?}"),
        },
        other => panic!("unexpected failure: {other:?}"),
    }
    let denied = recorded(&server);
    assert_eq!(denied.method, "GET");
    assert_eq!(denied.path(), "/api/v1/credits");

    // createKeys: exact body bytes with omission, explicit null and exact numbers.
    let mut create_body = CreateBody::new("Native Test Key".to_string());
    create_body.limit = Presence::Value("50.25".parse::<JsonNumber>().unwrap());
    create_body.limit_reset = Presence::Null;
    let CreateKeysSuccess::Status201(created) =
        client.create_keys(CreateKeys::new(create_body)).await.unwrap();
    assert_eq!(
        created.data.key,
        "sk-or-v1-fixture-only"
    );
    let key = &created.data.data;
    assert_eq!(key.hash, HASH);
    assert!(matches!(&key.limit, Nullable::Value(number) if number.as_str() == "50.250"));
    assert_eq!(key.updated_at, Nullable::Null);
    assert_eq!(key.external_user, Nullable::Null);
    let request = recorded(&server);
    assert_eq!(request.method, "POST");
    assert_eq!(request.path(), "/api/v1/keys");
    assert_eq!(request.header("content-type"), "application/json");
    assert_eq!(
        request.body,
        br#"{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}"#.to_vec()
    );

    // updateKeys: PATCH with the required hash parameter and optional body.
    let mut update_body = UpdateBody::new();
    update_body.disabled = Some(true);
    update_body.limit = Presence::Value("75.50".parse::<JsonNumber>().unwrap());
    update_body.limit_reset = Presence::Null;
    update_body.name = Some("Updated Native Key".to_string());
    let UpdateKeysSuccess::Status200(updated) =
        client.update_keys(UpdateKeys::new(HASH.to_string(), update_body)).await.unwrap();
    assert_eq!(updated.data.data.name, "Updated Native Key");
    assert!(
        matches!(&updated.data.data.limit_remaining, Nullable::Value(number) if number.as_str() == "49.5")
    );
    let request = recorded(&server);
    assert_eq!(request.method, "PATCH");
    assert_eq!(request.path(), format!("/api/v1/keys/{HASH}"));
    assert_eq!(
        request.body,
        br#"{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}"#.to_vec()
    );

    // getContainerFile: RFC3986 path escaping of !'()* and UTF-8.
    let GetContainerFileSuccess::Status200(file) = client
        .get_container_file(GetContainerFile::new(
            "sess_abc123".to_string(),
            "cfile_a/b 雪!'()*".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(file.data.id, "cfile_b3V0L3JlcG9ydC5jc3Y");
    assert_eq!(file.data.container_id, "sess_abc123");
    assert_eq!(file.data.bytes.as_str(), "123");
    assert_eq!(file.data.created_at.as_str(), "1755640000");
    assert_eq!(file.data.path, "out/report.csv");
    let request = recorded(&server);
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A"
    );

    let ListContainerFilesSuccess::Status200(list) = client.list_container_files(
        ListContainerFiles::new("sess_abc123".into()).with_limit(2).with_after("a/b +雪".into())
    ).await.unwrap();
    assert!(list.data.data.is_empty());
    assert!(matches!(list.data.first_id, Nullable::Null));
    assert!(!list.data.has_more);
    let request = recorded(&server);
    assert_eq!(request.target, "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA");

    // Invalid native input never enters the transport.
    let before = server.records().len();
    match client
        .create_keys(CreateKeys::new(CreateBody::new(String::new())))
        .await
        .unwrap_err()
    {
        CreateKeysError::Sdk(error) => assert_eq!(error.kind, SdkErrorKind::RequestValidation),
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(server.records().len(), before);
}

#[test]
fn all_five_operation_exports_are_consumable() {
    use openrouter_native_sdk::operations;
    let _ = operations::get_credits::GetCredits::new();
    let _ = operations::list_container_files::ListContainerFiles::new("sess_abc123".to_string());
    let _ = operations::get_container_file::GetContainerFile::new(
        "sess_abc123".to_string(),
        "cfile_x".to_string(),
    );
    let _ = operations::create_keys::CreateKeys::new(CreateBody::new("x".to_string()));
    let _ = operations::update_keys::UpdateKeys::new("hash".to_string(), UpdateBody::new());
}

#[tokio::test]
async fn recommended_adapter_disables_proxy_routing_even_on_custom_builders() {
    let (address, server) = recording::start(vec![(200, r#"{"data":{"total_credits":1,"total_usage":0}}"#)]);
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let builder = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://{}", proxy.local_addr().unwrap())).unwrap())
        .timeout(std::time::Duration::from_secs(2));
    let transport = openrouter_native_sdk::reqwest_transport::ReqwestTransport::from_builder(builder).unwrap();
    let client = Client::with_transport(transport, Credentials::api_key(MANAGEMENT_TOKEN))
        .with_options(ClientOptions { server_url:Some(format!("http://{address}/api/v1")),..Default::default() });
    client.get_credits(GetCredits::new()).await.unwrap();
    assert_eq!(recorded(&server).path(), "/api/v1/credits");
    assert!(matches!(proxy.accept(), Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}
"##;

const EXPANDED_CONSUMER: &str = r##"
#[cfg(test)]
mod tests {
    use sdk::{Client,ClientOptions,Credentials};
    use std::{io::{Read,Write},net::{TcpListener,TcpStream},sync::{Arc,Mutex},collections::VecDeque};
    struct Recorded {method:String,target:String,headers:Vec<(String,String)>,body:Vec<u8>}
    impl Recorded {fn header(&self,name:&str)->&str{self.headers.iter().find(|(k,_)|k.eq_ignore_ascii_case(name)).map(|(_,v)|v.as_str()).unwrap_or("")}}
    fn read(stream:&mut TcpStream)->Recorded {
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut bytes=Vec::new();let mut buffer=[0u8;1024];
        let end=loop{let n=stream.read(&mut buffer).unwrap();assert!(n>0);bytes.extend_from_slice(&buffer[..n]);if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){break end+4;}assert!(bytes.len()<65536);};
        let text=std::str::from_utf8(&bytes[..end]).unwrap();let mut lines=text.lines();let mut request=lines.next().unwrap().split(' ');
        let method=request.next().unwrap().into();let target=request.next().unwrap().into();
        let headers=lines.filter_map(|l|l.split_once(':')).map(|(k,v)|(k.into(),v.trim().into())).collect::<Vec<(String,String)>>();
        let length=headers.iter().find(|(k,_)|k.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.parse::<usize>().unwrap()).unwrap_or(0);
        while bytes.len()-end<length{let n=stream.read(&mut buffer).unwrap();assert!(n>0);bytes.extend_from_slice(&buffer[..n]);}
        Recorded{method,target,headers,body:bytes[end..end+length].to_vec()}
    }
    #[tokio::test]
    async fn source_declared_bytes_no_auth_and_delete_are_real_http_calls(){
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let seen=Arc::new(Mutex::new(Vec::new()));let record=seen.clone();
        let bytes=vec![0,255,128,13,10,0];
        let mut replies=VecDeque::from([
            (200,"application/octet-stream",bytes.clone()),
            (200,"application/octet-stream",bytes.clone()),
            (410,"application/json",br#"{"error":{"code":410,"message":"Deprecated endpoint"}}"#.to_vec()),
            (200,"application/json",br#"{"_shape":"openrouter","id":"or_file_native","type":"file_deleted"}"#.to_vec()),
        ]);
        let server=std::thread::spawn(move||{while let Some((status,media,body))=replies.pop_front(){let (mut stream,_)=listener.accept().unwrap();record.lock().unwrap().push(read(&mut stream));write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();stream.write_all(&body).unwrap();stream.flush().unwrap();}});
        let client=Client::with_reqwest(Credentials::api_key("caller-key")).unwrap().with_options(ClientOptions{server_url:Some(format!("http://{address}/api/v1")),..Default::default()});
        let container=client.download_container_file_content(sdk::operations::download_container_file_content::DownloadContainerFileContent::new("sess_native".into(),"cfile_a/b 雪".into())).await.unwrap().into_response();assert_eq!(container.status,200);assert_eq!(container.data,bytes);
        let file=client.download_file_content(sdk::operations::download_file_content::DownloadFileContent::new("or_file_native".into())).await.unwrap().into_response();assert_eq!(file.data,bytes);
        match client.create_coinbase_charge_default().await.unwrap_err(){sdk::operations::create_coinbase_charge::CreateCoinbaseChargeError::Api(error)=>{let sdk::operations::create_coinbase_charge::CreateCoinbaseChargeApiError::Status410(response)=*error;assert_eq!(response.data.error.code.as_str(),"410");},other=>panic!("unexpected error: {other:?}")}
        let deleted=client.delete_file(sdk::operations::delete_file::DeleteFile::new("or_file_native".into())).await.unwrap().into_response();assert_eq!(sdk::codecs::FileDeleteResponseCodec::encode(&deleted.data).unwrap(),r#"{"_shape":"openrouter","id":"or_file_native","type":"file_deleted"}"#);
        server.join().unwrap();let records=seen.lock().unwrap();assert_eq!(records.len(),4);
        assert_eq!(records[0].method,"GET");assert_eq!(records[0].target,"/api/v1/containers/sess_native/files/cfile_a%2Fb%20%E9%9B%AA/content");assert_eq!(records[0].header("authorization"),"Bearer caller-key");
        assert_eq!(records[1].target,"/api/v1/files/or_file_native/content");assert_eq!(records[1].header("authorization"),"Bearer caller-key");
        assert_eq!(records[2].method,"POST");assert_eq!(records[2].target,"/api/v1/credits/coinbase");assert_eq!(records[2].header("authorization"),"");
        assert_eq!(records[3].method,"DELETE");assert_eq!(records[3].target,"/api/v1/files/or_file_native");assert_eq!(records[3].header("authorization"),"Bearer caller-key");assert!(records.iter().all(|r|r.body.is_empty()));
    }
}
"##;
