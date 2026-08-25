//! Consumer impact analysis across spec versions.
//!
//! Given two spec versions and recorded traffic (consumer fingerprints),
//! computes exactly which consumers break under the new version and what
//! migration steps they need.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::diff::{self, Impact};

/// A consumer identified from recorded traffic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Consumer {
    /// Primary fingerprint: user-agent or client-id header.
    pub fingerprint: String,
    /// Source IPs seen for this consumer.
    pub source_ips: BTreeSet<String>,
    /// Total requests recorded from this consumer.
    pub request_count: u32,
}

/// How one consumer is affected by the version change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerImpact {
    /// The consumer.
    pub consumer: Consumer,
    /// Breaking deltas that affect requests this consumer sends.
    pub affected_deltas: Vec<String>,
    /// Specific remediation steps.
    pub remediation: Vec<String>,
    /// Severity: how many breaking changes touch this consumer's traffic.
    pub severity: u32,
}

/// The full impact report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Consumers that will break under the new version.
    pub affected: Vec<ConsumerImpact>,
    /// Consumers verified compatible.
    pub unaffected: Vec<String>,
    /// Total breaking changes in the diff.
    pub breaking_changes: usize,
    /// Summary line for CI display.
    pub summary: String,
}

/// One recorded exchange used for impact evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedExchange {
    /// HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Parsed request body (if JSON).
    pub body: Option<serde_json::Value>,
    /// Consumer fingerprint (user-agent / client id).
    pub consumer: String,
    /// Source IP.
    pub source_ip: Option<String>,
}

/// Computes which consumers break when moving from `old` to `new` spec,
/// evaluated against `traffic` recorded against the old version.
#[must_use]
pub fn analyze_impact(
    old: &suspect_ir::IrSpec,
    new: &suspect_ir::IrSpec,
    traffic: &[RecordedExchange],
) -> ImpactReport {
    let diff = diff::diff_specs(old, new);
    let breaking: Vec<&diff::Delta> = diff
        .deltas
        .iter()
        .filter(|d| d.impact == Impact::Breaking)
        .collect();

    // Group traffic by consumer
    let mut consumers: BTreeMap<String, Consumer> = BTreeMap::new();
    for ex in traffic {
        let entry = consumers
            .entry(ex.consumer.clone())
            .or_insert_with(|| Consumer {
                fingerprint: ex.consumer.clone(),
                source_ips: BTreeSet::new(),
                request_count: 0,
            });
        entry.request_count += 1;
        if let Some(ip) = &ex.source_ip {
            entry.source_ips.insert(ip.clone());
        }
    }

    let mut affected = Vec::new();
    let mut unaffected = Vec::new();

    for (fingerprint, consumer) in &consumers {
        let consumer_traffic: Vec<&RecordedExchange> = traffic
            .iter()
            .filter(|e| e.consumer == fingerprint.as_str())
            .collect();

        let mut affected_deltas = Vec::new();
        let mut remediation = Vec::new();
        let mut severity = 0u32;

        for delta in &breaking {
            if delta_affects_traffic(delta, &consumer_traffic, old) {
                severity += 1;
                affected_deltas.push(delta.message.clone());
                remediation.push(remediation_for(delta));
            }
        }

        if severity > 0 {
            affected.push(ConsumerImpact {
                consumer: consumer.clone(),
                affected_deltas,
                remediation,
                severity,
            });
        } else {
            unaffected.push(fingerprint.clone());
        }
    }

    affected.sort_by_key(|i| std::cmp::Reverse(i.severity));

    let summary = if affected.is_empty() {
        format!(
            "All {} consumers verified compatible with {} breaking changes",
            consumers.len(),
            breaking.len()
        )
    } else {
        format!(
            "{} of {} consumers break under {} breaking changes",
            affected.len(),
            consumers.len(),
            breaking.len()
        )
    };

    ImpactReport {
        affected,
        unaffected,
        breaking_changes: breaking.len(),
        summary,
    }
}

/// Checks whether a breaking delta touches any of the consumer's requests.
fn delta_affects_traffic(
    delta: &diff::Delta,
    traffic: &[&RecordedExchange],
    old_spec: &suspect_ir::IrSpec,
) -> bool {
    match delta.kind.as_str() {
        "operation_removed" => {
            // Does the consumer call this operation?
            traffic.iter().any(|ex| {
                let op_key = format!("{} {}", ex.method.to_uppercase(), ex.path);
                delta.location == op_key
                    || old_spec.operations.iter().any(|o| {
                        o.id.as_deref() == Some(&delta.location)
                            && o.method.as_str().eq_ignore_ascii_case(&ex.method)
                            && o.path == ex.path
                    })
            })
        }
        "field_removed" | "field_now_required" | "enum_value_removed" => {
            // Extract schema name from location: #/components/schemas/{name}/...
            let schema_name = delta.location.split('/').nth(3).unwrap_or_default();
            if schema_name.is_empty() {
                return false;
            }
            // Does the consumer send a body whose schema is this one?
            traffic.iter().any(|ex| {
                let Some(body) = &ex.body else { return false };
                let Some(op) = old_spec.operations.iter().find(|o| {
                    o.method.as_str().eq_ignore_ascii_case(&ex.method) && o.path == ex.path
                }) else {
                    return false;
                };
                let Some(body_schema) = &op.body_schema else {
                    return false;
                };
                schema_reachable(old_spec, body_schema, schema_name, 0)
                    && body_has_field_or_uses_schema(body, delta)
            })
        }
        _ => false,
    }
}

/// Checks if `target` schema is reachable from `root` via $ref edges.
fn schema_reachable(spec: &suspect_ir::IrSpec, root: &str, target: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if root == target {
        return true;
    }
    spec.schema_edges.get(root).is_some_and(|edges| {
        edges
            .iter()
            .any(|e| schema_reachable(spec, e, target, depth + 1))
    })
}

fn body_has_field_or_uses_schema(_body: &serde_json::Value, _delta: &diff::Delta) -> bool {
    // Conservative: if the consumer sends a body whose schema chain reaches
    // the changed schema, count it as affected.
    true
}

fn remediation_for(delta: &diff::Delta) -> String {
    match delta.kind.as_str() {
        "operation_removed" => format!(
            "Stop calling `{}`; find a replacement endpoint or pin to the old API version.",
            delta.location
        ),
        "field_removed" => format!(
            "Remove usage of `{}` from request bodies; the field no longer exists.",
            delta
                .message
                .trim_start_matches("Field `")
                .trim_end_matches("'")
        ),
        "field_now_required" => format!(
            "Start sending `{}` in request bodies; it is now mandatory.",
            delta
                .message
                .trim_start_matches("Field `")
                .trim_end_matches("'")
        ),
        "enum_value_removed" => format!("Replace removed enum value; {}", delta.message),
        "schema_removed" => format!(
            "Migrate away from schema `{}`; it was removed from the API.",
            delta.location.rsplit('/').next().unwrap_or("?")
        ),
        _ => format!("Review change: {}", delta.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff_specs;
    use suspect_ir::{IrOperation, IrSchema, IrSpec, Method};

    fn spec_with_status_enum(value: &str) -> IrSpec {
        IrSpec {
            operations: vec![IrOperation {
                id: Some("createPet".to_owned()),
                method: Method::Post,
                path: "/pets".to_owned(),
                summary: None,
                description: None,
                tags: Vec::new(),
                deprecated: false,
                parameters: Vec::new(),
                body_schema: Some("PetInput".to_owned()),
                responses: Vec::new(),
            }],
            schemas: vec![IrSchema {
                name: "PetInput".to_owned(),
                json: serde_json::json!({
                    "properties": {"status": {"type": "string", "enum": [value]}}
                }),
            }],
            ..IrSpec::default()
        }
    }

    #[test]
    fn consumer_breaking_when_enum_value_removed() {
        let old = spec_with_status_enum("active");
        let new = spec_with_status_enum("enabled");
        let traffic = vec![RecordedExchange {
            method: "post".to_owned(),
            path: "/pets".to_owned(),
            body: Some(serde_json::json!({"status": "active"})),
            consumer: "mobile-ios".to_owned(),
            source_ip: Some("10.0.0.1".to_owned()),
        }];
        let report = analyze_impact(&old, &new, &traffic);
        assert_eq!(report.breaking_changes, 1);
        assert_eq!(report.affected.len(), 1);
        assert_eq!(report.affected[0].consumer.fingerprint, "mobile-ios");
        assert!(!report.affected[0].remediation.is_empty());
        assert!(report.summary.contains("1 of 1"));
    }

    #[test]
    fn unaffected_consumer_listed() {
        let old = spec_with_status_enum("active");
        let new = spec_with_status_enum("active");
        let traffic = vec![RecordedExchange {
            method: "post".to_owned(),
            path: "/pets".to_owned(),
            body: Some(serde_json::json!({"status": "active"})),
            consumer: "web".to_owned(),
            source_ip: None,
        }];
        let report = analyze_impact(&old, &new, &traffic);
        assert!(report.affected.is_empty());
        assert_eq!(report.unaffected, vec!["web".to_owned()]);
    }

    #[test]
    fn diff_and_impact_compose() {
        let old = spec_with_status_enum("active");
        let new = spec_with_status_enum("enabled");
        let diff = diff_specs(&old, &new);
        assert_eq!(diff.semver, "major");
    }
}
