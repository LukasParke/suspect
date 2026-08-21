use suspect_low::ValueKind;

/// An owned, ordered value tree — the editable form overlay actions apply to.
///
/// Scalars are owned (decoded); objects keep document order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Box<str>),
    Array(Vec<Value>),
    /// Insertion-ordered map.
    Object(Vec<(Box<str>, Value)>),
}

impl Value {
    /// Materializes a low-level node into an owned value.
    #[must_use]
    pub fn from_node(node: suspect_low::NodeRef<'_>) -> Value {
        match node.kind() {
            ValueKind::Null => Value::Null,
            ValueKind::Bool => Value::Bool(node.as_bool().unwrap_or(false)),
            ValueKind::Int => Value::Int(node.as_i64().unwrap_or(0)),
            ValueKind::Float => Value::Float(node.as_f64().unwrap_or(0.0)),
            ValueKind::Str => Value::Str(String::from_utf8_lossy(node.scalar_bytes()).into()),
            ValueKind::Array => {
                Value::Array(node.items().into_iter().map(Value::from_node).collect())
            }
            ValueKind::Object => Value::Object(
                node.entries()
                    .into_iter()
                    .filter_map(|e| e.value.map(|v| (e.key.into(), Value::from_node(v))))
                    .collect(),
            ),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Recursively merges `update` into `self` per Overlay spec: objects
    /// merge by key (update wins, new keys append), anything else replaces.
    pub fn merge(&mut self, update: &Value) {
        match (self, update) {
            (Value::Object(entries), Value::Object(updates)) => {
                for (uk, uv) in updates {
                    match entries.iter_mut().find(|(k, _)| k.as_ref() == uk.as_ref()) {
                        Some((_, existing)) => existing.merge(uv),
                        None => entries.push((uk.clone(), uv.clone())),
                    }
                }
            }
            (slot, new) => *slot = new.clone(),
        }
    }

    /// Compact JSON (no whitespace).
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_json(self, &mut out, None, 0);
        out
    }

    /// Pretty JSON with two-space indent.
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        let mut out = String::new();
        write_json(self, &mut out, Some(2), 0);
        out
    }

    /// YAML 1.2 block-style emission.
    #[must_use]
    pub fn to_yaml(&self) -> String {
        let mut out = String::new();
        write_yaml(self, &mut out, 0, false);
        out
    }
}

fn write_json(v: &Value, out: &mut String, indent: Option<usize>, depth: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if f.is_finite() {
                let s = f.to_string();
                out.push_str(&s);
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    out.push_str(".0"); // keep it a float on re-parse
                }
            } else {
                out.push_str("null"); // JSON has no inf/nan
            }
        }
        Value::Str(s) => write_json_string(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Some(w) = indent {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', w * (depth + 1)));
                }
                write_json(item, out, indent, depth + 1);
            }
            if let Some(w) = indent {
                out.push('\n');
                out.extend(std::iter::repeat_n(' ', w * depth));
            }
            out.push(']');
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, val)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Some(w) = indent {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', w * (depth + 1)));
                }
                write_json_string(k, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_json(val, out, indent, depth + 1);
            }
            if let Some(w) = indent {
                out.push('\n');
                out.extend(std::iter::repeat_n(' ', w * depth));
            }
            out.push('}');
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
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
    out.push('"');
}

fn needs_quoting(s: &str) -> bool {
    s.is_empty()
        || s.parse::<i64>().is_ok()
        || s.parse::<f64>().is_ok()
        || matches!(s, "true" | "false" | "null" | "~")
        || !s
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '@' | '$' | '+' | '(' | ')' | '?' | '=' | ':' | '\'' | '*' | '&' | '%' | '!' | '#' | '[' | ']' | '<' | '>' | '~'))
}

fn write_yaml_str(s: &str, out: &mut String) {
    if needs_quoting(s) {
        // double-quote with escapes
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn write_yaml(v: &Value, out: &mut String, depth: usize, in_flow_parent: bool) {
    let pad = |out: &mut String, d: usize| out.extend(std::iter::repeat_n(' ', d * 2));
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if f.is_finite() {
                let s = f.to_string();
                out.push_str(&s);
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    out.push_str(".0");
                }
            } else {
                out.push_str(".inf");
            }
        }
        Value::Str(s) => write_yaml_str(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            for item in items {
                out.push('\n');
                pad(out, depth);
                out.push_str("- ");
                write_yaml(item, out, depth + 1, false);
            }
            let _ = in_flow_parent;
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            for (i, (k, val)) in entries.iter().enumerate() {
                if !(i == 0 && depth == 0 && in_flow_parent)
                    && (i > 0 || depth > 0) {
                        out.push('\n');
                        pad(out, depth);
                    }
                write_yaml_str(k, out);
                out.push(':');
                match val {
                    Value::Object(inner) if !inner.is_empty() => {
                        write_yaml(val, out, depth + 1, false);
                    }
                    Value::Array(inner) if !inner.is_empty() => {
                        // sequences under a key start on the next line at same indent
                        write_yaml(val, out, depth, false);
                    }
                    _ => {
                        out.push(' ');
                        write_yaml(val, out, depth + 1, false);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_emission() {
        let v = Value::Object(vec![
            ("a".into(), Value::Int(1)),
            ("b".into(), Value::Array(vec![Value::Str("x".into()), Value::Null])),
            ("c".into(), Value::Float(1.5)),
        ]);
        assert_eq!(v.to_json(), r#"{"a":1,"b":["x",null],"c":1.5}"#);
    }

    #[test]
    fn json_string_escaping() {
        let v = Value::Str("he said \"hi\"\n\tok".into());
        assert_eq!(v.to_json(), "\"he said \\\"hi\\\"\\n\\tok\"");
    }

    #[test]
    fn float_keeps_decimal_point() {
        assert_eq!(Value::Float(2.0).to_json(), "2.0");
        assert_eq!(Value::Float(0.1).to_json(), "0.1");
    }

    #[test]
    fn yaml_emission() {
        let v = Value::Object(vec![
            ("openapi".into(), Value::Str("3.1.0".into())),
            ("info".into(), Value::Object(vec![("title".into(), Value::Str("T".into()))])),
            ("n".into(), Value::Int(3)),
        ]);
        let yaml = v.to_yaml();
        assert!(yaml.contains("openapi: 3.1.0"), "got: {yaml}");
        assert!(yaml.contains("info:"), "got: {yaml}");
        assert!(yaml.contains("  title: T"), "got: {yaml}");
        assert!(yaml.contains("n: 3"), "got: {yaml}");
    }

    #[test]
    fn yaml_quotes_ambiguous_strings() {
        let v = Value::Object(vec![("v".into(), Value::Str("3.1".into()))]);
        assert!(v.to_yaml().contains("v: \"3.1\""), "got: {}", v.to_yaml());
    }

    #[test]
    fn merge_semantics() {
        let mut base = Value::Object(vec![
            ("a".into(), Value::Int(1)),
            ("nested".into(), Value::Object(vec![("x".into(), Value::Int(1))])),
        ]);
        let update = Value::Object(vec![
            ("a".into(), Value::Int(2)),
            ("nested".into(), Value::Object(vec![("y".into(), Value::Int(9))])),
            ("new".into(), Value::Bool(true)),
        ]);
        base.merge(&update);
        assert_eq!(base.get("a"), Some(&Value::Int(2)));
        assert_eq!(base.get("nested").unwrap().get("x"), Some(&Value::Int(1)));
        assert_eq!(base.get("nested").unwrap().get("y"), Some(&Value::Int(9)));
        assert_eq!(base.get("new"), Some(&Value::Bool(true)));
    }
}
