//! Schema dialects and directions: what a Schema Object means for the
//! document that declares it and the side of the exchange it is checked on.
//!
//! The schema compiler speaks JSON Schema 2020-12. OpenAPI 3.1 and 3.2
//! schemas are that dialect already. OpenAPI 3.0 schemas are not: they
//! spell "may be null" as `nullable: true` and an exclusive bound as a
//! boolean next to `minimum`/`maximum`. [`normalize`] rewrites those two
//! spellings into their 2020-12 equivalents before compilation, so a 3.0
//! description is checked for what it says instead of failing to compile.
//!
//! Both dialects mark properties `readOnly` or `writeOnly`. A required
//! property the current direction never carries is not required there: a
//! response may omit a `writeOnly` property and a request may omit a
//! `readOnly` one. Nothing else about the property changes (ADR 0024).

use serde_json::{Map, Value};

/// The schema dialect of a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    /// OpenAPI 3.0 Schema Objects: `nullable`, boolean exclusive bounds.
    Oas30,
    /// JSON Schema 2020-12: OpenAPI 3.1 and 3.2, and bare schemas.
    #[default]
    Json2020,
}

impl Dialect {
    /// The dialect an `openapi` version string selects. Only `3.0.x` is
    /// the 3.0 dialect; an empty or unknown version is 2020-12.
    #[must_use]
    pub fn of_openapi(version: &str) -> Self {
        if version == "3.0" || version.starts_with("3.0.") {
            Self::Oas30
        } else {
            Self::Json2020
        }
    }
}

/// Which side of an exchange a value belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A value the client sends: `readOnly` properties are not required.
    Request,
    /// A value the server returns: `writeOnly` properties are not required.
    Response,
}

impl Direction {
    /// The annotation that excuses a required property in this direction.
    fn excusing_annotation(self) -> &'static str {
        match self {
            Self::Request => "readOnly",
            Self::Response => "writeOnly",
        }
    }
}

/// Longest `$ref` chain followed to find a property's direction
/// annotation; bounds reference cycles.
const MAX_REF_HOPS: usize = 8;

/// Keywords whose value is one schema.
const SCHEMA_KEYWORDS: [&str; 12] = [
    "additionalItems",
    "additionalProperties",
    "contains",
    "contentSchema",
    "else",
    "if",
    "items",
    "not",
    "propertyNames",
    "then",
    "unevaluatedItems",
    "unevaluatedProperties",
];

/// Keywords whose value is an array of schemas.
const SCHEMA_ARRAY_KEYWORDS: [&str; 5] = ["allOf", "anyOf", "items", "oneOf", "prefixItems"];

/// Keywords whose value maps names to schemas.
const SCHEMA_MAP_KEYWORDS: [&str; 5] = [
    "$defs",
    "definitions",
    "dependentSchemas",
    "patternProperties",
    "properties",
];

/// Rewrites `schema` into the 2020-12 schema that means the same thing in
/// `dialect` when checked in `direction`.
///
/// `component` resolves a component name to its schema, so a property
/// declared through a `$ref` still reports its direction annotation. Only
/// schema positions are rewritten: `enum`, `const`, `default` and example
/// values are data and pass through untouched.
#[must_use]
pub fn normalize<'a>(
    schema: &Value,
    dialect: Dialect,
    direction: Option<Direction>,
    component: &dyn Fn(&str) -> Option<&'a Value>,
) -> Value {
    let Value::Object(object) = schema else {
        return schema.clone();
    };
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        let rewritten = match value {
            Value::Object(_) if SCHEMA_KEYWORDS.contains(&key.as_str()) => {
                normalize(value, dialect, direction, component)
            }
            Value::Array(items) if SCHEMA_ARRAY_KEYWORDS.contains(&key.as_str()) => Value::Array(
                items
                    .iter()
                    .map(|item| normalize(item, dialect, direction, component))
                    .collect(),
            ),
            Value::Object(entries) if SCHEMA_MAP_KEYWORDS.contains(&key.as_str()) => Value::Object(
                entries
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            normalize(entry, dialect, direction, component),
                        )
                    })
                    .collect(),
            ),
            other => other.clone(),
        };
        out.insert(key.clone(), rewritten);
    }
    if let Some(direction) = direction {
        relax_required(&mut out, object, direction, component);
    }
    if dialect == Dialect::Oas30 {
        exclusive_bound(&mut out, "exclusiveMinimum", "minimum");
        exclusive_bound(&mut out, "exclusiveMaximum", "maximum");
        return nullable(out);
    }
    Value::Object(out)
}

/// Drops from `required` every property the direction never carries.
fn relax_required<'a>(
    out: &mut Map<String, Value>,
    original: &Map<String, Value>,
    direction: Direction,
    component: &dyn Fn(&str) -> Option<&'a Value>,
) {
    let (Some(Value::Array(required)), Some(Value::Object(properties))) =
        (original.get("required"), original.get("properties"))
    else {
        return;
    };
    let annotation = direction.excusing_annotation();
    let kept: Vec<Value> = required
        .iter()
        .filter(|name| {
            name.as_str()
                .and_then(|name| properties.get(name))
                .is_none_or(|property| !annotated(property, annotation, component, 0))
        })
        .cloned()
        .collect();
    if kept.len() != required.len() {
        out.insert("required".to_owned(), Value::Array(kept));
    }
}

/// Whether a property schema carries `annotation: true`, directly, through
/// its `$ref` target, or through a member of its `allOf`.
fn annotated<'a>(
    schema: &Value,
    annotation: &str,
    component: &dyn Fn(&str) -> Option<&'a Value>,
    hops: usize,
) -> bool {
    if schema.get(annotation) == Some(&Value::Bool(true)) {
        return true;
    }
    if hops >= MAX_REF_HOPS {
        return false;
    }
    if let Some(target) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(component_name)
        .and_then(|name| component(&name))
        && annotated(target, annotation, component, hops + 1)
    {
        return true;
    }
    schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|members| {
            members
                .iter()
                .any(|member| annotated(member, annotation, component, hops + 1))
        })
}

/// The component a `#/components/schemas/<name>` reference names (the
/// first pointer token after the prefix, unescaped).
#[must_use]
pub fn component_name(reference: &str) -> Option<String> {
    let token = reference
        .strip_prefix("#/components/schemas/")?
        .split('/')
        .next()?;
    if token.is_empty() {
        return None;
    }
    let decoded = percent_decode(token);
    Some(decoded.replace("~1", "/").replace("~0", "~"))
}

/// Decodes `%XX` escapes of a URI fragment; malformed escapes stay as
/// written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// OpenAPI 3.0 `exclusiveMinimum: true` beside `minimum: n` is 2020-12
/// `exclusiveMinimum: n`. A `false` (or a `true` without its bound) says
/// nothing and is dropped.
fn exclusive_bound(schema: &mut Map<String, Value>, exclusive: &str, bound: &str) {
    let Some(Value::Bool(on)) = schema.get(exclusive) else {
        return;
    };
    let on = *on;
    schema.remove(exclusive);
    if on && let Some(limit) = schema.remove(bound) {
        schema.insert(exclusive.to_owned(), limit);
    }
}

/// OpenAPI 3.0 `nullable: true` admits `null`.
///
/// Beside a `type`, it widens that type, which is what 3.0.3 specifies.
/// Without one — the `nullable` + `allOf: [$ref]` idiom for a nullable
/// reference — the whole schema becomes one branch of an `anyOf` whose
/// other branch is `null`. 3.0.3 gives `nullable` no effect there, but
/// descriptions written that way mean "or null", and reading them
/// literally would fail every such response.
fn nullable(mut schema: Map<String, Value>) -> Value {
    let on = schema.remove("nullable") == Some(Value::Bool(true));
    if !on {
        return Value::Object(schema);
    }
    match schema.get("type") {
        Some(Value::String(name)) => {
            if name != "null" {
                let widened = Value::Array(vec![
                    Value::String(name.clone()),
                    Value::String("null".to_owned()),
                ]);
                schema.insert("type".to_owned(), widened);
            }
            Value::Object(schema)
        }
        Some(_) => Value::Object(schema),
        None => serde_json::json!({
            "anyOf": [{"type": "null"}, Value::Object(schema)]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schemas;
    use serde_json::json;

    fn schemas(dialect: Dialect, components: Value) -> Schemas {
        let map: std::collections::BTreeMap<String, Value> =
            serde_json::from_value(components).unwrap();
        Schemas::from_any_map(map.iter()).with_dialect(dialect)
    }

    fn accepts(schemas: &Schemas, direction: Direction, schema: &Value, instance: Value) -> bool {
        schemas.validate_as(schema, &instance, direction).is_empty()
    }

    #[test]
    fn the_openapi_version_selects_the_dialect() {
        assert_eq!(Dialect::of_openapi("3.0.3"), Dialect::Oas30);
        assert_eq!(Dialect::of_openapi("3.0"), Dialect::Oas30);
        assert_eq!(Dialect::of_openapi("3.1.0"), Dialect::Json2020);
        assert_eq!(Dialect::of_openapi("3.2.0"), Dialect::Json2020);
        assert_eq!(Dialect::of_openapi(""), Dialect::Json2020);
    }

    #[test]
    fn oas30_nullable_widens_the_declared_type() {
        let schemas = schemas(Dialect::Oas30, json!({}));
        let schema = json!({"type": "string", "nullable": true});
        assert!(accepts(&schemas, Direction::Response, &schema, json!(null)));
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!("rex")
        ));
        assert!(!accepts(&schemas, Direction::Response, &schema, json!(7)));
    }

    #[test]
    fn oas30_nullable_reference_idiom_admits_null() {
        let schemas = schemas(
            Dialect::Oas30,
            json!({"Owner": {"type": "object", "required": ["id"]}}),
        );
        let schema = json!({
            "type": "object",
            "properties": {
                "owner": {"nullable": true, "allOf": [{"$ref": "#/components/schemas/Owner"}]}
            }
        });
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"owner": null})
        ));
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"owner": {"id": 1}})
        ));
        assert!(!accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"owner": {}})
        ));
    }

    #[test]
    fn nullable_inside_a_referenced_component_is_honoured() {
        let schemas = schemas(
            Dialect::Oas30,
            json!({"Tag": {"type": "string", "nullable": true}}),
        );
        let schema = json!({"$ref": "#/components/schemas/Tag"});
        assert!(accepts(&schemas, Direction::Response, &schema, json!(null)));
    }

    #[test]
    fn nullable_is_not_a_2020_12_keyword() {
        let schemas = schemas(Dialect::Json2020, json!({}));
        let schema = json!({"type": "string", "nullable": true});
        assert!(!accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!(null)
        ));
    }

    #[test]
    fn oas30_boolean_exclusive_bounds_compile_and_exclude_the_bound() {
        let schemas = schemas(Dialect::Oas30, json!({}));
        let schema = json!({
            "type": "number",
            "minimum": 0, "exclusiveMinimum": true,
            "maximum": 10, "exclusiveMaximum": false
        });
        assert!(!accepts(&schemas, Direction::Response, &schema, json!(0)));
        assert!(accepts(&schemas, Direction::Response, &schema, json!(1)));
        assert!(accepts(&schemas, Direction::Response, &schema, json!(10)));
        assert!(!accepts(&schemas, Direction::Response, &schema, json!(11)));
    }

    #[test]
    fn a_response_may_omit_a_required_write_only_property() {
        let schemas = schemas(Dialect::Json2020, json!({}));
        let schema = json!({
            "type": "object",
            "required": ["name", "password"],
            "properties": {
                "name": {"type": "string"},
                "password": {"type": "string", "writeOnly": true}
            }
        });
        let without_password = json!({"name": "rex"});
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            without_password.clone()
        ));
        assert!(!accepts(
            &schemas,
            Direction::Request,
            &schema,
            without_password
        ));
        // Only requiredness relaxes: a present value is still type-checked.
        assert!(!accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"name": "rex", "password": 7})
        ));
    }

    #[test]
    fn a_request_may_omit_a_required_read_only_property_declared_by_reference() {
        let schemas = schemas(
            Dialect::Json2020,
            json!({"Id": {"type": "string", "readOnly": true}}),
        );
        let schema = json!({
            "type": "object",
            "required": ["id", "name"],
            "properties": {
                "id": {"$ref": "#/components/schemas/Id"},
                "name": {"type": "string"}
            }
        });
        let without_id = json!({"name": "rex"});
        assert!(accepts(
            &schemas,
            Direction::Request,
            &schema,
            without_id.clone()
        ));
        assert!(!accepts(&schemas, Direction::Response, &schema, without_id));
    }

    #[test]
    fn data_values_are_not_rewritten_as_schemas() {
        let schemas = schemas(Dialect::Oas30, json!({}));
        // The enum member is data that happens to look like a schema.
        let schema = json!({"enum": [{"type": "string", "nullable": true}]});
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"type": "string", "nullable": true})
        ));
        assert!(!accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"type": ["string", "null"]})
        ));
    }

    #[test]
    fn component_names_decode_pointer_and_uri_escapes() {
        assert_eq!(
            component_name("#/components/schemas/Pet").as_deref(),
            Some("Pet")
        );
        assert_eq!(
            component_name("#/components/schemas/a~1b%20c/properties/x").as_deref(),
            Some("a/b c")
        );
        assert_eq!(component_name("#/components/responses/Pet"), None);
        assert_eq!(component_name("other.yaml#/components/schemas/Pet"), None);
    }
}
