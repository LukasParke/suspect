# Toolchain unification proposals

Status: **implemented** (2026-10-31) on branch `t3/unify-toolchain-review`,
except the steps that by design belong to later releases or need accounts
outside this repository (see [Implementation status](#implementation-status)).
Each proposal below has its own document with the evidence, the design, the
PR-sized implementation plan (each step marked **Done.** with its
deviations), tests, alternatives, risks and open questions. Concrete defects
found while writing them were fixed directly and are listed in
[Already fixed](#already-fixed).

## Implementation status

| | Done | Not done, by design |
|---|---|---|
| [P1](P1-finding-model.md) | steps 1–9 | — |
| [P2](P2-configuration.md) | steps 1–8 | N+1: manifest `lint`/`validate`/`editor` policy is ignored instead of warned about |
| [P3](P3-command-grammar.md) | steps 1–5 | step 6 (N+1: editors switch to the new names, `--out` hidden) and step 7 (N+2: legacy names removed) |
| [P4](P4-editors.md) | steps 0–9 | publisher and registry account setup (P5 step 6) |
| [P5](P5-release.md) | steps 1–5, 7, 8; step 6 docs | step 6: create the Homebrew tap, Marketplace / Open VSX publishers, secrets, the first Zed submission, and tag `v0.1.0` (CONTRIBUTING "Releasing" lists them) |
| [P6](P6-docs.md) | steps 1–7 | — |
| [P7](P7-developer-workflow.md) | steps 1–8 | — |
| [P8](P8-crate-consolidation.md) | steps 1–5 | — |

Open questions were settled with their recommended answers; deviations from
a design are written next to each step's **Done.** marker.

## Goal

A user should meet one tool, not twenty-eight crates:

1. **One finding model.** A problem in a spec is reported once, with one code
   and one severity, in `suspect check`, `suspect ci`, SARIF and the editor.
2. **One configuration.** Every tool reads the same settings with the same
   precedence, from the same root, and `suspect config --explain` shows where
   each value came from.
3. **One command grammar.** Flags, formats and exit codes mean the same thing
   in every command.
4. **One editor story.** Setting names match the config file, the editor
   finds the matching binary itself, and unrelated YAML/JSON is left alone.
5. **One install and release path, one doc entry point, one developer
   entry point.**

The parse / `$ref` pipeline already meets this bar: every tool loads through
`suspect_ref::WorkspaceBuilder` + `LowDoc::parse`.

## Proposals

| | Proposal | What changes for a user | Estimate |
|---|---|---|---|
| [P1](P1-finding-model.md) | One finding model and one check pass | `suspect check` runs every check the editor runs and exits 1 on errors; one code per problem; inline suppression and a findings baseline | 9 days, 9 PRs |
| [P2](P2-configuration.md) | One configuration | `lint`, `ci` and the editor apply the same settings to the same file; `suspect config --explain`; JSON Schema for both config files | 8.5 days, 8 PRs |
| [P3](P3-command-grammar.md) | One command grammar | 49 top-level commands become 23 groups; per-command `--format` values; one `-o/--output`, `--to`, `--check`; typed exit codes; old names work for two releases | 5 days (+1 over two later releases) |
| [P4](P4-editors.md) | One editor story | settings generated from the config schema; binary bundled (VS Code) or downloaded and verified (Zed); no hover/format on non-spec files; server-side version check | 7–8 days, 10 PRs |
| [P5](P5-release.md) | One install and release path | `curl \| sh`, PowerShell and Homebrew installs; seven targets per `v*` tag; extensions published from the same workflow | 5–6 days, 8 PRs |
| [P6](P6-docs.md) | One doc entry point | docs split into `guide/`, `reference/`, `sdk/<lang>/`, `design/`, `internal/`; offline link checker in CI; generated reference pages | 2.5 days for the move, ~3 more for guide and generated pages |
| [P7](P7-developer-workflow.md) | One developer entry point | `cargo xtask ci [job]` runs exactly what CI runs; one OpenRouter input name; one skip-vs-fail rule | 4.5 days, 8 PRs |
| [P8](P8-crate-consolidation.md) | Crate consolidation | none (28 crates become 23) | 1 day, 5 PRs |

Total: about 46 days of work. Much of it can run in parallel (see
[Order](#order)).

## Decisions

Recorded:

- **P1** (2026-10-29): the `sdk-*` readiness codes stay a separate category,
  on only when codegen is configured (`--sdk` / `--no-sdk` override).
- **P2** (2026-10-29): option A. Lint policy lives only in `.suspect.yaml`.
  A manifest references it (`"config": ".suspect.yaml"`), and a manifest
  that still carries `lint` gets a deprecation warning for one release.
- **P5**: the `LICENSE-MIT` copyright line stays "Luke Parke and suspect
  contributors".

Still open (the recommended answer is in each document):

- **P5:** which registries ship the first release. Recommended: GitHub
  Releases, a Homebrew tap, VS Code Marketplace, Open VSX and the Zed
  registry. Not crates.io or npm yet.
- **P1:** the canonical default for a missing `operationId`. Recommended:
  warning.
- **P4:** whether client-only commands move to `suspect.ui.*`. Recommended:
  no rename (see below).
- **P7:** delete `tools/sdk-demo*`, 51 files that cannot run. Recommended:
  delete in a separate PR.

## Contracts between proposals

These are the points where one proposal relies on another. The documents
agree on them; change them in both places or not at all.

| Contract | Owner | Used by |
|---|---|---|
| `suspect-diag` is a leaf crate holding `Finding`, `Severity`, the one severity parser, and the text/JSON/SARIF/JUnit renderers; `suspect-config` re-exports the parser | P1 | P2, P3 (format enums render through it), P5 (public crate after P1) |
| One setting table in `suspect-config` drives parsing, env vars, `--explain`, LSP hover, and the checked-in JSON Schema (`editors/vscode/schemas/`, `SUSPECT_BLESS` pattern) | P2 | P4 (generated VS Code settings), P6 (generated config and env reference pages) |
| Precedence: default < manifest (deprecated) < `.suspect.yaml` < env < editor setting = flag. `lint.rules.<id>` merges per key in every tool | P2 | P1 (policy input), P4 |
| Exit codes: `0` clean, `1` findings or failed checks, `2` usage or environment error, `130` interrupted; long-running servers exit `0` on Ctrl-C | P3 | P1, P7, `CONTRIBUTING.md` |
| Release assets: `suspect-cli-{TRIPLE}.tar.gz` (`.zip` on Windows) plus `.sha256` per archive; Linux editors use the static musl build | P5 | P4 (Zed download, VS Code platform VSIX) |
| `cargo xtask ci <job>`, one subcommand per CI job, checked against the workflows by a test | P7 | P6 (`links` job), P5 (`check-versions`, `package-foundation`) |

## What the full versions changed

The short versions were written from a review. Checking them against the
code moved several of them:

- **P1.** Besides the `operationId` triple, five more
  validate/lint pairs fire at the same spot. Duplicates are removed after
  the checks run, not by switching rules off, because switching off would
  lose coverage on Swagger 2.0. The baseline key drops the message hash
  (messages carry how-to-fix text that changes), and inline suppression
  applies to the next node, not the next line.
- **P2.** No `schemars`: the LSP already has the right table in
  `config_schema.rs`, so it moves into `suspect-config` and grows columns.
  The LSP resolves config per document, so monorepos agree with the CLI.
  Option A also needs `project init` to stop writing `lint` into manifests,
  plus `extends:` and `suspect config migrate`.
- **P3.** The tree now places all 49 commands, including the four the short
  version left out (`admission`, `bench`, `evidence`, `contract`).
  `overlay-dry-run` is not a duplicate of `overlay explain` (it accepts
  targets `apply` rejects), and `docs` is not a duplicate of
  `gen --preset docs-md`. One `-o, --output` replaces the `--output`/`--out`
  split. Old flat names are rewritten before clap parses, because a clap
  alias cannot turn a command into a group.
- **P4.** No `suspect.ui.*` rename: the overlap is 11 palette wrappers that
  reuse server ids, so clients declare the ids they own in
  `experimental.suspect` and the server leaves them out. Document detection
  moves to the server, which fixes every client at once. VS Code ships one
  VSIX per platform with the binary inside; Zed downloads and checks a
  `.tar.gz`, because its WebAssembly API cannot unpack `.tar.xz`.
- **P5.** The likely public crate set (low, ref, oas, lint) cannot be
  published: `suspect-lint` depends on an internal crate. Seven parse-layer
  crates can (`source`, `syntax`, `low`, `ref`, `jsonpath`, `oas`,
  `overlay`); the other 21 get `publish = false`. `cargo install suspect-cli`
  from crates.io is not possible while `suspect-rules` embeds files from
  outside its crate.
- **P6.** A `docs/design/` directory holds the 19 architecture notes that
  code and tests point at, so they stay link-checked. The move rewrites 671
  links; a dry run on a copy found the same 4 broken links before and after.
- **P7.** The OpenRouter input has five names, not two; keep
  `OPENROUTER_WEB_ROOT` (188 of 224 uses). Toolchain gates are mixed today,
  not "skip": the rule becomes default-suite tests may skip with a standard
  line, `#[ignore]` gates always fail on a missing tool.
- **P8.** Five crates merge, not four: `suspect-rex` is used only by
  `suspect-test` (`suspect-lsp` lists it but never uses it).
  `suspect-faker` has two consumers and moves into `suspect-ir`.

## Order

| Phase | Work | Calendar | Why here |
|---|---|---|---|
| 0 | P8 (all); P7 steps 6–7 (skip/fail rule, Bun in CI, `native-evidence.yml`); P6 steps 1–2 (link checker, path guards) | ~3 days | no dependencies; makes CI tell the truth before the large changes land |
| 1 | P2 steps 1–7 | ~7 days | P1, P3 and P4 read configuration through `resolve` |
| 2 | P1; in parallel P4 steps 1–3 and P7 steps 1–5 | ~9 days | P3 renders P1 findings; P4 steps 1–3 need nothing else |
| 3 | P3 steps 1–5; in parallel P5 steps 1–6 and P6 steps 3–6 | ~6 days | P5's first tag should carry the new command tree |
| 4 | P4 steps 4–9; P6 step 7; P2 step 8 | ~5 days | needs the P2 schema and the P5 release assets |
| later | P3 steps 6–7 and P2's manifest-policy removal, one and two releases after phase 3 | ~1 day | deprecation windows |

About 30 working days of calendar time with two people working in
parallel. One person working alone needs the full ~46 days.

## Already fixed

Found while writing the full proposals and fixed directly (2026-10-30):

- `suspect ci`: one unrecognized severity in `lint.rules` silently dropped
  every override; the lint stage now fails and names the value. `ci` and
  `project build` now honour `validate.strict_format` (and
  `SUSPECT_STRICT_FORMAT`) as `suspect validate` does.
- `suspect rules check` reported a TS finding with severity `off` as a
  warning; `off` now drops it.
- Editor: an unknown `lint.min_severity`, `lint.rules` or `lint.design`
  severity, or a config file that failed to load, was applied silently; the
  server now shows a warning naming the key. Floor names go through the
  shared parser.
- `suspect config [INPUT]` help described discovery backwards.
- VS Code Live SDK Preview sent the whole `sdk.importNames` map as
  `importName`, so the first render failed with `-32602`.
- CI: the `test` job had no Bun, so the `suspect-rules` worker tests passed
  without running; `native-evidence.yml` named test targets that do not
  exist and hid the error with `|| true`, so 8 of 10 backends ran nothing.
  It now selects `<prefix>*` targets and fails on a real failure.
- Docs: 4 dead links removed; `examples/sdk-demo.json`, which the README's
  SDK demo needs, is restored (five operations, five backends; `--check`
  reports current).

Defects fixed directly, alongside the first version of this proposal:

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
