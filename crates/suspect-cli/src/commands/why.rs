//! `suspect why`: trace a validation failure back to its origin.
//!
//! Given the constraint that fired and the spec it fired in, print the
//! timeline the causal debugger builds: the source location, who last
//! touched the line (git blame), and whether any recorded traffic ever
//! passed the check — the difference between "always broken" and
//! "regressed".

use std::path::Path;

/// Runs `suspect why`.
///
/// # Errors
/// Filesystem and git failures render as errors; a missing offset is not
/// one (the trace degrades gracefully, exactly as the debugger does).
pub fn why(
    constraint: &str,
    spec: &Path,
    offset: Option<usize>,
    line: Option<usize>,
    column: Option<usize>,
    cassette_dir: Option<&Path>,
    json: bool,
) -> anyhow::Result<i32> {
    // An editor location (line/column, 1-based) beats a byte offset for
    // humans; a byte offset beats nothing.
    let byte_offset = match (offset, line) {
        (Some(off), _) => Some(off),
        (None, Some(line)) => Some(line_col_to_offset(spec, line, column.unwrap_or(1))?),
        (None, None) => None,
    };
    let trace = suspect_journal::causal::trace_failure(constraint, spec, byte_offset, cassette_dir);
    if json {
        println!("{}", serde_json::to_string_pretty(&trace)?);
        return Ok(0);
    }
    for step in &trace.steps {
        let where_ = step
            .line_col
            .map(|(line, column)| format!(" (line {line}, column {column})"))
            .unwrap_or_default();
        let who = match (&step.author, &step.date) {
            (Some(author), Some(date)) => format!(" — {author}, {date}"),
            (Some(author), None) => format!(" — {author}"),
            _ => String::new(),
        };
        println!("{}{where_}: {}", step.kind, step.message);
        if let Some(commit) = &step.commit {
            println!("    {commit}{who}");
        }
    }
    if trace.ever_passed {
        println!("history: recorded traffic passed this constraint before; it regressed");
    } else if cassette_dir.is_some() {
        println!("history: no recorded traffic ever passed this constraint");
    }
    Ok(0)
}

/// Converts a 1-based line/column to a byte offset by scanning the file.
///
/// # Errors
/// Fails when the file cannot be read or the position is past its end.
fn line_col_to_offset(path: &Path, line: usize, column: usize) -> anyhow::Result<usize> {
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let mut offset = 0usize;
    for _ in 1..line {
        let Some(nl) = bytes[offset..].iter().position(|&b| b == b'\n') else {
            anyhow::bail!("{} has fewer than {line} lines", path.display());
        };
        offset += nl + 1;
    }
    let column_bytes = column.saturating_sub(1);
    let Some(end) = bytes[offset..].iter().position(|&b| b == b'\n') else {
        if offset + column_bytes <= bytes.len() {
            return Ok(offset + column_bytes);
        }
        anyhow::bail!("line {line} has fewer than {column} columns");
    };
    anyhow::ensure!(
        column_bytes <= end,
        "line {line} has fewer than {column} columns"
    );
    Ok(offset + column_bytes)
}
