//! Documentation is an executable artifact: source-bound public inventories,
//! installed Python signatures, native Go declarations, strict Sphinx links,
//! typed generated samples and an independently specified HTTP fixture.
//!
//! Native gates are opt-in with --ignored. They retain private fixtures on disk
//! for diagnosis and never install into the user's interpreter or publish.

use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use suspect_codegen::{OutFile, go_http, python_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const IMPORT: &str = "m3_documented_sdk";
const MODULE: &str = "example.com/m3-documented-sdk";
const PROSE: &str = "Literal </script><img src=x onerror=alert(1)> & `field` [link]\n\n.. raw:: html\n\n   <script id=description-do-not-execute>alert(1)</script>\n\n.. include:: description-do-not-read\n\n:ref:`description-missing-target`\u{2028}.. raw:: html\n\n   <img src=description-injection>\n```python\nraise RuntimeError('description')\n```";

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn canonical_document() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = contract(&path);
    contract.document(contract.entry()).unwrap().clone()
}

fn fixture(root: &Path, mut document: Value) -> Arc<Contract> {
    document["paths"]["/widgets"]["post"]["description"] = json!(PROSE);
    document["components"]["schemas"]["Widget"]["description"] = json!(PROSE);
    document["components"]["schemas"]["Widget"]["properties"]["amount"]["description"] =
        json!(PROSE);
    let source = root.join("api.json");
    std::fs::write(&source, document.to_string()).unwrap();
    contract(&source)
}

fn python(plan: &python_http::HttpPlan) -> Vec<OutFile> {
    python_http::emit_http(
        plan,
        &python_http::PackageConfig {
            name: "m3-documented-sdk".into(),
            version: "0.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap()
}

fn go(plan: &go_http::HttpPlan) -> Vec<OutFile> {
    go_http::emit_http(
        plan,
        &go_http::PackageConfig {
            module_path: MODULE.into(),
            package_name: "sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap()
}

fn file<'a>(files: &'a [OutFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("missing artifact {path}"))
        .content
}

fn json_file(files: &[OutFile], path: &str) -> Value {
    serde_json::from_str(file(files, path)).unwrap()
}

fn assert_source(contract: &Contract, source: &Value) {
    let document = contract
        .documents()
        .find(|(uri, _)| uri.as_str() == source["document"].as_str().unwrap())
        .unwrap()
        .1;
    assert!(
        document
            .pointer(source["pointer"].as_str().unwrap())
            .is_some(),
        "unbound source {source}"
    );
}

fn inventory(files: &[OutFile], language: &str, contract: &Contract) -> (Value, BTreeSet<String>) {
    assert_eq!(
        files.len(),
        files
            .iter()
            .map(|file| &file.path)
            .collect::<BTreeSet<_>>()
            .len(),
        "artifact path collision"
    );
    let bindings = json_file(files, &format!("{language}/docs/source-bindings.json"));
    let api = file(files, &format!("{language}/docs/api.rst"));
    let mut names = BTreeSet::new();
    for symbol in bindings["symbols"].as_array().unwrap() {
        let name = symbol["name"].as_str().unwrap();
        assert!(
            names.insert(name.to_owned()),
            "duplicate documentation symbol {name}"
        );
        file(
            files,
            &format!("{language}/{}", symbol["file"].as_str().unwrap()),
        );
        assert!(
            api.contains(&format!(
                ".. _{}:",
                symbol["documentation"]["anchor"].as_str().unwrap()
            )),
            "undocumented symbol {name}"
        );
        if !symbol["source"].is_null() {
            assert_source(contract, &symbol["source"]);
        }
    }
    for symbol in bindings["symbols"].as_array().unwrap() {
        for link in symbol["related"].as_array().unwrap() {
            assert!(
                names.contains(link.as_str().unwrap()),
                "broken planned symbol link {link}"
            );
        }
    }
    for operation in bindings["operations"].as_array().unwrap() {
        assert_source(contract, &operation["source"]);
        for parameter in operation["parameters"].as_array().unwrap() {
            assert_source(contract, &parameter["source"]);
            assert_source(contract, &parameter["schema"]);
        }
        for response in operation["responses"].as_array().unwrap() {
            assert_source(contract, &response["source"]);
            assert_source(contract, &response["schema"]);
        }
    }
    (bindings, names)
}

#[test]
fn all_planned_native_symbols_have_browsable_source_bound_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let contract = fixture(root.path(), canonical_document());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let py = python_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    let go_plan = go_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    let py_files = python(&py);
    let go_files = go(&go_plan);
    let (py_docs, py_names) = inventory(&py_files, "python", &contract);
    let (go_docs, go_names) = inventory(&go_files, "go", &contract);
    assert_eq!(py_docs["import"], IMPORT);
    assert_eq!(go_docs["module"], MODULE);
    assert_eq!(py_docs["operations"].as_array().unwrap().len(), 4);
    assert_eq!(go_docs["operations"].as_array().unwrap().len(), 4);
    for symbol in py.codecs().models().symbols() {
        assert!(py_names.contains(&format!("{IMPORT}.models.{}", symbol.name())));
        assert!(py_names.contains(&format!("{IMPORT}.model_codecs.{}Codec", symbol.name())));
    }
    for field in py.codecs().models().fields() {
        assert!(py_names.contains(&format!("{IMPORT}.models.{}.{}", field.model, field.field)));
    }
    for op in py.operations() {
        for client in ["Client", "AsyncClient"] {
            assert!(py_names.contains(&format!("{IMPORT}.{client}.{}", op.snake_name)));
        }
        for response in op.responses() {
            assert!(py_names.contains(&format!("{IMPORT}._client.{}", response.class_name)));
        }
    }
    for symbol in go_plan.codecs().models().symbols() {
        assert!(go_names.contains(symbol.name()));
        assert!(go_names.contains(&format!("Codecs.{}", symbol.name())));
    }
    for field in go_plan.codecs().models().fields() {
        assert!(go_names.contains(&format!("{}.{}", field.model, field.field)));
    }
    for op in go_plan.operations() {
        assert!(go_names.contains(&format!("Client.{}", op.method_name)));
        assert!(go_names.contains(&op.input_constructor));
        for parameter in op.parameters() {
            if let Some(setter) = &parameter.setter_name {
                assert!(go_names.contains(&format!("{}.{setter}", op.input_type)));
            }
        }
        for response in op.responses() {
            assert!(go_names.contains(&response.type_name));
        }
    }
    for bindings in [&py_docs, &go_docs] {
        assert!(
            bindings["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|symbol| symbol["description"] == PROSE),
            "source prose must be retained verbatim as metadata"
        );
        assert!(
            bindings["examples"]
                .as_array()
                .unwrap()
                .iter()
                .all(|example| example["available"] == true)
        );
    }
    assert!(
        file(&py_files, "python/examples/validated.py")
            .contains(&format!("from {IMPORT} import Client"))
    );
    assert!(file(&go_files, "go/examples/validated/main.go").contains(MODULE));
    // Source descriptions cannot become document structure, including Unicode
    // separators that Python's text readers consider line boundaries.
    for (files, language) in [(&py_files, "python"), (&go_files, "go")] {
        let api = file(files, &format!("{language}/docs/api.rst"));
        assert!(!api.lines().any(
            |line| line.starts_with(".. raw::") || line.starts_with(".. include:: description")
        ));
    }
    assert_eq!(py_files, python(&py), "rendering must be deterministic");
    assert_eq!(go_files, go(&go_plan), "rendering must be deterministic");
}

#[test]
fn examples_bind_real_slots_and_report_missing_required_values() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("slots.json");
    let document = json!({
        "openapi":"3.1.0","info":{"title":"Native example slots","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"key":[]}],
        "components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}},
            "parameters":{"Path":{"name":"value","in":"path","required":true,"schema":{"type":"string"},"example":"path/value"}},
            "requestBodies":{"Body":{"required":true,"content":{"application/json":{"schema":{"type":"string"},"example":"body-value"}}}}},
        "paths":{
            "/slots/{value}":{"post":{"operationId":"bindSlots","parameters":[
                {"$ref":"#/components/parameters/Path"},
                {"name":"value","in":"query","schema":{"type":"string"},"example":"query-value"},
                {"name":"optional","in":"query","schema":{"type":"string","minLength":2,"maxLength":1}},
                {"name":"invalid-declaration","in":"query","schema":{"type":"string"},"example":false}
            ],"requestBody":{"$ref":"#/components/requestBodies/Body"},"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"string"},"example":"ok"}}}}}},
            "/missing/{value}":{"get":{"operationId":"missingRequired","parameters":[{"name":"value","in":"path","required":true,"schema":{"type":"string","minLength":2,"maxLength":1}}],
                "responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"string"}}}}}}}
        }
    });
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = contract(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let py = python_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    let go_plan = go_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    for (files, language, example_path) in [
        (
            python(&py),
            "python",
            format!("python/src/{IMPORT}/examples.json"),
        ),
        (go(&go_plan), "go", "go/examples.json".into()),
    ] {
        let (docs, _) = inventory(&files, language, &contract);
        let examples = json_file(&files, &example_path);
        assert!(
            examples["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "examples-declared-invalid")
        );
        let available = docs["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["available"] == true)
            .unwrap();
        let bound = available["bindings"].as_array().unwrap();
        assert_eq!(
            bound.len(),
            4,
            "path, query, synthesized valid query and referenced body; optional unavailable is absent; {language}: {available}"
        );
        let operation = examples["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["source"] == available["source"])
            .unwrap();
        let values = bound
            .iter()
            .map(|slot| {
                let value = &operation["entries"][slot["entry"].as_u64().unwrap() as usize];
                assert_eq!(slot["container"], value["container"]);
                value["value"].as_str().unwrap()
            })
            .collect::<BTreeSet<_>>();
        assert!(BTreeSet::from(["path/value", "query-value", "body-value"]).is_subset(&values));
        assert!(bound.iter().any(|slot| {
            let value = &operation["entries"][slot["entry"].as_u64().unwrap() as usize];
            value["origin"] == "synthesized" && value["value"].is_string()
        }));
        let missing = docs["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["available"] == false)
            .unwrap();
        assert_eq!(missing["reason"], "required input has no validated example");
    }
}

fn checked(command: &mut Command, retained: &Path) -> Output {
    let output = command.output().expect("required native tool is missing");
    assert!(
        output.status.success(),
        "native docs fixture retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn tools() -> PathBuf {
    std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        })
}

fn go_command() -> Command {
    let mut command =
        Command::new(std::env::var_os("SUSPECT_GO_BIN").unwrap_or_else(|| "go".into()));
    command.env("GOWORK", "off");
    if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
        command.env("GOTOOLCHAIN", toolchain);
    }
    command
}

fn sphinx(package: &Path, installed: Option<&Path>) -> Command {
    let mut command = Command::new(tools());
    command
        .args([
            "-m",
            "sphinx",
            "-W",
            "--keep-going",
            "-b",
            "html",
            "docs",
            "docs/_build/html",
        ])
        .current_dir(package);
    if let Some(installed) = installed {
        command.env("PYTHONPATH", installed);
    }
    command
}

fn html_coverage(package: &Path, language: &str, hostile_prose: bool) {
    let bindings: Value =
        serde_json::from_slice(&std::fs::read(package.join("docs/source-bindings.json")).unwrap())
            .unwrap();
    let html = std::fs::read_to_string(package.join("docs/_build/html/api.html")).unwrap();
    let coverage: Value = serde_json::from_slice(
        &std::fs::read(package.join("docs/_build/html/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        coverage["plannedSymbols"].as_u64().unwrap() as usize,
        bindings["symbols"].as_array().unwrap().len()
    );
    let names = coverage["documented"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for symbol in bindings["symbols"].as_array().unwrap() {
        assert!(
            names.contains(symbol["name"].as_str().unwrap()),
            "missing native docs for {symbol}"
        );
        assert!(
            html.contains(&format!(
                "id=\"{}\"",
                symbol["documentation"]["anchor"].as_str().unwrap()
            )),
            "missing browsable source anchor for {symbol}"
        );
        if !symbol["source"].is_null() {
            assert!(
                html.contains(
                    &symbol["source"]["pointer"]
                        .as_str()
                        .unwrap()
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                ),
                "source pointer lost from {language} docs"
            );
        }
    }
    if hostile_prose {
        assert!(
            html.contains("&lt;img"),
            "source prose must remain visible and escaped"
        );
    }
    assert!(
        !html.contains("<img src=x")
            && !html.contains("<script id=description-do-not-execute")
            && !html.contains("<img src=description-injection"),
        "source description became active HTML"
    );
    assert!(package.join("docs/_build/html/examples.html").is_file());
    assert!(package.join("docs/_build/html/operations.html").is_file());
}

fn install_python(package: &Path, root: &Path) -> (PathBuf, PathBuf) {
    checked(
        Command::new(tools()).args(["-c", "import sphinx; assert sphinx.__version__ == '8.2.3'"]),
        root,
    );
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(package),
        root,
    );
    let wheel = std::fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "whl"))
        .unwrap();
    let interpreter = std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3.11".into());
    checked(
        Command::new("uv")
            .args(["venv", "--python"])
            .arg(interpreter)
            .arg(root.join("venv")),
        root,
    );
    let python = root.join("venv/bin/python");
    checked(
        Command::new("uv")
            .args(["pip", "install", "--offline", "--python"])
            .arg(&python)
            .arg(wheel),
        root,
    );
    let site = checked(
        Command::new(&python).args(["-c", "import site; print(site.getsitepackages()[0])"]),
        root,
    );
    let site = PathBuf::from(String::from_utf8(site.stdout).unwrap().trim());
    (python, site)
}

#[test]
#[ignore = "requires pinned Python build/httpx/mypy/Sphinx tools, uv and a native Python interpreter"]
fn installed_python_docs_and_generated_samples_are_native_executable_artifacts() {
    let root = tempfile::tempdir().unwrap().keep();
    let contract = fixture(&root, canonical_document());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract, &selected, Default::default()).unwrap();
    suspect_codegen::write_files(&python(&plan), &root).unwrap();
    let package = root.join("python");
    let (python, site) = install_python(&package, &root);
    // Docs import the installed wheel, never the src tree. Sphinx itself runs
    // from the separately pinned tools environment.
    checked(&mut sphinx(&package, Some(&site)), &root);
    html_coverage(&package, "python", true);
    checked(&mut sphinx(&package, Some(&site)), &root); // unchanged incremental build
    std::fs::write(
        root.join("coverage_probe.py"),
        PYTHON_COVERAGE.replace("__IMPORT__", IMPORT),
    )
    .unwrap();
    checked(
        Command::new(&python)
            .arg(root.join("coverage_probe.py"))
            .arg(&package)
            .arg(&site),
        &root,
    );
    let sample = package.join("examples/validated.py");
    checked(
        Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join("mypy-cache"))
            .arg(&sample),
        &root,
    );
    let output = checked(Command::new(&python).arg(&sample), &root);
    assert!(String::from_utf8_lossy(&output.stdout).contains("validated-examples "));
    let fixture = RecordingFixture::new(8);
    for asynchronous in [false, true] {
        let mut command = Command::new(&python);
        command
            .arg(&sample)
            .arg("--server-url")
            .arg(&fixture.url)
            .args(["--token", "fixture-token"]);
        if asynchronous {
            command.arg("--async-client");
        }
        let output = checked(&mut command, &root);
        assert!(String::from_utf8_lossy(&output.stdout).contains("http-examples 4"));
    }
    fixture.verify(2);
    std::fs::write(root.join("bad_types.py"), format!("from {IMPORT} import Client\ndef bad(client: Client) -> None:\n    client.create_widget()\n    client.get_widget(widget_id=42)\n")).unwrap();
    let output = Command::new(tools())
        .args([
            "-m",
            "mypy",
            "--strict",
            "--python-version",
            "3.11",
            "--python-executable",
        ])
        .arg(&python)
        .arg("--cache-dir")
        .arg(root.join("bad-mypy-cache"))
        .arg(root.join("bad_types.py"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostics = String::from_utf8_lossy(&output.stdout);
    assert!(
        diagnostics.contains("[call-arg]") && diagnostics.contains("[arg-type]"),
        "{diagnostics}"
    );
    rejects_missing_documented_symbol(&package, Some(&site));
}

#[test]
#[ignore = "requires native Go and pinned Sphinx 8.2.3"]
fn go_docs_and_generated_samples_are_native_executable_artifacts() {
    let root = tempfile::tempdir().unwrap().keep();
    let contract = fixture(&root, canonical_document());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(contract, &selected, Default::default()).unwrap();
    suspect_codegen::write_files(&go(&plan), &root).unwrap();
    let package = root.join("go");
    checked(
        Command::new(tools()).args(["-c", "import sphinx; assert sphinx.__version__ == '8.2.3'"]),
        &root,
    );
    checked(
        go_command().args(["test", "./..."]).current_dir(&package),
        &root,
    );
    let native = checked(
        go_command()
            .args(["doc", "-all", "."])
            .current_dir(&package),
        &root,
    );
    let native = String::from_utf8(native.stdout).unwrap();
    assert!(
        native.contains("func (c *Client) CreateWidget(ctx context.Context")
            && native.contains("CreateWidgetStatus200")
    );
    checked(&mut sphinx(&package, None), &root);
    html_coverage(&package, "go", true);
    checked(&mut sphinx(&package, None), &root); // unchanged incremental build
    verify_go_type_bindings(&plan, &package, &root);
    let output = checked(
        go_command()
            .args(["run", "./examples/validated"])
            .current_dir(&package),
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("validated-examples "));
    let fixture = RecordingFixture::new(4);
    let output = checked(
        go_command()
            .args(["run", "./examples/validated", "-server-url"])
            .arg(&fixture.url)
            .args(["-token", "fixture-token"])
            .current_dir(&package),
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("http-examples 4"));
    fixture.verify(1);
    let consumer = root.join("negative-consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"), format!("module example.com/negative\n\ngo 1.23\nrequire {MODULE} v0.0.0\nreplace {MODULE} => ../go\n")).unwrap();
    std::fs::write(consumer.join("bad.go"), format!("package negative\nimport (\"context\"; sdk {MODULE:?})\nfunc bad(client *sdk.Client) {{ client.GetWidget(context.Background(), sdk.NewCreateWidgetInput(sdk.NewWidgetInput(\"x\"))) }}\n")).unwrap();
    let output = go_command()
        .args(["test", "./..."])
        .current_dir(&consumer)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "mistyped documented operation input unexpectedly compiled"
    );
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostics.contains("CreateWidgetInput") && diagnostics.contains("GetWidgetInput"),
        "{diagnostics}"
    );
    rejects_missing_documented_symbol(&package, None);
}

#[test]
#[ignore = "requires the tracked OpenRouter corpus and native Python/Go/Sphinx/mypy tools"]
fn tracked_five_operation_docs_and_example_artifacts_build_natively() {
    let source = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("OPENROUTER_WEB_ROOT")
                    .expect("set OPENROUTER_WEB_ROOT or SUSPECT_OPENROUTER_YAML"),
            )
            .join("projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&source);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let py_plan = python_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    let go_plan = go_http::plan_http(contract.clone(), &selected, Default::default()).unwrap();
    let py_files = python(&py_plan);
    let go_files = go(&go_plan);
    let py_examples = file(&py_files, &format!("python/src/{IMPORT}/examples.json"));
    assert_eq!(
        py_examples,
        file(&go_files, "go/examples.json"),
        "native targets must use the same validated examples and provenance"
    );
    let examples: Value = serde_json::from_str(py_examples).unwrap();
    let count: usize = examples["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["entries"].as_array().unwrap().len())
        .sum();
    assert!(
        count >= wanted.len(),
        "tracked source examples unexpectedly unavailable: {examples}"
    );
    for (files, language) in [(&py_files, "python"), (&go_files, "go")] {
        let (docs, _) = inventory(files, language, &contract);
        let ids = docs["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| op["operationId"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids, wanted.into_iter().collect());
        assert_eq!(docs["examples"].as_array().unwrap().len(), wanted.len());
    }
    let root = tempfile::tempdir().unwrap().keep();
    let go_root = root.join("go-fixture");
    suspect_codegen::write_files(&py_files, &root).unwrap();
    suspect_codegen::write_files(&go_files, &go_root).unwrap();
    let py_package = root.join("python");
    let go_package = go_root.join("go");
    let (python, site) = install_python(&py_package, &root);
    checked(&mut sphinx(&py_package, Some(&site)), &root);
    html_coverage(&py_package, "python", false);
    std::fs::write(
        root.join("coverage_probe.py"),
        PYTHON_COVERAGE.replace("__IMPORT__", IMPORT),
    )
    .unwrap();
    checked(
        Command::new(&python)
            .arg(root.join("coverage_probe.py"))
            .arg(&py_package)
            .arg(&site),
        &root,
    );
    checked(
        Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join("mypy-cache"))
            .arg(py_package.join("examples/validated.py")),
        &root,
    );
    let output = checked(
        Command::new(&python).arg(py_package.join("examples/validated.py")),
        &root,
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("validated-examples {count}")
    );
    checked(
        go_command()
            .args(["test", "./..."])
            .current_dir(&go_package),
        &root,
    );
    checked(&mut sphinx(&go_package, None), &root);
    html_coverage(&go_package, "go", false);
    verify_go_type_bindings(&go_plan, &go_package, &root);
    let output = checked(
        go_command()
            .args(["run", "./examples/validated"])
            .current_dir(&go_package),
        &root,
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("validated-examples {count}")
    );
}

fn verify_go_type_bindings(plan: &go_http::HttpPlan, package: &Path, root: &Path) {
    let consumer = root.join("binding-consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"), format!("module example.com/native-bindings\n\ngo 1.23\nrequire {MODULE} v0.0.0\nreplace {MODULE} => {:?}\n", package.to_string_lossy())).unwrap();
    let mut code = GO_BINDINGS.replace("__MODULE__", MODULE);
    code.push_str("\nfunc TestSourceBindings(t *testing.T) {\n");
    for symbol in plan.codecs().models().symbols() {
        let name = symbol.name();
        code.push_str(&format!("\tif reflect.TypeOf(sdk.Codecs.{name}) != reflect.TypeFor[sdk.Codec[sdk.{name}]]() {{ t.Fatal(\"codec model binding\", {name:?}) }}\n"));
    }
    for op in plan.operations() {
        code.push_str(&format!(
            "\tcheckMethod[sdk.{}, sdk.{}](t, {:?}, {})\n",
            op.input_type, op.success_type, op.method_name, op.optional_input
        ));
        let mut required = Vec::new();
        for parameter in op.parameters() {
            let model = &plan.symbols()[parameter.schema()];
            let ty = if parameter.setter_name.is_some() {
                format!("sdk.Optional[sdk.{model}]")
            } else {
                required.push(format!("reflect.TypeFor[sdk.{model}]()"));
                format!("sdk.{model}")
            };
            code.push_str(&format!(
                "\tcheckField[sdk.{}, {ty}](t, {:?})\n",
                op.input_type, parameter.field_name
            ));
        }
        if let Some(body) = op.body() {
            let model = &plan.symbols()[body
                .schema()
                .expect("this documentation fixture has one JSON body")];
            let ty = if body.setter_name.is_some() {
                format!("sdk.Optional[sdk.{model}]")
            } else {
                required.push(format!("reflect.TypeFor[sdk.{model}]()"));
                format!("sdk.{model}")
            };
            code.push_str(&format!(
                "\tcheckField[sdk.{}, {ty}](t, {:?})\n",
                op.input_type, body.field_name
            ));
        }
        code.push_str(&format!(
            "\tcheckConstructor[sdk.{}](t, sdk.{}, []reflect.Type{{{}}})\n",
            op.input_type,
            op.input_constructor,
            required.join(", ")
        ));
        for response in op.responses() {
            let model = &plan.symbols()[response
                .schema()
                .expect("this documentation fixture has JSON response codecs")];
            code.push_str(&format!(
                "\tcheckField[sdk.{}, sdk.{model}](t, \"Data\")\n",
                response.type_name
            ));
            if response.can_succeed() {
                code.push_str(&format!(
                    "\tvar _ sdk.{} = sdk.{}{{}}\n",
                    op.success_type, response.type_name
                ));
            }
            if response.can_fail() {
                code.push_str(&format!(
                    "\tvar _ sdk.{} = (*sdk.{})(nil)\n",
                    op.error_variant, response.type_name
                ));
            }
        }
    }
    code.push_str("}\n");
    std::fs::write(consumer.join("bindings_test.go"), code).unwrap();
    checked(
        go_command().args(["test", "./..."]).current_dir(&consumer),
        root,
    );
}

const GO_BINDINGS: &str = r#"package nativebindings
import (
    "context"
    "reflect"
    "testing"
    sdk "__MODULE__"
)

func checkField[Owner, Field any](t *testing.T, name string) {
    t.Helper()
    owner := reflect.TypeFor[Owner]()
    field, ok := owner.FieldByName(name)
    if !ok || field.Type != reflect.TypeFor[Field]() { t.Fatalf("%v.%s has the wrong source model type", owner, name) }
}

func checkMethod[Input, Result any](t *testing.T, name string, optional bool) {
    t.Helper()
    method, ok := reflect.TypeFor[*sdk.Client]().MethodByName(name)
    if !ok || method.Type.NumIn() != 3 || method.Type.NumOut() != 2 { t.Fatalf("missing native method shape: %s", name) }
    input := reflect.TypeFor[Input]()
    if optional { input = reflect.TypeFor[[]Input]() }
    if method.Type.IsVariadic() != optional || method.Type.In(1) != reflect.TypeFor[context.Context]() || method.Type.In(2) != input || method.Type.Out(0) != reflect.TypeFor[Result]() || method.Type.Out(1) != reflect.TypeFor[error]() { t.Fatalf("wrong native context/input/result binding: %s", name) }
}

func checkConstructor[Input any](t *testing.T, constructor any, required []reflect.Type) {
    t.Helper()
    signature := reflect.TypeOf(constructor)
    if signature.NumIn() != len(required) || signature.NumOut() != 1 || signature.Out(0) != reflect.TypeFor[Input]() { t.Fatal("wrong required input constructor") }
    for index, expected := range required {
        if signature.In(index) != expected { t.Fatal("wrong constructor argument binding", index) }
    }
}
"#;

fn rejects_missing_documented_symbol(package: &Path, installed: Option<&Path>) {
    // Mutate only documentation metadata in this retained private fixture. An
    // inventory that promises a symbol without a page must fail even when all
    // generated native code still compiles and Sphinx has an incremental cache.
    let path = package.join("docs/source-bindings.json");
    let mut bindings: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    bindings["symbols"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"DefinitelyMissingDocumentedSymbol"}));
    std::fs::write(path, bindings.to_string()).unwrap();
    let output = sphinx(package, installed).output().unwrap();
    assert!(
        !output.status.success(),
        "missing documentation coverage passed"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("DefinitelyMissingDocumentedSymbol"),
        "unexpected docs failure: {text}"
    );
}

const PYTHON_COVERAGE: &str = r#"import dataclasses
import importlib
import inspect
import json
from pathlib import Path
import sys
import typing

sdk = importlib.import_module('__IMPORT__')
package, site = map(Path, sys.argv[1:])
assert Path(sdk.__file__).is_relative_to(site), 'docs consumer must import the installed wheel'
coverage = json.loads((package / 'docs/_build/html/coverage.json').read_text())
bindings = json.loads((package / 'docs/source-bindings.json').read_text())
documented = set(coverage['documented'])
for name in ('Client', 'AsyncClient', 'SdkError', 'ApiError', 'Unset', 'UNSET', 'models', 'codecs'):
    assert '__IMPORT__.' + name in documented, name
for name in sdk.models.__all__:
    assert '__IMPORT__.models.' + name in documented, name
    model = getattr(sdk.models, name)
    if dataclasses.is_dataclass(model):
        for field in dataclasses.fields(model):
            if not field.name.startswith('_'):
                assert '__IMPORT__.models.' + name + '.' + field.name in documented, (name, field.name)
json_runtime = importlib.import_module('__IMPORT__.json_runtime')
for name in json_runtime.__all__:
    assert '__IMPORT__.json_runtime.' + name in documented, name
codec_annotations = typing.get_type_hints(sdk.codecs)
for symbol in bindings['symbols']:
    if symbol['kind'] == 'codec':
        model = getattr(sdk.models, symbol['related'][0].rsplit('.', 1)[1])
        assert typing.get_args(codec_annotations[symbol['qualname']]) == (model,), symbol
responses = importlib.import_module('__IMPORT__._client')
for operation in bindings['operations']:
    members = list(operation['parameters'])
    if operation['body'] is not None:
        members.append(operation['body'])
    for client in (sdk.Client, sdk.AsyncClient):
        method = getattr(client, operation['method'])
        full = '__IMPORT__.' + client.__name__ + '.' + operation['method']
        assert coverage['signatures'][full] == full + str(inspect.signature(method)), full
        parameters = inspect.signature(method).parameters
        assert set(parameters) == {'self', *(member['member'] for member in members)}, full
        annotations = typing.get_type_hints(method)
        assert annotations['return'] == getattr(responses, operation['success']), full
        assert inspect.iscoroutinefunction(method) == (client is sdk.AsyncClient), full
        for member in members:
            parameter = parameters[member['member']]
            model = getattr(sdk.models, member['model'])
            assert parameter.kind == inspect.Parameter.KEYWORD_ONLY, member
            assert parameter.default is (inspect.Parameter.empty if member['required'] else sdk.UNSET), member
            assert annotations[member['member']] == (model if member['required'] else typing.Union[model, sdk.Unset]), member
    for response in operation['responses']:
        concrete = getattr(responses, response['class'])
        model = getattr(sdk.models, response['model'])
        if 200 <= response['status'] < 300:
            annotations = typing.get_type_hints(concrete)
            assert annotations['data'] == model, response
            assert typing.get_args(annotations['status']) == (response['status'],), response
        else:
            assert any(typing.get_origin(base) is sdk.ApiError and typing.get_args(base) == (model,) for base in concrete.__orig_bases__), response
"#;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Request {
    method: String,
    target: String,
    authorization: String,
    body: Vec<u8>,
}

struct RecordingFixture {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Vec<Request>>>,
}

impl RecordingFixture {
    fn new(count: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            let mut seen = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(45);
            while seen.len() < count && !stopped.load(Ordering::Relaxed) {
                assert!(
                    Instant::now() < deadline,
                    "native sample fixture timed out: {seen:?}"
                );
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = receive(&mut stream);
                        // These wire responses and expectations are independent
                        // of HttpPlan, ExamplePlan, manifests and emitted code.
                        let body = if request.target.starts_with("/api/v1/widgets?") {
                            br#"{"items":[{"id":"w2","amount":1e-400,"payload":{"kind":"secure","vault":"v1"}}]}"#.as_slice()
                        } else {
                            br#"{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root"}}"#.as_slice()
                        };
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                        stream.write_all(body).unwrap();
                        seen.push(request);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
            seen
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }

    fn verify(mut self, repetitions: usize) {
        let mut actual = self.worker.take().unwrap().join().unwrap();
        let mut expected = Vec::new();
        for _ in 0..repetitions {
            for (method, target, body) in [
                ("GET", "/api/v1/widgets?tag=x&limit=1", ""),
                ("POST", "/api/v1/widgets", r#"{"name":"alpha"}"#),
                ("GET", "/api/v1/widgets/x", ""),
                ("PATCH", "/api/v1/widgets/x", "{}"),
            ] {
                expected.push(Request {
                    method: method.into(),
                    target: target.into(),
                    authorization: "Bearer fixture-token".into(),
                    body: body.as_bytes().into(),
                });
            }
        }
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "generated sample changed the independent canonical request contract"
        );
    }
}

impl Drop for RecordingFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn receive(stream: &mut TcpStream) -> Request {
    // Darwin accepted sockets inherit the listener's nonblocking flag. The
    // fixture uses bounded blocking reads after accepting each connection.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        assert!(bytes.len() < 65536, "fixture request headers too large");
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).unwrap();
        assert_ne!(count, 0, "request headers truncated");
        bytes.extend_from_slice(&buffer[..count]);
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let mut lines = headers.split("\r\n");
    let request = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
    let mut authorization = String::new();
    let mut length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("authorization") {
                authorization = value.trim().into();
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
    }
    assert!(length < 65536);
    while bytes.len() < end + length {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).unwrap();
        assert_ne!(count, 0, "request body truncated");
        bytes.extend_from_slice(&buffer[..count]);
    }
    Request {
        method: request[0].into(),
        target: request[1].into(),
        authorization,
        body: bytes[end..end + length].to_vec(),
    }
}
