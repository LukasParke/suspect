//! `suspect config [INPUT] [--explain]`: the configuration a command on
//! INPUT would use, and with `--explain` where every value came from.

use std::path::PathBuf;

use suspect_config::{Origin, Resolved, schema};

use crate::OutputFormat;

/// `suspect config` arguments.
#[derive(Debug, clap::Args)]
pub struct ConfigArgs {
    /// Report the configuration a command on this input would use: the
    /// nearest settings file above it, else the one above the working
    /// directory.
    pub input: Option<PathBuf>,
    /// Print every setting with its value and origin (`flag`, `env
    /// SUSPECT_X`, `.suspect.yaml:12`, `default`), plus debug variables.
    #[arg(long)]
    pub explain: bool,
    /// With `--explain`: hide settings that hold their default.
    #[arg(long, requires = "explain")]
    pub set_only: bool,
    /// Output format for `--explain` (`text` or `json`).
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// Runs `suspect config`.
///
/// # Errors
/// `--format sarif`; with no `--explain`, a rejected configuration value.
pub fn config(args: &ConfigArgs) -> anyhow::Result<i32> {
    crate::reject_sarif("config", args.text.format)?;
    let inputs: Vec<PathBuf> = args.input.iter().cloned().collect();
    let start = crate::settings::start(&inputs);
    let (resolved, failed) = match suspect_config::resolve(&start) {
        Ok(resolved) => (resolved, false),
        Err(error) if args.explain => (*error.resolved, true),
        Err(error) => return Err(anyhow::anyhow!("{error}")),
    };
    if !args.explain {
        crate::settings::emit_warnings(&resolved);
        println!("{}", resolved.describe());
        return Ok(0);
    }
    match args.text.format {
        OutputFormat::Json => {
            let mut json = resolved.explain_json();
            if args.set_only
                && let Some(values) = json["values"].as_array_mut()
            {
                values.retain(|v| v["layer"] != "default");
            }
            json["debug"] = schema::DEBUG_VARS
                .iter()
                .map(|var| {
                    serde_json::json!({
                        "name": var.name,
                        "set": std::env::var_os(var.name).is_some(),
                        "doc": var.doc,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => print!(
            "{}",
            explain_text(&resolved, args.input.as_deref(), args.set_only)
        ),
    }
    Ok(if failed { 2 } else { 0 })
}

/// The `--explain` text report.
#[must_use]
pub fn explain_text(
    resolved: &Resolved,
    input: Option<&std::path::Path>,
    set_only: bool,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let from = input.map_or_else(
        || "the working directory".to_owned(),
        |p| p.display().to_string(),
    );
    let _ = writeln!(
        out,
        "root     {}  (discovered from {from})",
        resolved.root.display()
    );
    if resolved.files.is_empty() {
        let _ = writeln!(out, "files    none (built-in defaults)");
    }
    for (i, file) in resolved.files.iter().enumerate() {
        let label = if Some(file) == resolved.manifest.as_ref() {
            " (manifest, deprecated policy)"
        } else if Some(file) != resolved.config.as_ref() {
            " (extends)"
        } else {
            ""
        };
        let head = if i == 0 { "files" } else { "" };
        let _ = writeln!(out, "{head:<8} {}{label}", file.display());
    }
    out.push('\n');
    let rows = resolved.explain();
    let width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0).max(24) + 2;
    for row in rows {
        if set_only && row.origin == Origin::Default {
            continue;
        }
        let value = row.value.clone().unwrap_or_else(|| "unset".to_owned());
        let mut notes: Vec<String> = Vec::new();
        if row.trace.len() > 1
            && let Some((origin, raw)) = row.trace.get(row.trace.len() - 2)
        {
            notes.push(format!("over {origin} = {raw}"));
        }
        if matches!(row.origin, Origin::Manifest { .. }) {
            notes.push("deprecated location".to_owned());
        }
        if let Some(var) = row.setting.env
            && !matches!(row.origin, Origin::Env { .. })
        {
            notes.push(format!("env: {var}"));
        }
        if !row.setting.flags.is_empty() {
            notes.push(format!("flag: {}", row.setting.flags.join(", ")));
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("  ({})", notes.join("; "))
        };
        let _ = writeln!(out, "{:<width$}{value:<24}  {}{notes}", row.key, row.origin);
    }
    out.push_str("\ndebug variables (not settings)\n");
    for var in schema::DEBUG_VARS {
        let state = if std::env::var_os(var.name).is_some() {
            "set"
        } else {
            "unset"
        };
        let _ = writeln!(out, "{:<width$}{state:<8}{}", var.name, var.doc);
    }
    if !resolved.warnings.is_empty() || !resolved.errors.is_empty() {
        out.push('\n');
    }
    for notice in &resolved.warnings {
        let _ = writeln!(out, "warning: {notice}");
    }
    for notice in &resolved.errors {
        let _ = writeln!(out, "error: {notice}");
    }
    out
}
