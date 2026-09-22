//! Java native compatibility from retained models and rich protocol bindings.
//! Runtime graph indices, source spans and rendered code are never API identities.
use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};
use crate::{
    backend,
    java_sdk::{
        self, SdkPlan,
        models::{JavaDeclaration, JavaSymbol, JavaType},
        protocol::{AggregateRules, JavaAggregate, JavaHeaders, JavaPart, JavaValue},
    },
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

fn name(plan: &SdkPlan, value: &str) -> String {
    format!("{}.{value}", plan.package().package)
}
fn nested(plan: &SdkPlan, value: &str) -> String {
    format!(
        "{}.{}.{value}",
        plan.package().package,
        plan.package().api_name
    )
}
fn named(value: impl Into<String>) -> Value {
    json!({"kind":"named","name":value.into()})
}
fn primitive(value: &str) -> Value {
    json!({"kind":"primitive","name":value})
}
fn generic(value: impl Into<String>, args: Vec<Value>) -> Value {
    json!({"kind":"generic","name":value.into(),"arguments":args})
}
fn nullable(value: Value) -> Value {
    if value["kind"] == "nullable" {
        value
    } else {
        json!({"kind":"nullable","type":value,"representation":"java-null"})
    }
}
fn string() -> Value {
    primitive("java.lang.String")
}
fn ty(plan: &SdkPlan, value: &JavaType) -> Value {
    match value {
        JavaType::String => string(),
        JavaType::Boolean => primitive("java.lang.Boolean"),
        JavaType::Number => named(name(plan, "JsonRuntime.JsonNumber")),
        JavaType::Json => named(name(plan, "JsonRuntime.JsonValue")),
        JavaType::Null => named(name(plan, "JsonRuntime.JsonNull")),
        JavaType::Never => named(name(plan, "Never")),
        JavaType::Nullable(v) => nullable(ty(plan, v)),
        JavaType::List(v) => generic("java.util.List", vec![ty(plan, v)]),
        JavaType::Named(id) => {
            let symbol = plan.models().symbol(id).expect("admitted model");
            match symbol.declaration() {
                JavaDeclaration::Alias(v) => ty(plan, v),
                JavaDeclaration::Union { .. } => named(name(plan, symbol.name())),
                _ => {
                    let v = named(name(plan, symbol.name()));
                    if symbol.nullable() { nullable(v) } else { v }
                }
            }
        }
    }
}
fn wire_ty(plan: &SdkPlan, value: &JavaValue) -> Value {
    match value {
        JavaValue::Model(id) => ty(plan, &JavaType::Named(id.clone())),
        JavaValue::Named(n) => named(name(plan, n)),
        JavaValue::Json => named(name(plan, "JsonRuntime.JsonValue")),
        JavaValue::Text => string(),
        JavaValue::Bytes => named(name(plan, "Bytes")),
        JavaValue::NoContent => named(name(plan, "NoContent")),
        JavaValue::Aggregate(a) => named(name(plan, &a.name)),
        JavaValue::ResponseBody(v) => generic(name(plan, "ResponseBody"), vec![wire_ty(plan, v)]),
        JavaValue::Stream { schema, request } => generic(
            if *request {
                "java.util.List".into()
            } else {
                name(plan, "EventStream")
            },
            vec![ty(plan, &JavaType::Named(schema.clone()))],
        ),
    }
}
fn presence(plan: &SdkPlan, value: Value, required: bool) -> Value {
    if required {
        value
    } else {
        generic(name(plan, "Presence"), vec![value])
    }
}
fn argument(n: &str, t: Value) -> Value {
    json!({"name":n,"type":t,"required":true,"hasDefault":false})
}
fn method(n: &str, args: Vec<Value>, result: Value, static_: bool) -> Value {
    json!({"name":n,"access":"public","static":static_,"parameters":args,"returns":result})
}
fn getter(n: &str, t: Value) -> Value {
    json!({"name":n,"type":t,"mutable":false,"setter":null,"getter":method(n,vec![],t.clone(),false)})
}
fn factory(owner: &str, args: Vec<Value>) -> Value {
    json!({"name":format!("{owner}.builder"),"member":"builder","owner":owner,"style":"builder-factory","access":"public","static":true,"parameters":args,"returns":named(format!("{owner}.Builder")),"canInitializeWithoutArguments":args.is_empty()})
}
fn wire_field(plan: &SdkPlan, owner: &str, n: &str, t: Value, required: bool) -> Value {
    let mut field = getter(n, presence(plan, t.clone(), required));
    field["storage"] = json!({"access":"private","final":true});
    field["required"] = json!(required);
    field["builderSetter"] = method(
        n,
        vec![argument("value", t)],
        named(format!("{owner}.Builder")),
        false,
    );
    field["omitter"] = if required {
        Value::Null
    } else {
        method(
            &format!("omit{}", crate::rust_models::pascal(n)),
            vec![],
            named(format!("{owner}.Builder")),
            false,
        )
    };
    field
}
fn builder_descriptor(owner: &str, fields: &[Value], extra: Vec<Value>) -> Value {
    json!({"name":format!("{owner}.Builder"),"access":"public","static":true,"final":true,"constructorAccess":"private","mutable":true,"build":method("build",vec![],named(owner),false),"buildResult":"independent-deep-immutable-snapshot",
        "setters":fields.iter().filter_map(|f|f.get("builderSetter")).filter(|v|!v.is_null()).cloned().chain(extra).collect::<Vec<_>>(),"omitters":fields.iter().filter_map(|f|f.get("omitter")).filter(|v|!v.is_null()).collect::<Vec<_>>()})
}
fn codec_name(plan: &SdkPlan, id: &SchemaId) -> String {
    let c = plan.models().codec(id);
    format!("{}.{}", name(plan, &c.holder), c.field)
}
fn fixed(plan: &SdkPlan, field: &crate::java_sdk::models::JavaField) -> Value {
    let Some(value) = &field.fixed else {
        return Value::Null;
    };
    let symbol = plan.models().symbol(&field.source).expect("fixed field");
    match symbol.declaration() {
        JavaDeclaration::Literals { values } if !value.is_null() => {
            let constant = values
                .iter()
                .find(|v| &v.value == value)
                .expect("fixed literal");
            json!({"kind":"literal","type":name(plan,symbol.name()),"member":constant.name,"value":value})
        }
        JavaDeclaration::Alias(JavaType::Null) => {
            json!({"kind":"literal","type":name(plan,"JsonRuntime.JsonNull"),"member":"INSTANCE","value":null})
        }
        _ => json!({"kind":"literal","value":value}),
    }
}
fn codec(plan: &SdkPlan, id: &SchemaId) -> Value {
    let binding = plan.models().codec(id);
    let t = ty(plan, &JavaType::Named(id.clone()));
    let json = named(name(plan, "JsonRuntime.JsonValue"));
    json!({"kind":"field","owner":name(plan,&binding.holder),"member":binding.field,"name":binding.field,"access":"public","static":true,"final":true,"mutable":false,"type":generic(name(plan,"ModelCodec"),vec![t.clone()]),"binding":"source-specific-checked-codec",
        "methods":[method("decode",vec![argument("json",string())],t.clone(),false),method("decode",vec![argument("json",named("byte[]"))],t.clone(),false),method("decodeValue",vec![argument("value",json.clone())],t.clone(),false),
            method("encode",vec![argument("value",t.clone())],string(),false),method("encodeValue",vec![argument("value",t.clone())],json,false),method("snapshot",vec![argument("value",t.clone())],t.clone(),false),
            method("withLimits",vec![argument("limits",named(name(plan,"ModelCodec.Limits")))],generic(name(plan,"ModelCodec"),vec![t]),false),method("limits",vec![],named(name(plan,"ModelCodec.Limits")),false),method("source",vec![],string(),false)]})
}
fn model(plan: &SdkPlan, symbol: &JavaSymbol) -> Value {
    let owner = name(plan, symbol.name());
    let builder = format!("{owner}.Builder");
    let t = ty(plan, &JavaType::Named(symbol.source().clone()));
    let mut descriptor = match symbol.declaration() {
        JavaDeclaration::Alias(_) => {
            json!({"kind":"codec-holder-class","access":"public","final":true,"constructorAccess":"private","declaresValueType":false,"nativeType":t})
        }
        JavaDeclaration::Literals { values } => {
            json!({"kind":"literal-class","access":"public","final":true,"constructorAccess":"private","mutable":false,"constants":values.iter().map(|v|json!({"kind":"literal","name":v.name,"type":owner,"value":v.value,"access":"public","static":true,"final":true})).collect::<Vec<_>>(),"fields":[getter("wireValue",named(name(plan,"JsonRuntime.JsonValue")))],"numericSpelling":"preserved"})
        }
        JavaDeclaration::Union {
            exclusive,
            variants,
        } => {
            json!({"kind":"sealed-interface","access":"public","closed":true,"exclusive":exclusive,"nullRepresentation":"selected-native-arm","variants":variants.iter().map(|v|json!({"name":format!("{owner}.{}",v.name),"declaration":union_arm(plan,&owner,v)})).collect::<Vec<_>>(),"encodingValidation":"selected-arm-and-parent"})
        }
        JavaDeclaration::Object {
            fields,
            extras,
            constructor,
        } => {
            let raw = crate::schema_view::raw(
                plan.contract()
                    .schema(symbol.source())
                    .expect("retained model"),
            );
            let required_additional = raw
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|name| fields.iter().all(|f| f.wire != *name))
                .collect::<Vec<_>>();
            let fields = fields
                .iter()
                .map(|f| {
                    let value = ty(plan, &f.ty);
                    let mut field = getter(&f.name, presence(plan, value.clone(), f.required));
                    field["wire"] = json!(f.wire);
                    field["sourceRequired"] = json!(f.required);
                    field["sourceNullable"] = json!(f.nullable);
                    field["storage"] = json!({"access":"private","final":true});
                    field["constructorParameter"] = json!(f.required && f.fixed.is_none());
                    field["fixed"] = fixed(plan, f);
                    field["initialization"] = if f.fixed.is_some() {
                        fixed(plan, f)
                    } else if f.required {
                        json!({"kind":"constructor-argument","name":f.name})
                    } else {
                        json!({"kind":"absent"})
                    };
                    field["builderSetter"] = if f.fixed.is_none() {
                        method(
                            &f.name,
                            vec![argument("value", value)],
                            named(&builder),
                            false,
                        )
                    } else {
                        Value::Null
                    };
                    field["omitter"] = f
                        .omit_method
                        .as_ref()
                        .map(|m| method(m, vec![], named(&builder), false))
                        .unwrap_or(Value::Null);
                    field["codec"] = json!(codec_name(plan, &f.source));
                    field
                })
                .chain(extras.iter().map(|v| {
                    let mut field = getter(
                        "additionalProperties",
                        generic("java.util.Map", vec![string(), ty(plan, v)]),
                    );
                    field["builderSetter"] = method(
                        "putAdditionalProperty",
                        vec![argument("name", string()), argument("value", ty(plan, v))],
                        named(&builder),
                        false,
                    );
                    field
                }))
                .collect::<Vec<_>>();
            let args = constructor
                .arguments
                .iter()
                .map(|a| argument(&a.name, ty(plan, &a.ty)))
                .collect::<Vec<_>>();
            json!({"kind":"class","access":"public","final":true,"constructorAccess":"private","mutable":false,"fields":fields,"requiredAdditionalProperties":required_additional,
                "constructor":{"name":format!("{owner}.{}",constructor.name),"member":constructor.name,"owner":owner,"style":"builder-factory","access":"public","static":true,"parameters":args,"returns":named(&builder),"canInitializeWithoutArguments":args.is_empty()},
                "builder":{"name":builder,"access":"public","static":true,"final":true,"constructorAccess":"private","mutable":true,"build":method("build",vec![],named(&owner),false),"buildResult":"independent-deep-immutable-snapshot",
                    "setters":fields.iter().filter_map(|f|f.get("builderSetter")).filter(|v|!v.is_null()).collect::<Vec<_>>(),"omitters":fields.iter().filter_map(|f|f.get("omitter")).filter(|v|!v.is_null()).collect::<Vec<_>>()},"containers":"deep-immutable-snapshots"})
        }
    };
    if matches!(symbol.declaration(), JavaDeclaration::Alias(JavaType::Json)) {
        descriptor["carrier"] = json!("immutable-json-value");
        descriptor["codecObligation"] = json!("source-bound-checked-decode-and-encode");
    }
    descriptor["codec"] = codec(plan, symbol.source());
    descriptor["conveniences"] = json!([
        method("decode", vec![argument("json", string())], t.clone(), true),
        method("encode", vec![argument("value", t)], string(), true)
    ]);
    descriptor
}
fn union_arm(plan: &SdkPlan, parent: &str, v: &crate::java_sdk::models::JavaVariant) -> Value {
    json!({"kind":"class","access":"public","static":true,"final":true,"implements":[parent],"constructor":{"name":format!("{parent}.{}",v.name),"access":"public","style":"constructor","parameters":v.constructor.arguments.iter().map(|a|argument(&a.name,ty(plan,&a.ty))).collect::<Vec<_>>(),"snapshot":"deep-immutable"},"fields":[getter("value",ty(plan,&v.ty))],"valueCodec":codec_name(plan,&v.source),"encodingValidation":"selected-arm-and-parent"})
}

fn part_type(plan: &SdkPlan, p: &JavaPart) -> Value {
    let v = p
        .wrapper
        .as_ref()
        .map_or_else(|| wire_ty(plan, &p.value), |n| named(name(plan, n)));
    if p.wire.multiplicity() == crate::http_protocol::PartMultiplicity::RepeatedArrayItems {
        generic("java.util.List", vec![v])
    } else {
        v
    }
}
fn header_model(plan: &SdkPlan, h: &JavaHeaders) -> Value {
    let owner = name(plan, &h.name);
    let args = h
        .fields
        .iter()
        .filter(|f| f.wire.required())
        .map(|f| argument(&f.name, ty(plan, &JavaType::Named(f.schema.clone()))))
        .collect::<Vec<_>>();
    let fields = h
        .fields
        .iter()
        .map(|f| {
            let mut value = wire_field(
                plan,
                &owner,
                &f.name,
                ty(plan, &JavaType::Named(f.schema.clone())),
                f.wire.required(),
            );
            value["wire"] = json!(f.wire.name());
            value["codec"] = json!(codec_name(plan, &f.schema));
            value
        })
        .collect::<Vec<_>>();
    json!({"kind":"header-class","access":"public","final":true,"constructorAccess":"private","fields":fields,
        "constructor":factory(&owner,args),"builder":builder_descriptor(&owner,&fields,vec![]),"snapshot":"deep-immutable"})
}
fn aggregate_model(plan: &SdkPlan, a: &JavaAggregate) -> Value {
    let owner = name(plan, &a.name);
    let builder = format!("{owner}.Builder");
    let positional = matches!(a.rules, AggregateRules::Positional { .. });
    let mut fields = a
        .parts
        .iter()
        .map(|p| {
            let mut value =
                wire_field(plan, &owner, &p.name, part_type(plan, p), p.wire.required());
            value["wire"] = json!(p.wire.name());
            value["valueCodec"] = p
                .value
                .schema()
                .map(|id| json!(codec_name(plan, id)))
                .unwrap_or(Value::Null);
            value
        })
        .collect::<Vec<_>>();
    let extra = a.additional.as_ref().map(|p| {
        if positional {
            let t = p
                .wrapper
                .as_ref()
                .map_or_else(|| wire_ty(plan, &p.value), |n| named(name(plan, n)));
            fields.push(getter("items", generic("java.util.List", vec![t.clone()])));
            method(
                "addItem",
                vec![argument("value", t)],
                named(&builder),
                false,
            )
        } else {
            let t = part_type(plan, p);
            fields.push(getter(
                "additionalParts",
                generic("java.util.Map", vec![string(), t.clone()]),
            ));
            method(
                "putAdditionalPart",
                vec![argument("name", string()), argument("value", t)],
                named(&builder),
                false,
            )
        }
    });
    json!({"kind":if matches!(a.rules,AggregateRules::Positional{..}){"positional-multipart"}else if a.multipart{"named-multipart"}else{"form"},"access":"public","final":true,"constructorAccess":"private",
        "fields":fields,"constructor":factory(&owner,a.parts.iter().filter(|p|p.wire.required()).map(|p|argument(&p.name,part_type(plan,p))).collect::<Vec<_>>()),
        "builder":builder_descriptor(&owner,&fields,extra.iter().cloned().collect()),"additional":extra,
        "codecDomain":"wire-parts-not-json","snapshot":"deep-immutable","structuralValidation":"source-required-and-cardinality"})
}
fn capture_value(plan: &SdkPlan, v: &JavaValue, out: &mut BTreeMap<String, NativeModel>) {
    if let JavaValue::Aggregate(a) = v {
        let n = name(plan, &a.name);
        out.insert(
            n.clone(),
            NativeModel {
                source: Location::at(plan.contract(), &a.source),
                name: n,
                role: "wire-model".into(),
                descriptor: Some(aggregate_model(plan, a)),
            },
        );
        for p in a.parts.iter().chain(a.additional.iter()) {
            if let Some(n) = &p.wrapper {
                let n = name(plan, n);
                let mut args = vec![argument("value", wire_ty(plan, &p.value))];
                if p.wire.content_types().len() > 1
                    || p.wire.content_types().first().is_some_and(|m| {
                        !matches!(m.range(), crate::http_protocol::MediaRange::Concrete { .. })
                    })
                {
                    args.push(argument("contentType", string()));
                }
                if let Some(h) = &p.headers
                    && h.fields.iter().any(|h| h.wire.required())
                {
                    args.push(argument("headers", named(name(plan, &h.name))));
                }
                let mut fields = vec![
                    wire_field(plan, &n, "value", wire_ty(plan, &p.value), true),
                    wire_field(plan, &n, "contentType", string(), true),
                    wire_field(plan, &n, "filename", nullable(string()), true),
                ];
                // Metadata absence is Java null, not Presence; setters take String.
                fields[2]["builderSetter"] = method(
                    "filename",
                    vec![argument("value", string())],
                    named(format!("{n}.Builder")),
                    false,
                );
                if let Some(h) = &p.headers {
                    fields.push(wire_field(
                        plan,
                        &n,
                        "headers",
                        named(name(plan, &h.name)),
                        true,
                    ));
                }
                out.insert(n.clone(),NativeModel{source:Location::at(plan.contract(),p.wire.source().use_site().source()),name:n.clone(),role:"wire-model".into(),descriptor:Some(json!({"kind":"part-class","access":"public","final":true,"constructorAccess":"private","constructor":factory(&n,args),"fields":fields,"builder":builder_descriptor(&n,&fields,vec![]),"value":wire_ty(plan,&p.value),"headers":p.headers.as_ref().map(|h|name(plan,&h.name)),"metadata":["contentType","filename"],"snapshot":"deep-immutable","bytes":"native-octets"}))});
            }
            if let Some(h) = &p.headers {
                let n = name(plan, &h.name);
                out.insert(
                    n.clone(),
                    NativeModel {
                        source: Location::at(plan.contract(), &h.source),
                        name: n,
                        role: "wire-model".into(),
                        descriptor: Some(header_model(plan, h)),
                    },
                );
            }
        }
    }
}
fn choice_model(
    plan: &SdkPlan,
    source: &SourceId,
    n: &str,
    media: &[crate::java_sdk::protocol::JavaMedia],
    request: bool,
) -> NativeModel {
    let owner = name(plan, n);
    NativeModel {
        source: Location::at(plan.contract(), source),
        name: owner.clone(),
        role: "wire-model".into(),
        descriptor: Some(
            json!({"kind":"sealed-media-class","request":request,"access":"public","closed":true,"abstract":true,"constructorAccess":"private","methods":[method("contentType",vec![],string(),false)],"variants":media.iter().map(|m|{
        let variant=format!("{owner}.{}",m.name);let mut constructors=Vec::new();
        if request{if matches!(m.wire.media_type().range(),crate::http_protocol::MediaRange::Concrete{..}){constructors.push(json!({"name":variant,"access":"public","parameters":[argument("value",wire_ty(plan,&m.value))]}));}constructors.push(json!({"name":variant,"access":"public","parameters":[argument("value",wire_ty(plan,&m.value)),argument("contentType",string())]}));}
        json!({"name":variant,"type":wire_ty(plan,&m.value),"contentType":m.wire.media_type().declared(),"constructorAccess":if request{"public"}else{"package"},"constructors":constructors,"fields":[getter("value",wire_ty(plan,&m.value)),getter("contentType",string())],"extends":owner,"final":true,"static":true,"explicitContentType":request})
    }).collect::<Vec<_>>(),"selection":"most-specific-source-media"}),
        ),
    }
}
fn controls(plan: &SdkPlan) -> Value {
    let options = name(plan, "HttpRuntime.Options");
    let builder = name(plan, "HttpRuntime.Options.Builder");
    let limits = name(plan, "ModelCodec.Limits");
    let json_limits = name(plan, "JsonRuntime.Limits");
    let mut setters = [
        ("serverUrl", named("java.net.URI")),
        ("timeout", named("java.time.Duration")),
        ("maxResponseBytes", primitive("int")),
        ("maxRequestBytes", primitive("int")),
        ("maxCaptureBytes", primitive("int")),
        ("maxUrlBytes", primitive("int")),
        ("httpClient", named("java.net.http.HttpClient")),
        ("codecLimits", named(&limits)),
        ("documentUrl", named("java.net.URI")),
        ("serverIndex", primitive("int")),
        ("serverName", string()),
        ("securityAlternative", primitive("int")),
        ("accept", string()),
        ("maxHeaderBytes", primitive("int")),
        ("maxStreamBufferBytes", primitive("int")),
    ]
    .into_iter()
    .map(|(n, t)| method(n, vec![argument("value", t)], named(&builder), false))
    .collect::<Vec<_>>();
    setters.push(method(
        "serverVariable",
        vec![argument("name", string()), argument("value", string())],
        named(&builder),
        false,
    ));
    let credential = method(
        "credential",
        vec![
            argument("sourceScheme", string()),
            argument("token", string()),
        ],
        named(&builder),
        false,
    );
    let request = name(plan, "RequestOptions");
    let request_builder = name(plan, "RequestOptions.Builder");
    json!({
        "options":{"name":options,"kind":"class","access":"public","final":true,"constructorAccess":"private","mutable":false,"factory":method("builder",vec![],named(&builder),true),
            "builder":{"name":builder,"kind":"class","access":"public","static":true,"final":true,"constructorAccess":"private","mutable":true,"methods":setters,"build":method("build",vec![],named(&options),false)},
            "deadline":"whole-call-including-codecs-and-body","streamDeadline":"until-close-or-eof","injectedClientOwnership":"caller"},
        "requestOptions":{"name":request,"kind":"class","access":"public","final":true,"mutable":false,"factory":method("builder",vec![],named(&request_builder),true),"defaults":method("defaults",vec![],named(&request),true),
            "methods":["serverUrl","documentUrl","serverIndex","serverName","serverVariable","securityAlternative","timeout","accept"]},
        "credentialMethods":[credential,method("basic",vec![argument("scheme",string()),argument("user",string()),argument("password",string())],named(&builder),false),method("apiKey",vec![argument("scheme",string()),argument("value",string())],named(&builder),false),method("authorization",vec![argument("scheme",string()),argument("provider",named(name(plan,"HttpRuntime.CredentialProvider")))],named(&builder),false)],
        "authorization":{"name":name(plan,"HttpRuntime.Authorization"),"factory":method("of",vec![argument("scheme",string()),argument("value",string())],named(name(plan,"HttpRuntime.Authorization")),true)},
        "provider":{"name":name(plan,"HttpRuntime.CredentialProvider"),"method":method("provide",vec![argument("context",named(name(plan,"HttpRuntime.CredentialContext")))],named(name(plan,"HttpRuntime.Authorization")),false)},
        "context":{"name":name(plan,"HttpRuntime.CredentialContext"),"kind":"record","fields":[getter("scheme",string()),getter("source",string()),getter("metadata",named(name(plan,"JsonRuntime.JsonValue"))),getter("permissions",generic("java.util.List",vec![string()])),getter("scopes",primitive("boolean"))]},
        "codecLimits":{"name":limits,"kind":"record","constructor":{"name":limits,"access":"public","parameters":[argument("json",named(&json_limits)),argument("maxConversionDepth",primitive("int")),argument("maxConversionSteps",primitive("long")),argument("maxEvaluationSteps",primitive("long")),argument("maxEqualitySteps",primitive("long"))]},
            "methods":[method("defaults",vec![],named(&limits),true),method("withJson",vec![argument("value",named(&json_limits))],named(&limits),false),method("withConversionDepth",vec![argument("value",primitive("int"))],named(&limits),false),method("withConversionSteps",vec![argument("value",primitive("long"))],named(&limits),false),method("withEvaluationSteps",vec![argument("value",primitive("long"))],named(&limits),false),method("withEqualitySteps",vec![argument("value",primitive("long"))],named(&limits),false)]},
        "jsonLimits":{"name":json_limits,"kind":"record","constructor":{"name":json_limits,"access":"public","parameters":[argument("maxInputBytes",primitive("int")),argument("maxOutputBytes",primitive("int")),argument("maxDepth",primitive("int")),argument("maxNumberBytes",primitive("int")),argument("maxWork",primitive("long"))]},
            "methods":[method("defaults",vec![],named(&json_limits),true),method("withInputBytes",vec![argument("value",primitive("int"))],named(&json_limits),false),method("withOutputBytes",vec![argument("value",primitive("int"))],named(&json_limits),false),method("withDepth",vec![argument("value",primitive("int"))],named(&json_limits),false),method("withWork",vec![argument("value",primitive("long"))],named(&json_limits),false)]},
        "failure":{"name":name(plan,"SdkException"),"base":"java.lang.RuntimeException","methods":[method("kind",vec![],string(),false),method("source",vec![],string(),false),method("status",vec![],primitive("int"),false),method("capture",vec![],primitive("byte[]"),false),method("truncated",vec![],primitive("boolean"),false),method("schemaSource",vec![],string(),false),method("instancePath",vec![],string(),false)]},
        "stream":{"type":name(plan,"EventStream"),"implements":["java.lang.Iterable","java.lang.AutoCloseable","java.util.concurrent.Flow.Publisher"],"flowItem":name(plan,"EventStream.Item"),"asyncPull":"nextAsync","asyncPresence":name(plan,"Presence"),"ownership":"response-or-stream-close","deadline":"whole-call-until-close-or-eof"}
    })
}
fn operation(plan: &SdkPlan, op: &crate::java_sdk::http::JavaOperation) -> NativeOperation {
    let client = name(plan, &plan.package().api_name);
    let input = nested(plan, &op.input_type);
    let result = if op.success_type == "Never" {
        name(plan, "Never")
    } else {
        nested(plan, &op.success_type)
    };
    let no_input = op.parameters.is_empty() && op.body.is_none();
    let mut symbols = BTreeMap::from([
        ("client".into(), client.clone()),
        ("method".into(), op.method_name.clone()),
        ("async-method".into(), op.async_method_name.clone()),
        ("input".into(), input.clone()),
        ("input-constructor".into(), format!("{input}.builder")),
        ("success".into(), result.clone()),
        ("error".into(), name(plan, "SdkException")),
    ]);
    if no_input {
        symbols.insert("no-input-method".into(), op.method_name.clone());
        symbols.insert("no-input-async-method".into(), op.async_method_name.clone());
    }
    let input_builder = format!("{input}.Builder");
    let slot = |member: &str, wire: &str, t: Value, required: bool| {
        json!({"member":member,"wire":wire,"model":t,"required":required,"type":presence(plan,t.clone(),required),"mutable":false,
        "getter":method(member,vec![],presence(plan,t.clone(),required),false),"storage":{"access":"private","final":true},
        "builderSetter":method(member,vec![argument("value",t)],named(&input_builder),false),
        "omitter":(!required).then(||method(&format!("omit{}",crate::rust_models::pascal(member)),vec![],named(&input_builder),false))})
    };
    let parameters = op
        .parameters
        .iter()
        .map(|p| {
            let mut value = slot(
                &p.native_name,
                p.wire.name(),
                ty(plan, &JavaType::Named(p.schema.clone())),
                p.required,
            );
            value["codec"] = json!(codec_name(plan, &p.schema));
            value
        })
        .collect::<Vec<_>>();
    let body = op
        .body
        .as_ref()
        .map(|b| slot(&b.native_name, "body", wire_ty(plan, &b.value), b.required));
    let properties = parameters
        .iter()
        .cloned()
        .chain(body.iter().cloned())
        .collect::<Vec<_>>();
    let success_count = op.responses.iter().filter(|r| r.can_succeed()).count();
    let singleton = op
        .wire
        .security()
        .alternatives()
        .first()
        .filter(|_| op.wire.security().alternatives().len() == 1)
        .and_then(|a| {
            if a.requirements().len() == 1 {
                a.requirements().first()
            } else {
                None
            }
        });
    let mut captured = NativeOperation {
        source: Location::at(plan.contract(), &op.source),
        operation_id: op.operation_id.clone(),
        symbols,
        descriptor: json!({
            "package":{"groupId":plan.maven_group_id(),"artifactId":plan.maven().artifact_id,"version":plan.package().version,"namespace":plan.package().package,"javaRelease":plan.maven().java_release},
            "constructor":{"input":{"name":format!("{input}.builder"),"owner":input,"member":"builder","style":"builder-factory","access":"public","static":true,"parameters":op.constructor.arguments.iter().map(|a|argument(&a.name,wire_ty(plan,&a.ty))).collect::<Vec<_>>(),"returns":named(&input_builder),"snapshot":"deep-immutable","canInitializeWithoutArguments":op.constructor.arguments.is_empty()},
                "inputDeclaration":{"name":input,"kind":"class","access":"public","static":true,"final":true,"constructorAccess":"private","mutable":false,"fields":properties,
                    "builder":{"name":input_builder,"access":"public","static":true,"final":true,"constructorAccess":"private","mutable":true,"build":method("build",vec![],named(&input),false),"result":"independent-deep-immutable-snapshot"}},
                "client":{"name":client,"type":client,"options":name(plan,"HttpRuntime.Options"),"implements":["java.lang.AutoCloseable"],"constructor":{"name":client,"access":"public","parameters":[argument("options",named(name(plan,"HttpRuntime.Options")))]},"close":method("close",vec![],primitive("void"),false),"controls":controls(plan)}},
            "parameters":{"members":parameters,
                "method":method(&op.method_name,vec![argument("input",named(&input))],named(&result),false),"asyncMethod":method(&op.async_method_name,vec![argument("input",named(&input))],generic("java.util.concurrent.CompletableFuture",vec![named(&result)]),false),
                "withOptions":method(&op.method_name,vec![argument("input",named(&input)),argument("options",named(name(plan,"RequestOptions")))],named(&result),false),
                "asyncWithOptions":method(&op.async_method_name,vec![argument("input",named(&input)),argument("options",named(name(plan,"RequestOptions")))],generic("java.util.concurrent.CompletableFuture",vec![named(&result)]),false),
                "noInputMethod":no_input.then(||method(&op.method_name,vec![],named(&result),false)),"noInputAsyncMethod":no_input.then(||method(&op.async_method_name,vec![],generic("java.util.concurrent.CompletableFuture",vec![named(&result)]),false)),
                "requestOptionsOverload":name(plan,"RequestOptions"),"canCallWithoutInput":no_input},
            "body":body,
            "responseUnions":{"success":{"name":result,"kind":if success_count==1{"concrete-result"}else if success_count==0{"uninhabited"}else{"sealed-interface"},"directResult":success_count==1,"closed":success_count>1,"members":op.responses.iter().filter(|r|r.can_succeed()).map(|r|nested(plan,&r.variant_name)).collect::<Vec<_>>()},"apiError":{"base":name(plan,"SdkException")}},
            "responses":op.responses.iter().flat_map(|r|{
                let mut values=Vec::new();for (name_,error) in [r.can_succeed().then_some((&r.variant_name,false)),r.error_variant_name.as_ref().map(|v|(v,true))].into_iter().flatten(){values.push(json!({"type":nested(plan,name_),"status":match r.wire.status(){crate::http_protocol::ResponseStatus::Exact(n)=>json!(n),_=>json!(r.wire.status_key())},"statusMatcher":r.wire.status(),"actualStatus":true,"error":error,"success":!error,"data":wire_ty(plan,&r.value),"fields":[getter("data",wire_ty(plan,&r.value)),getter("headers",generic("java.util.Map",vec![string(),generic("java.util.List",vec![string()])]))],"constructorAccess":"private","base":if error{name(plan,"SdkException")}else{"java.lang.Object".into()},"implements":if !error&&success_count>1{vec![result.clone()]}else{vec![]},"statusInherited":error,"statusGetter":method("status",vec![],primitive("int"),false),"typedHeaders":r.headers.as_ref().map(|h|name(plan,&h.name)),"closeable":true,"links":"metadata-only"}));}values
            }).collect::<Vec<_>>(),
            "credential":{"sourceSchemeKey":singleton.map(|r|r.name()),"owner":name(plan,"HttpRuntime.Options.Builder"),"method":method("credential",vec![argument("sourceScheme",string()),argument("token",string())],named(name(plan,"HttpRuntime.Options.Builder")),false),
                "alternatives":op.wire.security().alternatives().iter().map(|a|a.requirements().iter().map(|r|json!({"name":r.name(),"attachment":match r.credential(){crate::http_protocol::CredentialHook::Bearer{..}=>"bearer",crate::http_protocol::CredentialHook::Basic=>"basic",crate::http_protocol::CredentialHook::ApiKey{..}=>"api-key",crate::http_protocol::CredentialHook::OAuth2{..}=>"oauth2-hook",crate::http_protocol::CredentialHook::OpenIdConnect{..}=>"oidc-hook"},"permissions":match r.permissions(){crate::http_protocol::Permissions::Scopes(v)|crate::http_protocol::Permissions::Roles(v)=>v.iter().map(|v|v.value()).collect::<Vec<_>>()}})).collect::<Vec<_>>()).collect::<Vec<_>>(),"controls":controls(plan)},
            "protocol":op.wire,
        }),
    };
    if plan.credential_env().is_some() {
        captured
            .symbols
            .insert("env-factory".into(), format!("{client}.fromEnv"));
        captured.descriptor["constructor"]["client"]["environmentFactories"] = json!([
            method("fromEnv", vec![], named(&client), true),
            method(
                "fromEnv",
                vec![argument("transport", named("java.net.http.HttpClient"))],
                named(&client),
                true
            ),
            method(
                "fromEnv",
                vec![
                    argument("transport", named("java.net.http.HttpClient")),
                    argument(
                        "environment",
                        nullable(generic(
                            "java.util.function.Function",
                            vec![string(), string()]
                        ))
                    )
                ],
                named(&client),
                true
            )
        ]);
    }
    captured
}

pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    let (package, maven) = backend::java_config(&snapshot.target).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| PlanFinding {
                code: e.code.into(),
                source: e.source.as_ref().map(|s| Location::at(&contract, s)),
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let errors = |errors: Vec<java_sdk::HttpDiagnostic>| {
        errors
            .into_iter()
            .map(|e| PlanFinding {
                code: e.code.into(),
                source: Some(Location::at(&contract, &e.source)),
                message: e.message,
            })
            .collect::<Vec<_>>()
    };
    let plan = java_sdk::plan_sdk_with_protocol_v3(
        contract.clone(),
        selected,
        package,
        &[],
        maven,
        backend::java_options(&snapshot.generation, None),
    )
    .map_err(errors)?;
    plan.render().map_err(errors)?;
    snapshot.credential_env = plan
        .credential_env()
        .map(|policy| policy.semantic_descriptor());
    snapshot
        .operations
        .extend(plan.operations().iter().map(|op| operation(&plan, op)));
    for symbol in plan.models().symbols() {
        let source = Location::at(&contract, symbol.source());
        snapshot.models.push(NativeModel {
            source: source.clone(),
            name: name(&plan, symbol.name()),
            role: "model".into(),
            descriptor: Some(model(&plan, symbol)),
        });
        snapshot.models.push(NativeModel {
            source,
            name: codec_name(&plan, symbol.source()),
            role: "codec".into(),
            descriptor: Some(codec(&plan, symbol.source())),
        });
        if let JavaDeclaration::Union { variants, .. } = symbol.declaration() {
            let parent = name(&plan, symbol.name());
            for v in variants {
                snapshot.models.push(NativeModel {
                    source: Location::at(&contract, &v.source),
                    name: format!("{parent}.{}", v.name),
                    role: "union-arm".into(),
                    descriptor: Some(union_arm(&plan, &parent, v)),
                });
            }
        }
    }
    let mut extra = BTreeMap::new();
    for op in plan.operations() {
        if let Some(b) = &op.body {
            for m in &b.media {
                capture_value(&plan, &m.value, &mut extra);
            }
            if let Some(n) = &b.choice_type {
                let v = choice_model(&plan, &b.source, n, &b.media, true);
                extra.insert(v.name.clone(), v);
            }
        }
        for r in &op.responses {
            for m in &r.media {
                capture_value(&plan, &m.value, &mut extra);
            }
            if let Some(n) = &r.choice_type {
                let v = choice_model(&plan, &r.source, n, &r.media, false);
                extra.insert(v.name.clone(), v);
            }
            if let Some(h) = &r.headers {
                let n = name(&plan, &h.name);
                extra.insert(
                    n.clone(),
                    NativeModel {
                        source: Location::at(&contract, &h.source),
                        name: n,
                        role: "wire-model".into(),
                        descriptor: Some(header_model(&plan, h)),
                    },
                );
            }
        }
    }
    snapshot.models.extend(extra.into_values());
    Ok(())
}
