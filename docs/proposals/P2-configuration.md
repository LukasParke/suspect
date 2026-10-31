# P2. One configuration

Status: proposal. Depends on: none. Decided: option A (2026-10-29): lint
policy lives only in `.suspect.yaml`; a manifest references it
(`"config": ".suspect.yaml"`); a manifest that still carries policy gets a
one-release deprecation warning.


Fixed since this was written (2026-10-30, see the
[index](README.md#already-fixed)): `ci` dropping every override on one bad name, `ci` and `project build` ignoring `validate.strict_format`, the editor mapping an unknown floor to `hint` silently, and the `config [INPUT]` help text. The Problem section keeps
them as evidence.
## Summary

For a user: every command and the editor apply the same settings to the
same file. `suspect config --explain` prints every effective value and
where it came from (`flag`, `env SUSPECT_X`, `.suspect.yaml:12`, `editor`,
`default`). The manifest stops being a second, differently merged policy
file.

In the code: `suspect_config::resolve` becomes the only way to read
settings. It replaces `for_invocation`/`discover` in the CLI, the CI-only
`lint_policy`/`lint_overrides` merge, and the LSP's second config type
(`config_files::SuspectConfig` with its own file readers). One static
setting table in `suspect-config` drives parsing, env vars, `--explain`,
LSP hover/completion and the checked-in JSON Schema.

## Problem

### 1. Three merges of manifest policy, three different answers

Reproduction (`/tmp/p2-repro/proj`): `suspect project init`, then
`lint.rules` set in both files, and a spec with no `contact`/`license`:

```text
suspect.project.json   "lint": {"rules": {"info-contact": "off", "info-license": "error"}, ...}
.suspect.yaml          lint: { rules: { info-contact: error } }
```

| Tool | `info-contact` | `info-license` | Merge rule that produced it |
|---|---|---|---|
| `suspect lint openapi.yaml` | error | error | per key, file over manifest |
| `suspect lsp` (stdio, `publishDiagnostics`) | error | warning | whole map, file replaces manifest |
| `suspect ci . --stage lint` | off | error | whole map, manifest replaces file |

Commands run: `suspect lint --format json` (filtered to the two codes);
a 60-line Python LSP client (initialize, `workspace/configuration` answered
`null`, `didOpen`, first `publishDiagnostics`); `suspect ci --format json`
reported `"errors": 2, "warnings": 5`, which matches `suspect lint` with
`.suspect.yaml` removed (2 errors, 7 findings, no `info-contact`).

Where each rule lives:

- **`suspect_config::discover`** (`crates/suspect-config/src/file.rs:38-69`)
  records the nearest manifest (`:49-54`), returns at the first settings
  file (`:55-64`), and composes with `Loaded::layered`
  (`crates/suspect-config/src/lib.rs:260-319`), which inserts the file's
  `lint.rules` entries one key at a time (`lib.rs:298-300`).
  Because it returns at the first settings file, a manifest *above* that
  file is never seen, although the doc comment says "both searches are
  independent" (`file.rs:32-34`).
- **LSP** `config_files::file_layers` (`crates/suspect-lsp/src/config_files.rs:282-288`)
  overlays the settings file on the manifest with `SuspectConfig::overlay`,
  which replaces the whole `rules` map when the upper map is non-empty
  (`config_files.rs:163-167`). `read_manifest_policy` reads the manifest
  only in the root directory (`:302`, no walk), while `read_settings_value`
  walks up (`:344-359`).
- **CI** `lint_policy` and `lint_overrides`
  (`crates/suspect-cli/src/commands/ci.rs:353-397`, `:402-438`) read the
  manifest JSON first and fall back to `for_invocation` only when the
  manifest has no value. `project init` writes `"lint": {"min_severity":
  "hint", "rules": {}}` (`crates/suspect-cli/src/commands/project.rs:386-390`),
  so for every generated project the manifest "has a value": `.suspect.yaml`
  `lint.rules` and `lint.min_severity` never reach CI.

CI also hides bad values. With the manifest rule `"info-contact": "fatal"`,
`suspect lint` exits 2 with `error: unrecognized lint severity: fatal`, but
`suspect ci --stage lint` drops *every* override silently
(`ci.rs:437`, `LintOverrides::from_config(..).unwrap_or_default()`; the
comment above it says "must be loud"): `info-license` falls back from
error to warning and nothing is printed.

### 2. The LSP holds two config objects, parsed from the same files twice

`Backend::reload_config` (`crates/suspect-lsp/src/lib.rs:153-178`) builds:

| Object | Built by | Holds | Used for |
|---|---|---|---|
| `editor_config::EditorConfig` (`editor_config.rs:17-26`) | `suspect_config::discover(root)` + `apply_client` (`:106-129`) | `lint.ruleset`, `lint.min_severity`, `lint.design`, `validate.strict_format` | ruleset and strict-format per publish (`lib.rs:239-240`), severity floor |
| `config_files::SuspectConfig` (`config_files.rs:87-100`) | `file_layers` + `merge` (`:282-384`) | `lint.rules`, `lint.design`, `lint.recommended`, `ref.max_docs`, inlay hints, formatting, extensions | per-rule and design overrides (`diagnostics.rs:160-186`), inlay hints (`lib.rs:924-927`), formatting (`lib.rs:2465`), `max_docs` (`lib.rs:2541`) |

Each reload reads and parses `.suspect.yaml` twice (`file::load` through
`discover`, then `read_settings_value` with a second `LowDoc::parse`) and
the manifest twice (`load_manifest_policy`, `read_manifest_policy`). The
two paths disagree on errors: `discover` rejects a file with syntax errors
and `EditorConfig.error` reports it, while `read_settings_value` ignores
`syntax_errors()` and applies whatever partial tree parsed.

`lint.design` is held by both and read from `SuspectConfig` only.

The LSP also has its own severity parser. `EditorConfig::min_severity`
(`editor_config.rs:85-94`) maps any unknown name to `HINT`. With
`lint: {min_severity: fatal}`, `suspect lint` exits 2
(`lint.min_severity: unrecognized severity 'fatal'`), and the server
publishes every finding with no message. The "Already fixed" entry
"one parser now lives in `suspect-config`" holds for the CLI only.

`initializationOptions` never reach `EditorConfig`: `reload_config` passes
only the pulled client section to `for_workspace` (`lib.rs:164`), and
`pending_init_options` only to `merge` (`lib.rs:165-169`).

### 3. Discovery root differs

- CLI: `for_invocation` (`crates/suspect-config/src/lib.rs:331-339`; the
  short version cites `:276-284`, which is stale) calls `discover(".")`
  first and uses the input only when nothing is found above the cwd. A
  manifest-only hit above the cwd also counts as found.
- LSP: one `Resolved`-equivalent per server, from `st.root` (`rootUri`,
  `lib.rs:700`) or the workspace root, for every document.

Reproduction: from `/tmp/p2-repro/other` (its own `.suspect.yaml` with
`lint.min_severity: error`):

```text
$ suspect config ../proj/openapi.yaml
config: ./.suspect.yaml
  lint.min_severity: error
$ suspect lint ../proj/openapi.yaml --format json   # 1 finding, all Error
```

The project's own `.suspect.yaml` and manifest are both ignored. The help
for `suspect config [INPUT]` says it reports "the configuration that would
apply to this input, instead of the one above the working directory"
(`crates/suspect-cli/src/lib.rs:502-507`), which is the opposite of what it
does.

Credentials use a third rule: `suspect auth check` discovers
`.suspect/credentials.json` from the cwd (`commands/auth.rs:37-43`), and
`suspect test` from the Arazzo document's directory (`commands/test.rs:69-84`).

### 4. Most commands do not read config, and some settings are read by nobody

Readers of `suspect_config` in the CLI (grep for `suspect_config::` in
`crates/suspect-cli/src`): `validate` (`lib.rs:585`), `docs` (`:602`),
`lint` (`:638`), `fmt` (`:693`), `config` (`:887`), `codegen`
(`commands/codegen_cmd.rs:73-76`), `ci` (`commands/ci.rs:365,376,420`).
`rules` uses only `parse_severity` (`commands/rules.rs:21`).

Flag-only today (from `suspect --help`): `acquire`, `auth`, `check`,
`admission`, `breaking`, `stubs`, `arazzo-diff`, `overlay-dry-run`,
`upgrade`, `rules`, `overlay`, `stats`, `bundle`, `diff`, `drift`, `har`,
`bench`, `bridge`, `reverse`, `stateful`, `test`, `load`, `fuzz`, `why`,
`replay`, `gen`, `watch`, `gateway`, `codegen-terraform`, `codegen-cli`,
`codegen-mcp`, `codegen-profiles`, `codegen-session`, `codegen-compare`,
`release-plan`, `impact`, `release-publish`, `release-workflow`,
`evidence`, `contract`, `project`.

Most of these have no project-level setting, and flag-only is right for
them. The ones that matter are those whose flags duplicate a setting that
exists elsewhere:

- `validate.strict_format` is ignored by `ci`'s validate stage
  (`ci.rs:458`, `validate_file(&spec, None, false)`) and by `project build`
  (`project.rs:567`, `validate_workspace(&ws, spec, false)`).
- The base URL has five sources and two defaults: `test`, `fuzz`,
  `stateful --base-url` (clap default `http://localhost:8080`), the
  manifest's `tests.base_url` (`project init` writes `http://127.0.0.1:8080`,
  `project.rs:380`), VS Code `suspect.testBaseUrl`, and the LSP run lenses
  (`run_lenses.rs:403-435`: init options, then `SUSPECT_BASE_URL`, then
  `http://localhost:8080`). VS Code sends no `initializationOptions`
  (`editors/vscode/src/extension.ts:99-111`), so the lenses never see the
  editor setting or the manifest.
- The git baseline: `ci --baseline` and `breaking --baseline` (flags),
  VS Code `suspect.ci.baseline`, and `SUSPECT_GIT_BASE` read separately by
  the server (`lsp/src/lib.rs:2745`, default `HEAD~1`) and by the extension
  host (`editors/vscode/src/serverCommands.ts:106`), which can see
  different environments.
- `lint.recommended` is documented (`lsp/src/config_schema.rs:234`,
  "Apply the recommended lint rule set", default `true`), parsed
  (`config_files.rs:231`), written by `project init`, and read by nothing:
  `lint_recommended()` has no caller outside `config_files.rs`.
- `ProjectManifest.policy` (`project.rs:42-46`, filled at `:337`) is never
  read.

### 5. The schema is described three times

1. `suspect_config::Settings` and the hand-written parser in `file.rs:97-189`.
2. `SuspectConfig` + `parse_config` (`config_files.rs:214-272`), with extra
   keys the CLI does not know (`lint.recommended`, `ref.max_docs`,
   `inlay_hints.*`, `formatting.sort_keys`).
3. The LSP's hover/completion/validation tables, `SETTINGS` and `PROJECT`
   in `lsp/src/config_schema.rs:129` and `:327`, hand-authored "because the
   documentation lives in prose on the Rust structs" (`:1-8`).

`.suspect.yaml` has both `format:` (CLI `fmt` output, `json`/`yaml`) and
`formatting:` (LSP `sort_keys`).

### 6. Environment variables

`grep -rhoE 'SUSPECT_[A-Z0-9_]+'` over `crates xtask editors rules-runtime
.github` finds 253 distinct names. Almost all are test-gate inputs. The
ones the shipped binary reads (non-test code in `crates/*/src`):

| Variable | Read at | Class |
|---|---|---|
| `SUSPECT_LINT_RULESET` | `cli/src/lib.rs:644` | setting, CLI `lint` only |
| `SUSPECT_LINT_MIN_SEVERITY` | `cli/src/lib.rs:651` | setting, CLI `lint` only (not `ci`, not LSP) |
| `SUSPECT_STRICT_FORMAT` | `cli/src/lib.rs:589` | setting, CLI `validate` only |
| `SUSPECT_CREDENTIALS` | `cli/src/commands/auth.rs:39`, `test.rs:69` | setting, two readers |
| `SUSPECT_BASE_URL` | `lsp/src/run_lenses.rs:434` | setting, LSP only; `suspect test` ignores it |
| `SUSPECT_GIT_BASE` | `lsp/src/lib.rs:2745`, `editors/vscode/src/serverCommands.ts:106` | setting, LSP + extension host |
| `SUSPECT_PROFILE` | `lint/src/engine.rs:113,182`, `lint/src/fast.rs:236`, `validate/src/checks/mod.rs:92`, `checks/schemas.rs:13` | debug |
| `SUSPECT_GW_PROFILE` | `gateway/src/lib.rs:268` | debug |
| `SUSPECT_BUILD_PROFILE` | `cli/src/commands/contract.rs:109`, `project.rs:505` | debug |
| `SUSPECT_CONTRACT_TRACE` | `ir/src/contract/compile.rs:40,272` | debug |
| `SUSPECT_RULES_DEBUG` | `rules/src/host.rs:219,294`, `worker.rs:143-235`, `rules-runtime` | debug |
| `SUSPECT_HLDBG` | `lsp/src/semantic.rs:1061` | debug (missing from the short version) |

Test-only, not part of the registry: `HOVER_ATTEMPT_MS` (inside
`mod burst_tests`, `lsp/src/lib.rs:3489-3522`; the short version lists it
as a product variable, which is wrong), `SUSPECT_LATENCY_FULL`
(`lsp/src/latency.rs:75`, bench module), `SUSPECT_BLESS`,
`SUSPECT_TEST_*`, every `suspect_codegen::toolchain` override
(`SUSPECT_JAVA_BIN`, `SUSPECT_DOTNET_BIN`, ... in `toolchain.rs`, used only
by `#[ignore]` gates), and the xtask and editor smoke inputs.
`SUSPECT_GO_BIN` appears in `codegen/src/go_http/docs.rs:1228` inside a
Python script the generator emits; it is part of generated output.

`.github/actions/suspect/action.yml:71-82` exports `SUSPECT_COMMAND`,
`SUSPECT_PATHS`, `SUSPECT_BASE_URL`, `SUSPECT_REPORT`, `SUSPECT_FILTER`,
... into the step environment as shell variables. `SUSPECT_BASE_URL` there
already shares a name with the LSP setting; any of the others would
silently become a setting if the registry later used the name.

### 7. Config file formats

| File / flag | Parsed at | Parser | What it is |
|---|---|---|---|
| `.suspect.yaml` (+3 names, `file.rs:11-16`) | `suspect-config/src/file.rs:97` | `LowDoc` | tool settings |
| `suspect.project.json` | `cli/src/commands/project.rs:152`; policy also in `file.rs:75`, `config_files.rs:293`, `ci.rs:355,403` | `serde_json` (4 readers) | what to build |
| `release.json` (`release-publish`, `release-workflow --manifest`) | `cli/src/commands/publish.rs:148` | `serde_json::Value` | what to publish |
| `gen.toml` (`gen --manifest`) | `suspect-gen/src/orchestrate.rs:50` | hand-written TOML subset: `[[output]]` with `template`/`target`, other tables silently ignored | template set (an input) |
| `codegen-session --config` | `cli/src/commands/codegen_session.rs:89` | serde | session: source + options |
| `codegen --defaults` | `cli/src/commands/codegen_cmd.rs:118` | serde `GenerationOptions` (`codegen/src/backend/options.rs:11`) | generation options, same schema as the session config |
| `codegen-terraform --mapping/--target-config` | `cli/src/commands/terraform_cmd.rs:136` | serde | closed, versioned generator inputs |
| `codegen-cli/-mcp --mapping/--target-config` | `cli/src/commands/application_cmd.rs:253-257` | serde | closed, versioned generator inputs |
| `.suspect/credentials.json` | `suspect-test/src/auth.rs` | serde | secrets, usually git-ignored |

Only the first three describe tool behaviour or a project. The mapping and
target-config files are generator inputs, like a ruleset or a load
profile, and should stay separate files.

## Goals and non-goals

Goals:

- One entry point, `suspect_config::resolve`, used by the CLI, CI and the
  LSP. Deleting `lint_policy`, `lint_overrides`, `EditorConfig`,
  `file_layers`, `read_manifest_policy`, `read_settings_value`,
  `SuspectConfig::overlay`, and `config_files::merge`.
- One precedence: `flag | editor > env > .suspect.yaml > default`, and
  per-key merging of maps (`lint.rules`) at every boundary.
- One discovery rule: nearest config above the input, then the cwd. The LSP
  uses the same rule per document.
- A bad value is an error with `file:line` everywhere: CLI exit 2, a
  failing CI stage that names it, an LSP diagnostic on the config file.
- `suspect config --explain` with the source of every value.
- Every `SUSPECT_*` variable the binary reads is declared in one table.
- A JSON Schema for `.suspect.yaml` and `suspect.project.json`, generated
  and checked in.
- Option A: policy only in `.suspect.yaml`, with migration.

Non-goals:

- Renaming VS Code settings to match config keys (P4 owns it; this proposal
  gives the table an `editor` column so P4 can generate them).
- What the check pass runs (P1). P1 consumes `Resolved.settings`.
- Config inheritance across directories beyond one explicit `extends`.
- Making every flag a setting. Only values a project wants to commit.
- Changing the mapping/target-config/credentials formats.

## Design

### Module layout (`crates/suspect-config`)

```text
src/lib.rs        Settings (typed), re-exports; for_invocation kept as a deprecated wrapper for one PR
src/schema.rs     NEW  static SETTINGS / MANIFEST / DEBUG_VARS tables
src/layer.rs      NEW  Origin, Layer, layer readers (file, manifest, env, flags, editor)
src/resolve.rs    resolve(), resolve_with(), Resolved, explain(); Resolution kept for callers
src/discover.rs   NEW  discovery walk (replaces file::discover)
src/file.rs       YAML/JSON -> Layer, with line numbers
src/json_schema.rs NEW JSON Schema emitter from schema.rs
src/severity.rs   unchanged
```

`schema.rs` is where P1 also moves `config_schema::problems` and the LSP
schema tables (P1 table "Code that moves out of `suspect-lsp`"). The LSP
keeps hover and completion and reads the table from here.

### The setting table

```rust
// crates/suspect-config/src/schema.rs (new)
pub struct Setting {
    /// Dotted path in `.suspect.yaml`: "lint.min_severity".
    pub path: &'static str,
    pub kind: Kind,
    /// Default as written in YAML, for docs and the schema; None = unset.
    pub default: Option<&'static str>,
    pub doc: &'static str,
    /// Environment variable, if any: "SUSPECT_LINT_MIN_SEVERITY".
    pub env: Option<&'static str>,
    /// Client settings key under `suspect.` (today's camelCase spelling).
    pub editor: Option<&'static str>,
    /// Flags that set it, for `--explain` and the CLI reference.
    pub flags: &'static [&'static str],
    /// Older spellings still accepted, with a warning.
    pub aliases: &'static [&'static str],
}

pub enum Kind {
    Bool, Str, Path, Url, Port, Count,
    Enum(&'static [&'static str]),
    Severity,        // error|warning|info|hint (parse_severity, rejects off)
    PolicySeverity,  // the same plus off
    Map(&'static Kind), // lint.rules: rule id -> PolicySeverity
}

pub static SETTINGS: &[Setting] = &[
    Setting {
        path: "lint.min_severity", kind: Kind::Severity, default: Some("hint"),
        doc: "Minimum severity to report.",
        env: Some("SUSPECT_LINT_MIN_SEVERITY"), editor: Some("lint.minSeverity"),
        flags: &["lint --min-severity"], aliases: &[],
    },
    // ...
];

pub struct DebugVar { pub name: &'static str, pub doc: &'static str }
pub static DEBUG_VARS: &[DebugVar] = &[/* SUSPECT_PROFILE, ... */];
```

The typed `Settings` struct stays: consumers read
`resolved.settings.lint.min_severity`, not strings. A test (below) keeps
the table and the struct in step. This uses the table the LSP already has
(`config_schema.rs` `Field { path, kind, doc, default, values }`) rather
than adding `schemars`: the sentences are already written, the LSP needs
the table at runtime, and it carries env/editor/flag columns a derive
cannot.

### Keys

Existing keys keep their spelling. New keys (P2.3), each replacing a flag
default, an env var or an editor-only setting that exists today:

| Key | Default | Env | Editor (today) | Flags / readers |
|---|---|---|---|---|
| `lint.ruleset` | unset | `SUSPECT_LINT_RULESET` | `lint.ruleset` | `lint --ruleset`; `ci`; LSP |
| `lint.min_severity` | `hint` | `SUSPECT_LINT_MIN_SEVERITY` | `lint.minSeverity` | `lint --min-severity`; `ci`; LSP |
| `lint.design` | unset | | `lint.design` | `lint`, `ci`, LSP |
| `lint.rules.<id>` | `{}` | | `lint.rules` | `lint`, `ci`, LSP; merged per key |
| `lint.recommended` | `true` | | `lint.recommended` | wired in `lint`, `ci`, LSP (today: nobody) |
| `validate.strict_format` | `false` | `SUSPECT_STRICT_FORMAT` | `validate.strictFormat` | `validate --strict-format`; `ci`, `project build` (today: hard-coded `false`), LSP |
| `validate.reference_allowlist` | unset | | | `validate --reference-allowlist` |
| `format.json`, `format.yaml` | `false` | | | `fmt --json/--yaml` |
| `format.sort_keys` (alias `formatting.sort_keys`) | `false`, `true` if a `format` section exists | | `formatting.sortKeys` | LSP formatting |
| `docs.style`, `docs.out`, `codegen.*` | as today | | | `docs`, `codegen` |
| `ref.max_docs` | `500` | | `ref.maxDocs` | LSP; later `WorkspaceBuilder` in the CLI |
| `editor.inlay_hints.refs` (alias `inlay_hints.refs`, `inlay_hints.ref_targets`) | `true` | | `inlayHints.refs` | LSP |
| `editor.inlay_hints.properties` (alias `inlay_hints.properties`) | `true` | | `inlayHints.properties` | LSP |
| `test.base_url` | `http://localhost:8080` | `SUSPECT_BASE_URL` | `testBaseUrl` | `test`, `fuzz`, `stateful`, `load --base-url`; LSP run lenses |
| `test.credentials` | discovered `.suspect/credentials.json` | `SUSPECT_CREDENTIALS` | | `test`, `load`, `auth check --credentials` |
| `test.report` | `text` | | | `test`, `load --report` |
| `gateway.port` | `8080` | | `gatewayPort` | `gateway --port` |
| `gateway.mode` | `mock` | | | `gateway --mode` |
| `gateway.upstream` | unset | | | `gateway --upstream` |
| `diff.baseline` | unset (LSP: `HEAD~1`) | `SUSPECT_GIT_BASE` | `ci.baseline` | `ci --baseline`; LSP `suspect.breakingChanges` |
| `rules.enabled` | `false` | | | LSP and `check` (P1) |
| `rules.roots` | `[".suspect/rules"]` | | | `rules check` |

The short version also proposed a `load` section. Everything `load` needs
beyond `test.*` lives in the load profile document it already takes, so no
`load` section is added. `breaking` does not read `diff.baseline`: a
configured default would change what `breaking OLD NEW` means.

The manifest keeps `tests.base_url` because it is the project's declared
target. For suites a manifest declares, it sits between env and the file:
`flag > env > manifest tests.base_url > .suspect.yaml test.base_url >
default`. So a CI job can point every project at a preview deployment with
one variable.

### Layers and origins

```rust
// crates/suspect-config/src/layer.rs (new)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Default,
    File { path: PathBuf, line: u32 },     // .suspect.yaml:12
    Manifest { path: PathBuf, line: u32 }, // deprecated, release N only
    Env { var: &'static str },
    Editor { key: String },                // "suspect.lint.minSeverity"
    Flag { name: &'static str },           // "--min-severity"
}

/// One layer: setting path -> raw value with its origin. Map settings are
/// flattened to "lint.rules.<id>", so every merge is per key by construction.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    entries: BTreeMap<String, (Raw, Origin)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Raw { Bool(bool), Str(String), Int(u64), Path(PathBuf) }

impl Layer {
    /// CLI flags. `None` means "not passed" and adds nothing.
    pub fn flags() -> Self;
    pub fn set(self, path: &'static str, value: Option<Raw>, flag: &'static str) -> Self;
    /// A client `suspect` section (initializationOptions or
    /// workspace/configuration), camelCase or snake_case keys. Blank strings
    /// and null are "not set" (today's `set_string`, editor_config.rs:133).
    /// Relative paths resolve against `base` (today's rule, editor_config.rs:50-56).
    pub fn from_editor(section: &serde_json::Value, base: &Path) -> Self;
}
```

File layers come from `file.rs`, now producing a `Layer` with
`Origin::File { line: node.line_col().0 + 1 }` (`suspect_low::NodeRef::line_col`,
`crates/suspect-low/src/node.rs:370`). Relative paths resolve against the
declaring file's directory, as today (`file.rs:113`). The manifest is read
with `LowDoc` too (JSON parses as YAML), so its values also get lines.

Merging is one loop: for each layer from lowest to highest, insert every
entry. The typed `Settings` is then built from the merged entries, checked
against `Kind`. Unknown keys produce a warning with `file:line` (the LSP
already reports them through `config_schema`; the CLI starts to).

### `resolve`

```rust
// crates/suspect-config/src/resolve.rs
pub struct Resolved {
    /// Effective, validated settings.
    pub settings: Settings,
    /// Directory of the nearest config file, else the start directory.
    pub root: PathBuf,
    /// Every file read, lowest first (for watchers and the status bar).
    pub files: Vec<PathBuf>,
    /// Deprecations and unknown keys, each with an origin.
    pub warnings: Vec<Notice>,
    origins: BTreeMap<String, Origin>,
    trace: BTreeMap<String, Vec<(Origin, Raw)>>, // every layer's value, for --explain
}

pub struct Notice { pub origin: Origin, pub message: String }

impl Resolved {
    pub fn origin(&self, path: &str) -> &Origin;
    pub fn explain(&self) -> Vec<ExplainRow>; // one row per SETTINGS entry and per lint.rules key
}

pub struct Inputs<'a> {
    /// CLI flags: the top layer.
    pub flags: Layer,
    /// LSP: initializationOptions, then the pulled `suspect` section (wins).
    pub editor: &'a [&'a serde_json::Value],
    /// Process environment by default; tests pass a map.
    pub env: Env,
}

/// Files + environment + defaults, discovered from `start`.
pub fn resolve(start: &Path) -> Result<Resolved, ConfigError>;
/// The same with flag or editor layers on top.
pub fn resolve_with(start: &Path, inputs: Inputs<'_>) -> Result<Resolved, ConfigError>;
```

`ConfigError` carries every problem, each with an `Origin`, not only the
first. Syntax errors drop that file's layer; a bad value drops that key.
The CLI exits 2 on any error. The LSP keeps working on the rest and shows
the errors as diagnostics on the config file.

Layer order, lowest first:

```text
default < manifest policy (release N only, warned) < extends file < .suspect.yaml
        < env < editor (initializationOptions < workspace/configuration) < flags
```

The editor layer sits where flags sit, not below env. P4 makes the same
choice for the same reason: an editor setting the user just picked must
beat a variable in a shell profile. A process never has both editor and
flag layers.

`Inputs` matches the shape P4 sketches (`resolve_with(root, Inputs)`), with
the editor input as a slice so init options and the pulled section stay
two layers.

### Discovery

```rust
// crates/suspect-config/src/discover.rs (new)
pub struct Found { pub config: Option<PathBuf>, pub manifest: Option<PathBuf> }
pub fn discover(start: &Path) -> Found;
```

Walk up from `start` (a file's directory, or the directory itself). In
each directory:

1. A `CONFIG_NAMES` file: use it and stop.
2. Else a `suspect.project.json` with `"config": "<path>"`: use that file
   and stop.
3. Remember the first manifest seen (for the deprecated policy layer and
   the base URL), keep walking.

The walk still runs to the filesystem root. `start` is:

- CLI: the first input path; when there is no input, the cwd. When the
  input has no config above it, fall back to the cwd (the short version's
  rule). When later inputs resolve to a different config file, print
  `warning: <input> is governed by <other>; using <first> for this run`.
- CI and `project` commands: the manifest's directory.
- LSP: each document's directory. Results are cached per discovered
  config file (`HashMap<Option<PathBuf>, Arc<Resolved>>`) and dropped on a
  watched config file change (`lib.rs:1095-1113` already does this
  trigger). A monorepo with per-project `.suspect.yaml` then gets the same
  answer in the editor as `suspect lint` gives.

`.suspect.yaml` may name one parent with `extends: ../.suspect.yaml`
(relative to the file, one level, cycles rejected). It becomes a file
layer under the extending file. This exists for the option A migration in
monorepos (see Compatibility).

### CLI flags as the top layer

```rust
// crates/suspect-cli/src/lib.rs, Command::Lint (after)
let flags = Layer::flags()
    .set("lint.ruleset", ruleset.map(Raw::Path), "--ruleset")
    .set("lint.min_severity", min_severity.map(|s| Raw::Str(s.label().into())), "--min-severity");
// start, usage_error, emit_warnings and from_settings are new helpers.
let resolved = suspect_config::resolve_with(start(&paths)?, Inputs { flags, ..Inputs::default() })
    .map_err(usage_error)?;
emit_warnings(&resolved);
let lint = &resolved.settings.lint;
let overrides = LintOverrides::from_settings(lint); // no string re-parsing
commands::lint::lint(&paths, lint.ruleset.as_deref(), lint.min_severity.into(), &overrides, text.format)
```

The 40 lines of `Resolution::new` plumbing at `lib.rs:638-679` go away, as
do the per-field `if args.x.is_none()` blocks for `docs`
(`lib.rs:602-623`) and `codegen` (`codegen_cmd.rs:73-97`). Clap `bool`
flags add `Raw::Bool(true)` only when passed; `resolve_bool`'s documented
limitation (`resolve.rs:98-100`) stays, and a `--no-strict-format` is a P3
question.

`ci` calls `resolve(manifest.parent())` once per project and passes
`resolved.settings` to the lint and validate stages. `lint_policy`,
`lint_overrides` and the `false` at `ci.rs:458` and `project.rs:567` are
deleted.

### LSP: one `Resolved`

`State.config: SuspectConfig` and `State.editor_config: EditorConfig`
(`state.rs:217,230`) become one field:

```rust
pub resolved: ResolvedCache, // per-config-file Arc<Resolved>, plus the client layers
```

`reload_config` (`lib.rs:153-178`) becomes:

```rust
let init = st.pending_init_options.clone();
let editor: Vec<&serde_json::Value> = init.iter().chain(client.iter()).collect();
let fresh = ResolvedCache::new(&editor); // new; documents resolve lazily
let changed = st.resolved.fingerprint() != fresh.fingerprint();
```

`diagnostics::lint_diagnostics` takes `&suspect_config::LintSettings`
instead of `Option<&LintCfg>`; the severity floor uses `SeverityName`
(deleting `EditorConfig::min_severity`). `config_files.rs` keeps only the
file-operation half (`:386` onward). `extensions` (today in
`SuspectConfig`) moves into `Settings` as an opaque section owned by
`extensions_config.rs`.

Config errors and deprecations are published as diagnostics on the config
file through the existing `config_schema` path, so a broken `.suspect.yaml`
is visible where it is edited.

### `suspect config --explain`

```text
$ suspect config --explain openapi.yaml
root     /repo/api  (discovered from openapi.yaml)
files    /repo/.suspect.yaml (extends)
         /repo/api/.suspect.yaml

lint.min_severity          warning                 /repo/api/.suspect.yaml:2
lint.ruleset               /repo/policy/rules.yaml env SUSPECT_LINT_RULESET
lint.rules.info-contact    error                   /repo/api/.suspect.yaml:5  (over suspect.project.json:24 = off)
lint.rules.info-license    error                   suspect.project.json:25    deprecated location
validate.strict_format     false                   default
test.base_url              http://localhost:8080   default
...

debug variables (not settings)
SUSPECT_PROFILE            unset   lint/validate phase timings on stderr
...

warning: suspect.project.json:22: `lint` in a project manifest is deprecated; ...
```

- Every `SETTINGS` row is printed, set or not; `--set-only` hides defaults.
- `--format json` emits `{root, files, values: [{key, value, origin, trace}], warnings}`.
  P4's `suspect/explainConfig` request returns the same JSON from the
  server's cache, including the editor layer the CLI cannot see.
- `suspect config` without `--explain` keeps today's short summary
  (`Loaded::describe`, `lib.rs:140-183`), rendered from `Resolved`.
- Flags cannot appear in `suspect config --explain` (it has none for the
  other commands). Each row lists the flags that would override it, from
  `Setting.flags`.

### Env var registry

The registry is the `env` column of `SETTINGS` plus `DEBUG_VARS`. Rules:

- The binary reads a `SUSPECT_*` variable only through `suspect-config`
  (`Env::get(&'static str)`), except debug variables, which keep their
  `std::env::var_os` call in the owning crate but must be listed.
- `SUSPECT_GIT_BASE` becomes the env name of `diff.baseline`; the extension
  host stops reading it and asks the server (`suspect/explainConfig`, P4).
- `SUSPECT_BASE_URL` becomes the env name of `test.base_url` and starts
  applying to `suspect test`, `fuzz`, `stateful` and `load` as well as the
  lenses.
- Test-only variables are not registered. A test enforces the boundary
  (Testing).
- The GitHub Action renames its shell variables to `INPUT_*`
  (`action.yml:71-82`) so they cannot collide with a registered name.

### Manifest and config boundary (option A)

The manifest gains one key:

```json
{ "version": 1, "name": "my-api", "entry": "openapi.yaml", "config": ".suspect.yaml", ... }
```

`lint`, `validate` and `editor` leave the manifest. `project init` writes
`.suspect.yaml` (lint, validate, editor sections with today's values) and
`"config": ".suspect.yaml"` instead of the policy block at
`project.rs:383-393`. `project check` verifies the referenced file exists
and parses.

The crate doc (`lib.rs:17-20`, "a manifest is deliberately *not*
configuration") becomes true again, and `Loaded::from_manifest_policy`
(`lib.rs:194-254`) is deleted after release N.

### JSON Schema

`suspect_config::json_schema::settings()` and `::manifest()` (new) emit
draft 2020-12 schemas from `SETTINGS` and `MANIFEST`. Each property has
`description` (from `doc`), `default`, `enum` (from `Kind`), and the
annotations P4 asked for: `x-suspect-editor` when `editor` is set, and
`x-suspect-env` when `env` is set. `lint.rules` is
`additionalProperties: {enum: [error, warning, info, hint, off]}`.

Checked in with the bless pattern of `crates/suspect-cli/tests/cli_reference.rs`:

```rust
// crates/suspect-config/tests/json_schema.rs (new)
#[test]
fn the_checked_in_schemas_match_the_setting_table() {
    for (file, schema) in [
        ("suspect.schema.json", suspect_config::json_schema::settings()),
        ("suspect.project.schema.json", suspect_config::json_schema::manifest()),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/schemas").join(file);
        let expected = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        if std::env::var_os("SUSPECT_BLESS").is_some() {
            std::fs::write(&path, &expected).unwrap();
            continue;
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), expected,
            "{file} is stale; regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-config --test json_schema`");
    }
}
```

The files live in `editors/vscode/schemas/` because `vsce` packages only
the extension directory and P4's settings generator reads them there.
The workspace `serde_json` has no `preserve_order` (P4 notes this), so
object keys come out sorted. That is stable, which is all the bless test
needs.

VS Code ships them through `package.json`:

```json
"jsonValidation": [{ "fileMatch": "suspect.project.json", "url": "./schemas/suspect.project.schema.json" }],
"yamlValidation": [{ "fileMatch": [".suspect.yaml", ".suspect.yml", "suspect.yaml", "suspect.yml"],
                     "url": "./schemas/suspect.schema.json" }]
```

`jsonValidation` is core VS Code. `yamlValidation` is read only by the Red
Hat YAML extension; without it, the suspect server's own `config_schema`
validation (now driven by the same table) still covers `.suspect.yaml`.
SchemaStore gets the raw URL of the tagged file once P5 publishes releases.

### Other file formats

Realistic folds, all in step 8 (separate, deprecation-gated):

- `release.json` becomes the manifest's `release:` section with the same
  fields. `release-publish`/`release-workflow --manifest` accept either
  file; with a `suspect.project.json` they read `release`. `release.json`
  keeps working with a one-release warning.
- `codegen --defaults` and `codegen-session --config` already share the
  `GenerationOptions` schema. A manifest `codegen[]` target gains
  `options` with that schema; `--defaults` stays as an explicit override
  file.
- Every serde-parsed input (`--defaults`, `--config`, `--mapping`,
  `--target-config`) accepts YAML by parsing with `LowDoc` and converting
  through `suspect_overlay::Value::to_json` (as `config_files.rs:354` does
  today), then `serde_json::from_str`. JSON input is unchanged.
- `gen --manifest` accepts `gen.yaml`/`gen.json` by extension. The
  hand-written TOML subset reader stays without a deprecation: the value
  is a real parser for the new formats, not the rename, and removing TOML
  would break users for no gain. The TOML reader starts rejecting unknown
  tables instead of skipping them.

## Compatibility and migration

Behaviour changes, all in the release that lands steps 3-5 ("release N"):

| Change | Who notices | Mitigation |
|---|---|---|
| CLI discovers from the input, not the cwd | users running against files outside the cwd's tree | `suspect config --explain FILE` shows the file used; release notes |
| CI: `.suspect.yaml` beats manifest policy, per key | projects with policy in both files | a one-time CI note when the result differs (below) |
| LSP: per-key merge, per-document config, env vars apply | editor users with policy in both files, monorepos | same values as `suspect lint`, which is the point |
| Unknown severities are errors in CI and the LSP | configs with typos | the message names `file:line` |
| `validate.strict_format` applies in `ci` and `project build` | projects with it set to `true` | it was already documented as project policy |
| `lint.recommended: false` takes effect | anyone who set it | it was documented |
| `SUSPECT_BASE_URL` applies to `test`/`fuzz`/`stateful`/`load` | shells that export it | a flag still wins |

Aliases kept for one release, each with a warning naming the new key:
`formatting.sort_keys` -> `format.sort_keys`, top-level `inlay_hints.*` ->
`editor.inlay_hints.*`, `inlay_hints.ref_targets` -> `editor.inlay_hints.refs`,
`ref`/`refs` with camelCase `maxDocs`.

### Deprecation warning (option A)

Fires in release N when `resolve` reads a manifest whose `lint`,
`validate` or `editor` key is present (even empty, so every project made by
today's `project init` is told once). It does not fire for a manifest with
only `config`. Text, one line per key:

```text
warning: suspect.project.json:22: `lint` in a project manifest is deprecated and is ignored from the next release; move it to .suspect.yaml and add "config": ".suspect.yaml" to the manifest (`suspect config migrate` does both)
```

Where it appears:

- CLI: on stderr, once per manifest per process, from every command that
  resolves config. Never in JSON/SARIF stdout, so report schemas do not
  change.
- `suspect ci`, in addition, when the CI-effective value changes because of
  the new order:
  `note: suspect.project.json:24: lint.rules.info-contact is now error (.suspect.yaml:3); earlier releases applied off from the manifest`.
  Both layers are in `Resolved.trace`, so this is a comparison, not a
  second merge.
- LSP: a Warning diagnostic on the key in `suspect.project.json` (code
  `config-manifest-policy`), and one `window/logMessage` per session.
- `suspect project check`: a warning finding in release N, an error in
  N+1.

In release N, manifest policy still applies as the lowest file layer, with
`discover`'s current per-key semantics, now in every tool. In N+1 it is
ignored and the warning text changes to "is ignored".

### `suspect config migrate [MANIFEST]`

New. Moves the manifest's `lint`, `validate` and `editor` sections into
the config file discovery finds from the manifest directory, keeping the
file's value where both carry one (the release-N order), adds `"config"`,
and prints every key moved or dropped. Cases:

- No config file found: create `.suspect.yaml` beside the manifest.
- Found beside the manifest: append the missing keys.
- Found above the manifest (a shared monorepo file): create a project
  `.suspect.yaml` with `extends: <relative path to the shared file>` plus
  the manifest's policy, so the project keeps both.

Comments in an existing `.suspect.yaml` are kept; new keys are appended as
YAML text, not by re-serializing the file.

## Implementation plan

Each step leaves `main` green and shippable.

1. **Table, layers, `resolve`** (suspect-config only). `schema.rs`,
   `layer.rs`, `discover.rs`, `resolve.rs`; `file.rs` produces layers with
   lines; manifest read via `LowDoc`. `for_invocation` and `discover`
   become wrappers over the new code with today's semantics (cwd first) so
   no caller changes. Tests: unit tests per layer; new
   `crates/suspect-config/tests/resolve.rs` precedence matrix
   (default/manifest/file/env/editor/flag x scalar/bool/map), table vs
   struct round-trip. 1 day.
   **Done.** Deviations: the legacy `for_invocation`/`discover`/`Loaded`
   stayed as they were (not wrappers) until steps 3-4 moved their callers,
   because `resolve` rejects bad values that the legacy callers reported
   themselves; the old precedence enum `resolve::Layer` is now `LayerKind`
   (the name `Layer` is the new layer struct). `Kind` also has `Section`,
   `List`, `Object` (`extensions`) and `Targets` (manifest `codegen`); `Raw`
   also has `List` and `Json`. `resolve_with` returns
   `Err(ConfigError { problems, resolved })`, carrying the settings that did
   apply, so the LSP can keep working; `resolve_found` takes a discovery the
   caller already ran (the LSP cache key). `Settings` keeps canonical
   strings for `lint.min_severity`/`design`/`rules` (validated and
   normalized by `resolve`) with typed accessors (`floor()`,
   `design_severity()`, `rule_severities()`). `docs.output` with the
   `docs.out` alias (open question 8) landed here with the table.
2. **`suspect config --explain`** (`--format json`, `--set-only`) and
   the fixed help text for `config [INPUT]`. Files: `cli/src/lib.rs`
   (`Command::Config`). Tests: `crates/suspect-cli/tests/config_explain.rs`
   with a tempdir fixture asserting origins incl. `file:line`. 0.5 day.
   **Done.** `suspect config` itself discovers from INPUT (falling back to
   the working directory when nothing configures it) from this step on;
   with `--explain`, rejected values are listed as `error:` lines and the
   exit code is 2. Rows also name their env variable. The JSON adds
   `config`, `errors`, `layer` per value and a `debug` list.
3. **CLI and CI on `resolve`**. `validate`, `lint`, `fmt`, `docs`,
   `codegen`, `ci`, `project build` call `resolve_with`; input-first
   discovery; multi-config warning; delete `lint_policy`/`lint_overrides`;
   strict format in `ci` and `project build`; bad values fail the stage;
   `lint.recommended` wired. Files: `cli/src/lib.rs`,
   `commands/{ci,codegen_cmd,project,lint}.rs`. Tests: new
   `crates/suspect-cli/tests/config_agreement.rs` (the Problem 1 and 3
   reproductions as fixtures; `lint` and `ci` must report the same
   severities); existing `cli.rs` cases updated. 1 day.
   **Done.** `for_invocation` is deleted; the CLI helpers live in
   `cli/src/settings.rs` (`start`, `resolve`, `emit_warnings`). In `ci` a
   rejected value fails only the stage that owns the key (`lint.*` the lint
   stage, `validate.*` the validate stage; a broken file or `extends` both),
   rather than exiting 2, as the Goals ask ("a failing CI stage that names
   it"). `project build` resolves from the manifest directory and exits 2 on
   a rejected value. `fmt` keeps the rule that a passed `--json`/`--yaml`
   overrides both `format.json` and `format.yaml`. `codegen --profile` is
   still applied after resolve (the flag is a typed profile, not a string).
4. **LSP on `resolve`**. `ResolvedCache` in `state.rs`; delete
   `editor_config.rs`, the config half of `config_files.rs`, and
   `EditorConfig::min_severity`; `lint_diagnostics` takes `LintSettings`;
   per-document resolution; config errors as diagnostics; `config_schema`
   reads `suspect_config::schema`. Tests: port `editor_config.rs` tests to
   `suspect-config`; add an LSP case to `config_agreement.rs` using
   `tests/support/editor.rs` (same fixture, same severities as `lint`);
   `npm run test:host:smoke` unchanged and green. 1.5 days.
   **Done.** `ResolvedCache` lives in `config_files.rs` (state holds an
   `Arc` of it), keyed on the discovery result (plus the start directory
   when nothing configures it); `reload_config` builds a fresh cache and
   compares the workspace's and every open document's `Resolved` with the
   old one. `compute_diagnostics`, `pull_diagnostics`, `workspace_pull`
   (per-document settings through a closure) and `resolve_code_action` take
   `&suspect_config::Settings`; ruleset, strict format and floor come from
   it. `lint.recommended: false` now drops lint findings only without a
   `lint.ruleset`, as the CLI does. Rejected values are shown once per
   change with `window/showMessage` (`file:line: key: ...`) and, when the
   file is open, by the schema check, which now reads the shared table and
   also flags deprecated spellings. Not done: the `lsp_bench.rs` case and
   the host-smoke case (needs a VS Code host); the stdio agreement case is
   in `config_agreement.rs`.
5. **Option A**. Manifest `config` key in discovery and `project check`;
   deprecation warning (CLI, CI note, LSP diagnostic); `project init`
   writes `.suspect.yaml`; `extends`; `suspect config migrate`. Files:
   `suspect-config/src/{discover,layer}.rs`, `cli/src/commands/project.rs`,
   new `cli/src/commands/config_migrate.rs`, `lsp/src/config_schema.rs`.
   Tests: migrate fixtures (none / beside / above), warning text snapshot,
   `project init` output. 1 day.
   **Done.** `extends` landed in step 1. The deprecation text is
   `suspect_config::file::manifest_policy_warning`; `resolve` attaches it
   to the manifest layer, so every command that resolves prints it once
   (stderr), `project check` reports it as a `project-manifest-policy`
   warning (and `project-missing-config` / `project-config` errors for a
   missing or rejected settings file), and the editor shows it on the key
   (`config-manifest-policy`) and logs it with `window/logMessage` on each
   configuration change rather than once per session. The CI note is
   printed per project on stderr (so `--format json` runs, which silence
   stderr, do not show it). `config migrate` writes JSON by text: kept
   entries keep their text and order and are joined with the separator
   the file used between its first two keys; `"config"` becomes the first
   key. Tests are in `crates/suspect-cli/tests/config_migrate.rs` (none /
   beside / above, warning text, `project init`).
6. **Schema extension and env registry**. Keys in the table above; `test`,
   `fuzz`, `stateful`, `load`, `auth`, `gateway`, `rules` read
   `resolve_with`; LSP run lenses and `breakingChanges` read
   `test.base_url`/`diff.baseline`; extension host stops reading
   `SUSPECT_GIT_BASE`; Action variables renamed. Tests: the registry test
   (Testing); per-command flag-beats-config cases in `cli.rs`. 1.5 days.
   **Done.** The keys landed with the table in step 1; this step wires
   them. `test`, `fuzz`, `stateful` (`--base-url`), `test --report`,
   `test`/`auth check --credentials`, `gateway --port/--mode/--upstream`
   lost their clap defaults and resolve; `load` uses `test.base_url` only
   when something set it (its default does not beat the profile's
   `target`); `ci --baseline` falls back to the project's `diff.baseline`;
   `project build` applies env > manifest `tests.base_url` >
   `test.base_url` > default (the manifest default `http://127.0.0.1:8080`
   is gone); `rules check` scans `rules.roots` from the settings root
   instead of the cwd. `gateway.port` accepts `0` (a free port), as the
   flag does. `rules.enabled` is in the table and `Settings` for P1; nothing
   reads it yet. The LSP run lenses read `test.base_url` per document (the
   top-level `baseUrl` initialization option is no longer read;
   `run.baseUrl` is an alias of `test.base_url`) and `suspect.breakingChanges`
   reads `diff.baseline` and returns the baseline it used, which the
   extension host now shows instead of reading `SUSPECT_GIT_BASE`. The
   registry test is `crates/suspect-config/tests/env_registry.rs`; its
   harness list names the toolchain-gate and generator files. The
   flag-beats-config cases are in `config_explain.rs`.
7. **JSON Schema**. `json_schema.rs`, bless test, `editors/vscode/schemas/`,
   `jsonValidation`/`yamlValidation` in `package.json`. Tests:
   `crates/suspect-config/tests/json_schema.rs`; a `test/manifest.cjs`
   check that the two contribution entries point at existing files; one
   test validates `project init` output against the manifest schema. 0.5
   day.
   **Done.** `x-suspect-editor` is `{key, scope: "resource"}` (the key is
   today's client spelling, so P4's generator can map it); deprecated
   spellings are emitted with `deprecated: true` (sections as copies) so
   existing files keep validating; the manifest's `lint`/`validate`/`editor`
   are marked deprecated. Prose defaults ("the directory name") are left
   out of `default`. The `project init` check validates both generated
   files with `suspect-schema` (`config_migrate.rs`).
8. **Formats** (separate, deprecation-gated). Manifest `release:` section
   and `release.json` warning; `codegen[].options`; YAML for serde inputs;
   `gen.yaml`. Files: `commands/{publish,codegen_cmd,codegen_session,terraform_cmd,application_cmd,generate}.rs`,
   `suspect-gen/src/orchestrate.rs`. Tests: one YAML fixture per input in
   the existing `crates/suspect-cli/tests/*` suites. 1.5 days.
   **Done.** The shared reader is `cli/src/commands/structured.rs`
   (`.yaml`/`.yml` by extension, `LowDoc` -> `Value::to_json` -> the same
   serde parser). It serves `codegen --defaults`, `codegen-session --config`,
   `--mapping`/`--target-config` of `codegen-terraform`/`-cli`/`-mcp`,
   `test`/`load --inputs`, and the release manifest. A YAML syntax error
   carries the YAML position; a schema error in YAML is reported at 1:1
   without serde's position, which would point into the converted text.
   `release-publish`/`release-workflow --manifest` became optional: the
   default is `suspect.project.json` when it has `release` (or no
   `release.json` exists), then `release.json`. A document with `release`
   or `entry` is read as a project manifest; anything else is a standalone
   release manifest and prints `warning: <path>: a standalone release
   manifest is deprecated and is not read from the next release; ...`.
   The `release` section is closed (unknown keys are errors, from the
   `RELEASE_FIELDS`/`RELEASE_TARGET_FIELDS` tables); a standalone file
   still ignores unknown keys. `project check` reports a bad `release`
   section as `project-release`. `codegen[].options` merges with the
   flat `compatibility_profiles`/`credential_env`/`sdk_defaults` keys,
   which stay undeprecated; a key set both ways is an error. `gen
   --manifest` reads `.yaml`/`.yml`/`.json` as an `output:` list of
   `{template, target}` (closed, errors start with the line); the TOML
   reader now rejects any table other than `[[output]]`. Deviations:
   `release` is one `Kind::Object` row in `MANIFEST` rather than a
   section with rows, because `release.targets` is a list of objects and
   `Kind::Targets` is tied to the codegen fields; the JSON Schema spells
   it out from the two release tables, and the editor checks only that it
   is an object. Not converted: `gateway --scenario` (left for the P3 exit
   path work in the same file) and `.suspect/credentials.json` (discovered
   by name in `suspect-test`; the Non-goals keep its format).

Total: about 8.5 days (steps 1-7: 7 days). The short version's "1-2 days
for resolve + deletion" covered steps 1, 3 and 4 only and did not count
per-document LSP resolution or the CI behaviour note.

## Testing

- **Unit** (`suspect-config`): every `Kind` accepts and rejects the right
  values; aliases warn; `extends` cycle rejected; origins carry the right
  line; per-key map merge across all six layers.
- **Table vs struct**: for each `SETTINGS` row, write a non-default value
  into a temp `.suspect.yaml`, resolve, and assert `--explain` reports that
  key from `File` and the typed field changed. A row the struct ignores, or
  a struct field without a row, fails.
- **Registry**: `crates/suspect-config/tests/env_registry.rs` scans
  `crates/*/src` (skipping `*_tests.rs`, `tests*.rs`, `#[cfg(test)]`
  modules by a simple brace-depth scan, and files listed as test harnesses)
  for `"SUSPECT_` literals and asserts each is in `SETTINGS.env` or
  `DEBUG_VARS`. A new product variable must be declared.
- **Agreement** (`crates/suspect-cli/tests/config_agreement.rs`): the
  Problem 1 fixture; `suspect lint --format json`, `suspect ci --format
  json` and the LSP via `support::editor::Editor` must report identical
  severities for every code. The Problem 3 fixture: running from a sibling
  directory gives the same result as running inside the project. The
  `fatal` fixture: `lint` and `ci` exit with a usage error and the LSP
  publishes a diagnostic on `.suspect.yaml`.
- **Explain** snapshot (`config_explain.rs`), JSON shape included.
- **Schema** bless test; `project init` output validates.
- **Editor host smoke** (`npm run test:host:smoke`): add one case that
  edits `.suspect.yaml` and sees the floor change (the reload path already
  exists; the case guards the new cache).
- **CI**: no new job. The new tests run in the existing `cargo test` job;
  the `package.json` check runs in the VS Code job.

## Alternatives considered

- **Option B, manifest as a documented lowest file layer.** Decided against
  on 2026-10-29; listed for completeness. Release N behaves like B, so the
  migration has a working intermediate state.
- **`schemars` derive on `Settings`.** Not a dependency today (absent from
  `Cargo.lock`), and `suspect-config` has no `serde` dependency; it parses
  through `LowDoc`. A derive also cannot carry env, editor or flag names,
  and the LSP needs the table at runtime anyway.
- **`figment` or `config-rs` for layering.** They merge serde values but
  lose YAML line numbers, which `--explain` and the LSP diagnostics need,
  and they add a second YAML parser next to `suspect-low`.
- **A declarative macro generating struct, table and parser.** Removes the
  round-trip test, but error messages inside a large macro are poor and the
  struct becomes hard to read. Revisit if the table passes ~80 rows.
- **Share one merge helper but keep three call sites.** The call sites
  would still choose different inputs (CI reads the manifest JSON
  directly), which is how the orders diverged.
- **Keep cwd-first discovery and make the LSP match it.** The LSP has no
  meaningful cwd, and per-file results must match the editor.
- **Merge every `.suspect.yaml` up the tree (EditorConfig style).** Changes
  results for every existing layout; one explicit `extends` covers the
  migration case.

## Risks

- **CI results change on upgrade** for projects with policy in both files.
  The CI note names each changed key; the release notes include the
  `--explain` command.
- **Per-document LSP resolution** adds a directory walk per new document
  directory. It is cached per config file and invalidated only by watched
  config events. Not measured yet; step 4 adds a case to
  `crates/suspect-cli/tests/lsp_bench.rs`.
- **Env now applies in the editor.** A stale `SUSPECT_LINT_MIN_SEVERITY` in
  a login shell changes editor diagnostics. `--explain` and P4's status
  item show it; editor settings still win.
- **Table and struct drift** is caught by the round-trip test, not by the
  compiler.
- **Migrate on a hand-formatted `.suspect.yaml`** appends text; a file that
  already has a `lint:` block gets keys inserted under it by line, which
  can fail on flow-style YAML. Migrate then refuses and prints the keys.

## Open questions

1. **Should the LSP resolve per document or per workspace root?**
   Recommended: per document, cached per config file. Otherwise monorepos
   keep the Problem 3 disagreement in the editor.
2. **Multi-input CLI runs that span config files: one config or one per
   input?** Recommended: first input plus a warning, as designed. Per-input
   policy belongs to P1's check pass, which can group inputs by `Resolved`.
3. **Should discovery stop at a repository boundary (`.git`) or a
   `root: true` key?** Recommended: no; keep walking to the filesystem
   root as today. Add `root: true` only if someone reports a stray home
   directory config.
4. **`lint.recommended`: wire it or delete it?** Recommended: wire it
   (false and no ruleset means no Spectral rules), since `project init`
   already writes it, VS Code exposes it, and the Zed README documents it.
5. **What happens to manifest policy in N+1?** Recommended: ignored, with
   the warning reworded to "is ignored", and `project check` fails.
6. **Is `extends` in scope?** Recommended: yes, one level, because the
   monorepo migration otherwise forces copying the shared file.
7. **Where do the schemas live and get published?** Recommended:
   `editors/vscode/schemas/`, written by the bless test; SchemaStore points
   at the file on the latest release tag once P5 exists.
8. **Should `docs.out` (`.suspect.yaml`) and `docs.output` (manifest) share
   one spelling?** Recommended: yes, `output` in both, with `out` as an
   alias for one release; done in step 6.
