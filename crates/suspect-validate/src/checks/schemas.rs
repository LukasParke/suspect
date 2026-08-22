//! Schema checks: discriminators and raw `type` values.

use rustc_hash::FxHashSet;
use suspect_oas::{OpenApi, SchemaView};

use super::{diag, is_valid_type};
use crate::diagnostic::{Diagnostic, Severity};

/// Walks every schema reachable from `components/schemas` (through
/// properties, items, combinators) once each and runs the schema checks.
pub(crate) fn check_schemas(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let Some(components) = api.components() else {
        return;
    };
    let mut visited = FxHashSet::default();
    for (_, schema) in components.schemas() {
        walk(schema, &mut visited, api, out);
    }
}

/// Depth-first walk over `schema`'s properties, `items`, `prefixItems`,
/// combinators, and `not`, running the per-schema checks on each node once
/// (byte range deduplicated so shared `$ref`s are only checked a single
/// time).
fn walk<'s>(
    schema: SchemaView<'s>,
    visited: &mut FxHashSet<(usize, usize)>,
    api: &OpenApi<'_>,
    out: &mut Vec<Diagnostic>,
) {
    let r = schema.resolved();
    let range = r.node().byte_range();
    if !visited.insert((range.start, range.end)) {
        return;
    }
    check_unknown_type(api, r, out);
    check_discriminator(api, r, out);

    for (_, prop) in r.properties() {
        walk(prop, visited, api, out);
    }
    if let Some(items) = r.items() {
        walk(items, visited, api, out);
    }
    for sub in r.prefix_items() {
        walk(sub, visited, api, out);
    }
    for sub in r.all_of().into_iter().chain(r.any_of()).chain(r.one_of()) {
        walk(sub, visited, api, out);
    }
    if let Some(not) = r.not() {
        walk(not, visited, api, out);
    }
}

/// `oas-schema-unknown-type` (Error): the raw `type` value must be one of the
/// JSON-Schema primitives; array form checked item by item.
fn check_unknown_type(api: &OpenApi<'_>, schema: SchemaView<'_>, out: &mut Vec<Diagnostic>) {
    use suspect_low::ValueKind;

    let Some(t) = schema.node().get("type") else {
        return;
    };
    match t.kind() {
        ValueKind::Str => {
            if let Some(s) = t.as_str()
                && !is_valid_type(s)
            {
                out.push(diag(
                    api,
                    "oas-schema-unknown-type",
                    Severity::Error,
                    t.byte_range(),
                    format!("unknown schema type `{s}`"),
                ));
            }
        }
        ValueKind::Array => {
            for item in t.items() {
                if let Some(s) = item.as_str()
                    && !is_valid_type(s)
                {
                    out.push(diag(
                        api,
                        "oas-schema-unknown-type",
                        Severity::Error,
                        item.byte_range(),
                        format!("unknown schema type `{s}` in type array"),
                    ));
                }
            }
        }
        _ => {}
    }
}

/// `oas-discriminator-missing-property` (Error) and
/// `oas-discriminator-unknown-mapping` (Error).
fn check_discriminator(api: &OpenApi<'_>, schema: SchemaView<'_>, out: &mut Vec<Diagnostic>) {
    let Some(d) = schema.discriminator() else {
        return;
    };

    if let Some(pn) = d.property_name()
        && !property_declared(&schema, pn, 8)
    {
        out.push(diag(
                api,
                "oas-discriminator-missing-property",
                Severity::Error,
                d.node().byte_range(),
                format!(
                    "discriminator propertyName `{pn}` is neither required nor a declared property of the schema"
                ),
            ));
    }

    for (key, target) in d.mapping() {
        let Some(name) = target.strip_prefix("#/components/schemas/") else {
            continue;
        };
        let exists = api.components().is_some_and(|c| c.schema(name).is_some());
        if !exists {
            let range = d
                .node()
                .get("mapping")
                .and_then(|m| m.get(key))
                .map(|n| n.byte_range())
                .unwrap_or_else(|| d.node().byte_range());
            out.push(diag(
                api,
                "oas-discriminator-unknown-mapping",
                Severity::Error,
                range,
                format!("discriminator mapping key `{key}` points to missing schema `{name}`"),
            ));
        }
    }
}

/// True when `pn` is required or declared as a property on this schema or,
/// for the common allOf-inheritance pattern, on any allOf member.
fn property_declared(schema: &SchemaView<'_>, pn: &str, depth: usize) -> bool {
    if schema.required().contains(&pn) || schema.property(pn).is_some() {
        return true;
    }
    if depth == 0 {
        return false;
    }
    schema
        .all_of()
        .iter()
        .any(|member| property_declared(&member.resolved(), pn, depth - 1))
}
