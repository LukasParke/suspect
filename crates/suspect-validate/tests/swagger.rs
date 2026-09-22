//! Swagger 2.0 battery: the parse-tree checks fire on Swagger documents
//! and stay silent on 3.x ones.

use suspect_source::{Source, Uri};
use suspect_validate::Diagnostic;

fn doc(text: &str) -> Vec<Diagnostic> {
    let uri = Uri::parse("memory://swagger-check.yaml").expect("static uri");
    let low = suspect_low::LowDoc::parse(uri, Source::from_vec(text.as_bytes().to_vec()));
    suspect_validate::validate_swagger_low(&low)
}

fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
    diags.iter().map(|d| d.code).collect()
}

#[test]
fn info_and_operations_are_checked() {
    let firing = "\
swagger: \"2.0\"
paths:
  /a:
    get: {}
";
    let codes = codes(&doc(firing));
    assert!(codes.contains(&"swagger-info-required"), "{codes:?}");
    assert!(
        codes.contains(&"swagger-operation-missing-responses"),
        "{codes:?}"
    );
}

#[test]
fn parameters_need_name_in_and_body_schema() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
paths:
  /a:
    post:
      parameters:
        - in: body
        - name: q
      responses:
        '200': {}
";
    let codes = codes(&doc(firing));
    assert!(
        codes.contains(&"swagger-parameter-missing-name"),
        "{codes:?}"
    );
    assert!(codes.contains(&"swagger-parameter-missing-in"), "{codes:?}");
    assert!(codes.contains(&"swagger-body-param-schema"), "{codes:?}");
}

#[test]
fn path_templates_need_declared_path_parameters() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
paths:
  /pets/{petId}:
    get:
      parameters:
        - name: q
          in: query
          type: string
      responses:
        '200': {description: ok}
";
    let firing_codes = codes(&doc(firing));
    assert!(
        firing_codes.contains(&"swagger-path-param-undeclared"),
        "{firing_codes:?}"
    );

    let clean = firing.replace(
        "- name: q\n          in: query\n          type: string",
        "- name: petId\n          in: path\n          required: true\n          type: string",
    );
    assert!(!codes(&doc(&clean)).contains(&"swagger-path-param-undeclared"));
}

#[test]
fn security_requirements_need_declared_schemes() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
securityDefinitions:
  ApiKey: {type: apiKey}
security:
  - OAuth: []
paths: {}
";
    assert!(codes(&doc(firing)).contains(&"swagger-security-undefined"));

    let clean = firing.replace("- OAuth: []", "- ApiKey: []");
    assert!(!codes(&doc(&clean)).contains(&"swagger-security-undefined"));
}

#[test]
fn definitions_must_look_like_schemas() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
definitions:
  Empty: {}
paths: {}
";
    assert!(codes(&doc(firing)).contains(&"swagger-definition-shape"));

    let clean = firing.replace("Empty: {}", "Empty: {type: object}");
    assert!(!codes(&doc(&clean)).contains(&"swagger-definition-shape"));
}

#[test]
fn responses_need_descriptions() {
    let firing = "\
swagger: \"2.0\"
info: {title: t, version: \"1\"}
paths:
  /a:
    get:
      responses:
        '200': {}
";
    assert!(codes(&doc(firing)).contains(&"swagger-response-missing-description"));
}

#[test]
fn oas3_documents_get_nothing() {
    let text = "\
openapi: \"3.1.0\"
info: {title: t, version: \"1\"}
paths: {}
";
    assert!(doc(text).is_empty());
}
