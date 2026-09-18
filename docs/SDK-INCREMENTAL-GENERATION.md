# Canonical incremental SDK generation

The M6 session, CLI watch/preview and native compatibility commands share the
owned `Contract` and canonical language backends. Current scope and milestone
acceptance are recorded in [today's goal](SDK-TODAY-M3-M6.md).

## Generate, check or preview

Create a session configuration. `spec` is relative to this configuration file;
package identities are explicit configuration, independent of OpenAPI `info`.

```json
{
  "spec": "api.openapi.yaml",
  "targets": [
    {"backend": "typescript-http", "package_name": "example-sdk", "package_version": "1.0.0"},
    {"backend": "rust-http", "package_name": "example-sdk", "package_version": "1.0.0"},
    {"backend": "python-http", "package_name": "example-sdk", "package_version": "1.0.0", "import_name": "example_sdk"},
    {"backend": "go-http", "package_name": "example.com/example-sdk", "package_version": "1.0.0"},
    {"backend": "swift-http", "package_name": "ExampleSDK", "package_version": "1.0.0"}
  ],
  "operation_ids": ["getCredits"],
  "owner": "example-sdk",
  "cache_entries": 4,
  "cache_bytes": 134217728
}
```

An empty `operation_ids` attempts every outgoing operation. Each selected backend
appears once and owns its language subdirectory. Unknown configuration keys and
unsupported operations produce diagnostics.

```sh
suspect codegen-session --config sdk-session.json --out generated
suspect codegen-session --config sdk-session.json --out generated --check --format json
suspect codegen-session --config sdk-session.json --out generated --preview --format json
suspect codegen-session --config sdk-session.json --out generated --watch --preview --format json
```

The CLI resolves `--out` from the invocation directory. The editor deliberately
resolves its output choice from the configuration directory and passes an
absolute path. `--preview` and `--check` perform read-only ownership/drift checks;
preview includes generated artifact contents. `--watch` retains one process and
checks source/configuration/disk content on a configurable polling interval
(`--interval-ms`, default 250). Stop the CLI with normal process cancellation.

## Library interface

`crates/suspect-codegen/src/generation_session.rs` exposes:

- `Session::new(entry, SessionConfig)` and `set_config(config)`.
- `generate() -> SessionOutput`: one owned contract, complete artifact set,
  source/configuration `revision`, changed artifact paths, newly reached
  documents, per-refresh counters and cumulative counters.
- `write(&output, root)`: ownership-aware changed-file writes with the configured
  stable logical owner and `Adoption::Refuse`.

`SessionConfig` contains `targets: Vec<backend::TargetConfig>`, `operation_ids`,
`owner`, `cache_entries` and `cache_bytes`. `backend::generate` is also usable
directly with an immutable `Arc<Contract>` and selected operation source IDs.

## Snapshot and invalidation rules

- SHA-256 fingerprints cover the bytes actually loaded into every document in
  the canonical reference closure. A subsequent read verifies those fingerprints
  before artifacts enter the cache. Missing or concurrently changed inputs fail
  explicitly.
- Failed semantic file acquisitions are tracked as absent dependencies. Creating
  a missing example/reference file invalidates the cached fallback. Successfully
  loaded documents with unresolved pointers are fingerprinted too. Unreadable or
  oversized failed inputs prevent caching until their state can be established;
  literal `$ref`-shaped instance data does not become an acquisition dependency.
- Retrieval paths are lexical absolute file identities. Entry symlinks preserve
  the requested URI's relative-reference base.
- One accepted input snapshot has one shared `Arc<Contract>`. Every selected
  language receives it directly. Backend plans/rendered artifacts are reused on
  unchanged snapshots: zero additional compiles or renders.
- Configuration-only changes reuse the contract and reuse unaffected target
  artifacts. Changing one package identity replans that target; removing a target
  requires no render of the remaining targets.
- Source, annotation or example edits invalidate the relevant complete input
  snapshot. Newly introduced references are loaded during the fresh compilation.
  Native executable-equivalence and changed-file gates check the artifact result.
- Reverting to a retained snapshot reuses its exact contract/artifacts. Its
  content revision still differs from the intervening generation, so watch
  consumers receive the reverted preview even when disk drift paths are equal.
- The LRU is bounded by entries and retained source/artifact bytes. Graph overhead
  is additionally bounded by entries. Oversized snapshots can generate but are
  not retained. Previously returned caller-owned snapshots remain immutable.
- A failed refresh publishes no candidate artifacts and preserves prior accepted
  cached snapshots. Recovery can reuse an earlier successful snapshot.

The cache is in-process. Generator code and embedded runtime/template assets are
immutable for that process; applying their changes requires rebuilding and
starting the new executable. Cache data is not reused across executable versions.
Compatibility reports separately record generator/runtime asset provenance.

## Writes and protocol

The complete desired artifact set goes through the shared ownership writer.
Unchanged files keep their metadata. Obsolete, unedited owned files are removed;
user edits and other owners remain conflicts. `changed_paths` in the library is
relative to the previous accepted generation; `changedArtifacts` in CLI records
is the current ownership-aware disk drift.

JSON watch output is newline-delimited, with format `suspect.sdk.session.v1`:

```json
{
  "format": "suspect.sdk.session.v1",
  "generation": 2,
  "success": false,
  "status": "drift",
  "revision": "<source/configuration SHA-256>",
  "source": "/workspace/api.openapi.yaml",
  "output": "/workspace/generated",
  "config": "/workspace/sdk-session.json",
  "changedArtifacts": ["python/src/example_sdk/models.py"],
  "newDocuments": [],
  "delta": {"compiles": 0, "renders": 0, "cache_hits": 1},
  "stats": {"compiles": 2, "renders": 8, "cache_hits": 1},
  "diagnostics": []
}
```

Statuses are `current`, `written`, `drift`, `write-conflict` or `planning-error`.
Preview records additionally carry `artifacts: [{path, content}]`; planning errors
omit artifacts and preserve source-linked backend diagnostic codes/locations.
Watch emits changes and recovery, suppressing identical idle records. A finite
generation returns 0 on success and 1 for drift, conflicts or planning failure.

The editor provides preview, drift check, watch, latest-preview and stop commands.
It displays read-only virtual diffs, bounds protocol input, watches existing files,
and invalidates obsolete/error previews. See [editor integration](SDK-EDITOR-PREVIEW.md).

## Native and wire compatibility

Compare two session configurations, each pointing at its own source snapshot:

```sh
suspect codegen-compare --before baseline/sdk-session.json --after candidate/sdk-session.json --format json
suspect codegen-compare --before baseline/sdk-session.json --after candidate/sdk-session.json
```

JSON preserves separate wire/native deltas, before/after symbols, original source
locations, uncertainty, migration guidance and generator/runtime provenance. Text
output is Markdown migration notes. Exit 0 means proved compatible within the
reported scope, 1 means breaking/potentially-breaking/unknown findings, and 2 means
input/configuration/comparison failure. `targets: []` explicitly requests a wire-only
comparison. Selected operation removal remains a report when the ID exists on
either side. Changing selections compares the actual selected surfaces.

The library also supports retaining `CompatibilitySnapshot`s and comparing them
without replanning. Comparison is an explicit operation; ordinary idle watch
refreshes do not repeat native compatibility planning. See
[compatibility report semantics](SDK-COMPATIBILITY-REPORTS.md).

## Verification

Meaningful process/library checks cover unchanged reuse, target-only config
invalidation, external reference edits/deletion, lexical symlinks, cached reverts,
finite eviction, obsolete target removal, read-only previews, ownership conflicts,
source diagnostics, source/native comparison and real editor-helper execution.

```sh
cargo test --locked -p suspect-codegen --test generation_session
cargo test --locked -p suspect-cli --test codegen_session --test codegen_compare
```

Latency/RSS observations and calibrated numerical regression policy are recorded
separately by the session-performance tooling. Functional cache correctness and
zero-rewrite checks do not substitute for a calibrated p95 latency gate.
