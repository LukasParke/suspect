//! Diagnostic guidance: every code the battery emits carries a stable
//! summary and an actionable how-to-fix, in the spirit of pre-generation
//! linter rules that answer "how do I fix it?" without leaving the editor.

use std::path::Path;
use std::sync::Arc;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::validate_entry;

fn session_with(dir: &Path, name: &str, content: &str) -> Session {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    Session::new(Arc::new(ws))
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suspect-validate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A spec that trips codes across many check groups in one document.
const KITCHEN_SINK: &str = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
  license:
    name: MIT
servers:
  - url: https://example.com/{missing}
    variables:
      region:
        enum: [us, eu]
        default: ap
paths:
  /pets/:
    get:
      tags: [ghost]
      deprecated: true
      parameters:
        - name: q
      responses:
        '200':
          content:
            application/json:
              schema:
                type: string
                x-invalid: true
              examples:
                - value: 17
components:
  securitySchemes:
    ApiKey:
      type: apiKey
  schemas:
    Pet:
      type: object
      discriminator:
        mapping:
          dog: Ghost
      properties:
        kind:
          type: int32
";

#[test]
fn every_emitted_diagnostic_carries_summary_and_how_to_fix() {
    let dir = unique_dir("guidance");
    let session = session_with(&dir, "main.yaml", KITCHEN_SINK);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        findings.len() >= 10,
        "kitchen sink must trip many groups; got {}",
        findings.len()
    );
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for d in &findings {
        assert!(!d.summary.is_empty(), "summary missing for {}", d.code);
        assert!(
            !d.how_to_fix.is_empty(),
            "how-to-fix missing for {}: {}",
            d.code,
            d.message
        );
        // The summary is the rule's stable one-liner; the message is the
        // located instance. They must never be identical, or one of the two
        // is not doing its job.
        assert_ne!(d.summary, d.message, "{}", d.code);
        seen.insert(d.code);
    }
    // Spot-check the groups the fixture is designed to trip.
    for code in [
        "oas-path-trailing-slash",
        "oas-tag-undeclared",
        "oas-parameter-missing-in",
        "oas-response-missing-description",
        "oas-schema-unknown-type",
        "oas-server-variable-unknown",
        "oas-operation-missing-operationId",
        "oas-operation-no-error-response",
        "oas-license-missing-url",
        "oas-deprecated-operation",
    ] {
        assert!(seen.contains(code), "{code} not tripped by the fixture");
        assert!(
            findings
                .iter()
                .find(|d| d.code == code)
                .is_some_and(|d| !d.how_to_fix.is_empty()),
            "{code} carries no guidance"
        );
    }
}

#[test]
fn no_error_response_rule_fires_and_cleans() {
    let firing = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /a:
    get:
      operationId: getA
      responses:
        '200': {description: ok}
";
    let session = session_with(&unique_dir("no-err-fire"), "main.yaml", firing);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        findings
            .iter()
            .any(|d| d.code == "oas-operation-no-error-response"),
        "{:?}",
        findings.iter().map(|d| d.code).collect::<Vec<_>>()
    );

    let clean = firing.replace(
        "'200': {description: ok}",
        "'200': {description: ok}\n        '404': {description: nope}",
    );
    let session = session_with(&unique_dir("no-err-clean"), "main.yaml", &clean);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        !findings
            .iter()
            .any(|d| d.code == "oas-operation-no-error-response"),
        "{:?}",
        findings.iter().map(|d| d.code).collect::<Vec<_>>()
    );

    // `default` also satisfies the rule.
    let defaulted = firing.replace("'200': {description: ok}", "default: {description: err}");
    let session = session_with(&unique_dir("no-err-default"), "main.yaml", &defaulted);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        !findings
            .iter()
            .any(|d| d.code == "oas-operation-no-error-response"),
        "{:?}",
        findings.iter().map(|d| d.code).collect::<Vec<_>>()
    );
}
