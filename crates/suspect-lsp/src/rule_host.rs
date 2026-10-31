//! TypeScript rules in the editor: one lazily started [`RuleHost`] per
//! server, opt-in through `rules.enabled`.
//!
//! The host starts on the first diagnostics run of a document whose
//! settings enable rules, for the workspace root and the workspace's
//! `rules.roots`. Its findings join the builtin findings before
//! `apply_policy`, so overrides, suppressions and the floor apply to them.
//! A timeout or worker error drops the TS part of that run (and is
//! logged); it never holds back the builtin findings. A change under a
//! rules root stops the host, bumps the policy epoch and republishes; the
//! next run starts it again over the current files.
//!
//! [`RuleHost`]: suspect_rules::RuleHost

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use suspect_check::Registry;
use suspect_check::rules::{self, Collision, RuleSource, Startup};
use suspect_diag::{CheckId, Finding};
use suspect_low::LowDoc;

/// Upper bound on one TS run, over the host's own per-document deadline
/// (which restarts a hung worker): a restart that itself hangs must not
/// hold back the builtin findings.
const RUN_LIMIT: Duration = Duration::from_secs(5);

/// Where the server's rule runtime is.
#[derive(Default)]
enum HostState {
    /// Not started (or stopped after a change under a rules root).
    #[default]
    Idle,
    /// No rule files under the roots.
    NoRules,
    /// Rule files exist but bun is not on PATH.
    Unavailable {
        /// How many rule files will not run.
        files: usize,
    },
    /// The worker is running.
    Running(Box<dyn RuleSource>),
    /// Bun was found but the worker did not start; not retried until the
    /// next change under a rules root.
    Failed,
}

/// The server's one rule runtime and the registry its rule ids extend.
#[derive(Default)]
pub struct RuleSlot {
    /// Root and rules roots the current state was started for.
    key: Option<(PathBuf, Vec<PathBuf>)>,
    state: HostState,
    /// The builtin registry plus the running rules' ids.
    registry: Option<Arc<Registry>>,
}

impl RuleSlot {
    /// A slot already running `source` for `root` and `roots` (tests use a
    /// stub source).
    #[must_use]
    pub fn running(source: Box<dyn RuleSource>, root: PathBuf, roots: Vec<PathBuf>) -> Self {
        let mut slot = Self {
            key: Some((root, roots)),
            ..Self::default()
        };
        slot.install(source);
        slot
    }

    fn install(&mut self, source: Box<dyn RuleSource>) -> Vec<Collision> {
        let builtin = suspect_check::registry::builtin();
        let collisions = rules::collisions(builtin, source.rules());
        let ids = source
            .rules()
            .iter()
            .filter(|rule| !builtin.is_builtin_name(&rule.id))
            .map(|rule| (rule.id.clone(), CheckId::Rules));
        self.registry = Registry::builtin().with_dynamic(ids).ok().map(Arc::new);
        self.state = HostState::Running(source);
        collisions
    }

    /// Stops the worker (dropping the host kills its process); the next
    /// run starts it again.
    pub fn stop(&mut self) {
        self.state = HostState::Idle;
        self.key = None;
        self.registry = None;
    }

    /// Whether `path` lies under one of the rules roots the slot was
    /// started for (or under the default root before any start).
    #[must_use]
    pub fn watches(&self, path: &Path, fallback_roots: &[PathBuf]) -> bool {
        let roots = self
            .key
            .as_ref()
            .map_or(fallback_roots, |(_, roots)| roots.as_slice());
        roots.iter().any(|root| path.starts_with(root))
    }
}

/// What one TS run produced for a document.
#[derive(Default)]
pub struct RulesRun {
    /// Findings to merge before `apply_policy`.
    pub findings: Vec<Finding>,
    /// The registry with the running rules' ids, for the policy.
    pub registry: Option<Arc<Registry>>,
    /// Collisions found when the host started this run: shown once, on
    /// the rule files.
    pub collisions: Vec<Collision>,
    /// Something to log (a start failure, a worker error, a timeout).
    pub log: Option<String>,
}

/// TS findings for `doc` when `settings` enable rules, starting the host
/// for `root` and `roots` on first use (or when either changed).
pub async fn run(
    slot: &tokio::sync::Mutex<RuleSlot>,
    root: Option<&Path>,
    roots: &[PathBuf],
    settings: &suspect_config::Settings,
    doc: &LowDoc,
) -> RulesRun {
    let mut out = RulesRun::default();
    if !settings.rules.enabled {
        return out;
    }
    let Some(root) = root else {
        return out;
    };
    let mut slot = slot.lock().await;
    let key = (root.to_path_buf(), roots.to_vec());
    if slot.key.as_ref() != Some(&key) {
        slot.stop();
        slot.key = Some(key);
        match rules::start(root, roots, None).await {
            Startup::NoRules => slot.state = HostState::NoRules,
            Startup::Unavailable { files } => {
                slot.state = HostState::Unavailable { files: files.len() };
            }
            Startup::Started(host) => out.collisions = slot.install(host),
            Startup::Failed(error) => {
                out.log = Some(format!("TypeScript rules did not start: {error}"));
                slot.state = HostState::Failed;
            }
        }
    }
    out.registry.clone_from(&slot.registry);
    match &mut slot.state {
        HostState::Idle | HostState::NoRules | HostState::Failed => {}
        HostState::Unavailable { files } => {
            out.findings
                .push(rules::runtime_unavailable(doc.uri().clone(), *files));
        }
        HostState::Running(source) => {
            match tokio::time::timeout(RUN_LIMIT, suspect_check::check_rules(source.as_mut(), doc))
                .await
            {
                Ok(Ok(findings)) => out.findings = findings,
                Ok(Err(error)) => {
                    out.log = Some(format!("TypeScript rules failed on {}: {error}", doc.uri()));
                }
                Err(_) => {
                    out.log = Some(format!("TypeScript rules timed out on {}", doc.uri()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use suspect_rules::{RuleMeta, TsFinding};

    /// A rule source that reports one finding per run, or fails.
    struct Stub {
        rules: Vec<RuleMeta>,
        fail: bool,
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl RuleSource for Stub {
        fn rules(&self) -> &[RuleMeta] {
            &self.rules
        }

        fn evaluate<'a>(
            &'a mut self,
            _doc: &'a LowDoc,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<TsFinding>, rules::RulesError>> + Send + 'a>>
        {
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(rules::RulesError::WorkerDied("stub".to_owned()));
                }
                Ok(vec![TsFinding {
                    rule_id: "team-summary".to_owned(),
                    pointer: "/info".to_owned(),
                    message: "stub finding".to_owned(),
                    severity: Some("error".to_owned()),
                    span: Some((15, 19)),
                    fix: None,
                }])
            })
        }
    }

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

    fn stub(fail: bool) -> (Box<dyn RuleSource>, Arc<std::sync::atomic::AtomicUsize>) {
        let runs = Arc::default();
        let source = Stub {
            rules: vec![meta("team-summary"), meta("oas-tag-undeclared")],
            fail,
            runs: Arc::clone(&runs),
        };
        (Box::new(source), runs)
    }

    fn doc() -> LowDoc {
        LowDoc::parse(
            suspect_source::Uri::parse("file:///w/api.yaml").unwrap(),
            suspect_source::Source::from_vec(
                b"openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\npaths: {}\n".to_vec(),
            ),
        )
    }

    fn enabled() -> suspect_config::Settings {
        let mut settings = suspect_config::Settings::default();
        settings.rules.enabled = true;
        settings
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/w/.suspect/rules")]
    }

    #[tokio::test]
    async fn stub_findings_reach_the_published_diagnostics() {
        let (source, runs) = stub(false);
        let slot = tokio::sync::Mutex::new(RuleSlot::running(source, "/w".into(), roots()));
        let low = doc();
        let run = run(&slot, Some(Path::new("/w")), &roots(), &enabled(), &low).await;
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(run.log.is_none());
        assert_eq!(run.findings.len(), 1);
        let registry = run.registry.clone().expect("registry with rule ids");
        assert!(registry.is_known("team-summary"));

        let mut policy = suspect_check::Policy::from_settings(&enabled());
        policy.registry = registry;
        let diags =
            crate::diagnostics::diagnostics_under(None, &low, &policy, run.findings.clone());
        let d = diags
            .iter()
            .find(|d| {
                d.code
                    == Some(tower_lsp::lsp_types::NumberOrString::String(
                        "team-summary".to_owned(),
                    ))
            })
            .expect("TS diagnostic");
        assert_eq!(d.source.as_deref(), Some(crate::diagnostics::SOURCE_LINT));
        assert_eq!(
            d.severity,
            Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)
        );

        // Policy applies to TS ids like any other code.
        let mut off = enabled();
        off.lint
            .rules
            .insert("team-summary".to_owned(), "off".to_owned());
        let policy = suspect_check::Policy::from_settings(&off);
        let diags = crate::diagnostics::diagnostics_under(None, &low, &policy, run.findings);
        assert!(diags.iter().all(|d| d.message != "stub finding"));
    }

    #[tokio::test]
    async fn rules_stay_off_unless_enabled() {
        let (source, runs) = stub(false);
        let slot = tokio::sync::Mutex::new(RuleSlot::running(source, "/w".into(), roots()));
        let run = run(
            &slot,
            Some(Path::new("/w")),
            &roots(),
            &suspect_config::Settings::default(),
            &doc(),
        )
        .await;
        assert!(run.findings.is_empty());
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_worker_error_drops_the_ts_part_and_logs() {
        let (source, _) = stub(true);
        let slot = tokio::sync::Mutex::new(RuleSlot::running(source, "/w".into(), roots()));
        let run = run(&slot, Some(Path::new("/w")), &roots(), &enabled(), &doc()).await;
        assert!(run.findings.is_empty());
        assert!(run.log.unwrap().contains("stub"));
    }

    #[tokio::test]
    async fn a_new_root_restarts_and_finds_no_rules() {
        let (source, runs) = stub(false);
        let slot = tokio::sync::Mutex::new(RuleSlot::running(source, "/w".into(), roots()));
        let empty = tempfile::tempdir().unwrap();
        let roots = vec![empty.path().join(".suspect/rules")];
        let run = run(&slot, Some(empty.path()), &roots, &enabled(), &doc()).await;
        assert!(run.findings.is_empty());
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(matches!(slot.lock().await.state, HostState::NoRules));
    }

    #[test]
    fn the_slot_watches_its_roots() {
        let (source, _) = stub(false);
        let slot = RuleSlot::running(source, "/w".into(), roots());
        assert!(slot.watches(Path::new("/w/.suspect/rules/a.ts"), &[]));
        assert!(!slot.watches(Path::new("/w/api.yaml"), &[]));
        let idle = RuleSlot::default();
        assert!(idle.watches(Path::new("/x/r/a.ts"), &[PathBuf::from("/x/r")]));
    }
}
