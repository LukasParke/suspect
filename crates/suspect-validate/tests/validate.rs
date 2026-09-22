use std::path::Path;
use std::sync::Arc;

use suspect_oas::{ModelError, Session};
use suspect_ref::WorkspaceBuilder;
use suspect_validate::{
    Diagnostic, Severity, validate_entry, validate_openapi, validate_workspace,
};

fn session_with(dir: &Path, name: &str, content: &str) -> Session {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    Session::new(Arc::new(ws))
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suspect-validate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
    diags.iter().map(|d| d.code).collect()
}

#[test]
fn schema_values_have_valid_kinds_with_dialect_specific_boolean_positions() {
    for (version, expected) in [
        ("3.1.0", vec!["17", "[]", "bad", "null"]),
        ("3.0.3", vec!["17", "[]", "bad", "false", "null", "true"]),
    ] {
        let dir = unique_dir(&format!("schema-kinds-{version}"));
        let source = format!(
            "openapi: {version}\ninfo: {{title: test, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    MissingContract: null\n    NumberContract: 17\n    StringContract: bad\n    ArrayContract: []\n    BooleanContract: true\n    Model:\n      type: object\n      additionalProperties: false\n      properties:\n        additionalProperties: false\n"
        );
        let session = session_with(&dir, "main.yaml", &source);
        let findings = validate_entry(&session, "main.yaml").unwrap();
        let mut invalid: Vec<_> = findings
            .iter()
            .filter(|finding| finding.code == "oas-schema-invalid-kind")
            .map(|finding| &source[finding.range.clone()])
            .collect();
        invalid.sort_unstable();
        assert_eq!(invalid, expected, "{version}: {findings:?}");
    }
}

#[test]
fn type_declarations_require_valid_unique_strings_for_the_openapi_version() {
    for (index, version, declaration, errors) in [
        (0, "3.1.0", "type: 42", 1),
        (1, "3.1.0", "type: null", 1),
        (2, "3.1.0", "type:", 1),
        (3, "3.1.0", "type: []", 1),
        (4, "3.1.0", "type: [string, 7]", 1),
        (5, "3.1.0", "type: [string, string]", 1),
        (6, "3.1.0", r#"type: [string, "\u0073tring"]"#, 1),
        (7, "3.1.0", r#"type: "\u0073tring""#, 0),
        (8, "3.1.0", "type: [string, 'null']", 0),
        (9, "3.0.3", "type: [string, 'null']", 1),
        (10, "3.0.3", "type: 'null'", 1),
        (11, "3.0.3", "type: string, nullable: true", 0),
    ] {
        let dir = unique_dir(&format!("type-shape-{index}"));
        let source = format!(
            "openapi: {version}\ninfo: {{title: test, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Model: {{{declaration}}}\n"
        );
        let session = session_with(&dir, "main.yaml", &source);
        let findings = validate_entry(&session, "main.yaml").unwrap();
        let actual = findings
            .iter()
            .filter(|finding| {
                matches!(
                    finding.code,
                    "oas-schema-invalid-type" | "oas-schema-unknown-type"
                )
            })
            .count();
        assert_eq!(actual, errors, "{version} {declaration}: {findings:?}");
    }
}

#[test]
fn library_validation_distinguishes_contract_references_from_instance_data() {
    let dir = unique_dir("semantic-refs");
    let source = format!(
        "{HEADER}paths: {{}}\ncomponents:\n  schemas:\n    Document:\n      type: object\n      properties:\n        $ref: {{type: string}}\n      example: {{$ref: 'missing-example.yaml#/data'}}\n    Broken:\n      $ref: 'missing-schema.yaml#/Model'\n"
    );
    let session = session_with(&dir, "main.yaml", &source);
    let findings = validate_entry(&session, "main.yaml")
        .expect("a missing contract target is a located finding, not a failed entry load");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, "unresolved-ref");
    assert_eq!(
        &source[findings[0].range.clone()],
        "'missing-schema.yaml#/Model'"
    );
    assert_eq!(
        session.workspace().len(),
        1,
        "example data must not load a document"
    );
}

#[test]
fn example_type_checks_use_mathematical_numbers_and_versioned_nullability() {
    for (index, version, schema, example, expected) in [
        (0, "3.1.0", "{type: integer}", "1.0", 0),
        (1, "3.1.0", "{type: integer}", "1e400", 0),
        (
            2,
            "3.1.0",
            "{type: integer}",
            "1.0000000000000000000000000001",
            1,
        ),
        (3, "3.1.0", "{type: string, nullable: true}", "null", 1),
        (4, "3.0.3", "{type: string, nullable: true}", "null", 0),
        (5, "3.1.0", "{type: [string, 'null']}", "null", 0),
        (6, "3.1.0", "{properties: {id: {type: string}}}", "7", 0),
        (7, "3.1.0", "{items: {type: string}}", "false", 0),
        (8, "3.1.0", "{type: integer}", "1e-400", 1),
        (9, "3.1.0", "{type: string}", "~", 1),
    ] {
        let dir = unique_dir(&format!("example-type-{index}"));
        let source = format!(
            "openapi: {version}\ninfo: {{title: test, version: '1'}}\npaths:\n  /example:\n    get:\n      operationId: example\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {schema}\n              example: {example}\n"
        );
        let session = session_with(&dir, "main.yaml", &source);
        let findings = validate_entry(&session, "main.yaml").unwrap();
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.code == "oas-example-type-mismatch")
                .count(),
            expected,
            "{version} {schema} example {example}: {findings:?}"
        );
    }
}

#[test]
fn external_response_example_findings_retain_the_example_source() {
    let dir = unique_dir("external-example-provenance");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /example:\n    get:\n      operationId: example\n      responses:\n        '200': {{$ref: 'response.yaml#/Response'}}\n"
        ),
    );
    let external = "Response:\n  description: ok\n  content:\n    application/json:\n      schema: {type: string}\n      example: 42\n";
    std::fs::write(dir.join("response.yaml"), external).unwrap();
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let examples: Vec<_> = findings
        .iter()
        .filter(|finding| finding.code == "oas-example-type-mismatch")
        .collect();
    assert_eq!(examples.len(), 1, "{findings:?}");
    assert!(examples[0].doc.as_str().ends_with("/response.yaml"));
    assert_eq!(&external[examples[0].range.clone()], "42");
}

#[test]
fn nullable_discriminator_union_preserves_null_without_hiding_an_invalid_variant() {
    // Reduced from OpenRouter's ORAnthropicNullableCaller. Discriminator
    // metadata must not make the explicitly permitted null branch invalid.
    for (tag, alternative, expected_errors) in [
        ("null", "{type: 'null'}", 0),
        (
            "missing",
            "{type: object, properties: {id: {type: string}}}",
            1,
        ),
    ] {
        let dir = unique_dir(&format!("nullable-discriminator-{tag}"));
        let session = session_with(
            &dir,
            "main.yaml",
            &format!(
                "{HEADER}paths: {{}}\ncomponents:\n  schemas:\n    Caller:\n      discriminator: {{propertyName: type}}\n      oneOf:\n        - type: object\n          required: [type]\n          properties:\n            type: {{type: string, const: direct}}\n        - {alternative}\n"
            ),
        );
        let findings = validate_entry(&session, "main.yaml").unwrap();
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.code == "oas-discriminator-missing-property")
                .count(),
            expected_errors,
            "alternative {tag}: {findings:?}"
        );
    }
}

#[test]
fn invalid_nested_property_is_reported_once_despite_shared_references() {
    let dir = unique_dir("nested-shared-property");
    let source = format!(
        "{HEADER}paths: {{}}\ncomponents:\n  schemas:\n    Model:\n      properties:\n        suspect_regression_property: {{type: suspect_invalid_type}}\n    WrapperA:\n      allOf: [{{$ref: '#/components/schemas/Model'}}]\n    WrapperB:\n      allOf: [{{$ref: '#/components/schemas/Model'}}]\n"
    );
    let session = session_with(&dir, "main.yaml", &source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let matching: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "oas-schema-unknown-type")
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one finding per invalid property: {matching:?}"
    );
    assert_eq!(&source[matching[0].range.clone()], "suspect_invalid_type");
}

#[test]
fn referenced_schema_findings_point_to_the_file_containing_the_defect() {
    let dir = unique_dir("external-schema-provenance");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths: {{}}\ncomponents:\n  schemas:\n    Model: {{$ref: 'shared.yaml#/Model'}}\n"
        ),
    );
    let external = "Model:\n  properties:\n    value: {type: broken}\n";
    std::fs::write(dir.join("shared.yaml"), external).unwrap();
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let matching: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "oas-schema-unknown-type")
        .collect();
    assert_eq!(matching.len(), 1, "{findings:?}");
    assert!(
        matching[0].doc.as_str().ends_with("/shared.yaml"),
        "{matching:?}"
    );
    assert_eq!(&external[matching[0].range.clone()], "broken");
}

#[test]
fn checks_schema_applicators_without_interpreting_instance_data_as_schemas() {
    // OpenRouter's provider-monitor schema uses conditional schemas; it also
    // has properties literally named after schema keywords. Examples/defaults
    // are instance data, so schema-shaped data there must not produce errors.
    let dir = unique_dir("schema-applicators");
    let source = format!(
        "{HEADER}paths: {{}}\ncomponents:\n  schemas:\n    Monitor:\n      properties:\n        allOf: {{type: string}}\n      if:\n        properties:\n          enabled: {{const: true}}\n      then:\n        properties:\n          provider: {{type: broken_conditional}}\n      additionalProperties: {{type: broken_additional}}\n      dependentSchemas:\n        enabled: {{type: broken_dependent}}\n      patternProperties:\n        '^x-': {{type: broken_pattern}}\n      contains: {{type: broken_contains}}\n      propertyNames: {{type: broken_name}}\n      unevaluatedProperties: {{type: broken_unevaluated}}\n      unevaluatedItems: {{type: broken_items}}\n      $defs:\n        Hidden: {{type: broken_definition}}\n      example: {{type: broken_example, allOf: [{{type: broken_data}}]}}\n      default: {{type: broken_default}}\n"
    );
    let session = session_with(&dir, "main.yaml", &source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let mut invalid_types: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "oas-schema-unknown-type")
        .map(|f| &source[f.range.clone()])
        .collect();
    invalid_types.sort_unstable();
    assert_eq!(
        invalid_types,
        [
            "broken_additional",
            "broken_conditional",
            "broken_contains",
            "broken_definition",
            "broken_dependent",
            "broken_items",
            "broken_name",
            "broken_pattern",
            "broken_unevaluated",
        ]
    );
}

#[test]
fn validates_inline_transport_schemas_without_a_components_section() {
    let dir = unique_dir("inline-transport-schemas");
    let source = format!(
        "{HEADER}paths:
  /items:
    parameters:
      - name: q
        in: query
        schema: {{type: broken_query}}
    post:
      operationId: create
      requestBody:
        content:
          application/json:
            schema: {{type: broken_body}}
      responses:
        '200':
          description: ok
          headers:
            X-Rate:
              schema: {{type: broken_header}}
          content:
            application/json:
              schema: {{type: broken_response}}
      callbacks:
        changed:
          '{{$request.body#/callback}}':
            post:
              operationId: changed
              requestBody:
                content:
                  application/json:
                    schema: {{type: broken_callback}}
              responses:
                '204': {{description: ok}}
webhooks:
  created:
    post:
      operationId: created
      requestBody:
        content:
          application/json:
            schema: {{type: broken_webhook}}
      responses:
        '204': {{description: ok}}
"
    );
    let session = session_with(&dir, "main.yaml", &source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let mut invalid_types: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "oas-schema-unknown-type")
        .map(|f| &source[f.range.clone()])
        .collect();
    invalid_types.sort_unstable();
    assert_eq!(
        invalid_types,
        [
            "broken_body",
            "broken_callback",
            "broken_header",
            "broken_query",
            "broken_response",
            "broken_webhook"
        ]
    );
}

#[test]
fn schema_ref_siblings_are_checked_in_31_and_ignored_in_30() {
    for (version, expected_errors) in [("3.1.0", 1), ("3.0.3", 0)] {
        let dir = unique_dir(&format!("ref-siblings-{version}"));
        let source = format!(
            "openapi: {version}\ninfo: {{title: t, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Base: {{type: object}}\n    Refined:\n      $ref: '#/components/schemas/Base'\n      properties:\n        added: {{type: broken_sibling}}\n"
        );
        let session = session_with(&dir, "main.yaml", &source);
        let findings = validate_entry(&session, "main.yaml").unwrap();
        let matching: Vec<_> = findings
            .iter()
            .filter(|f| f.code == "oas-schema-unknown-type")
            .collect();
        assert_eq!(
            matching.len(),
            expected_errors,
            "OpenAPI {version}: {findings:?}"
        );
        if let Some(finding) = matching.first() {
            assert_eq!(&source[finding.range.clone()], "broken_sibling");
        }
    }
}

#[test]
fn absent_schema_values_are_located_errors() {
    let dir = unique_dir("missing-schema-values");
    let source = format!(
        "{HEADER}paths:
  /items:
    get:
      operationId: items
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
components:
  schemas:
    Missing:
    Object:
      type: object
      properties:
        absent:
    Array:
      type: array
      items:
"
    );
    let session = session_with(&dir, "main.yaml", &source);
    let findings = validate_entry(&session, "main.yaml").unwrap();
    let mut missing: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "oas-schema-invalid-kind")
        .map(|f| &source[f.range.clone()])
        .collect();
    missing.sort_unstable();
    assert_eq!(missing, ["Missing", "absent", "items", "schema"]);
}

const HEADER: &str = "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\n";

#[test]
fn missing_operation_id_is_warning() {
    let dir = unique_dir("missing-opid");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      responses:\n        '200':\n          description: ok\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(codes(&diags).contains(&"oas-operation-missing-operationId"));
    let d = diags
        .iter()
        .find(|d| d.code == "oas-operation-missing-operationId")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);
}

#[test]
fn duplicate_operation_id_is_error() {
    let dir = unique_dir("dup-opid");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /a:\n    get:\n      operationId: same\n      responses: {{'200': {{description: ok}}}}\n  /b:\n    get:\n      operationId: same\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let dups: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-duplicate-operation-id")
        .collect();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].severity, Severity::Error);
    assert!(dups[0].message.contains("same"));
}

#[test]
fn missing_responses_is_error() {
    let dir = unique_dir("missing-responses");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n"),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-operation-missing-responses")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn parameter_missing_name_and_in() {
    let dir = unique_dir("param-fields");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      parameters:\n        - schema: {{type: string}}\n        - name: limit\n          schema: {{type: integer}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(codes(&diags).contains(&"oas-parameter-missing-name"));
    assert!(codes(&diags).contains(&"oas-parameter-missing-in"));
}

#[test]
fn path_param_not_declared_and_unused() {
    let dir = unique_dir("path-params");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /pets/{{petId}}:\n    get:\n      operationId: op\n      responses: {{'200': {{description: ok}}}}\n  /things:\n    get:\n      operationId: op2\n      responses: {{'200': {{description: ok}}}}\n    parameters:\n      - name: extra\n        in: path\n        required: true\n        schema: {{type: string}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let missing = diags
        .iter()
        .find(|d| d.code == "oas-path-param-not-declared")
        .unwrap();
    assert_eq!(missing.severity, Severity::Error);
    assert!(missing.message.contains("petId"));
    let unused = diags
        .iter()
        .find(|d| d.code == "oas-unused-path-param")
        .unwrap();
    assert_eq!(unused.severity, Severity::Warning);
    assert!(unused.message.contains("extra"));
}

#[test]
fn path_param_required_false_is_error() {
    let dir = unique_dir("required-false");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /i/{{id}}:\n    get:\n      operationId: op\n      parameters:\n        - name: id\n          in: path\n          required: false\n          schema: {{type: string}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-parameter-required-missing")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn response_missing_description_is_error() {
    let dir = unique_dir("resp-desc");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      responses:\n        '200':\n          content:\n            application/json: {{schema: {{type: string}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-response-missing-description")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn security_unknown_scheme_is_error() {
    let dir = unique_dir("security");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}security:\n  - apiKey: []\npaths:\n  /p:\n    get:\n      operationId: op\n      security:\n        - missing2: []\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let hits: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-security-unknown-scheme")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|d| d.severity == Severity::Error));
}

#[test]
fn server_variable_unknown_is_error() {
    let dir = unique_dir("server-var");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}servers:\n  - url: 'https://{{host}}/v1'\n    variables: {{}}\npaths: {{}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-server-variable-unknown")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("host"));
}

#[test]
fn undeclared_tag_is_warning() {
    let dir = unique_dir("tags");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: op\n      tags: [pets]\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-tag-undeclared")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // declared tags do not warn
    let dir2 = unique_dir("tags-ok");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        &format!(
            "{HEADER}tags:\n  - name: pets\npaths:\n  /p:\n    get:\n      operationId: op\n      tags: [pets]\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-tag-undeclared"));
}

#[test]
fn discriminator_missing_property_is_error() {
    let dir = unique_dir("disc-prop");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Pet:\n      type: object\n      required: [name]\n      properties:\n        name: {{type: string}}\n      discriminator:\n        propertyName: kind\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-discriminator-missing-property")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn discriminator_property_via_all_of_is_accepted() {
    let dir = unique_dir("disc-allof");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Base:\n      type: object\n      required: [kind]\n      properties:\n        kind: {{type: string}}\n    Dog:\n      allOf:\n        - $ref: '#/components/schemas/Base'\n      discriminator:\n        propertyName: kind\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(!codes(&diags).contains(&"oas-discriminator-missing-property"));
}

#[test]
fn discriminator_unknown_mapping_is_error() {
    let dir = unique_dir("disc-map");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    Pet:\n      type: object\n      required: [kind]\n      properties:\n        kind: {{type: string}}\n      discriminator:\n        propertyName: kind\n        mapping:\n          dog: '#/components/schemas/Dog'\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-discriminator-unknown-mapping")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("Dog"));
}

#[test]
fn discriminator_property_in_each_union_variant_is_accepted() {
    // Reduced from OpenRouter's AnthropicCaller shape: the union delegates
    // its discriminator field to independently declared component variants.
    let dir = unique_dir("union-discriminator");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:
  schemas:
    Caller:
      discriminator:
        propertyName: type
      oneOf:
        - $ref: '#/components/schemas/Direct'
        - $ref: '#/components/schemas/Code'
    Direct:
      type: object
      required: [type]
      properties:
        type: {{type: string, const: direct}}
    Code:
      type: object
      required: [type]
      properties:
        type: {{type: string, const: code}}
"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    assert!(!codes(&diags).contains(&"oas-discriminator-missing-property"));
}

#[test]
fn schema_unknown_type_is_error() {
    let dir = unique_dir("bad-type");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}components:\n  schemas:\n    A:\n      type: striing\n    B:\n      type: [string, bogus]\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let hits: Vec<_> = diags
        .iter()
        .filter(|d| d.code == "oas-schema-unknown-type")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|d| d.severity == Severity::Error));
}

#[test]
fn example_type_mismatch_is_warning() {
    let dir = unique_dir("example-mismatch");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    post:\n      operationId: op\n      requestBody:\n        content:\n          application/json:\n            schema: {{type: integer}}\n            example: hello\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-example-type-mismatch")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // matching example does not warn
    let dir2 = unique_dir("example-match");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    post:\n      operationId: op\n      requestBody:\n        content:\n          application/json:\n            schema: {{type: integer}}\n            example: 42\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-example-type-mismatch"));
}

#[test]
fn trailing_slash_and_bad_path_key() {
    let dir = unique_dir("path-keys");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /pets/: {{}}\n  pets: {{}}\n"),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let slash = diags
        .iter()
        .find(|d| d.code == "oas-path-trailing-slash")
        .unwrap();
    assert_eq!(slash.severity, Severity::Warning);
    let empty = diags
        .iter()
        .find(|d| d.code == "oas-empty-path-template")
        .unwrap();
    assert_eq!(empty.severity, Severity::Error);
}

#[test]
fn duplicate_header_param_is_error() {
    let dir = unique_dir("dup-header");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    parameters:\n      - name: X-Req\n        in: header\n        schema: {{type: string}}\n    get:\n      operationId: op\n      parameters:\n        - name: X-Req\n          in: header\n          schema: {{type: string}}\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-duplicate-header-param")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn deprecated_operation_is_info() {
    let dir = unique_dir("deprecated");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      operationId: old\n      deprecated: true\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-deprecated-operation")
        .unwrap();
    assert_eq!(d.severity, Severity::Info);
}

#[test]
fn webhooks_on_30_is_error() {
    let dir = unique_dir("webhooks30");
    let session = session_with(
        &dir,
        "main.yaml",
        "openapi: 3.0.0\ninfo: {title: t, version: \"1\"}\nwebhooks: {}\npaths: {}\n",
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-webhook-unsupported-version")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn license_missing_url_is_warning() {
    // 3.0: url required
    let dir = unique_dir("license30");
    let session = session_with(
        &dir,
        "main.yaml",
        "openapi: 3.0.0\ninfo:\n  title: t\n  version: \"1\"\n  license: {name: MIT}\npaths: {}\n",
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let d = diags
        .iter()
        .find(|d| d.code == "oas-license-missing-url")
        .unwrap();
    assert_eq!(d.severity, Severity::Warning);

    // 3.1: identifier suffices
    let dir2 = unique_dir("license31");
    let session2 = session_with(
        &dir2,
        "main.yaml",
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\n  license: {name: MIT, identifier: MIT}\npaths: {}\n",
    );
    let diags2 = validate_entry(&session2, "main.yaml").unwrap();
    assert!(!codes(&diags2).contains(&"oas-license-missing-url"));
}

#[test]
fn petstore_corpus_has_no_errors() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.join("petstore-expanded.yaml").exists() {
        eprintln!("skipping: corpus/ is gitignored and absent");
        return;
    }
    let ws = WorkspaceBuilder::new().root(&corpus).build().unwrap();
    let session = Session::new(Arc::new(ws));
    let diags = validate_entry(&session, "petstore-expanded.yaml").unwrap();
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn generated_fixture_validates_without_panic() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let entry = fixtures.join("generated_100x100.json");
    if !entry.exists() {
        // fixtures/ is gitignored; skip on clean checkouts and CI
        eprintln!("skipping: generated fixtures not present");
        return;
    }
    let ws = WorkspaceBuilder::new().root(&fixtures).build().unwrap();
    let session = Session::new(Arc::new(ws));
    let diags = validate_entry(&session, "generated_100x100.json").unwrap();
    // no assertion on count; exercise the full pipeline
    let _ = diags.len();
}

#[test]
fn validation_is_deterministic() {
    let dir = unique_dir("determinism");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /a:\n    get:\n      responses: {{'200': {{}}}}\n  /b:\n    post:\n      operationId: x\n      responses: {{'200': {{description: ok}}}}\n"
        ),
    );
    let first = validate_entry(&session, "main.yaml").unwrap();
    let second = validate_entry(&session, "main.yaml").unwrap();
    assert_eq!(first, second);
}

#[test]
fn severity_mapping_matches_codes() {
    let dir = unique_dir("severities");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!(
            "{HEADER}paths:\n  /p:\n    get:\n      deprecated: true\n      responses: {{'200': {{}}}}\n"
        ),
    );
    let diags = validate_entry(&session, "main.yaml").unwrap();
    let by_code = |code: &str| diags.iter().find(|d| d.code == code).map(|d| d.severity);
    assert_eq!(by_code("oas-deprecated-operation"), Some(Severity::Info));
    assert_eq!(
        by_code("oas-response-missing-description"),
        Some(Severity::Error)
    );
    assert_eq!(
        by_code("oas-operation-missing-operationId"),
        Some(Severity::Warning)
    );
}

#[test]
fn workspace_validation_covers_loaded_docs() -> Result<(), ModelError> {
    let dir = unique_dir("workspace");
    let session = session_with(
        &dir,
        "main.yaml",
        &format!("{HEADER}paths:\n  /p:\n    get:\n      responses: {{'200': {{}}}}\n"),
    );
    // a non-OpenAPI sibling must be skipped without error
    std::fs::write(dir.join("other.yaml"), "just: data\n").unwrap();

    let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
    ws.load_all("main.yaml").unwrap();
    let session2 = Session::new(Arc::new(ws));
    let diags = validate_workspace(&session2)?;
    assert!(
        diags
            .iter()
            .any(|d| d.code == "oas-response-missing-description")
    );

    // entry-based API agrees
    let per_entry = validate_entry(&session, "main.yaml")?;
    assert_eq!(per_entry.len(), diags.len());

    // direct view API produces the same set for this single-doc workspace
    let api = session.load("main.yaml")?;
    assert_eq!(validate_openapi(&api), per_entry);
    Ok(())
}
