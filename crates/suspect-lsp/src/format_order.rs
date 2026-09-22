//! Comment-preserving canonical formatting: key reordering, indentation
//! normalization, and quote discipline, all as minimal text edits over the
//! CST — never a re-serialization. Comments, anchors, block scalars, and
//! quoting styles survive by construction.
//!
//! Reordering model: each mapping's children split into blocks (leading
//! comments + pair + trailing comments); blocks sort by the context's key
//! table ([`crate::keys`]), with registered extensions slotted at their
//! anchors and unknown keys after everything, alphabetically. Same-line
//! comments are inside their pair and travel with it; a comment group
//! attaches forward to the next pair unless a blank line separates them,
//! in which case it trails the previous pair — so no comment is ever
//! dropped or orphaned.
//!
//! `example`/`examples` subtrees are preserved verbatim: their payloads are
//! arbitrary data, and author order there is presentation. Flow-style
//! mappings are never reordered. Registry maps (component sections,
//! `properties`, `webhooks`, `paths` name maps excepted) sort
//! alphabetically like every unknown-key map; `paths` sorts by template
//! specificity then alphabetically, and response-code maps sort numerically.

use std::ops::Range;

use suspect_syntax::{SNode, ScalarStyle, SyntaxKind};
use tower_lsp::lsp_types::TextEdit;

use crate::keys::{self, Context};

/// Maximum reorder passes (one per nesting depth) before giving up; real
/// documents settle in a handful of layers.
const MAX_LAYERS: usize = 64;

/// One canonical-formatting pass over the whole document: reorder (when
/// enabled), then normalize indentation, then apply quote discipline. Each
/// step re-parses the intermediate text; all three are comment-preserving.
#[must_use]
pub fn canonical_format(
    text: &str,
    sort_keys: bool,
    extensions: &crate::extensions_config::ExtensionConfig,
) -> String {
    let mut text = text.to_owned();
    if sort_keys {
        let reordered = reorder_layers(&text, extensions);
        text = reordered;
    }
    let text = normalize_indentation(&text);
    apply_quote_discipline(&text)
}

// ---------------------------------------------------------------------------
// Reordering
// ---------------------------------------------------------------------------

/// One layer's reorder edits: disjoint `(byte range, replacement)` pairs.
type LayerEdits = Vec<(Range<usize>, String)>;

/// Applies reordering layer by layer, innermost mappings first: mappings at
/// one layer are disjoint siblings, so each layer is one batch of
/// non-overlapping edits applied before re-parsing for the next.
fn reorder_layers(text: &str, extensions: &crate::extensions_config::ExtensionConfig) -> String {
    let mut text = text.to_owned();
    for _ in 0..MAX_LAYERS {
        let Some(edits) = deepest_reorder_layer(&text, extensions) else {
            break;
        };
        if edits.is_empty() {
            break;
        }
        let byte_edits: Vec<(usize, usize, String)> = edits
            .into_iter()
            .map(|(range, new_text)| (range.start, range.end, new_text))
            .collect();
        text = apply_edits(&text, &byte_edits);
    }
    text
}

/// Collects the reorder edits for the deepest mapping layer that still
/// needs one. Each mapping's layer is its pointer-token count, which
/// strictly increases down the tree, so a deeper layer never overlaps a
/// shallower one.
fn deepest_reorder_layer(
    text: &str,
    extensions: &crate::extensions_config::ExtensionConfig,
) -> Option<Vec<(Range<usize>, String)>> {
    let uri = suspect_source::Uri::parse("mem://format-order.yaml").ok()?;
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    if !low.syntax_errors().is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut best: Option<(usize, LayerEdits)> = None;
    let empty: Vec<String> = Vec::new();
    collect_layer(
        text,
        bytes,
        low.inner().root(),
        &empty,
        false,
        extensions,
        &mut best,
    );
    best.map(|(_, edits)| edits)
}

/// DFS collecting reorder edits into `best`, keeping only the deepest
/// layer's edits. `preserved` carries the example-subtree state downward.
fn collect_layer(
    text: &str,
    bytes: &[u8],
    node: SNode<'_>,
    tokens: &[String],
    preserved: bool,
    extensions: &crate::extensions_config::ExtensionConfig,
    best: &mut Option<(usize, LayerEdits)>,
) {
    match node.kind() {
        SyntaxKind::Stream | SyntaxKind::Document => {
            if let Some(child) = node.first_meaningful_child() {
                collect_layer(text, bytes, child, tokens, preserved, extensions, best);
            }
            return;
        }
        SyntaxKind::Sequence => {
            for (idx, item) in node.sequence_items().into_iter().enumerate() {
                let mut child_tokens = tokens.to_vec();
                child_tokens.push(idx.to_string());
                collect_layer(
                    text,
                    bytes,
                    item,
                    &child_tokens,
                    preserved,
                    extensions,
                    best,
                );
            }
            return;
        }
        _ if matches!(
            node.raw_kind(),
            "block_node" | "flow_node" | "_value" | "block_sequence_item"
        ) =>
        {
            if let Some(child) = node.first_meaningful_child() {
                collect_layer(text, bytes, child, tokens, preserved, extensions, best);
            }
            return;
        }
        SyntaxKind::Mapping => {}
        _ => return,
    }

    let layer = tokens.len();

    // Recurse into child mappings first (deeper layers win).
    for (key, value) in node.mapping_entries() {
        let Some(value) = value else {
            continue;
        };
        let key_text = String::from_utf8_lossy(key.scalar_bytes()).into_owned();
        let child_preserved = preserved || key_text == "example" || key_text == "examples";
        let mut child_tokens = tokens.to_vec();
        child_tokens.push(key_text);
        collect_layer(
            text,
            bytes,
            value,
            &child_tokens,
            child_preserved,
            extensions,
            best,
        );
    }

    if preserved {
        return; // example subtrees keep author order at every level
    }
    // Cheap pre-check: when the keys are already in canonical relative
    // order, skip block building entirely. After the first pass over a
    // document this short-circuits nearly every mapping.
    let context = detect_context(tokens);
    let table = keys::table_for(context);
    let special = special_sort(tokens);
    if keys_already_canonical(&node, table, context, special, extensions) {
        return;
    }
    let Some((start, end, new_text)) = reorder_mapping(text, node, tokens, bytes, extensions)
    else {
        return;
    };
    let region = start..end;
    match best {
        Some((depth, edits)) if *depth > layer => {} // a deeper layer exists
        Some((depth, edits)) if *depth == layer => {
            let overlaps = edits
                .iter()
                .any(|(r, _)| r.start < region.end && region.start < r.end);
            if !overlaps {
                edits.push((region, new_text));
            }
        }
        _ => {
            *best = Some((layer, vec![(region, new_text)]));
        }
    }
}

/// Computes the canonical reorder for one mapping, or `None` when the
/// mapping must not be touched (too small, flow style, unexpected
/// structure, or already canonical).
fn reorder_mapping(
    text: &str,
    mapping: SNode<'_>,
    tokens: &[String],
    bytes: &[u8],
    extensions: &crate::extensions_config::ExtensionConfig,
) -> Option<(usize, usize, String)> {
    let all: Vec<SNode<'_>> = mapping.children().collect();
    let meaningful: Vec<SNode<'_>> = all
        .iter()
        .copied()
        .filter(|c| matches!(c.kind(), SyntaxKind::Pair | SyntaxKind::Comment))
        .collect();
    // Abort on anything unexpected interleaved (anchors, errors, tags):
    // reordering around unknown structure is not worth the risk.
    if meaningful.len() != all.len() {
        return None;
    }
    if meaningful
        .iter()
        .filter(|c| c.kind() == SyntaxKind::Pair)
        .count()
        < 2
    {
        return None;
    }
    if mapping.raw_kind().contains("flow") {
        return None;
    }
    let blocks = build_blocks(text, &meaningful, bytes)?;
    // A mapping that is a sequence item carries the `- ` marker on its
    // first line; sorting would put a non-marker line first and break the
    // sequence. Item objects keep author order.
    let region_first = blocks.iter().map(|b| b.start).min()?;
    let line_text = text
        .get(region_first..)
        .unwrap_or("")
        .split('\n')
        .next()
        .unwrap_or("");
    if line_text.trim_start().starts_with("- ") || line_text.trim_start() == "-" {
        return None;
    }

    let context = detect_context(tokens);
    let table = keys::table_for(context);
    let special = special_sort(tokens);
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by(|&a, &b| {
        let (ka, kb) = (blocks[a].key.as_str(), blocks[b].key.as_str());
        match special {
            SpecialSort::Path => compare_paths(ka, kb),
            SpecialSort::Numeric => compare_response_codes(ka, kb),
            SpecialSort::Standard => compare_standard(ka, kb, table, context, extensions),
        }
    });
    if order.iter().enumerate().all(|(i, &b)| i == b) {
        return None; // already canonical
    }
    let region_start = blocks.iter().map(|b| b.start).min()?;
    let region_end = blocks.iter().map(|b| b.end).max()?;
    let eol = if text.get(region_start..region_end)?.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let new_text = order
        .iter()
        .map(|&i| text.get(blocks[i].start..blocks[i].end).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(eol);
    Some((region_start, region_end, new_text))
}

/// True when the mapping's keys are already in canonical relative order —
/// rank non-decreasing across pairs (equal ranks keep document order).
fn keys_already_canonical(
    mapping: &SNode<'_>,
    table: &[&str],
    context: Context,
    special: SpecialSort,
    extensions: &crate::extensions_config::ExtensionConfig,
) -> bool {
    let entries = mapping.mapping_entries();
    let mut last_rank: Option<f64> = None;
    let mut last_key: Option<String> = None;
    for (key, value) in entries {
        let Some(value) = value else {
            return false; // null-valued pairs: full check
        };
        // Only compare direct pair keys; a non-scalar sibling structure is
        // still a pair — keys are what we rank.
        let _ = value;
        let key_text = String::from_utf8_lossy(key.scalar_bytes()).into_owned();
        let rank: Option<f64> = match special {
            SpecialSort::Standard => table
                .iter()
                .position(|k| *k == key_text)
                .map(|i| i as f64)
                .or_else(|| extensions.rank(&key_text, context, table)),
            // Special sorts need the full comparator; be conservative.
            _ => return false,
        };
        let this = rank.unwrap_or(
            f64::from(u16::MAX) + key_text.chars().map(|c| c as u32 as f64).sum::<f64>() / 1.0e9,
        );
        if let Some(prev) = last_rank {
            if this < prev - f64::EPSILON {
                return false;
            }
            if (this - prev).abs() < f64::EPSILON {
                // Equal ranks keep document order; unknown keys tie-break
                // alphabetically.
                let prev_key = last_key.clone().unwrap_or_default();
                if prev_key > key_text {
                    return false;
                }
            }
        }
        last_rank = Some(this);
        last_key = Some(key_text);
    }
    true
}

/// How a mapping's keys compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecialSort {
    /// Path keys: fewer `{var}` templates first, then alphabetical.
    Path,
    /// Response codes: numeric ascending, then the rest alphabetically.
    Numeric,
    /// Key-table order with extension anchors.
    Standard,
}

fn special_sort(tokens: &[String]) -> SpecialSort {
    if tokens.len() == 1 && (tokens[0] == "paths" || tokens[0] == "definitions") {
        return SpecialSort::Path;
    }
    if tokens.len() >= 2 && tokens[tokens.len() - 1] == "responses" {
        return SpecialSort::Numeric;
    }
    SpecialSort::Standard
}

/// Builds the reorderable blocks of one mapping. Comment groups attach
/// forward to the next pair; when a blank line separates the group from
/// that pair, the group attaches backward to the previous pair instead
/// (a leading group with no previous pair simply stays first).
fn build_blocks(text: &str, children: &[SNode<'_>], bytes: &[u8]) -> Option<Vec<Block>> {
    let _ = text;
    let mut blocks: Vec<Block> = Vec::new();
    let mut pending: Vec<(usize, usize)> = Vec::new();
    for child in children {
        let (start, end) = line_extended(bytes, child.byte_range());
        if child.kind() == SyntaxKind::Comment {
            pending.push((start, end));
            continue;
        }
        if let Some(&(last_end, _)) = pending.last()
            && separated_by_blank(bytes, last_end, start)
        {
            attach_trailing(&mut blocks, &mut pending);
        }
        let block_start = pending.first().map_or(start, |&(s, _)| s);
        blocks.push(Block {
            start: block_start,
            end,
            key: pair_key_text(*child)?,
        });
        pending.clear();
    }
    attach_trailing(&mut blocks, &mut pending);
    if blocks.iter().any(|b| b.end < b.start) {
        return None;
    }
    for pair in blocks.windows(2) {
        if pair[0].end > pair[1].start {
            return None;
        }
    }
    Some(blocks)
}

/// One reorderable block: a pair plus every comment attached to it.
struct Block {
    /// Line-start-extended start of the first attached node.
    start: usize,
    /// Line-end-extended end of the last attached node (no newline).
    end: usize,
    /// The pair's key text.
    key: String,
}

/// Attaches pending comment ranges to the last block, extending its start.
fn attach_trailing(blocks: &mut [Block], pending: &mut Vec<(usize, usize)>) {
    if let (Some(last), Some(&(s, _))) = (blocks.last_mut(), pending.first()) {
        last.start = last.start.min(s);
    }
    pending.clear();
}

/// Extracts a pair's key as text.
fn pair_key_text(pair: SNode<'_>) -> Option<String> {
    let key = pair.child_by_field("key")?;
    Some(String::from_utf8_lossy(key.content().scalar_bytes()).into_owned())
}

/// Extends a node range to whole lines: start of the first line, end of
/// the last line's content (no newline).
fn line_extended(bytes: &[u8], range: Range<usize>) -> (usize, usize) {
    let start = bytes[..range.start.min(bytes.len())]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    // A node range may include its own trailing newline (block scalars);
    // search from the last content byte so the line end never overshoots
    // into the following line.
    let last_content = range.end.min(bytes.len());
    let last_content = if last_content > 0 && bytes.get(last_content - 1) == Some(&b'\n') {
        last_content - 1
    } else {
        last_content
    };
    let end = bytes[last_content..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| last_content + p);
    (start.min(end), end)
}

/// True when the gap between two ranges contains a blank line.
fn separated_by_blank(bytes: &[u8], a_end: usize, b_start: usize) -> bool {
    if a_end >= b_start || b_start > bytes.len() {
        return false;
    }
    let gap = &bytes[a_end..b_start];
    let lines: Vec<&[u8]> = gap.split(|&b| b == b'\n').collect();
    lines
        .iter()
        .skip(1)
        .take(lines.len().saturating_sub(2))
        .any(|l| l.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r'))
}

// ---------------------------------------------------------------------------
// Context detection and comparators
// ---------------------------------------------------------------------------

/// Detects the object context from the mapping's pointer tokens. Registry
/// maps (component sections, `properties`, `webhooks`, `$defs`, …) come
/// back as [`Context::Unknown`] — an empty table, i.e. alphabetical.
#[must_use]
pub fn detect_context(tokens: &[String]) -> Context {
    use keys::Context as C;
    match tokens.len() {
        0 => return C::Root,
        1 if tokens[0] == "info" => return C::Info,
        2 if tokens[0] == "info" => {
            return match tokens[1].as_str() {
                "contact" => C::Contact,
                "license" => C::License,
                _ => C::Unknown,
            };
        }
        3 if tokens[0] == "paths" && is_method(&tokens[2]) => return C::Operation,
        2 if tokens[0] == "paths" => return C::PathItem,
        3 if tokens[0] == "components" => {
            return match tokens[1].as_str() {
                "schemas" => C::Schema,
                "parameters" => C::Parameter,
                "responses" => C::Response,
                "requestBodies" => C::RequestBody,
                "headers" => C::Header,
                "examples" => C::Example,
                "links" => C::Link,
                "callbacks" => C::Callback,
                "securitySchemes" => C::SecurityScheme,
                "pathItems" => C::PathItem,
                _ => C::Unknown,
            };
        }
        _ => {}
    }
    // Positional checks relative to the end of the token list.
    let n = tokens.len();
    if n < 2 {
        return C::Unknown;
    }
    let parent = tokens[n - 2].as_str();
    let owner = tokens[n - 1].as_str();
    match parent {
        "responses" => return C::Response,
        "parameters" => return C::Parameter,
        "headers" => return C::Header,
        "links" => return C::Link,
        "callbacks" => return C::Callback,
        "content" => return C::MediaType,
        "encoding" => return C::Encoding,
        "servers" => return C::Server,
        "variables" => return C::ServerVariable,
        "flows" => return C::OAuthFlow,
        "discriminator" => return C::Discriminator,
        "xml" => return C::Xml,
        "externalDocs" => return C::ExternalDocs,
        "requestBody" => return C::RequestBody,
        "allOf"
        | "anyOf"
        | "oneOf"
        | "not"
        | "items"
        | "additionalProperties"
        | "contains"
        | "unevaluatedItems"
        | "propertyNames"
        | "if"
        | "then"
        | "else"
        | "prefixItems" => return C::Schema,
        "tags" => return C::Tag,
        _ => {}
    }
    if parent == "webhooks" && is_method(owner) {
        return C::Operation;
    }
    if parent == "webhooks" {
        return C::PathItem;
    }
    C::Unknown
}

fn is_method(token: &str) -> bool {
    keys::HTTP_METHODS.contains(&token)
}

/// Standard comparator: table index, extension anchors slot at `±0.5`,
/// unknown keys after the table, alphabetically. Equal ranks keep order.
fn compare_standard(
    a: &str,
    b: &str,
    table: &[&str],
    context: Context,
    extensions: &crate::extensions_config::ExtensionConfig,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |key: &str| -> Option<f64> {
        table
            .iter()
            .position(|k| *k == key)
            .map(|i| i as f64)
            .or_else(|| extensions.rank(key, context, table))
    };
    match (rank(a), rank(b)) {
        (Some(ra), Some(rb)) => ra.partial_cmp(&rb).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// Path keys: fewer `{var}` templates first, then alphabetical.
fn compare_paths(a: &str, b: &str) -> std::cmp::Ordering {
    a.matches('{')
        .count()
        .cmp(&b.matches('{').count())
        .then_with(|| a.cmp(b))
}

/// Response codes: numeric ascending, then the rest alphabetically
/// (`default` and ranges land after the numbers).
fn compare_response_codes(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.parse::<u16>(), b.parse::<u16>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

// ---------------------------------------------------------------------------
// Indentation normalization
// ---------------------------------------------------------------------------

/// Normalizes block-mapping/sequence indentation to two spaces per depth,
/// skipping block-scalar interiors and flow collections. Comments align to
/// their containing mapping's column.
#[must_use]
fn normalize_indentation(text: &str) -> String {
    let Ok(uri) = suspect_source::Uri::parse("mem://format-indent.yaml") else {
        return text.to_owned();
    };
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    if !low.syntax_errors().is_empty() {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .collect();
    let mut expected: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut skip: Vec<Range<usize>> = Vec::new();
    collect_layout(
        bytes,
        &line_starts,
        low.inner().root(),
        0,
        &mut expected,
        &mut skip,
        false,
    );

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut line_start = 0usize;
    for (idx, line) in text.lines().enumerate() {
        let content = line.trim_start_matches([' ', '\t']);
        if !content.is_empty()
            && let Some(&want) = expected.get(&idx)
        {
            let col = line.len() - content.len();
            let offset = line_start + col;
            let inside_skip = skip.iter().any(|r| r.contains(&offset));
            if want != col && !inside_skip {
                edits.push((line_start, line_start + col, " ".repeat(want)));
            }
        }
        line_start += line.len() + 1;
    }
    if edits.is_empty() {
        return text.to_owned();
    }
    apply_edits(text, &edits)
}

/// Collects expected indentation columns per line and skip ranges (block
/// scalars, flow collections). Invariant: `depth` is the column multiplier
/// of the pairs inside the mapping currently visited — a mapping at depth
/// `d` has its pair keys at column `d * 2`; a sequence under a depth-`d`
/// pair renders items at column `(d + 1) * 2`; comments sit at their
/// mapping's column.
#[allow(clippy::too_many_arguments)]
fn collect_layout(
    bytes: &[u8],
    line_starts: &[usize],
    node: SNode<'_>,
    depth: usize,
    expected: &mut std::collections::BTreeMap<usize, usize>,
    skip: &mut Vec<Range<usize>>,
    in_skip: bool,
) {
    let is_block_scalar = node.scalar_style() == ScalarStyle::Block;
    if is_block_scalar || node.raw_kind().contains("flow") {
        skip.push(node.byte_range());
    }
    // `child_skip` covers this node itself: pairs and comments INSIDE a
    // flow mapping (which shares its host line) must not be recorded.
    let child_skip = in_skip || is_block_scalar || node.raw_kind().contains("flow");
    match node.kind() {
        SyntaxKind::Mapping => {
            for child in node.children() {
                match child.kind() {
                    SyntaxKind::Comment => {
                        if !child_skip {
                            expected
                                .entry(line_of(line_starts, child.start_byte()))
                                .or_insert(depth * 2);
                        }
                    }
                    SyntaxKind::Pair => {
                        if let Some(key) = child.child_by_field("key") {
                            if !child_skip {
                                expected
                                    .entry(line_of(line_starts, key.byte_range().start))
                                    .or_insert(depth * 2);
                            }
                            if let Some(value) = child.child_by_field("value") {
                                collect_value_layout(
                                    bytes,
                                    line_starts,
                                    value.content(),
                                    depth,
                                    expected,
                                    skip,
                                    child_skip,
                                );
                            }
                        }
                    }
                    other => {
                        let _ = other;
                        collect_layout(
                            bytes,
                            line_starts,
                            child,
                            depth,
                            expected,
                            skip,
                            child_skip,
                        );
                    }
                }
            }
        }
        SyntaxKind::Sequence => {
            // The `-` marker renders at one level below the sequence's
            // parent pair; item content starts two columns right of the
            // marker, and an item mapping's keys one further.
            for item in node.sequence_items() {
                if !child_skip {
                    expected
                        .entry(line_of(line_starts, item.start_byte()))
                        .or_insert((depth + 1) * 2);
                }
                collect_layout(
                    bytes,
                    line_starts,
                    item,
                    depth + 2,
                    expected,
                    skip,
                    child_skip,
                );
            }
        }
        _ => {
            for child in node.children() {
                collect_layout(bytes, line_starts, child, depth, expected, skip, child_skip);
            }
        }
    }
}

/// Walks a pair's value: a nested mapping shifts one level deeper; a
/// sequence shares the pair's depth (its items shift one).
fn collect_value_layout(
    bytes: &[u8],
    line_starts: &[usize],
    node: SNode<'_>,
    depth: usize,
    expected: &mut std::collections::BTreeMap<usize, usize>,
    skip: &mut Vec<Range<usize>>,
    in_skip: bool,
) {
    if node.kind() == SyntaxKind::Mapping {
        collect_layout(bytes, line_starts, node, depth + 1, expected, skip, in_skip);
    } else {
        collect_layout(bytes, line_starts, node, depth, expected, skip, in_skip);
    }
}

/// Zero-based line index of a byte offset (binary search over the
/// precomputed line-start table).
fn line_of(line_starts: &[usize], offset: usize) -> usize {
    line_starts
        .binary_search(&offset)
        .unwrap_or_else(|insert| insert - 1)
}

// ---------------------------------------------------------------------------
// Quote discipline
// ---------------------------------------------------------------------------

/// `$ref` values and bare ISO-8601 timestamps are emitted double-quoted
/// (parsers re-interpret bare timestamps as dates; refs read consistently
/// quoted).
#[must_use]
fn apply_quote_discipline(text: &str) -> String {
    let Ok(uri) = suspect_source::Uri::parse("mem://format-quotes.yaml") else {
        return text.to_owned();
    };
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    if !low.syntax_errors().is_empty() {
        return text.to_owned();
    }
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for n in low.inner().root().descendants() {
        if n.kind() != SyntaxKind::Pair {
            continue;
        }
        let Some(key) = n.child_by_field("key") else {
            continue;
        };
        let Some(value) = n.child_by_field("value") else {
            continue;
        };
        let key_text = String::from_utf8_lossy(key.content().scalar_bytes()).into_owned();
        let scalar = value.content();
        // $ref accepts plain or single-quoted sources (double-quoted is
        // already canonical); timestamps are only ever plain.
        let convertible = matches!(
            scalar.scalar_style(),
            ScalarStyle::Plain | ScalarStyle::SingleQuoted
        );
        if !convertible {
            continue;
        }
        let raw = String::from_utf8_lossy(scalar.scalar_bytes()).into_owned();
        if (key_text == "$ref" && raw.contains('#')) || is_iso_timestamp(&raw) {
            edits.push((
                scalar.byte_range().start,
                scalar.byte_range().end,
                format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\"")),
            ));
        }
    }
    if edits.is_empty() {
        return text.to_owned();
    }
    apply_edits(text, &edits)
}

/// Bare ISO-8601 calendar dates (`2024-01-01`, optionally followed by
/// `T…`/` …`), which YAML 1.1 parsers load as dates.
fn is_iso_timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() < 10 {
        return false;
    }
    let digits = |slice: &[u8]| slice.iter().all(u8::is_ascii_digit);
    digits(&b[..4])
        && b[4] == b'-'
        && digits(&b[5..7])
        && b[7] == b'-'
        && digits(&b[8..10])
        && matches!(b.get(10), None | Some(b'T') | Some(b' '))
}

// ---------------------------------------------------------------------------
// Edit application
// ---------------------------------------------------------------------------

/// Splices `(start, end, replacement)` edits into `text` in a single
/// pass; ranges must be disjoint (callers guarantee it).
fn apply_edits(text: &str, edits: &[(usize, usize, String)]) -> String {
    let mut ordered: Vec<&(usize, usize, String)> = edits.iter().collect();
    ordered.sort_by_key(|(s, _, _)| *s);
    let mut out = String::with_capacity(text.len() + 64);
    let mut cursor = 0usize;
    for (start, end, replacement) in ordered {
        let (start, end) = (*start, *end);
        if start < cursor || end < start {
            continue; // disjoint guarantee violated defensively
        }
        out.push_str(&text[cursor..start]);
        out.push_str(replacement);
        cursor = end.max(start);
    }
    out.push_str(&text[cursor..]);
    out
}

/// Converts byte edits into LSP text edits against `text`'s line index.
#[must_use]
pub fn edits_as_lsp(text: &str, edits: &[(Range<usize>, String)]) -> Vec<TextEdit> {
    let Ok(uri) = suspect_source::Uri::parse("mem://format-lsp.yaml") else {
        return Vec::new();
    };
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    let inner = low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    edits
        .iter()
        .map(|(range, new_text)| TextEdit {
            range: crate::state::lsp_range(bytes, li, range.clone()),
            new_text: new_text.clone(),
        })
        .collect()
}
