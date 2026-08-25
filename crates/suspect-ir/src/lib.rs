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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_low::SpecFamily;
use suspect_ref::Workspace;
use suspect_source::Uri;

pub mod common;
pub mod evolution;
pub mod fast;

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
        if !matches!(
            low.sniff_family(),
            SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
        ) {
            return Err("not an OpenAPI 3.x document".to_owned());
        }
        // Convert the resolved CST into the shared value tree and run the
        // same walk the fast path uses — one set of construction rules for
        // both pipelines.
        let root_value = fast::value_from_node(low.root());
        Ok(ir_from_fast(&root_value))
    }

    /// Builds the IR directly from one spec file.
    ///
    /// Tries the allocation-lean YAML-subset reader first; documents using
    /// features outside that subset fall back to a full workspace load of
    /// the file's directory (the same layout `suspect-cli` uses), so exotic
    /// YAML still produces identical output.
    ///
    /// # Errors
    /// `"not an OpenAPI 3.x document"` when the family sniff fails, or an
    /// I/O / workspace error message.
    pub fn from_file(path: &Path) -> Result<IrSpec, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path:?}: {e}"))?;
        if let Some(root) = suspect_syntax::try_parse_fast(&bytes) {
            return if fast::is_oas3(&root) {
                Ok(ir_from_fast(&root))
            } else {
                Err("not an OpenAPI 3.x document".to_owned())
            };
        }
        Self::from_file_via_workspace(path)
    }

    /// Fallback path: workspace-load the file's directory like
    /// `commands::workspace_dir_all`, then run the standard walk.
    fn from_file_via_workspace(path: &Path) -> Result<IrSpec, String> {
        use suspect_ref::WorkspaceBuilder;

        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let ws = WorkspaceBuilder::new()
            .root(&dir)
            .build()
            .map_err(|e| e.to_string())?;
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("yaml") | Some("yml") | Some("json")
                )
            })
            .collect();
        entries.sort();
        for entry in entries {
            if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                let _ = ws.load_all(name);
            }
        }
        let ws = Arc::new(ws);
        let uri = Uri::from_path(path).map_err(|e| e.to_string())?;
        Self::from_workspace(&ws, &uri)
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
