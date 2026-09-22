use std::sync::Arc;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;

#[test]
fn validation_uses_schema_anchors_instead_of_instance_data_anchors() {
    validate_without_reference_errors(
        "anchors",
        r##"{
        "openapi":"3.1.0","info":{"title":"Scoped refs","version":"1"},"paths":{},
        "components":{"schemas":{
            "Actual":{"$anchor":"Target","type":"string"},
            "Use":{"$ref":"#Target"},
            "ZData":{"type":"object","examples":[{"$anchor":"Target","$ref":"missing.json#/Gone"}]}
        }}
    }"##,
    );
}

#[test]
fn validation_ignores_non_schema_ancestor_ids_when_resolving_refs() {
    validate_without_reference_errors(
        "ids",
        r##"{
        "openapi":"3.1.0","info":{"title":"Scoped refs","version":"1"},"paths":{},
        "$id":"missing.json",
        "components":{"schemas":{"Actual":{"type":"string"},"Use":{"$ref":"#/components/schemas/Actual"}}}
    }"##,
    );
}

fn validate_without_reference_errors(name: &str, source: &str) {
    let dir = std::env::temp_dir().join(format!(
        "suspect-validate-scoped-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("api.json"), source).unwrap();
    let session = Session::new(Arc::new(
        WorkspaceBuilder::new().root(&dir).build().unwrap(),
    ));
    let findings = suspect_validate::validate_entry(&session, "api.json").unwrap();
    assert!(
        !findings
            .iter()
            .any(|d| matches!(d.code, "invalid-ref" | "unresolved-ref")),
        "{findings:?}"
    );
    assert_eq!(session.workspace().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
