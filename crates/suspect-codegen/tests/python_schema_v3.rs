//! Actual resource/dynamic SDK operations with native checked JSON carriers.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    python_http::{self, HttpPlan, PackageConfig},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const IMPORT: &str = "resource_python_sdk";
fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-native-python-tools/bin/python")
}
fn artifacts() -> PathBuf {
    tempfile::Builder::new()
        .prefix("sdk-python-resources-sdk-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn load(document: &Value) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            Uri::parse("https://download.example/latest.json").unwrap(),
            Uri::parse("https://cdn.example/specs/api.json").unwrap(),
            serde_json::to_vec(document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse("https://download.example/latest.json").unwrap(),
        )
        .unwrap(),
    )
}
fn emit(root: &Path) -> (HttpPlan, Vec<OutFile>) {
    let fixture: Value =
        serde_json::from_str(include_str!("python_schema_v3/schemas.json")).unwrap();
    let mut document = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/api.json#revision","info":{"title":"Resource Python SDK","version":"1"},"servers":[{"url":"../api"}],"paths":{},"components":{"schemas":fixture["support"]}});
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        document["components"]["schemas"][name] = case["schema"].clone();
        let schema = json!({"$ref":format!("#/components/schemas/{name}")});
        document["paths"][case["path"].as_str().unwrap()] = json!({"post":{"operationId":case["operation"],"requestBody":{"required":true,"content":{"application/json":{"schema":schema,"example":case["good"]}}},"responses":{"200":{"description":"Checked resource response","content":{"application/json":{"schema":schema,"example":case["good"]}}}}}});
    }
    document["paths"]["/bytes"] = json!({"get":{"operationId":"resourceBytes","responses":{"200":{"description":"Opaque resource bytes","content":{"application/octet-stream":{"schema":{"$id":"urn:bytes","maxLength":8}}}}}}});
    fs::write(
        root.join("api.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("fixture.json"),
        include_str!("python_schema_v3/schemas.json"),
    )
    .unwrap();
    let contract = load(&document);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract, &selected, Default::default()).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "resource-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root.join("generated")).unwrap();
    (plan, files)
}
fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn python_resource_admission_keeps_ordinary_programs_and_typed_capture() {
    let root = artifacts();
    let (plan, _) = emit(&root);
    assert_eq!(
        plan.codecs().validation_program().version,
        OwnedProgram::V3_VERSION
    );
    assert_eq!(plan.operations().len(), 8);
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .iter()
            .any(|source| source.pointer().contains("/application~1octet-stream/"))
    );
    assert!(
        !plan
            .codecs()
            .validation_program()
            .nodes
            .iter()
            .any(|node| node.source.pointer == "/components/schemas/Nested")
    );
    assert!(
        plan.examples()
            .operations()
            .iter()
            .filter(|operation| operation.operation_id != "resourceBytes")
            .all(|operation| operation.entries.len() == 2)
    );
    let snapshot = suspect_codegen::compatibility::snapshot(
        plan.contract().clone(),
        &[],
        &[suspect_codegen::backend::TargetConfig {
            backend: suspect_codegen::backend::Backend::PythonHttp,
            package_name: "resource-python-sdk".into(),
            package_version: "1.0.0".into(),
            import_name: Some(IMPORT.into()),
        }],
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    assert!(
        native
            .operations
            .iter()
            .all(|operation| operation.descriptor["validation"]["profile"]
                == OwnedProgram::V3_PROFILE)
    );
    assert!(
        native
            .models
            .iter()
            .all(|model| model.descriptor.as_ref().unwrap()["kind"] == "alias")
    );
    for path in [
        "python_validation/runtime_v3.py",
        "python_validation/resource_guard.py",
        "python_http/urls.py",
    ] {
        assert!(
            native
                .runtime
                .fingerprinted_assets
                .contains(&path.to_owned())
        );
    }
    fs::write(
        root.join("capture.json"),
        serde_json::to_vec_pretty(native).unwrap(),
    )
    .unwrap();
    for (schema, expected) in [
        (json!({"type":"integer"}), OwnedProgram::V1_VERSION),
        (
            json!({"type":"object","unevaluatedProperties":false}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let document = json!({"openapi":"3.1.2","info":{"title":"Frozen base","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}});
        let contract = load(&document);
        let plan = suspect_codegen::python_codecs::plan_codecs(
            contract.clone(),
            contract.schema_roots(),
            Default::default(),
        )
        .unwrap();
        let roots = plan
            .validation_program()
            .roots
            .iter()
            .map(|root| {
                let mut source = SourceId::new(
                    Uri::parse(&root.source.document).unwrap(),
                    Default::default(),
                );
                for token in root.source.pointer.split('/').skip(1) {
                    source = source.child(&token.replace("~1", "/").replace("~0", "~"));
                }
                source
            })
            .collect::<Vec<_>>();
        let original = OwnedCompiler::new(Default::default())
            .compile_v2(contract, &roots)
            .unwrap()
            .program();
        assert_eq!(plan.validation_program().version, expected);
        assert_eq!(
            serde_json::to_vec(plan.validation_program()).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );
        assert!(
            !plan
                .render()
                .iter()
                .any(|file| file.path.contains("resource_guard")
                    || file.path == "python/validation_v2.py")
        );
    }
    println!(
        "Python resource admission/capture evidence: {}",
        root.display()
    );
}

#[test]
#[ignore = "requires installed Python 3.11/3.14, uv and cached mypy/wheel/Sphinx dependencies"]
fn installed_python_v3_dynamic_operations_native_examples_types_and_sphinx() {
    let root = artifacts();
    let (plan, files) = emit(&root);
    let package = root.join("generated/python");
    fs::write(
        root.join("consumer.py"),
        include_str!("python_schema_v3/consumer.py"),
    )
    .unwrap();
    fs::write(
        root.join("negative.py"),
        include_str!("python_schema_v3/negative.py"),
    )
    .unwrap();
    let snippets = root.join("readme-snippets");
    fs::create_dir(&snippets).unwrap();
    for (index, block) in files
        .iter()
        .find(|file| file.path == "python/README.md")
        .unwrap()
        .content
        .split("```python\n")
        .skip(1)
        .enumerate()
    {
        let code = block.split_once("\n```").unwrap().0.to_owned() + "\n";
        assert!(
            files
                .iter()
                .any(|file| file.path.starts_with("python/examples/") && file.content == code)
        );
        fs::write(snippets.join(format!("snippet_{index}.py")), code).unwrap();
    }
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(&package),
        &root,
        "build",
    );
    let wheel = fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let venv = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&venv),
            &root,
            &format!("venv-{version}"),
        );
        let python = venv.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel)
                .arg("Sphinx==8.2.3"),
            &root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(&python).arg(root.join("consumer.py")),
            &root,
            &format!("native-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("consumer.py"))
                .arg(package.join("examples"))
                .arg(&snippets),
            &root,
            &format!("mypy-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("package-mypy-{version}")))
                .arg(package.join(format!("src/{IMPORT}"))),
            &root,
            &format!("package-mypy-{version}"),
        );
        let negative = Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--python-version",
                version,
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join(format!("negative-cache-{version}")))
            .arg(root.join("negative.py"))
            .output()
            .unwrap();
        fs::write(
            root.join(format!("negative-{version}.log")),
            &negative.stdout,
        )
        .unwrap();
        assert!(!negative.status.success());
        let errors = String::from_utf8_lossy(&negative.stdout);
        for line in [3, 4, 5, 6, 7] {
            assert!(
                errors.contains(&format!("negative.py:{line}: error:")),
                "{errors}"
            );
        }
        checked(
            Command::new(&python).arg(package.join("examples/validated.py")),
            &root,
            &format!("examples-{version}"),
        );
        checked(
            Command::new(&python)
                .args([
                    "-m",
                    "sphinx",
                    "-W",
                    "--keep-going",
                    "-E",
                    "-b",
                    "html",
                    "docs",
                ])
                .arg(root.join(format!("sphinx-{version}")))
                .current_dir(&package),
            &root,
            &format!("sphinx-{version}"),
        );
        let coverage: Value = serde_json::from_slice(
            &fs::read(root.join(format!("sphinx-{version}/coverage.json"))).unwrap(),
        )
        .unwrap();
        assert!(coverage["documented"].as_array().unwrap().len() > 150);
    }
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","operations":plan.operations().len(),"models":plan.codecs().models().symbols().len(),"examples":plan.examples().operations().iter().map(|op|op.entries.len()).sum::<usize>(),"artifacts":files.len(),"validation":{"version":plan.codecs().validation_program().version,"profile":plan.codecs().validation_program().profile},"interpreters":["3.11","3.14"]})).unwrap()).unwrap();
    println!("Python v3 installed SDK evidence: {}", root.display());
}
