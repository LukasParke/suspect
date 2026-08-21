use suspect_syntax::{Format, SyntaxKind};

use crate::scalar::{infer_scalar, parse_float, parse_int, ValueKind};
use crate::Pointer;

/// A semantic view over a CST node: typed scalar access, alias-transparent
/// navigation, and pointer evaluation. Copyable; borrows its [`LowDoc`](crate::LowDoc).
#[derive(Clone, Copy)]
pub struct NodeRef<'d> {
    pub(crate) raw: suspect_syntax::SNode<'d>,
}

/// One mapping entry with the key's unescaped scalar text.
#[derive(Clone, Copy)]
pub struct Entry<'d> {
    /// The mapping key with quotes stripped and no escape processing
    /// (what `scalar_bytes` yields for the key node).
    pub key: &'d str,
    /// The entry's value, or `None` for an empty value (`key:` with
    /// nothing after it).
    pub value: Option<NodeRef<'d>>,
}

impl<'d> std::fmt::Debug for Entry<'d> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("key", &self.key).finish_non_exhaustive()
    }
}

/// A duplicated mapping key with all of its occurrence ranges.
#[derive(Debug, Clone)]
pub struct DuplicateKey {
    /// The key text exactly as it appears (unescaped, unquoted).
    pub key: String,
    /// Byte range of each value node under the same key, in document order.
    pub occurrences: Vec<std::ops::Range<usize>>,
}

impl std::fmt::Debug for NodeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRef")
            .field("kind", &self.kind())
            .field("range", &self.byte_range())
            .finish()
    }
}

impl<'d> NodeRef<'d> {
    /// Wraps a syntax node into a semantic view.
    #[must_use]
    pub fn new(raw: suspect_syntax::SNode<'d>) -> Self {
        Self { raw }
    }

    /// The underlying syntax node (escape hatch for source maps).
    #[must_use]
    pub fn syntax(&self) -> &suspect_syntax::SNode<'d> {
        &self.raw
    }

    /// Serialization format of the underlying document.
    #[must_use]
    pub fn format(&self) -> Format {
        self.raw.doc().format()
    }

    /// Semantic kind after alias resolution and YAML 1.2 core inference.
    #[must_use]
    pub fn kind(&self) -> ValueKind {
        let node = self.resolved();
        match node.raw.kind() {
            SyntaxKind::Mapping => ValueKind::Object,
            SyntaxKind::Sequence => ValueKind::Array,
            _ => infer_scalar(node.raw.scalar_bytes(), node.raw.scalar_style(), node.format()),
        }
    }

    /// True when this node is a YAML alias (`*name`).
    #[must_use]
    pub fn is_alias(&self) -> bool {
        self.raw.kind() == SyntaxKind::Alias
    }

    /// Follows YAML aliases to the anchor target and descends through
    /// stream/document/wrapper nodes to the semantic value.
    ///
    /// Cycles (an anchor resolving to itself) are cut off after 64 hops;
    /// callers then see the [`SyntaxKind::Alias`] node itself rather than
    /// hanging. Non-alias nodes return their content view unchanged.
    pub fn resolved(&self) -> NodeRef<'d> {
        let mut node = *self;
        for _ in 0..64 {
            if node.raw.kind() != SyntaxKind::Alias {
                // descend through stream/document/wrapper nodes to the
                // semantic value
                return NodeRef::new(node.raw.content());
            }
            match node
                .raw
                .alias_name()
                .and_then(|name| node.raw.doc().anchor_target(name))
            {
                Some(target) => node = NodeRef::new(target),
                None => return NodeRef::new(node.raw.content()),
            }
        }
        // anchor cycle: callers see Alias kind
        node
    }

    /// Mapping lookup by unescaped key text. Merge keys (`<<`) participate:
    /// explicit keys win over merged ones.
    ///
    /// Allocation-free fast path: scans pairs directly instead of
    /// materializing the full entry list.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<NodeRef<'d>> {
        let node = self.resolved();
        if node.kind() != ValueKind::Object {
            return None;
        }
        let mut merges: smallvec::SmallVec<[NodeRef<'d>; 4]> = smallvec::SmallVec::new();
        for (k, v) in node.raw.mapping_entries() {
            let kb = k.scalar_bytes();
            if kb == key.as_bytes() {
                return v.map(NodeRef::new);
            }
            if kb == b"<<"
                && let Some(v) = v {
                    merges.push(NodeRef::new(v));
                }
        }
        merges.into_iter().find_map(|m| find_in_merge(m, key))
    }

    /// Sequence item by index.
    #[must_use]
    pub fn at(&self, index: usize) -> Option<NodeRef<'d>> {
        self.items().into_iter().nth(index)
    }

    /// Mapping entries in document order, aliases and merge keys expanded.
    ///
    /// Merge semantics: `<<` values (an alias to a mapping, a sequence of
    /// such, or an inline mapping) contribute their entries *after* the
    /// explicit ones, skipping keys already present.
    #[must_use]
    pub fn entries(&self) -> Vec<Entry<'d>> {
        let node = self.resolved();
        if node.kind() != ValueKind::Object {
            return Vec::new();
        }
        let mut out: Vec<Entry<'d>> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut merges: Vec<NodeRef<'d>> = Vec::new();
        for (key_node, value) in node.raw.mapping_entries() {
            let key_bytes = key_node.scalar_bytes();
            let Ok(key) = std::str::from_utf8(key_bytes) else { continue };
            if key == "<<" {
                if let Some(v) = value {
                    merges.push(NodeRef::new(v));
                }
                continue;
            }
            seen.push(key);
            out.push(Entry { key, value: value.map(NodeRef::new) });
        }
        for merge in merges {
            append_merge(&mut out, &mut seen, merge);
        }
        out
    }

    /// Sequence items in order.
    #[must_use]
    pub fn items(&self) -> Vec<NodeRef<'d>> {
        self.resolved()
            .raw
            .sequence_items()
            .into_iter()
            .map(NodeRef::new)
            .collect()
    }

    /// Evaluates an RFC 6901 pointer against this subtree.
    #[must_use]
    pub fn pointer(&self, pointer: &Pointer) -> Option<NodeRef<'d>> {
        let mut node = *self;
        for token in pointer.tokens() {
            node = match node.kind() {
                ValueKind::Object => node.get(token)?,
                ValueKind::Array => token.parse::<usize>().ok().and_then(|i| node.at(i))?,
                _ => return None,
            };
        }
        Some(node)
    }

    /// Unquoted scalar bytes (no escape processing).
    #[must_use]
    pub fn scalar_bytes(&self) -> &'d [u8] {
        self.resolved().raw.scalar_bytes()
    }

    /// Scalar as `str`, if valid UTF-8.
    #[must_use]
    pub fn as_str(&self) -> Option<&'d str> {
        std::str::from_utf8(self.scalar_bytes()).ok()
    }

    /// Scalar as a boolean, if the inferred kind is [`ValueKind::Bool`].
    /// Accepts the YAML 1.2 spellings `true`/`True`/`TRUE` (and their
    /// false counterparts); anything else under a bool kind is `false`.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self.kind() {
            ValueKind::Bool => match self.scalar_bytes() {
                b"true" | b"True" | b"TRUE" => Some(true),
                _ => Some(false),
            },
            _ => None,
        }
    }

    /// Scalar as a signed 64-bit integer, if the inferred kind is
    /// [`ValueKind::Int`]. Understands decimal, `0o` octal, and `0x`
    /// hexadecimal forms (YAML) and plain decimal (JSON).
    ///
    /// Returns `None` when the value does not fit in `i64`.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self.kind() {
            ValueKind::Int => parse_int(self.scalar_bytes(), self.format()),
            _ => None,
        }
    }

    /// Scalar as an unsigned 64-bit integer: like [`Self::as_i64`] but
    /// `None` for negative values or values above `u64::MAX`.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().and_then(|v| u64::try_from(v).ok())
    }

    /// Scalar as a float. Integers widen losslessly; floats accept YAML
    /// `.inf`/`.nan` forms and exponent notation.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self.kind() {
            ValueKind::Float => parse_float(self.scalar_bytes()),
            ValueKind::Int => parse_int(self.scalar_bytes(), self.format()).map(|v| v as f64),
            _ => None,
        }
    }

    /// Raw source slice including any quotes/decorations.
    #[must_use]
    pub fn raw_text(&self) -> &'d [u8] {
        self.raw.text()
    }

    /// Byte range of the resolved content node.
    #[must_use]
    pub fn byte_range(&self) -> std::ops::Range<usize> {
        self.resolved().raw.byte_range()
    }

    /// Zero-based `(line, column-in-scalars)` of the resolved node start.
    #[must_use]
    pub fn line_col(&self) -> (u32, u32) {
        let doc = self.raw.doc();
        doc.line_index().line_col(doc.bytes(), self.byte_range().start)
    }

    /// Pointer from the document root to this node.
    ///
    /// Computed by walking up through parents; O(depth × width). Ranges are
    /// matched exactly so wrapper/content span collisions resolve correctly.
    #[must_use]
    pub fn path_from_root(&self) -> Pointer {
        fn done(tokens: &mut Vec<Box<str>>) -> Pointer {
            tokens.reverse();
            Pointer::from_tokens(std::mem::take(tokens))
        }
        let mut tokens: Vec<Box<str>> = Vec::new();
        let mut node = self.resolved().raw;
        #[allow(clippy::while_let_loop)] // multiple exit points read clearer as loop/break
        loop {
            // climb out of wrappers (block_node/flow_node/pair/sequence-item)
            let Some(container) = node.parent() else { break };
            let container = match NodeRef::new(container).structural_ancestor() {
                Some(c) => c,
                None => break,
            };
            let node_range = node.byte_range();
            match container.kind() {
                ValueKind::Object => {
                    let found = container.raw.mapping_entries().into_iter().find_map(|(k, v)| {
                        v.filter(|v| NodeRef::new(*v).byte_range() == node_range).map(|_| k)
                    });
                    match found {
                        Some(k) => tokens.push(
                            String::from_utf8_lossy(k.scalar_bytes()).to_string().into_boxed_str(),
                        ),
                        None => return done(&mut tokens),
                    }
                }
                ValueKind::Array => {
                    match container
                        .raw
                        .sequence_items()
                        .into_iter()
                        .position(|it| it.byte_range() == node_range)
                    {
                        Some(i) => tokens.push(i.to_string().into_boxed_str()),
                        None => return done(&mut tokens),
                    }
                }
                _ => break,
            }
            node = container.raw;
        }
        done(&mut tokens)
    }

    /// Climbs from this node through wrappers/pairs to the nearest mapping
    /// or sequence ancestor.
    fn structural_ancestor(&self) -> Option<NodeRef<'d>> {
        let mut cur = *self;
        loop {
            match cur.raw.kind() {
                SyntaxKind::Mapping | SyntaxKind::Sequence => return Some(cur),
                _ => {
                    let p = cur.raw.parent()?;
                    cur = NodeRef::new(p)
                },
            }
        }
    }

    /// Reports duplicate mapping keys within this object (non-recursive).
    #[must_use]
    pub fn duplicate_keys(&self) -> Vec<DuplicateKey> {
        let node = self.resolved();
        if node.kind() != ValueKind::Object {
            return Vec::new();
        }
        let mut order: Vec<String> = Vec::new();
        let mut map: rustc_hash::FxHashMap<String, Vec<std::ops::Range<usize>>> =
            rustc_hash::FxHashMap::default();
        for (key_node, value) in node.raw.mapping_entries() {
            let key = String::from_utf8_lossy(key_node.scalar_bytes()).to_string();
            let range = value.as_ref().map_or(key_node.byte_range(), |v| v.byte_range());
            map.entry(key.clone()).or_default().push(range);
            if !order.contains(&key) {
                order.push(key);
            }
        }
        order
            .into_iter()
            .filter_map(|k| {
                let occurrences = map.get(&k)?;
                (occurrences.len() > 1).then(|| DuplicateKey { key: k, occurrences: occurrences.clone() })
            })
            .collect()
    }
}


/// Merge-aware recursive key search without building entry lists.
fn find_in_merge<'d>(merge: NodeRef<'d>, key: &str) -> Option<NodeRef<'d>> {
    let r = merge.resolved();
    match r.kind() {
        ValueKind::Object => {
            for (k, v) in r.raw.mapping_entries() {
                let kb = k.scalar_bytes();
                if kb == key.as_bytes() {
                    return v.map(NodeRef::new);
                }
                if kb == b"<<"
                    && let Some(v) = v
                        && let Some(found) = find_in_merge(NodeRef::new(v), key) {
                            return Some(found);
                        }
            }
            None
        }
        ValueKind::Array => {
            r.items().into_iter().find_map(|item| find_in_merge(item, key))
        }
        _ => None,
    }
}

fn append_merge<'d>(out: &mut Vec<Entry<'d>>, seen: &mut Vec<&'d str>, merge: NodeRef<'d>) {
    let resolved = merge.resolved();
    match resolved.kind() {
        ValueKind::Object => {
            for entry in resolved.entries() {
                if !seen.contains(&entry.key) {
                    seen.push(entry.key);
                    out.push(entry);
                }
            }
        }
        ValueKind::Array => {
            for item in resolved.items() {
                append_merge(out, seen, item);
            }
        }
        _ => {}
    }
}
