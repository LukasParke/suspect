//! Command implementations, one module per subcommand.

use suspect_ref::Workspace;
pub mod bench;
pub mod check;
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
    let dir = spec
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let ws = WorkspaceBuilder::new().root(&dir).build()?;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml") | Some("yml") | Some("json")
            )
        })
        .collect();
    entries.sort();
    for path in entries {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let _ = ws.load_all(name);
        }
    }
    Ok(std::sync::Arc::new(ws))
}
