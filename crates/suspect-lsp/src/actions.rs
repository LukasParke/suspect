//! Quick-fix code actions and full-document canonical formatting.
//!
//! Every quick fix is anchored in the CST: the diagnostic's byte range is
//! matched *exactly* against a block-mapping value (or a block-sequence
//! item mapping), so an edit is only produced when the anchor node can be
//! located. When the anchor cannot be found — malformed documents,
//! `$ref`-resolved targets, flow-style values — the action is silently
//! skipped rather than misplaced.

use std::collections::HashMap;

use suspect_low::{LowDoc, ValueKind};
use suspect_overlay::Value as OverlayValue;
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::*;

use crate::navigation;
use crate::state::{OpenDoc, lsp_range, offset_of_utf16};

/// Upper bound on quick-fix actions returned per request.
const MAX_ACTIONS: usize = 20;

/// Operation keys recognized when anchoring operation-level fixes.
const HTTP_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Computes quick-fix [`CodeAction`]s for `diagnostics` whose range
/// intersects `range` and whose code has a known fix, capped at
/// 20 entries (server-side cap).
///
/// When `range` covers the whole document, a single
/// `source.fixAll.suspect` action (`SOURCE_ORGANIZE` kind) applying every
/// applicable fix is appended; conflicting or overlapping edits are
/// dropped there.
#[must_use]
pub fn code_actions(
    doc: &OpenDoc,
    uri: &Url,
    range: Range,
    diagnostics: &[Diagnostic],
) -> Vec<CodeAction> {
    let mut actions = Vec::new();
    for d in diagnostics {
        if actions.len() >= MAX_ACTIONS {
            break;
        }
        if !intersects(d.range, range) {
            continue;
        }
        let Some(code) = string_code(d) else { continue };
        let Some(fix) = fix_for(doc, &code, d) else {
            continue;
        };
        let title = fix.title;
        actions.push(CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![d.clone()]),
            edit: Some(workspace_edit(uri, vec![fix.edit])),
            ..CodeAction::default()
        });
    }
    if covers_document(doc, range) {
        let edits = all_edits(doc, diagnostics);
        if !edits.is_empty() {
            actions.push(CodeAction {
                title: "Fix all suspect issues".to_owned(),
                kind: Some(CodeActionKind::new("source.fixAll.suspect")),
                data: Some(serde_json::json!("source.fixAll.suspect")),
                edit: Some(workspace_edit(uri, edits)),
                ..CodeAction::default()
            });
        }
    }
    actions
}

/// Full-document canonical format as a single [`TextEdit`] replacing the
/// entire buffer.
///
/// The document is materialized through [`OverlayValue::from_node`] and
/// re-emitted as YAML (or pretty JSON when the URI ends in `.json`) with
/// the default two-space indentation; formatting options are ignored.
/// Documents with syntax errors or non-collection roots are never
/// formatted, since emission would silently drop content.
#[must_use]
pub fn format_document(doc: &OpenDoc, uri: &Url) -> Option<TextEdit> {
    if !doc.low.syntax_errors().is_empty() {
        return None;
    }
    let root = doc.low.root().resolved();
    if !matches!(root.kind(), ValueKind::Object | ValueKind::Array) {
        return None;
    }
    let value = OverlayValue::from_node(root);
    let mut text = if uri.path().ends_with(".json") {
        value.to_json_pretty()
    } else {
        value.to_yaml()
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let inner = doc.low.inner();
    let range = lsp_range(inner.bytes(), inner.line_index(), 0..inner.bytes().len());
    Some(TextEdit {
        range,
        new_text: text,
    })
}

/// One computed fix: a human title, the byte span it touches (zero-width
/// for pure insertions), and the resulting LSP edit.
struct Fix {
    /// Human-readable action title.
    title: String,
    /// Byte span used for overlap/conflict detection during fix-all.
    span: std::ops::Range<usize>,
    /// The edit itself, positioned via UTF-16 LSP coordinates.
    edit: TextEdit,
}

/// Builds the fix for one diagnostic code, or `None` when the code is
/// unknown or its CST anchor cannot be located.
fn fix_for(doc: &OpenDoc, code: &str, diag: &Diagnostic) -> Option<Fix> {
    let inner = doc.low.inner();
    let bytes = inner.bytes();
    let li = inner.line_index();
    let start = offset_of_utf16(bytes, li, diag.range.start.line, diag.range.start.character)?;
    let end = offset_of_utf16(bytes, li, diag.range.end.line, diag.range.end.character)?;
    match code {
        "oas-path-trailing-slash" => trailing_slash_fix(doc, start..end),
        "oas-operation-missing-operationId" => operation_id_fix(doc, start..end),
        "oas-operation-missing-responses" => pair_insert_fix(
            doc,
            start..end,
            &HTTP_METHODS,
            &["responses:", "  default:", "    description: Responses"],
            "Add default responses",
        ),
        "oas-response-missing-description" => response_description_fix(doc, start..end),
        "oas-parameter-missing-name" => {
            any_insert_fix(doc, start..end, &[], &["name: "], "Add parameter name")
        }
        "info-contact" => pair_insert_fix(
            doc,
            start..end,
            &["info"],
            &[
                "contact:",
                "  name: API Support",
                "  email: support@example.com",
            ],
            "Add contact to info",
        ),
        "info-license" => pair_insert_fix(
            doc,
            start..end,
            &["info"],
            &["license:", "  name: MIT"],
            "Add license to info",
        ),
        "operation-tags" => pair_insert_fix(
            doc,
            start..end,
            &HTTP_METHODS,
            &["tags:", "  - default"],
            "Add tags to operation",
        ),
        _ => None,
    }
}

/// How a diagnostic's range was matched against the CST.
enum Anchor<'d> {
    /// The range is exactly the block-mapping value of a pair; carries the
    /// pair's key node.
    Pair {
        /// Key scalar node of the owning pair.
        key: SNode<'d>,
    },
    /// The range is exactly one block-sequence item mapping; carries the
    /// item's first entry key (insertions go above it).
    Item {
        /// First entry key of the item mapping.
        first_key: SNode<'d>,
    },
}

/// Locates the CST anchor for a diagnostic byte range: walks up from the
/// smallest node at the range start until a pair's block-mapping value or
/// a sequence-item mapping matches the range exactly.
fn anchor<'d>(low: &'d LowDoc, br: std::ops::Range<usize>) -> Option<Anchor<'d>> {
    let bytes = low.inner().bytes();
    let mut cur = navigation::node_at(low, br.start)?;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            if let (Some(key), Some(val)) = (cur.child_by_field("key"), cur.child_by_field("value"))
            {
                let (key, val) = (key.content(), val.content());
                if val.byte_range() == br
                    && bytes[key.end_byte()..val.start_byte()].contains(&b'\n')
                {
                    return Some(Anchor::Pair { key });
                }
            }
        } else if cur.kind() == SyntaxKind::Mapping
            && cur.byte_range() == br
            && ancestor_of_kind(cur.parent()?, SyntaxKind::Sequence).is_some()
        {
            let first_key = cur.mapping_entries().into_iter().next().map(|(k, _)| k)?;
            return Some(Anchor::Item { first_key });
        }
        cur = cur.parent()?;
    }
}

/// Nearest node of `kind` in `node`'s ancestor chain (including `node`).
///
/// The YAML CST interleaves wrapper nodes (`block_node`, `flow_node`)
/// between logical parents, so parent links cannot be trusted directly.
fn ancestor_of_kind<'d>(node: SNode<'d>, kind: SyntaxKind) -> Option<SNode<'d>> {
    let mut cur = node;
    loop {
        if cur.kind() == kind {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Inserts new mapping entries under the mapping owned by a pair whose key
/// is one of `owner_keys`; skips sequence-item anchors entirely.
fn pair_insert_fix(
    doc: &OpenDoc,
    br: std::ops::Range<usize>,
    owner_keys: &[&str],
    lines: &[&str],
    title: &str,
) -> Option<Fix> {
    match anchor(&doc.low, br)? {
        Anchor::Pair { key } => {
            let key_text = scalar_text(&key);
            if !owner_keys.is_empty() && !owner_keys.contains(&key_text.as_str()) {
                return None;
            }
            if inside_flow_collection(&key) {
                return None;
            }
            Some(insert_after_key_line(doc, &key, lines, title))
        }
        Anchor::Item { .. } => None,
    }
}

/// True when inserting block-style entries anchored at `key` would land
/// inside a flow-style collection and corrupt it: the owning pair is a
/// flow pair (`{a: b}` entry), the pair's value text opens a flow
/// collection (`{` / `[`, which also covers every JSON composite), or an
/// ancestor mapping/sequence is itself flow-style.
fn inside_flow_collection(key: &SNode<'_>) -> bool {
    let Some(pair) = ancestor_of_kind(*key, SyntaxKind::Pair) else {
        return true;
    };
    if pair.raw_kind() == "flow_pair" {
        return true;
    }
    if let Some(val) = pair.child_by_field("value") {
        let opens_flow = val
            .content()
            .text()
            .iter()
            .find(|&&b| !b.is_ascii_whitespace())
            .is_some_and(|&b| b == b'{' || b == b'[');
        if opens_flow {
            return true;
        }
    }
    let mut cur = pair.parent();
    while let Some(node) = cur {
        if matches!(node.raw_kind(), "flow_mapping" | "flow_sequence") {
            return true;
        }
        cur = node.parent();
    }
    false
}

/// Like [`pair_insert_fix`] but also accepts block-sequence item mappings,
/// inserting the new entries as the item's leading entries.
fn any_insert_fix(
    doc: &OpenDoc,
    br: std::ops::Range<usize>,
    owner_keys: &[&str],
    lines: &[&str],
    title: &str,
) -> Option<Fix> {
    match anchor(&doc.low, br)? {
        Anchor::Pair { key } => {
            let key_text = scalar_text(&key);
            if !owner_keys.is_empty() && !owner_keys.contains(&key_text.as_str()) {
                return None;
            }
            if inside_flow_collection(&key) {
                return None;
            }
            Some(insert_after_key_line(doc, &key, lines, title))
        }
        Anchor::Item { first_key } => {
            if inside_flow_collection(&first_key) {
                return None;
            }
            let bytes = doc.low.inner().bytes();
            let indent = " ".repeat(column_of(bytes, first_key.start_byte()));
            let mut new_text = lines.join(&format!("\n{indent}"));
            new_text.push('\n');
            new_text.push_str(&indent);
            let at = first_key.start_byte();
            Some(make_fix(doc, title, at..at, new_text))
        }
    }
}

/// Adds `description: <status> response` under the response mapping; the
/// status name comes from the owning pair's key (`'200':` → `200`).
fn response_description_fix(doc: &OpenDoc, br: std::ops::Range<usize>) -> Option<Fix> {
    let status = match anchor(&doc.low, br.clone())? {
        Anchor::Pair { key } => scalar_text(&key),
        Anchor::Item { .. } => return None,
    };
    let lines = [format!("description: {status} response")];
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut fix = pair_insert_fix(doc, br, &[], &line_refs, "add response description")?;
    fix.title = format!("Add description for `{status}` response");
    Some(fix)
}

/// Rewrites a plain-text path key without its trailing slash.
fn trailing_slash_fix(doc: &OpenDoc, br: std::ops::Range<usize>) -> Option<Fix> {
    let key = match anchor(&doc.low, br)? {
        Anchor::Pair { key } => key,
        Anchor::Item { .. } => return None,
    };
    // Plain style only: quoted keys would lose their quoting on rewrite.
    let raw = key.text_lossy();
    let scalar = scalar_text(&key);
    if raw != scalar || !scalar.ends_with('/') {
        return None;
    }
    let trimmed = scalar.strip_suffix('/')?;
    // `//` would trim to an empty (null) key; only a `/`-prefixed path
    // object key is rewriteable at all.
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return None;
    }
    let range = key.byte_range();
    Some(make_fix(
        doc,
        "Remove trailing slash",
        range,
        trimmed.to_owned(),
    ))
}

/// Inserts a deterministic camelCase `operationId` under the operation.
fn operation_id_fix(doc: &OpenDoc, br: std::ops::Range<usize>) -> Option<Fix> {
    let key = match anchor(&doc.low, br.clone())? {
        Anchor::Pair { key } => key,
        Anchor::Item { .. } => return None,
    };
    let method = scalar_text(&key);
    if !HTTP_METHODS.contains(&method.as_str()) {
        return None;
    }
    // `get:` pair → Path Item mapping → Path pair → path key.
    let pair = ancestor_of_kind(key, SyntaxKind::Pair)?;
    let item_map = ancestor_of_kind(pair.parent()?, SyntaxKind::Mapping)?;
    let path_pair = ancestor_of_kind(item_map.parent()?, SyntaxKind::Pair)?;
    let path = path_pair.child_by_field("key")?.content();
    let path = scalar_text(&path);
    if !path.starts_with('/') {
        return None;
    }
    let id = derive_operation_id(&method, &path);
    let lines = [format!("operationId: {id}")];
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut fix = pair_insert_fix(doc, br, &HTTP_METHODS, &line_refs, "add operationId")?;
    fix.title = format!("Add operationId `{id}`");
    Some(fix)
}

/// Derives an operationId: lowercase HTTP method followed by each path
/// segment in PascalCase; `{var}` template segments contribute `By` +
/// PascalCase variable name. Example: `/pets/{id}` + GET → `getPetsById`.
fn derive_operation_id(method: &str, path: &str) -> String {
    let mut out = method.to_ascii_lowercase();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if let Some(var) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            out.push_str("By");
            out.push_str(&pascal(var));
        } else {
            out.push_str(&pascal(seg));
        }
    }
    out
}

/// PascalCases one word: splits on punctuation, uppercases each part's
/// first letter and keeps the remainder (`my-page` → `MyPage`,
/// `userId` → `UserId`).
fn pascal(word: &str) -> String {
    word.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            let Some(first) = cs.next() else {
                return String::new();
            };
            first.to_uppercase().collect::<String>() + cs.as_str()
        })
        .collect()
}

/// Sorts fixes into descending positional order (the LSP requirement) and
/// drops every fix whose replaced span intersects an already-accepted
/// edit's span — overlapping or coincident — returning the surviving
/// edits.
fn merge_fixes(mut fixes: Vec<Fix>) -> Vec<TextEdit> {
    fixes.sort_by(|a, b| {
        b.span
            .start
            .cmp(&a.span.start)
            .then_with(|| b.span.end.cmp(&a.span.end))
    });
    let mut out = Vec::with_capacity(fixes.len());
    let mut limit = usize::MAX;
    for f in fixes {
        // Accept only spans lying entirely below the accepted region: a
        // span crossing `limit` would be applied at offsets the earlier
        // (higher) edit has already shifted, corrupting the buffer. An
        // insertion exactly at `limit` would collide with that edit too.
        if f.span.end < limit {
            limit = limit.min(f.span.start);
            out.push(f.edit);
        }
    }
    out
}

/// Merges every applicable fix across `diagnostics` into one edit list
/// via [`merge_fixes`].
fn all_edits(doc: &OpenDoc, diagnostics: &[Diagnostic]) -> Vec<TextEdit> {
    let fixes = diagnostics
        .iter()
        .filter_map(|d| fix_for(doc, &string_code(d)?, d))
        .collect();
    merge_fixes(fixes)
}

fn workspace_edit(uri: &Url, edits: Vec<TextEdit>) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: Some(HashMap::from([(uri.clone(), edits)])),
        ..WorkspaceEdit::default()
    }
}

/// Builds an insertion placed at the end of the anchor key's line, with
/// each new line indented to the key column + 2.
fn insert_after_key_line(doc: &OpenDoc, key: &SNode<'_>, lines: &[&str], title: &str) -> Fix {
    let bytes = doc.low.inner().bytes();
    let indent = " ".repeat(column_of(bytes, key.start_byte()) + 2);
    let at = line_end(bytes, key.end_byte());
    let mut new_text = String::new();
    for line in lines {
        new_text.push('\n');
        new_text.push_str(&indent);
        new_text.push_str(line);
    }
    make_fix(doc, title, at..at, new_text)
}

/// First byte offset of the line containing `offset`.
fn line_start(bytes: &[u8], offset: usize) -> usize {
    bytes[..offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1)
}

/// Byte offset of the newline terminating the line containing `offset`
/// (the buffer end when it is the last line).
fn line_end(bytes: &[u8], offset: usize) -> usize {
    bytes[offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| offset + p)
}

/// Zero-based column of `offset` within its line (in bytes).
fn column_of(bytes: &[u8], offset: usize) -> usize {
    offset - line_start(bytes, offset)
}

/// Assembles a [`Fix`] from a byte span, mapping it to UTF-16 LSP
/// coordinates through the document's line index.
fn make_fix(doc: &OpenDoc, title: &str, span: std::ops::Range<usize>, new_text: String) -> Fix {
    let inner = doc.low.inner();
    let edit = TextEdit {
        range: lsp_range(inner.bytes(), inner.line_index(), span.clone()),
        new_text,
    };
    Fix {
        title: title.to_owned(),
        span,
        edit,
    }
}

/// Unquoted scalar text of a key/value node, decoded lossily.
fn scalar_text(node: &SNode<'_>) -> String {
    String::from_utf8_lossy(node.scalar_bytes()).into_owned()
}

/// True when two LSP ranges share at least one point.
fn intersects(a: Range, b: Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Extracts the string form of a diagnostic's code, if any.
fn string_code(d: &Diagnostic) -> Option<String> {
    match &d.code {
        Some(NumberOrString::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// True when `range` spans the whole document starting at its very first
/// character (the shape editors send for "select all").
fn covers_document(doc: &OpenDoc, range: Range) -> bool {
    let inner = doc.low.inner();
    let end = lsp_range(inner.bytes(), inner.line_index(), 0..inner.bytes().len()).end;
    range.start == Position::default() && range.end >= end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::SOURCE;
    use suspect_source::Uri;

    const YAML_URI: &str = "mem://actions-test.yaml";
    const API: &str = "\
openapi: 3.1.0
info:
  title: Demo
  version: \"1\"
paths:
  /pets/{id}:
    get:
      summary: Find pet
";

    fn open(text: &str) -> OpenDoc {
        OpenDoc::parse(YAML_URI.into(), text.to_owned())
    }

    fn url(name: &str) -> Url {
        Url::parse(&format!("file:///workspaces/demo/{name}")).unwrap()
    }

    /// Diagnostics whose ranges are exactly the block values of every pair
    /// with scalar key `key`.
    fn diags_on_values(d: &OpenDoc, key: &[u8], code: &str) -> Vec<Diagnostic> {
        let inner = d.low.inner();
        inner
            .root()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::Pair)
            .filter(|n| {
                n.child_by_field("key")
                    .is_some_and(|k| k.content().scalar_bytes() == key)
            })
            .filter_map(|n| n.child_by_field("value"))
            .map(|v| byte_diag(d, v.content().byte_range(), code))
            .collect()
    }

    fn diag_on_value(d: &OpenDoc, key: &[u8], code: &str) -> Diagnostic {
        diags_on_values(d, key, code).into_iter().next().unwrap()
    }

    /// Diagnostic spanning the block-sequence item mapping that contains
    /// `needle`.
    fn diag_on_seq_item(d: &OpenDoc, needle: &str, code: &str) -> Diagnostic {
        let inner = d.low.inner();
        let off = d.text.find(needle).expect("needle present");
        let item = inner
            .root()
            .descendants()
            .find(|n| {
                n.kind() == SyntaxKind::Mapping
                    && n.byte_range().contains(&off)
                    && n.parent()
                        .and_then(|p| ancestor_of_kind(p, SyntaxKind::Sequence))
                        .is_some()
            })
            .expect("sequence item mapping");
        byte_diag(d, item.byte_range(), code)
    }

    fn byte_diag(d: &OpenDoc, br: std::ops::Range<usize>, code: &str) -> Diagnostic {
        let inner = d.low.inner();
        Diagnostic {
            range: lsp_range(inner.bytes(), inner.line_index(), br),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String(code.to_owned())),
            source: Some(SOURCE.to_owned()),
            message: "test finding".to_owned(),
            ..Diagnostic::default()
        }
    }

    fn whole_doc(d: &OpenDoc) -> Range {
        let inner = d.low.inner();
        lsp_range(inner.bytes(), inner.line_index(), 0..inner.bytes().len())
    }

    /// LSP edits belonging to `uri` across all returned actions.
    fn edits_of<'a>(actions: &'a [CodeAction], uri: &Url) -> Vec<&'a TextEdit> {
        actions
            .iter()
            .filter_map(|a| a.edit.as_ref())
            .flat_map(|w| w.changes.as_ref().unwrap().get(uri).unwrap())
            .collect()
    }

    /// Applies edits sequentially; callers must pass them in descending
    /// positional order so earlier (lower-offset) edits stay valid.
    fn apply(d: &OpenDoc, edits: &[&TextEdit]) -> String {
        let mut out = d.text.clone();
        let inner = d.low.inner();
        for e in edits {
            let s = offset_of_utf16(
                inner.bytes(),
                inner.line_index(),
                e.range.start.line,
                e.range.start.character,
            )
            .unwrap();
            let en = offset_of_utf16(
                inner.bytes(),
                inner.line_index(),
                e.range.end.line,
                e.range.end.character,
            )
            .unwrap();
            out.replace_range(s..en, &e.new_text);
        }
        out
    }

    #[test]
    fn operation_id_quick_fix_derives_deterministic_id() {
        let d = open(API);
        let diag = diag_on_value(&d, b"get", "oas-operation-missing-operationId");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1, "{acts:?}");
        assert_eq!(acts[0].kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(acts[0].title, "Add operationId `getPetsById`");
        let edits = edits_of(&acts, &uri);
        assert_eq!(edits.len(), 1);
        // Inserted right under `get:`, indented to its column + 2.
        let fixed = apply(&d, &edits);
        assert!(
            fixed.contains("    get:\n      operationId: getPetsById\n"),
            "{fixed}"
        );
    }

    #[test]
    fn responses_skeleton_quick_fix_inserts_default_response() {
        let d = open(API);
        let diag = diag_on_value(&d, b"get", "oas-operation-missing-responses");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].title, "Add default responses");
        let fixed = apply(&d, &edits_of(&acts, &uri));
        assert!(
            fixed.contains(
                "    get:\n      responses:\n        default:\n          description: Responses\n"
            ),
            "{fixed}"
        );
    }

    #[test]
    fn response_description_quick_fix_names_the_status() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /p:
    get:
      responses:
        '200':
          content: {}
";
        let d = open(text);
        let diag = diag_on_value(&d, b"200", "oas-response-missing-description");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].title, "Add description for `200` response");
        let fixed = apply(&d, &edits_of(&acts, &uri));
        assert!(
            fixed.contains("        '200':\n          description: 200 response\n"),
            "{fixed}"
        );
    }

    #[test]
    fn trailing_slash_quick_fix_rewrites_the_path_key() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /pets/:
    get:
      summary: S
";
        let d = open(text);
        let diag = diag_on_value(&d, b"/pets/", "oas-path-trailing-slash");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].title, "Remove trailing slash");
        let edits = edits_of(&acts, &uri);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "/pets");
        assert_eq!(edits[0].range.start, Position::new(4, 2));
        assert_eq!(edits[0].range.end, Position::new(4, 8));
        let fixed = apply(&d, &edits);
        assert!(fixed.contains("\n  /pets:\n"), "{fixed}");
    }

    #[test]
    fn parameter_name_quick_fix_for_named_component() {
        let text = "\
components:
  parameters:
    Limit:
      schema:
        type: integer
";
        let d = open(text);
        let diag = diag_on_value(&d, b"Limit", "oas-parameter-missing-name");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].title, "Add parameter name");
        let fixed = apply(&d, &edits_of(&acts, &uri));
        assert!(fixed.contains("    Limit:\n      name: \n"), "{fixed}");
    }

    #[test]
    fn parameter_name_quick_fix_anchors_inside_sequence_item() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /p:
    get:
      parameters:
        - in: query
";
        let d = open(text);
        let diag = diag_on_seq_item(&d, "in:", "oas-parameter-missing-name");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1, "{acts:?}");
        let fixed = apply(&d, &edits_of(&acts, &uri));
        assert!(fixed.contains("- name: \n          in: query\n"), "{fixed}");
    }

    #[test]
    fn contact_and_license_quick_fixes_target_info() {
        for (code, expect) in [
            (
                "info-contact",
                "info:\n  contact:\n    name: API Support\n    email: support@example.com\n  title: Demo\n",
            ),
            (
                "info-license",
                "info:\n  license:\n    name: MIT\n  title: Demo\n",
            ),
        ] {
            let d = open(API);
            let diag = diag_on_value(&d, b"info", code);
            let uri = url("api.yaml");
            let acts = code_actions(&d, &uri, diag.range, &[diag]);
            assert_eq!(acts.len(), 1, "{code}");
            let fixed = apply(&d, &edits_of(&acts, &uri));
            assert!(fixed.contains(expect), "{code}: {fixed}");
        }
    }

    #[test]
    fn tags_quick_fix_adds_default_tag_to_operation() {
        let d = open(API);
        let diag = diag_on_value(&d, b"get", "operation-tags");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].title, "Add tags to operation");
        let fixed = apply(&d, &edits_of(&acts, &uri));
        assert!(
            fixed.contains("    get:\n      tags:\n        - default\n"),
            "{fixed}"
        );
    }

    #[test]
    fn unknown_codes_and_missing_anchors_produce_nothing() {
        // Unknown code.
        let d = open(API);
        let uri = url("api.yaml");
        let diag = byte_diag(&d, 0..5, "totally-unknown-code");
        assert!(code_actions(&d, &uri, diag.range, &[diag]).is_empty());
        // Malformed buffer: anchors cannot be located, nothing panics.
        let bad = open("{: ::\n  - ]]\n\t: [\n");
        for code in [
            "oas-operation-missing-operationId",
            "oas-operation-missing-responses",
            "oas-response-missing-description",
            "oas-path-trailing-slash",
            "oas-parameter-missing-name",
            "info-contact",
            "info-license",
            "operation-tags",
        ] {
            let diag = byte_diag(&bad, 0..bad.text.len(), code);
            let acts = code_actions(&bad, &uri, whole_doc(&bad), &[diag]);
            assert!(
                acts.iter()
                    .all(|a| a.kind != Some(CodeActionKind::QUICKFIX)),
                "{code}: {acts:?}"
            );
        }
    }

    #[test]
    fn diagnostics_outside_the_request_range_are_filtered() {
        let d = open(API);
        let uri = url("api.yaml");
        let diag = diag_on_value(&d, b"get", "oas-operation-missing-operationId");
        // Line 0 only: the operation diagnostic sits far below.
        let acts = code_actions(
            &d,
            &uri,
            Range::new(Position::new(0, 0), Position::new(0, 0)),
            &[diag],
        );
        assert!(acts.is_empty(), "{acts:?}");
    }

    #[test]
    fn fix_all_merges_edits_descending_and_skips_conflicts() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /pets/:
    get:
      summary: S
";
        let d = open(text);
        let mut diags = vec![
            diag_on_value(&d, b"info", "info-contact"),
            diag_on_value(&d, b"info", "info-license"), // collides with contact
            diag_on_value(&d, b"/pets/", "oas-path-trailing-slash"),
            diag_on_value(&d, b"get", "operation-tags"),
        ];
        diags.push(diags[3].clone()); // exact duplicate must be skipped too
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, whole_doc(&d), &diags);
        let fix_all = acts
            .iter()
            .find(|a| a.kind == Some(CodeActionKind::new("source.fixAll.suspect")))
            .expect("fix-all action");
        let edits = edits_of(std::slice::from_ref(fix_all), &uri);
        // contact/license collide (same anchor line) → one survives; the
        // duplicate tags diagnostic is dropped as well.
        assert_eq!(edits.len(), 3, "{edits:?}");
        // LSP requires descending positional order.
        assert!(
            edits
                .windows(2)
                .all(|w| w[0].range.start > w[1].range.start)
        );
        let fixed = apply(&d, &edits);
        assert!(fixed.contains("/pets:"), "{fixed}");
        assert!(fixed.contains("name: API Support"), "{fixed}");
        assert!(fixed.contains("tags:\n        - default"), "{fixed}");
        let reparsed = LowDoc::parse(
            Uri::parse(YAML_URI).unwrap(),
            suspect_source::Source::from_vec(fixed.into_bytes()),
        );
        assert!(
            reparsed.syntax_errors().is_empty(),
            "merged edit must stay valid YAML"
        );
    }

    #[test]
    fn fix_all_only_appears_when_the_range_covers_the_document() {
        let d = open(API);
        let uri = url("api.yaml");
        let diags = vec![diag_on_value(&d, b"get", "operation-tags")];
        let acts = code_actions(&d, &uri, diags[0].range, &diags);
        assert!(
            acts.iter()
                .all(|a| a.kind == Some(CodeActionKind::QUICKFIX))
        );
    }

    #[test]
    fn quick_fix_list_is_capped_at_twenty() {
        let mut paths = String::new();
        for i in 0..30 {
            paths.push_str(&format!("  /p{i}:\n    get:\n      summary: s\n"));
        }
        let d = open(&format!(
            "openapi: 3.1.0\ninfo:\n  title: T\npaths:\n{paths}"
        ));
        let diags = diags_on_values(&d, b"get", "oas-operation-missing-responses");
        assert_eq!(diags.len(), 30);
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, whole_doc(&d), &diags);
        let quick = acts
            .iter()
            .filter(|a| a.kind == Some(CodeActionKind::QUICKFIX))
            .count();
        assert_eq!(quick, 20);
        assert!(
            acts.iter()
                .any(|a| a.kind == Some(CodeActionKind::new("source.fixAll.suspect")))
        );
    }

    #[test]
    fn formatting_round_trips_yaml() {
        let d = open(API);
        let uri = url("api.yaml");
        let edit = format_document(&d, &uri).expect("formats");
        assert_eq!(edit.range, whole_doc(&d));
        let formatted = edit.new_text;
        let reparsed = LowDoc::parse(
            Uri::parse(YAML_URI).unwrap(),
            suspect_source::Source::from_vec(formatted.clone().into_bytes()),
        );
        assert_eq!(reparsed.sniff_family(), d.low.sniff_family());
        assert!(reparsed.syntax_errors().is_empty(), "{formatted}");
        // Lossless semantics: the materialized tree is unchanged.
        assert_eq!(
            OverlayValue::from_node(reparsed.root()),
            OverlayValue::from_node(d.low.root()),
            "{formatted}"
        );
    }

    #[test]
    fn formatting_round_trips_json() {
        let text = r#"{"openapi":"3.1.0","info":{"title":"T","version":"1"},"paths":{}}"#;
        let d = OpenDoc::parse("mem://actions-test.json".into(), text.to_owned());
        let uri = url("api.json");
        let edit = format_document(&d, &uri).expect("formats");
        assert!(
            edit.new_text.starts_with("{\n  \"openapi\": \""),
            "{}",
            edit.new_text
        );
        let reparsed = LowDoc::parse(
            Uri::parse("mem://actions-test.json").unwrap(),
            suspect_source::Source::from_vec(edit.new_text.clone().into_bytes()),
        );
        assert_eq!(reparsed.format(), suspect_syntax::Format::Json);
        assert_eq!(reparsed.sniff_family(), d.low.sniff_family());
        assert!(reparsed.syntax_errors().is_empty(), "{}", edit.new_text);
    }

    #[test]
    fn formatting_skips_broken_or_scalar_documents() {
        let broken = open("title: \"unclosed\n");
        assert!(format_document(&broken, &url("b.yaml")).is_none());
        let scalar = open("just a string\n");
        assert!(format_document(&scalar, &url("s.yaml")).is_none());
        let empty_map = open("# only a comment\n");
        assert!(format_document(&empty_map, &url("e.yaml")).is_none());
    }

    #[test]
    fn derived_operation_ids_follow_path_segments() {
        assert_eq!(derive_operation_id("GET", "/pets/{id}"), "getPetsById");
        assert_eq!(
            derive_operation_id("post", "/users/{userId}/pets"),
            "postUsersByUserIdPets"
        );
        assert_eq!(
            derive_operation_id("get", "/my-page/items"),
            "getMyPageItems"
        );
    }
    #[test]
    fn trailing_slash_quick_fix_strips_exactly_one_slash() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /pets//:
    get:
      summary: S
";
        let d = open(text);
        let diag = diag_on_value(&d, b"/pets//", "oas-path-trailing-slash");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1, "{acts:?}");
        // Minimal edit: `/pets//` → `/pets/`, not an over-stripped `/pets`.
        assert_eq!(edits_of(&acts, &uri)[0].new_text, "/pets/");
    }

    #[test]
    fn trailing_slash_quick_fix_never_empties_the_key() {
        // `//` trims to `/` (which no longer trips the len>1 lint); the
        // pre-fix behavior rewrote the key to nothing.
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  //:
    get:
      summary: S
";
        let d = open(text);
        let diag = diag_on_value(&d, b"//", "oas-path-trailing-slash");
        let uri = url("api.yaml");
        let acts = code_actions(&d, &uri, diag.range, &[diag]);
        assert_eq!(acts.len(), 1, "{acts:?}");
        assert_eq!(edits_of(&acts, &uri)[0].new_text, "/");
    }

    #[test]
    fn insert_fixes_skip_flow_style_values() {
        // The value of `"info"` sits on its own line (so the anchor
        // matches) but is a flow mapping — inserting block entries after
        // the key line would corrupt it.
        let text =
            "{\n  \"openapi\": \"3.1.0\",\n  \"info\":\n    {\n      \"title\": \"T\"\n    }\n}\n";
        let d = OpenDoc::parse("mem://actions-test.json".into(), text.to_owned());
        let diag = diag_on_value(&d, b"info", "info-contact");
        let acts = code_actions(&d, &url("api.json"), diag.range, &[diag]);
        assert!(acts.is_empty(), "{acts:?}");
    }

    #[test]
    fn insert_fixes_skip_flow_style_ancestors() {
        // The item "mapping" is a flow mapping; inserting before its first
        // key would splice block lines inside `{…}`.
        let text = "\
components:
  parameters:
    - {name: a}
";
        let d = open(text);
        let diag = diag_on_seq_item(&d, "name", "oas-parameter-missing-name");
        let acts = code_actions(&d, &url("api.yaml"), diag.range, &[diag]);
        assert!(acts.is_empty(), "{acts:?}");
    }

    #[test]
    fn merge_fixes_rejects_overlapping_spans() {
        let fix = |start: usize, end: usize| Fix {
            title: String::new(),
            span: start..end,
            edit: TextEdit {
                range: Range::default(),
                new_text: String::new(),
            },
        };
        // [10,20] crosses [15,25]'s start → only the higher span survives;
        // the old condition accepted both and applied the second at stale
        // offsets.
        assert_eq!(merge_fixes(vec![fix(15, 25), fix(10, 20)]).len(), 1);
        // Fully disjoint spans both survive.
        assert_eq!(merge_fixes(vec![fix(15, 25), fix(0, 10)]).len(), 2);
        // Coincident zero-width insertions collapse to one.
        assert_eq!(merge_fixes(vec![fix(7, 7), fix(7, 7)]).len(), 1);
    }

    #[test]
    fn formatting_preserves_empty_valued_keys() {
        let text = "openapi: 3.1.0\ninfo:\n  title: T\ndescription:\n";
        let d = open(text);
        let uri = url("api.yaml");
        let edit = format_document(&d, &uri).expect("formats");
        assert!(edit.new_text.contains("description:"), "{}", edit.new_text);
        let reparsed = LowDoc::parse(
            Uri::parse(YAML_URI).unwrap(),
            suspect_source::Source::from_vec(edit.new_text.into_bytes()),
        );
        // The empty-valued key survives as null on both sides.
        assert_eq!(
            OverlayValue::from_node(reparsed.root()),
            OverlayValue::from_node(d.low.root())
        );
    }
}
