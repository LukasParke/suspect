//! Focused native physical-document-base adoption. Existing protocol/schema
//! matrices remain independent evidence and are not replayed by this target.
#![cfg(feature = "ruby-sdk")]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
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
    http_protocol as http,
    ruby_sdk::{self, PackageConfig, RubyConfig},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, Workspace, WorkspaceBuilder};
use suspect_source::Uri;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn contract(entry_origin: &str, parts_origin: &str) -> (Arc<Contract>, Arc<Workspace>, Value) {
    let entry_requested = "https://requested.example.test/old/api.json";
    let parts_requested = "https://requested.example.test/old/parts.json";
    let entry_effective = format!("{entry_origin}/entry/releases/api.json");
    let parts_effective = format!("{parts_origin}/nested/defs/parts.json");
    let entry = json!({"openapi":"3.2.0","$self":"https://logical.example.test/catalog/api.json","info":{"title":"Physical server witness","version":"1"},"paths":{
        "/inherited":{"$ref":"parts.json#/components/pathItems/Inherited"},
        "/empty":{"$ref":"parts.json#/components/pathItems/Empty"},
        "/relative":{"$ref":"parts.json#/components/pathItems/Relative"},
        "/oauth":{"$ref":"parts.json#/components/pathItems/OAuth"},
        "/oidc":{"$ref":"parts.json#/components/pathItems/Oidc"}
    }});
    let parts = json!({"openapi":"3.2.0","$self":"https://logical.example.test/catalog/parts.json","info":{"title":"Separate physical document","version":"1"},"components":{
        "pathItems":{
            "Inherited":{"get":{"operationId":"inheritedDocument","responses":{"200":{}}}},
            "Empty":{"get":{"operationId":"emptyOverride","servers":[],"responses":{"200":{}}}},
            "Relative":{"get":{"operationId":"relativeDocument","servers":[
                {"url":"../Api%2Fv1/%2e%2e/Keep","name":"relative"},
                {"url":"{endpoint}","name":"variable","variables":{"endpoint":{"default":"https://absolute.example.test/Base"}}}
            ],"responses":{"200":{}}}},
            "OAuth":{"get":{"operationId":"oauthMetadata","servers":[{"url":"./service/"}],"security":[{"flow":["read"]}],"responses":{"200":{}}}},
            "Oidc":{"get":{"operationId":"oidcMetadata","servers":[{"url":"./service/"}],"security":[{"oidc":["openid"]}],"responses":{"200":{}}}}
        },
        "securitySchemes":{
            "flow":{"type":"oauth2","oauth2MetadataUrl":"./.well-known/authorization-server","flows":{"authorizationCode":{"authorizationUrl":"../authorize","tokenUrl":"../token","scopes":{"read":"Read"}}}},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"../.well-known/openid-configuration"}
        }
    }});
    let records = [
        (entry_requested, entry_effective.as_str(), entry.clone()),
        (parts_requested, parts_effective.as_str(), parts.clone()),
    ];
    let provider = Arc::new(
        DocumentProvider::new(records.iter().map(|(requested, effective, value)| {
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
    let contract = Arc::new(
        Contract::from_workspace(&workspace, &Uri::parse(entry_requested).unwrap()).unwrap(),
    );
    (
        contract,
        workspace,
        json!({"requestedEntry":entry_requested,"entryDocument":entry_effective,"partsDocument":parts_effective,"entry":entry,"parts":parts}),
    )
}
fn selected(c: &Contract) -> Vec<SourceId> {
    c.operations().map(|o| o.source().clone()).collect()
}
fn candidate() -> RubyConfig {
    RubyConfig {
        document_relative_servers: true,
        schema_resources: false,
        dynamic_schema_references: false,
        ..Default::default()
    }
}

#[test]
fn physical_base_metadata_is_separate_and_schema_resource_fences_stay_closed() {
    let (c, witness, _) = contract(
        "http://physical-entry.example.test",
        "http://physical-parts.example.test",
    );
    let selection = selected(&c);
    let refused = ruby_sdk::plan_sdk(
        c.clone(),
        &selection,
        RubyConfig {
            document_relative_servers: false,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        refused
            .iter()
            .any(|e| e.message.contains("DocumentRelativeServers")
                || e.code == "http-capability-unsupported")
    );
    let plan = ruby_sdk::plan_sdk(c.clone(), &selection, candidate()).unwrap();
    assert!(
        plan.protocol()
            .capabilities()
            .supports(http::Capability::DocumentRelativeServers)
    );
    assert!(
        !plan
            .protocol()
            .capabilities()
            .supports(http::Capability::SchemaResources)
    );
    assert!(
        !plan
            .protocol()
            .capabilities()
            .supports(http::Capability::DynamicSchemaReferences)
    );
    assert!(
        plan.protocol().codec_roots().is_empty()
            && plan.protocol().codec_schema_closure().is_empty()
    );
    let inherited = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "inheritedDocument")
        .unwrap();
    assert_eq!(
        inherited.wire.servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "http://physical-entry.example.test/entry/releases/api.json"
    );
    let empty = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "emptyOverride")
        .unwrap();
    assert_eq!(
        empty.wire.servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "http://physical-parts.example.test/nested/defs/parts.json"
    );
    let relative = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "relativeDocument")
        .unwrap();
    let server = &relative.wire.servers().candidates()[0];
    assert_eq!(
        server.resolve_document_url(&BTreeMap::new()).unwrap(),
        "http://physical-parts.example.test/nested/Api%2Fv1/%2e%2e/Keep"
    );
    assert_eq!(
        server
            .source()
            .unwrap()
            .terminal_resource()
            .unwrap()
            .base_uri(),
        "https://logical.example.test/catalog/parts.json"
    );
    assert_eq!(server.url_base(), http::ApiUrlBase::ServerDocument);
    assert_eq!(
        Some(server.document_base().span()),
        c.source_span(server.document_base().source())
    );
    assert!(witness.failed_document_uris().is_empty());

    let schema = json!({"openapi":"3.2.0","info":{"title":"Resource refusal","version":"1"},"servers":[{"url":"https://physical.example.test"}],"paths":{"/value":{"post":{"operationId":"value","requestBody":{"content":{"application/json":{"schema":{"$id":"https://logical.example.test/schema","$dynamicAnchor":"node","type":"object","properties":{"next":{"$dynamicRef":"#node"}}}}}},"responses":{"200":{}}}}}});
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            Uri::parse("https://physical.example.test/api.json").unwrap(),
            Uri::parse("https://physical.example.test/api.json").unwrap(),
            serde_json::to_vec(&schema).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let ws = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let c = Arc::new(
        Contract::from_workspace(
            &ws,
            &Uri::parse("https://physical.example.test/api.json").unwrap(),
        )
        .unwrap(),
    );
    let plan = http::plan(
        &c,
        &selected(&c),
        http::Capabilities::for_adapter(
            "ruby-fence-witness",
            [
                http::Capability::AnonymousSecurity,
                http::Capability::UndeclaredResponseBody,
            ],
        ),
    );
    for cap in [
        http::Capability::SchemaResources,
        http::Capability::DynamicSchemaReferences,
    ] {
        assert!(
            plan.diagnostics()
                .iter()
                .any(|d| d.capability() == Some(cap) && d.resource_context().is_some())
        );
    }
    assert!(ruby_sdk::plan_sdk(c.clone(), &selected(&c), candidate()).is_err());
}

struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let values = seen.clone();
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        let mut input = BufReader::new(socket.try_clone().unwrap());
                        let mut line = String::new();
                        if input.read_line(&mut line).is_ok() {
                            values.lock().unwrap().push(line.trim_end().to_owned());
                            loop {
                                let mut h = String::new();
                                if input.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                                    break;
                                }
                            }
                            let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            origin,
            seen,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.worker.take() {
            w.join().unwrap();
        }
    }
}
fn ruby_home() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}
fn gem_defaults() -> PathBuf {
    std::fs::read_dir(ruby_home().join("lib/ruby/gems"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .unwrap()
}
fn command() -> Command {
    let mut c = Command::new(ruby_home().join("bin/ruby"));
    c.env_remove("RUBYOPT")
        .env_remove("RUBYLIB")
        .env("GEM_HOME", gem_defaults())
        .env("GEM_PATH", gem_defaults());
    c
}
fn checked(c: &mut Command, root: &Path, label: &str) {
    let output = c.output().unwrap();
    let text = format!(
        "{c:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(root.join(format!("{label}.log")), &text).unwrap();
    assert!(output.status.success(), "{}\n{text}", root.display());
}

#[test]
#[ignore = "requires installed Ruby 3.3.12+/4.0.6; builds a gem and records actual physical-origin requests"]
fn installed_gem_uses_effective_physical_document_server_bases() {
    let entry = Server::new();
    let parts = Server::new();
    let explicit = Server::new();
    let (c, workspace, source) = contract(&entry.origin, &parts.origin);
    let plan = ruby_sdk::plan_sdk(c.clone(), &selected(&c), candidate()).unwrap();
    let base = root().join("target/sdk-ruby-document-servers-native");
    std::fs::create_dir_all(&base).unwrap();
    let out = tempfile::Builder::new()
        .prefix("witness-")
        .tempdir_in(base)
        .unwrap()
        .keep();
    let pkg = PackageConfig {
        name: "ruby-document-server-gate".into(),
        version: "0.1.0".into(),
        require_name: "ruby_document_server".into(),
        namespace: "RubyDocumentServer".into(),
    };
    for file in ruby_sdk::emit_sdk(&plan, &pkg).unwrap() {
        let p = out.join(file.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, file.content).unwrap();
    }
    std::fs::write(out.join("source.json"), source.to_string()).unwrap();
    std::fs::write(
        out.join("origins.json"),
        json!({"entry":entry.origin,"parts":parts.origin,"explicit":explicit.origin}).to_string(),
    )
    .unwrap();
    checked(command().arg("-v"), &out, "ruby-version");
    checked(
        command()
            .arg(ruby_home().join("bin/gem"))
            .args(["build", "ruby-document-server-gate.gemspec"])
            .current_dir(out.join("ruby")),
        &out,
        "gem-build",
    );
    checked(
        command()
            .arg(ruby_home().join("bin/gem"))
            .args(["install", "--local", "--no-document"])
            .arg(out.join("ruby/ruby-document-server-gate-0.1.0.gem"))
            .env("GEM_HOME", out.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    out.join("installed").display(),
                    gem_defaults().display()
                ),
            ),
        &out,
        "gem-install",
    );
    let consumer = include_str!("../src/ruby_sdk/tests/document_servers.rb");
    std::fs::write(out.join("consumer.rb"), consumer).unwrap();
    checked(
        command()
            .arg("consumer.rb")
            .current_dir(&out)
            .env("GEM_HOME", out.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    out.join("installed").display(),
                    gem_defaults().display()
                ),
            ),
        &out,
        "native",
    );
    assert_eq!(*entry.seen.lock().unwrap(), vec!["GET /inherited HTTP/1.1"]);
    assert_eq!(
        *parts.seen.lock().unwrap(),
        vec![
            "GET /empty HTTP/1.1",
            "GET /nested/Api%2Fv1/%2e%2e/Keep/relative HTTP/1.1",
            "GET /nested/Dir//%2E%2E/%2f/KeEp/relative HTTP/1.1",
            "GET /nested/defs/service/oauth HTTP/1.1",
            "GET /nested/defs/service/oidc HTTP/1.1"
        ]
    );
    assert_eq!(
        *explicit.seen.lock().unwrap(),
        vec!["GET /other/Api%2Fv1/%2e%2e/Keep/relative HTTP/1.1"]
    );
    assert!(workspace.failed_document_uris().is_empty());
    std::fs::write(out.join("requests.json"),json!({"entry":*entry.seen.lock().unwrap(),"parts":*parts.seen.lock().unwrap(),"explicit":*explicit.seen.lock().unwrap()}).to_string()).unwrap();
    for file in ruby_sdk::emit_sdk(&plan, &pkg).unwrap() {
        let name = file.path.strip_prefix("ruby/").unwrap();
        if !name.ends_with(".gemspec") {
            assert_eq!(
                std::fs::read(
                    out.join("installed/gems/ruby-document-server-gate-0.1.0")
                        .join(name)
                )
                .unwrap(),
                file.content.as_bytes()
            );
        }
    }
}
