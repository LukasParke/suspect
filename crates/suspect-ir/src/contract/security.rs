//! Security declarations and their explicit OR/AND requirement references.

use serde_json::Value;

use super::http::Object;
use super::{Contract, ParameterLocation, SecurityUse, SourceId};

/// OpenAPI security scheme families supported by the contract metadata layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecuritySchemeKind {
    /// `type: apiKey` — a key sent in a query, header, or cookie parameter.
    ApiKey,
    /// `type: http` — an RFC 7235 `Authorization` scheme such as Basic or Bearer.
    Http,
    /// `type: oauth2`.
    OAuth2,
    /// `type: openIdConnect`.
    OpenIdConnect,
    /// `type: mutualTLS`.
    MutualTls,
}

impl SecuritySchemeKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "apiKey" => Some(Self::ApiKey),
            "http" => Some(Self::Http),
            "oauth2" => Some(Self::OAuth2),
            "openIdConnect" => Some(Self::OpenIdConnect),
            "mutualTLS" => Some(Self::MutualTls),
            _ => None,
        }
    }
}

/// OpenAPI OAuth flow objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowKind {
    /// `implicit` flow.
    Implicit,
    /// `password` (resource owner credentials) flow.
    Password,
    /// `clientCredentials` flow.
    ClientCredentials,
    /// `authorizationCode` flow.
    AuthorizationCode,
    /// OAuth 2.0 Device Authorization Grant, introduced by OpenAPI 3.2.
    DeviceAuthorization,
}

impl OAuthFlowKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "implicit" => Some(Self::Implicit),
            "password" => Some(Self::Password),
            "clientCredentials" => Some(Self::ClientCredentials),
            "authorizationCode" => Some(Self::AuthorizationCode),
            "deviceAuthorization" => Some(Self::DeviceAuthorization),
            _ => None,
        }
    }
}

impl Contract {
    /// Security schemes declared in the entry document. A requirement in an
    /// external document resolves against that document via `SecurityUse::scheme`.
    pub fn security_schemes(&self) -> Vec<SecurityScheme<'_>> {
        let root = SourceId::new(self.entry().clone(), suspect_low::Pointer::root());
        Object::new(self, root.child("components"))
            .named("securitySchemes")
            .into_iter()
            .map(|(name, object)| SecurityScheme { name, object })
            .collect()
    }
}

impl<'a> SecurityUse<'a> {
    /// Resolve the requirement's scheme name in the source document's components.
    /// Missing declarations produce an error and return None, with no fallback to
    /// another document's similarly named scheme.
    pub fn scheme(&self) -> Option<SecurityScheme<'a>> {
        let root = SourceId::new(
            self.source().document().clone(),
            suspect_low::Pointer::root(),
        );
        let source = root
            .child("components")
            .child("securitySchemes")
            .child(self.name());
        self.contract.source(&source)?;
        Some(SecurityScheme {
            name: self.name(),
            object: Object::new(self.contract, source),
        })
    }
}

/// A Security Scheme Object/reference with its component/requirement name.
#[derive(Debug, Clone)]
pub struct SecurityScheme<'a> {
    name: &'a str,
    object: Object<'a>,
}

impl<'a> SecurityScheme<'a> {
    /// Component or requirement name of the scheme.
    pub fn name(&self) -> &'a str {
        self.name
    }
    /// Source of the scheme declaration (the `$ref` site when referenced).
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// Terminal reference target; `None` if a reference is cyclic or dangling.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    /// The declaration as written, which may be a `$ref` object.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Declared `type`; `None` when absent or unrecognized.
    pub fn kind(&self) -> Option<SecuritySchemeKind> {
        SecuritySchemeKind::parse(self.object.string("type")?)
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    /// `name` of the header, query, or cookie parameter (apiKey schemes).
    pub fn parameter_name(&self) -> Option<&'a str> {
        self.object.string("name")
    }
    /// `in` location (apiKey schemes); `path` and `querystring` are rejected.
    pub fn parameter_location(&self) -> Option<ParameterLocation> {
        match self.object.string("in")? {
            "query" => Some(ParameterLocation::Query),
            "header" => Some(ParameterLocation::Header),
            "cookie" => Some(ParameterLocation::Cookie),
            _ => None,
        }
    }
    /// `scheme` of an http scheme (`basic`, `bearer`, ...).
    pub fn http_scheme(&self) -> Option<&'a str> {
        self.object.string("scheme")
    }
    /// `bearerFormat` hint of an http bearer scheme.
    pub fn bearer_format(&self) -> Option<&'a str> {
        self.object.string("bearerFormat")
    }
    /// `openIdConnectUrl` discovery document of an openIdConnect scheme.
    pub fn open_id_connect_url(&self) -> Option<&'a str> {
        self.object.string("openIdConnectUrl")
    }
    /// `oauth2MetadataUrl` (RFC 8414); only read from OpenAPI 3.2 documents.
    pub fn oauth2_metadata_url(&self) -> Option<&'a str> {
        self.object.field_32("oauth2MetadataUrl")?.1.as_str()
    }
    /// `deprecated` flag; only read from OpenAPI 3.2 documents.
    pub fn deprecated(&self) -> Option<bool> {
        self.object.field_32("deprecated")?.1.as_bool()
    }
    /// OAuth flows under `flows`, excluding `x-` extensions.
    pub fn flows(&self) -> Vec<OAuthFlow<'a>> {
        self.object
            .named("flows")
            .into_iter()
            .filter(|(name, _)| !name.starts_with("x-"))
            .map(|(name, object)| OAuthFlow { name, object })
            .collect()
    }
}

/// An OAuth flow, retaining endpoints and named scopes/prose without selecting
/// credentials or implementing an authentication policy in the contract layer.
#[derive(Debug, Clone)]
pub struct OAuthFlow<'a> {
    name: &'a str,
    object: Object<'a>,
}

impl<'a> OAuthFlow<'a> {
    /// Key under `flows` (`implicit`, `authorizationCode`, ...).
    pub fn name(&self) -> &'a str {
        self.name
    }
    /// Flow kind from the key; `deviceAuthorization` is only recognized in
    /// OpenAPI 3.2 documents.
    pub fn kind(&self) -> Option<OAuthFlowKind> {
        let kind = OAuthFlowKind::parse(self.name)?;
        (kind != OAuthFlowKind::DeviceAuthorization
            || self
                .object
                .contract
                .openapi_version_at(&self.object.source)
                .starts_with("3.2."))
        .then_some(kind)
    }
    /// Source of the flow object.
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// The flow object as written.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Declared `authorizationUrl` (implicit and authorizationCode flows).
    pub fn authorization_url(&self) -> Option<&'a str> {
        self.object.string("authorizationUrl")
    }
    /// Declared `deviceAuthorizationUrl`; only read from OpenAPI 3.2 documents.
    pub fn device_authorization_url(&self) -> Option<&'a str> {
        self.object.field_32("deviceAuthorizationUrl")?.1.as_str()
    }
    /// Declared `tokenUrl`.
    pub fn token_url(&self) -> Option<&'a str> {
        self.object.string("tokenUrl")
    }
    /// Declared `refreshUrl`, if any.
    pub fn refresh_url(&self) -> Option<&'a str> {
        self.object.string("refreshUrl")
    }
    /// Borrowed scope-name → description map; invalid values are diagnosed.
    pub fn scopes(&self) -> Option<&'a serde_json::Map<String, Value>> {
        self.object.field("scopes")?.1.as_object()
    }
}
