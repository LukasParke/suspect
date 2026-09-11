# Measuring SDK session performance

The canonical `Session` exposes per-refresh and cumulative counters through
`SessionOutput::delta` and `SessionOutput::stats`. Source/configuration revisions
and artifact changes accompany those counters, so reuse can be checked through
the same interface used by the CLI and editor.

## Self-contained functional checks

Run from the repository root:

```sh
cargo test --locked -p suspect-codegen --test generation_session
cargo test --locked -p suspect-cli --test codegen_session --test codegen_compare
```

These suites exercise fresh generation, unchanged reuse, source edits, cached
reverts, missing-reference recovery and target configuration changes. They check
artifact identity, unchanged-file metadata, bounded caching and session counters
against independent fixtures.

The workspace's Criterion benchmarks cover parsing, references, schema
evaluation, template rendering and platform stages:

```sh
cargo bench --workspace --locked -- --test
```

The `--test` invocation checks that benchmark workloads execute. Running without
it collects each benchmark's measurements.

## Session measurement boundaries

Measure `Session::generate()` separately for cold creation, unchanged refreshes,
source/schema/documentation edits, configuration-only changes and cached reverts.
Measure `Session::write()` separately from planning so file-system work remains
visible. Include complete source closures and emitted artifact sizes in results.

Provenance includes source spans. A documentation-only edit can legitimately
change serialized byte offsets throughout generated metadata. A semantic
comparison must distinguish those location changes from native interface or wire
changes while continuing to verify source identities and unrelated fields.

Record source closure, compiler/runtime assets, configuration, toolchain and
machine conditions when comparing runs. Keep workload and scenario scheduling
fixed and quantify run-to-run noise before interpreting a latency change.

Unchanged refreshes must reuse the accepted Contract and target artifacts.
Configuration-only changes must reuse unaffected targets; reverts can reuse
bounded cached snapshots. Source edits currently invalidate the relevant whole
snapshot. These are functional guarantees checked independently of timings.

Prepared-template throughput, cold end-to-end generation, warm refresh latency
and emitted-package build/runtime cost are distinct measurements. A successful
test run establishes functional behavior rather than a numerical regression
budget. See the [session contract](SDK-INCREMENTAL-GENERATION.md) for cache,
invalidation and write semantics.
