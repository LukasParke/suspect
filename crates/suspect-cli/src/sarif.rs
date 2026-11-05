use serde_json::Value;

use crate::output::{Finding, ShownPaths, to_diag};

/// Builds a SARIF 2.1.0 log from findings through
/// `suspect_diag::render::sarif`. Rules are collected from the findings
/// (deduplicated by id, the code as description); each result references
/// its rule by index and locates the finding at
/// `file:startLine:startColumn`.
#[must_use]
pub fn sarif_log(findings: &[Finding]) -> Value {
    suspect_diag::render::sarif(
        &to_diag(findings),
        &ShownPaths,
        &suspect_diag::render::NoRules,
    )
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
    use crate::output::Severity;

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
                range: Some(140..150),
            },
        ]
    }

    #[test]
    fn findings_round_trip_through_the_shared_model() {
        for finding in sample() {
            let back = Finding::from_diag(&finding.to_diag(), &ShownPaths);
            assert_eq!(back.to_line(), finding.to_line());
            assert_eq!(back.range, finding.range);
        }
        let text = crate::output::render_text(&sample());
        let expected: String = sample().iter().map(|f| f.to_line() + "\n").collect();
        assert_eq!(text, expected);
    }

    #[test]
    fn log_locates_cli_findings() {
        let log = sarif_log(&sample());
        let run = &log["runs"][0];
        assert_eq!(run["tool"]["driver"]["rules"].as_array().unwrap().len(), 1);
        assert_eq!(
            run["results"][1]["locations"][0]["physicalLocation"]["region"]["startLine"],
            20
        );
        assert_eq!(
            run["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "spec.yaml"
        );
    }
}
