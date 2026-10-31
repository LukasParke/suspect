//! Crate graph: `suspect-diag` stays a leaf, and only the front ends
//! (`suspect-lsp`, `suspect-cli`) depend on `suspect-check`.

use std::process::Command;

use serde_json::Value;

fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata is JSON")
}

/// `(package, dependency name, kind)` for every workspace package.
fn edges(metadata: &Value) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let name = package["name"].as_str().unwrap_or_default().to_owned();
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            out.push((
                name.clone(),
                dep["name"].as_str().unwrap_or_default().to_owned(),
                dep["kind"].as_str().unwrap_or("normal").to_owned(),
            ));
        }
    }
    out
}

#[test]
fn suspect_diag_is_a_leaf() {
    let edges = edges(&metadata());
    let deps: Vec<&str> = edges
        .iter()
        .filter(|(package, _, kind)| package == "suspect-diag" && kind == "normal")
        .map(|(_, dep, _)| dep.as_str())
        .collect();
    assert!(!deps.is_empty(), "suspect-diag not found");
    for dep in deps {
        assert!(
            matches!(dep, "suspect-source" | "serde" | "serde_json"),
            "suspect-diag must not depend on {dep}"
        );
    }
}

#[test]
fn only_front_ends_depend_on_suspect_check() {
    for (package, dep, _) in edges(&metadata()) {
        if dep == "suspect-check" {
            assert!(
                matches!(package.as_str(), "suspect-lsp" | "suspect-cli"),
                "{package} must not depend on suspect-check"
            );
        }
    }
}
