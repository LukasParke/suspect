#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const INPUTS: &[&str] = &[
    "projects/docs/openapi/openapi.yaml",
    "openrouter-management.openapi.yaml",
    "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
    "packages/temporal/benchmarks.openapi.json",
];
const SPEC: &str =
    "{\"openapi\":\"3.1.0\",\"info\":{\"title\":\"fixture\",\"version\":\"1\"},\"paths\":{}}\n";

fn git(source: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .arg("-C")
        .arg(source)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn source_repo(root: &Path) -> PathBuf {
    let source = root.join("upstream");
    fs::create_dir(&source).unwrap();
    for input in INPUTS {
        let path = source.join(input);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, SPEC).unwrap();
    }
    git(&source, &["init", "--quiet"]);
    git(&source, &["add", "."]);
    git(
        &source,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    source
}

fn suspect_stub(root: &Path, body: &str) -> PathBuf {
    let path = root.join("suspect-fixture");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// These fixtures exercise orchestration; native SDK fidelity has separate real-tool gates.
fn canonical_stub(root: &Path) -> PathBuf {
    suspect_stub(
        root,
        r#"
case "$1" in
  --version) echo fixture-canonical; exit 0 ;;
  check|validate|lint)
    if [ "$FAIL_SOURCE_CHECKS" = 1 ]; then
      case "$1:$2" in validate:*/public/input.yaml|validate:*/management/input.yaml|lint:*/management/input.yaml)
        echo '[{"severity":"Error","code":"fixture-source-defect"}]'; exit 1 ;;
      esac
    fi
    echo '[]'; exit 0 ;;
  gen|codegen)
    kind="$1"; shift; profile=""; check=0
    while [ "$#" -gt 0 ]; do
      case "$1" in --out) out="$2"; shift;; --profile) profile="$2"; shift;; --check) check=1;; esac
      shift
    done
    if [ "$kind" = gen ]; then mkdir -p "$out"; printf document > "$out/README.md"; exit 0; fi
    case "$profile" in typescript-http) target=typescript;; rust-http) target=rust;; *) exit 22;; esac
    output="$out/$target"
    if [ "$check" = 1 ]; then test -f "$output/owner-output"; exit $?; fi
    mkdir -p "$output/src"
    printf generated > "$output/owner-output"
    if [ "$target" = rust ]; then printf generated > "$output/Cargo.toml"; printf generated > "$output/src/lib.rs"
    else printf '{"private":true}' > "$output/package.json"; printf generated > "$output/src/index.ts"; fi ;;
esac
"#,
    )
}

fn canonical_tools(root: &Path) -> std::ffi::OsString {
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    for name in ["cargo", "node", "npm"] {
        let script = tools.join(name);
        fs::write(&script, r#"#!/bin/sh
case "$1" in --version) echo fixture-native; exit 0;; esac
case "$0" in
  */cargo)
    arguments="$*"
    while [ "$#" -gt 0 ]; do if [ "$1" = --manifest-path ]; then manifest="$2"; shift; fi; shift; done
    test -f "$manifest" && test -f "${manifest%/Cargo.toml}/src/lib.rs" || exit 20
    if [ "$FAIL_NATIVE" = 1 ]; then
      case "$manifest" in */canonical-rust-keys/*) case "$arguments" in *reqwest-rustls*) echo 'native fixture failure' >&2; exit 6;; esac;; esac
    fi ;;
  */npm|*/node) test -f package.json || exit 21 ;;
esac
exit 0
"#).unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::join_paths(
        std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap()
}

fn assert_inventory(report: &serde_json::Value) {
    let commands = report["commands"].as_array().unwrap();
    let inventory = report["stage_inventory"].as_object().unwrap();
    assert_eq!(report["stage_count"], commands.len());
    for (input, expected) in inventory {
        let actual = commands
            .iter()
            .filter(|c| c["input"] == input.as_str())
            .map(|c| c["stage"].clone())
            .collect::<Vec<_>>();
        assert_eq!(expected, &serde_json::json!(actual));
    }
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn stage_inventory_is_available_without_source_or_native_tools() {
    for (flags, expected) in [
        (vec![], 58),
        (vec!["--generation-only"], 28),
        (vec!["--include-local-generated"], 70),
        (vec!["--include-local-generated", "--generation-only"], 40),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["openrouter", "--list-stages"])
            .args(flags)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let stages = plan["stage_inventory"].as_object().unwrap();
        assert_eq!(plan["stage_count"], expected);
        assert_eq!(
            stages
                .values()
                .map(|s| s.as_array().unwrap().len())
                .sum::<usize>(),
            expected
        );
        assert_eq!(
            stages["management"],
            serde_json::json!(["check", "validate", "lint", "docs-md"])
        );
        for values in stages.values() {
            let values = values.as_array().unwrap();
            assert_eq!(
                values.len(),
                values
                    .iter()
                    .map(|s| s.as_str().unwrap())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
            );
        }
    }
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn missing_required_snapshot_fails_even_in_report_only_mode() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    fs::remove_file(source.join(INPUTS[0])).unwrap();
    let binary = suspect_stub(temp.path(), "exit 0");
    let out = temp.path().join("target/openrouter");

    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&out)
        .arg("--report-only")
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains(INPUTS[0]),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !out.exists(),
        "preflight failure must not create an output tree"
    );
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn a_failing_command_is_recorded_and_only_report_mode_returns_success() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(
        temp.path(),
        "case \"$1\" in validate) echo 'fixture validation failure' >&2; exit 7;; esac\nexit 0",
    );
    let strict_out = temp.path().join("target/strict");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&strict_out)
        .arg("--generation-only")
        .output()
        .unwrap();

    assert!(!result.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(strict_out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["acceptance"], false);
    assert_eq!(report["mode"], "acceptance");
    let commands = report["commands"].as_array().unwrap();
    assert_eq!(
        commands.len(),
        28,
        "4 source stages per input plus 12 canonical generation/drift stages run"
    );
    let failures: Vec<_> = commands
        .iter()
        .filter(|c| c["stage"] == "validate")
        .collect();
    assert_eq!(failures.len(), 4);
    assert_eq!(failures[0]["exit_code"], 7);
    assert_eq!(failures[0]["success"], false);
    let log = failures[0]["stderr"].as_str().unwrap();
    assert_eq!(
        fs::read_to_string(strict_out.join(log)).unwrap(),
        "fixture validation failure\n"
    );

    let report_out = temp.path().join("target/baseline");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&report_out)
        .args(["--generation-only", "--report-only"])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("NOT acceptance"));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(report_out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["mode"], "report-only");
    assert_eq!(report["acceptance"], false);
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn requested_native_checks_fail_preflight_when_a_required_tool_is_missing() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(temp.path(), "exit 0");
    let tool_path = temp.path().join("tools");
    fs::create_dir(&tool_path).unwrap();
    let git_path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|file| file.is_file())
        .unwrap();
    std::os::unix::fs::symlink(git_path, tool_path.join("git")).unwrap();
    let out = temp.path().join("target/native");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&out)
        .arg("--report-only")
        .env("PATH", &tool_path)
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("required tool cargo"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!out.exists());
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn snapshots_preserve_bytes_and_distinguish_ignored_files_from_the_revision() {
    let temp = TempDir::new().unwrap();
    let original = source_repo(temp.path());
    let source = temp.path().join("upstream with spaces");
    fs::rename(original, &source).unwrap();
    fs::write(
        source.join(".gitignore"),
        "openrouter-openapi.*\nopenapi-assembled.json\n",
    )
    .unwrap();
    git(&source, &["add", ".gitignore"]);
    git(
        &source,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "ignore outputs",
        ],
    );
    fs::write(source.join(INPUTS[0]), "abc").unwrap();
    fs::create_dir_all(source.join("packages/sdk-generation")).unwrap();
    fs::write(source.join("openrouter-openapi.yaml"), "abc").unwrap();
    fs::write(source.join("openrouter-openapi.json"), "abc").unwrap();
    fs::write(
        source.join("packages/sdk-generation/openapi-assembled.json"),
        "abc",
    )
    .unwrap();
    let before = git(&source, &["status", "--porcelain"]).stdout;
    let binary = suspect_stub(
        temp.path(),
        r#"
case "$1" in
  --version) echo fixture-1 ;;
  gen|codegen)
    kind="$1"; shift
    profile=""
    while [ "$#" -gt 0 ]; do
      case "$1" in --out) out="$2"; shift;; --profile) profile="$2"; shift;; esac
      shift
    done
    case "$profile" in typescript-http) target=typescript;; rust-http) target=rust;; esac
    if [ "$kind" = gen ]; then mkdir -p "$out"; printf document > "$out/README.md"
    elif [ "$profile" = rust-http ]; then
      mkdir -p "$out/rust/src"
      printf generated > "$out/rust/Cargo.toml"
      printf generated > "$out/rust/src/lib.rs"
    else mkdir -p "$out/$target"; printf generated > "$out/$target/artifact.txt"; fi ;;
esac
"#,
    );
    let out = temp.path().join("target/first run");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&out)
        .args(["--generation-only", "--report-only"])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["acceptance"], false);
    assert_eq!(report["complete"], false);
    assert_eq!(report["all_commands_passed"], true);
    assert_eq!(report["inputs"].as_array().unwrap().len(), 4);
    assert_eq!(report["inputs"][0]["provenance"], "tracked-modified");
    assert_eq!(
        report["inputs"][0]["source_revision"],
        serde_json::Value::Null
    );
    assert_eq!(
        report["inputs"][0]["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(report["inputs"][1]["provenance"], "tracked-head");
    assert_eq!(
        report["inputs"][1]["source_revision"],
        report["repository_head"]
    );
    assert_eq!(fs::read(out.join("public/input.yaml")).unwrap(), b"abc");
    assert_eq!(report["commands"][3]["artifact_files"], 1);
    assert_eq!(report["commands"][3]["artifact_bytes"], 8);

    let expanded = temp.path().join("target/expanded");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&expanded)
        .args([
            "--include-local-generated",
            "--generation-only",
            "--report-only",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(expanded.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["inputs"].as_array().unwrap().len(), 7);
    assert_eq!(report["inputs"][4]["provenance"], "ignored-generated");
    assert_eq!(
        report["inputs"][4]["source_revision"],
        serde_json::Value::Null
    );
    assert_eq!(
        report["commands"].as_array().unwrap().len(),
        40,
        "7 expanded inputs x 4 source stages plus 12 canonical generation/drift stages"
    );
    assert_eq!(git(&source, &["status", "--porcelain"]).stdout, before);
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn output_paths_cannot_write_into_upstream_even_through_symlinks() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(temp.path(), "exit 0");
    let alias = temp.path().join("target");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(alias.join("acceptance"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must not be inside"));
    assert!(!source.join("acceptance").exists());
    assert!(git(&source, &["status", "--porcelain"]).stdout.is_empty());
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn input_symlinks_cannot_escape_the_source_repository() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let outside = temp.path().join("outside.json");
    fs::write(&outside, SPEC).unwrap();
    fs::remove_file(source.join(INPUTS[0])).unwrap();
    std::os::unix::fs::symlink(&outside, source.join(INPUTS[0])).unwrap();
    let out = temp.path().join("target/check");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg("/bin/true")
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("regular file inside source"));
    assert!(!out.exists());
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn acceptance_requires_canonical_native_checks_and_keeps_source_errors_red() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = canonical_stub(temp.path());
    let path = canonical_tools(temp.path());
    for mode in ["success", "native-failure", "source-failures"] {
        let out = temp.path().join(format!("target/{mode}"));
        let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["openrouter", "--source"])
            .arg(&source)
            .arg("--bin")
            .arg(&binary)
            .arg("--out")
            .arg(&out)
            .env("PATH", &path)
            .env(
                "FAIL_NATIVE",
                if mode == "native-failure" { "1" } else { "0" },
            )
            .env(
                "FAIL_SOURCE_CHECKS",
                if mode == "source-failures" { "1" } else { "0" },
            )
            .output()
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(
            result.status.success(),
            mode == "success",
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(report["complete"], true);
        assert_eq!(report["acceptance"], mode == "success");
        assert_eq!(report["schema_version"], 2);
        assert_inventory(&report);
        let commands = report["commands"].as_array().unwrap();
        assert_eq!(
            commands.len(),
            58,
            "16 source/document stages plus 42 canonical package stages"
        );
        for group in ["credits", "keys", "list"] {
            for suffix in [
                "generate",
                "drift",
                "models-build",
                "http-build",
                "reqwest-build",
                "docs",
                "pack",
            ] {
                let stage = format!("canonical-rust-{group}-{suffix}");
                let command = commands
                    .iter()
                    .find(|command| command["stage"] == stage)
                    .unwrap();
                assert_eq!(command["input"], "public", "{stage}");
                assert_eq!(
                    command["success"],
                    !(mode == "native-failure" && stage == "canonical-rust-keys-reqwest-build"),
                    "{stage}"
                );
            }
        }
        let generate = commands
            .iter()
            .find(|command| command["stage"] == "canonical-rust-credits-generate")
            .unwrap();
        let generate_args = generate["args"].as_array().unwrap();
        let profile = generate_args
            .iter()
            .position(|arg| arg.as_str() == Some("--profile"))
            .map(|index| generate_args[index + 1].as_str().unwrap());
        assert_eq!(profile, Some("rust-http"));
        assert!(
            generate["artifact_files"].as_u64().unwrap() > 0,
            "canonical Rust generation produces nonempty artifacts"
        );
        if mode == "native-failure" {
            let failures: Vec<_> = commands
                .iter()
                .filter(|command| command["success"] == false)
                .collect();
            assert_eq!(failures.len(), 1);
            assert!(failures.iter().all(|command| command["stage"]
                == "canonical-rust-keys-reqwest-build"
                && command["exit_code"] == 6));
        }
        if mode == "source-failures" {
            let failures = commands
                .iter()
                .filter(|c| c["success"] == false)
                .map(|c| {
                    assert_eq!(c["exit_code"], 1);
                    format!(
                        "{}/{}",
                        c["input"].as_str().unwrap(),
                        c["stage"].as_str().unwrap()
                    )
                })
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                failures,
                ["public/validate", "management/validate", "management/lint"]
                    .map(str::to_owned)
                    .into_iter()
                    .collect()
            );
            assert_eq!(report["all_commands_passed"], false);
        }
    }
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn canonical_groups_keep_independent_ownership_roots() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = canonical_stub(temp.path());
    let path = canonical_tools(temp.path());
    let out = temp.path().join("target/isolated-owned-output");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "--source"])
        .arg(&source)
        .arg("--bin")
        .arg(&binary)
        .arg("--out")
        .arg(&out)
        .env("PATH", path)
        .output()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
    let commands = report["commands"].as_array().unwrap();
    let failures = commands
        .iter()
        .filter(|command| command["success"] == false)
        .map(|command| {
            let log = out.join(command["stderr"].as_str().unwrap());
            format!(
                "{} {}: {}",
                command["input"],
                command["stage"],
                fs::read_to_string(log).unwrap()
            )
        })
        .collect::<Vec<_>>();
    assert!(result.status.success(), "{failures:#?}");
    assert_eq!(
        commands.len(),
        58,
        "16 source/document stages plus 42 canonical package stages"
    );
    assert_eq!(report["acceptance"], true);
    assert_inventory(&report);
    let mut owners = std::collections::BTreeSet::new();
    for group in ["credits", "keys", "list"] {
        let stage = format!("canonical-{group}-generate");
        let generated = commands
            .iter()
            .find(|command| command["stage"] == stage)
            .unwrap();
        assert_eq!(generated["input"], "public");
        let artifact = out.join(generated["artifacts"].as_str().unwrap());
        assert!(
            artifact.join("owner-output").is_file(),
            "canonical outputs survive all later owners"
        );
        owners.insert(artifact.parent().unwrap().to_owned());
        for suffix in [
            "drift",
            "install",
            "build",
            "pack",
            "consumer-install",
            "consumer-import",
        ] {
            let stage = format!("canonical-{group}-{suffix}");
            assert_eq!(
                commands
                    .iter()
                    .find(|command| command["stage"] == stage)
                    .unwrap()["success"],
                true
            );
        }
    }
    assert_eq!(owners.len(), 3);
    let mut rust_owners = std::collections::BTreeSet::new();
    for group in ["credits", "keys", "list"] {
        let stage = format!("canonical-rust-{group}-generate");
        let generated = commands
            .iter()
            .find(|command| command["stage"] == stage)
            .unwrap();
        assert_eq!(generated["input"], "public");
        let generate_args = generated["args"].as_array().unwrap();
        let profile = generate_args
            .iter()
            .position(|arg| arg.as_str() == Some("--profile"))
            .map(|index| generate_args[index + 1].as_str().unwrap());
        assert_eq!(profile, Some("rust-http"));
        let artifact = out.join(generated["artifacts"].as_str().unwrap());
        assert!(
            artifact.join("Cargo.toml").is_file() && artifact.join("src/lib.rs").is_file(),
            "canonical Rust outputs survive all later owners"
        );
        rust_owners.insert(artifact.parent().unwrap().to_owned());
        for suffix in [
            "drift",
            "models-build",
            "http-build",
            "reqwest-build",
            "docs",
            "pack",
        ] {
            let stage = format!("canonical-rust-{group}-{suffix}");
            let command = commands
                .iter()
                .find(|command| command["stage"] == stage)
                .unwrap();
            assert_eq!(command["success"], true, "{stage}");
        }
        let drift = commands
            .iter()
            .find(|command| command["stage"] == format!("canonical-rust-{group}-drift"))
            .unwrap();
        let drift_args = drift["args"].as_array().unwrap();
        assert_eq!(
            drift_args[drift_args.len() - 3..].to_vec(),
            ["--check", "--format", "json"],
            "drift reuses the generate argv plus check/format selectors"
        );
    }
    assert_eq!(rust_owners.len(), 3);
    assert_eq!(
        owners.union(&rust_owners).count(),
        6,
        "TypeScript and Rust canonical groups own independent output roots"
    );
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn mutating_binary_or_snapshots_fails_even_report_only() {
    for mode in ["binary", "snapshot", "source"] {
        let temp = TempDir::new().unwrap();
        let source = source_repo(temp.path());
        let binary = suspect_stub(
            temp.path(),
            r#"
if [ "$1" = --version ]; then echo fixture; exit 0; fi
if [ "$1" = validate ]; then
  case "$MUTATION" in
    binary) printf '\n# modified\n' >> "$0" ;;
    snapshot) printf '\nmodified\n' >> "$2" ;;
    source) printf '\nmodified\n' >> "$SOURCE_INPUT" ;;
  esac
fi
exit 0
"#,
        );
        let out = temp.path().join("target/mutation");
        let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["openrouter", "--source"])
            .arg(&source)
            .arg("--bin")
            .arg(&binary)
            .arg("--out")
            .arg(&out)
            .args(["--generation-only", "--report-only"])
            .env("MUTATION", mode)
            .env("SOURCE_INPUT", source.join(INPUTS[0]))
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "{mode} mutation cannot become an accepted baseline"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        let field = if mode == "binary" {
            "binary_unchanged"
        } else {
            "inputs_unchanged"
        };
        assert_eq!(report[field], false);
        assert_eq!(
            report["commands"].as_array().unwrap().len(),
            28,
            "mutation does not hide other stage results"
        );
    }
}

fn commit_fixture(source: &Path) -> String {
    git(source, &["add", "."]);
    git(
        source,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "comparison fixture",
        ],
    );
    String::from_utf8(git(source, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .into()
}

fn compare_command(source: &Path, binary: &Path, out: &Path, base: &str, head: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["openrouter", "compare", "--source"])
        .arg(source)
        .arg("--bin")
        .arg(binary)
        .arg("--out")
        .arg(out)
        .args(["--base", base, "--head", head])
        .output()
        .unwrap()
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn pinned_comparison_classifies_line_movement_new_and_resolved_defects() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    fs::write(
        source.join(INPUTS[0]),
        "{\n  \"inherited/~\": true,\n  \"resolved\": true\n}\n",
    )
    .unwrap();
    let base = commit_fixture(&source);
    fs::write(
        source.join(INPUTS[0]),
        "{\n\n\n  \"inherited/~\": true,\n  \"new\": true\n}\n",
    )
    .unwrap();
    let head = commit_fixture(&source);
    // A dirty checkout is deliberately not the oracle: only the pinned generated blobs are read.
    fs::write(source.join(INPUTS[0]), "uncommitted and not parsed\n").unwrap();
    let before = git(&source, &["status", "--porcelain"]).stdout;
    let binary = suspect_stub(
        temp.path(),
        r#"
if [ "$1" = --version ]; then echo comparison-fixture; exit 0; fi
case "$1:$2" in
  validate:*/projects/docs/openapi/openapi.yaml)
    case "$2" in */head/*) first=4; second=5; start1=6; end1=19; start2=29; end2=34;; *) first=2; second=3; start1=4; end1=17; start2=27; end2=37;; esac
    # Detect the base==head comparison without relying on directory names.
    content=$(cat "$2")
    case "$content" in *resolved*) first=2; second=3; start1=4; end1=17; start2=27; end2=37;; esac
    printf '[{"file":"%s","severity":"Error","code":"fixture-defect","message":"problem","line":%s,"col":3,"range":{"start":%s,"end":%s}},{"file":"%s","severity":"Error","code":"fixture-defect","message":"other problem","line":%s,"col":3,"range":{"start":%s,"end":%s}}]\n' "$2" "$first" "$start1" "$end1" "$2" "$second" "$start2" "$end2"
    exit 1 ;;
  *) printf '[]\n' ;;
esac
"#,
    );
    let out = temp.path().join("target/comparison");
    let result = compare_command(&source, &binary, &out, &base, &head);
    assert!(
        !result.status.success(),
        "new errors fail even though later lint commands succeed"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["base"]["revision"], base);
    assert_eq!(report["head"]["revision"], head);
    assert_eq!(
        report["findings"]["inherited"][0]["base"]["pointer"],
        "/inherited~1~0"
    );
    assert_eq!(report["findings"]["inherited"][0]["base"]["line"], 2);
    assert_eq!(report["findings"]["inherited"][0]["head"]["line"], 4);
    assert_eq!(report["findings"]["inherited"].as_array().unwrap().len(), 1);
    assert_eq!(report["findings"]["new"].as_array().unwrap().len(), 1);
    assert_eq!(report["findings"]["new"][0]["pointer"], "/new");
    assert_eq!(report["findings"]["resolved"].as_array().unwrap().len(), 1);
    assert_eq!(report["findings"]["resolved"][0]["pointer"], "/resolved");
    assert_eq!(
        report["head"]["commands"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["exit_code"],
        0
    );
    assert_eq!(report["input_changes"][0]["changed"], true);
    assert_eq!(report["inputs_unchanged"], true);
    assert_eq!(report["binary_unchanged"], true);
    assert_eq!(git(&source, &["status", "--porcelain"]).stdout, before);
    assert_eq!(
        fs::read_to_string(source.join(INPUTS[0])).unwrap(),
        "uncommitted and not parsed\n"
    );
    let first_report = fs::read(out.join("report.json")).unwrap();
    assert!(
        !compare_command(&source, &binary, &out, &base, &head)
            .status
            .success()
    );
    assert_eq!(
        fs::read(out.join("report.json")).unwrap(),
        first_report,
        "no prior run is clobbered"
    );

    let inherited_out = temp.path().join("target/inherited-only");
    let result = compare_command(&source, &binary, &inherited_out, &base, &base);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let inherited: serde_json::Value =
        serde_json::from_slice(&fs::read(inherited_out.join("report.json")).unwrap()).unwrap();
    assert_eq!(inherited["passed"], true);
    assert_eq!(inherited["acceptance"], false);
    assert_eq!(
        inherited["findings"]["inherited"].as_array().unwrap().len(),
        2
    );
    assert!(
        inherited["findings"]["resolved"]
            .as_array()
            .unwrap()
            .is_empty(),
        "inherited defects were not fixed"
    );
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn comparison_requires_baseline_commit_and_every_baseline_input() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let complete = String::from_utf8(git(&source, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    fs::remove_file(source.join(INPUTS[1])).unwrap();
    let incomplete = commit_fixture(&source);
    let binary = suspect_stub(temp.path(), "echo fixture; exit 0");
    for (name, baseline) in [
        ("missing-ref", "does-not-exist"),
        ("missing-input", incomplete.as_str()),
    ] {
        let out = temp.path().join(format!("target/{name}"));
        let result = compare_command(&source, &binary, &out, baseline, &complete);
        assert!(!result.status.success());
        assert!(
            !out.exists(),
            "missing baselines fail before creating output"
        );
    }
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn comparison_does_not_misclassify_unreadable_findings_as_resolved() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(
        temp.path(),
        r#"
if [ "$1" = --version ]; then echo fixture; exit 0; fi
case "$2" in
  */base/*)
    size=$(wc -c < "$2"); end=$((size - 1))
    printf '[{"file":"%s","severity":"Error","code":"fixture","message":"inherited defect","line":1,"col":1,"range":{"start":0,"end":%s}}]\n' "$2" "$end"
    exit 1 ;;
  *) echo malformed ;;
esac
"#,
    );
    let out = temp.path().join("target/malformed-findings");
    let result = compare_command(&source, &binary, &out, "HEAD", "HEAD");
    assert!(!result.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["complete"], false);
    for side in ["base", "head"] {
        let commands = report[side]["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 8, "every independent analysis still runs");
        assert!(
            commands
                .iter()
                .all(|command| command["error"].is_string() == (side == "head"))
        );
    }
    assert!(
        report["findings"]["resolved"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        report["unclassified_base"].as_array().unwrap().len(),
        8,
        "known baseline defects stay visible without a trustworthy candidate analysis"
    );
}

fn real_comparison_binary() -> PathBuf {
    let path = std::env::var_os("SUSPECT_COMPARE_BIN")
        .expect("set SUSPECT_COMPARE_BIN to the freshly built, pinned Suspect CLI");
    PathBuf::from(path)
        .canonicalize()
        .expect("required comparison binary exists")
}

fn comparison_report(out: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap()
}

#[test]
#[ignore = "requires a real CLI with reference allowlists and exact diagnostic ranges; set SUSPECT_COMPARE_BIN"]
fn comparison_real_denies_valid_absolute_unpinned_refs() {
    let binary = real_comparison_binary();
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let external = temp.path().join("external-schema.json");
    fs::write(&external, "{\"type\":\"string\"}\n").unwrap();
    let external_uri = suspect_source::Uri::from_path(&external).unwrap();
    let spec = serde_json::json!({"openapi":"3.1.0", "info":{"title":"fixture","version":"1"},
        "paths":{}, "components":{"schemas":{"External":{"$ref": external_uri.as_str()}}}});
    fs::write(
        source.join(INPUTS[0]),
        serde_json::to_vec_pretty(&spec).unwrap(),
    )
    .unwrap();
    let revision = commit_fixture(&source);
    let out = temp.path().join("target/unpinned-ref");
    let result = compare_command(&source, &binary, &out, &revision, &revision);
    assert!(!result.status.success());
    let report = comparison_report(&out);
    assert_eq!(
        report["complete"], false,
        "valid but unpinned bytes cannot influence a completed comparison: {report:#}"
    );
    assert!(
        report["unclassified_base"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "ref-outside-allowlist")
    );
    assert!(
        !report["findings"]["inherited"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["base"]["entry_path"] == INPUTS[0])
    );
    assert!(
        report["findings"]["resolved"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(&external).unwrap(),
        "{\"type\":\"string\"}\n"
    );
    assert_eq!(report["manifests_unchanged"], true);
}

#[test]
#[ignore = "requires a real CLI with reference allowlists and exact diagnostic ranges; set SUSPECT_COMPARE_BIN"]
fn comparison_real_early_abort_never_resolves_unchecked_semantics() {
    let binary = real_comparison_binary();
    for (case, baseline, candidate, code) in [
        (
            "unknown-family",
            "openapi: 9.0.0\ninfo: {title: fixture, version: '1'}\npaths: {}\n",
            "openapi: 9.0.0\ninfo: {title: fixture, version: '1'}\npaths: {}\ncomponents: {schemas: {Broken: {type: 3}}}\n",
            "validation-input",
        ),
        (
            "syntax",
            "openapi: 3.1.0\ninfo: {title: fixture, version: '1'}\npaths: {}\ncomponents: {schemas: {Broken: {type: 3}}}\n",
            "openapi: 3.1.0\ninfo: {title: fixture, version: '1'}\npaths: {}\ncomponents: [\n",
            "syntax-error",
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let source = source_repo(temp.path());
        fs::write(source.join(INPUTS[0]), baseline).unwrap();
        let base = commit_fixture(&source);
        fs::write(source.join(INPUTS[0]), candidate).unwrap();
        let head = commit_fixture(&source);
        let out = temp.path().join(format!("target/{case}"));
        let result = compare_command(&source, &binary, &out, &base, &head);
        assert!(!result.status.success(), "{case}");
        let report = comparison_report(&out);
        assert_eq!(report["complete"], false, "{report:#}");
        assert!(
            report["unclassified_head"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["code"] == code),
            "{report:#}"
        );
        assert!(report["unclassified_base"].as_array().unwrap().iter().any(|finding|
            finding["entry_path"] == INPUTS[0] && finding["severity"] == "Error"), "{report:#}");
        assert!(
            !report["findings"]["resolved"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["entry_path"] == INPUTS[0])
        );
    }
}

#[test]
#[ignore = "requires a real CLI with reference allowlists and exact diagnostic ranges; set SUSPECT_COMPARE_BIN"]
fn comparison_real_yaml_mapping_reordering_preserves_container_identity() {
    let binary = real_comparison_binary();
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let prefix = "openapi: 3.1.0\ninfo: {title: fixture, version: '1'}\npaths:\n  /x:\n    get:\n";
    fs::write(
        source.join(INPUTS[0]),
        format!("{prefix}      summary: Example\n      operationId: getExample\n"),
    )
    .unwrap();
    let base = commit_fixture(&source);
    fs::write(
        source.join(INPUTS[0]),
        format!("{prefix}      operationId: getExample\n      summary: Example\n"),
    )
    .unwrap();
    let head = commit_fixture(&source);
    let out = temp.path().join("target/reordered-yaml");
    let result = compare_command(&source, &binary, &out, &base, &head);
    let report = comparison_report(&out);
    assert!(
        result.status.success(),
        "{}\n{report:#}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(report["complete"], true);
    let inherited = report["findings"]["inherited"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["base"]["code"] == "oas-operation-missing-responses")
        .unwrap();
    assert_eq!(inherited["base"]["pointer"], "/paths/~1x/get");
    assert_eq!(inherited["head"]["pointer"], "/paths/~1x/get");
    assert!(report["findings"]["new"].as_array().unwrap().is_empty());
    assert!(
        report["findings"]["resolved"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "requires a real CLI with reference allowlists and exact diagnostic ranges; set SUSPECT_COMPARE_BIN"]
fn comparison_real_maps_findings_in_another_pinned_input() {
    let binary = real_comparison_binary();
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let public = serde_json::json!({"openapi":"3.1.0", "info":{"title":"fixture","version":"1"}, "paths":{},
        "components":{"schemas":{"Shared":{"$ref":"../../../openrouter-management.openapi.yaml#/components/schemas/Target"}}}});
    let management = serde_json::json!({"openapi":"3.1.0", "info":{"title":"fixture","version":"1"}, "paths":{},
        "components":{"schemas":{"Target":{"type":"number","exclusiveMinimum":true}}}});
    fs::write(
        source.join(INPUTS[0]),
        serde_json::to_vec_pretty(&public).unwrap(),
    )
    .unwrap();
    fs::write(
        source.join(INPUTS[1]),
        serde_json::to_vec_pretty(&management).unwrap(),
    )
    .unwrap();
    let revision = commit_fixture(&source);
    let out = temp.path().join("target/cross-input-ref");
    let result = compare_command(&source, &binary, &out, &revision, &revision);
    let report = comparison_report(&out);
    assert!(
        result.status.success(),
        "{}\n{report:#}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(report["complete"], true);
    assert!(
        report["base"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["entry_path"] == INPUTS[0]
                && finding["source_path"] == INPUTS[1]
                && finding["pointer"] == "/components/schemas/Target/exclusiveMinimum"
                && finding["severity"] == "Error"),
        "{report:#}"
    );
    assert_eq!(report["manifests_unchanged"], true);
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn comparison_requires_exact_ranges_even_when_exit_status_matches() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(
        temp.path(),
        r#"
if [ "$1" = --version ]; then echo fixture; exit 0; fi
printf '[{"file":"%s","severity":"Error","code":"fixture-defect","message":"no source range","line":1,"col":1,"range":null}]\n' "$2"
exit 1
"#,
    );
    let out = temp.path().join("target/missing-ranges");
    let result = compare_command(&source, &binary, &out, "HEAD", "HEAD");
    assert!(!result.status.success());
    let report = comparison_report(&out);
    assert_eq!(report["complete"], false);
    assert_eq!(report["unclassified_base"].as_array().unwrap().len(), 8);
    assert_eq!(report["unclassified_head"].as_array().unwrap().len(), 8);
    assert!(
        report["findings"]["inherited"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        report["findings"]["resolved"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[ignore = "requires the OpenRouter web checkout"]
#[test]
fn comparison_rejects_reference_manifest_mutation() {
    let temp = TempDir::new().unwrap();
    let source = source_repo(temp.path());
    let binary = suspect_stub(
        temp.path(),
        r#"
if [ "$1" = --version ]; then echo fixture; exit 0; fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --reference-allowlist ]; then printf '\n' >> "$2"; fi
  shift
done
echo '[]'
"#,
    );
    let out = temp.path().join("target/manifest-mutation");
    let result = compare_command(&source, &binary, &out, "HEAD", "HEAD");
    assert!(!result.status.success());
    let report = comparison_report(&out);
    assert_eq!(
        report["complete"], true,
        "all orchestration fixture commands finished successfully"
    );
    assert_eq!(report["manifests_unchanged"], false);
    assert_eq!(report["passed"], false);
}
