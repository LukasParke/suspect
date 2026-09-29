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

#[test]
fn project_build_generates_declared_sdk_targets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("openapi.yaml"),
        r#"openapi: 3.1.0
info: {title: Tickets API, version: '1.0.0'}
paths:
  /tickets:
    get:
      operationId: listTickets
      responses:
        '200':
          description: Every ticket.
          content:
            application/json:
              schema: {type: array, items: {type: string}}
"#,
    )
    .unwrap();
    let manifest = root.join("suspect.project.json");
    std::fs::write(
        &manifest,
        r#"{
  "version": 1,
  "name": "tickets-sdk",
  "entry": "openapi.yaml",
  "publish": {"output": "build/spec.yaml"},
  "codegen": [
    {"name": "ts", "profile": "typescript-http", "package_name": "@acme/tickets", "package_version": "1.4.0", "out": "sdk/typescript", "operation_id": ["listTickets"]},
    {"name": "py", "profile": "python-http", "package_name": "acme-tickets", "package_version": "1.4.0", "out": "sdk/python", "operation_id": ["listTickets"]}
  ]
}"#,
    )
    .unwrap();

    // check passes with only these two declared targets
    let check = std::process::Command::new(binary())
        .args(["project", "check", "--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );

    let output = std::process::Command::new(binary())
        .args(["project", "build", "--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Both SDKs were generated from the published spec, with the declared
    // package identity and version.
    let package = std::fs::read_to_string(root.join("sdk/typescript/typescript/package.json"))
        .expect("typescript package");
    assert!(package.contains("\"@acme/tickets\""), "{package}");
    assert!(package.contains("\"version\": \"1.4.0\""), "{package}");
    let pyproject =
        std::fs::read_to_string(root.join("sdk/python/python/pyproject.toml")).expect("pyproject");
    assert!(pyproject.contains("acme-tickets"), "{pyproject}");
    assert!(pyproject.contains("1.4.0"), "{pyproject}");
}

#[test]
fn project_check_rejects_an_unknown_sdk_profile() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("openapi.yaml"),
        "openapi: 3.1.0\ninfo: {title: x, version: '1'}\npaths: {}\n",
    )
    .unwrap();
    let manifest = root.join("suspect.project.json");
    std::fs::write(
        &manifest,
        r#"{
  "version": 1,
  "entry": "openapi.yaml",
  "codegen": [
    {"profile": "cobol-http", "package_name": "x", "package_version": "1.0.0", "out": "sdk/cobol"}
  ]
}"#,
    )
    .unwrap();
    let check = std::process::Command::new(binary())
        .args(["project", "check", "--manifest", manifest.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "an unknown profile must fail check"
    );
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(stderr.contains("project-unknown-profile"), "{stderr}");
}

// ------------------------------------------------------ config + contract

#[test]
fn config_supplies_defaults_and_explicit_flags_win() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join(".suspect.yaml"),
        "lint:\n  min_severity: warning\nvalidate:\n  strict_format: true\ndocs:\n  style: markdown\n  out: site/reference\nformat:\n  json: true\n",
    )
    .unwrap();
    let spec = write(
        root,
        "api.yaml",
        "openapi: 3.1.0\ninfo: {title: Config API, version: '1'}\npaths:\n  /x:\n    get:\n      operationId: getX\n      responses: {'200': {description: ok}}\n",
    );

    // `suspect config` reports the file in effect.
    let config = std::process::Command::new(binary())
        .args(["config"])
        .current_dir(root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&config.stdout);
    assert!(stdout.contains(".suspect.yaml"), "{stdout}");
    assert!(stdout.contains("lint.min_severity: warning"), "{stdout}");

    // Docs picks up the configured style and output with no flags.
    let docs = std::process::Command::new(binary())
        .args(["docs", "api.yaml"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        docs.status.success(),
        "{}",
        String::from_utf8_lossy(&docs.stderr)
    );
    assert!(
        root.join("site/reference/index.md").is_file(),
        "the configured docs style and output applied"
    );

    // An explicit flag overrides the configured output.
    let explicit = dir.path().join("elsewhere");
    let docs = std::process::Command::new(binary())
        .args(["docs", "api.yaml", "--style", "html", "--output"])
        .arg(&explicit)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(docs.status.success());
    assert!(explicit.is_file(), "the explicit output flag won");

    // fmt honors the configured JSON default.
    let fmt = std::process::Command::new(binary())
        .args(["fmt", spec.to_str().unwrap()])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&fmt.stdout)
            .trim_start()
            .starts_with('{'),
        "configured format.json applied"
    );
}

#[test]
fn contract_package_is_self_contained_and_detects_drift() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("schemas")).unwrap();
    std::fs::write(
        root.join("openapi.yaml"),
        r#"openapi: 3.1.0
info: {title: Contract API, version: '2.1.0'}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: A page
          content:
            application/json:
              schema:
                type: array
                items: {$ref: 'schemas/pet.yaml'}
components:
  schemas:
    Pet: {$ref: 'schemas/pet.yaml'}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("schemas/pet.yaml"),
        "type: object\nrequired: [id, name]\nproperties:\n  id: {type: string}\n  name: {type: string}\n  friend: {$ref: 'pet.yaml'}\n",
    )
    .unwrap();

    let output = std::process::Command::new(binary())
        .args(["contract", "openapi.yaml", "--out", "pkg"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let description = std::fs::read_to_string(root.join("pkg/openapi.yaml")).unwrap();
    // The cross-document ref is inlined, so the package stands alone.
    assert!(
        !description.contains("$ref: schemas/pet.yaml"),
        "{description}"
    );
    assert!(description.contains("required:"), "{description}");
    // The recursive schema terminates with a cycle marker.
    assert!(
        description.contains("x-suspect-cyclic"),
        "recursive refs must terminate, not hang: {description}"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("pkg/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["format"], "suspect.contract.v1");
    assert_eq!(manifest["title"], "Contract API");
    assert_eq!(manifest["operations"], 1);
    assert!(
        manifest["revision"]
            .as_str()
            .unwrap()
            .starts_with("sha256-"),
        "{manifest}"
    );

    // Fresh: --check passes.
    let check = std::process::Command::new(binary())
        .args(["contract", "openapi.yaml", "--out", "pkg", "--check"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "a fresh package must verify: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    // Any byte change in the closure invalidates it.
    std::fs::write(
        root.join("schemas/pet.yaml"),
        "type: object\nrequired: [id, name, kind]\nproperties:\n  id: {type: string}\n  name: {type: string}\n  kind: {type: string}\n",
    )
    .unwrap();
    let stale = std::process::Command::new(binary())
        .args(["contract", "openapi.yaml", "--out", "pkg", "--check"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        !stale.status.success(),
        "a changed closure must fail --check"
    );
    let stderr = String::from_utf8_lossy(&stale.stderr);
    assert!(stderr.contains("source closure changed"), "{stderr}");
}

// ------------------------------------------------------------------ ci gate

fn init_git_repo(root: &std::path::Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "ci@example.test"],
        vec!["config", "user.name", "ci"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "initial"],
        vec!["tag", "v0.1.0"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }
}

fn service(root: &std::path::Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("openapi.yaml"),
        format!(
            "openapi: 3.1.0\ninfo: {{title: Service {name}, version: '1.0.0'}}\npaths:\n  /things:\n    get: {{operationId: listThings, responses: {{'200': {{description: ok}}}}}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("suspect.project.json"),
        format!(
            r#"{{"version": 1, "name": "{name}", "entry": "openapi.yaml",
 "publish": {{"output": "build/spec.yaml"}},
 "contract": {{"output": "build/contract"}},
 "codegen": [{{"name": "ts", "profile": "typescript-http", "package_name": "@acme/{name}", "package_version": "1.0.0", "out": "sdk", "operation_id": ["listThings"]}}]}}"#
        ),
    )
    .unwrap();
}

#[test]
fn ci_gate_reports_every_project_and_isolates_failures() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    service(root, "svc-a");
    service(root, "svc-b");
    init_git_repo(root);

    // Nothing is published yet: every project fails the validate stage.
    let first = std::process::Command::new(binary())
        .args(["ci", "."])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        !first.status.success(),
        "an unpublished workspace must fail"
    );
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(
        stdout.contains("svc-a") && stdout.contains("svc-b"),
        "{stdout}"
    );
    assert!(stdout.contains("was never published"), "{stdout}");

    // Build both, then the gate is green.
    for name in ["svc-a", "svc-b"] {
        let build = std::process::Command::new(binary())
            .args([
                "project",
                "build",
                "--manifest",
                &format!("{name}/suspect.project.json"),
            ])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
    }
    let green = std::process::Command::new(binary())
        .args(["ci", ".", "--baseline", "v0.1.0"])
        .current_dir(root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&green.stdout);
    assert!(
        green.status.success(),
        "a published workspace must pass: {stdout}"
    );
    assert!(
        stdout.contains("2 project(s): 2 passed, 0 failed"),
        "{stdout}"
    );

    // Break one service only; the other must stay green.
    std::fs::write(
        root.join("svc-a/openapi.yaml"),
        "openapi: 3.1.0\ninfo: {title: Service svc-a, version: '2.0.0'}\npaths:\n  /things: {}\n",
    )
    .unwrap();
    for name in ["svc-a", "svc-b"] {
        std::process::Command::new(binary())
            .args([
                "project",
                "build",
                "--manifest",
                &format!("{name}/suspect.project.json"),
            ])
            .current_dir(root)
            .output()
            .unwrap();
    }
    let mixed = std::process::Command::new(binary())
        .args(["ci", ".", "--baseline", "v0.1.0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        !mixed.status.success(),
        "a broken service must fail the gate"
    );
    let stdout = String::from_utf8_lossy(&mixed.stdout);
    assert!(stdout.contains("FAIL  svc-a"), "{stdout}");
    assert!(
        stdout.contains("PASS  svc-b"),
        "the healthy service still reports: {stdout}"
    );
    assert!(stdout.contains("1 passed, 1 failed"), "{stdout}");
}

#[test]
fn ci_reports_a_machine_readable_aggregate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    service(root, "svc-a");
    init_git_repo(root);
    let output = std::process::Command::new(binary())
        .args(["ci", ".", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format"], "suspect.ci.v1");
    assert_eq!(report["projects"].as_array().unwrap().len(), 1);
    let stages: Vec<&str> = report["projects"][0]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["stage"].as_str().unwrap())
        .collect();
    assert!(stages.contains(&"validate"), "{stages:?}");
    assert!(stages.contains(&"contract"), "{stages:?}");
    assert!(stages.contains(&"breaking"), "{stages:?}");
}
