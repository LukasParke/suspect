//! TS rules through the check pass, against a real Bun worker. Skipped
//! with the standard line when bun is not on PATH, unless
//! `SUSPECT_REQUIRE_TOOLS` lists `bun` (CI does).

use std::path::{Path, PathBuf};

use suspect_check::rules::{self, Startup};
use suspect_check::{Policy, apply_policy, check_rules};
use suspect_diag::{CheckId, Level, Severity};
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

const SUMMARY_RULE: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "team-operation-summary", description: "ops need summaries" },
  given: r.operation,
  check(op, ctx) {
    if (!op.summary) {
      ctx.report({ message: `${op.method.toUpperCase()} ${op.path} has no summary`, at: op });
    }
  },
});
"#;

/// Reuses a builtin code: a configuration error, and its findings never
/// reach the check pass.
const COLLIDING_RULE: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "oas-tag-undeclared", description: "collides" },
  given: r.operation,
  check(op, ctx) {
    ctx.report({ message: "never shown", at: op });
  },
});
"#;

const SPEC: &str = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\npaths:\n  /a:\n    get:\n      summary: ok\n      responses: {}\n    post:\n      responses: {}\n";

fn bun() -> bool {
    if suspect_rules::discover::find_bun().is_some() {
        return true;
    }
    suspect_codegen::toolchain::skip("bun", "bun not on PATH — install from https://bun.sh");
    false
}

fn workspace(rules: &[(&str, &str)]) -> (tempfile::TempDir, LowDoc) {
    let dir = tempfile::tempdir().unwrap();
    let rules_dir = dir.path().join(".suspect/rules");
    std::fs::create_dir_all(&rules_dir).unwrap();
    for (name, text) in rules {
        std::fs::write(rules_dir.join(name), text).unwrap();
    }
    let path = dir.path().join("api.yaml");
    std::fs::write(&path, SPEC).unwrap();
    let doc = LowDoc::parse(
        Uri::from_path(&path).unwrap(),
        Source::from_vec(SPEC.as_bytes().to_vec()),
    );
    (dir, doc)
}

fn roots() -> Vec<PathBuf> {
    vec![PathBuf::from(".suspect/rules")]
}

async fn host(root: &Path) -> Box<suspect_rules::RuleHost> {
    match rules::start(root, &roots(), Some(10_000)).await {
        Startup::Started(host) => host,
        Startup::NoRules => panic!("no rules found"),
        Startup::Unavailable { .. } => panic!("bun unavailable"),
        Startup::Failed(error) => panic!("host failed: {error}"),
    }
}

#[tokio::test]
async fn ts_findings_go_through_policy() {
    if !bun() {
        return;
    }
    let (dir, doc) = workspace(&[("summary.ts", SUMMARY_RULE)]);
    let mut host = host(dir.path()).await;
    let raw = check_rules(host.as_mut(), &doc).await.expect("evaluate");
    assert_eq!(raw.len(), 1, "{raw:?}");
    let finding = &raw[0];
    assert_eq!(finding.check, CheckId::Rules);
    assert_eq!(finding.code, "team-operation-summary");
    assert_eq!(finding.severity, Severity::Warning);
    assert!(finding.pointer.as_deref().unwrap().ends_with("/post"));
    let range = finding.range.clone().expect("span resolved");
    assert!(SPEC[range].contains("responses"));

    // `lint.rules` applies to TS rule ids like any other code.
    let mut policy = Policy::default();
    policy
        .overrides
        .insert("team-operation-summary".into(), Level::On(Severity::Error));
    let kept = apply_policy(raw.clone(), &doc, &policy);
    assert_eq!(kept[0].severity, Severity::Error);
    policy
        .overrides
        .insert("team-operation-summary".into(), Level::Off);
    assert!(apply_policy(raw, &doc, &policy).is_empty());
    let _ = host.shutdown().await;
}

#[tokio::test]
async fn a_colliding_rule_is_reported_and_skipped() {
    if !bun() {
        return;
    }
    let (dir, doc) = workspace(&[
        ("summary.ts", SUMMARY_RULE),
        ("colliding.ts", COLLIDING_RULE),
    ]);
    let mut host = host(dir.path()).await;
    let collisions = rules::collisions(suspect_check::registry::builtin(), host.rules());
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].id, "oas-tag-undeclared");
    assert!(collisions[0].file.ends_with("colliding.ts"));
    let finding = collisions[0].finding().expect("rule file uri");
    assert_eq!(finding.code, rules::ID_COLLISION);
    assert_eq!(finding.severity, Severity::Error);
    let raw = check_rules(host.as_mut(), &doc).await.expect("evaluate");
    assert!(
        raw.iter().all(|f| f.code == "team-operation-summary"),
        "{raw:?}"
    );
    let _ = host.shutdown().await;
}

#[tokio::test]
async fn no_rule_files_start_nothing() {
    let (dir, _doc) = workspace(&[]);
    assert!(matches!(
        rules::start(dir.path(), &roots(), None).await,
        Startup::NoRules
    ));
}

#[test]
fn the_unavailable_warning_is_a_registered_rules_finding() {
    let uri = Uri::parse("file:///w/api.yaml").unwrap();
    let finding = rules::runtime_unavailable(uri, 2);
    assert_eq!(finding.code, rules::RUNTIME_UNAVAILABLE);
    assert_eq!(finding.severity, Severity::Warning);
    let info = suspect_check::registry::builtin()
        .info(rules::RUNTIME_UNAVAILABLE)
        .expect("registered");
    assert_eq!(info.check, CheckId::Rules);
    assert!(finding.message.contains("bun"));
}
