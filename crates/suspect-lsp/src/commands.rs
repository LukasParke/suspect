//! `workspace/executeCommand` payload builders.
//!
//! Deterministic, side-effect-free computations behind the four
//! `suspect.*` commands advertised via `ExecuteCommandOptions`:
//!
//! | command | builder |
//! |---|---|
//! | `suspect.generateExample` | [`generate_example`] |
//! | `suspect.showRefGraph` | [`show_ref_graph`] |
//! | `suspect.breakingChanges` | [`breaking_changes`] |
//! | `suspect.contractCoverage` | [`contract_coverage`] + [`coverage_diagnostics`] |
//!
//! Nothing here touches a [`tower_lsp::Client`]: handlers in `lib.rs` take
//! short-lived read locks on [`crate::state::State`] and call straight into
//! these functions, then decide how to deliver the result (apply an edit,
//! show a document, publish diagnostics).
//!
//! Every function is deterministic: identical inputs produce byte-identical
//! outputs, so golden diffs stay stable. Example synthesis never draws
//! randomness and never reads the clock — all "sample" values are fixed
//! constants.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use suspect_arazzo::ArazzoDoc;
use suspect_low::{LowDoc, NodeRef, Pointer, SpecFamily, ValueKind};
use suspect_ref::{ParsedRef, Resolution, Workspace};
use suspect_source::{LineIndex, Source, Uri};
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::*;

use crate::navigation;
use crate::state::{OpenDoc, lsp_range};

// ---------------------------------------------------------------------------
// suspect.generateExample
// ---------------------------------------------------------------------------

/// Upper bound on example-synthesis nesting. Past this depth expansion stops
/// and a placeholder marker is emitted instead.
const MAX_EXAMPLE_DEPTH: u32 = 8;

/// Fixed sample constants for well-known `format` keywords; chosen once so
/// generated examples never churn between runs.
const SAMPLE_EMAIL: &str = "user@example.com";
const SAMPLE_DATETIME: &str = "2024-01-01T00:00:00Z";
const SAMPLE_UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const SAMPLE_URI: &str = "https://example.com";
const SAMPLE_IPV4: &str = "192.0.2.1";
const SAMPLE_DATE: &str = "2024-01-01";

/// The result of `suspect.generateExample`.
#[derive(Debug, Clone)]
pub struct GeneratedExample {
    /// The synthesized instance rendered as YAML at indentation zero (no
    /// `example:` wrapper). Suitable for hover previews and golden diffs.
    pub yaml_snippet: String,
    /// When computable, a zero-width edit that inserts
    /// `example:\n<indented body>` immediately after the schema node's last
    /// line, indented to align with the schema's own keys.
    pub insert_edit: Option<TextEdit>,
}

/// Synthesizes a deterministic example instance for the schema at
/// `schema_offset` in the live document `doc`, resolving `$ref`s through
/// `ws`.
///
/// Keyword precedence: explicit `example`/`default`/`examples` win, then
/// `enum` (first value), then `format`-aware generators, then type defaults
/// honoring `minimum`/`minLength`/simple anchored `pattern`s. Required
/// properties are always included; with `additionalProperties: false`
/// optional declared properties are dropped. Recursive `$ref` chains emit a
/// `null # circular $ref omitted` marker instead of expanding forever.
///
/// # Errors
/// `"no schema node at cursor position"` when `schema_offset` falls outside
/// any meaningful node (empty buffer, past EOF, comments only).
pub fn generate_example(
    ws: &Workspace,
    doc: &OpenDoc,
    schema_offset: usize,
) -> Result<GeneratedExample, String> {
    if schema_offset >= doc.low.inner().bytes().len() {
        return Err("no schema node at cursor position".to_owned());
    }
    let schema = schema_node_at(&doc.low, schema_offset)
        .ok_or_else(|| "no schema node at cursor position".to_owned())?;

    let mut seen: Vec<String> = Vec::new();
    let value = synth(ws, schema.resolved(), 0, &mut seen);

    let mut rendered = String::new();
    render_into(&value, 0, &mut rendered);
    let yaml_snippet = rendered.trim_end_matches('\n').to_owned();

    let inner = doc.low.inner();
    let (position, pad) = insertion_point(inner.bytes(), inner.line_index(), schema.byte_range());
    let pad_str = " ".repeat(pad);
    let mut new_text = format!("\n{pad_str}example:");
    for line in yaml_snippet.split('\n') {
        new_text.push('\n');
        new_text.push_str(&" ".repeat(pad + 2));
        new_text.push_str(line);
    }

    Ok(GeneratedExample {
        yaml_snippet,
        insert_edit: Some(TextEdit {
            range: Range::new(position, position),
            new_text,
        }),
    })
}

/// Locates the schema object the cursor sits on: anchors keys to their pair
/// values, resolves aliases, climbs out of bare scalars to the smallest
/// enclosing mapping, and treats list-valued keywords (`enum: [...]`) as
/// pointing at their enclosing schema object.
fn schema_node_at<'d>(low: &'d LowDoc, offset: usize) -> Option<NodeRef<'d>> {
    let snode = navigation::node_at(low, offset)?;
    // List-valued schema keywords (`enum: [...]`, `allOf: [...]`, ...): the
    // schema being edited is the pair's *enclosing* mapping, not the array.
    let mut cur = Some(snode);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair {
            let key = n
                .child_by_field("key")
                .and_then(|k| std::str::from_utf8(k.scalar_bytes()).ok())
                .unwrap_or_default();
            if matches!(
                key,
                "enum" | "examples" | "required" | "allOf" | "oneOf" | "anyOf"
            ) && let Some(map) = n.parent()
                && map.kind() == SyntaxKind::Mapping
            {
                return Some(NodeRef::new(map).resolved());
            }
            break;
        }
        cur = n.parent();
    }
    let anchored = NodeRef::new(navigation::value_anchor(snode));
    let node = anchored.resolved();
    Some(match node.kind() {
        ValueKind::Object | ValueKind::Array => node,
        _ => {
            // Bare scalar: the schema object is the nearest enclosing mapping.
            let mut s = *node.syntax();
            loop {
                if s.kind() == SyntaxKind::Mapping {
                    break;
                }
                s = s.parent()?;
            }
            NodeRef::new(s).resolved()
        }
    })
}

/// A synthesized YAML value awaiting rendering.
#[derive(Clone)]
enum Yaml {
    Scalar(String),
    /// Verbatim line content (recursion markers carry inline comments).
    Raw(String),
    /// Sequence of synthesized values.
    Seq(Vec<Yaml>),
    /// Mapping in insertion order.
    Map(Vec<(String, Yaml)>),
}

/// Recursively synthesizes an example from one schema node.
fn synth<'ws>(
    ws: &'ws Workspace,
    schema: NodeRef<'ws>,
    depth: u32,
    seen: &mut Vec<String>,
) -> Yaml {
    let node = schema.resolved();
    if depth > MAX_EXAMPLE_DEPTH {
        return Yaml::Raw(format!(
            "null # nesting deeper than {MAX_EXAMPLE_DEPTH} levels omitted"
        ));
    }

    // $ref: expand through the workspace with a cycle guard keyed by the
    // target's structural identity (doc URI + pointer).
    if let Some(rv) = node.get("$ref") {
        let raw = rv.as_str().unwrap_or_default().to_owned();
        return match deref_ref(ws, rv) {
            Some(target) => {
                let key = node_key(&target);
                if seen.contains(&key) {
                    Yaml::Raw("null # circular $ref omitted".to_owned())
                } else {
                    seen.push(key);
                    let out = synth(ws, target, depth + 1, seen);
                    seen.pop();
                    out
                }
            }
            None => Yaml::Raw(format!("null # unresolvable $ref '{raw}'")),
        };
    }

    // Explicit examples win over synthesis.
    for key in ["example", "default"] {
        if let Some(v) = node.get(key) {
            return node_to_yaml(v, depth);
        }
    }
    if let Some(first) = node
        .get("examples")
        .and_then(|e| e.items().into_iter().next())
    {
        return node_to_yaml(first, depth);
    }
    if let Some(c) = node.get("const") {
        return node_to_yaml(c, depth);
    }
    if let Some(first) = node.get("enum").and_then(|e| e.items().into_iter().next()) {
        return literal_yaml(first);
    }

    // Compositions: merge allOf branches when they synthesize to maps,
    // otherwise fall back to the first non-map branch; oneOf/anyOf pick the
    // first branch deterministically.
    if let Some(all) = node.get("allOf") {
        let parts: Vec<Yaml> = all
            .items()
            .iter()
            .map(|b| synth(ws, *b, depth + 1, seen))
            .collect();
        if let Some(merged) = merge_maps(&parts) {
            return merged;
        }
        if let Some(first) = parts
            .into_iter()
            .find(|p| !matches!(p, Yaml::Map(m) if m.is_empty()))
        {
            return first;
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = node.get(key).and_then(|b| b.items().into_iter().next()) {
            return synth(ws, first, depth + 1, seen);
        }
    }

    // Type dispatch; unknown types infer from shape, falling back to string.
    let types = declared_types(&node);
    let pick = |t: &str| types.iter().any(|cand| cand == t);
    if pick("object") || (types.is_empty() && node.get("properties").is_some()) {
        return synth_object(ws, &node, depth, seen);
    }
    if pick("array") || (types.is_empty() && node.get("items").is_some()) {
        return synth_array(ws, &node, depth, seen);
    }
    if pick("integer") {
        return Yaml::Scalar(
            node.get("minimum")
                .and_then(|n| n.as_i64())
                .unwrap_or(0)
                .to_string(),
        );
    }
    if pick("number") {
        return Yaml::Scalar(
            node.get("minimum")
                .and_then(|n| n.as_f64())
                .unwrap_or(0.0)
                .to_string(),
        );
    }
    if pick("boolean") {
        return Yaml::Scalar("true".to_owned());
    }
    if pick("null") {
        return Yaml::Scalar("null".to_owned());
    }
    synth_string(&node)
}

/// Synthesizes an object: required properties always, optional declared
/// properties unless `additionalProperties: false`.
fn synth_object<'ws>(
    ws: &'ws Workspace,
    node: &NodeRef<'ws>,
    depth: u32,
    seen: &mut Vec<String>,
) -> Yaml {
    let required: Vec<&str> = node
        .get("required")
        .map(|n| n.items().iter().filter_map(|i| i.as_str()).collect())
        .unwrap_or_default();
    let additional_false =
        node.get("additionalProperties").and_then(|n| n.as_bool()) == Some(false);

    let mut out: Vec<(String, Yaml)> = Vec::new();
    if let Some(props) = node.get("properties") {
        for e in props.entries() {
            let Some(v) = e.value else { continue };
            let is_required = required.contains(&e.key);
            if additional_false && !is_required {
                continue;
            }
            out.push((e.key.to_owned(), synth(ws, v, depth + 1, seen)));
        }
    }
    Yaml::Map(out)
}

/// Synthesizes a fixed-length array from `items`, honoring `minItems`
/// (capped at 4 elements to keep examples readable).
fn synth_array<'ws>(
    ws: &'ws Workspace,
    node: &NodeRef<'ws>,
    depth: u32,
    seen: &mut Vec<String>,
) -> Yaml {
    let Some(items) = node.get("items") else {
        return Yaml::Seq(Vec::new());
    };
    let count = node
        .get("minItems")
        .and_then(|n| n.as_i64())
        .unwrap_or(1)
        .clamp(1, 4) as usize;
    Yaml::Seq(
        (0..count)
            .map(|_| synth(ws, items, depth + 1, seen))
            .collect(),
    )
}

/// Synthesizes a string honoring `format` constants, simple anchored
/// `pattern`s, and `minLength`.
fn synth_string(node: &NodeRef<'_>) -> Yaml {
    if let Some(constant) = node
        .get("format")
        .and_then(|f| f.as_str())
        .and_then(format_constant)
    {
        return Yaml::Scalar(yaml_quote(constant));
    }
    let min_len = node
        .get("minLength")
        .and_then(|n| n.as_i64())
        .unwrap_or(0)
        .max(0) as usize;
    if let Some(pattern) = node.get("pattern").and_then(|p| p.as_str())
        && let Some(s) = synth_pattern(pattern, min_len)
    {
        return Yaml::Scalar(yaml_quote(&s));
    }
    let base = "string";
    let s = if base.len() < min_len {
        format!("{}{}", base, "a".repeat(min_len - base.len()))
    } else {
        base.to_owned()
    };
    Yaml::Scalar(yaml_quote(&s))
}

/// Fixed sample strings for well-known formats.
fn format_constant(fmt: &str) -> Option<&'static str> {
    Some(match fmt {
        "email" => SAMPLE_EMAIL,
        "date-time" => SAMPLE_DATETIME,
        "uuid" => SAMPLE_UUID,
        "uri" => SAMPLE_URI,
        "ipv4" => SAMPLE_IPV4,
        "date" => SAMPLE_DATE,
        _ => return None,
    })
}

/// Synthesizes a minimal deterministic string from a *simple* anchored
/// pattern: `^[class]<quantifier>$` with a single character class
/// (`[a-z]`, `[0-9]`, ...) and optional quantifier. Anything more complex
/// yields `None` and the caller falls back to `"string"`.
fn synth_pattern(pattern: &str, min_len: usize) -> Option<String> {
    let inner = pattern.strip_prefix('^')?.strip_suffix('$')?;
    let rest = inner.strip_prefix('[')?;
    let close = rest.find(']')?;
    let sample = class_sample(&rest[..close])?;
    let (lo, _) = quantifier(&rest[close + 1..])?;
    let count = lo.max(min_len).min(64);
    Some(std::iter::repeat_n(sample, count).collect())
}

/// First character a simple char class can produce.
fn class_sample(class: &str) -> Option<char> {
    let cs: Vec<char> = class.chars().collect();
    if cs.len() >= 3 && cs[1] == '-' && cs[0] <= cs[2] {
        Some(cs[0])
    } else {
        cs.first().copied()
    }
}

/// Min/max repetition counts of a simple quantifier suffix.
fn quantifier(q: &str) -> Option<(usize, usize)> {
    match q {
        "" => Some((1, 1)),
        "+" => Some((1, usize::MAX)),
        "*" => Some((0, usize::MAX)),
        "?" => Some((0, 1)),
        _ => {
            let inner = q.strip_prefix('{')?.strip_suffix('}')?;
            if let Ok(n) = inner.parse::<usize>() {
                return Some((n, n));
            }
            let (lo, hi) = inner.split_once(',')?;
            let hi = if hi.is_empty() {
                usize::MAX
            } else {
                hi.parse().ok()?
            };
            Some((lo.parse().ok()?, hi))
        }
    }
}

/// Merges `parts` into one map when every part synthesizes to a map; later
/// branches override earlier keys (first-seen order preserved).
fn merge_maps(parts: &[Yaml]) -> Option<Yaml> {
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, Yaml::Map(_))) {
        return None;
    }
    let mut out: Vec<(String, Yaml)> = Vec::new();
    for part in parts {
        let Yaml::Map(entries) = part else { continue };
        for (k, v) in entries {
            if let Some(slot) = out.iter_mut().find(|(ek, _)| ek == k) {
                slot.1 = v.clone();
            } else {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    Some(Yaml::Map(out))
}

/// `type` keyword as a list of accepted type names.
fn declared_types(node: &NodeRef<'_>) -> Vec<String> {
    match node.get("type") {
        Some(t) => match t.kind() {
            ValueKind::Str => t.as_str().map(|s| vec![s.to_owned()]).unwrap_or_default(),
            ValueKind::Array => t
                .items()
                .iter()
                .filter_map(|i| i.as_str())
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

/// Converts an existing schema node (an `example`/`default` value) into a
/// renderable YAML value, verbatim.
fn node_to_yaml(node: NodeRef<'_>, depth: u32) -> Yaml {
    let node = node.resolved();
    if depth > MAX_EXAMPLE_DEPTH {
        return Yaml::Raw("null # nesting limit reached".to_owned());
    }
    match node.kind() {
        ValueKind::Object => Yaml::Map(
            node.entries()
                .iter()
                .filter_map(|e| Some((e.key.to_owned(), node_to_yaml(e.value?, depth + 1))))
                .collect(),
        ),
        ValueKind::Array => Yaml::Seq(
            node.items()
                .iter()
                .map(|i| node_to_yaml(*i, depth + 1))
                .collect(),
        ),
        _ => literal_yaml(node),
    }
}

/// Renders one scalar node verbatim as a YAML token.
fn literal_yaml(node: NodeRef<'_>) -> Yaml {
    let token = match node.kind() {
        ValueKind::Null => "null".to_owned(),
        ValueKind::Bool => node.as_bool().unwrap_or(false).to_string(),
        ValueKind::Int => node.as_i64().unwrap_or(0).to_string(),
        ValueKind::Float => node.as_f64().unwrap_or(0.0).to_string(),
        ValueKind::Str => yaml_quote(node.as_str().unwrap_or_default()),
        ValueKind::Object | ValueKind::Array => String::new(),
    };
    Yaml::Scalar(token)
}

/// Quotes a string for YAML emission when it is not a safe plain scalar.
fn yaml_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '@'))
        && s.parse::<f64>().is_err()
        && !matches!(
            s,
            "true" | "false" | "null" | "True" | "False" | "Null" | "yes" | "no" | "on" | "off"
        );
    if plain {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

/// Follows one `$ref` value node to its target through the workspace. The
/// entry point may come from the live (dirty) buffer; resolution runs on the
/// workspace copies via the value's structural identity, mirroring how
/// navigation resolves unsaved edits.
fn deref_ref<'ws>(ws: &'ws Workspace, ref_value: NodeRef<'_>) -> Option<NodeRef<'ws>> {
    let uri = ref_value.syntax().doc().uri().clone();
    let handle = ws.get(&uri)?;
    let range = ref_value.syntax().byte_range();
    let inner = handle.doc().inner();
    let mut raw = inner
        .root()
        .raw()
        .descendant_for_byte_range(range.start, range.end.saturating_sub(1))?;
    while raw.byte_range() != range {
        raw = raw.parent()?;
    }
    let node = NodeRef::new(SNode::new(inner, raw));
    match handle.resolve_ref_value(node) {
        Ok(Resolution::Node(target)) => Some(target.resolved()),
        _ => None,
    }
}

/// Stable structural identity of a node: document URI plus pointer path.
fn node_key(node: &NodeRef<'_>) -> String {
    format!(
        "{}#{}",
        node.syntax().doc().uri(),
        node.path_from_root().to_path()
    )
}

/// Renders a [`Yaml`] tree as block-style YAML at `indent`.
fn render_into(value: &Yaml, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match value {
        Yaml::Scalar(s) | Yaml::Raw(s) => {
            out.push_str(&pad);
            out.push_str(s);
            out.push('\n');
        }
        Yaml::Seq(items) => {
            if items.is_empty() {
                out.push_str(&pad);
                out.push_str("[]\n");
            }
            for item in items {
                match item {
                    Yaml::Scalar(s) | Yaml::Raw(s) => {
                        out.push_str(&pad);
                        out.push_str("- ");
                        out.push_str(s);
                        out.push('\n');
                    }
                    Yaml::Seq(_) => {
                        out.push_str(&pad);
                        out.push_str("-\n");
                        render_into(item, indent + 1, out);
                    }
                    Yaml::Map(entries) => {
                        if entries.is_empty() {
                            out.push_str(&pad);
                            out.push_str("- {}\n");
                            continue;
                        }
                        // First key rides the dash; continuations align under it.
                        out.push_str(&pad);
                        out.push_str("- ");
                        let cont = "  ".repeat(indent + 1);
                        for (i, (k, v)) in entries.iter().enumerate() {
                            if i > 0 {
                                out.push_str(&cont);
                            }
                            match v {
                                Yaml::Scalar(s) | Yaml::Raw(s) => {
                                    out.push_str(k);
                                    out.push_str(": ");
                                    out.push_str(s);
                                    out.push('\n');
                                }
                                Yaml::Map(m) if m.is_empty() => {
                                    out.push_str(k);
                                    out.push_str(": {}\n");
                                }
                                Yaml::Seq(s) if s.is_empty() => {
                                    out.push_str(k);
                                    out.push_str(": []\n");
                                }
                                _ => {
                                    out.push_str(k);
                                    out.push_str(":\n");
                                    render_into(v, indent + 2, out);
                                }
                            }
                        }
                    }
                }
            }
        }
        Yaml::Map(entries) => {
            if entries.is_empty() {
                out.push_str(&pad);
                out.push_str("{}\n");
            }
            for (k, v) in entries {
                match v {
                    Yaml::Scalar(s) | Yaml::Raw(s) => {
                        out.push_str(&pad);
                        out.push_str(k);
                        out.push_str(": ");
                        out.push_str(s);
                        out.push('\n');
                    }
                    Yaml::Map(m) if m.is_empty() => {
                        out.push_str(&pad);
                        out.push_str(k);
                        out.push_str(": {}\n");
                    }
                    Yaml::Seq(i) if i.is_empty() => {
                        out.push_str(&pad);
                        out.push_str(k);
                        out.push_str(": []\n");
                    }
                    _ => {
                        out.push_str(&pad);
                        out.push_str(k);
                        out.push_str(":\n");
                        render_into(v, indent + 1, out);
                    }
                }
            }
        }
    }
}

/// Computes where the `example:` block is inserted: end of the schema node's
/// last line, with the indentation of the node's first line as the block pad.
fn insertion_point(
    bytes: &[u8],
    li: &LineIndex,
    range: std::ops::Range<usize>,
) -> (Position, usize) {
    let (start_line, _) = li.line_col(bytes, range.start);
    let pad = li
        .line_range(bytes, start_line)
        .map(|r| {
            bytes[r.start..r.end]
                .iter()
                .take_while(|&&b| b == b' ' || b == b'\t')
                .count()
        })
        .unwrap_or(0);
    let last = range
        .end
        .saturating_sub(1)
        .min(bytes.len().saturating_sub(1));
    let (end_line, _) = li.line_col(bytes, last);
    let content_end = li
        .line_range(bytes, end_line)
        .map(|r| {
            let mut e = r.end.min(bytes.len());
            while e > r.start && (bytes[e - 1] == b'\n' || bytes[e - 1] == b'\r') {
                e -= 1;
            }
            e
        })
        .unwrap_or(range.end);
    let (_, col_utf16) = li.line_col_utf16(bytes, content_end);
    (
        Position {
            line: end_line,
            character: col_utf16,
        },
        pad,
    )
}

// ---------------------------------------------------------------------------
// suspect.showRefGraph
// ---------------------------------------------------------------------------

/// Maximum number of nodes drawn before the remainder is summarized in an
/// omission comment.
const GRAPH_NODE_CAP: usize = 200;

/// Renders the workspace `$ref` graph as a Mermaid `graph LR` scratchpad:
/// documents/components as nodes, reference edges as arcs labeled with the
/// target pointer's last segment (`$ref` for whole-document targets).
///
/// Multi-edge reference cycles found by the per-document cycle census are
/// listed up front as `%% cycle:` comment lines. Ordering is fully
/// deterministic (nodes and edges sorted by URI + pointer), and the node
/// list is capped at 200 with the remainder noted in an omission comment so
/// huge workspaces still render.
#[must_use]
pub fn show_ref_graph(ws: &Workspace) -> String {
    let uris = ws.uris();

    // DocId → URI for labeling cycle members.
    let mut id_to_uri: HashMap<usize, Uri> = HashMap::new();
    for uri in &uris {
        if let Some(h) = ws.get(uri) {
            id_to_uri.insert(h.id(), uri.clone());
        }
    }

    // Raw node keys ("uri#ptr") → human labels; BTreeMap keeps ordering
    // deterministic.
    let mut nodes: BTreeMap<String, String> = BTreeMap::new();
    let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();
    for uri in &uris {
        let Some(handle) = ws.get(uri) else { continue };
        let fname = file_name(uri.as_str()).to_owned();
        nodes.insert((*uri.as_str()).to_owned(), fname.clone());
        for e in handle.edges().iter() {
            let src_key = format!("{}#{}", uri.as_str(), e.path.to_path());
            let src_label = match e.path.tokens().last() {
                Some(t) => format!("{fname}#{t}"),
                None => fname.clone(),
            };
            nodes.insert(src_key.clone(), src_label);

            let (tgt_key, tgt_label) = match &e.parsed {
                ParsedRef::Local(p) => pointer_target(uri.as_str(), p),
                ParsedRef::External { uri: tu, pointer } => pointer_target(tu.as_str(), pointer),
                ParsedRef::PlainName(name) => (format!("{uri}#{name}"), format!("{fname}#{name}")),
            };
            nodes.entry(tgt_key.clone()).or_insert_with(|| tgt_label);
            let label = edge_label(&e.parsed);
            edges.insert((src_key, tgt_key, label));
        }
    }

    let mut out = String::from("graph LR\n");

    // Cycle markers first: multi-edge loops per document census.
    let mut cycle_lines: BTreeSet<String> = BTreeSet::new();
    for uri in &uris {
        let Some(handle) = ws.get(uri) else { continue };
        for cycle in handle.cycles().cycles {
            if cycle.steps.len() <= 1 {
                continue;
            }
            // Steps land on `$ref` value nodes; the meaningful member name
            // is the *containing mapping's* last pointer segment, so match
            // each step back to its edge.
            let edges = handle.edges();
            let mut members: Vec<String> = cycle
                .steps
                .iter()
                .filter_map(|s| {
                    let du = id_to_uri.get(&s.doc)?;
                    let edge = edges.iter().find(|e| e.at == s.at)?;
                    let fname = file_name(du.as_str());
                    Some(match edge.path.tokens().last() {
                        Some(t) => format!("{fname}#{t}"),
                        None => fname.to_owned(),
                    })
                })
                .collect();
            members.sort();
            members.dedup();
            if members.len() > 1 {
                cycle_lines.insert(format!("%% cycle: {}", members.join(" <-> ")));
            }
        }
    }
    for line in &cycle_lines {
        out.push_str(line);
        out.push('\n');
    }

    if ws.is_empty() {
        out.push_str("%% no documents loaded\n");
        return out;
    }

    // Apply the node cap, then draw only edges whose endpoints survived.
    let total = nodes.len();
    let kept: BTreeSet<&String> = nodes.keys().take(GRAPH_NODE_CAP).collect();
    if total > GRAPH_NODE_CAP {
        out.push_str(&format!(
            "%% {} nodes omitted (cap {GRAPH_NODE_CAP})\n",
            total - GRAPH_NODE_CAP
        ));
    }
    // Unique mermaid ids: sanitized keys, disambiguated on collision.
    let mut ids: HashMap<&String, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for key in &kept {
        let mut id = sanitize_id(key);
        let mut n = 2;
        while !used.insert(id.clone()) {
            id = format!("{}_{}", sanitize_id(key), n);
            n += 1;
        }
        ids.insert(key, id);
    }
    for (src, tgt, label) in &edges {
        let (Some(sid), Some(tid)) = (ids.get(src), ids.get(tgt)) else {
            continue;
        };
        let slabel = mermaid_label(&nodes[src]);
        let tlabel = mermaid_label(&nodes[tgt]);
        out.push_str(&format!(
            "  {sid}[\"{slabel}\"] -->|\"{label}\"| {tid}[\"{tlabel}\"]\n"
        ));
    }
    out
}

/// Node key and label for a pointer target within one document.
fn pointer_target(doc_uri: &str, pointer: &Pointer) -> (String, String) {
    let fname = file_name(doc_uri).to_owned();
    match pointer.tokens().last() {
        Some(t) => (
            format!("{doc_uri}#{}", pointer.to_path()),
            format!("{fname}#{t}"),
        ),
        None => (doc_uri.to_owned(), fname),
    }
}

/// Edge label: the target pointer's last segment, or `$ref` for roots /
/// plain-name targets.
fn edge_label(parsed: &ParsedRef) -> String {
    match parsed {
        ParsedRef::Local(p) | ParsedRef::External { pointer: p, .. } => p
            .tokens()
            .last()
            .map_or_else(|| "$ref".to_owned(), |t| t.to_string()),
        ParsedRef::PlainName(name) => format!("#{name}"),
    }
}

/// Last path segment of a URI string.
fn file_name(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

/// Mermaid-safe node id: every non-alphanumeric byte becomes `_`.
fn sanitize_id(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Escapes a label for use inside Mermaid double quotes.
fn mermaid_label(label: &str) -> String {
    label.replace('\\', "\\\\").replace('"', "'")
}

// ---------------------------------------------------------------------------
// suspect.breakingChanges
// ---------------------------------------------------------------------------

/// One contract-breaking difference between an old spec revision and the
/// current workspace state.
#[derive(Debug, Clone)]
pub struct BreakingChange {
    /// ERROR for removals and newly-required fields, WARNING for constraint
    /// tightening.
    pub severity: DiagnosticSeverity,
    /// Current document URI the finding belongs to.
    pub uri: String,
    /// Location inside the current document; the definition site when
    /// identifiable, otherwise the file start.
    pub range: Range,
    /// Human-readable description of the break.
    pub message: String,
}

/// Per-document context used while reporting findings against the *current*
/// copy of a document.
struct DiffCtx<'a> {
    uri: &'a str,
    bytes: &'a [u8],
    li: &'a LineIndex,
}

/// Compares every spec present in **both** `old_text_by_uri` and the current
/// workspace, flagging consumer-visible breaks:
///
/// - removals (path, operation, response status, component schema, required
///   property, enum value) — [`DiagnosticSeverity::ERROR`];
/// - newly-required properties — [`DiagnosticSeverity::ERROR`];
/// - constraint tightening (type change, `minimum`/`maximum` tightened,
///   `minLength` raised, `maxLength` lowered) —
///   [`DiagnosticSeverity::WARNING`].
///
/// Schema bodies are compared by walking the parsed trees (local `$ref`s are
/// followed on both sides), matched by stable identity: path + method +
/// response code + media type, and component-schema name + property chain.
/// Old revisions absent from the workspace are ignored.
#[must_use]
pub fn breaking_changes(
    ws: &Workspace,
    old_text_by_uri: &HashMap<String, String>,
) -> Vec<BreakingChange> {
    let mut out = Vec::new();
    for uri in ws.uris() {
        let Some(old_text) = old_text_by_uri.get(uri.as_str()) else {
            continue;
        };
        let Some(handle) = ws.get(&uri) else { continue };
        let Ok(old_uri) = Uri::parse(uri.as_str()) else {
            continue;
        };
        let old_doc = LowDoc::parse(old_uri, Source::from_vec(old_text.as_bytes().to_vec()));
        let cur = handle.doc();
        let ctx = DiffCtx {
            uri: uri.as_str(),
            bytes: cur.inner().bytes(),
            li: cur.inner().line_index(),
        };
        diff_docs(&old_doc, cur, &ctx, &mut out);
    }
    out
}

/// Pushes one finding located at a current-document byte range.
fn report(
    out: &mut Vec<BreakingChange>,
    ctx: &DiffCtx<'_>,
    severity: DiagnosticSeverity,
    at: std::ops::Range<usize>,
    message: String,
) {
    out.push(BreakingChange {
        severity,
        uri: ctx.uri.to_owned(),
        range: lsp_range(ctx.bytes, ctx.li, at),
        message,
    });
}

/// Pushes one finding pinned to the file start (removals have no surviving
/// current location).
fn report_start(
    out: &mut Vec<BreakingChange>,
    ctx: &DiffCtx<'_>,
    severity: DiagnosticSeverity,
    message: String,
) {
    report(out, ctx, severity, 0..0, message);
}

/// Top-level diff: `paths` and `components/schemas`.
fn diff_docs(old: &LowDoc, cur: &LowDoc, ctx: &DiffCtx<'_>, out: &mut Vec<BreakingChange>) {
    let o = old.root();
    let c = cur.root();

    if let (Some(op), Some(cp)) = (o.get("paths"), c.get("paths")) {
        let cur_items: BTreeMap<&str, NodeRef<'_>> = cp
            .entries()
            .into_iter()
            .filter_map(|e| Some((e.key, e.value?)))
            .collect();
        for oe in op.entries() {
            let Some(ov) = oe.value else { continue };
            match cur_items.get(oe.key) {
                None => report_start(
                    out,
                    ctx,
                    DiagnosticSeverity::ERROR,
                    format!("path '{}' removed", oe.key),
                ),
                Some(cv) => diff_path_item(ov, *cv, oe.key, ctx, out),
            }
        }
    }

    let Some(oschemas) = o.get("components").and_then(|x| x.get("schemas")) else {
        return;
    };
    let Some(cschemas) = c.get("components").and_then(|x| x.get("schemas")) else {
        return;
    };
    let cur_schemas: BTreeMap<&str, NodeRef<'_>> = cschemas
        .entries()
        .into_iter()
        .filter_map(|e| Some((e.key, e.value?)))
        .collect();
    for oe in oschemas.entries() {
        let Some(ov) = oe.value else { continue };
        match cur_schemas.get(oe.key) {
            None => report_start(
                out,
                ctx,
                DiagnosticSeverity::ERROR,
                format!("component schema '{}' removed", oe.key),
            ),
            Some(cv) => diff_schema(ov, *cv, oe.key.to_owned(), ctx, out),
        }
    }
}

/// HTTP method keys compared between revisions, canonical order.
const DIFF_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Diffs one shared path item operation-by-operation.
fn diff_path_item(
    o: NodeRef<'_>,
    c: NodeRef<'_>,
    path: &str,
    ctx: &DiffCtx<'_>,
    out: &mut Vec<BreakingChange>,
) {
    for method in DIFF_METHODS {
        let upper = method.to_uppercase();
        match (o.get(method), c.get(method)) {
            (Some(_), None) => report_start(
                out,
                ctx,
                DiagnosticSeverity::ERROR,
                format!("{upper} {path}: operation removed"),
            ),
            (Some(on), Some(cn)) => diff_operation(on, cn, &upper, path, ctx, out),
            (None, _) => {}
        }
    }
}

/// Diffs one shared operation: response status codes and request/response
/// body schemas.
fn diff_operation(
    o: NodeRef<'_>,
    c: NodeRef<'_>,
    method: &str,
    path: &str,
    ctx: &DiffCtx<'_>,
    out: &mut Vec<BreakingChange>,
) {
    let head = format!("{method} {path}");
    if let (Some(or_), Some(cr_)) = (o.get("responses"), c.get("responses")) {
        let cur_resps: BTreeMap<&str, NodeRef<'_>> = cr_
            .entries()
            .into_iter()
            .filter_map(|e| Some((e.key, e.value?)))
            .collect();
        for re in or_.entries() {
            let Some(ov) = re.value else { continue };
            match cur_resps.get(re.key) {
                None => {
                    if re.key != "default" {
                        report_start(
                            out,
                            ctx,
                            DiagnosticSeverity::ERROR,
                            format!("{head}: response '{}' removed", re.key),
                        );
                    }
                }
                Some(cv) => diff_media_schemas(ov, *cv, &format!("{head}/{}", re.key), ctx, out),
            }
        }
    }
    if let (Some(ob), Some(cb)) = (o.get("requestBody"), c.get("requestBody")) {
        diff_media_schemas(ob, cb, &format!("{head}/requestBody"), ctx, out);
    }
}

/// Diffs the `content.<media-type>.schema` pairs two response/request-body
/// objects share.
fn diff_media_schemas(
    o: NodeRef<'_>,
    c: NodeRef<'_>,
    chain: &str,
    ctx: &DiffCtx<'_>,
    out: &mut Vec<BreakingChange>,
) {
    let (Some(oc), Some(cc)) = (o.get("content"), c.get("content")) else {
        return;
    };
    let cur_types: BTreeMap<&str, NodeRef<'_>> = cc
        .entries()
        .into_iter()
        .filter_map(|e| Some((e.key, e.value?)))
        .collect();
    for me in oc.entries() {
        let Some(ov) = me.value else { continue };
        let Some(cv) = cur_types.get(me.key) else {
            continue;
        };
        if let (Some(os), Some(cs)) = (ov.get("schema"), cv.get("schema")) {
            diff_schema(os, cs, format!("{chain}/{}/schema", me.key), ctx, out);
        }
    }
}

/// Follows matching local `$ref`s on both sides (bounded), then applies the
/// keyword-level narrowing rules and recurses into shared sub-schemas.
fn diff_schema(
    o: NodeRef<'_>,
    c: NodeRef<'_>,
    chain: String,
    ctx: &DiffCtx<'_>,
    out: &mut Vec<BreakingChange>,
) {
    let (o, c) = resolve_pair(o, c);

    // Type change (any change narrows some producers).
    if let (Some(ot), Some(ct)) = (o.get("type"), c.get("type"))
        && ot.kind() == ValueKind::Str
        && ct.kind() == ValueKind::Str
        && ot.scalar_bytes() != ct.scalar_bytes()
    {
        report(
            out,
            ctx,
            DiagnosticSeverity::WARNING,
            ct.byte_range(),
            format!(
                "{chain}: type changed from '{}' to '{}'",
                ot.as_str().unwrap_or_default(),
                ct.as_str().unwrap_or_default()
            ),
        );
    }

    // Enum narrowing: every value consumers could send that disappeared.
    if let (Some(oe), Some(ce)) = (o.get("enum"), c.get("enum"))
        && oe.kind() == ValueKind::Array
        && ce.kind() == ValueKind::Array
    {
        let cur_values: HashSet<Vec<u8>> = ce.items().iter().map(|i| decoded(i)).collect();
        for oi in oe.items() {
            if !cur_values.contains(&decoded(&oi)) {
                report(
                    out,
                    ctx,
                    DiagnosticSeverity::ERROR,
                    ce.byte_range(),
                    format!(
                        "{chain}: enum value '{}' removed",
                        String::from_utf8_lossy(&decoded(&oi))
                    ),
                );
            }
        }
    }

    // Numeric bound tightening.
    tighten_f64(out, ctx, &o, &c, &chain, "minimum", false);
    tighten_f64(out, ctx, &o, &c, &chain, "maximum", true);
    // Length bound tightening.
    tighten_i64(out, ctx, &o, &c, &chain, "minLength", false);
    tighten_i64(out, ctx, &o, &c, &chain, "maxLength", true);

    // Required-set delta.
    let oreq = required_set(&o);
    let creq = required_set(&c);
    let creq_node_range = c
        .get("required")
        .map_or_else(|| c.byte_range(), |n| n.byte_range());
    for name in oreq.difference(&creq) {
        report(
            out,
            ctx,
            DiagnosticSeverity::ERROR,
            c.byte_range(),
            format!("{chain}: required property '{name}' removed"),
        );
    }
    for name in creq.difference(&oreq) {
        report(
            out,
            ctx,
            DiagnosticSeverity::ERROR,
            creq_node_range.clone(),
            format!("{chain}: property '{name}' added to required"),
        );
    }

    // Recurse into shared structure.
    if let (Some(op), Some(cp)) = (o.get("properties"), c.get("properties")) {
        let cur_props: BTreeMap<&str, NodeRef<'_>> = cp
            .entries()
            .into_iter()
            .filter_map(|e| Some((e.key, e.value?)))
            .collect();
        for pe in op.entries() {
            let Some(pv) = pe.value else { continue };
            if let Some(cpv) = cur_props.get(pe.key) {
                diff_schema(pv, *cpv, format!("{chain}.{}", pe.key), ctx, out);
            }
        }
    }
    if let (Some(oi), Some(ci)) = (o.get("items"), c.get("items")) {
        diff_schema(oi, ci, format!("{chain}[]"), ctx, out);
    }
    if let (Some(oa), Some(ca)) = (o.get("allOf"), c.get("allOf"))
        && oa.kind() == ValueKind::Array
        && ca.kind() == ValueKind::Array
    {
        for (i, (ob, cb)) in oa.items().iter().zip(ca.items().iter()).enumerate() {
            diff_schema(*ob, *cb, format!("{chain}.allOf[{i}]"), ctx, out);
        }
    }
}

/// Reports numeric bound tightening: `minimum` raised / `maximum` lowered.
fn tighten_f64(
    out: &mut Vec<BreakingChange>,
    ctx: &DiffCtx<'_>,
    o: &NodeRef<'_>,
    c: &NodeRef<'_>,
    chain: &str,
    keyword: &str,
    lower_is_break: bool,
) {
    let (Some(ov), Some(cv)) = (o.get(keyword), c.get(keyword)) else {
        return;
    };
    let (Some(a), Some(b)) = (ov.as_f64(), cv.as_f64()) else {
        return;
    };
    let broke = if lower_is_break { b < a } else { b > a };
    if broke {
        report(
            out,
            ctx,
            DiagnosticSeverity::WARNING,
            cv.byte_range(),
            format!("{chain}: {keyword} changed from {a} to {b}"),
        );
    }
}

/// Reports length bound tightening: `minLength` raised / `maxLength` lowered.
fn tighten_i64(
    out: &mut Vec<BreakingChange>,
    ctx: &DiffCtx<'_>,
    o: &NodeRef<'_>,
    c: &NodeRef<'_>,
    chain: &str,
    keyword: &str,
    lower_is_break: bool,
) {
    let (Some(ov), Some(cv)) = (o.get(keyword), c.get(keyword)) else {
        return;
    };
    let (Some(a), Some(b)) = (ov.as_i64(), cv.as_i64()) else {
        return;
    };
    let broke = if lower_is_break { b < a } else { b > a };
    if broke {
        report(
            out,
            ctx,
            DiagnosticSeverity::WARNING,
            cv.byte_range(),
            format!("{chain}: {keyword} changed from {a} to {b}"),
        );
    }
}

/// The `required` array as a set of names.
fn required_set(schema: &NodeRef<'_>) -> HashSet<String> {
    schema
        .get("required")
        .map(|n| {
            n.items()
                .iter()
                .filter_map(|i| i.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Decoded scalar bytes of a node (quote/escape aware).
fn decoded(node: &NodeRef<'_>) -> Vec<u8> {
    node.decoded_scalar().into_owned()
}

/// While both sides are pure `$ref` objects, follows matching local refs so
/// comparisons see through component indirection. Bounded to avoid loops.
fn resolve_pair<'o, 'c>(mut o: NodeRef<'o>, mut c: NodeRef<'c>) -> (NodeRef<'o>, NodeRef<'c>) {
    for _ in 0..16 {
        let (Some(or_), Some(cr_)) = (o.get("$ref"), c.get("$ref")) else {
            break;
        };
        match (local_deref(or_), local_deref(cr_)) {
            (Some(a), Some(b)) => {
                o = a;
                c = b;
            }
            _ => break,
        }
    }
    (o, c)
}

/// Follows a fragment-only (`#/...`) `$ref` within its own document.
fn local_deref<'d>(ref_value: NodeRef<'d>) -> Option<NodeRef<'d>> {
    let raw = ref_value.as_str()?;
    if !raw.starts_with('#') {
        return None;
    }
    let pointer = Pointer::parse(raw).ok()?;
    let inner = ref_value.syntax().doc();
    NodeRef::new(inner.root()).pointer(&pointer)
}

// ---------------------------------------------------------------------------
// suspect.contractCoverage
// ---------------------------------------------------------------------------

/// Diagnostic `code` emitted by [`coverage_diagnostics`].
pub const COVERAGE_DIAGNOSTIC_CODE: &str = "suspect-arazzo-no-coverage";

/// One operation's Arazzo contract-test coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageGap {
    /// Normalized operation identity, `"METHOD /path"` (e.g. `GET /pets`).
    pub operation: String,
    /// IDs of workflows whose steps exercise this operation, sorted.
    pub covered_by: Vec<String>,
    /// True when no workflow step covers the operation.
    pub gap: bool,
}

/// Maps every OpenAPI operation in the workspace against Arazzo workflow
/// steps.
///
/// Arazzo documents are discovered among the workspace URIs (by
/// `*.arazzo.yaml`/`.yml` name or sniffed family); their steps reference
/// operations either by `operationId` or by `operationPath`, which accepts
/// `$sourceDescriptions.<name>#/paths/~1pets/get`,
/// `<name>#/users/{userId}/delete`, and literal `GET /pets` spellings. Every
/// discovered operation appears exactly once in the result, sorted by
/// operation string.
#[must_use]
pub fn contract_coverage(ws: &Workspace) -> Vec<CoverageGap> {
    let mut displays: BTreeSet<String> = BTreeSet::new();
    let mut by_id: HashMap<String, String> = HashMap::new();
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        let doc = handle.doc();
        if !matches!(
            doc.sniff_family(),
            SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
        ) {
            continue;
        }
        let Some(paths) = doc.root().get("paths") else {
            continue;
        };
        for e in paths.entries() {
            let Some(item) = e.value else { continue };
            for method in DIFF_METHODS {
                let Some(op) = item.get(method) else { continue };
                let display = format!("{} {}", method.to_uppercase(), e.key);
                displays.insert(display.clone());
                if let Some(id) = op.get("operationId").and_then(|n| n.as_str()) {
                    by_id.insert(id.to_owned(), display.clone());
                }
            }
        }
    }

    let mut covered: BTreeMap<String, Vec<String>> =
        displays.into_iter().map(|d| (d, Vec::new())).collect();

    for uri in ws.uris() {
        if !is_arazzo_doc(ws, &uri) {
            continue;
        }
        let Some(handle) = ws.get(&uri) else { continue };
        let arazzo = ArazzoDoc::new(handle.doc());
        for wf in arazzo.workflows() {
            let wid = wf.workflow_id;
            if wid.is_empty() {
                continue;
            }
            for step in wf.steps() {
                if let Some(op) = step_operation(step, &by_id) {
                    covered.entry(op).or_default().push(wid.to_owned());
                }
            }
        }
    }

    covered
        .into_iter()
        .map(|(operation, mut wfs)| {
            wfs.sort();
            wfs.dedup();
            let gap = wfs.is_empty();
            CoverageGap {
                operation,
                covered_by: wfs,
                gap,
            }
        })
        .collect()
}

/// Does this workspace URI look like an Arazzo document?
fn is_arazzo_doc(ws: &Workspace, uri: &Uri) -> bool {
    if let Some(h) = ws.get(uri)
        && h.doc().sniff_family() == SpecFamily::Arazzo10
    {
        return true;
    }
    let name = file_name(uri.as_str());
    name.ends_with(".arazzo.yaml") || name.ends_with(".arazzo.yml")
}

/// Resolves one workflow step to a normalized operation string.
fn step_operation(
    step: &suspect_arazzo::StepView<'_>,
    by_id: &HashMap<String, String>,
) -> Option<String> {
    if let Some(id) = step.operation_id()
        && let Some(display) = by_id.get(id)
    {
        return Some(display.clone());
    }
    parse_operation_path(step.operation_path()?)
}

/// Parses an `operationPath` spelling into `"METHOD /path"` form.
///
/// Accepted shapes: `$sourceDescriptions.<name>#/paths/~1pets/get`,
/// `<name>#/users/{{id}}/delete`, `#/pets/get`, and literal `GET /pets`.
fn parse_operation_path(path: &str) -> Option<String> {
    if let Some((_, frag)) = path.split_once('#') {
        return parse_pointer_fragment(frag);
    }
    // Literal "METHOD /path" form.
    let mut parts = path.split_whitespace();
    let method = parts.next()?.to_uppercase();
    if !DIFF_METHODS.iter().any(|m| m.eq_ignore_ascii_case(&method)) {
        return None;
    }
    let target = parts.next()?;
    Some(format!("{method} {}", target.trim_end_matches('/')))
}

/// Parses the fragment half of an `operationPath`.
fn parse_pointer_fragment(frag: &str) -> Option<String> {
    let trimmed = frag.trim_start_matches('/');
    let tokens: Vec<String> = trimmed
        .split('/')
        .filter(|t| !t.is_empty())
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let (method, segments): (&String, &[String]) = if tokens[0] == "paths" && tokens.len() >= 3 {
        // Canonical `/paths/~1pets/get`: token 1 already decodes to "/pets".
        let m = &tokens[2];
        let path = if tokens[1].starts_with('/') {
            tokens[1].clone()
        } else {
            format!("/{}", tokens[1])
        };
        return Some(format!("{} {path}", m.to_uppercase()));
    } else {
        let m = tokens.last()?;
        if !DIFF_METHODS.iter().any(|k| k.eq_ignore_ascii_case(m)) {
            return None;
        }
        (m, &tokens[..tokens.len() - 1])
    };
    if segments.is_empty() {
        return None;
    }
    Some(format!("{} /{}", method.to_uppercase(), segments.join("/")))
}

/// Builds informational diagnostics for uncovered operations.
///
/// Diagnostics carry severity INFORMATION, source `suspect`, and code
/// [`COVERAGE_DIAGNOSTIC_CODE`]; ranges point at file start since the
/// coverage index is not position-aware.
#[must_use]
pub fn coverage_diagnostics(gaps: &[CoverageGap]) -> Vec<Diagnostic> {
    gaps.iter()
        .filter(|g| g.gap)
        .map(|g| Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::INFORMATION),
            code: Some(NumberOrString::String(COVERAGE_DIAGNOSTIC_CODE.to_owned())),
            code_description: None,
            source: Some("suspect".to_owned()),
            message: format!("operation '{}' has no contract test coverage", g.operation),
            related_information: None,
            tags: None,
            data: None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_ref::{Workspace, WorkspaceBuilder};

    const MAIN: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
    post:
      responses:
        '200':
          description: ok
  /pets/{id}:
    delete:
      responses:
        '200':
          description: ok
components:
  schemas:
    Pet:
      type: object
      required: [id, email]
      properties:
        id:
          type: integer
        email:
          type: string
          format: email
        role:
          type: string
          enum: [admin, reader]
    Node:
      type: object
      properties:
        next:
          $ref: '#/components/schemas/Next'
    Next:
      type: object
      properties:
        back:
          $ref: '#/components/schemas/Node'
"#;

    const ARAZZO: &str = r#"
arazzo: 1.0.0
info:
  title: contracts
sourceDescriptions:
  - name: api
    type: openapi
    url: main.yaml
workflows:
  - workflowId: w1
    steps:
      - stepId: by-path
        operationPath: '$sourceDescriptions.api#/paths/~1pets/get'
      - stepId: by-id
        operationId: listPets
  - workflowId: w2
    steps:
      - stepId: delete-pet
        operationPath: 'api#/pets/{id}/delete'
"#;

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.yaml"), MAIN).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        Arc::new(ws)
    }

    fn workspace_with_arazzo(dir: &std::path::Path) -> Arc<Workspace> {
        let ws = workspace(dir);
        std::fs::write(dir.join("checkout.arazzo.yaml"), ARAZZO).unwrap();
        ws.load_all("checkout.arazzo.yaml").unwrap();
        ws
    }

    /// Writes `text` under a real file URI so workspace lookups match.
    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        LowDoc::parse(
            Uri::from_path(&path).unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn open_doc(low: LowDoc) -> OpenDoc {
        OpenDoc::parse(
            low.uri().clone(),
            String::from_utf8_lossy(low.inner().bytes()).into_owned(),
        )
    }

    fn offset_in(text: &str, needle: &str) -> usize {
        let at = text.find(needle).expect("needle present");
        at + needle.len() / 2
    }

    fn uri_ending(ws: &Workspace, suffix: &str) -> Uri {
        ws.uris()
            .into_iter()
            .find(|u| u.as_str().ends_with(suffix))
            .unwrap()
    }

    // -- generate_example ---------------------------------------------------

    #[test]
    fn generate_example_object_happy_path() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-happy");
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let doc = open_doc(low);
        let off = offset_in(MAIN, "Pet:");
        let generated = generate_example(&ws, &doc, off).expect("generates");
        assert!(
            generated.yaml_snippet.contains("id: 0"),
            "{}",
            generated.yaml_snippet
        );
        assert!(
            generated.yaml_snippet.contains("email: "),
            "{}",
            generated.yaml_snippet
        );
        // Optional properties are included when additionalProperties is free.
        assert!(
            generated.yaml_snippet.contains("role: admin"),
            "{}",
            generated.yaml_snippet
        );
    }

    #[test]
    fn generate_example_format_constants() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-format");
        let ws = workspace(&dir);
        let text = "components:\n  schemas:\n    T:\n      type: string\n      format: date-time\n";
        let low = low_at(&dir, "fmt.yaml", text);
        let doc = open_doc(low);
        let generated = generate_example(&ws, &doc, offset_in(text, "date-time")).unwrap();
        assert!(
            generated.yaml_snippet.contains("2024-01-01T00:00:00Z"),
            "{}",
            generated.yaml_snippet
        );
    }

    #[test]
    fn generate_example_prefers_explicit_example() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-expl");
        let ws = workspace(&dir);
        let text = "components:\n  schemas:\n    T:\n      type: integer\n      example: 42\n";
        let low = low_at(&dir, "expl.yaml", text);
        let doc = open_doc(low);
        let generated = generate_example(&ws, &doc, offset_in(text, "example")).unwrap();
        assert_eq!(generated.yaml_snippet, "42");
    }

    #[test]
    fn generate_example_enum_first_value_and_min_length_and_pattern() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-constraints");
        let ws = workspace(&dir);
        let text = concat!(
            "components:\n",
            "  schemas:\n",
            "    E:\n",
            "      type: string\n",
            "      enum: [alpha, beta]\n",
            "    L:\n",
            "      type: string\n",
            "      minLength: 10\n",
            "    P:\n",
            "      type: string\n",
            "      pattern: '^[a-z]{3}$'\n"
        );
        let low = low_at(&dir, "con.yaml", text);
        let doc = open_doc(low);
        let e = generate_example(&ws, &doc, offset_in(text, "enum")).unwrap();
        assert!(e.yaml_snippet.contains("alpha"), "{}", e.yaml_snippet);
        let l = generate_example(&ws, &doc, offset_in(text, "minLength")).unwrap();
        let rendered = l.yaml_snippet.trim_matches('"');
        assert!(rendered.len() >= 10, "{}", l.yaml_snippet);
        let p = generate_example(&ws, &doc, offset_in(text, "pattern")).unwrap();
        assert!(p.yaml_snippet.contains("aaa"), "{}", p.yaml_snippet);
    }

    #[test]
    fn generate_example_additional_properties_false_drops_optionals() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-addl");
        let ws = workspace(&dir);
        let text = concat!(
            "components:\n",
            "  schemas:\n",
            "    Strict:\n",
            "      type: object\n",
            "      additionalProperties: false\n",
            "      required: [keep]\n",
            "      properties:\n",
            "        keep:\n",
            "          type: integer\n",
            "        drop:\n",
            "          type: integer\n"
        );
        let low = low_at(&dir, "strict.yaml", text);
        let doc = open_doc(low);
        let generated = generate_example(&ws, &doc, offset_in(text, "Strict:")).unwrap();
        assert!(
            generated.yaml_snippet.contains("keep: 0"),
            "{}",
            generated.yaml_snippet
        );
        assert!(
            !generated.yaml_snippet.contains("drop"),
            "{}",
            generated.yaml_snippet
        );
    }

    #[test]
    fn generate_example_recursive_marker_and_cross_file_ref() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-recursion");
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let doc = open_doc(low);
        let generated = generate_example(&ws, &doc, offset_in(MAIN, "Node:")).unwrap();
        assert!(
            generated.yaml_snippet.contains("# circular $ref omitted"),
            "{}",
            generated.yaml_snippet
        );
        // Cross-file: PetList items resolve through schemas.yaml.
        let text = "allOf:\n  - $ref: 'schemas.yaml#/components/schemas/PetList'\n";
        std::fs::write(
            dir.join("schemas.yaml"),
            "components:\n  schemas:\n    PetList:\n      type: array\n      items:\n        $ref: '#/components/schemas/Pet'\n    Pet:\n      type: object\n      required: [id]\n      properties:\n        id:\n          type: integer\n",
        )
        .unwrap();
        ws.load_all("schemas.yaml").unwrap();
        let low2 = low_at(&dir, "entry.yaml", text);
        ws.load_all("entry.yaml").unwrap();
        let doc2 = open_doc(low2);
        let gen2 = generate_example(&ws, &doc2, offset_in(text, "allOf")).unwrap();
        assert!(
            gen2.yaml_snippet.contains("- id: 0"),
            "{}",
            gen2.yaml_snippet
        );
    }

    #[test]
    fn generate_example_insert_edit_shape() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-edit");
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let doc = open_doc(low);
        let generated = generate_example(&ws, &doc, offset_in(MAIN, "Pet:")).unwrap();
        let edit = generated.insert_edit.expect("edit computed");
        // Zero-width insertion at the end of the schema node's last line.
        assert_eq!(edit.range.start, edit.range.end);
        assert!(edit.new_text.starts_with('\n'));
        assert!(edit.new_text.contains("example:"));
        // Body is indented deeper than the `example:` key itself.
        let key_pad = edit.new_text.lines().nth(1).unwrap().len()
            - edit.new_text.lines().nth(1).unwrap().trim_start().len();
        let body_pad = edit.new_text.lines().nth(2).unwrap().len()
            - edit.new_text.lines().nth(2).unwrap().trim_start().len();
        assert_eq!(body_pad, key_pad + 2);
    }

    #[test]
    fn generate_example_cursor_not_on_anything_errors() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-cursor");
        let ws = workspace(&dir);
        let text = "";
        let low = low_at(&dir, "empty.yaml", text);
        let doc = open_doc(low);
        assert!(generate_example(&ws, &doc, 0).is_err());
        // Past EOF on a real doc.
        let low = low_at(&dir, "main.yaml", MAIN);
        let doc = open_doc(low);
        assert!(generate_example(&ws, &doc, MAIN.len() + 50).is_err());
    }

    #[test]
    fn generate_example_is_deterministic() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-gen-det");
        let ws = workspace(&dir);
        let low = low_at(&dir, "main.yaml", MAIN);
        let doc = open_doc(low);
        let off = offset_in(MAIN, "Pet:");
        let a = generate_example(&ws, &doc, off).unwrap();
        let b = generate_example(&ws, &doc, off).unwrap();
        assert_eq!(a.yaml_snippet, b.yaml_snippet);
        assert_eq!(a.insert_edit, b.insert_edit);
    }

    // -- show_ref_graph -----------------------------------------------------

    #[test]
    fn ref_graph_renders_edges_and_labels() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-graph-basic");
        let ws = workspace(&dir);
        let graph = show_ref_graph(&ws);
        assert!(graph.starts_with("graph LR\n"), "{}", graph);
        // Local ref edge from the response schema to Pet.
        assert!(graph.contains("-->|"), "{}", graph);
        assert!(
            graph.contains("\"$ref\"") || graph.contains("|\"Pet\"|"),
            "{}",
            graph
        );
        // Determinism.
        assert_eq!(graph, show_ref_graph(&ws));
    }

    #[test]
    fn ref_graph_marks_cycles() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-graph-cycle");
        let ws = workspace(&dir);
        let graph = show_ref_graph(&ws);
        assert!(graph.contains("%% cycle:"), "{}", graph);
        assert!(graph.contains(" <-> "), "{}", graph);
    }

    #[test]
    fn ref_graph_caps_nodes() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-graph-cap");
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::from("components:\n  schemas:\n");
        for i in 0..250 {
            text.push_str(&format!(
                "    C{i}:\n      $ref: '#/components/schemas/C0'\n"
            ));
        }
        std::fs::write(dir.join("big.yaml"), &text).unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("big.yaml").unwrap();
        let graph = show_ref_graph(&ws);
        assert!(graph.contains("nodes omitted (cap 200)"), "{}", graph);
        assert_eq!(graph, show_ref_graph(&ws));
    }

    #[test]
    fn ref_graph_empty_workspace() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-graph-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        let graph = show_ref_graph(&ws);
        assert_eq!(graph, "graph LR\n%% no documents loaded\n");
    }

    // -- breaking_changes ---------------------------------------------------

    const OLD_SPEC: &str = r#"
openapi: 3.1.0
paths:
  /pets:
    get:
      responses:
        '200':
          description: ok
        '404':
          description: gone
    post:
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [name]
              properties:
                name:
                  type: string
                tag:
                  type: string
                  enum: [a, b]
                size:
                  type: string
                  minLength: 1
  /legacy:
    get:
      responses:
        '200':
          description: ok
components:
  schemas:
    Pet:
      type: object
      required: [id]
      properties:
        id:
          type: integer
          minimum: 0
    Ghost:
      type: string
"#;

    const NEW_SPEC: &str = r#"
openapi: 3.1.0
paths:
  /pets:
    get:
      responses:
        '200':
          description: ok
    post:
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [name, id]
              properties:
                name:
                  type: string
                tag:
                  type: string
                  enum: [a]
                size:
                  type: string
                  minLength: 3
components:
  schemas:
    Pet:
      type: object
      required: [id]
      properties:
        id:
          type: integer
          minimum: 5
"#;

    fn breaking_ws(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("api.yaml"), NEW_SPEC).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("api.yaml").unwrap();
        Arc::new(ws)
    }

    fn find<'a>(changes: &'a [BreakingChange], needle: &str) -> &'a BreakingChange {
        changes
            .iter()
            .find(|c| c.message.contains(needle))
            .unwrap_or_else(|| panic!("no change matching {needle:?} in {changes:#?}"))
    }

    #[test]
    fn breaking_removed_response_status_is_error() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-resp");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        let c = find(&changes, "response '404' removed");
        assert_eq!(c.severity, DiagnosticSeverity::ERROR);
        assert_eq!(c.uri, uri.as_str());
    }

    #[test]
    fn breaking_removed_path_is_error() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-path");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        let c = find(&changes, "path '/legacy' removed");
        assert_eq!(c.severity, DiagnosticSeverity::ERROR);
        assert_eq!(c.range.start.line, 0);
    }

    #[test]
    fn breaking_added_required_is_error() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-reqadd");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        let c = find(&changes, "'id' added to required");
        assert_eq!(c.severity, DiagnosticSeverity::ERROR);
    }

    #[test]
    fn breaking_enum_value_removed_is_error() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-enum");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        let c = find(&changes, "enum value 'b' removed");
        assert_eq!(c.severity, DiagnosticSeverity::ERROR);
    }

    #[test]
    fn breaking_constraint_tightening_is_warning() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-tighten");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        let min = find(&changes, "minimum changed from 0 to 5");
        assert_eq!(min.severity, DiagnosticSeverity::WARNING);
        let ml = find(&changes, "minLength changed from 1 to 3");
        assert_eq!(ml.severity, DiagnosticSeverity::WARNING);
    }

    #[test]
    fn breaking_component_schema_removed_is_error() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-comp");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let changes = breaking_changes(&ws, &old);
        assert!(
            find(&changes, "component schema 'Ghost' removed").severity
                == DiagnosticSeverity::ERROR
        );
    }

    #[test]
    fn breaking_type_change_is_warning() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-type");
        std::fs::create_dir_all(&dir).unwrap();
        let old_text = "components:\n  schemas:\n    P:\n      type: string\n";
        let new_text = "components:\n  schemas:\n    P:\n      type: integer\n";
        std::fs::write(dir.join("p.yaml"), new_text).unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("p.yaml").unwrap();
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "p.yaml");
        old.insert(uri.as_str().to_owned(), old_text.to_owned());
        let changes = breaking_changes(&ws, &old);
        let c = find(&changes, "type changed from 'string' to 'integer'");
        assert_eq!(c.severity, DiagnosticSeverity::WARNING);
    }

    #[test]
    fn breaking_unknown_uris_and_empty_old_are_ignored() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-empty");
        let ws = breaking_ws(&dir);
        assert!(breaking_changes(&ws, &HashMap::new()).is_empty());
        let mut old = HashMap::new();
        old.insert(
            "file:///definitely/not/loaded.yaml".to_owned(),
            OLD_SPEC.to_owned(),
        );
        assert!(breaking_changes(&ws, &old).is_empty());
    }

    #[test]
    fn breaking_changes_are_deterministic() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-br-det");
        let ws = breaking_ws(&dir);
        let mut old = HashMap::new();
        let uri = uri_ending(&ws, "api.yaml");
        old.insert(uri.as_str().to_owned(), OLD_SPEC.to_owned());
        let a = breaking_changes(&ws, &old);
        let b = breaking_changes(&ws, &old);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (x.uri.as_str(), x.message.as_str()),
                (y.uri.as_str(), y.message.as_str())
            );
        }
    }

    // -- contract_coverage --------------------------------------------------

    #[test]
    fn coverage_hit_miss_and_sort_order() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-cov-basic");
        let ws = workspace_with_arazzo(&dir);
        let gaps = contract_coverage(&ws);
        let ops: Vec<&str> = gaps.iter().map(|g| g.operation.as_str()).collect();
        assert_eq!(ops, vec!["DELETE /pets/{id}", "GET /pets", "POST /pets"]);
        let get = gaps.iter().find(|g| g.operation == "GET /pets").unwrap();
        // Covered by both steps of w1 (deduped to one entry).
        assert_eq!(get.covered_by, vec!["w1"]);
        assert!(!get.gap);
        let del = gaps
            .iter()
            .find(|g| g.operation == "DELETE /pets/{id}")
            .unwrap();
        assert_eq!(del.covered_by, vec!["w2"]);
        let post = gaps.iter().find(|g| g.operation == "POST /pets").unwrap();
        assert!(post.covered_by.is_empty() && post.gap);
    }

    #[test]
    fn coverage_literal_method_path_form() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-cov-literal");
        let ws = workspace(&dir);
        std::fs::write(
            dir.join("run.arazzo.yaml"),
            "arazzo: 1.0.0\nworkflows:\n  - workflowId: lit\n    steps:\n      - stepId: s\n        operationPath: 'POST /pets'\n",
        )
        .unwrap();
        ws.load_all("run.arazzo.yaml").unwrap();
        let gaps = contract_coverage(&ws);
        let post = gaps.iter().find(|g| g.operation == "POST /pets").unwrap();
        assert_eq!(post.covered_by, vec!["lit"]);
    }

    #[test]
    fn coverage_without_arazzo_marks_everything_uncovered() {
        let dir = std::env::temp_dir().join("suspect-lsp-cmd-cov-none");
        let ws = workspace(&dir);
        let gaps = contract_coverage(&ws);
        assert!(!gaps.is_empty());
        assert!(gaps.iter().all(|g| g.gap && g.covered_by.is_empty()));
        assert_eq!(gaps, contract_coverage(&ws));
    }

    #[test]
    fn coverage_diagnostics_carry_code_and_severity() {
        let gaps = vec![CoverageGap {
            operation: "DELETE /pets/{id}".to_owned(),
            covered_by: vec![],
            gap: true,
        }];
        let diags = coverage_diagnostics(&gaps);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::INFORMATION));
        assert_eq!(
            diags[0].code,
            Some(NumberOrString::String(COVERAGE_DIAGNOSTIC_CODE.to_owned()))
        );
        assert_eq!(diags[0].source.as_deref(), Some("suspect"));
        assert!(coverage_diagnostics(&[]).is_empty());
    }
}
