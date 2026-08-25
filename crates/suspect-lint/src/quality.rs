//! Specification quality scoring with actionable feedback.
//!
//! Unlike binary lint pass/fail, this module produces a continuous 0–100
//! score per dimension with prioritized, actionable recommendations ranked
//! by developer impact.

use serde::{Deserialize, Serialize};

use suspect_ir::IrSpec;

/// One quality dimension scored 0–100.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    /// Stable dimension identifier.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Score from 0 (worst) to 100 (best).
    pub score: u8,
    /// Prioritized improvement actions.
    pub actions: Vec<Action>,
}

/// One concrete improvement recommendation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// What to do.
    pub message: String,
    /// Where to do it (JSON pointer into the spec).
    pub location: String,
    /// Estimated effort: `trivial` | `small` | `medium` | `large`.
    pub effort: String,
    /// How many consumers benefit (from traffic data, if available).
    pub consumer_impact: Option<u32>,
}

/// The full quality report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityReport {
    /// Per-dimension scores.
    pub dimensions: Vec<Dimension>,
    /// Overall weighted average.
    pub overall: u8,
    /// Total actionable recommendations.
    pub total_actions: usize,
}

/// Computes the quality score for a spec.
#[must_use]
pub fn score_spec(spec: &IrSpec) -> QualityReport {
    let dimensions = vec![
        score_documentation(spec),
        score_consistency(spec),
        score_constraints(spec),
        score_error_docs(spec),
        score_deprecation(spec),
    ];

    let total: u32 = dimensions.iter().map(|d| u32::from(d.score)).sum();
    let overall = if dimensions.is_empty() {
        0
    } else {
        (total / dimensions.len() as u32) as u8
    };
    let total_actions: usize = dimensions.iter().map(|d| d.actions.len()).sum();

    QualityReport {
        dimensions,
        overall,
        total_actions,
    }
}

fn score_documentation(spec: &IrSpec) -> Dimension {
    let mut ops_total = 0usize;
    let mut ops_with_desc = 0usize;
    let mut schemas_total = 0usize;
    let mut schemas_with_desc = 0usize;
    let mut props_total = 0usize;
    let mut props_with_desc = 0usize;

    for op in &spec.operations {
        ops_total += 1;
        if op.description.is_some() {
            ops_with_desc += 1;
        }
    }

    for schema in &spec.schemas {
        schemas_total += 1;
        if schema.json.get("description").is_some() {
            schemas_with_desc += 1;
        }
        if let Some(props) = schema.json.get("properties").and_then(|p| p.as_object()) {
            for (_name, prop) in props {
                props_total += 1;
                if prop.get("description").is_some() {
                    props_with_desc += 1;
                }
            }
        }
    }

    let mut actions = Vec::new();
    let mut score = 100u64;
    let mut weight = 0u64;

    // Operations missing descriptions
    if ops_total > 0 {
        let ratio = ops_with_desc as f64 / ops_total as f64;
        weight += 3;
        score -= ((1.0 - ratio) * 40.0) as u64;
        if ratio < 0.9 {
            let missing: Vec<&str> = spec
                .operations
                .iter()
                .filter(|o| o.description.is_none())
                .filter_map(|o| o.id.as_deref())
                .take(5)
                .collect();
            actions.push(Action {
                message: format!(
                    "Add descriptions to {} operations (e.g. {})",
                    ops_total - ops_with_desc,
                    missing.join(", ")
                ),
                location: "paths".to_owned(),
                effort: "small".to_owned(),
                consumer_impact: None,
            });
        }
    }

    // Schemas missing descriptions
    if schemas_total > 0 {
        let ratio = schemas_with_desc as f64 / schemas_total as f64;
        weight += 2;
        score -= ((1.0 - ratio) * 30.0) as u64;
    }

    // Properties missing descriptions
    if props_total > 0 {
        let ratio = props_with_desc as f64 / props_total as f64;
        weight += 3;
        score -= ((1.0 - ratio) * 30.0) as u64;
        if ratio < 0.8 {
            let mut examples: Vec<String> = Vec::new();
            for schema in &spec.schemas {
                if let Some(props) = schema.json.get("properties").and_then(|p| p.as_object()) {
                    for (name, prop) in props {
                        if prop.get("description").is_none() && examples.len() < 3 {
                            examples.push(format!("{}.{}", schema.name, name));
                        }
                    }
                }
            }
            actions.push(Action {
                message: format!(
                    "Document field descriptions for {} properties (e.g. {})",
                    props_total - props_with_desc,
                    examples.join(", ")
                ),
                location: "components/schemas".to_owned(),
                effort: "medium".to_owned(),
                consumer_impact: None,
            });
        }
    }

    let score = if weight == 0 {
        100u8
    } else {
        (score / weight.max(1)).min(100) as u8
    };

    Dimension {
        id: "documentation".to_owned(),
        label: "Documentation completeness".to_owned(),
        score,
        actions,
    }
}

fn score_consistency(spec: &IrSpec) -> Dimension {
    let mut actions = Vec::new();
    let mut issues = 0usize;

    // Check for inconsistent parameter naming (camelCase vs snake_case)
    let mut camel = 0usize;
    let mut snake = 0usize;
    for op in &spec.operations {
        for param in &op.parameters {
            let name = &param.name;
            if name.contains('_') && name.to_lowercase() == name.as_str() {
                snake += 1;
            } else if name.chars().any(|c| c.is_uppercase()) && !name.contains('_') {
                camel += 1;
            }
        }
    }
    if camel > 0 && snake > 0 {
        issues += 1;
        actions.push(Action {
            message: format!(
                "Mixed parameter naming: {} camelCase, {} snake_case. Pick one convention.",
                camel, snake
            ),
            location: "paths/*/parameters".to_owned(),
            effort: "medium".to_owned(),
            consumer_impact: None,
        });
    }

    // Check for inconsistent schema naming (PascalCase expected)
    let bad_names: Vec<&str> = spec
        .schemas
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| {
            n.chars().next().is_none_or(|c| !c.is_uppercase()) || n.contains('_') || n.contains('-')
        })
        .take(5)
        .collect();
    if !bad_names.is_empty() {
        issues += 1;
        actions.push(Action {
            message: format!(
                "Schema names should be PascalCase without separators: {}",
                bad_names.join(", ")
            ),
            location: "components/schemas".to_owned(),
            effort: "small".to_owned(),
            consumer_impact: None,
        });
    }

    // Check for missing operationIds
    let no_ids: Vec<String> = spec
        .operations
        .iter()
        .filter(|o| o.id.is_none())
        .map(|o| format!("{} {}", o.method.as_str().to_uppercase(), o.path))
        .take(3)
        .collect();
    if !no_ids.is_empty() {
        issues += 1;
        actions.push(Action {
            message: format!(
                "Missing operationId on {} operations (e.g. {})",
                spec.operations.iter().filter(|o| o.id.is_none()).count(),
                no_ids.join("; ")
            ),
            location: "paths".to_owned(),
            effort: "trivial".to_owned(),
            consumer_impact: None,
        });
    }

    let score = (100u8).saturating_sub(issues as u8 * 15);
    Dimension {
        id: "consistency".to_owned(),
        label: "Naming and style consistency".to_owned(),
        score,
        actions,
    }
}

fn score_constraints(spec: &IrSpec) -> Dimension {
    let mut actions = Vec::new();
    let mut string_fields = 0usize;
    let mut constrained_fields = 0usize;
    let mut number_fields = 0usize;
    let mut range_fields = 0usize;

    for schema in &spec.schemas {
        if let Some(props) = schema.json.get("properties").and_then(|p| p.as_object()) {
            for (_name, prop) in props {
                let prop_type = prop.get("type").and_then(|t| t.as_str());
                match prop_type {
                    Some("string") => {
                        string_fields += 1;
                        if prop.get("pattern").is_some()
                            || prop.get("enum").is_some()
                            || prop.get("minLength").is_some()
                            || prop.get("maxLength").is_some()
                            || prop.get("format").is_some()
                        {
                            constrained_fields += 1;
                        }
                    }
                    Some("integer") | Some("number") => {
                        number_fields += 1;
                        if prop.get("minimum").is_some()
                            || prop.get("maximum").is_some()
                            || prop.get("exclusiveMinimum").is_some()
                            || prop.get("exclusiveMaximum").is_some()
                        {
                            range_fields += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let string_ratio = if string_fields > 0 {
        constrained_fields as f64 / string_fields as f64
    } else {
        1.0
    };
    let number_ratio = if number_fields > 0 {
        range_fields as f64 / number_fields as f64
    } else {
        1.0
    };

    if string_ratio < 0.5 {
        actions.push(Action {
            message: format!(
                "{} of {} string fields lack validation (pattern/enum/length/format). \
                 Unvalidated strings are a common source of injection and parsing bugs.",
                string_fields - constrained_fields,
                string_fields
            ),
            location: "components/schemas/*/properties".to_owned(),
            effort: "medium".to_owned(),
            consumer_impact: None,
        });
    }
    if number_ratio < 0.5 {
        actions.push(Action {
            message: format!(
                "{} of {} numeric fields lack range constraints (minimum/maximum). \
                 Unbounded numbers cause overflow and DoS.",
                number_fields - range_fields,
                number_fields
            ),
            location: "components/schemas/*/properties".to_owned(),
            effort: "small".to_owned(),
            consumer_impact: None,
        });
    }

    let score = ((string_ratio + number_ratio) * 50.0) as u8;
    Dimension {
        id: "constraints".to_owned(),
        label: "Input validation density".to_owned(),
        score,
        actions,
    }
}

fn score_error_docs(spec: &IrSpec) -> Dimension {
    let mut actions = Vec::new();
    let mut ops_with_errors = 0usize;
    let mut ops_total = 0usize;
    let mut no_error_ops: Vec<String> = Vec::new();

    for op in &spec.operations {
        ops_total += 1;
        let has_error = op
            .responses
            .iter()
            .any(|r| r.status.is_some_and(|code| code >= 400));
        if has_error {
            ops_with_errors += 1;
        } else {
            no_error_ops.push(format!("{} {}", op.method.as_str().to_uppercase(), op.path));
        }
    }

    let ratio = if ops_total > 0 {
        ops_with_errors as f64 / ops_total as f64
    } else {
        1.0
    };

    if ratio < 0.7 && !no_error_ops.is_empty() {
        actions.push(Action {
            message: format!(
                "{} of {} operations don't document error responses (4xx/5xx). \
                 Consumers can't handle failures they don't expect. E.g. {}",
                ops_total - ops_with_errors,
                ops_total,
                no_error_ops
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            location: "paths/*/responses".to_owned(),
            effort: "small".to_owned(),
            consumer_impact: None,
        });
    }

    let score = (ratio * 100.0) as u8;
    Dimension {
        id: "error_docs".to_owned(),
        label: "Error response documentation".to_owned(),
        score,
        actions,
    }
}

fn score_deprecation(spec: &IrSpec) -> Dimension {
    let mut actions = Vec::new();
    let deprecated_ops: Vec<&str> = spec
        .operations
        .iter()
        .filter(|o| o.deprecated)
        .filter_map(|o| o.id.as_deref())
        .collect();

    // Check for deprecated operations without sunset info in description
    let no_sunset: Vec<&str> = deprecated_ops
        .iter()
        .copied()
        .filter(|id| {
            spec.operations
                .iter()
                .find(|o| o.id.as_deref() == Some(*id))
                .and_then(|o| o.description.as_deref())
                .is_none_or(|d| {
                    !d.to_lowercase().contains("sunset") && !d.to_lowercase().contains("removal")
                })
        })
        .collect();

    if !no_sunset.is_empty() {
        actions.push(Action {
            message: format!(
                "{} deprecated operations lack removal timeline in description. \
                 Add \"Sunset: YYYY-MM-DD\" so consumers can plan migration.",
                no_sunset.len()
            ),
            location: "paths".to_owned(),
            effort: "trivial".to_owned(),
            consumer_impact: None,
        });
    }

    let score = if deprecated_ops.is_empty() {
        100
    } else {
        let clean = 1.0 - (no_sunset.len() as f64 / deprecated_ops.len() as f64);
        (clean * 100.0) as u8
    };

    Dimension {
        id: "deprecation".to_owned(),
        label: "Deprecation hygiene".to_owned(),
        score,
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ir::{IrOperation, IrResponse, IrSchema, Method};

    fn ir(ops: Vec<IrOperation>, schemas: Vec<IrSchema>) -> IrSpec {
        IrSpec {
            operations: ops,
            schemas,
            ..IrSpec::default()
        }
    }

    fn op(id: &str, path: &str, statuses: &[Option<u16>]) -> IrOperation {
        IrOperation {
            id: Some(id.to_owned()),
            method: Method::Get,
            path: path.to_owned(),
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            parameters: Vec::new(),
            body_schema: None,
            responses: statuses
                .iter()
                .map(|s| IrResponse {
                    status: *s,
                    description: None,
                    schema: None,
                })
                .collect(),
        }
    }

    #[test]
    fn well_documented_spec_scores_high() {
        let spec = ir(
            vec![op("getPet", "/pets/{id}", &[Some(200), Some(404)])],
            vec![IrSchema {
                name: "Pet".to_owned(),
                json: serde_json::from_str(
                    r#"{"description":"A pet","properties":{"name":{"type":"string","description":"Name","minLength":1}}}"#,
                )
                .unwrap(),
            }],
        );
        let report = score_spec(&spec);
        assert!(report.overall >= 70, "got {}", report.overall);
        assert_eq!(report.dimensions.len(), 5);
    }

    #[test]
    fn undocumented_spec_scores_low_with_actions() {
        let spec = ir(
            vec![op("getPet", "/pets/{id}", &[Some(200)])],
            vec![IrSchema {
                name: "Pet".to_owned(),
                json: serde_json::from_str(r#"{"properties":{"name":{"type":"string"}}}"#).unwrap(),
            }],
        );
        let report = score_spec(&spec);
        assert!(report.overall < 80, "got {}", report.overall);
        assert!(report.total_actions > 0, "must surface actionable feedback");
    }
}
