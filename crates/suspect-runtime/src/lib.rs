//! The shared contract runtime: one interpretation of a contract, used by
//! every surface that has to make a pass/fail decision about it.
//!
//! Before this crate existed, three components each carried their own
//! interpretation:
//!
//! - the gateway hand-rolled parameter and body checking against
//!   `IrOperation` with its own `$ref` walker,
//! - the Arazzo executor validated responses with its own inline wrapper,
//! - documentation playgrounds had no validation at all.
//!
//! They agreed on the easy cases and diverged on the hard ones (recursive
//! schemas, `default` responses, exact numeric types). This crate owns the
//! decision: given an operation and an exchange, does it conform? The
//! gateway and the executor both call here, so "the toolchain agrees with
//! itself" is a property rather than a hope.
//!
//! Component references resolve through the schema compiler's
//! document-root fallback, so recursive and cross-document schemas are
//! validated natively rather than through a depth-capped inliner.

#![deny(missing_docs)]

use std::collections::BTreeMap;

use suspect_ir::IrResponse;

pub mod request;
pub mod response;

pub use request::{Exchange, validate_request};
pub use response::validate_response;

/// One contract violation, located by a JSON pointer into the offending
/// value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Violation {
    /// What is wrong, in the author's vocabulary.
    pub message: String,
    /// RFC 6901 pointer to the offending element.
    pub pointer: String,
}

impl Violation {
    /// Builds a violation.
    #[must_use]
    pub fn new(message: impl Into<String>, pointer: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            pointer: pointer.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.message, self.pointer)
    }
}

/// The component schemas a contract can resolve references into.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schemas {
    /// Component name → schema JSON.
    map: BTreeMap<String, serde_json::Value>,
}

impl Schemas {
    /// Builds a schema source from any map of schemas, preserving no
    /// order: component lookup is by name, so callers holding a
    /// `HashMap` need not convert first.
    #[must_use]
    pub fn from_any_map<'a, I>(map: I) -> Self
    where
        I: IntoIterator<Item = (&'a String, &'a serde_json::Value)>,
    {
        Self {
            map: map
                .into_iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        }
    }

    /// The schema declared under `name`, when the map carries one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.map
            .iter()
            .find(|(candidate, _)| candidate.as_str() == name)
            .map(|(_, value)| value)
    }

    /// Validates `instance` against `schema`, resolving component
    /// references natively.
    ///
    /// The schema and instance share a synthetic document so the compiler
    /// resolves `#/components/schemas/...` through the document-root
    /// fallback. A schema that cannot be compiled yields a violation
    /// rather than a silent pass: "we could not check this" must not read
    /// as "this is fine".
    #[must_use]
    pub fn validate(
        &self,
        schema: &serde_json::Value,
        instance: &serde_json::Value,
    ) -> Vec<Violation> {
        if self.map.is_empty() {
            return validate_without_refs(schema, instance);
        }
        // The components ride in the same document as the schema and the
        // instance, so the compiler's document-root fallback resolves
        // `#/components/schemas/...` — including recursive references.
        let wrapper = serde_json::json!({
            "components": {"schemas": self.map},
            "schema": schema,
            "instance": instance,
        });
        let Some(uri) = suspect_source::Uri::parse("mem://contract-runtime.json").ok() else {
            return vec![Violation::new(
                "could not prepare the validation wrapper",
                "/",
            )];
        };
        let document = suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(wrapper.to_string().into_bytes()),
        );
        if !document.syntax_errors().is_empty() {
            return vec![Violation::new(
                "the schema or instance is not valid JSON",
                "/",
            )];
        }
        let (Some(schema_node), Some(instance_node)) = (
            document.root().get("schema"),
            document.root().get("instance"),
        ) else {
            return vec![Violation::new("the validation wrapper is incomplete", "/")];
        };
        let refs = suspect_schema::DocumentRefs::scan(
            document.root(),
            suspect_schema::Config::default().max_depth,
        )
        .ok();
        match suspect_schema::Compiler::new(suspect_schema::Config::default())
            .compile_with_document_root(schema_node, refs.as_ref())
        {
            Ok(compiled) => compiled
                .validate(instance_node)
                .iter()
                .map(|error| Violation::new(error.message.clone(), String::new()))
                .collect(),
            Err(error) => vec![Violation::new(
                format!("the schema could not be compiled: {error}"),
                "/",
            )],
        }
    }
}

/// Validation for a document with no components to resolve into.
fn validate_without_refs(
    schema: &serde_json::Value,
    instance: &serde_json::Value,
) -> Vec<Violation> {
    let wrapper = serde_json::json!({"schema": schema, "instance": instance});
    let Ok(uri) = suspect_source::Uri::parse("mem://contract-runtime-standalone.json") else {
        return vec![Violation::new(
            "could not prepare the validation wrapper",
            "/",
        )];
    };
    let document = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(wrapper.to_string().into_bytes()),
    );
    let (Some(schema_node), Some(instance_node)) = (
        document.root().get("schema"),
        document.root().get("instance"),
    ) else {
        return vec![Violation::new("the validation wrapper is incomplete", "/")];
    };
    match suspect_schema::Compiler::new(suspect_schema::Config::default()).compile(schema_node) {
        Ok(compiled) => compiled
            .validate(instance_node)
            .iter()
            .map(|error| Violation::new(error.message.clone(), String::new()))
            .collect(),
        Err(error) => vec![Violation::new(
            format!("the schema could not be compiled: {error}"),
            "/",
        )],
    }
}

/// Selects the response schema *name* for a status: exact code first, then
/// `default`, per the OpenAPI response selection rule.
#[must_use]
pub fn response_schema_for(responses: &[IrResponse], status: u16) -> Option<&str> {
    responses
        .iter()
        .find(|response| response.status == Some(status))
        .or_else(|| responses.iter().find(|response| response.status.is_none()))
        .and_then(|response| response.schema.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{cookie_value, query_value, segment_for};
    use suspect_ir::{IrOperation, IrParameter, IrSpec, Method, ParamIn};

    fn spec() -> IrSpec {
        IrSpec {
            title: "Runtime".to_owned(),
            version: "1".to_owned(),
            operations: vec![IrOperation {
                id: Some("getPet".to_owned()),
                method: Method::Get,
                path: "/pets/{petId}".to_owned(),
                summary: None,
                description: None,
                tags: Vec::new(),
                deprecated: false,
                parameters: vec![
                    IrParameter {
                        name: "petId".to_owned(),
                        location: ParamIn::Path,
                        required: true,
                        schema: Some(serde_json::json!({"type": "string"})),
                    },
                    IrParameter {
                        name: "limit".to_owned(),
                        location: ParamIn::Query,
                        required: true,
                        schema: Some(serde_json::json!({"type": "integer"})),
                    },
                    IrParameter {
                        name: "trace".to_owned(),
                        location: ParamIn::Header,
                        required: false,
                        schema: Some(serde_json::json!({"type": "boolean"})),
                    },
                ],
                body_schema: None,
                responses: vec![IrResponse {
                    status: Some(200),
                    description: Some("ok".to_owned()),
                    schema: Some("Pet".to_owned()),
                }],
            }],
            schemas: vec![suspect_ir::IrSchema {
                name: "Pet".to_owned(),
                json: serde_json::json!({
                    "type": "object",
                    "required": ["name"],
                    "properties": {"name": {"type": "string"}}
                }),
            }],
            ..IrSpec::default()
        }
    }

    #[test]
    fn a_conforming_request_has_no_violations() {
        let spec = spec();
        let operation = spec.operations.first().unwrap();
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let headers = vec![("trace".to_owned(), "true".to_owned())];
        let violations = validate_request(
            operation,
            &schemas,
            Exchange {
                path: "/pets/42",
                query: Some("limit=10"),
                headers: &headers,
                body: &[],
            },
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn missing_and_mistyped_parameters_are_reported() {
        let spec = spec();
        let operation = spec.operations.first().unwrap();
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let violations = validate_request(
            operation,
            &schemas,
            Exchange {
                path: "/pets/42",
                // `limit` present but not an integer, `trace` wrong type.
                query: Some("limit=ten"),
                headers: &[("trace".to_owned(), "yes".to_owned())],
                body: &[],
            },
        );
        assert!(
            violations.iter().any(|v| v.pointer == "/limit"),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.pointer == "/trace"),
            "{violations:?}"
        );

        // A missing required parameter is its own finding.
        let violations = validate_request(
            operation,
            &schemas,
            Exchange {
                path: "/pets/42",
                query: None,
                headers: &[],
                body: &[],
            },
        );
        assert!(
            violations
                .iter()
                .any(|v| v.message.contains("missing required query parameter")),
            "{violations:?}"
        );
    }

    #[test]
    fn response_validation_selects_exact_then_default() {
        let spec = spec();
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let operation = spec.operations.first().unwrap();

        let ok = validate_response(&operation.responses, &schemas, 200, br#"{"name": "rex"}"#);
        assert!(ok.is_empty(), "{ok:?}");

        let missing_name = validate_response(&operation.responses, &schemas, 200, b"{}");
        assert!(
            !missing_name.is_empty(),
            "a missing required field is a finding"
        );

        // An undeclared status with no `default` has nothing to check.
        let undeclared = validate_response(&operation.responses, &schemas, 418, b"{}");
        assert!(undeclared.is_empty());
    }

    #[test]
    fn a_recursive_response_schema_validates_at_depth() {
        // The whole point of the shared runtime: a recursive component
        // graph is validated natively, not through a depth-capped inliner.
        let mut spec = spec();
        spec.schemas = vec![suspect_ir::IrSchema {
            name: "Node".to_owned(),
            json: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "child": {"$ref": "#/components/schemas/Node"}
                }
            }),
        }];
        spec.operations[0].responses[0].schema = Some("Node".to_owned());
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let operation = spec.operations.first().unwrap();

        let mut body = String::new();
        for level in 0..9 {
            body.push_str(&format!("{{\"name\": \"n{level}\", \"child\": "));
        }
        body.push_str("{\"name\": 7}");
        body.push_str(&"}".repeat(9));
        let violations = validate_response(&operation.responses, &schemas, 200, body.as_bytes());
        assert!(
            !violations.is_empty(),
            "a violation nine levels deep must be found"
        );

        let mut good = String::new();
        for level in 0..9 {
            good.push_str(&format!("{{\"name\": \"n{level}\", \"child\": "));
        }
        good.push_str("{\"name\": \"leaf\"}");
        good.push_str(&"}".repeat(9));
        assert!(validate_response(&operation.responses, &schemas, 200, good.as_bytes()).is_empty());
    }

    #[test]
    fn an_unknown_component_is_a_finding_not_a_pass() {
        let responses = vec![IrResponse {
            status: Some(200),
            description: None,
            schema: Some("Missing".to_owned()),
        }];
        let violations = validate_response(&responses, &Schemas::default(), 200, b"{}");
        assert!(
            violations
                .iter()
                .any(|v| v.message.contains("not in the component map")),
            "\"we could not check this\" must not read as a pass: {violations:?}"
        );
    }

    #[test]
    fn query_and_cookie_values_are_extracted() {
        assert_eq!(
            query_value(Some("a=1&limit=10"), "limit").as_deref(),
            Some("10")
        );
        assert_eq!(
            query_value(Some("limit=ten"), "limit").as_deref(),
            Some("ten")
        );
        assert_eq!(query_value(Some("a=1"), "limit"), None);
        assert_eq!(
            cookie_value("a=1; petId=42", "petId").as_deref(),
            Some("42")
        );
    }

    #[test]
    fn path_segments_resolve_through_the_template() {
        assert_eq!(
            segment_for("/pets/{petId}", "/pets/42", "petId").as_deref(),
            Some("42")
        );
        assert_eq!(segment_for("/pets/{petId}", "/pets/42", "other"), None);
    }
}
