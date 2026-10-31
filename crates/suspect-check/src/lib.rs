#![deny(missing_docs)]
//! suspect-check: one check pass over every spec check.
//!
//! Only `suspect-lsp` and `suspect-cli` may depend on this crate; producer
//! crates export their rule tables and stay below it
//! (docs/proposals/P1-finding-model.md, "Crate graph").

pub mod registry;

pub use registry::{Registry, RegistryError};
