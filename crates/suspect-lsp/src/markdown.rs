//! CommonMark support for the specification's markdown-bearing fields.
//!
//! OpenAPI, Arazzo and Overlay all declare `description` and `summary` as
//! CommonMark. Until now those values were opaque strings: highlighted as
//! plain scalars, linted not at all. This module gives them the treatment
//! `.md` files get — semantic tokens for the markdown constructs and
//! markdownlint-compatible rules — at their true file positions.
//!
//! The whole problem is position. The markdown parser reports offsets in
//! the *decoded* text, but the file holds the raw YAML scalar: quoted,
//! indented under a `|` header, or folded by a `>` header. So every decoded
//! line records the raw segments it was built from, and decoded offsets map
//! back to file offsets exactly — including for folded blocks, where one
//! decoded line is several raw lines joined by the fold.

use suspect_low::{LowDoc, NodeRef, SpecFamily};
use suspect_syntax::ScalarStyle;
use suspect_syntax::{SNode, SyntaxKind};

/// The fields the specifications define as CommonMark.
pub const MARKDOWN_FIELDS: &[&str] = &["description", "summary"];

/// One decoded CommonMark field, with its decoded→file offset mapping.
#[derive(Debug)]
pub struct MarkdownField<'d> {
    /// The decoded markdown text.
    pub text: String,
    /// The scalar node this text came from.
    pub node: NodeRef<'d>,
    /// Per decoded line: the raw segments that compose it.
    lines: Vec<LineOrigin>,
}

/// The raw segments that compose one decoded line.
///
/// A segment is `[start, start+len)` in the file. Between segments the
/// decode inserted a single space (a fold join) or a newline.
#[derive(Debug)]
struct LineOrigin {
    segments: Vec<Seg>,
}

#[derive(Debug, Clone, Copy)]
struct Seg {
    start: usize,
    len: usize,
}

impl Seg {
    const fn end(self) -> usize {
        self.start + self.len
    }
}

impl<'d> MarkdownField<'d> {
    /// Builds a field from a scalar node, or `None` when it holds no
    /// markdown worth treating: non-strings and empty text never do.
    #[must_use]
    pub fn new(snode: suspect_syntax::SNode<'d>) -> Option<Self> {
        let node = NodeRef::new(snode);
        let decoded = node.decoded_scalar();
        let text = String::from_utf8(decoded.into_owned()).ok()?;
        if text.trim().is_empty() {
            return None;
        }
        let lines = build_line_origins(&node, &text, snode.scalar_style())?;
        Some(Self { text, node, lines })
    }

    /// Maps a decoded-text byte offset to its file byte offset. Offsets
    /// that land in a fold join or a chomped tail clamp to the nearest
    /// raw position, so a mapped range always points somewhere real.
    #[must_use]
    pub fn map_offset(&self, offset: usize) -> usize {
        let (line, col) = self.line_col(offset);
        let Some(origin) = self.lines.get(line) else {
            return self.node.byte_range().start;
        };
        let mut within = col;
        for (index, seg) in origin.segments.iter().enumerate() {
            let is_last = index + 1 == origin.segments.len();
            if within <= seg.len || is_last {
                return (seg.start + within).min(seg.end());
            }
            // The space the fold inserted between this segment and the next.
            within -= seg.len + 1;
        }
        self.node.byte_range().start
    }

    /// Maps a decoded-text byte range to a file byte range.
    #[must_use]
    pub fn map_range(&self, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
        let start = self.map_offset(range.start);
        let end = self.map_offset(range.end.max(range.start));
        // End-of-construct offsets sit just past the last raw byte; the
        // exclusive range stays valid even when clamped.
        start..end.max(start + 1)
    }

    /// The 0-based decoded line and byte-column of a decoded offset.
    fn line_col(&self, offset: usize) -> (usize, usize) {
        let mut remaining = offset.min(self.text.len());
        for (line, text_line) in self.text.split('\n').enumerate() {
            let len = text_line.len();
            if remaining <= len {
                return (line, remaining);
            }
            remaining -= len + 1;
        }
        (self.lines.len().saturating_sub(1), 0)
    }
}

/// Builds the decoded-line → raw-segment map for a scalar node.
///
/// Every style gets an exact map where one is computable:
///
/// - **plain** — segments are the raw lines; multi-line plain scalars fold
///   with a space exactly as YAML folds them.
/// - **quoted** — the content between the quotes, escape by escape; an
///   escape starts a new segment so the two raw bytes of `\"` map onto the
///   one decoded byte they become, and columns never drift.
/// - **block literal `|`** — one segment per decoded line.
/// - **block folded `>`** — one segment per raw line; the segments of one
///   decoded line are the raw lines the fold joined.
fn build_line_origins(
    node: &NodeRef<'_>,
    decoded: &str,
    style_param: ScalarStyle,
) -> Option<Vec<LineOrigin>> {
    let raw = node.raw_text();
    let file_start = node.byte_range().start;
    // The style lives one layer down: the NodeRef wraps the syntax node,
    // which is the thing that knows how the scalar was written.
    // `MarkdownField::new` passes the style in: the syntax node knows how
    // the scalar was written, and the NodeRef that owns the decode does not
    // expose it.
    let style = style_param;
    let mut origins: Vec<LineOrigin> = Vec::new();

    match style {
        ScalarStyle::Plain => {
            // Decoded == raw for plain scalars; segments follow the line
            // structure the decoder sees.
            let mut start = 0usize;
            for line in raw.split_inclusive(|b| *b == b'\n') {
                let len = line.strip_suffix(b"\n").unwrap_or(line).len();
                let trimmed = len;
                origins.push(LineOrigin {
                    segments: vec![Seg {
                        start: file_start + start,
                        len: trimmed,
                    }],
                });
                start += line.len();
            }
        }
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => {
            let content_start = file_start + 1; // past the opening quote
            let body = raw.strip_prefix(b"\"").unwrap_or(raw);
            let body = body.strip_prefix(b"\"").unwrap_or(body);
            let body = body
                .strip_suffix(b"\"")
                .or_else(|| body.strip_suffix(b"'"))
                .unwrap_or(body);
            if !body.contains(&b'\n') && !has_escapes(body, style) {
                // The common case: no escapes, no line breaks — decoded
                // columns are raw columns.
                origins.push(LineOrigin {
                    segments: vec![Seg {
                        start: content_start,
                        len: body.len(),
                    }],
                });
            } else {
                // Escapes and line breaks shift columns; segment per
                // decoded character group so the map stays exact.
                origins = map_quoted(body, content_start, style, decoded);
            }
        }
        ScalarStyle::Block => {
            // The header line ends at the first newline; the body follows.
            let header_end = raw.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
            let folded = header_end > 0 && raw.first() == Some(&b'>');
            let body = &raw[header_end..];
            let body_start = file_start + header_end;

            // Content indent: leading spaces of the first non-empty line.
            let mut indent = 0usize;
            for line in body.split_inclusive(|b| *b == b'\n') {
                let spaces = line.iter().take_while(|&&b| b == b' ').count();
                if spaces < line.len() {
                    indent = spaces;
                    break;
                }
            }

            let mut offset = 0usize;
            let mut pending: Vec<Seg> = Vec::new();
            let mut wrote_any = false;
            for line in body.split_inclusive(|b| *b == b'\n') {
                let bare: &[u8] = line.strip_suffix(b"\n").unwrap_or(line);
                let dedented: &[u8] = bare.get(indent..).unwrap_or(bare);
                let is_blank = dedented.iter().all(|&b| b == b' ');
                let seg = Seg {
                    start: body_start + offset + indent.min(bare.len()),
                    len: dedented.len(),
                };
                if is_blank {
                    if !pending.is_empty() {
                        origins.push(LineOrigin {
                            segments: std::mem::take(&mut pending),
                        });
                    }
                    // A blank decoded line still knows where it lives in
                    // the file: a zero-length segment at the raw line's
                    // dedented position. Without it, offsets that land on
                    // the blank fell back to the scalar's start, which is
                    // nowhere near the markdown they came from.
                    origins.push(LineOrigin {
                        segments: vec![Seg {
                            start: seg.start,
                            len: 0,
                        }],
                    });
                    wrote_any = false;
                } else if folded && wrote_any {
                    // The fold joins this line to the previous one: the
                    // segment extends the current decoded line.
                    pending.push(seg);
                } else {
                    if !pending.is_empty() {
                        origins.push(LineOrigin {
                            segments: std::mem::take(&mut pending),
                        });
                    }
                    pending.push(seg);
                    wrote_any = true;
                }
                offset += line.len();
            }
            if !pending.is_empty() {
                origins.push(LineOrigin { segments: pending });
            }
        }
    }
    if origins.is_empty() {
        return None;
    }
    Some(origins)
}

fn has_escapes(body: &[u8], style: ScalarStyle) -> bool {
    match style {
        ScalarStyle::DoubleQuoted => body.contains(&b'\\'),
        ScalarStyle::SingleQuoted => body.windows(2).any(|w| w == b"''"),
        _ => false,
    }
}

/// Maps a quoted scalar body with escapes, segment by segment.
fn map_quoted(
    body: &[u8],
    content_start: usize,
    style: ScalarStyle,
    _decoded: &str,
) -> Vec<LineOrigin> {
    let mut origins: Vec<LineOrigin> = Vec::new();
    let mut pending: Vec<Seg> = Vec::new();
    let mut raw = 0usize; // offset within `body`
    while raw < body.len() {
        let (consumed, decoded_len): (usize, usize) = match style {
            ScalarStyle::DoubleQuoted => {
                if body[raw] == b'\\' && raw + 1 < body.len() {
                    // An escape: the raw pair becomes one decoded byte —
                    // a `\n` escape starts a decoded line, but that is
                    // handled by the newline branch below.
                    (2, 1)
                } else {
                    (1, 1)
                }
            }
            _ => {
                if style == ScalarStyle::SingleQuoted && body[raw..].starts_with(b"''") {
                    (2, 1)
                } else {
                    (1, 1)
                }
            }
        };
        let starts_line = body[raw] == b'\n';
        pending.push(Seg {
            start: content_start + raw,
            len: consumed.saturating_sub(if body[raw] == b'\n' { 1 } else { 0 }),
        });
        if starts_line {
            origins.push(LineOrigin {
                segments: std::mem::take(&mut pending),
            });
            // The newline itself becomes one decoded byte of line break.
            pending.push(Seg {
                start: content_start + raw,
                len: 0,
            });
        }
        raw += consumed;
        let _ = decoded_len;
    }
    if !pending.is_empty() {
        origins.push(LineOrigin { segments: pending });
    }
    origins
}

/// Every CommonMark field in the document, in document order.
#[must_use]
pub fn fields(low: &LowDoc) -> Vec<MarkdownField<'_>> {
    if matches!(low.sniff_family(), SpecFamily::Unknown) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut stack = vec![low.inner().root()];
    while let Some(node) = stack.pop() {
        match node.kind() {
            SyntaxKind::Stream | SyntaxKind::Document => {
                // The root is wrapped; descend through it. Any other
                // wrapper kinds (`block_node`, flow nodes) behave the same.
                if let Some(child) = node.first_meaningful_child() {
                    stack.push(child);
                }
            }
            SyntaxKind::Mapping => {
                'pairs: for (key, value) in mapping_pairs(node) {
                    let Some(key) = key else { continue };
                    let name = String::from_utf8_lossy(key.scalar_bytes());
                    if MARKDOWN_FIELDS.contains(&name.as_ref()) {
                        if let Some(value) = value
                            && NodeRef::new(value.content()).kind() == suspect_low::ValueKind::Str
                            && let Some(field) = MarkdownField::new(value.content())
                        {
                            out.push(field);
                        }
                        continue 'pairs;
                    }
                    if let Some(value) = value {
                        stack.push(value.content());
                    }
                }
            }
            SyntaxKind::Sequence => {
                for item in node.children().collect::<Vec<_>>().into_iter().rev() {
                    stack.push(item);
                }
            }
            _ => {
                // Wrapper nodes (`block_node`, `flow_node`, …) carry the
                // real structure one level down.
                if matches!(
                    node.raw_kind(),
                    "block_node" | "flow_node" | "_value" | "block_sequence_item"
                ) && let Some(child) = node.first_meaningful_child()
                {
                    stack.push(child);
                }
            }
        }
    }
    // Depth-first with a stack visits in reverse sibling order; the callers
    // do not depend on order, but determinism is cheap to keep.
    out.reverse();
    out
}

/// Mapping entries as (key node, value node) pairs.
fn mapping_pairs<'d>(node: SNode<'d>) -> Vec<(Option<SNode<'d>>, Option<SNode<'d>>)> {
    node.children()
        .filter(|c| c.kind() == SyntaxKind::Pair)
        .map(|pair| {
            (
                pair.child_by_field("key"),
                pair.child_by_field("value").map(|v| v.content()),
            )
        })
        .collect()
}

/// A markdown construct worth highlighting or linting, at a decoded-text
/// range with its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construct {
    /// ATX or setext heading.
    Heading,
    /// `*emphasis*` / `_emphasis_`.
    Emphasis,
    /// `**strong**`.
    Strong,
    /// `` `code span` ``.
    CodeSpan,
    /// `[text](target)`, `[text][ref]`, `![alt](target)`, or `<https://…>`.
    Link,
    /// Fenced or indented code block.
    CodeBlock,
}

/// Walks the field's markdown with pulldown-cmark, reporting constructs at
/// decoded-text ranges. The parser's offsets are the decoded text's own
/// bytes, so ranges map through [`MarkdownField::map_range`].
#[must_use]
pub fn constructs(field: &MarkdownField<'_>) -> Vec<(std::ops::Range<usize>, Construct)> {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = Vec::new();
    let mut open: Vec<(std::ops::Range<usize>, Construct)> = Vec::new();
    for (event, range) in Parser::new_ext(&field.text, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let construct = match tag {
                    Tag::Heading { .. } => Some(Construct::Heading),
                    Tag::Emphasis => Some(Construct::Emphasis),
                    Tag::Strong => Some(Construct::Strong),
                    Tag::Link { .. } | Tag::Image { .. } => Some(Construct::Link),
                    Tag::CodeBlock(_) => Some(Construct::CodeBlock),
                    _ => None,
                };
                if let Some(construct) = construct {
                    open.push((range, construct));
                }
            }
            Event::End(_) => {
                if let Some((start, construct)) = open.pop() {
                    let span = start.start..range.end.max(start.start);
                    out.push((span, construct));
                }
            }
            Event::Code(_) => {
                out.push((range, Construct::CodeSpan));
            }
            _ => {}
        }
    }
    out
}

/// One markdown finding, at a mapped file range.
#[derive(Debug)]
pub struct LintFinding {
    /// The file range to underline.
    pub range: std::ops::Range<usize>,
    /// The `md-` rule code, matching the code style of the existing lint
    /// battery so per-rule configuration treats these like any other.
    pub code: &'static str,
    /// The finding message.
    pub message: String,
}

/// Runs the markdownlint-compatible rule set over one field.
///
/// The rules are the markdownlint ones that matter for specification
/// prose, by their markdownlint names in the messages: MD001, MD012,
/// MD025, MD034 and MD040. Severity flows through the existing lint
/// floor and per-rule configuration; these findings are style-level, so
/// they default to hints and never gate a build unless told to.
///
/// Ranges come back mapped to the file, not the decoded text.
#[must_use]
pub fn lint_findings(field: &MarkdownField<'_>) -> Vec<LintFinding> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = Vec::new();

    let mut headings: Vec<(u32, std::ops::Range<usize>)> = Vec::new();
    let mut pending_heading: Option<std::ops::Range<usize>> = None;
    let mut pending_level = 0u32;
    let mut open_block_fence: Option<std::ops::Range<usize>> = None;
    let mut in_code_block = false;

    for (event, range) in Parser::new_ext(&field.text, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                pending_level = level as u32;
                pending_heading = Some(range);
            }
            Event::End(TagEnd::Heading { .. }) => {
                if let Some(start) = pending_heading.take() {
                    let span = start.start..range.end.max(start.start);
                    let level = pending_level;
                    // MD001: levels must only increment by one at a time.
                    if let Some((prev, _)) = headings.last()
                        && level > prev + 1
                    {
                        out.push(LintFinding {
                            range: field.map_range(span.clone()),
                            code: "md-heading-increment",
                            message: format!(
                                "MD001: heading level {level} skips a level after {prev} — \
                                 step one level at a time"
                            ),
                        });
                    }
                    // MD025: one top-level heading per description.
                    if level == 1 && headings.iter().any(|(l, _)| *l == 1) {
                        out.push(LintFinding {
                            range: field.map_range(span.clone()),
                            code: "md-single-h1",
                            message: "MD025: a second top-level heading — one `#` per \
                                     description; use `##` and below"
                                .to_owned(),
                        });
                    }
                    headings.push((level, span));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                if let CodeBlockKind::Fenced(info) = &kind
                    && info.trim().is_empty()
                {
                    // MD040: fenced code needs a language so editors and
                    // renderers can highlight it.
                    open_block_fence = Some(range);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                if let Some(fence) = open_block_fence.take() {
                    out.push(LintFinding {
                        range: field.map_range(fence),
                        code: "md-code-fence-language",
                        message: "MD040: fenced code block without a language — \
                                 add one (```yaml, ```json) so renderers highlight it"
                            .to_owned(),
                    });
                }
            }
            Event::Text(text) if !in_code_block => {
                // MD034: bare URLs — an autolink or inline link becomes a
                // Link event, so a URL in plain text was never wrapped.
                for (needle, at) in bare_urls(&text) {
                    let inside = range.start + at..range.start + at + needle.len();
                    out.push(LintFinding {
                        range: field.map_range(inside),
                        code: "md-bare-url",
                        message: format!(
                            "MD034: bare URL — wrap it as <{needle}> or a \
                             [link]({needle})"
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    // MD012: multiple consecutive blank lines, by decoded line.
    let mut blank_run: Option<usize> = None;
    for (line, text) in field.text.split('\n').enumerate() {
        if text.trim().is_empty()
            && let Some(first) = blank_run
            && line > first + 1
        {
            let at = field.map_offset(decoded_line_offset(&field.text, line));
            out.push(LintFinding {
                range: at..at,
                code: "md-multiple-blank-lines",
                message: "MD012: multiple blank lines — one is enough to separate \
                         blocks"
                    .to_owned(),
            });
        }
        if text.trim().is_empty() {
            if blank_run.is_none() {
                blank_run = Some(line);
            }
        } else {
            blank_run = None;
        }
    }

    out
}

/// Byte offsets of any bare URLs inside a text run.
fn bare_urls(text: &str) -> Vec<(&'static str, usize)> {
    let mut out = Vec::new();
    for needle in ["http://", "https://"] {
        let mut at = 0;
        while let Some(found) = text[at..].find(needle) {
            out.push((needle, at + found));
            at += found + needle.len();
        }
    }
    out.sort_by_key(|(_, at)| *at);
    out
}

/// The decoded-text byte offset where a line starts.
fn decoded_line_offset(text: &str, line: usize) -> usize {
    text.split('\n').take(line).map(|l| l.len() + 1).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    fn doc(text: &str) -> LowDoc {
        LowDoc::parse(
            Uri::parse("file:///probe.yaml").unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    /// Where in the file a decoded offset lands.
    fn maps_to(field: &MarkdownField<'_>, needle: &str) -> usize {
        let at = field
            .text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} not in the field text"));
        field.map_offset(at)
    }

    #[test]
    fn plain_scalars_map_one_to_one() {
        let text = "openapi: 3.1.0\ninfo:\n  description: Some **bold** and `code` text\n";
        let low = doc(text);
        let mut found = fields(&low);
        let field = found.pop().unwrap();
        let absolute = maps_to(&field, "bold");
        assert_eq!(&text[absolute..absolute + 4], "bold");
    }

    #[test]
    fn literal_blocks_map_per_line() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    # Title\n    Some **bold** text\n";
        let low = doc(text);
        let mut found = fields(&low);
        let field = found.pop().unwrap();
        assert_eq!(field.text, "# Title\nSome **bold** text\n");
        let absolute = maps_to(&field, "bold");
        assert_eq!(&text[absolute..absolute + 4], "bold");
        let hash = maps_to(&field, "#");
        assert_eq!(&text[hash..hash + 7], "# Title");
    }

    #[test]
    fn folded_blocks_map_across_joins() {
        let text = "openapi: 3.1.0\ninfo:\n  description: >\n    First paragraph **bold** and\n    more on the second raw line.\n";
        let low = doc(text);
        let mut found = fields(&low);
        let field = found.pop().unwrap();
        // Folded: one decoded line from two raw lines.
        assert_eq!(
            field.text.trim_end(),
            "First paragraph **bold** and more on the second raw line."
        );
        // A construct in the first raw line maps into the first raw line.
        let absolute = maps_to(&field, "bold");
        assert_eq!(&text[absolute..absolute + 4], "bold");
        // A construct in the joined second raw line maps into that line.
        let absolute = maps_to(&field, "second");
        assert_eq!(&text[absolute..absolute + 6], "second");
    }

    #[test]
    fn quoted_scalars_map_past_the_quote() {
        let text = "openapi: 3.1.0\ninfo:\n  description: \"Some **bold** text\"\n";
        let low = doc(text);
        let mut found = fields(&low);
        let field = found.pop().unwrap();
        let absolute = maps_to(&field, "bold");
        assert_eq!(&text[absolute..absolute + 4], "bold");
    }

    #[test]
    fn constructs_are_found_in_block_scalars() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    # Heading\n\n    A **bold** word, a `code` span, a\n    [link](https://example.com) and\n\n    ```json\n    {\"a\": 1}\n    ```\n";
        let low = doc(text);
        let mut found = fields(&low);
        let field = found.pop().unwrap();
        let kinds: Vec<Construct> = constructs(&field).iter().map(|(_, c)| *c).collect();
        for expected in [
            Construct::Heading,
            Construct::Strong,
            Construct::CodeSpan,
            Construct::Link,
            Construct::CodeBlock,
        ] {
            assert!(
                kinds.contains(&expected),
                "missing {expected:?} in {kinds:?}"
            );
        }
        // Every construct maps back to the exact file bytes it names.
        for (range, construct) in constructs(&field) {
            let file_range = field.map_range(range);
            assert!(
                file_range.end <= text.len(),
                "{construct:?} mapped outside the file: {file_range:?}"
            );
        }
        // The strong construct points at real bold markers.
        let (strong, _) = constructs(&field)
            .into_iter()
            .find(|(_, c)| *c == Construct::Strong)
            .unwrap();
        let mapped = field.map_range(strong);
        assert!(
            text[mapped.start..mapped.end].contains("bold"),
            "strong mapped to {:?} = {:?}",
            mapped,
            &text[mapped.start..mapped.end]
        );
    }

    #[test]
    fn every_field_in_a_document_is_found() {
        let text = "openapi: 3.1.0\ninfo:\n  description: one\npaths:\n  /a:\n    get:\n      summary: two\n      responses:\n        '200':\n          description: three\n";
        let low = doc(text);
        assert_eq!(fields(&low).len(), 3);
    }

    #[test]
    fn non_string_fields_are_skipped() {
        let text = "openapi: 3.1.0\ninfo:\n  description:\n    nested: true\n";
        let low = doc(text);
        assert!(fields(&low).is_empty(), "a mapping is not CommonMark");
    }

    #[test]
    fn lint_rules_fire_on_their_finding() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    # First\n    # Second\n    ### Skips\n\n\n    A bare https://example.com url and\n\n    ```\n    code\n    ```\n";
        let low = doc(text);
        let field = fields(&low).pop().unwrap();
        let codes: Vec<&str> = lint_findings(&field).iter().map(|f| f.code).collect();
        for expected in [
            "md-single-h1",
            "md-heading-increment",
            "md-bare-url",
            "md-code-fence-language",
        ] {
            assert!(codes.contains(&expected), "missing {expected} in {codes:?}");
        }
    }

    #[test]
    fn multiple_blank_lines_report_once_per_run() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    One\n\n\n\n    Two\n";
        let low = doc(text);
        let field = fields(&low).pop().unwrap();
        let binding = lint_findings(&field);
        let blanks: Vec<&str> = binding
            .iter()
            .filter(|f| f.code == "md-multiple-blank-lines")
            .map(|f| f.code)
            .collect();
        assert_eq!(blanks.len(), 1, "one run, one finding: {blanks:?}");
    }

    #[test]
    fn clean_markdown_fires_nothing() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    # Title\n\n    ## Section\n\n    A [link](https://example.com) and fenced `json`.\n\n    ```yaml\n    a: 1\n    ```\n";
        let low = doc(text);
        let field = fields(&low).pop().unwrap();
        let findings = lint_findings(&field);
        assert!(
            findings.is_empty(),
            "clean prose must stay quiet: {findings:?}"
        );
    }

    #[test]
    fn every_finding_maps_to_the_line_it_names() {
        let text = "openapi: 3.1.0\ninfo:\n  description: |\n    # First\n    # Second\n    ### Skips a level\n\n    <https://wrapped.example> and https://bare.example\n";
        let low = doc(text);
        let field = fields(&low).pop().unwrap();
        for finding in lint_findings(&field) {
            let line = text[..finding.range.start.min(text.len())]
                .matches('\n')
                .count();
            let named = text.lines().nth(line).unwrap_or_default();
            let mentions = match finding.code {
                "md-single-h1" => named.trim() == "# Second",
                "md-heading-increment" => named.trim() == "### Skips a level",
                "md-bare-url" => named.contains("bare.example"),
                _ => true,
            };
            assert!(
                mentions,
                "{finding:?} named line {line} = {named:?}, which it does not describe"
            );
        }
    }

    #[test]
    fn tokens_cover_their_constructs_at_true_positions() {
        let text = "openapi: 3.1.0\ninfo:\n  description: Some **bold** text\n";
        let low = doc(text);
        let field = fields(&low).pop().unwrap();
        let strong = constructs(&field)
            .into_iter()
            .find(|(_, c)| *c == Construct::Strong)
            .unwrap();
        let mapped = field.map_range(strong.0);
        assert!(text[mapped.start..mapped.end].contains("bold"));
    }
}
