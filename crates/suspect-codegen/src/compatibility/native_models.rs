//! Lossless adapters for retained native type descriptors. No code-text parsing.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};

use crate::{go_models, python_models, rust_models, typescript};

use super::{Location, native::NativeModel};

pub(super) fn role(role: rust_models::RepresentationRole) -> &'static str {
    match role {
        rust_models::RepresentationRole::Model => "model",
        rust_models::RepresentationRole::NonNullValue => "non-null-value",
    }
}

pub(super) fn typescript(contract: &Contract, plan: &typescript::ModelPlan) -> Vec<NativeModel> {
    plan.symbols()
        .iter()
        .map(|symbol| NativeModel {
            source: Location::at(contract, symbol.source()),
            name: symbol.name().into(),
            role: match symbol.view() {
                typescript::ModelView::Neutral => "neutral",
                typescript::ModelView::Request => "request",
                typescript::ModelView::Response => "response",
            }
            .into(),
            descriptor: Some(json!({"kind":"alias","type":typescript_type(symbol.expression())})),
        })
        .collect()
}

fn typescript_type(expression: &typescript::Expr) -> Value {
    use typescript::{Expr, Literal, Primitive};
    match expression {
        // Source addresses identify correspondence, not native type identity.
        // References below retain the actual allocated native name instead.
        Expr::At(_, expression) => typescript_type(expression),
        Expr::Any => json!({"kind":"primitive","name":"JsonValue"}),
        Expr::Never => json!({"kind":"primitive","name":"never"}),
        Expr::Primitive(primitive) => json!({"kind":"primitive","name":match primitive {
            Primitive::Null => "null",
            Primitive::Boolean => "boolean",
            Primitive::String => "string",
            Primitive::SafeInteger => "number",
            Primitive::Integer => "bigint",
            Primitive::Number => "JsonNumber",
            Primitive::AnyNumber => "number | bigint | JsonNumber",
        }}),
        Expr::Literal(Literal::Boolean(value)) => {
            json!({"kind":"literal","value":value,"nativeType":"boolean"})
        }
        Expr::Literal(Literal::String(value)) => {
            json!({"kind":"literal","value":value,"nativeType":"string"})
        }
        Expr::Literal(Literal::Integer { value, safe }) => {
            json!({"kind":"literal","value":value,"nativeType":if *safe {"number"} else {"bigint"}})
        }
        Expr::Reference(name) => json!({"kind":"named","name":name}),
        Expr::Array(item) => wrapper("array", typescript_type(item)),
        Expr::Object(fields, extra) => json!({
            "kind":"object",
            "fields":fields.iter().map(|field| json!({"name":field.name,"required":field.required,"type":typescript_type(&field.expression)})).collect::<Vec<_>>(),
            "extraType":extra.as_ref().map(|extra|typescript_type(extra)),
        }),
        Expr::Union(variants) => {
            json!({"kind":"union","variants":variants.iter().map(typescript_type).collect::<Vec<_>>()})
        }
        Expr::Intersection(members) => {
            json!({"kind":"intersection","members":members.iter().map(typescript_type).collect::<Vec<_>>()})
        }
    }
}

pub(super) fn rust(contract: &Contract, plan: &rust_models::ModelPlan) -> Vec<NativeModel> {
    let names: BTreeMap<_, _> = plan
        .symbols()
        .iter()
        .map(|symbol| {
            (
                (symbol.source().clone(), symbol.role()),
                symbol.name().to_owned(),
            )
        })
        .collect();
    plan.symbols().iter().map(|symbol| {
        let descriptor = plan.declarations.get(&(symbol.source().clone(), symbol.role())).map(|declaration| {
            use rust_models::Decl;
            match declaration {
                Decl::Alias(ty) => json!({"kind":"alias","type":rust_type(ty, &names)}),
                Decl::Struct { fields, extras } => json!({
                    "kind":"struct",
                    "fields":fields.iter().map(|field| json!({"name":field.name,"wire":field.wire,"type":rust_type(&field.ty, &names),"initialization":rust_init(field.init.as_ref(), &names)})).collect::<Vec<_>>(),
                    "extraType":extras.as_ref().map(|(_, ty)| rust_type(ty, &names)),
                    "constructor":{"name":"new","parameters":fields.iter().filter(|field|field.init.is_none()).map(|field|json!({"name":field.name,"type":rust_type(&field.ty, &names)})).collect::<Vec<_>>()},
                    "defaultAvailable":fields.iter().all(|field|field.init.is_some()),
                }),
                Decl::Enum(variants) => json!({"kind":"enum","variants":variants.iter().map(|variant| json!({"name":variant.name,"type":rust_type(&variant.ty, &names)})).collect::<Vec<_>>()}),
                Decl::Literals(values) => json!({"kind":"literals","variants":values.iter().map(|value| json!({"name":value.name,"value":value.value})).collect::<Vec<_>>()}),
            }
        });
        NativeModel { source: Location::at(contract, symbol.source()), name: symbol.name().into(), role: role(symbol.role()).into(), descriptor }
    }).collect()
}

fn rust_init(
    init: Option<&rust_models::Init>,
    names: &BTreeMap<rust_models::Key, String>,
) -> Value {
    match init {
        None => json!({"kind":"argument"}),
        Some(rust_models::Init::Absent) => json!({"kind":"absent"}),
        Some(rust_models::Init::Optional) => json!({"kind":"none"}),
        Some(rust_models::Init::Literal(key, variant)) => {
            json!({"kind":"literal","model":names[key],"variant":variant})
        }
    }
}

fn rust_type(ty: &rust_models::Type, names: &BTreeMap<rust_models::Key, String>) -> Value {
    use rust_models::Type;
    match ty {
        Type::Primitive(name) => json!({"kind":"primitive","name":name}),
        Type::Named(key) => json!({"kind":"named","name":names[key]}),
        Type::Nullable(ty) => wrapper("nullable", rust_type(ty, names)),
        Type::Optional(ty) => wrapper("optional", rust_type(ty, names)),
        Type::Presence(ty) => wrapper("presence", rust_type(ty, names)),
        Type::Vec(ty) => wrapper("vec", rust_type(ty, names)),
        Type::Map(ty) => wrapper("map", rust_type(ty, names)),
        Type::Boxed(ty) => wrapper("box", rust_type(ty, names)),
    }
}

pub(super) fn python(contract: &Contract, plan: &python_models::ModelPlan) -> Vec<NativeModel> {
    let names = plan
        .symbols()
        .iter()
        .map(|symbol| (symbol.source().clone(), symbol.name().to_owned()))
        .collect();
    plan.symbols().iter().map(|symbol| {
        let descriptor = plan.declarations().get(symbol.source()).map(|declaration| match declaration {
            python_models::PyDecl::Alias(ty) => json!({"kind":"alias","type":python_type(ty, &names)}),
            python_models::PyDecl::Dataclass { fields, extras } => json!({
                "kind":"dataclass","keywordOnly":true,
                "fields":fields.iter().map(|field| json!({"name":field.name,"wire":field.wire,"type":python_type(&field.ty, &names),"required":field.required,"fixed":field.fixed,"constructorParameter":field.fixed.is_none()})).collect::<Vec<_>>(),
                "extraType":extras.as_ref().map(|ty| python_type(ty, &names)),
            }),
        });
        NativeModel { source: Location::at(contract, symbol.source()), name: symbol.name().into(), role: role(symbol.role()).into(), descriptor }
    }).collect()
}

fn python_type(ty: &python_models::PyType, names: &BTreeMap<SchemaId, String>) -> Value {
    use python_models::PyType;
    match ty {
        PyType::Primitive(name) => json!({"kind":"primitive","name":name}),
        PyType::JsonValue => json!({"kind":"primitive","name":"_json.JsonValue"}),
        PyType::Named(id) => json!({"kind":"named","name":names[id]}),
        PyType::Nullable(ty) => wrapper("nullable", python_type(ty, names)),
        PyType::Optional(ty) => wrapper("unset", python_type(ty, names)),
        PyType::List(ty) => wrapper("list", python_type(ty, names)),
        PyType::Map(ty) => wrapper("dict", python_type(ty, names)),
        PyType::Union(types) => {
            json!({"kind":"union","variants":types.iter().map(|(_, ty)| python_type(ty, names)).collect::<Vec<_>>()})
        }
        PyType::Literal(values) => json!({"kind":"literal","values":values}),
    }
}

pub(super) fn go(contract: &Contract, plan: &go_models::ModelPlan) -> Vec<NativeModel> {
    let descriptors = plan.descriptors();
    let names = &descriptors.names;
    plan.symbols().iter().map(|symbol| {
        let descriptor = descriptors.declarations.get(&(symbol.source().clone(), symbol.role())).map(|declaration| match declaration {
            go_models::GoDecl::Alias(ty) => json!({"kind":"alias","type":go_type(ty, names)}),
            go_models::GoDecl::Struct { fields, extras } => json!({
                "kind":"struct",
                "fields":fields.iter().map(|field| json!({"name":field.name,"wire":field.wire,"type":go_type(&field.ty, names),"constructorParameter":field.param,"constructorRequired":field.init.is_none()})).collect::<Vec<_>>(),
                "extraType":extras.as_ref().map(|ty| go_type(ty, names)),
            }),
            go_models::GoDecl::Literals { underlying, values } => json!({"kind":"literals","underlying":underlying,"variants":values.iter().map(|value| json!({"name":value.name,"token":value.token})).collect::<Vec<_>>()}),
            go_models::GoDecl::Union(variants) => json!({"kind":"union","variants":variants.iter().map(|variant| json!({"name":variant.name,"type":go_type(&variant.ty, names)})).collect::<Vec<_>>()}),
        });
        NativeModel { source: Location::at(contract, symbol.source()), name: symbol.name().into(), role: role(symbol.role()).into(), descriptor }
    }).collect()
}

fn go_type(ty: &go_models::GoType, names: &BTreeMap<go_models::Key, String>) -> Value {
    use go_models::GoType;
    match ty {
        GoType::Primitive(name) => json!({"kind":"primitive","name":name}),
        GoType::Named(key) => json!({"kind":"named","name":names[key]}),
        GoType::Nullable(ty) => wrapper("nullable", go_type(ty, names)),
        GoType::Optional(ty) => wrapper("optional", go_type(ty, names)),
        GoType::Presence(ty) => wrapper("presence", go_type(ty, names)),
        GoType::Slice(ty) => wrapper("slice", go_type(ty, names)),
        GoType::Map(ty) => wrapper("map", go_type(ty, names)),
        GoType::Pointer(ty) => wrapper("pointer", go_type(ty, names)),
    }
}

fn wrapper(kind: &str, inner: Value) -> Value {
    json!({"kind":kind,"of":inner})
}
