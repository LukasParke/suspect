use serde_json::{Value, json};

use crate::output::{Finding, Severity};

/// Maps the tool's severity rank to a SARIF level (`note`, `warning`,
/// `error`).
fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info | Severity::Hint => "note",
    }
}

/// Builds a SARIF 2.1.0 log from findings. Rules are collected from the
/// findings (deduplicated by id, first description wins); each result
/// references its rule by index and locates the finding at
// `file:startLine:startColumn`.
#[must_use]
pub fn sarif_log(findings: &[Finding]) -> Value {
    // Deduplicate rules preserving first-seen order.
    let mut rule_ids: Vec<String> = Vec::new();
    let mut rules: Vec<Value> = Vec::new();
    for f in findings {
        if !rule_ids.contains(&f.code) {
            rule_ids.push(f.code.clone());
            rules.push(json!({
                "id": f.code,
                "shortDescription": {"text": f.code},
            }));
        }
    }
    let results: Vec<Value> = findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.code,
                "ruleIndex": rule_ids.iter().position(|r| *r == f.code).unwrap_or(0),
                "level": level(f.severity),
                "message": {"text": f.message},
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": f.file},
                        "region": {
                            "startLine": f.line,
                            "startColumn": f.col,
                        },
                    },
                }],
            })
        })
        .collect();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "suspect",
                    "informationUri": "https://github.com/LukasParke/suspect",
                    "rules": rules,
                },
            },
            "results": results,
        }],
    })
}

/// Pretty-prints a SARIF log to stdout.
///
/// # Errors
/// Propagates serialization failures.
pub fn print_sarif(findings: &[Finding]) -> anyhow::Result<()> {
    crate::output::print_json(&sarif_log(findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Finding> {
        vec![
            Finding {
                file: "spec.yaml".into(),
                severity: Severity::Error,
                code: "oas-schema-instance-invalid".into(),
                message: "default violates the schema".into(),
                line: 12,
                col: 3,
                range: None,
            },
            Finding {
                file: "spec.yaml".into(),
                severity: Severity::Warning,
                code: "oas-schema-instance-invalid".into(),
                message: "example violates the schema".into(),
                line: 20,
                col: 5,
                range: None,
            },
        ]
    }

    #[test]
    fn log_is_valid_sarif_210() {
        let log = sarif_log(&sample());
        assert_eq!(log["version"], "2.1.0");
        let run = &log["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], "suspect");
        // Two findings share one rule → one deduplicated rule entry.
        assert_eq!(run["tool"]["driver"]["rules"].as_array().unwrap().len(), 1);
        assert_eq!(run["results"].as_array().unwrap().len(), 2);
        assert_eq!(run["results"][0]["level"], "error");
        assert_eq!(run["results"][1]["level"], "warning");
        assert_eq!(
            run["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"],
            12
        );
    }
}
