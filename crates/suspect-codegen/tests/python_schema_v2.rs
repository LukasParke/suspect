//! Source-driven native SDK adoption of the nine scoped validation operations.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    python_http::{self, HttpPlan, PackageConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::OwnedProgram;
use suspect_source::Uri;

const IMPORT: &str = "scoped_python_sdk";

fn root(label: &str) -> PathBuf {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    tempfile::Builder::new()
        .prefix(&format!("sdk-python-scoped-{label}-"))
        .tempdir_in(target)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("python_schema_v2/schemas.json")).unwrap()
}

fn emit(root: &Path) -> (HttpPlan, Vec<OutFile>) {
    let fixture = fixture();
    let mut document = json!({"openapi":"3.1.2","info":{"title":"Scoped Python SDK","version":"1"},"servers":[{"url":"https://scoped.example.test"}],"paths":{},"components":{"schemas":fixture["support"]}});
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        document["components"]["schemas"][name] = case["schema"].clone();
        let schema = json!({"$ref":format!("#/components/schemas/{name}")});
        document["paths"][case["path"].as_str().unwrap()] = json!({"post":{
            "operationId":case["operation"],"description":format!("Source-validated {name} request and response."),
            "requestBody":{"required":true,"content":{"application/json":{"schema":schema,"examples":{"accepted":{"value":case["good"]},"rejected":{"value":case["bad"]}}}}},
            "responses":{"200":{"description":"Checked response","content":{"application/json":{"schema":schema,"example":case["good"]}}}}
        }});
    }
    let path = root.join("api.json");
    fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract, &selected, Default::default()).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "scoped-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root.join("generated")).unwrap();
    fs::write(
        root.join("fixture.json"),
        include_str!("python_schema_v2/schemas.json"),
    )
    .unwrap();
    (plan, files)
}

fn json_file(files: &[OutFile], name: &str) -> Value {
    serde_json::from_str(&files.iter().find(|file| file.path == name).unwrap().content).unwrap()
}

#[test]
fn scoped_python_http_plan_preserves_carriers_examples_and_typed_capture() {
    let root = root("sdk-plan");
    let (plan, files) = emit(&root);
    assert_eq!(plan.operations().len(), 12);
    assert_eq!(
        plan.codecs().validation_program().version,
        OwnedProgram::V2_VERSION
    );
    let program = serde_json::to_value(plan.codecs().validation_program()).unwrap();
    let ops = program["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|node| node["checks"].as_array().unwrap())
        .filter_map(|check| check["op"].as_str())
        .collect::<BTreeSet<_>>();
    for op in [
        "if",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "patternProperties",
        "additionalPropertiesWithPatterns",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        assert!(ops.contains(op), "missing {op}");
    }
    let bindings = json_file(&files, "python/docs/source-bindings.json");
    assert_eq!(bindings["recipes"]["quickstart"]["available"], true);
    assert_eq!(bindings["recipes"]["presence"]["available"], true);
    let manifest = json_file(&files, &format!("python/src/{IMPORT}/http-manifest.json"));
    assert_eq!(manifest["validation"]["profile"], OwnedProgram::V2_PROFILE);
    let examples = json_file(&files, &format!("python/src/{IMPORT}/examples.json"));
    fs::write(
        root.join("examples.json"),
        serde_json::to_vec_pretty(&examples).unwrap(),
    )
    .unwrap();
    assert!(
        plan.examples()
            .operations()
            .iter()
            .all(|op| op.entries.len() >= 2),
        "every operation needs accepted source examples"
    );
    for op in plan.examples().operations() {
        assert!(
            op.entries.iter().all(|example| matches!(
                example.origin,
                suspect_codegen::examples::ExampleOrigin::Declared
            )),
            "all values are supplied by the source"
        );
    }
    let capture = suspect_codegen::compatibility::snapshot(
        plan.contract().clone(),
        &[],
        &[suspect_codegen::backend::TargetConfig {
            backend: suspect_codegen::backend::Backend::PythonHttp,
            package_name: "scoped-python-sdk".into(),
            package_version: "1.0.0".into(),
            import_name: Some(IMPORT.into()),
        }],
    )
    .unwrap();
    let native = &capture.native[0];
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    assert_eq!(native.operations.len(), 12);
    assert!(
        native
            .operations
            .iter()
            .all(|operation| operation.descriptor["validation"]["version"]
                == OwnedProgram::V2_VERSION)
    );
    let bag = native
        .models
        .iter()
        .find(|model| model.name == "PatternBag")
        .unwrap();
    assert_eq!(
        bag.descriptor.as_ref().unwrap()["extraType"]["name"],
        "_json.JsonValue"
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .contains(&"python_validation/runtime_v2.py".into())
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .contains(&"python_validation/guard.py".into())
    );
    fs::write(
        root.join("capture.json"),
        serde_json::to_vec_pretty(native).unwrap(),
    )
    .unwrap();
    println!("Scoped Python SDK plan: {}", root.display());
}

fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-native-python-tools/bin/python")
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
#[ignore = "requires Python 3.11/3.14, uv and cached wheel/mypy/Sphinx dependencies"]
fn installed_scoped_python_sdk_operations_mutations_examples_types_and_sphinx() {
    let root = root("sdk-native");
    let (plan, files) = emit(&root);
    let package = root.join("generated/python");
    fs::write(
        root.join("consumer.py"),
        include_str!("python_schema_v2/consumer.py"),
    )
    .unwrap();
    fs::write(
        root.join("negative.py"),
        include_str!("python_schema_v2/negative.py"),
    )
    .unwrap();
    let snippets = root.join("readme-snippets");
    fs::create_dir(&snippets).unwrap();
    let readme = &files
        .iter()
        .find(|file| file.path == "python/README.md")
        .unwrap()
        .content;
    for (index, block) in readme.split("```python\n").skip(1).enumerate() {
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
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    let tiers =
        std::env::var("SUSPECT_PYTHON_SCOPED_VERSIONS").unwrap_or_else(|_| "3.11,3.14".into());
    let versions = tiers.split(',').collect::<Vec<_>>();
    assert!(
        versions
            .iter()
            .all(|version| matches!(*version, "3.11" | "3.14"))
    );
    for &version in &versions {
        let environment = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&environment),
            &root,
            &format!("venv-{version}"),
        );
        let python = environment.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel)
                .args(["Sphinx==8.2.3"]),
            &root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(&python).arg("consumer.py").current_dir(&root),
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
        for line in [3, 4, 5, 6, 7, 8, 9] {
            assert!(
                errors.contains(&format!("negative.py:{line}: error:")),
                "{errors}"
            );
        }
        assert!(
            errors.contains("[arg-type]")
                && errors.contains("[call-arg]")
                && errors.contains("[list-item]"),
            "{errors}"
        );
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
        assert!(
            coverage["signatures"][format!("{IMPORT}.models.NullFields.flag")]
                .as_str()
                .unwrap()
                .contains("None")
        );
    }
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","operations":plan.operations().len(),"models":plan.codecs().models().symbols().len(),"examples":plan.examples().operations().iter().map(|op|op.entries.len()).sum::<usize>(),"artifacts":files.len(),"validation":{"version":plan.codecs().validation_program().version,"profile":plan.codecs().validation_program().profile},"interpreters":versions})).unwrap()).unwrap();
    println!("Scoped Python SDK installed evidence: {}", root.display());
}
