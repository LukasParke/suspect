//! `type`, `enum`, `const`, `uniqueItems`: type membership and deep equality.

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::compile::TypeBits;
pub(crate) use crate::equality::EqualityBudget;
use crate::exec::{Ctx, Stack};

pub(crate) fn kind_name(k: ValueKind) -> &'static str {
    match k {
        ValueKind::Null => "null",
        ValueKind::Bool => "boolean",
        ValueKind::Int => "integer",
        ValueKind::Float => "number",
        ValueKind::Str => "string",
        ValueKind::Array => "array",
        ValueKind::Object => "object",
    }
}

/// Renders a [`TypeBits`] set for error messages (`integer`, `string|number`…).
pub(crate) fn type_names(bits: TypeBits) -> String {
    let mut names: Vec<&'static str> = Vec::new();
    let b = bits.0;
    if b & TypeBits::NULL != 0 {
        names.push("null");
    }
    if b & TypeBits::BOOL != 0 {
        names.push("boolean");
    }
    if b & TypeBits::INT != 0 {
        names.push("integer");
    }
    if b & TypeBits::NUM != 0 {
        names.push("number");
    }
    if b & TypeBits::STR != 0 {
        names.push("string");
    }
    if b & TypeBits::ARR != 0 {
        names.push("array");
    }
    if b & TypeBits::OBJ != 0 {
        names.push("object");
    }
    names.join("|")
}

pub(crate) fn check_enum<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    instance: NodeRef<'d>,
    values: &[NodeRef<'d>],
) -> bool {
    for value in values {
        if !ctx.step(st, at) {
            return false;
        }
        match ctx.equality.compare(instance, *value) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(message) => {
                ctx.fail_evaluation(st, at, message);
                return false;
            }
        }
    }
    ctx.emit(st, at, "value does not match any `enum` entry".into());
    false
}

pub(crate) fn check_const<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    instance: NodeRef<'d>,
    value: NodeRef<'d>,
) -> bool {
    match ctx.equality.compare(instance, value) {
        Ok(true) => true,
        Ok(false) => {
            ctx.emit(st, at, "value does not equal the `const` value".into());
            false
        }
        Err(message) => {
            ctx.fail_evaluation(st, at, message);
            false
        }
    }
}

pub(crate) fn check_unique_items<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    instance: NodeRef<'d>,
) -> bool {
    if instance.kind() != ValueKind::Array {
        return true;
    }
    let items = instance.items();
    for (index, item) in items.iter().enumerate() {
        if !ctx.step(st, at) {
            return false;
        }
        for (previous_index, previous) in items[..index].iter().enumerate() {
            if !ctx.step(st, at) {
                return false;
            }
            match ctx.equality.compare(*item, *previous) {
                Ok(false) => {}
                Ok(true) => {
                    ctx.emit(
                        st,
                        at,
                        format!(
                            "array items {previous_index} and {index} are equal (`uniqueItems`)"
                        ),
                    );
                    return false;
                }
                Err(message) => {
                    ctx.fail_evaluation(st, at, message);
                    return false;
                }
            }
        }
    }
    true
}
