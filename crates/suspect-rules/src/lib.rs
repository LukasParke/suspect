//! TS/JS custom rule runtime.
//!
//! Host side of the Bun rule worker (see `docs/TS-RULE-RUNTIME.md`): spawns
//! a long-lived `bun` sidecar, negotiates the NDJSON protocol, evaluates
//! each rule's `given` selector natively against the [`LowDoc`] CST, ships
//! only the selected nodes, enforces per-run deadlines with kill-and-restart
//! semantics, and resolves finding spans back to byte ranges.
//!
//! ```no_run
//! # async fn demo() -> Result<(), suspect_rules::Error> {
//! # let mut host = suspect_rules::RuleHost::start(suspect_rules::StartOptions {
//! #     workspace_root: std::path::PathBuf::from("."),
//! #     rule_files: vec![std::path::PathBuf::from(".suspect/rules/my-rule.ts")],
//! #     timeout_ms: Some(250),
//! #     ..Default::default()
//! # }).await?.expect("rules present");
//! # let doc = suspect_low::LowDoc::parse(
//! #     suspect_source::Uri::from("mem://openapi.yaml"),
//! #     suspect_source::Source::from_vec(vec![]),
//! # );
//! for finding in host.evaluate(&doc).await? {
//!     println!("{}: {}", finding.pointer, finding.message);
//! }
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

pub mod discover;
pub mod host;
pub mod mirrors;
pub mod node_json;
pub mod protocol;
pub mod select;
pub mod worker;

use std::path::PathBuf;

pub use discover::{RulesConfig, discover_rule_files, scaffold_rule, stage_worker_files};
pub use host::{RuleHost, StartOptions, TsFinding};
pub use protocol::{PROTOCOL_VERSION, RuleMeta, TargetKind};
pub use worker::WorkerError;

/// Anything that can go wrong in the rule runtime.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Bun is not on `PATH` or failed its version gate.
    #[error("bun runtime unavailable: {0}")]
    BunUnavailable(String),
    /// The worker violated the protocol (unparseable frame, wrong type).
    #[error("worker protocol violation: {0}")]
    Protocol(String),
    /// A rule file failed to load or declare itself.
    #[error("rule load failed: {0}")]
    RuleLoad(String),
    /// IO failure talking to the worker process.
    #[error("worker io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization failure on a protocol frame.
    #[error("frame codec: {0}")]
    Codec(#[from] serde_json::Error),
    /// The worker died (non-zero exit, closed stdout).
    #[error("worker exited: {0}")]
    WorkerDied(String),
    /// The JSONPath selector of a rule did not compile.
    #[error("bad selector in rule {rule}: {message}")]
    BadSelector {
        /// Rule id whose selector failed.
        rule: String,
        /// Underlying parse error.
        message: String,
    },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Default per-evaluate deadline for interactive (LSP) use.
pub const DEFAULT_TIMEOUT_MS: u64 = 250;

/// Directory (relative to the workspace root) TS rules are discovered in
/// when the config does not name files explicitly.
pub const DEFAULT_RULES_DIR: &str = ".suspect/rules";

/// Workspace-rooted path helper used across discovery and scaffolding.
#[must_use]
pub fn workspace_join(root: &std::path::Path, rel: &str) -> PathBuf {
    root.join(rel)
}
