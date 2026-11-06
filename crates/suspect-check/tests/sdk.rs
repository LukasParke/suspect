//! The SDK category: `sdk-*` readiness codes and admission findings are
//! reported only when codegen is configured (decided 2026-10-29).

use std::path::Path;
use std::sync::Arc;

use suspect_check::{Context, Policy, apply_policy, check_document, codegen_configured};
use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};

/// Two operations without an `operationId`: `sdk-operation-missing-id`
/// for validate, `http-operation-id` refusals for admission.
const SPEC: &str = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
paths:
  /a:
    get:
      responses:
        '200':
          description: ok
";

fn run(dir: &Path, policy: &Policy) -> Vec<Finding> {
    let path = dir.join("api.yaml");
    std::fs::write(&path, SPEC).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    ws.load_all("api.yaml").unwrap();
    let ws = Arc::new(ws);
    let low = LowDoc::parse(
        Uri::from_path(&path).unwrap(),
        Source::from_vec(SPEC.as_bytes().to_vec()),
    );
    let cx = Context {
        workspace: Some(&ws),
        policy,
    };
    apply_policy(check_document(&cx, &low), &low, policy)
}

fn has(findings: &[Finding], code: &str) -> bool {
    findings.iter().any(|f| f.code == code)
}

fn resolved(dir: &Path) -> suspect_config::Resolved {
    suspect_config::resolve(dir).expect("config resolves")
}

#[test]
fn sdk_codes_need_codegen_configured() {
    let dir = tempfile::tempdir().unwrap();
    let resolved = resolved(dir.path());
    assert!(!codegen_configured(&resolved));
    let findings = run(dir.path(), &Policy::from_resolved(&resolved));
    assert!(!has(&findings, "sdk-operation-missing-id"));
    assert!(findings.iter().all(|f| f.check != CheckId::Admission));
    // The document-level problem is still reported once.
    assert!(has(&findings, "oas-operation-missing-operationId"));
}

#[test]
fn a_codegen_section_turns_the_category_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".suspect.yaml"),
        "codegen:\n  profile: typescript-http\n",
    )
    .unwrap();
    let resolved = resolved(dir.path());
    assert!(codegen_configured(&resolved));
    let findings = run(dir.path(), &Policy::from_resolved(&resolved));
    assert!(has(&findings, "sdk-operation-missing-id"), "{findings:#?}");
    // Admission runs as a check: the missing operationId is a refusal.
    let refusals: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == CheckId::Admission)
        .collect();
    assert!(!refusals.is_empty(), "{findings:#?}");
    let registry = suspect_check::registry::builtin();
    for finding in &refusals {
        let info = registry
            .info(&finding.code)
            .unwrap_or_else(|| panic!("{} is not registered", finding.code));
        assert_eq!(info.check, CheckId::Admission);
        assert!(finding.range.is_some(), "{finding:?}");
    }
    assert!(
        refusals
            .iter()
            .any(|f| f.code == "http-operation-id" && f.severity == Severity::Error)
    );
}

#[test]
fn manifest_codegen_targets_turn_the_category_on() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("suspect.project.json");
    std::fs::write(
        &manifest,
        r#"{"version": 1, "entry": "api.yaml", "codegen": []}"#,
    )
    .unwrap();
    assert!(!codegen_configured(&resolved(dir.path())));
    std::fs::write(
        &manifest,
        r#"{"version": 1, "entry": "api.yaml",
            "codegen": [{"profile": "typescript-http", "package_name": "x"}]}"#,
    )
    .unwrap();
    let resolved = resolved(dir.path());
    assert!(codegen_configured(&resolved));
    assert!(Policy::from_resolved(&resolved).sdk);
}

#[test]
fn the_flag_overrides_detection() {
    let dir = tempfile::tempdir().unwrap();
    // `check --sdk` sets the field directly.
    let policy = Policy {
        sdk: true,
        ..Policy::from_resolved(&resolved(dir.path()))
    };
    assert!(has(&run(dir.path(), &policy), "sdk-operation-missing-id"));
    let policy = Policy {
        checks: suspect_check::CheckSet::only([CheckId::Admission]),
        ..policy
    };
    let findings = run(dir.path(), &policy);
    assert!(!findings.is_empty());
    assert!(findings.iter().all(|f| f.check == CheckId::Admission));
}
