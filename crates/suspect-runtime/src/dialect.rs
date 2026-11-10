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

use std::collections::BTreeSet;

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
    rewrite(schema, dialect, direction, component, &BTreeSet::new())
}

/// [`normalize`] for one schema. `inherited` names the properties the
/// enclosing `allOf` already knows the direction never carries.
fn rewrite<'a>(
    schema: &Value,
    dialect: Dialect,
    direction: Option<Direction>,
    component: &dyn Fn(&str) -> Option<&'a Value>,
    inherited: &BTreeSet<String>,
) -> Value {
    let Value::Object(object) = schema else {
        return schema.clone();
    };
    // A requirement and the property it names are often declared apart:
    // the properties in one `allOf` member (usually a reference) and
    // `required` in another, or beside the `allOf`. The properties of the
    // whole composition decide what a `required` anywhere in it may demand.
    let excused = match direction {
        Some(direction) if object.contains_key("required") || object.contains_key("allOf") => {
            let mut names = inherited.clone();
            excused_properties(
                schema,
                direction.excusing_annotation(),
                component,
                &mut BTreeSet::new(),
                &mut names,
            );
            names
        }
        _ => BTreeSet::new(),
    };
    let nothing = BTreeSet::new();
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        let rewritten = match value {
            Value::Object(_) if SCHEMA_KEYWORDS.contains(&key.as_str()) => {
                rewrite(value, dialect, direction, component, &nothing)
            }
            Value::Array(items) if SCHEMA_ARRAY_KEYWORDS.contains(&key.as_str()) => {
                let passed = if key == "allOf" { &excused } else { &nothing };
                Value::Array(
                    items
                        .iter()
                        .map(|item| rewrite(item, dialect, direction, component, passed))
                        .collect(),
                )
            }
            Value::Object(entries) if SCHEMA_MAP_KEYWORDS.contains(&key.as_str()) => Value::Object(
                entries
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            rewrite(entry, dialect, direction, component, &nothing),
                        )
                    })
                    .collect(),
            ),
            // The wrapping of a typeless nullable schema (see `nullable`)
            // moves what it holds; a reference into it moves with it.
            Value::String(reference) if key == "$ref" && dialect == Dialect::Oas30 => {
                relocated(reference, component).map_or_else(|| value.clone(), Value::String)
            }
            other => other.clone(),
        };
        out.insert(key.clone(), rewritten);
    }
    if let Some(Value::Array(required)) = object.get("required")
        && !excused.is_empty()
    {
        let kept: Vec<Value> = required
            .iter()
            .filter(|name| name.as_str().is_none_or(|name| !excused.contains(name)))
            .cloned()
            .collect();
        if kept.len() != required.len() {
            out.insert("required".to_owned(), Value::Array(kept));
        }
    }
    if dialect == Dialect::Oas30 {
        exclusive_bound(&mut out, "exclusiveMinimum", "minimum");
        exclusive_bound(&mut out, "exclusiveMaximum", "maximum");
        return nullable(out);
    }
    Value::Object(out)
}

/// Collects into `names` every property of `schema` that carries
/// `annotation: true`: its own `properties`, those of its `$ref` target and
/// those of each `allOf` member. `seen` holds the references already
/// followed, so a recursive composition is walked once.
fn excused_properties<'a>(
    schema: &Value,
    annotation: &str,
    component: &dyn Fn(&str) -> Option<&'a Value>,
    seen: &mut BTreeSet<String>,
    names: &mut BTreeSet<String>,
) {
    if let Some(Value::Object(properties)) = schema.get("properties") {
        for (name, property) in properties {
            if annotated(property, annotation, component, 0) {
                names.insert(name.clone());
            }
        }
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str)
        && seen.insert(reference.to_owned())
        && let Some(target) = follow(reference, component)
    {
        excused_properties(target, annotation, component, seen, names);
    }
    if let Some(Value::Array(members)) = schema.get("allOf") {
        for member in members {
            excused_properties(member, annotation, component, seen, names);
        }
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
        .and_then(|reference| follow(reference, component))
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

/// The schema a `#/components/schemas/<name>[/<pointer>]` reference
/// addresses: the component, then the rest of the pointer inside it.
pub(crate) fn follow<'a>(
    reference: &str,
    component: &dyn Fn(&str) -> Option<&'a Value>,
) -> Option<&'a Value> {
    let mut tokens = reference
        .strip_prefix("#/components/schemas/")?
        .split('/')
        .map(pointer_token);
    let mut current = component(&tokens.next().filter(|name| !name.is_empty())?)?;
    for token in tokens {
        current = match current {
            Value::Object(entries) => entries.get(&token)?,
            Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Where a reference into a component leads once the component is
/// normalized for OpenAPI 3.0: every typeless `nullable` schema on the way
/// is wrapped in an `anyOf` whose second branch holds it. `None` when the
/// reference passes through no such schema and stays as written.
fn relocated<'a>(reference: &str, component: &dyn Fn(&str) -> Option<&'a Value>) -> Option<String> {
    let mut tokens = reference.strip_prefix("#/components/schemas/")?.split('/');
    let name = tokens.next()?;
    let mut current = component(&pointer_token(name))?;
    let mut path = format!("#/components/schemas/{name}");
    let mut moved = false;
    for token in tokens {
        if current.get("nullable") == Some(&Value::Bool(true)) && current.get("type").is_none() {
            path.push_str("/anyOf/1");
            moved = true;
        }
        current = match current {
            Value::Object(entries) => entries.get(&pointer_token(token))?,
            Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
        path.push('/');
        path.push_str(token);
    }
    moved.then_some(path)
}

/// One pointer token of a URI fragment, unescaped.
fn pointer_token(token: &str) -> String {
    percent_decode(token).replace("~1", "/").replace("~0", "~")
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
    Some(pointer_token(token))
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

    /// Wrapping a typeless nullable schema moves what it holds one level
    /// down; a reference that points into it has to follow.
    #[test]
    fn a_reference_into_a_wrapped_nullable_schema_still_resolves() {
        let schemas = schemas(
            Dialect::Oas30,
            json!({
                "Owner": {
                    "nullable": true,
                    "allOf": [{"type": "object", "required": ["id"]}]
                },
                "Pet": {
                    "type": "object",
                    "properties": {
                        "owner": {
                            "nullable": true,
                            "allOf": [{"type": "object", "required": ["name"]}]
                        }
                    }
                }
            }),
        );
        let direct = json!({"$ref": "#/components/schemas/Owner/allOf/0"});
        assert!(accepts(
            &schemas,
            Direction::Response,
            &direct,
            json!({"id": 1})
        ));
        assert!(!accepts(&schemas, Direction::Response, &direct, json!({})));
        assert!(
            !accepts(&schemas, Direction::Response, &direct, json!(null)),
            "the member is not nullable, only the schema around it"
        );
        let nested = json!({"$ref": "#/components/schemas/Pet/properties/owner/allOf/0"});
        assert!(accepts(
            &schemas,
            Direction::Response,
            &nested,
            json!({"name": "rex"})
        ));
        assert!(!accepts(&schemas, Direction::Response, &nested, json!({})));
        let whole = json!({"$ref": "#/components/schemas/Pet/properties/owner"});
        assert!(accepts(&schemas, Direction::Response, &whole, json!(null)));
        assert!(!accepts(&schemas, Direction::Response, &whole, json!({})));
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

    /// The usual composition: the properties come from one `allOf` member
    /// and the requirement from another.
    #[test]
    fn a_requirement_declared_beside_the_properties_it_names_is_relaxed() {
        let schemas = schemas(
            Dialect::Json2020,
            json!({
                "UserFields": {
                    "type": "object",
                    "properties": {
                        "email": {"type": "string"},
                        "password": {"type": "string", "writeOnly": true},
                        "id": {"type": "string", "readOnly": true}
                    }
                }
            }),
        );
        let schema = json!({
            "allOf": [
                {"$ref": "#/components/schemas/UserFields"},
                {"required": ["email", "password", "id"]}
            ]
        });
        // A response carries `id` but never the write-only password.
        assert!(accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"email": "a@b.c", "id": "u1"})
        ));
        // A request carries the password but not the read-only id.
        assert!(accepts(
            &schemas,
            Direction::Request,
            &schema,
            json!({"email": "a@b.c", "password": "s3cret"})
        ));
        // What the direction does carry is still required.
        assert!(!accepts(
            &schemas,
            Direction::Response,
            &schema,
            json!({"email": "a@b.c"})
        ));
        assert!(!accepts(
            &schemas,
            Direction::Request,
            &schema,
            json!({"email": "a@b.c"})
        ));
    }

    #[test]
    fn a_reference_into_a_component_is_followed_to_its_end() {
        let components = json!({
            "User": {
                "type": "object",
                "properties": {"secret": {"type": "string", "writeOnly": true}},
                "oneOf": [{"title": "a/b"}]
            }
        });
        let component = |name: &str| components.get(name);
        assert_eq!(
            follow("#/components/schemas/User/properties/secret", &component),
            Some(&json!({"type": "string", "writeOnly": true}))
        );
        assert_eq!(
            follow("#/components/schemas/User/oneOf/0/title", &component),
            Some(&json!("a/b"))
        );
        assert_eq!(
            follow("#/components/schemas/User", &component),
            components.get("User")
        );
        assert_eq!(
            follow("#/components/schemas/User/properties/none", &component),
            None
        );
        assert_eq!(follow("#/components/schemas/Other", &component), None);
        assert_eq!(
            follow("other.yaml#/components/schemas/User", &component),
            None
        );

        // A property declared by such a reference takes its annotation from
        // the property it points at, not from the component around it.
        let schemas = schemas(Dialect::Json2020, components.clone());
        let schema = json!({
            "type": "object",
            "required": ["secret"],
            "properties": {
                "secret": {"$ref": "#/components/schemas/User/properties/secret"}
            }
        });
        assert!(accepts(&schemas, Direction::Response, &schema, json!({})));
        assert!(!accepts(&schemas, Direction::Request, &schema, json!({})));
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
