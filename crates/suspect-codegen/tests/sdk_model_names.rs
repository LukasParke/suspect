//! Public model-plan names derive from operation roles while retaining source IDs.

use std::{process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::python_models;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn specification() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Naming","version":"1"},
    "servers":[{"url":"https://api.example.test/v1"}], "security":[{"Bearer":[]}],
    "components":{"securitySchemes":{"Bearer":{"type":"http","scheme":"bearer"}}},
    "paths":{"/widgets":{"post":{"operationId":"createWidget",
        "requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","required":["name"],"additionalProperties":false,
            "properties":{"name":{"type":"string"},"metadata":{"type":"object",
                "additionalProperties":false,"properties":{"label":{"type":"string"}}}}
        }}}},
        "responses":{"201":{"description":"Created","content":{"application/json":{"schema":{
            "type":"object","required":["id"],"properties":{"id":{"type":"string"}}
        }}}}}
    }}}})
}

fn load(value: &Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

#[test]
fn native_python_constructor_has_an_operation_role_name_and_original_source_binding() {
    let contract = load(&specification());
    let operation = contract.operations().next().unwrap();
    let schema = operation.request_body().unwrap().content()[0]
        .schema()
        .unwrap()
        .id()
        .clone();
    let plan = python_models::plan_models(&contract, std::slice::from_ref(&schema));
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    let name = |source: &_| {
        plan.symbols()
            .iter()
            .find(|symbol| symbol.source() == source)
            .unwrap()
            .name()
    };
    assert_eq!(name(&schema), "CreateWidgetRequest");
    assert_eq!(
        name(&schema.child("properties").child("metadata")),
        "CreateWidgetRequestMetadata"
    );
    assert_eq!(
        schema.pointer(),
        "/paths/~1widgets/post/requestBody/content/application~1json/schema"
    );
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render().unwrap(), directory.path()).unwrap();
    let result = Command::new(std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()))
        .current_dir(directory.path()).args(["-c", "from python.models import CreateWidgetRequest, CreateWidgetRequestMetadata\nvalue = CreateWidgetRequest(name='demo', metadata=CreateWidgetRequestMetadata(label='owned'))\nassert value.name == 'demo' and value.metadata.label == 'owned'"])
        .output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn descriptions_do_not_choose_public_names() {
    let original = specification();
    let mut changed = original.clone();
    changed["paths"]["/widgets"]["post"]["description"] =
        json!("A completely different paragraph.");
    for value in [&original, &changed] {
        let contract = load(value);
        let root = contract.operations().next().unwrap().responses()[0].content()[0]
            .schema()
            .unwrap()
            .id()
            .clone();
        let plan = python_models::plan_models(&contract, std::slice::from_ref(&root));
        let model = plan
            .symbols()
            .iter()
            .find(|symbol| symbol.source() == &root)
            .unwrap();
        assert_eq!(model.name(), "CreateWidgetResponse201");
    }
}

#[test]
fn all_five_native_plans_keep_readable_inline_names_and_component_identity() {
    let mut value = specification();
    value["components"]["schemas"] = json!({"SharedLabel":{"type":"object","required":["label"],
        "properties":{"label":{"type":"string"}},"additionalProperties":false}});
    value["paths"]["/widgets"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]
        ["shared"] = json!({"$ref":"#/components/schemas/SharedLabel"});
    let contract = load(&value);
    let operation = contract.operations().next().unwrap();
    let request = operation.request_body().unwrap().content()[0]
        .schema()
        .unwrap()
        .id()
        .clone();
    let response = operation.responses()[0].content()[0]
        .schema()
        .unwrap()
        .id()
        .clone();
    let shared = suspect_ir::contract::SourceId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("SharedLabel");
    let roots = [request.clone(), response.clone()];
    let expected = [
        (&request, "CreateWidgetRequest"),
        (&response, "CreateWidgetResponse201"),
        (&shared, "SharedLabel"),
    ];
    let python = suspect_codegen::python_models::plan_models(&contract, &roots);
    let go = suspect_codegen::go_models::plan_models(&contract, &roots);
    let rust = suspect_codegen::rust_models::plan_models(&contract, &roots);
    let ts = suspect_codegen::typescript::plan_models(
        &contract,
        &roots,
        &[suspect_codegen::typescript::ModelView::Neutral],
    );
    let swift = suspect_codegen::swift_sdk::plan_sdk(
        contract.clone(),
        &[operation.source().clone()],
        Default::default(),
    )
    .unwrap();
    for (source, name) in expected {
        assert_eq!(
            python
                .symbols()
                .iter()
                .find(|symbol| symbol.source() == source)
                .unwrap()
                .name(),
            name
        );
        assert_eq!(
            go.symbols()
                .iter()
                .find(|symbol| symbol.source() == source)
                .unwrap()
                .name(),
            name
        );
        assert_eq!(
            rust.symbols()
                .iter()
                .find(|symbol| symbol.source() == source)
                .unwrap()
                .name(),
            name
        );
        assert_eq!(
            ts.symbols()
                .iter()
                .find(|symbol| symbol.source() == source)
                .unwrap()
                .name(),
            name
        );
        assert_eq!(swift.symbols()[source], name);
    }
}

#[test]
fn nested_property_keyword_names_remain_distinct_native_models() {
    let mut value = specification();
    value["paths"]["/widgets"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"] =
        json!({"name":{"type":"string"},"properties":{"type":"object"},"items":{"type":"object"}});
    let contract = load(&value);
    let root = contract
        .operations()
        .next()
        .unwrap()
        .request_body()
        .unwrap()
        .content()[0]
        .schema()
        .unwrap()
        .id()
        .clone();
    let plan = python_models::plan_models(&contract, std::slice::from_ref(&root));
    let names = plan
        .symbols()
        .iter()
        .map(|symbol| symbol.name())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("CreateWidgetRequestProperties"));
    assert!(names.contains("CreateWidgetRequestItems"));
}
