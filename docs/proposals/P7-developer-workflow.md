# P7. One developer entry point

Status: proposal. Depends on: none for `cargo xtask ci`. P6 for moving the
historical records. P2.4 only for listing `SUSPECT_REQUIRE_TOOLS` in the env
registry. Decided: native-toolchain fallbacks all go through
`suspect_codegen::toolchain` (done: `python`, `terraform` and nested mise
layouts were added). The source document records this under P7.


Fixed since this was written (2026-10-30, see the
[index](README.md#already-fixed)): Bun missing from the `ci.yml` `test` job and `native-evidence.yml` naming test targets that do not exist. The Problem section keeps
them as evidence.
## Summary

A contributor runs `cargo xtask ci` and gets the same commands, flags and
environment that CI runs, one job at a time, with a pass/fail/skip summary.
Each CI job becomes `cargo xtask ci <job>`, so the workflow files hold only
toolchain setup. The two milestone tasks get names that say what they verify.
The OpenRouter input has one name, `OPENROUTER_WEB_ROOT`, read by one helper.
One recorded rule decides when a test may skip.

## Problem

1. **`cargo xtask` does not exist.** There is no `.cargo/config.toml` alias:

   ```text
   $ cargo xtask --help
   error: no such command: `xtask`
   ```

   The only tasks are `fetch-corpus`, `gen-fixtures`, `sdk-full` and
   `sdk-m3-m6` (`xtask/src/main.rs:31-35`), invoked as
   `cargo run -p xtask -- <task>`. `cargo run -p xtask -- ci` prints
   `unknown task: "ci"` and exits 2.
2. **CONTRIBUTING and CI already differ.**
   - `CONTRIBUTING.md` runs `cargo clippy ... -- -D warnings` and a plain
     `cargo test`. `ci.yml:9` sets `RUSTFLAGS: "-D warnings"` for every job,
     so CI also fails test, bench and doc builds on rustc warnings.
     `platform.yml` sets no `RUSTFLAGS`.
   - CONTRIBUTING omits `bench-smoke` (`ci.yml:48-52`), `corpus-validation`
     as a gate, `platform-smoke` (`platform.yml:14-182`) and the CLI-backed
     `npm run test:generation` with `SUSPECT_TEST_BINARY`
     (`platform.yml:222-225`).
   - The Zed build differs. CI runs
     `cargo build --locked --manifest-path editors/zed/Cargo.toml --target wasm32-wasip1`
     (`platform.yml:246`). CONTRIBUTING runs `cd editors/zed && cargo build
     --target wasm32-wasip1`, without `--locked`.
   - `ci.yml` jobs use the runner's preinstalled Rust (no toolchain step).
     `platform.yml` uses `dtolnay/rust-toolchain@stable`. No
     `rust-toolchain.toml` exists. Locally: `rustc 1.99.1`.
3. **The milestone task names describe history, not what they check.**
   `sdk-full` "Verify the maintained twelve-language full SDK plan"
   (`main.rs:13`). `sdk-m3-m6` is the five-backend M3/M6 scope, whose
   inventory is frozen. `sdk_full.rs:3-4` says "The historical M3/M6
   inventory is not extended". References: `sdk-full` 115 times in 20
   files, `sdk-m3-m6` 20 times in 11 files. Outside `xtask/`, every
   reference is in `docs/` (historical records) and
   `tools/sdk-native-costs/*`. The string `sdk-full` is also the package
   name of the generated fixtures (`@suspect-fixtures/sdk-full`,
   `sdk_full`, `libsdk_full.*` in `tools/sdk-native-costs/`), so only the
   task name may change.
4. **The OpenRouter input has five names, not two.** `git grep -o` counts:

   | Variable | Means | Occurrences | Rust files (excl. xtask) | xtask |
   |---|---|---|---|---|
   | `OPENROUTER_WEB_ROOT` | checkout root | 188 | 58 | 2 |
   | `SUSPECT_OPENROUTER_YAML` | `<root>/projects/docs/openapi/openapi.yaml` | 16 | 10 | 1 |
   | `SUSPECT_OPENROUTER_OPENAPI` | same file | 8 | 2 | 1 |
   | `SUSPECT_OPENROUTER_SPEC` | same file (xtask), but `suspect-gen/tests/context_isolation.rs:238` says `openrouter-openapi.json` | 7 | 2 | 1 |
   | `OPENROUTER_PUBLIC_SCHEMA` | same file | 5 | 1 | 1 |

   The xtask sets all five to the same checkout. Examples:
   `sdk_m3_m6.rs:1156-1164` sets `OPENROUTER_WEB_ROOT={out}/inputs`,
   `SUSPECT_OPENROUTER_YAML` and `SUSPECT_OPENROUTER_SPEC` to
   `{out}/inputs/projects/docs/openapi/openapi.yaml`. `sdk_full.rs:1181`
   sets `SUSPECT_OPENROUTER_OPENAPI`, and `sdk_full.rs:2499` sets
   `OPENROUTER_PUBLIC_SCHEMA`. 12 test files also fall back silently to a
   sibling checkout,
   `CARGO_MANIFEST_DIR/../../../openrouter-web/projects/docs/openapi/openapi.yaml`.
   An example is `rust_codecs.rs:163-168`. Each file re-implements the
   lookup with its own `expect` message.
5. **Skip vs fail is decided per file, four ways.** A scan of
   `crates/*/tests` found 71 early-return skips inside `#[ignore]` tests,
   28 in default-suite tests and 5 in helpers. The scan matched a
   skip/degrade `eprintln!` followed by `return`. Concretely:
   - An `#[ignore]` gate that passes without running when its tool is
     missing: `csharp_oauth.rs:970-978` (`#[ignore = "requires the .NET SDK"]`,
     then `eprintln!("csharp_oauth: skipping ..."); return;`). Same for
     `cpp_incoming.rs:576-581`.
   - An `#[ignore]` gate that fails when its tool is missing:
     `csharp_protocol.rs:282`
     (`toolchain::gate("dotnet").unwrap_or_else(|skip| panic!("{skip}"))`),
     and all `java_*` gates.
   - An `#[ignore]` gate that fails when its input is missing:
     `go_json.rs:445` (`#[ignore = "... missing inputs fail"]`) and
     `numeric.rs:464` (`"absence is not acceptance"`).
   - Default-suite tests that skip. Some use the manifest
     (`sdk_compat_profiles.rs:342-347`, `sdk_model_names.rs:64-69`).
     Others hand-roll `tool_available` (`sdk_compat_profiles.rs:81`,
     `typescript_incoming.rs:227`, `typescript_oauth.rs:407`,
     `typescript_pagination.rs:163`, `typescript_stream_typed.rs:262`)
     or `bun_available` (`suspect-rules/tests/worker_integration.rs:106`).
   - `toolchain::gate` documents only the skip form
     (`toolchain.rs:345-358`: `Err(skip) => eprintln!("skipping: {skip}")`).
6. **CI hides skipped and missing gates.**
   - The `test` job in `ci.yml:43-47` does not install Bun, so every
     `suspect-rules` worker integration test returns early on CI.
   - `native-evidence.yml:135-145` passes `--test <name>_integration`,
     `_attribution`, `_elimination` and so on for every backend, then pipes
     to `tail` and appends `|| true`. Cargo rejects a missing target before
     running anything:

     ```text
     $ cargo test -p suspect-codegen --test rust_integration --test rust_protocol --no-run
     error: no test target named `rust_integration` in `suspect-codegen` package
     ```

     Checking each matrix name against `crates/suspect-codegen/tests/`,
     only `php` and `dart` have all nine targets. `node`, `dotnet` and `jdk`
     have none, because the files are `typescript_*`, `csharp_*` and
     `java_*`. So eight of ten evidence jobs run zero tests and report
     success. The matrix already declares a `test_filter` (`rust_`,
     `typescript_`, `csharp_`, …) that the step never uses.
   - `corpus_suite.rs:183-196` returns early when the corpus is incomplete,
     inside the job whose only purpose is that corpus (`ci.yml:53-58`).
7. **Historical records keep machine paths and dead links.**
   - `/Users/` appears in 9 tracked files: `docs/SDK-M3-M6-EXIT.md` (6),
     `docs/SDK-FULL-EXIT.md` (5), `docs/SDK-SESSION-HANDOFF.md` (2),
     `docs/SDK-GOLDEN-DEFAULTS-PLAN.md`, `docs/SDK-FULL-PLAN-STATUS.md`,
     `docs/SDK-DX-REVIEW-20260911.md`, `docs/SDK-COMPAT-DIFF-TYPESCRIPT.md`,
     `docs/SDK-COMPAT-DIFF-PYTHON.md` (1 each), and
     `tools/sdk-session-perf/runners/luke-mac-inventory-20260909.json` (4).
   - Links to root files that do not exist (lychee, offline): 18 to
     `DEMO-README.md`, 13 to `LIVE-DEMO-README.md`, 7 to
     `LIVE-ENV-DEMO-README.md`, 2 to `docs/OPENROUTER-ACCEPTANCE.md`. They
     are in `SDK-DX-REVIEW-20260911.md`, `SDK-CURRENT-SDK-COMPARISON-20260911.md`,
     `SDK-FULL-PLAN-STATUS.md`, `SDK-FULL-PLAN-WORK.md`, and outside the
     archive in `SDK-CREDENTIAL-ENV.md:175`, `SDK-CAPABILITIES.md:118`,
     `SDK-NATIVE-DX.md:16` and `tools/sdk-demo-web/README.md:26`.
   - It is not only docs. The scripts in `tools/sdk-demo/` (9 files),
     `tools/sdk-demo-live/` (22) and `tools/sdk-demo-web/` (20) read those
     missing files. Examples: `tools/sdk-demo/check_readme.py:42`
     (`REPO / "DEMO-README.md"`), `tools/sdk-demo/demo.py:17`
     (`examples/sdk-demo-all.json`) and `tools/sdk-demo-web/catalog.py:9`
     (`LIVE-ENV-DEMO-README.md`). They cannot run in this repository. No
     workflow or test references them.

## Goals and non-goals

Goals:

- `cargo xtask ci` runs every blocking CI job from `ci.yml` and
  `platform.yml`, plus the P6 `links` job, with the same commands and
  environment.
- Each CI job's `run:` is exactly `cargo xtask ci <job>`. A test fails when a
  workflow and the xtask job table diverge.
- One name and one helper for the OpenRouter input.
- One skip/fail rule, enforced by helpers, recorded in `CONTRIBUTING.md`.
- CI never reports a skipped gate as passed.

Non-goals:

- Porting `native-evidence.yml`, `sdk-session-performance.yml`,
  `application-targets.yml` or `docs.yml` to xtask. They are scheduled or
  input-gated. This proposal fixes only their masking bugs.
- Real-host editor smoke (`npm run test:host:smoke`,
  `test:generation:host`). They need a desktop session and stay manual.
- Adding clap to xtask. The hand parser in `main.rs` is enough for
  positional job names and four flags.
- Rewriting the contents of historical records.

## Design

### `.cargo/config.toml` (new)

```toml
[alias]
xtask = "run --locked --quiet -p xtask --"
```

`--locked` matches the `sdk-full` usage string (`sdk_full.rs:80`).

### Job table (new `xtask/src/ci.rs`)

One row per CI job. The job name matches the workflow job id, so a CI
failure names the local command to run.

| Job (workflow id) | Workflow | Commands, in order | Env | Tools probed |
|---|---|---|---|---|
| `fmt` | ci.yml | `cargo fmt --all --check` | | `rustfmt` |
| `clippy` | ci.yml | `cargo clippy --workspace --all-targets -- -D warnings` | | `cargo-clippy` |
| `docs` | ci.yml | `cargo doc --workspace --no-deps` | `RUSTDOCFLAGS=-D warnings` | |
| `cargo-deny` | ci.yml | `cargo deny --manifest-path ./Cargo.toml --workspace check` | | `cargo-deny` |
| `test` | ci.yml | `cargo test --workspace` | `SUSPECT_REQUIRE_TOOLS=bun,node,python` (in CI only; see rule below) | |
| `bench-smoke` | ci.yml | `cargo bench --workspace -- --test` | | |
| `corpus-validation` | ci.yml | `fetch_corpus(&[])` (in-process), `cargo test -p suspect-cli --test corpus_suite` | `SUSPECT_REQUIRE_CORPUS=1` | network |
| `links` | ci.yml (P6) | `lychee --config lychee.toml <git ls-files '*.md'>` | | `lychee` |
| `platform-smoke` | platform.yml | `cargo build --release -p suspect-cli`, then the gateway/Arazzo/cassette sequence in Rust (below) | | |
| `vscode-extension` | platform.yml | `cargo build -p suspect-cli`; in `editors/vscode`: `npm ci`, `npm run compile`, `npm run test:headless`, `npm run test:generation` | `SUSPECT_TEST_BINARY=<root>/target/debug/suspect` for the last | `node >= 22`, `npm` |
| `zed-extension` | platform.yml | `cargo build --locked --manifest-path editors/zed/Cargo.toml --target wasm32-wasip1` | | `rustup target wasm32-wasip1` |
| `rules-runtime` | platform.yml | in `rules-runtime`: `bun install`, `bun test` | | `bun` |

There is one deliberate change from today. `RUSTFLAGS=-D warnings`
(`ci.yml:9`) is dropped and `clippy` gets `-- -D warnings` instead, which is
what `CONTRIBUTING.md` already documents. Reason: `RUSTFLAGS` is part of
cargo's fingerprint. Setting it in a local xtask run would rebuild the whole
workspace every time a contributor switches between `cargo test` and
`cargo xtask ci test`. Clippy with `--all-targets` already runs rustc's
warnings over lib, bin, test, bench and example targets, so the gate is kept.

`platform-smoke` in Rust: the YAML fixtures become `const &str` in `ci.rs`.
The port is taken from `TcpListener::bind("127.0.0.1:0")` locally and stays
8091 in CI (`--port` flag). Readiness polls with `ureq`, which is already an
xtask dependency (`xtask/Cargo.toml`). The cassette is written with
`serde_json` + `sha2`, also existing dependencies, so the job no longer
needs `python3` or `curl`. The two `grep -q '"event":"run_done","passed":1,"failed":0'`
checks become a parse of the last NDJSON line.

### CLI surface

```text
cargo xtask ci [JOB...] [--list] [--skip-missing] [--fail-fast] [--ci]
  JOB             one or more job names from the table; none = all, in table order
  --list          print the job table (name, workflow, commands, tools) and exit 0
  --skip-missing  report a job whose tool is missing as SKIP instead of failing
  --fail-fast     stop at the first failing job (default: run all, then summarize)
  --ci            set by the workflows: forbids --skip-missing, uses the CI port
```

Output ends with a summary, and the exit code follows `CONTRIBUTING.md`
(0 clean, 1 a job failed, 2 environment error):

```text
ci: fmt ............ ok      (2.1s)
ci: clippy ......... ok      (48.0s)
ci: rules-runtime .. SKIP    bun not found; install from https://bun.sh (CI: oven-sh/setup-bun@v2)
ci: 11 ok, 0 failed, 1 skipped. A run with skips is not what CI runs.
```

Missing tools, locally:

- By default, a missing tool fails that job with exit 2 and the install
  hint, and the remaining jobs still run. This follows the skip rule below:
  a requested gate never passes without running.
- With `--skip-missing`, the job is reported `SKIP`. If nothing failed, the
  exit code is 0, and the summary says the run differs from CI.
- Probes are `rustup component list --installed` (rustfmt, clippy),
  `rustup target list --installed` (wasm32-wasip1), and `<tool> --version`
  for `cargo-deny`, `lychee`, `node`, `npm` and `bun`. For node, the major
  version is checked against 22. xtask has no dependency on
  `suspect-codegen` (`sdk_full.rs:3`), so `ci.rs` has its own 7-row tool
  table instead of reusing `suspect_codegen::toolchain::TOOLS`.

```rust
// new: xtask/src/ci.rs (shape)
pub(crate) struct Job {
    pub name: &'static str,                    // == workflow job id
    pub workflow: &'static str,                // ".github/workflows/ci.yml"
    pub tools: &'static [ToolProbe],
    pub run: fn(&Ctx) -> anyhow::Result<()>,   // runs the commands in order
}
pub(crate) struct ToolProbe { pub name: &'static str, pub argv: &'static [&'static str], pub install: &'static str }
pub(crate) enum Outcome { Ok(Duration), Failed(anyhow::Error), Skipped(String) }
pub(crate) const JOBS: &[Job] = &[/* rows of the table above */];
pub fn run(args: &[String]) -> anyhow::Result<()> { /* parse, probe, run, summarize, exit */ }
```

`main.rs` gains `"ci" => ci::run(rest)` and a `USAGE` line.

### CI calls the xtask, one job per subcommand

Each job keeps its setup steps and replaces its `run:` lines with one line,
so CI stays parallel and each check is a separate status:

```yaml
  rules-runtime:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: oven-sh/setup-bun@v2
      - run: cargo xtask ci rules-runtime --ci
```

`cargo-deny` moves from `EmbarkStudios/cargo-deny-action@v2` to
`taiki-e/install-action@v2` (`tool: cargo-deny`) plus
`cargo xtask ci cargo-deny --ci`, so the same binary invocation runs locally.
`ci.yml` jobs gain `dtolnay/rust-toolchain@stable` and `Swatinem/rust-cache@v2`,
as `platform.yml` already has. Building xtask costs one small crate per job.

Drift test (new, `xtask/src/ci.rs` `#[cfg(test)]`). It reads `ci.yml` and
`platform.yml`, parsed with `suspect_low`, which is already an xtask
dependency. It asserts three things:

- every job id appears in `JOBS` with the same `workflow`;
- every `run:` step in those two files is exactly
  `cargo xtask ci <id> --ci`;
- every `JOBS` row appears in a workflow.

This makes "CI and local can't drift" a test, not a promise.

### Rename milestone tasks

| Old | New | Why |
|---|---|---|
| `sdk-full` | `verify-sdk` | It is the maintained acceptance run for all twelve backends. |
| `sdk-m3-m6` | `verify-sdk-demo` | It verifies the frozen five-backend M3/M6 scope, whose accepted profile is `hackathon-demo`. "native" (the short version's `verify-sdk-native`) does not tell it apart, since both runners build native SDKs. |

Old names stay as aliases in the `main.rs` match for one release and print
`xtask: 'sdk-full' is now 'verify-sdk'` on stderr. Historical records in
`docs/internal/` keep the old command text, and the alias keeps it runnable.
`USAGE` strings (`sdk_full.rs:80`, `sdk_m3_m6.rs:19`) and the user-facing
references are updated: `docs/SDK-DEMO.md:66`, `docs/SDK-RELEASE-CHECKLIST.md`,
`tools/sdk-native-costs/run.py:2` (docstring) and
`crates/suspect-codegen/tests/sdk_native_measurements.rs:1,62` (comment and
ignore reason). Module and file names (`sdk_full.rs`, `sdk_m3_m6.rs`) and
fixture package names (`sdk-full`, `sdk_full`) do not change. Renaming them
would change generated package identities recorded in evidence.

### One OpenRouter input: `OPENROUTER_WEB_ROOT`

Recommendation: keep `OPENROUTER_WEB_ROOT` and retire the four file
variables and the sibling fallback.

- It already covers 188 of 224 uses, the CI variable
  (`sdk-session-performance.yml:86`) and `tools/sdk-session-perf/run.py:372`.
- Every file variable is `<root>/projects/docs/openapi/openapi.yaml`, so the
  root alone is enough.
- It stays outside `SUSPECT_*` on purpose. P2.4's registry covers product
  settings. This is a test input, so a test-only name keeps it out of
  `suspect config --explain`.

New helper, `std`-only, in `crates/suspect-codegen/src/test_inputs.rs`
(`pub mod test_inputs`). Codegen and CLI tests call it as
`suspect_codegen::test_inputs`. The 7 test files in crates that do not
depend on `suspect-codegen` include it with
`#[path = "../../suspect-codegen/src/test_inputs.rs"] mod test_inputs;`
(`suspect-ir`, `suspect-lint`, `suspect-schema`, `suspect-validate`).
That avoids a dev-dependency on the largest crate.

```rust
// new: crates/suspect-codegen/src/test_inputs.rs
//! Named external inputs for opt-in gates. A missing input fails the gate.
use std::path::PathBuf;

const ROOT: &str = "OPENROUTER_WEB_ROOT";
const SPEC: &str = "projects/docs/openapi/openapi.yaml";
/// Accepted for one release; each names the spec file, not the root.
const DEPRECATED_FILE_VARS: &[&str] = &[
    "SUSPECT_OPENROUTER_YAML", "SUSPECT_OPENROUTER_OPENAPI",
    "SUSPECT_OPENROUTER_SPEC", "OPENROUTER_PUBLIC_SCHEMA",
];

/// The openrouter-web checkout root. Panics with the exact export line when unset.
pub fn openrouter_root() -> PathBuf {
    if let Some(root) = std::env::var_os(ROOT).filter(|v| !v.is_empty()) {
        return PathBuf::from(root);
    }
    for var in DEPRECATED_FILE_VARS {
        if let Some(file) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            eprintln!("{var} is deprecated; set {ROOT} to the openrouter-web checkout");
            let file = PathBuf::from(file);
            // <root>/projects/docs/openapi/openapi.yaml -> <root>
            return file.ancestors().nth(4).map(PathBuf::from).unwrap_or(file);
        }
    }
    panic!("missing input: set {ROOT}=/path/to/openrouter-web (needs {SPEC})");
}

/// `<root>/projects/docs/openapi/openapi.yaml`, checked to exist.
pub fn openrouter_spec() -> PathBuf {
    let spec = openrouter_root().join(SPEC);
    assert!(spec.is_file(), "missing input: {} does not exist", spec.display());
    spec
}
```

Migration: one PR rewrites the lookups in the 70 Rust files outside
`xtask/` that read any of the five names or the sibling path
(`git grep -lE` over `*.rs`) to `openrouter_root()` / `openrouter_spec()`. The
same PR drops the 12 sibling fallbacks and makes the xtask set only
`OPENROUTER_WEB_ROOT`. That removes `sdk_m3_m6.rs:1158-1164`,
`sdk_full.rs:1181` and `sdk_full.rs:2498-2501`, and updates the test
assertions at `sdk_full.rs:6357-6360`. The deprecated variables keep working
for one release with the stderr note above, then are removed. Docs that
mention `SUSPECT_OPENROUTER_YAML` (`docs/SDK-GO-HTTP.md`,
`docs/SDK-PYTHON-HTTP.md`) and `OPENROUTER_PUBLIC_SCHEMA`
(`docs/SDK-SCHEMA-NUMERICS.md:95`) are updated.

### Skip vs fail: one rule

> A test in the default suite (`cargo test --workspace`) may skip when a
> native tool is missing. It prints one `skipped: <what> — <install hint>`
> line and returns. A test marked `#[ignore]` is an explicit gate: once it
> runs, a missing tool or input fails it. No gate passes by skipping. CI sets
> `SUSPECT_REQUIRE_TOOLS` so that default-suite skips for the tools it
> installs become failures.

This keeps the input rule that already exists ("missing inputs fail the
gate; they never skip it", `crates/suspect-codegen/tools/typescript-floor/README.md:15`)
and extends it to tools. Silent passes are the defect seen in Problem 6.
The rule also matches what contributors expect from `--ignored`: they asked
for the gate.

API in `suspect_codegen::toolchain`. `gate()` stays for compatibility:

```rust
/// Default-suite tests: Some(path), or None after printing the standard skip line.
/// Panics instead of skipping when `id` is listed in SUSPECT_REQUIRE_TOOLS (or it is "all").
pub fn optional(id: &str) -> Option<PathBuf>;   // new

/// #[ignore] gates: the executable, or a panic with the guidance text.
pub fn require(id: &str) -> PathBuf;            // new; = gate(id).unwrap_or_else(|m| panic!("{m}"))
```

Migration:

- The 71 `#[ignore]` early returns become `require`. Examples:
  `csharp_oauth.rs:970-978`, `cpp_incoming.rs:576-581` and the
  `go_toolchain()`/`dotnet()` helpers' `.ok()` callers inside ignored tests.
- The 28 default-suite skips become `optional`.
- The hand-rolled `tool_available` and `bun_available` helpers (6 files)
  are deleted. `suspect-rules` cannot depend on `suspect-codegen`, so its
  `bun_available` keeps its probe but reads `SUSPECT_REQUIRE_TOOLS` the
  same way (3 lines).
- A unit test in `toolchain.rs` asserts `optional` panics when
  `SUSPECT_REQUIRE_TOOLS=all`.

CI changes:

- `ci.yml` `test` job adds `oven-sh/setup-bun@v2` and runs with
  `SUSPECT_REQUIRE_TOOLS=bun,node,python`. `ubuntu-latest` ships node and
  python3, so today those tests already run, and the variable only stops a
  future silent regression.
- `corpus_suite.rs` checks `SUSPECT_REQUIRE_CORPUS` before its early
  return.
- `native-evidence.yml` replaces the nine `--test` flags with
  `cargo test -p suspect-codegen --release --tests -- --ignored ${{ matrix.test_filter }}`.
  It fixes the filters to the real file prefixes (`typescript_`, `csharp_`,
  `java_`) and drops `| tail -60 || true`. The job keeps
  `continue-on-error: true`, so it stays non-blocking but shows red when a
  backend fails.

Exact text for `CONTRIBUTING.md`, replacing the "Native SDK gates" paragraph:

```markdown
## Native SDK gates

Tests that compile and run generated SDKs need real toolchains (Go, JDK/Maven,
.NET, Swift, Dart, Node, Ruby, PHP, Python). The rule:

- A test in the default suite may skip when a tool is missing. It prints one
  `skipped:` line naming the tool and how to install it.
- A test marked `#[ignore]` is an explicit gate. When you run it
  (`cargo test -p suspect-codegen -- --ignored`), a missing tool or input
  fails it. No gate passes by skipping.
- CI sets `SUSPECT_REQUIRE_TOOLS` for the tools it installs, so a skip there
  is a failure.

Use `suspect_codegen::toolchain::optional` in default-suite tests and
`toolchain::require` in `#[ignore]` gates; `toolchain.rs` declares each tool,
its minimum version and its `SUSPECT_*_BIN` override. Gates that read the
OpenRouter specification need `OPENROUTER_WEB_ROOT=/path/to/openrouter-web`
and get it from `suspect_codegen::test_inputs`.
```

And replacing the "Checks" block:

```markdown
## Checks

`cargo xtask ci` runs every job CI runs, with the same commands; `cargo xtask
ci --list` shows them, `cargo xtask ci clippy test` runs a subset, and a CI
failure names the job to rerun locally. Jobs whose tool is missing fail with
an install hint; `--skip-missing` reports them as skipped instead.
```

### Historical records

All 8 documents with `/Users/` paths, and the 4 internal documents with dead
root-file links, are in P6's `docs/internal/` list. The P6 move takes them
out of the link checker, and `docs/internal/README.md` says why. They are
not edited.

The rest is handled separately:

- **`tools/sdk-session-perf/runners/luke-mac-inventory-20260909.json`.**
  Keep it in place, unchanged. It is a runner inventory, the machine path is
  its content, and `docs/SDK-SESSION-PERFORMANCE.md:515` cites it as "an
  observational record".
- **The 4 dead links in current docs.** Fixed in P6 step 1.
- **`tools/sdk-demo/`, `tools/sdk-demo-live/`, `tools/sdk-demo-web/` (51
  files).** They depend on root files and `examples/sdk-demo-*` configs
  that are not in the repository. They are dead code, not records. See Open
  questions.

## Compatibility and migration

| Change | Old keeps working for | Warning |
|---|---|---|
| `sdk-full`, `sdk-m3-m6` | one release | `xtask: 'sdk-full' is now 'verify-sdk'` |
| `SUSPECT_OPENROUTER_YAML`, `_OPENAPI`, `_SPEC`, `OPENROUTER_PUBLIC_SCHEMA` | one release | `<VAR> is deprecated; set OPENROUTER_WEB_ROOT ...` |
| Sibling `../openrouter-web` fallback | removed at once | panic text gives the `export` line |
| `toolchain::gate` | kept | none; new code uses `optional`/`require` |
| `#[ignore]` gates that skipped | removed at once | they now fail when run without the tool, which is the point |
| `RUSTFLAGS=-D warnings` in `ci.yml` | removed | clippy keeps `-D warnings` |

The only user-visible change is for contributors who run `--ignored` without
the tools: those runs now fail instead of passing.

## Implementation plan

1. **Alias and `ci` skeleton.** **Done.** Deviation: rustfmt, clippy and
   cargo-deny are probed with `cargo <sub> --version` (what the job runs)
   rather than `rustup component list`, so a non-rustup install works; a
   missing tool reports `MISSING` and exit 2 wins only when no job failed.
   Adds `.cargo/config.toml`, plus
   `xtask/src/ci.rs` with the job table, probes, `--list`, `--skip-missing`,
   `--fail-fast` and the summary. Ports `fmt`, `clippy`, `docs`,
   `cargo-deny`, `test`, `bench-smoke`, `corpus-validation`,
   `vscode-extension`, `zed-extension` and `rules-runtime`. Workflows are not
   changed yet. Tests: unit tests for argument parsing and the summary exit
   code. 1 day.
2. **`platform-smoke` in Rust.** **Done.** The check looks for the
   `run_done` record anywhere in stdout (the human report and the
   `run_summary` line follow it), as the old `grep -q` did. Fixtures, gateway child process with a free
   port, readiness poll, live run, cassette writer and offline run. Tests:
   the job itself, run locally and once in a PR. 0.5 day.
3. **Workflows call the xtask, and the drift test.** **Done.** Also adds
   `rust-toolchain.toml` (open question 1: `1.99.1`, with rustfmt and
   clippy). Workflows install it with `dtolnay/rust-toolchain@master`
   and `toolchain: "1.99.1"`; a second drift test checks every
   `dtolnay/rust-toolchain` step in `.github/workflows/` against the file
   (`docs.yml`, `application-targets.yml` and `native-evidence.yml`
   included; `sdk-session-performance.yml` keeps its own pinned
   `RUSTUP_TOOLCHAIN`). The drift test also rejects job-level `defaults`.
   Rewrites the `run:`
   lines in `ci.yml` and `platform.yml`. Adds toolchain/cache steps to
   `ci.yml`, moves `cargo-deny` to install-action, drops `RUSTFLAGS`, and
   adds `links` if P6 step 1 has landed. Adds the drift test. Updates the
   `CONTRIBUTING.md` "Checks" block. 0.5 day.
4. **Rename milestone tasks.** `verify-sdk`, `verify-sdk-demo`, aliases with
   warnings, updated `USAGE`, and the user-facing references listed above.
   Tests: an xtask unit test that both old names dispatch and warn. 0.25 day.
5. **`test_inputs` and the env-name migration.** Adds the helper, rewrites
   the lookup sites, drops the sibling fallbacks, and makes the xtask set one
   variable. Tests: unit tests for root vs deprecated-file resolution with a
   temp dir; `cargo test --workspace` (sites are under `#[ignore]`, so this
   checks compilation); `verify-sdk --check-syntax`. 1 day.
6. **Skip/fail rule.** Adds `toolchain::optional`/`require`, migrates the 71
   + 28 skip sites and the 6 hand-rolled probes, adds `SUSPECT_REQUIRE_TOOLS` and `SUSPECT_REQUIRE_CORPUS`,
   updates the `ci.yml` `test` job (Bun + variable), and updates the
   `CONTRIBUTING.md` "Native SDK gates" text. Tests: the `toolchain.rs` unit
   test, and a CI run showing the `suspect-rules` worker tests executing.
   1 day.
7. **Fix `native-evidence.yml`.** Filter-based selection, correct prefixes,
   no `|| true`. Tests: one `workflow_dispatch` run per backend. 0.25 day.
8. **`cargo xtask bless`.** Runs the P6 bless tests with `SUSPECT_BLESS=1`.
   0.1 day.

Total about 4.5 days. Steps 4-8 do not depend on each other. Step 3 depends
on 1-2.

## Testing

- `cargo test -p xtask`: argument parsing, summary/exit code, alias
  dispatch, and the workflow drift test, which parses YAML with
  `suspect_low`.
- `crates/suspect-codegen/src/toolchain.rs` tests: `optional` honours
  `SUSPECT_REQUIRE_TOOLS`, and `require` panics with the guidance text.
- `crates/suspect-codegen/src/test_inputs.rs` tests: root wins, a
  deprecated file variable maps to the root with a warning, and unset
  panics with the export line.
- CI: every job runs through `cargo xtask ci <job> --ci`. The `test` job
  now runs the Bun worker tests. `native-evidence` turns red on a real
  failure.
- Local: `cargo xtask ci --skip-missing` on a machine without Bun reports
  `rules-runtime` as `SKIP` and exits 0. Without the flag it exits 2.

## Alternatives considered

- **A `justfile` or `Makefile`.** It adds a tool to install, and it cannot
  run the gateway smoke portably on Windows. xtask only needs cargo. Lost.
- **One CI job that runs `cargo xtask ci` (all).** Simplest YAML, but it
  serializes about 12 jobs and gives one red/green status. Lost: CI keeps
  one job per subcommand.
- **Keep `RUSTFLAGS=-D warnings` and give xtask its own `target/ci`
  directory.** Exact parity, but double disk use and a cold build on first
  run. Lost to `clippy -- -D warnings`, which keeps the same gate.
- **Rename the input to `SUSPECT_OPENROUTER_ROOT`.** Matches the `SUSPECT_`
  prefix, but changes 188 uses and the CI variable, and it would put a test
  input into P2's product env registry. Lost.
- **Keep the file-level variable (`*_YAML`) as the one name.** Tests that
  need other files in the checkout (`schema_declarations.rs:101` reads
  `projects/docs/assets/provider-monitor-schema-v2.openapi.json`) would need
  a second variable. Lost.
- **"Everything skips" as the one rule.** It is what makes eight evidence
  jobs green today. Lost.
- **Add `suspect-codegen` as a dev-dependency of `suspect-schema`, `-lint`,
  `-validate`, `-ir` for `test_inputs`.** It compiles the largest crate for
  small test suites. Lost to the `#[path]` include of one `std`-only file.

## Risks

- **xtask build time in every job.** xtask depends on `suspect-low` and
  `suspect-source` (`xtask/Cargo.toml`). Both are already built by the jobs
  that use the workspace, and `rust-cache` keeps them. `rules-runtime` and
  `vscode-extension` pay a cold build once per cache key.
- **Contributors with partial toolchains.** Strict-by-default `cargo xtask
  ci` fails for them. The summary names the missing tools and the flag.
- **Gates that start failing.** Once `require` replaces skips in `#[ignore]`
  gates, `--ignored` runs on machines without .NET or CMake go red. That is
  the intent. Step 6's PR description lists the new failure modes.
- **Drift test brittleness.** Setup steps (`uses:`) are free-form. Only
  `run:` steps are constrained, so adding a cache step does not trip it.

## Open questions

1. **Pin the Rust toolchain?** Recommended: yes. Add a `rust-toolchain.toml`
   with an explicit stable version, so local clippy and CI clippy use the
   same lints. Today `ci.yml` uses the runner's Rust and `platform.yml` uses
   `@stable`. Update it in a monthly PR.
2. **Should `cargo xtask ci` default to all jobs, including `bench-smoke`
   and `corpus-validation` (network)?** Recommended: yes, because parity is
   the point. `--list` shows the cost, and `cargo xtask ci fmt clippy test`
   is the quick loop.
3. **How is `lychee` installed in CI?** Recommended:
   `taiki-e/install-action@v2` with `tool: lychee@0.24.2`, falling back to
   `lycheeverse/lychee-action@v2` (`lycheeVersion: v0.24.2`) only to put
   the binary on `PATH` if install-action does not carry lychee.
4. **What happens to `tools/sdk-demo*` (51 files that cannot run)?**
   Recommended: delete them in a separate PR. git history keeps them, and
   `docs/internal/` records cite their outputs, not their code. If they are
   to stay, move them under `tools/archive/` with a README.
5. **Keep `toolchain::gate`?** Recommended: keep it as the `Result` form
   that `optional` and `require` build on, and mark its doc example as the
   default-suite pattern.
6. **Should `SUSPECT_REQUIRE_TOOLS` be in P2.4's registry?** Recommended:
   yes, under the `debug` heading with the other `SUSPECT_*` development
   variables, so `suspect config --explain` stays complete.
