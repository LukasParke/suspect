//! Unmodified official source fixtures -> real compile_v3 -> native Python.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{python_json, python_validation};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.example.test/api.json";
fn id(document: &str, pointer: &str) -> SourceId {
    let mut source = SourceId::new(Uri::parse(document).unwrap(), Default::default());
    for token in pointer.split('/').skip(1) {
        source = source.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    source
}
fn root() -> SourceId {
    id(ENTRY, "/components/schemas/Root")
}
fn load(schemas: Value, extra: Vec<(&str, Vec<u8>)>) -> Arc<Contract> {
    let entry = json!({"openapi":"3.2.0","info":{"title":"Native resources","version":"1"},"paths":{},"components":{"schemas":schemas}});
    let provider = Arc::new(
        DocumentProvider::new(
            std::iter::once((ENTRY, serde_json::to_vec(&entry).unwrap()))
                .chain(extra)
                .map(|(uri, bytes)| {
                    ProvidedDocument::new(Uri::parse(uri).unwrap(), Uri::parse(uri).unwrap(), bytes)
                        .unwrap()
                }),
        )
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap())
}
fn compile(contract: Arc<Contract>, selected: &[SourceId], config: Config) -> OwnedProgram {
    let program = OwnedCompiler::new(config)
        .compile_v3(contract, selected)
        .unwrap()
        .program();
    program.check().unwrap();
    assert_eq!(program.version, OwnedProgram::V3_VERSION);
    program
}
#[expect(
    clippy::too_many_arguments,
    reason = "independent vectors retain explicit source and instance-path expectations"
)]
fn record(
    records: &mut Vec<Value>,
    name: &str,
    program: &OwnedProgram,
    selected: &SourceId,
    instance: Value,
    expected: &str,
    source: Option<&str>,
    path: &str,
) {
    let target = program
        .roots
        .iter()
        .find(|root| {
            root.source.document == selected.document().as_str()
                && root.source.pointer == selected.pointer()
        })
        .unwrap()
        .target;
    records.push(json!({"id":name,"program":program,"root":target,"instance":instance.to_string(),"expected":expected,"source":source,"path":path}));
}
fn vectors() -> (OwnedProgram, Value) {
    let fixture: Vec<Value> = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut records = Vec::new();
    let mut first = None;
    for group in fixture {
        let physical = "https://physical.example.test/official-schema.json";
        let contract=load(json!({"Root":{"$ref":physical}}),vec![
            (physical,serde_json::to_vec(&group["schema"]).unwrap()),
            ("http://localhost:1234/draft2020-12/tree.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json").to_vec()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json").to_vec()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json").to_vec()),
        ]);
        let selected = id(physical, "");
        let program = compile(
            contract,
            std::slice::from_ref(&selected),
            Default::default(),
        );
        if first.is_none() {
            first = Some(program.clone());
        }
        for case in group["tests"].as_array().unwrap() {
            let name = format!(
                "{} / {}",
                group["description"].as_str().unwrap(),
                case["description"].as_str().unwrap()
            );
            record(
                &mut records,
                &name,
                &program,
                &selected,
                case["data"].clone(),
                if case["valid"] == true {
                    "Valid"
                } else {
                    "Invalid"
                },
                None,
                "",
            );
        }
    }
    assert_eq!(records.len(), 44);
    let tree = json!({
        "Root":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false},
        "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
        "Unentered":{"$id":"urn:unentered","$dynamicAnchor":"node","not":{}}
    });
    let program = compile(
        load(tree, vec![]),
        &[root(), id(ENTRY, "/components/schemas/Unentered")],
        Default::default(),
    );
    record(
        &mut records,
        "strict-tree-unentered-candidate",
        &program,
        &root(),
        json!({"data":"root","children":[{"data":"child"}]}),
        "Valid",
        None,
        "",
    );
    record(
        &mut records,
        "strict-tree-rejects-child-extra",
        &program,
        &root(),
        json!({"children":[{"unexpected":1}]}),
        "Invalid",
        Some("/components/schemas/Root/unevaluatedProperties"),
        "/children/0/unexpected",
    );
    let outer = load(
        json!({
            "Root":{"$id":"urn:outer","$ref":"urn:middle","$dynamicAnchor":"node","required":["outer"]},
            "Middle":{"$id":"urn:middle","$ref":"urn:base","$dynamicAnchor":"node","required":["middle"]},
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"children":{"items":{"$dynamicRef":"#node"}}}}
        }),
        vec![],
    );
    let program = compile(outer, &[root()], Default::default());
    record(
        &mut records,
        "outermost-active-resource",
        &program,
        &root(),
        json!({"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}),
        "Valid",
        None,
        "",
    );
    record(
        &mut records,
        "outermost-is-not-nearest",
        &program,
        &root(),
        json!({"outer":true,"middle":true,"children":[{"middle":true}]}),
        "Invalid",
        Some("/components/schemas/Root/required"),
        "/children/0",
    );
    let scopes = load(
        json!({
            "Root":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:new-context"}},
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},
            "NewContext":{"$id":"urn:new-context","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"},
            "Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]}
        }),
        vec![],
    );
    let trial = id(ENTRY, "/components/schemas/Trial");
    let program = compile(scopes, &[root(), trial.clone()], Default::default());
    record(
        &mut records,
        "changed-context-revisit",
        &program,
        &root(),
        json!(7),
        "Valid",
        None,
        "",
    );
    record(
        &mut records,
        "failed-trial-restores-context",
        &program,
        &trial,
        json!("value"),
        "Valid",
        None,
        "",
    );
    record(
        &mut records,
        "restored-trial-rejects-wrong-value",
        &program,
        &trial,
        json!(7),
        "Invalid",
        Some("/components/schemas/Trial/anyOf"),
        "",
    );
    let detached = load(
        json!({
            "Root":{"$id":"urn:parent","const":false,"$defs":{"binding":{"$dynamicAnchor":"node","type":"integer"},"start":{"$ref":"urn:base#/$defs/use"}}},
            "Base":{"$id":"urn:base","$defs":{"binding":{"$dynamicAnchor":"node","type":"string"},"use":{"$dynamicRef":"#node"}}}
        }),
        vec![],
    );
    let nested = root().child("$defs").child("start");
    let program = compile(detached, std::slice::from_ref(&nested), Default::default());
    assert!(
        !program
            .nodes
            .iter()
            .any(|node| node.source.pointer == root().pointer())
    );
    record(
        &mut records,
        "nested-entry-enters-indexed-parent",
        &program,
        &nested,
        json!(7),
        "Valid",
        None,
        "",
    );
    record(
        &mut records,
        "nested-entry-uses-parent-binding",
        &program,
        &nested,
        json!("wrong"),
        "Invalid",
        Some("/components/schemas/Root/$defs/binding/type"),
        "",
    );
    for (name, use_site) in [
        (
            "if-failure",
            json!({"if":{"$dynamicRef":"urn:number#node"},"then":true,"else":true}),
        ),
        (
            "anyof-failure",
            json!({"anyOf":[true,{"$dynamicRef":"urn:number#node"}]}),
        ),
        (
            "not-failure",
            json!({"not":{"$dynamicRef":"urn:number#node"}}),
        ),
    ] {
        let program = compile(
            load(
                json!({"Root":use_site,"Number":{"$id":"urn:number","$dynamicAnchor":"node","maximum":0}}),
                vec![],
            ),
            &[root()],
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Default::default()
            },
        );
        record(
            &mut records,
            name,
            &program,
            &root(),
            json!(12345),
            "EvaluationFailure",
            Some("/components/schemas/Number/maximum"),
            "",
        );
    }
    for (budget, expected, source) in [
        (
            2,
            "EvaluationFailure",
            Some("/components/schemas/Root/type"),
        ),
        (3, "Valid", None),
    ] {
        let program = compile(
            load(
                json!({"Root":{"$id":"urn:budget","type":"integer"}}),
                vec![],
            ),
            &[root()],
            Config {
                max_evaluation_steps: budget,
                ..Default::default()
            },
        );
        record(
            &mut records,
            &format!("resource-entry-budget-{budget}"),
            &program,
            &root(),
            json!(1),
            expected,
            source,
            "",
        );
    }
    for (budget, expected, source) in [
        (
            5,
            "EvaluationFailure",
            Some("/components/schemas/Root/$dynamicRef"),
        ),
        (8, "Valid", None),
    ] {
        let contract = load(
            json!({
                "Root":{"$id":"urn:lookup","$defs":{"a":{"$dynamicAnchor":"a","type":"string"},"node":{"$dynamicAnchor":"node","type":"integer"}},"$dynamicRef":"urn:default#node"},
                "Default":{"$id":"urn:default","$dynamicAnchor":"node","type":"string"}
            }),
            vec![],
        );
        let program = compile(
            contract,
            &[root(), root().child("$defs").child("a")],
            Config {
                max_evaluation_steps: budget,
                ..Default::default()
            },
        );
        record(
            &mut records,
            &format!("dynamic-binding-budget-{budget}"),
            &program,
            &root(),
            json!(1),
            expected,
            source,
            "",
        );
    }
    let mut deep = serde_json::Map::new();
    for index in 0..550 {
        deep.insert(
            format!("N{index}"),
            json!({"$id":format!("urn:depth{index}"),"$ref":format!("urn:depth{}",index+1)}),
        );
    }
    deep.insert("N550".into(), json!({"$id":"urn:depth550"}));
    let selected = id(ENTRY, "/components/schemas/N0");
    let program = compile(
        load(Value::Object(deep), vec![]),
        std::slice::from_ref(&selected),
        Default::default(),
    );
    record(
        &mut records,
        "distinct-resource-depth",
        &program,
        &selected,
        Value::Null,
        "EvaluationFailure",
        Some("/components/schemas/N512"),
        "",
    );
    (first.unwrap(), json!({"officialCases":44,"cases":records}))
}

fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires Python 3.11/3.14; actual compile_v3 with closed supplied official documents"]
fn python_v3_executes_official_sources_dynamic_scopes_guards_and_exact_budgets() {
    let root = tempfile::Builder::new()
        .prefix("sdk-python-resources-runtime-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap();
    let (program, records) = vectors();
    let mut files = python_validation::emit(&program).unwrap();
    files.extend(python_json::emit());
    files.push(suspect_codegen::OutFile {
        path: "python/__init__.py".into(),
        content: String::new(),
    });
    suspect_codegen::write_files(&files, &root).unwrap();
    fs::write(
        root.join("python/vectors.json"),
        serde_json::to_vec(&records).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("python/consumer.py"),
        include_str!("python_resources/consumer.py"),
    )
    .unwrap();
    let tools = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-native-python-tools/bin/python");
    for version in ["3.11", "3.14"] {
        checked(
            Command::new(format!("python{version}"))
                .arg("consumer.py")
                .current_dir(root.join("python")),
            &root,
            &format!("native-{version}"),
        );
        checked(
            Command::new(&tools)
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--python-version",
                    version,
                    "--cache-dir",
                ])
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("python")),
            &root,
            &format!("mypy-{version}"),
        );
    }
    println!("Python v3 runtime evidence: {}", root.display());
}
