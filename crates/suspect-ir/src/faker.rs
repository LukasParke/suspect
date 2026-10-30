//! Realistic example-value synthesis ("faker-style"), deterministic.
//!
//! Given a JSON Schema fragment and the field's name, produces a value
//! that looks like real-world data: `email` fields get an email address,
//! `uuid`-format strings get a UUID, `date-time` gets an RFC 3339
//! timestamp. A pure function of `(schema, field_name)` — no RNG — so
//! runs are reproducible and snapshots stable.
//!
//! Priority order: explicit `default`, explicit `example`, `enum`,
//! `format`, field-name semantics, type fallback. String lengths respect
//! `minLength`/`maxLength`; numbers respect `minimum`/`maximum`.

use serde_json::Value;

/// Maximum nesting depth when synthesizing objects/arrays.
const MAX_DEPTH: usize = 4;

/// Synthesizes a realistic value for `schema`, guided by `field_name`.
#[must_use]
pub fn value(schema: &Value, field_name: &str) -> Value {
    value_at_depth(schema, field_name, 0)
}

fn value_at_depth(schema: &Value, field_name: &str, depth: usize) -> Value {
    let Some(obj) = schema.as_object() else {
        return Value::Null;
    };

    // Explicit default or example wins — the instance checks validate
    // these against the schema, so reusing them keeps bodies consistent.
    if let Some(default) = obj.get("default")
        && !default.is_null()
    {
        return default.clone();
    }
    if let Some(example) = obj.get("example")
        && !example.is_null()
    {
        return example.clone();
    }
    if let Some(examples) = obj.get("examples").and_then(Value::as_array)
        && let Some(first) = examples.first()
        && !first.is_null()
    {
        return first.clone();
    }
    if let Some(enum_values) = obj.get("enum").and_then(Value::as_array)
        && let Some(first) = enum_values.first()
    {
        return first.clone();
    }

    match obj.get("type").and_then(Value::as_str) {
        Some("string") => string_value(obj, field_name),
        Some("integer") | Some("number") => number_value(obj),
        Some("boolean") => Value::Bool(true),
        Some("array") if depth < MAX_DEPTH => {
            let items = obj.get("items").cloned().unwrap_or(Value::Null);
            Value::Array(vec![value_at_depth(
                &items,
                singular(field_name),
                depth + 1,
            )])
        }
        Some("object") | Some(_) if depth < MAX_DEPTH => object_value(obj, depth),
        _ => {
            // Missing type: infer from keywords.
            if obj.contains_key("properties") {
                if depth < MAX_DEPTH {
                    object_value(obj, depth)
                } else {
                    Value::Object(serde_json::Map::new())
                }
            } else {
                string_value(obj, field_name)
            }
        }
    }
}

/// Synthesizes an object from its `properties`, honoring `required`.
fn object_value(obj: &serde_json::Map<String, Value>, depth: usize) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(properties) = obj.get("properties").and_then(Value::as_object) {
        for (name, prop_schema) in properties {
            out.insert(name.clone(), value_at_depth(prop_schema, name, depth + 1));
        }
    }
    Value::Object(out)
}

/// Format-driven strings; falls back to name semantics, then generic text
/// padded to satisfy `minLength`.
fn string_value(obj: &serde_json::Map<String, Value>, field_name: &str) -> Value {
    let format = obj.get("format").and_then(Value::as_str).unwrap_or("");
    let base: String = match format {
        "uuid" | "uuidv4" => "f47ac10b-58cc-4372-a567-0e02b2c3d479".into(),
        "email" | "idn-email" => "ada@example.org".into(),
        "uri" | "uri-reference" | "iri" | "iri-reference" => {
            "https://api.example.org/v1/resource".into()
        }
        "date-time" | "datetime" => "2026-01-15T09:30:00Z".into(),
        "date" => "2026-01-15".into(),
        "time" => "09:30:00Z".into(),
        "duration" => "P1DT2H".into(),
        "ipv4" => "192.0.2.1".into(),
        "ipv6" => "2001:db8::1".into(),
        "hostname" | "idn-hostname" => "api.example.org".into(),
        "json-pointer" => "/resources/1".into(),
        "relative-json-pointer" => "0".into(),
        "regex" => "^[a-z]+$".into(),
        "byte" => "c3VzcGVjdA==".into(),
        "binary" => "c3VzcGVjdA==".into(),
        _ => name_based_string(field_name),
    };
    let _ = base;
    fit_length(base, obj)
}

/// Field-name semantics for strings without a `format`.
#[must_use]
fn name_based_string(field_name: &str) -> String {
    let lower = field_name.to_ascii_lowercase();
    let word = |needle: &str| lower.contains(needle);
    if word("email") {
        "ada@example.org".into()
    } else if word("url") || word("uri") || word("website") || word("homepage") || word("href") {
        "https://api.example.org/v1/resource".into()
    } else if word("phone") || word("tel") {
        "+1-555-0100".into()
    } else if word("first_name") || word("firstname") {
        "Ada".into()
    } else if word("last_name") || word("lastname") || word("surname") {
        "Lovelace".into()
    } else if word("fullname") || lower == "name" || word("author") || word("owner") {
        "Ada Lovelace".into()
    } else if word("city") {
        "Springfield".into()
    } else if word("country") {
        "US".into()
    } else if word("zip") || word("postal") {
        "90210".into()
    } else if word("address") || word("street") {
        "1600 Amphitheatre Parkway".into()
    } else if word("description") || word("summary") || word("bio") || word("comment") {
        "A concise, realistic sample description.".into()
    } else if word("token") || word("secret") || word("api_key") || word("apikey") {
        "9f2c1a7e5b4d3c8a".into()
    } else if word("color") || word("colour") {
        "#4A6FA5".into()
    } else if word("slug") {
        "sample-slug".into()
    } else if word("version") {
        "1.4.2".into()
    } else if word("language") || word("locale") {
        "en-US".into()
    } else if word("timezone") {
        "UTC".into()
    } else if lower.ends_with("_id") || lower == "id" || lower.ends_with("id") {
        // Identifier-ish: a short, readable id rather than a full UUID
        // (full UUIDs are the format's job).
        "1".into()
    } else if word("title") || word("label") {
        "Sample Title".into()
    } else {
        "sample text".into()
    }
}

/// Numbers honoring `minimum`/`maximum`; integers stay integers.
fn number_value(obj: &serde_json::Map<String, Value>) -> Value {
    let minimum = obj.get("minimum").and_then(Value::as_f64);
    let maximum = obj.get("maximum").and_then(Value::as_f64);
    let mut n: f64 = 1.0;
    if let Some(min) = minimum
        && n < min
    {
        n = min;
    }
    if let Some(max) = maximum
        && n > max
    {
        n = max;
    }
    let is_integer = obj.get("type").and_then(Value::as_str) == Some("integer") || n.fract() == 0.0;
    if is_integer {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::from(1))
    }
}

/// Pads or truncates to honor `minLength`/`maxLength` when present.
fn fit_length(mut text: String, obj: &serde_json::Map<String, Value>) -> Value {
    let min = obj.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    let max = obj
        .get("maxLength")
        .and_then(Value::as_u64)
        .map(|m| m as usize);
    if text.len() < min {
        while text.len() < min {
            text.push('x');
        }
    }
    if let Some(max) = max {
        while text.len() > max && !text.is_empty() {
            text.pop();
        }
    }
    Value::String(text)
}

/// `petIds` → `petId`, `tags` → `tag` (best-effort singular for array items).
fn singular(field_name: &str) -> &str {
    if field_name.ends_with("ies") && field_name.len() > 3 {
        return &field_name[..field_name.len() - 3];
        // "ies" → "y" would need an owned string; the trimmed form is fine.
    }
    if field_name.ends_with('s') && !field_name.ends_with("ss") {
        return &field_name[..field_name.len() - 1];
    }
    field_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_drive_strings() {
        assert_eq!(
            value(&json!({"type": "string", "format": "email"}), "contact"),
            json!("ada@example.org")
        );
        assert_eq!(
            value(&json!({"type": "string", "format": "uuid"}), "id"),
            json!("f47ac10b-58cc-4372-a567-0e02b2c3d479")
        );
        assert_eq!(
            value(
                &json!({"type": "string", "format": "date-time"}),
                "createdAt"
            ),
            json!("2026-01-15T09:30:00Z")
        );
        assert_eq!(
            value(&json!({"type": "string", "format": "ipv4"}), "ip"),
            json!("192.0.2.1")
        );
    }

    #[test]
    fn names_drive_unformatted_strings() {
        assert_eq!(
            value(&json!({"type": "string"}), "email"),
            json!("ada@example.org")
        );
        assert_eq!(
            value(&json!({"type": "string"}), "first_name"),
            json!("Ada")
        );
        assert_eq!(
            value(&json!({"type": "string"}), "city"),
            json!("Springfield")
        );
        assert_eq!(
            value(&json!({"type": "string"}), "description"),
            json!("A concise, realistic sample description.")
        );
    }

    #[test]
    fn default_example_and_enum_win() {
        assert_eq!(
            value(&json!({"type": "string", "default": "preset"}), "anything"),
            json!("preset")
        );
        assert_eq!(
            value(&json!({"type": "string", "example": "shown"}), "anything"),
            json!("shown")
        );
        assert_eq!(
            value(&json!({"type": "string", "enum": ["a", "b"]}), "anything"),
            json!("a")
        );
    }

    #[test]
    fn numbers_respect_bounds_and_integrality() {
        assert_eq!(value(&json!({"type": "integer"}), "count"), json!(1));
        assert_eq!(
            value(&json!({"type": "integer", "minimum": 5}), "count"),
            json!(5)
        );
        assert_eq!(
            value(&json!({"type": "integer", "maximum": 0}), "count"),
            json!(0)
        );
        assert_eq!(
            value(&json!({"type": "number", "minimum": 2.5}), "amount"),
            json!(2.5)
        );
    }

    #[test]
    fn lengths_are_honored() {
        let schema = json!({"type": "string", "minLength": 30, "maxLength": 34});
        let out = value(&schema, "bio");
        let text = out.as_str().unwrap();
        assert!((30..=34).contains(&text.len()), "{text}");
    }

    #[test]
    fn objects_recurse_and_arrays_singlularize() {
        let schema = json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "email": {"type": "string"},
                "count": {"type": "integer"}
            }
        });
        let out = value(&schema, "user");
        assert_eq!(
            out,
            json!({"id": "1", "email": "ada@example.org", "count": 1})
        );

        let array = json!({"type": "array", "items": {"type": "string"}});
        assert_eq!(value(&array, "tags"), json!(["sample text"]));
    }

    #[test]
    fn synthesis_is_deterministic() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        assert_eq!(value(&schema, "pet"), value(&schema, "pet"));
    }
}
