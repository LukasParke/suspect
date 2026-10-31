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

use Exit::{Clean, Findings, Interrupted, Usage};

/// Statuses of every command, rendered into `docs/CLI.md`: command path,
/// possible statuses, and what other than 0 means. Every non-server
/// command also exits 130 on Ctrl-C; the rows list `Interrupted` only
/// where the command drains and reports first. Servers exit 0 on Ctrl-C.
pub const DOCUMENTED: &[(&str, &[Exit], &str)] = &[
    (
        "check",
        &[Clean, Findings, Usage],
        "1: a document has an error-severity finding",
    ),
    (
        "validate",
        &[Clean, Findings, Usage],
        "1: an error-severity finding",
    ),
    (
        "lint",
        &[Clean, Findings, Usage],
        "1: an error-severity finding at or above the floor",
    ),
    (
        "fmt",
        &[Clean, Usage],
        "2: the input cannot be read or written",
    ),
    (
        "bundle",
        &[Clean, Findings, Usage],
        "1: an unresolved `$ref` or a load error",
    ),
    (
        "upgrade",
        &[Clean, Findings, Usage],
        "1: the upgraded document fails validation",
    ),
    ("stats", &[Clean, Usage], "2: the input cannot be read"),
    (
        "overlay apply",
        &[Clean, Usage],
        "2: a malformed overlay or an invalid action",
    ),
    (
        "overlay explain",
        &[Clean, Findings, Usage],
        "1 (`--check`): an action matches nothing",
    ),
    (
        "overlay diff",
        &[Clean, Usage],
        "2: an input cannot be read",
    ),
    (
        "diff structural",
        &[Clean, Findings, Usage],
        "1 (`--check`): anything differs",
    ),
    (
        "diff breaking",
        &[Clean, Findings, Usage],
        "1: any break, warnings included",
    ),
    (
        "diff impact",
        &[Clean, Findings, Usage],
        "1: a recorded exchange breaks against NEW",
    ),
    (
        "diff sdk",
        &[Clean, Findings, Usage],
        "1: compatibility not proven; 2: input or configuration error",
    ),
    (
        "diff routes",
        &[Clean, Findings, Usage],
        "1 (`--check`): undocumented or spec-only routes; 2: no routes extracted",
    ),
    (
        "gen docs",
        &[Clean, Usage],
        "2: the spec cannot be loaded or the output written",
    ),
    (
        "gen sdk",
        &[Clean, Findings, Usage],
        "1: refused, or (`--check`) the output drifted",
    ),
    (
        "gen stubs",
        &[Clean, Usage],
        "2: the spec cannot be loaded or the output written",
    ),
    (
        "gen contract",
        &[Clean, Findings, Usage],
        "1 (`--check`): the package drifted",
    ),
    (
        "gen terraform",
        &[Clean, Findings, Usage],
        "1: refused, or (`--check`) the output drifted",
    ),
    (
        "gen cli",
        &[Clean, Findings, Usage],
        "1: refused, or (`--check`) the output drifted",
    ),
    (
        "gen mcp",
        &[Clean, Findings, Usage],
        "1: refused, or (`--check`) the output drifted",
    ),
    (
        "gen template",
        &[Clean, Findings, Usage],
        "1 (`--check`/`--diff`): output or ownership drifted",
    ),
    (
        "gen session",
        &[Clean, Findings, Usage],
        "1: the session record is unsuccessful; 0 on Ctrl-C with `--watch`",
    ),
    ("gen bridge", &[Clean, Usage], "server: 0 on Ctrl-C"),
    (
        "gen profiles",
        &[Clean, Usage],
        "always 0 unless output fails",
    ),
    (
        "gen admission",
        &[Clean, Findings, Usage],
        "1: an error-severity admission finding",
    ),
    (
        "test run",
        &[Clean, Findings, Usage],
        "1: a workflow failed; 2: nothing to run (`--filter` matched none)",
    ),
    (
        "test stateful",
        &[Clean, Findings, Usage],
        "1: a sequence failed; 2: nothing to run (`--filter` matched none)",
    ),
    (
        "test fuzz",
        &[Clean, Findings, Usage],
        "1: a mutant crashed the server; 2: nothing to run (`--filter` matched none)",
    ),
    (
        "test load",
        &[Clean, Findings, Usage, Interrupted],
        "1: an `ensure` gate failed; 130: interrupted before any request",
    ),
    (
        "test replay",
        &[Clean, Findings, Usage],
        "1: a replayed exchange drifted",
    ),
    ("traffic gateway", &[Clean, Usage], "server: 0 on Ctrl-C"),
    (
        "traffic drift",
        &[Clean, Findings, Usage],
        "1 (`--check`): undeclared endpoints or parameters",
    ),
    (
        "traffic har",
        &[Clean, Usage],
        "2: the traffic file cannot be read",
    ),
    (
        "release plan",
        &[Clean, Usage],
        "2: an input cannot be loaded",
    ),
    (
        "release publish",
        &[Clean, Findings, Usage],
        "1: preflight blocked or a publish step failed",
    ),
    (
        "release workflow",
        &[Clean, Usage],
        "2: the release manifest is invalid",
    ),
    (
        "project init",
        &[Clean, Usage],
        "2: the manifest cannot be written",
    ),
    (
        "project check",
        &[Clean, Findings, Usage],
        "1: an error-severity finding",
    ),
    (
        "project build",
        &[Clean, Findings, Usage],
        "1: a build stage failed",
    ),
    (
        "ci",
        &[Clean, Findings, Usage],
        "1: a stage failed; 2: no project found",
    ),
    (
        "why",
        &[Clean, Usage],
        "2: the spec or git history cannot be read",
    ),
    (
        "rules check",
        &[Clean, Findings, Usage],
        "1: an error-severity finding; 2: Bun is unavailable",
    ),
    (
        "rules new",
        &[Clean, Usage],
        "2: the rule cannot be written",
    ),
    ("rules bench", &[Clean, Usage], "2: Bun is unavailable"),
    (
        "config",
        &[Clean, Usage],
        "2: a configuration file or setting is invalid",
    ),
    (
        "config migrate",
        &[Clean, Usage],
        "2: the manifest cannot be migrated",
    ),
    (
        "auth check",
        &[Clean, Findings, Usage],
        "1: a security scheme does not resolve",
    ),
    (
        "acquire",
        &[Clean, Findings, Usage, Interrupted],
        "1: acquisition or verification failed; 130: cancelled",
    ),
    ("watch", &[Clean, Usage], "server: 0 on Ctrl-C"),
    ("lsp", &[Clean, Usage], "server: 0 on Ctrl-C"),
    (
        "dev bench",
        &[Clean, Usage],
        "2: the fixture cannot be loaded",
    ),
    (
        "dev evidence",
        &[Clean, Findings, Usage],
        "1: a declared feature has no evidence",
    ),
];

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
