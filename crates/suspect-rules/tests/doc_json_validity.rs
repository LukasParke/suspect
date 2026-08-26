//! The string writer must produce JSON identical in structure to the
//! Value-tree path — a malformed document parses to nothing worker-side
//! and every selection silently misses.

use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

#[test]
fn doc_to_json_string_is_valid_and_faithful() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests-fixtures/rules-ts/openapi-basic.yaml");
    let bytes = std::fs::read(&path).expect("fixture");
    let doc = LowDoc::parse(Uri::from("mem://t.yaml"), Source::from_vec(bytes));

    let as_string = suspect_rules::node_json::doc_to_json_string(&doc.root());
    let from_string: serde_json::Value =
        serde_json::from_str(&as_string).expect("string writer emits valid JSON");
    let from_tree = suspect_rules::node_json::node_to_json(&doc.root());

    assert_eq!(
        from_string, from_tree,
        "string-writer and Value-tree paths disagree"
    );
    assert!(
        from_string.get("paths").is_some(),
        "paths present in emitted JSON"
    );
}
