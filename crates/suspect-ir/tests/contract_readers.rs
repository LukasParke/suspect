//! Reader-parity tests: `ContractReader::Fast` must produce the same
//! contract values as `ContractReader::Lossless` for accepted inputs, and
//! must decline unsupported fast syntax with an explicit error.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, ContractReader};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const SPEC: &str = r#"
openapi: "3.1.0"
info:
  title: Readers
  version: "1"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: 'parts.yaml#/Pet'
      responses:
        '201':
          description: Created
          content:
            application/json:
              schema:
                $ref: 'parts.yaml#/Pet'
security: []
components:
  schemas:
    Node:
      type: object
      properties:
        next:
          $ref: '#/components/schemas/Node'
        huge:
          type: number
          maximum: 1.0e+308
          minimum: -4.9e-324
          enum: [9007199254740993, 1.0e-10, 0.1]
        flagged:
          type: string
          const: "true"
        "on":
          type: string
          const: "3.1.0"
"#;

const PARTS: &str = r#"
Pet:
  type: object
  required: [id, name]
  properties:
    id:
      type: integer
      format: int64
    name:
      type: string
"#;

fn compile(
    dir: &tempfile::TempDir,
    entry: &str,
    reader: ContractReader,
) -> Result<Contract, suspect_ir::contract::ContractError> {
    std::fs::write(dir.path().join("api.yaml"), SPEC).unwrap();
    std::fs::write(dir.path().join("parts.yaml"), PARTS).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    Contract::from_workspace_with_reader(
        &workspace,
        &Uri::from_path(&dir.path().join(entry)).unwrap(),
        reader,
    )
}

fn documents(contract: &Contract) -> BTreeMap<String, Value> {
    contract
        .documents()
        .map(|(uri, value)| (uri.to_string(), value.clone()))
        .collect()
}

fn schemas(contract: &Contract) -> BTreeMap<(String, String), Value> {
    contract
        .schemas()
        .map(|schema| {
            (
                (
                    schema.id().document().to_string(),
                    schema.id().pointer().to_owned(),
                ),
                schema.raw().clone(),
            )
        })
        .collect()
}

#[test]
fn fast_matches_lossless_for_block_yaml_json_equivalents_and_splits() {
    let dir = tempfile::tempdir().unwrap();
    let lossless = compile(&dir, "api.yaml", ContractReader::Lossless).unwrap();
    let fast = compile(&dir, "api.yaml", ContractReader::Fast).unwrap();
    assert!(!lossless.has_errors(), "{:?}", lossless.diagnostics());
    assert!(!fast.has_errors(), "{:?}", fast.diagnostics());
    assert_eq!(documents(&lossless), documents(&fast));
    assert_eq!(schemas(&lossless), schemas(&fast));
}

/// JSON encoding of the same spec materializes identically under both readers.
#[test]
fn fast_matches_lossless_for_equivalent_json_entry() {
    let dir = tempfile::tempdir().unwrap();
    let json_entry = json!({
        "openapi": "3.1.0",
        "info": {"title": "Readers", "version": "1"},
        "paths": {
            "/pets": {
                "post": {
                    "operationId": "createPet",
                    "requestBody": {"required": true, "content": {
                        "application/json": {"schema": {"$ref": "parts.json#/Pet"}}
                    }},
                    "responses": {
                        "201": {"description": "Created", "content": {
                            "application/json": {"schema": {"$ref": "parts.json#/Pet"}}
                        }}
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "next": {"$ref": "#/components/schemas/Node"},
                        "huge": {"type": "number", "maximum": 1.0e308, "minimum": -4.9e-324,
                            "enum": [9007199254740993_u64, 1.0e-10, 0.1]},
                        "flagged": {"type": "string", "const": "true"},
                        "on": {"type":"string","const":"3.1.0"}
                    }
                }
            }
        }
    });
    let parts = json!({"Pet": {"type": "object", "required": ["id", "name"],
        "properties": {"id": {"type": "integer", "format": "int64"},
            "name": {"type": "string"}}}});
    std::fs::write(
        dir.path().join("api.json"),
        serde_json::to_vec(&json_entry).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("parts.json"),
        serde_json::to_vec(&parts).unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let uri = Uri::from_path(&dir.path().join("api.json")).unwrap();
    let lossless =
        Contract::from_workspace_with_reader(&workspace, &uri, ContractReader::Lossless).unwrap();
    let fast =
        Contract::from_workspace_with_reader(&workspace, &uri, ContractReader::Fast).unwrap();
    assert!(!lossless.has_errors(), "{:?}", lossless.diagnostics());
    assert!(!fast.has_errors(), "{:?}", fast.diagnostics());
    assert_eq!(documents(&lossless), documents(&fast));
    assert_eq!(schemas(&lossless), schemas(&fast));
}

#[test]
fn fast_and_lossless_agree_on_references_and_source_spans() {
    let dir = tempfile::tempdir().unwrap();
    let lossless = compile(&dir, "api.yaml", ContractReader::Lossless).unwrap();
    let fast = compile(&dir, "api.yaml", ContractReader::Fast).unwrap();
    for contract in [&lossless, &fast] {
        let node = contract
            .schemas()
            .find(|schema| schema.id().pointer() == "/components/schemas/Node")
            .unwrap();
        let next = node
            .raw()
            .pointer("/properties/next")
            .and_then(|value| value.get("$ref"))
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(next, "#/components/schemas/Node");
        let reference = contract
            .schema(&node.id().child("properties").child("next"))
            .unwrap();
        let target = reference.references()[0].target.as_ref().unwrap();
        assert_eq!(target.pointer(), "/components/schemas/Node");
        assert!(contract.source_span(node.id()).is_some());
    }
    assert_eq!(
        lossless.reachable_from(lossless.schema_roots()),
        fast.reachable_from(fast.schema_roots())
    );
    assert!(
        lossless
            .reachable_from(lossless.schema_roots())
            .iter()
            .any(|source| source.pointer() == "/components/schemas/Node/properties/huge")
    );
}

#[test]
fn fast_declines_unsupported_yaml_with_explicit_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("anchor.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\ncomponents:\n  schemas:\n    A:\n      type: object\n      default: &vals\n        kind: single\n",
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let uri = Uri::from_path(&dir.path().join("anchor.yaml")).unwrap();
    let lossless =
        Contract::from_workspace_with_reader(&workspace, &uri, ContractReader::Lossless).unwrap();
    assert!(!lossless.has_errors(), "{:?}", lossless.diagnostics());
    let error =
        Contract::from_workspace_with_reader(&workspace, &uri, ContractReader::Fast).unwrap_err();
    assert!(
        error.to_string().contains("fast reader declined"),
        "unexpected error: {error}"
    );
}

#[test]
fn invalid_syntax_fails_under_both_readers_and_duplicate_keys_stay_invariant() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.yaml"), "openapi: 3.1.0\ninfo: [\n").unwrap();
    std::fs::write(
        dir.path().join("dup.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: A\n  title: B\n  version: \"1\"\n",
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    for name in ["bad.yaml", "dup.yaml"] {
        let uri = Uri::from_path(&dir.path().join(name)).unwrap();
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            assert!(
                Contract::from_workspace_with_reader(&workspace, &uri, reader).is_err(),
                "{name} must fail with {reader:?}"
            );
        }
    }
}
const SPLIT_API: &str = r#"
openapi: 3.1.0
info: {title: Reader split matrix, version: '1'}
servers:
  - url: https://example.test/api
security:
  - bearer: []
paths:
  /items/{id}:
    $ref: parts.yaml#/Path
components:
  securitySchemes:
    bearer:
      $ref: parts.yaml#/Bearer
"#;

const SPLIT_PARTS: &str = r#"
Bearer:
  type: http
  scheme: bearer
Id:
  name: id
  in: path
  required: true
  schema: {type: string}
Path:
  parameters:
    - $ref: '#/Id'
    - name: limit
      in: query
      schema: {type: integer}
  post:
    operationId: upsert
    parameters:
      - name: limit
        in: query
        required: true
        schema: {type: integer, minimum: 1, maximum: 100}
    requestBody:
      $ref: '#/Body'
    responses:
      '201':
        $ref: '#/Reply'
Body:
  required: true
  content:
    application/json:
      schema:
        $ref: '#/Node'
Reply:
  description: Created
  headers:
    X-Exact:
      schema: {type: number, example: 1e-4000}
  content:
    application/json:
      schema:
        $ref: '#/Node'
Node:
  $anchor: Node
  type: object
  required: [name]
  properties:
    name:
      type: string
      const: '雪/~/\key'
    child:
      $ref: '#Node'
    amount:
      type: number
      minimum: -18446744073709551617.00000000001
      maximum: 1e4000
      example: 1e-4000
"#;

fn load(path: &std::path::Path, reader: ContractReader) -> Contract {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Contract::from_workspace_with_reader(&workspace, &Uri::from_path(path).unwrap(), reader)
        .unwrap()
}

#[test]
fn split_http_references_overrides_static_anchors_and_exact_values_survive_both_readers() {
    use suspect_ir::contract::{ParameterLocation, ResponseStatus, SecuritySchemeKind};
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("api.yaml");
    std::fs::write(&entry, SPLIT_API).unwrap();
    std::fs::write(dir.path().join("parts.yaml"), SPLIT_PARTS).unwrap();
    let left = load(&entry, ContractReader::Lossless);
    let right = load(&entry, ContractReader::Fast);
    assert_eq!(documents(&left), documents(&right));
    assert_eq!(schemas(&left), schemas(&right));
    for contract in [&left, &right] {
        assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
        assert_eq!(contract.documents().count(), 2);
        let operation = contract.operations().next().unwrap();
        assert_eq!(operation.operation_id(), Some("upsert"));
        assert_eq!(operation.path_template(), Some("/items/{id}"));
        assert_eq!(
            operation.effective_servers()[0].url(),
            Some("https://example.test/api")
        );
        let scheme = operation.effective_security()[0].requirements()[0]
            .scheme()
            .unwrap();
        assert_eq!(scheme.kind(), Some(SecuritySchemeKind::Http));
        assert_eq!(scheme.http_scheme(), Some("bearer"));
        let terminal = scheme.resolved_source().unwrap();
        assert_eq!(terminal.pointer(), "/Bearer");
        assert!(terminal.document().as_str().ends_with("/parts.yaml"));
        let parameters = operation.parameters();
        assert_eq!(parameters.len(), 2);
        let limit = parameters
            .iter()
            .find(|p| p.name() == Some("limit"))
            .unwrap();
        assert_eq!(limit.required(), Some(true));
        assert_eq!(limit.location(), Some(ParameterLocation::Query));
        assert_eq!(limit.schema().unwrap().raw()["maximum"], json!(100));
        let body = operation.request_body().unwrap();
        assert_eq!(body.resolved_source().unwrap().pointer(), "/Body");
        assert_eq!(body.required(), Some(true));
        let responses = operation.responses();
        assert_eq!(responses[0].status(), Some(ResponseStatus::Exact(201)));
        assert_eq!(responses[0].resolved_source().unwrap().pointer(), "/Reply");
        let node = contract
            .schemas()
            .find(|schema| schema.id().pointer() == "/Node")
            .unwrap();
        assert_eq!(node.raw()["properties"]["name"]["const"], "雪/~/\\key");
        assert_eq!(
            node.raw()["properties"]["amount"]["minimum"].to_string(),
            "-18446744073709551617.00000000001"
        );
        assert_eq!(
            node.raw()["properties"]["amount"]["maximum"].to_string(),
            "1e+4000"
        );
        assert_eq!(
            node.raw()["properties"]["amount"]["example"].to_string(),
            "1e-4000"
        );
        let child = contract
            .schema(&node.id().child("properties").child("child"))
            .unwrap();
        assert_eq!(child.references()[0].target.as_ref().unwrap(), node.id());
        assert_eq!(contract.source_span(node.id()), left.source_span(node.id()));
    }
    dir.close().unwrap();
    assert_eq!(
        documents(&left),
        documents(&right),
        "owned graphs survive deletion of source/workspaces"
    );
}

fn reverse_json(value: &Value) -> String {
    match value {
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .rev()
                .map(|(key, value)| format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    reverse_json(value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(reverse_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

#[test]
fn yaml_json_and_reordered_json_share_semantics_without_rounding() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = dir.path().join("api.yaml");
    std::fs::write(&yaml,"openapi: 3.1.0\ninfo: {title: Matrix, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Exact:\n      type: number\n      minimum: 9007199254740993\n      maximum: 1e4000\n      example: 1e-4000\n    Nullable:\n      type: [string, 'null']\n      enum: ['true', null, '雪']\n").unwrap();
    let expected:Value=serde_json::from_str(r#"{"openapi":"3.1.0","info":{"title":"Matrix","version":"1"},"paths":{},"components":{"schemas":{"Exact":{"type":"number","minimum":9007199254740993,"maximum":1e4000,"example":1e-4000},"Nullable":{"type":["string","null"],"enum":["true",null,"雪"]}}}}"#).unwrap();
    let json = dir.path().join("api.json");
    for data in [expected.to_string(), reverse_json(&expected)] {
        std::fs::write(&json, data).unwrap();
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            for path in [&yaml, &json] {
                let contract = load(path, reader);
                assert_eq!(contract.document(contract.entry()).unwrap(), &expected);
                assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
            }
        }
    }
}

#[test]
fn malformed_retained_declarations_have_identical_located_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.yaml");
    std::fs::write(&path,"openapi: 3.1.0\ninfo: {title: Bad, version: '1'}\npaths:\n  /items:\n    get:\n      operationId: bad\n      parameters: null\n      responses:\n        '20X':\n          description: 42\n").unwrap();
    let left = load(&path, ContractReader::Lossless);
    let right = load(&path, ContractReader::Fast);
    let findings = |contract: &Contract| {
        contract
            .diagnostics()
            .iter()
            .map(|d| json!({"source":d.source.pointer(),"code":d.code,"at":d.at}))
            .collect::<Vec<_>>()
    };
    assert_eq!(findings(&left), findings(&right));
    for contract in [&left, &right] {
        assert!(contract.has_errors());
        let op = contract.operations().next().unwrap();
        let collections = op.parameter_collections();
        assert_eq!(collections.len(), 1);
        assert_eq!(contract.source(&collections[0]), Some(&Value::Null));
    }
}

#[test]
#[ignore = "requires the four tracked OpenRouter specification snapshots"]
fn tracked_corpus_values_and_reference_targets_match_under_both_readers() {
    let root = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    for relative in [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ] {
        let path = root.join(relative);
        let left = load(&path, ContractReader::Lossless);
        let right = load(&path, ContractReader::Fast);
        assert_eq!(documents(&left), documents(&right), "{relative}");
        assert_eq!(schemas(&left), schemas(&right), "{relative}");
        for schema in left.schemas() {
            let same = right.schema(schema.id()).unwrap();
            assert_eq!(
                schema
                    .references()
                    .iter()
                    .map(|reference| &reference.target)
                    .collect::<Vec<_>>(),
                same.references()
                    .iter()
                    .map(|reference| &reference.target)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(left.operations().count(), right.operations().count());
        assert_eq!(
            left.diagnostics()
                .iter()
                .map(|d| (&d.source, d.code, &d.at))
                .collect::<Vec<_>>(),
            right
                .diagnostics()
                .iter()
                .map(|d| (&d.source, d.code, &d.at))
                .collect::<Vec<_>>()
        );
    }
}
