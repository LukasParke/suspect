//! Scoped (operation-selected) compilation must produce, for the selected
//! operations, exactly what the full compile produces: byte-identical
//! generated SDK files. Any difference would mean the scope changes more
//! than the time it takes.

use std::sync::Arc;

use suspect_codegen::backend::{self, Backend, GenerationOptions, TargetConfig};
use suspect_ir::contract::{Contract, OperationSelection};
use suspect_ref::WorkspaceBuilder;

const SPEC: &str = r##"openapi: 3.1.0
info:
  title: Scoped compile
  version: "1"
paths:
  /widgets:
    get:
      operationId: listWidgets
      parameters:
        - name: limit
          in: query
          schema: {type: integer}
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: array
                items: {$ref: "#/components/schemas/Widget"}
    post:
      operationId: createWidget
      requestBody:
        required: true
        content:
          application/json:
            schema: {$ref: "#/components/schemas/Widget"}
      responses:
        "201":
          description: created
          content:
            application/json:
              schema: {$ref: "#/components/schemas/Widget"}
  /widgets/{widgetId}:
    get:
      operationId: getWidget
      parameters:
        - name: widgetId
          in: path
          required: true
          schema: {type: string}
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: {$ref: "#/components/schemas/Widget"}
  /status:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: {$ref: "#/components/schemas/Status"}
components:
  schemas:
    Widget:
      type: object
      required: [id]
      properties:
        id: {type: string}
        size: {type: integer}
        tag: {type: string}
    Status:
      type: object
      properties:
        ok: {type: boolean}
"##;

fn compile_in(dir: &std::path::Path, selectors: Option<&[&str]>) -> Arc<Contract> {
    std::fs::write(dir.join("spec.yaml"), SPEC).expect("write");
    let ws = Arc::new(
        WorkspaceBuilder::new()
            .root(dir)
            .build()
            .expect("workspace"),
    );
    ws.load_all("spec.yaml").expect("load");
    let entry = ws.uris().first().expect("entry").clone();
    match selectors {
        None => Contract::from_workspace(&ws, &entry),
        Some(ids) => {
            let selection = OperationSelection::new(ids.iter().copied());
            Contract::from_workspace_scoped(&ws, &entry, &selection)
        }
    }
    .expect("compiles")
    .into()
}

fn compile(selectors: Option<&[&str]>) -> Arc<Contract> {
    let dir = tempfile::tempdir().expect("tempdir");
    compile_in(dir.path(), selectors)
}

fn sources(contract: &Contract) -> Vec<suspect_ir::contract::SourceId> {
    let mut sources: Vec<_> = contract
        .operations()
        .map(|op| op.source().clone())
        .collect();
    sources.sort();
    sources.dedup();
    sources
}

fn render(contract: Arc<Contract>, backend_kind: Backend, package: &str) -> Vec<(String, String)> {
    let target = TargetConfig {
        backend: backend_kind,
        package_name: package.to_owned(),
        package_version: "0.1.0".to_owned(),
        import_name: None,
    };
    let selected = sources(&contract);
    let files =
        backend::generate_with_options(contract, &selected, &target, &GenerationOptions::default())
            .expect("renders");
    let mut pairs: Vec<(String, String)> = files
        .into_iter()
        .map(|file| (file.path.clone(), file.content))
        .collect();
    pairs.sort();
    pairs
}

#[test]
fn scoped_with_every_operation_matches_the_full_compile_byte_for_byte() {
    // Both compiles share one directory: generated artifacts embed
    // absolute source paths, which would otherwise be the only difference.
    let dir = tempfile::tempdir().expect("tempdir");
    let full = compile_in(dir.path(), None);
    let ids: Vec<String> = full
        .operations()
        .filter_map(|op| op.operation_id().map(str::to_owned))
        .collect();
    assert_eq!(ids.len(), 3, "the fixture has three named operations");
    // The unnamed operation joins via its METHOD /path spelling, so the
    // selection covers every operation the full compile holds.
    let selectors: Vec<&str> = vec!["listWidgets", "createWidget", "getWidget", "GET /status"];
    let scoped = compile_in(dir.path(), Some(&selectors));

    for (backend_kind, package) in [
        (Backend::TypescriptHttp, "widgets-ts"),
        (Backend::PythonHttp, "widgets_py"),
        (Backend::GoHttp, "example.com/widgets"),
    ] {
        let from_full = render(full.clone(), backend_kind, package);
        let from_scoped = render(scoped.clone(), backend_kind, package);
        let paths_full: Vec<&String> = from_full.iter().map(|(path, _)| path).collect();
        let paths_scoped: Vec<&String> = from_scoped.iter().map(|(path, _)| path).collect();
        assert_eq!(
            paths_full, paths_scoped,
            "{package}: scoping must not change which artifacts render"
        );
        for ((path, full_content), (_, scoped_content)) in from_full.iter().zip(&from_scoped) {
            assert_eq!(
                full_content, scoped_content,
                "{path} ({package}): scoping must not change a single byte"
            );
        }
    }
}

#[test]
fn scoped_compile_keeps_only_the_selected_operations() {
    let scoped = compile(Some(&["createWidget"]));
    let ids: Vec<String> = scoped
        .operations()
        .filter_map(|op| op.operation_id().map(str::to_owned))
        .collect();
    assert_eq!(ids, vec!["createWidget".to_owned()]);

    // The shared schema closure is still reachable: the one kept operation
    // renders with its $ref resolved.
    let files = render(scoped, Backend::TypescriptHttp, "widgets-ts");
    assert!(
        files.iter().any(|(_, content)| content.contains("Widget")),
        "the referenced schema renders: {files:?}"
    );
}

#[test]
fn method_path_selectors_reach_unnamed_operations() {
    let scoped = compile(Some(&["GET /status"]));
    let sources = sources(&scoped);
    assert_eq!(sources.len(), 1, "the unnamed operation is selected");
    let files = render(scoped, Backend::TypescriptHttp, "widgets-ts");
    assert!(
        files.iter().any(|(_, content)| content.contains("Status")),
        "the unnamed operation's schema renders: {files:?}"
    );
}
