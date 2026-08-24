use std::sync::Arc;

use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

use crate::{IrSpec, Method, OpSelector, ParamIn};

const SPEC: &str = r#"
openapi: 3.1.0
info:
  title: Pets
  version: "2.1"
servers:
  - url: https://api.example.com/v1
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets]
      parameters:
        - name: limit
          in: query
          schema: { type: integer }
      responses:
        '200':
          description: page of pets
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PetList'
        default:
          description: error
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Pet'
      responses:
        '201': { description: created }
  /pets/{petId}:
    parameters:
      - name: petId
        in: path
        required: true
        schema: { type: string }
    get:
      operationId: showPetById
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema:
                $ref: 'https://example.com/other.yaml#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
        tag: { type: string }
        friend:
          $ref: '#/components/schemas/Pet'
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
"#;

fn ws_with(dir: &std::path::Path) -> Arc<suspect_ref::Workspace> {
    std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    let _ = ws.load_all("spec.yaml"); // remote-denied external refs are fine
    Arc::new(ws)
}

#[test]
fn indexes_operations_and_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_with(dir.path());
    let uri = Uri::from_path(&dir.path().join("spec.yaml")).unwrap();
    let ir = IrSpec::from_workspace(&ws, &uri).expect("oas spec");

    assert_eq!(ir.title, "Pets");
    assert_eq!(ir.version, "2.1");
    assert_eq!(ir.servers, vec!["https://api.example.com/v1"]);
    assert_eq!(ir.operations.len(), 3);

    // By-id lookups.
    let list = ir.operation(OpSelector::Id("listPets")).unwrap();
    assert_eq!(list.method, Method::Get);
    assert_eq!(list.path, "/pets");
    assert_eq!(list.tags, vec!["pets"]);

    // By method+path lookup.
    let shown = ir
        .operation(OpSelector::MethodPath(Method::Get, "/pets/{petId}"))
        .unwrap();
    assert_eq!(shown.id.as_deref(), Some("showPetById"));
    // Path-item parameter merged in.
    let pet_id = shown
        .parameters
        .iter()
        .find(|p| p.name == "petId")
        .expect("path parameter");
    assert_eq!(pet_id.location, ParamIn::Path);
    assert!(pet_id.required);

    // Request body resolution.
    let create = ir.operation(OpSelector::Id("createPet")).unwrap();
    assert_eq!(create.body_schema.as_deref(), Some("Pet"));

    // Response schema resolution + ordering (200 before default).
    let responses = &list.responses;
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].status, Some(200));
    assert_eq!(responses[0].schema.as_deref(), Some("PetList"));
    assert_eq!(responses[1].status, None);

    // Schemas and dependency edges.
    assert!(ir.schema("Pet").is_some());
    assert_eq!(
        ir.schema_edges["PetList"],
        vec!["Pet".to_owned()],
        "array item refs become edges"
    );
    assert!(
        ir.schema_edges["Pet"].contains(&"Pet".to_owned()),
        "self-referencing edge recorded"
    );
}

#[test]
fn external_refs_stay_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_with(dir.path());
    let uri = Uri::from_path(&dir.path().join("spec.yaml")).unwrap();
    let ir = IrSpec::from_workspace(&ws, &uri).unwrap();
    let shown = ir
        .operation(OpSelector::Id("showPetById"))
        .expect("operation exists");
    assert!(
        shown.responses[0].schema.is_none(),
        "external-file ref must not resolve to a local name"
    );
}

#[test]
fn non_oas_document_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.yaml"), "arazzo: 1.0.0\n").unwrap();
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    ws.load_all("a.yaml").unwrap();
    let uri = Uri::from_path(&dir.path().join("a.yaml")).unwrap();
    let err = IrSpec::from_workspace(&Arc::new(ws), &uri).unwrap_err();
    assert_eq!(err, "not an OpenAPI 3.x document");
}

#[test]
fn missing_document_errors() {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    let uri = Uri::from_path(&dir.path().join("nope.yaml")).unwrap();
    let err = IrSpec::from_workspace(&Arc::new(ws), &uri).unwrap_err();
    assert!(err.contains("not loaded"), "{err}");
}
