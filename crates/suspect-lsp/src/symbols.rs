//! Document symbols and folding ranges, built from the CST.

use suspect_arazzo::ArazzoDoc;
use suspect_low::{LowDoc, NodeRef, SpecFamily};
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{
    DocumentSymbol, FoldingRange, SymbolKind,
};

use crate::state::lsp_range;

/// HTTP methods surfaced as one symbol each under a path item.
const METHODS: &[&str] =
    &["get", "put", "post", "delete", "options", "head", "patch", "trace"];

/// Convenience constructor for a [`DocumentSymbol`] whose selection range
/// equals its full range.
fn symbol(
    name: String,
    kind: SymbolKind,
    range: tower_lsp::lsp_types::Range,
    children: Vec<DocumentSymbol>,
    ) -> DocumentSymbol {
    #[allow(deprecated)]
    DocumentSymbol {
        name,
        detail: None,
        kind,
        range,
        selection_range: range,
        children: Some(children),
        tags: None,
        deprecated: None,
    }
}

/// Hierarchical document symbols (nested one level) per spec family.
#[must_use]
pub fn document_symbols(low: &LowDoc) -> Vec<DocumentSymbol> {
    match low.sniff_family() {
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32 => oas_symbols(low),
        SpecFamily::Arazzo10 => arazzo_symbols(low),
        SpecFamily::Overlay10 => overlay_symbols(low),
        SpecFamily::Oas2 | SpecFamily::Unknown => Vec::new(),
    }
}

/// OAS 3.x symbols: one `METHOD <path>` symbol per operation, a
/// `components` module symbol per section with its named entries as
/// children, and one symbol per entry of the top-level `tags` sequence.
fn oas_symbols(low: &LowDoc) -> Vec<DocumentSymbol> {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    let rng = |n: &NodeRef<'_>| lsp_range(bytes, li, n.byte_range());
    let root = low.root();
    let mut out = Vec::new();

    if let Some(paths) = root.get("paths") {
        for path in paths.entries() {
            let Some(item) = path.value else { continue };
            for method in METHODS {
                if let Some(op) = item.get(method) {
                    out.push(symbol(
                        format!("{} {}", method.to_uppercase(), path.key),
                        SymbolKind::METHOD,
                        rng(&op),
                        Vec::new(),
                    ));
                }
            }
        }
    }

    if let Some(components) = root.get("components") {
        for section in components.entries() {
            let Some(sec_val) = section.value else { continue };
            let kind =
                if section.key == "schemas" { SymbolKind::CLASS } else { SymbolKind::VARIABLE };
            let children = sec_val
                .entries()
                .iter()
                .filter_map(|e| {
                    let v = e.value?;
                    Some(symbol(e.key.to_owned(), kind, rng(&v), Vec::new()))
                })
                .collect();
            out.push(symbol(
                section.key.to_owned(),
                SymbolKind::MODULE,
                rng(&sec_val),
                children,
            ));
        }
    }

    if let Some(tags) = root.get("tags") {
        for tag in tags.items() {
            let name = tag.get("name").and_then(|n| n.as_str()).unwrap_or("tag");
            out.push(symbol(name.to_owned(), SymbolKind::VARIABLE, rng(&tag), Vec::new()));
        }
    }
    out
}

/// Arazzo 1.0 symbols: one function symbol per workflow with its steps
/// (by `stepId`) nested as children.
fn arazzo_symbols(low: &LowDoc) -> Vec<DocumentSymbol> {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    let rng = |n: &NodeRef<'_>| lsp_range(bytes, li, n.byte_range());
    let doc = ArazzoDoc::new(low);
    doc.workflows()
        .iter()
        .map(|w| {
            let steps = w
                .steps()
                .iter()
                .map(|s| {
                    symbol(s.step_id.to_owned(), SymbolKind::VARIABLE, rng(&s.node()), Vec::new())
                })
                .collect();
            symbol(w.workflow_id.to_owned(), SymbolKind::FUNCTION, rng(&w.node()), steps)
        })
        .collect()
}

/// Overlay 1.0 symbols: one symbol per action, named by its `target`
/// (falling back to `action`) and disambiguated with its index.
fn overlay_symbols(low: &LowDoc) -> Vec<DocumentSymbol> {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    let rng = |n: &NodeRef<'_>| lsp_range(bytes, li, n.byte_range());
    let Some(actions) = low.root().get("actions") else { return Vec::new() };
    actions
        .items()
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let name = action
                .get("target")
                .and_then(|t| t.as_str())
                .unwrap_or("action")
                .to_owned();
            symbol(format!("{name} ({i})"), SymbolKind::VARIABLE, rng(action), Vec::new())
        })
        .collect()
}

/// Folding ranges for top-level mappings/sequences spanning more than 3
/// lines.
#[must_use]
pub fn folding_ranges(low: &LowDoc) -> Vec<FoldingRange> {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    low.inner()
        .root()
        .descendants()
        .filter(|n| matches!(n.kind(), SyntaxKind::Mapping | SyntaxKind::Sequence))
        .filter_map(|n| {
            let r = n.byte_range();
            let (start_line, _) = li.line_col_utf16(bytes, r.start);
            let (end_line, _) = li.line_col_utf16(bytes, r.end);
            (end_line.saturating_sub(start_line) > 3).then_some(FoldingRange {
                start_line,
                start_character: None,
                end_line,
                end_character: None,
                kind: None,
                collapsed_text: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::SymbolKind;

    fn low_of(text: &str) -> LowDoc {
        let uri = suspect_source::Uri::parse("file:///mem/doc.yaml").unwrap();
        LowDoc::parse(uri, suspect_source::Source::from_vec(text.as_bytes().to_vec()))
    }

    fn find<'a>(syms: &'a [DocumentSymbol], name: &str) -> &'a DocumentSymbol {
        syms.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("missing {name}: {syms:?}"))
    }

    #[test]
    fn oas_symbols_paths_components_tags() {
        let text = r#"
openapi: 3.1.0
tags:
  - name: pets
paths:
  /pets:
    get:
      operationId: listPets
    post:
      operationId: createPet
components:
  schemas:
    Pet:
      type: object
    PetList:
      type: array
  responses:
    Err:
      description: e
"#;
        let low = low_of(text);
        let syms = document_symbols(&low);
        let get = find(&syms, "GET /pets");
        assert_eq!(get.kind, SymbolKind::METHOD);
        assert!(find(&syms, "POST /pets").range.start >= get.range.start);
        let schemas = find(&syms, "schemas");
        assert_eq!(schemas.kind, SymbolKind::MODULE);
        let children = schemas.children.as_ref().unwrap();
        let pet = children.iter().find(|s| s.name == "Pet").unwrap();
        assert_eq!(pet.kind, SymbolKind::CLASS);
        let responses = find(&syms, "responses");
        assert_eq!(responses.kind, SymbolKind::MODULE);
        let err = responses.children.as_ref().unwrap().iter().find(|s| s.name == "Err").unwrap();
        assert_eq!(err.kind, SymbolKind::VARIABLE);
        assert_eq!(find(&syms, "pets").kind, SymbolKind::VARIABLE);
    }

    #[test]
    fn arazzo_symbols_workflows_and_steps() {
        let text = r#"
arazzo: 1.0.0
info:
  title: T
sourceDescriptions:
  - name: api
    url: openapi.yaml
workflows:
  - workflowId: login
    steps:
      - stepId: get-token
  - workflowId: logout
    steps:
      - stepId: drop-token
"#;
        let low = low_of(text);
        let syms = document_symbols(&low);
        let login = find(&syms, "login");
        assert_eq!(login.kind, SymbolKind::FUNCTION);
        let steps = login.children.as_ref().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].name, "get-token");
        assert_eq!(steps[0].kind, SymbolKind::VARIABLE);
        assert!(find(&syms, "logout").range.start > login.range.start);
    }

    #[test]
    fn overlay_symbols_actions() {
        let text = r#"
overlay: 1.0.0
info:
  title: T
actions:
  - target: $.info.title
    update: Renamed
  - target: $.paths
    remove: true
"#;
        let low = low_of(text);
        let syms = document_symbols(&low);
        assert_eq!(syms.len(), 2);
        assert!(syms[0].name.starts_with("$.info.title"));
        assert_eq!(syms[0].kind, SymbolKind::VARIABLE);
    }

    #[test]
    fn unknown_family_yields_no_symbols() {
        let low = low_of("just: a map\n");
        assert!(document_symbols(&low).is_empty());
    }

    #[test]
    fn folding_ranges_for_deep_containers() {
        let text = "a:\n  b:\n    c: 1\n    d: 2\n    e: 3\nsmall: 1\n";
        let low = low_of(text);
        let ranges = folding_ranges(&low);
        // Only the root mapping spans >3 lines (0-6 incl. trailing
        // newline); the `a` mapping spans exactly 3 lines (1-4).
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(ranges[0].start_line, 0);
        assert_eq!(ranges[0].end_line, 6);
    }
}
