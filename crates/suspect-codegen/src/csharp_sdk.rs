//! Source-selected .NET SDKs over the canonical, capability-checked HTTP protocol.
pub use crate::http_contract::HttpDiagnostic;
use crate::{OutFile, examples, http_protocol};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedProgram;

mod credential_env;
#[cfg(test)]
mod credential_env_tests;
mod docs;
mod emit;
mod http;
mod incoming;
pub mod models;
mod oauth;
mod pagination;
mod positional;
pub mod protocol;
mod resources;
#[cfg(test)]
mod resources_tests;
mod samples;
mod scoped_examples;
#[cfg(test)]
mod server_tests;
mod stream_events;
mod validation;
#[cfg(test)]
mod validation_tests;

/// NuGet identity and native namespace; these do not alter service semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkConfig {
    /// NuGet package id (ASCII alphanumerics and `._-`, at most 100 bytes).
    pub name: String,
    /// NuGet package version.
    pub version: String,
    /// Dotted C# namespace for generated types; keywords are refused.
    pub namespace: String,
}
impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            name: "Generated.Sdk".into(),
            version: "0.0.0".into(),
            namespace: "Generated.Sdk".into(),
        }
    }
}

/// Allocated native operation symbols and the immutable shared wire plan.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Operation use-site source location.
    pub source: SourceId,
    /// Declared `operationId`, or `METHOD path` when undeclared.
    pub operation_id: String,
    /// Allocated client method name (`<Stem>Async`).
    pub method_name: String,
    /// Generated input type name.
    pub input_type: String,
    /// Generated result type name.
    pub result_type: String,
    /// Generated API exception type name (`<Stem>ApiException`).
    pub error_type: String,
    /// HTTP method token.
    pub http_method: String,
    /// Declared path template.
    pub path: String,
    /// Declared operation description; empty when absent.
    pub description: String,
    /// Shared wire plan for the operation.
    pub wire: http_protocol::OperationPlan,
    /// Planned parameters in declaration order.
    pub parameters: Vec<PlannedParameter>,
    /// Planned request body, if declared.
    pub body: Option<PlannedBody>,
    /// Planned responses in declaration order.
    pub responses: Vec<PlannedResponse>,
}
impl PlannedOperation {
    /// Whether no parameter or body is required, so a no-input overload is emitted.
    #[must_use]
    pub fn has_no_input_overload(&self) -> bool {
        !self.parameters.iter().any(|p| p.required)
            && !self.body.as_ref().is_some_and(|b| b.required)
    }
    /// Whether exactly one response can succeed, so the result is that response's type.
    #[must_use]
    pub fn direct_result(&self) -> bool {
        self.responses.iter().filter(|r| r.may_succeed()).count() == 1
    }
}
/// A planned operation parameter, exposed as an input property.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Allocated input property name.
    pub property_name: String,
    /// Declaring use-site source location.
    pub source: SourceId,
    /// Schema of the parameter codec.
    pub schema: SchemaId,
    /// Spelled C# type of the property.
    pub native_type: String,
    /// Declared parameter name sent on the wire.
    pub wire_name: String,
    /// Where the parameter is sent.
    pub location: http_protocol::ParameterLocation,
    /// Whether the parameter is required.
    pub required: bool,
    /// Shared wire plan for the parameter.
    pub wire: http_protocol::ParameterPlan,
}
/// A planned operation request body.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Declaring use-site source location.
    pub source: SourceId,
    /// Spelled C# body type; a generated union when several media types are declared.
    pub native_type: String,
    /// Whether the body is required.
    pub required: bool,
    /// Shared wire plan for the body.
    pub wire: http_protocol::BodyPlan,
    /// Planned media types in declaration order.
    pub media: Vec<protocol::PlannedMedia>,
    /// Whether the body is a generated union over media types.
    pub union: bool,
}
/// A planned response for one declared status, status range or default.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Declaring use-site source location.
    pub source: SourceId,
    /// Spelled C# payload type: a generated union, a media type's type, `byte[]` or `HttpNoContent`.
    pub native_type: String,
    /// Success result type: the result type itself when it is the only success, else a nested `<Result>.<Status>` case.
    pub type_name: String,
    /// Nested error case type (`<ApiException>.<Status>`).
    pub error_type_name: String,
    /// Shared wire plan for the response.
    pub wire: http_protocol::ResponsePlan,
    /// Planned media types; empty when the response is always empty.
    pub media: Vec<protocol::PlannedMedia>,
    /// Planned response headers.
    pub headers: Vec<protocol::PlannedHeader>,
    /// Generated headers type, when any headers are declared.
    pub header_type: Option<String>,
    /// Whether the payload is a generated union (several media types, or media plus an empty case).
    pub union: bool,
    /// Whether a body is never present: HEAD, 1xx, 204 or 304.
    pub always_empty: bool,
    /// Whether a body may be absent: always-empty, or a 2XX/3XX/default selector.
    pub may_be_empty: bool,
}
impl PlannedResponse {
    /// Whether this response can land in the success result (2xx, 2XX or default).
    #[must_use]
    pub fn may_succeed(&self) -> bool {
        matches!(
            self.wire.status(),
            http_protocol::ResponseStatus::Exact(200..=299)
                | http_protocol::ResponseStatus::Range(2)
                | http_protocol::ResponseStatus::Default
        )
    }
    /// Whether this response can land in the error result (anything but 2xx/2XX).
    #[must_use]
    pub fn may_fail(&self) -> bool {
        !matches!(
            self.wire.status(),
            http_protocol::ResponseStatus::Exact(200..=299)
                | http_protocol::ResponseStatus::Range(2)
        )
    }
}

/// Immutable selected-operation plan. Unsupported profiles fail before emission.
#[derive(Debug)]
pub struct SdkPlan {
    contract: Arc<Contract>,
    config: SdkConfig,
    attribution: Option<crate::attribution::AttributionDescriptor>,
    operations: Vec<PlannedOperation>,
    models: models::ModelPlan,
    credentials: BTreeMap<String, String>,
    credential_bindings: Vec<protocol::PlannedCredential>,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    pagination: Option<http_protocol::PaginationOutcome>,
    oauth: Option<http_protocol::OAuthPlan>,
    stream_semantics: http_protocol::StreamSemanticsPlan,
    incoming: http_protocol::IncomingPlan,
    protocol: http_protocol::ProtocolPlan,
    program: OwnedProgram,
    indices: BTreeMap<SchemaId, usize>,
    examples: examples::ExamplePlan,
    samples: samples::Samples,
}
impl SdkPlan {
    /// Planned operations in emission order.
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    /// Planned C# model types and codecs.
    #[must_use]
    pub const fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    /// Scheme-use identity to allocated property. Prefer credential_bindings for complete provenance.
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<String, String> {
        &self.credentials
    }
    /// Planned credential bindings with full source provenance.
    #[must_use]
    pub fn credential_bindings(&self) -> &[protocol::PlannedCredential] {
        &self.credential_bindings
    }
    /// Explicit variable-name policy bound to the admitted source declarations.
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub(crate) fn credential_key(
        &self,
        requirement: &http_protocol::CredentialRequirement,
    ) -> String {
        credential_env::key(requirement, self.credential_env.is_some())
    }
    /// The canonical HTTP protocol plan the operations were lowered from.
    #[must_use]
    pub const fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
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
    /// Compiled typed-stream semantics over the admitted protocol plan. The
    /// planner is infallible and every entry carries its own evidence; only
    /// discriminated SSE operations lower into emitted typed events members.
    #[must_use]
    pub fn stream_semantics(&self) -> &http_protocol::StreamSemanticsPlan {
        &self.stream_semantics
    }
    /// Compiled incoming webhook/callback receipts over the whole contract.
    /// Their declared codec roots already extend the codec table; a plan
    /// without any declared receipt emits nothing at all.
    #[must_use]
    pub fn incoming(&self) -> &http_protocol::IncomingPlan {
        &self.incoming
    }
    /// `ua/v1` attribution constants compiled from package identity and source.
    #[must_use]
    pub fn attribution(&self) -> Option<&crate::attribution::AttributionDescriptor> {
        self.attribution.as_ref()
    }
    /// Package identity the plan was admitted under.
    #[must_use]
    pub const fn config(&self) -> &SdkConfig {
        &self.config
    }
    /// The admitted contract this plan was built from.
    #[must_use]
    pub const fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// The compiled schema validation program embedded in the package.
    #[must_use]
    pub const fn program(&self) -> &OwnedProgram {
        &self.program
    }
    /// Compiled request/response examples for documentation and samples.
    #[must_use]
    pub const fn examples(&self) -> &examples::ExamplePlan {
        &self.examples
    }
    /// Emit all NuGet source, native reference and executable example artifacts.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
        emit::package(self)
    }
}
/// Plan the exact source-selected HTTP closure and its actual JSON/text codec roots.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    protocol::plan(
        contract,
        selected,
        config,
        protocol::ProtocolOptions::default(),
    )
}
/// Explicit versioned interpretation and runtime credential-default policies.
/// Ordinary generation never infers environment names or legacy wire conventions.
pub fn plan_sdk_with_options(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    options: protocol::ProtocolOptions,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, options)
}
fn exported(value: &str) -> String {
    crate::rust_models::pascal(value)
}
fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}
fn reserved_types() -> Vec<&'static str> {
    "Client Credentials ClientOptions RequestOptions Codecs JsonNumber JsonInteger JsonNull Never Optional Quickstart ExampleHandler CodecException CodecErrorKind CodecRuntime ConversionContext ValidationSession ValidationProgram ExactNumber JsonRuntime LimitedStream ResponseMetadata WireRequest ApiException UnexpectedResponseException SdkException SdkErrorKind HttpRuntime WireResponse System Task CancellationToken HttpClient HttpRequestMessage HttpResponseMessage Uri String Object Array Enum Exception Math Convert Encoding JsonElement Utf8JsonWriter JsonValueKind BigInteger CultureInfo NumberStyles TimeSpan Stream MemoryStream List Dictionary HashSet ReadOnlyMemory Span ReadOnlySpan Action Func IDisposable StringComparer StringComparison InvalidOperationException ArgumentException ArgumentNullException ArgumentOutOfRangeException FormatException OverflowException ObjectDisposedException NotSupportedException IOException OperationCanceledException CancellationTokenSource Timeout HttpMethod HttpStatusCode HttpContent HttpMessageHandler HttpCompletionOption HttpRequestException HttpRequestError TimeoutException SocketsHttpHandler DecompressionMethods MediaTypeHeaderValue MediaTypeWithQualityHeaderValue AuthenticationHeaderValue ByteArrayContent ReadOnlyDictionary StringBuilder UriKind UriCreationOptions TaskContinuationOptions TaskScheduler TaskStatus ReferenceEqualityComparer JsonDocument JsonDocumentOptions JsonReaderOptions Utf8JsonReader JsonTokenType JsonWriterOptions UTF8Encoding DecoderFallbackException EncoderFallbackException IReadOnlyDictionary IReadOnlyList IEquatable IComparable IEnumerable Enumerable Stack Interlocked Volatile Console ValueTask IAsyncDisposable IAsyncEnumerable IAsyncEnumerator EnumeratorCancellationAttribute ProtocolData ProtocolRuntime WireEncoding MediaRuntime PartsRuntime HttpNoContent HttpStream StreamLease BasicCredential AuthorizationValue AuthorizationProvider CredentialContext OAuthFlowInfo ServerInfo ServerVariableInfo OperationInfo ResponseLinkInfo MultipartValue EncodedBody PartBag RawPart NativeProtocolPlan Rune Guid ReadOnlyCollection JsonNode JsonObject JsonArray JsonValue NullabilityInfoContext Pagination PaginationException PaginationDescriptor PaginationDescriptors".split_whitespace().collect()
}
fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.split('.').all(|part| {
            !part.is_empty()
                && (part.as_bytes()[0].is_ascii_alphabetic() || part.starts_with('_'))
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !CS_KEYWORDS.split_whitespace().any(|k| k == part)
        })
}
const CS_KEYWORDS: &str = "abstract as base bool break byte case catch char checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly ref return sbyte sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using virtual void volatile while record required file global __arglist __makeref __refvalue __reftype";
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
