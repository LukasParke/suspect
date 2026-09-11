use super::*;
use descriptors::source;
use wire::{PartMultiplicity, Representation};

pub(super) fn headers(plan: &HttpPlan, name: &str, headers: &[PlannedHeader]) -> String {
    let mut code = format!(
        "/// Typed declared headers, with their original source-bound codecs.\n#[derive(Debug,Clone)]\npub struct {name} {{\n"
    );
    let mut members = Vec::new();
    for h in headers {
        let ty = format!("crate::models::{}", h.model);
        code.push_str(&format!(
            "    /// Wire header {}. Source: {}#{}\n    pub {}:{},\n",
            prose(h.wire.name()),
            prose(h.wire.source().use_site().source().document().as_str()),
            prose(h.wire.source().use_site().source().pointer()),
            h.name,
            optional(&ty, h.wire.required())
        ));
        members.push((h.name.clone(), ty, h.wire.required()));
    }
    code.push_str("}\n");
    code.push_str(&constructor(name, &members, false));
    code.push_str(&format!("impl {name} {{\n    /// Encode each mutable native header through its bound codec.\n    pub fn to_headers(&self)->std::result::Result<crate::http::Headers,crate::http::SdkError> {{\n        let mut headers=std::vec::Vec::new();\n"));
    for h in headers {
        let value = if h.wire.required() {
            format!("&self.{}", h.name)
        } else {
            "value".into()
        };
        if !h.wire.required() {
            code.push_str(&format!(
                "        if let std::option::Option::Some(value)=&self.{} {{\n",
                h.name
            ));
        }
        let at = source(h.wire.source().use_site().source());
        code.push_str(&format!("        let value=crate::codecs::{}Codec::encode_value({value}).map_err(|e|crate::http::request_codec_error(OPERATION.source,{at},e))?;\n        let text=crate::http::serialize_parameter(OPERATION.source,&{},&value,OPERATION.limits.header,OPERATION.json_limits)?;\n        headers.push(({:?}.into(),text.into_bytes()));\n",h.model,descriptors::header(plan,&h.wire),h.wire.name()));
        if !h.wire.required() {
            code.push_str("        }\n");
        }
    }
    code.push_str("        crate::http::check_headers(OPERATION.source,OPERATION.source,&headers,OPERATION.limits.header,false)?;\n        std::result::Result::Ok(headers)\n    }\n    /// Decode declared headers, enforcing presence and rejecting ambiguous repetition.\n    pub fn from_headers(headers:&crate::http::Headers)->std::result::Result<Self,crate::http::SdkError> {\n");
    for h in headers {
        let at = source(h.wire.source().use_site().source());
        code.push_str(&format!("        let {}=crate::http::decode_header(OPERATION.source,&{},headers,OPERATION.json_limits)?.map(crate::codecs::{}Codec::decode_value).transpose().map_err(|e|crate::http::request_codec_error(OPERATION.source,{at},e))?{};\n",h.name,descriptors::header(plan,&h.wire),h.model,if h.wire.required(){".expect(\"required header checked\")"}else{""}));
    }
    code.push_str(&format!(
        "        std::result::Result::Ok(Self{{{}}})\n    }}\n}}\n",
        headers
            .iter()
            .map(|h| h.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    code
}

pub(super) fn optional(ty: &str, required: bool) -> String {
    if required {
        ty.into()
    } else {
        format!("std::option::Option<{ty}>")
    }
}
pub(super) fn constructor(
    name: &str,
    members: &[(String, String, bool)],
    content_type: bool,
) -> String {
    let args = members
        .iter()
        .filter(|(_, _, required)| *required)
        .map(|(name, ty, _)| format!("{name}:{ty}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut code = format!(
        "impl {name} {{\n    /// Construct required inputs; optional inputs start absent.\n    #[must_use]\n    pub fn new({args})->Self {{ Self{{"
    );
    for (name, _, required) in members {
        code.push_str(&format!(
            "{name}:{},",
            if *required {
                name.as_str()
            } else {
                "std::option::Option::None"
            }
        ));
    }
    if content_type {
        code.push_str("content_type:std::option::Option::None,");
    }
    code.push_str("} }\n");
    for (name, ty, required) in members {
        if !required {
            code.push_str(&format!("    #[must_use]\n    pub fn with_{name}(mut self,value:{ty})->Self{{self.{name}=std::option::Option::Some(value);self}}\n"));
        }
    }
    if content_type {
        code.push_str("    /// Select a concrete declared request representation (required for a wildcard).\n    #[must_use]\n    pub fn with_content_type(mut self,value:impl Into<std::string::String>)->Self{self.content_type=std::option::Option::Some(value.into());self}\n");
    }
    code.push_str("}\n");
    if args.is_empty() {
        code.push_str(&format!(
            "impl std::default::Default for {name}{{fn default()->Self{{Self::new()}}}}\n"
        ));
    }
    code
}
fn item_type(p: &PlannedPart, multipart: bool) -> String {
    let data = p
        .model
        .as_ref()
        .map(|m| format!("crate::models::{m}"))
        .unwrap_or_else(|| "std::vec::Vec<u8>".into());
    if multipart {
        format!(
            "crate::http::Part<{data},{}>",
            p.headers_type.as_deref().unwrap_or("()")
        )
    } else {
        data
    }
}
fn field_type(p: &PlannedPart, multipart: bool) -> String {
    let item = item_type(p, multipart);
    if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        format!("std::vec::Vec<{item}>")
    } else {
        item
    }
}

pub(super) fn definitions(plan: &HttpPlan, payload: &Payload, media: &wire::MediaPlan) -> String {
    let Payload::Parts(a) = payload else {
        return String::new();
    };
    let name = &a.type_name;
    let spec_name = format!("{}_SPEC", rust_models::snake(name).to_ascii_uppercase());
    let mut code = format!(
        "/// Native finite {} aggregate. Structural required/extras/cardinality rules\n/// are separate from part codecs; bytes never enter JSON validation.\n#[derive(Debug,Clone)]\npub struct {name} {{\n",
        if a.multipart { "multipart" } else { "form" }
    );
    let mut members = Vec::new();
    for p in &a.parts {
        let ty = field_type(p, a.multipart);
        code.push_str(&format!(
            "    /// Source: {}#{}\n    pub {}:{},\n",
            prose(p.wire.source().use_site().source().document().as_str()),
            prose(p.wire.source().use_site().source().pointer()),
            p.name,
            optional(&ty, p.wire.required())
        ));
        members.push((p.name.clone(), ty, p.wire.required()));
    }
    if let Some(extra) = &a.additional {
        code.push_str(&format!(
            "    pub {}:{},\n",
            extra.name,
            if a.positional {
                format!("std::vec::Vec<{}>", item_type(extra, a.multipart))
            } else {
                format!(
                    "std::collections::BTreeMap<std::string::String,{}>",
                    field_type(extra, a.multipart)
                )
            }
        ));
    }
    code.push_str("}\n");
    // Aggregate constructors initialize the extras collection, while required
    // members remain native constructor arguments (including repeated files).
    let args = members
        .iter()
        .filter(|(_, _, r)| *r)
        .map(|(n, t, _)| format!("{n}:{t}"))
        .collect::<Vec<_>>()
        .join(",");
    code.push_str(&format!(
        "impl {name} {{\n    #[must_use]\n    pub fn new({args})->Self{{Self{{"
    ));
    for (n, _, r) in &members {
        code.push_str(&format!(
            "{n}:{},",
            if *r {
                n.as_str()
            } else {
                "std::option::Option::None"
            }
        ));
    }
    if let Some(extra) = &a.additional {
        code.push_str(&format!("{}:std::default::Default::default(),", extra.name));
    }
    code.push_str("}}\n");
    for (n, t, r) in &members {
        if !r {
            code.push_str(&format!("    #[must_use]\n    pub fn with_{n}(mut self,value:{t})->Self{{self.{n}=std::option::Option::Some(value);self}}\n"));
        }
    }
    code.push_str("}\n");
    if args.is_empty() {
        code.push_str(&format!(
            "impl std::default::Default for {name}{{fn default()->Self{{Self::new()}}}}\n"
        ));
    }
    for p in a
        .parts
        .iter()
        .chain(a.additional.iter().map(|p| p.as_ref()))
    {
        if let Some(name) = &p.headers_type {
            code.push_str(&headers(plan, name, &p.headers));
        }
    }
    code.push_str(&format!("/// Structural and part codec wire descriptors.\npub static {spec_name}:crate::http::AggregateSpec={};\n",descriptors::aggregate(plan,a,media.representation())));
    let mutable = if !a.parts.is_empty() || a.additional.is_some() {
        "mut "
    } else {
        ""
    };
    code.push_str(&format!("impl {name} {{\n    /// Validate native structure and serialize a finite form/multipart body.\n    pub fn encode(&self,content_type:&str,limits:crate::http::Limits)->std::result::Result<(std::vec::Vec<u8>,std::string::String),crate::http::SdkError>{{\n        let spec=&{spec_name};\n        let {mutable}counts=std::vec::Vec::new();\n"));
    if a.positional {
        code.push_str(if a.parts.iter().any(|p| !p.wire.required()) {
            "        let mut missing=false;\n"
        } else {
            "        let missing=false;\n"
        });
    }
    for p in &a.parts {
        let value = if p.wire.required() {
            format!("&self.{}", p.name)
        } else {
            "value".into()
        };
        if !p.wire.required() {
            code.push_str(&format!(
                "        if let std::option::Option::Some(value)=&self.{} {{\n",
                p.name
            ));
            if p.wire.multiplicity() != PartMultiplicity::RepeatedArrayItems {
                code.push_str("        let _=value;\n");
            }
        }
        if a.positional {
            code.push_str("        if missing{return std::result::Result::Err(crate::http::representation_error(OPERATION.source,spec.source,\"positional multipart has an absent prefix before a present part\"));}\n");
        }
        code.push_str(&format!(
            "        counts.push(({:?}.into(),{}));\n",
            p.wire.name().unwrap_or(&p.name),
            if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
                format!("({value}).len()")
            } else {
                "1usize".into()
            }
        ));
        if !p.wire.required() {
            code.push_str(if a.positional {
                "        }else{missing=true;}\n"
            } else {
                "        }\n"
            });
        }
    }
    if let Some(extra) = &a.additional {
        if a.positional {
            code.push_str(&format!("        if missing&&!self.{}.is_empty(){{return std::result::Result::Err(crate::http::representation_error(OPERATION.source,spec.source,\"positional items cannot follow an omitted prefix\"));}}\n        counts.push((std::string::String::new(),self.{}.len()));\n",extra.name,extra.name));
        } else {
            code.push_str(&format!(
                "        for (name,value) in &self.{}{{counts.push((name.clone(),{}));}}\n",
                extra.name,
                if extra.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
                    "value.len()"
                } else {
                    "{let _=value;1usize}"
                }
            ));
        }
    }
    if a.positional {
        code.push_str("        let _=missing;\n");
    }
    code.push_str(&format!("        crate::http::validate_counts(&OPERATION,spec,&counts)?;\n        let {mutable}parts=std::vec::Vec::new();\n        let {mutable}total=0usize;\n"));
    for (i, p) in a.parts.iter().enumerate() {
        if !p.wire.required() {
            code.push_str(&format!(
                "        if let std::option::Option::Some(value)=&self.{} {{\n",
                p.name
            ));
        }
        let value = if p.wire.required() {
            format!("&self.{}", p.name)
        } else {
            "value".into()
        };
        let repeated = p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems;
        if repeated {
            code.push_str(&format!("        for value in {value} {{\n"));
        }
        code.push_str(&encode_part(
            p,
            &format!("&spec.parts[{i}]"),
            if repeated { "value" } else { &value },
            &format!("{:?}", p.wire.name()),
            a.multipart,
        ));
        if repeated {
            code.push_str("        }\n");
        }
        if !p.wire.required() {
            code.push_str("        }\n");
        }
    }
    if let Some(extra) = &a.additional {
        code.push_str(&format!("        let _extra_spec=spec.additional.expect(\"planned additional part\");\n        for {} in &self.{} {{\n",if a.positional{"value"}else{"(name,value)"},extra.name));
        let repeated = extra.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems;
        if repeated {
            code.push_str("        for value in value {\n");
        }
        code.push_str(&encode_part(
            extra,
            "_extra_spec",
            "value",
            if a.positional {
                "std::option::Option::None"
            } else {
                "std::option::Option::Some(name.as_str())"
            },
            a.multipart,
        ));
        if repeated {
            code.push_str("        }\n");
        }
        code.push_str("        }\n");
    }
    code.push_str("        let _=total;\n        crate::http::prepare_parts(&OPERATION,spec,parts,content_type,limits)\n    }\n    /// Decode finite parts and validate each source-bound native payload.\n    pub fn decode(bytes:&[u8],content_type:&str,limits:crate::http::Limits)->std::result::Result<Self,crate::http::SdkError>{\n");
    let (parsed, mutable) = if a.parts.is_empty() && a.additional.is_none() {
        ("_parsed", "")
    } else {
        ("parsed", if a.parts.is_empty() { "" } else { "mut " })
    };
    code.push_str(&format!("        let spec=&{spec_name};\n        let {mutable}{parsed}=crate::http::parse_parts(&OPERATION,spec,bytes,content_type,limits)?;\n"));
    for (i, p) in a.parts.iter().enumerate() {
        let vector = format!("std::mem::take(&mut parsed.fields[{i}])");
        code.push_str(&format!("        let mut values={vector};\n"));
        code.push_str(&format!(
            "        let {}={};\n",
            p.name,
            decode_field(
                p,
                &format!("&spec.parts[{i}]"),
                "values",
                a.multipart,
                p.wire.required()
            )
        ));
    }
    if let Some(extra) = &a.additional {
        code.push_str(
            "        let _extra_spec=spec.additional.expect(\"planned additional part\");\n",
        );
        if a.positional {
            code.push_str(&format!("        let {}=parsed.items.into_iter().map(|part|{{{}}}).collect::<std::result::Result<std::vec::Vec<_>,crate::http::SdkError>>()?;\n",extra.name,decode_part(extra,"_extra_spec",a.multipart)));
        } else {
            code.push_str(&format!("        let mut {}=std::collections::BTreeMap::new();\n        for (key,mut values) in parsed.additional {{\n            let value={};\n            {}.insert(key,value);\n        }}\n",extra.name,decode_field(extra,"_extra_spec","values",a.multipart,true),extra.name));
        }
    }
    code.push_str(&format!(
        "        std::result::Result::Ok(Self{{{}}})\n    }}\n}}\n",
        a.parts
            .iter()
            .map(|p| p.name.as_str())
            .chain(a.additional.iter().map(|p| p.name.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    ));
    code
}
fn encode_part(p: &PlannedPart, spec: &str, value: &str, name: &str, multipart: bool) -> String {
    let data = if multipart {
        format!("&({value}).data")
    } else {
        value.into()
    };
    let mut code = String::new();
    if let Some(model) = &p.model {
        code.push_str(&format!("        let wire=crate::codecs::{model}Codec::encode_value({data}).map_err(|e|crate::http::request_codec_error(OPERATION.source,({spec}).source,e))?;\n"));
    }
    let wire = if p.model.is_some() {
        "crate::http::PartValue::Json(&wire)".into()
    } else {
        format!("crate::http::PartValue::Bytes({data})")
    };
    code.push_str(&format!("        let part=crate::http::encode_part(&OPERATION,{spec},{name},{wire},{},{},{},{multipart},limits)?;\n        total=total.saturating_add(part.bytes.len());\n        if total>limits.request{{return std::result::Result::Err(crate::http::resource_error(OPERATION.source,({spec}).source,\"aggregate part bytes exceed the request ceiling\"));}}\n        parts.push(part);\n",
        if multipart{format!("({value}).filename.as_deref()")}else{"std::option::Option::None".into()},
        if multipart{format!("({value}).content_type.as_deref()")}else{"std::option::Option::None".into()},
        if multipart&&p.headers_type.is_some(){format!("({value}).headers.to_headers()?")}else{"std::vec::Vec::new()".into()}));
    code
}
fn decode_part(p: &PlannedPart, spec: &str, multipart: bool) -> String {
    let headers = if multipart && let Some(headers_type) = &p.headers_type {
        format!("{headers_type}::from_headers(&part.headers)?")
    } else {
        "()".into()
    };
    let data = if let Some(model) = &p.model {
        format!(
            "crate::codecs::{model}Codec::decode_value(crate::http::decode_part_value(&OPERATION,{spec},&part,{multipart})?).map_err(|e|crate::http::request_codec_error(OPERATION.source,({spec}).source,e))?"
        )
    } else {
        "part.bytes".into()
    };
    if multipart {
        format!(
            "let headers={headers};let data={data};std::result::Result::Ok::<_,crate::http::SdkError>(crate::http::Part{{data,headers,filename:part.filename,content_type:part.content_type}})"
        )
    } else {
        format!("std::result::Result::Ok::<_,crate::http::SdkError>({data})")
    }
}
fn decode_field(
    p: &PlannedPart,
    spec: &str,
    values: &str,
    multipart: bool,
    required: bool,
) -> String {
    let decode = decode_part(p, spec, multipart);
    let expr = if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        format!(
            "{values}.drain(..).map(|part|{{{decode}}}).collect::<std::result::Result<std::vec::Vec<_>,crate::http::SdkError>>()?"
        )
    } else {
        format!(
            "{{let part={values}.pop().expect(\"structural required property checked\");(||{{{decode}}})()?}}"
        )
    };
    if required {
        expr
    } else {
        format!(
            "if {values}.is_empty(){{std::option::Option::None}}else{{std::option::Option::Some({expr})}}"
        )
    }
}

pub(super) fn encode_media(plan: &HttpPlan, media: &PlannedMedia, value: &str) -> String {
    let source = source(media.wire.source().use_site().source());
    let value = match media.wire.representation() {
        Representation::Json { codec: Some(codec) } => format!(
            "crate::http::json_bytes(&OPERATION,{source},&crate::codecs::{}Codec::encode_value({value}).map_err(|e|crate::http::request_codec_error(OPERATION.source,{source},e))?)?",
            plan.symbols[codec.schema().id()]
        ),
        Representation::Json { codec: None } => {
            format!("crate::http::json_bytes(&OPERATION,{source},{value})?")
        }
        Representation::Text { codec, scalar, .. } => {
            let encoded=codec.as_ref().map(|c|format!("crate::codecs::{}Codec::encode_value({value}).map_err(|e|crate::http::request_codec_error(OPERATION.source,{source},e))?",plan.symbols[c.schema().id()]));
            if let Some(encoded) = encoded {
                format!(
                    "crate::http::scalar_bytes(OPERATION.source,{source},&({encoded}),{},limits.request)?",
                    descriptors::scalar(*scalar)
                )
            } else {
                format!(
                    "crate::http::checked_bytes(OPERATION.source,{source},({value}).as_bytes(),limits.request)?"
                )
            }
        }
        Representation::Binary { bytes, .. } => format!(
            "crate::http::checked_bytes(OPERATION.source,{source},{value},limits.request.min({}))?",
            bytes.max_bytes()
        ),
        Representation::Form { .. } | Representation::Multipart { .. } => {
            return format!("({value}).encode(&content_type,limits)?");
        }
        Representation::Stream { stream } => format!(
            "crate::http::encode_items(&OPERATION,{source},{value},crate::codecs::{}Codec::encode_value,crate::http::Framing::{:?},limits.item.min({}))?",
            plan.symbols[stream.item_codec().schema().id()],
            stream.framing(),
            stream.max_item_bytes()
        ),
    };
    format!("({value},content_type)")
}

pub(super) fn decode_response(
    plan: &HttpPlan,
    r: &PlannedResponse,
    v: &PlannedResponseVariant,
) -> String {
    let at = source(r.wire.source().use_site().source());
    let Some(index) = v.media_index else {
        return match v.payload {
            Payload::NoContent => "()".into(),
            _ => "raw.body.clone()".into(),
        };
    };
    let media = &r.wire.media()[index];
    match media.representation() {
        Representation::Json { codec: Some(c) } => format!(
            "crate::codecs::{}Codec::decode_bytes(&raw.body).map_err(|e|raw.decoding_error(OPERATION.source,{at},e))?",
            plan.symbols[c.schema().id()]
        ),
        Representation::Json { codec: None } => format!(
            "crate::http::parse_json_body(&OPERATION,{at},&raw.body).map_err(|e|raw.wire_error(OPERATION.source,{at},e))?"
        ),
        Representation::Text { codec, scalar, .. } => {
            let text = format!(
                "std::str::from_utf8(&raw.body).map_err(|_|raw.response_error(OPERATION.source,{at},crate::http::SdkErrorKind::ResponseDecoding,\"text body is not UTF-8\"))?"
            );
            if let Some(c) = codec {
                format!(
                    "crate::codecs::{}Codec::decode_value(crate::http::scalar_value(OPERATION.source,{at},{text},{}).map_err(|e|raw.wire_error(OPERATION.source,{at},e))?).map_err(|e|raw.decoding_error(OPERATION.source,{at},e))?",
                    plan.symbols[c.schema().id()],
                    descriptors::scalar(*scalar)
                )
            } else {
                format!("({text}).to_owned()")
            }
        }
        Representation::Binary { bytes, .. } => format!(
            "crate::http::checked_bytes(OPERATION.source,{at},&raw.body,raw.limits.response.min({})).map_err(|e|raw.wire_error(OPERATION.source,{at},e))?",
            bytes.max_bytes()
        ),
        Representation::Form { .. } | Representation::Multipart { .. } => format!(
            "{}::decode(&raw.body,raw.content_type.as_deref().expect(\"matched content type\"),raw.limits).map_err(|e|raw.wire_error(OPERATION.source,{at},e))?",
            payload_type(&v.payload)
        ),
        Representation::Stream { .. } => {
            unreachable!("stream is consumed before finite response dispatch")
        }
    }
}
