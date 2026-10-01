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

/// The reference implementation of the index walk: every `$ref` asks its
/// own node where it is. It is slow, and it is here because it is
/// obviously correct — no carried state to get wrong.
fn index_by_climbing(low: &LowDoc) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut pending = vec![low.root()];
    while let Some(node) = pending.pop() {
        if node.kind() == suspect_low::ValueKind::Object {
            for entry in node.entries() {
                if entry.key == "$ref"
                    && let Some(value) = entry.value
                {
                    out.push((node.path_from_root().to_path(), value.byte_range().start));
                }
                if let Some(child) = entry.value {
                    pending.push(child);
                }
            }
        } else if node.kind() == suspect_low::ValueKind::Array {
            pending.extend(node.items());
        }
    }
    out.sort();
    out
}

/// The same walk as `Index::index_document`, reduced to the pairs that
/// identify a reference.
fn index_by_carried_path(low: &LowDoc) -> Vec<(String, usize)> {
    let mut index = Index::default();
    index.index_document("mem://model.yaml", low);
    let mut out: Vec<(String, usize)> = index
        .references()
        .iter()
        .map(|reference| (reference.source.to_path(), reference.range.start))
        .collect();
    out.sort();
    out
}

/// The carried path must name exactly what climbing does.
///
/// `index_document` carries each node's pointer down the walk instead of
/// climbing to the root per reference, because climbing cost twelve times
/// the walk itself on the 63k-line Plex specification. Carrying a path is
/// exactly the kind of optimization that is right everywhere except the
/// one case it does not model: a YAML merge key, whose bytes live in
/// another mapping entirely. So the fixtures below are the cases that
/// could break it, and this is the gate that says they did not.
#[test]
fn the_carried_path_agrees_with_climbing_to_the_root() {
    let fixtures: Vec<(&str, &str)> = vec![
        (
            "merge keys",
            "base: &b\n  $ref: '#/components/schemas/One'\n  name: merged\nchild:\n  <<: *b\n  extra:\n    $ref: '#/components/schemas/Two'\n",
        ),
        (
            "anchors and aliases",
            "a: &x\n  p:\n    $ref: '#/c/d'\nb: *x\nlist:\n  - $ref: '#/one'\n  - k:\n      - $ref: '#/two'\n",
        ),
        (
            "duplicate keys and empty values",
            "k: 1\nk:\n  $ref: '#/last'\nempty:\nafter:\n  $ref: '#/after'\n",
        ),
        (
            "a realistic operation",
            "paths:\n  /a/{id}:\n    get:\n      parameters:\n        - $ref: '#/components/parameters/Id'\n        - in: query\n          $ref: '#/components/parameters/Q'\n      responses:\n        '200':\n          $ref: '#/components/responses/Ok'\n",
        ),
    ];
    for (name, text) in fixtures {
        let low = doc(text);
        assert_eq!(
            index_by_carried_path(&low),
            index_by_climbing(&low),
            "the carried path disagrees on {name}"
        );
    }

    // JSON resolves to pointers by index, and escaping is its own rule
    // (`~1` for a slash inside a key).
    let json = LowDoc::parse(
        "mem://model.json".into(),
        Source::from_vec(br##"{"a/b":{"~key":[{"$ref":"#/components/responses/Ok"}]}}"##.to_vec()),
    );
    assert_eq!(
        index_by_carried_path(&json),
        index_by_climbing(&json),
        "the carried path disagrees on JSON"
    );
    assert!(
        index_by_carried_path(&json)
            .iter()
            .any(|(path, _)| path == "/a~1b/~0key/0"),
        "escaped keys must survive the walk"
    );
}

/// A deep sequence must not recurse: the walk is iterative on purpose,
/// because a hostile document can nest far deeper than the stack allows.
#[test]
fn the_index_walk_survives_a_document_nested_past_the_stack() {
    let depth = 50_000;
    let mut text = String::new();
    for _ in 0..depth {
        text.push_str("  - ");
    }
    text.push_str("$ref: '#/deep'\n");
    let low = doc(&text);
    assert_eq!(index_by_carried_path(&low), index_by_climbing(&low));
}

/// Hover inside a schema says how you get there from the schema.
///
/// The full pointer of a property buried in an operation is mostly path
/// noise — `/paths/~1activities/get/responses/200/content/
/// application~1json/schema/properties/MediaContainer` — and the part an
/// author is actually asking about is the tail of it.
#[test]
fn the_schema_position_says_how_you_get_there_from_the_schema() {
    let text = "paths:\n  /activities:\n    get:\n      responses:\n        '200':\n          content:\n            application/json:\n              schema:\n                properties:\n                  MediaContainer:\n                    type: object\n";
    let low = doc(text);
    let offset = text.find("type: object").expect("present");
    let index = Index::default();
    let rendered = crate::hover_meaning(&low, offset, &index).expect("meaning half");
    assert!(
        rendered.contains("inside `properties \u{203a} MediaContainer \u{203a} type`"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("_schema position_"),
        "the placeholder must not reach a user: {rendered}"
    );
}
