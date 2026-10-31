//! The rule registry: every builtin code, registered once, and the
//! aliases that name the same problem under another code.

use std::collections::HashMap;

use suspect_diag::render::RuleLookup;
use suspect_diag::{CheckId, RuleFamily, RuleInfo, Severity};

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

/// Codes the check pass itself emits: syntax, the configuration-file
/// schema and vendor-extension schemas.
const PASS_RULES: &[RuleInfo] = &[
    RuleInfo::new(
        crate::checks::SYNTAX_CODE,
        CheckId::Syntax,
        Severity::Error,
        "YAML or JSON syntax error",
    ),
    RuleInfo::new(
        suspect_config::schema::problems::CODE,
        CheckId::Config,
        Severity::Warning,
        "Unknown key, wrong type or deprecated spelling in a suspect configuration file",
    ),
    RuleInfo::new(
        suspect_config::schema::problems::MANIFEST_POLICY_CODE,
        CheckId::Config,
        Severity::Warning,
        "Policy section in a project manifest (deprecated; move it to .suspect.yaml)",
    ),
    RuleInfo::new(
        crate::checks::extensions::CODE,
        CheckId::Extension,
        Severity::Error,
        "Vendor extension value fails its registered schema",
    ),
    RuleInfo::new(
        crate::rules::RUNTIME_UNAVAILABLE,
        CheckId::Rules,
        Severity::Warning,
        "TypeScript rules are configured but bun is not on PATH",
    ),
    RuleInfo::new(
        crate::rules::ID_COLLISION,
        CheckId::Rules,
        Severity::Error,
        "TypeScript rule id reuses a builtin code or alias",
    ),
    RuleInfo {
        how_to_fix: "Name a code or alias the checks report; `syntax-error` cannot be suppressed.",
        ..RuleInfo::new(
            crate::suppress::UNKNOWN_CODE,
            CheckId::Suppression,
            Severity::Warning,
            "Suppression names an unknown or unsuppressible code",
        )
    },
    RuleInfo {
        how_to_fix: "Remove the suppression, or the code it no longer needs.",
        ..RuleInfo::new(
            crate::suppress::UNUSED,
            CheckId::Suppression,
            Severity::Hint,
            "Suppression matched no finding (off unless `lint.rules` turns it on)",
        )
    },
];

/// Alias table v1: canonical code, then the codes that report the same
/// problem at the same position. The canonical code belongs to the check
/// that resolves `$ref`s and carries fix guidance (validate, arazzo,
/// overlay); the aliases are lint rules (and the editor's old `syntax`).
/// An alias may name several canonical codes (`arazzo-step-operation`).
pub const ALIASES: &[(&str, &[&str])] = &[
    (
        "oas-operation-missing-operationId",
        &["operation-operationId"],
    ),
    (
        "oas-duplicate-operation-id",
        &["operation-operationId-unique"],
    ),
    ("oas-path-trailing-slash", &["path-keys-no-trailing-slash"]),
    ("oas-license-missing-url", &["license-url"]),
    ("oas-tag-undeclared", &["operation-tag-defined"]),
    ("arazzo-step-missing-operation", &["arazzo-step-operation"]),
    ("arazzo-step-multiple-targets", &["arazzo-step-operation"]),
    (
        "overlay-action-missing-description",
        &["overlay-action-description"],
    ),
    (crate::checks::SYNTAX_CODE, &["syntax"]),
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
        suspect_lint::markdown::rules(),
        PASS_RULES,
    ];
    #[cfg(feature = "sdk")]
    tables.push(suspect_codegen::admission::rules());
    tables
}

/// The builtin code families: admission's open-ended `http-*` and
/// contract-compiler codes, registered by prefix.
#[must_use]
pub fn builtin_families() -> &'static [RuleFamily] {
    #[cfg(feature = "sdk")]
    return suspect_codegen::admission::families();
    #[cfg(not(feature = "sdk"))]
    return &[];
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
    /// An alias entry names a canonical code no table registers.
    UnknownCanonical {
        /// The code.
        code: &'static str,
    },
    /// A dynamic rule (a TS rule id) names a builtin code or alias.
    Collision {
        /// The rule id.
        code: String,
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
            Self::UnknownCanonical { code } => {
                write!(f, "alias entry names unregistered code `{code}`")
            }
            Self::Collision { code } => {
                write!(f, "rule id `{code}` reuses a builtin code or alias")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Every known code with its metadata, and the alias map.
///
/// An alias may itself be a registered code (the lint rule
/// `operation-operationId`, for instance, keeps its own entry, which a
/// custom ruleset redefining the rule still uses).
#[derive(Debug, Clone, Default)]
pub struct Registry {
    by_code: HashMap<&'static str, RuleInfo>,
    /// Registration order, for deterministic iteration.
    order: Vec<&'static str>,
    /// Alias -> canonical codes, in [`ALIASES`] order.
    alias_to: HashMap<&'static str, Vec<&'static str>>,
    /// Codes known at run time only (TS rule ids), by check.
    dynamic: HashMap<Box<str>, CheckId>,
    /// Prefix families for open-ended codes, consulted after `by_code`.
    families: Vec<RuleFamily>,
}

impl Registry {
    /// Every builtin table ([`builtin_tables`]).
    ///
    /// # Panics
    /// When two builtin tables register the same code; the registry tests
    /// rule this out.
    #[must_use]
    pub fn builtin() -> Self {
        match Self::from_tables(&builtin_tables()).and_then(|r| r.with_aliases(ALIASES)) {
            Ok(registry) => registry.with_families(builtin_families()),
            Err(error) => panic!("builtin rule tables conflict: {error}"),
        }
    }

    /// Adds alias entries; each canonical code's [`RuleInfo::aliases`] is
    /// set from them.
    ///
    /// # Errors
    /// [`RegistryError::UnknownCanonical`] when an entry names a code no
    /// table registered.
    pub fn with_aliases(
        mut self,
        aliases: &'static [(&'static str, &'static [&'static str])],
    ) -> Result<Self, RegistryError> {
        for (canonical, names) in aliases {
            let Some(rule) = self.by_code.get_mut(canonical) else {
                return Err(RegistryError::UnknownCanonical { code: canonical });
            };
            rule.aliases = names;
            for name in *names {
                self.alias_to.entry(name).or_default().push(canonical);
            }
        }
        Ok(self)
    }

    /// Adds prefix families: a code no table lists takes the metadata of
    /// the family with its longest matching prefix.
    #[must_use]
    pub fn with_families(mut self, families: &[RuleFamily]) -> Self {
        self.families.extend_from_slice(families);
        self
    }

    /// The family `code` belongs to (longest prefix), when no table lists
    /// it exactly.
    #[must_use]
    pub fn family(&self, code: &str) -> Option<&RuleFamily> {
        if self.by_code.contains_key(code) {
            return None;
        }
        self.families
            .iter()
            .filter(|f| code.starts_with(f.prefix))
            .max_by_key(|f| f.prefix.len())
    }

    /// Every family, in registration order.
    #[must_use]
    pub fn families(&self) -> &[RuleFamily] {
        &self.families
    }

    /// Adds codes known only at run time (TS rule ids), so suppressions
    /// naming them are recognized.
    ///
    /// # Errors
    /// [`RegistryError::Collision`] when a code is a builtin code or alias.
    pub fn with_dynamic(
        mut self,
        codes: impl IntoIterator<Item = (String, CheckId)>,
    ) -> Result<Self, RegistryError> {
        for (code, check) in codes {
            if self.is_builtin_name(&code) {
                return Err(RegistryError::Collision { code });
            }
            self.dynamic.insert(code.into_boxed_str(), check);
        }
        Ok(self)
    }

    /// Whether `code` is a builtin code, an alias, or in a builtin family.
    #[must_use]
    pub fn is_builtin_name(&self, code: &str) -> bool {
        self.by_code.contains_key(code)
            || self.alias_to.contains_key(code)
            || self.family(code).is_some()
    }

    /// Whether `code` is known at all: builtin, an alias, or dynamic.
    #[must_use]
    pub fn is_known(&self, code: &str) -> bool {
        self.is_builtin_name(code) || self.dynamic.contains_key(code)
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

    /// The canonical code for a registered code that is not an alias;
    /// `None` for an alias (see [`Registry::alias_targets`]) or an unknown
    /// code.
    #[must_use]
    pub fn canonical(&self, code: &str) -> Option<&'static str> {
        if self.alias_to.contains_key(code) {
            return None;
        }
        self.by_code.get(code).map(|rule| rule.code)
    }

    /// The canonical codes an alias names; empty when `code` is not an
    /// alias.
    #[must_use]
    pub fn alias_targets(&self, code: &str) -> &[&'static str] {
        self.alias_to.get(code).map_or(&[], Vec::as_slice)
    }

    /// Every canonical code `code` resolves to: itself when registered and
    /// not an alias, an alias's targets, else nothing.
    #[must_use]
    pub fn resolve(&self, code: &str) -> Vec<&'static str> {
        match self.alias_to.get(code) {
            Some(targets) => targets.clone(),
            None => self.canonical(code).into_iter().collect(),
        }
    }

    /// Metadata for a canonical code, else for the family it belongs to.
    #[must_use]
    pub fn info(&self, code: &str) -> Option<&RuleInfo> {
        self.by_code
            .get(code)
            .or_else(|| self.family(code).map(|f| &f.rule))
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
    fn every_alias_entry_names_registered_codes() {
        let registry = Registry::builtin();
        for (canonical, aliases) in ALIASES {
            let info = registry.info(canonical).expect("canonical registered");
            assert_ne!(info.check, CheckId::Lint, "{canonical} is a lint code");
            for alias in *aliases {
                // The lint rule keeps its entry; only the editor's old
                // `syntax` code was never a rule.
                assert!(
                    registry.info(alias).is_some() || *alias == "syntax",
                    "{alias} is not a registered code"
                );
            }
        }
    }

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
        assert_eq!(registry.canonical("operation-operationId"), None);
        assert_eq!(
            registry.alias_targets("operation-operationId"),
            ["oas-operation-missing-operationId"]
        );
        assert_eq!(
            registry.resolve("arazzo-step-operation"),
            [
                "arazzo-step-missing-operation",
                "arazzo-step-multiple-targets"
            ]
        );
        assert_eq!(registry.resolve("syntax"), ["syntax-error"]);
        assert_eq!(
            registry
                .info("oas-operation-missing-operationId")
                .map(|r| r.aliases),
            Some(&["operation-operationId"][..])
        );
        let info = registry.info("arazzo-missing-version").unwrap();
        assert_eq!(info.check, CheckId::Arazzo);
        assert_eq!(registry.rules().count(), registry.len());
        assert_eq!(
            registry.rule("overlay-empty-actions").map(|r| r.default),
            Some(Severity::Error)
        );
    }

    #[cfg(feature = "sdk")]
    #[test]
    fn families_answer_open_ended_admission_codes() {
        let registry = Registry::builtin();
        // Exact entries win over the family.
        let exact = registry.info("http-operation-id").unwrap();
        assert_eq!(exact.code, "http-operation-id");
        assert!(registry.family("http-operation-id").is_none());
        let member = registry.info("http-query-parameter-style").unwrap();
        assert_eq!(member.code, "http-*");
        assert_eq!(member.check, CheckId::Admission);
        assert_eq!(member.class, suspect_diag::RuleClass::Sdk);
        assert!(!member.summary.is_empty());
        assert_eq!(
            registry.info("HTTP_REFERENCE_CYCLE").map(|r| r.code),
            Some("HTTP_REFERENCE*")
        );
        // `invalid-ref` is a `$ref` code; the `invalid-` family does not
        // take it over.
        assert_eq!(
            registry.info("invalid-ref").map(|r| r.check),
            Some(CheckId::Ref)
        );
        assert!(registry.is_known("unsupported-schema-keyword"));
        assert!(registry.is_builtin_name("http-anything"));
        assert!(!registry.is_known("team-rule"));
    }
}
