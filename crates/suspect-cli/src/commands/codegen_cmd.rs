//! `suspect codegen` — compile a spec to idiomatic TS/Rust/Go.

use std::path::Path;

/// Runs `suspect codegen`.
///
/// # Errors
/// Propagates spec-loading and emission failures; drift in `--check` mode
/// reports through the exit code instead.
pub fn codegen(
    spec: &Path,
    targets: &[String],
    out: &Path,
    zod: bool,
    check: bool,
) -> anyhow::Result<i32> {
    let dir = spec
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let ws = suspect_ref::WorkspaceBuilder::new().root(&dir).build()?;
    let name = spec
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    ws.load_all(name)?;
    let uri = suspect_source::Uri::from_path(spec)?;
    let ws = std::sync::Arc::new(ws);
    let ir_spec = suspect_ir::IrSpec::from_workspace(&ws, &uri).map_err(anyhow::Error::msg)?;

    let graph = suspect_codegen::build_graph(&ir_spec);
    let opts = suspect_codegen::EmitOptions { zod };
    let target_refs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let files =
        suspect_codegen::emit_all(&graph, &target_refs, &opts).map_err(anyhow::Error::msg)?;

    if check {
        if suspect_codegen::matches_disk(&files, out) {
            println!("codegen up to date ({} files)", files.len());
            return Ok(0);
        }
        eprintln!(
            "drift detected: {} of {} files would change",
            files
                .iter()
                .filter(|f| {
                    std::fs::read_to_string(out.join(&f.path)).is_ok_and(|disk| disk != f.content)
                })
                .count(),
            files.len()
        );
        return Ok(1);
    }

    suspect_codegen::write_files(&files, out).map_err(anyhow::Error::msg)?;
    for f in &files {
        println!("emitted  {}", f.path);
    }
    Ok(0)
}
