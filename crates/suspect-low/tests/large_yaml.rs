//! Guards the vendored-scanner patch that lifted the ~32k-line YAML ceiling.
//!
//! Regression: upstream tree-sitter-yaml v0.7.2 tracks rows in int16_t
//! (scanner state + locals), corrupting parses of documents beyond 32,767
//! lines. Our vendored copy widens those to int32_t; this test fails if that
//! patch is ever lost.

use suspect_low::{LowDoc, SpecFamily};
use suspect_source::{Source, Uri};

fn big_yaml() -> Vec<u8> {
    let mut out = String::from(
        "openapi: 3.1.0\ninfo:\n  title: big\n  version: \"1\"\npaths:\n  /x:\n    get:\n      responses:\n        \"200\":\n          description: ok\ncomponents:\n  schemas:\n",
    );
    for i in 0..20_000 {
        out.push_str(&format!(
            "    Schema{i}:\n      type: object\n      properties:\n        name:\n          type: string\n"
        ));
    }
    out.into_bytes()
}

#[test]
fn large_yaml_parses() {
    let doc = LowDoc::parse(Uri::from("mem://big.yaml"), Source::from_vec(big_yaml()));
    assert!(doc.syntax_errors().is_empty(), "large YAML must parse without syntax errors");
    assert_eq!(doc.sniff_family(), SpecFamily::Oas31);
    let schemas = doc.root().get("components").and_then(|c| c.get("schemas"));
    assert_eq!(schemas.map(|s| s.entries().len()), Some(20_000));
    let deep = doc
        .root()
        .pointer(&suspect_low::Pointer::parse("#/components/schemas/Schema12345").unwrap());
    assert_eq!(deep.and_then(|s| s.get("type")).and_then(|t| t.as_str()), Some("object"));
}
