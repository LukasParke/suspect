//! One virtual user: pick inputs, run the scenario's workflow `iterations`
//! times over a shared plan, and stream samples to the aggregator.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::time::Instant;

use async_trait::async_trait;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;

use super::data::DataFeeder;
use super::metrics::{Event, ScenarioIdx, StepNames};
use super::profile::{InputSource, Scenario};
use crate::exec::{
    ExecObserver, HttpClient, InputMap, RunAuth, RunOpts, StepCx, StepSample, ThinkPolicy, VuId,
    run_workflow,
};
use crate::plan::{Plan, ThinkSpec};

/// Builds a fresh transport per user when `per_vu_connections` is on.
pub type HttpFactory = Arc<dyn Fn() -> Result<Arc<dyn HttpClient>, String> + Send + Sync>;

/// A scenario resolved against the plan.
pub struct CompiledScenario {
    /// Report name.
    pub name: String,
    /// Index into [`Plan::workflows`].
    pub wf_index: usize,
    /// Selection weight.
    pub weight: f64,
    /// Default think time between steps.
    pub think: Option<ThinkSpec>,
    /// Per-user input sources.
    pub inputs: Vec<(String, InputSource)>,
    /// Row feeder when the scenario declares `data`.
    pub data: Option<DataFeeder>,
    /// Workflow repetitions per user.
    pub iterations: u64,
    /// Keep iterating after a failed iteration.
    pub continue_on_failure: bool,
}

impl CompiledScenario {
    /// Resolves `scenario` against `plan`.
    ///
    /// # Errors
    /// Returns a message when the workflow is unknown or the data file
    /// cannot be loaded.
    pub fn compile(scenario: &Scenario, plan: &Plan) -> Result<Self, String> {
        let wf_index = plan
            .workflows
            .iter()
            .position(|w| w.workflow_id == scenario.workflow)
            .ok_or_else(|| {
                format!(
                    "scenario `{}`: workflow `{}` is not in the Arazzo document",
                    scenario.name, scenario.workflow
                )
            })?;
        let data = match &scenario.data {
            Some(src) => Some(
                DataFeeder::load(src).map_err(|e| format!("scenario `{}`: {e}", scenario.name))?,
            ),
            None => None,
        };
        Ok(Self {
            name: scenario.name.clone(),
            wf_index,
            weight: scenario.weight,
            think: scenario.think,
            inputs: scenario
                .inputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            data,
            iterations: scenario.iterations.max(1),
            continue_on_failure: scenario.continue_on_failure,
        })
    }
}

/// Everything users share for the duration of a run.
pub struct Shared {
    /// Compiled plan.
    pub plan: Arc<Plan>,
    /// Scenario table (index = [`ScenarioIdx`]).
    pub scenarios: Vec<CompiledScenario>,
    /// Target base URL.
    pub base_url: Arc<str>,
    /// Outputs of hook/prerequisite workflows, for `$workflows.<id>...`.
    pub workflows_outputs: Arc<InputMap>,
    /// Shared transport.
    pub http: Arc<dyn HttpClient>,
    /// Per-user transport factory, when enabled.
    pub http_factory: Option<HttpFactory>,
    /// Sample channel to the aggregator.
    pub tx: mpsc::Sender<Event>,
    /// Samples that could not be enqueued.
    pub dropped: AtomicU64,
    /// Run seed.
    pub seed: u64,
    /// Run identifier exposed as `$inputs.__run`.
    pub run_id: String,
    /// Environment name exposed as `$inputs.__env`.
    pub env: String,
    /// Users currently running.
    pub active: AtomicUsize,
    /// Credentials shared by every user (one token cache).
    pub auth: Arc<RunAuth>,
    /// Validate response bodies against their declared schemas.
    pub validate_responses: bool,
}

impl Shared {
    /// Non-blocking emit; a full queue counts a drop instead of stalling
    /// the user.
    pub fn emit(&self, event: Event) {
        if self.tx.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Picks a scenario index by weight.
    pub fn pick_scenario(&self, rng: &mut impl Rng) -> usize {
        let total: f64 = self.scenarios.iter().map(|s| s.weight).sum();
        let mut roll = rng.r#gen::<f64>() * total;
        for (i, s) in self.scenarios.iter().enumerate() {
            if roll < s.weight {
                return i;
            }
            roll -= s.weight;
        }
        self.scenarios.len() - 1
    }
}

/// Scenario/step labels for snapshots; deliberately holds no channel
/// sender so the aggregator can own it without keeping the queue open.
pub struct NameTable {
    plan: Arc<Plan>,
    /// `(scenario name, workflow index)` in scenario order.
    scenarios: Vec<(String, usize)>,
}

impl NameTable {
    /// Builds the table from compiled scenarios.
    #[must_use]
    pub fn new(plan: Arc<Plan>, scenarios: &[CompiledScenario]) -> Self {
        Self {
            plan,
            scenarios: scenarios
                .iter()
                .map(|s| (s.name.clone(), s.wf_index))
                .collect(),
        }
    }
}

impl StepNames for NameTable {
    fn names(&self, scenario: ScenarioIdx, step: u16) -> (String, String, String) {
        let Some((name, wf_index)) = self.scenarios.get(scenario as usize) else {
            return ("hook".to_owned(), format!("step{step}"), String::new());
        };
        let wf = &self.plan.workflows[*wf_index];
        let step_plan = wf.steps.get(step as usize);
        (
            name.clone(),
            step_plan.map_or_else(|| format!("step{step}"), |s| s.step_id.clone()),
            step_plan.map_or_else(String::new, |s| {
                format!(
                    "{} {}",
                    s.operation.method.as_str().to_uppercase(),
                    s.operation.path
                )
            }),
        )
    }

    fn scenario_name(&self, scenario: ScenarioIdx) -> String {
        self.scenarios
            .get(scenario as usize)
            .map_or_else(|| "hook".to_owned(), |(name, _)| name.clone())
    }
}

/// Observer streaming one user's step samples.
struct VuRecorder {
    scenario: ScenarioIdx,
    shared: Arc<Shared>,
}

#[async_trait]
impl ExecObserver for VuRecorder {
    async fn on_step_done(&self, cx: &StepCx<'_>, sample: &StepSample) {
        self.shared.emit(Event::Step {
            scenario: self.scenario,
            step: u16::try_from(cx.step_index).unwrap_or(u16::MAX),
            sample: sample.clone(),
        });
    }
}

/// Why a user could not even start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VuAbort {
    /// `order: unique` data ran out.
    DataExhausted,
    /// Per-user transport construction failed.
    Transport(String),
}

/// Runs one user to completion. Emits `VuCreated` first and `VuDone` last;
/// returns the abort reason when the user never ran its workflow.
pub async fn run_vu(shared: Arc<Shared>, vu: VuId, scenario_idx: usize) -> Result<bool, VuAbort> {
    let scenario_tag = ScenarioIdx::try_from(scenario_idx).unwrap_or(ScenarioIdx::MAX);
    shared.emit(Event::VuCreated {
        scenario: scenario_tag,
    });
    shared.active.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    let result = run_vu_inner(&shared, vu, scenario_idx, scenario_tag).await;
    shared.active.fetch_sub(1, Ordering::Relaxed);
    shared.emit(Event::VuDone {
        scenario: scenario_tag,
        ok: matches!(result, Ok(true)),
        duration: started.elapsed(),
    });
    result
}

async fn run_vu_inner(
    shared: &Arc<Shared>,
    vu: VuId,
    scenario_idx: usize,
    scenario_tag: ScenarioIdx,
) -> Result<bool, VuAbort> {
    let scenario = &shared.scenarios[scenario_idx];
    let mut rng = StdRng::seed_from_u64(shared.seed ^ vu.0.wrapping_mul(0x9E37_79B9_7F4A_7C15));

    let mut inputs = InputMap::new();
    for (key, source) in &scenario.inputs {
        let value = match source {
            InputSource::Literal(v) => v.clone(),
            InputSource::Pool(pool) => pool[rng.gen_range(0..pool.len())].clone(),
        };
        inputs.insert(key.clone(), value);
    }
    if let Some(feeder) = &scenario.data {
        let row = feeder
            .next_row(&mut rng)
            .map_err(|_| VuAbort::DataExhausted)?;
        inputs.extend(row);
    }
    inputs.insert("__vu".to_owned(), serde_json::json!(vu.0));
    inputs.insert("__run".to_owned(), serde_json::json!(shared.run_id));
    inputs.insert("__env".to_owned(), serde_json::json!(shared.env));
    inputs.insert("__scenario".to_owned(), serde_json::json!(scenario.name));

    let http: Arc<dyn HttpClient> = match &shared.http_factory {
        Some(factory) => factory().map_err(VuAbort::Transport)?,
        None => Arc::clone(&shared.http),
    };
    let observer: Arc<dyn ExecObserver> = Arc::new(VuRecorder {
        scenario: scenario_tag,
        shared: Arc::clone(shared),
    });

    let mut all_ok = true;
    for iteration in 0..scenario.iterations {
        inputs.insert("__iteration".to_owned(), serde_json::json!(iteration));
        let opts = RunOpts {
            base_url: Arc::clone(&shared.base_url),
            inputs: inputs.clone(),
            workflows_outputs: Arc::clone(&shared.workflows_outputs),
            think: ThinkPolicy::Honour {
                default: scenario.think,
            },
            vu,
            seed: shared.seed,
            auth: Arc::clone(&shared.auth),
            validate_responses: shared.validate_responses,
        };
        let outcome = run_workflow(
            &shared.plan,
            scenario.wf_index,
            &opts,
            http.as_ref(),
            observer.as_ref(),
        )
        .await;
        if !outcome.ok {
            all_ok = false;
            if !scenario.continue_on_failure {
                break;
            }
        }
    }
    Ok(all_ok)
}
