//! Vendor-extension validation: registered extension keys (builtin or
//! workspace-configured) validate their values against their JSON Schemas.

use suspect_lsp::diagnostics::compute_diagnostics_raw;
use suspect_lsp::state::OpenDoc;
use suspect_ref::WorkspaceBuilder;

fn low_at(dir: &std::path::Path, name: &str, text: &str) -> suspect_low::LowDoc {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    let uri = suspect_source::Uri::from_path(&path).unwrap();
    suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    )
}

fn codes(diags: &[tower_lsp::lsp_types::Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .filter_map(|d| match &d.code {
            Some(tower_lsp::lsp_types::NumberOrString::String(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn builtin_extension_schema_validates_values() {
    let dir = std::env::temp_dir().join("suspect-lsp-ext-builtin");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // x-codeSamples entries require lang and source.
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
x-internal: not-a-boolean
";
    let low = low_at(&dir, "spec.yaml", text);
    let ws = std::sync::Arc::new(
        suspect_ref::WorkspaceBuilder::new()
            .root(&dir)
            .build()
            .unwrap(),
    );
    let cfg = suspect_lsp::config_files::SuspectConfig::default();
    let diags = compute_diagnostics_raw(Some(&ws), &low, &cfg);
    let all = codes(&diags);
    assert!(all.iter().any(|c| c == "extension-schema"), "{all:?}");
}

#[test]
fn unknown_extensions_stay_silent() {
    let dir = std::env::temp_dir().join("suspect-lsp-ext-unknown");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
x-totally-custom:
  anything:
    goes: here
";
    let low = low_at(&dir, "spec.yaml", text);
    let ws = std::sync::Arc::new(WorkspaceBuilder::new().root(&dir).build().unwrap());
    let cfg = suspect_lsp::config_files::SuspectConfig::default();
    let diags = compute_diagnostics_raw(Some(&ws), &low, &cfg);
    assert!(
        !codes(&diags).iter().any(|c| c == "extension-schema"),
        "unknown extensions are legal: {:?}",
        codes(&diags)
    );
}

#[test]
fn workspace_config_supplies_custom_extension_schemas() {
    let dir = std::env::temp_dir().join("suspect-lsp-ext-custom");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("x-plex-token.schema.json"),
        r#"{"type":"object","required":["token"]}"#,
    )
    .unwrap();
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
x-plex-token:
  wrong: shape
";
    let low = low_at(&dir, "spec.yaml", text);
    let ws = std::sync::Arc::new(WorkspaceBuilder::new().root(&dir).build().unwrap());
    let cfg = suspect_lsp::config_files::parse_config(&serde_json::json!({
        "extensions": {
            "x-plex-token": {
                "schema": "x-plex-token.schema.json"
            }
        }
    }))
    .expect("config parses");
    let diags = compute_diagnostics_raw(Some(&ws), &low, &cfg);
    let all = codes(&diags);
    assert!(all.iter().any(|c| c == "extension-schema"), "{all:?}");

    // A conforming value is silent.
    let good = text.replace("  wrong: shape", "  token: abc123");
    let low = low_at(&dir, "spec.yaml", &good);
    let diags = compute_diagnostics_raw(Some(&ws), &low, &cfg);
    assert!(
        !codes(&diags).iter().any(|c| c == "extension-schema"),
        "{:?}",
        codes(&diags)
    );
    let _ = OpenDoc::parse(
        suspect_source::Uri::parse("mem://x.yaml").unwrap(),
        text.to_owned(),
    );
}

#[test]
fn inline_schema_in_config() {
    let dir = std::env::temp_dir().join("suspect-lsp-ext-inline");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
x-retry-count: not-a-number
";
    let low = low_at(&dir, "spec.yaml", text);
    let ws = std::sync::Arc::new(WorkspaceBuilder::new().root(&dir).build().unwrap());
    let cfg = suspect_lsp::config_files::parse_config(&serde_json::json!({
        "extensions": {
            "x-retry-count": { "schema": {"type": "integer"} }
        }
    }))
    .unwrap();
    let diags = compute_diagnostics_raw(Some(&ws), &low, &cfg);
    assert!(codes(&diags).iter().any(|c| c == "extension-schema"));
}
