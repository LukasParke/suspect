//! The linting engine: compiled ruleset execution and finding types.

use std::marker::PhantomData;
use std::ops::Range;

use suspect_low::{LowDoc, Pointer};

use crate::rule::Rule;
use crate::{ruleset, Severity};

/// One lint result: a rule violation anchored to a byte range and pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding<'d> {
    /// Rule code that produced this finding (e.g. `info-contact`).
    pub code: Box<str>,
    /// Configured severity of the violated rule.
    pub severity: Severity,
    /// Human-readable message: the rule's description, or the function's
    /// default message prefixed by the code.
    pub message: String,
    /// Byte range of the offending node in the source text.
    pub range: Range<usize>,
    /// JSON pointer to the offending node within the document.
    pub path: Pointer,
    pub(crate) _marker: PhantomData<&'d ()>,
}

/// A compiled ruleset ready to run against documents.
#[derive(Debug)]
pub struct Linter {
    rules: Vec<Rule>,
}

impl Linter {
    /// The builtin Spectral-style default pack: all OAS rules plus the
    /// overlay/arazzo checks, each at its default severity. Family gating
    /// decides at run time which rules apply to a document.
    #[must_use]
    pub fn spectral_default() -> Self {
        let mut rules = crate::packs::oas::rules();
        rules.extend(crate::packs::overlay_arazzo::rules());
        Self { rules }
    }

    /// Builds a linter from already-compiled rules.
    pub(crate) fn from_rules(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// Compiles a Spectral-style ruleset (YAML or JSON) parsed through
    /// suspect-low.
    ///
    /// Supported shape:
    ///
    /// ```yaml
    /// extends: spectral:oas
    /// rules:
    ///   my-rule:
    ///     description: Info must have a contact email.
    ///     given: $.info
    ///     severity: warn
    ///     formats: [oas3]
    ///     then:
    ///       function: defined
    ///       functionOptions:
    ///         property: contact
    /// ```
    ///
    /// `extends` accepts `spectral:oas`, `spectral:overlay`, and
    /// `spectral:arazzo` (string or array); rules with the same code as an
    /// extended builtin override it.
    ///
    /// # Errors
    /// [`RulesetError`] for unknown extends targets, unknown functions,
    /// invalid options, bad severities/formats, or invalid JSONPath queries.
    pub fn from_ruleset(doc: &LowDoc) -> Result<Self, RulesetError> {
        ruleset::compile(doc)
    }

    /// Runs every enabled, family-matching rule over `doc`. Findings are
    /// returned deterministically ordered by `(range, code)`.
    #[must_use]
    pub fn run<'d>(&self, doc: &'d LowDoc) -> Vec<Finding<'d>> {
        let family = doc.sniff_family();
        let root = doc.root();
        let mut findings: Vec<Finding<'d>> = Vec::new();
        for rule in &self.rules {
            if !rule.severity.is_enabled() || !rule.formats.contains(family) {
                continue;
            }
            for path in &rule.given {
                for node in path.query(root) {
                    crate::functions::apply(rule, node, root, &mut findings);
                }
            }
        }
        findings.sort_unstable_by(|a, b| {
            (&a.range.start, &a.range.end, &a.code).cmp(&(&b.range.start, &b.range.end, &b.code))
        });
        findings.dedup_by(|a, b| a.range == b.range && a.code == b.code && a.message == b.message && a.path == b.path);
        findings
    }

    /// The codes of all compiled rules (including disabled ones).
    pub fn rule_codes(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| &*r.code)
    }
}

/// Everything that can go wrong while compiling a ruleset document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RulesetError {
    /// A top-level ruleset field is malformed (e.g. an unknown extends target).
    #[error("invalid ruleset field `{field}`: {message}")]
    InvalidRuleset {
        /// The offending top-level field name.
        field: String,
        /// What exactly is wrong with it.
        message: String,
    },
    /// An individual rule is malformed (unknown function, bad option value,
    /// bad severity or format).
    #[error("invalid rule `{code}`: {message}")]
    BadRule {
        /// The code of the offending rule.
        code: String,
        /// What exactly is wrong with the rule.
        message: String,
    },
    /// A `given` query is not valid RFC 9535 JSONPath.
    #[error(transparent)]
    JsonPath(#[from] suspect_jsonpath::PathError),
}
