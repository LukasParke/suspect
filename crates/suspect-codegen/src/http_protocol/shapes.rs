use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::*;

/// Follow only indexed Schema Object references. The codec retains the original
/// root (and therefore assertion siblings); a shape proof never rewrites it.
pub(super) fn object<'a>(
    p: &mut Planner<'a>,
    schema: &SchemaUse,
) -> Option<(SourceId, &'a Map<String, Value>)> {
    let mut current = schema.id.clone();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.clone()) {
            p.unsupported(
                &current,
                "http-wire-shape-cycle",
                "a reference cycle does not establish a finite wire shape",
            );
            return None;
        }
        let Some(view) = p.contract.schema(&current) else {
            p.unsupported(
                &current,
                "http-schema-unindexed",
                "referenced wire schema is absent from the canonical schema index",
            );
            return None;
        };
        let Some(raw) = view.raw().as_object() else {
            p.unsupported(
                &current,
                "http-wire-shape-unknown",
                "boolean schema does not determine a scalar/array/object wire representation",
            );
            return None;
        };
        if !view.ignores_ref_siblings() && raw.contains_key("$dynamicRef") {
            p.unsupported(&current.child("$dynamicRef"), "http-dynamic-wire-shape", "dynamic reference bindings do not establish a single non-JSON wire shape; their initial target is not a static substitute");
            return None;
        }
        if raw.contains_key("$ref") {
            // A mixed multipart aggregate/binary value will not pass through a
            // JSON codec. Dropping even a required/maxLength assertion sibling
            // here would lose a wire obligation. 3.0 Reference Object siblings
            // are ignored by that dialect; 3.1+ assertions need a combined proof.
            for field in raw.keys() {
                if !p.is_30(&current)
                    && !matches!(
                        field.as_str(),
                        "$ref"
                            | "$schema"
                            | "$comment"
                            | "description"
                            | "summary"
                            | "title"
                            | "deprecated"
                            | "example"
                            | "examples"
                            | "default"
                            | "readOnly"
                            | "writeOnly"
                    )
                    && !field.starts_with("x-")
                    && !p.resource_annotation(&current, field)
                {
                    p.unsupported(&current.child(field), "http-wire-reference-siblings", "non-JSON wire shape with assertion $ref siblings requires an intersection proof");
                    return None;
                }
            }
            let Some(target) = view
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.as_ref())
            else {
                p.error(
                    &current.child("$ref"),
                    "http-schema-reference",
                    "wire schema reference has no indexed target",
                );
                return None;
            };
            current = target.clone();
        } else {
            return Some((current, raw));
        }
    }
}

pub(super) fn scalar(p: &mut Planner<'_>, schema: &SchemaUse) -> Option<ScalarType> {
    let (at, raw) = object(p, schema)?;
    non_null(p, &at, raw)?;
    match type_name(raw) {
        Some("string") => Some(ScalarType::String),
        Some("boolean") => Some(ScalarType::Boolean),
        Some("integer") => Some(ScalarType::Integer),
        Some("number") => Some(ScalarType::Number),
        _ => {
            p.unsupported(
                &at,
                "http-scalar-shape-unsupported",
                "wire value requires one non-null string, boolean, integer or number type",
            );
            None
        }
    }
}

pub(super) fn shape(p: &mut Planner<'_>, schema: &SchemaUse) -> Option<WireShape> {
    let (at, raw) = object(p, schema)?;
    non_null(p, &at, raw)?;
    match type_name(raw) {
        Some("string" | "boolean" | "integer" | "number") => {
            scalar(p, schema).map(|scalar| WireShape::Scalar { scalar })
        }
        Some("array") => {
            if raw.contains_key("prefixItems") {
                p.unsupported(&at.child("prefixItems"), "http-array-shape-unsupported", "parameter serialization requires homogeneous scalar items, not positional tuples");
                return None;
            }
            let items = p.schema(&at.child("items"))?;
            scalar(p, &items).map(|items| WireShape::Array { items })
        }
        Some("object") => {
            let mut properties = BTreeMap::new();
            if let Some(value) = raw.get("properties") {
                let Some(map) = value.as_object() else {
                    p.error(
                        &at.child("properties"),
                        "http-schema-properties",
                        "properties must be a map of schemas",
                    );
                    return None;
                };
                for name in map.keys() {
                    let property = p.schema(&at.child("properties").child(name))?;
                    properties.insert(name.clone(), scalar(p, &property)?);
                }
            }
            if raw.contains_key("patternProperties") || raw.contains_key("unevaluatedProperties") {
                let key = if raw.contains_key("patternProperties") {
                    "patternProperties"
                } else {
                    "unevaluatedProperties"
                };
                p.unsupported(
                    &at.child(key),
                    "http-object-shape-unsupported",
                    "pattern/evaluated-property wire shapes require a separate flat-value proof",
                );
                return None;
            }
            let additional = match raw.get("additionalProperties") {
                Some(Value::Bool(false)) => AdditionalScalars::Forbidden,
                None | Some(Value::Bool(true)) => AdditionalScalars::AnyScalar,
                Some(Value::Object(_)) => {
                    let schema = p.schema(&at.child("additionalProperties"))?;
                    AdditionalScalars::Typed(scalar(p, &schema)?)
                }
                Some(_) => {
                    p.error(
                        &at.child("additionalProperties"),
                        "http-schema-additional-properties",
                        "additionalProperties must be a schema or boolean",
                    );
                    return None;
                }
            };
            Some(WireShape::FlatObject {
                properties,
                additional,
            })
        }
        _ => {
            p.unsupported(&at, "http-parameter-shape-unsupported", "parameter style requires a scalar, a scalar array, or a flat scalar object; nullable/untyped/composed representations need an explicit wire policy");
            None
        }
    }
}

fn non_null(p: &mut Planner<'_>, at: &SourceId, raw: &Map<String, Value>) -> Option<()> {
    if p.is_30(at) && raw.get("nullable").is_some_and(|v| !v.is_boolean()) {
        p.error(
            &at.child("nullable"),
            "http-schema-nullable",
            "nullable must be boolean in OAS 3.0",
        );
        return None;
    }
    if p.is_30(at) && raw.get("nullable").and_then(Value::as_bool) == Some(true) {
        p.unsupported(
            &at.child("nullable"),
            "http-null-wire-policy",
            "nullable scalar/form values need an explicit null serialization policy",
        );
        return None;
    }
    Some(())
}

pub(super) fn type_name(raw: &Map<String, Value>) -> Option<&str> {
    match raw.get("type") {
        Some(Value::String(s)) => Some(s),
        Some(Value::Array(types)) if types.len() == 1 => types[0].as_str(),
        _ => None,
    }
}
