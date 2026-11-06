//! Native Dart SDKs from canonical contracts, checked schemas and typed HTTP plans.
use std::sync::Arc;

use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedProgram, OwnedSchema};

#[cfg(test)]
use crate as codegen;
pub use crate::http_contract::HttpDiagnostic;
use crate::{OutFile, http_protocol};

#[cfg(test)]
#[path = "../tests/dart_support/mod.rs"]
mod support;

#[cfg(test)]
mod document_tests;
mod emit;
mod environment;
mod http_emit;
pub mod incoming;
mod models;
mod oauth;
mod pagination;
mod protocol;
#[cfg(test)]
mod sdk_v2_tests;
#[cfg(test)]
mod v3_sdk_tests;
#[cfg(test)]
mod v3_tests;
mod stream_events;
mod validation;
#[cfg(test)]
mod validation_tests;

pub use models::{DartField, DartModel, Extras as DartExtras, ModelPlan, Shape as DartShape};
pub use protocol::{
    PlannedAggregate, PlannedBody, PlannedCredential, PlannedHeader, PlannedHeaders, PlannedMedia,
    PlannedOperation, PlannedParameter, PlannedPart, PlannedPayload, PlannedStatus, plan_sdk,
    plan_sdk_with_profiles,
};

/// Pub identity is presentation configuration, independent of API semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Pub package name: a lowercase Dart identifier of at most 64 ASCII characters.
    pub name: String,
    /// Exact SemVer package version.
    pub version: String,
}
impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            name: "generated_sdk".into(),
            version: "0.0.0".into(),
        }
    }
}

/// Independent, finite schema, conversion, wire and streaming resource policies.
#[derive(Debug, Clone)]
pub struct DartConfig {
    /// Pub package identity.
    pub package: PackageConfig,
    /// Explicit variable names bound to used source security declarations.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
    /// Shared owned-schema compiler/executor policy; depth at most 128.
    pub schema: suspect_schema::Config,
    /// JSON parse/serialize nesting ceiling, 1 to 128.
    pub max_json_depth: usize,
    /// Native conversion nesting ceiling, 1 to 128.
    pub max_conversion_depth: usize,
    /// Shared native conversion visits per call.
    pub max_conversion_steps: usize,
    /// JSON parse/serialize node visits per document.
    pub max_json_steps: usize,
    /// Maximum request-body and assembled-URL bytes; default for `Client`.
    pub max_request_bytes: usize,
    /// Maximum response-body bytes; default for `Client`.
    pub max_response_bytes: usize,
    /// Bounded raw body prefix retained by SDK failures; at most `max_response_bytes`.
    pub max_capture_bytes: usize,
    /// Aggregate response header bytes; default for `Client`.
    pub max_response_header_bytes: usize,
    /// Independent ceiling for each in-memory form/multipart part.
    pub max_part_bytes: usize,
    /// Maximum framed SSE event or JSON-lines item before decoding.
    pub max_stream_item_bytes: usize,
    /// Maximum buffered stream bytes; at least `max_stream_item_bytes`.
    pub max_stream_buffer_bytes: usize,
}
impl Default for DartConfig {
    fn default() -> Self {
        Self {
            package: PackageConfig::default(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
            schema: suspect_schema::Config {
                max_depth: 128,
                ..Default::default()
            },
            max_json_depth: 128,
            max_conversion_depth: 128,
            max_conversion_steps: 32 * 1024 * 1024,
            max_json_steps: 32 * 1024 * 1024,
            max_request_bytes: 8 * 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            max_capture_bytes: 64 * 1024,
            max_response_header_bytes: 64 * 1024,
            max_part_bytes: 8 * 1024 * 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_stream_buffer_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Immutable native descriptors and the exact admitted protocol/schema programs.
pub struct Plan {
    contract: Arc<Contract>,
    config: DartConfig,
    protocol: http_protocol::ProtocolPlan,
    program: OwnedProgram,
    compiled: OwnedSchema,
    models: ModelPlan,
    operations: Vec<PlannedOperation>,
    credentials: Vec<PlannedCredential>,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    pagination: Option<http_protocol::PaginationOutcome>,
    oauth: Option<http_protocol::OAuthPlan>,
    /// Compiled incoming webhook/callback receipts over the whole contract.
    incoming: http_protocol::IncomingPlan,
    /// Emission-ready receipt helpers bound to the compiled model codecs.
    incoming_receipts: incoming::Prepared,
    stream_semantics: http_protocol::StreamSemanticsPlan,
    examples: crate::examples::ExamplePlan,
}
impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("config", &self.config)
            .field("models", &self.models)
            .field("operations", &self.operations)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}
impl Plan {
    /// Canonical contract the plan was built from.
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// Native model symbols for every compiled codec schema.
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }
    /// Selected operations, in protocol order.
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    /// Client credential slots, one per used source security scheme.
    #[must_use]
    pub fn credentials(&self) -> &[PlannedCredential] {
        &self.credentials
    }
    /// Environment-variable bindings for credentials, when configured.
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    /// Compiled pagination selection over the admitted protocol plan, retained
    /// only when a client-default policy is configured.
    #[must_use]
    pub fn pagination(&self) -> Option<&http_protocol::PaginationOutcome> {
        self.pagination.as_ref()
    }
    /// Compiled OAuth lifecycle plan over the used source schemes, retained
    /// only when a client-default policy is configured.
    #[must_use]
    pub fn oauth(&self) -> Option<&http_protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// Compiled incoming webhook/callback receipts over the whole contract.
    /// Planning walks the whole Contract (selection-independent) and fails the
    /// plan on broken incoming declarations like every other diagnostic.
    #[must_use]
    pub fn incoming(&self) -> &http_protocol::IncomingPlan {
        &self.incoming
    }
    /// Emission-ready incoming receipt helpers. Receipts the v1 helpers cannot
    /// decode produce plan errors, so a planned receipt list is always
    /// renderable.
    #[must_use]
    pub fn incoming_receipts(&self) -> &incoming::Prepared {
        &self.incoming_receipts
    }
    /// Compiled typed-stream semantics over the admitted protocol plan. The
    /// planner is infallible and every entry carries its own evidence; only
    /// discriminated SSE operations lower into emitted typed events members.
    #[must_use]
    pub fn stream_semantics(&self) -> &http_protocol::StreamSemanticsPlan {
        &self.stream_semantics
    }
    /// Compiled ua/v1 attribution descriptor retained from the backend config.
    #[must_use]
    pub fn attribution(&self) -> Option<&crate::attribution::AttributionDescriptor> {
        self.config.attribution.as_ref()
    }
    /// Checked validation program executed by the generated runtime.
    #[must_use]
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    /// Admitted protocol plan for the selected operations.
    #[must_use]
    pub fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    /// Configuration the plan was admitted under.
    #[must_use]
    pub fn config(&self) -> &DartConfig {
        &self.config
    }
    /// Per-operation source examples used by generated docs.
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Render the complete pub package.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        emit::package(self)
    }
}
/// Backend-neutral alias for [`Plan`].
pub type SdkPlan = Plan;
/// Render the complete pub package; same as [`Plan::render`].
#[must_use]
pub fn emit_sdk(plan: &Plan) -> Vec<OutFile> {
    plan.render()
}

fn diag(
    contract: &Contract,
    source: SchemaId,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        message: message.into(),
    }
}
