//! Entry identity and explicit source discovery through real CLI commands.

use std::process::Command;

const API: &str = "openapi: 3.1.0
info: {title: Entry, version: '1'}
paths:
  /models:
    get:
      operationId: listModels
      responses:
        '200': {description: ok}
";

#[test]
#[cfg(unix)]
fn documentation_generation_accepts_a_relative_path_through_a_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let actual = directory.path().join("actual");
    std::fs::create_dir(&actual).unwrap();
    std::fs::write(actual.join("api.yaml"), API).unwrap();
    std::os::unix::fs::symlink(&actual, directory.path().join("alias")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(directory.path())
        .args([
            "gen",
            "alias/api.yaml",
            "--preset",
            "docs-md",
            "--out",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let operation =
        std::fs::read_to_string(directory.path().join("docs/docs/api/index.md")).unwrap();
    assert!(operation.contains("/models"), "{operation}");
}

#[test]
fn workflows_load_declared_sources_outside_the_entry_directory() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("apis/public")).unwrap();
    std::fs::create_dir(directory.path().join("flows")).unwrap();
    std::fs::write(directory.path().join("apis/public/api.yaml"), API).unwrap();
    std::fs::write(
        directory.path().join("flows/check.yaml"),
        "arazzo: 1.0.0
info: {title: Workflows, version: '1'}
sourceDescriptions:
  - name: api
    type: openapi
    url: ../apis/public/api.yaml
workflows:
  - workflowId: list
    steps:
      - stepId: models
        operationId: listModels
",
    )
    .unwrap();
    // Filtering occurs after plan compilation and avoids any network request.
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(directory.path())
        .args([
            "test",
            "flows/check.yaml",
            "--filter",
            "no-workflow-matches",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no workflows matched"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
