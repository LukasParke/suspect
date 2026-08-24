//! Deterministic example synthesis and the precompiled mock table.
//!
//! Mock bodies are **precompiled at startup**: every declared response of
//! every [`IrOperation`](suspect_ir::IrOperation) gets its JSON example
//! synthesized once from the materialized IR schema JSON and stored as
//! bytes; serving is then a lookup plus a copy, with no per-request
//! synthesis cost or nondeterminism.
//!
//! Synthesis precedence per node mirrors
//! [`suspect_gen::example_of`]: `example` > `default` > first `enum`
//! value > type defaults (`""`, `0`, `false`). Objects include every
//! declared property, arrays contain exactly one element, strings honor
//! `minLength` by padding with `'a'`, and `$ref`s resolve through the
//! spec's component-schema lookup map. Recursion (including self- and
//! mutually-referential schemas) is bounded by [`DEPTH_CAP`] — past the
//! cap synthesis emits `null`.

use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::Value;
use suspect_ir::{IrSpec, Method};

use crate::problem;

/// Component-name → materialized schema JSON, used to resolve local `$ref`s.
pub type SchemaRefs = HashMap<String, Value>;

/// Maximum synthesis recursion depth; deeper nesting collapses to `null`.
///
/// A fixed cap keeps synthesis total on cyclic schemas (`Pet.friend ->
/// Pet`) while still producing useful examples for shallow trees.
pub const DEPTH_CAP: u8 = 6;

/// One precompiled mock response: status (`None` for `default`) plus body.
#[derive(Debug, Clone)]
pub struct CompiledResponse {
    /// Declared status code; `None` represents the `default` response.
    pub status: Option<u16>,
    /// Precompiled example body bytes (always JSON).
    pub body: Bytes,
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
    if depth > DEPTH_CAP {
        return Value::Null;
    }
    if let Some(target) = schema.get("$ref").and_then(Value::as_str) {
        let name = target.rsplit('/').next().unwrap_or(target);
        return match refs.get(name) {
            Some(component) => synth_example(component, refs, depth + 1),
            None => Value::Null,
        };
    }
    if let Some(v) = schema.get("example") {
        return v.clone();
    }
    if let Some(v) = schema.get("default") {
        return v.clone();
    }
    if let Some(Value::Array(values)) = schema.get("enum")
        && let Some(first) = values.first()
    {
        return first.clone();
    }
    match schema_type(schema).as_deref() {
        Some("object") => {
            let mut out = serde_json::Map::new();
            if let Some(Value::Object(props)) = schema.get("properties") {
                for (key, prop) in props {
                    out.insert(key.clone(), synth_example(prop, refs, depth + 1));
                }
            }
            Value::Object(out)
        }
        Some("array") => Value::Array(vec![synth_example(
            schema.get("items").unwrap_or(&Value::Null),
            refs,
            depth + 1,
        )]),
        Some("string") => {
            let min_len = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0);
            Value::String("a".repeat(min_len as usize))
        }
        Some("integer" | "number") => Value::Number(0.into()),
        Some("boolean") => Value::Bool(false),
        _ => Value::Null,
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
                .map(|resp| {
                    let example = match &resp.schema {
                        // Unresolvable component names degrade to `null`, matching
                        // the "external refs stay opaque" policy of suspect-ir.
                        Some(name) => spec
                            .schema(name)
                            .map_or(Value::Null, |ir| synth_example(&ir.json, &refs, 0)),
                        None => Value::Null,
                    };
                    let body = Bytes::from(
                        serde_json::to_vec(&example).unwrap_or_else(|_| b"null".to_vec()),
                    );
                    CompiledResponse {
                        status: resp.status,
                        body,
                    }
                })
                .collect::<Vec<_>>();
            ((op.method, op.path.clone()), compiled)
        })
        .collect()
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

/// Builds the served response for one matched operation.
///
/// Falls back to `501 Not Implemented` problem+json when the operation
/// declares no responses to synthesize from.
#[must_use]
pub fn respond(compiled: &[CompiledResponse]) -> Response {
    match best(compiled) {
        Some(c) => {
            let status = StatusCode::from_u16(c.status.unwrap_or(200)).unwrap_or(StatusCode::OK);
            (
                status,
                [("content-type", "application/json")],
                c.body.clone(),
            )
                .into_response()
        }
        None => problem(
            StatusCode::NOT_IMPLEMENTED,
            "No synthesized response",
            Some("the operation declares no responses".to_owned()),
        ),
    }
}
