//! Readable native values lowered from typed model expressions and validated
//! example entries. Ordinary request examples never start by decoding JSON text.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::{HttpSymbols, PlannedOperation};
use crate::examples::{ExamplePlan, ExampleRole};
use crate::typescript::{Expr, Literal, Primitive, codecs::CodecPlan};
use serde_json::Value;

/// A native first-request recipe rendered from actual allocated operation types.
#[derive(Debug, Clone)]
pub struct FirstRequest {
    pub operation: String,
    pub input_type: String,
    /// A validated native input literal, when the source examples establish one.
    pub input: Option<String>,
    pub input_optional: bool,
}
impl FirstRequest {
    /// Executable TypeScript source, parameterized by explicit caller policy.
    #[must_use]
    pub fn source(&self, package: &str) -> String {
        self.render(&format!(
            "import {{ createClient, type operations, JsonNumber }} from {};",
            serde_json::to_string(package).unwrap()
        ))
    }

    pub(super) fn local_source(&self) -> String {
        self.render("import { createClient } from '../operations.js';\nimport type * as operations from '../operations.js';\nimport { JsonNumber } from '../json.js';")
    }

    pub(super) fn documentation_source(&self) -> String {
        self.render("import { createClient } from './operations.js';\nimport type * as operations from './operations.js';\nimport { JsonNumber } from './json.js';")
    }

    fn render(&self, imports: &str) -> String {
        let parameters = if self.input.is_some() || self.input_optional {
            String::new()
        } else {
            format!(", input: operations.{}", self.input_type)
        };
        let argument =
            if self.input_optional && self.input.as_deref().is_none_or(|input| input == "{}") {
                ""
            } else {
                self.input.as_deref().unwrap_or("input")
            };
        format!(
            "{imports}\n\nexport async function firstRequest(options: operations.ClientOptions{parameters}) {{\n  const client = createClient(options);\n  const response = await client.{}({argument});\n  return response;\n}}\n",
            self.operation
        )
    }
}

pub(super) fn emit(
    operations: &[PlannedOperation],
    symbols: &HttpSymbols,
    examples: &ExamplePlan,
    codecs: &CodecPlan,
) -> (String, Option<FirstRequest>) {
    let definitions = codecs
        .models()
        .symbols()
        .iter()
        .map(|symbol| (symbol.name(), symbol.expression()))
        .collect::<BTreeMap<_, _>>();
    let mut code = String::from(
        "/** Executable native values from source-validated examples; origins remain in examples.json. */\nimport * as Codecs from '../model-codecs.js';\nimport type * as Models from '../models.js';\nimport type * as Operations from '../operations.js';\nimport { JsonNumber } from '../json.js';\n\n",
    );
    let mut count = 0;
    let mut first = None;
    for (op_index, operation) in operations.iter().enumerate() {
        let entries = examples
            .operations()
            .iter()
            .find(|entry| {
                entry.source == operation.source
                    || &entry.source == operation.protocol().source().use_site().source()
            })
            .map(|entry| entry.entries.as_slice())
            .unwrap_or(&[]);
        let mut values = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let names = if entry.role.is_response() {
                &symbols.response
            } else {
                &symbols.request
            };
            let Some(name) = names.get(&entry.schema) else {
                continue;
            };
            let Some(value) =
                native_value(&entry.value, definitions[name.as_str()], &definitions, 0)
            else {
                continue;
            };
            writeln!(code,"// {} example at {}\nconst value_{op_index}_{index}: Models.{name} = {value};\nCodecs.{name}Codec.decode(Codecs.{name}Codec.encode(value_{op_index}_{index}));",crate::http_examples::origin(&entry.origin),crate::typescript::escape_prose(entry.schema.pointer())).unwrap();
            values.insert(index, value);
            count += 1;
        }
        let mut input = Vec::new();
        let mut available = true;
        for parameter in &operation.parameters {
            if !parameter.required {
                continue;
            }
            let example = entries.iter().enumerate().find(|(_, entry)| {
                matches!(entry.role, ExampleRole::Parameter { .. })
                    && entry.container == parameter.source
            });
            if let Some((index, _)) =
                example.and_then(|entry| values.contains_key(&entry.0).then_some(entry))
            {
                input.push(format!(
                    "{}: {}",
                    serde_json::to_string(&parameter.native_name).unwrap(),
                    values[&index]
                ));
            } else {
                available = false;
            }
        }
        if let Some(body) = operation.protocol().body().filter(|body| body.required()) {
            let example = entries.iter().enumerate().find(|(_, entry)| {
                matches!(entry.role, ExampleRole::RequestBody)
                    && body
                        .media()
                        .iter()
                        .any(|media| media.source().use_site().source() == &entry.container)
            });
            if let Some((index, entry)) =
                example.and_then(|entry| values.contains_key(&entry.0).then_some(entry))
            {
                let body_value = if super::protocol_emit::tagged_body(body) {
                    format!(
                        "{{ contentType: {}, data: {} }}",
                        serde_json::to_string(&entry.media_type).unwrap(),
                        values[&index]
                    )
                } else {
                    values[&index].clone()
                };
                input.push(format!("body: {body_value}"));
            } else {
                available = false;
            }
        }
        let input = available.then(|| {
            if input.is_empty() {
                "{}".into()
            } else {
                format!("{{ {} }}", input.join(", "))
            }
        });
        if let Some(input) = &input {
            writeln!(code,"/** Minimal typed native input for {}. */\nexport const input_{op_index}: Operations.{} = {input};\n",crate::typescript::escape_prose(&operation.operation_id),operation.input_type).unwrap();
        }
        let recipe = FirstRequest {
            operation: operation.function_name.clone(),
            input_type: operation.input_type.clone(),
            input,
            input_optional: operation.input_optional(),
        };
        if first
            .as_ref()
            .is_none_or(|first: &FirstRequest| first.input.is_none() && recipe.input.is_some())
        {
            first = Some(recipe);
        }
    }
    writeln!(code, "console.log('validated-examples', {count});").unwrap();
    (code, first)
}

fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
fn key(value: &str) -> String {
    if value == "__proto__" {
        format!("[{}]", q(value))
    } else {
        q(value)
    }
}
fn any_value(value: &Value) -> String {
    match value {
        Value::Number(value) => format!("JsonNumber.parse({})", q(&value.to_string())),
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(any_value).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(values) => format!(
            "{{ {} }}",
            values
                .iter()
                .map(|(name, value)| format!("{}: {}", key(name), any_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => value.to_string(),
    }
}
fn field<'a>(
    expression: &'a Expr,
    key: &str,
    definitions: &BTreeMap<&str, &'a Expr>,
    depth: usize,
) -> Option<Expr> {
    if depth > 128 {
        return None;
    }
    match expression {
        Expr::At(_, expression) => field(expression, key, definitions, depth + 1),
        Expr::Reference(name) => {
            field(definitions.get(name.as_str())?, key, definitions, depth + 1)
        }
        Expr::Object(fields, extra) => fields
            .iter()
            .find(|field| field.name == key)
            .map(|field| field.expression.clone())
            .or_else(|| extra.as_ref().map(|expression| *expression.clone())),
        Expr::Intersection(members) => Some(Expr::Intersection(
            members
                .iter()
                .filter_map(|member| field(member, key, definitions, depth + 1))
                .collect(),
        )),
        _ => None,
    }
}
fn native_value(
    value: &Value,
    expression: &Expr,
    definitions: &BTreeMap<&str, &Expr>,
    depth: usize,
) -> Option<String> {
    if depth > 128 {
        return None;
    }
    match expression {
        Expr::At(_, expression) => native_value(value, expression, definitions, depth + 1),
        Expr::Reference(name) => native_value(
            value,
            definitions.get(name.as_str())?,
            definitions,
            depth + 1,
        ),
        Expr::Any => Some(any_value(value)),
        Expr::Never => None,
        Expr::Primitive(Primitive::Null) if value.is_null() => Some("null".into()),
        Expr::Primitive(Primitive::Boolean) if value.is_boolean() => Some(value.to_string()),
        Expr::Primitive(Primitive::String) if value.is_string() => Some(value.to_string()),
        Expr::Primitive(Primitive::SafeInteger) if value.is_number() => {
            crate::typescript::integral_token(&value.to_string())
        }
        Expr::Primitive(Primitive::Integer) if value.is_number() => {
            crate::typescript::integral_token(&value.to_string()).map(|value| format!("{value}n"))
        }
        Expr::Primitive(Primitive::Number | Primitive::AnyNumber) if value.is_number() => {
            Some(format!("JsonNumber.parse({})", q(&value.to_string())))
        }
        Expr::Literal(Literal::String(expected)) if value.as_str() == Some(expected) => {
            Some(q(expected))
        }
        Expr::Literal(Literal::Boolean(expected)) if value.as_bool() == Some(*expected) => {
            Some(expected.to_string())
        }
        Expr::Literal(Literal::Integer {
            value: expected,
            safe,
        }) if crate::typescript::integral_token(&value.to_string()).as_ref() == Some(expected) => {
            Some(format!("{expected}{}", if *safe { "" } else { "n" }))
        }
        Expr::Array(item) => Some(format!(
            "[{}]",
            value
                .as_array()?
                .iter()
                .map(|value| native_value(value, item, definitions, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        )),
        Expr::Object(fields, extra) => {
            let value = value.as_object()?;
            if fields
                .iter()
                .any(|field| field.required && !value.contains_key(&field.name))
            {
                return None;
            }
            let members = value
                .iter()
                .map(|(key, value)| {
                    let expression = fields
                        .iter()
                        .find(|field| field.name == *key)
                        .map(|field| &field.expression)
                        .or(extra.as_deref())?;
                    Some(format!(
                        "{}: {}",
                        self::key(key),
                        native_value(value, expression, definitions, depth + 1)?
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{{ {} }}", members.join(", ")))
        }
        Expr::Union(members) => members
            .iter()
            .find_map(|member| native_value(value, member, definitions, depth + 1)),
        Expr::Intersection(members) if value.is_object() => {
            let mut result = Vec::new();
            for (key, value) in value.as_object()? {
                let expression = Expr::Intersection(
                    members
                        .iter()
                        .filter_map(|member| field(member, key, definitions, depth + 1))
                        .collect(),
                );
                result.push(format!(
                    "{}: {}",
                    self::key(key),
                    native_value(value, &expression, definitions, depth + 1)?
                ));
            }
            Some(format!("{{ {} }}", result.join(", ")))
        }
        Expr::Intersection(members) => members
            .iter()
            .filter(|member| !matches!(member, Expr::Any))
            .find_map(|member| native_value(value, member, definitions, depth + 1))
            .or_else(|| Some(any_value(value))),
        _ => None,
    }
}
