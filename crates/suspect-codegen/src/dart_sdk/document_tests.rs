//! Focused physical retrieval-base and encoded URI native acceptance.
use super::{DartConfig, Plan, codegen, support};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Command, sync::Arc};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn witness(entry: &str, parts: &str, local: &str) -> Value {
    let response = json!({"204":{"description":"No content"}});
    let operation = |id: &str, servers: Option<Value>| {
        let mut op = json!({"operationId":id,"responses":response});
        if let Some(v) = servers {
            op["servers"] = v;
        }
        json!({"get":op})
    };
    let mut path_items = json!({
        "Inherited":operation("inherited",None),"Empty":operation("empty",Some(json!([]))),
        "Relative":operation("relative",Some(json!([{"url":"../service/%2e%2e/Api%2Fv1"}]))),
        "Dots":operation("dots",Some(json!([{"url":"./a/../service"}]))),
        "Variable":operation("variable",Some(json!([{"url":"{base}","variables":{"base":{"default":format!("{entry}/absolute%2fBase")}}}]))),
        "Auth":operation("auth",Some(json!([{"url":"./authbase/"}])))
    });
    path_items["Relative"]["get"]["responses"]["204"]["links"] = json!({"entry":{"operationRef":"entry.json#/paths/~1entry/get","server":{"url":"../link/%2E/Keep%2fCase"}}});
    path_items["Auth"]["get"]["security"] = json!([{"oauth":["read"]},{"oidc":[]}]);
    let mut paths = json!({"/entry":operation("entry",None),"/local":{"$ref":"local.json#/components/pathItems/Local"}});
    for (name, mount) in [
        ("Inherited", "inherited"),
        ("Empty", "empty"),
        ("Relative", "relative"),
        ("Dots", "dots"),
        ("Variable", "variable"),
        ("Auth", "auth"),
    ] {
        paths[format!("/{mount}")] =
            json!({"$ref":format!("parts.json#/components/pathItems/{name}")});
    }
    json!({"entry":format!("{entry}/redirect/latest.json"),"documents":[
        {"requested":format!("{entry}/redirect/latest.json"),"effective":format!("{entry}/retrieved/entry/api.json"),"value":{"openapi":"3.2.0","$self":"https://logical.example/catalog/entry.json#definition","info":{"title":"Physical document servers","version":"1"},"security":[],"paths":paths}},
        {"requested":format!("{entry}/redirect/parts.json"),"effective":format!("{parts}/retrieved/fragments/parts.json"),"value":{"openapi":"3.2.0","$self":"https://logical.example/catalog/parts.json","info":{"title":"Physical fragments","version":"1"},"components":{"pathItems":path_items,"securitySchemes":{"oauth":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"./token","scopes":{"read":"read"}}}},"oidc":{"type":"openIdConnect","openIdConnectUrl":"./discovery"}}}}},
        {"requested":local,"effective":local,"value":{"openapi":"3.2.0","$self":"https://logical.example/catalog/local.json","info":{"title":"Local file fragment","version":"1"},"components":{"pathItems":{"Local":operation("local",Some(json!([{"url":"./api"}])))}}}}
    ]})
}
fn contract(witness: &Value) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(witness["documents"].as_array().unwrap().iter().map(|d| {
            ProvidedDocument::new(
                Uri::parse(d["requested"].as_str().unwrap()).unwrap(),
                Uri::parse(d["effective"].as_str().unwrap()).unwrap(),
                serde_json::to_vec(&d["value"]).unwrap(),
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
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse(witness["entry"].as_str().unwrap()).unwrap(),
        )
        .unwrap(),
    )
}
fn plan(w: &Value) -> Plan {
    let c = contract(w);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(c, &selected, DartConfig::default()).unwrap()
}
fn persist(root: &Path, w: &Value) {
    std::fs::write(
        root.join("source-documents.json"),
        serde_json::to_vec_pretty(w).unwrap(),
    )
    .unwrap();
}

#[test]
fn document_server_sources_are_physical_and_schema_resources_stay_fenced() {
    let root = support::root("document-plan-");
    let w = witness(
        "https://entry.example",
        "https://parts.example",
        "file:///local/fragment.json",
    );
    persist(&root, &w);
    let p = plan(&w);
    assert!(p.protocol().codec_roots().is_empty());
    assert!(p.protocol().codec_schema_closure().is_empty());
    let target = codegen::backend::TargetConfig {
        backend: codegen::backend::Backend::DartHttp,
        package_name: "generated_sdk".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    };
    let captured = codegen::compatibility::snapshot(p.contract().clone(), &[], &[target]).unwrap();
    assert_eq!(
        captured.native[0].status,
        codegen::compatibility::PlanStatus::Planned
    );
    let relative = captured.native[0]
        .operations
        .iter()
        .find(|o| o.operation_id == "relative")
        .unwrap();
    assert_eq!(
        relative.descriptor["servers"][0]["documentBase"]["source"]["document"],
        "https://parts.example/retrieved/fragments/parts.json"
    );
    assert_eq!(
        relative.descriptor["servers"][0]["resource"]["canonical_uri"],
        "https://logical.example/catalog/parts.json"
    );
    for (name, url) in [
        ("inherited", "https://entry.example/"),
        ("empty", "https://parts.example/"),
        (
            "relative",
            "https://parts.example/retrieved/service/%2e%2e/Api%2Fv1",
        ),
    ] {
        let op = p
            .operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap();
        let server = &op.wire.servers().candidates()[0];
        assert_eq!(server.resolve_document_url(&BTreeMap::new()).unwrap(), url);
        assert_eq!(server.document_base().source().pointer(), "");
        assert_eq!(
            op.wire.source().references().len(),
            op.wire.source().reference_resources().len()
        );
        assert!(
            op.wire
                .source()
                .terminal_resource()
                .unwrap()
                .canonical_uri()
                .starts_with("https://logical.example/")
        );
    }
    for schema in [
        json!({"$id":"https://schema.example/value","type":"string"}),
        json!({"$dynamicAnchor":"node","$dynamicRef":"#node"}),
    ] {
        let mut bad = w.clone();
        bad["documents"][0]["value"]["paths"]["/entry"]["get"]["servers"] =
            json!([{"url":"https://server.example"}]);
        bad["documents"][0]["value"]["paths"]["/entry"]["get"]["responses"] =
            json!({"200":{"content":{"application/json":{"schema":schema}}}});
        let c = contract(&bad);
        let selected = c
            .operations()
            .filter(|o| o.operation_id() == Some("entry"))
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        // A deliberately static adapter still refuses these executable profiles;
        // Dart's newly witnessed v3 dispatcher now enables them separately.
        let caps = codegen::http_protocol::Capabilities::for_adapter(
            "static-control",
            codegen::http_protocol::Capability::ALL
                .iter()
                .copied()
                .filter(|c| {
                    !matches!(
                        c,
                        codegen::http_protocol::Capability::SchemaResources
                            | codegen::http_protocol::Capability::DynamicSchemaReferences
                    )
                }),
        );
        let refused = codegen::http_protocol::plan(&c, &selected, caps);
        assert!(
            refused
                .diagnostics()
                .iter()
                .any(|e| e.source().source().document() == c.entry()
                    && !e.source().span().is_empty()
                    && e.code() == "http-capability-required"),
            "{:?}",
            refused.diagnostics()
        );
    }
    if let Some(paths) = std::env::var_os("SUSPECT_DART_DOCUMENT_WITNESS_ROOTS") {
        for root in std::env::split_paths(&paths) {
            let witness: Value =
                serde_json::from_slice(&std::fs::read(root.join("source-documents.json")).unwrap())
                    .unwrap();
            for file in plan(&witness).render() {
                assert_eq!(
                    file.content,
                    std::fs::read_to_string(root.join(&file.path)).unwrap(),
                    "public document-base admission changed {}",
                    file.path
                );
            }
            println!("DART_DOCUMENT_PUBLIC_BYTES_IDENTICAL={}", root.display());
        }
    }
}

#[test]
#[ignore = "installed VM/JS document-relative server and physical/encoded URI witness"]
fn native_document_relative_servers() {
    let root = support::root("document-native-");
    let entry =
        support::Server::start(|stream, _, _| support::reply(stream, 204, "application/json", b""));
    let parts =
        support::Server::start(|stream, _, _| support::reply(stream, 204, "application/json", b""));
    let local = Uri::from_path(&root.join("local-fragment.json")).unwrap();
    let w = witness(&entry.url, &parts.url, local.as_str());
    persist(&root, &w);
    let p = plan(&w);
    support::install(&root, &p.render());
    let consumer = root.join("consumer");
    let source = include_str!("native_document.dart")
        .replace("__ENTRY_ORIGIN__", &super::emit::quote(&entry.url))
        .replace("__PARTS_ORIGIN__", &super::emit::quote(&parts.url));
    std::fs::write(consumer.join("bin/portable.dart"), source).unwrap();
    std::fs::write(consumer.join("bin/main.dart"),"import 'package:generated_sdk/generated_sdk_io.dart';\nimport 'portable.dart' as witness;\nFuture<void> main()=>witness.exercise(IoTransport());\n").unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root.join("consumer-vm"))
            .current_dir(&consumer),
        &root,
        "compile-vm",
    );
    support::check(&mut Command::new(root.join("consumer-vm")), &root, "run-vm");
    let expected_entry = [
        "/entry",
        "/inherited",
        "/absolute%2fBase/variable",
        "/service/%2e%2e/Api%2Fv1/relative",
        "/runtime/api/local",
        "/chosen/relative",
    ];
    let expected_parts = [
        "/empty",
        "/retrieved/service/%2e%2e/Api%2Fv1/relative",
        "/retrieved/fragments/service/dots",
        "/retrieved/runtime/%2E/Keep%2fCase/variable",
        "/retrieved/fragments/authbase/auth",
        "/retrieved/fragments/authbase/auth",
    ];
    for (name, server, expected) in [
        ("entry", &entry, expected_entry.as_slice()),
        ("parts", &parts, expected_parts.as_slice()),
    ] {
        let records = server.records.lock().unwrap();
        assert_eq!(
            records.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            expected
        );
        assert!(records.iter().all(|r| r.method == "GET"));
        std::fs::write(
            root.join(format!("wire-{name}.json")),
            serde_json::to_vec_pretty(&*records).unwrap(),
        )
        .unwrap();
    }
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/portable.dart", "-o"])
            .arg(root.join("portable.js"))
            .current_dir(&consumer),
        &root,
        "compile-js",
    );
    support::node(&root, "portable.js", "run-js");
    println!("DART_DOCUMENT_SERVER_GATE_ROOT={}", root.display());
}
