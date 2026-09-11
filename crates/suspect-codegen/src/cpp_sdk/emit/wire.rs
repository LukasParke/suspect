//! Lower immutable shared wire descriptors into C++ protocol tables.
use super::super::{PlannedOperation, PlannedPart, ValueKind, ValueType};
use super::{SdkPlan, source_expr, string};
use crate::http_protocol as w;
use std::fmt::Write;

pub(super) fn scalar(value: w::ScalarType) -> &'static str {
    match value {
        w::ScalarType::String => "String",
        w::ScalarType::Boolean => "Boolean",
        w::ScalarType::Integer => "Integer",
        w::ScalarType::Number => "Number",
    }
}
pub(super) fn encoding(value: w::PercentEncoding) -> &'static str {
    match value {
        w::PercentEncoding::UriComponent => "Component",
        w::PercentEncoding::ReservedExpansion => "Reserved",
        w::PercentEncoding::None => "None",
        w::PercentEncoding::FormUrlEncoded => "Form",
    }
}
pub(super) fn location(value: w::ParameterLocation) -> &'static str {
    match value {
        w::ParameterLocation::Path => "Path",
        w::ParameterLocation::Query => "Query",
        w::ParameterLocation::Querystring => "Querystring",
        w::ParameterLocation::Header => "Header",
        w::ParameterLocation::Cookie => "Cookie",
    }
}
pub(super) fn shape(value: &w::WireShape) -> String {
    let (kind, scalar, props, additional, extra) = match value {
        w::WireShape::Scalar { scalar: s } => ("Scalar", scalar(*s), String::new(), false, "Any"),
        w::WireShape::Array { items } => ("Array", scalar(*items), String::new(), false, "Any"),
        w::WireShape::FlatObject {
            properties,
            additional,
        } => {
            let (allowed, extra) = match additional {
                w::AdditionalScalars::Forbidden => (false, "Any"),
                w::AdditionalScalars::AnyScalar => (true, "Any"),
                w::AdditionalScalars::Typed(s) => (true, scalar(*s)),
            };
            (
                "Object",
                "Any",
                properties
                    .iter()
                    .map(|(k, v)| format!("{{{}, detail::Scalar::{}}}", string(k), scalar(*v)))
                    .collect::<Vec<_>>()
                    .join(", "),
                allowed,
                extra,
            )
        }
    };
    format!(
        "detail::WireShape{{detail::ShapeKind::{kind}, detail::Scalar::{scalar}, {{{props}}}, {additional}, detail::Scalar::{extra}}}"
    )
}
pub(super) fn serialization(value: &w::ParameterSerialization) -> String {
    match value {
        w::ParameterSerialization::Style {
            style,
            explode,
            shape: s,
            percent_encoding,
        } => {
            let style = match style {
                w::Style::Simple => "Simple",
                w::Style::Label => "Label",
                w::Style::Matrix => "Matrix",
                w::Style::Form => "Form",
                w::Style::SpaceDelimited => "SpaceDelimited",
                w::Style::PipeDelimited => "PipeDelimited",
                w::Style::DeepObject => "DeepObject",
                w::Style::Cookie => "Cookie",
            };
            format!(
                "detail::Serialization{{detail::Style::{style}, {explode}, {}, detail::Encoding::{}, false}}",
                shape(s),
                encoding(*percent_encoding)
            )
        }
        w::ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => format!(
            "detail::Serialization{{detail::Style::Content, false, {{}}, detail::Encoding::{}, {}}}",
            encoding(*percent_encoding),
            media_type.is_json()
        ),
    }
}
pub(super) fn media(value: &w::MediaType, utf8: bool) -> String {
    let (t, s) = match value.range() {
        w::MediaRange::Any => ("*", "*"),
        w::MediaRange::Type { type_name } => (type_name.as_str(), "*"),
        w::MediaRange::Concrete { type_name, subtype } => (type_name.as_str(), subtype.as_str()),
    };
    format!(
        "detail::Media{{{}, {}, {}, {{{}}}, {utf8}}}",
        string(value.declared()),
        string(t),
        string(s),
        value
            .parameters()
            .iter()
            .map(|(k, v)| format!("{{{}, {}}}", string(k), string(v)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(super) fn media_plan(value: &w::MediaPlan) -> String {
    media(
        value.media_type(),
        matches!(
            value.representation(),
            w::Representation::Text { .. }
                | w::Representation::Form { .. }
                | w::Representation::Stream { .. }
        ),
    )
}
pub(super) fn operation(plan: &SdkPlan, op: &PlannedOperation) -> String {
    let servers = op
        .wire
        .servers()
        .candidates()
        .iter()
        .map(|s| {
            let source = s
                .source()
                .map(|p| p.terminal().source())
                .or_else(|| s.default_from().map(|p| p.source()))
                .unwrap();
            let variables = s
                .variables()
                .iter()
                .map(|v| {
                    format!(
                        "detail::ServerVariable{{{}, {}, {}, {}}}",
                        source_expr(plan, v.source().terminal().source()),
                        string(v.name()),
                        string(v.default().value()),
                        v.values()
                            .map(|values| format!(
                                "std::vector<std::string>{{{}}}",
                                values
                                    .iter()
                                    .map(|v| string(v.value()))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ))
                            .unwrap_or_else(|| "std::nullopt".into())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "detail::Server{{{}, {}, {}, {{{variables}}}}}",
                source_expr(plan, source),
                string(s.document_base().source().document().as_str()),
                string(s.template())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let security=op.wire.security().alternatives().iter().map(|a|format!("{{{}}}",a.requirements().iter().map(|r|{
        let credential=plan.credential_for(r);
        let(kind,name)=match r.credential(){w::CredentialHook::Bearer{..}=>("Bearer",""),w::CredentialHook::Basic=>("Basic",""),w::CredentialHook::OAuth2{..}|w::CredentialHook::OpenIdConnect{..}=>("Provider",""),w::CredentialHook::ApiKey{location,name}=>(match location{w::ParameterLocation::Header=>"HeaderKey",w::ParameterLocation::Cookie=>"CookieKey",_=>"QueryKey"},name.value().as_str())};
        let(scopes,roles)=match r.permissions(){w::Permissions::Scopes(v)=>(v.as_slice(),&[][..]),w::Permissions::Roles(v)=>(&[][..],v.as_slice())};
        format!("detail::Requirement{{{}, {}, {}, detail::CredentialKind::{kind}, {}, {{{}}}, {{{}}}, detail::literal({})}}",source_expr(plan,r.scheme().terminal().source()),string(&credential.field_name),string(r.name()),string(name),scopes.iter().map(|s|string(s.value())).collect::<Vec<_>>().join(", "),roles.iter().map(|s|string(s.value())).collect::<Vec<_>>().join(", "),string(&serde_json::to_string(r.credential()).unwrap()))
    }).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join(", ");
    let mut accept = std::collections::BTreeMap::new();
    for r in op.wire.responses() {
        for m in r.media() {
            accept.insert(m.media_type().declared(), media_plan(m));
        }
    }
    format!(
        "detail::Operation{{{}, {}, {}, {}, {{{servers}}}, {{{security}}}, {{{}}}}}",
        source_expr(plan, &op.source),
        string(&op.operation_id),
        string(op.wire.method().as_str()),
        string(op.wire.path()),
        accept.values().cloned().collect::<Vec<_>>().join(", ")
    )
}
pub(super) fn part(plan: &SdkPlan, value: &PlannedPart) -> String {
    let p = &value.wire;
    let (kind, scalar, serialization, outer, max) = match p.representation() {
        w::PartRepresentation::Json { outer_encoding, .. } => (
            "Json",
            "String",
            "{}".into(),
            encoding(*outer_encoding),
            plan.config().max_part_bytes,
        ),
        w::PartRepresentation::Text {
            scalar: s,
            outer_encoding,
            ..
        } => (
            "Text",
            scalar(*s),
            "{}".into(),
            encoding(*outer_encoding),
            plan.config().max_part_bytes,
        ),
        w::PartRepresentation::Binary { bytes } => (
            "Binary",
            "String",
            "{}".into(),
            "None",
            bytes.max_bytes() as usize,
        ),
        w::PartRepresentation::Style {
            serialization: s, ..
        } => (
            "Style",
            "String",
            serialization(s),
            "None",
            plan.config().max_part_bytes,
        ),
    };
    format!(
        "detail::PartRules{{{}, {}, {}, {}, {}, {}, {max}, detail::PartEncoding::{kind}, detail::Scalar::{scalar}, {serialization}, detail::Encoding::{outer}, {{{}}}}}",
        source_expr(plan, &value.source),
        string(p.name().unwrap_or("")),
        p.multiplicity() == w::PartMultiplicity::RepeatedArrayItems,
        p.required(),
        count(p.min_items()),
        count(p.max_items()),
        p.content_types()
            .iter()
            .map(|m| media(m, m.is_text()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn count(value: Option<&w::Located<u64>>) -> String {
    value
        .map(|v| v.value().to_string())
        .unwrap_or_else(|| "std::nullopt".into())
}
pub(super) fn rules(plan: &SdkPlan, rules: &w::ObjectRules) -> String {
    format!(
        "detail::ObjectRules{{{}, {{{}}}, {}, {}}}",
        source_expr(plan, rules.schema().id()),
        rules
            .required()
            .iter()
            .map(|v| string(v.value()))
            .collect::<Vec<_>>()
            .join(", "),
        count(rules.min_properties()),
        count(rules.max_properties())
    )
}
pub(super) fn encode_json(
    plan: &SdkPlan,
    value: &ValueType,
    expression: &str,
    out: &mut String,
    variable: &str,
    source: &str,
) {
    match &value.kind {
        ValueKind::Model(id) => {
            let symbol = plan.models().get(id);
            writeln!(out,"auto {variable} = detail::encode_{}({expression}, context, \"\");\ncontext.validation.require({}, {variable}, \"\");",symbol.index,symbol.index).unwrap();
        }
        ValueKind::Json => {
            writeln!(
                out,
                "auto {variable} = context.copy({expression}, {source}, \"\");"
            )
            .unwrap();
        }
        ValueKind::Scalar(_) => {
            writeln!(out, "auto {variable} = JsonValue({expression});").unwrap();
        }
        _ => unreachable!("only actual JSON/text codec inputs enter JSON conversion"),
    }
}
pub(super) fn decode_json(
    plan: &SdkPlan,
    value: &ValueType,
    json: &str,
    out: &mut String,
    variable: &str,
    source: &str,
) {
    match &value.kind {
        ValueKind::Model(id) => {
            let symbol = plan.models().get(id);
            writeln!(out,"context.validation.require({}, {json}, \"\");\nauto {variable} = detail::decode_{}({json}, context, \"\");",symbol.index,symbol.index).unwrap();
        }
        ValueKind::Json => {
            writeln!(
                out,
                "auto {variable} = context.copy({json}, {source}, \"\");"
            )
            .unwrap();
        }
        ValueKind::Scalar(s) => {
            let ty = match s {
                w::ScalarType::String => "std::string",
                w::ScalarType::Boolean => "bool",
                _ => "JsonNumber",
            };
            if *s == w::ScalarType::Integer {
                writeln!(out,"auto {variable}_integer = JsonInteger::from_number(detail::as<JsonNumber>({json}, {source}, \"\"));\nif (!{variable}_integer) throw detail::Failure{{std::move({variable}_integer).error()}};\nauto {variable} = std::move({variable}_integer).value();").unwrap();
            } else {
                writeln!(
                    out,
                    "auto {variable} = detail::as<{ty}>({json}, {source}, \"\");"
                )
                .unwrap();
            }
        }
        _ => unreachable!("non-JSON representation has a separate native decoder"),
    }
}
pub(super) fn links(plan: &SdkPlan, links: &[w::LinkPlan]) -> String {
    links
        .iter()
        .map(|link| {
            let (kind, target) = match link.target() {
                w::LinkTarget::OperationId { value, .. } => ("operationId", value.value()),
                w::LinkTarget::OperationRef { value, .. } => ("operationRef", value.value()),
            };
            let params = link
                .parameters()
                .iter()
                .map(|(k, v)| (k.clone(), v.value().clone()))
                .collect::<serde_json::Map<_, _>>();
            format!(
                "LinkMetadata{{{}, {}, {}, {}, detail::literal({}), {}, {}}}",
                string(link.name()),
                source_expr(plan, link.source().use_site().source()),
                string(kind),
                string(target),
                string(&serde_json::to_string(&params).unwrap()),
                link.request_body()
                    .map(|v| format!(
                        "detail::literal({})",
                        string(&serde_json::to_string(v.value()).unwrap())
                    ))
                    .unwrap_or_else(|| "std::nullopt".into()),
                link.server()
                    .map(|v| format!(
                        "detail::literal({})",
                        string(&serde_json::to_string(v).unwrap())
                    ))
                    .unwrap_or_else(|| "std::nullopt".into())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
