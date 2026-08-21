//! Semantic structural diff: materialize two documents into owned value
//! trees and report added/removed/changed scalars by JSON-pointer-ish path.
//! Objects compare by key, arrays by index; numbers compare as `f64`.

use std::path::Path;

use serde::Serialize;
use suspect_overlay::Value;

use crate::output;
use crate::OutputFormat;

/// One changed scalar: `from` -> `to` at `path`.
#[derive(Debug, Clone, Serialize)]
pub struct Changed {
    pub path: String,
    pub from: String,
    pub to: String,
}

/// Collected structural differences between two documents.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<Changed>,
}

impl DiffReport {
    /// True when nothing differs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Renders a value compactly for `from`/`to` spans (scalars render bare).
#[must_use]
pub fn render_scalar(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        other => other.to_json(),
    }
}

/// Escapes one JSON-pointer token (`~` -> `~0`, `/` -> `~1`).
#[must_use]
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Recursively diffs `a` against `b`, appending to `report`.
pub fn diff_values(path: &str, a: &Value, b: &Value, report: &mut DiffReport) {
    match (a, b) {
        (Value::Object(ae), Value::Object(be)) => {
            for (k, av) in ae {
                let child = format!("{path}/{}", escape_token(k));
                match be.iter().find(|(bk, _)| bk == k) {
                    Some((_, bv)) => diff_values(&child, av, bv, report),
                    None => report.removed.push(child),
                }
            }
            for (k, bv) in be {
                if !ae.iter().any(|(ak, _)| ak == k) {
                    report.added.push(format!("{path}/{}", escape_token(k)));
                    let _ = bv;
                }
            }
        }
        (Value::Array(ai), Value::Array(bi)) => {
            let shared = ai.len().min(bi.len());
            for (i, (av, bv)) in ai.iter().zip(bi.iter()).enumerate().take(shared) {
                diff_values(&format!("{path}/{i}"), av, bv, report);
            }
            for (i, av) in ai.iter().enumerate().skip(shared) {
                report.removed.push(format!("{path}/{i}"));
                let _ = av;
            }
            for i in shared..bi.len() {
                report.added.push(format!("{path}/{i}"));
            }
        }
        _ => {
            let kind_of = |v: &Value| std::mem::discriminant(v);
            let equal = match (a, b) {
                (Value::Int(x), Value::Int(y)) => x == y,
                (Value::Float(x), Value::Float(y)) => x == y,
                // numeric cross-comparison: 1 == 1.0
                (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => {
                    (*x as f64) == *y
                }
                _ => kind_of(a) == kind_of(b) && a == b,
            };
            if !equal {
                report.changed.push(Changed {
                    path: path.to_string(),
                    from: render_scalar(a),
                    to: render_scalar(b),
                });
            }
        }
    }
}

/// Diffs two documents and prints the report; exit code is always 0
/// (differences are a result, not a failure).
///
/// # Errors
/// IO or parse failures on either input.
pub fn diff_files(a: &Path, b: &Path, format: OutputFormat) -> anyhow::Result<i32> {
    let doc_a = crate::load_doc(a)?;
    let doc_b = crate::load_doc(b)?;
    let va = Value::from_node(doc_a.root());
    let vb = Value::from_node(doc_b.root());
    let mut report = DiffReport::default();
    diff_values("", &va, &vb, &mut report);
    match format {
        OutputFormat::Text => {
            for p in &report.added {
                println!("+ {p}");
            }
            for p in &report.removed {
                println!("- {p}");
            }
            for c in &report.changed {
                println!("~ {}: {} -> {}", c.path, c.from, c.to);
            }
        }
        OutputFormat::Json => output::print_json(&report)?,
    }
    Ok(0)
}
