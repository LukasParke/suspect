//! Breaking-change detection between two spec revisions, for CI.
//!
//! Runs the workspace-backed break differ (path/operation/response/
//! component removals, newly-required members, security additions,
//! media-type removal, `additionalProperties: false`, parameter
//! requiredness, webhook removal) between `OLD` and `NEW`, printing
//! located findings. Exit code is non-zero when any break exists.

use std::path::PathBuf;

use suspect_ref::WorkspaceBuilder;

use crate::OutputFormat;

/// One breaking-change review.
#[derive(Debug, clap::Args)]
pub struct BreakingArgs {
    /// The previous spec revision.
    #[arg(required = true)]
    pub old: PathBuf,
    /// The current spec revision.
    #[arg(required = true)]
    pub new: PathBuf,
    /// Output format for the report.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// One rendered break: severity plus located message.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BreakFinding {
    /// `error` (removals, newly-required) or `warning` (constraint
    /// tightening, security additions).
    pub severity: String,
    /// Human-readable break description.
    pub message: String,
    /// Zero-based line in the NEW revision, when the break is locatable
    /// there (removals anchor at line 0).
    pub line: u32,
}

/// Reviews OLD against NEW. The NEW document is loaded into a workspace so
/// the differ resolves cross-file refs through the live copies; the old
/// revision is fed as text.
///
/// # Errors
/// Propagates file IO and contract compilation failures.
pub fn break_findings(
    old_path: &std::path::Path,
    new_path: &std::path::Path,
) -> anyhow::Result<Vec<BreakFinding>> {
    let old_absolute = old_path.canonicalize()?;
    let new_absolute = new_path.canonicalize()?;
    let old_text = std::fs::read_to_string(&old_absolute)?;

    let dir = new_absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("new path has no parent"))?;
    let ws = WorkspaceBuilder::new()
        .root(dir)
        .build()
        .map_err(anyhow::Error::msg)?;
    ws.load_all(
        new_absolute
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("new path is not UTF-8"))?,
    )
    .map_err(anyhow::Error::msg)?;
    let uri = ws
        .uris()
        .into_iter()
        .find(|u| {
            u.as_str().ends_with(
                &new_absolute
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .ok_or_else(|| anyhow::anyhow!("new document not loaded into the workspace"))?;

    let mut old_map = std::collections::HashMap::new();
    old_map.insert(uri.as_str().to_owned(), old_text);

    let changes = suspect_lsp::commands::breaking_changes(&ws, &old_map);
    let new_low = ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("new document not loaded"))?
        .doc();
    let inner = new_low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());

    Ok(changes
        .iter()
        .map(|change| {
            let line = suspect_lsp::state::offset_of_utf16(
                bytes,
                li,
                change.range.start.line,
                change.range.start.character,
            )
            .map(|offset| {
                u32::try_from(bytes[..offset].iter().filter(|&&b| b == b'\n').count() + 1)
                    .unwrap_or(1)
            })
            .unwrap_or(1);
            BreakFinding {
                severity: if change.severity == tower_lsp::lsp_types::DiagnosticSeverity::ERROR {
                    "error"
                } else {
                    "warning"
                }
                .to_owned(),
                message: change.message.clone(),
                line,
            }
        })
        .collect())
}

/// Runs the review and prints the report; exits non-zero when any break
/// exists.
///
/// # Errors
/// Propagates output serialization failures.
pub fn breaking(args: &BreakingArgs) -> anyhow::Result<i32> {
    use crate::output;
    let findings = break_findings(&args.old, &args.new)?;
    let rendered: Vec<output::Finding> = findings
        .iter()
        .map(|f| output::Finding {
            file: "old → new".to_owned(),
            severity: if f.severity == "error" {
                output::Severity::Error
            } else if f.severity == "warning" {
                output::Severity::Warning
            } else {
                output::Severity::Info
            },
            code: "breaking-change".into(),
            message: f.message.clone(),
            line: f.line,
            col: 1,
            range: None,
        })
        .collect();
    match args.text.format {
        OutputFormat::Text => output::print_findings(&rendered),
        OutputFormat::Json => output::print_json(&findings)?,
        OutputFormat::Sarif => {
            let sarif = crate::sarif::sarif_log(&rendered);
            output::print_json(&sarif)?;
        }
    }
    Ok(i32::from(!findings.is_empty()))
}
