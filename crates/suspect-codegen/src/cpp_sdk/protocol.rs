//! Native C++ bindings over the admitted shared protocol, including non-JSON
//! values. Only real codec inputs participate in the model/OwnedProgram graph.
use super::{HttpDiagnostic, SdkConfig, SdkPlan, diagnostic, models};
use crate::{examples, http_protocol as wire};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedCompiler;

#[derive(Debug, Clone)]
pub enum ValueKind {
    Model(SchemaId),
    Json,
    Scalar(wire::ScalarType),
    Bytes,
    Unit,
    Aggregate(String),
    Choice(String),
    Stream {
        schema: SchemaId,
        framing: wire::StreamFraming,
        max_item_bytes: usize,
    },
}
#[derive(Debug, Clone)]
pub struct ValueType {
    pub cpp_type: String,
    pub kind: ValueKind,
}
impl ValueType {
    pub fn schema(&self) -> Option<&SchemaId> {
        match &self.kind {
            ValueKind::Model(id) | ValueKind::Stream { schema: id, .. } => Some(id),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub source: SourceId,
    pub field_name: String,
    pub value: ValueType,
    pub wire: wire::HeaderPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedPart {
    pub source: SourceId,
    pub field_name: String,
    pub name: Option<String>,
    pub value: ValueType,
    pub part_type: Option<String>,
    pub headers_type: Option<String>,
    pub headers: Vec<PlannedHeader>,
    pub wire: wire::PartPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    pub source: SourceId,
    pub type_name: String,
    pub multipart: bool,
    pub fields: Vec<PlannedPart>,
    pub additional: Option<PlannedPart>,
    pub rules: wire::ObjectRules,
}
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub source: SourceId,
    pub value: ValueType,
    pub wrapper_type: String,
    pub requires_content_type: bool,
    pub wire: wire::MediaPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub required: bool,
    pub cpp_type: String,
    pub value: ValueType,
    pub media: Vec<PlannedMedia>,
    pub choice_type: Option<String>,
}
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub source: SourceId,
    pub field_name: String,
    pub wire_name: String,
    pub required: bool,
    pub cpp_type: String,
    pub value: ValueType,
    pub wire: wire::ParameterPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedResponseCase {
    pub source: SourceId,
    pub variant_type: String,
    pub value: ValueType,
    pub media: Option<wire::MediaPlan>,
    pub forbidden: bool,
}
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub source: SourceId,
    pub status: wire::ResponseStatus,
    pub status_key: String,
    pub headers_type: Option<String>,
    pub headers: Vec<PlannedHeader>,
    pub cases: Vec<PlannedResponseCase>,
    pub wire: wire::ResponsePlan,
}
impl PlannedResponse {
    pub fn can_succeed(&self) -> bool {
        matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299)
                | wire::ResponseStatus::Range(2)
                | wire::ResponseStatus::Default
        )
    }
    pub fn can_fail(&self) -> bool {
        !matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        )
    }
}
#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub source: SourceId,
    pub field_name: String,
    pub cpp_type: String,
    pub wire: wire::CredentialRequirement,
}
#[derive(Debug, Clone)]
pub struct InputArgument {
    pub name: String,
    pub member_name: String,
    pub value: ValueType,
    pub cpp_type: String,
}
#[derive(Debug, Clone)]
pub struct InputConstructor {
    pub name: String,
    pub parameters: Vec<InputArgument>,
}
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub declared_operation_id: Option<String>,
    pub method_name: String,
    pub input_type: String,
    pub success_type: String,
    pub error_type: String,
    pub constructor: InputConstructor,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
    pub wire: wire::OperationPlan,
}

/// Each enabled capability has a native witness in cpp_protocol.rs. Positional
/// and streamed multipart remain distinct profiles and are not opted in here.
pub fn capabilities(config: &SdkConfig) -> wire::Capabilities {
    use wire::Capability::*;
    let result = wire::Capabilities::for_adapter(
        "cpp20-libcurl-protocol-v2",
        [
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
            OpenApi30,
            OpenApi32,
            SchemaResources,
            DynamicSchemaReferences,
        ],
    )
    .with_limits(wire::ByteLimits::new(
        config.max_request_bytes.max(config.max_response_bytes) as u64,
        config.max_part_bytes as u64,
        config.max_stream_item_bytes as u64,
    ));
    if config.legacy_binary_strings {
        result.with_profile(wire::CompatibilityProfile::LegacyBinaryStringV1)
    } else {
        result
    }
}

pub(super) fn plan(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_profile(contract, selected, config, true)
}
pub(super) fn plan_profile(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    scoped: bool,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let capabilities = capabilities(&config);
    plan_capabilities(contract, selected, config, scoped, capabilities)
}
pub(super) fn plan_capabilities(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    scoped: bool,
    capabilities: wire::Capabilities,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    super::validate_config(&contract, selected, &config)?;
    let resources = capabilities.supports(wire::Capability::SchemaResources)
        && capabilities.supports(wire::Capability::DynamicSchemaReferences);
    let protocol = wire::plan(&contract, selected, capabilities)
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
    validate_native_boundaries(&contract, &protocol)?;
    // This is the candidate-aware schema planning closure, not additional HTTP
    // body inputs. Actual wire codecs remain protocol.codec_roots(). Resource
    // and dynamic execution stay behind their separate capability fences.
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming = wire::plan_incoming(&contract)?;
    // Incoming receipts extend the codec table only when declared: their
    // candidate-aware schema closure joins the selected operations', so their
    // body schemas become actual codec inputs with the same program roots.
    let mut roots = protocol.codec_schema_closure().to_vec();
    if !incoming.is_empty() {
        roots.extend(incoming.codec_schema_closure().iter().cloned());
        roots.sort();
        roots.dedup();
    }
    for id in &roots {
        if let Some(schema) = contract.schema(id) {
            let raw = crate::schema_view::raw(schema);
            for key in ["readOnly", "writeOnly"] {
                if raw
                    .get(key)
                    .is_some_and(|v| v != &serde_json::Value::Bool(false))
                {
                    return Err(vec![diagnostic(
                        &contract,
                        id.child(key),
                        "cpp-directional-unsupported",
                        "this C++ codec view requires directional annotations to be absent or false",
                    )]);
                }
            }
        }
    }
    let compiler = OwnedCompiler::new(config.validation.clone());
    let needs_resources = roots.iter().any(|id| {
        contract.schema(id).is_some_and(|schema| {
            !schema.ignores_ref_siblings()
                && (contract
                    .resource_scope(id)
                    .is_some_and(|scope| scope.base_source().is_some())
                    || ["$id", "$anchor", "$dynamicAnchor", "$dynamicRef"]
                        .iter()
                        .any(|key| schema.raw().get(*key).is_some()))
        })
    });
    let validation = (if resources && needs_resources {
        compiler.compile_v3(contract.clone(), &roots)
    } else if scoped {
        compiler.compile_v2(contract.clone(), &roots)
    } else {
        compiler.compile(contract.clone(), &roots)
    })
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "cpp-schema-compilation",
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = validation.program();
    super::check_program_resources(&contract, &program, scoped, resources)?;
    let mut seeds = BTreeMap::new();
    let mut reserved = BTreeSet::new();
    for op in protocol.operations() {
        let base = operation_base(op);
        for suffix in ["Input", "Success", "Error"] {
            reserved.insert(format!("{base}{suffix}"));
        }
        if let Some(body) = op.body() {
            for media in body.media() {
                seed_media(
                    media,
                    &format!(
                        "{base}Body{}",
                        if body.media().len() == 1 {
                            String::new()
                        } else {
                            media_suffix(media)
                        }
                    ),
                    &mut seeds,
                );
            }
        }
        for response in op.responses() {
            let stem = format!("{base}Status{}", models::pascal(response.status_key()));
            // Exact numeric status names stay Status200 (not StatusN200).
            let stem = if matches!(response.status(), wire::ResponseStatus::Exact(_)) {
                format!("{base}Status{}", response.status_key())
            } else {
                stem
            };
            reserved.insert(stem.clone());
            for media in response.media() {
                seed_media(
                    media,
                    &format!(
                        "{stem}Body{}",
                        if response.media().len() == 1 {
                            String::new()
                        } else {
                            media_suffix(media)
                        }
                    ),
                    &mut seeds,
                );
            }
            for header in response.headers() {
                seeds.insert(
                    header.codec().schema().id().clone(),
                    format!("{stem}Header{}", models::pascal(header.name())),
                );
            }
        }
        for parameter in op.parameters() {
            let base = format!("{base}{}", models::pascal(parameter.name()));
            if let Some(media) = parameter.content_media() {
                seed_media(media, &base, &mut seeds);
            }
            if protocol
                .codec_roots()
                .contains(parameter.codec().schema().id())
            {
                seeds.insert(parameter.codec().schema().id().clone(), base);
            }
        }
    }
    let models = models::plan(
        &contract,
        &roots,
        &seeds,
        &reserved,
        &program,
        &config.namespace,
    )?;
    let mut names = models::reserved_names();
    for symbol in models.symbols() {
        names.extend([
            symbol.name.clone(),
            symbol.definition.clone(),
            symbol.codec_name.clone(),
        ]);
    }
    let mut state = State {
        models: &models,
        namespace: &config.namespace,
        names,
        aggregates: Vec::new(),
    };
    let mut methods = BTreeSet::from([
        "with_curl".into(),
        "transport_".into(),
        "credentials_".into(),
        "options_".into(),
    ]);
    if credential_env.is_some() {
        methods.extend(["from_env".into(), "from_env_with_transport".into()]);
    }
    let mut credential_names = BTreeSet::new();
    let mut credentials = BTreeMap::new();
    let mut operations = Vec::new();
    for op in protocol.operations() {
        let base = operation_base(op);
        let mut stem = base.clone();
        let mut suffix = 2;
        loop {
            let group = [
                format!("{stem}Input"),
                format!("{stem}Success"),
                format!("{stem}Error"),
            ];
            if group.iter().all(|n| !state.names.contains(n)) {
                state.names.extend(group);
                break;
            }
            stem = format!("{base}{suffix}");
            suffix += 1;
        }
        let source = op.source().use_site().source().clone();
        if !super::valid_path(op.path()) {
            return Err(vec![diagnostic(
                &contract,
                source,
                "cpp-path-literal-unsupported",
                "paths require balanced placeholders, valid percent escapes and no ambiguous dot segments",
            )]);
        }
        for alternative in op.security().alternatives() {
            for requirement in alternative.requirements() {
                let source = if credential_env.is_some() {
                    requirement.scheme().use_site().source()
                } else {
                    requirement.scheme().terminal().source()
                }
                .clone();
                credentials
                    .entry(source.clone())
                    .or_insert_with(|| PlannedCredential {
                        source,
                        field_name: models::allocate(
                            &models::snake(requirement.name()),
                            &mut credential_names,
                        ),
                        cpp_type: match requirement.credential() {
                            wire::CredentialHook::Basic => "BasicCredentials",
                            wire::CredentialHook::OAuth2 { .. }
                            | wire::CredentialHook::OpenIdConnect { .. } => "CredentialProvider",
                            _ => "std::string",
                        }
                        .into(),
                        wire: requirement.clone(),
                    });
            }
        }
        let mut members = BTreeSet::from(["body".into()]);
        let mut parameters = Vec::new();
        for parameter in op.parameters() {
            let value = if let Some(media) = parameter
                .content_media()
                .filter(|m| matches!(m.representation(), wire::Representation::Form { .. }))
            {
                state.media_value(
                    media,
                    &format!("{stem}{}", models::pascal(parameter.name())),
                    false,
                )?
            } else {
                state.model(parameter.codec().schema().id())
            };
            parameters.push(PlannedParameter {
                source: parameter.source().use_site().source().clone(),
                field_name: models::allocate(&models::snake(parameter.name()), &mut members),
                wire_name: parameter.name().into(),
                required: parameter.required(),
                cpp_type: present(&value.cpp_type, parameter.required()),
                value,
                wire: parameter.clone(),
            });
        }
        let body = if let Some(body) = op.body() {
            let choice = body.media().len() > 1
                || body
                    .media()
                    .iter()
                    .any(|m| !matches!(m.media_type().range(), wire::MediaRange::Concrete { .. }));
            let choice_type =
                choice.then(|| models::allocate(&format!("{stem}Body"), &mut state.names));
            let mut media = Vec::new();
            for m in body.media() {
                let name = format!(
                    "{stem}Body{}",
                    if choice {
                        media_suffix(m)
                    } else {
                        String::new()
                    }
                );
                let value = state.media_value(m, &name, true)?;
                media.push(PlannedMedia {
                    source: m.source().use_site().source().clone(),
                    wrapper_type: models::allocate(&format!("{name}Content"), &mut state.names),
                    requires_content_type: !matches!(
                        m.media_type().range(),
                        wire::MediaRange::Concrete { .. }
                    ),
                    value,
                    wire: m.clone(),
                });
            }
            let value = if let Some(name) = &choice_type {
                ValueType {
                    cpp_type: state.qualified(name),
                    kind: ValueKind::Choice(name.clone()),
                }
            } else {
                media[0].value.clone()
            };
            Some(PlannedBody {
                source: body.source().use_site().source().clone(),
                required: body.required(),
                cpp_type: present(&value.cpp_type, body.required()),
                value,
                media,
                choice_type,
            })
        } else {
            None
        };
        let mut args = parameters
            .iter()
            .filter(|p| p.required)
            .map(|p| (p.field_name.clone(), p.value.clone()))
            .collect::<Vec<_>>();
        if let Some(body) = &body
            && body.required
        {
            args.push(("body".into(), body.value.clone()));
        }
        let constructor = InputConstructor {
            name: format!("{stem}Input"),
            parameters: args
                .into_iter()
                .enumerate()
                .map(|(i, (member_name, value))| InputArgument {
                    name: format!("arg{i}"),
                    member_name,
                    cpp_type: value.cpp_type.clone(),
                    value,
                })
                .collect(),
        };
        let mut responses = Vec::new();
        for response in op.responses() {
            let status_name = match response.status() {
                wire::ResponseStatus::Exact(status) => status.to_string(),
                wire::ResponseStatus::Range(class) => format!("{class}XX"),
                wire::ResponseStatus::Default => "Default".into(),
            };
            let base = format!("{stem}Status{status_name}");
            let (headers_type, headers) =
                state.headers(response.headers(), &format!("{base}Headers"));
            let forbidden = op.method() == wire::Method::Head
                || matches!(
                    response.status(),
                    wire::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                        | wire::ResponseStatus::Range(1)
                );
            let conditional_none = !forbidden
                && matches!(
                    response.status(),
                    wire::ResponseStatus::Range(2 | 3) | wire::ResponseStatus::Default
                );
            let mut cases = Vec::new();
            if forbidden || response.media().is_empty() {
                cases.push(PlannedResponseCase {
                    source: response.source().use_site().source().clone(),
                    variant_type: models::allocate(&base, &mut state.names),
                    value: ValueType {
                        cpp_type: if forbidden { "Unit" } else { "Bytes" }.into(),
                        kind: if forbidden {
                            ValueKind::Unit
                        } else {
                            ValueKind::Bytes
                        },
                    },
                    media: None,
                    forbidden,
                });
            } else {
                for media in response.media() {
                    let name = if response.media().len() == 1 {
                        base.clone()
                    } else {
                        format!("{base}{}", media_suffix(media))
                    };
                    cases.push(PlannedResponseCase {
                        source: media.source().use_site().source().clone(),
                        variant_type: models::allocate(&name, &mut state.names),
                        value: state.media_value(media, &format!("{name}Body"), false)?,
                        media: Some(media.clone()),
                        forbidden: false,
                    });
                }
            }
            if conditional_none {
                cases.push(PlannedResponseCase {
                    source: response.source().use_site().source().clone(),
                    variant_type: models::allocate(&format!("{base}NoContent"), &mut state.names),
                    value: ValueType {
                        cpp_type: "Unit".into(),
                        kind: ValueKind::Unit,
                    },
                    media: None,
                    forbidden: true,
                });
            }
            responses.push(PlannedResponse {
                source: response.source().use_site().source().clone(),
                status: response.status(),
                status_key: response.status_key().into(),
                headers_type,
                headers,
                cases,
                wire: response.clone(),
            });
        }
        let id = op.operation_id().map(|id| id.value().clone());
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id: id
                .clone()
                .unwrap_or_else(|| format!("{} {}", op.method().as_str(), op.path())),
            declared_operation_id: id,
            method_name: models::allocate(&models::snake(&base), &mut methods),
            input_type: format!("{stem}Input"),
            success_type: format!("{stem}Success"),
            error_type: format!("{stem}Error"),
            constructor,
            parameters,
            body,
            responses,
            wire: op.clone(),
        });
    }
    let aggregates = state.aggregates;
    let pagination = match config.sdk_defaults.as_ref() {
        Some(defaults) => {
            let outcome = wire::plan_pagination(&contract, &protocol, Some(defaults))?;
            Some(super::pagination::lower(
                &models,
                &outcome,
                &operations,
                &mut state.names,
                &mut methods,
            ))
        }
        None => None,
    };
    let oauth = match config.sdk_defaults.as_ref() {
        Some(defaults) => {
            let outcome = wire::plan_oauth(&contract, &protocol, Some(defaults))?;
            super::oauth::lower(&outcome, &mut state.names)
        }
        None => None,
    };
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Only the lowered emission is
    // conditional on the compiled discrimination evidence.
    let stream_semantics = wire::plan_stream_semantics(&contract, &protocol);
    let stream_events = super::stream_events::lower(
        &models,
        &stream_semantics,
        &operations,
        &mut state.names,
        &mut methods,
    );
    // Emission-ready incoming receipt helpers; receipts the v1 helpers cannot
    // represent surface as the shared plan errors. Lowered last so the fixed
    // surface names join the allocation only when receipts are emitted.
    let incoming = if incoming.is_empty() {
        None
    } else {
        Some(super::incoming::lower(&contract, &incoming, &models, &mut state.names)?)
    };
    let examples = if program.version == suspect_schema::OwnedProgram::V3_VERSION {
        super::scoped_examples::plan_resources(contract.clone(), &protocol, Default::default())
    } else if program.version == suspect_schema::OwnedProgram::V2_VERSION {
        super::scoped_examples::plan(contract.clone(), &protocol, Default::default())
    } else {
        examples::plan_protocol_examples(contract.clone(), &protocol, Default::default())
    };
    Ok(SdkPlan {
        contract,
        config,
        models,
        operations,
        credentials,
        credential_env,
        aggregates,
        program,
        examples,
        protocol,
        pagination,
        oauth,
        stream_events,
        incoming,
    })
}

fn validate_native_boundaries(
    contract: &Contract,
    protocol: &wire::ProtocolPlan,
) -> Result<(), Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let mut header_check = |header: &wire::HeaderPlan| {
        if matches!(
            header.serialization(),
            wire::ParameterSerialization::Style {
                shape: wire::WireShape::FlatObject {
                    additional: wire::AdditionalScalars::AnyScalar,
                    ..
                },
                ..
            }
        ) {
            errors.push(diagnostic(contract,header.codec().schema().id().clone(),"cpp-header-value-ambiguous","response/MIME header object extras require a declared scalar type; text cannot reveal an undeclared scalar kind"));
        }
    };
    for operation in protocol.operations() {
        for header in operation.responses().iter().flat_map(|r| r.headers()) {
            header_check(header);
        }
        for media in operation
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(operation.responses().iter().flat_map(|r| r.media()))
        {
            let parts = match media.representation() {
                wire::Representation::Multipart {
                    multipart: wire::MultipartPlan::Named { parts, .. },
                } => parts.as_slice(),
                wire::Representation::Form { form } => form.fields(),
                _ => &[],
            };
            for part in parts {
                for header in part.headers() {
                    header_check(header);
                }
            }
        }
    }
    for operation in protocol.operations() {
        for parameter in operation.parameters() {
            if parameter.location() == wire::ParameterLocation::Header
                && [
                    "host",
                    "content-length",
                    "transfer-encoding",
                    "connection",
                    "expect",
                ]
                .contains(&parameter.name().to_ascii_lowercase().as_str())
            {
                errors.push(diagnostic(contract,parameter.source().use_site().source().clone(),"cpp-transport-header-owned","this header controls HTTP framing/routing and needs a dedicated native transport profile"));
            }
        }
        for media in operation
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(operation.responses().iter().flat_map(|r| r.media()))
        {
            if let wire::Representation::Multipart {
                multipart:
                    wire::MultipartPlan::Named {
                        parts, additional, ..
                    },
            } = media.representation()
            {
                for part in parts.iter().chain(match additional {
                    wire::AdditionalParts::Allowed(part) => Some(part.as_ref()),
                    wire::AdditionalParts::Forbidden => None,
                }) {
                    for header in part.headers() {
                        if ["content-disposition", "content-transfer-encoding"]
                            .contains(&header.name().to_ascii_lowercase().as_str())
                        {
                            errors.push(diagnostic(contract,header.source().use_site().source().clone(),"cpp-mime-header-profile","Content-Disposition is owned by named-part metadata; Content-Transfer-Encoding requires a separate codec profile"));
                        }
                    }
                }
            }
        }
        for response in operation.responses() {
            for media in response.media() {
                let parts = match media.representation() {
                    wire::Representation::Form { form } => form.fields(),
                    wire::Representation::Multipart {
                        multipart: wire::MultipartPlan::Named { parts, .. },
                    } => parts,
                    _ => &[],
                };
                for part in parts {
                    if matches!(
                        part.representation(),
                        wire::PartRepresentation::Style {
                            serialization: wire::ParameterSerialization::Style {
                                shape: wire::WireShape::FlatObject {
                                    additional: wire::AdditionalScalars::AnyScalar,
                                    ..
                                },
                                ..
                            },
                            ..
                        }
                    ) {
                        errors.push(diagnostic(contract,part.source().use_site().source().clone(),"cpp-part-value-ambiguous","styled response part extras require a declared scalar type; use JSON-content parts for arbitrary JSON values"));
                    }
                }
                if let wire::Representation::Form { form } = media.representation() {
                    for part in form.fields() {
                        if matches!(
                            part.representation(),
                            wire::PartRepresentation::Style {
                                serialization: wire::ParameterSerialization::Style {
                                    explode: true,
                                    shape: wire::WireShape::FlatObject { .. },
                                    ..
                                },
                                ..
                            } | wire::PartRepresentation::Style {
                                serialization: wire::ParameterSerialization::Style {
                                    style: wire::Style::DeepObject,
                                    shape: wire::WireShape::FlatObject { .. },
                                    ..
                                },
                                ..
                            }
                        ) {
                            errors.push(diagnostic(contract,part.source().use_site().source().clone(),"cpp-form-object-response-ambiguous","flattened object response forms need an explicit disjoint-key ownership profile; JSON-content object form fields are supported"));
                        }
                    }
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn present(ty: &str, required: bool) -> String {
    if required {
        ty.into()
    } else {
        format!("Presence<{ty}>")
    }
}
fn operation_base(op: &wire::OperationPlan) -> String {
    models::pascal(
        &op.operation_id()
            .map(|id| id.value().clone())
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    op.method().as_str().to_ascii_lowercase(),
                    op.path()
                )
            }),
    )
}
pub(crate) fn media_suffix(media: &wire::MediaPlan) -> String {
    models::pascal(media.media_type().declared())
}
fn seed_media(media: &wire::MediaPlan, name: &str, seeds: &mut BTreeMap<SchemaId, String>) {
    match media.representation() {
        wire::Representation::Json { codec: Some(codec) }
        | wire::Representation::Text {
            codec: Some(codec), ..
        } => {
            seeds.insert(codec.schema().id().clone(), name.into());
        }
        wire::Representation::Stream { stream } => {
            if let Some(codec) = stream.item_codec() {
                seeds.insert(codec.schema().id().clone(), format!("{name}Item"));
            }
        }
        wire::Representation::Form { form } => {
            for part in form.fields() {
                seed_part(part, name, seeds);
            }
        }
        wire::Representation::Multipart {
            multipart: wire::MultipartPlan::Named { parts, .. },
        } => {
            for part in parts {
                seed_part(part, name, seeds);
            }
        }
        _ => {}
    }
}
fn seed_part(part: &wire::PartPlan, name: &str, seeds: &mut BTreeMap<SchemaId, String>) {
    let name = format!("{name}{}", models::pascal(part.name().unwrap_or("Extra")));
    match part.representation() {
        wire::PartRepresentation::Json { codec, .. }
        | wire::PartRepresentation::Text { codec, .. }
        | wire::PartRepresentation::Style { codec, .. } => {
            seeds.insert(codec.schema().id().clone(), format!("{name}Value"));
        }
        _ => {}
    }
    for header in part.headers() {
        seeds.insert(
            header.codec().schema().id().clone(),
            format!("{name}Header{}", models::pascal(header.name())),
        );
    }
}
struct State<'a> {
    models: &'a models::ModelPlan,
    namespace: &'a str,
    names: BTreeSet<String>,
    aggregates: Vec<PlannedAggregate>,
}
impl State<'_> {
    fn qualified(&self, name: &str) -> String {
        format!("::{}::{name}", self.namespace)
    }
    fn model(&self, id: &SchemaId) -> ValueType {
        ValueType {
            cpp_type: self.models.get(id).cpp_type.clone(),
            kind: ValueKind::Model(id.clone()),
        }
    }
    fn headers(
        &mut self,
        headers: &[wire::HeaderPlan],
        name: &str,
    ) -> (Option<String>, Vec<PlannedHeader>) {
        let name = (!headers.is_empty()).then(|| models::allocate(name, &mut self.names));
        let mut names = BTreeSet::new();
        (
            name,
            headers
                .iter()
                .map(|h| PlannedHeader {
                    source: h.source().use_site().source().clone(),
                    field_name: models::allocate(&models::snake(h.name()), &mut names),
                    value: self.model(h.codec().schema().id()),
                    wire: h.clone(),
                })
                .collect(),
        )
    }
    fn part(
        &mut self,
        part: &wire::PartPlan,
        parent: &str,
        multipart: bool,
        members: &mut BTreeSet<String>,
    ) -> PlannedPart {
        let name = part.name().map(str::to_owned);
        let base = format!("{parent}{}", models::pascal(part.name().unwrap_or("Extra")));
        let value = match part.representation() {
            wire::PartRepresentation::Json { codec, .. }
            | wire::PartRepresentation::Text { codec, .. }
            | wire::PartRepresentation::Style { codec, .. } => self.model(codec.schema().id()),
            wire::PartRepresentation::Binary { .. } => ValueType {
                cpp_type: "Bytes".into(),
                kind: ValueKind::Bytes,
            },
        };
        let (headers_type, headers) = self.headers(part.headers(), &format!("{base}Headers"));
        PlannedPart {
            source: part.source().use_site().source().clone(),
            field_name: models::allocate(&models::snake(part.name().unwrap_or("extra")), members),
            name,
            value,
            part_type: multipart.then(|| models::allocate(&format!("{base}Part"), &mut self.names)),
            headers_type,
            headers,
            wire: part.clone(),
        }
    }
    fn media_value(
        &mut self,
        media: &wire::MediaPlan,
        name: &str,
        request: bool,
    ) -> Result<ValueType, Vec<HttpDiagnostic>> {
        Ok(match media.representation() {
            wire::Representation::Json { codec: Some(codec) }
            | wire::Representation::Text {
                codec: Some(codec), ..
            } => self.model(codec.schema().id()),
            wire::Representation::Json { codec: None } => ValueType {
                cpp_type: "JsonValue".into(),
                kind: ValueKind::Json,
            },
            wire::Representation::Text {
                codec: None,
                scalar,
                ..
            } => ValueType {
                cpp_type: match scalar {
                    wire::ScalarType::String => "std::string",
                    wire::ScalarType::Boolean => "bool",
                    wire::ScalarType::Integer => "JsonInteger",
                    wire::ScalarType::Number => "JsonNumber",
                }
                .into(),
                kind: ValueKind::Scalar(*scalar),
            },
            wire::Representation::Binary { .. } => ValueType {
                cpp_type: "Bytes".into(),
                kind: ValueKind::Bytes,
            },
            wire::Representation::Stream { stream } => match stream.item_codec() {
                Some(codec) => {
                    let schema = codec.schema().id();
                    ValueType {
                        cpp_type: format!(
                            "{}<{}>",
                            if request { "std::vector" } else { "ItemStream" },
                            self.models.get(schema).cpp_type
                        ),
                        kind: ValueKind::Stream {
                            schema: schema.clone(),
                            framing: stream.framing(),
                            max_item_bytes: stream.max_item_bytes() as usize,
                        },
                    }
                }
                // A schemaless stream surfaces untyped whole-body JSON values
                // because the native stream runtime has no untyped codec.
                None => ValueType {
                    cpp_type: if request { "std::vector<Json>" } else { "Json" }.into(),
                    kind: ValueKind::Json,
                },
            },
            representation => {
                let (rules, parts, extra, multipart) = match representation {
                    wire::Representation::Form { form } => {
                        (form.rules(), form.fields(), form.additional(), false)
                    }
                    wire::Representation::Multipart {
                        multipart:
                            wire::MultipartPlan::Named {
                                rules,
                                parts,
                                additional,
                            },
                    } => (rules, parts.as_slice(), additional, true),
                    _ => unreachable!("positional multipart blocked by capability admission"),
                };
                let type_name = models::allocate(name, &mut self.names);
                let mut members = BTreeSet::from(["extra".into()]);
                let fields = parts
                    .iter()
                    .map(|part| self.part(part, &type_name, multipart, &mut members))
                    .collect();
                let additional = match extra {
                    wire::AdditionalParts::Forbidden => None,
                    wire::AdditionalParts::Allowed(part) => {
                        Some(self.part(part, &type_name, multipart, &mut BTreeSet::new()))
                    }
                };
                self.aggregates.push(PlannedAggregate {
                    source: media.source().use_site().source().clone(),
                    type_name: type_name.clone(),
                    multipart,
                    fields,
                    additional,
                    rules: rules.clone(),
                });
                ValueType {
                    cpp_type: self.qualified(&type_name),
                    kind: ValueKind::Aggregate(type_name),
                }
            }
        })
    }
}
