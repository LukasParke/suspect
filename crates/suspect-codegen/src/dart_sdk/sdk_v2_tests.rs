//! Installed real-operation witnesses for scoped native models, docs and codecs.
use super::{DartConfig, DartExtras, DartShape, Plan, support};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use suspect_schema::OwnedProgram;

const DOCUMENT: &str = include_str!("../../tests/fixtures/dart-applicators-v2.json");
const WIRE: &[u8] = br#"{"flexible":null,"patch":{"billing":null,"card":null,"kind":"card","x-n":9007199254740993,"x-null":null},"sequence":["s",9007199254740993,1e+000008],"stamp":null}"#;
const MIXED: &[u8] = br#"{"s-required":"r","s-title":"t","s-null":null,"count":9007199254740993}"#;
fn fixture(root: &Path) -> Plan {
    let path = root.join("api.json");
    std::fs::write(&path, DOCUMENT).unwrap();
    let contract = support::load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(contract, &selected, DartConfig::default()).unwrap()
}

#[test]
fn v2_model_admission_retains_patterned_values_and_nullable_carriers() {
    let root = support::root("models-");
    let plan = fixture(&root);
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    for name in ["Patch", "MixedExtras"] {
        let model = plan
            .models()
            .symbols()
            .iter()
            .find(|m| m.name == name)
            .unwrap();
        let DartShape::Object { fields, extras } = &model.shape else {
            panic!("{name}: expected native object")
        };
        assert!(matches!(extras, DartExtras::Checked));
        let required = fields
            .iter()
            .find(|f| f.wire_name == if name == "Patch" { "x-n" } else { "s-required" })
            .unwrap();
        assert!(
            required.required && required.target.is_none(),
            "pattern-only required field must not use additionalProperties as its type"
        );
    }
    let flexible = plan
        .models()
        .symbols()
        .iter()
        .find(|m| m.name == "EnvelopeFlexible")
        .unwrap();
    assert!(flexible.nullable);
    assert_eq!(plan.models().ty(flexible.index), "Flexible");
    assert!(!plan.models().uses_native_null(flexible.index));
    let patch = plan
        .models()
        .symbols()
        .iter()
        .find(|m| m.name == "EnvelopePatch")
        .unwrap();
    assert_eq!(plan.models().ty(patch.index), "Patch?");
    let scoped = plan.examples();
    assert_eq!(scoped.diagnostics().len(), 1, "{:#?}", scoped.diagnostics());
    assert_eq!(scoped.diagnostics()[0].code, "examples-declared-invalid");
    assert!(
        scoped.diagnostics()[0]
            .source
            .pointer()
            .ends_with("/examples/bad/dataValue")
    );
    assert!(
        scoped
            .operations()
            .iter()
            .flat_map(|o| &o.entries)
            .any(|e| e
                .declared_source
                .as_ref()
                .is_some_and(|s| s.pointer() == "/components/examples/EnvelopeExample/dataValue"))
    );
    assert!(
        scoped
            .operations()
            .iter()
            .flat_map(|o| &o.entries)
            .any(|e| matches!(e.role, crate::examples::ExampleRole::ResponseItem { .. }))
    );
    for e in plan.examples().operations().iter().flat_map(|o| &o.entries) {
        assert!(matches!(
            plan.compiled.validate(&e.schema, &e.value),
            suspect_schema::OwnedOutcome::Valid
        ));
        assert!(
            plan.contract()
                .source(e.declared_source.as_ref().unwrap())
                .is_some()
        );
    }
    assert!(
        plan.render()
            .iter()
            .find(|f| f.path == "dart/example/quickstart.dart")
            .unwrap()
            .content
            .contains(".scopedEcho(")
    );
    // Optional release-evidence comparison. The maintained fixture above is
    // source-driven and works without any prior target/ output.
    if let Some(paths) = std::env::var_os("SUSPECT_DART_V2_WITNESS_ROOTS") {
        for witness in std::env::split_paths(&paths) {
            let contract = support::load(&witness.join("api.json"));
            let selected = contract
                .operations()
                .map(|o| o.source().clone())
                .collect::<Vec<_>>();
            let public = super::plan_sdk(contract, &selected, DartConfig::default()).unwrap();
            for file in public.render() {
                assert_eq!(
                    file.content,
                    std::fs::read_to_string(witness.join(&file.path)).unwrap(),
                    "public admission changed witnessed {}",
                    file.path
                );
            }
            println!(
                "DART_V2_PUBLIC_WITNESS_BYTES_IDENTICAL={}",
                witness.display()
            );
        }
    }
}

#[test]
fn unused_v2_schemas_do_not_change_a_v1_package() {
    let root = support::root("package-v1-identity-");
    let path = root.join("api.json");
    let mut document: Value = serde_json::from_str(DOCUMENT).unwrap();
    document["paths"]["/v1"] = json!({"post":{"operationId":"baseOnly","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"optional":{"type":["string","null"]},"amount":{"type":"number","minimum":0}},"additionalProperties":true}}}},"responses":{"204":{"description":"empty"}}}});
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = support::load(&path);
    let selected = contract
        .operations()
        .filter(|o| o.operation_id() == Some("baseOnly"))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let mut old = super::plan_sdk(contract.clone(), &selected, DartConfig::default()).unwrap();
    let roots = crate::schema_view::closure(&contract, old.protocol().codec_roots());
    old.compiled = suspect_schema::OwnedCompiler::new(old.config.schema.clone())
        .compile(contract.clone(), &roots)
        .unwrap();
    old.program = old.compiled.program();
    old.models = super::models::plan(&contract, &old.compiled, &old.program).unwrap();
    let new = super::plan_sdk(contract, &selected, DartConfig::default()).unwrap();
    assert_eq!(new.program().version, OwnedProgram::V1_VERSION);
    assert_eq!(old.program(), new.program());
    assert_eq!(old.render(), new.render());
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"files_identical":old.render().len(),"program_version":new.program().version}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "installed Dart v2 operations, independent sockets, strict native types and executable rendered guides"]
fn native_v2_sdk_operations() {
    let root = support::root("sdk-");
    let plan = fixture(&root);
    let files = plan.render();
    support::install(&root, &files);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("native_v2.dart"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("bin/main.dart"),
        include_str!("native_v2_io.dart"),
    )
    .unwrap();
    let readme = files.iter().find(|f| f.path == "dart/README.md").unwrap();
    let snippet = readme
        .content
        .split_once("```dart\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let quickstart = files
        .iter()
        .find(|f| f.path == "dart/example/quickstart.dart")
        .unwrap();
    assert_eq!(snippet, quickstart.content.trim_end());
    std::fs::write(consumer.join("bin/readme.dart"), snippet).unwrap();
    std::fs::copy(
        root.join("dart/example/source_examples.dart"),
        consumer.join("bin/source_examples.dart"),
    )
    .unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    for (name, input) in [
        ("consumer-vm", "main"),
        ("readme-vm", "readme"),
        ("examples-vm", "source_examples"),
    ] {
        support::check(
            support::dart(&root)
                .args(["compile", "exe"])
                .arg(format!("bin/{input}.dart"))
                .arg("-o")
                .arg(root.join(name))
                .current_dir(&consumer),
            &root,
            &format!("compile-{name}"),
        );
    }
    support::check(
        &mut Command::new(root.join("examples-vm")),
        &root,
        "examples-run-vm",
    );
    let echo = AtomicUsize::new(0);
    let server = support::Server::start(move |stream, request, _| {
        if request.path == "/v2/rows" {
            let mut rows = WIRE.to_vec();
            rows.extend_from_slice(b"\n{\"stamp\":null}\n");
            support::reply(stream, 200, "application/x-ndjson", &rows);
        } else if request.path == "/v2/echo" && echo.fetch_add(1, Ordering::SeqCst) == 1 {
            support::reply(stream, 200, "application/json", br#"{"stamp":null}"#);
        } else {
            support::reply(
                stream,
                200,
                "application/json",
                if request.body.is_empty() {
                    b"null"
                } else {
                    &request.body
                },
            );
        }
    });
    support::check(
        Command::new(root.join("consumer-vm")).env("DART_V2_BASE", format!("{}/v2", server.url)),
        &root,
        "consumer-run-vm",
    );
    support::check(
        Command::new(root.join("readme-vm")).env("API_SERVER", format!("{}/v2", server.url)),
        &root,
        "readme-run-vm",
    );
    {
        let records = server.records.lock().unwrap();
        assert_eq!(
            records.len(),
            7,
            "actual native calls plus the exact README quickstart"
        );
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/v2/echo");
        assert_eq!(records[0].body, WIRE);
        assert_eq!(records[1].path, "/v2/patch");
        assert!(records[1].body.is_empty());
        assert!(
            !records[1]
                .headers
                .iter()
                .any(|(name, _)| name == "content-type")
        );
        assert_eq!(records[2].path, "/v2/patch");
        assert_eq!(records[2].body, b"null");
        assert_eq!(records[3].path, "/v2/extras");
        assert_eq!(records[3].body, MIXED);
        assert_eq!(records[4].body, WIRE);
        assert_eq!(records[5].method, "GET");
        assert_eq!(records[5].path, "/v2/rows");
        assert_eq!(records[6].body, WIRE);
        std::fs::write(
            root.join("wire-records.json"),
            serde_json::to_vec_pretty(&*records).unwrap(),
        )
        .unwrap();
    }
    drop(server);
    for (name, input) in [("portable", "portable"), ("examples", "source_examples")] {
        support::check(
            support::dart(&root)
                .args(["compile", "js"])
                .arg(format!("bin/{input}.dart"))
                .arg("-o")
                .arg(root.join(format!("{name}.js")))
                .current_dir(&consumer),
            &root,
            &format!("compile-{name}-js"),
        );
        support::node(&root, &format!("{name}.js"), &format!("run-{name}-js"));
    }
    support::check(
        support::dart(&root)
            .args(["doc", "--validate-links", "--output"])
            .arg(root.join("dartdoc"))
            .current_dir(root.join("dart")),
        &root,
        "dartdoc",
    );
    let doclog = std::fs::read_to_string(root.join("logs/dartdoc.log"))
        .unwrap()
        .to_lowercase();
    assert!(!doclog.lines().any(|l|l.trim_start().starts_with("warning:") || l.trim_start().starts_with("error:")),"{doclog}");
    for (path, text) in [
        ("Patch/Patch.html", "xN"),
        ("MixedExtras/extraFields.html", "JsonValue"),
        ("Flexible/Flexible.fromJson.html", "JsonValue"),
        ("Client/scopedEcho.html", "Envelope"),
        ("Client/scopedRows.html", "Stream"),
    ] {
        let doc = root.join("dartdoc/generated_sdk").join(path);
        assert!(
            std::fs::read_to_string(&doc)
                .unwrap_or_else(|e| panic!("{}: {e}", doc.display()))
                .contains(text),
            "rendered {path}"
        );
    }
    for (n, (source, code)) in [
        (
            "void f(Envelope value) { value.stamp = 1; }",
            "invalid_assignment",
        ),
        (
            "void main() { Patch(kind: 'cash'); }",
            "missing_required_argument",
        ),
        (
            "void main() { MixedExtras(sRequired: 'not a JSON carrier'); }",
            "argument_type_not_assignable",
        ),
        (
            "void f(Envelope value) { value.optional = null; }",
            "invalid_assignment",
        ),
        (
            "void f(Envelope value) { value.flexible = null; }",
            "invalid_assignment",
        ),
        (
            "void f(MixedExtras value) { value.extraFields['s-null'] = null; }",
            "invalid_assignment",
        ),
        (
            "void f(Sequence value) { value.value = const JsonNull(); }",
            "assignment_to_final",
        ),
        (
            "void f(Client client) { client.scopedRows().then((value) {}); }",
            "undefined_method",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = consumer.join(format!("negative-{n}.dart"));
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
            &format!("negative-{n}"),
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            !result.status.success() && text.contains(code) && !text.contains("uri_does_not_exist"),
            "{text}"
        );
    }
}
