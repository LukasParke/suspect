//! Maintained closed-provider, source-driven resource/dynamic runtime witnesses.
use super::{
    codegen, emit, support, validation,
    validation_tests::{locations, outcome},
};
use serde_json::{Value, json};
use std::{fmt::Write, path::Path, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction as I, ProgramSource};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.dart.test/api.json";
const OFFICIAL: &str = "https://physical.dart.test/official-schema.json";
fn id(uri: &str, pointer: &str) -> SchemaId {
    pointer.split('/').skip(1).fold(
        SchemaId::new(Uri::parse(uri).unwrap(), Default::default()),
        |id, key| id.child(&key.replace("~1", "/").replace("~0", "~")),
    )
}
fn root(name: &str) -> SchemaId {
    id(ENTRY, "/components/schemas").child(name)
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Dart resource runtime","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn load(document: Value, external: Vec<(&str, Value)>) -> Arc<Contract> {
    let mut docs = vec![(ENTRY, document)];
    docs.extend(external);
    let provider = Arc::new(
        DocumentProvider::new(docs.into_iter().map(|(uri, value)| {
            ProvidedDocument::new(
                Uri::parse(uri).unwrap(),
                Uri::parse(uri).unwrap(),
                serde_json::to_vec(&value).unwrap(),
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
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap());
    assert!(workspace.failed_document_uris().is_empty());
    contract
}
struct Case {
    name: String,
    contract: Arc<Contract>,
    roots: Vec<SchemaId>,
    selected: SchemaId,
    value: Value,
    expected: &'static str,
    config: Config,
    steps: Option<usize>,
    sweep: usize,
}
fn control(
    name: &str,
    schemas: Value,
    selected: &str,
    value: Value,
    expected: &'static str,
) -> Case {
    Case {
        name: name.into(),
        contract: load(api(schemas), vec![]),
        roots: vec![root(selected)],
        selected: root(selected),
        value,
        expected,
        config: Config::default(),
        steps: None,
        sweep: 0,
    }
}
fn controls() -> Vec<Case> {
    let mut cases = Vec::new();
    let trees = json!({"Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#node"}}}},"Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},"Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},"Unentered":{"$id":"urn:aaa","$dynamicAnchor":"node","not":{}}});
    for (label, value, expected) in [
        (
            "outermost-wins",
            json!({"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}),
            "valid",
        ),
        (
            "outermost-required",
            json!({"outer":true,"middle":true,"children":[{"middle":true}]}),
            "invalid",
        ),
    ] {
        let mut c = control(label, trees.clone(), "Outer", value, expected);
        c.roots.push(root("Unentered"));
        cases.push(c);
    }
    let modes = json!({"Base":{"$id":"urn:base","type":"string","$defs":{"Target":{"$dynamicAnchor":"node","$anchor":"plain","type":"string"}}},"Outer":{"$id":"urn:outer","$defs":{"Override":{"$dynamicAnchor":"node","type":"integer"}},"properties":{"dynamic":{"$dynamicRef":"urn:base#node"},"encoded":{"$dynamicRef":"urn:base#n%6fde"},"pointer":{"$dynamicRef":"urn:base#/$defs/Target"},"plain":{"$dynamicRef":"urn:base#plain"},"empty":{"$dynamicRef":"urn:base#"},"static":{"$ref":"urn:base#node"}}}});
    cases.push(control(
        "fallback-modes",
        modes.clone(),
        "Outer",
        json!({"dynamic":1,"encoded":2,"pointer":"s","plain":"s","empty":"s","static":"s"}),
        "valid",
    ));
    for name in ["pointer", "plain", "empty", "static"] {
        cases.push(control(
            &format!("no-override-{name}"),
            modes.clone(),
            "Outer",
            json!({name:1}),
            "invalid",
        ));
    }
    let scopes = json!({"Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},"Outer":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:new-context"}},"NewContext":{"$id":"urn:new-context","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"},"Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},"Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},"Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]}});
    cases.push(control(
        "changed-context-is-not-a-cycle",
        scopes.clone(),
        "Outer",
        json!(7),
        "valid",
    ));
    cases.push(control(
        "failed-trial-restores-resources",
        scopes,
        "Trial",
        json!("s"),
        "valid",
    ));
    let outer = "https://physical.dart.test/outer.json";
    let base = "https://physical.dart.test/base.json";
    let nested = load(
        api(json!({"Use":{"$ref":format!("{outer}#/$defs/start")}})),
        vec![
            (
                outer,
                json!({"$id":"urn:outer","const":false,"$defs":{"binding":{"$dynamicAnchor":"node","type":"integer"},"start":{"$ref":"urn:base#/$defs/use"}}}),
            ),
            (
                base,
                json!({"$id":"urn:base","$defs":{"node":{"$dynamicAnchor":"node","type":"string"},"use":{"$dynamicRef":"#node"}}}),
            ),
        ],
    );
    for (name, value, expected) in [
        ("nested-entry-enters-indexed-resource", json!(7), "valid"),
        ("nested-entry-override", json!("s"), "invalid"),
    ] {
        cases.push(Case {
            name: name.into(),
            contract: nested.clone(),
            roots: vec![id(outer, "/$defs/start")],
            selected: id(outer, "/$defs/start"),
            value,
            expected,
            config: Config::default(),
            steps: None,
            sweep: 0,
        });
    }
    for (n, schema) in [
        json!({"if":{"$dynamicRef":"urn:number#node"},"then":true,"else":true}),
        json!({"not":{"$dynamicRef":"urn:number#node"}}),
        json!({"anyOf":[true,{"$dynamicRef":"urn:number#node"}]}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = control(
            &format!("noninvertible-numeric-{n}"),
            json!({"Use":schema,"Number":{"$id":"urn:number","$dynamicAnchor":"node","maximum":0}}),
            "Use",
            json!(12345),
            "evaluationFailure",
        );
        c.config.max_number_bytes = 3;
        c.config.max_errors = 1;
        cases.push(c);
    }
    cases.push(control("context-cycle-failure",json!({"Root":{"$id":"urn:cycle","$dynamicAnchor":"node","anyOf":[true,{"$dynamicRef":"#node"}]}}),"Root",Value::Null,"evaluationFailure"));
    let mut equality = control(
        "shared-equality-failure",
        json!({"Use":{"not":{"$dynamicRef":"urn:const#node"}},"Const":{"$id":"urn:const","$dynamicAnchor":"node","const":"s"}}),
        "Use",
        json!("s"),
        "evaluationFailure",
    );
    equality.config.max_equality_steps = 0;
    cases.push(equality);
    for (name, schemas, steps) in [
        (
            "one-resource-entry",
            json!({"Root":{"$id":"urn:root","type":"string"}}),
            3,
        ),
        (
            "same-resource-lookup",
            json!({"Root":{"$id":"urn:root","$defs":{"binding":{"$dynamicAnchor":"x","type":"string"}},"$dynamicRef":"#x"}}),
            7,
        ),
        (
            "lookup-before-fallback-entry",
            json!({"Root":{"$id":"urn:root","$dynamicRef":"urn:target#x"},"Target":{"$id":"urn:target","$dynamicAnchor":"x","type":"string"}}),
            7,
        ),
        (
            "empty-anchor-does-not-scan",
            json!({"Root":{"$id":"urn:root","$dynamicRef":"urn:target#"},"Target":{"$id":"urn:target","type":"string"}}),
            6,
        ),
    ] {
        let mut c = control(name, schemas, "Root", json!("s"), "valid");
        c.steps = Some(steps);
        c.sweep = steps + 2;
        cases.push(c);
    }
    let mut schemas = serde_json::Map::new();
    for n in 0..150 {
        schemas.insert(
            format!("N{n}"),
            json!({"$id":format!("urn:n{n}"),"$ref":format!("urn:n{}",n+1)}),
        );
    }
    schemas.insert("N150".into(), json!({"$id":"urn:n150"}));
    let mut depth = control(
        "distinct-resource-depth",
        Value::Object(schemas),
        "N0",
        Value::Null,
        "evaluationFailure",
    );
    depth.config.max_depth = 128;
    cases.push(depth);
    cases
}

#[test]
fn checked_v3_resources_and_static_bytes() {
    let c = control(
        "guard",
        json!({"Tree":{"$id":"urn:tree","$dynamicAnchor":"node","properties":{"child":{"$dynamicRef":"#node"}}},"Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false}}),
        "Strict",
        json!({}),
        "valid",
    );
    let program = OwnedCompiler::new(c.config)
        .compile_v3(c.contract, &c.roots)
        .unwrap()
        .program();
    validation::runtime(&program).unwrap();
    validation::render(&program).unwrap();
    for mutation in 0..10 {
        let mut bad = program.clone();
        match mutation {
            0 => bad.resource_context = None,
            1 => {
                bad.version = OwnedProgram::V2_VERSION;
                bad.profile = OwnedProgram::V2_PROFILE;
            }
            2 => {
                bad.resource_context.as_mut().unwrap().node_scopes.pop();
            }
            3 => bad.resource_context.as_mut().unwrap().node_scopes[0].0 = usize::MAX,
            4 => bad.resource_context.as_mut().unwrap().node_scopes[0].2 = "urn:wrong".into(),
            5 => bad.resource_context.as_mut().unwrap().resources[0]
                .aliases
                .clear(),
            6 => {
                let context = bad.resource_context.as_mut().unwrap();
                let uri = context.resources[0].canonical_uri.clone();
                context.resources[1].aliases.push(uri);
            }
            7 => {
                bad.resource_context
                    .as_mut()
                    .unwrap()
                    .resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap()
                    .dynamic_anchors[0]
                    .2 = usize::MAX
            }
            8 => {
                let check = bad
                    .nodes
                    .iter_mut()
                    .flat_map(|n| &mut n.checks)
                    .find(|c| matches!(c.instruction, I::DynamicRef { .. }))
                    .unwrap();
                if let I::DynamicRef {
                    initial_resource, ..
                } = &mut check.instruction
                {
                    *initial_resource = usize::MAX;
                }
            }
            _ => {
                let check = bad
                    .nodes
                    .iter_mut()
                    .flat_map(|n| &mut n.checks)
                    .find(|c| matches!(c.instruction, I::DynamicRef { .. }))
                    .unwrap();
                if let I::DynamicRef { anchor, .. } = &mut check.instruction {
                    *anchor = Some("not-indexed".into());
                }
            }
        }
        assert!(validation::runtime(&bad).is_err(), "mutation {mutation}");
        assert!(validation::render(&bad).is_err());
    }
    for schema in [
        json!({"type":"string"}),
        json!({"properties":{"a":true},"unevaluatedProperties":false}),
    ] {
        let c = load(api(json!({"Root":schema})), vec![]);
        let program = OwnedCompiler::new(Config::default())
            .compile_v2(c, &[root("Root")])
            .unwrap()
            .program();
        let text = validation::runtime(&program).unwrap();
        assert_eq!(
            text,
            if program.version == OwnedProgram::V1_VERSION {
                include_str!("validation.dart")
            } else {
                include_str!("validation_v2.dart")
            }
        );
        assert!(
            !validation::render(&program)
                .unwrap()
                .contains("_nodeResources")
        );
    }
}

#[test]
#[ignore = "44 unmodified official resource/dynamic cases and independent scoped controls, installed VM/JS"]
fn native_v3_source_resources() {
    let root_path = support::root("v3-vectors-");
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut cases = Vec::new();
    for group in groups {
        let c=load(api(json!({"Use":{"$ref":OFFICIAL}})),vec![
            (OFFICIAL,group["schema"].clone()),
            ("http://localhost:1234/draft2020-12/tree.json",serde_json::from_str(include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/tree.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",serde_json::from_str(include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json",serde_json::from_str(include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json")).unwrap()),
        ]);
        for test in group["tests"].as_array().unwrap() {
            cases.push(Case {
                name: format!(
                    "{} / {}",
                    group["description"].as_str().unwrap(),
                    test["description"].as_str().unwrap()
                ),
                contract: c.clone(),
                roots: vec![id(OFFICIAL, "")],
                selected: id(OFFICIAL, ""),
                value: test["data"].clone(),
                expected: if test["valid"].as_bool().unwrap() {
                    "valid"
                } else {
                    "invalid"
                },
                config: Config::default(),
                steps: None,
                sweep: 0,
            });
        }
    }
    assert_eq!(cases.len(), 44);
    cases.extend(controls());
    execute_cases(&root_path, cases);
}
fn execute_cases(root_path: &Path, cases: Vec<Case>) {
    let mut files=vec![codegen::OutFile{path:"dart/pubspec.yaml".into(),content:"name: generated_sdk\nversion: 0.0.0\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\n".into()},codegen::OutFile{path:"dart/analysis_options.yaml".into(),content:"analyzer:\n  language:\n    strict-casts: true\n    strict-inference: true\n    strict-raw-types: true\n".into()}];
    let mut main = String::new();
    let mut calls = String::new();
    let mut evidence = Vec::new();
    for (index, c) in cases.iter().enumerate() {
        let schema = OwnedCompiler::new(c.config.clone())
            .compile_v3(c.contract.clone(), &c.roots)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
        let program = schema.program();
        let root = program
            .roots
            .iter()
            .find(|r| r.source == ProgramSource::from(&c.selected))
            .unwrap()
            .target;
        let (status, findings) = outcome(schema.validate(&c.selected, &c.value));
        assert_eq!(status, c.expected, "{}", c.name);
        let mut library = format!(
            "library;\nimport 'dart:collection';\nimport 'dart:convert' show utf8;\n{}\n{}\n{}\n",
            include_str!("json.dart"),
            validation::runtime(&program).unwrap(),
            validation::render(&program).unwrap()
        );
        library.push_str("void verifyResult(_ValidationSession session, ValidationResult result, ValidationStatus expected, List<(SchemaSource,String)> locations) {\n if(result.status!=expected||result.findings.length!=locations.length)throw StateError('outcome ${result.status} expected $expected');\n for(var i=0;i<locations.length;i++){if(result.findings[i].source!=locations[i].$1||result.findings[i].instancePath!=locations[i].$2)throw StateError('location ${result.findings[i].source} ${result.findings[i].instancePath}');}\n if(session.resourceStack.isNotEmpty||session.enteredResources.isNotEmpty||session.context!=0||session.depth!=0||session.scopedActive.values.any((v)=>v.isNotEmpty))throw StateError('resource context leaked');\n}\n");
        writeln!(library,"void verify() {{\n final value=parseJson({});final session=_ValidationSession();\n verifyResult(session, session.validate({root},value),ValidationStatus.{status},{});",emit::quote(&c.value.to_string()),locations(&findings)).unwrap();
        if let Some(steps) = c.steps {
            writeln!(library," if(_validationLimits.maxEvaluationSteps-session.steps!={steps})throw StateError('step count ${{_validationLimits.maxEvaluationSteps-session.steps}}');").unwrap();
        }
        let mut sweep = Vec::new();
        if c.sweep > 0 {
            for budget in 0..=c.sweep {
                let mut config = c.config.clone();
                config.max_evaluation_steps = budget;
                let schema = OwnedCompiler::new(config)
                    .compile_v3(c.contract.clone(), &c.roots)
                    .unwrap();
                let (status, findings) = outcome(schema.validate(&c.selected, &c.value));
                writeln!(library," {{final s=_ValidationSession()..steps={budget};verifyResult(s,s.validate({root},value),ValidationStatus.{status},{});}}",locations(&findings)).unwrap();
                sweep.push(json!({"budget":budget,"expected":status,"locations":findings}));
            }
        }
        writeln!(
            library,
            " print({} + ': ${{_validationLimits.maxEvaluationSteps-session.steps}} steps');\n}}",
            emit::quote(&c.name)
        )
        .unwrap();
        files.push(codegen::OutFile {
            path: format!("dart/lib/case_{index}.dart"),
            content: library,
        });
        writeln!(
            main,
            "import 'package:generated_sdk/case_{index}.dart' as c{index};"
        )
        .unwrap();
        writeln!(calls, "c{index}.verify();").unwrap();
        evidence.push(json!({"id":c.name,"program":program,"rootTarget":root,"instanceJson":c.value.to_string(),"expected":c.expected,"locations":findings,"steps":c.steps,"sweep":sweep}));
    }
    writeln!(
        main,
        "void main() {{ {calls} print('DART_V3_RESOURCES_OK cases={}'); }}",
        cases.len()
    )
    .unwrap();
    std::fs::write(
        root_path.join("source-driven-programs.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    support::install(root_path, &files);
    let consumer = root_path.join("consumer");
    std::fs::write(consumer.join("bin/main.dart"), main).unwrap();
    support::check(
        support::dart(root_path)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        root_path,
        "consumer-analyze",
    );
    support::check(
        support::dart(root_path)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root_path.join("vectors-vm"))
            .current_dir(&consumer),
        root_path,
        "compile-vm",
    );
    support::check(
        &mut std::process::Command::new(root_path.join("vectors-vm")),
        root_path,
        "run-vm",
    );
    support::check(
        support::dart(root_path)
            .args(["compile", "js", "bin/main.dart", "-o"])
            .arg(root_path.join("vectors.js"))
            .current_dir(&consumer),
        root_path,
        "compile-js",
    );
    support::node(root_path, "vectors.js", "run-js");
}
