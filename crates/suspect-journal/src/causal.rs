//! Causal contract debugger: connects runtime validation failures back to
//! the exact spec source location, git history, and historical traffic.
//!
//! Given a validation error, this module produces a timeline:
//! - Which schema constraint fired
//! - Where in the YAML source it lives (byte offset from the lossless CST)
//! - Who introduced it and when (git blame)
//! - Whether any recorded traffic ever passed this check
//! - When it started failing

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One step in the causal chain from failure to origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalStep {
    /// Step type: `constraint` | `source` | `blame` | `history`.
    pub kind: String,
    /// What this step tells us.
    pub message: String,
    /// Byte offset into the spec file (when applicable).
    pub byte_offset: Option<usize>,
    /// Line/column in the spec file (when applicable).
    pub line_col: Option<(usize, usize)>,
    /// Git commit SHA (when applicable).
    pub commit: Option<String>,
    /// Git author name (when applicable).
    pub author: Option<String>,
    /// Commit date ISO 8601 (when applicable).
    pub date: Option<String>,
}

/// The full causal chain for one failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalTrace {
    /// The constraint or check that fired.
    pub constraint: String,
    /// The spec file path.
    pub spec_path: PathBuf,
    /// Ordered steps from symptom to origin.
    pub steps: Vec<CausalStep>,
    /// Whether historical traffic ever passed this constraint.
    pub ever_passed: bool,
    /// When it first failed in recorded traffic (if known).
    pub first_failure: Option<String>,
}

/// Traces one failure back through the causal chain.
#[must_use]
pub fn trace_failure(
    constraint: &str,
    spec_path: &Path,
    byte_offset: Option<usize>,
    cassette_dir: Option<&Path>,
) -> CausalTrace {
    let mut steps = Vec::new();

    // Step 1: the constraint itself
    steps.push(CausalStep {
        kind: "constraint".to_owned(),
        message: format!("Constraint fired: {constraint}"),
        byte_offset,
        line_col: None,
        commit: None,
        author: None,
        date: None,
    });

    // Step 2: source location (byte offset → line/col)
    let line_col = byte_offset.and_then(|off| offset_to_line_col(spec_path, off));
    if let Some((line, col)) = line_col {
        steps.push(CausalStep {
            kind: "source".to_owned(),
            message: format!("Spec source: line {}, column {}", line, col),
            byte_offset,
            line_col: Some((line, col)),
            commit: None,
            author: None,
            date: None,
        });
    }

    // Step 3: git blame for that line
    if let Some((line, _)) = line_col
        && let Some(blame) = git_blame(spec_path, line)
    {
        steps.push(CausalStep {
            kind: "blame".to_owned(),
            message: format!(
                "Introduced by {} on {} ({})",
                blame.author.as_deref().unwrap_or("unknown"),
                blame.date.as_deref().unwrap_or("unknown"),
                blame.commit.as_deref().unwrap_or("unknown")
            ),
            byte_offset,
            line_col,
            commit: blame.commit,
            author: blame.author,
            date: blame.date,
        });
    }

    // Step 4: historical traffic
    let (ever_passed, first_failure) = search_cassette_history(constraint, cassette_dir);
    if cassette_dir.is_some() {
        steps.push(CausalStep {
            kind: "history".to_owned(),
            message: if ever_passed {
                format!(
                    "Constraint previously passed in recorded traffic; first failure: {}",
                    first_failure.as_deref().unwrap_or("unknown")
                )
            } else {
                "No recorded traffic has ever passed this constraint".to_owned()
            },
            byte_offset,
            line_col,
            commit: None,
            author: None,
            date: None,
        });
    }

    CausalTrace {
        constraint: constraint.to_owned(),
        spec_path: spec_path.to_path_buf(),
        steps,
        ever_passed,
        first_failure,
    }
}

/// Converts a byte offset to a 1-indexed line and column.
#[must_use]
pub fn offset_to_line_col(path: &Path, offset: usize) -> Option<(usize, usize)> {
    let content = std::fs::read(path).ok()?;
    let clamped = offset.min(content.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for &byte in &content[..clamped] {
        if byte == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Some((line, col))
}

/// Git blame result for one line.
#[derive(Debug, Clone, Default)]
pub struct BlameInfo {
    /// Commit SHA that last touched the line.
    pub commit: Option<String>,
    /// Author name from the porcelain `author` header.
    pub author: Option<String>,
    /// Author time as ISO 8601 UTC (from `author-time`).
    pub date: Option<String>,
}

/// Runs `git blame` for one line in a file.
#[must_use]
pub fn git_blame(path: &Path, line: usize) -> Option<BlameInfo> {
    let output = Command::new("git")
        .args([
            "blame",
            "--porcelain",
            "-L",
            &format!("{line},{line}"),
            path.to_str()?,
        ])
        .current_dir(path.parent()?)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(parse_blame_porcelain(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parses `git blame --porcelain` output for one line. The first line of
/// each entry is `<commit> <orig-line> <final-line> [<count>]`, where the
/// commit is 40 (SHA-1) or 64 (SHA-256) hex digits.
fn parse_blame_porcelain(text: &str) -> BlameInfo {
    let mut info = BlameInfo::default();
    for line in text.lines() {
        let first = line.split(' ').next().unwrap_or("");
        if info.commit.is_none()
            && matches!(first.len(), 40 | 64)
            && first.chars().all(|c| c.is_ascii_hexdigit())
        {
            info.commit = Some(first.to_owned());
        } else if let Some(author) = line.strip_prefix("author ") {
            info.author = Some(author.to_owned());
        } else if let Some(ts) = line.strip_prefix("author-time ")
            && let Ok(secs) = ts.parse::<i64>()
        {
            info.date = Some(unix_to_iso(secs));
        }
    }
    info
}

/// Converts a Unix timestamp to ISO 8601 UTC.
#[must_use]
pub fn unix_to_iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil-from-days algorithm (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Searches cassette files for evidence that a constraint ever passed.
#[must_use]
pub fn search_cassette_history(
    constraint: &str,
    cassette_dir: Option<&Path>,
) -> (bool, Option<String>) {
    let Some(dir) = cassette_dir else {
        return (false, None);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (false, None);
    };

    let mut first_failure: Option<String> = None;
    let mut ever_passed = false;

    // Directory order is unspecified; sort so results are reproducible.
    let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let ext = path.extension().and_then(|e| e.to_str());
        if !matches!(ext, Some("jsonl") | Some("ndjson") | Some("json")) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        for line in content.lines() {
            // Look for violation records matching this constraint
            if !line.contains(constraint) {
                continue;
            }
            // Try to parse as JSON and extract timestamp + pass/fail
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let ts = v
                    .get("ts")
                    .or_else(|| v.get("timestamp"))
                    .and_then(|t| t.as_str())
                    .map(std::borrow::ToOwned::to_owned);
                let is_violation = v.get("kind").and_then(|k| k.as_str()) == Some("violation")
                    || v.get("violation").is_some();
                if is_violation {
                    // The earliest timestamp wins; RFC 3339 / ISO 8601 UTC
                    // strings order correctly as text.
                    if let Some(ts) = ts
                        && first_failure.as_ref().is_none_or(|current| ts < *current)
                    {
                        first_failure = Some(ts);
                    }
                } else {
                    ever_passed = true;
                }
            }
        }
    }

    (ever_passed, first_failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_iso_known_values() {
        assert_eq!(unix_to_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_iso(1_710_489_600), "2024-03-15T08:00:00Z");
    }

    #[test]
    fn offset_to_line_col_counts_newlines() {
        let dir = std::env::temp_dir().join("suspect-causal-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("spec.yaml");
        std::fs::write(&path, b"openapi: 3.1.0\ninfo:\n  title: T\n").unwrap();
        // Offset of "title" line start: 21
        assert_eq!(offset_to_line_col(&path, 21), Some((3, 1)));
        assert_eq!(offset_to_line_col(&path, 0), Some((1, 1)));
    }

    #[test]
    fn porcelain_blame_yields_the_commit_author_and_date() {
        let sha = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        let text = format!(
            "{sha} 12 12 1\nauthor Ada Lovelace\nauthor-mail <ada@example.com>\nauthor-time 1710489600\nauthor-tz +0000\nsummary fix\nfilename spec.yaml\n\tminLength: 3\n"
        );
        let info = parse_blame_porcelain(&text);
        assert_eq!(info.commit.as_deref(), Some(sha));
        assert_eq!(info.author.as_deref(), Some("Ada Lovelace"));
        assert_eq!(info.date.as_deref(), Some("2024-03-15T08:00:00Z"));
    }

    #[test]
    fn the_earliest_violation_is_the_first_failure_regardless_of_file_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.jsonl"),
            "{\"kind\":\"violation\",\"constraint\":\"minLength\",\"ts\":\"2024-05-01T00:00:00Z\"}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.jsonl"),
            "{\"kind\":\"violation\",\"constraint\":\"minLength\",\"ts\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .unwrap();
        let (ever_passed, first) = search_cassette_history("minLength", Some(dir.path()));
        assert!(!ever_passed);
        assert_eq!(first.as_deref(), Some("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn trace_failure_produces_constraint_step() {
        let trace = trace_failure("minLength", Path::new("/nonexistent.yaml"), None, None);
        assert_eq!(trace.steps[0].kind, "constraint");
        assert!(!trace.ever_passed);
    }
}
