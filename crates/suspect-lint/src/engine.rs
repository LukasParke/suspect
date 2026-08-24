//! The linting engine: compiled ruleset execution and finding types.

use std::marker::PhantomData;
use std::ops::Range;

use rustc_hash::FxHashMap;
use suspect_low::{LowDoc, NodeRef, Pointer, SpecFamily};

use crate::rule::Rule;
use crate::{Severity, ruleset};

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

        let enabled: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.severity.is_enabled() && r.formats.contains(family))
            .collect();

        // Fast path: ONE traversal feeding every classifiable rule, with
        // top-level sections walked in parallel; remaining rules fall back
        // to shared JSONPath evaluation. If even that shape is not
        // representable (aliases, merge keys), everything runs generic.
        let plan = crate::fast::Plan::compile(&enabled);
        if let Some((buckets, ptrs)) = plan.execute(root) {
            let profile = std::env::var_os("SUSPECT_PROFILE").is_some();
            let t_apply = profile.then(std::time::Instant::now);
            let mut findings: Vec<Finding<'d>> = Vec::new();
            plan.apply(&enabled, &buckets, &ptrs, &mut findings);
            if let Some(t) = t_apply {
                eprintln!(
                    "[lint fast] apply {:.2} ms ({} findings)",
                    t.elapsed().as_secs_f64() * 1000.0,
                    findings.len()
                );
            }
            // Rules with unclassifiable queries: evaluate via the JSONPath
            // engine (shared query cache; typically few distinct queries).
            let t_gen = profile.then(std::time::Instant::now);
            let mut query_cache: FxHashMap<Box<str>, std::sync::Arc<suspect_jsonpath::NodeList>> =
                FxHashMap::default();
            let empty_ptrs = crate::fast::PtrMap::empty();
            for &ri in &plan.generic {
                let rule = enabled[ri];
                for path in &rule.given {
                    let key = path.as_key().into_boxed_str();
                    let matches = match query_cache.get(&key) {
                        Some(m) => std::sync::Arc::clone(m),
                        None => {
                            let m = std::sync::Arc::new(path.query(root));
                            query_cache.insert(key, std::sync::Arc::clone(&m));
                            m
                        }
                    };
                    for node in matches.iter() {
                        crate::functions::apply(rule, node, &empty_ptrs, &mut findings);
                    }
                }
            }
            if let Some(t) = t_gen {
                eprintln!(
                    "[lint fast] generic rules {:.2} ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
            let t_fin = profile.then(std::time::Instant::now);
            let out = finalize(findings);
            if let Some(t) = t_fin {
                eprintln!(
                    "[lint fast] finalize {:.2} ms ({} findings)",
                    t.elapsed().as_secs_f64() * 1000.0,
                    out.len()
                );
            }
            return out;
        }
        self.run_generic(root, family)
    }

    /// Generic engine: distinct `given` expressions are deduped and
    /// evaluated once per document via the JSONPath evaluator, then each
    /// rule consumes its precomputed match list.
    fn run_generic<'d>(&self, root: NodeRef<'d>, family: SpecFamily) -> Vec<Finding<'d>> {
        // Single-pass query evaluation: rules commonly share queries (many
        // target `$.paths[*].get` etc.), so this collapses repeated
        // full-tree walks into one per distinct expression.
        let mut query_cache: FxHashMap<Box<str>, std::sync::Arc<suspect_jsonpath::NodeList>> =
            FxHashMap::default();
        let mut findings: Vec<Finding<'d>> = Vec::new();
        for rule in &self.rules {
            if !rule.severity.is_enabled() || !rule.formats.contains(family) {
                continue;
            }
            // Optional per-rule cost breakdown (`SUSPECT_PROFILE=1`).
            let profile = std::env::var_os("SUSPECT_PROFILE").is_some();
            let t = profile.then(std::time::Instant::now);
            for path in &rule.given {
                let key = path.as_key().into_boxed_str();
                let matches = match query_cache.get(&key) {
                    Some(m) => std::sync::Arc::clone(m),
                    None => {
                        let m = std::sync::Arc::new(path.query(root));
                        query_cache.insert(key, std::sync::Arc::clone(&m));
                        m
                    }
                };
                let empty_ptrs = crate::fast::PtrMap::empty();
                for node in matches.iter() {
                    crate::functions::apply(rule, node, &empty_ptrs, &mut findings);
                }
            }
            if let Some(t) = t {
                eprintln!(
                    "[suspect-lint profile]   {:>9.2} ms  {}",
                    t.elapsed().as_secs_f64() * 1000.0,
                    rule.code
                );
            }
        }
        finalize(findings)
    }

    /// The codes of all compiled rules (including disabled ones).
    pub fn rule_codes(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| &*r.code)
    }
}

/// Sorts findings by `(range, code)` and removes exact duplicates.
fn finalize(mut findings: Vec<Finding<'_>>) -> Vec<Finding<'_>> {
    findings.sort_unstable_by(|a, b| {
        (&a.range.start, &a.range.end, &a.code).cmp(&(&b.range.start, &b.range.end, &b.code))
    });
    findings.dedup_by(|a, b| {
        a.range == b.range && a.code == b.code && a.message == b.message && a.path == b.path
    });
    findings
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
