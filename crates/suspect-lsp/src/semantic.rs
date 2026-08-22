//! Semantic tokens and inlay hints, computed from the CST.
//!
//! Semantic tokens classify mapping keys (`$ref`/`operationId` keywords,
//! HTTP methods, component schema names, property names, top-level root
//! keys), numeric and `pattern` scalar values, and comments. Plain string
//! scalars are deliberately left untokenized: in OpenAPI documents nearly
//! every value is a string, so tokenizing them is pure noise.
//!
//! Inlay hints annotate every `$ref` value with the resolved target name
//! (`→ Pet (schemas.yaml)`) and every property key inside
//! `components/schemas` with its declared type set (`: string|null`).

use suspect_low::{NodeRef, SpecFamily, ValueKind};
use suspect_oas::TypeSet;
use suspect_ref::{Resolution, Workspace};
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::{
    DocumentHighlight, DocumentHighlightKind, InlayHint, InlayHintKind, InlayHintLabel,
    InlayHintTooltip, Position, Range, SelectionRange, SemanticToken, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens,
};

use crate::navigation::{excerpt, rederive};
use crate::state::{OpenDoc, lsp_range, offset_of_utf16};

/// Token types emitted by [`semantic_tokens_full`], in legend order.
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::TYPE,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::METHOD,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::NUMBER,
    SemanticTokenType::REGEXP,
    SemanticTokenType::COMMENT,
];

/// Token modifiers emitted by [`semantic_tokens_full`], in legend order.
pub const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[SemanticTokenModifier::DEFINITION];

// Legend indices into [`TOKEN_TYPES`].
const NAMESPACE: u32 = 0;
const TYPE: u32 = 1;
const KEYWORD: u32 = 2;
const METHOD: u32 = 3;
const PROPERTY: u32 = 4;
const NUMBER: u32 = 5;
const REGEXP: u32 = 6;
const COMMENT: u32 = 7;

/// Modifier bit for component schema names (`definition`).
const DEFINITION: u32 = 1;

/// Keys highlighted as keywords wherever they appear.
const KEYWORDS: &[&str] = &[
    "$ref",
    "operationId",
    "operationPath",
    "webhooks",
    "openapi",
    "swagger",
    "arazzo",
    "overlay",
];

/// HTTP method keys highlighted as methods.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Upper bound on inlay hints per document, so pathological inputs cannot
/// flood the client.
const MAX_HINTS: usize = 500;

/// One raw token before delta encoding: absolute position, UTF-16 length,
/// legend type index, and modifier bitmask.
struct RawToken {
    line: u32,
    col: u32,
    len: u32,
    ty: u32,
    mods: u32,
}

/// The token legend advertised in the server capabilities.
#[must_use]
pub fn legend() -> tower_lsp::lsp_types::SemanticTokensLegend {
    tower_lsp::lsp_types::SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Full-document semantic tokens for an open document.
///
/// Documents whose spec family is [`SpecFamily::Unknown`] produce no tokens
/// at all — the highlighter is tuned to OpenAPI/Arazzo/Overlay shapes.
#[must_use]
pub fn semantic_tokens_full(doc: &OpenDoc) -> SemanticTokens {
    SemanticTokens {
        result_id: None,
        data: collect_tokens(doc),
    }
}

/// Collects and delta-encodes all tokens of `doc`.
fn collect_tokens(doc: &OpenDoc) -> Vec<SemanticToken> {
    let mut raw: Vec<RawToken> = Vec::new();
    if !matches!(doc.low.sniff_family(), SpecFamily::Unknown) {
        let inner = doc.low.inner();
        let (bytes, li) = (inner.bytes(), inner.line_index());
        for n in inner.root().descendants() {
            match n.kind() {
                SyntaxKind::Comment => push_token(&mut raw, bytes, li, &n.content(), COMMENT, 0),
                SyntaxKind::Pair => classify_pair(&mut raw, bytes, li, n),
                _ => {}
            }
        }
    }
    encode(&mut raw)
}

/// Emits the key token and any scalar-value token for one mapping pair.
fn classify_pair(
    raw: &mut Vec<RawToken>,
    bytes: &[u8],
    li: &suspect_source::LineIndex,
    pair: SNode<'_>,
) {
    let (Some(key), Some(value)) = (pair.child_by_field("key"), pair.child_by_field("value"))
    else {
        return;
    };
    let kc = key.content();
    let key_text = String::from_utf8_lossy(kc.scalar_bytes()).into_owned();

    let ty = key_type(NodeRef::new(pair).path_from_root().tokens(), &key_text);
    if let Some(ty) = ty {
        let mods = if ty == TYPE { DEFINITION } else { 0 };
        push_token(raw, bytes, li, &kc, ty, mods);
    }

    // Scalar values: numbers and `pattern` regexes only; strings stay clean.
    let vc = value.content();
    if key_text == "pattern" {
        push_token(raw, bytes, li, &vc, REGEXP, 0);
    } else if matches!(NodeRef::new(vc).kind(), ValueKind::Int | ValueKind::Float) {
        push_token(raw, bytes, li, &vc, NUMBER, 0);
    }
}

/// Token type for a mapping key, given its root-pointer tokens and text.
fn key_type(path_tokens: &[std::boxed::Box<str>], key_text: &str) -> Option<u32> {
    if KEYWORDS.contains(&key_text) {
        return Some(KEYWORD);
    }
    if METHODS.contains(&key_text) {
        return Some(METHOD);
    }
    if path_tokens.len() == 1 {
        return Some(NAMESPACE);
    }
    if path_tokens.len() == 3
        && path_tokens[0].as_ref() == "components"
        && path_tokens[1].as_ref() == "schemas"
    {
        return Some(TYPE);
    }
    if path_tokens.len() >= 2 && path_tokens[path_tokens.len() - 2].as_ref() == "properties" {
        return Some(PROPERTY);
    }
    None
}

/// Appends one token unless the node spans multiple lines (keys, numbers,
/// and regexes never do; this guards against block scalars).
fn push_token(
    raw: &mut Vec<RawToken>,
    bytes: &[u8],
    li: &suspect_source::LineIndex,
    node: &SNode<'_>,
    ty: u32,
    mods: u32,
) {
    let r = node.byte_range();
    if bytes[r.start..r.end].contains(&b'\n') {
        return;
    }
    let (line, col) = li.line_col_utf16(bytes, r.start);
    let len: u32 = node
        .text_lossy()
        .chars()
        .map(char::len_utf16)
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX);
    raw.push(RawToken {
        line,
        col,
        len,
        ty,
        mods,
    });
}

/// Sorts by position and delta-encodes per the LSP spec.
fn encode(raw: &mut [RawToken]) -> Vec<SemanticToken> {
    raw.sort_by_key(|t| (t.line, t.col));
    let mut out = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_col = 0u32;
    for t in raw.iter() {
        let delta_line = t.line - prev_line;
        let delta_start = if delta_line == 0 {
            t.col - prev_col
        } else {
            t.col
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: t.len,
            token_type: t.ty,
            token_modifiers_bitset: t.mods,
        });
        prev_line = t.line;
        prev_col = t.col;
    }
    out
}

/// Inlay hints for `$ref` targets and property types within `range`.
#[must_use]
pub fn inlay_hints(doc: &OpenDoc, ws: &Workspace, range: Range) -> Vec<InlayHint> {
    let Some(handle) = ws.get(doc.low.uri()) else {
        return Vec::new();
    };
    let inner = doc.low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    let mut out: Vec<InlayHint> = Vec::new();
    for n in inner.root().descendants() {
        if out.len() >= MAX_HINTS {
            break;
        }
        if n.kind() != SyntaxKind::Pair {
            continue;
        }
        let (Some(key), Some(value)) = (n.child_by_field("key"), n.child_by_field("value")) else {
            continue;
        };
        let key_text = String::from_utf8_lossy(key.content().scalar_bytes());
        if key_text == "$ref" {
            let pos = end_position(bytes, li, &value.content());
            if !position_in_range(pos, &range) {
                continue;
            }
            if let Some(hint) = ref_hint(&handle, ws, &value.content(), bytes, li) {
                out.push(hint);
            }
        } else if key_text == "properties" {
            // `: type` hints for property keys inside schema property maps
            if let Some(types) = property_type_hint(&handle, &NodeRef::new(n)) {
                let kc = key.content();
                let pos = end_position(bytes, li, &kc);
                if !position_in_range(pos, &range) {
                    continue;
                }
                out.push(InlayHint {
                    position: pos,
                    label: InlayHintLabel::String(format!(": {types}")),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                });
            }
        }
    }
    out.truncate(MAX_HINTS);
    out
}

/// Builds the `→ Target (file)` hint for one `$ref` value node.
fn ref_hint(
    handle: &suspect_ref::DocHandle<'_>,
    ws: &Workspace,
    value: &SNode<'_>,
    bytes: &[u8],
    li: &suspect_source::LineIndex,
) -> Option<InlayHint> {
    let node = rederive(handle, value.byte_range())?;
    let (label, tooltip_source) = match handle.resolve_ref_value(node) {
        Ok(Resolution::Node(target)) => {
            let name = target
                .path_from_root()
                .tokens()
                .last()
                .map_or_else(|| "(root)".to_owned(), |t| t.to_string());
            let file = basename(target.syntax().doc().uri().as_str());
            let excerpt = excerpt(target.syntax().doc().bytes(), target.byte_range(), 10);
            (format!("→ {name} ({file})"), excerpt)
        }
        Ok(Resolution::WholeDoc(id)) => {
            let uri = ws
                .uris()
                .into_iter()
                .find(|u| ws.get(u).is_some_and(|h| h.id() == id))?;
            let file = basename(uri.as_str());
            let doc_bytes = ws.get(&uri)?.doc().inner().bytes().to_vec();
            (
                format!("→ {file}"),
                excerpt(&doc_bytes, 0..doc_bytes.len(), 10),
            )
        }
        Ok(Resolution::Cycle { .. }) | Err(_) => return None,
    };
    let lang = match value.doc().format() {
        suspect_syntax::Format::Json => "json",
        suspect_syntax::Format::Yaml => "yaml",
    };
    Some(InlayHint {
        position: end_position(bytes, li, value),
        label: InlayHintLabel::String(label),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: Some(InlayHintTooltip::String(format!(
            "```{lang}\n{tooltip_source}\n```"
        ))),
        padding_left: None,
        padding_right: Some(true),
        data: None,
    })
}

/// Rendered type set (e.g. `string|null`) for a property key pair, or `None`
/// when the pair is not a property inside `components/schemas` or its schema
/// declares no type.
fn property_type_hint(handle: &suspect_ref::DocHandle<'_>, pair: &NodeRef<'_>) -> Option<String> {
    let pointer = pair.path_from_root();
    let tokens = pointer.tokens();
    if tokens.len() < 5
        || tokens[0].as_ref() != "components"
        || tokens[1].as_ref() != "schemas"
        || tokens[3].as_ref() != "properties"
    {
        return None;
    }
    let value = NodeRef::new(pair.syntax().child_by_field("value")?.content());
    // Follow one `$ref` hop so the hint shows the target's declared type.
    let target = match value.get("$ref") {
        Some(_) => {
            let node = rederive(handle, value.byte_range())?;
            match handle.resolve_ref_value(node) {
                Ok(Resolution::Node(t)) => t,
                _ => return None,
            }
        }
        None => value,
    };
    render_types(type_mask(&target))
}

/// Type bitmask declared directly on a schema node (with object/array
/// inference and 3.0-style `nullable`).
fn type_mask(schema: &NodeRef<'_>) -> u8 {
    let mut set = 0u8;
    match schema.get("type") {
        Some(t) => match t.kind() {
            ValueKind::Str => insert_type_str(&mut set, t.as_str()),
            ValueKind::Array => {
                for item in t.items() {
                    insert_type_str(&mut set, item.as_str());
                }
            }
            _ => {}
        },
        None => {
            if schema.get("properties").is_some()
                || schema.get("additionalProperties").is_some()
                || schema.get("required").is_some()
                || schema.get("patternProperties").is_some()
            {
                set |= TypeSet::OBJECT;
            }
            if schema.get("items").is_some() || schema.get("prefixItems").is_some() {
                set |= TypeSet::ARRAY;
            }
        }
    }
    if schema
        .get("nullable")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        set |= TypeSet::NULL;
    }
    set
}

/// Folds one `type` spelling into the bitmask.
fn insert_type_str(set: &mut u8, s: Option<&str>) {
    match s {
        Some("null") => *set |= TypeSet::NULL,
        Some("boolean") => *set |= TypeSet::BOOL,
        Some("object") => *set |= TypeSet::OBJECT,
        Some("array") => *set |= TypeSet::ARRAY,
        Some("number") => *set |= TypeSet::NUMBER,
        Some("integer") => *set |= TypeSet::INTEGER,
        Some("string") => *set |= TypeSet::STRING,
        _ => {}
    }
}

/// Renders a type bitmask in display order, `None` when empty.
fn render_types(set: u8) -> Option<String> {
    const ORDER: &[(u8, &str)] = &[
        (TypeSet::STRING, "string"),
        (TypeSet::INTEGER, "integer"),
        (TypeSet::NUMBER, "number"),
        (TypeSet::BOOL, "boolean"),
        (TypeSet::OBJECT, "object"),
        (TypeSet::ARRAY, "array"),
        (TypeSet::NULL, "null"),
    ];
    let names: Vec<&str> = ORDER
        .iter()
        .filter(|(bit, _)| set & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    (!names.is_empty()).then(|| names.join("|"))
}

/// UTF-16 end position of a node.
fn end_position(bytes: &[u8], li: &suspect_source::LineIndex, node: &SNode<'_>) -> Position {
    let (line, col) = li.line_col_utf16(bytes, node.byte_range().end);
    Position {
        line,
        character: col,
    }
}

/// True when `p` lies within `range` (inclusive bounds).
fn position_in_range(p: Position, range: &Range) -> bool {
    p >= range.start && p <= range.end
}

/// Last path segment of a URI string.
fn basename(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

/// Key node of the mapping-pair enclosing `node` (climbing through wrapper
/// nodes like `block_node`).
fn enclosing_pair_key(mut node: SNode<'_>) -> Option<SNode<'_>> {
    loop {
        if node.kind() == SyntaxKind::Pair {
            return node.child_by_field("key");
        }
        node = node.parent()?;
    }
}

/// Normalized `$ref` target text for a value node: decoded scalar, trimmed.
fn normalized_ref(value: &suspect_syntax::SNode<'_>) -> String {
    let decoded = suspect_low::NodeRef::new(*value).decoded_scalar();
    String::from_utf8_lossy(&decoded).trim().to_owned()
}

/// Highlights every `$ref` occurrence in `doc` that points at the same
/// target as the construct at `offset`, plus the target's declaration key
/// when it lives in this document. The declaration uses the Write kind;
/// references use Text. Empty when `offset` is not on a `$ref` value or a
/// component declaration key.
#[must_use]
pub fn document_highlights(doc: &OpenDoc, offset: usize) -> Vec<DocumentHighlight> {
    let inner = doc.low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());

    let mut wanted: Option<String> =
        crate::navigation::ref_value_node(&doc.low, offset).map(|v| normalized_ref(&v));
    if wanted.is_none() {
        // position may sit on a component declaration key
        if let Some(node) = crate::navigation::node_at(&doc.low, offset)
            && let Some(pair) = node.parent()
        {
            let nr = suspect_low::NodeRef::new(pair);
            let path = nr.path_from_root().to_path();
            if let Some(rest) = path.strip_prefix("/components/") {
                wanted = Some(format!("#/{rest}"));
            }
        }
    }
    let Some(target) = wanted else {
        return Vec::new();
    };
    if std::env::var("SUSPECT_HLDBG").is_ok() {
        eprintln!("HLDBG target={target:?}");
    }

    let mut out: Vec<DocumentHighlight> = Vec::new();
    for pair in inner
        .root()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::Pair)
    {
        let Some(key) = pair.child_by_field("key") else {
            continue;
        };
        if key.scalar_bytes() != b"$ref" {
            continue;
        }
        let Some(value) = pair.child_by_field("value") else {
            continue;
        };
        let value = value.content();
        if normalized_ref(&value) == target {
            eprintln!(
                "HLDBG ref match at {:?} norm={:?}",
                lsp_range(bytes, li, value.byte_range()),
                normalized_ref(&value)
            );
            out.push(DocumentHighlight {
                range: lsp_range(bytes, li, value.byte_range()),
                kind: Some(DocumentHighlightKind::TEXT),
            });
        }
    }

    // declaration key, when the target resolves inside this document
    if let Ok(ptr) = suspect_low::Pointer::parse(&target)
        && let Some(decl) = doc.low.root().pointer(&ptr)
        && let Some(key) = enclosing_pair_key(*decl.syntax())
    {
        out.push(DocumentHighlight {
            range: lsp_range(bytes, li, key.byte_range()),
            kind: Some(DocumentHighlightKind::WRITE),
        });
    }
    out
}

/// LSP selection ranges: the CST ancestor chain from the node at each
/// position up to the document root.
#[must_use]
pub fn selection_ranges(doc: &OpenDoc, positions: &[Position]) -> Vec<SelectionRange> {
    let inner = doc.low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    positions
        .iter()
        .filter_map(|pos| {
            let offset = offset_of_utf16(bytes, li, pos.line, pos.character)?;
            let start = crate::navigation::node_at(&doc.low, offset)?;
            let mut cur = Some(start);
            let mut chain: Vec<suspect_syntax::SNode<'_>> = Vec::new();
            while let Some(n) = cur {
                // skip zero-width duplicates
                if chain
                    .last()
                    .is_none_or(|l| l.byte_range() != n.byte_range())
                {
                    chain.push(n);
                }
                cur = n.parent();
            }
            let mut parent: Option<Box<SelectionRange>> = None;
            for n in chain.iter().rev() {
                parent = Some(Box::new(SelectionRange {
                    range: lsp_range(bytes, li, n.byte_range()),
                    parent,
                }));
            }
            parent.map(|p| *p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
openapi: 3.1.0
info: {title: t, version: "1"}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        name:
          type: string
"#;

    fn open() -> OpenDoc {
        OpenDoc::parse("file:///mem/spec.yaml".into(), SPEC.to_owned())
    }

    #[test]
    fn tokens_fire_for_keywords_methods_types_numbers() {
        let doc = open();
        let tokens = semantic_tokens_full(&doc);
        assert!(!tokens.data.is_empty());
        // decode and classify
        let mut line = 0u32;
        let mut col = 0u32;
        let mut saw_keyword = false;
        let mut saw_method = false;
        let mut saw_type_def = false;
        let mut saw_number = false;
        for t in &tokens.data {
            line += t.delta_line;
            col = if t.delta_line == 0 {
                col + t.delta_start
            } else {
                t.delta_start
            };
            if t.token_type == KEYWORD {
                saw_keyword = true;
            }
            if t.token_type == METHOD {
                saw_method = true;
            }
            if t.token_type == TYPE && t.token_modifiers_bitset & DEFINITION != 0 {
                saw_type_def = true;
            }
            if t.token_type == NUMBER {
                // SPEC has `format: int64` (string) and a quoted version —
                // but `required` arrays etc. contain none; keep the flag for
                // the negative assertion below.
                saw_number = true;
            }
            let _ = (line, col);
        }
        assert!(saw_keyword, "$ref/operationId must be keywords");
        assert!(saw_method, "get must be a method token");
        assert!(saw_type_def, "Pet must be a definition-typed type");
        assert!(!saw_number, "this fixture contains no bare numbers");
    }

    #[test]
    fn unknown_family_yields_no_tokens() {
        let doc = OpenDoc::parse("file:///m/r.yaml".into(), "random: document\n".into());
        assert!(semantic_tokens_full(&doc).data.is_empty());
    }

    #[test]
    fn inlay_hint_shows_ref_target() {
        let dir = std::env::temp_dir().join("suspect-lsp-inlay-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
        let uri = suspect_source::Uri::from_path(&dir.join("spec.yaml")).unwrap();
        let ws = suspect_ref::WorkspaceBuilder::new()
            .root(&dir)
            .build()
            .unwrap();
        ws.load_all(uri.as_str()).unwrap();
        let doc = OpenDoc::parse(uri.clone(), SPEC.to_owned());
        let full = Range {
            start: Position::default(),
            end: Position {
                line: u32::MAX,
                character: u32::MAX,
            },
        };
        let hints = inlay_hints(&doc, &ws, full);
        assert!(!hints.is_empty(), "ref hints must fire");
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            other => panic!("string label expected, got {other:?}"),
        };
        assert!(
            label.contains("\u{2192} Pet"),
            "target name in label: {label}"
        );
    }

    #[test]
    fn document_highlights_finds_refs_and_declaration() {
        let doc = open();
        // the singular-Pet ref (the Pets ref has a different target)
        let off = SPEC
            .rfind("'#/components/schemas/Pet'")
            .map(|i| i + 2)
            .unwrap();
        let highlights = document_highlights(&doc, off);
        // this $ref + the declaration key (the other $ref targets Pets)
        assert_eq!(highlights.len(), 2, "{highlights:?}");
        assert!(
            highlights
                .iter()
                .any(|h| h.kind == Some(DocumentHighlightKind::WRITE)),
            "declaration must be highlighted as Write"
        );
    }

    #[test]
    fn selection_range_chain_reaches_root() {
        let doc = open();
        let off = SPEC.find("listPets").unwrap();
        let pos = Position {
            line: 5,
            character: 20,
        };
        let ranges = selection_ranges(&doc, &[pos]);
        assert_eq!(ranges.len(), 1);
        // walk parents to root; count > 4 for this nesting depth
        let mut count = 1usize;
        let mut cur = &ranges[0];
        while let Some(p) = &cur.parent {
            count += 1;
            cur = p;
        }
        assert!(count > 4, "chain depth {count}");
        let _ = off;
    }
}
