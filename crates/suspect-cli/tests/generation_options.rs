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

#[test]
fn suspect_yaml_codegen_defaults_fill_omitted_flags() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    // The codegen section supplies what the flags omit — the same surface
    // `suspect config` and the editor's completions advertise.
    std::fs::write(
        root.join(".suspect.yaml"),
        "codegen:\n  profile: typescript-http\n  package_version: 9.9.9\n  out: generated\n",
    )
    .expect("settings");
    let output = run(
        root,
        &[
            "codegen",
            "api.yaml",
            "--package-name",
            "defaults-test",
            "--operation-id",
            "getCurrentKey",
        ],
    );
    assert!(
        output.status.success(),
        "the settings must supply the profile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = std::fs::read_to_string(root.join("generated/typescript/package.json"))
        .expect("settings output root + generated package");
    assert!(
        version.contains("\"version\": \"9.9.9\""),
        "the settings version lands in the generated package: {version}"
    );
}

#[test]
fn suspect_yaml_supplies_package_name_operation_ids_and_import_names() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join(".suspect.yaml"),
        "codegen:\n  profile: python-http\n  package_name: settings-sdk\n  \
         package_version: 1.2.3\n  out: generated\n  operation_ids: [getCurrentKey]\n  \
         import_names:\n    python-http: settings_sdk\n",
    )
    .expect("settings");
    let output = run(root, &["codegen", "api.yaml"]);
    assert!(
        output.status.success(),
        "the settings must supply every required value: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = root.join("generated");
    let files: Vec<String> = walk(&generated)
        .into_iter()
        .map(|p| {
            p.strip_prefix(&generated)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        files.iter().any(|f| f.contains("settings_sdk")),
        "the configured import name names the package: {files:?}"
    );
    let all: String = walk(&generated)
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    assert!(all.contains("settings-sdk"), "the configured package name");
    // A flag still wins over the file.
    let output = run(
        root,
        &[
            "codegen",
            "api.yaml",
            "--package-name",
            "flag-sdk",
            "-o",
            "flagged",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let all: String = walk(&root.join("flagged"))
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    assert!(all.contains("flag-sdk") && !all.contains("settings-sdk"));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn a_yaml_defaults_file_reaches_the_generated_artifacts() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    std::fs::write(
        root.join("defaults.yaml"),
        "credential_env:\n  version: v1\n  schemes:\n    apiKey: OPENROUTER_API_KEY\n",
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
            "defaults.yaml",
            "-o",
            "sdk",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_text(&root.join("sdk")).contains("OPENROUTER_API_KEY"));

    // A typo is rejected with the key named, as for JSON.
    std::fs::write(root.join("typo.yml"), "credential_envs: {}\n").expect("typo");
    let output = run(
        root,
        &[
            "codegen",
            "api.yaml",
            "--profile",
            "python-http",
            "--package-name",
            "openrouter",
            "--defaults",
            "typo.yml",
            "-o",
            "sdk-typo",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid generation options typo.yml")
            && stderr.contains("credential_envs"),
        "{stderr}"
    );
}

#[test]
fn manifest_codegen_options_reach_the_generated_artifacts() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_api(root);
    let target = |options: Value| {
        let mut target = json!({
            "name": "sdk",
            "profile": "python-http",
            "package_name": "openrouter",
            "package_version": "0.1.0",
            "import_name": "openrouter",
            "out": "build/sdk",
            "options": options,
        });
        if let Some(extra) = target["options"].get("beside").cloned() {
            target["options"].as_object_mut().unwrap().remove("beside");
            target["credential_env"] = extra;
        }
        target
    };
    let manifest = |target: Value| {
        json!({
            "version": 1,
            "name": "opts",
            "entry": "api.yaml",
            "publish": {"output": "build/spec.yaml"},
            "codegen": [target]
        })
        .to_string()
    };
    let policy = serde_json::from_str::<Value>(POLICY).unwrap();
    std::fs::write(
        root.join("suspect.project.json"),
        manifest(target(json!({"credential_env": policy}))),
    )
    .expect("manifest");
    let output = run(root, &["project", "build", "--skip-tests"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output_text(&root.join("build/sdk")).contains("OPENROUTER_API_KEY"),
        "`options.credential_env` lands in the generated SDK"
    );

    // The same key inside and beside `options` is ambiguous.
    std::fs::write(
        root.join("suspect.project.json"),
        manifest(target(json!({"credential_env": policy, "beside": policy}))),
    )
    .expect("manifest");
    let output = run(root, &["project", "check"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`credential_env` is set both in `options` and beside it"),
        "{stderr}"
    );
}
