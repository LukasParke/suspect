//! Contract-position and diagnostic regressions through the public CLI.

use std::process::Command;

fn validate(source: &str) -> (i32, serde_json::Value) {
    validate_named("api.yaml", source)
}

fn validate_named(name: &str, source: &str) -> (i32, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args(["validate", "--format", "json"])
        .arg(path)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr))),
    )
}

#[test]
fn reference_shaped_example_data_does_not_load_a_document_or_fail_validation() {
    // A model may describe JSON documents. Like the schema-keyword-named
    // properties in OpenRouter's provider-monitor contract, these are data.
    let (status, findings) = validate(
        "openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Document:
      type: object
      properties:
        $ref: {type: string}
      example: {$ref: 'missing-example.yaml#/data'}
      default: {$ref: '#/not-a-contract-reference'}
",
    );
    assert_eq!(status, 0, "{findings}");
    assert_eq!(findings.as_array().unwrap().len(), 0);
}

#[test]
fn json_validation_rejects_yaml_only_escape_sequences() {
    let (status, findings) = validate_named(
        "api.json",
        r#"{
  "openapi": "3.1.0",
  "info": {"title": "t", "version": "1"},
  "paths": {},
  "components": {"schemas": {"Letter": {"type": "string", "default": "\x41"}}}
}"#,
    );
    assert_eq!(
        status, 1,
        "invalid JSON must not be repaired as YAML: {findings}"
    );
    assert!(
        findings
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "syntax-error" && finding["line"] == 5)
    );
}

#[test]
fn malformed_reference_values_are_located_errors() {
    for value in ["'#/components/schemas/%ZZ'", "42", ""] {
        let (status, findings) = validate(&format!(
            "openapi: 3.1.0\ninfo: {{title: t, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Broken:\n      $ref: {value}\n"
        ));
        assert_eq!(status, 1, "malformed reference {value:?}: {findings}");
        assert!(
            findings
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["code"] == "invalid-ref" && finding["line"] == 7),
            "{findings}"
        );
    }
}

#[test]
fn referenced_json_syntax_errors_keep_their_source_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    std::fs::write(&path, "openapi: 3.1.0\ninfo: {title: t, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Model: {$ref: 'shared.json#/Model'}\n").unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"Model": {"type": "string", "default": "\x41"}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args(["validate", "--format", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let findings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        findings
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "syntax-error"
                && finding["line"] == 1
                && finding["file"].as_str().unwrap().ends_with("/shared.json")),
        "{findings}"
    );
}
