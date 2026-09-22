//! Canonical source-selected HTTP planning for the TypeScript SDK.
//!
//! The verified `crate::http_protocol` descriptors own admission and wire
//! semantics. Native names, model/codec bindings, Fetch policy and documentation
//! remain in this adapter. No emitted source is parsed to recover a plan.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_gen::filters::ts_identifier;
use suspect_ir::contract::{Contract, ParameterLocation, ParameterStyle, SchemaId, SourceId};

use super::ModelView;
use super::codecs::{CodecConfig, CodecPlan, plan_codecs_with_views};
use crate::OutFile;

/// Exact HTTP adapter source closure for compatibility/runtime provenance.
/// Paths are relative to `suspect-codegen/src`, not generated artifact text.
#[must_use]
pub fn source_assets() -> &'static [(&'static str, &'static [u8])] {
    &[
        (
            "typescript/validation-resources.ts",
            include_bytes!("validation-resources.ts"),
        ),
        ("typescript/uri.ts", include_bytes!("uri.ts")),
        ("typescript/http.rs", include_bytes!("http.rs")),
        (
            "typescript/http/credential-env.ts",
            include_bytes!("http/credential-env.ts"),
        ),
        (
            "typescript/http/protocol_emit.rs",
            include_bytes!("http/protocol_emit.rs"),
        ),
        (
            "typescript/http/stream_emit.rs",
            include_bytes!("http/stream_emit.rs"),
        ),
        (
            "typescript/http/native_examples.rs",
            include_bytes!("http/native_examples.rs"),
        ),
        (
            "typescript/http/interface.rs",
            include_bytes!("http/interface.rs"),
        ),
        (
            "typescript/http/runtime-protocol.ts",
            include_bytes!("http/runtime-protocol.ts"),
        ),
        ("typescript/http/types.ts", include_bytes!("http/types.ts")),
        (
            "typescript/http/common.ts",
            include_bytes!("http/common.ts"),
        ),
        ("typescript/http/wire.ts", include_bytes!("http/wire.ts")),
        ("typescript/http/media.ts", include_bytes!("http/media.ts")),
        (
            "typescript/http/security.ts",
            include_bytes!("http/security.ts"),
        ),
        ("typescript/http/parts.ts", include_bytes!("http/parts.ts")),
        (
            "typescript/http/multipart-style.ts",
            include_bytes!("http/multipart-style.ts"),
        ),
        (
            "typescript/http/streams.ts",
            include_bytes!("http/streams.ts"),
        ),
    ]
}
#[cfg(feature = "http-protocol")]
use crate::http_protocol as protocol;
#[cfg(feature = "http-protocol")]
mod incoming_emit;
#[cfg(feature = "http-protocol")]
mod interface;
#[cfg(feature = "http-protocol")]
mod native_examples;
#[cfg(feature = "http-protocol")]
mod oauth_emit;
#[cfg(feature = "http-protocol")]
mod pagination_emit;
#[cfg(feature = "http-protocol")]
pub(crate) mod protocol_emit;
#[cfg(feature = "http-protocol")]
mod stream_emit;
#[cfg(feature = "http-protocol")]
pub use native_examples::FirstRequest;

#[cfg(feature = "http-protocol")]
pub(super) fn credential_env_documentation(
    policy: &crate::credential_env::CredentialEnvPlan,
) -> String {
    protocol_emit::credential_env_docs(policy)
}

/// Explicit native capability profile. Compatibility departures are separate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HttpProfile {
    /// Original JSON/bearer/static-server surface, admitted by the verified core.
    #[default]
    StrictJson,
    /// Source-declared protocol families witnessed by the native adapter tests.
    ExpandedV1,
}

/// Explicit resource and codec policy for generated clients.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Model validation/conversion policy shared by every operation codec.
    pub codecs: CodecConfig,
    /// Default maximum decoded response bytes. Callers may choose a smaller limit.
    pub max_response_bytes: usize,
    /// Maximum encoded request bytes, including multipart framing.
    pub max_request_bytes: usize,
    /// Maximum bytes in one form/multipart part, checked separately from the body.
    pub max_part_bytes: usize,
    /// Maximum bytes in a stream item (including its framing).
    pub max_stream_item_bytes: usize,
    /// Maximum individual transport chunk retained by an item iterator.
    pub max_stream_buffer_bytes: usize,
    /// Maximum number of yielded items per response.
    pub max_stream_items: usize,
    pub profile: HttpProfile,
    /// Explicit OAS 3.1+ string/binary-marker compatibility; off by default.
    pub legacy_binary_string: bool,
    /// Explicit versioned source-interpretation profiles; off by default.
    #[cfg(feature = "http-protocol")]
    pub compatibility_profiles: std::collections::BTreeSet<protocol::CompatibilityProfile>,
    /// Explicit source-scheme to runtime environment-variable names; no values are read during generation.
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
            max_part_bytes: 1024 * 1024,
            max_stream_item_bytes: 64 * 1024,
            max_stream_buffer_bytes: 128 * 1024,
            max_stream_items: 100_000,
            profile: HttpProfile::StrictJson,
            legacy_binary_string: false,
            #[cfg(feature = "http-protocol")]
            compatibility_profiles: std::collections::BTreeSet::new(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        }
    }
}

impl HttpConfig {
    /// Opt in to the explicitly enumerated, native-tested protocol families.
    #[must_use]
    pub fn expanded() -> Self {
        Self {
            profile: HttpProfile::ExpandedV1,
            ..Self::default()
        }
    }

    /// Enumerated native capabilities and explicit compatibility profiles used by this configuration.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn capabilities(&self) -> protocol::Capabilities {
        use protocol::Capability::*;
        let enabled: &[protocol::Capability] = match self.profile {
            HttpProfile::StrictJson => &[],
            HttpProfile::ExpandedV1 => &[
                UnnamedOperations,
                AdditionalMethods,
                CustomMethods,
                HttpServers,
                RelativeServers,
                DocumentRelativeServers,
                MultipleServers,
                ServerVariables,
                AnonymousSecurity,
                SecurityAlternatives,
                ConjunctiveSecurity,
                HttpBasic,
                ApiKeys,
                OAuth2,
                OpenIdConnect,
                SecurityRoles,
                ParameterStyles,
                HeaderParameters,
                CookieParameters,
                ReservedParameters,
                ContentParameters,
                QuerystringParameters,
                QuerystringForm,
                RangeResponses,
                DefaultResponses,
                MultipleMediaTypes,
                MediaRanges,
                MediaTypeParameters,
                StructuredJsonMedia,
                SchemaFreeJson,
                TextBodies,
                BinaryBodies,
                UndeclaredResponseBody,
                ResponseHeaders,
                ResponseLinks,
                FormBodies,
                MultipartBodies,
                PartEncodings,
                PositionalMultipart,
                ServerSentEvents,
                JsonLines,
                OpenApi30,
                OpenApi32,
                SchemaResources,
                DynamicSchemaReferences,
            ],
        };
        let mut capabilities = protocol::Capabilities::for_adapter(
            match self.profile {
                HttpProfile::StrictJson => "typescript-fetch-strict-json-v1",
                HttpProfile::ExpandedV1 => "typescript-fetch-expanded-v1",
            },
            enabled.iter().copied(),
        )
        .with_limits(protocol::ByteLimits::new(
            self.max_response_bytes.max(self.max_request_bytes) as u64,
            self.max_part_bytes as u64,
            self.max_stream_item_bytes as u64,
        ));
        if self.legacy_binary_string {
            capabilities =
                capabilities.with_profile(protocol::CompatibilityProfile::LegacyBinaryStringV1);
        }
        for profile in &self.compatibility_profiles {
            capabilities = capabilities.with_profile(*profile);
        }
        capabilities
    }

    /// Select a native capability profile without changing wire semantics.
    #[must_use]
    pub fn with_profile(mut self, profile: HttpProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Explicitly enable a versioned protocol compatibility departure.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn with_compatibility_profile(mut self, profile: protocol::CompatibilityProfile) -> Self {
        if profile == protocol::CompatibilityProfile::LegacyBinaryStringV1 {
            self.legacy_binary_string = true;
        }
        self.compatibility_profiles.insert(profile);
        self
    }
}

pub use crate::http_contract::HttpDiagnostic;

pub struct HttpPlan {
    contract: Arc<Contract>,
    operations: Vec<PlannedOperation>,
    codecs: CodecPlan,
    files: Vec<OutFile>,
    examples: crate::examples::ExamplePlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    config: HttpConfig,
    #[cfg(feature = "http-protocol")]
    protocol: protocol::ProtocolPlan,
    /// Compiled pagination policy, present exactly when SDK defaults were configured.
    #[cfg(feature = "http-protocol")]
    pagination: Option<protocol::PaginationOutcome>,
    /// Compiled OAuth lifecycle policy, present exactly when SDK defaults were configured.
    #[cfg(feature = "http-protocol")]
    oauth: Option<protocol::OAuthPlan>,
    /// Compiled typed-stream semantics for every declared operation stream media.
    #[cfg(feature = "http-protocol")]
    stream_semantics: protocol::StreamSemanticsPlan,
    /// Compiled incoming webhook/callback receipts over the whole contract.
    #[cfg(feature = "http-protocol")]
    incoming: protocol::IncomingPlan,
    #[cfg(feature = "http-protocol")]
    first_request: Option<FirstRequest>,
}

impl std::fmt::Debug for HttpPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpPlan")
            .field("operations", &self.operations.len())
            .field("codecs", &self.codecs)
            .field("artifacts", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl HttpPlan {
    /// Source-bound runtime credential defaults, separate from source security semantics.
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

    /// Original immutable, admitted wire descriptors, including provenance.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn protocol(&self) -> &protocol::ProtocolPlan {
        &self.protocol
    }
    /// A native first-call recipe, lowered from the same typed plan and examples.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn first_request(&self) -> Option<&FirstRequest> {
        self.first_request.as_ref()
    }
    /// Compiled pagination policy for this plan, when SDK defaults were configured.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn pagination(&self) -> Option<&protocol::PaginationOutcome> {
        self.pagination.as_ref()
    }
    /// Compiled OAuth lifecycle policy for this plan, when SDK defaults were configured.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn oauth(&self) -> Option<&protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// Compiled typed-stream semantics (declared event kinds, payload codecs,
    /// sentinel and completion policies) for every operation stream media, in
    /// protocol-plan operation order. Emission consumes only the discriminated
    /// subset; every other operation keeps its existing untyped stream path.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn stream_semantics(&self) -> &protocol::StreamSemanticsPlan {
        &self.stream_semantics
    }
    /// Compiled incoming webhook/callback receipts for the whole contract,
    /// independent of operation selection. Empty when the source declares none.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn incoming(&self) -> &protocol::IncomingPlan {
        &self.incoming
    }
    /// Whether this plan emits the generated `typescript/oauth.ts` module:
    /// exactly when a compiled scheme carries an executable flow.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn oauth_emitted(&self) -> bool {
        self.oauth.as_ref().is_some_and(oauth_emit::has_usable)
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
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        self.files.clone()
    }
}

#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub description: String,
    pub function_name: String,
    pub input_type: String,
    pub success_type: String,
    pub error_type: String,
    pub error_guard: String,
    pub method: String,
    pub path: String,
    pub server: String,
    pub security_scheme_name: String,
    pub security_source: SourceId,
    pub security_use_source: SourceId,
    pub security_definition_source: SourceId,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
    #[cfg(feature = "http-protocol")]
    protocol: protocol::OperationPlan,
    #[cfg(feature = "http-protocol")]
    interface: Value,
}

impl PlannedOperation {
    /// Complete protocol view. Legacy JSON fields are only a baseline projection.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn protocol(&self) -> &protocol::OperationPlan {
        &self.protocol
    }

    /// Source-location/prose-free native interface emitted from this exact plan.
    /// Model bindings, multipart/stream shapes and default-call signatures remain explicit.
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn interface(&self) -> &Value {
        &self.interface
    }

    #[must_use]
    pub fn input_optional(&self) -> bool {
        #[cfg(feature = "http-protocol")]
        {
            !self
                .protocol
                .parameters()
                .iter()
                .any(protocol::ParameterPlan::required)
                && self.protocol.body().is_none_or(|body| !body.required())
        }
        #[cfg(not(feature = "http-protocol"))]
        {
            !self.parameters.iter().any(|p| p.required)
                && self.body.as_ref().is_none_or(|b| !b.required)
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub source: SourceId,
    pub wire_name: String,
    pub native_name: String,
    pub location: ParameterLocation,
    pub required: bool,
    pub schema: SchemaId,
    pub style: ParameterStyle,
    pub explode: bool,
    pub array: bool,
    #[cfg(feature = "http-protocol")]
    protocol: protocol::ParameterPlan,
}
impl PlannedParameter {
    #[cfg(feature = "http-protocol")]
    #[must_use]
    pub fn protocol(&self) -> &protocol::ParameterPlan {
        &self.protocol
    }
}
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub media_source: SourceId,
    pub required: bool,
    pub media_type: String,
    pub schema: SchemaId,
}
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub source: SourceId,
    pub media_source: SourceId,
    pub status: u16,
    pub media_type: String,
    pub schema: SchemaId,
}

struct HttpSymbols {
    request: BTreeMap<SchemaId, String>,
    response: BTreeMap<SchemaId, String>,
    credential_env_helper: Option<String>,
}

/// Plan a complete immutable HTTP artifact set from exactly the selected operation IDs.
#[cfg(feature = "http-protocol")]
pub fn plan_http(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    if config.max_response_bytes == 0 || config.max_response_bytes > 9_007_199_254_740_991usize {
        errors.push(diag(
            &contract,
            selected
                .first()
                .cloned()
                .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default())),
            "http-resource-policy",
            "max_response_bytes must be a positive JavaScript safe integer",
        ));
    }
    for (name, value) in [
        ("max_request_bytes", config.max_request_bytes),
        ("max_part_bytes", config.max_part_bytes),
        ("max_stream_item_bytes", config.max_stream_item_bytes),
        ("max_stream_buffer_bytes", config.max_stream_buffer_bytes),
        ("max_stream_items", config.max_stream_items),
    ] {
        if value > 9_007_199_254_740_991usize {
            errors.push(diag(
                &contract,
                selected
                    .first()
                    .cloned()
                    .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default())),
                "http-resource-policy",
                format!("{name} must be a JavaScript safe integer"),
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let wire = match protocol::plan(&contract, selected, config.capabilities()).into_result() {
        Ok(wire) => wire,
        Err(shared) => {
            errors.extend(shared.into_iter().map(|d| HttpDiagnostic {
                source: d.source().source().clone(),
                at: d.source().span(),
                code: d.code(),
                message: d.message().to_owned(),
            }));
            return Err(errors);
        }
    };
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming = protocol::plan_incoming(&contract)?;
    // Incoming receipts extend the codec table only when declared: their body
    // schemas become actual codec inputs beside the selected operations'.
    let mut codec_roots: Vec<SchemaId> = wire.codec_roots().to_vec();
    if !incoming.is_empty() {
        codec_roots.extend(incoming.codec_roots().iter().cloned());
        codec_roots.sort();
        codec_roots.dedup();
    }
    let credential_env =
        crate::credential_env::plan(&contract, &wire, config.credential_env.as_ref())?;
    for operation in wire.operations() {
        for media in operation
            .body()
            .into_iter()
            .flat_map(protocol::BodyPlan::media)
            .chain(
                operation
                    .responses()
                    .iter()
                    .flat_map(protocol::ResponsePlan::media),
            )
        {
            let protocol::Representation::Multipart { multipart } = media.representation() else {
                continue;
            };
            let (parts, additional) = match multipart {
                protocol::MultipartPlan::Named {
                    parts, additional, ..
                } => (parts.as_slice(), additional),
                protocol::MultipartPlan::Positional { prefix, items, .. } => {
                    (prefix.as_slice(), items)
                }
            };
            for part in parts.iter().chain(match additional {
                protocol::AdditionalParts::Allowed(part) => Some(part.as_ref()),
                _ => None,
            }) {
                if let protocol::PartRepresentation::Style { serialization, .. } =
                    part.representation()
                {
                    if !matches!(media.media_type().range(), protocol::MediaRange::Concrete { type_name, subtype } if type_name=="multipart" && subtype=="form-data")
                    {
                        errors.push(diag(&contract,part.encoding_source().unwrap_or_else(||part.source()).terminal().source().clone(),"http-typescript-multipart-content-plan-required","RFC6570 style fields are ignored outside multipart/form-data; the core plan must preserve the content representation for this part"));
                        continue;
                    }
                    let expanded = match serialization {
                        protocol::ParameterSerialization::Style {
                            style: protocol::Style::DeepObject,
                            ..
                        } => true,
                        protocol::ParameterSerialization::Style {
                            style: protocol::Style::Form,
                            explode: true,
                            shape,
                            ..
                        } => !matches!(shape, protocol::WireShape::Scalar { .. }),
                        _ => false,
                    };
                    if expanded
                        && (part.multiplicity() == protocol::PartMultiplicity::RepeatedArrayItems
                            || matches!(multipart, protocol::MultipartPlan::Positional { .. }))
                    {
                        errors.push(diag(&contract,part.encoding_source().unwrap_or_else(||part.source()).terminal().source().clone(),"http-typescript-multipart-style-grouping","expanding one positional/repeated composite item into multiple physical parts has no recoverable item-group boundary"));
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Compiled pagination policy: SDK defaults are required, and a policy
    // failure is a plan failure like every other diagnostic.
    let pagination = match config.sdk_defaults.as_ref() {
        Some(defaults) => Some(protocol::plan_pagination(&contract, &wire, Some(defaults))?),
        None => None,
    };
    // Compiled OAuth lifecycle policy shares the pagination gate: without
    // configured client defaults the runtime stays exactly the pre-OAuth
    // emission, and configuration errors surface as the shared
    // HttpDiagnostics. Schemes without an executable flow emit nothing.
    let oauth = if config.sdk_defaults.is_some() {
        Some(protocol::plan_oauth(
            &contract,
            &wire,
            config.sdk_defaults.as_ref(),
        )?)
    } else {
        None
    };
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Emission stays conditional.
    let stream_semantics = protocol::plan_stream_semantics(&contract, &wire);
    let mut operations: Vec<_> = wire
        .operations()
        .iter()
        .map(|op| {
            let operation_id = op
                .operation_id()
                .map(|v| v.value().clone())
                .unwrap_or_else(|| format!("{} {}", op.method().as_str(), op.path()));
            let function_name = op
                .operation_id()
                .map_or_else(|| unnamed_function(op), |id| ts_identifier(id.value()));
            let stem = upper(&function_name);
            let mut parameters: Vec<PlannedParameter> = op
                .parameters()
                .iter()
                .map(|p| PlannedParameter {
                    native_name: parameter_member(p.name(), native_location(p.location())),
                    source: p.source().use_site().source().clone(),
                    wire_name: p.name().to_owned(),
                    location: native_location(p.location()),
                    required: p.required(),
                    schema: p.codec().schema().id().clone(),
                    style: native_style(p.serialization()),
                    explode: matches!(
                        p.serialization(),
                        protocol::ParameterSerialization::Style { explode: true, .. }
                    ),
                    array: matches!(
                        p.serialization(),
                        protocol::ParameterSerialization::Style {
                            shape: protocol::WireShape::Array { .. },
                            ..
                        }
                    ),
                    protocol: p.clone(),
                })
                .collect();
            // Preserve existing path member names. Query collisions are qualified by location.
            let mut occupied: BTreeSet<_> = parameters
                .iter()
                .filter(|p| p.location == ParameterLocation::Path)
                .map(|p| p.native_name.clone())
                .chain(std::iter::once("body".to_owned()))
                .collect();
            for location in [
                ParameterLocation::Query,
                ParameterLocation::Querystring,
                ParameterLocation::Header,
                ParameterLocation::Cookie,
            ] {
                let mut allocated = Vec::new();
                for parameter in parameters.iter_mut().filter(|p| p.location == location) {
                    if occupied.contains(&parameter.native_name) {
                        parameter.native_name =
                            format!("{}_{}", location_name(location), parameter.native_name);
                    }
                    allocated.push(parameter.native_name.clone());
                }
                occupied.extend(allocated);
            }
            let requirement = op
                .security()
                .alternatives()
                .first()
                .and_then(|a| a.requirements().first());
            let source = op.source().terminal().source().clone();
            let security_source = match op.security() {
                protocol::SecurityPlan::Undeclared { source }
                | protocol::SecurityPlan::NoAuth { source }
                | protocol::SecurityPlan::Alternatives { source, .. } => source.source().clone(),
            };
            PlannedOperation {
                source: source.clone(),
                operation_id,
                description: op
                    .description()
                    .or(op.summary())
                    .map(|v| v.value().clone())
                    .unwrap_or_default(),
                function_name,
                input_type: format!("{stem}Input"),
                success_type: format!("{stem}Success"),
                error_type: format!("{stem}ApiError"),
                error_guard: format!("is{stem}ApiError"),
                method: op.method().as_str().to_owned(),
                path: op.path().to_owned(),
                server: op.servers().candidates()[0].template().to_owned(),
                security_scheme_name: requirement.map(|r| r.name().to_owned()).unwrap_or_default(),
                security_source,
                security_use_source: requirement
                    .map(|r| r.source().source().clone())
                    .unwrap_or_else(|| source.clone()),
                security_definition_source: requirement
                    .map(|r| r.scheme().terminal().source().clone())
                    .unwrap_or_else(|| source.clone()),
                parameters,
                body: op.body().and_then(|b| {
                    let media = b.media().first().filter(|_| b.media().len() == 1)?;
                    let protocol::Representation::Json { codec: Some(codec) } =
                        media.representation()
                    else {
                        return None;
                    };
                    Some(PlannedBody {
                        source: b.source().use_site().source().clone(),
                        media_source: media.source().use_site().source().clone(),
                        required: b.required(),
                        media_type: protocol_emit::canonical_media(media.media_type()),
                        schema: codec.schema().id().clone(),
                    })
                }),
                responses: op
                    .responses()
                    .iter()
                    .filter_map(|r| {
                        let protocol::ResponseStatus::Exact(status) = r.status() else {
                            return None;
                        };
                        if op.method() == protocol::Method::Head
                            || matches!(status, 100..=199 | 204 | 205 | 304)
                        {
                            return None;
                        }
                        let media = r.media().first().filter(|_| r.media().len() == 1)?;
                        let protocol::Representation::Json { codec: Some(codec) } =
                            media.representation()
                        else {
                            return None;
                        };
                        Some(PlannedResponse {
                            source: r.source().use_site().source().clone(),
                            media_source: media.source().use_site().source().clone(),
                            status,
                            media_type: protocol_emit::canonical_media(media.media_type()),
                            schema: codec.schema().id().clone(),
                        })
                    })
                    .collect(),
                protocol: op.clone(),
                interface: Value::Null,
            }
        })
        .collect();
    // Emission-ready pagination data for exactly the operations the policy paginated.
    let paginated = pagination.as_ref().map_or_else(Vec::new, |outcome| {
        pagination_emit::prepare(&operations, outcome)
    });
    // Emission-ready OAuth data: only schemes carrying at least one executable
    // (non-deprecated) flow participate.
    let oauth_schemes = oauth
        .as_ref()
        .map_or_else(Vec::new, |plan| oauth_emit::usable(plan));
    // Preserve neutral names for equivalent closures; annotated contracts use
    // explicitly bound request/response models and validation programs.
    let mut directional = false;
    for id in crate::schema_view::closure(&contract, &codec_roots) {
        if let Some(schema) = contract.schema(&id) {
            let raw = crate::schema_view::raw(schema);
            for keyword in ["readOnly", "writeOnly"] {
                if let Some(value) = raw.get(keyword) {
                    match value {
                        Value::Bool(value) => directional |= value,
                        _ => errors.push(diag(
                            &contract,
                            id.child(keyword),
                            "http-directional-metadata-invalid",
                            "readOnly and writeOnly must be booleans",
                        )),
                    }
                }
            }
        }
    }
    let mut public = [
        "createClient",
        "isSdkError",
        "operationMetadata",
        "Credentials",
        "ClientCredentials",
        "ClientOptions",
        "RuntimeClientOptions",
        "CallOptions",
        "Credential",
        "BasicCredential",
        "AuthorizationCredential",
        "ApiResponse",
        "DeclaredApiError",
        "MediaBody",
        "Part",
        "LinkMetadata",
        "StatusClass",
        "Digit",
        "Models",
        "Codecs",
        "JsonNumber",
        "executeOperation",
        "createRuntimeClient",
        "isDeclaredApiError",
        "freezeMetadata",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    public.extend(
        [
            "CredentialContext",
            "CredentialProvider",
            "ReadonlyResponseData",
            "ReadonlyBytes",
            "SdkError",
            "SdkFailureKind",
            "Fetch",
            "BinaryPart",
            "ServerChoice",
            "ServerPlan",
            "CredentialRequirement",
            "CredentialValue",
            "OAuthFlow",
            "Located",
            "Provenance",
            "ResourceContext",
            "SourceLocation",
            "Location",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    // The generated oauth.ts re-exports participate in the public symbol
    // allocation, so an operation or model name colliding with them is a
    // typed planning error rather than a broken generated package.
    for name in oauth_emit::exported_symbols(&oauth_schemes) {
        if !public.insert(name.to_owned()) {
            errors.push(diag(
                &contract,
                SourceId::new(contract.entry().clone(), Default::default()),
                "http-symbol-collision",
                format!("allocated public TypeScript OAuth symbol `{name}` collides or is unsafe"),
            ));
        }
    }
    for op in &operations {
        let mut allocated = vec![
            op.function_name.clone(),
            op.input_type.clone(),
            op.success_type.clone(),
            op.error_type.clone(),
            op.error_guard.clone(),
            format!("{}Source", op.function_name),
            format!("{}Wire", op.function_name),
            format!("{}Descriptor", op.function_name),
        ];
        if paginated
            .iter()
            .any(|entry| entry.function_name == op.function_name)
        {
            allocated.push(format!("{}Pages", op.function_name));
            allocated.push(format!("{}Items", op.function_name));
            allocated.push(format!("{}NextPage", op.function_name));
            allocated.push(format!("{}Item", upper(&op.function_name)));
        }
        for response in op.protocol().responses() {
            let stem = format!(
                "{}Response{}",
                upper(&op.function_name),
                upper(response.status_key())
            );
            if !response.headers().is_empty() {
                allocated.push(format!("{stem}Headers"));
            }
            if !response.links().is_empty() {
                allocated.push(format!("{stem}Links"));
            }
        }
        for name in allocated {
            if name == "__proto__" || !public.insert(name.clone()) {
                errors.push(diag(
                    &contract,
                    op.source.clone(),
                    "http-symbol-collision",
                    format!("allocated public TypeScript symbol `{name}` collides or is unsafe"),
                ));
            }
        }
        let mut members = BTreeSet::new();
        for p in &op.parameters {
            if p.native_name == "__proto__" || !members.insert(p.native_name.clone()) {
                errors.push(diag(
                    &contract,
                    p.source.clone(),
                    "http-input-member-collision",
                    "allocated TypeScript input member collides or is unsafe",
                ));
            }
        }
        if op.protocol().body().is_some() && !members.insert("body".into()) {
            errors.push(diag(
                &contract,
                op.source.clone(),
                "http-input-member-collision",
                "body collides with an allocated parameter member",
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let views: &[ModelView] = if directional {
        &[ModelView::Request, ModelView::Response]
    } else {
        &[ModelView::Neutral]
    };
    let mut codec_config = config.codecs.clone();
    codec_config.dialect = crate::schema_view::DialectPolicy::from_profiles(
        config.compatibility_profiles.iter().copied(),
    );
    let codecs = plan_codecs_with_views(contract.clone(), &codec_roots, views, codec_config)
        .map_err(|ds| {
            ds.into_iter()
                .map(|d| HttpDiagnostic {
                    source: d.source,
                    at: d.at,
                    code: d.code,
                    message: d.message,
                })
                .collect::<Vec<_>>()
        })?;
    let symbols_for = |view| {
        codecs
            .models()
            .symbols()
            .iter()
            .filter(|symbol| {
                symbol.view()
                    == if directional {
                        view
                    } else {
                        ModelView::Neutral
                    }
            })
            .map(|symbol| (symbol.source().clone(), symbol.name().to_owned()))
            .collect()
    };
    let symbols = HttpSymbols {
        request: symbols_for(ModelView::Request),
        response: symbols_for(ModelView::Response),
        credential_env_helper: credential_env.as_ref().map(|_| {
            let base = "__suspectCredentialEnv";
            let mut name = base.to_owned();
            let mut suffix = 0;
            while !public.insert(name.clone()) {
                suffix += 1;
                name = format!("{base}_{suffix}");
            }
            name
        }),
    };
    // Emission-ready typed stream decode data for exactly the operations whose
    // compiled stream plan carries a discriminated SSE event set. Allocated
    // event type names join the public symbol set so nothing else can take them.
    let stream_events = stream_emit::prepare(
        &operations,
        &symbols.response,
        &mut public,
        &stream_semantics,
    );
    // Emission-ready incoming receipt helpers. Decoded payloads bind response
    // views and constructed replies bind request views; allocated receipt names
    // join the public symbol set, and receipts the v1 helpers cannot express
    // surface as the shared plan errors.
    let mut incoming_errors = Vec::new();
    let incoming_helpers = incoming_emit::prepare(
        &contract,
        &incoming,
        &symbols.request,
        &symbols.response,
        &mut public,
        &mut incoming_errors,
    );
    errors.extend(incoming_errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let interfaces = interface::capture(&contract, &operations, &symbols, &config);
    for (operation, native) in operations.iter_mut().zip(interfaces) {
        operation.interface = native;
    }
    let examples = if codecs.validation_profile().0 == suspect_schema::OwnedProgram::V3_VERSION {
        crate::examples::plan_protocol_examples_v3(contract.clone(), &wire, Default::default())
    } else {
        crate::examples::plan_protocol_examples_v2(contract.clone(), &wire, Default::default())
    };
    let mut files = codecs.render();
    if credential_env.is_some() {
        files.push(OutFile {
            path: "typescript/http/credential-env.ts".into(),
            content: include_str!("http/credential-env.ts").into(),
        });
    }
    files.push(OutFile {
        path: "typescript/examples.json".into(),
        content: crate::http_examples::manifest(&examples),
    });
    files.push(OutFile {
        path: "typescript/examples.md".into(),
        content: crate::http_examples::markdown(&examples, "node dist/examples/validated.js"),
    });
    let (example_code, first_request) =
        native_examples::emit(&operations, &symbols, &examples, &codecs);
    files.push(OutFile {
        path: "typescript/examples/validated.ts".into(),
        content: example_code,
    });
    files.push(OutFile {
        path: "typescript/examples/first-request.ts".into(),
        content: first_request.as_ref().map_or_else(
            || "// No operations were selected.\nexport {};\n".into(),
            FirstRequest::local_source,
        ),
    });
    files.push(OutFile {
        path: "typescript/runtime.ts".into(),
        content: include_str!("http/runtime-protocol.ts").into(),
    });
    for (name, content) in [
        ("types", include_str!("http/types.ts")),
        ("common", include_str!("http/common.ts")),
        ("wire", include_str!("http/wire.ts")),
        ("media", include_str!("http/media.ts")),
        ("security", include_str!("http/security.ts")),
        ("parts", include_str!("http/parts.ts")),
        ("multipart-style", include_str!("http/multipart-style.ts")),
        ("streams", include_str!("http/streams.ts")),
    ] {
        files.push(OutFile {
            path: format!("typescript/http/{name}.ts"),
            content: content.into(),
        });
    }
    files.push(OutFile {
        path: "typescript/operations.ts".into(),
        content: protocol_emit::emit(
            &operations,
            &symbols,
            &config,
            credential_env.as_ref(),
            config.attribution.as_ref(),
            &paginated,
            &oauth_schemes,
            &stream_events,
        ),
    });
    if !paginated.is_empty() {
        files.push(OutFile {
            path: "typescript/pagination.ts".into(),
            content: pagination_emit::emit_library(&paginated),
        });
    }
    if !incoming_helpers.is_empty() {
        files.push(OutFile {
            path: "typescript/incoming.ts".into(),
            content: incoming_emit::emit(&incoming_helpers),
        });
    }
    if !oauth_schemes.is_empty() {
        files.push(OutFile {
            path: "typescript/oauth.ts".into(),
            content: oauth_emit::emit(
                &oauth_schemes,
                &oauth_emit::no_replay_requirements(&oauth_schemes, &operations),
            ),
        });
    }
    files.push(OutFile {
        path: "typescript/http.md".into(),
        content: protocol_emit::docs(
            &operations,
            &config,
            directional,
            first_request.as_ref(),
            &stream_events,
        ),
    });
    files.push(OutFile {
        path: "typescript/http-manifest.json".into(),
        content: serde_json::to_string_pretty(&protocol_emit::manifest(
            &operations,
            &symbols,
            &config,
            &wire,
            directional,
            credential_env.as_ref(),
        ))
        .expect("HTTP manifest is serializable"),
    });
    for file in &mut files {
        match file.path.as_str() {
            "typescript/docs-manifest.json" => {
                let mut manifest: Value =
                    serde_json::from_str(&file.content).expect("codec manifest");
                manifest["httpImplemented"] = json!(true);
                manifest["httpManifest"] = json!("http-manifest.json");
                file.content = serde_json::to_string_pretty(&manifest).unwrap();
            }
            "typescript/typedoc.json" => {
                let mut options: Value =
                    serde_json::from_str(&file.content).expect("native docs options");
                options["entryPoints"] = json!(["models.ts", "model-codecs.ts", "operations.ts"]);
                file.content = serde_json::to_string_pretty(&options).unwrap();
            }
            "typescript/tsconfig.docs.json" => {
                let mut options: Value =
                    serde_json::from_str(&file.content).expect("native docs compiler options");
                options["files"] = json!(["models.ts", "model-codecs.ts", "operations.ts"]);
                file.content = serde_json::to_string_pretty(&options).unwrap();
            }
            "typescript/docs-readme.md" => {
                file.content = format!(
                    "# OpenAPI HTTP operations\n\nThis package implements the source-selected HTTP protocol and its native model codecs. The HTTP manifest records the exact capability profile and source descriptors. Native documentation records inputs, response types and provenance. Credentials and transport policy are explicit. See http.md for native requests, streaming lifetimes and platform boundaries.\n\n## Operations\n\n{}\n\n## Models\n\n{}\n",
                    operations
                        .iter()
                        .map(|op| format!("- {{@link operations.{}}}", op.function_name))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    codecs
                        .models()
                        .symbols()
                        .iter()
                        .map(|model| format!("- {{@link models.{}}}", model.name()))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            _ => {}
        }
        if codecs.validation_profile().0 == suspect_schema::OwnedProgram::V2_VERSION
            && matches!(
                file.path.as_str(),
                "typescript/http.md" | "typescript/docs-readme.md"
            )
        {
            file.content
                .push_str(super::validation::SCOPED_DOCUMENTATION);
        }
        if codecs.validation_profile().0 == suspect_schema::OwnedProgram::V3_VERSION
            && matches!(
                file.path.as_str(),
                "typescript/http.md" | "typescript/docs-readme.md"
            )
        {
            file.content
                .push_str(super::validation::RESOURCE_DOCUMENTATION);
        }
        if let Some(policy) = &credential_env
            && matches!(
                file.path.as_str(),
                "typescript/http.md" | "typescript/docs-readme.md"
            )
        {
            file.content
                .push_str(&protocol_emit::credential_env_docs(policy));
        }
    }
    Ok(HttpPlan {
        contract,
        operations,
        codecs,
        files,
        examples,
        credential_env,
        config,
        protocol: wire,
        pagination,
        oauth,
        stream_semantics,
        incoming,
        first_request,
    })
}

#[cfg(not(feature = "http-protocol"))]
pub fn plan_http(
    contract: Arc<Contract>,
    selected: &[SourceId],
    _config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    Err(vec![diag(
        &contract,
        selected
            .first()
            .cloned()
            .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default())),
        "http-protocol-feature-required",
        "TypeScript HTTP planning requires the http-protocol feature",
    )])
}

#[cfg(feature = "http-protocol")]
fn native_location(location: protocol::ParameterLocation) -> ParameterLocation {
    match location {
        protocol::ParameterLocation::Path => ParameterLocation::Path,
        protocol::ParameterLocation::Query => ParameterLocation::Query,
        protocol::ParameterLocation::Querystring => ParameterLocation::Querystring,
        protocol::ParameterLocation::Header => ParameterLocation::Header,
        protocol::ParameterLocation::Cookie => ParameterLocation::Cookie,
    }
}

#[cfg(feature = "http-protocol")]
fn unnamed_function(operation: &protocol::OperationPlan) -> String {
    let mut name = operation.method().as_str().to_ascii_lowercase();
    for segment in operation
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        if let Some(parameter) = segment
            .strip_prefix('{')
            .and_then(|segment| segment.strip_suffix('}'))
        {
            name.push_str("By");
            name.push_str(&crate::rust_models::pascal(parameter));
        } else {
            name.push_str(&crate::rust_models::pascal(segment));
        }
    }
    ts_identifier(&name)
}

#[cfg(feature = "http-protocol")]
fn native_style(serialization: &protocol::ParameterSerialization) -> ParameterStyle {
    let protocol::ParameterSerialization::Style { style, .. } = serialization else {
        return ParameterStyle::Form;
    };
    match style {
        protocol::Style::Simple => ParameterStyle::Simple,
        protocol::Style::Label => ParameterStyle::Label,
        protocol::Style::Matrix => ParameterStyle::Matrix,
        protocol::Style::Form => ParameterStyle::Form,
        protocol::Style::Cookie => ParameterStyle::Cookie,
        protocol::Style::SpaceDelimited => ParameterStyle::SpaceDelimited,
        protocol::Style::PipeDelimited => ParameterStyle::PipeDelimited,
        protocol::Style::DeepObject => ParameterStyle::DeepObject,
    }
}

fn parameter_member(name: &str, location: ParameterLocation) -> String {
    // Object's inherited callable members otherwise make ordinary {} inputs
    // incompatible with optional string/number properties of the same name.
    if matches!(
        name,
        "constructor"
            | "toString"
            | "toLocaleString"
            | "valueOf"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "__proto__"
            | "__defineGetter__"
            | "__defineSetter__"
            | "__lookupGetter__"
            | "__lookupSetter__"
    ) {
        format!("{}_{name}", location_name(location))
    } else {
        ts_identifier(name)
    }
}
fn source_json(s: &SourceId) -> Value {
    json!({"document":s.document().to_string(),"pointer":s.pointer()})
}
fn location_name(location: ParameterLocation) -> &'static str {
    match location {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Querystring => "querystring",
        ParameterLocation::Header => "header",
        ParameterLocation::Cookie => "cookie",
    }
}
fn src(s: &SourceId) -> String {
    format!("{}#{}", s.document(), s.pointer())
}
fn upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(x) => x.to_uppercase().collect::<String>() + c.as_str(),
    }
}
fn diag(
    c: &Contract,
    source: SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: c.source_span(&source).unwrap_or(0..0),
        source,
        code,
        message: message.into(),
    }
}
