//! Lossless, source-addressed contract graph behavior through the public API.

use std::sync::Arc;

use serde_json::json;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn schema_id(uri: &Uri, pointer: &str) -> SchemaId {
    SchemaId::new(uri.clone(), Pointer::parse(pointer).unwrap())
}

#[test]
fn schemas_borrow_exact_raw_data_and_index_inline_positions_without_expansion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    let value = json!({
        "openapi": "3.1.0", "info": {"title": "Contracts", "version": "1"},
        "paths": {"/node": {"get": {"responses": {"200": {
            "description": "OK", "content": {"application/json": {"schema": {
                "anyOf": [{"$ref": "#/components/schemas/Node"}, {"type": "null"}]
            }}}
        }}}}},
        "components": {"schemas": {"Node": {
            "type": "object", "required": ["kind"],
            "properties": {
                "kind": {"const": "node"},
                "present": {"type": ["string", "null"], "default": null},
                "absent": {"type": "string"},
                "$ref": {"type": "string"},
                "next": {"$ref": "#/components/schemas/Node"}
            },
            "discriminator": {"propertyName": "kind", "mapping": {"node": "#/components/schemas/Node"}},
            "examples": [{"$ref": "does-not-exist.yaml", "present": null}]
        }}}
    });
    let source = serde_json::to_string_pretty(&value).unwrap();
    std::fs::write(&path, &source).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    drop(workspace);

    assert_eq!(contract.document(&uri).unwrap(), &value);
    let id = schema_id(&uri, "/components/schemas/Node");
    let node = contract.schema(&id).unwrap();
    assert_eq!(node.raw(), &value["components"]["schemas"]["Node"]);
    assert!(std::ptr::eq(
        node.raw(),
        &contract.document(&uri).unwrap()["components"]["schemas"]["Node"]
    ));
    assert!(
        node.raw()["properties"]["present"]
            .get("default")
            .unwrap()
            .is_null()
    );
    assert!(node.raw()["properties"]["absent"].get("default").is_none());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&source[node.span()]).unwrap(),
        *node.raw()
    );
    let inline = schema_id(
        &uri,
        "/paths/~1node/get/responses/200/content/application~1json/schema",
    );
    assert!(contract.schema_roots().contains(&inline));
    assert!(
        contract
            .schema(&schema_id(&uri, "/components/schemas/Node/properties/$ref"))
            .is_some()
    );
    assert!(
        contract
            .schema(&schema_id(&uri, "/components/schemas/Node/examples/0"))
            .is_none()
    );
    let next = contract
        .schema(&schema_id(&uri, "/components/schemas/Node/properties/next"))
        .unwrap();
    assert_eq!(next.references()[0].target.as_ref(), Some(&id));
    assert_eq!(contract.reachable_from(&[id]).len(), 6);
    assert!(
        contract.diagnostics().is_empty(),
        "{:?}",
        contract.diagnostics()
    );
}

#[test]
fn only_semantically_reachable_documents_and_schema_edges_enter_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("api.yaml");
    let target = dir.path().join("node.yaml");
    std::fs::write(&main, "openapi: 3.1.0\ninfo: {title: Graph, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Node: {$ref: 'node.yaml'}\n  examples:\n    Example:\n      value: {$ref: 'never-load.yaml'}\n").unwrap();
    std::fs::write(
        &target,
        "type: object\nproperties:\n  next: {$ref: 'node.yaml'}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("unrelated.yaml"),
        "$ref: 'also-never-load.yaml'\n",
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    workspace.open("unrelated.yaml").unwrap();
    let uri = Uri::from_path(&main).unwrap();
    let target_uri = Uri::from_path(&target).unwrap();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    assert_eq!(workspace.len(), 3);
    assert_eq!(contract.documents().count(), 2);
    assert!(
        contract
            .document(&Uri::from_path(&dir.path().join("unrelated.yaml")).unwrap())
            .is_none()
    );
    let root = schema_id(&uri, "/components/schemas/Node");
    let external = schema_id(&target_uri, "");
    assert_eq!(
        contract.schema(&root).unwrap().references()[0].target,
        Some(external.clone())
    );
    assert_eq!(contract.reachable_from(&[root]).len(), 3);
    assert!(
        contract.schema(&external).unwrap().raw()["properties"]["next"]
            .get("$ref")
            .is_some()
    );
    assert!(contract.diagnostics().is_empty());
}

#[test]
fn schema_identity_survives_reformatting_at_the_same_retrieval_uri() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    let uri = Uri::from_path(&path).unwrap();
    let inputs = [
        "openapi: 3.1.0\ninfo: {title: Stable, version: '1'}\npaths: {}\ncomponents: {schemas: {Value: {type: string}}}\n",
        "openapi: 3.1.0\ninfo:\n  title: Stable\n  version: '1'\npaths: {}\ncomponents:\n  schemas:\n    Value:\n      type: string\n",
        r#"{"openapi":"3.1.0","info":{"title":"Stable","version":"1"},"paths":{},"components":{"schemas":{"Value":{"type":"string"}}}}"#,
    ];
    let mut identities = Vec::new();
    let mut spans = Vec::new();
    for input in inputs {
        std::fs::write(&path, input).unwrap();
        let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
        let contract = Contract::from_workspace(&workspace, &uri).unwrap();
        let schema = contract.schemas().next().unwrap();
        identities.push(schema.id().clone());
        spans.push(schema.span());
    }
    assert!(identities.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(spans.windows(2).all(|pair| pair[0] != pair[1]));
}

#[test]
fn unsupported_schema_semantics_produce_diagnostics_and_keep_raw_keywords() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "openapi": "3.1.0", "info": {"title": "Dialect", "version": "1"}, "paths": {},
            "components": {"schemas": {"Custom": {
                "$schema": "https://example.test/custom-dialect", "$dynamicRef": "#node",
                "type": "object", "customRule": {"required": true}
            }}}
        }))
        .unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let uri = Uri::from_path(&path).unwrap();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    let schema = contract
        .schema(&schema_id(&uri, "/components/schemas/Custom"))
        .unwrap();
    assert_eq!(schema.raw()["customRule"], json!({"required": true}));
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "unsupported-schema-dialect")
    );
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "unsupported-dynamic-reference")
    );
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "unknown-schema-keyword")
    );
    assert!(
        schema.references().is_empty(),
        "dynamic resolution must not be guessed as a static ref"
    );
}

#[test]
fn dialect_is_lexical_and_openapi_32_keeps_the_openapi_31_base_dialect() {
    use suspect_ir::contract::SchemaDialect;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    let uri = Uri::from_path(&path).unwrap();
    std::fs::write(&path, serde_json::to_vec(&json!({
        "openapi":"3.2.0", "info":{"title":"Dialect", "version":"1"}, "paths":{},
        "components":{"schemas":{
            "Plain":{"type":"string"},
            "Container":{"$schema":"https://json-schema.org/draft/2020-12/schema", "$defs":{"Nested":{"type":"string"}}},
            "Target":{"$ref":"#/components/schemas/Container/$defs/Nested"}
        }}
    })).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    assert_eq!(
        contract
            .schema(&schema_id(&uri, "/components/schemas/Plain"))
            .unwrap()
            .dialect(),
        &SchemaDialect::Uri("https://spec.openapis.org/oas/3.1/dialect/base".to_owned())
    );
    assert_eq!(
        contract
            .schema(&schema_id(
                &uri,
                "/components/schemas/Container/$defs/Nested"
            ))
            .unwrap()
            .dialect(),
        &SchemaDialect::Uri("https://json-schema.org/draft/2020-12/schema".to_owned())
    );
}

#[test]
fn ambiguous_source_values_fail_instead_of_silently_changing_identity_or_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    let uri = Uri::from_path(&path).unwrap();
    for schemas in [
        "    Value: {type: string, type: integer}\n",
        "    Value: {properties: {\"caf\\u00e9\": {type: string}, café: {type: integer}}}\n",
        "    Value: &shared {type: string}\n    Alias: *shared\n",
        "    Value: &cycle {properties: {next: *cycle}}\n",
    ] {
        std::fs::write(&path, format!("openapi: 3.1.0\ninfo: {{title: Ambiguous, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n{schemas}")).unwrap();
        let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
        assert!(
            Contract::from_workspace(&workspace, &uri).is_err(),
            "{schemas}"
        );
    }
}

#[test]
fn absent_reference_values_produce_source_linked_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    std::fs::write(&path, "openapi: 3.1.0\ninfo: {title: Refs, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Value:\n      $ref:\n").unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let uri = Uri::from_path(&path).unwrap();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "invalid-reference"
                && d.source == schema_id(&uri, "/components/schemas/Value"))
    );
    assert!(contract.has_errors());
}

#[test]
fn escaped_schema_keyword_names_preserve_the_same_graph() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(
        &path,
        r##"{
      "openapi":"3.1.0","info":{"title":"Escapes","version":"1"},"paths":{},
      "components":{"schemas":{"caf\u00e9":{
        "type":"object","\u0070roperties":{"next":{"\u0024ref":"#/components/schemas/caf%C3%A9"}}
      }}}
    }"##,
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let uri = Uri::from_path(&path).unwrap();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    let root = schema_id(&uri, "/components/schemas/café");
    let child = schema_id(&uri, "/components/schemas/café/properties/next");
    assert!(contract.schema(&child).is_some());
    assert_eq!(
        contract.schema(&child).unwrap().references()[0]
            .target
            .as_ref(),
        Some(&root)
    );
    assert_eq!(contract.reachable_from(&[root]).len(), 2);
}

#[test]
fn absent_yaml_schema_slots_keep_identity_and_are_diagnosed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    std::fs::write(&path, "openapi: 3.1.0\ninfo: {title: Missing, version: '1'}\npaths:\n  /value:\n    get:\n      responses:\n        '200':\n          description: OK\n          content:\n            application/json:\n              schema:\ncomponents:\n  schemas:\n    Missing:\n    Array:\n      type: array\n      items:\n    Object:\n      properties:\n        missing:\n").unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    for pointer in [
        "/components/schemas/Missing",
        "/components/schemas/Array/items",
        "/components/schemas/Object/properties/missing",
        "/paths/~1value/get/responses/200/content/application~1json/schema",
    ] {
        let id = schema_id(&uri, pointer);
        let schema = contract
            .schema(&id)
            .unwrap_or_else(|| panic!("missing source slot {pointer}"));
        assert!(schema.raw().is_null());
        assert!(
            contract
                .diagnostics()
                .iter()
                .any(|d| d.source == id && d.code == "invalid-schema")
        );
        assert!(!schema.span().is_empty());
    }
}

#[test]
fn openapi_30_boolean_additional_properties_remains_valid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&json!({
        "openapi":"3.0.3","info":{"title":"Booleans","version":"1"},"paths":{},
        "components":{"schemas":{
            "Closed":{"type":"object","additionalProperties":false,"properties":{"additionalProperties":false}},
            "Invalid":{"type":"array","items":true,"$schema":"https://json-schema.org/draft/2020-12/schema"}
        }}
    })).unwrap()).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    let valid = schema_id(&uri, "/components/schemas/Closed/additionalProperties");
    let invalid = schema_id(&uri, "/components/schemas/Invalid/items");
    assert_eq!(contract.schema(&valid).unwrap().raw(), &json!(false));
    assert!(!contract.diagnostics().iter().any(|d| d.source == valid));
    let property = schema_id(
        &uri,
        "/components/schemas/Closed/properties/additionalProperties",
    );
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.source == property && d.code == "invalid-schema")
    );
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.source == invalid && d.code == "invalid-schema")
    );
    let invalid_root = schema_id(&uri, "/components/schemas/Invalid");
    assert_eq!(
        contract.schema(&invalid_root).unwrap().dialect(),
        &suspect_ir::contract::SchemaDialect::OpenApi30
    );
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.source == invalid_root && d.code == "unsupported-schema-keyword")
    );
}

#[test]
fn transport_metadata_named_schema_does_not_change_schema_dialect() {
    use suspect_ir::contract::SchemaDialect;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&json!({
        "openapi":"3.1.0","info":{"title":"Dialect scope","version":"1"},
        "paths":{"/value":{"get":{"$schema":"https://example.test/not-a-schema-resource","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}}}}}
    })).unwrap()).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    let schema = contract.schemas().next().unwrap();
    assert_eq!(
        schema.dialect(),
        &SchemaDialect::Uri("https://spec.openapis.org/oas/3.1/dialect/base".to_owned())
    );
}

#[test]
#[ignore = "set SUSPECT_CONTRACT_SPEC and SUSPECT_CONTRACT_ORACLE to tracked corpus and independent normalized JSON"]
fn real_openrouter_contract_preserves_documents_schemas_and_reference_graph() {
    let path =
        std::path::PathBuf::from(std::env::var_os("SUSPECT_CONTRACT_SPEC").expect("corpus path"));
    let oracle = std::env::var_os("SUSPECT_CONTRACT_ORACLE").expect("independent JSON oracle path");
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(oracle).unwrap()).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let start = std::time::Instant::now();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    assert_eq!(contract.document(&uri).unwrap(), &expected);
    let named = expected["components"]["schemas"].as_object().unwrap();
    for (name, raw) in named {
        let id = SchemaId::new(
            uri.clone(),
            Pointer::root()
                .push("components")
                .push("schemas")
                .push(name),
        );
        assert_eq!(contract.schema(&id).unwrap().raw(), raw, "{name}");
    }
    for schema in contract.schemas() {
        for reference in schema.references() {
            assert!(
                reference
                    .target
                    .as_ref()
                    .is_some_and(|id| contract.schema(id).is_some()),
                "{:?}",
                schema.id()
            );
        }
    }
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    eprintln!(
        "contract {:?}: {} documents, {} named schemas, {} total schema positions, {} roots, {} direct refs, {} diagnostics",
        start.elapsed(),
        contract.documents().count(),
        named.len(),
        contract.schemas().count(),
        contract.schema_roots().len(),
        contract
            .schemas()
            .map(|s| s.references().len())
            .sum::<usize>(),
        contract.diagnostics().len()
    );
}
