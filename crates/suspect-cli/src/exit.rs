//! Typed process status for every `suspect` command, and the one sanctioned
//! immediate process exit.

/// Process status for every `suspect` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Nothing to report, or the gate passed.
    Clean,
    /// Findings at or above the command's floor, failed tests, drift, or
    /// an incompatible comparison.
    Findings,
    /// Usage, configuration, input or environment error.
    Usage,
    /// Stopped by SIGINT before the work completed.
    Interrupted,
}

impl Exit {
    /// The process status: 0, 1, 2 or 130.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Clean => 0,
            Self::Findings => 1,
            Self::Usage => 2,
            Self::Interrupted => 130,
        }
    }

    /// `Findings` when `failed`, else `Clean`.
    #[must_use]
    pub const fn failed_if(failed: bool) -> Self {
        if failed { Self::Findings } else { Self::Clean }
    }

    /// Transitional, while command functions still return `i32`: 0, 1 and
    /// 130 keep their meaning, anything else is a usage error.
    #[must_use]
    pub const fn from_legacy(code: i32) -> Self {
        match code {
            0 => Self::Clean,
            1 => Self::Findings,
            130 => Self::Interrupted,
            _ => Self::Usage,
        }
    }
}

impl From<Exit> for std::process::ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}

/// The one sanctioned immediate exit: a second Ctrl-C, or a first Ctrl-C
/// in a command that did not opt into graceful shutdown. Skips destructors.
#[allow(clippy::disallowed_methods)]
pub fn abort(exit: Exit) -> ! {
    std::process::exit(i32::from(exit.code()))
}

#[cfg(test)]
mod tests {
    use super::Exit;

    #[test]
    fn codes_are_the_documented_statuses() {
        assert_eq!(Exit::Clean.code(), 0);
        assert_eq!(Exit::Findings.code(), 1);
        assert_eq!(Exit::Usage.code(), 2);
        assert_eq!(Exit::Interrupted.code(), 130);
    }

    #[test]
    fn failed_if_picks_findings_or_clean() {
        assert_eq!(Exit::failed_if(true), Exit::Findings);
        assert_eq!(Exit::failed_if(false), Exit::Clean);
    }

    #[test]
    fn legacy_codes_round_trip_and_unknown_codes_are_usage() {
        for exit in [Exit::Clean, Exit::Findings, Exit::Usage, Exit::Interrupted] {
            assert_eq!(Exit::from_legacy(i32::from(exit.code())), exit);
        }
        assert_eq!(Exit::from_legacy(3), Exit::Usage);
        assert_eq!(Exit::from_legacy(-1), Exit::Usage);
    }
}
