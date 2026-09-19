//! Installed SDK and physical-document-base proofs over the same native planner.
use super::validation_v3_support as support;
use super::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use suspect_schema::{OwnedCompiler, OwnedProgram};

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}

fn proof_plan(contract: Arc<Contract>) -> SdkPlan {
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    for capability in [
        http_protocol::Capability::DocumentRelativeServers,
        http_protocol::Capability::SchemaResources,
        http_protocol::Capability::DynamicSchemaReferences,
    ] {
        assert!(plan.protocol.capabilities().supports(capability));
    }
    plan
}

fn sdk_contract(physical: &str) -> Arc<Contract> {
    let schemas = json!({
        "Tree":{"$id":"schemas/tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}},"example":{"data":1,"children":[{"data":2}]}},
        "Strict":{"$id":"schemas/strict","$dynamicAnchor":"node","$ref":"tree","unevaluatedProperties":false,"example":{"data":1,"children":[{"data":2}]}},
        "Fallback":{"$id":"urn:choice-base","$defs":{"Slot":{"$dynamicAnchor":"value","type":"string"}}},
        "Choice":{"$id":"schemas/choice","$defs":{"Override":{"$dynamicAnchor":"value","type":["integer","null"]}},"type":"object","properties":{"selected":{"$dynamicRef":"urn:choice-base#value"}},"additionalProperties":false,"example":{"selected":null}},
        "DynamicUnion":{"$id":"schemas/union","$defs":{"Override":{"$dynamicAnchor":"value","type":"integer"}},"oneOf":[{"$dynamicRef":"urn:choice-base#value"},{"type":"boolean"}],"example":7},
        "Record":{"$id":"schemas/record","type":"object","properties":{"id":{"type":"string"},"amount":{"$ref":"threshold"},"note":{"type":["string","null"]}},"required":["id","amount"],"additionalProperties":false,"$defs":{"Threshold":{"$id":"threshold","type":"integer","minimum":9007199254740993u64}},"example":{"id":"r","amount":9007199254740993u64}},
        "Cycle":{"$id":"schemas/cycle","$dynamicAnchor":"cycle","$dynamicRef":"#cycle"}
    });
    let mut paths = serde_json::Map::new();
    for (path, operation, reference, example) in [
        (
            "/tree",
            "roundTripStrictTree",
            "https://logical.swift.test/schemas/strict",
            json!({"children":[{"data":1}]}),
        ),
        (
            "/open",
            "roundTripOpenTree",
            "https://logical.swift.test/schemas/tree",
            json!({"extra":true}),
        ),
        (
            "/choice",
            "roundTripChoice",
            "https://logical.swift.test/schemas/choice",
            json!({"selected":null}),
        ),
        (
            "/union",
            "roundTripDynamicUnion",
            "https://logical.swift.test/schemas/union",
            json!(7),
        ),
        (
            "/record",
            "roundTripRecord",
            "https://logical.swift.test/schemas/record",
            json!({"id":"r","amount":9007199254740993u64}),
        ),
        (
            "/detached",
            "roundTripDetached",
            "urn:detached#/$defs/Entry",
            json!(7),
        ),
        (
            "/escaped",
            "roundTripEscaped",
            "urn:escaped#/$defs/a~1b~0%25%20%23%C3%A9",
            json!(9007199254740993u64),
        ),
    ] {
        let content = json!({"application/json":{"schema":{"$ref":reference},"example":example}});
        paths.insert(path.into(), json!({"post":{"operationId":operation,"requestBody":{"required":true,"content":content},"responses":{"200":{"description":"Echoes the checked value","content":content}}}}));
    }
    paths.insert("/cycle".into(), json!({"post":{"operationId":"triggerResourceCycle","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"https://logical.swift.test/schemas/cycle"}}}},"responses":{"204":{"description":"Never reached by an incomplete request"}}}}));
    let document = json!({"openapi":"3.2.0","$self":"https://logical.swift.test/api.json#revision","info":{"title":"Swift v3 installed source codecs","version":"1"},"servers":[{"url":"../service/v1"}],"paths":paths,"components":{"schemas":schemas}});
    support::provided("https://download.swift.test/api.json", vec![
        ("https://download.swift.test/api.json", physical, document.to_string().into_bytes()),
        ("https://download.swift.test/detached.json", "https://cdn.swift.test/detached.json", json!({"$id":"urn:detached","const":false,"$defs":{"Override":{"$dynamicAnchor":"value","type":"integer"},"Entry":{"$ref":"urn:slot#/$defs/DynamicSlot"}}}).to_string().into_bytes()),
        ("https://download.swift.test/slot.json", "https://cdn.swift.test/slot.json", json!({"$id":"urn:slot","$defs":{"Initial":{"$dynamicAnchor":"value","type":"string"},"DynamicSlot":{"$dynamicRef":"#value"}}}).to_string().into_bytes()),
        ("https://download.swift.test/escaped.json", "https://cdn.swift.test/releases/escaped.json", json!({"$id":"urn:escaped","$defs":{"a/b~% #é":{"type":"number","minimum":9007199254740993u64}}}).to_string().into_bytes()),
    ])
}

fn document_contract(base: &str) -> Arc<Contract> {
    let physical = format!("{base}/specs/releases/api.json");
    let parts = format!("{base}/artifacts/nested/parts.json");
    let links = format!("{base}/artifacts/nested/links.json");
    let response = json!({"200":{"description":"OK"}});
    let document = json!({"openapi":"3.2.0","$self":"https://logical.swift.test/api.json#revision","info":{"title":"Physical server source witnesses","version":"1"},
        "servers":[{"url":"../service/{version}","variables":{"version":{"default":"v1","enum":["v1","v2"]}}}],
        "paths":{
            "/root":{"get":{"operationId":"rootServer","responses":response}},
            "/mounted":{"$ref":"parts.json#/components/pathItems/Mounted"},
            "/local":{"$ref":"local.json#/components/pathItems/Local"},
            "/encoded":{"get":{"operationId":"encodedServer","servers":[{"url":"%2e%2e/Api%2Fv1"}],"responses":response}},
            "/absolute":{"get":{"operationId":"absoluteServer","servers":[{"url":"https://EXAMPLE.TEST/a/../API%2Fv1"}],"responses":response}},
            "/network":{"get":{"operationId":"networkServer","servers":[{"url":"//OTHER.TEST/base/../API"}],"responses":response}},
            "/variable":{"get":{"operationId":"variableServer","servers":[{"url":"./{segment}","variables":{"segment":{"default":"plain"}}}],"responses":response}},
            "/default":{"get":{"operationId":"defaultServer","servers":[],"responses":response}},
            "/oauth":{"get":{"operationId":"oauthServer","security":[{"oauth":[]}],"responses":response}},
            "/oidc":{"get":{"operationId":"oidcServer","security":[{"oidc":[]}],"responses":response}},
            "/link":{"get":{"operationId":"linkServer","responses":{"200":{"description":"Metadata only","links":{"follow":{"$ref":"links.json#/components/links/Follow"}}}}}}
        },
        "components":{"securitySchemes":{
            "oauth":{"type":"oauth2","oauth2MetadataUrl":"../metadata","flows":{"clientCredentials":{"tokenUrl":"./token","scopes":{}}}},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"./openid"}
        }}
    });
    let external = json!({"openapi":"3.2.0","$self":"https://logical.swift.test/parts.json","info":{"title":"Mounted servers","version":"1"},"paths":{},"components":{"pathItems":{"Mounted":{"servers":[{"url":"../owned/{version}","variables":{"version":{"default":"v1","enum":["v1","v2"]}}}],"get":{"operationId":"mountedServer","responses":response}}}}});
    let local = json!({"openapi":"3.2.0","$self":"https://logical.swift.test/local.json","info":{"title":"File base is not an origin","version":"1"},"paths":{},"components":{"pathItems":{"Local":{"servers":[{"url":"./local"}],"get":{"operationId":"localServer","responses":response}}}}});
    let link = json!({"openapi":"3.2.0","$self":"https://logical.swift.test/links.json","info":{"title":"Link server source","version":"1"},"paths":{},"components":{"links":{"Follow":{"operationRef":"https://logical.swift.test/api.json#/paths/~1root/get","server":{"url":"../linked"}}}}});
    support::provided(
        "https://download.swift.test/api.json",
        vec![
            (
                "https://download.swift.test/api.json",
                &physical,
                document.to_string().into_bytes(),
            ),
            (
                "https://download.swift.test/parts.json",
                &parts,
                external.to_string().into_bytes(),
            ),
            (
                "file:///SwiftProvided/local.json",
                "file:///SwiftProvided/local.json",
                local.to_string().into_bytes(),
            ),
            (
                "https://download.swift.test/links.json",
                &links,
                link.to_string().into_bytes(),
            ),
        ],
    )
}

#[test]
fn resource_plan_has_contextual_carriers_actual_roots_and_physical_metadata() {
    let plan = proof_plan(sdk_contract(
        "http://127.0.0.1:12345/specs/releases/api.json",
    ));
    assert_eq!(plan.program.version, OwnedProgram::V3_VERSION);
    assert_eq!(plan.program.profile, OwnedProgram::V3_PROFILE);
    assert!(
        plan.program
            .resource_context
            .as_ref()
            .unwrap()
            .resources
            .iter()
            .any(
                |r| r.canonical_uri == "https://logical.swift.test/schemas/strict"
                    && r.source.document == "http://127.0.0.1:12345/specs/releases/api.json"
            )
    );
    for name in [
        "Strict",
        "Tree",
        "Choice",
        "DynamicUnion",
        "DynamicSlot",
        "Cycle",
    ] {
        let id = plan
            .models
            .names
            .iter()
            .find(|(_, n)| *n == name)
            .unwrap()
            .0;
        assert!(
            matches!(
                plan.models.declarations.get(id),
                Some(models::Declaration::Checked { .. })
            ),
            "{name}"
        );
        assert!(
            !plan.models.types[id].nullable,
            "contextual carriers retain the entire null domain"
        );
    }
    let record = plan
        .models
        .names
        .iter()
        .find(|(_, name)| *name == "Record")
        .unwrap()
        .0;
    assert!(matches!(
        plan.models.declarations[record],
        models::Declaration::Object { .. }
    ));
    assert!(
        !plan
            .protocol
            .codec_roots()
            .iter()
            .any(|id| id.pointer().ends_with("/$defs/Override"))
    );
    assert!(
        plan.protocol
            .codec_schema_closure()
            .iter()
            .any(|id| id.pointer().ends_with("/$defs/Override"))
    );
    let examples = plan
        .examples
        .operations()
        .iter()
        .flat_map(|o| &o.entries)
        .collect::<Vec<_>>();
    assert!(!examples.is_empty());
    assert!(examples.iter().any(|e| e.value == json!({"selected":null})
        && e.origin == crate::examples::ExampleOrigin::Declared));
    assert!(
        !plan
            .examples
            .diagnostics()
            .iter()
            .any(|d| d.code.contains("unsupported")),
        "{:?}",
        plan.examples.diagnostics()
    );
    let files = plan.render();
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("GettingStarted.md") && f.content.contains("Strict(value:"))
    );
}

#[test]
fn physical_server_adoption_keeps_logical_context_and_file_override_distinct() {
    let plan = proof_plan(document_contract("http://127.0.0.1:12345"));
    assert_eq!(plan.program.version, OwnedProgram::V1_VERSION);
    let operation = |name: &str| {
        plan.operations
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap()
    };
    let mounted = operation("mountedServer");
    let server = &mounted.wire.servers().candidates()[0];
    assert_eq!(
        server.document_base().source().document().as_str(),
        "http://127.0.0.1:12345/artifacts/nested/parts.json"
    );
    assert_eq!(
        server.resolve_document_url(&Default::default()).unwrap(),
        "http://127.0.0.1:12345/artifacts/owned/v1"
    );
    assert_eq!(
        mounted
            .wire
            .source()
            .terminal_resource()
            .unwrap()
            .base_uri(),
        "https://logical.swift.test/parts.json"
    );
    assert!(
        operation("localServer").wire.servers().candidates()[0]
            .resolve_document_url(&Default::default())
            .is_err()
    );
}

struct WireServer {
    base: String,
    done: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl WireServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let flag = done.clone();
        let capture = requests.clone();
        let worker = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let record = serve(&mut stream)
                            .unwrap_or_else(|error| json!({"error":error.to_string()}));
                        capture.lock().unwrap().push(record);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("wire listener: {error}"),
                }
            }
        });
        Self {
            base,
            done,
            requests,
            worker: Some(worker),
        }
    }
    fn finish(mut self, root: &Path) -> Vec<Value> {
        self.stop();
        let records = self.requests.lock().unwrap().clone();
        std::fs::write(
            root.join("wire-requests.json"),
            serde_json::to_string_pretty(&records).unwrap(),
        )
        .unwrap();
        assert!(
            records.iter().all(|r| r.get("error").is_none()),
            "{records:?}"
        );
        records
    }
    fn stop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Drop for WireServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(stream: &mut TcpStream) -> std::io::Result<Value> {
    // Accepted sockets inherit O_NONBLOCK on Darwin. The listener polls, while
    // each bounded request read uses a blocking socket with a finite timeout.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end = loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 65_536 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    let header = String::from_utf8(bytes[..header_end].to_vec())
        .map_err(|_| std::io::ErrorKind::InvalidData)?;
    let mut lines = header.split("\r\n");
    let head = lines.next().unwrap();
    let fields = head.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 3 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut headers = serde_json::Map::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or(std::io::ErrorKind::InvalidData)?;
        headers.insert(name.to_ascii_lowercase(), json!(value.trim()));
    }
    if headers.contains_key("transfer-encoding") {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let length: usize = headers
        .get("content-length")
        .and_then(Value::as_str)
        .unwrap_or("0")
        .parse()
        .map_err(|_| std::io::ErrorKind::InvalidData)?;
    if length > 65_536 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = &bytes[header_end..header_end + length];
    let response = if fields[1] == "/invalid/tree" {
        br#"{"children":[{"unexpected":1}]}"#.as_slice()
    } else {
        body
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )?;
    stream.write_all(response)?;
    Ok(
        json!({"method":fields[0],"target":fields[1],"headers":headers,"body":String::from_utf8_lossy(body)}),
    )
}

pub(super) fn installed(plan: &SdkPlan, root: &Path, native: &str, base: &str) {
    crate::write_files(&plan.render(), &root.join("sdk")).unwrap();
    support::checked(
        support::swift("test")
            .args(["--jobs", "4"])
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        root,
    );
    std::fs::create_dir_all(root.join("consumer/Tests/Consumer")).unwrap();
    std::fs::write(root.join("consumer/Package.swift"), "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"Consumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"Consumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(root.join("consumer/Tests/Consumer/Resources.swift"), native).unwrap();
    support::checked(
        support::swift("test")
            .args(["--jobs", "4"])
            .arg("--package-path")
            .arg(root.join("consumer"))
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"])
            .env("SUSPECT_SWIFT_RESOURCE_BASE", base),
        root,
    );
}

fn typing(root: &Path, positive: &str, probes: &[(&str, &str)]) {
    let modules = support::directory(&root.join("build/sdk"), "Modules").unwrap();
    let path = root.join("positive.swift");
    std::fs::write(
        &path,
        format!("import Foundation\nimport GeneratedSDK\n{positive}\n"),
    )
    .unwrap();
    support::checked(
        support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&path),
        root,
    );
    for (index, (source, expected)) in probes.iter().enumerate() {
        let path = root.join(format!("negative-{index}.swift"));
        std::fs::write(
            &path,
            format!("import Foundation\nimport GeneratedSDK\n{source}\n"),
        )
        .unwrap();
        let output = support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&path)
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        std::fs::write(
            root.join(format!("negative-{index}.log")),
            format!("status: {}\n{errors}", output.status),
        )
        .unwrap();
        assert!(
            !output.status.success() && errors.contains(expected),
            "probe {index} must fail for {expected:?}: {source}\n{errors}"
        );
        assert!(
            !errors.contains("no such module")
                && !errors.contains("unable to load standard library")
        );
    }
}

#[test]
#[ignore = "installed V3 source-bound carriers, actual HTTP wire calls, executable examples, positive-controlled types and DocC"]
fn native_installed_v3_resources_codecs_types_wire_and_docs() {
    let root = support::root("sdk-v3-");
    let server = WireServer::start();
    let plan = proof_plan(sdk_contract(&format!(
        "{}/specs/releases/api.json",
        server.base
    )));
    installed(
        &plan,
        &root,
        include_str!("validation_v3_native.swift"),
        &server.base,
    );
    let requests = server.finish(&root);
    for target in [
        "/specs/service/v1/tree",
        "/specs/service/v1/choice",
        "/specs/service/v1/union",
        "/specs/service/v1/record",
        "/specs/service/v1/detached",
        "/specs/service/v1/escaped",
        "/invalid/tree",
    ] {
        assert!(
            requests
                .iter()
                .any(|r| r["method"] == "POST" && r["target"] == target),
            "missing actual wire request {target}: {requests:?}"
        );
    }
    assert_eq!(
        requests.len(),
        7,
        "invalid requests must never reach the socket"
    );
    let escaped = requests
        .iter()
        .find(|r| r["target"] == "/specs/service/v1/escaped")
        .unwrap();
    assert_eq!(escaped["body"], "9007199254740993.000000000000001");
    typing(
        &root,
        "let _ = Strict(value: .object(.init()))\nlet _ = Choice(value: .object(try JsonObject([(\"selected\", JsonValue.null)])))\nlet _ = Record(amount: 9007199254740993, id: \"r\", note: .null)\nlet _ = RoundTripDetachedInput(body: DynamicSlot(value: .number(7)))\n",
        &[
            ("let _ = Strict(value: \"fallback\")", "cannot convert"),
            ("let _ = Choice(value: nil)", "not compatible"),
            (
                "let _ = RoundTripStrictTreeInput(body: JsonValue.object(.init()))",
                "cannot convert",
            ),
            ("let _ = Record(amount: 1)", "missing argument"),
            ("let _ = Record(amount: 1, id: 1)", "cannot convert"),
            ("let _ = Record(amount: 1.5, id: \"r\")", "cannot convert"),
            (
                "let _ = Record(amount: 1, id: \"r\", note: nil)",
                "not compatible",
            ),
            ("let _ = DynamicUnion.variant1(\"fallback\")", "no member"),
            (
                "let _ = RoundTripDetachedInput(body: \"fallback\")",
                "cannot convert",
            ),
        ],
    );
    support::docs(&root, "GeneratedSDK");
    println!(
        "Swift installed V3 SDK/codecs/types/wire/DocC passed at {}",
        root.display()
    );
}

#[test]
#[ignore = "focused installed physical-document server witness, exact native wire URLs, OAuth/OIDC metadata and DocC"]
fn native_installed_physical_document_servers() {
    let root = support::root("document-servers-");
    let server = WireServer::start();
    let plan = proof_plan(document_contract(&server.base));
    installed(
        &plan,
        &root,
        include_str!("protocol_resources_native.swift"),
        &server.base,
    );
    let requests = server.finish(&root);
    for target in [
        "/specs/service/v1/root",
        "/specs/service/v2/root",
        "/override/service/v1/root",
        "/custom/root",
        "/artifacts/owned/v1/mounted",
        "/artifacts/owned/v2/mounted",
        "/specs/releases/%2e%2e/Api%2Fv1/encoded",
        "/default",
        "/specs/service/v1/oauth",
        "/specs/service/v1/oidc",
        "/specs/service/v1/link",
        "/explicit/specs/local/local",
    ] {
        assert!(
            requests
                .iter()
                .any(|r| r["method"] == "GET" && r["target"] == target),
            "missing exact wire URL {target}: {requests:?}"
        );
    }
    assert_eq!(requests.len(), 12);
    for name in ["oauth", "oidc"] {
        let request = requests
            .iter()
            .find(|r| r["target"] == format!("/specs/service/v1/{name}"))
            .unwrap();
        assert_eq!(
            request["headers"]["authorization"],
            format!("Caller {name}")
        );
    }
    typing(
        &root,
        "let _: HTTPURLBase = RootServerHTTP.metadata.servers[0].urlBase\nlet _: HTTPResourceContext? = MountedServerHTTP.metadata.source.terminalResource\nlet _: SourceLocation = MountedServerHTTP.metadata.servers[0].documentBase\n",
        &[
            (
                "RootServerHTTP.metadata.servers[0].documentBase = SourceLocation(document: \"https://wrong.test\", pointer: \"\")",
                "cannot assign",
            ),
            ("let _: HTTPURLBase = .schemaResource", "no member"),
        ],
    );
    support::docs(&root, "GeneratedSDK");
    println!(
        "Swift physical-document server/native URL/DocC gate passed at {}",
        root.display()
    );
}

#[test]
fn base_and_scoped_envelopes_stay_unchanged_and_invalid_resources_stay_located() {
    for (schema, version) in [
        (json!({"type":"integer"}), OwnedProgram::V1_VERSION),
        (
            json!({"type":"object","dependentRequired":{"a":["b"]}}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let contract = support::load(
            json!({"openapi":"3.1.0","info":{"title":"frozen","version":"1"},"servers":[{"url":"https://api.test"}],"paths":{"/x":{"post":{"operationId":"plain","requestBody":{"content":{"application/json":{"schema":schema}}},"responses":{"204":{"description":"OK"}}}}}}),
            vec![],
        );
        let plan = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap();
        assert_eq!(plan.program.version, version);
        assert!(plan.program.resource_context.is_none());
        let roots = plan.protocol.codec_schema_closure();
        let expected = OwnedCompiler::new(plan.config.validation.clone())
            .compile_v2(contract, roots)
            .unwrap()
            .program();
        assert_eq!(
            serde_json::to_vec(&expected).unwrap(),
            serde_json::to_vec(&plan.program).unwrap()
        );
    }
    let config = SwiftConfig::default();
    for schema in [
        json!({"$id":"urn:bad#fragment"}),
        json!({"$dynamicRef":"urn:missing#node"}),
        json!({"$id":"urn:dup","$defs":{"A":{"$anchor":"same"},"B":{"$dynamicAnchor":"same"}}}),
    ] {
        let contract = support::load(
            json!({"openapi":"3.2.0","info":{"title":"invalid","version":"1"},"servers":[{"url":"https://api.test"}],"paths":{"/x":{"post":{"operationId":"invalid","requestBody":{"content":{"application/json":{"schema":schema}}},"responses":{"204":{"description":"OK"}}}}}}),
            vec![],
        );
        let errors = plan_sdk(contract.clone(), &selected(&contract), config.clone()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.at.end > error.at.start
                && error.source.document().as_str() == support::ENTRY),
            "{errors:?}"
        );
    }
}
