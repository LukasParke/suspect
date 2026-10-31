#![deny(missing_docs)]
//! suspect-check: one check pass over every spec check.
//!
//! [`check_document`] runs every builtin check for one document and
//! [`apply_policy`] decides what is reported. The editor calls both
//! through `suspect_lsp::diagnostics::compute_diagnostics`.
//!
//! Only `suspect-lsp` and `suspect-cli` may depend on this crate; producer
//! crates export their rule tables and stay below it
//! (docs/proposals/P1-finding-model.md, "Crate graph").

pub mod checks;
mod policy;
pub mod registry;
pub mod rules;
pub mod suppress;

pub use checks::{Context, check_document};
pub use policy::{
    CheckSet, Policy, PolicyOutcome, apply_policy, apply_policy_report, codegen_configured,
};
pub use registry::{Registry, RegistryError};
pub use rules::{RulesError, check_rules};
