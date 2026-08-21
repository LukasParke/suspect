//! Position-based navigation: node lookup, `$ref` go-to-definition,
//! reverse reference lookup, and hover rendering.

use suspect_low::{NodeRef, Pointer};
use suspect_ref::{DocHandle, ParsedRef, Resolution, Workspace};
use suspect_source::Uri;
use suspect_syntax::{SNode, SyntaxKind};

/// A resolved target location: document URI plus byte range inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// Document containing the target.
    pub uri: Uri,
    /// Byte range of the target inside that document.
    pub range: std::ops::Range<usize>,
}

/// Smallest meaningful node whose byte range contains `offset`.
#[must_use]
pub fn node_at<'d>(low: &'d suspect_low::LowDoc, offset: usize) -> Option<SNode<'d>> {
    let root = low.inner().root();
    let mut best: Option<SNode<'d>> = None;
    for n in root.descendants() {
        if matches!(n.kind(), SyntaxKind::Comment | SyntaxKind::Directive) || n.is_error() {
            continue;
        }
        let r = n.byte_range();
        if r.start <= offset && offset <= r.end {
            // Pre-order walk: prefer later (deeper) nodes on span ties.
            if best.is_none_or(|b| r.len() <= b.byte_range().len()) {
                best = Some(n);
            }
        }
    }
    best
}

/// If the node at `offset` sits inside the *value* of a `$ref` mapping
/// entry, returns that value node's content.
#[must_use]
pub fn ref_value_node<'d>(low: &'d suspect_low::LowDoc, offset: usize) -> Option<SNode<'d>> {
    let node = node_at(low, offset)?;
    let mut cur = node;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            let key = cur.child_by_field("key")?;
            if key.scalar_bytes() == b"$ref" {
                let value = cur.child_by_field("value")?;
                let (vr, nr) = (value.byte_range(), node.byte_range());
                return if vr.start <= nr.start && nr.end <= vr.end {
                    Some(value.content())
                } else {
                    None
                };
            }
        }
        cur = cur.parent()?;
    }
}

/// Re-derives a node at `range` from a workspace-borrowed document so it can
/// be handed to resolution APIs that require the workspace lifetime.
fn rederive<'ws>(handle: &DocHandle<'ws>, range: std::ops::Range<usize>) -> Option<NodeRef<'ws>> {
    let inner = handle.doc().inner();
    let mut raw = inner.root().raw().descendant_for_byte_range(
        range.start,
        range.end.saturating_sub(1).max(range.start),
    )?;
    while raw.byte_range() != range {
        raw = raw.parent()?;
    }
    Some(NodeRef::new(SNode::new(inner, raw)))
}

/// Resolves the `$ref` value at `offset`, following chains through the
/// workspace. Returns the final target's location.
#[must_use]
pub fn goto_definition(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    offset: usize,
) -> Option<Definition> {
    let refv = ref_value_node(low, offset)?;
    let handle = ws.get(low.uri())?;
    let node = rederive(&handle, refv.byte_range())?;
    match handle.resolve_ref_value(node) {
        Ok(Resolution::Node(target)) => Some(Definition {
            uri: target.syntax().doc().uri().clone(),
            range: target.byte_range(),
        }),
        Ok(Resolution::WholeDoc(id)) => {
            let uri = ws.uris().into_iter().find(|u| ws.get(u).is_some_and(|h| h.id() == id))?;
            Some(Definition { uri, range: 0..0 })
        }
        Ok(Resolution::Cycle { .. }) | Err(_) => None,
    }
}

/// Maps a key-side node to its owning pair's value; `path_from_root` only
/// resolves through mapping *values*, so pointer math must anchor there.
fn value_anchor<'d>(node: SNode<'d>) -> SNode<'d> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair {
            if let Some(key) = n.child_by_field("key") {
                let (kr, nr) = (key.byte_range(), node.byte_range());
                if kr.start <= nr.start && nr.end <= kr.end
                    && let Some(v) = n.child_by_field("value") {
                        return v.content();
                    }
            }
            break;
        }
        cur = n.parent();
    }
    node
}

/// Fallback for non-`$ref` positions: when the node lives under
/// `/components/<section>/<Name>`, links to that named component itself.
#[must_use]
pub fn self_definition(low: &suspect_low::LowDoc, offset: usize) -> Option<Definition> {
    let node = node_at(low, offset)?;
    let ptr = NodeRef::new(value_anchor(node)).path_from_root();
    let tokens = ptr.tokens();
    if tokens.len() < 3 || tokens[0].as_ref() != "components" {
        return None;
    }
    let named =
        Pointer::from_tokens(vec![tokens[0].clone(), tokens[1].clone(), tokens[2].clone()]);
    let value = low.root().pointer(&named)?;
    Some(Definition { uri: low.uri().clone(), range: value.byte_range() })
}

/// Finds all `$ref` edges across the workspace whose parsed pointer targets
/// the node at `offset` (identified by its [`value_anchor`]-derived
/// `path_from_root`). Local (`#/...`) and external (`file.yaml#/...`)
/// pointers into this document both match; plain-name refs never do.
///
/// With `include_declaration`, the node itself is returned first as the
/// declaration. Matches are returned in workspace URI order.
#[must_use]
pub fn references(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    offset: usize,
    include_declaration: bool,
) -> Vec<Definition> {
    let Some(node) = node_at(low, offset) else { return Vec::new() };
    let ptr = NodeRef::new(value_anchor(node)).path_from_root();
    let doc_uri = low.uri();
    let mut out = Vec::new();
    if include_declaration {
        out.push(Definition { uri: doc_uri.clone(), range: node.byte_range() });
    }
    for uri in ws.uris() {
        let Some(h) = ws.get(&uri) else { continue };
        for edge in h.edges().iter() {
            let hit = match &edge.parsed {
                ParsedRef::Local(p) => uri == *doc_uri && p == &ptr,
                ParsedRef::External { uri: target, pointer } => target == doc_uri && pointer == &ptr,
                ParsedRef::PlainName(_) => false,
            };
            if hit {
                out.push(Definition { uri: uri.clone(), range: edge.at.clone() });
            }
        }
    }
    out
}

/// Source excerpt of `range`, truncated to `max_lines` lines with a `...`
/// marker when truncated.
#[must_use]
pub fn excerpt(bytes: &[u8], range: std::ops::Range<usize>, max_lines: usize) -> String {
    let end = range.end.min(bytes.len());
    let text = String::from_utf8_lossy(&bytes[range.start.min(end)..end]);
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        lines.push("...");
    }
    lines.join("\n")
}

/// Hover markdown: for `$ref` values a fenced block of the target's source
/// (truncated to 40 lines); otherwise the node kind plus its first line.
#[must_use]
pub fn hover_markdown(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    offset: usize,
) -> Option<String> {
    if let Some(refv) = ref_value_node(low, offset) {
        let handle = ws.get(low.uri())?;
        if let Some(node) = rederive(&handle, refv.byte_range())
            && let Ok(Resolution::Node(target)) = handle.resolve_ref_value(node) {
                let tdoc = target.syntax().doc();
                let lang = match tdoc.format() {
                    suspect_syntax::Format::Json => "json",
                    suspect_syntax::Format::Yaml => "yaml",
                };
                return Some(format!(
                    "```{lang}\n{}\n```",
                    excerpt(tdoc.bytes(), target.byte_range(), 40)
                ));
            }
    }
    let node = node_at(low, offset)?;
    let semantic = NodeRef::new(node);
    let first_line = excerpt(node.doc().bytes(), node.byte_range(), 1);
    Some(format!(
        "`{:?}`\n\n```\n{first_line}\n```",
        semantic.kind()
    ))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_low::Pointer;
    use suspect_ref::{Workspace, WorkspaceBuilder};

    const MAIN: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        name:
          type: string
"#;

    const SCHEMAS: &str = r#"
components:
  schemas:
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
    Pet:
      type: object
"#;

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::write(dir.join("main.yaml"), MAIN).unwrap();
        std::fs::write(dir.join("schemas.yaml"), SCHEMAS).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        Arc::new(ws)
    }

    /// Parses `text` under the real file URI so workspace lookups match.
    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> suspect_low::LowDoc {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let uri = Uri::from_path(&path).unwrap();
        suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(text.as_bytes().to_vec()))
    }

    fn offset_in(text: &str, needle: &str) -> usize {
        let at = text.find(needle).expect("needle present");
        at + needle.len() / 2
    }

    #[test]
    fn node_at_finds_smallest_containing_node() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-nodeat");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "a:\n  b: cat\n";
        let low = low_at(&dir, "n.yaml", text);
        let off = offset_in(text, "cat");
        let node = node_at(&low, off).unwrap();
        assert_eq!(node.scalar_bytes(), b"cat");
    }

    #[test]
    fn goto_definition_same_document() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-same");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let off = offset_in(MAIN, "#/components/schemas/Pet");
        let def = goto_definition(&ws, &low, off).expect("resolves");
        assert_eq!(def.uri, *low.uri());
        let expected = low
            .root()
            .pointer(&Pointer::parse("/components/schemas/Pet").unwrap())
            .unwrap()
            .byte_range();
        assert_eq!(def.range, expected);
    }

    #[test]
    fn goto_definition_cross_file() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-cross");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        // A doc whose ref points into schemas.yaml.
        let text = "allOf:\n  - $ref: 'schemas.yaml#/components/schemas/PetList'\n";
        let low = low_at(&dir, "entry.yaml", text);
        ws.load_all("entry.yaml").unwrap();
        let off = offset_in(text, "PetList");
        let def = goto_definition(&ws, &low, off).expect("resolves");
        let target_uri =
            ws.uris().into_iter().find(|u| u.as_str().ends_with("schemas.yaml")).unwrap();
        assert_eq!(def.uri, target_uri);
        let handle = ws.get(&target_uri).unwrap();
        let expected = handle
            .doc()
            .root()
            .pointer(&Pointer::parse("/components/schemas/PetList").unwrap())
            .unwrap()
            .byte_range();
        assert_eq!(def.range, expected);
    }

    #[test]
    fn self_definition_links_named_component() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-self");
        std::fs::create_dir_all(&dir).unwrap();
        let low = low_at(&dir, "main.yaml", MAIN);
        // On the `name` key inside the Pet schema body.
        let off = offset_in(MAIN, "name:");
        let def = self_definition(&low, off).expect("self link");
        assert_eq!(def.uri, *low.uri());
        let expected = low
            .root()
            .pointer(&Pointer::parse("/components/schemas/Pet").unwrap())
            .unwrap()
            .byte_range();
        assert_eq!(def.range, expected);
    }

    #[test]
    fn references_reverse_lookup() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-refs");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "schemas.yaml", SCHEMAS);
        // Position on the Pet definition's *key*; the reverse lookup must
        // anchor through the pair's value side.
        let off = SCHEMAS.rfind("Pet:").unwrap() + 1;
        ws.load_all("schemas.yaml").unwrap();
        let refs = references(&ws, &low, off, true);
        // Declaration (the 3-byte `Pet` key) plus the edge whose location is
        // the $ref value string inside PetList.items. main.yaml's $ref is
        // local to main.yaml and does not target this document.
        assert_eq!(refs.len(), 2, "got {refs:?}");
        assert!(refs.iter().any(|r| r.uri == *low.uri() && r.range.len() == 3));
        let items_ref = low
            .root()
            .pointer(&Pointer::parse("/components/schemas/PetList/items/$ref").unwrap())
            .unwrap();
        assert!(refs.iter().any(|r| r.uri == *low.uri() && r.range == items_ref.byte_range()));
    }

    #[test]
    fn hover_excerpt_truncates_to_40_lines() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-hover");
        std::fs::create_dir_all(&dir).unwrap();
        let mut target = String::from("components:\n  schemas:\n    Big:\n");
        for i in 0..60 {
            target.push_str(&format!("      k{i}: v{i}\n"));
        }
        std::fs::write(dir.join("big.yaml"), &target).unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("big.yaml").unwrap();
        let text = "schema:\n  $ref: 'big.yaml#/components/schemas/Big'\n";
        let low = low_at(&dir, "h.yaml", text);
        ws.load_all("h.yaml").unwrap();
        let md = hover_markdown(&ws, &low, offset_in(text, "Big")).unwrap();
        assert!(md.starts_with("```yaml\n"), "{md}");
        assert!(md.ends_with("\n```"));
        let body = md.trim_start_matches("```yaml\n").trim_end_matches("\n```");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 41, "40 lines plus ... marker");
        assert_eq!(lines[40], "...");
    }

    #[test]
    fn hover_non_ref_shows_kind_and_first_line() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-hover2");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let md = hover_markdown(&ws, &low, offset_in(MAIN, "object")).expect("hover");
        let body = md.trim_start_matches('`').trim_end_matches('`');
        assert!(body.contains("object"), "first line shown: {md}");
    }
}
