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

/// How a planned value is represented and encoded in the generated C++.
#[derive(Debug, Clone)]
pub enum ValueKind {
    /// A generated model type with a JSON codec.
    Model(SchemaId),
    /// Untyped JSON, surfaced as the runtime's `JsonValue`/`Json`.
    Json,
    /// A schemaless text value decoded as one scalar.
    Scalar(wire::ScalarType),
    /// Opaque bytes.
    Bytes,
    /// No payload (`Unit`).
    Unit,
    /// A generated form or multipart aggregate, by type name.
    Aggregate(String),
    /// A generated request-body variant over several media types, by type name.
    Choice(String),
    /// A framed stream of codec-backed items.
    Stream {
        /// Schema of each stream item.
        schema: SchemaId,
        /// Wire framing of the stream.
        framing: wire::StreamFraming,
        /// Per-item byte ceiling declared by the protocol plan.
        max_item_bytes: usize,
    },
}
/// A planned value's spelled C++ type together with its representation.
#[derive(Debug, Clone)]
pub struct ValueType {
    /// Fully spelled C++ type (qualified, before any `Presence<..>` wrapping).
    pub cpp_type: String,
    /// Representation used to encode/decode the value.
    pub kind: ValueKind,
}
impl ValueType {
    /// The schema backing a model or stream value, if any.
    pub fn schema(&self) -> Option<&SchemaId> {
        match &self.kind {
            ValueKind::Model(id) | ValueKind::Stream { schema: id, .. } => Some(id),
            _ => None,
        }
    }
}
/// A planned request or response header.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Declaring source location.
    pub source: SourceId,
    /// Allocated C++ member name.
    pub field_name: String,
    /// Planned header value.
    pub value: ValueType,
    /// Shared wire plan for the header.
    pub wire: wire::HeaderPlan,
}
/// One field of a form or multipart aggregate.
#[derive(Debug, Clone)]
pub struct PlannedPart {
    /// Declaring source location.
    pub source: SourceId,
    /// Allocated C++ member name.
    pub field_name: String,
    /// Declared wire field name; `None` for the additional-fields catch-all.
    pub name: Option<String>,
    /// Planned field value.
    pub value: ValueType,
    /// Generated per-part wrapper type; multipart only.
    pub part_type: Option<String>,
    /// Generated type holding the part's headers, when it declares any.
    pub headers_type: Option<String>,
    /// Planned per-part headers.
    pub headers: Vec<PlannedHeader>,
    /// Shared wire plan for the part.
    pub wire: wire::PartPlan,
}
/// A generated `application/x-www-form-urlencoded` or multipart aggregate type.
#[derive(Debug, Clone)]
pub struct PlannedAggregate {
    /// Declaring media source location.
    pub source: SourceId,
    /// Allocated, unqualified C++ type name.
    pub type_name: String,
    /// Whether this is multipart (otherwise a URL-encoded form).
    pub multipart: bool,
    /// Declared fields in wire order.
    pub fields: Vec<PlannedPart>,
    /// Catch-all for undeclared fields, when additional fields are allowed.
    pub additional: Option<PlannedPart>,
    /// Shared object rules (required/additional handling) for the aggregate.
    pub rules: wire::ObjectRules,
}
/// One media type of a request body.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Declaring media source location.
    pub source: SourceId,
    /// Planned payload value for this media type.
    pub value: ValueType,
    /// Generated wrapper type pairing the value with its content type.
    pub wrapper_type: String,
    /// Whether the caller must supply a concrete content type (the declared media is a range).
    pub requires_content_type: bool,
    /// Shared wire plan for the media type.
    pub wire: wire::MediaPlan,
}
/// A planned operation request body.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Declaring source location.
    pub source: SourceId,
    /// Whether the body is required.
    pub required: bool,
    /// Input member type: `value.cpp_type`, wrapped in `Presence<..>` when optional.
    pub cpp_type: String,
    /// Body value; a [`ValueKind::Choice`] when several media types are declared.
    pub value: ValueType,
    /// Planned media types in declaration order.
    pub media: Vec<PlannedMedia>,
    /// Generated variant type, present when the body is a choice over media types.
    pub choice_type: Option<String>,
}
/// A planned operation parameter (path, query, header or cookie).
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Declaring use-site source location.
    pub source: SourceId,
    /// Allocated C++ input member name.
    pub field_name: String,
    /// Declared parameter name sent on the wire.
    pub wire_name: String,
    /// Whether the parameter is required.
    pub required: bool,
    /// Input member type: `value.cpp_type`, wrapped in `Presence<..>` when optional.
    pub cpp_type: String,
    /// Planned parameter value.
    pub value: ValueType,
    /// Shared wire plan for the parameter.
    pub wire: wire::ParameterPlan,
}
/// One alternative of a planned response: a media type or a no-content case.
#[derive(Debug, Clone)]
pub struct PlannedResponseCase {
    /// Declaring source location.
    pub source: SourceId,
    /// Generated variant type for this case.
    pub variant_type: String,
    /// Decoded payload value.
    pub value: ValueType,
    /// Media plan; `None` for bodiless cases.
    pub media: Option<wire::MediaPlan>,
    /// Whether a body is forbidden: HEAD, 1xx/204/205/304, or the implicit no-content case of a range/default response.
    pub forbidden: bool,
}
/// A planned response for one declared status, status range or default.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Declaring source location.
    pub source: SourceId,
    /// Declared status selector.
    pub status: wire::ResponseStatus,
    /// Declared status key, as spelled in the responses map.
    pub status_key: String,
    /// Generated type holding the response headers, when any are declared.
    pub headers_type: Option<String>,
    /// Planned response headers.
    pub headers: Vec<PlannedHeader>,
    /// Cases in match order.
    pub cases: Vec<PlannedResponseCase>,
    /// Shared wire plan for the response.
    pub wire: wire::ResponsePlan,
}
impl PlannedResponse {
    /// Whether this response can land in the success result (2xx, 2XX or default).
    pub fn can_succeed(&self) -> bool {
        matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299)
                | wire::ResponseStatus::Range(2)
                | wire::ResponseStatus::Default
        )
    }
    /// Whether this response can land in the error result (anything but 2xx/2XX).
    pub fn can_fail(&self) -> bool {
        !matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        )
    }
}
/// A planned security-scheme credential member on the client.
#[derive(Debug, Clone)]
pub struct PlannedCredential {
    /// Security-scheme source location.
    pub source: SourceId,
    /// Allocated C++ member name.
    pub field_name: String,
    /// C++ type of the credential member.
    pub cpp_type: String,
    /// Shared wire requirement for the credential.
    pub wire: wire::CredentialRequirement,
}
/// One positional parameter of an operation input constructor.
#[derive(Debug, Clone)]
pub struct InputArgument {
    /// Constructor parameter name (`argN`).
    pub name: String,
    /// Input member initialized from this argument.
    pub member_name: String,
    /// Argument value.
    pub value: ValueType,
    /// Spelled C++ parameter type.
    pub cpp_type: String,
}
/// The positional constructor of an operation input, over its required members.
#[derive(Debug, Clone)]
pub struct InputConstructor {
    /// Input type name the constructor belongs to.
    pub name: String,
    /// Required parameters, then a required body, in order.
    pub parameters: Vec<InputArgument>,
}
/// A planned outgoing operation with its generated input, success and error types.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Operation source location.
    pub source: SourceId,
    /// Declared `operationId`, or `METHOD path` when undeclared.
    pub operation_id: String,
    /// `operationId` exactly as declared, if any.
    pub declared_operation_id: Option<String>,
    /// Allocated client method name.
    pub method_name: String,
    /// Generated input type name.
    pub input_type: String,
    /// Generated success result type name.
    pub success_type: String,
    /// Generated error result type name.
    pub error_type: String,
    /// Positional input constructor.
    pub constructor: InputConstructor,
    /// Planned parameters in declaration order.
    pub parameters: Vec<PlannedParameter>,
    /// Planned request body, if declared.
    pub body: Option<PlannedBody>,
    /// Planned responses in declaration order.
    pub responses: Vec<PlannedResponse>,
    /// Shared wire plan for the operation.
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
