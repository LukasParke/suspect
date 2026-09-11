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
pub mod models;
mod positional;
pub mod protocol;
mod resources;
#[cfg(test)]
mod resources_tests;
mod samples;
mod scoped_examples;
#[cfg(test)]
mod server_tests;
mod validation;
#[cfg(test)]
mod validation_tests;

/// NuGet identity and native namespace; these do not alter service semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkConfig {
    pub name: String,
    pub version: String,
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
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub input_type: String,
    pub result_type: String,
    pub error_type: String,
    pub http_method: String,
    pub path: String,
    pub description: String,
    pub wire: http_protocol::OperationPlan,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
}
impl PlannedOperation {
    #[must_use]
    pub fn has_no_input_overload(&self) -> bool {
        !self.parameters.iter().any(|p| p.required)
            && !self.body.as_ref().is_some_and(|b| b.required)
    }
    #[must_use]
    pub fn direct_result(&self) -> bool {
        self.responses.iter().filter(|r| r.may_succeed()).count() == 1
    }
}
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub property_name: String,
    pub source: SourceId,
    pub schema: SchemaId,
    pub native_type: String,
    pub wire_name: String,
    pub location: http_protocol::ParameterLocation,
    pub required: bool,
    pub wire: http_protocol::ParameterPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub native_type: String,
    pub required: bool,
    pub wire: http_protocol::BodyPlan,
    pub media: Vec<protocol::PlannedMedia>,
    pub union: bool,
}
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub source: SourceId,
    pub native_type: String,
    pub type_name: String,
    pub error_type_name: String,
    pub wire: http_protocol::ResponsePlan,
    pub media: Vec<protocol::PlannedMedia>,
    pub headers: Vec<protocol::PlannedHeader>,
    pub header_type: Option<String>,
    pub union: bool,
    pub always_empty: bool,
    pub may_be_empty: bool,
}
impl PlannedResponse {
    #[must_use]
    pub fn may_succeed(&self) -> bool {
        matches!(
            self.wire.status(),
            http_protocol::ResponseStatus::Exact(200..=299)
                | http_protocol::ResponseStatus::Range(2)
                | http_protocol::ResponseStatus::Default
        )
    }
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
    operations: Vec<PlannedOperation>,
    models: models::ModelPlan,
    credentials: BTreeMap<String, String>,
    credential_bindings: Vec<protocol::PlannedCredential>,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    protocol: http_protocol::ProtocolPlan,
    program: OwnedProgram,
    indices: BTreeMap<SchemaId, usize>,
    examples: examples::ExamplePlan,
    samples: samples::Samples,
}
impl SdkPlan {
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub const fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    /// Scheme-use identity to allocated property. Prefer credential_bindings for complete provenance.
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<String, String> {
        &self.credentials
    }
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
    #[must_use]
    pub const fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    #[must_use]
    pub const fn package(&self) -> &SdkConfig {
        &self.config
    }
    #[must_use]
    pub const fn config(&self) -> &SdkConfig {
        &self.config
    }
    #[must_use]
    pub const fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub const fn program(&self) -> &OwnedProgram {
        &self.program
    }
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
    "Client Credentials ClientOptions RequestOptions Codecs JsonNumber JsonInteger JsonNull Never Optional Quickstart ExampleHandler CodecException CodecErrorKind CodecRuntime ConversionContext ValidationSession ValidationProgram ExactNumber JsonRuntime LimitedStream ResponseMetadata WireRequest ApiException UnexpectedResponseException SdkException SdkErrorKind HttpRuntime WireResponse System Task CancellationToken HttpClient HttpRequestMessage HttpResponseMessage Uri String Object Array Enum Exception Math Convert Encoding JsonElement Utf8JsonWriter JsonValueKind BigInteger CultureInfo NumberStyles TimeSpan Stream MemoryStream List Dictionary HashSet ReadOnlyMemory Span ReadOnlySpan Action Func IDisposable StringComparer StringComparison InvalidOperationException ArgumentException ArgumentNullException ArgumentOutOfRangeException FormatException OverflowException ObjectDisposedException NotSupportedException IOException OperationCanceledException CancellationTokenSource Timeout HttpMethod HttpStatusCode HttpContent HttpMessageHandler HttpCompletionOption HttpRequestException HttpRequestError TimeoutException SocketsHttpHandler DecompressionMethods MediaTypeHeaderValue MediaTypeWithQualityHeaderValue AuthenticationHeaderValue ByteArrayContent ReadOnlyDictionary StringBuilder UriKind UriCreationOptions TaskContinuationOptions TaskScheduler TaskStatus ReferenceEqualityComparer JsonDocument JsonDocumentOptions JsonReaderOptions Utf8JsonReader JsonTokenType JsonWriterOptions UTF8Encoding DecoderFallbackException EncoderFallbackException IReadOnlyDictionary IReadOnlyList IEquatable IComparable IEnumerable Enumerable Stack Interlocked Volatile Console ValueTask IAsyncDisposable IAsyncEnumerable IAsyncEnumerator EnumeratorCancellationAttribute ProtocolData ProtocolRuntime WireEncoding MediaRuntime PartsRuntime HttpNoContent HttpStream StreamLease BasicCredential AuthorizationValue AuthorizationProvider CredentialContext OAuthFlowInfo ServerInfo ServerVariableInfo OperationInfo ResponseLinkInfo MultipartValue EncodedBody PartBag RawPart NativeProtocolPlan Rune Guid ReadOnlyCollection JsonNode JsonObject JsonArray JsonValue NullabilityInfoContext".split_whitespace().collect()
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
