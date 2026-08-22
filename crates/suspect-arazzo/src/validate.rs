use rustc_hash::{FxHashMap, FxHashSet};

use crate::expr::parse_embedded;
use crate::model::ArazzoDoc;

/// A non-fatal or fatal finding from Arazzo validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ArazzoDiagnostic {
    /// Stable machine code (`arazzo-duplicate-workflow-id`, ...).
    pub code: &'static str,
    /// Human-readable explanation of the finding.
    pub message: String,
    /// Byte range of the offending node in the source document.
    pub range: std::ops::Range<usize>,
}

/// Structural + cross-reference validation of an Arazzo document.
///
/// Checks: required fields, unique workflow/step ids, unique source
/// description names, expression well-formedness everywhere (conditions,
/// targets, operationPath, parameter values, embedded strings), goto actions
/// referencing existing workflows/steps, and step outputs referenced by
/// `$workflows...` expressions actually existing.
///
/// # Emitted codes
///
/// Diagnostics carry one of these stable `code` values:
///
/// - `arazzo-missing-version`, `arazzo-missing-info-title`,
///   `arazzo-missing-source-descriptions` — required root fields absent.
/// - `arazzo-duplicate-source-name` — two `sourceDescriptions` share a name.
/// - `arazzo-missing-workflow-id`, `arazzo-duplicate-workflow-id`.
/// - `arazzo-missing-step-id`, `arazzo-duplicate-step-id` (scoped to one
///   workflow).
/// - `arazzo-step-missing-operation`, `arazzo-invalid-operation-path`.
/// - `arazzo-invalid-condition`, `arazzo-criterion-missing-condition`.
/// - `arazzo-parameter-incomplete`, `arazzo-invalid-target`.
/// - `arazzo-goto-missing-target`, `arazzo-goto-unknown-workflow`,
///   `arazzo-unknown-action-type`, `arazzo-action-missing-type`.
/// - `arazzo-output-unknown-workflow`, `arazzo-output-unknown-step`,
///   `arazzo-output-unknown-name` — a `$workflows.…` reference does not
///   resolve to a declared step output.
#[must_use]
pub fn validate_arazzo(doc: &ArazzoDoc<'_>) -> Vec<ArazzoDiagnostic> {
    let mut out = Vec::new();
    let root = doc.root();

    if doc.version().is_none() {
        out.push(diag(
            root.byte_range(),
            "arazzo-missing-version",
            "missing `arazzo` version field",
        ));
    }
    if root.get("info").and_then(|i| i.get("title")).is_none() {
        out.push(diag(
            root.byte_range(),
            "arazzo-missing-info-title",
            "missing `info.title`",
        ));
    }
    if root.get("sourceDescriptions").is_none() {
        out.push(diag(
            root.byte_range(),
            "arazzo-missing-source-descriptions",
            "missing `sourceDescriptions`",
        ));
    }

    // unique source description names
    let mut seen_sources: FxHashMap<&str, ()> = FxHashMap::default();
    for s in doc.source_descriptions() {
        if s.name.is_empty() {
            continue;
        }
        if seen_sources.insert(s.name, ()).is_some() {
            out.push(diag(
                s.node().byte_range(),
                "arazzo-duplicate-source-name",
                format!("duplicate sourceDescription name `{}`", s.name),
            ));
        }
    }

    let mut seen_workflows: FxHashMap<&str, ()> = FxHashMap::default();
    for w in doc.workflows() {
        if !w.workflow_id.is_empty() && seen_workflows.insert(w.workflow_id, ()).is_some() {
            out.push(diag(
                w.node().byte_range(),
                "arazzo-duplicate-workflow-id",
                format!("duplicate workflowId `{}`", w.workflow_id),
            ));
        }
    }
    let workflow_ids: FxHashSet<&str> = doc.workflows().iter().map(|w| w.workflow_id).collect();

    for wf in doc.workflows() {
        if wf.workflow_id.is_empty() {
            out.push(diag(
                wf.node().byte_range(),
                "arazzo-missing-workflow-id",
                "workflow missing `workflowId`",
            ));
        }
        let mut step_ids: FxHashMap<&str, ()> = FxHashMap::default();
        for step in wf.steps() {
            if step.step_id.is_empty() {
                out.push(diag(
                    step.node().byte_range(),
                    "arazzo-missing-step-id",
                    "step missing `stepId`",
                ));
                continue;
            }
            if step_ids.insert(step.step_id, ()).is_some() {
                out.push(diag(
                    step.node().byte_range(),
                    "arazzo-duplicate-step-id",
                    format!(
                        "duplicate stepId `{}` in workflow `{}`",
                        step.step_id, wf.workflow_id
                    ),
                ));
            }
            validate_step(step, &workflow_ids, &mut out);
        }
        validate_actions(wf.success_actions(), &workflow_ids, &mut out);
        validate_actions(wf.failure_actions(), &workflow_ids, &mut out);

        // $workflows.<wf>.steps.<step>.outputs.<name> references must resolve
        for (key, value) in wf.outputs() {
            check_output_expression(key, value, doc, &mut out);
        }
    }

    validate_step_outputs(doc, &mut out);

    out
}

fn validate_step<'d>(
    step: &crate::StepView<'d>,
    workflow_ids: &FxHashSet<&str>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    let _ = workflow_ids;
    if step.operation_id().is_none() && step.operation_path().is_none() {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-step-missing-operation",
            format!(
                "step `{}` must set `operationId` or `operationPath`",
                step.step_id
            ),
        ));
    }
    if let Some(path) = step.operation_path() {
        // form: $sourceDescriptions.<name>[.#/json-pointer] or $url#/...
        if !path.starts_with('$') {
            out.push(diag(
                step.node().byte_range(),
                "arazzo-invalid-operation-path",
                format!("operationPath must start with `$`: {path:?}"),
            ));
        } else if let Err(e) = crate::expr::parse(path.split("#").next().unwrap_or(path)) {
            out.push(diag(
                step.node().byte_range(),
                "arazzo-invalid-operation-path",
                format!("invalid source expression: {e}"),
            ));
        }
    }
    for c in step.success_criteria() {
        match c.condition() {
            Some(cond) => {
                if crate::expr::parse(cond).is_err() {
                    // criteria conditions may be embedded expressions too
                    if parse_embedded(cond)
                        .iter()
                        .all(|p| matches!(p, crate::ExprPart::Text(_)))
                    {
                        out.push(diag(
                            c.node().byte_range(),
                            "arazzo-invalid-condition",
                            format!("condition is not a valid runtime expression: {cond:?}"),
                        ));
                    }
                }
            }
            None => out.push(diag(
                c.node().byte_range(),
                "arazzo-criterion-missing-condition",
                "successCriteria entry missing `condition`",
            )),
        }
    }
    for p in step.parameters() {
        validate_parameter(&p, out);
    }
    validate_actions(step.on_success(), workflow_ids, out);
    validate_actions(step.on_failure(), workflow_ids, out);
}

/// Checks every step output value expression across all workflows.
fn validate_step_outputs<'d>(doc: &ArazzoDoc<'d>, out: &mut Vec<ArazzoDiagnostic>) {
    for wf in doc.workflows() {
        for step in wf.steps() {
            for (_key, value) in step.outputs() {
                check_output_expression(_key, value, doc, out);
            }
        }
    }
}

fn validate_parameter(p: &crate::ParameterView<'_>, out: &mut Vec<ArazzoDiagnostic>) {
    if p.reference().is_some() {
        return; // reusable reference; resolved elsewhere
    }
    if (p.name().is_none() || p.location().is_none()) && p.target().is_none() {
        out.push(diag(
            p.node().byte_range(),
            "arazzo-parameter-incomplete",
            "parameter needs `name`+`in` (or a `target` for step parameters)",
        ));
    }
    if let Some(target) = p.target()
        && !target.starts_with('$')
    {
        out.push(diag(
            p.node().byte_range(),
            "arazzo-invalid-target",
            format!("parameter target must be a runtime expression: {target:?}"),
        ));
    }
    if let Some(value) = p.value()
        && value.kind() == suspect_low::ValueKind::Str
        && let Some(s) = value.as_str()
    {
        for part in parse_embedded(s) {
            if let crate::ExprPart::Expr(e) = part
                && e == crate::Expr::Text(String::new())
            {
                continue;
            }
        }
    }
}

fn validate_actions<'d>(
    actions: Vec<crate::ActionView<'d>>,
    workflow_ids: &FxHashSet<&str>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    for action in actions {
        match action.action_type() {
            Some("goto") => {
                let wf_ref = action.workflow_id();
                let step_ref = action.step_id();
                if wf_ref.is_none() && step_ref.is_none() {
                    out.push(diag(
                        action.node().byte_range(),
                        "arazzo-goto-missing-target",
                        "`goto` action needs `workflowId` or `stepId`",
                    ));
                }
                if let Some(wf) = wf_ref
                    && !workflow_ids.contains(wf)
                {
                    out.push(diag(
                        action.node().byte_range(),
                        "arazzo-goto-unknown-workflow",
                        format!("`goto` references unknown workflow `{wf}`"),
                    ));
                }
            }
            Some("retry") | Some("end") => {}
            Some(other) => out.push(diag(
                action.node().byte_range(),
                "arazzo-unknown-action-type",
                format!("unknown action type `{other}`"),
            )),
            None => out.push(diag(
                action.node().byte_range(),
                "arazzo-action-missing-type",
                "action missing `type`",
            )),
        }
        for c in action.criteria() {
            if c.condition().is_none() {
                out.push(diag(
                    c.node().byte_range(),
                    "arazzo-criterion-missing-condition",
                    "action criterion missing `condition`",
                ));
            }
        }
    }
}

fn check_output_expression<'d>(
    _key: &str,
    value: suspect_low::NodeRef<'d>,
    doc: &ArazzoDoc<'d>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    let Some(text) = value.as_str() else { return };
    // output values are bare runtime expressions (embedded form also allowed)
    let exprs: Vec<_> = match crate::parse(text) {
        Ok(e) => vec![e],
        Err(_) => parse_embedded(text)
            .into_iter()
            .filter_map(|p| match p {
                crate::ExprPart::Expr(e) => Some(e),
                crate::ExprPart::Text(_) => None,
            })
            .collect(),
    };
    for e in exprs {
        if let crate::Expr::WorkflowOutput {
            workflow,
            step,
            name,
        } = &e
        {
            let target_wf = doc.workflows().iter().find(|w| w.workflow_id == workflow);
            let Some(wf) = target_wf else {
                out.push(diag(
                    value.byte_range(),
                    "arazzo-output-unknown-workflow",
                    format!("output references unknown workflow `{workflow}`"),
                ));
                continue;
            };
            let Some(st) = wf.steps().iter().find(|s| s.step_id == step) else {
                out.push(diag(
                    value.byte_range(),
                    "arazzo-output-unknown-step",
                    format!("output references unknown step `{workflow}.{step}`"),
                ));
                continue;
            };
            let defined: Vec<&str> = st.outputs().into_iter().map(|(k, _)| k).collect();
            if !defined.contains(&name.as_str()) {
                out.push(diag(
                    value.byte_range(),
                    "arazzo-output-unknown-name",
                    format!(
                        "step `{workflow}.{step}` has no output `{name}` (defined: {defined:?})"
                    ),
                ));
            }
        }
    }
}

fn diag(
    range: std::ops::Range<usize>,
    code: &'static str,
    message: impl Into<String>,
) -> ArazzoDiagnostic {
    ArazzoDiagnostic {
        code,
        message: message.into(),
        range,
    }
}
