//! Enum/type compatibility through the public default lint interface.

use suspect_lint::{Finding, Linter};
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

fn document(text: &str) -> LowDoc {
    LowDoc::parse(
        Uri::parse("memory://enum-fixture.yaml").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

fn enum_findings(doc: &LowDoc) -> Vec<Finding<'_>> {
    Linter::spectral_default()
        .run(doc)
        .into_iter()
        .filter(|finding| finding.code.as_ref() == "typed-enum")
        .collect()
}

#[test]
fn openrouter_nullable_string_enum_is_valid() {
    // Reduced from the tracked public ReasoningFormat schema at
    // db378a2a90d0167b9dca4f98b52074c54d249e1f.
    let doc = document(
        "openapi: 3.1.0\ninfo: {title: test, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    ReasoningFormat:\n      enum: ['unknown', 'openai-responses-v1', null]\n      type: ['string', 'null']\n",
    );
    assert!(enum_findings(&doc).is_empty());
}

#[test]
fn enum_shaped_instance_values_are_not_treated_as_schemas() {
    let text = r#"openapi: 3.1.0
info: {title: test, version: '1'}
paths: {}
components:
  schemas:
    Example:
      type: object
      properties:
        example: {type: integer, enum: [wrong]}
      example: {type: integer, enum: [instance]}
      examples: [{type: integer, enum: [instance]}]
      default: {type: integer, enum: [instance]}
      const: {type: integer, enum: [instance]}
      enum: [{type: integer, enum: [instance]}]
      x-custom: {type: integer, enum: [extension]}
"#;
    let doc = document(text);
    let findings = enum_findings(&doc);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0].path.to_path(),
        "/components/schemas/Example/properties/example/enum/0"
    );
    assert_eq!(&text[findings[0].range.clone()], "wrong");
}

#[test]
fn integer_enum_checks_do_not_round_decimal_values() {
    let text = "openapi: 3.1.0\ninfo: {title: test, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Count:\n      type: integer\n      enum: [1, 1.0, 1e400, 0e-400, 100e-2, 1.0000000000000000001, 1e-400]\n";
    let doc = document(text);
    let findings = enum_findings(&doc);
    let invalid_values: Vec<_> = findings.iter().map(|f| &text[f.range.clone()]).collect();
    assert_eq!(invalid_values, ["1.0000000000000000001", "1e-400"]);
}

#[test]
fn mixed_enums_follow_declared_types_and_versioned_nullability() {
    for (version, schema, expected) in [
        ("3.1.0", "enum: [one, 2, false, null, {}, []]", 0),
        (
            "3.1.0",
            "type: [number, boolean, 'null']\n      enum: [1, 1.5, false, null]",
            0,
        ),
        (
            "3.0.3",
            "type: string\n      nullable: true\n      enum: [one, null]",
            0,
        ),
        (
            "3.0.3",
            "type: string\n      nullable: false\n      enum: [one, null]",
            1,
        ),
        (
            "3.1.0",
            "type: string\n      nullable: true\n      enum: [one, null]",
            1,
        ),
        (
            "3.2.0",
            "type: string\n      nullable: true\n      enum: [one, null]",
            1,
        ),
        ("3.1.0", "type: string\n      enum: [1, 2]", 2),
        ("3.1.0", "type: [string, 'null']\n      enum: [false]", 1),
        (
            "3.1.0",
            "type: [object, array]\n      enum: [{a: 1}, [2]]",
            0,
        ),
    ] {
        let doc = document(&format!(
            "openapi: {version}\ninfo: {{title: test, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Value:\n      {schema}\n"
        ));
        assert_eq!(enum_findings(&doc).len(), expected, "{version}: {schema}");
    }
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT; run the dedicated acceptance gate"]
fn tracked_openrouter_nullable_enums_pass_and_a_mutation_is_located() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT to the OpenRouter source checkout");
    let text = std::fs::read_to_string(root.join("projects/docs/openapi/openapi.yaml"))
        .expect("read required tracked public OpenRouter spec");
    let original = document(&text);
    assert!(enum_findings(&original).is_empty());
    let start = text
        .find("    ReasoningFormat:\n")
        .expect("known public enum");
    let end = start
        + text[start..]
            .find("    ReasoningItem:\n")
            .expect("following schema");
    let relative = text[start..end]
        .find("        - null\n")
        .expect("explicit nullable member");
    let offset = start + relative;
    let mut mutated = text.clone();
    mutated.replace_range(offset..offset + "        - null".len(), "        - 123");
    let doc = document(&mutated);
    let findings = enum_findings(&doc);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0].path.to_path(),
        "/components/schemas/ReasoningFormat/enum/9"
    );
    assert_eq!(&mutated[findings[0].range.clone()], "123");

    let management_text = std::fs::read_to_string(root.join("openrouter-management.openapi.yaml"))
        .expect("read required tracked management OpenRouter spec");
    let management = document(&management_text);
    let mismatches = enum_findings(&management);
    assert_eq!(
        mismatches.len(),
        5,
        "3.1 nullable annotations do not authorize null"
    );
    assert!(
        mismatches
            .iter()
            .all(|finding| &management_text[finding.range.clone()] == "null")
    );
    let naming: Vec<_> = Linter::spectral_default()
        .run(&management)
        .into_iter()
        .filter(|finding| finding.code.as_ref() == "operation-operationId")
        .collect();
    assert_eq!(
        naming.len(),
        5,
        "the independent operation naming rule remains enabled"
    );
}
