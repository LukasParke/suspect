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
//!
//! What is checked, for a response ([`check_response`]):
//!
//! 1. the status is declared (exact code, then its `4XX` range, then
//!    `default`);
//! 2. required response headers are present and well-typed;
//! 3. the content type is one the response declares;
//! 4. a JSON body satisfies the schema of that media type, in the
//!    document's own schema dialect ([`Dialect`]) and direction
//!    ([`Direction`]).
//!
//! [`Conformance`] selects how much of that applies. Whatever could not be
//! checked is reported as such ([`Coverage`]), never as a pass.

#![deny(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

pub mod dialect;
pub mod request;
pub mod response;

pub use dialect::{Dialect, Direction};
pub use request::{Exchange, validate_request};
pub use response::{Conformance, Coverage, Reply, ResponseCheck, check_response};

/// What part of an exchange a [`Violation`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViolationKind {
    /// A value does not satisfy its schema (a body, or a parameter).
    #[default]
    Schema,
    /// The response status is not one the operation declares.
    Status,
    /// The response content type is not one the response declares.
    ContentType,
    /// A declared response header is missing or ill-typed.
    Header,
}

impl ViolationKind {
    /// The check name reports use for a response violation of this kind.
    #[must_use]
    pub fn response_check(self) -> &'static str {
        match self {
            Self::Schema => "response-schema",
            Self::Status => "response-status",
            Self::ContentType => "response-content-type",
            Self::Header => "response-header",
        }
    }
}

/// One contract violation, located by a JSON pointer into the offending
/// value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Violation {
    /// What is wrong, in the author's vocabulary.
    pub message: String,
    /// RFC 6901 pointer to the offending element; empty for the value
    /// itself.
    pub pointer: String,
    /// What part of the exchange is wrong.
    pub kind: ViolationKind,
}

impl Violation {
    /// Builds a schema violation.
    #[must_use]
    pub fn new(message: impl Into<String>, pointer: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            pointer: pointer.into(),
            kind: ViolationKind::Schema,
        }
    }

    /// The same violation, about a different part of the exchange.
    #[must_use]
    pub fn of_kind(mut self, kind: ViolationKind) -> Self {
        self.kind = kind;
        self
    }

    /// The message with its location: `/items/0/id: value has type
    /// integer, expected `string``. A violation of the whole value has no
    /// location to show.
    #[must_use]
    pub fn located(&self) -> String {
        if self.pointer.is_empty() || self.pointer == "/" {
            self.message.clone()
        } else {
            format!("{}: {}", self.pointer, self.message)
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.located())
    }
}

/// The component schemas a contract can resolve references into, and the
/// dialect they are written in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schemas {
    /// Component name → schema JSON.
    map: BTreeMap<String, serde_json::Value>,
    /// The dialect every schema of the contract is written in.
    dialect: Dialect,
}

impl Schemas {
    /// Builds a schema source from any map of schemas, preserving no
    /// order: component lookup is by name, so callers holding a
    /// `HashMap` need not convert first. The dialect is JSON Schema
    /// 2020-12; see [`Schemas::with_dialect`].
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
            dialect: Dialect::default(),
        }
    }

    /// The schemas of an IR snapshot — components and registered inline
    /// schemas alike — in the dialect its `openapi` version selects.
    #[must_use]
    pub fn from_spec(spec: &suspect_ir::IrSpec) -> Self {
        Self::from_any_map(
            spec.schemas
                .iter()
                .map(|schema| (&schema.name, &schema.json)),
        )
        .with_dialect(Dialect::of_openapi(&spec.openapi))
    }

    /// The same schemas, read in `dialect`.
    #[must_use]
    pub fn with_dialect(mut self, dialect: Dialect) -> Self {
        self.dialect = dialect;
        self
    }

    /// The dialect the schemas are written in.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The schema declared under `name`, when the map carries one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.map.get(name)
    }

    /// Validates `instance` against `schema` with no direction: `readOnly`
    /// and `writeOnly` relax nothing. Prefer [`Schemas::validate_as`] for
    /// a value that belongs to one side of an exchange.
    #[must_use]
    pub fn validate(
        &self,
        schema: &serde_json::Value,
        instance: &serde_json::Value,
    ) -> Vec<Violation> {
        self.validate_in(schema, instance, None)
    }

    /// Validates `instance` against `schema` as a value travelling in
    /// `direction`, resolving component references natively.
    ///
    /// A schema that cannot be compiled yields a violation rather than a
    /// silent pass: "we could not check this" must not read as "this is
    /// fine". Each violation's pointer locates the offending value inside
    /// `instance`.
    #[must_use]
    pub fn validate_as(
        &self,
        schema: &serde_json::Value,
        instance: &serde_json::Value,
        direction: Direction,
    ) -> Vec<Violation> {
        self.validate_in(schema, instance, Some(direction))
    }

    fn validate_in(
        &self,
        schema: &serde_json::Value,
        instance: &serde_json::Value,
        direction: Option<Direction>,
    ) -> Vec<Violation> {
        let component = |name: &str| self.map.get(name);
        let rewrite = |schema: &serde_json::Value| {
            dialect::normalize(schema, self.dialect, direction, &component)
        };
        // Only the components the schema can reach ride along: the wrapper
        // is parsed per validation, and a large description would otherwise
        // be re-read for every response.
        let components: serde_json::Map<String, serde_json::Value> = self
            .reachable(schema)
            .into_iter()
            .filter_map(|name| {
                self.map
                    .get(name)
                    .map(|value| (name.clone(), rewrite(value)))
            })
            .collect();
        // The components ride in the same document as the schema and the
        // instance, so the compiler's document-root fallback resolves
        // `#/components/schemas/...` — including recursive references.
        let wrapper = serde_json::json!({
            "components": {"schemas": components},
            "schema": rewrite(schema),
            "instance": instance,
        });
        let Some(uri) = suspect_source::Uri::parse("mem://contract-runtime.json").ok() else {
            return vec![Violation::new(
                "could not prepare the validation wrapper",
                "",
            )];
        };
        let wrapper = wrapper.to_string().into_bytes();
        let config = budget(wrapper.len());
        let document = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(wrapper));
        if !document.syntax_errors().is_empty() {
            return vec![Violation::new(
                "the schema or instance is not valid JSON",
                "",
            )];
        }
        let (Some(schema_node), Some(instance_node)) = (
            document.root().get("schema"),
            document.root().get("instance"),
        ) else {
            return vec![Violation::new("the validation wrapper is incomplete", "")];
        };
        let refs = suspect_schema::DocumentRefs::scan(document.root(), config.max_depth).ok();
        match suspect_schema::Compiler::new(config)
            .compile_with_document_root(schema_node, refs.as_ref())
        {
            Ok(compiled) => compiled
                .validate(instance_node)
                .iter()
                .map(|error| {
                    Violation::new(
                        error.message.clone(),
                        instance_pointer(&error.instance_path),
                    )
                })
                .collect(),
            Err(error) => vec![Violation::new(
                format!("the schema could not be compiled: {error}"),
                "",
            )],
        }
    }

    /// The component names `schema` can reach through local references.
    ///
    /// A reference the walk cannot follow by name (an anchor, a
    /// `$dynamicRef`, a relative URI) makes every component reachable, so
    /// the compiler sees the whole table it may need.
    fn reachable(&self, schema: &serde_json::Value) -> BTreeSet<&String> {
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        let mut queue: Vec<&serde_json::Value> = vec![schema];
        let mut opaque = false;
        while let Some(value) = queue.pop() {
            let mut names = Vec::new();
            collect_refs(value, &mut names, &mut opaque);
            for name in names {
                if let Some((key, target)) = self.map.get_key_value(&name)
                    && seen.insert(key)
                {
                    queue.push(target);
                }
            }
        }
        if opaque {
            return self.map.keys().collect();
        }
        seen
    }
}

/// The validation budgets for a document of `bytes` bytes.
///
/// The compiler's defaults bound work for small schema-authoring checks;
/// a response body is data of whatever size the API returns, and running
/// out of budget on one would report a conforming reply as a violation.
/// The work bounds therefore grow with the document, and stay bounds: a
/// validation still ends after work proportional to its input.
fn budget(bytes: usize) -> suspect_schema::Config {
    let work = 1_000_000usize.saturating_add(bytes.saturating_mul(64));
    suspect_schema::Config {
        max_evaluation_steps: work,
        max_equality_steps: work,
        ..suspect_schema::Config::default()
    }
}

/// Collects the component names referenced anywhere under `value`; sets
/// `opaque` when a reference is not a `#/components/schemas/...` pointer.
fn collect_refs(value: &serde_json::Value, names: &mut Vec<String>, opaque: &mut bool) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("$ref", serde_json::Value::String(reference)) => {
                        match dialect::component_name(reference) {
                            Some(name) => names.push(name),
                            None => *opaque = true,
                        }
                    }
                    ("$dynamicRef" | "$recursiveRef", _) => *opaque = true,
                    _ => collect_refs(child, names, opaque),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_refs(item, names, opaque);
            }
        }
        _ => {}
    }
}

/// An error's location inside the validated instance: the compiler reports
/// it relative to the wrapper document, where the instance sits under
/// `/instance`.
fn instance_pointer(path: &suspect_low::Pointer) -> String {
    let full = path.to_path();
    full.strip_prefix("/instance")
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
        .map_or(full.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{cookie_value, query_value, segment_for};
    use suspect_ir::{IrMedia, IrOperation, IrParameter, IrResponse, IrSpec, Method, ParamIn};

    /// The violations of a JSON reply, checked strictly.
    fn response_violations(
        responses: &[IrResponse],
        schemas: &Schemas,
        status: u16,
        body: &[u8],
    ) -> Vec<Violation> {
        check_response(
            responses,
            schemas,
            Reply {
                method: "GET",
                status,
                headers: &[],
                body,
            },
            Conformance::Strict,
        )
        .violations
    }

    /// A JSON response declaring `schema` for `status`.
    fn json_response(status: u16, schema: &str) -> IrResponse {
        IrResponse {
            status: Some(status),
            schema: Some(schema.to_owned()),
            content: vec![IrMedia {
                media_type: "application/json".to_owned(),
                schema: Some(schema.to_owned()),
                unresolved: None,
            }],
            ..IrResponse::default()
        }
    }

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
                    description: Some("ok".to_owned()),
                    ..json_response(200, "Pet")
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
    fn response_validation_selects_the_declared_status() {
        let spec = spec();
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let operation = spec.operations.first().unwrap();

        let ok = response_violations(&operation.responses, &schemas, 200, br#"{"name": "rex"}"#);
        assert!(ok.is_empty(), "{ok:?}");

        let missing_name = response_violations(&operation.responses, &schemas, 200, b"{}");
        assert!(
            !missing_name.is_empty(),
            "a missing required field is a finding"
        );

        // A status the operation does not declare is itself a finding.
        let undeclared = response_violations(&operation.responses, &schemas, 418, b"{}");
        assert_eq!(undeclared.len(), 1, "{undeclared:?}");
        assert_eq!(undeclared[0].kind, ViolationKind::Status);
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
        spec.operations[0].responses = vec![json_response(200, "Node")];
        let schemas = Schemas::from_any_map(spec.schemas.iter().map(|s| (&s.name, &s.json)));
        let operation = spec.operations.first().unwrap();

        let mut body = String::new();
        for level in 0..9 {
            body.push_str(&format!("{{\"name\": \"n{level}\", \"child\": "));
        }
        body.push_str("{\"name\": 7}");
        body.push_str(&"}".repeat(9));
        let violations = response_violations(&operation.responses, &schemas, 200, body.as_bytes());
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
        assert!(
            response_violations(&operation.responses, &schemas, 200, good.as_bytes()).is_empty()
        );
    }

    #[test]
    fn an_unknown_component_is_a_finding_not_a_pass() {
        let responses = vec![json_response(200, "Missing")];
        let violations = response_violations(&responses, &Schemas::default(), 200, b"{}");
        assert!(
            violations
                .iter()
                .any(|v| v.message.contains("not in the component map")),
            "\"we could not check this\" must not read as a pass: {violations:?}"
        );
    }

    /// A large but ordinary payload is decided, not abandoned: the
    /// evaluation budget is sized for real response bodies.
    #[test]
    fn a_large_payload_is_validated_to_the_end() {
        let schemas = Schemas::default();
        let schema = serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "required": ["id", "name", "tags"],
                "properties": {
                    "id": {"type": "integer", "minimum": 0},
                    "name": {"type": "string", "minLength": 1},
                    "tags": {"type": "array", "items": {"type": "string"}}
                }
            }
        });
        let item = |id: u64| serde_json::json!({"id": id, "name": "pet", "tags": ["a", "b", "c"]});
        let good = serde_json::Value::Array((0..20_000).map(item).collect());
        assert!(
            schemas.validate(&schema, &good).is_empty(),
            "20,000 conforming items conform"
        );
        // And a violation at the far end is still found.
        let mut bad: Vec<serde_json::Value> = (0..20_000).map(item).collect();
        bad[19_999]["name"] = serde_json::json!("");
        let violations = schemas.validate(&schema, &serde_json::Value::Array(bad));
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].pointer, "/19999/name");
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
