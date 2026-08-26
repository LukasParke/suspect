//! `RuleHost`: the orchestration layer — start, evaluate a document,
//! enforce policy (disable-on-error, restart-on-timeout), sort findings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use suspect_low::LowDoc;

use crate::protocol::{RuleMeta, TargetKind};
use crate::select::{CompiledSelector, span_at_pointer};
use crate::worker::{TsWorker, WorkerError};
use crate::{
    DEFAULT_TIMEOUT_MS, Error, Result, RulesConfig, discover_rule_files, stage_worker_files,
};

/// A finding projected back to document coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct TsFinding {
    /// Rule that produced it.
    pub rule_id: String,
    /// RFC 6901 pointer into the evaluated document.
    pub pointer: String,
    /// Message text.
    pub message: String,
    /// Severity (rule default when the finding omits it).
    pub severity: Option<String>,
    /// Byte span in the source, resolved host-side from the CST.
    pub span: Option<(usize, usize)>,
    /// Structured fix intent (pass-through).
    pub fix: Option<serde_json::Value>,
}

/// Start configuration.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Workspace root (worker cwd; file-read jail).
    pub workspace_root: PathBuf,
    /// Rule files; empty + no `config.dir` scans `.suspect/rules`.
    pub rule_files: Vec<PathBuf>,
    /// Per-evaluate deadline override.
    pub timeout_ms: Option<u64>,
    /// `auto` | `require` | `off`.
    pub bun: Option<String>,
    /// Cache dir for staged worker files (default: OS temp +
    /// `suspect-rules-worker`).
    pub cache_dir: Option<PathBuf>,
}

/// The host-side rule runtime.
pub struct RuleHost {
    worker: Option<TsWorker>,
    rules: Vec<RuleMeta>,
    selectors: BTreeMap<String, CompiledSelector>,
    disabled: BTreeSet<String>,
    options: StartOptions,
    timeout: Duration,
    bun: PathBuf,
    cache_dir: PathBuf,
    runs: u64,
    /// Worker-reported duration of the last evaluate round.
    last_worker_ms: f64,
}

impl RuleHost {
    /// Discovers rules, stages the worker bundle, spawns bun, and
    /// handshakes. Returns `Ok(None)` when bun mode is `off` or `auto`
    /// without bun on PATH (TS rules silently absent — native rules still
    /// run).
    ///
    /// # Errors
    /// `require` mode with bun missing; spawn/handshake failures; missing
    /// explicitly-named rule files; uncompilable selectors.
    pub async fn start(options: StartOptions) -> Result<Option<Self>> {
        let mode = options.bun.as_deref().unwrap_or("auto");
        if mode == "off" {
            return Ok(None);
        }
        let Some(bun) = crate::discover::find_bun() else {
            if mode == "require" {
                return Err(Error::BunUnavailable(
                    "bun mode is `require` but no bun binary on PATH".to_owned(),
                ));
            }
            return Ok(None);
        };

        let config = RulesConfig {
            rule_files: options.rule_files.clone(),
            dir: None,
            timeout_ms: options.timeout_ms,
            bun: options.bun.clone(),
        };
        let rule_files = discover_rule_files(&options.workspace_root, &config)?;
        if rule_files.is_empty() {
            return Ok(None);
        }

        let cache_dir = options
            .cache_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("suspect-rules-worker"));
        let entry = stage_worker_files(&cache_dir)?;

        let worker = TsWorker::spawn(
            &bun,
            &entry,
            &cache_dir,
            &options.workspace_root,
            &rule_files,
        )
        .await?;

        let mut selectors = BTreeMap::new();
        for rule in &worker.ready.rules {
            if rule.shape == crate::protocol::Shape::Point
                && let Some(given) = &rule.given
            {
                selectors.insert(rule.id.clone(), CompiledSelector::parse(&rule.id, given)?);
            }
        }

        let timeout = Duration::from_millis(options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        let ready = worker.ready.clone();
        Ok(Some(Self {
            worker: Some(worker),
            rules: ready.rules,
            selectors,
            disabled: BTreeSet::new(),
            options,
            timeout,
            bun,
            cache_dir,
            runs: 0,
            last_worker_ms: 0.0,
        }))
    }

    /// Loaded rule metadata.
    #[must_use]
    pub fn rules(&self) -> &[RuleMeta] {
        &self.rules
    }

    /// Rules disabled this session (threw or timed out).
    #[must_use]
    pub fn disabled(&self) -> &BTreeSet<String> {
        &self.disabled
    }

    /// Bun binary in use.
    #[must_use]
    pub fn bun(&self) -> &Path {
        &self.bun
    }

    /// Evaluates all enabled rules against `doc`. Findings are sorted by
    /// `(rule_id, pointer)`; spans resolved from the CST. Rules that threw
    /// are disabled for the session; a timed-out run restarts the worker.
    ///
    /// # Errors
    /// Selector compilation failures; worker restart failures.
    pub async fn evaluate(&mut self, doc: &LowDoc) -> Result<Vec<TsFinding>> {
        let Some(worker) = self.worker.as_mut() else {
            return Ok(Vec::new());
        };
        self.runs += 1;

        let t_start = std::time::Instant::now();
        // Spans captured during selection; findings reuse them instead of
        // re-resolving each pointer against the CST (659 lookups cost
        // ~550ms; a hash hit is O(1)).
        let span_cache: std::collections::HashMap<String, (usize, usize)> =
            std::collections::HashMap::new();
        let doc_json = crate::node_json::doc_to_json_string(&doc.root());
        let document = crate::protocol::RunDocument {
            value: serde_json::value::RawValue::from_string(doc_json)?,
        };
        let t_doc = t_start.elapsed();
        let mut selections: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut walks = BTreeMap::new();
        let t_select_start = std::time::Instant::now();
        for rule in &self.rules {
            if self.disabled.contains(&rule.id) {
                continue;
            }
            match rule.shape {
                crate::protocol::Shape::Point => {
                    let Some(selector) = self.selectors.get(&rule.id) else {
                        continue;
                    };
                    let targets_ok =
                        rule.targets.is_empty() || rule.targets.contains(&TargetKind::Spec);
                    if !targets_ok {
                        continue;
                    }
                    let pointers = selector.select_pointers(doc);
                    selections.insert(rule.id.clone(), pointers);
                }
                crate::protocol::Shape::Walk => {
                    let targets_ok =
                        rule.targets.is_empty() || rule.targets.contains(&TargetKind::Spec);
                    if !targets_ok {
                        continue;
                    }
                    walks.insert(
                        rule.id.clone(),
                        crate::protocol::WalkEntry {
                            root_pointer: String::new(),
                        },
                    );
                }
            }
        }

        if selections.is_empty() && walks.is_empty() {
            return Ok(Vec::new());
        }
        if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
            eprintln!(
                "[host] doc build {t_doc:?}, select {:?}, total prep {:?}",
                t_select_start.elapsed(),
                t_start.elapsed()
            );
        }

        let outcome = match worker
            .evaluate(
                TargetKind::Spec,
                None,
                document,
                selections,
                walks,
                self.timeout,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(WorkerError::Timeout { .. }) => {
                // Kill, restart, re-handshake; the timed-out run's rules
                // stay enabled (a slow doc is not a broken rule) but the
                // run yields nothing.
                self.restart().await?;
                return Ok(Vec::new());
            }
            Err(WorkerError::Fatal(msg)) => {
                // Crash: restart and retry once; if the retry also dies,
                // surface the error.
                self.restart().await?;
                let Some(worker) = self.worker.as_mut() else {
                    return Ok(Vec::new());
                };
                worker
                    .evaluate(
                        TargetKind::Spec,
                        None,
                        crate::protocol::RunDocument {
                            value: serde_json::value::RawValue::from_string("null".to_owned())?,
                        },
                        BTreeMap::new(),
                        BTreeMap::new(),
                        self.timeout,
                    )
                    .await
                    .map_err(|e| Error::WorkerDied(e.to_string()))?;
                return Err(Error::WorkerDied(msg));
            }
        };

        self.last_worker_ms = outcome.done.ms;
        for err in &outcome.rule_errors {
            self.disabled.insert(err.rule_id.clone());
        }

        let t_findings_start = std::time::Instant::now();
        let mut findings: Vec<TsFinding> = outcome
            .findings
            .into_iter()
            .map(|f| {
                let span = span_cache
                    .get(&f.pointer)
                    .copied()
                    .or_else(|| span_at_pointer(doc, &f.pointer));
                TsFinding {
                    span,
                    rule_id: f.rule_id,
                    pointer: f.pointer,
                    message: f.message,
                    severity: f.severity,
                    fix: f.fix,
                }
            })
            .collect();
        if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
            eprintln!("[host] findings map {:?}", t_findings_start.elapsed());
        }
        findings.sort_by(|a, b| {
            (&a.rule_id, &a.pointer, &a.message).cmp(&(&b.rule_id, &b.pointer, &b.message))
        });
        Ok(findings)
    }

    /// Hot-reloads the given rule files (worker re-imports, host
    /// recompiles selectors).
    ///
    /// # Errors
    /// Reload failures from the worker; selector compilation failures.
    pub async fn reload(&mut self, files: &[PathBuf]) -> Result<()> {
        let Some(worker) = self.worker.as_mut() else {
            return Ok(());
        };
        let ready = worker.reload(files).await?;
        self.rules = ready.rules;
        self.selectors.clear();
        self.disabled.clear();
        for rule in &self.rules {
            if rule.shape == crate::protocol::Shape::Point
                && let Some(given) = &rule.given
            {
                self.selectors
                    .insert(rule.id.clone(), CompiledSelector::parse(&rule.id, given)?);
            }
        }
        Ok(())
    }

    async fn restart(&mut self) -> Result<()> {
        if let Some(worker) = self.worker.as_mut() {
            let _ = worker.kill().await;
        }
        let rule_files = discover_rule_files(
            &self.options.workspace_root,
            &RulesConfig {
                rule_files: self.options.rule_files.clone(),
                dir: None,
                timeout_ms: None,
                bun: None,
            },
        )?;
        let entry = stage_worker_files(&self.cache_dir)?;
        let worker = TsWorker::spawn(
            &self.bun,
            &entry,
            &self.cache_dir,
            &self.options.workspace_root,
            &rule_files,
        )
        .await?;
        self.rules = worker.ready.rules.clone();
        self.worker = Some(worker);
        Ok(())
    }

    /// Kills the worker.
    ///
    /// # Errors
    /// Best-effort kill; errors are swallowed upstream of this call.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(worker) = self.worker.as_mut() {
            worker.kill().await?;
        }
        self.worker = None;
        Ok(())
    }

    /// Completed evaluate rounds (observability).
    #[must_use]
    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// Worker-reported duration of the last evaluate round (observability).
    #[must_use]
    pub fn last_worker_ms(&self) -> f64 {
        self.last_worker_ms
    }
}
