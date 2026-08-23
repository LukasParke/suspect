//! Call hierarchy over the `$ref` graph, plus declaration and
//! type-definition navigation.
//!
//! The call hierarchy treats every named component entry
//! (`/components/<section>/<Name>`) as a callable: incoming calls invert the
//! workspace's `$ref` edges (who points here), outgoing calls follow them
//! forward (what does this component point at). Referencing sites are
//! attributed to their enclosing *named thing* — the owning component when
//! the `$ref` sits under `components`, otherwise the operation
//! (`"get /pets"`) when it sits under `paths`.
//!
//! `declaration` and `type_definition` are position-based navigators:
//! declaration resolves a `$ref` to the target's declaring KEY line
//! (`Pet:` rather than its full body), and type definition jumps from a
//! typed usage site (property name, media-type key, response status,
//! parameter name) to the schema that types it.
//!
//! All functions are pure: they take the [`Workspace`] plus a parsed
//! [`LowDoc`] and return LSP types; no client handle is involved.

use std::collections::HashMap;
use std::ops::Range;

use suspect_low::{LowDoc, NodeRef, Pointer};
use suspect_ref::{DocHandle, Resolution, Workspace};
use suspect_source::Uri;
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, Location, SymbolKind,
    Url,
};

use crate::navigation::{self, Definition};
use crate::state::lsp_range;

/// Component sections whose entries are named and referenceable.
const COMPONENT_SECTIONS: &[&str] = &[
    "schemas",
    "responses",
    "parameters",
    "examples",
    "requestBodies",
    "headers",
    "securitySchemes",
    "links",
    "callbacks",
    "pathItems",
];

/// HTTP verbs that may appear as a path-item operation key.
const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Prepares a call-hierarchy item for the cursor position.
///
/// Two positions qualify:
///
/// - the cursor sits on a component key (`Pet:` under
///   `/components/<section>/`) — the item describes that component;
/// - the cursor sits on a `$ref` value — the item describes the resolved
///   target component (chains followed through the workspace).
///
/// `None` when the cursor is elsewhere, on an unresolvable `$ref`, or on a
/// `$ref` whose target is not a named component.
#[must_use]
pub fn prepare_call_hierarchy(
    ws: &Workspace,
    low: &LowDoc,
    offset: usize,
) -> Option<CallHierarchyItem> {
    let handle = ws.get(low.uri())?;
    if let Some(def) = navigation::goto_definition(ws, low, offset) {
        let target_handle = ws.get(&def.uri)?;
        // Cursor on a `$ref` value: describe the target component.
        let target = ws_node(&target_handle, def.range.clone())?;
        let target_ptr = target.path_from_root();
        let (sec, name) = component_of(&target_ptr)?;
        return component_item(&target_handle, sec, name);
    }
    // Cursor on a component key: `/components/<section>/<Name>` — the
    // pair-value anchor of the key-side node is exactly 3 tokens deep, and
    // the token under the cursor must be the component's own name (rejects
    // nested keys like `/components/schemas/Pet/type`).
    let node = navigation::node_at(low, offset)?;
    let ptr = NodeRef::new(value_anchor(node)).path_from_root();
    if let Some((sec, name)) = component_of(&ptr)
        && let Some((_, key)) = key_pair(node)
        && key.scalar_bytes() == name.as_bytes()
    {
        return component_item(&handle, sec, name);
    }
    None
}

/// Finds every call site that references the component described by `item`.
///
/// Each workspace `$ref` edge whose resolved target equals this component's
/// `(uri, pointer)` contributes one source span (`from_ranges`); sources are
/// grouped per enclosing named thing and returned in deterministic
/// `(uri, name)` order. Returns an empty vector when `item` was not produced
/// by [`prepare_call_hierarchy`] or has no referrers.
#[must_use]
pub fn incoming_calls(ws: &Workspace, item: &CallHierarchyItem) -> Vec<CallHierarchyIncomingCall> {
    let mut out: Vec<CallHierarchyIncomingCall> = Vec::new();
    let Some((item_uri, target)) = target_of_item(item) else {
        return out;
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        let inner = handle.doc().inner();
        let edges = handle.edges();
        for (i, edge) in edges.iter().enumerate() {
            let Ok(Resolution::Node(n)) = handle.resolve_edge(i) else {
                continue;
            };
            if *n.syntax().doc().uri() != item_uri || n.path_from_root() != target {
                continue;
            }
            let Some(from) = enclosing_item(&handle, &edge.path) else {
                continue;
            };
            let key = format!("{}\u{0}{}", from.uri, from.name);
            let slot = match index.get(&key) {
                Some(&s) => s,
                None => {
                    out.push(CallHierarchyIncomingCall {
                        from,
                        from_ranges: Vec::new(),
                    });
                    let s = out.len() - 1;
                    index.insert(key, s);
                    s
                }
            };
            out[slot].from_ranges.push(lsp_range(
                inner.bytes(),
                inner.line_index(),
                edge.at.clone(),
            ));
        }
    }
    for call in &mut out {
        call.from_ranges
            .sort_by_key(|r| (r.start.line, r.start.character));
    }
    out.sort_by(|a, b| {
        (a.from.uri.as_str(), a.from.name.as_str())
            .cmp(&(b.from.uri.as_str(), b.from.name.as_str()))
    });
    out
}

/// Finds every component referenced from within the component described by
/// `item`.
///
/// Every `$ref` edge whose containing mapping lives under this component's
/// pointer is resolved through its full chain; targets that land in a named
/// component become outgoing calls with the originating `$ref` spans as
/// `to_ranges`. Returns an empty vector when `item` was not produced by
/// [`prepare_call_hierarchy`] or references nothing.
#[must_use]
pub fn outgoing_calls(ws: &Workspace, item: &CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
    let mut out: Vec<CallHierarchyOutgoingCall> = Vec::new();
    let Some((item_uri, root)) = target_of_item(item) else {
        return out;
    };
    let Some(handle) = ws.get(&item_uri) else {
        return out;
    };
    let inner = handle.doc().inner();
    let root_tokens = root.tokens();
    let mut index: HashMap<String, usize> = HashMap::new();
    let edges = handle.edges();
    for (i, edge) in edges.iter().enumerate() {
        // Only edges whose containing mapping is inside this component.
        let site = edge.path.tokens();
        if site.len() < root_tokens.len() || site[..root_tokens.len()] != root_tokens[..] {
            continue;
        }
        let Ok(Resolution::Node(n)) = handle.resolve_edge(i) else {
            continue;
        };
        let n_ptr = n.path_from_root();
        let Some((sec, name)) = component_of(&n_ptr) else {
            continue;
        };
        let doc_uri = n.syntax().doc().uri().clone();
        let Some(target_handle) = ws.get(&doc_uri) else {
            continue;
        };
        let Some(to) = component_item(&target_handle, sec, name) else {
            continue;
        };
        let key = format!("{}\u{0}{}", to.uri, to.name);
        let slot = match index.get(&key) {
            Some(&s) => s,
            None => {
                out.push(CallHierarchyOutgoingCall {
                    to,
                    from_ranges: Vec::new(),
                });
                let s = out.len() - 1;
                index.insert(key, s);
                s
            }
        };
        out[slot].from_ranges.push(lsp_range(
            inner.bytes(),
            inner.line_index(),
            edge.at.clone(),
        ));
    }
    for call in &mut out {
        call.from_ranges
            .sort_by_key(|r| (r.start.line, r.start.character));
    }
    out.sort_by(|a, b| {
        (a.to.uri.as_str(), a.to.name.as_str()).cmp(&(b.to.uri.as_str(), b.to.name.as_str()))
    });
    out
}

/// Resolves the declaration at the cursor.
///
/// Declaration ≈ definition for `$ref`-based specs; the one meaningful
/// distinction is that declaration lands on the target's declaring KEY line
/// (`Tag:` under `components/schemas`) instead of its full body. Qualifying
/// positions:
///
/// - a `$ref` value itself;
/// - a property name under `properties:` whose schema is a `$ref`;
/// - a parameter `name:` whose parameter object carries a `$ref`.
///
/// `None` otherwise (no `$ref` involved, or unresolvable).
#[must_use]
pub fn declaration(ws: &Workspace, low: &LowDoc, offset: usize) -> Option<Vec<Location>> {
    let ref_offset = declaration_ref_offset(low, offset)?;
    let def = navigation::goto_definition(ws, low, ref_offset)?;
    // Stop at the declaring key of the resolved target, not its body.
    let target_handle = ws.get(&def.uri)?;
    let target = ws_node(&target_handle, def.range.clone())?;
    let at_key = Definition {
        uri: def.uri.clone(),
        range: key_span(target.syntax()),
    };
    let loc = to_location(ws, low, &at_key)?;
    Some(vec![loc])
}

/// Jumps from a typed usage site to the schema that types it.
///
/// Recognized sites: a property name under `properties:` (or
/// `patternProperties`), an `items`/`prefixItems` key, a media-type key
/// (`application/json`) under `content:`, a response status key (`'200'`)
/// under `responses:`, and a parameter `name:`. `$ref`s are followed to
/// their final schema; an inline schema yields its own location. Response
/// status keys produce one location per declared media type's schema.
///
/// `None` when the cursor is not on a recognized site or nothing types it.
#[must_use]
pub fn type_definition(ws: &Workspace, low: &LowDoc, offset: usize) -> Option<Vec<Location>> {
    let node = navigation::node_at(low, offset)?;
    let defs = typing_definitions(ws, low, node)?;
    let locs: Vec<Location> = defs
        .iter()
        .filter_map(|d| to_location(ws, low, d))
        .collect();
    (!locs.is_empty()).then_some(locs)
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

/// Maps a key-side node to its owning pair's value content. Mirrors
/// navigation's value anchoring: `path_from_root` only traverses mapping
/// values, so pointer math must anchor on the value side.
fn value_anchor<'d>(node: SNode<'d>) -> SNode<'d> {
    let nr = node.byte_range();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(key) = n.child_by_field("key")
        {
            let kr = key.byte_range();
            if kr.start <= nr.start
                && nr.end <= kr.end
                && let Some(v) = n.child_by_field("value")
            {
                return v.content();
            }
            break;
        }
        cur = n.parent();
    }
    node
}

/// Byte range of the KEY of the pair that owns `node` (a value-side node);
/// falls back to `node`'s own range when no owning pair exists.
fn key_span(node: &SNode<'_>) -> Range<usize> {
    let nr = node.byte_range();
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(v) = n.child_by_field("value")
        {
            let vr = v.byte_range();
            if vr.start <= nr.start
                && nr.end <= vr.end
                && let Some(k) = n.child_by_field("key")
            {
                return k.byte_range();
            }
        }
        cur = n.parent();
    }
    nr
}

/// The nearest ancestor Pair of `pair` together with its key text — the
/// mapping-key context (`properties`, `responses`, `content`, …) that
/// classifies the site.
fn ancestor_pair_key(pair: &SNode<'_>) -> Option<String> {
    let mut cur = pair.parent();
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(k) = n.child_by_field("key")
        {
            return std::str::from_utf8(k.scalar_bytes())
                .ok()
                .map(str::to_owned);
        }
        cur = n.parent();
    }
    None
}

/// The Pair owning `node` as its KEY, paired with the key node itself.
fn key_pair<'d>(node: SNode<'d>) -> Option<(SNode<'d>, SNode<'d>)> {
    let nr = node.byte_range();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(key) = n.child_by_field("key")
        {
            let kr = key.byte_range();
            return (kr.start <= nr.start && nr.end <= kr.end).then_some((n, key));
        }
        cur = n.parent();
    }
    None
}

/// The Pair owning `node` as its VALUE content (e.g. the scalar after
/// `name:`), for value-side lookups.
fn value_pair<'d>(node: SNode<'d>) -> Option<SNode<'d>> {
    let nr = node.byte_range();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(v) = n.child_by_field("value")
        {
            let vr = v.byte_range();
            if vr.start <= nr.start && nr.end <= vr.end {
                return Some(n);
            }
        }
        cur = n.parent();
    }
    None
}

/// Splits a pointer into `(section, name)` when it addresses a named
/// component entry (`/components/<section>/<Name>`).
fn component_of(ptr: &Pointer) -> Option<(&str, &str)> {
    let t = ptr.tokens();
    if t.len() < 3 || t[0].as_ref() != "components" {
        return None;
    }
    let sec = t[1].as_ref();
    if !COMPONENT_SECTIONS.contains(&sec) {
        return None;
    }
    Some((sec, t[2].as_ref()))
}

/// RFC 6901 escaping for one pointer token.
fn escape_token(tok: &str) -> String {
    tok.replace('~', "~0").replace('/', "~1")
}

/// Machine-readable component address stashed in
/// [`CallHierarchyItem::detail`] so `incoming_calls`/`outgoing_calls` can
/// recover the exact `(uri, pointer)` identity.
fn component_detail(sec: &str, name: &str) -> String {
    format!("/components/{}/{}", escape_token(sec), escape_token(name))
}

/// Recovers `(document, pointer)` from an item previously built by
/// [`prepare_call_hierarchy`].
fn target_of_item(item: &CallHierarchyItem) -> Option<(Uri, Pointer)> {
    let uri = Uri::parse(item.uri.as_str()).ok()?;
    let ptr = Pointer::parse(item.detail.as_deref()?).ok()?;
    component_of(&ptr)?;
    Some((uri, ptr))
}

/// Parses a document URI into an LSP URL, or `None`.
fn to_url(uri: &Uri) -> Option<Url> {
    Url::parse(uri.as_str()).ok()
}

/// Builds a call-hierarchy item whose visible range is the given key span
/// (both `range` and `selection_range`; selection ⊆ range holds trivially).
fn build_item(
    uri: &Uri,
    name: &str,
    kind: SymbolKind,
    detail: String,
    bytes: &[u8],
    li: &suspect_source::LineIndex,
    sel: Range<usize>,
) -> Option<CallHierarchyItem> {
    let range = lsp_range(bytes, li, sel);
    Some(CallHierarchyItem {
        name: name.to_owned(),
        kind,
        tags: None,
        detail: Some(detail),
        uri: to_url(uri)?,
        range,
        selection_range: range,
        data: None,
    })
}

/// The call-hierarchy item describing component `<Name>` in document
/// `handle`, anchored on its declaring key span.
fn component_item(handle: &DocHandle<'_>, sec: &str, name: &str) -> Option<CallHierarchyItem> {
    let ptr = Pointer::from_tokens(vec!["components".into(), sec.into(), name.into()]);
    let value = handle.doc().root().pointer(&ptr)?;
    let inner = handle.doc().inner();
    let kind = if sec == "schemas" {
        SymbolKind::STRUCT
    } else {
        SymbolKind::CLASS
    };
    build_item(
        handle.uri(),
        name,
        kind,
        component_detail(sec, name),
        inner.bytes(),
        inner.line_index(),
        key_span(value.syntax()),
    )
}

/// The call-hierarchy item describing operation `{method} {path}`, anchored
/// on the method key span.
fn operation_item(
    handle: &DocHandle<'_>,
    path_tok: &str,
    method: &str,
) -> Option<CallHierarchyItem> {
    let ptr = Pointer::from_tokens(vec!["paths".into(), path_tok.into(), method.into()]);
    let node = handle.doc().root().pointer(&ptr)?;
    let inner = handle.doc().inner();
    let name = format!("{method} {}", path_tok);
    build_item(
        handle.uri(),
        &name,
        SymbolKind::METHOD,
        ptr.to_path(),
        inner.bytes(),
        inner.line_index(),
        key_span(node.syntax()),
    )
}

/// Attributes a referencing site (the pointer of the mapping holding the
/// `$ref`) to its enclosing named thing: a component under `components`, or
/// the operation under `paths`. `None` for unattributable sites.
fn enclosing_item(handle: &DocHandle<'_>, site: &Pointer) -> Option<CallHierarchyItem> {
    if let Some((sec, name)) = component_of(site) {
        return component_item(handle, sec, name);
    }
    let t = site.tokens();
    if t.len() >= 3 && t[0].as_ref() == "paths" && HTTP_METHODS.contains(&t[2].as_ref()) {
        return operation_item(handle, &t[1], &t[2]);
    }
    None
}

/// Re-derives the node at `range` inside a workspace-borrowed document so it
/// can be handed to resolution APIs requiring the workspace lifetime. Fails
/// when the live buffer diverges from the workspace copy.
fn ws_node<'ws>(handle: &DocHandle<'ws>, range: Range<usize>) -> Option<NodeRef<'ws>> {
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

/// Resolves a `$ref` VALUE node (workspace coordinates) through its full
/// chain.
fn resolve_ws_ref<'ws>(handle: &DocHandle<'ws>, ref_value: &SNode<'_>) -> Option<Resolution<'ws>> {
    let n = ws_node(handle, ref_value.byte_range())?;
    handle.resolve_ref_value(n).ok()
}

/// Converts a resolved node into a location-style definition.
fn resolution_to_def(resolution: Resolution<'_>) -> Option<Definition> {
    match resolution {
        Resolution::Node(n) => Some(Definition {
            uri: n.syntax().doc().uri().clone(),
            range: n.byte_range(),
        }),
        Resolution::WholeDoc(_) | Resolution::Cycle { .. } => None,
    }
}

/// Type of one LIVE schema node: follow `$ref` chains fully, or the inline
/// node itself.
fn live_schema_def(ws: &Workspace, low: &LowDoc, schema: NodeRef<'_>) -> Option<Definition> {
    match schema.get("$ref") {
        Some(rv) => {
            let r = rv.byte_range();
            navigation::goto_definition(ws, low, r.start + r.len() / 2)
        }
        None => Some(Definition {
            uri: low.uri().clone(),
            range: schema.byte_range(),
        }),
    }
}

/// Type of one workspace-coordinate schema node: follow `$ref` chains fully,
/// or the inline node itself.
fn ws_schema_def(handle: &DocHandle<'_>, schema: NodeRef<'_>) -> Option<Definition> {
    match schema.get("$ref") {
        Some(rv) => resolve_ws_ref(handle, rv.syntax()).and_then(resolution_to_def),
        None => Some(Definition {
            uri: handle.uri().clone(),
            range: schema.byte_range(),
        }),
    }
}

/// Typing schemas of a response object in workspace coordinates: one per
/// declared media type's `schema`.
fn ws_response_schemas(handle: &DocHandle<'_>, resp: NodeRef<'_>) -> Vec<Definition> {
    let mut out = Vec::new();
    let Some(content) = resp.get("content") else {
        return out;
    };
    for e in content.entries() {
        let Some(v) = e.value else { continue };
        if let Some(schema) = v.get("schema")
            && let Some(d) = ws_schema_def(handle, schema)
        {
            out.push(d);
        }
    }
    out
}

/// Definitions for a response status key: resolve `$ref` responses to their
/// component first, then collect each media type's schema.
fn response_defs(ws: &Workspace, low: &LowDoc, pair: &SNode<'_>) -> Vec<Definition> {
    let Some(value) = pair.child_by_field("value") else {
        return Vec::new();
    };
    let resp = NodeRef::new(value.content());
    if let Some(rv) = resp.get("$ref") {
        let Some(handle) = ws.get(low.uri()) else {
            return Vec::new();
        };
        return match resolve_ws_ref(&handle, rv.syntax()) {
            Some(Resolution::Node(target)) => ws_response_schemas(&handle, target),
            _ => Vec::new(),
        };
    }
    // Inline response: schemas are live nodes.
    let mut out = Vec::new();
    let Some(content) = resp.get("content") else {
        return out;
    };
    for e in content.entries() {
        let Some(v) = e.value else { continue };
        if let Some(schema) = v.get("schema") {
            // An inline schema's own location comes straight from the buffer;
            // a nested $ref resolves through the workspace engine.
            let d = if schema.get("$ref").is_some() {
                live_schema_def(ws, low, schema)
            } else {
                Some(Definition {
                    uri: low.uri().clone(),
                    range: schema.byte_range(),
                })
            };
            if let Some(d) = d {
                out.push(d);
            }
        }
    }
    out
}

/// Classifies the cursor site and produces its typing definitions.
fn typing_definitions(ws: &Workspace, low: &LowDoc, node: SNode<'_>) -> Option<Vec<Definition>> {
    let (pair, key_text) = match key_pair(node) {
        Some((p, k)) => {
            let kt = std::str::from_utf8(k.scalar_bytes()).ok()?.to_owned();
            (p, kt)
        }
        None => return parameter_name_value_def(ws, low, node),
    };
    let value = pair.child_by_field("value")?;
    let parent_key = ancestor_pair_key(&pair);

    // Media-type key under `content:` → its `schema`.
    if parent_key.as_deref() == Some("content") {
        let schema = NodeRef::new(value.content()).get("schema")?;
        return Some(vec![live_schema_def(ws, low, schema)?]);
    }
    // Response status key under `responses:` → response schema(s).
    if parent_key.as_deref() == Some("responses") && key_text.parse::<u16>().is_ok() {
        let defs = response_defs(ws, low, &pair);
        return (!defs.is_empty()).then_some(defs);
    }
    // Parameter name KEY → sibling `schema:` of the parameter object.
    if key_text == "name"
        && let Some(mapping) = pair.parent()
        && let Some(schema) = NodeRef::new(mapping.content()).get("schema")
    {
        return Some(vec![live_schema_def(ws, low, schema)?]);
    }
    // Array element schema keys.
    if matches!(key_text.as_str(), "items" | "prefixItems") {
        return Some(vec![live_schema_def(
            ws,
            low,
            NodeRef::new(value.content()),
        )?]);
    }
    // Property name under `properties:` / `patternProperties:` → value schema.
    if matches!(
        parent_key.as_deref(),
        Some("properties") | Some("patternProperties")
    ) {
        return Some(vec![live_schema_def(
            ws,
            low,
            NodeRef::new(value.content()),
        )?]);
    }
    None
}

/// Offset of the `$ref` value backing the declaration at `offset`, if any.
fn declaration_ref_offset(low: &LowDoc, offset: usize) -> Option<usize> {
    // Already on (or inside) a `$ref` value.
    if navigation::ref_value_node(low, offset).is_some() {
        return Some(offset);
    }
    let node = navigation::node_at(low, offset)?;
    // Key-side cases.
    if let Some((pair, key)) = key_pair(node) {
        let value = pair.child_by_field("value")?;
        // Property name whose schema is a `$ref`.
        if ancestor_pair_key(&pair).as_deref() == Some("properties")
            && let Some(rv) = NodeRef::new(value.content()).get("$ref")
        {
            return Some(rv.byte_range().start);
        }
        // Parameter name key inside a `$ref`-carrying parameter object.
        if key.scalar_bytes() == b"name"
            && let Some(mapping) = pair.parent()
            && let Some(rv) = NodeRef::new(mapping.content()).get("$ref")
        {
            return Some(rv.byte_range().start);
        }
        return None;
    }
    // Value side: the parameter NAME scalar of a `$ref`-carrying object.
    let pair = value_pair(node)?;
    let key = pair.child_by_field("key")?;
    if key.scalar_bytes() == b"name"
        && let Some(mapping) = pair.parent()
        && let Some(rv) = NodeRef::new(mapping.content()).get("$ref")
    {
        return Some(rv.byte_range().start);
    }
    None
}

/// Type definition for the cursor on a parameter's NAME VALUE (the scalar
/// after `name:`): the sibling `schema:` of the owning parameter object.
fn parameter_name_value_def(
    ws: &Workspace,
    low: &LowDoc,
    node: SNode<'_>,
) -> Option<Vec<Definition>> {
    let pair = value_pair(node)?;
    let key = pair.child_by_field("key")?;
    if key.scalar_bytes() != b"name" {
        return None;
    }
    let mapping = pair.parent()?;
    let schema = NodeRef::new(mapping.content()).get("schema")?;
    Some(vec![live_schema_def(ws, low, schema)?])
}

/// Converts a definition into an LSP location, reporting same-document
/// targets against the live buffer and foreign targets against their
/// workspace copies.
fn to_location(ws: &Workspace, live: &LowDoc, def: &Definition) -> Option<Location> {
    let (bytes, li) = if def.uri == *live.uri() {
        let inner = live.inner();
        (inner.bytes(), inner.line_index())
    } else {
        let handle = ws.get(&def.uri)?;
        let inner = handle.doc().inner();
        (inner.bytes(), inner.line_index())
    };
    Some(Location {
        uri: to_url(&def.uri)?,
        range: lsp_range(bytes, li, def.range.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::lsp_range;
    use std::sync::Arc;
    use suspect_ref::{Workspace, WorkspaceBuilder};
    use tower_lsp::lsp_types::{Position, Range as LspRange};

    const MAIN: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      parameters:
        - $ref: '#/components/parameters/LimitParam'
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PetsPage'
components:
  schemas:
    Pet:
      type: object
      properties:
        tag:
          $ref: '#/components/schemas/Tag'
        size:
          type: integer
    Tag:
      type: string
    Order:
      type: object
      properties:
        pet:
          $ref: '#/components/schemas/Pet'
    Alias:
      $ref: '#/components/schemas/Pet'
    Wrapped:
      type: object
      properties:
        inner:
          $ref: '#/components/schemas/Alias'
    PetsPage:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
  parameters:
    LimitParam:
      name: limit
      in: query
      schema:
        type: integer
"#;

    const EXTRA: &str = r#"
components:
  schemas:
    External:
      type: object
      properties:
        t:
          $ref: 'main.yaml#/components/schemas/Tag'
"#;

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::write(dir.join("main.yaml"), MAIN).unwrap();
        std::fs::write(dir.join("extra.yaml"), EXTRA).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        ws.load_all("extra.yaml").unwrap();
        Arc::new(ws)
    }

    /// Parses `text` under the real file URI so workspace lookups match.
    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let uri = Uri::from_path(&path).unwrap();
        LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn offset_in(text: &str, needle: &str) -> usize {
        let at = text.find(needle).expect("needle present");
        at + needle.len() / 2
    }

    fn rng(low: &LowDoc, r: std::ops::Range<usize>) -> LspRange {
        let inner = low.inner();
        lsp_range(inner.bytes(), inner.line_index(), r)
    }

    /// LSP range of the node at `ptr` in `low`.
    fn node_rng(low: &LowDoc, ptr: &str) -> LspRange {
        let n = low
            .root()
            .pointer(&Pointer::parse(ptr).unwrap())
            .expect("pointer resolves");
        rng(low, n.byte_range())
    }

    /// LSP range of the `$ref` value under the mapping at `ptr`.
    fn ref_value_rng(low: &LowDoc, ptr: &str) -> LspRange {
        let n = low
            .root()
            .pointer(&Pointer::parse(ptr).unwrap())
            .expect("pointer resolves");
        rng(low, n.get("$ref").expect("has $ref").byte_range())
    }

    /// LSP range of the declaring KEY of the node at `ptr`.
    fn key_rng(low: &LowDoc, ptr: &str) -> LspRange {
        let n = low
            .root()
            .pointer(&Pointer::parse(ptr).unwrap())
            .expect("pointer resolves");
        rng(low, key_span(n.syntax()))
    }

    fn main_url(low: &LowDoc) -> Url {
        Url::parse(low.uri().as_str()).unwrap()
    }

    #[test]
    fn prepare_from_component_key() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-prepare-key");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "Tag:"))
            .expect("component key prepares");
        assert_eq!(item.name, "Tag");
        assert_eq!(item.kind, SymbolKind::STRUCT);
        assert_eq!(item.uri, main_url(&low));
        assert_eq!(item.detail.as_deref(), Some("/components/schemas/Tag"));
        let expected = key_rng(&low, "/components/schemas/Tag");
        assert_eq!(item.range, expected);
        assert_eq!(item.selection_range, expected);
    }

    #[test]
    fn prepare_from_ref_value_resolves_target() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-prepare-ref");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let off = offset_in(MAIN, "'#/components/schemas/PetsPage'");
        let item = prepare_call_hierarchy(&ws, &low, off).expect("ref value prepares");
        assert_eq!(item.name, "PetsPage");
        assert_eq!(item.detail.as_deref(), Some("/components/schemas/PetsPage"));
        assert_eq!(
            item.selection_range,
            key_rng(&low, "/components/schemas/PetsPage")
        );
    }

    #[test]
    fn prepare_rejects_cursor_not_on_component_or_ref() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-prepare-none");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        // On an unrelated key (`title`) and on a nested non-component key.
        assert!(prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "title")).is_none());
        assert!(prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "type: object")).is_none());
        // Past the end of the buffer.
        assert!(prepare_call_hierarchy(&ws, &low, MAIN.len() + 64).is_none());
    }

    #[test]
    fn prepare_empty_doc_is_none() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-prepare-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "empty.yaml", "");
        assert!(prepare_call_hierarchy(&ws, &low, 0).is_none());
    }

    #[test]
    fn incoming_groups_operation_referrer() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-in-op");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "PetsPage:")).unwrap();
        let calls = incoming_calls(&ws, &item);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "get /pets");
        assert_eq!(calls[0].from.kind, SymbolKind::METHOD);
        assert_eq!(calls[0].from_ranges.len(), 1);
        assert_eq!(
            calls[0].from_ranges[0],
            ref_value_rng(
                &low,
                "/paths/~1pets/get/responses/200/content/application~1json/schema"
            )
        );
    }

    #[test]
    fn incoming_groups_every_component_referrer() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-in-many");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "Pet:")).unwrap();
        let calls = incoming_calls(&ws, &item);
        let names: Vec<&str> = calls.iter().map(|c| c.from.name.as_str()).collect();
        assert_eq!(names, vec!["Alias", "Order", "PetsPage", "Wrapped"]);
        for c in &calls {
            assert_eq!(c.from.kind, SymbolKind::STRUCT);
            assert_eq!(c.from_ranges.len(), 1);
        }
        // Order's range comes from its `pet` property's $ref value.
        let order = calls.iter().find(|c| c.from.name == "Order").unwrap();
        assert_eq!(
            order.from_ranges[0],
            ref_value_rng(&low, "/components/schemas/Order/properties/pet")
        );
    }

    #[test]
    fn incoming_crosses_file_boundaries() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-in-cross");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "Tag:")).unwrap();
        let calls = incoming_calls(&ws, &item);
        let names: Vec<&str> = calls.iter().map(|c| c.from.name.as_str()).collect();
        assert_eq!(names, vec!["External", "Pet"]);
        assert!(calls[0].from.uri.as_str().ends_with("extra.yaml"));
    }

    #[test]
    fn incoming_finds_parameter_referrers() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-in-param");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "LimitParam:")).unwrap();
        assert_eq!(item.kind, SymbolKind::CLASS);
        let calls = incoming_calls(&ws, &item);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "get /pets");
        assert_eq!(
            calls[0].from_ranges[0],
            ref_value_rng(&low, "/paths/~1pets/get/parameters/0")
        );
    }

    #[test]
    fn incoming_ignores_non_component_items() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-in-fake");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let mk = |detail: Option<&str>| CallHierarchyItem {
            name: "x".to_owned(),
            kind: SymbolKind::STRUCT,
            tags: None,
            detail: detail.map(str::to_owned),
            uri: Url::parse("file:///x.yaml").unwrap(),
            range: LspRange {
                start: Position::new(0, 0),
                end: Position::new(0, 0),
            },
            selection_range: LspRange {
                start: Position::new(0, 0),
                end: Position::new(0, 0),
            },
            data: None,
        };
        assert!(incoming_calls(&ws, &mk(None)).is_empty());
        assert!(incoming_calls(&ws, &mk(Some("not a pointer"))).is_empty());
        assert!(incoming_calls(&ws, &mk(Some("/paths/~1pets/get"))).is_empty());
    }

    #[test]
    fn outgoing_follows_direct_refs() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-out-direct");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "Pet:")).unwrap();
        let calls = outgoing_calls(&ws, &item);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "Tag");
        assert_eq!(
            calls[0].to.detail.as_deref(),
            Some("/components/schemas/Tag")
        );
        assert_eq!(
            calls[0].from_ranges[0],
            ref_value_rng(&low, "/components/schemas/Pet/properties/tag")
        );
    }

    #[test]
    fn outgoing_follows_chains_to_final_target() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-out-chain");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let item = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "\n    Wrapped:")).unwrap();
        let calls = outgoing_calls(&ws, &item);
        assert_eq!(calls.len(), 1);
        // Wrapped → Alias → Pet: the chain resolves to Pet.
        assert_eq!(calls[0].to.name, "Pet");
    }

    #[test]
    fn outgoing_covers_items_edges_and_empty_case() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-out-items");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let page = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "PetsPage:")).unwrap();
        let calls = outgoing_calls(&ws, &page);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "Pet");
        assert_eq!(
            calls[0].from_ranges[0],
            ref_value_rng(&low, "/components/schemas/PetsPage/items")
        );
        // A component with no refs has no outgoing calls.
        let tag = prepare_call_hierarchy(&ws, &low, offset_in(MAIN, "Tag:")).unwrap();
        assert!(outgoing_calls(&ws, &tag).is_empty());
    }

    #[test]
    fn declaration_on_property_key_lands_on_target_key_line() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-decl-prop");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs =
            declaration(&ws, &low, offset_in(MAIN, "tag:")).expect("declaration on $ref property");
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, main_url(&low));
        assert_eq!(locs[0].range, key_rng(&low, "/components/schemas/Tag"));
    }

    #[test]
    fn declaration_on_ref_value_lands_on_key_line() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-decl-refval");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let off = offset_in(MAIN, "'#/components/parameters/LimitParam'");
        let locs = declaration(&ws, &low, off).expect("declaration on ref value");
        assert_eq!(
            locs[0].range,
            key_rng(&low, "/components/parameters/LimitParam")
        );
    }

    #[test]
    fn declaration_returns_none_without_ref() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-decl-none");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        // Property with inline schema: no declaration.
        assert!(declaration(&ws, &low, offset_in(MAIN, "size:")).is_none());
        // Cursor not on anything declarable.
        assert!(declaration(&ws, &low, offset_in(MAIN, "title")).is_none());
    }

    #[test]
    fn declaration_empty_doc_is_none() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-decl-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "empty.yaml", "");
        assert!(declaration(&ws, &low, 0).is_none());
    }

    #[test]
    fn type_definition_property_follows_ref() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-prop");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs = type_definition(&ws, &low, offset_in(MAIN, "tag:")).expect("typed property");
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].range, node_rng(&low, "/components/schemas/Tag"));
    }

    #[test]
    fn type_definition_inline_property_points_at_itself() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-inline");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs = type_definition(&ws, &low, offset_in(MAIN, "size:"))
            .expect("inline property types to itself");
        assert_eq!(
            locs[0].range,
            node_rng(&low, "/components/schemas/Pet/properties/size")
        );
    }

    #[test]
    fn type_definition_media_type_key() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-media");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs =
            type_definition(&ws, &low, offset_in(MAIN, "application/json")).expect("media type");
        assert_eq!(
            locs[0].range,
            node_rng(&low, "/components/schemas/PetsPage")
        );
    }

    #[test]
    fn type_definition_status_key() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-status");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs = type_definition(&ws, &low, offset_in(MAIN, "'200'")).expect("status key");
        assert_eq!(locs.len(), 1);
        assert_eq!(
            locs[0].range,
            node_rng(&low, "/components/schemas/PetsPage")
        );
    }

    #[test]
    fn type_definition_follows_chain_fully() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-chain");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        // inner → Alias → Pet.
        let locs = type_definition(&ws, &low, offset_in(MAIN, "inner:")).expect("chain");
        assert_eq!(locs[0].range, node_rng(&low, "/components/schemas/Pet"));
    }

    #[test]
    fn type_definition_parameter_name() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-param");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs = type_definition(&ws, &low, offset_in(MAIN, "limit")).expect("parameter name");
        assert_eq!(
            locs[0].range,
            node_rng(&low, "/components/parameters/LimitParam/schema")
        );
    }

    #[test]
    fn type_definition_items_key() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-items");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let locs = type_definition(&ws, &low, offset_in(MAIN, "items:")).expect("items");
        assert_eq!(locs[0].range, node_rng(&low, "/components/schemas/Pet"));
    }

    #[test]
    fn type_definition_unrecognized_sites_are_none() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-none");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        assert!(type_definition(&ws, &low, offset_in(MAIN, "title")).is_none());
        assert!(type_definition(&ws, &low, MAIN.len() + 64).is_none());
    }

    #[test]
    fn type_definition_empty_doc_is_none() {
        let dir = std::env::temp_dir().join("suspect-lsp-ch-typedef-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "empty.yaml", "");
        assert!(type_definition(&ws, &low, 0).is_none());
    }
}
