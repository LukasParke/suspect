//! Cross-runtime normative/adversarial vectors for the Python program executor.
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler};
use suspect_source::Uri;

#[test]
#[ignore = "requires native Python"]
fn portable_program_matches_independent_runtime_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let directory = tempfile::tempdir().unwrap().keep();
    for (index, case) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        let root = directory.join(index.to_string());
        std::fs::create_dir(&root).unwrap();
        let input = root.join("input.json");
        std::fs::write(&input,json!({"openapi":"3.1.0","info":{"title":"Runtime vectors","version":"1"},"paths":{},"components":{"schemas":{"Root":case["schema"]}}}).to_string()).unwrap();
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
        let compiled = OwnedCompiler::new(Config::default())
            .compile(contract, &[id])
            .unwrap();
        let program = compiled.program();
        let target = program.roots[0].target;
        let mut files = suspect_codegen::python_validation::emit(&program).unwrap();
        files.extend(suspect_codegen::python_json::emit());
        suspect_codegen::write_files(&files, &root).unwrap();
        let python = root.join("python");
        std::fs::write(python.join("vectors.json"), case.to_string()).unwrap();
        std::fs::write(
            python.join("consumer.py"),
            format!(
                r#"import json
from json_runtime import parse_json
from validation import validate, ValidationError
case=json.load(open('vectors.json'))
for expected in ('valid','invalid'):
    for text in case[expected]:
        try: validate({target},parse_json(text))
        except ValidationError as error:
            assert expected=='invalid' and error.kind=='invalid',(case['name'],text,expected,error)
        else: assert expected=='valid',(case['name'],text,'unexpected valid')
"#
            ),
        )
        .unwrap();
        let output = Command::new(
            std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()),
        )
        .arg("consumer.py")
        .current_dir(&python)
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{} {}\n{}",
            directory.display(),
            case["name"],
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
