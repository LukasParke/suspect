//! `suspect config --explain`: every setting with its value and origin,
//! discovered from the input rather than the working directory.

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

/// A project with policy in both files, and a sibling directory with its
/// own settings file (the working directory in these tests).
fn fixture(root: &Path) {
    write(
        root,
        "proj/suspect.project.json",
        "{\n  \"version\": 1,\n  \"entry\": \"openapi.yaml\",\n  \"lint\": {\n    \"rules\": {\"info-contact\": \"off\", \"info-license\": \"error\"}\n  }\n}\n",
    );
    write(
        root,
        "proj/.suspect.yaml",
        "lint:\n  min_severity: warning\n  rules:\n    info-contact: error\n",
    );
    write(
        root,
        "proj/openapi.yaml",
        "openapi: 3.1.0\ninfo: {title: t, version: '1'}\npaths: {}\n",
    );
    write(
        root,
        "other/.suspect.yaml",
        "lint:\n  min_severity: error\n",
    );
}

fn explain(root: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(binary())
        .arg("config")
        .args(args)
        .current_dir(root.join("other"))
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .env_remove("SUSPECT_LINT_RULESET")
        .env_remove("SUSPECT_STRICT_FORMAT")
        .env("SUSPECT_BASE_URL", "http://preview.test")
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn line<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find(|l| l.split_whitespace().next() == Some(key))
        .unwrap_or_else(|| panic!("no {key} row in:\n{text}"))
}

#[test]
fn explain_names_every_origin_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (text, code) = explain(dir.path(), &["--explain", "../proj/openapi.yaml"]);
    assert_eq!(code, 0, "{text}");
    // The input's project governs, not the working directory's file.
    assert!(text.starts_with("root     "), "{text}");
    assert!(line(&text, "root").contains("proj"), "{text}");
    let floor = line(&text, "lint.min_severity");
    assert!(floor.contains("warning"), "{floor}");
    assert!(floor.contains("proj/.suspect.yaml:2"), "{floor}");
    let contact = line(&text, "lint.rules.info-contact");
    assert!(contact.contains("proj/.suspect.yaml:4"), "{contact}");
    assert!(
        contact.contains("over ") && contact.contains("suspect.project.json:5 = off"),
        "{contact}"
    );
    let license = line(&text, "lint.rules.info-license");
    assert!(license.contains("suspect.project.json:5"), "{license}");
    assert!(license.contains("deprecated location"), "{license}");
    assert!(line(&text, "test.base_url").contains("env SUSPECT_BASE_URL"));
    assert!(line(&text, "validate.strict_format").contains("default"));
    assert!(text.contains("debug variables (not settings)"), "{text}");
    assert!(line(&text, "SUSPECT_PROFILE").contains("phase timings"));
}

#[test]
fn set_only_hides_defaults() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (text, _) = explain(
        dir.path(),
        &["--explain", "--set-only", "../proj/openapi.yaml"],
    );
    assert!(text.contains("lint.min_severity"), "{text}");
    assert!(!text.contains("validate.strict_format"), "{text}");
}

#[test]
fn explain_json_has_values_with_origins_and_traces() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (text, code) = explain(
        dir.path(),
        &["--explain", "--format", "json", "../proj/openapi.yaml"],
    );
    assert_eq!(code, 0, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(json["files"].as_array().unwrap().len(), 2);
    let value = |key: &str| {
        json["values"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["key"] == key)
            .unwrap_or_else(|| panic!("{key} missing"))
            .clone()
    };
    let contact = value("lint.rules.info-contact");
    assert_eq!(contact["value"], "error");
    assert_eq!(contact["layer"], "file");
    assert!(
        contact["origin"]
            .as_str()
            .unwrap()
            .ends_with(".suspect.yaml:4")
    );
    assert_eq!(contact["trace"].as_array().unwrap().len(), 2);
    assert_eq!(value("test.base_url")["layer"], "env");
    assert_eq!(value("ref.max_docs")["value"], 500);
    assert!(json["debug"].as_array().unwrap().len() >= 6);
}

#[test]
fn without_an_input_the_working_directory_governs() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (text, _) = explain(dir.path(), &["--explain"]);
    let floor = line(&text, "lint.min_severity");
    assert!(floor.contains("error"), "{floor}");
    assert!(floor.contains(".suspect.yaml:2"), "{floor}");
}

#[test]
fn a_bad_value_is_shown_and_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    write(
        dir.path(),
        "proj/.suspect.yaml",
        "lint:\n  min_severity: fatal\n",
    );
    let (text, code) = explain(dir.path(), &["--explain", "../proj/openapi.yaml"]);
    assert_eq!(code, 2);
    assert!(
        text.contains("error: ") && text.contains(".suspect.yaml:2: `lint.min_severity`"),
        "{text}"
    );
    // Without --explain it is a usage error.
    let out = Command::new(binary())
        .args(["config", "../proj/openapi.yaml"])
        .current_dir(dir.path().join("other"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("`fatal`"));
}

/// The settings that replaced flag defaults: a flag beats the file, the
/// environment sits between them.
#[test]
fn flags_beat_env_beat_the_new_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".suspect.yaml",
        "test:\n  credentials: from-file.json\ngateway:\n  port: 9999\ndiff:\n  baseline: origin/main\n",
    );
    for name in ["from-file.json", "from-env.json", "from-flag.json"] {
        write(root, name, "{\"auth\": {}}\n");
    }
    let auth = |env: Option<&str>, flag: Option<&str>| {
        let mut cmd = Command::new(binary());
        cmd.args(["auth", "check"]).current_dir(root);
        if let Some(flag) = flag {
            cmd.args(["--credentials", flag]);
        }
        match env {
            Some(env) => cmd.env("SUSPECT_CREDENTIALS", env),
            None => cmd.env_remove("SUSPECT_CREDENTIALS"),
        };
        String::from_utf8_lossy(&cmd.output().unwrap().stdout).into_owned()
    };
    assert!(auth(None, None).contains("from-file.json"));
    assert!(auth(Some("from-env.json"), None).contains("from-env.json"));
    assert!(auth(Some("from-env.json"), Some("from-flag.json")).contains("from-flag.json"));

    let out = Command::new(binary())
        .args(["config", "--explain"])
        .current_dir(root)
        .env("SUSPECT_GIT_BASE", "HEAD~3")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        line(&text, "gateway.port").contains(".suspect.yaml:4"),
        "{text}"
    );
    let baseline = line(&text, "diff.baseline");
    assert!(
        baseline.contains("HEAD~3") && baseline.contains("env SUSPECT_GIT_BASE"),
        "{baseline}"
    );
}
