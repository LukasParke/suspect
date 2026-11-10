//! Request conformance: parameters and body against a declared operation.

use suspect_ir::IrOperation;

use crate::{Direction, Schemas, Violation};

/// One inbound exchange, in the shape every caller already has.
#[derive(Debug, Clone, Copy)]
pub struct Exchange<'a> {
    /// Request path without the query string.
    pub path: &'a str,
    /// Raw query string, without the leading `?`.
    pub query: Option<&'a str>,
    /// Request headers in wire order.
    pub headers: &'a [(String, String)],
    /// Request body bytes.
    pub body: &'a [u8],
}

/// Validates a request against its operation: every declared parameter
/// (present, correctly typed, required ones present) and the request body
/// against the declared schema.
///
/// Returns an empty vector when the request conforms. Every violation is
/// reported, not just the first, so a client fixing one problem does not
/// discover the next only after re-running.
#[must_use]
pub fn validate_request(
    operation: &IrOperation,
    schemas: &Schemas,
    exchange: Exchange<'_>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    for parameter in &operation.parameters {
        let Some(schema) = parameter.schema.as_ref() else {
            continue;
        };
        let pointer = format!("/{}", parameter.name);
        match parameter.location {
            suspect_ir::ParamIn::Query => match query_value(exchange.query, &parameter.name) {
                Some(raw) => check_scalar(
                    &raw,
                    schema,
                    schemas,
                    &pointer,
                    Direction::Request,
                    &mut out,
                ),
                None if parameter.required => out.push(Violation::new(
                    format!("missing required query parameter `{}`", parameter.name),
                    &pointer,
                )),
                None => {}
            },
            suspect_ir::ParamIn::Header => {
                let found = exchange
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(&parameter.name));
                match found {
                    Some((_, value)) => {
                        check_scalar(
                            value,
                            schema,
                            schemas,
                            &pointer,
                            Direction::Request,
                            &mut out,
                        );
                    }
                    None if parameter.required => out.push(Violation::new(
                        format!("missing required header `{}`", parameter.name),
                        &pointer,
                    )),
                    None => {}
                }
            }
            suspect_ir::ParamIn::Path => {
                // A templated path segment, resolved through the operation's
                // template so the concrete value can be checked.
                if let Some(raw) = segment_for(&operation.path, exchange.path, &parameter.name) {
                    check_scalar(
                        &raw,
                        schema,
                        schemas,
                        &pointer,
                        Direction::Request,
                        &mut out,
                    );
                } else if parameter.required {
                    out.push(Violation::new(
                        format!("missing required path parameter `{}`", parameter.name),
                        &pointer,
                    ));
                }
            }
            suspect_ir::ParamIn::Cookie => {
                let found = exchange
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("cookie"));
                match found.and_then(|(_, value)| cookie_value(value, &parameter.name)) {
                    Some(raw) => check_scalar(
                        &raw,
                        schema,
                        schemas,
                        &pointer,
                        Direction::Request,
                        &mut out,
                    ),
                    None if parameter.required => out.push(Violation::new(
                        format!("missing required cookie `{}`", parameter.name),
                        &pointer,
                    )),
                    None => {}
                }
            }
        }
    }

    // Body: only when the operation declares one and bytes arrived.
    if let Some(schema_name) = &operation.body_schema
        && !exchange.body.is_empty()
    {
        match serde_json::from_slice::<serde_json::Value>(exchange.body) {
            Ok(value) => match schemas.get(schema_name) {
                Some(schema) => {
                    out.extend(schemas.validate_as(schema, &value, Direction::Request));
                }
                None => out.push(Violation::new(
                    format!("request body schema `{schema_name}` is not in the component map"),
                    "/",
                )),
            },
            Err(error) => out.push(Violation::new(
                format!("the request body is not valid JSON: {error}"),
                "/",
            )),
        }
    }
    out
}

/// A query parameter's raw value.
#[must_use]
pub fn query_value(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=')
            && percent_decode(key) == name
        {
            return Some(percent_decode(value));
        }
    }
    None
}

/// A cookie's value from a `Cookie` header.
#[must_use]
pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

/// The segment substituted for `{name}` given the operation's template.
#[must_use]
pub fn segment_for(template: &str, path: &str, name: &str) -> Option<String> {
    let template: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
    let actual: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if template.len() != actual.len() {
        return None;
    }
    template
        .iter()
        .zip(&actual)
        .find(|(expected, _)| **expected == format!("{{{name}}}"))
        .map(|(_, got)| percent_decode(got))
}

/// Coerces a raw string to the JSON type its schema declares, then
/// validates it. A value that will not coerce is reported as a type
/// violation against the same schema, which is the honest reading.
///
/// An `array` or `object` schema is not checked: how such a value is
/// serialized into one string depends on the parameter's `style` and
/// `explode`, which this runtime does not decode, and reading the raw text
/// as a string would report a type error for a well-formed value.
pub(crate) fn check_scalar(
    raw: &str,
    schema: &serde_json::Value,
    schemas: &Schemas,
    pointer: &str,
    direction: Direction,
    out: &mut Vec<Violation>,
) {
    let value = match declared_type(schema, schemas).as_deref() {
        Some("array" | "object") => return,
        Some("integer") => raw.parse::<i64>().map_or_else(
            |_| serde_json::Value::String(raw.to_owned()),
            |number| serde_json::json!(number),
        ),
        Some("number") => raw.parse::<f64>().map_or_else(
            |_| serde_json::Value::String(raw.to_owned()),
            |number| serde_json::json!(number),
        ),
        Some("boolean") => match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => serde_json::Value::String(raw.to_owned()),
        },
        _ => serde_json::Value::String(raw.to_owned()),
    };
    for violation in schemas.validate_as(schema, &value, direction) {
        out.push(Violation {
            pointer: pointer.to_owned(),
            ..violation
        });
    }
}

/// The JSON type a schema declares for coercion: its `type` (the first
/// non-null member of a type array), found through component references
/// with a hop bound so a self-referential schema cannot loop.
fn declared_type(schema: &serde_json::Value, schemas: &Schemas) -> Option<String> {
    const MAX_HOPS: usize = 8;
    let mut current = schema;
    for _ in 0..MAX_HOPS {
        match current.get("type") {
            Some(serde_json::Value::String(name)) => return Some(name.clone()),
            Some(serde_json::Value::Array(names)) => {
                return names
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .find(|name| *name != "null")
                    .map(str::to_owned);
            }
            _ => {}
        }
        current = current
            .get("$ref")
            .and_then(serde_json::Value::as_str)
            .and_then(crate::dialect::component_name)
            .and_then(|name| schemas.get(&name))?;
    }
    None
}

/// Percent-decoding for a URI component (path segment, header): `+` is a
/// literal plus, because form encoding does not apply outside a query
/// string.
#[must_use]
pub fn percent_decode(text: &str) -> String {
    decode(text, false)
}

/// Percent-decoding for a form-encoded query component, where `+` means
/// a space.
#[must_use]
pub fn percent_decode_form(text: &str) -> String {
    decode(text, true)
}

fn decode(text: &str, plus_is_space: bool) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        if bytes[index] == b'+' && plus_is_space {
            out.push(b' ');
        } else {
            out.push(bytes[index]);
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ir::{IrOperation, IrParameter, Method, ParamIn};

    fn operation(parameters: Vec<IrParameter>, body_schema: Option<&str>) -> IrOperation {
        IrOperation {
            id: Some("op".to_owned()),
            method: Method::Post,
            path: "/pets".to_owned(),
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            parameters,
            body_schema: body_schema.map(str::to_owned),
            responses: Vec::new(),
        }
    }

    fn query(name: &str, schema: serde_json::Value) -> IrParameter {
        IrParameter {
            name: name.to_owned(),
            location: ParamIn::Query,
            required: false,
            schema: Some(schema),
        }
    }

    fn schemas(components: serde_json::Value) -> Schemas {
        let map: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_value(components).unwrap();
        Schemas::from_any_map(map.iter())
    }

    fn violations(
        operation: &IrOperation,
        schemas: &Schemas,
        query: &str,
        body: &str,
    ) -> Vec<Violation> {
        validate_request(
            operation,
            schemas,
            Exchange {
                path: "/pets",
                query: Some(query),
                headers: &[],
                body: body.as_bytes(),
            },
        )
    }

    #[test]
    fn a_parameter_typed_by_reference_is_coerced_to_that_type() {
        let schemas = schemas(serde_json::json!({"Limit": {"type": "integer", "maximum": 50}}));
        let operation = operation(
            vec![query(
                "limit",
                serde_json::json!({"$ref": "#/components/schemas/Limit"}),
            )],
            None,
        );
        assert!(violations(&operation, &schemas, "limit=10", "").is_empty());
        let over = violations(&operation, &schemas, "limit=99", "");
        assert_eq!(over.len(), 1, "{over:?}");
        assert_eq!(over[0].pointer, "/limit");
    }

    #[test]
    fn a_serialized_array_parameter_is_not_misread_as_a_string() {
        let operation = operation(
            vec![query(
                "ids",
                serde_json::json!({"type": "array", "items": {"type": "integer"}}),
            )],
            None,
        );
        assert!(violations(&operation, &Schemas::default(), "ids=1,2,3", "").is_empty());
    }

    #[test]
    fn a_request_body_may_omit_a_required_read_only_property() {
        let schemas = schemas(serde_json::json!({
            "Pet": {
                "type": "object",
                "required": ["id", "name"],
                "properties": {
                    "id": {"type": "string", "readOnly": true},
                    "name": {"type": "string"}
                }
            }
        }));
        let operation = operation(Vec::new(), Some("Pet"));
        assert!(violations(&operation, &schemas, "", r#"{"name": "rex"}"#).is_empty());
        let missing_name = violations(&operation, &schemas, "", r#"{"id": "p1"}"#);
        assert_eq!(missing_name.len(), 1, "{missing_name:?}");
    }
}
