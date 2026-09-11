//! Source-validated native constructor expressions retained before rendering.
use super::{
    HttpDiagnostic, diagnostic,
    emit::quote,
    models::{CsDecl, CsType, Key, ModelPlan, key},
};
use serde_json::Value;
use std::collections::BTreeMap;
use suspect_ir::contract::SourceId;
use suspect_schema::{OwnedOutcome, OwnedSchema};

#[derive(Debug)]
pub(super) enum Expression {
    Null,
    NullOnly,
    String(String),
    Boolean(bool),
    Number(String, bool),
    Json(Value),
    Literal(Key, String),
    Union(Key, String, Box<Expression>),
    Object(
        Key,
        Vec<(String, CsType, bool, Expression)>,
        Option<(CsType, Vec<(String, Expression)>)>,
    ),
    List(CsType, Vec<Expression>),
    Map(CsType, Vec<(String, Expression)>),
}
pub(super) type Samples = BTreeMap<(SourceId, usize), Expression>;

pub(super) fn plan(
    models: &ModelPlan,
    examples: &crate::examples::ExamplePlan,
    compiled: &OwnedSchema,
) -> Result<Samples, Vec<HttpDiagnostic>> {
    let mut result = BTreeMap::new();
    let mut errors = Vec::new();
    for operation in examples.operations() {
        for (index, entry) in operation.entries.iter().enumerate() {
            match lower(
                models,
                compiled,
                &CsType::Named(key(&entry.schema)),
                &entry.value,
                0,
            ) {
                Ok(expression) => {
                    result.insert((operation.source.clone(), index), expression);
                }
                Err(message) => errors.push(diagnostic(
                    examples.contract(),
                    entry.schema.clone(),
                    "csharp-native-example-unavailable",
                    message,
                )),
            }
        }
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}

fn lower(
    models: &ModelPlan,
    compiled: &OwnedSchema,
    ty: &CsType,
    value: &Value,
    depth: usize,
) -> Result<Expression, &'static str> {
    if depth >= 128 {
        return Err("native example construction exceeded its finite depth budget");
    }
    let child = |ty: &CsType, value: &Value| lower(models, compiled, ty, value, depth + 1);
    Ok(match ty {
        CsType::Native("string") => Expression::String(
            value
                .as_str()
                .ok_or("source example is not a native string")?
                .into(),
        ),
        CsType::Native("bool") => Expression::Boolean(
            value
                .as_bool()
                .ok_or("source example is not a native boolean")?,
        ),
        CsType::Native("JsonNull") => Expression::NullOnly,
        CsType::Native(_) => return Err("an uninhabited native type has no executable example"),
        CsType::Number | CsType::Integer => {
            Expression::Number(value.to_string(), matches!(ty, CsType::Integer))
        }
        CsType::Json => Expression::Json(value.clone()),
        CsType::Nullable(inner) => {
            if value.is_null() {
                Expression::Null
            } else {
                child(inner, value)?
            }
        }
        CsType::List(inner) => Expression::List(
            (**inner).clone(),
            value
                .as_array()
                .ok_or("source example is not an array")?
                .iter()
                .map(|v| child(inner, v))
                .collect::<Result<_, _>>()?,
        ),
        CsType::Dict(inner) => Expression::Map(
            (**inner).clone(),
            value
                .as_object()
                .ok_or("source example is not an object")?
                .iter()
                .map(|(name, v)| Ok::<_, &'static str>((name.clone(), child(inner, v)?)))
                .collect::<Result<_, _>>()?,
        ),
        CsType::Named(key) => match &models.declarations[key] {
            CsDecl::Alias(inner) => child(inner, value)?,
            _ if value.is_null() && models.is_nullable(&key.0) == Some(true) => Expression::Null,
            CsDecl::Literals { values } => {
                let member = values
                    .iter()
                    .find(|(_, token)| {
                        serde_json::from_str::<Value>(token).ok().as_ref() == Some(value)
                    })
                    .ok_or("source example has no native literal member")?;
                Expression::Literal(key.clone(), member.0.clone())
            }
            CsDecl::Union { branches } => {
                let mut selected = None;
                for branch in branches {
                    match compiled.validate(&branch.source, value) {
                        OwnedOutcome::Valid => {
                            selected = Some(branch);
                            break;
                        }
                        OwnedOutcome::Invalid(_) => {}
                        OwnedOutcome::EvaluationFailure(_) => {
                            return Err("native example union branch proof did not complete");
                        }
                    }
                }
                let branch = selected.ok_or("source example has no native union branch")?;
                Expression::Union(
                    key.clone(),
                    branch.name.clone(),
                    Box::new(child(&branch.ty, value)?),
                )
            }
            CsDecl::Record { fields, extras } => {
                let object = value
                    .as_object()
                    .ok_or("source example is not a native record")?;
                let mut properties = Vec::new();
                for field in fields {
                    if let Some(value) = object.get(&field.wire) {
                        properties.push((
                            field.name.clone(),
                            field.ty.clone(),
                            field.required,
                            child(&field.ty, value)?,
                        ));
                    }
                }
                let extra = extras
                    .as_ref()
                    .map(|ty| {
                        let values = object
                            .iter()
                            .filter(|(name, _)| !fields.iter().any(|f| &f.wire == *name))
                            .map(|(name, value)| {
                                Ok::<_, &'static str>((name.clone(), child(ty, value)?))
                            })
                            .collect::<Result<_, _>>()?;
                        Ok::<_, &'static str>((ty.clone(), values))
                    })
                    .transpose()?;
                Expression::Object(key.clone(), properties, extra)
            }
        },
    })
}

pub(super) fn render(expression: &Expression, models: &ModelPlan) -> String {
    let child = |value: &Expression| render(value, models);
    let ty = |ty: &CsType| models.render_type(ty);
    let name = |key: &Key| models.names[key].clone();
    let map = |item: &CsType, values: &[(String, Expression)]| {
        format!(
            "new global::System.Collections.Generic.Dictionary<string, {}> {{ {} }}",
            ty(item),
            values
                .iter()
                .map(|(key, value)| format!("[{}] = {}", quote(key), child(value)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    match expression {
        Expression::Null => "null".into(),
        Expression::NullOnly => "default(JsonNull)".into(),
        Expression::String(value) => quote(value),
        Expression::Boolean(value) => value.to_string(),
        Expression::Number(token, integral) => format!(
            "new {}({})",
            if *integral {
                "JsonInteger"
            } else {
                "JsonNumber"
            },
            quote(token)
        ),
        Expression::Json(value) => format!("Quickstart.Json({})", quote(&value.to_string())),
        Expression::Literal(key, member) => format!("{}.{member}", name(key)),
        Expression::Union(key, branch, value) => {
            format!("new {}.{branch}({})", name(key), child(value))
        }
        Expression::Object(key, fields, extras) => {
            let mut properties = fields
                .iter()
                .map(|(name, field_type, required, value)| {
                    format!(
                        "{name} = {}",
                        if !required && matches!(value, Expression::Null) {
                            format!("Optional<{}>.Present(null)", ty(field_type))
                        } else {
                            child(value)
                        }
                    )
                })
                .collect::<Vec<_>>();
            if let Some((item, values)) = extras
                && !values.is_empty()
            {
                properties.push(format!("Extra = {}", map(item, values)));
            }
            format!("new {} {{ {} }}", name(key), properties.join(", "))
        }
        Expression::List(item, values) => format!(
            "new global::System.Collections.Generic.List<{}> {{ {} }}",
            ty(item),
            values.iter().map(child).collect::<Vec<_>>().join(", ")
        ),
        Expression::Map(item, values) => map(item, values),
    }
}
