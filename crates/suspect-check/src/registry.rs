//! The rule registry: every builtin code, registered once.

use std::collections::HashMap;

use suspect_diag::render::RuleLookup;
use suspect_diag::{CheckId, RuleInfo, Severity};

/// Overlay structure codes (`suspect_overlay::validate_overlay`). The table
/// lives here because `suspect-overlay` is a publishable foundation crate
/// and cannot depend on the unpublished `suspect-diag` yet.
const OVERLAY_RULES: &[RuleInfo] = &[
    RuleInfo::new(
        "overlay-empty-actions",
        CheckId::Overlay,
        Severity::Error,
        "Overlay without actions",
    ),
    RuleInfo::new(
        "overlay-update-with-remove",
        CheckId::Overlay,
        Severity::Warning,
        "Action sets `update` while `remove` is true",
    ),
    RuleInfo::new(
        "overlay-action-missing-description",
        CheckId::Overlay,
        Severity::Hint,
        "Action without a description",
    ),
];

/// The process-wide [`Registry::builtin`].
#[must_use]
pub fn builtin() -> &'static Registry {
    builtin_arc()
}

/// The process-wide [`Registry::builtin`], shareable.
#[must_use]
pub fn builtin_arc() -> &'static std::sync::Arc<Registry> {
    static BUILTIN: std::sync::OnceLock<std::sync::Arc<Registry>> = std::sync::OnceLock::new();
    BUILTIN.get_or_init(|| std::sync::Arc::new(Registry::builtin()))
}

/// The builtin rule tables, one per producer.
#[must_use]
pub fn builtin_tables() -> Vec<&'static [RuleInfo]> {
    #[cfg_attr(not(feature = "sdk"), allow(unused_mut))]
    let mut tables: Vec<&'static [RuleInfo]> = vec![
        suspect_validate::rules(),
        suspect_lint::builtin_rules(),
        suspect_arazzo::rules(),
        OVERLAY_RULES,
        suspect_test::load::rules(),
    ];
    #[cfg(feature = "sdk")]
    tables.push(suspect_codegen::admission::rules());
    tables
}

/// A code registered by two tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// `code` appears in the tables of both checks.
    Duplicate {
        /// The code.
        code: &'static str,
        /// The check that registered it first.
        first: CheckId,
        /// The check that registered it again.
        second: CheckId,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate {
                code,
                first,
                second,
            } => write!(
                f,
                "code `{code}` is registered by both {} and {}",
                first.name(),
                second.name()
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Every known code with its metadata. Codes have no aliases yet, so each
/// code is its own canonical code.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    by_code: HashMap<&'static str, RuleInfo>,
    /// Registration order, for deterministic iteration.
    order: Vec<&'static str>,
}

impl Registry {
    /// Every builtin table ([`builtin_tables`]).
    ///
    /// # Panics
    /// When two builtin tables register the same code; the registry tests
    /// rule this out.
    #[must_use]
    pub fn builtin() -> Self {
        match Self::from_tables(&builtin_tables()) {
            Ok(registry) => registry,
            Err(error) => panic!("builtin rule tables conflict: {error}"),
        }
    }

    /// Merges `tables`.
    ///
    /// # Errors
    /// [`RegistryError::Duplicate`] when a code is registered twice.
    pub fn from_tables(tables: &[&'static [RuleInfo]]) -> Result<Self, RegistryError> {
        let mut registry = Self::default();
        for rule in tables.iter().flat_map(|t| t.iter()) {
            if let Some(first) = registry.by_code.get(rule.code) {
                return Err(RegistryError::Duplicate {
                    code: rule.code,
                    first: first.check,
                    second: rule.check,
                });
            }
            registry.by_code.insert(rule.code, *rule);
            registry.order.push(rule.code);
        }
        Ok(registry)
    }

    /// The canonical code for `code`, when registered.
    #[must_use]
    pub fn canonical(&self, code: &str) -> Option<&'static str> {
        self.by_code.get(code).map(|rule| rule.code)
    }

    /// Metadata for a canonical code.
    #[must_use]
    pub fn info(&self, code: &str) -> Option<&RuleInfo> {
        self.by_code.get(code)
    }

    /// Every rule, in registration order.
    pub fn rules(&self) -> impl Iterator<Item = &RuleInfo> {
        self.order.iter().filter_map(|code| self.by_code.get(code))
    }

    /// Number of registered codes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

impl RuleLookup for Registry {
    fn rule(&self, code: &str) -> Option<RuleInfo> {
        self.info(code).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_are_rejected() {
        const A: &[RuleInfo] = &[RuleInfo::new("x", CheckId::Lint, Severity::Hint, "x")];
        const B: &[RuleInfo] = &[RuleInfo::new("x", CheckId::Validate, Severity::Hint, "x")];
        assert_eq!(
            Registry::from_tables(&[A, B]).unwrap_err(),
            RegistryError::Duplicate {
                code: "x",
                first: CheckId::Lint,
                second: CheckId::Validate,
            }
        );
    }

    #[test]
    fn lookups_follow_registration() {
        let registry = Registry::builtin();
        assert_eq!(
            registry.canonical("oas-duplicate-operation-id"),
            Some("oas-duplicate-operation-id")
        );
        assert_eq!(registry.canonical("not-a-code"), None);
        let info = registry.info("arazzo-missing-version").unwrap();
        assert_eq!(info.check, CheckId::Arazzo);
        assert_eq!(registry.rules().count(), registry.len());
        assert_eq!(
            registry.rule("overlay-empty-actions").map(|r| r.default),
            Some(Severity::Error)
        );
    }
}
