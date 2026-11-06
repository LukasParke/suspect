//! Target admission and native names over the rich, immutable protocol plan.
use super::{DartConfig, HttpDiagnostic, ModelPlan, Plan, diag, models};
use crate::http_protocol::{self as wire, Capability as C, Representation as R};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedCompiler;

/// One client credential slot, shared by every operation that names the same source scheme.
#[derive(Debug, Clone)]
pub struct PlannedCredential {
    /// Source security-scheme declaration this credential satisfies.
    pub source: SourceId,
    /// Security-scheme name as declared in the source document.
    pub wire_name: String,
    /// Collision-free Dart constructor parameter and member name.
    pub name: String,
    /// Nullable Dart carrier: `BasicCredentials?`, `CredentialProvider?` or `String?`.
    pub native_type: String,
    /// How the credential is attached to outgoing requests.
    pub hook: wire::CredentialHook,
}
/// A caller-supplied path, query, header or cookie parameter of one operation.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Source parameter declaration.
    pub source: SourceId,
    /// Codec root schema of the parameter value.
    pub schema: SchemaId,
    /// Parameter name on the wire.
    pub wire_name: String,
    /// Dart method parameter name, unique within the operation's locals.
    pub name: String,
    /// Whether the source marks the parameter required.
    pub required: bool,
    /// Dart type of the parameter value.
    pub native_type: String,
    /// Generated codec that converts the value to and from JSON.
    pub codec_name: String,
    /// Admitted wire serialization plan.
    pub wire: wire::ParameterPlan,
}
/// One typed header field of a response or part header record.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Dart member name, unique within its header record.
    pub name: String,
    /// Dart type of the header value.
    pub native_type: String,
    /// Generated codec that converts the decoded value.
    pub codec_name: String,
    /// Codec root schema of the header value.
    pub schema: SchemaId,
    /// Admitted header parsing plan.
    pub wire: wire::HeaderPlan,
}
/// A generated Dart class grouping the declared headers of a response or part.
#[derive(Debug, Clone)]
pub struct PlannedHeaders {
    /// Collision-free Dart class name.
    pub name: String,
    /// Declared header fields, in protocol order.
    pub fields: Vec<PlannedHeader>,
}
/// One named field of a form or multipart aggregate, or its additional-fields slot.
#[derive(Debug, Clone)]
pub struct PlannedPart {
    /// Dart member name; `extraFields` for the additional-parts slot.
    pub name: String,
    /// Dart type of a single part value (`Uint8List` for binary parts).
    pub value_type: String,
    /// Member type after wrapping and repetition, e.g. `List<Wrapper>`.
    pub native_type: String,
    /// Value codec; `None` for binary parts.
    pub codec_name: Option<String>,
    /// Wrapper class carrying per-part headers or content type, when one is needed.
    pub wrapper_name: Option<String>,
    /// Declared part headers.
    pub headers: Option<PlannedHeaders>,
    /// Admitted part encoding plan.
    pub wire: wire::PartPlan,
}
/// A generated fields class for a form or named-multipart body.
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    /// Collision-free Dart class name.
    pub name: String,
    /// Object rules (required, bounds) the aggregate enforces.
    pub rules: wire::ObjectRules,
    /// Declared named fields.
    pub fields: Vec<PlannedPart>,
    /// Additional-fields slot, when the body admits undeclared parts.
    pub extra: Option<Box<PlannedPart>>,
    /// `true` for multipart, `false` for URL-encoded forms.
    pub multipart: bool,
}
/// The Dart representation of one request or response media type.
// The public typed plan retains inline aggregate descriptors for its capture API.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PlannedPayload {
    /// JSON body; schema-free when `schema` is `None` (surfaced as `JsonValue`).
    Json {
        /// Codec root schema, if the media declares one.
        schema: Option<SchemaId>,
        /// Generated codec for `schema`.
        codec: Option<String>,
    },
    /// Text body, decoded as `String` unless a schema codec applies.
    Text {
        /// Codec root schema, if the media declares one.
        schema: Option<SchemaId>,
        /// Generated codec for `schema`.
        codec: Option<String>,
        /// Scalar interpretation of the text.
        scalar: wire::ScalarType,
    },
    /// Opaque binary body carried as `Uint8List`.
    Bytes {
        /// Byte ceiling for this body.
        max_bytes: u64,
    },
    /// Form or multipart body bound to a generated fields class.
    Aggregate(PlannedAggregate),
    /// SSE or JSON-lines item stream; items are `JsonValue` when schema-free.
    Stream {
        /// Item codec root schema, if the stream declares one.
        schema: Option<SchemaId>,
        /// Generated codec for `schema`.
        codec: Option<String>,
        /// Item framing on the wire.
        framing: wire::StreamFraming,
        /// Byte ceiling for one framed item.
        max_item_bytes: u64,
    },
}
impl PlannedPayload {
    /// Generated codec of a JSON, text or stream payload, if it is schema-bound.
    pub fn codec_name(&self) -> Option<&str> {
        match self {
            Self::Json { codec, .. } | Self::Text { codec, .. } => codec.as_deref(),
            Self::Stream { codec, .. } => codec.as_deref(),
            _ => None,
        }
    }
    /// Codec root schema of a JSON, text or stream payload, if it declares one.
    pub fn schema(&self) -> Option<&SchemaId> {
        match self {
            Self::Json { schema, .. } | Self::Text { schema, .. } => schema.as_ref(),
            Self::Stream { schema, .. } => schema.as_ref(),
            _ => None,
        }
    }
}
/// One admitted media type of a request body or response status.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Admitted media plan.
    pub wire: wire::MediaPlan,
    /// Dart type of the decoded or encoded payload.
    pub native_type: String,
    /// Variant class name used when the body or status is a choice.
    pub variant_name: String,
    /// Payload representation.
    pub payload: PlannedPayload,
}
/// The request body of one operation.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Source request-body declaration.
    pub source: SourceId,
    /// Whether the source marks the body required.
    pub required: bool,
    /// Dart type of the `body` parameter: the choice class or the single media type.
    pub native_type: String,
    /// Sealed choice class, present for multiple media or a media range.
    pub choice_name: Option<String>,
    /// Admitted media types.
    pub media: Vec<PlannedMedia>,
}
/// One declared response status (exact, range or `default`) of an operation.
#[derive(Debug, Clone)]
pub struct PlannedStatus {
    /// Source response declaration.
    pub source: SourceId,
    /// Admitted response plan.
    pub wire: wire::ResponsePlan,
    /// Success class, when the status can be a 2xx outcome.
    pub success_name: Option<String>,
    /// Exception class, when the status can be a non-2xx outcome.
    pub error_name: Option<String>,
    /// Dart content type: the choice class, a single media type, `NoBody` or `Uint8List`.
    pub native_type: String,
    /// Sealed content choice class, when several media or an optional body apply.
    pub choice_name: Option<String>,
    /// Choice variant for an absent body on range and `default` statuses.
    pub none_variant: Option<String>,
    /// Choice variant for raw bytes when no media is declared.
    pub bytes_variant: Option<String>,
    /// Admitted media types; empty when the status never carries a body.
    pub media: Vec<PlannedMedia>,
    /// Declared response headers.
    pub headers: Option<PlannedHeaders>,
}
/// One selected outgoing operation and its Dart client method.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Source operation declaration.
    pub source: SourceId,
    /// Source `operationId`.
    pub operation_id: String,
    /// Collision-free `Client` method name.
    pub method_name: String,
    /// Sealed success class across all success statuses.
    pub success_type: String,
    /// Sealed exception class across all error statuses.
    pub error_type: String,
    /// `Future<T>` or `Stream<T>`; `T` is the sole success class or `success_type`.
    pub return_type: String,
    /// Whether any status carries a stream payload.
    pub stream: bool,
    /// Admitted operation plan.
    pub wire: wire::OperationPlan,
    /// Caller parameters, in protocol order.
    pub parameters: Vec<PlannedParameter>,
    /// Request body, if declared.
    pub body: Option<PlannedBody>,
    /// Declared response statuses, in protocol order.
    pub statuses: Vec<PlannedStatus>,
}

/// Capabilities with target implementations; positional and request streaming
/// remain explicit refusals, rather than inheriting every planner capability.
pub fn capabilities(config: &DartConfig) -> wire::Capabilities {
    wire::Capabilities::for_adapter(
        "dart-protocol-v1",
        [
            C::AdditionalMethods,
            C::CustomMethods,
            C::HttpServers,
            C::RelativeServers,
            C::DocumentRelativeServers,
            C::MultipleServers,
            C::ServerVariables,
            C::AnonymousSecurity,
            C::SecurityAlternatives,
            C::ConjunctiveSecurity,
            C::HttpBasic,
            C::ApiKeys,
            C::OAuth2,
            C::OpenIdConnect,
            C::SecurityRoles,
            C::ParameterStyles,
            C::HeaderParameters,
            C::CookieParameters,
            C::ReservedParameters,
            C::ContentParameters,
            C::QuerystringParameters,
            C::QuerystringForm,
            C::RangeResponses,
            C::DefaultResponses,
            C::UndeclaredResponses,
            C::MultipleMediaTypes,
            C::MediaRanges,
            C::MediaTypeParameters,
            C::StructuredJsonMedia,
            C::SchemaFreeJson,
            C::TextBodies,
            C::BinaryBodies,
            C::UndeclaredResponseBody,
            C::ResponseHeaders,
            C::ResponseLinks,
            C::FormBodies,
            C::MultipartBodies,
            C::PartEncodings,
            C::ServerSentEvents,
            C::JsonLines,
            C::OpenApi30,
            C::OpenApi32,
            C::SchemaResources,
            C::DynamicSchemaReferences,
        ],
    )
    .with_limits(wire::ByteLimits::new(
        config.max_response_bytes.max(config.max_request_bytes) as u64,
        config.max_part_bytes as u64,
        config.max_stream_item_bytes as u64,
    ))
}

/// Plan a Dart SDK for the selected operations with no compatibility profiles.
///
/// # Errors
///
/// Returns every package, resource-policy, protocol-admission, schema and
/// naming diagnostic found; see [`plan_sdk_with_profiles`].
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: DartConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    plan_sdk_with_profiles(contract, selected, config, &BTreeSet::new())
}

/// Plan with only explicitly requested, versioned compatibility departures.
pub fn plan_sdk_with_profiles(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: DartConfig,
    profiles: &BTreeSet<wire::CompatibilityProfile>,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    let entry = SourceId::new(contract.entry().clone(), Default::default());
    let mut errors = Vec::new();
    if selected.is_empty() {
        errors.push(diag(
            &contract,
            entry.clone(),
            "http-no-operations",
            "select at least one outgoing operation",
        ));
    }
    if config.package.name.is_empty()
        || config.package.name.len() > 64
        || !config.package.name.as_bytes()[0].is_ascii_lowercase()
        || !config
            .package
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || models::keyword(&config.package.name)
    {
        errors.push(diag(
            &contract,
            entry.clone(),
            "dart-package-name",
            "pub names must be lowercase Dart identifiers of at most 64 ASCII characters",
        ));
    }
    if semver::Version::parse(&config.package.version).is_err() {
        errors.push(diag(
            &contract,
            entry.clone(),
            "dart-package-version",
            "pub version must be an exact semantic version",
        ));
    }
    if [
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_capture_bytes,
        config.max_response_header_bytes,
        config.max_conversion_steps,
        config.max_json_steps,
        config.max_part_bytes,
        config.max_stream_item_bytes,
        config.max_stream_buffer_bytes,
    ]
    .iter()
    .any(|v| *v == 0 || *v > 2_147_483_647)
        || config.max_capture_bytes > config.max_response_bytes
        || config.max_stream_buffer_bytes < config.max_stream_item_bytes
        || !(1..=128).contains(&config.max_json_depth)
        || !(1..=128).contains(&config.max_conversion_depth)
        || !(1..=128).contains(&config.schema.max_depth)
        || config.schema.max_number_bytes > 65536
        || config.schema.max_errors == 0
        || config.schema.max_errors > 10000
        || config.schema.max_evaluation_steps > 100_000_000
        || config.schema.max_equality_steps > 100_000_000
    {
        errors.push(diag(&contract,entry.clone(),"dart-resource-policy","Dart requires finite positive 31-bit budgets, capture <= response, stream buffer >= item, depth <=128 and bounded schema work"));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut admitted = crate::schema_view::oas30_capability_for(&contract, capabilities(&config));
    for profile in profiles {
        admitted = admitted.with_profile(*profile);
    }
    let protocol = wire::plan(&contract, selected, admitted)
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| HttpDiagnostic {
                    source: e.source().source().clone(),
                    at: e.source().span(),
                    code: e.code(),
                    message: e.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
    let credential_env =
        crate::credential_env::plan_with_defaults(&contract, &protocol, config.credential_env.as_ref(), config.sdk_defaults.as_ref())?;
    let pagination = if config.sdk_defaults.is_some() {
        Some(wire::plan_pagination(&contract, &protocol, config.sdk_defaults.as_ref())?)
    } else {
        None
    };
    let oauth = if config.sdk_defaults.is_some() {
        Some(wire::plan_oauth(&contract, &protocol, config.sdk_defaults.as_ref())?)
    } else {
        None
    };
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Emission stays conditional on
    // the discriminated SSE subset, so every other document stays byte-identical.
    let stream_semantics = wire::plan_stream_semantics(&contract, &protocol);
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming = wire::plan_incoming(&contract)?;
    // Incoming receipts extend the compiled schema catalogue only when
    // declared: their closure joins the protocol's so receipt bodies compile
    // into the same model plan. Actual wire inputs remain
    // protocol.codec_roots(); the retained closure is the candidate-aware
    // schema catalogue. Resource/dynamic execution is fenced.
    let mut roots = protocol.codec_schema_closure().to_vec();
    if !incoming.is_empty() {
        roots.extend(incoming.codec_schema_closure().iter().cloned());
        roots.sort();
        roots.dedup();
    }
    for id in &roots {
        // OAS 3.0 Reference Object siblings (including readOnly/writeOnly)
        // have no semantic effect, so they cannot activate a directional view.
        if contract
            .schema(id)
            .is_some_and(crate::schema_view::reference_only)
        {
            continue;
        }
        for key in ["readOnly", "writeOnly"] {
            if contract
                .source(id)
                .and_then(|v| v.get(key))
                .is_some_and(|v| v != &serde_json::Value::Bool(false))
            {
                errors.push(diag(
                    &contract,
                    id.child(key),
                    "http-directional-codec-unsupported",
                    "Dart's neutral codecs require directional annotations to be absent or false",
                ));
            }
        }
    }
    for op in protocol.operations() {
        if !stable_path(op.path()) {
            errors.push(diag(&contract,op.source().use_site().source().clone(),"dart-path-unsupported","Dart paths require valid escapes and no dot segments, backslashes, controls, query or fragment delimiters"));
        }
        if op.operation_id().is_none() {
            errors.push(diag(
                &contract,
                op.source().use_site().source().clone(),
                "dart-operation-id",
                "Dart requires a source operationId",
            ));
        }
        for p in op.parameters() {
            if p.location() == wire::ParameterLocation::Header && controlled_header(p.name()) {
                errors.push(diag(
                    &contract,
                    p.source().use_site().source().clone(),
                    "dart-controlled-header",
                    "this transport-owned framing header cannot be a caller parameter",
                ));
            }
        }
        if let Some(body) = op.body() {
            for media in body.media() {
                if matches!(media.representation(), R::Stream { .. }) {
                    errors.push(diag(
                        &contract,
                        media.source().use_site().source().clone(),
                        "dart-request-stream-unsupported",
                        "request item streams need a separately verified upload adapter",
                    ));
                }
            }
        }
        for response in op.responses() {
            for media in response.media() {
                if matches!(media.representation(), R::Form { .. } | R::Multipart { .. }) {
                    errors.push(diag(&contract,media.source().use_site().source().clone(),"dart-response-parts-unsupported","form/multipart response decoding needs a separately verified native profile"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Installed floor/current VM, JavaScript and browser witnesses cover the
    // scoped executor and native carriers. Unused v2 forms keep v1 programs.
    let mut schema_config = config.schema.clone();
    schema_config.oas30_nullable_in_31 =
        profiles.contains(&wire::CompatibilityProfile::Oas30NullableIn31V1);
    let compiler = OwnedCompiler::new(schema_config);
    let compiled = compiler
        .compile_v2(contract.clone(), &roots)
        // Keep ordinary closures' established v1/v2 representation. The checked
        // v3 compiler adds resource semantics only after static admission fails.
        .or_else(|_| compiler.compile_v3(contract.clone(), &roots))
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| HttpDiagnostic {
                    source: e.source,
                    at: e.span.unwrap_or(0..0),
                    code: "dart-schema-compilation",
                    message: format!("{:?}: {}", e.kind, e.message),
                })
                .collect::<Vec<_>>()
        })?;
    let program = compiled.program();
    program.check().map_err(|e| {
        vec![diag(
            &contract,
            entry,
            "dart-validation-program",
            e.to_string(),
        )]
    })?;
    super::validation::runtime(&program).map_err(|e| {
        vec![diag(
            &contract,
            e.source.as_ref().map_or_else(
                || SourceId::new(contract.entry().clone(), Default::default()),
                |s| {
                    s.pointer.split('/').skip(1).fold(
                        SourceId::new(
                            suspect_source::Uri::parse(&s.document).expect("checked source URI"),
                            Default::default(),
                        ),
                        |id, part| id.child(&part.replace("~1", "/").replace("~0", "~")),
                    )
                },
            ),
            "dart-validation-program",
            e.to_string(),
        )]
    })?;
    let model_plan = models::plan(&contract, &compiled, &program)?;
    let mut names = model_plan.used_names();
    // Emission-ready incoming receipt helpers, planned beside the operations so
    // receipt codecs bind the same compiled models and every emitted name is
    // collision-free. Receipts the v1 helpers cannot decode produce
    // source-linked sdk-incoming-* errors instead of silent skips.
    let incoming_receipts = super::incoming::prepare(&contract, &incoming, &model_plan, &mut names)?;
    let mut methods = BTreeSet::from([
        "close".into(),
        "runtimeType".into(),
        "hashCode".into(),
        "toString".into(),
        "noSuchMethod".into(),
    ]);
    let mut credential_names = methods.clone();
    let mut credential_map = BTreeMap::<SourceId, PlannedCredential>::new();
    for op in protocol.operations() {
        for alternative in op.security().alternatives() {
            for requirement in alternative.requirements() {
                let source = requirement.scheme().use_site().source().clone();
                credential_map
                    .entry(source.clone())
                    .or_insert_with(|| PlannedCredential {
                        source,
                        wire_name: requirement.name().into(),
                        name: models::allocate(
                            &models::member(requirement.name()),
                            &mut credential_names,
                        ),
                        native_type: match requirement.credential() {
                            wire::CredentialHook::Basic => "BasicCredentials?",
                            wire::CredentialHook::OAuth2 { .. }
                            | wire::CredentialHook::OpenIdConnect { .. } => "CredentialProvider?",
                            _ => "String?",
                        }
                        .into(),
                        hook: requirement.credential().clone(),
                    });
            }
        }
    }
    let mut operations = Vec::new();
    for op in protocol.operations() {
        let id = op.operation_id().expect("admitted operation id").value();
        let stem = models::exported(id);
        // The fixed call-option parameters plus every local the emitted
        // method body declares (`http_emit::client`): Dart scopes a local over
        // its whole block, so a parameter sharing a local's name is either a
        // `referenced_before_declaration` error or silently shadowed.
        let mut members = BTreeSet::from([
            "body".into(),
            "cancellation".into(),
            "timeout".into(),
            "server".into(),
            "securityAlternative".into(),
            "prepare".into(),
            "inputs".into(),
            "selected".into(),
            "response".into(),
        ]);
        let parameters = op
            .parameters()
            .iter()
            .map(|p| PlannedParameter {
                source: p.source().use_site().source().clone(),
                schema: p.codec().schema().id().clone(),
                wire_name: p.name().into(),
                name: models::allocate(&models::member(p.name()), &mut members),
                required: p.required(),
                native_type: model_plan
                    .native_type(p.codec().schema().id())
                    .expect("parameter codec root"),
                codec_name: model_plan
                    .model(p.codec().schema().id())
                    .expect("parameter codec root")
                    .codec_name
                    .clone(),
                wire: p.clone(),
            })
            .collect();
        let body = op
            .body()
            .map(|body| -> Result<PlannedBody, Vec<HttpDiagnostic>> {
                let media = body
                    .media()
                    .iter()
                    .map(|m| plan_media(&model_plan, m, &format!("{stem}Body"), &mut names))
                    .collect::<Result<Vec<_>, _>>()?;
                let choice = media.len() != 1
                    || media.iter().any(|m| {
                        !matches!(
                            m.wire.media_type().range(),
                            wire::MediaRange::Concrete { .. }
                        )
                    });
                let choice_name =
                    choice.then(|| models::allocate(&format!("{stem}RequestBody"), &mut names));
                Ok(PlannedBody {
                    source: body.source().use_site().source().clone(),
                    required: body.required(),
                    native_type: choice_name
                        .clone()
                        .unwrap_or_else(|| media[0].native_type.clone()),
                    choice_name,
                    media,
                })
            })
            .transpose()?;
        let mut statuses = Vec::new();
        for response in op.responses() {
            let key = response.status_key();
            let suffix = if key == "default" {
                "Default".into()
            } else {
                key.to_owned()
            };
            let can_success = matches!(
                response.status(),
                wire::ResponseStatus::Exact(200..=299)
                    | wire::ResponseStatus::Range(2)
                    | wire::ResponseStatus::Default
            );
            let can_error = !matches!(
                response.status(),
                wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
            );
            let always_none = op.method() == wire::Method::Head
                || matches!(
                    response.status(),
                    wire::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                        | wire::ResponseStatus::Range(1)
                );
            let sometimes_none = !always_none
                && matches!(
                    response.status(),
                    wire::ResponseStatus::Range(2 | 3) | wire::ResponseStatus::Default
                );
            let media = if always_none {
                Vec::new()
            } else {
                response
                    .media()
                    .iter()
                    .map(|m| {
                        plan_media(&model_plan, m, &format!("{stem}Status{suffix}"), &mut names)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            let choice = !always_none && (media.len() > 1 || sometimes_none);
            let choice_name = choice
                .then(|| models::allocate(&format!("{stem}Status{suffix}Content"), &mut names));
            let none_variant = (choice && sometimes_none)
                .then(|| models::allocate(&format!("{stem}Status{suffix}NoBody"), &mut names));
            let bytes_variant = (choice && response.media().is_empty())
                .then(|| models::allocate(&format!("{stem}Status{suffix}Bytes"), &mut names));
            let native_type = choice_name.clone().unwrap_or_else(|| {
                if always_none {
                    "NoBody".into()
                } else if media.is_empty() {
                    "Uint8List".into()
                } else {
                    media[0].native_type.clone()
                }
            });
            statuses.push(PlannedStatus {
                source: response.source().use_site().source().clone(),
                wire: response.clone(),
                native_type,
                choice_name,
                none_variant,
                bytes_variant,
                success_name: can_success
                    .then(|| models::allocate(&format!("{stem}Status{suffix}"), &mut names)),
                error_name: can_error.then(|| {
                    models::allocate(
                        &format!(
                            "{stem}Status{suffix}{}",
                            if can_success { "ApiException" } else { "" }
                        ),
                        &mut names,
                    )
                }),
                headers: plan_headers(
                    &model_plan,
                    response.headers(),
                    &format!("{stem}Status{suffix}Headers"),
                    &mut names,
                ),
                media,
            });
        }
        let stream = statuses.iter().any(|s| {
            s.media
                .iter()
                .any(|m| matches!(m.payload, PlannedPayload::Stream { .. }))
        });
        if stream {
            for status in &statuses {
                if status.error_name.is_some()
                    && status
                        .media
                        .iter()
                        .any(|m| matches!(m.payload, PlannedPayload::Stream { .. }))
                    || status.none_variant.is_some()
                {
                    errors.push(diag(&contract,status.source.clone(),"dart-stream-response-profile","stream operations require bounded non-stream errors"));
                }
            }
        }
        let success_type = models::allocate(&format!("{stem}Success"), &mut names);
        let error_type = models::allocate(&format!("{stem}ApiException"), &mut names);
        let successes = statuses
            .iter()
            .filter_map(|s| s.success_name.as_ref())
            .collect::<Vec<_>>();
        let result = if let [one] = successes.as_slice() {
            (*one).clone()
        } else {
            success_type.clone()
        };
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id: id.clone(),
            method_name: models::allocate(&models::member(id), &mut methods),
            success_type,
            error_type,
            return_type: format!("{}<{result}>", if stream { "Stream" } else { "Future" }),
            stream,
            wire: op.clone(),
            parameters,
            body,
            statuses,
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let examples = if program.version == suspect_schema::OwnedProgram::V3_VERSION {
        crate::examples::plan_protocol_examples_v3(contract.clone(), &protocol, Default::default())
    } else if program.version == suspect_schema::OwnedProgram::V2_VERSION {
        crate::examples::plan_protocol_examples_v2(contract.clone(), &protocol, Default::default())
    } else {
        crate::examples::plan_protocol_examples(contract.clone(), &protocol, Default::default())
    };
    Ok(Plan {
        contract,
        config,
        protocol,
        program,
        compiled,
        models: model_plan,
        operations,
        credentials: credential_map.into_values().collect(),
        credential_env,
        pagination,
        oauth,
        incoming,
        incoming_receipts,
        stream_semantics,
        examples,
    })
}

fn controlled_header(name: &str) -> bool {
    [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "accept-encoding",
    ]
    .iter()
    .any(|v| name.eq_ignore_ascii_case(v))
}
fn stable_path(path: &str) -> bool {
    let mut out = String::new();
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return false;
                }
                out.push('x');
            }
            '%' => {
                let Some(a) = chars.next().and_then(|v| v.to_digit(16)) else {
                    return false;
                };
                let Some(b) = chars.next().and_then(|v| v.to_digit(16)) else {
                    return false;
                };
                let byte = (a * 16 + b) as u8;
                if byte < 32 || byte == 127 || byte == b'\\' {
                    return false;
                }
                out.push(char::from(byte));
            }
            '}' | '?' | '#' | '\\' => return false,
            c if c.is_control() || c.is_whitespace() => return false,
            c => out.push(c),
        }
    }
    !out.split('/').any(|part| matches!(part, "." | ".."))
}
fn plan_headers(
    models: &ModelPlan,
    headers: &[wire::HeaderPlan],
    stem: &str,
    names: &mut BTreeSet<String>,
) -> Option<PlannedHeaders> {
    if headers.is_empty() {
        return None;
    }
    let mut used = BTreeSet::from([
        "runtimeType".into(),
        "hashCode".into(),
        "toString".into(),
        "noSuchMethod".into(),
    ]);
    Some(PlannedHeaders {
        name: models::allocate(stem, names),
        fields: headers
            .iter()
            .map(|h| PlannedHeader {
                name: models::allocate(&models::member(h.name()), &mut used),
                schema: h.codec().schema().id().clone(),
                native_type: models
                    .native_type(h.codec().schema().id())
                    .expect("header codec root"),
                codec_name: models
                    .model(h.codec().schema().id())
                    .expect("header codec root")
                    .codec_name
                    .clone(),
                wire: h.clone(),
            })
            .collect(),
    })
}
fn plan_part(
    models: &ModelPlan,
    wire: &wire::PartPlan,
    stem: &str,
    names: &mut BTreeSet<String>,
    members: &mut BTreeSet<String>,
) -> PlannedPart {
    let name = models::allocate(
        &models::member(wire.name().unwrap_or("extraFields")),
        members,
    );
    let (value_type, codec_name) = match wire.representation() {
        wire::PartRepresentation::Binary { .. } => ("Uint8List".into(), None),
        wire::PartRepresentation::Json { codec, .. }
        | wire::PartRepresentation::Text { codec, .. }
        | wire::PartRepresentation::Style { codec, .. } => (
            models
                .native_type(codec.schema().id())
                .expect("part codec root"),
            Some(
                models
                    .model(codec.schema().id())
                    .expect("part codec")
                    .codec_name
                    .clone(),
            ),
        ),
    };
    let headers = plan_headers(
        models,
        wire.headers(),
        &format!("{stem}{}Headers", models::exported(&name)),
        names,
    );
    let wrapped = headers.is_some()
        || wire.content_types().len() > 1
        || wire
            .content_types()
            .iter()
            .any(|m| !matches!(m.range(), wire::MediaRange::Concrete { .. }));
    let wrapper_name =
        wrapped.then(|| models::allocate(&format!("{stem}{}Part", models::exported(&name)), names));
    let element = wrapper_name.clone().unwrap_or_else(|| value_type.clone());
    let native_type = if wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
        format!("List<{element}>")
    } else {
        element
    };
    PlannedPart {
        name,
        value_type,
        native_type,
        codec_name,
        wrapper_name,
        headers,
        wire: wire.clone(),
    }
}
fn plan_media(
    models: &ModelPlan,
    media: &wire::MediaPlan,
    stem: &str,
    names: &mut BTreeSet<String>,
) -> Result<PlannedMedia, Vec<HttpDiagnostic>> {
    let role = match media.representation() {
        R::Json { .. } => "Json",
        R::Text { .. } => "Text",
        R::Binary { .. } => "Bytes",
        R::Form { .. } => "Form",
        R::Multipart { .. } => "Multipart",
        R::Stream { .. } => "Stream",
    };
    let variant_name = models::allocate(&format!("{stem}{role}"), names);
    let (native_type, payload) = match media.representation() {
        R::Json { codec } => (
            codec.as_ref().map_or_else(
                || "JsonValue".into(),
                |c| models.native_type(c.schema().id()).expect("JSON root"),
            ),
            PlannedPayload::Json {
                schema: codec.as_ref().map(|c| c.schema().id().clone()),
                codec: codec
                    .as_ref()
                    .map(|c| models.model(c.schema().id()).unwrap().codec_name.clone()),
            },
        ),
        R::Text { codec, scalar, .. } => (
            codec.as_ref().map_or_else(
                || "String".into(),
                |c| models.native_type(c.schema().id()).expect("text root"),
            ),
            PlannedPayload::Text {
                schema: codec.as_ref().map(|c| c.schema().id().clone()),
                codec: codec
                    .as_ref()
                    .map(|c| models.model(c.schema().id()).unwrap().codec_name.clone()),
                scalar: *scalar,
            },
        ),
        R::Binary { bytes, .. } => (
            "Uint8List".into(),
            PlannedPayload::Bytes {
                max_bytes: bytes.max_bytes(),
            },
        ),
        R::Stream { stream } => match stream.item_codec() {
            Some(codec) => (
                models.native_type(codec.schema().id()).expect("item root"),
                PlannedPayload::Stream {
                    schema: Some(codec.schema().id().clone()),
                    codec: Some(
                        models
                            .model(codec.schema().id())
                            .unwrap()
                            .codec_name
                            .clone(),
                    ),
                    framing: stream.framing(),
                    max_item_bytes: stream.max_item_bytes(),
                },
            ),
            // A schemaless stream surfaces untyped parsed envelope values.
            None => (
                "JsonValue".into(),
                PlannedPayload::Stream {
                    schema: None,
                    codec: None,
                    framing: stream.framing(),
                    max_item_bytes: stream.max_item_bytes(),
                },
            ),
        },
        R::Form { form } => {
            let name = models::allocate(&format!("{stem}Fields"), names);
            let mut members = BTreeSet::from(["extraFields".into()]);
            let fields = form
                .fields()
                .iter()
                .map(|p| plan_part(models, p, &name, names, &mut members))
                .collect();
            let extra = match form.additional() {
                wire::AdditionalParts::Forbidden => None,
                wire::AdditionalParts::Allowed(p) => {
                    Some(Box::new(plan_part(models, p, &name, names, &mut members)))
                }
            };
            (
                name.clone(),
                PlannedPayload::Aggregate(PlannedAggregate {
                    name,
                    rules: form.rules().clone(),
                    fields,
                    extra,
                    multipart: false,
                }),
            )
        }
        R::Multipart {
            multipart:
                wire::MultipartPlan::Named {
                    rules,
                    parts,
                    additional,
                },
        } => {
            let name = models::allocate(&format!("{stem}Fields"), names);
            let mut members = BTreeSet::from(["extraFields".into()]);
            let fields = parts
                .iter()
                .map(|p| plan_part(models, p, &name, names, &mut members))
                .collect();
            let extra = match additional {
                wire::AdditionalParts::Forbidden => None,
                wire::AdditionalParts::Allowed(p) => {
                    Some(Box::new(plan_part(models, p, &name, names, &mut members)))
                }
            };
            (
                name.clone(),
                PlannedPayload::Aggregate(PlannedAggregate {
                    name,
                    rules: rules.clone(),
                    fields,
                    extra,
                    multipart: true,
                }),
            )
        }
        R::Multipart {
            multipart: wire::MultipartPlan::Positional { .. },
        } => unreachable!("capability not admitted"),
    };
    Ok(PlannedMedia {
        wire: media.clone(),
        native_type,
        variant_name,
        payload,
    })
}
