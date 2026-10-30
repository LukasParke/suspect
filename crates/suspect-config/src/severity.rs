//! Severity names as they appear in committed configuration, environment
//! variables, and flags — parsed in one place so every consumer accepts
//! the same spellings.

/// A configured severity name, normalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeverityName {
    /// `error`.
    Error,
    /// `warning` or `warn`.
    Warning,
    /// `info` or `information`.
    Info,
    /// `hint`.
    Hint,
    /// `off`: drop the finding. Only meaningful where the caller supports
    /// silencing (per-rule and design-class overrides).
    Off,
}

impl SeverityName {
    /// The canonical lowercase spelling.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
            Self::Off => "off",
        }
    }
}

/// Parses a severity name: `error`, `warning`/`warn`, `info`/`information`,
/// `hint`, or `off`, case-insensitively and ignoring surrounding
/// whitespace. `None` for anything else; callers that do not support `off`
/// reject [`SeverityName::Off`] themselves.
#[must_use]
pub fn parse_severity(name: &str) -> Option<SeverityName> {
    match name.trim().to_ascii_lowercase().as_str() {
        "error" => Some(SeverityName::Error),
        "warning" | "warn" => Some(SeverityName::Warning),
        "info" | "information" => Some(SeverityName::Info),
        "hint" => Some(SeverityName::Hint),
        "off" => Some(SeverityName::Off),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_parses() {
        for (text, expected) in [
            ("error", SeverityName::Error),
            ("ERROR", SeverityName::Error),
            ("warning", SeverityName::Warning),
            ("warn", SeverityName::Warning),
            ("info", SeverityName::Info),
            (" information ", SeverityName::Info),
            ("hint", SeverityName::Hint),
            ("off", SeverityName::Off),
        ] {
            assert_eq!(parse_severity(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(parse_severity("fatal"), None);
        assert_eq!(parse_severity(""), None);
    }
}
