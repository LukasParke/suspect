//! Old flat command names, rewritten to the grouped tree before clap parses.
//!
//! A clap alias only works among siblings, and `diff`, `test` and `gen` are
//! both old leaves and new groups, so the rewrite happens on argv. The top
//! level takes no options before the subcommand, so `argv[1]` is always the
//! command name. The rewritten invocation prints one deprecation line on
//! stderr; stdout and the exit status are those of the new command.

use std::ffi::OsString;

use clap::CommandFactory;

/// One moved command.
#[derive(Debug, Clone, Copy)]
pub struct Renamed {
    /// The old top-level name.
    pub old: &'static str,
    /// The new command path.
    pub new: &'static [&'static str],
    /// Hidden flags appended so the new command reads the old arguments.
    pub extra: &'static [&'static str],
}

impl Renamed {
    /// The new invocation to suggest: the path plus the extra flags a
    /// user would type (hidden compatibility flags are left out).
    #[must_use]
    pub fn replacement(&self) -> String {
        let root = crate::Cli::command();
        let leaf = self
            .new
            .iter()
            .try_fold(&root, |cmd, name| cmd.find_subcommand(name));
        let visible = |flag: &&&str| {
            leaf.and_then(|cmd| {
                cmd.get_arguments()
                    .find(|a| a.get_long() == Some(flag.trim_start_matches("--")))
            })
            .is_some_and(|arg| !arg.is_hide_set())
        };
        let mut words: Vec<&str> = self.new.to_vec();
        words.extend(self.extra.iter().filter(visible));
        words.join(" ")
    }
}

const fn moved(old: &'static str, new: &'static [&'static str]) -> Renamed {
    Renamed {
        old,
        new,
        extra: &[],
    }
}

/// Every moved command, in the order of the command tree.
pub const RENAMED: &[Renamed] = &[
    moved("breaking", &["diff", "breaking"]),
    moved("arazzo-diff", &["diff", "breaking"]),
    moved("impact", &["diff", "impact"]),
    moved("codegen-compare", &["diff", "sdk"]),
    Renamed {
        old: "reverse",
        new: &["diff", "routes"],
        extra: &["--source-first"],
    },
    Renamed {
        old: "overlay-dry-run",
        new: &["overlay", "explain"],
        extra: &["--check"],
    },
    moved("docs", &["gen", "docs"]),
    moved("docs-gen", &["gen", "docs"]),
    moved("codegen", &["gen", "sdk"]),
    moved("stubs", &["gen", "stubs"]),
    moved("contract", &["gen", "contract"]),
    moved("codegen-terraform", &["gen", "terraform"]),
    moved("codegen-cli", &["gen", "cli"]),
    moved("codegen-mcp", &["gen", "mcp"]),
    moved("codegen-session", &["gen", "session"]),
    moved("bridge", &["gen", "bridge"]),
    moved("codegen-profiles", &["gen", "profiles"]),
    moved("admission", &["gen", "admission"]),
    moved("stateful", &["test", "stateful"]),
    moved("fuzz", &["test", "fuzz"]),
    moved("load", &["test", "load"]),
    moved("replay", &["test", "replay"]),
    moved("gateway", &["traffic", "gateway"]),
    moved("drift", &["traffic", "drift"]),
    moved("har", &["traffic", "har"]),
    moved("release-plan", &["release", "plan"]),
    moved("release-publish", &["release", "publish"]),
    moved("release-workflow", &["release", "workflow"]),
    moved("bench", &["dev", "bench"]),
    moved("evidence", &["dev", "evidence"]),
];

/// Groups whose old leaf form took positionals:
/// `suspect diff A B` -> `suspect diff structural A B`.
pub const DEFAULT_SUB: &[(&str, &str)] =
    &[("diff", "structural"), ("test", "run"), ("gen", "template")];

/// The release that drops the old names: two minor releases after this
/// one (before 1.0 a rename bumps the minor version).
#[must_use]
pub fn removed_in() -> String {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let major: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    format!("{major}.{}.0", minor + 2)
}

fn note(old: &str, new: &str) -> String {
    format!(
        "warning: `suspect {old}` is deprecated; use `suspect {new}` (the old name is removed in {})",
        removed_in()
    )
}

/// Rewrites an old invocation; returns the new argv and the stderr note
/// (`None` when argv already uses the new tree).
#[must_use]
pub fn rewrite(mut argv: Vec<OsString>) -> (Vec<OsString>, Option<String>) {
    let Some(first) = argv.get(1).and_then(|a| a.to_str()).map(str::to_owned) else {
        return (argv, None);
    };
    // `suspect help <old>` -> `suspect help <new...>`.
    if first == "help" {
        let Some(row) = argv
            .get(2)
            .and_then(|a| a.to_str())
            .and_then(|name| RENAMED.iter().find(|r| r.old == name))
        else {
            return (argv, None);
        };
        argv.splice(2..3, row.new.iter().map(OsString::from));
        let new = row.new.join(" ");
        return (
            argv,
            Some(note(&format!("help {}", row.old), &format!("help {new}"))),
        );
    }
    if let Some(row) = RENAMED.iter().find(|r| r.old == first) {
        let mut text = note(row.old, &row.replacement());
        if row.old == "overlay-dry-run" {
            for line in dry_run_target_notes(&argv[2..]) {
                text.push('\n');
                text.push_str(&line);
            }
        }
        argv.splice(1..2, row.new.iter().map(OsString::from));
        argv.extend(row.extra.iter().map(OsString::from));
        return (argv, Some(text));
    }
    if let Some((group, sub)) = DEFAULT_SUB.iter().find(|(g, _)| *g == first) {
        let Some(next) = argv.get(2).map(|a| a.to_string_lossy().into_owned()) else {
            return (argv, None);
        };
        if matches!(next.as_str(), "-h" | "--help" | "help") || is_subcommand(group, &next) {
            return (argv, None);
        }
        argv.insert(2, OsString::from(*sub));
        let text = note(&format!("{group} <args>"), &format!("{group} {sub} <args>"));
        return (argv, Some(text));
    }
    (argv, None)
}

/// One old flag spelling: the commands it applies to (`*` for any), the
/// old flag, and the replacement to name.
const OLD_FLAGS: &[(&[&str], &str, &str)] = &[
    (
        &[
            "gen template",
            "gen sdk",
            "gen stubs",
            "gen contract",
            "gen terraform",
            "gen cli",
            "gen mcp",
            "gen session",
        ],
        "--out",
        "--output",
    ),
    (&["test run", "test load"], "--report", "--format"),
    (&["test run", "test load"], "--inputs", "--inputs-file"),
    (
        &[
            "test run",
            "test stateful",
            "test fuzz",
            "test load",
            "test replay",
        ],
        "--json",
        "--report-file",
    ),
    (&["fmt", "gen contract"], "--json", "--to json"),
    (&["fmt"], "--yaml", "--to yaml"),
    (&["bundle", "upgrade"], "--format", "--to"),
    (&["traffic drift"], "--exit-on-gap", "--check"),
    (
        &["why"],
        "--spec",
        "the first positional (`why SPEC CONSTRAINT`)",
    ),
    (
        &["diff sdk"],
        "--before",
        "the first positional (`diff sdk OLD NEW`)",
    ),
    (
        &["diff sdk"],
        "--after",
        "the second positional (`diff sdk OLD NEW`)",
    ),
    (&["test run"], "--offline", "`--cassette` alone"),
];

/// One stderr note per old flag spelling (or old value) in an argv that
/// already uses the new command tree. Arguments after `--` belong to
/// another program and are not read.
#[must_use]
pub fn flag_notes(argv: &[OsString]) -> Vec<String> {
    let words: Vec<String> = argv
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let path = match words.get(1) {
        Some(second) if is_subcommand(first, second) => format!("{first} {second}"),
        _ => first.clone(),
    };
    let mut notes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let args: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .take_while(|a| *a != "--")
        .collect();
    for (i, arg) in args.iter().enumerate() {
        let (flag, value) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (*arg, args.get(i + 1).copied()),
        };
        if let Some((_, old, new)) = OLD_FLAGS
            .iter()
            .find(|(paths, old, _)| *old == flag && paths.contains(&path.as_str()))
            && seen.insert(*old)
        {
            notes.push(format!(
                "warning: `{old}` on `suspect {path}` is deprecated; use {new} (removed in {})",
                removed_in()
            ));
        }
        let stream = matches!(
            path.as_str(),
            "test run" | "test load" | "gen bridge" | "gen session"
        );
        if stream && flag == "--format" && value == Some("json") && seen.insert("--format json") {
            notes.push(format!(
                "warning: `--format json` on `suspect {path}` is deprecated; use `--format ndjson` (removed in {})",
                removed_in()
            ));
        }
        if path == "gen template"
            && flag == "--diff"
            && !args.contains(&"--check")
            && seen.insert("--diff")
        {
            notes.push(format!(
                "warning: `--diff` without `--check` on `suspect gen template` is deprecated; use `--check --diff` (removed in {})",
                removed_in()
            ));
        }
    }
    notes
}

/// Whether `name` is a subcommand (or alias) of `group`, read from the
/// clap tree so the rewrite cannot drift from it.
fn is_subcommand(group: &str, name: &str) -> bool {
    crate::Cli::command()
        .find_subcommand(group)
        .is_some_and(|g| g.find_subcommand(name).is_some())
}

/// `overlay-dry-run` normalized bare-dot targets that `overlay apply`
/// rejects; name each one the old command would have accepted.
fn dry_run_target_notes(args: &[OsString]) -> Vec<String> {
    let Some(overlay) = args.iter().find(|a| !a.to_string_lossy().starts_with('-')) else {
        return Vec::new();
    };
    let Ok(doc) = crate::load_doc(std::path::Path::new(overlay)) else {
        return Vec::new();
    };
    let json = suspect_overlay::Value::from_node(doc.root()).to_json();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
        return Vec::new();
    };
    let parses = |expr: &str| suspect_jsonpath::Path::parse(expr).is_ok();
    value
        .get("actions")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|action| action.get("target").and_then(|t| t.as_str()))
        .filter(|target| {
            !parses(target) && parses(&crate::commands::overlay_dry_run::normalize_target(target))
        })
        .map(|target| {
            format!("note: target \"{target}\" is not RFC 9535; overlay apply rejects it")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (Vec<String>, Option<String>) {
        let argv = std::iter::once("suspect")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect();
        let (argv, note) = rewrite(argv);
        (
            argv.into_iter()
                .skip(1)
                .map(|a| a.into_string().unwrap())
                .collect(),
            note,
        )
    }

    #[test]
    fn old_names_splice_the_new_path_and_extra_flags() {
        let (argv, note) = run(&["breaking", "a.yaml", "b.yaml"]);
        assert_eq!(argv, ["diff", "breaking", "a.yaml", "b.yaml"]);
        let note = note.unwrap();
        assert!(
            note.starts_with(
                "warning: `suspect breaking` is deprecated; use `suspect diff breaking`"
            )
        );
        let (argv, _) = run(&["reverse", "src", "spec.yaml"]);
        assert_eq!(
            argv,
            ["diff", "routes", "src", "spec.yaml", "--source-first"]
        );
    }

    #[test]
    fn old_leaves_get_their_default_subcommand() {
        assert_eq!(run(&["diff", "a", "b"]).0, ["diff", "structural", "a", "b"]);
        assert_eq!(
            run(&["test", "x.arazzo.yaml"]).0,
            ["test", "run", "x.arazzo.yaml"]
        );
        assert_eq!(
            run(&["gen", "--preset", "docs-md", "s.yaml"]).0,
            ["gen", "template", "--preset", "docs-md", "s.yaml"]
        );
    }

    #[test]
    fn new_invocations_pass_through_untouched() {
        for args in [
            &["diff", "breaking", "a", "b"][..],
            &["diff"],
            &["diff", "--help"],
            &["test", "-h"],
            &["gen", "sdk", "s.yaml"],
            &["lint", "a.yaml"],
            &["help", "diff"],
        ] {
            let (argv, note) = run(args);
            assert_eq!(argv, args, "{args:?}");
            assert!(note.is_none(), "{args:?}");
        }
    }

    #[test]
    fn help_for_an_old_name_shows_the_new_one() {
        let (argv, note) = run(&["help", "codegen"]);
        assert_eq!(argv, ["help", "gen", "sdk"]);
        assert!(note.is_some());
    }

    #[test]
    fn old_flag_spellings_get_one_note_each() {
        let notes = |args: &[&str]| {
            let argv: Vec<OsString> = std::iter::once("suspect")
                .chain(args.iter().copied())
                .map(OsString::from)
                .collect();
            flag_notes(&argv)
        };
        let out = notes(&["gen", "sdk", "s.yaml", "--out", "a", "--out=b"]);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(
            out[0].starts_with("warning: `--out` on `suspect gen sdk` is deprecated; use --output")
        );
        assert_eq!(notes(&["test", "run", "x", "--report", "ndjson"]).len(), 1);
        assert_eq!(notes(&["test", "run", "x", "--format", "json"]).len(), 1);
        assert_eq!(notes(&["gen", "template", "s", "--diff"]).len(), 1);
        assert!(notes(&["gen", "template", "s", "--check", "--diff"]).is_empty());
        assert!(notes(&["gen", "sdk", "s.yaml", "--output", "a"]).is_empty());
        // `--json` is an old spelling only where the command had it.
        assert!(notes(&["lint", "a.yaml", "--json"]).is_empty());
        // After `--` the arguments belong to the watched command.
        assert!(notes(&["watch", "src", "--", "suspect", "fmt", "--json"]).is_empty());
    }

    #[test]
    fn removal_is_two_minor_releases_out() {
        let mut parts = env!("CARGO_PKG_VERSION").split('.');
        let major = parts.next().unwrap();
        let minor: u64 = parts.next().unwrap().parse().unwrap();
        assert_eq!(removed_in(), format!("{major}.{}.0", minor + 2));
    }
}
