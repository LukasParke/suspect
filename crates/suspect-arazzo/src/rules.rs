//! The rule table for [`crate::validate_arazzo`]. Every finding blocks a
//! run, so every code defaults to error.

use suspect_diag::{CheckId, RuleInfo, Severity};

const fn rule(code: &'static str, summary: &'static str) -> RuleInfo {
    RuleInfo::new(code, CheckId::Arazzo, Severity::Error, summary)
}

const RULES: &[RuleInfo] = &[
    rule(
        "arazzo-missing-version",
        "Document without an `arazzo` version",
    ),
    rule("arazzo-unsupported-version", "Arazzo version not supported"),
    rule("arazzo-missing-info-title", "Info object without a title"),
    rule(
        "arazzo-self-fragment",
        "`$self` contains a fragment identifier",
    ),
    rule(
        "arazzo-missing-source-descriptions",
        "Document without source descriptions",
    ),
    rule(
        "arazzo-duplicate-source-name",
        "Duplicate source description name",
    ),
    rule(
        "arazzo-missing-workflow-id",
        "Workflow without a workflowId",
    ),
    rule("arazzo-duplicate-workflow-id", "Duplicate workflowId"),
    rule("arazzo-missing-step-id", "Step without a stepId"),
    rule(
        "arazzo-duplicate-step-id",
        "Duplicate stepId within a workflow",
    ),
    rule(
        "arazzo-step-missing-operation",
        "Step without an operation, channel or workflow target",
    ),
    rule(
        "arazzo-step-multiple-targets",
        "Step with more than one target",
    ),
    rule(
        "arazzo-invalid-operation-path",
        "Invalid operationPath expression",
    ),
    rule("arazzo-invalid-condition", "Invalid criterion condition"),
    rule(
        "arazzo-criterion-missing-condition",
        "Criterion without a condition",
    ),
    rule(
        "arazzo-parameter-incomplete",
        "Parameter without a name and location or target",
    ),
    rule(
        "arazzo-invalid-target",
        "Parameter target is not a runtime expression",
    ),
    rule(
        "arazzo-goto-missing-target",
        "`goto` action without a workflowId or stepId",
    ),
    rule(
        "arazzo-goto-unknown-workflow",
        "`goto` action names an unknown workflow",
    ),
    rule("arazzo-unknown-action-type", "Unknown action type"),
    rule("arazzo-action-missing-type", "Action without a type"),
    rule(
        "arazzo-output-unknown-workflow",
        "Output reference names an unknown workflow",
    ),
    rule(
        "arazzo-output-unknown-step",
        "Output reference names an unknown step",
    ),
    rule(
        "arazzo-output-unknown-name",
        "Output reference names an undeclared output",
    ),
    rule("arazzo-invalid-depends-on", "Invalid dependsOn reference"),
    rule(
        "arazzo-depends-on-unknown-workflow",
        "dependsOn names an undeclared workflow",
    ),
    rule(
        "arazzo-depends-on-unknown-step",
        "dependsOn names an unknown or later step",
    ),
    rule(
        "arazzo-depends-on-cycle",
        "Workflows depend on each other in a cycle",
    ),
    rule(
        "arazzo-invalid-timeout",
        "Step timeout is not a positive integer",
    ),
    rule(
        "arazzo-channel-on-non-asyncapi-source",
        "channelPath on a source that is not AsyncAPI",
    ),
    rule(
        "arazzo-async-fields-on-http-step",
        "AsyncAPI step fields on an HTTP step",
    ),
    rule(
        "arazzo-invalid-action-kind",
        "Step action is neither `send` nor `receive`",
    ),
    rule(
        "arazzo-forward-output-reference",
        "Reference to a later step's outputs without dependsOn",
    ),
    rule(
        "arazzo-selector-output-unsupported",
        "Selector Object outputs are not executed",
    ),
];

/// Every code [`crate::validate_arazzo`] emits.
#[must_use]
pub fn rules() -> &'static [RuleInfo] {
    RULES
}
