#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use suspect_codegen::kotlin_sdk::{self, SdkConfig};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;
#[path = "kotlin_support/mod.rs"]
#[allow(dead_code)]
mod support;

struct Server {
    base: String,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let captured = seen.clone();
        let flag = stop.clone();
        let join = thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                if flag.load(Ordering::Acquire) {
                    break;
                }
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0u8; 1];
                while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 32768 {
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    bytes.push(byte[0]);
                }
                let text = String::from_utf8(bytes).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(text.lines().next().unwrap_or("").into());
                let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\nConnection: close\r\n\r\nok\0");
            }
        });
        Self {
            base,
            seen,
            stop,
            join: Some(join),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
    }
}
fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-documents");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gate-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn operation(name: &str, servers: Option<Value>) -> Value {
    let mut op = json!({"operationId":name,"responses":{"200":{"description":"bytes","content":{"application/octet-stream":{}}}}});
    if let Some(servers) = servers {
        op["servers"] = servers;
    }
    json!({"get":op})
}
fn plan(root: &Path, a: &str, b: &str) -> kotlin_sdk::Plan {
    let mut items = serde_json::Map::new();
    items.insert("Inherited".into(), operation("inheritedDoc", None));
    items.insert("Empty".into(), operation("emptyDoc", Some(json!([]))));
    items.insert(
        "Relative".into(),
        operation("relativeDoc", Some(json!([{"url":"%2e%2e/Api%2Fv1"}]))),
    );
    items.insert(
        "Root".into(),
        operation("rootDoc", Some(json!([{"url":"/root"}]))),
    );
    items.insert(
        "Absolute".into(),
        operation(
            "absoluteDoc",
            Some(json!([{"url":format!("{a}/absolute")}])),
        ),
    );
    items.insert("Variable".into(),operation("variableDoc",Some(json!([{"url":"{server}","variables":{"server":{"default":format!("{a}/from-default")}}}]))));
    let mut oauth = operation("oauthDoc", Some(json!([{"url":"../api/"}])));
    oauth["get"]["security"] = json!([{"oauth":["read"]}]);
    items.insert("OAuth".into(), oauth);
    let entry_uri = format!("{a}/cdn/entry/openapi.json");
    let defs_uri = format!("{b}/storage/fragments/paths.json");
    let requested = format!("{a}/requested/entry.json");
    let local_path = root.join("local.json");
    let local_uri = Uri::from_path(&local_path).unwrap().to_string();
    let local = json!({"openapi":"3.2.0","info":{"title":"local","version":"1"},"components":{"pathItems":{"Local":operation("localDoc",Some(json!([{"url":"../v1"}])))}},"paths":{}});
    std::fs::write(&local_path, local.to_string()).unwrap();
    let entry = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/entry.json#definition","info":{"title":"entry","version":"1"},"security":[],"paths":{
        "/inherited":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Inherited"},
        "/empty":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Empty"},
        "/relative":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Relative"},
        "/root":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Root"},
        "/absolute":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Absolute"},
        "/variable":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/Variable"},
        "/oauth":{"$ref":"https://logical.example/catalog/paths.json#/components/pathItems/OAuth"},
        "/local":{"$ref":format!("{local_uri}#/components/pathItems/Local")}
    }});
    let defs = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/paths.json","info":{"title":"definitions","version":"1"},"security":[],"components":{"pathItems":items,"securitySchemes":{"oauth":{"type":"oauth2","flows":{"authorizationCode":{"authorizationUrl":"../authorize","tokenUrl":"token","scopes":{"read":"read"}}}}}},"paths":{}});
    let data = [
        (requested.clone(), entry_uri.clone(), entry),
        (
            format!("{b}/requested/definitions.json"),
            defs_uri.clone(),
            defs,
        ),
        (local_uri.clone(), local_uri, local),
    ];
    std::fs::write(root.join("documents.json"),serde_json::to_string_pretty(&json!({"entry":requested,"documents":data.iter().map(|(requested,effective,value)|json!({"requested":requested,"effective":effective,"value":value})).collect::<Vec<_>>()})).unwrap()).unwrap();
    let provider = Arc::new(
        DocumentProvider::new(data.iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec(value).unwrap(),
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
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(&requested).unwrap()).unwrap());
    assert_eq!(contract.entry().as_str(), entry_uri);
    assert!(workspace.failed_document_uris().is_empty());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 8);
    let plan = kotlin_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            group_id: "test.suspect.kotlin".into(),
            artifact_id: "document-server-sdk".into(),
            version: "0.4.0".into(),
            package_name: "example.documents".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        },
    )
    .unwrap();
    for (name, document) in [
        ("inheritedDoc", &entry_uri),
        ("emptyDoc", &defs_uri),
        ("relativeDoc", &defs_uri),
    ] {
        let op = plan
            .operations()
            .iter()
            .find(|op| op.operation_id == name)
            .unwrap();
        assert_eq!(
            op.wire.servers().candidates()[0]
                .document_base()
                .source()
                .document()
                .as_str(),
            document
        );
        assert_ne!(
            op.wire.source().terminal_resource().unwrap().base_uri(),
            document
        );
    }
    assert!(plan.protocol().codec_roots().is_empty());
    assert_eq!(
        plan.program().version,
        suspect_schema::OwnedProgram::V1_VERSION
    );
    assert!(plan.program().resource_context.is_none());
    plan
}

#[test]
fn physical_document_bases_and_logical_contexts_are_distinct() {
    let root = root();
    let plan = plan(&root, "http://127.0.0.1:10001", "http://127.0.0.1:10002");
    assert!(plan.protocol().codec_roots().is_empty());
    assert_eq!(plan.operations().len(), 8);
}

#[test]
#[ignore = "installed JDK21/25 focused physical document, redirect, override and encoded-path native witness"]
fn native_physical_document_servers() {
    let root = root();
    let a = Server::new();
    let b = Server::new();
    let plan = plan(&root, &a.base, &b.base);
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    let source = include_str!("../src/kotlin_sdk/native_document_servers.kt")
        .replace("__A__", &a.base)
        .replace("__B__", &b.base);
    std::fs::write(consumer.join("src/main/kotlin/NativeDocuments.kt"), source).unwrap();
    std::fs::write(consumer.join("pom.xml"),format!(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>document-consumer</artifactId><version>0.4.0</version><properties><kotlin.compiler.daemon>false</kotlin.compiler.daemon><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties><dependencies><dependency><groupId>test.suspect.kotlin</groupId><artifactId>document-server-sdk</artifactId><version>0.4.0</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin><plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>consumer.NativeDocumentsKt</argument></arguments></configuration></plugin></plugins></build></project>"#,kotlin_sdk::KOTLIN_VERSION)).unwrap();
    for (version, home) in support::java_homes() {
        for (label, dir, args) in [
            ("sdk", root.join("kotlin"), vec!["install"]),
            ("consumer", consumer.clone(), vec!["compile", "exec:exec"]),
        ] {
            let output = support::maven(&home)
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::write(root.join(format!("{label}-{version}.log")), &log).unwrap();
            assert!(output.status.success(), "{}\n{log}", root.display());
        }
        support::native_docs(&plan, &root, &version);
    }
    assert_eq!(a.seen.lock().unwrap().len(), 12);
    assert_eq!(b.seen.lock().unwrap().len(), 10);
    std::fs::write(
        root.join("wire.json"),
        serde_json::to_string_pretty(
            &json!({"a":a.seen.lock().unwrap().clone(),"b":b.seen.lock().unwrap().clone()}),
        )
        .unwrap(),
    )
    .unwrap();
    println!("Kotlin document server witness: {}", root.display());
}
