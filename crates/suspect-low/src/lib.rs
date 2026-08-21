//! suspect-low: generic ordered document model with source maps.
//!
//! A semantic view over [`suspect_syntax`] CSTs: typed scalar access,
//! RFC 6901 JSON Pointer navigation, transparent YAML alias resolution,
//! merge-key (`<<`) expansion, duplicate-key reporting, and OpenAPI/Arazzo/
//! Overlay family sniffing. This crate's API is the contract every higher
//! layer builds on.

mod doc;
mod node;
mod pointer;
mod scalar;

pub use doc::{LowDoc, SpecFamily};
pub use node::{DuplicateKey, Entry, NodeRef};
pub use pointer::{percent_decode_fragment, Pointer};
pub use scalar::ValueKind;
