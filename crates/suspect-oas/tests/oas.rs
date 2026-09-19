use std::sync::Arc;

use suspect_low::ValueKind;
use suspect_oas::{ModelError, OasVersion, Session};
use suspect_ref::WorkspaceBuilder;

fn write(dir: &std::path::Path, name: &str, content: &str) -> suspect_source::Uri {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    suspect_source::Uri::from_path(&path).unwrap()
}

fn workspace_with(dir: &std::path::Path, entry: &str) -> Arc<suspect_ref::Workspace> {
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    ws.load_all(entry).unwrap();
    Arc::new(ws)
}

#[test]
fn primitive_type_views_preserve_explicit_declarations_and_versioned_nullability() {
    let dir =
        std::env::temp_dir().join(format!("suspect-oas-type-semantics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "main.json",
        r#"{
      "openapi":"3.1.0","info":{"title":"Types","version":"1"},"paths":{},
      "components":{"schemas":{
        "UntypedObject":{"properties":{"id":{"type":"string"}}},
        "UntypedArray":{"items":{"type":"integer"}},
        "AnnotationOnly":{"nullable":true},
        "LegacyNullable":{"type":"string","nullable":true},
        "ActualNullable":{"type":["string","null"]},
        "EscapedType":{"type":"\u0073tring"}
      }}
    }"#,
    );
    let session = Session::new(workspace_with(&dir, "main.json"));
    let api = session.open("main.json").unwrap();
    let components = api.components().unwrap();
    for name in ["UntypedObject", "UntypedArray", "AnnotationOnly"] {
        assert_eq!(components.schema(name).unwrap().type_(), None, "{name}");
    }
    let legacy = components.schema("LegacyNullable").unwrap();
    assert_eq!(legacy.type_().unwrap().bits(), suspect_oas::TypeSet::STRING);
    assert_eq!(
        legacy.type_set_for(OasVersion::V30).unwrap().bits(),
        suspect_oas::TypeSet::STRING | suspect_oas::TypeSet::NULL
    );
    assert_eq!(
        legacy.type_set_for(OasVersion::V31).unwrap().bits(),
        suspect_oas::TypeSet::STRING
    );
    assert_eq!(
        components
            .schema("AnnotationOnly")
            .unwrap()
            .type_set_for(OasVersion::V30),
        None
    );
    let nullable = components.schema("ActualNullable").unwrap();
    assert_eq!(
        nullable.type_().unwrap().bits(),
        suspect_oas::TypeSet::STRING | suspect_oas::TypeSet::NULL
    );
    assert_eq!(
        components
            .schema("EscapedType")
            .unwrap()
            .type_()
            .unwrap()
            .bits(),
        suspect_oas::TypeSet::STRING
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn typed_schema_resolution_uses_decoded_reference_fragments() {
    let dir = std::env::temp_dir().join(format!("suspect-oas-decoded-ref-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "main.json",
        r##"{
      "openapi":"3.1.0","info":{"title":"Refs","version":"1"},"paths":{},
      "components":{"schemas":{
        "caf\u00e9":{"type":"string"},
        "Use":{"$ref":"#/components/schemas/caf%C3%A9"},
        "EncodedRoot":{"$ref":"#%2Fcomponents%2Fschemas%2Fcaf%C3%A9"}
      }}
    }"##,
    );
    let session = Session::new(workspace_with(&dir, "main.json"));
    let api = session.load("main.json").unwrap();
    let schemas = api.components().unwrap().schemas();
    let (_, schema) = schemas.iter().find(|(name, _)| *name == "Use").unwrap();
    assert!(
        schema
            .type_()
            .is_some_and(|t| t.contains(suspect_oas::TypeSet::STRING))
    );
    assert!(!schema.resolved().is_cyclic());
    let (_, encoded_root) = schemas
        .iter()
        .find(|(name, _)| *name == "EncodedRoot")
        .unwrap();
    assert!(
        encoded_root
            .type_()
            .is_some_and(|t| t.contains(suspect_oas::TypeSet::STRING))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn typed_schemas_guard_cross_file_root_cycles_and_keep_legal_recursion() {
    let dir = std::env::temp_dir().join(format!("suspect-oas-cross-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "main.yaml",
        "openapi: 3.1.0\ninfo: {title: Refs, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Cycle: {$ref: 'a.yaml'}\n    Node: {$ref: 'node.yaml'}\n",
    );
    write(&dir, "a.yaml", "$ref: 'b.yaml'\n");
    write(&dir, "b.yaml", "$ref: 'a.yaml'\n");
    write(
        &dir,
        "node.yaml",
        "type: object\nproperties:\n  next: {$ref: 'node.yaml'}\n",
    );
    let session = Session::new(workspace_with(&dir, "main.yaml"));
    let api = session.load("main.yaml").unwrap();
    let schemas = api.components().unwrap().schemas();
    let (_, cycle) = schemas.iter().find(|(name, _)| *name == "Cycle").unwrap();
    assert!(cycle.resolved().is_cyclic());
    let (_, node) = schemas.iter().find(|(name, _)| *name == "Node").unwrap();
    assert!(!node.resolved().is_cyclic());
    let properties = node.properties();
    let (_, next) = properties.iter().find(|(name, _)| *name == "next").unwrap();
    assert!(!next.resolved().is_cyclic());
    assert!(
        next.type_()
            .is_some_and(|t| t.contains(suspect_oas::TypeSet::OBJECT))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn openapi_traversal_and_refs() -> Result<(), ModelError> {
    let dir = std::env::temp_dir().join("suspect-oas-test1");
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "main.yaml",
        r#"
openapi: 3.1.0
info:
  title: Pets API
  version: "2.0"
servers:
  - url: https://api.example.com
    description: prod
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets]
      parameters:
        - name: limit
          in: query
          schema:
            type: integer
      responses:
        '200':
          description: A list of pets
          content:
            application/json:
              schema:
                $ref: 'schemas.yaml#/components/schemas/PetList'
        default:
          $ref: '#/components/responses/Err'
components:
  responses:
    Err:
      description: error
"#,
    );
    write(
        &dir,
        "schemas.yaml",
        r#"
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id:
          type: integer
        name:
          type: string
        tag:
          type: string
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
"#,
    );

    let ws = workspace_with(&dir, "main.yaml");
    let session = Session::new(ws);
    let api = session.load("main.yaml")?;

    assert_eq!(api.version(), OasVersion::V31);
    let info = api.info().unwrap();
    assert_eq!(info.title(), Some("Pets API"));
    assert_eq!(info.version(), Some("2.0"));
    assert_eq!(api.servers()[0].url(), Some("https://api.example.com"));

    // paths → operation → parameter typing
    let paths = api.paths().unwrap();
    let pets = paths.get("/pets").unwrap();
    let get = pets.operation("get").unwrap();
    assert_eq!(get.operation_id(), Some("listPets"));
    assert_eq!(get.tags(), vec!["pets"]);
    let limit = &get.parameters()[0];
    assert_eq!(limit.name(), Some("limit"));
    assert_eq!(limit.location(), Some(suspect_oas::ParameterIn::Query));
    let param_schema = limit.schema().unwrap();
    let ts = param_schema.type_().unwrap();
    assert!(
        ts.contains(suspect_oas::TypeSet::INTEGER),
        "limit param must be integer-typed"
    );

    // cross-file $ref through responses → PetList → items → Pet
    let responses = get.responses().unwrap();
    let ok = responses.get("200").unwrap();
    let content = ok.content();
    let (_, media) = &content[0];
    let list_schema = media.schema().unwrap();
    let resolved_list = list_schema.resolved();
    assert_eq!(
        resolved_list
            .type_()
            .map(|t| t.contains(suspect_oas::TypeSet::ARRAY)),
        Some(true)
    );
    let item = resolved_list.items().unwrap().resolved();
    assert_eq!(
        item.property("name")
            .unwrap()
            .type_()
            .map(|t| t.contains(suspect_oas::TypeSet::STRING)),
        Some(true)
    );
    assert_eq!(item.required(), vec!["id", "name"]);

    // local ref with sibling-only object (default response)
    let def = responses.default().unwrap();
    assert!(def.is_ref());
    let err = def.resolved();
    assert_eq!(err.description(), Some("error"));

    Ok(())
}

#[test]
fn cycles_do_not_hang_models() -> Result<(), ModelError> {
    let dir = std::env::temp_dir().join("suspect-oas-test2");
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "cycle.yaml",
        r#"
openapi: 3.0.3
info: {title: cyc, version: "1"}
paths:
  /a:
    get:
      responses:
        '200': {description: ok}
components:
  schemas:
    Node:
      type: object
      properties:
        next:
          $ref: '#/components/schemas/Node'
    Loop:
      $ref: '#/components/schemas/Loop'
"#,
    );
    let ws = workspace_with(&dir, "cycle.yaml");
    let session = Session::new(ws);
    let api = session.load("cycle.yaml")?;
    let schemas = api.components().unwrap();
    let node = schemas.schema("Node").unwrap();
    let next = node.resolved().property("next").unwrap();
    let _again = next.resolved(); // legal recursion: must terminate
    let loop_schema = schemas.schema("Loop").unwrap();
    let l = loop_schema.resolved(); // direct self-cycle: degrades, no hang
    assert_eq!(l.node().kind(), ValueKind::Object);
    Ok(())
}

#[test]
fn non_openapi_entry_rejected() {
    let dir = std::env::temp_dir().join("suspect-oas-test3");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "notoas.yaml", "random: document\n");
    let ws = workspace_with(&dir, "notoas.yaml");
    let session = Session::new(ws);
    match session.load("notoas.yaml") {
        Err(ModelError::NotOpenApi { .. }) => {}
        other => panic!("expected NotOpenApi, got {other:?}"),
    }
}
