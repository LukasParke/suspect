//! Tree-shape probes: verify grammar assumptions against real parses.
//! These double as regression tests for the kind mapping.

use crate::{Format, ScalarStyle, SourceDoc, SyntaxKind};
use suspect_source::Source;

fn parse_yaml(src: &str) -> SourceDoc {
    SourceDoc::with_format("mem://probe.yaml".into(), Source::from_vec(src.as_bytes().to_vec()), Format::Yaml)
}

fn parse_json(src: &str) -> SourceDoc {
    SourceDoc::with_format("mem://probe.json".into(), Source::from_vec(src.as_bytes().to_vec()), Format::Json)
}

#[test]
fn probe_yaml_mapping_shape() {
    let doc = parse_yaml("openapi: 3.1.0\ninfo:\n  title: t\n");
    let root = doc.root();
    println!("YAML sexp: {}", root.to_sexp());
    let content = root.content();
    assert_eq!(content.kind(), SyntaxKind::Mapping, "root content should be a mapping");
    let entries = content.mapping_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0.scalar_bytes(), b"openapi");
    assert_eq!(entries[0].1.unwrap().scalar_bytes(), b"3.1.0");
    let info = entries[1].1.unwrap();
    assert_eq!(info.kind(), SyntaxKind::Mapping);
}

#[test]
fn probe_yaml_anchors_aliases() {
    let src = "base: &b\n  x: 1\nuse: *b\n";
    let doc = parse_yaml(src);
    println!("ANCHORS sexp: {}", doc.root().to_sexp());
    println!("anchors map: {:?}", doc.anchors());
    let use_val = doc.root().content().get(b"use").unwrap();
    println!("use node kind={:?} raw={}", use_val.kind(), use_val.raw_kind());
    if use_val.kind() == SyntaxKind::Alias {
        let target = doc.anchor_target(use_val.alias_name().unwrap()).unwrap();
        assert_eq!(target.content().kind(), SyntaxKind::Mapping);
    }
}

#[test]
fn probe_yaml_merge_key() {
    let src = "a: &a\n  x: 1\nb:\n  <<: *a\n  y: 2\n";
    let doc = parse_yaml(src);
    println!("MERGE sexp: {}", doc.root().to_sexp());
    let b = doc.root().content().get(b"b").unwrap();
    let keys: Vec<_> = b.mapping_entries().iter().map(|(k, _)| k.scalar_bytes().to_vec()).collect();
    println!("b keys: {keys:?}");
}

#[test]
fn probe_yaml_sequences_and_flow() {
    let src = "arr:\n  - 1\n  - two\nflow: [1, 2]\nmap: {k: v}\n";
    let doc = parse_yaml(src);
    println!("SEQ sexp: {}", doc.root().to_sexp());
    let arr = doc.root().content().get(b"arr").unwrap();
    assert_eq!(arr.kind(), SyntaxKind::Sequence);
    let items = arr.sequence_items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].scalar_bytes(), b"1");
    assert_eq!(items[1].scalar_bytes(), b"two");
    let flow = doc.root().content().get(b"flow").unwrap();
    assert_eq!(flow.kind(), SyntaxKind::Sequence);
    assert_eq!(flow.sequence_items().len(), 2);
    let map = doc.root().content().get(b"map").unwrap();
    assert_eq!(map.kind(), SyntaxKind::Mapping);
}

#[test]
fn probe_yaml_quoted_and_block_scalars() {
    let src = "s: 'single'\nd: \"double \\n esc\"\nb: |\n  line1\n  line2\np: plain text\n";
    let doc = parse_yaml(src);
    println!("SCALARS sexp: {}", doc.root().to_sexp());
    let root = doc.root().content();
    assert_eq!(root.get(b"s").unwrap().scalar_style(), crate::ScalarStyle::SingleQuoted);
    assert_eq!(root.get(b"d").unwrap().scalar_style(), ScalarStyle::DoubleQuoted);
    assert_eq!(root.get(b"b").unwrap().scalar_style(), ScalarStyle::Block);
    assert_eq!(root.get(b"p").unwrap().scalar_style(), ScalarStyle::Plain);
}

#[test]
fn probe_json_shape() {
    let doc = parse_json(r#"{"openapi": "3.1.0", "n": 1, "t": true, "x": null, "a": [1, {"k": "v"}]}"#);
    let root = doc.root();
    println!("JSON sexp: {}", root.to_sexp());
    let content = root.content();
    assert_eq!(content.kind(), SyntaxKind::Mapping);
    let entries = content.mapping_entries();
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0].0.scalar_bytes(), b"openapi");
    let arr = entries[4].1.unwrap();
    assert_eq!(arr.kind(), SyntaxKind::Sequence);
    let items = arr.sequence_items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].kind(), SyntaxKind::Mapping);
}

#[test]
fn probe_error_recovery() {
    let doc = parse_json(r#"{"a": 1, "b": }"#);
    println!("ERR sexp: {}", doc.root().to_sexp());
    println!("errors: {:?}", doc.errors());
    assert!(!doc.errors().is_empty());
    // must still expose the good part
    let entries = doc.root().content().mapping_entries();
    assert!(!entries.is_empty());
}
