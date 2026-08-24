//! Code lenses, `$ref` document links, and monikers.
//!
//! All three features are pure functions over the live [`LowDoc`] plus the
//! ref [`Workspace`]: lenses advertise component reference counts and
//! operation ids, links turn `$ref` values into clickable cross-file
//! targets, and monikers emit stable workspace-wide identifiers for
//! external indexers.

use std::path::PathBuf;

use crate::state::OpenDoc;

use serde_json::{Value, json};
use suspect_low::{LowDoc, NodeRef, Pointer, ValueKind};
use suspect_ref::{ParsedRef, Workspace};
use suspect_source::Uri;
use tower_lsp::lsp_types::{
    CodeAction, CodeLens, Command, DocumentLink, Moniker, MonikerKind, Position, Range, TextEdit,
    UniquenessLevel, Url,
};

use crate::navigation::{node_at, value_anchor};
use crate::state::lsp_range;
use crate::symbols::METHODS;

use suspect_syntax::{SNode, SyntaxKind};

/// Command invoked by a resolved lens to list the references it counted.
pub const SHOW_REFS_COMMAND: &str = "suspect.showRefs";

/// Command suggested by a resolved lens when an operation lacks an id.
pub const ADD_OPERATION_ID_COMMAND: &str = "suspect.addOperationId";

// ---- code lens ----------------------------------------------------------

/// One unresolved lens above each component schema key (`[N] references`
/// once resolved) and each operation method key (its `operationId`).
///
/// Lenses carry only their range and an opaque `data` pointer; the counts
/// and command payloads are attached by [`code_lens_resolve`].
#[must_use]
pub fn code_lens(_ws: &Workspace, doc: &LowDoc) -> Vec<CodeLens> {
    let bytes = doc.inner().bytes();
    let li = doc.inner().line_index();
    let root = doc.root();
    let mut out = Vec::new();

    if let Some(components) = root.get("components") {
        for section in components.entries() {
            let Some(sec_val) = section.value else {
                continue;
            };
            for entry in sec_val.entries() {
                let Some(value) = entry.value else { continue };
                let Some(key) = key_node_of(&value) else {
                    continue;
                };
                let ptr = Pointer::root()
                    .push("components")
                    .push(section.key)
                    .push(entry.key);
                out.push(CodeLens {
                    range: lsp_range(bytes, li, key.byte_range()),
                    command: None,
                    data: Some(json!({ "ptr": ptr.to_path() })),
                });
            }
        }
    }

    if let Some(paths) = root.get("paths") {
        for path in paths.entries() {
            let Some(item) = path.value else { continue };
            for method in METHODS {
                let Some(op) = item.get(method) else { continue };
                let Some(key) = key_node_of(&op) else {
                    continue;
                };
                let ptr = Pointer::root().push("paths").push(path.key).push(method);
                out.push(CodeLens {
                    range: lsp_range(bytes, li, key.byte_range()),
                    command: None,
                    data: Some(json!({ "ptr": ptr.to_path() })),
                });
            }
        }
    }
    out
}

/// Fills a lens from [`code_lens`] with its command payload.
///
/// Component lenses become `{N} references` + `suspect.showRefs`, counting
/// direct `$ref` edges across every loaded workspace document that target
/// the component. Operation lenses show the operation's `operationId` (or
/// suggest `suspect.addOperationId` when missing). A lens without usable
/// `data` — stale or foreign — is returned unchanged rather than erroring.
#[must_use]
pub fn code_lens_resolve(ws: &Workspace, doc: &LowDoc, mut lens: CodeLens) -> CodeLens {
    let Some(data_ptr) = lens_data_str(&lens.data, "ptr") else {
        return lens;
    };
    let Ok(ptr) = Pointer::parse(data_ptr) else {
        return lens;
    };
    let tokens = ptr.tokens();
    let first = tokens.first().map(|t| t.as_ref());

    let command = match first {
        Some("components") if tokens.len() >= 3 => Command {
            title: format!("{} references", count_component_refs(ws, doc.uri(), &ptr)),
            command: SHOW_REFS_COMMAND.to_owned(),
            arguments: Some(vec![json!(doc.uri().as_str()), json!(data_ptr)]),
        },
        Some("paths") if tokens.len() >= 3 => match operation_id(doc, &ptr) {
            Some(id) => Command {
                title: id,
                command: SHOW_REFS_COMMAND.to_owned(),
                arguments: Some(vec![json!(doc.uri().as_str()), json!(data_ptr)]),
            },
            None => Command {
                title: "add operationId".to_owned(),
                command: ADD_OPERATION_ID_COMMAND.to_owned(),
                arguments: Some(vec![json!(doc.uri().as_str()), json!(data_ptr)]),
            },
        },
        _ => return lens,
    };
    lens.command = Some(command);
    lens
}

/// Counts loaded-workspace `$ref` edges whose target is `home` + `ptr`.
fn count_component_refs(ws: &Workspace, home: &Uri, ptr: &Pointer) -> usize {
    let mut n = 0usize;
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        for edge in handle.edges().iter() {
            let hits = match &edge.parsed {
                ParsedRef::Local(p) => &uri == home && p == ptr,
                ParsedRef::External { uri, pointer } => uri == home && pointer == ptr,
                ParsedRef::PlainName(_) => false,
            };
            if hits {
                n += 1;
            }
        }
    }
    n
}

/// The `operationId` string of the operation at `ptr`, if present.
fn operation_id(doc: &LowDoc, ptr: &Pointer) -> Option<String> {
    doc.root()
        .pointer(ptr)?
        .get("operationId")?
        .as_str()
        .map(str::to_owned)
}

/// Reads a string field out of a lens/link's opaque `data` object.
fn lens_data_str<'a>(data: &'a Option<Value>, field: &str) -> Option<&'a str> {
    data.as_ref()?.get(field)?.as_str()
}

pub(crate) fn key_node_of<'d>(value: &NodeRef<'d>) -> Option<SNode<'d>> {
    let mut cur = *value.syntax();
    loop {
        if cur.kind() == SyntaxKind::Pair
            && let Some(k) = cur.child_by_field("key")
        {
            return Some(k);
        }
        cur = cur.parent()?;
    }
}

// ---- document link ------------------------------------------------------

/// Turns every path-or-URL `$ref` value into a clickable link.
///
/// Local pointers target the document's own URI with a fragment; external
/// pointers join against the base URI to an absolute file URL. Plain-name
/// fragments (`#Pet`) address no path and are skipped. The tooltip is the
/// resolved pointer (`#/components/schemas/Pet`); `data` carries the raw
/// pointer for [`document_link_resolve`].
#[must_use]
pub fn document_link(_ws: &Workspace, doc: &LowDoc) -> Vec<DocumentLink> {
    let bytes = doc.inner().bytes();
    let li = doc.inner().line_index();
    let mut out = Vec::new();
    for r in live_refs(doc) {
        let (target_uri, frag) = match &r.parsed {
            ParsedRef::Local(p) => (doc.uri().clone(), p.to_path()),
            ParsedRef::External { uri, pointer } => (uri.clone(), pointer.to_path()),
            // Plain-name fragments name no path or URL; nothing to link.
            ParsedRef::PlainName(_) => continue,
        };
        let Ok(mut url) = Url::parse(target_uri.as_str()) else {
            continue;
        };
        // Root-pointer external refs link the whole file: no fragment.
        if !frag.is_empty() {
            url.set_fragment(Some(&frag));
        }
        let tooltip = (!frag.is_empty()).then(|| format!("#{frag}"));
        out.push(DocumentLink {
            range: lsp_range(bytes, li, r.at),
            target: Some(url),
            tooltip,
            data: Some(json!({ "ptr": frag })),
        });
    }
    out
}

/// Attaches a human summary of the linked target's kind as the tooltip.
///
/// Recognizes component sections (schema/parameter/response/…) and path
/// items; unknown or missing `data` returns the link unchanged.
#[must_use]
pub fn document_link_resolve(mut link: DocumentLink) -> DocumentLink {
    let Some(data_ptr) = lens_data_str(&link.data, "ptr") else {
        return link;
    };
    let Ok(ptr) = Pointer::parse(data_ptr) else {
        return link;
    };
    link.tooltip = Some(describe_pointer(&ptr));
    link
}

/// Human description of what an RFC 6901 pointer addresses.
fn describe_pointer(ptr: &Pointer) -> String {
    let token = |i: usize| ptr.tokens().get(i).map(|t| t.as_ref());
    match (token(0), token(1), token(2)) {
        (Some("components"), Some(section), Some(name)) => {
            let kind = match section {
                "schemas" => "schema",
                "parameters" => "parameter",
                "responses" => "response",
                "pathItems" => "path item",
                "requestBodies" => "request body",
                "headers" => "header",
                "examples" => "example",
                "links" => "link",
                "callbacks" => "callback",
                "securitySchemes" => "security scheme",
                _ => "component",
            };
            format!("{kind} '{name}'")
        }
        (Some("paths"), Some(path), _) => format!("path item '{path}'"),
        _ if ptr.is_root() => "whole document".to_owned(),
        _ => format!("spec element at #{}", ptr),
    }
}

// ---- moniker ------------------------------------------------------------

/// Monikers for the element at byte `offset`.
///
/// On a `$ref` pointing into another document's `components/*`, emits one
/// [`MonikerKind::Import`] carrying the *target's* identifier; anywhere
/// inside a named component entry emits one [`MonikerKind::Export`] for
/// that component. Identifiers are stable across files:
/// `urn:suspect:openapi:<workspace-relative path>:<pointer>` of the owning
/// document. Positions on nothing addressable yield `None`.
#[must_use]
pub fn moniker(ws: &Workspace, doc: &LowDoc, offset: usize) -> Option<Vec<Moniker>> {
    // Import: cursor sits on a $ref value with a cross-file component target.
    for r in live_refs(doc) {
        if r.at.start <= offset && offset <= r.at.end {
            return match &r.parsed {
                ParsedRef::External { uri, pointer } if is_component_pointer(pointer) => {
                    Some(vec![Moniker {
                        scheme: "suspect".to_owned(),
                        identifier: stable_identifier(ws, uri, pointer),
                        unique: UniquenessLevel::Document,
                        kind: Some(MonikerKind::Import),
                    }])
                }
                _ => None,
            };
        }
    }

    // Export: cursor inside a named component entry under components/*.
    let node = node_at(doc, offset)?;
    let anchor = NodeRef::new(value_anchor(node));
    let ptr = anchor.path_from_root();
    if !is_component_pointer(&ptr) {
        return None;
    }
    // Identify the component itself, not the position inside its body:
    // truncate to components/<section>/<Name>.
    let comp = Pointer::from_tokens(ptr.tokens()[..3].to_vec());
    Some(vec![Moniker {
        scheme: "suspect".to_owned(),
        identifier: stable_identifier(ws, doc.uri(), &comp),
        unique: UniquenessLevel::Document,
        kind: Some(MonikerKind::Export),
    }])
}

/// True for pointers addressing a named entry under `components/<section>/`.
fn is_component_pointer(ptr: &Pointer) -> bool {
    let tokens = ptr.tokens();
    tokens.len() >= 3 && tokens[0].as_ref() == "components"
}

/// Stable cross-file identifier for a document position:
/// `urn:suspect:openapi:<rel-path>:<pointer>` with the pointer's leading
/// slash dropped.
fn stable_identifier(ws: &Workspace, uri: &Uri, ptr: &Pointer) -> String {
    let tail = ptr.to_path();
    let tail = tail.trim_start_matches('/');
    format!(
        "urn:suspect:openapi:{}:{}",
        relative_doc_path(ws, uri),
        tail
    )
}

/// The document's canonical path relative to the workspace's common root
/// directory (the closest available proxy for the builder root, which the
/// workspace does not expose); absolute when no common root exists.
fn relative_doc_path(ws: &Workspace, uri: &Uri) -> String {
    let Some(path) = uri.as_path() else {
        return uri.as_str().to_owned();
    };
    let text = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    if let Some(root) = common_root_dir(ws)
        && let Ok(rel) = path.strip_prefix(&root)
    {
        return text(rel);
    }
    text(&path)
}

/// Longest directory prefix shared by every loaded document's parent dir.
fn common_root_dir(ws: &Workspace) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = ws
        .uris()
        .iter()
        .filter_map(|u| u.as_path())
        .filter_map(|p| p.parent().map(|d| d.to_path_buf()))
        .collect();
    dirs.sort();
    dirs.dedup();
    let mut prefix = dirs.first()?.clone();
    for dir in &dirs[1..] {
        while !dir.starts_with(&prefix) {
            prefix = prefix.parent()?.to_path_buf();
        }
    }
    Some(prefix)
}

// ---- live $ref scan -----------------------------------------------------

/// One `$ref` occurrence found in the live buffer.
struct LiveRef {
    /// Byte range of the `$ref` value node (the string).
    at: std::ops::Range<usize>,
    /// Parsed address: local pointer, external URI plus pointer, or plain
    /// name.
    parsed: ParsedRef,
}

/// Scans the open document for `$ref` values so links and monikers reflect
/// unsaved edits. Stack walk over the semantic tree with alias expansion;
/// duplicates collapse by byte range and an on-path set guards alias cycles.
fn live_refs(low: &LowDoc) -> Vec<LiveRef> {
    type Child<'d> = (Option<Box<str>>, NodeRef<'d>);

    struct Frame<'d> {
        children: Vec<Child<'d>>,
        next: usize,
    }
    fn children_of(node: NodeRef<'_>) -> Vec<Child<'_>> {
        match node.kind() {
            ValueKind::Object => node
                .entries()
                .into_iter()
                .filter_map(|e| e.value.map(|v| (Some(Box::from(e.key)), v)))
                .collect(),
            ValueKind::Array => node.items().into_iter().map(|v| (None, v)).collect(),
            _ => Vec::new(),
        }
    }

    let root = low.root();
    let mut out = Vec::new();
    let mut seen: Vec<std::ops::Range<usize>> = Vec::new();
    let mut on_path: Vec<std::ops::Range<usize>> = vec![root.byte_range()];
    let mut stack: Vec<Frame<'_>> = vec![Frame {
        children: children_of(root),
        next: 0,
    }];
    while let Some(frame) = stack.last_mut() {
        let Some((key, child)) = frame.children.get(frame.next).cloned() else {
            stack.pop();
            on_path.pop();
            continue;
        };
        frame.next += 1;
        // A `$ref` value is a string keyed `$ref` in its owning mapping.
        if child.kind() == ValueKind::Str && key.as_deref() == Some("$ref") {
            if seen.contains(&child.byte_range()) {
                continue;
            }
            seen.push(child.byte_range());
            // Block scalars must be decoded, not read as raw source slices.
            let decoded = child.decoded_scalar();
            if let Ok(raw) = std::str::from_utf8(&decoded).map(str::trim)
                && let Some(parsed) = parse_ref_string(low.uri(), raw)
            {
                out.push(LiveRef {
                    at: child.byte_range(),
                    parsed,
                });
            }
            continue;
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
                children: children_of(child),
                next: 0,
            });
        }
    }
    out
}

/// Parses a raw `$ref` string against a base document URI: fragment-only
/// refs are local, anything with a document part joins against the base,
/// and fragments percent-decode before RFC 6901 parsing. Unparseable refs
/// yield `None`.
pub(crate) fn parse_ref_string(base: &Uri, raw: &str) -> Option<ParsedRef> {
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

/// Command served by `executeCommand` to reveal a resolved `$ref` target.
pub const OPEN_REF_COMMAND: &str = "suspect.openRefTarget";

/// "Open referenced definition" for a `$ref` value at the request range.
///
/// The returned code action carries a [`OPEN_REF_COMMAND`] command with
/// the absolute target URI plus line/column baked into its arguments —
/// `executeCommand` requests carry no cursor, so the position must travel
/// with the action. Returns `None` off-`$ref` or when the target document
/// is not loaded.
#[must_use]
pub fn open_ref_action(ws: &Workspace, doc: &OpenDoc, range: Range) -> Option<CodeAction> {
    use crate::navigation::node_at;
    use tower_lsp::lsp_types::{CodeActionKind, Command};

    let inner = doc.low.inner();
    let offset = crate::state::offset_of_utf16(
        inner.bytes(),
        inner.line_index(),
        range.start.line,
        range.start.character,
    )?;
    // Climb from the cursor node to an enclosing `$ref:` pair.
    let mut cur = node_at(&doc.low, offset)?;
    loop {
        let is_ref_pair = cur
            .child_by_field("key")
            .is_some_and(|k| String::from_utf8_lossy(k.content().scalar_bytes()) == "$ref");
        if is_ref_pair {
            break;
        }
        cur = cur.parent()?;
    }
    let value = cur.child_by_field("value")?;
    let raw = String::from_utf8_lossy(value.content().scalar_bytes()).into_owned();
    let home = Uri::parse(doc.low.uri().as_str()).ok()?;
    let (target_url, pos) = ref_target_position(ws, &home, &raw)?;
    Some(CodeAction {
        title: format!("Open referenced definition ({raw})"),
        kind: Some(CodeActionKind::new("refactor.suspect.openTarget")),
        command: Some(Command {
            title: "Open referenced definition".to_owned(),
            command: OPEN_REF_COMMAND.to_owned(),
            arguments: Some(vec![
                serde_json::json!(target_url.as_str()),
                serde_json::json!(pos.line),
                serde_json::json!(pos.character),
            ]),
        }),
        ..CodeAction::default()
    })
}

/// Resolves a raw `$ref` string to a clickable location.
///
/// Returns the canonical target URL and the position of the target key's
/// start. Fragment-only refs stay in the home document; plain-name anchors
/// are unsupported.
#[must_use]
pub fn ref_target_position(ws: &Workspace, base: &Uri, raw: &str) -> Option<(Url, Position)> {
    use tower_lsp::lsp_types::Position;

    match parse_ref_string(base, raw)? {
        ParsedRef::Local(ptr) => {
            let handle = ws.get(base)?;
            let low = handle.doc();
            let inner = low.inner();
            let offset = pointer_offset_from(inner.root(), &ptr)?;
            let (line, character) = inner.line_index().line_col_utf16(inner.bytes(), offset);
            Some((
                Url::parse(base.as_str()).ok()?,
                Position { line, character },
            ))
        }
        ParsedRef::External { uri, pointer } => {
            let handle = ws.get(&uri)?;
            let low = handle.doc();
            let inner = low.inner();
            let offset = pointer_offset_from(inner.root(), &pointer)?;
            let (line, character) = inner.line_index().line_col_utf16(inner.bytes(), offset);
            Some((Url::parse(uri.as_str()).ok()?, Position { line, character }))
        }
        ParsedRef::PlainName(_) => None,
    }
}

/// Byte offset of the JSON-pointer target's key inside a document root,
/// following one path segment per mapping child.
fn pointer_offset_from(root: SNode<'_>, ptr: &Pointer) -> Option<usize> {
    // Walks Pair nodes explicitly via `child_by_field("key")`:
    // `scalar_bytes()` on a non-scalar wrapper returns whole-node text, so
    // comparing raw children against pointer tokens never matches.
    let mut cur = root.content();
    let tokens = ptr.tokens();
    let last = tokens.len().checked_sub(1)?;
    for (i, token) in tokens.iter().enumerate() {
        let target = token.as_ref();
        let mut found = None;
        for child in cur.children() {
            if child.kind() != SyntaxKind::Pair {
                continue;
            }
            let Some(k) = child.child_by_field("key") else {
                continue;
            };
            if k.scalar_bytes() == target.as_bytes() {
                found = Some(child);
                break;
            }
        }
        let pair = found?;
        if i == last {
            return Some(pair.child_by_field("key")?.start_byte());
        }
        cur = unwrap_to_mapping(pair.child_by_field("value")?)?;
    }
    None
}

/// Descends wrapper/error nodes to the mapping they hold.
///
/// The YAML grammar occasionally wraps block mappings in an `ERROR` node
/// (recovery artifacts); pointer walks must see through them.
fn unwrap_to_mapping(node: SNode<'_>) -> Option<SNode<'_>> {
    let mut cur = node;
    loop {
        match cur.kind() {
            SyntaxKind::Mapping => return Some(cur),
            SyntaxKind::Stream | SyntaxKind::Document | SyntaxKind::Error => {
                cur = cur.first_meaningful_child()?;
            }
            _ => return None,
        }
    }
}

/// A resolved raw `$ref` string, rendered for hover/completion display.
pub struct RefTarget {
    /// `→ Name (file)` summary line.
    pub detail: String,
    /// Fenced-code excerpt of the target's source text.
    pub markdown_excerpt: String,
}

/// Resolves a raw `$ref` string (`#/a/b`, `other.yaml#/x`, `#Anchor`)
/// against its owning document and renders the target for display.
///
/// Returns `None` for unparseable refs, plain-name anchors without an
/// index hit, cycles, and documents missing from the workspace — callers
/// degrade gracefully by showing the unresolved item as-is.
#[must_use]
pub fn resolve_ref_string(ws: &Workspace, base: &Uri, raw: &str) -> Option<RefTarget> {
    use suspect_ref::{DocHandle, Resolution};

    let handle_for = |uri: &Uri| -> Option<DocHandle<'_>> { ws.get(uri) };
    let parsed = parse_ref_string(base, raw)?;
    let resolution = match parsed {
        ParsedRef::Local(ptr) => {
            let home = handle_for(base)?;
            home.resolve_pointer(home.id(), &ptr).ok()?
        }
        ParsedRef::External { uri, pointer } => {
            let target = handle_for(&uri)?;
            target.resolve_pointer(target.id(), &pointer).ok()?
        }
        ParsedRef::PlainName(_) => return None,
    };
    let (name, file, source_bytes, range, lang) = match resolution {
        Resolution::Node(target) => {
            let doc = target.syntax().doc();
            let name = target
                .path_from_root()
                .tokens()
                .last()
                .map_or_else(|| "(root)".to_owned(), |t| t.to_string());
            (
                name,
                basename(doc.uri().as_str()).to_owned(),
                doc.bytes(),
                target.byte_range(),
                lang_of(doc.format()),
            )
        }
        Resolution::WholeDoc(id) => {
            let uri = ws
                .uris()
                .into_iter()
                .find(|u| ws.get(u).is_some_and(|h| h.id() == id))?;
            let low = ws.get(&uri)?.doc();
            let inner = low.inner();
            (
                "(document)".to_owned(),
                basename(uri.as_str()).to_owned(),
                inner.bytes(),
                0..inner.bytes().len(),
                lang_of(low.format()),
            )
        }
        Resolution::Cycle { .. } => return None,
    };
    Some(RefTarget {
        detail: format!("→ {name} ({file})"),
        markdown_excerpt: format!(
            "```{lang}\n{}\n```",
            crate::navigation::excerpt(source_bytes, range, 10)
        ),
    })
}

/// `json`/`yaml` fence language for a document format.
fn lang_of(format: suspect_syntax::Format) -> &'static str {
    match format {
        suspect_syntax::Format::Json => "json",
        suspect_syntax::Format::Yaml => "yaml",
    }
}

/// Final path segment of a URI string, for `(file)` summaries.
pub(crate) fn basename(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

/// Resolves a JSON-pointer string (`#/a/b~1c`) against `low` and returns the
/// byte offset of the value's key node.
#[must_use]
pub fn pointer_offset(low: &LowDoc, ptr: &str) -> Option<usize> {
    let inner = low.inner();
    let decoded = ptr.trim_start_matches('#');
    let mut cur = inner.root().content();
    let segs: Vec<String> = decoded
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let last = segs.len().checked_sub(1)?;
    for (i, seg) in segs.iter().enumerate() {
        let mut found = None;
        for child in cur.children() {
            if child.kind() != SyntaxKind::Pair {
                continue;
            }
            let Some(k) = child.child_by_field("key") else {
                continue;
            };
            if k.scalar_bytes() == seg.as_bytes() {
                found = Some(child);
                break;
            }
        }
        let pair = found?;
        if i == last {
            return Some(pair.start_byte());
        }
        cur = pair.child_by_field("value")?;
    }
    None
}

/// Builds an insert edit adding a generated `operationId` under the operation
/// at `ptr` (`/paths/~1pets/get`). Idempotent: returns `None` when the
/// operation already declares one or the pointer is not an operation.
#[must_use]
pub fn operation_id_edit(doc: &OpenDoc, ptr: &str) -> Option<TextEdit> {
    use tower_lsp::lsp_types::{Position, Range};
    let inner = doc.low.inner();
    let bytes = inner.bytes();
    let decoded = ptr.replace("~1", "/").replace("~0", "~");
    let segs: Vec<&str> = decoded
        .trim_start_matches('#')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segs.len() < 3 || segs.first() != Some(&"paths") {
        return None;
    }
    let path_key = segs.get(segs.len() - 2)?;
    let method = segs.last()?.to_ascii_lowercase();

    let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    let mut path_indent = 0usize;
    let mut found: Option<(usize, usize)> = None; // (path_line, method_line)
    for (i, raw) in lines.iter().enumerate() {
        let text = std::str::from_utf8(raw).ok()?;
        let trimmed = text.trim_start();
        if trimmed == format!("{}:", path_key) {
            path_indent = text.len() - trimmed.len();
            // find method key beneath
            for (j, line2) in lines.iter().enumerate().skip(i + 1) {
                let Some(l2) = std::str::from_utf8(line2).ok() else {
                    break;
                };
                let t2 = l2.trim_start();
                if t2.is_empty() {
                    continue;
                }
                if l2.len() - t2.len() <= path_indent {
                    break;
                }
                let rest = &t2[method.len()..];
                if t2.starts_with(&method)
                    && rest.trim_start().starts_with(':')
                    && rest.contains("operationId")
                {
                    return None;
                }
                if t2.starts_with(&method) && rest.trim_start().starts_with(':') {
                    found = Some((i, j));
                    break;
                }
            }
            break;
        }
    }
    let (_pl, mline) = found?;
    let id = format!("{method}_{}", path_key.trim_matches('/').replace('/', "_"));
    let indent = " ".repeat(path_indent + 2);
    let pos = Position {
        line: mline as u32 + 1,
        character: 0,
    };
    Some(TextEdit {
        range: Range::new(pos, pos),
        new_text: format!("{}operationId: {}\n", indent, id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_ref::{Workspace, WorkspaceBuilder};

    const MAIN: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
    post:
      responses:
        '201':
          description: created
components:
  schemas:
    Pet:
      type: object
    External:
      properties:
        list:
          $ref: 'schemas.yaml#/components/schemas/PetList'
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

    #[test]
    fn open_ref_action_resolves_local_and_cross_file_targets() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-openref");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let text = MAIN.to_owned();
        let low = low_at(&dir, "main.yaml", &text);
        let doc = crate::state::OpenDoc::parse(low.uri().clone(), text.clone());

        // Cursor inside the local `$ref: '#/components/schemas/Pet'`.
        let off = text.find("#/components/schemas/Pet'").unwrap();
        let (line, col) = {
            let inner = low.inner();
            inner.line_index().line_col_utf16(inner.bytes(), off)
        };
        let range = Range::new(
            Position {
                line,
                character: col,
            },
            Position {
                line,
                character: col,
            },
        );
        let action = open_ref_action(ws.as_ref(), &doc, range).expect("local target action");
        assert!(action.title.contains("Pet"), "{}", action.title);
        let cmd = action.command.as_ref().expect("command attached");
        assert_eq!(cmd.command, OPEN_REF_COMMAND);
        let args = cmd.arguments.as_ref().unwrap();
        // The baked position points at the Pet definition, not the cursor.
        let baked = args[1].as_u64().expect("line argument");
        assert_ne!(baked as u32, line, "target is elsewhere in the file");

        // Cursor inside the cross-file `schemas.yaml#/...` ref.
        let off2 = text
            .find("schemas.yaml#/components/schemas/PetList'")
            .unwrap();
        let (line2, col2) = {
            let inner = low.inner();
            inner.line_index().line_col_utf16(inner.bytes(), off2)
        };
        let range2 = Range::new(
            Position {
                line: line2,
                character: col2,
            },
            Position {
                line: line2,
                character: col2,
            },
        );
        let cross = open_ref_action(ws.as_ref(), &doc, range2).expect("cross-file action");
        let args2 = cross.command.as_ref().unwrap().arguments.as_ref().unwrap();
        assert!(
            args2[0].as_str().unwrap().ends_with("schemas.yaml"),
            "target URI points at the other file: {}",
            args2[0]
        );

        // Off-ref positions yield nothing.
        let (l3, c3) = {
            let inner = low.inner();
            inner
                .line_index()
                .line_col_utf16(inner.bytes(), text.find("title").unwrap())
        };
        let range3 = Range::new(
            Position {
                line: l3,
                character: c3,
            },
            Position {
                line: l3,
                character: c3,
            },
        );
        assert!(open_ref_action(ws.as_ref(), &doc, range3).is_none());
    }

    #[test]
    fn open_ref_action_survives_unresolvable_refs() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-openref-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let text = "components:\n  schemas:\n    A:\n      $ref: '#/components/schemas/Missing'\n";
        let low = low_at(&dir, "broken-ref.yaml", text);
        let doc = crate::state::OpenDoc::parse(low.uri().clone(), text.to_owned());
        let (line, col) = {
            let inner = low.inner();
            inner
                .line_index()
                .line_col_utf16(inner.bytes(), text.find('#').unwrap())
        };
        let range = Range::new(
            Position {
                line,
                character: col,
            },
            Position {
                line,
                character: col,
            },
        );
        assert!(open_ref_action(ws.as_ref(), &doc, range).is_none());
    }

    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        LowDoc::parse(
            Uri::from_path(&p).unwrap(),
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn offset_of(text: &str, needle: &str) -> usize {
        text.find(needle).expect("needle present")
    }

    /// The lens whose `data.ptr` equals `ptr`.
    fn lens_for<'a>(lenses: &'a [CodeLens], ptr: &str) -> &'a CodeLens {
        lenses
            .iter()
            .find(|l| lens_data_str(&l.data, "ptr") == Some(ptr))
            .unwrap_or_else(|| panic!("no lens for {ptr}"))
    }

    #[test]
    fn code_lens_covers_components_and_operations() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-lenses");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let lenses = code_lens(&ws, &low);

        let mut ptrs: Vec<&str> = lenses
            .iter()
            .filter_map(|l| lens_data_str(&l.data, "ptr"))
            .collect();
        ptrs.sort_unstable();
        assert_eq!(
            ptrs,
            vec![
                "/components/schemas/External",
                "/components/schemas/Pet",
                "/paths/~1pets/get",
                "/paths/~1pets/post",
            ]
        );

        // Lens sits above the key it decorates: the `get:` method key line.
        let get_line = MAIN.lines().position(|l| l.trim() == "get:").unwrap() as u32;
        let get_lens = lens_for(&lenses, "/paths/~1pets/get");
        assert_eq!(get_lens.range.start.line, get_line);
        // Unresolved lenses carry no command.
        assert!(get_lens.command.is_none());
    }

    #[test]
    fn code_lens_empty_and_refless_docs() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-lens-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let empty = low_at(&dir, "empty.yaml", "");
        assert!(code_lens(&ws, &empty).is_empty());

        let bare = "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\n";
        let bare_doc = low_at(&dir, "bare.yaml", bare);
        assert!(code_lens(&ws, &bare_doc).is_empty());
    }

    #[test]
    fn resolve_component_lens_counts_references() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-resolve-comp");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let lenses = code_lens(&ws, &low);

        // One direct edge (main.yaml's own local $ref) targets Pet.
        let pet = code_lens_resolve(
            &ws,
            &low,
            lens_for(&lenses, "/components/schemas/Pet").clone(),
        );
        let cmd = pet.command.expect("resolved command");
        assert_eq!(cmd.title, "1 references");
        assert_eq!(cmd.command, SHOW_REFS_COMMAND);
        let args = cmd.arguments.unwrap();
        assert_eq!(args[0], json!(low.uri().as_str()));
        assert_eq!(args[1], json!("/components/schemas/Pet"));
    }

    #[test]
    fn resolve_operation_lenses_reflect_operation_id() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-resolve-ops");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let lenses = code_lens(&ws, &low);

        let get = code_lens_resolve(&ws, &low, lens_for(&lenses, "/paths/~1pets/get").clone());
        let cmd = get.command.expect("resolved command");
        assert_eq!(cmd.title, "listPets");
        assert_eq!(cmd.command, SHOW_REFS_COMMAND);

        let post = code_lens_resolve(&ws, &low, lens_for(&lenses, "/paths/~1pets/post").clone());
        let cmd = post.command.expect("resolved command");
        assert_eq!(cmd.title, "add operationId");
        assert_eq!(cmd.command, ADD_OPERATION_ID_COMMAND);
    }

    #[test]
    fn resolve_returns_lens_without_data_unchanged() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-resolve-stale");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let stale = CodeLens {
            range: Default::default(),
            command: None,
            data: None,
        };
        assert_eq!(code_lens_resolve(&ws, &low, stale.clone()), stale);
        let junk = CodeLens {
            range: Default::default(),
            command: None,
            data: Some(json!({ "ptr": "#not a pointer" })),
        };
        assert_eq!(code_lens_resolve(&ws, &low, junk.clone()), junk);
    }

    #[test]
    fn document_links_local_external_and_skip_plain_names() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-doclink");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let links = document_link(&ws, &low);
        assert_eq!(links.len(), 2, "local Pet ref + external schemas.yaml ref");

        let own_uri = Url::parse(low.uri().as_str()).unwrap();
        let mut schemas_uri = own_uri.clone();
        schemas_uri.set_path(&dir.join("schemas.yaml").to_string_lossy());
        let mut local_target = own_uri.clone();
        local_target.set_fragment(Some("/components/schemas/Pet"));

        let local = links
            .iter()
            .find(|l| l.target.as_ref() == Some(&local_target))
            .expect("local link");
        assert_eq!(local.tooltip.as_deref(), Some("#/components/schemas/Pet"));

        let mut expected_external = schemas_uri.clone();
        expected_external.set_fragment(Some("/components/schemas/PetList"));
        let external = links
            .iter()
            .find(|l| l.target.as_ref() == Some(&expected_external))
            .expect("external link");
        assert_eq!(
            external.tooltip.as_deref(),
            Some("#/components/schemas/PetList")
        );
    }

    #[test]
    fn document_link_skips_non_refs_and_handles_empty_doc() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-doclink-edge");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);

        let plain = "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\n";
        assert!(document_link(&ws, &low_at(&dir, "plain.yaml", plain)).is_empty());
        assert!(document_link(&ws, &low_at(&dir, "empty.yaml", "")).is_empty());

        let named = "components:\n  schemas:\n    A:\n      $ref: '#Pet'\n";
        assert!(document_link(&ws, &low_at(&dir, "named.yaml", named)).is_empty());
    }

    #[test]
    fn document_link_resolve_summarizes_target_kind() {
        let mk = |ptr: &str| DocumentLink {
            range: Default::default(),
            target: None,
            tooltip: Some("#/somewhere".to_owned()),
            data: Some(json!({ "ptr": ptr })),
        };
        assert_eq!(
            document_link_resolve(mk("/components/schemas/Pet")).tooltip,
            Some("schema 'Pet'".to_owned())
        );
        assert_eq!(
            document_link_resolve(mk("/components/parameters/Limit")).tooltip,
            Some("parameter 'Limit'".to_owned())
        );
        assert_eq!(
            document_link_resolve(mk("/components/responses/NotFound")).tooltip,
            Some("response 'NotFound'".to_owned())
        );
        assert_eq!(
            document_link_resolve(mk("/paths/~1pets/get")).tooltip,
            Some("path item '/pets'".to_owned())
        );

        // No usable data: returned unchanged.
        let bare = DocumentLink {
            range: Default::default(),
            target: None,
            tooltip: None,
            data: None,
        };
        assert_eq!(document_link_resolve(bare.clone()), bare);
    }

    #[test]
    fn moniker_exports_component_definitions() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-moniker-export");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);

        // On the component *key*.
        let at_key = moniker(&ws, &low, offset_of(MAIN, "\n    Pet:") + 6).unwrap();
        assert_eq!(at_key.len(), 1);
        assert_eq!(at_key[0].scheme, "suspect");
        assert_eq!(at_key[0].kind, Some(MonikerKind::Export));
        assert_eq!(
            at_key[0].identifier,
            "urn:suspect:openapi:main.yaml:components/schemas/Pet"
        );

        // Deep inside the component body too (`type: object` value).
        let in_body = moniker(&ws, &low, offset_of(MAIN, "type: object") + 2).unwrap();
        assert_eq!(
            in_body[0].identifier,
            "urn:suspect:openapi:main.yaml:components/schemas/Pet"
        );
    }

    #[test]
    fn moniker_imports_cross_file_component_refs() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-moniker-import");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);

        let needle = "schemas.yaml#/components/schemas/PetList";
        let off = offset_of(MAIN, needle) + 4;
        let mks = moniker(&ws, &low, off).unwrap();
        assert_eq!(mks.len(), 1);
        assert_eq!(mks[0].kind, Some(MonikerKind::Import));
        assert_eq!(
            mks[0].identifier,
            "urn:suspect:openapi:schemas.yaml:components/schemas/PetList"
        );
    }

    #[test]
    fn moniker_identifiers_are_stable_across_files() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-moniker-stable");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);

        let main_low = low_at(&dir, "main.yaml", MAIN);
        let schemas_low = low_at(&dir, "schemas.yaml", SCHEMAS);

        let import = moniker(
            &ws,
            &main_low,
            offset_of(MAIN, "schemas.yaml#/components/schemas/PetList") + 8,
        )
        .unwrap()
        .remove(0);
        let export = moniker(&ws, &schemas_low, offset_of(SCHEMAS, "\n    PetList:") + 6)
            .unwrap()
            .remove(0);

        assert_eq!(import.identifier, export.identifier);
        assert_eq!(
            export.identifier,
            "urn:suspect:openapi:schemas.yaml:components/schemas/PetList"
        );
    }

    #[test]
    fn moniker_none_off_component_elements() {
        let dir = std::env::temp_dir().join("suspect-lsp-links-moniker-none");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);

        // On the info title value: not a component, not a $ref.
        let title_off = offset_of(MAIN, "title: T") + 9;
        assert!(moniker(&ws, &low, title_off).is_none());
        // On a same-document (non-import) $ref value.
        let local_ref_off = offset_of(MAIN, "'#/components/schemas/Pet'") + 3;
        assert!(moniker(&ws, &low, local_ref_off).is_none());
        // Empty document.
        let empty = low_at(&dir, "empty.yaml", "");
        assert!(moniker(&ws, &empty, 0).is_none());
        // Offset past everything.
        assert!(moniker(&ws, &low, MAIN.len() + 10).is_none());
    }
}
