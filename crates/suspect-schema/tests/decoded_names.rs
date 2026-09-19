//! Escaped schema and instance text must have identical semantic meaning.

use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn json(text: &str) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("https://example.test/schema.json").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        Format::Json,
    )
}

#[test]
fn escaped_keywords_property_names_and_reference_paths_preserve_assertions() {
    for (schema, instance, expected) in [
        (
            r#"{"properties":{"a":{"type":"integer"}}}"#,
            r#"{"\u0061":"wrong"}"#,
            false,
        ),
        (
            r#"{"properties":{"\u0061":{"type":"integer"}},"additionalProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"properties":{"\u0061":{"type":"integer"}}}"#,
            r#"{"a":"wrong"}"#,
            false,
        ),
        (
            r##"{"$defs":{"value":{"type":"integer"}},"$r\u0065f":"#/$defs/value"}"##,
            "\"wrong\"",
            false,
        ),
        (
            r##"{"properties":{"\u0061":{"$id":"nested","$defs":{"value":{"$anchor":"value","type":"integer"}},"$ref":"#value"}}}"##,
            r#"{"a":1}"#,
            true,
        ),
        (r#"{"t\u0079pe":"\u0069nteger"}"#, "1", true),
        (r#"{"requ\u0069red":["\u0061"]}"#, r#"{"a":1}"#, true),
        (
            r#"{"dependentRequired":{"\u0061":["\u0062"]}}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"dependentSchemas":{"\u0061":{"required":["b"]}}}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"allOf":[{"properties":{"\u0061":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"patternProperties":{"^\u0061$":{"type":"integer"}},"additionalProperties":false}"#,
            r#"{"\u0061":1}"#,
            true,
        ),
    ] {
        let doc = json(schema);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let value = json(instance);
        assert_eq!(
            compiled.validate(value.root()).is_empty(),
            expected,
            "{schema}: {instance}"
        );
    }
}

#[test]
fn patterns_formats_and_property_name_schemas_use_decoded_strings() {
    for (schema, instance, expected) in [
        (r#"{"pattern":"^\\d+$"}"#, r#""123""#, true),
        (r#"{"pattern":"^a$"}"#, r#""\u0061""#, true),
        (
            r#"{"propertyNames":{"const":"\u0061"}}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"propertyNames":{"enum":["a"]}}"#,
            r#"{"\u0061":1}"#,
            true,
        ),
        (
            r#"{"propertyNames":{"maxLength":1}}"#,
            r#"{"\u0061":1}"#,
            true,
        ),
        (r#"{"format":"\u0069pv4"}"#, r#""invalid""#, false),
        (r#"{"format":"ipv4"}"#, r#""127.0.0.\u0031""#, true),
    ] {
        let doc = json(schema);
        let compiled = Compiler::new(Config {
            format_assertion: true,
            ..Config::default()
        })
        .compile(doc.root())
        .unwrap();
        let value = json(instance);
        assert_eq!(
            compiled.validate(value.root()).is_empty(),
            expected,
            "{schema}: {instance}"
        );
    }
}

#[test]
fn findings_use_decoded_schema_and_instance_pointers() {
    let schema = json(r#"{"properties":{"\u0061":{"t\u0079pe":"integer"}}}"#);
    let instance = json(r#"{"\u0061":"wrong"}"#);
    let compiled = Compiler::new(Config::default())
        .compile(schema.root())
        .unwrap();
    let errors = compiled.validate(instance.root());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].instance_path.to_path(), "/a");
    assert_eq!(errors[0].schema_path.to_path(), "/properties/a/type");
}
