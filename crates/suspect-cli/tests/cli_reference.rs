//! Keeps `docs/CLI.md` in lockstep with the clap command tree.
//!
//! The reference is rendered from [`suspect_cli::Cli`] itself, so a new
//! command or flag that is not reflected in the checked-in file fails this
//! test. Regenerate with:
//!
//! ```text
//! SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference
//! ```

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

fn reference() -> String {
    let mut cmd = suspect_cli::Cli::command().term_width(100);
    cmd.build();
    let mut out = String::from(
        "# `suspect` command reference\n\n\
         <!-- Generated from the clap command tree by \
         crates/suspect-cli/tests/cli_reference.rs; do not edit by hand. \
         Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`. -->\n\n\
         Exit codes: `0` clean, `1` findings or failed checks, `2` usage or environment error.\n\n",
    );
    render(&mut cmd, "suspect", &mut out);
    out
}

#[test]
fn the_cli_reference_matches_the_command_tree() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/CLI.md");
    let expected = reference();
    if std::env::var_os("SUSPECT_BLESS").is_some() {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "docs/CLI.md is stale; regenerate with \
         `SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`"
    );
}
