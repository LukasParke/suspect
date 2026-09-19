use std::sync::Arc;

use serde_json::json;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[test]
fn instance_data_anchors_cannot_shadow_a_real_schema_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&json!({
        "openapi":"3.1.0","info":{"title":"Anchors","version":"1"},
        "components":{"schemas":{
            "Actual":{"$anchor":"Target","type":"string"},
            "Use":{"$ref":"#Target"},
            "ZData":{"type":"object","default":{"$anchor":"Target","type":"integer"},"const":{"$anchor":"Target","type":"boolean"},"enum":[{"$anchor":"Target","type":"array"}],"examples":[{"$anchor":"Target","type":"number"}]}
        }}
    })).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    let use_schema = contract
        .schemas()
        .find(|s| s.id().pointer() == "/components/schemas/Use")
        .unwrap();
    let target = use_schema.references()[0].target.as_ref().unwrap();
    assert_eq!(target.pointer(), "/components/schemas/Actual");
    assert_eq!(contract.schemas().count(), 3);
    assert_eq!(contract.schema(target).unwrap().raw()["type"], "string");
}

#[test]
fn non_schema_ancestor_ids_cannot_rebase_openapi_references() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "openapi":"3.1.0","info":{"title":"Bases","version":"1"},
            "$id":"wrong.json",
            "components":{"schemas":{
                "Actual":{"type":"string"},
                "Use":{"$ref":"#/components/schemas/Actual"}
            }}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("wrong.json"),
        serde_json::to_vec(&json!({
            "components":{"schemas":{"Actual":{"type":"integer"}}}
        }))
        .unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(contract.documents().count(), 1);
    let use_schema = contract
        .schemas()
        .find(|s| s.id().pointer() == "/components/schemas/Use")
        .unwrap();
    let target = use_schema.references()[0].target.as_ref().unwrap();
    assert_eq!(target.document(), contract.entry());
    assert_eq!(contract.schema(target).unwrap().raw()["type"], "string");
}

#[test]
fn missing_or_ambiguous_schema_anchors_fail_without_generic_fallback() {
    for (name, schemas) in [
        (
            "missing",
            json!({"Use":{"$ref":"#Target"},"Data":{"default":{"$anchor":"Target","type":"integer"}}}),
        ),
        (
            "ambiguous",
            json!({"Use":{"$ref":"#Target"},"First":{"$anchor":"Target","type":"string"},"Second":{"$anchor":"Target","type":"integer"}}),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.json");
        std::fs::write(&path, serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":name,"version":"1"},"components":{"schemas":schemas}})).unwrap()).unwrap();
        let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
        let contract =
            Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
        let use_schema = contract
            .schemas()
            .find(|s| s.id().pointer() == "/components/schemas/Use")
            .unwrap();
        assert!(use_schema.references()[0].target.is_none());
        assert!(
            contract
                .diagnostics()
                .iter()
                .any(|d| d.code == "invalid-reference" && d.source == *use_schema.id())
        );
        assert!(
            !contract
                .schemas()
                .any(|s| s.id().pointer().contains("/default"))
        );
    }
}

#[test]
fn external_schema_anchors_use_schema_vocabulary_and_canonical_fragment_decoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":"External anchor","version":"1"},"components":{"schemas":{"Use":{"$ref":"model.json#T%61rget"}}}})).unwrap()).unwrap();
    std::fs::write(
        dir.path().join("model.json"),
        serde_json::to_vec(&json!({
            "$defs":{"Actual":{"$anchor":"Target","type":"string"}},
            "examples":[{"$anchor":"Target","type":"integer"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    let use_schema = contract
        .schemas()
        .find(|s| s.id().pointer() == "/components/schemas/Use")
        .unwrap();
    let target = use_schema.references()[0].target.as_ref().unwrap();
    assert_eq!(
        target.document(),
        &Uri::from_path(&dir.path().join("model.json")).unwrap()
    );
    assert_eq!(target.pointer(), "/$defs/Actual");
    assert_eq!(contract.schema(target).unwrap().raw()["type"], "string");
}

#[test]
fn openapi_30_does_not_treat_unknown_anchor_keywords_as_schema_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(&json!({"openapi":"3.0.3","info":{"title":"Old dialect","version":"1"},"paths":{},"components":{"schemas":{"Actual":{"$anchor":"Target","type":"string"},"Use":{"$ref":"#Target"}}}})).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
    let use_schema = contract
        .schemas()
        .find(|s| s.id().pointer() == "/components/schemas/Use")
        .unwrap();
    assert!(use_schema.references()[0].target.is_none());
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "unsupported-schema-keyword"
                && d.source.pointer() == "/components/schemas/Actual")
    );
}
