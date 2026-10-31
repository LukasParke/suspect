//! Inline and extension suppression through `apply_policy`.

use std::sync::Arc;

use suspect_check::{Context, Policy, PolicyOutcome, apply_policy_report, check_document};
use suspect_diag::{CheckId, Finding, Level, Severity};
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};

const MISSING_ID: &str = "oas-operation-missing-operationId";

fn doc(name: &str, text: &str) -> LowDoc {
    LowDoc::parse(
        Uri::parse(&format!("file:///w/{name}")).unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

/// Every check (no workspace: lint, markdown, syntax, extensions) under
/// `policy`.
fn run_with(low: &LowDoc, policy: &Policy) -> PolicyOutcome {
    let cx = Context {
        workspace: None,
        policy,
    };
    apply_policy_report(check_document(&cx, low), low, policy)
}

fn run(low: &LowDoc) -> PolicyOutcome {
    run_with(low, &Policy::default())
}

/// Lines (1-based) of the findings carrying `code`.
fn lines(low: &LowDoc, findings: &[Finding], code: &str) -> Vec<u32> {
    findings
        .iter()
        .filter(|f| f.code == code)
        .map(|f| {
            let start = f.range.as_ref().map_or(0, |r| r.start);
            low.inner()
                .line_index()
                .line_col(low.inner().bytes(), start)
                .0
                + 1
        })
        .collect()
}

fn has(findings: &[Finding], code: &str) -> bool {
    findings.iter().any(|f| f.code == code)
}

const TWO_OPS: &str = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
paths:
  /a:
    # suspect-disable-next-line oas-operation-missing-operationId -- partner team
    get:
      summary: a
      responses: {}
  /b:
    get:
      summary: b
      responses: {}
";

#[test]
fn a_comment_suppresses_the_next_pair_and_its_subtree() {
    let low = doc("api.yaml", TWO_OPS);
    let out = run(&low);
    // The `/b` operation keeps its finding; `/a`'s is suppressed.
    assert_eq!(lines(&low, &out.findings, MISSING_ID), [13]);
    assert_eq!(lines(&low, &out.suppressed, MISSING_ID), [9]);
    assert!(!has(&out.findings, "suppression-unknown-code"));
}

#[test]
fn the_short_form_and_alias_names_work() {
    // `operation-operationId` is the lint alias of the canonical code.
    let text = TWO_OPS.replace(
        "suspect-disable-next-line oas-operation-missing-operationId -- partner team",
        "suspect-disable operation-operationId",
    );
    let low = doc("api.yaml", &text);
    let out = run(&low);
    assert_eq!(lines(&low, &out.findings, MISSING_ID), [13]);
    assert_eq!(out.suppressed.len(), 1);
    assert!(!has(&out.findings, "suppression-unknown-code"));
}

#[test]
fn a_comment_suppresses_a_sequence_item() {
    let text = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
servers:
  # suspect-disable-next-line md-bare-url
  - url: https://a.example.com
    description: See https://a.example.com
  - url: https://b.example.com
    description: See https://b.example.com
paths: {}
";
    let low = doc("api.yaml", text);
    let out = run(&low);
    assert_eq!(lines(&low, &out.findings, "md-bare-url"), [10]);
    assert_eq!(lines(&low, &out.suppressed, "md-bare-url"), [8]);
}

#[test]
fn a_trailing_comment_applies_to_its_own_line() {
    let text = TWO_OPS.replace(
        "    # suspect-disable-next-line oas-operation-missing-operationId -- partner team\n    get:\n",
        "    get: # suspect-disable oas-operation-missing-operationId\n",
    );
    let low = doc("api.yaml", &text);
    let out = run(&low);
    assert_eq!(lines(&low, &out.findings, MISSING_ID), [12]);
    assert_eq!(out.suppressed.len(), 1);
}

#[test]
fn file_level_suppression_covers_the_document() {
    let text = format!("# suspect-disable-file {MISSING_ID} info-contact\n{TWO_OPS}");
    let low = doc("api.yaml", &text);
    let out = run(&low);
    assert!(!has(&out.findings, MISSING_ID));
    assert!(!has(&out.findings, "info-contact"));
    assert!(has(&out.suppressed, "info-contact"));
}

#[test]
fn the_extension_suppresses_inside_its_mapping_in_json() {
    let text = r#"{
  "openapi": "3.1.0",
  "info": {"title": "T", "version": "1"},
  "paths": {
    "/a": {"get": {"x-suspect-disable": ["oas-operation-missing-operationId"], "responses": {}}},
    "/b": {"get": {"responses": {}}}
  }
}
"#;
    let low = doc("api.json", text);
    let out = run(&low);
    assert_eq!(lines(&low, &out.findings, MISSING_ID), [6]);
    assert_eq!(lines(&low, &out.suppressed, MISSING_ID), [5]);
    // The key has a builtin schema: a list of codes.
    assert!(!has(&out.findings, "extension-schema"));
}

#[test]
fn the_extension_works_in_yaml_and_is_schema_checked() {
    let text = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
  x-suspect-disable: [info-contact]
paths:
  /a:
    get:
      x-suspect-disable: operation-operationId
      responses: {}
";
    let low = doc("api.yaml", text);
    let out = run(&low);
    assert!(!has(&out.findings, "info-contact"));
    assert!(has(&out.suppressed, "info-contact"));
    // A bare string is not honoured, and the schema says so.
    assert!(has(&out.findings, MISSING_ID));
    assert!(has(&out.findings, "extension-schema"));
}

#[test]
fn syntax_errors_cannot_be_suppressed() {
    let text = "# suspect-disable-file syntax-error syntax\na: \"unclosed\n";
    let low = doc("bad.yaml", text);
    let out = run(&low);
    assert!(has(&out.findings, "syntax-error"));
    assert!(out.suppressed.is_empty());
    let problems: Vec<&str> = out
        .findings
        .iter()
        .filter(|f| f.code == "suppression-unknown-code")
        .map(|f| f.message.as_str())
        .collect();
    assert_eq!(
        problems,
        [
            "`syntax-error` cannot be suppressed",
            "suppression names unknown code `syntax`"
        ]
    );
}

#[test]
fn unknown_codes_warn_and_unused_ones_are_off_by_default() {
    let text = TWO_OPS.replace(
        "oas-operation-missing-operationId -- partner team",
        "oas-operation-missing-operationid info-license",
    );
    let low = doc("api.yaml", &text);
    let out = run(&low);
    let unknown: Vec<&Finding> = out
        .findings
        .iter()
        .filter(|f| f.code == "suppression-unknown-code")
        .collect();
    assert_eq!(unknown.len(), 1, "{unknown:?}");
    assert_eq!(unknown[0].severity, Severity::Warning);
    assert_eq!(unknown[0].check, CheckId::Suppression);
    assert!(
        unknown[0]
            .message
            .contains("oas-operation-missing-operationid")
    );
    // `info-license` is known but fires at `info`, outside the node.
    assert!(!has(&out.findings, "suppression-unused"));

    let mut policy = Policy::default();
    policy
        .overrides
        .insert("suppression-unused".into(), Level::On(Severity::Warning));
    policy
        .overrides
        .insert("suppression-unknown-code".into(), Level::Off);
    let out = run_with(&low, &policy);
    let unused: Vec<&Finding> = out
        .findings
        .iter()
        .filter(|f| f.code == "suppression-unused")
        .collect();
    assert_eq!(unused.len(), 1);
    assert_eq!(unused[0].severity, Severity::Warning);
    assert!(unused[0].message.contains("info-license"));
    assert!(!has(&out.findings, "suppression-unknown-code"));
}

#[test]
fn validate_findings_are_suppressed_with_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let text = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
paths:
  # suspect-disable oas-path-param-not-declared
  /a/{id}:
    get:
      operationId: a
      responses: {}
  /b/{id}:
    get:
      operationId: b
      responses: {}
";
    let path = dir.path().join("api.yaml");
    std::fs::write(&path, text).unwrap();
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    ws.load_all("api.yaml").unwrap();
    let ws = Arc::new(ws);
    let low = LowDoc::parse(
        Uri::from_path(&path).unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
    );
    let policy = Policy::default();
    let cx = Context {
        workspace: Some(&ws),
        policy: &policy,
    };
    let out = apply_policy_report(check_document(&cx, &low), &low, &policy);
    assert_eq!(
        lines(&low, &out.findings, "oas-path-param-not-declared"),
        [12]
    );
    assert_eq!(
        lines(&low, &out.suppressed, "oas-path-param-not-declared"),
        [8]
    );
}
