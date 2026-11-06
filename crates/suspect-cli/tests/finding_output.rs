//! Golden output for `check`, `validate` and `lint` on
//! `docs/demo/petstore.yaml`.
//!
//! All three run the check pass and render through `suspect_diag::render`
//! (P1 step 5), so a byte of drift fails here. Regenerate after an
//! intended change with:
//!
//! ```text
//! SUSPECT_BLESS=1 cargo test -p suspect-cli --test finding_output
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn check(command: &str, format: &str, golden: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(workspace_root())
        .args([command, "docs/demo/petstore.yaml", "--format", format])
        .output()
        .expect("run suspect");
    assert_eq!(output.status.code(), Some(1), "{command} --format {format}");
    let actual = String::from_utf8(output.stdout).expect("UTF-8 output");
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(golden);
    if std::env::var_os("SUSPECT_BLESS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{command} --format {format} differs from {}; regenerate with \
         `SUSPECT_BLESS=1 cargo test -p suspect-cli --test finding_output`\n--- actual\n{actual}",
        path.display()
    );
}

#[test]
fn check_text() {
    check("check", "text", "petstore.check.txt");
}

#[test]
fn check_json() {
    check("check", "json", "petstore.check.json");
}

#[test]
fn check_sarif() {
    check("check", "sarif", "petstore.check.sarif");
}

#[test]
fn check_junit() {
    check("check", "junit", "petstore.check.junit.xml");
}

#[test]
fn validate_text() {
    check("validate", "text", "petstore.validate.txt");
}

#[test]
fn validate_json() {
    check("validate", "json", "petstore.validate.json");
}

#[test]
fn validate_sarif() {
    check("validate", "sarif", "petstore.validate.sarif");
}

#[test]
fn lint_text() {
    check("lint", "text", "petstore.lint.txt");
}

#[test]
fn lint_json() {
    check("lint", "json", "petstore.lint.json");
}

#[test]
fn lint_sarif() {
    check("lint", "sarif", "petstore.lint.sarif");
}

#[test]
fn validate_junit() {
    check("validate", "junit", "petstore.validate.junit.xml");
}
