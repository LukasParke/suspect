# P8. Crate consolidation

Status: proposal. Depends on: none. P5 step 1 (the public crate set and the
`publish = false` list) should follow it, so that list is written once.
Decided: none.

## Summary

The workspace goes from 28 crates to 23. Five small crates that exist for
one consumer move into that consumer as a module: `suspect-watch`,
`suspect-reverse` and `suspect-upgrade` into `suspect-cli`, `suspect-faker`
into `suspect-ir`, and `suspect-rex` into `suspect-arazzo`. Users see no
change: no command, flag, output or exit code moves. The code sees five
fewer manifests, five fewer `[workspace.dependencies]` entries to bump on
release, and an internal crate table that matches what is actually shared.

## Problem

1. **The short version's consumer claim is half right.** Measured with
   `cargo metadata --no-deps` (normal dependencies only):

   | Crate | `src` lines | Normal-dependency consumers |
   |---|---|---|
   | `suspect-watch` | 265 | `suspect-cli` |
   | `suspect-faker` | 341 | `suspect-gen`, `suspect-test` |
   | `suspect-reverse` | 361 | `suspect-cli` |
   | `suspect-upgrade` | 675 | `suspect-cli` |
   | `suspect-rex` | 1144 | `suspect-test`, `suspect-lsp` (declared, unused) |

   The line counts in the short version are correct. `suspect-faker` is not
   single-consumer: `crates/suspect-gen/src/filters.rs:481` calls
   `suspect_faker::value`, and `crates/suspect-test/src/lib.rs:46`
   re-exports it as `pub use suspect_faker as faker` for
   `crates/suspect-test/src/fuzz.rs:202`.

2. **The short version missed `suspect-rex`.** `crates/suspect-lsp/Cargo.toml:32`
   declares `suspect-rex.workspace = true`, but `grep -rn suspect_rex
   crates/suspect-lsp/` finds no use in source. Its only real consumer is
   `suspect-test` (`exec.rs`, `plan.rs`, `lib.rs`, `tests.rs`).

3. **Two runtime-expression parsers in two crates.** `suspect_arazzo::expr`
   (`crates/suspect-arazzo/src/expr.rs`, 397 lines, Arazzo §4.3 grammar
   including `$components.*` and embedded `{$expr}`) and `suspect-rex`
   (`crates/suspect-rex/src/lib.rs`, grammar subset, rejects `$url`, has an
   evaluator). Both are used by `suspect-test`. They live in different crates,
   so nobody reading one sees the other.

4. **Three example-value synthesizers.** `suspect_faker::value`,
   `suspect_gen::filters::synth_example` (`filters.rs:417`, which calls the
   faker for scalars) and `suspect_gateway::mock::synth_example`
   (`crates/suspect-gateway/src/mock.rs:79`, which does not). They disagree
   on priority: the faker uses `default` before `example`
   (`crates/suspect-faker/src/lib.rs:9`); the gateway mock uses `example`
   before `default` (`mock.rs:91-96`). The only crate all three depend on is
   `suspect-ir`.

5. **Manifest drift in the small crates.** `suspect-reverse` uses
   `suspect-ir.path = "../suspect-ir"` instead of the workspace entry and has
   no `rust-version.workspace` (`crates/suspect-reverse/Cargo.toml:1-9`).
   `suspect-upgrade` lists `suspect-low`, `suspect-overlay` and
   `suspect-source` as normal dependencies, but uses them only inside
   `#[cfg(test)] mod tests` (`crates/suspect-upgrade/src/lib.rs:514-582`).
   Every extra crate is one more manifest to keep consistent, and one more
   version line in the root `Cargo.toml` (`Cargo.toml:32-42`) to bump on each
   release (P5).

6. **Other single-consumer crates.** `suspect-gateway`, `suspect-lsp` and
   `suspect-rules` are also consumed only by `suspect-cli`. They are not
   small and have their own reasons to stay (see Design). No other crate has
   a single consumer.

## Goals and non-goals

Goals:

- Each remaining crate either has two or more consumers, is a published
  library (P5), or is a large component with its own tests and benches.
- No behaviour change. Every test that passes today passes after each step,
  moved with its code.
- Fewer internal version lines for P5's release bump.

Non-goals:

- Unifying the two expression parsers or the three example synthesizers.
  Both change behaviour (the grammars and priorities differ) and need their
  own proposal. This proposal only puts each pair in one crate, so that
  follow-up is a local refactor.
- Splitting `suspect-codegen` (173,489 `src` lines, 284 files). It is the
  long pole of the build, but splitting it is a separate design.
- Renaming crates that stay.

## Design

### Per-crate decision

| Crate | Decision | Target | Graph effect |
|---|---|---|---|
| `suspect-watch` | merge | `suspect_cli::watch` (`crates/suspect-cli/src/watch/{mod.rs,tests.rs}`) | `notify` becomes a direct dependency of `suspect-cli`; no new edges |
| `suspect-reverse` | merge | `suspect_cli::reverse` (`crates/suspect-cli/src/reverse.rs`) | `regex` becomes a direct dependency of `suspect-cli` (already in the graph via `suspect-lint`); no new edges |
| `suspect-upgrade` | merge | `suspect_cli::upgrade` (`crates/suspect-cli/src/upgrade.rs`) | none; its only real dependency is `serde_json` |
| `suspect-faker` | merge | `suspect_ir::faker` (`crates/suspect-ir/src/faker.rs`) | none; `suspect-gen` and `suspect-test` already depend on `suspect-ir`, which already depends on `serde_json` |
| `suspect-rex` | merge | `suspect_arazzo::rex` (`crates/suspect-arazzo/src/rex.rs`) | `suspect-arazzo` gains `serde`; `suspect-test` already depends on `suspect-arazzo`; `suspect-lsp` drops an unused edge |
| `suspect-runtime` | keep | | two consumers; it exists so the gateway and the executor share one conformance decision (`crates/suspect-runtime/src/lib.rs:1-16`) |
| `suspect-artifact` | keep | | two consumers; confined writing via `cap-std` is a security boundary and has its own `tests/artifact_safety.rs` in both consumers |
| `suspect-rules` | keep | | single consumer today, but P1's check pass calls it from the LSP too |
| `suspect-gateway` | keep | | 3,945 lines, benches, its own test module |
| `suspect-lsp` | keep | | 35,701 lines, benches |

Three choices differ from the obvious "move it into the caller":

- **Faker goes to `suspect-ir`, not `suspect-gen` or `suspect-test`.**
  `suspect-test` does not depend on `suspect-gen`, so either of those would
  add an edge; `suspect-ir` is already a dependency of the faker's two
  consumers and of the gateway, which holds the third synthesizer.
- **Rex goes to `suspect-arazzo`, not `suspect-test`.** Both targets add no
  edges, but `suspect-arazzo` already holds the other expression parser, so
  the duplication becomes visible in one crate.
- **Watch, reverse and upgrade become `pub` modules of the `suspect-cli`
  library, not private `commands/` code.** The examples
  `crates/suspect-cli/examples/platform_bench.rs:662`,
  `analysis_demo.rs:79` and `feature_bench.rs:144` call their APIs, and
  examples see only the library's public items. `crates/suspect-cli/src/lib.rs`
  already exposes `pub mod commands`, `pub mod silence` and others.

### Module layout after the change

```text
crates/suspect-cli/src/
  watch/mod.rs        was suspect-watch/src/lib.rs   (WatcherHandle, DebounceEvent, watch, normalize, is_watched)
  watch/tests.rs      was suspect-watch/src/tests.rs
  reverse.rs          was suspect-reverse/src/lib.rs (ExtractedRoute, ReverseReport, extract_routes, extract_from_tree, cross_reference)
  upgrade.rs          was suspect-upgrade/src/lib.rs (upgrade)
  commands/watch.rs   unchanged glue; `use crate::watch::{self as watcher, DebounceEvent};`
  commands/reverse.rs unchanged glue; calls `crate::reverse::...`
  commands/upgrade.rs unchanged glue; calls `crate::upgrade::upgrade`
crates/suspect-ir/src/faker.rs      was suspect-faker/src/lib.rs (value)
crates/suspect-arazzo/src/rex.rs    was suspect-rex/src/lib.rs   (Rex, RexCtx, parse_rex, ...)
```

Call-site changes (all mechanical):

```rust
// crates/suspect-cli/src/lib.rs (new lines)
pub mod reverse;
pub mod upgrade;
pub mod watch;

// crates/suspect-cli/src/commands/watch.rs:10
-use suspect_watch::{self as watcher, DebounceEvent};
+use crate::watch::{self as watcher, DebounceEvent};

// crates/suspect-gen/src/filters.rs:481
-            suspect_faker::value(&schema, name)
+            suspect_ir::faker::value(&schema, name)

// crates/suspect-test/src/lib.rs:46 (keeps the public path `suspect_test::faker`)
-pub use suspect_faker as faker;
+pub use suspect_ir::faker;

// crates/suspect-test/src/{exec,plan,lib,tests}.rs
-use suspect_rex::{...};
+use suspect_arazzo::rex::{...};
```

`suspect-ir` gains `pub mod faker;` next to `pub mod fast;`
(`crates/suspect-ir/src/lib.rs:23-26`). `suspect-arazzo` gains `pub mod rex;`
and `serde.workspace = true`. Both crates use `#![deny(missing_docs)]` where
the source crate did; the moved items keep their doc comments, so no new
docs are needed.

Tests: each crate's `#[cfg(test)] mod tests` moves with its file. The test-only
uses in `suspect-upgrade` (`suspect_low::LowDoc`, `suspect_overlay::Value`,
`suspect_source::Source`) resolve because `suspect-cli` already depends on
all three.

### Compile graph

Measured unit times (`cargo build --timings`, debug, run alongside another
build so absolute numbers are inflated): `suspect-watch` 2.0 s, `suspect-reverse`
2.5 s, `suspect-upgrade` 1.2 s, `suspect-rex` 0.7 s, `suspect-faker` 0.6 s,
against `suspect-ir` 17.8 s and `notify` 24.3 s. The effect is small either
way:

- Clean builds: five fewer rustc invocations and metadata files. These
  leaves compile in parallel with `suspect-codegen`, so wall-clock time for a
  clean `cargo build -p suspect-cli` does not change measurably.
- Incremental builds: an edit to watch, reverse or upgrade already rebuilt
  `suspect-cli`; now it rebuilds only `suspect-cli`. An edit to the faker
  now rebuilds `suspect-ir` and its twelve dependents instead of
  `suspect-gen`, `suspect-test` and their dependents. The faker has one
  commit in its history (`git log -- crates/suspect-faker`: `0456673`), so
  this cost is rare.
- `cargo test -p suspect-watch` and the like stop existing; the tests run
  under `cargo test -p suspect-cli` / `-p suspect-ir` / `-p suspect-arazzo`.

### Tie to P5

P5 recommends a publishable foundation set (`suspect-source`,
`suspect-syntax`, `suspect-low`, `suspect-ref`, `suspect-jsonpath`,
`suspect-oas`, `suspect-overlay`) and `publish = false` on everything else.
None of the merge targets here is in that set, so no published API changes.
`suspect-arazzo` and `suspect-ir` stay `publish = false`; if either is
published later, `rex` and `faker` become part of its API, which is the
right home for them. Each merge removes one line from
`[workspace.dependencies]` that P5's version bump would otherwise edit.

## Compatibility and migration

- No user-visible change. The `suspect` binary, its commands and the LSP
  are byte-for-byte the same in behaviour.
- No crate in this list has been published, so no downstream Rust user
  breaks. The re-export `suspect_test::faker` stays.
- README's crate table (`README.md:120-127`) drops five rows.
  `docs/LOAD-TESTING-PLAN.md:367` mentions `suspect-rex`; it is a plan
  document (P6 moves it to `docs/internal/`) and is left as written.

## Implementation plan

Each step is one PR, independent of the others, and leaves `cargo test
--workspace`, clippy and `cargo doc` green.

1. **Drop the unused `suspect-rex` edge from `suspect-lsp`.** Delete
   `crates/suspect-lsp/Cargo.toml:32`. Test: `cargo build -p suspect-lsp`.
   0.25 h. **Done.**
2. **Merge `suspect-watch` into `suspect_cli::watch`.** Move
   `src/lib.rs` and `src/tests.rs`; add `notify.workspace = true` to
   `suspect-cli`; update `commands/watch.rs` and `examples/platform_bench.rs`;
   delete the crate and its `Cargo.toml:40` line; update README. Tests: the
   moved `watch/tests.rs` runs under `cargo test -p suspect-cli --lib`.
   1-2 h. **Done.**
3. **Merge `suspect-reverse` and `suspect-upgrade` into `suspect_cli`.**
   Same pattern; add `regex.workspace = true`; update `commands/reverse.rs`,
   `commands/upgrade.rs`, `examples/analysis_demo.rs`,
   `examples/feature_bench.rs`; delete `Cargo.toml:32,42`. Tests: the moved
   unit tests plus the existing CLI integration tests that run them
   (`crates/suspect-cli/tests/ecosystem_commands.rs:48-94` for `upgrade`,
   `crates/suspect-cli/tests/orphan_wire.rs:222,250` for `reverse`). 2 h. **Done.**
4. **Merge `suspect-faker` into `suspect_ir::faker`.** Update
   `suspect-gen/src/filters.rs:481`, `suspect-test/src/lib.rs:46`, both
   manifests, `Cargo.toml:41`. Tests: the moved faker tests under
   `cargo test -p suspect-ir`; existing `suspect-gen` docs snapshot tests
   and `suspect-test` fuzz tests unchanged. 1-2 h. **Done.**
5. **Merge `suspect-rex` into `suspect_arazzo::rex`.** Move the file; add
   `serde` to `suspect-arazzo`; update the four `suspect-test` files and its
   manifest (`crates/suspect-test/Cargo.toml:17`, which is path-only today);
   delete `Cargo.toml:34`. Tests: the moved rex tests under `cargo test -p
   suspect-arazzo`; the `suspect-test` executor tests unchanged. 2-3 h.

Total: about 1 day. Do it before P5's first tag so the release bump and the
`publish = false` list are written once.

## Testing

- Each step: `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps` (intra-doc links such as `[`suspect_watch`]` in
  `commands/watch.rs:3` must be rewritten to `[`crate::watch`]`).
- Behaviour parity: run `cargo test -p suspect-cli --test cli_reference`
  (the generated `docs/CLI.md` must not change) and the existing
  `platform-smoke` job in `.github/workflows/platform.yml`.
- A count check is not needed; `cargo metadata` is the source of truth.

## Alternatives considered

- **Leave the crates as they are.** Costs nothing now, but every release
  bump (P5) and every manifest convention change touches 28 manifests, and
  the duplicate parsers and synthesizers stay spread out. Lost because the
  merges are about a day of mechanical work.
- **Merge faker into `suspect-gen`.** `suspect-test` would have to depend on
  `suspect-gen` (and through it `suspect-artifact`) for one function. Lost
  on graph cost.
- **Merge rex into `suspect-test`.** Same graph cost as `suspect-arazzo`,
  but leaves the two parsers in different crates. Lost on locality.
- **One `suspect-core` crate for the parse layer** (`source`, `syntax`,
  `low`, `ref`, `jsonpath`). It would remove four more crates, but those are
  the crates P5 proposes to publish, they have five or more consumers each, and
  splitting them lets `suspect-config` and `suspect-rex` build without
  `suspect-ref`. Lost because it costs parallelism and fights P5.
- **Merge `suspect-gateway` and `suspect-rules` into `suspect-cli`.** Both
  have one consumer, but the gateway has benches and its own tests, and P1
  gives `suspect-rules` a second consumer. Lost.

## Risks

- **Intra-doc links and `missing_docs`.** Moving a crate root into a module
  turns `//!` crate docs into module docs and can break `[`suspect_x`]`
  links. CI's `docs` job catches it; fix in the same PR.
- **Faker edits rebuild more.** See Compile graph. Low impact.
- **Conflicts with in-flight work.** Moves conflict with any open branch
  that touches these files. Land each step as a pure move (`git mv` plus
  import edits) so rebases are trivial.

## Open questions

1. **Should `suspect-runtime` (724 lines) merge into `suspect-test`?**
   `suspect-gateway` already depends on `suspect-test`, so it would add no
   edges. Recommended: no. The crate exists so the gateway and the executor
   share one conformance decision, and hiding it inside the executor crate
   undoes that.
2. **Should the follow-up that unifies `suspect_arazzo::expr` with
   `suspect_arazzo::rex`, and the three example synthesizers behind
   `suspect_ir::faker`, be scheduled?** Recommended: yes, as a separate
   proposal after P1, with snapshot tests that pin today's gateway mock
   output first, because the `example`/`default` priority differs.
