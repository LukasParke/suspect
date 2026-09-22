//! Native allocation over the single shared protocol plan.
use super::*;
use protocol::{
    AdditionalParts, MultipartPlan, PartRepresentation, Representation, ResponseStatus,
};

/// Explicit opt-ins implemented by the generated standard-library runtime.
/// Unsupported shapes remain shared-planner errors; this is never `Capability::ALL`.
pub(super) fn capabilities(config: &HttpConfig) -> protocol::Capabilities {
    use protocol::Capability::*;
    let mut value = protocol::Capabilities::for_adapter(
        "go-net-http-protocol-v1",
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
            PositionalMultipart,
            ServerSentEvents,
            JsonLines,
            OpenApi30,
            OpenApi32,
            SchemaResources,
            DynamicSchemaReferences,
        ],
    )
    .with_limits(protocol::ByteLimits::new(
        config.max_request_bytes.max(config.max_response_bytes) as u64,
        config.max_part_bytes as u64,
        config.max_stream_item_bytes as u64,
    ));
    for profile in &config.compatibility_profiles {
        value = value.with_profile(*profile);
    }
    value
}

pub(super) fn plan(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    let fallback = selected
        .first()
        .cloned()
        .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()));
    if [
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_part_bytes,
        config.max_parts,
        config.max_stream_item_bytes,
    ]
    .iter()
    .any(|&n| n == 0 || n > i32::MAX as usize)
    {
        return Err(vec![diagnostic(
            &contract,
            fallback,
            "http-resource-policy",
            "Go HTTP byte ceilings must be positive and fit a portable signed 32-bit int",
        )]);
    }
    let wire = protocol::plan(&contract, selected, capabilities(&config))
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| HttpDiagnostic {
                    at: e.source().span(),
                    source: e.source().source().clone(),
                    code: e.code(),
                    message: e.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
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
    let credential_env = crate::credential_env::plan_with_defaults(
        &contract,
        &wire,
        config.credential_env.as_ref(),
        config.sdk_defaults.as_ref(),
    )?;
    // Compiled OAuth lifecycle planning follows the configured policy, like
    // pagination helpers; helpers are emitted only for usable schemes, so
    // no-policy output stays byte-identical.
    let oauth = super::oauth::plan(&contract, &wire, config.sdk_defaults.as_ref())?;
    let oauth_emits = oauth.as_ref().is_some_and(super::oauth::emits);
    let mut errors = Vec::new();
    for operation in wire.operations() {
        for parameter in operation.parameters() {
            if parameter.location() == protocol::ParameterLocation::Header
                && ["content-length", "transfer-encoding", "trailer"]
                    .contains(&parameter.name().to_ascii_lowercase().as_str())
            {
                errors.push(diagnostic(&contract,parameter.source().use_site().source().clone(),"http-go-header-framing-unsupported","net/http owns message framing; this adapter cannot send a source parameter as Content-Length, Transfer-Encoding or Trailer"));
            }
        }
    }
    // Incoming receipts widen the checked closure exactly as their codec roots
    // widen the codec table; receipt-less plans keep the selected closure.
    let mut directional: BTreeSet<SchemaId> = wire.codec_schema_closure().iter().cloned().collect();
    if !incoming.is_empty() {
        directional.extend(incoming.codec_schema_closure().iter().cloned());
    }
    for id in &directional {
        if let Some(raw) = contract.source(id) {
            for keyword in ["readOnly", "writeOnly"] {
                if raw
                    .get(keyword)
                    .is_some_and(|value| value != &serde_json::Value::Bool(false))
                {
                    errors.push(diagnostic(&contract, id.child(keyword), "http-directional-codec-unsupported", "the Go HTTP neutral codec profile requires directional annotations to be absent or false throughout its codec closure"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut codec_config = config.codecs.clone();
    codec_config.dialect = crate::schema_view::DialectPolicy::from_profiles(
        config.compatibility_profiles.iter().copied(),
    );
    let codecs = crate::go_codecs::plan_codecs(contract.clone(), &codec_roots, codec_config)
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
    let symbols: BTreeMap<SchemaId, String> = codecs
        .models()
        .symbols()
        .iter()
        .filter(|s| s.role() == crate::rust_models::RepresentationRole::Model)
        .map(|s| (s.source().clone(), s.name().to_owned()))
        .collect();
    let mut emitted: Vec<&'static str> = Vec::new();
    if oauth_emits {
        emitted.extend_from_slice(super::oauth::PACKAGE_NAMES);
        // The replaying credential wrapper joins only with an executable
        // client-credentials endpoint, mirroring its conditional emission.
        if oauth
            .as_ref()
            .is_some_and(super::oauth::has_client_credentials)
        {
            emitted.extend_from_slice(super::oauth::REPLAY_PACKAGE_NAMES);
        }
    }
    // The emitted incoming receipt helpers own fixed package-level names only
    // when receipts are declared; receipt-less plans reserve nothing.
    if !incoming.is_empty() {
        emitted.extend_from_slice(super::incoming::PACKAGE_NAMES);
    }
    let mut names = model_package_names(&contract, codecs.models(), &emitted)?;
    let mut methods = BTreeSet::from(["CloseIdleConnections".into()]);
    // The emitted OAuth lifecycle methods are fixed API, so source operations
    // that would collide allocate suffixed names instead.
    if oauth_emits {
        methods.extend(
            super::oauth::CLIENT_METHODS
                .iter()
                .map(|name| (*name).to_owned()),
        );
    }
    // Reserve all real operation methods before allocating convenience methods.
    let method_names = wire
        .operations()
        .iter()
        .map(|op| {
            let id = op.operation_id().map_or_else(
                || format!("{} {}", op.method().as_str(), op.path()),
                |v| v.value().clone(),
            );
            (id.clone(), allocate(&exported(&id), &mut methods))
        })
        .collect::<Vec<_>>();
    let mut operations = Vec::new();
    for (op, (operation_id, method_name)) in wire.operations().iter().zip(method_names) {
        let mut members = BTreeSet::from(["Body".into()]);
        let mut parameters = op
            .parameters()
            .iter()
            .map(|p| {
                let base = exported(p.name());
                let base = if members.contains(&base) {
                    format!("{}{base}", exported(&format!("{:?}", p.location())))
                } else {
                    base
                };
                PlannedParameter {
                    wire: p.clone(),
                    field_name: allocate(&base, &mut members),
                    setter_name: None,
                }
            })
            .collect::<Vec<_>>();
        for p in &mut parameters {
            if !p.wire.required() {
                p.setter_name = Some(allocate(&format!("With{}", p.field_name), &mut members));
            }
        }
        let input_type = allocate(&format!("{method_name}Input"), &mut names);
        let input_constructor = allocate(&format!("New{input_type}"), &mut names);
        let success_type = allocate(&format!("{method_name}Result"), &mut names);
        let error_variant = allocate(&format!("{method_name}ApiError"), &mut names);
        let body = op.body().map(|b| {
            let media = b
                .media()
                .iter()
                .map(|m| {
                    media(
                        m,
                        &format!("{method_name}Body"),
                        true,
                        b.media().len() > 1,
                        &symbols,
                        &mut names,
                    )
                })
                .collect::<Vec<_>>();
            let native_type = if media.len() > 1 {
                allocate(&format!("{method_name}Body"), &mut names)
            } else if media[0].requires_content_type() {
                format!("Content[{}]", media[0].native_type)
            } else {
                media[0].native_type.clone()
            };
            PlannedBody {
                wire: b.clone(),
                field_name: "Body".into(),
                setter_name: (!b.required()).then(|| allocate("WithBody", &mut members)),
                native_type,
                media,
            }
        });
        let mut responses = Vec::new();
        for (response_index, r) in op.responses().iter().enumerate() {
            let stem = format!("{method_name}Status{}", exported(r.status_key()));
            // Numeric status keys must preserve the original Status200 names.
            let stem = if matches!(
                r.status(),
                ResponseStatus::Exact(_) | ResponseStatus::Range(_)
            ) {
                format!("{method_name}Status{}", r.status_key())
            } else {
                stem
            };
            let mut fields = BTreeSet::new();
            let headers = r
                .headers()
                .iter()
                .map(|h| PlannedHeader {
                    field_name: allocate(&exported(h.name()), &mut fields),
                    wire: h.clone(),
                })
                .collect::<Vec<_>>();
            let headers_type = allocate(&format!("{stem}Headers"), &mut names);
            let forbidden = op.method().as_str() == "HEAD"
                || matches!(
                    r.status(),
                    ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1)
                );
            if forbidden || r.media().is_empty() {
                responses.push(PlannedResponse {
                    type_name: allocate(&stem, &mut names),
                    native_type: if forbidden { "NoContent" } else { "[]byte" }.into(),
                    headers_type,
                    headers,
                    response_index,
                    media_index: None,
                    forbidden_body: forbidden,
                    wire: r.clone(),
                    media: None,
                });
                continue;
            }
            for (media_index, m) in r.media().iter().enumerate() {
                let m = media(m, &stem, false, false, &symbols, &mut names);
                let type_name = allocate(
                    &if r.media().len() == 1 {
                        stem.clone()
                    } else {
                        format!("{stem}{}", media_suffix(m.wire.media_type()))
                    },
                    &mut names,
                );
                responses.push(PlannedResponse {
                    type_name,
                    native_type: m.native_type.clone(),
                    headers_type: headers_type.clone(),
                    headers: headers.clone(),
                    response_index,
                    media_index: Some(media_index),
                    forbidden_body: false,
                    wire: r.clone(),
                    media: Some(m),
                });
            }
            if matches!(
                r.status(),
                ResponseStatus::Range(1..=3) | ResponseStatus::Default
            ) {
                responses.push(PlannedResponse {
                    type_name: allocate(&format!("{stem}NoContent"), &mut names),
                    native_type: "NoContent".into(),
                    headers_type,
                    headers,
                    response_index,
                    media_index: None,
                    forbidden_body: true,
                    wire: r.clone(),
                    media: None,
                });
            }
        }
        let successful = responses
            .iter()
            .filter(|r| r.can_succeed())
            .collect::<Vec<_>>();
        let data_method = (successful.len() == 1 && !successful[0].can_fail())
            .then(|| allocate(&format!("{method_name}Data"), &mut methods));
        let optional_input = parameters.iter().all(|p| !p.wire.required())
            && body.as_ref().is_none_or(|b| !b.wire.required());
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id,
            method_name,
            input_type,
            input_constructor,
            success_type,
            error_variant,
            data_method,
            optional_input,
            wire: op.clone(),
            parameters,
            body,
            responses,
        });
    }
    let credentials = wire
        .operations()
        .iter()
        .flat_map(|op| op.security().alternatives())
        .flat_map(|a| a.requirements())
        .map(|r| r.name().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|scheme| {
            let name = allocate(&exported(&scheme), &mut names);
            (scheme, name)
        })
        .collect();
    let credential_env_factory = credential_env
        .as_ref()
        .map(|_| allocate("NewClientFromEnv", &mut names));
    // Emission-ready incoming receipt helpers. Receipts the Go v1 helpers
    // cannot express surface as the shared plan errors; allocated receipt
    // names join the model namespace so nothing else can take them.
    let mut incoming_errors = Vec::new();
    let incoming_receipts = incoming::prepare(
        &contract,
        &incoming,
        &symbols,
        &mut names,
        &mut incoming_errors,
    );
    if !incoming_errors.is_empty() {
        return Err(incoming_errors);
    }
    let pagination = super::pagination::plan(
        &contract,
        &wire,
        config.sdk_defaults.as_ref(),
        &operations,
        &symbols,
        &codecs,
        &mut names,
        &mut methods,
    )?;
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Emission — and the names it
    // reserves — stay conditional on a discriminated SSE event set, so plans
    // without one keep every artifact byte-identical.
    let stream_semantics = protocol::plan_stream_semantics(&contract, &wire);
    let stream_events = super::stream_events::plan(
        &stream_semantics,
        &operations,
        &symbols,
        codecs.models(),
        &mut names,
        &mut methods,
    );
    let examples =
        if codecs.validation_program().version == suspect_schema::OwnedProgram::V3_VERSION {
            crate::examples::plan_protocol_examples_v3(contract.clone(), &wire, Default::default())
        } else if codecs.validation_program().version == suspect_schema::OwnedProgram::V2_VERSION {
            crate::examples::plan_protocol_examples_v2(contract.clone(), &wire, Default::default())
        } else {
            crate::examples::plan_protocol_examples(contract.clone(), &wire, Default::default())
        };
    Ok(HttpPlan {
        contract,
        operations,
        codecs,
        symbols,
        credentials,
        config,
        examples,
        protocol: wire,
        credential_env,
        credential_env_factory,
        pagination,
        oauth,
        stream_events,
        incoming,
        incoming_receipts,
    })
}

pub(super) fn media_suffix(m: &protocol::MediaType) -> String {
    match m.range() {
        protocol::MediaRange::Any => "AnyMedia".into(),
        protocol::MediaRange::Type { type_name } => format!("{}Media", exported(type_name)),
        _ => exported(m.declared()),
    }
}

fn media(
    wire: &protocol::MediaPlan,
    stem: &str,
    request: bool,
    choice: bool,
    symbols: &BTreeMap<SchemaId, String>,
    names: &mut BTreeSet<String>,
) -> PlannedMedia {
    let suffix = media_suffix(wire.media_type());
    let aggregate = match wire.representation() {
        Representation::Form { form } => Some(aggregate(
            &format!("{stem}{suffix}Fields"),
            false,
            false,
            form.fields(),
            form.additional(),
            symbols,
            names,
        )),
        Representation::Multipart {
            multipart: MultipartPlan::Named {
                parts, additional, ..
            },
        } => Some(aggregate(
            &format!("{stem}{suffix}Parts"),
            true,
            false,
            parts,
            additional,
            symbols,
            names,
        )),
        Representation::Multipart {
            multipart: MultipartPlan::Positional { prefix, items, .. },
        } => Some(aggregate(
            &format!("{stem}{suffix}Parts"),
            true,
            true,
            prefix,
            items,
            symbols,
            names,
        )),
        _ => None,
    };
    let native_type = match wire.representation() {
        Representation::Json { codec } => codec
            .as_ref()
            .map_or_else(|| "Value".into(), |c| symbols[c.schema().id()].clone()),
        Representation::Text { codec, .. } => codec
            .as_ref()
            .map_or_else(|| "string".into(), |c| symbols[c.schema().id()].clone()),
        Representation::Binary { .. } => "[]byte".into(),
        Representation::Stream { stream } => {
            let item = stream.item_codec().map_or_else(
                || "Value".to_owned(),
                |codec| symbols[codec.schema().id()].clone(),
            );
            if request {
                format!("[]{item}")
            } else {
                format!("*Stream[{item}]")
            }
        }
        _ => aggregate.as_ref().unwrap().type_name.clone(),
    };
    let choice_type = if choice {
        allocate(&format!("{stem}{suffix}"), names)
    } else {
        String::new()
    };
    let constructor = if choice {
        allocate(&format!("New{choice_type}"), names)
    } else {
        String::new()
    };
    PlannedMedia {
        wire: wire.clone(),
        native_type,
        choice_type,
        constructor,
        aggregate,
    }
}

fn aggregate(
    stem: &str,
    multipart: bool,
    positional: bool,
    parts: &[protocol::PartPlan],
    additional: &AdditionalParts,
    symbols: &BTreeMap<SchemaId, String>,
    names: &mut BTreeSet<String>,
) -> PlannedAggregate {
    let type_name = allocate(stem, names);
    let constructor = allocate(&format!("New{type_name}"), names);
    let mut fields = BTreeSet::from([if positional { "Items" } else { "Extras" }.into()]);
    let mut parts = parts
        .iter()
        .enumerate()
        .map(|(index, p)| {
            part(
                p,
                &p.name().map_or_else(|| format!("Item{index}"), exported),
                multipart,
                symbols,
                &mut fields,
            )
        })
        .collect::<Vec<_>>();
    for p in &mut parts {
        if !p.wire.required() {
            p.setter_name = Some(allocate(&format!("With{}", p.field_name), &mut fields));
        }
    }
    let additional = match additional {
        AdditionalParts::Allowed(p) => {
            let mut part = part(
                p,
                if positional { "Items" } else { "Extras" },
                multipart,
                symbols,
                &mut BTreeSet::new(),
            );
            part.native_type = format!(
                "{}{}",
                if positional { "[]" } else { "map[string]" },
                part.native_type
            );
            Some(part)
        }
        AdditionalParts::Forbidden => None,
    };
    PlannedAggregate {
        type_name,
        constructor,
        multipart,
        positional,
        parts,
        additional,
    }
}
fn part(
    wire: &protocol::PartPlan,
    field: &str,
    multipart: bool,
    symbols: &BTreeMap<SchemaId, String>,
    fields: &mut BTreeSet<String>,
) -> PlannedPart {
    let data_type = match wire.representation() {
        PartRepresentation::Binary { .. } => "[]byte".into(),
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => symbols[codec.schema().id()].clone(),
    };
    let mut native_type = if multipart {
        format!("Part[{data_type}]")
    } else {
        data_type.clone()
    };
    if wire.multiplicity() == protocol::PartMultiplicity::RepeatedArrayItems {
        native_type = format!("[]{native_type}");
    }
    PlannedPart {
        field_name: allocate(field, fields),
        setter_name: None,
        data_type,
        native_type,
        wire: wire.clone(),
    }
}
