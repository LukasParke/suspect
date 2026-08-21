//! `$ref` edge extraction: scanning a parsed document for mapping entries
//! keyed `"$ref"` and indexing plain-name anchor targets.
//!
//! The scan walks the *semantic* tree (`NodeRef::entries`/`items`, which are
//! alias-transparent and expand YAML merge keys) with an explicit stack, so
//! arbitrarily deep nesting cannot overflow the native stack. Containers whose
//! byte range is already on the walk path are skipped, which makes alias
//! cycles (`A: &x {b: *x}`) terminate.
//!
//! Limitations (v1):
//! - `$ref` values that fail to parse as URI references (unjoinable relative
//!   parts, invalid percent-escapes) are not recorded as edges; resolving
//!   such a node surfaces [`RefError::InvalidRef`] at resolution time.
//! - Plain-name fragments in *external* refs (`other.yaml#Pet`) parse to
//!   [`ParsedRef::PlainName`] but resolve against the referencing document's
//!   anchors; cross-file plain-name lookup is out of scope for v1.
//! - `$id` base-URI inheritance applies to the ancestor chain of each edge's
//!   containing mapping only (see `Workspace::effective_parsed`).

use std::collections::HashMap;
use std::ops::Range;

use percent_encoding::percent_decode_str;
use suspect_low::{LowDoc, NodeRef, Pointer, ValueKind};
use suspect_source::Uri;

use crate::error::RefError;

/// One discovered `$ref` occurrence inside a document.
#[derive(Debug, Clone)]
pub struct RefEdge {
    /// Byte range of the `$ref` **value** node (the string).
    pub at: Range<usize>,
    /// The unescaped scalar text of the `$ref` value.
    pub raw: Box<str>,
    /// The parsed reference (local / external / plain name).
    pub parsed: ParsedRef,
    /// RFC 6901 pointer to the containing mapping — the object that carries
    /// the `$ref` key, not the value itself.
    pub path: Pointer,
}

/// A `$ref` value split into its addressable parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedRef {
    /// Same-document pointer (fragment only, e.g. `#/components/Schemas/Pet`).
    Local(Pointer),
    /// Another document plus an in-document pointer. A root pointer means
    /// the whole document.
    External {
        /// Canonical fragment-free target document URI.
        uri: Uri,
        /// Pointer into the target document (root for whole-doc refs).
        pointer: Pointer,
    },
    /// Plain-name fragment (`#Pet`, `$anchor`) — resolved through the
    /// per-document anchors index.
    PlainName(Box<str>),
}

/// Per-edge auxiliary ranges kept out of the public edge list.
#[derive(Debug, Default)]
pub(crate) struct EdgeMeta {
    /// Byte range of each edge's containing mapping, parallel to the edges.
    pub(crate) mapping_ranges: Vec<Range<usize>>,
    /// `$ref`-value byte range → edge index.
    pub(crate) value_index: HashMap<Range<usize>, usize>,
}

/// Everything one scan of a document produces.
#[derive(Debug, Default)]
pub(crate) struct Scanned {
    pub(crate) edges: Vec<RefEdge>,
    pub(crate) meta: EdgeMeta,
    /// Plain-name targets: `$anchor: name` and Swagger 2.0-style
    /// `id: "#name"` fields, mapped to their containing mapping's pointer.
    pub(crate) anchors: HashMap<String, Pointer>,
    /// `$id` values (URI references) indexed by the pointer of the mapping
    /// that declares them; drives base-URI inheritance during resolution.
    pub(crate) ids: HashMap<Pointer, String>,
}

/// Parses a raw `$ref` string against a base document URI.
///
/// Percent-decoding happens before [`Pointer::parse`], so `%7B` becomes `{`
/// first and `~1`/`~0` unescaping happens inside the pointer parser — the
/// RFC 7644 / OAS layering for refs like `#/paths/~1pets~1%7Bid%7D/get`.
pub(crate) fn parse_ref(base: &Uri, raw: &str) -> Result<ParsedRef, RefError> {
    let invalid = |reason: String| RefError::InvalidRef {
        raw: raw.to_owned(),
        reason,
    };
    let (doc_part, frag) = Uri::split_ref(raw);
    match doc_part {
        None => fragment_only(frag),
        Some(doc) => {
            let uri = base
                .join(doc)
                .map_err(|e| invalid(format!("cannot join `{doc}` against the base URI: {e}")))?;
            with_fragment(uri, frag)
        }
    }
}

fn fragment_only(frag: &str) -> Result<ParsedRef, RefError> {
    if frag.is_empty() {
        return Ok(ParsedRef::Local(Pointer::root()));
    }
    if frag.starts_with('/') {
        return Ok(ParsedRef::Local(decode_pointer(frag)?));
    }
    Ok(ParsedRef::PlainName(decode_name(frag)?))
}

fn with_fragment(uri: Uri, frag: &str) -> Result<ParsedRef, RefError> {
    if frag.is_empty() {
        return Ok(ParsedRef::External { uri, pointer: Pointer::root() });
    }
    if frag.starts_with('/') {
        let pointer = decode_pointer(frag)?;
        return Ok(ParsedRef::External { uri, pointer });
    }
    Ok(ParsedRef::PlainName(decode_name(frag)?))
}

/// Percent-decodes a fragment body to UTF-8 (`%XX` sequences; invalid
/// escapes pass through verbatim per RFC 3986 leniency).
fn decode_utf8(frag: &str) -> Result<String, RefError> {
    percent_decode_str(frag)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|_| RefError::InvalidRef {
            raw: frag.to_owned(),
            reason: "percent-decoded fragment is not valid UTF-8".to_owned(),
        })
}

/// Percent-decodes a fragment body, then parses it as an RFC 6901 pointer
/// (which applies `~0`/`~1` unescaping on top).
fn decode_pointer(frag: &str) -> Result<Pointer, RefError> {
    let s = decode_utf8(frag)?;
    Pointer::parse(&s).map_err(|e| RefError::InvalidRef {
        raw: frag.to_owned(),
        reason: e.to_string(),
    })
}

/// Percent-decodes a plain-name fragment.
fn decode_name(frag: &str) -> Result<Box<str>, RefError> {
    Ok(decode_utf8(frag)?.into_boxed_str())
}

struct Frame<'d> {
    ptr: Pointer,
    /// Byte range of this frame's container node.
    range: Range<usize>,
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

/// Scans a document for `$ref` edges, plain-name anchors, and `$id` bases.
///
/// Single iterative pass over the semantic tree; aliases and merge keys are
/// expanded by the semantic layer, and duplicate expansions of the same
/// physical `$ref` node are collapsed via its byte range.
pub(crate) fn scan(doc: &LowDoc) -> Scanned {
    let mut sc = Scanned::default();
    let root = doc.root();
    let mut stack: Vec<Frame<'_>> = vec![Frame {
        ptr: Pointer::root(),
        range: root.byte_range(),
        children: children_of(root),
        next: 0,
    }];
    // Byte ranges of containers on the current path; guards against infinite
    // expansion through self-referential YAML aliases.
    let mut on_path: Vec<Range<usize>> = vec![root.byte_range()];


    while let Some(frame) = stack.last_mut() {
        let Some((key, child_node, index)) = frame.children.get(frame.next).cloned() else {
            stack.pop();
            on_path.pop();
            continue;
        };
        frame.next += 1;
        let child_ptr = match key.as_deref() {
            Some(k) => frame.ptr.push(k), // keys from entries() are unescaped
            None => frame.ptr.push(&index.to_string()),
        };

        if child_node.kind() == ValueKind::Str {
            if key.as_deref() == Some("$ref") {
                record_ref(&mut sc, doc, child_node, frame.range.clone(), &frame.ptr);
            } else if let (Some(name), true) =
                (child_node.as_str(), matches!(key.as_deref(), Some("$anchor")))
            {
                sc.anchors.insert(name.to_owned(), frame.ptr.clone());
            } else if let Some(v) = child_node.as_str()
                && matches!(key.as_deref(), Some("id" | "$id"))
            {
                // Swagger 2.0-style JSON `id` / 3.1 `$id` written as a
                // plain-name fragment target (`#Pet`).
                if let Some(name) = v.strip_prefix('#') {
                    if !name.is_empty() && !name.starts_with('/') {
                        sc.anchors.insert(name.to_owned(), frame.ptr.clone());
                    }
                } else if key.as_deref() == Some("$id") {
                    sc.ids.insert(frame.ptr.clone(), v.to_owned());
                }
            }
        }

        // Descend only into containers not already on the walk path
        // (alias-cycle guard).
        match child_node.kind() {
            ValueKind::Object | ValueKind::Array => {
                let range = child_node.byte_range();
                if on_path.iter().any(|r| r.start == range.start && r.end == range.end) {
                    continue;
                }
                on_path.push(range.clone());
                stack.push(Frame {
                    ptr: child_ptr,
                    range,
                    children: children_of(child_node),
                    next: 0,
                });
            }
            _ => {}
        }
    }
    sc
}

fn record_ref(
    sc: &mut Scanned,
    doc: &LowDoc,
    value: NodeRef<'_>,
    mapping_range: Range<usize>,
    mapping_ptr: &Pointer,
) {
    let range = value.byte_range();
    let Some(raw) = value.as_str() else { return };
    // Collapse duplicate expansions of the same physical node (aliases).
    if sc.meta.value_index.contains_key(&range) {
        return;
    }
    let Ok(parsed) = parse_ref(doc.uri(), raw) else { return };
    sc.meta.value_index.insert(range.clone(), sc.edges.len());
    sc.meta.mapping_ranges.push(mapping_range);
    sc.edges.push(RefEdge {
        at: range,
        raw: Box::from(raw),
        parsed,
        path: mapping_ptr.clone(),
    });
}

