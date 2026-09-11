//! Typed native example construction. No emitted-code or schema-text parsing.
use serde_json::Value;
use suspect_ir::contract::SourceId;

use super::{ExtraFields, ModelPlan, ModelShape};
use crate::examples::ExamplePlan;

/// A native constructor expression; the emitter supplies only Ruby syntax.
#[derive(Debug, Clone)]
pub enum SampleValue {
    Json(Value),
    Array(Vec<SampleValue>),
    Object {
        schema_index: usize,
        fields: Vec<(String, SampleValue)>,
        extras: Vec<(String, SampleValue)>,
    },
    /// Union dispatch is performed by its checked source codec, keeping branch
    /// selection and the parent assertions on the ordinary runtime path.
    Decoded {
        schema_index: usize,
        value: Value,
    },
}

#[derive(Debug, Clone)]
pub struct NativeExample {
    pub operation_source: SourceId,
    pub entry_index: usize,
    pub schema_index: usize,
    pub value: SampleValue,
}

pub(super) fn plan(
    models: &ModelPlan,
    examples: &ExamplePlan,
    resources: bool,
) -> Vec<NativeExample> {
    examples
        .operations()
        .iter()
        .flat_map(|op| {
            op.entries
                .iter()
                .enumerate()
                .filter_map(|(entry_index, entry)| {
                    let symbol = models.source_symbol(&entry.schema)?;
                    Some(NativeExample {
                        operation_source: op.source.clone(),
                        entry_index,
                        schema_index: symbol.schema_index,
                        // Nested constructors validate at their own source root;
                        // a resource-aware example must retain its actual outer
                        // entry during validation and materialization instead.
                        value: if resources {
                            SampleValue::Decoded {
                                schema_index: symbol.schema_index,
                                value: entry.value.clone(),
                            }
                        } else {
                            lower(models, symbol.schema_index, &entry.value, 0)
                        },
                    })
                })
        })
        .collect()
}

pub(super) fn lower(models: &ModelPlan, index: usize, value: &Value, depth: usize) -> SampleValue {
    // Example discovery already bounded candidate bytes/visits/depth. Alias
    // chains have a separate finite bound and retain a source-checked carrier.
    if depth >= 64 {
        return SampleValue::Decoded {
            schema_index: index,
            value: value.clone(),
        };
    }
    match &models.symbol(index).expect("retained native example").shape {
        ModelShape::Alias(target) => lower(models, *target, value, depth + 1),
        ModelShape::Object { fields, extras, .. } if value.is_object() => {
            let object = value.as_object().unwrap();
            let native_fields = fields
                .iter()
                .filter_map(|field| {
                    object.get(&field.wire_name).map(|child| {
                        (
                            field.name.clone(),
                            lower(models, field.schema_index, child, depth + 1),
                        )
                    })
                })
                .collect();
            let native_extras = object
                .iter()
                .filter(|(key, _)| !fields.iter().any(|field| &field.wire_name == *key))
                .map(|(key, child)| {
                    (
                        key.clone(),
                        if let ExtraFields::Typed(target) = extras {
                            lower(models, *target, child, depth + 1)
                        } else {
                            SampleValue::Json(child.clone())
                        },
                    )
                })
                .collect();
            SampleValue::Object {
                schema_index: index,
                fields: native_fields,
                extras: native_extras,
            }
        }
        ModelShape::Array { items, prefix, .. } if value.is_array() => SampleValue::Array(
            value
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    prefix.get(i).copied().or(*items).map_or_else(
                        || SampleValue::Json(value.clone()),
                        |index| lower(models, index, value, depth + 1),
                    )
                })
                .collect(),
        ),
        ModelShape::Union { .. } => SampleValue::Decoded {
            schema_index: index,
            value: value.clone(),
        },
        _ => SampleValue::Json(value.clone()),
    }
}
