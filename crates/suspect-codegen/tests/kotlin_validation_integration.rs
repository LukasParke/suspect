#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, NativeSnapshot, PlanStatus},
    kotlin_sdk,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture(value: &Value) -> (PathBuf, Arc<Contract>) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-kotlin-applicators-integration");
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep();
    let path = root.join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    (path.clone(), load(&path))
}
fn load(path: &Path) -> Arc<Contract> {
    let ws = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&ws, &Uri::from_path(path).unwrap()).unwrap())
}
fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::KotlinHttp,
        package_name: "test.suspect.kotlin:scoped-sdk".into(),
        package_version: "0.3.0".into(),
        import_name: Some("example.scoped.sdk".into()),
    }
}
fn snapshot(contract: Arc<Contract>) -> NativeSnapshot {
    let mut snapshot = compatibility::snapshot(contract, &[], &[target()]).unwrap();
    let native = snapshot.native.remove(0);
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    native
}
fn descriptor<'a>(native: &'a NativeSnapshot, name: &str) -> &'a Value {
    native
        .models
        .iter()
        .find(|m| m.name == format!("example.scoped.sdk.{name}") && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap()
}

#[test]
fn scoped_carriers_and_pattern_extras_have_typed_native_records() {
    let doc: Value = serde_json::from_str(include_str!("kotlin_support/scoped-sdk.json")).unwrap();
    let (_, contract) = fixture(&doc);
    let native = snapshot(contract);
    assert!(
        native
            .runtime
            .profile
            .contains(suspect_schema::OwnedProgram::V2_PROFILE)
    );
    for name in ["ConditionalCarrier", "ScopedComposition", "ScopedArray"] {
        let desc = descriptor(&native, name);
        assert_eq!(desc["representation"], "schema-bound-json-carrier");
        assert_eq!(desc["constructor"]["parameters"][0]["name"], "value");
        assert_eq!(
            desc["constructor"]["parameters"][0]["type"]["name"],
            "example.scoped.sdk.JsonValue"
        );
        assert_eq!(desc["constructor"]["parameters"][0]["hasDefault"], false);
        assert_eq!(desc["componentMembers"], json!(["component1"]));
        assert_eq!(desc["modelOnlyObligation"], "use-source-bound-codec");
    }
    let pattern = descriptor(&native, "PatternRecord");
    let extra = pattern["constructor"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "additionalProperties")
        .unwrap();
    assert_eq!(extra["type"]["name"], "kotlin.collections.Map");
    assert_eq!(
        extra["type"]["arguments"][1]["name"],
        "example.scoped.sdk.JsonValue"
    );
    assert!(
        pattern["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "kind" && f["constructorParameter"] == false)
    );
}

#[test]
fn adding_patterns_to_a_closed_native_object_reports_its_new_carrier_field() {
    let mut doc = json!({"openapi":"3.1.2","info":{"title":"Pattern carrier compatibility","version":"1"},"security":[],"servers":[{"url":"https://example.test"}],"components":{"schemas":{"Record":{"type":"object","required":["label"],"properties":{"label":{"type":"string"}},"additionalProperties":false,"example":{"label":"source"}}}},"paths":{"/record":{"post":{"operationId":"record","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}},"responses":{"200":{"description":"record","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}}}}}}});
    let (path, before) = fixture(&doc);
    doc["components"]["schemas"]["Record"]["patternProperties"] = json!({"^x":{"type":"integer"}});
    std::fs::write(&path, doc.to_string()).unwrap();
    let report = compatibility::compare(before, load(&path), &[], &[target()]).unwrap();
    assert!(
        report.native[0]
            .before
            .as_ref()
            .unwrap()
            .findings
            .is_empty()
            && report.native[0].after.as_ref().unwrap().findings.is_empty()
    );
    assert!(report.native[0].changes.iter().any(
        |c| c.code == "native-model-shape-changed" && c.subject == "example.scoped.sdk.Record"
    ));
    let new = descriptor(report.native[0].after.as_ref().unwrap(), "Record");
    assert!(
        new["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "additionalProperties")
    );
}

#[test]
fn verified_default_sdk_adoption_matches_the_explicit_v2_entrypoint() {
    let doc: Value = serde_json::from_str(include_str!("kotlin_support/scoped-sdk.json")).unwrap();
    let (_, contract) = fixture(&doc);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = kotlin_sdk::SdkConfig {
        group_id: "test.suspect.kotlin".into(),
        artifact_id: "scoped-sdk".into(),
        version: "0.3.0".into(),
        package_name: "example.scoped.sdk".into(),
        credential_env: None,
    };
    let explicit = kotlin_sdk::plan_sdk_v2(contract.clone(), &selected, config.clone()).unwrap();
    let default = kotlin_sdk::plan_sdk(contract.clone(), &selected, config).unwrap();
    assert_eq!(explicit.program(), default.program());
    assert_eq!(explicit.render().unwrap(), default.render().unwrap());
    assert_eq!(
        backend::generate(contract, &selected, &target()).unwrap(),
        default.render().unwrap()
    );
}
