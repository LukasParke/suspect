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

/// `(command, flag, reason)` triples exempt from the rules below.
const ALLOWLIST: &[(&str, &str, &str)] = &[(
    "suspect test replay",
    "diff",
    "prints response-body diffs of a live replay; there is nothing to --check",
)];

fn allowed(path: &str, flag: &str) -> bool {
    ALLOWLIST.iter().any(|(p, f, _)| *p == path && *f == flag)
}

/// Runs `rule` on every leaf and fails with every violation it reports.
fn enforce(name: &str, rule: impl Fn(&str, &clap::Command) -> Vec<String>) {
    let bad: Vec<String> = leaves()
        .iter()
        .flat_map(|(path, cmd)| rule(path, cmd))
        .collect();
    assert!(bad.is_empty(), "{name}:\n{}", bad.join("\n"));
}

fn long<'c>(cmd: &'c clap::Command, name: &str) -> Option<&'c clap::Arg> {
    cmd.get_arguments().find(|a| a.get_long() == Some(name))
}

fn value_name(arg: &clap::Arg) -> Option<String> {
    arg.get_value_names()
        .and_then(|names| names.first())
        .map(ToString::to_string)
}

#[test]
fn format_flags_use_one_of_three_kinds() {
    const KINDS: [&[&str]; 3] = [
        &["text", "json", "sarif", "junit"],
        &["text", "json"],
        &["text", "ndjson"],
    ];
    enforce("--format outside the three kinds", |path, cmd| {
        let Some(arg) = long(cmd, "format") else {
            return Vec::new();
        };
        let values = values(arg);
        if allowed(path, "format") || KINDS.iter().any(|kind| *kind == values) {
            Vec::new()
        } else {
            vec![format!("{path}: --format {values:?}")]
        }
    });
}

#[test]
fn output_is_one_flag_with_a_short_form() {
    enforce("output flags", |path, cmd| {
        let mut bad = Vec::new();
        if let Some(arg) = long(cmd, "output")
            && arg.get_short() != Some('o')
        {
            bad.push(format!("{path}: --output has no -o"));
        }
        for name in ["out", "json", "yaml"] {
            if long(cmd, name).is_some() && !allowed(path, name) {
                bad.push(format!("{path}: --{name}"));
            }
        }
        bad
    });
}

#[test]
fn to_selects_json_or_yaml() {
    enforce("--to values", |path, cmd| match long(cmd, "to") {
        Some(arg) if values(arg) != ["json", "yaml"] => {
            vec![format!("{path}: --to {:?}", values(arg))]
        }
        _ => Vec::new(),
    });
}

#[test]
fn check_and_diff_are_switches_and_diff_needs_check() {
    enforce("--check/--diff", |path, cmd| {
        let mut bad = Vec::new();
        for name in ["check", "diff"] {
            if let Some(arg) = long(cmd, name)
                && !matches!(arg.get_action(), clap::ArgAction::SetTrue)
            {
                bad.push(format!("{path}: --{name} is not a switch"));
            }
        }
        if long(cmd, "diff").is_some() && long(cmd, "check").is_none() && !allowed(path, "diff") {
            bad.push(format!("{path}: --diff without --check"));
        }
        bad
    });
}

#[test]
fn report_files_take_a_file_and_report_is_not_a_flag() {
    enforce("report flags", |path, cmd| {
        let mut bad = Vec::new();
        if long(cmd, "report").is_some() {
            bad.push(format!("{path}: --report (use --format or --report-file)"));
        }
        for name in ["report-file", "junit", "journal", "inputs-file"] {
            if let Some(arg) = long(cmd, name)
                && value_name(arg).as_deref() != Some("FILE")
            {
                bad.push(format!("{path}: --{name} does not take FILE"));
            }
        }
        if long(cmd, "inputs").is_some() {
            bad.push(format!("{path}: --inputs (use --inputs-file)"));
        }
        bad
    });
}

#[test]
fn allowlist_has_no_stale_entries() {
    let leaves = leaves();
    for (path, flag, _) in ALLOWLIST {
        let (_, cmd) = leaves
            .iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("allowlisted command {path} does not exist"));
        assert!(
            long(cmd, flag).is_some(),
            "allowlisted {path} --{flag} no longer exists; drop the entry"
        );
    }
}

#[test]
fn sarif_is_rejected_by_clap_where_it_was_never_rendered() {
    for argv in [
        &["suspect", "ci", "--format", "sarif"][..],
        &[
            "suspect", "release", "plan", "a.yaml", "b.yaml", "--format", "sarif",
        ],
        &["suspect", "gen", "bridge", "a.yaml", "--format", "sarif"],
        &["suspect", "why", "a.yaml", "type", "--format", "sarif"],
    ] {
        let err = suspect_cli::Cli::try_parse_from(argv).expect_err("sarif must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue, "{argv:?}");
    }
}

/// Parses `suspect <argv>`.
fn parse(argv: &[&str]) -> suspect_cli::Command {
    let argv: Vec<&str> = std::iter::once("suspect")
        .chain(argv.iter().copied())
        .collect();
    match suspect_cli::Cli::try_parse_from(&argv) {
        Ok(cli) => cli.command,
        Err(err) => panic!("{argv:?}: {err}"),
    }
}

/// `suspect <argv>` is a clap usage error.
fn rejected(argv: &[&str]) {
    let argv: Vec<&str> = std::iter::once("suspect")
        .chain(argv.iter().copied())
        .collect();
    assert!(
        suspect_cli::Cli::try_parse_from(&argv).is_err(),
        "{argv:?} still parses"
    );
}

/// Hidden arguments that are not old spellings: limits for scripts and
/// tests that a user never types.
const HIDDEN: &[(&str, &str, &str)] = &[
    (
        "suspect gen bridge",
        "max-ticks",
        "stops the bridge after N ticks for scripts",
    ),
    (
        "suspect gen session",
        "max-iterations",
        "stops a watch after N iterations for process-level tests",
    ),
];

/// Value aliases that are synonyms, not old spellings.
const VALUE_SYNONYMS: &[(&str, &str, &str)] = &[
    (
        "min-severity",
        "information",
        "the config severity parser's name",
    ),
    ("min-severity", "warn", "the config severity parser's name"),
];

#[test]
fn no_command_flag_or_value_has_an_old_spelling() {
    let mut root = suspect_cli::Cli::command();
    root.build();
    let mut bad = Vec::new();
    fn commands(cmd: &clap::Command, path: String, bad: &mut Vec<String>) {
        if cmd.get_all_aliases().next().is_some()
            || cmd.get_all_short_flag_aliases().next().is_some()
        {
            bad.push(format!("{path}: command alias"));
        }
        if cmd.is_hide_set() {
            bad.push(format!("{path}: hidden command"));
        }
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            commands(sub, format!("{path} {}", sub.get_name()), bad);
        }
    }
    commands(&root, "suspect".to_owned(), &mut bad);
    for (path, cmd) in leaves() {
        for arg in cmd.get_arguments() {
            let name = arg.get_long().unwrap_or(arg.get_id().as_str());
            if arg.get_all_aliases().is_some() || arg.get_all_short_aliases().is_some() {
                bad.push(format!("{path} --{name}: flag alias"));
            }
            if arg.is_hide_set()
                && !["help", "version"].contains(&name)
                && !HIDDEN.iter().any(|(p, f, _)| *p == path && *f == name)
            {
                bad.push(format!("{path} --{name}: hidden flag"));
            }
            for value in arg.get_possible_values() {
                for alias in value.get_name_and_aliases().skip(1) {
                    if !VALUE_SYNONYMS
                        .iter()
                        .any(|(f, a, _)| *f == name && *a == alias)
                    {
                        bad.push(format!("{path} --{name}: value alias {alias}"));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "old spellings remain:\n{}", bad.join("\n"));
}

#[test]
fn removed_names_and_flags_are_usage_errors() {
    // The flat names the grouped tree replaced.
    for old in [
        "breaking",
        "arazzo-diff",
        "impact",
        "codegen-compare",
        "reverse",
        "overlay-dry-run",
        "docs",
        "docs-gen",
        "codegen",
        "stubs",
        "contract",
        "codegen-terraform",
        "codegen-cli",
        "codegen-mcp",
        "codegen-session",
        "bridge",
        "codegen-profiles",
        "admission",
        "stateful",
        "fuzz",
        "load",
        "replay",
        "gateway",
        "drift",
        "har",
        "release-plan",
        "release-publish",
        "release-workflow",
        "bench",
        "evidence",
    ] {
        rejected(&[old, "a.yaml", "b.yaml"]);
    }
    // Groups do not fall back to their old leaf.
    rejected(&["diff", "a.yaml", "b.yaml"]);
    rejected(&["test", "a.arazzo.yaml"]);
    rejected(&["gen", "a.yaml", "--preset", "docs-md"]);
    // Old flag spellings.
    rejected(&["gen", "sdk", "s.yaml", "--out", "dir"]);
    rejected(&["gen", "session", "--config", "c.json", "--format", "json"]);
    rejected(&["gen", "template", "s.yaml", "--preset", "docs-md", "--diff"]);
    rejected(&["gen", "contract", "a.yaml", "--json"]);
    rejected(&["fmt", "a.yaml", "--json"]);
    rejected(&["fmt", "a.yaml", "--yaml"]);
    rejected(&["bundle", "a.yaml", "--format", "json"]);
    rejected(&["upgrade", "a.yaml", "--format", "yaml"]);
    rejected(&["traffic", "drift", "a.yaml", "t.jsonl", "--exit-on-gap"]);
    rejected(&["test", "load", "p.yaml", "--json", "r.json"]);
    rejected(&["test", "load", "p.yaml", "--report", "ndjson"]);
    rejected(&["test", "run", "a.yaml", "--report", "ndjson"]);
    rejected(&["test", "run", "a.yaml", "--inputs", "i.yaml"]);
    rejected(&[
        "test",
        "run",
        "a.yaml",
        "--cassette",
        "c.jsonl",
        "--offline",
    ]);
    rejected(&["why", "type", "--spec", "a.yaml"]);
    rejected(&["diff", "routes", "src", "a.yaml", "--source-first"]);
    rejected(&["diff", "sdk", "--before", "a.json", "--after", "b.json"]);
}

#[test]
fn new_spellings_parse() {
    use suspect_cli::{Command, DiffCmd, DocFormat, GenCmd, StreamFormat, TestCmd};
    let Command::Fmt { out, .. } = parse(&["fmt", "a.yaml", "--to", "json"]) else {
        panic!("fmt")
    };
    assert_eq!(out.to, Some(DocFormat::Json));
    let Command::Gen(GenCmd::Contract(args)) =
        parse(&["gen", "contract", "a.yaml", "--to", "json", "-o", "pkg"])
    else {
        panic!("gen contract")
    };
    assert_eq!((args.to, args.out), (Some(DocFormat::Json), "pkg".into()));
    let Command::Gen(GenCmd::Template { diff, check, .. }) = parse(&[
        "gen", "template", "a.yaml", "--preset", "docs-md", "--check", "--diff",
    ]) else {
        panic!("gen template")
    };
    assert!(diff && check.check);
    let Command::Gen(GenCmd::Bridge { stream, .. }) =
        parse(&["gen", "bridge", "a.yaml", "--format", "ndjson"])
    else {
        panic!("gen bridge")
    };
    assert_eq!(stream.format, StreamFormat::Ndjson);
    let Command::Test(TestCmd::Run {
        format, cassette, ..
    }) = parse(&[
        "test",
        "run",
        "a.yaml",
        "--format",
        "ndjson",
        "--cassette",
        "c.jsonl",
    ])
    else {
        panic!("test run")
    };
    assert_eq!(format, Some(StreamFormat::Ndjson));
    assert!(cassette.is_some());
    let Command::Why {
        spec, constraint, ..
    } = parse(&["why", "a.yaml", "type"])
    else {
        panic!("why")
    };
    assert_eq!((spec, constraint.as_str()), ("a.yaml".into(), "type"));
    let Command::Diff(DiffCmd::Routes { spec, source, .. }) =
        parse(&["diff", "routes", "a.yaml", "src"])
    else {
        panic!("diff routes")
    };
    assert_eq!((spec, source), ("a.yaml".into(), "src".into()));
    let Command::Diff(DiffCmd::Sdk(args)) = parse(&["diff", "sdk", "a.json", "b.json"]) else {
        panic!("diff sdk")
    };
    assert_eq!((args.old, args.new), ("a.json".into(), "b.json".into()));
    // -o and --output are one flag on every generator.
    for words in [
        &["gen", "template", "s.yaml", "--preset", "docs-md"][..],
        &["gen", "sdk", "s.yaml", "--package-name", "p"],
        &["gen", "session", "--config", "c.json"],
        &["gen", "contract", "s.yaml"],
        &["gen", "stubs", "s.yaml", "--backend", "rust-axum"],
    ] {
        let mut short = words.to_vec();
        short.extend(["-o", "dir"]);
        let mut long = words.to_vec();
        long.extend(["--output", "dir"]);
        assert_eq!(
            format!("{:?}", parse(&short)),
            format!("{:?}", parse(&long)),
            "{words:?}"
        );
    }
}

#[test]
fn gateway_mode_is_checked_by_clap() {
    use suspect_cli::commands::gateway::GatewayMode;
    let suspect_cli::Command::Traffic(suspect_cli::TrafficCmd::Gateway { mode, upstream, .. }) =
        parse(&[
            "traffic",
            "gateway",
            "a.yaml",
            "--mode",
            "proxy",
            "--upstream",
            "http://localhost:9",
        ])
    else {
        panic!("gateway")
    };
    assert_eq!(mode, Some(GatewayMode::Proxy));
    assert_eq!(upstream.as_deref(), Some("http://localhost:9"));
    let err = suspect_cli::Cli::try_parse_from([
        "suspect", "traffic", "gateway", "a.yaml", "--mode", "mirror",
    ])
    .expect_err("unknown mode");
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

/// The clap command at `path` (`["diff", "breaking"]`), if it exists.
fn find<'c>(root: &'c clap::Command, path: &[&str]) -> Option<&'c clap::Command> {
    path.iter()
        .try_fold(root, |cmd, name| cmd.find_subcommand(name))
}

#[test]
fn every_leaf_documents_its_exit_statuses() {
    use suspect_cli::exit::DOCUMENTED;
    let documented: Vec<String> = DOCUMENTED
        .iter()
        .map(|(path, _, _)| format!("suspect {path}"))
        .collect();
    let missing: Vec<String> = leaves()
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| !documented.contains(path))
        .collect();
    assert!(
        missing.is_empty(),
        "no exit::DOCUMENTED entry for:\n{}",
        missing.join("\n")
    );
    // No stale or duplicate rows; a row may name a runnable group (`config`).
    let root = suspect_cli::Cli::command();
    for (i, (path, statuses, _)) in DOCUMENTED.iter().enumerate() {
        let words: Vec<&str> = path.split(' ').collect();
        assert!(find(&root, &words).is_some(), "{path} does not exist");
        assert!(
            DOCUMENTED[..i].iter().all(|(p, _, _)| p != path),
            "{path} is listed twice"
        );
        assert!(
            statuses.contains(&suspect_cli::Exit::Clean)
                && statuses.contains(&suspect_cli::Exit::Usage),
            "{path}: every command can exit 0 and 2"
        );
    }
}
