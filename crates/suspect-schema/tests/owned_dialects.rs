//! Independent normative and adversarial vectors through the public SDK seam.
//!
//! Oracles: OAS 3.0.4 §§4.4, 4.7.23–26; Wright-00 Validation §§4–6;
//! JSON Schema 2020-12 Core §§7–11 and Validation §§6–9;
//! OAS 3.1.2 Schema Object and OAS 3.2.0 §§4.1, 4.24–26.
//! Exact primary links and capability boundaries: docs/SDK-SCHEMA-DIALECTS.md.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{
    Config, OwnedCompileError, OwnedCompileErrorKind, OwnedCompiler, OwnedOutcome, OwnedSchema,
    ProgramInstruction, ProgramType,
};
use suspect_source::Uri;

fn api(version: &str, schemas: Value) -> Value {
    json!({"openapi":version,"info":{"title":"Dialect vectors","version":"1"},"paths":{},"components":{"schemas":schemas}})
}

fn contract(document: Value, external: &[(&str, Value)]) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("api.json");
    std::fs::write(&entry, serde_json::to_vec(&document).unwrap()).unwrap();
    for (name, value) in external {
        std::fs::write(
            directory.path().join(name),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap())
}

fn id(contract: &Contract, pointer: &str) -> SchemaId {
    SchemaId::new(contract.entry().clone(), Pointer::parse(pointer).unwrap())
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn compile(version: &str, schema: Value, config: Config) -> (OwnedSchema, SchemaId) {
    let contract = contract(api(version, json!({"Model":schema})), &[]);
    let root = id(&contract, "/components/schemas/Model");
    let validator = OwnedCompiler::new(config)
        .compile(contract, std::slice::from_ref(&root))
        .unwrap_or_else(|errors| panic!("{version}: {errors:?}"));
    checked_sources(&validator);
    (validator, root)
}

fn checked_sources(validator: &OwnedSchema) {
    let program = validator.program();
    program
        .check()
        .expect("every public compiler output passes portable admission");
    assert_eq!(program.version, "suspect.validation.experimental.v1");
    assert_eq!(program.profile, "oas31-jsonschema202012-static-subset");
    for node in &program.nodes {
        let source = SchemaId::new(
            Uri::parse(&node.source.document).unwrap(),
            Pointer::parse(&node.source.pointer).unwrap(),
        );
        assert!(
            validator.contract().schema(&source).is_some(),
            "no invented schema: {source:?}"
        );
        for check in &node.checks {
            let source = SchemaId::new(
                Uri::parse(&check.source.document).unwrap(),
                Pointer::parse(&check.source.pointer).unwrap(),
            );
            assert!(
                validator.contract().source(&source).is_some(),
                "no invented keyword: {source:?}"
            );
            assert!(validator.contract().source_span(&source).is_some());
        }
    }
}

fn verdict(validator: &OwnedSchema, root: &SchemaId, instance: Value, expected: bool) {
    let actual = match validator.validate(root, &instance) {
        OwnedOutcome::Valid => true,
        OwnedOutcome::Invalid(findings) => {
            assert!(!findings.is_empty());
            for finding in findings {
                assert!(validator.contract().source(&finding.source).is_some());
            }
            false
        }
        OwnedOutcome::EvaluationFailure(finding) => {
            panic!("unexpected incomplete evaluation for {instance}: {finding:?}")
        }
    };
    assert_eq!(actual, expected, "instance: {instance}");
}

fn errors(version: &str, schema: Value) -> Vec<OwnedCompileError> {
    let contract = contract(api(version, json!({"Model":schema})), &[]);
    let root = id(&contract, "/components/schemas/Model");
    OwnedCompiler::new(Config::default())
        .compile(contract, &[root])
        .err()
        .expect("must refuse compilation")
}

fn located(errors: &[OwnedCompileError], kind: OwnedCompileErrorKind, pointer: &str) {
    assert!(
        errors.iter().any(|error| error.kind == kind
            && error.source.pointer() == pointer
            && error.span.is_some()),
        "expected {kind:?} at {pointer}: {errors:?}"
    );
}

#[test]
fn oas30_nullable_only_modifies_the_same_object_type() {
    // OAS 3.0.4 nullable explicitly leaves all other constraints in force.
    for (schema, instance, expected) in [
        (
            r#"{"type":"string","nullable":true,"minLength":1}"#,
            "null",
            true,
        ),
        (
            r#"{"type":"string","nullable":true,"minLength":1}"#,
            r#""""#,
            false,
        ),
        (r#"{"type":"string","nullable":false}"#, "null", false),
        (r#"{"nullable":false}"#, "null", true),
        (r#"{"nullable":true}"#, "7", true),
        (
            r#"{"nullable":true,"allOf":[{"type":"string"}]}"#,
            "null",
            false,
        ),
        (
            r#"{"type":"string","nullable":true,"allOf":[{"type":"string"}]}"#,
            "null",
            false,
        ),
        (
            r#"{"type":"string","nullable":true,"enum":["x"]}"#,
            "null",
            false,
        ),
        (
            r#"{"type":"string","nullable":true,"enum":[null,"x"]}"#,
            "null",
            true,
        ),
        (r#"{"type":"string","enum":[null]}"#, "null", false),
        (
            r#"{"type":"string","nullable":true,"not":{"enum":[null]}}"#,
            "null",
            false,
        ),
        (
            r#"{"type":"array","nullable":true,"items":{},"minItems":1}"#,
            "null",
            true,
        ),
        (
            r#"{"type":"array","nullable":true,"items":{},"minItems":1}"#,
            "[]",
            false,
        ),
        (
            r#"{"oneOf":[{"type":"string","nullable":true},{"type":"integer","nullable":true}]}"#,
            "null",
            false,
        ),
        (
            r#"{"oneOf":[{"type":"string","nullable":true},{"type":"integer","nullable":true}]}"#,
            "7",
            true,
        ),
        (
            r#"{"oneOf":[{"type":"number"},{"type":"integer"}]}"#,
            "1.0",
            false,
        ),
        (
            r#"{"oneOf":[{"type":"number"},{"type":"integer"}]}"#,
            "1e-400",
            true,
        ),
    ] {
        let (validator, root) = compile("3.0.4", value(schema), Config::default());
        verdict(&validator, &root, value(instance), expected);
    }
    let (validator, root) = compile(
        "3.0.4",
        json!({"type":"string","nullable":true}),
        Config::default(),
    );
    let program = validator.program();
    let node = &program.nodes[program.roots[0].target];
    assert_eq!(node.checks.len(), 1);
    assert_eq!(node.checks[0].source.pointer, root.child("type").pointer());
    assert_eq!(
        node.checks[0].instruction,
        ProgramInstruction::Type {
            types: vec![ProgramType::Null, ProgramType::String]
        }
    );
    assert_eq!(
        validator.contract().source(&root.child("type")),
        Some(&json!("string")),
        "Contract is immutable"
    );
}

#[test]
fn oas30_presence_and_null_are_independent_and_defaults_do_not_supply_values() {
    for required in [false, true] {
        for nullable in [false, true] {
            let mut schema = json!({"type":"object","properties":{"a/b~":{"type":"integer","nullable":nullable,"default":7}}});
            if required {
                schema["required"] = json!(["a/b~"]);
            }
            let (validator, root) = compile("3.0.3", schema, Config::default());
            verdict(&validator, &root, json!({}), !required);
            verdict(&validator, &root, json!({"a/b~":null}), nullable);
            verdict(&validator, &root, json!({"a/b~":7}), true);
            let OwnedOutcome::Invalid(findings) =
                validator.validate(&root, &json!({"a/b~":"wrong"}))
            else {
                panic!()
            };
            assert!(
                findings
                    .iter()
                    .any(|finding| finding.instance_path.to_path() == "/a~1b~0"
                        && finding.source == root.child("properties").child("a/b~").child("type"))
            );
        }
    }
}

#[test]
fn oas30_exclusive_flags_lower_exact_bounds_with_original_numeric_provenance() {
    for (schema, instance, expected) in [
        (
            r#"{"minimum":9007199254740992,"exclusiveMinimum":true}"#,
            "9007199254740993",
            true,
        ),
        (
            r#"{"minimum":9007199254740992,"exclusiveMinimum":true}"#,
            "9007199254740992.0",
            false,
        ),
        (
            r#"{"maximum":18446744073709551616,"exclusiveMaximum":true}"#,
            "18446744073709551616",
            false,
        ),
        (
            r#"{"maximum":18446744073709551616,"exclusiveMaximum":true}"#,
            "18446744073709551615",
            true,
        ),
        (
            r#"{"minimum":1e-400,"exclusiveMinimum":true}"#,
            "1e-400",
            false,
        ),
        (
            r#"{"minimum":1e-400,"exclusiveMinimum":false}"#,
            "1e-400",
            true,
        ),
        (
            r#"{"maximum":-0e999,"exclusiveMaximum":true}"#,
            "-1e-400",
            true,
        ),
        (r#"{"maximum":-0e999,"exclusiveMaximum":true}"#, "0", false),
        (
            r#"{"exclusiveMinimum":true,"exclusiveMaximum":false}"#,
            "-1e400",
            true,
        ),
        (r#"{"minimum":1,"exclusiveMinimum":true}"#, "null", true),
        (r#"{"multipleOf":0.01}"#, "0.07", true),
        (r#"{"multipleOf":0.01}"#, "0.070000000001", false),
    ] {
        let (validator, root) = compile("3.0.4", value(schema), Config::default());
        verdict(&validator, &root, value(instance), expected);
    }
    let (validator, root) = compile(
        "3.0.4",
        value(
            r#"{"minimum":9007199254740993,"exclusiveMinimum":true,"maximum":1e999999999999999999,"exclusiveMaximum":false}"#,
        ),
        Config::default(),
    );
    let program = validator.program();
    let checks = &program.nodes[program.roots[0].target].checks;
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().any(
        |check| check.source.pointer == root.child("minimum").pointer()
            && check.instruction
                == ProgramInstruction::Bound {
                    value: "9007199254740993".into(),
                    maximum: false,
                    exclusive: true
                }
    ));
    assert!(
        !checks
            .iter()
            .any(|check| check.source.pointer.ends_with("/exclusiveMinimum"))
    );
    let OwnedOutcome::Invalid(findings) = validator.validate(&root, &value("9007199254740993"))
    else {
        panic!()
    };
    assert_eq!(findings[0].source, root.child("minimum"));
    assert_eq!(
        validator.contract().source(&root.child("exclusiveMinimum")),
        Some(&json!(true))
    );
}

#[test]
fn normalized_bound_admission_keeps_the_v1_source_and_operand_guards() {
    let (validator, _) = compile(
        "3.0.4",
        json!({"minimum":7,"exclusiveMinimum":true}),
        Config::default(),
    );
    let original = validator.program();
    let root = original.roots[0].target;
    for bad_source in [
        "/components/schemas/Model/maximum",
        "/components/schemas/Model/nullable",
        "/components/schemas/Model/minimum/0",
    ] {
        let mut changed = original.clone();
        changed.nodes[root].checks[0].source.pointer = bad_source.into();
        assert!(changed.check().is_err(), "{bad_source}");
    }
    let mut duplicate = original.clone();
    duplicate.nodes[root]
        .checks
        .push(original.nodes[root].checks[0].clone());
    assert!(duplicate.check().is_err());
    let mut malformed = original.clone();
    malformed.nodes[root].checks[0].instruction = ProgramInstruction::Bound {
        value: "true".into(),
        maximum: false,
        exclusive: true,
    };
    assert!(malformed.check().is_err());
    // A valid existing v1 program may have both inclusive and exclusive bounds.
    let (validator, _) = compile(
        "3.1.2",
        json!({"minimum":1,"exclusiveMinimum":2}),
        Config::default(),
    );
    assert_eq!(validator.program().nodes[0].checks.len(), 2);
}

#[test]
fn oas30_reference_objects_ignore_even_invalid_and_unsupported_siblings() {
    let snapshot = contract(
        api(
            "3.0.4",
            json!({
                "Base":{"type":"integer","nullable":true},
                "Use":{
                    "$ref":"#/components/schemas/Base", "$schema":false,"$id":"ignored.json",
                    "$dynamicRef":"#bad", "$vocabulary":false, "type":false,"nullable":"invalid",
                    "enum":["ignored"],"minimum":999999999999u64,"exclusiveMinimum":42,"default":{},
                    "properties":{"Bad":{"$schema":"https://custom.example/schema","$ref":"#/missing","contains":true}},
                    "allOf":[false],"discriminator":false,"xml":false
                }
            }),
        ),
        &[],
    );
    assert!(
        snapshot.has_errors(),
        "exercise raw Contract diagnostics under ignored siblings"
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let validator = OwnedCompiler::new(Config {
        max_number_bytes: 1,
        ..Config::default()
    })
    .compile(Arc::clone(&snapshot), std::slice::from_ref(&root))
    .unwrap();
    checked_sources(&validator);
    assert_eq!(
        validator.program().nodes.len(),
        2,
        "ignored siblings do not expand the executable closure"
    );
    verdict(&validator, &root, json!(7), true);
    verdict(&validator, &root, json!(null), true);
    verdict(&validator, &root, json!("ignored"), false);
    assert!(
        snapshot
            .source(&root.child("properties").child("Bad"))
            .is_some()
    );
    // Explicitly selecting an ignored sibling still requires admitting that
    // source as a schema; reference-object ignoring is not a global whitelist.
    let bad = root.child("properties").child("Bad");
    assert!(
        OwnedCompiler::new(Config::default())
            .compile(snapshot, &[root, bad])
            .is_err()
    );
}

#[test]
fn modern_schema_references_keep_sibling_assertions_in_all_supported_dialects() {
    for version in ["3.1.0", "3.1.2", "3.2.0"] {
        for override_dialect in [false, true] {
            let mut document = api(
                version,
                json!({
                    "Base":{"type":"integer"},
                    "Use":{"$ref":"#/components/schemas/Base","minimum":7,"nullable":true}
                }),
            );
            if override_dialect {
                document["jsonSchemaDialect"] =
                    json!("https://json-schema.org/draft/2020-12/schema");
            }
            let snapshot = contract(document, &[]);
            let root = id(&snapshot, "/components/schemas/Use");
            let validator = OwnedCompiler::new(Config::default())
                .compile(snapshot, std::slice::from_ref(&root))
                .unwrap();
            checked_sources(&validator);
            verdict(&validator, &root, json!(7), true);
            verdict(&validator, &root, json!(6), false);
            verdict(&validator, &root, Value::Null, false);
        }
    }
}

#[test]
fn oas30_external_recursive_references_preserve_decoded_source_identity() {
    let snapshot = contract(
        api("3.0.4", json!({"Use":{"$ref":"models.json#/Node"}})),
        &[(
            "models.json",
            json!({
                "Node":{"type":"object","required":["a/b~"],"properties":{
                    "a/b~":{"type":"integer"},"next":{"$ref":"#/Node","nullable":true}
                }}
            }),
        )],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let validator = OwnedCompiler::new(Config::default())
        .compile(snapshot, std::slice::from_ref(&root))
        .unwrap();
    checked_sources(&validator);
    verdict(&validator, &root, json!({"a/b~":7,"next":{"a/b~":8}}), true);
    verdict(&validator, &root, json!({"a/b~":7,"next":null}), false);
    let OwnedOutcome::Invalid(findings) = validator.validate(&root, &json!({"a/b~":"bad"})) else {
        panic!()
    };
    assert!(
        findings[0]
            .source
            .document()
            .as_str()
            .ends_with("/models.json")
    );
    assert_eq!(findings[0].source.pointer(), "/Node/properties/a~1b~0/type");
    assert_eq!(findings[0].instance_path.to_path(), "/a~1b~0");
}

#[test]
fn oas30_vocabulary_shapes_and_default_type_rules_are_not_modernized_silently() {
    for (schema, suffix) in [
        (json!(true), ""),
        (json!({"type":["string"]}), "/type"),
        (json!({"type":"null"}), "/type"),
        (json!({"type":"array"}), "/type"),
        (json!({"type":"array","items":true}), "/items"),
        (json!({"items":[{}]}), "/items"),
        (json!({"nullable":null}), "/nullable"),
        (
            json!({"minimum":0,"exclusiveMinimum":1}),
            "/exclusiveMinimum",
        ),
        (
            json!({"maximum":0,"exclusiveMaximum":null}),
            "/exclusiveMaximum",
        ),
        (json!({"required":[]}), "/required"),
        (json!({"required":["a","a"]}), "/required/1"),
        (json!({"allOf":[]}), "/allOf"),
        (json!({"anyOf":[true]}), "/anyOf/0"),
        (json!({"oneOf":[false]}), "/oneOf/0"),
        (json!({"not":false}), "/not"),
        (json!({"properties":{"a":true}}), "/properties/a"),
        (json!({"enum":false}), "/enum"),
        (json!({"readOnly":true,"writeOnly":true}), "/writeOnly"),
        (json!({"type":"integer","default":1.25}), "/default"),
        (json!({"type":"string","default":null}), "/default"),
        (json!({"title":false}), "/title"),
    ] {
        located(
            &errors("3.0.4", schema),
            OwnedCompileErrorKind::Invalid,
            &format!("/components/schemas/Model{suffix}"),
        );
    }
    for (keyword, declaration) in [
        (
            "$schema",
            json!("https://json-schema.org/draft/2020-12/schema"),
        ),
        ("$defs", json!({})),
        ("$comment", json!("not in 3.0")),
        ("const", json!(7)),
        ("examples", json!([])),
        ("prefixItems", json!([{}])),
        ("if", json!({})),
        ("then", json!({})),
        ("else", json!({})),
        ("contains", json!({})),
        ("minContains", json!(0)),
        ("dependentRequired", json!({})),
        ("dependentSchemas", json!({})),
        ("propertyNames", json!({})),
        ("patternProperties", json!({})),
        ("unevaluatedProperties", json!(true)),
        ("contentSchema", json!({})),
        ("contentEncoding", json!("base64")),
        ("id", json!("other.json")),
        ("unknownAssertion", json!(true)),
    ] {
        let schema = Value::Object([(keyword.to_owned(), declaration)].into_iter().collect());
        located(
            &errors("3.0.4", schema),
            OwnedCompileErrorKind::Invalid,
            &format!("/components/schemas/Model/{keyword}"),
        );
    }
    for schema in [
        json!({"type":"string","nullable":true,"default":null}),
        value(r#"{"type":"integer","default":100e-2}"#),
        json!({"type":"integer","minimum":10,"default":0}), // type conformity only
        json!({"default":null,"x-data":{"const":true,"$schema":false}}),
        json!({"additionalProperties":false}),
    ] {
        compile("3.0.4", schema, Config::default());
    }
}

#[test]
fn enum_recommendations_are_not_misreported_as_mandatory_restrictions() {
    // Wright-00 §5.20 and 2020-12 §6.1.2 both say SHOULD, not MUST, for
    // nonempty/unique enum entries. AllOf/anyOf/oneOf really MUST be nonempty.
    for version in ["3.0.4", "3.1.2", "3.2.0"] {
        for (schema, instance, expected) in [
            (r#"{"enum":[]}"#, "null", false),
            (r#"{"enum":[1,1.0]}"#, "100e-2", true),
            (r#"{"enum":[null,true,1,"1"]}"#, "null", true),
            (
                r#"{"enum":[9007199254740992,9007199254740993]}"#,
                "9007199254740993",
                true,
            ),
            (r#"{"enum":[9007199254740992]}"#, "9007199254740993", false),
            (r#"{"enum":[{"a":[1,-0]}]}"#, r#"{"a":[1.0,0]}"#, true),
        ] {
            let (validator, root) = compile(version, value(schema), Config::default());
            verdict(&validator, &root, value(instance), expected);
        }
    }
}

#[test]
fn dialect_errors_identify_the_real_declaration_including_inherited_context() {
    for declaration in [
        json!(false),
        json!(null),
        json!(7),
        json!("relative/schema"),
        json!(" https://json-schema.org/draft/2020-12/schema"),
    ] {
        let snapshot = contract(
            {
                let mut document = api(
                    "3.1.2",
                    json!({"Model":{"properties":{"child":{"type":"integer"}}}}),
                );
                document["jsonSchemaDialect"] = declaration;
                document
            },
            &[],
        );
        let root = id(&snapshot, "/components/schemas/Model/properties/child");
        let result = OwnedCompiler::new(Config::default())
            .compile(snapshot, &[root])
            .err()
            .unwrap();
        located(
            &result,
            OwnedCompileErrorKind::Invalid,
            "/jsonSchemaDialect",
        );
    }
    for declaration in [
        json!(false),
        json!("relative/schema"),
        json!("https://json-schema.org/draft/2020-12/schema##"),
    ] {
        let snapshot = contract(
            api(
                "3.1.2",
                json!({"Model":{"$schema":declaration,"properties":{"child":{"type":"integer"}}}}),
            ),
            &[],
        );
        let root = id(&snapshot, "/components/schemas/Model/properties/child");
        let result = OwnedCompiler::new(Config::default())
            .compile(snapshot, &[root])
            .err()
            .unwrap();
        located(
            &result,
            OwnedCompileErrorKind::Invalid,
            "/components/schemas/Model/$schema",
        );
    }
    let snapshot = contract(
        api(
            "3.2.0",
            json!({"Model":{"$schema":"https://spec.openapis.org/oas/3.2/dialect/base","type":"integer"}}),
        ),
        &[],
    );
    let root = id(&snapshot, "/components/schemas/Model");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::Unsupported,
        "/components/schemas/Model/$schema",
    );
    let nested = json!({"properties":{"child":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"integer"}}});
    located(
        &errors("3.1.2", nested),
        OwnedCompileErrorKind::Invalid,
        "/components/schemas/Model/properties/child/$schema",
    );
}

#[test]
fn schema_dialect_override_and_external_document_defaults_are_source_scoped() {
    let mut document = api(
        "3.2.0",
        json!({"Model":{"$schema":"https://json-schema.org/draft/2020-12/schema#","type":["integer","null"]}}),
    );
    document["jsonSchemaDialect"] = json!("https://custom.example/unsupported");
    let snapshot = contract(document, &[]);
    let root = id(&snapshot, "/components/schemas/Model");
    let validator = OwnedCompiler::new(Config::default())
        .compile(snapshot, std::slice::from_ref(&root))
        .unwrap();
    checked_sources(&validator);
    verdict(&validator, &root, Value::Null, true);

    let snapshot = contract(
        api(
            "3.2.0",
            json!({"Use":{"$ref":"models.json#/components/schemas/Model"}}),
        ),
        &[("models.json", {
            let mut document = api("3.1.2", json!({"Model":{"type":"string"}}));
            document["jsonSchemaDialect"] = json!("https://custom.example/external");
            document
        })],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::Unsupported,
        "/jsonSchemaDialect",
    );
    assert!(
        result
            .iter()
            .any(|error| error.source.document().as_str().ends_with("/models.json"))
    );

    // A property *named* $schema must not act as a declaration on its map.
    let (validator, root) = compile(
        "3.1.2",
        json!({"type":"object","properties":{"$schema":{"type":"string"}},"default":{"$schema":false}}),
        Config::default(),
    );
    verdict(&validator, &root, json!({"$schema":"data"}), true);
}

#[test]
fn oas32_annotation_extensions_do_not_change_json_validation_or_oneof_matching() {
    let schema = json!({
        "type":"object", "oneOf":[{"properties":{"kind":{"const":"cat"}}},{"properties":{"kind":{"const":"dog"}}}],
        "discriminator":{"propertyName":"kind","mapping":{"cat":"Cat","dog":"Dog"},"defaultMapping":"Other"},
        "xml":{"nodeType":"element","name":"pet"},"externalDocs":{"url":"https://example.com/docs"}
    });
    let (validator, root) = compile("3.2.0", schema, Config::default());
    verdict(&validator, &root, json!({"kind":"cat"}), true);
    verdict(&validator, &root, json!({"kind":"unknown"}), false);
    verdict(&validator, &root, json!({}), false); // both branches match; defaultMapping cannot pick one
    assert!(
        validator
            .program()
            .nodes
            .iter()
            .flat_map(|node| &node.checks)
            .all(|check| !check.source.pointer.contains("discriminator")
                && !check.source.pointer.contains("xml"))
    );
    for version in ["3.0.4", "3.1.2"] {
        located(
            &errors(
                version,
                json!({"discriminator":{"propertyName":"kind","defaultMapping":"Other"}}),
            ),
            OwnedCompileErrorKind::Invalid,
            "/components/schemas/Model/discriminator/defaultMapping",
        );
        located(
            &errors(version, json!({"xml":{"nodeType":"text"}})),
            OwnedCompileErrorKind::Invalid,
            "/components/schemas/Model/xml/nodeType",
        );
    }
    for (schema, suffix) in [
        (
            json!({"discriminator":{"propertyName":"kind","defaultMapping":false}}),
            "/discriminator/defaultMapping",
        ),
        (
            json!({"xml":{"nodeType":"text","wrapped":false}}),
            "/xml/wrapped",
        ),
        (json!({"xml":{"nodeType":"other"}}), "/xml/nodeType"),
        (json!({"externalDocs":{"url":null}}), "/externalDocs/url"),
    ] {
        located(
            &errors("3.2.0", schema),
            OwnedCompileErrorKind::Invalid,
            &format!("/components/schemas/Model{suffix}"),
        );
    }
}

#[test]
fn known_semantic_features_are_located_refusals_with_concrete_needed_operations() {
    for (schema, keyword, operation, explanation) in [
        (
            json!({"if":{"required":["a"]},"then":{"required":["b"]}}),
            "if",
            "conditional",
            "skipped branches",
        ),
        (
            json!({"dependentRequired":{"a":["b"]}}),
            "dependentRequired",
            "dependentRequired",
            "absent and null",
        ),
        (
            json!({"dependentSchemas":{"a":{"required":["b"]}}}),
            "dependentSchemas",
            "dependentSchemas",
            "entire object",
        ),
        (
            json!({"contains":{"type":"integer"},"minContains":0,"maxContains":2}),
            "contains",
            "contains",
            "match counting",
        ),
        (
            json!({"propertyNames":{"pattern":"^x"}}),
            "propertyNames",
            "propertyNames",
            "property-name string",
        ),
        (
            json!({"patternProperties":{"^x":{"type":"integer"}},"additionalProperties":false}),
            "patternProperties",
            "patternProperties",
            "exclusions",
        ),
        (
            json!({"unevaluatedProperties":false}),
            "unevaluatedProperties",
            "unevaluatedProperties",
            "successful-evaluation",
        ),
        (
            json!({"unevaluatedItems":false}),
            "unevaluatedItems",
            "unevaluatedItems",
            "successful-evaluation",
        ),
        (
            json!({"$dynamicRef":"#node"}),
            "$dynamicRef",
            "dynamicRef",
            "dynamic-scope",
        ),
    ] {
        let result = errors("3.2.0", schema);
        let pointer = format!("/components/schemas/Model/{keyword}");
        located(&result, OwnedCompileErrorKind::Unsupported, &pointer);
        assert!(
            result.iter().any(|error| error.source.pointer() == pointer
                && error.message.contains(&format!("`{operation}` operation"))
                && error.message.contains(explanation)),
            "{result:?}"
        );
    }
}

#[test]
fn known_inert_keywords_are_admitted_only_with_valid_declarations() {
    let schema = json!({
        "minContains":0,"maxContains":0,"patternProperties":{},"dependentSchemas":{},
        "dependentRequired":{"a":[],"b":["b"]},"then":{"const":12345},"else":false,
        "unevaluatedProperties":true,"unevaluatedItems":true,
        "nullable":{"not":"a standard modern keyword"},"customAnnotation":{"contains":false}
    });
    let (validator, root) = compile(
        "3.2.0",
        schema,
        Config {
            max_number_bytes: 1,
            max_equality_steps: 0,
            ..Config::default()
        },
    );
    for instance in [
        json!({}),
        json!({"a":null,"b":null}),
        json!([1, 1]),
        json!("a"),
        json!(12345),
        Value::Null,
    ] {
        verdict(&validator, &root, instance, true);
    }
    let (validator, root) = compile(
        "3.1.2",
        json!({"if":{"const":12345}}),
        Config {
            max_number_bytes: 1,
            max_equality_steps: 0,
            ..Config::default()
        },
    );
    verdict(&validator, &root, json!(12345), true); // no assertions depend on the condition
    for (schema, suffix) in [
        (json!({"minContains":-1}), "/minContains"),
        (value(r#"{"maxContains":1e-400}"#), "/maxContains"),
        (json!({"contains":null}), "/contains"),
        (json!({"if":[]}), "/if"),
        (json!({"then":null}), "/then"),
        (json!({"dependentRequired":[]}), "/dependentRequired"),
        (
            json!({"dependentRequired":{"a":["b","b"]}}),
            "/dependentRequired/a/1",
        ),
        (
            json!({"dependentRequired":{"a":[null]}}),
            "/dependentRequired/a/0",
        ),
        (
            json!({"dependentSchemas":{"a":null}}),
            "/dependentSchemas/a",
        ),
        (json!({"patternProperties":[]}), "/patternProperties"),
        (json!({"propertyNames":null}), "/propertyNames"),
        (json!({"contentSchema":null}), "/contentSchema"),
    ] {
        located(
            &errors("3.1.2", schema),
            OwnedCompileErrorKind::Invalid,
            &format!("/components/schemas/Model{suffix}"),
        );
    }
}

#[test]
fn resources_and_oas30_directional_requirements_cannot_be_widened_by_neutral_compilation() {
    let mut document = api("3.2.0", json!({"Model":{"type":"integer"}}));
    document["$self"] = json!("https://example.com/canonical");
    let snapshot = contract(document, &[]);
    let root = id(&snapshot, "/components/schemas/Model");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(&result, OwnedCompileErrorKind::Unsupported, "/$self");
    assert!(
        result
            .iter()
            .any(|error| error.message.contains("Contract"))
    );
    for (schema, suffix, kind) in [
        (
            json!({"$id":"https://example.com/resource"}),
            "/$id",
            OwnedCompileErrorKind::Unsupported,
        ),
        (
            json!({"$id":"bad uri"}),
            "/$id",
            OwnedCompileErrorKind::Invalid,
        ),
        (
            json!({"$id":"https://example.com/resource#named"}),
            "/$id",
            OwnedCompileErrorKind::Invalid,
        ),
        (
            json!({"$vocabulary":{"https://example.com/vocab":"required"}}),
            "/$vocabulary/https:~1~1example.com~1vocab",
            OwnedCompileErrorKind::Invalid,
        ),
    ] {
        located(
            &errors("3.1.2", schema),
            kind,
            &format!("/components/schemas/Model{suffix}"),
        );
    }
    for schema in [
        json!({"required":["id"],"properties":{"id":{"type":"integer","readOnly":true}}}),
        json!({"allOf":[{"required":["id"]},{"properties":{"id":{"type":"integer","writeOnly":true}}}]}),
    ] {
        let result = errors("3.0.4", schema);
        assert!(
            result
                .iter()
                .any(|error| error.kind == OwnedCompileErrorKind::Unsupported
                    && error.source.pointer().ends_with("/required")
                    && error.message.contains("directional validation view")),
            "{result:?}"
        );
    }
    // Modern annotations do not normatively relax required by direction.
    let (validator, root) = compile(
        "3.2.0",
        json!({"required":["id"],"properties":{"id":{"readOnly":true}}}),
        Config::default(),
    );
    verdict(&validator, &root, json!({}), false);
    verdict(&validator, &root, json!({"id":null}), true);
}

#[test]
fn normalization_does_not_turn_arithmetic_recursion_or_work_failures_into_validity() {
    for schema in [
        json!({"not":{"minimum":0,"exclusiveMinimum":true}}),
        json!({"anyOf":[{}, {"minimum":0,"exclusiveMinimum":true}]}),
        json!({"oneOf":[{}, {"minimum":0,"exclusiveMinimum":true}]}),
        json!({"allOf":[{"not":{}}, {"minimum":0,"exclusiveMinimum":true}]}),
    ] {
        let (validator, root) = compile(
            "3.0.4",
            schema,
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Config::default()
            },
        );
        let OwnedOutcome::EvaluationFailure(finding) = validator.validate(&root, &json!(12345))
        else {
            panic!("numeric limit must survive logical trials")
        };
        assert!(finding.source.pointer().ends_with("/minimum"));
        assert!(finding.message.contains("3 source bytes"));
    }
    let (validator, root) = compile(
        "3.0.4",
        json!({"anyOf":[{}, {"enum":[1]}]}),
        Config {
            max_equality_steps: 0,
            ..Config::default()
        },
    );
    assert!(matches!(
        validator.validate(&root, &json!(1)),
        OwnedOutcome::EvaluationFailure(_)
    ));
    let (validator, root) = compile(
        "3.0.4",
        json!({"anyOf":[{}, {"items":{"type":"integer"}}]}),
        Config {
            max_evaluation_steps: 10,
            ..Config::default()
        },
    );
    assert!(matches!(
        validator.validate(&root, &json!([1, 2, 3, 4, 5])),
        OwnedOutcome::EvaluationFailure(_)
    ));
    let (validator, root) = compile(
        "3.0.4",
        json!({"$ref":"#/components/schemas/Model","nullable":true}),
        Config::default(),
    );
    assert!(matches!(
        validator.validate(&root, &Value::Null),
        OwnedOutcome::EvaluationFailure(_)
    ));
    let snapshot = contract(
        api(
            "3.0.4",
            json!({"Model":{"minimum":12345,"exclusiveMinimum":true}}),
        ),
        &[],
    );
    let root = id(&snapshot, "/components/schemas/Model");
    let result = OwnedCompiler::new(Config {
        max_number_bytes: 3,
        ..Config::default()
    })
    .compile(snapshot, &[root])
    .err()
    .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::ResourceLimit,
        "/components/schemas/Model/minimum",
    );
}

#[test]
fn oas30_patterns_do_not_acquire_modern_unicode_escape_or_codepoint_semantics() {
    // OAS 3.0.4 links ECMA-262 5.1 §15.10. Positive BMP patterns share
    // semantics with the portable Unicode NFA; e.g. ^.$ does not on astral text.
    for (pattern, instance, expected) in [
        (r"^[A-Za-z0-9_-]+$", "name_1", true),
        (r"^[A-Za-z0-9_-]+$", "name\n", false),
        (r"^[A-Za-z0-9_-]+$", "🌟", false),
        (r"^é{2}$", "éé", true),
        (r"^\w+$", "name_1", true),
        (r"^\d{2}$", "12", true),
        (r"^\u0041$", "A", true),
    ] {
        let (validator, root) = compile("3.0.4", json!({"pattern":pattern}), Config::default());
        verdict(&validator, &root, json!(instance), expected);
    }
    for pattern in [
        r"^.$",
        r"^[^a]$",
        r"^🌟+$",
        r"^\u{41}$",
        r"\s",
        r"\S",
        r"\00",
        r"\a",
    ] {
        let result = errors("3.0.4", json!({"pattern":pattern}));
        located(
            &result,
            OwnedCompileErrorKind::Unsupported,
            "/components/schemas/Model/pattern",
        );
        assert!(
            result
                .iter()
                .any(|error| error.message.contains("ECMA-262 5.1"))
        );
    }
    let (validator, root) = compile("3.1.2", json!({"pattern":"^.$"}), Config::default());
    verdict(&validator, &root, json!("🌟"), true);
}

#[test]
fn cross_document_dialects_and_reference_target_shapes_are_admitted_explicitly() {
    let snapshot = contract(
        api(
            "3.2.0",
            json!({"Use":{"$ref":"models.json#/components/schemas/Model"}}),
        ),
        &[(
            "models.json",
            api(
                "3.0.4",
                json!({"Model":{"type":"integer","nullable":true,"minimum":7,"exclusiveMinimum":true}}),
            ),
        )],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let validator = OwnedCompiler::new(Config::default())
        .compile(snapshot, std::slice::from_ref(&root))
        .unwrap();
    checked_sources(&validator);
    verdict(&validator, &root, json!(8), true);
    verdict(&validator, &root, Value::Null, true);
    verdict(&validator, &root, json!(7), false);

    let snapshot = contract(
        api(
            "3.0.4",
            json!({"Use":{"$ref":"models.json#/components/schemas/Model"}}),
        ),
        &[(
            "models.json",
            api("3.1.2", json!({"Model":{"type":["integer","null"]}})),
        )],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::Unsupported,
        "/components/schemas/Use/$ref",
    );

    let snapshot = contract(
        api(
            "3.0.4",
            json!({"Use":{"$ref":"#/components/schemas/Model/additionalProperties"},"Model":{"additionalProperties":false}}),
        ),
        &[],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::Invalid,
        "/components/schemas/Use/$ref",
    );

    let snapshot = contract(
        api(
            "3.2.0",
            json!({"Use":{"$ref":"external.json#/$defs/Model"}}),
        ),
        &[(
            "external.json",
            json!({"$schema":false,"$defs":{"Model":{"type":"integer"}}}),
        )],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let result = OwnedCompiler::new(Config::default())
        .compile(snapshot, &[root])
        .err()
        .unwrap();
    located(&result, OwnedCompileErrorKind::Invalid, "/$schema");
    assert!(
        result
            .iter()
            .any(|error| error.source.document().as_str().ends_with("/external.json"))
    );
}

#[test]
fn ignored_ref_direction_annotations_do_not_change_30_required() {
    let snapshot = contract(
        api(
            "3.0.4",
            json!({
                "Model":{"required":["id"],"properties":{"id":{"$ref":"#/components/schemas/Value","readOnly":true}}},
                "Value":{"type":"integer"}
            }),
        ),
        &[],
    );
    let root = id(&snapshot, "/components/schemas/Model");
    let validator = OwnedCompiler::new(Config::default())
        .compile(snapshot, std::slice::from_ref(&root))
        .unwrap();
    checked_sources(&validator);
    verdict(&validator, &root, json!({}), false);
    verdict(&validator, &root, json!({"id":7}), true);
}

#[test]
fn format_declarations_and_default_integer_budgets_fail_at_the_original_value() {
    for version in ["3.0.4", "3.1.2", "3.2.0"] {
        let snapshot = contract(api(version, json!({"Model":{"format":false}})), &[]);
        let root = id(&snapshot, "/components/schemas/Model");
        let result = OwnedCompiler::new(Config {
            format_assertion: true,
            ..Config::default()
        })
        .compile(snapshot, &[root])
        .err()
        .unwrap();
        located(
            &result,
            OwnedCompileErrorKind::Invalid,
            "/components/schemas/Model/format",
        );
        let (validator, root) = compile(version, json!({"format":"email"}), Config::default());
        verdict(&validator, &root, json!("not an email"), true);
        verdict(&validator, &root, json!(42), true);
    }
    let snapshot = contract(
        api("3.0.4", json!({"Model":{"type":"integer","default":12345}})),
        &[],
    );
    let root = id(&snapshot, "/components/schemas/Model");
    let result = OwnedCompiler::new(Config {
        max_number_bytes: 3,
        ..Config::default()
    })
    .compile(snapshot, &[root])
    .err()
    .unwrap();
    located(
        &result,
        OwnedCompileErrorKind::ResourceLimit,
        "/components/schemas/Model/default",
    );
    // 2020-12 defaults are annotations even when not of the declared type.
    let (validator, root) = compile(
        "3.1.2",
        json!({"type":"integer","default":"text"}),
        Config::default(),
    );
    verdict(&validator, &root, json!(7), true);
}

#[test]
fn self_is_only_a_base_uri_field_on_an_oas32_openapi_object() {
    let snapshot = contract(
        api("3.2.0", json!({"Use":{"$ref":"schema.json"}})),
        &[(
            "schema.json",
            json!({
                "$schema":"https://json-schema.org/draft/2020-12/schema",
                "$self":{"arbitrary":"annotation"},"type":"integer"
            }),
        )],
    );
    let root = id(&snapshot, "/components/schemas/Use");
    let validator = OwnedCompiler::new(Config::default())
        .compile(snapshot, std::slice::from_ref(&root))
        .unwrap();
    checked_sources(&validator);
    verdict(&validator, &root, json!(7), true);
    verdict(&validator, &root, json!("text"), false);
    let (validator, root) = compile(
        "3.2.0",
        json!({"$self":"also an annotation here","type":"integer"}),
        Config::default(),
    );
    verdict(&validator, &root, json!(7), true);
}
