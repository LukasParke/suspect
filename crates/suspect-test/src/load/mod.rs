//! Artillery-style load generation over Arazzo workflows.
//!
//! A [`LoadProfile`] names weighted scenarios (Arazzo workflows), phased
//! arrival rates, data sources and SLOs. [`run_load`] drives it: hook and
//! prerequisite workflows run once, then virtual users arrive on schedule,
//! each running its scenario's workflow over the shared [`Plan`] while a
//! single aggregator task folds step samples into interval and cumulative
//! [`MetricsSnapshot`]s. The run ends with `ensure` evaluation and a
//! [`LoadSummary`] that reports render.
//!
//! ```text
//! LoadProfile ─▶ phases ─arrival─▶ spawn run_vu ─▶ run_workflow ─▶ VuRecorder
//!                                                                      │ Event
//!                                     aggregator ◀──────────────────────┘
//!                                        │ interval snapshots ─▶ LoadSink
//!                                        └ cumulative ─▶ ensure ─▶ LoadSummary
//! ```

pub mod data;
pub mod ensure;
pub mod metrics;
pub mod profile;
pub mod report;
pub mod schedule;
pub mod vu;

/// Claims one iteration from a shared closed-loop budget, or returns
/// `false` once it is exhausted. A compare-exchange loop rather than
/// `fetch_update`, which newer toolchains deprecate in favour of
/// `try_update` — unavailable at the workspace MSRV.
fn take_iteration(budget: &AtomicU64) -> bool {
    let mut current = budget.load(Ordering::Relaxed);
    while let Some(next) = current.checked_sub(1) {
        match budget.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
    false
}
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

pub use ensure::{CheckResult, EnsureReport};
pub use metrics::{MetricsSnapshot, Quantiles};
pub use profile::{
    LoadProfile, Phase, PhaseKind, ProfileDiagnostic, ProfileError, Scenario, read_profile,
    read_profile_lenient,
};
pub use schedule::{Arrivals, Jitter};
pub use vu::HttpFactory;

use crate::exec::{HttpClient, InputMap, NullObserver, RunAuth, RunOpts, VuId, run_workflow};
use crate::plan::{Plan, dependency_waves};
use metrics::{Event, StepNames, Window};
use vu::{CompiledScenario, NameTable, Shared, VuAbort};

/// Run-time configuration that is not part of the profile.
pub struct LoadConfig {
    /// Target base URL (profile `target` or `--base-url`).
    pub base_url: String,
    /// Shared transport.
    pub http: Arc<dyn HttpClient>,
    /// Per-user transport factory (`per_vu_connections`).
    pub http_factory: Option<HttpFactory>,
    /// Snapshot cadence.
    pub report_interval: Duration,
    /// How long to wait for in-flight users after the last phase.
    pub grace: Duration,
    /// RNG seed.
    pub seed: u64,
    /// Environment label.
    pub env: String,
    /// Inter-arrival jitter.
    pub jitter: Jitter,
    /// Flips to `true` to stop arrivals and drain.
    pub cancel: Option<watch::Receiver<bool>>,
    /// Credentials injected per the operations' security requirements;
    /// one token cache is shared by hooks and every virtual user.
    pub auth: Arc<RunAuth>,
    /// Validate every response body against its declared schema (a
    /// conformance failure counts as a contract failure). Costs a schema
    /// compile per response on the generator, so it is opt-in.
    pub validate_responses: bool,
}

/// Receives progress during a run.
pub trait LoadSink: Send + Sync {
    /// A phase began.
    fn phase_started(&self, _index: usize, _phase: &Phase) {}
    /// An interval elapsed.
    fn interval(&self, _report: &IntervalReport) {}
    /// Free-form note (hook outcomes, warnings).
    fn note(&self, _message: &str) {}
}

/// Sink that ignores everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct SilentSink;

impl LoadSink for SilentSink {}

/// One reporting interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalReport {
    /// Zero-based interval index.
    pub index: usize,
    /// Phase label active when the interval closed.
    pub phase: String,
    /// Seconds since the run started.
    pub at_s: f64,
    /// Metrics for this interval only.
    pub interval: MetricsSnapshot,
    /// Metrics since the run started.
    pub cumulative: MetricsSnapshot,
}

/// Outcome of a hook or prerequisite workflow run once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookOutcome {
    /// Workflow id.
    pub workflow: String,
    /// `before`, `after` or `dependsOn`.
    pub role: String,
    /// Whether it passed.
    pub ok: bool,
    /// Steps passed.
    pub passed: usize,
    /// Steps failed.
    pub failed: usize,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// A phase as executed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseSummary {
    /// Label.
    pub label: String,
    /// Kind rendering (`arrivalRate 10→50/s`, `vus 20`, ...).
    pub kind: String,
    /// Planned length in seconds.
    pub planned_s: f64,
    /// Offset from run start in seconds.
    pub started_at_s: f64,
    /// Arrivals the schedule intended (open-loop phases).
    pub expected_arrivals: u64,
}

/// Scenario table entry for reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioInfo {
    /// Report name.
    pub name: String,
    /// Workflow id.
    pub workflow: String,
    /// Weight.
    pub weight: f64,
    /// Share of users in percent.
    pub share_pct: f64,
}

/// Everything a finished run knows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSummary {
    /// Unix epoch milliseconds at start.
    pub started_at_ms: u64,
    /// Wall-clock length in seconds (hooks included).
    pub duration_s: f64,
    /// Whether arrivals were cut short by cancellation.
    pub interrupted: bool,
    /// Target base URL.
    pub target: String,
    /// Seed used.
    pub seed: u64,
    /// Scenario table.
    pub scenarios: Vec<ScenarioInfo>,
    /// Phases as executed.
    pub phases: Vec<PhaseSummary>,
    /// Hook outcomes.
    pub hooks: Vec<HookOutcome>,
    /// Interval reports.
    pub intervals: Vec<IntervalReport>,
    /// Cumulative metrics.
    pub aggregate: MetricsSnapshot,
    /// SLO results.
    pub ensure: EnsureReport,
}

/// Fatal run failure (configuration, hook failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError(
    /// Description.
    pub String,
);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

/// Resolves the profile against the plan without sending anything.
/// Returned scenarios are in profile order.
///
/// # Errors
/// Returns the first resolution failure (unknown workflow, data file).
pub fn compile_scenarios(
    profile: &LoadProfile,
    plan: &Plan,
) -> Result<Vec<CompiledScenario>, LoadError> {
    if profile.scenarios.is_empty() {
        return Err(LoadError("profile declares no scenarios".to_owned()));
    }
    profile
        .scenarios
        .iter()
        .map(|s| CompiledScenario::compile(s, plan).map_err(LoadError))
        .collect()
}

/// Workflows that must run once before users arrive: the `before` hook
/// plus every `dependsOn` prerequisite of the scenario workflows, in
/// dependency order. Scenario workflows themselves are excluded.
#[must_use]
pub fn prerequisite_workflows(profile: &LoadProfile, plan: &Plan) -> Vec<usize> {
    let scenario_ids: BTreeSet<&str> = profile
        .scenarios
        .iter()
        .map(|s| s.workflow.as_str())
        .collect();
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = profile.before.iter().cloned().collect();
    for s in &profile.scenarios {
        if let Some(wf) = plan.workflows.iter().find(|w| w.workflow_id == s.workflow) {
            stack.extend(wf.depends_on.iter().cloned());
        }
    }
    while let Some(id) = stack.pop() {
        if !wanted.insert(id.clone()) {
            continue;
        }
        if let Some(wf) = plan.workflows.iter().find(|w| w.workflow_id == id) {
            stack.extend(wf.depends_on.iter().cloned());
        }
    }
    let waves = dependency_waves(&plan.workflows).unwrap_or_default();
    let mut order = Vec::new();
    for wave in waves {
        for idx in wave {
            let id = plan.workflows[idx].workflow_id.as_str();
            if wanted.contains(id)
                && (!scenario_ids.contains(id) || profile.before.as_deref() == Some(id))
            {
                order.push(idx);
            }
        }
    }
    order
}

/// Runs a load profile to completion.
///
/// # Errors
/// Returns a [`LoadError`] when scenarios cannot be resolved or a
/// prerequisite/`before` workflow fails (nothing is load-tested in that
/// case). SLO breaches are *not* errors: read [`LoadSummary::ensure`].
pub async fn run_load(
    plan: Arc<Plan>,
    profile: &LoadProfile,
    cfg: LoadConfig,
    sink: Arc<dyn LoadSink>,
) -> Result<LoadSummary, LoadError> {
    let run_start = Instant::now();
    let started_at_ms = suspect_journal::Journal::now_ms();
    let scenarios = compile_scenarios(profile, &plan)?;
    let base_url: Arc<str> = Arc::from(cfg.base_url.trim_end_matches('/'));
    let mut hooks = Vec::new();

    // Hooks and prerequisites, once, sequentially in dependency order.
    let mut workflows_outputs = InputMap::new();
    for idx in prerequisite_workflows(profile, &plan) {
        let wf = &plan.workflows[idx];
        let role = if profile.before.as_deref() == Some(wf.workflow_id.as_str()) {
            "before"
        } else {
            "dependsOn"
        };
        let opts = RunOpts {
            base_url: Arc::clone(&base_url),
            workflows_outputs: Arc::new(workflows_outputs.clone()),
            seed: cfg.seed,
            auth: Arc::clone(&cfg.auth),
            validate_responses: cfg.validate_responses,
            ..RunOpts::default()
        };
        let outcome = run_workflow(&plan, idx, &opts, cfg.http.as_ref(), &NullObserver).await;
        hooks.push(HookOutcome {
            workflow: wf.workflow_id.clone(),
            role: role.to_owned(),
            ok: outcome.ok,
            passed: outcome.passed,
            failed: outcome.failed,
            duration_ms: u64::try_from(outcome.duration.as_millis()).unwrap_or(u64::MAX),
        });
        sink.note(&format!(
            "{role} workflow `{}` {} ({} ms)",
            wf.workflow_id,
            if outcome.ok { "passed" } else { "FAILED" },
            outcome.duration.as_millis()
        ));
        if !outcome.ok {
            return Err(LoadError(format!(
                "{role} workflow `{}` failed; aborting before any load",
                wf.workflow_id
            )));
        }
        workflows_outputs.insert(
            wf.workflow_id.clone(),
            crate::exec::workflow_outputs_value(&outcome),
        );
    }

    let (tx, rx) = mpsc::channel::<Event>(65_536);
    let shared = Arc::new(Shared {
        plan: Arc::clone(&plan),
        scenarios,
        base_url: Arc::clone(&base_url),
        workflows_outputs: Arc::new(workflows_outputs),
        http: Arc::clone(&cfg.http),
        http_factory: cfg.http_factory.clone(),
        tx,
        dropped: AtomicU64::new(0),
        seed: cfg.seed,
        run_id: format!("{started_at_ms:x}-{:04x}", cfg.seed & 0xffff),
        env: cfg.env.clone(),
        active: AtomicUsize::new(0),
        auth: Arc::clone(&cfg.auth),
        validate_responses: cfg.validate_responses,
    });

    let names: Arc<dyn StepNames> = Arc::new(NameTable::new(Arc::clone(&plan), &shared.scenarios));
    let (phase_tx, phase_rx) = watch::channel(String::from("starting"));
    let aggregator = tokio::spawn(aggregate(
        rx,
        cfg.report_interval,
        Arc::clone(&names),
        Arc::clone(&sink),
        phase_rx,
        run_start,
    ));

    let load_start = Instant::now();
    let mut users = JoinSet::new();
    let mut next_vu: u64 = 1;
    let mut rng = rand::rngs::StdRng::seed_from_u64(cfg.seed);
    let mut phases = Vec::new();
    let mut interrupted = false;
    let mut cancel = cfg.cancel.clone();

    'phases: for (index, phase) in profile.phases.iter().enumerate() {
        sink.phase_started(index, phase);
        let _ = phase_tx.send(phase.label());
        let phase_start = Instant::now();
        let arrivals = Arrivals::new(phase, cfg.jitter, cfg.seed ^ index as u64);
        phases.push(PhaseSummary {
            label: phase.label(),
            kind: describe_kind(&phase.kind),
            planned_s: phase.duration.as_secs_f64(),
            started_at_s: load_start.elapsed().as_secs_f64(),
            expected_arrivals: arrivals.expected_total(),
        });

        match &phase.kind {
            PhaseKind::Pause => {
                if !sleep_or_cancel(phase_start + phase.duration, &mut cancel).await {
                    interrupted = true;
                    break 'phases;
                }
            }
            PhaseKind::ArrivalRate { .. } | PhaseKind::ArrivalCount { .. } => {
                for offset in arrivals {
                    if !sleep_or_cancel(phase_start + offset, &mut cancel).await {
                        interrupted = true;
                        break 'phases;
                    }
                    if phase
                        .max_vusers
                        .is_some_and(|max| shared.active.load(Ordering::Relaxed) >= max)
                    {
                        shared.emit(Event::VuSkipped);
                        continue;
                    }
                    let scenario = shared.pick_scenario(&mut rng);
                    let vu = VuId(next_vu);
                    next_vu += 1;
                    users.spawn(run_vu_task(Arc::clone(&shared), vu, scenario));
                    reap_finished(&mut users, &sink);
                }
                // Let the phase run out its clock even if the last arrival
                // was early (e.g. arrivalCount spacing).
                if !sleep_or_cancel(phase_start + phase.duration, &mut cancel).await {
                    interrupted = true;
                    break 'phases;
                }
            }
            PhaseKind::ConstantVus { vus, iterations } => {
                let (stop_tx, stop_rx) = watch::channel(false);
                let budget = iterations.map(|n| Arc::new(AtomicU64::new(n)));
                let count = (*vus).min(phase.max_vusers.unwrap_or(usize::MAX)).max(1);
                // Loopers report when they exit so an iteration-bounded
                // phase can end before its clock does.
                let (alive_tx, mut alive_rx) = watch::channel(count);
                let alive_tx = Arc::new(alive_tx);
                for _ in 0..count {
                    let shared = Arc::clone(&shared);
                    let mut stop = stop_rx.clone();
                    let budget = budget.clone();
                    let alive = Arc::clone(&alive_tx);
                    let mut vu_rng = rand::rngs::StdRng::seed_from_u64(rng.r#gen());
                    let first_vu = next_vu;
                    next_vu += 1;
                    users.spawn(async move {
                        let mut vu = first_vu;
                        loop {
                            if *stop.borrow() {
                                break;
                            }
                            if let Some(b) = &budget
                                && !take_iteration(b)
                            {
                                break;
                            }
                            let scenario = shared.pick_scenario(&mut vu_rng);
                            let _ = run_vu_task(Arc::clone(&shared), VuId(vu), scenario).await;
                            // Closed-loop users get a fresh id per loop so
                            // seeded jitter/data differ between iterations.
                            vu = vu.wrapping_add(1_000_003);
                            if stop.has_changed().unwrap_or(true) && *stop.borrow_and_update() {
                                break;
                            }
                        }
                        alive.send_modify(|n| *n = n.saturating_sub(1));
                        Ok(true)
                    });
                }
                let deadline = phase_start + phase.duration;
                let completed = loop {
                    if *alive_rx.borrow_and_update() == 0 {
                        break true;
                    }
                    let mut cancel_rx = cancel.clone();
                    tokio::select! {
                        done = sleep_or_cancel(deadline, &mut cancel_rx) => break done,
                        changed = alive_rx.changed() => {
                            if changed.is_err() {
                                break true;
                            }
                        }
                    }
                };
                let _ = stop_tx.send(true);
                if !completed {
                    interrupted = true;
                    break 'phases;
                }
            }
        }
    }

    // Drain in-flight users up to the grace period, then abort the rest.
    let _ = phase_tx.send("draining".to_owned());
    let deadline = Instant::now() + cfg.grace;
    let mut aborted: u64 = 0;
    loop {
        if users.is_empty() {
            break;
        }
        tokio::select! {
            res = users.join_next() => {
                if let Some(Ok(Err(reason))) = res {
                    note_abort(&sink, &reason);
                }
            }
            () = tokio::time::sleep_until(deadline) => {
                aborted = users.len() as u64;
                users.abort_all();
                while users.join_next().await.is_some() {}
                break;
            }
        }
    }
    if aborted > 0 {
        shared.emit(Event::VuAborted { count: aborted });
        sink.note(&format!(
            "{aborted} user(s) still running after the {}s grace period were aborted",
            cfg.grace.as_secs()
        ));
    }
    let dropped = shared.dropped.load(Ordering::Relaxed);
    if dropped > 0 {
        let _ = shared.tx.send(Event::Dropped { count: dropped }).await;
    }

    // `after` hook.
    if let Some(after) = &profile.after
        && let Some(idx) = plan.workflows.iter().position(|w| &w.workflow_id == after)
    {
        let opts = RunOpts {
            base_url: Arc::clone(&base_url),
            workflows_outputs: Arc::clone(&shared.workflows_outputs),
            seed: cfg.seed,
            auth: Arc::clone(&cfg.auth),
            validate_responses: cfg.validate_responses,
            ..RunOpts::default()
        };
        let outcome = run_workflow(&plan, idx, &opts, cfg.http.as_ref(), &NullObserver).await;
        sink.note(&format!(
            "after workflow `{after}` {} ({} ms)",
            if outcome.ok { "passed" } else { "FAILED" },
            outcome.duration.as_millis()
        ));
        hooks.push(HookOutcome {
            workflow: after.clone(),
            role: "after".to_owned(),
            ok: outcome.ok,
            passed: outcome.passed,
            failed: outcome.failed,
            duration_ms: u64::try_from(outcome.duration.as_millis()).unwrap_or(u64::MAX),
        });
    }

    let total_weight: f64 = shared.scenarios.iter().map(|s| s.weight).sum();
    let scenario_infos: Vec<ScenarioInfo> = shared
        .scenarios
        .iter()
        .map(|s| ScenarioInfo {
            name: s.name.clone(),
            workflow: plan.workflows[s.wf_index].workflow_id.clone(),
            weight: s.weight,
            share_pct: s.weight * 100.0 / total_weight,
        })
        .collect();
    // Every user is gone; dropping our `Shared` drops the last sender, the
    // channel closes, and the aggregator hands back its windows.
    drop(shared);
    let (window, intervals) = aggregator
        .await
        .map_err(|e| LoadError(format!("aggregator task failed: {e}")))?;
    let cumulative = window.snapshot(load_start.elapsed(), names.as_ref());

    let ensure = ensure::evaluate(&profile.ensure, &cumulative);
    Ok(LoadSummary {
        started_at_ms,
        duration_s: run_start.elapsed().as_secs_f64(),
        interrupted,
        target: base_url.to_string(),
        seed: cfg.seed,
        scenarios: scenario_infos,
        phases,
        hooks,
        intervals,
        aggregate: cumulative,
        ensure,
    })
}

async fn run_vu_task(shared: Arc<Shared>, vu: VuId, scenario: usize) -> Result<bool, VuAbort> {
    vu::run_vu(shared, vu, scenario).await
}

fn reap_finished(users: &mut JoinSet<Result<bool, VuAbort>>, sink: &Arc<dyn LoadSink>) {
    while let Some(res) = users.try_join_next() {
        if let Ok(Err(reason)) = res {
            note_abort(sink, &reason);
        }
    }
}

fn note_abort(sink: &Arc<dyn LoadSink>, reason: &VuAbort) {
    match reason {
        VuAbort::DataExhausted => sink.note("user skipped: unique data rows exhausted"),
        VuAbort::Transport(e) => sink.note(&format!("user skipped: transport setup failed: {e}")),
    }
}

/// Sleeps until `deadline`; returns `false` when cancelled first.
async fn sleep_or_cancel(deadline: Instant, cancel: &mut Option<watch::Receiver<bool>>) -> bool {
    match cancel {
        None => {
            tokio::time::sleep_until(deadline).await;
            true
        }
        Some(rx) => {
            if *rx.borrow() {
                return false;
            }
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => true,
                changed = rx.changed() => !(changed.is_ok() && *rx.borrow()),
            }
        }
    }
}

fn describe_kind(kind: &PhaseKind) -> String {
    match kind {
        PhaseKind::ArrivalRate {
            rate,
            ramp_to: None,
        } => format!("arrivalRate {rate}/s"),
        PhaseKind::ArrivalRate {
            rate,
            ramp_to: Some(to),
        } => format!("arrivalRate {rate}→{to}/s"),
        PhaseKind::ArrivalCount { count } => format!("arrivalCount {count}"),
        PhaseKind::Pause => "pause".to_owned(),
        PhaseKind::ConstantVus {
            vus,
            iterations: None,
        } => format!("vus {vus}"),
        PhaseKind::ConstantVus {
            vus,
            iterations: Some(n),
        } => format!("vus {vus} × {n} iterations"),
    }
}

/// Folds events into cumulative + interval windows until the channel
/// closes; emits an [`IntervalReport`] every `interval`.
async fn aggregate(
    mut rx: mpsc::Receiver<Event>,
    interval: Duration,
    names: Arc<dyn StepNames>,
    sink: Arc<dyn LoadSink>,
    phase: watch::Receiver<String>,
    run_start: Instant,
) -> (Window, Vec<IntervalReport>) {
    let mut cumulative = Window::new();
    let mut current = Window::new();
    let mut reports = Vec::new();
    let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut window_start = Instant::now();
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(ev) => {
                        cumulative.record(&ev);
                        current.record(&ev);
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {
                let now = Instant::now();
                let report = IntervalReport {
                    index: reports.len(),
                    phase: phase.borrow().clone(),
                    at_s: (now - run_start).as_secs_f64(),
                    interval: current.snapshot(now - window_start, names.as_ref()),
                    cumulative: cumulative.snapshot(now - run_start, names.as_ref()),
                };
                sink.interval(&report);
                reports.push(report);
                current = Window::new();
                window_start = now;
            }
        }
    }
    (cumulative, reports)
}
