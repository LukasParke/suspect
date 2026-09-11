//! Annotation applicability across successful schema evaluation scopes.

use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn json(text: &str) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("memory://annotations.json").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        Format::Json,
    )
}

#[test]
fn only_successful_in_place_applicators_contribute_annotations() {
    for (schema, instance, expected) in [
        (
            r#"{"anyOf":[{"properties":{"a":true},"required":["missing"]},true],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"allOf":[{"properties":{"a":true}},{"unevaluatedProperties":false}]}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"allOf":[{"unevaluatedProperties":false},{"properties":{"a":true}}]}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"allOf":[{"properties":{"a":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"anyOf":[{"prefixItems":[true],"minItems":2},true],"unevaluatedItems":false}"#,
            "[1]",
            false,
        ),
        (
            r#"{"allOf":[{"prefixItems":[true]},{"unevaluatedItems":false}]}"#,
            "[1]",
            false,
        ),
        (
            r#"{"not":{"not":{"properties":{"a":true}}},"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"required":["a"],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            false,
        ),
        (
            r#"{"allOf":[{"properties":{"a":{"properties":{"x":true}}}},{"properties":{"a":{"unevaluatedProperties":false}}}]}"#,
            r#"{"a":{"x":1}}"#,
            false,
        ),
    ] {
        let doc = json(schema);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let value = json(instance);
        let errors = compiled.validate(value.root());
        assert_eq!(
            errors.is_empty(),
            expected,
            "{schema}: {instance}: {errors:?}"
        );
    }
}

#[test]
fn true_applicators_and_successful_conditions_keep_their_annotations() {
    for (schema, instance, expected) in [
        (
            r#"{"additionalProperties":true,"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"allOf":[{"unevaluatedProperties":true}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"allOf":[{"unevaluatedItems":true}],"unevaluatedItems":false}"#,
            "[1]",
            true,
        ),
        (
            r#"{"if":{"properties":{"a":true}},"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            true,
        ),
        (
            r#"{"if":{"properties":{"a":true},"required":["missing"]},"else":true,"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            false,
        ),
    ] {
        let doc = json(schema);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let value = json(instance);
        let errors = compiled.validate(value.root());
        assert_eq!(
            errors.is_empty(),
            expected,
            "{schema}: {instance}: {errors:?}"
        );
    }
}

#[test]
fn branch_error_limits_do_not_hide_the_result_of_a_union() {
    for alternative in ["true", "false"] {
        let doc = json(&format!(
            r#"{{"anyOf":[{{"properties":{{"a":{{"type":"integer"}},"b":{{"type":"integer"}},"c":{{"type":"integer"}}}}}},{alternative}]}}"#
        ));
        let compiled = Compiler::new(Config {
            max_errors: 1,
            ..Config::default()
        })
        .compile(doc.root())
        .unwrap();
        let instance = json(r#"{"a":"x","b":"x","c":"x"}"#);
        assert_eq!(
            compiled.validate(instance.root()).is_empty(),
            alternative == "true",
            "{alternative}"
        );
        assert_eq!(
            compiled.validate_first(instance.root()).is_none(),
            alternative == "true",
            "{alternative}"
        );
    }
}
