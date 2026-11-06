//! Tests for change-impact awareness.

use super::*;
use suspect_source::Source;

fn doc(text: &str) -> LowDoc {
    LowDoc::parse(
        "mem://impact.yaml".into(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

const SPEC: &str = r#"
openapi: 3.1.0
info: {title: Impact, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Pet'}
  /owners:
    get:
      operationId: listOwners
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Owner'}
components:
  schemas:
    Pet:
      type: object
      properties:
        name: {type: string}
    Owner:
      type: object
      properties:
        pet: {$ref: '#/components/schemas/Pet'}
"#;

fn context<'a>(
    index: &'a Index,
    spec: &'a IrSpec,
    workflows: &'a [(String, suspect_arazzo::ArazzoDoc<'a>)],
    artifacts: &'a std::collections::BTreeMap<String, Vec<String>>,
    traffic: &'a std::collections::BTreeMap<String, usize>,
) -> ImpactContext<'a> {
    ImpactContext {
        index,
        spec: Some(spec),
        workflows,
        artifacts,
        traffic,
    }
}

/// A workspace holding the spec, so the operation index is real.
fn workspace_with(spec: &str) -> (std::sync::Arc<suspect_ref::Workspace>, IrSpec, &LowDoc) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("openapi.yaml"), spec).expect("write");
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("workspace");
    ws.load_all("openapi.yaml").expect("load");
    let ws = std::sync::Arc::new(ws);
    let ir = IrSpec::from_workspace(&ws, &ws.uris()[0]).expect("contract");
    let low = ws.get(&ws.uris()[0]).expect("handle").doc();
    // The documents live as long as the workspace; leak the handle so the
    // borrow outlives this function.
    let low: &'static LowDoc = unsafe { std::mem::transmute(low) };
    std::mem::forget(ws.clone());
    std::mem::forget(dir);
    (ws, ir, low)
}

#[test]
fn a_change_reaches_every_operation_that_uses_the_definition() {
    let (_ws, spec, low) = workspace_with(SPEC);
    let mut index = Index::default();
    index.index_document(low.uri().as_str(), low);
    let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
    let no_artifacts = std::collections::BTreeMap::new();
    let no_traffic = std::collections::BTreeMap::new();
    let model = crate::meaning::Model::new(low);
    // The cursor is on Pet's `name` property, inside the schema body.
    let offset = low
        .inner()
        .bytes()
        .windows(11)
        .position(|window| window == b"name: {type")
        .expect("the name property")
        + 2;
    let meaning = model.at(offset).expect("meaning");
    let ctx = context(&index, &spec, &empty, &no_artifacts, &no_traffic);
    let report = ctx.impact_of(low.uri().as_str(), &meaning);

    assert!(
        report.origin.contains("/components/schemas/Pet"),
        "{}",
        report.origin
    );
    // Pet is the 200 response schema of /pets, and Owner (reached in one
    // hop) is the response schema of /owners: two operations, and the
    // second one is found through the graph rather than by name.
    assert_eq!(report.operations, 2, "{report:?}");
    let subjects: Vec<&str> = report
        .impacts
        .iter()
        .filter(|impact| impact.kind == ImpactKind::Operation)
        .map(|impact| impact.subject.as_str())
        .collect();
    assert!(subjects.contains(&"GET /pets"), "{subjects:?}");
    assert!(subjects.contains(&"GET /owners"), "{subjects:?}");
    assert!(
        report.summary().contains("2 operation(s)"),
        "{}",
        report.summary()
    );
    // The nearest hop is the direct response reference.
    let nearest = report
        .impacts
        .iter()
        .filter(|impact| impact.kind == ImpactKind::Operation)
        .map(|impact| impact.distance)
        .min()
        .expect("an operation");
    assert_eq!(
        nearest, 1,
        "a direct response reference is one hop: {report:?}"
    );
}

#[test]
fn a_local_change_reaches_nothing() {
    let low = doc(
        "openapi: 3.1.0\ninfo: {title: a, version: '1'}\npaths:\n  /x:\n    get:\n      operationId: getX\n      summary: Unused\n      responses: {'200': {description: ok}}\n",
    );
    let mut index = Index::default();
    index.index_document(low.uri().as_str(), &low);
    let spec = IrSpec::default();
    let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
    let no_artifacts = std::collections::BTreeMap::new();
    let no_traffic = std::collections::BTreeMap::new();
    let model = crate::meaning::Model::new(&low);
    let meaning = model
        .at(low.root().byte_range().start + 5)
        .expect("meaning");
    let ctx = context(&index, &spec, &empty, &no_artifacts, &no_traffic);
    let report = ctx.impact_of(low.uri().as_str(), &meaning);
    assert!(report.is_local(), "{report:?}");
    assert!(report.summary().contains("no other part"));
}

#[test]
fn artifacts_workflows_and_traffic_are_named_by_component() {
    let low = doc(SPEC);
    let mut index = Index::default();
    index.index_document(low.uri().as_str(), &low);
    let spec = IrSpec::default();
    let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
    let mut artifacts = std::collections::BTreeMap::new();
    artifacts.insert("Pet".to_owned(), vec!["python/models.py".to_owned()]);
    let mut traffic = std::collections::BTreeMap::new();
    traffic.insert("Pet".to_owned(), 12usize);
    let model = crate::meaning::Model::new(&low);
    let offset = low
        .inner()
        .bytes()
        .windows(11)
        .position(|window| window == b"name: {type")
        .expect("the name property")
        + 2;
    let meaning = model.at(offset).expect("meaning");
    let ctx = context(&index, &spec, &empty, &artifacts, &traffic);
    let report = ctx.impact_of(low.uri().as_str(), &meaning);
    assert_eq!(report.artifacts, 1, "{report:?}");
    assert_eq!(report.traffic, 1, "{report:?}");
    assert!(
        report.summary().contains("artifact"),
        "{}",
        report.summary()
    );
    assert!(
        report.summary().contains("recorded exchange"),
        "{}",
        report.summary()
    );
    assert!(
        report
            .impacts
            .iter()
            .any(|i| i.subject == "python/models.py"),
        "{report:?}"
    );
}

#[test]
fn component_names_are_read_from_the_pointer() {
    let low = doc(SPEC);
    let model = crate::meaning::Model::new(&low);
    for pointer in ["/components/schemas/Pet", "/components/schemas/Owner"] {
        let node = low
            .root()
            .pointer(&Pointer::parse(pointer).unwrap())
            .expect("node");
        let meaning = model.at(node.byte_range().start).expect("meaning");
        assert_eq!(
            component_name(&meaning).as_deref(),
            Some(&pointer[20..]),
            "{pointer}"
        );
    }
}
