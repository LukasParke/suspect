//! Enum/type diagnostics without interpreting instance-valued annotations.

use super::{Finding, NodeRef, Pointer, Rule, ValueKind, push};

/// Checks enum values against the enclosing explicit type constraint. An
/// enum without a type may contain any JSON values, including mixed kinds.
pub(super) fn check<'d>(
    node: &NodeRef<'d>,
    rule: &Rule,
    ptrs: &crate::fast::PtrMap,
    out: &mut Vec<Finding<'d>>,
) {
    let resolved = node.resolved();
    if resolved.kind() != ValueKind::Array {
        return;
    }
    let Some(pointer) = ptrs.pointer_for(node) else {
        return;
    };
    if enum_is_instance_data(&pointer) {
        return;
    }
    let Some(start) = node.syntax().parent() else {
        return;
    };
    let mut schema = NodeRef::new(start);
    while schema.kind() != ValueKind::Object {
        let Some(parent) = schema.syntax().parent() else {
            return;
        };
        schema = NodeRef::new(parent);
    }
    let Some(declared) = schema.get("type") else {
        return;
    };
    let types = if declared.kind() == ValueKind::Array {
        declared.items()
    } else {
        vec![declared]
    };
    if types.is_empty() {
        return;
    }
    let root = NodeRef::new(node.syntax().doc().root());
    let nullable = root
        .get("openapi")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.starts_with("3.0."))
        && declared.kind() == ValueKind::Str
        && schema.get("nullable").and_then(|v| v.as_bool()) == Some(true);
    for member in resolved.items() {
        let kind = member.kind();
        if nullable && kind == ValueKind::Null {
            continue;
        }
        let matches = types.iter().any(|expected| match expected.as_str() {
            Some("string") => kind == ValueKind::Str,
            Some("boolean") => kind == ValueKind::Bool,
            Some("null") => kind == ValueKind::Null,
            Some("object") => kind == ValueKind::Object,
            Some("array") => kind == ValueKind::Array,
            Some("number") => matches!(kind, ValueKind::Int | ValueKind::Float),
            Some("integer") => member.is_integral_number(),
            // Malformed/unknown type declarations belong to schema validation.
            _ => true,
        });
        if !matches {
            push(out, rule, &member, ptrs);
        }
    }
}

/// Recursive JSONPath also finds enum-shaped data inside annotations. Named
/// map entries are consumed separately so a property/schema called `example`,
/// `default`, or `x-custom` does not disable checking of its actual schema.
fn enum_is_instance_data(pointer: &Pointer) -> bool {
    let tokens = pointer.tokens();
    let mut ancestors = tokens[..tokens.len().saturating_sub(1)].iter();
    while let Some(token) = ancestors.next() {
        match &**token {
            "schemas" | "definitions" | "$defs" | "properties" | "patternProperties"
            | "dependentSchemas" | "dependencies" | "paths" | "webhooks" | "callbacks"
            | "headers" | "responses" | "requestBodies" | "parameters" | "content"
            | "pathItems" => {
                ancestors.next();
            }
            "example" | "examples" | "default" | "const" | "enum" => return true,
            token if token.starts_with("x-") => return true,
            _ => {}
        }
    }
    false
}
