//! IR construction from the fast YAML-subset tree.
//!
//! [`ir_from_fast`] walks a `FastValue` tree with exactly the same rules
//! as [`IrSpec::from_workspace`] applied to a
//! CST: component schemas first, then operations in document order, with
//! identical parameter merging, response ordering, `$ref` naming (including
//! percent-decoding), and index-map population. the `value_from_node` adapter
//! resolved CST nodes into `FastValue` so both pipelines share one walk.

use rayon::prelude::*;

use crate::common::{collect_local_refs, local_schema_ref, method_key, scalar_json};
use crate::{IrOperation, IrParameter, IrResponse, IrSchema, IrSpec, Method, ParamIn};
use suspect_low::NodeRef;
use suspect_syntax::FastValue;

/// Parallel conversion kicks in above these entry counts.
const PAR_SCHEMA_THRESHOLD: usize = 64;
const PAR_PATH_THRESHOLD: usize = 32;

/// Mirrors `LowDoc::sniff_family` for the fast tree: `true` only when the
/// root declares `openapi: 3.x`.
#[must_use]
pub fn is_oas3(root: &FastValue) -> bool {
    match root.get("openapi").and_then(as_str) {
        Some(v) => v.starts_with("3.0") || v.starts_with("3.1") || v.starts_with("3.2"),
        None => false,
    }
}

/// Builds an [`IrSpec`] from a fast-parsed document root.
///
/// Non-object roots yield an empty spec, mirroring how `from_workspace`
/// degrades when lookups miss. The caller is responsible for the OpenAPI
/// 3.x family check.
#[must_use]
pub fn ir_from_fast(value: &FastValue) -> IrSpec {
    let mut spec = IrSpec::default();
    if !matches!(value, FastValue::Object(_)) {
        return spec;
    }

    if let Some(info) = get(value, "info") {
        spec.title = get(info, "title").and_then(as_str).unwrap_or("").to_owned();
        spec.version = get(info, "version")
            .and_then(as_str)
            .unwrap_or("")
            .to_owned();
    }
    if let Some(servers) = get(value, "servers") {
        spec.servers = servers
            .items()
            .iter()
            .filter_map(|s| get(s, "url").and_then(as_str).map(str::to_owned))
            .collect();
    }

    // Component schemas first: operations reference them by name.
    if let Some(components) = get(value, "components")
        && let Some(schemas_node) = get(components, "schemas")
    {
        let entries = schemas_node.entries();
        // Materialization and reference collection run together across
        // threads; index maps are filled sequentially in document order.
        let convert = |(name, v): &(String, FastValue)| -> (IrSchema, Vec<String>) {
            let json_value = json(Some(v));
            let refs = collect_local_refs(&json_value);
            (
                IrSchema {
                    name: name.clone(),
                    json: json_value,
                },
                refs,
            )
        };
        let converted: Vec<(IrSchema, Vec<String>)> = if entries.len() > PAR_SCHEMA_THRESHOLD {
            entries.par_iter().map(convert).collect()
        } else {
            entries.iter().map(convert).collect()
        };
        for (schema, refs) in converted {
            let idx = spec.schemas.len() as u32;
            spec.schema_index.insert(schema.name.clone(), idx);
            spec.schema_edges.insert(schema.name.clone(), refs);
            spec.schemas.push(schema);
        }
    }

    // Operations.
    if let Some(paths) = get(value, "paths") {
        let entries = paths.entries();
        let per_path: Vec<Vec<IrOperation>> = if entries.len() > PAR_PATH_THRESHOLD {
            entries
                .par_iter()
                .map(|(path, item)| path_operations(path, item))
                .collect()
        } else {
            entries
                .iter()
                .map(|(path, item)| path_operations(path, item))
                .collect()
        };
        for ops in per_path {
            for op in ops {
                let idx = spec.operations.len() as u32;
                if let Some(id) = &op.id {
                    spec.by_operation_id.insert(id.clone(), idx);
                }
                spec.by_method_path
                    .insert((op.method, op.path.clone()), idx);
                spec.operations.push(op);
            }
        }
    }
    spec
}

/// All operations of one path item, canonical method order.
fn path_operations(path: &str, item: &FastValue) -> Vec<IrOperation> {
    let mut out = Vec::new();
    if !matches!(item, FastValue::Object(_)) {
        return out;
    }
    let item_params = parameters_of(item);
    for method in Method::ALL {
        let Some(op) = get(item, method_key(method)) else {
            continue;
        };
        let mut params = item_params.clone();
        params.extend(parameters_of(op));
        out.push(IrOperation {
            id: get(op, "operationId").and_then(as_str).map(str::to_owned),
            method,
            path: path.to_owned(),
            summary: get(op, "summary").and_then(as_str).map(str::to_owned),
            description: get(op, "description").and_then(as_str).map(str::to_owned),
            tags: get(op, "tags")
                .map(|t| {
                    t.items()
                        .iter()
                        .filter_map(as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            deprecated: get(op, "deprecated").and_then(as_bool).unwrap_or(false),
            parameters: params,
            body_schema: get(op, "requestBody")
                .and_then(|rb| get(rb, "content"))
                .and_then(|c| get(c, "application/json"))
                .and_then(|j| get(j, "schema"))
                .and_then(ref_name),
            responses: responses_of(op),
        });
    }
    out
}

/// First mapping entry stored under `key`.
fn get<'v>(value: &'v FastValue, key: &str) -> Option<&'v FastValue> {
    value.get(key)
}

/// Scalar text; non-scalars yield `None` like `NodeRef::as_str` on
/// composite nodes.
fn as_str(value: &FastValue) -> Option<&str> {
    match value {
        FastValue::Scalar { raw, .. } => Some(raw),
        _ => None,
    }
}

/// Core-schema boolean lookup, mirroring `NodeRef::as_bool`: only unquoted
/// YAML spellings count.
fn as_bool(value: &FastValue) -> Option<bool> {
    match value {
        FastValue::Scalar { raw, quoted: false } => match raw.as_bytes() {
            b"true" | b"True" | b"TRUE" => Some(true),
            b"false" | b"False" | b"FALSE" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Resolves a schema node's `$ref` to a local component name.
fn ref_name(node: &FastValue) -> Option<String> {
    get(node, "$ref")
        .and_then(as_str)
        .and_then(local_schema_ref)
}

/// Materializes any fast value into owned JSON (`None` → JSON null).
pub(crate) fn json(value: Option<&FastValue>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    match value {
        FastValue::Scalar { raw, quoted } => scalar_json(raw, *quoted),
        FastValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            out.extend(items.iter().map(|v| json(Some(v))));
            serde_json::Value::Array(out)
        }
        FastValue::Object(entries) => {
            let mut map = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                map.insert(k.clone(), json(Some(v)));
            }
            serde_json::Value::Object(map)
        }
    }
}

/// One operation parameter, mirroring the CST-based filter rules.
fn parameter(param: &FastValue) -> Option<IrParameter> {
    let name = get(param, "name").and_then(as_str)?;
    let location = match get(param, "in").and_then(as_str)? {
        "query" => ParamIn::Query,
        "header" => ParamIn::Header,
        "path" => ParamIn::Path,
        "cookie" => ParamIn::Cookie,
        _ => return None,
    };
    Some(IrParameter {
        name: name.to_owned(),
        location,
        required: get(param, "required").and_then(as_bool).unwrap_or(false),
        schema: get(param, "schema").map(|v| json(Some(v))),
    })
}

fn parameters_of(node: &FastValue) -> Vec<IrParameter> {
    get(node, "parameters")
        .map(|p| p.items().iter().filter_map(parameter).collect())
        .unwrap_or_default()
}

/// Responses sorted by numeric status ascending, `default` last.
fn responses_of(op: &FastValue) -> Vec<IrResponse> {
    let Some(responses) = get(op, "responses") else {
        return Vec::new();
    };
    let mut entries: Vec<(Option<u16>, IrResponse)> = responses
        .entries()
        .iter()
        .map(|(key, value)| {
            let status = key.parse::<u16>().ok();
            (
                status,
                IrResponse {
                    status,
                    description: get(value, "description")
                        .and_then(as_str)
                        .map(str::to_owned),
                    schema: get(value, "content")
                        .and_then(|c| get(c, "application/json"))
                        .and_then(|j| get(j, "schema"))
                        .and_then(ref_name),
                },
            )
        })
        .collect();
    entries.sort_by_key(|(status, _)| status.unwrap_or(u16::MAX));
    entries.into_iter().map(|(_, r)| r).collect()
}

/// Converts a resolved CST node to the same decoded values as the fast reader.
///
/// Invalid string escapes or UTF-8 return a source-linked error instead of
/// silently changing the scalar's contents.
pub(crate) fn value_from_node(node: NodeRef<'_>) -> Result<FastValue, String> {
    use suspect_low::ValueKind;
    use suspect_syntax::ScalarStyle;

    let resolved = node.resolved();
    Ok(match resolved.kind() {
        ValueKind::Object => FastValue::Object(
            resolved
                .entries()
                .into_iter()
                .map(|entry| {
                    let key = decoded_text(entry.key_node)?;
                    let value = match entry.value {
                        Some(child) => value_from_node(child)?,
                        None => FastValue::null(),
                    };
                    Ok((key, value))
                })
                .collect::<Result<_, String>>()?,
        ),
        ValueKind::Array => FastValue::Array(
            resolved
                .items()
                .into_iter()
                .map(value_from_node)
                .collect::<Result<_, _>>()?,
        ),
        _ => FastValue::Scalar {
            raw: decoded_text(resolved)?,
            quoted: resolved.syntax().scalar_style() != ScalarStyle::Plain,
        },
    })
}

fn decoded_text(node: NodeRef<'_>) -> Result<String, String> {
    let invalid = || {
        format!(
            "invalid scalar at {} bytes {}..{}",
            node.syntax().doc().uri(),
            node.byte_range().start,
            node.byte_range().end
        )
    };
    let bytes = node.try_decoded_scalar().ok_or_else(invalid)?;
    String::from_utf8(bytes.into_owned()).map_err(|_| invalid())
}
