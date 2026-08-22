//! `suspect fmt`: parse a document and re-emit it in canonical JSON/YAML.
//! Canonical output drops formatting trivia (comments, quoting style,
//! anchor/alias spelling); structure, key order, and scalar values survive.

use std::path::Path;

use suspect_overlay::Value;

use crate::DocFormat;
use crate::output::{self};

/// `suspect fmt <IN> [-o OUT] [--json|--yaml]`. Format defaults to the input
/// extension (`.json` -> JSON, else YAML).
///
/// # Errors
/// IO or parse failures.
pub fn fmt(input: &Path, out: Option<&Path>, json: bool, yaml: bool) -> anyhow::Result<i32> {
    let doc = crate::load_doc(input)?;
    let value = Value::from_node(doc.root());

    let fmt = if json {
        DocFormat::Json
    } else if yaml {
        DocFormat::Yaml
    } else {
        output::pick_doc_format(out, input)
    };
    let text = match fmt {
        DocFormat::Json => value.to_json_pretty(),
        DocFormat::Yaml => value.to_yaml(),
    };
    output::write_or_stdout(&text, out)?;
    Ok(0)
}
