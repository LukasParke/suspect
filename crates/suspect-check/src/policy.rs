//! Policy: which checks run, and what happens to their findings.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use suspect_diag::{CheckId, Finding, Level, RuleClass, Severity, parse_level};
use suspect_low::LowDoc;

use crate::Registry;
use crate::checks::extensions::ExtensionSchemas;

/// A set of [`CheckId`]s (`--only`); the default is every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSet(Option<Vec<CheckId>>);

impl CheckSet {
    /// Every check.
    #[must_use]
    pub const fn all() -> Self {
        Self(None)
    }

    /// Only these checks.
    #[must_use]
    pub fn only(checks: impl IntoIterator<Item = CheckId>) -> Self {
        Self(Some(checks.into_iter().collect()))
    }

    /// Whether `check` runs.
    #[must_use]
    pub fn contains(&self, check: CheckId) -> bool {
        self.0.as_ref().is_none_or(|only| only.contains(&check))
    }
}

impl Default for CheckSet {
    fn default() -> Self {
        Self::all()
    }
}

/// Everything that decides which findings a run reports, and at what
/// severity.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Checks that run (`--only`).
    pub checks: CheckSet,
    /// Findings below this are dropped (`lint.min_severity`).
    pub floor: Severity,
    /// Per-code levels (`lint.rules`), keyed as written.
    pub overrides: BTreeMap<Box<str>, Level>,
    /// Level for [`RuleClass::Design`] findings (`lint.design`).
    pub design: Option<Level>,
    /// `lint.recommended`: with `false` and no custom ruleset, no lint or
    /// markdown rules run.
    pub recommended: bool,
    /// Whether `lint.ruleset` names a ruleset.
    pub custom_ruleset: bool,
    /// Whether SDK-readiness findings ([`RuleClass::Sdk`], admission) are
    /// kept. On until P1 step 9 detects configured codegen.
    pub sdk: bool,
    /// `validate.strict_format`.
    pub strict_format: bool,
    /// The compiled ruleset: the custom one, or the builtin packs.
    pub linter: Arc<suspect_lint::Linter>,
    /// Vendor-extension schemas.
    pub extensions: ExtensionSchemas,
    /// Every known code.
    pub registry: Arc<Registry>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            checks: CheckSet::all(),
            floor: Severity::Hint,
            overrides: BTreeMap::new(),
            design: None,
            recommended: true,
            custom_ruleset: false,
            sdk: true,
            strict_format: false,
            linter: spectral_default(),
            extensions: ExtensionSchemas::default(),
            registry: Arc::clone(crate::registry::builtin_arc()),
        }
    }
}

impl Policy {
    /// The policy resolved settings name. Unparseable severities were
    /// already rejected by `suspect_config::resolve`; any left are ignored.
    ///
    /// A custom ruleset that cannot be read or compiled falls back to the
    /// builtin packs, as the editor always has. Compiled rulesets are
    /// cached by path and file stamp, so building a policy per run does
    /// not recompile an unchanged file.
    #[must_use]
    pub fn from_settings(settings: &suspect_config::Settings) -> Self {
        let lint = &settings.lint;
        let floor = lint
            .min_severity
            .as_deref()
            .and_then(parse_level)
            .and_then(Level::severity)
            .unwrap_or(Severity::Hint);
        let overrides = lint
            .rules
            .iter()
            .filter_map(|(code, name)| Some((code.as_str().into(), parse_level(name)?)))
            .collect();
        let linter = lint
            .ruleset
            .as_deref()
            .and_then(cached_ruleset)
            .unwrap_or_else(spectral_default);
        Self {
            floor,
            overrides,
            design: lint.design.as_deref().and_then(parse_level),
            recommended: lint.recommended,
            custom_ruleset: lint.ruleset.is_some(),
            strict_format: settings.validate.strict_format,
            linter,
            extensions: ExtensionSchemas::from_value(settings.extensions.as_ref()),
            ..Self::default()
        }
    }

    /// Whether lint and markdown rules run at all.
    #[must_use]
    pub const fn lint_enabled(&self) -> bool {
        self.recommended || self.custom_ruleset
    }

    /// The override for a (canonical) code: one keyed on the code itself
    /// wins, else one keyed on any of its aliases.
    fn override_for(&self, code: &str) -> Option<Level> {
        if let Some(level) = self.overrides.get(code) {
            return Some(*level);
        }
        let aliases = self
            .registry
            .info(code)
            .map_or(&[][..], |info| info.aliases);
        aliases
            .iter()
            .find_map(|alias| self.overrides.get(*alias).copied())
    }

    /// The rule class of a finding: a lint rule's from the compiled
    /// ruleset (a ruleset rule extending a builtin keeps its class), any
    /// other code's from the registry.
    fn class_of(&self, finding: &Finding) -> RuleClass {
        let lint_code = self
            .registry
            .info(&finding.code)
            .is_none_or(|info| info.check == CheckId::Lint);
        if finding.check == CheckId::Lint && lint_code {
            return match self.linter.category_of(&finding.code) {
                suspect_lint::Category::Design => RuleClass::Design,
                suspect_lint::Category::Document => RuleClass::Document,
            };
        }
        self.registry
            .info(&finding.code)
            .map_or(RuleClass::Document, |info| info.class)
    }
}

/// The builtin packs, compiled once per process.
fn spectral_default() -> Arc<suspect_lint::Linter> {
    static DEFAULT: OnceLock<Arc<suspect_lint::Linter>> = OnceLock::new();
    Arc::clone(DEFAULT.get_or_init(|| Arc::new(suspect_lint::Linter::spectral_default())))
}

type Stamp = (Option<SystemTime>, u64);

/// Compiled rulesets by path, with the file stamp they were compiled from.
type RulesetCache = Mutex<HashMap<PathBuf, (Stamp, Arc<suspect_lint::Linter>)>>;

/// A compiled custom ruleset, recompiled only when the file's modification
/// time or length changes. `None` when it cannot be read or compiled.
fn cached_ruleset(path: &Path) -> Option<Arc<suspect_lint::Linter>> {
    static CACHE: OnceLock<RulesetCache> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    let stamp = (meta.modified().ok(), meta.len());
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((cached, linter)) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(path)
        && *cached == stamp
    {
        return Some(Arc::clone(linter));
    }
    let bytes = std::fs::read(path).ok()?;
    let uri = suspect_source::Uri::from_path(path).ok()?;
    let doc = LowDoc::parse(uri, suspect_source::Source::from_vec(bytes));
    let linter = Arc::new(suspect_lint::Linter::from_ruleset(&doc).ok()?);
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_path_buf(), (stamp, Arc::clone(&linter)));
    Some(linter)
}

/// Applies `policy` to the findings of one document, in a fixed order:
///
/// 1. **Canonicalize**: a finding from a builtin rule whose code is an
///    alias takes the canonical code and its default severity
///    (`operation-operationId` becomes the warning
///    `oas-operation-missing-operationId`). An alias with several targets
///    takes the one reported at the same position, else keeps its code. A
///    custom ruleset rule reusing a builtin name is a different check and
///    keeps its code.
/// 2. **Dedupe**: findings with the same canonical code, document and
///    position that came from different codes are one problem; the one
///    from the code's owner check is kept, else the first. Repeats of one
///    code (two undeclared parameters on one path item) are all kept.
/// 3. **Overrides**: the level configured for the canonical code, else
///    for one of its aliases, else `lint.design` for a design-class rule,
///    else the finding's own severity; `off` drops the finding.
/// 4. **Category gates**: SDK-readiness findings only with `policy.sdk`;
///    lint and markdown findings only when lint is enabled.
/// 5. **Suppressions**: findings an inline `# suspect-disable…` comment
///    or an `x-suspect-disable` extension covers are set aside (see
///    [`crate::suppress`]); problems with the suppressions themselves are
///    added, subject to overrides.
/// 6. **Floor**: findings below `policy.floor` are dropped, from the
///    suppressed list too.
#[must_use]
pub fn apply_policy(findings: Vec<Finding>, doc: &LowDoc, policy: &Policy) -> Vec<Finding> {
    apply_policy_report(findings, doc, policy).findings
}

/// What [`apply_policy_report`] keeps and what suppressions hid.
#[derive(Debug, Clone, Default)]
pub struct PolicyOutcome {
    /// Findings to report.
    pub findings: Vec<Finding>,
    /// Findings an inline or extension suppression hid (`--format json`
    /// lists them under `suppressed`).
    pub suppressed: Vec<Finding>,
}

/// [`apply_policy`], also returning the suppressed findings.
#[must_use]
pub fn apply_policy_report(findings: Vec<Finding>, doc: &LowDoc, policy: &Policy) -> PolicyOutcome {
    let canonical = canonicalize(findings, policy);
    let gated: Vec<Finding> = dedupe(canonical, &policy.registry)
        .into_iter()
        .filter_map(|mut finding| {
            // 3. Overrides.
            let class = policy.class_of(&finding);
            let level = policy.override_for(&finding.code).or_else(|| {
                (class == RuleClass::Design)
                    .then_some(policy.design)
                    .flatten()
            });
            match level {
                Some(Level::Off) => return None,
                Some(Level::On(severity)) => finding.severity = severity,
                None => {}
            }
            // 4. Category gates.
            let sdk = class == RuleClass::Sdk || finding.check == CheckId::Admission;
            if sdk && !policy.sdk {
                return None;
            }
            if matches!(finding.check, CheckId::Lint | CheckId::Markdown) && !policy.lint_enabled()
            {
                return None;
            }
            Some(finding)
        })
        .collect();
    // 5. Suppressions.
    let custom =
        |code: &str| policy.custom_ruleset && policy.linter.rule_codes().any(|c| c == code);
    let mut directives = crate::suppress::collect(doc, &policy.registry, &custom);
    let (mut kept, suppressed) = crate::suppress::apply(gated, doc, &mut directives);
    // `suppression-unused` is off unless configured.
    let unused = match policy.override_for(crate::suppress::UNUSED) {
        Some(Level::On(severity)) => Some(severity),
        _ => None,
    };
    for mut problem in crate::suppress::problems(doc, &directives, unused) {
        match policy.override_for(&problem.code) {
            Some(Level::Off) => continue,
            Some(Level::On(severity)) => problem.severity = severity,
            None => {}
        }
        kept.push(problem);
    }
    // 6. Floor.
    let floor = |f: &Finding| f.severity >= policy.floor;
    PolicyOutcome {
        findings: kept.into_iter().filter(floor).collect(),
        suppressed: suppressed.into_iter().filter(floor).collect(),
    }
}

/// A finding and the code it was emitted with, when canonicalization
/// renamed it.
struct Canonical {
    finding: Finding,
    renamed_from: Option<Box<str>>,
}

/// Where a finding is, for dedupe: document plus start offset, else the
/// pointer.
type Location = (String, Option<usize>, Option<Box<str>>);

fn location(finding: &Finding) -> Location {
    let start = finding.range.as_ref().map(|r| r.start);
    let pointer = if start.is_none() {
        finding.pointer.clone()
    } else {
        None
    };
    (finding.document.as_str().to_owned(), start, pointer)
}

/// Step 1.
fn canonicalize(findings: Vec<Finding>, policy: &Policy) -> Vec<Canonical> {
    let registry = &policy.registry;
    // Codes present per location, for aliases with several targets.
    let present: std::collections::HashSet<(Location, String)> = findings
        .iter()
        .map(|f| (location(f), f.code.to_string()))
        .collect();
    findings
        .into_iter()
        .map(|mut finding| {
            let targets = registry.alias_targets(&finding.code);
            // Aliases apply to builtin rule definitions only.
            let builtin = finding.check != CheckId::Lint || policy.linter.is_builtin(&finding.code);
            let target = match targets {
                [] => None,
                _ if !builtin => None,
                [one] => Some(*one),
                many => {
                    let at = location(&finding);
                    many.iter()
                        .copied()
                        .find(|code| present.contains(&(at.clone(), (*code).to_owned())))
                }
            };
            let Some(target) = target else {
                return Canonical {
                    finding,
                    renamed_from: None,
                };
            };
            let from = std::mem::replace(&mut finding.code, target.into());
            if let Some(info) = registry.info(target) {
                finding.severity = info.default;
            }
            Canonical {
                finding,
                renamed_from: Some(from.into_owned().into_boxed_str()),
            }
        })
        .collect()
}

/// Step 2.
fn dedupe(findings: Vec<Canonical>, registry: &Registry) -> Vec<Finding> {
    let owner = |c: &Canonical| {
        registry
            .info(&c.finding.code)
            .is_some_and(|info| info.check == c.finding.check)
    };
    let mut kept: Vec<Canonical> = Vec::with_capacity(findings.len());
    // (code, location) -> indices in `kept` with that key.
    let mut by_key: HashMap<(String, Location), Vec<usize>> = HashMap::new();
    for candidate in findings {
        let key = (
            candidate.finding.code.to_string(),
            location(&candidate.finding),
        );
        let slots = by_key.entry(key).or_default();
        // Only findings that reached this code by different routes are the
        // same problem; one code repeated at one spot is not.
        let twin = slots.iter().copied().find(|&i| {
            let k = &kept[i];
            (k.renamed_from.is_some() || candidate.renamed_from.is_some())
                && k.renamed_from != candidate.renamed_from
        });
        match twin {
            Some(i) => {
                if owner(&candidate) && !owner(&kept[i]) {
                    kept[i] = candidate;
                }
            }
            None => {
                slots.push(kept.len());
                kept.push(candidate);
            }
        }
    }
    kept.into_iter().map(|c| c.finding).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    fn doc() -> LowDoc {
        LowDoc::parse(
            Uri::parse("mem://api.yaml").unwrap(),
            Source::from_vec(b"openapi: 3.1.0\n".to_vec()),
        )
    }

    fn finding(check: CheckId, code: &'static str, severity: Severity) -> Finding {
        Finding::new(check, code, severity, doc().uri().clone(), "m").with_range(0..1)
    }

    fn codes(findings: &[Finding]) -> Vec<(String, Severity)> {
        findings
            .iter()
            .map(|f| (f.code.to_string(), f.severity))
            .collect()
    }

    #[test]
    fn an_alias_takes_the_canonical_code_and_severity_and_the_owner_is_kept() {
        let policy = Policy::default();
        // The lint finding comes first; the validate finding owns the code.
        let kept = apply_policy(
            vec![
                finding(CheckId::Lint, "operation-operationId", Severity::Error),
                finding(
                    CheckId::Validate,
                    "oas-operation-missing-operationId",
                    Severity::Warning,
                ),
            ],
            &doc(),
            &policy,
        );
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(kept[0].check, CheckId::Validate);
        assert_eq!(kept[0].severity, Severity::Warning);
        // Alone, the alias survives under the canonical code and severity.
        let kept = apply_policy(
            vec![finding(
                CheckId::Lint,
                "operation-operationId",
                Severity::Error,
            )],
            &doc(),
            &policy,
        );
        assert_eq!(
            codes(&kept),
            vec![(
                "oas-operation-missing-operationId".to_owned(),
                Severity::Warning
            )]
        );
    }

    #[test]
    fn repeats_of_one_code_at_one_spot_are_kept() {
        let kept = apply_policy(
            vec![
                finding(
                    CheckId::Validate,
                    "oas-path-param-not-declared",
                    Severity::Error,
                ),
                finding(
                    CheckId::Validate,
                    "oas-path-param-not-declared",
                    Severity::Error,
                ),
            ],
            &doc(),
            &Policy::default(),
        );
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn findings_at_different_positions_are_not_merged() {
        let kept = apply_policy(
            vec![
                finding(
                    CheckId::Validate,
                    "oas-operation-missing-operationId",
                    Severity::Warning,
                ),
                finding(CheckId::Lint, "operation-operationId", Severity::Error).with_range(5..9),
            ],
            &doc(),
            &Policy::default(),
        );
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn an_override_remaps_or_drops_any_code() {
        let mut policy = Policy::default();
        policy
            .overrides
            .insert("oas-tag-undeclared".into(), Level::On(Severity::Error));
        policy.overrides.insert("md-bare-url".into(), Level::Off);
        let kept = apply_policy(
            vec![
                finding(CheckId::Validate, "oas-tag-undeclared", Severity::Warning),
                finding(CheckId::Markdown, "md-bare-url", Severity::Hint),
            ],
            &doc(),
            &policy,
        );
        assert_eq!(
            codes(&kept),
            vec![("oas-tag-undeclared".to_owned(), Severity::Error)]
        );
    }

    #[test]
    fn a_rule_override_beats_the_design_class() {
        let mut policy = Policy {
            design: Some(Level::Off),
            ..Policy::default()
        };
        let design = "security-no-delete-without-id";
        let kept = apply_policy(
            vec![finding(CheckId::Lint, design, Severity::Warning)],
            &doc(),
            &policy,
        );
        assert!(kept.is_empty(), "lint.design: off drops design rules");
        policy
            .overrides
            .insert(design.into(), Level::On(Severity::Info));
        let kept = apply_policy(
            vec![finding(CheckId::Lint, design, Severity::Warning)],
            &doc(),
            &policy,
        );
        assert_eq!(codes(&kept), vec![(design.to_owned(), Severity::Info)]);
    }

    #[test]
    fn gates_drop_sdk_findings_and_lint_without_rules() {
        let mut policy = Policy {
            sdk: false,
            ..Policy::default()
        };
        let all = || {
            vec![
                finding(
                    CheckId::Validate,
                    "sdk-operation-missing-id",
                    Severity::Info,
                ),
                finding(CheckId::Lint, "info-contact", Severity::Warning),
                finding(CheckId::Markdown, "md-bare-url", Severity::Hint),
            ]
        };
        assert_eq!(apply_policy(all(), &doc(), &policy).len(), 2);
        policy.recommended = false;
        assert!(apply_policy(all(), &doc(), &policy).is_empty());
        policy.custom_ruleset = true;
        assert_eq!(apply_policy(all(), &doc(), &policy).len(), 2);
    }

    #[test]
    fn the_floor_applies_after_overrides() {
        let mut policy = Policy {
            floor: Severity::Warning,
            ..Policy::default()
        };
        policy
            .overrides
            .insert("md-bare-url".into(), Level::On(Severity::Error));
        let kept = apply_policy(
            vec![
                finding(CheckId::Markdown, "md-bare-url", Severity::Hint),
                finding(CheckId::Lint, "info-contact", Severity::Info),
                finding(CheckId::Validate, "oas-tag-undeclared", Severity::Warning),
            ],
            &doc(),
            &policy,
        );
        assert_eq!(
            codes(&kept),
            vec![
                ("md-bare-url".to_owned(), Severity::Error),
                ("oas-tag-undeclared".to_owned(), Severity::Warning),
            ]
        );
    }
}
