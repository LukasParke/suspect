//! Public OpenAPI -> validated example plan regressions, with independent data.
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use suspect_codegen::examples::{
    ExampleConfig, ExampleOrigin, ExamplePlan, ExampleRole, plan_examples,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn api(schema: Value, annotations: Value) -> Value {
    let mut media = annotations;
    media["schema"] = schema;
    json!({"openapi":"3.1.0","info":{"title":"Examples","version":"1"},"servers":[{"url":"https://example.test"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/items":{"post":{"operationId":"create","requestBody":{"required":true,"content":{"application/json":media}},"responses":{"201":{"description":"Created","content":{"application/json":{"schema":{"type":"boolean"},"example":true}}}}}}}})
}
fn plan(value: Value, extra: Option<Value>, config: ExampleConfig) -> ExamplePlan {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("api.json");
    std::fs::write(&source, value.to_string()).unwrap();
    if let Some(extra) = extra {
        std::fs::write(directory.path().join("examples.json"), extra.to_string()).unwrap();
    }
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    drop(workspace);
    directory.close().unwrap();
    plan_examples(contract, &selected, config)
}
fn bodies(plan: &ExamplePlan) -> Vec<&suspect_codegen::examples::ExampleEntry> {
    plan.operations()
        .iter()
        .flat_map(|op| &op.entries)
        .filter(|entry| entry.role == ExampleRole::RequestBody)
        .collect()
}

#[test]
fn declared_exact_values_and_null_retain_original_locations() {
    let value: Value =
        serde_json::from_str(r#"{"id":9007199254740993,"small":1e-4000,"nullable":null}"#).unwrap();
    let schema = json!({"type":"object","required":["id","small","nullable"],"properties":{"id":{"type":"integer"},"small":{"type":"number"},"nullable":{"type":["string","null"]}}});
    let plan = plan(
        api(schema, json!({"example":value})),
        None,
        Default::default(),
    );
    let entry = bodies(&plan)[0];
    assert_eq!(entry.value, value);
    assert_eq!(entry.origin, ExampleOrigin::Declared);
    let source = entry.declared_source.as_ref().unwrap();
    assert_eq!(
        source.pointer(),
        "/paths/~1items/post/requestBody/content/application~1json/example"
    );
    assert_eq!(plan.contract().source(source), Some(&value));
    assert!(!plan.contract().source_span(source).unwrap().is_empty());
    assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
}

#[test]
fn referenced_parameter_example_retains_definition_and_use_site() {
    let mut document = api(json!({"type":"string"}), json!({"example":"body"}));
    let item = document["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/items")
        .unwrap();
    document["paths"]["/items/{id}"] = item;
    document["components"]["parameters"] = json!({"Id":{"name":"id","in":"path","required":true,"schema":{"type":"string"},"example":"component-value"}});
    document["paths"]["/items/{id}"]["post"]["parameters"] =
        json!([{"$ref":"#/components/parameters/Id"}]);
    let result = plan(document, None, Default::default());
    let parameter = result.operations()[0]
        .entries
        .iter()
        .find(|entry| matches!(&entry.role,ExampleRole::Parameter{wire_name,..} if wire_name=="id"))
        .unwrap();
    assert_eq!(parameter.origin, ExampleOrigin::Declared);
    assert_eq!(parameter.value, "component-value");
    assert_eq!(
        parameter.container.pointer(),
        "/paths/~1items~1{id}/post/parameters/0"
    );
    assert_eq!(
        parameter.declared_source.as_ref().unwrap().pointer(),
        "/components/parameters/Id/example"
    );
    assert_eq!(
        result
            .contract()
            .source(parameter.declared_source.as_ref().unwrap()),
        Some(&parameter.value)
    );
}

#[test]
fn invalid_declared_value_is_not_repaired_or_relabelled() {
    let schema = json!({"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":2,"maxLength":2}},"additionalProperties":false});
    let plan = plan(api(schema, json!({"example":{}})), None, Default::default());
    let entry = bodies(&plan)[0];
    assert_eq!(entry.origin, ExampleOrigin::Synthesized);
    assert!(entry.declared_source.is_none());
    assert_eq!(entry.value, json!({"name":"xx"}));
    let finding = plan
        .diagnostics()
        .iter()
        .find(|finding| finding.code == "examples-declared-invalid")
        .unwrap();
    assert!(finding.message.contains("name"));
    assert!(!finding.at.is_empty());
    assert_eq!(plan.contract().source(&finding.source), Some(&json!({})));
}

#[test]
fn referenced_examples_use_terminal_value_provenance_and_do_not_resolve_instance_refs() {
    let schema =
        json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}});
    let extra = json!({"First":{"$ref":"#/Second"},"Second":{"summary":"Source prose","value":{"value":7,"$ref":"literal instance data"}}});
    let plan = plan(
        api(
            schema,
            json!({"examples":{"one":{"$ref":"examples.json#/First","summary":"Use-site prose"}}}),
        ),
        Some(extra),
        Default::default(),
    );
    let entry = bodies(&plan)[0];
    assert_eq!(entry.origin, ExampleOrigin::Declared);
    assert_eq!(entry.name.as_deref(), Some("one"));
    assert_eq!(entry.summary.as_deref(), Some("Use-site prose"));
    let source = entry.declared_source.as_ref().unwrap();
    assert!(source.document().as_str().ends_with("/examples.json"));
    assert_eq!(source.pointer(), "/Second/value");
    assert_eq!(entry.value["$ref"], "literal instance data");
}

#[test]
fn container_examples_override_schema_annotations_and_duplicate_origins_remain_distinct() {
    let schema = json!({"type":"integer","example":1,"examples":[2,3]});
    let plan = plan(
        api(
            schema,
            json!({"examples":{"a":{"value":4},"b":{"value":4}}}),
        ),
        None,
        Default::default(),
    );
    let examples = bodies(&plan);
    assert_eq!(examples.len(), 2);
    assert!(examples.iter().all(|entry| entry.value == 4));
    assert_ne!(examples[0].declared_source, examples[1].declared_source);
    let plan = self::plan(
        api(
            json!({"type":["string","null"],"examples":[null,"text"]}),
            json!({}),
        ),
        None,
        Default::default(),
    );
    assert_eq!(
        bodies(&plan)
            .iter()
            .map(|entry| entry.value.clone())
            .collect::<Vec<_>>(),
        [Value::Null, json!("text")]
    );
}

#[test]
fn malformed_external_and_over_budget_examples_are_explicit() {
    for annotations in [
        json!({"examples":42}),
        json!({"example":1,"examples":{}}),
        json!({"examples":{"remote":{"externalValue":"https://unrequested.invalid/value"}}}),
    ] {
        let plan = plan(
            api(json!({"type":"integer"}), annotations),
            None,
            Default::default(),
        );
        assert!(!plan.diagnostics().is_empty());
        assert!(
            bodies(&plan)
                .iter()
                .all(|entry| entry.origin == ExampleOrigin::Synthesized)
        );
    }
    let plan = plan(
        api(
            json!({"type":"string"}),
            json!({"example":"x".repeat(2048)}),
        ),
        None,
        ExampleConfig {
            max_work: 64,
            ..Default::default()
        },
    );
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-synthesis-limit")
    );
    assert!(
        !bodies(&plan)
            .iter()
            .any(|entry| entry.origin == ExampleOrigin::Declared)
    );
}

#[test]
fn recursion_and_incomplete_validation_never_count_as_invalid_examples() {
    let mut value = api(json!({"$ref":"#/components/schemas/Node"}), json!({}));
    value["components"]["schemas"] = json!({"Node":{"type":"object","required":["child"],"properties":{"child":{"$ref":"#/components/schemas/Node"}}}});
    let plan = plan(value, None, Default::default());
    assert!(bodies(&plan).is_empty());
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-synthesis-unsupported")
    );
    let plan = self::plan(
        api(
            json!({"type":"object","required":["x"],"properties":{"x":{"type":"integer"}}}),
            json!({"example":{"x":1}}),
        ),
        None,
        ExampleConfig {
            max_validation_steps: 1,
            ..Default::default()
        },
    );
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-evaluation-incomplete")
    );
    assert!(
        !plan
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid")
    );
}

#[test]
fn directional_example_context_is_explicitly_unavailable() {
    let plan = plan(
        api(
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string","readOnly":true}}}),
            json!({"example":{}}),
        ),
        None,
        Default::default(),
    );
    assert!(bodies(&plan).is_empty());
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-directional-unsupported")
    );
    assert!(
        !plan
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid")
    );
}

#[test]
#[ignore = "requires the tracked OpenRouter public snapshot"]
fn tracked_examples_keep_the_real_patch_key_source_defect_visible() {
    let source =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"))
            .join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let names = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| names.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    let plan = plan_examples(contract, &selected, Default::default());
    assert_eq!(plan.operations().len(), 5);
    let patch = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "updateKeys")
        .unwrap();
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid"
                && finding.source.pointer().starts_with(patch.source.pointer())
                && finding.message.contains("external_user")),
        "{:?}",
        plan.diagnostics()
    );
    for finding in plan.diagnostics() {
        assert!(!finding.at.is_empty(), "{finding:?}");
    }
}
