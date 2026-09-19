use std::{collections::BTreeMap, ops::Range};

use serde::{Serialize, Serializer};
use serde_json::Value;
use suspect_ir::contract::{SchemaId, SourceId};

use super::{ByteLimits, Capabilities, Capability, MediaType};

// Descriptors expose shared borrows, never mutable collections or setters.
macro_rules! getters {
    ($($name:ident : $ty:ty),* $(,)?) => { $(
        #[must_use]
        pub fn $name(&self) -> &$ty { &self.$name }
    )* };
}

pub(super) fn serialize_source_id<S: Serializer>(
    id: &SourceId,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut out = serializer.serialize_struct("SourceId", 2)?;
    out.serialize_field("document", id.document().as_str())?;
    out.serialize_field("pointer", id.pointer())?;
    out.end()
}
pub(super) fn serialize_roots<S: Serializer>(
    ids: &[SchemaId],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    struct Id<'a>(&'a SourceId);
    impl Serialize for Id<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            serialize_source_id(self.0, s)
        }
    }
    let mut seq = serializer.serialize_seq(Some(ids.len()))?;
    for id in ids {
        seq.serialize_element(&Id(id))?;
    }
    seq.end()
}

/// An actual source value and its byte span in the canonical retrieval document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    #[serde(serialize_with = "serialize_source_id")]
    pub(super) source: SourceId,
    pub(super) span: Range<usize>,
}
impl SourceLocation {
    getters!(source: SourceId);
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
}

/// Declaration/use-site and terminal definition are distinct, even when they
/// refer to byte-identical data. Reference hops are original source objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provenance {
    pub(super) use_site: SourceLocation,
    pub(super) terminal: SourceLocation,
    pub(super) references: Vec<SourceLocation>,
    pub(super) use_site_resource: Option<ResourceContext>,
    pub(super) terminal_resource: Option<ResourceContext>,
    pub(super) reference_resources: Vec<Option<ResourceContext>>,
}
impl Provenance {
    getters!(use_site: SourceLocation, terminal: SourceLocation);
    #[must_use]
    pub fn references(&self) -> &[SourceLocation] {
        &self.references
    }
    #[must_use]
    pub fn use_site_resource(&self) -> Option<&ResourceContext> {
        self.use_site_resource.as_ref()
    }
    #[must_use]
    pub fn terminal_resource(&self) -> Option<&ResourceContext> {
        self.terminal_resource.as_ref()
    }
    /// One lexical resource context per entry in `references()`, in the same order.
    #[must_use]
    pub fn reference_resources(&self) -> &[Option<ResourceContext>] {
        &self.reference_resources
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Document,
    OpenApiDocument,
    Schema,
}

/// Logical resource names and reference bases are separate from every physical
/// SourceId. `scope_address` belongs to the nearest registered source scope; it
/// is not a fabricated address for an unregistered keyword/annotation child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceContext {
    pub(super) source: SourceLocation,
    pub(super) resource: SourceLocation,
    pub(super) kind: ResourceKind,
    pub(super) canonical_uri: String,
    pub(super) base_uri: String,
    pub(super) base_source: Option<SourceLocation>,
    pub(super) scope_address: String,
    pub(super) schema_root: Option<SourceLocation>,
    pub(super) aliases: Vec<String>,
}
impl ResourceContext {
    getters!(source: SourceLocation, resource: SourceLocation);
    #[must_use]
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    #[must_use]
    pub fn canonical_uri(&self) -> &str {
        &self.canonical_uri
    }
    #[must_use]
    pub fn base_uri(&self) -> &str {
        &self.base_uri
    }
    #[must_use]
    pub fn base_source(&self) -> Option<&SourceLocation> {
        self.base_source.as_ref()
    }
    #[must_use]
    pub fn scope_address(&self) -> &str {
        &self.scope_address
    }
    #[must_use]
    pub fn schema_root(&self) -> Option<&SourceLocation> {
        self.schema_root.as_ref()
    }
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }
    /// Resolve a description URI (for example an Example.externalValue) without
    /// looking up, loading or fetching its destination. API URLs use ApiUrlBase.
    pub fn resolve_reference(
        &self,
        reference: &str,
    ) -> Result<String, suspect_ir::contract::resource_uri::Error> {
        suspect_ir::contract::resource_uri::resolve_reference(&self.base_uri, reference)
    }
}

/// A value plus its exact location. In particular, defaults and annotations
/// retain their own location rather than borrowing a nearby schema's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Located<T> {
    pub(super) source: SourceLocation,
    pub(super) value: T,
}
impl<T> Located<T> {
    getters!(source: SourceLocation, value: T);
}

/// Instance-ready and serialized examples remain distinct. None of these values
/// is walked as a schema, reference graph, credential, or transport instruction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExampleMetadata {
    pub(super) inline: Option<Located<Value>>,
    pub(super) named: Vec<ExamplePlan>,
}
impl ExampleMetadata {
    #[must_use]
    pub fn inline(&self) -> Option<&Located<Value>> {
        self.inline.as_ref()
    }
    #[must_use]
    pub fn named(&self) -> &[ExamplePlan] {
        &self.named
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExamplePlan {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) summary: Option<Located<String>>,
    pub(super) description: Option<Located<String>>,
    pub(super) value: Option<Located<Value>>,
    pub(super) data_value: Option<Located<Value>>,
    pub(super) serialized_value: Option<Located<String>>,
    pub(super) external_value: Option<Located<String>>,
}
impl ExamplePlan {
    getters!(source: Provenance);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn value(&self) -> Option<&Located<Value>> {
        self.value.as_ref()
    }
    #[must_use]
    pub fn data_value(&self) -> Option<&Located<Value>> {
        self.data_value.as_ref()
    }
    #[must_use]
    pub fn serialized_value(&self) -> Option<&Located<String>> {
        self.serialized_value.as_ref()
    }
    #[must_use]
    pub fn external_value(&self) -> Option<&Located<String>> {
        self.external_value.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticKind {
    InvalidSource,
    Unsupported,
    Capability,
    Annotation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub(super) source: SourceLocation,
    pub(super) code: &'static str,
    pub(super) severity: Severity,
    pub(super) kind: DiagnosticKind,
    pub(super) message: String,
    pub(super) capability: Option<Capability>,
    pub(super) related: Vec<SourceLocation>,
    pub(super) resource_context: Option<ResourceContext>,
}
impl Diagnostic {
    getters!(source: SourceLocation);
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }
    #[must_use]
    pub fn kind(&self) -> DiagnosticKind {
        self.kind
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    #[must_use]
    pub fn capability(&self) -> Option<Capability> {
        self.capability
    }
    #[must_use]
    pub fn related(&self) -> &[SourceLocation] {
        &self.related
    }
    #[must_use]
    pub fn resource_context(&self) -> Option<&ResourceContext> {
        self.resource_context.as_ref()
    }
}

/// Atomic plan for exactly the selected outgoing operations. Codec roots are
/// canonical indexed schema IDs, sorted and deduplicated. A multipart aggregate
/// or binary schema is not a JSON codec input merely because it has a schema.
#[derive(Debug, Clone, Serialize)]
pub struct ProtocolPlan {
    pub(super) version: u32,
    pub(super) capabilities: Capabilities,
    pub(super) operations: Vec<OperationPlan>,
    #[serde(serialize_with = "serialize_roots")]
    pub(super) codec_roots: Vec<SchemaId>,
    #[serde(serialize_with = "serialize_roots")]
    pub(super) codec_schema_closure: Vec<SchemaId>,
    pub(super) diagnostics: Vec<Diagnostic>,
}
impl ProtocolPlan {
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
    getters!(capabilities: Capabilities);
    #[must_use]
    pub fn operations(&self) -> &[OperationPlan] {
        &self.operations
    }
    #[must_use]
    pub fn codec_roots(&self) -> &[SchemaId] {
        &self.codec_roots
    }
    /// Candidate-aware effective closure supplied by Contract. These are not
    /// extra codec inputs or a compile-time selection of dynamic bindings.
    #[must_use]
    pub fn codec_schema_closure(&self) -> &[SchemaId] {
        &self.codec_schema_closure
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub fn is_admitted(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    /// Convenience for emitters which use `?` to guard artifact construction.
    pub fn into_result(self) -> Result<Self, Vec<Diagnostic>> {
        if self.is_admitted() {
            Ok(self)
        } else {
            Err(self.diagnostics)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
    Options,
    Head,
    Patch,
    Trace,
    Query,
    /// An exact, case-sensitive HTTP token from `additionalOperations`.
    Custom(String),
}
impl Method {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Post => "POST",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Head => "HEAD",
            Self::Patch => "PATCH",
            Self::Trace => "TRACE",
            Self::Query => "QUERY",
            Self::Custom(token) => token,
        }
    }
    #[must_use]
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }
}
impl Serialize for Method {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
// Preserve the existing `operation.method() == Method::Head` call style while
// borrowing owned custom tokens. Matching the enum's fixed variants still works.
impl PartialEq<Method> for &Method {
    fn eq(&self, other: &Method) -> bool {
        <Method as PartialEq<Method>>::eq(*self, other)
    }
}
impl PartialEq<&Method> for Method {
    fn eq(&self, other: &&Method) -> bool {
        <Method as PartialEq<Method>>::eq(self, *other)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationPlan {
    pub(super) source: Provenance,
    pub(super) path_item: SourceLocation,
    pub(super) method: Method,
    pub(super) path: String,
    pub(super) operation_id: Option<Located<String>>,
    pub(super) summary: Option<Located<String>>,
    pub(super) description: Option<Located<String>>,
    pub(super) deprecated: bool,
    pub(super) tags: Vec<Located<String>>,
    pub(super) servers: ServersPlan,
    pub(super) security: SecurityPlan,
    pub(super) parameters: Vec<ParameterPlan>,
    pub(super) body: Option<BodyPlan>,
    pub(super) responses: Vec<ResponsePlan>,
    pub(super) annotations: Vec<Located<Value>>,
}
impl OperationPlan {
    getters!(source: Provenance, path_item: SourceLocation, servers: ServersPlan, security: SecurityPlan);
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    #[must_use]
    pub fn operation_id(&self) -> Option<&Located<String>> {
        self.operation_id.as_ref()
    }
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    #[must_use]
    pub fn tags(&self) -> &[Located<String>] {
        &self.tags
    }
    #[must_use]
    pub fn parameters(&self) -> &[ParameterPlan] {
        &self.parameters
    }
    #[must_use]
    pub fn body(&self) -> Option<&BodyPlan> {
        self.body.as_ref()
    }
    #[must_use]
    pub fn responses(&self) -> &[ResponsePlan] {
        &self.responses
    }
    #[must_use]
    pub fn annotations(&self) -> &[Located<Value>] {
        &self.annotations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServersPlan {
    /// Effective array, or the containing operation if the OAS default applies.
    pub(super) source: SourceLocation,
    pub(super) candidates: Vec<ServerPlan>,
}
impl ServersPlan {
    getters!(source: SourceLocation);
    #[must_use]
    pub fn candidates(&self) -> &[ServerPlan] {
        &self.candidates
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerPlan {
    pub(super) source: Option<Provenance>,
    pub(super) default_from: Option<SourceLocation>,
    pub(super) template: String,
    pub(super) description: Option<Located<String>>,
    pub(super) name: Option<Located<String>>,
    pub(super) variables: Vec<ServerVariable>,
    pub(super) document_base: SourceLocation,
}
impl ServerPlan {
    /// Physical document serving this Server Object. A default inherited from
    /// an absent root servers declaration belongs to the entry document.
    #[must_use]
    pub fn document_base(&self) -> &SourceLocation {
        &self.document_base
    }
    #[must_use]
    pub fn source(&self) -> Option<&Provenance> {
        self.source.as_ref()
    }
    #[must_use]
    pub fn default_from(&self) -> Option<&SourceLocation> {
        self.default_from.as_ref()
    }
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.default_from.is_some()
    }
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn name(&self) -> Option<&Located<String>> {
        self.name.as_ref()
    }
    #[must_use]
    pub fn variables(&self) -> &[ServerVariable] {
        &self.variables
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerVariable {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) default: Located<String>,
    pub(super) values: Option<Vec<Located<String>>>,
    pub(super) description: Option<Located<String>>,
}
impl ServerVariable {
    getters!(source: Provenance, default: Located<String>);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn values(&self) -> Option<&[Located<String>]> {
        self.values.as_deref()
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
}

/// Absence, explicit disabling, and an anonymous alternative are distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SecurityPlan {
    Undeclared {
        source: SourceLocation,
    },
    NoAuth {
        source: SourceLocation,
    },
    Alternatives {
        source: SourceLocation,
        alternatives: Vec<SecurityAlternative>,
    },
}
impl SecurityPlan {
    #[must_use]
    pub fn alternatives(&self) -> &[SecurityAlternative] {
        match self {
            Self::Alternatives { alternatives, .. } => alternatives,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityAlternative {
    pub(super) source: SourceLocation,
    /// Empty means explicitly anonymous; nonempty requirements are AND.
    pub(super) requirements: Vec<CredentialRequirement>,
}
impl SecurityAlternative {
    getters!(source: SourceLocation);
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.requirements.is_empty()
    }
    #[must_use]
    pub fn requirements(&self) -> &[CredentialRequirement] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialRequirement {
    pub(super) source: SourceLocation,
    pub(super) name: String,
    pub(super) scheme: Provenance,
    pub(super) description: Option<Located<String>>,
    pub(super) deprecated: Option<Located<bool>>,
    pub(super) permissions: Permissions,
    pub(super) credential: CredentialHook,
}
impl CredentialRequirement {
    getters!(source: SourceLocation, scheme: Provenance, permissions: Permissions, credential: CredentialHook);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated.as_ref().is_some_and(|value| value.value)
    }
    #[must_use]
    pub fn deprecated_source(&self) -> Option<&Located<bool>> {
        self.deprecated.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "names", rename_all = "kebab-case")]
pub enum Permissions {
    Scopes(Vec<Located<String>>),
    Roles(Vec<Located<String>>),
}

/// Hooks describe credential *attachment*. OAuth/OIDC callers provide an
/// authorization credential; there is no acquisition, refresh, retry, or scope
/// inference. API key locations are explicit and HTTP schemes are fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CredentialHook {
    Bearer {
        bearer_format: Option<Located<String>>,
    },
    Basic,
    ApiKey {
        location: ParameterLocation,
        name: Located<String>,
    },
    OAuth2 {
        flows: Vec<OAuthFlow>,
        metadata_url: Option<Located<String>>,
    },
    OpenIdConnect {
        discovery_url: Located<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OAuthFlowKind {
    Implicit,
    Password,
    ClientCredentials,
    AuthorizationCode,
    DeviceAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthFlow {
    pub(super) source: SourceLocation,
    pub(super) kind: OAuthFlowKind,
    pub(super) authorization_url: Option<Located<String>>,
    pub(super) token_url: Option<Located<String>>,
    pub(super) refresh_url: Option<Located<String>>,
    pub(super) device_authorization_url: Option<Located<String>>,
    pub(super) scopes: BTreeMap<String, Located<String>>,
}
impl OAuthFlow {
    getters!(source: SourceLocation, scopes: BTreeMap<String, Located<String>>);
    #[must_use]
    pub fn kind(&self) -> OAuthFlowKind {
        self.kind
    }
    #[must_use]
    pub fn authorization_url(&self) -> Option<&Located<String>> {
        self.authorization_url.as_ref()
    }
    #[must_use]
    pub fn token_url(&self) -> Option<&Located<String>> {
        self.token_url.as_ref()
    }
    #[must_use]
    pub fn refresh_url(&self) -> Option<&Located<String>> {
        self.refresh_url.as_ref()
    }
    #[must_use]
    pub fn device_authorization_url(&self) -> Option<&Located<String>> {
        self.device_authorization_url.as_ref()
    }
}

/// `id` is always an actual indexed schema, including the `$ref` use-site when
/// it is a Schema Object. A terminal-only root would drop 3.1 `$ref` siblings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaUse {
    #[serde(serialize_with = "serialize_source_id")]
    pub(super) id: SchemaId,
    pub(super) source: Provenance,
}
impl SchemaUse {
    getters!(id: SchemaId, source: Provenance);
    #[must_use]
    pub fn resource_context(&self) -> Option<&ResourceContext> {
        self.source.use_site_resource()
    }
}

/// Reference-resource bases apply to description identifiers. API URLs instead
/// use these location bases, as specified by OAS 3.2 §4.5.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiUrlBase {
    ServerDocument,
    EffectiveServer,
}

impl CredentialHook {
    /// Relative OAuth/OIDC metadata endpoints use the selected server, not the
    /// schema/OpenAPI logical resource base. This is metadata, not acquisition.
    #[must_use]
    pub fn url_base(&self) -> Option<ApiUrlBase> {
        matches!(self, Self::OAuth2 { .. } | Self::OpenIdConnect { .. })
            .then_some(ApiUrlBase::EffectiveServer)
    }
}

impl OAuthFlow {
    #[must_use]
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::EffectiveServer
    }
}

impl ServerPlan {
    #[must_use]
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::ServerDocument
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodecInput {
    Json,
    TextScalar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodecRef {
    pub(super) schema: SchemaUse,
    pub(super) input: CodecInput,
}
impl CodecRef {
    getters!(schema: SchemaUse);
    #[must_use]
    pub fn input(&self) -> CodecInput {
        self.input
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Path,
    Query,
    /// The entire URL query, with no parameter-name prefix (OAS 3.2).
    Querystring,
    Header,
    Cookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    Simple,
    Label,
    Matrix,
    Form,
    SpaceDelimited,
    PipeDelimited,
    DeepObject,
    Cookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalarType {
    String,
    Boolean,
    Integer,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WireShape {
    Scalar {
        scalar: ScalarType,
    },
    Array {
        items: ScalarType,
    },
    FlatObject {
        properties: BTreeMap<String, ScalarType>,
        additional: AdditionalScalars,
    },
}

/// Unknown object properties cannot bypass the flat-value wire guard. This
/// guard is independent of schema validation (nested RFC6570 values are undefined).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "scalar", rename_all = "kebab-case")]
pub enum AdditionalScalars {
    Forbidden,
    Typed(ScalarType),
    AnyScalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PercentEncoding {
    /// RFC3986 unreserved; reserved delimiters inside data are encoded.
    UriComponent,
    /// RFC6570 reserved expansion; caller pre-escapes query/form hazards and
    /// active delimiters. Valid percent triples pass through exactly once.
    ReservedExpansion,
    /// No URI encoding/automatic quoting, for headers and multipart parts.
    None,
    /// Content-based form values: spaces become `+`, literal plus is `%2B`.
    FormUrlEncoded,
}

/// Complete parameter serialization strategy, independent of emitter text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ParameterSerialization {
    Style {
        style: Style,
        explode: bool,
        shape: WireShape,
        percent_encoding: PercentEncoding,
    },
    Content {
        media_type: MediaType,
        percent_encoding: PercentEncoding,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParameterPlan {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) location: ParameterLocation,
    pub(super) required: bool,
    pub(super) deprecated: bool,
    pub(super) description: Option<Located<String>>,
    pub(super) codec: CodecRef,
    pub(super) serialization: ParameterSerialization,
    pub(super) content_media: Option<MediaPlan>,
    pub(super) examples: ExampleMetadata,
}
impl ParameterPlan {
    getters!(examples: ExampleMetadata);
    getters!(source: Provenance, codec: CodecRef, serialization: ParameterSerialization);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn location(&self) -> ParameterLocation {
        self.location
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// The source-backed media representation of a content parameter. A form
    /// querystring uses its FormPlan and receives no second URI-encoding pass.
    #[must_use]
    pub fn content_media(&self) -> Option<&MediaPlan> {
        self.content_media.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyPlan {
    pub(super) source: Provenance,
    pub(super) required: bool,
    pub(super) description: Option<Located<String>>,
    pub(super) media: Vec<MediaPlan>,
    pub(super) limits: ByteLimits,
}
impl BodyPlan {
    getters!(source: Provenance);
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn media(&self) -> &[MediaPlan] {
        &self.media
    }
    #[must_use]
    pub fn limits(&self) -> ByteLimits {
        self.limits
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaPlan {
    pub(super) source: Provenance,
    pub(super) media_type: MediaType,
    pub(super) representation: Representation,
    pub(super) examples: ExampleMetadata,
}
impl MediaPlan {
    getters!(source: Provenance, media_type: MediaType, representation: Representation, examples: ExampleMetadata);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Representation {
    Json {
        codec: Option<CodecRef>,
    },
    Text {
        codec: Option<CodecRef>,
        scalar: ScalarType,
        encoding: TextEncoding,
    },
    Binary {
        schema: Option<SchemaUse>,
        bytes: BytePolicy,
    },
    Form {
        form: FormPlan,
    },
    Multipart {
        multipart: MultipartPlan,
    },
    Stream {
        stream: StreamPlan,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    Utf8,
}

/// In-memory bytes, never filesystem paths, JSON strings, or placeholder nulls.
/// A native adapter checks the bound before buffering/copying a body or part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BytePolicy {
    pub(super) max_bytes: u64,
    pub(super) declared_max_bytes: Option<Located<u64>>,
}
impl BytePolicy {
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    #[must_use]
    pub fn declared_max_bytes(&self) -> Option<&Located<u64>> {
        self.declared_max_bytes.as_ref()
    }
}

/// Whole-object assertions that can be enforced without pretending bytes are
/// JSON. More complex cross-property assertions decline at their schema keyword.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectRules {
    pub(super) schema: SchemaUse,
    pub(super) required: Vec<Located<String>>,
    pub(super) min_properties: Option<Located<u64>>,
    pub(super) max_properties: Option<Located<u64>>,
}
impl ObjectRules {
    getters!(schema: SchemaUse);
    #[must_use]
    pub fn required(&self) -> &[Located<String>] {
        &self.required
    }
    #[must_use]
    pub fn min_properties(&self) -> Option<&Located<u64>> {
        self.min_properties.as_ref()
    }
    #[must_use]
    pub fn max_properties(&self) -> Option<&Located<u64>> {
        self.max_properties.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormPlan {
    pub(super) rules: ObjectRules,
    pub(super) fields: Vec<PartPlan>,
    pub(super) additional: AdditionalParts,
}
impl FormPlan {
    getters!(rules: ObjectRules, additional: AdditionalParts);
    #[must_use]
    pub fn fields(&self) -> &[PartPlan] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MultipartPlan {
    Named {
        rules: ObjectRules,
        parts: Vec<PartPlan>,
        additional: AdditionalParts,
    },
    Positional {
        schema: SchemaUse,
        prefix: Vec<PartPlan>,
        items: AdditionalParts,
        min_items: Option<Located<u64>>,
        max_items: Option<Located<u64>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "part", rename_all = "kebab-case")]
pub enum AdditionalParts {
    Forbidden,
    Allowed(Box<PartPlan>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartMultiplicity {
    One,
    RepeatedArrayItems,
}

/// The schema root for an array property is metadata. The per-item codec is
/// the actual input for repeated parts; byte items never enter a JSON codec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartPlan {
    pub(super) source: Provenance,
    pub(super) name: Option<String>,
    pub(super) schema: SchemaUse,
    pub(super) required: bool,
    pub(super) multiplicity: PartMultiplicity,
    pub(super) min_items: Option<Located<u64>>,
    pub(super) max_items: Option<Located<u64>>,
    pub(super) encoding_source: Option<Provenance>,
    pub(super) content_types: Vec<MediaType>,
    pub(super) representation: PartRepresentation,
    pub(super) headers: Vec<HeaderPlan>,
}
impl PartPlan {
    getters!(source: Provenance, schema: SchemaUse, representation: PartRepresentation);
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    #[must_use]
    pub fn multiplicity(&self) -> PartMultiplicity {
        self.multiplicity
    }
    #[must_use]
    pub fn min_items(&self) -> Option<&Located<u64>> {
        self.min_items.as_ref()
    }
    #[must_use]
    pub fn max_items(&self) -> Option<&Located<u64>> {
        self.max_items.as_ref()
    }
    #[must_use]
    pub fn encoding_source(&self) -> Option<&Provenance> {
        self.encoding_source.as_ref()
    }
    #[must_use]
    pub fn content_types(&self) -> &[MediaType] {
        &self.content_types
    }
    #[must_use]
    pub fn headers(&self) -> &[HeaderPlan] {
        &self.headers
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PartRepresentation {
    Json {
        codec: CodecRef,
        outer_encoding: PercentEncoding,
    },
    Text {
        codec: CodecRef,
        scalar: ScalarType,
        outer_encoding: PercentEncoding,
    },
    Binary {
        bytes: BytePolicy,
    },
    Style {
        codec: CodecRef,
        serialization: ParameterSerialization,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StreamFraming {
    ServerSentEvents,
    JsonLines,
}

/// OAS 3.2 `itemSchema` describes the parsed item. For SSE this is the event
/// envelope, with string `data` and integer `retry`. No sentinel or nested JSON
/// decode is inferred, including from a `contentSchema` annotation. Under
/// `SchemalessStreamEventsV1` a schemaless SSE stream carries no item codec at
/// all: frames surface as untyped parsed envelope values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamPlan {
    pub(super) source: SourceLocation,
    pub(super) framing: StreamFraming,
    pub(super) item_codec: Option<CodecRef>,
    pub(super) max_item_bytes: u64,
}
impl StreamPlan {
    getters!(source: SourceLocation);
    #[must_use]
    pub fn item_codec(&self) -> Option<&CodecRef> {
        self.item_codec.as_ref()
    }
    #[must_use]
    pub fn framing(&self) -> StreamFraming {
        self.framing
    }
    #[must_use]
    pub fn max_item_bytes(&self) -> u64 {
        self.max_item_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum ResponseStatus {
    Exact(u16),
    Range(u8),
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponsePlan {
    pub(super) source: Provenance,
    pub(super) status_key: String,
    pub(super) status: ResponseStatus,
    pub(super) description: Located<String>,
    pub(super) description_declared: bool,
    pub(super) summary: Option<Located<String>>,
    pub(super) media: Vec<MediaPlan>,
    pub(super) headers: Vec<HeaderPlan>,
    pub(super) links: Vec<LinkPlan>,
    pub(super) max_body_bytes: u64,
}
impl ResponsePlan {
    getters!(source: Provenance, description: Located<String>);
    /// OAS 3.2 permits absence. The existing `description()` display accessor
    /// returns empty text anchored at the response in that case; it does not
    /// invent a description source field.
    #[must_use]
    pub fn declared_description(&self) -> Option<&Located<String>> {
        self.description_declared.then_some(&self.description)
    }
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    #[must_use]
    pub fn status_key(&self) -> &str {
        &self.status_key
    }
    #[must_use]
    pub fn status(&self) -> ResponseStatus {
        self.status
    }
    #[must_use]
    pub fn media(&self) -> &[MediaPlan] {
        &self.media
    }
    #[must_use]
    pub fn headers(&self) -> &[HeaderPlan] {
        &self.headers
    }
    #[must_use]
    pub fn links(&self) -> &[LinkPlan] {
        &self.links
    }
    #[must_use]
    pub fn max_body_bytes(&self) -> u64 {
        self.max_body_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseBodyDisposition {
    Declared,
    UndeclaredBoundedBytes,
    ForbiddenByHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeaderPlan {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) deprecated: bool,
    pub(super) description: Option<Located<String>>,
    pub(super) codec: CodecRef,
    pub(super) serialization: ParameterSerialization,
    pub(super) content_media: Option<MediaPlan>,
    pub(super) examples: ExampleMetadata,
}
impl HeaderPlan {
    getters!(examples: ExampleMetadata);
    getters!(source: Provenance, codec: CodecRef, serialization: ParameterSerialization);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn content_media(&self) -> Option<&MediaPlan> {
        self.content_media.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkPlan {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) target: LinkTarget,
    pub(super) parameters: BTreeMap<String, Located<Value>>,
    pub(super) request_body: Option<Located<Value>>,
    pub(super) description: Option<Located<String>>,
    pub(super) server: Option<ServerPlan>,
}
impl LinkPlan {
    getters!(source: Provenance, target: LinkTarget, parameters: BTreeMap<String, Located<Value>>);
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn request_body(&self) -> Option<&Located<Value>> {
        self.request_body.as_ref()
    }
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    #[must_use]
    pub fn server(&self) -> Option<&ServerPlan> {
        self.server.as_ref()
    }
}

/// Link values/expressions remain metadata. No linked operation is selected or
/// called, and literal objects containing `schema`/`$ref` are not schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LinkTarget {
    OperationId {
        value: Located<String>,
        operation: SourceLocation,
    },
    OperationRef {
        value: Located<String>,
        operation: SourceLocation,
    },
}
