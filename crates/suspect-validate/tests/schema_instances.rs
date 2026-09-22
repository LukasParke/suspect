//! Schema-instance validation: declared examples and defaults must satisfy
//! their schemas (JSON Schema evaluation via the wrapper-document trick).

use std::path::Path;
use std::sync::Arc;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::validate_entry;

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

fn codes(dir: &Path, name: &str, spec: &str) -> Vec<&'static str> {
    let session = session_with(dir, name, spec);
    validate_entry(&session, name)
        .unwrap()
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn example_violating_the_schema_is_flagged() {
    let firing = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Port:
      type: integer
      minimum: 1
      example: 0
";
    let codes = codes(&unique_dir("inst-fire"), "main.yaml", firing);
    assert!(codes.contains(&"oas-schema-instance-invalid"), "{codes:?}");
}

#[test]
fn default_violating_enum_is_flagged() {
    let firing = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Status:
      type: string
      enum: [available, pending]
      default: sold
";
    let codes = codes(&unique_dir("inst-enum"), "main.yaml", firing);
    assert!(codes.contains(&"oas-schema-instance-invalid"), "{codes:?}");
}

#[test]
fn conforming_example_stays_silent() {
    let clean = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Port:
      type: integer
      minimum: 1
      example: 8080
      default: 32400
    Status:
      type: string
      enum: [available, pending]
      examples: [available]
";
    let codes = codes(&unique_dir("inst-clean"), "main.yaml", clean);
    assert!(!codes.contains(&"oas-schema-instance-invalid"), "{codes:?}");
}

#[test]
fn ref_bearing_schemas_are_skipped() {
    // A $ref-bearing schema cannot be inlined into the wrapper without a
    // resolver; instance checking has nothing honest to say there.
    let spec = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Ref:
      $ref: '#/components/schemas/Other'
      example: whatever
    Other:
      type: string
";
    let codes = codes(&unique_dir("inst-ref"), "main.yaml", spec);
    assert!(!codes.contains(&"oas-schema-instance-invalid"), "{codes:?}");
}
