//! Transport metadata is projected from typed plans, never emitted source text.
use super::*;
use protocol::*;
use serde_json::{Value, json};

pub(super) fn at(id: &SourceId) -> Value {
    json!({"Document":id.document().as_str(),"Pointer":id.pointer()})
}
fn provenance(p: &Provenance) -> Value {
    json!({"UseSite":at(p.use_site().source()),"Definition":at(p.terminal().source()),"References":p.references().iter().map(|r|at(r.source())).collect::<Vec<_>>()})
}
fn scalar(s: ScalarType) -> &'static str {
    match s {
        ScalarType::String => "string",
        ScalarType::Boolean => "boolean",
        ScalarType::Integer => "integer",
        ScalarType::Number => "number",
    }
}
fn encoding(p: PercentEncoding) -> &'static str {
    match p {
        PercentEncoding::UriComponent => "uri",
        PercentEncoding::ReservedExpansion => "reserved",
        PercentEncoding::None => "none",
        PercentEncoding::FormUrlEncoded => "form",
    }
}
fn location(p: ParameterLocation) -> &'static str {
    match p {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Querystring => "querystring",
        ParameterLocation::Header => "header",
        ParameterLocation::Cookie => "cookie",
    }
}
fn codec(c: &CodecRef, plan: &HttpPlan) -> String {
    plan.symbols[c.schema().id()].clone()
}
pub(super) fn serialization(s: &ParameterSerialization) -> Value {
    match s {
        ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => {
            json!({"Content":if media_type.is_json(){"json"}else{"text"},"Encoding":encoding(*percent_encoding)})
        }
        ParameterSerialization::Style {
            style,
            explode,
            shape,
            percent_encoding,
        } => {
            let (kind, value, props, extra) = match shape {
                WireShape::Scalar { scalar: s } => ("scalar", scalar(*s), Value::Null, ""),
                WireShape::Array { items } => ("array", scalar(*items), Value::Null, ""),
                WireShape::FlatObject {
                    properties,
                    additional,
                } => (
                    "object",
                    "",
                    json!(
                        properties
                            .iter()
                            .map(|(k, v)| (k, scalar(*v)))
                            .collect::<BTreeMap<_, _>>()
                    ),
                    match additional {
                        AdditionalScalars::Forbidden => "",
                        AdditionalScalars::AnyScalar => "any",
                        AdditionalScalars::Typed(s) => scalar(*s),
                    },
                ),
            };
            json!({"Style":style,"Explode":explode,"Shape":kind,"Scalar":value,"Properties":props,"Additional":extra,"Encoding":encoding(*percent_encoding)})
        }
    }
}
fn header(h: &HeaderPlan, plan: &HttpPlan) -> Value {
    let mut serial = serialization(h.serialization());
    if let Some(Representation::Text { scalar: s, .. }) =
        h.content_media().map(MediaPlan::representation)
    {
        serial["Scalar"] = json!(scalar(*s));
    }
    json!({"Name":h.name(),"Required":h.required(),"Source":at(h.source().use_site().source()),"Codec":codec(h.codec(),plan),"Serial":serial})
}
fn media_type(m: &MediaType) -> Value {
    let (kind, subtype, rank) = match m.range() {
        MediaRange::Any => ("*", "*", 0),
        MediaRange::Type { type_name } => (type_name.as_str(), "*", 1),
        MediaRange::Concrete { type_name, subtype } => (type_name.as_str(), subtype.as_str(), 2),
    };
    json!({"Declared":m.declared(),"Type":kind,"Subtype":subtype,"Rank":rank,"Parameters":m.parameters()})
}
fn server(s: &ServerPlan) -> Value {
    json!({
        "URL":s.template(),"Source":s.source().map(provenance),"DefaultFrom":s.default_from().map(|s|at(s.source())),"DocumentBase":at(s.document_base().source()),"Name":s.name().map(|v|v.value()),"Description":s.description().map(|v|v.value()),
        "Variables":s.variables().iter().map(|v|json!({"Name":v.name(),"Default":v.default().value(),"Values":v.values().map(|e|e.iter().map(|v|v.value()).collect::<Vec<_>>()),"Source":provenance(v.source()),"Description":v.description().map(|d|d.value())})).collect::<Vec<_>>()
    })
}
fn flow(f: &OAuthFlow) -> Value {
    json!({"Source":at(f.source().source()),"Kind":f.kind(),"URLBase":f.url_base(),"AuthorizationURL":f.authorization_url().map(|v|v.value()),"TokenURL":f.token_url().map(|v|v.value()),"RefreshURL":f.refresh_url().map(|v|v.value()),"DeviceAuthorizationURL":f.device_authorization_url().map(|v|v.value()),"Scopes":f.scopes().iter().map(|(k,v)|(k,v.value())).collect::<BTreeMap<_,_>>()})
}
fn requirement(r: &CredentialRequirement) -> Value {
    let (kind, loc, name, flows, metadata, discovery) = match r.credential() {
        CredentialHook::Bearer { .. } => ("bearer", "", "", vec![], None, None),
        CredentialHook::Basic => ("basic", "", "", vec![], None, None),
        CredentialHook::ApiKey { location: l, name } => (
            "api-key",
            location(*l),
            name.value().as_str(),
            vec![],
            None,
            None,
        ),
        CredentialHook::OAuth2 {
            flows,
            metadata_url,
        } => (
            "oauth2",
            "",
            "",
            flows.iter().map(flow).collect(),
            metadata_url.as_ref().map(|v| v.value()),
            None,
        ),
        CredentialHook::OpenIdConnect { discovery_url } => (
            "openid-connect",
            "",
            "",
            vec![],
            None,
            Some(discovery_url.value()),
        ),
    };
    let (scopes, roles) = match r.permissions() {
        Permissions::Scopes(names) => (names.iter().map(|v| v.value()).collect::<Vec<_>>(), vec![]),
        Permissions::Roles(names) => (vec![], names.iter().map(|v| v.value()).collect::<Vec<_>>()),
    };
    json!({"Name":r.name(),"Kind":kind,"Location":loc,"WireName":name,"URLBase":r.credential().url_base(),"Source":at(r.source().source()),"Scheme":provenance(r.scheme()),"Scopes":scopes,"Roles":roles,"Flows":flows,"MetadataURL":metadata,"DiscoveryURL":discovery,"Description":r.description().map(|v|v.value())})
}
fn link(l: &LinkPlan) -> Value {
    let (id, reference, target) = match l.target() {
        LinkTarget::OperationId { value, operation } => (Some(value.value()), None, operation),
        LinkTarget::OperationRef { value, operation } => (None, Some(value.value()), operation),
    };
    let located = |v: &Located<Value>| json!({"Source":at(v.source().source()),"JSON":v.value()});
    json!({"Name":l.name(),"Source":provenance(l.source()),"OperationID":id,"OperationRef":reference,"Target":at(target.source()),"Parameters":l.parameters().iter().map(|(k,v)|(k,located(v))).collect::<BTreeMap<_,_>>(),"RequestBody":l.request_body().map(located),"Description":l.description().map(|v|v.value()),"Server":l.server().map(server)})
}
fn part(p: &PlannedPart, plan: &HttpPlan) -> Value {
    let w = &p.wire;
    let mut v = json!({"Name":w.name(),"Field":p.field_name,"Required":w.required(),"Repeated":w.multiplicity()==PartMultiplicity::RepeatedArrayItems,"Source":at(w.source().use_site().source()),"Min":w.min_items().map(|v|v.value()),"Max":w.max_items().map(|v|v.value()),"Media":w.content_types().iter().map(media_type).collect::<Vec<_>>(),"Headers":w.headers().iter().map(|h|header(h,plan)).collect::<Vec<_>>(),"MaxBytes":plan.config.max_part_bytes});
    match w.representation() {
        PartRepresentation::Binary { bytes } => {
            v["Kind"] = json!("binary");
            v["MaxBytes"] = json!(bytes.max_bytes());
        }
        PartRepresentation::Json {
            codec: c,
            outer_encoding,
        } => {
            v["Kind"] = json!("json");
            v["Codec"] = json!(codec(c, plan));
            v["Encoding"] = json!(encoding(*outer_encoding));
        }
        PartRepresentation::Text {
            codec: c,
            scalar: s,
            outer_encoding,
        } => {
            v["Kind"] = json!("text");
            v["Codec"] = json!(codec(c, plan));
            v["Scalar"] = json!(scalar(*s));
            v["Encoding"] = json!(encoding(*outer_encoding));
        }
        PartRepresentation::Style {
            codec: c,
            serialization: s,
        } => {
            v["Kind"] = json!("style");
            v["Codec"] = json!(codec(c, plan));
            v["Serial"] = serialization(s);
        }
    }
    v
}
pub(super) fn media(m: &PlannedMedia, plan: &HttpPlan) -> Value {
    let mut v = json!({"Source":at(m.wire.source().use_site().source()),"Media":media_type(m.wire.media_type())});
    match m.wire.representation() {
        Representation::Json { codec: c } => {
            v["Kind"] = json!("json");
            v["Codec"] = json!(c.as_ref().map(|c| codec(c, plan)));
        }
        Representation::Text {
            codec: c,
            scalar: s,
            ..
        } => {
            v["Kind"] = json!("text");
            v["Codec"] = json!(c.as_ref().map(|c| codec(c, plan)));
            v["Scalar"] = json!(scalar(*s));
        }
        Representation::Binary { bytes, .. } => {
            v["Kind"] = json!("binary");
            v["MaxBytes"] = json!(bytes.max_bytes());
        }
        Representation::Stream { stream } => {
            v["Kind"] = json!("stream");
            v["Codec"] = json!(codec(stream.item_codec(), plan));
            v["Framing"] = json!(stream.framing());
            v["MaxItemBytes"] = json!(stream.max_item_bytes());
        }
        Representation::Form { .. } | Representation::Multipart { .. } => {
            let a = m.aggregate.as_ref().unwrap();
            v["Kind"] = json!(if a.multipart { "multipart" } else { "form" });
            let (min, max, required, source) = match m.wire.representation() {
                Representation::Form { form } => (
                    form.rules().min_properties(),
                    form.rules().max_properties(),
                    form.rules()
                        .required()
                        .iter()
                        .map(|r| r.value())
                        .collect::<Vec<_>>(),
                    form.rules().schema().id(),
                ),
                Representation::Multipart {
                    multipart: MultipartPlan::Named { rules, .. },
                } => (
                    rules.min_properties(),
                    rules.max_properties(),
                    rules.required().iter().map(|r| r.value()).collect(),
                    rules.schema().id(),
                ),
                Representation::Multipart {
                    multipart:
                        MultipartPlan::Positional {
                            schema,
                            min_items,
                            max_items,
                            ..
                        },
                } => (min_items.as_ref(), max_items.as_ref(), vec![], schema.id()),
                _ => unreachable!(),
            };
            v["Aggregate"] = json!({"Type":a.type_name,"Positional":a.positional,"Multipart":a.multipart,"Parts":a.parts.iter().map(|p|part(p,plan)).collect::<Vec<_>>(),"Additional":a.additional.as_ref().map(|p|part(p,plan)),"Min":min.map(|v|v.value()),"Max":max.map(|v|v.value()),"MaxParts":plan.config.max_parts,"Required":required,"Source":at(source)});
        }
    }
    v
}

pub(super) fn operation(op: &PlannedOperation, plan: &HttpPlan) -> Value {
    json!({"ID":op.operation_id,"Method":op.wire.method().as_str(),"Path":op.wire.path(),"Source":at(&op.source),
        "MaxRequest":plan.config.max_request_bytes,"MaxResponse":plan.config.max_response_bytes,"MaxPart":plan.config.max_part_bytes,"MaxItem":plan.config.max_stream_item_bytes,
        "Servers":op.wire.servers().candidates().iter().map(server).collect::<Vec<_>>(),
        "Security":op.wire.security().alternatives().iter().map(|a|json!({"Source":at(a.source().source()),"Requirements":a.requirements().iter().map(requirement).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "Parameters":op.parameters.iter().map(|p|json!({"Name":p.wire.name(),"Location":location(p.wire.location()),"Required":p.wire.required(),"Source":at(p.wire.source().use_site().source()),"Codec":codec(p.wire.codec(),plan),"Serial":serialization(p.wire.serialization()),"QueryForm":query_form(p.wire.content_media(),plan)})).collect::<Vec<_>>(),
        "Body":op.body().map(|b|b.media.iter().map(|m|media(m,plan)).collect::<Vec<_>>()),
        "Responses":op.wire.responses().iter().enumerate().map(|(i,r)|json!({"Status":r.status_key(),"Source":at(r.source().use_site().source()),"Media":op.responses.iter().filter(|v|v.response_index==i).filter_map(|v|v.media.as_ref()).map(|m|media(m,plan)).collect::<Vec<_>>(),"Headers":r.headers().iter().map(|h|header(h,plan)).collect::<Vec<_>>(),"Links":r.links().iter().map(link).collect::<Vec<_>>()})).collect::<Vec<_>>()
    })
}

fn query_form(media: Option<&MediaPlan>, plan: &HttpPlan) -> Value {
    let Some(Representation::Form { form }) = media.map(MediaPlan::representation) else {
        return Value::Null;
    };
    let descriptor = |p: &PartPlan| {
        part(
            &PlannedPart {
                field_name: p.name().unwrap_or("").into(),
                setter_name: None,
                data_type: String::new(),
                native_type: String::new(),
                wire: p.clone(),
            },
            plan,
        )
    };
    json!({"Parts":form.fields().iter().map(descriptor).collect::<Vec<_>>(),"Additional":match form.additional(){AdditionalParts::Allowed(p)=>descriptor(p),AdditionalParts::Forbidden=>Value::Null},"MaxParts":plan.config.max_parts,"Source":at(form.rules().schema().id())})
}
