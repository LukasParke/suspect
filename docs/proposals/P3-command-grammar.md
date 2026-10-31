# P3. One command grammar

Status: proposal. Depends on: P1 (what `suspect check` runs, the shared
finding renderers, JUnit for findings), P2 (config-driven defaults for
`--base-url`, output roots and formats). Decided: none specific to P3. The
P1 decision that `sdk-*` readiness codes stay a separate category, and the
P2 decision that policy lives only in `.suspect.yaml` (option A), are
assumed here and not reopened.

## Summary

A user gets 23 top-level commands instead of 49. Related commands move into
groups (`diff`, `gen`, `test`, `traffic`, `release`, `dev`). `--format`,
`-o/--output`, `--to`, `--check` and the exit status mean the same thing in
every command. Every old name keeps working for two releases. It prints one
deprecation line on stderr and nothing changes on stdout. In the code, a new
`args` module holds per-kind format enums and shared output flags. A new
`exit` module holds a typed `Exit` enum and the one sanctioned process exit.
A new `legacy` module rewrites old argv before clap parses it. A new
`cli_conventions` test walks the clap tree and enforces all of this.

## Problem

All references are to this worktree at `c7e87fb`. The binary was run as
`target/debug/suspect` (no CLI source changes since it was built).

### 1. 49 flat top-level commands

`suspect --help` lists 49 commands plus `help`:

```text
$ target/debug/suspect --help | sed -n '/Commands:/,/help /p' | grep -vc -E 'Commands:|^\s+help '
49
```

They are the variants of `Command` in `crates/suspect-cli/src/lib.rs:58-517`.
Five of them already have nested subcommands: `auth` (check), `rules`
(check, new, bench), `overlay` (apply, explain, diff), `project` (init,
check, build). `docs` also has a hidden alias `docs-gen` (`lib.rs:97`).
Prefix families show the missing grouping: seven `codegen*` commands, three
`release*`-style commands (`release-plan`, `release-publish`,
`release-workflow`), and seven commands that compare two things.

### 2. Overlapping commands that are not actually duplicates

The short version says these overlap. They do, but each pair differs in
behaviour, so a merge has to choose.

**`docs --style markdown` vs `gen --preset docs-md`.** Two renderers with
two models.

- `docs` parses one file with `LowDoc::parse` and renders through
  `suspect_lsp::docs_gen` (`commands/docs_gen_cmd.rs:50-60`). Markdown style
  writes frontmatter pages per tag and per schema plus `toc.json`. It has no
  ownership manifest and no drift check.
- `gen` builds an `IrSpec` from the `$ref` workspace and renders minijinja
  templates from `suspect-gen/presets/docs-md/` (`commands/generate.rs:7-60`,
  `suspect-gen/src/presets.rs:27-50`). It writes an ownership manifest and
  supports `--diff`, `--owner` and `--adopt-identical`.

```text
$ suspect docs pet.yaml --style markdown --output d1
docs → d1 (5 files: index, toc.json, 1 operation pages, 2 schema pages)
$ suspect gen pet.yaml --preset docs-md -o d2
  created  /private/tmp/p3/d2/docs/api/index.md
  created  /private/tmp/p3/d2/docs/api/schemas.md
  created  /private/tmp/p3/d2/.suspect-artifacts.json
```

The output trees and the page content differ (`d1/index.md` starts with
YAML frontmatter and a tag table; `d2/docs/api/index.md` is a single TOC
page). `README.md:182` points users at the `gen` form for Markdown.

**`overlay-dry-run` vs `overlay explain`.** These do not share code.

- `overlay-dry-run` (`commands/overlay_dry_run.rs`) parses the overlay as a
  raw JSON value, not through `OverlayDoc::parse`. It rewrites bare-dot
  targets (`$.paths./pets.get`) with its own `normalize_target` (`:98`),
  which `overlay apply` does not do. It reports `remove: true` as `update`,
  because it compares the boolean with the string `"true"` (`:72`). It
  prints to stderr only and exits 1 when any action matches nothing
  (`:87`).
- `overlay explain` (`commands/overlay.rs:97-123`) uses `OverlayDoc::parse`
  and `suspect_overlay::explain`, the same path as `apply`. It prints
  before/after values and always exits 0.

```text
$ suspect overlay-dry-run ov.yaml pet.yaml      # action 0 target: $.paths./pets.get
action 0: target=$.paths./pets.get matches=1 kind=update
action 1: target=$.info.contact matches=0 kind=update      # this action is `remove: true`
...
exit 1
$ suspect overlay explain ov.yaml pet.yaml
error: action #0 has invalid target "$.paths./pets.get": invalid JSONPath query at offset 8 ...
exit 2
$ suspect overlay apply ov.yaml pet.yaml
error: action #0 has invalid target "$.paths./pets.get": ...
exit 2
```

So the dry run reports a match for a target that `apply` rejects, and it
mislabels removals. `explain` is the correct base. Only the "exit 1 on
unmatched" behaviour is worth keeping.

**The comparison family.** Seven commands, five input shapes:

| Command | What it compares | Inputs | Exit |
|---|---|---|---|
| `diff` | any two JSON/YAML value trees, added/removed/changed scalars (`diff.rs:1-3`) | `A B` | always 0 (`diff.rs:141`) |
| `breaking` | OpenAPI break differ, workspace-backed (`commands/breaking.rs:1-7`) | `[OLD] [NEW]` or `--current NEW --baseline REF` | 1 on any finding, warnings included (`:190`) |
| `arazzo-diff` | Arazzo breaking changes | `OLD NEW` | 1 on any finding (`arazzo_diff.rs:259`) |
| `overlay diff` | synthesizes an Overlay 1.1 document from two revisions (`overlay.rs:124-142`) | `OLD NEW`, `-o FILE` | 0 |
| `codegen-compare` | wire contract and native SDK interfaces of two SDK session configs | `--before CFG --after CFG` (`codegen_compare.rs:19-22`) | 0 compatible, 1 not proven, 2 error |
| `release-plan` | break differ plus changelog and semver bump (`release.rs:1-8`) | same as `breaking` | 0 |
| `impact` | replays a cassette against two revisions (`impact.rs:1-7`) | `OLD NEW CASSETTE` | 1 when any exchange breaks (`:98`) |

`overlay diff` produces a document, not a verdict, so it is not a
comparison report and stays with `overlay`. `release-plan` produces a
release artifact and belongs with `release`. The other five produce a
"what changed / what breaks" verdict.

### 3. One `--format` enum shared by 27 commands with different abilities

`OutputFormat { Text, Json, Sarif }` (`lib.rs:24-33`) is the only report
format enum. 27 commands accept `--format text|json|sarif` (count of
`possible values: text, json, sarif` in `docs/CLI.md`). Only 7 render
SARIF: `validate`, `lint`, `admission`, `rules check`, `breaking`,
`arazzo-diff`, `drift`. For the other 20, clap accepts `sarif`, and then:

| Behaviour on `--format sarif` | Commands | Where |
|---|---|---|
| silently prints JSON | `check`, `stats`, `bench`, `diff`, `acquire`, `evidence`, `codegen-profiles`, `ci`, `codegen`, `codegen-terraform`, `codegen-cli`, `codegen-mcp` | `check.rs:164`, `stats.rs:151`, `bench.rs:242`, `diff.rs:139`, `acquire.rs:58`, `evidence.rs:78`, `sdk_profiles.rs:30,68`, `ci.rs:216`, `sdk.rs:704`, `terraform_cmd.rs:288`, `application_cmd.rs:435` |
| silently prints text | `why`, `codegen-session`, `codegen-compare` (Markdown) | `lib.rs:818`, `codegen_session.rs:149`, `codegen_compare.rs:71,80` |
| runtime error, exit 2, before work | `bridge`, `reverse` | `lib.rs:537-542`, `:755`, `:768` |
| runtime error, exit 2, after the work ran | `release-plan`, `impact`, `release-publish` | `release.rs:213`, `impact.rs:81`, `publish.rs:587` |

```text
$ suspect check docs/demo/petstore.yaml --format sarif | head -3
[
  {
    "path": "docs/demo/petstore.yaml",
$ suspect why type --spec docs/demo/petstore.yaml --format sarif | head -1
constraint: Constraint fired: type
$ suspect release-plan pet.yaml pet2.yaml --format sarif
error: SARIF output is not defined for release plans
exit 2
```

Streams use two spellings. `test` and `load` use `--report text|ndjson`
(`lib.rs:553-560`, `load.rs:83`). `bridge` and `codegen-session` print
NDJSON under `--format json` (`lib.rs:235-238`, CLI.md "versioned
newline-delimited JSON records").

The other value enums are correct and stay: `DocFormat`, `Strategy`,
`Severity`, `DocsStyle`, the stubs backend, `SdkProfile`,
`SdkCompatibilityProfile`, `ProfileKind`, `JitterArg`. `gateway --mode` is a
`String` checked at runtime (`gateway.rs:73-95`), and `gateway --upstream`
is typed `PathBuf` for a URL (`lib.rs:429-430`).

### 4. Output, document-format, report and check flags

Measured from `docs/CLI.md` (generated from clap) and the arg structs:

| Concern | Today |
|---|---|
| `-o, --output FILE` | `fmt`, `bundle`, `har`, `overlay apply`, `overlay diff`, `release-workflow` |
| `--output` without `-o` | `docs` (file for html, directory for markdown/sveltekit), `upgrade` |
| `-o, --out DIR` | `gen`, `codegen`, `codegen-terraform`, `codegen-cli`, `codegen-mcp`, `contract` |
| `--out DIR` without `-o` | `stubs` (required), `codegen-session` |
| default output dirs (7) | `gen-out` (`lib.rs:390`), `codegen-out` (`codegen_cmd.rs:142`, after `codegen.out`), `terraform-out` (`terraform_cmd.rs:44`), `cli-out` and `mcp-out` (`application_cmd.rs:153-154`), `sdk-out` (`codegen_session.rs:26`), `contract` (`contract.rs:36`) |
| document format | `fmt --json/--yaml`; `contract --json` (default YAML); `bundle --format json\|yaml`; `upgrade --format json\|yaml`; `overlay apply/diff` pick from the output extension (`output.rs:133`) |
| report files | `load --json FILE`, `load --junit FILE` only (`load.rs:101-104`); `--journal FILE` on `test`, `fuzz`, `replay`, `gateway` |
| read-only drift check | `--check` on `codegen`, `codegen-terraform`, `codegen-cli`, `codegen-mcp`, `codegen-session`, `contract`; `gen --diff` (prints diffs and exits 1, `lib.rs:393`) |
| gate flags with other names | `drift --exit-on-gap`; `overlay-dry-run` exits 1 by default |
| dry run | `load --dry-run` (prints the plan); `release-publish --dry-run` (a no-op: dry run is the default, `--execute` turns it off) |
| "print diffs" | `gen --diff` (generated files), `replay --diff` (response bodies) |

Inputs:

| Today | Command |
|---|---|
| spec as a flag | `why CONSTRAINT --spec SPEC` (`lib.rs:343-347`) |
| spec second | `reverse SOURCE SPEC` (`lib.rs:242-246`) |
| comparison as flags | `codegen-compare --before --after` |
| comparison positional | `breaking`, `arazzo-diff`, `release-plan`, `impact`, `diff`, `overlay diff` |
| two input spellings | `test --input NAME=VALUE` and `test --inputs FILE` (`lib.rs:280`, `:307`); same pair on `load` |
| redundant flag | `test --offline` is required by `--cassette` and then ignored (`lib.rs:283-287`, `offline: _` at `:786`) |
| line/column | `why --why-line --why-col` |

### 5. Exit codes are integers, with two process exits and no interrupt rule

- `execute` returns `anyhow::Result<i32>` (`lib.rs:576`). `main` casts with
  `code as u8` and maps every `Err` to 2 (`main.rs:9-17`).
- Two `std::process::exit` calls bypass that path:
  `codegen_cmd.rs:129-132` (missing `--profile`, exit 2) and `load.rs:231`
  (second Ctrl-C, exit 130).
- Three commands catch SIGINT, with three different results:
  `load` drains and returns 130 only when no request was sent
  (`load.rs:227-232`, `:306-307`); `acquire` cancels the transfer and
  returns 130 (`acquire.rs:37-49`); `gateway` returns 0 (`gateway.rs:129-142`).
  Every other command dies by signal.
- tokio's handler is process-wide and permanent once registered: "the
  default platform behavior will NOT be reset"
  (`tokio-1.54.1/src/signal/ctrl_c.rs:22-31`). A command that registers it
  and then runs more synchronous work can no longer be stopped by Ctrl-C.
  No command does this today, but `ci` and `project build` chain stages, so
  the next stage that adds a handler would.
- The crate doc says exit 1 means "findings at/above Error" (`lib.rs:4`).
  `breaking` and `arazzo-diff` exit 1 on warnings too.
- `diff`, `overlay explain`, `reverse` and `why` always exit 0, so they
  cannot gate CI. `reverse` returns 0 even when it finds undocumented routes
  (`reverse.rs:37`, `:63`).
- `codegen-compare` prints its error on stdout and exits 2.

## Goals and non-goals

Goals:

- A two-level command tree. Every existing command has exactly one new home.
- Every old invocation keeps working, with the same stdout and exit status,
  for two releases. It prints one deprecation line on stderr.
- `--format` accepts only values the command renders. Clap rejects the rest
  with exit 2 before any work starts.
- One spelling each for output path, document format, gate mode, report
  files and inputs.
- A typed `Exit` returned by every command. One place maps it to a process
  status, and one place handles Ctrl-C.
- A test that fails when a new command or flag breaks the conventions.

Non-goals:

- What `suspect check` runs, and the finding JSON shape. P1 owns both. This
  proposal only gives `check`, `validate` and `lint` their place in the tree.
- Config keys for defaults (`base_url`, output roots, report format). P2
  owns them. This proposal keeps today's built-in defaults.
- Changing any JSON report shape. Merged commands dispatch to the existing
  renderers.
- Shell completions and man pages. They become cheap once the tree is
  stable, but nothing asks for them yet.

## Design

### Command tree

```text
suspect check         run every check (P1)               [--only validate|lint|... per P1]
suspect validate      filtered view of check (P1)
suspect lint          filtered view of check (P1)
suspect fmt | bundle | upgrade | stats
suspect overlay       apply | explain | diff
suspect diff          structural | breaking | impact | sdk | routes
suspect gen           docs | sdk | stubs | contract | terraform | cli | mcp
                      template | session | bridge | profiles | admission
suspect test          run | stateful | fuzz | load | replay
suspect traffic       gateway | drift | har
suspect release       plan | publish | workflow
suspect project       init | check | build
suspect ci
suspect why
suspect rules         check | new | bench
suspect config | auth | acquire | watch | lsp
suspect dev           bench | evidence
```

That is 23 top-level commands. Mapping of all 49 (nested commands not
listed are unchanged):

| Today | Proposed | Note |
|---|---|---|
| `acquire` | `acquire` | |
| `auth check` | `auth check` | |
| `check` | `check` | semantics change in P1; the parse report moves to `stats` or `check --parse-only` per P1 |
| `validate`, `lint` | `validate`, `lint` | P1 makes them filters over the check pass |
| `admission` | `gen admission` | P1 may also expose it as `check --only admission` |
| `breaking` | `diff breaking` | |
| `arazzo-diff` | `diff breaking` | dispatch on `LowDoc::sniff_family()` of NEW |
| `docs` (`docs-gen`) | `gen docs` | |
| `stubs` | `gen stubs` | |
| `overlay-dry-run` | `overlay explain --check` | see below |
| `upgrade`, `fmt`, `stats`, `bundle` | unchanged | |
| `rules check\|new\|bench` | unchanged | |
| `overlay apply\|explain\|diff` | unchanged | `explain` gains `--check` and `--format` |
| `diff` | `diff structural` | |
| `drift` | `traffic drift` | |
| `har` | `traffic har` | |
| `bench` | `dev bench` | |
| `bridge` | `gen bridge` | |
| `reverse` | `diff routes` | positionals become `SPEC SOURCE` |
| `stateful` | `test stateful` | |
| `test` | `test run` | |
| `load` | `test load` | |
| `fuzz` | `test fuzz` | |
| `why` | `why` | positionals become `SPEC CONSTRAINT` |
| `replay` | `test replay` | |
| `gen` | `gen template` | `--preset docs-md` stays here |
| `watch` | `watch` | |
| `gateway` | `traffic gateway` | |
| `codegen` | `gen sdk` | |
| `codegen-terraform` | `gen terraform` | |
| `codegen-cli` | `gen cli` | |
| `codegen-mcp` | `gen mcp` | |
| `codegen-profiles` | `gen profiles` | |
| `codegen-session` | `gen session` | |
| `codegen-compare` | `diff sdk` | positionals `OLD NEW` (session configs) |
| `release-plan` | `release plan` | the module doc already says `suspect release plan` (`release.rs:1`) |
| `impact` | `diff impact` | |
| `release-publish` | `release publish` | |
| `release-workflow` | `release workflow` | |
| `evidence` | `dev evidence` | |
| `ci` | `ci` | |
| `contract` | `gen contract` | |
| `config`, `lsp` | unchanged | |
| `project init\|check\|build` | unchanged | |

Changes against the short version's tree, one reason each:

- **`admission`, `bench`, `evidence` and `contract` were missing.**
  `contract` writes a package into an output root with `--check`, so it is
  generation. `admission` is the codegen admission layer, run without
  generating. `bench` (pipeline micro-benchmark) and `evidence` (its default
  `--fragments` path is `crates/suspect-codegen/tests/behavioral`, so it only
  works inside this repo) are developer tools and go under `dev`.
- **`ci` stays top level, not `project ci`.** `project` subcommands act on
  one `--manifest`, while `ci` discovers every `suspect.project.json` under a
  root (`ci.rs:145-152`), and CI scripts call it by name.
- **`why` stays top level, not under `traffic`.** It traces a finding to
  its source, blame and history. Cassettes are an optional input
  (`--cassette-dir`).
- **`reverse` goes to `diff routes`, not `traffic`.** It compares the spec
  with server source and reads no traffic (`reverse.rs:1-6`).
- **`bridge` goes to `gen bridge`, not `traffic`.** The CLI drives only the
  "spec change becomes regeneration plan" loop (`bridge.rs:6-8`).
- **`codegen-compare` goes to `diff sdk`, not `gen compare`.** It returns a
  compatible/incompatible verdict with exit 1, the same as the other `diff`
  commands.
- **No `diff --arazzo` flag.** `diff breaking` dispatches on
  `LowDoc::sniff_family()` (`suspect-low/src/doc.rs:103`): `Arazzo10` goes
  to `arazzo_diff::arazzo_diff`, the OpenAPI families go to
  `breaking::breaking_between`. Each keeps its JSON shape. The git-baseline
  mode (`--current`/`--baseline`) then also works for Arazzo.
- **`replay` stays in `test`, as in the short version.** It re-issues
  traffic against a live upstream and reports drift, like the other `test`
  commands. `traffic` keeps only commands that produce or read cassettes and
  journals offline, plus the gateway that records them.

### Merges

**`overlay-dry-run` into `overlay explain --check`.** `explain` gains:

- `--check`: exit 1 when any action matches zero nodes (the dry-run's useful
  behaviour).
- `--format text|json`: JSON is the `Vec` of explain steps that
  `suspect_overlay::explain` already returns.

`normalize_target` is not carried over. Accepting a target in the dry run
that `apply` rejects is the bug. The legacy name prints the deprecation line
plus one more line when the old command would have normalized a target:
`note: target "$.paths./pets.get" is not RFC 9535; overlay apply rejects it`.
`commands/overlay_dry_run.rs` is deleted when the alias is removed.

**`docs` and `gen --preset docs-md`.** `gen docs --style
html|markdown|sveltekit` is the documentation command. `gen template` keeps
`--preset docs-md` as the starter for writing custom templates, and its help
says so. `README.md:182` changes to point Markdown users at `gen docs
--style markdown`. The two renderers are not merged in P3. Their output
differs, and both have users. Whether to retire `docs-md` is an open
question below.

**`diff` becomes a group.** Old `suspect diff A B` is rewritten to
`suspect diff structural A B` (see "Legacy dispatch").

### Shared argument module

New file `crates/suspect-cli/src/args.rs`. It replaces `OutputFormat`,
`TextFormat`, `ReportFormat` and `reject_sarif` in `lib.rs`.

```rust
// new: crates/suspect-cli/src/args.rs
use std::path::PathBuf;
use clap::{Args, ValueEnum};

/// Commands that emit located findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum FindingsFormat { #[default] Text, Json, Sarif }   // P1 adds Junit

/// Commands that emit one structured report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat { #[default] Text, Json }

/// Long-running commands that emit one record per event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum StreamFormat {
    #[default] Text,
    /// `json` is accepted for `bridge` and `codegen-session` compatibility.
    #[value(alias = "json")] Ndjson,
}

#[derive(Debug, Args)]
pub struct Findings { #[arg(long, value_enum, default_value_t)] pub format: FindingsFormat }
#[derive(Debug, Args)]
pub struct Report   { #[arg(long, value_enum, default_value_t)] pub format: ReportFormat }
#[derive(Debug, Args)]
pub struct Stream {
    #[arg(long, value_enum, default_value_t, alias = "report")]
    pub format: StreamFormat,
}

/// A document written to one file, or stdout.
#[derive(Debug, Args)]
pub struct DocOut {
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Serialization; default: the output extension, else the input's.
    #[arg(long = "to", value_enum)]
    pub to: Option<crate::DocFormat>,
}

/// Read-only gate: write nothing, exit 1 when the result is not clean.
#[derive(Debug, Args)]
pub struct Check {
    #[arg(long)] pub check: bool,
    /// With --check: also print unified diffs.
    #[arg(long, requires = "check")] pub diff: bool,
}

/// Machine-readable report files for test commands.
#[derive(Debug, Args)]
pub struct ReportFiles {
    #[arg(long = "report-file", value_name = "FILE", alias = "json")]
    pub report_file: Option<PathBuf>,
    #[arg(long, value_name = "FILE")]
    pub junit: Option<PathBuf>,
}
```

The output-directory flag is not a shared struct. Each generator has its
own default, so each declares
`#[arg(short = 'o', long = "output", visible_alias = "out", value_name = "DIR", default_value = "...")]`.
`visible_alias` keeps `--out` documented for the alias window. It becomes a
hidden `alias` in the release after that.

Assignments:

| Kind | Commands |
|---|---|
| `Findings` | `check`, `validate`, `lint`, `rules check`, `diff breaking`, `traffic drift`, `gen admission` |
| `Report` | `stats`, `diff structural`, `diff impact`, `diff sdk` (text is Markdown), `diff routes`, `why`, `acquire`, `ci`, `gen sdk\|terraform\|cli\|mcp\|profiles`, `release plan\|publish`, `overlay explain`, `dev bench\|evidence` |
| `Stream` | `test run`, `test load`, `gen bridge`, `gen session` |
| `DocOut` | `fmt`, `bundle`, `upgrade`, `overlay apply`, `overlay diff`, `traffic har`, `release workflow` (`--to` hidden where only one format exists) |
| `Check` | `gen sdk\|terraform\|cli\|mcp\|session\|contract\|template\|docs`, `diff structural`, `diff routes`, `overlay explain`, `traffic drift` |
| `ReportFiles` | `test run`, `test load`, `test stateful`, `test fuzz`, `test replay` |

`ci --format sarif` and `release plan --format sarif` are rejected by clap
after this change. Clap reports `invalid value 'sarif' for '--format'`,
exit 2. Whether `ci` should render its findings as SARIF once P1 provides
them is an open question.

### Flag conventions

| Concern | Convention | Old spellings (hidden aliases, two releases) |
|---|---|---|
| stdout format | `--format`, per-kind enum (above) | `test/load --report text\|ndjson`; `bridge`/`codegen-session --format json` (value alias) |
| document serialization | `--to json\|yaml` | `fmt --json/--yaml`, `contract --json`, `bundle/upgrade --format` |
| output path | `-o, --output PATH`; `value_name` says `FILE` or `DIR` | `--out` (all generators, `stubs`, `codegen-session`); `docs --output` keeps its name and gains `-o` |
| gate | `--check` exits 1 when not clean, writes nothing; `--diff` adds unified diffs | `gen --diff` becomes `--check --diff`; `drift --exit-on-gap` becomes `--check`; `overlay-dry-run` becomes `explain --check` |
| report files | `--report-file FILE` (JSON), `--junit FILE` | `load --json FILE` |
| event journal | `--journal FILE` (unchanged) | |
| spec input | first positional; comparisons are `OLD NEW` | `why --spec`; `reverse SOURCE SPEC`; `codegen-compare --before/--after` |
| workflow inputs | `--input NAME=VALUE` (repeatable), `--inputs-file FILE` | `--inputs FILE` on `test` and `load` |
| typed choices | `ValueEnum` | `gateway --mode` (String), `gateway --upstream` (`PathBuf`, becomes `String`) |

One change against the short version. It proposed `-o, --output FILE` and
`-o, --out DIR` as two flags. Two long names three letters apart, sharing
`-o`, are what users confuse today. So the long name is `--output`
everywhere and `value_name` carries the file/directory distinction.

Positional changes cannot be made with clap aliases. They are handled as
follows:

- `why SPEC CONSTRAINT`: a hidden `--spec` arg remains. When it is present,
  the single positional is the constraint.
- `diff routes SPEC SOURCE`: the legacy rewrite for `reverse` appends a
  hidden `--source-first` flag that swaps the two positionals.
- `diff sdk OLD NEW`: the hidden `--before`/`--after` flags conflict with
  the positionals and fill them.

Built-in defaults stay as they are: `http://localhost:8080` for
`--base-url` and the seven output roots. Renaming the roots would strand the
`.suspect-artifacts.json` ownership manifests that live in them. P2 makes
these values configurable.

### Exit status

New file `crates/suspect-cli/src/exit.rs`:

```rust
// new: crates/suspect-cli/src/exit.rs
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
    #[must_use]
    pub const fn code(self) -> u8 {
        match self { Self::Clean => 0, Self::Findings => 1, Self::Usage => 2, Self::Interrupted => 130 }
    }
    /// `Findings` when `failed`, else `Clean`.
    #[must_use]
    pub const fn failed_if(failed: bool) -> Self { if failed { Self::Findings } else { Self::Clean } }
    /// Transitional, while command functions still return `i32`.
    #[must_use]
    pub fn from_legacy(code: i32) -> Self {
        match code { 0 => Self::Clean, 1 => Self::Findings, 130 => Self::Interrupted, _ => Self::Usage }
    }
}

impl From<Exit> for std::process::ExitCode {
    fn from(exit: Exit) -> Self { Self::from(exit.code()) }
}

/// The one sanctioned immediate exit: a second Ctrl-C, or a first Ctrl-C
/// in a command that did not opt into graceful shutdown.
#[allow(clippy::disallowed_methods)]
pub fn abort(exit: Exit) -> ! { std::process::exit(i32::from(exit.code())) }
```

`execute` becomes `pub fn execute(cli: Cli) -> anyhow::Result<Exit>`. Usage
errors detected at runtime (missing `--profile` in `codegen_cmd.rs:129`, an
empty `--filter` match in `test.rs:119`, `stateful.rs:36`, `fuzz.rs:109`)
return `Err` or `Ok(Exit::Usage)` instead of exiting. A new
`crates/suspect-cli/clippy.toml` bans
`std::process::exit` (`disallowed-methods`), so `exit::abort` is the only
call site.

`main.rs` after the change:

```rust
fn main() -> ExitCode {
    let (argv, note) = suspect_cli::legacy::rewrite(std::env::args_os().collect());
    if let Some(note) = note { eprintln!("{note}"); }
    let cli = suspect_cli::Cli::parse_from(argv);   // clap exits 2 on usage, 0 on --help
    suspect_cli::interrupt::install();
    match suspect_cli::execute(cli) {
        Ok(exit) => exit.into(),
        Err(e) => { eprintln!("error: {e:#}"); suspect_cli::Exit::Usage.into() }
    }
}
```

**Interrupt handling.** New file `crates/suspect-cli/src/interrupt.rs`.
Because tokio's SIGINT handler is permanent once registered, the CLI
registers it once, at start-up, on a dedicated thread with a
current-thread runtime. Commands never call `tokio::signal::ctrl_c`
themselves.

```rust
// new: crates/suspect-cli/src/interrupt.rs
static GRACEFUL: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();

/// Starts the one SIGINT listener. First Ctrl-C: if a command opted in,
/// signal it; otherwise abort with 130. Second Ctrl-C: abort with 130.
pub fn install() { /* thread + current_thread runtime looping on ctrl_c() */ }

/// Opt in to graceful shutdown; returns a receiver that flips to `true`
/// on the first Ctrl-C.
pub fn graceful() -> tokio::sync::watch::Receiver<bool> { /* sets GRACEFUL */ }
```

- `load` and `acquire` call `interrupt::graceful()` instead of their own
  `ctrl_c` tasks (`load.rs:225-234`, `acquire.rs:33-45`), drain, and return
  `Exit::Interrupted`. `load` keeps its current rule: an interrupted run
  that sent requests reports its `ensure` verdict.
- Servers call `graceful()` and return `Exit::Clean` on the first Ctrl-C,
  because Ctrl-C is how they stop normally: `traffic gateway`
  (`gateway.rs:142`), `gen bridge`, `gen session --watch`, `watch`, `lsp`.
- Every other command keeps the abort path. Ctrl-C exits 130 at once, the
  same status a shell reports for death by SIGINT today.

**Per-command exit table.** New `exit::DOCUMENTED: &[(&str, &[Exit], &str)]`
(command path, possible statuses, one-line meaning). `cli_reference.rs`
renders it into `docs/CLI.md`, and the conventions test requires an entry
for every leaf command. Behaviour changes, each behind `--check` so default
behaviour stays the same:

| Command | Today | Proposed |
|---|---|---|
| `diff structural` | always 0 | `--check`: 1 when anything differs |
| `diff routes` | 0, or 2 when no routes are extracted | `--check`: 1 on undocumented or unimplemented routes |
| `overlay explain` | always 0 | `--check`: 1 on zero-match actions |
| `traffic drift` | 1 only with `--exit-on-gap` | same, spelled `--check` |
| `diff breaking` | 1 on any finding, warnings included | unchanged; documented. P1's severity floor decides whether warnings gate |
| `traffic gateway` and other servers | gateway 0 on Ctrl-C, others die by signal | 0 on Ctrl-C |

### Legacy dispatch

Clap's `alias` only works among siblings. `breaking` cannot be an alias of
`diff breaking`. Hidden top-level variants that reuse the same `Args` types
fail too, because `diff`, `test` and `gen` are old leaf names that become
group names. Clap cannot hold both a `diff` leaf and a `diff` group.

The top-level `Cli` has no options before the subcommand (`lib.rs:564-570`),
so `argv[1]` is always the command name. That makes an argv rewrite before
parsing exact. New file `crates/suspect-cli/src/legacy.rs`:

```rust
// new: crates/suspect-cli/src/legacy.rs
pub struct Renamed {
    pub old: &'static str,
    pub new: &'static [&'static str],
    /// Hidden flags appended so the new command reads old positionals.
    pub extra: &'static [&'static str],
}

pub const RENAMED: &[Renamed] = &[
    Renamed { old: "breaking",          new: &["diff", "breaking"],   extra: &[] },
    Renamed { old: "arazzo-diff",       new: &["diff", "breaking"],   extra: &[] },
    Renamed { old: "impact",            new: &["diff", "impact"],     extra: &[] },
    Renamed { old: "codegen-compare",   new: &["diff", "sdk"],        extra: &[] },
    Renamed { old: "reverse",           new: &["diff", "routes"],     extra: &["--source-first"] },
    Renamed { old: "overlay-dry-run",   new: &["overlay", "explain"], extra: &["--check"] },
    Renamed { old: "docs",              new: &["gen", "docs"],        extra: &[] },
    Renamed { old: "docs-gen",          new: &["gen", "docs"],        extra: &[] },
    Renamed { old: "codegen",           new: &["gen", "sdk"],         extra: &[] },
    // ... one row per moved command (30 rows)
];

/// Groups whose old leaf form took positionals:
/// `suspect diff A B` -> `suspect diff structural A B`.
pub const DEFAULT_SUB: &[(&str, &str)] =
    &[("diff", "structural"), ("test", "run"), ("gen", "template")];

/// Rewrites an old invocation; returns the new argv and the stderr note.
pub fn rewrite(mut argv: Vec<OsString>) -> (Vec<OsString>, Option<String>) {
    // 1. `help <old>` -> `help <new...>`.
    // 2. argv[1] in RENAMED -> splice `new`, append `extra`.
    // 3. argv[1] in DEFAULT_SUB and argv[2] is neither a subcommand name of
    //    that group (read from `Cli::command()`, not hard-coded) nor
    //    -h/--help -> insert the default subcommand.
    // Note: "warning: `suspect breaking` is deprecated; use `suspect diff
    // breaking` (the old name is removed in <version>)".
}
```

Notes:

- Group subcommand names come from `Cli::command().find_subcommand(group)`,
  so the rewrite cannot drift from the tree.
- `suspect diff` with no arguments, `suspect diff --help` and `suspect test
  --help` show the group help. In the old CLI they printed the leaf's
  missing-argument error or help. Both exit with the same status.
- An old leaf invocation whose first argument is a file literally named
  like a new subcommand (`suspect diff breaking other.yaml`) now runs the
  subcommand. `./breaking` avoids it. This is documented in the deprecation
  note's help link.
- Old flag spellings are clap `alias`es on the new args, not rewrites.
  `alias` (not `visible_alias`) keeps them out of `--help`, and
  `docs/CLI.md` stays clean.
- The note goes to stderr. Stdout and exit status are byte-for-byte the old
  ones. The VS Code extension only logs stderr (`generation.ts:104`,
  `runner.ts:147`).
- `cli_reference.rs` renders `RENAMED` as a "Renamed commands" table at the
  end of `docs/CLI.md`.

## Compatibility and migration

- **Release N (this work):** new tree. Every old command name, old flag
  spelling and old value (`--format json` on streams, `--report`) still
  works and prints one stderr line per invocation. Flag aliases print no
  note in N, so a single run does not print a wall of notes. `--out` stays a
  visible alias.
- **Release N+1:** flag aliases print a note too. `--out` becomes hidden.
- **Release N+2:** `RENAMED`, `DEFAULT_SUB` and the hidden aliases are
  deleted. `commands/overlay_dry_run.rs` is deleted.
- **What breaks in N, by design:**
  - `--format sarif` on the 20 commands that never rendered SARIF now fails
    in clap. Before, it printed JSON or text, or failed after the work. Any
    script that relied on the silent JSON was getting JSON under the wrong
    name. The fix is `--format json`, which works in N-1 too.
  - `overlay-dry-run` rejects targets that `overlay apply` rejects.
- **Editors.** The VS Code extension spawns `lsp`, `gateway`, `codegen`,
  `codegen-profiles`, `codegen-session` and `test --report ndjson`
  (`extension.ts:95,311`, `generation.ts:55,75,176`, `runner.ts:128`). It
  keeps these names until its minimum supported binary is N, because a newer
  extension can run against an older binary (P4 version handshake). It moves
  to the new names in N+1, well inside the alias window. `xtask` calls
  `codegen-profiles` and `codegen-session` (`xtask/src/sdk_full.rs:1404`,
  `sdk_m3_m6.rs:1536`). These move in the same PR as the tree.
- **Generated files.** SvelteKit output embeds `GENERATED by suspect docs`
  in file headers (`suspect-lsp/src/docs_gen/sveltekit.rs:257` and others).
  That text stays unchanged. Changing it would make every existing site
  report drift on the next regeneration.
- **Tests.** The 22 test files that run `CARGO_BIN_EXE_suspect` keep their
  old invocations until N+2. That gives free coverage of the rewrite. They
  are migrated in the removal PR.

## Implementation plan

Each step leaves `main` green. `docs/CLI.md` is regenerated
(`SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`) in every
step that changes the tree.

1. **Typed exit and one interrupt path.** Add `exit.rs` and `interrupt.rs`.
   `execute` returns `Exit` via `Exit::from_legacy`. Replace the two
   `process::exit` calls. Move `load`, `acquire` and `gateway` to
   `interrupt::graceful()`. Add `crates/suspect-cli/clippy.toml`. Tests: unit
   tests for `Exit::code`; an integration test that spawns `load` against a local
   listener, sends SIGINT, and expects 130 (Unix only); missing `--profile` exits 2 through `main`. Files:
   `main.rs`, `lib.rs`, `commands/{codegen_cmd,load,acquire,gateway}.rs`.
   Estimate: 1 day.
   **Done.** Deviations: `execute` wraps a private `dispatch` that still
   returns `i32`, through `Exit::from_legacy`. `interrupt` has a third
   mode, `interrupt::server()`: the first Ctrl-C exits 0 at once. `lsp`,
   `watch`, `bridge` and `codegen-session --watch` call it at dispatch in
   `lib.rs` (open question 2), since they have nothing to drain. `gateway`
   uses `graceful()`, including scenario mode, which used to die by signal.
   Missing `--profile` is now an `Err`, so its stderr line gains the
   `error: ` prefix. Tests are in `tests/exit_status.rs`, plus a gateway
   SIGINT test that expects 0.
2. **Per-kind format enums.** Add `args.rs` with `Findings`, `Report` and
   `Stream`. Move the 27 commands. Delete `reject_sarif`, `TextFormat` and
   `OutputFormat`, and replace the `Json | Sarif` arms. Add
   `tests/cli_conventions.rs` with the format rule and an allowlist of
   not-yet-converted flags (empty at the end of this step). Files: `lib.rs`
   and the 20 command modules listed in Problem 3. Estimate: 1 day.
   **Done.** Deviations: `Findings` is `text, json, sarif, junit`; JUnit
   renders through a new `suspect_diag::render::junit` (one `testsuite` per
   document, one `testcase` per finding, `failure` for errors and
   warnings), and `output::print_findings_as` is the one dispatch for the
   seven finding commands. `breaking --format json` keeps its break-record
   shape. `check` is `Report` until P1 step 5 makes it a finding command
   (today it prints parse reports). `config` is `Report` too, so its
   `reject_sarif` call is gone. `test` and `load` move to `--format
   text|ndjson` here, with `--report` as a hidden alias (`test` keeps an
   `Option` so `test.report` still applies). The format allowlist holds
   `bundle` and `upgrade` (`--format json|yaml`, document serialization)
   until step 3 moves them to `--to`.
3. **Output, document-format, gate and input flags.** `DocOut`, `Check`,
   `ReportFiles`, `--output` with `--out` alias, `--to`, `--inputs-file`,
   `gateway --mode` as `ValueEnum`, `test --offline` as a hidden no-op.
   Extend `cli_conventions.rs` with those rules. Add `--check` to `diff`,
   `reverse` and `overlay explain`, and `--format` to `overlay explain`.
   Tests: one alias test per renamed flag (old spelling parses to the same
   value). Estimate: 1 day.
   **Done.** Deviations: `Check` holds only `--check`. `--diff` exists
   only on `gen`, where it implies `--check` instead of requiring it, so
   the old `gen --diff` keeps working unchanged; the conventions rule is
   "`--diff` only where `--check` exists", with `replay --diff`
   (response-body diffs) allowlisted. Existing `--check` flags on the
   generators stay plain `bool` fields; `drift` spells its own
   `--check` (alias `--exit-on-gap`). `har` and `release-workflow` keep
   `-o/--output` without `--to` (one format each). `fmt --json/--yaml`
   and `contract --json` are hidden switches beside `--to`, since a bool
   cannot alias a value flag. `DocOut.to` carries the `--format` alias for
   `bundle`/`upgrade`. `ReportFiles` writes a small JSON run report
   (`version`, `command`, `passed`, `failed`, `skipped`, `duration_ms`) and
   JUnit through `suspect_test::reporters::junit` for `test`, `stateful`,
   `fuzz` and `replay`; `load` keeps its own report shapes behind the same
   flags. `docs --output` gains `-o` and keeps its long name. `diff
   routes`' `--check` gates on undocumented and spec-only routes, not
   method mismatches. Tests: `tests/cli_conventions.rs` (rules plus
   alias tests) and `tests/gate_flags.rs` (`diff`, `reverse`, `overlay
   explain`, `gen --check`).
4. **Command tree and legacy dispatch.** Regroup `Command` into group enums
   (`DiffCmd`, `GenCmd`, `TestCmd`, `TrafficCmd`, `ReleaseCmd`, `DevCmd`) in
   `lib.rs`, one variant per existing `Args` type. Add `legacy.rs` and the
   `main.rs` call. Add `diff breaking` family dispatch, `why`/`diff
   routes`/`diff sdk` positionals, and the "Renamed commands" section in
   `cli_reference.rs`. Update `xtask` call sites and `README.md:182`. Tests:
   every `RENAMED` row rewrites to a path that exists in `Cli::command()`;
   spawn `breaking a b --format json` and assert JSON stdout, the
   deprecation line on stderr, and the old exit status; `diff A B` and `diff
   structural A B` give identical stdout. Estimate: 1.5 days.
   **Done.** Deviations: groups are newtype variants
   (`Command::Diff(DiffCmd)`, ...). `legacy::RENAMED` has the 30 rows
   above; the old names are removed in two minor releases
   (`legacy::removed_in()`, `0.3.0` from `0.1.0`), and `Renamed::replacement()`
   names the new invocation without hidden compatibility flags (so the
   `reverse` note says `diff routes`, not `--source-first`). The
   `DEFAULT_SUB` note reads "`suspect diff <args>` is deprecated; use
   `suspect diff structural <args>`". `why` takes `SPEC [CONSTRAINT]`:
   the constraint is required unless the hidden `--spec` is given, in
   which case the one positional is the constraint. `diff sdk` takes
   optional `OLD NEW` positionals, each required unless its hidden
   `--before`/`--after` is given. `overlay-dry-run` runs `overlay explain
   --check` as designed, so its stdout now carries the explain steps and
   a bare-dot target exits 2 (the note names it); this is the one rename
   whose stdout and status differ, and `tests/ecosystem_commands.rs`
   asserts the new behaviour. `ArazzoDiffArgs` and `overlay_dry_run()`
   stay as library items until N+2. The `release publish` test helper uses
   the new name, since it asserts that no deprecation reaches stderr.
   xtask (`ci` gateway smoke, `sdk_full`, `sdk_m3_m6`), `README.md`, the
   GitHub Action (`test run`, `gen template`) and the user guides under
   `docs/` use the new names; `docs/SDK-EDITOR-PREVIEW.md` still shows
   the names the extension spawns. Editors are unchanged: VS Code and Zed
   read stderr only into error messages, so the note never reaches a
   parsed stream. Tests: `tests/legacy_names.rs` (spawned `breaking ...
   --format json`, `diff A B` vs `diff structural A B`, Arazzo dispatch,
   group help) and two conventions tests (every row targets an existing
   leaf and its extra flags exist; old positional orders parse).
5. **Exit table.** `exit::DOCUMENTED`, rendered into `docs/CLI.md`. The
   conventions test requires an entry for every leaf. Estimate: 0.5 day.
   **Done.** Deviations: paths omit the `suspect` prefix (`"diff
   breaking"`), and `config` has a row although it is a group, because it
   runs without a subcommand. Rows list `Interrupted` only for `test load`
   and `acquire`, which drain and report; the rendered "Exit status"
   section says every other non-server command exits 130 on Ctrl-C. The
   conventions test also rejects stale or duplicate rows and rows without
   `0` and `2`.
6. **(N+1) Editor switch.** VS Code moves to the new names and gets a
   minimum binary version check. `--out` becomes hidden, and flag-alias notes
   are turned on. Estimate: 0.5 day.
7. **(N+2) Removal.** Delete `legacy.rs` rows, hidden aliases and
   `overlay_dry_run.rs`. Migrate the 22 test files to the new names.
   Estimate: 0.5 day.

Total for release N: 5 days (the short version estimated 3.5 days; the
interrupt path, the positional changes and the exit table account for the
difference).

## Testing

- **`crates/suspect-cli/tests/cli_conventions.rs` (new).** It walks
  `Cli::command()` like `cli_reference.rs` does (`render`, lines 16-29), but
  visits every leaf and checks arguments with clap's introspection API:

  ```rust
  fn leaves(cmd: &clap::Command, path: String, out: &mut Vec<(String, clap::Command)>) {
      let subs: Vec<_> = cmd.get_subcommands().filter(|s| s.get_name() != "help").collect();
      if subs.is_empty() { out.push((path, cmd.clone())); }
      for sub in subs { leaves(sub, format!("{path} {}", sub.get_name()), out); }
  }

  #[test]
  fn format_flags_use_one_of_three_kinds() {
      const KINDS: [&[&str]; 3] = [&["text", "json", "sarif"], &["text", "json"], &["text", "ndjson"]];
      for (path, cmd) in all_leaves() {
          let Some(arg) = cmd.get_arguments().find(|a| a.get_long() == Some("format")) else { continue };
          let values: Vec<_> = arg.get_possible_values().iter().map(|v| v.get_name().to_owned()).collect();
          assert!(KINDS.iter().any(|k| *k == values), "{path}: --format {values:?}");
      }
  }
  ```

  Rules, each with a reasoned allowlist that starts empty:
  - `--format` values are exactly one of the three kinds.
  - Every visible `--output` has `-o`. No visible `--out`, `--json` or
    `--yaml` flag exists.
  - `--to` values are exactly `json, yaml`.
  - `--check` and `--diff` are `SetTrue` flags, and `--diff` requires
    `--check`.
  - No visible `--report` flag exists, and `--report-file`/`--junit` take
    `FILE`.
  - Every leaf has an `exit::DOCUMENTED` entry.
  - Every `legacy::RENAMED` target path exists, and no old name is still a
    visible top-level command.
- **Integration tests** in `crates/suspect-cli/tests/`: the legacy spawn
  tests (step 4), the SIGINT test (step 1, `#[cfg(unix)]`), and one test per
  former silent-degrade command asserting `--format sarif` exits 2 with
  clap's message and empty stdout.
- **Existing tests** keep running old names through the rewrite until N+2.
- **Editor host smoke.** `npm run test:host:smoke` in `editors/vscode` runs
  unchanged in N against the new binary, which proves the extension's old
  names still work. In N+1 it runs with the new names.
- **CI.** No new job. `cli_conventions` and `cli_reference` run under the
  existing `cargo test`. Clippy picks up the new `clippy.toml`.

## Alternatives considered

- **Hidden top-level clap variants reusing each `Args` type.** It needs no
  argv rewrite and gives typed parsing. It lost because `diff`, `test` and
  `gen` are both old leaves and new groups, and clap cannot hold two
  subcommands with one name.
- **`#[command(flatten)]` of a hidden `LegacyCommand` enum.** Same name
  collision, and it doubles the variants that `execute` must match.
- **External subcommand fallback (`allow_external_subcommands`).** It
  catches unknown names after a failed parse, but it loses typed parsing of
  the old args and turns typos into "unknown command" handling.
- **One `--format` enum plus a per-command `value_parser` list.** It rejects
  bad values too, but each command would repeat the list as strings, the
  renderers would still match on a three-value enum, and `Json | Sarif`
  fallthroughs could come back.
- **Separate `--out DIR` and `--output FILE` (the short version).** Rejected
  above. Two near-identical long names sharing `-o` is the confusion this
  removes.
- **Exit 1 by default from `diff structural`, like GNU `diff`.** It would
  silently change scripts that print a diff. `--check` gives the gate
  without changing the default.
- **Per-command Ctrl-C handlers (today's pattern).** They give inconsistent
  statuses, and tokio's permanent handler makes later sync stages
  unkillable.
- **Merging the two Markdown renderers now.** They produce different trees
  for different consumers (MDX pipelines vs template starters). Choosing one
  is a product decision, not grammar.

## Risks

- **Argv rewrite surprises.** A file named like a new subcommand changes
  meaning under `diff`, `test` and `gen`. The risk is low, the case is
  documented, and `./name` avoids it.
- **Deprecation noise in CI logs.** One line per invocation. Scripts that
  treat any stderr as failure would fail. There is no evidence of such
  scripts in-repo. A suppression variable would need P2's env registry, so
  it is left out.
- **Abort path skips destructors.** `exit::abort` on the first Ctrl-C in a
  non-graceful command skips `Drop`. That matches today's death by signal,
  but temp files from `baseline::materialize` stay behind, as they already
  do. `breaking --baseline` could opt into `graceful()` and clean up.
- **Exit status seen by Node.** The child now exits with code 130 rather
  than dying with signal `SIGINT`. The VS Code runner formats both
  (`generation.ts:108`). Third-party wrappers that check only `signal` would
  see a code instead.
- **Group help for old leaf `--help`.** `suspect test --help` shows the
  `test` group, not the old leaf. This is cosmetic, and the old leaf help is
  one level down.

## Open questions

1. **Retire the `docs-md` preset?** Recommended: no. Keep it under
   `gen template` as the documented starter for custom templates, and stop
   advertising it for docs (`README.md:182`).
2. **Ctrl-C status for servers (`gateway`, `lsp`, `watch`, `gen bridge`,
   `gen session --watch`).** Recommended: 0. Ctrl-C is how they stop, and
   `gateway` already returns 0.
3. **SARIF for `ci`.** Recommended: not in P3. `ci` moves to `Report`
   (`text, json`). It moves to `Findings` once P1 gives each stage a finding
   list.
4. **Should `diff breaking` exit 1 on warning-only breaks?** Recommended:
   keep today's behaviour in P3, and let P1's `--min-severity` floor apply
   to it once breaks are P1 findings.
5. **Alias window length.** Recommended: two minor releases, as the short
   version says, with flag-alias notes turned on only in the second.
6. **`gateway` at top level?** It is the command users type most after
   `lint`. Recommended: keep it in `traffic` as the short version proposed.
   The legacy alias covers existing muscle memory for two releases.
   Revisit only if users ask.
