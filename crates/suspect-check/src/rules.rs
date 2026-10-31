//! TypeScript rules: the Bun sidecar's findings as [`Finding`]s.
//!
//! The runtime lives in `suspect-rules` ([`RuleHost`]). This module
//! discovers rule files from `rules.roots`, starts the host, maps its
//! findings, and reports the two configuration problems a rule set can
//! have: no Bun to run it ([`RUNTIME_UNAVAILABLE`]) and a rule id that
//! names a builtin code or alias ([`ID_COLLISION`]).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use suspect_diag::{CheckId, Finding, Level, Severity, parse_level};
use suspect_low::LowDoc;
use suspect_rules::{RuleHost, RuleMeta, RulesConfig, StartOptions, TsFinding};
use suspect_source::Uri;

use crate::Registry;

pub use suspect_rules::Error as RulesError;

/// Rule files exist but no Bun runtime is available to run them.
pub const RUNTIME_UNAVAILABLE: &str = "rules-runtime-unavailable";

/// A TS rule id equals a builtin code or alias.
pub const ID_COLLISION: &str = "rules-id-collision";

/// Anything that evaluates TS rules over a document: [`RuleHost`], or a
/// stub in tests.
pub trait RuleSource: Send {
    /// Loaded rule metadata.
    fn rules(&self) -> &[RuleMeta];

    /// Findings for `doc`, in document coordinates.
    fn evaluate<'a>(
        &'a mut self,
        doc: &'a LowDoc,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TsFinding>, RulesError>> + Send + 'a>>;
}

impl RuleSource for RuleHost {
    fn rules(&self) -> &[RuleMeta] {
        Self::rules(self)
    }

    fn evaluate<'a>(
        &'a mut self,
        doc: &'a LowDoc,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TsFinding>, RulesError>> + Send + 'a>> {
        Box::pin(Self::evaluate(self, doc))
    }
}

/// TS rules for one document, before policy. A rule whose id collides with
/// a builtin code ([`collisions`]) is skipped: its findings would be read
/// as the builtin's. A finding with severity `off` is dropped; one with no
/// severity (or a name the parser does not know) is a warning.
///
/// # Errors
/// The worker failed and could not be restarted.
pub async fn check_rules(
    host: &mut dyn RuleSource,
    doc: &LowDoc,
) -> Result<Vec<Finding>, RulesError> {
    let colliding: Vec<String> = collisions(crate::registry::builtin(), host.rules())
        .into_iter()
        .map(|c| c.id)
        .collect();
    let findings = host.evaluate(doc).await?;
    Ok(findings
        .into_iter()
        .filter(|f| !colliding.contains(&f.rule_id))
        .filter_map(|f| to_finding(f, doc.uri()))
        .collect())
}

/// One TS finding as a [`Finding`]; `None` for severity `off`.
#[must_use]
pub fn to_finding(f: TsFinding, document: &Uri) -> Option<Finding> {
    let severity = match f.severity.as_deref().map(parse_level) {
        Some(Some(Level::Off)) => return None,
        Some(Some(Level::On(severity))) => severity,
        _ => Severity::Warning,
    };
    let mut finding = Finding::new(
        CheckId::Rules,
        f.rule_id,
        severity,
        document.clone(),
        f.message,
    );
    finding.range = f.span.map(|(start, end)| start..end);
    finding.pointer = Some(f.pointer.into_boxed_str());
    Some(finding)
}

/// A TS rule id that names a builtin code or alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The rule id.
    pub id: String,
    /// The rule file that declares it.
    pub file: PathBuf,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TypeScript rule `{}` ({}) reuses a builtin code; rename the rule",
            self.id,
            self.file.display()
        )
    }
}

impl Collision {
    /// The collision as an error finding on the rule file.
    #[must_use]
    pub fn finding(&self) -> Option<Finding> {
        let uri = Uri::from_path(&self.file).ok()?;
        Some(Finding::new(
            CheckId::Rules,
            ID_COLLISION,
            Severity::Error,
            uri,
            self.to_string(),
        ))
    }
}

/// Rule ids in `rules` that equal a code or alias `registry` knows. A
/// collision is a configuration error: the CLI exits 2, the editor shows
/// it on the rule file.
#[must_use]
pub fn collisions(registry: &Registry, rules: &[RuleMeta]) -> Vec<Collision> {
    rules
        .iter()
        .filter(|rule| registry.is_builtin_name(&rule.id))
        .map(|rule| Collision {
            id: rule.id.clone(),
            file: PathBuf::from(&rule.file),
        })
        .collect()
}

/// Rule files under every `rules.roots` directory (`*.ts`, `*.js`,
/// `*.mts`, `*.mjs`), sorted and without repeats. Relative roots resolve
/// against `root`.
#[must_use]
pub fn rule_files(root: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = roots
        .iter()
        .flat_map(|dir| {
            let config = RulesConfig {
                dir: Some(root.join(dir)),
                ..RulesConfig::default()
            };
            suspect_rules::discover_rule_files(root, &config).unwrap_or_default()
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// How starting the rule runtime went.
pub enum Startup {
    /// No rule files: nothing to run.
    NoRules,
    /// Rule files exist but Bun is not on `PATH`.
    Unavailable {
        /// The rule files that will not run.
        files: Vec<PathBuf>,
    },
    /// The host is running.
    Started(Box<RuleHost>),
    /// Bun was found but the worker failed to start or load the rules.
    Failed(RulesError),
}

/// Starts the rule host for `root` over the rule files in `roots`.
/// `timeout_ms` is the per-document deadline (the host's default, 250 ms,
/// when `None`).
pub async fn start(root: &Path, roots: &[PathBuf], timeout_ms: Option<u64>) -> Startup {
    let files = rule_files(root, roots);
    if files.is_empty() {
        return Startup::NoRules;
    }
    if suspect_rules::discover::find_bun().is_none() {
        return Startup::Unavailable { files };
    }
    let options = StartOptions {
        workspace_root: root.to_path_buf(),
        rule_files: files,
        timeout_ms,
        bun: Some("auto".to_owned()),
        cache_dir: None,
    };
    match RuleHost::start(options).await {
        Ok(Some(host)) => Startup::Started(Box::new(host)),
        Ok(None) => Startup::NoRules,
        Err(error) => Startup::Failed(error),
    }
}

/// The `rules-runtime-unavailable` warning for `document`: TS rules are
/// configured but cannot run, so the rest of the checks ran without them.
#[must_use]
pub fn runtime_unavailable(document: Uri, files: usize) -> Finding {
    Finding::new(
        CheckId::Rules,
        RUNTIME_UNAVAILABLE,
        Severity::Warning,
        document,
        format!(
            "{files} TypeScript rule file(s) were not run: bun is not on PATH \
             (install from https://bun.sh)"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str) -> RuleMeta {
        RuleMeta {
            id: id.to_owned(),
            file: "/w/.suspect/rules/r.ts".to_owned(),
            targets: Vec::new(),
            given: None,
            shape: suspect_rules::protocol::Shape::Point,
            visitors: Vec::new(),
            uses_zod: false,
        }
    }

    #[test]
    fn ids_naming_builtin_codes_or_aliases_collide() {
        let registry = Registry::builtin();
        let rules = [
            meta("my-rule"),
            meta("oas-tag-undeclared"),
            meta("operation-operationId"),
            meta("syntax-error"),
        ];
        let ids: Vec<String> = collisions(&registry, &rules)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            ids,
            [
                "oas-tag-undeclared",
                "operation-operationId",
                "syntax-error"
            ]
        );
    }

    #[test]
    fn findings_map_severity_span_and_pointer() {
        let uri = Uri::parse("file:///w/api.yaml").unwrap();
        let ts = |severity: Option<&str>| TsFinding {
            rule_id: "my-rule".to_owned(),
            pointer: "/paths/~1a/get".to_owned(),
            message: "m".to_owned(),
            severity: severity.map(str::to_owned),
            span: Some((3, 9)),
            fix: None,
        };
        let f = to_finding(ts(None), &uri).unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.check, CheckId::Rules);
        assert_eq!(f.range, Some(3..9));
        assert_eq!(f.pointer.as_deref(), Some("/paths/~1a/get"));
        assert_eq!(
            to_finding(ts(Some("error")), &uri).unwrap().severity,
            Severity::Error
        );
        assert!(to_finding(ts(Some("off")), &uri).is_none());
    }

    #[test]
    fn rule_files_scan_every_root() {
        let dir = tempfile::tempdir().unwrap();
        for (sub, name) in [("a", "one.ts"), ("b", "two.js"), ("b", "notes.md")] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
            std::fs::write(dir.path().join(sub).join(name), "").unwrap();
        }
        let files = rule_files(
            dir.path(),
            &[PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("a")],
        );
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["one.ts", "two.js"]);
    }
}
