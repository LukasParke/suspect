//! Vendor extension ordering and schema registry.
//!
//! Extensions (`x-*` keys) never mix into standard keys: known extensions
//! are positioned relative to the context's standard keys via
//! `before`/`after` anchors; unknown extensions sort after everything,
//! alphabetically. The built-in registry covers the widely used extension
//! families; workspaces extend or override it through configuration
//! (see [`config_files`] and the `suspect.extensions` settings section).

/// Position of one extension relative to a standard key of its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Directly before the named standard key.
    Before(&'static str),
    /// Directly after the named standard key.
    After(&'static str),
}

/// One registered extension: where it sits and (optionally) the JSON Schema
/// its value must satisfy.
#[derive(Debug, Clone)]
pub struct ExtensionEntry {
    /// The extension key (`x-tagGroups`).
    pub key: &'static str,
    /// Contexts the positioning applies to; empty means every context.
    pub contexts: &'static [crate::keys::Context],
    /// Where the extension sits in its context.
    pub anchor: Anchor,
    /// Optional JSON Schema (as raw JSON text) validating the value.
    pub schema: Option<&'static str>,
}

/// The built-in extension registry: the extension families an OpenAPI
/// author is likely to meet, in their conventional positions.
pub const BUILTIN: &[ExtensionEntry] = &[
    // -- documentation/catalog families -----------------------------------
    ExtensionEntry {
        key: "x-tagGroups",
        contexts: &[crate::keys::Context::Root],
        anchor: Anchor::After("tags"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-badges",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::After("tags"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-codeSamples",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::After("servers"),
        schema: Some(
            r#"{"type":"array","items":{"type":"object","required":["lang","source"],
               "properties":{"lang":{"type":"string"},"label":{"type":"string"},
               "source":{"type":"string"}}}}"#,
        ),
    },
    ExtensionEntry {
        key: "x-displayName",
        contexts: &[
            crate::keys::Context::Tag,
            crate::keys::Context::Root,
            crate::keys::Context::Components,
        ],
        anchor: Anchor::Before("description"),
        schema: Some(r#"{"type":"string"}"#),
    },
    ExtensionEntry {
        key: "x-order",
        contexts: &[],
        anchor: Anchor::Before("description"),
        schema: None,
    },
    // -- Speakeasy generator family ---------------------------------------
    ExtensionEntry {
        key: "x-speakeasy-globals",
        contexts: &[crate::keys::Context::Root],
        anchor: Anchor::After("security"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-speakeasy-globals-hidden",
        contexts: &[crate::keys::Context::Root],
        anchor: Anchor::After("security"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-speakeasy-pagination",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::After("parameters"),
        schema: Some(
            r#"{"type":"object","required":["type","inputs","output"],
               "properties":{"type":{"enum":["cursor","offset","page","url"]}}}"#,
        ),
    },
    ExtensionEntry {
        key: "x-speakeasy-usage-example",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::After("deprecated"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-speakeasy-retries",
        contexts: &[
            crate::keys::Context::Operation,
            crate::keys::Context::Schema,
        ],
        anchor: Anchor::Before("parameters"),
        schema: None,
    },
    ExtensionEntry {
        key: "x-speakeasy-name-override",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::Before("operationId"),
        schema: Some(r#"{"type":"string"}"#),
    },
    ExtensionEntry {
        key: "x-speakeasy-group",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::Before("summary"),
        schema: Some(r#"{"type":"string"}"#),
    },
    // -- SailPoint family ---------------------------------------------------
    ExtensionEntry {
        key: "x-sailpoint-userLevels",
        contexts: &[crate::keys::Context::Operation],
        anchor: Anchor::After("security"),
        schema: None,
    },
    // -- OpenAPI utilities ---------------------------------------------------
    ExtensionEntry {
        key: "x-internal",
        contexts: &[],
        anchor: Anchor::After("description"),
        schema: Some(r#"{"type":"boolean"}"#),
    },
    ExtensionEntry {
        key: "x-omitempty",
        contexts: &[],
        anchor: Anchor::After("description"),
        schema: Some(r#"{"type":"boolean"}"#),
    },
];

/// Resolves the ordering rank of an extension key in `context`: lower ranks
/// sort earlier. Extensions anchored before a standard key rank just under
/// that key's index; extensions anchored after rank just above it.
/// Returns `None` when the key is not registered for the context (the
/// caller then treats it as unknown: alphabetical, after all known keys).
#[must_use]
pub fn rank(key: &str, context: crate::keys::Context, table: &[&str]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for entry in BUILTIN.iter().filter(|e| e.key == key) {
        if !entry.contexts.is_empty() && !entry.contexts.contains(&context) {
            continue;
        }
        let rank = match entry.anchor {
            Anchor::Before(anchor) => {
                let idx = table.iter().position(|k| *k == anchor)? as f64;
                idx - 0.5
            }
            Anchor::After(anchor) => {
                let idx = table.iter().position(|k| *k == anchor)? as f64;
                idx + 0.5
            }
        };
        best = Some(match best {
            Some(current) => current.min(rank),
            None => rank,
        });
    }
    best
}

/// Every built-in extension key visible for `context` (used by completion
/// and validation wiring).
#[must_use]
pub fn keys_for(context: crate::keys::Context) -> Vec<&'static str> {
    BUILTIN
        .iter()
        .filter(|e| e.contexts.is_empty() || e.contexts.contains(&context))
        .map(|e| e.key)
        .collect()
}

/// The JSON Schema text registered for `key` in `context`, if any.
#[must_use]
pub fn schema_for(key: &str, context: crate::keys::Context) -> Option<&'static str> {
    BUILTIN
        .iter()
        .find(|e| e.key == key && (e.contexts.is_empty() || e.contexts.contains(&context)))
        .and_then(|e| e.schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{Context, OPERATION, ROOT};

    #[test]
    fn builtin_keys_are_unique_per_context() {
        for context in [Context::Root, Context::Operation, Context::Schema] {
            let keys = keys_for(context);
            let mut seen = std::collections::BTreeSet::new();
            for key in keys {
                assert!(seen.insert(key), "duplicate {key}");
            }
        }
    }

    #[test]
    fn ranks_honor_anchors() {
        // x-tagGroups after `tags` in the root table.
        let tags_idx = ROOT.iter().position(|k| *k == "tags").unwrap() as f64;
        let tags_rank = rank("x-tagGroups", Context::Root, ROOT).unwrap();
        assert!((tags_rank - (tags_idx + 0.5)).abs() < f64::EPSILON);
        // x-speakeasy-name-override before `operationId`.
        let op_idx = OPERATION.iter().position(|k| *k == "operationId").unwrap() as f64;
        let override_rank =
            rank("x-speakeasy-name-override", Context::Operation, OPERATION).unwrap();
        assert!((override_rank - (op_idx - 0.5)).abs() < f64::EPSILON);
        // Context filtering: x-tagGroups is root-only.
        assert!(rank("x-tagGroups", Context::Operation, OPERATION).is_none());
    }

    #[test]
    fn schemas_that_exist_are_valid_json() {
        for entry in BUILTIN {
            if let Some(schema) = entry.schema {
                let parsed: Result<serde_json::Value, _> = serde_json::from_str(schema);
                assert!(parsed.is_ok(), "{}: invalid schema JSON", entry.key);
            }
        }
    }
}
