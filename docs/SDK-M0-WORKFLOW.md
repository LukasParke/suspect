# Historical M0–M2 workflow measurements

The archived M0–M2 run measured TS/JS and Rust over one owned `Arc<Contract>`.
Its `suspect-sdk-milestone-bench-v1` reports preserve source/executable/config
identity, timing samples, file/byte counts, unchanged file stamps, change diffs
and native tool logs. These are observational historical baselines. Current
five-profile measurement and calibration use [SDK-SESSION-PERFORMANCE.md](SDK-SESSION-PERFORMANCE.md).

The recorded run covered three fixture classes:

1. Shared create/update/list/get M2 contract.
2. `tests/fixtures/m2/split.openapi.yaml` plus its external recursive schema,
   request body, response and examples.
3. The tracked public OpenRouter snapshot, selecting `getCredits`, `createKeys`,
   `updateKeys`, `listContainerFiles` and `getContainerFile` explicitly.

## Measured boundaries

- Entry read/lossless parse; reference closure, exact value materialization and
  owned contract construction.
- TS HTTP planning (including model, codec/program, examples and eager language
  lowering), then package/documentation assembly. Rust HTTP planning, then
  native package/code/documentation rendering. These are real API boundaries;
  internal combined work is not reported as fictitious separate measurements.
- Initial owned write, unchanged ownership comparison/write and changed-snapshot
  generation/comparison/write. Byte content, mtimes and inodes prove zero
  unchanged rewrites, including the ownership manifest on no-change runs.
- Native TS build, pack, install, import/startup, exact JSON codec loop and Node
  peak RSS/heap. Native Rust build, pack, install, installed JSON codec loop,
  process wall time and macOS peak RSS. Toolchain work is separate from generation.

Cold samples each create a fresh compiler/workspace; OS caches are not flushed.
Both targets share that sample's single contract. Warm samples reuse the last
owned contract. The report includes schema-node/reference-edge counts so the
recursive fixture's bounded graph/artifact size stays observable.

Each change scenario uses private copies of the exact dependency closure:
an operation description edit, one reachable object-schema edit, and one
operation's response-contract edit. External schema edits address the copied
external file, preserving reference bases. Absolute references escaping the
private closure fail the scenario. Original input hashes are rechecked afterward.

The controlled docs-only edit must update the operation's documentation and
preserve executable syntax after lexically removing genuine doc comments.
Quoted/raw/multiline strings remain intact; ambiguous syntax conservatively
requires native verification. HTTP manifests may change only that operation's
description fields, and example manifests may update source byte ranges. Other
manifest data and unrelated artifacts must stay unchanged. Regression vectors
reject altered methods/package metadata and comment-shaped literal changes.
This is not a general language-equivalence or compatibility algorithm. Semantic
edits must reach both native outputs; unchanged artifacts retain their stamps.

These reports predate persistent sessions and do not establish a sustained p95
budget. Current session behavior and native/runtime verification are recorded
in [SDK-PROGRESS.md](SDK-PROGRESS.md).

## Recorded baseline (2026-09-09)

All three measured classes pass in
`target/sdk-m0-m2-verified/workflow-{small,split-recursive,openrouter}/report.json`.
Three cold samples were taken with the debug generator; these medians are
observational, and generation excludes subsequent native toolchain invocations.

| Class | Entry parse | Closure + owned contract | TS plan + package | Rust plan + package | Unchanged rewrites |
| --- | ---: | ---: | ---: | ---: | ---: |
| Shared small slice | 0.60 ms | 9.24 ms | 10.04 ms | 7.47 ms | 0 |
| Split recursive | 0.14 ms | 2.46 ms | 1.77 ms | 1.13 ms | 0 |
| Public OpenRouter / five operations | 89.63 ms | 2,203.28 ms | 41.11 ms | 34.56 ms | 0 |

The public owned graph has 7,192 schema nodes and 1,798 direct reference edges;
the recursive split fixture has six schema nodes and three reference edges.
The same snapshots drive both targets. Per-file counts, bytes, native timings,
source/configuration/executable hashes and command logs are in the reports.
`target/sdk-m0-m2-post-review-verified/report.json` records six passing checks
after the final description-propagation assertion was added to the comparator.
