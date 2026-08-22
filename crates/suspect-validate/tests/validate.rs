use std::path::Path;
use std::sync::Arc;

use suspect_oas::{ModelError, Session};
use suspect_ref::WorkspaceBuilder;
use suspect_validate::{
    Diagnostic, Severity, validate_entry, validate_openapi, validate_workspace,
};

fn session_with(dir: &Path, name: &str, content: &str) -> Session {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    Session::new(Arc::new(ws))
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suspect-validate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
    diags.iter().map(|d| d.code).collect()
}

const HEADER: &str = "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\n";

#[test]
fn missing_operation_id_is_warning() {
    let dir = unique_dir("missing-opid");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      responses:\n        '200':\n          description: ok\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(codes(&diags).contains(&"oas-operation-missing-operationId"));
    let d = diags
        .iter()
        .find(|d| d.code == "oas-operation-missing-operationId")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);
}

#[test]
fn duplicate_operation_id_is_error() {
    let dir = unique_dir("dup-opid");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /a:\n    get:\n      operationId: same\n      responses: {{'200': {{description: ok}}}}\n  /b:\n    get:\n      operationId: same\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let dups: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-duplicate-operation-id")
        .collect();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].severity, Severity::Error);
    assert!(dups[0].message.contains("same"));
}

#[test]
fn missing_responses_is_error() {
    let dir = unique_dir("missing-responses");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n"),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-operation-missing-responses")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn parameter_missing_name_and_in() {
    let dir = unique_dir("param-fields");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      parameters:\n        - schema: {{type: string}}\n        - name: limit\n          schema: {{type: integer}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(codes(&diags).contains(&"oas-parameter-missing-name"));
    assert!(codes(&diags).contains(&"oas-parameter-missing-in"));
}

#[test]
fn path_param_not_declared_and_unused() {
    let dir = unique_dir("path-params");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /pets/{{petId}}:\n    get:\n      operationId: op\n      responses: {{'200': {{description: ok}}}}\n  /things:\n    get:\n      operationId: op2\n      responses: {{'200': {{description: ok}}}}\n    parameters:\n      - name: extra\n        in: path\n        required: true\n        schema: {{type: string}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let missing = diags
        .iter()
        .find(|d| d.code == "oas-path-param-not-declared")
        .unwrap();
    assert_eq!(missing.severity, Severity::Error);
    assert!(missing.message.contains("petId"));
    let unused = diags
        .iter()
        .find(|d| d.code == "oas-unused-path-param")
        .unwrap();
    assert_eq!(unused.severity, Severity::Warning);
    assert!(unused.message.contains("extra"));
}

#[test]
fn path_param_required_false_is_error() {
    let dir = unique_dir("required-false");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /i/{{id}}:\n    get:\n      operationId: op\n      parameters:\n        - name: id\n          in: path\n          required: false\n          schema: {{type: string}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-parameter-required-missing")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn response_missing_description_is_error() {
    let dir = unique_dir("resp-desc");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      responses:\n        '200':\n          content:\n            application/json: {{schema: {{type: string}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-response-missing-description")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn security_unknown_scheme_is_error() {
    let dir = unique_dir("security");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}security:\n  - apiKey: []\npaths:\n  /p:\n    get:\n      operationId: op\n      security:\n        - missing2: []\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let hits: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-security-unknown-scheme")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|d| d.severity == Severity::Error));
}

#[test]
fn server_variable_unknown_is_error() {
    let dir = unique_dir("server-var");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}servers:\n  - url: 'https://{{host}}/v1'\n    variables: {{}}\npaths: {{}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-server-variable-unknown")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("host"));
}

#[test]
fn undeclared_tag_is_warning() {
    let dir = unique_dir("tags");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      tags: [pets]\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-tag-undeclared")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // declared tags do not warn
    let dir2 = unique_dir("tags-ok");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        &format!(
            "{HEADER}tags:\n  - name: pets\npaths:\n  /p:\n    get:\n      operationId: op\n      tags: [pets]\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-tag-undeclared"));
}

#[test]
fn discriminator_missing_property_is_error() {
    let dir = unique_dir("disc-prop");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Pet:\n      type: object\n      required: [name]\n      properties:\n        name: {{type: string}}\n      discriminator:\n        propertyName: kind\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-discriminator-missing-property")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn discriminator_property_via_all_of_is_accepted() {
    let dir = unique_dir("disc-allof");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Base:\n      type: object\n      required: [kind]\n      properties:\n        kind: {{type: string}}\n    Dog:\n      allOf:\n        - $ref: '#/components/schemas/Base'\n      discriminator:\n        propertyName: kind\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(!codes(&diags).contains(&"oas-discriminator-missing-property"));
}

#[test]
fn discriminator_unknown_mapping_is_error() {
    let dir = unique_dir("disc-map");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Pet:\n      type: object\n      required: [kind]\n      properties:\n        kind: {{type: string}}\n      discriminator:\n        propertyName: kind\n        mapping:\n          dog: '#/components/schemas/Dog'\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-discriminator-unknown-mapping")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("Dog"));
}

#[test]
fn schema_unknown_type_is_error() {
    let dir = unique_dir("bad-type");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    A:\n      type: striing\n    B:\n      type: [string, bogus]\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let hits: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-schema-unknown-type")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|d| d.severity == Severity::Error));
}

#[test]
fn example_type_mismatch_is_warning() {
    let dir = unique_dir("example-mismatch");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    post:\n      operationId: op\n      requestBody:\n        content:\n          application/json:\n            schema: {{type: integer}}\n            example: hello\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-example-type-mismatch")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // matching example does not warn
    let dir2 = unique_dir("example-match");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    post:\n      operationId: op\n      requestBody:\n        content:\n          application/json:\n            schema: {{type: integer}}\n            example: 42\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-example-type-mismatch"));
}

#[test]
fn trailing_slash_and_bad_path_key() {
    let dir = unique_dir("path-keys");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /pets/: {{}}\n  pets: {{}}\n"),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let slash = diags
        .iter()
        .find(|d| d.code == "oas-path-trailing-slash")
        .unwrap();
    assert_eq!(slash.severity, Severity::Warning);
    let empty = diags
        .iter()
        .find(|d| d.code == "oas-empty-path-template")
        .unwrap();
    assert_eq!(empty.severity, Severity::Error);
}

#[test]
fn duplicate_header_param_is_error() {
    let dir = unique_dir("dup-header");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    parameters:\n      - name: X-Req\n        in: header\n        schema: {{type: string}}\n    get:\n      operationId: op\n      parameters:\n        - name: X-Req\n          in: header\n          schema: {{type: string}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-duplicate-header-param")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn deprecated_operation_is_info() {
    let dir = unique_dir("deprecated");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: old\n      deprecated: true\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-deprecated-operation")
        .unwrap();
    assert_eq!(d.severity, Severity::Info);
}

#[test]
fn webhooks_on_30_is_error() {
    let dir = unique_dir("webhooks30");
    let session = session_with(
        &dir,
        "main.yaml",
        "openapi: 3.0.0\ninfo: {title: t, version: \"1\"}\nwebhooks: {}\npaths: {}\n",
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-webhook-unsupported-version")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn license_missing_url_is_warning() {
    // 3.0: url required
    let dir = unique_dir("license30");
    let session = session_with(
        &dir,
        "main.yaml",
        "openapi: 3.0.0\ninfo:\n  title: t\n  version: \"1\"\n  license: {name: MIT}\npaths: {}\n",
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-license-missing-url")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // 3.1: identifier suffices
    let dir2 = unique_dir("license31");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\n  license: {name: MIT, identifier: MIT}\npaths: {}\n",
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-license-missing-url"));
}

#[test]
fn petstore_corpus_has_no_errors() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let ws = WorkspaceBuilder::new().root(&corpus).build().unwrap();
    let session = Session::new(Arc::new(ws));
    let diags = validate_entry(&session, "petstore-expanded.yaml").unwrap();
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn generated_fixture_validates_without_panic() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let ws = WorkspaceBuilder::new().root(&fixtures).build().unwrap();
    let session = Session::new(Arc::new(ws));
    let diags = validate_entry(&session, "generated_100x100.json").unwrap();
    // no assertion on count; exercise the full pipeline
    let _ = diags.len();
}

#[test]
fn validation_is_deterministic() {
    let dir = unique_dir("determinism");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /a:\n    get:\n      responses: {{'200': {{}}}}\n  /b:\n    post:\n      operationId: x\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let first = validate_entry(&session, "main.yaml").unwrap();
    let second = validate_entry(&session, "main.yaml").unwrap();
    assert_eq!(first, second);
}

#[test]
fn severity_mapping_matches_codes() {
    let dir = unique_dir("severities");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      deprecated: true\n      responses: {{'200': {{}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let by_code = |code: &str| diags.iter().find(|d| d.code == code).map(|d| d.severity);
    assert_eq!(by_code("oas-deprecated-operation"), Some(Severity::Info));
    assert_eq!(
        by_code("oas-response-missing-description"),
        Some(Severity::Error)
    );
    assert_eq!(
        by_code("oas-operation-missing-operationId"),
        Some(Severity::Warning)
    );
}

#[test]
fn workspace_validation_covers_loaded_docs() -> Result<(), ModelError> {
    let dir = unique_dir("workspace");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /p:\n    get:\n      responses: {{'200': {{}}}}\n"),
    );
    // a non-OpenAPI sibling must be skipped without error
    std::fs::write(dir.join("other.yaml"), "just: data\n").unwrap();

    let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
    ws.load_all("main.yaml").unwrap();
    let session2 = Session::new(Arc::new(ws));
    let diags = validate_workspace(&session2)?;
    assert!(
        diags
            .iter()
            .any(|d| d.code == "oas-response-missing-description")
    );

    // entry-based API agrees
    let per_entry = validate_entry(&session, "main.yaml")?;
    assert_eq!(per_entry.len(), diags.len());

    // direct view API produces the same set for this single-doc workspace
    let api = session.load("main.yaml")?;
    assert_eq!(validate_openapi(&api), per_entry);
    Ok(())
}
