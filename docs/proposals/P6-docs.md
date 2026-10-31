# P6. One doc entry point

Status: proposal. Depends on: none for the move and the link checker; P1
(rule registry) and P2 (config schema, env registry) for the generated
reference pages. Decided: `README.platform.md` is already merged into
`README.md`, `docs/README.md` already indexes every file, and `docs/CLI.md`
is already generated from clap and checked by a test (see the source
document, P6 "Done so far").


Fixed since this was written (2026-10-30, see the
[index](README.md#already-fixed)): the 4 broken links and the missing `examples/sdk-demo.json`. The Problem section keeps
them as evidence.
## Summary

A reader finds every user document under four directories: `docs/guide/`
(tasks), `docs/reference/` (lookup tables, some generated), `docs/sdk/<lang>/`
(per-backend notes) and `docs/design/` (current architecture). Plans, exit
evidence, dated reviews and research move to `docs/internal/`, unchanged
except for link paths. In the code, 119 flat files move with 671 relative
links rewritten by one script. 19 load-bearing paths in `xtask/src/sdk_full.rs`
and the tests change, and a link checker (lychee, offline) runs in CI over
everything except `docs/internal/`.

## Problem

1. **119 flat files, four kinds of content mixed.** `docs/` holds 119
   Markdown files with no directory structure. The source document says 120.
   It counted the `ls` header. `docs/README.md` groups them by heading only
   (`docs/README.md:7-119`). User guides, generated references, per-language
   notes, design notes and 28 "plans, reviews and history" entries
   (`docs/README.md:85-119`) sit side by side. The source document says ~27.
2. **Links are broken today, and nothing checks them.** There is no
   `lychee.toml`, `.lycheeignore` or link job in `.github/workflows/`. lychee
   0.24.2 is installed locally. An offline run over every tracked `*.md`:

   ```text
   $ lychee --offline --format detailed $(git ls-files '*.md')
   Total 1537  Successful 517  Excluded 496  Errors 524
   ```

   All 524 errors are "File not found". By target: 454 point into `target/`
   (build output that is not committed), 18 at `DEMO-README.md`, 13 at
   `LIVE-DEMO-README.md`, 7 at `LIVE-ENV-DEMO-README.md`, 2 at
   `docs/OPENROUTER-ACCEPTANCE.md`, and the rest at `examples/sdk-demo-*`
   files that do not exist (`ls examples` shows only `load/`). By source:
   348 are in `docs/SDK-CURRENT-SDK-COMPARISON-20260911.md` and 155 in
   `docs/SDK-DX-REVIEW-20260911.md`, both dated reviews. Only 4 broken links
   are in documents a user reads (see Design, "Link checker").
3. **A user-facing entry point is broken in a way lychee cannot see.**
   `README.md:17` sends SDK users to `docs/SDK-DEMO.md`. That page tells them
   to run `codegen-session --config examples/sdk-demo.json`
   (`docs/SDK-DEMO.md:13-15`). The file does not exist and never existed in
   this repository's history (`git log -- examples/sdk-demo.json` is empty).
   The path is inside a code block, so a link checker does not see it.
4. **Code depends on doc paths, and CI cannot see when they break.** 64
   `docs/<NAME>.md` references sit in 22 non-Markdown files
   (`git grep -noE 'docs/[A-Z][A-Za-z0-9_-]+\.md' -- ':!*.md'`). Most are
   comments or error text. These read the file at runtime:
   - `crates/suspect-cli/tests/cli_reference.rs:48` (`docs/CLI.md`, bless test).
   - `crates/suspect-codegen/tests/feature_manifest.rs:74`
     (`docs/SDK-FEATURES.md`, bless test).
   - `crates/suspect-codegen/tests/kotlin_sdk.rs:417`,
     `tests/kotlin_support/mod.rs:80`, `tests/kotlin_validation_v2.rs:677`,
     `tests/kotlin_validation_v3.rs:379`. They extract Kotlin snippets from
     `docs/SDK-KOTLIN.md`. All four run only inside `#[ignore]` native gates,
     so a bad path would pass CI.
   - `xtask/src/sdk_full.rs:97-108`: `Target.docs` for 12 backends, checked by
     `syntax_check` at `sdk_full.rs:5015-5021`.
   - `xtask/src/sdk_full.rs:5003-5009`: 7 required "support sources".
     `syntax_check` runs only from `sdk-full --check-syntax`, never in
     `cargo test`. Today it reports `"readyToBuild":true`.
   - `xtask/src/sdk_full.rs:80` and `xtask/src/sdk_m3_m6.rs:19`. Both
     `USAGE` strings point at `docs/SDK-FULL-EXIT.md` and
     `docs/SDK-M3-M6-EXIT.md`.
5. **Two bless variables for one pattern.** `docs/CLI.md` regenerates with
   `SUSPECT_BLESS=1` (`cli_reference.rs:50`). `docs/SDK-FEATURES.md`
   regenerates with `SUSPECT_REGEN_FEATURES=1` (`feature_manifest.rs:76`).
6. **No generated reference for diagnostics, config or env vars.**
   Diagnostic codes live in a `match` in `suspect-validate/src/guidance.rs`
   (`summary_entry`, not iterable) and in the crate-private
   `suspect_lint::packs::oas::rules()` (`packs/oas.rs:42`). No page lists
   them. `suspect config` has no schema output (`docs/CLI.md:1067-1079`).

## Goals and non-goals

Goals:

- Every file under `docs/` belongs to exactly one of `guide/`, `reference/`,
  `sdk/<lang>/`, `design/`, `internal/` or `proposals/`, and the move table
  below covers all 119.
- `README.md` stays the single landing page. `docs/README.md` stays the single
  index.
- Relative links are checked on every PR, offline, in under a second.
- Every doc path that code reads is checked by a default-suite test.
- Reference pages that can be generated are generated, with one bless
  variable.

Non-goals:

- Rewriting historical records. `docs/internal/` keeps its files byte-for-byte
  apart from link paths, and keeps its original file names.
- Publishing Markdown as a site (mdBook). `docs.yml` keeps publishing rustdoc
  only.
- Checking external `http(s)` links on PRs. Network checks are flaky; see
  Open questions.
- Writing the new guide pages' prose in the move PRs.

## Design

### Layout

```text
README.md                   install, tour, command map, links (unchanged role)
docs/README.md              the index, regrouped by directory
docs/guide/                 task-oriented pages
docs/reference/             lookup pages; cli, sdk-features, diagnostics,
                            config, environment are generated
docs/sdk/<lang>/            per-backend notes; README.md is the overview
docs/design/                current architecture notes (link-checked)
docs/internal/              plans, exits, evidence, dated reviews, research
                            (flat, original names, not link-checked)
docs/proposals/             README.md (the umbrella proposal) + P1..P8
docs/images/, docs/demo/, docs/capture/   unchanged
```

Two changes from the short version:

- **`docs/design/` is added.** 19 files are current design records that code
  comments cite and tests tell contributors to update. Examples:
  `docs/SDK-ELIMINATION-AUDIT.md`, which `cpp_elimination.rs:599` tells
  contributors to update, and `docs/SDK-PINNED-CLOSURE-DESIGN.md`, cited by
  `suspect-ref/src/acquire.rs:6`. Putting them in `internal/` would exclude
  living documents from the link checker.
- **`docs/internal/` stays flat with original names.** Its 34 files link to
  each other heavily. Keeping names unchanged means only links that leave
  `internal/` change, and the old names stay searchable.
- **`CLI.md` goes to `reference/cli.md`, not `guide/`.** It is generated
  lookup material.
- **No `guide/getting-started.md`.** `README.md` already holds the tour, and
  a second copy would bring back the two-README problem.

### Move table

All 119 files. `docs/` prefix omitted on both sides. Destinations are
lower-case kebab-case except under `internal/`.

Root and proposals:

| Old | New |
|---|---|
| `README.md` | `README.md` (rewritten index) |
| `TOOLCHAIN-UNIFICATION-PROPOSAL.md` | `proposals/README.md` (done 2026-10-30) |

`guide/` (10):

| Old | New |
|---|---|
| `LOAD-TESTING.md` | `guide/load-testing.md` |
| `TEST-CREDENTIALS.md` | `guide/test-credentials.md` |
| `SDK-GOLDEN-DEFAULTS.md` | `guide/sdk-generation.md` |
| `SDK-INCREMENTAL-GENERATION.md` | `guide/sdk-incremental-generation.md` |
| `SDK-EDITOR-PREVIEW.md` | `guide/sdk-editor-preview.md` |
| `SDK-COMPATIBILITY-REPORTS.md` | `guide/sdk-compatibility-reports.md` |
| `SDK-INTERPRETATION-PROFILES.md` | `guide/sdk-interpretation-profiles.md` |
| `SDK-LARGE-CONTRACTS.md` | `guide/sdk-large-contracts.md` |
| `SDK-RELEASE-CHECKLIST.md` | `guide/sdk-release-checklist.md` |
| `APPLICATION-TARGETS.md` | `guide/application-targets.md` |

`reference/` (7 moved, 3 new generated):

| Old | New |
|---|---|
| `CLI.md` | `reference/cli.md` (generated, exists) |
| `SDK-FEATURES.md` | `reference/sdk-features.md` (generated, exists) |
| `LSP_FEATURES.md` | `reference/lsp-features.md` |
| `SDK-CAPABILITIES.md` | `reference/sdk-capabilities.md` |
| `SDK-OPENAPI-MATRIX.md` | `reference/openapi-matrix.md` |
| `SDK-CREDENTIAL-ENV.md` | `reference/credential-env.md` |
| `SDK-ARTIFACT-OWNERSHIP.md` | `reference/artifact-ownership.md` |
| (new) | `reference/diagnostics.md` (generated, P1) |
| (new) | `reference/config.md` + `reference/suspect.schema.json` (generated, P2.7) |
| (new) | `reference/environment.md` (generated, P2.4) |

`sdk/<lang>/` (47). The language prefix is dropped from the file name. Where a
single overview exists, it becomes `README.md`. Where none exists, the move PR
adds a short `README.md` that lists the files, taken from
`docs/README.md:50-53`.

| Old | New |
|---|---|
| `SDK-TYPESCRIPT-{CODECS,CREDENTIAL-ENV,DIRECTIONAL,HTTP,JSON,MODEL-PLAN,PROTOCOL,VALIDATION-V2,VALIDATION-V3}.md` | `sdk/typescript/{codecs,credential-env,directional,http,json,model-plan,protocol,validation-v2,validation-v3}.md` |
| `SDK-PYTHON-{CODECS,DX,HTTP,JSON,MODEL-PLAN,PROTOCOL,RESOURCES,VALIDATION}.md` | `sdk/python/{codecs,dx,http,json,model-plan,protocol,resources,validation}.md` |
| `SDK-GO-{CODECS,CREDENTIAL-ENV,HTTP,JSON,MODEL-PLAN,PROTOCOL,SCHEMA-V2,SCHEMA-V3,VALIDATION}.md` | `sdk/go/{codecs,credential-env,http,json,model-plan,protocol,schema-v2,schema-v3,validation}.md` |
| `SDK-RUST-{CREDENTIAL-ENV,HTTP,JSON,MODEL-PLAN,PROTOCOL,RESOURCES,SERDE,VALIDATION-V2}.md` | `sdk/rust/{credential-env,http,json,model-plan,protocol,resources,serde,validation-v2}.md` |
| `SDK-SWIFT.md` | `sdk/swift/README.md` |
| `SDK-SWIFT-{CREDENTIAL-ENV,PROTOCOL,RESOURCES,VALIDATION-V2}.md` | `sdk/swift/{credential-env,protocol,resources,validation-v2}.md` |
| `SDK-JAVA.md` | `sdk/java/README.md` |
| `SDK-KOTLIN.md` | `sdk/kotlin/README.md` |
| `SDK-CSHARP.md` | `sdk/csharp/README.md` |
| `SDK-RUBY.md` | `sdk/ruby/README.md` |
| `SDK-PHP.md` | `sdk/php/README.md` |
| `SDK-DART.md` | `sdk/dart/README.md` |
| `SDK-CPP.md` | `sdk/cpp/README.md` |
| `SDK-TERRAFORM-STRETCH.md` | `sdk/terraform/README.md` |

`design/` (19):

| Old | New |
|---|---|
| `SDK-CONTRACT-READERS.md` | `design/contract-readers.md` |
| `SDK-CONTRACT-RESOURCES.md` | `design/contract-resources.md` |
| `SDK-HTTP-PROTOCOL.md` | `design/http-protocol.md` |
| `SDK-NATIVE-DX.md` | `design/native-dx.md` |
| `SDK-OAS32-CONTRACT.md` | `design/oas32-contract.md` |
| `SDK-OWNED-VALIDATION.md` | `design/owned-validation.md` |
| `SDK-PINNED-CLOSURE-DESIGN.md` | `design/pinned-closure.md` |
| `SDK-PROTOCOL-NEXT.md` | `design/typescript-protocol-next.md` |
| `SDK-REMAINING-DX.md` | `design/remaining-native-dx.md` |
| `SDK-SCHEMA-APPLICATORS.md` | `design/schema-applicators.md` |
| `SDK-SCHEMA-DIALECTS.md` | `design/schema-dialects.md` |
| `SDK-SCHEMA-NATIVE-ADOPTION.md` | `design/schema-native-adoption.md` |
| `SDK-SCHEMA-NUMERICS.md` | `design/schema-numerics.md` |
| `SDK-SCHEMA-RESOURCES.md` | `design/schema-resources.md` |
| `SDK-SCHEMA-URIS.md` | `design/schema-uris.md` |
| `SDK-ELIMINATION-AUDIT.md` | `design/elimination-audit.md` |
| `SDK-EXAMPLES.md` | `design/sdk-examples.md` |
| `SDK-SESSION-PERFORMANCE.md` | `design/sdk-session-performance.md` |
| `TS-RULE-RUNTIME.md` | `design/ts-rule-runtime.md` |

`internal/` (34, same file name, `internal/<NAME>.md`):

| Kind | Files |
|---|---|
| Plans | `ARBITER-FOLD-PLAN`, `CODE-FOLD-PLAN`, `DEPTH-ANALYSIS`, `FEATURE-ROADMAP`, `LOAD-TESTING-PLAN`, `LSP-DEPTH-ENRICHMENT`, `REVOLUTIONARY-FEATURES`, `SDK-GENERATION-PLAN`, `SDK-GOLDEN-DEFAULTS-PLAN`, `SDK-FULL-PLAN-WORK`, `SDK-COMPAT-DROPIN-PLAN`, `SDK-TODAY-M3-M6` |
| Exit and evidence records | `SDK-FULL-EXIT`, `SDK-FULL-PLAN-STATUS`, `SDK-M0-KNOWN-FAILURES`, `SDK-M0-M2-DX-APPROVAL`, `SDK-M0-M2-EXIT`, `SDK-M0-WORKFLOW`, `SDK-M2-CALLSITES`, `SDK-M3-M6-EXIT`, `SDK-PROGRESS`, `SDK-SESSION-HANDOFF`, `SDK-NATIVE-COSTS`, `PERFORMANCE`, `SDK-DEMO` |
| Dated reviews | `SDK-CURRENT-SDK-COMPARISON-20260911`, `SDK-DX-REVIEW-20260911`, `SDK-DX-ASSESSMENT`, `SDK-COMPAT-DIFF-PYTHON`, `SDK-COMPAT-DIFF-TYPESCRIPT` |
| Research | `LSP_COMPARISON`, `SDK-OPENAPI-RESEARCH`, `SDK-SPEAKEASY-RESEARCH`, `SPEAKEASY-OPENAPI-GENERATION-REVIEW` |

Classification notes, one line each:

- `SDK-DEMO.md` is internal because it is the hackathon script and its
  config file is missing (Problem 3). `README.md` links
  `guide/sdk-generation.md` instead.
- `SDK-PROGRESS.md` ("Updated 2026-09-10", hackathon scope) is internal.
  `README.md:19` drops the link.
- `LSP_COMPARISON.md` is a competitive analysis. `README.md:48` keeps its
  link, which may point into `internal/` because lychee checks the README's
  own links, and the target exists.
- `PERFORMANCE.md` is titled "Historical performance profiling report".
  `BENCHMARKS.md:226` keeps its link.
- `SDK-TERRAFORM-STRETCH.md` documents the shipped Terraform target, so it
  goes under `sdk/`, not `internal/`.

The new hand-written guide pages are `guide/config.md`, `guide/editors.md`,
`guide/linting.md` (including custom TS rules), `guide/gateway.md` and
`guide/docs-generation.md`. They come after the move (step 5), written from
the existing `README.md` sections. `guide/config.md` waits for P2.

### Link rewrite

One script does `git mv` and rewrites every relative Markdown link. It covers
inline links and reference-style definitions (`[x]: path`). It skips fenced
code and URLs with a scheme. For each link it resolves the old target
against the old source location, maps both through the move table, and
writes `os.path.relpath(new_target, new_source_dir)` with the fragment kept.
The script goes in the PR description, not in the repository.

I ran this design against a copy of the tracked tree (`/tmp/p6sim`):

```text
links inbound to docs/*.md: 414 (399 from docs/, 15 from outside)
relative links rewritten:   671 (366 because the target moved,
                                 305 because the source moved)
  in docs/internal/ 342, docs/README.md and proposals 119, sdk/ 90,
  reference/ 41, guide/ 35, design/ 31, README.md 10, BENCHMARKS.md 2,
  tools/sdk-demo/README.md 1
lychee (candidate config, below) before the move: 4 errors; after: the same 4
```

The 15 inbound links from outside `docs/` are `README.md` (11),
`BENCHMARKS.md` (2), `CONTRIBUTING.md:11` (`docs/README.md`, unchanged) and
`tools/sdk-demo/README.md:3`. The `README.md` links to `docs/images/*` and
`docs/demo/petstore.yaml` do not change.

Code paths are changed by hand in the same PR as the files they name:

| File | Change |
|---|---|
| `crates/suspect-cli/tests/cli_reference.rs:1,48,57` and the generated header text | `docs/reference/cli.md` |
| `CONTRIBUTING.md` "Generated files" | `docs/reference/cli.md` |
| `crates/suspect-codegen/tests/feature_manifest.rs:9,74,80,84`, `src/features.rs:8,234` | `docs/reference/sdk-features.md` |
| `crates/suspect-codegen/tests/kotlin_sdk.rs:417`, `kotlin_support/mod.rs:80`, `kotlin_validation_v2.rs:677`, `kotlin_validation_v3.rs:379` | `docs/sdk/kotlin/README.md` |
| `xtask/src/sdk_full.rs:97-108` (`TARGETS[*].docs`) | `docs/sdk/<lang>/README.md`, or `http.md` for typescript, rust, python, go |
| `xtask/src/sdk_full.rs:5003-5009` | `docs/sdk/rust/resources.md`, `docs/sdk/swift/resources.md`, `docs/sdk/go/schema-v3.md`, `docs/design/http-protocol.md`, `docs/sdk/typescript/protocol.md`, `docs/reference/credential-env.md`, `docs/design/schema-native-adoption.md` |
| `xtask/src/sdk_full.rs:80`, `xtask/src/sdk_m3_m6.rs:19` (`USAGE`) | `docs/internal/SDK-FULL-EXIT.md`, `docs/internal/SDK-M3-M6-EXIT.md` |
| Comments and messages: 22 in `*_elimination.rs`, plus `sdk_xml_media.rs:6`, `contract_oas32.rs:3`, `owned_dialects.rs:6`, `acquire.rs:6`, `suspect-rules/src/lib.rs:3`, `suspect-gateway/src/lib.rs:1104`, `sdk_session_bench.rs:1`, `application-targets.yml:8` | new paths |

### Guard: doc paths named in code must exist (new)

New test `xtask/src/docs_paths.rs` (`#[cfg(test)]`). It runs in
`cargo test --workspace` because `xtask` is a workspace member
(`Cargo.toml:3`). It scans the tracked non-Markdown files for repository
doc paths and asserts each file exists. The pattern accepts today's
upper-case names and the new directories only. Generated SDK packages also
emit paths like `"docs/reference.md"` (`cpp_sdk/emit.rs:890`) and
`"docs/guide.md"` (`:894`), and a plain `docs/*.md` pattern would flag them
falsely. I checked that no generated package path matches
`docs/(guide|reference|sdk|design|internal|proposals)/`. A second test calls
`sdk_full::syntax_check(repo_root())` and asserts `readyToBuild`. Without
it, the 19 `sdk_full.rs` paths are only checked by a manual command.

```rust
// new: xtask/src/docs_paths.rs
#[test]
fn every_docs_path_named_in_code_exists() {
    let root = crate::repo_root();
    let re = regex_lite::Regex::new(
        r"docs/(?:[A-Z][A-Za-z0-9_-]*\.md|(?:guide|reference|sdk|design|internal|proposals)/[A-Za-z0-9_./-]+\.md)",
    )
    .unwrap();
    let mut missing = Vec::new();
    for file in crate::tracked_files(&root, |p| !p.ends_with(".md")) {
        let text = std::fs::read_to_string(root.join(&file)).unwrap_or_default();
        for m in re.find_iter(&text) {
            if !root.join(m.as_str()).is_file() { missing.push(format!("{file}: {}", m.as_str())); }
        }
    }
    assert!(missing.is_empty(), "doc paths named in code do not exist:\n{}", missing.join("\n"));
}
```

(`regex-lite` is new to xtask. A hand-written scanner is fine as well.
`repo_root` and `tracked_files` are new helpers shared with P7's `ci`
module.)

### Link checker

New `lychee.toml` at the root. I validated it with lychee 0.24.2. In 0.24,
`include_fragments` takes a string, and `true` is a parse error.

```toml
# lychee.toml — relative links only; external URLs are excluded by `offline`.
offline = true
include_fragments = "anchor-only"
no_progress = true
exclude_path = [
  "(^|/)target/",
  "(^|/)node_modules/",
  # SDK guide templates: their links resolve inside the generated package.
  "crates/suspect-codegen/src/[a-z]+_sdk/",
  # Frozen historical records; see docs/internal/README.md.
  "docs/internal/",
]
exclude = [
  "^file:///ws/",   # example URI in reference/lsp-features.md
  "/target/",       # links to local build evidence
]
```

Inputs are `git ls-files '*.md'`, so untracked build output is never
scanned. Run locally:

```text
$ lychee -c lychee.toml $(git ls-files '*.md')     # today, with docs/internal/ simulated
Total 496  Successful 384  Excluded 108  Errors 4
  docs/SDK-CAPABILITIES.md:118   OPENROUTER-ACCEPTANCE.md
  docs/SDK-NATIVE-DX.md:16       OPENROUTER-ACCEPTANCE.md
  docs/SDK-CREDENTIAL-ENV.md:175 ../LIVE-ENV-DEMO-README.md
  tools/sdk-demo-web/README.md:26 ../../LIVE-ENV-DEMO-README.md
```

The first PR fixes these 4 by removing the link and keeping the text, since
none of the targets exists in this repository. The same scan of
`docs/internal/` alone reports 60 errors outside `target/`. That set is
excluded and left as recorded.

CI job, added to `.github/workflows/ci.yml`. It is shown here in its
pre-P7 form. P7 replaces the `run` with `cargo xtask ci links`.

```yaml
  links:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: lycheeverse/lychee-action@v2
        with:
          lycheeVersion: v0.24.2
          args: --config lychee.toml --no-progress './**/*.md'
          fail: true
```

`exclude_path` keeps `target/` and `node_modules/` out when the glob form is
used in CI.

`docs/internal/README.md` (new, 5 lines) states that the directory is
frozen, not link-checked, and may name machines and files that no longer
exist.

### Generated reference pages

There is one pattern, which already exists twice: a test renders the page,
compares it with the checked-in file, and rewrites it when `SUSPECT_BLESS` is
set. The new test file `crates/suspect-cli/tests/reference_docs.rs` absorbs
`cli_reference.rs` and renders four pages:

| Page | Source | Depends on |
|---|---|---|
| `reference/cli.md` | `suspect_cli::Cli::command()` (as today) | none |
| `reference/diagnostics.md` | P1 rule registry. Columns: code, owning check, default severity, aliases, summary, how to fix. Summary and how-to-fix come from today's `suspect_validate::guidance::{summary, how_to_fix}`. | P1 step "rule registry" |
| `reference/config.md`, `reference/suspect.schema.json` | P2.7 JSON Schema generated from the `suspect-config` types. The Markdown is a key table rendered from the schema. | P2.7 |
| `reference/environment.md` | P2.4 env var registry (`SUSPECT_*` name, setting path, debug flag) | P2.4 |

`docs/reference/sdk-features.md` stays generated by
`crates/suspect-codegen/tests/feature_manifest.rs`, because the registry lives
in `suspect-codegen`. That test switches from `SUSPECT_REGEN_FEATURES` to
`SUSPECT_BLESS` and accepts the old name for one release.

Today there is no public way to list every diagnostic code. `guidance.rs` is
a `match` and `packs::oas::rules()` is `pub(crate)`. So
`reference/diagnostics.md` waits for P1. A stopgap from today's sources
would have to be deleted when P1 lands.

## Compatibility and migration

- **Old URLs.** GitHub does not redirect moved files. External bookmarks to
  `docs/SDK-*.md` break. The project is not published yet (P5 has no
  release), so I recommend no stub files.
- **Bless variable.** `SUSPECT_REGEN_FEATURES=1` keeps working for one
  release. When used, it prints
  `SUSPECT_REGEN_FEATURES is deprecated; use SUSPECT_BLESS=1`.
- **xtask.** `sdk-full --check-syntax` gives the same result after the move.
  Step 3 verifies this by running it and by the new test.
- **Open branches.** A branch that edits a moved file gets a rename conflict.
  git usually follows renames. The PR description lists `old -> new`, so
  rebasing is mechanical.
- **Other proposals.** `docs/proposals/P*.md` link to
  `../TOOLCHAIN-UNIFICATION-PROPOSAL.md`. The script rewrites those to
  `README.md`.

## Implementation plan

1. **Link checker on today's layout.** **Done.** The 4 links were already
   fixed. The `links` job runs `cargo xtask ci links --ci` (P7) with lychee
   from `taiki-e/install-action@v2` (`lychee@0.24.2`, P7 open question 3)
   instead of `lychee-action`; inputs are `git ls-files '*.md'`, run with
   `-q`. Adds `lychee.toml`. Its
   `exclude_path` temporarily lists the 34 internal file names as one regex
   (tested above). Adds the `links` job in `ci.yml`, fixes the 4 broken links,
   and adds a `lychee` line to `CONTRIBUTING.md`. Files: `lychee.toml`,
   `.github/workflows/ci.yml`, `docs/SDK-CAPABILITIES.md`,
   `docs/SDK-NATIVE-DX.md`, `docs/SDK-CREDENTIAL-ENV.md`,
   `tools/sdk-demo-web/README.md`, `CONTRIBUTING.md`. Tests: the CI job.
   0.5 day.
2. **Guard tests.** **Done.** The pattern is a hand-written scanner (no
   `regex-lite`, so `Cargo.lock` is unchanged); `docs_paths.rs` is a
   `#[cfg(test)]` module and skips its own samples. `repo_root` and
   `tracked_files` live in `xtask/src/main.rs`, shared with `ci.rs`. Adds `xtask/src/docs_paths.rs` with
   `every_docs_path_named_in_code_exists` and
   `repository_passes_sdk_full_syntax_check`. Both pass on today's layout.
   Files: `xtask/src/main.rs`, `xtask/src/docs_paths.rs`, `xtask/Cargo.toml`.
   0.5 day.
3. **Move `internal/` (34 files).** Runs the script for the internal rows
   only. Adds `docs/internal/README.md`, collapses `exclude_path` to
   `docs/internal/`, and updates the two `USAGE` strings and
   `suspect-gateway/src/lib.rs:1104`. Regroups `docs/README.md` "Plans,
   reviews and history" to point into `internal/`. Tests: lychee job, guard
   tests, `cargo run -p xtask -- sdk-full --check-syntax`. 0.5 day.
   **Done.** The table still covered every file (no new flat docs since it
   was written). The script rewrote 421 links (342 in `docs/internal/`, 34
   in `docs/README.md`, 4 in `README.md`, 1 each in `BENCHMARKS.md` and
   `tools/sdk-demo/README.md`, the rest in user docs that cite records);
   lychee reports 0 errors before and after. Entries that pointed at
   internal records from other `docs/README.md` sections (LSP comparison,
   SDK demo, performance report, native costs, DX assessment, OpenAPI
   research) moved into the "Internal" section, and the elimination audit
   moved from it to "Design notes". The prose path in `TS-RULE-RUNTIME.md:3`
   became a link. The guard's syntax-check test stands in for the manual
   `verify-sdk --check-syntax` run.
4. **Move `sdk/`, `design/`, `reference/`, `guide/`, `proposals/` (85
   files).** Runs the script for the rest. Applies the code-path table above
   and adds the four `sdk/<lang>/README.md` index pages (typescript, python,
   go, rust). Rewrites `docs/README.md` as one section per directory. Updates
   `README.md`: the SDK links (`:17-19`) point at `guide/sdk-generation.md`
   and `reference/sdk-capabilities.md`, and the `:156` links follow the move.
   Tests: lychee job, guard tests, `cli_reference`, `feature_manifest`.
   1 day.
   **Done.** `proposals/` was already in place, so 83 files moved. The
   script rewrote 337 links (90 in `sdk/`, 84 in `docs/README.md`, 52 in
   `internal/` pointing out of it, 39 in `reference/`, 35 in `guide/`, 31
   in `design/`, 5 in `README.md`, 1 in `BENCHMARKS.md`); lychee reports 0
   errors after. The review grep for `docs/[A-Z]` found 26 prose paths in
   code spans outside `internal/` and `proposals/` (`CONTRIBUTING.md`,
   `README.md`, `design/schema-native-adoption.md`, `reference/openapi-matrix.md`,
   `guide/sdk-release-checklist.md`, `tools/sdk-demo/README.md`); they name
   the new paths too. `internal/` and `proposals/` keep their prose paths
   as recorded. The generated `reference/sdk-features.md` links its source
   one level deeper, so `features.rs` renders `../../crates/...`. The
   `README.md` SDK paragraph links `guide/sdk-generation.md` and
   `reference/sdk-capabilities.md` (the demo and progress links are gone),
   and "Roadmap" links `proposals/README.md` first (open question 3).
   `docs/README.md` has a directory table and one section per directory.
5. **New guide pages.** Adds `guide/editors.md`, `guide/linting.md`,
   `guide/gateway.md` and `guide/docs-generation.md`, written from
   `README.md` sections and `editors/*/README.md`. Shortens `README.md` to
   install, tour, command map and links. 1 day.
   **Done.** `guide/linting.md` covers `check`/`validate`/`lint`, the SDK
   category, `lint.*` settings, suppression comments and
   `x-suspect-disable`, baselines, rulesets and TS rules (P1 steps 5-8);
   `guide/editors.md` takes the README's screenshot gallery and the VS Code
   binary order, settings and commands; `guide/gateway.md` takes the
   cassette format; `guide/docs-generation.md` covers `gen docs` and `gen
   template` (manifest, context, filters, user-code regions, ownership).
   The README command map is the P3 tree. The crate table and the
   "Notable engineering" notes moved to `CONTRIBUTING.md` ("Crates", which
   gains the `suspect-diag` and `suspect-check` rows). `guide/config.md` is
   not written here; `reference/config.md` (step 7) is generated instead.
6. **`reference_docs.rs` and `SUSPECT_BLESS` everywhere.** Folds
   `cli_reference.rs` into `reference_docs.rs` and renames the
   feature-manifest variable with the alias. 0.25 day.
   **Done.** `reference_docs.rs` has one `bless(page, rendered)` helper and
   one test per page; the generated header of `reference/cli.md` names the
   new test. `feature_manifest` reads `SUSPECT_BLESS`, and
   `SUSPECT_REGEN_FEATURES=1` still writes the page after printing
   `SUSPECT_REGEN_FEATURES is deprecated; use SUSPECT_BLESS=1`. `BLESS_TESTS`
   in `xtask/src/main.rs` now lists every test that rewrites a checked-in
   file under `SUSPECT_BLESS`: `reference_docs`, `feature_manifest`,
   `json_schema` (the VS Code schemas, P2.7), and the `finding_output` and
   `lsp_diagnostics_golden` goldens (P1), so `cargo xtask bless` producing
   no diff checks all of them. `CONTRIBUTING.md` "Generated files" lists
   them.
7. **Generated pages after P1/P2.** Adds `reference/diagnostics.md` (after
   P1), `reference/config.md` + `suspect.schema.json` (after P2.7) and
   `reference/environment.md` (after P2.4). Each is one render function plus
   one assertion in `reference_docs.rs`. 0.5 day each.
   **Done.** `diagnostics_page` renders `suspect_check::registry::builtin()`
   with one section per check (code, default severity, class, summary, how
   to fix, aliases; a lint code that is an alias says "alias of `X`"),
   plus the admission code families. `config_page` renders
   `schema::SETTINGS` one subsection per top-level key (type and values,
   default, description, env variable, VS Code setting and its older
   `suspect.*` keys, flags, older spellings), `schema::MANIFEST`,
   `TARGET_FIELDS`, `RELEASE_FIELDS` and `RELEASE_TARGET_FIELDS`.
   `environment_page` renders `schema::env_vars()` and `DEBUG_VARS`.
   Deviations: no `reference/suspect.schema.json` copy; `config.md` links
   the schemas P2.7 already blesses in `editors/vscode/schemas/`, so there
   is one checked-in schema per file. The setting tables name flags with
   the pre-P3 command names (`gateway --port`); the page spells them with
   today's path through `legacy::RENAMED`/`DEFAULT_SUB`
   (`suspect traffic gateway --port`) and fails when a flag no longer
   exists in the clap tree. A prose default ("the directory name") is not
   put in a code span.

8. **(N+1) Bless variable** (Compatibility): `SUSPECT_REGEN_FEATURES`
   stops working. **Done** (in release N: no release had been tagged).
   `feature_manifest.rs` reads only `SUSPECT_BLESS`.

Steps 1-4 total 2.5 days. Each step leaves CI green: after steps 3 and 4 the
lychee job, the guard tests and the two bless tests all run on the new paths.

## Testing

- `links` CI job: lychee over `git ls-files '*.md'` with `lychee.toml`, under
  1 second offline (the local run took 11 ms for `docs/internal/`).
- `cargo test -p xtask`:
  `docs_paths::every_docs_path_named_in_code_exists` and
  `docs_paths::repository_passes_sdk_full_syntax_check` (new, default
  suite).
- `cargo test -p suspect-cli --test reference_docs` (renamed from
  `cli_reference`) and `cargo test -p suspect-codegen --test
  feature_manifest` keep the generated pages exact.
- The move script run is reproducible. The PR description includes its
  output, `rewritten links total: N`, and the lychee summary before and after.
- No editor host smoke is affected. The extensions do not read `docs/`.

## Alternatives considered

- **Keep flat, prefix-only (`docs/SDK-*`, `docs/INTERNAL-*`).** No link
  rewrites, but no directory exclusion for the link checker and no per-language
  grouping. Lost because it does not separate history from current docs.
- **Rename internal files to kebab-case too.** Cosmetic. It rewrites about
  340 intra-internal links in frozen records and breaks text search for the old
  names. Lost.
- **Delete historical records instead of moving them.** They are cited by
  current docs (for example `SDK-NATIVE-COSTS.md` cites
  `SDK-FULL-EXIT.md#native-cost-integration-contract`) and hold acceptance
  evidence. Lost. git history alone is harder to cite.
- **`markdown-link-check` (npm) or a Rust test instead of lychee.**
  markdown-link-check needs Node in the Rust jobs and is slower. A custom test
  would re-implement fragment checking. lychee is a single binary, already used
  locally, and handles reference-style links and anchors.
- **Generate reference pages with an `xtask docs` command.** A bless test
  fails CI on drift without a separate job, and it already works for two
  pages. Lost. P7's `cargo xtask bless` only wraps the bless tests.

## Risks

- **Silent breakage in ignored gates.** The Kotlin snippet extraction reads a
  moved file only under `#[ignore]`. The guard test covers the path string.
  Without it, the break would surface only in a native run.
- **Script misses a link form.** HTML `<a href>` and bare paths in prose are
  not rewritten. lychee before/after counts catch HTML links. Bare paths in
  code spans (like Problem 3) are not checked by anything, so step 4's review
  greps for `docs/[A-Z]` afterwards.
- **Large diff.** Steps 3 and 4 touch about 120 files. They contain only
  renames plus link edits, so reviewers can use `git diff -M --stat` and
  `git diff -M --word-diff`.

## Open questions

1. **Check external URLs?** Recommended: not on PRs. Add a weekly scheduled
   job (`lychee --config lychee.toml --no-offline`) that opens no issues and
   only uploads its report.
2. **Stub files at old paths?** Recommended: no. Nothing is published yet.
3. **Should `FEATURE-ROADMAP.md` stay user-facing (it is linked from
   `README.md:215`)?** Recommended: move to `internal/` as listed, and have
   `README.md` link `docs/proposals/README.md` for current plans. The roadmap
   lists features "next", which goes stale.
4. **Fix the missing `examples/sdk-demo.json` (Problem 3)?** Recommended:
   out of scope here. `SDK-DEMO.md` moves to `internal/`, and
   `guide/sdk-generation.md` uses `examples/load/petstore.openapi.yaml` or a
   new small example in step 5.
5. **`lycheeVersion` pinning?** Recommended: pin `v0.24.2` (the local
   version) in CI and in `CONTRIBUTING.md`. The config syntax changed in 0.24
   (`include_fragments` became a string).
