//! Shared argument types: one `--format` enum per output kind, so clap
//! accepts only the values a command renders and rejects the rest (exit 2)
//! before any work starts.

use std::path::PathBuf;

use clap::{Args, ValueEnum};

/// `--format` for commands that emit located findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum FindingsFormat {
    /// One `file:line:col [SEVERITY] code: message` line per finding.
    #[default]
    Text,
    /// One JSON array of findings.
    Json,
    /// SARIF 2.1.0 log for code-scanning integrations.
    Sarif,
    /// JUnit XML, one test case per finding.
    Junit,
}

/// `--format` for commands that emit one structured report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ReportFormat {
    /// Human-readable text.
    #[default]
    Text,
    /// One pretty-printed JSON document.
    Json,
}

impl ReportFormat {
    /// `true` for [`ReportFormat::Json`].
    #[must_use]
    pub const fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

/// `--format` for long-running commands that emit one record per event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum StreamFormat {
    /// Human-readable progress lines.
    #[default]
    Text,
    /// One JSON object per line. `json` is accepted for the `bridge` and
    /// `codegen-session` spelling.
    #[value(alias = "json")]
    Ndjson,
}

impl StreamFormat {
    /// `true` for [`StreamFormat::Ndjson`].
    #[must_use]
    pub const fn is_ndjson(self) -> bool {
        matches!(self, Self::Ndjson)
    }

    /// The value's name, as config files spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Ndjson => "ndjson",
        }
    }
}

/// `--format text|json|sarif|junit`.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Findings {
    /// Output format for the finding list.
    #[arg(long, value_enum, default_value_t)]
    pub format: FindingsFormat,
}

/// `--format text|json`.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Report {
    /// Output format for the report.
    #[arg(long, value_enum, default_value_t)]
    pub format: ReportFormat,
}

/// `--format text|ndjson`; `--report` is the old `test`/`load` spelling.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Stream {
    /// Event stream format: human lines or one JSON object per line.
    #[arg(long, value_enum, default_value_t, alias = "report")]
    pub format: StreamFormat,
}

/// A document written to one file, or stdout.
#[derive(Debug, Clone, Default, Args)]
pub struct DocOut {
    /// Write the document here instead of stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Serialization (default: the `--output` extension, else the
    /// input's). `--format` is the old `bundle`/`upgrade` spelling.
    #[arg(long = "to", value_enum, alias = "format")]
    pub to: Option<crate::DocFormat>,
}

/// Read-only gate: exit 1 when the result is not clean.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Check {
    /// Exit 1 when the result is not clean.
    #[arg(long)]
    pub check: bool,
}

/// Machine-readable report files for test commands.
#[derive(Debug, Clone, Default, Args)]
pub struct ReportFiles {
    /// Write the JSON run report here. `--json` is the old `load`
    /// spelling.
    #[arg(long = "report-file", value_name = "FILE", alias = "json")]
    pub report_file: Option<PathBuf>,
    /// Write a JUnit XML report here.
    #[arg(long, value_name = "FILE")]
    pub junit: Option<PathBuf>,
}

impl ReportFiles {
    /// Writes the requested files for a run of `command` with this
    /// outcome: the JSON report (`version`, `command`, counts and
    /// duration) and JUnit through `suspect_test::reporters::junit`.
    ///
    /// # Errors
    /// Filesystem IO.
    pub fn write(&self, command: &str, summary: &suspect_test::RunSummary) -> anyhow::Result<()> {
        if let Some(path) = &self.report_file {
            let report = serde_json::json!({
                "version": 1,
                "command": command,
                "passed": summary.passed,
                "failed": summary.failed,
                "skipped": summary.skipped,
                "duration_ms": summary.duration_ms,
            });
            write_report(path, &(serde_json::to_string_pretty(&report)? + "\n"))?;
        }
        if let Some(path) = &self.junit {
            write_report(path, &suspect_test::reporters::junit(summary))?;
        }
        Ok(())
    }

    /// [`ReportFiles::write`] for a run that counts passes and failures
    /// and started at `started`.
    ///
    /// # Errors
    /// Filesystem IO.
    pub fn write_counts(
        &self,
        command: &str,
        passed: usize,
        failed: usize,
        started: std::time::Instant,
    ) -> anyhow::Result<()> {
        let summary = suspect_test::RunSummary {
            passed,
            failed,
            skipped: 0,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        self.write(command, &summary)
    }
}

/// Writes `text` to `path`, creating parent directories.
///
/// # Errors
/// Filesystem IO, naming the path.
pub fn write_report(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_files_write_json_and_junit() {
        let dir = tempfile::tempdir().unwrap();
        let files = ReportFiles {
            report_file: Some(dir.path().join("out/run.json")),
            junit: Some(dir.path().join("out/run.xml")),
        };
        let summary = suspect_test::RunSummary {
            passed: 3,
            failed: 1,
            skipped: 0,
            duration_ms: 42,
        };
        files.write("replay", &summary).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("out/run.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["command"], "replay");
        assert_eq!(json["passed"], 3);
        assert_eq!(json["failed"], 1);
        let xml = std::fs::read_to_string(dir.path().join("out/run.xml")).unwrap();
        assert!(xml.contains("failures=\"1\""), "{xml}");
        // Nothing requested, nothing written.
        ReportFiles::default().write("replay", &summary).unwrap();
    }
}
