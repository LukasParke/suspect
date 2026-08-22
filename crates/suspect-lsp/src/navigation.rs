//! Position-based navigation: node lookup, `$ref` go-to-definition,
//! reverse reference lookup, and hover rendering.

use suspect_low::{NodeRef, Pointer, ValueKind};
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
    let inner = low.inner();
    let root = inner.root();
    // Two O(log n) probes bracket the cursor: `[offset, offset+1]` reaches
    // nodes extending past it, `[offset-1, offset]` reaches nodes ending at
    // it. Every node whose inclusive range contains `offset` spans one of
    // the two, so the smaller meaningful survivor equals the result of a
    // smallest-containing-node scan — without walking the whole tree.
    fn probe<'d>(
        inner: &'d suspect_syntax::SourceDoc,
        root: &SNode<'d>,
        start: usize,
        end: usize,
    ) -> Option<SNode<'d>> {
        let mut cur = SNode::new(inner, root.raw().descendant_for_byte_range(start, end)?);
        while matches!(cur.kind(), SyntaxKind::Comment | SyntaxKind::Directive) || cur.is_error() {
            cur = cur.parent()?;
        }
        Some(cur)
    }
    let past = probe(inner, &root, offset, offset.saturating_add(1));
    let ending = probe(inner, &root, offset.saturating_sub(1), offset);
    match (past, ending) {
        (Some(a), Some(b)) => {
            let (a_len, b_len) = (a.byte_range().len(), b.byte_range().len());
            Some(if a_len <= b_len { a } else { b })
        }
        (a, b) => a.or(b),
    }
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
pub(crate) fn rederive<'ws>(
    handle: &DocHandle<'ws>,
    range: std::ops::Range<usize>,
) -> Option<NodeRef<'ws>> {
    let inner = handle.doc().inner();
    let mut raw = inner
        .root()
        .raw()
        .descendant_for_byte_range(range.start, range.end.saturating_sub(1).max(range.start))?;
    while raw.byte_range() != range {
        raw = raw.parent()?;
    }
    Some(NodeRef::new(SNode::new(inner, raw)))
}

/// Resolves the `$ref` value at `offset`, following chains through the
/// workspace. Returns the final target's location.
///
/// The entry point (ref value and containing pointer) is derived from the
/// *live* document, so unsaved edits are respected. Targets inside the live
/// document are reported in buffer coordinates; foreign-file targets stay
/// in workspace coordinates.
#[must_use]
pub fn goto_definition(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    offset: usize,
) -> Option<Definition> {
    let refv = ref_value_node(low, offset)?;
    let handle = ws.get(low.uri())?;
    let resolution = resolve_live_ref(&handle, &refv)?;
    definition_from(ws, low, resolution)
}

/// Maps a key-side node to its owning pair's value; `path_from_root` only
/// resolves through mapping *values*, so pointer math must anchor there.
pub(crate) fn value_anchor<'d>(node: SNode<'d>) -> SNode<'d> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair {
            if let Some(key) = n.child_by_field("key") {
                let (kr, nr) = (key.byte_range(), node.byte_range());
                if kr.start <= nr.start
                    && nr.end <= kr.end
                    && let Some(v) = n.child_by_field("value")
                {
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
    let named = Pointer::from_tokens(vec![
        tokens[0].clone(),
        tokens[1].clone(),
        tokens[2].clone(),
    ]);
    let value = low.root().pointer(&named)?;
    Some(Definition {
        uri: low.uri().clone(),
        range: value.byte_range(),
    })
}

/// Finds all `$ref` edges across the workspace whose parsed pointer targets
/// the node at `offset` (identified by its value-anchor-derived
/// `path_from_root`). Local (`#/...`) and external (`file.yaml#/...`)
/// pointers into this document both match; plain-name refs never do.
///
/// Edges of the open document are scanned fresh from the live buffer, so
/// unsaved edits participate immediately; other documents match their
/// workspace scans. With `include_declaration`, the node itself is returned
/// first as the declaration. Matches are returned in workspace URI order.
#[must_use]
pub fn references(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    offset: usize,
    include_declaration: bool,
) -> Vec<Definition> {
    let Some(node) = node_at(low, offset) else {
        return Vec::new();
    };
    let ptr = NodeRef::new(value_anchor(node)).path_from_root();
    let doc_uri = low.uri().clone();
    let mut out = Vec::new();
    if include_declaration {
        out.push(Definition {
            uri: doc_uri.clone(),
            range: node.byte_range(),
        });
    }
    for uri in ws.uris() {
        if uri == doc_uri {
            // The open buffer shadows the disk copy behind the workspace:
            // rescan this document's own edges from live text.
            for edge in live_edges(low) {
                let hit = match &edge.parsed {
                    ParsedRef::Local(p) => p == &ptr,
                    ParsedRef::External {
                        uri: target,
                        pointer,
                    } => target == &doc_uri && pointer == &ptr,
                    ParsedRef::PlainName(_) => false,
                };
                if hit {
                    out.push(Definition {
                        uri: uri.clone(),
                        range: edge.at,
                    });
                }
            }
        } else if let Some(h) = ws.get(&uri) {
            for edge in h.edges().iter() {
                let hit = match &edge.parsed {
                    ParsedRef::External {
                        uri: target,
                        pointer,
                    } => target == &doc_uri && pointer == &ptr,
                    ParsedRef::Local(_) | ParsedRef::PlainName(_) => false,
                };
                if hit {
                    out.push(Definition {
                        uri: uri.clone(),
                        range: edge.at.clone(),
                    });
                }
            }
        }
    }
    out
}

/// One `$ref` occurrence discovered in the live buffer.
struct LiveEdge {
    /// Byte range of the `$ref` value node (the string).
    at: std::ops::Range<usize>,
    /// Parsed address: local pointer, external URI plus pointer, or plain
    /// name.
    parsed: ParsedRef,
}

/// Scans the open document for `$ref` edges so reverse lookup reflects
/// unsaved edits. Mirrors `suspect-ref`'s workspace scan: a stack walk over
/// the semantic tree with alias/merge-key expansion, duplicate expansions
/// collapsed by byte range, and an on-path set guarding self-referential
/// alias cycles.
fn live_edges(low: &suspect_low::LowDoc) -> Vec<LiveEdge> {
    struct Frame<'d> {
        ptr: Pointer,
        children: Vec<(Option<Box<str>>, NodeRef<'d>, usize)>,
        next: usize,
    }
    fn children_of<'d>(node: NodeRef<'d>) -> Vec<(Option<Box<str>>, NodeRef<'d>, usize)> {
        match node.kind() {
            ValueKind::Object => node
                .entries()
                .into_iter()
                .filter_map(|e| e.value.map(|v| (Some(Box::from(e.key)), v, 0usize)))
                .collect(),
            ValueKind::Array => node
                .items()
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, i))
                .collect(),
            _ => Vec::new(),
        }
    }

    let root = low.root();
    let mut out = Vec::new();
    let mut seen: Vec<std::ops::Range<usize>> = Vec::new();
    let mut on_path: Vec<std::ops::Range<usize>> = vec![root.byte_range()];
    let mut stack: Vec<Frame<'_>> = vec![Frame {
        ptr: Pointer::root(),
        children: children_of(root),
        next: 0,
    }];
    while let Some(frame) = stack.last_mut() {
        let Some((key, child, index)) = frame.children.get(frame.next).cloned() else {
            stack.pop();
            on_path.pop();
            continue;
        };
        frame.next += 1;
        let child_ptr = match key.as_deref() {
            Some(k) => frame.ptr.push(k), // entry keys are already unescaped
            None => frame.ptr.push(&index.to_string()),
        };
        if child.kind() == ValueKind::Str
            && key.as_deref() == Some("$ref")
            && !seen.contains(&child.byte_range())
        {
            seen.push(child.byte_range());
            // Block scalars must be decoded, not read as raw source slices.
            let decoded = child.decoded_scalar();
            if let Ok(raw) = std::str::from_utf8(&decoded).map(str::trim)
                && let Some(parsed) = parse_live_ref(low.uri(), raw)
            {
                out.push(LiveEdge {
                    at: child.byte_range(),
                    parsed,
                });
            }
        }
        // Descend only into containers not already on the walk path
        // (alias-cycle guard).
        if matches!(child.kind(), ValueKind::Object | ValueKind::Array) {
            let range = child.byte_range();
            if on_path.contains(&range) {
                continue;
            }
            on_path.push(range);
            stack.push(Frame {
                ptr: child_ptr,
                children: children_of(child),
                next: 0,
            });
        }
    }
    out
}

/// Parses a raw `$ref` string against a base document URI, mirroring
/// `suspect-ref`'s parser: fragment-only refs are local, anything with a
/// document part joins against the base, and fragments percent-decode
/// before RFC 6901 parsing. Unparseable refs yield `None` (never a match).
fn parse_live_ref(base: &Uri, raw: &str) -> Option<ParsedRef> {
    /// Percent-decodes a fragment body to UTF-8 (`None` when invalid).
    fn decode(frag: &str) -> Option<String> {
        String::from_utf8(suspect_low::percent_decode_fragment(frag)).ok()
    }
    let (doc_part, frag) = Uri::split_ref(raw);
    Some(match doc_part {
        None => match frag {
            "" => ParsedRef::Local(Pointer::root()),
            f if f.starts_with('/') => ParsedRef::Local(Pointer::parse(&decode(f)?).ok()?),
            f => ParsedRef::PlainName(decode(f)?.into_boxed_str()),
        },
        Some(doc) => {
            let uri = base.join(doc).ok()?;
            match frag {
                "" => ParsedRef::External {
                    uri,
                    pointer: Pointer::root(),
                },
                f if f.starts_with('/') => ParsedRef::External {
                    uri,
                    pointer: Pointer::parse(&decode(f)?).ok()?,
                },
                f => ParsedRef::PlainName(decode(f)?.into_boxed_str()),
            }
        }
    })
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
///
/// Same-document targets are excerpted from the live buffer, so hover shows
/// unsaved edits; foreign-file targets come from the workspace copies.
#[must_use]
pub fn hover_markdown(ws: &Workspace, low: &suspect_low::LowDoc, offset: usize) -> Option<String> {
    if let Some(refv) = ref_value_node(low, offset) {
        let handle = ws.get(low.uri())?;
        if let Some(Resolution::Node(target)) = resolve_live_ref(&handle, &refv) {
            let tdoc = target.syntax().doc();
            if *tdoc.uri() == *low.uri() {
                // Re-express the target against the buffer and excerpt the
                // live bytes, not the disk copy.
                let ptr = target.path_from_root();
                let node = low.root().pointer(&ptr)?;
                return Some(format!(
                    "```{}\n{}\n```",
                    lang_tag(low.inner().format()),
                    excerpt(low.inner().bytes(), node.byte_range(), 40)
                ));
            }
            return Some(format!(
                "```{}\n{}\n```",
                lang_tag(tdoc.format()),
                excerpt(tdoc.bytes(), target.byte_range(), 40)
            ));
        }
    }
    let node = node_at(low, offset)?;
    let semantic = NodeRef::new(node);
    let first_line = excerpt(node.doc().bytes(), node.byte_range(), 1);
    Some(format!("`{:?}`\n\n```\n{first_line}\n```", semantic.kind()))
}

/// Code-fence language tag for a document format.
fn lang_tag(format: suspect_syntax::Format) -> &'static str {
    match format {
        suspect_syntax::Format::Json => "json",
        suspect_syntax::Format::Yaml => "yaml",
    }
}

/// Resolves the `$ref` chain whose value lives at `refv` in the live
/// document. Structural pointers are version-independent, so the chain runs
/// through the workspace engine even though the entry point comes from
/// possibly-dirty buffer text.
fn resolve_live_ref<'ws>(handle: &DocHandle<'ws>, refv: &SNode<'_>) -> Option<Resolution<'ws>> {
    // The value node's pointer is one token deeper than the containing
    // mapping that carries the `$ref` key.
    let containing = NodeRef::new(*refv)
        .path_from_root()
        .parent()
        .unwrap_or_default();
    if containing.is_root() {
        // A `$ref` mapping at the document root must hop off the root;
        // `resolve_pointer` short-circuits root pointers to `WholeDoc`, so
        // resolve through the disk-backed value node as before.
        let node = rederive(handle, refv.byte_range())?;
        return handle.resolve_ref_value(node).ok();
    }
    // Chain hops read `$ref` values from the workspace copy, so proceed
    // only when the disk text still carries the same ref at this pointer —
    // an edited-but-unsaved ref value has no sound resolution yet.
    let disk_ref = handle
        .doc()
        .root()
        .pointer(&containing)?
        .get("$ref")?
        .decoded_scalar();
    let live_ref = NodeRef::new(*refv).decoded_scalar();
    if disk_ref.trim_ascii() != live_ref.trim_ascii() {
        return None;
    }
    handle.resolve_pointer(handle.id(), &containing).ok()
}

/// Converts a workspace resolution into a [`Definition`]. Targets inside
/// the live document are re-expressed against the buffer via their
/// structural pointer, so ranges survive unsaved edits; foreign targets
/// keep their workspace coordinates.
fn definition_from(
    ws: &Workspace,
    low: &suspect_low::LowDoc,
    resolution: Resolution<'_>,
) -> Option<Definition> {
    match resolution {
        Resolution::Node(target) => {
            let uri = target.syntax().doc().uri().clone();
            let range = if uri == *low.uri() {
                let ptr = target.path_from_root();
                low.root().pointer(&ptr)?.byte_range()
            } else {
                target.byte_range()
            };
            Some(Definition { uri, range })
        }
        Resolution::WholeDoc(id) => {
            let uri = ws
                .uris()
                .into_iter()
                .find(|u| ws.get(u).is_some_and(|h| h.id() == id))?;
            Some(Definition { uri, range: 0..0 })
        }
        Resolution::Cycle { .. } => None,
    }
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
        suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
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
        let target_uri = ws
            .uris()
            .into_iter()
            .find(|u| u.as_str().ends_with("schemas.yaml"))
            .unwrap();
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
        assert!(
            refs.iter()
                .any(|r| r.uri == *low.uri() && r.range.len() == 3)
        );
        let items_ref = low
            .root()
            .pointer(&Pointer::parse("/components/schemas/PetList/items/$ref").unwrap())
            .unwrap();
        assert!(
            refs.iter()
                .any(|r| r.uri == *low.uri() && r.range == items_ref.byte_range())
        );
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

    /// Parses `text` WITHOUT writing it to disk, simulating an unsaved
    /// buffer over the workspace copy of `name`.
    fn low_buffer(dir: &std::path::Path, name: &str, text: &str) -> suspect_low::LowDoc {
        let uri = Uri::from_path(&dir.join(name)).unwrap();
        suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    #[test]
    fn node_at_includes_tokens_ending_at_cursor() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-nodeat-edge");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "a:\n  b: cat\n";
        let low = low_at(&dir, "n.yaml", text);
        // Cursor immediately after the scalar: closed-interval containment
        // must still select the scalar, not an enclosing container.
        let off = text.find("cat").unwrap() + "cat".len();
        let node = node_at(&low, off).unwrap();
        assert_eq!(node.scalar_bytes(), b"cat");
    }

    #[test]
    fn goto_definition_tracks_unsaved_edits() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-dirty-def");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        // Buffer inserts two lines above the target; every byte after the
        // insertion shifts relative to the disk copy the workspace holds.
        let dirty = MAIN.replace("paths:", "paths:\n  x:\n    y: z");
        assert_ne!(dirty, MAIN);
        let low = low_buffer(&dir, "main.yaml", &dirty);
        let off = offset_in(&dirty, "#/components/schemas/Pet");
        let def = goto_definition(&ws, &low, off).expect("resolves against buffer");
        assert_eq!(def.uri, *low.uri());
        let expected = low
            .root()
            .pointer(&Pointer::parse("/components/schemas/Pet").unwrap())
            .unwrap()
            .byte_range();
        assert_eq!(def.range, expected);
        // The disk-backed copy would have produced the stale range.
        let stale = ws
            .get(low.uri())
            .unwrap()
            .doc()
            .root()
            .pointer(&Pointer::parse("/components/schemas/Pet").unwrap())
            .unwrap()
            .byte_range();
        assert_ne!(def.range, stale);
    }

    #[test]
    fn references_see_unsaved_edges() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-dirty-refs");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        // A second local $ref to Pet exists only in the buffer.
        let needle = "$ref: '#/components/schemas/Pet'";
        let extra =
            "\n              annotations:\n                $ref: '#/components/schemas/Pet'";
        let dirty = MAIN.replacen(needle, &format!("{needle}{extra}"), 1);
        assert_ne!(dirty, MAIN);
        let low = low_buffer(&dir, "main.yaml", &dirty);
        let off = dirty.rfind("Pet:").unwrap() + 1;
        let refs = references(&ws, &low, off, true);
        // Declaration + the on-disk edge + the buffer-only edge.
        assert_eq!(refs.len(), 3, "got {refs:?}");
        let expected = low
            .root()
            .pointer(
                &Pointer::parse(
                    "/paths/~1pets/get/responses/200/content/application~1json/annotations/$ref",
                )
                .unwrap(),
            )
            .unwrap()
            .byte_range();
        assert!(
            refs.iter()
                .any(|r| r.uri == *low.uri() && r.range == expected),
            "got {refs:?}"
        );
    }

    #[test]
    fn hover_excerpts_unsaved_buffer() {
        let dir = std::env::temp_dir().join("suspect-lsp-nav-dirty-hover");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        // Pet's type is edited in the buffer only; hovering the local $ref
        // must excerpt the edited source at buffer coordinates.
        let dirty = MAIN.replace("      type: object", "      type: string");
        assert_ne!(dirty, MAIN);
        let low = low_buffer(&dir, "main.yaml", &dirty);
        let md = hover_markdown(&ws, &low, offset_in(&dirty, "#/components/schemas/Pet"))
            .expect("hover resolves");
        assert!(md.contains("type: string"), "{md}");
        assert!(!md.contains("type: object"), "{md}");
    }
}
