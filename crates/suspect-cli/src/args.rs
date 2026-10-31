//! Shared argument types: one `--format` enum per output kind, so clap
//! accepts only the values a command renders and rejects the rest (exit 2)
//! before any work starts.

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
