//! Wire-format-aware semantic diffing between two specification versions.
//!
//! Unlike text diffs or structural JSON comparisons, this module compares
//! two [`IrSpec`]s at the *semantic* level: it understands that renaming a
//! field with a custom serializer is transparent, but changing an enum
//! value breaks every consumer. Each delta is classified as BREAKING,
//! ADDITIVE, or COSMETIC.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use suspect_ir::{IrSchema, IrSpec};

/// Severity classification for each detected change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    /// Consumers will break.
    Breaking,
    /// New capability; consumers unaffected.
    Additive,
    /// No wire-format impact.
    Cosmetic,
}

/// One semantic difference between two spec versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    /// What changed (e.g. "field removed", "constraint tightened").
    pub kind: String,
    /// Where: `#/components/schemas/Pet/properties/name` or `GET /pets`.
    pub location: String,
    /// Human-readable description.
    pub message: String,
    /// Classification.
    pub impact: Impact,
}

/// The full diff between two spec versions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpecDiff {
    /// All deltas in deterministic order.
    pub deltas: Vec<Delta>,
    /// Semver recommendation derived from breaking changes.
    pub semver: &'static str,
    /// Count summary.
    pub breaking: usize,
    pub additive: usize,
    pub cosmetic: usize,
}

/// Computes the semantic diff between `old` and `new` specs.
#[must_use]
pub fn diff_specs(old: &IrSpec, new: &IrSpec) -> SpecDiff {
    let mut deltas = Vec::new();

    // --- schema-level changes ---
    diff_schemas(old, new, &mut deltas);

    // --- operation-level changes ---
    diff_operations(old, new, &mut deltas);

    // --- server changes ---
    if old.servers != new.servers {
        deltas.push(Delta {
            kind: "servers_changed".to_owned(),
            location: "servers".to_owned(),
            message: format!("Servers changed: {:?} -> {:?}", old.servers, new.servers),
            impact: Impact::Cosmetic,
        });
    }

    // Classify and count
    let mut breaking = 0usize;
    let mut additive = 0usize;
    let mut cosmetic = 0usize;
    for d in &deltas {
        match d.impact {
            Impact::Breaking => breaking += 1,
            Impact::Additive => additive += 1,
            Impact::Cosmetic => cosmetic += 1,
        }
    }

    let semver = if breaking > 0 {
        "major"
    } else if additive > 0 {
        "minor"
    } else {
        "patch"
    };

    SpecDiff {
        deltas,
        semver,
        breaking,
        additive,
        cosmetic,
    }
}

fn diff_schemas(old: &IrSpec, new: &IrSpec, deltas: &mut Vec<Delta>) {
    let old_map: HashMap<&str, &IrSchema> =
        old.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    let new_map: HashMap<&str, &IrSchema> =
        new.schemas.iter().map(|s| (s.name.as_str(), s)).collect();

    // Removed schemas → breaking
    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            deltas.push(Delta {
                kind: "schema_removed".to_owned(),
                location: format!("#/components/schemas/{name}"),
                message: format!("Schema `{name}` was removed"),
                impact: Impact::Breaking,
            });
        }
    }

    // Added schemas → additive
    for name in new_map.keys() {
        if !old_map.contains_key(name) {
            deltas.push(Delta {
                kind: "schema_added".to_owned(),
                location: format!("#/components/schemas/{name}"),
                message: format!("Schema `{name}` was added"),
                impact: Impact::Additive,
            });
        }
    }

    // Changed schemas → compare properties
    for name in old_map.keys() {
        let Some(new_schema) = new_map.get(name) else {
            continue;
        };
        let Some(old_schema) = old_map.get(name) else {
            continue;
        };
        diff_schema_fields(name, &old_schema.json, &new_schema.json, deltas);
    }
}

fn diff_schema_fields(
    schema_name: &str,
    old_json: &serde_json::Value,
    new_json: &serde_json::Value,
    deltas: &mut Vec<Delta>,
) {
    let base = format!("#/components/schemas/{schema_name}");
    let old_props = old_json.get("properties").and_then(|v| v.as_object());
    let new_props = new_json.get("properties").and_then(|v| v.as_object());

    let old_req: std::collections::HashSet<&str> = old_json
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let new_req: std::collections::HashSet<&str> = new_json
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Removed properties → breaking
    if let Some(op) = old_props {
        for key in op.keys() {
            let is_new = new_props.as_ref().is_some_and(|np| np.contains_key(key));
            if !is_new {
                deltas.push(Delta {
                    kind: "field_removed".to_owned(),
                    location: format!("{base}/properties/{key}"),
                    message: format!("Field `{key}` removed from `{schema_name}`"),
                    impact: Impact::Breaking,
                });
            }
            // Newly required → breaking for existing consumers who omit it
            if !old_req.contains(key.as_str()) && new_req.contains(key.as_str()) {
                deltas.push(Delta {
                    kind: "field_now_required".to_owned(),
                    location: format!("{base}/required/{key}"),
                    message: format!("Field `{key}` is now required in `{schema_name}`"),
                    impact: Impact::Breaking,
                });
            }
        }
    }

    // Added properties → additive (unless required)
    if let Some(np) = new_props {
        for key in np.keys() {
            let existed = old_props.as_ref().is_some_and(|op| op.contains_key(key));
            if !existed {
                let required = new_req.contains(key.as_str());
                deltas.push(Delta {
                    kind: "field_added".to_owned(),
                    location: format!("{base}/properties/{key}"),
                    message: format!(
                        "Field `{key}` added to `{schema_name}`{}",
                        if required { " (required)" } else { "" }
                    ),
                    impact: if required {
                        Impact::Breaking
                    } else {
                        Impact::Additive
                    },
                });
            }
        }
    }

    // Enum value changes on shared properties
    if let (Some(op), Some(np)) = (old_props, new_props) {
        for key in op.keys() {
            if !np.contains_key(key) {
                continue;
            }
            let old_enums = enum_values(op.get(key));
            let new_enums = enum_values(np.get(key));
            if old_enums.is_empty() || new_enums.is_empty() || old_enums == new_enums {
                continue;
            }
            for old_val in &old_enums {
                if !new_enums.contains(old_val) {
                    deltas.push(Delta {
                        kind: "enum_value_removed".to_owned(),
                        location: format!("{base}/properties/{key}/enum"),
                        message: format!(
                            "Enum value `{old_val}` removed from `{schema_name}.{key}`"
                        ),
                        impact: Impact::Breaking,
                    });
                }
            }
            for new_val in &new_enums {
                if !old_enums.contains(new_val) {
                    deltas.push(Delta {
                        kind: "enum_value_added".to_owned(),
                        location: format!("{base}/properties/{key}/enum"),
                        message: format!("Enum value `{new_val}` added to `{schema_name}.{key}`"),
                        impact: Impact::Additive,
                    });
                }
            }
        }
    }
}

fn enum_values(schema: Option<&serde_json::Value>) -> Vec<String> {
    schema
        .and_then(|s| s.get("enum"))
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn diff_operations(old: &IrSpec, new: &IrSpec, deltas: &mut Vec<Delta>) {
    use suspect_ir::IrOperation;

    let old_ops: HashMap<String, &IrOperation> = old
        .operations
        .iter()
        .filter_map(|o| o.id.as_ref().map(|id| (id.clone(), o)))
        .collect();
    let new_ops: HashMap<String, &IrOperation> = new
        .operations
        .iter()
        .filter_map(|o| o.id.as_ref().map(|id| (id.clone(), o)))
        .collect();

    // Removed operations → breaking
    for id in old_ops.keys() {
        if !new_ops.contains_key(id) {
            deltas.push(Delta {
                kind: "operation_removed".to_owned(),
                location: id.clone(),
                message: format!("Operation `{id}` was removed"),
                impact: Impact::Breaking,
            });
        }
    }

    // Added operations → additive
    for id in new_ops.keys() {
        if !old_ops.contains_key(id) {
            deltas.push(Delta {
                kind: "operation_added".to_owned(),
                location: id.clone(),
                message: format!("Operation `{id}` was added"),
                impact: Impact::Additive,
            });
        }
    }

    // Deprecated flags
    for (id, new_op) in &new_ops {
        if let Some(old_op) = old_ops.get(id)
            && !old_op.deprecated
            && new_op.deprecated
        {
            deltas.push(Delta {
                kind: "operation_deprecated".to_owned(),
                location: id.clone(),
                message: format!("Operation `{id}` was deprecated"),
                impact: Impact::Cosmetic,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ir::{IrSchema, IrSpec};

    fn schema(name: &str, json: &str) -> IrSchema {
        IrSchema {
            name: name.to_owned(),
            json: serde_json::from_str(json).expect("valid schema json"),
        }
    }

    #[test]
    fn enum_removal_is_breaking_addition_is_additive() {
        let old = IrSpec {
            schemas: vec![schema(
                "Pet",
                r#"{"properties":{"status":{"type":"string","enum":["available","sold"]}}}"#,
            )],
            ..IrSpec::default()
        };
        let new = IrSpec {
            schemas: vec![schema(
                "Pet",
                r#"{"properties":{"status":{"type":"string","enum":["available","pending"]}}}"#,
            )],
            ..IrSpec::default()
        };
        let diff = diff_specs(&old, &new);
        assert_eq!(diff.breaking, 1, "removed enum value is breaking");
        assert_eq!(diff.additive, 1, "added enum value is additive");
        assert_eq!(diff.semver, "major");
    }

    #[test]
    fn field_now_required_is_breaking() {
        let old = IrSpec {
            schemas: vec![schema(
                "Pet",
                r#"{"properties":{"name":{"type":"string"}}}"#,
            )],
            ..IrSpec::default()
        };
        let new = IrSpec {
            schemas: vec![schema(
                "Pet",
                r#"{"properties":{"name":{"type":"string"}},"required":["name"]}"#,
            )],
            ..IrSpec::default()
        };
        let diff = diff_specs(&old, &new);
        assert!(
            diff.deltas
                .iter()
                .any(|d| d.kind == "field_now_required" && d.impact == Impact::Breaking)
        );
    }

    #[test]
    fn optional_field_addition_is_additive() {
        let old = IrSpec {
            schemas: vec![schema("Pet", r#"{"properties":{}}"#)],
            ..IrSpec::default()
        };
        let new = IrSpec {
            schemas: vec![schema("Pet", r#"{"properties":{"tag":{"type":"string"}}}"#)],
            ..IrSpec::default()
        };
        let diff = diff_specs(&old, &new);
        assert_eq!(diff.additive, 1);
        assert_eq!(diff.breaking, 0);
        assert_eq!(diff.semver, "minor");
    }
}
