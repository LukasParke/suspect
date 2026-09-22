//! Workspace-configured vendor extensions: ordering anchors and JSON
//! Schemas for value validation, layered over the built-in registry.
//!
//! Configuration shape (under `suspect.extensions` in the workspace
//! config or LSP settings):
//!
//! ```json
//! {
//!   "x-plex-attribute": {
//!     "after": "security",
//!     "contexts": ["root"],
//!     "schema": "schemas/x-plex-attribute.json"
//!   },
//!   "x-inline": { "before": "responses", "schema": {"type": "boolean"} }
//! }
//! ```
//!
//! `schema` accepts an inline JSON Schema or a workspace-relative path to
//! one (JSON or YAML). Custom entries win over built-ins for the same key.

use crate::keys::Context;
use std::collections::BTreeMap;

/// One workspace-configured extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomExtension {
    /// Position directly before this standard key.
    pub before: Option<String>,
    /// Position directly after this standard key.
    pub after: Option<String>,
    /// Restricts the extension to these contexts (empty: every context).
    /// Context names are the snake_case variants (`root`, `operation`,
    /// `schema`, …).
    pub contexts: Vec<String>,
    /// Inline JSON Schema text or a workspace-relative path.
    pub schema: Option<String>,
}

/// The merged extension configuration: builtin + custom entries.
#[derive(Debug, Clone, Default)]
pub struct ExtensionConfig {
    /// Custom entries by extension key.
    pub custom: BTreeMap<String, CustomExtension>,
}

impl PartialEq for ExtensionConfig {
    fn eq(&self, other: &Self) -> bool {
        self.custom == other.custom
    }
}

impl ExtensionConfig {
    /// Parses the `suspect.extensions` settings object.
    #[must_use]
    pub fn from_value(value: &serde_json::Value) -> Self {
        let mut custom = BTreeMap::new();
        let Some(obj) = value.as_object() else {
            return Self { custom };
        };
        for (key, entry) in obj {
            if !key.starts_with("x-") {
                continue;
            }
            let mut ext = CustomExtension::default();
            if let Some(before) = entry.get("before").and_then(|v| v.as_str()) {
                ext.before = Some(before.to_owned());
            }
            if let Some(after) = entry.get("after").and_then(|v| v.as_str()) {
                ext.after = Some(after.to_owned());
            }
            if let Some(contexts) = entry.get("contexts").and_then(|v| v.as_array()) {
                ext.contexts = contexts
                    .iter()
                    .filter_map(|c| c.as_str().map(str::to_owned))
                    .collect();
            }
            if let Some(schema) = entry.get("schema") {
                ext.schema = match schema {
                    serde_json::Value::String(path) => Some(path.clone()),
                    serde_json::Value::Object(_) => Some(schema.to_string()),
                    _ => None,
                };
            }
            custom.insert(key.clone(), ext);
        }
        Self { custom }
    }

    /// Ordering rank for `key` in `context`: custom entries win, then
    /// builtins, then unknown (the caller's alphabetical fallback).
    #[must_use]
    pub fn rank(&self, key: &str, context: Context, table: &[&str]) -> Option<f64> {
        if let Some(custom) = self.custom.get(key)
            && (custom.contexts.is_empty()
                || custom
                    .contexts
                    .iter()
                    .any(|c| context_name(context) == c.as_str()))
        {
            if let Some(before) = &custom.before
                && let Some(idx) = table.iter().position(|k| k == before)
            {
                return Some(idx as f64 - 0.5);
            }
            if let Some(after) = &custom.after
                && let Some(idx) = table.iter().position(|k| k == after)
            {
                return Some(idx as f64 + 0.5);
            }
        }
        super::extensions_registry::rank(key, context, table)
    }

    /// Resolves the JSON Schema text for `key`: custom inline or
    /// workspace-relative path first, then the builtin registry.
    /// `workspace_root` anchors relative paths.
    #[must_use]
    pub fn schema_text(
        &self,
        key: &str,
        context: Context,
        workspace_root: Option<&std::path::Path>,
    ) -> Option<String> {
        if let Some(custom) = self.custom.get(key) {
            let context_ok = custom.contexts.is_empty()
                || custom
                    .contexts
                    .iter()
                    .any(|c| context_name(context) == c.as_str());
            if context_ok && let Some(schema) = &custom.schema {
                // Inline JSON (object) vs path (string).
                if schema.trim_start().starts_with('{') {
                    return Some(schema.clone());
                }
                if let Some(root) = workspace_root {
                    let path = root.join(schema);
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        return Some(text);
                    }
                }
                return None;
            }
        }
        super::extensions_registry::schema_for(key, context).map(str::to_owned)
    }
}

/// The snake_case name matching configuration `contexts` entries.
#[must_use]
pub fn context_name(context: Context) -> &'static str {
    match context {
        Context::Root => "root",
        Context::Info => "info",
        Context::Contact => "contact",
        Context::License => "license",
        Context::Components => "components",
        Context::Operation => "operation",
        Context::Parameter => "parameter",
        Context::Schema => "schema",
        Context::Response => "response",
        Context::SecurityScheme => "security_scheme",
        Context::OAuthFlow => "oauth_flow",
        Context::Server => "server",
        Context::ServerVariable => "server_variable",
        Context::Tag => "tag",
        Context::ExternalDocs => "external_docs",
        Context::PathItem => "path_item",
        Context::RequestBody => "request_body",
        Context::MediaType => "media_type",
        Context::Encoding => "encoding",
        Context::Header => "header",
        Context::Link => "link",
        Context::Example => "example",
        Context::Discriminator => "discriminator",
        Context::Xml => "xml",
        Context::Callback => "callback",
        Context::Unknown => "unknown",
    }
}
