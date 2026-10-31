//! Keeps the generated pages under `docs/reference/` in lockstep with
//! their sources. Each page has one render function and one test that
//! compares it with the checked-in file:
//!
//! - `cli.md` from the clap command tree ([`suspect_cli::Cli`]), so a new
//!   command or flag that is not reflected in the file fails here.
//!
//! Regenerate every page with:
//!
//! ```text
//! SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs
//! ```
//!
//! (`cargo xtask bless` runs this and the other bless tests.)

use std::fmt::Write as _;
use std::path::Path;

use clap::CommandFactory;

/// Renders one command and, depth first, every visible subcommand.
fn render(cmd: &mut clap::Command, path: &str, out: &mut String) {
    let help = cmd.render_help().to_string();
    writeln!(out, "## `{path}`\n").unwrap();
    writeln!(out, "```text\n{}\n```\n", help.trim_end()).unwrap();
    let names: Vec<String> = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .map(|sub| sub.get_name().to_owned())
        .collect();
    for name in names {
        let sub = cmd.find_subcommand_mut(&name).expect("listed subcommand");
        render(sub, &format!("{path} {name}"), out);
    }
}

fn cli_page() -> String {
    let mut cmd = suspect_cli::Cli::command().term_width(100);
    cmd.build();
    let mut out = String::from(
        "# `suspect` command reference\n\n\
         <!-- Generated from the clap command tree by \
         crates/suspect-cli/tests/reference_docs.rs; do not edit by hand. \
         Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`. -->\n\n\
         Exit codes: `0` clean, `1` findings or failed checks, `2` usage or environment error, `130` interrupted (long-running servers exit `0` on Ctrl-C); per command: [Exit status](#exit-status).\n\n",
    );
    render(&mut cmd, "suspect", &mut out);
    exit_statuses(&mut out);
    renamed(&mut out);
    out
}

/// The per-command exit table from `exit::DOCUMENTED`.
fn exit_statuses(out: &mut String) {
    writeln!(out, "## Exit status\n").unwrap();
    writeln!(
        out,
        "`0` clean, `1` findings or failed checks, `2` usage or environment \
         error, `130` interrupted. Every command that is not a server also \
         exits `130` on Ctrl-C; the table lists `130` only where the command \
         stops gracefully and reports first. Servers exit `0` on Ctrl-C.\n"
    )
    .unwrap();
    writeln!(out, "| Command | Status | Meaning |\n|---|---|---|").unwrap();
    for (path, statuses, meaning) in suspect_cli::exit::DOCUMENTED {
        let codes: Vec<String> = statuses
            .iter()
            .map(|status| format!("`{}`", status.code()))
            .collect();
        writeln!(
            out,
            "| `suspect {path}` | {} | {meaning} |",
            codes.join(", ")
        )
        .unwrap();
    }
    out.push('\n');
}

/// The legacy rewrite table: old names keep working until they are removed.
fn renamed(out: &mut String) {
    use suspect_cli::legacy::{DEFAULT_SUB, RENAMED, removed_in};
    writeln!(out, "## Renamed commands\n").unwrap();
    writeln!(
        out,
        "The old names still run the new command and print one deprecation \
         line on stderr; stdout and the exit status are unchanged. They are \
         removed in {}. Under `diff`, `test` and `gen`, a first argument named \
         like a subcommand (`suspect diff breaking other.yaml`) runs that \
         subcommand; write `./breaking` for a file.\n",
        removed_in()
    )
    .unwrap();
    writeln!(out, "| Old | New | Note |\n|---|---|---|").unwrap();
    for row in RENAMED {
        let note = match row.old {
            "arazzo-diff" => "the family of NEW picks the differ",
            "reverse" => "positionals are `SPEC SOURCE`",
            "codegen-compare" => "`--before`/`--after` are positionals `OLD NEW`",
            "overlay-dry-run" => "targets `overlay apply` rejects are rejected",
            _ => "",
        };
        writeln!(
            out,
            "| `suspect {}` | `suspect {}` | {note} |",
            row.old,
            row.replacement()
        )
        .unwrap();
    }
    for (group, sub) in DEFAULT_SUB {
        writeln!(
            out,
            "| `suspect {group} <args>` | `suspect {group} {sub} <args>` | |"
        )
        .unwrap();
    }
    writeln!(
        out,
        "| `suspect why CONSTRAINT --spec SPEC` | `suspect why SPEC CONSTRAINT` | |"
    )
    .unwrap();
}

/// Compares `rendered` with `docs/reference/<page>`, or writes it when
/// `SUSPECT_BLESS` is set.
fn bless(page: &str, rendered: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/reference")
        .join(page);
    if std::env::var_os("SUSPECT_BLESS").is_some() {
        std::fs::write(&path, rendered).unwrap();
        return;
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == rendered,
        "docs/reference/{page} is stale; regenerate with \
         `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`"
    );
}

#[test]
fn the_cli_reference_matches_the_command_tree() {
    bless("cli.md", &cli_page());
}
