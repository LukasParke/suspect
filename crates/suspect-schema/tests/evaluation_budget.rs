//! One per-call work budget must survive trial branches and lazy references.

use serde_json::{Value, json};
use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config, SchemaErrorKind};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn document(value: &Value) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("https://example.test/evaluation.json").unwrap(),
        Source::from_vec(serde_json::to_vec(value).unwrap()),
        Format::Json,
    )
}

fn expect_exhaustion(schema: Value, instance: Value, steps: usize, errors: usize) {
    let schema_doc = document(&schema);
    let instance_doc = document(&instance);
    let compiled = Compiler::new(Config {
        max_evaluation_steps: steps,
        max_errors: errors,
        ..Config::default()
    })
    .compile(schema_doc.root())
    .unwrap();
    // Repeated calls exercise both cold and warm lazy-reference caches. Each
    // public entrypoint starts a fresh budget; branch trials never do.
    for _ in 0..2 {
        let findings = compiled.validate(instance_doc.root());
        let first = findings.first().expect("work limit must reject evaluation");
        assert_eq!(first.kind, SchemaErrorKind::Evaluation, "{findings:?}");
        assert!(first.message.contains("evaluation steps"), "{first:?}");
        if errors != 0 {
            assert!(findings.len() <= errors);
        }
        let first = compiled.validate_first(instance_doc.root()).unwrap();
        assert_eq!(first.kind, SchemaErrorKind::Evaluation, "{first:?}");
        assert!(first.message.contains("evaluation steps"), "{first:?}");
    }
}

fn duplicate_ref_graph(levels: usize) -> Value {
    let mut defs = serde_json::Map::new();
    defs.insert("n0".into(), json!({"type":"integer"}));
    for level in 1..=levels {
        let target = format!("#/$defs/n{}", level - 1);
        defs.insert(
            format!("n{level}"),
            json!({"allOf":[{"$ref":target},{"$ref":target}]}),
        );
    }
    json!({"$defs":defs,"$ref":format!("#/$defs/n{levels}")})
}

#[test]
fn acyclic_reference_graph_cannot_multiply_work_past_one_call_budget() {
    expect_exhaustion(duplicate_ref_graph(12), json!(7), 1000, 1);
    let schema_doc = document(&duplicate_ref_graph(5));
    let instances =
        [(json!(7), true), (json!("wrong"), false)].map(|(value, valid)| (document(&value), valid));
    let compiled = Compiler::new(Config::default())
        .compile(schema_doc.root())
        .unwrap();
    for (instance, valid) in &instances {
        for _ in 0..2 {
            let findings = compiled.validate(instance.root());
            assert_eq!(findings.is_empty(), *valid);
            assert!(findings.iter().all(|f| f.kind == SchemaErrorKind::Invalid));
            assert_eq!(compiled.validate_first(instance.root()).is_none(), *valid);
        }
    }
}

#[test]
fn exhaustion_is_never_inverted_or_hidden_by_a_successful_alternative() {
    let expensive = json!({"allOf":vec![json!(true); 30]});
    for schema in [
        json!({"not":expensive}),
        json!({"anyOf":[true,expensive]}),
        json!({"oneOf":[true,expensive]}),
        json!({"if":expensive,"then":false,"else":true}),
        json!({"contains":expensive,"minContains":0}),
    ] {
        expect_exhaustion(schema, json!([1]), 16, 1);
    }
}

#[test]
fn pattern_transition_work_is_shared_and_non_invertible_in_logical_branches() {
    let expensive = json!({"pattern":"^(a?)*$"});
    for schema in [
        json!({"not":expensive}),
        json!({"anyOf":[true,expensive]}),
        json!({"oneOf":[true,expensive]}),
    ] {
        expect_exhaustion(schema, json!("a".repeat(100)), 32, 1);
    }
}

#[test]
fn capped_invalid_trials_share_the_budget_without_hiding_exhaustion() {
    let invalid = json!({"required":["missing1","missing2","missing3"]});
    for keyword in ["anyOf", "oneOf"] {
        let mut branches = vec![invalid.clone(); 40];
        branches.push(json!(true));
        expect_exhaustion(json!({keyword:branches}), json!({}), 80, 1);
    }
}

#[test]
fn collection_scans_are_bounded_even_when_no_child_schema_is_evaluated() {
    let object = Value::Object((0..100).map(|i| (format!("k{i}"), json!(i))).collect());
    let names: Vec<_> = (0..100).map(|i| format!("k{i}")).collect();
    for schema in [
        json!({"properties":{"absent":false}}),
        json!({"patternProperties":{"^absent$":false}}),
        json!({"additionalProperties":true}),
        json!({"unevaluatedProperties":true}),
        json!({"minProperties":0}),
        json!({"required":names}),
        json!({"dependentRequired":{"k0":names}}),
        json!({"dependentSchemas":{"absent":false}}),
    ] {
        expect_exhaustion(schema, object.clone(), 32, 0);
    }
    let array = json!((0..100).collect::<Vec<_>>());
    for schema in [
        json!({"items":true}),
        json!({"prefixItems":vec![true;100]}),
        json!({"contains":false,"minContains":0}),
        json!({"unevaluatedItems":true}),
        json!({"minItems":0}),
        json!({"uniqueItems":true}),
    ] {
        expect_exhaustion(schema, array.clone(), 32, 0);
    }
    expect_exhaustion(
        json!({"enum":(0..100).collect::<Vec<_>>()}),
        json!(101),
        32,
        0,
    );
}

#[test]
fn property_name_evaluation_uses_the_same_non_invertible_budget() {
    let expensive = json!({"allOf":vec![true;30]});
    for sub in [
        json!({"not":expensive}),
        json!({"anyOf":[expensive,true]}),
        json!({"oneOf":[true,expensive]}),
        json!({"if":expensive,"then":false,"else":true}),
        json!({"enum":(0..100).map(|i| format!("k{i}")).collect::<Vec<_>>()}),
    ] {
        expect_exhaustion(json!({"propertyNames":sub}), json!({"name":1}), 16, 1);
    }
    let mut graph = duplicate_ref_graph(12);
    graph["$defs"]["n0"] = json!(true);
    expect_exhaustion(
        json!({"$defs":graph["$defs"],"propertyNames":{"$ref":"#/$defs/n12"}}),
        json!({"name":1}),
        1000,
        1,
    );
}

#[test]
fn successful_annotation_merges_spend_the_shared_allowance() {
    for (inner, instance) in [
        (json!({"items":true}), json!(vec![0; 100])),
        (
            json!({"additionalProperties":true}),
            Value::Object((0..100).map(|i| (format!("k{i}"), json!(i))).collect()),
        ),
    ] {
        // The child fits with ample room for a wrapper's own checks. Moving
        // its 100 evaluated-member annotations into the parent consumes the
        // remaining allowance, even when no assertion rejects the instance.
        let child = document(&inner);
        let value = document(&instance);
        let schema = Compiler::new(Config {
            max_evaluation_steps: 350,
            ..Config::default()
        })
        .compile(child.root())
        .unwrap();
        assert!(schema.validate(value.root()).is_empty());
        assert!(schema.validate_first(value.root()).is_none());
        for wrapper in [
            json!({"allOf":[inner.clone()]}),
            json!({"anyOf":[inner.clone()]}),
            json!({"oneOf":[inner.clone()]}),
            json!({"if":inner.clone()}),
            json!({"$defs":{"child":inner.clone()},"$ref":"#/$defs/child"}),
        ] {
            expect_exhaustion(wrapper, instance.clone(), 350, 1);
        }
    }
}

#[test]
fn successive_property_names_cannot_restart_the_inner_schema_budget() {
    let object = Value::Object((0..100).map(|i| (format!("k{i}"), json!(i))).collect());
    // Each name's assertion is cheap, and the member scan alone fits. Schema
    // and keyword visits must accumulate across every name in the same call.
    expect_exhaustion(json!({"propertyNames":{"pattern":"^k"}}), object, 250, 1);
}

#[test]
fn zero_budget_permits_no_schema_visit() {
    expect_exhaustion(json!(true), json!(null), 0, 0);
    expect_exhaustion(json!({}), json!(null), 0, 1);
}

#[test]
fn every_public_validation_call_gets_a_fresh_allowance() {
    let schema_doc = document(&json!({"type":"integer"}));
    let valid = document(&json!(7));
    let invalid = document(&json!("wrong"));
    let schema = Compiler::new(Config {
        max_evaluation_steps: 8,
        ..Config::default()
    })
    .compile(schema_doc.root())
    .unwrap();
    for _ in 0..32 {
        assert!(schema.validate(valid.root()).is_empty());
        assert!(schema.validate_first(valid.root()).is_none());
        let findings = schema.validate(invalid.root());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SchemaErrorKind::Invalid);
        assert_eq!(
            schema.validate_first(invalid.root()).unwrap().kind,
            SchemaErrorKind::Invalid
        );
    }
}

#[test]
fn exhaustion_preserves_nested_instance_and_schema_locations() {
    let schema_doc = document(&json!({"properties":{"value":{"allOf":vec![true;30]}}}));
    let instance = document(&json!({"value":7}));
    let schema = Compiler::new(Config {
        max_evaluation_steps: 16,
        ..Config::default()
    })
    .compile(schema_doc.root())
    .unwrap();
    for finding in [
        schema.validate(instance.root()).remove(0),
        schema.validate_first(instance.root()).unwrap(),
    ] {
        assert_eq!(finding.kind, SchemaErrorKind::Evaluation);
        assert_eq!(finding.instance_path.to_path(), "/value");
        assert!(
            finding
                .schema_path
                .to_path()
                .starts_with("/properties/value/allOf/")
        );
    }
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT pointing at the tracked primary corpus"]
fn tracked_openrouter_selected_reference_closures_keep_valid_and_invalid_outcomes() {
    let root = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    let path = root.join("projects/docs/openapi/openapi.yaml");
    let bytes = std::fs::read(&path).expect("read tracked OpenRouter YAML");
    for (name, cases) in [
        (
            "ORAnthropicNullableCaller",
            vec![
                (json!(null), true),
                (json!({"type":"direct"}), true),
                (
                    json!({"type":"code_execution_20260120","tool_id":"tool"}),
                    true,
                ),
                (json!({"type":"code_execution_20260120"}), false),
                (json!({"type":"unknown"}), false),
            ],
        ),
        (
            "AnthropicImageBlockParam",
            vec![
                (
                    json!({"type":"image","source":{"type":"url","url":"https://example.test/image"}}),
                    true,
                ),
                (
                    json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"abc"}}),
                    true,
                ),
                (
                    json!({"type":"image","source":{"type":"base64","url":"https://example.test/image"}}),
                    false,
                ),
                (json!({"type":"image","source":{"type":"url"}}), false),
            ],
        ),
    ] {
        // Add only a root selector; the complete tracked source follows
        // unchanged so component $refs retain their original document paths.
        let mut selected = format!("$ref: '#/components/schemas/{name}'\n").into_bytes();
        selected.extend_from_slice(&bytes);
        let source = LowDoc::with_format(
            Uri::from_path(&path).unwrap(),
            Source::from_vec(selected),
            Format::Yaml,
        );
        assert!(source.syntax_errors().is_empty());
        let instances: Vec<_> = cases
            .into_iter()
            .map(|(value, valid)| (document(&value), value, valid))
            .collect();
        let schema = Compiler::new(Config::default())
            .compile(source.root())
            .unwrap();
        for (instance, value, valid) in &instances {
            let errors = schema.validate(instance.root());
            assert_eq!(errors.is_empty(), *valid, "{name}: {value}: {errors:?}");
            assert!(errors.iter().all(|e| e.kind == SchemaErrorKind::Invalid));
            let first = schema.validate_first(instance.root());
            assert_eq!(first.is_none(), *valid, "{name}: {value}: {first:?}");
            assert!(first.is_none_or(|e| e.kind == SchemaErrorKind::Invalid));
        }
    }
}
