//! Type hierarchy over schema composition (`allOf`/`anyOf`/`oneOf`) plus
//! selection-scoped canonical range formatting.
//!
//! Two independent concerns share this module because neither warrants its
//! own backend surface:
//!
//! * **Type hierarchy** — OpenAPI expresses inheritance through composition:
//!   a schema listing another schema in its `allOf` extends it.
//!   [`prepare_type_hierarchy`] anchors the hierarchy on a component schema
//!   (directly, or through the `$ref` under the cursor); [`supertypes`] walks
//!   one level up the composition axis and [`subtypes`] walks one level down,
//!   including children declared in the parent's `discriminator.mapping`.
//!   Items carry `{uri, ptr}` JSON data so every returned item can be
//!   re-resolved by follow-up requests; a visited set keyed by `(uri, ptr)`
//!   dedups cycles.
//!
//! * **Range formatting** — canonicalizes only the lines intersecting the
//!   selection: block mappings and sequences fully contained in the range are
//!   re-indented from tree depth (two spaces per level, `- ` markers two
//!   spaces past their parent key), single-quoted scalars whose content is
//!   safe as plain YAML are unquoted, and comments plus blank lines pass
//!   through untouched. Output is minimal: only lines whose emitted form
//!   differs produce a [`TextEdit`].

use std::collections::HashSet;

use suspect_low::{NodeRef, Pointer, ValueKind};
use suspect_ref::{DocHandle, RefError, Resolution, Workspace};
use suspect_source::{LineIndex, Uri};
use suspect_syntax::{Format, SNode, ScalarStyle, SyntaxKind};
use tower_lsp::lsp_types::{SymbolKind, TextEdit, TypeHierarchyItem, Url};

use crate::navigation;
use crate::state::{OpenDoc, lsp_range};

/// Composition keywords that express schema inheritance.
const COMPOSITION_KEYS: [&str; 3] = ["allOf", "anyOf", "oneOf"];

/// Pointer to the named-schema table inside any loaded document.
const SCHEMAS_PTR: &str = "/components/schemas";

/// String spellings that must stay quoted because plain form would read as a
/// boolean/null-ish literal under permissive YAML consumers. Compared
/// case-insensitively.
const QUOTE_REQUIRED: [&str; 11] = [
    "y", "yes", "no", "n", "on", "off", "none", "null", "true", "false", "~",
];

// ---------------------------------------------------------------------------
// Part 1: type hierarchy
// ---------------------------------------------------------------------------

/// Prepares a type hierarchy rooted at the component schema under `offset`.
///
/// The cursor qualifies when it sits anywhere inside a named entry of
/// `/components/schemas` (the name key or the schema body — pointer tokens
/// are truncated to the component path), or on a `$ref` value resolving to
/// such an entry. The returned item's `data` carries `{uri, ptr}` so
/// [`supertypes`]/[`subtypes`] can re-resolve it; ranges are reported in the
/// workspace copy's coordinates.
#[must_use]
pub fn prepare_type_hierarchy(
    ws: &Workspace,
    doc: &OpenDoc,
    offset: usize,
) -> Option<Vec<TypeHierarchyItem>> {
    let low = &doc.low;

    // `$ref` path: cursor inside a reference value targeting a component.
    if let Some(refv) = navigation::ref_value_node(low, offset)
        && let Some(handle) = ws.get(low.uri())
        && let Some(resolution) = resolve_live(&handle, &refv)
        && let Some((turi, tptr)) = resolution_target(&resolution)
        && let Some(cptr) = component_ptr(&tptr)
        && let Some(thandle) = ws.get(&turi)
        && let Some(item) = hierarchy_item(&thandle, &turi, &cptr, None)
    {
        return Some(vec![item]);
    }

    // Key path: cursor inside a named component entry.
    let node = navigation::node_at(low, offset)?;
    let ptr = NodeRef::new(navigation::value_anchor(node)).path_from_root();
    let cptr = component_ptr(&ptr)?;
    let handle = ws.get(low.uri())?;
    let item = hierarchy_item(&handle, low.uri(), &cptr, None)?;
    Some(vec![item])
}

/// Returns the supertypes of `item`: the targets of every `$ref` inside the
/// item's `allOf`/`anyOf`/`oneOf` entries. One level per call; each returned
/// item carries fresh `{uri, ptr}` data so it can be resolved again. Cycles
/// are cut by a visited set seeded with `item` itself.
#[must_use]
pub fn supertypes(ws: &Workspace, item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    let mut out = Vec::new();
    let Some((uri, ptr)) = item_target(item) else {
        return out;
    };
    let Some(handle) = ws.get(&uri) else {
        return out;
    };
    let Some(schema) = handle.doc().root().pointer(&ptr) else {
        return out;
    };
    let mut visited = HashSet::new();
    visited.insert(target_key(&uri, &ptr));
    for keyword in COMPOSITION_KEYS {
        walk_composition(
            ws,
            &handle,
            schema.get(keyword).as_ref(),
            &mut visited,
            &mut out,
            None,
        );
    }
    out
}

/// Returns the subtypes of `item`: every workspace schema composing `item`
/// via `allOf`/`anyOf`/`oneOf`, plus children named in the item's own
/// `discriminator.mapping`. Mapping-derived children carry the detail
/// `"via discriminator"`. Deduplicated by `(uri, ptr)` like [`supertypes`].
#[must_use]
pub fn subtypes(ws: &Workspace, item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    let mut out = Vec::new();
    let Some((uri, ptr)) = item_target(item) else {
        return out;
    };
    let Some(home) = ws.get(&uri) else {
        return out;
    };
    let mut visited = HashSet::new();
    visited.insert(target_key(&uri, &ptr));

    // Discriminator-declared children: mapping values are direct ref
    // strings to child schemas of this parent (not `$ref` objects).
    if let Some(schema) = home.doc().root().pointer(&ptr)
        && let Some(mapping) = schema.get("discriminator").and_then(|d| d.get("mapping"))
        && mapping.kind() == ValueKind::Object
    {
        for entry in mapping.entries() {
            let Some(value) = entry.value else {
                continue;
            };
            if value.kind() != ValueKind::Str {
                continue;
            }
            let Some((turi, tptr)) = ref_target(&uri, value.decoded_scalar().trim_ascii()) else {
                continue;
            };
            let key = target_key(&turi, &tptr);
            if !visited.insert(key) {
                continue;
            }
            if let Some(thandle) = ws.get(&turi)
                && let Some(child) =
                    hierarchy_item(&thandle, &turi, &tptr, Some("via discriminator"))
            {
                out.push(child);
            }
        }
    }

    // Inverse scan: any workspace schema whose composition arrays ref us.
    for doc_uri in ws.uris() {
        let Some(handle) = ws.get(&doc_uri) else {
            continue;
        };
        let Some(schemas) = handle.doc().root().pointer(&schemas_pointer()) else {
            continue;
        };
        if schemas.kind() != ValueKind::Object {
            continue;
        }
        for entry in schemas.entries() {
            let Some(schema) = entry.value else {
                continue;
            };
            let child_ptr = Pointer::from_tokens(vec![
                "components".into(),
                "schemas".into(),
                entry.key.into(),
            ]);
            let composes_item = COMPOSITION_KEYS.iter().any(|keyword| {
                schema.get(keyword).is_some_and(|comp| {
                    comp.kind() == ValueKind::Array
                        && comp.items().iter().any(|candidate| {
                            candidate.get("$ref").is_some_and(|refv| {
                                resolution_matches(&handle.resolve_ref_value(refv), &uri, &ptr)
                            })
                        })
                })
            });
            if !composes_item {
                continue;
            }
            let key = target_key(&doc_uri, &child_ptr);
            if visited.insert(key)
                && let Some(child) = hierarchy_item(&handle, &doc_uri, &child_ptr, None)
            {
                out.push(child);
            }
        }
    }
    out
}

/// Parses the static named-schema table pointer.
fn schemas_pointer() -> Pointer {
    Pointer::parse(SCHEMAS_PTR).expect("static pointer")
}
/// Resolves a plain `$ref`-style string (e.g. a `discriminator.mapping`
/// value) against the document carrying it. Fragment-only refs stay in the
/// base document; document-qualified refs join against the base.
fn ref_target(base: &Uri, raw: &[u8]) -> Option<(Uri, Pointer)> {
    let raw = std::str::from_utf8(raw).ok()?;
    let (doc_part, frag) = Uri::split_ref(raw);
    let uri = match doc_part {
        None => base.clone(),
        Some(doc) => base.join(doc).ok()?,
    };
    if frag.is_empty() {
        return None; // whole-document targets carry no schema pointer
    }
    let decoded = String::from_utf8(suspect_low::percent_decode_fragment(frag)).ok()?;
    let ptr = Pointer::parse(&decoded).ok()?;
    Some((uri, ptr))
}

/// Truncates `ptr` to `/components/schemas/<Name>` when it points inside the
/// named schema table.
fn component_ptr(ptr: &Pointer) -> Option<Pointer> {
    let tokens = ptr.tokens();
    if tokens.len() >= 3 && &*tokens[0] == "components" && &*tokens[1] == "schemas" {
        return Some(Pointer::from_tokens(vec![
            tokens[0].clone(),
            tokens[1].clone(),
            tokens[2].clone(),
        ]));
    }
    None
}

/// Extracts the `(uri, ptr)` round-trip data stashed on an item.
fn item_target(item: &TypeHierarchyItem) -> Option<(Uri, Pointer)> {
    let data = item.data.as_ref()?;
    let uri = Uri::parse(data.get("uri")?.as_str()?).ok()?;
    let ptr = Pointer::parse(data.get("ptr")?.as_str()?).ok()?;
    Some((uri, ptr))
}

/// Cycle/dedup key for a target location.
fn target_key(uri: &Uri, ptr: &Pointer) -> String {
    format!("{}#{}", uri.as_str(), ptr.to_path())
}

/// Narrows a resolution to a concrete `(uri, ptr)` location.
fn resolution_target(resolution: &Resolution<'_>) -> Option<(Uri, Pointer)> {
    match resolution {
        Resolution::Node(node) => Some((node.syntax().doc().uri().clone(), node.path_from_root())),
        Resolution::WholeDoc(_) | Resolution::Cycle { .. } => None,
    }
}

/// Whether a resolution landed exactly on `(uri, ptr)`; failed resolutions
/// never match.
fn resolution_matches(
    resolution: &Result<Resolution<'_>, RefError>,
    uri: &Uri,
    ptr: &Pointer,
) -> bool {
    resolution
        .as_ref()
        .ok()
        .and_then(resolution_target)
        .is_some_and(|(u, p)| u == *uri && p == *ptr)
}

/// Resolves a live-buffer `$ref` value through the workspace, mirroring the
/// chain logic of `navigation::resolve_live_ref` (kept local because that
/// helper is private to its module).
fn resolve_live<'ws>(handle: &DocHandle<'ws>, refv: &SNode<'_>) -> Option<Resolution<'ws>> {
    let containing = NodeRef::new(*refv)
        .path_from_root()
        .parent()
        .unwrap_or_default();
    if containing.is_root() {
        let node = navigation::rederive(handle, refv.byte_range())?;
        return handle.resolve_ref_value(node).ok();
    }
    // Chain hops read the disk copy; an unsaved ref value has no sound
    // resolution yet.
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

/// Builds a hierarchy item for a target location; `detail` annotates
/// discriminator-derived children.
fn hierarchy_item(
    handle: &DocHandle<'_>,
    uri: &Uri,
    ptr: &Pointer,
    detail: Option<&str>,
) -> Option<TypeHierarchyItem> {
    let node = handle.doc().root().pointer(ptr)?;
    let name = ptr.tokens().last()?.to_string();
    let key_range = node
        .syntax()
        .parent()
        .filter(|pair| pair.kind() == SyntaxKind::Pair)
        .and_then(|pair| pair.child_by_field("key"))
        .map_or_else(|| node.byte_range(), |key| key.byte_range());
    let inner = handle.doc().inner();
    let url = Url::parse(uri.as_str()).ok()?;
    Some(TypeHierarchyItem {
        name,
        kind: SymbolKind::CLASS,
        tags: None,
        detail: detail.map(str::to_owned),
        uri: url,
        range: lsp_range(inner.bytes(), inner.line_index(), key_range.clone()),
        selection_range: lsp_range(inner.bytes(), inner.line_index(), key_range),
        data: Some(serde_json::json!({
            "uri": uri.as_str(),
            "ptr": ptr.to_path(),
        })),
    })
}

/// Resolves every `$ref` under a composition array and appends new items.
fn walk_composition(
    ws: &Workspace,
    handle: &DocHandle<'_>,
    comp: Option<&NodeRef<'_>>,
    visited: &mut HashSet<String>,
    out: &mut Vec<TypeHierarchyItem>,
    detail: Option<&str>,
) {
    let Some(comp) = comp else {
        return;
    };
    if comp.kind() != ValueKind::Array {
        return;
    }
    for entry in comp.items() {
        let Some(refv) = entry.get("$ref") else {
            continue;
        };
        let Ok(resolution) = handle.resolve_ref_value(refv) else {
            continue;
        };
        push_target(ws, &resolution, visited, out, detail);
    }
}

/// Converts a resolution into a deduped hierarchy item, appending to `out`.
fn push_target(
    ws: &Workspace,
    resolution: &Resolution<'_>,
    visited: &mut HashSet<String>,
    out: &mut Vec<TypeHierarchyItem>,
    detail: Option<&str>,
) {
    let Some((turi, tptr)) = resolution_target(resolution) else {
        return;
    };
    if !visited.insert(target_key(&turi, &tptr)) {
        return;
    }
    if let Some(thandle) = ws.get(&turi)
        && let Some(item) = hierarchy_item(&thandle, &turi, &tptr, detail)
    {
        out.push(item);
    }
}

// ---------------------------------------------------------------------------
// Part 2: range formatting
// ---------------------------------------------------------------------------

/// Canonicalizes only the lines intersecting `range` (byte offsets into the
/// buffer): contained block mappings/sequences are re-indented from tree
/// depth, single-quoted scalars safe as plain text are unquoted, and
/// comments/blank lines are preserved verbatim. Documents with syntax errors
/// are never touched. Only lines whose emitted form differs yield edits.
#[must_use]
pub fn range_formatting(doc: &OpenDoc, range: std::ops::Range<usize>) -> Vec<TextEdit> {
    if !doc.low.syntax_errors().is_empty() || range.is_empty() {
        return Vec::new();
    }
    let inner = doc.low.inner();
    let bytes = inner.bytes();
    let li = inner.line_index();

    // Clamp and expand the selection to whole lines; an end sitting exactly
    // on a line start excludes that line.
    let len = bytes.len();
    let start = range.start.min(len);
    let end = range.end.min(len).max(start);
    let first_line = line_of(li, bytes, start);
    let mut last_line = line_of(li, bytes, end);
    if last_line > first_line
        && li
            .line_range(bytes, last_line)
            .is_some_and(|r| r.start == end)
    {
        last_line -= 1;
    }
    let Some(window) = line_window(li, bytes, first_line, last_line) else {
        return Vec::new();
    };

    let root = inner.root();
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut claimed: HashSet<u32> = HashSet::new();

    // Re-indent every block container fully contained in the window, each
    // anchored by its own structural depth in the tree.
    let mut roots = Vec::new();
    collect_contained(root, window.clone(), &mut roots);
    for node in roots {
        let anchor = structural_anchor(node);
        match node.kind() {
            SyntaxKind::Mapping => {
                collect_mapping(node, anchor, bytes, li, &mut claimed, &mut edits)
            }
            SyntaxKind::Sequence => {
                collect_sequence(node, anchor, bytes, li, &mut claimed, &mut edits)
            }
            _ => {}
        }
    }

    // Unquote single-quoted scalars whose plain rendering is lossless.
    for node in root.descendants() {
        quote_edit(node, &window, inner.format(), bytes, &mut edits);
    }

    finish_edits(edits, bytes, li)
}

/// Canonical start column of a container: two spaces per enclosing pair.
fn structural_anchor(node: SNode<'_>) -> usize {
    let mut pairs = 0;
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        if current.kind() == SyntaxKind::Pair {
            pairs += 1;
        }
        ancestor = current.parent();
    }
    2 * pairs
}

/// Zero-based line containing `offset`.
fn line_of(li: &LineIndex, bytes: &[u8], offset: usize) -> u32 {
    u32::try_from(li.line_col_bytes(bytes, offset).0).unwrap_or(u32::MAX)
}
/// Whether a node is a flow-style collection (never re-indented).
fn is_flow_kind(raw_kind: &str) -> bool {
    matches!(raw_kind, "flow_mapping" | "flow_sequence")
}

/// Byte window spanning whole lines `first..=last`, including the final
/// line's terminator so document-spanning nodes still count as contained.
fn line_window(
    li: &LineIndex,
    bytes: &[u8],
    first_line: u32,
    last_line: u32,
) -> Option<std::ops::Range<usize>> {
    let start = li.line_range(bytes, first_line)?.start;
    let mut end = li.line_range(bytes, last_line)?.end;
    if let Some(next) = li.line_range(bytes, last_line + 1) {
        end = end.max(next.start);
    } else {
        end = bytes.len();
    }
    Some(start..end)
}

/// Collects maximal Mapping/Sequence nodes fully contained in `win`.
fn collect_contained<'d>(node: SNode<'d>, win: std::ops::Range<usize>, out: &mut Vec<SNode<'d>>) {
    let span = node.byte_range();
    if span.end <= win.start || span.start >= win.end {
        return;
    }
    if win.start <= span.start
        && span.end <= win.end
        && matches!(node.kind(), SyntaxKind::Mapping | SyntaxKind::Sequence)
        && !is_flow_kind(node.raw_kind())
    {
        out.push(node);
        return;
    }
    for child in node.children() {
        collect_contained(child, win.clone(), out);
    }
}

/// Emits indentation fixes for a block mapping anchored at column `anchor`.
///
/// `claimed` records lines already laid out by an enclosing sequence marker;
/// pairs sharing those lines are inline content and keep their positions.
fn collect_mapping(
    node: SNode<'_>,
    anchor: usize,
    bytes: &[u8],
    li: &LineIndex,
    claimed: &mut HashSet<u32>,
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
) {
    for (key, value) in node.mapping_entries() {
        let key_start = key.byte_range().start;
        let line = line_of(li, bytes, key_start);
        if !claimed.insert(line) {
            continue; // inline under an ancestor's `- `
        }
        match line_head(bytes, li, line) {
            None => {}            // blank line: preserve
            Some((_, b'#')) => {} // comment-only line: preserve
            Some((_, b'-')) => {} // unexpected marker layout: skip
            Some(_) => push_indent_edit(edits, bytes, li, key_start, anchor),
        }

        if let Some(value) = value {
            descend(value, anchor + 2, bytes, li, claimed, edits);
        }
    }
}

/// Emits indentation fixes for a block sequence anchored at column `anchor`
/// (`- ` markers sit two spaces inside their parent key).
fn collect_sequence(
    node: SNode<'_>,
    anchor: usize,
    bytes: &[u8],
    li: &LineIndex,
    claimed: &mut HashSet<u32>,
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
) {
    for item in node.sequence_items() {
        let item_start = item.byte_range().start;
        let line = line_of(li, bytes, item_start);
        if !claimed.insert(line) {
            continue; // inline under an outer `- -` marker
        }
        let Some((head_col, head_byte)) = line_head(bytes, li, line) else {
            continue;
        };
        let next_is_space = bytes
            .get(head_col + 1)
            .is_some_and(|b| b.is_ascii_whitespace());
        if head_byte != b'-' || !next_is_space {
            continue; // comment line or missing marker: untouched
        }
        push_indent_edit(edits, bytes, li, item_start, anchor);
        descend(item, anchor + 2, bytes, li, claimed, edits);
    }
}

/// Recurses into `value` when it is a multi-line block collection nested
/// under a key or sequence item.
fn descend(
    value: SNode<'_>,
    child_anchor: usize,
    bytes: &[u8],
    li: &LineIndex,
    claimed: &mut HashSet<u32>,
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
) {
    let kind = value.kind();
    let is_block_container = matches!(kind, SyntaxKind::Mapping | SyntaxKind::Sequence)
        && !is_flow_kind(value.raw_kind());
    if !is_block_container {
        return;
    }
    // A block collection always spans past its first line; anything written
    // inline (flow style, parse recovery) is left alone.
    let span = value.byte_range();
    if !bytes[span].contains(&b'\n') {
        return;
    }
    match kind {
        SyntaxKind::Mapping => collect_mapping(value, child_anchor, bytes, li, claimed, edits),
        SyntaxKind::Sequence => collect_sequence(value, child_anchor, bytes, li, claimed, edits),
        _ => {}
    }
}

/// First non-space byte of `line` as `(byte_offset, byte)`.
fn line_head(bytes: &[u8], li: &LineIndex, line: u32) -> Option<(usize, u8)> {
    let range = li.line_range(bytes, line)?;
    for (i, byte) in bytes[range.clone()].iter().enumerate() {
        if !byte.is_ascii_whitespace() {
            return Some((range.start + i, *byte));
        }
    }
    None
}

/// Queues an indent fix for the line containing `offset` unless the line is
/// blank, comment-only, or already correctly indented.
fn push_indent_edit(
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
    bytes: &[u8],
    li: &LineIndex,
    offset: usize,
    expected: usize,
) {
    let line = line_of(li, bytes, offset);
    let Some(range) = li.line_range(bytes, line) else {
        return;
    };
    let Some((head, byte)) = line_head(bytes, li, line) else {
        return; // blank line: preserve
    };
    if byte == b'#' {
        return; // comment line: preserve
    }
    if head - range.start == expected {
        return; // already canonical
    }
    edits.push((range.start..head, " ".repeat(expected)));
}

/// Unquotes one single-quoted scalar when its plain rendering is provably
/// lossless (structure-identical reparse, same string value).
fn quote_edit(
    node: SNode<'_>,
    window: &std::ops::Range<usize>,
    format: Format,
    bytes: &[u8],
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
) {
    let span = node.byte_range();
    if span.start < window.start || span.end > window.end {
        return;
    }
    if node.kind() != SyntaxKind::Scalar || node.scalar_style() != ScalarStyle::SingleQuoted {
        return;
    }
    if bytes[span.clone()].contains(&b'\n') {
        return;
    }
    // Never touch scalars nested inside flow collections.
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        if is_flow_kind(current.raw_kind()) {
            return;
        }
        ancestor = current.parent();
    }
    let decoded = NodeRef::new(node).decoded_scalar();
    let Ok(content) = std::str::from_utf8(&decoded) else {
        return;
    };
    if !plain_safe(content, format) {
        return;
    }
    edits.push((span, content.to_owned()));
}

/// Cheap predicate for "this string can be written without quotes".
fn plain_safe(content: &str, format: Format) -> bool {
    if content.is_empty() || content.trim() != content {
        return false;
    }
    if QUOTE_REQUIRED.contains(&content.to_ascii_lowercase().as_str()) {
        return false;
    }
    let mut chars = content.chars();
    let first = chars.next().unwrap_or_default();
    if "-?:,[]{}#&*!|>'\"%@`\t".contains(first) {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || "_-. /".contains(c)) {
        return false;
    }
    // Structural probe: the plain form must reparse as the same string,
    // rejecting numbers, booleans, nulls, and anything ambiguous.
    plain_reparses_as_string(content, format)
}

/// Reparses `__probe: <plain>` and checks the value decodes back to
/// `content` as a string.
fn plain_reparses_as_string(content: &str, format: Format) -> bool {
    let probe_text = format!("__probe: {content}\n");
    let Ok(uri) = Uri::parse("mem://fmt-probe.yaml") else {
        return false;
    };
    let probe = suspect_low::LowDoc::with_format(
        uri,
        suspect_source::Source::from_vec(probe_text.into_bytes()),
        format,
    );
    let Some(value) = probe.root().get("__probe") else {
        return false;
    };
    value.kind() == ValueKind::Str && value.decoded_scalar().trim_ascii() == content.as_bytes()
}

/// Sorts, dedups, and converts queued byte-range edits to LSP edits.
fn finish_edits(
    mut edits: Vec<(std::ops::Range<usize>, String)>,
    bytes: &[u8],
    li: &LineIndex,
) -> Vec<TextEdit> {
    edits.sort_by_key(|(range, _)| range.start);
    edits.dedup_by(|a, b| a.0.start == b.0.start);
    edits
        .into_iter()
        .map(|(range, new_text)| TextEdit {
            range: lsp_range(bytes, li, range),
            new_text,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use suspect_low::LowDoc;
    use suspect_ref::{Workspace, WorkspaceBuilder};
    use suspect_source::{Source, Uri};
    use tower_lsp::lsp_types::{Position, Range as LspRange};

    const HIER: &str = r#"
openapi: 3.1.0
components:
  schemas:
    PetBase:
      type: object
      discriminator:
        propertyName: petType
        mapping:
          dog: '#/components/schemas/Dog'
    Pet:
      allOf:
        - $ref: '#/components/schemas/PetBase'
    Dog:
      allOf:
        - $ref: '#/components/schemas/Pet'
      discriminator:
        propertyName: petType
    Cat:
      allOf:
        - $ref: '#/components/schemas/PetBase'
"#;

    const CYCLIC: &str = r#"
components:
  schemas:
    SelfRef:
      allOf:
        - $ref: '#/components/schemas/SelfRef'
"#;

    fn workspace(dir: &std::path::Path, name: &str, text: &str) -> Arc<Workspace> {
        std::fs::write(dir.join(name), text).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all(name).unwrap();
        Arc::new(ws)
    }

    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        LowDoc::parse(
            Uri::from_path(&path).unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn open_at(dir: &std::path::Path, name: &str, text: &str) -> OpenDoc {
        let path = dir.join(name);
        let uri = Uri::from_path(&path).unwrap();
        OpenDoc::parse(uri, text.to_owned())
    }

    fn offset_in(text: &str, needle: &str) -> usize {
        let at = text.find(needle).expect("needle present");
        at + needle.len() / 2
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("suspect-lsp-th-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn item_named<'a>(items: &'a [TypeHierarchyItem], name: &str) -> &'a TypeHierarchyItem {
        items.iter().find(|i| i.name == name).expect("item present")
    }

    /// A fabricated item without round-trip data.
    fn bogus_item() -> TypeHierarchyItem {
        TypeHierarchyItem {
            name: "?".into(),
            kind: SymbolKind::CLASS,
            tags: None,
            detail: None,
            uri: Url::parse("file:///nope.yaml").unwrap(),
            range: LspRange {
                start: Position::new(0, 0),
                end: Position::new(0, 0),
            },
            selection_range: LspRange {
                start: Position::new(0, 0),
                end: Position::new(0, 0),
            },
            data: None,
        }
    }

    #[test]
    fn prepare_on_component_key() {
        let dir = scratch("prepare-key");
        let ws = workspace(&dir, "main.yaml", HIER);
        let doc = open_at(&dir, "main.yaml", HIER);
        let off = offset_in(HIER, "Pet:");
        let items = prepare_type_hierarchy(&ws, &doc, off).expect("prepared");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.name, "Pet");
        assert_eq!(item.kind, SymbolKind::CLASS);
        assert_eq!(item.uri.as_str(), doc.low.uri().as_str());
        let data = item.data.as_ref().unwrap();
        assert_eq!(data["ptr"], "/components/schemas/Pet");
    }

    #[test]
    fn prepare_on_ref_value_targets_component() {
        let dir = scratch("prepare-ref");
        let ws = workspace(&dir, "main.yaml", HIER);
        let doc = open_at(&dir, "main.yaml", HIER);
        let off = offset_in(HIER, "'#/components/schemas/PetBase'");
        let items = prepare_type_hierarchy(&ws, &doc, off).expect("prepared");
        assert_eq!(items[0].name, "PetBase");
        let data = items[0].data.as_ref().unwrap();
        assert_eq!(data["ptr"], "/components/schemas/PetBase");
    }

    #[test]
    fn prepare_misses_return_none() {
        let dir = scratch("prepare-miss");
        let ws = workspace(&dir, "main.yaml", HIER);
        // Cursor outside components entirely.
        let doc = open_at(&dir, "main.yaml", HIER);
        let off = offset_in(HIER, "3.1.0");
        assert!(prepare_type_hierarchy(&ws, &doc, off).is_none());

        // Empty document: no nodes at all.
        let empty = open_at(&dir, "empty.yaml", "");
        assert!(prepare_type_hierarchy(&ws, &empty, 0).is_none());
    }

    #[test]
    fn supertypes_follow_composition_one_level() {
        let dir = scratch("supers");
        let ws = workspace(&dir, "main.yaml", HIER);
        let doc = open_at(&dir, "main.yaml", HIER);
        let off = offset_in(HIER, "Pet:");
        let prepared = prepare_type_hierarchy(&ws, &doc, off).unwrap();
        let supers = supertypes(&ws, &prepared[0]);
        assert_eq!(supers.len(), 1);
        assert_eq!(supers[0].name, "PetBase");

        // PetBase itself has no parents.
        let base_off = offset_in(HIER, "PetBase:");
        let base = prepare_type_hierarchy(&ws, &doc, base_off).unwrap();
        assert!(supertypes(&ws, &base[0]).is_empty());

        // Returned items remain resolvable: two hops from Dog land on
        // Pet, then PetBase.
        let dog_off = offset_in(HIER, "Dog:");
        let dog = prepare_type_hierarchy(&ws, &doc, dog_off).unwrap();
        let up = supertypes(&ws, &dog[0]);
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].name, "Pet");
        assert_eq!(
            supertypes(&ws, &up[0])[0].name,
            "PetBase",
            "chained resolution works"
        );
    }

    #[test]
    fn supertypes_dedup_self_cycles() {
        let dir = scratch("cycle-super");
        let ws = workspace(&dir, "cyc.yaml", CYCLIC);
        let low = low_at(&dir, "cyc.yaml", CYCLIC);
        let off = offset_in(CYCLIC, "SelfRef:");
        let node = navigation::node_at(&low, off).unwrap();
        let ptr = NodeRef::new(navigation::value_anchor(node)).path_from_root();
        let handle = ws.get(low.uri()).unwrap();
        let item = hierarchy_item(&handle, low.uri(), &ptr, None).unwrap();
        // The only edge points back at the item itself; the visited seed
        // must suppress it instead of looping.
        assert!(supertypes(&ws, &item).is_empty());
    }

    #[test]
    fn subtypes_scan_and_discriminator() {
        let dir = scratch("subs");
        let ws = workspace(&dir, "main.yaml", HIER);
        let doc = open_at(&dir, "main.yaml", HIER);
        let off = offset_in(HIER, "PetBase:");
        let base = prepare_type_hierarchy(&ws, &doc, off).unwrap();

        let subs = subtypes(&ws, &base[0]);
        // Pet and Cat compose PetBase directly; Dog additionally arrives via
        // discriminator mapping and must be flagged.
        assert_eq!(subs.len(), 3, "got {:?}", subs.iter().map(|i| &i.name));
        let dog = item_named(&subs, "Dog");
        assert_eq!(dog.detail.as_deref(), Some("via discriminator"));
        assert!(item_named(&subs, "Cat").detail.is_none());
        assert!(item_named(&subs, "Pet").detail.is_none());

        // One level down from Pet sits Dog via plain allOf.
        let pet_off = offset_in(HIER, "Pet:");
        let pet = prepare_type_hierarchy(&ws, &doc, pet_off).unwrap();
        let pet_subs = subtypes(&ws, &pet[0]);
        assert_eq!(pet_subs.len(), 1);
        assert_eq!(pet_subs[0].name, "Dog");
        assert!(pet_subs[0].detail.is_none());
    }

    #[test]
    fn subtypes_handle_bad_items_gracefully() {
        let dir = scratch("subs-bad");
        let ws = workspace(&dir, "main.yaml", HIER);
        let bogus = bogus_item();
        assert!(subtypes(&ws, &bogus).is_empty());
        assert!(supertypes(&ws, &bogus).is_empty());
    }

    const MESSY: &str = "a:\n    b: 1\n    c:\n        - x\n        - 'yes'\n# top comment\n\nd:\n  e: 'hello world'\n";

    #[test]
    fn range_formatting_reindents_and_unquotes() {
        let dir = scratch("fmt-basic");
        let doc = open_at(&dir, "m.yaml", MESSY);
        let edits = range_formatting(&doc, 0..MESSY.len());
        let applied = apply_edits(MESSY, &edits);
        assert_eq!(
            applied,
            "a:\n  b: 1\n  c:\n    - x\n    - 'yes'\n# top comment\n\nd:\n  e: hello world\n"
        );
    }

    /// `'200'` reads numeric and `'yes'` reads boolean-ish when unquoted, so
    /// both must survive verbatim while prose quotes normalize away.
    #[test]
    fn range_formatting_keeps_semantic_quotes() {
        let src = "codes:\n  '200': ok\nflags:\n  a: 'yes'\n  b: 'null'\n  k: 'plain text'\n";
        let dir = scratch("fmt-quotes");
        let doc = open_at(&dir, "q.yaml", src);
        let applied = apply_edits(src, &range_formatting(&doc, 0..src.len()));
        assert_eq!(
            applied,
            "codes:\n  '200': ok\nflags:\n  a: 'yes'\n  b: 'null'\n  k: plain text\n"
        );
    }

    #[test]
    fn range_formatting_preserves_comments_blank_lines_and_flow() {
        let src = "# header\nflow: {a: 1}\n\nlist:\n  - 'safe'\n";
        let dir = scratch("fmt-preserve");
        let doc = open_at(&dir, "p.yaml", src);
        let applied = apply_edits(src, &range_formatting(&doc, 0..src.len()));
        assert_eq!(applied, "# header\nflow: {a: 1}\n\nlist:\n  - safe\n");
    }

    #[test]
    fn range_formatting_partial_range_never_touches_outside_lines() {
        let src = "top:\n  inner:\n      a: 1\n      b: 2\n  tail: 3\n";
        let dir = scratch("fmt-partial");
        let doc = open_at(&dir, "s.yaml", src);
        let li = doc.low.inner().line_index();
        let bytes = doc.low.inner().bytes();

        // Window covering only the two mis-indented lines: the inner mapping
        // is fully contained and snaps to its structural depth (column 4).
        let w0 = li.line_range(bytes, 2).unwrap().start;
        let w1 = li.line_range(bytes, 3).unwrap().end;
        let applied = apply_edits(src, &range_formatting(&doc, w0..w1));
        assert_eq!(applied, "top:\n  inner:\n    a: 1\n    b: 2\n  tail: 3\n");

        // Window covering only line 3 (`b: 2`): no container fits wholly
        // inside, so nothing may change.
        let o0 = li.line_range(bytes, 3).unwrap().start;
        let o1 = li.line_range(bytes, 3).unwrap().end;
        assert!(range_formatting(&doc, o0..o1).is_empty());
    }

    #[test]
    fn range_formatting_canonical_input_is_noop() {
        let src = "a:\n  b:\n    - x: 1\n      y: 2\n    - z\n";
        let dir = scratch("fmt-noop");
        let doc = open_at(&dir, "n.yaml", src);
        assert!(range_formatting(&doc, 0..src.len()).is_empty());
    }

    #[test]
    fn range_formatting_skips_syntax_errors_and_empty_ranges() {
        let dir = scratch("fmt-bad");
        let broken = open_at(&dir, "x.yaml", "a: [unclosed\n");
        assert!(!broken.low.syntax_errors().is_empty());
        assert!(range_formatting(&broken, 0..14).is_empty());

        let fine = open_at(&dir, "y.yaml", "a:\n  b: 1\n");
        assert!(range_formatting(&fine, 5..5).is_empty());
    }

    /// Applies edits the way an LSP client would (sorted, non-overlapping).
    /// Test inputs are ASCII, so scalar columns equal UTF-16 columns.
    fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
        let li = LineIndex::new(text.as_bytes());
        let mut resolved: Vec<(std::ops::Range<usize>, &str)> = edits
            .iter()
            .map(|e| {
                let start = li
                    .offset_of(text.as_bytes(), e.range.start.line, e.range.start.character)
                    .unwrap();
                let end = li
                    .offset_of(text.as_bytes(), e.range.end.line, e.range.end.character)
                    .unwrap();
                (start..end, e.new_text.as_str())
            })
            .collect();
        resolved.sort_by_key(|(r, _)| r.start);
        let mut out = String::new();
        let mut pos = 0;
        for (range, new_text) in resolved {
            out.push_str(&text[pos..range.start]);
            out.push_str(new_text);
            pos = range.end;
        }
        out.push_str(&text[pos..]);
        out
    }
}
