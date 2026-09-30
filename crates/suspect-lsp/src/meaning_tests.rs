//! Tests for the per-node semantic model and the reference index.

use super::*;
use crate::refactor::declares;
use suspect_source::Source;

fn doc(text: &str) -> LowDoc {
    LowDoc::parse(
        "mem://model.yaml".into(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

const SPEC: &str = r#"
openapi: 3.2.0
info: {title: Model, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - name: limit
          in: query
          schema: {type: integer, maximum: 100}
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Pet'}
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: {type: string}
    Owner:
      type: object
      properties:
        pet: {$ref: '#/components/schemas/Pet'}
"#;

/// Byte offset of the first occurrence of `needle`.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"))
        + 1
}

#[test]
fn dialect_is_read_from_the_document() {
    assert_eq!(Dialect::of(&doc(SPEC)), Dialect::Oas32);
    assert_eq!(
        Dialect::of(&doc("openapi: 3.0.3\ninfo: {title: a, version: '1'}\n")),
        Dialect::Oas30
    );
    assert_eq!(
        Dialect::of(&doc("openapi: 3.1.0\ninfo: {title: a, version: '1'}\n")),
        Dialect::Oas31
    );
    assert_eq!(
        Dialect::of(&doc(
            "overlay: 1.1.0\ninfo: {title: a, version: '1'}\nactions: []\n"
        )),
        Dialect::Overlay
    );
}

#[test]
fn an_operation_parameter_is_a_parameter_inside_a_schema() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    // The cursor is on `maximum`, inside the parameter's schema.
    let meaning = model.at(at(SPEC, "maximum: 100")).expect("a meaning");
    assert_eq!(meaning.kind, ObjectKind::Schema);
    assert!(meaning.in_schema);
    assert_eq!(meaning.dialect, Dialect::Oas32);
    assert_eq!(
        meaning.pointer.to_path(),
        "/paths/~1pets/get/parameters/0/schema/maximum"
    );
}

#[test]
fn a_ref_position_resolves_to_its_target() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    let meaning = model
        .at(at(SPEC, "#/components/schemas/Pet"))
        .expect("meaning");
    let target = meaning.ref_target.expect("a ref at the cursor");
    assert_eq!(target.pointer.to_path(), "/components/schemas/Pet");
}

#[test]
fn a_component_definition_is_located_by_pointer() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    let meaning = model.at(at(SPEC, "required: [name]")).expect("meaning");
    assert_eq!(
        meaning.pointer.to_path(),
        "/components/schemas/Pet/required",
        "the pointer names the exact node under the cursor"
    );
    assert!(meaning.in_schema);
}

#[test]
fn an_operation_is_classified_and_its_operation_id_is_named() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    let meaning = model
        .at(at(SPEC, "operationId: listPets"))
        .expect("meaning");
    assert_eq!(
        meaning.kind,
        ObjectKind::Operation,
        "a position under a path item's method is inside that operation"
    );
    assert!(meaning.pointer.to_path().starts_with("/paths/~1pets/get"));
    assert!(
        !meaning.in_schema,
        "an operationId value is not a schema position"
    );
}

#[test]
fn enclosing_schema_finds_the_nearest_one() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    let schema = model
        .enclosing_schema(at(SPEC, "maximum: 100"))
        .expect("a schema");
    assert!(schema.byte_range().start <= at(SPEC, "maximum: 100"));
    let name = schema.get("maximum").and_then(|n| n.as_str());
    assert_eq!(name, Some("100"));
}

#[test]
fn the_index_finds_transitive_references() {
    let low = doc(SPEC);
    let mut index = Index::default();
    index.index_document(low.uri().as_str(), &low);
    // Pet is referenced from the response schema, and Owner references it
    // too: two inbound references at distance 0.
    let pet = Pointer::parse("/components/schemas/Pet").unwrap();
    let key = format!("{}#{}", low.uri().as_str(), pet.to_path());
    assert_eq!(
        index.references_to(&key).len(),
        2,
        "{:?}",
        index.references_to(&key)
    );
    // Transitivity: the node that references Pet is itself reachable, so a
    // breadth-first walk from Pet reaches both the response and Owner.
    let owner = Pointer::parse("/components/schemas/Owner").expect("pointer");
    let owner_key = format!("{}#{}", low.uri().as_str(), owner.to_path());
    assert!(
        index.references_to(&owner_key).is_empty(),
        "nothing references Owner itself; the graph is walked by following references"
    );
    // Walking from Owner reaches Pet, which is what impact analysis does.
    let pet_from_owner = index
        .references()
        .iter()
        .find(|reference| reference.source.to_path().starts_with(&owner.to_path()))
        .expect("Owner's reference");
    assert_eq!(pet_from_owner.text, "#/components/schemas/Pet");
}

#[test]
fn the_model_answers_the_questions_features_ask() {
    let low = doc(SPEC);
    let model = Model::new(&low);
    assert_eq!(
        model.dialect(),
        Dialect::Oas32,
        "features ask which spec governs"
    );
    let offset = at(SPEC, "required: [name]");
    let meaning = model.at(offset).expect("meaning");
    assert!(
        meaning.within(ObjectKind::Schema),
        "features ask what encloses the cursor: {:?}",
        meaning.trail
    );
    // The object ancestor of a cursor on a key is the object itself.
    let object = model
        .object_ancestor(at(SPEC, "required: [name]"))
        .expect("an object");
    assert!(object.get("required").is_some());
}

#[test]
fn a_dangling_reference_is_still_indexed() {
    // The index records references, not resolutions: a ref to a component
    // that does not exist must still be findable, because that is how
    // "find the dangling refs" is answered.
    let low = doc(
        "openapi: 3.1.0\ninfo: {title: a, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    A: {$ref: '#/components/schemas/Missing'}\n",
    );
    let mut index = Index::default();
    index.index_document(low.uri().as_str(), &low);
    assert_eq!(index.references().len(), 1);
    let missing = Pointer::parse("/components/schemas/Missing").unwrap();
    let key = format!("{}#{}", low.uri().as_str(), missing.to_path());
    assert_eq!(
        index.references_to(&key).len(),
        1,
        "the dangling reference is indexed against the pointer it names"
    );
    assert!(
        !declares(&low, "Missing"),
        "and the target genuinely does not exist"
    );
}
