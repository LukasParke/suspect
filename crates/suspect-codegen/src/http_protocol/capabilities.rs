use std::collections::BTreeSet;

use serde::Serialize;

/// An adapter assertion, not a claim that any particular native adapter has
/// passed its wire, codec, resource-limit, and documentation gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Operations without a (non-empty) `operationId`.
    UnnamedOperations,
    /// Fixed methods beyond GET/POST/PUT/PATCH/DELETE (HEAD, OPTIONS, TRACE, QUERY).
    AdditionalMethods,
    /// Case-preserved `additionalOperations` tokens, including lowercase fixed
    /// spellings which transports such as Fetch might otherwise normalize.
    CustomMethods,
    /// Plain `http://` server URLs; the default profile accepts only HTTPS.
    HttpServers,
    /// Relative server URLs, including the implicit `/` default server.
    RelativeServers,
    /// Resolve relative server URLs using their declaring document's effective
    /// retrieval URL (or an explicit caller override), never its `$self`/`$id`.
    DocumentRelativeServers,
    /// More than one server candidate for an operation.
    MultipleServers,
    /// Server URL templates with `variables`.
    ServerVariables,
    /// Undeclared, empty, or optional (`{}`) security requirements.
    AnonymousSecurity,
    /// More than one security requirement alternative.
    SecurityAlternatives,
    /// Requirement objects that combine several schemes at once.
    ConjunctiveSecurity,
    /// HTTP `basic` authentication.
    HttpBasic,
    /// `apiKey` schemes in a header, query, or cookie.
    ApiKeys,
    /// `oauth2` schemes and their flows.
    OAuth2,
    /// `openIdConnect` schemes.
    OpenIdConnect,
    /// Role lists on non-OAuth security requirements (OAS 3.1+).
    SecurityRoles,
    /// Parameter serializations other than simple path or form query values.
    ParameterStyles,
    /// `in: header` parameters.
    HeaderParameters,
    /// `in: cookie` parameters.
    CookieParameters,
    /// `allowReserved` expansion of parameter values.
    ReservedParameters,
    /// Parameters serialized through `content` instead of `schema`.
    ContentParameters,
    /// `in: querystring` parameters (OAS 3.2).
    QuerystringParameters,
    /// Form-encoded `in: querystring` parameters.
    QuerystringForm,
    /// Status range keys such as `4XX`.
    RangeResponses,
    /// `default` response keys.
    DefaultResponses,
    /// Valid absence of Operation.responses (OAS 3.1+). All actual statuses
    /// remain undeclared; no default success/body type is invented.
    UndeclaredResponses,
    /// Bodies that declare more than one media type.
    MultipleMediaTypes,
    /// Wildcard media ranges such as `text/*`.
    MediaRanges,
    /// Media types with parameters such as `charset`.
    MediaTypeParameters,
    /// JSON media other than exactly `application/json` (`+json` suffixes).
    StructuredJsonMedia,
    /// JSON media without a schema.
    SchemaFreeJson,
    /// Text bodies decoded as UTF-8 strings.
    TextBodies,
    /// Opaque byte bodies.
    BinaryBodies,
    /// Responses with no or empty `content`.
    UndeclaredResponseBody,
    /// Declared response headers.
    ResponseHeaders,
    /// Declared response links.
    ResponseLinks,
    /// `application/x-www-form-urlencoded` bodies.
    FormBodies,
    /// `multipart/*` bodies.
    MultipartBodies,
    /// Per-part `encoding` objects, including part headers.
    PartEncodings,
    /// Multipart bodies using `prefixEncoding`/`itemEncoding` (OAS 3.2).
    PositionalMultipart,
    /// `text/event-stream` response streams.
    ServerSentEvents,
    /// Line-delimited JSON streams.
    JsonLines,
    /// OAS 3.0 documents; 3.1 is the baseline.
    OpenApi30,
    /// OAS 3.2 documents.
    OpenApi32,
    /// The native codec profile understands canonical schema resource identity.
    SchemaResources,
    /// The native codec profile implements dynamic scope rather than replacing
    /// `$dynamicRef` with its initial target or a candidate chosen at generation.
    DynamicSchemaReferences,
}

impl Capability {
    /// Complete planner feature vocabulary, useful for a separately verified
    /// reference interpreter. This is deliberately not an adapter default.
    pub const ALL: &'static [Self] = &[
        Self::UnnamedOperations,
        Self::AdditionalMethods,
        Self::CustomMethods,
        Self::HttpServers,
        Self::RelativeServers,
        Self::DocumentRelativeServers,
        Self::MultipleServers,
        Self::ServerVariables,
        Self::AnonymousSecurity,
        Self::SecurityAlternatives,
        Self::ConjunctiveSecurity,
        Self::HttpBasic,
        Self::ApiKeys,
        Self::OAuth2,
        Self::OpenIdConnect,
        Self::SecurityRoles,
        Self::ParameterStyles,
        Self::HeaderParameters,
        Self::CookieParameters,
        Self::ReservedParameters,
        Self::ContentParameters,
        Self::QuerystringParameters,
        Self::QuerystringForm,
        Self::RangeResponses,
        Self::DefaultResponses,
        Self::UndeclaredResponses,
        Self::MultipleMediaTypes,
        Self::MediaRanges,
        Self::MediaTypeParameters,
        Self::StructuredJsonMedia,
        Self::SchemaFreeJson,
        Self::TextBodies,
        Self::BinaryBodies,
        Self::UndeclaredResponseBody,
        Self::ResponseHeaders,
        Self::ResponseLinks,
        Self::FormBodies,
        Self::MultipartBodies,
        Self::PartEncodings,
        Self::PositionalMultipart,
        Self::ServerSentEvents,
        Self::JsonLines,
        Self::OpenApi30,
        Self::OpenApi32,
        Self::SchemaResources,
        Self::DynamicSchemaReferences,
    ];
}

/// Explicit, versioned departures from ordinary OpenAPI semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
pub enum CompatibilityProfile {
    /// Interpret the legacy `type: string, format: binary` marker as raw bytes
    /// in OAS 3.1/3.2 *binary* media/parts. No JSON null stand-in is constructed.
    /// This does not affect JSON media, base64 strings, or streaming semantics.
    #[serde(rename = "legacy-binary-string-v1")]
    LegacyBinaryStringV1,
    /// Interpret the OAS 3.0 `nullable` keyword on OAS 3.1/3.2 schema nodes with
    /// its 3.0 semantics: `nullable: true` appends `"null"` to the same-object
    /// type for wire decode/encode purposes, and `nullable: false` removes
    /// `"null"` from a type array. This does not affect OAS 3.0 documents
    /// (whose dialect already gives `nullable` that meaning) and does not
    /// change enum/composition nullability.
    #[serde(rename = "oas30-nullable-in-3.1-v1")]
    Oas30NullableIn31V1,
    /// Normalize legacy colon path-template segments (`/keys/:hash`) to OAS
    /// brace expressions (`/keys/{hash}`) when the path item declares a
    /// required path parameter of that exact name. A colon segment without a
    /// matching declared parameter is still refused, never guessed.
    #[serde(rename = "colon-path-parameters-v1")]
    ColonPathParametersV1,
    /// Admit a schemaless `text/event-stream` response — one whose media object
    /// declares no schema, or only a plain free-form schema that does not
    /// declare event structure — as an untyped frame stream. Declared
    /// structured item schemas and JSON-lines framing keep their ordinary
    /// strict admission, and non-stream media are unaffected.
    #[serde(rename = "schemaless-stream-events-v1")]
    SchemalessStreamEventsV1,
    /// Route schemas with `allOf` intersections through the scoped (checked)
    /// model/codec path instead of the unscoped projection. Required for
    /// contracts that compose object carriers through `allOf` where the
    /// unscoped path would refuse them.
    #[serde(rename = "all-of-scoped-mode-v1")]
    AllOfScopedModeV1,
}

impl CompatibilityProfile {
    /// Closed, versioned interpretation choices advertised by the canonical CLI.
    pub const ALL: &'static [Self] = &[
        Self::LegacyBinaryStringV1,
        Self::Oas30NullableIn31V1,
        Self::ColonPathParametersV1,
        Self::SchemalessStreamEventsV1,
        Self::AllOfScopedModeV1,
    ];

    /// Stable configuration name, identical to the serialized form.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LegacyBinaryStringV1 => "legacy-binary-string-v1",
            Self::Oas30NullableIn31V1 => "oas30-nullable-in-3.1-v1",
            Self::ColonPathParametersV1 => "colon-path-parameters-v1",
            Self::SchemalessStreamEventsV1 => "schemaless-stream-events-v1",
            Self::AllOfScopedModeV1 => "all-of-scoped-mode-v1",
        }
    }
}

/// Finite transport bounds, independent of JSON Schema evaluation. Zero bounds
/// are permitted (only an empty input fits); unbounded values are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ByteLimits {
    pub(super) body: u64,
    pub(super) part: u64,
    pub(super) stream_item: u64,
}

impl ByteLimits {
    /// Limits for a whole body, one multipart part, and one stream item, in bytes.
    #[must_use]
    pub const fn new(body: u64, part: u64, stream_item: u64) -> Self {
        Self {
            body,
            part,
            stream_item,
        }
    }
    /// Maximum bytes in one whole request or response body.
    #[must_use]
    pub const fn body(self) -> u64 {
        self.body
    }
    /// Maximum bytes in one multipart part.
    #[must_use]
    pub const fn part(self) -> u64 {
        self.part
    }
    /// Maximum bytes in one stream frame or line.
    #[must_use]
    pub const fn stream_item(self) -> u64 {
        self.stream_item
    }
}

impl Default for ByteLimits {
    fn default() -> Self {
        Self::new(8 * 1024 * 1024, 8 * 1024 * 1024, 1024 * 1024)
    }
}

/// Explicit native adapter capabilities. The default is the existing strict
/// JSON/exact-status/bearer/static-HTTPS wire profile. Schema/compiler guards
/// and native naming/resource limits remain the adapter's responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub(super) adapter: String,
    pub(super) enabled: BTreeSet<Capability>,
    pub(super) limits: ByteLimits,
    pub(super) profiles: BTreeSet<CompatibilityProfile>,
}

impl Capabilities {
    /// Named adapter profile with exactly `enabled`, default limits and no profiles.
    #[must_use]
    pub fn for_adapter(
        adapter: impl Into<String>,
        enabled: impl IntoIterator<Item = Capability>,
    ) -> Self {
        Self {
            adapter: adapter.into(),
            enabled: enabled.into_iter().collect(),
            limits: ByteLimits::default(),
            profiles: BTreeSet::new(),
        }
    }
    /// Enable one more capability.
    #[must_use]
    pub fn with(mut self, capability: Capability) -> Self {
        self.enabled.insert(capability);
        self
    }
    /// Replace the transport byte limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ByteLimits) -> Self {
        self.limits = limits;
        self
    }
    /// Opt in to one compatibility profile.
    #[must_use]
    pub fn with_profile(mut self, profile: CompatibilityProfile) -> Self {
        self.profiles.insert(profile);
        self
    }
    /// Whether the adapter asserts `capability`.
    #[must_use]
    pub fn supports(&self, capability: Capability) -> bool {
        self.enabled.contains(&capability)
    }
    /// Adapter profile name recorded in plans and captures.
    #[must_use]
    pub fn adapter(&self) -> &str {
        &self.adapter
    }
    /// Every capability the adapter asserts.
    #[must_use]
    pub fn enabled(&self) -> &BTreeSet<Capability> {
        &self.enabled
    }
    /// Transport byte limits.
    #[must_use]
    pub fn limits(&self) -> ByteLimits {
        self.limits
    }
    /// Compatibility profiles the adapter opted in to.
    #[must_use]
    pub fn profiles(&self) -> &BTreeSet<CompatibilityProfile> {
        &self.profiles
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::for_adapter("strict-json-v1", [])
    }
}
