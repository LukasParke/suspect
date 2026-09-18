//! Native compatibility data from the same typed plans used by emission.
//! Prose, source IDs and byte spans deliberately stay outside interface equality.

use super::{HttpConfig, HttpSymbols, PlannedOperation, protocol_emit as emit};
use crate::http_protocol::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use suspect_ir::contract::{Contract, SchemaId};

type Names = BTreeMap<SchemaId, String>;

pub(super) fn capture(
    contract: &Contract,
    operations: &[PlannedOperation],
    symbols: &HttpSymbols,
    config: &HttpConfig,
) -> Vec<Value> {
    let (credentials, required) = emit::client_requirements(operations);
    let options_required = !required.is_empty() && symbols.credential_env_helper.is_none();
    let client = json!({"credentials":credentials,"requiredAlternatives":required,"optionsRequired":options_required,"credentialsType":format!("Partial<Credentials>{}",required.iter().map(|requirement|format!(" & {requirement}")).collect::<String>())});
    operations.iter().map(|operation| {
        let wire = operation.protocol();
        let parameters = operation.parameters.iter().zip(wire.parameters()).map(|(native, parameter)| {
            let native_type=format!("Models.{}",symbols.request[parameter.codec().schema().id()]);
            let mut content=parameter.content_media().map(|value|media(value,&symbols.request,false));
            if parameter.location()==ParameterLocation::Querystring && let Some(content)=&mut content {
                content["type"]=json!(native_type);
                if content["kind"]=="form" {content["object"]["additionalMember"]=Value::Null;}
            }
            json!({"member":native.native_name,"wire":parameter.name(),"location":parameter.location(),"required":parameter.required(),"model":symbols.request[parameter.codec().schema().id()],"type":native_type,"codec":codec(parameter.codec(),&symbols.request),"serialization":serialization(parameter.serialization()),"content":content})
        }).collect::<Vec<_>>();
        let body = wire.body().map(|body|json!({"member":"body","required":body.required(),"taggedMedia":emit::tagged_body(body),"type":emit::body_type(body,&symbols.request),"media":body.media().iter().map(|value|media(value,&symbols.request,false)).collect::<Vec<_>>() }));
        let responses = wire.responses().iter().map(|response| {
            let forbidden = wire.method()==Method::Head || matches!(response.status(),ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1));
            json!({"status":response.status_key(),"selector":response.status(),"body":if forbidden {"http-forbidden"} else if response.media().is_empty() {"bounded-bytes"} else {"declared"},"media":if forbidden {Vec::new()} else {response.media().iter().map(|value|media(value,&symbols.response,true)).collect::<Vec<_>>()},"headers":headers(response.headers(),&symbols.response),"headersType":emit::header_type(response.headers(),&symbols.response),"links":response.links().iter().map(|link|link_interface(contract,link)).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        let signature=json!({"clientOptional":required.is_empty()&&operation.input_optional(),"inputOptional":operation.input_optional(),"callOptional":true,"inputType":operation.input_type,"callType":"CallOptions","returnType":format!("Promise<{}>",operation.success_type)});
        let success=emit::response_union(operation,&symbols.response,true);
        let error=emit::response_union(operation,&symbols.response,false);
        json!({"format":"typescript-native-http-v2","signature":signature,"constructor":{"name":"createClient","optionsType":"ClientOptions","optionsRequired":options_required,"methodSignature":signature},"client":client,"credential":{"client":client,"security":security(wire.security())},"method":wire.method().as_str(),"parameters":parameters,"body":body,"responses":responses,"successType":success,"errorType":error,"responseUnions":{"success":success,"error":error},"security":security(wire.security()),"servers":wire.servers().candidates().iter().map(server).collect::<Vec<_>>(),"limits":{"request":config.max_request_bytes,"response":config.max_response_bytes,"part":config.max_part_bytes,"streamItem":config.max_stream_item_bytes,"streamBuffer":config.max_stream_buffer_bytes,"streamItems":config.max_stream_items}})
    }).collect()
}

fn codec(codec: &CodecRef, names: &Names) -> Value {
    let name = &names[codec.schema().id()];
    json!({"model":name,"value":format!("{name}Codec"),"input":codec.input()})
}
fn media(value: &MediaPlan, names: &Names, response: bool) -> Value {
    let mut value_json = representation(value.representation(), names, response);
    let object = value_json.as_object_mut().unwrap();
    object.insert(
        "mediaType".into(),
        json!(emit::canonical_media(value.media_type())),
    );
    object.insert(
        "type".into(),
        json!(emit::media_type(value, names, response)),
    );
    value_json
}
fn representation(value: &Representation, names: &Names, response: bool) -> Value {
    match value {
        Representation::Json { codec: value } => {
            json!({"kind":"json","codec":value.as_ref().map(|value|codec(value,names))})
        }
        Representation::Text {
            codec: value,
            scalar,
            encoding,
        } => {
            json!({"kind":"text","codec":value.as_ref().map(|value|codec(value,names)),"scalar":scalar,"encoding":encoding})
        }
        Representation::Binary { bytes, .. } => json!({"kind":"binary","bytes":byte_policy(bytes)}),
        Representation::Form { form } => {
            json!({"kind":"form","object":object(form.rules(),form.fields(),form.additional(),names,response,false)})
        }
        Representation::Multipart {
            multipart:
                MultipartPlan::Named {
                    rules,
                    parts,
                    additional,
                },
        } => {
            json!({"kind":"multipart","layout":"named","object":object(rules,parts,additional,names,response,true)})
        }
        Representation::Multipart {
            multipart:
                MultipartPlan::Positional {
                    prefix,
                    items,
                    min_items,
                    max_items,
                    ..
                },
        } => {
            json!({"kind":"multipart","layout":"positional","prefix":prefix.iter().map(|value|part(value,names,response,true)).collect::<Vec<_>>(),"items":additional(items,names,response,true),"minItems":min_items.as_ref().map(Located::value),"maxItems":max_items.as_ref().map(Located::value)})
        }
        Representation::Stream { stream } => {
            json!({"kind":"stream","framing":stream.framing(),"itemCodec":stream.item_codec().map(|c|codec(c,names)),"maxItemBytes":stream.max_item_bytes()})
        }
    }
}
fn byte_policy(bytes: &BytePolicy) -> Value {
    json!({"maxBytes":bytes.max_bytes(),"declaredMaxBytes":bytes.declared_max_bytes().map(Located::value)})
}
fn object(
    rules: &ObjectRules,
    parts: &[PartPlan],
    extra: &AdditionalParts,
    names: &Names,
    response: bool,
    multipart: bool,
) -> Value {
    json!({"required":rules.required().iter().map(Located::value).collect::<Vec<_>>(),"minProperties":rules.min_properties().map(Located::value),"maxProperties":rules.max_properties().map(Located::value),"fields":parts.iter().map(|value|part(value,names,response,multipart)).collect::<Vec<_>>(),"additional":additional(extra,names,response,multipart),"additionalMember":matches!(extra,AdditionalParts::Allowed(_)).then(||emit::extra_member(parts))})
}
fn additional(value: &AdditionalParts, names: &Names, response: bool, multipart: bool) -> Value {
    match value {
        AdditionalParts::Forbidden => json!({"kind":"forbidden"}),
        AdditionalParts::Allowed(value) => {
            json!({"kind":"allowed","part":part(value,names,response,multipart)})
        }
    }
}
fn part(value: &PartPlan, names: &Names, response: bool, multipart: bool) -> Value {
    let representation = match value.representation() {
        PartRepresentation::Json {
            codec: value,
            outer_encoding,
        } => json!({"kind":"json","codec":codec(value,names),"outerEncoding":outer_encoding}),
        PartRepresentation::Text {
            codec: value,
            scalar,
            outer_encoding,
        } => {
            json!({"kind":"text","codec":codec(value,names),"scalar":scalar,"outerEncoding":outer_encoding})
        }
        PartRepresentation::Binary { bytes } => json!({"kind":"binary","bytes":byte_policy(bytes)}),
        PartRepresentation::Style {
            codec: value,
            serialization: value_serialization,
        } => {
            json!({"kind":"style","codec":codec(value,names),"serialization":serialization(value_serialization)})
        }
    };
    json!({"name":value.name(),"required":value.required(),"multiplicity":value.multiplicity(),"minItems":value.min_items().map(Located::value),"maxItems":value.max_items().map(Located::value),"itemType":emit::part_item_type(value,names,response,multipart),"contentTypes":value.content_types().iter().map(emit::canonical_media).collect::<Vec<_>>(),"representation":representation,"headers":headers(value.headers(),names)})
}
fn headers(values: &[HeaderPlan], names: &Names) -> Vec<Value> {
    values.iter().map(|value|json!({"name":value.name(),"required":value.required(),"codec":codec(value.codec(),names),"serialization":serialization(value.serialization()),"content":value.content_media().map(|value|media(value,names,true))})).collect()
}
fn serialization(value: &ParameterSerialization) -> Value {
    match value {
        ParameterSerialization::Style {
            style,
            explode,
            shape,
            percent_encoding,
        } => {
            json!({"kind":"style","style":style,"explode":explode,"shape":shape,"percentEncoding":percent_encoding})
        }
        ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => {
            json!({"kind":"content","mediaType":emit::canonical_media(media_type),"percentEncoding":percent_encoding})
        }
    }
}
fn server(value: &ServerPlan) -> Value {
    json!({"template":value.template(),"name":value.name().map(Located::value),"variables":value.variables().iter().map(|value|json!({"name":value.name(),"default":value.default().value(),"enum":value.values().map(|values|values.iter().map(Located::value).collect::<Vec<_>>())})).collect::<Vec<_>>()})
}
fn security(value: &SecurityPlan) -> Value {
    match value {
        SecurityPlan::Undeclared { .. } => json!({"kind":"undeclared"}),
        SecurityPlan::NoAuth { .. } => json!({"kind":"no-auth"}),
        SecurityPlan::Alternatives { alternatives, .. } => {
            json!({"kind":"alternatives","alternatives":alternatives.iter().map(|alternative|alternative.requirements().iter().map(|requirement|{
            let permission=match requirement.permissions(){Permissions::Scopes(names)=>json!({"kind":"scopes","names":names.iter().map(Located::value).collect::<Vec<_>>()}),Permissions::Roles(names)=>json!({"kind":"roles","names":names.iter().map(Located::value).collect::<Vec<_>>()})};
            let hook=match requirement.credential(){
                CredentialHook::Bearer {..}=>json!({"kind":"bearer"}),CredentialHook::Basic=>json!({"kind":"basic"}),
                CredentialHook::ApiKey {location,name}=>json!({"kind":"api-key","location":location,"name":name.value()}),
                CredentialHook::OpenIdConnect {discovery_url}=>json!({"kind":"open-id-connect","discoveryUrl":discovery_url.value()}),
                CredentialHook::OAuth2 {flows,metadata_url}=>json!({"kind":"oauth2","metadataUrl":metadata_url.as_ref().map(Located::value),"flows":flows.iter().map(|flow|json!({"kind":flow.kind(),"authorizationUrl":flow.authorization_url().map(Located::value),"tokenUrl":flow.token_url().map(Located::value),"refreshUrl":flow.refresh_url().map(Located::value),"deviceAuthorizationUrl":flow.device_authorization_url().map(Located::value),"scopes":flow.scopes().keys().collect::<Vec<_>>()})).collect::<Vec<_>>()})
            };
            json!({"name":requirement.name(),"type":emit::credentials_type(requirement),"permissions":permission,"hook":hook})
        }).collect::<Vec<_>>()).collect::<Vec<_>>()})
        }
    }
}
fn link_interface(contract: &Contract, link: &LinkPlan) -> Value {
    let target = match link.target() {
        LinkTarget::OperationId { value, .. } => {
            json!({"kind":"operation-id","operationId":value.value()})
        }
        LinkTarget::OperationRef { operation, .. } => {
            json!({"kind":"operation-ref","operationId":contract.operations().find(|candidate|candidate.source()==operation.source()).and_then(|candidate|candidate.operation_id()).map(str::to_owned)})
        }
    };
    // Instance data is copied as an opaque value. A literal object's own
    // "source", "schema" or "description" keys are never stripped.
    json!({"name":link.name(),"type":"LinkMetadata","target":target,"parameters":link.parameters().iter().map(|(name,value)|(name.clone(),json!({"kind":"literal","value":value.value()}))).collect::<BTreeMap<_,_>>(),"requestBody":link.request_body().map(|value|json!({"kind":"literal","value":value.value()})),"server":link.server().map(server)})
}
