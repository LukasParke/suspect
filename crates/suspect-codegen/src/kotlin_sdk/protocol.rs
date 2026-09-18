use super::*;
use crate::{
    examples::{
        ExampleConfig, plan_protocol_examples, plan_protocol_examples_v2, plan_protocol_examples_v3,
    },
    http_protocol as w,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaDialect, SourceId};
use suspect_schema::Config;

pub(super) fn capabilities() -> w::Capabilities {
    use w::Capability::*;
    w::Capabilities::for_adapter(
        "kotlin-jvm-protocol-v1",
        [
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
    .with_limits(w::ByteLimits::new(
        4 * 1024 * 1024,
        4 * 1024 * 1024,
        1024 * 1024,
    ))
}

pub(super) fn plan(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    profiles: &BTreeSet<w::CompatibilityProfile>,
    scoped: bool,
    resources: bool,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    let root = SourceId::new(contract.entry().clone(), Default::default());
    let error = |source: SourceId, message: &str| {
        vec![diagnostic(
            &contract,
            source,
            "kotlin-native-protocol-unsupported",
            message,
        )]
    };
    if selected.is_empty() {
        return Err(error(root, "select at least one outgoing operation"));
    }
    if !valid_package(&config.package_name)
        || !valid_coordinate(&config.group_id)
        || !valid_coordinate(&config.artifact_id)
        || semver::Version::parse(&config.version).is_err()
    {
        return Err(vec![diagnostic(
            &contract,
            root,
            "kotlin-package-identity",
            "expected Maven coordinates, an exact version and a legal Kotlin package",
        )]);
    }
    let mut capabilities = profiles
        .iter()
        .fold(capabilities(), |caps, profile| caps.with_profile(*profile));
    if resources {
        capabilities = capabilities
            .with(w::Capability::SchemaResources)
            .with(w::Capability::DynamicSchemaReferences);
    }
    let protocol = w::plan(&contract, selected, capabilities)
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .filter(|e| e.severity() == w::Severity::Error)
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
    let pagination_outcome = match config.sdk_defaults.as_ref() {
        Some(defaults) => Some(w::plan_pagination(&contract, &protocol, Some(defaults))?),
        None => None,
    };
    let oauth_outcome = match config.sdk_defaults.as_ref() {
        Some(defaults) => Some(w::plan_oauth(&contract, &protocol, Some(defaults))?),
        None => None,
    };
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Only the lowered emission is
    // conditional on the compiled discrimination evidence.
    let stream_semantics = w::plan_stream_semantics(&contract, &protocol);
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming_plan = w::plan_incoming(&contract)?;
    // Emission is conditional: `OAuth.kt` joins the package exactly when a
    // usable scheme exists, and the identifiers it declares are reserved
    // against model and method allocation in that case only, so no-policy
    // output stays byte-identical.
    let oauth_emits = oauth_outcome.as_ref().is_some_and(super::oauth::emittable);
    let mut roots = protocol.codec_schema_closure().to_vec();
    if !incoming_plan.is_empty() {
        // Incoming receipts extend the codec table only when declared: their
        // body schemas become actual codec inputs beside the selected
        // operations'. The effective closure is recomputed over the merged
        // roots once, so the resource gate, the compiled program and the
        // native models all agree on exactly the same schema closure.
        let mut codec_roots = protocol.codec_roots().to_vec();
        codec_roots.extend(incoming_plan.codec_roots().iter().cloned());
        codec_roots.sort();
        codec_roots.dedup();
        roots = contract.effective_schema_closure(&codec_roots);
    }
    if let Some(id) = roots.iter().find(|id| {
        contract
            .schema(id)
            .is_some_and(|s| matches!(s.dialect(), SchemaDialect::OpenApi30))
    }) {
        return Err(error(
            id.clone(),
            "OpenAPI 3.0 native schema views are not enabled",
        ));
    }
    let needs_resources = resources
        || roots.iter().any(|id| {
            contract.schema(id).is_some_and(|schema| {
                !schema.ignores_ref_siblings()
                    && (["$id", "$anchor", "$dynamicAnchor", "$dynamicRef"]
                        .iter()
                        .any(|key| schema.raw().get(*key).is_some())
                        || contract
                            .resource_scope(id)
                            .is_some_and(|scope| scope.base_source().is_some()))
            })
        });
    let compile = if needs_resources {
        validation::plan_validation_v3
    } else if scoped {
        validation::plan_validation_v2
    } else {
        validation::plan_validation
    };
    let program = compile(
        contract.clone(),
        &roots,
        Config {
            max_depth: 128,
            ..Default::default()
        },
    )?;
    if protocol.operations().len() > 256 {
        return Err(error(root, "at most 256 native operations are admitted"));
    }
    let mut used = models::reserved_types();
    if credential_env.is_some() {
        used.extend(
            [
                "CredentialEnvironment",
                "EnvironmentSnapshot",
                "EnvironmentCredentials",
                "CredentialEnvironmentKt",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    used.extend(
        [
            "BasicCredentials",
            "CredentialProvider",
            "CredentialContext",
            "CredentialMetadata",
            "Upload",
            "StreamingTransport",
            "StreamingResponse",
            "BodyReader",
            "ProtocolRuntime",
            "ProtocolData",
            "PreparedInput",
            "PreparedBody",
            "PreparedPart",
            "LinkMetadata",
            "ProtocolResponse",
            "ProtocolLease",
            "JdkStreaming",
            "HeaderSnapshot",
            "ProtocolCredential",
            "ProtocolValue",
            "ProtocolPartValue",
            "PaginationStalled",
            "ProtocolBudget",
            "ProtocolMedia",
            "ProtocolKt",
            "PayloadKt",
            "StreamingKt",
            "DocumentServersKt",
            "UUID",
            "Base64",
            "Executors",
            "InputStream",
            "ConcurrentHashMap",
            "AtomicBoolean",
            "Comparator",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    if oauth_emits {
        used.extend(
            super::oauth::reserved_types(
                oauth_outcome
                    .as_ref()
                    .is_some_and(super::oauth::has_discovery_among),
                oauth_outcome
                    .as_ref()
                    .is_some_and(super::oauth::has_client_credentials),
            )
            .into_iter()
            .map(|name| name.to_owned()),
        );
    }
    let mut names = BTreeSet::new();
    let mut credentials = BTreeMap::new();
    for op in protocol.operations() {
        native_guards(&contract, op)?;
        for alternative in op.security().alternatives() {
            for c in alternative.requirements() {
                let source = c.scheme().terminal().source().clone();
                credentials
                    .entry(source.clone())
                    .or_insert_with(|| PlannedCredential {
                        source,
                        scheme_name: c.name().into(),
                        name: models::allocate(&models::member_name(c.name()), &mut names),
                        kotlin_type: match c.credential() {
                            w::CredentialHook::Basic => "BasicCredentials?",
                            w::CredentialHook::OAuth2 { .. }
                            | w::CredentialHook::OpenIdConnect { .. } => "CredentialProvider?",
                            _ => "String?",
                        }
                        .into(),
                        wire: c.clone(),
                    });
            }
        }
    }
    if credentials.len() > 100 {
        return Err(error(
            root,
            "at most 100 source credential constructor fields are admitted",
        ));
    }
    let mut methods: BTreeSet<String> = [
        "close",
        "toString",
        "hashCode",
        "equals",
        "credentials",
        "transport",
        "options",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if credential_env.is_some() {
        methods.insert("fromEnv".into());
    }
    if oauth_emits {
        methods.insert(super::oauth::RESERVED_MEMBER.to_owned());
    }
    let mut operations = Vec::new();
    for op in protocol.operations() {
        let source = op.source().terminal().source().clone();
        if op.method().as_str() == "CONNECT" {
            return Err(error(
                source,
                "CONNECT requires a separate tunnel transport profile",
            ));
        }
        if !valid_path(op.path()) {
            return Err(vec![diagnostic(
                &contract,
                source,
                "kotlin-path-unsupported",
                "path requires ASCII URI syntax, valid escapes and no literal dot segments",
            )]);
        }
        let id = op
            .operation_id()
            .map(|v| v.value().clone())
            .ok_or_else(|| error(source.clone(), "operationId is required for native names"))?;
        let stem = models::type_name(&id);
        let input_type = models::allocate(&format!("{stem}Input"), &mut used);
        let result_type = models::allocate(&format!("{stem}Result"), &mut used);
        let error_type = models::allocate(&format!("{stem}ApiException"), &mut used);
        let mut members = BTreeSet::from(["body".into()]);
        let parameters = op
            .parameters()
            .iter()
            .map(|p| PlannedParameter {
                source: p.source().use_site().source().clone(),
                schema: p.codec().schema().id().clone(),
                wire_name: p.name().into(),
                name: models::allocate(&models::member_name(p.name()), &mut members),
                kotlin_type: String::new(),
                codec_name: String::new(),
                required: p.required(),
                wire: p.clone(),
            })
            .collect();
        let body = op
            .body()
            .map(|body| -> Result<PlannedBody, Vec<HttpDiagnostic>> {
                let mut media = Vec::new();
                let mut media_names = BTreeSet::new();
                for (i, m) in body.media().iter().enumerate() {
                    let mut native =
                        media_plan(&contract, m, &format!("{stem}Request"), i, &mut used)?;
                    if matches!(m.representation(), w::Representation::Stream { .. }) {
                        native.ty = NativeType::Flow(Box::new(native.ty));
                    }
                    native.name = models::allocate(&native.name, &mut media_names);
                    media.push(native);
                }
                let choice_type = (media.len() > 1)
                    .then(|| models::allocate(&format!("{stem}RequestBody"), &mut used));
                let ty = choice_type.as_ref().map_or_else(
                    || media[0].ty.clone(),
                    |name| NativeType::Named(name.clone()),
                );
                Ok(PlannedBody {
                    source: body.source().use_site().source().clone(),
                    name: "body".into(),
                    ty,
                    kotlin_type: String::new(),
                    required: body.required(),
                    media,
                    choice_type,
                })
            })
            .transpose()?;
        let mut responses = Vec::new();
        let mut result_cases = BTreeSet::new();
        let mut error_cases = BTreeSet::new();
        for (ri, r) in op.responses().iter().enumerate() {
            let key = match r.status() {
                w::ResponseStatus::Exact(n) => format!("Status{n}"),
                w::ResponseStatus::Range(n) => format!("Range{n}XX"),
                w::ResponseStatus::Default => "Default".into(),
            };
            let (headers_type, headers) =
                headers(&format!("{stem}{key}Headers"), r.headers(), &mut used);
            let always_none = op.method() == w::Method::Head
                || matches!(
                    r.status(),
                    w::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                        | w::ResponseStatus::Range(1)
                );
            let possible_none = always_none
                || matches!(
                    r.status(),
                    w::ResponseStatus::Range(2 | 3) | w::ResponseStatus::Default
                );
            let groups = match r.status() {
                w::ResponseStatus::Exact(n) => vec![(200..300).contains(&n)],
                w::ResponseStatus::Range(n) => vec![n == 2],
                w::ResponseStatus::Default => vec![true, false],
            };
            let media = if always_none {
                Vec::new()
            } else {
                r.media()
                    .iter()
                    .enumerate()
                    .map(|(mi, m)| media_plan(&contract, m, &format!("{stem}{key}"), mi, &mut used))
                    .collect::<Result<Vec<_>, _>>()?
            };
            for success in groups {
                let mut variants = Vec::new();
                if !always_none {
                    if r.media().is_empty() {
                        variants.push((
                            key.clone(),
                            NativeType::Bytes,
                            None,
                            None,
                            w::ResponseBodyDisposition::UndeclaredBoundedBytes,
                        ));
                    } else {
                        for (mi, media) in media.iter().enumerate() {
                            let label = if r.media().len() == 1 {
                                key.clone()
                            } else {
                                format!("{key}{}", media.name)
                            };
                            variants.push((
                                label,
                                media.ty.clone(),
                                Some(media.clone()),
                                Some(mi),
                                w::ResponseBodyDisposition::Declared,
                            ));
                        }
                    }
                }
                if possible_none
                    && (always_none
                        || success
                        || matches!(
                            r.status(),
                            w::ResponseStatus::Range(3) | w::ResponseStatus::Default
                        ))
                {
                    variants.push((
                        if always_none {
                            key.clone()
                        } else {
                            format!("{key}NoContent")
                        },
                        NativeType::Unit,
                        None,
                        None,
                        w::ResponseBodyDisposition::ForbiddenByHttp,
                    ));
                }
                for (name, ty, media, mi, disposition) in variants {
                    let variant_name = models::allocate(
                        &name,
                        if success {
                            &mut result_cases
                        } else {
                            &mut error_cases
                        },
                    );
                    let stream = media.as_ref().is_some_and(|m| {
                        matches!(m.wire.representation(), w::Representation::Stream { .. })
                    });
                    responses.push(PlannedResponse {
                        source: r.source().use_site().source().clone(),
                        status: r.status(),
                        status_key: r.status_key().into(),
                        success,
                        constructor: format!(
                            "{}.{variant_name}",
                            if success { &result_type } else { &error_type }
                        ),
                        variant_name,
                        ty,
                        kotlin_type: String::new(),
                        schema: media.as_ref().and_then(|m| m.schema.clone()),
                        codec_name: None,
                        media,
                        headers_type: headers_type.clone(),
                        headers: headers.clone(),
                        disposition,
                        stream,
                        response_index: ri,
                        media_index: mi,
                    });
                }
            }
        }
        let flow = responses.iter().any(|r| r.stream);
        let selected_credentials = op
            .security()
            .alternatives()
            .iter()
            .flat_map(|a| a.requirements())
            .map(|c| credentials[c.scheme().terminal().source()].clone())
            .collect();
        operations.push(PlannedOperation {
            source,
            operation_id: id.clone(),
            method_name: models::allocate(&models::member_name(&id), &mut methods),
            constructor: input_type.clone(),
            input_type,
            result_type,
            result_data_type: None,
            result_data: None,
            error_type,
            parameters,
            body,
            responses,
            flow,
            credentials: selected_credentials,
            wire: op.clone(),
        });
    }
    // Typed-stream public type names join the reservation set before model
    // planning, so model symbols can never take them; the emission itself is
    // lowered after model planning binds the item codecs.
    let mut stream_events =
        super::stream_events::reserve(&operations, &stream_semantics, &mut used, &mut methods);
    // Incoming receipt public names join the reservation set before model
    // planning, so model symbols can never take them; the emission itself is
    // lowered after model planning binds the codecs. Unrepresentable receipts
    // refuse the plan here, exactly like the shared planner's own refusals.
    let mut incoming_entries = if incoming_plan.is_empty() {
        Vec::new()
    } else {
        super::incoming::reserve(&contract, &incoming_plan, &mut used)?
    };
    let models = models::plan(&contract, &roots, &program, used)?;
    for op in &mut operations {
        for p in &mut op.parameters {
            let s = models.get(&p.schema);
            p.kotlin_type = if p.required {
                s.kotlin_type.clone()
            } else {
                format!("Presence<{}>", s.kotlin_type)
            };
            p.codec_name = s.codec_name.clone();
        }
        if let Some(body) = &mut op.body {
            for media in &mut body.media {
                bind_media(media, &models);
            }
            body.kotlin_type = if body.required {
                body.ty.kotlin(&models)
            } else {
                format!("Presence<{}>", body.ty.kotlin(&models))
            };
        }
        for r in &mut op.responses {
            if let Some(media) = &mut r.media {
                bind_media(media, &models);
                r.codec_name = media.codec_name.clone();
            }
            bind_headers(&mut r.headers, &models);
            r.kotlin_type = r.ty.kotlin(&models);
        }
        let types: BTreeSet<_> = op
            .responses
            .iter()
            .filter(|r| r.success)
            .map(|r| r.kotlin_type.clone())
            .collect();
        if types.len() == 1 {
            op.result_data_type = types.into_iter().next();
            op.result_data = op
                .responses
                .iter()
                .find(|r| r.success)
                .map(|r| r.ty.clone());
        }
    }
    let pagination = pagination_outcome
        .map(|outcome| super::pagination::lower(&models, &outcome, &operations, &mut methods));
    // `Some` exactly when `OAuth.kt` will be emitted.
    let oauth = oauth_outcome.filter(super::oauth::emittable);
    super::stream_events::bind(&mut stream_events, &models);
    super::incoming::bind(&mut incoming_entries, &models);
    let examples = if program.version == OwnedProgram::V3_VERSION {
        plan_protocol_examples_v3(contract.clone(), &protocol, ExampleConfig::default())
    } else if program.version == OwnedProgram::V2_VERSION {
        plan_protocol_examples_v2(contract.clone(), &protocol, ExampleConfig::default())
    } else {
        plan_protocol_examples(contract.clone(), &protocol, ExampleConfig::default())
    };
    Ok(Plan {
        contract,
        config,
        models,
        operations,
        credentials,
        program,
        protocol,
        examples,
        credential_env,
        pagination,
        oauth,
        stream_events: super::stream_events::StreamEventsPlan {
            semantics: stream_semantics,
            operations: stream_events,
        },
        incoming: incoming_plan,
        incoming_entries,
    })
}

fn native_guards(contract: &Contract, op: &w::OperationPlan) -> Result<(), Vec<HttpDiagnostic>> {
    if op.parameters().len() > 100 {
        return Err(vec![diagnostic(
            contract,
            op.source().use_site().source().clone(),
            "kotlin-input-size",
            "at most 100 source parameters are admitted per native constructor",
        )]);
    }
    let restricted = |name: &str| {
        ["host", "content-length", "connection", "expect", "upgrade"]
            .contains(&name.to_ascii_lowercase().as_str())
    };
    for p in op.parameters() {
        if p.location() == w::ParameterLocation::Header && restricted(p.name()) {
            return Err(vec![diagnostic(
                contract,
                p.source().use_site().source().clone(),
                "kotlin-jdk-header-unsupported",
                "the JDK transport does not expose this restricted request header",
            )]);
        }
    }
    for alternative in op.security().alternatives() {
        for c in alternative.requirements() {
            if let w::CredentialHook::ApiKey {
                location: w::ParameterLocation::Header,
                name,
            } = c.credential()
                && restricted(name.value())
            {
                return Err(vec![diagnostic(
                    contract,
                    c.source().source().clone(),
                    "kotlin-jdk-header-unsupported",
                    "the JDK transport cannot attach an API key through this restricted header",
                )]);
            }
        }
    }
    for response in op.responses() {
        if response.headers().len() > 100 {
            return Err(vec![diagnostic(
                contract,
                response.source().use_site().source().clone(),
                "kotlin-header-size",
                "at most 100 native response header fields are admitted",
            )]);
        }
        for h in response.headers() {
            if matches!(
                h.serialization(),
                w::ParameterSerialization::Style {
                    shape: w::WireShape::FlatObject {
                        additional: w::AdditionalScalars::AnyScalar,
                        ..
                    },
                    ..
                }
            ) {
                return Err(vec![diagnostic(
                    contract,
                    h.source().use_site().source().clone(),
                    "kotlin-header-extra-type-unsupported",
                    "response header extras need a declared scalar type for faithful decoding",
                )]);
            }
        }
        for m in response.media() {
            if let w::Representation::Form { form } = m.representation() {
                for p in form.fields().iter().chain(match form.additional() {
                    w::AdditionalParts::Forbidden => None,
                    w::AdditionalParts::Allowed(p) => Some(p.as_ref()),
                }) {
                    if matches!(
                        p.representation(),
                        w::PartRepresentation::Style {
                            serialization: w::ParameterSerialization::Style {
                                shape: w::WireShape::Array { .. } | w::WireShape::FlatObject { .. },
                                ..
                            },
                            ..
                        }
                    ) {
                        return Err(vec![diagnostic(
                            contract,
                            p.source().use_site().source().clone(),
                            "kotlin-response-form-style-unsupported",
                            "response form composites need JSON part content or a separately specified grouping profile",
                        )]);
                    }
                }
            }
        }
    }
    Ok(())
}

fn headers(
    stem: &str,
    fields: &[w::HeaderPlan],
    used: &mut BTreeSet<String>,
) -> (Option<String>, Vec<PlannedHeader>) {
    if fields.is_empty() {
        return (None, Vec::new());
    }
    let name = models::allocate(stem, used);
    let mut names = BTreeSet::new();
    (
        Some(name),
        fields
            .iter()
            .map(|h| PlannedHeader {
                name: models::allocate(&models::member_name(h.name()), &mut names),
                ty: NativeType::Model(h.codec().schema().id().clone()),
                codec_name: String::new(),
                wire: h.clone(),
            })
            .collect(),
    )
}
fn bind_headers(headers: &mut [PlannedHeader], models: &models::ModelPlan) {
    for h in headers {
        h.codec_name = models.get(h.wire.codec().schema().id()).codec_name.clone();
    }
}
fn part(
    contract: &Contract,
    p: &w::PartPlan,
    stem: &str,
    used: &mut BTreeSet<String>,
    names: &mut BTreeSet<String>,
) -> Result<PlannedPart, Vec<HttpDiagnostic>> {
    if p.headers().len() > 100 {
        return Err(vec![diagnostic(
            contract,
            p.source().use_site().source().clone(),
            "kotlin-part-header-size",
            "at most 100 native part header fields are admitted",
        )]);
    }
    let name = models::allocate(
        &models::member_name(p.name().unwrap_or("additionalProperties")),
        names,
    );
    let value_type = match p.representation() {
        w::PartRepresentation::Json { codec, .. }
        | w::PartRepresentation::Text { codec, .. }
        | w::PartRepresentation::Style { codec, .. } => {
            NativeType::Model(codec.schema().id().clone())
        }
        w::PartRepresentation::Binary { .. } => NativeType::Named("Upload".into()),
    };
    let (headers_type, headers) = headers(
        &format!("{stem}{}Headers", models::type_name(&name)),
        p.headers(),
        used,
    );
    let choice = p.content_types().len() != 1
        || p.content_types()
            .iter()
            .any(|m| !matches!(m.range(), w::MediaRange::Concrete { .. }));
    let wrapper = (headers_type.is_some()
        || choice && !matches!(&value_type, NativeType::Named(name) if name == "Upload"))
    .then(|| models::allocate(&format!("{stem}{}Part", models::type_name(&name)), used));
    let mut ty = wrapper.as_ref().map_or_else(
        || value_type.clone(),
        |name| NativeType::Named(name.clone()),
    );
    if p.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
        ty = NativeType::List(Box::new(ty));
    }
    Ok(PlannedPart {
        name,
        ty,
        value_type,
        wrapper,
        headers_type,
        headers,
        wire: p.clone(),
    })
}
fn media_plan(
    contract: &Contract,
    m: &w::MediaPlan,
    stem: &str,
    index: usize,
    used: &mut BTreeSet<String>,
) -> Result<PlannedMedia, Vec<HttpDiagnostic>> {
    let mut form = None;
    let mut schema = None;
    let (label, ty) = match m.representation() {
        w::Representation::Json { codec } => {
            schema = codec.as_ref().map(|c| c.schema().id().clone());
            (
                "Json",
                schema.clone().map_or(NativeType::Json, NativeType::Model),
            )
        }
        w::Representation::Text { codec, scalar, .. } => {
            schema = codec.as_ref().map(|c| c.schema().id().clone());
            (
                "Text",
                schema.clone().map_or_else(
                    || match scalar {
                        w::ScalarType::String => NativeType::String,
                        w::ScalarType::Boolean => NativeType::Boolean,
                        _ => NativeType::Number,
                    },
                    NativeType::Model,
                ),
            )
        }
        w::Representation::Binary { .. } => ("Bytes", NativeType::Bytes),
        w::Representation::Stream { stream } => {
            schema = stream.item_codec().map(|codec| codec.schema().id().clone());
            let native = match schema.clone() {
                Some(model) => NativeType::Model(model),
                // A schemaless stream surfaces untyped parsed envelope values.
                None => NativeType::Json,
            };
            ("Items", native)
        }
        w::Representation::Form { form: f } => {
            if f.fields().len() > 100 {
                return Err(vec![diagnostic(
                    contract,
                    m.source().use_site().source().clone(),
                    "kotlin-form-size",
                    "at most 100 declared native form fields are admitted",
                )]);
            }
            let name = models::allocate(
                &format!(
                    "{stem}FormBody{}",
                    if index == 0 {
                        String::new()
                    } else {
                        (index + 1).to_string()
                    }
                ),
                used,
            );
            let mut names = BTreeSet::from(["additionalProperties".into()]);
            let mut fields = Vec::new();
            for p in f.fields() {
                fields.push(part(contract, p, &name, used, &mut names)?);
            }
            let additional = match f.additional() {
                w::AdditionalParts::Forbidden => None,
                w::AdditionalParts::Allowed(p) => Some(Box::new(part(
                    contract,
                    p,
                    &name,
                    used,
                    &mut BTreeSet::new(),
                )?)),
            };
            form = Some(PlannedForm {
                name: name.clone(),
                source: f.rules().schema().id().clone(),
                multipart: false,
                rules: f.rules().clone(),
                fields,
                additional,
            });
            ("Form", NativeType::Named(name))
        }
        w::Representation::Multipart { multipart } => {
            let w::MultipartPlan::Named {
                rules,
                parts,
                additional,
            } = multipart
            else {
                return Err(vec![diagnostic(
                    contract,
                    m.source().use_site().source().clone(),
                    "kotlin-positional-multipart-unsupported",
                    "positional multipart needs its own native profile",
                )]);
            };
            if parts.len() > 100 {
                return Err(vec![diagnostic(
                    contract,
                    m.source().use_site().source().clone(),
                    "kotlin-multipart-size",
                    "at most 100 declared native multipart fields are admitted",
                )]);
            }
            let name = models::allocate(
                &format!(
                    "{stem}MultipartBody{}",
                    if index == 0 {
                        String::new()
                    } else {
                        (index + 1).to_string()
                    }
                ),
                used,
            );
            let mut names = BTreeSet::from(["additionalProperties".into()]);
            let mut fields = Vec::new();
            for p in parts {
                fields.push(part(contract, p, &name, used, &mut names)?);
            }
            let additional = match additional {
                w::AdditionalParts::Forbidden => None,
                w::AdditionalParts::Allowed(p) => Some(Box::new(part(
                    contract,
                    p,
                    &name,
                    used,
                    &mut BTreeSet::new(),
                )?)),
            };
            form = Some(PlannedForm {
                name: name.clone(),
                source: rules.schema().id().clone(),
                multipart: true,
                rules: rules.clone(),
                fields,
                additional,
            });
            ("Multipart", NativeType::Named(name))
        }
    };
    Ok(PlannedMedia {
        name: label.into(),
        ty,
        schema,
        codec_name: None,
        form,
        wire: m.clone(),
    })
}
fn bind_media(m: &mut PlannedMedia, models: &models::ModelPlan) {
    m.codec_name = m
        .schema
        .as_ref()
        .map(|id| models.get(id).codec_name.clone());
    if let Some(form) = &mut m.form {
        for p in form
            .fields
            .iter_mut()
            .chain(form.additional.iter_mut().map(|p| p.as_mut()))
        {
            bind_headers(&mut p.headers, models);
        }
    }
}
fn valid_path(path: &str) -> bool {
    if !path.starts_with('/') || path.starts_with("//") {
        return false;
    }
    let mut placeholder = false;
    let mut literal = String::new();
    for c in path.chars() {
        if c == '{' {
            if placeholder {
                return false;
            }
            placeholder = true;
            literal.push('x');
        } else if c == '}' {
            if !placeholder {
                return false;
            }
            placeholder = false;
        } else if !placeholder {
            if !c.is_ascii_alphanumeric() && !"-._~!$&'()*+,;=:@/%".contains(c) {
                return false;
            }
            literal.push(c);
        }
    }
    if placeholder {
        return false;
    }
    let b = literal.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return false;
            }
            let Some(a) = char::from(b[i + 1]).to_digit(16) else {
                return false;
            };
            let Some(c) = char::from(b[i + 2]).to_digit(16) else {
                return false;
            };
            decoded.push((a * 16 + c) as u8);
            i += 3;
        } else {
            decoded.push(b[i]);
            i += 1;
        }
    }
    !decoded
        .split(|b| *b == b'/')
        .any(|v| v == b"." || v == b"..")
        && !decoded.iter().any(|b| b.is_ascii_control() || *b == b'\\')
}
