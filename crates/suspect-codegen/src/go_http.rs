//! Source-selected Go HTTP clients using only the standard library `net/http`.
//!
//! HTTP admission and wire semantics come from `http_protocol`. Native names,
//! typed inputs, transport policy, packaging and
//! documentation are planned here. Directional (`readOnly`/`writeOnly`)
//! annotations are explicitly unsupported in the initial Go HTTP profile and
//! produce source-linked diagnostics.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{OutFile, http_protocol as protocol};

mod credential_env;
mod descriptors;
mod docs;
mod emit;
pub mod incoming;
mod native_examples;
mod oauth;
pub mod pagination;
mod planning;
pub mod stream_events;

/// Finite transport and codec policy embedded in the generated package.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Shared model validation and exact JSON conversion policy (go_codecs).
    pub codecs: crate::go_codecs::CodecConfig,
    /// Generated response ceiling; callers may only lower it.
    pub max_response_bytes: usize,
    /// Ceiling for an assembled URL and for a serialized request body.
    pub max_request_bytes: usize,
    /// Independent in-memory part ceiling, checked before copying a part.
    pub max_part_bytes: usize,
    /// Finite number of form fields / MIME parts, including repeated expansions.
    pub max_parts: usize,
    /// Independent parsed stream-item ceiling.
    pub max_stream_item_bytes: usize,
    /// Explicit versioned compatibility policies; ordinary OAS semantics are the default.
    pub compatibility_profiles: BTreeSet<protocol::CompatibilityProfile>,
    /// Explicit source-scheme to runtime environment VARIABLE NAME policy.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            codecs: crate::go_codecs::CodecConfig::default(),
            max_response_bytes: 8 * 1024 * 1024,
            max_request_bytes: 8 * 1024 * 1024,
            max_part_bytes: 1024 * 1024,
            max_parts: 4096,
            max_stream_item_bytes: 64 * 1024,
            compatibility_profiles: BTreeSet::new(),
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
    /// Exported Go method name on the Client, e.g. `GetCredits`.
    pub method_name: String,
    /// Exported Go input struct name, e.g. `GetCreditsInput`.
    pub input_type: String,
    /// Allocated package-level constructor for the required operation inputs.
    pub input_constructor: String,
    /// Exported exact-status success payload type, e.g. `GetCreditsResult`.
    pub success_type: String,
    /// Typed per-status API error variant, e.g. `GetCreditsApiError`.
    pub error_variant: String,
    /// Direct-data convenience method for one unambiguous successful representation.
    pub data_method: Option<String>,
    /// Whether the call accepts zero or one input (all members are optional).
    pub optional_input: bool,
    wire: protocol::OperationPlan,
    parameters: Vec<PlannedParameter>,
    body: Option<PlannedBody>,
    responses: Vec<PlannedResponse>,
}

impl PlannedOperation {
    /// Allocated input members in source parameter order.
    #[must_use]
    pub fn parameters(&self) -> &[PlannedParameter] {
        &self.parameters
    }
    /// The request body's allocated input member, when present.
    #[must_use]
    pub fn body(&self) -> Option<&PlannedBody> {
        self.body.as_ref()
    }
    /// Allocated concrete success and API error types in source status order.
    #[must_use]
    pub fn responses(&self) -> &[PlannedResponse] {
        &self.responses
    }
    #[must_use]
    pub const fn wire(&self) -> &protocol::OperationPlan {
        &self.wire
    }
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    wire: protocol::ParameterPlan,
    /// Exported Go field name inside the input struct.
    pub field_name: String,
    /// Allocated fluent setter, present only for optional parameters.
    pub setter_name: Option<String>,
}

impl PlannedParameter {
    /// Canonical schema identity for this input member's native model.
    #[must_use]
    pub fn schema(&self) -> &SchemaId {
        self.wire.codec().schema().id()
    }
    #[must_use]
    pub const fn wire(&self) -> &protocol::ParameterPlan {
        &self.wire
    }
}

/// One request body's retained native input names and wire identity.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    wire: protocol::BodyPlan,
    /// Exported Go field name inside the input struct.
    pub field_name: String,
    /// Allocated fluent setter, present only for an optional request body.
    pub setter_name: Option<String>,
    /// Exact Go type of the body input, including a closed media choice when needed.
    pub native_type: String,
    media: Vec<PlannedMedia>,
}

impl PlannedBody {
    /// Canonical schema identity for the body member's native model.
    #[must_use]
    pub fn schema(&self) -> Option<&SchemaId> {
        (self.media.len() == 1)
            .then(|| self.media[0].schema())
            .flatten()
    }
    #[must_use]
    pub const fn wire(&self) -> &protocol::BodyPlan {
        &self.wire
    }
    #[must_use]
    pub fn media(&self) -> &[PlannedMedia] {
        &self.media
    }
    #[must_use]
    pub fn is_choice(&self) -> bool {
        self.media.len() > 1
    }
}

/// One status/media representation with its allocated package-level Go type.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Concrete exported response wrapper, shared by code, docs and consumers.
    pub type_name: String,
    pub native_type: String,
    pub headers_type: String,
    pub headers: Vec<PlannedHeader>,
    /// Response index in the shared operation descriptor.
    pub response_index: usize,
    pub media_index: Option<usize>,
    pub forbidden_body: bool,
    wire: protocol::ResponsePlan,
    media: Option<PlannedMedia>,
}

impl PlannedResponse {
    /// Source-declared selector, independent of the actual response status.
    #[must_use]
    pub fn status(&self) -> protocol::ResponseStatus {
        self.wire.status()
    }
    /// Canonical schema identity for this response's native payload model.
    #[must_use]
    pub fn schema(&self) -> Option<&SchemaId> {
        self.media.as_ref().and_then(PlannedMedia::schema)
    }
    #[must_use]
    pub const fn wire(&self) -> &protocol::ResponsePlan {
        &self.wire
    }
    #[must_use]
    pub fn media(&self) -> Option<&PlannedMedia> {
        self.media.as_ref()
    }
    #[must_use]
    pub fn can_succeed(&self) -> bool {
        matches!(
            self.status(),
            protocol::ResponseStatus::Exact(200..=299)
                | protocol::ResponseStatus::Range(2)
                | protocol::ResponseStatus::Default
        )
    }
    #[must_use]
    pub fn can_fail(&self) -> bool {
        !matches!(
            self.status(),
            protocol::ResponseStatus::Exact(200..=299) | protocol::ResponseStatus::Range(2)
        )
    }
}

/// Native representation of one actual shared MediaPlan.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub native_type: String,
    /// Allocated only for a member of a closed request-body media choice.
    pub choice_type: String,
    /// Allocated only for that closed request-body media choice.
    pub constructor: String,
    pub aggregate: Option<PlannedAggregate>,
    wire: protocol::MediaPlan,
}
impl PlannedMedia {
    #[must_use]
    pub const fn wire(&self) -> &protocol::MediaPlan {
        &self.wire
    }
    /// An actual JSON/text/item codec input. Binary and structural aggregates have none.
    #[must_use]
    pub fn schema(&self) -> Option<&SchemaId> {
        match self.wire.representation() {
            protocol::Representation::Json { codec }
            | protocol::Representation::Text { codec, .. } => {
                codec.as_ref().map(|c| c.schema().id())
            }
            protocol::Representation::Stream { stream } => {
                stream.item_codec().map(|codec| codec.schema().id())
            }
            _ => None,
        }
    }
    #[must_use]
    pub fn requires_content_type(&self) -> bool {
        !matches!(
            self.wire.media_type().range(),
            protocol::MediaRange::Concrete { .. }
        )
    }
}

/// Structurally validated form or finite multipart input/output, never a JSON aggregate.
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    pub type_name: String,
    pub constructor: String,
    pub multipart: bool,
    pub positional: bool,
    pub parts: Vec<PlannedPart>,
    pub additional: Option<PlannedPart>,
}
#[derive(Debug, Clone)]
pub struct PlannedPart {
    pub field_name: String,
    pub setter_name: Option<String>,
    pub data_type: String,
    pub native_type: String,
    pub wire: protocol::PartPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub field_name: String,
    pub wire: protocol::HeaderPlan,
}

/// Immutable complete selected-operation plan. It does not certify a whole SDK.
#[derive(Debug)]
pub struct HttpPlan {
    contract: Arc<Contract>,
    operations: Vec<PlannedOperation>,
    codecs: crate::go_codecs::CodecPlan,
    symbols: BTreeMap<SchemaId, String>,
    credentials: BTreeMap<String, String>,
    config: HttpConfig,
    examples: crate::examples::ExamplePlan,
    protocol: protocol::ProtocolPlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    credential_env_factory: Option<String>,
    pagination: Option<pagination::PaginationPlan>,
    oauth: Option<protocol::OAuthPlan>,
    stream_events: Option<stream_events::StreamEventsPlan>,
    incoming: protocol::IncomingPlan,
    incoming_receipts: Vec<incoming::IncomingReceipt>,
}

impl HttpPlan {
    /// Generated finite native transport and codec policy.
    #[must_use]
    pub const fn config(&self) -> &HttpConfig {
        &self.config
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn codecs(&self) -> &crate::go_codecs::CodecPlan {
        &self.codecs
    }
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Native model symbol names keyed by schema source.
    #[must_use]
    pub fn symbols(&self) -> &BTreeMap<SchemaId, String> {
        &self.symbols
    }
    /// Security scheme name to allocated package-level credential constructor.
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<String, String> {
        &self.credentials
    }
    /// Source-bound runtime environment policy; generation never reads values.
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
    /// Allocated Go factory name, present only for a configured environment policy.
    #[must_use]
    pub fn credential_env_factory(&self) -> Option<&str> {
        self.credential_env_factory.as_deref()
    }
    /// The compiled pagination selection carried by this plan, when a policy is
    /// configured. Helpers are emitted only when [`PaginationPlan::operations`]
    /// is non-empty, so no-policy output stays byte-identical.
    #[must_use]
    pub fn pagination(&self) -> Option<&pagination::PaginationPlan> {
        self.pagination.as_ref()
    }
    /// The compiled OAuth/OIDC lifecycle plan, carried only when a configured
    /// policy yields schemes. The generated lifecycle runtime is emitted only
    /// for usable schemes, so no-policy output stays byte-identical.
    #[must_use]
    pub fn oauth(&self) -> Option<&protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// The compiled typed-event subset of the stream semantics plan, carried
    /// only when an operation will emit a typed events iterator. Computed
    /// unconditionally during planning; storing stays conditional so
    /// no-stream plans stay cheap and their output byte-identical.
    #[must_use]
    pub fn stream_events(&self) -> Option<&stream_events::StreamEventsPlan> {
        self.stream_events.as_ref()
    }
    /// The compiled incoming webhook/callback receipt plan over the whole
    /// contract. Planning walks the whole Contract (selection-independent) and
    /// fails the plan on broken incoming declarations like every other
    /// diagnostic.
    #[must_use]
    pub fn incoming(&self) -> &protocol::IncomingPlan {
        &self.incoming
    }
    /// Emission-ready incoming receipt helpers. Receipts the Go v1 helpers
    /// cannot express surface as plan errors, so the helpers exist exactly
    /// when the compiled plan carries receipts and `go/incoming.go` is emitted
    /// exactly when these are non-empty; receipt-less output stays
    /// byte-identical.
    #[must_use]
    pub fn incoming_receipts(&self) -> &[incoming::IncomingReceipt] {
        &self.incoming_receipts
    }
    /// The single admitted shared protocol plan, including located annotations.
    #[must_use]
    pub const fn protocol(&self) -> &protocol::ProtocolPlan {
        &self.protocol
    }
    /// Render a Go module using the default package identity.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        emit_http(self, &PackageConfig::default()).expect("default package identity is valid")
    }
}

/// Go module identity, independent of every API semantic and source version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Module path, e.g. `example.com/team/generated-sdk`.
    pub module_path: String,
    /// Importable package name; default `sdk`.
    pub package_name: String,
    /// An exact SemVer; version requirements are not accepted.
    pub version: String,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            module_path: "example.com/generated-sdk".into(),
            package_name: "sdk".into(),
            version: "0.0.0".into(),
        }
    }
}

/// Validated Go module path admission failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePathError;

impl std::fmt::Display for ModulePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid Go module path")
    }
}
impl std::error::Error for ModulePathError {}

/// Validated Go package name admission failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageNameError;

impl std::fmt::Display for PackageNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid Go package name")
    }
}
impl std::error::Error for PackageNameError {}

/// Validated exact SemVer admission failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionError;

impl std::fmt::Display for VersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid exact package version")
    }
}
impl std::error::Error for VersionError {}

/// Emit a Go module: go.mod with the configured module identity, package
/// sources, generated docs, examples and manifest.
///
/// # Errors
/// Invalid module path, package name or version fails before any artifact is
/// returned or written.
pub fn emit_http(plan: &HttpPlan, package: &PackageConfig) -> Result<Vec<OutFile>, Vec<String>> {
    let mut errors = Vec::new();
    if !valid_module_path(&package.module_path) {
        errors.push(ModulePathError.to_string());
    }
    if !valid_package_name(&package.package_name) || package.package_name != "sdk" {
        errors.push(PackageNameError.to_string());
    }
    if semver::Version::parse(&package.version).is_err() {
        errors.push(VersionError.to_string());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(emit::package(plan, package))
}

/// Go module path admission mirrors `go mod` rules: no scheme, a dotted domain
/// first, lower-case letters, digits, `-`, `.`, `_`, no `..` elements, no
/// leading/trailing separators.
fn valid_module_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 256
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.contains("..")
        || path
            .split('/')
            .next()
            .is_none_or(|host| !host.contains('.') || host.split('.').any(|el| el.is_empty()))
    {
        return false;
    }
    path.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_' | '/'))
        && path
            .split('/')
            .all(|el| !el.starts_with('.') && !el.ends_with('-') && !el.ends_with('_'))
}

/// Go package names must be legal identifiers and not reserved keywords so
/// imports never need renaming.
fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !is_go_keyword(name)
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

/// Plan exactly the selected outgoing operations using shared source semantics.
///
/// # Errors
/// Unsupported declarations, directional models and resource policies produce
/// source-linked findings before package emission.
pub fn plan_http(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    planning::plan(contract, selected, config)
}

// All source-derived identifiers start with an ASCII uppercase letter, so they
// cannot collide with private runtime names or imports. This is the exported
// package namespace of the copied JSON/model/validation/codec/HTTP runtimes.
const RUNTIME_NAMES: &[&str] = &[
    "Value",
    "Number",
    "Integer",
    "JSONErrorKind",
    "JSONSyntax",
    "JSONDuplicateName",
    "JSONBadUnicode",
    "JSONLimit",
    "JSONCycle",
    "JSONType",
    "JSONError",
    "Limits",
    "DefaultLimits",
    "ParseNumber",
    "ParseInteger",
    "Parse",
    "Encode",
    "Nullable",
    "NullableNull",
    "NullableValue",
    "Optional",
    "OptionalAbsent",
    "OptionalSome",
    "Presence",
    "PresenceMissing",
    "PresenceNull",
    "PresenceSome",
    "ValidationSource",
    "ValidationError",
    "ValidationFinding",
    "Validate",
    "CodecError",
    "Codec",
    "Codecs",
    "HTTPSource",
    "SDKError",
    "Doer",
    "Credentials",
    "ClientOptions",
    "Client",
    "NewClient",
    "APIResponse",
    "Content",
    "Part",
    "NewPart",
    "NoContent",
    "Stream",
    "Link",
    "LocatedValue",
    "CredentialRequest",
    "CredentialHook",
    "OAuthFlow",
    "Server",
    "ServerVariable",
    "HTTPProvenance",
    "Authorization",
    "SecurityRequirement",
    "SecurityAlternative",
];

/// Reserve declarations from typed lowering, including names not represented by
/// a standalone GoSymbol (constructors, literals and closed-union variants).
/// `emitted` carries the package-level names of conditionally emitted runtime
/// files (currently `go/oauth.go`); an inconsistent model namespace must be
/// rejected at its source, never repaired by rewriting the already rendered
/// model or codec artifacts.
fn model_package_names(
    contract: &Contract,
    models: &crate::go_models::ModelPlan,
    emitted: &[&str],
) -> Result<BTreeSet<String>, Vec<HttpDiagnostic>> {
    use crate::go_models::GoDecl;

    let mut used: BTreeMap<String, Option<SourceId>> = RUNTIME_NAMES
        .iter()
        .chain(emitted.iter())
        .map(|name| ((*name).into(), None))
        .collect();
    let mut errors = Vec::new();
    let mut reserve = |name: String, source: &SourceId| {
        if let Some(previous) = used.get(&name) {
            let owner = previous.as_ref().map_or_else(
                || "the generated runtime".into(),
                |source| format!("{}#{}", source.document(), source.pointer()),
            );
            errors.push(diagnostic(
                contract,
                source.clone(),
                "http-go-package-name-collision",
                format!("Go model symbol {name} conflicts with {owner}; the model plan must allocate distinct package declarations"),
            ));
        } else {
            used.insert(name, Some(source.clone()));
        }
    };
    let descriptors = models.descriptors();
    for (key, declaration) in &descriptors.declarations {
        let name = &descriptors.names[key];
        reserve(name.clone(), &key.0);
        match declaration {
            GoDecl::Struct { .. } => reserve(format!("New{name}"), &key.0),
            GoDecl::Literals { values, .. } => {
                for value in values {
                    reserve(format!("{name}{}", value.name), &key.0);
                }
            }
            GoDecl::Union(variants) => {
                for variant in variants {
                    reserve(format!("{name}{}", variant.name), &variant.source);
                    reserve(format!("New{name}{}", variant.name), &variant.source);
                }
            }
            GoDecl::Alias(_) => {}
        }
    }
    if errors.is_empty() {
        Ok(used.into_keys().collect())
    } else {
        Err(errors)
    }
}

/// Exported Go identifier (UpperCamel) from a wire/source name.
fn exported(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if upper_next {
                out.extend(c.to_uppercase());
                upper_next = false;
            } else {
                out.push(c);
            }
        } else if c.is_ascii() {
            upper_next = true;
        } else {
            // Keep non-ASCII source identities distinguishable while emitting
            // portable, exported identifiers, including uncased Unicode letters.
            out.push_str(&format!("U{:x}", u32::from(c)));
            upper_next = true;
        }
    }
    if out.is_empty() {
        "Op".into()
    } else {
        if out.starts_with(|c: char| c.is_ascii_digit()) {
            out.insert_str(0, "Op");
        }
        out
    }
}

fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}{suffix}");
        suffix += 1;
    }
    name
}

/// Go reserved words (all versions incl. future-reserved); package names and
/// allocated identifiers never collide with them.
fn is_go_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "chan"
            | "const"
            | "continue"
            | "default"
            | "defer"
            | "else"
            | "fallthrough"
            | "for"
            | "func"
            | "go"
            | "goto"
            | "if"
            | "import"
            | "interface"
            | "map"
            | "package"
            | "range"
            | "return"
            | "select"
            | "struct"
            | "switch"
            | "type"
            | "var"
    )
}
