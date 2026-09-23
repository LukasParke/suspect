//! `suspect overlay-dry-run` — report overlay action target matches
//! without applying.

use std::path::PathBuf;

/// One overlay dry-run.
#[derive(Debug, clap::Args)]
pub struct OverlayDryRunArgs {
    /// The Overlay document.
    #[arg(required = true)]
    pub overlay: PathBuf,
    /// The target document the overlay applies to.
    #[arg(required = true)]
    pub target: PathBuf,
}

/// Reports matches per action without modifying the target.
///
/// # Errors
/// Propagates IO and parsing failures.
pub fn overlay_dry_run(args: &OverlayDryRunArgs) -> anyhow::Result<i32> {
    let overlay_text = std::fs::read_to_string(&args.overlay)?;
    let target_text = std::fs::read_to_string(&args.target)?;
    let overlay_uri = suspect_source::Uri::from_path(
        &args
            .overlay
            .canonicalize()
            .unwrap_or_else(|_| args.overlay.clone()),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let target_uri = suspect_source::Uri::from_path(
        &args
            .target
            .canonicalize()
            .unwrap_or_else(|_| args.target.clone()),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let overlay_doc = suspect_low::LowDoc::parse(
        overlay_uri,
        suspect_source::Source::from_vec(overlay_text.as_bytes().to_vec()),
    );
    let target_doc = suspect_low::LowDoc::parse(
        target_uri,
        suspect_source::Source::from_vec(target_text.as_bytes().to_vec()),
    );
    let overlay_value = suspect_overlay::Value::from_node(overlay_doc.root());
    let Ok(ov_obj) = serde_json::from_str::<serde_json::Value>(&overlay_value.to_json()) else {
        anyhow::bail!("overlay is not a valid document");
    };
    let actions = ov_obj
        .get("actions")
        .and_then(|a| a.as_array())
        .ok_or_else(|| anyhow::anyhow!("overlay has no actions array"))?;
    let mut total_matches = 0usize;
    let mut zero_match = 0usize;
    for (idx, action) in actions.iter().enumerate() {
        let Some(target_expr) = action.get("target").and_then(|t| t.as_str()) else {
            eprintln!("action {idx}: no target (skipped)");
            continue;
        };
        let matches = suspect_jsonpath::Path::parse(target_expr)
            .map(|path| path.query(target_doc.root()).len())
            .unwrap_or(0);
        let update_kind = if action.get("remove").and_then(|r| r.as_bool()) == Some(true) {
            "remove"
        } else {
            "update"
        };
        eprintln!("action {idx}: target={target_expr} matches={matches} kind={update_kind}");
        total_matches += matches;
        if matches == 0 {
            zero_match += 1;
        }
    }
    eprintln!(
        "total: {total_matches} nodes matched across {} actions ({zero_match} zero-match actions)",
        actions.len()
    );
    Ok(i32::from(zero_match > 0))
}
