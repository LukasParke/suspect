//! Vendor-extension schemas: every `x-*` pair whose key has a registered
//! JSON Schema (builtin or configured under `extensions`) is validated
//! against it.
//!
//! Configuration shape (the `extensions` settings section):
//!
//! ```json
//! {
//!   "x-plex-attribute": { "contexts": ["root"], "schema": "schemas/x-plex-attribute.json" },
//!   "x-inline": { "schema": {"type": "boolean"} }
//! }
//! ```
//!
//! `schema` accepts an inline JSON Schema or a path relative to the
//! workspace root (JSON or YAML). Configured entries win over builtins for
//! the same key. Unknown extensions are legal per the OpenAPI spec and
//! produce nothing. The ordering half of the same section (`before` /
//! `after`) belongs to the editor's formatter, in `suspect-lsp`.

use std::collections::BTreeMap;
use std::path::Path;

use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::LowDoc;

/// The code every schema failure carries.
pub const CODE: &str = "extension-schema";

/// The context the check looks schemas up in. The pairs are not
/// classified by their position in the document, so only schemas that
/// apply in every context (or name this one) are enforced.
pub const CONTEXT: &str = "unknown";

/// A builtin extension schema.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinSchema {
    /// The extension key.
    pub key: &'static str,
    /// Context names the schema applies in; empty means every context.
    pub contexts: &'static [&'static str],
    /// JSON Schema text.
    pub schema: &'static str,
}

/// Builtin schemas for the widely used extension families.
pub const BUILTIN: &[BuiltinSchema] = &[
    BuiltinSchema {
        key: "x-codeSamples",
        contexts: &["operation"],
        schema: r#"{"type":"array","items":{"type":"object","required":["lang","source"],
               "properties":{"lang":{"type":"string"},"label":{"type":"string"},
               "source":{"type":"string"}}}}"#,
    },
    BuiltinSchema {
        key: "x-displayName",
        contexts: &["tag", "root", "components"],
        schema: r#"{"type":"string"}"#,
    },
    BuiltinSchema {
        key: "x-speakeasy-pagination",
        contexts: &["operation"],
        schema: r#"{"type":"object","required":["type","inputs","output"],
               "properties":{"type":{"enum":["cursor","offset","page","url"]}}}"#,
    },
    BuiltinSchema {
        key: "x-speakeasy-name-override",
        contexts: &["operation"],
        schema: r#"{"type":"string"}"#,
    },
    BuiltinSchema {
        key: "x-speakeasy-group",
        contexts: &["operation"],
        schema: r#"{"type":"string"}"#,
    },
    BuiltinSchema {
        key: "x-internal",
        contexts: &[],
        schema: r#"{"type":"boolean"}"#,
    },
    BuiltinSchema {
        key: "x-omitempty",
        contexts: &[],
        schema: r#"{"type":"boolean"}"#,
    },
];

/// One configured extension schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomSchema {
    /// Context names the entry applies in (empty: every context).
    pub contexts: Vec<String>,
    /// Inline JSON Schema text or a workspace-relative path.
    pub schema: Option<String>,
}

/// Configured schemas over [`BUILTIN`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionSchemas {
    /// Configured entries by extension key.
    pub custom: BTreeMap<String, CustomSchema>,
}

impl ExtensionSchemas {
    /// Parses the `extensions` settings object; anything that is not an
    /// object yields the builtins alone.
    #[must_use]
    pub fn from_value(value: Option<&serde_json::Value>) -> Self {
        let mut custom = BTreeMap::new();
        let Some(obj) = value.and_then(serde_json::Value::as_object) else {
            return Self { custom };
        };
        for (key, entry) in obj {
            if !key.starts_with("x-") {
                continue;
            }
            let contexts = entry
                .get("contexts")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|c| c.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let schema = match entry.get("schema") {
                Some(serde_json::Value::String(path)) => Some(path.clone()),
                Some(schema @ serde_json::Value::Object(_)) => Some(schema.to_string()),
                _ => None,
            };
            custom.insert(key.clone(), CustomSchema { contexts, schema });
        }
        Self { custom }
    }

    /// The JSON Schema text for `key` in `context`: a configured inline
    /// schema or workspace-relative file first, then the builtins.
    #[must_use]
    pub fn schema_text(&self, key: &str, context: &str, root: Option<&Path>) -> Option<String> {
        if let Some(custom) = self.custom.get(key) {
            let applies =
                custom.contexts.is_empty() || custom.contexts.iter().any(|c| c == context);
            if applies && let Some(schema) = &custom.schema {
                if schema.trim_start().starts_with('{') {
                    return Some(schema.clone());
                }
                return root.and_then(|root| std::fs::read_to_string(root.join(schema)).ok());
            }
        }
        BUILTIN
            .iter()
            .find(|b| b.key == key && (b.contexts.is_empty() || b.contexts.contains(&context)))
            .map(|b| b.schema.to_owned())
    }
}

/// Validates every registered `x-*` value in `low`. The schema and the
/// serialized instance are materialized into one wrapper document, since
/// the compiler's evaluation shares a single lifetime across both;
/// extension values are rare enough that per-pair compilation is cheap. A
/// registered schema that does not parse or compile is skipped.
#[must_use]
pub fn check(low: &LowDoc, schemas: &ExtensionSchemas, root: Option<&Path>) -> Vec<Finding> {
    let mut out = Vec::new();
    for n in low.inner().root().descendants() {
        if n.kind() != suspect_syntax::SyntaxKind::Pair {
            continue;
        }
        let Some(key) = n.child_by_field("key") else {
            continue;
        };
        let key_text = String::from_utf8_lossy(key.content().scalar_bytes()).into_owned();
        if !key_text.starts_with("x-") {
            continue;
        }
        let Some(value) = n.child_by_field("value") else {
            continue;
        };
        let Some(schema_text) = schemas.schema_text(&key_text, CONTEXT, root) else {
            continue;
        };
        let instance_json =
            suspect_overlay::Value::from_node(suspect_low::NodeRef::new(value.content())).to_json();
        let wrapper_text = format!("{{\"schema\": {schema_text}, \"instance\": {instance_json}}}");
        let Ok(wrapper_uri) = suspect_source::Uri::parse("mem://extension-check.json") else {
            continue;
        };
        let wrapper = LowDoc::parse(
            wrapper_uri,
            suspect_source::Source::from_vec(wrapper_text.into_bytes()),
        );
        if !wrapper.syntax_errors().is_empty() {
            continue;
        }
        let (Some(schema_node), Some(instance_node)) =
            (wrapper.root().get("schema"), wrapper.root().get("instance"))
        else {
            continue;
        };
        let Ok(schema) =
            suspect_schema::Compiler::new(suspect_schema::Config::default()).compile(schema_node)
        else {
            continue;
        };
        for error in schema.validate(instance_node) {
            out.push(
                Finding::new(
                    CheckId::Extension,
                    CODE,
                    Severity::Error,
                    low.uri().clone(),
                    format!("`{key_text}` fails its extension schema: {}", error.message),
                )
                .with_range(value.content().byte_range()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_schemas_are_valid_json() {
        for entry in BUILTIN {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(entry.schema);
            assert!(parsed.is_ok(), "{}: invalid schema JSON", entry.key);
        }
    }

    #[test]
    fn configured_entries_win_and_contexts_filter() {
        let schemas = ExtensionSchemas::from_value(Some(&serde_json::json!({
            "x-internal": {"schema": {"type": "string"}},
            "x-root-only": {"contexts": ["root"], "schema": {"type": "integer"}},
            "not-an-extension": {"schema": {}},
        })));
        assert_eq!(schemas.custom.len(), 2);
        assert_eq!(
            schemas.schema_text("x-internal", CONTEXT, None).as_deref(),
            Some(r#"{"type":"string"}"#)
        );
        assert!(schemas.schema_text("x-root-only", CONTEXT, None).is_none());
        assert!(schemas.schema_text("x-root-only", "root", None).is_some());
        // Builtins scoped to a context are not enforced by the check.
        assert!(
            schemas
                .schema_text("x-codeSamples", CONTEXT, None)
                .is_none()
        );
    }
}
