use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::http_protocol::{self as wire, *};

/// Explicit native implementation profile. New shared vocabulary is never
/// enabled implicitly by `Capability::ALL` or by a serialized descriptor.
#[must_use]
pub fn native_capabilities() -> Capabilities {
    Capabilities::for_adapter(
        "rust-http-protocol-v1",
        [
            Capability::UnnamedOperations,
            Capability::AdditionalMethods,
            Capability::CustomMethods,
            Capability::HttpServers,
            Capability::RelativeServers,
            Capability::DocumentRelativeServers,
            Capability::MultipleServers,
            Capability::ServerVariables,
            Capability::AnonymousSecurity,
            Capability::SecurityAlternatives,
            Capability::ConjunctiveSecurity,
            Capability::HttpBasic,
            Capability::ApiKeys,
            Capability::OAuth2,
            Capability::OpenIdConnect,
            Capability::SecurityRoles,
            Capability::ParameterStyles,
            Capability::HeaderParameters,
            Capability::CookieParameters,
            Capability::ReservedParameters,
            Capability::ContentParameters,
            Capability::QuerystringParameters,
            Capability::QuerystringForm,
            Capability::RangeResponses,
            Capability::DefaultResponses,
            Capability::UndeclaredResponses,
            Capability::MultipleMediaTypes,
            Capability::MediaRanges,
            Capability::MediaTypeParameters,
            Capability::StructuredJsonMedia,
            Capability::SchemaFreeJson,
            Capability::TextBodies,
            Capability::BinaryBodies,
            Capability::UndeclaredResponseBody,
            Capability::ResponseHeaders,
            Capability::ResponseLinks,
            Capability::FormBodies,
            Capability::MultipartBodies,
            Capability::PartEncodings,
            Capability::PositionalMultipart,
            Capability::ServerSentEvents,
            Capability::JsonLines,
            Capability::OpenApi30,
            Capability::OpenApi32,
        ],
    )
}

/// Resource/dynamic execution is an explicit native profile; v1/v2 retain their
/// resource fences. All protocol behavior remains shared with the base profile.
#[must_use]
pub fn native_capabilities_v3() -> Capabilities {
    Capabilities::for_adapter(
        "rust-http-resources-v3",
        native_capabilities().enabled().iter().copied(),
    )
    .with(Capability::SchemaResources)
    .with(Capability::DynamicSchemaReferences)
}

/// Plan selected operations from the shared rich protocol plan and bind its
/// actual codec roots to the existing native codec/compiler pipeline.
pub fn plan_http(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    plan_with_codecs(contract, selected, config, false, false)
}

/// Plan the same native HTTP protocol with explicit scoped-applicator codecs
/// and source-backed v2 examples. Existing `plan_http` remains the v1 entry.
pub fn plan_http_v2(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    plan_with_codecs(contract, selected, config, true, false)
}

/// Resource-capable SDK with source-selected v3 codec/example admission.
/// Ordinary closures retain the complete v2 HTTP plan and artifact bytes,
/// including its adapter/capability metadata and v1/v2 validation program.
pub fn plan_http_v3(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    plan_with_codecs(contract, selected, config, true, true)
}

fn plan_with_codecs(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
    scoped: bool,
    resources: bool,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    let fallback = selected
        .first()
        .cloned()
        .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()));
    let mut errors = Vec::new();
    if selected.is_empty() {
        errors.push(diagnostic(
            &contract,
            fallback.clone(),
            "http-no-operations",
            "select at least one outgoing operation",
        ));
    }
    let limits = [
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_part_bytes,
        config.max_stream_item_bytes,
        config.max_chunk_bytes,
        config.max_header_bytes,
    ];
    if limits.iter().any(|&v| v == 0 || u32::try_from(v).is_err()) {
        errors.push(diagnostic(
            &contract,
            fallback,
            "http-resource-policy",
            "HTTP byte ceilings must be positive and fit a portable 32-bit usize",
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let byte_limits = ByteLimits::new(
        u64::try_from(config.max_request_bytes.max(config.max_response_bytes))
            .expect("portable limit"),
        u64::try_from(config.max_part_bytes).expect("portable limit"),
        u64::try_from(config.max_stream_item_bytes).expect("portable limit"),
    );
    let configured = |capabilities: Capabilities| {
        config.compatibility_profiles.iter().fold(
            capabilities.with_limits(byte_limits),
            |capabilities, profile| capabilities.with_profile(*profile),
        )
    };
    let mut protocol = wire::plan(&contract, selected, configured(native_capabilities()));
    // Keep the baseline descriptor/example profile until the selected inputs
    // require resources. Typed capability findings include canonical anchors;
    // native scope admission also covers an id-less ancestor dynamic anchor
    // that need not itself occur among the selected closure's schema nodes.
    let resources = resources
        && (protocol.diagnostics().iter().any(|finding| {
            matches!(
                finding.capability(),
                Some(Capability::SchemaResources | Capability::DynamicSchemaReferences)
            )
        }) || rust_models::resources::required(&contract, protocol.codec_roots()));
    if resources {
        protocol = wire::plan(&contract, selected, configured(native_capabilities_v3()));
    }
    let protocol = protocol.into_result().map_err(|findings| {
        findings
            .into_iter()
            .filter(|d| d.severity() == Severity::Error)
            .map(|d| HttpDiagnostic {
                source: d.source().source().clone(),
                at: d.source().span(),
                code: d.code(),
                message: d.message().into(),
            })
            .collect::<Vec<_>>()
    })?;
    // RFC6570 form expansion needs a variable name. Positional parts have no
    // such binding; do not invent an empty name for an Encoding Object.
    for operation in protocol.operations() {
        for media in operation
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(operation.responses().iter().flat_map(|r| r.media()))
        {
            if let Representation::Multipart {
                multipart: MultipartPlan::Positional { prefix, items, .. },
            } = media.representation()
            {
                for part in prefix.iter().chain(match items {
                    AdditionalParts::Allowed(part) => Some(part.as_ref()),
                    AdditionalParts::Forbidden => None,
                }) {
                    if matches!(part.representation(), PartRepresentation::Style { .. }) {
                        let at = part
                            .encoding_source()
                            .map(|s| s.use_site().source())
                            .unwrap_or_else(|| part.source().use_site().source());
                        errors.push(diagnostic(&contract,at.clone(),"http-rust-positional-style-unsupported","positional RFC6570 part expansion has no declared variable name; use explicit JSON/text/byte content encoding"));
                    }
                }
            }
        }
    }
    for id in protocol.codec_schema_closure() {
        if let Some(schema) = contract.schema(id) {
            let raw = crate::schema_view::raw(schema);
            for keyword in ["readOnly", "writeOnly"] {
                if raw
                    .get(keyword)
                    .is_some_and(|v| v != &serde_json::Value::Bool(false))
                {
                    errors.push(diagnostic(&contract, id.child(keyword), "http-directional-codec-unsupported", "the Rust HTTP profile requires directional annotations to be absent or false throughout its neutral codec closure"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let credential_env =
        crate::credential_env::plan(&contract, &protocol, config.credential_env.as_ref())?;
    let codec_planner = if resources {
        crate::rust_codecs::plan_codecs_v3
    } else if scoped {
        crate::rust_codecs::plan_codecs_v2
    } else {
        crate::rust_codecs::plan_codecs
    };
    let codecs = codec_planner(
        contract.clone(),
        protocol.codec_roots(),
        config.codecs.clone(),
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.at,
                code: e.code,
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let symbols: BTreeMap<_, _> = codecs
        .models()
        .symbols()
        .iter()
        .filter(|s| s.role() == rust_models::RepresentationRole::Model)
        .map(|s| (s.source().clone(), s.name().to_owned()))
        .collect();
    let mut modules = BTreeSet::new();
    let mut methods = [
        "with_transport",
        "with_options",
        "send",
        "open",
        "limits",
        "with_reqwest",
        "drop",
        "with_credential_hook",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut credential_methods = [
        "new",
        "default",
        "with_bearer",
        "bearer_token",
        "with_basic",
        "with_api_key",
        "with_authorization",
        "with_source_bearer",
        "with_source_basic",
        "with_source_api_key",
        "with_source_authorization",
        "with_hook",
        "select_alternative",
        "get",
        "insert",
        "attach",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut credentials = BTreeMap::new();
    if credential_env.is_some() {
        methods.extend(["from_env".into(), "with_transport_from_env".into()]);
        credential_methods.insert("from_env".into());
    }
    let mut operations = Vec::new();
    for op in protocol.operations() {
        let operation_id = op
            .operation_id()
            .map(|v| v.value().clone())
            .unwrap_or_else(|| {
                format!(
                    "{}_{}",
                    op.method().as_str().to_ascii_lowercase(),
                    op.path()
                )
            });
        let module_name = allocate(&rust_models::snake(&operation_id), &mut modules);
        let function_name = allocate(&rust_models::snake(&operation_id), &mut methods);
        let stem = rust_models::pascal(&operation_id);
        let mut types = [
            stem.clone(),
            format!("{stem}Success"),
            format!("{stem}Error"),
            format!("{stem}ApiError"),
        ]
        .into_iter()
        .collect();
        let mut members = ["body", "content_type"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let parameters = op
            .parameters()
            .iter()
            .map(|p| {
                let base = rust_models::snake(p.name());
                let proposed = if members.contains(&base) {
                    format!(
                        "{}_{base}",
                        format!("{:?}", p.location()).to_ascii_lowercase()
                    )
                } else {
                    base
                };
                PlannedParameter {
                    wire: p.clone(),
                    name: allocate(&proposed, &mut members),
                    model: symbols[p.codec().schema().id()].clone(),
                }
            })
            .collect::<Vec<_>>();
        let body = op.body().map(|b| {
            let choice_name = allocate(&format!("{stem}Body"), &mut types);
            let mut variants = BTreeSet::new();
            let media = b
                .media()
                .iter()
                .map(|m| {
                    let variant = allocate(&media_name(m.media_type()), &mut variants);
                    let payload = payload(
                        m,
                        &format!("{choice_name}{variant}"),
                        true,
                        &symbols,
                        &mut types,
                    );
                    PlannedMedia {
                        wire: m.clone(),
                        variant,
                        payload,
                    }
                })
                .collect::<Vec<_>>();
            let type_name = if media.len() == 1 {
                super::emit::payload_type(&media[0].payload)
            } else {
                choice_name
            };
            PlannedBody {
                wire: b.clone(),
                type_name,
                media,
            }
        });
        let default_function_name = if parameters.iter().all(|p| !p.wire.required())
            && body.as_ref().is_none_or(|b| !b.wire.required())
        {
            Some(allocate(&format!("{function_name}_default"), &mut methods))
        } else {
            None
        };
        let mut response_names = BTreeSet::new();
        let responses = op
            .responses()
            .iter()
            .map(|r| {
                let base = match r.status() {
                    ResponseStatus::Exact(s) => format!("Status{s}"),
                    ResponseStatus::Range(s) => format!("Range{s}XX"),
                    ResponseStatus::Default => "Default".into(),
                };
                let headers_type = (!r.headers().is_empty())
                    .then(|| allocate(&format!("{stem}{base}Headers"), &mut types));
                let headers = headers(r.headers(), &symbols);
                let success = match r.status() {
                    ResponseStatus::Exact(s) => (200..300).contains(&s),
                    ResponseStatus::Range(s) => s == 2,
                    ResponseStatus::Default => true,
                };
                let error = match r.status() {
                    ResponseStatus::Exact(s) => !(200..300).contains(&s),
                    ResponseStatus::Range(s) => s != 2,
                    ResponseStatus::Default => true,
                };
                let always_forbidden = op.method().as_str() == "HEAD"
                    || matches!(
                        r.status(),
                        ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                            | ResponseStatus::Range(1)
                    );
                let sometimes_forbidden = !always_forbidden
                    && matches!(
                        r.status(),
                        ResponseStatus::Range(2 | 3) | ResponseStatus::Default
                    );
                let mut variants = Vec::new();
                if always_forbidden || r.media().is_empty() {
                    variants.push(PlannedResponseVariant {
                        name: allocate(&base, &mut response_names),
                        media_index: None,
                        success,
                        error,
                        forbidden: always_forbidden,
                        payload: if always_forbidden {
                            Payload::NoContent
                        } else {
                            Payload::Bytes
                        },
                    });
                } else {
                    for (index, m) in r.media().iter().enumerate() {
                        let proposed = if r.media().len() == 1 {
                            base.clone()
                        } else {
                            format!("{base}{}", media_name(m.media_type()))
                        };
                        let name = allocate(&proposed, &mut response_names);
                        variants.push(PlannedResponseVariant {
                            payload: payload(
                                m,
                                &format!("{stem}{name}Body"),
                                false,
                                &symbols,
                                &mut types,
                            ),
                            name,
                            media_index: Some(index),
                            success,
                            error,
                            forbidden: false,
                        });
                    }
                }
                if sometimes_forbidden && !r.media().is_empty() {
                    variants.push(PlannedResponseVariant {
                        name: allocate(&format!("{base}NoContent"), &mut response_names),
                        media_index: None,
                        success,
                        error,
                        forbidden: true,
                        payload: Payload::NoContent,
                    });
                }
                PlannedResponse {
                    wire: r.clone(),
                    headers_type,
                    headers,
                    variants,
                }
            })
            .collect();
        let mut operation_credentials = Vec::new();
        for requirement in op
            .security()
            .alternatives()
            .iter()
            .flat_map(|a| a.requirements())
        {
            let source = requirement.scheme().use_site().source().clone();
            let binding = credentials
                .entry(source)
                .or_insert_with(|| PlannedCredential {
                    constructor: allocate(
                        &rust_models::snake(requirement.name()),
                        &mut credential_methods,
                    ),
                    requirement: requirement.clone(),
                });
            operation_credentials.push(PlannedCredential {
                constructor: binding.constructor.clone(),
                requirement: requirement.clone(),
            });
        }
        operations.push(PlannedOperation {
            source: op.source().terminal().source().clone(),
            operation_id,
            module_name,
            function_name,
            input_type: stem.clone(),
            success_type: format!("{stem}Success"),
            error_type: format!("{stem}Error"),
            api_error_type: format!("{stem}ApiError"),
            default_function_name,
            wire: op.clone(),
            parameters,
            body,
            responses,
            credentials: operation_credentials,
        });
    }
    let example_planner = if resources {
        crate::examples::plan_protocol_examples_v3
    } else if scoped {
        crate::examples::plan_protocol_examples_v2
    } else {
        crate::examples::plan_protocol_examples
    };
    let examples = example_planner(contract.clone(), &protocol, Default::default());
    Ok(HttpPlan {
        contract,
        protocol,
        operations,
        codecs,
        symbols,
        credentials,
        config,
        examples,
        credential_env,
    })
}

fn media_name(media: &MediaType) -> String {
    let base = match media.range() {
        MediaRange::Any => "Any".into(),
        MediaRange::Type { type_name } => format!("{}Any", rust_models::pascal(type_name)),
        MediaRange::Concrete { type_name, subtype } => {
            rust_models::pascal(&format!("{type_name}_{subtype}"))
        }
    };
    if media.parameters().is_empty() {
        base
    } else {
        format!(
            "{base}{}",
            rust_models::pascal(
                &media
                    .parameters()
                    .iter()
                    .map(|(k, v)| format!("{k}_{v}"))
                    .collect::<Vec<_>>()
                    .join("_")
            )
        )
    }
}

fn headers(wire: &[HeaderPlan], symbols: &BTreeMap<SchemaId, String>) -> Vec<PlannedHeader> {
    let mut names = BTreeSet::new();
    wire.iter()
        .map(|h| PlannedHeader {
            wire: h.clone(),
            name: allocate(&rust_models::snake(h.name()), &mut names),
            model: symbols[h.codec().schema().id()].clone(),
        })
        .collect()
}

fn part(
    wire: &PartPlan,
    name: &str,
    stem: &str,
    symbols: &BTreeMap<SchemaId, String>,
    types: &mut BTreeSet<String>,
) -> PlannedPart {
    let codec = match wire.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => Some(codec),
        PartRepresentation::Binary { .. } => None,
    };
    PlannedPart {
        wire: wire.clone(),
        name: name.into(),
        model: codec.map(|c| symbols[c.schema().id()].clone()),
        headers_type: (!wire.headers().is_empty())
            .then(|| allocate(&format!("{stem}Headers"), types)),
        headers: headers(wire.headers(), symbols),
    }
}

fn payload(
    media: &MediaPlan,
    stem: &str,
    request: bool,
    symbols: &BTreeMap<SchemaId, String>,
    types: &mut BTreeSet<String>,
) -> Payload {
    match media.representation() {
        Representation::Json { codec: Some(codec) }
        | Representation::Text {
            codec: Some(codec), ..
        } => Payload::Model {
            schema: codec.schema().id().clone(),
            model: symbols[codec.schema().id()].clone(),
        },
        Representation::Json { codec: None } => Payload::Json,
        Representation::Text { codec: None, .. } => Payload::Text,
        Representation::Binary { .. } => Payload::Bytes,
        Representation::Stream { stream } => Payload::Stream {
            schema: stream.item_codec().schema().id().clone(),
            model: symbols[stream.item_codec().schema().id()].clone(),
            request,
        },
        Representation::Form { .. } | Representation::Multipart { .. } => {
            let (multipart, positional, parts, additional) = match media.representation() {
                Representation::Form { form } => (false, false, form.fields(), form.additional()),
                Representation::Multipart {
                    multipart:
                        MultipartPlan::Named {
                            parts, additional, ..
                        },
                } => (true, false, parts.as_slice(), additional),
                Representation::Multipart {
                    multipart: MultipartPlan::Positional { prefix, items, .. },
                } => (true, true, prefix.as_slice(), items),
                _ => unreachable!("aggregate representation"),
            };
            let type_name = allocate(stem, types);
            let mut names = ["additional".into(), "items".into()].into_iter().collect();
            let parts = parts
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    let name = allocate(
                        &p.name()
                            .map(rust_models::snake)
                            .unwrap_or_else(|| format!("part_{index}")),
                        &mut names,
                    );
                    part(
                        p,
                        &name,
                        &format!("{type_name}{}", rust_models::pascal(&name)),
                        symbols,
                        types,
                    )
                })
                .collect();
            let additional = match additional {
                AdditionalParts::Forbidden => None,
                AdditionalParts::Allowed(p) => Some(Box::new(part(
                    p,
                    if positional { "items" } else { "additional" },
                    &format!("{type_name}Additional"),
                    symbols,
                    types,
                ))),
            };
            Payload::Parts(Box::new(PlannedAggregate {
                type_name,
                multipart,
                positional,
                parts,
                additional,
            }))
        }
    }
}
