//! Normalized, indexed document model shared by the generator, test
//! executor, gateway, and language server.
//!
//! [`IrSpec::from_workspace`] walks an OpenAPI 3.x entry document (plus its
//! loaded `$ref` closure) once and materializes an **owned** snapshot:
//! operations indexed by `operationId` and by `(method, path)`, component
//! schemas as plain JSON, and the schema dependency graph. Downstream
//! consumers never touch document lifetimes again — they query the index.
//!
//! Resolution policy: local `#/components/schemas/{name}` references are
//! resolved to the bare component name; references into other files stay
//! unresolved (`None`) in v1 — presets treat those schemas as opaque
//! objects. Cross-file resolution is a planned upgrade behind the same API.

use std::collections::HashMap;
use std::sync::Arc;

use suspect_low::{NodeRef, SpecFamily};
use suspect_ref::Workspace;
use suspect_source::Uri;

/// HTTP method of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    /// GET.
    Get,
    /// PUT.
    Put,
    /// POST.
    Post,
    /// DELETE.
    Delete,
    /// OPTIONS.
    Options,
    /// HEAD.
    Head,
    /// PATCH.
    Patch,
    /// TRACE.
    Trace,
}

impl Method {
    /// Every method, in canonical order.
    pub const ALL: [Method; 8] = [
        Method::Get,
        Method::Put,
        Method::Post,
        Method::Delete,
        Method::Options,
        Method::Head,
        Method::Patch,
        Method::Trace,
    ];

    /// Uppercase wire name (`"GET"`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Post => "POST",
            Method::Delete => "DELETE",
            Method::Options => "OPTIONS",
            Method::Head => "HEAD",
            Method::Patch => "PATCH",
            Method::Trace => "TRACE",
        }
    }

    /// Parses a lowercase/uppercase spec key (`"get"`).
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "get" => Method::Get,
            "put" => Method::Put,
            "post" => Method::Post,
            "delete" => Method::Delete,
            "options" => Method::Options,
            "head" => Method::Head,
            "patch" => Method::Patch,
            "trace" => Method::Trace,
            _ => return None,
        })
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a parameter lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamIn {
    /// Query string parameter.
    Query,
    /// Header parameter.
    Header,
    /// Path template parameter.
    Path,
    /// Cookie parameter.
    Cookie,
}

/// One operation parameter.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter location.
    pub location: ParamIn,
    /// Whether it is required.
    pub required: bool,
    /// Materialized schema JSON, when declared inline or resolvable locally.
    pub schema: Option<serde_json::Value>,
}

/// One response definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrResponse {
    /// Status code; `None` for `default`.
    pub status: Option<u16>,
    /// Response description.
    pub description: Option<String>,
    /// `application/json` schema reference resolved to a component name.
    pub schema: Option<String>,
}

/// One operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrOperation {
    /// `operationId`, when present.
    pub id: Option<String>,
    /// HTTP method.
    pub method: Method,
    /// Path template including `{param}` placeholders.
    pub path: String,
    /// Short summary.
    pub summary: Option<String>,
    /// Long description.
    pub description: Option<String>,
    /// Tags for grouping.
    pub tags: Vec<String>,
    /// Deprecated flag.
    pub deprecated: bool,
    /// Merged path-item + operation parameters.
    pub parameters: Vec<IrParameter>,
    /// `application/json` request body schema component name.
    pub body_schema: Option<String>,
    /// Declared responses.
    pub responses: Vec<IrResponse>,
}

/// One component schema, materialized to plain JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrSchema {
    /// Component name (`components/schemas/{name}`).
    pub name: String,
    /// Schema JSON (owned snapshot).
    pub json: serde_json::Value,
}

/// How to look an operation up.
#[derive(Debug, Clone, Copy)]
pub enum OpSelector<'a> {
    /// By `operationId`.
    Id(&'a str),
    /// By method + path pair.
    MethodPath(Method, &'a str),
}

/// The normalized spec snapshot.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IrSpec {
    /// `info.title`.
    pub title: String,
    /// `info.version`.
    pub version: String,
    /// Server URLs.
    pub servers: Vec<String>,
    /// All operations, document order.
    pub operations: Vec<IrOperation>,
    /// Component schemas, document order.
    pub schemas: Vec<IrSchema>,
    /// `operationId` -> index into [`IrSpec::operations`].
    pub by_operation_id: HashMap<String, u32>,
    /// `(method, path)` -> index.
    pub by_method_path: HashMap<(Method, String), u32>,
    /// Schema name -> index into [`IrSpec::schemas`].
    pub schema_index: HashMap<String, u32>,
    /// Schema name -> locally-resolved referenced names (dependency graph).
    pub schema_edges: HashMap<String, Vec<String>>,
}

impl IrSpec {
    /// Builds the IR from one OAS 3.x entry document inside `ws`.
    ///
    /// # Errors
    /// `"not an OpenAPI 3.x document"` when the family sniff fails; other
    /// failures degrade to empty collections rather than erroring — partial
    /// specs still produce partial IR.
    pub fn from_workspace(ws: &Arc<Workspace>, entry_uri: &Uri) -> Result<IrSpec, String> {
        let handle = ws
            .get(entry_uri)
            .ok_or_else(|| format!("document not loaded: {entry_uri}"))?;
        let low = handle.doc();
        if !matches!(
            low.sniff_family(),
            SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
        ) {
            return Err("not an OpenAPI 3.x document".to_owned());
        }
        let mut spec = IrSpec::default();
        let root = low.root();

        if let Some(info) = root.get("info") {
            spec.title = info
                .get("title")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_owned();
            spec.version = info
                .get("version")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_owned();
        }
        if let Some(servers) = root.get("servers") {
            spec.servers = servers
                .items()
                .iter()
                .filter_map(|s| s.get("url").and_then(|n| n.as_str()))
                .map(str::to_owned)
                .collect();
        }

        // Component schemas first: operations reference them by name.
        if let Some(components) = root.get("components")
            && let Some(schemas_node) = components.get("schemas")
        {
            for e in schemas_node.entries() {
                let json = materialize(e.value);
                let idx = spec.schemas.len() as u32;
                spec.schemas.push(IrSchema {
                    name: e.key.to_owned(),
                    json,
                });
                spec.schema_index.insert(e.key.to_owned(), idx);
            }
            for s in &spec.schemas {
                let refs = collect_local_refs(&s.json);
                spec.schema_edges.insert(s.name.clone(), refs);
            }
        }

        // Operations.
        if let Some(paths) = root.get("paths") {
            for e in paths.entries() {
                let path = e.key.to_owned();
                let Some(item) = e.value else { continue };
                let item_params = parameters_of(item);
                for method in Method::ALL {
                    let Some(op) = item.get(method_key(method)) else {
                        continue;
                    };
                    let mut params = item_params.clone();
                    params.extend(parameters_of(op));
                    let id = op.get("operationId").and_then(|n| n.as_str());
                    let idx = spec.operations.len() as u32;
                    spec.operations.push(IrOperation {
                        id: id.map(str::to_owned),
                        method,
                        path: path.clone(),
                        summary: op
                            .get("summary")
                            .and_then(|n| n.as_str())
                            .map(str::to_owned),
                        description: op
                            .get("description")
                            .and_then(|n| n.as_str())
                            .map(str::to_owned),
                        tags: op
                            .get("tags")
                            .map(|t| {
                                t.items()
                                    .iter()
                                    .filter_map(|n| n.as_str())
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        deprecated: op
                            .get("deprecated")
                            .and_then(|n| n.as_bool())
                            .unwrap_or(false),
                        parameters: params,
                        body_schema: op.get("requestBody").and_then(|rb| {
                            rb.get("content")
                                .and_then(|c| c.get("application/json"))
                                .and_then(|j| j.get("schema"))
                                .and_then(|n| schema_ref_name(n))
                        }),
                        responses: responses_of(op),
                    });
                    if let Some(id) = id {
                        spec.by_operation_id.insert(id.to_owned(), idx);
                    }
                    spec.by_method_path.insert((method, path.clone()), idx);
                }
            }
        }
        Ok(spec)
    }

    /// Looks up an operation by selector.
    #[must_use]
    pub fn operation(&self, sel: OpSelector<'_>) -> Option<&IrOperation> {
        let idx = match sel {
            OpSelector::Id(id) => self.by_operation_id.get(id).copied()?,
            OpSelector::MethodPath(m, p) => self.by_method_path.get(&(m, p.to_owned())).copied()?,
        };
        self.operations.get(usize::try_from(idx).ok()?)
    }

    /// Looks up a schema by name.
    #[must_use]
    pub fn schema(&self, name: &str) -> Option<&IrSchema> {
        self.schema_index
            .get(name)
            .copied()
            .and_then(|i| self.schemas.get(usize::try_from(i).ok()?))
    }
}

fn method_key(method: Method) -> &'static str {
    match method {
        Method::Get => "get",
        Method::Put => "put",
        Method::Post => "post",
        Method::Delete => "delete",
        Method::Options => "options",
        Method::Head => "head",
        Method::Patch => "patch",
        Method::Trace => "trace",
    }
}

fn parameters_of(node: NodeRef<'_>) -> Vec<IrParameter> {
    node.get("parameters")
        .map(|p| {
            p.items()
                .iter()
                .filter_map(|param| {
                    // $ref parameters are not followed in v1; record the name only.
                    let name = param.get("name").and_then(|n| n.as_str())?;
                    let location = match param.get("in").and_then(|n| n.as_str())? {
                        "query" => ParamIn::Query,
                        "header" => ParamIn::Header,
                        "path" => ParamIn::Path,
                        "cookie" => ParamIn::Cookie,
                        _ => return None,
                    };
                    Some(IrParameter {
                        name: name.to_owned(),
                        location,
                        required: param
                            .get("required")
                            .and_then(|n| n.as_bool())
                            .unwrap_or(false),
                        schema: param.get("schema").map(Some).map(materialize),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn responses_of(op: NodeRef<'_>) -> Vec<IrResponse> {
    let Some(responses) = op.get("responses") else {
        return Vec::new();
    };
    let mut entries: Vec<(Option<u16>, IrResponse)> = responses
        .entries()
        .iter()
        .filter_map(|e| {
            let value = e.value?;
            let status = e.key.parse::<u16>().ok();
            let r = IrResponse {
                status,
                description: value
                    .get("description")
                    .and_then(|n| n.as_str())
                    .map(str::to_owned),
                schema: value
                    .get("content")
                    .and_then(|c| c.get("application/json"))
                    .and_then(|j| j.get("schema"))
                    .and_then(|n| schema_ref_name(n)),
            };
            Some((status, r))
        })
        .collect();
    // Numeric statuses ascending, then `default` last.
    entries.sort_by_key(|(status, _)| status.unwrap_or(u16::MAX));
    entries.into_iter().map(|(_, r)| r).collect()
}

/// Resolves a schema node's `$ref` to a local component name.
fn schema_ref_name(node: NodeRef<'_>) -> Option<String> {
    let raw = node.get("$ref").and_then(|n| n.as_str())?;
    raw.strip_prefix("#/components/schemas/")
        .map(percent_decode)
        .filter(|n| !n.is_empty())
}

fn percent_decode(text: &str) -> String {
    // ~1/~0 JSON-pointer escapes plus %XX; small enough to hand-roll.
    let unescaped = text.replace("~1", "/").replace("~0", "~");
    let bytes = unescaped.as_bytes();
    let mut out = String::with_capacity(unescaped.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &unescaped[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Materializes any node into owned JSON via the overlay value tree.
fn materialize(node: Option<NodeRef<'_>>) -> serde_json::Value {
    let Some(node) = node else {
        return serde_json::Value::Null;
    };
    let json_string = suspect_overlay::Value::from_node(node.resolved()).to_json();
    serde_json::from_str(&json_string).unwrap_or(serde_json::Value::Null)
}

/// Collects local `#/components/schemas/{name}` references from JSON.
fn collect_local_refs(json: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_refs(json, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk_refs(json: &serde_json::Value, out: &mut Vec<String>) {
    match json {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref"
                    && let Some(name) = v
                        .as_str()
                        .and_then(|r| r.strip_prefix("#/components/schemas/"))
                {
                    out.push(name.to_owned());
                } else {
                    walk_refs(v, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_refs(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
