//! Public owned-program behavior; registered after the workspace checkpoint.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompileErrorKind, OwnedCompiler, OwnedOutcome};
use suspect_source::Uri;

fn snapshot(source: Value, external: &[(&str, &str)]) -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    for (name, content) in external {
        std::fs::write(dir.path().join(name), content).unwrap();
    }
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn api(schemas: Value) -> Value {
    json!({"openapi":"3.1.0", "info":{"title":"Owned validation", "version":"1"}, "components":{"schemas":schemas}})
}

fn id(contract: &Contract, pointer: &str) -> SchemaId {
    SchemaId::new(contract.entry().clone(), Pointer::parse(pointer).unwrap())
}

#[test]
fn tracked_openrouter_patterns_keep_their_owned_public_semantics() {
    let contract = snapshot(
        api(json!({
            "AdvisorName":{"pattern":"^[a-zA-Z0-9 _-]+$"},
            "Domain":{"pattern":r"^[a-z0-9*]([a-z0-9*-]{0,61}[a-z0-9*])?(\.[a-z0-9*]([a-z0-9*-]{0,61}[a-z0-9*])?)*$"},
            "Container":{"pattern":r"^[\w-]+$"},
            "SubagentName":{"pattern":"^[a-zA-Z0-9 _-]+$"}
        })),
        &[],
    );
    let roots = ["AdvisorName", "Domain", "Container", "SubagentName"]
        .map(|name| id(&contract, &format!("/components/schemas/{name}")));
    let validator = OwnedCompiler::new(Config::default())
        .compile(Arc::clone(&contract), &roots)
        .unwrap();
    for (root, accepted, rejected) in [
        (&roots[0], "reviewer", "name\n"),
        (&roots[1], "files.pythonhosted.org", r"pypi\xorg"),
        (&roots[2], "sess_abc123", r"\w"),
        (&roots[3], "summarizer", "é"),
    ] {
        assert!(matches!(
            validator.validate(root, &json!(accepted)),
            OwnedOutcome::Valid
        ));
        assert!(matches!(
            validator.validate(root, &json!(rejected)),
            OwnedOutcome::Invalid(_)
        ));
    }
}

#[test]
fn program_owns_a_finite_external_recursive_closure_and_validates_concurrently() {
    let contract = snapshot(
        api(json!({
            "Root":{"$ref":"nodes.json#/$defs/Node"},
            "Unrelated":{"pattern":"unsupported in this owned subset"}
        })),
        &[(
            "nodes.json",
            r##"{"$defs":{"Node":{"type":"object","required":["value"],"properties":{"value":{"type":"integer"},"next":{"anyOf":[{"type":"null"},{"$ref":"#/$defs/Node"}]}}}}}"##,
        )],
    );
    let root = id(&contract, "/components/schemas/Root");
    let validator = Arc::new(
        OwnedCompiler::new(Config::default())
            .compile(contract, std::slice::from_ref(&root))
            .unwrap(),
    );
    // The source loader, workspace, and files are already gone. The same
    // immutable program validates distinct instances from different threads.
    let threads: Vec<_> = (0..4)
        .map(|value| {
            let validator = Arc::clone(&validator);
            let root = root.clone();
            std::thread::spawn(move || {
                validator.validate(
                    &root,
                    &json!({"value":value,"next":{"value":7,"next":null}}),
                )
            })
        })
        .collect();
    for thread in threads {
        assert!(matches!(thread.join().unwrap(), OwnedOutcome::Valid));
    }
    let OwnedOutcome::Invalid(findings) = validator.validate(&root, &json!({"value":"bad"})) else {
        panic!("a string does not satisfy integer");
    };
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0]
            .source
            .document()
            .as_str()
            .ends_with("/nodes.json")
    );
    assert_eq!(
        findings[0].source.pointer(),
        "/$defs/Node/properties/value/type"
    );
    assert_eq!(findings[0].instance_path.to_path(), "/value");
}

#[test]
fn roots_are_strict_and_unsupported_features_only_block_their_selected_closure() {
    let contract = snapshot(
        api(json!({
            "Safe":{"type":"integer"},
            "KnownButNotSelected":{"type":"string"},
            "Pattern":{"pattern":"x"},
            "Custom":{"$schema":"https://custom.example/schema", "minimum":false},
            "Dynamic":{"$dynamicRef":"#node"}
        })),
        &[],
    );
    let safe = id(&contract, "/components/schemas/Safe");
    let compiler = OwnedCompiler::new(Config::default());
    let program = compiler
        .compile(Arc::clone(&contract), std::slice::from_ref(&safe))
        .unwrap();
    assert!(matches!(
        program.validate(&safe, &json!(7)),
        OwnedOutcome::Valid
    ));
    for pointer in [
        "/components/schemas/Missing",
        "/components/schemas/KnownButNotSelected",
    ] {
        let unselected = id(&contract, pointer);
        let OwnedOutcome::EvaluationFailure(finding) = program.validate(&unselected, &json!(7))
        else {
            panic!("unselected IDs must never validate successfully");
        };
        assert_eq!(finding.source, unselected);
    }
    let absent = id(&contract, "/components/schemas/Missing");
    let errors = compiler
        .compile(Arc::clone(&contract), &[safe, absent.clone()])
        .err()
        .expect("missing roots cannot be omitted");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, OwnedCompileErrorKind::UnknownRoot);
    assert_eq!(errors[0].source, absent);
    let pattern = id(&contract, "/components/schemas/Pattern");
    let pattern_program = compiler
        .compile(Arc::clone(&contract), std::slice::from_ref(&pattern))
        .unwrap();
    assert!(matches!(
        pattern_program.validate(&pattern, &json!("prefix-x-suffix")),
        OwnedOutcome::Valid
    ));
    assert!(matches!(
        pattern_program.validate(&pattern, &json!("absent")),
        OwnedOutcome::Invalid(_)
    ));
    for name in ["Custom", "Dynamic"] {
        let root = id(&contract, &format!("/components/schemas/{name}"));
        let errors = compiler
            .compile(Arc::clone(&contract), std::slice::from_ref(&root))
            .err()
            .expect("unsupported selected closure");
        assert!(
            errors
                .iter()
                .any(|e| e.kind == OwnedCompileErrorKind::Unsupported),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|e| e.source.document() == root.document())
        );
    }
    let mut old = api(json!({"Model":{"type":"string"}}));
    old["openapi"] = json!("3.0.3");
    let old = snapshot(old, &[]);
    let root = id(&old, "/components/schemas/Model");
    let program = compiler
        .compile(old, std::slice::from_ref(&root))
        .expect("OAS 3.0 strings are admitted by the declared normalization profile");
    program.program().check().unwrap();
    assert!(matches!(
        program.validate(&root, &json!("value")),
        OwnedOutcome::Valid
    ));
    assert!(matches!(
        program.validate(&root, &json!(7)),
        OwnedOutcome::Invalid(_)
    ));
}

fn outcome(schema: &str, instance: &str, config: Config) -> OwnedOutcome {
    let contract = snapshot(
        api(json!({"Model":serde_json::from_str::<Value>(schema).unwrap()})),
        &[],
    );
    let root = id(&contract, "/components/schemas/Model");
    let program = OwnedCompiler::new(config)
        .compile(contract, std::slice::from_ref(&root))
        .unwrap();
    program.validate(&root, &serde_json::from_str(instance).unwrap())
}

#[test]
fn exact_numbers_counts_and_equality_keep_their_mathematical_values() {
    for (schema, instance, valid) in [
        (r#"{"type":"integer"}"#, "1e-400", false),
        (r#"{"type":"integer"}"#, "1e400", true),
        (r#"{"type":"integer"}"#, "100e-2", true),
        (
            r#"{"type":"integer"}"#,
            "1.00000000000000000000000001",
            false,
        ),
        (
            r#"{"type":"integer"}"#,
            "-0e-999999999999999999999999999999",
            true,
        ),
        (r#"{"maximum":9007199254740992}"#, "9007199254740993", false),
        (
            r#"{"exclusiveMinimum":18446744073709551616}"#,
            "18446744073709551616",
            false,
        ),
        (r#"{"minimum":1e-400}"#, "0", false),
        (r#"{"multipleOf":0.01}"#, "0.07", true),
        (r#"{"multipleOf":0.01}"#, "0.070000000001", false),
        (r#"{"multipleOf":1e-999999999999999999999999}"#, "1", true),
        (r#"{"maxItems":1e999999999999999999999999}"#, "[]", true),
        (r#"{"minItems":1e999999999999999999999999}"#, "[]", false),
        (r#"{"minItems":-0.0}"#, "[]", true),
        (r#"{"minLength":2,"maxLength":2}"#, r#""é🌟""#, true),
        (r#"{"enum":[]}"#, "null", false),
        (r#"{"enum":[1,1.0]}"#, "10e-1", true),
        (
            r#"{"const":{"a":[1,-0],"b":2}}"#,
            r#"{"b":2.0,"\u0061":[10e-1,0.000]}"#,
            true,
        ),
        (
            r#"{"uniqueItems":true}"#,
            "[9007199254740992,9007199254740993]",
            true,
        ),
        (r#"{"uniqueItems":true}"#, "[1e400,10e399]", false),
    ] {
        let result = outcome(schema, instance, Config::default());
        assert!(
            !matches!(result, OwnedOutcome::EvaluationFailure(_)),
            "evaluation should complete: {schema} {instance}: {result:?}"
        );
        assert_eq!(
            matches!(result, OwnedOutcome::Valid),
            valid,
            "{schema} {instance}: {result:?}"
        );
    }
}

#[test]
fn an_empty_fragment_does_not_change_a_supported_dialect_identity() {
    let contract = snapshot(
        api(
            json!({"Model":{"$schema":"https://json-schema.org/draft/2020-12/schema#","type":"integer"}}),
        ),
        &[],
    );
    let root = id(&contract, "/components/schemas/Model");
    let program = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .unwrap();
    assert!(matches!(
        program.validate(&root, &json!(7)),
        OwnedOutcome::Valid
    ));
}

#[test]
fn selected_nested_roots_cannot_bypass_an_unsupported_enclosing_schema_resource() {
    let contract = snapshot(
        api(
            json!({"Resource":{"$id":"https://example.com/embedded","properties":{"child":{"type":"string"}}}}),
        ),
        &[],
    );
    let root = id(&contract, "/components/schemas/Resource/properties/child");
    let errors = OwnedCompiler::new(Config::default())
        .compile(Arc::clone(&contract), &[root])
        .err()
        .expect("a nested root remains within its enclosing unsupported resource");
    let declaration = id(&contract, "/components/schemas/Resource/$id");
    assert!(errors.iter().any(
        |error| error.kind == OwnedCompileErrorKind::Unsupported && error.source == declaration
    ));
    // Canonical resources are implemented by Contract. Frozen v1 admission
    // still declines them at the actual identifier keyword, independently of
    // the now-retired blanket Contract diagnostic.
    assert!(
        contract
            .resource(&id(&contract, "/components/schemas/Resource"))
            .is_some()
    );
    assert!(
        errors
            .iter()
            .any(|error| error.source == declaration
                && error.span == contract.source_span(&declaration)),
        "retain the original diagnostic range: {errors:?}"
    );
}

#[test]
fn logical_branches_and_error_caps_cannot_hide_incomplete_evaluation() {
    for schema in [
        r#"{"not":{"maximum":0}}"#,
        r#"{"anyOf":[true,{"maximum":0}]}"#,
        r#"{"oneOf":[true,{"maximum":0}]}"#,
        r#"{"allOf":[false,{"maximum":0}]}"#,
    ] {
        let result = outcome(
            schema,
            "1000",
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Config::default()
            },
        );
        let OwnedOutcome::EvaluationFailure(finding) = result else {
            panic!("numeric budget failure cannot become a logical result: {schema}: {result:?}");
        };
        assert!(finding.message.contains("3 source bytes"));
        assert!(finding.source.pointer().ends_with("/maximum"));
    }
    for schema in [
        r#"{"not":{"const":"x"}}"#,
        r#"{"anyOf":[true,{"enum":["x"]}]}"#,
        r#"{"oneOf":[true,{"uniqueItems":true}]}"#,
    ] {
        let input = if schema.contains("uniqueItems") {
            r#"["x","x"]"#
        } else {
            r#""x""#
        };
        assert!(
            matches!(
                outcome(
                    schema,
                    input,
                    Config {
                        max_equality_steps: 0,
                        ..Config::default()
                    }
                ),
                OwnedOutcome::EvaluationFailure(_)
            ),
            "equality budget survives branches: {schema}"
        );
    }
    assert!(matches!(
        outcome(
            r#"{"uniqueItems":false}"#,
            "[1,1]",
            Config {
                max_equality_steps: 0,
                ..Config::default()
            }
        ),
        OwnedOutcome::Valid
    ));
    let contract = snapshot(
        api(json!({"Cycle":{"anyOf":[true,{"$ref":"#/components/schemas/Cycle"}]}})),
        &[],
    );
    let root = id(&contract, "/components/schemas/Cycle");
    let program = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .unwrap();
    assert!(
        matches!(
            program.validate(&root, &json!(7)),
            OwnedOutcome::EvaluationFailure(_)
        ),
        "nonproductive recursion cannot be interpreted as true"
    );

    let contract = snapshot(api(json!({"Model":{"maximum":12345}})), &[]);
    let root = id(&contract, "/components/schemas/Model");
    let errors = OwnedCompiler::new(Config {
        max_number_bytes: 3,
        ..Config::default()
    })
    .compile(contract, &[root])
    .err()
    .expect("schema operand budget is checked at compile time");
    assert_eq!(errors[0].kind, OwnedCompileErrorKind::ResourceLimit);
    assert_eq!(
        errors[0].source.pointer(),
        "/components/schemas/Model/maximum"
    );
    assert!(errors[0].span.is_some());
}

#[test]
fn object_array_and_composition_applicability_never_invent_a_type_or_required_value() {
    for (schema, instance, valid) in [
        (
            r#"{"properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}"#,
            "7",
            true,
        ),
        (
            r#"{"properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}"#,
            r#"{"value":1}"#,
            true,
        ),
        (
            r#"{"properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}"#,
            "{}",
            false,
        ),
        (
            r#"{"properties":{"value":{"type":"integer"}},"additionalProperties":false}"#,
            r#"{"other":1}"#,
            false,
        ),
        (
            r#"{"properties":{"properties":false,"type":{"type":"string"}}}"#,
            r#"{"type":"data"}"#,
            true,
        ),
        (
            r#"{"properties":{"properties":false}}"#,
            r#"{"properties":null}"#,
            false,
        ),
        (
            r#"{"required":["undeclared"]}"#,
            r#"{"undeclared":null}"#,
            true,
        ),
        (r#"{"minItems":2,"items":{"type":"integer"}}"#, "null", true),
        (r#"{"minProperties":2}"#, "[]", true),
        (
            r#"{"prefixItems":[{"type":"string"},{"type":"integer"}],"items":false}"#,
            "[]",
            true,
        ),
        (
            r#"{"prefixItems":[{"type":"string"},{"type":"integer"}],"items":false}"#,
            r#"["x",7]"#,
            true,
        ),
        (
            r#"{"prefixItems":[{"type":"string"},{"type":"integer"}],"items":false}"#,
            r#"["x",7,8]"#,
            false,
        ),
        (
            r#"{"oneOf":[{"type":"number"},{"type":"integer"}]}"#,
            "7",
            false,
        ),
        (
            r#"{"oneOf":[{"type":"number"},{"type":"integer"}]}"#,
            "1.5",
            true,
        ),
        (
            r#"{"allOf":[{"properties":{"a":{}}},{"properties":{"b":{}}}],"additionalProperties":false}"#,
            r#"{"a":1,"b":2}"#,
            false,
        ),
        (r#"{"type":["string","null"],"minLength":1}"#, "null", true),
    ] {
        let result = outcome(schema, instance, Config::default());
        assert_eq!(
            matches!(result, OwnedOutcome::Valid),
            valid,
            "{schema} {instance}: {result:?}"
        );
        assert!(
            !matches!(result, OwnedOutcome::EvaluationFailure(_)),
            "{result:?}"
        );
    }
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT; missing tracked input fails this acceptance test"]
fn tracked_openrouter_index_and_messages_validate_through_owned_contract_identity() {
    let root_dir = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT to the source checkout");
    let path = root_dir.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root_dir).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    drop(workspace);
    let index = id(&contract, "/components/schemas/ChatChoice/properties/index");
    let messages = id(
        &contract,
        "/components/schemas/ChatRequest/properties/messages",
    );
    let program = OwnedCompiler::new(Config::default())
        .compile(contract, &[index.clone(), messages.clone()])
        .unwrap();
    for (value, expected) in [("1e-400", false), ("1e400", true), ("100e-2", true)] {
        let result = program.validate(&index, &serde_json::from_str(value).unwrap());
        assert_eq!(
            matches!(result, OwnedOutcome::Valid),
            expected,
            "tracked index {value}: {result:?}"
        );
    }
    let OwnedOutcome::Invalid(findings) = program.validate(&messages, &json!([])) else {
        panic!("tracked messages requires at least one message");
    };
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].source, messages.child("minItems"));
    assert_eq!(findings[0].instance_path.to_path(), "");
    assert!(matches!(
        program.validate(&messages, &json!([{"role":"user","content":"Hello!"}])),
        OwnedOutcome::Valid
    ));
    let OwnedOutcome::Invalid(findings) =
        program.validate(&messages, &json!([{"role":"unknown","content":"Hello!"}]))
    else {
        panic!("role must match a referenced ChatMessages alternative");
    };
    assert!(findings.iter().any(|f| f.source.pointer()
        == "/components/schemas/ChatMessages/oneOf"
        && f.instance_path.to_path() == "/0"));
    assert!(
        findings
            .iter()
            .all(|f| f.source.document() == messages.document())
    );
}

#[test]
fn malformed_declarations_never_compile_as_noop_assertions() {
    for (schema, suffix) in [
        (r#"{"type":[]}"#, "/type"),
        (r#"{"type":["string","string"]}"#, "/type/1"),
        (r#"{"required":["x","\u0078"]}"#, "/required/1"),
        (r#"{"minItems":1.5}"#, "/minItems"),
        (r#"{"multipleOf":0}"#, "/multipleOf"),
        (r#"{"exclusiveMinimum":true}"#, "/exclusiveMinimum"),
        (r#"{"allOf":[]}"#, "/allOf"),
        (r#"{"prefixItems":[]}"#, "/prefixItems"),
        (r#"{"properties":false}"#, "/properties"),
        (r#"{"items":null}"#, "/items"),
        (r#"{"enum":{}}"#, "/enum"),
        (r#"{"uniqueItems":"yes"}"#, "/uniqueItems"),
        (r#"{"$ref":42}"#, "/$ref"),
        (r#"{"$schema":false}"#, "/$schema"),
        (r#"{"$anchor":"1bad"}"#, "/$anchor"),
    ] {
        let contract = snapshot(
            api(json!({"Model":serde_json::from_str::<Value>(schema).unwrap()})),
            &[],
        );
        let root = id(&contract, "/components/schemas/Model");
        let errors = OwnedCompiler::new(Config::default())
            .compile(contract, &[root])
            .err()
            .unwrap_or_else(|| panic!("invalid declaration compiled: {schema}"));
        assert!(
            errors
                .iter()
                .any(|e| e.kind == OwnedCompileErrorKind::Invalid
                    && e.source.pointer().ends_with(suffix)
                    && e.span.is_some()),
            "{schema}: {errors:?}"
        );
    }
    let contract = snapshot(api(json!({"Format":{"format":"uri"}})), &[]);
    let root = id(&contract, "/components/schemas/Format");
    let errors = OwnedCompiler::new(Config {
        format_assertion: true,
        ..Config::default()
    })
    .compile(contract, &[root])
    .err()
    .expect("format assertion must be explicit unsupported scope");
    assert!(
        errors
            .iter()
            .all(|e| e.kind == OwnedCompileErrorKind::Unsupported)
    );
}

#[test]
fn reference_siblings_and_decoded_property_locations_keep_their_own_constraints() {
    let contract = snapshot(
        api(json!({
            "Base":{"type":"object","properties":{"a/b~":{"type":"integer"}}},
            "Use":{"$ref":"#/components/schemas/Base","required":["needed"],"default":{"needed":7}}
        })),
        &[],
    );
    let root = id(&contract, "/components/schemas/Use");
    let program = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .unwrap();
    let OwnedOutcome::Invalid(findings) = program.validate(&root, &json!({"a/b~":"bad"})) else {
        panic!("reference constraints and siblings must both apply");
    };
    assert!(findings.iter().any(|f| f.source.pointer()
        == "/components/schemas/Base/properties/a~1b~0/type"
        && f.instance_path.to_path() == "/a~1b~0"));
    assert!(
        findings.iter().any(|f| f.source == root.child("required")),
        "default cannot fill absent required fields: {findings:?}"
    );
    assert!(matches!(
        program.validate(&root, &json!({"needed":null,"a/b~":7})),
        OwnedOutcome::Valid
    ));
}

#[test]
fn a_small_acyclic_schema_graph_cannot_expand_into_unbounded_evaluation_work() {
    let mut schemas = serde_json::Map::new();
    schemas.insert("N0".into(), Value::Bool(true));
    for level in 1..=16 {
        let reference = format!("#/components/schemas/N{}", level - 1);
        schemas.insert(
            format!("N{level}"),
            json!({"allOf":[{"$ref":reference},{"$ref":reference}]}),
        );
    }
    let contract = snapshot(api(Value::Object(schemas)), &[]);
    let root = id(&contract, "/components/schemas/N16");
    let program = OwnedCompiler::new(Config {
        max_errors: 1,
        max_equality_steps: 0,
        ..Config::default()
    })
    .compile(contract, std::slice::from_ref(&root))
    .unwrap();
    let result = program.validate(&root, &Value::Null);
    let OwnedOutcome::EvaluationFailure(finding) = result else {
        panic!("finite DAGs need a total work bound, not just recursion depth: {result:?}");
    };
    assert!(finding.message.contains("evaluation steps"), "{finding:?}");
    assert_eq!(finding.source.document(), root.document());
}

#[test]
fn work_limits_survive_logical_trials_and_count_collection_visits() {
    for schema in [
        r#"{"not":{"items":true}}"#,
        r#"{"anyOf":[true,{"items":true}]}"#,
        r#"{"oneOf":[true,{"items":true}]}"#,
        r#"{"allOf":[false,{"items":true}]}"#,
    ] {
        let instance = serde_json::to_string(&vec![0; 100]).unwrap();
        let result = outcome(
            schema,
            &instance,
            Config {
                max_evaluation_steps: 32,
                max_errors: 1,
                ..Config::default()
            },
        );
        assert!(
            matches!(result, OwnedOutcome::EvaluationFailure(_)),
            "work failure cannot be hidden or inverted: {schema}: {result:?}"
        );
    }
    assert!(matches!(
        outcome(
            "true",
            "null",
            Config {
                max_evaluation_steps: 0,
                ..Config::default()
            }
        ),
        OwnedOutcome::EvaluationFailure(_)
    ));
    assert!(matches!(
        outcome(
            r#"{"items":{"type":"integer"}}"#,
            "[1,2,3]",
            Config {
                max_evaluation_steps: 100,
                ..Config::default()
            }
        ),
        OwnedOutcome::Valid
    ));
}

#[test]
fn reference_syntax_must_be_valid_before_trusting_a_resolved_contract_edge() {
    for reference in [
        " #/components/schemas/Target",
        "#/components/schemas/Resource/$defs/With Space",
    ] {
        let contract = snapshot(
            api(json!({
                "Target":{"type":"integer"},
                "Resource":{"$defs":{"With Space":{"type":"integer"}}},
                "Use":{"$ref":reference}
            })),
            &[],
        );
        let root = id(&contract, "/components/schemas/Use");
        let errors = OwnedCompiler::new(Config::default())
            .compile(contract, &[root])
            .err()
            .unwrap_or_else(|| {
                panic!("malformed URI must not be silently repaired: {reference:?}")
            });
        assert!(
            errors
                .iter()
                .any(|e| e.kind == OwnedCompileErrorKind::Invalid
                    && e.source.pointer() == "/components/schemas/Use/$ref"
                    && e.span.is_some()),
            "{errors:?}"
        );
    }
    let contract = snapshot(
        api(
            json!({"Resource":{"$defs":{"With Space":{"type":"integer"}}},"Use":{"$ref":"#/components/schemas/Resource/$defs/With%20Space"}}),
        ),
        &[],
    );
    let root = id(&contract, "/components/schemas/Use");
    let program = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .unwrap();
    assert!(matches!(
        program.validate(&root, &json!(7)),
        OwnedOutcome::Valid
    ));
}
