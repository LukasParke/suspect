use std::{collections::BTreeMap, ops::Range};

use serde::{Serialize, Serializer};
use serde_json::Value;
use suspect_ir::contract::{SchemaId, SourceId};

use super::{ByteLimits, Capabilities, Capability, MediaType};

// Descriptors expose shared borrows, never mutable collections or setters.
macro_rules! getters {
    ($($(#[$doc:meta])* $name:ident : $ty:ty),* $(,)?) => { $(
        $(#[$doc])*
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
    getters!(
        /// Source node the value was read from.
        source: SourceId,
    );
    /// Byte range of the value in its retrieval document.
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
    getters!(
        /// Where the value is used, before following any `$ref`.
        use_site: SourceLocation,
        /// The definition reached after following every reference hop.
        terminal: SourceLocation,
    );
    /// Each `$ref` hop between `use_site` and `terminal`, in traversal order.
    #[must_use]
    pub fn references(&self) -> &[SourceLocation] {
        &self.references
    }
    /// Lexical resource context of `use_site`.
    #[must_use]
    pub fn use_site_resource(&self) -> Option<&ResourceContext> {
        self.use_site_resource.as_ref()
    }
    /// Lexical resource context of `terminal`.
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

/// What kind of logical resource a [`ResourceContext`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    /// A non-OpenAPI document resource.
    Document,
    /// An OpenAPI document, named by its `$self` or retrieval URI.
    OpenApiDocument,
    /// A schema resource with its own `$id`.
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
    getters!(
        /// Node this context was computed for.
        source: SourceLocation,
        /// Root node of the enclosing logical resource.
        resource: SourceLocation,
    );
    /// Kind of the enclosing resource.
    #[must_use]
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    /// Canonical URI: the declared `$self`/`$id`, or the retrieval URI otherwise.
    #[must_use]
    pub fn canonical_uri(&self) -> &str {
        &self.canonical_uri
    }
    /// Fragment-free base URI that relative references in this scope resolve against.
    #[must_use]
    pub fn base_uri(&self) -> &str {
        &self.base_uri
    }
    /// The `$id`/`$self` that set `base_uri`; `None` when it comes from the retrieval URI.
    #[must_use]
    pub fn base_source(&self) -> Option<&SourceLocation> {
        self.base_source.as_ref()
    }
    /// Absolute URI plus resource-relative encoded pointer of the nearest registered scope.
    #[must_use]
    pub fn scope_address(&self) -> &str {
        &self.scope_address
    }
    /// Nearest `$id` resource root, or the root of an embedded id-less schema.
    #[must_use]
    pub fn schema_root(&self) -> Option<&SourceLocation> {
        self.schema_root.as_ref()
    }
    /// Accepted canonical/retrieval/requested names of the resource.
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
    getters!(
        /// Exact location of the value.
        source: SourceLocation,
        /// The value itself.
        value: T,
    );
}

/// Instance-ready and serialized examples remain distinct. None of these values
/// is walked as a schema, reference graph, credential, or transport instruction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExampleMetadata {
    pub(super) inline: Option<Located<Value>>,
    pub(super) named: Vec<ExamplePlan>,
}
impl ExampleMetadata {
    /// The singular `example` field.
    #[must_use]
    pub fn inline(&self) -> Option<&Located<Value>> {
        self.inline.as_ref()
    }
    /// Named Example Objects from `examples`, in declaration order.
    #[must_use]
    pub fn named(&self) -> &[ExamplePlan] {
        &self.named
    }
}

/// One named Example Object.
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
    getters!(
        /// Example Object use site and terminal definition.
        source: Provenance,
    );
    /// Key in the `examples` map.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Declared `summary`.
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Declared `value`.
    #[must_use]
    pub fn value(&self) -> Option<&Located<Value>> {
        self.value.as_ref()
    }
    /// Declared OAS 3.2 `dataValue`: the instance before serialization.
    #[must_use]
    pub fn data_value(&self) -> Option<&Located<Value>> {
        self.data_value.as_ref()
    }
    /// Declared OAS 3.2 `serializedValue`: the exact wire text.
    #[must_use]
    pub fn serialized_value(&self) -> Option<&Located<String>> {
        self.serialized_value.as_ref()
    }
    /// Declared `externalValue` URI, never fetched.
    #[must_use]
    pub fn external_value(&self) -> Option<&Located<String>> {
        self.external_value.as_ref()
    }
}

/// Whether a diagnostic blocks admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Refuses the whole plan.
    Error,
    /// Recorded only; the plan may still be admitted.
    Warning,
}

/// Broad category of a planner diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticKind {
    /// The source violates OpenAPI or JSON Schema.
    InvalidSource,
    /// Valid source the shared descriptors cannot represent.
    Unsupported,
    /// Representable source that needs a capability the adapter has not opted in to.
    Capability,
    /// A warning about an uninterpreted or ignored construct.
    Annotation,
}

/// One located planner finding.
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
    getters!(
        /// Construct the finding is about.
        source: SourceLocation,
    );
    /// Stable machine-readable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
    /// Whether the finding blocks admission.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }
    /// Broad category of the finding.
    #[must_use]
    pub fn kind(&self) -> DiagnosticKind {
        self.kind
    }
    /// Human-readable explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    /// The missing capability, for [`DiagnosticKind::Capability`] findings.
    #[must_use]
    pub fn capability(&self) -> Option<Capability> {
        self.capability
    }
    /// Reference use sites that reach `source`, so the finding can be shown where it is used.
    #[must_use]
    pub fn related(&self) -> &[SourceLocation] {
        &self.related
    }
    /// Lexical resource context of `source`.
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
    /// Descriptor semantics version; see [`super::PROTOCOL_PLAN_VERSION`].
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
    getters!(
        /// Adapter capabilities the plan was computed against.
        capabilities: Capabilities,
    );
    /// Planned operations in selection order; empty unless admitted.
    #[must_use]
    pub fn operations(&self) -> &[OperationPlan] {
        &self.operations
    }
    /// Actual JSON/text codec input schemas, sorted and deduplicated.
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
    /// Every error and warning found while planning.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    /// Whether planning produced no error diagnostics.
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

/// HTTP request method of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `PUT`.
    Put,
    /// `POST`.
    Post,
    /// `DELETE`.
    Delete,
    /// `OPTIONS`.
    Options,
    /// `HEAD`.
    Head,
    /// `PATCH`.
    Patch,
    /// `TRACE`.
    Trace,
    /// `QUERY` (OAS 3.2).
    Query,
    /// An exact, case-sensitive HTTP token from `additionalOperations`.
    Custom(String),
}
impl Method {
    /// Wire token; uppercase for fixed methods, verbatim for custom ones.
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
    /// Whether this is an `additionalOperations` token.
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

/// One selected outgoing operation with every wire decision made.
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
    getters!(
        /// Operation Object use site and terminal definition.
        source: Provenance,
        /// Path Item Object that declares the operation.
        path_item: SourceLocation,
        /// Effective server candidates.
        servers: ServersPlan,
        /// Effective security requirements.
        security: SecurityPlan,
    );
    /// Request method.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }
    /// Path template exactly as declared, such as `/pets/{id}`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Declared `operationId`.
    #[must_use]
    pub fn operation_id(&self) -> Option<&Located<String>> {
        self.operation_id.as_ref()
    }
    /// Declared `summary`.
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Declared `deprecated: true`.
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    /// Declared tags, in order.
    #[must_use]
    pub fn tags(&self) -> &[Located<String>] {
        &self.tags
    }
    /// Effective path-item and operation parameters.
    #[must_use]
    pub fn parameters(&self) -> &[ParameterPlan] {
        &self.parameters
    }
    /// Request body, when declared.
    #[must_use]
    pub fn body(&self) -> Option<&BodyPlan> {
        self.body.as_ref()
    }
    /// Declared responses; empty when `responses` is absent.
    #[must_use]
    pub fn responses(&self) -> &[ResponsePlan] {
        &self.responses
    }
    /// `x-` extension values on the Operation Object, uninterpreted.
    #[must_use]
    pub fn annotations(&self) -> &[Located<Value>] {
        &self.annotations
    }
}

/// Effective server candidates for one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServersPlan {
    /// Effective array, or the containing operation if the OAS default applies.
    pub(super) source: SourceLocation,
    pub(super) candidates: Vec<ServerPlan>,
}
impl ServersPlan {
    getters!(
        /// Effective `servers` array, or the operation when the OAS default applies.
        source: SourceLocation,
    );
    /// Candidates in declaration order.
    #[must_use]
    pub fn candidates(&self) -> &[ServerPlan] {
        &self.candidates
    }
}

/// One Server Object, or the implicit `/` default.
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
    /// Server Object definition; `None` for the implicit default.
    #[must_use]
    pub fn source(&self) -> Option<&Provenance> {
        self.source.as_ref()
    }
    /// Node whose absent `servers` produced the implicit default.
    #[must_use]
    pub fn default_from(&self) -> Option<&SourceLocation> {
        self.default_from.as_ref()
    }
    /// Whether this is the implicit `/` default server.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.default_from.is_some()
    }
    /// URL template exactly as declared, including `{variable}` expressions.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Declared OAS 3.2 `name`.
    #[must_use]
    pub fn name(&self) -> Option<&Located<String>> {
        self.name.as_ref()
    }
    /// Declared template variables.
    #[must_use]
    pub fn variables(&self) -> &[ServerVariable] {
        &self.variables
    }
}

/// One server URL template variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerVariable {
    pub(super) source: Provenance,
    pub(super) name: String,
    pub(super) default: Located<String>,
    pub(super) values: Option<Vec<Located<String>>>,
    pub(super) description: Option<Located<String>>,
}
impl ServerVariable {
    getters!(
        /// Server Variable Object use site and terminal definition.
        source: Provenance,
        /// Declared `default` substitution.
        default: Located<String>,
    );
    /// Variable name used in the template.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Declared `enum` of allowed substitutions.
    #[must_use]
    pub fn values(&self) -> Option<&[Located<String>]> {
        self.values.as_deref()
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
}

/// Absence, explicit disabling, and an anonymous alternative are distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SecurityPlan {
    /// No `security` applies at root or operation level.
    Undeclared {
        /// The operation.
        source: SourceLocation,
    },
    /// `security: []` explicitly disables authentication.
    NoAuth {
        /// The effective `security` array.
        source: SourceLocation,
    },
    /// One or more alternatives, any one of which satisfies the operation.
    Alternatives {
        /// The effective `security` array.
        source: SourceLocation,
        /// Alternatives in declaration order.
        alternatives: Vec<SecurityAlternative>,
    },
}
impl SecurityPlan {
    /// Declared alternatives; empty unless `Alternatives`.
    #[must_use]
    pub fn alternatives(&self) -> &[SecurityAlternative] {
        match self {
            Self::Alternatives { alternatives, .. } => alternatives,
            _ => &[],
        }
    }
}

/// One Security Requirement Object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityAlternative {
    pub(super) source: SourceLocation,
    /// Empty means explicitly anonymous; nonempty requirements are AND.
    pub(super) requirements: Vec<CredentialRequirement>,
}
impl SecurityAlternative {
    getters!(
        /// The Security Requirement Object.
        source: SourceLocation,
    );
    /// Whether this is the explicit anonymous alternative `{}`.
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.requirements.is_empty()
    }
    /// Schemes that must all be satisfied together.
    #[must_use]
    pub fn requirements(&self) -> &[CredentialRequirement] {
        &self.requirements
    }
}

/// One scheme named by a security requirement, with how its credential attaches.
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
    getters!(
        /// The requirement entry naming the scheme.
        source: SourceLocation,
        /// Security Scheme Object use site and terminal definition.
        scheme: Provenance,
        /// Scopes or roles the requirement lists.
        permissions: Permissions,
        /// How the credential attaches to requests.
        credential: CredentialHook,
    );
    /// Security Scheme name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Declared scheme `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Whether the scheme declares `deprecated: true` (OAS 3.2).
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated.as_ref().is_some_and(|value| value.value)
    }
    /// Declared `deprecated` value and its location.
    #[must_use]
    pub fn deprecated_source(&self) -> Option<&Located<bool>> {
        self.deprecated.as_ref()
    }
}

/// Names listed by one security requirement entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "names", rename_all = "kebab-case")]
pub enum Permissions {
    /// OAuth 2.0 / OpenID Connect scopes.
    Scopes(Vec<Located<String>>),
    /// Roles for other scheme types (OAS 3.1+).
    Roles(Vec<Located<String>>),
}

/// Hooks describe credential *attachment*. OAuth/OIDC callers provide an
/// authorization credential; there is no acquisition, refresh, retry, or scope
/// inference. API key locations are explicit and HTTP schemes are fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CredentialHook {
    /// HTTP `bearer`: `Authorization: Bearer <token>`.
    Bearer {
        /// Declared `bearerFormat` hint.
        bearer_format: Option<Located<String>>,
    },
    /// HTTP `basic`: `Authorization: Basic <base64(user:password)>`.
    Basic,
    /// `apiKey` sent as a named header, query parameter, or cookie.
    ApiKey {
        /// Where the key is sent.
        location: ParameterLocation,
        /// Header, query parameter, or cookie name.
        name: Located<String>,
    },
    /// `oauth2`: the caller supplies the access token as a bearer credential.
    OAuth2 {
        /// Declared flows, in declaration order.
        flows: Vec<OAuthFlow>,
        /// Declared OAS 3.2 `oauth2MetadataUrl`.
        metadata_url: Option<Located<String>>,
    },
    /// `openIdConnect`: the caller supplies the access token as a bearer credential.
    OpenIdConnect {
        /// Declared `openIdConnectUrl`.
        discovery_url: Located<String>,
    },
}

/// Key of one entry in an OAuth Flows Object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OAuthFlowKind {
    /// `implicit`.
    Implicit,
    /// `password`.
    Password,
    /// `clientCredentials`.
    ClientCredentials,
    /// `authorizationCode`.
    AuthorizationCode,
    /// `deviceAuthorization` (OAS 3.2).
    DeviceAuthorization,
}

/// One declared OAuth Flow Object.
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
    getters!(
        /// The OAuth Flow Object.
        source: SourceLocation,
        /// Declared scope name to description.
        scopes: BTreeMap<String, Located<String>>,
    );
    /// Which flow this is.
    #[must_use]
    pub fn kind(&self) -> OAuthFlowKind {
        self.kind
    }
    /// Declared `authorizationUrl`.
    #[must_use]
    pub fn authorization_url(&self) -> Option<&Located<String>> {
        self.authorization_url.as_ref()
    }
    /// Declared `tokenUrl`.
    #[must_use]
    pub fn token_url(&self) -> Option<&Located<String>> {
        self.token_url.as_ref()
    }
    /// Declared `refreshUrl`.
    #[must_use]
    pub fn refresh_url(&self) -> Option<&Located<String>> {
        self.refresh_url.as_ref()
    }
    /// Declared OAS 3.2 `deviceAuthorizationUrl`.
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
    getters!(
        /// Indexed schema at the use site.
        id: SchemaId,
        /// Schema use site and terminal definition.
        source: Provenance,
    );
    /// Lexical resource context of the use site.
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
    /// The document that declares the Server Object.
    ServerDocument,
    /// The server URL selected for the request.
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
    /// Relative flow endpoints resolve against the selected server.
    #[must_use]
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::EffectiveServer
    }
}

impl ServerPlan {
    /// Relative server URLs resolve against the serving document.
    #[must_use]
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::ServerDocument
    }
}

/// What kind of value a codec decodes or encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodecInput {
    /// A JSON document.
    Json,
    /// A text value parsed as one scalar.
    TextScalar,
}

/// A schema and the input form its codec handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodecRef {
    pub(super) schema: SchemaUse,
    pub(super) input: CodecInput,
}
impl CodecRef {
    getters!(
        /// Schema the codec is generated from.
        schema: SchemaUse,
    );
    /// Input form the codec handles.
    #[must_use]
    pub fn input(&self) -> CodecInput {
        self.input
    }
}

/// Where a parameter (or API key) is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    /// A path template expression.
    Path,
    /// One named query parameter.
    Query,
    /// The entire URL query, with no parameter-name prefix (OAS 3.2).
    Querystring,
    /// A request header.
    Header,
    /// A `Cookie` header pair.
    Cookie,
}

/// OpenAPI parameter `style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    /// `simple`: comma-separated (RFC 6570 `{x}`).
    Simple,
    /// `label`: dot-prefixed (RFC 6570 `{.x}`).
    Label,
    /// `matrix`: semicolon-prefixed (RFC 6570 `{;x}`).
    Matrix,
    /// `form`: `name=value` pairs (RFC 6570 `{?x}`).
    Form,
    /// `spaceDelimited`: array values joined by spaces.
    SpaceDelimited,
    /// `pipeDelimited`: array values joined by `|`.
    PipeDelimited,
    /// `deepObject`: `name[key]=value` pairs.
    DeepObject,
    /// `cookie` (OAS 3.2): `name=value` cookie pairs.
    Cookie,
}

/// JSON Schema scalar type of a serialized value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalarType {
    /// `string`.
    String,
    /// `boolean`.
    Boolean,
    /// `integer`.
    Integer,
    /// `number`.
    Number,
}

/// Value shape a style serialization accepts; nesting beyond one level is undefined by RFC 6570.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WireShape {
    /// One scalar value.
    Scalar {
        /// Type of the value.
        scalar: ScalarType,
    },
    /// An array of scalars.
    Array {
        /// Type of every item.
        items: ScalarType,
    },
    /// An object whose property values are all scalars.
    FlatObject {
        /// Declared property name to scalar type.
        properties: BTreeMap<String, ScalarType>,
        /// Policy for undeclared properties.
        additional: AdditionalScalars,
    },
}

/// Unknown object properties cannot bypass the flat-value wire guard. This
/// guard is independent of schema validation (nested RFC6570 values are undefined).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "scalar", rename_all = "kebab-case")]
pub enum AdditionalScalars {
    /// `additionalProperties: false`.
    Forbidden,
    /// Undeclared properties of one scalar type.
    Typed(ScalarType),
    /// Undeclared properties of any scalar type.
    AnyScalar,
}

/// How serialized text is percent-encoded on the wire.
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
    /// `schema`-based RFC 6570-style serialization.
    Style {
        /// Effective style.
        style: Style,
        /// Effective `explode`.
        explode: bool,
        /// Value shape the style is applied to.
        shape: WireShape,
        /// Encoding applied to the serialized text.
        percent_encoding: PercentEncoding,
    },
    /// `content`-based serialization through one media type.
    Content {
        /// The single declared media type.
        media_type: MediaType,
        /// Encoding applied to the serialized text.
        percent_encoding: PercentEncoding,
    },
}

/// One effective request parameter.
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
    getters!(
        /// Parameter-level `example`/`examples`.
        examples: ExampleMetadata,
    );
    getters!(
        /// Parameter Object use site and terminal definition.
        source: Provenance,
        /// Codec for the parameter value.
        codec: CodecRef,
        /// How the value is written to the wire.
        serialization: ParameterSerialization,
    );
    /// Declared parameter name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Where the parameter is sent.
    #[must_use]
    pub fn location(&self) -> ParameterLocation {
        self.location
    }
    /// Whether the caller must supply a value.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    /// Declared `deprecated: true`.
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    /// Declared `description`.
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

/// A request body (or incoming request body).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyPlan {
    pub(super) source: Provenance,
    pub(super) required: bool,
    pub(super) description: Option<Located<String>>,
    pub(super) media: Vec<MediaPlan>,
    pub(super) limits: ByteLimits,
}
impl BodyPlan {
    getters!(
        /// Request Body Object use site and terminal definition.
        source: Provenance,
    );
    /// Declared `required: true`.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Declared media types, in declaration order.
    #[must_use]
    pub fn media(&self) -> &[MediaPlan] {
        &self.media
    }
    /// Adapter byte limits applied to the body.
    #[must_use]
    pub fn limits(&self) -> ByteLimits {
        self.limits
    }
}

/// One declared media type of a body, response, or content parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaPlan {
    pub(super) source: Provenance,
    pub(super) media_type: MediaType,
    pub(super) representation: Representation,
    pub(super) examples: ExampleMetadata,
}
impl MediaPlan {
    getters!(
        /// Media Type Object use site and terminal definition.
        source: Provenance,
        /// Declared media type or range.
        media_type: MediaType,
        /// How values of this media type are encoded and decoded.
        representation: Representation,
        /// Media-level `example`/`examples`.
        examples: ExampleMetadata,
    );
}

/// How values of one media type are encoded and decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Representation {
    /// A JSON document.
    Json {
        /// Codec for the declared schema; `None` for schema-free JSON.
        codec: Option<CodecRef>,
    },
    /// UTF-8 text holding one scalar.
    Text {
        /// Codec for the declared schema; `None` when no schema is declared.
        codec: Option<CodecRef>,
        /// Scalar type the text is parsed as.
        scalar: ScalarType,
        /// Character encoding of the text.
        encoding: TextEncoding,
    },
    /// Opaque bytes.
    Binary {
        /// Declared schema, kept as metadata only.
        schema: Option<SchemaUse>,
        /// Size bound for the bytes.
        bytes: BytePolicy,
    },
    /// `application/x-www-form-urlencoded`.
    Form {
        /// Field plan.
        form: FormPlan,
    },
    /// `multipart/*`.
    Multipart {
        /// Part plan.
        multipart: MultipartPlan,
    },
    /// A sequential stream of items (SSE or JSON lines).
    Stream {
        /// Framing and item codec.
        stream: StreamPlan,
    },
}

/// Character encoding of a text body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    /// UTF-8; other charsets are refused.
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
    /// Effective bound: the adapter limit, lowered by any declared maximum.
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    /// Declared OAS 3.2 `maxLength` of the byte schema.
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
    getters!(
        /// Object schema the rules come from.
        schema: SchemaUse,
    );
    /// Declared `required` property names.
    #[must_use]
    pub fn required(&self) -> &[Located<String>] {
        &self.required
    }
    /// Declared `minProperties`.
    #[must_use]
    pub fn min_properties(&self) -> Option<&Located<u64>> {
        self.min_properties.as_ref()
    }
    /// Declared `maxProperties`.
    #[must_use]
    pub fn max_properties(&self) -> Option<&Located<u64>> {
        self.max_properties.as_ref()
    }
}

/// Fields of a form-urlencoded body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormPlan {
    pub(super) rules: ObjectRules,
    pub(super) fields: Vec<PartPlan>,
    pub(super) additional: AdditionalParts,
}
impl FormPlan {
    getters!(
        /// Whole-object assertions.
        rules: ObjectRules,
        /// Policy for undeclared fields.
        additional: AdditionalParts,
    );
    /// Declared fields, one per schema property.
    #[must_use]
    pub fn fields(&self) -> &[PartPlan] {
        &self.fields
    }
}

/// Parts of a multipart body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MultipartPlan {
    /// Parts named by object schema properties.
    Named {
        /// Whole-object assertions.
        rules: ObjectRules,
        /// Declared parts, one per schema property.
        parts: Vec<PartPlan>,
        /// Policy for undeclared parts.
        additional: AdditionalParts,
    },
    /// Unnamed parts in array order (OAS 3.2 `prefixEncoding`/`itemEncoding`).
    Positional {
        /// Array schema of the whole body.
        schema: SchemaUse,
        /// Parts for the leading `prefixItems`.
        prefix: Vec<PartPlan>,
        /// Policy for parts after the prefix.
        items: AdditionalParts,
        /// Declared `minItems`.
        min_items: Option<Located<u64>>,
        /// Declared `maxItems`.
        max_items: Option<Located<u64>>,
    },
}

/// Whether parts or fields beyond the declared ones are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "part", rename_all = "kebab-case")]
pub enum AdditionalParts {
    /// No further parts are accepted.
    Forbidden,
    /// Further parts are accepted and encoded with this plan.
    Allowed(Box<PartPlan>),
}

/// How many wire parts one property produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartMultiplicity {
    /// Exactly one part.
    One,
    /// One part per array item, all with the same name.
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
    getters!(
        /// Property schema use site and terminal definition.
        source: Provenance,
        /// Property schema (the array schema for repeated parts).
        schema: SchemaUse,
        /// How each part value is encoded.
        representation: PartRepresentation,
    );
    /// Property name; `None` for positional parts.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    /// Whether the enclosing schema requires the property.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    /// How many wire parts the property produces.
    #[must_use]
    pub fn multiplicity(&self) -> PartMultiplicity {
        self.multiplicity
    }
    /// Declared `minItems` for repeated parts.
    #[must_use]
    pub fn min_items(&self) -> Option<&Located<u64>> {
        self.min_items.as_ref()
    }
    /// Declared `maxItems` for repeated parts.
    #[must_use]
    pub fn max_items(&self) -> Option<&Located<u64>> {
        self.max_items.as_ref()
    }
    /// Encoding Object that configures the part.
    #[must_use]
    pub fn encoding_source(&self) -> Option<&Provenance> {
        self.encoding_source.as_ref()
    }
    /// Declared or defaulted part content types; empty for style-serialized parts.
    #[must_use]
    pub fn content_types(&self) -> &[MediaType] {
        &self.content_types
    }
    /// Declared multipart part headers.
    #[must_use]
    pub fn headers(&self) -> &[HeaderPlan] {
        &self.headers
    }
}

/// How one form field or multipart part value is encoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PartRepresentation {
    /// A JSON document.
    Json {
        /// Codec for the part value.
        codec: CodecRef,
        /// Encoding applied around the JSON text (form fields only).
        outer_encoding: PercentEncoding,
    },
    /// UTF-8 text holding one scalar.
    Text {
        /// Codec for the part value.
        codec: CodecRef,
        /// Scalar type the text is parsed as.
        scalar: ScalarType,
        /// Encoding applied around the text (form fields only).
        outer_encoding: PercentEncoding,
    },
    /// Opaque bytes (multipart only).
    Binary {
        /// Size bound for the bytes.
        bytes: BytePolicy,
    },
    /// RFC 6570-style serialization selected by `style`/`explode`/`allowReserved`.
    Style {
        /// Codec for the part value.
        codec: CodecRef,
        /// How the value is serialized.
        serialization: ParameterSerialization,
    },
}

/// How a sequential body is split into items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StreamFraming {
    /// `text/event-stream` events.
    ServerSentEvents,
    /// One JSON value per line.
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
    getters!(
        /// The stream Media Type Object.
        source: SourceLocation,
    );
    /// Codec for the declared `itemSchema`.
    #[must_use]
    pub fn item_codec(&self) -> Option<&CodecRef> {
        self.item_codec.as_ref()
    }
    /// How the body is split into items.
    #[must_use]
    pub fn framing(&self) -> StreamFraming {
        self.framing
    }
    /// Bound for one item, from the adapter stream-item limit.
    #[must_use]
    pub fn max_item_bytes(&self) -> u64 {
        self.max_item_bytes
    }
}

/// Parsed key of a Responses Object entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum ResponseStatus {
    /// An exact status code such as `404`.
    Exact(u16),
    /// A status class such as `4XX`, holding its first digit.
    Range(u8),
    /// `default`.
    Default,
}

/// One declared response.
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
    getters!(
        /// Response Object use site and terminal definition.
        source: Provenance,
        /// Declared description, or empty text anchored at the response when absent.
        description: Located<String>,
    );
    /// OAS 3.2 permits absence. The existing `description()` display accessor
    /// returns empty text anchored at the response in that case; it does not
    /// invent a description source field.
    #[must_use]
    pub fn declared_description(&self) -> Option<&Located<String>> {
        self.description_declared.then_some(&self.description)
    }
    /// Declared OAS 3.2 `summary`.
    #[must_use]
    pub fn summary(&self) -> Option<&Located<String>> {
        self.summary.as_ref()
    }
    /// Responses map key exactly as declared (`200`, `2XX`, `default`).
    #[must_use]
    pub fn status_key(&self) -> &str {
        &self.status_key
    }
    /// Parsed status selector.
    #[must_use]
    pub fn status(&self) -> ResponseStatus {
        self.status
    }
    /// Declared media types; empty when the body is undeclared.
    #[must_use]
    pub fn media(&self) -> &[MediaPlan] {
        &self.media
    }
    /// Declared response headers.
    #[must_use]
    pub fn headers(&self) -> &[HeaderPlan] {
        &self.headers
    }
    /// Declared response links.
    #[must_use]
    pub fn links(&self) -> &[LinkPlan] {
        &self.links
    }
    /// Bound for the response body, from the adapter body limit.
    #[must_use]
    pub fn max_body_bytes(&self) -> u64 {
        self.max_body_bytes
    }
}

/// How an actual response body is handled after matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseBodyDisposition {
    /// Decode with the matched media declaration.
    Declared,
    /// No media is declared; read at most the body limit as opaque bytes.
    UndeclaredBoundedBytes,
    /// HTTP forbids a body (HEAD, 1xx, 204, 205, 304); ignore it.
    ForbiddenByHttp,
}

/// One declared response or part header.
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
    getters!(
        /// Header-level `example`/`examples`.
        examples: ExampleMetadata,
    );
    getters!(
        /// Header Object use site and terminal definition.
        source: Provenance,
        /// Codec for the header value.
        codec: CodecRef,
        /// How the value is read from the wire.
        serialization: ParameterSerialization,
    );
    /// Header name as declared.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Declared `required: true`.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    /// Declared `deprecated: true`.
    #[must_use]
    pub fn deprecated(&self) -> bool {
        self.deprecated
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Media representation of a `content`-based header.
    #[must_use]
    pub fn content_media(&self) -> Option<&MediaPlan> {
        self.content_media.as_ref()
    }
}

/// One declared response Link Object, kept as metadata.
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
    getters!(
        /// Link Object use site and terminal definition.
        source: Provenance,
        /// Operation the link points at.
        target: LinkTarget,
        /// Declared parameter name to literal value or runtime expression.
        parameters: BTreeMap<String, Located<Value>>,
    );
    /// Key in the `links` map.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Declared `requestBody` literal or runtime expression.
    #[must_use]
    pub fn request_body(&self) -> Option<&Located<Value>> {
        self.request_body.as_ref()
    }
    /// Declared `description`.
    #[must_use]
    pub fn description(&self) -> Option<&Located<String>> {
        self.description.as_ref()
    }
    /// Declared `server` override.
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
    /// Target named by `operationId`.
    OperationId {
        /// Declared `operationId`.
        value: Located<String>,
        /// The uniquely resolved Operation Object.
        operation: SourceLocation,
    },
    /// Target named by `operationRef`.
    OperationRef {
        /// Declared `operationRef` URI reference.
        value: Located<String>,
        /// The resolved Operation Object.
        operation: SourceLocation,
    },
}
