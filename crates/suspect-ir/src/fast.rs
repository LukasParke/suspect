//! IR construction from the fast YAML-subset tree.
//!
//! [`ir_from_fast`] walks a `FastValue` tree with exactly the same rules
//! as [`IrSpec::from_workspace`] applied to a
//! CST: component schemas first, then operations in document order, with
//! identical parameter merging, response ordering, `$ref` naming (including
//! percent-decoding), and index-map population. the `value_from_node` adapter
//! resolved CST nodes into `FastValue` so both pipelines share one walk.

use rayon::prelude::*;

use crate::common::{
    collect_local_refs, local_component_ref, local_schema_ref, method_key, scalar_json,
};
use crate::{
    IrHeader, IrMedia, IrOperation, IrParameter, IrResponse, IrSchema, IrSpec, Method, ParamIn,
    json_media,
};
use suspect_low::NodeRef;
use suspect_syntax::FastValue;

/// Parallel conversion kicks in above these entry counts.
const PAR_SCHEMA_THRESHOLD: usize = 64;
const PAR_PATH_THRESHOLD: usize = 32;

/// Longest chain of component-to-component `$ref`s followed before the
/// node is left as written; bounds reference cycles.
const MAX_REF_HOPS: usize = 8;

/// The document's `components` object, through which local references to
/// reusable responses, parameters, request bodies, headers and path items
/// resolve. Schema references are not followed here: schemas stay
/// addressable by name.
#[derive(Clone, Copy)]
struct Components<'v>(Option<&'v FastValue>);

impl<'v> Components<'v> {
    /// The node a `#/components/<section>/<name>` reference chain leads
    /// to. A node without `$ref` is returned as is, and so is one whose
    /// reference is not local to `section` or names nothing.
    fn deref(self, node: &'v FastValue, section: &str) -> &'v FastValue {
        let mut current = node;
        for _ in 0..MAX_REF_HOPS {
            let target = get(current, "$ref")
                .and_then(as_str)
                .and_then(|reference| local_component_ref(reference, section))
                .and_then(|name| {
                    self.0
                        .and_then(|components| get(components, section))
                        .and_then(|entries| get(entries, &name))
                });
            match target {
                Some(next) => current = next,
                None => break,
            }
        }
        current
    }
}

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

    spec.openapi = get(value, "openapi")
        .and_then(as_str)
        .unwrap_or("")
        .to_owned();
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

    // Operations. Inline response and request-body schemas are registered
    // under generated names in the same pass, so they stay addressable
    // exactly like component schemas — the gateway mock and contract
    // tooling synthesize from them without a `$ref` in sight.
    if let Some(paths) = get(value, "paths") {
        let components = Components(get(value, "components"));
        let entries = paths.entries();
        let per_path: Vec<PathContribution> = if entries.len() > PAR_PATH_THRESHOLD {
            entries
                .par_iter()
                .map(|(path, item)| path_operations(path, item, components))
                .collect()
        } else {
            entries
                .iter()
                .map(|(path, item)| path_operations(path, item, components))
                .collect()
        };
        for (ops, inlines) in per_path {
            for (schema, refs) in inlines {
                let idx = spec.schemas.len() as u32;
                spec.schema_index.insert(schema.name.clone(), idx);
                spec.schema_edges.insert(schema.name.clone(), refs);
                spec.schemas.push(schema);
            }
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

/// One path item's contribution to the spec: its operations plus the
/// inline schemas those operations registered.
type PathContribution = (Vec<IrOperation>, Vec<(IrSchema, Vec<String>)>);

/// All operations of one path item, canonical method order, plus the
/// inline schemas those operations registered.
fn path_operations(path: &str, item: &FastValue, components: Components<'_>) -> PathContribution {
    let mut out = Vec::new();
    let mut inlines: Vec<(IrSchema, Vec<String>)> = Vec::new();
    let item = components.deref(item, "pathItems");
    if !matches!(item, FastValue::Object(_)) {
        return (out, inlines);
    }
    let item_params = parameters_of(item, components);
    for method in Method::ALL {
        let Some(op) = get(item, method_key(method)) else {
            continue;
        };
        let mut params = item_params.clone();
        params.extend(parameters_of(op, components));
        let request_content = get(op, "requestBody")
            .map(|body| components.deref(body, "requestBodies"))
            .map(|body| content_of(body, &mut inlines, path, method, "request"))
            .unwrap_or_default();
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
            body_schema: json_media(&request_content).and_then(|media| media.schema.clone()),
            responses: responses_of(op, path, method, &mut inlines, components),
        });
    }
    (out, inlines)
}

/// Every media type of a response or request body's `content` map, in
/// document order, each schema registered under an addressable name.
///
/// `slot` names the body within its operation (`request`, `response200`).
/// The `application/json` entry keeps that slot as its inline-schema
/// name; every other media type appends its own escaped key, so two
/// inline schemas of one body never collide.
fn content_of(
    body: &FastValue,
    inlines: &mut Vec<(IrSchema, Vec<String>)>,
    path: &str,
    method: Method,
    slot: &str,
) -> Vec<IrMedia> {
    let Some(content) = get(body, "content") else {
        return Vec::new();
    };
    content
        .entries()
        .iter()
        .map(|(media_type, media)| {
            let mut entry = IrMedia {
                media_type: media_type.clone(),
                ..IrMedia::default()
            };
            if let Some(node) = get(media, "schema") {
                let media_slot = if media_type == "application/json" {
                    slot.to_owned()
                } else {
                    format!("{slot}~~{}", escape_path(media_type))
                };
                entry.schema = named_schema(node, inlines, path, method, &media_slot);
                if entry.schema.is_none() {
                    entry.unresolved = get(node, "$ref").and_then(as_str).map(str::to_owned);
                }
            }
            entry
        })
        .collect()
}

/// The addressable name for one schema node: a `$ref` keeps its target
/// name; an inline schema is materialized and registered under a
/// deterministic generated name unique to its operation and slot.
fn named_schema(
    node: &FastValue,
    inlines: &mut Vec<(IrSchema, Vec<String>)>,
    path: &str,
    method: Method,
    slot: &str,
) -> Option<String> {
    // A `$ref` — local or external — is never inline material: locals keep
    // their component name; externals stay opaque (None), matching the
    // "external refs stay unresolved" policy of the fast reader.
    if get(node, "$ref").is_some() {
        return ref_name(node);
    }
    let name = format!(
        "~~inline~~{}~~{}~~{slot}",
        escape_path(path),
        method_key(method),
    );
    let json_value = json(Some(node));
    let refs = collect_local_refs(&json_value);
    inlines.push((
        IrSchema {
            name: name.clone(),
            json: json_value,
        },
        refs,
    ));
    Some(name)
}

/// Path text safe for a schema name: hex-escapes everything outside
/// `[A-Za-z0-9]`, so the mapping is injective (`~~` never appears in the
/// escaped form).
fn escape_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(byte as char);
        } else {
            out.push_str(&format!("-{byte:02x}-"));
        }
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

fn parameters_of(node: &FastValue, components: Components<'_>) -> Vec<IrParameter> {
    get(node, "parameters")
        .map(|p| {
            p.items()
                .iter()
                .filter_map(|item| parameter(components.deref(item, "parameters")))
                .collect()
        })
        .unwrap_or_default()
}

/// A `responses` key as `(exact status, range hundreds digit)`: `"404"` is
/// `(Some(404), None)`, `"4XX"` is `(None, Some(4))`, and `default` (or any
/// other key) is `(None, None)`.
fn status_key(key: &str) -> (Option<u16>, Option<u8>) {
    if let Ok(status) = key.parse::<u16>() {
        return (Some(status), None);
    }
    match key.as_bytes() {
        [class @ b'1'..=b'5', b'X' | b'x', b'X' | b'x'] => (None, Some(class - b'0')),
        _ => (None, None),
    }
}

/// Declared headers of one response, `$ref`s to `components/headers`
/// followed.
fn headers_of(response: &FastValue, components: Components<'_>) -> Vec<IrHeader> {
    let Some(headers) = get(response, "headers") else {
        return Vec::new();
    };
    headers
        .entries()
        .iter()
        .map(|(name, header)| {
            let header = components.deref(header, "headers");
            IrHeader {
                name: name.clone(),
                required: get(header, "required").and_then(as_bool).unwrap_or(false),
                schema: get(header, "schema").map(|schema| json(Some(schema))),
            }
        })
        .collect()
}

/// Responses in selection order: exact statuses ascending, then status
/// ranges ascending, then `default`. A `$ref` to `components/responses`
/// is followed, so a shared response contributes its content and headers.
fn responses_of(
    op: &FastValue,
    path: &str,
    method: Method,
    inlines: &mut Vec<(IrSchema, Vec<String>)>,
    components: Components<'_>,
) -> Vec<IrResponse> {
    let Some(responses) = get(op, "responses") else {
        return Vec::new();
    };
    let mut entries: Vec<IrResponse> = responses
        .entries()
        .iter()
        .map(|(key, value)| {
            let value = components.deref(value, "responses");
            let (status, range) = status_key(key);
            let content = content_of(value, inlines, path, method, &format!("response{key}"));
            IrResponse {
                status,
                range,
                description: get(value, "description")
                    .and_then(as_str)
                    .map(str::to_owned),
                schema: json_media(&content).and_then(|media| media.schema.clone()),
                content,
                headers: headers_of(value, components),
            }
        })
        .collect();
    entries.sort_by_key(|response| match (response.status, response.range) {
        (Some(status), _) => (0u8, status),
        (None, Some(range)) => (1, u16::from(range)),
        (None, None) => (2, 0),
    });
    entries
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
