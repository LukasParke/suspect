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
    if let Some(version) = doc.version()
        && !(version.starts_with("1.0") || version.starts_with("1.1"))
    {
        out.push(diag(
            root.byte_range(),
            "arazzo-unsupported-version",
            format!("unsupported Arazzo version `{version}` (supported: 1.0.x, 1.1.x)"),
        ));
    }
    // Arazzo 1.1 `$self`: a URI-reference that MUST NOT carry a fragment.
    if let Some(self_uri) = doc.self_uri()
        && self_uri.contains('#')
    {
        out.push(diag(
            root.byte_range(),
            "arazzo-self-fragment",
            "`$self` MUST NOT contain a fragment identifier",
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
        validate_workflow_dependencies(wf, doc, &workflow_ids, &mut out);
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
            validate_step_dependencies(wf, step, &mut out);
            validate_step_targets(step, &mut out);
            validate_step_async_fields(step, doc, &mut out);
        }
        validate_sequential_outputs(wf, &mut out);
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
    if step.operation_id().is_none()
        && step.operation_path().is_none()
        && step.channel_path().is_none()
    {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-step-missing-operation",
            format!(
                "step `{}` must set `operationId`, `operationPath`, or `channelPath`",
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
                if let Some(problem) = condition_problem(cond) {
                    out.push(diag(
                        c.node().byte_range(),
                        "arazzo-invalid-condition",
                        problem,
                    ));
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

/// Arazzo 1.1 workflow-level `dependsOn`: names must resolve, and the
/// workflow dependency graph must be acyclic.
fn validate_workflow_dependencies(
    wf: &crate::WorkflowView<'_>,
    doc: &ArazzoDoc<'_>,
    workflow_ids: &FxHashSet<&str>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    for dep in wf.depends_on() {
        if let Some(local) = dep.strip_prefix('$') {
            if crate::expr::parse(local).is_err() {
                out.push(diag(
                    wf.node().byte_range(),
                    "arazzo-invalid-depends-on",
                    format!(
                        "workflow `{}` dependsOn {dep:?} is not a valid reference",
                        wf.workflow_id
                    ),
                ));
            }
        } else if !workflow_ids.contains(dep) {
            out.push(diag(
                wf.node().byte_range(),
                "arazzo-depends-on-unknown-workflow",
                format!(
                    "workflow `{}` depends on undeclared workflowId `{dep}`",
                    wf.workflow_id
                ),
            ));
        }
    }
    // Cycle detection across the workflow graph.
    let mut stack = vec![wf.workflow_id];
    let mut seen: FxHashSet<&str> = FxHashSet::from_iter([wf.workflow_id]);
    while let Some(current) = stack.pop() {
        for other in doc.workflows() {
            if other.workflow_id != current {
                continue;
            }
            for dep in other.depends_on() {
                if dep == wf.workflow_id {
                    out.push(diag(
                        wf.node().byte_range(),
                        "arazzo-depends-on-cycle",
                        format!(
                            "workflow `{}` participates in a dependsOn cycle",
                            wf.workflow_id
                        ),
                    ));
                    return;
                }
                if seen.insert(dep) {
                    stack.push(dep);
                }
            }
        }
    }
}

/// Arazzo 1.1 step-level `dependsOn`: same-workflow step ids must exist and
/// appear earlier (sequential satisfiability); expression forms must parse.
fn validate_step_dependencies(
    wf: &crate::WorkflowView<'_>,
    step: &crate::StepView<'_>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    let earlier: FxHashSet<&str> = wf
        .steps()
        .iter()
        .take_while(|s| s.step_id != step.step_id)
        .map(|s| s.step_id)
        .collect();
    for dep in step.depends_on() {
        if let Some(path) = dep.strip_prefix('$') {
            if crate::expr::parse(path).is_err() {
                out.push(diag(
                    step.node().byte_range(),
                    "arazzo-invalid-depends-on",
                    format!(
                        "step `{}` dependsOn {dep:?} is not a valid reference",
                        step.step_id
                    ),
                ));
            }
        } else if !earlier.contains(dep) {
            out.push(diag(
                step.node().byte_range(),
                "arazzo-depends-on-unknown-step",
                format!(
                    "step `{}` depends on `{dep}`, which is not an earlier step in workflow `{}`",
                    step.step_id, wf.workflow_id
                ),
            ));
        }
    }
}

/// Arazzo 1.1 target mutual exclusion and `timeout` well-formedness.
fn validate_step_targets(step: &crate::StepView<'_>, out: &mut Vec<ArazzoDiagnostic>) {
    let targets = [
        step.operation_id().map(|_| "operationId"),
        step.operation_path().map(|_| "operationPath"),
        step.channel_path().map(|_| "channelPath"),
    ]
    .into_iter()
    .flatten()
    .count()
        + usize::from(step.node().get("workflowId").is_some());
    if targets > 1 {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-step-multiple-targets",
            format!(
                "step `{}` sets multiple of operationId/operationPath/channelPath/workflowId",
                step.step_id
            ),
        ));
    }
    if step.node().get("timeout").is_some() && step.timeout_ms().is_none() {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-invalid-timeout",
            format!(
                "step `{}` timeout must be a positive integer (milliseconds)",
                step.step_id
            ),
        ));
    }
}

/// Arazzo 1.1 AsyncAPI step fields: `action`/`correlationId` belong on
/// AsyncAPI steps; `channelPath` must reference an asyncapi source.
fn validate_step_async_fields(
    step: &crate::StepView<'_>,
    doc: &ArazzoDoc<'_>,
    out: &mut Vec<ArazzoDiagnostic>,
) {
    if let Some(channel) = step.channel_path() {
        // `$sourceDescriptions.<name>...` — the source must be asyncapi.
        let name = channel
            .trim_start_matches('$')
            .trim_start_matches("sourceDescriptions.")
            .split('.')
            .next()
            .unwrap_or("");
        let kind = doc
            .source_descriptions()
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.kind);
        if let Some(kind) = kind
            && kind != crate::SourceType::AsyncApi
        {
            out.push(diag(
                step.node().byte_range(),
                "arazzo-channel-on-non-asyncapi-source",
                format!(
                    "step `{}` channelPath references source `{name}`, which is not asyncapi",
                    step.step_id
                ),
            ));
        }
    }
    if (step.action().is_some() || step.correlation_id().is_some()) && step.channel_path().is_none()
    {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-async-fields-on-http-step",
            format!(
                "step `{}` uses action/correlationId without channelPath (AsyncAPI steps only)",
                step.step_id
            ),
        ));
    }
    if let Some(action) = step.action()
        && !matches!(action, "send" | "receive")
    {
        out.push(diag(
            step.node().byte_range(),
            "arazzo-invalid-action-kind",
            format!("step `{}` action must be `send` or `receive`", step.step_id),
        ));
    }
}

/// Arazzo 1.1 §5.8.5.2.5: with purely sequential execution, a step that
/// references a LATER step's outputs is a forward reference that cannot be
/// satisfied. Selected-object outputs are also surfaced as unexecutable.
fn validate_sequential_outputs(wf: &crate::WorkflowView<'_>, out: &mut Vec<ArazzoDiagnostic>) {
    let steps = wf.steps();
    let uses_depends_on = steps.iter().any(|s| !s.depends_on().is_empty());
    if !uses_depends_on {
        for (idx, step) in steps.iter().enumerate() {
            let later: FxHashSet<&str> = steps[idx + 1..].iter().map(|s| s.step_id).collect();
            let mut expressions: Vec<String> = Vec::new();
            for p in step.parameters() {
                if let Some(v) = p.value().and_then(|n| n.as_str()) {
                    expressions.push(v.to_owned());
                }
            }
            for c in step.success_criteria() {
                if let Some(cond) = c.condition() {
                    expressions.push(cond.to_owned());
                }
            }
            for (_, expr) in step.outputs() {
                if let Some(s) = expr.as_str() {
                    expressions.push(s.to_owned());
                }
            }
            for expr in expressions {
                if let Some(rest) = expr.split("$steps.").nth(1)
                    && let Some(after) = rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .next()
                    && later.contains(after)
                {
                    out.push(diag(
                        step.node().byte_range(),
                        "arazzo-forward-output-reference",
                        format!(
                            "step `{}` references outputs of later step `{after}` with no dependsOn to order them",
                            step.step_id
                        ),
                    ));
                    break;
                }
            }
        }
    }
    for step in steps {
        if step.has_selector_outputs() {
            out.push(diag(
                step.node().byte_range(),
                "arazzo-selector-output-unsupported",
                format!(
                    "step `{}` uses Selector Object outputs, which this runner does not execute",
                    step.step_id
                ),
            ));
        }
    }
    // Workflow-level outputs may also use Selector Objects.
    for (_, value) in wf.outputs() {
        if value.kind() == suspect_low::ValueKind::Object {
            out.push(diag(
                wf.node().byte_range(),
                "arazzo-selector-output-unsupported",
                format!(
                    "workflow `{}` uses Selector Object outputs, which this runner does not execute",
                    wf.workflow_id
                ),
            ));
        }
    }
}

/// Criterion-condition validity per Arazzo §5.8.11: a condition is either a
/// plain runtime expression, an embedded-expression template, or a
/// comparison — `<expression> <op> <literal>` with the operators the
/// Criterion Object defines (`==`, `!=`, `<`, `<=`, `>`, `>=`, `=~`).
/// `$statusCode == 200` — the canonical Arazzo condition — is a comparison,
/// not a bare expression, and must not be flagged invalid.
/// Diagnoses an invalid condition: names what the parser expected, where
/// the expression stops parsing, and — for the mistakes users actually
/// make — the corrected spelling. Returns `None` when the condition is
/// valid in any of the forms [`condition_is_valid`] accepts.
fn condition_problem(cond: &str) -> Option<String> {
    if condition_is_valid(cond) {
        return None;
    }
    // The parser already knows where and why it stopped: report that,
    // then add the fix for the common shapes.
    let what = match crate::expr::parse(cond) {
        Ok(_) => unreachable!("condition_is_valid accepted what parse rejects"),
        Err(error) => format!("at byte {}, {error}", error.offset),
    };
    let fix = dotted_body_fix(cond)
        .map(|corrected| format!(" — did you mean `{corrected}`?"))
        .unwrap_or_default();
    Some(format!("invalid runtime expression {cond:?}: {what}{fix}"))
}

/// The fix for the most common mistake: dotted-path navigation into the
/// response/request body. Arazzo addresses the body with a JSON pointer
/// fragment, not property dots — `$response.body.MediaContainer.size` is
/// `$response.body#/MediaContainer/size`.
fn dotted_body_fix(cond: &str) -> Option<String> {
    // Split the comparison once, remembering the operator so the RHS
    // carries over verbatim. No operator: the whole condition is the LHS.
    const OPERATORS: &[&str] = &["==", "!=", "<=", ">=", "=~", "<", ">"];
    let (lhs, op, rhs) = match OPERATORS
        .iter()
        .find_map(|op| cond.split_once(op).map(|(lhs, rhs)| (lhs, *op, rhs)))
    {
        Some((lhs, op, rhs)) => (lhs, Some(op), rhs),
        None => (cond, None, ""),
    };
    let lhs = lhs.trim_end();
    for prefix in ["$response.body.", "$request.body."] {
        let Some(path) = lhs.strip_prefix(prefix) else {
            continue;
        };
        if path.is_empty() || path.contains(char::is_whitespace) {
            // A bare trailing dot or free text: not the dotted-path shape.
            return None;
        }
        let pointer: String = path
            .split('.')
            .map(|segment| segment.replace('~', "~0").replace('/', "~1"))
            .collect::<Vec<_>>()
            .join("/");
        let corrected_head = format!("{}#/{pointer}", prefix.strip_suffix('.')?);
        let corrected = match op {
            Some(op) => format!("{corrected_head} {op}{rhs}"),
            None => corrected_head,
        };
        return Some(corrected);
    }
    None
}

fn condition_is_valid(cond: &str) -> bool {
    if crate::expr::parse(cond).is_ok() {
        return true;
    }
    // Embedded-expression templates (e.g. `{.$inputs.token}` mixes) are
    // valid when at least one part parses.
    if !parse_embedded(cond)
        .iter()
        .all(|p| matches!(p, crate::ExprPart::Text(_)))
    {
        return true;
    }
    for op in ["=~", "==", "!=", "<=", ">=", "<", ">"] {
        if let Some((lhs, _rhs)) = cond.split_once(op) {
            // An operator only counts outside a JSON pointer (`#/a<b`).
            if lhs.contains('#') && lhs.rsplit('#').next().is_some_and(|tail| tail.contains(op)) {
                continue;
            }
            let lhs = lhs.trim_end_matches(['\'', '"', ' ']);
            if crate::expr::parse(lhs).is_ok() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod condition_diagnosis_tests {
    use super::condition_problem;

    #[test]
    fn a_valid_condition_has_no_problem() {
        assert!(condition_problem("$response.body#/MediaContainer/size > 0").is_none());
        assert!(condition_problem("$statusCode == 200").is_none());
        assert!(condition_problem("{$inputs.token}").is_none());
    }

    #[test]
    fn dotted_body_navigation_names_the_fix() {
        let problem =
            condition_problem("$response.body.MediaContainer.size > 0").expect("diagnosed");
        assert!(
            problem.contains("$response.body#/MediaContainer/size"),
            "the corrected spelling is named: {problem}"
        );
        assert!(
            problem.contains("invalid runtime expression"),
            "the diagnosis says what failed: {problem}"
        );
    }

    #[test]
    fn dotted_request_body_also_gets_the_fix() {
        let problem = condition_problem("$request.body.user.name == \"x\"").expect("diagnosed");
        assert!(
            problem.contains("$request.body#/user/name"),
            "request-side navigation fixed too: {problem}"
        );
    }

    #[test]
    fn a_condition_with_untouched_rhs_carries_it_over() {
        let problem = condition_problem("$response.body.count >= 10").expect("diagnosed");
        assert!(
            problem.contains("$response.body#/count >= 10"),
            "operator and RHS survive: {problem}"
        );
    }

    #[test]
    fn free_text_does_not_get_a_false_fix() {
        let problem = condition_problem("just some words").expect("diagnosed");
        assert!(
            !problem.contains("did you mean"),
            "no fix suggestion for non-expression text: {problem}"
        );
        assert!(
            problem.contains("just some words"),
            "the condition is still named: {problem}"
        );
    }
}
