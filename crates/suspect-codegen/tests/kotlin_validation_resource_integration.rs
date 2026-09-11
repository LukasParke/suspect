#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, PlanStatus},
    kotlin_sdk::{self, SdkConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
fn load(value: Value) -> Arc<Contract> {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-resource-integration");
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep();
    let path = root.join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}
fn config() -> SdkConfig {
    SdkConfig {
        group_id: "test.suspect.kotlin".into(),
        artifact_id: "resource-sdk".into(),
        version: "0.4.0".into(),
        package_name: "example.resources.sdk".into(),
        credential_env: None,
    }
}
fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::KotlinHttp,
        package_name: "test.suspect.kotlin:resource-sdk".into(),
        package_version: "0.4.0".into(),
        import_name: Some("example.resources.sdk".into()),
    }
}

#[test]
fn verified_default_resource_admission_matches_explicit_v3() {
    let contract =
        load(serde_json::from_str(include_str!("kotlin_support/resource-sdk.json")).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let explicit = kotlin_sdk::plan_sdk_v3(contract.clone(), &selected, config()).unwrap();
    let default = kotlin_sdk::plan_sdk(contract.clone(), &selected, config()).unwrap();
    assert_eq!(
        default.program().version,
        suspect_schema::OwnedProgram::V3_VERSION
    );
    assert_eq!(explicit.program(), default.program());
    assert_eq!(explicit.render().unwrap(), default.render().unwrap());
    assert_eq!(
        backend::generate(contract, &selected, &target()).unwrap(),
        default.render().unwrap()
    );
}

#[test]
fn resource_snapshot_keeps_dynamic_carrier_types_and_physical_locations() {
    let contract =
        load(serde_json::from_str(include_str!("kotlin_support/resource-sdk.json")).unwrap());
    let physical = contract.entry().to_string();
    let snapshot = compatibility::snapshot(contract, &[], &[target()]).unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert!(
        native
            .runtime
            .profile
            .contains(suspect_schema::OwnedProgram::V3_PROFILE)
    );
    for name in ["TreeChildrenItem", "NumbersItem"] {
        let model = native
            .models
            .iter()
            .find(|m| m.name == format!("example.resources.sdk.{name}") && m.role == "model")
            .unwrap();
        assert_eq!(model.source.document, physical);
        assert_eq!(
            model.descriptor.as_ref().unwrap()["representation"],
            "schema-bound-json-carrier"
        );
        assert_eq!(
            model.descriptor.as_ref().unwrap()["constructor"]["parameters"][0]["type"]["name"],
            "example.resources.sdk.JsonValue"
        );
    }
}

#[test]
fn ordinary_default_closures_keep_their_established_versions() {
    for (schema, version) in [
        (
            json!({"type":"string","example":"base"}),
            suspect_schema::OwnedProgram::V1_VERSION,
        ),
        (
            json!({"if":{"type":"string"},"then":{"minLength":2},"else":false,"example":"scoped"}),
            suspect_schema::OwnedProgram::V2_VERSION,
        ),
    ] {
        let operation = json!({"operationId":"value","responses":{"200":{"description":"value","content":{"application/json":{"schema":schema}}}}});
        let contract = load(
            json!({"openapi":"3.1.2","info":{"title":"Version selection","version":"1"},"servers":[{"url":"https://example.test"}],"security":[],"paths":{"/value":{"get":operation}}}),
        );
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let plan = kotlin_sdk::plan_sdk(contract, &selected, config()).unwrap();
        assert_eq!(plan.program().version, version);
        assert!(plan.program().resource_context.is_none());
    }
}
