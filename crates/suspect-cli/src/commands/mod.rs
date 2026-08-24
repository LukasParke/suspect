//! Command implementations, one module per subcommand.

use suspect_ref::Workspace;
pub mod bench;
pub mod check;
pub mod codegen_cmd;
pub mod fmt;
pub mod fuzz;
pub mod gateway;
pub mod generate;
pub mod http;
pub mod lint;
pub mod overlay;
pub mod replay;
pub mod stats;
pub mod test;
pub mod watch;

/// Loads every YAML/JSON document in `spec`'s directory into one
/// workspace. Arazzo source descriptions reference sibling files without
/// `$ref`, so a directory scan is the reliable way to have them loaded.
pub fn workspace_dir_all(spec: &std::path::Path) -> anyhow::Result<std::sync::Arc<Workspace>> {
    use suspect_ref::WorkspaceBuilder;

    // Canonicalize to an absolute path: relative inputs make
    // Path::parent() return empty/degenerate results.
    let spec = spec.canonicalize()?;
    // ...then use the spec's parent (or grandparent for workflow subdirs)
    // as the workspace root so all referenced documents resolve.
    let spec_dir = spec
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let root = spec_dir.parent().unwrap_or(&spec_dir).to_path_buf();

    let ws = WorkspaceBuilder::new().root(&root).build()?;

    // Collect unique yaml/yml/json paths from both root and spec_dir.
    let mut seen_paths = std::collections::HashSet::new();
    let mut all_files = Vec::new();
    for dir in [&root, &spec_dir] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if seen_paths.insert(p.clone())
                    && matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("yaml") | Some("yml") | Some("json")
                    )
                {
                    all_files.push(p);
                }
            }
        }
    }
    all_files.sort();

    if std::env::var_os("SUSPECT_TRACE").is_some() {
        for f in &all_files {
            eprintln!("[files] {}", f.display());
        }
    }

    for path in &all_files {
        // Path relative to the workspace root.
        let rel_str = path.strip_prefix(&root).and_then(|r| r.to_str());
        if let Some(rel_str) = rel_str {
            match ws.load_all(rel_str) {
                Ok(n) => eprintln!("[ws] loaded {rel_str} ({n} docs)"),
                Err(e) => eprintln!("[ws] FAILED {rel_str}: {e}"),
            }
        }
    }

    Ok(std::sync::Arc::new(ws))
}
