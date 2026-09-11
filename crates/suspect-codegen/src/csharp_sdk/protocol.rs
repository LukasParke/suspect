//! C# native projection of the shared protocol. Only codec_roots enter models.
use super::{
    HttpDiagnostic, PlannedBody, PlannedOperation, PlannedParameter, PlannedResponse, SdkConfig,
    SdkPlan, allocate, diagnostic, exported, models,
};
use crate::{examples, http_protocol as p};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::{OwnedCompiler, OwnedOutcome};

/// Explicit interpretation profiles and runtime client-default policies.
#[derive(Debug, Clone, Default)]
pub struct ProtocolOptions {
    pub compatibility_profiles: Vec<p::CompatibilityProfile>,
    /// Source scheme names mapped to runtime environment-variable names only.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
}

#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub key: String,
    pub property_name: String,
    pub native_type: String,
    pub requirement: p::CredentialRequirement,
}
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub property_name: String,
    pub native_type: String,
    pub wire: p::HeaderPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedPart {
    pub property_name: String,
    pub native_type: String,
    pub value_type: String,
    pub wrapper_type: Option<String>,
    pub header_type: Option<String>,
    pub headers: Vec<PlannedHeader>,
    pub wire: p::PartPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedParts {
    pub native_type: String,
    pub multipart: bool,
    pub rules: p::ObjectRules,
    pub fields: Vec<PlannedPart>,
    pub additional: Option<Box<PlannedPart>>,
}
/// Ordered, finite multipart values. The shared plan truncates a false-prefix
/// barrier and forbids the tail; no unreachable native slot is invented.
#[derive(Debug, Clone)]
pub struct PlannedPositionalParts {
    pub native_type: String,
    pub schema: p::SchemaUse,
    pub prefix: Vec<PlannedPart>,
    pub items: Option<Box<PlannedPart>>,
    pub min_items: Option<p::Located<u64>>,
    pub max_items: Option<p::Located<u64>>,
}
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub variant_name: String,
    pub native_type: String,
    pub wire: p::MediaPlan,
    pub parts: Option<PlannedParts>,
    pub positional: Option<PlannedPositionalParts>,
}
impl PlannedMedia {
    pub fn schema(&self) -> Option<&SchemaId> {
        match self.wire.representation() {
            p::Representation::Json { codec } | p::Representation::Text { codec, .. } => {
                codec.as_ref().map(|c| c.schema().id())
            }
            p::Representation::Stream { stream } => Some(stream.item_codec().schema().id()),
            _ => None,
        }
    }
    pub fn is_stream(&self) -> bool {
        matches!(self.wire.representation(), p::Representation::Stream { .. })
    }
}
/// Explicitly implemented native wire vocabulary; no wildcard capability opt-in.
pub fn capabilities() -> p::Capabilities {
    use p::Capability::*;
    p::Capabilities::for_adapter(
        "csharp-http-protocol-v1",
        [
            UnnamedOperations,
            AdditionalMethods,
            CustomMethods,
            HttpServers,
            RelativeServers,
            DocumentRelativeServers,
            SchemaResources,
            DynamicSchemaReferences,
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
            OpenApi32,
        ],
    )
    .with_limits(p::ByteLimits::new(
        8 * 1024 * 1024,
        1024 * 1024,
        1024 * 1024,
    ))
}
pub(super) fn plan(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    options: ProtocolOptions,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_with_validation(contract, selected, config, options, true)
}

// Normal generation selects explicit v3 only for a resource-bearing closure;
// ordinary closures keep their native-verified v1/v2 programs. The v1 path is
// retained for the focused inactive-profile byte-parity witness.
pub(super) fn plan_with_validation(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    options: ProtocolOptions,
    scoped: bool,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_with_capabilities(contract, selected, config, options, scoped, capabilities())
}

pub(super) fn plan_with_capabilities(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    options: ProtocolOptions,
    scoped: bool,
    capabilities: p::Capabilities,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let fallback = selected
        .first()
        .cloned()
        .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()));
    if selected.is_empty() {
        return Err(vec![diagnostic(
            &contract,
            fallback,
            "http-no-operations",
            "select at least one outgoing operation",
        )]);
    }
    if !super::valid_package(&config.name)
        || !super::valid_namespace(&config.namespace)
        || semver::Version::parse(&config.version).is_err()
    {
        return Err(vec![diagnostic(
            &contract,
            fallback,
            "http-packaging-identity",
            "C# requires a portable NuGet ID, exact SemVer and non-keyword namespace",
        )]);
    }
    let capabilities = options
        .compatibility_profiles
        .into_iter()
        .fold(capabilities, |cap, profile| cap.with_profile(profile));
    let wire = p::plan(&contract, selected, capabilities)
        .into_result()
        .map_err(|ds| {
            ds.into_iter()
                .map(|d| HttpDiagnostic {
                    source: d.source().source().clone(),
                    at: d.source().span(),
                    code: d.code(),
                    message: d.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
    let credential_env =
        crate::credential_env::plan(&contract, &wire, options.credential_env.as_ref())?;
    let mut errors = Vec::new();
    for op in wire.operations() {
        if op.method().is_custom()
            && [
                "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE", "CONNECT",
            ]
            .iter()
            .any(|known| {
                op.method().as_str().eq_ignore_ascii_case(known) && op.method().as_str() != *known
            })
        {
            errors.push(diagnostic(&contract,op.source().use_site().source().clone(),"csharp-method-case-unsupported","HttpClient's built-in method handling normalizes known method spellings; a case-distinct custom spelling requires a different verified transport profile"));
        }
        for h in op.responses().iter().flat_map(|r| r.headers()) {
            if matches!(
                h.serialization(),
                p::ParameterSerialization::Style {
                    shape: p::WireShape::FlatObject {
                        additional: p::AdditionalScalars::AnyScalar,
                        ..
                    },
                    ..
                }
            ) {
                errors.push(diagnostic(&contract,h.source().use_site().source().clone(),"csharp-header-untyped-extras-unsupported","untyped extra header values have no unambiguous scalar decode; declare their scalar type"));
            }
        }
        for media in op.body().into_iter().flat_map(|b| b.media()) {
            if matches!(media.representation(), p::Representation::Stream { .. }) {
                errors.push(diagnostic(&contract,media.source().use_site().source().clone(),"csharp-request-stream-unsupported","sequential response streams are supported; streaming uploads require an explicit producer/lifetime profile"));
            }
        }
        for media in op
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(op.responses().iter().flat_map(|r| r.media()))
        {
            if let p::Representation::Multipart {
                multipart: p::MultipartPlan::Positional { prefix, items, .. },
            } = media.representation()
            {
                for part in prefix.iter().chain(match items {
                    p::AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    if matches!(part.representation(), p::PartRepresentation::Style { .. }) {
                        errors.push(diagnostic(&contract,part.encoding_source().map_or_else(||part.source().use_site().source(),|s|s.use_site().source()).clone(),"csharp-positional-style-unsupported","positional parts have no source field name for RFC6570-style expansion; declare JSON/text/binary content encoding"));
                    }
                }
            }
        }
        for media in op.responses().iter().flat_map(|r| r.media()) {
            if let p::Representation::Multipart {
                multipart:
                    p::MultipartPlan::Named {
                        parts, additional, ..
                    },
            } = media.representation()
            {
                for part in parts.iter().chain(match additional {
                    p::AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    if matches!(
                        part.representation(),
                        p::PartRepresentation::Style {
                            serialization: p::ParameterSerialization::Style {
                                shape: p::WireShape::Array { .. } | p::WireShape::FlatObject { .. },
                                ..
                            },
                            ..
                        }
                    ) {
                        errors.push(diagnostic(&contract,part.source().use_site().source().clone(),"csharp-response-part-style-unsupported","composite styled multipart responses require an explicit inverse delimiter/grouping convention"));
                    }
                }
            }
            if let p::Representation::Multipart {
                multipart: p::MultipartPlan::Positional { prefix, items, .. },
            } = media.representation()
            {
                for part in prefix.iter().chain(match items {
                    p::AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    for header in part.headers() {
                        if matches!(
                            header.serialization(),
                            p::ParameterSerialization::Style {
                                shape: p::WireShape::FlatObject {
                                    additional: p::AdditionalScalars::AnyScalar,
                                    ..
                                },
                                ..
                            }
                        ) {
                            errors.push(diagnostic(&contract,header.source().use_site().source().clone(),"csharp-header-untyped-extras-unsupported","untyped extra part-header values have no unambiguous scalar decode"));
                        }
                    }
                }
            }
            if let p::Representation::Form { form } = media.representation() {
                for part in form.fields().iter().chain(match form.additional() {
                    p::AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    if matches!(
                        part.representation(),
                        p::PartRepresentation::Style {
                            serialization: p::ParameterSerialization::Style {
                                shape: p::WireShape::Array { .. } | p::WireShape::FlatObject { .. },
                                ..
                            },
                            ..
                        }
                    ) {
                        errors.push(diagnostic(&contract,part.source().use_site().source().clone(),"csharp-response-form-style-unsupported","composite RFC6570 form responses need an unambiguous inverse field-grouping convention; content-based fields are supported"));
                    }
                }
            }
        }
    }
    let resources = scoped && super::resources::required(&contract, wire.codec_schema_closure());
    let reachable = if resources {
        wire.codec_schema_closure().to_vec()
    } else {
        contract.reachable_from(wire.codec_roots())
    };
    for id in &reachable {
        if let Some(raw) = contract.source(id) {
            for keyword in ["readOnly", "writeOnly"] {
                if raw
                    .get(keyword)
                    .is_some_and(|v| v != &serde_json::Value::Bool(false))
                {
                    errors.push(diagnostic(&contract,id.child(keyword),"csharp-directional-codec-unsupported","active directional projections are not implemented by this native model profile"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let compiler = OwnedCompiler::new(suspect_schema::Config {
        max_depth: 128,
        ..Default::default()
    });
    let compiled = if resources {
        compiler.compile_v3(contract.clone(), &reachable)
    } else if scoped {
        compiler.compile_v2(contract.clone(), &reachable)
    } else {
        compiler.compile(contract.clone(), &reachable)
    }
    .map_err(|ds| {
        ds.into_iter()
            .map(|d| HttpDiagnostic {
                source: d.source,
                at: d.span.unwrap_or(0..0),
                code: "csharp-schema-compilation",
                message: d.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = compiled.program();
    super::validation::check(&contract, &program)?;
    let sensitive = if resources {
        super::resources::context_sensitive(&contract, &reachable)
    } else {
        BTreeSet::new()
    };
    let mut nullable = BTreeMap::new();
    for id in &reachable {
        if sensitive.contains(id) {
            // A context-free dynamic fallback cannot prove a child's null domain.
            // Keep a conservative native domain; complete-root validation still
            // checks actual null membership under the entered resource context.
            let raw = contract.source(id).expect("compiled schema");
            let permits_null = match raw.get("type") {
                Some(serde_json::Value::String(ty)) => {
                    ty == "null" || raw.get("nullable") == Some(&serde_json::Value::Bool(true))
                }
                Some(serde_json::Value::Array(types)) => types.iter().any(|ty| ty == "null"),
                _ => true,
            };
            nullable.insert(id.clone(), permits_null);
            continue;
        }
        match compiled.validate(id, &serde_json::Value::Null) {
            OwnedOutcome::Valid => {
                nullable.insert(id.clone(), true);
            }
            OwnedOutcome::Invalid(_) => {
                nullable.insert(id.clone(), false);
            }
            OwnedOutcome::EvaluationFailure(f) => errors.push(diagnostic(
                &contract,
                f.source,
                "csharp-nullability-unproved",
                f.message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let scoped = matches!(
        program.version,
        suspect_schema::OwnedProgram::V2_VERSION | suspect_schema::OwnedProgram::V3_VERSION
    ) || crate::schema_view::has_intersections(&contract, &reachable);
    let mut models = models::plan(
        &contract,
        wire.codec_roots(),
        nullable,
        scoped,
        resources,
        sensitive,
    )?;
    let mut runtime_names = vec![
        "HttpCodecs",
        "StreamFramer",
        "JsonSerializer",
        "ContentDispositionHeaderValue",
        "ScalarComparer",
        "PositionalRuntime",
        "ServerRuntime",
        "ResourceInfo",
        "ApiUrlBase",
    ];
    if scoped {
        runtime_names.extend([
            "ValidationProgramGuard",
            "SortedDictionary",
            "SortedSet",
            "IComparer",
            "KeyValuePair",
        ]);
    }
    if resources {
        runtime_names.extend(["IPAddress", "AddressFamily"]);
    }
    if credential_env.is_some() {
        runtime_names.push("CredentialEnvironment");
    }
    let mut allocated: BTreeSet<String> = models
        .names
        .values()
        .cloned()
        .chain(runtime_names.iter().map(|s| (*s).into()))
        .collect();
    for name in models.names.values_mut() {
        if runtime_names.contains(&name.as_str()) {
            *name = allocate(name, &mut allocated);
        }
    }
    let indices = reachable
        .iter()
        .filter_map(|id| {
            program
                .roots
                .iter()
                .find(|r| {
                    r.source.document == id.document().as_str() && r.source.pointer == id.pointer()
                })
                .map(|r| (id.clone(), r.target))
        })
        .collect();
    let mut used: BTreeSet<String> = models
        .names
        .values()
        .cloned()
        .chain(super::reserved_types().into_iter().map(str::to_owned))
        .collect();
    let mut methods = BTreeSet::from(["Client".into(), "Dispose".into()]);
    if credential_env.is_some() {
        methods.insert(super::credential_env::FACTORY.into());
    }
    let mut credential_names = member_names("Credentials");
    let mut credential_bindings = Vec::<PlannedCredential>::new();
    let mut credentials = BTreeMap::new();
    for op in wire.operations() {
        for requirement in op
            .security()
            .alternatives()
            .iter()
            .flat_map(|a| a.requirements())
        {
            let key = super::credential_env::key(requirement, credential_env.is_some());
            if !credentials.contains_key(&key) {
                let property_name = allocate(&exported(requirement.name()), &mut credential_names);
                let native_type = match requirement.credential() {
                    p::CredentialHook::Basic => "BasicCredential",
                    p::CredentialHook::OAuth2 { .. } | p::CredentialHook::OpenIdConnect { .. } => {
                        "AuthorizationProvider"
                    }
                    _ => "string",
                }
                .into();
                credentials.insert(key.clone(), property_name.clone());
                credential_bindings.push(PlannedCredential {
                    key,
                    property_name,
                    native_type,
                    requirement: requirement.clone(),
                });
            }
        }
    }
    let mut operations = Vec::new();
    for op in wire.operations() {
        let operation_id = op
            .operation_id()
            .map(|v| v.value().clone())
            .unwrap_or_else(|| format!("{} {}", op.method().as_str(), op.path()));
        let stem = exported(&operation_id);
        let input_type = allocate(&format!("{stem}Input"), &mut used);
        let result_type = allocate(&format!("{stem}Result"), &mut used);
        let error_type = allocate(&format!("{stem}ApiException"), &mut used);
        let mut members = member_names(&input_type);
        members.insert("Body".into());
        let parameters = op
            .parameters()
            .iter()
            .map(|p| PlannedParameter {
                property_name: allocate(&exported(p.name()), &mut members),
                source: p.source().use_site().source().clone(),
                schema: p.codec().schema().id().clone(),
                native_type: models.native_type(p.codec().schema().id()),
                wire_name: p.name().into(),
                location: p.location(),
                required: p.required(),
                wire: p.clone(),
            })
            .collect();
        let body = op.body().map(|body| {
            let media = media(body.media(), &format!("{stem}Request"), &models, &mut used);
            let union = media.len() != 1;
            let native_type = if union {
                allocate(&format!("{stem}Body"), &mut used)
            } else {
                media[0].native_type.clone()
            };
            PlannedBody {
                source: body.source().use_site().source().clone(),
                native_type,
                required: body.required(),
                wire: body.clone(),
                media,
                union,
            }
        });
        let successes = op
            .responses()
            .iter()
            .filter(|r| {
                matches!(
                    r.status(),
                    p::ResponseStatus::Exact(200..=299)
                        | p::ResponseStatus::Range(2)
                        | p::ResponseStatus::Default
                )
            })
            .count();
        let responses = op
            .responses()
            .iter()
            .map(|r| {
                let tag = status_name(r.status());
                let always_empty = op.method().as_str() == "HEAD"
                    || matches!(
                        r.status(),
                        p::ResponseStatus::Exact(100..=199 | 204 | 304)
                            | p::ResponseStatus::Range(1)
                    );
                let may_be_empty = always_empty
                    || matches!(
                        r.status(),
                        p::ResponseStatus::Range(2 | 3) | p::ResponseStatus::Default
                    );
                let media = if always_empty {
                    Vec::new()
                } else {
                    media(r.media(), &format!("{stem}{tag}"), &models, &mut used)
                };
                let union = !always_empty && (media.len() > 1 || may_be_empty);
                let native_type = if always_empty {
                    "HttpNoContent".into()
                } else if union {
                    allocate(&format!("{stem}{tag}Body"), &mut used)
                } else if media.is_empty() {
                    "byte[]".into()
                } else {
                    media[0].native_type.clone()
                };
                let may_succeed = matches!(
                    r.status(),
                    p::ResponseStatus::Exact(200..=299)
                        | p::ResponseStatus::Range(2)
                        | p::ResponseStatus::Default
                );
                let headers = headers(r.headers(), &models);
                PlannedResponse {
                    source: r.source().use_site().source().clone(),
                    native_type,
                    type_name: if may_succeed && successes == 1 {
                        result_type.clone()
                    } else {
                        format!("{result_type}.{tag}")
                    },
                    error_type_name: format!("{error_type}.{tag}"),
                    wire: r.clone(),
                    media,
                    header_type: (!headers.is_empty())
                        .then(|| allocate(&format!("{stem}{tag}Headers"), &mut used)),
                    headers,
                    union,
                    always_empty,
                    may_be_empty,
                }
            })
            .collect();
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id,
            method_name: allocate(&format!("{stem}Async"), &mut methods),
            input_type,
            result_type,
            error_type,
            http_method: op.method().as_str().into(),
            path: op.path().into(),
            description: op.description().map_or("", |d| d.value()).into(),
            wire: op.clone(),
            parameters,
            body,
            responses,
        });
    }
    let examples = if scoped {
        super::scoped_examples::plan(contract.clone(), &wire, &compiled)?
    } else {
        examples::plan_protocol_examples(contract.clone(), &wire, Default::default())
    };
    let samples = super::samples::plan(&models, &examples, &compiled)?;
    Ok(SdkPlan {
        contract,
        config,
        operations,
        models,
        credentials,
        credential_bindings,
        credential_env,
        protocol: wire,
        program,
        indices,
        examples,
        samples,
    })
}
pub fn credential_key(r: &p::CredentialRequirement) -> String {
    format!(
        "{}#{}",
        r.scheme().terminal().source().document(),
        r.scheme().terminal().source().pointer()
    )
}
pub fn status_name(status: p::ResponseStatus) -> String {
    match status {
        p::ResponseStatus::Exact(s) => format!("Status{s}"),
        p::ResponseStatus::Range(c) => format!("Status{c}xx"),
        p::ResponseStatus::Default => "Default".into(),
    }
}
fn member_names(owner: &str) -> BTreeSet<String> {
    [
        owner,
        "Extra",
        "Equals",
        "GetHashCode",
        "ToString",
        "GetType",
        "MemberwiseClone",
        "Finalize",
        "Clone",
        "EqualityContract",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn headers(wire: &[p::HeaderPlan], models: &models::ModelPlan) -> Vec<PlannedHeader> {
    let mut names = member_names("Headers");
    wire.iter()
        .map(|h| PlannedHeader {
            property_name: allocate(&exported(h.name()), &mut names),
            native_type: models.native_type(h.codec().schema().id()),
            wire: h.clone(),
        })
        .collect()
}
fn media(
    wire: &[p::MediaPlan],
    stem: &str,
    models: &models::ModelPlan,
    used: &mut BTreeSet<String>,
) -> Vec<PlannedMedia> {
    let mut names = BTreeSet::from(["NoContent".into(), "UndeclaredBytes".into(), "Value".into()]);
    wire.iter()
        .map(|m| {
            let kind = match m.representation() {
                p::Representation::Json { .. } => "Json",
                p::Representation::Text { .. } => "Text",
                p::Representation::Binary { .. } => "Bytes",
                p::Representation::Form { .. } => "Form",
                p::Representation::Multipart { .. } => "Multipart",
                p::Representation::Stream { stream } => match stream.framing() {
                    p::StreamFraming::JsonLines => "JsonLines",
                    p::StreamFraming::ServerSentEvents => "Events",
                },
            };
            let variant_name = allocate(kind, &mut names);
            let parts = match m.representation() {
                p::Representation::Form { form } => Some(parts(
                    &format!("{stem}Form"),
                    false,
                    form.rules(),
                    form.fields(),
                    form.additional(),
                    models,
                    used,
                )),
                p::Representation::Multipart {
                    multipart:
                        p::MultipartPlan::Named {
                            rules,
                            parts: fields,
                            additional,
                        },
                } => Some(parts(
                    &format!("{stem}Multipart"),
                    true,
                    rules,
                    fields,
                    additional,
                    models,
                    used,
                )),
                _ => None,
            };
            let positional = match m.representation() {
                p::Representation::Multipart {
                    multipart:
                        p::MultipartPlan::Positional {
                            schema,
                            prefix,
                            items,
                            min_items,
                            max_items,
                        },
                } => {
                    let name = allocate(&format!("{stem}Positional"), used);
                    let prefix = prefix
                        .iter()
                        .enumerate()
                        .map(|(i, p)| part(&name, format!("Item{}", i + 1), p, true, models, used))
                        .collect();
                    let items = match items {
                        p::AdditionalParts::Allowed(p) => {
                            Some(Box::new(part(&name, "Items".into(), p, true, models, used)))
                        }
                        _ => None,
                    };
                    Some(PlannedPositionalParts {
                        native_type: name,
                        schema: schema.clone(),
                        prefix,
                        items,
                        min_items: min_items.clone(),
                        max_items: max_items.clone(),
                    })
                }
                _ => None,
            };
            let native_type = match m.representation() {
                p::Representation::Json { codec } => codec
                    .as_ref()
                    .map_or("global::System.Text.Json.JsonElement".into(), |c| {
                        models.native_type(c.schema().id())
                    }),
                p::Representation::Text { codec, .. } => codec
                    .as_ref()
                    .map_or("string".into(), |c| models.native_type(c.schema().id())),
                p::Representation::Binary { .. } => "byte[]".into(),
                p::Representation::Stream { stream } => format!(
                    "HttpStream<{}>",
                    models.native_type(stream.item_codec().schema().id())
                ),
                _ => parts
                    .as_ref()
                    .map(|p| p.native_type.clone())
                    .or_else(|| positional.as_ref().map(|p| p.native_type.clone()))
                    .expect("admitted multipart representation"),
            };
            PlannedMedia {
                variant_name,
                native_type,
                wire: m.clone(),
                parts,
                positional,
            }
        })
        .collect()
}
fn parts(
    stem: &str,
    multipart: bool,
    rules: &p::ObjectRules,
    fields: &[p::PartPlan],
    additional: &p::AdditionalParts,
    models: &models::ModelPlan,
    used: &mut BTreeSet<String>,
) -> PlannedParts {
    let native_type = allocate(stem, used);
    let mut names = member_names(&native_type);
    let mut part = |p: &p::PartPlan| {
        let property_name = allocate(&exported(p.name().unwrap_or("Additional")), &mut names);
        self::part(stem, property_name, p, multipart, models, used)
    };
    let fields = fields.iter().map(&mut part).collect();
    let additional = match additional {
        p::AdditionalParts::Allowed(p) => Some(Box::new(part(p))),
        _ => None,
    };
    PlannedParts {
        native_type,
        multipart,
        rules: rules.clone(),
        fields,
        additional,
    }
}
fn part(
    stem: &str,
    property_name: String,
    p: &p::PartPlan,
    multipart: bool,
    models: &models::ModelPlan,
    used: &mut BTreeSet<String>,
) -> PlannedPart {
    let value_type = match p.representation() {
        p::PartRepresentation::Binary { .. } => "byte[]".into(),
        p::PartRepresentation::Json { codec, .. }
        | p::PartRepresentation::Text { codec, .. }
        | p::PartRepresentation::Style { codec, .. } => models.native_type(codec.schema().id()),
    };
    let headers = headers(p.headers(), models);
    let wrapper_type = multipart.then(|| allocate(&format!("{stem}{property_name}Part"), used));
    let header_type =
        (!headers.is_empty()).then(|| allocate(&format!("{stem}{property_name}PartHeaders"), used));
    let item_type = wrapper_type.clone().unwrap_or_else(|| value_type.clone());
    let native_type = if p.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
        format!("global::System.Collections.Generic.List<{item_type}>")
    } else {
        item_type
    };
    PlannedPart {
        property_name,
        native_type,
        value_type,
        wrapper_type,
        header_type,
        headers,
        wire: p.clone(),
    }
}
