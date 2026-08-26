//! Native `given` selection.
//!
//! The SDK's selector DSL emits a small set of fixed JSONPath patterns
//! (`$.paths[*][*]`, `$.components.schemas[*]`, …). Those are resolved by
//! direct CST navigation — microseconds, not the generic JSONPath engine's
//! full-tree walk (which the lint engine also avoids via its fast path).
//! Unknown selectors fall back to [`suspect_jsonpath`].

use serde_json::Value;

use crate::node_json::node_to_json;
use crate::{Error, Result};
use suspect_jsonpath::Path;
use suspect_low::{LowDoc, NodeRef};

/// One host-selected node ready for the wire.
#[derive(Debug, Clone)]
pub struct Selected {
    /// RFC 6901 pointer from the document root.
    pub pointer: String,
    /// Plain JSON value at the pointer.
    pub value: serde_json::Value,
    /// Byte range in the source, when the node is a CST node.
    pub span: Option<(usize, usize)>,
}

/// Structural selector kinds — exact matches for the SDK's DSL constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structural {
    /// `$.paths[*][*]` — path-item children (operations + non-method keys;
    /// the worker filters to operations).
    PathItemChildren,
    /// `$.paths[*]` — path items.
    PathItems,
    /// `$.paths[*][*].parameters[*]` — operation parameters.
    OperationParameters,
    /// `$.paths[*][*].responses[*]` — operation responses.
    OperationResponses,
    /// `$.components.schemas[*]` — component schemas.
    ComponentSchemas,
    /// `$.operations[*]` — fact-space routes.
    FactOperations,
    /// `$.responses[*]` — fact-space response signals.
    FactResponses,
}

impl Structural {
    fn parse(given: &str) -> Option<Self> {
        match given {
            "$.paths[*][*]" => Some(Self::PathItemChildren),
            "$.paths[*]" => Some(Self::PathItems),
            "$.paths[*][*].parameters[*]" => Some(Self::OperationParameters),
            "$.paths[*][*].responses[*]" => Some(Self::OperationResponses),
            "$.components.schemas[*]" => Some(Self::ComponentSchemas),
            "$.operations[*]" => Some(Self::FactOperations),
            "$.responses[*]" => Some(Self::FactResponses),
            _ => None,
        }
    }

    /// The DSL selector text this kind mirrors (for metadata round-trips).
    #[must_use]
    pub fn as_jsonpath(self) -> &'static str {
        match self {
            Self::PathItemChildren => "$.paths[*][*]",
            Self::PathItems => "$.paths[*]",
            Self::OperationParameters => "$.paths[*][*].parameters[*]",
            Self::OperationResponses => "$.paths[*][*].responses[*]",
            Self::ComponentSchemas => "$.components.schemas[*]",
            Self::FactOperations => "$.operations[*]",
            Self::FactResponses => "$.responses[*]",
        }
    }
}

/// A compiled selector: parse once per rule, evaluate per document.
pub struct CompiledSelector {
    inner: Inner,
    source: String,
}

enum Inner {
    Structural(Structural),
    JsonPath(Path),
}

impl CompiledSelector {
    /// Parses `given` from a rule's ready metadata. Structural DSL
    /// patterns take the fast path; anything else compiles as JSONPath.
    ///
    /// # Errors
    /// [`Error::BadSelector`] when a fallback JSONPath does not compile.
    pub fn parse(rule_id: &str, given: &str) -> Result<Self> {
        let inner = if let Some(kind) = Structural::parse(given) {
            Inner::Structural(kind)
        } else {
            let path = Path::parse(given).map_err(|e| Error::BadSelector {
                rule: rule_id.to_owned(),
                message: e.to_string(),
            })?;
            Inner::JsonPath(path)
        };
        Ok(Self {
            inner,
            source: given.to_owned(),
        })
    }

    /// The original selector text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Evaluates against the document, producing wire-ready nodes.
    /// Structural paths build pointers top-down during the descent
    /// (`path_from_root` per node is a tree climb and dominates at scale).
    #[must_use]
    pub fn select(&self, doc: &LowDoc) -> Vec<Selected> {
        let root = doc.root();
        match &self.inner {
            Inner::Structural(kind) => select_structural(*kind, root),
            Inner::JsonPath(path) => path
                .query(root)
                .iter()
                .map(|node| Selected {
                    pointer: node.path_from_root().to_string(),
                    value: node_to_json(&node),
                    span: Some((node.byte_range().start, node.byte_range().end)),
                })
                .collect(),
        }
    }

    /// Pointer-only selection: the worker resolves values against the
    /// shipped document, so structural selection skips per-node JSON
    /// conversion entirely (591 ops on stripe: 647ms → ~2ms).
    #[must_use]
    pub fn select_pointers(&self, doc: &LowDoc) -> Vec<String> {
        let root = doc.root();
        match &self.inner {
            Inner::Structural(kind) => select_structural_inner(*kind, root, false)
                .into_iter()
                .map(|s| s.pointer)
                .collect(),
            Inner::JsonPath(path) => path
                .query(root)
                .iter()
                .map(|n| n.path_from_root().to_string())
                .collect(),
        }
    }
}

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

fn select_structural<'d>(kind: Structural, root: NodeRef<'d>) -> Vec<Selected> {
    select_structural_inner(kind, root, true)
}

fn select_structural_inner<'d>(
    kind: Structural,
    root: NodeRef<'d>,
    with_values: bool,
) -> Vec<Selected> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Selected>, pointer: String, node: &NodeRef<'d>| {
        let value = if with_values {
            node_to_json(node)
        } else {
            Value::Null
        };
        out.push(Selected {
            pointer,
            value,
            span: Some((node.byte_range().start, node.byte_range().end)),
        });
    };
    match kind {
        Structural::PathItems | Structural::PathItemChildren => {
            let Some(paths) = root.get("paths") else {
                return out;
            };
            for item in paths.entries() {
                let Some(item_node) = &item.value else {
                    continue;
                };
                let item_pointer = format!("/paths/{}", escape_token(item.key));
                if kind == Structural::PathItems {
                    push(&mut out, item_pointer, item_node);
                    continue;
                }
                for child in item_node.entries() {
                    // All path-item children (worker filters to operations).
                    if let Some(child_node) = &child.value {
                        let pointer = format!("{item_pointer}/{}", escape_token(child.key));
                        push(&mut out, pointer, child_node);
                    }
                }
            }
        }
        Structural::OperationParameters | Structural::OperationResponses => {
            let field = if kind == Structural::OperationParameters {
                "parameters"
            } else {
                "responses"
            };
            for (op_pointer, op) in operations_with_pointers(root) {
                let Some(section) = op.get(field) else {
                    continue;
                };
                if kind == Structural::OperationResponses {
                    for entry in section.entries() {
                        if let Some(node) = &entry.value {
                            let pointer =
                                format!("{op_pointer}/{field}/{}", escape_token(entry.key));
                            push(&mut out, pointer, node);
                        }
                    }
                } else {
                    for (i, node) in section.items().iter().enumerate() {
                        push(&mut out, format!("{op_pointer}/{field}/{i}"), node);
                    }
                }
            }
        }
        Structural::ComponentSchemas => {
            let Some(schemas) = root.get("components").and_then(|c| c.get("schemas")) else {
                return out;
            };
            for entry in schemas.entries() {
                if let Some(node) = &entry.value {
                    let pointer = format!("/components/schemas/{}", escape_token(entry.key));
                    push(&mut out, pointer, node);
                }
            }
        }
        Structural::FactOperations | Structural::FactResponses => {
            let key = if kind == Structural::FactOperations {
                "operations"
            } else {
                "responses"
            };
            let Some(section) = root.get(key) else {
                return out;
            };
            for (i, node) in section.items().iter().enumerate() {
                push(&mut out, format!("/{key}/{i}"), node);
            }
        }
    }
    out
}

/// RFC 6901 token escaping.
fn escape_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Yields `(pointer, node)` for operations: `paths[*]` children whose key
/// is an HTTP method.
fn operations_with_pointers<'d>(root: NodeRef<'d>) -> Vec<(String, NodeRef<'d>)> {
    let mut out = Vec::new();
    let Some(paths) = root.get("paths") else {
        return out;
    };
    for item in paths.entries() {
        let Some(item_node) = &item.value else {
            continue;
        };
        let item_pointer = format!("/paths/{}", escape_token(item.key));
        for child in item_node.entries() {
            if METHODS.contains(&child.key)
                && let Some(node) = &child.value
            {
                out.push((format!("{item_pointer}/{}", escape_token(child.key)), *node));
            }
        }
    }
    out
}

/// Resolves a pointer back to its byte span in the document.
#[must_use]
pub fn span_at_pointer(doc: &LowDoc, pointer: &str) -> Option<(usize, usize)> {
    let ptr = suspect_low::Pointer::parse(pointer).ok()?;
    let node = doc.root().pointer(&ptr)?;
    Some((node.byte_range().start, node.byte_range().end))
}
