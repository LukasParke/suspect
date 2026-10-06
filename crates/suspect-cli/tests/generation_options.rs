//! Generation options reach every surface: `suspect codegen --defaults`,
//! the project manifest's codegen targets, and the parse's diagnostics for
//! unknown keys and typo'd styles.

use serde_json::{Value, json};
use std::process::Command;

const API: &str = r#"{
    "openapi": "3.1.2",
    "info": {"title": "Generation options", "version": "1"},
    "servers": [{"url": "https://source.example.test/v1"}],
    "components": {"securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}}},
    "security": [{"apiKey": []}],
    "paths": {"/key": {"get": {"operationId": "getCurrentKey",
        "responses": {"204": {"description": "Done"}}}}}
}"#;

const POLICY: &str = r#"{"version": "v1", "schemes": {"apiKey": "OPENROUTER_API_KEY"}}"#;

const SDK_DEFAULTS: &str =
    r#"{"version": "v1", "env_prefix": "EXAMPLE", "pagination": {"mode": "off"}}"#;

fn run(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .env("OPENROUTER_API_KEY", "GenerationOptionsCanary9183")
        .args(args)
        .output()
        .expect("run")
}

fn write_api(root: &std::path::Path) {
    std::fs::write(root.join("api.yaml"), API).expect("api");
}

fn output_text(root: &std::path::Path) -> String {
    let mut text = String::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("readdir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e != "lock") {
                text.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    text
}

#[test]
fn codegen_defaults_file_reaches_the_generated_artifacts() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join("defaults.json"),
        format!(r#"{{"credential_env": {POLICY}, "sdk_defaults": {SDK_DEFAULTS}}}"#),
    )
    .expect("defaults");
    let output = run(
        root,
        &[
            "codegen",
            "api.yaml",
            "--profile",
            "python-http",
            "--package-name",
            "openrouter",
            "--package-version",
            "0.1.0",
            "--import-name",
            "openrouter",
            "--defaults",
            "defaults.json",
            "-o",
            "sdk",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the defaults file must drive generation: {stdout} {stderr}"
    );
    assert!(
        !stdout.contains("GenerationOptionsCanary9183")
            && !stderr.contains("GenerationOptionsCanary9183"),
        "a secret env value never reaches command output"
    );
    let artifacts = output_text(&root.join("sdk"));
    assert!(
        artifacts.contains("OPENROUTER_API_KEY"),
        "the credential env mapping lands in the generated SDK"
    );
    // Explicit mappings win entirely, so env_prefix composes on its own
    // generation.
    std::fs::write(
        root.join("prefix-only.json"),
        format!(r#"{{"sdk_defaults": {SDK_DEFAULTS}}}"#),
    )
    .expect("prefix-only");
    let output = run(
        root,
        &[
            "codegen",
            "api.yaml",
            "--profile",
            "python-http",
            "--package-name",
            "openrouter",
            "--package-version",
            "0.1.0",
            "--import-name",
            "openrouter",
            "--defaults",
            "prefix-only.json",
            "-o",
            "sdk-prefix",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prefix_artifacts = output_text(&root.join("sdk-prefix"));
    assert!(
        prefix_artifacts.contains("EXAMPLE_API_KEY"),
        "the sdk_defaults env_prefix composes the automatic API-key variable"
    );
}

#[test]
fn project_manifest_generation_options_reach_the_generated_artifacts() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join("suspect.project.json"),
        json!({
            "version": 1,
            "name": "opts",
            "entry": "api.yaml",
            "publish": {"output": "build/spec.yaml"},
            "codegen": [{
                "name": "sdk",
                "profile": "python-http",
                "package_name": "openrouter",
                "package_version": "0.1.0",
                "import_name": "openrouter",
                "out": "build/sdk",
                "credential_env": serde_json::from_str::<Value>(POLICY).unwrap()
            }, {
                "name": "sdk-prefix",
                "profile": "python-http",
                "package_name": "openrouter",
                "package_version": "0.1.0",
                "import_name": "openrouter",
                "out": "build/sdk-prefix",
                "sdk_defaults": serde_json::from_str::<Value>(SDK_DEFAULTS).unwrap()
            }]
        })
        .to_string(),
    )
    .expect("manifest");
    let output = run(root, &["project", "build", "--skip-tests"]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the manifest target must drive generation: {stdout} {stderr}"
    );
    let artifacts = output_text(&root.join("build/sdk"));
    assert!(
        artifacts.contains("OPENROUTER_API_KEY"),
        "manifest credential_env lands in the generated SDK"
    );
    let prefix_artifacts = output_text(&root.join("build/sdk-prefix"));
    assert!(
        prefix_artifacts.contains("EXAMPLE_API_KEY"),
        "manifest sdk_defaults lands in the generated SDK"
    );
}

#[test]
fn an_unknown_codegen_target_key_is_an_error_not_a_silent_drop() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join("suspect.project.json"),
        json!({
            "version": 1,
            "name": "opts",
            "entry": "api.yaml",
            "publish": {"output": "build/spec.yaml"},
            "codegen": [{
                "profile": "python-http",
                "package_name": "openrouter",
                "package_version": "0.1.0",
                "compatibility_profile": ["legacy-binary-string-v1"]
            }]
        })
        .to_string(),
    )
    .expect("manifest");
    // `compatibility_profile` (singular) is not a key the builder reads.
    let output = run(root, &["project", "check"]);
    assert!(
        !output.status.success(),
        "an unknown key must fail the parse, not be silently dropped"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("unknown key `compatibility_profile`"),
        "the error names the dropped key: {stderr}"
    );
}

#[test]
fn a_docs_style_typo_fails_the_build_instead_of_falling_back() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join("suspect.project.json"),
        json!({
            "version": 1,
            "name": "opts",
            "entry": "api.yaml",
            "publish": {"output": "build/spec.yaml"},
            "docs": {"style": "SvelteKit", "output": "build/docs"}
        })
        .to_string(),
    )
    .expect("manifest");
    let output = run(root, &["project", "build", "--skip-tests"]);
    assert!(
        !output.status.success(),
        "a typo'd style must fail, not silently become HTML"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("docs style `SvelteKit`"),
        "the error names the unrecognized style: {stderr}"
    );
}

#[test]
fn project_init_declares_profiles_where_the_parser_reads_them() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("proj");
    let output = run(directory.path(), &["project", "init", "proj"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("suspect.project.json")).expect("manifest"),
    )
    .expect("json");
    assert!(
        manifest["publish"]["profiles"].is_object(),
        "the starter manifest declares profiles inside `publish`, where the parser reads them: {manifest}"
    );
    assert!(
        manifest.get("publish_profiles").is_none(),
        "the dead `publish_profiles` key is gone from the template"
    );
}
