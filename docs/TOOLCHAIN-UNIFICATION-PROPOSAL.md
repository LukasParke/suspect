# Toolchain unification proposal

Status: **proposal** — design and alignment changes that need a decision
before implementation. Concrete defects found in the same review (crashes,
wrong exit codes, editor start-up, config reload, repo hygiene, CI gaps) were
fixed directly and are listed in [Already fixed](#already-fixed) for context.

## Goal

A user should meet one tool, not twenty-eight crates. Concretely:

1. **One finding model.** A problem in a spec is reported once, with one code
   and one severity, whether it surfaced in `suspect lint`, `suspect ci`, SARIF
   upload or the editor.
2. **One configuration.** Every tool reads the same settings with the same
   precedence, from the same root, and `suspect config` explains every value.
3. **One command grammar.** Flags, formats and exit codes mean the same thing
   in every command.
4. **One editor story.** One extension per editor, one binary acquisition
   path, one settings namespace that maps 1:1 onto the config schema.
5. **One install and release path**, and one doc entry point.

The parse / `$ref` pipeline already meets this bar: every tool loads through
`suspect_ref::WorkspaceBuilder` + `LowDoc::parse`. The proposals below bring
the rest of the toolchain to the same standard.

---

## P1. One finding model and one check pass

### Today

- About 30 diagnostic types: `suspect_validate::Diagnostic`,
  `suspect_lint::Finding`, `ArazzoDiagnostic`, `RefDiagnostic`,
  `OverlayDiagnostic`, `ContractDiagnostic`, `ProfileDiagnostic`
  (`error: bool`), `AdmissionFinding`, `TsFinding` (string severity), and ~10
  codegen types. The CLI (`suspect-cli/src/output.rs`) and the LSP
  (`suspect-lsp/src/diagnostics.rs`) each convert them independently.
- The CLI and LSP run **different check sets**. The LSP runs Arazzo
  validation, Swagger checks, description checks and load-profile checks
  (`diagnostics.rs:374-402`); no CLI command calls `validate_arazzo`. Custom TS
  rules (`suspect-rules`) run only under `suspect rules check` — not in `lint`,
  `ci`, or the editor.
- **The same problem is reported several times at different severities.** A
  missing `operationId` on `docs/demo/petstore.yaml` produces:

  | Command | Code | Severity |
  |---|---|---|
  | `validate` | `oas-operation-missing-operationId` | warning |
  | `validate` | `sdk-operation-missing-id` | info |
  | `lint` | `operation-operationId` | error |

- No suppression mechanism exists (no inline disable, no finding baseline).
  `baseline.rs` is a git-revision checkout for `breaking`; `silence.rs`
  redirects stdout.
- Codegen admission refusals appear only through the on-demand
  `suspect/generationContract` request, never as editor diagnostics.

### Proposal

1. **`suspect-diag` crate** with one `Finding { code, severity, message,
   range, document, source: Source, related: Vec<Related>, fix: Option<Fix> }`
   and one `Severity { Error, Warning, Info, Hint }` with one parser (accepts
   `warn`/`information`/`off`, rejects anything else loudly). SARIF, LSP,
   text, JSON and JUnit renderers are written once against it.
2. **One check pass**, `suspect_check::run(workspace, &Policy) -> Vec<Finding>`,
   that runs syntax, `$ref`, semantic validation, Spectral rules, Arazzo,
   Overlay, load-profile, TS custom rules and (opt-in) SDK admission. The LSP
   and the CLI both call it; the LSP adds only incremental scheduling.
3. **Rule registry with ownership.** Each code is owned by exactly one check.
   Overlaps are resolved by an explicit alias table (`operation-operationId`
   → canonical `oas-operation-missing-operationId`), so `lint.rules` overrides
   apply to either name and the finding is emitted once.
4. **Suppression**, in two forms:
   - inline: `# suspect-disable-next-line <code>` (YAML) and an
     `x-suspect-disable: [codes]` extension on any object (JSON and YAML);
   - a findings baseline file (`suspect check --write-baseline
     .suspect-baseline.json`), keyed by code + JSON pointer + message hash,
     so CI can gate on *new* findings only.
5. **Command surface:** `suspect check` becomes "run every check" (today it
   is a parse report and exits 0 on files with semantic errors). The current
   parse report moves to `suspect check --parse-only` / `suspect stats`.
   `validate` and `lint` remain as filtered views (`--only validate|lint`).

**Decided (2026-10-29):** the `sdk-*` readiness codes stay a separate
category, shown only when codegen is configured, so a problem such as a
missing `operationId` is reported once by default.

Estimate: 2–3 days for crate + renderers + LSP/CLI switch-over; +1 day for
suppression; +1 day for the rule alias table and dedup tests.

---

## P2. One configuration

### Today

- `suspect-config` declares itself the single schema (precedence
  `flag > env > file > default`) but only `validate`, `lint`, `docs`, `fmt`,
  `codegen` and `ci` read it. `gateway`, `test`, `load`, `fuzz`, `gen`,
  `release`, `project` and the `codegen-*` family are flag-only.
- **Three merges of manifest policy with three orders.** `discover` lets the
  settings file win and merges `lint.rules` per key (`file.rs:38-70`); the
  LSP's `config_files::file_layers` replaces the whole map
  (`config_files.rs:162-166`); `ci.rs:352-417` lets the manifest win.
- The LSP holds two config objects for one diagnostic pass (`EditorConfig`
  and its own `SuspectConfig`) and parses config files twice.
- **Discovery root differs.** The CLI searches upward from the cwd first and
  falls back to the input's directory (`suspect-config/src/lib.rs:276-284`);
  the LSP uses the workspace root.
- About eight separate config formats: `.suspect.yaml`,
  `suspect.project.json`, `release.json`, `gen.toml`, the `codegen-session
  --config` JSON, `codegen --defaults`, `--mapping`, `--target-config`.
- Private env vars outside the precedence model: `SUSPECT_BASE_URL` (LSP run
  lenses, hardcoded `localhost:8080` fallback that silently mirrors the
  gateway default), `SUSPECT_GIT_BASE`, `HOVER_ATTEMPT_MS`,
  `SUSPECT_CREDENTIALS` (read in two places), `SUSPECT_BUILD_PROFILE` (two
  places).

### Proposal

1. **`suspect_config::resolve(root) -> Resolved`** is the only entry point.
   It owns discovery, manifest-policy layering and precedence; CI, the CLI and
   the LSP delete their own merges (`ci.rs` `lint_policy`/`lint_overrides`,
   LSP `read_manifest_policy`/`read_settings_value`).
2. **Manifest/config boundary — decided (2026-10-29): option A.** The crate
   doc says a manifest is "deliberately not configuration", yet manifests
   carry lint policy. Options considered:
   - **(A, recommended)** policy lives only in `.suspect.yaml`; the manifest
     references it (`"config": ".suspect.yaml"`). One-release deprecation
     warning when a manifest carries `lint`.
   - (B) manifest policy is a documented lowest file layer, identical in all
     tools.
3. **Extend the schema** with `gateway` (port, mode, upstream), `test`
   (base_url, credentials, report), `load`, `ref` (`max_docs`,
   `allowed_documents`), `rules` (TS runtime roots, enabled), and `editor`
   (inlay hints, formatting) so the LSP's `SuspectConfig` disappears.
4. **Env var registry.** Every `SUSPECT_*` variable is declared once in
   `suspect-config` with its setting path; `suspect config --explain` prints
   every effective value with its layer (`flag`, `env SUSPECT_X`,
   `.suspect.yaml:12`, `default`). Debug/profiling vars
   (`SUSPECT_PROFILE`, `SUSPECT_GW_PROFILE`, `SUSPECT_RULES_DEBUG`,
   `SUSPECT_CONTRACT_TRACE`) are listed under a `debug` heading.
5. **Discovery from the input**, matching the LSP: the nearest config above
   the first input wins, then the cwd.
6. **One file format family.** Accept YAML everywhere (JSON is valid YAML);
   rename `gen.toml` → `gen.yaml` with a compatibility reader, and fold
   `release.json` into the manifest's `release:` section.
7. **JSON Schema for `.suspect.yaml` and `suspect.project.json`**, generated
   from the Rust types, shipped in the VS Code extension
   (`yamlValidation`/`jsonValidation`) and published for SchemaStore.

Estimate: 1–2 days for resolve + deletion of duplicate merges; +1 day for
schema extension and `--explain`; format consolidation is a separate,
deprecation-gated change.

---

## P3. One command grammar

### Today

- ~50 flat top-level commands with overlapping jobs: `docs` and
  `gen --preset docs-md` both render Markdown; `overlay-dry-run` duplicates
  `overlay explain`; comparison is spread over `diff`, `breaking`,
  `arazzo-diff`, `overlay diff`, `codegen-compare`, `release-plan`, `impact`.
- `--format` shares one `text|json|sarif` enum across commands that support
  different subsets; unsupported values silently degrade to JSON or text.
- Output flags: `-o/--output` (`fmt`, `bundle`, `har`, `overlay apply`),
  `--output` only (`docs`, `upgrade`), `-o/--out` (`gen`, `codegen`,
  `contract`), `--out` only (`stubs`). Six different default output dirs.
- `fmt --json/--yaml` and `contract --json` vs `bundle --format json|yaml`;
  `test`/`load --report text|ndjson` vs `bridge`/`codegen-session` NDJSON
  under `--format json`; `gen --diff` vs `--check` elsewhere; JUnit only on
  `load`.
- Inputs: `why --spec` vs positional elsewhere; `codegen-compare
  --before/--after` vs `breaking OLD NEW`; `test --input` and `--inputs`.

### Proposal

**Command tree** (old names stay as hidden aliases for two releases, with a
one-line deprecation note on stderr):

```text
suspect check        run every check (P1)        [validate, lint as filters]
suspect fmt | bundle | upgrade | stats
suspect overlay      apply | explain | diff
suspect diff         structural | breaking | impact     (--arazzo for workflows)
suspect gen          docs | sdk | stubs | terraform | cli | mcp | template
                     profiles | session | compare
suspect test         run | stateful | fuzz | load | replay
suspect traffic      gateway | drift | har | why | reverse | bridge
suspect release      plan | publish | workflow
suspect project      init | check | build | ci
suspect config | auth | acquire | rules | watch | lsp
```

**Flag conventions** (enforced by a shared `clap` args module + a test that
walks the command tree):

| Concern | Convention |
|---|---|
| Report format | `--format` with a **per-command** `ValueEnum` (findings: `text,json,sarif,junit`; reports: `text,json`; streams: `text,ndjson`) so clap rejects unsupported values |
| Document format | `--to json\|yaml` (default: input extension) |
| Single output file | `-o, --output FILE` (stdout when absent) |
| Output directory | `-o, --out DIR` |
| Read-only drift check | `--check` |
| Report files | `--report-file FILE` (+ `--junit FILE`) on every test command |
| Spec input | first positional; comparisons are `OLD NEW` positionals |

**Exit codes** as a typed enum returned by every command (no
`process::exit`): `0` clean, `1` findings/failures, `2` usage or environment
error, `130` interrupted. A test asserts every command's documented codes.

**Generated CLI reference** (`cargo xtask cli-docs` via `clap_mangen` +
Markdown) checked in CI, replacing hand-maintained README command lists
(this exists now as `docs/CLI.md`, rendered from clap by
`crates/suspect-cli/tests/cli_reference.rs`; regrouping the tree regenerates it).

Estimate: ~2 days for the tree + aliases, ~1 day for the flag module and
per-command format enums, ~0.5 day for the reference generator.

---

## P4. One editor story

### Today (after the fixes below)

- One VS Code extension (`editors/vscode`), Zed, and smoke configs for Neovim.
- Settings names differ from config keys (`suspect.lint.minSeverity` vs
  `lint.min_severity`), and the editor-only settings (`testBaseUrl`,
  `gatewayPort`, `ci.baseline`, `sdk.*`) aren't in the config schema.
- The extension registers some commands itself and the server advertises
  overlapping IDs; the overlap is now filtered client-side, but ownership is
  implicit.
- No binary acquisition: every client fails when `suspect` is not installed.
- Zed attaches to every YAML/JSON buffer with no OpenAPI detection.

### Proposal

1. **Settings = config.** Every `suspect.*` editor setting is generated from
   the config schema (P2.7), with identical keys (`suspect.lint.minSeverity`
   ⇄ `lint.min_severity`). Editor settings are the highest file layer; a
   status-bar item shows "config: .suspect.yaml" and opens `suspect config
   --explain` output.
2. **Command ownership.** Server-executed commands keep `suspect.*`;
   client-only UI commands move to `suspect.ui.*`. Custom requests are
   advertised in `initializeResult.capabilities.experimental.suspect` with
   versions so clients feature-detect instead of guessing.
3. **Binary acquisition.** Clients resolve, in order: explicit setting →
   `PATH` → a managed download from the release (P5) matching the extension
   version, with checksum verification and a "Locate binary…" action.
4. **Version handshake.** Clients compare `serverInfo.version` with their own
   and warn on major/minor mismatch (the VS Code part is implemented).
5. **Document detection.** The server answers a cheap
   `suspect/documentKind` (openapi/arazzo/overlay/load/none) used by Zed and
   VS Code to avoid attaching to unrelated YAML/JSON, plus file-glob
   associations for `*.openapi.*`, `*.arazzo.*`, `*.overlay.*`, `*.load.*`.
6. **Publish** the VS Code extension (Marketplace + Open VSX) and the Zed
   extension from the release workflow.

---

## P5. One install and release path

### Today

No release workflow, no binaries, no packaged extensions; no crate sets
`publish = false`; no `CHANGELOG`. (`LICENSE-MIT`, `LICENSE-APACHE` and
`CONTRIBUTING.md` were added alongside this proposal.)

### Proposal

1. **cargo-dist** (or an equivalent tag-triggered workflow) producing
   binaries for macOS (arm64/x64), Linux (x64/arm64, musl), Windows, plus a
   shell/PowerShell installer and a Homebrew tap.
2. The same workflow packages the `.vsix` and the Zed extension (P4.6).
3. `publish = false` on internal crates; decide which crates are public API
   (likely `suspect-low`, `suspect-ref`, `suspect-oas`, `suspect-lint`).
4. Add `CHANGELOG.md` (generated from conventional commits) and point
   `CONTRIBUTING.md` at `cargo xtask ci` once it exists (P7).

**Decided:** the `LICENSE-MIT` copyright line stays "Luke Parke and suspect
contributors". **Needs a decision:** which registries
(crates.io, Homebrew, npm wrapper) are in scope.

---

## P6. One doc entry point

### Today

Two READMEs with overlapping content; `docs/` holds 120 flat files with no
index, of which ~27 are internal plans, exit reports, status files and dated
reviews (`SDK-FULL-EXIT.md`, `SDK-FULL-PLAN-STATUS.md`,
`SDK-DX-REVIEW-20260911.md`, `*-FOLD-PLAN.md`, …).

### Proposal

```text
README.md                  install, 5-minute tour, command map, links
docs/README.md             index
docs/guide/                getting-started, cli (generated), config, editors,
                           linting & custom rules, testing & load, gateway &
                           cassettes, docs generation, sdk generation
docs/reference/            sdk-capabilities, openapi-matrix, credential-env,
                           diagnostics (generated code list), config schema
docs/sdk/<language>/       per-language notes (today's SDK-<LANG>-*.md)
docs/internal/             plans, exits, status, research, dated reviews
```

Done so far: `README.platform.md` is merged into `README.md` (performance
table in `BENCHMARKS.md`), `docs/README.md` indexes every file, and
`docs/CLI.md` is generated from clap and checked by a test. Remaining: the
directory moves above, and a link checker (lychee) in CI.

---

## P7. One developer entry point

- `cargo xtask ci` runs exactly what CI runs: fmt, clippy, tests, docs,
  `cargo deny`, editor build + tests, `bun test`, link check. CI calls the
  same xtask so local and CI can't drift.
- Rename milestone tasks (`sdk-full`, `sdk-m3-m6`) to what they verify
  (`verify-sdk`, `verify-sdk-native`).
- **One name for the OpenRouter input.** Native-toolchain fallbacks now all
  go through `suspect_codegen::toolchain` (done; it gained `python`,
  `terraform` and nested mise layouts). The spec checkout is still named two
  ways — `OPENROUTER_WEB_ROOT` (a checkout, 19 test files) and
  `SUSPECT_OPENROUTER_OPENAPI` (a file, the Java gates and xtask). Pick one.
- **Skip vs fail for missing inputs.** Opt-in gates fail when an input such
  as the OpenRouter checkout is unset ("missing inputs fail the gate; they
  never skip it"), while toolchain gates skip. Decide one rule and record it
  in `CONTRIBUTING.md`.
- **Historical records keep machine paths.** Exit/evidence documents
  (`SDK-*-EXIT.md`, `SDK-SESSION-HANDOFF.md`, dated reviews) and
  `tools/sdk-session-perf/runners/*-inventory-*.json` record the machine they
  ran on and several link to root files that no longer exist
  (`DEMO-README.md`, `LIVE-DEMO-README.md`). Moving them to `docs/internal/`
  (P6) and excluding that directory from the link checker keeps them honest
  without breaking CI.

## P8. Crate consolidation (optional)

Small crates that are single-consumer implementation details:
`suspect-watch` (265 lines), `suspect-faker` (341), `suspect-reverse` (361),
`suspect-upgrade` (675). Merging them into their consumers would shorten
build graphs and the crate table without changing behaviour. Low priority;
only worth doing alongside P5.3 (deciding the public crate set).

---

## Suggested order

| Phase | Work | Estimate |
|---|---|---|
| 1 | P2.1–P2.5 config resolve + explain | 2–3 days |
| 2 | P1 finding model + one check pass + aliases | 3–4 days |
| 3 | P3 command tree, flag module, exit enum, generated reference | 3–4 days |
| 4 | P4 settings generation, command namespaces, binary acquisition | 2–3 days |
| 5 | P5 release + P6 docs + P7 xtask | 2–3 days |

Phases 1 and 2 come first because P3 and P4 render and configure what they
define.

## Already fixed

Defects fixed directly, alongside this proposal:

- `suspect drift --format sarif` panicked (`unimplemented!()`).
- `suspect rules` always exited 0; Bun-unavailable exited 1 instead of 2;
  `--format sarif` printed JSON.
- `drift --exit-on-gap` exited 2 for findings instead of 1.
- `arazzo-diff` JSON/SARIF bypassed the shared finding list.
- Severity names were parsed three different ways; unknown values silently
  became `hint`.
  One parser now lives in `suspect-config` (`severity.rs`); an unknown
  `lint.min_severity` is a usage error (exit 2).
- Boolean config layers merged with `|=`, so a higher layer could not turn a
  setting off; and an unset `--strict-format` flag always beat
  `validate.strict_format: true` in config.
- `upgrade` always emitted JSON and ignored `--format` (now `json|yaml`,
  defaulting to the input/output extension).
- Inaccurate help text (`--format` described as global, `bridge`/`reverse`,
  `overlay`); `bridge`/`reverse --format sarif` silently printed text and is
  now rejected.
- `lint --ruleset` skipped the environment layer (`SUSPECT_LINT_RULESET`).
- VS Code: duplicate command registration with server-advertised commands;
  settings changes never reached the server; empty `minSeverity` overrode
  `.suspect.yaml`; `suspect.server.path` alias, missing-binary action and
  version check.
- LSP: config files on disk were never reloaded; `didChangeConfiguration`
  with a `null` payload (VS Code's default) applied nothing; blank client
  strings overrode `.suspect.yaml`; relative client rulesets resolved against
  the server cwd; code actions used a different diagnostic set than published
  diagnostics; initialization options were consumed on first read, so later
  reloads and `runWorkflow` lost them.
- LSP, found by running the extension in a real VS Code: the server both
  pushed and served pulled diagnostics, so VS Code listed every finding
  twice; `workspace/diagnostic` ignored the severity floor; pull result ids
  hashed the unfiltered findings, so a floor change answered "unchanged";
  and a report computed under the old policy could be cached after a config
  change cleared the caches. `npm run test:host:smoke` now checks this in a
  real VS Code window.
- Zed: binary lookup inside the WASI sandbox, `binary.path` ignored, README
  documented methods the server does not implement; smoke script hard-coded
  a local path and the wrong server id.
- CI: the VS Code job targeted a legacy extension and could not fail;
  `editors/vscode`, `rules-runtime` and `cargo deny` were untested; fmt,
  clippy and tests ran twice per push.
- Repo: committed `node_modules`/`dist`, a stray gateway log, the superseded
  `.vscode-suspect` extension, duplicate workspace members, hard-coded home
  paths in examples and the toolchain manifest, contradictory README backend
  counts.
