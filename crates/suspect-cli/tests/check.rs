//! `suspect check`: exit statuses, the `--only` / `--parse-only` filters,
//! `validate` and `lint` as filters of the same pass, the SDK category,
//! and TypeScript rules in the pass.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn suspect(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(cwd)
        .args(args)
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .env_remove("SUSPECT_LINT_RULESET")
        .env_remove("SUSPECT_STRICT_FORMAT")
        .output()
        .expect("run suspect")
}

/// Exit status and the `findings` of `--format json`.
fn json(cwd: &Path, args: &[&str]) -> (i32, Vec<Value>, Value) {
    let mut argv = args.to_vec();
    argv.extend(["--format", "json"]);
    let out = suspect(cwd, &argv);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    (out.status.code().unwrap_or(-1), findings, report)
}

/// `(code, line, col)` of each finding, sorted.
fn located(findings: &[Value]) -> Vec<(String, u64, u64)> {
    let mut out: Vec<(String, u64, u64)> = findings
        .iter()
        .map(|f| {
            (
                f["code"].as_str().unwrap().to_owned(),
                f["line"].as_u64().unwrap(),
                f["col"].as_u64().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn codes(findings: &[Value]) -> Vec<&str> {
    findings
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect()
}

fn checks(findings: &[Value]) -> Vec<&str> {
    let mut out: Vec<&str> = findings
        .iter()
        .map(|f| f["check"].as_str().unwrap())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

const CLEAN: &str = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\n  description: d\n  contact: {name: c, url: 'https://example.com'}\n  license: {name: MIT, url: 'https://opensource.org/licenses/MIT'}\ntags:\n  - {name: things, description: Things}\npaths:\n  /things:\n    get:\n      operationId: listThings\n      summary: List things\n      description: Lists things.\n      tags: [things]\n      responses:\n        '200': {description: ok}\n        '429': {description: slow down}\n        '400': {description: bad}\n";

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

#[test]
fn exit_statuses() {
    let dir = project(&[
        ("clean.yaml", CLEAN),
        // An undeclared path parameter is an error.
        (
            "broken.yaml",
            "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a/{id}:\n    get:\n      operationId: getA\n      responses: {'200': {description: ok}}\n",
        ),
        // Missing operationId only: warnings never fail the run.
        (
            "warned.yaml",
            "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a:\n    get:\n      responses: {'200': {description: ok}}\n",
        ),
    ]);
    let root = dir.path();
    let (status, findings, _) = json(root, &["check", "clean.yaml"]);
    assert_eq!(status, 0, "{findings:#?}");
    assert!(
        findings.iter().all(|f| f["severity"] != "error"),
        "{findings:#?}"
    );
    let (status, findings, _) = json(root, &["check", "broken.yaml"]);
    assert_eq!(status, 1);
    assert!(codes(&findings).contains(&"oas-path-param-not-declared"));
    let (status, findings, _) = json(root, &["check", "warned.yaml"]);
    assert_eq!(status, 0, "{findings:#?}");
    assert!(codes(&findings).contains(&"oas-operation-missing-operationId"));
    // A missing input is a finding, so the other inputs still report.
    let (status, findings, _) = json(root, &["check", "missing.yaml", "clean.yaml"]);
    assert_eq!(status, 1);
    assert_eq!(codes(&findings), ["io-error"], "{findings:#?}");
    // Usage and configuration errors are 2.
    assert_eq!(
        suspect(root, &["check", "clean.yaml", "--only", "nope"])
            .status
            .code(),
        Some(2)
    );
    std::fs::write(
        root.join(".suspect.yaml"),
        "lint:\n  rules:\n    info-contact: fatal\n",
    )
    .unwrap();
    assert_eq!(
        suspect(root, &["check", "clean.yaml"]).status.code(),
        Some(2)
    );
    std::fs::write(root.join(".suspect.yaml"), "lint:\n  ruleset: nope.yaml\n").unwrap();
    assert_eq!(
        suspect(root, &["check", "clean.yaml"]).status.code(),
        Some(2)
    );
}

#[test]
fn validate_and_lint_are_filters_of_check() {
    let repo = repo();
    let spec = "docs/demo/petstore.yaml";
    let (_, all, _) = json(&repo, &["check", spec]);
    let (_, validate, _) = json(&repo, &["validate", spec]);
    let (_, lint, _) = json(&repo, &["lint", spec]);
    // The union, with the one problem both report (a missing operationId,
    // validate's code and lint's alias) counted once, as `check` does.
    let union: std::collections::BTreeSet<_> = located(&validate)
        .into_iter()
        .chain(located(&lint))
        .collect();
    let all_set: std::collections::BTreeSet<_> = located(&all).into_iter().collect();
    assert_eq!(union, all_set, "check = validate + lint on petstore");
    assert_eq!(all_set.len(), all.len(), "check reports each problem once");
    assert_eq!(checks(&validate), ["validate"]);
    assert_eq!(checks(&lint), ["lint"]);
    // `--only` spells the same filters.
    let (_, only, _) = json(&repo, &["check", spec, "--only", "lint,markdown,rules"]);
    assert_eq!(located(&only), located(&lint));
    let (_, only, _) = json(
        &repo,
        &[
            "check",
            spec,
            "--only",
            "syntax,ref,validate,swagger,arazzo,overlay,load-profile,config",
            "--only",
            "extension",
        ],
    );
    assert_eq!(located(&only), located(&validate));
    // One finding per operation for a missing operationId, everywhere.
    for findings in [&all, &validate, &lint] {
        let missing = codes(findings)
            .into_iter()
            .filter(|c| c.contains("operationId"))
            .collect::<Vec<_>>();
        assert_eq!(missing, ["oas-operation-missing-operationId"; 2]);
    }
    // The lint finding carries its old name for one release.
    let renamed: Vec<&Value> = lint
        .iter()
        .filter(|f| f["code"] == "oas-operation-missing-operationId")
        .collect();
    assert!(
        renamed
            .iter()
            .all(|f| f["alias_of"] == "operation-operationId")
    );
    // No SDK category without codegen configured.
    assert!(!codes(&all).contains(&"sdk-operation-missing-id"));
}

#[test]
fn parse_only_runs_syntax_and_ref() {
    let dir = project(&[
        (
            "api.yaml",
            "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a/{id}:\n    get:\n      responses: {'200': {description: ok}}\ncomponents:\n  schemas:\n    A: {$ref: '#/components/schemas/Missing'}\n",
        ),
        ("bad.yaml", "openapi: 3.1.0\npaths: [1, 2\n  bad: : :\n"),
    ]);
    let (status, findings, _) = json(dir.path(), &["check", "--parse-only", "api.yaml"]);
    assert_eq!(status, 1, "{findings:#?}");
    assert_eq!(checks(&findings), ["ref"], "{findings:#?}");
    let (status, findings, _) = json(dir.path(), &["check", "--parse-only", "bad.yaml"]);
    assert_eq!(status, 1);
    assert!(checks(&findings).contains(&"syntax"), "{findings:#?}");
    assert!(
        suspect(
            dir.path(),
            &["check", "--parse-only", "--only", "lint", "api.yaml"]
        )
        .status
        .code()
            == Some(2),
        "--parse-only conflicts with --only"
    );
}

#[test]
fn the_cli_runs_the_checks_only_the_editor_ran() {
    let fixtures = repo().join("crates/suspect-check/tests/fixtures");
    for (name, check) in [
        ("broken.swagger.yaml", "swagger"),
        ("broken.arazzo.yaml", "arazzo"),
        ("empty.overlay.yaml", "overlay"),
        ("broken.load.yaml", "load-profile"),
    ] {
        let (_, findings, _) = json(&fixtures, &["check", name]);
        assert!(
            checks(&findings).contains(&check),
            "{name}: {check} findings expected: {findings:#?}"
        );
    }
    let dir = project(&[(
        "api.yaml",
        "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\n  description: See https://example.com/docs\npaths: {}\n",
    )]);
    let (_, findings, _) = json(dir.path(), &["check", "api.yaml"]);
    let bare = findings
        .iter()
        .find(|f| f["code"] == "md-bare-url")
        .expect("markdown in the CLI");
    assert_eq!(bare["severity"], "hint");
}

#[test]
fn sdk_findings_follow_codegen_and_the_flags() {
    let spec = "openapi: 3.1.0\ninfo: {title: T, version: '1'}\nservers: [{url: 'https://api.example.com'}]\npaths:\n  /a:\n    get:\n      responses: {'200': {description: ok}}\n";
    let dir = project(&[("api.yaml", spec)]);
    let root = dir.path();
    let sdk = |args: &[&str]| {
        let (_, findings, _) = json(root, args);
        codes(&findings).contains(&"sdk-operation-missing-id")
    };
    assert!(!sdk(&["check", "api.yaml"]));
    assert!(sdk(&["check", "api.yaml", "--sdk"]));
    std::fs::write(
        root.join(".suspect.yaml"),
        "codegen:\n  profile: typescript-http\n",
    )
    .unwrap();
    assert!(sdk(&["check", "api.yaml"]), "codegen configured");
    assert!(!sdk(&["check", "api.yaml", "--no-sdk"]));
    // `validate` runs no admission and keeps the readiness code only
    // with codegen configured.
    let (_, findings, _) = json(root, &["validate", "api.yaml"]);
    assert!(codes(&findings).contains(&"sdk-operation-missing-id"));
    assert!(checks(&findings).iter().all(|c| *c != "admission"));
}

#[test]
fn text_output_shows_fix_guidance_and_counts_suppressions_on_stderr() {
    let dir = project(&[(
        "api.yaml",
        "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a:\n    # suspect-disable oas-operation-missing-operationId -- owned elsewhere\n    get:\n      responses: {'200': {description: ok}}\n  /b:\n    get:\n      responses: {'200': {description: ok}}\n",
    )]);
    let out = suspect(dir.path(), &["check", "api.yaml", "--only", "validate"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let missing: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("oas-operation-missing-operationId"))
        .collect();
    assert_eq!(missing.len(), 1, "{stdout}");
    assert!(missing[0].starts_with("api.yaml:10:"), "{stdout}");
    assert!(
        stdout.contains("\nhow to fix: Add an `operationId`"),
        "{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("1 suppressed finding(s) hidden"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (_, _, report) = json(dir.path(), &["check", "api.yaml", "--only", "validate"]);
    assert_eq!(
        report["suppressed"][0]["code"],
        "oas-operation-missing-operationId"
    );
}

#[test]
fn sarif_and_junit_render_check_findings() {
    let repo = repo();
    let out = suspect(
        &repo,
        &["check", "docs/demo/petstore.yaml", "--format", "sarif"],
    );
    let log: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = log["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    for result in results {
        let region = &result["locations"][0]["physicalLocation"]["region"];
        assert!(region["endLine"].is_u64(), "{result}");
    }
    let rule = log["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "oas-path-param-not-declared")
        .unwrap();
    assert!(rule["help"]["text"].as_str().unwrap().contains("in: path"));
    let out = suspect(
        &repo,
        &["check", "docs/demo/petstore.yaml", "--format", "junit"],
    );
    let xml = String::from_utf8_lossy(&out.stdout);
    assert!(
        xml.contains("<testcase name=\"oas-path-param-not-declared\""),
        "{xml}"
    );
}

const RULE: &str = r#"
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

const COLLIDING: &str = r#"
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "oas-tag-undeclared", description: "collides" },
  given: r.operation,
  check(op, ctx) {
    ctx.report({ message: "never shown", at: op });
  },
});
"#;

const UNSUMMARIZED: &str = "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a:\n    get:\n      operationId: getA\n      responses: {'200': {description: ok}}\n";

#[test]
fn without_bun_typescript_rules_are_a_warning_not_a_failure() {
    let dir = project(&[
        (".suspect.yaml", "rules:\n  enabled: true\n"),
        (".suspect/rules/summary.ts", RULE),
        ("api.yaml", UNSUMMARIZED),
    ]);
    let no_bun = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_suspect"))
            .current_dir(dir.path())
            .args(args)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap()
    };
    let out = no_bun(&["check", "api.yaml", "--only", "rules", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["findings"][0]["code"], "rules-runtime-unavailable");
    assert_eq!(report["findings"][0]["severity"], "warning");
    // `rules check` still treats a missing runtime as an environment error.
    assert_eq!(
        no_bun(&["rules", "check", "api.yaml"]).status.code(),
        Some(2)
    );
    // Without `rules.enabled`, a filter that does not name `rules` skips them.
    std::fs::write(dir.path().join(".suspect.yaml"), "{}\n").unwrap();
    let out = no_bun(&["check", "api.yaml", "--format", "json"]);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["code"] != "rules-runtime-unavailable")
    );
}

fn bun() -> bool {
    if suspect_rules::discover::find_bun().is_some() {
        return true;
    }
    suspect_codegen::toolchain::skip("bun", "bun not on PATH — install from https://bun.sh");
    false
}

#[test]
fn typescript_rules_run_in_check_and_collisions_exit_2() {
    if !bun() {
        return;
    }
    let dir = project(&[
        (".suspect.yaml", "rules:\n  enabled: true\n"),
        (".suspect/rules/summary.ts", RULE),
        ("api.yaml", UNSUMMARIZED),
    ]);
    let (status, findings, _) = json(dir.path(), &["check", "api.yaml", "--only", "rules"]);
    assert_eq!(status, 0, "{findings:#?}");
    assert_eq!(codes(&findings), ["team-operation-summary"]);
    assert_eq!(findings[0]["check"], "rules");
    // `lint` includes them; `validate` does not.
    let (_, findings, _) = json(dir.path(), &["lint", "api.yaml"]);
    assert!(codes(&findings).contains(&"team-operation-summary"));
    let (_, findings, _) = json(dir.path(), &["validate", "api.yaml"]);
    assert!(!codes(&findings).contains(&"team-operation-summary"));

    std::fs::write(dir.path().join(".suspect/rules/collide.ts"), COLLIDING).unwrap();
    let out = suspect(dir.path(), &["check", "api.yaml"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("`oas-tag-undeclared`"), "{stderr}");
}

#[test]
fn stats_carries_the_census_check_used_to_print() {
    let out = suspect(
        &repo(),
        &["stats", "docs/demo/petstore.yaml", "--format", "json"],
    );
    let stats: Value = serde_json::from_slice(&out.stdout).unwrap();
    for key in [
        "ref_edges",
        "legal_cycles",
        "illegal_cycles",
        "workspace_docs",
        "workspace_edges",
    ] {
        assert!(stats[key].is_u64(), "{key}: {stats}");
    }
    assert_eq!(stats["workspace_docs"], 1);
}
