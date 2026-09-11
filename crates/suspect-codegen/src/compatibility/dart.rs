//! Native Dart interfaces from the admitted rich protocol and model plans.
use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};
use crate::{
    backend,
    dart_sdk::{
        self, DartExtras, DartShape, ModelPlan, Plan, PlannedAggregate, PlannedHeaders,
        PlannedMedia, PlannedPayload as P,
    },
    http_protocol as w,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SourceId};

fn named(name: &str) -> Value {
    json!({"kind":"named","name":name})
}
fn primitive(name: &str) -> Value {
    json!({"kind":"primitive","name":name})
}
fn generic(name: &str, arguments: Vec<Value>) -> Value {
    json!({"kind":"generic","name":name,"arguments":arguments})
}
fn nullable(of: Value) -> Value {
    json!({"kind":"nullable","of":of})
}
fn presence(of: Value, required: bool) -> Value {
    if required {
        of
    } else {
        generic("Presence", vec![of])
    }
}
fn initial(required: bool) -> Value {
    json!({"kind":if required{"argument"}else{"absent"}})
}
fn null() -> Value {
    json!({"kind":"literal","value":null})
}
fn parameter(name: &str, ty: Value, required: bool, default: Value) -> Value {
    json!({"name":name,"type":ty,"required":required,"initialization":default})
}
fn field(name: &str, ty: Value, readonly: bool) -> Value {
    json!({"name":name,"type":ty,"readOnly":readonly,"storage":if readonly{"final-field"}else{"mutable-field"}})
}
fn method(name: &str, parameters: Vec<Value>, result: Value) -> Value {
    json!({"name":name,"parameters":parameters,"result":result})
}
fn ty(models: &ModelPlan, index: usize) -> Value {
    let m = &models.symbols()[index];
    let base = match &m.shape {
        DartShape::Any | DartShape::Contextual => named("JsonValue"),
        DartShape::Never => primitive("Never"),
        DartShape::Null => primitive("Null"),
        DartShape::Boolean => primitive("bool"),
        DartShape::String => primitive("String"),
        DartShape::Number => named("JsonNumber"),
        DartShape::Integer => named("JsonInteger"),
        DartShape::Array(item) => generic("List", vec![optional_ty(models, *item)]),
        DartShape::Alias(target) => {
            let t = ty(models, *target);
            if t["kind"] == "nullable" {
                t["of"].clone()
            } else {
                t
            }
        }
        _ => named(&m.name),
    };
    if models.uses_native_null(index) {
        nullable(base)
    } else {
        base
    }
}
fn optional_ty(models: &ModelPlan, index: Option<usize>) -> Value {
    index.map_or_else(|| named("JsonValue"), |index| ty(models, index))
}
fn at(plan: &Plan, id: &SourceId) -> Value {
    ty(
        plan.models(),
        plan.models()
            .model(id)
            .expect("admitted codec source")
            .index,
    )
}
fn payload(plan: &Plan, media: &PlannedMedia) -> Value {
    match &media.payload {
        P::Json {
            schema: Some(id), ..
        }
        | P::Text {
            schema: Some(id), ..
        }
        | P::Stream { schema: id, .. } => at(plan, id),
        P::Json { schema: None, .. } => named("JsonValue"),
        P::Text { schema: None, .. } => primitive("String"),
        P::Bytes { .. } => named("Uint8List"),
        P::Aggregate(a) => named(&a.name),
    }
}
fn record(
    contract: &Contract,
    source: &SourceId,
    name: &str,
    role: &str,
    descriptor: Value,
) -> NativeModel {
    NativeModel {
        source: Location::at(contract, source),
        name: name.into(),
        role: role.into(),
        descriptor: Some(descriptor),
    }
}
fn headers(plan: &Plan, source: &SourceId, h: &PlannedHeaders) -> NativeModel {
    record(
        plan.contract(),
        source,
        &h.name,
        "headers",
        json!({"kind":"class","modifiers":["final"],
        "fields":h.fields.iter().map(|p|field(&p.name,presence(at(plan,&p.schema),p.wire.required()),true)).collect::<Vec<_>>(),
        "constructor":{"name":h.name,"const":true,"arguments":"named","parameters":h.fields.iter().map(|p|parameter(&p.name,presence(at(plan,&p.schema),p.wire.required()),p.wire.required(),initial(p.wire.required()))).collect::<Vec<_>>()}}),
    )
}
fn aggregate(plan: &Plan, value: &PlannedAggregate) -> Vec<NativeModel> {
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut args = Vec::new();
    for p in value
        .fields
        .iter()
        .chain(value.extra.iter().map(Box::as_ref))
    {
        if let Some(h) = &p.headers {
            records.push(headers(plan, p.wire.source().use_site().source(), h));
        }
        let part_type = match p.wire.representation() {
            w::PartRepresentation::Json { codec, .. }
            | w::PartRepresentation::Text { codec, .. }
            | w::PartRepresentation::Style { codec, .. } => at(plan, codec.schema().id()),
            w::PartRepresentation::Binary { .. } => named("Uint8List"),
        };
        if let Some(name) = &p.wrapper_name {
            let mut members = vec![
                field("value", part_type.clone(), false),
                field("contentType", nullable(primitive("String")), true),
                field("filename", nullable(primitive("String")), true),
            ];
            let mut ctor = vec![
                parameter("value", part_type.clone(), true, initial(true)),
                parameter("contentType", nullable(primitive("String")), false, null()),
                parameter("filename", nullable(primitive("String")), false, null()),
            ];
            if let Some(h) = &p.headers {
                members.push(field("headers", named(&h.name), true));
                ctor.push(parameter("headers", named(&h.name), true, initial(true)));
            }
            records.push(record(plan.contract(),p.wire.source().use_site().source(),name,"part",json!({"kind":"class","modifiers":["final"],"fields":members,"constructor":{"name":name,"arguments":"named","parameters":ctor}})));
        }
        let value_type = p
            .wrapper_name
            .as_ref()
            .map_or(part_type, |name| named(name));
        let part_type = if p.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            generic("List", vec![value_type])
        } else {
            value_type
        };
        if p.wire.name().is_some() {
            fields.push(json!({"name":p.name,"wire":p.wire.name(),"type":presence(part_type.clone(),p.wire.required()),"readOnly":false,"required":p.wire.required()}));
            args.push(parameter(
                &p.name,
                presence(part_type, p.wire.required()),
                p.wire.required(),
                initial(p.wire.required()),
            ));
        } else {
            let map = generic("Map", vec![primitive("String"), part_type]);
            fields.push(field("extraFields", map.clone(), true));
            args.push(parameter("extraFields", nullable(map), false, null()));
        }
    }
    records.push(record(plan.contract(),value.rules.schema().id(),&value.name,"aggregate",json!({"kind":"class","modifiers":["final"],"fields":fields,
        "representation":if value.multipart{"multipart"}else{"form"},"constructor":{"name":value.name,"arguments":"named","parameters":args},
        "structuralRules":{"required":value.rules.required().iter().map(|v|v.value()).collect::<Vec<_>>(),"minimum":value.rules.min_properties().map(|v|v.value()),"maximum":value.rules.max_properties().map(|v|v.value())}})));
    records
}
fn model_records(plan: &Plan) -> Vec<NativeModel> {
    let mut result = Vec::new();
    for m in plan.models().symbols() {
        let mut description = match &m.shape {
            DartShape::Object { fields, extras } => {
                let mut args = Vec::new();
                let mut members = Vec::new();
                for f in fields {
                    let field_type = presence(
                        optional_ty(plan.models(), f.target),
                        f.required || f.fixed.is_some(),
                    );
                    let init = f.fixed.as_ref().map_or_else(
                        || initial(f.required),
                        |v| json!({"kind":"literal","value":v}),
                    );
                    members.push(json!({"name":f.name,"wire":f.wire_name,"type":field_type,"required":f.required,"readOnly":f.fixed.is_some(),"storage":if f.fixed.is_some(){"getter"}else{"mutable-field"},"initialization":init,"fixed":f.fixed}));
                    if f.fixed.is_none() {
                        args.push(parameter(&f.name, field_type, f.required, init));
                    }
                }
                let extra = match extras {
                    DartExtras::Closed => Value::Null,
                    _ => {
                        let target = match extras {
                            DartExtras::Typed(i) => Some(*i),
                            _ => None,
                        };
                        let map = generic(
                            "Map",
                            vec![primitive("String"), optional_ty(plan.models(), target)],
                        );
                        args.push(parameter(
                            "extraFields",
                            nullable(map.clone()),
                            false,
                            null(),
                        ));
                        let mut extra = json!({"name":"extraFields","type":map,"readOnly":true,"mutableEntries":true});
                        if matches!(extras, DartExtras::Checked) {
                            extra["representation"] = json!("checked-exact-json");
                            extra["validation"] = json!("complete-object-schema");
                        }
                        extra
                    }
                };
                args.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                let mut parents = m.parents.clone();
                parents.sort();
                json!({"kind":"class","modifiers":["final"],"fields":members,"extraFields":extra,"implements":parents,"schemaDefaultsApplied":false,
                    "constructor":{"name":m.name,"const":false,"arguments":if args.is_empty(){"none"}else{"named"},"parameters":args}})
            }
            DartShape::Enum(values) => {
                json!({"kind":"enum","constructor":null,"values":values.iter().enumerate().map(|(i,(wire,name))|json!({"name":name,"ordinal":i,"wireValue":{"kind":"literal","value":wire}})).collect::<Vec<_>>(),"fields":[field("wireValue",primitive("String"),true)]})
            }
            DartShape::Checked => {
                json!({"kind":"class","modifiers":["final"],"representation":"checked-exact-json","fields":[field("value",named("JsonValue"),true)],"constructor":{"name":format!("{}.fromJson",m.name),"factory":true,"arguments":"positional","parameters":[parameter("value",named("JsonValue"),true,initial(true))]}})
            }
            DartShape::Union {
                targets,
                direct,
                variants,
            } => {
                if !direct {
                    for (&target, name) in targets.iter().zip(variants) {
                        let model = &plan.models().symbols()[target];
                        result.push(record(plan.contract(),&model.source,name,"union-variant",json!({"kind":"class","modifiers":["final"],"extends":m.name,"fields":[field("value",ty(plan.models(),target),false)],"constructor":{"name":name,"arguments":"positional","parameters":[parameter("value",ty(plan.models(),target),true,initial(true))]}})));
                    }
                }
                json!({"kind":"class","modifiers":["sealed"],"constructor":null,"direct":direct,"variants":targets.iter().enumerate().map(|(i,&target)|if *direct{json!({"type":ty(plan.models(),target),"membership":"implements"})}else{json!({"name":variants[i],"type":ty(plan.models(),target),"membership":"extends"})}).collect::<Vec<_>>()})
            }
            _ => {
                json!({"kind":"type-alias","nominal":false,"type":at(plan,&m.source),"signatureType":plan.models().native_type(&m.source),"constructor":null})
            }
        };
        description["deprecated"] = json!(m.deprecated);
        if matches!(m.shape, DartShape::Contextual) {
            description["representation"] = json!("context-bound-exact-json");
            description["validation"] = json!("complete-enclosing-codec-resource-scope");
        }
        result.push(record(
            plan.contract(),
            &m.source,
            &m.name,
            "model",
            description,
        ));
        let t = at(plan, &m.source);
        let mut codec = json!({"kind":"model-codec","constructor":null,"storage":"top-level-final","type":generic("ModelCodec",vec![t.clone()]),
            "fields":[field("source",named("SchemaSource"),true)],"methods":[
                method("validate",vec![parameter("value",named("JsonValue"),true,initial(true))],named("ValidationResult")),
                method("decode",vec![parameter("text",primitive("String"),true,initial(true))],t.clone()),
                method("decodeBytes",vec![parameter("bytes",generic("List",vec![primitive("int")]),true,initial(true))],t.clone()),
                method("fromJson",vec![parameter("value",named("JsonValue"),true,initial(true))],t.clone()),
                method("toJson",vec![parameter("value",t.clone(),true,initial(true))],named("JsonValue")),
                method("encode",vec![parameter("value",t.clone(),true,initial(true))],primitive("String")),
                method("encodeBytes",vec![parameter("value",t,true,initial(true))],named("Uint8List"))]});
        if plan.program().version != suspect_schema::OwnedProgram::V1_VERSION {
            codec["validation"] = json!({"version":plan.program().version,"profile":plan.program().profile,"decode":"validate-before-conversion","encode":"revalidate-current-value","jsonNullAccepted":m.nullable});
        }
        result.push(record(
            plan.contract(),
            &m.source,
            &m.codec_name,
            "codec",
            codec,
        ));
    }
    result
}
fn credential_type(c: &dart_sdk::PlannedCredential) -> Value {
    nullable(match c.hook {
        w::CredentialHook::Basic => named("BasicCredentials"),
        w::CredentialHook::OAuth2 { .. } | w::CredentialHook::OpenIdConnect { .. } => {
            named("CredentialProvider")
        }
        _ => primitive("String"),
    })
}
pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    if snapshot.target.import_name.is_some() {
        return Err(vec![PlanFinding {
            source: None,
            code: "sdk-package".into(),
            message: "Dart library identity comes from package_name; import_name is not supported"
                .into(),
        }]);
    }
    let mut config = backend::dart_config(&snapshot.target);
    config.credential_env = snapshot.generation.credential_env.clone();
    let plan = dart_sdk::plan_sdk_with_profiles(
        contract.clone(),
        selected,
        config,
        &snapshot.generation.compatibility_profiles,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| PlanFinding {
                source: Some(Location::at(&contract, &e.source)),
                code: e.code.into(),
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    snapshot.credential_env = plan
        .credential_env()
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    let mut records = model_records(&plan);
    let mut operations = Vec::new();
    for op in plan.operations() {
        let mut parameters=op.parameters.iter().map(|p|json!({"member":p.name,"wire":p.wire_name,"model":p.native_type,"location":format!("{:?}",p.wire.location()),"required":p.required,"type":presence(at(&plan,&p.schema),p.required),"initialization":initial(p.required),"codec":p.codec_name,"serialization":p.wire.serialization()})).collect::<Vec<_>>();
        for (name, t) in [
            ("cancellation", nullable(named("CancellationToken"))),
            ("timeout", nullable(named("Duration"))),
            ("server", nullable(named("ServerSelection"))),
            ("securityAlternative", nullable(primitive("int"))),
        ] {
            parameters.push(json!({"member":name,"type":t,"required":false,"initialization":(null()),"role":"control"}));
        }
        parameters.sort_by(|a, b| a["member"].as_str().cmp(&b["member"].as_str()));
        let body=op.body.as_ref().map(|body|{
            for media in &body.media{if let P::Aggregate(a)=&media.payload{records.extend(aggregate(&plan,a));}}
            if let Some(name)=&body.choice_name{
                records.push(record(&contract,&body.source,name,"request-media",json!({"kind":"class","modifiers":["sealed"],"constructor":null,"variants":body.media.iter().map(|m|m.variant_name.clone()).collect::<Vec<_>>()})));
                for media in &body.media{records.push(record(&contract,media.wire.source().use_site().source(),&media.variant_name,"request-media-variant",json!({"kind":"class","modifiers":["final"],"extends":name,"fields":[field("value",payload(&plan,media),true),field("contentType",primitive("String"),true)],"constructor":{"name":media.variant_name,"arguments":"positional-value-and-named-contentType","value":payload(&plan,media),"contentTypeRequired":!matches!(media.wire.media_type().range(),w::MediaRange::Concrete{..}),"contentTypeDefault":media.wire.media_type().declared()}})));}
            }
            let t=body.choice_name.as_ref().map_or_else(||payload(&plan,&body.media[0]),|name|named(name));
            json!({"member":"body","required":body.required,"model":body.native_type,"type":presence(t,body.required),"initialization":initial(body.required)})
        });
        let mut responses = Vec::new();
        for status in &op.statuses {
            if let Some(h) = &status.headers {
                records.push(headers(&plan, &status.source, h));
            }
            let t = status.choice_name.as_ref().map_or_else(
                || {
                    if status.media.is_empty() {
                        named(&status.native_type)
                    } else {
                        payload(&plan, &status.media[0])
                    }
                },
                |n| named(n),
            );
            if let Some(name) = &status.choice_name {
                records.push(record(&contract,&status.source,name,"response-media",json!({"kind":"class","modifiers":["sealed"],"constructor":null,"variants":status.media.iter().map(|m|json!({"name":m.variant_name,"type":payload(&plan,m)})).collect::<Vec<_>>(),"noBody":status.none_variant,"undeclaredBytes":status.bytes_variant})));
                for media in &status.media {
                    records.push(record(&contract,media.wire.source().use_site().source(),&media.variant_name,"response-media-variant",json!({"kind":"class","modifiers":["final"],"constructor":null,"extends":name,"fields":[field("value",payload(&plan,media),true)]})));
                }
                if let Some(variant) = &status.none_variant {
                    records.push(record(&contract,&status.source,variant,"response-no-body",json!({"kind":"class","modifiers":["final"],"constructor":null,"extends":name,"fields":[]})));
                }
                if let Some(variant) = &status.bytes_variant {
                    records.push(record(&contract,&status.source,variant,"response-undeclared-bytes",json!({"kind":"class","modifiers":["final"],"constructor":null,"extends":name,"fields":[field("value",named("Uint8List"),true)]})));
                }
            }
            for (name, parent, role) in [
                (
                    status.success_name.as_ref(),
                    &op.success_type,
                    "success-response",
                ),
                (status.error_name.as_ref(), &op.error_type, "error-response"),
            ] {
                if let Some(name) = name {
                    let mut members = vec![field("data", t.clone(), true)];
                    if let Some(h) = &status.headers {
                        members.push(field("headers", named(&h.name), true));
                    }
                    let desc = json!({"kind":"class","modifiers":["final"],"constructor":null,"fields":members,"membership":{"kind":"extends","type":parent},"statusPattern":status.wire.status_key(),"actualStatus":true});
                    records.push(record(&contract, &status.source, name, role, desc));
                }
            }
            responses.push(json!({"pattern":status.wire.status_key(),"success":status.success_name,"error":status.error_name,"dataType":t,
                "headers":status.headers.as_ref().map(|h|&h.name),"media":status.media.iter().map(|m|json!({"declared":m.wire.media_type().declared(),"type":payload(&plan,m),"kind":match m.payload{P::Json{..}=>"json",P::Text{..}=>"text",P::Bytes{..}=>"bytes",P::Stream{..}=>"stream",P::Aggregate(_)=>"parts"}})).collect::<Vec<_>>(),
                "links":status.wire.links().iter().map(|l|json!({"name":l.name(),"target":match l.target(){w::LinkTarget::OperationId{value,..}|w::LinkTarget::OperationRef{value,..}=>value.value()},"parameters":l.parameters().iter().map(|(name,value)|(name,value.value())).collect::<BTreeMap<_,_>>(),"requestBody":l.request_body().map(|v|json!({"kind":"literal","value":v.value()}))})).collect::<Vec<_>>()}));
        }
        for (name, role, base) in [
            (&op.success_type, "success-union", "SdkResponse"),
            (&op.error_type, "error-union", "ApiException"),
        ] {
            records.push(record(
                &contract,
                &op.source,
                name,
                role,
                json!({"kind":"class","modifiers":["sealed"],"extends":base,"constructor":null}),
            ));
        }
        let success_names = op
            .statuses
            .iter()
            .filter_map(|s| s.success_name.as_ref())
            .collect::<Vec<_>>();
        let result = if let [s] = success_names.as_slice() {
            (*s).clone()
        } else {
            op.success_type.clone()
        };
        let security=op.wire.security().alternatives().iter().map(|a|a.requirements().iter().map(|r|{
            let c=plan.credentials().iter().find(|c|&c.source==r.scheme().use_site().source()).expect("allocated scheme");
            json!({"name":c.name,"wire":r.name(),"type":credential_type(c),"urlBase":r.credential().url_base(),"scopesOrRoles":match r.permissions(){w::Permissions::Scopes(v)|w::Permissions::Roles(v)=>v.iter().map(|v|v.value()).collect::<Vec<_>>()},"kind":match r.credential(){w::CredentialHook::Bearer{..}=>"bearer",w::CredentialHook::Basic=>"basic",w::CredentialHook::ApiKey{..}=>"api-key",w::CredentialHook::OAuth2{..}=>"oauth2",w::CredentialHook::OpenIdConnect{..}=>"openid-connect"}})
        }).collect::<Vec<_>>()).collect::<Vec<_>>();
        operations.push(NativeOperation{source:Location::at(&contract,&op.source),operation_id:op.operation_id.clone(),symbols:BTreeMap::from([("client".into(),"Client".into()),("method".into(),op.method_name.clone()),("success".into(),op.success_type.clone()),("api-error".into(),op.error_type.clone())]),
            descriptor:json!({"arguments":"named","inputObject":null,"parameters":parameters,"body":body,"responses":responses,
                "responseUnions":{"result":generic(if op.stream{"Stream"}else{"Future"},vec![named(&result)]),"directSingleSuccess":success_names.len()==1},
                "credential":{"alternatives":security,"anonymous":op.wire.security().alternatives().is_empty()||op.wire.security().alternatives().iter().any(w::SecurityAlternative::is_anonymous)},
                "call":{"emptyAllowed":op.parameters.iter().all(|p|!p.required)&&op.body.as_ref().is_none_or(|b|!b.required),"wireInputs":op.parameters.len()+usize::from(op.body.is_some())},
                "servers":op.wire.servers().candidates().iter().map(|s|json!({"template":s.template(),"name":s.name().map(|n|n.value()),"documentBase":s.document_base(),"urlBase":s.url_base(),"resource":s.source().and_then(w::Provenance::terminal_resource),"variables":s.variables().iter().map(|v|json!({"name":v.name(),"default":v.default().value(),"values":v.values().map(|v|v.iter().map(|v|v.value()).collect::<Vec<_>>())})).collect::<Vec<_>>()})).collect::<Vec<_>>()})});
    }
    let root = SourceId::new(contract.entry().clone(), Default::default());
    records.push(record(&contract,&root,"Credentials","credentials",json!({"kind":"class","modifiers":["final"],"fields":plan.credentials().iter().map(|c|field(&c.name,credential_type(c),true)).collect::<Vec<_>>(),"constructor":{"name":"Credentials","const":true,"arguments":"named","parameters":plan.credentials().iter().map(|c|parameter(&c.name,credential_type(c),false,null())).collect::<Vec<_>>()}})));
    let mut client_parameters = vec![
        parameter("transport", named("HttpTransport"), true, initial(true)),
        parameter(
            "credentials",
            named("Credentials"),
            false,
            if plan.credential_env().is_some() {
                json!({"kind":"omitted","runtimeDefault":"environment-snapshot"})
            } else {
                json!({"kind":"constructor","name":"Credentials"})
            },
        ),
        parameter("server", nullable(named("Uri")), false, null()),
        parameter(
            "serverSelection",
            nullable(named("ServerSelection")),
            false,
            null(),
        ),
        parameter(
            "timeout",
            named("Duration"),
            false,
            json!({"kind":"duration","seconds":30}),
        ),
    ];
    if plan.credential_env().is_some() {
        client_parameters.push(parameter("environment",nullable(json!({"kind":"function","parameters":[primitive("String")],"result":nullable(primitive("String"))})),false,null()));
    }
    for (name, value) in [
        ("maxRequestBytes", plan.config().max_request_bytes),
        ("maxResponseBytes", plan.config().max_response_bytes),
        ("maxCaptureBytes", plan.config().max_capture_bytes),
        (
            "maxResponseHeaderBytes",
            plan.config().max_response_header_bytes,
        ),
        (
            "maxStreamBufferBytes",
            plan.config().max_stream_buffer_bytes,
        ),
    ] {
        client_parameters.push(parameter(
            name,
            primitive("int"),
            false,
            json!({"kind":"literal","value":value}),
        ));
    }
    records.push(record(&contract,&root,"Client","client",json!({"kind":"class","modifiers":["final"],"constructor":{"name":"Client","arguments":"named","parameters":client_parameters},
        "methods":[method("close",vec![],generic("Future",vec![primitive("void")]))]})));
    snapshot.models = records;
    snapshot.operations = operations;
    Ok(())
}
