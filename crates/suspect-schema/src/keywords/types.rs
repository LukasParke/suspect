//! `type`, `enum`, `const`: type membership and deep value equality.

use rustc_hash::FxHashMap;
use suspect_low::{NodeRef, ValueKind};

use crate::compile::TypeBits;

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

/// Deep structural equality over schema/instance subtrees.
///
/// Numbers compare numerically (so `1` equals `1.0`); objects are
/// order-insensitive; arrays order-sensitive. `depth` guards against
/// hostile nesting — beyond the cap values are considered unequal, which is
/// always safe (it can only reject).
pub(crate) fn value_eq(a: NodeRef<'_>, b: NodeRef<'_>, depth: usize) -> bool {
    if depth > 256 {
        return false;
    }
    let ka = a.kind();
    let kb = b.kind();
    match (ka, kb) {
        (ValueKind::Int | ValueKind::Float, ValueKind::Int | ValueKind::Float) => {
            match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => x == y,
                _ => a.scalar_bytes() == b.scalar_bytes(),
            }
        }
        _ if ka != kb => false,
        (ValueKind::Null, ValueKind::Null) => true,
        (ValueKind::Bool, ValueKind::Bool) => a.as_bool() == b.as_bool(),
        (ValueKind::Str, ValueKind::Str) => a.as_str() == b.as_str(),
        (ValueKind::Array, ValueKind::Array) => {
            let ai = a.items();
            let bi = b.items();
            ai.len() == bi.len() && ai.iter().zip(&bi).all(|(x, y)| value_eq(*x, *y, depth + 1))
        }
        (ValueKind::Object, ValueKind::Object) => {
            let ae = a.entries();
            let be = b.entries();
            if ae.len() != be.len() {
                return false;
            }
            let map: FxHashMap<&str, NodeRef<'_>> = be
                .into_iter()
                .filter_map(|e| e.value.map(|v| (e.key, v)))
                .collect();
            ae.iter().all(|e| {
                map.get(e.key)
                    .is_some_and(|bv| e.value.is_some_and(|av| value_eq(av, *bv, depth + 1)))
            })
        }
        _ => false,
    }
}
