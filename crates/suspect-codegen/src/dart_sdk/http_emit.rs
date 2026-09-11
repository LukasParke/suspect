//! Rendering of typed HTTP/part/stream descriptors; no schema interpretation.
use super::emit::{literal, native_value, quote as q};
use super::{
    Plan, PlannedAggregate, PlannedHeaders, PlannedMedia, PlannedOperation, PlannedPart,
    PlannedPayload as P, PlannedStatus,
};
use crate::http_protocol::{self as w, ParameterSerialization as S, Representation as R};
use serde_json::{Value, json};
use std::fmt::Write;

fn source(id: &suspect_ir::contract::SourceId) -> String {
    format!(
        "SchemaSource({}, {})",
        q(id.document().as_str()),
        q(id.pointer())
    )
}
fn strings(values: impl IntoIterator<Item = String>) -> String {
    format!(
        "[{}]",
        values
            .into_iter()
            .map(|v| q(&v))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn scalar(value: w::ScalarType) -> &'static str {
    match value {
        w::ScalarType::String => "string",
        w::ScalarType::Boolean => "boolean",
        w::ScalarType::Integer => "integer",
        w::ScalarType::Number => "number",
    }
}
fn location(value: w::ParameterLocation) -> &'static str {
    match value {
        w::ParameterLocation::Path => "path",
        w::ParameterLocation::Query => "query",
        w::ParameterLocation::Querystring => "querystring",
        w::ParameterLocation::Header => "header",
        w::ParameterLocation::Cookie => "cookie",
    }
}
fn encoding(value: w::PercentEncoding) -> &'static str {
    match value {
        w::PercentEncoding::UriComponent => "component",
        w::PercentEncoding::ReservedExpansion => "reserved",
        w::PercentEncoding::None => "none",
        w::PercentEncoding::FormUrlEncoded => "form",
    }
}
fn style(value: w::Style) -> &'static str {
    match value {
        w::Style::Simple => "simple",
        w::Style::Label => "label",
        w::Style::Matrix => "matrix",
        w::Style::Form => "form",
        w::Style::SpaceDelimited => "spaceDelimited",
        w::Style::PipeDelimited => "pipeDelimited",
        w::Style::DeepObject => "deepObject",
        w::Style::Cookie => "cookie",
    }
}
fn serialization(value: &S) -> String {
    match value {
        S::Content {
            media_type,
            percent_encoding,
        } => format!(
            "_Serialization(content: true, jsonContent: {}, encoding: _Encoding.{})",
            media_type.is_json(),
            encoding(*percent_encoding)
        ),
        S::Style {
            style: s,
            explode,
            shape,
            percent_encoding,
        } => {
            let fields = match shape {
                w::WireShape::Scalar { scalar: s } => {
                    format!("shape: _Shape.scalar, scalar: _Scalar.{}", scalar(*s))
                }
                w::WireShape::Array { items } => {
                    format!("shape: _Shape.array, scalar: _Scalar.{}", scalar(*items))
                }
                w::WireShape::FlatObject {
                    properties,
                    additional,
                } => format!(
                    "shape: _Shape.object, properties: {{{}}}, {}",
                    properties
                        .iter()
                        .map(|(name, kind)| format!("{}: _Scalar.{}", q(name), scalar(*kind)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    match additional {
                        w::AdditionalScalars::Forbidden => "anyExtra: false".into(),
                        w::AdditionalScalars::AnyScalar => "anyExtra: true".into(),
                        w::AdditionalScalars::Typed(s) => format!("extra: _Scalar.{}", scalar(*s)),
                    }
                ),
            };
            format!(
                "_Serialization(style: _Style.{}, explode: {explode}, encoding: _Encoding.{}, {fields})",
                style(*s),
                encoding(*percent_encoding)
            )
        }
    }
}
fn parameter(value: &w::ParameterPlan) -> String {
    let form = value
        .content_media()
        .and_then(|m| match m.representation() {
            R::Form { form } => Some(format!(
                ", form: {}",
                form_spec(form.rules(), form.fields(), form.additional())
            )),
            _ => None,
        })
        .unwrap_or_default();
    format!(
        "_Parameter({}, _Location.{}, {}, {}{form})",
        q(value.name()),
        location(value.location()),
        value.required(),
        serialization(value.serialization())
    )
}
fn header(value: &w::HeaderPlan) -> String {
    let serialization = match value.content_media().map(w::MediaPlan::representation) {
        Some(R::Text { scalar: s, .. }) => format!(
            "_Serialization(content: true, scalar: _Scalar.{}, encoding: _Encoding.none)",
            scalar(*s)
        ),
        _ => serialization(value.serialization()),
    };
    format!(
        "_Parameter({}, _Location.header, {}, {serialization})",
        q(value.name()),
        value.required()
    )
}
fn count(value: Option<&w::Located<u64>>) -> String {
    value.map_or_else(|| "null".into(), |v| q(&v.value().to_string()))
}
fn part_spec(value: &w::PartPlan) -> String {
    let (name, fields) = match value.representation() {
        w::PartRepresentation::Json { outer_encoding, .. } => (
            "json",
            format!("encoding: _Encoding.{}", encoding(*outer_encoding)),
        ),
        w::PartRepresentation::Text {
            scalar: s,
            outer_encoding,
            ..
        } => (
            "text",
            format!(
                "scalar: _Scalar.{}, encoding: _Encoding.{}",
                scalar(*s),
                encoding(*outer_encoding)
            ),
        ),
        w::PartRepresentation::Binary { bytes } => {
            ("bytes", format!("maxBytes: {}", bytes.max_bytes()))
        }
        w::PartRepresentation::Style {
            serialization: s, ..
        } => ("style", format!("serialization: {}", serialization(s))),
    };
    format!(
        "_PartSpec({}, _PartKind.{name}, {fields}, required: {}, repeated: {}, minimum: {}, maximum: {}, contentTypes: {})",
        value.name().map_or_else(|| "null".into(), q),
        value.required(),
        value.multiplicity() == w::PartMultiplicity::RepeatedArrayItems,
        count(value.min_items()),
        count(value.max_items()),
        strings(
            value
                .content_types()
                .iter()
                .map(|m| m.declared().to_owned())
        )
    )
}
fn form_spec(rules: &w::ObjectRules, fields: &[w::PartPlan], extra: &w::AdditionalParts) -> String {
    format!(
        "_FormSpec({{{}}}, extra: {}, required: {}, minimum: {}, maximum: {})",
        fields
            .iter()
            .map(|p| format!("{}: {}", q(p.name().expect("named part")), part_spec(p)))
            .collect::<Vec<_>>()
            .join(", "),
        match extra {
            w::AdditionalParts::Forbidden => "null".into(),
            w::AdditionalParts::Allowed(p) => part_spec(p),
        },
        strings(rules.required().iter().map(|v| v.value().clone())),
        count(rules.min_properties()),
        count(rules.max_properties())
    )
}
fn aggregate_spec(value: &PlannedAggregate) -> String {
    format!(
        "_FormSpec({{{}}}, extra: {}, required: {}, minimum: {}, maximum: {})",
        value
            .fields
            .iter()
            .map(|p| format!(
                "{}: {}",
                q(p.wire.name().expect("named part")),
                part_spec(&p.wire)
            ))
            .collect::<Vec<_>>()
            .join(", "),
        value
            .extra
            .as_ref()
            .map_or_else(|| "null".into(), |p| part_spec(&p.wire)),
        strings(value.rules.required().iter().map(|v| v.value().clone())),
        count(value.rules.min_properties()),
        count(value.rules.max_properties())
    )
}
fn media(value: &PlannedMedia) -> String {
    let (kind, args) = match &value.payload {
        P::Json { .. } => ("json", String::new()),
        P::Text { scalar: s, .. } => ("text", format!(", scalar: _Scalar.{}", scalar(*s))),
        P::Bytes { max_bytes } => ("bytes", format!(", maxBytes: {max_bytes}")),
        P::Aggregate(a) => (
            if a.multipart { "multipart" } else { "form" },
            String::new(),
        ),
        P::Stream {
            framing,
            max_item_bytes,
            ..
        } => (
            if *framing == w::StreamFraming::ServerSentEvents {
                "sse"
            } else {
                "jsonl"
            },
            format!(", maxItemBytes: {max_item_bytes}"),
        ),
    };
    format!(
        "_Media({}, _MediaKind.{kind}{args})",
        q(value.wire.media_type().declared())
    )
}
fn server(value: &w::ServerPlan) -> String {
    let at = value
        .source()
        .map(|s| s.use_site().source())
        .or_else(|| value.default_from().map(w::SourceLocation::source))
        .expect("server source");
    format!(
        "ServerInfo({}, {}, variables: [{}], name: {}, description: {}, documentBase: {})",
        q(value.template()),
        source(at),
        value
            .variables()
            .iter()
            .map(|v| format!(
                "ServerVariable({}, {}, values: {})",
                q(v.name()),
                q(v.default().value()),
                strings(v.values().unwrap_or(&[]).iter().map(|v| v.value().clone()))
            ))
            .collect::<Vec<_>>()
            .join(", "),
        value.name().map_or_else(|| "null".into(), |v| q(v.value())),
        value
            .description()
            .map_or_else(|| "null".into(), |v| q(v.value())),
        source(value.document_base().source())
    )
}
fn requirement(plan: &Plan, value: &w::CredentialRequirement) -> String {
    let cred = plan
        .credentials
        .iter()
        .find(|c| &c.source == value.scheme().use_site().source())
        .expect("allocated credential");
    let (scopes, roles) = match value.permissions() {
        w::Permissions::Scopes(s) => (strings(s.iter().map(|v| v.value().clone())), "[]".into()),
        w::Permissions::Roles(r) => ("[]".into(), strings(r.iter().map(|v| v.value().clone()))),
    };
    let (kind, public, attach) = match value.credential() {
        w::CredentialHook::Bearer { .. } => ("bearer", "bearer", String::new()),
        w::CredentialHook::Basic => ("basic", "basic", String::new()),
        w::CredentialHook::ApiKey { location: l, name } => (
            "key",
            "api-key",
            format!(
                ", location: _Location.{}, wireName: {}",
                location(*l),
                q(name.value())
            ),
        ),
        w::CredentialHook::OAuth2 { .. } => ("authorization", "oauth2", String::new()),
        w::CredentialHook::OpenIdConnect { .. } => {
            ("authorization", "openid-connect", String::new())
        }
    };
    format!(
        "_Requirement(CredentialInfo({}, {}, {}, {}, {scopes}, {roles}, {}, urlBase: {}), _CredentialKind.{kind}{attach})",
        q(value.name()),
        q(&cred.name),
        q(public),
        source(value.scheme().use_site().source()),
        literal(&serde_json::to_value(value.credential()).unwrap()),
        value.credential().url_base().map_or_else(
            || "null".into(),
            |base| match base {
                w::ApiUrlBase::EffectiveServer => q("effective-server"),
                w::ApiUrlBase::ServerDocument => q("server-document"),
            }
        )
    )
}
pub(super) fn data(plan: &Plan) -> String {
    let mut out = String::new();
    for (index, op) in plan.operations.iter().enumerate() {
        writeln!(
            out,
            "final _operation{index} = _WireOperation({}, {}, {}, [{}], [{}], [{}], [{}]);",
            source(&op.source),
            q(op.wire.method().as_str()),
            q(op.wire.path()),
            op.wire
                .servers()
                .candidates()
                .iter()
                .map(server)
                .collect::<Vec<_>>()
                .join(", "),
            op.wire
                .security()
                .alternatives()
                .iter()
                .map(|alt| format!(
                    "[{}]",
                    alt.requirements()
                        .iter()
                        .map(|r| requirement(plan, r))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", "),
            op.parameters
                .iter()
                .map(|p| parameter(&p.wire))
                .collect::<Vec<_>>()
                .join(", "),
            op.statuses
                .iter()
                .map(|s| format!(
                    "_Response({}, [{}])",
                    q(s.wire.status_key()),
                    s.media.iter().map(media).collect::<Vec<_>>().join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    out
}

fn doc(out: &mut String, text: &str) {
    for line in text.replace(['\r', '\u{2028}', '\u{2029}'], " ").lines() {
        writeln!(
            out,
            "/// {}",
            line.replace('[', "&#91;")
                .replace(']', "&#93;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        )
        .unwrap();
    }
}
fn headers(out: &mut String, value: &PlannedHeaders) {
    writeln!(
        out,
        "/// Source-bound typed HTTP header values.\nfinal class {} {{",
        value.name
    )
    .unwrap();
    for h in &value.fields {
        writeln!(
            out,
            "  final {} {};",
            if h.wire.required() {
                h.native_type.clone()
            } else {
                format!("Presence<{}>", h.native_type)
            },
            h.name
        )
        .unwrap();
    }
    writeln!(
        out,
        "  const {}({{{}}});\n}}",
        value.name,
        value
            .fields
            .iter()
            .map(|h| if h.wire.required() {
                format!("required this.{}", h.name)
            } else {
                format!("this.{} = const Absent()", h.name)
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();
    writeln!(
        out,
        "// A header group is used in only one wire direction.\n// ignore: unused_element\n{} _decode{}(RawResponse raw) {{",
        value.name, value.name
    )
    .unwrap();
    for (i, h) in value.fields.iter().enumerate() {
        writeln!(out, "  final h{i} = _headerJson(raw, {});", header(&h.wire)).unwrap();
    }
    writeln!(out,"  try {{ return {}({}); }} on CodecException catch (error) {{ throw InvalidResponseException(raw, codecFailure: error); }}\n}}",value.name,
        value.fields.iter().enumerate().map(|(i,h)|format!("{}: {}",h.name,if h.wire.required(){format!("{}.fromJson(h{i}!)",h.codec_name)}else{format!("h{i} == null ? const Absent() : Present({}.fromJson(h{i}))",h.codec_name)})).collect::<Vec<_>>().join(", ")).unwrap();
    writeln!(out,"// ignore: unused_element\nMap<String,String> _encode{}({} value, int maximum) {{\n  final result = <String,String>{{}};",value.name,value.name).unwrap();
    for (i, h) in value.fields.iter().enumerate() {
        let v = if h.wire.required() {
            format!("value.{}", h.name)
        } else {
            format!("h{i}.value")
        };
        if !h.wire.required() {
            writeln!(
                out,
                "  final h{i} = value.{}; if (h{i} is Present<{}>) {{",
                h.name, h.native_type
            )
            .unwrap();
        }
        writeln!(
            out,
            "  result[{}] = _serialize({}, {}.toJson({v}), maximum);",
            q(h.wire.name()),
            header(&h.wire),
            h.codec_name
        )
        .unwrap();
        if !h.wire.required() {
            out.push_str("  }\n");
        }
    }
    out.push_str("  return result;\n}\n");
}
fn part_value(part: &PlannedPart, expr: &str) -> String {
    let value = if part.wrapper_name.is_some() {
        format!("{expr}.value")
    } else {
        expr.into()
    };
    let value = part.codec_name.as_ref().map_or_else(
        || {
            format!(
                "_partRaw({value}, c, {})",
                match part.wire.representation() {
                    w::PartRepresentation::Binary { bytes } => bytes.max_bytes(),
                    _ => 0,
                }
            )
        },
        |codec| format!("{codec}._toJsonWith({value}, c)"),
    );
    let extra = if part.wrapper_name.is_some() {
        format!(
            ", contentType: {expr}.contentType, filename: {expr}.filename{}",
            part.headers.as_ref().map_or_else(String::new, |h| format!(
                ", headers: _encode{}({expr}.headers, maximum)",
                h.name
            ))
        )
    } else {
        String::new()
    };
    format!("_PartData({value}{extra})")
}
fn aggregate(out: &mut String, value: &PlannedAggregate) {
    for part in value
        .fields
        .iter()
        .chain(value.extra.iter().map(Box::as_ref))
    {
        if let Some(h) = &part.headers {
            headers(out, h);
        }
        if let Some(name) = &part.wrapper_name {
            writeln!(out,"/// A typed part with explicit content type/header metadata.\nfinal class {name} {{\n  {} value;\n  final String? contentType;\n  final String? filename;",part.value_type).unwrap();
            if let Some(h) = &part.headers {
                writeln!(out, "  final {} headers;", h.name).unwrap();
            }
            writeln!(
                out,
                "  {name}({{required this.value, this.contentType, this.filename{}}});\n}}",
                if part.headers.is_some() {
                    ", required this.headers"
                } else {
                    ""
                }
            )
            .unwrap();
        }
    }
    writeln!(out,"/// Native fields; bytes are never replaced with JSON null during validation.\nfinal class {} {{",value.name).unwrap();
    for p in &value.fields {
        writeln!(
            out,
            "  {} {};",
            if p.wire.required() {
                p.native_type.clone()
            } else {
                format!("Presence<{}>", p.native_type)
            },
            p.name
        )
        .unwrap();
    }
    if let Some(extra) = &value.extra {
        writeln!(
            out,
            "  final Map<String,{}> extraFields;",
            extra.native_type
        )
        .unwrap();
    }
    writeln!(out, "  {}({{", value.name).unwrap();
    for p in &value.fields {
        writeln!(
            out,
            "    {},",
            if p.wire.required() {
                format!("required this.{}", p.name)
            } else {
                format!("this.{} = const Absent()", p.name)
            }
        )
        .unwrap();
    }
    if let Some(extra) = &value.extra {
        writeln!(
            out,
            "    Map<String,{}>? extraFields,\n  }}) : extraFields = Map.of(extraFields ?? {{}});",
            extra.native_type
        )
        .unwrap();
    } else if value.fields.is_empty() {
        let start = out.rfind(&format!("  {}({{\n", value.name)).unwrap();
        out.truncate(start);
        writeln!(out, "  {}();", value.name).unwrap();
    } else {
        out.push_str("  });\n");
    }
    out.push_str("}\n");
    writeln!(out,"_BodyContent _encode{}({} value, String contentType, int maximum) {{\n  final fields = <String,List<_PartData>>{{}};",value.name,value.name).unwrap();
    if !value.fields.is_empty() || value.extra.is_some() {
        writeln!(
            out,
            "  final c = _Conversion({});\n  var partCount=0;",
            source(value.rules.schema().id())
        )
        .unwrap();
    }
    for (i, p) in value.fields.iter().enumerate() {
        let expr = if p.wire.required() {
            format!("value.{}", p.name)
        } else {
            format!("v{i}.value")
        };
        if !p.wire.required() {
            writeln!(
                out,
                "  final v{i}=value.{}; if (v{i} is Present<{}>) {{",
                p.name, p.native_type
            )
            .unwrap();
        }
        let items = if p.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(
                out,
                "  partCount=_partCount(partCount,{expr}.length,maximum);"
            )
            .unwrap();
            format!("[for (final item in {expr}) {}]", part_value(p, "item"))
        } else {
            writeln!(out, "  partCount=_partCount(partCount,1,maximum);").unwrap();
            format!("[{}]", part_value(p, &expr))
        };
        writeln!(out, "  fields[{}] = {items};", q(p.wire.name().unwrap())).unwrap();
        if !p.wire.required() {
            out.push_str("  }\n");
        }
    }
    if let Some(p) = &value.extra {
        writeln!(out,"  for (final entry in value.extraFields.entries) {{\n    c.string(entry.key);\n    if (const <String>[{}].contains(entry.key)) {{ throw const ConfigurationException('extra part shadows declared field'); }}",value.fields.iter().map(|p|q(p.wire.name().unwrap())).collect::<Vec<_>>().join(", ")).unwrap();
        let items = if p.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(
                out,
                "  partCount=_partCount(partCount,entry.value.length,maximum);"
            )
            .unwrap();
            format!(
                "[for (final item in entry.value) {}]",
                part_value(p, "item")
            )
        } else {
            writeln!(out, "  partCount=_partCount(partCount,1,maximum);").unwrap();
            format!("[{}]", part_value(p, "entry.value"))
        };
        writeln!(out, "    fields[entry.key] = {items};\n  }}").unwrap();
    }
    writeln!(
        out,
        "  return _aggregateBody({}, fields, contentType, maximum, multipart: {});\n}}",
        aggregate_spec(value),
        value.multipart
    )
    .unwrap();
}
fn link(value: &w::LinkPlan) -> String {
    let (target, reference) = match value.target() {
        w::LinkTarget::OperationId { value, .. } => (value.value(), false),
        w::LinkTarget::OperationRef { value, .. } => (value.value(), true),
    };
    format!(
        "LinkMetadata(name: {}, target: {}, byReference: {reference}, source: {}, parameters: {}, requestBody: {}, server: {})",
        q(value.name()),
        q(target),
        source(value.source().use_site().source()),
        literal(&json!(
            value
                .parameters()
                .iter()
                .map(|(k, v)| (k, v.value()))
                .collect::<std::collections::BTreeMap<_, _>>()
        )),
        value.request_body().map_or_else(
            || "const Absent()".into(),
            |v| format!("Present({})", literal(v.value()))
        ),
        value.server().map_or_else(|| "null".into(), server)
    )
}
fn body_expression(media: &PlannedMedia, value: &str, content: &str) -> String {
    let bytes = match &media.payload {
        P::Json { codec, .. } => format!(
            "_jsonBytes({}, _maxRequestBytes)",
            codec
                .as_ref()
                .map_or(value.into(), |c| format!("{c}.toJson({value})"))
        ),
        P::Text { codec, .. } => format!(
            "_textBytes({}, _maxRequestBytes)",
            codec.as_ref().map_or(value.into(), |c| format!(
                "_scalarText({c}.toJson({value}))"
            ))
        ),
        P::Bytes { max_bytes } => format!(
            "_bytes({value}, _maxRequestBytes < {max_bytes} ? _maxRequestBytes : {max_bytes})"
        ),
        P::Aggregate(a) => {
            return format!("_encode{}({value}, {content}, _maxRequestBytes)", a.name);
        }
        P::Stream { .. } => unreachable!("request streams refused"),
    };
    format!("_BodyContent({bytes}, {content})")
}
fn decode_expression(media: &PlannedMedia) -> String {
    match &media.payload {
        P::Json {
            codec: Some(codec), ..
        } => format!("_decoded(response, () => {codec}.decodeBytes(response.bytes))"),
        P::Json { codec: None, .. } => "_responseJson(response)".into(),
        P::Text {
            codec: Some(codec),
            scalar: s,
            ..
        } => format!(
            "_decoded(response, () => {codec}.fromJson(_textJson(_responseText(response), _Scalar.{})))",
            scalar(*s)
        ),
        P::Text { codec: None, .. } => "_responseText(response)".into(),
        P::Bytes { .. } => "response.bytes.asUnmodifiableView()".into(),
        P::Stream { codec, .. } => format!("_decoded(response, () => {codec}.fromJson(item!))"),
        P::Aggregate(_) => unreachable!("response aggregate refused"),
    }
}
fn result_type(op: &PlannedOperation) -> String {
    let names = op
        .statuses
        .iter()
        .filter_map(|s| s.success_name.as_ref())
        .collect::<Vec<_>>();
    if let [name] = names.as_slice() {
        (*name).clone()
    } else {
        op.success_type.clone()
    }
}
pub(super) fn client(plan: &Plan) -> String {
    let mut out = String::from(if plan.credential_env().is_some() {
        "// Some selected status families have no concrete error alternatives.\n// ignore_for_file: unused_element_parameter\n/// Source-named values. Explicit credentials override the whole environment snapshot.\nfinal class Credentials {\n"
    } else {
        "// Some selected status families have no concrete error alternatives.\n// ignore_for_file: unused_element_parameter\n/// Source-named runtime credentials; omitted values are never acquired implicitly.\nfinal class Credentials {\n"
    });
    for c in &plan.credentials {
        writeln!(out, "  final {} {};", c.native_type, c.name).unwrap();
    }
    if plan.credentials.is_empty() {
        out.push_str("  const Credentials();\n");
    } else {
        writeln!(
            out,
            "  const Credentials({{{}}});",
            plan.credentials
                .iter()
                .map(|c| format!("this.{}", c.name))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    out.push_str("  Object? _get(String name) => switch(name) {\n");
    for c in &plan.credentials {
        writeln!(out, "    {} => {},", q(&c.name), c.name).unwrap();
    }
    out.push_str(
        "    _ => null,\n  };\n  @override String toString() => 'Credentials(redacted)';\n}\n",
    );
    for (op_index, op) in plan.operations.iter().enumerate() {
        if let Some(body) = &op.body {
            for m in &body.media {
                if let P::Aggregate(a) = &m.payload {
                    aggregate(&mut out, a);
                }
            }
            if let Some(choice) = &body.choice_name {
                writeln!(out, "sealed class {choice} {{ const {choice}._(); }}").unwrap();
                for m in &body.media {
                    let mandatory =
                        !matches!(m.wire.media_type().range(), w::MediaRange::Concrete { .. });
                    writeln!(out,"final class {} extends {choice} {{\n  final {} value;\n  final String contentType;\n  {}(this.value, {{ {} }}) : super._();\n}}",m.variant_name,m.native_type,m.variant_name,
                        if mandatory{"required this.contentType".into()}else{format!("this.contentType = {}",q(m.wire.media_type().declared()))}).unwrap();
                }
            }
        }
        doc(
            &mut out,
            &format!("Success alternatives for {}.", op.operation_id),
        );
        writeln!(out,"sealed class {} extends SdkResponse {{ const {}._(super.response, {{super.links}}); }}",op.success_type,op.success_type).unwrap();
        writeln!(out,"sealed class {} extends ApiException {{ const {}._(super.response, {{super.links}}); }}",op.error_type,op.error_type).unwrap();
        for (status_index, s) in op.statuses.iter().enumerate() {
            if let Some(h) = &s.headers {
                headers(&mut out, h);
            }
            if let Some(choice) = &s.choice_name {
                writeln!(out, "sealed class {choice} {{ const {choice}._(); }}").unwrap();
                for m in &s.media {
                    writeln!(out,"final class {} extends {choice} {{ const {}._(this.value) : super._(); final {} value; }}",m.variant_name,m.variant_name,m.native_type).unwrap();
                }
                if let Some(n) = &s.none_variant {
                    writeln!(
                        out,
                        "final class {n} extends {choice} {{ const {n}._() : super._(); }}"
                    )
                    .unwrap();
                }
                if let Some(n) = &s.bytes_variant {
                    writeln!(out,"final class {n} extends {choice} {{ const {n}._(this.value) : super._(); final Uint8List value; }}").unwrap();
                }
            }
            for (name, parent) in [
                (s.success_name.as_ref(), &op.success_type),
                (s.error_name.as_ref(), &op.error_type),
            ] {
                if let Some(name) = name {
                    writeln!(out,"/// Exact actual status is available through status; this class matches {}.\nfinal class {name} extends {parent} {{\n  final {} data;",s.wire.status_key(),s.native_type).unwrap();
                    if let Some(h) = &s.headers {
                        writeln!(out, "  final {} headers;", h.name).unwrap();
                    }
                    writeln!(
                        out,
                        "  {name}._(this.data, RawResponse raw{}) : super._(raw, links: [{}]);\n}}",
                        if s.headers.is_some() {
                            ", this.headers"
                        } else {
                            ""
                        },
                        s.wire
                            .links()
                            .iter()
                            .map(link)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap();
                }
            }
            response_decoder(&mut out, op_index, status_index, s);
        }
        writeln!(out,"{} _decodeOperation{op_index}(_Received response, [JsonValue? item]) {{\n  switch(response.responseIndex) {{",result_type(op)).unwrap();
        for (i, s) in op.statuses.iter().enumerate() {
            if s.native_type == "Never" {
                writeln!(
                    out,
                    "    case {i}: return _payload{op_index}_{i}(response,item);"
                )
                .unwrap();
                continue;
            }
            writeln!(
                out,
                "    case {i}:\n      final data = _payload{op_index}_{i}(response,item);"
            )
            .unwrap();
            let h = s.headers.as_ref().map_or_else(String::new, |h| {
                format!(", _decode{}(response.raw)", h.name)
            });
            if let Some(name) = &s.success_name {
                writeln!(out,"      if(response.status >= 200 && response.status < 300) {{ return {name}._(data,response.raw{h}); }}").unwrap();
            }
            if let Some(name) = &s.error_name {
                writeln!(out, "      throw {name}._(data,response.raw{h});").unwrap();
            } else {
                out.push_str("      throw UnexpectedResponseException(response.raw);\n");
            }
        }
        out.push_str("    default: throw UnexpectedResponseException(response.raw);\n  }\n}\n");
    }
    let c = &plan.config;
    let (credentials_parameter, credentials_value) = if plan.credential_env().is_some() {
        (
            "Credentials credentials = const _OmittedCredentials(), String? Function(String)? environment",
            "credentials is _OmittedCredentials ? _credentialsFromEnvironment(environment ?? _environment.readVariable,maxRequestBytes) : credentials",
        )
    } else {
        (
            "Credentials credentials = const Credentials()",
            "credentials",
        )
    };
    writeln!(out,"/// Native Future and lazy single-subscription Stream operations.\nfinal class Client extends _ClientBase {{\n  Client({{required HttpTransport transport, {credentials_parameter}, Uri? server, ServerSelection? serverSelection, Duration timeout = const Duration(seconds:30), int maxRequestBytes = {}, int maxResponseBytes = {}, int maxCaptureBytes = {}, int maxResponseHeaderBytes = {}, int maxStreamBufferBytes = {}}}) : super(transport,{credentials_value},serverSelection ?? ServerSelection(override:server),timeout,maxRequestBytes,maxResponseBytes,maxCaptureBytes,maxResponseHeaderBytes,maxStreamBufferBytes,[{},{},{},{},{}]);",c.max_request_bytes,c.max_response_bytes,c.max_capture_bytes,c.max_response_header_bytes,c.max_stream_buffer_bytes,c.max_request_bytes,c.max_response_bytes,c.max_capture_bytes,c.max_response_header_bytes,c.max_stream_buffer_bytes).unwrap();
    for (i, op) in plan.operations.iter().enumerate() {
        doc(
            &mut out,
            op.wire
                .description()
                .map_or(&op.operation_id, |v| v.value()),
        );
        writeln!(out, "  {} {}({{", op.return_type, op.method_name).unwrap();
        for p in &op.parameters {
            writeln!(
                out,
                "    {},",
                if p.required {
                    format!("required {} {}", p.native_type, p.name)
                } else {
                    format!("Presence<{}> {} = const Absent()", p.native_type, p.name)
                }
            )
            .unwrap();
        }
        if let Some(body) = &op.body {
            writeln!(
                out,
                "    {},",
                if body.required {
                    format!("required {} body", body.native_type)
                } else {
                    format!("Presence<{}> body = const Absent()", body.native_type)
                }
            )
            .unwrap();
        }
        out.push_str("    CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative,\n  })");
        if !op.stream {
            out.push_str(" async");
        }
        out.push_str(" {\n    _RequestInput prepare() {\n      final inputs = <_InputValue>[];\n");
        for (n, p) in op.parameters.iter().enumerate() {
            if !p.required {
                writeln!(
                    out,
                    "      if ({} is Present<{}>) {{",
                    p.name, p.native_type
                )
                .unwrap();
            }
            writeln!(
                out,
                "      inputs.add(_InputValue(_operation{i}.parameters[{n}], {}.toJson({})));",
                p.codec_name,
                if p.required {
                    p.name.clone()
                } else {
                    format!("{}.value", p.name)
                }
            )
            .unwrap();
            if !p.required {
                out.push_str("      }\n");
            }
        }
        if let Some(body) = &op.body {
            let value = if body.required { "body" } else { "body.value" };
            if !body.required {
                writeln!(
                    out,
                    "      if (body is! Present<{}>) {{ return _RequestInput(inputs,null); }}",
                    body.native_type
                )
                .unwrap();
            }
            if body.choice_name.is_some() {
                writeln!(
                    out,
                    "      final selected = {value};\n      switch (selected) {{"
                )
                .unwrap();
                for (n, m) in body.media.iter().enumerate() {
                    writeln!(out,"        case {}():\n          if (_selectMedia([{}], selected.contentType) != {n}) {{ throw const ConfigurationException('body representation cannot bypass a more specific content declaration'); }}\n          return _RequestInput(inputs, {});",m.variant_name,body.media.iter().map(media).collect::<Vec<_>>().join(", "),body_expression(m,"selected.value","selected.contentType")).unwrap();
                }
                out.push_str("      }\n");
            } else {
                writeln!(
                    out,
                    "      return _RequestInput(inputs, {});",
                    body_expression(
                        &body.media[0],
                        value,
                        &q(body.media[0].wire.media_type().declared())
                    )
                )
                .unwrap();
            }
        } else {
            out.push_str("      return _RequestInput(inputs,null);\n");
        }
        out.push_str("    }\n");
        if op.stream {
            writeln!(out,"    return _stream(_operation{i},prepare,(frame)=>_decodeOperation{i}(frame.received,frame.item),cancellation,timeout,server,securityAlternative);\n  }}").unwrap();
        } else {
            writeln!(out,"    final response = await _exchange(_operation{i},prepare,cancellation,timeout,server,securityAlternative);\n    return _decodeOperation{i}(response);\n  }}").unwrap();
        }
    }
    out.push_str("}\n");
    out
}
fn response_decoder(out: &mut String, op: usize, index: usize, status: &PlannedStatus) {
    writeln!(
        out,
        "{} _payload{op}_{index}(_Received response, JsonValue? item) {{",
        status.native_type
    )
    .unwrap();
    if let Some(choice) = &status.choice_name {
        if let Some(name) = &status.none_variant {
            writeln!(out, "  if(response.noBody) {{ return const {name}._(); }}").unwrap();
        }
        if let Some(name) = &status.bytes_variant {
            writeln!(
                out,
                "  return {name}._(response.bytes.asUnmodifiableView());"
            )
            .unwrap();
        } else {
            out.push_str("  switch(response.mediaIndex) {\n");
            for (n, m) in status.media.iter().enumerate() {
                writeln!(
                    out,
                    "    case {n}: return {};",
                    if m.native_type == "Never" {
                        decode_expression(m)
                    } else {
                        format!("{}._({})", m.variant_name, decode_expression(m))
                    }
                )
                .unwrap();
            }
            writeln!(out,"    default: throw MediaTypeException(response.raw);\n  }}\n  // {choice} has no undeclared payload coercion.").unwrap();
        }
    } else if status.native_type == "NoBody" {
        out.push_str("  return const NoBody();\n");
    } else if status.media.is_empty() {
        out.push_str("  return response.bytes.asUnmodifiableView();\n");
    } else {
        writeln!(out, "  return {};", decode_expression(&status.media[0])).unwrap();
    }
    out.push_str("}\n");
}

pub(super) fn examples(plan: &Plan) -> String {
    let name = &plan.config.package.name;
    let used = plan.examples.operations().iter().any(|op| {
        op.entries
            .iter()
            .any(|e| plan.models.model(&e.schema).is_some())
    });
    let mut out = if used {
        format!("import 'package:{name}/{name}.dart';\n")
    } else {
        String::new()
    };
    if plan.credential_env().is_some() && plan.models.symbols().iter().any(|m| m.deprecated) {
        out.insert_str(0,"// Source-valid examples construct source-required deprecated types.\n// ignore_for_file: deprecated_member_use, deprecated_member_use_from_same_package\n");
    }
    out.push_str("void main() {\n  var checked=0;\n");
    for op in plan.examples.operations() {
        for e in &op.entries {
            if let Some(model) = plan.models.model(&e.schema) {
                writeln!(
                    out,
                    "  // {:?} {:?}\n  {}.encode({});\n  checked++;",
                    e.origin,
                    e.role,
                    model.codec_name,
                    native_value(plan, model.index, &e.value)
                )
                .unwrap();
            }
        }
    }
    out.push_str("  print('Validated $checked source model examples.');\n}\n");
    out
}
pub(super) fn quickstart(plan: &Plan) -> String {
    let name = &plan.config.package.name;
    for op in &plan.operations {
        if op.stream {
            continue;
        }
        let Some(examples) = plan
            .examples
            .operations()
            .iter()
            .find(|e| e.source == op.source)
        else {
            continue;
        };
        let mut args = Vec::new();
        let mut missing = false;
        for p in &op.parameters {
            if let Some(e) = examples.entries.iter().find(|e| e.container == p.source) {
                args.push(format!(
                    "{}: {}",
                    p.name,
                    if p.required {
                        native_value(plan, plan.models.model(&p.schema).unwrap().index, &e.value)
                    } else {
                        format!(
                            "Present({})",
                            native_value(
                                plan,
                                plan.models.model(&p.schema).unwrap().index,
                                &e.value
                            )
                        )
                    }
                ));
            } else if p.required {
                missing = true;
            }
        }
        if let Some(body) = &op.body {
            let found = body.media.iter().find_map(|m| {
                let id = match &m.payload {
                    P::Json { schema, .. } | P::Text { schema, .. } => schema.as_ref(),
                    _ => None,
                }?;
                let e = examples.entries.iter().find(|e| {
                    e.container == *m.wire.source().use_site().source()
                        && e.role == crate::examples::ExampleRole::RequestBody
                })?;
                let value = native_value(plan, plan.models.model(id)?.index, &e.value);
                Some(if body.choice_name.is_some() {
                    format!("{}({value})", m.variant_name)
                } else {
                    value
                })
            });
            if let Some(value) = found {
                args.push(format!(
                    "body: {}",
                    if body.required {
                        value
                    } else {
                        format!("Present({value})")
                    }
                ));
            } else if body.required {
                missing = true;
            }
        }
        if missing {
            continue;
        }
        let credential = op
            .wire
            .security()
            .alternatives()
            .iter()
            .find(|a| a.requirements().len() == 1)
            .and_then(|a| a.requirements().first())
            .and_then(|r| {
                plan.credentials
                    .iter()
                    .find(|c| &c.source == r.scheme().use_site().source())
            });
        let credentials = credential
            .filter(|c| c.native_type == "String?")
            .map_or_else(
                || "const Credentials()".into(),
                |c| {
                    format!(
                        "Credentials({}: host.Platform.environment['API_TOKEN'])",
                        c.name
                    )
                },
            );
        let credentials_argument = if plan.credential_env().is_some() {
            String::new()
        } else {
            format!("credentials:{credentials},")
        };
        return format!("import 'dart:io' as host show Platform;\nimport 'package:{name}/{name}_io.dart';\nFuture<void> main() async {{\n  final override=host.Platform.environment['API_SERVER'];\n  final client=Client(transport:IoTransport(),{credentials_argument}server:override==null?null:Uri.parse(override));\n  final cancellation=CancellationToken();\n  try {{\n    final response=await client.{}({},cancellation:cancellation,timeout:const Duration(seconds:20));\n    print('HTTP ${{response.status}}');\n  }} finally {{ await client.close(); }}\n}}\n",op.method_name,args.join(", ")).replace("(,cancellation:","(cancellation:");
    }
    if plan.program.version == suspect_schema::OwnedProgram::V2_VERSION {
        "// This selection needs a schema-valid declared input/byte recipe; see the manifest.\nvoid main() {}\n".into()
    } else {
        "// This selection needs caller-provided byte/credential examples; see the manifest.\nvoid main() {}\n".into()
    }
}
pub(super) fn reference(plan: &Plan) -> String {
    let mut out = String::from("# Dart operation reference\n\n");
    for op in &plan.operations {
        writeln!(
            out,
            "## `{}`\n\n`{} {}` → `{}`\n\nSource: `{}#{}`\n",
            op.method_name,
            op.wire.method().as_str(),
            op.wire.path(),
            op.return_type,
            op.source.document(),
            op.source.pointer()
        )
        .unwrap();
        for s in &op.statuses {
            writeln!(
                out,
                "- `{}`: success {:?}, error {:?}, data `{}`",
                s.wire.status_key(),
                s.success_name,
                s.error_name,
                s.native_type
            )
            .unwrap();
        }
    }
    out
}
pub(super) fn manifest(plan: &Plan) -> String {
    let operations=plan.operations.iter().map(|op|json!({"operation_id":op.operation_id,"method":op.method_name,"return_type":op.return_type,"stream":op.stream,
        "source":{"document":op.source.document().to_string(),"pointer":op.source.pointer()},"success":op.success_type,"error":op.error_type,
        "body":op.body.as_ref().map(|b|json!({"type":b.native_type,"required":b.required,"media":b.media.iter().map(|m|json!({"media":m.wire.media_type().declared(),"type":m.native_type,"variant":m.variant_name})).collect::<Vec<_>>()})),
        "statuses":op.statuses.iter().map(|s|json!({"pattern":s.wire.status_key(),"success":s.success_name,"error":s.error_name,"type":s.native_type})).collect::<Vec<_>>()})).collect::<Vec<_>>();
    let mut manifest = json!({"version":"suspect.dart-sdk.v2","package":{"name":plan.config.package.name,"version":plan.config.package.version},
        "operations":operations,"models":plan.models.symbols().iter().map(|m|json!({"name":m.name,"codec":m.codec_name,"type":plan.models.ty(m.index),"source":{"document":m.source.document().to_string(),"pointer":m.source.pointer()}})).collect::<Vec<_>>(),
        "protocol":plan.protocol,"automatic_retries":false,"inferred_pagination":false,
        "examples":serde_json::from_str::<Value>(&crate::http_examples::manifest(&plan.examples)).unwrap(),
        "native_gate_toolchains":["3.9.4","3.13.3"]});
    if let Some(policy) = plan.credential_env() {
        manifest["credential_env"] = serde_json::to_value(policy).unwrap();
    }
    format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap())
}
