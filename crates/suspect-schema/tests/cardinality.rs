//! Collection bounds using tracked OpenRouter contracts and independent cases.

use suspect_low::{LowDoc, Pointer};
use suspect_schema::{Compiler, Config};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn json(text: &str) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("memory://cardinality.json").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        Format::Json,
    )
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT; missing primary input fails this acceptance test"]
fn openrouter_requires_nonempty_messages_and_pricing_predicates() {
    let root = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    for (relative, format, schema_path, keyword, empty) in [
        (
            "projects/docs/openapi/openapi.yaml",
            Format::Yaml,
            "/components/schemas/ChatRequest/properties/messages",
            "minItems",
            "[]",
        ),
        (
            "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
            Format::Json,
            "/components/schemas/PricingParameterPredicate",
            "minProperties",
            "{}",
        ),
    ] {
        let path = root.join(relative);
        let doc = LowDoc::with_format(
            Uri::from_path(&path).unwrap(),
            Source::from_vec(std::fs::read(&path).expect("read tracked OpenRouter input")),
            format,
        );
        assert!(doc.syntax_errors().is_empty(), "{relative}");
        let node = doc
            .root()
            .pointer(&Pointer::parse(schema_path).unwrap())
            .unwrap();
        assert_eq!(node.get(keyword).unwrap().as_u64(), Some(1));
        // Empty instances exercise the real bound without following item/value
        // references; full reference-context validation is a separate gate.
        let schema = Compiler::new(Config::default()).compile(node).unwrap();
        let instance = json(empty);
        let errors = schema.validate(instance.root());
        assert!(
            errors
                .iter()
                .any(|error| error.schema_path.to_path() == format!("/{keyword}")),
            "{relative} {schema_path} must reject {empty} through {keyword}: {errors:?}"
        );
    }
}

#[test]
fn collection_bounds_are_nonnegative_mathematical_integers() {
    for keyword in [
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
        "minLength",
        "maxLength",
        "minContains",
        "maxContains",
    ] {
        for invalid in [
            "-1",
            "1.0000000000000000000000000001",
            "true",
            "null",
            "\"1\"",
            "[]",
        ] {
            let doc = json(&format!("{{\"{keyword}\":{invalid}}}"));
            assert!(
                Compiler::new(Config::default())
                    .compile(doc.root())
                    .is_err(),
                "{keyword}: {invalid} must not be silently ignored"
            );
        }
    }
}

#[test]
fn size_bounds_beyond_machine_limits_keep_their_comparison_semantics() {
    for (minimum, maximum, value, prefix) in [
        ("minItems", "maxItems", "[1,2]", ""),
        ("minProperties", "maxProperties", r#"{"a":1,"b":2}"#, ""),
        ("minLength", "maxLength", r#""ab""#, ""),
        ("minContains", "maxContains", "[1,2]", r#""contains":true,"#),
    ] {
        for (bound, expected_min, expected_max) in [
            ("2.0", true, true),
            ("20e-1", true, true),
            ("-0.0", true, false),
            ("18446744073709551616", false, true),
            ("1e100000000000000000000000000000000000", false, true),
        ] {
            for (keyword, expected) in [(minimum, expected_min), (maximum, expected_max)] {
                let doc = json(&format!("{{{prefix}\"{keyword}\":{bound}}}"));
                let schema = Compiler::new(Config::default())
                    .compile(doc.root())
                    .unwrap();
                let instance = json(value);
                assert_eq!(
                    schema.validate(instance.root()).is_empty(),
                    expected,
                    "{keyword}: {bound} / {value}"
                );
            }
        }
    }
}

#[test]
fn string_length_counts_decoded_unicode_values() {
    for (schema_text, instance_text, expected) in [
        (r#"{"maxLength":1}"#, r#""\u0061""#, true),
        (r#"{"maxLength":1}"#, r#""\uD834\uDD1E""#, true),
        (r#"{"minLength":3}"#, r#""e\u0301""#, false),
    ] {
        let doc = json(schema_text);
        let schema = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let instance = json(instance_text);
        assert_eq!(
            schema.validate(instance.root()).is_empty(),
            expected,
            "{schema_text}: {instance_text}"
        );
    }
}

#[test]
fn contains_applies_only_to_arrays() {
    for text in [
        r#"{"contains":false}"#,
        r#"{"contains":false,"minContains":2}"#,
    ] {
        let doc = json(text);
        let instances: Vec<_> = ["null", "true", "7", "\"text\"", "{}"]
            .into_iter()
            .map(|literal| (literal, json(literal)))
            .collect();
        let schema = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        for (literal, instance) in &instances {
            assert!(
                schema.validate(instance.root()).is_empty(),
                "{text}: {literal}"
            );
        }
        let instance = json("[]");
        assert!(!schema.validate(instance.root()).is_empty(), "{text}: []");
    }
}
