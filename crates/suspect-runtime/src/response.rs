//! Response conformance: is this reply one the operation declares?

use suspect_ir::{IrMedia, IrResponse, is_json_media_type, json_media, media_essence};

use crate::{Direction, Schemas, Violation, ViolationKind};

/// How much of the response contract a check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conformance {
    /// The whole contract: the status and content type must be declared,
    /// required headers present, and the body must satisfy its schema.
    #[default]
    Strict,
    /// Only the body schema of a declared response. A status or content
    /// type the description does not mention is not a finding. For
    /// descriptions that document the success path only.
    Schema,
    /// Nothing: responses are judged by the caller's own criteria alone.
    Off,
}

impl Conformance {
    /// Every level, strictest first.
    pub const ALL: [Self; 3] = [Self::Strict, Self::Schema, Self::Off];

    /// The level's configuration spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Schema => "schema",
            Self::Off => "off",
        }
    }

    /// Parses a configuration spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == text)
    }
}

impl std::fmt::Display for Conformance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A response as it arrived.
#[derive(Debug, Clone, Copy)]
pub struct Reply<'a> {
    /// The request method; a `HEAD` reply carries no body to check.
    pub method: &'a str,
    /// Response status code.
    pub status: u16,
    /// Response headers in wire order.
    pub headers: &'a [(String, String)],
    /// Response body bytes.
    pub body: &'a [u8],
}

/// How far a check could go. Only [`Coverage::Schema`] means a body was
/// held to a schema; the others say why it was not, so a caller never
/// reports "matches the contract" for a body nothing looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// The body was validated against the schema declared for its status
    /// and content type.
    Schema,
    /// The response is declared, but there is no body schema to apply: no
    /// content, no schema, a media type that is not JSON, or a reply that
    /// carries no body (`HEAD`, `204`, `304`).
    Declared,
    /// The operation declares nothing for this status.
    Undeclared,
    /// Checking is off.
    Skipped,
}

/// The outcome of [`check_response`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseCheck {
    /// Everything the reply gets wrong; empty when it conforms.
    pub violations: Vec<Violation>,
    /// How far the check could go.
    pub coverage: Coverage,
}

impl ResponseCheck {
    /// Whether no violation was found.
    #[must_use]
    pub fn conforms(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks a reply against the responses an operation declares.
///
/// The governing response is the exact status, else its range (`4XX`),
/// else `default`. Under [`Conformance::Strict`] a status with none of
/// those is a violation, required headers must be present and well-typed,
/// and the content type must be one the response declares. At every level
/// but [`Conformance::Off`] a JSON body must satisfy the schema of its
/// media type, read in the description's own dialect; a schema that cannot
/// be resolved or compiled is a violation, not a pass.
#[must_use]
pub fn check_response(
    responses: &[IrResponse],
    schemas: &Schemas,
    reply: Reply<'_>,
    level: Conformance,
) -> ResponseCheck {
    if level == Conformance::Off {
        return ResponseCheck {
            violations: Vec::new(),
            coverage: Coverage::Skipped,
        };
    }
    let strict = level == Conformance::Strict;
    let Some(declared) = suspect_ir::response_for(responses, reply.status) else {
        let mut violations = Vec::new();
        if strict && !responses.is_empty() {
            violations.push(
                Violation::new(
                    format!(
                        "status {} is not declared for this operation (declared: {})",
                        reply.status,
                        keys(responses.iter().map(IrResponse::key))
                    ),
                    "",
                )
                .of_kind(ViolationKind::Status),
            );
        }
        return ResponseCheck {
            violations,
            coverage: Coverage::Undeclared,
        };
    };
    let mut violations = Vec::new();
    if strict {
        check_headers(declared, schemas, reply.headers, &mut violations);
    }
    let coverage = check_body(declared, schemas, &reply, strict, &mut violations);
    ResponseCheck {
        violations,
        coverage,
    }
}

/// A comma-separated list for a message.
fn keys(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}

/// The value of the first header named `name`, compared case-insensitively.
fn header<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Whether the reply cannot carry a body: a `HEAD` exchange, or a status
/// HTTP defines as bodiless.
fn bodiless(reply: &Reply<'_>) -> bool {
    reply.method.eq_ignore_ascii_case("HEAD") || matches!(reply.status, 100..=199 | 204 | 304)
}

/// Required headers are present; present headers satisfy their schema.
fn check_headers(
    declared: &IrResponse,
    schemas: &Schemas,
    headers: &[(String, String)],
    out: &mut Vec<Violation>,
) {
    for expected in &declared.headers {
        // OpenAPI: a response header named `Content-Type` is ignored; the
        // content map describes it.
        if expected.name.eq_ignore_ascii_case("content-type") {
            continue;
        }
        let pointer = format!("/{}", expected.name);
        match header(headers, &expected.name) {
            None if expected.required => out.push(
                Violation::new(
                    format!("required response header `{}` is missing", expected.name),
                    pointer,
                )
                .of_kind(ViolationKind::Header),
            ),
            None => {}
            Some(raw) => {
                if let Some(schema) = &expected.schema {
                    let before = out.len();
                    crate::request::check_scalar(
                        raw,
                        schema,
                        schemas,
                        &pointer,
                        Direction::Response,
                        out,
                    );
                    for violation in &mut out[before..] {
                        violation.message =
                            format!("response header `{}`: {}", expected.name, violation.message);
                        violation.kind = ViolationKind::Header;
                    }
                }
            }
        }
    }
}

/// The declared media type that governs an actual content type: the exact
/// type, else its `type/*` range, else `*/*`.
fn declared_media<'c>(content: &'c [IrMedia], essence: &str) -> Option<&'c IrMedia> {
    let range = essence.split_once('/').map(|(kind, _)| format!("{kind}/*"));
    let find = |wanted: &str| {
        content
            .iter()
            .find(|media| media_essence(&media.media_type) == wanted)
    };
    find(essence)
        .or_else(|| range.as_deref().and_then(find))
        .or_else(|| find("*/*"))
}

/// Holds the body to the schema of its media type and reports how far the
/// check could go.
fn check_body(
    declared: &IrResponse,
    schemas: &Schemas,
    reply: &Reply<'_>,
    strict: bool,
    out: &mut Vec<Violation>,
) -> Coverage {
    if bodiless(reply) || declared.content.is_empty() {
        return Coverage::Declared;
    }
    let actual = header(reply.headers, "content-type")
        .map(media_essence)
        .filter(|essence| !essence.is_empty());
    // Without a content type to go by — no header, or (below strict) a
    // type the response does not declare — the body is read as JSON and
    // held to the JSON media type, the one a JSON API means.
    let (media, assumed_json) = match actual
        .as_deref()
        .map(|essence| (essence, declared_media(&declared.content, essence)))
    {
        Some((_, Some(media))) => (media, false),
        Some((essence, None)) if strict => {
            out.push(
                Violation::new(
                    format!(
                        "content type `{essence}` is not declared for status {} (declared: {})",
                        declared.key(),
                        keys(
                            declared
                                .content
                                .iter()
                                .map(|media| media.media_type.clone())
                        )
                    ),
                    "",
                )
                .of_kind(ViolationKind::ContentType),
            );
            return Coverage::Declared;
        }
        Some((_, None)) | None => match json_media(&declared.content) {
            Some(media) => (media, true),
            None => return Coverage::Declared,
        },
    };
    if let Some(reference) = &media.unresolved {
        out.push(Violation::new(
            format!(
                "the schema of `{}` is `{reference}`, a reference that could not be loaded, \
                 so the body cannot be checked against it",
                media.media_type
            ),
            "",
        ));
        return Coverage::Declared;
    }
    let Some(name) = &media.schema else {
        return Coverage::Declared;
    };
    let json = assumed_json || actual.as_deref().is_some_and(is_json_media_type);
    if !json {
        return Coverage::Declared;
    }
    match parse_json(reply.body) {
        None => out.push(Violation::new("the response body is not valid JSON", "")),
        Some(instance) => match schemas.get(name) {
            // The name is not in the component map: the body cannot be
            // checked, which is a finding, not a pass.
            None => out.push(Violation::new(
                format!("response schema `{name}` is not in the component map"),
                "",
            )),
            Some(schema) => {
                out.extend(schemas.validate_as(schema, &instance, Direction::Response));
            }
        },
    }
    Coverage::Schema
}

/// Parses a body as JSON; `None` when it is not.
///
/// An empty body is JSON `null`, matching the convention the rest of the
/// toolchain uses for an absent payload.
fn parse_json(body: &[u8]) -> Option<serde_json::Value> {
    if body.is_empty() {
        return Some(serde_json::Value::Null);
    }
    serde_json::from_slice(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ir::{IrSpec, OpSelector};

    const SPEC: &str = r##"
openapi: 3.1.0
info: { title: Pets, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: a page, declared inline
          headers:
            X-Total:
              required: true
              schema: { type: integer }
          content:
            application/json:
              schema:
                type: object
                required: [items]
                properties:
                  items:
                    type: array
                    items: { $ref: '#/components/schemas/Pet' }
        '404':
          $ref: '#/components/responses/NotFound'
        5XX:
          description: server trouble
          content:
            application/problem+json:
              schema: { $ref: '#/components/schemas/Problem' }
    head:
      operationId: probePets
      responses:
        '200':
          description: exists
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
  /export:
    get:
      operationId: export
      responses:
        '200':
          description: a file
          content:
            text/csv:
              schema: { type: string }
            application/*:
              schema: { $ref: '#/components/schemas/Pet' }
        default:
          description: anything else
          content:
            '*/*':
              schema: { $ref: '#/components/schemas/Problem' }
  /external:
    get:
      operationId: external
      responses:
        '200':
          description: declared in another file
          content:
            application/json:
              schema: { $ref: './shared.yaml#/Pet' }
  /silent:
    delete:
      operationId: silent
      responses:
        '204': { description: gone }
  /undocumented:
    get:
      operationId: undocumented
components:
  responses:
    NotFound:
      description: no such pet
      content:
        application/problem+json:
          schema: { $ref: '#/components/schemas/Problem' }
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string }
"##;

    fn spec_of(yaml: &str) -> IrSpec {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spec.yaml");
        std::fs::write(&path, yaml).unwrap();
        IrSpec::from_file(&path).expect("an OpenAPI document")
    }

    struct Fixture {
        spec: IrSpec,
        schemas: Schemas,
    }

    fn fixture() -> Fixture {
        let spec = spec_of(SPEC);
        let schemas = Schemas::from_spec(&spec);
        Fixture { spec, schemas }
    }

    impl Fixture {
        fn check(
            &self,
            operation: &str,
            status: u16,
            content_type: Option<&str>,
            body: &str,
            level: Conformance,
        ) -> ResponseCheck {
            let mut headers = vec![("X-Total".to_owned(), "1".to_owned())];
            if let Some(content_type) = content_type {
                headers.push(("Content-Type".to_owned(), content_type.to_owned()));
            }
            self.check_with(operation, status, &headers, body, level)
        }

        fn check_with(
            &self,
            operation: &str,
            status: u16,
            headers: &[(String, String)],
            body: &str,
            level: Conformance,
        ) -> ResponseCheck {
            let operation = self
                .spec
                .operation(OpSelector::Id(operation))
                .expect("operation");
            check_response(
                &operation.responses,
                &self.schemas,
                Reply {
                    method: operation.method.as_str(),
                    status,
                    headers,
                    body: body.as_bytes(),
                },
                level,
            )
        }
    }

    const JSON: Option<&str> = Some("application/json");
    const GOOD_PAGE: &str = r#"{"items": [{"id": "p1", "name": "Rex"}]}"#;

    #[test]
    fn a_conforming_inline_response_is_schema_checked() {
        let check = fixture().check("listPets", 200, JSON, GOOD_PAGE, Conformance::Strict);
        assert!(check.conforms(), "{:?}", check.violations);
        assert_eq!(check.coverage, Coverage::Schema);
    }

    #[test]
    fn an_inline_response_schema_is_enforced() {
        let check = fixture().check(
            "listPets",
            200,
            JSON,
            r#"{"items": [{"id": 42}]}"#,
            Conformance::Strict,
        );
        assert_eq!(check.coverage, Coverage::Schema);
        let messages: Vec<String> = check.violations.iter().map(Violation::located).collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("/items/0/id") && m.contains("integer")),
            "the wrong type is located: {messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("name")),
            "the missing property is reported: {messages:?}"
        );
        assert!(
            check
                .violations
                .iter()
                .all(|v| v.kind == ViolationKind::Schema)
        );
    }

    #[test]
    fn an_undeclared_status_is_a_violation_only_when_strict() {
        let fixture = fixture();
        let strict = fixture.check("listPets", 418, JSON, "{}", Conformance::Strict);
        assert_eq!(strict.coverage, Coverage::Undeclared);
        assert_eq!(strict.violations.len(), 1, "{:?}", strict.violations);
        assert_eq!(strict.violations[0].kind, ViolationKind::Status);
        assert!(
            strict.violations[0]
                .message
                .contains("status 418 is not declared")
                && strict.violations[0].message.contains("200, 404, 5XX"),
            "{}",
            strict.violations[0].message
        );

        let schema_only = fixture.check("listPets", 418, JSON, "{}", Conformance::Schema);
        assert!(schema_only.conforms());
        assert_eq!(schema_only.coverage, Coverage::Undeclared);
    }

    #[test]
    fn a_status_range_declares_its_statuses() {
        let fixture = fixture();
        let problem = Some("application/problem+json");
        let ok = fixture.check(
            "listPets",
            503,
            problem,
            r#"{"title": "down"}"#,
            Conformance::Strict,
        );
        assert!(ok.conforms(), "{:?}", ok.violations);
        assert_eq!(ok.coverage, Coverage::Schema);
        let bad = fixture.check("listPets", 503, problem, "{}", Conformance::Strict);
        assert!(!bad.conforms(), "the 5XX schema is enforced");
    }

    #[test]
    fn a_shared_component_response_is_enforced() {
        let fixture = fixture();
        let bad = fixture.check(
            "listPets",
            404,
            Some("application/problem+json"),
            r#"{"detail": "gone"}"#,
            Conformance::Strict,
        );
        assert_eq!(bad.coverage, Coverage::Schema);
        assert!(
            bad.violations.iter().any(|v| v.message.contains("title")),
            "{:?}",
            bad.violations
        );
    }

    #[test]
    fn an_undeclared_content_type_is_a_violation_when_strict() {
        let fixture = fixture();
        let strict = fixture.check(
            "listPets",
            200,
            Some("text/html"),
            "<html></html>",
            Conformance::Strict,
        );
        assert_eq!(strict.violations.len(), 1, "{:?}", strict.violations);
        assert_eq!(strict.violations[0].kind, ViolationKind::ContentType);
        assert!(
            strict.violations[0]
                .message
                .contains("`text/html` is not declared for status 200")
                && strict.violations[0].message.contains("application/json"),
            "{}",
            strict.violations[0].message
        );

        // Below strict the body is still held to the JSON schema.
        let schema_only = fixture.check(
            "listPets",
            200,
            Some("text/html"),
            "<html></html>",
            Conformance::Schema,
        );
        assert!(
            schema_only
                .violations
                .iter()
                .any(|v| v.message.contains("not valid JSON")),
            "{:?}",
            schema_only.violations
        );
    }

    #[test]
    fn content_type_parameters_and_case_are_ignored() {
        let check = fixture().check(
            "listPets",
            200,
            Some("Application/JSON; charset=utf-8"),
            GOOD_PAGE,
            Conformance::Strict,
        );
        assert!(check.conforms(), "{:?}", check.violations);
        assert_eq!(check.coverage, Coverage::Schema);
    }

    #[test]
    fn a_missing_content_type_is_read_as_json() {
        let fixture = fixture();
        let headers = [("X-Total".to_owned(), "1".to_owned())];
        let good = fixture.check_with("listPets", 200, &headers, GOOD_PAGE, Conformance::Strict);
        assert!(good.conforms(), "{:?}", good.violations);
        assert_eq!(good.coverage, Coverage::Schema);
        let bad = fixture.check_with("listPets", 200, &headers, "{}", Conformance::Strict);
        assert!(!bad.conforms());
    }

    #[test]
    fn media_type_ranges_govern_the_types_they_cover() {
        let fixture = fixture();
        // `application/*` covers `application/json`, and its schema applies.
        let bad = fixture.check("export", 200, JSON, "{}", Conformance::Strict);
        assert_eq!(bad.coverage, Coverage::Schema);
        assert!(!bad.conforms());
        // `*/*` on the default response covers anything.
        let any = fixture.check(
            "export",
            500,
            Some("application/problem+json"),
            r#"{"title": "boom"}"#,
            Conformance::Strict,
        );
        assert!(any.conforms(), "{:?}", any.violations);
        assert_eq!(any.coverage, Coverage::Schema);
    }

    #[test]
    fn a_non_json_body_is_declared_but_not_schema_checked() {
        let check = fixture().check(
            "export",
            200,
            Some("text/csv"),
            "a,b\n",
            Conformance::Strict,
        );
        assert!(check.conforms(), "{:?}", check.violations);
        assert_eq!(check.coverage, Coverage::Declared);
    }

    #[test]
    fn a_missing_required_header_is_a_violation() {
        let fixture = fixture();
        let headers = [("content-type".to_owned(), "application/json".to_owned())];
        let check = fixture.check_with("listPets", 200, &headers, GOOD_PAGE, Conformance::Strict);
        assert_eq!(check.violations.len(), 1, "{:?}", check.violations);
        assert_eq!(check.violations[0].kind, ViolationKind::Header);
        assert!(check.violations[0].message.contains("`X-Total` is missing"));
        // Headers are part of the strict contract only.
        let lenient = fixture.check_with("listPets", 200, &headers, GOOD_PAGE, Conformance::Schema);
        assert!(lenient.conforms());
    }

    #[test]
    fn an_ill_typed_header_is_a_violation() {
        let fixture = fixture();
        let headers = [
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-total".to_owned(), "many".to_owned()),
        ];
        let check = fixture.check_with("listPets", 200, &headers, GOOD_PAGE, Conformance::Strict);
        assert_eq!(check.violations.len(), 1, "{:?}", check.violations);
        assert_eq!(check.violations[0].kind, ViolationKind::Header);
        assert!(
            check.violations[0].message.contains("`X-Total`"),
            "{}",
            check.violations[0].message
        );
    }

    #[test]
    fn bodiless_replies_have_no_body_to_check() {
        let fixture = fixture();
        let head = fixture.check("probePets", 200, JSON, "", Conformance::Strict);
        assert!(head.conforms(), "{:?}", head.violations);
        assert_eq!(head.coverage, Coverage::Declared);
        let gone = fixture.check("silent", 204, None, "", Conformance::Strict);
        assert!(gone.conforms(), "{:?}", gone.violations);
        assert_eq!(gone.coverage, Coverage::Declared);
    }

    #[test]
    fn an_unfollowable_schema_reference_is_a_finding_not_a_pass() {
        let check = fixture().check("external", 200, JSON, "{}", Conformance::Schema);
        assert_eq!(check.violations.len(), 1, "{:?}", check.violations);
        assert!(
            check.violations[0].message.contains("./shared.yaml#/Pet")
                && check.violations[0].message.contains("could not be loaded"),
            "{}",
            check.violations[0].message
        );
    }

    #[test]
    fn an_operation_without_responses_has_nothing_to_violate() {
        let check = fixture().check("undocumented", 200, JSON, "{}", Conformance::Strict);
        assert!(check.conforms());
        assert_eq!(check.coverage, Coverage::Undeclared);
    }

    #[test]
    fn off_checks_nothing() {
        let check = fixture().check("listPets", 418, Some("text/html"), "<", Conformance::Off);
        assert!(check.conforms());
        assert_eq!(check.coverage, Coverage::Skipped);
    }

    #[test]
    fn an_openapi_30_description_is_checked_in_its_own_dialect() {
        let spec = spec_of(
            r##"
openapi: 3.0.3
info: { title: Legacy, version: "1" }
paths:
  /pet:
    get:
      operationId: getPet
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
components:
  schemas:
    Pet:
      type: object
      required: [id, password]
      properties:
        id: { type: string }
        tag: { type: string, nullable: true }
        password: { type: string, writeOnly: true }
        price: { type: number, minimum: 0, exclusiveMinimum: true }
"##,
        );
        let schemas = Schemas::from_spec(&spec);
        let operation = spec.operation(OpSelector::Id("getPet")).unwrap();
        let reply = |body: &'static str| Reply {
            method: "GET",
            status: 200,
            headers: &[],
            body: body.as_bytes(),
        };
        let good = check_response(
            &operation.responses,
            &schemas,
            reply(r#"{"id": "p1", "tag": null, "price": 5}"#),
            Conformance::Strict,
        );
        assert!(good.conforms(), "{:?}", good.violations);
        assert_eq!(good.coverage, Coverage::Schema);
        let bad = check_response(
            &operation.responses,
            &schemas,
            reply(r#"{"id": "p1", "price": 0}"#),
            Conformance::Strict,
        );
        assert_eq!(bad.violations.len(), 1, "{:?}", bad.violations);
        assert_eq!(bad.violations[0].pointer, "/price");
    }

    #[test]
    fn levels_parse_from_their_spelling() {
        for level in Conformance::ALL {
            assert_eq!(Conformance::parse(level.as_str()), Some(level));
        }
        assert_eq!(Conformance::parse("loose"), None);
    }
}
