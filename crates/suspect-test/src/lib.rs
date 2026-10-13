#![deny(missing_docs)]
//! suspect-test: Arazzo-driven contract test planning and execution.
//!
//! The crate turns an [Arazzo 1.0] description plus a loaded OpenAPI
//! workspace into an executable [`Plan`], runs the plan against
//! an [`HttpClient`] transport, and reports results as
//! console text, JUnit XML, or NDJSON events.
//!
//! Pipeline:
//!
//! ```text
//! ArazzoDoc + Workspace --compile_plan--> Plan --run_plan--> RunSummary
//!                                                |
//!                                          TestEvent stream
//! ```
//!
//! [`compile_plan`] resolves every step's `operationId`/`operationPath`
//! against normalized IR snapshots ([`suspect_ir::IrSpec`]) so steps address
//! canonical `method + path` keys, parses success criteria into a pragmatic
//! criterion model, and keeps runtime expressions ([`suspect_rex::Rex`])
//! for late evaluation. [`run_plan`] executes workflows concurrently and
//! steps sequentially within each workflow, substituting parameters,
//! chaining step outputs, and evaluating criteria against live responses.
//!
//! Execution is observable through the [`ExecObserver`] seam:
//! [`EventObserver`] renders the [`TestEvent`] stream, and
//! [`run_workflow_owned`] runs one workflow as a spawnable future so many
//! virtual users can share one plan (the basis of load generation).
//!
//! No HTTP client ships in this crate; real transports arrive with the CLI.
//! [`transports`] provides deterministic in-process transports for testing:
//! [`CannedTransport`](transports::CannedTransport) (request-matched canned
//! responses), [`ReplayTransport`](transports::ReplayTransport)
//! (sequential cassette replay) and
//! [`DelayTransport`](transports::DelayTransport) (simulated latency).
//!
//! [Arazzo 1.0]: https://spec.openapis.org/arazzo/v1.0.0

pub mod auth;
pub mod duration;
pub mod evolved_fuzz;
pub mod exec;
pub mod fuzz;
pub mod messaging;
pub use suspect_faker as faker;
pub mod plan;
pub mod reporters;
pub mod stateful;
pub mod transports;

#[cfg(test)]
mod tests;

pub use exec::{
    ErrorClass, EventObserver, ExecObserver, HttpClient, HttpRequest, HttpResponse, InputMap,
    NullObserver, RunAuth, RunOpts, RunSummary, StepCx, StepSample, StepVerdict, TestEvent,
    ThinkPolicy, TransportError, VuId, WfOutcome, run_plan, run_plan_observed, run_plan_with,
    run_plan_with_auth, run_plan_with_auth_and_messages, run_plan_with_messages, run_workflow,
    run_workflow_owned,
};
pub use messaging::{FileBroker, LoopbackBroker, Message, MessageError, MessageTransport};
pub use plan::{
    Comparison, CompileError, CriterionKind, CriterionPlan, Expected, MessageDirection,
    MessageStep, OpKey, Plan, StepParam, StepPlan, ThinkSpec, WfPlan, compile_plan,
    dependency_waves,
};
