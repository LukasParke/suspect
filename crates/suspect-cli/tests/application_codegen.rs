//! Real CLI routing for the two separate application output roots.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_source::Uri;

/// Stable ownership identities of the two application output roots.
const CLI_OWNER: &str = "suspect-cli-app:lifecycle-v1";
const MCP_OWNER: &str = "suspect-mcp-app:lifecycle-v1";

/// Copies one application fixture into its own subdirectory so both fixtures
/// can share a working root without either shadowing the other's entry name.
fn fixture(root: &Path, name: &str) -> PathBuf {
    let directory = root.join(name);
    std::fs::create_dir_all(&directory).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suspect-codegen/tests/fixtures")
        .join(name);
    for file in ["openapi.json", "mapping.json", "target.json"] {
        std::fs::copy(source.join(file), directory.join(file)).unwrap();
    }
    directory
}

fn output(root: &Path, args: &[&str], expected: i32) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{args:?}\n{text}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    text
}

fn run(root: &Path, args: &[&str], expected: i32) -> Value {
    serde_json::from_str(&output(root, args, expected)).unwrap()
}

/// One full application generation invocation against the named fixture.
fn generate(
    root: &Path,
    command: &str,
    name: &str,
    out: &str,
    extra: &[&str],
    expected: i32,
) -> Value {
    let spec = format!("{name}/openapi.json");
    let mapping = format!("{name}/mapping.json");
    let target = format!("{name}/target.json");
    let mut args = vec![
        command,
        spec.as_str(),
        "--mapping",
        mapping.as_str(),
        "--target-config",
        target.as_str(),
        "--out",
        out,
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    run(root, &args, expected)
}

fn artifacts(report: &Value) -> BTreeSet<&str> {
    report["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect()
}

fn operations(report: &Value) -> BTreeSet<&str> {
    report["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["operationId"].as_str().unwrap())
        .collect()
}

#[test]
fn explicit_cli_mapping_generates_one_owned_go_application_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    let spec = std::fs::read(root.join("api-cli-v1/openapi.json")).unwrap();

    // Nothing exists yet, so the read-only check reports pending work and
    // writes nothing at all.
    let absent = generate(
        root,
        "codegen-cli",
        "api-cli-v1",
        "generated",
        &["--check"],
        1,
    );
    assert_eq!(absent["status"], "drift");
    assert!(!root.join("generated").exists());

    let report = generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 0);
    assert_eq!(report["format"], "suspect.application.cli.generation.v1");
    assert_eq!(report["profile"], "suspect.application.cli.v1");
    assert_eq!(
        report["surfaceFormat"],
        "suspect.application.cli.surface.v1"
    );
    assert_eq!(report["owner"], CLI_OWNER);
    assert_eq!(report["status"], "generated");
    assert!(report["diagnostics"].as_array().unwrap().is_empty());

    let files = artifacts(&report);
    for path in [
        "go.mod",
        "go.sum",
        "cmd/widgetctl/main.go",
        "internal/cli/commands.go",
        "internal/cli/runtime.go",
        "internal/sdk/operations.go",
        "application-surface.json",
    ] {
        assert!(files.contains(path), "missing {path} in {files:?}");
    }
    // Every planned artifact is actually on disk after a write run.
    for path in &files {
        assert!(
            root.join("generated").join(path).is_file(),
            "unwritten {path}"
        );
    }
    // Each mapped operation is bound to a real canonical SDK operation.
    let mapped = operations(&report);
    for id in ["getWidget", "createWidget", "purgeWidgets"] {
        assert!(mapped.contains(id), "unmapped {id} in {mapped:?}");
    }

    // The input specification is never rewritten by generation.
    assert_eq!(
        std::fs::read(root.join("api-cli-v1/openapi.json")).unwrap(),
        spec
    );

    // A second run of either mode is current and byte-for-byte idle.
    let commands = root.join("generated/internal/cli/commands.go");
    let bytes = std::fs::read(&commands).unwrap();
    let timestamp = std::fs::metadata(&commands).unwrap().modified().unwrap();
    assert_eq!(
        generate(
            root,
            "codegen-cli",
            "api-cli-v1",
            "generated",
            &["--check"],
            0
        )["status"],
        "current"
    );
    assert_eq!(
        generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 0)["status"],
        "current"
    );
    assert_eq!(
        std::fs::metadata(&commands).unwrap().modified().unwrap(),
        timestamp
    );
    assert_eq!(std::fs::read(&commands).unwrap(), bytes);

    // Text output names the same classification without a JSON document.
    let text = output(
        root,
        &[
            "codegen-cli",
            "api-cli-v1/openapi.json",
            "--mapping",
            "api-cli-v1/mapping.json",
            "--target-config",
            "api-cli-v1/target.json",
            "--out",
            "generated",
        ],
        0,
    );
    assert!(text.contains("current"), "{text}");
    assert!(text.contains("suspect.application.cli.v1"), "{text}");
}

#[test]
fn a_user_edit_to_an_owned_application_artifact_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 0);

    let commands = root.join("generated/internal/cli/commands.go");
    let runtime = root.join("generated/internal/cli/runtime.go");
    let runtime_bytes = std::fs::read(&runtime).unwrap();
    let mut edited = std::fs::read(&commands).unwrap();
    edited.extend_from_slice(b"\n// retained user edit\n");
    std::fs::write(&commands, &edited).unwrap();

    let conflict = generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 1);
    assert_eq!(conflict["status"], "conflict");
    assert!(
        conflict["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "application-cli-artifact-conflict"),
        "{conflict:#?}"
    );
    // The user's bytes survive, and no sibling artifact was rewritten.
    assert_eq!(std::fs::read(&commands).unwrap(), edited);
    assert_eq!(std::fs::read(&runtime).unwrap(), runtime_bytes);

    // The read-only mode reports the same refusal and still writes nothing.
    let checked = generate(
        root,
        "codegen-cli",
        "api-cli-v1",
        "generated",
        &["--check"],
        1,
    );
    assert_eq!(checked["status"], "conflict");
    assert_eq!(std::fs::read(&commands).unwrap(), edited);
}

#[test]
fn check_mode_reports_pending_drift_without_touching_the_output_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 0);

    // Re-point the mapping at a different public command path: the planned
    // artifacts now differ from the owned ones on disk.
    let path = root.join("api-cli-v1/mapping.json");
    let mut mapping: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    mapping["commands"][0]["path"] = json!(["widgets", "read"]);
    std::fs::write(&path, mapping.to_string()).unwrap();

    let commands = root.join("generated/internal/cli/commands.go");
    let bytes = std::fs::read(&commands).unwrap();
    let timestamp = std::fs::metadata(&commands).unwrap().modified().unwrap();
    let drift = generate(
        root,
        "codegen-cli",
        "api-cli-v1",
        "generated",
        &["--check"],
        1,
    );
    assert_eq!(drift["status"], "drift");
    assert!(drift["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(std::fs::read(&commands).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&commands).unwrap().modified().unwrap(),
        timestamp
    );

    // The very same inputs on the write path resolve that drift.
    assert_eq!(
        generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 0)["status"],
        "generated"
    );
    assert!(std::fs::read(&commands).unwrap() != bytes);
}

#[test]
fn malformed_mapping_and_target_configuration_stay_located_before_any_write() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    std::fs::write(
        root.join("api-cli-v1/mapping.json"),
        "{\n  \"format\": \"suspect.application.cli.v1\",\n  \"commands\": [,]\n}\n",
    )
    .unwrap();
    let report = generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 1);
    assert_eq!(report["status"], "failed");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "application-cli-mapping-input"
                && finding["line"].as_u64().unwrap() == 3
                && finding["col"].as_u64().unwrap() > 0
                && finding["file"]
                    .as_str()
                    .is_some_and(|file| file.ends_with("mapping.json"))),
        "{report:#?}"
    );
    assert!(!root.join("generated").exists());

    fixture(root, "api-cli-v1");
    let path = root.join("api-cli-v1/target.json");
    let mut target: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    target["guess_binary_name"] = json!(true);
    std::fs::write(&path, serde_json::to_vec_pretty(&target).unwrap()).unwrap();
    let report = generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 1);
    assert_eq!(report["status"], "failed");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "application-cli-target-input"
                && finding["line"].as_u64().unwrap() > 0),
        "{report:#?}"
    );
    assert!(!root.join("generated").exists());
}

#[test]
fn an_unmapped_selector_is_refused_at_its_mapping_pointer_and_contract_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    let path = root.join("api-cli-v1/mapping.json");
    let mut mapping: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    mapping["commands"][0]["selector"] = json!("not-a-declared-operation");
    std::fs::write(&path, mapping.to_string()).unwrap();
    let report = generate(root, "codegen-cli", "api-cli-v1", "generated", &[], 1);
    assert_eq!(report["status"], "failed");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["mappingPointer"]
                .as_str()
                .is_some_and(|pointer| pointer.contains("/commands/0"))
                && finding["file"]
                    .as_str()
                    .is_some_and(|file| file.ends_with("openapi.json"))),
        "{report:#?}"
    );
    assert!(!root.join("generated").exists());
}

#[test]
fn explicit_mcp_mapping_generates_one_owned_typescript_server_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "mcp-v1");

    let absent = generate(root, "codegen-mcp", "mcp-v1", "server", &["--check"], 1);
    assert_eq!(absent["status"], "drift");
    assert!(!root.join("server").exists());

    let report = generate(root, "codegen-mcp", "mcp-v1", "server", &[], 0);
    assert_eq!(report["format"], "suspect.application.mcp.generation.v1");
    assert_eq!(report["profile"], "suspect.application.mcp.v1");
    assert_eq!(
        report["surfaceFormat"],
        "suspect.application.mcp.surface.v1"
    );
    assert_eq!(report["owner"], MCP_OWNER);
    assert_eq!(report["status"], "generated");

    let files = artifacts(&report);
    for path in [
        "package.json",
        "package-lock.json",
        "tsconfig.json",
        "server/main.ts",
        "server/runtime.ts",
        "application-surface.json",
    ] {
        assert!(files.contains(path), "missing {path} in {files:?}");
    }
    assert!(
        files.iter().any(|path| path.starts_with("typescript/")),
        "{files:?}"
    );
    for path in &files {
        assert!(root.join("server").join(path).is_file(), "unwritten {path}");
    }
    let mapped = operations(&report);
    for id in ["getWidget", "createWidget", "purgeWidgets"] {
        assert!(mapped.contains(id), "unmapped {id} in {mapped:?}");
    }

    assert_eq!(
        generate(root, "codegen-mcp", "mcp-v1", "server", &["--check"], 0)["status"],
        "current"
    );
    assert_eq!(
        generate(root, "codegen-mcp", "mcp-v1", "server", &[], 0)["status"],
        "current"
    );
}

#[test]
fn the_two_application_roots_never_take_over_each_others_owned_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    fixture(root, "mcp-v1");
    generate(root, "codegen-cli", "api-cli-v1", "shared", &[], 0);
    let surface = root.join("shared/application-surface.json");
    let owned = std::fs::read(&surface).unwrap();

    // The MCP target plans the same surface manifest path inside a root the
    // CLI target already owns: neither check nor write may claim it.
    let checked = generate(root, "codegen-mcp", "mcp-v1", "shared", &["--check"], 1);
    assert_eq!(checked["status"], "conflict");
    let conflict = generate(root, "codegen-mcp", "mcp-v1", "shared", &[], 1);
    assert_eq!(conflict["status"], "conflict");
    assert!(
        conflict["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |finding| finding["code"] == "application-mcp-artifact-conflict"
                    && finding["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(CLI_OWNER))
            ),
        "{conflict:#?}"
    );
    assert_eq!(std::fs::read(&surface).unwrap(), owned);
    assert!(!root.join("shared/server/main.ts").exists());

    // The CLI target's own root is untouched by the refusal.
    assert_eq!(
        generate(root, "codegen-cli", "api-cli-v1", "shared", &["--check"], 0)["status"],
        "current"
    );
}

#[test]
fn application_profiles_are_discoverable_separately_from_sdk_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();

    // The existing SDK inventory is unchanged, and never names an application.
    let sdk = run(root, &["codegen-profiles", "--format", "json"], 0);
    assert_eq!(sdk["format"], "suspect.sdk.profiles.v1");
    assert_eq!(sdk["profiles"].as_array().unwrap().len(), 12);
    assert!(
        sdk["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|profile| !profile["profile"]
                .as_str()
                .unwrap()
                .contains("suspect.application")),
        "{sdk:#?}"
    );

    let applications = run(
        root,
        &[
            "codegen-profiles",
            "--kind",
            "applications",
            "--format",
            "json",
        ],
        0,
    );
    assert_eq!(applications["format"], "suspect.application.profiles.v1");
    let listed = applications["profiles"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    let cli = listed
        .iter()
        .find(|profile| profile["command"] == "codegen-cli")
        .unwrap();
    assert_eq!(cli["profile"], "suspect.application.cli.v1");
    assert_eq!(cli["surfaceFormat"], "suspect.application.cli.surface.v1");
    assert_eq!(cli["owner"], CLI_OWNER);
    // The mapping profile is reported exactly once, under `profile`.
    assert!(cli["mapping"].is_null(), "{cli}");
    let mcp = listed
        .iter()
        .find(|profile| profile["command"] == "codegen-mcp")
        .unwrap();
    assert_eq!(mcp["profile"], "suspect.application.mcp.v1");
    assert_eq!(mcp["surfaceFormat"], "suspect.application.mcp.surface.v1");
    assert_eq!(mcp["owner"], MCP_OWNER);

    let text = output(root, &["codegen-profiles", "--kind", "applications"], 0);
    assert!(text.contains(CLI_OWNER), "{text}");
    assert!(text.contains(MCP_OWNER), "{text}");
}

#[test]
fn a_verified_pin_manifest_drives_application_generation_with_no_entry_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root, "api-cli-v1");
    let spec = root.join("api-cli-v1/openapi.json");
    let entry = Uri::from_path(&spec).unwrap().to_string();
    let bytes = std::fs::read(&spec).unwrap();
    std::fs::write(
        root.join("pins.json"),
        json!({"manifest_version":1,"entry":entry,"resources":[
            json!({"requested_uri":entry,"effective_uri":entry,"digest":format!("sha256-{:x}",Sha256::digest(&bytes)),
                "media_type":"application/json","via":"local","redirects":[],"retrieved_at":"2026-09-21T00:00:00Z","attempts":0})]})
        .to_string(),
    )
    .unwrap();
    let acquisition = run(
        root,
        &[
            "acquire",
            "pins.json",
            "--cache-dir",
            "cache",
            "--format",
            "json",
        ],
        0,
    );
    assert_eq!(acquisition["networkRequests"], 0);
    std::fs::remove_file(&spec).unwrap();

    let args = [
        "codegen-cli",
        "--pins",
        "pins.json",
        "--cache-dir",
        "cache",
        "--mapping",
        "api-cli-v1/mapping.json",
        "--target-config",
        "api-cli-v1/target.json",
        "--out",
        "generated",
        "--format",
        "json",
    ];
    let report = run(root, &args, 0);
    assert_eq!(report["status"], "generated");
    assert_eq!(report["source"], entry);
    assert!(
        report["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|operation| operation["document"] == entry),
        "{report:#?}"
    );

    // A corrupted cache blob is refused by digest, leaving the owned root as
    // the last verified generation left it.
    let commands = root.join("generated/internal/cli/commands.go");
    let generated = std::fs::read(&commands).unwrap();
    let cached = root.join(acquisition["documents"][0]["cachePath"].as_str().unwrap());
    // Acquired blobs are correctly read-only. Replace only this private test
    // blob to verify digest refusal without changing production cache policy.
    std::fs::remove_file(&cached).unwrap();
    std::fs::write(cached, b"corrupted cache fixture").unwrap();
    let rejected = run(root, &args, 1);
    assert_eq!(rejected["status"], "failed");
    assert!(
        rejected["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "pin-digest-mismatch"),
        "{rejected:#?}"
    );
    assert_eq!(std::fs::read(&commands).unwrap(), generated);
}
