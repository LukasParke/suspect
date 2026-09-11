//! CLI source coordinates must be directly usable by editors and PR annotations.

use std::process::Command;

#[test]
fn json_lint_findings_use_one_based_source_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("api.yaml");
    std::fs::write(
        &source,
        "openapi: 3.1.0\ninfo:\n  title: Lint coordinates\n  version: '1'\npaths: {}\ncomponents:\n  schemas:\n    Count:\n      type: integer\n      enum: [wrong]\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .arg("lint")
        .arg(&source)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout).unwrap();
    let finding = findings
        .iter()
        .find(|finding| finding["code"] == "typed-enum")
        .expect("invalid enum member must be diagnosed");
    assert_eq!(finding["file"], source.to_str().unwrap());
    assert_eq!(finding["line"], 10);
    assert_eq!(finding["col"], 14);
}
