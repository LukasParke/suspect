//! sdk-full's native build/import/codec/request observation entrypoint.
//!
//! The Python collector launches actual native tools against the runner's
//! immutable emitted packages and installed consumers. Libtest elapsed time is
//! never used as a native measurement.

use std::{path::PathBuf, process::Command};

fn collector_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/sdk-native-costs")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("SUSPECT_PYTHON_CURRENT_BIN").unwrap_or_else(|| "python3".into())
}

#[test]
fn collector_unit_and_adversarial_evidence_checks() {
    let output = Command::new(python())
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
#[ignore = "requires sdk-full's immutable twelve-target packages, installed native consumers, explicit identities, fresh output and all declared tools"]
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
    let status = Command::new(python())
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
