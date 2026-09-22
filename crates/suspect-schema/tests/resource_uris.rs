//! Public URI-reference syntax and resource-identity regressions.

use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config, SchemaErrorKind};
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
fn malformed_uri_references_are_rejected_before_resource_resolution() {
    // These are JSON string literals, so the schema parser also exercises
    // decoding before URI syntax validation. None is a valid RFC 3986
    // URI-reference, even where a browser URL parser repairs the spelling.
    for literal in [
        r#"" leading""#,
        r#""trailing ""#,
        r#""inside space""#,
        r#""tab\tinside""#,
        r#""line\ninside""#,
        r#""sub\\resource""#,
        r#""café""#,
        r#""bad%""#,
        r#""bad%2""#,
        r#""bad%GG""#,
        r#""1invalid:scheme""#,
        r#""https://[broken/resource""#,
        r#""https://[vG.invalid]/resource""#,
        r#""https://example.test:port/resource""#,
        r#""https://external.test/resource#bad%""#,
        r#""https://external.test/resource#two#fragments""#,
    ] {
        for keyword in ["$id", "$ref", "$dynamicRef"] {
            let source = format!(r#"{{"{keyword}":{literal}}}"#);
            let doc = json(&source);
            assert!(
                Compiler::new(Config::default())
                    .compile(doc.root())
                    .is_err(),
                "malformed `{keyword}` accepted: {literal}",
            );
        }
    }
}

#[test]
fn resource_resolution_preserves_rfc3986_identifier_components() {
    for source in [
        // Relative resolution removes literal dot segments exactly once.
        r##"{"$id":"https://example.test/a/b/root.json","$defs":{"target":{"$id":"../shared/target","type":"string"}},"$ref":"../shared/./target"}"##,
        // An opaque base supports the RFC query-only resolution operation.
        r##"{"$id":"urn:example:schema","$defs":{"target":{"$id":"?version=two","type":"string"}},"$ref":"?version=two"}"##,
        r##"{"$defs":{"target":{"$id":"urn:example:target","type":"string"}},"$ref":"urn:example:target"}"##,
        // Encoded dots remain encoded path data; they are not dot segments.
        r##"{"$defs":{"target":{"$id":"https://example.test/a/%2E%2E/value","type":"string"},"other":{"$id":"https://example.test/value","type":"integer"}},"$ref":"https://example.test/a/%2E%2E/value"}"##,
        // Encoded slashes and hash signs do not become URI delimiters.
        r##"{"$defs":{"target":{"$id":"target%2Fpart","type":"string"},"other":{"$id":"target/part","type":"integer"}},"$ref":"target%2Fpart"}"##,
        r##"{"$defs":{"target":{"$id":"target%23part","$anchor":"value","type":"string"}},"$ref":"target%23part#%76alue"}"##,
        // Query syntax is retained, including '?', '+', and encoded delimiters.
        r##"{"$defs":{"target":{"$id":"?q=a%2Fb+z&x=%23?ok","type":"string"},"other":{"$id":"?q=a/b+z&x=%23?ok","type":"integer"}},"$ref":"?q=a%2Fb+z&x=%23?ok"}"##,
        // RFC 3986 section 5.4.2: an explicit scheme is absolute, even when
        // it matches the base scheme and is not followed by '//'.
        r##"{"$id":"http://example.test/root","$defs":{"target":{"$id":"http:g","type":"string"},"other":{"$id":"g","type":"integer"}},"$ref":"http:g"}"##,
        // Percent decoding the JSON Pointer fragment happens after splitting
        // the URI into its document and fragment components.
        r##"{"$defs":{"target":{"type":"string"}},"$ref":"#%2F$defs%2Ftarget"}"##,
    ] {
        let doc = json(source);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let good = json(r#""hello""#);
        let bad = json("42");
        assert!(compiled.validate(good.root()).is_empty(), "{source}");
        let errors = compiled.validate(bad.root());
        assert!(!errors.is_empty(), "{source}");
        assert!(
            errors
                .iter()
                .all(|error| error.kind == SchemaErrorKind::Invalid),
            "valid reference did not resolve: {source}: {errors:?}",
        );
    }
}

#[test]
fn unrepresentable_relative_resolution_is_rejected_without_rewriting_identity() {
    // RFC resolution would produce an authorityless path starting with '//'.
    // Serializing it as a new authority or silently injecting '/.' would
    // change the resource's identity. The URI helper reports the failure.
    let doc = json(r#"{"$id":"scheme:","$defs":{"target":{"$id":".///path"}}}"#);
    assert!(
        Compiler::new(Config::default())
            .compile(doc.root())
            .is_err()
    );
}

#[test]
fn resource_identity_folds_scheme_and_host_case_only() {
    let doc = json(
        r##"{"$defs":{"target":{"$id":"HTTPS://User:Pass@EXAMPLE.test/Target?Key=Value","type":"string"},"other":{"$id":"https://User:Pass@example.test/target?Key=Value","type":"integer"}},"$ref":"https://User:Pass@example.test/Target?Key=Value"}"##,
    );
    let compiled = Compiler::new(Config::default())
        .compile(doc.root())
        .unwrap();
    let good = json(r#""hello""#);
    assert!(compiled.validate(good.root()).is_empty());

    // User information, paths, and queries are not ASCII-case folded.
    for reference in [
        "https://user:Pass@example.test/Target?Key=Value",
        "https://User:Pass@example.test/target?Key=Value",
        "https://User:Pass@example.test/Target?key=Value",
    ] {
        let source = format!(
            r##"{{"$defs":{{"target":{{"$id":"HTTPS://User:Pass@EXAMPLE.test/Target?Key=Value","type":"string"}}}},"$ref":"{reference}"}}"##,
        );
        let doc = json(&source);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let good = json(r#""hello""#);
        assert_eq!(
            compiled
                .validate(good.root())
                .first()
                .map(|error| error.kind),
            Some(SchemaErrorKind::Evaluation),
            "distinct resource identifier was case-folded: {reference}",
        );
    }
}

#[test]
fn retrieval_alias_uses_the_same_validated_resource_identity() {
    let doc = LowDoc::with_format(
        Uri::from("URN:example:schema"),
        Source::from_vec(
            br##"{"$defs":{"target":{"type":"string"}},"$ref":"urn:example:schema#/$defs/target"}"##.to_vec(),
        ),
        Format::Json,
    );
    let compiled = Compiler::new(Config::default())
        .compile(doc.root())
        .unwrap();
    let good = json(r#""hello""#);
    assert!(compiled.validate(good.root()).is_empty());

    let malformed = LowDoc::with_format(
        Uri::from("invalid URI"),
        Source::from_vec(b"true".to_vec()),
        Format::Json,
    );
    assert!(
        Compiler::new(Config::default())
            .compile(malformed.root())
            .is_err()
    );
}
