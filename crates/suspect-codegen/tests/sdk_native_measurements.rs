//! `cargo xtask verify-sdk`'s native build/import/codec/request observation entrypoint.
//!
//! The Python collector launches actual native tools against the runner's
//! immutable emitted packages and installed consumers. Libtest elapsed time is
//! never used as a native measurement.

use std::{path::PathBuf, process::Command};

fn collector_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/sdk-native-costs")
}

/// The runner-selected `SUSPECT_PYTHON_CURRENT_BIN`, else the manifest's
/// Python 3.11+ (`SUSPECT_PYTHON_BIN`, then `PATH`); the collector imports
/// `tomllib`, so older interpreters are refused with install guidance.
fn python() -> Result<PathBuf, String> {
    match std::env::var_os("SUSPECT_PYTHON_CURRENT_BIN") {
        Some(python) if !python.is_empty() => Ok(python.into()),
        _ => suspect_codegen::toolchain::gate("python"),
    }
}

#[test]
fn collector_unit_and_adversarial_evidence_checks() {
    let python = match python() {
        Ok(python) => python,
        Err(message) => {
            suspect_codegen::toolchain::skip("python", &message);
            return;
        }
    };
    let output = Command::new(python)
        .args([
            "-B",
            "-m",
            "unittest",
            "discover",
            "-s",
            ".",
            "-p",
            "test_*.py",
            "-v",
        ])
        .current_dir(collector_root())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("native collector host checks require installed Python 3.11+");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}{stderr}");
    assert!(
        output.status.success(),
        "native collector host checks failed"
    );
    assert!(
        stderr.contains("\nOK\n") && stderr.contains("Ran ") && !stderr.contains("Ran 0 tests"),
        "host evidence checks did not actually execute"
    );
}

#[test]
#[ignore = "requires verify-sdk's immutable twelve-target packages, installed native consumers, explicit identities, fresh output and all declared tools"]
fn native_build_import_codec_request_costs() {
    for key in [
        "SUSPECT_SDK_FULL_PACKAGES",
        "SUSPECT_SDK_FULL_NATIVE_ROOT",
        "SUSPECT_SDK_FULL_TARGETS",
        "SUSPECT_SDK_FULL_SOURCE_SHA256",
        "SUSPECT_SDK_FULL_BINARY_SHA256",
        "SUSPECT_SDK_FULL_MEASUREMENTS",
    ] {
        assert!(
            std::env::var_os(key).is_some_and(|value| !value.is_empty()),
            "{key} is required when native_build_import_codec_request_costs is explicitly run"
        );
    }
    let status = Command::new(python().unwrap_or_else(|skip| panic!("{skip}")))
        .arg("-B")
        .arg(collector_root().join("run.py"))
        .arg("collect")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .status()
        .expect("launch maintained native collector using the runner-selected installed Python");
    assert!(
        status.success(),
        "native observations incomplete; inspect SUSPECT_SDK_FULL_MEASUREMENTS/report.json and retained subprocess logs"
    );
}
