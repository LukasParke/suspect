//! SDK-readiness notes: informational diagnostics mirroring the SDK
//! compiler's generation behaviors.

use std::path::Path;
use std::sync::Arc;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::{Diagnostic, validate_entry};

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

#[test]
fn unnamed_operations_are_flagged_for_method_path_selection() {
    let dir = unique_dir("sdk-op-id");
    let source = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /a:
    get:
      responses:
        '200': {description: ok}
  /b:
    get:
      operationId: getB
      responses:
        '200': {description: ok}
";
    let session = session_with(&dir, "main.yaml", source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let hits: Vec<&str> = findings
        .iter()
        .filter(|d| d.code == "sdk-operation-missing-id")
        .map(|d| d.code)
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", codes(&findings));
}

#[test]
fn untyped_stream_responses_are_flagged() {
    let dir = unique_dir("sdk-stream");
    let source = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /events:
    get:
      operationId: streamEvents
      responses:
        '200':
          description: stream
          content:
            text/event-stream: {}
";
    let session = session_with(&dir, "main.yaml", source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        codes(&findings).contains(&"sdk-stream-response-untyped"),
        "{:?}",
        codes(&findings)
    );

    // A schema on the media type clears the note.
    let typed = source.replace(
        "text/event-stream: {}",
        "text/event-stream:\n              schema:\n                type: object",
    );
    let session = session_with(&unique_dir("sdk-stream-typed"), "main.yaml", &typed);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        !codes(&findings).contains(&"sdk-stream-response-untyped"),
        "{:?}",
        codes(&findings)
    );
}

#[test]
fn colon_path_templates_are_flagged_for_the_profile() {
    let dir = unique_dir("sdk-colon");
    let source = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /users/:id:
    get:
      operationId: getUser
      responses:
        '200': {description: ok}
";
    let session = session_with(&dir, "main.yaml", source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    assert!(
        codes(&findings).contains(&"sdk-colon-path-parameters"),
        "{:?}",
        codes(&findings)
    );
}

#[test]
fn clean_documents_stay_silent() {
    let dir = unique_dir("sdk-clean");
    let source = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /users/{id}:
    get:
      operationId: getUser
      responses:
        '200':
          description: ok
";
    let session = session_with(&dir, "main.yaml", source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let sdk: Vec<&str> = findings
        .iter()
        .filter(|d| d.code.starts_with("sdk-"))
        .map(|d| d.code)
        .collect();
    assert!(sdk.is_empty(), "{:?} in {:?}", sdk, codes(&findings));
}
