//! The rule table for load-profile diagnostics ([`super::read_profile_lenient`]
//! and [`super::profile::check_against_plan`]). Blocking findings default to
//! error, advisory ones to warning.

use suspect_diag::{CheckId, RuleInfo, Severity};

const fn error(code: &'static str, summary: &'static str) -> RuleInfo {
    RuleInfo::new(code, CheckId::LoadProfile, Severity::Error, summary)
}

const fn warning(code: &'static str, summary: &'static str) -> RuleInfo {
    RuleInfo::new(code, CheckId::LoadProfile, Severity::Warning, summary)
}

const RULES: &[RuleInfo] = &[
    error(
        "load-missing-version",
        "Profile without a `suspectLoad` version",
    ),
    error(
        "load-unsupported-version",
        "Load profile version not supported",
    ),
    error("load-type", "Value has the wrong type"),
    error("load-missing-phases", "Profile without phases"),
    error("load-empty-phases", "Profile with an empty phase list"),
    error("load-missing-scenarios", "Profile without scenarios"),
    error(
        "load-empty-scenarios",
        "Profile with an empty scenario list",
    ),
    error(
        "load-threshold-shape",
        "Threshold is not a single `metric: max` pair",
    ),
    error("load-condition-shape", "Condition without an expression"),
    error(
        "load-condition-syntax",
        "Condition expression does not parse",
    ),
    error("load-duration", "Invalid duration"),
    error("load-phase-shape", "Phase is not a mapping"),
    error(
        "load-phase-kind",
        "Phase does not set exactly one arrival kind",
    ),
    error("load-phase-duration", "Phase without a valid duration"),
    error("load-phase-rate", "Invalid arrival rate"),
    error("load-phase-count", "Arrival count below 1"),
    error("load-phase-vus", "Virtual user count below 1"),
    warning(
        "load-phase-ramp",
        "`rampTo` on a phase that is not `arrivalRate`",
    ),
    error("load-scenario-workflow", "Scenario without a workflow id"),
    error("load-scenario-weight", "Scenario weight is not positive"),
    error("load-empty-pool", "Input pool is empty"),
    error("load-data-path", "Data source without a path"),
    error("load-data-order", "Unknown data order"),
    error("load-data-missing", "Data file not found"),
    error("load-data-fields", "CSV data without field names"),
    error(
        "load-unknown-workflow",
        "Workflow is not in the Arazzo document",
    ),
    warning("load-hook-is-scenario", "Hook workflow is also a scenario"),
];

/// Every code the load-profile reader and plan cross-check emit.
#[must_use]
pub fn rules() -> &'static [RuleInfo] {
    RULES
}
