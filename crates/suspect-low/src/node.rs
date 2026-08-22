use suspect_syntax::{Format, ScalarStyle, SyntaxKind};

use crate::Pointer;
use crate::scalar::{ValueKind, infer_scalar, parse_float, parse_int};

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
        f.debug_struct("Entry")
            .field("key", &self.key)
            .finish_non_exhaustive()
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
            _ => infer_scalar(
                node.raw.scalar_bytes(),
                node.raw.scalar_style(),
                node.format(),
            ),
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
                && let Some(v) = v
            {
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
            let Ok(key) = std::str::from_utf8(key_bytes) else {
                continue;
            };
            if key == "<<" {
                if let Some(v) = value {
                    merges.push(NodeRef::new(v));
                }
                continue;
            }
            seen.push(key);
            out.push(Entry {
                key,
                value: value.map(NodeRef::new),
            });
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

    /// Fully decoded scalar text per YAML 1.2 / JSON:
    ///
    /// - plain scalars: verbatim bytes
    /// - single-quoted: quotes stripped, `''` -> `'`
    /// - double-quoted: quotes stripped and escape sequences decoded
    ///   (`\n`, `\t`, `\\`, `\"`, `\uXXXX`, ...)
    /// - block scalars (`|` / `>` with chomping indicators): header stripped,
    ///   indentation removed, folding applied (`>` folds single breaks into
    ///   spaces), chomping applied (clip default, strip `-`, keep `+`)
    ///
    /// Multi-line non-block scalars are not produced by either grammar.
    #[must_use]
    pub fn decoded_scalar(&self) -> std::borrow::Cow<'d, [u8]> {
        let node = self.resolved().raw;
        match node.scalar_style() {
            suspect_syntax::ScalarStyle::Plain => std::borrow::Cow::Borrowed(node.scalar_bytes()),
            ScalarStyle::SingleQuoted => {
                let inner = strip_outer_quotes(node.text());
                // '' collapses to '
                if inner.windows(2).any(|w| w == b"''") {
                    std::borrow::Cow::Owned(replace_all(inner, b"''", b"'"))
                } else {
                    std::borrow::Cow::Borrowed(inner)
                }
            }
            ScalarStyle::DoubleQuoted => {
                let inner = strip_outer_quotes(node.text());
                std::borrow::Cow::Owned(unescape_double(inner))
            }
            ScalarStyle::Block => std::borrow::Cow::Owned(decode_block_scalar(node.text())),
        }
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
        doc.line_index()
            .line_col(doc.bytes(), self.byte_range().start)
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
            // A pair node is located by its value (or key when valueless):
            // parent mappings enumerate values, so compare against that range.
            let locate_range = if node.kind() == SyntaxKind::Pair {
                node.child_by_field("value")
                    .or_else(|| node.child_by_field("key"))
                    .map_or(node.byte_range(), |c| c.byte_range())
            } else {
                node.byte_range()
            };
            // climb out of wrappers (block_node/flow_node/pair/sequence-item)
            let Some(container) = node.parent() else {
                break;
            };
            let container = match NodeRef::new(container).structural_ancestor() {
                Some(c) => c,
                None => break,
            };
            let node_range = locate_range;
            match container.kind() {
                ValueKind::Object => {
                    let found = container
                        .raw
                        .mapping_entries()
                        .into_iter()
                        .find_map(|(k, v)| {
                            v.filter(|v| NodeRef::new(*v).byte_range() == node_range)
                                .map(|_| k)
                        });
                    match found {
                        Some(k) => tokens.push(
                            String::from_utf8_lossy(k.scalar_bytes())
                                .to_string()
                                .into_boxed_str(),
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
                }
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
            let range = value
                .as_ref()
                .map_or(key_node.byte_range(), |v| v.byte_range());
            map.entry(key.clone()).or_default().push(range);
            if !order.contains(&key) {
                order.push(key);
            }
        }
        order
            .into_iter()
            .filter_map(|k| {
                let occurrences = map.get(&k)?;
                (occurrences.len() > 1).then(|| DuplicateKey {
                    key: k,
                    occurrences: occurrences.clone(),
                })
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
                    && let Some(found) = find_in_merge(NodeRef::new(v), key)
                {
                    return Some(found);
                }
            }
            None
        }
        ValueKind::Array => r
            .items()
            .into_iter()
            .find_map(|item| find_in_merge(item, key)),
        _ => None,
    }
}

fn strip_outer_quotes(text: &[u8]) -> &[u8] {
    if text.len() >= 2
        && ((text[0] == b'"' && text[text.len() - 1] == b'"')
            || (text[0] == b'\'' && text[text.len() - 1] == b'\''))
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn replace_all(haystack: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}

fn unescape_double(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'b' => out.push(0x08),
                b'f' => out.push(0x0C),
                b'0' => out.push(0),
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b'/' => out.push(b'/'),
                b'u' | b'U' | b'x' => {
                    let width = match bytes[i] {
                        b'u' => 4,
                        b'U' => 8,
                        _ => 2,
                    };
                    if i + width < bytes.len() {
                        let hex = std::str::from_utf8(&bytes[i + 1..i + 1 + width]).ok();
                        if let Some(v) = hex
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .and_then(char::from_u32)
                        {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(v.encode_utf8(&mut buf).as_bytes());
                            i += width;
                            i += 1;
                            continue;
                        }
                    }
                    out.push(bytes[i]); // invalid escape: keep literally
                }
                other => out.push(other),
            }
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Decodes a `|`/`>` block scalar: strips the header, removes indentation,
/// applies folding and chomping.
fn decode_block_scalar(text: &[u8]) -> Vec<u8> {
    let split = text
        .iter()
        .position(|&b| b == b'\n')
        .map_or(text.len(), |i| i + 1);
    let header = &text[..split.min(text.len())];
    let body = &text[split..];
    let folded = header.first() == Some(&b'>');
    let chomp = header.iter().skip(1).find(|b| **b == b'-' || **b == b'+');

    // content indent = leading spaces of the first non-empty line
    let mut indent = None;
    for line in body.split_inclusive(|&b| b == b'\n') {
        let nonspace = line.iter().take_while(|&&b| b == b' ').count();
        if nonspace < line.len() {
            indent = Some(nonspace);
            break;
        }
    }
    let indent = indent.unwrap_or(0);

    let mut out: Vec<u8> = Vec::with_capacity(body.len());
    let mut lines = body
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty() || false)
        .peekable();
    // Re-split preserving structure: iterate raw lines without dropping empties.
    let mut raw_lines: Vec<&[u8]> = body.split_inclusive(|&b| b == b'\n').collect();
    if raw_lines.last().is_some_and(|l| l.ends_with(b"\n"))
        && let Some(last) = raw_lines.last_mut()
    {
        *last = &last[..last.len() - 1];
    }
    let mut prev_folded_break = false;
    let mut wrote_any = false;
    for line in &mut raw_lines {
        let bare: &[u8] = if line.ends_with(b"\n") {
            &line[..line.len() - 1]
        } else {
            line
        };
        let dedented: &[u8] =
            bare.get(indent..)
                .unwrap_or(if bare.is_empty() { b"" } else { bare });
        let is_blank = dedented.iter().all(|&b| b == b' ');
        if folded && !is_blank && wrote_any && !prev_folded_break {
            // fold: single break between two non-empty lines becomes a space
            out.push(b' ');
        } else if wrote_any {
            out.push(b'\n');
        }
        if is_blank {
            prev_folded_break = true;
            continue;
        }
        prev_folded_break = false;
        out.extend_from_slice(dedented);
        wrote_any = true;
        let _ = lines.next();
    }

    // chomping: clip (default) keeps exactly one trailing break if any content;
    // strip removes all trailing breaks; keep preserves them.
    match chomp {
        Some(b'-') => {
            while out.last() == Some(&b'\n') || out.last() == Some(&b' ') {
                if out.last() == Some(&b' ')
                    && !out.ends_with(b"\n ")
                    && !out.iter().all(|&b| b == b' ')
                {
                    break;
                }
                out.pop();
            }
        }
        Some(b'+') => {}
        _ => {
            while matches!(out.last(), Some(b'\n') | Some(b' ')) {
                out.pop();
            }
            out.push(b'\n');
        }
    }
    let _ = lines;
    out
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
