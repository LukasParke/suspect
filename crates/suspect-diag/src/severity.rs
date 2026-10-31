//! Severity ranks and the one parser for configured severity names.
//!
//! Committed configuration, environment variables, flags and editor
//! settings all go through [`parse_level`], so every consumer accepts the
//! same spellings.

/// How severe a finding is. Ordered so `>=` is "at least as severe"
/// (`Hint < Info < Warning < Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Stylistic note; never fails a run.
    Hint,
    /// Informational note.
    Info,
    /// Likely problem worth attention.
    Warning,
    /// Definite problem; fails the run.
    Error,
}

impl Severity {
    /// Every severity, least severe first.
    pub const ALL: [Self; 4] = [Self::Hint, Self::Info, Self::Warning, Self::Error];

    /// Lowercase label (`hint`, `info`, `warning`, `error`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Uppercase label for `[SEVERITY]` spans in text output.
    #[must_use]
    pub const fn upper(self) -> &'static str {
        match self {
            Self::Hint => "HINT",
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A configured level: a severity, or `off` (drop the finding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// Drop the finding.
    Off,
    /// Report at this severity.
    On(Severity),
}

impl Level {
    /// The canonical lowercase spelling.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On(severity) => severity.label(),
        }
    }

    /// The severity, or `None` for [`Level::Off`].
    #[must_use]
    pub const fn severity(self) -> Option<Severity> {
        match self {
            Self::Off => None,
            Self::On(severity) => Some(severity),
        }
    }
}

impl From<Severity> for Level {
    fn from(severity: Severity) -> Self {
        Self::On(severity)
    }
}

/// Parses a severity name: `error`, `warning`/`warn`, `info`/`information`,
/// `hint`, or `off`, case-insensitively and ignoring surrounding
/// whitespace. `None` for anything else; callers that do not support `off`
/// reject [`Level::Off`] themselves.
#[must_use]
pub fn parse_level(name: &str) -> Option<Level> {
    match name.trim().to_ascii_lowercase().as_str() {
        "error" => Some(Level::On(Severity::Error)),
        "warning" | "warn" => Some(Level::On(Severity::Warning)),
        "info" | "information" => Some(Level::On(Severity::Info)),
        "hint" => Some(Level::On(Severity::Hint)),
        "off" => Some(Level::Off),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_parses() {
        for (text, expected) in [
            ("error", Level::On(Severity::Error)),
            ("ERROR", Level::On(Severity::Error)),
            ("warning", Level::On(Severity::Warning)),
            ("warn", Level::On(Severity::Warning)),
            (" Warning ", Level::On(Severity::Warning)),
            ("info", Level::On(Severity::Info)),
            (" information ", Level::On(Severity::Info)),
            ("hint", Level::On(Severity::Hint)),
            ("off", Level::Off),
            ("OFF", Level::Off),
        ] {
            assert_eq!(parse_level(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        for text in ["fatal", "", "loud", "warnings", "0"] {
            assert_eq!(parse_level(text), None, "{text:?}");
        }
    }

    #[test]
    fn labels_round_trip() {
        for severity in Severity::ALL {
            assert_eq!(parse_level(severity.label()), Some(Level::On(severity)));
        }
        assert_eq!(parse_level(Level::Off.label()), Some(Level::Off));
    }

    #[test]
    fn order_is_ascending_by_severity() {
        assert!(Severity::Hint < Severity::Info);
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        let mut shuffled = [
            Severity::Warning,
            Severity::Error,
            Severity::Hint,
            Severity::Info,
        ];
        shuffled.sort();
        assert_eq!(shuffled, Severity::ALL);
    }

    #[test]
    fn severity_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Severity::Warning).unwrap(),
            "\"warning\""
        );
    }
}
