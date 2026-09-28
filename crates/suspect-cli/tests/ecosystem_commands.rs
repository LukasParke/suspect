//! CLI contract tests: the upgrade, arazzo-diff, and overlay-dry-run
//! commands behave as documented.

use std::path::Path;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_suspect")
}

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ------------------------------------------------------------------ upgrade

#[test]
fn upgrade_converts_swagger_to_openapi31() {
    let dir = tempfile::tempdir().unwrap();
    let input = write(
        dir.path(),
        "swagger.yaml",
        r#"swagger: "2.0"
info: {title: Upgrade Suite, version: '1'}
host: api.suite.test
basePath: /v1
schemes: [https]
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: page
          schema: {$ref: '#/definitions/Pet'}
definitions:
  Pet:
    type: object
    required: [name]
    properties:
      name: {type: string}
"#,
    );
    let output = dir.path().join("upgraded.yaml");
    let status = std::process::Command::new(binary())
        .args([
            "upgrade",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "upgrade failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let text = std::fs::read_to_string(&output).unwrap();
    assert!(text.contains("\"openapi\": \"3.1.0\""));
    assert!(text.contains("#/components/schemas/Pet"));
    assert!(text.contains("servers"));
}

#[test]
fn upgrade_rejects_non_swagger_documents() {
    let dir = tempfile::tempdir().unwrap();
    let input = write(
        dir.path(),
        "already31.yaml",
        "openapi: 3.1.0\ninfo: {title: x, version: '1'}\npaths: {}\n",
    );
    let status = std::process::Command::new(binary())
        .args(["upgrade", input.to_str().unwrap(), "--output", "/dev/null"])
        .output()
        .unwrap();
    assert!(!status.status.success());
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(stderr.contains("not a Swagger 2.0 document"), "{stderr}");
}

// -------------------------------------------------------------- arazzo diff

/// One minimal Arazzo suite for diffing.
const SUITE_OLD: &str = r#"arazzo: 1.0.1
info: {title: Suite, version: '1'}
sourceDocuments:
  api: openapi.yaml
workflows:
  - workflowId: order-flow
    inputs:
      $values:
        petId: 1
    outputs:
      orderId: $steps.create.outputs.body.id
    steps:
      - stepId: create
        operationPath: 'api#/paths/~1pets/post'
        outputs:
          created: $response.body#/id
        successCriteria:
          - condition: $statusCode == 201
      - stepId: fetch
        operationPath: 'api#/paths/~1pets/{petId}/get'
        successCriteria:
          - condition: $statusCode == 200
"#;

const SUITE_NEW_BREAKING: &str = r#"arazzo: 1.0.1
info: {title: Suite, version: '1'}
sourceDocuments:
  api: openapi.yaml
workflows:
  - workflowId: order-flow
    inputs:
      $values: {}
    steps:
      - stepId: create
        operationPath: 'api#/paths/~1pets/put'
        successCriteria:
          - condition: $statusCode == 200
"#;

#[test]
fn arazzo_diff_reports_every_breaking_class() {
    let dir = tempfile::tempdir().unwrap();
    let old = write(dir.path(), "old.arazzo.yaml", SUITE_OLD);
    let new = write(dir.path(), "new.arazzo.yaml", SUITE_NEW_BREAKING);
    let output = std::process::Command::new(binary())
        .args(["arazzo-diff", old.to_str().unwrap(), new.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "breaking revisions must fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Removed step.
    assert!(stdout.contains("step 'fetch' removed"), "{stdout}");
    // Changed operation target.
    assert!(
        stdout.contains("step 'create' now targets operation 'put' (was 'post')"),
        "{stdout}"
    );
    // Removed workflow input.
    assert!(
        stdout.contains("workflow input 'petId' removed"),
        "{stdout}"
    );
    // Removed workflow output.
    assert!(
        stdout.contains("workflow output 'orderId' removed"),
        "{stdout}"
    );
    // Removed step output.
    assert!(
        stdout.contains("step 'create' output 'created' removed"),
        "{stdout}"
    );
    // Changed success criteria.
    assert!(stdout.contains("success criteria changed"), "{stdout}");
}

#[test]
fn arazzo_diff_is_quiet_when_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let old = write(dir.path(), "old.arazzo.yaml", SUITE_OLD);
    let new = write(dir.path(), "new.arazzo.yaml", SUITE_OLD);
    let output = std::process::Command::new(binary())
        .args(["arazzo-diff", old.to_str().unwrap(), new.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("arazzo-breaking-change"),
        "identical revisions must report no findings: {stdout}"
    );
}

// ------------------------------------------------------------ overlay dry-run

#[test]
fn overlay_dry_run_reports_matches_without_applying() {
    let dir = tempfile::tempdir().unwrap();
    let target = write(
        dir.path(),
        "target.yaml",
        r#"openapi: 3.1.0
info: {title: Target, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      summary: Lists pets
      responses:
        '200': {description: ok}
"#,
    );
    let overlay = write(
        dir.path(),
        "overlay.yaml",
        r#"overlay: 1.0.0
info: {title: Polish, version: '1'}
actions:
  - target: '$.paths./pets.get'
    update:
      summary: Lists every pet
  - target: '$.paths./missing'
    update: {summary: never matches}
"#,
    );
    let original = std::fs::read_to_string(&target).unwrap();
    let output = std::process::Command::new(binary())
        .args([
            "overlay-dry-run",
            overlay.to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    // One action has a zero-match target: the exit code flags it.
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("matches=1"), "{stderr}");
    assert!(stderr.contains("matches=0"), "{stderr}");
    assert!(stderr.contains("1 zero-match actions"), "{stderr}");
    // Dry run: the target file must be untouched.
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        original,
        "overlay-dry-run must not modify the target"
    );
}
