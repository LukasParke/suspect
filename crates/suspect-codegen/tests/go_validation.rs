//! The same independent portable-program vectors execute in native Go.
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler};
use suspect_source::Uri;

#[test]
#[ignore = "requires native Go"]
fn portable_program_matches_shared_runtime_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let directory = tempfile::tempdir().unwrap().keep();
    for (index, case) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        let root = directory.join(index.to_string());
        std::fs::create_dir(&root).unwrap();
        let input = root.join("input.json");
        std::fs::write(&input,json!({"openapi":"3.1.0","info":{"title":"Vectors","version":"1"},"paths":{},"components":{"schemas":{"Root":case["schema"]}}}).to_string()).unwrap();
        let workspace = Arc::new(WorkspaceBuilder::new().build().unwrap());
        let contract = Arc::new(
            Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap(),
        );
        let id = contract
            .schema_roots()
            .iter()
            .find(|id| id.pointer() == "/components/schemas/Root")
            .unwrap()
            .clone();
        let program = OwnedCompiler::new(Config::default())
            .compile(contract, &[id])
            .unwrap()
            .program();
        let target = program.roots[0].target;
        let mut files = suspect_codegen::go_validation::emit(&program).unwrap();
        files.extend(suspect_codegen::go_json::emit());
        suspect_codegen::write_files(&files, &root).unwrap();
        let go = root.join("go");
        std::fs::write(
            go.join("go.mod"),
            format!("module example.com/vectors/{index}\n\ngo 1.23\n"),
        )
        .unwrap();
        std::fs::write(go.join("vectors.json"), case.to_string()).unwrap();
        std::fs::write(go.join("validation_test.go"),format!(r#"package sdk
import("testing";"os";"encoding/json";"errors")
func TestVectors(t *testing.T){{
 data,err:=os.ReadFile("vectors.json");if err!=nil{{t.Fatal(err)}}
 var item struct{{Name string;Valid,Invalid []string}};if err=json.Unmarshal(data,&item);err!=nil{{t.Fatal(err)}}
 for _,expect:=range []struct{{values []string;valid bool}}{{{{item.Valid,true}},{{item.Invalid,false}}}}{{for _,text:=range expect.values{{
  value,err:=Parse([]byte(text),DefaultLimits());if err!=nil{{t.Fatal(err)}}
  err=Validate({target},value)
  if expect.valid&&err!=nil{{t.Fatalf("%s %s: %v",item.Name,text,err)}}
  if !expect.valid{{var problem *ValidationError;if !errors.As(err,&problem)||problem.Kind!="invalid"{{t.Fatalf("%s %s: wanted invalid, got %v",item.Name,text,err)}}}}
 }}}}
}}
"#)).unwrap();
        let mut command = Command::new("go");
        command
            .args(["test", "./..."])
            .current_dir(&go)
            .env("GOWORK", "off");
        if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
            command.env("GOTOOLCHAIN", toolchain);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{} {}\n{}{}",
            directory.display(),
            case["name"],
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
