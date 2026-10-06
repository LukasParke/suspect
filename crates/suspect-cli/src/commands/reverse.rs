//! `suspect reverse`: detect spec drift from server source code.
//!
//! Extracts routes registered in server framework code (Rust axum/actix,
//! TypeScript Express, Go net/http & Gin) and cross-references them with
//! the spec: endpoints implemented but undocumented, documented but never
//! implemented, and same-path method mismatches.

use std::path::Path;

use suspect_ir::IrSpec;

/// Runs `suspect reverse`.
///
/// # Errors
/// Workspace loading, IR compilation, and report serialization failures.
pub fn reverse(source: &Path, spec: &Path, json: bool) -> anyhow::Result<i32> {
    let ws = super::workspace_for_entry(spec)?;
    let uri = suspect_source::Uri::from_path(spec)?;
    ws.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("spec document not loaded: {uri}"))?;
    let ir = IrSpec::from_workspace(&ws, &uri)
        .map_err(|e| anyhow::anyhow!("IR compilation failed: {e}"))?;
    let routes = suspect_reverse::extract_from_tree(source);
    if routes.is_empty() {
        eprintln!(
            "no routes extracted from {} (supported: Rust axum/actix, TypeScript Express, Go net/http, Gin)",
            source.display()
        );
        return Ok(2);
    }
    let report = suspect_reverse::cross_reference(&ir, &routes);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| anyhow::anyhow!("{e}"))?
        );
        return Ok(0);
    }
    println!(
        "{} routes extracted from {} against {}",
        report.extracted.len(),
        source.display(),
        spec.display()
    );
    for mismatch in &report.undocumented {
        println!("undocumented: {} — {}", mismatch.route, mismatch.message);
        if let Some(fragment) = &mismatch.spec_fragment {
            println!("    suggested spec fragment:\n    {fragment}");
        }
    }
    for mismatch in &report.spec_only {
        println!("spec-only: {} — {}", mismatch.route, mismatch.message);
    }
    for mismatch in &report.method_mismatches {
        println!("method mismatch: {} — {}", mismatch.route, mismatch.message);
    }
    println!(
        "reverse complete: {} undocumented, {} spec-only, {} method mismatches",
        report.undocumented.len(),
        report.spec_only.len(),
        report.method_mismatches.len()
    );
    Ok(0)
}
