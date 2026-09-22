//! Execute the source-validated example artifacts shipped in both native packages.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
};
use suspect_codegen::{
    OutFile, rust_http,
    typescript::{http, package},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const PACKAGE: &str = "sdk-example-probe";
static NATIVE: Mutex<()> = Mutex::new(());

fn checked(command: &mut Command, root: &Path) -> Output {
    let output = command.output().expect("required native tool missing");
    assert!(
        output.status.success(),
        "retained {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn cargo(command: &mut Command) -> &mut Command {
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings")
}
fn content<'a>(files: &'a [OutFile], path: &str) -> &'a str {
    &files.iter().find(|file| file.path == path).unwrap().content
}

fn native(source: &Path, operation_ids: &[&str]) {
    let _lock = NATIVE.lock().unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap().keep();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(source).unwrap()).unwrap());
    let operations = contract
        .operations()
        .filter(|op| operation_ids.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), operation_ids.len());
    let ts = http::plan_http(contract.clone(), &operations, Default::default()).unwrap();
    let rs = rust_http::plan_http(contract, &operations, Default::default()).unwrap();
    let ts = package::emit_http(
        &ts,
        &package::PackageConfig {
            name: PACKAGE.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    let rs = rust_http::emit_http(
        &rs,
        &rust_http::PackageConfig {
            name: PACKAGE.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    assert_eq!(
        content(&ts, "typescript/examples.json"),
        content(&rs, "rust/examples.json"),
        "both languages must use the same values and source provenance"
    );
    let manifest: Value = serde_json::from_str(content(&ts, "typescript/examples.json")).unwrap();
    assert_eq!(manifest["format"], "suspect-sdk-examples-v1");
    let count: usize = manifest["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["entries"].as_array().unwrap().len())
        .sum();
    assert!(
        count >= operation_ids.len(),
        "each fixture operation needs executable example coverage: {manifest}"
    );
    suspect_codegen::write_files(&ts, &root).unwrap();
    suspect_codegen::write_files(&rs, &root.join("rs")).unwrap();

    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let output = checked(
        Command::new(&node).args(["--print", "process.execPath"]),
        &root,
    );
    let node = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let npm_command = |cwd: &Path| {
        let mut command = Command::new(&node);
        command.arg(&npm).current_dir(cwd).env(
            "PATH",
            std::env::join_paths(std::iter::once(node.parent().unwrap().to_owned()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap(),
        );
        command
    };
    let package = root.join("typescript");
    checked(
        npm_command(&package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &root,
    );
    checked(npm_command(&package).args(["run", "build"]), &root);
    let packed = checked(
        npm_command(&package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
        &root,
    );
    let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
    let tarball = package.join(packed[0]["filename"].as_str().unwrap());
    let consumer = root.join("js");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        "{\"private\":true,\"type\":\"module\"}",
    )
    .unwrap();
    checked(
        npm_command(&consumer)
            .args([
                "install",
                "--offline",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ])
            .arg(&tarball),
        &root,
    );
    let installed = consumer.join("node_modules").join(PACKAGE);
    assert_eq!(
        std::fs::read_to_string(installed.join("examples.json")).unwrap(),
        content(&ts, "typescript/examples.json")
    );
    let executed = checked(
        Command::new(&node).arg(installed.join("dist/examples/validated.js")),
        &root,
    );
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap().trim(),
        format!("validated-examples {count}")
    );

    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-sdk-examples");
    let generated = root.join("rs/rust");
    checked(
        cargo(
            Command::new("cargo")
                .args([
                    "package",
                    "--offline",
                    "--quiet",
                    "--allow-dirty",
                    "--no-verify",
                    "--manifest-path",
                ])
                .arg(generated.join("Cargo.toml"))
                .arg("--target-dir")
                .arg(&target),
        ),
        &root,
    );
    let archive = target.join(format!("package/{PACKAGE}-0.0.0.crate"));
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&archive).unwrap()));
    let vendor = root.join("vendor");
    std::fs::create_dir(&vendor).unwrap();
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&vendor),
        &root,
    );
    let installed = vendor.join(format!("{PACKAGE}-0.0.0"));
    assert_eq!(
        std::fs::read_to_string(installed.join("examples.json")).unwrap(),
        content(&rs, "rust/examples.json")
    );
    let executed = checked(
        cargo(
            Command::new("cargo")
                .args([
                    "run",
                    "--offline",
                    "--quiet",
                    "--features",
                    "http",
                    "--example",
                    "validated",
                    "--manifest-path",
                ])
                .arg(installed.join("Cargo.toml"))
                .arg("--target-dir")
                .arg(target.join(digest)),
        ),
        &root,
    );
    assert_eq!(
        String::from_utf8(executed.stdout).unwrap().trim(),
        format!("validated-examples {count}")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires native Cargo/Node/npm caches"]
fn shared_contract_examples_execute_from_both_installed_packages() {
    native(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
        &["createWidget", "updateWidget", "listWidgets", "getWidget"],
    );
}

#[test]
#[ignore = "requires native Cargo/Node/npm caches and the tracked OpenRouter corpus"]
fn tracked_operation_examples_execute_from_both_installed_packages() {
    let source =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"))
            .join("projects/docs/openapi/openapi.yaml");
    native(
        &source,
        &[
            "getCredits",
            "createKeys",
            "updateKeys",
            "listContainerFiles",
            "getContainerFile",
        ],
    );
}
