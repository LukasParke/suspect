//! PHP interface capture from the admitted model/operation plan.
//!
//! Source locations are correspondence evidence. Native identity consists of
//! allocated PHP symbols and typed declarations, never validation node indices
//! or declarations recovered from emitted PHP.

use std::{collections::BTreeMap, sync::Arc};

use serde::Serialize;
use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};
#[cfg(not(feature = "http-protocol"))]
use crate::php_sdk::PlannedOperation;
use crate::{
    backend,
    php_sdk::{
        self, Plan,
        models::{Extras, Field, Initializer, ModelPlan, Node, Shape},
    },
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Type {
    Primitive {
        name: &'static str,
    },
    Named {
        name: String,
    },
    List {
        items: Box<Type>,
    },
    Map {
        keys: &'static str,
        values: Box<Type>,
    },
    Union {
        variants: Vec<Type>,
    },
    Generic {
        name: String,
        arguments: Vec<Type>,
    },
}

impl Type {
    fn union(types: impl IntoIterator<Item = Self>) -> Self {
        let mut variants = Vec::new();
        for ty in types {
            match ty {
                Self::Union { variants: inner } => variants.extend(inner),
                other => variants.push(other),
            }
        }
        variants.sort();
        variants.dedup();
        if variants.len() == 1 {
            variants.pop().expect("one type")
        } else {
            Self::Union { variants }
        }
    }

    fn allows_php_null(&self) -> bool {
        match self {
            Self::Primitive { name: "null" } => true,
            Self::Union { variants } => variants.iter().any(Self::allows_php_null),
            _ => false,
        }
    }
}

fn qualified(namespace: &str, name: &str) -> String {
    format!("{namespace}\\{name}")
}
fn primitive(name: &'static str) -> Type {
    Type::Primitive { name }
}
fn named(namespace: &str, name: &str) -> Type {
    Type::Named {
        name: qualified(namespace, name),
    }
}
fn absent(namespace: &str) -> Value {
    json!({"kind":"absent","owner":qualified(namespace,"Absent"),"member":"Value"})
}
fn null() -> Value {
    json!({"kind":"literal","value":null})
}
fn construct(namespace: &str, name: &str) -> Value {
    json!({"kind":"constructor-call","type":qualified(namespace,name),"member":"__construct","arguments":[]})
}

struct Types<'a> {
    models: &'a ModelPlan,
    namespace: &'a str,
    memo: BTreeMap<SchemaId, Type>,
}

impl Types<'_> {
    fn at(&mut self, id: &SchemaId) -> Type {
        if let Some(value) = self.memo.get(id) {
            return value.clone();
        }
        let node = &self.models.nodes[id];
        let ty = match &node.shape {
            Shape::Object { .. } | Shape::Enum { .. } => named(self.namespace, &node.name),
            Shape::Json => named(self.namespace, "JsonValue"),
            Shape::Null => primitive("null"),
            Shape::Boolean => primitive("bool"),
            Shape::Number => named(self.namespace, "JsonNumber"),
            Shape::String => primitive("string"),
            Shape::Ref(target) => self.at(target),
            Shape::Array { item } => Type::List {
                items: Box::new(
                    item.as_ref()
                        .map_or_else(|| named(self.namespace, "JsonValue"), |item| self.at(item)),
                ),
            },
            Shape::Union(branches) => Type::union(branches.iter().map(|branch| self.at(branch))),
            Shape::Types(kinds) => Type::union(kinds.iter().map(|kind| match kind.as_str() {
                "null" => primitive("null"),
                "boolean" => primitive("bool"),
                "string" => primitive("string"),
                "integer" | "number" => named(self.namespace, "JsonNumber"),
                "object" => named(self.namespace, "JsonValue"),
                "array" => Type::List {
                    items: Box::new(named(self.namespace, "JsonValue")),
                },
                _ => unreachable!("admitted PHP type keyword"),
            })),
        };
        let ty = if node.nullable {
            Type::union([ty, primitive("null")])
        } else {
            ty
        };
        self.memo.insert(id.clone(), ty.clone());
        ty
    }

    fn signature(&mut self, id: &SchemaId, required: bool) -> Value {
        let ty = self.at(id);
        let php_null = ty.allows_php_null();
        let source_type = |doc| self.models.type_name(id, doc);
        let spelling = |doc| {
            if required {
                source_type(doc)
            } else {
                php_sdk::models::union(vec![source_type(doc), "Absent".into()])
            }
        };
        json!({"kind":"php-type","native":spelling(false),"phpdoc":spelling(true),
            "phpNullAllowed":php_null,
            "shape":if required {ty} else {Type::union([ty,named(self.namespace,"Absent")])}})
    }

    fn extra(&mut self, extras: &Extras) -> Option<Value> {
        let (value, phpdoc) = match extras {
            Extras::Closed => return None,
            Extras::Json | Extras::Patterned(_) => {
                (named(self.namespace, "JsonValue"), "JsonValue".to_owned())
            }
            Extras::Typed(id) => (self.at(id), self.models.type_name(id, true)),
        };
        Some(
            json!({"kind":"php-type","native":"array","phpdoc":format!("array<array-key, {phpdoc}>"),
            "phpNullAllowed":false,"shape":Type::Map {keys:"array-key",values:Box::new(value)}}),
        )
    }
}

fn support_type(namespace: &str, name: &str, nullable: bool) -> Value {
    let ty = named(namespace, name);
    json!({"kind":"php-type","native":if nullable {format!("{name}|null")} else {name.into()},
        "phpdoc":if nullable {format!("{name}|null")} else {name.into()},"phpNullAllowed":nullable,
        "shape":if nullable {Type::union([ty,primitive("null")])} else {ty}})
}
fn scalar_type(name: &'static str) -> Value {
    json!({"kind":"php-type","native":name,"phpdoc":name,"phpNullAllowed":name=="null","shape":primitive(name)})
}
fn parameter(name: &str, ty: Value, default: Option<Value>) -> Value {
    json!({"name":name,"type":ty,"required":default.is_none(),"hasDefault":default.is_some(),
        "initialization":default.unwrap_or_else(||json!({"kind":"argument"}))})
}
fn constructor(namespace: &str, owner: &str, member: &str, parameters: Vec<Value>) -> Value {
    json!({"owner":qualified(namespace,owner),"member":member,"access":"public","argumentStyle":"positional-or-named",
        "canInitializeWithoutArguments":parameters.iter().all(|p|p["hasDefault"] == true),"parameters":parameters})
}
fn field(types: &mut Types<'_>, field: &Field) -> Value {
    let initialization = match &field.initializer {
        Some(Initializer::EnumCase {
            type_name,
            case_name,
            wire_value,
        }) => json!({
            "kind":"literal","type":qualified(types.namespace,type_name),"member":case_name,"value":wire_value,
        }),
        None if field.required => json!({"kind":"constructor-argument","name":field.name}),
        None => absent(types.namespace),
    };
    json!({"kind":"property","name":field.name,"wire":field.wire,"type":types.signature(&field.source,field.required),
        "access":"public","required":field.required,"constructorParameter":field.initializer.is_none(),
        "readonly":field.initializer.is_some(),"mutable":field.initializer.is_none(),"initialization":initialization})
}

fn model(types: &mut Types<'_>, node: &Node) -> Option<Value> {
    let namespace = types.namespace;
    let methods = json!([
        {"owner":qualified(namespace,&node.name),"name":"fromJson","static":true,"access":"public",
            "parameters":[parameter("json",scalar_type("string"),None)],"returns":types.signature(&node.source,true),
            "target":format!("{}::{}",qualified(namespace,"Codecs"),node.codecs.decode)},
        {"owner":qualified(namespace,&node.name),"name":"toJson","static":false,"access":"public","parameters":[],
            "returns":scalar_type("string"),"target":format!("{}::{}",qualified(namespace,"Codecs"),node.codecs.encode)}
    ]);
    Some(match &node.shape {
        Shape::Object { fields, extras } => {
            let mut properties = fields.iter().map(|f| field(types, f)).collect::<Vec<_>>();
            let ctor = node
                .constructor
                .as_ref()
                .expect("retained object constructor");
            let mut parameters = ctor
                .parameters
                .iter()
                .map(|name| {
                    let field = fields
                        .iter()
                        .find(|f| &f.name == name)
                        .expect("allocated constructor field");
                    parameter(
                        name,
                        types.signature(&field.source, field.required),
                        (!field.required).then(|| absent(namespace)),
                    )
                })
                .collect::<Vec<_>>();
            if let Some(ty) = types.extra(extras) {
                let name = ctor
                    .extra_parameter
                    .as_deref()
                    .expect("retained extra parameter");
                let initial = json!({"kind":"literal","value":[]});
                parameters.push(parameter(name, ty.clone(), Some(initial.clone())));
                properties.push(json!({"kind":"property","name":name,"type":ty,"access":"public","readonly":false,
                    "mutable":true,"constructorParameter":true,"initialization":initial,"extraMembers":true}));
            }
            let key_policy = match extras {
                Extras::Patterned(patterns) => {
                    json!({"kind":"pattern-dispatched-json","overlaps":"all-matching-patterns","additionalMembers":"source-validated","patterns":patterns.iter().map(|(pattern,id)|json!({"pattern":pattern,"valueType":types.signature(id,true)})).collect::<Vec<_>>()})
                }
                Extras::Closed => json!({"kind":"closed"}),
                Extras::Json => json!({"kind":"json","validation":"whole-source-schema"}),
                Extras::Typed(_) => json!({"kind":"typed"}),
            };
            json!({"kind":"class","final":true,"implements":[qualified(namespace,"Model")],"fields":properties,"extraKeyPolicy":key_policy,
                "constructor":constructor(namespace,&node.name,&ctor.method,parameters),"methods":methods,
                "constants":[{"name":"SCHEMA","type":scalar_type("int"),"valuePolicy":"opaque-validation-root"}],
                "extraMemberCollisionPolicy":"declared-wire-name-rejected","encodingValidation":"current-mutable-native-value"})
        }
        Shape::Enum { cases } => {
            json!({"kind":"enum","backingType":scalar_type("string"),"implements":[qualified(namespace,"Model")],
            "cases":cases.iter().map(|(name,value)|json!({"kind":"literal","name":name,"value":value})).collect::<Vec<_>>(),
            "methods":methods,"construction":"native-enum-cases"})
        }
        // Primitive, list, reference and union naming hints are codec suffixes;
        // PHP does not export fabricated alias classes for them.
        _ => return None,
    })
}

fn codec_records(plan: &Plan, types: &mut Types<'_>, node: &Node) -> Vec<NativeModel> {
    let namespace = types.namespace;
    let value = types.signature(&node.source, true);
    let json_value = support_type(namespace, "JsonValue", false);
    let context = parameter(
        "context",
        support_type(namespace, "CodecContext", true),
        Some(null()),
    );
    [
        ("codec-decode", &node.codecs.decode, "json", scalar_type("string"), value.clone()),
        ("codec-encode", &node.codecs.encode, "value", value.clone(), scalar_type("string")),
        ("codec-from-value", &node.codecs.from_value, "value", json_value.clone(), value.clone()),
        ("codec-to-value", &node.codecs.to_value, "value", value, json_value),
    ].into_iter().map(|(role,method,arg,input,returns)| NativeModel {
        source:Location::at(plan.contract(),&node.source),name:format!("{}::{method}",qualified(namespace,"Codecs")),role:role.into(),
        descriptor:Some(json!({"kind":"static-method","access":"public","owner":qualified(namespace,"Codecs"),"member":method,
            "parameters":[parameter(arg,input,None),context],"returns":returns,
            "validation":"source-schema-and-selected-native-carrier","sharedContextOptional":true,"validationVersion":plan.program().version,"validationProfile":plan.program().profile})),
    }).collect()
}

#[cfg(not(feature = "http-protocol"))]
fn operation(plan: &Plan, types: &mut Types<'_>, op: &PlannedOperation) -> NativeOperation {
    let namespace = types.namespace;
    let mut inputs = op
        .parameters
        .iter()
        .map(|p| (p.name.as_str(), &p.schema, p.required))
        .chain(
            op.body
                .iter()
                .map(|b| (b.name.as_str(), &b.schema, b.required)),
        )
        .collect::<Vec<_>>();
    // This stable required-first ordering is the actual input-constructor rule.
    inputs.sort_by_key(|(_, _, required)| !*required);
    let input_parameters = inputs
        .iter()
        .map(|(name, id, required)| {
            parameter(
                name,
                types.signature(id, *required),
                (!required).then(|| absent(namespace)),
            )
        })
        .collect::<Vec<_>>();
    let no_input = input_parameters.iter().all(|p| p["hasDefault"] == true);
    let result_names = op
        .success_types
        .iter()
        .map(|(_, n)| n.clone())
        .collect::<Vec<_>>();
    let result_native = if result_names.is_empty() {
        "never".into()
    } else {
        php_sdk::models::union(result_names.clone())
    };
    let result = json!({"kind":"php-type","native":result_native,"phpdoc":result_native,"phpNullAllowed":false,
        "shape":if result_names.is_empty() {primitive("never")} else {Type::union(result_names.iter().map(|name|named(namespace,name)))}});
    let method = |owner: &str| {
        json!({"kind":"instance-method","access":"public","owner":qualified(namespace,owner),"name":op.method_name,
        "parameters":[parameter("input",support_type(namespace,&op.input_type,false),no_input.then(||construct(namespace,&op.input_type))),
            parameter("options",support_type(namespace,"RequestOptions",true),Some(null()))],
        "returns":result,"canCallWithoutInput":no_input,
        "throws":op.error_types.iter().map(|(_,name)|qualified(namespace,name)).chain(std::iter::once(qualified(namespace,"SdkError"))).collect::<Vec<_>>()})
    };
    let slot = |types: &mut Types<'_>,
                name: &str,
                wire: &str,
                schema: &SchemaId,
                required: bool| {
        json!({"member":name,"wire":wire,"model":types.signature(schema,true),"type":types.signature(schema,required),
            "required":required,"mutable":true,"readonly":false,"hasDefault":!required,
            "initialization":if required {json!({"kind":"argument"})} else {absent(namespace)},
            "codec":format!("{}::{}",qualified(namespace,"Codecs"),plan.models().nodes[schema].codecs.to_value)})
    };
    let parameters = op
        .parameters
        .iter()
        .map(|p| {
            let mut value = slot(types, &p.name, &p.wire_name, &p.schema, p.required);
            value["location"] = json!(format!("{:?}", p.location));
            value
        })
        .collect::<Vec<_>>();
    let body = op
        .body
        .as_ref()
        .map(|b| slot(types, &b.name, "body", &b.schema, b.required));
    let fields = inputs
        .iter()
        .map(|(name, id, required)| {
            json!({"kind":"property","name":name,"type":types.signature(id,*required),
        "access":"public","mutable":true,"readonly":false,"promoted":true})
        })
        .collect::<Vec<_>>();
    let responses = op.responses.iter().map(|r| {
        let ty = types.signature(&r.schema,true);
        json!({"type":qualified(namespace,&r.type_name),"status":r.status,"mediaType":r.media_type,"success":r.success,
            "kind":"class","final":true,"readonlyClass":r.success,"base":(!r.success).then(||qualified(namespace,&op.error_type)),
            "bodyMember":r.body_member,"metadataMember":r.metadata_member,
            "fields":[{"name":r.body_member,"type":ty,"access":"public","readonly":true,"inherited":false},
                {"name":r.metadata_member,"type":support_type(namespace,"HttpResponse",false),"access":"public","readonly":true,"inherited":!r.success}],
            "constructor":constructor(namespace,&r.type_name,"__construct",vec![parameter(&r.body_member,ty,None),parameter(&r.metadata_member,support_type(namespace,"HttpResponse",false),None)]),
            "constants":[{"kind":"literal","name":"STATUS","value":r.status}],
            "codec":format!("{}::{}",qualified(namespace,"Codecs"),plan.models().nodes[&r.schema].codecs.decode)})
    }).collect::<Vec<_>>();
    let symbols = BTreeMap::from([
        ("client".into(), qualified(namespace, "Client")),
        ("interface".into(), qualified(namespace, "ClientInterface")),
        ("method".into(), op.method_name.clone()),
        ("input".into(), qualified(namespace, &op.input_type)),
        (
            "input-constructor".into(),
            format!(
                "{}::{}",
                qualified(namespace, &op.input_type),
                op.input_constructor
            ),
        ),
        (
            "success".into(),
            if result_names.is_empty() {
                "never".into()
            } else {
                php_sdk::models::union(
                    result_names
                        .iter()
                        .map(|name| qualified(namespace, name))
                        .collect(),
                )
            },
        ),
        ("api-error".into(), qualified(namespace, &op.error_type)),
    ]);
    NativeOperation {
        source: Location::at(plan.contract(), &op.source),
        operation_id: op.operation_id.clone(),
        symbols,
        descriptor: json!({
            "package":{"composerName":plan.config().package_name,"version":plan.config().package_version,"namespace":namespace},
            "constructor":{"input":constructor(namespace,&op.input_type,&op.input_constructor,input_parameters),
                "inputDeclaration":{"kind":"class","final":true,"fields":fields},
                "client":{"kind":"class","final":true,"implements":[qualified(namespace,"ClientInterface")],
                    "signature":constructor(namespace,"Client","__construct",vec![
                        parameter("credentials",support_type(namespace,"Credentials",false),None),
                        parameter("transport",support_type(namespace,"Transport",false),Some(construct(namespace,"CurlTransport"))),
                        parameter("options",support_type(namespace,"ClientOptions",false),Some(construct(namespace,"ClientOptions")))])}},
            "parameters":{"members":parameters,"method":method("Client"),"interfaceMethod":method("ClientInterface")},
            "body":body,
            "responseUnions":{"success":{"kind":if result_names.len()==1 {"concrete-result"} else if result_names.is_empty() {"never"} else {"native-union"},
                    "directResult":result_names.len()==1,"type":result},
                "apiError":{"kind":"abstract-class","type":qualified(namespace,&op.error_type),"base":qualified(namespace,"ApiError"),"sealed":false,
                    "constructor":constructor(namespace,&op.error_type,"__construct",vec![parameter("response",support_type(namespace,"HttpResponse",false),None)])}},
            "responses":responses,
            "credential":{"kind":"bearer","class":qualified(namespace,"Credentials"),"sourceSchemeKey":op.wire.security_scheme_name,
                "keyType":"array-key","valueType":"string","requiredAtCall":true,"lookup":"explicit-map",
                "constructorMember":"__construct","parameter":"tokens"},
        }),
    }
}

#[cfg(feature = "http-protocol")]
fn payload_type(types: &mut Types<'_>, payload: &php_sdk::protocol::Payload) -> Value {
    use php_sdk::protocol::Payload;
    match payload {
        Payload::Schema(id) => types.signature(id, true),
        Payload::Text => scalar_type("string"),
        Payload::Json => support_type(types.namespace, "JsonValue", false),
        Payload::Bytes => support_type(types.namespace, "Bytes", false),
        Payload::NoBody => support_type(types.namespace, "NoBody", false),
        Payload::Object(name) => support_type(types.namespace, name, false),
        Payload::Stream(id) => {
            json!({"kind":"php-type","native":"ItemStream","phpdoc":format!("ItemStream<{}>",types.models.type_name(id,true)),"phpNullAllowed":false,
            "shape":Type::Generic{name:qualified(types.namespace,"ItemStream"),arguments:vec![types.at(id)]}})
        }
    }
}

#[cfg(feature = "http-protocol")]
fn request_payload_type(types: &mut Types<'_>, payload: &php_sdk::protocol::Payload) -> Value {
    if let php_sdk::protocol::Payload::Stream(id) = payload {
        json!({"kind":"php-type","native":"iterable","phpdoc":format!("iterable<{}>",types.models.type_name(id,true)),"phpNullAllowed":false,
            "shape":Type::Generic{name:"iterable".into(),arguments:vec![types.at(id)]}})
    } else {
        payload_type(types, payload)
    }
}

#[cfg(feature = "http-protocol")]
fn payload_codecs(plan: &Plan, payload: &php_sdk::protocol::Payload) -> Value {
    use php_sdk::protocol::Payload;
    let namespace = &plan.config().namespace;
    match payload {
        Payload::Schema(id) | Payload::Stream(id) => {
            let codecs = &plan.models().nodes[id].codecs;
            json!({"kind":if matches!(payload,Payload::Stream(_)){"per-item"}else{"schema"},
                "fromValue":format!("{}::{}",qualified(namespace,"Codecs"),codecs.from_value),
                "toValue":format!("{}::{}",qualified(namespace,"Codecs"),codecs.to_value),
                "decode":format!("{}::{}",qualified(namespace,"Codecs"),codecs.decode),
                "encode":format!("{}::{}",qualified(namespace,"Codecs"),codecs.encode)})
        }
        Payload::Object(name) => {
            json!({"kind":"parts","decode":format!("{}::fromParts",qualified(namespace,name)),"encode":format!("{}::toParts",qualified(namespace,name))})
        }
        Payload::Bytes => {
            json!({"kind":"bytes","constructor":qualified(namespace,"Bytes"),"member":"value"})
        }
        Payload::Json => {
            json!({"kind":"schema-free-json","decode":format!("{}::parse",qualified(namespace,"JsonValue")),"encode":"toJson"})
        }
        Payload::Text => json!({"kind":"text","native":"string"}),
        Payload::NoBody => {
            json!({"kind":"enum-case","owner":qualified(namespace,"NoBody"),"member":"Value"})
        }
    }
}

#[cfg(feature = "http-protocol")]
fn optional_signature(mut ty: Value, required: bool, namespace: &str) -> Value {
    if !required {
        ty["native"] = json!(php_sdk::models::union(vec![
            ty["native"].as_str().unwrap().into(),
            "Absent".into()
        ]));
        ty["phpdoc"] = json!(php_sdk::models::union(vec![
            ty["phpdoc"].as_str().unwrap().into(),
            "Absent".into()
        ]));
        ty["shape"] = json!({"kind":"union","variants":[ty["shape"],named(namespace,"Absent")]});
    }
    ty
}

#[cfg(feature = "http-protocol")]
fn operation(
    plan: &Plan,
    types: &mut Types<'_>,
    op: &php_sdk::protocol::Operation,
) -> NativeOperation {
    let namespace = types.namespace;
    let mut input_args = op
        .parameters
        .iter()
        .map(|p| {
            parameter(
                &p.name,
                types.signature(&p.schema, p.wire.required()),
                (!p.wire.required()).then(|| absent(namespace)),
            )
        })
        .collect::<Vec<_>>();
    let body = if op.body.is_empty() {
        None
    } else {
        let variants = op
            .body
            .iter()
            .map(|m| {
                m.wrapper
                    .as_ref()
                    .map(|n| support_type(namespace, n, false))
                    .unwrap_or_else(|| request_payload_type(types, &m.payload))
            })
            .collect::<Vec<_>>();
        let ty = if variants.len() == 1 {
            variants[0].clone()
        } else {
            json!({"kind":"php-type","native":php_sdk::models::union(variants.iter().map(|v|v["native"].as_str().unwrap().into()).collect()),"phpdoc":php_sdk::models::union(variants.iter().map(|v|v["phpdoc"].as_str().unwrap().into()).collect()),"phpNullAllowed":false,"shape":{"kind":"union","variants":variants.iter().map(|v|v["shape"].clone()).collect::<Vec<_>>()}})
        };
        input_args.push(parameter(
            "body",
            optional_signature(ty.clone(), op.body_required, namespace),
            (!op.body_required).then(|| absent(namespace)),
        ));
        Some(
            json!({"member":"body","model":ty,"type":optional_signature(ty,op.body_required,namespace),"required":op.body_required,
            "media":op.body.iter().map(|m|json!({"contentType":m.wire.media_type().declared(),"wrapper":m.wrapper.as_ref().map(|n|qualified(namespace,n)),"valueType":request_payload_type(types,&m.payload),"codecs":payload_codecs(plan,&m.payload)})).collect::<Vec<_>>()}),
        )
    };
    input_args.sort_by_key(|p| p["required"] != true);
    let input_fields = input_args.iter().map(|p|json!({"kind":"property","name":p["name"],"type":p["type"],"access":"public","mutable":true,"readonly":false,"promoted":true})).collect::<Vec<_>>();
    let no_input = input_args.iter().all(|p| p["hasDefault"] == true);
    let successes = op
        .responses
        .iter()
        .filter(|r| r.success)
        .map(|r| r.name.clone())
        .collect::<Vec<_>>();
    let result = if successes.is_empty() {
        scalar_type("never")
    } else {
        json!({"kind":"php-type","native":php_sdk::models::union(successes.clone()),"phpdoc":php_sdk::models::union(successes.clone()),"phpNullAllowed":false,"shape":Type::union(successes.iter().map(|n|named(namespace,n)))})
    };
    let method = |owner: &str| {
        json!({"kind":"instance-method","access":"public","owner":qualified(namespace,owner),"name":op.method,
        "parameters":[parameter("input",support_type(namespace,&op.input,false),no_input.then(||construct(namespace,&op.input))),parameter("options",support_type(namespace,"RequestOptions",true),Some(null()))],
        "returns":result,"canCallWithoutInput":no_input,"throws":op.responses.iter().filter(|r|!r.success).map(|r|qualified(namespace,&r.name)).chain(std::iter::once(qualified(namespace,"SdkError"))).collect::<Vec<_>>()})
    };
    let parameters=op.parameters.iter().map(|p|json!({"member":p.name,"wire":p.wire.name(),"model":types.signature(&p.schema,true),"type":types.signature(&p.schema,p.wire.required()),"required":p.wire.required(),"location":format!("{:?}",p.wire.location()),"initialization":if p.wire.required(){json!({"kind":"argument"})}else{absent(namespace)},"codec":format!("{}::{}",qualified(namespace,"Codecs"),plan.models().nodes[&p.schema].codecs.to_value)})).collect::<Vec<_>>();
    let responses=op.responses.iter().map(|r|{
        let ty=payload_type(types,&r.payload);let response_type=if matches!(r.payload,php_sdk::protocol::Payload::Stream(_)){"StreamResponse"}else{"HttpResponse"};
        let mut metadata=support_type(namespace,response_type,false);
        if !r.success {
            metadata["native"]=json!("HttpResponse|StreamResponse");
            metadata["shape"]=json!(Type::union([named(namespace,"HttpResponse"),named(namespace,"StreamResponse")]));
            metadata["phpdocShape"]=json!(named(namespace,response_type));
        }
        let links=json!({"kind":"php-type","native":"array","phpdoc":"list<Link>","phpNullAllowed":false,"shape":Type::List{items:Box::new(named(namespace,"Link"))}});
        let mut constants=vec![json!({"kind":"literal","name":"STATUS_PATTERN","value":r.status_key})];
        if let crate::http_protocol::ResponseStatus::Exact(status)=r.wire.status(){constants.push(json!({"kind":"literal","name":"STATUS","value":status}));}
        json!({"type":qualified(namespace,&r.name),"statusPattern":r.status_key,"status":r.wire.status(),"success":r.success,"mediaType":r.media.as_ref().map(|m|m.wire.media_type().declared()),
            "kind":"class","final":true,"readonlyClass":r.success,"base":(!r.success).then(||qualified(namespace,&op.error)),"bodyMember":"body","metadataMember":"response",
            "fields":[{"name":"body","type":ty,"readonly":true},{"name":"response","type":metadata,"readonly":true,"inherited":!r.success},
                {"name":"headers","type":support_type(namespace,&r.headers.name,false),"readonly":true},{"name":"links","type":links,"readonly":true}],
            "constructor":constructor(namespace,&r.name,"__construct",vec![parameter("body",ty,None),parameter("response",support_type(namespace,response_type,false),None),parameter("headers",support_type(namespace,&r.headers.name,false),None),parameter("links",links,Some(json!({"kind":"literal","value":[]})))]),
            "constants":constants,"codecs":payload_codecs(plan,&r.payload),
            "headers":r.headers.fields.iter().map(|h|json!({"name":h.name,"wire":h.wire.name(),"type":types.signature(&h.schema,h.wire.required()),"required":h.wire.required()})).collect::<Vec<_>>()})
    }).collect::<Vec<_>>();
    let mut error_metadata = op
        .responses
        .iter()
        .filter(|r| !r.success)
        .map(|r| {
            if matches!(r.payload, php_sdk::protocol::Payload::Stream(_)) {
                "StreamResponse".to_owned()
            } else {
                "HttpResponse".to_owned()
            }
        })
        .collect::<Vec<_>>();
    if error_metadata.is_empty() {
        error_metadata = vec!["HttpResponse".into(), "StreamResponse".into()];
    }
    let error_metadata_spelling = php_sdk::models::union(error_metadata.clone());
    let error_metadata_type = json!({"kind":"php-type","native":error_metadata_spelling,"phpdoc":error_metadata_spelling,"phpNullAllowed":false,"shape":Type::union(error_metadata.iter().map(|name|named(namespace,name)))});
    NativeOperation {
        source: Location::at(plan.contract(), &op.source),
        operation_id: op.id.clone(),
        symbols: BTreeMap::from([
            ("client".into(), qualified(namespace, "Client")),
            ("interface".into(), qualified(namespace, "ClientInterface")),
            ("method".into(), op.method.clone()),
            ("input".into(), qualified(namespace, &op.input)),
            (
                "input-constructor".into(),
                format!("{}::__construct", qualified(namespace, &op.input)),
            ),
            ("api-error".into(), qualified(namespace, &op.error)),
            (
                "success".into(),
                if successes.is_empty() {
                    "never".into()
                } else {
                    php_sdk::models::union(
                        successes.iter().map(|n| qualified(namespace, n)).collect(),
                    )
                },
            ),
        ]),
        descriptor: json!({"package":{"composerName":plan.config().package_name,"namespace":namespace,"version":plan.config().package_version},
            "constructor":{"input":constructor(namespace,&op.input,"__construct",input_args),"inputDeclaration":{"kind":"class","final":true,"fields":input_fields},"client":{"kind":"class","final":true,"implements":[qualified(namespace,"ClientInterface")],"signature":constructor(namespace,"Client","__construct",vec![parameter("credentials",support_type(namespace,"Credentials",false),None),parameter("transport",support_type(namespace,"Transport",false),Some(construct(namespace,"CurlTransport"))),parameter("options",support_type(namespace,"ClientOptions",false),Some(construct(namespace,"ClientOptions")))])}},
            "parameters":{"members":parameters,"method":method("Client"),"interfaceMethod":method("ClientInterface")},"body":body,
            "responseUnions":{"success":{"kind":if successes.len()==1{"concrete-result"}else if successes.is_empty(){"never"}else{"native-union"},"directResult":successes.len()==1,"type":result},"apiError":{"kind":"abstract-class","type":qualified(namespace,&op.error),"base":qualified(namespace,"ApiError"),"constructor":constructor(namespace,&op.error,"__construct",vec![parameter("response",error_metadata_type,None)])}},
            "responses":responses,"credential":{"configuration":"explicit-credential-map-or-caller-hook","class":qualified(namespace,"Credentials"),"keyType":"array-key","constructorMember":"__construct","parameter":"tokens","valueType":"string|BasicCredential|ApiKeyCredential|AuthorizationCredential|Closure(CredentialRequest):AuthorizationCredential","stringMeaning":"bearer-token","sourceSchemeKey":single_scheme(op.wire.security()),"security":security_identity(op.wire.security()),"metadataUrlBase":"selected-effective-server","requestMetadataMembers":["operationId","scheme","kind","permissions","metadata","serverUrl"]},
            "protocol":serde_json::to_value(&op.wire).expect("typed protocol metadata")}),
    }
}

#[cfg(feature = "http-protocol")]
fn security_identity(security: &crate::http_protocol::SecurityPlan) -> Value {
    use crate::http_protocol::{CredentialHook, Permissions, SecurityPlan};
    match security {
        SecurityPlan::Undeclared { .. } => json!({"kind":"undeclared"}),
        SecurityPlan::NoAuth { .. } => json!({"kind":"none"}),
        SecurityPlan::Alternatives { alternatives, .. } => {
            json!({"kind":"alternatives","alternatives":alternatives.iter().map(|a|a.requirements().iter().map(|r|json!({"key":r.name(),"kind":match r.credential(){CredentialHook::Bearer{..}=>"bearer",CredentialHook::Basic=>"basic",CredentialHook::ApiKey{..}=>"api-key",CredentialHook::OAuth2{..}=>"oauth2",CredentialHook::OpenIdConnect{..}=>"open-id-connect"},"metadata":credential_metadata(r.credential()),"permissionKind":match r.permissions(){Permissions::Scopes(_)=>"scopes",Permissions::Roles(_)=>"roles"},"permissions":match r.permissions(){Permissions::Scopes(p)|Permissions::Roles(p)=>p.iter().map(|v|v.value()).collect::<Vec<_>>()}})).collect::<Vec<_>>()).collect::<Vec<_>>()})
        }
    }
}

#[cfg(feature = "http-protocol")]
fn credential_metadata(hook: &crate::http_protocol::CredentialHook) -> Value {
    use crate::http_protocol::CredentialHook;
    match hook {
        CredentialHook::Bearer { bearer_format } => {
            json!({"native":"string","bearerFormat":bearer_format.as_ref().map(|v|v.value())})
        }
        CredentialHook::Basic => json!({"native":"BasicCredential"}),
        CredentialHook::ApiKey { location, name } => {
            json!({"native":"ApiKeyCredential","location":location,"wireName":name.value()})
        }
        CredentialHook::OAuth2 {
            flows,
            metadata_url,
        } => {
            json!({"native":"AuthorizationCredential","callerHook":"Closure(CredentialRequest):AuthorizationCredential","metadataUrl":metadata_url.as_ref().map(|v|v.value()),
            "flows":flows.iter().map(|f|json!({"kind":f.kind(),"authorizationUrl":f.authorization_url().map(|v|v.value()),"tokenUrl":f.token_url().map(|v|v.value()),"refreshUrl":f.refresh_url().map(|v|v.value()),"deviceAuthorizationUrl":f.device_authorization_url().map(|v|v.value()),"scopes":f.scopes().keys().collect::<Vec<_>>()})).collect::<Vec<_>>()})
        }
        CredentialHook::OpenIdConnect { discovery_url } => {
            json!({"native":"AuthorizationCredential","callerHook":"Closure(CredentialRequest):AuthorizationCredential","discoveryUrl":discovery_url.value()})
        }
    }
}

#[cfg(feature = "http-protocol")]
fn single_scheme(security: &crate::http_protocol::SecurityPlan) -> Option<&str> {
    let [alternative] = security.alternatives() else {
        return None;
    };
    let [requirement] = alternative.requirements() else {
        return None;
    };
    Some(requirement.name())
}

#[cfg(feature = "http-protocol")]
fn protocol_models(plan: &Plan, types: &mut Types<'_>) -> Vec<NativeModel> {
    use crate::php_sdk::protocol::{HeaderObject, Part};
    let namespace = types.namespace;
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let context = || {
        parameter(
            "context",
            support_type(namespace, "CodecContext", false),
            None,
        )
    };
    let parts_type = json!({"kind":"php-type","native":"array","phpdoc":"array<array-key,list<PartValue>>","phpNullAllowed":false,"shape":Type::Map{keys:"array-key",values:Box::new(Type::List{items:Box::new(named(namespace,"PartValue"))})}});
    let headers_type = json!({"kind":"php-type","native":"array","phpdoc":"array<array-key,string>","phpNullAllowed":false,"shape":Type::Map{keys:"array-key",values:Box::new(primitive("string"))}});
    let method = |owner: &str,
                  name: &str,
                  is_static: bool,
                  parameters: Vec<Value>,
                  returns: Value| json!({"owner":qualified(namespace,owner),"name":name,"static":is_static,"access":"public","parameters":parameters,"returns":returns});
    let part_ty = |types: &mut Types<'_>, part: &Part| {
        let ty = part
            .wrapper
            .as_ref()
            .map(|name| support_type(namespace, name, false))
            .unwrap_or_else(|| payload_type(types, &part.payload));
        if part.wire.multiplicity() == crate::http_protocol::PartMultiplicity::RepeatedArrayItems {
            json!({"kind":"php-type","native":"array","phpdoc":format!("list<{}>",ty["phpdoc"].as_str().unwrap()),"phpNullAllowed":false,"shape":{"kind":"list","items":ty["shape"]}})
        } else {
            ty
        }
    };
    let mut headers: Vec<(&HeaderObject, &SourceId)> = Vec::new();
    for object in &plan.surface.objects {
        let mut ordered = object.parts.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|p| !p.wire.required());
        let mut fields = Vec::new();
        let mut args = Vec::new();
        for part in ordered {
            let ty = optional_signature(part_ty(types, part), part.wire.required(), namespace);
            fields.push(json!({"name":part.name,"wire":part.wire.name(),"type":ty,"mutable":true,"required":part.wire.required(),"codecs":payload_codecs(plan,&part.payload)}));
            args.push(parameter(
                &part.name,
                ty,
                (!part.wire.required()).then(|| absent(namespace)),
            ));
        }
        if let Some(extra) = &object.extra {
            let ty = part_ty(types, extra);
            let map = json!({"native":"array","phpdoc":format!("array<array-key,{}>",ty["phpdoc"].as_str().unwrap()),"shape":{"kind":"map","keys":"array-key","values":ty["shape"]}});
            fields.push(json!({"name":"extra","type":map,"mutable":true}));
            args.push(parameter(
                "extra",
                map,
                Some(json!({"kind":"literal","value":[]})),
            ));
        }
        out.push(NativeModel{source:Location::at(plan.contract(),&object.source),name:qualified(namespace,&object.name),role:"protocol-body".into(),descriptor:Some(json!({"kind":"class","final":true,"fields":fields,"constructor":constructor(namespace,&object.name,"__construct",args),"methods":[method(&object.name,"toParts",false,vec![context()],parts_type.clone()),method(&object.name,"fromParts",true,vec![parameter("parts",parts_type.clone(),None),context()],support_type(namespace,&object.name,false))],"payloadKind":if object.multipart{"multipart"}else{"form"}}))});
        for part in object.parts.iter().chain(object.extra.iter()) {
            if let Some(wrapper) = &part.wrapper {
                headers.push((&part.headers, part.wire.source().use_site().source()));
                let mut args = vec![parameter("value", payload_type(types, &part.payload), None)];
                let required_headers = part.headers.fields.iter().any(|h| h.wire.required());
                if required_headers {
                    args.push(parameter(
                        "headers",
                        support_type(namespace, &part.headers.name, false),
                        None,
                    ));
                }
                let text = json!({"native":"string|null","phpdoc":"string|null","shape":Type::union([primitive("string"),primitive("null")])});
                args.push(parameter("filename", text.clone(), Some(null())));
                args.push(parameter("contentType", text, Some(null())));
                if !required_headers {
                    args.push(parameter(
                        "headers",
                        support_type(namespace, &part.headers.name, false),
                        Some(construct(namespace, &part.headers.name)),
                    ));
                }
                args.push(parameter(
                    "extraHeaders",
                    json!({"native":"array","phpdoc":"array<array-key,string>"}),
                    Some(json!({"kind":"literal","value":[]})),
                ));
                let fields=args.iter().map(|p|json!({"kind":"property","name":p["name"],"type":p["type"],"access":"public","mutable":true,"readonly":false,"promoted":true})).collect::<Vec<_>>();
                out.push(NativeModel{source:Location::at(plan.contract(),part.wire.source().use_site().source()),name:qualified(namespace,wrapper),role:"protocol-part".into(),descriptor:Some(json!({"kind":"class","final":true,"mutable":true,"constructor":constructor(namespace,wrapper,"__construct",args),"fields":fields,"methods":[method(wrapper,"toPart",false,vec![context()],support_type(namespace,"PartValue",false)),method(wrapper,"fromPart",true,vec![parameter("part",support_type(namespace,"PartValue",false),None),context()],support_type(namespace,wrapper,false))],"codecs":payload_codecs(plan,&part.payload)}))});
            }
        }
    }
    for operation in plan.operations() {
        for response in &operation.responses {
            headers.push((&response.headers, &response.source));
        }
        for media in &operation.body {
            if let Some(wrapper) = &media.wrapper {
                let mut parameters = vec![parameter(
                    "value",
                    request_payload_type(types, &media.payload),
                    None,
                )];
                parameters.push(parameter(
                    "contentType",
                    scalar_type("string"),
                    matches!(
                        media.wire.media_type().range(),
                        crate::http_protocol::MediaRange::Concrete { .. }
                    )
                    .then(|| json!({"kind":"literal","value":media.wire.media_type().declared()})),
                ));
                let fields=parameters.iter().map(|p|json!({"kind":"property","name":p["name"],"type":p["type"],"access":"public","mutable":false,"readonly":true,"promoted":true})).collect::<Vec<_>>();
                out.push(NativeModel{source:Location::at(plan.contract(),&media.source),name:qualified(namespace,wrapper),role:"request-media".into(),descriptor:Some(json!({"kind":"class","final":true,"readonly":true,"constructor":constructor(namespace,wrapper,"__construct",parameters),"fields":fields,"methods":[method(wrapper,"toPayload",false,vec![context(),parameter("maxBytes",scalar_type("int"),Some(json!({"kind":"class-constant","owner":qualified(namespace,"RuntimeConfig"),"member":"MAX_REQUEST_BYTES","value":plan.config().max_request_bytes})))],support_type(namespace,"PayloadValue",false))],"codecs":payload_codecs(plan,&media.payload)}))});
            }
        }
    }
    for (header, source) in headers {
        if !seen.insert(header.name.clone()) {
            continue;
        }
        let mut fields = header.fields.iter().collect::<Vec<_>>();
        fields.sort_by_key(|f| !f.wire.required());
        let params = fields
            .iter()
            .map(|f| {
                parameter(
                    &f.name,
                    types.signature(&f.schema, f.wire.required()),
                    (!f.wire.required()).then(|| absent(namespace)),
                )
            })
            .collect::<Vec<_>>();
        out.push(NativeModel{source:Location::at(plan.contract(),source),name:qualified(namespace,&header.name),role:"protocol-headers".into(),descriptor:Some(json!({"kind":"class","final":true,"mutable":true,"constructor":constructor(namespace,&header.name,"__construct",params),"fields":fields.iter().map(|h|json!({"name":h.name,"wire":h.wire.name(),"type":types.signature(&h.schema,h.wire.required()),"required":h.wire.required(),"codecs":payload_codecs(plan,&php_sdk::protocol::Payload::Schema(h.schema.clone()))})).collect::<Vec<_>>(),"methods":[method(&header.name,"fromResponse",true,vec![parameter("response",json!({"kind":"php-type","native":"HttpResponse|StreamResponse","phpdoc":"HttpResponse|StreamResponse","shape":Type::union([named(namespace,"HttpResponse"),named(namespace,"StreamResponse")]),"phpNullAllowed":false}),None),context()],support_type(namespace,&header.name,false)),method(&header.name,"toHeaders",false,vec![context()],headers_type.clone())]}))});
    }
    out
}

/// Capture the same admitted package and namespace policy as backend::generate.
pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    let mut config = backend::php_config(&snapshot.target);
    config.credential_env = snapshot.generation.credential_env.clone();
    let plan = php_sdk::protocol::plan_sdk(
        contract.clone(),
        selected,
        config,
        snapshot
            .generation
            .apply_to(php_sdk::protocol::capabilities()),
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: Some(Location::at(&contract, &error.source)),
                message: error.message,
            })
            .collect::<Vec<_>>()
    })?;
    snapshot.credential_env = plan
        .credential_env()
        .map(|policy| policy.semantic_descriptor());
    if plan.credential_env().is_some() {
        let namespace = &plan.config().namespace;
        snapshot.models.push(NativeModel{source:Location::at(&contract,&SourceId::new(contract.entry().clone(),Default::default())),name:format!("{}::fromEnv",qualified(namespace,"Client")),role:"credential-env-factory".into(),descriptor:Some(json!({"kind":"static-method","access":"public","owner":qualified(namespace,"Client"),"member":"fromEnv","parameters":[parameter("transport",support_type(namespace,"Transport",false),Some(construct(namespace,"CurlTransport"))),parameter("options",support_type(namespace,"ClientOptions",false),Some(construct(namespace,"ClientOptions")))],"returns":support_type(namespace,"Client",false),"environmentRead":"client-factory-time","explicitConstructorUsesEnvironment":false,"missingValues":"unavailable-credential"}))});
    }
    let mut types = Types {
        models: plan.models(),
        namespace: &plan.config().namespace,
        memo: BTreeMap::new(),
    };
    for op in plan.operations() {
        snapshot.operations.push(operation(&plan, &mut types, op));
    }
    for node in plan.models().nodes.values() {
        if let Some(descriptor) = model(&mut types, node) {
            snapshot.models.push(NativeModel {
                source: Location::at(&contract, &node.source),
                name: qualified(types.namespace, &node.name),
                role: "model".into(),
                descriptor: Some(descriptor),
            });
        }
        snapshot
            .models
            .extend(codec_records(&plan, &mut types, node));
    }
    #[cfg(feature = "http-protocol")]
    snapshot.models.extend(protocol_models(&plan, &mut types));
    Ok(())
}
