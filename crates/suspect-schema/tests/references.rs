//! Resource identity, dynamic scope and non-invertible resolution failures.

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
fn resolution_failures_cannot_be_inverted_or_hidden_by_branches() {
    for reference in ["#/$defs/missing", "https://external.test/schema.json"] {
        for template in [
            r#"{"not":{"$ref":"REF"}}"#,
            r#"{"anyOf":[true,{"$ref":"REF"}]}"#,
            r#"{"if":{"$ref":"REF"},"else":true}"#,
            r#"{"propertyNames":{"not":{"$ref":"REF"}}}"#,
        ] {
            let text = template.replace("REF", reference);
            let doc = json(&text);
            let compiled = Compiler::new(Config::default())
                .compile(doc.root())
                .unwrap();
            let instance = json(r#"{"name":1}"#);
            assert_eq!(
                compiled.validate(instance.root()).first().map(|e| e.kind),
                Some(SchemaErrorKind::Evaluation),
                "{text}"
            );
            assert_eq!(
                compiled.validate_first(instance.root()).map(|e| e.kind),
                Some(SchemaErrorKind::Evaluation),
                "{text}"
            );
        }
    }
}

#[test]
fn anchors_are_scoped_to_resources_and_instance_data_cannot_register_them() {
    for schema in [
        r##"{"$defs":{"a":{"$id":"a","$anchor":"value","type":"integer"},"b":{"$id":"b","$anchor":"value","type":"string"}},"$ref":"b#value"}"##,
        r##"{"$defs":{"a":{"$anchor":"value","type":"string"}},"examples":[{"$anchor":"value","type":"integer"}],"$ref":"#value"}"##,
        r##"{"$defs":{"a":{"$id":"sub/","$defs":{"b":{"$anchor":"value","type":"string"}},"$ref":"#value"}},"$ref":"sub/"}"##,
    ] {
        let doc = json(schema);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let good = json(r#""hello""#);
        let bad = json("42");
        assert!(compiled.validate(good.root()).is_empty(), "{schema}");
        assert!(!compiled.validate(bad.root()).is_empty(), "{schema}");
    }
}

#[test]
fn dynamic_references_require_a_dynamic_static_target_before_scope_override() {
    // A normal anchor and a pointer target both retain static resolution,
    // even when the outer resource declares an anchor with the same name.
    for (anchor, reference) in [("$anchor", "#value"), ("$dynamicAnchor", "#/$defs/value")] {
        let text = format!(
            r##"{{"$defs":{{"outer":{{"$dynamicAnchor":"value","type":"integer"}},"base":{{"$id":"base","$defs":{{"value":{{"{anchor}":"value","type":"string"}}}},"$dynamicRef":"{reference}"}}}},"$ref":"base"}}"##
        );
        let doc = json(&text);
        let compiled = Compiler::new(Config::default())
            .compile(doc.root())
            .unwrap();
        let good = json(r#""hello""#);
        let bad = json("42");
        assert!(compiled.validate(good.root()).is_empty(), "{text}");
        assert!(!compiled.validate(bad.root()).is_empty(), "{text}");
    }
}

#[test]
fn resource_uris_preserve_query_and_resolve_relative_ids_once() {
    let doc = json(
        r##"{"$id":"schemas/root.json","$defs":{"a":{"$id":"?version=one","type":"integer"},"b":{"$id":"?version=two","type":"string"}},"$ref":"?version=two"}"##,
    );
    let compiled = Compiler::new(Config::default())
        .compile(doc.root())
        .unwrap();
    let good = json(r#""hello""#);
    let bad = json("42");
    assert!(compiled.validate(good.root()).is_empty());
    assert!(!compiled.validate(bad.root()).is_empty());
}

#[test]
fn content_schema_resources_are_indexed_without_asserting_decoded_content() {
    let doc = json(
        r##"{"contentMediaType":"application/json","contentSchema":{"$id":"nested","$anchor":"value","type":"integer"},"$ref":"nested#value"}"##,
    );
    let compiled = Compiler::new(Config::default())
        .compile(doc.root())
        .unwrap();
    let good = json("42");
    let bad = json(r#""42""#);
    assert!(compiled.validate(good.root()).is_empty());
    assert!(!compiled.validate(bad.root()).is_empty());
}

#[test]
fn anchor_declarations_enforce_the_plain_name_syntax() {
    for keyword in ["$anchor", "$dynamicAnchor"] {
        for name in ["1bad", "has space", "x:y", "", "café"] {
            let doc = json(&format!(r#"{{"{keyword}":"{name}"}}"#));
            assert!(
                Compiler::new(Config::default())
                    .compile(doc.root())
                    .is_err(),
                "{keyword}: {name}"
            );
        }
        for name in ["_", "value", "V.a-l_u3"] {
            let doc = json(&format!(r#"{{"{keyword}":"{name}"}}"#));
            assert!(
                Compiler::new(Config::default()).compile(doc.root()).is_ok(),
                "{keyword}: {name}"
            );
        }
    }
}
