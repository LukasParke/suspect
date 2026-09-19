//! Public cross-language M2 vertical gate: one canonical contract drives both
//! the canonical TypeScript and the native Rust HTTP packages through private
//! pack/install, native consumers, independently specified recording HTTP
//! fixtures and compiled native documentation. Opt-in: it requires native
//! Cargo, a pinned Node/npm toolchain and the cached TypeScript devDependency.

use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
};

use suspect_codegen::{
    rust_http::{
        HttpConfig as RustHttpConfig, HttpPlan as RustHttpPlan, PackageConfig as RustPackage,
        PlannedOperation as RustPlanned, emit_http as emit_rust, plan_http as plan_rust,
    },
    rust_models,
    typescript::{
        http::{HttpConfig as TsHttpConfig, HttpPlan as TsHttpPlan, plan_http as plan_ts},
        package::{PackageConfig as TsPackage, emit_http as emit_ts_package},
    },
    write_files,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const TS_PACKAGE: &str = "@suspect-fixtures/m2-canonical";
const TS_VERSION: &str = "0.1.0-m2.1";
const RUST_PACKAGE: &str = "m2-canonical-sdk";
const RUST_VERSION: &str = "0.0.0";
const WANTED: [&str; 4] = ["createWidget", "listWidgets", "getWidget", "updateWidget"];

fn checked(command: &mut Command, retained: &Path) -> Output {
    let output = command.output().expect("required native tool missing");
    assert!(
        output.status.success(),
        "native fixture retained at {}\ncommand: {command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn native_env(command: &mut Command) -> &mut Command {
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings")
}

/// The single canonical contract fixture; loaded from its real source path so
/// provenance pointers stay intact.
fn contract() -> Arc<Contract> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn selected(contract: &Arc<Contract>) -> Vec<SourceId> {
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| WANTED.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        WANTED.len(),
        "the four canonical operations must be present in the source contract"
    );
    selected
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(character.to_ascii_lowercase());
    }
    out
}

fn pascal(name: &str) -> String {
    let mut characters = name.chars();
    characters
        .next()
        .map(|c| c.to_ascii_uppercase())
        .into_iter()
        .collect::<String>()
        + characters.as_str()
}

fn rust_planned<'a>(plan: &'a RustHttpPlan, id: &str) -> &'a RustPlanned {
    plan.operations()
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap_or_else(|| panic!("operation {id} was not planned"))
}

/// Every consumer below is written against names located through public plan
/// symbols, never by parsing emitter text.
fn assert_planned_names(ts: &TsHttpPlan, rust: &RustHttpPlan) {
    for id in WANTED {
        let ts_operation = ts
            .operations()
            .iter()
            .find(|op| op.operation_id == id)
            .unwrap_or_else(|| panic!("operation {id} was not planned"));
        assert_eq!(ts_operation.function_name, id);
        assert_eq!(ts_operation.input_type, format!("{}Input", pascal(id)));
        assert_eq!(ts_operation.success_type, format!("{}Success", pascal(id)));
        assert_eq!(ts_operation.error_type, format!("{}ApiError", pascal(id)));

        let module = snake(id);
        let rust_operation = rust_planned(rust, id);
        assert_eq!(
            (
                rust_operation.module_name.as_str(),
                rust_operation.function_name.as_str(),
                rust_operation.input_type.as_str(),
                rust_operation.success_type.as_str(),
                rust_operation.error_type.as_str(),
                rust_operation.api_error_type.as_str(),
            ),
            (
                module.as_str(),
                module.as_str(),
                pascal(id).as_str(),
                format!("{}Success", pascal(id)).as_str(),
                format!("{}Error", pascal(id)).as_str(),
                format!("{}ApiError", pascal(id)).as_str(),
            ),
            "unexpected native naming for {id}"
        );
    }
}

/// The outer model for an operation's request body, located through public
/// source pointers: the body schema position itself and, when the schema is a
/// `$ref`, its resolved target. Only full `Model` symbols count; union branch
/// and other supporting roles must never be selected.
fn body_symbol(plan: &RustHttpPlan, operation: &RustPlanned) -> String {
    let schema = operation
        .source
        .child("requestBody")
        .child("content")
        .child("application/json")
        .child("schema");
    let matches = plan
        .codecs()
        .models()
        .symbols()
        .iter()
        .filter(|symbol| symbol.role() == rust_models::RepresentationRole::Model)
        .filter(|symbol| symbol.source() == &schema)
        .map(|symbol| symbol.name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "expected one body model at {schema:?}, found {matches:?}"
    );
    matches.into_iter().next().unwrap()
}

struct Toolchain {
    node: PathBuf,
    npm: PathBuf,
}

fn toolchain() -> Toolchain {
    let selected = std::env::var_os("SUSPECT_PACKAGE_NODE")
        .or_else(|| std::env::var_os("SUSPECT_DOCS_NODE"))
        .unwrap_or_else(|| "node".into());
    let output = Command::new(&selected)
        .args(["--print", "process.execPath"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pinned Node is required: select it with SUSPECT_PACKAGE_NODE"
    );
    let node = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let node_command = || {
        let mut command = Command::new(&node);
        command.env(
            "PATH",
            std::env::join_paths(std::iter::once(node.parent().unwrap().to_owned()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap(),
        );
        command
    };
    let version =
        String::from_utf8(node_command().arg("--version").output().unwrap().stdout).unwrap();
    assert_eq!(
        version.trim(),
        "v22.23.1",
        "select pinned Node through SUSPECT_PACKAGE_NODE"
    );
    let version = String::from_utf8(
        node_command()
            .arg(&npm)
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(version.trim(), "10.9.8");
    Toolchain { node, npm }
}

impl Toolchain {
    fn node(&self) -> Command {
        let mut command = Command::new(&self.node);
        command.env(
            "PATH",
            std::env::join_paths(
                std::iter::once(self.node.parent().unwrap().to_owned()).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ),
            )
            .unwrap(),
        );
        command
    }

    fn npm(&self, root: &Path) -> Command {
        let mut command = self.node();
        command.arg(&self.npm).current_dir(root);
        command
    }
}

fn ts_gate(plan: &TsHttpPlan, tools: &Toolchain, root: &Path) {
    let files = emit_ts_package(
        plan,
        &TsPackage {
            name: TS_PACKAGE.into(),
            version: TS_VERSION.into(),
        },
    )
    .unwrap();
    let generation = root.join("generation");
    write_files(&files, &generation).unwrap();
    let package = generation.join("typescript");
    checked(
        tools.npm(&package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        root,
    );
    checked(tools.npm(&package).args(["run", "build"]), root);
    let packed = checked(
        tools
            .npm(&package)
            .args(["pack", "--offline", "--ignore-scripts", "--json"]),
        root,
    );
    let packed: serde_json::Value = serde_json::from_slice(&packed.stdout).unwrap();
    assert_eq!(packed[0]["name"], TS_PACKAGE);
    assert_eq!(packed[0]["version"], TS_VERSION);
    let tarball = package.join(packed[0]["filename"].as_str().unwrap());
    let floor = root.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let pinned = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-floor");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(pinned.join(file), floor.join(file)).unwrap();
    }
    checked(
        tools.npm(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        root,
    );

    let javascript = include_str!("fixtures/m2/js_consumer.mjs").replace("__PACKAGE__", TS_PACKAGE);
    let typescript = include_str!("fixtures/m2/ts_consumer.ts").replace("__PACKAGE__", TS_PACKAGE);
    for (language, consumer) in [("javascript", javascript), ("typescript", typescript)] {
        let directory = root.join(language);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("package.json"),
            "{\"private\":true,\"type\":\"module\"}\n",
        )
        .unwrap();
        checked(
            tools
                .npm(&directory)
                .args([
                    "install",
                    "--offline",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                    "--omit=dev",
                ])
                .arg(&tarball),
            root,
        );
        let installed = directory.join("node_modules/@suspect-fixtures/m2-canonical");
        for file in &files {
            let relative = file.path.strip_prefix("typescript/").unwrap();
            if relative == "package-lock.json" {
                continue; // npm intentionally omits it.
            }
            assert_eq!(
                std::fs::read_to_string(installed.join(relative)).unwrap(),
                file.content,
                "installed provenance: {relative}"
            );
        }
        if language == "javascript" {
            std::fs::write(directory.join("consumer.mjs"), consumer).unwrap();
            checked(
                tools.node().current_dir(&directory).arg("consumer.mjs"),
                root,
            );
            let docs =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs");
            checked(tools.node().arg(docs).arg(&installed), root);
            let coverage: serde_json::Value = serde_json::from_slice(
                &std::fs::read(installed.join("docs/coverage.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                coverage["operations"].as_array().unwrap().len(),
                WANTED.len()
            );
        } else {
            std::fs::write(directory.join("consumer.ts"), consumer).unwrap();
            for tool_root in [&floor, &package] {
                checked(
                    tools
                        .node()
                        .arg(tool_root.join("node_modules/typescript/bin/tsc"))
                        .current_dir(&directory)
                        .args([
                            "--strict",
                            "--exactOptionalPropertyTypes",
                            "--noUncheckedIndexedAccess",
                            "--target",
                            "ES2022",
                            "--module",
                            "NodeNext",
                            "--moduleResolution",
                            "NodeNext",
                            "--outDir",
                            "out",
                            "--pretty",
                            "false",
                            "consumer.ts",
                        ]),
                    root,
                );
            }
            checked(
                tools.node().current_dir(&directory).arg("out/consumer.js"),
                root,
            );
        }
    }
}

/// Content digest of the packaged `.crate` bytes. Consumer verification and
/// native documentation run under a digest-keyed target so a same-version
/// stale Cargo archive can never be reused after the emitted sources — and
/// therefore the repacked archive — change.
fn digest(crate_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(crate_bytes);
    format!("{:x}", hasher.finalize())
}

/// Rustdoc invokes doctest compilers while holding build state; serialize the
/// doctest phase so concurrent gates cannot race on shared caches.
static DOC_LOCK: Mutex<()> = Mutex::new(());

fn rust_gate(plan: &RustHttpPlan, root: &Path, create_body: &str, update_body: &str) {
    let files = emit_rust(
        plan,
        &RustPackage {
            name: RUST_PACKAGE.into(),
            version: RUST_VERSION.into(),
        },
    )
    .unwrap();
    let generation = root.join("rust");
    write_files(&files, &generation).unwrap();
    let generation = generation.join("rust");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/m2-vertical");

    checked(
        native_env(
            Command::new("cargo")
                .args([
                    "package",
                    "--offline",
                    "--allow-dirty",
                    "--no-verify",
                    "--quiet",
                    "--manifest-path",
                ])
                .arg(generation.join("Cargo.toml"))
                .arg("--target-dir")
                .arg(&target),
        ),
        root,
    );
    let crate_path = target.join(format!("package/{RUST_PACKAGE}-{RUST_VERSION}.crate"));
    assert!(
        crate_path.is_file(),
        "packaged .crate missing at {}",
        crate_path.display()
    );
    let digest = digest(&std::fs::read(&crate_path).unwrap());
    let consumer_target = target.join(format!("consumer-{digest}"));
    let doc_target = target.join(format!("doc-{digest}"));
    let vendor = root.join("vendor");
    std::fs::create_dir_all(&vendor).unwrap();
    checked(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&crate_path)
            .arg("-C")
            .arg(&vendor),
        root,
    );
    let installed = vendor.join(format!("{RUST_PACKAGE}-{RUST_VERSION}"));
    assert!(installed.join("Cargo.toml").is_file());

    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        format!(
            "[package]\nname = \"m2-canonical-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n\n[dependencies]\nm2_canonical_sdk = {{ package = \"{RUST_PACKAGE}\", path = \"../vendor/{RUST_PACKAGE}-{RUST_VERSION}\", features = [\"http\"] }}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        consumer.join("src/lib.rs"),
        include_str!("fixtures/m2/rust_consumer.rs")
            .replace("__CREATE_BODY__", create_body)
            .replace("__UPDATE_BODY__", update_body),
    )
    .unwrap();
    checked(
        native_env(
            Command::new("cargo")
                .args(["test", "--offline", "--quiet", "--manifest-path"])
                .arg(consumer.join("Cargo.toml"))
                .arg("--target-dir")
                .arg(&consumer_target),
        ),
        root,
    );

    let _doctest = DOC_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    for (mode, extra) in [
        ("test", vec!["--doc", "--features", "http"]),
        ("doc", vec!["--no-deps", "--features", "http"]),
    ] {
        checked(
            native_env(
                Command::new("cargo")
                    .args([mode, "--offline", "--quiet", "--manifest-path"])
                    .arg(generation.join("Cargo.toml"))
                    .arg("--target-dir")
                    .arg(&doc_target)
                    .args(extra),
            ),
            root,
        );
    }
}

#[test]
#[ignore = "opt-in: requires native Cargo, tar, the pinned Node 22.23.1/npm 10.9.8 toolchain and cached TypeScript"]
fn m2_vertical_canonical_contract_drives_both_native_packages() {
    for tool in ["cargo", "tar", "node", "npm"] {
        assert!(
            Command::new(tool).arg("--version").output().is_ok(),
            "native tool {tool} is required by this opt-in gate"
        );
    }
    let contract = contract();
    let selected = selected(&contract);
    let ts_plan = plan_ts(contract.clone(), &selected, TsHttpConfig::default()).unwrap();
    let rust_plan = plan_rust(contract, &selected, RustHttpConfig::default()).unwrap();
    assert_planned_names(&ts_plan, &rust_plan);
    let create_body = body_symbol(
        &rust_plan,
        rust_plan
            .operations()
            .iter()
            .find(|op| op.operation_id == "createWidget")
            .unwrap(),
    );
    let update_body = body_symbol(
        &rust_plan,
        rust_plan
            .operations()
            .iter()
            .find(|op| op.operation_id == "updateWidget")
            .unwrap(),
    );

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.keep();
    let tools = toolchain();
    ts_gate(&ts_plan, &tools, &root);
    rust_gate(&rust_plan, &root, &create_body, &update_body);
    std::fs::remove_dir_all(&root).unwrap();
}
