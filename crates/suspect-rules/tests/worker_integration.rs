//! Integration tests against a real bun worker. Skipped gracefully when
//! bun is not on PATH (mirrors the corpus-test convention).

use std::path::PathBuf;

use suspect_low::LowDoc;
use suspect_rules::{RuleHost, RulesConfig, StartOptions, discover_rule_files};
use suspect_source::{Source, Uri};

fn bun_available() -> bool {
    suspect_rules::discover::find_bun().is_some()
}

fn workspace() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests-fixtures/rules-ts")
}

fn spec_doc(rel: &str) -> LowDoc {
    let path = workspace().join(rel);
    let bytes = std::fs::read(&path).expect("fixture readable");
    LowDoc::parse(
        Uri::from(format!("mem://{}", rel).leak() as &str),
        Source::from_vec(bytes),
    )
}

const SUMMARY_RULE: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "operation-summary", description: "ops need summaries" },
  given: r.operation,
  check(op, ctx) {
    if (!op.summary) {
      ctx.report({
        message: `${op.method.toUpperCase()} ${op.path} is missing a summary`,
        at: op,
      });
    }
  },
});
"#;

const HANGING_RULE: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "hang-forever", description: "test watchdog" },
  given: r.operation,
  check(_op, _ctx) {
    // Busy-loop: the host watchdog must kill the worker.
    const start = Date.now();
    while (Date.now() - start < 60_000) {}
  },
});
"#;

const THROWING_RULE: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "throws", description: "test error handling" },
  given: r.operation,
  check() {
    throw new Error("boom from rule");
  },
});
"#;

const WALK_RULE: &str = r#"
import { defineRule } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "unique-operation-ids", description: "no duplicate ids" },
  visitors: {
    onDocument(_doc, ctx) {
      ctx.state.ids = new Map();
    },
    Operation(op, ctx) {
      const ids = ctx.state.ids;
      if (typeof op.operationId === "string") {
        if (ids.has(op.operationId)) {
          ctx.report({
            message: `duplicate operationId ${op.operationId}`,
            at: { pointer: op.pointer },
          });
        }
        ids.set(op.operationId, op.pointer);
      }
    },
  },
});
"#;

fn write_rules(dir: &std::path::Path, files: &[(&str, &str)]) -> PathBuf {
    std::fs::create_dir_all(dir).expect("mkdir");
    for (name, content) in files {
        std::fs::write(dir.join(name), content).expect("write rule");
    }
    dir.to_path_buf()
}

#[tokio::test]
async fn point_rule_finds_violations_with_spans() {
    if !bun_available() {
        eprintln!("skipping: bun not on PATH");
        return;
    }
    let ws = workspace();
    let rules_dir = write_rules(&ws.join(".rules-it/point"), &[("summary.ts", SUMMARY_RULE)]);

    let mut host = RuleHost::start(StartOptions {
        workspace_root: ws.clone(),
        rule_files: vec![rules_dir.join("summary.ts")],
        timeout_ms: Some(10_000),
        ..Default::default()
    })
    .await
    .expect("host starts")
    .expect("rules present");

    assert_eq!(host.rules().len(), 1);
    let doc = spec_doc("openapi-basic.yaml");
    let findings = host.evaluate(&doc).await.expect("evaluate");

    let missing: Vec<&str> = findings
        .iter()
        .filter(|f| f.pointer.contains("post"))
        .map(|f| f.pointer.as_str())
        .collect();
    assert_eq!(
        missing,
        ["/paths/~1pets/post"],
        "exactly the post op flagged"
    );
    let f = &findings[0];
    assert!(f.span.is_some(), "host resolves spans from the CST");
    assert_eq!(f.severity, None);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn throwing_rule_is_disabled_not_fatal() {
    if !bun_available() {
        eprintln!("skipping: bun not on PATH");
        return;
    }
    let ws = workspace();
    let rules_dir = write_rules(
        &ws.join(".rules-it/throw"),
        &[("summary.ts", SUMMARY_RULE), ("throws.ts", THROWING_RULE)],
    );

    let mut host = RuleHost::start(StartOptions {
        workspace_root: ws.clone(),
        rule_files: vec![rules_dir.join("summary.ts"), rules_dir.join("throws.ts")],
        timeout_ms: Some(10_000),
        ..Default::default()
    })
    .await
    .expect("host starts")
    .expect("rules present");

    let doc = spec_doc("openapi-basic.yaml");
    let findings = host.evaluate(&doc).await.expect("evaluate survives");
    assert!(
        findings.iter().all(|f| f.rule_id != "throws"),
        "throwing rule produced no findings"
    );
    assert!(host.disabled().contains("throws"), "rule disabled");
    // Healthy rules still ran.
    assert!(findings.iter().any(|f| f.rule_id == "operation-summary"));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn watchdog_kills_and_restarts_on_hang() {
    if !bun_available() {
        eprintln!("skipping: bun not on PATH");
        return;
    }
    let ws = workspace();
    let rules_dir = write_rules(&ws.join(".rules-it/hang"), &[("hang.ts", HANGING_RULE)]);

    let mut host = RuleHost::start(StartOptions {
        workspace_root: ws.clone(),
        rule_files: vec![rules_dir.join("hang.ts")],
        timeout_ms: Some(500),
        ..Default::default()
    })
    .await
    .expect("host starts")
    .expect("rules present");

    let doc = spec_doc("openapi-basic.yaml");
    let started = std::time::Instant::now();
    let findings = host
        .evaluate(&doc)
        .await
        .expect("evaluate times out gracefully");
    let elapsed = started.elapsed();
    assert!(findings.is_empty(), "timed-out run yields nothing");
    assert!(
        elapsed >= std::time::Duration::from_millis(500)
            && elapsed < std::time::Duration::from_secs(10),
        "watchdog fired near the deadline: {elapsed:?}"
    );
    // The host is still alive: a healthy re-run works.
    assert!(host.runs() >= 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn walk_rule_reports_duplicates() {
    if !bun_available() {
        eprintln!("skipping: bun not on PATH");
        return;
    }
    let ws = workspace();
    let rules_dir = write_rules(&ws.join(".rules-it/walk"), &[("dup-ids.ts", WALK_RULE)]);

    let mut host = RuleHost::start(StartOptions {
        workspace_root: ws.clone(),
        rule_files: vec![rules_dir.join("dup-ids.ts")],
        timeout_ms: Some(10_000),
        ..Default::default()
    })
    .await
    .expect("host starts")
    .expect("rules present");

    let doc = spec_doc("openapi-dup-ids.yaml");
    let findings = host.evaluate(&doc).await.expect("evaluate");
    assert_eq!(findings.len(), 1, "one duplicate pair flagged");
    assert_eq!(findings[0].rule_id, "unique-operation-ids");
    assert!(findings[0].message.contains("duplicate operationId"));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn hot_reload_picks_up_new_rules() {
    if !bun_available() {
        eprintln!("skipping: bun not on PATH");
        return;
    }
    let ws = workspace();
    let rules_dir = write_rules(
        &ws.join(".rules-it/reload"),
        &[("summary.ts", SUMMARY_RULE)],
    );

    let mut host = RuleHost::start(StartOptions {
        workspace_root: ws.clone(),
        rule_files: vec![rules_dir.join("summary.ts")],
        timeout_ms: Some(10_000),
        ..Default::default()
    })
    .await
    .expect("host starts")
    .expect("rules present");
    assert_eq!(host.rules().len(), 1);

    std::fs::write(rules_dir.join("extra.ts"), THROWING_RULE).expect("write extra");
    host.reload(&[rules_dir.join("summary.ts"), rules_dir.join("extra.ts")])
        .await
        .expect("reload");
    assert_eq!(host.rules().len(), 2, "new rule registered after reload");
    host.shutdown().await.unwrap();
}

#[test]
fn discovery_scans_sorted_without_bun() {
    let ws = workspace();
    let dir = write_rules(
        &ws.join(".rules-it/discover"),
        &[("b.ts", SUMMARY_RULE), ("a.ts", SUMMARY_RULE)],
    );
    let config = RulesConfig {
        rule_files: Vec::new(),
        dir: Some(dir.clone()),
        timeout_ms: None,
        bun: None,
    };
    let files = discover_rule_files(&ws, &config).expect("discovery");
    let names: Vec<&str> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["a.ts", "b.ts"], "deterministic order");
}
