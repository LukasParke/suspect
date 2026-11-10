//! Deterministic example synthesis and the precompiled mock table.
//!
//! Mock bodies are **precompiled at startup**: every declared response of
//! every [`IrOperation`](suspect_ir::IrOperation) gets its JSON example
//! synthesized once from the materialized IR schema JSON and stored as
//! bytes; serving is then a lookup plus a copy, with no per-request
//! synthesis cost or nondeterminism.
//!
//! Synthesis precedence per node mirrors
//! [`suspect_gen::example_of`]: `example` > `default` > `const` > first
//! `enum` value > a value built from the type. A built value satisfies the
//! schema it is built from, so the mock's own replies pass the contract
//! check: numbers respect their bounds, strings their length and `format`,
//! arrays their `minItems`, `oneOf`/`anyOf` take their first non-null
//! branch (merged with the properties declared beside the union), `allOf`
//! compositions union-merge their members — or, for a scalar or a list
//! described piecewise, intersect them — and `$ref`s resolve through the
//! spec's component-schema lookup map.
//!
//! Objects include every declared property and arrays one element, until
//! the schema turns back on itself or nesting passes [`DEPTH_CAP`]. From
//! there only what a schema requires is built: optional properties are
//! left out and lists are empty, so a recursive schema (`Pet.friend ->
//! Pet`) ends in a valid value instead of a `null` its type forbids.

use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::Value;
use suspect_ir::{IrSpec, Method};

use crate::problem;

/// Component-name → materialized schema JSON, used to resolve local `$ref`s.
pub type SchemaRefs = HashMap<String, Value>;

/// Nesting depth past which only what a schema requires is synthesized.
///
/// A fixed cap keeps examples small and synthesis total on deep or cyclic
/// schemas while still producing full examples for shallow trees.
pub const DEPTH_CAP: u8 = 6;

/// Nesting depth at which synthesis gives up with `null`: only a schema
/// that *requires* itself without end gets here, and no finite value
/// satisfies it.
const HARD_CAP: u8 = 4 * DEPTH_CAP;

/// Largest number of elements built for an array's `minItems`.
const MAX_ITEMS: u64 = 16;

/// Number of values past which an example is finished with only what its
/// schema requires, whatever the depth: a mock reply is read by people.
const FULL_VALUES: usize = 5_000;

/// Most values one example holds. Like [`HARD_CAP`], only a schema that
/// requires itself without end gets here; what is built for it ends in
/// `null` instead of growing with every level.
const MAX_VALUES: usize = 20_000;

/// Headers that say how a message is framed or what its connection does.
/// The server sets them to match what it sends; an example value from the
/// description would contradict the body that follows.
const FRAMING_HEADERS: [&str; 7] = [
    "connection",
    "content-encoding",
    "content-length",
    "keep-alive",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// One precompiled mock response: status (`None` for `default`), body and
/// the media type the body is served as.
#[derive(Debug, Clone)]
pub struct CompiledResponse {
    /// Status the response is served with: the declared code, the first
    /// status of a declared range (`400` for `4XX`), or `None` for the
    /// `default` response.
    pub status: Option<u16>,
    /// Precompiled example body bytes; empty when the response declares
    /// no content.
    pub body: Bytes,
    /// The declared media type the body is an example of; `None` when the
    /// response declares no content, so no body is served.
    pub content_type: Option<String>,
    /// The headers the response declares, each with an example value.
    pub headers: Vec<(String, String)>,
}

impl CompiledResponse {
    /// The response as served: `status`, the declared headers, and `body`
    /// under the declared media type. A response that declares no content
    /// is served without a body.
    #[must_use]
    pub fn serve(&self, status: StatusCode, body: Bytes) -> Response {
        let mut response = match &self.content_type {
            Some(content_type) => {
                (status, [("content-type", content_type.clone())], body).into_response()
            }
            None => status.into_response(),
        };
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::HeaderName::try_from(name.as_str()),
                axum::http::HeaderValue::try_from(value.as_str()),
            ) {
                response.headers_mut().insert(name, value);
            }
        }
        response
    }
}

/// Builds a component-name → schema-JSON map from an IR snapshot.
#[must_use]
pub fn schema_refs(spec: &IrSpec) -> SchemaRefs {
    spec.schemas
        .iter()
        .map(|s| (s.name.clone(), s.json.clone()))
        .collect()
}

/// Extracts a schema's base type, tolerating type arrays such as
/// `["integer", "null"]` (the first non-null entry wins).
fn schema_type(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(ToOwned::to_owned),
        _ => None,
    }
}

/// Synthesizes a deterministic JSON example from an OpenAPI schema object.
///
/// `refs` maps component names to their materialized schema JSON (see
/// [`schema_refs`]); local `$ref`s whose tail matches a component name are
/// resolved through it. `depth` guards recursion — callers start at 0.
/// The function is pure: identical inputs always produce identical output,
/// which is what makes recorded gateway traffic diffable in git.
#[must_use]
pub fn synth_example(schema: &Value, refs: &SchemaRefs, depth: u8) -> Value {
    synth(schema, refs, depth, &mut Walk::default(), false)
}

/// The state of one synthesis.
#[derive(Default)]
struct Walk {
    /// The components being expanded, outermost first.
    stack: Vec<String>,
    /// How many values have been built.
    values: usize,
}

/// The component a `$ref` names: its last path segment.
fn ref_name(schema: &Value) -> Option<&str> {
    let target = schema.get("$ref").and_then(Value::as_str)?;
    Some(target.rsplit('/').next().unwrap_or(target))
}

/// `schema`, or the component it refers to.
fn resolved<'v>(schema: &'v Value, refs: &'v SchemaRefs) -> &'v Value {
    ref_name(schema)
        .and_then(|name| refs.get(name))
        .unwrap_or(schema)
}

/// Whether `schema` is, or is a list of, a component already being
/// expanded: following it would only repeat what is on the stack.
fn recurses(schema: &Value, stack: &[String]) -> bool {
    let on_stack =
        |schema: &Value| ref_name(schema).is_some_and(|name| stack.iter().any(|s| s == name));
    on_stack(schema) || schema.get("items").is_some_and(on_stack)
}

/// The property names a schema requires.
fn required_of(schema: &Value) -> impl Iterator<Item = &str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// [`synth_example`] with the state of the walk and whether only required
/// parts are built (`minimal`).
fn synth(schema: &Value, refs: &SchemaRefs, depth: u8, walk: &mut Walk, minimal: bool) -> Value {
    walk.values += 1;
    if depth > HARD_CAP || walk.values > MAX_VALUES {
        return Value::Null;
    }
    let minimal = minimal || depth > DEPTH_CAP || walk.values > FULL_VALUES;
    if let Some(name) = ref_name(schema) {
        let Some(component) = refs.get(name) else {
            return Value::Null;
        };
        // A schema that turns back on itself is built minimally from here.
        let minimal = minimal || walk.stack.iter().any(|s| s == name);
        walk.stack.push(name.to_owned());
        let value = synth(component, refs, depth + 1, walk, minimal);
        walk.stack.pop();
        return value;
    }
    for keyword in ["example", "default", "const"] {
        if let Some(v) = schema.get(keyword) {
            return v.clone();
        }
    }
    if let Some(v) = schema.get("enum")
        && let Some(first) = v.as_array().and_then(|values| values.first())
    {
        return first.clone();
    }
    if let Some(Value::Array(members)) = schema.get("allOf")
        && !members.is_empty()
    {
        // A scalar or a list described piecewise (`[{type: string},
        // {minLength: 3}]`) is one schema: its pieces are intersected.
        if let Some(whole) = intersected(schema, refs) {
            return synth(&whole, refs, depth + 1, walk, minimal);
        }
        // Anything else is an object: every member is synthesized and the
        // results union-merged, so composed schemas (the spec-heavy
        // pattern) mock as the merge the wire actually carries. A member
        // that says nothing about the value leaves it as it is.
        let mut merged = Value::Null;
        for member in members {
            merged = merge_example(merged, synth(member, refs, depth + 1, walk, minimal));
        }
        // Properties declared beside `allOf` are part of the value too.
        if schema.get("properties").is_some() {
            merged = merge_example(merged, object(schema, refs, depth, walk, minimal));
        }
        if let Value::Object(map) = &mut merged {
            let parts: Vec<&Value> = std::iter::once(schema)
                .chain(members.iter())
                .map(|member| resolved(member, refs))
                .collect();
            fill_required(map, &parts, refs, depth, walk, minimal);
        }
        return merged;
    }
    // A union is satisfied by any one branch: `null` itself once only the
    // minimum is being built, else the first that says more than "null"
    // and does not lead straight back into a component being expanded.
    for keyword in ["oneOf", "anyOf"] {
        if let Some(Value::Array(branches)) = schema.get(keyword)
            && !branches.is_empty()
        {
            let is_null =
                |branch: &&Value| branch.get("type").and_then(Value::as_str) == Some("null");
            let chosen = {
                let returns = |branch: &&Value| {
                    ref_name(branch).is_some_and(|name| walk.stack.iter().any(|s| s == name))
                };
                // `null` is only an answer where the schema's own `type`
                // does not rule it out.
                let may_be_null = match schema.get("type") {
                    None => true,
                    Some(Value::Array(types)) => types.iter().any(|kind| kind == "null"),
                    Some(kind) => kind == "null",
                };
                if minimal && may_be_null {
                    branches.iter().find(is_null)
                } else {
                    None
                }
                .or_else(|| {
                    branches
                        .iter()
                        .find(|branch| !is_null(branch) && !returns(branch))
                })
                .or_else(|| branches.iter().find(|branch| !is_null(branch)))
                .unwrap_or(&branches[0])
            };
            let value = synth(chosen, refs, depth + 1, walk, minimal);
            if is_null(&chosen)
                || (schema.get("properties").is_none() && schema.get("required").is_none())
            {
                return value;
            }
            // Properties and requirements declared beside the union hold
            // whichever branch is taken: the branch adds to the object.
            let mut merged = merge_example(object(schema, refs, depth, walk, minimal), value);
            if let Value::Object(map) = &mut merged {
                let parts = [schema, resolved(chosen, refs)];
                if keyword == "oneOf" {
                    // Exactly one branch may hold: an optional property
                    // that another branch requires would satisfy that
                    // branch too.
                    let needed: Vec<&str> =
                        parts.iter().flat_map(|part| required_of(part)).collect();
                    for other in branches
                        .iter()
                        .filter(|other| !std::ptr::eq(*other, chosen))
                    {
                        for key in required_of(resolved(other, refs)) {
                            if !needed.contains(&key) {
                                map.remove(key);
                            }
                        }
                    }
                }
                fill_required(map, &parts, refs, depth, walk, minimal);
            }
            return merged;
        }
    }
    let inferred = schema_type(schema).or_else(|| {
        if schema.get("properties").is_some() || schema.get("required").is_some() {
            Some("object".to_owned())
        } else if schema.get("items").is_some() {
            Some("array".to_owned())
        } else {
            None
        }
    });
    match inferred.as_deref() {
        Some("object") => object(schema, refs, depth, walk, minimal),
        Some("array") => {
            let items = schema.get("items").unwrap_or(&Value::Null);
            let least = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            let count = if minimal || recurses(schema, &walk.stack) {
                least
            } else {
                least.max(1)
            }
            .min(MAX_ITEMS);
            Value::Array(
                (0..count)
                    .map(|_| synth(items, refs, depth + 1, walk, minimal))
                    .collect(),
            )
        }
        Some("string") => Value::String(string(schema)),
        Some(kind @ ("integer" | "number")) => number(schema, kind == "integer"),
        Some("boolean") => Value::Bool(false),
        _ => Value::Null,
    }
}

/// Puts into `map` what the schemas of a composition (`parts`) require
/// between them. One part may require what another declares: a property
/// left out as optional where it is declared, or held only as a
/// placeholder by the part that requires it, gets the value its
/// declaration describes.
fn fill_required(
    map: &mut serde_json::Map<String, Value>,
    parts: &[&Value],
    refs: &SchemaRefs,
    depth: u8,
    walk: &mut Walk,
    minimal: bool,
) {
    for key in parts.iter().flat_map(|part| required_of(part)) {
        if map.get(key).is_some_and(|value| !value.is_null()) {
            continue;
        }
        if let Some(property) = parts
            .iter()
            .find_map(|part| part.get("properties").and_then(|props| props.get(key)))
        {
            map.insert(
                key.to_owned(),
                synth(property, refs, depth + 1, walk, minimal),
            );
        }
    }
}

/// Keywords of which the largest value binds when schemas are intersected.
const LOWER_BOUNDS: [&str; 4] = ["exclusiveMinimum", "minItems", "minLength", "minimum"];

/// Keywords of which the smallest value binds when schemas are intersected.
const UPPER_BOUNDS: [&str; 4] = ["exclusiveMaximum", "maxItems", "maxLength", "maximum"];

/// The one schema an `allOf` over a scalar or a list amounts to: the type
/// its members agree on, the tightest of their bounds, the values every
/// `enum` among them allows, and otherwise the first member to use a
/// keyword. `None` when the composition is an object, a union, or has no
/// type to go by; those are merged value by value instead.
fn intersected(schema: &Value, refs: &SchemaRefs) -> Option<Value> {
    let mut parts: Vec<&Value> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut queue: Vec<&Value> = vec![schema];
    while let Some(part) = queue.pop() {
        let part = match ref_name(part) {
            Some(name) if seen.contains(&name) => continue,
            Some(name) => {
                seen.push(name);
                refs.get(name)?
            }
            None => part,
        };
        if ["anyOf", "oneOf", "not", "properties", "required"]
            .iter()
            .any(|keyword| part.get(keyword).is_some())
        {
            return None;
        }
        if let Some(Value::Array(members)) = part.get("allOf") {
            // Reversed, so members are visited in the order written.
            queue.extend(members.iter().rev());
        }
        parts.push(part);
    }
    let kind = parts.iter().find_map(|part| schema_type(part))?;
    if kind == "object"
        || parts
            .iter()
            .filter_map(|part| schema_type(part))
            .any(|other| other != kind)
    {
        return None;
    }
    let mut whole = serde_json::Map::new();
    for part in parts {
        let Value::Object(keywords) = part else {
            continue;
        };
        for (keyword, value) in keywords {
            if keyword == "allOf" {
                continue;
            }
            let Some(held) = whole.get_mut(keyword) else {
                whole.insert(keyword.clone(), value.clone());
                continue;
            };
            let tighter = |wanted: std::cmp::Ordering| {
                held.as_f64()
                    .zip(value.as_f64())
                    .is_some_and(|(held, new)| new.partial_cmp(&held) == Some(wanted))
            };
            if (LOWER_BOUNDS.contains(&keyword.as_str()) && tighter(std::cmp::Ordering::Greater))
                || (UPPER_BOUNDS.contains(&keyword.as_str()) && tighter(std::cmp::Ordering::Less))
            {
                *held = value.clone();
            } else if keyword == "enum"
                && let (Value::Array(held), Value::Array(allowed)) = (held, value)
                && held.iter().any(|option| allowed.contains(option))
            {
                held.retain(|option| allowed.contains(option));
            }
        }
    }
    Some(Value::Object(whole))
}

/// An object example: every declared property, or only the required ones
/// when `minimal`. An optional property that would re-enter a component
/// being expanded is left out.
fn object(schema: &Value, refs: &SchemaRefs, depth: u8, walk: &mut Walk, minimal: bool) -> Value {
    let required: Vec<&str> = required_of(schema).collect();
    let mut out = serde_json::Map::new();
    if let Some(Value::Object(props)) = schema.get("properties") {
        for (key, prop) in props {
            let needed = required.contains(&key.as_str());
            if !needed && (minimal || walk.values > FULL_VALUES || recurses(prop, &walk.stack)) {
                continue;
            }
            out.insert(key.clone(), synth(prop, refs, depth + 1, walk, minimal));
        }
    }
    // A property that is required but declared nowhere is constrained only
    // to exist: any value satisfies it.
    for name in required {
        if !out.contains_key(name) {
            out.insert(name.to_owned(), Value::Null);
        }
    }
    Value::Object(out)
}

/// A string example that satisfies the schema's `format` and length.
fn string(schema: &Value) -> String {
    let mut text = match schema.get("format").and_then(Value::as_str) {
        Some("date-time") => "2024-01-01T00:00:00Z",
        Some("date") => "2024-01-01",
        Some("time") => "00:00:00Z",
        Some("uuid") => "00000000-0000-4000-8000-000000000000",
        Some("email") => "user@example.com",
        Some("uri" | "url" | "iri" | "uri-reference") => "https://example.com",
        Some("hostname") => "example.com",
        Some("ipv4") => "192.0.2.1",
        Some("ipv6") => "2001:db8::1",
        _ => "",
    }
    .to_owned();
    let length = |key: &str| schema.get(key).and_then(Value::as_u64).map(|n| n as usize);
    if let Some(least) = length("minLength") {
        let have = text.chars().count();
        if have < least {
            text.push_str(&"a".repeat(least - have));
        }
    }
    if let Some(most) = length("maxLength")
        && text.chars().count() > most
    {
        text = text.chars().take(most).collect();
    }
    if schema.get("pattern").is_some() {
        return patterned(schema, text);
    }
    text
}

/// Strings tried, in order, against a `pattern` the built string does not
/// match: the shapes identifiers, codes, versions and dates usually take.
const PATTERN_CANDIDATES: [&str; 30] = [
    "a",
    "A",
    "0",
    "1",
    "aa",
    "a0",
    "abc",
    "ABC",
    "abc123",
    "a-b",
    "a_b",
    "a.b",
    "a b",
    "123",
    "12345",
    "1.0",
    "0.0.0",
    "en",
    "en-US",
    "USD",
    "#000000",
    "+10000000000",
    "2024-01-01",
    "2024-01-01T00:00:00Z",
    "user@example.com",
    "https://example.com",
    "/a",
    "00000000-0000-4000-8000-000000000000",
    "aaaaaaaaaaaaaaaaaaaaaaaa",
    "000000000000000000000000",
];

/// A string that satisfies the schema's `pattern` and lengths: `text` when
/// it already does, else the first candidate that does, else `text`.
///
/// The check is the contract runtime's own, so the mock and the contract
/// test cannot disagree about what a pattern matches. Results are kept per
/// constraint, since one pattern is typically declared many times.
fn patterned(schema: &Value, text: String) -> String {
    use std::collections::HashMap as Map;
    use std::sync::{Mutex, OnceLock};
    static KNOWN: OnceLock<Mutex<Map<String, Option<String>>>> = OnceLock::new();

    let mut constraint = serde_json::Map::new();
    constraint.insert("type".to_owned(), Value::String("string".to_owned()));
    for key in ["pattern", "minLength", "maxLength"] {
        if let Some(value) = schema.get(key) {
            constraint.insert(key.to_owned(), value.clone());
        }
    }
    let constraint = Value::Object(constraint);
    let key = format!("{constraint}\u{0}{text}");
    let known = KNOWN.get_or_init(Mutex::default);
    if let Some(found) = known
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return found.clone().unwrap_or(text);
    }
    let schemas = suspect_runtime::Schemas::default();
    let satisfies = |candidate: &str| {
        schemas
            .validate(&constraint, &Value::String(candidate.to_owned()))
            .is_empty()
    };
    let found = if satisfies(&text) {
        Some(text.clone())
    } else {
        // A string read off the pattern itself, then the usual shapes.
        schema
            .get("pattern")
            .and_then(Value::as_str)
            .and_then(pattern_example)
            .filter(|candidate| satisfies(candidate))
            .or_else(|| {
                PATTERN_CANDIDATES
                    .iter()
                    .find(|candidate| satisfies(candidate))
                    .map(|candidate| (*candidate).to_owned())
            })
    };
    known
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, found.clone());
    found.unwrap_or(text)
}

/// A number example inside the schema's bounds: `0` when the bounds allow
/// it, else the nearest value that does.
fn number(schema: &Value, integer: bool) -> Value {
    let bound = |key: &str| schema.get(key).and_then(Value::as_f64);
    // OpenAPI 3.0 spells an exclusive bound as a boolean beside it.
    let exclusive = |key: &str| schema.get(key).and_then(Value::as_bool).unwrap_or(false);
    let step = if integer { 1.0 } else { 0.5 };
    let mut value: f64 = 0.0;
    if let Some(least) = bound("minimum") {
        let least = if exclusive("exclusiveMinimum") {
            least + step
        } else {
            least
        };
        value = value.max(least);
    }
    if let Some(above) = bound("exclusiveMinimum")
        && value <= above
    {
        value = above + step;
    }
    if let Some(most) = bound("maximum") {
        let most = if exclusive("exclusiveMaximum") {
            most - step
        } else {
            most
        };
        value = value.min(most);
    }
    if let Some(below) = bound("exclusiveMaximum")
        && value >= below
    {
        value = below - step;
    }
    if let Some(multiple) = bound("multipleOf")
        && multiple > 0.0
    {
        value = (value / multiple).ceil() * multiple;
    }
    if integer {
        value = value.ceil();
    }
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::from(0), Value::Number)
    }
}

/// A string a regular expression of the common, simple kind matches:
/// literals stand for themselves, a class for one of its members, `+` for
/// one repetition, `*` and `?` for none, `{n,m}` for `n`, a group for its
/// first alternative. `None` for syntax this does not read; the caller
/// checks the result against the real matcher either way.
fn pattern_example(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut position = 0;
    let out = alternation(&chars, &mut position)?;
    (position == chars.len()).then_some(out)
}

/// The first alternative of `a|b|c`, up to the group's `)` or the end.
fn alternation(chars: &[char], position: &mut usize) -> Option<String> {
    let first = sequence(chars, position)?;
    while chars.get(*position) == Some(&'|') {
        *position += 1;
        sequence(chars, position)?;
    }
    Some(first)
}

fn sequence(chars: &[char], position: &mut usize) -> Option<String> {
    let mut out = String::new();
    while let Some(&ch) = chars.get(*position) {
        let atom = match ch {
            '|' | ')' => break,
            '^' | '$' => {
                *position += 1;
                continue;
            }
            '(' => {
                *position += 1;
                // `(?:` and the like group without changing what matches.
                if chars.get(*position) == Some(&'?') {
                    *position += 1;
                    match chars.get(*position) {
                        Some(':') => *position += 1,
                        _ => return None,
                    }
                }
                let inner = alternation(chars, position)?;
                if chars.get(*position) != Some(&')') {
                    return None;
                }
                *position += 1;
                inner
            }
            '[' => class(chars, position)?.to_string(),
            '\\' => {
                *position += 1;
                let escaped = *chars.get(*position)?;
                *position += 1;
                match escaped {
                    'd' => "0".to_owned(),
                    'w' => "a".to_owned(),
                    's' => " ".to_owned(),
                    'D' | 'W' | 'S' | 'b' | 'B' => return None,
                    'n' => "\n".to_owned(),
                    't' => "\t".to_owned(),
                    other if other.is_ascii_alphanumeric() => return None,
                    other => other.to_string(),
                }
            }
            '.' => {
                *position += 1;
                "a".to_owned()
            }
            '*' | '+' | '?' | '{' => return None,
            other => {
                *position += 1;
                other.to_string()
            }
        };
        out.push_str(&atom.repeat(repetitions(chars, position)?));
    }
    Some(out)
}

/// How many times the atom before `position` must appear, consuming its
/// quantifier: the least the quantifier allows, and one for `+`.
fn repetitions(chars: &[char], position: &mut usize) -> Option<usize> {
    let count = match chars.get(*position) {
        Some('*' | '?') => {
            *position += 1;
            0
        }
        Some('+') => {
            *position += 1;
            1
        }
        Some('{') => {
            let close = chars[*position..].iter().position(|c| *c == '}')? + *position;
            let body: String = chars[*position + 1..close].iter().collect();
            *position = close + 1;
            body.split(',').next()?.trim().parse::<usize>().ok()?
        }
        _ => return Some(1),
    };
    // A lazy or possessive marker changes how, not what, it matches.
    if matches!(chars.get(*position), Some('?' | '+')) {
        *position += 1;
    }
    Some(count.min(64))
}

/// One character a `[...]` class matches, consuming the class.
fn class(chars: &[char], position: &mut usize) -> Option<char> {
    *position += 1;
    let negated = chars.get(*position) == Some(&'^');
    if negated {
        *position += 1;
    }
    let mut members: Vec<(char, char)> = Vec::new();
    let mut first = true;
    loop {
        let ch = *chars.get(*position)?;
        if ch == ']' && !first {
            *position += 1;
            break;
        }
        first = false;
        *position += 1;
        let low = if ch == '\\' {
            let escaped = *chars.get(*position)?;
            *position += 1;
            match escaped {
                'd' => {
                    members.push(('0', '9'));
                    continue;
                }
                'w' => {
                    members.extend([('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')]);
                    continue;
                }
                's' => {
                    members.push((' ', ' '));
                    continue;
                }
                other if other.is_ascii_alphanumeric() => return None,
                other => other,
            }
        } else {
            ch
        };
        if chars.get(*position) == Some(&'-') && chars.get(*position + 1).is_some_and(|c| *c != ']')
        {
            let high = chars[*position + 1];
            *position += 2;
            members.push((low, high));
        } else {
            members.push((low, low));
        }
    }
    if !negated {
        return members.first().map(|(low, _)| *low);
    }
    "a0A_- ./:".chars().find(|candidate| {
        !members
            .iter()
            .any(|(low, high)| (*low..=*high).contains(candidate))
    })
}

/// Recursive union of two synthesized examples: object members merge
/// key-wise, and any other pair resolves to the overlay (later member
/// wins) — matching what an `allOf` composition means on the wire. A
/// `null` overlay is a schema that had nothing to say about the value (an
/// annotation, a bare `required`), so it replaces nothing.
fn merge_example(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (base, Value::Null) => base,
        (Value::Object(mut base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                let merged = match base.remove(&key) {
                    Some(existing) => merge_example(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            Value::Object(base)
        }
        (_, overlay) => overlay,
    }
}

/// Precompiles every declared response of every operation.
///
/// Keyed by `(method, path template)` so request dispatch is a single map
/// hit after route matching. Synthesis is pure, so operations are
/// compiled in parallel across the rayon pool.
#[must_use]
pub fn compile_all(spec: &IrSpec) -> HashMap<(Method, String), Vec<CompiledResponse>> {
    use rayon::prelude::*;

    let refs = schema_refs(spec);
    spec.operations
        .par_iter()
        .map(|op| {
            let compiled = op
                .responses
                .iter()
                .map(|resp| compile_response(spec, &refs, resp))
                .collect::<Vec<_>>();
            ((op.method, op.path.clone()), compiled)
        })
        .collect()
}

/// Precompiles one declared response: an example of the media type a JSON
/// client reads (`application/json`, else the first `+json` type, else the
/// first declared type), served under that media type.
fn compile_response(
    spec: &IrSpec,
    refs: &SchemaRefs,
    resp: &suspect_ir::IrResponse,
) -> CompiledResponse {
    let status = resp
        .status
        .or_else(|| resp.range.map(|class| u16::from(class) * 100));
    // Every declared header is sent with an example of its schema, so a
    // consumer sees the response's shape and a required header is present.
    // `Content-Type` is the content map's business, and the framing of the
    // message the server's.
    let headers: Vec<(String, String)> = resp
        .headers
        .iter()
        .filter(|header| {
            let name = header.name.to_ascii_lowercase();
            name != "content-type" && !FRAMING_HEADERS.contains(&name.as_str())
        })
        .map(|header| {
            let example = header
                .schema
                .as_ref()
                .map_or(Value::Null, |schema| synth_example(schema, refs, 0));
            let text = match example {
                Value::String(text) => text,
                Value::Null => String::new(),
                other => other.to_string(),
            };
            (header.name.clone(), text)
        })
        .collect();
    let Some(media) = resp.json_media().or_else(|| resp.content.first()) else {
        return CompiledResponse {
            status,
            body: Bytes::new(),
            content_type: None,
            headers,
        };
    };
    // Unresolvable component names degrade to `null`, matching the
    // "external refs stay opaque" policy of suspect-ir.
    let example = media
        .schema
        .as_deref()
        .and_then(|name| spec.schema(name))
        .map_or(Value::Null, |ir| synth_example(&ir.json, refs, 0));
    let json = suspect_ir::is_json_media_type(&media.media_type);
    let body = match &example {
        // A textual media type carries the text itself, not its JSON
        // spelling.
        Value::String(text) if !json => text.clone().into_bytes(),
        other => serde_json::to_vec(other).unwrap_or_else(|_| b"null".to_vec()),
    };
    // A wildcard range is not a type a server can answer with.
    let content_type = if media.media_type.contains('*') {
        "application/json".to_owned()
    } else {
        media.media_type.clone()
    };
    CompiledResponse {
        status,
        body: Bytes::from(body),
        content_type: Some(content_type),
        headers,
    }
}

/// Picks the best mock response: lowest 2xx, else lowest numeric status,
/// else the `default` response.
#[must_use]
pub fn best(compiled: &[CompiledResponse]) -> Option<&CompiledResponse> {
    compiled.iter().min_by_key(|c| {
        (
            match c.status {
                Some(s) if (200..300).contains(&s) => 0u8,
                Some(_) => 1,
                None => 2,
            },
            c.status.unwrap_or(u16::MAX),
        )
    })
}

/// The success response an operation declares: its lowest `2xx` status,
/// or its `default` response when no status is declared.
#[must_use]
pub fn success(compiled: &[CompiledResponse]) -> Option<&CompiledResponse> {
    best(compiled).filter(|c| c.status.is_none_or(|status| (200..300).contains(&status)))
}

/// Serves a stored resource the way the operation declares its success:
/// under the declared status and media type, and with no body at all when
/// the declared response has no content. An operation that declares no
/// success response gets `fallback` and `application/json`.
#[must_use]
pub fn respond_with(
    compiled: Option<&[CompiledResponse]>,
    fallback: StatusCode,
    body: Bytes,
) -> Response {
    let Some(declared) = compiled.and_then(success) else {
        return (fallback, [("content-type", "application/json")], body).into_response();
    };
    let status = declared
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(fallback);
    declared.serve(status, body)
}

/// Rejects a request that breaks the contract, the way the operation
/// declares its rejections: its `400` response, else `422`, else another
/// declared `4xx`, else `default`, served with that response's example.
/// An operation that declares none of them gets a `400` problem document
/// listing what was wrong with the request.
#[must_use]
pub fn reject(
    compiled: Option<&[CompiledResponse]>,
    violations: &[suspect_journal::Violation],
) -> Response {
    let declared = compiled.and_then(|compiled| {
        let exact = |status: u16| compiled.iter().find(|c| c.status == Some(status));
        exact(400)
            .or_else(|| exact(422))
            .or_else(|| {
                compiled
                    .iter()
                    .find(|c| c.status.is_some_and(|status| (400..500).contains(&status)))
            })
            .or_else(|| compiled.iter().find(|c| c.status.is_none()))
    });
    let Some(declared) = declared else {
        let detail = serde_json::json!({
            "title": "Request failed validation",
            "status": 400,
            "violations": violations.iter().map(|violation| serde_json::json!({
                "message": violation.message,
                "pointer": violation.pointer,
            })).collect::<Vec<_>>(),
        })
        .to_string();
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/problem+json")],
            detail,
        )
            .into_response();
    };
    let status = declared
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::BAD_REQUEST);
    declared.serve(status, declared.body.clone())
}

/// Builds the served response for one matched operation.
///
/// Falls back to `501 Not Implemented` problem+json when the operation
/// declares no responses to synthesize from.
#[must_use]
pub fn respond(compiled: &[CompiledResponse]) -> Response {
    match best(compiled) {
        Some(c) => {
            let status = StatusCode::from_u16(c.status.unwrap_or(200)).unwrap_or(StatusCode::OK);
            c.serve(status, c.body.clone())
        }
        None => problem(
            StatusCode::NOT_IMPLEMENTED,
            "No synthesized response",
            Some("the operation declares no responses".to_owned()),
        ),
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::pattern_example;

    #[test]
    fn simple_patterns_yield_a_string_they_match() {
        for (pattern, expected) in [
            ("^/v1/webhook_endpoints", "/v1/webhook_endpoints"),
            (
                "^/v1/tax/calculations/[^/]+/line_items",
                "/v1/tax/calculations/a/line_items",
            ),
            ("^[A-Z]{3}$", "AAA"),
            ("^[0-9]+\\.[0-9]+$", "0.0"),
            ("^\\d{4}-\\d{2}-\\d{2}$", "0000-00-00"),
            ("^(cat|dog)s?$", "cat"),
            ("^(?:ab)+c*$", "ab"),
            ("^[a-z0-9_-]{2,5}$", "aa"),
            ("^\\w+@\\w+\\.com$", "a@a.com"),
        ] {
            assert_eq!(
                pattern_example(pattern).as_deref(),
                Some(expected),
                "{pattern}"
            );
        }
    }

    #[test]
    fn syntax_it_does_not_read_yields_nothing() {
        for pattern in [
            "^(?=a)b",
            "\\bword\\b",
            "a{",
            "(unclosed",
            "[unclosed",
            "*a",
        ] {
            assert_eq!(pattern_example(pattern), None, "{pattern}");
        }
    }
}
