//! Severity ranks and the internal compiled-rule representation.

use suspect_jsonpath::Path;
use suspect_low::SpecFamily;

/// Finding severity, ordered by rank (`Error` > `Warn` > `Info` > `Hint`).
/// [`Severity::Off`] compares equal only to itself: it disables a rule and
/// takes no part in the partial order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Hard failure; the default for rules without an explicit severity.
    Error,
    /// Likely problem worth fixing.
    Warn,
    /// Informational observation.
    Info,
    /// Stylistic suggestion.
    Hint,
    /// Rule disabled; never matches and excluded from [`Severity`]'s
    /// ordering (compares equal only to itself).
    Off,
}

impl Severity {
    fn rank(self) -> Option<u8> {
        match self {
            Self::Error => Some(4),
            Self::Warn => Some(3),
            Self::Info => Some(2),
            Self::Hint => Some(1),
            Self::Off => None,
        }
    }

    pub(crate) fn from_text(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "hint" => Some(Self::Hint),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// Spectral numeric severities: `0`=error, `1`=warn, `2`=info, `3`=hint.
    pub(crate) fn from_number(n: i64) -> Option<Self> {
        match n {
            0 => Some(Self::Error),
            1 => Some(Self::Warn),
            2 => Some(Self::Info),
            3 => Some(Self::Hint),
            _ => None,
        }
    }

    pub(crate) fn is_enabled(self) -> bool {
        self != Self::Off
    }
}

impl PartialOrd for Severity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self.rank(), other.rank()) {
            (Some(a), Some(b)) => Some(a.cmp(&b)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Hint => "hint",
            Self::Off => "off",
        };
        f.write_str(text)
    }
}

/// Bit set over the specification families a rule applies to. Ruleset
/// `formats` strings map onto these; the engine picks one bit from
/// [`SpecFamily`] at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FamilySet(u8);

impl FamilySet {
    pub(crate) const OAS2: Self = Self(1);
    pub(crate) const OAS30: Self = Self(1 << 1);
    pub(crate) const OAS31: Self = Self(1 << 2);
    pub(crate) const OAS32: Self = Self(1 << 3);
    pub(crate) const OVERLAY: Self = Self(1 << 4);
    pub(crate) const ARAZZO: Self = Self(1 << 5);
    pub(crate) const UNKNOWN: Self = Self(1 << 6);

    /// Every family (used when a rule declares no `formats`).
    pub(crate) const ALL: Self = Self(u8::MAX);

    /// No family (empty set, built up by `union`).
    pub(crate) const NONE: Self = Self(0);

    pub(crate) const OAS3: Self = Self(Self::OAS30.0 | Self::OAS31.0 | Self::OAS32.0);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, family: SpecFamily) -> bool {
        let bit = match family {
            SpecFamily::Oas2 => Self::OAS2.0,
            SpecFamily::Oas30 => Self::OAS30.0,
            SpecFamily::Oas31 => Self::OAS31.0,
            SpecFamily::Oas32 => Self::OAS32.0,
            SpecFamily::Overlay10 => Self::OVERLAY.0,
            SpecFamily::Arazzo10 => Self::ARAZZO.0,
            SpecFamily::Unknown => Self::UNKNOWN.0,
        };
        self.0 & bit != 0
    }

    /// Maps a ruleset `formats` token; returns `None` for unknown tokens.
    pub(crate) fn from_format_token(token: &str) -> Option<Self> {
        match token {
            "oas2" => Some(Self::OAS2),
            "oas3" => Some(Self::OAS3),
            "oas3_0" | "oas3.0" => Some(Self::OAS30),
            "oas3_1" | "oas3.1" => Some(Self::OAS31),
            "oas3_2" | "oas3.2" => Some(Self::OAS32),
            "overlay" => Some(Self::OVERLAY),
            "arazzo" => Some(Self::ARAZZO),
            _ => None,
        }
    }
}

/// A compiled lint rule: parsed JSONPath queries plus a resolved `then`
/// function with its options.
#[derive(Debug)]
pub(crate) struct Rule {
    pub code: Box<str>,
    pub description: Option<Box<str>>,
    /// Original `given` expression texts (parallel to `given`); the fast
    /// path classifies queries from these.
    pub given_exprs: Vec<Box<str>>,
    pub given: Vec<Path>,
    pub then: crate::functions::Function,
    pub severity: Severity,
    pub formats: FamilySet,
}

impl Rule {
    /// Compiles a rule from already-validated parts. Static pack literals
    /// are trusted; a parse failure there is a bug in this crate.
    pub(crate) fn new(
        code: &str,
        description: &str,
        severity: Severity,
        formats: FamilySet,
        given: &[&str],
        then: crate::functions::Function,
    ) -> Self {
        let paths: Vec<Path> = given
            .iter()
            .map(|g| match Path::parse(g) {
                Ok(p) => p,
                Err(_) => unreachable!("builtin pack JSONPath must be valid: {g}"),
            })
            .collect();
        Self {
            code: code.into(),
            description: Some(description.into()),
            given_exprs: given.iter().map(|g| (*g).into()).collect(),
            given: paths,
            then,
            severity,
            formats,
        }
    }
}
