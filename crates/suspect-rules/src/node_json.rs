//! CST → JSON conversion for shipping documents to the worker.
//!
//! Two paths:
//! - [`node_to_json`]: CST → `serde_json::Value` (small subtrees, tests).
//! - [`doc_to_json_string`]: CST → JSON text in one pass, no intermediate
//!   tree. Full-document conversion of stripe (6.1MB) costs ~1.4s through
//!   the `Value` tree (a `Map` allocation per object + key `String`s, then
//!   serde re-serialization) versus ~200ms through the string writer; the
//!   frame embeds the text via `serde_json::value::RawValue` so it is never
//!   re-parsed or re-serialized host-side.

use serde_json::{Number, Value};
use suspect_low::NodeRef;

/// Converts a CST subtree into plain JSON.
///
/// Objects iterate raw mapping pairs (no per-object `Vec` + duplicate
/// dedup — `entries()` is O(n²) per object on wide mappings; last-key-wins
/// matches JSON semantics and duplicate keys are reported by validation
/// separately). Aliases resolve transparently via `NodeRef::resolved`.
#[must_use]
pub fn node_to_json<'d>(node: &NodeRef<'d>) -> Value {
    match node.kind() {
        suspect_low::ValueKind::Object => {
            let mut map = serde_json::Map::new();
            for (key, value) in node.mapping_pairs() {
                if let Some(value) = value {
                    map.insert(key, node_to_json(&value));
                }
            }
            Value::Object(map)
        }
        suspect_low::ValueKind::Array => {
            Value::Array(node.items().iter().map(node_to_json).collect())
        }
        _ => scalar_to_json(node),
    }
}

fn scalar_to_json<'d>(node: &NodeRef<'d>) -> Value {
    if let Some(b) = node.as_bool() {
        return Value::Bool(b);
    }
    if let Some(i) = node.as_i64() {
        return Value::Number(Number::from(i));
    }
    if let Some(u) = node.as_u64() {
        return Value::Number(Number::from(u));
    }
    if let Some(f) = node.as_f64() {
        return Number::from_f64(f).map_or(Value::Null, Value::Number);
    }
    match node.as_str() {
        Some(s) => Value::String(s.to_owned()),
        // Nulls and empty plain tokens both decode to JSON null.
        None => Value::Null,
    }
}

/// Writes a CST subtree as JSON text. One pass, no intermediate tree.
pub fn write_json_string<'d>(node: &NodeRef<'d>, out: &mut String) {
    match node.kind() {
        suspect_low::ValueKind::Object => {
            out.push('{');
            let mut first = true;
            for (key, value) in node.mapping_pairs() {
                if !first {
                    out.push(',');
                }
                first = false;
                escape_json_string(&key, out);
                out.push(':');
                if let Some(value) = value {
                    write_json_string(&value, out);
                } else {
                    out.push_str("null");
                }
            }
            out.push('}');
        }
        suspect_low::ValueKind::Array => {
            out.push('[');
            let mut first = true;
            for item in node.items() {
                if !first {
                    out.push(',');
                }
                first = false;
                write_json_string(&item, out);
            }
            out.push(']');
        }
        _ => write_scalar_string(node, out),
    }
}

/// Scalar leaf writer: no intermediate `Value`, no per-scalar allocation
/// beyond the escaped output. Strings take a fast path when no escapes are
/// needed (the overwhelming majority of YAML scalars).
fn write_scalar_string<'d>(node: &NodeRef<'d>, out: &mut String) {
    if let Some(b) = node.as_bool() {
        out.push_str(if b { "true" } else { "false" });
        return;
    }
    if let Some(i) = node.as_i64() {
        out.push_str(&i.to_string());
        return;
    }
    if let Some(u) = node.as_u64() {
        out.push_str(&u.to_string());
        return;
    }
    if let Some(f) = node.as_f64() {
        match serde_json::Number::from_f64(f) {
            Some(n) => out.push_str(&n.to_string()),
            None => out.push_str("null"),
        }
        return;
    }
    match node.as_str() {
        Some(s) => escape_json_string(s, out),
        None => out.push_str("null"),
    }
}

/// Writes `s` as a JSON string literal with a no-escape fast path.
fn escape_json_string(s: &str, out: &mut String) {
    out.push('"');
    if s.bytes().all(|b| b >= 0x20 && b != b'"' && b != b'\\') {
        out.push_str(s);
    } else {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
    }
    out.push('"');
}

/// Full-document JSON text in one pass.
#[must_use]
pub fn doc_to_json_string<'d>(node: &NodeRef<'d>) -> String {
    let mut out = String::with_capacity(1024 * 1024);
    write_json_string(node, &mut out);
    out
}
