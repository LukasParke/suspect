//! Direct Swift constructors from already planned native types and validated
//! source values. There is no decode-from-JSON detour in executable quickstarts.
use serde_json::{Value, json};
use suspect_ir::contract::SchemaId;
use suspect_schema::OwnedOutcome;

use super::{
    PlannedOperation, SdkPlan,
    models::{Declaration, Type},
    protocol_metadata::q,
};
use crate::http_protocol::{PartMultiplicity, PartRepresentation, Representation};

pub(super) fn value(plan: &SdkPlan, ty: &Type, value: &Value, depth: usize) -> Option<String> {
    if depth > 32 {
        return None;
    }
    Some(match ty {
        Type::Primitive("String") => q(value.as_str()?),
        Type::Primitive("Bool") => value.as_bool()?.to_string(),
        Type::Primitive("JsonInteger") => {
            format!("try JsonInteger({})", q(&value.as_number()?.to_string()))
        }
        Type::Primitive("JsonNumber") => {
            format!("try JsonNumber({})", q(&value.as_number()?.to_string()))
        }
        Type::Primitive("JsonNull") if value.is_null() => "JsonNull()".into(),
        Type::Primitive("JsonValue") => public_json(value),
        Type::Primitive(_) => return None,
        Type::Nullable(inner) => {
            if value.is_null() {
                ".null".into()
            } else {
                format!(".value({})", self::value(plan, inner, value, depth + 1)?)
            }
        }
        Type::Indirect(inner) => format!(".value({})", self::value(plan, inner, value, depth + 1)?),
        Type::Array(inner) => format!(
            "[{}]",
            value
                .as_array()?
                .iter()
                .map(|v| self::value(plan, inner, v, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        Type::Named(id) => {
            let name = &plan.models.names[id];
            let declaration = &plan.models.declarations[id];
            match declaration {
                Declaration::Checked { value: carrier } => format!(
                    "{name}(value: {})",
                    self::value(plan, carrier, value, depth + 1)?
                ),
                Declaration::Literals(values) => format!(
                    "{name}.{}",
                    values
                        .iter()
                        .find(|(_, v)| Some(v.as_str()) == value.as_str())?
                        .0
                ),
                Declaration::Union(variants) => {
                    let variant = variants.iter().find(|v| {
                        matches!(
                            plan.validator.validate(&v.source, value),
                            OwnedOutcome::Valid
                        )
                    })?;
                    format!(
                        "{name}.{}({})",
                        variant.name,
                        self::value(plan, &variant.ty, value, depth + 1)?
                    )
                }
                Declaration::Object { fields, extras } => {
                    let object = value.as_object()?;
                    let mut args = Vec::new();
                    for f in declaration.constructor_fields(&plan.models) {
                        if let Some(v) = object.get(&f.wire) {
                            let expression = if f.required {
                                self::value(plan, &f.model_type.full(), v, depth + 1)?
                            } else if v.is_null() && f.model_type.nullable {
                                ".null".into()
                            } else {
                                format!(
                                    ".value({})",
                                    self::value(plan, &f.model_type.core, v, depth + 1)?
                                )
                            };
                            args.push(format!("{}: {expression}", f.name));
                        } else if f.required && f.default(&plan.models).is_none() {
                            return None;
                        }
                    }
                    if let Some(extra) = extras {
                        let values = object
                            .iter()
                            .filter(|(key, _)| !fields.iter().any(|f| &f.wire == *key))
                            .map(|(k, v)| {
                                Some(format!(
                                    "({}, {})",
                                    q(k),
                                    self::value(plan, extra, v, depth + 1)?
                                ))
                            })
                            .collect::<Option<Vec<_>>>()?;
                        if !values.is_empty() {
                            args.push(format!(
                                "additionalProperties: try JsonObject<{}>([{}])",
                                extra.render(&plan.models),
                                values.join(", ")
                            ));
                        }
                    }
                    format!("{name}({})", args.join(", "))
                }
            }
        }
    })
}
fn public_json(v: &Value) -> String {
    match v {
        Value::Null => "JsonValue.null".into(),
        Value::Bool(b) => format!("JsonValue.bool({b})"),
        Value::String(s) => format!("JsonValue.string({})", q(s)),
        Value::Number(n) => format!("JsonValue.number(try JsonNumber({}))", q(&n.to_string())),
        Value::Array(a) => format!(
            "JsonValue.array([{}])",
            a.iter().map(public_json).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(o) => format!(
            "JsonValue.object(try JsonObject<JsonValue>([{}]))",
            o.iter()
                .map(|(k, v)| format!("({}, {})", q(k), public_json(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn sample(plan: &SdkPlan, id: &SchemaId, depth: usize) -> Option<Value> {
    if depth > 24 {
        return None;
    }
    let valid = |v: &Value| matches!(plan.validator.validate(id, v), OwnedOutcome::Valid);
    if let Some(v) = plan
        .examples
        .operations()
        .iter()
        .flat_map(|o| &o.entries)
        .find(|e| &e.schema == id)
        .map(|e| &e.value)
    {
        return Some(v.clone());
    }
    if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        // The shared bounded v3 planner owns source/example discovery. In
        // particular a failed dynamic evaluation cannot be turned into a native
        // fallback example by trying annotations from another binding or scope.
        return None;
    }
    let raw = plan.contract.schema(id)?.raw();
    for candidate in raw
        .get("examples")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(raw.get("example"))
        .chain(raw.get("const"))
        .chain(
            raw.get("enum")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .take(32)
    {
        if valid(candidate) {
            return Some(candidate.clone());
        }
    }
    if let Some(target) = plan
        .contract
        .schema(id)?
        .references()
        .iter()
        .find(|r| r.keyword == "$ref")
        .and_then(|r| r.target.as_ref())
        && let Some(v) = sample(plan, target, depth + 1).filter(valid)
    {
        return Some(v);
    }
    let ty = &plan.models.types.get(id)?.core;
    let candidate = match ty {
        Type::Named(target) => match &plan.models.declarations[target] {
            Declaration::Checked { .. } => None,
            Declaration::Literals(values) => values.first().map(|(_, v)| Value::String(v.clone())),
            Declaration::Union(variants) => variants
                .iter()
                .find_map(|v| sample(plan, &v.source, depth + 1).filter(valid)),
            Declaration::Object { fields, .. } => {
                let mut object = serde_json::Map::new();
                for f in fields.iter().filter(|f| f.required) {
                    object.insert(f.wire.clone(), sample(plan, &f.source, depth + 1)?);
                }
                let minimum = raw
                    .get("minProperties")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(32) as usize;
                for f in fields.iter().filter(|f| !f.required) {
                    if object.len() >= minimum {
                        break;
                    }
                    if let Some(v) = sample(plan, &f.source, depth + 1) {
                        object.insert(f.wire.clone(), v);
                    }
                }
                Some(Value::Object(object))
            }
        },
        Type::Array(_) => {
            let count = raw.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            if count > 16 {
                None
            } else if count == 0 {
                Some(json!([]))
            } else {
                Some(Value::Array(vec![
                    sample(
                        plan,
                        &id.child("items"),
                        depth + 1
                    )?;
                    count as usize
                ]))
            }
        }
        _ => None,
    };
    if let Some(v) = candidate.filter(valid) {
        return Some(v);
    }
    let mut candidates = Vec::new();
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "default",
    ] {
        if let Some(v) = raw.get(key) {
            candidates.push(v.clone());
            if let Some(n) = v.as_i64()
                && let Some(n) = if key == "exclusiveMinimum" {
                    n.checked_add(1)
                } else {
                    n.checked_sub(1)
                }
            {
                candidates.push(json!(n));
            }
        }
    }
    let length = raw.get("minLength").and_then(Value::as_u64).unwrap_or(1);
    if length <= 4096 {
        candidates.push(json!("a".repeat(length as usize)));
    }
    candidates.extend([
        json!("example"),
        json!("x"),
        json!(0),
        json!(1),
        json!(-1),
        json!(true),
        json!(false),
        Value::Null,
    ]);
    candidates.into_iter().find(valid)
}
fn sample_expression(plan: &SdkPlan, id: &SchemaId) -> Option<String> {
    value(
        plan,
        &plan.models.types.get(id)?.full(),
        &sample(plan, id, 0)?,
        0,
    )
}
fn part_example(plan: &SdkPlan, p: &super::PlannedPart, multipart: bool) -> Option<String> {
    let value = match p.wire.representation() {
        PartRepresentation::Binary { .. } => "Data()".into(),
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => sample_expression(plan, codec.schema().id())?,
    };
    let value = part_item(plan, p, multipart, value)?;
    if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        let count = p.wire.min_items().map(|n| *n.value()).unwrap_or(1).max(1);
        if count > 8 {
            return None;
        }
        Some(format!("[{}]", vec![value; count as usize].join(", ")))
    } else {
        Some(value)
    }
}

fn part_item(
    plan: &SdkPlan,
    p: &super::PlannedPart,
    multipart: bool,
    value: String,
) -> Option<String> {
    Some(if multipart {
        let mut args = vec![if p.header_type.is_some() {
            format!("value: {value}")
        } else {
            value
        }];
        if let Some(headers) = &p.header_type {
            let fields = p
                .headers
                .iter()
                .filter(|h| h.wire.required())
                .map(|h| {
                    Some(format!(
                        "{}: {}",
                        h.field_name,
                        sample_expression(plan, h.wire.codec().schema().id())?
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            args.push(format!("headers: {headers}({})", fields.join(", ")));
        }
        format!("{}({})", p.item_type, args.join(", "))
    } else {
        value
    })
}

fn declared_aggregate<'a>(
    plan: &'a SdkPlan,
    op: &PlannedOperation,
    media: &super::PlannedMedia,
) -> Option<&'a Value> {
    plan.examples
        .operations()
        .iter()
        .find(|examples| &examples.source == op.wire.source().terminal().source())?
        .validated_aggregates
        .iter()
        .find(|entry| {
            entry.role == crate::examples::ExampleRole::RequestBody
                && &entry.container == media.wire.source().use_site().source()
                && entry.media_type == media.wire.media_type().declared()
        })
        .map(|entry| &entry.value)
}

fn aggregate_part(
    plan: &SdkPlan,
    p: &super::PlannedPart,
    multipart: bool,
    instance: &Value,
) -> Option<String> {
    let codec = match p.wire.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => codec,
        PartRepresentation::Binary { .. } => return None,
    };
    let item = |instance: &Value| {
        if !matches!(
            plan.validator.validate(codec.schema().id(), instance),
            OwnedOutcome::Valid
        ) {
            return None;
        }
        part_item(
            plan,
            p,
            multipart,
            value(
                plan,
                &plan.models.types.get(codec.schema().id())?.full(),
                instance,
                0,
            )?,
        )
    };
    if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        let array = instance.as_array()?;
        if array.len() > 128 || array.is_empty() && p.wire.required() {
            return None;
        }
        Some(format!(
            "[{}]",
            array
                .iter()
                .map(item)
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ))
    } else {
        item(instance)
    }
}

fn aggregate_body(plan: &SdkPlan, media: &super::PlannedMedia, instance: &Value) -> Option<String> {
    if let Some(parts) = &media.positional {
        let array = instance.as_array()?;
        if array.len() > 128 {
            return None;
        }
        let mut fields = Vec::new();
        for (index, part) in parts.prefix.iter().enumerate() {
            if let Some(value) = array.get(index) {
                let value = aggregate_part(plan, part, true, value)?;
                fields.push(format!(
                    "{}: {}",
                    part.field_name,
                    if part.wire.required() {
                        value
                    } else {
                        format!(".value({value})")
                    }
                ));
            } else if part.wire.required() {
                return None;
            }
        }
        if array.len() > parts.prefix.len() {
            let item = parts.items.as_ref()?;
            let tail = array[parts.prefix.len()..]
                .iter()
                .map(|value| aggregate_part(plan, item, true, value))
                .collect::<Option<Vec<_>>>()?;
            fields.push(format!("items: [{}]", tail.join(", ")));
        }
        return Some(format!("{}({})", parts.type_name, fields.join(", ")));
    }
    let parts = media.parts.as_ref()?;
    let object = instance.as_object()?;
    if object.len() > 128 {
        return None;
    }
    let mut ordered = parts.fields.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|part| !part.wire.required());
    let mut fields = Vec::new();
    let mut present = std::collections::BTreeSet::new();
    for part in ordered {
        let name = part.wire.name()?;
        if let Some(value) = object.get(name) {
            let expression = aggregate_part(plan, part, parts.multipart, value)?;
            if part.wire.multiplicity() != PartMultiplicity::RepeatedArrayItems
                || value.as_array().is_some_and(|v| !v.is_empty())
            {
                present.insert(name);
            }
            fields.push(format!(
                "{}: {}",
                part.field_name,
                if part.wire.required() {
                    expression
                } else {
                    format!(".value({expression})")
                }
            ));
        } else if part.wire.required() {
            return None;
        }
    }
    let mut extras = Vec::new();
    for (name, value) in object.iter().filter(|(name, _)| {
        !parts
            .fields
            .iter()
            .any(|p| p.wire.name() == Some(name.as_str()))
    }) {
        let part = parts.additional.as_ref()?;
        let expression = aggregate_part(plan, part, parts.multipart, value)?;
        if part.wire.multiplicity() != PartMultiplicity::RepeatedArrayItems
            || value.as_array().is_some_and(|v| !v.is_empty())
        {
            present.insert(name.as_str());
        }
        extras.push(format!("({}, {expression})", q(name)));
    }
    if !extras.is_empty() {
        fields.push(format!(
            "additionalProperties: try JsonObject([{}])",
            extras.join(", ")
        ));
    }
    // Empty repeated arrays have no wire member. Do not invent an element to
    // make an otherwise unrepresentable declared aggregate satisfy wire rules.
    if parts
        .rules
        .required()
        .iter()
        .any(|name| !present.contains(name.value().as_str()))
        || parts
            .rules
            .min_properties()
            .is_some_and(|n| (present.len() as u64) < *n.value())
        || parts
            .rules
            .max_properties()
            .is_some_and(|n| (present.len() as u64) > *n.value())
    {
        return None;
    }
    Some(format!("{}({})", parts.type_name, fields.join(", ")))
}

pub(super) fn call(plan: &SdkPlan, op: &PlannedOperation) -> Option<String> {
    let mut args = Vec::new();
    for p in op.parameters.iter().filter(|p| p.wire.required()) {
        args.push(format!(
            "{}: {}",
            p.field_name,
            sample_expression(plan, p.wire.codec().schema().id())?
        ));
    }
    if let Some(body) = op.body.as_ref().filter(|b| {
        b.wire.required()
            || b.media
                .iter()
                .any(|media| declared_aggregate(plan, op, media).is_some())
    }) {
        let media = body
            .media
            .iter()
            .find(|media| declared_aggregate(plan, op, media).is_some())
            .or_else(|| {
                body.media.iter().find(|m| {
                    matches!(
                        m.wire.representation(),
                        Representation::Json { codec: Some(_) }
                    )
                })
            })
            .or_else(|| body.media.first())?;
        let value = if let Some(instance) = declared_aggregate(plan, op, media) {
            aggregate_body(plan, media, instance)?
        } else {
            match media.wire.representation() {
                Representation::Json { codec: Some(codec) }
                | Representation::Text {
                    codec: Some(codec), ..
                } => sample_expression(plan, codec.schema().id())?,
                Representation::Json { codec: None } => {
                    "JsonValue.object(try JsonObject<JsonValue>([]))".into()
                }
                Representation::Text { codec: None, .. } => q("example"),
                Representation::Binary { .. } => "Data()".into(),
                Representation::Multipart { .. } if media.positional.is_some() => {
                    let parts = media.positional.as_ref()?;
                    let mut fields = parts
                        .prefix
                        .iter()
                        .filter(|p| p.wire.required())
                        .map(|p| {
                            Some(format!(
                                "{}: {}",
                                p.field_name,
                                part_example(plan, p, true)?
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let minimum = parts.min_items.as_ref().map(|n| *n.value()).unwrap_or(0);
                    if minimum > parts.prefix.len() as u64 {
                        let item = parts.items.as_ref()?;
                        let count = minimum - parts.prefix.len() as u64;
                        if count > 8 {
                            return None;
                        }
                        fields.push(format!(
                            "items: [{}]",
                            vec![part_example(plan, item, true)?; count as usize].join(", ")
                        ));
                    }
                    format!("{}({})", parts.type_name, fields.join(", "))
                }
                Representation::Form { .. } | Representation::Multipart { .. } => {
                    let parts = media.parts.as_ref()?;
                    let fields = parts
                        .fields
                        .iter()
                        .filter(|p| p.wire.required())
                        .map(|p| {
                            Some(format!(
                                "{}: {}",
                                p.field_name,
                                part_example(plan, p, parts.multipart)?
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    format!("{}({})", parts.type_name, fields.join(", "))
                }
                Representation::Stream { .. } => return None,
            }
        };
        let value = if body.is_enum {
            let content = if matches!(
                media.wire.media_type().range(),
                crate::http_protocol::MediaRange::Concrete { .. }
            ) {
                String::new()
            } else {
                ", contentType: \"application/octet-stream\"".into()
            };
            format!("{}.{}({value}{content})", body.type_name, media.case_name)
        } else {
            value
        };
        args.push(format!(
            "body: {}",
            if body.wire.required() {
                value
            } else {
                format!(".value({value})")
            }
        ));
    }
    let input = if args.is_empty() {
        String::new()
    } else {
        format!("{}({})", op.input_type, args.join(", "))
    };
    Some(format!(
        "        return try await client.{}({input})\n",
        op.method_name
    ))
}
