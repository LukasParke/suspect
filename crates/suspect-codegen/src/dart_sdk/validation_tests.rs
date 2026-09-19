//! Source-driven native witnesses before planner admission of checked v2.
use super::{codegen, emit, support, validation};

use serde_json::{Value, json};
use std::{fmt::Write, path::Path, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome, OwnedProgram, ProgramInstruction as I};

fn source(root: &Path, name: &str, schema: Value) -> (Arc<Contract>, SchemaId) {
    let path = root.join(format!("{name}.json"));
    std::fs::write(&path, json!({"openapi":"3.1.2","info":{"title":"Dart v2 source witnesses","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let contract = support::load(&path);
    let id = SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (contract, id)
}
fn config(case: &Value) -> Config {
    let mut c = Config::default();
    for (name, target) in [
        ("maxDepth", &mut c.max_depth),
        ("maxErrors", &mut c.max_errors),
        ("maxNumberBytes", &mut c.max_number_bytes),
        ("maxEqualitySteps", &mut c.max_equality_steps),
        ("maxEvaluationSteps", &mut c.max_evaluation_steps),
    ] {
        if let Some(n) = case["limits"][name].as_u64() {
            *target = n as usize;
        }
    }
    c
}
pub(super) fn outcome(value: OwnedOutcome) -> (&'static str, Vec<(String, String, String)>) {
    let (kind, findings) = match value {
        OwnedOutcome::Valid => ("valid", vec![]),
        OwnedOutcome::Invalid(findings) => ("invalid", findings),
        OwnedOutcome::EvaluationFailure(finding) => ("evaluationFailure", vec![finding]),
    };
    (
        kind,
        findings
            .into_iter()
            .map(|f| {
                (
                    f.source.document().to_string(),
                    f.source.pointer().to_owned(),
                    f.instance_path.to_path(),
                )
            })
            .collect(),
    )
}
fn expected(value: &str) -> &str {
    match value {
        "Valid" => "valid",
        "Invalid" => "invalid",
        "EvaluationFailure" => "evaluationFailure",
        _ => panic!("unknown fixture outcome"),
    }
}
pub(super) fn locations(findings: &[(String, String, String)]) -> String {
    format!(
        "[{}]",
        findings
            .iter()
            .map(|(document, pointer, path)| format!(
                "(SchemaSource({}, {}), {})",
                emit::quote(document),
                emit::quote(pointer),
                emit::quote(path)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[test]
fn v2_emission_checks_the_complete_envelope_and_every_new_operand() {
    let root = support::root("guards-");
    let (contract, id) = source(
        &root,
        "guards",
        json!({
            "if":true,"then":false,"else":true,"dependentRequired":{"x":["y"]},
            "dependentSchemas":{"x":true},"contains":true,"minContains":0,"maxContains":2,
            "properties":{"a":true},"patternProperties":{"^x":true},"additionalProperties":false,
            "propertyNames":true,"unevaluatedProperties":false,"unevaluatedItems":false
        }),
    );
    let program = OwnedCompiler::new(Config::default())
        .compile_v2(contract.clone(), &[id])
        .unwrap()
        .program();
    assert_eq!(program.version, OwnedProgram::V2_VERSION);
    assert!(
        validation::runtime(&program)
            .unwrap()
            .contains("_Annotations")
    );
    validation::render(&program).unwrap();
    let new_ops = program
        .nodes
        .iter()
        .flat_map(|n| &n.checks)
        .filter(|c| c.instruction.requires_v2())
        .count();
    assert_eq!(new_ops, 9);
    let mut findings = Vec::new();
    for mutation in 0..15 {
        let mut bad = program.clone();
        let checks = &mut bad.nodes[program.roots[0].target].checks;
        match mutation {
            0 => bad.version = "unknown",
            1 => bad.profile = OwnedProgram::V1_PROFILE,
            2 => {
                bad.version = OwnedProgram::V1_VERSION;
                bad.profile = OwnedProgram::V1_PROFILE;
            }
            3 => {
                if let I::If { condition, .. } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::If { .. }))
                    .unwrap()
                    .instruction
                {
                    *condition = usize::MAX;
                }
            }
            4 => {
                if let I::If { then_target, .. } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::If { .. }))
                    .unwrap()
                    .instruction
                {
                    *then_target = Some(program.roots[0].target);
                }
            }
            5 => {
                if let I::DependentRequired { dependencies } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::DependentRequired { .. }))
                    .unwrap()
                    .instruction
                {
                    dependencies[0].1.push("y".into());
                }
            }
            6 => {
                if let I::DependentSchemas { dependencies } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::DependentSchemas { .. }))
                    .unwrap()
                    .instruction
                {
                    dependencies[0].target = 0;
                }
            }
            7 => {
                if let I::Contains { minimum, .. } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::Contains { .. }))
                    .unwrap()
                    .instruction
                {
                    *minimum = Some("1e-400".into());
                }
            }
            8 => {
                if let I::PatternProperties { patterns } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::PatternProperties { .. }))
                    .unwrap()
                    .instruction
                {
                    patterns[0].1.start = usize::MAX;
                }
            }
            9 => {
                if let I::AdditionalPropertiesWithPatterns { declared, .. } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::AdditionalPropertiesWithPatterns { .. }))
                    .unwrap()
                    .instruction
                {
                    declared.clear();
                }
            }
            10 => {
                if let I::PropertyNames { target } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::PropertyNames { .. }))
                    .unwrap()
                    .instruction
                {
                    *target = usize::MAX;
                }
            }
            11 => {
                if let I::UnevaluatedProperties { target } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::UnevaluatedProperties { .. }))
                    .unwrap()
                    .instruction
                {
                    *target = 0;
                }
            }
            12 => {
                if let I::UnevaluatedItems { target } = &mut checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::UnevaluatedItems { .. }))
                    .unwrap()
                    .instruction
                {
                    *target = 0;
                }
            }
            13 => checks.reverse(),
            14 => {
                let check = checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, I::AdditionalPropertiesWithPatterns { .. }))
                    .unwrap();
                if let I::AdditionalPropertiesWithPatterns { declared, target } = &check.instruction
                {
                    check.instruction = I::AdditionalProperties {
                        declared: declared.clone(),
                        target: *target,
                    };
                }
            }
            _ => unreachable!(),
        }
        let error = validation::render(&bad).expect_err("malformed native program must not emit");
        assert!(
            validation::runtime(&bad).is_err(),
            "unchecked executable selection"
        );
        if mutation > 2 {
            assert!(
                error.source.is_some(),
                "located operand {mutation}: {error}"
            );
        }
        findings.push(json!({"mutation":mutation,"error":error.to_string()}));
    }
    std::fs::write(
        root.join("guard-findings.json"),
        serde_json::to_vec_pretty(&findings).unwrap(),
    )
    .unwrap();
}

#[test]
fn base_closures_keep_v1_instructions_runtime_and_rendering() {
    let root = support::root("v1-identity-");
    for (n,schema) in [
        json!({"type":"integer","minimum":0}),
        json!({"type":"object","properties":{"key":{"type":"string","pattern":"^x+$"}},"additionalProperties":false}),
        json!({"if":{"const":12345},"dependentRequired":{"self":["self"]},"unevaluatedItems":true}),
    ].into_iter().enumerate() {
        let (contract,id) = source(&root,&format!("base-{n}"),schema);
        let compiler = OwnedCompiler::new(Config::default());
        let old = compiler.compile(contract.clone(),std::slice::from_ref(&id)).unwrap().program();
        let new = compiler.compile_v2(contract,&[id]).unwrap().program();
        assert_eq!(old,new);
        assert_eq!(new.version,OwnedProgram::V1_VERSION);
        assert_eq!(validation::runtime(&new).unwrap(),include_str!("validation.dart"));
        assert_eq!(validation::render(&old).unwrap(),validation::render(&new).unwrap());
        assert!(!validation::render(&new).unwrap().contains("_Op.ifValue"));
    }
}

#[test]
fn resource_programs_keep_static_envelopes_fenced() {
    let root = support::root("dynamic-refusal-");
    let (contract, id) = source(
        &root,
        "dynamic",
        json!({"$id":"https://scoped.example/root","$dynamicAnchor":"node","$dynamicRef":"#node"}),
    );
    let compiler = OwnedCompiler::new(Config::default());
    let errors = compiler
        .compile_v2(contract.clone(), std::slice::from_ref(&id))
        .err()
        .expect("static compiler must refuse dynamic scope");
    assert!(
        errors
            .iter()
            .any(|e| e.source.pointer().starts_with(id.pointer()) && e.span.is_some())
    );
    let program = compiler.compile_v3(contract, &[id]).unwrap().program();
    program.check().unwrap();
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut old = program.clone();
        old.version = version;
        old.profile = profile;
        assert!(validation::runtime(&old).is_err());
        assert!(validation::render(&old).is_err());
    }
}

fn controls() -> Vec<Value> {
    let case = |id: &str, schema: Value, instance: Value, result: &str, limits: Value| json!({"id":id,"schemaJson":schema.to_string(),"instanceJson":instance.to_string(),"expected":result,"limits":limits});
    vec![
        case(
            "implicit-contains-without-numeric-operand",
            json!({"contains":true}),
            json!([12345]),
            "Valid",
            json!({"maxNumberBytes":0}),
        ),
        case(
            "exact-negative-zero-and-huge-maximum",
            serde_json::from_str(
                r#"{"contains":true,"minContains":-0.0,"maxContains":1e999999999999999999999}"#,
            )
            .unwrap(),
            json!([1]),
            "Valid",
            json!({}),
        ),
        case(
            "exact-huge-minimum",
            serde_json::from_str(r#"{"contains":true,"minContains":1e999999999999999999999}"#)
                .unwrap(),
            json!([1]),
            "Invalid",
            json!({}),
        ),
        case(
            "exact-contains-enum",
            serde_json::from_str(
                r#"{"contains":{"enum":[9007199254740993]},"minContains":1,"maxContains":1}"#,
            )
            .unwrap(),
            serde_json::from_str("[9007199254740993]").unwrap(),
            "Valid",
            json!({}),
        ),
        case(
            "property-name-equality-zero",
            json!({"propertyNames":{"enum":["a"]}}),
            json!({"a":null}),
            "EvaluationFailure",
            json!({"maxEqualitySteps":0}),
        ),
        case(
            "if-shared-equality",
            json!({"if":{"const":1},"then":{"const":1},"else":false}),
            json!(1),
            "EvaluationFailure",
            json!({"maxEqualitySteps":1}),
        ),
        case(
            "contains-shared-equality",
            json!({"contains":{"const":1}}),
            json!([1, 1]),
            "EvaluationFailure",
            json!({"maxEqualitySteps":1}),
        ),
        case(
            "not-cannot-invert-equality-failure",
            json!({"not":{"contains":{"const":1}}}),
            json!([1]),
            "EvaluationFailure",
            json!({"maxEqualitySteps":0}),
        ),
        case(
            "report-cap-cannot-hide-failure",
            json!({"allOf":[{"unevaluatedProperties":false},{"dependentSchemas":{"x":{"properties":{"n":{"type":"integer"}}}}}]}),
            json!({"x":null,"n":12345}),
            "EvaluationFailure",
            json!({"maxErrors":1,"maxNumberBytes":3}),
        ),
        case(
            "zero-errors-means-unlimited",
            json!({"dependentRequired":{"a":["b","c"]}}),
            json!({"a":null}),
            "Invalid",
            json!({"maxErrors":0}),
        ),
        case(
            "temporary-name-recursion",
            json!({"propertyNames":{"$ref":"#/components/schemas/Root/propertyNames"}}),
            json!({"a":1}),
            "EvaluationFailure",
            json!({}),
        ),
        case(
            "not-cannot-invert-recursion",
            json!({"not":{"dependentSchemas":{"x":{"$ref":"#/components/schemas/Root"}}}}),
            json!({"x":null}),
            "EvaluationFailure",
            json!({}),
        ),
        case(
            "decoded-name-escaping",
            json!({"propertyNames":{"maxLength":0}}),
            json!({"a/b~":null}),
            "Invalid",
            json!({}),
        ),
        case(
            "unicode-scalar-visits",
            json!({"patternProperties":{"":{"type":"integer"}}}),
            json!({"\u{10000}":12345,"\u{e000}":12345}),
            "EvaluationFailure",
            json!({"maxNumberBytes":3}),
        ),
        case(
            "name-not-value-cache",
            json!({"propertyNames":{"oneOf":[{"const":"a"},{"const":"b"}]},"unevaluatedProperties":true}),
            json!({"b":"a","a":"b"}),
            "Valid",
            json!({}),
        ),
        case(
            "no-unicode-normalization",
            json!({"propertyNames":{"maxLength":1},"unevaluatedProperties":true}),
            json!({"é":null,"e\u{301}":null}),
            "Invalid",
            json!({}),
        ),
        case(
            "pattern-exclusion-before-value-result",
            json!({"patternProperties":{"^x":false},"additionalProperties":{"type":"integer"}}),
            json!({"x":12345}),
            "Invalid",
            json!({"maxNumberBytes":3}),
        ),
        case(
            "fresh-then-scope",
            json!({"if":{"properties":{"a":true}},"then":{"unevaluatedProperties":false}}),
            json!({"a":1}),
            "Invalid",
            json!({}),
        ),
    ]
}

#[test]
#[ignore = "source-driven installed Dart VM/JS witnesses for all 32 v2 vectors and native controls"]
fn native_v2_source_vectors() {
    let root = support::root("vectors-");
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    let mut cases = fixture["cases"].as_array().unwrap().clone();
    assert_eq!(cases.len(), 32);
    cases.extend(controls());
    // Independent counts from the published node/check/visit/merge algorithm.
    for (id, schema, instance, steps, kind) in [
        (
            "properties-count",
            json!({"properties":{"a":true},"unevaluatedProperties":false}),
            json!({"a":1}),
            8,
            "Valid",
        ),
        (
            "allof-merge-count",
            json!({"allOf":[{"properties":{"a":true}}],"unevaluatedProperties":false}),
            json!({"a":1}),
            13,
            "Valid",
        ),
        (
            "allof-duplicate-merge-count",
            json!({"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            json!({"a":1}),
            21,
            "Valid",
        ),
        (
            "anyof-all-passing-merge-count",
            json!({"anyOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            json!({"a":1}),
            21,
            "Valid",
        ),
        (
            "oneof-no-union-on-overlap",
            json!({"oneOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            json!({"a":1}),
            20,
            "Invalid",
        ),
        (
            "if-moves-and-merges",
            json!({"if":{"properties":{"a":true}},"unevaluatedProperties":false}),
            json!({"a":1}),
            11,
            "Valid",
        ),
        (
            "dependent-schema-two-merges",
            json!({"dependentSchemas":{"a":{"properties":{"a":true}}},"unevaluatedProperties":false}),
            json!({"a":1}),
            13,
            "Valid",
        ),
        (
            "contains-immediate-merge",
            json!({"contains":true,"unevaluatedItems":false}),
            json!([1, 2]),
            13,
            "Valid",
        ),
    ] {
        cases.push(json!({"id":id,"schemaJson":schema.to_string(),"instanceJson":instance.to_string(),"expected":kind,"steps":steps,"sweep":steps+1}));
    }
    cases.push(json!({"id":"nfa-exclusion-budget-sweep","schemaJson":json!({"patternProperties":{"^x":true},"additionalProperties":false}).to_string(),"instanceJson":"{\"x\":null}","expected":"Valid","sweep":100}));
    let mut files = vec![
        codegen::OutFile {path:"dart/pubspec.yaml".into(),content:"name: generated_sdk\nversion: 0.0.0\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\n".into()},
        codegen::OutFile {path:"dart/analysis_options.yaml".into(),content:"analyzer:\n  language:\n    strict-casts: true\n    strict-inference: true\n    strict-raw-types: true\n".into()},
    ];
    let mut main = String::new();
    let mut body = String::new();
    let mut evidence = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let name = case["id"].as_str().unwrap();
        let (contract, id) = source(
            &root,
            name,
            serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap(),
        );
        let config = config(case);
        let compiled = OwnedCompiler::new(config.clone())
            .compile_v2(contract.clone(), std::slice::from_ref(&id))
            .unwrap();
        let program = compiled.program();
        assert_eq!(program.version, OwnedProgram::V2_VERSION, "{name}");
        let instance: Value = serde_json::from_str(case["instanceJson"].as_str().unwrap()).unwrap();
        let (kind, findings) = outcome(compiled.validate(&id, &instance));
        assert_eq!(kind, expected(case["expected"].as_str().unwrap()), "{name}");
        if let Some(pointer) = case["source"].as_str() {
            assert!(
                findings.iter().any(|(_, p, _)| p == pointer),
                "independent source: {name}"
            );
        }
        let mut library = format!(
            "library;\nimport 'dart:collection';\nimport 'dart:convert' show utf8;\n{}\n{}\n{}\n",
            include_str!("json.dart"),
            validation::runtime(&program).unwrap(),
            validation::render(&program).unwrap()
        );
        library.push_str("void expectResult(ValidationResult result, ValidationStatus status, List<(SchemaSource, String)> locations) {\n  if(result.status != status || result.findings.length != locations.length) throw StateError('outcome or finding count: ${result.status} ${result.findings.map((f) => '${f.source} ${f.instancePath}').join(';')}');\n  for(var i=0;i<locations.length;i++){ if(result.findings[i].source != locations[i].$1 || result.findings[i].instancePath != locations[i].$2) throw StateError('finding $i: ${result.findings[i].source} ${result.findings[i].instancePath}'); }\n}\n");
        writeln!(library,"void verify() {{\n  final value = parseJson({});\n  final session = _ValidationSession();\n  final result = session.validate({}, value);\n  expectResult(result, ValidationStatus.{kind}, {});",emit::quote(case["instanceJson"].as_str().unwrap()),program.roots[0].target,locations(&findings)).unwrap();
        if let Some(steps) = case["steps"].as_u64() {
            writeln!(library,"  if(_validationLimits.maxEvaluationSteps - session.steps != {steps}) throw StateError('step count: ${{_validationLimits.maxEvaluationSteps - session.steps}}');").unwrap();
        }
        let mut sweep = Vec::new();
        if let Some(maximum) = case["sweep"].as_u64() {
            for budget in 0..=maximum {
                let mut config = config.clone();
                config.max_evaluation_steps = budget as usize;
                let schema = OwnedCompiler::new(config)
                    .compile_v2(contract.clone(), std::slice::from_ref(&id))
                    .unwrap();
                let (status, locs) = outcome(schema.validate(&id, &instance));
                writeln!(library,"  expectResult((_ValidationSession()..steps = {budget}).validate({}, value), ValidationStatus.{status}, {});",program.roots[0].target,locations(&locs)).unwrap();
                sweep.push(json!({"budget":budget,"expected":status,"locations":locs}));
            }
        }
        writeln!(library,"  print('{}: ${{result.status.name}} steps=${{_validationLimits.maxEvaluationSteps - session.steps}}');\n}}",name).unwrap();
        files.push(codegen::OutFile {
            path: format!("dart/lib/case_{index}.dart"),
            content: library,
        });
        writeln!(
            main,
            "import 'package:generated_sdk/case_{index}.dart' as c{index};"
        )
        .unwrap();
        writeln!(body, "  c{index}.verify();").unwrap();
        evidence.push(
            json!({"id":name,"program":program,"case":case,"locations":findings,"sweep":sweep}),
        );
    }
    writeln!(
        main,
        "void main() {{\n{body}  print('DART_V2_VECTORS_OK cases={}');\n}}",
        cases.len()
    )
    .unwrap();
    std::fs::write(
        root.join("source-driven-programs.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    support::install(&root, &files);
    let consumer = root.join("consumer");
    std::fs::write(consumer.join("bin/main.dart"), main).unwrap();
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
            .arg(root.join("vectors-vm"))
            .current_dir(&consumer),
        &root,
        "compile-vm",
    );
    support::check(
        &mut std::process::Command::new(root.join("vectors-vm")),
        &root,
        "run-vm",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/main.dart", "-o"])
            .arg(root.join("vectors.js"))
            .current_dir(&consumer),
        &root,
        "compile-js",
    );
    support::node(&root, "vectors.js", "run-js");
}
