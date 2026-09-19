//! Opt-in native Rustdoc gate: real generated HTML must document the
//! http-manifest surface, and hostile source prose must render inert.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::rust_http::{HttpConfig, PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

/// Hostile prose: HTML injection, a fake Rust code fence, comment close,
/// an intra-doc link attempt and raw markup entities.
const PROSE: &str = "Literal </script><img src=x onerror=alert(1)> ```rust panic!(\"injected\") ``` */ {@link Missing} [Result] & < >";
const PACKAGE: &str = "docs-probe-local-sdk";

fn checked(command: &mut Command, retained: &Path) {
    let output = command.output().expect("required native tool missing");
    assert!(
        output.status.success(),
        "native fixture retained at {}\ncommand: {command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn spec() -> Value {
    json!({
        "openapi": "3.1.0", "info": {"title": "Docs probe", "version": "1"},
        "servers": [{"url": "https://example.test/api/v1"}], "security": [{"key": []}],
        "components": {
            "securitySchemes": {"key": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "Result": {"type": "object", "required": ["value"], "properties": {"value": {"type": "string"}}},
                "Client": {"type": "object", "required": ["message"], "properties": {"message": {"type": "string"}}}
            }
        },
        "paths": {"/items/{item-id}": {"get": {
            "operationId": "listResults", "description": PROSE,
            "parameters": [
                {"name": "item-id", "in": "path", "required": true, "schema": {"type": "string", "minLength": 1}},
                {"name": "tags", "in": "query", "explode": false, "schema": {"type": "array", "items": {"type": "string"}}}
            ],
            "responses": {
                "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Result"}}}},
                "418": {"description": "error", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Client"}}}}
            }
        }}}
    })
}

fn page(html: &Path, relative: &str) -> String {
    let path = html.join(relative);
    assert!(path.is_file(), "missing Rustdoc page {}", path.display());
    std::fs::read_to_string(path).unwrap()
}

fn link(source: &Value) -> String {
    format!(
        "Source: {}#{}",
        source["document"].as_str().unwrap(),
        source["pointer"].as_str().unwrap()
    )
}

fn model_page(html: &Path, name: &str) -> String {
    let paths = ["struct", "enum", "type"]
        .into_iter()
        .map(|kind| format!("models/{kind}.{name}.html"))
        .filter(|path| html.join(path).is_file())
        .collect::<Vec<_>>();
    assert_eq!(
        paths.len(),
        1,
        "missing/ambiguous native model page for {name}"
    );
    paths.into_iter().next().unwrap()
}

#[test]
#[ignore = "requires native Cargo and the pinned url crate cache"]
fn native_rustdoc_documents_manifest_surface_with_inert_hostile_prose() {
    // Retained on any failure so the exact unmodified emitted package can be
    // diagnosed; removed only after every check passes.
    let temporary = tempfile::tempdir().unwrap();
    let retained = temporary.keep();
    let source_path = retained.join("api.json");
    std::fs::write(&source_path, spec().to_string()).unwrap();
    let contract = contract(&source_path);
    let selected: Vec<_> = contract
        .operations()
        .map(|op| op.source().clone())
        .collect();
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    let generated = emit_http(
        &plan,
        &PackageConfig {
            name: PACKAGE.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&generated, &retained).unwrap();
    let package = retained.join("rust");

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(package.join("http-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["package"]["name"], PACKAGE);
    assert_eq!(manifest["package"]["crate"], "docs_probe_local_sdk");
    assert_eq!(manifest["releaseReady"], false);
    let operation = &manifest["operations"][0];
    assert_eq!(operation["operationId"], "listResults");
    assert_eq!(
        operation["descriptionText"], PROSE,
        "hostile source prose must reach the manifest verbatim"
    );
    // Independent native naming expectations, not emitter mirrors.
    assert_eq!(
        (
            operation["module"].as_str().unwrap(),
            operation["function"].as_str().unwrap(),
            operation["inputType"].as_str().unwrap(),
            operation["successType"].as_str().unwrap(),
            operation["errorType"].as_str().unwrap(),
            operation["apiErrorType"].as_str().unwrap()
        ),
        (
            "list_results",
            "list_results",
            "ListResults",
            "ListResultsSuccess",
            "ListResultsError",
            "ListResultsApiError"
        )
    );
    let model_names: Vec<&str> = manifest["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|model| {
            model["source"]["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/components/schemas/")
        })
        .filter_map(|model| model["name"].as_str())
        .collect();
    assert_eq!(
        model_names,
        ["Client", "Result"],
        "both source models stay usable under their own names"
    );

    // Generated docs must already render prose inert before Cargo runs.
    for document in [
        "src/lib.rs",
        "src/operations/mod.rs",
        "src/operations/list_results.rs",
        "README.md",
    ] {
        let text = std::fs::read_to_string(package.join(document)).unwrap();
        assert!(
            !text.contains("<script") && !text.contains("<img "),
            "unescaped HTML injection in {document}"
        );
    }

    let target =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-http-docs");
    for (mode, extra) in [
        ("test", vec!["--doc", "--features", "http"]),
        ("doc", vec!["--no-deps", "--features", "http"]),
    ] {
        checked(
            native_env(
                Command::new("cargo")
                    .args([mode, "--offline", "--quiet", "--manifest-path"])
                    .arg(package.join("Cargo.toml")),
            )
            .arg("--target-dir")
            .arg(&target)
            .args(&extra),
            &retained,
        );
    }

    let html = target.join("doc").join("docs_probe_local_sdk");
    let directory = "operations/list_results";
    // RUSTDOCFLAGS=-D warnings already turns broken generated intra-doc symbol
    // links into build failures; the pages below prove the rendered HTML.
    let module = page(&html, &format!("{directory}/index.html"));
    assert!(
        module.contains("&lt;/script&gt;") && module.contains("&lt;img src=x"),
        "hostile prose missing or not escaped in native module docs"
    );
    assert!(
        module.contains("&amp; &lt; &gt;"),
        "raw markup entities not escaped in HTML"
    );
    assert!(
        !module.contains("<img src=x"),
        "injected element survived into native Rustdoc HTML"
    );
    assert!(
        module.contains(&link(&operation["source"])),
        "native module docs lost the manifest source link"
    );

    let function = page(&html, &format!("{directory}/fn.list_results.html"));
    for symbol in [
        "struct.ListResults.html",
        "enum.ListResultsSuccess.html",
        "enum.ListResultsError.html",
    ] {
        assert!(
            function.contains(symbol),
            "fn page lost symbol link {symbol}"
        );
    }

    let input = page(&html, &format!("{directory}/struct.ListResults.html"));
    for parameter in operation["parameters"].as_array().unwrap() {
        let member = parameter["member"].as_str().unwrap();
        assert!(
            input.contains(member) && input.contains(&link(&parameter["source"])),
            "input page lost parameter {member} or its source link"
        );
    }

    for response in operation["responses"].as_array().unwrap() {
        let status = response["status"].as_u64().unwrap();
        let enum_page = if (200..300).contains(&status) {
            "enum.ListResultsSuccess.html"
        } else {
            "enum.ListResultsApiError.html"
        };
        let alternatives = page(&html, &format!("{directory}/{enum_page}"));
        let model = model_page(&html, response["model"].as_str().unwrap());
        assert!(
            alternatives.contains(&model) && alternatives.contains(&link(&response["source"])),
            "{enum_page} lost model link {model} or source link for status {status}"
        );
    }

    for model in manifest["models"].as_array().unwrap() {
        let name = model["name"].as_str().unwrap();
        let rendered = page(&html, &model_page(&html, name));
        assert!(
            rendered.contains(&link(&model["source"])),
            "model page {name} lost its manifest source link"
        );
    }

    std::fs::remove_dir_all(&retained).unwrap();
}
