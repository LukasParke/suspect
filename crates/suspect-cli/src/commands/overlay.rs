//! `suspect overlay apply`: parse an Overlay 1.0 document, apply its actions
//! to a target document, and emit the transformed tree as JSON or YAML.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use suspect_overlay::{self, OverlayDoc};

use crate::output::{self};
use crate::DocFormat;

/// Overlay subcommands.
#[derive(Debug, Subcommand)]
pub enum OverlayCmd {
    /// Apply an overlay document to a target document.
    Apply {
        /// Overlay 1.0 document.
        overlay: PathBuf,
        /// Document the overlay acts on.
        target: PathBuf,
        /// Write the result here instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}


/// Applies an overlay document to a target document, returning the raw
/// apply result (testable core of the `overlay apply` command).
///
/// # Errors
/// IO, malformed overlay document, or an invalid action.
pub fn apply_docs(
    overlay: &Path,
    target: &Path,
) -> anyhow::Result<suspect_overlay::Applied> {
    let ov_doc = crate::load_doc(overlay)?;
    let target_doc = crate::load_doc(target)?;
    let parsed = OverlayDoc::parse(&ov_doc)?;
    Ok(suspect_overlay::apply(&parsed, target_doc.root())?)
}

/// `suspect overlay apply <OVERLAY> <TARGET> [-o OUT]`. Prints an
/// applied/unmatched summary to stderr; always exits 0 on success.
///
/// # Errors
/// See [`apply_docs`]; IO on the output path.
pub fn run(cmd: OverlayCmd) -> anyhow::Result<i32> {
    let OverlayCmd::Apply { overlay, target, output } = cmd;
    let applied = apply_docs(&overlay, &target)?;

    let fmt = output::pick_doc_format(output.as_deref(), &target);
    let text = match fmt {
        DocFormat::Json => applied.output.to_json_pretty(),
        DocFormat::Yaml => applied.output.to_yaml(),
    };
    output::write_or_stdout(&text, output.as_deref())?;

    eprintln!(
        "applied {} action(s), {} unmatched target(s)",
        applied.applied_actions,
        applied.unmatched_targets.len()
    );
    for t in &applied.unmatched_targets {
        eprintln!("  unmatched: {t}");
    }
    Ok(0)
}
