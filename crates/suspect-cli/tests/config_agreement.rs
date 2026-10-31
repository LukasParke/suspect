//! `suspect lint` and `suspect ci` apply the same settings to the same
//! file: one discovery rule, one precedence, per-key merging of
//! `lint.rules`, wherever they are run from.

use std::path::Path;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_suspect")
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The Problem 1 reproduction: `lint.rules` in both files, a spec with no
/// `contact` or `license`, and a sibling directory with its own floor.
fn fixture(root: &Path) {
    write(
        root,
        "proj/suspect.project.json",
        r#"{
  "version": 1,
  "name": "proj",
  "entry": "openapi.yaml",
  "lint": {"min_severity": "hint", "rules": {"info-contact": "off", "info-license": "error"}}
}
"#,
    );
    write(
        root,
        "proj/.suspect.yaml",
        "lint:\n  rules:\n    info-contact: error\n",
    );
    write(
        root,
        "proj/openapi.yaml",
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: ok\n",
    );
    write(
        root,
        "other/.suspect.yaml",
        "lint:\n  min_severity: error\n",
    );
}

fn command(cwd: &Path) -> Command {
    let mut cmd = Command::new(binary());
    cmd.current_dir(cwd)
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .env_remove("SUSPECT_LINT_RULESET")
        .env_remove("SUSPECT_STRICT_FORMAT");
    cmd
}

/// `(code, severity)` for every `suspect lint --format json` finding
/// (`findings` of the report; severities are lowercase).
fn lint(cwd: &Path, spec: &str) -> Vec<(String, String)> {
    let out = command(cwd)
        .args(["lint", spec, "--format", "json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    let mut pairs: Vec<(String, String)> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["code"].as_str().unwrap().to_owned(),
                f["severity"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    pairs.sort();
    pairs
}

/// The lint stage's `(errors, warnings)` from `suspect ci --format json`.
fn ci_lint(root: &Path) -> (u64, u64, serde_json::Value) {
    let out = command(root)
        .args(["ci", ".", "--stage", "lint", "--format", "json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    let stage = report["projects"][0]["stages"][0].clone();
    (
        stage["errors"].as_u64().unwrap(),
        stage["warnings"].as_u64().unwrap(),
        stage,
    )
}

fn severity_of<'a>(findings: &'a [(String, String)], code: &str) -> Option<&'a str> {
    findings
        .iter()
        .find(|(c, _)| c == code)
        .map(|(_, s)| s.as_str())
}

#[test]
fn lint_and_ci_agree_on_policy_from_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let findings = lint(&root.join("proj"), "openapi.yaml");
    // The settings file wins per key; the manifest fills the rest.
    assert_eq!(severity_of(&findings, "info-contact"), Some("error"));
    assert_eq!(severity_of(&findings, "info-license"), Some("error"));

    let errors = findings.iter().filter(|(_, s)| s == "error").count() as u64;
    let warnings = findings.len() as u64 - errors;
    let (ci_errors, ci_warnings, stage) = ci_lint(root);
    assert_eq!(
        (ci_errors, ci_warnings),
        (errors, warnings),
        "ci and lint disagree: {stage} vs {findings:?}"
    );
}

#[test]
fn the_input_governs_wherever_lint_runs_from() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let inside = lint(&root.join("proj"), "openapi.yaml");
    let beside = lint(&root.join("other"), "../proj/openapi.yaml");
    assert_eq!(inside, beside, "the sibling's floor must not apply");
    assert!(inside.iter().any(|(_, s)| s != "error"), "{inside:?}");
}

#[test]
fn an_unknown_severity_fails_lint_and_the_ci_stage() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    write(
        root,
        "proj/.suspect.yaml",
        "lint:\n  rules:\n    info-contact: fatal\n",
    );
    let out = command(&root.join("proj"))
        .args(["lint", "openapi.yaml"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr
            .contains(".suspect.yaml:3: `lint.rules.info-contact`: unrecognized severity `fatal`"),
        "{stderr}"
    );

    let (errors, _, stage) = ci_lint(root);
    assert_eq!(stage["passed"], false, "{stage}");
    assert!(errors >= 1);
    assert!(
        stage["summary"]
            .as_str()
            .unwrap()
            .contains("unrecognized severity `fatal`"),
        "{stage}"
    );
}

#[test]
fn lint_recommended_false_runs_no_builtin_rules() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    write(root, "proj/.suspect.yaml", "lint:\n  recommended: false\n");
    // The manifest's per-rule entries have no rules left to apply to.
    assert!(lint(&root.join("proj"), "openapi.yaml").is_empty());
    let (errors, warnings, _) = ci_lint(root);
    assert_eq!((errors, warnings), (0, 0));
}

#[test]
fn ci_notes_values_the_new_order_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let out = command(root)
        .args(["ci", ".", "--stage", "lint"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("lint.rules.info-contact is now error (")
            && stderr.contains("earlier releases applied off from the manifest"),
        "{stderr}"
    );
    assert!(!stderr.contains("info-license is now"), "{stderr}");
}

#[allow(dead_code)]
mod support {
    pub mod editor;
}

/// The server, started at the parent of both directories (a monorepo
/// root), resolves the document's own project like `suspect lint` does.
#[test]
fn the_editor_reports_the_severities_lint_reports() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let findings = lint(&root.join("proj"), "openapi.yaml");
    let spec = root.join("proj/openapi.yaml");
    let editor = support::editor::Editor::start_bare(root);
    let uri = editor.open(&spec, &std::fs::read_to_string(&spec).unwrap());
    let report = editor
        .request(
            "textDocument/diagnostic",
            serde_json::json!({"textDocument": {"uri": uri}}),
        )
        .expect("pull answered");
    let names = ["", "error", "warning", "info", "hint"];
    for code in ["info-contact", "info-license"] {
        let item = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == code)
            .unwrap_or_else(|| panic!("{code} not reported: {report}"));
        let severity = names[item["severity"].as_u64().unwrap() as usize];
        assert_eq!(Some(severity), severity_of(&findings, code), "{code}");
    }
}
