use super::*;
use descriptors::source;

pub(super) fn streams(op: &PlannedOperation) -> bool {
    op.responses
        .iter()
        .flat_map(|r| &r.variants)
        .any(|v| matches!(v.payload, Payload::Stream { request: false, .. }))
}
fn response_type(r: &PlannedResponse, v: &PlannedResponseVariant) -> String {
    format!(
        "crate::http::ApiResponse<{},{}>",
        payload_type(&v.payload),
        r.headers_type.as_deref().unwrap_or("()")
    )
}
pub(super) fn emit(plan: &HttpPlan, op: &PlannedOperation, crate_name: &str) -> String {
    let input = &op.input_type;
    let success = &op.success_type;
    let error = &op.error_type;
    let api_error = &op.api_error_type;
    let mut code = format!(
        "//! {}\n//! OpenAPI operation: {}. Source: {}#{}\n\n",
        prose(
            op.wire
                .description()
                .map(|d| d.value().as_str())
                .unwrap_or("")
        ),
        prose(&op.operation_id),
        prose(op.source.document().as_str()),
        prose(op.source.pointer())
    );
    let mut members = Vec::new();
    code.push_str(&format!("/// Typed source input. Optional values start absent; schema defaults are not applied.\n#[derive(Debug,Clone)]\npub struct {input} {{\n"));
    for p in &op.parameters {
        let ty = format!("crate::models::{}", p.model);
        code.push_str(&format!(
            "    /// {:?} parameter {}. Source: {}#{}\n    pub {}:{},\n",
            p.wire.location(),
            prose(p.wire.name()),
            prose(p.wire.source().use_site().source().document().as_str()),
            prose(p.wire.source().use_site().source().pointer()),
            p.name,
            bodies::optional(&ty, p.wire.required())
        ));
        members.push((p.name.clone(), ty, p.wire.required()));
    }
    if let Some(b) = &op.body {
        code.push_str(&format!("    /// A source-declared body representation, validated before transport.\n    pub body:{},\n    content_type:std::option::Option<std::string::String>,\n",bodies::optional(&b.type_name,b.wire.required())));
        members.push(("body".into(), b.type_name.clone(), b.wire.required()));
    }
    code.push_str("}\n");
    code.push_str(&bodies::constructor(input, &members, op.body.is_some()));
    if let Some(b) = &op.body {
        if b.media.len() > 1 {
            code.push_str(&format!("/// Explicit native request-media choices. A concrete Content-Type selects this variant's source codec.\n#[derive(Debug,Clone)]\npub enum {} {{\n",b.type_name));
            for m in &b.media {
                code.push_str(&format!(
                    "    /// {}\n    {}({}),\n",
                    prose(m.wire.media_type().declared()),
                    m.variant,
                    payload_type(&m.payload)
                ));
            }
            code.push_str("}\n");
        }
        for m in &b.media {
            code.push_str(&bodies::definitions(plan, &m.payload, &m.wire));
        }
    }
    for r in &op.responses {
        if let Some(h) = &r.headers_type {
            code.push_str(&bodies::headers(plan, h, &r.headers));
        }
        for v in &r.variants {
            if let Some(i) = v.media_index {
                code.push_str(&bodies::definitions(plan, &v.payload, &r.wire.media()[i]));
            }
        }
    }
    for (name, is_success) in [(success, true), (api_error, false)] {
        code.push_str(&format!("/// Native {} alternatives. Default responses are classified by the actual status.\n#[derive(Debug)]\npub enum {name} {{\n",if is_success{"successful"}else{"declared error"}));
        for r in &op.responses {
            for v in r
                .variants
                .iter()
                .filter(|v| if is_success { v.success } else { v.error })
            {
                let model = if let Payload::Model { model, .. } = &v.payload {
                    format!(" [`crate::models::{model}`].")
                } else {
                    String::new()
                };
                code.push_str(&format!(
                    "    /// HTTP {} {}.{model} Source: {}#{}\n    {}({}),\n",
                    r.wire.status_key(),
                    prose(
                        v.media_index
                            .map(|i| r.wire.media()[i].media_type().declared())
                            .unwrap_or("no declared media")
                    ),
                    prose(r.wire.source().use_site().source().document().as_str()),
                    prose(r.wire.source().use_site().source().pointer()),
                    v.name,
                    response_type(r, v)
                ));
            }
        }
        code.push_str("}\n");
    }
    let single = op
        .responses
        .iter()
        .flat_map(|r| r.variants.iter().filter(|v| v.success).map(move |v| (r, v)))
        .collect::<Vec<_>>();
    if let [(r, v)] = single.as_slice() {
        let ty = response_type(r, v);
        let data = payload_type(&v.payload);
        code.push_str(&format!("impl {success} {{\n    /// Consume the sole successful alternative without a match.\n    pub fn into_response(self)->{ty}{{match self{{Self::{}(response)=>response}}}}\n    pub fn into_data(self)->{data}{{self.into_response().data}}\n}}\nimpl std::ops::Deref for {success}{{type Target={ty};fn deref(&self)->&Self::Target{{match self{{Self::{}(response)=>response}}}}}}\n",v.name,v.name));
    }
    code.push_str(&format!("impl std::fmt::Display for {api_error} {{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {{"));
    let errors = op
        .responses
        .iter()
        .flat_map(|r| r.variants.iter().filter(|v| v.error))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        code.push_str("let _=f;match *self{} ");
    } else {
        code.push_str("match self{");
        for v in errors {
            code.push_str(&format!("Self::{}(response)=>write!(f,\"declared API response HTTP {{}}\",response.status),",v.name));
        }
        code.push('}');
    }
    code.push_str(&format!("}}}}\nimpl std::error::Error for {api_error} {{}}\n/// Boxed native API alternatives or a source-linked SDK failure.\n#[derive(Debug)]\npub enum {error}{{Api(std::boxed::Box<{api_error}>),Sdk(std::boxed::Box<crate::http::SdkError>)}}\nimpl std::convert::From<crate::http::SdkError> for {error}{{fn from(error:crate::http::SdkError)->Self{{Self::Sdk(std::boxed::Box::new(error))}}}}\nimpl std::fmt::Display for {error}{{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{{match self{{Self::Api(e)=>std::fmt::Display::fmt(e,f),Self::Sdk(e)=>std::fmt::Display::fmt(e,f)}}}}}}\nimpl std::error::Error for {error}{{fn source(&self)->std::option::Option<&(dyn std::error::Error+'static)>{{match self{{Self::Api(e)=>std::option::Option::Some(e.as_ref()),Self::Sdk(e)=>std::option::Option::Some(e.as_ref())}}}}}}\n"));
    code.push_str(&descriptors::operation(plan, op));
    let bound = if streams(op) {
        " where T::Body:'static"
    } else {
        ""
    };
    code.push_str(&format!("/// Execute {} once with validated native inputs. Drop the future or returned item stream to cancel.\n///\n/// ```no_run\n",prose(&op.operation_id)));
    for line in docs::generic_example(op, crate_name).lines() {
        code.push_str(&format!("/// {line}\n"));
    }
    code.push_str(&format!("/// ```\npub async fn {}<T:crate::http::Transport>(client:&crate::http::Client<T>,input:{input})->std::result::Result<{success},{error}>{bound}{{\n    let limits=client.limits(&OPERATION)?;\n",op.function_name));
    if members.is_empty() {
        code.push_str("    let _=input;\n");
    }
    code.push_str(&format!(
        "    let {}parameters=std::vec::Vec::new();\n",
        if op.parameters.is_empty() { "" } else { "mut " }
    ));
    for (i, p) in op.parameters.iter().enumerate() {
        if !p.wire.required() {
            code.push_str(&format!(
                "    if let std::option::Option::Some(value)=&input.{} {{\n",
                p.name
            ));
        }
        code.push_str(&format!("    parameters.push(crate::http::ParameterValue{{parameter:OPERATION.parameters[{i}],value:crate::codecs::{}Codec::encode_value({}).map_err(|e|crate::http::request_codec_error(OPERATION.source,{},e))?}});\n",p.model,if p.wire.required(){format!("&input.{}",p.name)}else{"value".into()},source(p.wire.source().use_site().source())));
        if !p.wire.required() {
            code.push_str("    }\n");
        }
    }
    if let Some(body) = &op.body {
        let value = if body.wire.required() {
            "&input.body"
        } else {
            "value"
        };
        if !body.wire.required() {
            code.push_str("    let body=if let std::option::Option::Some(value)=&input.body {\n");
        } else {
            code.push_str("    let body={\n");
        }
        if body.media.len() > 1 {
            code.push_str(&format!(
                "        let (index,bytes,content_type)=match {value} {{\n"
            ));
            for (i, m) in body.media.iter().enumerate() {
                code.push_str(&format!("            {}::{}(value)=>{{let content_type=input.content_type.as_deref().unwrap_or({:?}).to_owned();let (bytes,content_type)={};({i},bytes,content_type)}},\n",body.type_name,m.variant,m.wire.media_type().declared(),bodies::encode_media(plan,m,"value")));
            }
            code.push_str("        };\n");
        } else {
            let m = &body.media[0];
            code.push_str(&format!("        let index=0;let content_type=input.content_type.as_deref().unwrap_or({:?}).to_owned();\n        let (bytes,content_type)={};\n",m.wire.media_type().declared(),bodies::encode_media(plan,m,value)));
        }
        code.push_str("        std::option::Option::Some(crate::http::PreparedBody::new(&OPERATION,index,content_type,bytes)?)\n");
        code.push_str(if body.wire.required() {
            "    };\n"
        } else {
            "    }else{std::option::Option::None};\n"
        });
    } else {
        code.push_str("    let body=std::option::Option::None;\n");
    }
    code.push_str(
        "    let _=limits;\n    let exchange=client.open(&OPERATION,&parameters,body).await?;\n",
    );
    if streams(op) {
        code.push_str("    let selected=match exchange.selection(){std::result::Result::Ok(value)=>value,std::result::Result::Err(_)=>{let raw=exchange.read().await?;let failure=raw.selection(&OPERATION).expect_err(\"unmatched response\");return std::result::Result::Err(failure.into());}};\n");
        for (ri, r) in op.responses.iter().enumerate() {
            for v in &r.variants {
                if let Payload::Stream { model, .. } = &v.payload {
                    let mi = v.media_index.expect("stream media");
                    let wire::Representation::Stream { stream } =
                        r.wire.media()[mi].representation()
                    else {
                        unreachable!()
                    };
                    code.push_str(&format!("    if selected.response=={ri}&&selected.media==std::option::Option::Some({mi}) {{\n        let status=exchange.status;\n        let typed_headers={};\n        let response=exchange.into_item_response(crate::codecs::{model}Codec::decode_value,crate::http::Framing::{:?},{},{},{:?},typed_headers,OPERATION.responses[{ri}].links);\n        {}\n    }}\n",r.headers_type.as_ref().map(|name|format!("{name}::from_headers(&exchange.headers).map_err(|e|exchange.header_error({},e))?",source(r.wire.source().use_site().source()))).unwrap_or_else(||"()".into()),stream.framing(),source(stream.item_codec().schema().id()),stream.max_item_bytes(),r.wire.media()[mi].media_type().declared(),return_response(op,v,"return ")));
                }
            }
        }
    }
    code.push_str("    let raw=exchange.read().await?;\n    let selected=raw.selection(&OPERATION)?;\n    match (selected.response,selected.media,selected.forbidden) {\n");
    for (ri, r) in op.responses.iter().enumerate() {
        for v in &r.variants {
            if matches!(v.payload, Payload::Stream { .. }) {
                continue;
            }
            let media = v
                .media_index
                .map(|i| format!("std::option::Option::Some({i})"))
                .unwrap_or_else(|| "std::option::Option::None".into());
            code.push_str(&format!("        ({ri},{media},{})=>{{\n            let status=raw.status;\n            let typed_headers={};\n            let data={};\n            let response=raw.into_typed_response(data,{:?},typed_headers,OPERATION.responses[{ri}].links);\n            {}\n        }},\n",if v.forbidden{"true"}else{"_"},r.headers_type.as_ref().map(|h|format!("{h}::from_headers(&raw.headers).map_err(|e|raw.wire_error(OPERATION.source,{},e))?",source(r.wire.source().use_site().source()))).unwrap_or_else(||"()".into()),bodies::decode_response(plan,r,v),v.media_index.map(|i|r.wire.media()[i].media_type().declared()).unwrap_or(""),return_response(op,v,"")));
        }
    }
    code.push_str("        _=>std::result::Result::Err(raw.unexpected_error(OPERATION.source,OPERATION.source).into()),\n    }\n}\n");
    if let Some(name) = &op.default_function_name {
        code.push_str(&format!("/// Execute with every optional input absent.\npub async fn {name}<T:crate::http::Transport>(client:&crate::http::Client<T>)->std::result::Result<{success},{error}>{bound}{{{}(client,{input}::new()).await}}\n",op.function_name));
    }
    let first_request = format!(
        "# Native first request\n\n```no_run\n{}\n```",
        docs::example(plan, op, crate_name)
    );
    code.push_str(&format!(
        "#[cfg(feature=\"reqwest-rustls\")]\n#[doc={first_request:?}]\npub mod quickstart {{}}\n"
    ));
    code
}
fn return_response(op: &PlannedOperation, v: &PlannedResponseVariant, prefix: &str) -> String {
    let good = format!(
        "std::result::Result::Ok({}::{}(response))",
        op.success_type, v.name
    );
    let bad = format!(
        "std::result::Result::Err({}::Api(std::boxed::Box::new({}::{}(response))))",
        op.error_type, op.api_error_type, v.name
    );
    let value = if v.success && v.error {
        format!("if (200..300).contains(&status){{{good}}}else{{{bad}}}")
    } else {
        format!("{{let _=status;{}}}", if v.success { good } else { bad })
    };
    format!(
        "{prefix}{value}{}",
        if prefix.is_empty() { "" } else { ";" }
    )
}
