//! Schema checks: discriminators and raw `type` values.

use rustc_hash::FxHashSet;
use suspect_low::{NodeRef, ValueKind};
use suspect_oas::{OpenApi, SchemaView};

use super::{diag_at, is_valid_type};
use crate::diagnostic::{Diagnostic, Severity};

/// Walks every schema reachable from `components/schemas` (through
/// properties, items, combinators) once each and runs the schema checks.
pub(crate) fn check_schemas(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let profile = std::env::var_os("SUSPECT_PROFILE").is_some();
    let t_all = profile.then(std::time::Instant::now);
    // Single shared visited set: every reachable schema node is resolved
    // and checked exactly once, in document order.
    let mut visited = FxHashSet::default();
    let mut dialects = super::schema_dialect::Contexts::default();
    for schema in api.schema_roots() {
        walk(schema, &mut visited, &mut dialects, api, out);
    }
    if let Some(t) = t_all {
        eprintln!(
            "[suspect-validate profile]   schemas total {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Depth-first walk over `schema`'s properties, `items`, `prefixItems`,
/// combinators, and `not`, running the per-schema checks on each node once
/// (document and byte range deduplicated so shared `$ref`s are checked once).
fn walk<'s>(
    schema: SchemaView<'s>,
    visited: &mut FxHashSet<(suspect_source::Uri, usize, usize)>,
    dialects: &mut super::schema_dialect::Contexts,
    api: &OpenApi<'_>,
    out: &mut Vec<Diagnostic>,
) {
    let mut pending = vec![(schema, false)];
    while let Some((schema, is_additional_properties)) = pending.pop() {
        let node = schema.node();
        let range = node.byte_range();
        if !visited.insert((node.syntax().doc().uri().clone(), range.start, range.end)) {
            continue;
        }
        if schema.is_missing_value() {
            out.push(diag_at(
                node,
                "oas-schema-invalid-kind",
                Severity::Error,
                range,
                "schema value is missing; use a Schema Object or an allowed boolean schema",
            ));
            continue;
        }
        let Some(is_31_plus) = dialects.resolve(schema, api, out) else {
            pending.extend(schema.subschemas().into_iter().map(|child| (child, false)));
            continue;
        };
        let allowed_boolean = is_31_plus || is_additional_properties;
        if node.kind() != ValueKind::Object && !(node.kind() == ValueKind::Bool && allowed_boolean)
        {
            out.push(diag_at(
                node,
                "oas-schema-invalid-kind",
                Severity::Error,
                range,
                if is_31_plus {
                    "schema must be an object or boolean; null is an instance value, not a schema"
                } else {
                    "OpenAPI 3.0 requires a Schema Object; boolean values are allowed only for additionalProperties"
                },
            ));
            continue;
        }
        if node.get("$ref").is_some() {
            if let Ok(Some(target)) = schema.reference_target() {
                pending.push((target, false));
            }
            // OAS 3.0 Reference Object siblings are ignored. In 3.1+ a
            // Schema Object's $ref is an applicator, alongside other keywords.
            if !is_31_plus {
                continue;
            }
        } else {
            check_discriminator(api, schema, out);
        }
        check_type(schema, is_31_plus, out);
        super::schema_keywords::check(schema.node(), is_31_plus, out);
        let additional_properties = node.get("additionalProperties");
        pending.extend(schema.subschemas().into_iter().map(|child| {
            let is_additional_properties = additional_properties.is_some_and(|value| {
                value.syntax().raw() == child.node().syntax().raw()
                    && value.syntax().doc().uri() == child.node().syntax().doc().uri()
            });
            (child, is_additional_properties)
        }));
    }
}

/// Validate the declared keyword, including absent YAML values, before
/// typed accessors can filter out invalid entries or collapse duplicates.
fn check_type(schema: SchemaView<'_>, is_31_plus: bool, out: &mut Vec<Diagnostic>) {
    let Some(entry) = schema.node().entries().into_iter().find(|entry| {
        entry
            .key_node
            .try_decoded_scalar()
            .is_some_and(|key| key.as_ref() == b"type")
    }) else {
        return;
    };
    let Some(t) = entry.value else {
        invalid_type(entry.key_node, "type requires a value", out);
        return;
    };
    match t.kind() {
        ValueKind::Str => {
            check_type_name(t, is_31_plus, out);
        }
        ValueKind::Array => {
            if !is_31_plus {
                invalid_type(t, "OpenAPI 3.0 type must be a single string", out);
                return;
            }
            let items = t.items();
            if items.is_empty() {
                invalid_type(t, "type array must contain at least one type", out);
            }
            let mut seen = FxHashSet::default();
            for item in items {
                if let Some(name) = check_type_name(item, is_31_plus, out)
                    && !seen.insert(name)
                {
                    invalid_type(item, "type array entries must be unique", out);
                }
            }
        }
        _ => invalid_type(
            t,
            "type must be a string or a nonempty array of unique strings",
            out,
        ),
    }
}

fn invalid_type(node: NodeRef<'_>, message: &str, out: &mut Vec<Diagnostic>) {
    out.push(diag_at(
        node,
        "oas-schema-invalid-type",
        Severity::Error,
        node.byte_range(),
        message,
    ));
}

fn check_type_name(
    node: NodeRef<'_>,
    is_31_plus: bool,
    out: &mut Vec<Diagnostic>,
) -> Option<String> {
    if node.kind() != ValueKind::Str {
        invalid_type(node, "type names must be strings", out);
        return None;
    }
    let Some(name) = node
        .try_decoded_scalar()
        .and_then(|bytes| String::from_utf8(bytes.into_owned()).ok())
    else {
        invalid_type(node, "type name is not a valid string", out);
        return None;
    };
    if !is_valid_type(&name) {
        out.push(diag_at(
            node,
            "oas-schema-unknown-type",
            Severity::Error,
            node.byte_range(),
            format!("unknown schema type `{name}`"),
        ));
    } else if name == "null" && !is_31_plus {
        invalid_type(
            node,
            "OpenAPI 3.0 does not support type null; use nullable with an explicit type",
            out,
        );
    }
    Some(name)
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
        out.push(diag_at(
                d.node(),
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
            out.push(diag_at(
                d.node(),
                "oas-discriminator-unknown-mapping",
                Severity::Error,
                range,
                format!("discriminator mapping key `{key}` points to missing schema `{name}`"),
            ));
        }
    }
}

/// A discriminator can be declared by a conjunct or by every alternative;
/// a union does not need to repeat its variants' property declarations.
fn property_declared(schema: &SchemaView<'_>, pn: &str, depth: usize) -> bool {
    if schema.required().contains(&pn) || schema.property(pn).is_some() {
        return true;
    }
    if depth == 0 {
        return false;
    }
    if schema
        .all_of()
        .iter()
        .any(|member| property_declared(&member.resolved(), pn, depth - 1))
    {
        return true;
    }
    [schema.one_of(), schema.any_of()]
        .into_iter()
        .any(|members| {
            !members.is_empty()
                && members.iter().all(|member| {
                    let member = member.resolved();
                    // A discriminator selects object alternatives; it must not
                    // remove an explicitly allowed null value from the union.
                    member
                        .type_()
                        .is_some_and(|t| t.bits() == suspect_oas::TypeSet::NULL)
                        || property_declared(&member, pn, depth - 1)
                })
        })
}
