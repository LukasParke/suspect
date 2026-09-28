//! Contract-position and diagnostic regressions through the public CLI.

use std::process::Command;

fn validate(source: &str) -> (i32, serde_json::Value) {
    validate_named("api.yaml", source)
}

fn validate_named(name: &str, source: &str) -> (i32, serde_json::Value) {
    validate_with(name, source, &[])
}

fn validate_with(name: &str, source: &str, extra: &[&str]) -> (i32, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args(["validate", "--format", "json"])
        .args(extra)
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

// -------------------------------------------------- instance diagnostics

#[test]
fn instance_diagnostics_land_in_the_original_file_not_the_wrapper() {
    // Regression: the schema-instance checks compile schemas in a
    // synthetic `mem://` wrapper document; their findings used to carry
    // that URI, which the CLI refused ("diagnostic document is not
    // loaded") instead of reporting the violation.
    let (status, findings) = validate(
        r#"openapi: 3.1.0
info: {title: Format Suite, version: '1'}
paths:
  /events:
    get:
      operationId: listEvents
      responses:
        '200':
          description: page
          content:
            application/json:
              schema:
                type: object
                properties:
                  when: {type: string}
                examples: [{when: 123}]
"#,
    );
    let violations: Vec<_> = findings
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == "oas-schema-instance-invalid")
        .collect();
    assert!(
        !violations.is_empty(),
        "instance violation must surface: {findings}"
    );
    for finding in violations {
        assert!(
            finding["file"].as_str().unwrap().ends_with("api.yaml"),
            "instance diagnostic must attribute the original file: {finding}"
        );
    }
    assert_eq!(status, 0, "warnings do not fail the run");
}

#[test]
fn strict_format_turns_format_keywords_into_assertions() {
    let source = r#"openapi: 3.1.0
info: {title: Format Suite, version: '1'}
paths:
  /events:
    get:
      operationId: listEvents
      responses:
        '200':
          description: page
          content:
            application/json:
              schema:
                type: object
                properties:
                  when: {type: string, format: date-time}
                required: [when]
                examples: [{when: 'not a timestamp'}]
"#;
    // Default: `format` is an annotation (RFC 2020-12) — no finding.
    let (default_status, default_findings) = validate(source);
    assert!(
        !serde_json::to_string(&default_findings)
            .unwrap()
            .contains("valid `date-time`"),
        "format must stay annotation-only by default: {default_findings}"
    );
    assert_eq!(default_status, 0);

    // --strict-format: the declared format asserts.
    let (strict_status, strict_findings) = validate_with("api.yaml", source, &["--strict-format"]);
    let text = serde_json::to_string(&strict_findings).unwrap();
    assert!(text.contains("valid `date-time`"), "{text}");
    let _ = strict_status;
}

#[test]
fn strict_format_passes_clean_declared_formats() {
    let (status, findings) = validate_with(
        "api.yaml",
        r#"openapi: 3.1.0
info: {title: Format Suite, version: '1'}
paths:
  /events:
    get:
      operationId: listEvents
      responses:
        '200':
          description: page
          content:
            application/json:
              schema:
                type: object
                properties:
                  when: {type: string, format: date-time}
                  count: {type: integer, format: int32}
                required: [when]
                examples: [{when: '2026-01-15T09:30:00Z', count: 3}]
"#,
        &["--strict-format"],
    );
    assert_eq!(status, 0, "{findings}");
    assert!(
        !serde_json::to_string(&findings)
            .unwrap()
            .contains("oas-schema-instance-invalid"),
        "valid formats must not violate: {findings}"
    );
}

#[test]
fn recursive_component_refs_validate_examples_at_depth() {
    // Previously blocked: recursive `$ref`s collapsed to permissive
    // beyond inlining depth 8. Components now ride along in the compile
    // wrapper and the compiler resolves refs against it, so a violation
    // nested through the recursion surfaces.
    let (_, findings) = validate(
        r#"openapi: 3.1.0
info: {title: Tree Suite, version: '1'}
paths:
  /trees:
    get:
      operationId: listTrees
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Node'}
components:
  schemas:
    Node:
      type: object
      required: [name]
      properties:
        name: {type: string}
        child: {$ref: '#/components/schemas/Node'}
      examples:
        - name: root
          child:
            name: mid
            child:
              name: 7
"#,
    );
    let text = serde_json::to_string(&findings).unwrap();
    assert!(
        text.contains("oas-schema-instance-invalid") && text.contains("expected `string`"),
        "the recursion-depth violation must surface: {text}"
    );
}
