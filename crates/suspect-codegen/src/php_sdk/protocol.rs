//! Allocated PHP protocol surfaces over the rich, source-checked HTTP plan.
use super::{HttpDiagnostic, PhpConfig, models};
use crate::http_protocol::{
    self as wire, AdditionalParts, MultipartPlan, PartRepresentation, Representation,
    ResponseStatus,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

#[path = "protocol_emit.rs"]
mod emit;
#[path = "oauth.rs"]
mod oauth;
#[path = "pagination.rs"]
mod pagination;
#[path = "stream.rs"]
mod stream;
#[path = "incoming.rs"]
mod incoming;

#[derive(Debug)]
pub struct SdkPlan {
    pub(super) core: super::ModelCore,
    pub surface: Surface,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    pagination: Option<crate::http_protocol::PaginationOutcome>,
    oauth: Option<crate::http_protocol::OAuthPlan>,
    /// Compiled typed-stream semantics, carried from the infallible shared
    /// planner. Emission stays conditional on discriminated SSE operations.
    stream_semantics: crate::http_protocol::StreamSemanticsPlan,
    /// Compiled incoming webhook/callback receipts over the whole contract,
    /// independent of operation selection. Empty when the source declares none.
    incoming: crate::http_protocol::IncomingPlan,
    /// Emission-ready incoming receipt helpers with allocated member names.
    incoming_receipts: Vec<incoming::Receipt>,
}
impl SdkPlan {
    pub fn config(&self) -> &PhpConfig {
        self.core.config()
    }
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    /// Compiled pagination selection carried from the configured SDK defaults.
    pub fn pagination(&self) -> Option<&crate::http_protocol::PaginationOutcome> {
        self.pagination.as_ref()
    }
    /// Compiled OAuth lifecycle selection carried from the configured SDK defaults.
    pub fn oauth(&self) -> Option<&crate::http_protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// Compiled typed-stream semantics for every operation stream media,
    /// carried from the infallible shared planner.
    pub fn stream_semantics(&self) -> &crate::http_protocol::StreamSemanticsPlan {
        &self.stream_semantics
    }
    /// Compiled incoming webhook/callback receipts for the whole contract,
    /// independent of operation selection. Empty when the source declares none.
    pub fn incoming(&self) -> &crate::http_protocol::IncomingPlan {
        &self.incoming
    }
    pub fn contract(&self) -> &Arc<Contract> {
        self.core.contract()
    }
    pub fn models(&self) -> &models::ModelPlan {
        self.core.models()
    }
    pub fn operations(&self) -> &[Operation] {
        &self.surface.operations
    }
    pub fn program(&self) -> &suspect_schema::OwnedProgram {
        self.core.program()
    }
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        self.core.examples()
    }
    pub fn render(&self) -> Vec<crate::OutFile> {
        emit::package(self)
    }
}

/// Construct a complete native surface from explicitly enabled protocol features.
/// Capability enablement is a native adapter decision, not a source override.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
    capabilities: wire::Capabilities,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_sdk_inner(contract, selected, config, capabilities, true)
}

pub(in crate::php_sdk) fn plan_sdk_inner(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
    capabilities: wire::Capabilities,
    scoped: bool,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_sdk_mode(contract, selected, config, capabilities, scoped, true)
}

pub(in crate::php_sdk) fn plan_sdk_mode(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
    capabilities: wire::Capabilities,
    scoped: bool,
    resources: bool,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let entry = SourceId::new(contract.entry().clone(), Default::default());
    let parts = config.package_name.split('/').collect::<Vec<_>>();
    let mut errors = Vec::new();
    if parts.len() != 2
        || !super::composer_part(parts[0], false)
        || !super::composer_part(parts[1], true)
        || config.package_name.len() > 128
    {
        errors.push(super::diagnostic(
            &contract,
            entry.clone(),
            "php-package-name",
            "Composer identity must be lowercase vendor/package",
        ));
    }
    if semver::Version::parse(&config.package_version).is_err() {
        errors.push(super::diagnostic(
            &contract,
            entry.clone(),
            "php-package-version",
            "package version must be exact SemVer",
        ));
    }
    if config.namespace.len() > 128 || !config.namespace.split('\\').all(models::identifier) {
        errors.push(super::diagnostic(
            &contract,
            entry.clone(),
            "php-namespace",
            "namespace must consist of non-reserved ASCII PHP identifiers",
        ));
    }
    if !super::valid_policy(&config)
        || [
            capabilities.limits().body(),
            capabilities.limits().part(),
            capabilities.limits().stream_item(),
        ]
        .iter()
        .any(|limit| *limit > i32::MAX as u64)
    {
        errors.push(super::diagnostic(
            &contract,
            entry.clone(),
            "php-resource-policy",
            "invalid finite PHP runtime policy",
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut used = models::reserved_symbols();
    let mut surface = plan(contract.clone(), selected, &config, capabilities, &mut used)?;
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming = crate::http_protocol::plan_incoming(&contract)?;
    let credential_env =
        crate::credential_env::plan_with_defaults(&contract, &surface.protocol, config.credential_env.as_ref(), config.sdk_defaults.as_ref())?;
    // Compiled pagination policy: SDK defaults are required, and a policy
    // failure is a plan failure like every other diagnostic.
    let pagination = match config.sdk_defaults.as_ref() {
        Some(defaults) => Some(crate::http_protocol::plan_pagination(
            &contract,
            &surface.protocol,
            Some(defaults),
        )?),
        None => None,
    };
    // Compiled OAuth lifecycle policy, mirrored from pagination: SDK defaults
    // are required, and a policy failure is a plan failure like every other
    // diagnostic.
    let oauth = match config.sdk_defaults.as_ref() {
        Some(defaults) => Some(crate::http_protocol::plan_oauth(
            &contract,
            &surface.protocol,
            Some(defaults),
        )?),
        None => None,
    };
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Emission stays conditional on
    // discriminated SSE operations and is never gated on SDK defaults.
    let stream_semantics = crate::http_protocol::plan_stream_semantics(&contract, &surface.protocol);
    // The emitted OAuth class names share the package namespace with native
    // models; reserve them only while OAuth emission participates, so
    // no-policy allocation behavior is unchanged. The replaying transport
    // class joins the reservation only while the replaying credential
    // wrapper participates, so non-replaying plans allocate unchanged.
    if let Some(oauth) = &oauth
        && oauth::emittable(oauth)
    {
        for name in oauth::CLASS_NAMES {
            used.insert(name.to_ascii_lowercase());
        }
        if oauth::replaying(oauth) {
            used.insert("replaytransport".to_owned());
        }
    }
    // The emitted Incoming class names join the reservation only while
    // receipts are declared, so receipt-less allocation behavior is unchanged.
    if incoming::emittable(&incoming) {
        for name in incoming::CLASS_NAMES {
            used.insert(name.to_ascii_lowercase());
        }
    }
    // The receipt payload and reply schemas join the named model plan under
    // the receipt's own stem, so the receipt codecs read like the receipts
    // they serve. Hints only apply outside /components/schemas (the shared
    // model-planner rule), so shared component schemas keep their source
    // names, and the planner's allocator keeps every name collision-free.
    if !incoming.is_empty() {
        for operation in incoming.operations() {
            let stem = models::pascal(operation.name());
            if let Some(body) = operation.request().body() {
                for (index, media) in body.media().iter().enumerate() {
                    if let Representation::Json { codec: Some(codec) } = media.representation() {
                        let suffix = if body.media().len() == 1 {
                            String::new()
                        } else {
                            media_name(media, index)
                        };
                        surface
                            .names
                            .entry(codec.schema().id().clone())
                            .or_insert_with(|| format!("{stem}Body{suffix}"));
                    }
                }
            }
            for response in operation.responses() {
                for (index, media) in response.media().iter().enumerate() {
                    if let Representation::Json { codec: Some(codec) } = media.representation() {
                        let suffix = if response.media().len() == 1 {
                            String::new()
                        } else {
                            media_name(media, index)
                        };
                        surface
                            .names
                            .entry(codec.schema().id().clone())
                            .or_insert_with(|| {
                                format!("{stem}Response{}{suffix}", response.status_key())
                            });
                    }
                }
            }
        }
    }
    for operation in &surface.operations {
        for response in &operation.responses {
            if let Payload::Object(name) = &response.payload
                && let Some(object) = surface.objects.iter().find(|object| &object.name == name)
            {
                for part in object.parts.iter().chain(object.extra.iter()) {
                    if matches!(
                        part.wire.representation(),
                        PartRepresentation::Style {
                            serialization: wire::ParameterSerialization::Style {
                                shape: wire::WireShape::Array { .. }
                                    | wire::WireShape::FlatObject { .. },
                                ..
                            },
                            ..
                        }
                    ) {
                        return Err(vec![diag(
                            &contract,
                            part.wire.source().use_site().source(),
                            "composite response form/part styles require an unambiguous native decoding profile",
                        )]);
                    }
                }
            }
        }
    }
    // Incoming receipts widen the directional-codec refusal closure exactly as
    // their compiled schemas join the codec table; receipt-less plans keep the
    // selected closure.
    let mut directional = surface.protocol.codec_schema_closure().to_vec();
    if !incoming.is_empty() {
        directional.extend(incoming.codec_schema_closure().iter().cloned());
        directional.sort();
        directional.dedup();
    }
    for source in &directional {
        for keyword in ["readOnly", "writeOnly"] {
            if contract
                .source(source)
                .and_then(|v| v.get(keyword))
                .is_some_and(|v| v != &serde_json::Value::Bool(false))
            {
                return Err(vec![diag(
                    &contract,
                    &source.child(keyword),
                    "directional PHP model codecs require explicit native admission",
                )]);
            }
        }
    }
    let compiler = suspect_schema::OwnedCompiler::new(config.validation.clone());
    // Incoming receipts extend the codec table only when declared: their body
    // schemas become actual codec inputs beside the selected operations'.
    let mut reachable = directional;
    if !incoming.is_empty() {
        reachable.extend(incoming.codec_roots().iter().cloned());
        reachable.sort();
        reachable.dedup();
    }
    let validation = if resources {
        compiler
            .compile_v2(contract.clone(), &reachable)
            .or_else(|_| compiler.compile_v3(contract.clone(), &reachable))
    } else if scoped {
        compiler.compile_v2(contract.clone(), &reachable)
    } else {
        compiler.compile(contract.clone(), &reachable)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "php-schema-compilation",
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = validation.program();
    super::native_program_check_mode(&program, scoped, resources).map_err(|error| {
        let source = error
            .source
            .as_ref()
            .and_then(super::program_source)
            .unwrap_or_else(|| entry.clone());
        vec![super::diagnostic(
            &contract,
            source,
            "php-validation-program",
            error.to_string(),
        )]
    })?;
    let models = if program.version == "suspect.validation.experimental.v3" {
        models::plan_resources(&contract, &reachable, &surface.names, &mut used)
    } else if program.version != "suspect.validation.experimental.v1"
        || crate::schema_view::has_intersections(&contract, &reachable)
    {
        models::plan_scoped(&contract, &reachable, &surface.names, &mut used)
    } else {
        models::plan_named(&contract, &reachable, &surface.names, &mut used)
    }?;
    let example_planner = if resources {
        crate::examples::plan_protocol_examples_v3
    } else if scoped {
        crate::examples::plan_protocol_examples_v2
    } else {
        crate::examples::plan_protocol_examples
    };
    let examples = example_planner(contract.clone(), &surface.protocol, Default::default());
    let samples = super::samples::plan(&models, &validation, &examples);
    // Emission-ready incoming receipt helpers. Receipts the PHP v1 helpers
    // cannot express surface as the shared plan errors instead of silent
    // skips.
    let mut incoming_errors = Vec::new();
    let incoming_receipts = incoming::prepare(&contract, &incoming, &models, &mut incoming_errors);
    if !incoming_errors.is_empty() {
        return Err(incoming_errors);
    }
    Ok(SdkPlan {
        core: super::ModelCore {
            contract,
            config,
            models,
            operations: Vec::new(),
            program,
            examples,
            samples,
        },
        surface,
        credential_env,
        pagination,
        oauth,
        stream_semantics,
        incoming,
        incoming_receipts,
    })
}

/// Implemented native protocol features. Default generation has no compatibility
/// profiles; legacy binary marker conversion requires an explicit versioned opt-in.
pub fn capabilities() -> wire::Capabilities {
    use wire::Capability::*;
    wire::Capabilities::for_adapter(
        "php-http-v2",
        [
            AdditionalMethods,
            CustomMethods,
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
            UndeclaredResponses,
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
            ServerSentEvents,
            JsonLines,
            OpenApi32,
            SchemaResources,
            DynamicSchemaReferences,
        ],
    )
}

#[derive(Debug, Clone)]
pub enum Payload {
    Schema(SchemaId),
    Json,
    Text,
    Bytes,
    NoBody,
    Object(String),
    Stream(SchemaId),
}
#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub schema: SchemaId,
    pub wire: wire::ParameterPlan,
}
#[derive(Debug, Clone)]
pub struct Media {
    pub source: SourceId,
    pub name: String,
    pub wrapper: Option<String>,
    pub payload: Payload,
    pub wire: wire::MediaPlan,
}
#[derive(Debug, Clone)]
pub struct Header {
    pub name: String,
    pub schema: SchemaId,
    pub wire: wire::HeaderPlan,
}
#[derive(Debug, Clone)]
pub struct HeaderObject {
    pub name: String,
    pub fields: Vec<Header>,
}
#[derive(Debug, Clone)]
pub struct Part {
    pub name: String,
    pub payload: Payload,
    pub wrapper: Option<String>,
    pub headers: HeaderObject,
    pub wire: wire::PartPlan,
}
#[derive(Debug, Clone)]
pub struct BodyObject {
    pub source: SourceId,
    pub name: String,
    pub parts: Vec<Part>,
    pub extra: Option<Part>,
    pub multipart: bool,
    pub rules: wire::ObjectRules,
}
#[derive(Debug, Clone)]
pub struct Response {
    pub source: SourceId,
    pub status_key: String,
    pub success: bool,
    pub name: String,
    pub payload: Payload,
    pub media: Option<Media>,
    pub headers: HeaderObject,
    pub wire: wire::ResponsePlan,
}
#[derive(Debug, Clone)]
pub struct Operation {
    pub source: SourceId,
    pub id: String,
    pub method: String,
    pub input: String,
    pub error: String,
    pub parameters: Vec<Parameter>,
    pub body: Vec<Media>,
    pub body_required: bool,
    pub responses: Vec<Response>,
    pub wire: wire::OperationPlan,
}
#[derive(Debug)]
pub struct Surface {
    pub protocol: wire::ProtocolPlan,
    pub operations: Vec<Operation>,
    pub objects: Vec<BodyObject>,
    pub names: BTreeMap<SchemaId, String>,
}

fn diag(contract: &Contract, source: &SourceId, message: &str) -> HttpDiagnostic {
    super::diagnostic(
        contract,
        source.clone(),
        "php-protocol-representation",
        message,
    )
}

/// Only validated adapters should supply capabilities at this seam. The PHP
/// backend's public admission table is enabled by its native protocol gates.
pub fn plan(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: &PhpConfig,
    capabilities: wire::Capabilities,
    used: &mut BTreeSet<String>,
) -> Result<Surface, Vec<HttpDiagnostic>> {
    let protocol = wire::plan(&contract, selected, capabilities)
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|error| HttpDiagnostic {
                    source: error.source().source().clone(),
                    at: error.source().span(),
                    code: error.code(),
                    message: error.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
    let mut surface = Surface {
        protocol,
        operations: Vec::new(),
        objects: Vec::new(),
        names: BTreeMap::new(),
    };
    let mut methods = BTreeSet::from(["__construct".into(), "exchange".into()]);
    if config.credential_env.is_some() {
        methods.insert("fromenv".into());
    }
    for op in surface.protocol.operations().to_vec() {
        let source = op.source().use_site().source().clone();
        let id = op
            .operation_id()
            .map(|v| v.value().clone())
            .ok_or_else(|| {
                vec![diag(
                    &contract,
                    &source,
                    "PHP requires an explicit operationId",
                )]
            })?;
        let stem = models::pascal(&id);
        let mut fields = BTreeSet::from(["body".into(), "options".into()]);
        let parameters = op
            .parameters()
            .iter()
            .map(|p| {
                let schema = p.codec().schema().id().clone();
                surface
                    .names
                    .entry(schema.clone())
                    .or_insert_with(|| format!("{stem}{}", models::pascal(p.name())));
                Parameter {
                    name: models::allocate(&models::member(p.name()), &mut fields),
                    schema,
                    wire: p.clone(),
                }
            })
            .collect();
        let mut body = Vec::new();
        if let Some(request) = op.body() {
            for (i, media) in request.media().iter().enumerate() {
                let suffix = if request.media().len() == 1 {
                    String::new()
                } else {
                    media_name(media, i)
                };
                body.push(media_plan(
                    &contract,
                    &mut surface,
                    media,
                    &format!("{stem}Body{suffix}"),
                    request.media().len() > 1,
                    used,
                )?);
            }
        }
        let mut responses = Vec::new();
        for response in op.responses() {
            let key = response.status_key().to_owned();
            let statuses: Vec<bool> = match response.status() {
                ResponseStatus::Exact(s) => vec![(200..300).contains(&s)],
                ResponseStatus::Range(n) => vec![n == 2],
                ResponseStatus::Default => vec![true, false],
            };
            let forbidden = op.method() == wire::Method::Head
                || matches!(
                    response.status(),
                    ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1)
                );
            let headers = header_object(
                &mut surface,
                response.headers(),
                &format!("{stem}Status{key}Headers"),
                used,
            );
            for success in statuses {
                let suffix = if key == "default" { "Default" } else { &key };
                let class = format!("{stem}Status{suffix}{}", if success { "" } else { "Error" });
                if !forbidden
                    && matches!(
                        response.status(),
                        ResponseStatus::Range(1..=3) | ResponseStatus::Default
                    )
                {
                    responses.push(Response {
                        source: response.source().use_site().source().clone(),
                        status_key: key.clone(),
                        success,
                        name: models::allocate(&format!("{class}NoBody"), used),
                        payload: Payload::NoBody,
                        media: None,
                        headers: headers.clone(),
                        wire: response.clone(),
                    });
                }
                if forbidden || response.media().is_empty() {
                    responses.push(Response {
                        source: response.source().use_site().source().clone(),
                        status_key: key.clone(),
                        success,
                        name: models::allocate(&class, used),
                        payload: if forbidden {
                            Payload::NoBody
                        } else {
                            Payload::Bytes
                        },
                        media: None,
                        headers: headers.clone(),
                        wire: response.clone(),
                    });
                } else {
                    for (i, media) in response.media().iter().enumerate() {
                        let suffix = if response.media().len() == 1 {
                            String::new()
                        } else {
                            media_name(media, i)
                        };
                        let planned = media_plan(
                            &contract,
                            &mut surface,
                            media,
                            &format!("{stem}Response{key}{suffix}"),
                            false,
                            used,
                        )?;
                        responses.push(Response {
                            source: response.source().use_site().source().clone(),
                            status_key: key.clone(),
                            success,
                            name: models::allocate(&format!("{class}{suffix}"), used),
                            payload: planned.payload.clone(),
                            media: Some(planned),
                            headers: headers.clone(),
                            wire: response.clone(),
                        });
                    }
                }
            }
        }
        surface.operations.push(Operation {
            source,
            id: id.clone(),
            method: models::allocate(&models::member(&id), &mut methods),
            input: models::allocate(&format!("{stem}Input"), used),
            error: models::allocate(&format!("{stem}ApiError"), used),
            parameters,
            body,
            body_required: op.body().is_some_and(|b| b.required()),
            responses,
            wire: op,
        });
    }
    let _ = config;
    Ok(surface)
}

fn media_name(media: &wire::MediaPlan, index: usize) -> String {
    let stem = match media.representation() {
        Representation::Json { .. } => "Json",
        Representation::Text { .. } => "Text",
        Representation::Binary { .. } => "Bytes",
        Representation::Form { .. } => "Form",
        Representation::Multipart { .. } => "Multipart",
        Representation::Stream { .. } => "Stream",
    };
    format!("{stem}{}", index + 1)
}
fn header_object(
    surface: &mut Surface,
    headers: &[wire::HeaderPlan],
    name: &str,
    used: &mut BTreeSet<String>,
) -> HeaderObject {
    let name = models::allocate(name, used);
    let mut members = BTreeSet::new();
    let fields = headers
        .iter()
        .map(|header| {
            let schema = header.codec().schema().id().clone();
            surface
                .names
                .entry(schema.clone())
                .or_insert_with(|| format!("{name}{}", models::pascal(header.name())));
            Header {
                name: models::allocate(&models::member(header.name()), &mut members),
                schema,
                wire: header.clone(),
            }
        })
        .collect();
    HeaderObject { name, fields }
}
fn media_plan(
    contract: &Contract,
    surface: &mut Surface,
    media: &wire::MediaPlan,
    name: &str,
    wrapper: bool,
    used: &mut BTreeSet<String>,
) -> Result<Media, Vec<HttpDiagnostic>> {
    let source = media.source().use_site().source().clone();
    let payload = match media.representation() {
        Representation::Json { codec: Some(codec) }
        | Representation::Text {
            codec: Some(codec), ..
        } => {
            let schema = codec.schema().id().clone();
            surface
                .names
                .entry(schema.clone())
                .or_insert_with(|| name.into());
            Payload::Schema(schema)
        }
        Representation::Json { codec: None } => Payload::Json,
        Representation::Text { codec: None, .. } => Payload::Text,
        Representation::Binary { .. } => Payload::Bytes,
        Representation::Stream { stream } => match stream.item_codec() {
            Some(codec) => {
                let schema = codec.schema().id().clone();
                surface
                    .names
                    .entry(schema.clone())
                    .or_insert_with(|| format!("{name}Item"));
                Payload::Stream(schema)
            }
            // A schemaless stream surfaces untyped parsed envelope values.
            None => Payload::Json,
        },
        Representation::Form { form } => {
            let class = body_object(
                contract,
                surface,
                form.rules(),
                form.fields(),
                form.additional(),
                name,
                false,
                used,
            )?;
            Payload::Object(class)
        }
        Representation::Multipart {
            multipart:
                MultipartPlan::Named {
                    rules,
                    parts,
                    additional,
                },
        } => {
            let class = body_object(
                contract, surface, rules, parts, additional, name, true, used,
            )?;
            Payload::Object(class)
        }
        Representation::Multipart {
            multipart: MultipartPlan::Positional { .. },
        } => {
            return Err(vec![diag(
                contract,
                &source,
                "PHP positional multipart has no admitted native carrier",
            )]);
        }
    };
    let wrapper = if wrapper
        || !matches!(
            media.media_type().range(),
            wire::MediaRange::Concrete { .. }
        ) {
        Some(models::allocate(&format!("{name}Representation"), used))
    } else {
        None
    };
    Ok(Media {
        source,
        name: name.into(),
        wrapper,
        payload,
        wire: media.clone(),
    })
}
#[allow(clippy::too_many_arguments)]
fn body_object(
    contract: &Contract,
    surface: &mut Surface,
    rules: &wire::ObjectRules,
    fields: &[wire::PartPlan],
    additional: &AdditionalParts,
    name: &str,
    multipart: bool,
    used: &mut BTreeSet<String>,
) -> Result<String, Vec<HttpDiagnostic>> {
    let name = models::allocate(name, used);
    let mut members = BTreeSet::from(["extra".into()]);
    let mut parts = Vec::new();
    for field in fields {
        let member = models::allocate(
            &models::member(field.name().expect("named part")),
            &mut members,
        );
        parts.push(part_plan(surface, field, &name, member, multipart, used));
    }
    let extra = match additional {
        AdditionalParts::Forbidden => None,
        AdditionalParts::Allowed(part) => Some(part_plan(
            surface,
            part,
            &name,
            "extra".into(),
            multipart,
            used,
        )),
    };
    surface.objects.push(BodyObject {
        source: rules.schema().id().clone(),
        name: name.clone(),
        parts,
        extra,
        multipart,
        rules: rules.clone(),
    });
    let _ = contract;
    Ok(name)
}
fn part_plan(
    surface: &mut Surface,
    part: &wire::PartPlan,
    parent: &str,
    name: String,
    multipart: bool,
    used: &mut BTreeSet<String>,
) -> Part {
    let stem = format!("{parent}{}", models::pascal(&name));
    let payload = match part.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => {
            let schema = codec.schema().id().clone();
            surface
                .names
                .entry(schema.clone())
                .or_insert_with(|| stem.clone());
            Payload::Schema(schema)
        }
        PartRepresentation::Binary { .. } => Payload::Bytes,
    };
    let headers = header_object(surface, part.headers(), &format!("{stem}Headers"), used);
    let wrapper = multipart.then(|| models::allocate(&format!("{stem}Part"), used));
    Part {
        name,
        payload,
        wrapper,
        headers,
        wire: part.clone(),
    }
}
