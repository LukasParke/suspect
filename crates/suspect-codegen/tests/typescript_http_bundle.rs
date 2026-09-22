//! Pinned installed-package bundle and representative-path evidence for HTTP output.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::typescript::{
    http::{HttpConfig, plan_http},
    package::{PackageConfig, emit_http},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("native bundle toolchain required");
    assert!(
        output.status.success(),
        "{command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Tools {
    node: PathBuf,
    npm: PathBuf,
    path: OsString,
}
impl Tools {
    fn pinned() -> Self {
        let selected = std::env::var_os("SUSPECT_PACKAGE_NODE")
            .or_else(|| std::env::var_os("SUSPECT_DOCS_NODE"))
            .unwrap_or_else(|| "node".into());
        let node = PathBuf::from(
            String::from_utf8(
                checked(Command::new(selected).args(["--print", "process.execPath"])).stdout,
            )
            .unwrap()
            .trim(),
        );
        let bin = node.parent().unwrap();
        let npm = bin
            .parent()
            .unwrap()
            .join("lib/node_modules/npm/bin/npm-cli.js");
        let path = std::env::join_paths(std::iter::once(bin.to_owned()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let tools = Self { node, npm, path };
        assert_eq!(
            String::from_utf8(checked(tools.node().arg("--version")).stdout)
                .unwrap()
                .trim(),
            "v22.23.1"
        );
        assert_eq!(
            String::from_utf8(checked(tools.npm(Path::new(".")).arg("--version")).stdout)
                .unwrap()
                .trim(),
            "10.9.8"
        );
        tools
    }
    fn node(&self) -> Command {
        let mut command = Command::new(&self.node);
        command.env("PATH", &self.path);
        command
    }
    fn npm(&self, root: &Path) -> Command {
        let mut command = self.node();
        command.arg(&self.npm).current_dir(root);
        command
    }
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

#[test]
#[ignore = "requires tracked OpenRouter, pinned Node 22.23.1/npm, cached TypeScript and esbuild"]
fn installed_openrouter_http_bundle_records_size_shaking_and_runtime_cost() {
    let tools = Tools::pinned();
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/github/openrouter-web"));
    let contract = load(&checkout.join("projects/docs/openapi/openapi.yaml"));
    let selected = contract
        .operations()
        .filter(|operation| {
            matches!(
                operation.operation_id(),
                Some("getCredits" | "createKeys" | "updateKeys")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    let package = PackageConfig {
        name: "@suspect-fixtures/http-bundle".into(),
        version: "0.1.0-bundle.1".into(),
    };
    let files = emit_http(&plan, &package).unwrap();
    let metadata: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "typescript/package.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert!(
        metadata.get("dependencies").is_none(),
        "generated HTTP package must have zero runtime dependencies"
    );
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let generated = directory.path().join("typescript");
    checked(tools.npm(&generated).args([
        "ci",
        "--offline",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
    ]));
    checked(tools.npm(&generated).args(["run", "build"]));
    let packed: Value = serde_json::from_slice(
        &checked(
            tools
                .npm(&generated)
                .args(["pack", "--offline", "--ignore-scripts", "--json"]),
        )
        .stdout,
    )
    .unwrap();
    let tarball = generated.join(packed[0]["filename"].as_str().unwrap());
    let consumer = directory.path().join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        json!({"private":true,"type":"module"}).to_string(),
    )
    .unwrap();
    checked(tools.npm(&consumer).args([
        "install",
        "--offline",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        tarball.to_str().unwrap(),
    ]));
    let bundle_tool =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-http-bundle");
    checked(tools.npm(&bundle_tool).args([
        "ci",
        "--offline",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
    ]));
    let codec = format!(
        "{}Codec",
        plan.codecs()
            .models()
            .symbols()
            .iter()
            .find(|symbol| symbol.name() == "BadRequestResponse")
            .expect("tracked error response model")
            .name()
    );
    let report = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/typescript-http-bundle-report.json");
    checked(tools.node().arg(bundle_tool.join("build.mjs")).args([
        consumer.join("bundle").to_str().unwrap(),
        &package.name,
        &codec,
        report.to_str().unwrap(),
    ]));
    let evidence: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(evidence["node"], "22.23.1");
    assert_eq!(evidence["esbuild"], "0.28.2");
    for name in ["json", "codec", "operation", "allOperations"] {
        assert!(evidence["bundles"][name]["bytes"].as_u64().unwrap() > 0);
        assert!(evidence["bundles"][name]["gzipBytes"].as_u64().unwrap() > 0);
    }
    assert_eq!(
        evidence["bundles"]["operation"]["retainsCreateKeys"], false,
        "a getCredits-only bundle retained createKeys code"
    );
    assert_eq!(
        evidence["bundles"]["operation"]["retainsUpdateKeys"], false,
        "a getCredits-only bundle retained updateKeys code"
    );
    assert!(
        evidence["benchmark"]["json"]["nsPerOperation"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(
        evidence["benchmark"]["codec"]["nsPerOperation"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(
        evidence["benchmark"]["operation"]["nsPerOperation"]
            .as_f64()
            .unwrap()
            > 0.0
    );
}
