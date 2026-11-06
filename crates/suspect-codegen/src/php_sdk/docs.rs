//! Executable native PHP expressions for retained example descriptors.

use super::super::samples::Expression;
use super::php;
use serde_json::Value;

pub(in crate::php_sdk) fn render_at(expression: &Expression, prefix: &str) -> String {
    match expression {
        Expression::Null => "null".into(),
        Expression::Boolean(v) => v.to_string(),
        Expression::String(v) => php(v),
        Expression::Number(v) => format!("{prefix}JsonNumber::fromString({})", php(v)),
        Expression::Array(v) => format!(
            "[{}]",
            v.iter()
                .map(|v| render_at(v, prefix))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Members(v) => format!(
            "[{}]",
            v.iter()
                .map(|(k, v)| format!("{} => {}", php(k), render_at(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Construct { name, arguments } => format!(
            "new {prefix}{name}({})",
            arguments
                .iter()
                .map(|(k, v)| format!("{k}: {}", render_at(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Enum { name, case } => format!("{prefix}{name}::{case}"),
        Expression::Json(value) => json_expression(value, prefix),
    }
}

fn json_expression(value: &Value, prefix: &str) -> String {
    match value {
        Value::Null => format!("{prefix}JsonValue::null()"),
        Value::Bool(v) => format!("{prefix}JsonValue::fromBool({v})"),
        Value::String(v) => format!("{prefix}JsonValue::fromString({})", php(v)),
        Value::Number(v) => format!(
            "{prefix}JsonValue::fromNumber({prefix}JsonNumber::fromString({}))",
            php(&v.to_string())
        ),
        Value::Array(v) => format!(
            "{prefix}JsonValue::fromArray([{}])",
            v.iter()
                .map(|v| json_expression(v, prefix))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(v) => format!(
            "{prefix}JsonValue::fromObject([{}])",
            v.iter()
                .map(|(k, v)| format!("{} => {}", php(k), json_expression(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
