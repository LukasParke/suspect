//! RFC 8259 numeric syntax through the public, explicitly JSON CST parser.

use suspect_source::Source;
use suspect_syntax::{Format, SourceDoc};

fn document(literal: &str) -> SourceDoc {
    SourceDoc::with_format(
        "memory://numbers.json".into(),
        Source::from_vec(format!("{{\"value\":{literal}}}").into_bytes()),
        Format::Json,
    )
}

#[test]
fn json_numbers_accept_both_exponent_signs_and_require_fraction_digits() {
    for literal in [
        "0",
        "-0",
        "1",
        "-1",
        "123456789012345678901234567890",
        "0.0",
        "1e400",
        "1e-400",
        "1e+400",
        "-1e-400",
        "-1e+400",
        "1.25e+400",
        "-1.25E-400",
    ] {
        let doc = document(literal);
        assert!(!doc.has_errors(), "{literal}: {:?}", doc.errors());
        let value = doc.root().content().get(b"value").unwrap();
        assert_eq!(value.scalar_bytes(), literal.as_bytes());
    }
    for literal in [
        "+1", "01", "-01", ".1", "1.", "1.e2", "1e", "1e+", "1e-", "1.2e+", "--1", "1+2", "NaN",
        "Infinity", "0x1", "0o7", "1_000",
    ] {
        let doc = document(literal);
        assert!(
            doc.has_errors(),
            "invalid JSON numeric spelling {literal} was accepted"
        );
    }
}
