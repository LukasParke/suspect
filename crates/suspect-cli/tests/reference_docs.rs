//! Keeps the generated pages under `docs/reference/` in lockstep with
//! their sources. Each page has one render function and one test that
//! compares it with the checked-in file:
//!
//! - `cli.md` from the clap command tree ([`suspect_cli::Cli`]), so a new
//!   command or flag that is not reflected in the file fails here.
//! - `diagnostics.md` from the `suspect-check` rule registry.
//! - `config.md` from the `suspect_config::schema` setting tables.
//! - `environment.md` from the same tables' `env` column and
//!   `schema::DEBUG_VARS`.
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

/// One Markdown table cell: pipes escaped, line breaks folded.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// A list of names as backticked, comma-separated code spans.
fn codes<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    names
        .into_iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn generated_note(source: &str) -> String {
    format!(
        "<!-- Generated from {source} by crates/suspect-cli/tests/reference_docs.rs; \
         do not edit by hand. Regenerate with \
         `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`. -->\n\n"
    )
}

fn class_name(class: suspect_diag::RuleClass) -> &'static str {
    match class {
        suspect_diag::RuleClass::Document => "document",
        suspect_diag::RuleClass::Design => "design",
        suspect_diag::RuleClass::Sdk => "sdk",
    }
}

/// `docs/reference/diagnostics.md`: every builtin code, grouped by check.
fn diagnostics_page() -> String {
    let registry = suspect_check::registry::builtin();
    let mut out = String::from("# Diagnostic codes\n\n");
    out.push_str(&generated_note("the suspect-check rule registry"));
    out.push_str(
        "Every code a builtin check reports, with the check that owns it, its \
         default severity, its class and its aliases. `lint.rules.<code>` in \
         `.suspect.yaml` overrides a severity (or turns a code `off`), \
         `lint.design` sets every design-class code at once, and SDK-class \
         codes appear only when the project configures code generation. An \
         alias names the same problem under another code (usually the Spectral \
         rule name); it is accepted wherever a code is, and findings report the \
         canonical code. See the [linting guide](../guide/linting.md) for \
         settings, suppression comments and baselines.\n\n",
    );
    let mut checks: Vec<suspect_diag::CheckId> = registry.rules().map(|r| r.check).collect();
    checks.sort();
    checks.dedup();
    let links: Vec<String> = checks
        .iter()
        .map(|c| format!("[`{0}`](#{0})", c.name()))
        .collect();
    writeln!(out, "Checks: {}.\n", links.join(", ")).unwrap();
    for check in checks {
        writeln!(out, "## {}\n", check.name()).unwrap();
        writeln!(
            out,
            "| Code | Default | Class | Summary | How to fix | Aliases |\n\
             |---|---|---|---|---|---|"
        )
        .unwrap();
        let mut rules: Vec<&suspect_diag::RuleInfo> =
            registry.rules().filter(|r| r.check == check).collect();
        rules.sort_by_key(|r| r.code);
        for rule in rules {
            let targets = registry.alias_targets(rule.code);
            let aliases = if targets.is_empty() {
                codes(rule.aliases.iter().copied())
            } else {
                format!("alias of {}", codes(targets.iter().copied()))
            };
            writeln!(
                out,
                "| `{}` | {} | {} | {} | {} | {} |",
                rule.code,
                rule.default.label(),
                class_name(rule.class),
                cell(rule.summary),
                cell(rule.how_to_fix),
                aliases
            )
            .unwrap();
        }
        out.push('\n');
    }
    let families = registry.families();
    if !families.is_empty() {
        out.push_str(
            "## Code families\n\n\
             Open-ended codes registered by prefix: a code no table lists takes the \
             metadata of the family with the longest matching prefix.\n\n\
             | Family | Check | Default | Class | Summary |\n|---|---|---|---|---|\n",
        );
        for family in families {
            writeln!(
                out,
                "| `{}` | {} | {} | {} | {} |",
                family.rule.code,
                family.rule.check.name(),
                family.rule.default.label(),
                class_name(family.rule.class),
                cell(family.rule.summary)
            )
            .unwrap();
        }
        out.push('\n');
    }
    out
}

/// A setting's flag as the setting table writes it (`traffic gateway
/// --port`), with the `suspect` prefix.
///
/// # Panics
/// When the command or the flag does not exist, so a renamed flag cannot
/// leave the page pointing at nothing.
fn current_flag(flag: &str) -> String {
    let (command, long) = flag.rsplit_once(' ').expect("`<command> --<flag>`");
    let words: Vec<&str> = command.split(' ').collect();
    let root = suspect_cli::Cli::command();
    let leaf = words
        .iter()
        .try_fold(&root, |cmd, name| cmd.find_subcommand(name))
        .unwrap_or_else(|| panic!("setting flag `{flag}`: no command `{}`", words.join(" ")));
    let name = long.trim_start_matches("--");
    let known = leaf.get_arguments().any(|arg| arg.get_long() == Some(name));
    assert!(
        known,
        "setting flag `{flag}`: `suspect {}` has no `{long}`",
        words.join(" ")
    );
    format!("suspect {} {long}", words.join(" "))
}

/// The type of a setting as the reference shows it.
fn kind_text(kind: suspect_config::schema::Kind) -> String {
    use suspect_config::schema::Kind;
    let (name, values) = match kind {
        Kind::Map(inner) => (format!("map of {}", inner.type_name()), inner.values()),
        other => (other.type_name().to_owned(), other.values()),
    };
    if values.is_empty() {
        name
    } else {
        format!("{name}: {}", codes(values.iter().copied()))
    }
}

/// A default as YAML in a code span, or as prose ("the directory name").
fn default_text(setting: &suspect_config::schema::Setting) -> String {
    match setting.default {
        None => String::new(),
        Some(d) if d.contains(' ') && !d.starts_with(['[', '{', '"']) => cell(d),
        Some(d) => format!("`{}`", cell(d)),
    }
}

/// One settings table, one subsection per top-level key.
fn settings_table(out: &mut String, table: &'static [suspect_config::schema::Setting]) {
    use suspect_config::schema::Kind;
    let mut open = false;
    for setting in table.iter().filter(|s| !s.path.is_empty()) {
        if !setting.path.contains('.') {
            if open {
                out.push('\n');
            }
            writeln!(out, "### `{}`\n", setting.path).unwrap();
            open = false;
            if setting.kind == Kind::Section {
                writeln!(out, "{}\n", setting.doc).unwrap();
                continue;
            }
        }
        if setting.kind == Kind::Section {
            // A nested section: its rows follow with full paths.
            continue;
        }
        if !open {
            out.push_str(
                "| Key | Type | Default | Description | Env | Editor setting | Flags |\n\
                 |---|---|---|---|---|---|---|\n",
            );
            open = true;
        }
        let editor = setting.editor.map_or_else(String::new, |key| {
            let mut text = format!("`suspect.{key}`");
            if !setting.editor_aliases.is_empty() {
                let old: Vec<String> = setting
                    .editor_aliases
                    .iter()
                    .map(|a| format!("suspect.{a}"))
                    .collect();
                write!(text, " (was {})", codes(old.iter().map(String::as_str))).unwrap();
            }
            text
        });
        let flags: Vec<String> = setting.flags.iter().map(|f| current_flag(f)).collect();
        writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            setting.path,
            cell(&kind_text(setting.kind)),
            default_text(setting),
            cell(setting.doc),
            setting.env.map_or_else(String::new, |e| format!("`{e}`")),
            editor,
            codes(flags.iter().map(String::as_str)),
        )
        .unwrap();
    }
    if open {
        out.push('\n');
    }
}

/// A table of nested object fields (codegen targets, release entries).
fn fields_table(out: &mut String, fields: &'static [suspect_config::schema::Setting]) {
    out.push_str("| Field | Type | Default | Description |\n|---|---|---|---|\n");
    for field in fields {
        writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            field.path,
            cell(&kind_text(field.kind)),
            default_text(field),
            cell(field.doc)
        )
        .unwrap();
    }
    out.push('\n');
}

/// `docs/reference/config.md`: both configuration files, key by key.
fn config_page() -> String {
    use suspect_config::schema;
    let mut out = String::from("# Configuration reference\n\n");
    out.push_str(&generated_note("the suspect_config::schema setting tables"));
    out.push_str(
        "suspect reads two files. `.suspect.yaml` holds the settings every \
         command, CI and the editor share. `suspect.project.json` describes what \
         a project builds. Both are validated against checked-in JSON Schemas \
         generated from the same tables: \
         [`suspect.schema.json`](../../editors/vscode/schemas/suspect.schema.json) \
         and [`suspect.project.schema.json`](../../editors/vscode/schemas/suspect.project.schema.json). \
         Point an editor's YAML or JSON schema setting at them; the VS Code \
         extension does this already.\n\n\
         Precedence, lowest first: the default, `.suspect.yaml` (over its \
         `extends` parent), \
         the environment variable, then an editor setting or a command-line \
         flag. `lint.rules` merges per rule across layers. Paths are relative \
         to the file that sets them. `suspect config --explain` lists every key \
         with its value and where it came from. A project manifest's old \
         `lint`, `validate` and `editor` sections are ignored with a warning \
         (`suspect config migrate` moves them here). Environment variables are \
         also listed in the [environment reference](environment.md).\n\n",
    );
    writeln!(out, "## `.suspect.yaml`\n\n{}\n", schema::SETTINGS[0].doc).unwrap();
    settings_table(&mut out, schema::SETTINGS);
    writeln!(
        out,
        "## `suspect.project.json`\n\n{}\n",
        schema::MANIFEST[0].doc
    )
    .unwrap();
    out.push_str(
        "Settings that moved to `.suspect.yaml` (`lint`, `validate`, `editor`) are \
         ignored here, with a warning, and fail `suspect project check`; \
         `suspect config migrate` moves them.\n\n",
    );
    fields_table(&mut out, &schema::MANIFEST[1..]);
    out.push_str("### `codegen` targets\n\nEach entry of the manifest's `codegen` list.\n\n");
    fields_table(&mut out, schema::TARGET_FIELDS);
    out.push_str("### `release` fields\n\n");
    fields_table(&mut out, schema::RELEASE_FIELDS);
    out.push_str("### `release.targets` entries\n\n");
    fields_table(&mut out, schema::RELEASE_TARGET_FIELDS);
    out
}

/// `docs/reference/environment.md`: every `SUSPECT_*` variable the binary
/// reads.
fn environment_page() -> String {
    use suspect_config::schema;
    let mut out = String::from("# Environment variables\n\n");
    out.push_str(&generated_note(
        "the suspect_config::schema tables (`env` column and `DEBUG_VARS`)",
    ));
    out.push_str(
        "Every `SUSPECT_*` variable the `suspect` binary reads. A test \
         (`crates/suspect-config/tests/env_registry.rs`) fails when the code \
         reads one that is not listed here.\n\n\
         ## Settings\n\n\
         Each variable sets one [configuration key](config.md). It overrides \
         `.suspect.yaml` and is overridden by an editor setting or a flag; \
         `suspect config --explain` names it as the origin.\n\n\
         | Variable | Key | Type | Default | Description |\n|---|---|---|---|---|\n",
    );
    let mut vars: Vec<(&str, &schema::Setting)> = schema::env_vars().collect();
    vars.sort_by_key(|(name, _)| *name);
    for (name, setting) in vars {
        writeln!(
            out,
            "| `{name}` | `{}` | {} | {} | {} |",
            setting.path,
            cell(&kind_text(setting.kind)),
            default_text(setting),
            cell(setting.doc)
        )
        .unwrap();
    }
    out.push_str(
        "\n## Debugging\n\n\
         These change diagnostic output only; `suspect config --explain` lists \
         the ones that are set.\n\n\
         | Variable | Prints |\n|---|---|\n",
    );
    for var in schema::DEBUG_VARS {
        writeln!(out, "| `{}` | {} |", var.name, cell(var.doc)).unwrap();
    }
    out
}

#[test]
fn the_diagnostics_reference_matches_the_registry() {
    bless("diagnostics.md", &diagnostics_page());
}

#[test]
fn the_config_reference_matches_the_setting_tables() {
    bless("config.md", &config_page());
}

#[test]
fn the_environment_reference_matches_the_env_registry() {
    bless("environment.md", &environment_page());
}
