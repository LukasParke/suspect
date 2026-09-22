//! Immutable native execution and metadata descriptors, emitted from typed plans.

use super::{JsonLimits, Media, Parameter, ResponseSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub document: &'static str,
    pub pointer: &'static str,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    pub use_site: Source,
    pub terminal: Source,
    pub references: &'static [Source],
    pub use_site_resource: Option<ResourceContext>,
    pub terminal_resource: Option<ResourceContext>,
    pub reference_resources: &'static [Option<ResourceContext>],
}
/// Logical reference metadata beside physical source ownership. These values
/// never replace a Source or select a schema/dynamic-reference execution target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceContext {
    pub source: Source,
    pub resource: Source,
    pub kind: ResourceKind,
    pub canonical_uri: &'static str,
    pub base_uri: &'static str,
    pub base_source: Option<Source>,
    pub scope_address: &'static str,
    pub schema_root: Option<Source>,
    pub aliases: &'static [&'static str],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    OpenApiDocument,
    Schema,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiUrlBase {
    ServerDocument,
    EffectiveServer,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocatedText {
    pub source: Source,
    pub value: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub request: usize,
    pub response: usize,
    pub part: usize,
    pub item: usize,
    pub chunk: usize,
    pub header: usize,
}

#[derive(Debug)]
pub struct Operation {
    pub operation_id: &'static str,
    pub source: Source,
    pub provenance: Provenance,
    pub method: &'static str,
    pub path_template: &'static str,
    pub servers: &'static [Server],
    pub security: Security,
    pub parameters: &'static [Parameter],
    pub request_media: &'static [Media],
    pub responses: &'static [ResponseSpec],
    pub accept: &'static str,
    pub limits: Limits,
    pub json_limits: JsonLimits,
}
#[derive(Debug, Clone, Copy)]
pub struct Server {
    pub source: Source,
    /// Effective physical retrieval document, independent of logical resources.
    pub document_base: Source,
    pub provenance: Option<Provenance>,
    pub template: &'static str,
    pub name: Option<LocatedText>,
    pub description: Option<LocatedText>,
    pub variables: &'static [ServerVariable],
}
impl Server {
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::ServerDocument
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ServerVariable {
    pub source: Source,
    pub name: &'static str,
    pub default: LocatedText,
    pub values: &'static [LocatedText],
    pub description: Option<LocatedText>,
}

#[derive(Debug, Clone, Copy)]
pub enum Security {
    Undeclared(Source),
    NoAuth(Source),
    Alternatives(&'static [SecurityAlternative]),
}
#[derive(Debug, Clone, Copy)]
pub struct SecurityAlternative {
    pub source: Source,
    pub requirements: &'static [CredentialRequirement],
}
#[derive(Debug, Clone, Copy)]
pub enum Permissions {
    Scopes(&'static [LocatedText]),
    Roles(&'static [LocatedText]),
}
#[derive(Debug, Clone, Copy)]
pub enum CredentialKind {
    Bearer {
        format: Option<LocatedText>,
    },
    Basic,
    ApiKey {
        location: super::ParameterLocation,
        name: LocatedText,
    },
    OAuth2 {
        flows: &'static [OAuthFlow],
        metadata_url: Option<LocatedText>,
    },
    OpenIdConnect {
        discovery_url: LocatedText,
    },
}
impl CredentialKind {
    /// OAuth/OIDC endpoint URLs are metadata relative to the selected API
    /// server. No discovery, token acquisition or URI rewriting is performed.
    pub fn url_base(&self) -> Option<ApiUrlBase> {
        matches!(self, Self::OAuth2 { .. } | Self::OpenIdConnect { .. })
            .then_some(ApiUrlBase::EffectiveServer)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct CredentialRequirement {
    pub source: Source,
    pub name: &'static str,
    pub scheme: Provenance,
    pub permissions: Permissions,
    pub kind: CredentialKind,
    pub description: Option<LocatedText>,
}
#[derive(Debug, Clone, Copy)]
pub enum OAuthFlowKind {
    Implicit,
    Password,
    ClientCredentials,
    AuthorizationCode,
    DeviceAuthorization,
}
#[derive(Debug, Clone, Copy)]
pub struct OAuthFlow {
    pub source: Source,
    pub kind: OAuthFlowKind,
    pub authorization_url: Option<LocatedText>,
    pub token_url: Option<LocatedText>,
    pub refresh_url: Option<LocatedText>,
    pub device_authorization_url: Option<LocatedText>,
    pub scopes: &'static [(&'static str, LocatedText)],
}
impl OAuthFlow {
    pub fn url_base(&self) -> ApiUrlBase {
        ApiUrlBase::EffectiveServer
    }
}

/// Exact immutable JSON metadata; number tokens are retained as tokens.
#[derive(Debug, Clone, Copy)]
pub enum MetadataValue {
    Null,
    Bool(bool),
    Number(&'static str),
    String(&'static str),
    Array(&'static [MetadataValue]),
    Object(&'static [(&'static str, MetadataValue)]),
}
#[derive(Debug, Clone, Copy)]
pub struct LocatedValue {
    pub source: Source,
    pub value: MetadataValue,
}
#[derive(Debug, Clone, Copy)]
pub enum LinkTarget {
    OperationId {
        value: LocatedText,
        operation: Source,
    },
    OperationRef {
        value: LocatedText,
        operation: Source,
    },
}
#[derive(Debug, Clone, Copy)]
pub struct Link {
    pub source: Provenance,
    pub name: &'static str,
    pub target: LinkTarget,
    pub parameters: &'static [(&'static str, LocatedValue)],
    pub request_body: Option<LocatedValue>,
    pub description: Option<LocatedText>,
    pub server: Option<Server>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub response: usize,
    pub media: Option<usize>,
    pub forbidden: bool,
}
