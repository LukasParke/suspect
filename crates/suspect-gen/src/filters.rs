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
            .add_filter("scalar_example", |v: Value| json_filter(&v, scalar_example));
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
/// arrays→`T[]` via `items` (parenthesized when the item type contains a
/// space, e.g. `(string | null)[]`), `$ref`s to their target name, and
/// objects or anything unrecognized to `Record<string, unknown>`.
/// Nullable schemas append `| null`.
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
                let item = ts_type(&items);
                if item.contains(' ') {
                    format!("({item})[]")
                } else {
                    format!("{item}[]")
                }
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
/// or anything unrecognized to `serde_json::Value`. Schema-level
/// nullability (`nullable: true` or a type array containing `"null"`)
/// wraps the base type in `Option<T>`, so nullable array items yield
/// `Vec<Option<T>>`. A leading `Option<` is never doubled. Requiredness
/// context is not consulted here; callers wrap optional fields themselves
/// and should skip wrapping when the result is already an `Option`.
#[must_use]
pub fn rust_type(schema: &Json) -> String {
    let ty = if let Some(r) = schema.get("$ref").and_then(Json::as_str) {
        sanitize_rust_path(&ref_name(r))
    } else {
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
    };
    if is_nullable(schema) && !ty.starts_with("Option<") {
        format!("Option<{ty}>")
    } else {
        ty
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
/// (component name → schema), bounded by `depth` indirections. When the
/// bound is exhausted (e.g. a reference cycle), the schema is returned
/// as-is rather than collapsed to null.
#[must_use]
fn resolve_ref(schema: &Json, refs: &serde_json::Map<String, Json>, depth: usize) -> Json {
    if depth > 8 {
        return schema.clone();
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
    // Named `$ref`: memoize one synthesis per component per refs payload.
    // `component_start` marks the slot before recursing so cycles resolve
    // to null (depth-capped as before) without poisoning the cache, and no
    // borrow is held across the recursive call.
    if let Some(name) = schema.get("$ref").and_then(Json::as_str).map(ref_name) {
        let cached = EXAMPLE_CACHE.with(|c| c.borrow().component_hit(&name));
        match cached {
            Some(Some(example)) => return example,
            Some(None) => return Json::Null, // in flight: cycle
            None => {}
        }
        if refs.contains_key(&name) {
            EXAMPLE_CACHE.with(|c| c.borrow_mut().component_start(&name));
            let target = &refs[&name];
            // Same depth: cycle safety comes from the in-flight marker;
            // pure chain length must not eat the synthesis-depth budget.
            let example = synth_example(target, refs, depth);
            EXAMPLE_CACHE.with(|c| c.borrow_mut().component_finish(&name, &example));
            return example;
        }
        return schema.clone();
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

// ------------------------------------------------------- example memoization
//
// `example_of` runs once per table cell in generated docs; on large specs
// the same component schemas resolve thousands of times. A thread-local
// cache keyed by the refs payload (invalidated when it changes) stores the
// synthesized example for every distinct schema input and every resolved
// component, turning repeated property rows into hash lookups.

thread_local! {
    static EXAMPLE_CACHE: std::cell::RefCell<ExampleCache> =
        std::cell::RefCell::new(ExampleCache::default());
}

#[derive(Default)]
struct ExampleCache {
    /// Hash of the refs payload the cache is valid for.
    refs_key: u64,
    /// Synthesized examples by hash of the schema input.
    by_input: std::collections::HashMap<u64, Json>,
    /// Synthesized component examples by name; `None` while a computation
    /// for that name is in flight (cycle guard).
    components: std::collections::HashMap<String, Option<Json>>,
}

impl ExampleCache {
    fn component_hit(&self, name: &str) -> Option<Option<Json>> {
        // Borrow is dropped before any recursion can re-enter.
        self.components.get(name).cloned()
    }
    fn component_start(&mut self, name: &str) {
        self.components.insert(name.to_owned(), None);
    }
    /// Stores a completed example unless it is null (nulls stay uncached so
    /// cyclic/depth-capped paths never poison later lookups).
    fn component_finish(&mut self, name: &str, value: &Json) {
        if !value.is_null() {
            self.components.insert(name.to_owned(), Some(value.clone()));
        } else {
            self.components.remove(name);
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Shallow literal for one docs-table cell: type default without
/// recursion (`string` -> `""`, `integer` -> `0`, `boolean` -> `false`,
/// arrays -> `[]`, objects -> `{}`, `$ref`/unknown -> ellipsis). Deep
/// component examples live in the context map instead.
#[must_use]
pub fn scalar_example(schema: &Json) -> String {
    if schema.get("$ref").is_some() {
        return "...".to_owned();
    }
    if let Some(v) = schema.get("example") {
        return v.to_string();
    }
    match base_type(schema).as_deref() {
        Some("string") => "\"\"".to_owned(),
        Some("integer") | Some("number") => "0".to_owned(),
        Some("boolean") => "false".to_owned(),
        Some("array") => "[]".to_owned(),
        Some("object") => "{}".to_owned(),
        _ => "null".to_owned(),
    }
}

/// Computes deep examples for every component in one pass.
///
/// Context builders call this once instead of templates calling
/// [`example_of`] per table cell: the thread-local cache makes each
/// distinct component synthesize exactly once, and `$ref` chains share
/// results across components.
#[must_use]
pub fn examples_for_components(
    components: &serde_json::Map<String, Json>,
) -> serde_json::Map<String, Json> {
    let mut out = serde_json::Map::new();
    EXAMPLE_CACHE.with(|cache| {
        let refs_text =
            serde_json::to_string(&Json::Object(components.clone())).unwrap_or_default();
        let key = fnv1a(refs_text.as_bytes());
        if cache.borrow().refs_key != key {
            cache.borrow_mut().refs_key = key;
            cache.borrow_mut().by_input.clear();
            cache.borrow_mut().components.clear();
        }
    });
    for (name, schema) in components {
        let ex = synth_example(schema, components, 0);
        out.insert(name.clone(), Json::String(ex.to_string()));
    }
    out
}

/// Synthesizes a deterministic example for a serialized schema,
/// resolving component references through the serialized refs map.
///
/// Memoized per `(refs payload, schema input)` in a thread-local cache.
#[must_use]
pub fn example_of(schema_json_str: &str, refs_json_str: &str) -> String {
    let schema: Json = serde_json::from_str(schema_json_str).unwrap_or(Json::Null);
    let refs: Json = serde_json::from_str(refs_json_str).unwrap_or(Json::Null);
    let refs_key = fnv1a(refs_json_str.as_bytes());
    let schema_key = fnv1a(schema_json_str.as_bytes());
    let empty = serde_json::Map::new();
    let refs_map = refs.as_object().unwrap_or(&empty);

    // Phase 1: invalidate + lookup. The borrow is dropped before
    // synthesis so recursive component lookups cannot double-borrow.
    let hit = EXAMPLE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.refs_key != refs_key {
            *cache = ExampleCache {
                refs_key,
                ..ExampleCache::default()
            };
        }
        cache.by_input.get(&schema_key).cloned()
    });
    if let Some(example) = hit {
        return serde_json::to_string(&example).unwrap_or_else(|_| "null".into());
    }

    // Phase 2: synthesize with only short-lived internal borrows.
    let example = synth_example(&schema, refs_map, 0);

    // Phase 3: store.
    EXAMPLE_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .by_input
            .insert(schema_key, example.clone());
    });
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
