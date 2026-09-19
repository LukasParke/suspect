//! Focused native document-base adoption; no schema-resource execution opt-in.
use super::*;
use crate::http_protocol as wire;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-cpp-server-base-gates");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("physical-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn provided(entry: &str, documents: &[(String, String, Value)]) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(requested, effective, document)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec(document).unwrap(),
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
fn documents(entry_origin: &str, fragment_origin: &str) -> Vec<(String, String, Value)> {
    let mut paths = serde_json::Map::new();
    for (path, name) in [
        ("/inherited", "Inherited"),
        ("/empty", "Empty"),
        ("/declared", "Declared"),
        ("/variable", "Variable"),
        ("/oauth", "OAuth"),
        ("/oidc", "Oidc"),
        ("/bytes", "Bytes"),
    ] {
        paths.insert(path.into(),json!({"$ref":format!("https://logical.example/parts/ops.json#/components/pathItems/{name}")}));
    }
    let entry = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/api.json","info":{"title":"Physical base entry","version":"1"},"paths":paths});
    let response = json!({"204":{"description":"No content"}});
    let fragment = json!({"openapi":"3.2.0","$self":"https://logical.example/parts/ops.json","info":{"title":"Physical definitions","version":"1"},"components":{
        "securitySchemes":{
            "oauth":{"type":"oauth2","flows":{"authorizationCode":{"authorizationUrl":"../authorize","tokenUrl":"tokens","scopes":{"read":"read"}}}},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":".well-known/openid-configuration"}
        },
        "pathItems":{
            "Inherited":{"get":{"operationId":"inherited","responses":{"204":{"content":{"application/json":{"schema":{"$id":"https://logical.example/unused/schema","$dynamicAnchor":"node","type":"object"}}}}}}},
            "Empty":{"get":{"operationId":"empty","servers":[],"responses":response}},
            "Declared":{"get":{"operationId":"declared","servers":[{"url":"../{segment}/%2e%2E/Api%2Fv1//","variables":{"segment":{"default":"Svc","enum":["Svc","Alt"]}}}],"responses":response}},
            "Variable":{"get":{"operationId":"variable","servers":[{"url":"{endpoint}","variables":{"endpoint":{"default":format!("{fragment_origin}/Variable/%2e%2E/Default%2FCase")}}}],"responses":response}},
            "OAuth":{"get":{"operationId":"oauthCall","servers":[{"url":"/Protected/%2e/Api"}],"security":[{"oauth":["read"]}],"responses":response}},
            "Oidc":{"get":{"operationId":"oidcCall","security":[{"oidc":["openid"]}],"responses":response}},
            "Bytes":{"get":{"operationId":"bytes","responses":{"200":{"content":{"application/octet-stream":{"schema":{"$id":"https://logical.example/bytes/schema","maxLength":3}}}}}}}
        }
    }});
    vec![
        (
            "https://requested.invalid/api.json".into(),
            format!("{entry_origin}/docs/v1/api.json"),
            entry,
        ),
        (
            "https://requested.invalid/parts.json".into(),
            format!("{fragment_origin}/files/parts.json"),
            fragment,
        ),
    ]
}
fn tool(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
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
        "{command:?}\n{}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "installed C++ libcurl physical-document-base/redirect-provenance/encoded-path witness"]
fn native_document_relative_servers() {
    let root = root();
    let a = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let b = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    a.set_nonblocking(true).unwrap();
    b.set_nonblocking(true).unwrap();
    let entry = format!("http://{}", a.local_addr().unwrap());
    let fragment = format!("http://{}", b.local_addr().unwrap());
    let docs = documents(&entry, &fragment);
    std::fs::write(
        root.join("source-documents.json"),
        serde_json::to_string_pretty(&docs).unwrap(),
    )
    .unwrap();
    let contract = provided(&docs[0].0, &docs);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let config = SdkConfig::default();
    let capabilities = capabilities(&config).with(wire::Capability::DocumentRelativeServers);
    assert!(capabilities.supports(wire::Capability::DocumentRelativeServers));
    let plan = plan_sdk(contract.clone(), &selected, config).unwrap_or_else(|e| panic!("{e:#?}"));
    assert!(plan.protocol().codec_roots().is_empty());
    assert!(plan.protocol().codec_schema_closure().is_empty());
    for operation in plan.protocol().operations() {
        assert_eq!(
            operation.source().use_site().source().document().as_str(),
            format!("{entry}/docs/v1/api.json")
        );
        assert_eq!(
            operation.source().terminal().source().document().as_str(),
            format!("{fragment}/files/parts.json")
        );
        assert_eq!(
            operation.source().terminal_resource().unwrap().base_uri(),
            "https://logical.example/parts/ops.json"
        );
        assert_eq!(
            operation.source().references().len(),
            operation.source().reference_resources().len()
        );
        let server = &operation.servers().candidates()[0];
        let inherited = matches!(
            operation.operation_id().unwrap().value().as_str(),
            "inherited" | "oidcCall" | "bytes"
        );
        assert_eq!(
            server.document_base().source().document().as_str(),
            if inherited {
                format!("{entry}/docs/v1/api.json")
            } else {
                format!("{fragment}/files/parts.json")
            }
        );
        assert_eq!(server.url_base(), wire::ApiUrlBase::ServerDocument);
        if operation.operation_id().unwrap().value() == "declared" {
            assert_eq!(
                server.resolve_document_url(&BTreeMap::new()).unwrap(),
                format!("{fragment}/Svc/%2e%2E/Api%2Fv1//")
            );
        }
    }
    super::v2_tests::package(&plan, &root);
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("main.cpp"),
        include_str!("tests/native_server_base.cpp"),
    )
    .unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"),"cmake_minimum_required(VERSION 3.24)\nproject(DocumentBaseConsumer LANGUAGES CXX)\nfind_package(generated_sdk CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE generated_sdk::generated_sdk)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n").unwrap();
    let cmake = tool("SUSPECT_CPP_CMAKE", "cmake");
    checked(
        Command::new(&cmake)
            .arg("-S")
            .arg(&consumer)
            .arg("-B")
            .arg(root.join("build-consumer"))
            .arg(format!(
                "-DCMAKE_PREFIX_PATH={}",
                root.join("install").display()
            ))
            .arg(format!(
                "-DCMAKE_CXX_COMPILER={}",
                tool("SUSPECT_CPP_CXX", "clang++").display()
            )),
        &root,
    );
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.join("build-consumer")),
        &root,
    );
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let log = root.join("wire.json");
    let server = std::thread::spawn(move || {
        let mut records = Vec::new();
        while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
            for (origin, listener) in [("entry", &a), ("fragment", &b)] {
                let mut stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                }
                let request = String::from_utf8(bytes).unwrap();
                let first = request.lines().next().unwrap().to_owned();
                records.push(json!({"origin":origin,"request":first,"headers":request}));
                if first == "GET /bytes HTTP/1.1" {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\nConnection: close\r\n\r\n\0\xff\x01").unwrap();
                } else {
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                        .unwrap();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        std::fs::write(log, serde_json::to_string_pretty(&records).unwrap()).unwrap();
        records
    });
    let output = Command::new(root.join("build-consumer/consumer"))
        .arg(&entry)
        .arg(&fragment)
        .output()
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let records = server.join().unwrap();
    std::fs::write(
        root.join("native.log"),
        [output.stdout.clone(), output.stderr.clone()].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = [
        ("entry", "/inherited"),
        ("fragment", "/empty"),
        ("fragment", "/Svc/%2e%2E/Api%2Fv1//declared"),
        ("entry", "/caller/Svc/%2e%2E/Api%2Fv1//declared"),
        ("fragment", "/Variable/%2e%2E/Default%2FCase/variable"),
        ("fragment", "/Relative/%2Ekeep/variable"),
        ("entry", "/Override/%2e%2E/v2/inherited"),
        ("fragment", "/Protected/%2e/Api/oauth"),
        ("entry", "/Override/%2e%2E/v2/oauth"),
        ("entry", "/oidc"),
        ("entry", "/bytes"),
    ];
    assert_eq!(records.len(), expected.len());
    for (record, (origin, path)) in records.iter().zip(expected) {
        assert_eq!(record["origin"], origin);
        assert_eq!(record["request"], format!("GET {path} HTTP/1.1"));
    }
    assert!(
        records[7]["headers"]
            .as_str()
            .unwrap()
            .contains("Authorization: Bearer provider-token")
    );
    assert!(
        records[9]["headers"]
            .as_str()
            .unwrap()
            .contains("Authorization: Bearer oidc-token")
    );
    println!(
        "C++ physical-document-base native witness: {}",
        root.display()
    );
}

#[test]
fn schema_resource_and_dynamic_execution_require_capabilities() {
    let mut docs = documents(
        "http://physical-entry.example",
        "http://physical-defs.example",
    );
    let cap = wire::Capabilities::for_adapter(
        "cpp-static-only-control",
        capabilities(&Default::default())
            .enabled()
            .iter()
            .copied()
            .filter(|c| {
                !matches!(
                    c,
                    wire::Capability::SchemaResources | wire::Capability::DynamicSchemaReferences
                )
            }),
    );
    docs[1].2["components"]["pathItems"]["Bytes"]["get"]["responses"]["200"]["content"] = json!({"application/json":{"schema":{"$id":"https://logical.example/schema","$dynamicAnchor":"node","$dynamicRef":"#node"}}});
    let contract = provided(&docs[0].0, &docs);
    let selected = contract
        .operations()
        .filter(|o| o.operation_id() == Some("bytes"))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let refused = wire::plan(&contract, &selected, cap);
    assert!(!refused.is_admitted());
    for capability in [
        wire::Capability::SchemaResources,
        wire::Capability::DynamicSchemaReferences,
    ] {
        let diagnostic = refused
            .diagnostics()
            .iter()
            .find(|d| d.capability() == Some(capability))
            .unwrap();
        assert_eq!(
            diagnostic.source().source().document().as_str(),
            "http://physical-defs.example/files/parts.json"
        );
        assert!(diagnostic.resource_context().is_some());
        assert!(!diagnostic.source().span().is_empty());
    }
}
