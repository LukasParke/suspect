//! Native TypeDoc verifies the public HTTP surface against HttpPlan metadata.

use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    typescript::http::{HttpConfig, HttpPlan, plan_http},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn fixture() -> HttpPlan {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let prose = "Literal </script><img src=x onerror=alert(1)> {@link Missing} @example nope */ export const injected=true; /* & < >";
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"HTTP docs","version":"1"},"servers":[{"url":"https://example.test/api/v1"}],"security":[{"odd-scheme":[]}],"components":{"securitySchemes":{"odd-scheme":{"type":"http","scheme":"bearer"}}},"paths":{"/items/{item-id}":{"get":{"operationId":"class","description":prose,"parameters":[{"name":"item-id","in":"path","required":true,"schema":{"type":"string"}},{"name":"filter","in":"query","schema":{"type":"array","items":{"type":"string"}}},{"name":"plain","in":"query","schema":{"type":"string"}}],"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}}},"418":{"description":"error","content":{"application/json":{"schema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}}},"422":{"description":"other error","content":{"application/json":{"schema":{"type":"object","properties":{"reason":{"type":"boolean"}},"required":["reason"]}}}}}}}}}).to_string()).unwrap();
    let c = contract(&path);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_http(c, &selected, HttpConfig::default()).unwrap()
}
fn build(root: &Path) -> Output {
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs");
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    Command::new(node).arg(tool).arg(root).output().unwrap()
}
fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}
fn native(files: &[OutFile]) -> (tempfile::TempDir, Output) {
    let d = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(files, d.path()).unwrap();
    let o = build(&d.path().join("typescript"));
    (d, o)
}

#[test]
#[ignore = "requires pinned Node 22.23.1, TypeDoc and TypeScript"]
fn native_http_docs_preserve_hostile_prose_names_and_actual_bindings() {
    let plan = fixture();
    let planned = plan.operations()[0].clone();
    let (directory, output) = native(&plan.render());
    assert!(output.status.success(), "{}", text(&output));
    let coverage: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("typescript/docs/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(coverage["httpImplemented"], true);
    assert_eq!(coverage["operations"][0]["name"], planned.function_name);
    assert_eq!(coverage["operations"][0]["documented"], true);
}

#[test]
#[ignore = "requires pinned Node 22.23.1 and TypeDoc"]
fn native_http_gate_rejects_manifest_export_prose_and_type_binding_mutations() {
    for mutation in [
        "manifest",
        "export",
        "prose",
        "binding",
        "swapped-responses",
        "guard-predicate",
        "query-requiredness",
        "query-model",
        "query-serialization",
        "query-source",
        "query-erased-alias",
        "query-erased-intrinsic",
    ] {
        let mut files = fixture().render();
        match mutation {
            "query-erased-alias" | "query-erased-intrinsic" => {
                let manifest: Value = serde_json::from_str(
                    &files
                        .iter()
                        .find(|file| file.path == "typescript/http-manifest.json")
                        .unwrap()
                        .content,
                )
                .unwrap();
                let parameters = manifest["operations"][0]["parameters"].as_array().unwrap();
                let original = parameters[2]["model"].as_str().unwrap();
                let wrong = parameters[0]["model"].as_str().unwrap();
                let replacement = if mutation == "query-erased-alias" {
                    format!("Models.{wrong}")
                } else {
                    "string".to_owned()
                };
                let file = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/operations.ts")
                    .unwrap();
                file.content = file.content.replacen(
                    &format!("plain?: Models.{original};"),
                    &format!("plain?: {replacement};"),
                    1,
                );
            }
            "query-requiredness" | "query-model" | "query-serialization" | "query-source" => {
                let file = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/http-manifest.json")
                    .unwrap();
                let mut manifest: Value = serde_json::from_str(&file.content).unwrap();
                let query = &mut manifest["operations"][0]["parameters"][1];
                match mutation {
                    "query-requiredness" => query["required"] = json!(true),
                    "query-model" => query["model"] = json!("MissingQueryModel"),
                    "query-serialization" => query["explode"] = json!(false),
                    "query-source" => query["source"]["pointer"] = json!("/wrong/source"),
                    _ => unreachable!(),
                }
                file.content = serde_json::to_string_pretty(&manifest).unwrap();
            }
            "manifest" => files.retain(|f| f.path != "typescript/http-manifest.json"),
            "export" => {
                let manifest: Value = serde_json::from_str(
                    &files
                        .iter()
                        .find(|x| x.path == "typescript/http-manifest.json")
                        .unwrap()
                        .content,
                )
                .unwrap();
                let export = manifest["operations"][0]["export"].as_str().unwrap();
                let f = files
                    .iter_mut()
                    .find(|f| f.path == "typescript/operations.ts")
                    .unwrap();
                f.content = f.content.replacen(
                    &format!("export function {export}"),
                    &format!("function {export}"),
                    1,
                )
            }
            "prose" => {
                let f = files
                    .iter_mut()
                    .find(|f| f.path == "typescript/operations.ts")
                    .unwrap();
                f.content = f
                    .content
                    .replacen("Literal &lt;/script&gt;", "Changed prose", 1)
            }
            "binding" => {
                let manifest: Value = serde_json::from_str(
                    &files
                        .iter()
                        .find(|x| x.path == "typescript/http-manifest.json")
                        .unwrap()
                        .content,
                )
                .unwrap();
                let success = manifest["operations"][0]["successType"].as_str().unwrap();
                let error = manifest["operations"][0]["errorType"].as_str().unwrap();
                let f = files
                    .iter_mut()
                    .find(|f| f.path == "typescript/operations.ts")
                    .unwrap();
                f.content = f.content.replacen(
                    &format!("Promise<{success}>"),
                    &format!("Promise<{error}>"),
                    1,
                )
            }
            "swapped-responses" => {
                let manifest: Value = serde_json::from_str(
                    &files
                        .iter()
                        .find(|x| x.path == "typescript/http-manifest.json")
                        .unwrap()
                        .content,
                )
                .unwrap();
                let responses = manifest["operations"][0]["responses"].as_array().unwrap();
                let first = responses.iter().find(|r| r["status"] == 418).unwrap()["model"]
                    .as_str()
                    .unwrap();
                let second = responses.iter().find(|r| r["status"] == 422).unwrap()["model"]
                    .as_str()
                    .unwrap();
                let f = files
                    .iter_mut()
                    .find(|f| f.path == "typescript/operations.ts")
                    .unwrap();
                let marker = "__SWAPPED_RESPONSE_MODEL__";
                f.content = f
                    .content
                    .replacen(
                        &format!("Models.{first}, 418"),
                        &format!("Models.{marker}, 418"),
                        1,
                    )
                    .replacen(
                        &format!("Models.{second}, 422"),
                        &format!("Models.{first}, 422"),
                        1,
                    )
                    .replacen(
                        &format!("Models.{marker}, 418"),
                        &format!("Models.{second}, 418"),
                        1,
                    );
            }
            "guard-predicate" => {
                let manifest: Value = serde_json::from_str(
                    &files
                        .iter()
                        .find(|x| x.path == "typescript/http-manifest.json")
                        .unwrap()
                        .content,
                )
                .unwrap();
                let error = manifest["operations"][0]["errorType"].as_str().unwrap();
                let success = manifest["operations"][0]["successType"].as_str().unwrap();
                let f = files
                    .iter_mut()
                    .find(|f| f.path == "typescript/operations.ts")
                    .unwrap();
                f.content = f.content.replacen(
                    &format!("error is {error}"),
                    &format!("error is {success}"),
                    1,
                );
            }
            _ => unreachable!(),
        };
        let (_d, o) = native(&files);
        assert!(
            !o.status.success(),
            "{mutation} mutation unexpectedly passed"
        );
    }
}

#[test]
#[ignore = "requires pinned Node 22.23.1, TypeDoc, TypeScript and tracked OpenRouter YAML"]
fn native_http_docs_cover_tracked_openrouter_operations_and_query_inputs() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let c = contract(&path);
    let ids = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = c
        .operations()
        .filter(|o| o.operation_id().is_some_and(|id| ids.contains(&id)))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), ids.len());
    let plan = plan_http(c, &selected, HttpConfig::default()).unwrap();
    let (d, o) = native(&plan.render());
    assert!(o.status.success(), "{}", text(&o));
    let coverage: Value = serde_json::from_slice(
        &std::fs::read(d.path().join("typescript/docs/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(coverage["operations"].as_array().unwrap().len(), ids.len());
}
