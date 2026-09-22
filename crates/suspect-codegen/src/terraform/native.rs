//! Bind to retained Go lowering descriptors; never inspect emitted Go or
//! reinterpret OpenAPI schemas. Unsupported representations fail in planning.
use std::collections::BTreeSet;

use crate::{
    go_http::HttpPlan,
    go_models::{GoDecl, GoDescriptors, GoField, GoType, Key},
    rust_models::RepresentationRole,
};
use suspect_ir::contract::{SchemaId, SourceId};

use super::AttributeType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Presence {
    Required,
    Optional,
    Nullable,
    OptionalNullable,
}

#[derive(Debug, Clone)]
pub(super) struct Scalar {
    pub kind: AttributeType,
    /// Fully qualified type, including allocated aliases/literal types.
    pub value_type: String,
    pub presence: Presence,
}

#[derive(Debug, Clone)]
pub(super) struct Field {
    pub name: String,
    pub source: SourceId,
    pub scalar: Scalar,
}

pub(super) fn table(plan: &HttpPlan) -> &GoDescriptors {
    plan.codecs().models().descriptors()
}

pub(super) fn record<'a>(
    table: &'a GoDescriptors,
    schema: &SchemaId,
) -> Result<&'a [GoField], String> {
    let mut key = (schema.clone(), RepresentationRole::Model);
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(key.clone()) {
            return Err("cyclic native alias is outside lifecycle v1".into());
        }
        match table.declarations.get(&key) {
            Some(GoDecl::Alias(GoType::Named(next))) => key = next.clone(),
            Some(GoDecl::Struct {
                fields,
                extras: None,
            }) => return Ok(fields),
            _ => return Err("lifecycle v1 needs a closed, non-null native Go record".into()),
        }
    }
}

pub(super) fn field(
    table: &GoDescriptors,
    schema: &SchemaId,
    path: &[String],
) -> Result<Field, String> {
    if path.len() != 1 {
        return Err("lifecycle v1 supports one-level native record property paths".into());
    }
    let field = record(table, schema)?
        .iter()
        .find(|f| f.wire == path[0])
        .ok_or_else(|| "path does not bind an allocated native Go record field".to_owned())?;
    Ok(Field {
        name: field.name.clone(),
        source: field.source.clone(),
        scalar: scalar(table, &field.ty)?,
    })
}

pub(super) fn parameter(
    table: &GoDescriptors,
    schema: &SchemaId,
    required: bool,
) -> Result<Scalar, String> {
    let native = GoType::Named((schema.clone(), RepresentationRole::Model));
    let ty = if required {
        native
    } else {
        GoType::Optional(Box::new(native))
    };
    scalar(table, &ty)
}

fn scalar(table: &GoDescriptors, ty: &GoType) -> Result<Scalar, String> {
    // Model fields have wrappers outside named aliases. Parameters can have a
    // named nullable alias; preserve its emitted wrapper and concrete value type.
    let mut optional = false;
    let mut nullable = false;
    let mut current = ty;
    let mut value_type = None;
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    let kind = loop {
        match current {
            GoType::Optional(inner) if !optional => {
                optional = true;
                current = inner;
            }
            GoType::Nullable(inner) if !nullable => {
                nullable = true;
                value_type = None;
                current = inner;
            }
            GoType::Presence(inner) if !optional && !nullable => {
                optional = true;
                nullable = true;
                current = inner;
            }
            GoType::Named(key) if seen.insert(key.clone()) => {
                value_type.get_or_insert_with(|| format!("sdk.{}", table.names[key]));
                match table.declarations.get(key) {
                    Some(GoDecl::Alias(next)) => current = next,
                    Some(GoDecl::Literals { underlying, .. }) => {
                        break match *underlying {
                            "string" => AttributeType::String,
                            "bool" => AttributeType::Bool,
                            _ => return Err("unsupported native literal representation".into()),
                        };
                    }
                    _ => return Err("lifecycle v1 maps only native string/bool scalars".into()),
                }
            }
            GoType::Primitive("string") => break AttributeType::String,
            GoType::Primitive("bool") => break AttributeType::Bool,
            _ => return Err("lifecycle v1 maps only native string/bool scalar carriers".into()),
        }
    };
    // Optional[Nullable[T]] is not the same struct as Presence[T]. Refuse this
    // parameter shape instead of emitting an invented wrapper conversion.
    if matches!(ty, GoType::Optional(_)) && nullable {
        return Err("optional nullable operation parameters are outside lifecycle v1".into());
    }
    Ok(Scalar {
        kind,
        value_type: value_type.unwrap_or_else(|| kind.hcl().into()),
        presence: match (optional, nullable) {
            (false, false) => Presence::Required,
            (true, false) => Presence::Optional,
            (false, true) => Presence::Nullable,
            (true, true) => Presence::OptionalNullable,
        },
    })
}
