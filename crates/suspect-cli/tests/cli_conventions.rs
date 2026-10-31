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
            && !arg.is_hide_set()
            && arg.get_short() != Some('o')
        {
            bad.push(format!("{path}: --output has no -o"));
        }
        for name in ["out", "json", "yaml"] {
            if let Some(arg) = long(cmd, name)
                && !arg.is_hide_set()
                && !allowed(path, name)
            {
                bad.push(format!("{path}: visible --{name}"));
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
        if long(cmd, "report").is_some_and(|arg| !arg.is_hide_set()) {
            bad.push(format!("{path}: visible --report"));
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
        let argv = argv.iter().map(Into::into).collect();
        let (argv, _) = suspect_cli::legacy::rewrite(argv);
        let err = suspect_cli::Cli::try_parse_from(&argv).expect_err("sarif must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue, "{argv:?}");
    }
}

/// Parses like the binary does: old names go through the legacy rewrite.
fn parse(argv: &[&str]) -> suspect_cli::Command {
    let argv: Vec<std::ffi::OsString> = std::iter::once("suspect")
        .chain(argv.iter().copied())
        .map(Into::into)
        .collect();
    let (argv, _) = suspect_cli::legacy::rewrite(argv);
    match suspect_cli::Cli::try_parse_from(&argv) {
        Ok(cli) => cli.command,
        Err(err) => panic!("{argv:?}: {err}"),
    }
}

/// The old spelling parses to exactly the value the new one does.
fn same(old: &[&str], new: &[&str]) {
    assert_eq!(
        format!("{:?}", parse(old)),
        format!("{:?}", parse(new)),
        "{old:?} vs {new:?}"
    );
}

#[test]
fn renamed_flags_keep_their_old_spellings() {
    // --out on every generator that now takes -o/--output.
    for (cmd, rest) in [
        ("gen", &["s.yaml", "--preset", "docs-md"][..]),
        ("codegen", &["s.yaml", "--package-name", "p"]),
        (
            "codegen-terraform",
            &["s.yaml", "--mapping", "m", "--target-config", "t"],
        ),
        (
            "codegen-cli",
            &["s.yaml", "--mapping", "m", "--target-config", "t"],
        ),
        (
            "codegen-mcp",
            &["s.yaml", "--mapping", "m", "--target-config", "t"],
        ),
        ("codegen-session", &["--config", "c.json"]),
        ("contract", &["s.yaml"]),
        ("stubs", &["s.yaml", "--backend", "rust-axum"]),
    ] {
        let mut old = vec![cmd];
        old.extend_from_slice(rest);
        let mut new = old.clone();
        old.extend(["--out", "dir"]);
        new.extend(["-o", "dir"]);
        same(&old, &new);
        new.splice(new.len() - 2..new.len() - 1, ["--output"]);
        same(&old, &new);
    }
    // --format json|yaml on bundle and upgrade is now --to.
    same(
        &["bundle", "a.yaml", "--format", "json"],
        &["bundle", "a.yaml", "--to", "json"],
    );
    same(
        &["upgrade", "a.yaml", "--format", "yaml"],
        &["upgrade", "a.yaml", "--to", "yaml"],
    );
    // upgrade --output gains -o.
    same(
        &["upgrade", "a.yaml", "--output", "b.yaml"],
        &["upgrade", "a.yaml", "-o", "b.yaml"],
    );
    // docs --output gains -o.
    same(
        &["docs", "a.yaml", "--output", "site"],
        &["docs", "a.yaml", "-o", "site"],
    );
    // drift --exit-on-gap is --check.
    same(
        &["drift", "a.yaml", "t.jsonl", "--exit-on-gap"],
        &["drift", "a.yaml", "t.jsonl", "--check"],
    );
    // load --json FILE is --report-file FILE.
    same(
        &["load", "p.yaml", "--json", "r.json"],
        &["load", "p.yaml", "--report-file", "r.json"],
    );
    // --inputs FILE is --inputs-file FILE on test and load.
    same(
        &["test", "a.yaml", "--inputs", "i.yaml"],
        &["test", "a.yaml", "--inputs-file", "i.yaml"],
    );
    same(
        &["load", "p.yaml", "--inputs", "i.yaml"],
        &["load", "p.yaml", "--inputs-file", "i.yaml"],
    );
    // --report on test and load is --format.
    same(
        &["test", "a.yaml", "--report", "ndjson"],
        &["test", "a.yaml", "--format", "ndjson"],
    );
    same(
        &["load", "p.yaml", "--report", "ndjson"],
        &["load", "p.yaml", "--format", "ndjson"],
    );
}

#[test]
fn old_switches_map_onto_the_new_flags() {
    use suspect_cli::{Command, DocFormat, GenCmd, TestCmd};
    // fmt --json/--yaml are hidden and conflict with --to.
    let Command::Fmt { json, out, .. } = parse(&["fmt", "a.yaml", "--json"]) else {
        panic!("fmt")
    };
    assert!(json && out.to.is_none());
    let Command::Fmt { yaml, .. } = parse(&["fmt", "a.yaml", "--yaml"]) else {
        panic!("fmt")
    };
    assert!(yaml);
    let Command::Fmt { out, .. } = parse(&["fmt", "a.yaml", "--to", "json"]) else {
        panic!("fmt")
    };
    assert_eq!(out.to, Some(DocFormat::Json));
    assert!(
        suspect_cli::Cli::try_parse_from(["suspect", "fmt", "a.yaml", "--json", "--to", "yaml"])
            .is_err()
    );
    // contract --json is the old --to json.
    let Command::Gen(GenCmd::Contract(args)) = parse(&["contract", "a.yaml", "--json"]) else {
        panic!("contract")
    };
    assert!(args.json);
    let Command::Gen(GenCmd::Contract(args)) = parse(&["contract", "a.yaml", "--to", "json"])
    else {
        panic!("contract")
    };
    assert_eq!(args.to, Some(DocFormat::Json));
    // gen --diff alone keeps working and implies --check.
    let Command::Gen(GenCmd::Template { diff, check, .. }) =
        parse(&["gen", "a.yaml", "--preset", "docs-md", "--diff"])
    else {
        panic!("gen")
    };
    assert!(diff && !check.check);
    // test --offline is a hidden no-op; --cassette no longer needs it.
    same(
        &[
            "test",
            "a.arazzo.yaml",
            "--offline",
            "--cassette",
            "c.jsonl",
        ],
        &[
            "test",
            "a.arazzo.yaml",
            "--cassette",
            "c.jsonl",
            "--offline",
        ],
    );
    let Command::Test(TestCmd::Run { cassette, .. }) =
        parse(&["test", "a.arazzo.yaml", "--cassette", "c.jsonl"])
    else {
        panic!("test")
    };
    assert!(cassette.is_some());
}

#[test]
fn gateway_mode_is_checked_by_clap() {
    use suspect_cli::commands::gateway::GatewayMode;
    let suspect_cli::Command::Traffic(suspect_cli::TrafficCmd::Gateway { mode, upstream, .. }) =
        parse(&[
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

#[test]
fn stream_commands_accept_the_old_spellings() {
    use suspect_cli::{Command, GenCmd, StreamFormat, TestCmd};
    // `json` is the old `bridge`/`codegen-session` spelling of `ndjson`.
    let Command::Gen(GenCmd::Bridge { stream, .. }) =
        parse(&["bridge", "a.yaml", "--format", "json"])
    else {
        panic!("bridge")
    };
    assert_eq!(stream.format, StreamFormat::Ndjson);
    // `--report` is the old `test`/`load` flag.
    let Command::Test(TestCmd::Run { format, .. }) =
        parse(&["test", "a.yaml", "--report", "ndjson"])
    else {
        panic!("test")
    };
    assert_eq!(format, Some(StreamFormat::Ndjson));
    let Command::Test(TestCmd::Load(args)) = parse(&["load", "p.yaml", "--report", "ndjson"])
    else {
        panic!("load")
    };
    assert_eq!(args.stream.format, StreamFormat::Ndjson);
}

/// The clap command at `path` (`["diff", "breaking"]`), if it exists.
fn find<'c>(root: &'c clap::Command, path: &[&str]) -> Option<&'c clap::Command> {
    path.iter()
        .try_fold(root, |cmd, name| cmd.find_subcommand(name))
}

#[test]
fn every_renamed_command_points_at_an_existing_path() {
    let root = suspect_cli::Cli::command();
    for row in suspect_cli::legacy::RENAMED {
        let target = find(&root, row.new)
            .unwrap_or_else(|| panic!("{} -> {:?} does not exist", row.old, row.new));
        assert!(
            target.get_subcommands().next().is_none(),
            "{} -> {:?} is a group, not a leaf",
            row.old,
            row.new
        );
        for flag in row.extra {
            let long = flag.trim_start_matches("--");
            assert!(
                target.get_arguments().any(|a| a.get_long() == Some(long)),
                "{} -> {:?}: extra flag {flag} does not exist",
                row.old,
                row.new
            );
        }
        // The old name is gone from the visible top level.
        assert!(
            root.get_subcommands()
                .all(|sub| sub.get_name() != row.old || sub.is_hide_set()),
            "{} is still a visible top-level command",
            row.old
        );
    }
    for (group, sub) in suspect_cli::legacy::DEFAULT_SUB {
        assert!(find(&root, &[group, sub]).is_some(), "{group} {sub}");
    }
}

#[test]
fn old_positional_orders_parse_to_the_new_commands() {
    use suspect_cli::{Command, DiffCmd};
    // why CONSTRAINT --spec SPEC == why SPEC CONSTRAINT.
    let Command::Why {
        first,
        constraint,
        spec,
        ..
    } = parse(&["why", "type", "--spec", "a.yaml"])
    else {
        panic!("why")
    };
    assert_eq!(
        (first.as_str(), constraint, spec),
        ("type", None, Some("a.yaml".into()))
    );
    let Command::Why {
        first, constraint, ..
    } = parse(&["why", "a.yaml", "type"])
    else {
        panic!("why")
    };
    assert_eq!(
        (first.as_str(), constraint.as_deref()),
        ("a.yaml", Some("type"))
    );
    // reverse SOURCE SPEC keeps its order behind --source-first.
    let Command::Diff(DiffCmd::Routes {
        spec,
        source,
        source_first,
        ..
    }) = parse(&["reverse", "src", "a.yaml"])
    else {
        panic!("reverse")
    };
    assert!(source_first);
    assert_eq!((spec, source), ("src".into(), "a.yaml".into()));
    // codegen-compare --before/--after fill the OLD NEW positionals.
    let Command::Diff(DiffCmd::Sdk(args)) =
        parse(&["codegen-compare", "--before", "a.json", "--after", "b.json"])
    else {
        panic!("codegen-compare")
    };
    assert_eq!(
        (args.before, args.after, args.old, args.new),
        (Some("a.json".into()), Some("b.json".into()), None, None)
    );
    let Command::Diff(DiffCmd::Sdk(args)) = parse(&["diff", "sdk", "a.json", "b.json"]) else {
        panic!("diff sdk")
    };
    assert_eq!(
        (args.old, args.new),
        (Some("a.json".into()), Some("b.json".into()))
    );
    assert!(
        suspect_cli::Cli::try_parse_from(["suspect", "diff", "sdk", "a.json", "--before", "b"])
            .is_err()
    );
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
