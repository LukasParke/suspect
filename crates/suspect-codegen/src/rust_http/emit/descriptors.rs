use super::*;
use wire::*;

pub(super) fn source(id: &SourceId) -> String {
    format!(
        "crate::http::Source {{ document: {:?}, pointer: {:?} }}",
        id.document().as_str(),
        id.pointer()
    )
}
fn provenance(p: &Provenance) -> String {
    format!(
        "crate::http::Provenance {{ use_site:{},terminal:{},references:&[{}],use_site_resource:{},terminal_resource:{},reference_resources:&[{}] }}",
        source(p.use_site().source()),
        source(p.terminal().source()),
        p.references()
            .iter()
            .map(|s| source(s.source()))
            .collect::<Vec<_>>()
            .join(","),
        option(p.use_site_resource(), resource_context),
        option(p.terminal_resource(), resource_context),
        p.reference_resources()
            .iter()
            .map(|context| option(context.as_ref(), resource_context))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn resource_context(context: &ResourceContext) -> String {
    format!(
        "crate::http::ResourceContext {{ source:{},resource:{},kind:crate::http::ResourceKind::{:?},canonical_uri:{:?},base_uri:{:?},base_source:{},scope_address:{:?},schema_root:{},aliases:&{:?} }}",
        source(context.source().source()),
        source(context.resource().source()),
        context.kind(),
        context.canonical_uri(),
        context.base_uri(),
        option(context.base_source(), |value| source(value.source())),
        context.scope_address(),
        option(context.schema_root(), |value| source(value.source())),
        context.aliases()
    )
}
fn text(v: &Located<String>) -> String {
    format!(
        "crate::http::LocatedText {{ source:{},value:{:?} }}",
        source(v.source().source()),
        v.value()
    )
}
fn optional_text(v: Option<&Located<String>>) -> String {
    v.map(|v| format!("std::option::Option::Some({})", text(v)))
        .unwrap_or_else(|| "std::option::Option::None".into())
}
fn option<T>(v: Option<T>, emit: impl FnOnce(T) -> String) -> String {
    v.map(|v| format!("std::option::Option::Some({})", emit(v)))
        .unwrap_or_else(|| "std::option::Option::None".into())
}
pub(super) fn scalar(s: ScalarType) -> String {
    format!("crate::http::ScalarType::{s:?}")
}
fn shape(s: &WireShape) -> String {
    match s {
        WireShape::Scalar { scalar: s } => format!("crate::http::Shape::Scalar({})", scalar(*s)),
        WireShape::Array { items } => format!("crate::http::Shape::Array({})", scalar(*items)),
        WireShape::FlatObject {
            properties,
            additional,
        } => format!(
            "crate::http::Shape::Object {{properties:&[{}],additional:{}}}",
            properties
                .iter()
                .map(|(k, v)| format!("({k:?},{})", scalar(*v)))
                .collect::<Vec<_>>()
                .join(","),
            match additional {
                AdditionalScalars::Forbidden => "crate::http::AdditionalScalars::Forbidden".into(),
                AdditionalScalars::AnyScalar => "crate::http::AdditionalScalars::AnyScalar".into(),
                AdditionalScalars::Typed(s) =>
                    format!("crate::http::AdditionalScalars::Typed({})", scalar(*s)),
            }
        ),
    }
}

pub(super) fn serialization(
    plan: &HttpPlan,
    s: &ParameterSerialization,
    media: Option<&MediaPlan>,
) -> String {
    if let Some(Representation::Form { form }) = media.map(MediaPlan::representation) {
        return format!(
            "crate::http::Serialization::Form {{spec:&{}}}",
            form_descriptor(plan, form)
        );
    }
    match s {
        ParameterSerialization::Style {
            style,
            explode,
            shape: s,
            percent_encoding,
        } => format!(
            "crate::http::Serialization::Style {{style:crate::http::Style::{style:?},explode:{explode},shape:{},encoding:crate::http::PercentEncoding::{percent_encoding:?}}}",
            shape(s)
        ),
        ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => format!(
            "crate::http::Serialization::Content {{json:{},scalar:{},encoding:crate::http::PercentEncoding::{percent_encoding:?}}}",
            media_type.is_json(),
            scalar(match media.map(MediaPlan::representation) {
                Some(Representation::Text { scalar, .. }) => *scalar,
                _ => ScalarType::String,
            })
        ),
    }
}
pub(super) fn parameter(plan: &HttpPlan, p: &ParameterPlan) -> String {
    format!(
        "crate::http::Parameter {{name:{:?},source:{},location:crate::http::ParameterLocation::{:?},required:{},serialization:{}}}",
        p.name(),
        source(p.source().use_site().source()),
        p.location(),
        p.required(),
        serialization(plan, p.serialization(), p.content_media())
    )
}
pub(super) fn header(plan: &HttpPlan, h: &HeaderPlan) -> String {
    format!(
        "crate::http::Parameter {{name:{:?},source:{},location:crate::http::ParameterLocation::Header,required:{},serialization:{}}}",
        h.name(),
        source(h.source().use_site().source()),
        h.required(),
        serialization(plan, h.serialization(), h.content_media())
    )
}

pub(super) fn media(m: &MediaType, at: &SourceId, kind: &str) -> String {
    let range = match m.range() {
        MediaRange::Any => "crate::http::MediaRange::Any".into(),
        MediaRange::Type { type_name } => format!("crate::http::MediaRange::Type({type_name:?})"),
        MediaRange::Concrete { type_name, subtype } => {
            format!("crate::http::MediaRange::Concrete({type_name:?},{subtype:?})")
        }
    };
    format!(
        "crate::http::Media {{source:{},declared:{:?},range:{range},parameters:&[{}],kind:crate::http::MediaKind::{kind}}}",
        source(at),
        m.declared(),
        m.parameters()
            .iter()
            .map(|(k, v)| format!("({k:?},{v:?})"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn media_plan(m: &MediaPlan) -> String {
    media(
        m.media_type(),
        m.source().use_site().source(),
        match m.representation() {
            Representation::Json { .. } => "Json",
            Representation::Text { .. } => "Text",
            Representation::Binary { .. } => "Binary",
            Representation::Form { .. } => "Form",
            Representation::Multipart { .. } => "Multipart",
            Representation::Stream { .. } => "Stream",
        },
    )
}
fn server(s: &ServerPlan) -> String {
    let at = s
        .source()
        .map(|s| s.use_site())
        .or(s.default_from())
        .expect("server provenance")
        .source();
    let variables=s.variables().iter().map(|v|format!("crate::http::ServerVariable {{source:{},name:{:?},default:{},values:&[{}],description:{}}}",source(v.source().use_site().source()),v.name(),text(v.default()),v.values().unwrap_or(&[]).iter().map(text).collect::<Vec<_>>().join(","),optional_text(v.description()))).collect::<Vec<_>>().join(",");
    format!(
        "crate::http::Server {{source:{},document_base:{},provenance:{},template:{:?},name:{},description:{},variables:&[{variables}]}}",
        source(at),
        source(s.document_base().source()),
        option(s.source(), provenance),
        s.template(),
        optional_text(s.name()),
        optional_text(s.description())
    )
}
fn requirement(r: &CredentialRequirement) -> String {
    let permissions = match r.permissions() {
        Permissions::Scopes(v) => format!(
            "crate::http::Permissions::Scopes(&[{}])",
            v.iter().map(text).collect::<Vec<_>>().join(",")
        ),
        Permissions::Roles(v) => format!(
            "crate::http::Permissions::Roles(&[{}])",
            v.iter().map(text).collect::<Vec<_>>().join(",")
        ),
    };
    let kind = match r.credential() {
        CredentialHook::Bearer { bearer_format } => format!(
            "crate::http::CredentialKind::Bearer {{format:{}}}",
            optional_text(bearer_format.as_ref())
        ),
        CredentialHook::Basic => "crate::http::CredentialKind::Basic".into(),
        CredentialHook::ApiKey { location, name } => format!(
            "crate::http::CredentialKind::ApiKey {{location:crate::http::ParameterLocation::{location:?},name:{}}}",
            text(name)
        ),
        CredentialHook::OpenIdConnect { discovery_url } => format!(
            "crate::http::CredentialKind::OpenIdConnect {{discovery_url:{}}}",
            text(discovery_url)
        ),
        CredentialHook::OAuth2 {
            flows,
            metadata_url,
        } => {
            let flows=flows.iter().map(|f|format!("crate::http::OAuthFlow {{source:{},kind:crate::http::OAuthFlowKind::{:?},authorization_url:{},token_url:{},refresh_url:{},device_authorization_url:{},scopes:&[{}]}}",source(f.source().source()),f.kind(),optional_text(f.authorization_url()),optional_text(f.token_url()),optional_text(f.refresh_url()),optional_text(f.device_authorization_url()),f.scopes().iter().map(|(k,v)|format!("({k:?},{})",text(v))).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(",");
            format!(
                "crate::http::CredentialKind::OAuth2 {{flows:&[{flows}],metadata_url:{}}}",
                optional_text(metadata_url.as_ref())
            )
        }
    };
    format!(
        "crate::http::CredentialRequirement {{source:{},name:{:?},scheme:{},permissions:{permissions},kind:{kind},description:{}}}",
        source(r.source().source()),
        r.name(),
        provenance(r.scheme()),
        optional_text(r.description())
    )
}
fn security(s: &SecurityPlan) -> String {
    match s {
        SecurityPlan::NoAuth { source: s } => {
            format!("crate::http::Security::NoAuth({})", source(s.source()))
        }
        SecurityPlan::Undeclared { source: s } => {
            format!("crate::http::Security::Undeclared({})", source(s.source()))
        }
        SecurityPlan::Alternatives { alternatives, .. } => format!(
            "crate::http::Security::Alternatives(&[{}])",
            alternatives
                .iter()
                .map(|a| format!(
                    "crate::http::SecurityAlternative {{source:{},requirements:&[{}]}}",
                    source(a.source().source()),
                    a.requirements()
                        .iter()
                        .map(requirement)
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn metadata(v: &Value) -> String {
    match v {
        Value::Null => "crate::http::MetadataValue::Null".into(),
        Value::Bool(v) => format!("crate::http::MetadataValue::Bool({v})"),
        Value::Number(v) => format!("crate::http::MetadataValue::Number({:?})", v.as_str()),
        Value::String(v) => format!("crate::http::MetadataValue::String({v:?})"),
        Value::Array(v) => format!(
            "crate::http::MetadataValue::Array(&[{}])",
            v.iter().map(metadata).collect::<Vec<_>>().join(",")
        ),
        Value::Object(v) => format!(
            "crate::http::MetadataValue::Object(&[{}])",
            v.iter()
                .map(|(k, v)| format!("({k:?},{})", metadata(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn located_value(v: &Located<Value>) -> String {
    format!(
        "crate::http::LocatedValue {{source:{},value:{}}}",
        source(v.source().source()),
        metadata(v.value())
    )
}
fn link(l: &LinkPlan) -> String {
    let target = match l.target() {
        LinkTarget::OperationId { value, operation } => format!(
            "crate::http::LinkTarget::OperationId {{value:{},operation:{}}}",
            text(value),
            source(operation.source())
        ),
        LinkTarget::OperationRef { value, operation } => format!(
            "crate::http::LinkTarget::OperationRef {{value:{},operation:{}}}",
            text(value),
            source(operation.source())
        ),
    };
    format!(
        "crate::http::Link {{source:{},name:{:?},target:{target},parameters:&[{}],request_body:{},description:{},server:{}}}",
        provenance(l.source()),
        l.name(),
        l.parameters()
            .iter()
            .map(|(k, v)| format!("({k:?},{})", located_value(v)))
            .collect::<Vec<_>>()
            .join(","),
        option(l.request_body(), located_value),
        optional_text(l.description()),
        option(l.server(), server)
    )
}
pub(super) fn operation(plan: &HttpPlan, op: &PlannedOperation) -> String {
    let c = &plan.config;
    let j = &c.codecs.json_limits;
    let responses = op
        .responses
        .iter()
        .map(|r| {
            let status = match r.wire.status() {
                ResponseStatus::Exact(s) => format!("crate::http::Status::Exact({s})"),
                ResponseStatus::Range(s) => format!("crate::http::Status::Range({s})"),
                ResponseStatus::Default => "crate::http::Status::Default".into(),
            };
            format!(
                "crate::http::ResponseSpec {{source:{},status:{status},media:&[{}],links:&[{}]}}",
                source(r.wire.source().use_site().source()),
                r.wire
                    .media()
                    .iter()
                    .map(media_plan)
                    .collect::<Vec<_>>()
                    .join(","),
                r.wire
                    .links()
                    .iter()
                    .map(link)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let accept = op
        .wire
        .responses()
        .iter()
        .flat_map(|r| r.media())
        .map(|m| m.media_type().declared())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "/// Source-backed protocol and metadata for this operation.\npub static OPERATION:crate::http::Operation=crate::http::Operation {{operation_id:{:?},source:{},provenance:{},method:{:?},path_template:{:?},servers:&[{}],security:{},parameters:&[{}],request_media:&[{}],responses:&[{responses}],accept:{accept:?},limits:crate::http::Limits{{request:{},response:{},part:{},item:{},chunk:{},header:{}}},json_limits:crate::JsonLimits{{max_input_bytes:{},max_output_bytes:{},max_depth:{},max_work:{}}}}};\n",
        op.operation_id,
        source(&op.source),
        provenance(op.wire.source()),
        op.wire.method().as_str(),
        op.wire.path(),
        op.wire
            .servers()
            .candidates()
            .iter()
            .map(server)
            .collect::<Vec<_>>()
            .join(","),
        security(op.wire.security()),
        op.wire
            .parameters()
            .iter()
            .map(|p| parameter(plan, p))
            .collect::<Vec<_>>()
            .join(","),
        op.wire
            .body()
            .map(|b| b
                .media()
                .iter()
                .map(media_plan)
                .collect::<Vec<_>>()
                .join(","))
            .unwrap_or_default(),
        c.max_request_bytes,
        c.max_response_bytes,
        c.max_part_bytes,
        c.max_stream_item_bytes,
        c.max_chunk_bytes,
        c.max_header_bytes,
        j.max_input_bytes,
        j.max_output_bytes,
        j.max_depth,
        j.max_work
    )
}
pub(super) fn part(plan: &HttpPlan, p: &PartPlan) -> String {
    let (kind, limit) = match p.representation() {
        PartRepresentation::Json { .. } => (
            "crate::http::PartKind::Json".into(),
            plan.config.max_part_bytes,
        ),
        PartRepresentation::Text { scalar: s, .. } => (
            format!("crate::http::PartKind::Text({})", scalar(*s)),
            plan.config.max_part_bytes,
        ),
        PartRepresentation::Binary { bytes } => (
            "crate::http::PartKind::Binary".into(),
            usize::try_from(bytes.max_bytes()).expect("portable byte policy"),
        ),
        PartRepresentation::Style {
            serialization: s, ..
        } => (
            format!(
                "crate::http::PartKind::Style({})",
                serialization(plan, s, None)
            ),
            plan.config.max_part_bytes,
        ),
    };
    let media_kind = match p.representation() {
        PartRepresentation::Json { .. } => "Json",
        PartRepresentation::Text { .. } => "Text",
        PartRepresentation::Binary { .. } => "Binary",
        PartRepresentation::Style { .. } => "Text",
    };
    format!(
        "crate::http::PartSpec {{source:{},name:{:?},required:{},repeated:{},min_items:{:?},max_items:{:?},media:&[{}],kind:{kind},max_bytes:{limit}}}",
        source(p.source().use_site().source()),
        p.name(),
        p.required(),
        p.multiplicity() == PartMultiplicity::RepeatedArrayItems,
        p.min_items().map(|v| *v.value()),
        p.max_items().map(|v| *v.value()),
        p.content_types()
            .iter()
            .map(|m| media(m, p.source().use_site().source(), media_kind))
            .collect::<Vec<_>>()
            .join(",")
    )
}
pub(super) fn aggregate(
    plan: &HttpPlan,
    a: &PlannedAggregate,
    representation: &Representation,
) -> String {
    let (source_id, required, min, max) = match representation {
        Representation::Form { form } => (
            form.rules().schema().id(),
            form.rules()
                .required()
                .iter()
                .map(|v| v.value().as_str())
                .collect::<Vec<_>>(),
            form.rules().min_properties().map(|v| *v.value()),
            form.rules().max_properties().map(|v| *v.value()),
        ),
        Representation::Multipart {
            multipart: MultipartPlan::Named { rules, .. },
        } => (
            rules.schema().id(),
            rules
                .required()
                .iter()
                .map(|v| v.value().as_str())
                .collect(),
            rules.min_properties().map(|v| *v.value()),
            rules.max_properties().map(|v| *v.value()),
        ),
        Representation::Multipart {
            multipart:
                MultipartPlan::Positional {
                    schema,
                    min_items,
                    max_items,
                    ..
                },
        } => (
            schema.id(),
            Vec::new(),
            min_items.as_ref().map(|v| *v.value()),
            max_items.as_ref().map(|v| *v.value()),
        ),
        _ => unreachable!("aggregate"),
    };
    format!(
        "crate::http::AggregateSpec {{source:{},multipart:{},positional:{},parts:&[{}],additional:{},required:&{:?},min:{min:?},max:{max:?}}}",
        source(source_id),
        a.multipart,
        a.positional,
        a.parts
            .iter()
            .map(|p| part(plan, &p.wire))
            .collect::<Vec<_>>()
            .join(","),
        option(a.additional.as_deref(), |p| format!(
            "&{}",
            part(plan, &p.wire)
        )),
        required
    )
}

fn form_descriptor(plan: &HttpPlan, form: &FormPlan) -> String {
    let additional = match form.additional() {
        AdditionalParts::Forbidden => "std::option::Option::None".into(),
        AdditionalParts::Allowed(p) => format!("std::option::Option::Some(&{})", part(plan, p)),
    };
    format!(
        "crate::http::AggregateSpec{{source:{},multipart:false,positional:false,parts:&[{}],additional:{additional},required:&{:?},min:{:?},max:{:?}}}",
        source(form.rules().schema().id()),
        form.fields()
            .iter()
            .map(|p| part(plan, p))
            .collect::<Vec<_>>()
            .join(","),
        form.rules()
            .required()
            .iter()
            .map(|v| v.value().as_str())
            .collect::<Vec<_>>(),
        form.rules().min_properties().map(|v| *v.value()),
        form.rules().max_properties().map(|v| *v.value())
    )
}
