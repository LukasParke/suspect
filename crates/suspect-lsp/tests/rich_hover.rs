//! Rich value hover: component targets of `$ref`s render structured
//! markdown (security scheme tables, parameter summaries, response
//! content lists, example values) rather than raw YAML excerpts.

use std::path::Path;

use suspect_lsp::hover_detail;
use suspect_source::{Source, Uri};

fn low_at(dir: &Path, name: &str, text: &str) -> suspect_low::LowDoc {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    suspect_low::LowDoc::parse(uri, Source::from_vec(text.as_bytes().to_vec()))
}

#[test]
fn security_scheme_ref_renders_the_scheme_table() {
    const SPEC: &str = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /p:
    get:
      operationId: getP
      responses:
        '200': {description: ok}
components:
  securitySchemes:
    ApiKeyAuth:
      type: http
      scheme: bearer
      bearerFormat: JWT
      description: Server-issued JWT
";
    let dir = std::env::temp_dir().join("suspect-lsp-hover-scheme");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let low = low_at(&dir, "spec.yaml", SPEC);
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .unwrap();
    ws.load_all("spec.yaml").unwrap();
    // Hover the component key directly for the rich render.
    let key_off = low
        .inner()
        .bytes()
        .windows(10)
        .position(|w| w == b"ApiKeyAuth")
        .unwrap();
    let md = suspect_lsp::navigation::hover_markdown(&ws, &low, key_off + 5).unwrap();
    assert!(md.contains("🔑 ApiKeyAuth"), "{md}");
    assert!(md.contains("bearer"), "{md}");
    assert!(md.contains("JWT"), "{md}");
    assert!(md.contains("Server-issued JWT"), "{md}");
}

#[test]
fn parameter_ref_renders_location_and_schema() {
    const SPEC: &str = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  parameters:
    Limit:
      name: limit
      in: query
      description: Max items per page
      schema:
        type: integer
        default: 20
        enum: [10, 20, 50]
";
    let dir = std::env::temp_dir().join("suspect-lsp-hover-param");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let low = low_at(&dir, "spec.yaml", SPEC);
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .unwrap();
    ws.load_all("spec.yaml").unwrap();
    let key_off = low
        .inner()
        .bytes()
        .windows(5)
        .position(|w| w == b"Limit")
        .unwrap();
    let md = suspect_lsp::navigation::hover_markdown(&ws, &low, key_off + 2).unwrap();
    assert!(md.contains("**Limit**"), "{md}");
    assert!(md.contains("in: query"), "{md}");
    assert!(md.contains("Max items per page"), "{md}");
    assert!(md.contains("**Default:** `20`"), "{md}");
    assert!(md.contains("**Enum:** `10` · `20` · `50`"), "{md}");
}

#[test]
fn response_ref_renders_content_types() {
    const SPEC: &str = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  responses:
    NotFound:
      description: Resource not found
      content:
        application/json:
          schema: {type: object}
";
    let dir = std::env::temp_dir().join("suspect-lsp-hover-resp");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let low = low_at(&dir, "spec.yaml", SPEC);
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .unwrap();
    ws.load_all("spec.yaml").unwrap();
    let key_off = low
        .inner()
        .bytes()
        .windows(8)
        .position(|w| w == b"NotFound")
        .unwrap();
    let md = suspect_lsp::navigation::hover_markdown(&ws, &low, key_off + 2).unwrap();
    assert!(md.contains("**NotFound**"), "{md}");
    assert!(md.contains("Resource not found"), "{md}");
    assert!(md.contains("**Content:** `application/json`"), "{md}");
    let _ = hover_detail::try_rich_hover;
}

#[test]
fn example_ref_renders_pretty_value() {
    const SPEC: &str = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  examples:
    SamplePet:
      summary: A sample pet
      value:
        name: Rex
        age: 3
";
    let dir = std::env::temp_dir().join("suspect-lsp-hover-ex");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let low = low_at(&dir, "spec.yaml", SPEC);
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .unwrap();
    ws.load_all("spec.yaml").unwrap();
    let key_off = low
        .inner()
        .bytes()
        .windows(9)
        .position(|w| w == b"SamplePet")
        .unwrap();
    let md = suspect_lsp::navigation::hover_markdown(&ws, &low, key_off + 4).unwrap();
    assert!(md.contains("**SamplePet**"), "{md}");
    assert!(md.contains("A sample pet"), "{md}");
    assert!(md.contains("```json"), "{md}");
    assert!(md.contains("\"name\": \"Rex\""), "{md}");
}
