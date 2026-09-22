//! The public admission review: shared refusals, incoming admission,
//! contract limitations, and cross-convention naming analysis, all without
//! generating anything.

use std::path::Path;
use std::sync::Arc;

use suspect_codegen::admission::{self, FindingKind};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn temp(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("suspect-admission-{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("spec.yaml");
    std::fs::write(&path, body).unwrap();
    path
}

/// The admission profile requires exactly one static absolute HTTPS server
/// and one bearer security scheme.
const PREAMBLE: &str = "\
openapi: 3.1.0
info: {title: t, version: '1'}
servers:
  - url: https://api.example.com/v1
security:
  - apiKey: []
components:
  securitySchemes:
    apiKey:
      type: http
      scheme: bearer
";

const CLEAN: &str = "\
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
    post:
      operationId: createPet
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: {type: object}
";

#[test]
fn clean_documents_are_admissible_with_operation_verdicts() {
    let contract = load(&temp("clean", &format!("{PREAMBLE}{CLEAN}")));
    let report = admission::review(&contract);
    assert!(report.is_admissible(), "{:?}", report.findings);
    assert!(report.is_clean(), "{:?}", report.findings);
    let ids: Vec<&str> = report
        .operations
        .iter()
        .map(|o| o.operation_id.as_str())
        .collect();
    assert_eq!(ids, ["listPets", "createPet"]);
    let first = &report.operations[0];
    assert_eq!(first.method, "GET");
    assert_eq!(first.path, "/pets");
}

#[test]
fn unnamed_and_unsupported_operations_refuse() {
    let body = "\
paths:
  /a:
    get:
      responses:
        '200': {description: ok}
  /b:
    head:
      operationId: headB
      responses:
        '200': {description: ok}
";
    let contract = load(&temp("refuse", &format!("{PREAMBLE}{body}")));
    let report = admission::review(&contract);
    assert!(!report.is_admissible());
    let codes: Vec<&str> = report.findings.iter().map(|f| f.code).collect();
    assert!(codes.contains(&"http-operation-id"), "{codes:?}");
    assert!(codes.contains(&"http-method-unsupported"), "{codes:?}");
    // Guidance rides on refusals.
    assert!(
        report
            .findings
            .iter()
            .all(|f| !f.summary.is_empty() || !f.how_to_fix.is_empty()),
        "{:?}",
        report.findings
    );
}

#[test]
fn naming_collisions_are_advice_not_refusals() {
    let body = "openapi: 3.1.0
info: {title: t, version: '1'}
servers:
  - url: https://api.example.com/v1
security:
  - apiKey: []
paths:
  /a:
    get:
      operationId: get-pets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
  /b:
    get:
      operationId: get_pets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
components:
  securitySchemes:
    apiKey:
      type: http
      scheme: bearer
  schemas:
    my-model:
      type: object
    my_model:
      type: object
";
    let contract = load(&temp("naming", body));
    let report = admission::review(&contract);
    // Advice does not block admission.
    assert!(report.is_admissible(), "{:?}", report.findings);
    assert!(!report.is_clean());
    let method = report
        .findings
        .iter()
        .find(|f| f.code == "naming-method-collision")
        .expect("method collision");
    assert_eq!(method.kind, FindingKind::Advice);
    assert!(method.message.contains("get-pets"));
    assert!(method.message.contains("get_pets"));
    assert!(
        method.how_to_fix.contains("method names differ"),
        "{method:?}"
    );
    let model = report
        .findings
        .iter()
        .find(|f| f.code == "naming-model-collision")
        .expect("model collision");
    assert!(model.message.contains("my-model"));
    assert!(model.message.contains("my_model"));
    // Identical ids collapse too.
    let dup = "openapi: 3.1.0
info: {title: t, version: '1'}
servers:
  - url: https://api.example.com/v1
security:
  - apiKey: []
paths:
  /a:
    get:
      operationId: same
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
  /b:
    get:
      operationId: same
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
components:
  securitySchemes:
    apiKey:
      type: http
      scheme: bearer
";
    let contract = load(&temp("naming-dup", dup));
    let report = admission::review(&contract);
    // Identical ids are a hard refusal (the shared layer rejects them);
    // separator/case-only differences are the cross-convention advice.
    assert!(!report.is_admissible());
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "DUPLICATE_OPERATION_ID"),
        "{:?}",
        report.findings
    );
}

#[test]
fn broken_incoming_declarations_refuse() {
    // OAS 3.0 has no webhooks collection: the contract index skips it, so
    // the declaration would vanish from every generated SDK.
    let preamble = PREAMBLE.replace("openapi: 3.1.0", "openapi: 3.0.3");
    let body = "\
paths: {}
webhooks:
  petCreated:
    post:
      operationId: petCreated
      responses:
        '200': {description: ok}
";
    let contract = load(&temp("incoming", &format!("{preamble}{body}")));
    let report = admission::review(&contract);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "sdk-incoming-version" && f.kind == FindingKind::Refusal),
        "{:?}",
        report.findings
    );
}

#[test]
fn contract_limitations_surface_in_the_report() {
    // An unresolved reference is a contract-level limitation the report
    // must carry (and refuse on) rather than crash on.
    let body = "\
paths:
  /a:
    get:
      operationId: getA
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Missing'
";
    let contract = load(&temp("contract", &format!("{PREAMBLE}{body}")));
    let report = admission::review(&contract);
    assert!(
        !report.findings.is_empty(),
        "unresolved reference must surface: {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().all(|f| f.at.start <= f.at.end),
        "ranges must be well-formed: {:?}",
        report.findings
    );
}
