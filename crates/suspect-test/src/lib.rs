#![deny(missing_docs)]
//! suspect-test: Arazzo-driven contract test planning and execution.
//!
//! The crate turns an [Arazzo 1.0] description plus a loaded OpenAPI
//! workspace into an executable [`Plan`](plan::Plan), runs the plan against
//! an [`HttpClient`](exec::HttpClient) transport, and reports results as
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
//! No HTTP client ships in this crate; real transports arrive with the CLI.
//! [`transports`] provides deterministic in-process transports for testing:
//! [`CannedTransport`](transports::CannedTransport) (request-matched canned
//! responses) and [`ReplayTransport`](transports::ReplayTransport)
//! (sequential cassette replay).
//!
//! [Arazzo 1.0]: https://spec.openapis.org/arazzo/v1.0.0

pub mod exec;
pub mod fuzz;
pub mod plan;
pub mod reporters;
pub mod transports;

#[cfg(test)]
mod tests;

pub use exec::{
    HttpClient, HttpRequest, HttpResponse, RunSummary, TestEvent, TransportError, run_plan,
};
pub use plan::{
    CompileError, CriterionKind, CriterionPlan, OpKey, Plan, StepParam, StepPlan, WfPlan,
    compile_plan,
};
