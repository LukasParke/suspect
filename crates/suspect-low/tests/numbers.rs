//! Numeric spelling and bounded conversion through the public low-model view.

use suspect_low::{LowDoc, ValueKind};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn document(text: &str, format: Format) -> LowDoc {
    LowDoc::with_format(
        Uri::parse("memory://numeric-fixture").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        format,
    )
}

#[test]
fn signed_json_exponents_remain_numeric_without_f64_materialization() {
    // RFC 8259 section 6 allows a sign on the exponent. Its magnitude is not
    // restricted to IEEE-754's range, and Float here describes the spelling.
    for literal in [
        "1e-400",
        "1e+400",
        "-1e-400",
        "-1e+400",
        "1.25e+400",
        "-1.25E-400",
    ] {
        let doc = document(&format!("{{\"value\":{literal}}}"), Format::Json);
        assert!(
            doc.syntax_errors().is_empty(),
            "{literal}: {:?}",
            doc.syntax_errors()
        );
        let value = doc.root().get("value").unwrap();
        assert_eq!(value.kind(), ValueKind::Float, "{literal}");
        assert_eq!(value.syntax().scalar_bytes(), literal.as_bytes());
        assert_eq!(
            value.as_i64(),
            None,
            "floating spelling is not an integer accessor input"
        );
    }
}

#[test]
fn yaml_signed_integer_limits_convert_without_overflow() {
    for (literal, expected) in [
        ("-9223372036854775808", Some(i64::MIN)),
        ("-0x8000000000000000", Some(i64::MIN)),
        ("-0o1000000000000000000000", Some(i64::MIN)),
        ("9223372036854775807", Some(i64::MAX)),
        ("+0x7fffffffffffffff", Some(i64::MAX)),
        ("+0o777777777777777777777", Some(i64::MAX)),
        ("9223372036854775808", None),
        ("+0x8000000000000000", None),
        ("0o1000000000000000000000", None),
        ("-9223372036854775809", None),
        ("-0x8000000000000001", None),
        ("-0o1000000000000000000001", None),
        ("184467440737095516160000000000000000000", None),
        ("-0xffffffffffffffffffffffff", None),
    ] {
        let doc = document(&format!("value: {literal}\n"), Format::Yaml);
        assert!(doc.syntax_errors().is_empty(), "{literal}");
        let value = doc.root().get("value").unwrap();
        assert_eq!(value.kind(), ValueKind::Int, "{literal}");
        assert_eq!(value.as_i64(), expected, "{literal}");
        assert_eq!(value.syntax().scalar_bytes(), literal.as_bytes());
    }
}

#[test]
fn unsigned_integer_conversion_uses_the_full_u64_range() {
    for format in [Format::Json, Format::Yaml] {
        for (literal, expected) in [
            ("9223372036854775808", Some(1_u64 << 63)),
            ("18446744073709551615", Some(u64::MAX)),
            ("18446744073709551616", None),
            ("-1", None),
            ("-0", Some(0)),
            ("1.0", None),
            ("\"18446744073709551615\"", None),
        ] {
            let text = match format {
                Format::Json => format!("{{\"value\":{literal}}}"),
                Format::Yaml => format!("value: {literal}\n"),
            };
            let doc = document(&text, format);
            assert!(doc.syntax_errors().is_empty(), "{format}: {literal}");
            assert_eq!(
                doc.root().get("value").unwrap().as_u64(),
                expected,
                "{format}: {literal}"
            );
        }
    }
    for (literal, expected) in [
        ("+18446744073709551615", Some(u64::MAX)),
        ("0xffffffffffffffff", Some(u64::MAX)),
        ("0o1777777777777777777777", Some(u64::MAX)),
        ("-0x0", Some(0)),
        ("-0x1", None),
        ("0x10000000000000000", None),
        ("0o2000000000000000000000", None),
    ] {
        let doc = document(&format!("value: {literal}\n"), Format::Yaml);
        assert!(doc.syntax_errors().is_empty(), "{literal}");
        assert_eq!(
            doc.root().get("value").unwrap().as_u64(),
            expected,
            "{literal}"
        );
    }
}

#[test]
fn numeric_kind_is_separate_from_i64_range_and_quoted_text() {
    for format in [Format::Json, Format::Yaml] {
        for (literal, expected) in [
            ("-9223372036854775808", Some(i64::MIN)),
            ("9223372036854775807", Some(i64::MAX)),
            ("9223372036854775808", None),
            ("-9223372036854775809", None),
            ("184467440737095516160000000000000000000", None),
        ] {
            let text = match format {
                Format::Json => format!("{{\"value\":{literal}}}"),
                Format::Yaml => format!("value: {literal}\n"),
            };
            let doc = document(&text, format);
            assert!(doc.syntax_errors().is_empty());
            let value = doc.root().get("value").unwrap();
            assert_eq!(value.kind(), ValueKind::Int, "{format}: {literal}");
            assert_eq!(value.as_i64(), expected, "{format}: {literal}");
        }
        for literal in ["1e+400", "-9223372036854775808"] {
            let text = match format {
                Format::Json => format!("{{\"value\":\"{literal}\"}}"),
                Format::Yaml => format!("value: '{literal}'\n"),
            };
            let doc = document(&text, format);
            let value = doc.root().get("value").unwrap();
            assert_eq!(value.kind(), ValueKind::Str);
            assert_eq!(value.as_i64(), None);
        }
    }
}

#[test]
fn malformed_json_numeric_spellings_are_not_accepted_as_valid_input() {
    for literal in [
        "01", "+1", "1.", "1.e2", "1e", "1e+", "1e-", "1+2", "1_000", "0x10",
    ] {
        let doc = document(&format!("{{\"value\":{literal}}}"), Format::Json);
        assert!(!doc.syntax_errors().is_empty(), "{literal}");
    }
    // YAML's own numeric grammar remains independent from JSON's restrictions.
    for (literal, kind) in [
        ("+1", ValueKind::Int),
        ("01", ValueKind::Int),
        ("1.", ValueKind::Float),
        (".1", ValueKind::Float),
    ] {
        let doc = document(&format!("value: {literal}\n"), Format::Yaml);
        assert!(doc.syntax_errors().is_empty(), "{literal}");
        assert_eq!(doc.root().get("value").unwrap().kind(), kind, "{literal}");
    }
}

#[test]
fn mathematical_integrality_is_exact_for_decimal_and_exponent_spellings() {
    for format in [Format::Json, Format::Yaml] {
        for (literal, expected) in [
            ("1.0", true),
            ("-1.000", true),
            ("1e400", true),
            ("1e-400", false),
            ("1.0000000000000000000000000001", false),
            ("1.25e2", true),
            ("1.25e1", false),
            ("1200.0e-2", true),
            ("1200.0e-3", false),
            ("0.000e-99999999999999999999999999", true),
            ("1e9999999999999999999999999999", true),
            ("1e-9999999999999999999999999999", false),
            ("184467440737095516160000000000000000000", true),
            ("\"1.0\"", false),
            ("true", false),
            ("null", false),
            ("[]", false),
            ("{}", false),
        ] {
            let text = match format {
                Format::Json => format!("{{\"value\":{literal}}}"),
                Format::Yaml => format!("value: {literal}\n"),
            };
            let doc = document(&text, format);
            assert!(doc.syntax_errors().is_empty(), "{format}: {literal}");
            assert_eq!(
                doc.root().get("value").unwrap().is_integral_number(),
                expected,
                "{format}: {literal}"
            );
        }
    }
    for (literal, expected) in [
        ("+0xffffffffffffffffffffffff", true),
        ("-0o777777777777777777777777", true),
        ("100.e-2", true),
        ("100.e-3", false),
        (".0", true),
        (".001", false),
        ("+.inf", false),
        ("-.INF", false),
        (".NaN", false),
        ("1_000", false),
    ] {
        let doc = document(&format!("value: {literal}\n"), Format::Yaml);
        assert!(doc.syntax_errors().is_empty(), "{literal}");
        assert_eq!(
            doc.root().get("value").unwrap().is_integral_number(),
            expected,
            "{literal}"
        );
    }
    let doc = document("source: &number 1.00e400\nvalue: *number\n", Format::Yaml);
    assert!(doc.root().get("value").unwrap().is_integral_number());
}
