//! Validate declarations without trying to prove a schema satisfiable.

use std::cmp::Ordering;
use std::collections::HashSet;

use suspect_low::{NodeRef, ValueKind};

use super::diag_at;
use crate::{Diagnostic, Severity};

pub(super) fn check(schema: NodeRef<'_>, is_31_plus: bool, out: &mut Vec<Diagnostic>) {
    if !is_31_plus
        && schema
            .get("readOnly")
            .is_some_and(|v| v.as_bool() == Some(true))
        && let Some(write) = schema
            .get("writeOnly")
            .filter(|v| v.as_bool() == Some(true))
    {
        invalid(
            write,
            "writeOnly",
            "cannot be true together with readOnly in OpenAPI 3.0",
            out,
        );
    }
    for entry in schema.entries() {
        let Some(bytes) = entry.key_node.try_decoded_scalar() else {
            continue;
        };
        let Ok(keyword) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let at = entry.value.unwrap_or(entry.key_node);
        if !is_31_plus && unsupported_in_30(keyword) {
            invalid(
                at,
                keyword,
                "is outside the OpenAPI 3.0 Schema Object vocabulary",
                out,
            );
            continue;
        }
        match keyword {
            "type"
                if !is_31_plus
                    && entry.value.is_some_and(|v| {
                        v.kind() == ValueKind::Str
                            && v.try_decoded_scalar()
                                .is_some_and(|s| s.as_ref() == b"array")
                    })
                    && !schema.entries().iter().any(|e| {
                        e.key_node
                            .try_decoded_scalar()
                            .is_some_and(|s| s.as_ref() == b"items")
                    }) =>
            {
                invalid(
                    at,
                    "items",
                    "is required when type is array in OpenAPI 3.0",
                    out,
                )
            }
            "nullable" if !is_31_plus => kind(
                keyword,
                entry.value,
                at,
                ValueKind::Bool,
                "must be a boolean in OpenAPI 3.0",
                out,
            ),
            "title" | "description" | "format" | "pattern" => kind(
                keyword,
                entry.value,
                at,
                ValueKind::Str,
                "must be a string",
                out,
            ),
            "contentEncoding" | "contentMediaType" if is_31_plus => kind(
                keyword,
                entry.value,
                at,
                ValueKind::Str,
                "must be a string",
                out,
            ),
            "uniqueItems" | "readOnly" | "writeOnly" | "deprecated" => kind(
                keyword,
                entry.value,
                at,
                ValueKind::Bool,
                "must be a boolean",
                out,
            ),
            "examples" if is_31_plus => kind(
                keyword,
                entry.value,
                at,
                ValueKind::Array,
                "must be an array of instance values",
                out,
            ),
            "minimum" | "maximum" => number(keyword, entry.value, at, false, out),
            "multipleOf" => number(keyword, entry.value, at, true, out),
            "exclusiveMinimum" | "exclusiveMaximum" => {
                if is_31_plus {
                    number(keyword, entry.value, at, false, out);
                } else if !entry.value.is_some_and(|v| v.kind() == ValueKind::Bool) {
                    invalid(at, keyword, "must be a boolean in OpenAPI 3.0", out);
                }
            }
            "enum" => {
                if !entry.value.is_some_and(|v| v.kind() == ValueKind::Array) {
                    invalid(
                        at,
                        keyword,
                        "must be an array; nonempty and unique entries are recommendations",
                        out,
                    );
                }
            }
            "required" => string_array(keyword, entry.value, at, !is_31_plus, out),
            "dependentRequired" if is_31_plus => {
                object(keyword, entry.value, at, out);
                if let Some(value) = entry.value
                    && value.kind() == ValueKind::Object
                {
                    for dependency in value.entries() {
                        string_array(
                            keyword,
                            dependency.value,
                            dependency.value.unwrap_or(dependency.key_node),
                            false,
                            out,
                        );
                    }
                }
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" => {
                cardinality(keyword, entry.value, at, out);
            }
            "minContains" | "maxContains" if is_31_plus => {
                cardinality(keyword, entry.value, at, out);
            }
            "allOf" | "anyOf" | "oneOf" => schema_array(keyword, entry.value, at, out),
            "prefixItems" if is_31_plus => schema_array(keyword, entry.value, at, out),
            "properties" => object(keyword, entry.value, at, out),
            "patternProperties" | "$defs" | "dependentSchemas" if is_31_plus => {
                object(keyword, entry.value, at, out)
            }
            _ => {}
        }
    }
}

// OAS 3.0 §4.7.24 admits a specific subset of JSON Schema, plus its own
// documented fields and specification extensions. Newer standard keywords
// cannot silently acquire 2020-12 semantics inside that subset.
fn unsupported_in_30(keyword: &str) -> bool {
    matches!(
        keyword,
        "$schema"
            | "$id"
            | "id"
            | "$anchor"
            | "$dynamicAnchor"
            | "$dynamicRef"
            | "$recursiveAnchor"
            | "$recursiveRef"
            | "$vocabulary"
            | "$comment"
            | "$defs"
            | "const"
            | "examples"
            | "patternProperties"
            | "dependencies"
            | "definitions"
            | "additionalItems"
            | "contains"
            | "prefixItems"
            | "propertyNames"
            | "if"
            | "then"
            | "else"
            | "contentEncoding"
            | "contentMediaType"
            | "contentSchema"
            | "dependentRequired"
            | "dependentSchemas"
            | "unevaluatedItems"
            | "unevaluatedProperties"
            | "minContains"
            | "maxContains"
    )
}

fn schema_array(
    keyword: &str,
    value: Option<NodeRef<'_>>,
    at: NodeRef<'_>,
    out: &mut Vec<Diagnostic>,
) {
    if !value.is_some_and(|v| v.kind() == ValueKind::Array && !v.items().is_empty()) {
        invalid(at, keyword, "must be a nonempty array of schemas", out);
    }
}

fn kind(
    keyword: &str,
    value: Option<NodeRef<'_>>,
    at: NodeRef<'_>,
    expected: ValueKind,
    reason: &str,
    out: &mut Vec<Diagnostic>,
) {
    if !value.is_some_and(|v| v.kind() == expected) {
        invalid(at, keyword, reason, out);
    }
}

fn number(
    keyword: &str,
    value: Option<NodeRef<'_>>,
    at: NodeRef<'_>,
    positive: bool,
    out: &mut Vec<Diagnostic>,
) {
    let sign = value.and_then(number_sign);
    if sign.is_none() || (positive && sign != Some(Ordering::Greater)) {
        invalid(
            at,
            keyword,
            if positive {
                "must be a finite number strictly greater than zero"
            } else {
                "must be a finite number"
            },
            out,
        );
    }
}

fn string_array(
    keyword: &str,
    value: Option<NodeRef<'_>>,
    at: NodeRef<'_>,
    nonempty: bool,
    out: &mut Vec<Diagnostic>,
) {
    let Some(value) = value.filter(|v| v.kind() == ValueKind::Array) else {
        invalid(at, keyword, "must be an array of unique strings", out);
        return;
    };
    let items = value.items();
    if nonempty && items.is_empty() {
        invalid(at, keyword, "must be nonempty in OpenAPI 3.0", out);
    }
    let mut seen = HashSet::new();
    for item in items {
        if item.kind() != ValueKind::Str {
            invalid(item, keyword, "array entries must be strings", out);
            continue;
        }
        let Some(text) = item.try_decoded_scalar() else {
            invalid(item, keyword, "array entries must be valid strings", out);
            continue;
        };
        if !seen.insert(text.into_owned()) {
            invalid(item, keyword, "array entries must be unique", out);
        }
    }
}

fn cardinality(
    keyword: &str,
    value: Option<NodeRef<'_>>,
    at: NodeRef<'_>,
    out: &mut Vec<Diagnostic>,
) {
    if !value.is_some_and(|v| {
        v.is_integral_number() && number_sign(v).is_some_and(|sign| sign != Ordering::Less)
    }) {
        invalid(
            at,
            keyword,
            "must be a nonnegative mathematical integer",
            out,
        );
    }
}

fn object(keyword: &str, value: Option<NodeRef<'_>>, at: NodeRef<'_>, out: &mut Vec<Diagnostic>) {
    if !value.is_some_and(|v| v.kind() == ValueKind::Object) {
        invalid(at, keyword, "must be an object", out);
    }
}

fn invalid(at: NodeRef<'_>, keyword: &str, reason: &str, out: &mut Vec<Diagnostic>) {
    out.push(diag_at(
        at,
        "oas-schema-invalid-keyword",
        Severity::Error,
        at.byte_range(),
        format!("`{keyword}` {reason}"),
    ));
}

// Scalar classification already establishes numeric grammar. Compare only sign
// and zero, without conversion, exponent expansion, rounding, or integer bounds.
fn number_sign(value: NodeRef<'_>) -> Option<Ordering> {
    if !matches!(value.kind(), ValueKind::Int | ValueKind::Float) {
        return None;
    }
    let raw = value.scalar_bytes();
    let negative = raw.first() == Some(&b'-');
    let body = match raw.first() {
        Some(b'+' | b'-') => &raw[1..],
        _ => raw,
    };
    let coefficient = if body.starts_with(b"0x")
        || body.starts_with(b"0X")
        || body.starts_with(b"0o")
        || body.starts_with(b"0O")
    {
        &body[2..]
    } else {
        body.split(|b| matches!(b, b'e' | b'E')).next()?
    };
    if coefficient.is_empty()
        || !coefficient
            .iter()
            .all(|b| b.is_ascii_hexdigit() || *b == b'.')
    {
        return None;
    }
    if coefficient.iter().all(|b| matches!(b, b'0' | b'.')) {
        Some(Ordering::Equal)
    } else if negative {
        Some(Ordering::Less)
    } else {
        Some(Ordering::Greater)
    }
}
