//! Option A: lint policy lives in `.suspect.yaml`. A manifest that still
//! carries it is warned about, `suspect config migrate` moves it, and
//! `project init` writes the settings file.

use std::path::Path;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_suspect")
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .current_dir(cwd)
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .env_remove("SUSPECT_LINT_RULESET")
        .env_remove("SUSPECT_STRICT_FORMAT")
        .output()
        .unwrap()
}

const SPEC: &str = "openapi: 3.1.0\ninfo: {title: t, version: '1'}\npaths: {}\n";

/// `(key, value, layer)` rows of `suspect config --explain --format json`.
fn explain(cwd: &Path) -> Vec<(String, serde_json::Value, String)> {
    let out = run(cwd, &["config", "--explain", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    json["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["key"].as_str().unwrap().to_owned(),
                v["value"].clone(),
                v["layer"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn value(rows: &[(String, serde_json::Value, String)], key: &str) -> (serde_json::Value, String) {
    rows.iter()
        .find(|(k, _, _)| k == key)
        .map(|(_, v, l)| (v.clone(), l.clone()))
        .unwrap_or_else(|| panic!("{key} missing"))
}

#[test]
fn manifest_policy_is_warned_about_once_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "suspect.project.json",
        "{\n  \"version\": 1,\n  \"entry\": \"openapi.yaml\",\n  \"lint\": {}\n}\n",
    );
    write(root, "openapi.yaml", SPEC);
    let out = run(
        root,
        &["lint", "openapi.yaml", "openapi.yaml", "--format", "json"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected = "warning: suspect.project.json:4: `lint` in a project manifest is deprecated \
                    and is ignored from the next release; move it to .suspect.yaml and add \
                    \"config\": \".suspect.yaml\" to the manifest (`suspect config migrate` does both)";
    assert_eq!(stderr.matches(expected).count(), 1, "{stderr}");
    // Never on stdout: the report keeps its shape.
    assert!(serde_json::from_slice::<serde_json::Value>(&out.stdout).is_ok());

    // `project check` reports it as a warning finding, and still passes.
    let check = run(root, &["project", "check"]);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(stderr.contains("project-manifest-policy"), "{stderr}");
    assert!(check.status.success(), "{stderr}");
}

#[test]
fn migrate_creates_a_settings_file_when_none_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "suspect.project.json",
        "{\n  \"version\": 1,\n  \"entry\": \"openapi.yaml\",\n  \"lint\": {\"min_severity\": \"warning\", \"rules\": {\"info-contact\": \"off\"}},\n  \"validate\": {\"strict_format\": true},\n  \"editor\": {\"ref\": {\"max_docs\": 9}, \"colors\": true}\n}\n",
    );
    let before = explain(root);
    let out = run(root, &["config", "migrate"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("created .suspect.yaml"), "{stdout}");
    assert!(stdout.contains("editor.colors: dropped"), "{stdout}");
    assert_eq!(
        read(root, "suspect.project.json"),
        "{\n  \"config\": \".suspect.yaml\",\n  \"version\": 1,\n  \"entry\": \"openapi.yaml\"\n}\n"
    );
    let after = explain(root);
    for key in [
        "lint.min_severity",
        "lint.rules.info-contact",
        "validate.strict_format",
        "ref.max_docs",
    ] {
        assert_eq!(value(&before, key).0, value(&after, key).0, "{key}");
        assert_eq!(value(&before, key).1, "manifest", "{key}");
        assert_eq!(value(&after, key).1, "file", "{key}");
    }
    // Nothing left to warn about.
    let lint = run(root, &["config"]);
    assert!(!String::from_utf8_lossy(&lint.stderr).contains("deprecated"));
}

#[test]
fn migrate_keeps_the_files_values_and_its_comments() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "suspect.project.json",
        r#"{"version": 1, "entry": "openapi.yaml", "lint": {"min_severity": "info", "design": "off", "rules": {"info-contact": "off", "info-license": "error", "operation-tags": "off"}}}"#,
    );
    write(
        root,
        ".suspect.yaml",
        "# keep me\nlint:\n  # floor\n  min_severity: warning\n  rules:\n    info-contact: error\ndocs:\n  style: html\n",
    );
    let before = explain(root);
    let out = run(root, &["config", "migrate"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("lint.min_severity: kept"), "{stdout}");
    assert_eq!(
        read(root, ".suspect.yaml"),
        "# keep me\nlint:\n  # floor\n  min_severity: warning\n  rules:\n    info-contact: error\n    info-license: error\n    operation-tags: \"off\"\n  design: \"off\"\ndocs:\n  style: html\n"
    );
    let after = explain(root);
    for key in [
        "lint.min_severity",
        "lint.design",
        "lint.rules.info-contact",
        "lint.rules.info-license",
    ] {
        assert_eq!(value(&before, key).0, value(&after, key).0, "{key}");
    }
}

#[test]
fn migrate_under_a_shared_file_extends_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, ".suspect.yaml", "lint:\n  min_severity: error\n");
    write(
        root,
        "svc/suspect.project.json",
        "{\n  \"version\": 1,\n  \"entry\": \"openapi.yaml\",\n  \"lint\": {\"rules\": {\"info-license\": \"off\"}}\n}\n",
    );
    let out = run(root, &["config", "migrate", "svc/suspect.project.json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        read(root, "svc/.suspect.yaml")
            .contains("extends: ../.suspect.yaml\nlint:\n  rules:\n    info-license: \"off\"\n")
    );
    let rows = explain(&root.join("svc"));
    assert_eq!(
        value(&rows, "lint.min_severity").0,
        "error",
        "the shared file still applies"
    );
    assert_eq!(value(&rows, "lint.rules.info-license").0, "off");
}

#[test]
fn project_init_writes_the_settings_file_and_points_at_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = run(root, &["project", "init", "api"]);
    assert!(out.status.success());
    let manifest: serde_json::Value =
        serde_json::from_str(&read(&root.join("api"), "suspect.project.json")).unwrap();
    assert_eq!(manifest["config"], ".suspect.yaml");
    for section in ["lint", "validate", "editor"] {
        assert!(
            manifest.get(section).is_none(),
            "{section} left in the manifest"
        );
    }
    let rows = explain(&root.join("api"));
    assert_eq!(
        value(&rows, "lint.min_severity"),
        ("hint".into(), "file".into())
    );
    assert_eq!(value(&rows, "ref.max_docs"), (500.into(), "file".into()));
    // A fresh project is warning-free.
    write(&root.join("api"), "openapi.yaml", SPEC);
    let lint = run(&root.join("api"), &["lint", "openapi.yaml"]);
    assert!(!String::from_utf8_lossy(&lint.stderr).contains("warning:"));
}
