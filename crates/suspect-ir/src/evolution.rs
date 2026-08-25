//! Traffic-informed spec evolution: detects undocumented fields in live
//! responses and proposes spec amendments.
//!
//! The gateway records real traffic; this module compares observed response
//! shapes against the declared schemas and produces a proposed YAML patch
//! for fields that appear in responses but are missing from documentation.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::IrSpec;

/// One undocumented field observed in live traffic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Operation the response came from (`operationId` or `METHOD /path`).
    pub operation: String,
    /// JSON pointer to the undocumented field (e.g. `/customer_id`).
    pub pointer: String,
    /// Inferred JSON type from observed values.
    pub inferred_type: String,
    /// How many times this field was observed.
    pub frequency: u32,
    /// Sample observed values (up to 3, for type inference confidence).
    pub samples: Vec<String>,
}

/// Proposed spec amendment for one undocumented field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// The operation this applies to.
    pub operation: String,
    /// Field name to add.
    pub field: String,
    /// Proposed OpenAPI schema fragment for the field.
    pub schema_fragment: String,
    /// Confidence: high (≥10 observations), medium (≥3), low (<3).
    pub confidence: String,
    /// Observation count backing this proposal.
    pub observations: u32,
}

/// The full evolution report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvolutionReport {
    /// All undocumented observations, grouped by operation.
    pub observations: Vec<Observation>,
    /// Proposed amendments, ranked by confidence then frequency.
    pub proposals: Vec<Proposal>,
    /// Operations with no undocumented fields (clean).
    pub clean_operations: usize,
}

/// Analyzes observed response bodies against declared schemas.
///
/// `responses` maps operation identifiers to a list of response bodies
/// (parsed JSON). `spec` provides the declared schemas for comparison.
#[must_use]
pub fn analyze_traffic(
    spec: &IrSpec,
    responses: &HashMap<String, Vec<serde_json::Value>>,
) -> EvolutionReport {
    let mut report = EvolutionReport::default();
    let mut by_op: BTreeMap<String, Vec<Observation>> = BTreeMap::new();

    for (op_id, bodies) in responses {
        let declared = declared_fields(spec, op_id);
        let mut op_obs: HashMap<String, Observation> = HashMap::new();

        for body in bodies {
            collect_undocumented(op_id, body, &declared, "", &mut op_obs);
        }

        if op_obs.is_empty() {
            report.clean_operations += 1;
        } else {
            let mut obs: Vec<Observation> = op_obs.into_values().collect();
            obs.sort_by(|a, b| {
                b.frequency
                    .cmp(&a.frequency)
                    .then(a.pointer.cmp(&pointer_key(&b.pointer)))
            });
            by_op.insert(op_id.clone(), obs);
        }
    }

    // Flatten observations in deterministic order
    for obs in by_op.into_values().flatten() {
        report.observations.push(obs);
    }

    // Build proposals from observations
    let mut proposals: Vec<Proposal> = report
        .observations
        .iter()
        .map(|o| Proposal {
            operation: o.operation.clone(),
            field: o.pointer.trim_start_matches('/').to_owned(),
            schema_fragment: infer_schema_fragment(o),
            confidence: if o.frequency >= 10 {
                "high".to_owned()
            } else if o.frequency >= 3 {
                "medium".to_owned()
            } else {
                "low".to_owned()
            },
            observations: o.frequency,
        })
        .collect();
    proposals.sort_by(|a, b| {
        b.observations
            .cmp(&a.observations)
            .then(a.operation.cmp(&b.operation))
            .then(a.field.cmp(&b.field))
    });
    report.proposals = proposals;

    report
}

fn pointer_key(p: &str) -> String {
    p.to_owned()
}

/// Returns the set of top-level property names declared for an operation's
/// success response schema.
fn declared_fields(spec: &IrSpec, op_id: &str) -> std::collections::HashSet<String> {
    let mut fields = std::collections::HashSet::new();

    let Some(op) = spec
        .operations
        .iter()
        .find(|o| o.id.as_deref() == Some(op_id))
        .or_else(|| {
            spec.operations
                .iter()
                .find(|o| format!("{} {}", o.method.as_str().to_uppercase(), o.path) == op_id)
        })
    else {
        return fields;
    };

    // Walk response schemas
    for resp in &op.responses {
        let Some(code) = resp.status else { continue };
        if !(200..300).contains(&code) {
            continue;
        }
        let Some(schema_name) = resp.schema.as_deref() else {
            continue;
        };
        collect_schema_fields(spec, schema_name, &mut fields, 0);
    }

    fields
}

fn collect_schema_fields(
    spec: &IrSpec,
    schema_name: &str,
    fields: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Some(schema) = spec.schemas.iter().find(|s| s.name == schema_name) else {
        return;
    };
    if let Some(props) = schema.json.get("properties").and_then(|p| p.as_object()) {
        for key in props.keys() {
            fields.insert(key.clone());
        }
    }
    // Follow allOf composition
    if let Some(all_of) = schema.json.get("allOf").and_then(|a| a.as_array()) {
        for item in all_of {
            if let Some(r) = item.get("$ref").and_then(|r| r.as_str()) {
                let name = r.rsplit('/').next().unwrap_or(r);
                collect_schema_fields(spec, name, fields, depth + 1);
            }
        }
    }
}

fn collect_undocumented(
    op_id: &str,
    value: &serde_json::Value,
    declared: &std::collections::HashSet<String>,
    prefix: &str,
    out: &mut HashMap<String, Observation>,
) {
    let serde_json::Value::Object(map) = value else {
        return;
    };
    for (key, val) in map {
        let pointer = if prefix.is_empty() {
            format!("/{key}")
        } else {
            format!("{prefix}/{key}")
        };
        if !declared.contains(key) {
            let entry = out.entry(pointer.clone()).or_insert_with(|| Observation {
                operation: op_id.to_owned(),
                pointer: pointer.clone(),
                inferred_type: json_type_name(val),
                frequency: 0,
                samples: Vec::new(),
            });
            entry.frequency += 1;
            if entry.samples.len() < 3 {
                let sample = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                entry.samples.push(sample.chars().take(80).collect());
            }
        }
        // Recurse into nested objects (one level of array unwrapping)
        match val {
            serde_json::Value::Object(_) => {
                collect_undocumented(op_id, val, declared, &pointer, out);
            }
            serde_json::Value::Array(items) => {
                if let Some(first) = items.first()
                    && first.is_object()
                {
                    collect_undocumented(op_id, first, declared, &pointer, out);
                }
            }
            _ => {}
        }
    }
}

fn json_type_name(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(_) => "boolean".to_owned(),
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer".to_owned()
            } else {
                "number".to_owned()
            }
        }
        serde_json::Value::String(_) => "string".to_owned(),
        serde_json::Value::Array(_) => "array".to_owned(),
        serde_json::Value::Object(_) => "object".to_owned(),
    }
}

fn infer_schema_fragment(o: &Observation) -> String {
    let type_line = match o.inferred_type.as_str() {
        "integer" => "type: integer".to_owned(),
        "number" => "type: number".to_owned(),
        "boolean" => "type: boolean".to_owned(),
        "array" => "type: array\n        items: {}".to_owned(),
        "object" => "type: object".to_owned(),
        "null" => "type: string\n        nullable: true".to_owned(),
        t => format!("type: {t}"),
    };
    format!(
        "      {}:\n        {}\n        description: >-\n          Undocumented field observed in {} live responses.",
        o.pointer.trim_start_matches('/'),
        type_line,
        o.frequency
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_with_pet() -> IrSpec {
        IrSpec {
            operations: vec![crate::IrOperation {
                id: Some("listPets".to_owned()),
                method: crate::Method::Get,
                path: "/pets".to_owned(),
                summary: None,
                description: None,
                tags: Vec::new(),
                deprecated: false,
                parameters: Vec::new(),
                body_schema: None,
                responses: vec![crate::IrResponse {
                    status: Some(200),
                    description: None,
                    schema: Some("Pets".to_owned()),
                }],
            }],
            schemas: vec![crate::IrSchema {
                name: "Pets".to_owned(),
                json: serde_json::json!({
                    "properties": {"id": {"type": "integer"}}
                }),
            }],
            ..IrSpec::default()
        }
    }

    #[test]
    fn detects_undocumented_response_field() {
        let spec = spec_with_pet();
        let mut traffic = HashMap::new();
        traffic.insert(
            "listPets".to_owned(),
            vec![serde_json::json!({"id": 1, "mystery": true})],
        );
        let report = analyze_traffic(&spec, &traffic);
        assert_eq!(report.observations.len(), 1);
        assert_eq!(report.observations[0].pointer, "/mystery");
        assert_eq!(report.observations[0].inferred_type, "boolean");
        assert_eq!(report.proposals.len(), 1);
        assert_eq!(report.proposals[0].confidence, "low");
    }

    #[test]
    fn documented_fields_are_clean() {
        let spec = spec_with_pet();
        let mut traffic = HashMap::new();
        traffic.insert("listPets".to_owned(), vec![serde_json::json!({"id": 1})]);
        let report = analyze_traffic(&spec, &traffic);
        assert_eq!(report.clean_operations, 1);
        assert!(report.proposals.is_empty());
    }

    #[test]
    fn confidence_ranks_with_frequency() {
        let spec = spec_with_pet();
        let mut traffic = HashMap::new();
        let bodies: Vec<serde_json::Value> = (0..12)
            .map(|i| serde_json::json!({"id": i, "extra": i}))
            .collect();
        traffic.insert("listPets".to_owned(), bodies);
        let report = analyze_traffic(&spec, &traffic);
        assert_eq!(report.proposals[0].confidence, "high");
        assert_eq!(report.proposals[0].observations, 12);
    }
}
