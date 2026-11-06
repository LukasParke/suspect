//! Source-selected Kotlin/JVM SDK with native models and coroutine protocol clients.
pub use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::{OutFile, examples::ExamplePlan};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedProgram;

mod emit;
mod environment;
mod incoming;
pub mod models;
mod oauth;
mod pagination;
mod protocol;
mod rich_emit;
mod samples;
mod stream_events;
pub mod validation;
mod validation_v3;

pub use incoming::{IncomingBody, IncomingEntry, IncomingPayload, IncomingReply};
pub use pagination::{Advance, ControlKind, PaginationOperation, PaginationPlan};
pub use stream_events::{StreamEventsEntry, StreamEventsPlan};

/// Kotlin compiler and stdlib version pinned in the generated Maven build.
pub const KOTLIN_VERSION: &str = "2.4.20";
/// `kotlinx-coroutines` version pinned in the generated Maven build.
pub const COROUTINES_VERSION: &str = "1.11.0";
/// Dokka documentation plugin version pinned in the generated Maven build.
pub const DOKKA_VERSION: &str = "2.2.0";

/// Maven/package identity; does not add API semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkConfig {
    /// Maven group ID.
    pub group_id: String,
    /// Maven artifact ID.
    pub artifact_id: String,
    /// Maven artifact version.
    pub version: String,
    /// Kotlin package of all generated declarations; must not start with
    /// `java` or `kotlin`.
    pub package_name: String,
    /// Explicit runtime variable names, bound to used source security schemes.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
}
impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            group_id: "generated".into(),
            artifact_id: "sdk".into(),
            version: "0.0.0".into(),
            package_name: "generated.sdk".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        }
    }
}

/// A native type binding. Only Model references are canonical JSON codec roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    /// A planned model, decoded by its checked codec.
    Model(SchemaId),
    /// Schemaless JSON as `JsonValue`.
    Json,
    /// Schemaless text scalar as `String`.
    String,
    /// Schemaless boolean text scalar.
    Boolean,
    /// Schemaless numeric text scalar as `JsonNumber`.
    Number,
    /// Raw octets as `ByteArray`.
    Bytes,
    /// No body.
    Unit,
    /// A generated or runtime type referenced by name.
    Named(String),
    /// Repeated values as `List<T>`.
    List(Box<NativeType>),
    /// Streamed items as a coroutine `Flow<T>`.
    Flow(Box<NativeType>),
}
impl NativeType {
    /// Kotlin source spelling of this type.
    pub(crate) fn kotlin(&self, models: &models::ModelPlan) -> String {
        match self {
            Self::Model(id) => models.get(id).kotlin_type.clone(),
            Self::Json => "JsonValue".into(),
            Self::String => "String".into(),
            Self::Boolean => "Boolean".into(),
            Self::Number => "JsonNumber".into(),
            Self::Bytes => "ByteArray".into(),
            Self::Unit => "Unit".into(),
            Self::Named(name) => name.clone(),
            Self::List(ty) => format!("List<{}>", ty.kotlin(models)),
            Self::Flow(ty) => format!("kotlinx.coroutines.flow.Flow<{}>", ty.kotlin(models)),
        }
    }
}

/// One source security scheme as a client credential constructor field.
#[derive(Debug, Clone)]
pub struct PlannedCredential {
    /// Terminal security scheme.
    pub source: SourceId,
    /// Source security scheme name.
    pub scheme_name: String,
    /// Kotlin field name, unique across the client's credentials.
    pub name: String,
    /// Nullable credential type: `BasicCredentials?`, `CredentialProvider?`
    /// for OAuth 2 / OpenID Connect, otherwise `String?`.
    pub kotlin_type: String,
    /// Shared wire requirement that first introduced the scheme.
    pub wire: wire::CredentialRequirement,
}

/// One operation parameter as an input-class property.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Parameter use site.
    pub source: SourceId,
    /// Parameter codec schema.
    pub schema: SchemaId,
    /// Source parameter name.
    pub wire_name: String,
    /// Kotlin property name, unique within the input class.
    pub name: String,
    /// Property type; optional parameters are wrapped in `Presence<T>`.
    pub kotlin_type: String,
    /// Codec object for `schema`.
    pub codec_name: String,
    /// Whether the parameter must be supplied.
    pub required: bool,
    /// Shared wire decisions for the parameter.
    pub wire: wire::ParameterPlan,
}

/// One typed field of a generated response or part headers class.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Kotlin property name, unique within the headers class.
    pub name: String,
    /// Header model type.
    pub ty: NativeType,
    /// Codec object for the header schema.
    pub codec_name: String,
    /// Shared wire decisions for the header.
    pub wire: wire::HeaderPlan,
}

/// Finite named form/multipart field. Binary parts carry bytes, never JSON stand-ins.
#[derive(Debug, Clone)]
pub struct PlannedPart {
    /// Kotlin property name, unique within the form.
    pub name: String,
    /// Property type: the wrapper or value type, as a `List` for repeated array items.
    pub ty: NativeType,
    /// Type of one part value: its model, or `Upload` for binary parts.
    pub value_type: NativeType,
    /// Generated part class carrying headers or a content-type choice, when needed.
    pub wrapper: Option<String>,
    /// Generated part headers class, when headers are declared.
    pub headers_type: Option<String>,
    /// Part header fields in declaration order.
    pub headers: Vec<PlannedHeader>,
    /// Shared wire decisions for the part.
    pub wire: wire::PartPlan,
}

/// Generated URL-encoded form or named multipart body class.
#[derive(Debug, Clone)]
pub struct PlannedForm {
    /// Generated class name.
    pub name: String,
    /// Schema the object rules are drawn from.
    pub source: SourceId,
    /// Multipart rather than URL-encoded form.
    pub multipart: bool,
    /// Object rules the runtime enforces over fields.
    pub rules: wire::ObjectRules,
    /// Declared fields in source order.
    pub fields: Vec<PlannedPart>,
    /// Additional-property part; `None` when additional fields are forbidden.
    pub additional: Option<Box<PlannedPart>>,
}

/// One request or response media type and its native value.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Representation label (`Json`, `Text`, `Bytes`, `Items`, `Form`,
    /// `Multipart`), made unique within a request body.
    pub name: String,
    /// Native value type; request streams are wrapped in [`NativeType::Flow`].
    pub ty: NativeType,
    /// Codec schema of the value or stream item, when one exists.
    pub schema: Option<SchemaId>,
    /// Codec object for `schema`.
    pub codec_name: Option<String>,
    /// Generated form or multipart class, for aggregate representations.
    pub form: Option<PlannedForm>,
    /// Shared wire decisions for the media.
    pub wire: wire::MediaPlan,
}

/// Operation request body as the input's `body` property.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Request body use site.
    pub source: SourceId,
    /// Kotlin property name.
    pub name: String,
    /// Body type: the choice type, or the single media's type.
    pub ty: NativeType,
    /// Property type; an optional body is wrapped in `Presence<T>`.
    pub kotlin_type: String,
    /// Whether the body must be supplied.
    pub required: bool,
    /// Planned media in protocol order.
    pub media: Vec<PlannedMedia>,
    /// Generated choice type over several media; `None` for a single media.
    pub choice_type: Option<String>,
}

/// One native status/media/disposition constructor. Defaults have success and
/// failure members distinguished using the *actual* status at runtime.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Response use site.
    pub source: SourceId,
    /// Declared status, code range or `default`.
    pub status: wire::ResponseStatus,
    /// Source status key, e.g. `200`, `4XX` or `default`.
    pub status_key: String,
    /// Member of the result type rather than the API exception type.
    pub success: bool,
    /// Nested case class name, unique within its result or exception type.
    pub variant_name: String,
    /// Qualified constructor, `<Result|ApiException>.<variant>`.
    pub constructor: String,
    /// Case data type.
    pub ty: NativeType,
    /// Kotlin spelling of `ty`.
    pub kotlin_type: String,
    /// Codec schema of the media value, when one exists.
    pub schema: Option<SchemaId>,
    /// Codec object for `schema`.
    pub codec_name: Option<String>,
    /// Media decoded by this case; `None` for undeclared or absent bodies.
    pub media: Option<PlannedMedia>,
    /// Generated response headers class, when headers are declared.
    pub headers_type: Option<String>,
    /// Response header fields in declaration order.
    pub headers: Vec<PlannedHeader>,
    /// Whether the body is declared, undeclared bounded bytes, or forbidden.
    pub disposition: wire::ResponseBodyDisposition,
    /// Whether `media` is a stream representation.
    pub stream: bool,
    /// Index of the source response within the operation's protocol responses.
    pub response_index: usize,
    /// Index of `media` within that response's media, when declared.
    pub media_index: Option<usize>,
}

/// Generated client method, input class and result/exception types for one operation.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Terminal source operation.
    pub source: SourceId,
    /// Source `operationId`.
    pub operation_id: String,
    /// Suspending client method.
    pub method_name: String,
    /// Generated input class.
    pub input_type: String,
    /// Input constructor; the input class name.
    pub constructor: String,
    /// Sealed result type over success cases.
    pub result_type: String,
    /// Kotlin type shared by every success case, when there is exactly one.
    pub result_data_type: Option<String>,
    /// Native type shared by every success case, when there is exactly one.
    pub result_data: Option<NativeType>,
    /// Generated API exception type over failure cases.
    pub error_type: String,
    /// Planned parameters in protocol order.
    pub parameters: Vec<PlannedParameter>,
    /// Planned request body, when declared.
    pub body: Option<PlannedBody>,
    /// One case per response status, success group and media.
    pub responses: Vec<PlannedResponse>,
    /// Whether any response case streams, so the method returns a `Flow`.
    pub flow: bool,
    /// Credentials across every security alternative, in requirement order.
    pub credentials: Vec<PlannedCredential>,
    /// Shared wire decisions for the operation.
    pub wire: wire::OperationPlan,
}
impl PlannedOperation {
    /// Whether every parameter and the body are optional, so the input has a
    /// no-argument default.
    pub fn input_has_default(&self) -> bool {
        self.parameters.iter().all(|p| !p.required)
            && self.body.as_ref().is_none_or(|b| !b.required)
    }
    /// Only the simple single-scheme view; full security is retained in `wire`.
    pub fn security_scheme_name(&self) -> &str {
        self.credentials
            .first()
            .map_or("", |c| c.scheme_name.as_str())
    }
}

/// Complete immutable Kotlin SDK plan for one selected source closure.
#[derive(Debug)]
pub struct Plan {
    contract: Arc<Contract>,
    config: SdkConfig,
    models: models::ModelPlan,
    operations: Vec<PlannedOperation>,
    credentials: BTreeMap<SourceId, PlannedCredential>,
    program: OwnedProgram,
    protocol: wire::ProtocolPlan,
    examples: ExamplePlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    pagination: Option<pagination::PaginationPlan>,
    oauth: Option<wire::OAuthPlan>,
    /// The compiled, infallible typed-stream semantics and the lowered
    /// emission for exactly the discriminated SSE stream operations.
    stream_events: stream_events::StreamEventsPlan,
    /// The compiled incoming webhook/callback receipts over the whole
    /// contract. Compiled unconditionally; emission is conditional on
    /// receipts being declared at all.
    incoming: wire::IncomingPlan,
    /// The lowered, codec-bound emission entries for the incoming receipts.
    incoming_entries: Vec<incoming::IncomingEntry>,
}
/// Backend-uniform name for [`Plan`].
pub type SdkPlan = Plan;
impl Plan {
    /// Original retained source contract.
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// Packaging identity and options the plan was built with.
    pub fn config(&self) -> &SdkConfig {
        &self.config
    }
    /// Native model declarations and codec names.
    pub fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    /// Planned client operations in protocol order.
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    /// Client credential fields keyed by terminal security scheme.
    pub fn credentials(&self) -> &BTreeMap<SourceId, PlannedCredential> {
        &self.credentials
    }
    /// Source-bound runtime environment policy; never contains environment values.
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub(crate) fn credential_source<'a>(
        &self,
        requirement: &'a wire::CredentialRequirement,
    ) -> &'a SourceId {
        requirement.scheme().terminal().source()
    }
    /// Checked instruction graph and source bindings.
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    /// Retained rich source-backed HTTP decisions and codec roots.
    pub fn protocol(&self) -> &wire::ProtocolPlan {
        &self.protocol
    }
    /// Validated examples and original findings, including invalid declarations.
    pub fn examples(&self) -> &ExamplePlan {
        &self.examples
    }
    /// The compiled pagination selection carried by this plan, when SDK
    /// defaults were configured.
    pub fn pagination(&self) -> Option<&pagination::PaginationPlan> {
        self.pagination.as_ref()
    }
    /// The compiled OAuth lifecycle selection carried by this plan. `Some`
    /// exactly when SDK defaults were configured and at least one source
    /// scheme carries an executable flow, so `OAuth.kt` is emitted.
    pub fn oauth(&self) -> Option<&wire::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// The compiled typed-stream semantics carried by this plan, with the
    /// lowered emission for exactly the discriminated SSE stream operations.
    /// Compiled unconditionally; emission is conditional on the compiled
    /// discrimination evidence.
    pub fn stream_events(&self) -> &stream_events::StreamEventsPlan {
        &self.stream_events
    }
    /// The compiled incoming webhook/callback receipts over the whole
    /// contract. Compiled unconditionally; emission is conditional on
    /// receipts being declared at all.
    pub fn incoming(&self) -> &wire::IncomingPlan {
        &self.incoming
    }
    /// The lowered, codec-bound emission entries for the incoming receipts.
    pub fn incoming_entries(&self) -> &[incoming::IncomingEntry] {
        &self.incoming_entries
    }
    /// Deterministic Maven project, sources, docs and examples for this plan.
    ///
    /// # Errors
    ///
    /// Returns diagnostics when validation admission or native sample planning
    /// refuses the plan.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
        emit::package(self)
    }
}

/// Native adapter fence. Unsupported directions/representations have additional
/// located native admission guards; compatibility profiles are never implicit.
pub fn capabilities() -> wire::Capabilities {
    protocol::capabilities()
}
/// Plans a Kotlin SDK for `selected` operations without compatibility profiles.
///
/// # Errors
///
/// Returns located diagnostics when protocol admission, native naming or
/// model planning refuses the selected closure.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    plan_sdk_with_profiles(contract, selected, config, &Default::default())
}

/// Explicit interpretation entry point used by the shared backend registry.
/// Native protocol support never implicitly opts into compatibility semantics.
pub fn plan_sdk_with_profiles(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    profiles: &std::collections::BTreeSet<wire::CompatibilityProfile>,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, profiles, true, false)
}

/// Explicit canonical-resource and dynamic-reference SDK compilation.
/// It does not acquire documents or interpret a dynamic fallback as static.
pub fn plan_sdk_v3(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, &Default::default(), true, true)
}
/// Renders a planned SDK; equivalent to [`Plan::render`].
///
/// # Errors
///
/// Returns diagnostics when validation admission or native sample planning
/// refuses the plan.
pub fn emit_sdk(plan: &Plan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    plan.render()
}

fn valid_coordinate(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn valid_package(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !["java", "kotlin"].contains(&s.split('.').next().unwrap_or(""))
        && s.split('.').all(|part| {
            !models::keyword(part)
                && part.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}
pub(super) fn diagnostic(
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
