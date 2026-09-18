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
mod incoming;
mod media;
mod model;
mod oauth;
mod pagination;
mod parameters;
mod planner;
mod resource;
mod responses;
mod security;
mod servers;
mod shapes;
mod stream_plan;
mod wire;

pub use capabilities::*;
pub use incoming::plan_incoming;
pub use incoming::{
    IncomingKind, IncomingOperationPlan, IncomingPlan, IncomingRequestPlan, IncomingResponsePlan,
    IncomingRoute, admits_receipt_decode,
};
pub use media::{MediaMatchError, MediaRange, MediaType, ResponseMatch, ResponseMatchError};
pub use model::*;
pub use oauth::{
    OAuthClientAuth, OAuthDefaults, OAuthFlowDescriptor, OAuthFlowDescriptorKind, OAuthMode,
    OAuthPlan, OAuthRefresh, OAuthSchemeConfig, OAuthSchemeKind, OAuthSchemePlan, OAuthStorage,
    plan as plan_oauth,
};
pub use pagination::plan as plan_pagination;
pub use pagination::{OperationPagination, PaginationOutcome, SinglePageExplanation};
pub use planner::plan;
pub use stream_plan::plan as plan_stream_semantics;
pub use stream_plan::{
    EventPayload, FrameRules, InvalidPayloadPolicy, SentinelEvidence, SentinelPolicy,
    SentinelStage, StreamEventPlan, StreamItemMetadata, StreamOperationPlan, StreamSemanticsPlan,
    TerminalAction, TerminalPolicy, UnknownEventPolicy, UnknownEventRepresentation,
};
pub use wire::{SerializedParameter, WireError};

/// Version of the public, serializable descriptor semantics and fixture set.
pub const PROTOCOL_PLAN_VERSION: u32 = 1;
