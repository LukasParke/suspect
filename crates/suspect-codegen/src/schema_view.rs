//! Shared source-preserving dialect views for native model/codec planning.
//! Native backends share `crate::schema_view`. OwnedCompiler remains
//! the full schema-admission gate; these helpers do not invent schema identities.

use serde_json::Value;
use std::{borrow::Cow, collections::BTreeSet};
use suspect_ir::contract::{Contract, ContractDiagnostic, Schema, SchemaDialect, SchemaId};

/// Versioned dialect interpretation choices shared by model/codec planners.
/// The zero value preserves the ordinary strict dialect semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DialectPolicy {
    /// Interpret the OAS 3.0 `nullable` annotation on OAS 3.1/3.2 schemas with
    /// its 3.0 semantics: the same-object type gains (or, with
    /// `nullable: false`, loses) null. Mirrors the
    /// `CompatibilityProfile::Oas30NullableIn31V1` generation option.
    pub oas30_nullable_in_31: bool,
}

impl DialectPolicy {
    /// The policy implied by the adapter's explicit compatibility profiles.
    pub(crate) fn from_profiles<I>(profiles: I) -> Self
    where
        I: IntoIterator<Item = crate::http_protocol::CompatibilityProfile>,
    {
        let mut policy = Self::default();
        for profile in profiles {
            if profile == crate::http_protocol::CompatibilityProfile::Oas30NullableIn31V1 {
                policy.oas30_nullable_in_31 = true;
            }
        }
        policy
    }
}

pub(crate) fn reference_only(schema: Schema<'_>) -> bool {
    schema.ignores_ref_siblings()
}

/// Borrow normal schemas; expose only the original ref value on a 3.0 Reference
/// Object. Contract and source IDs are unchanged, and no target is inlined.
pub(crate) fn raw(schema: Schema<'_>) -> Cow<'_, Value> {
    if reference_only(schema) {
        Cow::Owned(Value::Object(
            [("$ref".into(), schema.raw()["$ref"].clone())]
                .into_iter()
                .collect(),
        ))
    } else {
        Cow::Borrowed(schema.raw())
    }
}

pub(crate) fn description(schema: Schema<'_>) -> &str {
    if reference_only(schema) {
        ""
    } else {
        schema
            .raw()
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

pub(crate) fn closure(contract: &Contract, roots: &[SchemaId]) -> Vec<SchemaId> {
    contract.effective_schema_closure(roots)
}

pub(crate) fn has_intersections(contract: &Contract, reachable: &[SchemaId]) -> bool {
    reachable
        .iter()
        .filter_map(|id| contract.schema(id))
        .any(|schema| raw(schema).get("allOf").is_some())
}

pub(crate) fn diagnostic_applies(
    contract: &Contract,
    reachable: &[SchemaId],
    diagnostic: &ContractDiagnostic,
) -> bool {
    contract.schema_diagnostic_applies(reachable, diagnostic)
}

pub(crate) fn null_allowed(
    contract: &Contract,
    root: &SchemaId,
    policy: DialectPolicy,
) -> Result<bool, Problem> {
    fn visit(
        contract: &Contract,
        id: &SchemaId,
        active: &mut BTreeSet<SchemaId>,
        work: &mut usize,
        policy: DialectPolicy,
    ) -> Option<bool> {
        *work = work.checked_sub(1)?;
        if active.len() >= 256 || !active.insert(id.clone()) {
            return None;
        }
        let result = (|| {
            let schema = contract.schema(id)?;
            let value = raw(schema);
            if let Some(value) = value.as_bool() {
                return Some(value);
            }
            let object = value.as_object()?;
            if object.contains_key("$dynamicRef")
                || object.contains_key("$recursiveRef")
                || object.contains_key("if")
                    && (object.contains_key("then") || object.contains_key("else"))
            {
                return None;
            }
            let mut accepts = accepts_literal(schema, &Value::Null, policy);
            if let Some(value) = object.get("const") {
                accepts &= value.is_null();
            }
            if let Some(values) = object.get("enum").and_then(Value::as_array) {
                accepts &= values.iter().any(Value::is_null);
            }
            for reference in schema.references() {
                accepts &= visit(contract, reference.target.as_ref()?, active, work, policy)?;
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                if let Some(values) = object.get(keyword).and_then(Value::as_array) {
                    let mut matched = 0;
                    for index in 0..values.len() {
                        matched += usize::from(visit(
                            contract,
                            &id.child(keyword).child(&index.to_string()),
                            active,
                            work,
                            policy,
                        )?);
                    }
                    accepts &= match keyword {
                        "allOf" => matched == values.len(),
                        "anyOf" => matched != 0,
                        _ => matched == 1,
                    };
                }
            }
            if object.contains_key("not") {
                accepts &= !visit(contract, &id.child("not"), active, work, policy)?;
            }
            Some(accepts)
        })();
        active.remove(id);
        result
    }
    visit(contract, root, &mut BTreeSet::new(), &mut 100_000, policy).ok_or_else(|| Problem {
        source:root.clone(), code:"native-nullability-analysis", message:"nullability proof is incomplete (unsupported applicability or finite reference/depth/work limits); incomplete analysis cannot become a non-null type",
    })
}

/// Local type intersection only; nullable cannot bypass enum or composition.
pub(crate) fn accepts_literal(schema: Schema<'_>, value: &Value, policy: DialectPolicy) -> bool {
    if reference_only(schema) {
        return true;
    }
    let raw = schema.raw();
    if value.is_null() && raw.get("nullable").is_some() {
        let declared_nullable = raw.get("nullable") == Some(&Value::Bool(true));
        let mutates_type =
            matches!(schema.dialect(), SchemaDialect::OpenApi30) || policy.oas30_nullable_in_31;
        if mutates_type && raw.get("type").is_some_and(Value::is_string) {
            return declared_nullable;
        }
        if mutates_type && declared_nullable && raw.get("type").is_some_and(Value::is_array) {
            return true;
        }
        if mutates_type && raw.get("nullable") == Some(&Value::Bool(false)) {
            // nullable: false removes "null" from a declared type array.
            return !raw.get("type").is_some_and(Value::is_array);
        }
    }
    let Some(types) = raw.get("type") else {
        return true;
    };
    let matches = |name: &str| match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => value.as_number().is_some_and(|number| {
            number
                .as_str()
                .parse::<crate::rust_models::runtime::JsonInteger>()
                .is_ok()
        }),
        _ => false,
    };
    types.as_str().is_some_and(matches)
        || types
            .as_array()
            .is_some_and(|types| types.iter().filter_map(Value::as_str).any(matches))
}

/// Exact sufficient bounded-integer proof. Fractional/out-of-i128 operands
/// return None: retain the lossless integer carrier, never round a native range.
/// Callers must already be representing mathematical integers (a declared type
/// or integer literal); bounds alone never establish the schema's instance type.
pub(crate) fn integer_interval(schema: Schema<'_>) -> Option<(i128, i128)> {
    if reference_only(schema) {
        return None;
    }
    let object = schema.raw().as_object()?;
    let old = matches!(schema.dialect(), SchemaDialect::OpenApi30);
    let mut lower: Option<i128> = None;
    let mut upper: Option<i128> = None;
    for (key, maximum, exclusive) in [
        ("minimum", false, false),
        ("maximum", true, false),
        ("exclusiveMinimum", false, true),
        ("exclusiveMaximum", true, true),
    ] {
        let Some(value) = object.get(key) else {
            continue;
        };
        if old && exclusive {
            continue;
        }
        let value = value
            .as_number()?
            .as_str()
            .parse::<crate::rust_models::runtime::JsonInteger>()
            .ok()?
            .to_i128()?;
        let exclusive = exclusive
            || old
                && object.get(if maximum {
                    "exclusiveMaximum"
                } else {
                    "exclusiveMinimum"
                }) == Some(&Value::Bool(true));
        let value = if exclusive {
            if maximum {
                value.checked_sub(1)?
            } else {
                value.checked_add(1)?
            }
        } else {
            value
        };
        if maximum {
            upper = Some(upper.map_or(value, |previous| previous.min(value)));
        } else {
            lower = Some(lower.map_or(value, |previous| previous.max(value)));
        }
    }
    let bounds = (lower?, upper?);
    (bounds.0 <= bounds.1).then_some(bounds)
}

pub(crate) struct Problem {
    pub source: SchemaId,
    pub code: &'static str,
    pub message: &'static str,
}

/// Model-only APIs cannot advertise unconditional 3.0 directional requirements.
pub(crate) fn problems(contract: &Contract, reachable: &[SchemaId]) -> Vec<Problem> {
    let mut out = Vec::new();
    for id in reachable {
        let Some(schema) = contract.schema(id) else {
            continue;
        };
        if !matches!(schema.dialect(), SchemaDialect::OpenApi30) || reference_only(schema) {
            continue;
        }
        if schema
            .raw()
            .get("type")
            .is_some_and(|ty| !ty.is_string() || ty == "null")
        {
            out.push(Problem { source:id.child("type"), code:"invalid-oas30-type", message:"OpenAPI 3.0 requires one non-null type name; nullable only modifies a same-object type" });
        }
        let mut directional = BTreeSet::new();
        let mut required = Vec::new();
        for at in in_place(contract, id) {
            let Some(schema) = contract.schema(&at) else {
                continue;
            };
            let value = raw(schema);
            if let Some(names) = value.get("required").and_then(Value::as_array) {
                required.extend(
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|name| (at.child("required"), name.to_owned())),
                );
            }
            if let Some(fields) = value.get("properties").and_then(Value::as_object) {
                for name in fields.keys() {
                    if in_place(contract, &at.child("properties").child(name))
                        .iter()
                        .any(|child| {
                            contract.schema(child).is_some_and(|schema| {
                                let value = raw(schema);
                                value.get("readOnly") == Some(&Value::Bool(true))
                                    || value.get("writeOnly") == Some(&Value::Bool(true))
                            })
                        })
                    {
                        directional.insert(name.to_owned());
                    }
                }
            }
        }
        for (source, name) in required {
            if directional.contains(&name) {
                out.push(Problem { source, code:"oas30-directional-required-unsupported", message:"OpenAPI 3.0 directional required needs an explicit supported request/response validation profile; neutral model/codec planning cannot make it unconditional or drop it" });
            }
        }
    }
    out.sort_by(|a, b| (&a.source, a.code).cmp(&(&b.source, b.code)));
    out.dedup_by(|a, b| a.source == b.source && a.code == b.code);
    out
}

fn in_place(contract: &Contract, root: &SchemaId) -> Vec<SchemaId> {
    let mut pending = vec![root.clone()];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    while let Some(id) = pending.pop() {
        let Some(schema) = contract.schema(&id) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        pending.extend(
            schema
                .references()
                .iter()
                .filter_map(|reference| reference.target.clone()),
        );
        if reference_only(schema) {
            continue;
        }
        for keyword in ["allOf", "anyOf", "oneOf", "not"] {
            if let Some(value) = schema.raw().get(keyword) {
                if let Some(values) = value.as_array() {
                    pending.extend(
                        (0..values.len()).map(|index| id.child(keyword).child(&index.to_string())),
                    );
                } else {
                    pending.push(id.child(keyword));
                }
            }
        }
        out.push(id);
    }
    out
}
