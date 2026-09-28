//! `suspect overlay` subcommands: apply, dry-run, explain, and diff-driven
//! overlay synthesis.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use suspect_overlay::{OverlayDoc, Value};

use crate::DocFormat;
use crate::output::{self};

/// Overlay subcommands.
#[derive(Debug, Subcommand)]
pub enum OverlayCmd {
    /// Apply an overlay document to a target document.
    Apply {
        /// Overlay 1.0/1.1 document.
        overlay: PathBuf,
        /// Document the overlay acts on.
        target: PathBuf,
        /// Write the result here instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Explain what each action selects and changes, in sequence.
    Explain {
        /// Overlay 1.0/1.1 document.
        overlay: PathBuf,
        /// Document the overlay acts on.
        target: PathBuf,
    },
    /// Synthesize an Overlay 1.1 document from two document revisions.
    Diff {
        /// The current document.
        old: PathBuf,
        /// The revised document.
        new: PathBuf,
        /// Write the overlay here instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

/// Applies an overlay document to a target document, returning the raw
/// apply result (testable core of the `overlay apply` command).
///
/// # Errors
/// IO, malformed overlay document, or an invalid action.
pub fn apply_docs(overlay: &Path, target: &Path) -> anyhow::Result<suspect_overlay::Applied> {
    let (applied, _) = apply_with_doc(overlay, target)?;
    Ok(applied)
}

/// Applies an overlay, also returning the parsed overlay document.
fn apply_with_doc(
    overlay: &Path,
    target: &Path,
) -> anyhow::Result<(suspect_overlay::Applied, OverlayDoc<'static>)> {
    let ov_doc = crate::load_doc(overlay)?;
    let target_doc = crate::load_doc(target)?;
    // Keep the overlay document alive for the action views' lifetime.
    let ov_doc: &'static suspect_low::LowDoc = Box::leak(Box::new(ov_doc));
    let parsed = OverlayDoc::parse(ov_doc)?;
    Ok((suspect_overlay::apply(&parsed, target_doc.root())?, parsed))
}

/// Runs the overlay subcommands.
///
/// # Errors
/// IO, malformed documents, invalid actions, output serialization.
pub fn run(cmd: OverlayCmd) -> anyhow::Result<i32> {
    match cmd {
        OverlayCmd::Apply {
            overlay,
            target,
            output,
        } => {
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
        OverlayCmd::Explain { overlay, target } => {
            let (parsed, target_doc) = {
                let ov_doc = crate::load_doc(&overlay)?;
                let target_doc = crate::load_doc(&target)?;
                let ov_doc: &'static suspect_low::LowDoc = Box::leak(Box::new(ov_doc));
                (OverlayDoc::parse(ov_doc)?, target_doc)
            };
            let steps = suspect_overlay::explain(&parsed, target_doc.root())?;
            let width = steps.len().checked_ilog10().unwrap_or(0) as usize + 1;
            for step in &steps {
                println!(
                    "{:>width$}. [{}] {} — {} match(es)",
                    step.index + 1,
                    step.kind,
                    step.target,
                    step.matches,
                    width = width
                );
                if let Some(before) = &step.before {
                    println!("     before: {before}");
                }
                if let Some(after) = &step.after {
                    println!("     after:  {after}");
                }
            }
            eprintln!("{} action(s) explained", steps.len());
            Ok(0)
        }
        OverlayCmd::Diff { old, new, output } => {
            let old_doc = crate::load_doc(&old)?;
            let new_doc = crate::load_doc(&new)?;
            let title = format!("Changes from {} to {}", old.display(), new.display());
            let overlay: Value =
                suspect_overlay::synthesize_overlay(old_doc.root(), new_doc.root(), &title);
            let fmt = output::pick_doc_format(output.as_deref(), &new);
            let text = match fmt {
                DocFormat::Json => overlay.to_json_pretty(),
                DocFormat::Yaml => overlay.to_yaml(),
            };
            output::write_or_stdout(&text, output.as_deref())?;
            eprintln!(
                "synthesized overlay from {} → {}",
                old.display(),
                new.display()
            );
            Ok(0)
        }
    }
}
