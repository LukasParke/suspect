//! Flag conventions every `suspect` command follows
//! (docs/proposals/P3-command-grammar.md, "Flag conventions").
//!
//! The test walks the clap tree, so a new command or flag that breaks a
//! convention fails here. An allowlist entry names a `(command path, flag)`
//! pair and why it is exempt.

use clap::{CommandFactory, Parser};

/// Every leaf command with its full path (`suspect overlay apply`).
fn leaves() -> Vec<(String, clap::Command)> {
    fn walk(cmd: &clap::Command, path: String, out: &mut Vec<(String, clap::Command)>) {
        let subs: Vec<_> = cmd
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
            .collect();
        if subs.is_empty() {
            out.push((path.clone(), cmd.clone()));
        }
        for sub in subs {
            walk(sub, format!("{path} {}", sub.get_name()), out);
        }
    }
    let mut cmd = suspect_cli::Cli::command();
    cmd.build();
    let mut out = Vec::new();
    walk(&cmd, "suspect".to_owned(), &mut out);
    out
}

fn values(arg: &clap::Arg) -> Vec<String> {
    arg.get_possible_values()
        .iter()
        .map(|v| v.get_name().to_owned())
        .collect()
}

/// `(command, flag)` pairs exempt from the `--format` rule, with reasons.
const FORMAT_ALLOWLIST: &[(&str, &str)] = &[
    // Document serialization; moves to `--to` in P3 step 3.
    ("suspect bundle", "format"),
    ("suspect upgrade", "format"),
];

#[test]
fn format_flags_use_one_of_three_kinds() {
    const KINDS: [&[&str]; 3] = [
        &["text", "json", "sarif", "junit"],
        &["text", "json"],
        &["text", "ndjson"],
    ];
    let mut bad = Vec::new();
    for (path, cmd) in leaves() {
        let Some(arg) = cmd.get_arguments().find(|a| a.get_long() == Some("format")) else {
            continue;
        };
        if FORMAT_ALLOWLIST.contains(&(path.as_str(), "format")) {
            continue;
        }
        let values = values(arg);
        if !KINDS.iter().any(|kind| *kind == values) {
            bad.push(format!("{path}: --format {values:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "--format outside the three kinds:\n{}",
        bad.join("\n")
    );
}

#[test]
fn format_allowlist_has_no_stale_entries() {
    let leaves = leaves();
    for (path, flag) in FORMAT_ALLOWLIST {
        let (_, cmd) = leaves
            .iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("allowlisted command {path} does not exist"));
        assert!(
            cmd.get_arguments().any(|a| a.get_long() == Some(flag)),
            "allowlisted {path} --{flag} no longer exists; drop the entry"
        );
    }
}

#[test]
fn sarif_is_rejected_by_clap_where_it_was_never_rendered() {
    for argv in [
        &["suspect", "check", "a.yaml", "--format", "sarif"][..],
        &["suspect", "ci", "--format", "sarif"],
        &[
            "suspect",
            "release-plan",
            "a.yaml",
            "b.yaml",
            "--format",
            "sarif",
        ],
        &["suspect", "bridge", "a.yaml", "--format", "sarif"],
        &[
            "suspect", "why", "type", "--spec", "a.yaml", "--format", "sarif",
        ],
    ] {
        let err = suspect_cli::Cli::try_parse_from(argv).expect_err("sarif must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue, "{argv:?}");
    }
}

#[test]
fn stream_commands_accept_the_old_spellings() {
    use suspect_cli::{Command, StreamFormat};
    // `json` is the old `bridge`/`codegen-session` spelling of `ndjson`.
    let cli = suspect_cli::Cli::parse_from(["suspect", "bridge", "a.yaml", "--format", "json"]);
    let Command::Bridge { stream, .. } = cli.command else {
        panic!("bridge")
    };
    assert_eq!(stream.format, StreamFormat::Ndjson);
    // `--report` is the old `test`/`load` flag.
    let cli = suspect_cli::Cli::parse_from(["suspect", "test", "a.yaml", "--report", "ndjson"]);
    let Command::Test { format, .. } = cli.command else {
        panic!("test")
    };
    assert_eq!(format, Some(StreamFormat::Ndjson));
    let cli = suspect_cli::Cli::parse_from(["suspect", "load", "p.yaml", "--report", "ndjson"]);
    let Command::Load(args) = cli.command else {
        panic!("load")
    };
    assert_eq!(args.stream.format, StreamFormat::Ndjson);
}
