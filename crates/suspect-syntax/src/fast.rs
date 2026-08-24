//! Allocation-lean reader for the common block-style YAML subset.
//!
//! This module implements a deliberately restricted YAML reader that covers
//! the shape of virtually every real-world OpenAPI document: nested block
//! mappings and sequences driven by indentation, plain / single-quoted /
//! double-quoted scalars, block scalars (`|` / `>` with chomping and
//! indentation behaviour matching [`suspect_low`]'s CST decoder), single-line
//! flow collections (`{}` / `[]`), comments, and core-schema null / bool /
//! int / float tokens.
//!
//! Safety comes from failing closed: any construct the reader does not
//! understand — tabs, CRLF, document markers beyond line 0, directives,
//! anchors, aliases, tags, multi-line flow, multi-line plain scalars, merge
//! keys, deeply nested input — makes [`try_parse_fast`] return `None` so the
//! caller can fall back to the full CST pipeline. The reader therefore never
//! mis-parses: it either produces the same value the lossless parser would,
//! or it declines.

use rayon::prelude::*;

/// A value parsed from the block-style YAML subset.
///
/// Document order is preserved everywhere; duplicate keys are kept (callers
/// collapse them with first-match-wins lookup, mirroring the CST model).
#[derive(Debug, Clone, PartialEq)]
pub enum FastValue {
    /// Mapping (block, or single-line flow) preserving document order.
    Object(Vec<(String, FastValue)>),
    /// Sequence (block, or single-line flow).
    Array(Vec<FastValue>),
    /// Scalar leaf.
    Scalar {
        /// Unquoted decoded text for quoted strings; the literal token
        /// otherwise (type semantics are decided downstream with the same
        /// core-schema rules `suspect-low` uses).
        raw: String,
        /// `true` when written quoted (`'…'` / `"…"`) or as a block scalar —
        /// such scalars are always strings regardless of their content.
        quoted: bool,
    },
}

impl FastValue {
    /// The canonical null scalar (empty plain token).
    #[must_use]
    pub fn null() -> FastValue {
        FastValue::Scalar {
            raw: String::new(),
            quoted: false,
        }
    }

    /// First mapping entry stored under `key`.
    ///
    /// Returns `None` for non-objects, mirroring `NodeRef::get`.
    pub fn get(&self, key: &str) -> Option<&FastValue> {
        match self {
            FastValue::Object(entries) => entries
                .iter()
                .find(|(k, _)| k.as_bytes() == key.as_bytes())
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sequence items; empty for anything that is not an array (mirrors
    /// `NodeRef::items`).
    pub fn items(&self) -> &[FastValue] {
        match self {
            FastValue::Array(items) => items,
            _ => &[],
        }
    }

    /// Mapping entries; empty for anything that is not an object.
    pub fn entries(&self) -> &[(String, FastValue)] {
        match self {
            FastValue::Object(entries) => entries,
            _ => &[],
        }
    }
}

/// Files larger than this parse their top-level entries across threads.
const PAR_THRESHOLD: usize = 256 * 1024;

/// Line preparation runs single-threaded below this size.
const SCAN_PAR_THRESHOLD: usize = 1024 * 1024;

/// Mappings whose byte span exceeds this are parsed across threads, one
/// task per self-contained entry group.
const MAP_PAR_SPAN: usize = 64 * 1024;

/// Target byte size for the smallest parallel parse leaf.
const PAR_LEAF_SPAN: usize = 192 * 1024;

/// Nesting guard; deeper documents decline to the fallback.
const MAX_DEPTH: u32 = 384;

/// One prepared source line.
struct Line<'a> {
    /// Leading spaces.
    indent: u32,
    /// Whitespace-only line.
    ws_blank: bool,
    /// Comment-only or blank line: insignificant outside block scalars.
    skippable: bool,
    /// Line content after the indent (whole line when blank).
    text: &'a [u8],
    /// Full raw line including the indent — block scalars need it to
    /// preserve indentation beyond the block's own indent level.
    raw: &'a [u8],
}

/// Tries to parse `bytes` as the supported YAML subset.
///
/// Returns `None` when the input uses anything outside the subset; callers
/// must then use the full parser. Never returns a value that disagrees with
/// the full parser for accepted inputs.
#[must_use]
pub fn try_parse_fast(bytes: &[u8]) -> Option<FastValue> {
    // Cheap whole-buffer guards: tabs and CRs never occur in the subset.
    // (memchr-free scans are ~1 GB/s; two passes over even 10 MB are noise.)
    if bytes.contains(&b'\t') || bytes.contains(&b'\r') {
        return None;
    }
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    // One whole-buffer UTF-8 check up front: every slice of a valid UTF-8
    // buffer is itself valid UTF-8, so scalar decoding below can skip
    // per-token validation.
    if std::str::from_utf8(bytes).is_err() {
        return None;
    }
    let lines = scan_lines(bytes)?;
    if lines.is_empty() {
        return None;
    }

    // Group top-level entries into self-contained ranges so parallel
    // workers never share indentation context. A new range starts at every
    // significant column-0 mapping key; column-0 sequence items belong to
    // the range that opened before them (a key with a block sequence at
    // the same column), or open the first one for a sequence root.
    let mut bounds: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.skippable {
            continue;
        }
        if bounds.is_empty() {
            if line.indent != 0 {
                return None; // indented content before any root entry
            }
            bounds.push(idx);
        } else if line.indent == 0 && !is_seq_item(line.text) {
            bounds.push(idx);
        }
    }
    if bounds.is_empty() {
        return None;
    }

    // Expand oversized top-level ranges into balanced sub-ranges: a 4 MB
    // `paths:` entry parsed as one task would dominate the parallel sweep,
    // so large mappings recurse into their own entry boundaries (indent
    // +2, then +4, ...) until every leaf is under [`PAR_LEAF_SPAN`] or no
    // finer mapping boundary exists. Every leaf stays self-contained, so
    // results merge identically.
    fn expand(
        lines: &[Line<'_>],
        start: usize,
        end: usize,
        child_indent: u32,
        out: &mut Vec<std::ops::Range<usize>>,
    ) -> Option<()> {
        let span: usize = lines[start..end].iter().map(|l| l.raw.len() + 1).sum();
        if span <= PAR_LEAF_SPAN {
            out.push(start..end);
            return Some(());
        }
        // Mapping-entry starts at child_indent inside this range.
        let mut bounds: Vec<usize> = Vec::new();
        let mut i = start;
        while i < end {
            let line = &lines[i];
            if line.skippable {
                i += 1;
                continue;
            }
            if line.indent < child_indent {
                break;
            }
            if line.indent == child_indent && !is_seq_item(line.text) {
                bounds.push(i);
            }
            i += 1;
        }
        if bounds.len() < 2 || !split_key_value(lines[bounds[0]].text).is_some() {
            out.push(start..end);
            return Some(());
        }
        let mut bounded = bounds.clone();
        bounded.push(end);
        for w in bounded.windows(2) {
            let sub = (w[0], w[1]);
            // Child entries nest two columns deeper (block mappings); their
            // children sit at +2 again. Recurse with child_indent + 2.
            expand(lines, sub.0, sub.1, child_indent + 2, out)?;
        }
        Some(())
    }

    let parse_range = |range: std::ops::Range<usize>| parse_top_level(&lines[range]);

    let mut leaf_ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for (&s, e) in bounds.iter().zip(
        bounds
            .iter()
            .skip(1)
            .copied()
            .chain(std::iter::once(lines.len())),
    ) {
        expand(&lines, s, e, 2, &mut leaf_ranges)?;
    }

    let parts: Option<Vec<FastValue>> = if bytes.len() > PAR_THRESHOLD && leaf_ranges.len() > 1 {
        leaf_ranges
            .par_iter()
            .map(|r| parse_range(r.clone()))
            .collect()
    } else {
        let mut out = Vec::with_capacity(leaf_ranges.len());
        for r in &leaf_ranges {
            out.push(parse_range(r.clone())?);
        }
        Some(out)
    };
    let parts = parts?;

    // Combine in document order.
    let mut parts = parts.into_iter();
    let first = parts.next()?;
    let is_array = matches!(first, FastValue::Array(_));
    let mut root = first;
    for part in parts {
        match (&mut root, part) {
            (FastValue::Object(entries), FastValue::Object(new)) => entries.extend(new),
            (FastValue::Array(items), FastValue::Array(new)) => items.extend(new),
            _ => return None,
        }
    }
    if is_array != matches!(root, FastValue::Array(_)) {
        return None;
    }
    Some(root)
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Splits `bytes` into prepared lines, declining on document markers,
/// directives, and other stream-level features the subset does not cover.
/// Large buffers are prepared in parallel over newline-aligned chunks.
fn scan_lines(bytes: &[u8]) -> Option<Vec<Line<'_>>> {
    if bytes.len() < SCAN_PAR_THRESHOLD {
        return scan_lines_chunk(bytes, true, true);
    }
    // Newline-aligned split points, roughly four chunks per worker.
    let target = (bytes.len() / rayon::current_num_threads().max(1)).max(SCAN_PAR_THRESHOLD) / 4;
    let mut splits: Vec<usize> = vec![0];
    let mut pos = target;
    while pos < bytes.len() {
        if let Some(off) = bytes[pos..].iter().position(|&b| b == b'\n') {
            let next = pos + off + 1;
            if next < bytes.len() {
                splits.push(next);
            }
            pos = next + target;
        } else {
            break;
        }
    }
    let chunks: Vec<&[u8]> = splits
        .iter()
        .zip(
            splits
                .iter()
                .skip(1)
                .copied()
                .chain(std::iter::once(bytes.len())),
        )
        .map(|(&s, e)| &bytes[s..e])
        .collect();
    let last = chunks.len() - 1;
    let parts: Option<Vec<Vec<Line<'_>>>> = chunks
        .par_iter()
        .enumerate()
        .map(|(idx, chunk)| scan_lines_chunk(chunk, idx == 0, idx == last))
        .collect();
    let parts = parts?;
    let total: usize = parts.iter().map(Vec::len).sum();
    let mut lines = Vec::with_capacity(total);
    for part in parts {
        lines.extend(part);
    }
    Some(lines)
}

/// Prepares one newline-aligned chunk of source lines.
///
/// `at_file_start` marks the chunk containing byte 0, where a leading
/// `---` document marker is permitted. `is_last` marks the chunk holding
/// EOF, whose final line has no trailing newline of its own.
fn scan_lines_chunk(chunk: &[u8], at_file_start: bool, is_last: bool) -> Option<Vec<Line<'_>>> {
    let mut lines = Vec::with_capacity(chunk.len() / 24 + 1);
    for (lineno, raw) in chunk.split(|&b| b == b'\n').enumerate() {
        if at_file_start && lineno == 0 && raw == b"---" {
            continue; // leading document marker
        }
        if is_doc_marker(raw) {
            return None; // "---"/"..." beyond position 0, or with payload
        }
        if raw.first() == Some(&b'%') {
            return None; // %YAML/%TAG directives
        }
        let indent = raw.iter().take_while(|&&b| b == b' ').count() as u32;
        let text = &raw[indent as usize..];
        // Blank requires every byte to be a space; a single look at the
        // first content byte settles almost every line without scanning.
        let ws_blank = match text.first() {
            None => true,
            Some(&b' ') => text.iter().all(|&b| b == b' '),
            _ => false,
        };
        let skippable = ws_blank || text.first() == Some(&b'#');
        lines.push(Line {
            indent,
            ws_blank,
            skippable,
            text,
            raw,
        });
    }
    // Non-final chunks end with a completed newline; `split` then yields a
    // phantom empty last "line" that must not leak into the stream.
    if !is_last && chunk.ends_with(b"\n") {
        lines.pop();
    }
    Some(lines)
}

/// `---` / `...` alone on a line (with optional trailing spaces).
fn is_doc_marker(raw: &[u8]) -> bool {
    let t = if raw.starts_with(b"---") || raw.starts_with(b"...") {
        &raw[3..]
    } else {
        return false;
    };
    t.iter().all(|&b| b == b' ')
}

/// Advances past insignificant lines; returns `false` at end of input.
macro_rules! next_significant {
    ($lines:expr, $i:expr) => {{
        while $i < $lines.len() && $lines[$i].skippable {
            $i += 1;
        }
        $i < $lines.len()
    }};
}

/// Parses one top-level entry range (column-0 mapping entry or sequence item).
fn parse_top_level(lines: &[Line<'_>]) -> Option<FastValue> {
    let mut i = 0;
    if !next_significant!(lines, i) {
        return None;
    }
    let value = if is_seq_item(lines[i].text) {
        parse_seq(lines, &mut i, 0, 0)
    } else {
        parse_map(lines, &mut i, 0, 0)
    }?;
    // Every significant line in the range must have been consumed;
    // leftovers mean the range is not self-contained.
    if next_significant!(lines, i) {
        return None;
    }
    Some(value)
}

/// Block mapping at indent `d`.
///
/// Large mappings are split at their own entry boundaries and parsed across
/// threads; each task receives whole entries so indentation stays
/// self-contained. Entry order is preserved on join.
fn parse_map(lines: &[Line<'_>], i: &mut usize, d: u32, depth: u32) -> Option<FastValue> {
    if depth > MAX_DEPTH {
        return None;
    }

    // Structural extent: entry-start offsets of this mapping, ending at the
    // first significant line at a lower column or a sequence item.
    let mut starts: Vec<usize> = Vec::new();
    let mut span = 0usize;
    let mut j = *i;
    while j < lines.len() {
        if lines[j].skippable {
            j += 1;
            continue;
        }
        let line = &lines[j];
        if line.indent < d || (line.indent == d && is_seq_item(line.text)) {
            break;
        }
        if line.indent == d {
            if is_merge_key(line.text) {
                return None; // merge keys need anchor resolution
            }
            starts.push(j);
        }
        span += line.raw.len() + 1;
        j += 1;
    }

    if starts.len() < 2 || span <= MAP_PAR_SPAN || j - *i < 2 {
        let entries = map_entries(lines, i, d, depth)?;
        return Some(FastValue::Object(entries));
    }

    // Parallel: cut at entry boundaries, grouping so every task covers at
    // least `MAP_PAR_SPAN` bytes. The first range starts at the mapping's
    // first line so leading blank/comment lines ride along.
    let first = *i;
    *i = j;
    let mut bounds = starts;
    bounds.push(j);
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut group_start = first;
    let mut acc = 0usize;
    for window in bounds.windows(2) {
        // True byte span of the entry: every line it owns, not just its
        // key line.
        acc += lines[window[0]..window[1]]
            .iter()
            .map(|l| l.raw.len() + 1)
            .sum::<usize>();
        if acc >= MAP_PAR_SPAN {
            groups.push((group_start, window[1]));
            group_start = window[1];
            acc = 0;
        }
    }
    if group_start < j {
        groups.push((group_start, j));
    }

    let parts: Option<Vec<Vec<(String, FastValue)>>> = groups
        .par_iter()
        .map(|&(s, e)| {
            let slice = &lines[s..e];
            let mut k = 0;
            let entries = map_entries(slice, &mut k, d, depth)?;
            while k < slice.len() && slice[k].skippable {
                k += 1;
            }
            if k < slice.len() {
                return None; // unconsumed content: not self-contained
            }
            Some(entries)
        })
        .collect();
    let mut entries: Vec<(String, FastValue)> = Vec::new();
    for part in parts? {
        entries.extend(part);
    }
    Some(FastValue::Object(entries))
}

/// Sequential mapping-entry loop; also the per-range worker body.
fn map_entries(
    lines: &[Line<'_>],
    i: &mut usize,
    d: u32,
    depth: u32,
) -> Option<Vec<(String, FastValue)>> {
    let mut entries: Vec<(String, FastValue)> = Vec::with_capacity(8);
    while *i < lines.len() {
        if lines[*i].skippable {
            *i += 1;
            continue;
        }
        let line = &lines[*i];
        if line.indent < d {
            break;
        }
        if line.indent > d {
            return None; // dangling deeper content
        }
        if is_seq_item(line.text) {
            break; // sequence belonging to an enclosing key
        }
        if is_merge_key(line.text) {
            return None;
        }
        let (key_raw, value_raw) = split_key_value(line.text)?;
        let key = decode_key(key_raw)?;
        *i += 1;
        let value = value_after_key(lines, i, value_raw, d, depth)?;
        entries.push((key, value));
    }
    Some(entries)
}

/// Block sequence at indent `d` (supports `- key: value` compact mappings).
fn parse_seq(lines: &[Line<'_>], i: &mut usize, d: u32, depth: u32) -> Option<FastValue> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut items: Vec<FastValue> = Vec::with_capacity(8);
    while *i < lines.len() {
        if lines[*i].skippable {
            *i += 1;
            continue;
        }
        let line = &lines[*i];
        if line.indent < d || !is_seq_item(line.text) {
            break;
        }
        if line.indent > d {
            return None;
        }
        let after = &line.text[1..];
        let lead = after.iter().take_while(|&&b| b == b' ').count();
        let rest = &after[lead..];
        let rest_col = d + 1 + lead as u32;
        if rest.is_empty() || rest.first() == Some(&b'#') {
            // Bare dash: value is the following deeper block (or null).
            *i += 1;
            items.push(child_block(lines, i, d, depth)?);
            continue;
        }
        if is_seq_item(rest) {
            return None; // compactly nested sequences: fall back
        }
        if let Some((key_raw, value_raw)) = split_key_value(rest) {
            // Compact mapping starting on the dash line.
            if is_merge_key(rest) {
                return None;
            }
            let key = decode_key(key_raw)?;
            *i += 1;
            let value = value_after_key(lines, i, value_raw, rest_col, depth)?;
            let mut entries = vec![(key, value)];
            while *i < lines.len() {
                if lines[*i].skippable {
                    *i += 1;
                    continue;
                }
                let cont = &lines[*i];
                if cont.indent < rest_col || is_seq_item(cont.text) {
                    break;
                }
                if cont.indent > rest_col {
                    return None;
                }
                if is_merge_key(cont.text) {
                    return None;
                }
                let (k2, v2) = split_key_value(cont.text)?;
                let key2 = decode_key(k2)?;
                *i += 1;
                entries.push((key2, value_after_key(lines, i, v2, rest_col, depth)?));
            }
            items.push(FastValue::Object(entries));
        } else {
            // Plain scalar item.
            *i += 1;
            items.push(value_after_key(lines, i, rest, d, depth)?);
        }
    }
    Some(FastValue::Array(items))
}

/// Value for a key (or sequence item) whose inline text is `inline`.
///
/// `owner_col` is the column the owning key/item starts at; nested blocks
/// must be indented deeper, except sequences which may sit at the same
/// column. Consumes every line belonging to the value or declines.
fn value_after_key(
    lines: &[Line<'_>],
    i: &mut usize,
    inline: &[u8],
    owner_col: u32,
    depth: u32,
) -> Option<FastValue> {
    let inline = strip_comment(inline);
    let inline = trim_spaces(inline);
    if inline.is_empty() {
        return child_block(lines, i, owner_col, depth);
    }
    match inline[0] {
        b'|' | b'>' => block_scalar(lines, i, owner_col, inline),
        b'[' | b'{' => {
            let mut f = FlowParser {
                b: inline,
                i: 0,
                depth: 0,
            };
            let value = f.value()?;
            if !trim_spaces(f.rest()).is_empty() {
                return None; // trailing garbage after the flow collection
            }
            Some(value)
        }
        b'&' | b'*' | b'!' | b'%' | b'@' | b'`' => {
            // Anchors, aliases, tags, and reserved indicators at value
            // position are outside the subset.
            None
        }
        _ => {
            let (raw, quoted) = decode_scalar(inline)?;
            if !quoted && (find_plain_colon(inline) || inline.last() == Some(&b':')) {
                // Plain scalars cannot contain ": " or end in ':'.
                return None;
            }
            // Multi-line plain scalars are not supported: a deeper
            // significant line after an inline scalar means continuation.
            let mut j = *i;
            if next_significant!(lines, j) && lines[j].indent > owner_col {
                return None;
            }
            Some(FastValue::Scalar { raw, quoted })
        }
    }
}

/// The block value that follows a key with empty inline text.
fn child_block(lines: &[Line<'_>], i: &mut usize, owner_col: u32, depth: u32) -> Option<FastValue> {
    if !next_significant!(lines, *i) {
        return Some(FastValue::null());
    }
    let line = &lines[*i];
    if line.indent > owner_col {
        return parse_node(lines, i, line.indent, depth);
    }
    if line.indent == owner_col && is_seq_item(line.text) {
        // Sequences may sit at the same column as their owning key.
        return parse_seq(lines, i, owner_col, depth);
    }
    Some(FastValue::null())
}

fn parse_node(lines: &[Line<'_>], i: &mut usize, d: u32, depth: u32) -> Option<FastValue> {
    if is_seq_item(lines[*i].text) {
        parse_seq(lines, i, d, depth + 1)
    } else if split_key_value(lines[*i].text).is_some() {
        parse_map(lines, i, d, depth + 1)
    } else {
        // A lone scalar occupying its own block.
        let inline = lines[*i].text;
        *i += 1;
        value_after_key(lines, i, inline, d.saturating_sub(1), depth)
    }
}

/// True when a plain scalar contains `": "` at depth zero.
fn find_plain_colon(text: &[u8]) -> bool {
    text.windows(2).any(|w| w == b": ")
}

/// `- ` item detector.
fn is_seq_item(text: &[u8]) -> bool {
    text == b"-" || text.starts_with(b"- ")
}

/// Merge-key detector (`<<`), which the subset declines.
fn is_merge_key(text: &[u8]) -> bool {
    text.starts_with(b"<<:")
}

/// Splits `key: value` at the first structural `: `/trailing `:`,
/// honouring quotes. Returns `None` when the line is not a mapping entry.
fn split_key_value(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut in_single = false;
    let mut in_double = false;
    let mut idx = 0;
    while idx < text.len() {
        let b = text[idx];
        if in_double {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                // `''` is an escaped quote; stay inside.
                if text.get(idx + 1) == Some(&b'\'') {
                    idx += 2;
                    continue;
                }
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b':' if idx + 1 == text.len() || text[idx + 1] == b' ' => {
                    return Some((&text[..idx], &text[idx + 1..]));
                }
                _ => {}
            }
        }
        idx += 1;
    }
    None
}

/// Slice-to-str for slices of an already-validated UTF-8 buffer.
///
/// Owned string for slices of an already-validated UTF-8 buffer.
///
/// # Safety
/// Caller guarantees `bytes` is a subslice of validated UTF-8 input.
unsafe fn str_owned_unchecked(bytes: &[u8]) -> String {
    // SAFETY: validity is the caller's contract, per above.
    unsafe { std::str::from_utf8_unchecked(bytes) }.to_owned()
}

/// Trims spaces from both ends.
fn trim_spaces(mut b: &[u8]) -> &[u8] {
    while b.first() == Some(&b' ') {
        b = &b[1..];
    }
    while let Some(last) = b.last() {
        if *last == b' ' {
            b = &b[..b.len() - 1];
        } else {
            break;
        }
    }
    b
}

/// Cuts a trailing ` # comment` (quote-aware). `#` at position 0 counts.
fn strip_comment(text: &[u8]) -> &[u8] {
    let mut in_single = false;
    let mut in_double = false;
    let mut idx = 0;
    while idx < text.len() {
        let b = text[idx];
        if in_double {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                if text.get(idx + 1) == Some(&b'\'') {
                    idx += 2;
                    continue;
                }
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'#' if idx == 0 || text[idx - 1] == b' ' => return text[..idx].as_ref(),
                _ => {}
            }
        }
        idx += 1;
    }
    text
}

/// Decodes a mapping key: plain tokens and quoted strings; declines on
/// indicators the subset does not model.
fn decode_key(raw: &[u8]) -> Option<String> {
    let raw = trim_spaces(raw);
    if raw.is_empty() {
        return None;
    }
    if raw[0] == b'"' || raw[0] == b'\'' {
        let (decoded, quoted) = decode_scalar(raw)?;
        debug_assert!(quoted);
        return Some(decoded);
    }
    if matches!(
        raw[0],
        b'&' | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'%'
            | b'@'
            | b'`'
            | b'?'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
    ) {
        return None;
    }
    // SAFETY: subslice of the validated input buffer.
    Some(unsafe { str_owned_unchecked(raw) })
}

/// Decodes an inline scalar token (plain, single- or double-quoted).
///
/// Returns `(text, quoted)` where quoted tokens have their outer quotes
/// stripped. Escape sequences survive literally (`\n` stays `\\n`) and
/// `''` is not collapsed — byte-compatible with `suspect-low`'s
/// `scalar_bytes`, which strips quotes without further decoding. Declines
/// when the token is malformed or has trailing junk after a closing quote.
fn decode_scalar(text: &[u8]) -> Option<(String, bool)> {
    match text[0] {
        b'"' => {
            let inner = quoted_inner(text, b'"', true)?;
            // SAFETY: subslice of the validated input buffer.
            Some((unsafe { str_owned_unchecked(inner) }, true))
        }
        b'\'' => {
            let inner = quoted_inner(text, b'\'', false)?;
            // SAFETY: subslice of the validated input buffer.
            Some((unsafe { str_owned_unchecked(inner) }, true))
        }
        // SAFETY: subslice of the validated input buffer.
        _ => Some((unsafe { str_owned_unchecked(text) }, false)),
    }
}

/// Content between matching quotes, declining on unterminated quotes or
/// trailing junk. `double` enables backslash-escape awareness.
fn quoted_inner(text: &[u8], quote: u8, double: bool) -> Option<&[u8]> {
    let mut idx = 1;
    while idx < text.len() {
        let b = text[idx];
        if double && b == b'\\' {
            idx += 2;
            continue;
        }
        if b == quote {
            if quote == b'\'' && text.get(idx + 1) == Some(&b'\'') {
                idx += 2;
                continue;
            }
            if !trim_spaces(&text[idx + 1..]).is_empty() {
                return None; // junk after closing quote
            }
            return Some(&text[1..idx]);
        }
        idx += 1;
    }
    None // unterminated (multi-line) quoted scalar
}

/// Parses a `|` / `>` block scalar starting on the key line (header in
/// `header`), consuming its content lines from `lines[*i]`.
///
/// Folding, indentation detection, and chomping mirror `suspect-low`'s CST
/// decoder byte-for-byte so both pipelines agree.
fn block_scalar(
    lines: &[Line<'_>],
    i: &mut usize,
    owner_col: u32,
    header: &[u8],
) -> Option<FastValue> {
    let folded = header[0] == b'>';
    // Header may carry an explicit indentation digit and/or chomping
    // indicator; anything else declines.
    let mut chomp = b' ';
    for &b in &header[1..] {
        match b {
            b'-' | b'+' => {
                if chomp != b' ' {
                    return None;
                }
                chomp = b;
            }
            b'0'..=b'9' => {} // explicit indent: content detection below wins
            _ => return None,
        }
    }

    // Content: every following line until a non-blank line at or above the
    // owner's column. Blank and comment-looking lines are content here.
    let start = *i;
    let mut end = start;
    while end < lines.len() {
        let line = &lines[end];
        if !line.ws_blank && line.indent <= owner_col {
            break;
        }
        end += 1;
    }
    let body = &lines[start..end];
    *i = end;

    // Content indent = leading spaces of the first non-empty line.
    let mut indent = None;
    for line in body {
        let nonspace = line.raw.iter().take_while(|&&b| b == b' ').count();
        if nonspace < line.raw.len() {
            indent = Some(nonspace);
            break;
        }
    }
    let indent = indent.unwrap_or(0);

    let mut out: Vec<u8> = Vec::new();
    let mut prev_folded_break = false;
    let mut wrote_any = false;
    for line in body {
        // Dedent by the block indent only; deeper indentation survives
        // (mirrors the CST decoder operating on raw source lines).
        let bare = line.raw;
        let dedented: &[u8] = if indent <= bare.len() {
            &bare[indent..]
        } else if bare.is_empty() {
            b""
        } else {
            bare
        };
        let is_blank = dedented.iter().all(|&b| b == b' ');
        if folded && !is_blank && wrote_any && !prev_folded_break {
            out.push(b' '); // fold: single break between non-empty lines
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
    }

    // Chomping: clip keeps exactly one trailing break, strip removes all,
    // keep preserves (byte-compatible with the CST decoder).
    match chomp {
        b'-' => {
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
        b'+' => {}
        _ => {
            while matches!(out.last(), Some(b'\n') | Some(b' ')) {
                out.pop();
            }
            out.push(b'\n');
        }
    }
    // Block content may contain arbitrary non-UTF-8 bytes; stay lossy here
    // (the CST decoder is lossy too).
    Some(FastValue::Scalar {
        raw: String::from_utf8_lossy(&out).into_owned(),
        quoted: true,
    })
}

/// Single-line flow collection parser (`{...}` / `[...]`, nestable).
struct FlowParser<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl FlowParser<'_> {
    fn rest(&self) -> &'_ [u8] {
        &self.b[self.i..]
    }

    fn value(&mut self) -> Option<FastValue> {
        if self.depth > MAX_DEPTH {
            return None;
        }
        self.skip_ws();
        match self.b.get(self.i)? {
            b'{' => self.mapping(),
            b'[' => self.sequence(),
            _ => self.scalar(),
        }
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b' ' => self.i += 1,
                b'#' if self.i == 0 || self.b[self.i - 1] == b' ' => {
                    self.i = self.b.len(); // comment runs to end of line
                }
                _ => break,
            }
        }
    }

    fn mapping(&mut self) -> Option<FastValue> {
        self.depth += 1;
        self.i += 1; // '{'
        let mut entries: Vec<(String, FastValue)> = Vec::new();
        loop {
            self.skip_ws();
            match self.b.get(self.i)? {
                b'}' => {
                    self.i += 1;
                    break;
                }
                b',' | b':' => return None, // missing key
                _ => {}
            }
            let key = self.token(true)?;
            self.skip_ws();
            if self.b.get(self.i) != Some(&b':') {
                return None;
            }
            self.i += 1;
            let value = self.value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.b.get(self.i)? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        self.depth -= 1;
        Some(FastValue::Object(entries))
    }

    fn sequence(&mut self) -> Option<FastValue> {
        self.depth += 1;
        self.i += 1; // '['
        let mut items: Vec<FastValue> = Vec::new();
        loop {
            self.skip_ws();
            match self.b.get(self.i)? {
                b']' => {
                    self.i += 1;
                    break;
                }
                b',' => return None,
                _ => {}
            }
            items.push(self.value()?);
            self.skip_ws();
            match self.b.get(self.i)? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        self.depth -= 1;
        Some(FastValue::Array(items))
    }

    /// Quoted string or plain token. `for_key` declines empty tokens.
    fn token(&mut self, for_key: bool) -> Option<String> {
        self.skip_ws();
        match self.b.get(self.i)? {
            b'"' | b'\'' => {
                let quote = self.b[self.i];
                let start = self.i + 1;
                let mut j = start;
                while j < self.b.len() {
                    if quote == b'"' && self.b[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if self.b[j] == quote {
                        if quote == b'\'' && self.b.get(j + 1) == Some(&b'\'') {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    j += 1;
                }
                if j >= self.b.len() || self.b[j] != quote {
                    return None;
                }
                // Quotes stripped only; escapes stay literal (see
                // `decode_scalar` for the rationale).
                let inner = &self.b[start..j];
                self.i = j + 1;
                // SAFETY: subslice of the validated input buffer.
                Some(unsafe { str_owned_unchecked(inner) })
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b']' | b'}' | b':')
                {
                    self.i += 1;
                }
                let token = trim_spaces(&self.b[start..self.i]);
                if token.is_empty() && for_key {
                    return None;
                }
                // SAFETY: subslice of the validated input buffer.
                Some(unsafe { str_owned_unchecked(token) })
            }
        }
    }

    /// Scalar value inside flow: quoted string, plain token, or empty (null).
    fn scalar(&mut self) -> Option<FastValue> {
        match self.b.get(self.i) {
            None => Some(FastValue::null()),
            Some(b',') | Some(b']') | Some(b'}') => Some(FastValue::null()),
            Some(b':') => None, // `{: v}` has no key
            Some(_) => {
                let raw = self.token(false)?;
                Some(FastValue::Scalar { raw, quoted: false })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(bytes: &[u8]) -> FastValue {
        try_parse_fast(bytes).expect("parses in subset")
    }

    fn none(bytes: &[u8]) {
        assert!(try_parse_fast(bytes).is_none(), "must decline: {bytes:?}");
    }

    #[test]
    fn nested_mapping_and_sequences() {
        let doc = b"\
openapi: 3.1.0
info:
  title: Pets # trailing comment
  version: \"2.1\"
servers:
  - url: https://api.example.com/v1
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets, friends]
      parameters:
        - name: limit
          in: query
          schema:
            type: integer
      responses:
        '200':
          description: ok
";
        let root = v(doc);
        let root = match &root {
            FastValue::Object(e) => e,
            _ => panic!("root object"),
        };
        assert_eq!(root[0].0, "openapi");
        assert_eq!(
            root[0].1,
            FastValue::Scalar {
                raw: "3.1.0".into(),
                quoted: false
            }
        );
        let info = root[1].1.get("title").unwrap();
        assert_eq!(
            info,
            &FastValue::Scalar {
                raw: "Pets".into(),
                quoted: false
            }
        );
        assert_eq!(
            root[1].1.get("version").unwrap(),
            &FastValue::Scalar {
                raw: "2.1".into(),
                quoted: true
            }
        );
        let get = root[3].1.get("/pets").unwrap().get("get").unwrap();
        let limit = get.get("parameters").unwrap().items()[0]
            .get("schema")
            .unwrap()
            .get("type")
            .unwrap();
        assert_eq!(
            limit,
            &FastValue::Scalar {
                raw: "integer".into(),
                quoted: false
            }
        );
        // Flow sequence + quoted key.
        assert_eq!(
            get.get("tags").unwrap(),
            &FastValue::Array(vec![
                FastValue::Scalar {
                    raw: "pets".into(),
                    quoted: false
                },
                FastValue::Scalar {
                    raw: "friends".into(),
                    quoted: false
                },
            ])
        );
        assert_eq!(
            get.get("responses")
                .unwrap()
                .get("200")
                .unwrap()
                .get("description")
                .unwrap(),
            &FastValue::Scalar {
                raw: "ok".into(),
                quoted: false
            }
        );
    }

    #[test]
    fn scalars_nulls_bools_comments() {
        let doc = b"\
a: null
b: ~
c:
d: true
e: FALSE
f: 0x1F
g: 0o17
h: -3.5e2
i: 'it''s'
j: \"tab\\there\"
k: plain # not a comment marker: #
# whole line
l: http://x/#anchor
m: []
n: {}
o: [1, two, 'three']
p: {q: 1, r: s}
";
        let root = match v(doc) {
            FastValue::Object(e) => e,
            _ => panic!(),
        };
        let get = |k: &str| {
            root.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v)
                .unwrap()
        };
        // Explicit tokens keep their raw text; typing happens downstream.
        assert_eq!(
            get("a"),
            &FastValue::Scalar {
                raw: "null".into(),
                quoted: false
            }
        );
        assert_eq!(
            get("b"),
            &FastValue::Scalar {
                raw: "~".into(),
                quoted: false
            }
        );
        assert_eq!(get("c"), &FastValue::null());
        assert_eq!(
            get("d"),
            &FastValue::Scalar {
                raw: "true".into(),
                quoted: false
            }
        );
        assert_eq!(
            get("h"),
            &FastValue::Scalar {
                raw: "-3.5e2".into(),
                quoted: false
            }
        );
        // Escapes/quote-doubling stay literal: byte-compatible with
        // suspect-low's scalar_bytes.
        assert_eq!(
            get("i"),
            &FastValue::Scalar {
                raw: "it''s".into(),
                quoted: true
            }
        );
        assert_eq!(
            get("j"),
            &FastValue::Scalar {
                raw: "tab\\there".into(),
                quoted: true
            }
        );
        assert_eq!(
            get("k"),
            &FastValue::Scalar {
                raw: "plain".into(),
                quoted: false
            }
        );
        assert_eq!(
            get("l"),
            &FastValue::Scalar {
                raw: "http://x/#anchor".into(),
                quoted: false
            }
        );
        assert_eq!(get("m"), &FastValue::Array(vec![]));
        assert_eq!(get("n"), &FastValue::Object(vec![]));
        assert_eq!(
            get("p"),
            &FastValue::Object(vec![
                (
                    "q".into(),
                    FastValue::Scalar {
                        raw: "1".into(),
                        quoted: false
                    }
                ),
                (
                    "r".into(),
                    FastValue::Scalar {
                        raw: "s".into(),
                        quoted: false
                    }
                ),
            ])
        );
    }

    #[test]
    fn block_scalars_match_reference_decoder() {
        let literal = "description: |\n  line one\n  line two\n";
        assert_eq!(
            v(literal.as_bytes()).get("description").unwrap(),
            &FastValue::Scalar {
                raw: "line one\nline two\n".into(),
                quoted: true
            }
        );
        let strip = "description: |-\n  a\n  b\n";
        assert_eq!(
            v(strip.as_bytes()).get("description").unwrap(),
            &FastValue::Scalar {
                raw: "a\nb".into(),
                quoted: true
            }
        );
        let folded = "description: >-\n  a\n  b\n\n  c\n";
        assert_eq!(
            v(folded.as_bytes()).get("description").unwrap(),
            // One embedded blank line: the fold break plus the blank break,
            // byte-compatible with suspect-low's CST decoder.
            &FastValue::Scalar {
                raw: "a b\n\nc".into(),
                quoted: true
            }
        );
        let keep = "description: |+\n  a\n\n\n";
        assert_eq!(
            v(keep.as_bytes()).get("description").unwrap(),
            &FastValue::Scalar {
                raw: "a\n\n\n".into(),
                quoted: true
            }
        );
        // Markdown indicators inside block content are literal text.
        let md = "d: >-\n  see *this* & [that]\n";
        assert_eq!(
            v(md.as_bytes()).get("d").unwrap(),
            &FastValue::Scalar {
                raw: "see *this* & [that]".into(),
                quoted: true
            }
        );
        // Blank line between blocks terminates the first one.
        let two = "a: |\n  x\n\nb: |\n  y\n";
        let root = v(two.as_bytes());
        assert_eq!(
            root.get("a").unwrap(),
            &FastValue::Scalar {
                raw: "x\n".into(),
                quoted: true
            }
        );
        assert_eq!(
            root.get("b").unwrap(),
            &FastValue::Scalar {
                raw: "y\n".into(),
                quoted: true
            }
        );
    }

    #[test]
    fn compact_sequence_mappings() {
        let doc = b"\
parameters:
- name: limit
  in: query
  required: true
- name: q
  in: query
";
        let root = v(doc);
        let items = root.get("parameters").unwrap().items();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("required").unwrap(),
            &FastValue::Scalar {
                raw: "true".into(),
                quoted: false
            }
        );
        // Indented form too.
        let doc2 = b"\
list:
  - a: 1
    b: 2
  - a: 3
";
        let list_val = v(doc2);
        let items = list_val.get("list").unwrap().items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].entries().len(), 1);
    }

    #[test]
    fn declines_unsupported_features() {
        none(b"a: &anchor 1\n");
        none(b"a: *anchor\n");
        none(b"a: !!str 1\n");
        none(b"a:\tb\n");
        none(b"a: 1\r\n");
        none(b"---\na: 1\n---\nb: 2\n");
        none(b"%YAML 1.2\n---\na: 1\n");
        none(b"a:\n  b: 1\n c: 2\n"); // ragged indent
        none(b"a: b: c\n"); // plain scalar containing ": "
        none(b"a: \"unterminated\n");
        none(b"<<: *base\n");
        none(b"a: multi\n  continued\n");
        none(b"key: value\n  deeper: 1\n");
        none(b"- - nested\n");
        none(b"a: [1,\n2]\n"); // multi-line flow
    }

    #[test]
    fn sequence_root_and_leading_marker() {
        let doc = b"---\n- 1\n- two\n";
        assert_eq!(
            v(doc),
            FastValue::Array(vec![
                FastValue::Scalar {
                    raw: "1".into(),
                    quoted: false
                },
                FastValue::Scalar {
                    raw: "two".into(),
                    quoted: false
                },
            ])
        );
    }

    #[test]
    fn parallel_split_matches_sequential() {
        // Build a document comfortably above the parallelism threshold.
        let mut doc = String::from("openapi: 3.1.0\ncomponents:\n  schemas:\n");
        for i in 0..20_000 {
            doc.push_str(&format!(
                "    Schema{i}:\n      type: object\n      x-id: {i}\n"
            ));
        }
        let bytes = doc.into_bytes();
        let fast = try_parse_fast(&bytes).expect("subset");
        let schemas = fast
            .get("components")
            .unwrap()
            .get("schemas")
            .unwrap()
            .entries();
        assert_eq!(schemas.len(), 20_000);
        assert_eq!(schemas[0].0, "Schema0");
        assert_eq!(schemas[19_999].0, "Schema19999");
        assert_eq!(
            schemas[12_345].1.get("x-id").unwrap(),
            &FastValue::Scalar {
                raw: "12345".into(),
                quoted: false
            }
        );
    }
}
