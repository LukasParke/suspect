//! Built-in template filters for code generation.
//!
//! [`FilterRegistry`] installs a small set of pure, deterministic filters
//! onto a [`MinijinjaEngine`](crate::MinijinjaEngine): identifier case
//! conversion, OpenAPI-schema-to-type mappings for TypeScript and Rust,
//! deterministic example synthesis, and Mermaid dependency graphs.
//!
//! Every behavior is also exposed as a free function so callers can use it
//! without going through an engine.

use std::collections::BTreeSet;

use minijinja::Value;
use serde_json::Value as Json;

use crate::MinijinjaEngine;

/// Registry that installs the built-in generation filters onto an engine.
///
/// After registration, templates may use the filters by name, e.g.
/// `{{ "petStoreId" | snake_case }}`. Hyphenated contract names
/// (`kebab-case`, `CONSTANT_CASE`) are registered verbatim, plus
/// underscore aliases (`kebab_case`, `constant_case`) usable inside
/// template expressions where `-` would parse as subtraction.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterRegistry;

impl FilterRegistry {
    /// Registers every built-in filter onto `engine`.
    pub fn register(engine: &mut MinijinjaEngine) {
        engine
            .env
            .add_filter("snake_case", |v: Value| string_filter(&v, to_snake_case));
        engine
            .env
            .add_filter("camelCase", |v: Value| string_filter(&v, to_camel_case));
        engine
            .env
            .add_filter("PascalCase", |v: Value| string_filter(&v, to_pascal_case));
        engine
            .env
            .add_filter("kebab-case", |v: Value| string_filter(&v, to_kebab_case));
        engine.env.add_filter("CONSTANT_CASE", |v: Value| {
            string_filter(&v, to_constant_case)
        });
        // Aliases usable inside template expressions.
        engine
            .env
            .add_filter("kebab_case", |v: Value| string_filter(&v, to_kebab_case));
        engine.env.add_filter("constant_case", |v: Value| {
            string_filter(&v, to_constant_case)
        });
        engine
            .env
            .add_filter("ts_type", |v: Value| json_filter(&v, ts_type));
        engine
            .env
            .add_filter("rust_type", |v: Value| json_filter(&v, rust_type));
        engine
            .env
            .add_filter("example_of", |schema: Value, refs: Option<Value>| {
                example_filter(&schema, refs.as_ref())
            });
        engine
            .env
            .add_filter("mermaid_refs", |v: Value| json_filter(&v, mermaid_of_spec));
    }
}

/// Runs an infallible string mapping as a filter.
fn string_filter(value: &Value, f: fn(&str) -> String) -> Result<String, minijinja::Error> {
    match value.as_str() {
        Some(s) => Ok(f(s)),
        None => Err(minijinja::Error::new(
            minijinja::ErrorKind::InvalidOperation,
            format!("expected a string, got {value:?}"),
        )),
    }
}

/// Converts a filter argument into text JSON for the text-based helpers.
#[must_use]
fn json_text(value: &Json) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// Mermaid flowchart of a spec JSON value (filter-facing wrapper).
fn mermaid_of_spec(spec: &Json) -> String {
    let text = json_text(spec);
    mermaid_refs(&text)
}

/// Runs an infallible JSON mapping as a filter.
fn json_filter(value: &Value, f: fn(&Json) -> String) -> Result<String, minijinja::Error> {
    let json = serde_json::to_value(value).map_err(|e| {
        minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
    })?;
    Ok(f(&json))
}

/// The two-argument `example_of` filter: schema plus optional refs map.
fn example_filter(schema: &Value, refs: Option<&Value>) -> Result<String, minijinja::Error> {
    let to_json = |v: &Value| {
        serde_json::to_value(v).map_err(|e| {
            minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
        })
    };
    let schema = to_json(schema)?;
    let empty = Json::Object(serde_json::Map::new());
    let refs = match refs {
        Some(r) => to_json(r)?,
        None => empty,
    };
    Ok(example_of(&json_text(&schema), &json_text(&refs)))
}

// ------------------------------------------------------------------ cases

/// Splits an identifier into words on non-alphanumeric boundaries and
/// existing camel/Pascal humps (`"foo_barBaz-qux"` → `["foo","bar","Baz","qux"]`).
#[must_use]
fn split_words(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if prev.is_lowercase() || (prev.is_uppercase() && next_lower) {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Uppercases the first character of `word`.
#[must_use]
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Converts any identifier shape to `snake_case`.
#[must_use]
pub fn to_snake_case(input: &str) -> String {
    split_words(input)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// Converts any identifier shape to `camelCase`.
#[must_use]
pub fn to_camel_case(input: &str) -> String {
    split_words(input)
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.to_lowercase()
            } else {
                capitalize(w)
            }
        })
        .collect()
}

/// Converts any identifier shape to `PascalCase`.
#[must_use]
pub fn to_pascal_case(input: &str) -> String {
    split_words(input).iter().map(|w| capitalize(w)).collect()
}

/// Converts any identifier shape to `kebab-case`.
#[must_use]
pub fn to_kebab_case(input: &str) -> String {
    split_words(input)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("-")
}

/// Converts any identifier shape to `CONSTANT_CASE`.
#[must_use]
pub fn to_constant_case(input: &str) -> String {
    split_words(input)
        .iter()
        .map(|w| w.to_uppercase())
        .collect::<Vec<_>>()
        .join("_")
}

// -------------------------------------------------------------- type maps

/// Extracts the bare component name from a `$ref` such as
/// `"#/components/schemas/Pet"` → `"Pet"`; foreign refs keep their tail.
#[must_use]
pub fn ref_name(r#ref: &str) -> String {
    r#ref.rsplit('/').next().unwrap_or(r#ref).to_owned()
}

/// The base JSON `type` of a schema, tolerating type arrays
/// (`["integer", "null"]`) and missing types.
#[must_use]
fn base_type(schema: &Json) -> Option<String> {
    match schema.get("type") {
        Some(Json::String(t)) => Some(t.clone()),
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(Json::as_str)
            .find(|t| *t != "null")
            .map(str::to_owned),
        _ => None,
    }
}

/// Whether the schema is nullable: explicit `nullable: true` or a type
/// array containing `"null"`.
#[must_use]
fn is_nullable(schema: &Json) -> bool {
    if schema.get("nullable").and_then(Json::as_bool) == Some(true) {
        return true;
    }
    matches!(schema.get("type"), Some(Json::Array(items)) if items.iter().any(|v| v == "null"))
}

/// Maps an OpenAPI schema object to a TypeScript type string.
///
/// `string`→`string`, `integer`/`number`→`number`, `boolean`→`boolean`,
/// arrays→`T[]` via `items`, `$ref`s to their target name, and objects or
/// anything unrecognized to `Record<string, unknown>`. Nullable schemas
/// append `| null`.
#[must_use]
pub fn ts_type(schema: &Json) -> String {
    let base = match schema.get("$ref").and_then(Json::as_str) {
        Some(r) => ref_name(r),
        None => match base_type(schema).as_deref() {
            Some("string") => "string".into(),
            Some("integer") | Some("number") => "number".into(),
            Some("boolean") => "boolean".into(),
            Some("array") => {
                let items = schema.get("items").cloned().unwrap_or(Json::Null);
                format!("{}[]", ts_type(&items))
            }
            _ => "Record<string, unknown>".into(),
        },
    };
    if is_nullable(schema) && !base.ends_with("| null") {
        format!("{base} | null")
    } else {
        base
    }
}

/// Maps an OpenAPI schema object to a Rust type string.
///
/// `string`→`String`, `integer`→`i64`, `number`→`f64`, `boolean`→`bool`,
/// arrays→`Vec<T>` via `items`, `$ref`s to their target name, and objects
/// or anything unrecognized to `serde_json::Value`. Requiredness context is
/// not consulted here, so no `Option<T>` wrapping is applied.
#[must_use]
pub fn rust_type(schema: &Json) -> String {
    if let Some(r) = schema.get("$ref").and_then(Json::as_str) {
        return sanitize_rust_path(&ref_name(r));
    }
    match base_type(schema).as_deref() {
        Some("string") => "String".into(),
        Some("integer") => "i64".into(),
        Some("number") => "f64".into(),
        Some("boolean") => "bool".into(),
        Some("array") => {
            let items = schema.get("items").cloned().unwrap_or(Json::Null);
            format!("Vec<{}>", rust_type(&items))
        }
        _ => "serde_json::Value".into(),
    }
}

/// Normalizes a component name into valid Rust path segments.
#[must_use]
fn sanitize_rust_path(name: &str) -> String {
    name.split(['.', '-', ' '])
        .filter(|s| !s.is_empty())
        .map(to_pascal_case)
        .collect::<Vec<_>>()
        .join("::")
}

// -------------------------------------------------------------- examples

/// Resolves top-level `$ref`s in `schema` against `refs`
/// (component name → schema), bounded by `depth` indirections.
#[must_use]
fn resolve_ref(schema: &Json, refs: &serde_json::Map<String, Json>, depth: usize) -> Json {
    if depth > 8 {
        return Json::Null;
    }
    match schema.get("$ref").and_then(Json::as_str) {
        Some(r) => match refs.get(&ref_name(r)) {
            Some(target) => resolve_ref(target, refs, depth + 1),
            None => schema.clone(),
        },
        None => schema.clone(),
    }
}

/// Synthesizes a deterministic JSON example from an OpenAPI schema object.
///
/// Precedence per node: `example` > `default` > first `enum` value >
/// type defaults (`""`, `0`, `false`). Objects include every property,
/// arrays contain exactly one element, strings honor `minLength` by padding
/// with `'a'`, and `$ref`s resolve through `refs`.
#[must_use]
fn synth_example(schema: &Json, refs: &serde_json::Map<String, Json>, depth: usize) -> Json {
    if depth > 8 {
        return Json::Null;
    }
    let schema = resolve_ref(schema, refs, 0);
    if let Some(v) = schema.get("example") {
        return v.clone();
    }
    if let Some(v) = schema.get("default") {
        return v.clone();
    }
    if let Some(Json::Array(values)) = schema.get("enum")
        && let Some(first) = values.first()
    {
        return first.clone();
    }
    match base_type(&schema).as_deref() {
        Some("object") => {
            let mut out = serde_json::Map::new();
            if let Some(Json::Object(props)) = schema.get("properties") {
                for (key, prop) in props {
                    out.insert(key.clone(), synth_example(prop, refs, depth + 1));
                }
            }
            Json::Object(out)
        }
        Some("array") => Json::Array(vec![synth_example(
            schema.get("items").unwrap_or(&Json::Null),
            refs,
            depth + 1,
        )]),
        Some("string") => {
            let min_len = schema.get("minLength").and_then(Json::as_u64).unwrap_or(0);
            Json::String("a".repeat(min_len as usize))
        }
        Some("integer") | Some("number") => Json::Number(0.into()),
        Some("boolean") => Json::Bool(false),
        _ => Json::Null,
    }
}

/// Synthesizes a deterministic JSON example from serialized schema text.
///
/// `schema_json_str` is an OpenAPI schema object; `refs_json_str` is an
/// object mapping component names to schema JSON used to resolve `$ref`s.
/// Returns the example serialized as JSON text.
#[must_use]
pub fn example_of(schema_json_str: &str, refs_json_str: &str) -> String {
    let schema: Json = serde_json::from_str(schema_json_str).unwrap_or(Json::Null);
    let refs: Json = serde_json::from_str(refs_json_str).unwrap_or(Json::Null);
    let empty = serde_json::Map::new();
    let example = synth_example(&schema, refs.as_object().unwrap_or(&empty), 0);
    serde_json::to_string(&example).unwrap_or_else(|_| "null".into())
}

// --------------------------------------------------------------- mermaid

/// Renders the schema dependency graph of a serialized
/// [`IrSpec`](suspect_ir::IrSpec) as a Mermaid flowchart.
///
/// Reuses the precomputed `schema_edges` map when present; otherwise walks
/// each schema's JSON for `$ref` occurrences. Edges are deduplicated and
/// sorted, one `From --> To` line each under a `flowchart TD` header.
#[must_use]
pub fn mermaid_refs(spec_json_str: &str) -> String {
    let spec: Json = serde_json::from_str(spec_json_str).unwrap_or(Json::Null);
    let mut edges = BTreeSet::new();
    if let Some(map) = spec.get("schema_edges").and_then(Json::as_object) {
        for (from, tos) in map {
            for to in tos
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
            {
                edges.insert((from.clone(), to.to_owned()));
            }
        }
    } else if let Some(schemas) = spec.get("schemas").and_then(Json::as_array) {
        for entry in schemas {
            let (Some(from), Some(json)) =
                (entry.get("name").and_then(Json::as_str), entry.get("json"))
            else {
                continue;
            };
            walk_ref_names(json, &mut |to| {
                if to != from {
                    edges.insert((from.to_owned(), to.to_owned()));
                }
            });
        }
    }
    let mut out = String::from("flowchart TD\n");
    for (from, to) in &edges {
        out.push_str("  ");
        out.push_str(&sanitize_mermaid_id(from));
        out.push_str(" --> ");
        out.push_str(&sanitize_mermaid_id(to));
        out.push('\n');
    }
    out
}

/// Visits every `$ref` target name in `json`.
fn walk_ref_names(json: &Json, out: &mut impl FnMut(&str)) {
    match json {
        Json::Object(map) => {
            if let Some(Json::String(r)) = map.get("$ref") {
                out(&ref_name(r));
            }
            map.values().for_each(|v| walk_ref_names(v, out));
        }
        Json::Array(items) => items.iter().for_each(|v| walk_ref_names(v, out)),
        _ => {}
    }
}

/// Quotes identifiers Mermaid would otherwise reject.
#[must_use]
fn sanitize_mermaid_id(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}
