//! `suspect arazzo-diff` — breaking changes between two Arazzo documents.

use std::path::PathBuf;

use suspect_overlay::Value as Ov;

/// One Arazzo breaking-change review.
#[derive(Debug, clap::Args)]
pub struct ArazzoDiffArgs {
    /// The previous Arazzo revision.
    #[arg(required = true)]
    pub old: PathBuf,
    /// The current Arazzo revision.
    #[arg(required = true)]
    pub new: PathBuf,
    /// Output format for the report.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// One detected break.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArazzoBreak {
    /// Human description.
    pub message: String,
    /// Severity: `error` or `warning`.
    pub severity: String,
}

impl ArazzoBreak {
    /// The finding severity as an output enum value.
    pub fn output_severity(&self) -> bool {
        self.severity == "error"
    }
}

/// Reviews OLD against NEW for workflow-level breaks.
///
/// # Errors
/// Propagates file IO failures.
pub fn arazzo_break_findings(
    old_path: &PathBuf,
    new_path: &PathBuf,
) -> anyhow::Result<Vec<ArazzoBreak>> {
    let old_text = std::fs::read_to_string(old_path)?;
    let new_text = std::fs::read_to_string(new_path)?;
    let parse = |text: &str| -> anyhow::Result<serde_json::Value> {
        let uri = suspect_source::Uri::parse("mem://arazzo.yaml")
            .map_err(|e| anyhow::Error::msg(e.to_string()))?;
        let low = suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        );
        let json = Ov::from_node(low.root()).to_json();
        serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|e| anyhow::Error::msg(e.to_string()))
    };
    let old_doc = parse(&old_text)?;
    let new_doc = parse(&new_text)?;
    let mut breaks = Vec::new();

    // Removed workflows.
    if let (Some(old_wfs), Some(new_wfs)) = (
        old_doc.get("workflows").and_then(|w| w.as_array()),
        new_doc.get("workflows").and_then(|w| w.as_array()),
    ) {
        let new_ids: Vec<&str> = new_wfs
            .iter()
            .filter_map(|w| w.get("workflowId").and_then(|v| v.as_str()))
            .collect();
        for wf in old_wfs {
            let Some(id) = wf.get("workflowId").and_then(|v| v.as_str()) else {
                continue;
            };
            if !new_ids.contains(&id) {
                breaks.push(ArazzoBreak {
                    message: format!("workflow '{id}' removed"),
                    severity: "error".into(),
                });
            }
        }
        // Removed steps within shared workflows.
        for new_wf in new_wfs {
            let Some(new_id) = new_wf.get("workflowId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(old_wf) = old_wfs
                .iter()
                .find(|w| w.get("workflowId").and_then(|v| v.as_str()) == Some(new_id))
            else {
                continue;
            };
            let (Some(old_steps), Some(new_steps)) = (
                old_wf.get("steps").and_then(|s| s.as_array()),
                new_wf.get("steps").and_then(|s| s.as_array()),
            ) else {
                continue;
            };
            let new_step_ids: Vec<&str> = new_steps
                .iter()
                .filter_map(|s| s.get("stepId").and_then(|v| v.as_str()))
                .collect();
            for old_step in old_steps {
                let Some(step_id) = old_step.get("stepId").and_then(|v| v.as_str()) else {
                    continue;
                };
                if !new_step_ids.contains(&step_id) {
                    breaks.push(ArazzoBreak {
                        message: format!("workflow '{new_id}': step '{step_id}' removed"),
                        severity: "error".into(),
                    });
                    continue;
                }
                let Some(new_step) = new_steps
                    .iter()
                    .find(|s| s.get("stepId").and_then(|v| v.as_str()) == Some(step_id))
                else {
                    continue;
                };
                // Changed operation target: the step now drives a
                // different operation — replays break.
                let old_target = step_operation_target(old_step);
                let new_target = step_operation_target(new_step);
                if let (Some(old), Some(new)) = (&old_target, &new_target)
                    && old != new
                {
                    breaks.push(ArazzoBreak {
                        message: format!(
                            "workflow '{new_id}': step '{step_id}' now targets \
                             operation '{new}' (was '{old}')"
                        ),
                        severity: "error".into(),
                    });
                }
                // Removed workflow inputs: callers supplying them fail.
                // Arazzo wraps concrete values under `$values`; the input
                // names live there when present.
                let old_inputs = input_names(old_wf.get("inputs"));
                let new_inputs = input_names(new_wf.get("inputs"));
                for input in &old_inputs - &new_inputs {
                    breaks.push(ArazzoBreak {
                        message: format!("workflow '{new_id}': workflow input '{input}' removed"),
                        severity: "error".into(),
                    });
                }
                // Removed outputs: dependent workflows lose their source.
                let old_outputs = name_set(old_wf.get("outputs"));
                let new_outputs = name_set(new_wf.get("outputs"));
                for output in &old_outputs - &new_outputs {
                    breaks.push(ArazzoBreak {
                        message: format!("workflow '{new_id}': workflow output '{output}' removed"),
                        severity: "error".into(),
                    });
                }
                // Removed step outputs: later steps' expressions dangle.
                let old_step_outputs = name_set(old_step.get("outputs"));
                let new_step_outputs = name_set(new_step.get("outputs"));
                for output in &old_step_outputs - &new_step_outputs {
                    breaks.push(ArazzoBreak {
                        message: format!(
                            "workflow '{new_id}': step '{step_id}' output \
                             '{output}' removed"
                        ),
                        severity: "error".into(),
                    });
                }
                // Changed success criteria: the pass/fail contract moves.
                if !criteria_equal(
                    old_step.get("successCriteria"),
                    new_step.get("successCriteria"),
                ) {
                    breaks.push(ArazzoBreak {
                        message: format!(
                            "workflow '{new_id}': step '{step_id}' success \
                             criteria changed"
                        ),
                        severity: "warning".into(),
                    });
                }
            }
        }
    }
    Ok(breaks)
}

/// The operation a step drives: `operationPath` (Arazzo) resolved to its
/// trailing operationId fragment, or the `workflowId` reference.
fn step_operation_target(step: &serde_json::Value) -> Option<String> {
    if let Some(operation_path) = step.get("operationPath").and_then(|v| v.as_str()) {
        // `#/paths/~1pets/get` or `openapi.yaml#/paths/...` — the
        // decoded operation identity is the fragment tail.
        let tail = operation_path.rsplit('/').next().unwrap_or(operation_path);
        return Some(tail.to_owned());
    }
    step.get("workflowId")
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Workflow input names: the keys of `inputs` or of its `$values` map.
fn input_names(value: Option<&serde_json::Value>) -> std::collections::BTreeSet<String> {
    let Some(inputs) = value else {
        return Default::default();
    };
    if let Some(values) = inputs.get("$values").and_then(|v| v.as_object()) {
        return values.keys().cloned().collect();
    }
    inputs
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// The set of mapping keys under an `inputs`/`outputs` node.
fn name_set(value: Option<&serde_json::Value>) -> std::collections::BTreeSet<String> {
    value
        .and_then(|v| v.as_object())
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// Structural equality for success criteria (a criterion list).
fn criteria_equal(old: Option<&serde_json::Value>, new: Option<&serde_json::Value>) -> bool {
    match (old, new) {
        (None, None) => true,
        (Some(old), Some(new)) => old == new,
        _ => false,
    }
}

/// Runs the review and prints the report; exits non-zero on breaks.
///
/// # Errors
/// Propagates serialization failures.
pub fn arazzo_diff(args: &ArazzoDiffArgs) -> anyhow::Result<i32> {
    use crate::output;
    let findings = arazzo_break_findings(&args.old, &args.new)?;
    let rendered: Vec<output::Finding> = findings
        .iter()
        .map(|f| output::Finding {
            file: format!("{} → {}", args.old.display(), args.new.display()),
            severity: if f.severity == "error" {
                output::Severity::Error
            } else {
                output::Severity::Warning
            },
            code: "arazzo-breaking-change".into(),
            message: f.message.clone(),
            line: 1,
            col: 1,
            range: None,
        })
        .collect();
    match args.text.format {
        crate::OutputFormat::Text => output::print_findings(&rendered),
        crate::OutputFormat::Json | crate::OutputFormat::Sarif => output::print_json(&findings)?,
    }
    Ok(i32::from(!findings.is_empty()))
}
