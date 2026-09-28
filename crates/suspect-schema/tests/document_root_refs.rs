//! Document-root `$ref` fallback: schemas compiled from inside a larger
//! document resolve sibling and recursive references without pre-inlining.

use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

const DOC: &str = r#"
components:
  schemas:
    Pet:
      type: object
      properties:
        name: {type: string}
        bestFriend: {$ref: '#/components/schemas/Pet'}
      required: [name]
"#;

#[test]
fn sibling_and_recursive_refs_resolve_through_the_document_root() {
    let uri = Uri::parse("memory://doc.yaml").unwrap();
    let low = LowDoc::parse(uri, Source::from_vec(DOC.as_bytes().to_vec()));
    let root = low.root();

    // Compile ONLY the component schema; historically
    // `#/components/schemas/Pet` was unresolvable from here.
    let pet = root
        .pointer(&suspect_low::Pointer::parse("/components/schemas/Pet").unwrap())
        .unwrap();
    let compiled = suspect_schema::Compiler::new(suspect_schema::Config::default())
        .compile_with_document_root(pet, Some(root))
        .expect("compiles");

    // Direct sibling-style data.
    let instance =
        parse(r#"{"name": "rex", "bestFriend": {"name": "fido", "bestFriend": {"name": "spot"}}}"#);
    let errors = compiled.validate(instance);
    assert!(errors.is_empty(), "deep refs must resolve: {errors:?}");

    // A violation one recursion level deep is located, not swallowed.
    let invalid =
        parse(r#"{"name": "rex", "bestFriend": {"name": "fido", "bestFriend": {"name": 7}}}"#);
    let errors = compiled.validate(invalid);
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("expected `string`")),
        "the nested type violation must surface: {errors:?}"
    );

    // Without the fallback, the same schema stays unresolvable (the
    // historical semantics that made deep validation collapse). The ref
    // is lazy: only an instance that traverses `bestFriend` surfaces it.
    let standalone = suspect_schema::Compiler::new(suspect_schema::Config::default())
        .compile(pet)
        .unwrap();
    let errors = standalone.validate(parse(r#"{"name": "rex", "bestFriend": {"name": "fido"}}"#));
    assert!(
        errors.iter().any(|e| e.message.contains("unresolvable")),
        "subtree-only compilation must not resolve the sibling ref: {errors:?}"
    );
}

fn parse(json: &str) -> suspect_low::NodeRef<'static> {
    let leaked: &'static str = Box::leak(json.to_owned().into_boxed_str());
    let uri = Uri::parse("memory://instance.json").unwrap();
    let doc: &'static LowDoc = Box::leak(Box::new(LowDoc::parse(
        uri,
        Source::from_vec(leaked.as_bytes().to_vec()),
    )));
    doc.root()
}
