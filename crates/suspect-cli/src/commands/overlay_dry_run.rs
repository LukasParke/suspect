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
        // Overlay targets conventionally use bare dot segments for keys
        // with special characters (`$.paths./pets.get`). Normalize the
        // expression to RFC 9535 bracket form before matching, and count
        // both forms so a parser regression cannot silently zero out.
        let normalized = normalize_target(target_expr);
        let parse = |expr: &str| {
            suspect_jsonpath::Path::parse(expr)
                .map(|path| path.query(target_doc.root()).len())
                .unwrap_or(0)
        };
        let matches = std::cmp::max(parse(target_expr), parse(&normalized));
        let update_kind = if action.get("remove").and_then(|r| r.as_str()) == Some("true") {
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

/// Normalizes an Overlay/JSONPath target so that dot segments naming
/// keys with special characters become bracket segments:
/// `$.paths./pets.get` → `$['paths']['/pets'].get`.
///
/// Filter expressions (`?(...)`) and descendant probes (`..`) are
/// returned unchanged — the rewrite only covers the plain member-access
/// shapes the Overlay spec's examples use.
#[must_use]
pub fn normalize_target(expr: &str) -> String {
    if expr.contains("?(") || expr.contains("..") {
        return expr.to_owned();
    }
    let Some((anchor, rest)) = expr.split_once('$') else {
        return expr.to_owned();
    };
    let _ = anchor;
    let mut out = String::from("$");
    for segment in rest.split('.') {
        if segment.is_empty() {
            continue;
        }
        if segment == "*" {
            out.push_str(".*");
            continue;
        }
        let simple = segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '-')
            && segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$');
        if simple {
            out.push('.');
            out.push_str(segment);
        } else {
            out.push_str(&format!("['{}']", segment.replace('\'', "\\'")));
        }
    }
    out
}
