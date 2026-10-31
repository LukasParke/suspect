//! Old flat command names keep working through the argv rewrite: stdout
//! and the exit status are the old ones, plus one deprecation line on
//! stderr (docs/proposals/P3-command-grammar.md, "Legacy dispatch").

use std::path::Path;
use std::process::{Command, Output};

const OLD: &str = r#"openapi: 3.1.0
info: {title: Pets, version: "1"}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        "200": {description: ok}
  /pets/{id}:
    get:
      operationId: getPet
      parameters:
        - {name: id, in: path, required: true, schema: {type: string}}
      responses:
        "200": {description: ok}
"#;

const NEW: &str = r#"openapi: 3.1.0
info: {title: Pets, version: "2"}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        "200": {description: ok}
"#;

fn suspect(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn suspect")
}

fn fixtures() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("old.yaml"), OLD).unwrap();
    std::fs::write(dir.path().join("new.yaml"), NEW).unwrap();
    dir
}

#[test]
fn old_breaking_prints_json_on_stdout_and_the_note_on_stderr() {
    let dir = fixtures();
    let old = suspect(
        dir.path(),
        &["breaking", "old.yaml", "new.yaml", "--format", "json"],
    );
    let new = suspect(
        dir.path(),
        &[
            "diff", "breaking", "old.yaml", "new.yaml", "--format", "json",
        ],
    );
    let stdout = String::from_utf8(old.stdout.clone()).unwrap();
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("JSON stdout");
    assert!(
        report.as_array().is_some_and(|breaks| !breaks.is_empty()),
        "{stdout}"
    );
    // A removed path is a break: exit 1, as `breaking` always did.
    assert_eq!(old.status.code(), Some(1));
    assert_eq!(old.status.code(), new.status.code());
    assert_eq!(old.stdout, new.stdout);
    let stderr = String::from_utf8(old.stderr).unwrap();
    assert!(
        stderr
            .starts_with("warning: `suspect breaking` is deprecated; use `suspect diff breaking`"),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        new.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&new.stderr)
    );
}

#[test]
fn old_diff_matches_diff_structural() {
    let dir = fixtures();
    let old = suspect(dir.path(), &["diff", "old.yaml", "new.yaml"]);
    let new = suspect(dir.path(), &["diff", "structural", "old.yaml", "new.yaml"]);
    assert!(!old.stdout.is_empty());
    assert_eq!(old.stdout, new.stdout);
    assert_eq!(old.status.code(), Some(0));
    assert_eq!(old.status.code(), new.status.code());
    assert!(String::from_utf8_lossy(&old.stderr).contains("use `suspect diff structural"));
}

#[test]
fn diff_breaking_dispatches_arazzo_documents_to_the_arazzo_differ() {
    let dir = tempfile::tempdir().unwrap();
    let doc = |workflows: &str| {
        format!(
            "arazzo: 1.0.1\ninfo: {{title: t, version: '1'}}\n\
             sourceDescriptions: [{{name: api, url: ./api.yaml, type: openapi}}]\n\
             workflows:\n{workflows}"
        )
    };
    let step = "    steps: [{stepId: s, operationId: op}]\n";
    std::fs::write(
        dir.path().join("old.arazzo.yaml"),
        doc(&format!(
            "  - workflowId: a\n{step}  - workflowId: b\n{step}"
        )),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("new.arazzo.yaml"),
        doc(&format!("  - workflowId: a\n{step}")),
    )
    .unwrap();
    let args = ["old.arazzo.yaml", "new.arazzo.yaml", "--format", "json"];
    let old = suspect(dir.path(), &[&["arazzo-diff"][..], &args].concat());
    let new = suspect(dir.path(), &[&["diff", "breaking"][..], &args].concat());
    assert_eq!(old.status.code(), Some(1));
    assert_eq!(old.status.code(), new.status.code());
    assert_eq!(old.stdout, new.stdout);
    let stdout = String::from_utf8(new.stdout).unwrap();
    assert!(stdout.contains("arazzo-breaking-change"), "{stdout}");
    assert!(stdout.contains("workflow 'b' removed"), "{stdout}");
}

#[test]
fn group_help_and_a_bare_group_exit_like_clap() {
    let dir = fixtures();
    let help = suspect(dir.path(), &["diff", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stderr.is_empty());
    let bare = suspect(dir.path(), &["test"]);
    assert_eq!(bare.status.code(), Some(2));
}
