//! Builtin Overlay 1.0 and Arazzo 1.0 rule packs.

use crate::functions::Function;
use crate::rule::{FamilySet, Rule, Severity};

/// Overlay rules at their default severities.
pub(crate) fn overlay_rules() -> Vec<Rule> {
    let f = FamilySet::OVERLAY;
    vec![
        Rule::new(
            "overlay-info-description",
            "Overlay `info` object should have a `description`.",
            Severity::Hint,
            f,
            &["$.info"],
            Function::Defined {
                property: "description".into(),
            },
        ),
        Rule::new(
            "overlay-action-description",
            "Every overlay action should have a `description`.",
            Severity::Hint,
            f,
            &["$.actions.*"],
            Function::Defined {
                property: "description".into(),
            },
        ),
    ]
}

/// Arazzo rules at their default severities.
pub(crate) fn arazzo_rules() -> Vec<Rule> {
    let f = FamilySet::ARAZZO;
    vec![
        Rule::new(
            "arazzo-workflow-description",
            "Every Arazzo workflow should have a `description`.",
            Severity::Hint,
            f,
            &["$.workflows.*"],
            Function::Defined {
                property: "description".into(),
            },
        ),
        Rule::new(
            "arazzo-step-operation",
            "Every Arazzo step must target exactly one of `operationId` or `operationPath`.",
            Severity::Error,
            f,
            &["$.workflows.*.steps.*"],
            Function::Xor {
                properties: vec!["operationId".into(), "operationPath".into()],
            },
        ),
    ]
}

/// Both packs combined (used by [`Linter::spectral_default`]).
pub(crate) fn rules() -> Vec<Rule> {
    let mut all = overlay_rules();
    all.extend(arazzo_rules());
    all
}
