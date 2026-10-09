//! Compares `IrSpec::from_file` (fast path + fallback) against
//! `IrSpec::from_workspace` (directory-scan path) for both a single-file
//! spec and a multi-file spec with cross-document `$ref`s.

use std::path::Path;

fn main() {
    for path in std::env::args().skip(1) {
        let path = Path::new(&path);
        let started = std::time::Instant::now();
        let fast = suspect_ir::IrSpec::from_file(path);
        let fast_ms = started.elapsed().as_secs_f64() * 1000.0;

        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let started = std::time::Instant::now();
        let ws = std::sync::Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(&root)
                .build()
                .expect("workspace"),
        );
        ws.load_all(&name).expect("load");
        let uri = ws.uris().first().expect("entry").clone();
        let full = suspect_ir::IrSpec::from_workspace(&ws, &uri);
        let full_ms = started.elapsed().as_secs_f64() * 1000.0;

        match (fast, full) {
            (Ok(fast), Ok(full)) => {
                println!(
                    "{}: fast {} ms ({}/{} ops, {} schemas) vs full {} ms ({}/{} ops, {} schemas)",
                    path.display(),
                    fast_ms,
                    fast.operations.len(),
                    full.operations.len(),
                    fast.schemas.len(),
                    full.schemas.len(),
                    full_ms,
                    full.operations.len(),
                    full.schemas.len(),
                );
                // Spot check: every full op id must exist in the fast IR.
                let missing: Vec<_> = full
                    .operations
                    .iter()
                    .filter(|op| {
                        op.id.as_ref().is_some_and(|id| {
                            !fast
                                .operations
                                .iter()
                                .any(|f| f.id.as_deref() == Some(id.as_str()))
                        })
                    })
                    .collect();
                println!(
                    "  ops missing from fast: {}  schemas missing: {}",
                    missing.len(),
                    full.schemas
                        .iter()
                        .filter(|s| !fast.schemas.iter().any(|f| f.name == s.name))
                        .count(),
                );
            }
            (Err(e), Ok(_)) => println!("{}: FAST PATH FAILED: {e}", path.display()),
            (Ok(_), Err(e)) => println!("{}: full path failed: {e}", path.display()),
            (Err(e), Err(e2)) => println!("{}: both failed: {e} / {e2}", path.display()),
        }
    }
}
