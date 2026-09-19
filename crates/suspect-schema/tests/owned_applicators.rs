//! Public opt-in v2 compilation, checked program admission and actual execution.
use serde_json::{Value, json};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome, OwnedProgram, ProgramInstruction};
use suspect_source::Uri;

fn load(schema: Value) -> (Arc<Contract>, SchemaId) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path,json!({"openapi":"3.1.2","info":{"title":"Applicator oracle","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let root = SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (contract, root)
}

#[test]
fn independent_shared_v2_vectors_execute_with_real_sources() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/owned-applicators-v2.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    let mut executable = Vec::new();
    assert!(cases.len() >= 17);
    for case in cases {
        let name = case["id"].as_str().unwrap();
        let (contract, root) =
            load(serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap());
        let mut config = Config::default();
        if let Some(cap) = case["limits"]["maxNumberBytes"].as_u64() {
            config.max_number_bytes = cap as usize;
        }
        if let Some(cap) = case["limits"]["maxEvaluationSteps"].as_u64() {
            config.max_evaluation_steps = cap as usize;
        }
        let schema = OwnedCompiler::new(config)
            .compile_v2(contract.clone(), std::slice::from_ref(&root))
            .unwrap_or_else(|errors| panic!("{name}: {errors:?}"));
        let program = schema.program();
        program
            .check()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(program.version, OwnedProgram::V2_VERSION, "{name}");
        for node in &program.nodes {
            let source = SchemaId::new(
                Uri::parse(&node.source.document).unwrap(),
                suspect_low::Pointer::parse(&node.source.pointer).unwrap(),
            );
            assert!(
                contract.schema(&source).is_some(),
                "invented node: {name} {source:?}"
            );
        }
        let mut old = program.clone();
        old.version = OwnedProgram::V1_VERSION;
        old.profile = OwnedProgram::V1_PROFILE;
        assert!(old.check().is_err(), "v1 admitted a new form: {name}");
        let instance = serde_json::from_str(case["instanceJson"].as_str().unwrap()).unwrap();
        let result = schema.validate(&root, &instance);
        let (kind, findings) = match result {
            OwnedOutcome::Valid => ("Valid", vec![]),
            OwnedOutcome::Invalid(findings) => ("Invalid", findings),
            OwnedOutcome::EvaluationFailure(finding) => ("EvaluationFailure", vec![finding]),
        };
        assert_eq!(
            kind,
            case["expected"].as_str().unwrap(),
            "{name}: {findings:?}"
        );
        for finding in &findings {
            assert!(
                contract.source(&finding.source).is_some(),
                "{name}: {finding:?}"
            );
            assert!(contract.source_span(&finding.source).is_some());
        }
        if let Some(expected) = case["source"].as_str() {
            assert!(
                findings
                    .iter()
                    .any(|finding| finding.source.pointer() == expected),
                "{name}: expected {expected}, got {findings:?}"
            );
        }
        executable.push(json!({"id":name,"program":program,"instanceJson":case["instanceJson"],"expected":case["expected"],"source":case.get("source")}));
    }
    if let Some(path) = std::env::var_os("SUSPECT_APPLICATORS_EXPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"format":"suspect.schema.applicators.executable-fixtures.v2","cases":executable})).unwrap()).unwrap();
    }
    eprintln!("{} independent applicator vectors passed", cases.len());
}

#[test]
fn base_program_keeps_frozen_v1_bytes_under_both_entrypoints() {
    let (contract, root) = load(json!({"type":"integer","minimum":0}));
    let compiler = OwnedCompiler::new(Config::default());
    let old = compiler
        .compile(contract.clone(), std::slice::from_ref(&root))
        .unwrap();
    let new = compiler
        .compile_v2(contract.clone(), std::slice::from_ref(&root))
        .unwrap();
    let bytes = serde_json::to_string(&old.program()).unwrap();
    assert_eq!(bytes, serde_json::to_string(&new.program()).unwrap());
    let document = serde_json::to_string(&contract.entry().to_string()).unwrap();
    let expected = format!(
        r#"{{"version":"suspect.validation.experimental.v1","profile":"oas31-jsonschema202012-static-subset","roots":[{{"source":{{"document":{document},"pointer":"/components/schemas/Root"}},"target":0}}],"nodes":[{{"source":{{"document":{document},"pointer":"/components/schemas/Root"}},"checks":[{{"source":{{"document":{document},"pointer":"/components/schemas/Root/minimum"}},"op":"bound","value":"0","maximum":false,"exclusive":false}},{{"source":{{"document":{document},"pointer":"/components/schemas/Root/type"}},"op":"type","types":["integer"]}}]}}],"limits":{{"maxDepth":512,"maxErrors":100,"maxNumberBytes":4096,"maxEqualitySteps":100000,"maxEvaluationSteps":100000}}}}"#
    );
    assert_eq!(bytes, expected);
    new.program().check().unwrap();
}

#[test]
fn v1_compiler_still_refuses_new_applicators() {
    for input in [
        json!({"if":true,"then":false}),
        json!({"contains":true}),
        json!({"unevaluatedProperties":false}),
        json!({"dependentRequired":{"a":["b"]}}),
    ] {
        let (contract, root) = load(input);
        assert!(
            OwnedCompiler::new(Config::default())
                .compile(contract, &[root])
                .is_err()
        );
    }
}

#[test]
fn v2_guard_rejects_wrong_sources_targets_counts_and_pattern_exclusions() {
    let (contract, root) = load(
        json!({"if":true,"then":true,"contains":true,"minContains":0,"patternProperties":{"x":true},"additionalProperties":false,"unevaluatedProperties":false}),
    );
    let original = OwnedCompiler::new(Config::default())
        .compile_v2(contract, &[root])
        .unwrap()
        .program();
    original.check().unwrap();
    let root = original.roots[0].target;
    for mutation in 0..5 {
        let mut program = original.clone();
        match mutation {
            0 => {
                let check = program.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|check| matches!(check.instruction, ProgramInstruction::If { .. }))
                    .unwrap();
                if let ProgramInstruction::If { then_target, .. } = &mut check.instruction {
                    *then_target = Some(root);
                }
            }
            1 => {
                let check = program.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|check| matches!(check.instruction, ProgramInstruction::Contains { .. }))
                    .unwrap();
                if let ProgramInstruction::Contains { minimum, .. } = &mut check.instruction {
                    *minimum = Some("1e-400".into());
                }
            }
            2 => {
                let check = program.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|check| {
                        matches!(
                            check.instruction,
                            ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
                        )
                    })
                    .unwrap();
                if let ProgramInstruction::AdditionalPropertiesWithPatterns { declared, target } =
                    &check.instruction
                {
                    check.instruction = ProgramInstruction::AdditionalProperties {
                        declared: declared.clone(),
                        target: *target,
                    };
                }
            }
            3 => {
                program.nodes[root].checks.reverse();
            }
            _ => {
                program.profile = OwnedProgram::V1_PROFILE;
            }
        }
        assert!(program.check().is_err(), "mutation {mutation}");
    }
}

fn evaluate(schema: Value, instance: Value, config: Config) -> OwnedOutcome {
    let (contract, root) = load(schema);
    let schema = OwnedCompiler::new(config)
        .compile_v2(contract, std::slice::from_ref(&root))
        .unwrap();
    schema.program().check().unwrap();
    schema.validate(&root, &instance)
}

#[test]
fn conditional_branch_and_annotation_scopes_are_independent() {
    assert!(
        matches!(
            evaluate(
                json!({"if":{"properties":{"a":true}},"then":{"unevaluatedProperties":false}}),
                json!({"a":1}),
                Config::default()
            ),
            OwnedOutcome::Invalid(_)
        ),
        "then must not inherit the condition's evaluated set"
    );
    assert!(
        matches!(
            evaluate(
                json!({"if":{"properties":{"a":true}},"unevaluatedProperties":false}),
                json!({"a":1}),
                Config::default()
            ),
            OwnedOutcome::Valid
        ),
        "standalone if contributes annotations when v2 needs them"
    );
    assert!(
        matches!(
            evaluate(
                json!({"allOf":[{"unevaluatedProperties":true}],"unevaluatedProperties":false}),
                json!({"a":1}),
                Config::default()
            ),
            OwnedOutcome::Valid
        ),
        "true unevaluated subschemas must contribute in v2"
    );
    assert!(
        matches!(
            evaluate(
                json!({"if":{"required":["absent"],"properties":{"a":true}},"else":true,"unevaluatedProperties":false}),
                json!({"a":1}),
                Config::default()
            ),
            OwnedOutcome::Invalid(_)
        ),
        "a failed condition cannot contribute annotations"
    );
}

#[test]
fn pattern_exclusions_depend_on_names_not_value_validation() {
    let schema =
        json!({"patternProperties":{"^x":false},"additionalProperties":{"type":"integer"}});
    let config = Config {
        max_number_bytes: 3,
        ..Config::default()
    };
    let result = evaluate(schema.clone(), json!({"x":12345}), config.clone());
    let OwnedOutcome::Invalid(findings) = result else {
        panic!("matched names stay excluded even when their schema fails: {result:?}")
    };
    assert!(findings.iter().all(|finding| {
        finding
            .source
            .pointer()
            .starts_with("/components/schemas/Root/patternProperties/")
    }));
    assert!(matches!(
        evaluate(schema, json!({"other":12345}), config),
        OwnedOutcome::EvaluationFailure(_)
    ));
    assert!(
        matches!(
            evaluate(
                json!({"patternProperties":{"^x+$":true}}),
                json!({"xxxxxxxxxxxxxxxx":true}),
                Config {
                    max_evaluation_steps: 8,
                    ..Config::default()
                }
            ),
            OwnedOutcome::EvaluationFailure(_)
        ),
        "name matching spends the shared NFA/work budget"
    );
}

#[test]
fn property_names_use_full_string_evaluation_and_real_member_paths() {
    let result = evaluate(
        json!({"propertyNames":{"enum":["a"]}}),
        json!({"a":null}),
        Config {
            max_equality_steps: 0,
            ..Config::default()
        },
    );
    let OwnedOutcome::EvaluationFailure(finding) = result else {
        panic!("name equality must use the normal budget")
    };
    assert_eq!(
        finding.source.pointer(),
        "/components/schemas/Root/propertyNames/enum"
    );
    assert_eq!(finding.instance_path.to_path(), "/a");
    let result = evaluate(
        json!({"propertyNames":{"maxLength":0}}),
        json!({"a/b~":null}),
        Config::default(),
    );
    let OwnedOutcome::Invalid(findings) = result else {
        panic!()
    };
    assert_eq!(findings[0].instance_path.to_path(), "/a~1b~0");
    assert!(matches!(
        evaluate(
            json!({"propertyNames":{"if":{"const":"a"},"then":false,"else":true}}),
            json!({"a":7}),
            Config::default()
        ),
        OwnedOutcome::Invalid(_)
    ));
    let recursive = json!({"propertyNames":{"$ref":"#/components/schemas/Root/propertyNames"}});
    assert!(matches!(
        evaluate(recursive.clone(), json!({}), Config::default()),
        OwnedOutcome::Valid
    ));
    assert!(
        matches!(
            evaluate(recursive, json!({"a":1}), Config::default()),
            OwnedOutcome::EvaluationFailure(_)
        ),
        "temporary key identity must remain stable through refs"
    );
}

#[test]
fn contains_counts_are_exact_and_defaults_have_no_invented_numeric_operand() {
    assert!(matches!(
        evaluate(
            json!({"contains":true}),
            json!([12345]),
            Config {
                max_number_bytes: 0,
                ..Config::default()
            }
        ),
        OwnedOutcome::Valid
    ));
    for (text, expected) in [
        (
            r#"{"contains":true,"minContains":-0.0,"maxContains":1e999999999999999999999}"#,
            true,
        ),
        (
            r#"{"contains":true,"minContains":1e999999999999999999999}"#,
            false,
        ),
        (
            r#"{"contains":{"enum":[9007199254740993]},"minContains":1,"maxContains":1}"#,
            true,
        ),
    ] {
        let result = evaluate(
            serde_json::from_str(text).unwrap(),
            serde_json::from_str("[9007199254740993]").unwrap(),
            Config::default(),
        );
        assert!(
            !matches!(result, OwnedOutcome::EvaluationFailure(_)),
            "{result:?}"
        );
        assert_eq!(
            matches!(result, OwnedOutcome::Valid),
            expected,
            "{text}: {result:?}"
        );
    }
    assert!(matches!(
        evaluate(
            json!({"contains":false,"minContains":9,"maxContains":0}),
            Value::Null,
            Config::default()
        ),
        OwnedOutcome::Valid
    ));
}

#[test]
fn work_error_caps_and_false_branches_cannot_suppress_evaluation_failures() {
    let result = evaluate(
        json!({"allOf":[{"unevaluatedProperties":false},{"dependentSchemas":{"x":{"properties":{"n":{"type":"integer"}}}}}]}),
        json!({"x":null,"n":12345}),
        Config {
            max_number_bytes: 3,
            max_errors: 1,
            ..Config::default()
        },
    );
    assert!(
        matches!(result, OwnedOutcome::EvaluationFailure(_)),
        "{result:?}"
    );
    for schema in [
        json!({"not":{"dependentSchemas":{"x":{"$ref":"#/components/schemas/Root"}}}}),
        json!({"anyOf":[true,{"dependentSchemas":{"x":{"$ref":"#/components/schemas/Root"}}}]}),
    ] {
        assert!(matches!(
            evaluate(schema, json!({"x":null}), Config::default()),
            OwnedOutcome::EvaluationFailure(_)
        ));
    }
    assert!(matches!(
        evaluate(
            json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}),
            json!({"next":{}}),
            Config {
                max_depth: 2,
                ..Config::default()
            }
        ),
        OwnedOutcome::EvaluationFailure(_)
    ));
}

#[test]
fn malformed_modern_declarations_remain_invalid_in_v2() {
    for (schema, suffix) in [
        (json!({"if":null,"then":true}), "/if"),
        (
            json!({"dependentRequired":{"x":["a","a"]}}),
            "/dependentRequired/x/1",
        ),
        (json!({"dependentSchemas":{"x":42}}), "/dependentSchemas/x"),
        (json!({"contains":true,"minContains":1.5}), "/minContains"),
        (
            json!({"patternProperties":{"[":true}}),
            "/patternProperties/[",
        ),
        (json!({"propertyNames":[]}), "/propertyNames"),
        (json!({"unevaluatedItems":null}), "/unevaluatedItems"),
    ] {
        let (contract, root) = load(schema);
        let errors = OwnedCompiler::new(Config::default())
            .compile_v2(contract, &[root])
            .err()
            .expect("invalid declaration must not compile");
        assert!(
            errors.iter().any(
                |error| error.kind == suspect_schema::OwnedCompileErrorKind::Invalid
                    && error.source.pointer().ends_with(suffix)
                    && error.span.is_some()
            ),
            "{errors:?}"
        );
    }
}

#[test]
fn v1_inert_forms_keep_their_existing_budget_and_byte_behavior() {
    let (contract, root) = load(
        json!({"if":{"const":12345},"unevaluatedItems":true,"dependentRequired":{"self":["self"]}}),
    );
    let compiler = OwnedCompiler::new(Config {
        max_number_bytes: 0,
        max_equality_steps: 0,
        ..Config::default()
    });
    let old = compiler
        .compile(contract.clone(), std::slice::from_ref(&root))
        .unwrap();
    let new = compiler
        .compile_v2(contract, std::slice::from_ref(&root))
        .unwrap();
    assert_eq!(old.program(), new.program());
    assert_eq!(new.program().version, OwnedProgram::V1_VERSION);
    assert_eq!(
        old.validate(&root, &json!(12345)),
        new.validate(&root, &json!(12345))
    );
    assert!(matches!(
        new.validate(&root, &json!(12345)),
        OwnedOutcome::Valid
    ));
}

#[test]
fn default_depth_limit_fails_cleanly_on_an_ordinary_thread_stack() {
    for modern in [false, true] {
        let mut source =
            json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}}});
        if modern {
            source["unevaluatedProperties"] = json!(false);
        }
        let (contract, root) = load(source);
        let schema = OwnedCompiler::new(Config::default())
            .compile_v2(contract, std::slice::from_ref(&root))
            .unwrap();
        let mut instance = json!({});
        for _ in 0..300 {
            instance = json!({"next":instance});
        }
        let result = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || schema.validate(&root, &instance))
            .unwrap()
            .join()
            .unwrap();
        let OwnedOutcome::EvaluationFailure(finding) = result else {
            panic!("depth exhaustion must be explicit: {result:?}")
        };
        assert!(finding.message.contains("depth"));
    }
}
