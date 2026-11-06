//! Native-gate plumbing shared by the Terraform acceptance files.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// A fresh retained evidence directory named `<prefix>XXXXXX`.
///
/// It is created under `SUSPECT_TERRAFORM_EVIDENCE` when that is set (so a
/// CI job can collect every gate's evidence from one place), else under the
/// system temporary directory. Every gate gets its own new directory, so
/// gates running concurrently in one test binary never share or mix
/// evidence with each other or with an earlier run.
pub fn evidence_dir(prefix: &str) -> PathBuf {
    let base = std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE")
        .filter(|value| !value.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&base).unwrap();
    let evidence = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&base)
        .unwrap()
        .keep();
    eprintln!("terraform evidence: {}", evidence.display());
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    evidence
}

/// A fresh retained native root under the approved temporary directory
/// (`$TMPDIR/suspect-terraform-native`, the `APPROVED` root the Python runners
/// assert), recorded
/// in `evidence/native-root.txt`.
pub fn native_root(prefix: &str, evidence: &Path) -> PathBuf {
    let approved = std::env::temp_dir().join("suspect-terraform-native");
    std::fs::create_dir_all(&approved).unwrap();
    let root = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&approved)
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    root
}

/// The native Go and Terraform executables the Python runners use, or
/// `None` after the toolchain skip message.
pub struct Native {
    pub go: PathBuf,
    pub terraform: PathBuf,
}

pub fn native() -> Option<Native> {
    let terraform = suspect_codegen::toolchain::require("terraform");
    let go = suspect_codegen::toolchain::require("go");
    Some(Native { go, terraform })
}

/// Runs `tools/<script>` with `--root`/`--evidence` (and `--fixtures`), using
/// the resolved toolchains, retains its output in the evidence directory,
/// and asserts it passed.
pub fn run_runner(
    native: &Native,
    script: &str,
    root: &Path,
    evidence: &Path,
    fixtures: Option<&Path>,
) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools")
        .join(script);
    let mut command = std::process::Command::new("python3");
    command
        .arg(&script)
        .arg("--root")
        .arg(root)
        .arg("--evidence")
        .arg(evidence);
    if let Some(fixtures) = fixtures {
        command.arg("--fixtures").arg(fixtures);
    }
    let output = command
        .env("SUSPECT_GO_BIN", &native.go)
        .env("SUSPECT_TERRAFORM_BIN", &native.terraform)
        .output()
        .unwrap();
    std::fs::write(evidence.join("runner.stdout.log"), &output.stdout).unwrap();
    std::fs::write(evidence.join("runner.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "retained {} and {}\n{}\n{}",
        root.display(),
        evidence.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The evidence root for one `#[ignore]`d Terraform gate. A
/// `SUSPECT_TERRAFORM_EVIDENCE` path that does not exist yet is used as is
/// (one gate run on its own, as the Terraform guide shows); otherwise each
/// gate gets a fresh directory under it (or the temp dir), so a whole-group
/// run such as the nightly job never reuses one. Holds a copy of the test
/// binary that produced it.
pub fn fresh_evidence(prefix: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        // Claim the exact path atomically: gates in one binary run
        // concurrently, so only the one whose `create_dir` succeeds owns it.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        if std::fs::create_dir(&path).is_ok() {
            std::fs::copy(
                std::env::current_exe().unwrap(),
                path.join("generator-test.bin"),
            )
            .unwrap();
            return path;
        }
    }
    evidence_dir(prefix)
}
