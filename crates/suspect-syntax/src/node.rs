use std::ops::Range;

use crate::doc::SourceDoc;
use crate::{Format, ScalarStyle, SyntaxKind};

/// A handle to a CST node, borrowing its [`SourceDoc`].
#[derive(Clone, Copy)]
pub struct SNode<'d> {
    doc: &'d SourceDoc,
    raw: tree_sitter::Node<'d>,
}

impl<'d> SNode<'d> {
    /// Wraps a raw tree-sitter node together with its owning document.
    #[must_use]
    pub fn new(doc: &'d SourceDoc, raw: tree_sitter::Node<'d>) -> Self {
        Self { doc, raw }
    }

    /// Underlying tree-sitter node (escape hatch).
    #[must_use]
    pub fn raw(&self) -> &tree_sitter::Node<'d> {
        &self.raw
    }

    /// The document this node belongs to; source text and lookups resolve
    /// against it.
    #[must_use]
    pub fn doc(&self) -> &'d SourceDoc {
        self.doc
    }

    /// Normalized kind, unified across JSON and YAML grammars.
    #[must_use]
    pub fn kind(&self) -> SyntaxKind {
        map_kind(self.doc.format(), self.raw.kind())
    }

    /// Grammar-native kind name (escape hatch for diagnostics).
    #[must_use]
    pub fn raw_kind(&self) -> &str {
        self.raw.kind()
    }

    /// Raw source slice for this node — lossless by construction.
    #[must_use]
    pub fn text(&self) -> &'d [u8] {
        let r = self.byte_range();
        &self.doc.bytes()[r.start..r.end]
    }

    /// Raw source slice as `str`, if valid UTF-8.
    #[must_use]
    pub fn text_lossy(&self) -> std::borrow::Cow<'d, str> {
        String::from_utf8_lossy(self.text())
    }

    /// Half-open byte range of this node in the document buffer, including
    /// any attached decorations.
    #[must_use]
    pub fn byte_range(&self) -> Range<usize> {
        self.raw.byte_range()
    }

    /// First byte of [`byte_range`](Self::byte_range).
    #[must_use]
    pub fn start_byte(&self) -> usize {
        self.raw.start_byte()
    }

    /// One past the last byte of [`byte_range`](Self::byte_range).
    #[must_use]
    pub fn end_byte(&self) -> usize {
        self.raw.end_byte()
    }

    /// Enclosing node, if any (the root returns `None`).
    #[must_use]
    pub fn parent(&self) -> Option<SNode<'d>> {
        self.raw.parent().map(|n| SNode::new(self.doc, n))
    }

    /// Child occupying a named grammar field (e.g. `"key"`/`"value"` on a
    /// pair), if that field is present for this node's grammar shape.
    #[must_use]
    pub fn child_by_field(&self, field: &str) -> Option<SNode<'d>> {
        self.raw.child_by_field_name(field).map(|n| SNode::new(self.doc, n))
    }

    /// All children including anonymous tokens.
    pub fn children(&self) -> impl Iterator<Item = SNode<'d>> {
        let mut cursor = self.raw.walk();
        let mut started = false;
        let doc = self.doc;
        std::iter::from_fn(move || {
            if !started {
                if !cursor.goto_first_child() {
                    return None;
                }
                started = true;
            } else if !cursor.goto_next_sibling() {
                return None;
            }
            Some(SNode::new(doc, cursor.node()))
        })
    }

    /// All named children.
    pub fn named_children(&self) -> impl Iterator<Item = SNode<'d>> {
        self.children().filter(|c| c.raw().is_named())
    }

    /// Pre-order descendants including `self`.
    pub fn descendants(&self) -> impl Iterator<Item = SNode<'d>> {
        let mut stack = vec![*self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            let mut cursor = node.raw.walk();
            if cursor.goto_first_child() {
                loop {
                    stack.push(SNode::new(node.doc(), cursor.node()));
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            Some(node)
        })
    }

    /// True for tree-sitter ERROR or MISSING nodes. Such nodes can appear
    /// anywhere in an otherwise-parseable document; callers must tolerate
    /// them rather than assuming a well-formed tree.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.raw.is_error() || self.raw.is_missing()
    }

    /// Descends through stream/document/wrapper nodes (YAML `block_node`,
    /// `flow_node`; JSON `_value`) to the first meaningful node.
    #[must_use]
    pub fn content(&self) -> SNode<'d> {
        let mut node = *self;
        loop {
            match node.kind() {
                SyntaxKind::Stream | SyntaxKind::Document => {}
                _ if is_wrapper(node.raw_kind()) => {}
                _ => return node,
            }
            match node.first_meaningful_child() {
                Some(child) => node = child,
                None => return node,
            }
        }
    }

    fn first_meaningful_child(&self) -> Option<SNode<'d>> {
        self.children().find(|c| c.raw().is_named() && !is_decoration(c.kind()))
    }

    /// If this is a mapping, yields `(key, value)` pairs with keys resolved
    /// to their scalar content node and values to their content node
    /// (`None` for empty values like `key:` with nothing after it).
    ///
    /// Duplicate keys are yielded as-is; policy belongs above this layer.
    #[must_use]
    pub fn mapping_entries(&self) -> Vec<(SNode<'d>, Option<SNode<'d>>)> {
        let node = self.content();
        if node.kind() != SyntaxKind::Mapping {
            return Vec::new();
        }
        let mut out = Vec::new();
        for child in node.children() {
            if child.kind() != SyntaxKind::Pair || child.is_error() {
                continue;
            }
            let key = child.child_by_field("key").map(|k| k.content());
            let value = child.child_by_field("value").map(|v| v.content());
            if let Some(key) = key {
                out.push((key, value));
            }
        }
        out
    }

    /// If this is a sequence, yields item content nodes.
    #[must_use]
    pub fn sequence_items(&self) -> Vec<SNode<'d>> {
        let node = self.content();
        if node.kind() != SyntaxKind::Sequence {
            return Vec::new();
        }
        let mut out = Vec::new();
        for child in node.children().filter(|c| c.raw().is_named()) {
            match child.kind() {
                SyntaxKind::Comment | SyntaxKind::Directive | SyntaxKind::Anchor | SyntaxKind::Tag => {}
                _ if child.is_error() => {}
                _ => out.push(child.content()),
            }
        }
        out
    }

    /// Looks up a key in a mapping (first match wins; duplicates reported by
    /// a dedicated duplicate scan).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<SNode<'d>> {
        self.mapping_entries()
            .into_iter()
            .find(|(k, _)| k.scalar_bytes() == key)
            .and_then(|(_, v)| v)
    }

    /// Unquoted scalar bytes: strips JSON string quotes and YAML quoting,
    /// without escape processing (`\n` stays literal). Block scalars return
    /// their raw slice including indentation — semantic decoding is a
    /// `suspect-low` concern.
    #[must_use]
    pub fn scalar_bytes(&self) -> &'d [u8] {
        let node = self.content();
        let text = node.text();
        match node.scalar_style() {
            ScalarStyle::DoubleQuoted | ScalarStyle::SingleQuoted => {
                strip_quotes(text)
            }
            _ => text,
        }
    }

    /// How this scalar was written.
    #[must_use]
    pub fn scalar_style(&self) -> ScalarStyle {
        let node = self.content();
        match node.doc.format() {
            Format::Json => match node.raw.kind() {
                "string" => ScalarStyle::DoubleQuoted,
                _ => ScalarStyle::Plain,
            },
            Format::Yaml => match node.raw.kind() {
                "single_quote_scalar" => ScalarStyle::SingleQuoted,
                "double_quote_scalar" => ScalarStyle::DoubleQuoted,
                "block_scalar" => ScalarStyle::Block,
                _ => ScalarStyle::Plain,
            },
        }
    }


    /// S-expression dump (debugging).
    #[must_use]
    pub fn to_sexp(&self) -> String {
        self.raw.to_sexp()
    }

    /// Alias target name for an `*alias` node (without the `*`).
    #[must_use]
    pub fn alias_name(&self) -> Option<&'d str> {
        let text = std::str::from_utf8(self.text()).ok()?;
        text.strip_prefix('*')
    }

    /// Anchor name for an `&anchor` node (without the `&`).
    #[must_use]
    pub fn anchor_name(&self) -> Option<&'d str> {
        let text = std::str::from_utf8(self.text()).ok()?;
        text.strip_prefix('&')
    }
}

fn strip_quotes(text: &[u8]) -> &[u8] {
    if text.len() >= 2
        && ((text[0] == b'"' && text[text.len() - 1] == b'"')
            || (text[0] == b'\'' && text[text.len() - 1] == b'\''))
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn is_wrapper(raw_kind: &str) -> bool {
    matches!(raw_kind, "block_node" | "flow_node" | "_value" | "block_sequence_item")
}

/// Decorations attach to values but are not values.
#[must_use]
pub fn is_decoration(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::Anchor | SyntaxKind::Tag | SyntaxKind::Comment | SyntaxKind::Directive)
}

fn map_kind(format: Format, raw: &str) -> SyntaxKind {
    match format {
        Format::Json => match raw {
            "document" => SyntaxKind::Document,
            "object" => SyntaxKind::Mapping,
            "array" => SyntaxKind::Sequence,
            "pair" => SyntaxKind::Pair,
            "string" | "number" | "true" | "false" | "null" => SyntaxKind::Scalar,
            "comment" => SyntaxKind::Comment,
            "ERROR" => SyntaxKind::Error,
            _ => SyntaxKind::Error,
        },
        Format::Yaml => match raw {
            "stream" => SyntaxKind::Stream,
            "document" => SyntaxKind::Document,
            "block_mapping" | "flow_mapping" => SyntaxKind::Mapping,
            "block_sequence" | "flow_sequence" => SyntaxKind::Sequence,
            "block_mapping_pair" | "flow_pair" => SyntaxKind::Pair,
            "plain_scalar"
            | "single_quote_scalar"
            | "double_quote_scalar"
            | "block_scalar"
            | "integer_scalar"
            | "float_scalar"
            | "boolean_scalar"
            | "null_scalar"
            | "string_scalar"
            | "timestamp_scalar" => SyntaxKind::Scalar,
            "anchor" => SyntaxKind::Anchor,
            "alias" => SyntaxKind::Alias,
            "tag" => SyntaxKind::Tag,
            "comment" => SyntaxKind::Comment,
            "yaml_directive" | "tag_directive" | "reserved_directive" => SyntaxKind::Directive,
            "ERROR" => SyntaxKind::Error,
            _ => SyntaxKind::Error,
        },
    }
}
