//! `suspect upgrade` — Swagger 2.0 to OpenAPI 3.1 conversion.

use std::path::PathBuf;

/// One Swagger 2.0 → OpenAPI 3.1 conversion.
#[derive(Debug, clap::Args)]
pub struct UpgradeArgs {
    /// The Swagger 2.0 document to upgrade.
    #[arg(required = true)]
    pub input: PathBuf,
    /// Output file and serialization (default: the `--output` extension,
    /// else the input's — `.yaml`/`.yml` emit YAML, anything else JSON).
    #[command(flatten)]
    pub out: crate::DocOut,
}

/// The emission format: the explicit flag, then a `.json`/`.yaml`/`.yml`
/// output path, then the input's extension; JSON when nothing says YAML.
fn pick_format(args: &UpgradeArgs) -> crate::DocFormat {
    let by_extension = |p: &std::path::Path| match p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("yaml" | "yml") => Some(crate::DocFormat::Yaml),
        Some("json") => Some(crate::DocFormat::Json),
        _ => None,
    };
    args.out
        .to
        .or_else(|| args.out.output.as_deref().and_then(by_extension))
        .or_else(|| by_extension(&args.input))
        .unwrap_or(crate::DocFormat::Json)
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
    let upgraded = crate::upgrade::upgrade(&doc).map_err(anyhow::Error::msg)?;

    let json_text = serde_json::to_string_pretty(&upgraded)? + "\n";
    let output_text = match pick_format(args) {
        crate::DocFormat::Json => json_text,
        crate::DocFormat::Yaml => {
            // Re-parse the JSON text (valid YAML) so the shared emitter
            // `fmt` and `bundle` use writes it, keeping key order.
            let uri = suspect_source::Uri::parse("mem://upgraded.json")
                .map_err(|e| anyhow::Error::msg(e.to_string()))?;
            let reparsed = suspect_low::LowDoc::parse(
                uri,
                suspect_source::Source::from_vec(json_text.into_bytes()),
            );
            let mut yaml = suspect_overlay::Value::from_node(reparsed.root()).to_yaml();
            if !yaml.ends_with('\n') {
                yaml.push('\n');
            }
            yaml
        }
    };
    match &args.out.output {
        Some(path) => {
            std::fs::write(path, &output_text)?;
            eprintln!("upgraded → {}", path.display());
        }
        None => print!("{output_text}"),
    }
    // Validate the upgraded document with the 3.1 battery so the user
    // sees the remaining findings when the output is a file.
    if let Some(out_path) = &args.out.output {
        let findings = crate::commands::validate::validate_file(out_path.as_path(), None, false);
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
