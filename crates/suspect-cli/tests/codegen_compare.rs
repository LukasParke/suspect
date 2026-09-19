//! Public comparison command keeps native and wire variance independent.
use serde_json::{Value, json};
use std::{path::Path, process::Command};
fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let document = json!({"openapi":"3.1.0","info":{"title":"Compatibility","version":"1"},"servers":[{"url":"https://example.test/v1"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/value":{"get":{"operationId":"getValue","responses":{"200":{"description":"value","content":{"application/json":{"schema":{"type":"string"}}}}}}}}});
    for side in ["old", "new"] {
        std::fs::create_dir(root.path().join(side)).unwrap();
        std::fs::write(
            root.path().join(side).join("api.json"),
            document.to_string(),
        )
        .unwrap();
        std::fs::write(root.path().join(side).join("sdk.json"),json!({"spec":"api.json","targets":[{"backend":"python-http","package_name":"compare-sdk","package_version":"1.0.0"}],"operation_ids":["getValue"]}).to_string()).unwrap();
    }
    root
}
fn compare(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args([
            "codegen-compare",
            "--before",
            "old/sdk.json",
            "--after",
            "new/sdk.json",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
#[test]
fn response_widening_and_operation_removal_are_source_linked_reports() {
    let root = setup();
    let path = root.path().join("new/api.json");
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["paths"]["/value"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["type"] =
        json!(["string", "null"]);
    std::fs::write(&path, doc.to_string()).unwrap();
    let result = compare(root.path(), &["--format", "json"]);
    assert_eq!(result.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["format"], "suspect-sdk-compatibility-v1");
    assert!(
        report["wire"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["direction"] == "response" && finding["impact"] != "compatible")
    );
    assert!(
        report["native"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "native-model-shape-changed")
    );
    doc["paths"] = json!({});
    std::fs::write(&path, doc.to_string()).unwrap();
    let result = compare(root.path(), &["--format", "json"]);
    assert_eq!(result.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        report["wire"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "wire-operation-removed")
    );
    assert!(
        report["native"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "native-operation-removed")
    );
    assert!(!root.path().join("python").exists());
    let notes = compare(root.path(), &[]);
    assert!(
        String::from_utf8(notes.stdout)
            .unwrap()
            .contains("# SDK compatibility")
    );
}
#[test]
fn invalid_comparison_configuration_is_distinct_from_incompatibility() {
    let root = setup();
    std::fs::write(root.path().join("new/sdk.json"), "invalid").unwrap();
    let result = compare(root.path(), &["--format", "json"]);
    assert_eq!(result.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["status"], "comparison-error");
}
