//! Public resource-aware Rust planners -> packaged SDK -> installed type/wire consumers.
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
    http_protocol::Capability,
    rust_codecs,
    rust_http::{self, HttpConfig, HttpPlan, PackageConfig},
    rust_models,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::OwnedProgram;
use suspect_source::Uri;

const REQUESTED: &str = "https://requested.example/v3/api.json";
const LIBRARY_REQUESTED: &str = "https://requested.example/v3/library.json";
const LIBRARY_PHYSICAL: &str = "https://physical.example/retrieved/library.json";
const LIBRARY_LOGICAL: &str = "https://logical.example/models/library.json";

fn source(document: &str, tokens: &[&str]) -> SourceId {
    tokens.iter().fold(
        SourceId::new(Uri::parse(document).unwrap(), Default::default()),
        |id, token| id.child(token),
    )
}
fn supplied(entry: &str, documents: Vec<(&str, &str, Value)>) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec_pretty(&value).unwrap(),
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
fn library() -> Value {
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":LIBRARY_LOGICAL,"$defs":{
        "Static":{"type":"string","minLength":1},
        "Fallback":{"$id":"fallback","$dynamicAnchor":"value","type":"string","minLength":1,"examples":["fallback-must-not-be-a-dynamic-example"]},
        "Generic":{"$id":"generic","type":"object","required":["payload"],"properties":{
            "label":{"$ref":format!("{LIBRARY_REQUESTED}#/$defs/Static")},
            "payload":{"anyOf":[{"$dynamicRef":"fallback#value"}]},
            "direct":{"$dynamicRef":"fallback#value"}
        },"additionalProperties":false,"unevaluatedProperties":false},
        "IntegerEnvelope":{"$id":"integers","$defs":{"Slot":{"$dynamicAnchor":"value","type":"integer","minimum":0}},"$ref":"generic"},
        "NullableEnvelope":{"$id":"nullable","$defs":{"Slot":{"$dynamicAnchor":"value","type":["integer","null"]}},"$ref":"generic"}
    }})
}
fn contract(base: &str, library: Value) -> Arc<Contract> {
    let physical = format!("{base}/source/deep/api.json");
    let integer = json!({"schema":{"$ref":"https://logical.example/models/integers"},"examples":{
        "good":{"value":{"payload":9007199254740993u64,"label":"source-backed"}},
        "wrongBinding":{"value":{"payload":"fallback-must-not-be-a-dynamic-example"}},
        "wrongType":{"value":{"payload":null}},"extra":{"value":{"payload":1,"extra":true}}
    }});
    let nullable = json!({"schema":{"$ref":"https://logical.example/models/nullable"},"examples":{
        "good":{"value":{"payload":null,"direct":null}},"wrongBinding":{"value":{"payload":"fallback"}}
    }});
    let entry = json!({"openapi":"3.2.0","$self":"https://logical.example/v3/api.json","info":{"title":"Rust native resources","version":"1"},"servers":[{"url":"../wire%2fV3"}],"paths":{
        "/integers":{"post":{"operationId":"putInteger","requestBody":{"required":true,"content":{"application/json":integer.clone()}},"responses":{"200":{"description":"resource-bound result","content":{"application/json":integer.clone()}}}}},
        "/nullable":{"post":{"operationId":"putNullable","requestBody":{"required":true,"content":{"application/json":nullable.clone()}},"responses":{"200":{"description":"resource-bound nullable result","content":{"application/json":nullable}}}}},
        "/invalid-response":{"get":{"operationId":"invalidResponse","responses":{"200":{"description":"deliberately invalid wire control","content":{"application/json":integer}}}}}
    }});
    supplied(
        REQUESTED,
        vec![
            (REQUESTED, &physical, entry),
            (LIBRARY_REQUESTED, LIBRARY_PHYSICAL, library),
        ],
    )
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}
fn plan(base: &str, config: HttpConfig) -> HttpPlan {
    let contract = contract(base, library());
    rust_http::plan_http_v3(contract.clone(), &selected(&contract), config).unwrap()
}

#[test]
fn rust_v3_planners_preserve_ordinary_program_bytes_and_old_resource_fences() {
    let entry = "https://physical.example/base.json";
    for (schema, version) in [
        (
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false}),
            OwnedProgram::V1_VERSION,
        ),
        (
            json!({"type":"object","properties":{"active":{"type":"boolean"},"count":{"type":"integer"}},"if":{"properties":{"active":{"const":true}}},"then":{"required":["count"]},"unevaluatedProperties":false}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let c = supplied(
            entry,
            vec![(
                entry,
                entry,
                json!({"openapi":"3.1.2","info":{"title":"Base byte witness","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}),
            )],
        );
        let roots = [source(entry, &["components", "schemas", "Root"])];
        assert_eq!(
            rust_models::plan_models_v2(&c, &roots).render().unwrap(),
            rust_models::plan_models_v3(&c, &roots).render().unwrap()
        );
        let old = rust_codecs::plan_codecs_v2(c.clone(), &roots, Default::default()).unwrap();
        let new = rust_codecs::plan_codecs_v3(c, &roots, Default::default()).unwrap();
        assert_eq!(new.validation_version(), version);
        assert_eq!(
            old.render(),
            new.render(),
            "v3 selection must not rewrite an ordinary package"
        );
    }
    let c = contract("https://physical.example", library());
    let roots = [source(LIBRARY_PHYSICAL, &["$defs", "IntegerEnvelope"])];
    for result in [
        rust_codecs::plan_codecs(c.clone(), &roots, Default::default()),
        rust_codecs::plan_codecs_v2(c.clone(), &roots, Default::default()),
    ] {
        let diagnostics = result.unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.source.document().as_str() == LIBRARY_PHYSICAL && d.at.start < d.at.end)
        );
    }
    for result in [
        rust_http::plan_http(c.clone(), &selected(&c), Default::default()),
        rust_http::plan_http_v2(c.clone(), &selected(&c), Default::default()),
    ] {
        let diagnostics = result.unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == "http-capability-required" && d.at.start < d.at.end),
            "{diagnostics:?}"
        );
    }
    assert!(!rust_http::native_capabilities().supports(Capability::SchemaResources));
    assert!(!rust_http::native_capabilities().supports(Capability::DynamicSchemaReferences));
}

#[test]
fn rust_v3_nested_entry_retains_idless_ancestor_resource_admission() {
    let entry = "https://physical.example/nested.json";
    let c = supplied(
        entry,
        vec![(
            entry,
            entry,
            json!({"openapi":"3.1.2","info":{"title":"Nested native entry","version":"1"},"paths":{},"components":{"schemas":{
                "Outer":{"$dynamicAnchor":"outer","type":"boolean","$defs":{"Nested":{"type":"integer"}}},
                "Ordinary":{"type":"string"}
            }}}),
        )],
    );
    let root = source(
        entry,
        &["components", "schemas", "Outer", "$defs", "Nested"],
    );
    let p = rust_codecs::plan_codecs_v3(c.clone(), std::slice::from_ref(&root), Default::default())
        .unwrap();
    assert_eq!(p.validation_version(), OwnedProgram::V3_VERSION);
    assert!(rust_codecs::plan_codecs_v2(c.clone(), &[root], Default::default()).is_err());
    // A dynamic anchor elsewhere in the same physical document is not an
    // ancestor of this ordinary entry and must not promote its base program.
    let root = source(entry, &["components", "schemas", "Ordinary"]);
    let p =
        rust_codecs::plan_codecs_v3(c, std::slice::from_ref(&root), Default::default()).unwrap();
    assert_eq!(p.validation_version(), OwnedProgram::V1_VERSION);
}

#[test]
fn rust_v3_http_examples_and_candidate_diagnostics_keep_physical_sources() {
    let c = contract("https://physical.example", library());
    let p = rust_http::plan_http_v3(c.clone(), &selected(&c), Default::default()).unwrap();
    assert_eq!(
        (
            p.codecs().validation_version(),
            p.codecs().validation_profile()
        ),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    for feature in [
        Capability::SchemaResources,
        Capability::DynamicSchemaReferences,
        Capability::DocumentRelativeServers,
    ] {
        assert!(p.protocol().capabilities().supports(feature));
    }
    let binding = source(
        LIBRARY_PHYSICAL,
        &["$defs", "IntegerEnvelope", "$defs", "Slot"],
    );
    assert!(p.protocol().codec_schema_closure().contains(&binding));
    assert!(
        !p.protocol().codec_roots().contains(&binding),
        "candidate schemas are not additional HTTP codec inputs"
    );
    let examples = p.examples();
    let good = examples
        .operations()
        .iter()
        .flat_map(|op| &op.entries)
        .collect::<Vec<_>>();
    assert_eq!(good.len(), 5);
    assert!(good.iter().all(|e| {
        e.declared_source.as_ref().is_some_and(|s| {
            s.document() == c.entry() && s.pointer().ends_with("/examples/good/value")
        })
    }));
    assert!(
        good.iter()
            .all(|e| e.value["payload"].is_number() || e.value["payload"].is_null())
    );
    for suffix in [
        "/examples/wrongBinding/value",
        "/examples/wrongType/value",
        "/examples/extra/value",
    ] {
        assert!(
            examples
                .diagnostics()
                .iter()
                .any(|d| d.source.document() == c.entry()
                    && d.source.pointer().ends_with(suffix)
                    && d.at.start < d.at.end),
            "{:?}",
            examples.diagnostics()
        );
    }
    let mut value = library();
    value["$defs"]["IntegerEnvelope"]["$defs"]["Slot"]["readOnly"] = json!(true);
    let c = contract("https://physical.example", value);
    let error = rust_http::plan_http_v3(c.clone(), &selected(&c), Default::default()).unwrap_err();
    assert!(
        error.iter().any(|d| d.source == binding.child("readOnly")
            && d.code == "http-directional-codec-unsupported"
            && d.at.start < d.at.end),
        "{error:?}"
    );
}

fn profile_document(schema: Value, example: Value) -> Value {
    let media = json!({"schema":{"$ref":"#/components/schemas/Root"},"example":example});
    json!({"openapi":"3.1.2","info":{"title":"HTTP profile selection","version":"1"},
        "servers":[{"url":"https://example.test/api"}],"security":[],
        "components":{"schemas":{"Root":schema,
            "Unselected":{"$id":"urn:unselected","$dynamicAnchor":"unused","type":"boolean"}}},
        "paths":{"/echo":{"post":{"operationId":"echo",
            "requestBody":{"required":true,"content":{"application/json":media.clone()}},
            "responses":{"200":{"description":"Echo","content":{"application/json":media}}}}},
            "/unselected":{"get":{"operationId":"unselected","responses":{"200":{"description":"Outside selected closure",
                "content":{"application/json":{"schema":{"$ref":"#/components/schemas/Unselected"}}}}}}}}
    })
}

fn profile_contract(document: Value) -> Arc<Contract> {
    let entry = "https://physical.example/http-profile.json";
    supplied(entry, vec![(entry, entry, document)])
}

fn metadata_only_byte_document() -> Value {
    json!({"openapi":"3.2.0","$self":"https://logical.example/byte-api.json",
        "info":{"title":"Metadata without JSON codec roots","version":"1"},
        "paths":{"/echo":{"post":{"operationId":"echo","security":[],
            "requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},
            "responses":{"204":{"description":"Stored"}}}}}})
}

#[test]
fn rust_v3_http_profile_preserves_all_ordinary_artifacts() {
    use suspect_codegen::http_protocol::CompatibilityProfile;
    let documents = [
        (
            profile_document(json!({"type":"string"}), json!("value")),
            OwnedProgram::V1_VERSION,
        ),
        (
            profile_document(
                json!({"type":"object","required":["enabled"],"properties":{"enabled":{"type":"boolean"}},
            "if":{"properties":{"enabled":{"const":true}}},"then":false,"else":true,"unevaluatedProperties":false}),
                json!({"enabled":false}),
            ),
            OwnedProgram::V2_VERSION,
        ),
        (metadata_only_byte_document(), OwnedProgram::V1_VERSION),
    ];
    for (document, version) in documents {
        let c = profile_contract(document);
        let selection = c
            .operations()
            .filter(|op| op.operation_id() == Some("echo"))
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let config = HttpConfig {
            max_request_bytes: 8192,
            max_response_bytes: 4096,
            max_part_bytes: 1024,
            max_stream_item_bytes: 128,
            max_chunk_bytes: 1024,
            max_header_bytes: 512,
            compatibility_profiles: vec![CompatibilityProfile::LegacyBinaryStringV1],
            ..Default::default()
        };
        let old = rust_http::plan_http_v2(c.clone(), &selection, config.clone()).unwrap();
        let new = rust_http::plan_http_v3(c, &selection, config).unwrap();
        assert_eq!(new.codecs().validation_version(), version);
        assert_eq!(new.protocol().capabilities(), old.protocol().capabilities());
        assert_eq!(
            new.protocol().capabilities().adapter(),
            "rust-http-protocol-v1"
        );
        let package = PackageConfig {
            name: "http-profile-sdk".into(),
            version: "1.2.3".into(),
        };
        let expected = rust_http::emit_http(&old, &package).unwrap();
        let actual = rust_http::emit_http(&new, &package).unwrap();
        let differences = actual
            .iter()
            .filter(|file| !expected.contains(file))
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert!(
            actual == expected,
            "all files, including manifest and examples, must match: {differences:?}"
        );
    }
}

#[test]
fn rust_v3_http_profile_capture_tracks_selected_resource_admission() {
    use suspect_codegen::{
        backend::{Backend, GenerationOptions, TargetConfig},
        compatibility::{self, PlanStatus},
        http_protocol::CompatibilityProfile,
    };
    let mut nested = profile_document(
        json!({"$ref":"#/components/schemas/Outer/$defs/Nested"}),
        json!(7),
    );
    nested["components"]["schemas"]["Outer"] =
        json!({"$dynamicAnchor":"outer","type":"boolean","$defs":{"Nested":{"type":"integer"}}});
    let target = TargetConfig {
        backend: Backend::RustHttp,
        package_name: "http-profile-sdk".into(),
        package_version: "1.2.3".into(),
        import_name: None,
    };
    let options = GenerationOptions {
        compatibility_profiles: [CompatibilityProfile::LegacyBinaryStringV1]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let resource = contract("https://physical.example", library());
    for (c, operation_ids, profile) in [
        (
            profile_contract(profile_document(json!({"type":"string"}), json!("value"))),
            vec!["echo".to_owned()],
            "rust-http-protocol-v1",
        ),
        (
            profile_contract(metadata_only_byte_document()),
            vec!["echo".to_owned()],
            "rust-http-protocol-v1",
        ),
        (
            profile_contract(nested),
            vec!["echo".to_owned()],
            "rust-http-resources-v3",
        ),
        (
            resource,
            vec!["putInteger".to_owned()],
            "rust-http-resources-v3",
        ),
    ] {
        let selection = c
            .operations()
            .filter(|op| {
                op.operation_id()
                    .is_some_and(|id| operation_ids.iter().any(|selected| selected == id))
            })
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let plan = rust_http::plan_http_v3(
            c.clone(),
            &selection,
            HttpConfig {
                compatibility_profiles: options.compatibility_profiles.iter().copied().collect(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(plan.protocol().capabilities().adapter(), profile);
        if profile == "rust-http-resources-v3" {
            assert_eq!(plan.codecs().validation_version(), OwnedProgram::V3_VERSION);
            assert!(rust_http::plan_http_v2(c.clone(), &selection, Default::default()).is_err());
        }
        let snapshot = compatibility::snapshot_with_options(
            c,
            &operation_ids,
            std::slice::from_ref(&target),
            &options,
        )
        .unwrap();
        let native = &snapshot.native[0];
        assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
        assert_eq!(native.runtime.profile, profile);
        assert_eq!(native.generation, options);
        assert_eq!(native.operations.len(), plan.operations().len());
        for op in plan.operations() {
            let captured = native
                .operations
                .iter()
                .find(|captured| captured.operation_id == op.operation_id)
                .unwrap();
            assert_eq!(captured.descriptor, op.interface());
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    target: String,
    content_type: String,
    body: String,
}
struct Wire {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Wire {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let records = requests.clone();
        let stopped = stop.clone();
        let task = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut buffer = [0u8; 1024];
                        let header_end = loop {
                            if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                                break index + 4;
                            }
                            let n = stream.read(&mut buffer).unwrap();
                            assert!(n > 0);
                            bytes.extend_from_slice(&buffer[..n]);
                            assert!(bytes.len() < 65536);
                        };
                        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
                        let target = headers
                            .lines()
                            .next()
                            .unwrap()
                            .split(' ')
                            .nth(1)
                            .unwrap()
                            .to_owned();
                        let header = |name: &str| {
                            headers
                                .lines()
                                .filter_map(|line| line.split_once(':'))
                                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                                .map(|(_, value)| value.trim().to_owned())
                        };
                        let content_type = header("content-type").unwrap_or_default();
                        let length: usize = header("content-length")
                            .map(|v| v.parse().unwrap())
                            .unwrap_or(0);
                        assert!(header("authorization").is_none());
                        assert!(header("transfer-encoding").is_none());
                        assert!(length < 65536);
                        while bytes.len() < header_end + length {
                            let n = stream.read(&mut buffer).unwrap();
                            assert!(n > 0);
                            bytes.extend_from_slice(&buffer[..n]);
                        }
                        let body = std::str::from_utf8(&bytes[header_end..header_end + length])
                            .unwrap()
                            .to_owned();
                        let response = if target.ends_with("/invalid-response") {
                            "{\"payload\":\"fallback\"}".to_owned()
                        } else {
                            body.clone()
                        };
                        records.lock().unwrap().push(Request {
                            target,
                            content_type,
                            body,
                        });
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("v3 wire server: {error}"),
                }
            }
        });
        Self {
            base,
            requests,
            stop,
            task: Some(task),
        }
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take()
            && let Err(error) = task.join()
            && !thread::panicking()
        {
            std::panic::resume_unwind(error);
        }
    }
}
fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut c = Command::new("cargo");
    c.args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        c.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    c
}
fn checked(command: &mut Command, root: &Path) {
    let out = command.output().unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        log,
        "{command:?}\nstatus: {}\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .unwrap();
    assert!(
        out.status.success(),
        "retained v3 SDK attempt {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn target() -> PathBuf {
    std::env::var_os("SUSPECT_RUST_V3_HTTP_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-http-v3-msrv"
                } else {
                    "../../target/native-rust-http-v3"
                },
            )
        })
}
fn symbol(plan: &HttpPlan, source: &SchemaId) -> String {
    plan.codecs()
        .models()
        .symbols()
        .iter()
        .find(|s| s.source() == source && s.role() == rust_models::RepresentationRole::Model)
        .unwrap()
        .name()
        .to_owned()
}
fn install(
    plan: &HttpPlan,
    name: &str,
    root: &Path,
    target: &Path,
    vendor: &Path,
    docs: bool,
) -> String {
    let files = rust_http::emit_http(
        plan,
        &PackageConfig {
            name: name.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    let directory = root.join(name);
    suspect_codegen::write_files(&files, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    if docs {
        checked(
            cargo("check", &manifest, target).arg("--no-default-features"),
            root,
        );
        checked(
            cargo("test", &manifest, target).args(["--doc", "--all-features"]),
            root,
        );
        checked(
            cargo("doc", &manifest, target).args(["--no-deps", "--all-features"]),
            root,
        );
        checked(
            cargo("run", &manifest, target).args(["--example", "validated", "--features", "http"]),
            root,
        );
    }
    checked(
        cargo("package", &manifest, target).args(["--allow-dirty", "--no-verify"]),
        root,
    );
    let archive = target.join(format!("package/{name}-0.0.0.crate"));
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(vendor),
        root,
    );
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()))
}

#[test]
#[ignore = "requires native Cargo/current or 1.88, pinned reqwest and tar"]
fn installed_rust_v3_sdk_preserves_dynamic_models_examples_and_real_wire() {
    let wire = Wire::start();
    let p = plan(&wire.base, Default::default());
    let root = tempfile::Builder::new()
        .prefix("rust-v3-sdk-")
        .tempdir()
        .unwrap()
        .keep();
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let target = target();
    let digest = install(
        &p,
        "resource-sdk",
        &root,
        &target,
        &consumer.join("vendor"),
        true,
    );
    let mut config = HttpConfig::default();
    config.codecs.schema.max_evaluation_steps = 2;
    let limited = plan(&wire.base, config);
    let limited_digest = install(
        &limited,
        "limited-resource-sdk",
        &root,
        &target,
        &consumer.join("vendor"),
        false,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"resource-sdk-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"resource-sdk\",path=\"vendor/resource-sdk-0.0.0\",features=[\"reqwest-rustls\"]}\nlimited={package=\"limited-resource-sdk\",path=\"vendor/limited-resource-sdk-0.0.0\",features=[\"reqwest-rustls\"]}\ntokio={version=\"=1.53.1\",features=[\"macros\",\"rt\"]}\n").unwrap();
    let integer = source(LIBRARY_PHYSICAL, &["$defs", "IntegerEnvelope"]);
    let nullable = source(LIBRARY_PHYSICAL, &["$defs", "NullableEnvelope"]);
    let generic = source(LIBRARY_PHYSICAL, &["$defs", "Generic"]);
    let text = CONSUMER
        .replace("__GENERIC__", &symbol(&p, &generic))
        .replace("__INTEGER__", &symbol(&p, &integer))
        .replace("__NULLABLE__", &symbol(&p, &nullable));
    let text = format!(
        "#[cfg(test)] const PHYSICAL: &str={LIBRARY_PHYSICAL:?};\n#[cfg(test)] const ENTRY: &str={:?};\n{text}",
        format!("{}/source/deep/api.json", wire.base)
    );
    std::fs::write(consumer.join("src/lib.rs"), text).unwrap();
    checked(
        cargo(
            "test",
            &consumer.join("Cargo.toml"),
            &target.join("installed").join(digest).join(limited_digest),
        )
        .args(["--", "--test-threads=1"]),
        &root,
    );
    assert_eq!(
        *wire.requests.lock().unwrap(),
        vec![
            Request {
                target: "/source/wire%2fV3/integers".into(),
                content_type: "application/json".into(),
                body: "{\"label\":\"native\",\"payload\":9007199254740993}".into()
            },
            Request {
                target: "/source/wire%2fV3/nullable".into(),
                content_type: "application/json".into(),
                body: "{\"direct\":null,\"payload\":null}".into()
            },
            Request {
                target: "/source/wire%2fV3/invalid-response".into(),
                content_type: String::new(),
                body: String::new()
            },
            Request {
                target: "/source/wire%2fV3/invalid-response".into(),
                content_type: String::new(),
                body: String::new()
            },
        ]
    );
    eprintln!("native Rust v3 SDK retained {}", root.display());
}

const CONSUMER: &str = r###"
/// Exact dynamic payloads are explicit JSON containers, not the fallback's type.
/// ```compile_fail
/// sdk::models::__GENERIC__::new(9007199254740993.0f64);
/// ```
/// The operation retains its source-bound object constructor.
/// ```compile_fail
/// sdk::operations::put_integer::PutInteger::new(sdk::Nullable::Value("fallback".to_owned()));
/// ```
/// Indexed resource metadata is immutable at the execution boundary.
/// ```compile_fail
/// sdk::validation::resources()[0].canonical_uri = "https://logical.example/rebound";
/// ```
pub struct NativeTypes;

#[cfg(test)]
mod tests {
    use super::{ENTRY,PHYSICAL};
    use sdk::{JsonNonNullValue as J,Nullable as N,Presence,codecs::{CodecError,__INTEGER__Codec as IntegerCodec,__NULLABLE__Codec as NullableCodec,__GENERIC__Codec as GenericCodec},models::__GENERIC__ as Generic};
    fn json(text:&str)->sdk::JsonValue{sdk::parse_json(text,Default::default()).unwrap()}
    fn invalid(error:CodecError,pointer:&str,path:&str) {
        let CodecError::Invalid(findings)=error else{panic!("ordinary invalidity must survive conversion: {error:?}")};
        assert!(findings.iter().any(|f|f.document==PHYSICAL&&f.pointer==pointer&&f.instance_path==path),"{findings:?}");
        assert!(findings.iter().all(|f|!f.document.starts_with("https://logical.example/")));
    }
    #[test]
    fn mutable_models_preserve_context_sensitive_unions_direct_refs_and_null() {
        let mut generic=GenericCodec::decode(r#"{"payload":"fallback"}"#).unwrap();
        assert!(matches!(&generic.payload,N::Value(J::String(v)) if v=="fallback"));
        generic.payload=json("1");assert!(matches!(GenericCodec::encode(&generic),Err(CodecError::Invalid(_))));
        // The same Generic.payload union must be checked under IntegerEnvelope,
        // not by a fresh standalone union trial that selects the string fallback.
        let mut integer=IntegerCodec::decode(r#"{"payload":9007199254740993,"direct":1}"#).unwrap();
        let N::Value(value)=&mut integer else{panic!("object carrier")};
        assert!(matches!(&value.payload,N::Value(J::Number(n)) if n.as_str()=="9007199254740993"));
        value.direct=Presence::Value(J::String("wrong binding".into()));
        invalid(IntegerCodec::encode(&integer).unwrap_err(),"/$defs/IntegerEnvelope/$defs/Slot/type","/direct");
        let N::Value(value)=&mut integer else{panic!()};value.direct=Presence::Absent;
        assert_eq!(IntegerCodec::encode(&integer).unwrap(),r#"{"payload":9007199254740993}"#);
        let N::Value(value)=&mut integer else{panic!()};value.payload=json("-1");
        assert!(matches!(IntegerCodec::encode(&integer),Err(CodecError::Invalid(_))));
        assert!(matches!(IntegerCodec::decode(r#"{"payload":"fallback"}"#),Err(CodecError::Invalid(_))));
        let nullable=NullableCodec::decode(r#"{"payload":null,"direct":null}"#).unwrap();
        let N::Value(value)=&nullable else{panic!("object carrier")};assert!(matches!(value.payload,N::Null));assert!(matches!(value.direct,Presence::Null));
        assert_eq!(NullableCodec::encode(&nullable).unwrap(),r#"{"direct":null,"payload":null}"#);
        // A previous different entry cannot supply a persistent dynamic binding.
        assert!(GenericCodec::decode(r#"{"payload":"after-other-entry"}"#).is_ok());
        assert!(matches!(GenericCodec::decode(r#"{"payload":null}"#),Err(CodecError::Invalid(_))));
    }
    #[test]
    fn emitted_metadata_separates_aliases_from_physical_ownership() {
        let resources=sdk::validation::resources();let scopes=sdk::validation::node_scopes();
        let physical=resources.iter().find(|r|r.canonical_uri=="https://logical.example/models/library.json").unwrap();
        assert_eq!(physical.source.document,PHYSICAL);assert_eq!(physical.source.pointer,"");
        assert!(physical.aliases.contains(&"https://requested.example/v3/library.json"));assert!(physical.aliases.contains(&PHYSICAL));
        assert_eq!(physical.declaration_source.unwrap().pointer,"/$id");
        let integer=resources.iter().find(|r|r.canonical_uri=="https://logical.example/models/integers").unwrap();
        assert_eq!(integer.source.document,PHYSICAL);assert_eq!(integer.source.pointer,"/$defs/IntegerEnvelope");
        let binding=&integer.dynamic_anchors[0];assert_eq!(binding.name,"value");assert_eq!(binding.source.pointer,"/$defs/IntegerEnvelope/$defs/Slot/$dynamicAnchor");
        assert_eq!(resources[scopes[binding.target].resource].source,integer.source);
        assert_eq!(scopes[binding.target].canonical_address,"https://logical.example/models/integers#/$defs/Slot");
        assert!(resources.iter().all(|r|r.source.document==PHYSICAL||r.source.document==ENTRY));
        let op=&sdk::operations::put_integer::OPERATION;
        assert_eq!(op.source.document,ENTRY);assert_eq!(op.provenance.terminal_resource.unwrap().canonical_uri,"https://logical.example/v3/api.json");
        assert_eq!(op.servers[0].document_base.document,ENTRY);
    }
    #[tokio::test(flavor="current_thread")]
    async fn reqwest_uses_root_context_on_both_directions_and_never_sends_invalid_models() {
        use sdk::{Client,Credentials,operations::{put_integer,put_nullable,invalid_response},http::SdkErrorKind};
        let client=Client::with_reqwest(Credentials::new()).unwrap();
        let mut body=Generic::new(json("9007199254740993"));body.label=Some("native".into());
        let response=client.put_integer(put_integer::PutInteger::new(N::Value(body))).await.unwrap().into_response();
        assert_eq!(response.status,200);let N::Value(value)=response.data else{panic!("response object")};
        assert!(matches!(&value.payload,N::Value(J::Number(n)) if n.as_str()=="9007199254740993"));
        let mut body=Generic::new(N::Null);body.direct=Presence::Null;
        let response=client.put_nullable(put_nullable::PutNullable::new(N::Value(body))).await.unwrap().into_data();
        let N::Value(value)=response else{panic!("response object")};assert!(matches!(value.payload,N::Null));
        let body=N::Value(Generic::new(json("\"fallback\"")));
        let put_integer::PutIntegerError::Sdk(error)=client.put_integer(put_integer::PutInteger::new(body)).await.unwrap_err() else{panic!("invalid request")};
        assert_eq!(error.kind,SdkErrorKind::RequestValidation);assert!(error.status.is_none());assert_eq!(error.source.document,ENTRY);
        assert!(matches!(error.cause.unwrap().downcast::<CodecError>().unwrap().as_ref(),CodecError::Invalid(_)));
        let invalid_response::InvalidResponseError::Sdk(error)=client.invalid_response_default().await.unwrap_err() else{panic!("invalid response")};
        assert_eq!(error.kind,SdkErrorKind::ResponseDecoding);assert_eq!(error.status,Some(200));assert_eq!(error.raw_capture,br#"{"payload":"fallback"}"#);
        assert!(matches!(error.cause.unwrap().downcast::<CodecError>().unwrap().as_ref(),CodecError::Invalid(_)));
        use limited::{Client as LimitedClient,Credentials as LimitedCredentials,models::__GENERIC__ as LimitedGeneric,operations::{put_integer as lp,invalid_response as lr},codecs::CodecError as LimitedCodecError,http::SdkErrorKind as LimitedKind};
        let client=LimitedClient::with_reqwest(LimitedCredentials::new()).unwrap();
        let body=limited::Nullable::Value(LimitedGeneric::new(limited::parse_json("1",Default::default()).unwrap()));
        let lp::PutIntegerError::Sdk(error)=client.put_integer(lp::PutInteger::new(body)).await.unwrap_err() else{panic!("resource request failure")};
        assert_eq!(error.kind,LimitedKind::ResourceLimit);assert!(error.status.is_none());
        assert!(matches!(error.cause.unwrap().downcast::<LimitedCodecError>().unwrap().as_ref(),LimitedCodecError::EvaluationFailure(_)));
        let lr::InvalidResponseError::Sdk(error)=client.invalid_response_default().await.unwrap_err() else{panic!("resource response failure")};
        assert_eq!(error.kind,LimitedKind::ResourceLimit);assert_eq!(error.status,Some(200));
        assert!(matches!(error.cause.unwrap().downcast::<LimitedCodecError>().unwrap().as_ref(),LimitedCodecError::EvaluationFailure(_)));
    }
}
"###;
