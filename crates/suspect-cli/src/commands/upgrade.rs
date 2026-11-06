//! `suspect upgrade` — Swagger 2.0 to OpenAPI 3.1 conversion.

use std::path::PathBuf;

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

    // The output format follows the destination's extension (the input's
    // when writing to stdout): YAML for `.yaml`/`.yml`, JSON otherwise.
    let target = args.output.as_deref().unwrap_or(&absolute);
    let yaml = target
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"));
    let document = to_ordered(&upgraded, true);
    let output_text = if yaml {
        document.to_yaml()
    } else {
        document.to_json_pretty() + "\n"
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
    if let Some(out_path) = &args.output {
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

/// Conventional OpenAPI top-level key order; other keys (extensions)
/// follow in their existing order.
const TOP_LEVEL_ORDER: [&str; 9] = [
    "openapi",
    "info",
    "jsonSchemaDialect",
    "servers",
    "security",
    "tags",
    "externalDocs",
    "paths",
    "components",
];

/// Converts the upgraded JSON into the overlay value tree, which has
/// ordered objects and a YAML emitter. The root's keys are put in
/// conventional OpenAPI order.
fn to_ordered(value: &serde_json::Value, root: bool) -> suspect_overlay::Value {
    use suspect_overlay::Value as V;
    match value {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Bool(b) => V::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(V::Int)
            .or_else(|| n.as_f64().map(V::Float))
            .unwrap_or_else(|| V::Str(n.to_string().into())),
        serde_json::Value::String(s) => V::Str(s.as_str().into()),
        serde_json::Value::Array(items) => {
            V::Array(items.iter().map(|item| to_ordered(item, false)).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(Box<str>, V)> = map
                .iter()
                .map(|(k, v)| (k.as_str().into(), to_ordered(v, false)))
                .collect();
            if root {
                let rank = |key: &str| {
                    TOP_LEVEL_ORDER
                        .iter()
                        .position(|known| *known == key)
                        .unwrap_or(TOP_LEVEL_ORDER.len())
                };
                entries.sort_by_key(|(key, _)| rank(key));
            }
            V::Object(entries)
        }
    }
}
