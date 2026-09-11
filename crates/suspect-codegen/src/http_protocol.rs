//! Source-backed, language-neutral HTTP protocol planning.
//!
//! [`plan`] consumes the canonical [`suspect_ir::contract::Contract`]. It makes
//! wire decisions once and publishes immutable, source-located descriptors.
//! It does not allocate language symbols, compile a second schema graph, emit
//! clients, acquire credentials, or infer workflows. Native adapters opt in to
//! capabilities only after testing their implementation of those descriptors.
//!
//! A plan is atomic: errors leave no operations or codec roots. Warnings retain
//! uninterpreted annotations. Only actual JSON/text codec inputs become roots;
//! opaque bytes have a separate finite byte policy.

mod bodies;
mod capabilities;
mod examples;
mod media;
mod model;
mod parameters;
mod planner;
mod resource;
mod responses;
mod security;
mod servers;
mod shapes;
mod wire;

pub use capabilities::*;
pub use media::{MediaMatchError, MediaRange, MediaType, ResponseMatch, ResponseMatchError};
pub use model::*;
pub use planner::plan;
pub use wire::{SerializedParameter, WireError};

/// Version of the public, serializable descriptor semantics and fixture set.
pub const PROTOCOL_PLAN_VERSION: u32 = 1;
