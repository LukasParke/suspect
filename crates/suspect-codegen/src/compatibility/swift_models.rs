//! Swift declarations and codec properties from the retained native plan.
//!
//! A source can own both a non-null declaration and a codec for its full value
//! domain. These are distinct roles, even when their SourceId is identical.
//! Scalar/ref type bindings are recorded in codec signatures; a planning name
//! hint is not invented into an exported Swift type declaration.

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};

use crate::swift_sdk::models::{Declaration, Field, Initializer, ModelPlan, Type};

use super::{Location, native::NativeModel};

pub(super) fn capture(contract: &Contract, plan: &ModelPlan) -> Vec<NativeModel> {
    let mut records = Vec::new();
    for (source, declaration) in plan.declarations() {
        let descriptor = match declaration {
            Declaration::Checked { value } => json!({
                "kind":"checked-carrier","conformances":["SourceCodable","Equatable"],
                "value":{"name":"value","type":type_descriptor(value,plan),"mutable":true},
                "wireRepresentation":"complete-value","validation":"source-codec-on-decode-and-encode",
                "constructor":{"name":"init","parameters":[{"name":"value","type":type_descriptor(value,plan),"hasDefault":false}],"canInitializeWithoutArguments":false}
            }),
            Declaration::Object { fields, extras } => {
                let mut arguments = declaration.constructor_fields(plan).iter().map(|field| {
                    json!({"name":field.name,"type":field_type(field, plan),
                        "hasDefault":field.initializer(plan).is_some(),"initialization":initializer(field.initializer(plan), plan)})
                }).collect::<Vec<_>>();
                let extra_type = extras
                    .as_ref()
                    .map(|ty| wrapper("json-object", type_descriptor(ty, plan)));
                if let Some(ty) = &extra_type {
                    arguments.push(json!({"name":"additionalProperties","type":ty,"hasDefault":true,"initialization":{"kind":"empty-object"}}));
                }
                json!({
                    "kind":"struct","conformances":["SourceCodable","Equatable"],
                    "fields":fields.iter().map(|field|json!({"name":field.name,"wire":field.wire,"required":field.required,
                        "type":field_type(field, plan),"initialization":initializer(field.initializer(plan), plan)})).collect::<Vec<_>>(),
                    "additionalProperties":extra_type.map(|ty|json!({"member":"additionalProperties","type":ty})),
                    "constructor":{"name":"init","parameters":arguments,"canInitializeWithoutArguments":fields.iter().all(|field|field.initializer(plan).is_some())},
                })
            }
            Declaration::Literals(values) => json!({
                "kind":"literals","conformances":["SourceCodable","Equatable"],
                "variants":values.iter().map(|(name, value)|json!({"name":name,"value":value})).collect::<Vec<_>>(),
                "rawValue":{"name":"rawValue","type":{"kind":"primitive","name":"String"},"mutable":false},
            }),
            Declaration::Union(variants) => json!({
                "kind":"union","indirect":true,"conformances":["SourceCodable","Equatable"],
                "variants":variants.iter().map(|variant|json!({"name":variant.name,"type":type_descriptor(&variant.ty, plan)})).collect::<Vec<_>>(),
            }),
        };
        let mut descriptor = descriptor;
        // Declaration.codec always consumes the non-null declaration. The
        // separately allocated Codecs member below can consume Nullable<T>.
        descriptor["codec"] = json!({"member":"codec","static":true,"mutable":false,
            "type":wrapper("model-codec", type_descriptor(&Type::Named(source.clone()), plan))});
        records.push(NativeModel {
            source: Location::at(contract, source),
            name: plan.names()[source].clone(),
            role: "model".into(),
            descriptor: Some(descriptor),
        });
    }
    for (source, member) in plan.codec_symbols() {
        records.push(NativeModel {
            source: Location::at(contract, source),
            name: codec_name(plan, source),
            role: "codec".into(),
            descriptor: Some(json!({"kind":"codec","owner":"Codecs","member":member,
                "static":true,"mutable":false,"type":wrapper("model-codec", type_at(plan, source))})),
        });
    }
    records
}

pub(super) fn codec_name(plan: &ModelPlan, source: &SchemaId) -> String {
    format!("Codecs.{}", plan.codec_symbols()[source])
}

pub(super) fn type_at(plan: &ModelPlan, source: &SchemaId) -> Value {
    type_descriptor(&plan.types()[source].full(), plan)
}

pub(super) fn input_type(plan: &ModelPlan, source: &SchemaId, required: bool) -> Value {
    let ty = type_at(plan, source);
    if required {
        ty
    } else {
        wrapper("optional-field", ty)
    }
}

fn field_type(field: &Field, plan: &ModelPlan) -> Value {
    if field.required {
        type_descriptor(&field.model_type.full(), plan)
    } else {
        wrapper(
            if field.model_type.nullable {
                "presence"
            } else {
                "optional-field"
            },
            type_descriptor(&field.model_type.core, plan),
        )
    }
}

fn type_descriptor(ty: &Type, plan: &ModelPlan) -> Value {
    match ty {
        Type::Primitive(name) => json!({"kind":"primitive","name":name}),
        Type::Named(source) => json!({"kind":"named","name":plan.names()[source]}),
        Type::Array(inner) => wrapper("array", type_descriptor(inner, plan)),
        Type::Nullable(inner) => wrapper("nullable", type_descriptor(inner, plan)),
        Type::Indirect(inner) => wrapper("indirect", type_descriptor(inner, plan)),
    }
}

fn initializer(value: Option<Initializer>, plan: &ModelPlan) -> Value {
    match value {
        None => json!({"kind":"argument"}),
        Some(Initializer::Missing) => json!({"kind":"missing"}),
        Some(Initializer::StringLiteral {
            source,
            case_name,
            wire_value,
        }) => {
            json!({"kind":"literal","model":plan.names()[&source],"case":case_name,"value":wire_value})
        }
    }
}

fn wrapper(kind: &str, inner: Value) -> Value {
    json!({"kind":kind,"of":inner})
}
