//! Severity names as they appear in committed configuration, environment
//! variables, and flags. The parser lives in `suspect-diag`
//! ([`suspect_diag::parse_level`]); this module re-exports it and keeps the
//! older [`SeverityName`] / [`parse_severity`] names working on top of it.

pub use suspect_diag::{Level, Severity, parse_level};

/// A configured severity name, normalized. Older shape of
/// [`suspect_diag::Level`]; new code should use [`parse_level`].
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
        Level::from(self).label()
    }
}

impl From<Level> for SeverityName {
    fn from(level: Level) -> Self {
        match level {
            Level::Off => Self::Off,
            Level::On(Severity::Error) => Self::Error,
            Level::On(Severity::Warning) => Self::Warning,
            Level::On(Severity::Info) => Self::Info,
            Level::On(Severity::Hint) => Self::Hint,
        }
    }
}

impl From<SeverityName> for Level {
    fn from(name: SeverityName) -> Self {
        match name {
            SeverityName::Off => Self::Off,
            SeverityName::Error => Self::On(Severity::Error),
            SeverityName::Warning => Self::On(Severity::Warning),
            SeverityName::Info => Self::On(Severity::Info),
            SeverityName::Hint => Self::On(Severity::Hint),
        }
    }
}

/// Parses a severity name through [`parse_level`]: `error`,
/// `warning`/`warn`, `info`/`information`, `hint`, or `off`,
/// case-insensitively and ignoring surrounding whitespace. `None` for
/// anything else; callers that do not support `off` reject
/// [`SeverityName::Off`] themselves.
#[must_use]
pub fn parse_severity(name: &str) -> Option<SeverityName> {
    parse_level(name).map(SeverityName::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_follow_the_shared_parser() {
        for (text, expected) in [
            ("error", SeverityName::Error),
            ("warn", SeverityName::Warning),
            (" information ", SeverityName::Info),
            ("hint", SeverityName::Hint),
            ("off", SeverityName::Off),
        ] {
            assert_eq!(parse_severity(text), Some(expected), "{text}");
            assert_eq!(parse_level(text).map(SeverityName::from), Some(expected));
            assert_eq!(expected.label(), Level::from(expected).label());
        }
        assert_eq!(parse_severity("fatal"), None);
    }
}
