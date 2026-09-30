//! Response conformance: the declared response for a status, validated.

use crate::{Schemas, Violation, response_schema_for};

/// Validates a response against the operation's declared schema for its
/// status: the exact code first, then `default`.
///
/// A response with no declared schema conforms — there is nothing to
/// violate. A declared schema that fails to compile is a violation, not a
/// pass: "we could not check this" must not read as "this is fine".
#[must_use]
pub fn validate_response(
    responses: &[suspect_ir::IrResponse],
    schemas: &Schemas,
    status: u16,
    body: &[u8],
) -> Vec<Violation> {
    let Some(name) = response_schema_for(responses, status) else {
        return Vec::new();
    };
    // `response_schema_for` yields the component name when the response
    // declares one; resolve it through the shared schema source.
    schemas.get(name).map_or_else(
        || {
            // The name is not in the component map: the body cannot be
            // checked, which is a finding, not a pass.
            vec![Violation::new(
                format!("response schema `{name}` is not in the component map"),
                "/",
            )]
        },
        |schema| match body_is_json(body) {
            None => vec![Violation::new(
                "the response body is not valid JSON against a JSON schema",
                "/",
            )],
            Some(instance) => schemas.validate(schema, &instance),
        },
    )
}

/// Parses a body as JSON; `None` when it is not.
///
/// An empty body is JSON `null`, matching the convention the rest of the
/// toolchain uses for an absent payload.
fn body_is_json(body: &[u8]) -> Option<serde_json::Value> {
    if body.is_empty() {
        return Some(serde_json::Value::Null);
    }
    serde_json::from_slice(body).ok()
}
