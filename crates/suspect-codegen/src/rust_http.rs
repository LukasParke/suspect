//! Source-selected Rust HTTP clients, exact codecs and private Cargo packaging.
//!
//! HTTP admission consumes the shared source-backed protocol descriptors once.
//! Native names, codecs, parts, streams, transport and documentation are planned here.

use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{
    OutFile, http_protocol,
    rust_codecs::{CodecConfig, CodecPlan},
    rust_models,
};

mod emit;
mod incoming;
mod plan;
mod stream_events;

pub use incoming::{IncomingPayload, IncomingReceipt};
pub use plan::{
    native_capabilities, native_capabilities_v3, plan_http, plan_http_v2, plan_http_v3,
};
pub use stream_events::{EventMetadata, StreamEventsEntry};

/// Exact adapter source closure for compatibility/runtime provenance. Paths are
/// relative to `suspect-codegen/src`; these are compile-time assets, not parsed
/// generated code or a second admission pipeline.
#[must_use]
pub fn source_assets() -> &'static [(&'static str, &'static [u8])] {
    &[
        ("rust_http.rs", include_bytes!("rust_http.rs")),
        ("rust_http/plan.rs", include_bytes!("rust_http/plan.rs")),
        ("rust_http/emit.rs", include_bytes!("rust_http/emit.rs")),
        (
            "rust_http/emit/bodies.rs",
            include_bytes!("rust_http/emit/bodies.rs"),
        ),
        (
            "rust_http/emit/descriptors.rs",
            include_bytes!("rust_http/emit/descriptors.rs"),
        ),
        (
            "rust_http/emit/docs.rs",
            include_bytes!("rust_http/emit/docs.rs"),
        ),
        (
            "rust_http/emit/credential_env.rs",
            include_bytes!("rust_http/emit/credential_env.rs"),
        ),
        (
            "rust_http/emit/operation.rs",
            include_bytes!("rust_http/emit/operation.rs"),
        ),
        (
            "rust_http/runtime.rs",
            include_bytes!("rust_http/runtime.rs"),
        ),
        (
            "rust_http/runtime/descriptors.rs",
            include_bytes!("rust_http/runtime/descriptors.rs"),
        ),
        (
            "rust_http/runtime/errors.rs",
            include_bytes!("rust_http/runtime/errors.rs"),
        ),
        (
            "rust_http/runtime/media.rs",
            include_bytes!("rust_http/runtime/media.rs"),
        ),
        (
            "rust_http/runtime/parameters.rs",
            include_bytes!("rust_http/runtime/parameters.rs"),
        ),
        (
            "rust_http/runtime/parts.rs",
            include_bytes!("rust_http/runtime/parts.rs"),
        ),
        (
            "rust_http/runtime/security.rs",
            include_bytes!("rust_http/runtime/security.rs"),
        ),
        (
            "rust_http/runtime/servers.rs",
            include_bytes!("rust_http/runtime/servers.rs"),
        ),
        (
            "rust_http/runtime/stream.rs",
            include_bytes!("rust_http/runtime/stream.rs"),
        ),
        (
            "rust_http/reqwest.rs",
            include_bytes!("rust_http/reqwest.rs"),
        ),
        (
            "rust_models/applicators.rs",
            include_bytes!("rust_models/applicators.rs"),
        ),
        (
            "rust_validation/runtime_v2.rs",
            include_bytes!("rust_validation/runtime_v2.rs"),
        ),
        (
            "rust_models/resources.rs",
            include_bytes!("rust_models/resources.rs"),
        ),
        (
            "rust_validation/runtime_v3.rs",
            include_bytes!("rust_validation/runtime_v3.rs"),
        ),
    ]
}

/// Finite transport and codec policy embedded in the generated package.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Shared model validation and exact JSON conversion policy.
    pub codecs: CodecConfig,
    /// Generated response ceiling; callers may only lower it.
    pub max_response_bytes: usize,
    /// Ceiling for an assembled URL and for a serialized request body.
    pub max_request_bytes: usize,
    /// Independent ceiling for each finite form/multipart part.
    pub max_part_bytes: usize,
    /// Maximum buffered SSE event or JSON-lines item, including framing.
    pub max_stream_item_bytes: usize,
    /// Maximum individual transport chunk retained by the runtime.
    pub max_chunk_bytes: usize,
    /// Maximum total raw header-name/value bytes per response or part.
    pub max_header_bytes: usize,
    /// Explicit versioned wire departures; ordinary OpenAPI semantics are default.
    pub compatibility_profiles: Vec<http_protocol::CompatibilityProfile>,
    /// Explicit runtime environment-variable names, bound after protocol admission.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            codecs: CodecConfig::default(),
            max_response_bytes: 8 * 1024 * 1024,
            max_request_bytes: 8 * 1024 * 1024,
            max_part_bytes: 8 * 1024 * 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_chunk_bytes: 8 * 1024 * 1024,
            max_header_bytes: 64 * 1024,
            compatibility_profiles: Vec::new(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        }
    }
}

pub use crate::http_contract::HttpDiagnostic;

/// Allocated native operation names shared by code, examples and documentation.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub module_name: String,
    pub function_name: String,
    pub input_type: String,
    pub success_type: String,
    pub error_type: String,
    pub api_error_type: String,
    /// Allocated no-input helper, present when no input is required.
    pub default_function_name: Option<String>,
    wire: http_protocol::OperationPlan,
    parameters: Vec<PlannedParameter>,
    body: Option<PlannedBody>,
    responses: Vec<PlannedResponse>,
    credentials: Vec<PlannedCredential>,
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    wire: http_protocol::ParameterPlan,
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct PlannedHeader {
    wire: http_protocol::HeaderPlan,
    pub name: String,
    pub model: String,
}

/// Native payload identity. Byte payloads never have a JSON codec binding.
#[derive(Debug, Clone)]
pub enum Payload {
    Model {
        schema: SchemaId,
        model: String,
    },
    Json,
    Text,
    Bytes,
    NoContent,
    Stream {
        schema: Option<SchemaId>,
        model: Option<String>,
        request: bool,
    },
    Parts(Box<PlannedAggregate>),
}

#[derive(Debug, Clone)]
pub struct PlannedMedia {
    wire: http_protocol::MediaPlan,
    pub variant: String,
    pub payload: Payload,
}

#[derive(Debug, Clone)]
pub struct PlannedBody {
    wire: http_protocol::BodyPlan,
    /// A choice enum for multiple media, otherwise the native payload type.
    pub type_name: String,
    media: Vec<PlannedMedia>,
}

/// A finite structural aggregate with separately bound part/item codecs.
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    pub type_name: String,
    pub multipart: bool,
    pub positional: bool,
    parts: Vec<PlannedPart>,
    additional: Option<Box<PlannedPart>>,
}

#[derive(Debug, Clone)]
pub struct PlannedPart {
    wire: http_protocol::PartPlan,
    pub name: String,
    pub model: Option<String>,
    pub headers_type: Option<String>,
    headers: Vec<PlannedHeader>,
}

#[derive(Debug, Clone)]
pub struct PlannedResponse {
    wire: http_protocol::ResponsePlan,
    pub headers_type: Option<String>,
    headers: Vec<PlannedHeader>,
    variants: Vec<PlannedResponseVariant>,
}

#[derive(Debug, Clone)]
pub struct PlannedResponseVariant {
    pub name: String,
    pub media_index: Option<usize>,
    pub success: bool,
    pub error: bool,
    pub forbidden: bool,
    pub payload: Payload,
}

macro_rules! wire_accessor {
    ($native:ty, $wire:ty) => {
        impl $native {
            #[must_use]
            pub fn wire(&self) -> &$wire {
                &self.wire
            }
        }
    };
}
wire_accessor!(PlannedOperation, http_protocol::OperationPlan);
wire_accessor!(PlannedParameter, http_protocol::ParameterPlan);
wire_accessor!(PlannedHeader, http_protocol::HeaderPlan);
wire_accessor!(PlannedMedia, http_protocol::MediaPlan);
wire_accessor!(PlannedBody, http_protocol::BodyPlan);
wire_accessor!(PlannedPart, http_protocol::PartPlan);
wire_accessor!(PlannedResponse, http_protocol::ResponsePlan);

impl PlannedOperation {
    #[must_use]
    pub fn parameters(&self) -> &[PlannedParameter] {
        &self.parameters
    }
    #[must_use]
    pub fn body(&self) -> Option<&PlannedBody> {
        self.body.as_ref()
    }
    #[must_use]
    pub fn responses(&self) -> &[PlannedResponse] {
        &self.responses
    }
    #[must_use]
    pub fn credentials(&self) -> &[PlannedCredential] {
        &self.credentials
    }
    /// Structured native bindings, produced from this plan without reading artifacts.
    #[must_use]
    pub fn interface(&self) -> serde_json::Value {
        emit::interface(self)
    }
}
impl PlannedBody {
    #[must_use]
    pub fn media(&self) -> &[PlannedMedia] {
        &self.media
    }
}
impl PlannedAggregate {
    #[must_use]
    pub fn parts(&self) -> &[PlannedPart] {
        &self.parts
    }
    #[must_use]
    pub fn additional(&self) -> Option<&PlannedPart> {
        self.additional.as_deref()
    }
}
impl PlannedPart {
    #[must_use]
    pub fn headers(&self) -> &[PlannedHeader] {
        &self.headers
    }
}
impl PlannedResponse {
    #[must_use]
    pub fn headers(&self) -> &[PlannedHeader] {
        &self.headers
    }
    #[must_use]
    pub fn variants(&self) -> &[PlannedResponseVariant] {
        &self.variants
    }
}

/// Immutable complete selected-operation plan. It does not certify a whole SDK.
#[derive(Debug)]
pub struct HttpPlan {
    contract: Arc<Contract>,
    protocol: http_protocol::ProtocolPlan,
    operations: Vec<PlannedOperation>,
    codecs: CodecPlan,
    symbols: BTreeMap<SchemaId, String>,
    credentials: BTreeMap<SourceId, PlannedCredential>,
    config: HttpConfig,
    examples: crate::examples::ExamplePlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    pagination: Option<http_protocol::PaginationOutcome>,
    oauth: Option<http_protocol::OAuthPlan>,
    stream_events: Vec<stream_events::StreamEventsEntry>,
    incoming: Option<http_protocol::IncomingPlan>,
    incoming_receipts: Vec<incoming::IncomingReceipt>,
}

#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub constructor: String,
    pub requirement: http_protocol::CredentialRequirement,
}

impl HttpPlan {
    /// The exact admitted shared plan, including source provenance and warnings.
    #[must_use]
    pub fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    #[must_use]
    pub fn symbols(&self) -> &BTreeMap<SchemaId, String> {
        &self.symbols
    }
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<SourceId, PlannedCredential> {
        &self.credentials
    }
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    /// Compiled `ua/v1` attribution constants carried by this plan, when configured.
    #[must_use]
    pub fn attribution(&self) -> Option<&crate::attribution::AttributionDescriptor> {
        self.config.attribution.as_ref()
    }
    /// Application-selected client defaults carried by this plan, when configured.
    #[must_use]
    pub fn sdk_defaults(&self) -> Option<&crate::sdk_defaults::SdkDefaults> {
        self.config.sdk_defaults.as_ref()
    }
    /// Compiled pagination selection carried by this plan. Only the canonical
    /// v3 planning path with configured `sdk_defaults` populates it; the
    /// retained v1/v2 APIs leave it `None` and their artifacts stay unchanged.
    #[must_use]
    pub fn pagination(&self) -> Option<&http_protocol::PaginationOutcome> {
        self.pagination.as_ref()
    }
    /// Compiled OAuth lifecycle plan carried by this plan. Only the canonical
    /// v3 planning path with configured `sdk_defaults` in `auto` mode and
    /// compiled source schemes populates it; the retained v1/v2 APIs leave it
    /// `None` and their artifacts stay unchanged. Emission additionally gates
    /// on executable flows, so plans with only deprecated or
    /// discovery-defined schemes still emit nothing.
    #[must_use]
    pub fn oauth(&self) -> Option<&http_protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// The compiled typed-event subset of the stream semantics plan: one
    /// entry per operation that will emit a typed events iterator. Computed
    /// unconditionally by the canonical v3 path; only admitted discriminated
    /// SSE operations produce entries, so other plans gain no artifact.
    #[must_use]
    pub fn stream_events(&self) -> &[stream_events::StreamEventsEntry] {
        &self.stream_events
    }
    /// Compiled incoming webhook/callback receipts carried by this plan. Only
    /// the canonical v3 planning path populates it; the retained v1/v2 APIs
    /// leave it `None` and their artifacts stay unchanged. A `None` value (and
    /// an empty plan) emits no artifact at all.
    #[must_use]
    pub fn incoming(&self) -> Option<&http_protocol::IncomingPlan> {
        self.incoming.as_ref()
    }
    /// The emission-ready incoming receipts lowered from the compiled incoming
    /// plan against this plan's compiled codec symbols. Empty exactly when no
    /// receipt helper is emitted.
    #[must_use]
    pub fn incoming_receipts(&self) -> &[incoming::IncomingReceipt] {
        &self.incoming_receipts
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn codecs(&self) -> &CodecPlan {
        &self.codecs
    }
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// Validated declared/synthesized examples and source-linked findings.
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Render a private `generated-sdk` package using the default package identity.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        emit_http(self, &PackageConfig::default()).expect("default package identity is valid")
    }
}

/// Portable Cargo identity, independent of every API semantic and source version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Lowercase ASCII name, beginning with a letter, up to 64 bytes.
    /// Remaining characters may be letters, digits, hyphens or underscores.
    pub name: String,
    /// An exact Cargo SemVer; version requirements are not accepted.
    pub version: String,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            name: "generated-sdk".into(),
            version: "0.0.0".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    InvalidName,
    InvalidVersion,
}
impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid portable Cargo package name",
            Self::InvalidVersion => "invalid exact Cargo package version",
        })
    }
}
impl std::error::Error for PackageError {}

/// Emit an installable Cargo package, all native docs and its HTTP manifest.
/// Model-only use has no dependencies; HTTP and reqwest/rustls are opt-in features.
///
/// # Errors
/// Invalid package identity fails before any artifact is returned or written.
pub fn emit_http(plan: &HttpPlan, package: &PackageConfig) -> Result<Vec<OutFile>, PackageError> {
    let name = &package.name;
    let crate_name = name.replace('-', "_");
    if name.is_empty()
        || name.len() > 64
        || !name.as_bytes()[0].is_ascii_lowercase()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        || rust_models::snake(&crate_name) != crate_name
        || matches!(crate_name.as_str(), "std" | "core" | "alloc" | "test")
    {
        return Err(PackageError::InvalidName);
    }
    if semver::Version::parse(&package.version).is_err() {
        return Err(PackageError::InvalidVersion);
    }
    Ok(emit::package(plan, package, &crate_name))
}

fn diagnostic(
    contract: &Contract,
    source: SourceId,
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

fn allocate(base: &str, used: &mut std::collections::BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}
