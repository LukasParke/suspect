use super::{DartConfig, Plan, codegen, support};
use serde_json::Value;
use std::{
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::OwnedProgram;
use suspect_source::Uri;

fn fixture(root: &Path) -> Plan {
    let input = include_str!("../../tests/fixtures/dart-resources-v3.json");
    std::fs::write(root.join("source-documents.json"), input).unwrap();
    let v: Value = serde_json::from_str(input).unwrap();
    let provider = Arc::new(
        DocumentProvider::new(v["documents"].as_array().unwrap().iter().map(|d| {
            ProvidedDocument::new(
                Uri::parse(d["requested"].as_str().unwrap()).unwrap(),
                Uri::parse(d["effective"].as_str().unwrap()).unwrap(),
                serde_json::to_vec(&d["value"]).unwrap(),
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let w = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let c = Arc::new(
        Contract::from_workspace(&w, &Uri::parse(v["entry"].as_str().unwrap()).unwrap()).unwrap(),
    );
    let selected = c
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(c, &selected, DartConfig::default()).unwrap()
}
#[test]
fn v3_dynamic_models_keep_contextual_values_and_physical_sources() {
    let root = support::root("v3-models-");
    let p = fixture(&root);
    assert_eq!(p.program().version, OwnedProgram::V3_VERSION);
    let payload = p
        .models()
        .symbols()
        .iter()
        .find(|m| m.name == "TemplatePayload")
        .unwrap();
    assert!(matches!(payload.shape, super::DartShape::Contextual));
    assert_eq!(p.models().ty(payload.index), "JsonValue");
    let context = p.program().resource_context.as_ref().unwrap();
    let external = context
        .resources
        .iter()
        .find(|r| r.canonical_uri == "urn:counter")
        .unwrap();
    assert_eq!(
        external.source.document,
        "https://cdn.dart.test/artifacts/counter.json"
    );
    assert!(
        external
            .aliases
            .contains(&"https://download.dart.test/counter.json".into())
    );
    assert!(
        !p.examples()
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-schema-unsupported"),
        "{:?}",
        p.examples().diagnostics()
    );
    assert!(
        p.examples()
            .operations()
            .iter()
            .flat_map(|o| &o.entries)
            .count()
            >= 6
    );
    let target = codegen::backend::TargetConfig {
        backend: codegen::backend::Backend::DartHttp,
        package_name: "generated_sdk".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    };
    let snapshot = codegen::compatibility::snapshot(p.contract().clone(), &[], &[target]).unwrap();
    assert_eq!(
        snapshot.native[0].status,
        codegen::compatibility::PlanStatus::Planned
    );
    let desc = snapshot.native[0]
        .models
        .iter()
        .find(|m| m.name == "TemplatePayload" && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(desc["representation"], "context-bound-exact-json");
    assert_eq!(desc["signatureType"], "JsonValue");
    let desc = snapshot.native[0]
        .models
        .iter()
        .find(|m| m.name == "templatePayloadCodec" && m.role == "codec")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(desc["validation"]["profile"], OwnedProgram::V3_PROFILE);
    if let Some(paths) = std::env::var_os("SUSPECT_DART_V3_WITNESS_ROOTS") {
        for root in std::env::split_paths(&paths) {
            for file in p.render() {
                assert_eq!(
                    file.content,
                    std::fs::read_to_string(root.join(&file.path)).unwrap(),
                    "public v3 admission changed {}",
                    file.path
                );
            }
            println!("DART_V3_PUBLIC_BYTES_IDENTICAL={}", root.display());
        }
    }
}

#[test]
fn v3_dispatch_preserves_ordinary_v1_v2_programs_and_native_files() {
    for (n, schema, version) in [
        (
            1,
            serde_json::json!({"type":"integer","minimum":0}),
            OwnedProgram::V1_VERSION,
        ),
        (
            2,
            serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"unevaluatedProperties":false}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let root = support::root(&format!("v3-preserve-v{n}-"));
        let path = root.join("api.json");
        std::fs::write(&path,serde_json::json!({"openapi":"3.2.0","info":{"title":"Preserved profile","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/value":{"get":{"operationId":"value","responses":{"200":{"content":{"application/json":{"schema":schema}}}}}}}}).to_string()).unwrap();
        let c = support::load(&path);
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let mut p = super::plan_sdk(c.clone(), &selected, Default::default()).unwrap();
        assert_eq!(p.program().version, version);
        assert!(p.program().resource_context.is_none());
        let emitted = p.render();
        p.compiled = suspect_schema::OwnedCompiler::new(p.config.schema.clone())
            .compile_v2(c.clone(), p.protocol().codec_schema_closure())
            .unwrap();
        p.program = p.compiled.program();
        p.models = super::models::plan(&c, &p.compiled, &p.program).unwrap();
        assert_eq!(emitted, p.render());
    }
}
#[test]
#[ignore = "installed native resource/dynamic SDK operations, models/types, examples/docs and independent sockets"]
fn native_v3_sdk_operations() {
    let root = support::root("v3-sdk-");
    let p = fixture(&root);
    support::install(&root, &p.render());
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("native_v3.dart"),
    )
    .unwrap();
    std::fs::write(consumer.join("bin/main.dart"),"import 'dart:io' show Platform;\nimport 'package:generated_sdk/generated_sdk_io.dart';\nimport 'portable.dart' as witness;\nFuture<void> main()=>witness.exercise(IoTransport(),server:Uri.parse(Platform.environment['DART_V3_BASE']!));\n").unwrap();
    std::fs::copy(
        root.join("dart/example/source_examples.dart"),
        consumer.join("bin/examples.dart"),
    )
    .unwrap();
    let readme = std::fs::read_to_string(root.join("dart/README.md")).unwrap();
    let snippet = readme
        .split_once("```dart\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    assert!(snippet.contains("client."));
    std::fs::write(consumer.join("bin/readme.dart"), snippet).unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    for input in ["main", "examples", "readme"] {
        support::check(
            support::dart(&root)
                .args(["compile", "exe"])
                .arg(format!("bin/{input}.dart"))
                .arg("-o")
                .arg(root.join(format!("{input}-vm")))
                .current_dir(&consumer),
            &root,
            &format!("compile-{input}-vm"),
        );
    }
    support::check(
        &mut Command::new(root.join("examples-vm")),
        &root,
        "examples-run-vm",
    );
    let strict = AtomicUsize::new(0);
    let server = support::Server::start(move |stream, request, _| {
        let invalid = br#"{"children":[{"data":"child","extra":null}],"data":"root"}"#;
        if request.path == "/v3/rows" {
            support::reply(stream,200,"application/x-ndjson",b"{\"children\":[{\"data\":\"child\"}],\"data\":\"root\"}\n{\"children\":[{\"data\":\"child\",\"extra\":null}],\"data\":\"root\"}\n");
        } else if request.path == "/v3/strict" && strict.fetch_add(1, Ordering::SeqCst) == 1 {
            support::reply(stream, 200, "application/json", invalid);
        } else {
            support::reply(stream, 200, "application/json", &request.body);
        }
    });
    support::check(
        Command::new(root.join("main-vm")).env("DART_V3_BASE", format!("{}/v3", server.url)),
        &root,
        "run-vm",
    );
    support::check(
        Command::new(root.join("readme-vm")).env("API_SERVER", format!("{}/v3", server.url)),
        &root,
        "readme-run-vm",
    );
    {
        let records = server.records.lock().unwrap();
        assert_eq!(records.len(), 7);
        assert_eq!(
            records[0].body,
            br#"{"children":[{"data":"child"}],"data":"root"}"#
        );
        assert_eq!(
            records[1].body,
            br#"{"children":[{"data":"child","extra":null}],"data":"root"}"#
        );
        assert_eq!(
            records[2].body,
            br#"{"note":null,"payload":9007199254740993}"#
        );
        assert_eq!(records[3].body, b"9007199254740993");
        assert_eq!(records[5].method, "GET");
        assert!(records.iter().all(|r| r.path.starts_with("/v3/")));
        std::fs::write(
            root.join("wire-records.json"),
            serde_json::to_vec_pretty(&*records).unwrap(),
        )
        .unwrap();
    }
    drop(server);
    for input in ["portable", "examples"] {
        support::check(
            support::dart(&root)
                .args(["compile", "js"])
                .arg(format!("bin/{input}.dart"))
                .arg("-o")
                .arg(root.join(format!("{input}.js")))
                .current_dir(&consumer),
            &root,
            &format!("compile-{input}-js"),
        );
        support::node(&root, &format!("{input}.js"), &format!("run-{input}-js"));
    }
    support::check(
        support::dart(&root)
            .args(["doc", "--validate-links", "--output"])
            .arg(root.join("dartdoc"))
            .current_dir(root.join("dart")),
        &root,
        "dartdoc",
    );
    let log = std::fs::read_to_string(root.join("logs/dartdoc.log")).unwrap();
    assert!(log.contains("Found 0 warnings and 0 errors."), "{log}");
    assert!(
        std::fs::read_to_string(root.join("dartdoc/index.html"))
            .unwrap()
            .contains("Resource and dynamic validation")
    );
    for path in ["Template/payload.html", "Tree/children.html"] {
        assert!(
            std::fs::read_to_string(root.join("dartdoc/generated_sdk").join(path))
                .unwrap()
                .contains("JsonValue")
        );
    }
    for (i, (source, code)) in [
        (
            "void main(){ Template(payload: 1.5); }",
            "argument_type_not_assignable",
        ),
        (
            "void main(){ Tree(children: const Absent()); }",
            "missing_required_argument",
        ),
        (
            "void f(Template value){ value.payload=null; }",
            "invalid_assignment",
        ),
        (
            "void f(Template value){ value.note=null; }",
            "invalid_assignment",
        ),
        (
            "void f(Client client){client.counter(body: 9007199254740993);}",
            "argument_type_not_assignable",
        ),
        (
            "void f(Client client){client.resourceRows().then((v){});}",
            "undefined_method",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = consumer.join(format!("negative-{i}.dart"));
        std::fs::write(
            &path,
            format!("import 'package:generated_sdk/generated_sdk.dart';\n{source}\n"),
        )
        .unwrap();
        let result = support::output(
            support::dart(&root)
                .args(["analyze", "--fatal-infos"])
                .arg(&path)
                .current_dir(&consumer),
            &root,
            &format!("negative-{i}"),
        );
        let text = String::from_utf8_lossy(&result.stdout);
        assert!(
            !result.status.success() && text.contains(code) && !text.contains("uri_does_not_exist"),
            "{text}"
        );
    }
}
