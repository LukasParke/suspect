//! `check_document` per family (moved from `suspect-lsp/src/diagnostics.rs`
//! with the checks).

use std::path::Path;
use std::sync::Arc;

use suspect_check::{Context, Policy, apply_policy, check_document};
use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::LowDoc;
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::{Source, Uri};

fn low_at(dir: &Path, name: &str, text: &str) -> LowDoc {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    LowDoc::parse(
        Uri::from_path(&path).unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

fn run(ws: Option<&Arc<Workspace>>, low: &LowDoc) -> Vec<Finding> {
    let policy = Policy::default();
    let cx = Context {
        workspace: ws,
        policy: &policy,
    };
    apply_policy(check_document(&cx, low), low, &policy)
}

fn codes(findings: &[Finding]) -> Vec<String> {
    findings.iter().map(|f| f.code.to_string()).collect()
}

#[test]
fn oas_docs_get_validation_and_lint() {
    let dir = tempfile::tempdir().unwrap();
    let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\npaths: {}\n";
    let low = low_at(dir.path(), "api.yaml", text);
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    ws.load_all("api.yaml").unwrap();
    let ws = Arc::new(ws);
    let findings = run(Some(&ws), &low);
    assert!(findings.iter().all(|f| f.check != CheckId::Syntax));
    assert!(findings.iter().any(|f| f.check == CheckId::Lint));
    // Without a workspace the semantic validation does not run.
    let bare = run(None, &low);
    assert!(bare.len() <= findings.len());
    assert!(bare.iter().all(|f| f.check != CheckId::Validate));
}

#[test]
fn arazzo_duplicate_workflow_ids_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let text = r#"
arazzo: 1.0.0
info:
  title: T
sourceDescriptions:
  - name: api
    url: openapi.yaml
workflows:
  - workflowId: w
    steps:
      - stepId: s1
  - workflowId: w
    steps:
      - stepId: s2
"#;
    let low = low_at(dir.path(), "flow.yaml", text);
    let findings = suspect_check::checks::arazzo(&low);
    assert!(
        findings.iter().any(|f| f.code.starts_with("arazzo-")),
        "{findings:?}"
    );
    assert!(findings.iter().all(|f| f.severity == Severity::Error));
}

#[test]
fn non_arazzo_family_skips_arazzo_checks() {
    let dir = tempfile::tempdir().unwrap();
    let low = low_at(dir.path(), "oas.yaml", "openapi: 3.1.0\ninfo: {}\n");
    assert!(suspect_check::checks::arazzo(&low).is_empty());
    assert!(suspect_check::checks::load_profile(None, &low).is_empty());
    assert!(suspect_check::checks::overlay(&low).is_empty());
    assert!(suspect_check::checks::swagger(&low).is_empty());
}

#[test]
fn load_profile_errors_and_unknown_workflows_are_located() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    std::fs::write(
        dir.join("spec.yaml"),
        "openapi: 3.1.0\ninfo: {title: t, version: '1'}\npaths:\n  /p:\n    get:\n      operationId: getP\n      responses:\n        '200': {description: ok}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("flows.arazzo.yaml"),
        "arazzo: 1.0.0\ninfo: {title: f, version: '1'}\nsourceDescriptions:\n  - name: api\n    url: spec.yaml\nworkflows:\n  - workflowId: real\n    steps:\n      - stepId: s\n        operationId: getP\n",
    )
    .unwrap();
    let text = "suspectLoad: \"1.0\"\narazzo: ./flows.arazzo.yaml\nphases:\n  - { duration: 10s, arrivalRate: 5 }\n  - { pause: soon }\nscenarios:\n  - workflow: ghost\n  - workflow: real\n";
    let low = low_at(dir, "p.load.yaml", text);
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    ws.load_all("spec.yaml").unwrap();
    ws.load_all("flows.arazzo.yaml").unwrap();
    let ws = Arc::new(ws);

    let findings = suspect_check::checks::load_profile(Some(&ws), &low);
    let all = codes(&findings);
    assert!(all.iter().any(|c| c == "load-duration"), "{all:?}");
    assert!(all.iter().any(|c| c == "load-unknown-workflow"), "{all:?}");
    // The bad `pause` literal is anchored on its own line.
    let pause = findings
        .iter()
        .find(|f| f.message.contains("`pause`"))
        .expect("pause finding");
    let start = pause.range.clone().unwrap().start;
    assert_eq!(text[..start].matches('\n').count(), 4, "{pause:?}");
    assert_eq!(pause.severity, Severity::Error);
    // The full pass routes by family too.
    assert!(!run(Some(&ws), &low).is_empty());
}

#[test]
fn overlay_structure_is_checked_with_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let text = "overlay: 1.0.0\ninfo:\n  title: O\n  version: '1'\nactions: []\n";
    let low = low_at(dir.path(), "o.overlay.yaml", text);
    let findings = suspect_check::checks::overlay(&low);
    assert_eq!(codes(&findings), vec!["overlay-empty-actions"]);
    assert_eq!(findings[0].severity, Severity::Error);
    assert_eq!(&text[findings[0].range.clone().unwrap()], "[]");
}

#[test]
fn configuration_files_are_checked_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let text = "lint:\n  min_severty: error\n";
    let config = low_at(dir.path(), ".suspect.yaml", text);
    let findings = run(None, &config);
    assert_eq!(codes(&findings), vec!["suspect.config"]);
    assert!(
        findings[0]
            .message
            .ends_with("Did you mean `min_severity`?"),
        "{findings:?}"
    );
    let other = low_at(dir.path(), "other.yaml", text);
    assert!(suspect_check::checks::config(&other).is_empty());
}

#[test]
fn only_selected_checks_run() {
    let dir = tempfile::tempdir().unwrap();
    let low = low_at(dir.path(), "bad.yaml", "openapi: 3.1.0\ninfo: \"x\n");
    let policy = Policy {
        checks: suspect_check::CheckSet::only([CheckId::Syntax]),
        ..Policy::default()
    };
    let cx = Context {
        workspace: None,
        policy: &policy,
    };
    let findings = check_document(&cx, &low);
    assert!(!findings.is_empty());
    assert!(findings.iter().all(|f| f.check == CheckId::Syntax));
}
