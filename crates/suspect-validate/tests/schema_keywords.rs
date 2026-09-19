use std::sync::Arc;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::Diagnostic;

fn validate(name: &str, source: &str) -> Vec<Diagnostic> {
    let dir = std::env::temp_dir().join(format!(
        "suspect-schema-keywords-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("api.yaml"), source).unwrap();
    let session = Session::new(Arc::new(
        WorkspaceBuilder::new().root(&dir).build().unwrap(),
    ));
    let findings = suspect_validate::validate_entry(&session, "api.yaml").unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    findings
}

#[test]
fn malformed_cardinalities_and_compositions_are_located_declaration_errors() {
    let source = r#"openapi: 3.1.0
info: {title: declarations, version: '1'}
paths: {}
components:
  schemas:
    Bad:
      minItems: -1
      maxItems: 1.5
      minLength: '2'
      maxProperties: null
      minContains:
      allOf: []
      anyOf: 7
      oneOf: {}
      properties: false
      dependentSchemas: []
"#;
    let findings = validate("basic-invalid", source);
    let invalid: Vec<_> = findings
        .iter()
        .filter(|d| d.code == "oas-schema-invalid-keyword")
        .collect();
    assert_eq!(invalid.len(), 10, "{findings:?}");
    for spelling in [
        "-1",
        "1.5",
        "'2'",
        "null",
        "minContains",
        "[]",
        "7",
        "{}",
        "false",
    ] {
        assert!(
            invalid.iter().any(|d| source[d.range.clone()] == *spelling),
            "missing {spelling:?}: {invalid:?}"
        );
    }
}

#[test]
fn enum_recommendations_do_not_become_errors_and_required_uses_its_dialect() {
    // Wright validation-00 §§5.15/5.20 and 2020-12 §§6.1.2/6.5.3:
    // enum nonempty/unique is SHOULD in both; required nonempty is a MUST
    // only in the older dialect. Required names need not be declared properties.
    for (version, count) in [("3.0.3", 4), ("3.1.0", 3)] {
        let source = format!(
            r#"openapi: {version}
info: {{title: declarations, version: '1'}}
paths: {{}}
components:
  schemas:
    EmptyEnum: {{enum: []}}
    DuplicateEnum: {{enum: [1, 1.0]}}
    UntypedEnum: {{enum: [null, false, 42, hello, {{type: 7}}]}}
    UnlistedRequirement: {{required: [undeclared]}}
    EmptyRequired: {{required: []}}
    BadRequired: {{required: [name, 42, name]}}
    BadEnum: {{enum: {{type: string}}}}
"#
        );
        let findings = validate(&format!("enum-required-{version}"), &source);
        let invalid: Vec<_> = findings
            .iter()
            .filter(|d| d.code == "oas-schema-invalid-keyword")
            .collect();
        assert_eq!(invalid.len(), count, "{version}: {findings:?}");
        assert_eq!(
            invalid
                .iter()
                .filter(|d| d.message.contains("`enum`"))
                .count(),
            1
        );
        assert!(!findings.iter().any(|d| d.code == "oas-schema-unknown-type"));
    }
}

#[test]
fn dependent_required_is_a_map_of_unique_string_arrays_and_leaves_instance_data_alone() {
    let source = r#"openapi: 3.1.0
info: {title: declarations, version: '1'}
paths: {}
components:
  schemas:
    Model:
      dependentRequired:
        account: [name, name, false]
        other: null
        empty: []
      properties:
        required: {type: string}
        dependentRequired: {type: integer}
      default: {dependentRequired: {x: false}}
      examples: [{required: 7}]
      const: {required: 7}
      enum: [{dependentRequired: [false]}]
    BadMap:
      dependentRequired: []
"#;
    let findings = validate("dependent-required", source);
    let invalid: Vec<_> = findings
        .iter()
        .filter(|d| d.code == "oas-schema-invalid-keyword")
        .collect();
    assert_eq!(invalid.len(), 4, "{findings:?}");
    assert!(
        invalid
            .iter()
            .all(|d| d.message.contains("dependentRequired"))
    );
}

#[test]
fn external_document_and_lexical_schema_dialects_control_declaration_rules() {
    let dir = std::env::temp_dir().join(format!(
        "suspect-schema-keywords-dialects-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("api.yaml"), "openapi: 3.1.0\ninfo: {title: entry, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Old: {$ref: 'old.yaml#/components/schemas/Model'}\n").unwrap();
    let old = "openapi: 3.0.3\ninfo: {title: old, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Model: {required: []}\n";
    std::fs::write(dir.join("old.yaml"), old).unwrap();
    let session = Session::new(Arc::new(
        WorkspaceBuilder::new().root(&dir).build().unwrap(),
    ));
    let findings = suspect_validate::validate_entry(&session, "api.yaml").unwrap();
    let invalid = findings
        .iter()
        .find(|d| d.code == "oas-schema-invalid-keyword")
        .unwrap_or_else(|| panic!("{findings:?}"));
    assert!(invalid.doc.as_str().ends_with("/old.yaml"));
    assert_eq!(&old[invalid.range.clone()], "[]");
    std::fs::remove_dir_all(dir).unwrap();

    let source = r#"openapi: 3.1.0
info: {title: dialect scopes, version: '1'}
paths:
  /value:
    get:
      operationId: value
      $schema: https://not-a-schema.example/dialect
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema: {minItems: false}
components:
  schemas:
    Custom:
      $schema: https://custom.example/dialect
      properties:
        child: {minItems: false}
    Direct: {$ref: '#/components/schemas/Custom/properties/child'}
"#;
    let findings = validate("lexical-dialect", source);
    assert_eq!(
        findings
            .iter()
            .filter(|d| d.code == "oas-schema-unsupported-dialect")
            .count(),
        1,
        "{findings:?}"
    );
    let invalid: Vec<_> = findings
        .iter()
        .filter(|d| d.code == "oas-schema-invalid-keyword")
        .collect();
    assert_eq!(
        invalid.len(),
        1,
        "only the ordinary transport schema uses supported rules: {findings:?}"
    );
    assert_eq!(&source[invalid[0].range.clone()], "false");
}

#[test]
fn numeric_declarations_use_exact_values_and_versioned_exclusive_bounds() {
    for (version, exclusive, bad_exclusive) in [("3.0.3", "true", "0.5"), ("3.1.0", "0.5", "true")]
    {
        let source = format!(
            r#"openapi: {version}
info: {{title: exact declarations, version: '1'}}
paths: {{}}
components:
  schemas:
    Valid:
      minimum: 1e999999999999999999999999
      maximum: -1e999999999999999999999999
      multipleOf: 1e-999999999999999999999999
      minItems: -0.0e999999999999999999999999
      maxItems: 100e-2
      minLength: 1e999999999999999999999999
      maxProperties: 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
      exclusiveMinimum: {exclusive}
    ExclusiveWithoutBound: {{exclusiveMaximum: {exclusive}}}
    Bad:
      minimum: .nan
      maximum: .inf
      exclusiveMinimum: {bad_exclusive}
      exclusiveMaximum: '2'
      multipleOf: -0.0e999999999999999999999999
      minItems: 1e-999999999999999999999999
    NegativeMultiple: {{multipleOf: -1e-999999999999999999999999}}
    StringMultiple: {{multipleOf: '0.01'}}
"#
        );
        let findings = validate(&format!("numeric-{version}"), &source);
        let invalid: Vec<_> = findings
            .iter()
            .filter(|d| d.code == "oas-schema-invalid-keyword")
            .collect();
        assert_eq!(invalid.len(), 8, "{version}: {findings:?}");
        assert!(
            invalid
                .iter()
                .all(|d| d.range.start > source.find("    Bad:").unwrap()),
            "valid impossible and exact declarations: {findings:?}"
        );
    }
}

#[test]
fn annotation_and_flag_shapes_follow_active_vocabulary_without_interpreting_data() {
    let source = r#"openapi: 3.1.0
info: {title: annotation declarations, version: '1'}
paths: {}
components:
  schemas:
    Valid:
      title: ''
      pattern: '(?<=a)b'
      nullable: {unrecognized: annotation}
      definitions: {legacy: {type: 7}}
      dependencies: {legacy: false}
      additionalItems: 7
      example: {title: false}
      examples: [{title: false}]
      default: {uniqueItems: 7}
      const: {description: 7}
      properties:
        title: {type: string}
        description: {type: string}
        uniqueItems: {type: boolean}
        examples: {type: array}
    Bad:
      title: false
      description:
      pattern: 7
      format: {}
      uniqueItems: 'true'
      readOnly: null
      writeOnly: 1
      deprecated: []
      contentEncoding: true
      contentMediaType: 42
      examples: {title: false}
"#;
    let findings = validate("annotation-shapes", source);
    let invalid: Vec<_> = findings
        .iter()
        .filter(|d| d.code == "oas-schema-invalid-keyword")
        .collect();
    assert_eq!(invalid.len(), 11, "{findings:?}");
    assert!(
        findings
            .iter()
            .all(|d| d.range.start > source.find("    Bad:").unwrap()),
        "instance data and unknown annotations must be opaque: {findings:?}"
    );
}

#[test]
fn tuple_and_applicator_declarations_allow_impossible_schemas_without_extra_coupling() {
    let source = r#"openapi: 3.1.0
info: {title: applicator declarations, version: '1'}
paths: {}
components:
  schemas:
    Valid:
      prefixItems: [true, {type: string}]
      items: false
      contains: false
      minContains: 3
      maxContains: 1
      if: true
      then: false
      else: false
      allOf: [false]
      anyOf: [false]
      oneOf: [false]
      not: false
      properties:
        prefixItems: {type: string}
    ThenAlone: {then: false}
    MinContainsAlone: {minContains: 3}
    Empty: {prefixItems: []}
    Object: {prefixItems: {type: string}}
    Absent: {prefixItems: }
    Members: {prefixItems: [null, literal, 7]}
"#;
    let findings = validate("tuple-shapes", source);
    assert_eq!(
        findings
            .iter()
            .filter(|d| d.code == "oas-schema-invalid-keyword")
            .count(),
        3,
        "{findings:?}"
    );
    assert_eq!(
        findings
            .iter()
            .filter(|d| d.code == "oas-schema-invalid-kind")
            .count(),
        3,
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|d| d.range.start > source.find("    Empty:").unwrap()),
        "valid impossible schemas must remain valid: {findings:?}"
    );
}

#[test]
fn openapi_30_applies_its_schema_subset_and_array_items_requirement() {
    let source = r#"openapi: 3.0.3
info: {title: old declarations, version: '1'}
paths: {}
components:
  schemas:
    Valid:
      type: array
      items: {}
      nullable: true
      additionalProperties: false
      properties:
        const: {type: string}
        nullable: {type: boolean}
        items: {type: array, items: {type: string}}
      x-annotation: {required: []}
      example: {const: 7, nullable: 'yes'}
      default: {nullable: 'yes'}
    UntypedItems: {items: {}}
    ReferenceSiblings: {$ref: '#/components/schemas/Valid', nullable: 'yes', type: array}
    MissingItems: {type: array}
    BadNullable: {nullable: 'yes'}
    Const: {const: 7}
    Tuple: {prefixItems: [{}]}
    Definitions: {$defs: {Value: {}}}
    Conditional: {if: {type: string}}
    Dialect: {$schema: 'https://json-schema.org/draft/2020-12/schema'}
"#;
    let findings = validate("old-subset", source);
    let invalid: Vec<_> = findings
        .iter()
        .filter(|d| d.code == "oas-schema-invalid-keyword")
        .collect();
    assert_eq!(invalid.len(), 7, "{findings:?}");
    assert!(
        findings
            .iter()
            .all(|d| d.range.start > source.find("    MissingItems:").unwrap()),
        "valid old dialect declarations: {findings:?}"
    );
}

#[test]
fn read_and_write_annotations_are_exclusive_only_in_openapi_30() {
    for (version, count) in [("3.0.3", 1), ("3.1.0", 0)] {
        let source = format!(
            "openapi: {version}\ninfo: {{title: annotations, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Model:\n      properties:\n        secret: {{readOnly: true, writeOnly: true}}\n"
        );
        let findings = validate(&format!("read-write-{version}"), &source);
        assert_eq!(
            findings
                .iter()
                .filter(|d| d.code == "oas-schema-invalid-keyword")
                .count(),
            count,
            "{version}: {findings:?}"
        );
    }
}
