//! Generates the data behind the README's LSP screenshots.
//!
//! Every entry is real server output for `docs/demo/petstore.yaml`,
//! expressed in editor coordinates (zero-based line / UTF-16 column) so
//! `docs/capture/render.py` can draw it exactly where VS Code would.
//!
//! Run with: `cargo run --example showcase -p suspect-lsp`, then
//! `python3 docs/capture/render.py` to redraw `docs/images/lsp-*.png`.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_lsp::state::{OpenDoc, lsp_range};
use suspect_lsp::{
    actions, completion, diagnostics, navigation, rename, semantic, workspace_symbol,
};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
use tower_lsp::lsp_types::{
    CodeActionKind, Diagnostic, DiagnosticSeverity, InlayHintKind, InlayHintLabel, NumberOrString,
    Position, Range, Url,
};

const SPEC: &str = include_str!("../../../docs/demo/petstore.yaml");

/// Byte offset of the `n`th occurrence (zero-based) of `needle`, plus
/// `within` bytes; the showcase anchors every feature on a spec token.
fn offset_of(needle: &str, nth: usize, within: usize) -> usize {
    SPEC.match_indices(needle)
        .nth(nth)
        .map(|(i, _)| i + within)
        .unwrap_or_else(|| panic!("`{needle}` #{nth} is not in docs/demo/petstore.yaml"))
}

fn pos(p: Position) -> Value {
    json!({ "line": p.line, "character": p.character })
}

fn range(r: Range) -> Value {
    json!({ "start": pos(r.start), "end": pos(r.end) })
}

fn severity(s: Option<DiagnosticSeverity>) -> &'static str {
    match s {
        Some(DiagnosticSeverity::ERROR) => "error",
        Some(DiagnosticSeverity::WARNING) => "warning",
        Some(DiagnosticSeverity::HINT) => "hint",
        _ => "info",
    }
}

fn code(c: Option<&NumberOrString>) -> String {
    match c {
        Some(NumberOrString::String(s)) => s.clone(),
        Some(NumberOrString::Number(n)) => n.to_string(),
        None => String::new(),
    }
}

fn diagnostic(d: &Diagnostic) -> Value {
    json!({
        "range": range(d.range),
        "severity": severity(d.severity),
        "code": code(d.code.as_ref()),
        "source": d.source.clone().unwrap_or_else(|| "suspect".to_owned()),
        "message": d.message,
    })
}

fn main() {
    let dir = std::env::temp_dir().join("suspect-showcase");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("petstore.yaml"), SPEC).unwrap();

    let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
    ws.load_all("petstore.yaml").unwrap();
    let ws = Arc::new(ws);
    let file_uri = Uri::from_path(&dir.join("petstore.yaml")).unwrap();
    let lsp_url = Url::parse(file_uri.as_str()).unwrap();
    let doc = OpenDoc::parse(file_uri.clone(), SPEC.to_owned());
    let low = &doc.low;
    let inner = low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    let to_range = |r: std::ops::Range<usize>| lsp_range(bytes, li, r);
    let to_pos = |off: usize| to_range(off..off).start;

    let mut out = serde_json::Map::new();
    out.insert("file".into(), json!("petstore.yaml"));
    out.insert("text".into(), json!(SPEC));

    // Diagnostics: the validator plus the Spectral-compatible linter, as the
    // server publishes them (severity, code, source, editor range).
    let mut all_diags = diagnostics::validate_diagnostics(&ws, low, false);
    all_diags.extend(diagnostics::lint_diagnostics(low, None, None));
    all_diags.sort_by_key(|d| (d.range.start.line, d.range.start.character));
    out.insert(
        "diagnostics".into(),
        Value::Array(all_diags.iter().map(diagnostic).collect()),
    );

    // Hover on the `$ref` to `Pet`: the resolved target rendered richly.
    let hover_off = offset_of("'#/components/schemas/Pet'", 0, 3);
    let hover_md =
        navigation::hover_markdown(&ws, low, hover_off).expect("hover on the Pet $ref resolves");
    let ref_value = offset_of("'#/components/schemas/Pet'", 0, 0);
    out.insert(
        "hover".into(),
        json!({
            "range": range(to_range(ref_value..ref_value + "'#/components/schemas/Pet'".len())),
            "markdown": hover_md,
        }),
    );

    // Code actions with the cursor inside the first operation (on its
    // `summary` key, where the operation diagnostics are anchored), fed the
    // real diagnostics that intersect it, plus the whole-document fix-all.
    let get_off = offset_of("summary: List pets", 0, 0);
    let get_range = to_range(get_off..get_off + "summary".len());
    let quick_fixes =
        actions::code_actions(&doc, &lsp_url, get_range, &all_diags, Some(&ws), false);
    let whole = Range::new(Position::new(0, 0), to_pos(SPEC.len()));
    let fix_all = actions::code_actions(&doc, &lsp_url, whole, &all_diags, Some(&ws), false)
        .into_iter()
        .filter(|a| {
            a.kind
                .as_ref()
                .is_some_and(|k| k != &CodeActionKind::QUICKFIX)
        });
    let action_items: Vec<Value> = quick_fixes
        .iter()
        .cloned()
        .chain(fix_all)
        .map(|a| {
            json!({
                "title": a.title,
                "kind": a.kind.map(|k| k.as_str().to_owned()),
            })
        })
        .collect();
    assert!(
        action_items.len() > 1,
        "the demo operation has quick fixes, got {action_items:?}"
    );
    out.insert(
        "code_actions".into(),
        json!({ "range": range(get_range), "actions": action_items }),
    );

    // Completion inside a `$ref` string: component pointers from the workspace.
    let compl_off = offset_of("'#/components/schemas/Pets'", 0, 1);
    assert_eq!(
        completion::context_at(low, compl_off),
        completion::CompletionContext::Refs,
        "cursor inside the $ref value is a ref-completion site"
    );
    let candidates = completion::ref_candidates(&ws, low.uri());
    assert!(!candidates.is_empty(), "the workspace has $ref candidates");
    out.insert(
        "completion".into(),
        json!({
            "position": pos(to_pos(compl_off)),
            "typed": "#/",
            "items": candidates,
        }),
    );

    // Go to definition from the `Pets` $ref to its declaration.
    let goto_off = offset_of("'#/components/schemas/Pets'", 0, 3);
    let def = navigation::goto_definition(&ws, low, goto_off).expect("the Pets $ref resolves");
    let ref_value = offset_of("'#/components/schemas/Pets'", 0, 0);
    out.insert(
        "goto_def".into(),
        json!({
            "source": range(to_range(ref_value..ref_value + "'#/components/schemas/Pets'".len())),
            "target": range(to_range(def.range)),
        }),
    );

    // Semantic tokens, decoded from the relative encoding into legend names.
    let tokens = semantic::semantic_tokens_full(&doc);
    let (mut line, mut col) = (0u32, 0u32);
    let token_items: Vec<Value> = tokens
        .data
        .iter()
        .map(|t| {
            if t.delta_line > 0 {
                line += t.delta_line;
                col = t.delta_start;
            } else {
                col += t.delta_start;
            }
            let modifiers: Vec<&str> = semantic::TOKEN_MODIFIERS
                .iter()
                .enumerate()
                .filter(|(i, _)| t.token_modifiers_bitset & (1 << i) != 0)
                .map(|(_, m)| m.as_str())
                .collect();
            json!({
                "line": line,
                "character": col,
                "length": t.length,
                "type": semantic::TOKEN_TYPES[t.token_type as usize].as_str(),
                "modifiers": modifiers,
            })
        })
        .collect();
    out.insert("semantic_tokens".into(), Value::Array(token_items));

    // Inlay hints over the whole document.
    let hints = semantic::inlay_hints(&doc, &ws, whole);
    let hint_items: Vec<Value> = hints
        .iter()
        .map(|h| {
            let label = match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                InlayHintLabel::LabelParts(parts) => {
                    parts.iter().map(|p| p.value.as_str()).collect()
                }
            };
            json!({
                "position": pos(h.position),
                "label": label,
                "kind": match h.kind {
                    Some(InlayHintKind::PARAMETER) => "parameter",
                    _ => "type",
                },
                "padding_left": h.padding_left.unwrap_or(false),
                "padding_right": h.padding_right.unwrap_or(false),
            })
        })
        .collect();
    assert!(!hint_items.is_empty(), "the demo spec yields inlay hints");
    out.insert("inlay_hints".into(), Value::Array(hint_items));

    // Workspace symbols: paths, operations and components.
    #[allow(deprecated)]
    let symbol_items: Vec<Value> = workspace_symbol::workspace_symbols(&ws, "")
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "kind": format!("{:?}", s.kind),
                "container": s.container_name,
                "line": s.location.range.start.line,
            })
        })
        .collect();
    out.insert("workspace_symbols".into(), Value::Array(symbol_items));

    // Rename `Pet` → `Animal`: the declaration plus every `$ref` to it.
    let pet_key = offset_of("    Pet:", 0, 4);
    let placeholder = rename::prepare_rename(&doc, pet_key).expect("`Pet` is renameable");
    let edit = rename::rename(&ws, &doc, "Animal", pet_key).expect("rename Pet → Animal");
    let mut edits: Vec<Value> = Vec::new();
    if let Some(changes) = &edit.changes {
        for (uri, text_edits) in changes {
            for e in text_edits {
                edits.push(json!({
                    "file": uri.path_segments().and_then(|mut s| s.next_back()).unwrap_or(""),
                    "range": range(e.range),
                    "new_text": e.new_text,
                }));
            }
        }
    }
    if let Some(tower_lsp::lsp_types::DocumentChanges::Edits(doc_edits)) = &edit.document_changes {
        for de in doc_edits {
            let file = de
                .text_document
                .uri
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or("")
                .to_owned();
            for e in &de.edits {
                let e = match e {
                    tower_lsp::lsp_types::OneOf::Left(e) => e.clone(),
                    tower_lsp::lsp_types::OneOf::Right(a) => a.text_edit.clone(),
                };
                edits
                    .push(json!({ "file": file, "range": range(e.range), "new_text": e.new_text }));
            }
        }
    }
    assert!(
        edits.len() >= 3,
        "rename rewrites the key and both $refs, got {edits:?}"
    );
    edits.sort_by_key(|e| {
        (
            e["range"]["start"]["line"].as_u64(),
            e["range"]["start"]["character"].as_u64(),
        )
    });
    out.insert(
        "rename".into(),
        json!({
            "range": range(to_range(placeholder.range)),
            "placeholder": placeholder.placeholder,
            "new_name": "Animal",
            "edits": edits,
        }),
    );

    let json = serde_json::to_string_pretty(&Value::Object(out)).unwrap();
    let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/capture/showcase_data.json");
    std::fs::write(&out_path, format!("{json}\n")).unwrap();
    println!("Wrote showcase data to {}", out_path.display());
}
