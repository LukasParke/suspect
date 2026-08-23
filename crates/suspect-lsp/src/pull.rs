//! Pull-mode features: `textDocument/diagnostic`, `workspace/diagnostic`,
//! `textDocument/semanticTokens/range`, `semanticTokens/full/delta`, and
//! `textDocument/linkedEditingRange`.
//!
//! Everything here is a pure function of its arguments — no client handle,
//! no shared state — so handlers in `lib.rs` only take a state read lock,
//! call in, and wrap the result.

use std::sync::Arc;

use suspect_low::LowDoc;
use suspect_ref::Workspace;
use suspect_source::{LineIndex, Uri};
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::{
    Diagnostic, SemanticToken, SemanticTokensDelta, SemanticTokensEdit,
    SemanticTokensFullDeltaResult,
};

use crate::state::{OpenDoc, lsp_range, offset_of_utf16};

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime multiplier.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Computes a stable result id for a full diagnostic report.
///
/// The id is the hex-encoded FNV-1a hash of the diagnostics' codes, ranges,
/// and messages (sorted first so publication order never affects it). A
/// client that pulls with `previous_result_id` equal to the returned id can
/// keep its cached report instead of re-transferring the items.
#[must_use]
pub fn diagnostics_result_id(diagnostics: &[Diagnostic]) -> String {
    let mut lines: Vec<String> = diagnostics
        .iter()
        .map(|d| {
            let code = match &d.code {
                Some(tower_lsp::lsp_types::NumberOrString::String(s)) => s.clone(),
                Some(tower_lsp::lsp_types::NumberOrString::Number(n)) => n.to_string(),
                None => String::new(),
            };
            format!(
                "{}|{},{},{},{}|{}",
                code,
                d.range.start.line,
                d.range.start.character,
                d.range.end.line,
                d.range.end.character,
                d.message
            )
        })
        .collect();
    lines.sort();
    let mut hasher = Fnv1a::new();
    for line in &lines {
        hasher.write(line.as_bytes());
        hasher.write(b"\n");
    }
    format!("{:016x}", hasher.finish())
}

/// Computes a stable result id for a full semantic-token response.
///
/// Hashes the raw delta quintuples of every token in order; two token arrays
/// collide only through astronomically unlikely FNV accidents.
#[must_use]
pub fn tokens_result_id(tokens: &[SemanticToken]) -> String {
    let mut hasher = Fnv1a::new();
    for t in tokens {
        for v in [
            t.delta_line,
            t.delta_start,
            t.length,
            t.token_type,
            t.token_modifiers_bitset,
        ] {
            hasher.write(&v.to_le_bytes());
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Pulls the full diagnostic battery for one document.
///
/// Runs exactly the push pipeline's battery (`diagnostics::compute_diagnostics`:
/// syntax recovery errors, OAS semantic validation, lint, Arazzo checks) and
/// stamps the result with a stable result id. Full-report mode: the items are
/// always returned; the caller compares `previous_result_id` with the
/// returned id to decide whether it may answer `unchanged` instead.
#[must_use]
pub fn pull_diagnostics(
    ws: &Arc<Workspace>,
    doc: &LowDoc,
    previous_result_id: Option<String>,
    cfg: &crate::config_files::SuspectConfig,
) -> (String, Vec<Diagnostic>) {
    let diagnostics = crate::diagnostics::compute_diagnostics(Some(ws), doc, cfg);
    let result_id = diagnostics_result_id(&diagnostics);
    // The previous id is consumed by the caller's unchanged check; this pure
    // function always produces the full report.
    drop(previous_result_id);
    (result_id, diagnostics)
}

/// Pulls diagnostics for every document loaded in the workspace.
///
/// Documents that only parse with tree-sitter recovery errors still appear —
/// their syntax errors come back as ordinary diagnostics from the battery —
/// so project-wide error trees cover unopened files too.
#[must_use]
pub fn workspace_pull(
    ws: &Arc<Workspace>,
    cfg: &crate::config_files::SuspectConfig,
) -> Vec<(Uri, Vec<Diagnostic>)> {
    ws.uris()
        .into_iter()
        .filter_map(|uri| {
            let low = ws.get(&uri)?.doc();
            let diagnostics = crate::diagnostics::compute_diagnostics(Some(ws), low, cfg);
            Some((uri.clone(), diagnostics))
        })
        .collect()
}

/// One decoded semantic token at an absolute position.
struct AbsToken {
    /// Zero-based line.
    line: u32,
    /// UTF-16 column on [`AbsToken::line`].
    col: u32,
    /// Token length in UTF-16 code units.
    len: u32,
    /// Legend index of the token type.
    ty: u32,
    /// Bitset of legend modifier indices.
    mods: u32,
}

/// Decodes LSP delta encoding into absolute `(line, col)` positions.
fn decode_tokens(tokens: &[SemanticToken]) -> Vec<AbsToken> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut line = 0u32;
    let mut col = 0u32;
    for t in tokens {
        line += t.delta_line;
        col = if t.delta_line == 0 {
            col + t.delta_start
        } else {
            t.delta_start
        };
        out.push(AbsToken {
            line,
            col,
            len: t.length,
            ty: t.token_type,
            mods: t.token_modifiers_bitset,
        });
    }
    out
}

/// Delta-encodes absolute tokens, with the first token's delta taken
/// relative to `seed` (the absolute `(line, col)` of the token preceding
/// the slice; origin for standalone arrays).
fn encode_from(abs: &[AbsToken], seed: (u32, u32)) -> Vec<SemanticToken> {
    abs.iter()
        .scan(seed, |prev, t| {
            let delta_line = t.line - prev.0;
            let delta_start = if delta_line == 0 {
                t.col - prev.1
            } else {
                t.col
            };
            *prev = (t.line, t.col);
            Some(SemanticToken {
                delta_line,
                delta_start,
                length: t.len,
                token_type: t.ty,
                token_modifiers_bitset: t.mods,
            })
        })
        .collect()
}

/// Delta-encodes absolute tokens per the LSP spec (origin `(0, 0)`).
fn encode_tokens(abs: &[AbsToken]) -> Vec<SemanticToken> {
    encode_from(abs, (0, 0))
}

/// Byte span covered by a token encoded at `(line, col_utf16, len_utf16)`.
///
/// Mirrors how [`crate::semantic`] produced the token: columns and lengths
/// are UTF-16 counts mapped back onto the lossless buffer.
fn token_span(
    bytes: &[u8],
    li: &LineIndex,
    line: u32,
    col_utf16: u32,
    len_utf16: u32,
) -> Option<std::ops::Range<usize>> {
    let start = offset_of_utf16(bytes, li, line, col_utf16)?;
    let text = String::from_utf8_lossy(&bytes[start..]);
    let mut units = 0u32;
    let mut end = start;
    for ch in text.chars() {
        if units >= len_utf16 {
            break;
        }
        units += u32::try_from(ch.len_utf16()).unwrap_or(0);
        end += ch.len_utf8();
    }
    Some(start..end)
}

/// Semantic tokens restricted to a byte range of the open document.
///
/// Tokens are kept when they intersect `[range.start, range.end)` and
/// re-encoded with standard origin-relative deltas (the first returned token
/// keeps its absolute position). Uses [`crate::semantic::semantic_tokens_full`]
/// as the single source of truth, so the classification always matches the
/// advertised legend exactly.
#[must_use]
pub fn semantic_tokens_range(
    doc: &OpenDoc,
    li: &LineIndex,
    range: std::ops::Range<usize>,
) -> Vec<SemanticToken> {
    if range.start >= range.end {
        return Vec::new();
    }
    let inner = doc.low.inner();
    let bytes = inner.bytes();
    let full = decode_tokens(&crate::semantic::semantic_tokens_full(doc).data);
    let kept: Vec<AbsToken> = full
        .into_iter()
        .filter(|t| {
            token_span(bytes, li, t.line, t.col, t.len)
                .is_some_and(|span| span.end > range.start && span.start < range.end)
        })
        .collect();
    encode_tokens(&kept)
}

/// Diffs two full-token arrays into an LSP `semanticTokens/full/delta`
/// response.
///
/// The lists are compared as raw delta-encoded quintuples: the common
/// encoded prefix and encoded-equal suffix stay untouched, and at most one
/// edit covers the differing middle — an insertion (`deleteCount: 0`), a
/// deletion (no `data`), or a replacement. Matching on *encoded* values is
/// what makes the single-edit splice valid: untouched tokens keep their
/// stored deltas, so the inserted `data` only has to be re-encoded relative
/// to the token surviving just before the edit (the origin when the edit
/// starts at index 0). When nothing changed the edit list is empty.
#[must_use]
pub fn semantic_tokens_delta(
    prev: &[SemanticToken],
    next_full: &[SemanticToken],
) -> SemanticTokensFullDeltaResult {
    let mut prefix = 0usize;
    while prefix < prev.len()
        && prefix < next_full.len()
        && same_encoded(&prev[prefix], &next_full[prefix])
    {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < prev.len() - prefix
        && suffix < next_full.len() - prefix
        && same_encoded(
            &prev[prev.len() - 1 - suffix],
            &next_full[next_full.len() - 1 - suffix],
        )
    {
        suffix += 1;
    }

    let delete_count = prev.len() - prefix - suffix;
    let inserted = &next_full[prefix..next_full.len() - suffix];

    let edits = if delete_count == 0 && inserted.is_empty() {
        Vec::new()
    } else {
        let data = (!inserted.is_empty()).then(|| {
            // First inserted token continues from the surviving
            // predecessor's absolute position (origin at index 0).
            let seed = if prefix > 0 {
                decode_last(prev, prefix - 1)
            } else {
                (0, 0)
            };
            encode_from(&decode_tokens(inserted), seed)
        });
        vec![SemanticTokensEdit {
            start: u32::try_from(prefix).unwrap_or(u32::MAX),
            delete_count: u32::try_from(delete_count).unwrap_or(u32::MAX),
            data,
        }]
    };

    SemanticTokensFullDeltaResult::TokensDelta(SemanticTokensDelta {
        result_id: Some(tokens_result_id(next_full)),
        edits,
    })
}

/// True when two tokens carry identical encoded delta quintuples.
fn same_encoded(a: &SemanticToken, b: &SemanticToken) -> bool {
    a.delta_line == b.delta_line
        && a.delta_start == b.delta_start
        && a.length == b.length
        && a.token_type == b.token_type
        && a.token_modifiers_bitset == b.token_modifiers_bitset
}

/// Decoded absolute `(line, col)` of the `index`-th token of an encoded
/// array.
fn decode_last(tokens: &[SemanticToken], index: usize) -> (u32, u32) {
    let abs = decode_tokens(&tokens[..=index]);
    (abs[abs.len() - 1].line, abs[abs.len() - 1].col)
}

/// Anchor identity of a mapping key: `Some(name)` when the key subtree
/// carries an `&anchor` decoration or is an `*alias`, `None` for plain keys.
fn binding_name(key: &SNode<'_>) -> Option<String> {
    for d in key.descendants() {
        match d.kind() {
            SyntaxKind::Anchor => {
                if let Some(name) = d.anchor_name() {
                    return Some(name.to_owned());
                }
            }
            SyntaxKind::Alias => {
                if let Some(name) = d.alias_name() {
                    return Some(name.to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte span a linked edit should replace inside a scalar key.
///
/// Quoted scalars shrink to their interior so the quote characters survive
/// multi-cursor editing; plain keys keep their full byte range.
fn scalar_interior(key: &SNode<'_>) -> std::ops::Range<usize> {
    let content = key.content();
    let r = content.byte_range();
    if r.end - r.start < 2 {
        return r;
    }
    let text = content.text();
    match (text.first(), text.last()) {
        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\'')) => r.start + 1..r.end - 1,
        _ => r,
    }
}

/// Ranges that should be edited together with the mapping key under the
/// cursor.
///
/// Two coupling modes, checked in order:
///
/// 1. **Anchors.** Cursor on a mapping key carrying an `&name` anchor (or on
///    any `*name` alias occurrence) returns the name interiors of the
///    anchor declaration and of every alias referencing that name anywhere
///    in the document. All returned ranges contain identical text of
///    identical length, as `linkedEditingRange` requires, so typing
///    renames the anchor and all its aliases in one multi-cursor edit.
/// 2. **Quoted duplicates.** Cursor on a quoted scalar key whose exact text
///    appears on more than one sibling key of the *same* mapping returns
///    those duplicate keys' interior spans.
///
/// Returns `None` whenever the cursor is not on a mapping key or an alias,
/// the site carries no anchor binding, and no quoted-duplicate siblings
/// exist.
#[must_use]
pub fn linked_editing_range(
    low: &LowDoc,
    offset: usize,
) -> Option<Vec<tower_lsp::lsp_types::Range>> {
    let inner = low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());

    let node = crate::navigation::node_at(low, offset)?;
    // Climb to the enclosing pair; positions between pairs hit the root and
    // fail the climb.
    let mut cur = node;
    let pair = loop {
        if cur.kind() == SyntaxKind::Pair {
            break cur;
        }
        cur = cur.parent()?;
    };
    let mut on_key = false;
    let key = pair.child_by_field("key")?;
    let kr = key.byte_range();
    if kr.start <= offset && offset <= kr.end {
        on_key = true;
    }

    // Anchor identity: the key's own `&name`, else an alias `*name` directly
    // under the cursor (aliases cannot be mapping keys in this grammar, so
    // their occurrences live on value positions).
    let mut target = if on_key { binding_name(&key) } else { None };
    if target.is_none() {
        // The cursor may sit on a marker/child token of the alias node, so
        // climb as well as descend.
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n.kind() == SyntaxKind::Alias
                && let Some(name) = n.alias_name()
            {
                target = Some(name.to_owned());
                break;
            }
            cur = n.parent();
        }
    }

    if let Some(target) = target {
        let mut ranges = Vec::new();
        for d in inner.root().descendants() {
            let name = match d.kind() {
                SyntaxKind::Anchor => d.anchor_name(),
                SyntaxKind::Alias => d.alias_name(),
                _ => continue,
            };
            if name != Some(target.as_str()) {
                continue;
            }
            let r = d.byte_range();
            if r.end > r.start + 1 {
                // Interior after the '&'/'*' marker — the shared text.
                ranges.push(lsp_range(bytes, li, r.start + 1..r.end));
            }
        }
        if !ranges.is_empty() {
            // descendants() walks children in reverse pre-order.
            ranges.sort_by_key(|r| (r.start.line, r.start.character));
            return Some(ranges);
        }
    }
    if !on_key {
        // Cursor on a value/colon with no alias under it: nothing to link.
        return None;
    }

    // Duplicate quoted scalar keys within one mapping.
    let content = key.content();
    if !matches!(
        content.scalar_style(),
        suspect_syntax::ScalarStyle::DoubleQuoted | suspect_syntax::ScalarStyle::SingleQuoted
    ) {
        return None;
    }
    let mut ancestor = pair.parent();
    while let Some(m) = ancestor {
        if m.kind() == SyntaxKind::Mapping {
            break;
        }
        ancestor = m.parent();
    }
    let mapping = ancestor?;
    let target_bytes = content.scalar_bytes();
    let mut ranges = Vec::new();
    for (sibling, _) in mapping.mapping_entries() {
        let sc = sibling.content();
        if !matches!(
            sc.scalar_style(),
            suspect_syntax::ScalarStyle::DoubleQuoted | suspect_syntax::ScalarStyle::SingleQuoted
        ) || sc.scalar_bytes() != target_bytes
        {
            continue;
        }
        ranges.push(lsp_range(bytes, li, scalar_interior(&sibling)));
    }
    (ranges.len() > 1).then_some(ranges)
}

/// Incremental FNV-1a 64-bit hasher (no external dependency needed).
struct Fnv1a {
    /// Current hash state.
    state: u64,
}

impl Fnv1a {
    /// Starts a fresh hash.
    fn new() -> Self {
        Self { state: FNV_OFFSET }
    }

    /// Mixes one byte slice into the state.
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    /// Returns the final hash value.
    fn finish(self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use suspect_ref::{Workspace, WorkspaceBuilder};
    use suspect_source::{Source, Uri};
    use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Position, Range};

    const MAIN: &str = "\
openapi: 3.1.0
info:
  title: Petstore
  version: \"1.0\"
paths:
  /pets:
    get:
      responses:
        '200':
          description: ok
components:
  schemas:
    Pet:
      type: object
      properties:
        name:
          type: string
    PetRef:
      $ref: ./comp.yaml#/Comp
";

    const BROKEN: &str = "\
openapi: 3.1.0
info:
  title: Broken
  version: \"1.0\"
paths: {}
components:
  schemas:
    Bad:
      $ref: '#/components/schemas/Missing'
";

    /// Writes `MAIN` plus a companion file and builds a loaded workspace.
    fn workspace(dir: &std::path::Path, companion: &str) -> Arc<Workspace> {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.yaml"), MAIN).unwrap();
        std::fs::write(dir.join("comp.yaml"), companion).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        Arc::new(ws)
    }

    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        LowDoc::parse(
            Uri::from_path(&p).unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn open_at(dir: &std::path::Path, name: &str, text: &str) -> OpenDoc {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        OpenDoc::parse(Uri::from_path(&p).unwrap(), text.to_owned())
    }

    fn offset_mid(text: &str, needle: &str) -> usize {
        let at = text.find(needle).expect("needle present");
        at + needle.len() / 2
    }

    fn diag(code: &str, line: u32, message: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 5 },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String(code.to_owned())),
            message: message.to_owned(),
            ..Diagnostic::default()
        }
    }

    fn tok(delta_line: u32, delta_start: u32, length: u32, token_type: u32) -> SemanticToken {
        SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        }
    }

    /// Applies a delta response to a prev array; returns the spliced result.
    fn apply(prev: &[SemanticToken], edits: &[SemanticTokensEdit]) -> Vec<SemanticToken> {
        let mut out: Vec<SemanticToken> = prev.to_vec();
        for e in edits {
            let start = e.start as usize;
            let del = e.delete_count as usize;
            let data = e.data.clone().unwrap_or_default();
            out.splice(start..start + del, data);
        }
        out
    }

    // ---- result ids -------------------------------------------------------

    #[test]
    fn diagnostics_result_id_is_order_independent_and_sensitive() {
        let a = vec![diag("x-a", 1, "one"), diag("x-b", 2, "two")];
        let b = vec![diag("x-b", 2, "two"), diag("x-a", 1, "one")];
        assert_eq!(diagnostics_result_id(&a), diagnostics_result_id(&b));

        let longer = vec![
            diag("x-a", 1, "one"),
            diag("x-b", 2, "two"),
            diag("x-c", 3, "three"),
        ];
        assert_ne!(diagnostics_result_id(&a), diagnostics_result_id(&longer));

        // Same code+range but different message must change the id.
        let changed = vec![diag("x-a", 1, "changed message")];
        assert_ne!(
            diagnostics_result_id(&a[..1]),
            diagnostics_result_id(&changed)
        );

        assert_eq!(diagnostics_result_id(&[]), diagnostics_result_id(&[]));
    }

    #[test]
    fn tokens_result_id_tracks_content() {
        let a = vec![tok(0, 0, 3, 1), tok(1, 2, 4, 2)];
        let same = vec![tok(0, 0, 3, 1), tok(1, 2, 4, 2)];
        assert_eq!(tokens_result_id(&a), tokens_result_id(&same));
        assert_ne!(tokens_result_id(&a), tokens_result_id(&[tok(0, 0, 3, 1)]));
    }

    // ---- pull_diagnostics -------------------------------------------------

    #[test]
    fn pull_diagnostics_is_stable_and_full_report() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-clean");
        let ws = workspace(
            &dir,
            "components:\n  schemas:\n    Comp:\n      type: object\n",
        );
        let low = low_at(&dir, "main.yaml", MAIN);

        let (id1, d1) = pull_diagnostics(&ws, &low, None, &Default::default());
        let (id2, d2) = pull_diagnostics(&ws, &low, Some(id1.clone()), &Default::default());
        assert_eq!(id1, id2, "same input must yield the same result id");
        assert_eq!(d1, d2, "full-report mode ignores the previous id");
    }

    #[test]
    fn pull_diagnostics_reports_broken_refs_differently() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-broken");
        let ws = workspace(
            &dir,
            "components:\n  schemas:\n    Comp:\n      type: object\n",
        );
        let clean = low_at(&dir, "main.yaml", MAIN);
        let broken = low_at(&dir, "broken.yaml", BROKEN);

        let (_, broken_diags) = pull_diagnostics(&ws, &broken, None, &Default::default());
        assert!(
            !broken_diags.is_empty(),
            "unresolved $ref must produce diagnostics"
        );

        let (id_broken, _) = pull_diagnostics(&ws, &broken, None, &Default::default());
        let (id_clean, _) = pull_diagnostics(&ws, &clean, None, &Default::default());
        assert_ne!(id_broken, id_clean);
    }

    #[test]
    fn pull_diagnostics_covers_syntax_errors() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-syntax");
        let garbage = ": : :\nfoo: [unclosed\n";
        let ws = workspace(&dir, garbage);
        let low = low_at(&dir, "comp.yaml", garbage);
        let (_, diags) = pull_diagnostics(&ws, &low, None, &Default::default());
        assert!(!diags.is_empty(), "parse recovery errors must surface");
    }

    // ---- workspace_pull ---------------------------------------------------

    #[test]
    fn workspace_pull_covers_every_loaded_document() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-ws");
        let comp = "components:\n  schemas:\n    Comp:\n      type: object\n";
        let ws = workspace(&dir, comp);

        let results = workspace_pull(&ws, &Default::default());
        assert_eq!(results.len(), 2, "main.yaml and comp.yaml are loaded");
        for (uri, _) in &results {
            let name = uri.as_str().rsplit('/').next().unwrap();
            assert!(matches!(name, "main.yaml" | "comp.yaml"), "got {name}");
        }
    }

    #[test]
    fn workspace_pull_surfaces_unparsable_documents() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-ws-bad");
        let garbage = ": : :\nkey: [unclosed\n";
        let ws = workspace(&dir, garbage);
        let results = workspace_pull(&ws, &Default::default());
        let comp = results
            .iter()
            .find(|(uri, _)| uri.as_str().ends_with("comp.yaml"))
            .expect("companion present even when unparsable");
        assert!(
            !comp.1.is_empty(),
            "syntax errors must be emitted, not skipped"
        );
    }

    // ---- semantic_tokens_range ---------------------------------------------

    /// Decodes an origin-encoded array back into absolute quintuples.
    fn decode_abs(tokens: &[SemanticToken]) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut out = Vec::with_capacity(tokens.len());
        let (mut line, mut col) = (0u32, 0u32);
        for t in tokens {
            line += t.delta_line;
            col = if t.delta_line == 0 {
                col + t.delta_start
            } else {
                t.delta_start
            };
            out.push((line, col, t.length, t.token_type, t.token_modifiers_bitset));
        }
        out
    }

    #[test]
    fn semantic_tokens_range_slices_the_full_set() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-range");
        let doc = open_at(&dir, "main.yaml", MAIN);
        let inner = doc.low.inner();
        let (bytes, li) = (inner.bytes(), inner.line_index());

        // Whole document: identical to the full walk.
        let all = semantic_tokens_range(&doc, li, 0..bytes.len());
        assert_eq!(all, crate::semantic::semantic_tokens_full(&doc).data);

        // Only the line carrying `get:`.
        let get_off = MAIN.find("\n    get:").expect("get: present") + 1;
        let (get_line, _) = li.line_col(bytes, get_off);
        let line_start = offset_of_utf16(bytes, li, get_line, 0).unwrap();
        let line_end = offset_of_utf16(bytes, li, get_line + 1, 0).unwrap_or(bytes.len());
        let slice = semantic_tokens_range(&doc, li, line_start..line_end);

        let abs = decode_abs(&slice);
        assert!(!abs.is_empty(), "the get: line has at least one token");
        for &(line, col, len, _, _) in &abs {
            let span = token_span(bytes, li, line, col, len).unwrap();
            assert!(
                span.start >= line_start && span.end <= line_end,
                "token at {line}:{col} escapes the requested range"
            );
        }
        // First token keeps its absolute position (delta from origin).
        assert_eq!(abs[0].0, get_line);
    }

    #[test]
    fn semantic_tokens_range_edge_cases() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-range-edge");
        let doc = open_at(&dir, "main.yaml", MAIN);
        let inner = doc.low.inner();

        // Empty and inverted ranges produce no tokens.
        assert!(semantic_tokens_range(&doc, inner.line_index(), 5..5).is_empty());
        #[allow(clippy::reversed_empty_ranges)] // deliberately inverted input
        let inverted = 10..3;
        assert!(semantic_tokens_range(&doc, inner.line_index(), inverted).is_empty());

        // An empty document yields nothing anywhere.
        let empty = open_at(&dir, "empty.yaml", "");
        let ei = empty.low.inner();
        assert!(
            semantic_tokens_range(&empty, ei.line_index(), 0..ei.bytes().len().max(1)).is_empty()
        );
    }

    // ---- semantic_tokens_delta ----------------------------------------------

    #[test]
    fn delta_no_change_gives_no_edits_but_new_result_id() {
        let next = vec![tok(0, 0, 4, 2), tok(2, 1, 3, 4)];
        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&next, &next)
        else {
            panic!("expected delta variant");
        };
        assert!(d.edits.is_empty());
        assert_eq!(
            d.result_id.as_deref(),
            Some(tokens_result_id(&next).as_str())
        );
    }

    #[test]
    fn delta_insertion_mid_file_shifts_subsequent_tokens() {
        // Before: tokens on lines 0 and 1. After typing a line above the
        // second token it lands on line 2 — a shifted copy replaces it.
        let prev = vec![tok(0, 0, 5, 1), tok(1, 0, 3, 2)];
        let next = vec![tok(0, 0, 5, 1), tok(2, 0, 3, 2)];

        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&prev, &next)
        else {
            panic!("expected delta variant");
        };
        assert_eq!(d.edits.len(), 1);
        let e = &d.edits[0];
        assert_eq!(e.start, 1);
        let data = e.data.as_ref().expect("replacement carries data");

        assert_eq!(e.delete_count, 1);

        // Splicing reproduces the next array exactly…
        assert_eq!(apply(&prev, &d.edits), next);
        // …and the replacement slice is origin-encoded (absolute line 2).
        assert_eq!(data[0].delta_line, 2);
    }

    #[test]
    fn delta_deletion_emits_delete_only_edit() {
        // Deleting the trailing token: nothing follows the edit, so a pure
        // delete with no data suffices.
        let prev = vec![tok(0, 0, 4, 0), tok(1, 0, 3, 1)];
        let next = vec![tok(0, 0, 4, 0)];

        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&prev, &next)
        else {
            panic!("expected delta variant");
        };
        assert_eq!(d.edits.len(), 1);
        assert_eq!(d.edits[0].start, 1);
        assert_eq!(d.edits[0].delete_count, 1);
        assert!(d.edits[0].data.is_none(), "pure deletions carry no data");
        assert_eq!(apply(&prev, &d.edits), next);
    }

    #[test]
    fn delta_replacement_trims_encoded_suffix() {
        // A token length change mid-file: the equal-encoded tail stays put
        // and only the middle token is re-emitted (seeded by its surviving
        // predecessor).
        let prev = vec![tok(0, 0, 4, 0), tok(1, 0, 3, 1), tok(0, 5, 2, 2)];
        let next = vec![tok(0, 0, 4, 0), tok(1, 0, 9, 1), tok(0, 5, 2, 2)];

        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&prev, &next)
        else {
            panic!("expected delta variant");
        };
        assert_eq!(d.edits.len(), 1);
        assert_eq!(d.edits[0].start, 1);
        assert_eq!(d.edits[0].delete_count, 1);
        let data = d.edits[0].data.as_ref().expect("carries replacement");
        // Seeded by the surviving predecessor on line 0.
        assert_eq!(data[0].delta_line, 1);
        assert_eq!(data[0].delta_start, 0);
        assert_eq!(apply(&prev, &d.edits), next);
    }

    #[test]
    fn delta_replacement_swaps_middle_run() {
        let prev = vec![tok(0, 0, 4, 0), tok(1, 2, 3, 1), tok(1, 6, 3, 1)];
        let next = vec![tok(0, 0, 4, 0), tok(1, 2, 7, 3), tok(1, 9, 1, 5)];

        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&prev, &next)
        else {
            panic!("expected delta variant");
        };
        assert_eq!(apply(&prev, &d.edits), next);
    }

    #[test]
    fn delta_from_and_to_empty() {
        // Empty → full: single insertion at index 0.
        let full = vec![tok(0, 0, 2, 0), tok(3, 1, 2, 1)];
        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&[], &full)
        else {
            panic!("expected delta variant");
        };
        assert_eq!(apply(&[], &d.edits), full);

        // Full → empty: single deletion of everything.
        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&full, &[])
        else {
            panic!("expected delta variant");
        };
        assert_eq!(apply(&full, &d.edits), Vec::new());

        // Empty → empty stays empty.
        let SemanticTokensFullDeltaResult::TokensDelta(d) = semantic_tokens_delta(&[], &[]) else {
            panic!("expected delta variant");
        };
        assert!(d.edits.is_empty());
    }

    // ---- linked_editing_range ------------------------------------------------

    const ANCHORED: &str = "\
components:
  schemas:
    &Pet Pet:
      type: string
    Alias: *Pet
    Other: {}
";

    const DUPLICATES: &str = "\
components:
  schemas:
    \"dup\": 1
    'dup': 2
    other: 3
";

    #[test]
    fn linked_editing_links_anchor_and_alias_occurrences() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-linked-anchor");
        let low = low_at(&dir, "main.yaml", ANCHORED);

        // Cursor inside the anchored key site (`&Pet Pet:`).
        let off = offset_mid(ANCHORED, "&Pet ");
        let ranges = linked_editing_range(&low, off).expect("anchor site links");

        assert_eq!(ranges.len(), 2, "declaration + alias occurrence");
        let texts: Vec<&str> = ranges.iter().map(|r| range_text(ANCHORED, r)).collect();
        assert!(texts.iter().all(|t| *t == "Pet"), "got {texts:?}");
        assert_eq!(ranges[0].start.line, 2, "anchored declaration first");
        assert_eq!(ranges[1].start.line, 4, "alias occurrence second");

        // Cursor directly on the alias occurrence finds the same group.
        let off_alias = ANCHORED.find("*Pet").expect("alias present") + 1;
        let again = linked_editing_range(&low, off_alias).expect("alias links");
        assert_eq!(ranges, again);
    }

    #[test]
    fn linked_editing_ignores_non_key_positions() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-linked-value");
        let low = low_at(&dir, "main.yaml", ANCHORED);

        // On the scalar VALUE of the anchored key's mapping body.
        let off_value = offset_mid(ANCHORED, "type:");
        assert!(linked_editing_range(&low, off_value).is_none());

        // On a plain, unbound key.
        let off_other = offset_mid(ANCHORED, "Other");
        assert!(linked_editing_range(&low, off_other).is_none());
    }

    #[test]
    fn linked_editing_links_duplicate_quoted_keys() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-linked-dup");
        let low = low_at(&dir, "main.yaml", DUPLICATES);

        let off_double = offset_mid(DUPLICATES, "\"dup\"");
        let ranges = linked_editing_range(&low, off_double).expect("quoted duplicates link");
        assert_eq!(ranges.len(), 2);
        for r in &ranges {
            assert_eq!(range_text(DUPLICATES, r), "dup", "interior excludes quotes");
        }

        // The single-quoted twin links to the same set.
        let off_single = DUPLICATES.find("'dup'").expect("quoted present") + 2;
        assert_eq!(linked_editing_range(&low, off_single), Some(ranges.clone()));
    }

    #[test]
    fn linked_editing_requires_quotes_for_duplicates() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-linked-plain");
        let text = "alpha: 1\nalpha: 2\n";
        let low = low_at(&dir, "main.yaml", text);
        // Plain duplicated keys are deliberately not linked (only quoted).
        let off = offset_mid(text, "alpha: 2");
        assert!(linked_editing_range(&low, off).is_none());
    }

    #[test]
    fn linked_editing_handles_degenerate_inputs() {
        let dir = std::env::temp_dir().join("suspect-lsp-pull-linked-empty");
        // Empty document: nothing at any offset.
        let empty = low_at(&dir, "empty.yaml", "");
        assert!(linked_editing_range(&empty, 0).is_none());

        // Anchors absent and duplicates absent: cursor on a plain key.
        let text = "alpha: 1\nbeta: 2\n";
        let plain = low_at(&dir, "plain.yaml", text);
        assert!(linked_editing_range(&plain, offset_mid(text, "alpha")).is_none());

        // Offset far past the end of the buffer.
        assert!(linked_editing_range(&plain, 10_000).is_none());
    }

    /// Slices the source text covered by a single-line LSP range.
    fn range_text<'a>(source: &'a str, r: &Range) -> &'a str {
        let line = source
            .split('\n')
            .nth(r.start.line as usize)
            .unwrap_or_default();
        let s = (r.start.character as usize).min(line.len());
        let e = (r.end.character as usize).min(line.len());
        &line[s..e]
    }
}
