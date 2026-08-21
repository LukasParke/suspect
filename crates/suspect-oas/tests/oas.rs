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
    assert!(ts.contains(suspect_oas::TypeSet::INTEGER), "limit param must be integer-typed");

    // cross-file $ref through responses → PetList → items → Pet
    let responses = get.responses().unwrap();
    let ok = responses.get("200").unwrap();
    let content = ok.content();
    let (_, media) = &content[0];
    let list_schema = media.schema().unwrap();
    let resolved_list = list_schema.resolved();
    assert_eq!(resolved_list.type_().map(|t| t.contains(suspect_oas::TypeSet::ARRAY)), Some(true));
    let item = resolved_list.items().unwrap().resolved();
    assert_eq!(item.property("name").unwrap().type_().map(|t| t.contains(suspect_oas::TypeSet::STRING)), Some(true));
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
