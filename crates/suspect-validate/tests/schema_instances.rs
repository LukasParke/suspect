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

/// Swagger 2.0 documents route through the low-tree battery, not the
/// 3.x Session model.
fn swagger_codes(dir: &Path, name: &str, spec: &str) -> Vec<&'static str> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), spec).unwrap();
    let uri = suspect_source::Uri::from_path(&dir.join(name)).unwrap();
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(spec.as_bytes().to_vec()),
    );
    suspect_validate::validate_swagger_low(&low)
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

#[test]
fn swagger_definitions_get_instance_checks() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
definitions:
  Port:
    type: integer
    minimum: 1
    example: 0
paths: {}
";
    let firing_codes = swagger_codes(&unique_dir("inst-swagger"), "main.yaml", firing);
    assert!(
        firing_codes.contains(&"oas-schema-instance-invalid"),
        "{firing_codes:?}"
    );

    let clean = firing.replace("example: 0", "example: 8080");
    let clean_codes = swagger_codes(&unique_dir("inst-swagger-clean"), "main.yaml", &clean);
    assert!(
        !clean_codes.contains(&"oas-schema-instance-invalid"),
        "{clean_codes:?}"
    );
}
