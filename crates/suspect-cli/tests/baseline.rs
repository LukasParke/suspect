//! Findings baselines: `check --write-baseline`, `--baseline` and
//! `check.baseline`. A baselined finding is keyed by code, document and
//! JSON pointer, so moving it keeps it baselined and renaming its path
//! does not.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const SPEC: &str = "openapi: 3.1.0
info: {title: T, version: '1'}
paths:
  /pets:
    get:
      responses: {'200': {description: ok}}
  /pets/{petId}:
    get:
      responses: {'200': {description: ok}}
";

fn suspect(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(cwd)
        .args(args)
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .output()
        .expect("run suspect")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A project with the spec and a baseline written from it.
fn baselined() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api.yaml"), SPEC).unwrap();
    let out = suspect(dir.path(), &["check", "api.yaml"]);
    assert_eq!(out.status.code(), Some(1), "the spec has an error");
    let out = suspect(
        dir.path(),
        &[
            "check",
            "api.yaml",
            "--write-baseline",
            ".suspect/baseline.json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("finding(s) to .suspect/baseline.json"));
    dir
}

#[test]
fn a_written_baseline_hides_every_current_finding() {
    let dir = baselined();
    let text = std::fs::read_to_string(dir.path().join(".suspect/baseline.json")).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["version"], 1);
    assert_eq!(file["tool"], "suspect");
    let entries = file["findings"].as_array().unwrap();
    // Documents are relative to the baseline file.
    let undeclared = entries
        .iter()
        .find(|e| e["code"] == "oas-path-param-not-declared")
        .expect("the error is recorded");
    assert_eq!(undeclared["document"], "../api.yaml");
    assert_eq!(undeclared["pointer"], "/paths/~1pets~1{petId}");
    assert_eq!(undeclared["count"], 1);
    // Sorted by document, pointer, code.
    let keys: Vec<(String, String, String)> = entries
        .iter()
        .map(|e| {
            (
                e["document"].as_str().unwrap().to_owned(),
                e["pointer"].as_str().unwrap().to_owned(),
                e["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(text.ends_with('\n'));

    let out = suspect(
        dir.path(),
        &["check", "api.yaml", "--baseline", ".suspect/baseline.json"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("baselined finding(s) hidden"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_new_finding_fails_and_is_the_only_one_shown() {
    let dir = baselined();
    std::fs::write(
        dir.path().join("api.yaml"),
        format!("{SPEC}  /owners/{{ownerId}}:\n    get:\n      operationId: getOwner\n      responses: {{'200': {{description: ok}}}}\n"),
    )
    .unwrap();
    let out = suspect(
        dir.path(),
        &[
            "check",
            "api.yaml",
            "--baseline",
            ".suspect/baseline.json",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let findings = report["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    for f in findings {
        assert!(
            f["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/paths/~1owners~1{ownerId}"),
            "only the new path's findings: {f}"
        );
    }
    assert!(report["baselined"].as_u64().unwrap() > 0);
}

#[test]
fn moving_an_operation_keeps_it_baselined_and_renaming_its_path_does_not() {
    let dir = baselined();
    // The two path items swap places, and every line moves.
    let moved = "openapi: 3.1.0\ninfo: {title: T, version: '1'}\n\npaths:\n  /pets/{petId}:\n    get:\n      responses: {'200': {description: ok}}\n\n  /pets:\n    get:\n      responses: {'200': {description: ok}}\n";
    std::fs::write(dir.path().join("api.yaml"), moved).unwrap();
    let out = suspect(
        dir.path(),
        &["check", "api.yaml", "--baseline", ".suspect/baseline.json"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(stdout(&out), "");

    let renamed = moved.replace("/pets/{petId}", "/animals/{petId}");
    std::fs::write(dir.path().join("api.yaml"), renamed).unwrap();
    let out = suspect(
        dir.path(),
        &["check", "api.yaml", "--baseline", ".suspect/baseline.json"],
    );
    assert_eq!(out.status.code(), Some(1));
    let shown = stdout(&out);
    assert!(shown.contains("oas-path-param-not-declared"), "{shown}");
    assert!(
        !shown.contains("/pets`"),
        "the unchanged path stays hidden: {shown}"
    );
}

#[test]
fn check_baseline_in_settings_applies_to_check_validate_lint_and_ci() {
    let dir = baselined();
    std::fs::write(
        dir.path().join(".suspect.yaml"),
        "check:\n  baseline: .suspect/baseline.json\n",
    )
    .unwrap();
    for command in ["check", "validate", "lint"] {
        let out = suspect(dir.path(), &[command, "api.yaml"]);
        assert_eq!(out.status.code(), Some(0), "{command}: {}", stdout(&out));
    }
    std::fs::write(
        dir.path().join("suspect.project.json"),
        r#"{"version": 1, "name": "p", "entry": "api.yaml"}"#,
    )
    .unwrap();
    let out = suspect(
        dir.path(),
        &["ci", ".", "--stage", "check", "--format", "json"],
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let stage = &report["projects"][0]["stages"][0];
    assert_eq!(stage["passed"], true, "{stage}");
    assert!(
        stage["summary"].as_str().unwrap().contains("baselined"),
        "{stage}"
    );
}

#[test]
fn sarif_marks_baseline_state_and_fingerprints() {
    let dir = baselined();
    std::fs::write(
        dir.path().join("api.yaml"),
        SPEC.replace("/pets:\n", "/cats:\n"),
    )
    .unwrap();
    let out = suspect(
        dir.path(),
        &[
            "check",
            "api.yaml",
            "--baseline",
            ".suspect/baseline.json",
            "--format",
            "sarif",
        ],
    );
    let log: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = log["runs"][0]["results"].as_array().unwrap();
    let state = |s: &str| results.iter().filter(|r| r["baselineState"] == s).count();
    assert!(state("unchanged") > 0, "{log}");
    assert!(state("new") > 0, "{log}");
    assert_eq!(state("unchanged") + state("new"), results.len());
    let fingerprints: std::collections::HashSet<&str> = results
        .iter()
        .map(|r| r["partialFingerprints"]["suspect/v1"].as_str().unwrap())
        .collect();
    assert!(fingerprints.iter().all(|f| f.len() == 64));
}

#[test]
fn an_unreadable_or_malformed_baseline_exits_2() {
    let dir = baselined();
    let out = suspect(
        dir.path(),
        &["check", "api.yaml", "--baseline", "missing.json"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("cannot read baseline"),
        "{}",
        stderr(&out)
    );
    std::fs::write(dir.path().join("bad.json"), "[1, 2]").unwrap();
    let out = suspect(dir.path(), &["check", "api.yaml", "--baseline", "bad.json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("is not a suspect findings baseline"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        suspect(
            dir.path(),
            &[
                "check",
                "api.yaml",
                "--baseline",
                "a.json",
                "--write-baseline",
                "b.json"
            ]
        )
        .status
        .code(),
        Some(2),
        "the two flags conflict"
    );
}
