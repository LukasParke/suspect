//! Kotlin interface capture from retained shapes and collision-allocated symbols.
//!
//! Schema addresses are correspondence metadata. Native types refer to actual
//! Kotlin declarations, never validation-program indices or rendered source.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{
    backend,
    kotlin_sdk::{
        self, NativeType, Plan, PlannedOperation,
        models::{Additional, Field, ModelPlan, Shape, Symbol},
    },
};

use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};

fn qualified(package: &str, name: &str) -> String {
    format!("{package}.{name}")
}

fn named(name: impl Into<String>) -> Value {
    json!({"kind":"named", "name":name.into()})
}

fn primitive(name: &str) -> Value {
    json!({"kind":"primitive", "name":name})
}

fn generic(name: impl Into<String>, arguments: Vec<Value>) -> Value {
    json!({"kind":"generic", "name":name.into(), "arguments":arguments})
}

fn nullable(value: Value) -> Value {
    if value["kind"] == "nullable" {
        value
    } else {
        json!({"kind":"nullable", "type":value})
    }
}

fn presence(package: &str, value: Value) -> Value {
    generic(qualified(package, "Presence"), vec![value])
}

fn type_at(models: &ModelPlan, source: &SchemaId, package: &str) -> Value {
    let symbol = models.symbol(source).expect("admitted Kotlin type");
    let value = match &symbol.shape {
        Shape::Any => named(qualified(package, "JsonValue")),
        Shape::Never => primitive("kotlin.Nothing"),
        Shape::Null => nullable(primitive("kotlin.Nothing")),
        Shape::Boolean => primitive("kotlin.Boolean"),
        Shape::String => primitive("kotlin.String"),
        Shape::Number => named(qualified(package, "JsonNumber")),
        Shape::Alias(target) => type_at(models, target, package),
        Shape::Array(item) => generic(
            "kotlin.collections.List",
            vec![item.as_ref().map_or_else(
                || named(qualified(package, "JsonValue")),
                |source| type_at(models, source, package),
            )],
        ),
        Shape::Object { .. } | Shape::StringEnum(_) | Shape::Union { .. } | Shape::CheckedJson => {
            named(qualified(package, &symbol.name))
        }
    };
    if symbol.nullable {
        nullable(value)
    } else {
        value
    }
}

fn input_type(models: &ModelPlan, source: &SchemaId, required: bool, package: &str) -> Value {
    let value = type_at(models, source, package);
    if required {
        value
    } else {
        presence(package, value)
    }
}

fn native_type(plan: &Plan, ty: &NativeType) -> Value {
    let package = &plan.config().package_name;
    match ty {
        NativeType::Model(source) => type_at(plan.models(), source, package),
        NativeType::Json => named(qualified(package, "JsonValue")),
        NativeType::String => primitive("kotlin.String"),
        NativeType::Boolean => primitive("kotlin.Boolean"),
        NativeType::Number => named(qualified(package, "JsonNumber")),
        NativeType::Bytes => primitive("kotlin.ByteArray"),
        NativeType::Unit => primitive("kotlin.Unit"),
        NativeType::Named(name) => named(qualified(package, name)),
        NativeType::List(inner) => {
            generic("kotlin.collections.List", vec![native_type(plan, inner)])
        }
        NativeType::Flow(inner) => generic(
            "kotlinx.coroutines.flow.Flow",
            vec![native_type(plan, inner)],
        ),
        NativeType::Presence(inner) => presence(package, native_type(plan, inner)),
    }
}

fn wire_input_type(plan: &Plan, ty: &NativeType, required: bool) -> Value {
    let value = native_type(plan, ty);
    if required {
        value
    } else {
        presence(&plan.config().package_name, value)
    }
}

fn codec_name(models: &ModelPlan, source: &SchemaId, package: &str) -> String {
    qualified(
        package,
        &format!(
            "Codecs.{}",
            models.symbol(source).expect("admitted codec").codec_name
        ),
    )
}

fn codec_type(value: Value, package: &str) -> Value {
    generic(qualified(package, "ModelCodec"), vec![value])
}

fn codec_methods(value: &Value, package: &str) -> Value {
    let limits = || {
        argument(
            "limits",
            named(qualified(package, "CodecLimits")),
            Some(construct(&qualified(package, "CodecLimits"))),
        )
    };
    let method = |name: &str, parameter: Value, result: Value| {
        json!({
            "name":name,"access":"public","parameters":[parameter,limits()],"returns":result,
        })
    };
    json!([
        method(
            "decode",
            argument("text", primitive("kotlin.String"), None),
            value.clone()
        ),
        method(
            "decode",
            argument("bytes", primitive("kotlin.ByteArray"), None),
            value.clone()
        ),
        method(
            "decodeJson",
            argument("value", named(qualified(package, "JsonValue")), None),
            value.clone()
        ),
        method(
            "encode",
            argument("value", value.clone(), None),
            primitive("kotlin.String")
        ),
        method(
            "encodeJson",
            argument("value", value.clone(), None),
            named(qualified(package, "JsonValue"))
        ),
    ])
}

fn absent(package: &str) -> Value {
    json!({"kind":"singleton", "name":qualified(package,"Presence.Absent")})
}

fn construct(name: &str) -> Value {
    json!({"kind":"constructor-call", "name":name, "arguments":[]})
}

fn argument(name: &str, ty: Value, default: Option<Value>) -> Value {
    json!({"name":name,"type":ty,"required":default.is_none(),"hasDefault":default.is_some(),
        "initialization":default.unwrap_or_else(||json!({"kind":"argument"}))})
}

fn property(name: &str, ty: Value, initialization: Value) -> Value {
    json!({"kind":"property","name":name,"type":ty,"access":"public","getter":true,
        "setter":null,"mutable":false,"initialization":initialization})
}

fn constructor(name: &str, arguments: Vec<Value>) -> Value {
    json!({"name":name,"access":"public","style":"primary-constructor","argumentStyle":"positional-or-named",
        "canInitializeWithoutArguments":arguments.iter().all(|p|p["hasDefault"] == true),"parameters":arguments})
}

fn fixed_getter(models: &ModelPlan, field: &Field, package: &str) -> Option<Value> {
    field.constant.as_ref()?;
    let target = models.target(&field.schema);
    let Shape::StringEnum(cases) = &target.shape else {
        unreachable!("planned fixed string tag")
    };
    let [(case, wire)] = cases.as_slice() else {
        unreachable!("planned singleton enum")
    };
    Some(
        json!({"kind":"literal","type":qualified(package,&target.name),"member":case,"value":wire}),
    )
}

fn field(plan: &Plan, field: &Field) -> Value {
    let package = &plan.config().package_name;
    let ty = input_type(plan.models(), &field.schema, field.required, package);
    let fixed = fixed_getter(plan.models(), field, package);
    let initialization = fixed.clone().unwrap_or_else(|| {
        if field.required {
            json!({"kind":"constructor-argument","name":field.name})
        } else {
            absent(package)
        }
    });
    let mut value = property(&field.name, ty, initialization);
    value["wire"] = json!(field.wire_name);
    value["sourceRequired"] = json!(field.required);
    value["constructorParameter"] = json!(fixed.is_none());
    value["getterKind"] = json!(if fixed.is_some() {
        "fixed-enum-member"
    } else {
        "stored-val"
    });
    value
}

fn extra_type(plan: &Plan, extra: &Additional) -> Option<Value> {
    let package = &plan.config().package_name;
    let value = match extra {
        Additional::Closed => return None,
        Additional::Any | Additional::Scoped => named(qualified(package, "JsonValue")),
        Additional::Typed(source) => type_at(plan.models(), source, package),
    };
    Some(generic(
        "kotlin.collections.Map",
        vec![primitive("kotlin.String"), value],
    ))
}

fn union_arm(plan: &Plan, parent: &str, name: &str, source: &SchemaId) -> Value {
    let ty = type_at(plan.models(), source, &plan.config().package_name);
    let name = format!("{parent}.{name}");
    json!({"kind":"data-class","access":"public","final":true,"implements":[parent],
        "fields":[property("value",ty.clone(),json!({"kind":"constructor-argument","name":"value"}))],
        "constructor":constructor(&name,vec![argument("value",ty,None)]),
        "copy":true,"componentMembers":["component1"],"encodingValidation":"selected-arm-and-parent"})
}

fn model(plan: &Plan, symbol: &Symbol) -> Option<Value> {
    let package = &plan.config().package_name;
    let name = qualified(package, &symbol.name);
    Some(match &symbol.shape {
        Shape::CheckedJson => {
            let ty = named(qualified(package, "JsonValue"));
            let parameters = vec![argument("value", ty.clone(), None)];
            json!({"kind":"data-class","access":"public","final":true,"representation":"schema-bound-json-carrier",
                "fields":[property("value",ty.clone(),json!({"kind":"constructor-argument","name":"value"}))],
                "constructor":constructor(&name,parameters),"copy":{"name":"copy","returns":named(&name),"parameters":[argument("value",ty,Some(json!({"kind":"current-property","name":"value"})))]},
                "componentMembers":["component1"],"codec":{"owner":format!("{name}.Companion"),"member":"codec","target":codec_name(plan.models(),&symbol.source,package),"type":codec_type(named(&name),package)},
                "validation":"complete-scoped-schema-on-encode-and-decode","modelOnlyObligation":"use-source-bound-codec"})
        }
        Shape::Object { fields, additional } => {
            let mut properties = fields.iter().map(|f| field(plan, f)).collect::<Vec<_>>();
            let mut parameters = fields
                .iter()
                .filter(|f| f.constant.is_none())
                .map(|f| {
                    argument(
                        &f.name,
                        input_type(plan.models(), &f.schema, f.required, package),
                        (!f.required).then(|| absent(package)),
                    )
                })
                .collect::<Vec<_>>();
            if let Some(ty) = extra_type(plan, additional) {
                let initial = json!({"kind":"empty-map","function":"kotlin.collections.emptyMap"});
                properties.push(property(
                    "additionalProperties",
                    ty.clone(),
                    initial.clone(),
                ));
                parameters.push(argument("additionalProperties", ty, Some(initial)));
            }
            let data_class = !parameters.is_empty();
            let components = (1..=parameters.len())
                .map(|i| format!("component{i}"))
                .collect::<Vec<_>>();
            json!({"kind":if data_class {"data-class"} else {"class"},"access":"public","final":true,
                "fields":properties,"constructor":constructor(&qualified(package,symbol.constructor.as_ref().expect("planned object constructor")),parameters.clone()),
                "copy":if data_class {Some(json!({"name":"copy","returns":named(&name),
                    "parameters":parameters.iter().map(|p|argument(p["name"].as_str().unwrap(),p["type"].clone(),Some(json!({"kind":"current-property","name":p["name"]})))).collect::<Vec<_>>()}))} else {None},
                "componentMembers":if data_class {components} else {Vec::new()},
                "codec":{"owner":format!("{name}.Companion"),"receiver":"companion-object","member":"codec","mutable":false,
                    "type":codec_type(type_at(plan.models(),&symbol.source,package),package),"target":codec_name(plan.models(),&symbol.source,package)},
                "encodingValidation":"current-native-value"})
        }
        Shape::StringEnum(cases) => {
            json!({"kind":"enum","access":"public","constructorAccess":"private",
            // Enum ordinals are native API positions, not validation graph indices.
            "members":cases.iter().enumerate().map(|(ordinal,(member,wire))|json!({"kind":"literal","name":member,"ordinal":ordinal,"value":wire})).collect::<Vec<_>>(),
            "wireValue":property("wireValue",primitive("kotlin.String"),json!({"kind":"enum-constructor-argument"}))})
        }
        Shape::Union { variants, .. } => {
            json!({"kind":"sealed-interface","access":"public","closed":true,
            "variants":variants.iter().map(|(variant,source)|json!({"name":format!("{name}.{variant}"),"declaration":union_arm(plan,&name,variant,source)})).collect::<Vec<_>>(),
            "encodingValidation":"selected-arm-and-parent","decodingSelection":"first-valid-source-arm"})
        }
        // Aliases and primitives have source-bound codecs but no invented class
        // or typealias export under the internal schema naming hint.
        _ => return None,
    })
}

fn credentials(plan: &Plan, operation: &PlannedOperation) -> Value {
    let package = &plan.config().package_name;
    let initial = json!({"kind":"literal","value":null});
    let fields = plan
        .credentials()
        .values()
        .map(|credential| {
            let mut field = property(
                &credential.name,
                credential_type(plan, credential),
                initial.clone(),
            );
            field["wire"] = json!(credential.scheme_name);
            field
        })
        .collect::<Vec<_>>();
    json!({"kind":"source-security-alternatives","class":qualified(package,"Credentials"),"fields":fields,
        "constructor":constructor(&qualified(package,"Credentials"),plan.credentials().values().map(|c|argument(&c.name,credential_type(plan,c),Some(initial.clone()))).collect()),
        "selectedMember":operation.credentials.first().map(|c|&c.name),
        "alternatives":operation.wire.security().alternatives().iter().map(|a|a.requirements().iter().map(|c|plan.credentials()[plan.credential_source(c)].name.clone()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "runtimeRequired":!operation.wire.security().alternatives().is_empty()&&!operation.wire.security().alternatives().iter().any(|a|a.is_anonymous()),
        "providerMethod":{"name":"authorization","kind":"suspend-method","context":named(qualified(package,"CredentialContext")),"returns":primitive("kotlin.String")}})
}

fn credential_type(plan: &Plan, credential: &kotlin_sdk::PlannedCredential) -> Value {
    use crate::http_protocol::CredentialHook;
    nullable(match credential.wire.credential() {
        CredentialHook::Basic => named(qualified(&plan.config().package_name, "BasicCredentials")),
        CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. } => {
            named(qualified(&plan.config().package_name, "CredentialProvider"))
        }
        _ => primitive("kotlin.String"),
    })
}

fn request_controls(package: &str) -> Value {
    let optional = |name: &str, ty: Value| {
        argument(
            name,
            nullable(ty),
            Some(json!({"kind":"literal","value":null})),
        )
    };
    json!({"type":qualified(package,"RequestOptions"),"constructor":constructor(&qualified(package,"RequestOptions"),vec![
        optional("timeout",named("java.time.Duration")),optional("serverIndex",primitive("kotlin.Int")),
        optional("serverVariables",generic("kotlin.collections.Map",vec![primitive("kotlin.String"),primitive("kotlin.String")])),
        optional("documentUrl",named("java.net.URI")),optional("securityAlternative",primitive("kotlin.Int")),
        optional("requestMedia",primitive("kotlin.String")),optional("responseMedia",primitive("kotlin.String")),
    ]),"runtime":runtime_types(package)})
}

fn runtime_types(package: &str) -> Value {
    let ty = |name: &str| named(qualified(package, name));
    let string = || primitive("kotlin.String");
    let integer = || primitive("kotlin.Int");
    let bytes = || primitive("kotlin.ByteArray");
    let headers = || {
        generic(
            "kotlin.collections.Map",
            vec![string(), generic("kotlin.collections.List", vec![string()])],
        )
    };
    let variables = || generic("kotlin.collections.Map", vec![string(), string()]);
    let null = || Some(json!({"kind":"literal","value":null}));
    let lit = |value: Value| Some(json!({"kind":"literal","value":value}));
    let declaration = |name: &str, args: Vec<Value>| constructor(&qualified(package, name), args);
    json!({
        "ClientOptions":declaration("ClientOptions",vec![
            argument("serverUrl",nullable(named("java.net.URI")),null()),
            argument("timeout",named("java.time.Duration"),Some(json!({"kind":"static-call","name":"java.time.Duration.ofSeconds","arguments":[30]}))),
            argument("maxResponseBytes",integer(),lit(json!(4194304))),
            argument("captureBytes",integer(),Some(json!({"kind":"expression","value":"minOf(8192, maxResponseBytes)"}))),
            argument("maxRequestBytes",integer(),lit(json!(4194304))),
            argument("codecLimits",ty("CodecLimits"),Some(construct(&qualified(package,"CodecLimits")))),
            argument("serverIndex",integer(),lit(json!(0))),argument("serverVariables",variables(),Some(json!({"kind":"empty-map"}))),
            argument("documentUrl",nullable(named("java.net.URI")),null()),
            argument("maxChunkBytes",integer(),lit(json!(65536))),argument("maxStreamItemBytes",integer(),lit(json!(1048576))),
        ]),
        "BasicCredentials":declaration("BasicCredentials",vec![argument("username",string(),None),argument("password",string(),None)]),
        "Upload":declaration("Upload",vec![argument("data",bytes(),None),argument("filename",nullable(string()),null()),argument("contentType",nullable(string()),null())]),
        "CredentialContext":declaration("CredentialContext",vec![argument("operationId",string(),None),argument("scheme",string(),None),
            argument("scopes",generic("kotlin.collections.List",vec![string()]),None),argument("roles",generic("kotlin.collections.List",vec![string()]),None),
            argument("source",ty("SourceLocation"),None),argument("metadata",ty("JsonObject"),None),argument("effectiveServer",nullable(named("java.net.URI")),null())]),
        "LinkMetadata":declaration("LinkMetadata",vec![argument("name",string(),None),argument("declaration",ty("JsonObject"),None)]),
        "ResponseInfo":declaration("ResponseInfo",vec![argument("status",integer(),None),argument("headers",headers(),None),argument("bodyPreview",bytes(),None),
            argument("truncated",primitive("kotlin.Boolean"),None),argument("headersTruncated",primitive("kotlin.Boolean"),lit(json!(false))),
            argument("contentType",nullable(string()),null()),argument("links",generic("kotlin.collections.List",vec![ty("LinkMetadata")]),Some(json!({"kind":"empty-list"})))]),
        "StreamingResponse":declaration("StreamingResponse",vec![argument("status",integer(),None),argument("headers",headers(),None),argument("body",ty("BodyReader"),None)]),
        "BodyReader":{"kind":"interface","type":ty("BodyReader"),"extends":["java.lang.AutoCloseable"],"methods":[
            {"name":"read","kind":"suspend-method","parameters":[],"returns":nullable(bytes())},
            {"name":"close","kind":"method","parameters":[],"returns":primitive("kotlin.Unit")}]},
        "StreamingTransport":{"kind":"interface","type":ty("StreamingTransport"),"extends":[ty("Transport")],"methods":[
            {"name":"open","kind":"suspend-method","parameters":[argument("request",ty("HttpRequest"),None)],"returns":ty("StreamingResponse")},
            {"name":"execute","kind":"suspend-method","parameters":[argument("request",ty("HttpRequest"),None)],"returns":ty("HttpResponse"),"hasDefault":true}]},
    })
}

fn wire_declarations(plan: &Plan, snapshot: &mut NativeSnapshot) {
    let package = &plan.config().package_name;
    let mut seen = std::collections::BTreeSet::new();
    let mut add = |source: &SourceId, name: &str, role: &str, fields: Vec<Value>| {
        if !seen.insert(name.to_owned()) {
            return;
        }
        let arguments = fields
            .iter()
            .map(|f| {
                argument(
                    f["name"].as_str().unwrap(),
                    f["type"].clone(),
                    if f["hasDefault"] == true {
                        Some(f["initialization"].clone())
                    } else {
                        None
                    },
                )
            })
            .collect::<Vec<_>>();
        let copy=(!arguments.is_empty()).then(||json!({"name":"copy","returns":named(qualified(package,name)),"parameters":arguments.iter().map(|p|argument(p["name"].as_str().unwrap(),p["type"].clone(),Some(json!({"kind":"current-property","name":p["name"]})))).collect::<Vec<_>>()}));
        snapshot.models.push(NativeModel{source:Location::at(plan.contract(),source),name:qualified(package,name),role:role.into(),descriptor:Some(json!({"kind":if fields.is_empty(){"class"}else{"data-class"},"fields":fields,"copy":copy,"componentMembers":(1..=arguments.len()).map(|i|format!("component{i}")).collect::<Vec<_>>(),"constructor":constructor(&qualified(package,name),arguments)}))});
    };
    let field = |name: &str, ty: Value, default: Option<Value>| {
        let mut result = property(
            name,
            ty,
            default
                .clone()
                .unwrap_or_else(|| json!({"kind":"constructor-argument","name":name})),
        );
        result["hasDefault"] = json!(default.is_some());
        result
    };
    for op in plan.operations() {
        for media in op
            .body
            .iter()
            .flat_map(|b| b.media.iter())
            .chain(op.responses.iter().filter_map(|r| r.media.as_ref()))
        {
            if let Some(form) = &media.form {
                let mut fields = form
                    .fields
                    .iter()
                    .map(|p| {
                        field(
                            &p.name,
                            wire_input_type(plan, &p.ty, p.wire.required()),
                            (!p.wire.required()).then(|| absent(package)),
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(extra) = &form.additional {
                    fields.push(field(
                        "additionalProperties",
                        generic(
                            "kotlin.collections.Map",
                            vec![primitive("kotlin.String"), native_type(plan, &extra.ty)],
                        ),
                        Some(json!({"kind":"empty-map"})),
                    ));
                }
                add(&form.source, &form.name, "wire-body", fields);
                for part in form
                    .fields
                    .iter()
                    .chain(form.additional.iter().map(|p| p.as_ref()))
                {
                    if let Some(name) = &part.headers_type {
                        add(
                            part.wire.source().use_site().source(),
                            name,
                            "part-headers",
                            part.headers
                                .iter()
                                .map(|h| {
                                    field(
                                        &h.name,
                                        wire_input_type(plan, &h.ty, h.wire.required()),
                                        (!h.wire.required()).then(|| absent(package)),
                                    )
                                })
                                .collect(),
                        );
                    }
                    if let Some(name) = &part.wrapper {
                        let mut fields =
                            vec![field("value", native_type(plan, &part.value_type), None)];
                        if let Some(headers) = &part.headers_type {
                            fields.push(field(
                                "headers",
                                named(qualified(package, headers)),
                                part.headers
                                    .iter()
                                    .all(|h| !h.wire.required())
                                    .then(|| construct(&qualified(package, headers))),
                            ));
                        }
                        fields.push(field(
                            "contentType",
                            nullable(primitive("kotlin.String")),
                            Some(json!({"kind":"literal","value":null})),
                        ));
                        add(
                            part.wire.source().use_site().source(),
                            name,
                            "part-wrapper",
                            fields,
                        );
                    }
                }
            }
        }
        for response in &op.responses {
            if let Some(name) = &response.headers_type {
                add(
                    &response.source,
                    name,
                    "response-headers",
                    response
                        .headers
                        .iter()
                        .map(|h| {
                            field(
                                &h.name,
                                wire_input_type(plan, &h.ty, h.wire.required()),
                                (!h.wire.required()).then(|| absent(package)),
                            )
                        })
                        .collect(),
                );
            }
        }
    }
    for op in plan.operations() {
        if let Some(body) = &op.body
            && let Some(name) = &body.choice_type
        {
            snapshot.models.push(NativeModel{source:Location::at(plan.contract(),&body.source),name:qualified(package,name),role:"request-media-choice".into(),descriptor:Some(json!({"kind":"sealed-interface","variants":body.media.iter().map(|m|json!({"name":qualified(package,&format!("{name}.{}",m.name)),"constructor":constructor(&qualified(package,&format!("{name}.{}",m.name)),vec![argument("value",native_type(plan,&m.ty),None)])})).collect::<Vec<_>>()}))});
        }
    }
}

fn operation(plan: &Plan, op: &PlannedOperation) -> NativeOperation {
    let package = &plan.config().package_name;
    let input = qualified(package, &op.input_type);
    let result = qualified(package, &op.result_type);
    let error = qualified(package, &op.error_type);
    let input_parameters = op
        .parameters
        .iter()
        .map(|p| {
            argument(
                &p.name,
                input_type(plan.models(), &p.schema, p.required, package),
                (!p.required).then(|| absent(package)),
            )
        })
        .chain(op.body.iter().map(|b| {
            argument(
                &b.name,
                wire_input_type(plan, &b.ty, b.required),
                (!b.required).then(|| absent(package)),
            )
        }))
        .collect::<Vec<_>>();
    let slot = |name: &str, wire: &str, source: &SchemaId, required: bool| {
        json!({
            "member":name,"wire":wire,"model":type_at(plan.models(),source,package),"required":required,
            "type":input_type(plan.models(),source,required,package),"getter":true,"mutable":false,
            "hasDefault":!required,"initialization":if required {json!({"kind":"argument"})} else {absent(package)},
            "codec":codec_name(plan.models(),source,package),
        })
    };
    let direct_data = op.result_data_type.as_ref().map(|_| {
        let response = op
            .responses
            .iter()
            .find(|r| r.success)
            .expect("planned direct result data");
        property(
            "data",
            native_type(plan, &response.ty),
            json!({"kind":"abstract-val"}),
        )
    });
    let mut operation = NativeOperation {
        source: Location::at(plan.contract(), &op.source),
        operation_id: op.operation_id.clone(),
        symbols: BTreeMap::from([
            ("client".into(), qualified(package, "Client")),
            ("method".into(), op.method_name.clone()),
            ("input".into(), input.clone()),
            (
                "input-constructor".into(),
                qualified(package, &op.constructor),
            ),
            ("success".into(), result.clone()),
            ("api-error".into(), error.clone()),
        ]),
        descriptor: json!({
            // Packaging is recorded here; the shared comparator handles Maven
            // identity/version in TargetConfig rather than changing constructors.
            "package":{"groupId":plan.config().group_id,"artifactId":plan.config().artifact_id,"version":plan.config().version,"namespace":package},
            "constructor":{"input":constructor(&qualified(package,&op.constructor),input_parameters.clone()),
                "inputDeclaration":{"kind":if input_parameters.is_empty() {"class"} else {"data-class"},"final":true,"properties":"public-val"},
                "client":{"type":qualified(package,"Client"),"final":true,"implements":["java.lang.AutoCloseable"],
                    "signature":constructor(&qualified(package,"Client"),vec![argument("credentials",named(qualified(package,"Credentials")),plan.credential_env().is_none().then(||construct(&qualified(package,"Credentials")))),
                        argument("transport",named(qualified(package,"Transport")),Some(construct(&qualified(package,"JdkTransport")))),
                        argument("options",named(qualified(package,"ClientOptions")),Some(construct(&qualified(package,"ClientOptions"))))])}},
            "parameters":{"members":op.parameters.iter().map(|p| { let mut member = slot(&p.name,&p.wire_name,&p.schema,p.required); member["location"]=json!(format!("{:?}",p.wire.location())); member }).collect::<Vec<_>>(),
                "method":{"kind":if op.flow {"cold-flow-method"}else{"suspend-method"},"access":"public","receiver":qualified(package,"Client"),"name":op.method_name,
                    "parameters":[argument("input",named(&input),op.input_has_default().then(||construct(&qualified(package,&op.constructor)))),
                        argument("requestOptions",named(qualified(package,"RequestOptions")),Some(construct(&qualified(package,"RequestOptions"))))],
                    "returns":if op.flow {generic("kotlinx.coroutines.flow.Flow",vec![named(&result)])}else{named(&result)},"canCallWithoutInputs":op.input_has_default(),"cancellation":"caller-coroutine-context"},
                "controls":request_controls(package)},
            "body":op.body.as_ref().map(|b|json!({"member":b.name,"required":b.required,"type":wire_input_type(plan,&b.ty,b.required),"hasDefault":!b.required,"initialization":if b.required {json!({"kind":"argument"})}else{absent(package)},"mediaChoice":b.choice_type.as_ref().map(|n|qualified(package,n)),"representations":b.media.iter().map(|m|json!({"case":m.name,"type":native_type(plan,&m.ty),"codec":m.codec_name.as_ref().map(|c|qualified(package,&format!("Codecs.{c}")))})).collect::<Vec<_>>()})),
            "responseUnions":{"success":{"name":result,"kind":"sealed-interface","closed":true,
                    "response":property("response",named(qualified(package,"ResponseInfo")),json!({"kind":"abstract-val"})),"data":direct_data},
                "apiError":{"name":error,"kind":"sealed-class","closed":true,"base":qualified(package,"ApiException"),
                    "constructor":{"access":"protected","parameters":[argument("response",named(qualified(package,"ResponseInfo")),None)]}}},
            "responses":op.responses.iter().map(|r| {
                let ty = native_type(plan,&r.ty);
                json!({"type":qualified(package,&r.constructor),"variant":r.variant_name,"status":match r.status{crate::http_protocol::ResponseStatus::Exact(n)=>json!(n),_=>json!(r.status_key)},"mediaType":r.media.as_ref().map(|m|m.wire.media_type().declared()),"success":r.success,
                    "kind":if r.success {"data-class"} else {"class"},"final":true,"base":if r.success {&result} else {&error},
                    "constructor":constructor(&qualified(package,&r.constructor),vec![argument("data",ty.clone(),None),argument("response",named(qualified(package,"ResponseInfo")),None)].into_iter().chain(r.headers_type.iter().map(|name|argument("responseHeaders",named(qualified(package,name)),None))).collect()),
                    "fields":vec![property("data",ty,json!({"kind":"constructor-argument","name":"data"})),property("response",named(qualified(package,"ResponseInfo")),json!({"kind":"constructor-argument","name":"response"}))].into_iter().chain(r.headers_type.iter().map(|name|property("responseHeaders",named(qualified(package,name)),json!({"kind":"constructor-argument","name":"responseHeaders"})))).collect::<Vec<_>>(),
                    "dataOverrides":r.success && op.result_data_type.is_some(),"responseOverrides":r.success,"responseInherited":!r.success,
                    "streamItem":r.stream,"bodyDisposition":r.disposition,"headers":r.headers_type.as_ref().map(|name|qualified(package,name)),
                    "codec":r.codec_name.as_ref().map(|name|qualified(package,&format!("Codecs.{name}")))})
            }).collect::<Vec<_>>(),
            "credential":credentials(plan,op),
        }),
    };
    if plan.credential_env().is_some() {
        let arguments = vec![
            argument(
                "transport",
                named(qualified(package, "Transport")),
                Some(construct(&qualified(package, "JdkTransport"))),
            ),
            argument(
                "options",
                named(qualified(package, "ClientOptions")),
                Some(construct(&qualified(package, "ClientOptions"))),
            ),
        ];
        let client = &mut operation.descriptor["constructor"]["client"];
        client["omittedCredentials"] =
            constructor(&qualified(package, "Client"), arguments.clone());
        client["environmentFactory"] = json!({"owner":qualified(package,"Client.Companion"),"name":"fromEnv","kind":"method","parameters":arguments.into_iter().chain([argument("environment",named(qualified(package,"CredentialEnvironment")),Some(json!({"kind":"property","name":qualified(package,"CredentialEnvironment.system")})))]).collect::<Vec<_>>(),"returns":named(qualified(package,"Client")),"snapshot":"client-creation","explicitCredentials":"whole-argument-authoritative"});
        client["environmentReader"] = json!({"type":qualified(package,"CredentialEnvironment"),"kind":"fun-interface","method":{"name":"value","parameters":[argument("name",primitive("kotlin.String"),None)],"returns":nullable(primitive("kotlin.String"))}});
    }
    operation
}

/// Capture the same default package policy used by the public backend boundary.
pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    let mut config = backend::kotlin_config(&snapshot.target).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: error.source.as_ref().map(|s| Location::at(&contract, s)),
                message: error.message,
            })
            .collect::<Vec<_>>()
    })?;
    config.credential_env = snapshot.generation.credential_env.clone();
    let errors = |errors: Vec<kotlin_sdk::HttpDiagnostic>| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: Some(Location::at(&contract, &error.source)),
                message: error.message,
            })
            .collect::<Vec<_>>()
    };
    let plan = kotlin_sdk::plan_sdk_with_profiles(
        contract.clone(),
        selected,
        config,
        &snapshot.generation.compatibility_profiles,
    )
    .map_err(errors)?;
    // Artifact admission, including native-example/package preflight, is shared
    // with generation. No emitted Kotlin is inspected for interface facts.
    plan.render().map_err(errors)?;
    snapshot.credential_env = plan.credential_env().map(|env| env.semantic_descriptor());
    if plan.program().version != suspect_schema::OwnedProgram::V1_VERSION {
        snapshot.runtime.profile =
            format!("{}+{}", snapshot.runtime.profile, plan.program().profile);
    }
    let package = &plan.config().package_name;
    for op in plan.operations() {
        snapshot.operations.push(operation(&plan, op));
    }
    for symbol in plan.models().symbols() {
        let source = Location::at(&contract, &symbol.source);
        if let Some(descriptor) = model(&plan, symbol) {
            snapshot.models.push(NativeModel {
                source: source.clone(),
                name: qualified(package, &symbol.name),
                role: "model".into(),
                descriptor: Some(descriptor),
            });
        }
        if let Shape::Union { variants, .. } = &symbol.shape {
            let parent = qualified(package, &symbol.name);
            for (name, id) in variants {
                snapshot.models.push(NativeModel {
                    source: Location::at(&contract, id),
                    name: format!("{parent}.{name}"),
                    role: "union-arm".into(),
                    descriptor: Some(union_arm(&plan, &parent, name, id)),
                });
            }
        }
        snapshot.models.push(NativeModel { source,name:codec_name(plan.models(),&symbol.source,package),role:"codec".into(),
            descriptor:Some(json!({"kind":"property","owner":qualified(package,"Codecs"),"receiver":"singleton-object","member":symbol.codec_name,
                "access":"public","getter":true,"mutable":false,"type":codec_type(type_at(plan.models(),&symbol.source,package),package),
                "methods":codec_methods(&type_at(plan.models(),&symbol.source,package),package)})) });
    }
    wire_declarations(&plan, snapshot);
    Ok(())
}
