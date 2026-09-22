//! Numeric behavior through the public compiler/validator seam.

use suspect_low::LowDoc;
use suspect_schema::{CompileError, Compiler, Config, SchemaErrorKind};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn doc(text: &str, format: Format) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("memory://numeric.json").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        format,
    )
}

fn accepts(schema: &str, instance: &str) -> bool {
    let schema_doc = doc(schema, Format::Json);
    let instance_doc = doc(instance, Format::Json);
    let compiled = Compiler::new(Config::default())
        .compile(schema_doc.root())
        .unwrap();
    compiled.validate(instance_doc.root()).is_empty()
}

#[test]
fn integer_type_uses_the_decimal_value_without_underflow() {
    // The shape of OpenRouter's ChatChoice.index, with independent values.
    let schema = r#"{"description":"Choice index","example":0,"type":"integer"}"#;
    assert!(!accepts(schema, "1e-400"));
    assert!(!accepts(schema, "-1e-400"));
    assert!(accepts(schema, "1e+400"));
    assert!(accepts(schema, "100e-2"));
    assert!(accepts(schema, "0e-9999999999999999999999999999999"));
}

#[test]
fn bounds_distinguish_adjacent_wide_numbers_and_accept_wide_schema_bounds() {
    for (schema, instance, expected) in [
        (r#"{"maximum":0}"#, "9223372036854775808", false),
        (
            r#"{"maximum":9007199254740992.0}"#,
            "9007199254740993",
            false,
        ),
        (r#"{"maximum":18446744073709551616}"#, "0", true),
        (
            r#"{"maximum":18446744073709551616}"#,
            "18446744073709551616",
            true,
        ),
        (
            r#"{"exclusiveMaximum":18446744073709551616}"#,
            "18446744073709551616",
            false,
        ),
        (
            r#"{"minimum":-18446744073709551616}"#,
            "-18446744073709551617",
            false,
        ),
        (
            r#"{"exclusiveMinimum":-18446744073709551616}"#,
            "-18446744073709551615",
            true,
        ),
        (r#"{"maximum":1e400}"#, "9e399", true),
        (r#"{"maximum":1e400}"#, "11e399", false),
        (r#"{"maximum":1e-400}"#, "2e-400", false),
    ] {
        assert_eq!(accepts(schema, instance), expected, "{schema}: {instance}");
    }
}

#[test]
fn multiples_require_an_exact_integer_quotient() {
    for (divisor, instance, expected) in [
        ("0.01", "0.070000000001", false),
        ("0.01", "0.07", true),
        ("0.125", "0.375", true),
        ("0.125", "0.37", false),
        ("1.5", "-4.5", true),
        (
            "3",
            "1000000000000000000000000000000000000000000000000000000000000000",
            false,
        ),
        ("0.03", "0.3", true),
        ("0.03", "0.1", false),
        ("1e-400", "1", true),
        ("1e400", "1e399", false),
        ("1e400", "2e400", true),
        ("18446744073709551616", "36893488147419103232", true),
        ("18446744073709551616", "36893488147419103233", false),
    ] {
        let schema = format!(r#"{{"multipleOf":{divisor}}}"#);
        assert_eq!(accepts(&schema, instance), expected, "{schema}: {instance}");
    }
}

#[test]
fn enum_and_const_use_exact_numeric_equality_in_nested_values() {
    for keyword in ["enum", "const"] {
        for (value, instance, expected) in [
            ("9007199254740992", "9007199254740993", false),
            ("1e400", "2e400", false),
            ("1e-400", "0", false),
            ("1e400", "10e399", true),
            ("-0", "0.000e-9999999999999999999999999999999", true),
            (
                r#"{"a":[9007199254740992,1],"b":0}"#,
                r#"{"b":-0.0,"a":[9007199254740992.0,10e-1]}"#,
                true,
            ),
            (
                r#"{"a":[9007199254740992]}"#,
                r#"{"a":[9007199254740993]}"#,
                false,
            ),
        ] {
            let operand = if keyword == "enum" {
                format!("[{value}]")
            } else {
                value.to_owned()
            };
            let schema = format!(r#"{{"{keyword}":{operand}}}"#);
            assert_eq!(accepts(&schema, instance), expected, "{schema}: {instance}");
        }
    }
}

#[test]
fn unique_items_shares_the_exact_value_equality_rules() {
    for (instance, expected) in [
        ("[1,1.0]", false),
        ("[9007199254740992,9007199254740993]", true),
        ("[0,1e-400]", true),
        ("[1e400,2e400]", true),
        ("[1e400,10e399]", false),
        (r#"[{"a":[1,0],"b":2},{"b":2.0,"a":[10e-1,-0]}]"#, false),
        (r#"[[1,2],[2,1]]"#, true),
        ("[]", true),
        ("42", true),
    ] {
        assert_eq!(
            accepts(r#"{"uniqueItems":true}"#, instance),
            expected,
            "{instance}"
        );
    }
    assert!(accepts(r#"{"uniqueItems":false}"#, "[1,1]"));
}

#[test]
fn thousand_digit_exponents_are_compared_and_divided_without_expansion() {
    let power = format!("1{}", "0".repeat(1000));
    let previous = "9".repeat(1000);
    for (schema, instance, expected) in [
        (
            format!(r#"{{"maximum":1e{power}}}"#),
            format!("9e{previous}"),
            true,
        ),
        (
            format!(r#"{{"maximum":1e{power}}}"#),
            format!("11e{previous}"),
            false,
        ),
        (
            format!(r#"{{"const":1e{power}}}"#),
            format!("10e{previous}"),
            true,
        ),
        (
            format!(r#"{{"const":10e-{power}}}"#),
            format!("1e-{previous}"),
            true,
        ),
        (
            format!(r#"{{"minimum":1e-{power}}}"#),
            format!("1e-{previous}"),
            true,
        ),
        (
            r#"{"multipleOf":0.125}"#.to_owned(),
            format!("1e{power}"),
            true,
        ),
        (
            r#"{"multipleOf":3}"#.to_owned(),
            format!("1e{power}"),
            false,
        ),
        (
            format!(r#"{{"multipleOf":1e-{power}}}"#),
            format!("1e-{previous}"),
            true,
        ),
        (
            format!(r#"{{"multipleOf":1e-{previous}}}"#),
            format!("1e-{power}"),
            false,
        ),
        (
            format!(r#"{{"multipleOf":1e{power}}}"#),
            format!("-0e-{power}"),
            true,
        ),
    ] {
        assert_eq!(
            accepts(&schema, &instance),
            expected,
            "astronomical exponent case"
        );
    }
}

#[test]
fn numeric_resource_failures_survive_logical_branches_and_error_caps() {
    let number = "1".repeat(40);
    let instance = doc(&number, Format::Json);
    for schema in [
        r#"{"maximum":0}"#,
        r#"{"not":{"maximum":0}}"#,
        r#"{"anyOf":[true,{"maximum":0}]}"#,
        r#"{"oneOf":[true,{"maximum":0}]}"#,
        r#"{"if":{"maximum":0},"then":true,"else":true}"#,
        r#"{"not":{"const":0}}"#,
        r#"{"type":"string","maximum":0}"#,
    ] {
        let schema_doc = doc(schema, Format::Json);
        let compiled = Compiler::new(Config {
            max_number_bytes: 32,
            max_errors: 1,
            ..Config::default()
        })
        .compile(schema_doc.root())
        .unwrap();
        let errors = compiled.validate(instance.root());
        assert_eq!(errors.len(), 1, "{schema}: {errors:?}");
        assert_eq!(errors[0].kind, SchemaErrorKind::Evaluation, "{schema}");
        assert!(errors[0].message.contains("32 source bytes"));
        assert_eq!(
            compiled.validate_first(instance.root()),
            Some(errors[0].clone())
        );
    }
    for keyword in [
        "maximum",
        "minimum",
        "exclusiveMaximum",
        "exclusiveMinimum",
        "multipleOf",
    ] {
        let schema_doc = doc(&format!(r#"{{"{keyword}":{number}}}"#), Format::Json);
        let compiler = Compiler::new(Config {
            max_number_bytes: 32,
            ..Config::default()
        });
        assert!(matches!(
            compiler.compile(schema_doc.root()),
            Err(CompileError::ResourceLimit { .. })
        ));
    }
}

#[test]
fn equality_resource_failures_are_not_inequality() {
    let instance = doc("[1,2,3,4]", Format::Json);
    for schema in [
        r#"{"uniqueItems":true}"#,
        r#"{"not":{"uniqueItems":true}}"#,
        r#"{"not":{"const":[1,2,3,4]}}"#,
        r#"{"anyOf":[true,{"const":[1,2,3,4]}]}"#,
        r#"{"if":{"enum":[[1,2,3,4]]},"then":true,"else":true}"#,
    ] {
        let schema_doc = doc(schema, Format::Json);
        let compiled = Compiler::new(Config {
            max_equality_steps: 3,
            ..Config::default()
        })
        .compile(schema_doc.root())
        .unwrap();
        let first = compiled
            .validate_first(instance.root())
            .expect("evaluation must fail");
        assert_eq!(first.kind, SchemaErrorKind::Evaluation, "{schema}");
        assert!(first.message.contains("3 node comparisons"));
    }
    let schema_doc = doc(r#"{"not":{"const":[[[[1]]]]}}"#, Format::Json);
    let instance = doc("[[[[1]]]]", Format::Json);
    let compiled = Compiler::new(Config {
        max_depth: 2,
        ..Config::default()
    })
    .compile(schema_doc.root())
    .unwrap();
    let first = compiled
        .validate_first(instance.root())
        .expect("depth must fail");
    assert_eq!(first.kind, SchemaErrorKind::Evaluation);
    assert!(
        first
            .message
            .contains("equality evaluation depth exceeds 2")
    );
}

#[test]
fn lazy_reference_numeric_compile_limits_remain_evaluation_failures() {
    let number = "1".repeat(40);
    let instance = doc("0", Format::Json);
    for reference in [
        r##"{"$ref":"#/$defs/limited"}"##,
        r##"{"$dynamicRef":"#limited"}"##,
    ] {
        let schema_doc = doc(
            &format!(
                r##"{{"$defs":{{"limited":{{"$dynamicAnchor":"limited","maximum":{number}}}}},"not":{reference}}}"##
            ),
            Format::Json,
        );
        let compiled = Compiler::new(Config {
            max_number_bytes: 32,
            ..Config::default()
        })
        .compile(schema_doc.root())
        .unwrap();
        // Exercise both the initial lazy compilation and its cached result.
        for _ in 0..2 {
            let first = compiled
                .validate_first(instance.root())
                .expect("lazy compilation limit must fail");
            assert_eq!(first.kind, SchemaErrorKind::Evaluation);
            assert!(first.message.contains("32 source bytes"));
        }
    }
}

#[test]
fn yaml_radix_and_decimal_spellings_preserve_their_numeric_value() {
    for (schema, instance, valid) in [
        ("const: 0x20000000000001", "9007199254740993", true),
        ("maximum: 0x20000000000000", "9007199254740993", false),
        (
            "minimum: -0x10000000000000001",
            "-18446744073709551617",
            true,
        ),
        (
            "const: 0o1777777777777777777777",
            "18446744073709551615",
            true,
        ),
        ("multipleOf: +.125", ".375", true),
        ("multipleOf: .125", ".37", false),
        ("const: 1.", "+1e0", true),
    ] {
        let schema_doc = doc(schema, Format::Yaml);
        let instance = doc(instance, Format::Yaml);
        let compiled = Compiler::new(Config::default())
            .compile(schema_doc.root())
            .unwrap();
        assert_eq!(
            compiled.validate(instance.root()).is_empty(),
            valid,
            "{schema}"
        );
    }
}

#[test]
fn nonfinite_yaml_values_are_explicit_numeric_evaluation_failures() {
    for literal in [".inf", "-.inf", "+.inf", ".nan"] {
        let instance = doc(literal, Format::Yaml);
        for schema in [
            r#"{"type":"number"}"#,
            r#"{"not":{"type":"number"}}"#,
            r#"{"maximum":0}"#,
            r#"{"not":{"enum":[0]}}"#,
        ] {
            let schema_doc = doc(schema, Format::Json);
            let compiled = Compiler::new(Config::default())
                .compile(schema_doc.root())
                .unwrap();
            let error = compiled
                .validate_first(instance.root())
                .expect("JSON numbers must be finite");
            assert_eq!(
                error.kind,
                SchemaErrorKind::Evaluation,
                "{schema}: {literal}"
            );
        }
        for keyword in ["maximum", "minimum", "multipleOf"] {
            let schema_doc = doc(&format!("{keyword}: {literal}"), Format::Yaml);
            assert!(matches!(
                Compiler::new(Config::default()).compile(schema_doc.root()),
                Err(CompileError::Invalid { .. })
            ));
        }
    }
}

#[test]
fn equality_compares_decoded_strings_and_object_keys() {
    for (schema, instance, valid) in [
        (r#"{"const":"a"}"#, r#""\u0061""#, true),
        (r#"{"enum":["a\nb","𝄞"]}"#, r#""\uD834\uDD1E""#, true),
        (
            r#"{"const":{"a":["b",9007199254740993]}}"#,
            r#"{"\u0061":["\u0062",9007199254740993.0]}"#,
            true,
        ),
        (r#"{"uniqueItems":true}"#, r#"["a","\u0061"]"#, false),
        (
            r#"{"uniqueItems":true}"#,
            r#"[{"a":1},{"\u0061":1.0}]"#,
            false,
        ),
    ] {
        assert_eq!(accepts(schema, instance), valid, "{schema}: {instance}");
    }
    let schema = doc("enum:\n  - &text 'it''s fine'\n  - *text\n", Format::Yaml);
    let instance = doc(r#""it's fine""#, Format::Json);
    let compiled = Compiler::new(Config::default())
        .compile(schema.root())
        .unwrap();
    assert!(compiled.validate(instance.root()).is_empty());
}

#[test]
fn malformed_or_ambiguous_text_is_an_evaluation_failure() {
    for (schema, instance) in [
        (r#"{"not":{"const":"text"}}"#, r#""\uD800""#),
        (
            r#"{"not":{"const":{"a":1,"b":1}}}"#,
            r#"{"a":1,"\u0061":1}"#,
        ),
        (
            r#"{"not":{"const":{"a":1,"\u0061":1}}}"#,
            r#"{"a":1,"b":1}"#,
        ),
    ] {
        let schema_doc = doc(schema, Format::Json);
        let instance_doc = doc(instance, Format::Json);
        let compiled = Compiler::new(Config::default())
            .compile(schema_doc.root())
            .unwrap();
        let error = compiled
            .validate_first(instance_doc.root())
            .expect("evaluation must fail");
        assert_eq!(
            error.kind,
            SchemaErrorKind::Evaluation,
            "{schema}: {instance}"
        );
    }
}

#[test]
#[ignore = "requires OPENROUTER_PUBLIC_SCHEMA pointing to the pinned public OpenAPI snapshot"]
fn openrouter_chat_choice_index_rejects_a_nonzero_fraction() {
    let path = std::env::var("OPENROUTER_PUBLIC_SCHEMA")
        .expect("set OPENROUTER_PUBLIC_SCHEMA; absence is not acceptance");
    let bytes = std::fs::read(&path).expect("read OpenRouter public OpenAPI snapshot");
    let api = LowDoc::with_format(
        Uri::from_path(std::path::Path::new(&path)).unwrap(),
        Source::from_vec(bytes),
        Format::Yaml,
    );
    let index = api
        .root()
        .get("components")
        .unwrap()
        .get("schemas")
        .unwrap()
        .get("ChatChoice")
        .unwrap()
        .get("properties")
        .unwrap()
        .get("index")
        .unwrap();
    assert_eq!(index.get("type").unwrap().as_str(), Some("integer"));
    assert_eq!(
        index.get("description").unwrap().as_str(),
        Some("Choice index")
    );
    let cases: Vec<_> = [("1e-400", false), ("0", true), ("1e400", true)]
        .into_iter()
        .map(|(literal, valid)| (literal, valid, doc(literal, Format::Json)))
        .collect();
    let compiled = Compiler::new(Config::default()).compile(index).unwrap();
    for (literal, valid, instance) in &cases {
        assert_eq!(
            compiled.validate(instance.root()).is_empty(),
            *valid,
            "{literal}"
        );
    }
}
