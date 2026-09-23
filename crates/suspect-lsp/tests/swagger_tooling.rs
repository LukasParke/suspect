//! Swagger 2.0 shaping: symbols, completion contexts, and instance checks
//! on inline parameter schemas.

use suspect_source::{Source, Uri};

const SPEC: &str = "\
swagger: \"2.0\"
info: {title: t, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - name: limit
          in: query
          type: integer
          minimum: 1
          default: 0
      responses:
        '200': {description: ok}
definitions:
  Pet:
    type: object
    required: [name]
    properties:
      name: {type: string}
";

#[test]
fn swagger_documents_produce_symbols() {
    let dir = std::env::temp_dir().join("suspect-lsp-sw-symbols");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
    let uri = Uri::from_path(&dir.join("spec.yaml")).unwrap();
    let low = suspect_low::LowDoc::parse(uri, Source::from_vec(SPEC.as_bytes().to_vec()));
    let symbols = suspect_lsp::symbols::document_symbols(&low);
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"GET /pets") && names.contains(&"definitions"),
        "{names:?}"
    );
    // definitions children include the Pet class.
    let defs = symbols.iter().find(|s| s.name == "definitions").unwrap();
    let children = defs.children.as_ref().expect("definitions children");
    assert!(children.iter().any(|c| c.name == "Pet"), "{children:?}");
}

#[test]
fn swagger_keys_complete_per_context() {
    let dir = std::env::temp_dir().join("suspect-lsp-sw-completion");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
    let low = suspect_low::LowDoc::parse(
        Uri::from_path(&dir.join("spec.yaml")).unwrap(),
        Source::from_vec(SPEC.as_bytes().to_vec()),
    );
    // Root mapping key position.
    let off = SPEC.find("info:").unwrap();
    let ctx = suspect_lsp::completion::context_at(&low, off + 1);
    match ctx {
        suspect_lsp::completion::CompletionContext::Keys(keys) => {
            assert!(keys.contains(&"definitions"), "{keys:?}");
            assert!(keys.contains(&"host"), "{keys:?}");
        }
        other => panic!("expected keys, got {other:?}"),
    }
    // Operation key position.
    let off = SPEC.find("operationId").unwrap();
    let ctx = suspect_lsp::completion::context_at(&low, off + 1);
    match ctx {
        suspect_lsp::completion::CompletionContext::Keys(keys) => {
            assert!(keys.contains(&"responses"), "{keys:?}");
        }
        other => panic!("expected keys, got {other:?}"),
    }
}

#[test]
fn swagger_inline_parameter_defaults_get_instance_checks() {
    let dir = std::env::temp_dir().join("suspect-lsp-sw-instances");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
    let uri = Uri::from_path(&dir.join("spec.yaml")).unwrap();
    let low = suspect_low::LowDoc::parse(uri, Source::from_vec(SPEC.as_bytes().to_vec()));
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .unwrap();
    ws.load_all("spec.yaml").unwrap();
    let cfg = suspect_lsp::config_files::SuspectConfig::default();
    let diags = suspect_lsp::diagnostics::compute_diagnostics_raw(
        Some(&std::sync::Arc::new(ws)),
        &low,
        &cfg,
    );
    // default 0 violates minimum 1 on the inline parameter schema.
    let hit = diags.iter().any(|d| {
        matches!(
            &d.code,
            Some(tower_lsp::lsp_types::NumberOrString::String(s))
                if s == "oas-schema-instance-invalid"
        )
    });
    assert!(
        hit,
        "{:?}",
        diags.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}
