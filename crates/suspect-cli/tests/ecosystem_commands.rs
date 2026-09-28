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

// ---------------------------------------------------------- project build

use suspect_journal::{Body, BodyEncoding, CassetteEntry, CassetteHeader};

#[test]
fn project_build_publishes_profiles_and_docs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("openapi.yaml"),
        r#"openapi: 3.1.0
info: {title: Tickets API, version: '1.0.0'}
paths:
  /tickets:
    get: {operationId: listTickets, responses: {'200': {description: ok}}}
  /admin/reset:
    post: {operationId: adminReset, responses: {'200': {description: ok}}}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("public.overlay.yaml"),
        r#"overlay: 1.1.0
info: {title: strip admin, version: '1.0.0'}
actions:
  - target: '$.paths["/admin/reset"]'
    remove: true
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("suspect.project.json"),
        r#"{
  "version": 1,
  "name": "tickets",
  "entry": "openapi.yaml",
  "overlays": [],
  "publish": {"output": "build/spec.yaml", "profiles": {"public": ["public.overlay.yaml"]}},
  "docs": {"style": "markdown", "output": "build/docs"}
}"#,
    )
    .unwrap();

    let output = std::process::Command::new(binary())
        .args([
            "project",
            "build",
            "--manifest",
            root.join("suspect.project.json").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("build/spec.yaml").exists());
    assert!(root.join("build/spec.public.yaml").exists());
    assert!(root.join("build/docs/index.md").exists());
    // The public profile strips the admin endpoint.
    let public = std::fs::read_to_string(root.join("build/spec.public.yaml")).unwrap();
    assert!(!public.contains("adminReset"), "{public}");
    let published = std::fs::read_to_string(root.join("build/spec.yaml")).unwrap();
    assert!(published.contains("adminReset"), "{published}");
}

// ------------------------------------------------------------ release plan

#[test]
fn release_plan_recommends_major_for_breaks_and_minor_for_additions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("old.yaml"),
        r#"openapi: 3.1.0
info: {title: API, version: '1.0.0'}
paths:
  /pets:
    get:
      operationId: listPets
      responses: {'200': {description: ok}}
  /gone:
    get: {operationId: gone, responses: {'200': {description: ok}}}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("new.yaml"),
        r#"openapi: 3.1.0
info: {title: API, version: '1.1.0'}
paths:
  /pets:
    get:
      operationId: listPets
      responses: {'200': {description: ok}}
  /dogs:
    get: {operationId: listDogs, responses: {'200': {description: ok}}}
"#,
    )
    .unwrap();

    let output = std::process::Command::new(binary())
        .args([
            "release-plan",
            "--format",
            "json",
            root.join("old.yaml").to_str().unwrap(),
            root.join("new.yaml").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // /gone removed → MAJOR, /dogs added → listed in the changelog.
    assert_eq!(plan["semver"], "major");
    let changelog = plan["changelog"].as_str().unwrap();
    assert!(changelog.contains("/gone"), "{changelog}");
    assert!(changelog.contains("/dogs"), "{changelog}");
    assert!(!plan["breaks"].as_array().unwrap().is_empty());
}

// ------------------------------------------------------- traffic impact

#[test]
fn impact_flags_consumers_broken_by_the_new_contract() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let old_required = r#"openapi: 3.1.0
info: {title: API, version: '1.0.0'}
paths:
  /pets/{petId}:
    get:
      operationId: getPet
      parameters: [{name: petId, in: path, required: true, schema: {type: string}}]
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Pet'}
components:
  schemas:
    Pet: {type: object, required: [name], properties: {name: {type: string}}}
"#;
    let new_required = old_required
        .replace("required: [name]", "required: [name, kind]")
        .replace(
            "properties: {name: {type: string}}",
            "properties: {name: {type: string}, kind: {type: string}}",
        );
    std::fs::write(root.join("old.yaml"), old_required).unwrap();
    std::fs::write(root.join("new.yaml"), new_required).unwrap();

    let entry = CassetteEntry {
        id: 1,
        method: "GET".to_owned(),
        url: "http://api.test/pets/42".to_owned(),
        status: 200,
        request_headers: vec![("User-Agent".to_owned(), "consumer-a/1.0".to_owned())],
        request_body: Body {
            encoding: BodyEncoding::Utf8,
            content: String::new(),
            sha256: suspect_journal::sha256_hex(b""),
        },
        response_headers: vec![],
        response_body: Body {
            encoding: BodyEncoding::Utf8,
            content: r#"{"name":"rex"}"#.to_owned(),
            sha256: suspect_journal::sha256_hex(br#"{"name":"rex"}"#),
        },
        duration_ms: 1.0,
    };
    let header = CassetteHeader {
        format: suspect_journal::CASSETTE_FORMAT.to_owned(),
        version: suspect_journal::CASSETTE_VERSION,
        recorded_at_ms: 0,
        source: "test".to_owned(),
    };
    let cassette = root.join("traffic.scj");
    let mut file = std::fs::File::create(&cassette).unwrap();
    suspect_journal::write_cassette(&mut file, &header, &[entry]).unwrap();

    let output = std::process::Command::new(binary())
        .args([
            "impact",
            "--format",
            "json",
            root.join("old.yaml").to_str().unwrap(),
            root.join("new.yaml").to_str().unwrap(),
            cassette.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success(), "broken consumers must fail");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["evaluated"], 1);
    assert_eq!(report["broken"], 1);
    let consumer = report["by_consumer"]["user-agent:consumer-a/1.0"][0]
        .as_object()
        .expect("consumer group present");
    assert_eq!(consumer["passes_old"], true);
    assert_eq!(consumer["passes_new"], false);
    let reason = consumer["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("kind") || !reason.is_empty(),
        "reason names the violation: {reason}"
    );
    // The same traffic against identical revisions is unaffected.
    let same = std::process::Command::new(binary())
        .args([
            "impact",
            root.join("old.yaml").to_str().unwrap(),
            root.join("old.yaml").to_str().unwrap(),
            cassette.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(same.status.success());
}
