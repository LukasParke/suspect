//! Source-validated example expressions, lowered once from the retained native plan.

use super::models::{Extras, ModelPlan, Shape};
use serde_json::Value;
use suspect_ir::contract::{SchemaId, SourceId};
use suspect_schema::{OwnedOutcome, OwnedSchema};

#[derive(Debug)]
pub(super) enum Expression {
    Null,
    Boolean(bool),
    String(String),
    Number(String),
    Array(Vec<Expression>),
    Members(Vec<(String, Expression)>),
    Construct {
        name: String,
        arguments: Vec<(String, Expression)>,
    },
    Enum {
        name: String,
        case: String,
    },
    /// Only the schema's actual arbitrary JSON domain uses JsonValue factories.
    Json(Value),
}

#[derive(Debug)]
pub(super) struct NativeExample {
    pub operation: SourceId,
    pub entry_index: usize,
    pub expression: Expression,
}

pub(super) fn plan(
    models: &ModelPlan,
    validation: &OwnedSchema,
    examples: &crate::examples::ExamplePlan,
) -> Vec<NativeExample> {
    examples
        .operations()
        .iter()
        .flat_map(|op| {
            op.entries
                .iter()
                .enumerate()
                .map(|(entry_index, entry)| NativeExample {
                    operation: op.source.clone(),
                    entry_index,
                    expression: expression(models, validation, &entry.schema, &entry.value, 0),
                })
        })
        .collect()
}

fn expression(
    models: &ModelPlan,
    validation: &OwnedSchema,
    id: &SchemaId,
    value: &Value,
    depth: usize,
) -> Expression {
    assert!(depth < 256, "bounded validated example graph");
    let node = &models.nodes[id];
    if node.nullable && value.is_null() {
        return Expression::Null;
    }
    match &node.shape {
        Shape::Json => Expression::Json(value.clone()),
        Shape::Null => Expression::Null,
        Shape::Boolean => Expression::Boolean(value.as_bool().expect("validated boolean")),
        Shape::Number => Expression::Number(value.to_string()),
        Shape::String => Expression::String(value.as_str().expect("validated string").into()),
        Shape::Enum { cases } => Expression::Enum {
            name: node.name.clone(),
            case: cases
                .iter()
                .find(|(_, v)| value.as_str() == Some(v.as_str()))
                .expect("validated enum")
                .0
                .clone(),
        },
        Shape::Ref(target) => expression(models, validation, target, value, depth + 1),
        Shape::Union(branches) => {
            let branch = branches
                .iter()
                .find(|id| matches!(validation.validate(id, value), OwnedOutcome::Valid))
                .expect("validated union example");
            expression(models, validation, branch, value, depth + 1)
        }
        Shape::Array { item } => Expression::Array(
            value
                .as_array()
                .expect("validated array")
                .iter()
                .map(|v| {
                    item.as_ref()
                        .map(|id| expression(models, validation, id, v, depth + 1))
                        .unwrap_or_else(|| Expression::Json(v.clone()))
                })
                .collect(),
        ),
        Shape::Types(_) => match value {
            Value::Null => Expression::Null,
            Value::Bool(v) => Expression::Boolean(*v),
            Value::String(v) => Expression::String(v.clone()),
            Value::Number(v) => Expression::Number(v.to_string()),
            Value::Array(v) => Expression::Array(v.iter().cloned().map(Expression::Json).collect()),
            Value::Object(_) => Expression::Json(value.clone()),
        },
        Shape::Object { fields, extras } => {
            let object = value.as_object().expect("validated object");
            let mut args: Vec<_> = fields
                .iter()
                .filter(|f| f.initializer.is_none())
                .filter_map(|f| {
                    object.get(&f.wire).map(|v| {
                        (
                            f.name.clone(),
                            expression(models, validation, &f.source, v, depth + 1),
                        )
                    })
                })
                .collect();
            let extra: Vec<_> = object
                .iter()
                .filter(|(k, _)| !fields.iter().any(|f| &f.wire == *k))
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match extras {
                            Extras::Typed(id) => expression(models, validation, id, v, depth + 1),
                            _ => Expression::Json(v.clone()),
                        },
                    )
                })
                .collect();
            if !extra.is_empty() {
                args.push(("extra".into(), Expression::Members(extra)));
            }
            Expression::Construct {
                name: node.name.clone(),
                arguments: args,
            }
        }
    }
}
