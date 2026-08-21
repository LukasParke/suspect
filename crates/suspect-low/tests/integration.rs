use suspect_low::{LowDoc, SpecFamily, ValueKind};
use suspect_source::Source;

fn parse(src: &str) -> LowDoc {
    LowDoc::parse("mem://test.yaml".into(), Source::from_vec(src.as_bytes().to_vec()))
}

#[test]
fn navigation_and_typing() {
    let doc = parse(
        r#"
openapi: 3.1.0
info:
  title: Test
  version: "1.0"
count: 42
ratio: 1.5
enabled: true
nothing: null
tags:
  - alpha
  - beta
"#,
    );
    let root = doc.root();
    assert_eq!(doc.sniff_family(), SpecFamily::Oas31);
    assert_eq!(root.kind(), ValueKind::Object);

    let info = root.get("info").unwrap();
    assert_eq!(info.kind(), ValueKind::Object);
    assert_eq!(info.get("title").unwrap().as_str(), Some("Test"));
    // quoted version stays a string
    assert_eq!(info.get("version").unwrap().kind(), ValueKind::Str);

    assert_eq!(root.get("count").unwrap().as_i64(), Some(42));
    assert_eq!(root.get("ratio").unwrap().as_f64(), Some(1.5));
    assert_eq!(root.get("enabled").unwrap().as_bool(), Some(true));
    assert_eq!(root.get("nothing").unwrap().kind(), ValueKind::Null);

    let tags = root.get("tags").unwrap();
    assert_eq!(tags.kind(), ValueKind::Array);
    assert_eq!(tags.at(0).unwrap().as_str(), Some("alpha"));
    assert_eq!(tags.at(1).unwrap().as_str(), Some("beta"));
    assert!(tags.at(2).is_none());
}

#[test]
fn version_string_is_not_a_number() {
    // the classic OAS footgun: unquoted 3.1 would be a float
    let doc = parse("openapi: 3.0.0\n");
    assert_eq!(doc.root().get("openapi").unwrap().kind(), ValueKind::Str);
}

#[test]
fn pointer_navigation() {
    let doc = parse(
        "components:\n  schemas:\n    Pet:\n      type: object\n    /weird/key:\n      x: 1\n",
    );
    let root = doc.root();
    let p = suspect_low::Pointer::parse("#/components/schemas/Pet/type").unwrap();
    assert_eq!(root.pointer(&p).unwrap().as_str(), Some("object"));
    let escaped = suspect_low::Pointer::parse("#/components/schemas/~1weird~1key/x").unwrap();
    assert_eq!(root.pointer(&escaped).unwrap().as_i64(), Some(1));
}

#[test]
fn aliases_resolve_transparently() {
    let doc = parse(
        "base: &base\n  x: 1\n  y: 2\nuser: *base\n",
    );
    let user = doc.root().get("user").unwrap();
    assert!(user.is_alias());
    assert_eq!(user.kind(), ValueKind::Object);
    assert_eq!(user.get("x").unwrap().as_i64(), Some(1));
    assert_eq!(user.get("y").unwrap().as_i64(), Some(2));
}

#[test]
fn merge_keys_expand() {
    let doc = parse(
        "defaults: &d\n  a: 1\n  b: 2\nmerged:\n  <<: *d\n  b: 3\n  c: 4\n",
    );
    let merged = doc.root().get("merged").unwrap();
    let keys: Vec<_> = merged.entries().iter().map(|e| e.key).collect();
    assert_eq!(keys, ["b", "c", "a"], "explicit keys first, merged fill-ins after");
    // explicit wins over merged
    assert_eq!(merged.get("b").unwrap().as_i64(), Some(3));
    assert_eq!(merged.get("a").unwrap().as_i64(), Some(1));
}

#[test]
fn duplicate_keys_detected() {
    let doc = parse("a: 1\na: 2\nb: 3\n");
    let dups = doc.root().duplicate_keys();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].key, "a");
    assert_eq!(dups[0].occurrences.len(), 2);
}

#[test]
fn source_maps_point_at_real_bytes() {
    let src = "info:\n  title: Hi\n";
    let doc = parse(src);
    let title = doc.root().get("info").unwrap().get("title").unwrap();
    let (line, col) = title.line_col();
    assert_eq!((line, col), (1, 9));
    let range = title.byte_range();
    assert_eq!(&src[range], "Hi");
}

#[test]
fn path_from_root_round_trips() {
    let doc = parse("a:\n  b:\n    - x\n    - y: 7\n");
    let target = doc
        .root()
        .get("a")
        .unwrap()
        .get("b")
        .unwrap()
        .at(1)
        .unwrap()
        .get("y")
        .unwrap();
    let p = target.path_from_root();
    assert_eq!(p.to_path(), "/a/b/1/y");
    assert_eq!(doc.root().pointer(&p).unwrap().as_i64(), Some(7));
}

#[test]
fn json_documents_work() {
    let doc = LowDoc::parse(
        "mem://t.json".into(),
        Source::from_vec(br#"{"openapi": "3.1.0", "n": 5}"#.to_vec()),
    );
    assert_eq!(doc.format(), suspect_syntax::Format::Json);
    assert_eq!(doc.sniff_family(), SpecFamily::Oas31);
    assert_eq!(doc.root().get("n").unwrap().as_i64(), Some(5));
}

#[test]
fn family_sniffing_all_kinds() {
    assert_eq!(parse("swagger: '2.0'\n").sniff_family(), SpecFamily::Oas2);
    assert_eq!(parse("openapi: 3.0.3\n").sniff_family(), SpecFamily::Oas30);
    assert_eq!(parse("openapi: 3.2.0\n").sniff_family(), SpecFamily::Oas32);
    assert_eq!(parse("arazzo: '1.0.0'\n").sniff_family(), SpecFamily::Arazzo10);
    assert_eq!(parse("overlay: '1.0.0'\n").sniff_family(), SpecFamily::Overlay10);
    assert_eq!(parse("random: doc\n").sniff_family(), SpecFamily::Unknown);
}

#[test]
fn decoded_scalars_all_styles() {
    let doc = parse(
        "a: plain\n\
         s: 'it''s'\n\
         d: \"line\\nnext\\u00e9\"\n\
         lit: |\n  one\n  two\n\
         fold: >-\n  hello\n  world\n\
         keep: |+\n  x\n\n",
    );
    let root = doc.root();
    assert_eq!(root.get("a").unwrap().decoded_scalar().as_ref(), b"plain");
    assert_eq!(root.get("s").unwrap().decoded_scalar().as_ref(), b"it's");
    assert_eq!(
        root.get("d").unwrap().decoded_scalar().as_ref(),
        "line\nnext\u{e9}".as_bytes()
    );
    assert_eq!(root.get("lit").unwrap().decoded_scalar().as_ref(), b"one\ntwo\n");
    // folded + strip chomping: single space join, no trailing newline
    assert_eq!(root.get("fold").unwrap().decoded_scalar().as_ref(), b"hello world");
    // keep chomping preserves interior/trailing structure
    let kept = root.get("keep").unwrap().decoded_scalar();
    assert!(kept.starts_with(b"x"), "got {kept:?}");
}
