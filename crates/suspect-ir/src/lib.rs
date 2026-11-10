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
//! resolved to the bare component name. A reference into another file is
//! loaded through the workspace and its target becomes a component of the
//! snapshot under a generated name, so a description split across files
//! reads like a bundled one. A reference that cannot be loaded (a missing
//! file, a remote URL with no provider) stays unresolved (`None`), and
//! [`IrMedia::unresolved`] keeps its text for whoever has to report it.
//!
//! [`contract`] is the lossless counterpart: a source-addressed view over
//! every loaded document, used where exact provenance and OpenAPI 3.1/3.2
//! HTTP semantics matter more than a flattened snapshot.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use suspect_low::SpecFamily;
use suspect_ref::Workspace;
use suspect_source::Uri;

pub mod common;
pub mod contract;
pub mod evolution;
pub mod faker;
pub mod fast;
mod internalize;

pub use fast::ir_from_fast;

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

/// One media type a response or request body declares.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct IrMedia {
    /// The media type key as written (`application/json`, `text/*`, `*/*`).
    pub media_type: String,
    /// The media type's schema as an addressable name: a local component
    /// keeps its name, an inline schema gets a generated one. `None` when
    /// no schema is declared or the reference is not local.
    pub schema: Option<String>,
    /// The schema's `$ref` text when it could not be resolved to a name
    /// (another document, or a pointer that is not a component schema).
    /// Conformance checks report it instead of treating the body as
    /// unconstrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<String>,
}

/// One header a response declares.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct IrHeader {
    /// Header name as written.
    pub name: String,
    /// Whether the response must carry it.
    pub required: bool,
    /// Materialized schema JSON, when declared.
    pub schema: Option<serde_json::Value>,
}

/// One response definition.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct IrResponse {
    /// Exact status code; `None` for `default` and for a status range
    /// (see [`IrResponse::range`]).
    pub status: Option<u16>,
    /// Response description.
    pub description: Option<String>,
    /// Schema of the JSON-compatible media type, as an addressable name:
    /// `application/json` when declared, else the first `+json` media type.
    pub schema: Option<String>,
    /// Hundreds digit of a status range key: `4` for `4XX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<u8>,
    /// Every declared media type, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<IrMedia>,
    /// Declared response headers, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<IrHeader>,
    /// The `$ref` text of the response, or of one of its headers, when it
    /// could not be followed (a missing component or file). The response
    /// then declares less than can be read, and conformance checks report
    /// it instead of treating the status as declared without constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<String>,
}

impl IrResponse {
    /// Whether this is the `default` response: neither an exact status nor
    /// a status range.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.status.is_none() && self.range.is_none()
    }

    /// The `responses` key this entry was declared under: `200`, `4XX` or
    /// `default`.
    #[must_use]
    pub fn key(&self) -> String {
        match (self.status, self.range) {
            (Some(status), _) => status.to_string(),
            (None, Some(range)) => format!("{range}XX"),
            (None, None) => "default".to_owned(),
        }
    }

    /// The media type whose schema [`IrResponse::schema`] names.
    #[must_use]
    pub fn json_media(&self) -> Option<&IrMedia> {
        json_media(&self.content)
    }
}

/// The JSON-compatible entry of a content map: `application/json` when
/// declared, else the first media type [`is_json_media_type`] accepts.
#[must_use]
pub fn json_media(content: &[IrMedia]) -> Option<&IrMedia> {
    content
        .iter()
        .find(|media| media_essence(&media.media_type) == "application/json")
        .or_else(|| {
            content
                .iter()
                .find(|media| is_json_media_type(&media.media_type))
        })
}

/// A media type without its parameters, lowercased: `Application/JSON;
/// charset=utf-8` is `application/json`.
#[must_use]
pub fn media_essence(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase()
}

/// Whether a media type carries JSON: a `json` subtype or a `+json`
/// structured-syntax suffix (RFC 6839). Parameters and case are ignored;
/// wildcards are not JSON.
#[must_use]
pub fn is_json_media_type(media_type: &str) -> bool {
    let essence = media_essence(media_type);
    let Some((_, subtype)) = essence.split_once('/') else {
        return false;
    };
    subtype == "json" || subtype.ends_with("+json")
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
    /// Declared responses: exact statuses ascending, then status ranges,
    /// then `default`.
    pub responses: Vec<IrResponse>,
    /// The `$ref` text of every parameter, request body and request-body
    /// schema that could not be followed (a missing component or file).
    /// The request side of the operation then declares less than its
    /// description means, and request checks report it instead of passing
    /// what they could not read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
}

impl IrOperation {
    /// The declared response that governs `status`, by the OpenAPI
    /// selection rule: the exact code, else its range (`4XX`), else
    /// `default`. `None` when the operation declares none of them — the
    /// status is undocumented.
    #[must_use]
    pub fn response_for(&self, status: u16) -> Option<&IrResponse> {
        response_for(&self.responses, status)
    }
}

/// [`IrOperation::response_for`] over a bare response list.
#[must_use]
pub fn response_for(responses: &[IrResponse], status: u16) -> Option<&IrResponse> {
    let class = u8::try_from(status / 100).ok();
    responses
        .iter()
        .find(|response| response.status == Some(status))
        .or_else(|| {
            responses.iter().find(|response| {
                response.status.is_none() && response.range.is_some() && response.range == class
            })
        })
        .or_else(|| responses.iter().find(|response| response.is_default()))
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
    /// The document's `openapi` version string (`3.0.3`, `3.1.0`); empty
    /// when the snapshot was not built from a document. It selects the
    /// schema dialect conformance checks apply.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub openapi: String,
    /// Server URLs.
    pub servers: Vec<String>,
    /// All operations, document order.
    pub operations: Vec<IrOperation>,
    /// Component schemas, document order.
    pub schemas: Vec<IrSchema>,
    /// `operationId` -> index into [`IrSpec::operations`] (derived; not
    /// serialized).
    #[serde(skip)]
    pub by_operation_id: HashMap<String, u32>,
    /// `(method, path)` -> index (tuple keys are not valid JSON map keys,
    /// so this is skipped when serializing).
    #[serde(skip)]
    pub by_method_path: HashMap<(Method, String), u32>,
    /// Schema name -> index into [`IrSpec::schemas`] (derived; not
    /// serialized).
    #[serde(skip)]
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
        if entry_uri.as_path().is_some_and(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        }) {
            // Generic syntax auto-detection permits flow-style YAML. A
            // declared JSON input must not use that fallback to repair an
            // invalid JSON escape or other JSON syntax.
            serde_json::from_slice::<&serde_json::value::RawValue>(low.inner().bytes())
                .map_err(|error| format!("invalid JSON in {entry_uri}: {error}"))?;
        }
        if let Some(error) = low.syntax_errors().first() {
            return Err(format!("syntax error in {entry_uri}: {error:?}"));
        }
        if !matches!(
            low.sniff_family(),
            SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
        ) {
            return Err("not an OpenAPI 3.x document".to_owned());
        }
        // Convert the resolved CST into the shared value tree and run the
        // same walk the fast path uses — one set of construction rules for
        // both pipelines.
        let mut root_value = fast::value_from_node(low.root())?;
        // A description split across files addresses its parts by URI;
        // the snapshot addresses them by component name.
        internalize::internalize(ws, entry_uri, &mut root_value);
        Ok(ir_from_fast(&root_value))
    }

    /// Builds the IR directly from one spec file.
    ///
    /// Tries the allocation-lean YAML-subset reader first; documents using
    /// features outside that subset, or referencing other documents, fall
    /// back to a workspace load of the entry file, so exotic YAML still
    /// produces identical output. Neither path implicitly loads unrelated
    /// neighboring documents: only the documents the entry references are
    /// read, and what they declare becomes components of the snapshot.
    /// Native SDK planning uses [`contract::Contract`]'s source-addressed
    /// semantic graph instead.
    ///
    /// # Errors
    /// `"not an OpenAPI 3.x document"` when the family sniff fails, or an
    /// I/O / workspace error message.
    pub fn from_file(path: &Path) -> Result<IrSpec, String> {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            return Self::from_file_via_workspace(path);
        }
        let bytes = std::fs::read(path).map_err(|e| format!("read {path:?}: {e}"))?;
        if let Some(root) = suspect_syntax::try_parse_fast(&bytes) {
            if !fast::is_oas3(&root) {
                return Err("not an OpenAPI 3.x document".to_owned());
            }
            // References into other documents need the workspace to load
            // them; a self-contained document never pays for one.
            if !internalize::has_external_refs(&root) {
                return Ok(ir_from_fast(&root));
            }
        }
        Self::from_file_via_workspace(path)
    }

    /// Fallback path: open the entry using its retrieval URI, then run the
    /// standard walk. Referencing a document is distinct from sharing a folder.
    fn from_file_via_workspace(path: &Path) -> Result<IrSpec, String> {
        use suspect_ref::WorkspaceBuilder;

        let ws = WorkspaceBuilder::new().build().map_err(|e| e.to_string())?;
        let uri = Uri::from_path(path).map_err(|e| e.to_string())?;
        ws.open(uri.as_str()).map_err(|e| e.to_string())?;
        Self::from_workspace(&Arc::new(ws), &uri)
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

#[cfg(test)]
mod tests;
