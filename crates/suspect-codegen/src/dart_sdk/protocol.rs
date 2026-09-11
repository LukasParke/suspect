//! Target admission and native names over the rich, immutable protocol plan.
use super::{DartConfig, HttpDiagnostic, ModelPlan, Plan, diag, models};
use crate::http_protocol::{self as wire, Capability as C, Representation as R};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedCompiler;

#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub source: SourceId,
    pub wire_name: String,
    pub name: String,
    pub native_type: String,
    pub hook: wire::CredentialHook,
}
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub source: SourceId,
    pub schema: SchemaId,
    pub wire_name: String,
    pub name: String,
    pub required: bool,
    pub native_type: String,
    pub codec_name: String,
    pub wire: wire::ParameterPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub name: String,
    pub native_type: String,
    pub codec_name: String,
    pub schema: SchemaId,
    pub wire: wire::HeaderPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedHeaders {
    pub name: String,
    pub fields: Vec<PlannedHeader>,
}
#[derive(Debug, Clone)]
pub struct PlannedPart {
    pub name: String,
    pub value_type: String,
    pub native_type: String,
    pub codec_name: Option<String>,
    pub wrapper_name: Option<String>,
    pub headers: Option<PlannedHeaders>,
    pub wire: wire::PartPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    pub name: String,
    pub rules: wire::ObjectRules,
    pub fields: Vec<PlannedPart>,
    pub extra: Option<Box<PlannedPart>>,
    pub multipart: bool,
}
// The public typed plan retains inline aggregate descriptors for its capture API.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PlannedPayload {
    Json {
        schema: Option<SchemaId>,
        codec: Option<String>,
    },
    Text {
        schema: Option<SchemaId>,
        codec: Option<String>,
        scalar: wire::ScalarType,
    },
    Bytes {
        max_bytes: u64,
    },
    Aggregate(PlannedAggregate),
    Stream {
        schema: SchemaId,
        codec: String,
        framing: wire::StreamFraming,
        max_item_bytes: u64,
    },
}
impl PlannedPayload {
    pub fn codec_name(&self) -> Option<&str> {
        match self {
            Self::Json { codec, .. } | Self::Text { codec, .. } => codec.as_deref(),
            Self::Stream { codec, .. } => Some(codec),
            _ => None,
        }
    }
    pub fn schema(&self) -> Option<&SchemaId> {
        match self {
            Self::Json { schema, .. } | Self::Text { schema, .. } => schema.as_ref(),
            Self::Stream { schema, .. } => Some(schema),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub wire: wire::MediaPlan,
    pub native_type: String,
    pub variant_name: String,
    pub payload: PlannedPayload,
}
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub required: bool,
    pub native_type: String,
    pub choice_name: Option<String>,
    pub media: Vec<PlannedMedia>,
}
#[derive(Debug, Clone)]
pub struct PlannedStatus {
    pub source: SourceId,
    pub wire: wire::ResponsePlan,
    pub success_name: Option<String>,
    pub error_name: Option<String>,
    pub native_type: String,
    pub choice_name: Option<String>,
    pub none_variant: Option<String>,
    pub bytes_variant: Option<String>,
    pub media: Vec<PlannedMedia>,
    pub headers: Option<PlannedHeaders>,
}
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub success_type: String,
    pub error_type: String,
    pub return_type: String,
    pub stream: bool,
    pub wire: wire::OperationPlan,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
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
    let mut admitted = capabilities(&config);
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
        crate::credential_env::plan(&contract, &protocol, config.credential_env.as_ref())?;
    // Actual wire inputs remain protocol.codec_roots(); the retained closure is
    // the candidate-aware schema catalogue. Resource/dynamic execution is fenced.
    let roots = protocol.codec_schema_closure().to_vec();
    for id in &roots {
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
    let compiler = OwnedCompiler::new(config.schema.clone());
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
        let mut members = BTreeSet::from([
            "body".into(),
            "cancellation".into(),
            "timeout".into(),
            "server".into(),
            "securityAlternative".into(),
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
                    || status.success_name.is_some()
                        && (status.media.len() != 1
                            || !matches!(status.media[0].payload, PlannedPayload::Stream { .. })
                            || status.none_variant.is_some())
                {
                    errors.push(diag(&contract,status.source.clone(),"dart-stream-response-profile","stream operations require an unambiguous item stream for each success and bounded non-stream errors"));
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
        R::Stream { stream } => (
            models
                .native_type(stream.item_codec().schema().id())
                .expect("item root"),
            PlannedPayload::Stream {
                schema: stream.item_codec().schema().id().clone(),
                codec: models
                    .model(stream.item_codec().schema().id())
                    .unwrap()
                    .codec_name
                    .clone(),
                framing: stream.framing(),
                max_item_bytes: stream.max_item_bytes(),
            },
        ),
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
