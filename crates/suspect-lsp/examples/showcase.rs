//! Generates JSON data for LSP feature screenshots.
//!
//! Run with: `cargo run --example showcase -p suspect-lsp`

use std::sync::Arc;

use suspect_lint::Linter;
use suspect_lsp::actions;
use suspect_lsp::navigation;
use suspect_lsp::semantic;
use suspect_lsp::state::OpenDoc;
use suspect_lsp::workspace_symbol;
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
use suspect_validate::validate_entry;

const SPEC: &str = include_str!("../../../docs/demo/petstore.yaml");

fn offset_of(needle: &str) -> usize {
    SPEC.find(needle).unwrap_or(0) + needle.len() / 2
}

fn main() {
    let dir = std::env::temp_dir().join("suspect-showcase");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("petstore.yaml"), SPEC).unwrap();

    let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
    ws.load_all("petstore.yaml").unwrap();
    let ws = Arc::new(ws);
    let session = Session::new(Arc::clone(&ws));
    let file_uri = Uri::from_path(&dir.join("petstore.yaml")).unwrap();
    let doc = OpenDoc::parse(file_uri.clone(), SPEC.to_owned());
    let low = &doc.low;

    let mut entries: Vec<serde_json::Value> = Vec::new();

    // 1. Hover on $ref
    let hover_off = offset_of("#/components/schemas/Pets");
    if let Some(md) = navigation::hover_markdown(&ws, low, hover_off) {
        entries.push(serde_json::json!({
            "feature": "hover",
            "title": "Hover: resolved $ref target",
            "offset": hover_off,
            "markdown": md,
        }));
    }

    // 2. Diagnostics (validate + lint)
    let diags = validate_entry(&session, "petstore.yaml").unwrap_or_default();
    let lint_results = Linter::spectral_default().run(low);
    let diag_items: Vec<serde_json::Value> = diags
        .iter()
        .map(|d| {
            serde_json::json!({
                "code": d.code,
                "severity": format!("{:?}", d.severity),
                "message": d.message,
                "start": d.range.start,
            })
        })
        .collect();
    let lint_items: Vec<serde_json::Value> = lint_results
        .iter()
        .map(|f| {
            serde_json::json!({
                "code": f.code,
                "severity": format!("{:?}", f.severity),
                "message": f.message,
                "start": f.range.start,
            })
        })
        .collect();
    entries.push(serde_json::json!({
        "feature": "diagnostics",
        "title": "Diagnostics: semantic + lint",
        "validate": diag_items,
        "lint": lint_items,
    }));

    // 3. Go-to-definition on $ref
    let goto_off = offset_of("#/components/schemas/Pet");
    if let Some(def) = navigation::goto_definition(&ws, low, goto_off) {
        entries.push(serde_json::json!({
            "feature": "goto_def",
            "title": "Go to definition: $ref target",
            "target_line": {
                "uri": def.uri.as_str(),
                "start": def.range.start,
            },
        }));
    }

    // 4. Semantic tokens count
    let tokens = semantic::semantic_tokens_full(&doc);
    entries.push(serde_json::json!({
        "feature": "semantic_tokens",
        "title": "Semantic tokens",
        "count": tokens.data.len(),
    }));

    // 5. Code actions at missing-operationId position
    let action_range = tower_lsp::lsp_types::Range {
        start: tower_lsp::lsp_types::Position {
            line: 4,
            character: 4,
        },
        end: tower_lsp::lsp_types::Position {
            line: 4,
            character: 8,
        },
    };
    let diag = tower_lsp::lsp_types::Diagnostic {
        range: action_range,
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            "oas-operation-missing-operationId".into(),
        )),
        message: "Operation must have an `operationId`.".into(),
        ..Default::default()
    };
    let lsp_url = tower_lsp::lsp_types::Url::parse(file_uri.as_str()).unwrap();
    let actions = actions::code_actions(&doc, &lsp_url, action_range, &[diag]);
    let action_items: Vec<serde_json::Value> = actions
        .iter()
        .map(|a| serde_json::json!({ "title": a.title, "kind": format!("{:?}", a.kind) }))
        .collect();
    entries.push(serde_json::json!({
        "feature": "code_actions",
        "title": "Code actions: quick fixes",
        "actions": action_items,
    }));

    // 6. Inlay hints
    let full_range = tower_lsp::lsp_types::Range {
        start: tower_lsp::lsp_types::Position::default(),
        end: tower_lsp::lsp_types::Position {
            line: u32::MAX,
            character: u32::MAX,
        },
    };
    let hints = semantic::inlay_hints(&doc, &ws, full_range);
    let hint_items: Vec<serde_json::Value> = hints
        .iter()
        .map(|h| {
            serde_json::json!({
                "label": format!("{h:?}"),
                "line": h.position.line,
            })
        })
        .collect();
    entries.push(serde_json::json!({
        "feature": "inlay_hints",
        "title": "Inlay hints: $ref targets + property types",
        "hints": hint_items,
    }));

    // 7. Workspace symbols
    let symbols = workspace_symbol::workspace_symbols(&ws, "");
    let sym_names: Vec<String> = symbols.iter().map(|s| s.name.clone()).collect();
    entries.push(serde_json::json!({
        "feature": "workspace_symbols",
        "title": "Workspace symbols",
        "names": sym_names,
    }));

    // Output JSON
    let json = serde_json::to_string_pretty(&entries).unwrap();
    let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/capture/showcase_data.json");
    std::fs::write(&out_path, json).unwrap();
    println!(
        "Wrote {} feature entries to {}",
        entries.len(),
        out_path.display()
    );
}
