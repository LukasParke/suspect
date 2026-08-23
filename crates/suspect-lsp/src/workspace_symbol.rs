//! Workspace-wide symbol search across every loaded document.

use suspect_arazzo::ArazzoDoc;
use suspect_low::{LowDoc, SpecFamily};
use suspect_ref::Workspace;
use suspect_source::LineIndex;
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{Location, OneOf, SymbolInformation, SymbolKind, Url, WorkspaceSymbol};

use crate::state::lsp_range;
use crate::symbols::METHODS;

/// Maximum number of symbols returned from one query.
pub const SYMBOL_CAP: usize = 200;

/// Per-document collector: applies the query filter and the global cap
/// while accumulating flat [`SymbolInformation`]s for one file.
struct Sink {
    /// Lowercased query; empty means match everything.
    query: String,
    /// Parsed URL for `Location`s in this document.
    url: Option<Url>,
    /// Document bytes and line index for range conversion.
    bytes: Vec<u8>,
    li: LineIndex,
    /// Accumulated symbols.
    out: Vec<SymbolInformation>,
}

impl Sink {
    /// Adds one symbol unless it fails the substring filter or the cap is
    /// already reached. `range` is a byte range inside this document.
    fn push(
        &mut self,
        name: &str,
        kind: SymbolKind,
        container: Option<String>,
        range: std::ops::Range<usize>,
    ) {
        if self.out.len() >= SYMBOL_CAP || self.url.is_none() {
            return;
        }
        if !self.query.is_empty() && !name.to_lowercase().contains(&self.query) {
            return;
        }
        // `deprecated` is still a mandatory initializer field even though
        // lsp-types deprecates it in favor of `tags`.
        #[allow(deprecated)]
        self.out.push(SymbolInformation {
            name: name.to_owned(),
            kind,
            tags: None,
            deprecated: None,
            location: Location {
                uri: self.url.clone().unwrap(),
                range: lsp_range(&self.bytes, &self.li, range),
            },
            container_name: container,
        });
    }
}

/// Collects workspace symbols across all loaded documents: component names,
/// path keys, operations (`GET /pets`), operationIds, top-level tags, and
/// Arazzo workflows. Overlay actions are intentionally skipped.
/// Results are filtered by case-insensitive substring on the name and capped
/// at [`SYMBOL_CAP`]. Documents are visited in workspace URI order, so the
/// result is deterministic; only documents loaded into `ws` (opened files
/// plus their `$ref` closure) are searched.
#[must_use]
pub fn workspace_symbols(ws: &Workspace, query: &str) -> Vec<SymbolInformation> {
    let query_lower = query.to_lowercase();
    let mut out: Vec<SymbolInformation> = Vec::new();
    for uri in ws.uris() {
        if out.len() >= SYMBOL_CAP {
            break;
        }
        let Some(handle) = ws.get(&uri) else { continue };
        let low = handle.doc();
        let inner = low.inner();
        let mut sink = Sink {
            query: query_lower.clone(),
            url: Url::parse(uri.as_str()).ok(),
            bytes: inner.bytes().to_vec(),
            li: inner.line_index().clone(),
            out: Vec::new(),
        };
        collect_doc(low, &mut sink);
        if sink.out.len() + out.len() > SYMBOL_CAP {
            let room = SYMBOL_CAP - out.len();
            out.extend(sink.out.into_iter().take(room));
            break;
        }
        out.append(&mut sink.out);
    }
    out
}

/// Wraps flat symbols into resolve-ready `WorkspaceSymbol`s.
///
/// Locations stay inline (the `lsp-types` type requires one), but each
/// symbol carries a `data` marker naming the symbol and its container so
/// [`resolve_workspace_symbol`] can re-derive a fresh range on
/// `workspaceSymbol/resolve` — the buffer may have moved since the query.
#[must_use]
pub fn workspace_symbols_nested(ws: &Workspace, query: &str) -> Vec<WorkspaceSymbol> {
    workspace_symbols(ws, query)
        .into_iter()
        .map(|si| WorkspaceSymbol {
            name: si.name.clone(),
            kind: si.kind,
            tags: si.tags,
            container_name: si.container_name.clone(),
            location: OneOf::Left(si.location),
            data: Some(serde_json::json!({
                "suspect": "wsym",
                "name": si.name,
                "container": si.container_name,
            })),
        })
        .collect()
}

/// Refreshes one symbol's location against the current workspace state.
///
/// Re-runs an exact-name lookup (container-aware when known); when the
/// symbol still exists its range reflects the live buffer, otherwise the
/// input symbol is returned untouched.
#[must_use]
pub fn resolve_workspace_symbol(sym: WorkspaceSymbol, ws: &Workspace) -> WorkspaceSymbol {
    let Some(data) = sym.data.as_ref() else {
        return sym;
    };
    let (Some(name), _) = (
        data.get("name").and_then(|n| n.as_str()),
        data.get("container").and_then(|c| c.as_str()),
    ) else {
        return sym;
    };
    // Exact-name match beats the substring filter of the original query.
    let fresh = workspace_symbols_nested(ws, name)
        .into_iter()
        .find(|s| s.name == name && s.container_name == sym.container_name);
    match fresh {
        Some(mut found) => {
            found.data = sym.data.clone();
            found
        }
        None => sym,
    }
}

/// Emits this document's symbols into `sink`, dispatched by spec family.
fn collect_doc(low: &LowDoc, sink: &mut Sink) {
    match low.sniff_family() {
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32 => {
            oas_workspace_symbols(low, sink)
        }
        SpecFamily::Arazzo10 => arazzo_workspace_symbols(low, sink),
        // Overlay actions are skipped; OAS 2.x / unknown families emit none.
        SpecFamily::Overlay10 | SpecFamily::Oas2 | SpecFamily::Unknown => {}
    }
}

/// OAS 3.x symbols: paths (Interface), operations (`GET /pets`, Method),
/// operationIds (Function), component names (Struct, container-qualified by
/// section), and top-level tags (Constant).
fn oas_workspace_symbols(low: &LowDoc, sink: &mut Sink) {
    let root = low.root();
    if let Some(paths) = root.get("paths") {
        for path in paths.entries() {
            let Some(item) = path.value else { continue };
            let path_range =
                key_range(low, &["paths", path.key]).unwrap_or_else(|| item.byte_range());
            sink.push(path.key, SymbolKind::INTERFACE, None, path_range);
            for method in METHODS {
                let Some(op) = item.get(method) else { continue };
                let op_name = format!("{} {}", method.to_uppercase(), path.key);
                let op_range =
                    key_range(low, &["paths", path.key, method]).unwrap_or_else(|| op.byte_range());
                sink.push(
                    &op_name,
                    SymbolKind::METHOD,
                    Some(path.key.to_owned()),
                    op_range,
                );
                if let Some(oid) = op.get("operationId").and_then(|n| n.as_str()) {
                    let oid_range = key_range(low, &["paths", path.key, method, "operationId"])
                        .unwrap_or_else(|| op.byte_range());
                    sink.push(oid, SymbolKind::FUNCTION, Some(op_name.clone()), oid_range);
                }
            }
        }
    }
    if let Some(components) = root.get("components") {
        for section in components.entries() {
            let Some(sec_val) = section.value else {
                continue;
            };
            let container = format!("components/{}", section.key);
            for entry in sec_val.entries() {
                // vendor extensions are not symbols
                if entry.key.starts_with("x-") {
                    continue;
                }
                let Some(v) = entry.value else { continue };
                let r = key_range(low, &["components", section.key, entry.key])
                    .unwrap_or_else(|| v.byte_range());
                sink.push(entry.key, SymbolKind::STRUCT, Some(container.clone()), r);
            }
        }
    }
    if let Some(tags) = root.get("tags") {
        for tag in tags.items() {
            let Some(name) = tag.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            sink.push(
                name,
                SymbolKind::CONSTANT,
                Some("tags".to_owned()),
                tag.byte_range(),
            );
        }
    }
}

/// Arazzo 1.0 symbols: one Object-kind symbol per workflow.
fn arazzo_workspace_symbols(low: &LowDoc, sink: &mut Sink) {
    let doc = ArazzoDoc::new(low);
    for w in doc.workflows() {
        sink.push(
            w.workflow_id,
            SymbolKind::OBJECT,
            Some("workflows".to_owned()),
            w.node().byte_range(),
        );
    }
}

/// Syntax-tree descent to the key node spelled at `path` (e.g.
/// `["components", "schemas", "Pet"]`), so symbol locations point at the
/// declaration key rather than its (possibly large) value.
#[must_use]
fn key_range(low: &LowDoc, path: &[&str]) -> Option<std::ops::Range<usize>> {
    let mut cur = low.inner().root();
    for (i, token) in path.iter().enumerate() {
        let target = token.as_bytes();
        let mut pair = None;
        for child in cur.children() {
            if child.kind() != SyntaxKind::Pair {
                continue;
            }
            let Some(k) = child.child_by_field("key") else {
                continue;
            };
            if k.scalar_bytes() == target {
                pair = Some(child);
                break;
            }
        }
        let pair = pair?;
        if i + 1 == path.len() {
            return Some(pair.child_by_field("key")?.byte_range());
        }
        cur = pair.child_by_field("value")?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ref::WorkspaceBuilder;

    const DOC: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
tags:
  - name: pets
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
components:
  schemas:
    Pet:
      type: object
"#;

    fn ws_with(dir: &std::path::Path, name: &str, text: &str) -> Workspace {
        std::fs::write(dir.join(name), text).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all(name).unwrap();
        ws
    }

    #[test]
    fn finds_component_and_operation_and_filters_by_substring() {
        let dir = std::env::temp_dir().join("suspect-lsp-wssym-query");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = ws_with(&dir, "doc.yaml", DOC);
        let all = workspace_symbols(&ws, "");
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Pet"), "{names:?}");
        assert!(names.contains(&"listPets"), "{names:?}");
        assert!(names.contains(&"GET /pets"), "{names:?}");
        assert!(names.contains(&"pets"), "tag symbol present: {names:?}");
        assert!(names.contains(&"/pets"), "{names:?}");

        let pet = all.iter().find(|s| s.name == "Pet").unwrap();
        assert_eq!(pet.kind, SymbolKind::STRUCT);
        assert_eq!(pet.container_name.as_deref(), Some("components/schemas"));
        let op = all.iter().find(|s| s.name == "GET /pets").unwrap();
        assert_eq!(op.kind, SymbolKind::METHOD);
        let oid = all.iter().find(|s| s.name == "listPets").unwrap();
        assert_eq!(oid.kind, SymbolKind::FUNCTION);

        // Case-insensitive substring filter.
        let filtered = workspace_symbols(&ws, "PET");
        let filtered_names: Vec<&str> = filtered.iter().map(|s| s.name.as_str()).collect();
        assert!(filtered_names.contains(&"Pet"));
        assert!(filtered_names.contains(&"listPets"));
        assert!(
            filtered_names.contains(&"/pets"),
            "\"/pets\" also matches \"pet\""
        );
        assert!(workspace_symbols(&ws, "zzz-not-there").is_empty());
    }

    #[test]
    fn nested_symbols_carry_data_and_resolve_refreshes_location() {
        let dir = std::env::temp_dir().join("suspect-lsp-wssym-resolve");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = ws_with(&dir, "doc.yaml", DOC);
        let nested = workspace_symbols_nested(&ws, "Pet");
        let pet = nested.iter().find(|s| s.name == "Pet").expect("Pet symbol");
        let data = pet.data.as_ref().expect("resolve marker present");
        assert_eq!(data.get("suspect").and_then(|v| v.as_str()), Some("wsym"));
        let OneOf::Left(Location { range: orig, .. }) = &pet.location else {
            panic!("inline location expected");
        };
        // Resolve against the same workspace keeps the location stable.
        let resolved = resolve_workspace_symbol(pet.clone(), &ws);
        let OneOf::Left(Location { range: same, .. }) = &resolved.location else {
            panic!("inline location expected");
        };
        assert_eq!(same, orig, "unchanged document keeps range");

        // A workspace where Pet moved down four comment lines: resolve
        // re-derives the fresh range instead of returning the stale one.
        let shifted = "# s1\n# s2\n# s3\n# s4\n".to_owned() + DOC;
        let dir2 = std::env::temp_dir().join("suspect-lsp-wssym-resolve2");
        std::fs::create_dir_all(&dir2).unwrap();
        let ws2 = ws_with(&dir2, "doc.yaml", &shifted);
        let refreshed = resolve_workspace_symbol(pet.clone(), &ws2);
        let OneOf::Left(Location { range: fresh, .. }) = &refreshed.location else {
            panic!("inline location expected");
        };
        assert_ne!(
            fresh.start, orig.start,
            "range must track the symbol's new position"
        );
        assert!(fresh.start.line > orig.start.line);
    }

    #[test]
    fn arazzo_workflows_surface_as_object_symbols() {
        let dir = std::env::temp_dir().join("suspect-lsp-wssym-arazzo");
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"
arazzo: 1.0.0
info:
  title: T
sourceDescriptions:
  - name: api
    url: openapi.yaml
workflows:
  - workflowId: checkout
    steps:
      - stepId: add
"#;
        let ws = ws_with(&dir, "flow.yaml", text);
        {
            let h = ws.get(ws.uris().first().unwrap()).unwrap();
            eprintln!(
                "family={:?} keys={:?}",
                h.doc().sniff_family(),
                h.doc()
                    .root()
                    .entries()
                    .iter()
                    .map(|e| e.key)
                    .collect::<Vec<_>>()
            );
        }
        let syms = workspace_symbols(&ws, "checkout");
        assert_eq!(syms[0].kind, SymbolKind::OBJECT);
    }

    #[test]
    fn results_are_capped_at_two_hundred() {
        let dir = std::env::temp_dir().join("suspect-lsp-wssym-cap");
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::from(
            "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\ncomponents:\n  schemas:\n",
        );
        for i in 0..250 {
            text.push_str(&format!("    Sym{i}:\n      type: object\n"));
        }
        let ws = ws_with(&dir, "big.yaml", &text);
        let syms = workspace_symbols(&ws, "");
        assert_eq!(syms.len(), SYMBOL_CAP);
        assert_eq!(syms[0].name, "Sym0");
    }
}

#[cfg(test)]
mod extension_filter_tests {
    use super::*;
    use suspect_ref::WorkspaceBuilder;

    #[test]
    fn vendor_extensions_are_not_symbols() {
        let dir = std::env::temp_dir().join("suspect-lsp-wsym-xfilter");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("spec.yaml"),
            "openapi: 3.1.0\ninfo: {title: t, version: \"1\"}\npaths: {}\ncomponents:\n  schemas:\n    Real:\n      type: object\n      x-meta: not-a-symbol\n  x-extension: {}\n",
        )
        .unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("spec.yaml").unwrap();
        let syms = workspace_symbols(&ws, "");
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Real"), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("x-")), "{names:?}");
    }
}
