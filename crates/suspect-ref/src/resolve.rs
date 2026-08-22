//! Resolution chains: memoized pointer evaluation and `$ref` chain
//! following.
//!
//! A *chain* is the sequence of locations visited while chasing `$ref`
//! objects: resolving a pointer that lands on an object carrying `$ref`
//! continues at the referenced location until a plain value, a whole
//! document, or a revisit (cycle) is reached. Chains are followed
//! iteratively with a step list (no recursion); depth is capped by the
//! workspace's `depth_cap` (default 256).
//!
//! Results are memoized per `(DocId, pointer)` in serialized `/a/b` form,
//! which is injective thanks to RFC 6901 escaping. Memoized values carry no
//! borrows (`DocId` + owned [`Pointer`] / steps), so they live happily in a
//! `DashMap` on a lifetime-free [`Workspace`].

use std::ops::Range;

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::edges::{ParsedRef, parse_ref};
use crate::error::RefError;
use crate::workspace::Workspace;

/// One visited location in a resolution chain or cycle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Step {
    /// Document in which this step's node lives.
    pub doc: crate::DocId,
    /// Byte range of the node this step landed on.
    pub at: Range<usize>,
}

impl Ord for Step {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.doc
            .cmp(&other.doc)
            .then(self.at.start.cmp(&other.at.start))
            .then(self.at.end.cmp(&other.at.end))
    }
}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The outcome of resolving a reference.
#[derive(Clone)]
pub enum Resolution<'ws> {
    /// A concrete node inside some loaded document.
    Node(NodeRef<'ws>),
    /// The target was a document root (empty fragment).
    WholeDoc(crate::DocId),
    /// Following the chain revisited a location; `path` lists every step of
    /// the loop in visit order.
    Cycle {
        /// The loop's steps.
        path: Box<[Step]>,
    },
}

impl<'ws> std::fmt::Debug for Resolution<'ws> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resolution::Node(n) => f
                .debug_struct("Node")
                .field("kind", &n.kind())
                .field("range", &n.byte_range())
                .finish(),
            Resolution::WholeDoc(d) => f.debug_tuple("WholeDoc").field(d).finish(),
            Resolution::Cycle { path } => f.debug_struct("Cycle").field("path", path).finish(),
        }
    }
}

/// Lifetime-free memoized resolution outcome.
#[derive(Debug, Clone)]
pub(crate) enum MemoVal {
    /// Resolved to a node at this location.
    Loc {
        /// Owning document.
        doc: crate::DocId,
        /// Pointer to the resolved node.
        ptr: Pointer,
    },
    /// Resolved to a document root.
    Whole(crate::DocId),
    /// Chain revisited itself; steps recorded in visit order.
    Cycle(Box<[Step]>),
}

impl Workspace {
    /// Memoized pointer resolution against one loaded document.
    pub(crate) fn resolve_memo(
        &self,
        doc: crate::DocId,
        ptr: &Pointer,
    ) -> Result<MemoVal, RefError> {
        let key = (doc, ptr.to_path().into_boxed_str());
        if let Some(mv) = self.memos.get(&key) {
            self.memo_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(mv.clone());
        }
        self.memo_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mv = self.eval_chain(doc, ptr)?;
        self.memos.insert(key, mv.clone());
        Ok(mv)
    }

    /// Memoized resolution of an *edge*: the chain starts at the edge's
    /// containing mapping. The memo key folds in the raw value so duplicate
    /// `$ref` keys in one mapping cannot collide.
    pub(crate) fn resolve_edge_memo(
        &self,
        doc: crate::DocId,
        containing: &Pointer,
        raw: &str,
    ) -> Result<MemoVal, RefError> {
        let mut key_str = containing.to_path();
        key_str.push('\u{0}');
        key_str.push_str(raw);
        let key = (doc, key_str.into_boxed_str());
        if let Some(mv) = self.memos.get(&key) {
            self.memo_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(mv.clone());
        }
        self.memo_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mv = self.eval_edge(doc, containing, raw)?;
        self.memos.insert(key, mv.clone());
        Ok(mv)
    }

    /// Follows a chain of `$ref` objects starting at `(doc, ptr)`.
    ///
    /// A root pointer means the whole document. Iterative; records every
    /// landing as a [`Step`]. Revisiting any step yields [`MemoVal::Cycle`].
    /// Plain (non-`$ref`) landing spots are also memoized under their own
    /// keys as shortcuts.
    pub(crate) fn eval_chain(&self, doc: crate::DocId, ptr: &Pointer) -> Result<MemoVal, RefError> {
        if ptr.is_root() {
            return Ok(MemoVal::Whole(doc));
        }
        self.run_chain((doc, ptr.clone()), Vec::new())
    }

    /// Chain entry for an *edge*: starts at the edge's containing mapping.
    /// When that mapping is the document root (a top-level `$ref`), the root
    /// itself is seeded as the first step and the hop happens immediately —
    /// the root pointer must not short-circuit to `WholeDoc` here.
    pub(crate) fn eval_edge(
        &self,
        doc: crate::DocId,
        containing: &Pointer,
        raw: &str,
    ) -> Result<MemoVal, RefError> {
        if !containing.is_root() {
            return self.run_chain((doc, containing.clone()), Vec::new());
        }
        let d = self.extend_doc(doc).ok_or_else(|| RefError::MissingDoc {
            uri: format!("doc #{doc}"),
        })?;
        let seed = vec![Step {
            doc,
            at: d.root().byte_range(),
        }];
        let next = self.hop(doc, containing, raw)?;
        self.run_chain(next, seed)
    }

    fn run_chain(
        &self,
        start: (crate::DocId, Pointer),
        mut steps: Vec<Step>,
    ) -> Result<MemoVal, RefError> {
        let mut cur = start;
        loop {
            if steps.len() > self.depth_cap {
                return Err(RefError::TooDeep {
                    cap: self.depth_cap,
                });
            }
            let (d, p) = &cur;
            if p.is_root() {
                return Ok(MemoVal::Whole(*d));
            }
            let Some(doc_ref) = self.extend_doc(*d) else {
                return Err(RefError::MissingDoc {
                    uri: format!("doc #{d}"),
                });
            };
            let Some(node) = doc_ref.root().pointer(p) else {
                return Err(RefError::MissingPointer {
                    doc_uri: doc_ref.uri().to_string(),
                    pointer: p.to_path(),
                });
            };
            let step = Step {
                doc: *d,
                at: node.byte_range(),
            };
            if steps.contains(&step) {
                return Ok(MemoVal::Cycle(steps.into_boxed_slice()));
            }
            steps.push(step);

            match node.get("$ref") {
                Some(rv) if rv.kind() == ValueKind::Str => {
                    // Block scalars (Stripe style) must be decoded, not read
                    // as raw source slices.
                    let decoded = rv.decoded_scalar();
                    let raw = std::str::from_utf8(&decoded)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_owned();
                    cur = self.hop(*d, p, &raw)?;
                }
                _ => {
                    // Plain landing spot: memoize it as its own shortcut so
                    // later chains passing through here skip re-walking.
                    let key = (*d, p.to_path().into_boxed_str());
                    let mv = MemoVal::Loc {
                        doc: *d,
                        ptr: p.clone(),
                    };
                    self.memos.insert(key, mv.clone());
                    return Ok(mv);
                }
            }
        }
    }

    /// Computes the next location after one `$ref` hop from `(doc, ptr)`
    /// whose raw value is `raw`.
    pub(crate) fn hop(
        &self,
        doc: crate::DocId,
        containing: &Pointer,
        raw: &str,
    ) -> Result<(crate::DocId, Pointer), RefError> {
        let parsed = self.effective_parsed(doc, containing, raw)?;
        self.hop_parsed(doc, &parsed, raw)
    }

    /// Applies `$id` base-URI inheritance to a raw ref value: if any mapping
    /// on the ancestor chain (root → `containing`, inclusive) declares
    /// `$id`, those values join into a running base starting at the owning
    /// document's URI, and the ref value joins the final base.
    ///
    /// Without `$id` ancestors this returns the precomputed parse unchanged
    /// (zero extra allocation).
    pub(crate) fn effective_parsed(
        &self,
        doc: crate::DocId,
        containing: &Pointer,
        raw: &str,
    ) -> Result<ParsedRef, RefError> {
        let ids = self.ids_of(doc);
        let tokens = containing.tokens();
        let has_base = (0..=tokens.len())
            .any(|n| ids.contains_key(&Pointer::from_tokens(tokens[..n].to_vec())));
        if !has_base {
            // Fast path: no $id anywhere on the ancestor chain.
            let edges = self.edges_of(doc);
            if let Some(edge) = edges
                .iter()
                .find(|e| e.path == *containing && &*e.raw == raw)
            {
                return Ok(edge.parsed.clone());
            }
            // Fall through to a fresh parse when no edge matches (e.g.
            // resolve_ref_value on an unindexed node).
            let base = self.doc_uri(doc);
            return parse_ref(&base, raw);
        }
        let mut base = self.doc_uri(doc);
        for n in 0..=tokens.len() {
            let prefix = Pointer::from_tokens(tokens[..n].to_vec());
            if let Some(id) = ids.get(&prefix) {
                base = base.join(id).map_err(|e| RefError::InvalidRef {
                    raw: raw.to_owned(),
                    reason: format!("cannot join $id `{id}` against base `{base}`: {e}"),
                })?;
            }
        }
        // A fragment-only ref joined against a base that left the owning
        // document actually targets the base document.
        let parsed = parse_ref(&base, raw)?;
        match parsed {
            ParsedRef::Local(p) => {
                if base != self.doc_uri(doc) {
                    return Ok(ParsedRef::External {
                        uri: base,
                        pointer: p,
                    });
                }
                Ok(ParsedRef::Local(p))
            }
            other => Ok(other),
        }
    }

    /// Turns a parsed ref (relative to `doc`) into the next concrete
    /// location, loading external documents as needed.
    pub(crate) fn hop_parsed(
        &self,
        doc: crate::DocId,
        parsed: &ParsedRef,
        raw_for_errors: &str,
    ) -> Result<(crate::DocId, Pointer), RefError> {
        match parsed {
            ParsedRef::Local(p) => Ok((doc, p.clone())),
            ParsedRef::PlainName(name) => {
                let anchors = self.anchors_of(doc);
                match anchors.get(name.as_ref()) {
                    Some(ap) => Ok((doc, ap.clone())),
                    None => Err(RefError::InvalidRef {
                        raw: raw_for_errors.to_owned(),
                        reason: format!(
                            "plain-name fragment `#{name}` matches no `$anchor` or `id` in this document"
                        ),
                    }),
                }
            }
            ParsedRef::External { uri, pointer } => {
                let target = self.load_uri(uri)?;
                Ok((target, pointer.clone()))
            }
        }
    }

    /// Materializes a memoized outcome into a borrowed [`Resolution`].
    pub(crate) fn materialize<'ws>(&'ws self, mv: &MemoVal) -> Result<Resolution<'ws>, RefError> {
        match mv {
            MemoVal::Loc { doc, ptr } => {
                let d = self.extend_doc(*doc).ok_or_else(|| RefError::MissingDoc {
                    uri: format!("doc #{doc}"),
                })?;
                let node = d
                    .root()
                    .pointer(ptr)
                    .ok_or_else(|| RefError::MissingPointer {
                        doc_uri: d.uri().to_string(),
                        pointer: ptr.to_path(),
                    })?;
                Ok(Resolution::Node(node))
            }
            MemoVal::Whole(d) => Ok(Resolution::WholeDoc(*d)),
            MemoVal::Cycle(steps) => Ok(Resolution::Cycle {
                path: steps.clone(),
            }),
        }
    }

    fn doc_uri(&self, doc: crate::DocId) -> suspect_source::Uri {
        self.extend_doc(doc)
            .map(|d| d.uri().clone())
            .unwrap_or_else(|| suspect_source::Uri::from(String::from("file:///")))
    }
}
