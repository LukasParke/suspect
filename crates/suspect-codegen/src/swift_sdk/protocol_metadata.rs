//! Swift literal lowering for typed HTTP descriptors (not a runtime schema parser).
use crate::http_protocol::*;
use suspect_ir::contract::SourceId;

use super::{
    SdkPlan,
    models::{Declaration, Type},
    validation,
};

pub(super) fn q(value: &str) -> String {
    validation::string(value)
}
pub(super) fn source(id: &SourceId) -> String {
    validation::source(&suspect_schema::ProgramSource::from(id))
}
pub(super) fn provenance(p: &Provenance) -> String {
    format!(
        "HTTPProvenance(useSite: {}, terminal: {}, references: [{}], useSiteResource: {}, terminalResource: {}, referenceResources: [{}])",
        source(p.use_site().source()),
        source(p.terminal().source()),
        p.references()
            .iter()
            .map(|s| source(s.source()))
            .collect::<Vec<_>>()
            .join(", "),
        p.use_site_resource()
            .map(resource)
            .unwrap_or_else(|| "nil".into()),
        p.terminal_resource()
            .map(resource)
            .unwrap_or_else(|| "nil".into()),
        p.reference_resources()
            .iter()
            .map(|r| r.as_ref().map(resource).unwrap_or_else(|| "nil".into()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn resource(r: &ResourceContext) -> String {
    format!(
        "HTTPResourceContext(source: {}, resource: {}, kind: {}, canonicalURI: {}, baseURI: {}, baseSource: {}, scopeAddress: {}, schemaRoot: {}, aliases: [{}])",
        source(r.source().source()),
        source(r.resource().source()),
        match r.kind() {
            ResourceKind::Document => ".document",
            ResourceKind::OpenApiDocument => ".openAPIDocument",
            ResourceKind::Schema => ".schema",
        },
        q(r.canonical_uri()),
        q(r.base_uri()),
        r.base_source()
            .map(|s| source(s.source()))
            .unwrap_or_else(|| "nil".into()),
        q(r.scope_address()),
        r.schema_root()
            .map(|s| source(s.source()))
            .unwrap_or_else(|| "nil".into()),
        r.aliases()
            .iter()
            .map(|a| q(a))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn url_base(base: ApiUrlBase) -> &'static str {
    match base {
        ApiUrlBase::ServerDocument => ".serverDocument",
        ApiUrlBase::EffectiveServer => ".effectiveServer",
    }
}
fn located(v: &Located<String>) -> String {
    format!(
        "HTTPLocated(value: {}, source: {})",
        q(v.value()),
        source(v.source().source())
    )
}
fn optional(v: Option<&Located<String>>) -> String {
    v.map(located).unwrap_or_else(|| "nil".into())
}
fn strings(values: &[Located<String>]) -> String {
    format!(
        "[{}]",
        values.iter().map(located).collect::<Vec<_>>().join(", ")
    )
}
fn dictionary(values: impl IntoIterator<Item = (String, String)>) -> String {
    let pairs = values
        .into_iter()
        .map(|(k, v)| format!("{}: {v}", q(&k)))
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        "[:]".into()
    } else {
        format!("[{}]", pairs.join(", "))
    }
}
fn exact_dictionary(values: impl IntoIterator<Item = (String, String)>) -> String {
    format!(
        "JsonObject(trusted: [{}])",
        values
            .into_iter()
            .map(|(k, v)| format!("({}, {v})", q(&k)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(super) fn json(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Null => ".null".into(),
        Value::Bool(b) => format!(".bool({b})"),
        Value::String(s) => format!(".string({})", q(s)),
        Value::Number(n) => format!(".number(JsonNumber(trusted: {}))", q(&n.to_string())),
        Value::Array(a) => format!(
            ".array([{}])",
            a.iter().map(json).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(o) => format!(
            ".object(JsonObject(trusted: [{}]))",
            o.iter()
                .map(|(k, v)| format!("({}, {})", q(k), json(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
pub(super) fn media(m: &MediaType) -> String {
    let (t, s) = match m.range() {
        MediaRange::Any => ("*", "*"),
        MediaRange::Type { type_name } => (type_name.as_str(), "*"),
        MediaRange::Concrete { type_name, subtype } => (type_name.as_str(), subtype.as_str()),
    };
    format!(
        "HTTPMediaType(declared: {}, type: {}, subtype: {}, parameters: {})",
        q(m.declared()),
        q(t),
        q(s),
        dictionary(m.parameters().iter().map(|(k, v)| (k.clone(), q(v))))
    )
}
pub(super) fn media_list(values: &[MediaPlan]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|m| media(m.media_type()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(super) fn scalar(s: ScalarType) -> &'static str {
    match s {
        ScalarType::String => ".string",
        ScalarType::Boolean => ".boolean",
        ScalarType::Integer => ".integer",
        ScalarType::Number => ".number",
    }
}
pub(super) fn encoding(p: PercentEncoding) -> &'static str {
    match p {
        PercentEncoding::UriComponent => ".uriComponent",
        PercentEncoding::ReservedExpansion => ".reservedExpansion",
        PercentEncoding::None => ".none",
        PercentEncoding::FormUrlEncoded => ".formUrlEncoded",
    }
}
fn text_scalar(plan: &SdkPlan, codec: &CodecRef) -> &'static str {
    match &plan.models.types[codec.schema().id()].core {
        Type::Primitive("Bool") => ".boolean",
        Type::Primitive("JsonInteger") => ".integer",
        Type::Primitive("JsonNumber") => ".number",
        Type::Named(id)
            if matches!(
                plan.models.declarations.get(id),
                Some(Declaration::Literals(_))
            ) =>
        {
            ".string"
        }
        _ => ".string",
    }
}
pub(super) fn serialization(
    p: &ParameterSerialization,
    codec: &CodecRef,
    plan: &SdkPlan,
) -> String {
    match p {
        ParameterSerialization::Style {
            style,
            explode,
            shape,
            percent_encoding,
        } => {
            let style = match style {
                Style::Simple => ".simple",
                Style::Label => ".label",
                Style::Matrix => ".matrix",
                Style::Form => ".form",
                Style::SpaceDelimited => ".spaceDelimited",
                Style::PipeDelimited => ".pipeDelimited",
                Style::DeepObject => ".deepObject",
                Style::Cookie => ".cookie",
            };
            let shape = match shape {
                WireShape::Scalar { scalar: s } => format!(".scalar({})", scalar(*s)),
                WireShape::Array { items } => format!(".array({})", scalar(*items)),
                WireShape::FlatObject {
                    properties,
                    additional,
                } => format!(
                    ".object(properties: {}, additional: {}, allowAny: {})",
                    exact_dictionary(
                        properties
                            .iter()
                            .map(|(k, v)| (k.clone(), scalar(*v).into()))
                    ),
                    if let AdditionalScalars::Typed(s) = additional {
                        scalar(*s)
                    } else {
                        "nil"
                    },
                    *additional == AdditionalScalars::AnyScalar
                ),
            };
            format!(
                ".style({style}, explode: {explode}, shape: {shape}, encoding: {})",
                encoding(*percent_encoding)
            )
        }
        ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => format!(
            ".content(json: {}, scalar: {}, encoding: {})",
            media_type.is_json(),
            text_scalar(plan, codec),
            encoding(*percent_encoding)
        ),
    }
}
pub(super) fn parameter(p: &ParameterPlan, plan: &SdkPlan) -> String {
    parameter_value(
        p.name(),
        &format!("{:?}", p.location()).to_ascii_lowercase(),
        p.required(),
        p.source().use_site().source(),
        p.serialization(),
        p.codec(),
        plan,
    )
}
pub(super) fn header(p: &HeaderPlan, plan: &SdkPlan) -> String {
    parameter_value(
        p.name(),
        "header",
        p.required(),
        p.source().use_site().source(),
        p.serialization(),
        p.codec(),
        plan,
    )
}
pub(super) fn parameter_value(
    name: &str,
    location: &str,
    required: bool,
    at: &SourceId,
    s: &ParameterSerialization,
    codec: &CodecRef,
    plan: &SdkPlan,
) -> String {
    format!(
        "HTTPParameter(name: {}, location: {}, required: {required}, source: {}, serialization: {})",
        q(name),
        q(location),
        source(at),
        serialization(s, codec, plan)
    )
}
pub(super) fn server(s: &ServerPlan) -> String {
    format!("HTTPServer(source: {}, defaultFrom: {}, documentBase: {}, urlBase: {}, template: {}, name: {}, description: {}, variables: [{}])",
        s.source().map(provenance).unwrap_or_else(|| "nil".into()), s.default_from().map(|s| source(s.source())).unwrap_or_else(|| "nil".into()), source(s.document_base().source()), url_base(s.url_base()), q(s.template()), optional(s.name()), optional(s.description()),
        s.variables().iter().map(|v| format!("HTTPServerVariable(name: {}, source: {}, defaultValue: {}, values: {}, description: {})", q(v.name()), provenance(v.source()), located(v.default()), v.values().map(strings).unwrap_or_else(|| "nil".into()), optional(v.description()))).collect::<Vec<_>>().join(", "))
}
fn requirement(r: &CredentialRequirement) -> String {
    let (kind, bearer, flows, metadata, discovery) = match r.credential() {
        CredentialHook::Bearer { bearer_format } => (".bearer".into(), optional(bearer_format.as_ref()), "[]".into(), "nil".into(), "nil".into()),
        CredentialHook::Basic => (".basic".into(), "nil".into(), "[]".into(), "nil".into(), "nil".into()),
        CredentialHook::ApiKey { location, name } => (format!(".apiKey(location: {}, name: {})", q(&format!("{location:?}").to_ascii_lowercase()), q(name.value())), "nil".into(), "[]".into(), "nil".into(), "nil".into()),
        CredentialHook::OAuth2 { flows, metadata_url } => (".oauth2".into(), "nil".into(), format!("[{}]", flows.iter().map(|flow| format!("HTTPOAuthFlow(source: {}, kind: {}, urlBase: {}, authorizationURL: {}, tokenURL: {}, refreshURL: {}, deviceAuthorizationURL: {}, scopes: {})",
            source(flow.source().source()), q(match flow.kind() { OAuthFlowKind::Implicit => "implicit", OAuthFlowKind::Password => "password", OAuthFlowKind::ClientCredentials => "clientCredentials", OAuthFlowKind::AuthorizationCode => "authorizationCode", OAuthFlowKind::DeviceAuthorization => "deviceAuthorization" }), url_base(flow.url_base()),
            optional(flow.authorization_url()), optional(flow.token_url()), optional(flow.refresh_url()), optional(flow.device_authorization_url()), exact_dictionary(flow.scopes().iter().map(|(k,v)| (k.clone(), located(v)))))).collect::<Vec<_>>().join(", ")), optional(metadata_url.as_ref()), "nil".into()),
        CredentialHook::OpenIdConnect { discovery_url } => (".openIdConnect".into(), "nil".into(), "[]".into(), "nil".into(), located(discovery_url)),
    };
    let permissions = match r.permissions() {
        Permissions::Scopes(s) => format!(".scopes({})", strings(s)),
        Permissions::Roles(s) => format!(".roles({})", strings(s)),
    };
    format!(
        "HTTPCredentialContext(source: {}, scheme: {}, name: {}, kind: {kind}, permissions: {permissions}, description: {}, bearerFormat: {bearer}, flows: {flows}, metadataURL: {metadata}, discoveryURL: {discovery}, urlBase: {})",
        source(r.source().source()),
        provenance(r.scheme()),
        q(r.name()),
        optional(r.description()),
        r.credential().url_base().map(url_base).unwrap_or("nil")
    )
}
pub(super) fn metadata(op: &OperationPlan) -> String {
    let security = match op.security() {
        SecurityPlan::Undeclared { source: s } => format!(".undeclared({})", source(s.source())),
        SecurityPlan::NoAuth { source: s } => format!(".disabled({})", source(s.source())),
        SecurityPlan::Alternatives { alternatives, .. } => format!(
            ".alternatives([{}])",
            alternatives
                .iter()
                .map(|a| format!(
                    "[{}]",
                    a.requirements()
                        .iter()
                        .map(requirement)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!(
        "HTTPMetadata(source: {}, servers: [{}], security: {security})",
        provenance(op.source()),
        op.servers()
            .candidates()
            .iter()
            .map(server)
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(super) fn response(r: &ResponsePlan) -> String {
    let (exact, range) = match r.status() {
        ResponseStatus::Exact(n) => (n.to_string(), "nil".into()),
        ResponseStatus::Range(n) => ("nil".into(), n.to_string()),
        ResponseStatus::Default => ("nil".into(), "nil".into()),
    };
    format!(
        "HTTPResponseRule(exact: {exact}, range: {range}, media: {}, source: {})",
        media_list(r.media()),
        source(r.source().use_site().source())
    )
}
pub(super) fn links(values: &[LinkPlan]) -> String {
    let lit = |v: &Located<serde_json::Value>| {
        format!(
            "HTTPLocated<JsonValue>(value: {}, source: {})",
            json(v.value()),
            source(v.source().source())
        )
    };
    format!("[{}]", values.iter().map(|l| {
        let target = match l.target() {
            LinkTarget::OperationId { value, operation } => format!(".operationID(value: {}, operation: {})", located(value), source(operation.source())),
            LinkTarget::OperationRef { value, operation } => format!(".operationReference(value: {}, operation: {})", located(value), source(operation.source())),
        };
        format!("HTTPLink(name: {}, source: {}, target: {target}, parameters: {}, requestBody: {}, description: {}, server: {})", q(l.name()), provenance(l.source()), exact_dictionary(l.parameters().iter().map(|(k,v)| (k.clone(), lit(v)))), l.request_body().map(lit).unwrap_or_else(|| "nil".into()), optional(l.description()), l.server().map(server).unwrap_or_else(|| "nil".into()))
    }).collect::<Vec<_>>().join(", "))
}
pub(super) fn count(v: Option<&Located<u64>>) -> String {
    v.map(|v| v.value().to_string())
        .unwrap_or_else(|| "nil".into())
}
pub(super) fn part(p: &PartPlan, plan: &SdkPlan) -> String {
    let max = match p.representation() {
        PartRepresentation::Binary { bytes } => bytes.max_bytes() as usize,
        _ => plan.config.max_part_bytes,
    };
    format!(
        "HTTPPartRule(name: {}, required: {}, repeated: {}, minItems: {}, maxItems: {}, contentTypes: [{}], maxBytes: {max})",
        p.name().map(q).unwrap_or_else(|| "nil".into()),
        p.required(),
        p.multiplicity() == PartMultiplicity::RepeatedArrayItems,
        count(p.min_items()),
        count(p.max_items()),
        p.content_types()
            .iter()
            .map(media)
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(super) fn rules(parts: &super::PlannedParts) -> String {
    format!(
        "HTTPObjectRules(names: [{}], required: [{}], additional: {}, minProperties: {}, maxProperties: {})",
        parts
            .fields
            .iter()
            .map(|p| q(p.wire.name().unwrap()))
            .collect::<Vec<_>>()
            .join(", "),
        parts
            .rules
            .required()
            .iter()
            .map(|n| q(n.value()))
            .collect::<Vec<_>>()
            .join(", "),
        parts.additional.is_some(),
        count(parts.rules.min_properties()),
        count(parts.rules.max_properties())
    )
}
