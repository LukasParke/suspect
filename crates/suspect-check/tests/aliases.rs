//! Aliases, dedupe and canonical severity (P1 step 4), through the whole
//! pass: `check_document` then `apply_policy`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_check::{Context, Policy, apply_policy, check_document};
use suspect_diag::{CheckId, Finding, Level, Severity};
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(path: &Path) -> LowDoc {
    LowDoc::parse(
        Uri::from_path(path).unwrap(),
        Source::from_vec(std::fs::read(path).unwrap()),
    )
}

/// The pass over `path`, with a workspace rooted at its directory.
fn run(path: &Path, policy: &Policy) -> Vec<Finding> {
    let low = load(path);
    let dir = path.parent().unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    let _ = ws.load_all(path.file_name().unwrap().to_str().unwrap());
    let ws = Arc::new(ws);
    let cx = Context {
        workspace: Some(&ws),
        policy,
    };
    apply_policy(check_document(&cx, &low), &low, policy)
}

fn with_code<'a>(findings: &'a [Finding], code: &str) -> Vec<&'a Finding> {
    findings.iter().filter(|f| f.code == code).collect()
}

#[test]
fn petstore_has_one_missing_operation_id_per_operation() {
    let findings = run(&repo().join("docs/demo/petstore.yaml"), &Policy::default());
    let missing = with_code(&findings, "oas-operation-missing-operationId");
    assert_eq!(missing.len(), 2, "{findings:#?}");
    assert!(missing.iter().all(|f| f.severity == Severity::Warning));
    // The owner's finding is the one kept.
    assert!(missing.iter().all(|f| f.check == CheckId::Validate));
    assert!(with_code(&findings, "operation-operationId").is_empty());
}

#[test]
fn an_override_through_the_alias_applies_and_the_canonical_one_wins() {
    let petstore = repo().join("docs/demo/petstore.yaml");
    let mut policy = Policy::default();
    policy
        .overrides
        .insert("operation-operationId".into(), Level::On(Severity::Error));
    let findings = run(&petstore, &policy);
    let missing = with_code(&findings, "oas-operation-missing-operationId");
    assert_eq!(missing.len(), 2);
    assert!(
        missing.iter().all(|f| f.severity == Severity::Error),
        "the alias override applies to the canonical code"
    );

    policy.overrides.insert(
        "oas-operation-missing-operationId".into(),
        Level::On(Severity::Info),
    );
    let findings = run(&petstore, &policy);
    assert!(
        with_code(&findings, "oas-operation-missing-operationId")
            .iter()
            .all(|f| f.severity == Severity::Info),
        "the canonical override wins over the alias"
    );

    policy.overrides.clear();
    policy
        .overrides
        .insert("operation-operationId".into(), Level::Off);
    let findings = run(&petstore, &policy);
    assert!(with_code(&findings, "oas-operation-missing-operationId").is_empty());
}

#[test]
fn swagger_keeps_the_lint_finding_under_the_canonical_code() {
    let swagger = repo().join("crates/suspect-check/tests/fixtures/broken.swagger.yaml");
    let findings = run(&swagger, &Policy::default());
    let missing = with_code(&findings, "oas-operation-missing-operationId");
    // Validate has no missing-operationId check for Swagger 2.0; lint's
    // finding is the only report and must survive.
    assert_eq!(missing.len(), 2, "{findings:#?}");
    assert!(missing.iter().all(|f| f.check == CheckId::Lint));
    assert!(missing.iter().all(|f| f.severity == Severity::Warning));
}

#[test]
fn a_redefined_custom_rule_keeps_its_code() {
    let dir = tempfile::tempdir().unwrap();
    let ruleset = dir.path().join("rules.yaml");
    std::fs::write(
        &ruleset,
        "rules:\n  operation-operationId:\n    description: Operations need a summary.\n    given: $.paths.*.get\n    severity: error\n    then:\n      function: defined\n      functionOptions:\n        property: summary\n",
    )
    .unwrap();
    let spec = dir.path().join("api.yaml");
    std::fs::write(
        &spec,
        "openapi: 3.1.0\ninfo: {title: t, version: '1'}\npaths:\n  /a:\n    get:\n      responses:\n        '200': {description: ok}\n",
    )
    .unwrap();
    let mut settings = suspect_config::Settings::default();
    settings.lint.ruleset = Some(ruleset);
    let policy = Policy::from_settings(&settings);
    assert!(policy.custom_ruleset);
    let findings = run(&spec, &policy);
    let custom = with_code(&findings, "operation-operationId");
    assert_eq!(custom.len(), 1, "{findings:#?}");
    assert_eq!(custom[0].severity, Severity::Error);
    // Validate's own finding is untouched and not merged with it.
    assert_eq!(
        with_code(&findings, "oas-operation-missing-operationId").len(),
        1
    );
}

#[test]
fn a_multi_target_alias_merges_with_the_target_at_its_position() {
    let arazzo = repo().join("crates/suspect-check/tests/fixtures/broken.arazzo.yaml");
    let findings = run(&arazzo, &Policy::default());
    assert!(
        with_code(&findings, "arazzo-step-operation").is_empty(),
        "{findings:#?}"
    );
    assert_eq!(
        with_code(&findings, "arazzo-step-missing-operation").len(),
        1
    );
    assert_eq!(
        with_code(&findings, "arazzo-step-multiple-targets").len(),
        1
    );
}

#[test]
fn the_overlay_description_pair_is_one_finding() {
    let overlay = repo().join("crates/suspect-check/tests/fixtures/actions.overlay.yaml");
    let findings = run(&overlay, &Policy::default());
    assert!(with_code(&findings, "overlay-action-description").is_empty());
    let kept = with_code(&findings, "overlay-action-missing-description");
    assert_eq!(kept.len(), 1, "{findings:#?}");
    assert_eq!(kept[0].check, CheckId::Overlay);
}

#[test]
fn syntax_errors_carry_one_code_configured_by_that_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.yaml");
    std::fs::write(&path, "a: \"unclosed\n").unwrap();
    let findings = run(&path, &Policy::default());
    assert!(!with_code(&findings, "syntax-error").is_empty());
    let mut policy = Policy::default();
    policy.overrides.insert("syntax".into(), Level::Off);
    assert!(
        !with_code(&run(&path, &policy), "syntax-error").is_empty(),
        "the old `syntax` name configures nothing"
    );
    policy.overrides.insert("syntax-error".into(), Level::Off);
    assert!(with_code(&run(&path, &policy), "syntax-error").is_empty());
}
