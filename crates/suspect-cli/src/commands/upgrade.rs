//! `suspect upgrade` — Swagger 2.0 to OpenAPI 3.1 conversion.

use std::path::PathBuf;

use suspect_overlay::Value as OverlayValue;

use crate::OutputFormat;

/// One Swagger 2.0 → OpenAPI 3.1 conversion.
#[derive(Debug, clap::Args)]
pub struct UpgradeArgs {
    /// The Swagger 2.0 document to upgrade.
    #[arg(required = true)]
    pub input: PathBuf,
    /// Output path (`stdout` when omitted).
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Serialization format for the upgraded document.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// Upgrades the document and writes the result; also validates the output
/// with the OpenAPI 3.1 battery so the user sees the converted document's
/// remaining findings.
///
/// # Errors
/// Propagates IO, parsing, and serialization failures.
pub fn upgrade(args: &UpgradeArgs) -> anyhow::Result<i32> {
    let absolute = args.input.canonicalize()?;
    let text = std::fs::read_to_string(&absolute)?;
    let uri = suspect_source::Uri::from_path(&absolute).map_err(anyhow::Error::msg)?;
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    if low.sniff_family() != suspect_low::SpecFamily::Oas2 {
        anyhow::bail!(
            "input is not a Swagger 2.0 document (sniffed {:?})",
            low.sniff_family()
        );
    }
    let doc = suspect_overlay::Value::from_node(low.root()).to_json();
    let doc: serde_json::Value = serde_json::from_str(&doc)?;
    let upgraded = suspect_upgrade::upgrade(&doc).map_err(anyhow::Error::msg)?;

    let output_text = if absolute.to_string_lossy().ends_with(".json") {
        serde_json::to_string_pretty(&upgraded)? + "\n"
    } else {
        // JSON is valid YAML; serialize through the JSON string to keep
        // key insertion order.
        serde_json::to_string_pretty(&upgraded)? + "\n"
    };
    match &args.output {
        Some(path) => {
            std::fs::write(path, &output_text)?;
            eprintln!("upgraded → {}", path.display());
        }
        None => print!("{output_text}"),
    }
    // Validate the upgraded document with the 3.1 battery so the user
    // sees the remaining findings when the output is a file.
    if args.output.is_some() {
        let out_path = args.output.as_ref().unwrap();
        let findings = crate::commands::validate::validate_file(out_path.as_path(), None);
        let error_count = findings
            .iter()
            .filter(|f| f.severity == crate::output::Severity::Error)
            .count();
        eprintln!(
            "validation: {} findings ({} errors) — run `suspect validate {}` for details",
            findings.len(),
            error_count,
            out_path.display()
        );
        if error_count > 0 {
            return Ok(1);
        }
    }
    Ok(0)
}
