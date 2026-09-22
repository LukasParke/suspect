//! Focused native physical-document server-base and resource-metadata witness.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use suspect_codegen::{
    http_protocol::{ApiUrlBase, Capability},
    rust_http::{self, HttpPlan, PackageConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

struct Server {
    base: String,
    seen: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let records = seen.clone();
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut buffer = [0u8; 1024];
                        while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                            let n = stream.read(&mut buffer).unwrap();
                            assert!(n > 0);
                            bytes.extend_from_slice(&buffer[..n]);
                            assert!(bytes.len() < 65536);
                        }
                        let text = std::str::from_utf8(&bytes).unwrap();
                        let mut lines = text.lines();
                        let target = lines.next().unwrap().split(' ').nth(1).unwrap().to_owned();
                        let authorization = lines
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                            .map(|(_, v)| v.trim().to_owned())
                            .unwrap_or_default();
                        records.lock().unwrap().push((target, authorization));
                        stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("local wire server: {error}"),
                }
            }
        });
        Self {
            base,
            seen,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take()
            && let Err(error) = thread.join()
            && !thread::panicking()
        {
            std::panic::resume_unwind(error);
        }
    }
}

fn supplied(entry: &str, documents: &[(&str, &str, Value)]) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap())
}
fn fixture(entry_base: &str, component_base: &str, local: &str) -> HttpPlan {
    let entry_uri = format!("{entry_base}/retrieved/deep/entry.json");
    let component_uri = format!("{component_base}/library/nested/components.json");
    let entry = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/entry.json","info":{"title":"Physical bases","version":"1"},"paths":{
        "/implicit":{"$ref":"https://logical.example/shared/components.json#/components/pathItems/Implicit"},
        "/explicit":{"$ref":"https://logical.example/shared/components.json#/components/pathItems/Explicit"},
        "/empty":{"$ref":"https://logical.example/shared/components.json#/components/pathItems/Empty"},
        "/local":{"$ref":"https://logical.example/local/api.json#/components/pathItems/Local"},
        "/normalization":{"$ref":"https://logical.example/shared/components.json#/components/pathItems/Normalization"}
    }});
    let component = json!({"openapi":"3.2.0","$self":"https://logical.example/shared/components.json","info":{"title":"Referenced operations","version":"1"},"components":{
        "securitySchemes":{"auth":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"tokens","scopes":{"read":"Read"}}}},"oidc":{"type":"openIdConnect","openIdConnectUrl":"../discovery"}},
        "pathItems":{
            "Implicit":{"get":{"operationId":"implicit","security":[],"responses":{"204":{}}}},
            "Explicit":{"get":{"operationId":"explicit","servers":[{"url":"../Api%2fV1"}],"security":[{"auth":["read"]},{"oidc":[]}],"responses":{"204":{}}}},
            "Empty":{"get":{"operationId":"empty","servers":[],"security":[],"responses":{"204":{}}}},
            "Normalization":{"get":{"operationId":"normalization","servers":[{"url":"%2e%2e/api"}],"security":[],"responses":{"204":{}}}}
        }
    }});
    let local_value = json!({"openapi":"3.2.0","$self":"https://logical.example/local/api.json","info":{"title":"Local physical document","version":"1"},"components":{"pathItems":{
        "Local":{"get":{"operationId":"local","servers":[{"url":"."}],"security":[],"responses":{"204":{}}}}
    }}});
    let contract = supplied(
        "https://requested.example/entry.json",
        &[
            ("https://requested.example/entry.json", &entry_uri, entry),
            (
                "https://requested.example/components.json",
                &component_uri,
                component,
            ),
            (local, local, local_value),
        ],
    );
    assert_eq!(contract.entry().as_str(), entry_uri);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = rust_http::plan_http_v2(contract, &selected, Default::default()).unwrap();
    assert!(plan.protocol().codec_roots().is_empty());
    for op in plan.operations() {
        let server = &op.wire().servers().candidates()[0];
        assert_eq!(server.url_base(), ApiUrlBase::ServerDocument);
        let expected = match op.operation_id.as_str() {
            "implicit" => entry_uri.as_str(),
            "local" => local,
            _ => component_uri.as_str(),
        };
        assert_eq!(
            server.document_base().source().document().as_str(),
            expected
        );
        assert!(server.document_base().source().pointer().is_empty());
        assert!(
            op.wire()
                .source()
                .terminal_resource()
                .unwrap()
                .canonical_uri()
                .starts_with("https://logical.example/")
        );
    }
    plan
}

#[test]
fn rust_document_base_capability_does_not_enable_schema_or_dynamic_execution() {
    let c = rust_http::native_capabilities();
    assert!(c.supports(Capability::DocumentRelativeServers));
    assert!(!c.supports(Capability::SchemaResources));
    assert!(!c.supports(Capability::DynamicSchemaReferences));
    let plan = fixture(
        "https://physical.example",
        "https://cdn.example",
        "file:///physical/local.json",
    );
    let implicit = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "implicit")
        .unwrap();
    assert_eq!(
        implicit.source.document().as_str(),
        "https://cdn.example/library/nested/components.json"
    );
    assert_eq!(
        implicit.wire().servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "https://physical.example/retrieved/deep/entry.json"
    );
}

fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}
fn checked(command: &mut Command, root: &Path) {
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
        "retained physical-base witness {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires native Cargo/current or 1.88, pinned reqwest and tar"]
fn installed_rust_relative_servers_use_effective_physical_documents() {
    let entry = Server::new();
    let component = Server::new();
    let directory = tempfile::Builder::new()
        .prefix("rust-physical-base-")
        .tempdir()
        .unwrap()
        .keep();
    let local = Uri::from_path(&directory.join("physical/local.json"))
        .unwrap()
        .to_string();
    let plan = fixture(&entry.base, &component.base, &local);
    suspect_codegen::write_files(
        &rust_http::emit_http(
            &plan,
            &PackageConfig {
                name: "physical-document-sdk".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        &directory,
    )
    .unwrap();
    let target = std::env::var_os("SUSPECT_RUST_RESOURCES_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-physical-base-msrv"
                } else {
                    "../../target/native-rust-physical-base"
                },
            )
        });
    let manifest = directory.join("rust/Cargo.toml");
    checked(
        cargo("test", &manifest, &target).args(["--doc", "--features", "reqwest-rustls"]),
        &directory,
    );
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &directory,
    );
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join("package/physical-document-sdk-0.0.0.crate");
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &directory,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"physical-base-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"physical-document-sdk\",path=\"vendor/physical-document-sdk-0.0.0\",features=[\"reqwest-rustls\"]}\ntokio={version=\"=1.53.1\",features=[\"macros\",\"rt\"]}\n").unwrap();
    let constants = format!(
        "const ENTRY:&str={:?};\nconst COMPONENT:&str={:?};\nconst LOCAL:&str={local:?};\n",
        entry.base, component.base
    );
    std::fs::write(
        consumer.join("src/main.rs"),
        format!("{constants}\n{CONSUMER}"),
    )
    .unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()));
    checked(
        &mut cargo(
            "run",
            &consumer.join("Cargo.toml"),
            &target.join("installed").join(digest),
        ),
        &directory,
    );
    assert_eq!(
        *entry.seen.lock().unwrap(),
        vec![
            ("/implicit".into(), "".into()),
            (
                "/runtime/Api%2fV1/explicit".into(),
                "Bearer caller-owned".into()
            ),
            ("/runtime/deep/local".into(), "".into())
        ]
    );
    assert_eq!(
        *component.seen.lock().unwrap(),
        vec![
            (
                "/library/Api%2fV1/explicit".into(),
                "Bearer caller-owned".into()
            ),
            ("/empty".into(), "".into())
        ]
    );
    eprintln!(
        "native physical-base witness retained {}",
        directory.display()
    );
}

const CONSUMER: &str = r#"
use sdk::{Client,ClientOptions,Credentials,operations};
use sdk::http::{ApiUrlBase,CredentialKind,Security,SdkErrorKind};
fn client()->Client<sdk::reqwest_transport::ReqwestTransport>{Client::with_reqwest(Credentials::auth("Bearer caller-owned")).unwrap()}
#[tokio::main(flavor="current_thread")]
async fn main(){
    let implicit=&operations::implicit::OPERATION;
    assert_eq!(implicit.source.document,format!("{COMPONENT}/library/nested/components.json"));
    assert_eq!(implicit.servers[0].document_base.document,format!("{ENTRY}/retrieved/deep/entry.json"));
    assert_eq!(implicit.provenance.use_site_resource.unwrap().canonical_uri,"https://logical.example/catalog/entry.json");
    assert_eq!(implicit.provenance.terminal_resource.unwrap().canonical_uri,"https://logical.example/shared/components.json");
    assert_eq!(implicit.provenance.references.len(),implicit.provenance.reference_resources.len());
    assert!(implicit.provenance.use_site_resource.unwrap().aliases.contains(&"https://requested.example/entry.json"));
    let explicit=&operations::explicit::OPERATION;let server=&explicit.servers[0];
    assert_eq!(server.document_base.document,format!("{COMPONENT}/library/nested/components.json"));
    assert_eq!(server.url_base(),ApiUrlBase::ServerDocument);
    assert_eq!(server.provenance.unwrap().terminal_resource.unwrap().base_uri,"https://logical.example/shared/components.json");
    let Security::Alternatives(alternatives)=explicit.security else{panic!("OAuth metadata")};let requirement=&alternatives[0].requirements[0];
    assert_eq!(requirement.kind.url_base(),Some(ApiUrlBase::EffectiveServer));
    let CredentialKind::OAuth2{flows,..}=requirement.kind else{panic!("OAuth metadata")};
    assert_eq!(flows[0].url_base(),ApiUrlBase::EffectiveServer);assert_eq!(flows[0].token_url.unwrap().value,"tokens");
    let oidc=&alternatives[1].requirements[0];assert_eq!(oidc.kind.url_base(),Some(ApiUrlBase::EffectiveServer));
    let CredentialKind::OpenIdConnect{discovery_url}=oidc.kind else{panic!("OIDC metadata")};assert_eq!(discovery_url.value,"../discovery");
    client().implicit_default().await.unwrap();client().explicit_default().await.unwrap();client().empty_default().await.unwrap();
    client().with_options(ClientOptions{document_url:Some(format!("{ENTRY}/runtime/deep/api.json")),..Default::default()}).explicit_default().await.unwrap();
    let error=client().local_default().await.unwrap_err();let operations::local::LocalError::Sdk(error)=error else{panic!("local base refusal")};
    assert_eq!(error.kind,SdkErrorKind::RequestRepresentation);assert_eq!(error.source.document,LOCAL);
    client().with_options(ClientOptions{document_url:Some(format!("{ENTRY}/runtime/deep/api.json")),..Default::default()}).local_default().await.unwrap();
    let error=client().normalization_default().await.unwrap_err();let operations::normalization::NormalizationError::Sdk(error)=error else{panic!("normalization refusal")};
    assert_eq!(error.kind,SdkErrorKind::RequestRepresentation);assert!(error.source.pointer.ends_with("/servers/0"));
    for segment in ["%2e","%2E%2e",".%2E","%2e."] {
        let error=client().with_options(ClientOptions{document_url:Some(format!("{ENTRY}/runtime/{segment}/deep/api.json")),..Default::default()}).explicit_default().await.unwrap_err();
        let operations::explicit::ExplicitError::Sdk(error)=error else{panic!("document override normalization refusal")};
        assert_eq!(error.kind,SdkErrorKind::RequestRepresentation);assert_eq!(error.source,server.document_base);
        let error=client().with_options(ClientOptions{server_url:Some(format!("{ENTRY}/runtime/{segment}/api")),..Default::default()}).explicit_default().await.unwrap_err();
        let operations::explicit::ExplicitError::Sdk(error)=error else{panic!("server override normalization refusal")};assert_eq!(error.kind,SdkErrorKind::RequestRepresentation);
    }
}
"#;
