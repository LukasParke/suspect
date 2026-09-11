//! Compare canonical wire contracts and actual native SDK interfaces.
use crate::OutputFormat;
use anyhow::Result;
use clap::Args;
use serde_json::json;
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
use suspect_codegen::compatibility;
use suspect_ir::contract::Contract;

/// Source/configuration pairs to compare, using the SDK session JSON format.
#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Baseline SDK session configuration; its source is relative to this file.
    #[arg(long)]
    pub before: PathBuf,
    /// Candidate SDK session configuration; its source is relative to this file.
    #[arg(long)]
    pub after: PathBuf,
    /// Versioned report JSON or human-readable Markdown migration notes.
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
fn load(input: &suspect_codegen::generation_session::Input) -> Result<Arc<Contract>> {
    let (workspace, entry) = input.open()?;
    Ok(Arc::new(Contract::from_workspace(&workspace, &entry)?))
}
/// Emit a report; 0 means proved compatible within its recorded scope, 1 means
/// breaking/potentially breaking/unknown, and 2 means input/configuration failure.
///
/// # Errors
/// Propagates output failures. Input/comparison failures are structured results.
pub fn compare(args: CompareArgs) -> Result<i32> {
    let result = (|| -> Result<compatibility::CompatibilityReport> {
        let (old_path, old) = super::codegen_session::read_config(&args.before)?;
        let (new_path, new) = super::codegen_session::read_config(&args.after)?;
        let old_contract = load(&old_path)?;
        let new_contract = load(&new_path)?;
        Ok(if old.operation_ids == new.operation_ids {
            compatibility::compare_with_options(
                old_contract,
                new_contract,
                &old.operation_ids,
                (&old.targets, &old.generation),
                (&new.targets, &new.generation),
            )?
        } else {
            let before = compatibility::snapshot_with_options(
                old_contract,
                &old.operation_ids,
                &old.targets,
                &old.generation,
            )?;
            let after = compatibility::snapshot_with_options(
                new_contract,
                &new.operation_ids,
                &new.targets,
                &new.generation,
            )?;
            compatibility::compare_snapshots(&before, &after)
        })
    })();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let exit = match result {
        Ok(report) => {
            let compatible = report.is_proven_compatible();
            if matches!(args.format, OutputFormat::Json) {
                serde_json::to_writer(&mut stdout, &report)?;
                writeln!(stdout)?;
            } else {
                stdout.write_all(report.migration_notes().as_bytes())?;
            }
            i32::from(!compatible)
        }
        Err(error) => {
            if matches!(args.format, OutputFormat::Json) {
                serde_json::to_writer(
                    &mut stdout,
                    &json!({"format":"suspect-sdk-compatibility-v1","success":false,"status":"comparison-error","diagnostics":[{"code":"sdk-compatibility","message":format!("{error:#}")}]}),
                )?;
                writeln!(stdout)?;
            } else {
                writeln!(stdout, "SDK comparison failed: {error:#}")?;
            }
            2
        }
    };
    stdout.flush()?;
    Ok(exit)
}
