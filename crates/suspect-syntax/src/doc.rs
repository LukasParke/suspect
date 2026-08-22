use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use suspect_source::{LineIndex, Source, Uri};
use tree_sitter::{InputEdit, Parser, Tree};

use crate::node::SNode;
use crate::{Format, SyntaxKind};

/// A position (row, column) in the decoded buffer; columns are byte offsets
/// within the line, matching tree-sitter points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    /// Zero-based line number.
    pub row: usize,
    /// Zero-based byte offset within the line.
    pub column: usize,
}

impl From<tree_sitter::Point> for Point {
    fn from(p: tree_sitter::Point) -> Self {
        Self {
            row: p.row,
            column: p.column,
        }
    }
}

/// A text edit with exact positions, used for incremental reparsing.
#[derive(Debug, Clone, Copy)]
pub struct Edit {
    /// Byte offset where the replaced region begins.
    pub start_byte: usize,
    /// Byte offset just past the replaced region in the *old* text.
    pub old_end_byte: usize,
    /// Byte offset just past the replacement in the *new* text
    /// (`start_byte + new_text_len`).
    pub new_end_byte: usize,
    /// Position of `start_byte` in the old document.
    pub start_point: Point,
    /// Position of `old_end_byte` in the old document.
    pub old_end_point: Point,
    /// Position of `new_end_byte` in the new document.
    pub new_end_point: Point,
}

impl Edit {
    /// Builds an edit from byte offsets plus precomputed points.
    #[must_use]
    pub const fn new(
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
        start_point: Point,
        old_end_point: Point,
        new_end_point: Point,
    ) -> Self {
        Self {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point,
        }
    }

    /// Computes exact points against `old_doc`'s buffer; `new_text_len` is the
    /// replacement length in bytes. The new end point assumes the replacement
    /// introduces no new lines beyond those in the removed text — callers
    /// replacing across line boundaries should supply points explicitly.
    #[must_use]
    pub fn from_bytes(
        doc: &SourceDoc,
        start_byte: usize,
        old_end_byte: usize,
        new_text_len: usize,
    ) -> Self {
        let bytes = doc.bytes();
        let (start_row, start_col) = doc.line_index().line_col_bytes(bytes, start_byte);
        let (old_row, old_col) = doc.line_index().line_col_bytes(bytes, old_end_byte);
        let inserted = &doc.bytes()[start_byte..old_end_byte];
        let new_lines = inserted.iter().filter(|&&b| b == b'\n').count();
        let new_end_point = if new_lines == 0 {
            Point {
                row: start_row,
                column: start_col + new_text_len,
            }
        } else {
            // last inserted line length after the final '\n'
            let last_nl = inserted
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |i| i + 1);
            let tail = new_text_len.saturating_sub(inserted.len() - last_nl);
            Point {
                row: start_row + new_lines,
                column: tail,
            }
        };
        Self {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + new_text_len,
            start_point: Point {
                row: start_row,
                column: start_col,
            },
            old_end_point: Point {
                row: old_row,
                column: old_col,
            },
            new_end_point,
        }
    }
}

/// Anchor name -> (value byte range, value kind).
pub(crate) type AnchorMap = HashMap<Box<str>, (Range<usize>, SyntaxKind)>;

/// A syntax-level problem found by tree-sitter error recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Half-open byte range of the offending node in the source.
    pub range: Range<usize>,
    /// Human-readable description of the problem.
    pub message: String,
}

/// A parsed document: bytes + tree-sitter tree + line index, immutable.
///
/// All node access goes through [`SNode`] handles borrowed from
/// `&SourceDoc`. Reparsing produces a fresh `SourceDoc`; unchanged subtrees
/// are reused by tree-sitter internally.
pub struct SourceDoc {
    uri: Uri,
    source: Source,
    format: Format,
    tree: Tree,
    line_index: LineIndex,
    errors: OnceLock<Vec<SyntaxError>>,
    anchors: OnceLock<AnchorMap>,
}

impl SourceDoc {
    /// Parses with format auto-detection (JSON first when it looks like JSON,
    /// YAML as fallback, fewest-errors wins for ambiguous inputs).
    #[must_use]
    pub fn parse(uri: Uri, source: Source) -> SourceDoc {
        let format = detect_format(source.bytes());
        Self::with_format(uri, source, format)
    }

    /// Parses with an explicit format.
    #[must_use]
    pub fn with_format(uri: Uri, source: Source, format: Format) -> SourceDoc {
        let mut parser = Parser::new();
        let language = match format {
            Format::Json => crate::json_language(),
            Format::Yaml => crate::yaml_language(),
        };
        parser
            .set_language(&language)
            .expect("vendored grammar ABI must match runtime");
        let tree = parser
            .parse(source.bytes(), None)
            .expect("tree-sitter parse never returns None without a timeout");
        let line_index = LineIndex::new(source.bytes());
        Self {
            uri,
            source,
            format,
            tree,
            line_index,
            errors: OnceLock::new(),
            anchors: OnceLock::new(),
        }
    }

    /// Reparses after edits, reusing unchanged subtrees (incremental).
    #[must_use]
    pub fn reparse(&self, new_source: Source, edits: &[Edit]) -> SourceDoc {
        let mut tree = self.tree.clone();
        for edit in edits {
            tree.edit(&InputEdit {
                start_byte: edit.start_byte,
                old_end_byte: edit.old_end_byte,
                new_end_byte: edit.new_end_byte,
                start_position: tree_sitter::Point {
                    row: edit.start_point.row,
                    column: edit.start_point.column,
                },
                old_end_position: tree_sitter::Point {
                    row: edit.old_end_point.row,
                    column: edit.old_end_point.column,
                },
                new_end_position: tree_sitter::Point {
                    row: edit.new_end_point.row,
                    column: edit.new_end_point.column,
                },
            });
        }
        let mut parser = Parser::new();
        let language = match self.format {
            Format::Json => crate::json_language(),
            Format::Yaml => crate::yaml_language(),
        };
        parser
            .set_language(&language)
            .expect("vendored grammar ABI must match runtime");
        let new_tree = parser
            .parse(new_source.bytes(), Some(&tree))
            .expect("tree-sitter parse never returns None without a timeout");
        let line_index = LineIndex::new(new_source.bytes());
        SourceDoc {
            uri: self.uri.clone(),
            source: new_source,
            format: self.format,
            line_index,
            tree: new_tree,
            errors: OnceLock::new(),
            anchors: OnceLock::new(),
        }
    }
    /// Document identifier; carried through reparses unchanged.
    #[must_use]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
    /// The format this document was parsed as.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }
    /// The full source buffer, lossless.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.source.bytes()
    }

    /// Whole-document lossless emission (the decoded buffer itself).
    #[must_use]
    pub fn emit(&self) -> &[u8] {
        self.source.bytes()
    }
    /// Line index mapping byte offsets to `(row, column)` positions and back.
    #[must_use]
    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// Root node of the syntax tree.
    #[must_use]
    pub fn root(&self) -> SNode<'_> {
        SNode::new(self, self.tree.root_node())
    }

    /// `(row, column-in-bytes)` for an offset — tree-sitter point semantics.
    #[must_use]
    pub fn point_at(&self, offset: usize) -> Point {
        let (row, column) = self.line_index.line_col_bytes(self.bytes(), offset);
        Point { row, column }
    }

    /// Syntax errors (ERROR/MISSING nodes), computed once on first call.
    #[must_use]
    pub fn errors(&self) -> &[SyntaxError] {
        self.errors.get_or_init(|| {
            let mut out = Vec::new();
            collect_errors(self.root(), &mut out);
            out
        })
    }
    /// Cheap error check without materializing the [`SyntaxError`] list:
    /// true when the tree contains any ERROR or MISSING node.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.tree.root_node().has_error()
    }

    /// YAML anchor name → byte range of the anchored value. Empty for JSON.
    #[must_use]
    pub fn anchors(&self) -> &HashMap<Box<str>, (Range<usize>, SyntaxKind)> {
        self.anchors.get_or_init(|| {
            let mut map = HashMap::default();
            if self.format == Format::Yaml {
                collect_anchors(self.root(), &mut map);
            }
            map
        })
    }

    /// Resolves an alias name to the anchored node, if present.
    #[must_use]
    pub fn anchor_target(&self, name: &str) -> Option<SNode<'_>> {
        let (range, kind) = self.anchors().get(name)?.clone();
        // wrappers and their content can share a byte range; pre-order scan
        // takes the outermost exact match
        self.root()
            .descendants()
            .find(|n| n.byte_range() == range && n.kind() == kind)
    }
}

impl std::fmt::Debug for SourceDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceDoc")
            .field("uri", &self.uri.as_str())
            .field("format", &self.format)
            .field("len", &self.source.len())
            .field("errors", &self.errors().len())
            .finish()
    }
}

fn collect_errors(node: SNode<'_>, out: &mut Vec<SyntaxError>) {
    if node.raw().is_error() {
        out.push(SyntaxError {
            range: node.byte_range(),
            message: format!("syntax error near {:?}", node.text_lossy()),
        });
    } else if node.raw().is_missing() {
        out.push(SyntaxError {
            range: node.byte_range(),
            message: "missing node".into(),
        });
    }
    for child in node.children() {
        collect_errors(child, out);
    }
}

fn collect_anchors(node: SNode<'_>, map: &mut AnchorMap) {
    if node.kind() == SyntaxKind::Anchor
        && let Some(name) = anchor_name(node)
        && let Some(value) = anchored_value(node)
    {
        map.entry(name)
            .or_insert((value.byte_range(), value.kind()));
    }
    for child in node.children() {
        collect_anchors(child, map);
    }
}

fn anchor_name(anchor: SNode<'_>) -> Option<Box<str>> {
    let text: &str = &anchor.text_lossy();
    let stripped = text.strip_prefix('&').unwrap_or(text);
    let name = stripped.split_whitespace().next()?;
    Some(name.into())
}

fn anchored_value(anchor: SNode<'_>) -> Option<SNode<'_>> {
    let parent = anchor.parent()?;
    let mut seen_anchor = false;
    for child in parent.children() {
        match child.kind() {
            SyntaxKind::Anchor | SyntaxKind::Tag | SyntaxKind::Comment | SyntaxKind::Directive => {
                if child.raw().id() == anchor.raw().id() {
                    seen_anchor = true;
                }
            }
            _ if seen_anchor => return Some(child),
            _ => {}
        }
    }
    None
}

fn detect_format(bytes: &[u8]) -> Format {
    let first = bytes.iter().find(|&&b| !b.is_ascii_whitespace());
    match first {
        Some(b'{') | Some(b'[') => {
            // Looks like JSON. Commit if it parses cleanly; otherwise compare
            // both parses and keep the one with fewer errors (JSON wins ties).
            if let Some(json_errors) = count_errors(bytes, Format::Json) {
                if json_errors == 0 {
                    return Format::Json;
                }
                if let Some(yaml_errors) = count_errors(bytes, Format::Yaml) {
                    return if json_errors <= yaml_errors {
                        Format::Json
                    } else {
                        Format::Yaml
                    };
                }
            }
            Format::Json
        }
        _ => Format::Yaml,
    }
}

fn count_errors(bytes: &[u8], format: Format) -> Option<usize> {
    let mut parser = Parser::new();
    let language = match format {
        Format::Json => crate::json_language(),
        Format::Yaml => crate::yaml_language(),
    };
    parser.set_language(&language).ok()?;
    let tree = parser.parse(bytes, None)?;
    let mut count = 0usize;
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            count += 1;
        }
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                stack.push(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    Some(count)
}
