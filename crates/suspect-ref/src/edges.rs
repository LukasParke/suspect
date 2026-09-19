//! `$ref` edge extraction: scanning a parsed document for mapping entries
//! keyed `"$ref"` and indexing plain-name anchor targets.
//!
//! The scan walks the *semantic* tree (`NodeRef::entries`/`items`, which are
//! alias-transparent and expand YAML merge keys) with an explicit stack, so
//! arbitrarily deep nesting cannot overflow the native stack. Containers whose
//! byte range is already on the walk path are skipped, which makes alias
//! cycles (`A: &x {b: *x}`) terminate.
//!
//! This is a generic reference scan, not an OpenAPI vocabulary walk: callers
//! interpreting arbitrary example/default data must scope results to their
//! schema or OpenAPI reference positions.
//!
//! Limitations (v1):
//! - `$id` base-URI inheritance applies to the ancestor chain of each edge's
//!   containing mapping only (see `Workspace::effective_parsed`).

use std::collections::{HashMap, HashSet};
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

/// A malformed `$ref` occurrence found by the generic document scan.
///
/// The owning URI is supplied by the [`crate::DocHandle`] that returns it.
/// This scan does not infer OpenAPI/schema semantics: a literal `$ref` key
/// inside example data is included, so semantic consumers must scope these
/// diagnostics to positions where their vocabulary defines references.
#[derive(Debug, Clone)]
pub struct RefDiagnostic {
    /// Value byte range, or key byte range when a YAML value is absent.
    pub at: Range<usize>,
    /// Pointer to the mapping containing the malformed `$ref`.
    pub path: Pointer,
    /// Decoded string, when the value was a valid string scalar.
    pub raw: Option<Box<str>>,
    /// Why the occurrence could not be indexed as a reference edge.
    pub reason: String,
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
    /// A plain-name fragment anchored in another document.
    ExternalAnchor {
        /// Canonical fragment-free target document URI.
        uri: Uri,
        /// Decoded plain-name fragment.
        name: Box<str>,
    },
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
    pub(crate) diagnostics: Vec<RefDiagnostic>,
    seen_refs: HashSet<Range<usize>>,
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
/// RFC 6901 / OAS layering for refs like `#/paths/~1pets~1%7Bid%7D/get`.
///
/// # Errors
/// Invalid percent escapes, invalid UTF-8 fragments, malformed JSON
/// pointers, or a document URI that cannot join the base.
pub fn parse_ref(base: &Uri, raw: &str) -> Result<ParsedRef, RefError> {
    let invalid = |reason: String| RefError::InvalidRef {
        raw: raw.to_owned(),
        reason,
    };
    validate_percent_escapes(raw)?;
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
    let decoded = decode_utf8(frag)?;
    let frag = decoded.as_str();
    if frag.is_empty() {
        return Ok(ParsedRef::Local(Pointer::root()));
    }
    if frag.starts_with('/') {
        return Ok(ParsedRef::Local(parse_pointer(frag)?));
    }
    Ok(ParsedRef::PlainName(frag.into()))
}

fn with_fragment(uri: Uri, frag: &str) -> Result<ParsedRef, RefError> {
    let decoded = decode_utf8(frag)?;
    let frag = decoded.as_str();
    if frag.is_empty() {
        return Ok(ParsedRef::External {
            uri,
            pointer: Pointer::root(),
        });
    }
    if frag.starts_with('/') {
        let pointer = parse_pointer(frag)?;
        return Ok(ParsedRef::External { uri, pointer });
    }
    Ok(ParsedRef::ExternalAnchor {
        uri,
        name: frag.into(),
    })
}

fn validate_percent_escapes(raw: &str) -> Result<(), RefError> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if !bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                return Err(RefError::InvalidRef {
                    raw: raw.to_owned(),
                    reason: format!("invalid percent escape at byte {i}"),
                });
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Percent-decodes a validated fragment body to UTF-8.
fn decode_utf8(frag: &str) -> Result<String, RefError> {
    percent_decode_str(frag)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|_| RefError::InvalidRef {
            raw: frag.to_owned(),
            reason: "percent-decoded fragment is not valid UTF-8".to_owned(),
        })
}

/// Parses a decoded fragment as an RFC 6901 pointer (`~0`/`~1` unescaping).
fn parse_pointer(frag: &str) -> Result<Pointer, RefError> {
    Pointer::parse(frag).map_err(|e| RefError::InvalidRef {
        raw: frag.to_owned(),
        reason: e.to_string(),
    })
}

pub(crate) fn ref_text(node: NodeRef<'_>) -> Result<String, RefError> {
    let invalid = |reason: &str| RefError::InvalidRef {
        raw: String::from_utf8_lossy(node.scalar_bytes()).into_owned(),
        reason: reason.to_owned(),
    };
    if node.kind() != ValueKind::Str {
        return Err(invalid("node is not a $ref string value"));
    }
    let decoded = node
        .try_decoded_scalar()
        .ok_or_else(|| invalid("invalid string escapes"))?;
    std::str::from_utf8(&decoded)
        .map(|s| s.trim().to_owned())
        .map_err(|_| invalid("reference is not valid UTF-8"))
}

struct Frame<'d> {
    ptr: Pointer,
    /// Byte range of this frame's container node.
    range: Range<usize>,
    children: Vec<(Option<Box<str>>, NodeRef<'d>, usize, bool)>,
    next: usize,
}

fn children_of<'d>(node: NodeRef<'d>) -> Vec<(Option<Box<str>>, NodeRef<'d>, usize, bool)> {
    match node.kind() {
        ValueKind::Object => node
            .entries()
            .into_iter()
            .filter_map(|e| {
                let decoded = e.key_node.try_decoded_scalar()?;
                let key = std::str::from_utf8(&decoded).ok()?;
                Some((
                    Some(Box::from(key)),
                    e.value.unwrap_or(e.key_node),
                    0usize,
                    e.value.is_none(),
                ))
            })
            .collect(),
        ValueKind::Array => node
            .items()
            .into_iter()
            .enumerate()
            .map(|(i, v)| (None, v, i, false))
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
        let Some((key, child_node, index, missing)) = frame.children.get(frame.next).cloned()
        else {
            stack.pop();
            on_path.pop();
            continue;
        };
        frame.next += 1;
        let child_ptr = match key.as_deref() {
            Some(k) => frame.ptr.push(k), // keys from entries() are unescaped
            None => frame.ptr.push(&index.to_string()),
        };

        if key.as_deref() == Some("$ref") {
            record_ref(
                &mut sc,
                doc,
                child_node,
                frame.range.clone(),
                &frame.ptr,
                missing,
            );
        } else if !missing && child_node.kind() == ValueKind::Str {
            if let (Ok(name), true) = (
                ref_text(child_node),
                matches!(key.as_deref(), Some("$anchor")),
            ) {
                sc.anchors.insert(name, frame.ptr.clone());
            } else if let Ok(v) = ref_text(child_node)
                && matches!(key.as_deref(), Some("id" | "$id"))
            {
                // Swagger 2.0-style JSON `id` / 3.1 `$id` written as a
                // plain-name fragment target (`#Pet`).
                if let Some(name) = v.strip_prefix('#') {
                    if !name.is_empty() && !name.starts_with('/') {
                        sc.anchors.insert(name.to_owned(), frame.ptr.clone());
                    }
                } else if key.as_deref() == Some("$id") {
                    sc.ids.insert(frame.ptr.clone(), v);
                }
            }
        }

        // Descend only into containers not already on the walk path
        // (alias-cycle guard).
        match child_node.kind() {
            ValueKind::Object | ValueKind::Array => {
                let range = child_node.byte_range();
                if on_path
                    .iter()
                    .any(|r| r.start == range.start && r.end == range.end)
                {
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
    missing: bool,
) {
    let range = value.byte_range();
    if !sc.seen_refs.insert(range.clone()) {
        return;
    }
    // Stripe and friends write refs as folded block scalars (`>-`); the
    // decoded value is the pointer text, not the raw source slice.
    let decoded = if missing {
        Err(RefError::InvalidRef {
            raw: String::new(),
            reason: "$ref value is absent (null)".to_owned(),
        })
    } else {
        ref_text(value)
    };
    let (raw, parsed) = match decoded {
        Ok(raw) => match parse_ref(doc.uri(), &raw) {
            Ok(parsed) => (raw, parsed),
            Err(error) => {
                sc.diagnostics.push(RefDiagnostic {
                    at: range,
                    path: mapping_ptr.clone(),
                    raw: Some(raw.into_boxed_str()),
                    reason: error.to_string(),
                });
                return;
            }
        },
        Err(error) => {
            sc.diagnostics.push(RefDiagnostic {
                at: range,
                path: mapping_ptr.clone(),
                raw: None,
                reason: error.to_string(),
            });
            return;
        }
    };
    sc.meta.value_index.insert(range.clone(), sc.edges.len());
    sc.meta.mapping_ranges.push(mapping_range);
    sc.edges.push(RefEdge {
        at: range,
        raw: raw.into_boxed_str(),
        parsed,
        path: mapping_ptr.clone(),
    });
}
