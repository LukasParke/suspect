# SDK Session performance

The v2 harness in `tools/sdk-session-perf` covers all **five canonical HTTP
backends: TypeScript, Rust, Python, Go and Swift**, through the persistent `Session` API. Functional checks
can run without timing. Performance reports remain **observational** until a
separate collector/qualifier verifies repeated execution, pinned runner identity,
sample counts, quantile confidence, stationarity and repeatability.

This implements the measurement/regression seam for the complete generation
workflow; the numbers are observations, not gates
([ADR 0033](adr/0033-performance-numbers-are-observations.md)). The shipped
policy is a **candidate** “investigate >10% beyond measured noise” policy. It
is not an approved absolute interactive-latency budget.

## Timing attribution and schedule controls

The harness provides **prospective attribution and workload-order controls**;
they do not change the numerical thresholds:

- `run.py` and `calibrate.py` accept `--attribution`. The native benchmark records
  process user/system CPU time, block IO operations, page faults, context switches
  and lifetime process peak RSS immediately outside each timed phase. Missing or
  inconsistent counters fail validation. These counters do not identify a thermal
  cause, physical IO bytes, or a per-refresh peak.
- `--schedule fixed` pins each scenario to one position in every cycle. The default
  `rotating` schedule rotates the first edit. The schedules have different workload
  identities and cannot be compared as equivalent baseline/candidate evidence.
- Native samples record actual cycle position, ordinal and elapsed observation
  time. Validation checks those fields against the declared schedule.
- `diagnose.py` reads complete suites/collections without modifying them and shows
  process strata, position strata, pooled quantiles and serial correlations.
  Optional cluster/circular-block bootstrap intervals remain exploratory; their
  nominal coverage is not established and they cannot qualify a baseline.

For a new controlled collection, add `--attribution --schedule fixed` to the
normal `run.py` invocation. These options do not change qualification thresholds.
The attribution smoke is a tool check, not numerical acceptance evidence.

```sh
python3 tools/sdk-session-perf/diagnose.py \
  --input target/<baseline-run>/collection.json \
  --out target/NEW-diagnosis.json
```

Use `--metric refresh_ms --bootstrap-replicates 1000 --block-length 5` to request
the explicitly labeled exploratory interval. Comparing different block lengths
is a sensitivity analysis, not a way to choose a passing confidence bound.

## Run it

From the repository root, with Rust/Cargo, a C compiler and Python 3.10+.
For a focused **untimed** check:

```sh
cargo build --locked --offline -p suspect-codegen --example sdk_session_bench \
  --target-dir target/sdk-session-perf-build

target/sdk-session-perf-build/debug/examples/sdk_session_bench \
  --spec crates/suspect-codegen/tests/fixtures/m2/split.openapi.yaml \
  --fixture split-recursive --out target/sdk-session-five-functional \
  --functional-only
```

`--functional-only` executes one complete checked cycle and configuration probes,
with status `not-measured`; zero-valued timing/allocation fields are placeholders,
not samples of zero-cost execution. The suite wrapper accepts the same flag and
`--fixture split-recursive --group all` for an attributed focused check.

To collect observations:

```sh
python3 tools/sdk-session-perf/run.py \
  --out target/sdk-session-five-observation \
  --suite public --openrouter-root /path/to/openrouter-web \
  --iterations 5 --warmups 1 --offline
```

`compact` runs the small `m2-small` fixture and the `split-recursive` closure.
`public` adds the **tracked** `projects/docs/openapi/openapi.yaml`, selecting
`getCredits`, `createKeys`, `updateKeys`, `listContainerFiles`, and `getContainerFile`. This
uses the canonical representative contract and tracked public HTTP workload. The provider-only,
management and Temporal documents, and unselected public operations, are not
represented as passing this HTTP workload. An unavailable/untracked public
input fails explicitly. `OPENROUTER_WEB_ROOT` supplies the same root option.

Each fixture runs as TypeScript alone and as all **five** profiles. To exercise
individual targets, repeat `--group`, for example `--group swift-http --group all`.
`all` is an explicit versioned set, not an automatic expansion of the backend
enum.

Swift's `TargetConfig` is `backend: SwiftHttp`, `package_name: "BenchmarkSDK"`,
`package_version: "0.0.0"`, `import_name: None`. Its default module is therefore
the legal Swift identifier `BenchmarkSDK`. A second probe sets
`import_name: Some("BenchmarkModule")`. The Swift root is discovered from the
actual `Package.swift` artifact; current output uses root-level `Package.swift`
and `Sources/BenchmarkSDK/`, rather than assuming a `swift/` prefix.

The runner builds the example in the isolated `target/sdk-session-perf-build`,
freezes its executable under the new report directory, and records raw samples,
commands and logs. Add `--offline` to require the existing Cargo dependency
cache. `--profile debug` is useful for development but cannot qualify for a
numerical gate.

The stable scratch path `target/sdk-session-perf-work` is created exclusively and
removed after success. It is retained on failure or with `--keep-work`. A
subsequent run refuses an existing scratch directory. Inspect and remove that
owned tree before retrying; the runner never takes over an unfamiliar directory.
Report directories are also exclusive, and baseline files are never overwritten.

For a custom admitted contract, the low-level executable accepts the same
`--spec`, repeatable `--operation-id`, comma-separated `--targets`, sample counts
and cache budgets:

```sh
cargo run --locked --release -p suspect-codegen --example sdk_session_bench -- \
  --spec crates/suspect-codegen/tests/fixtures/m2/split.openapi.yaml \
  --out target/session-split-direct --fixture split-recursive --iterations 5
```

The direct report contains binary, input and configuration fingerprints, but the
suite runner adds build/source/tool/runner attribution required by the comparator.

## What a sample measures

The harness uses the actual API:

```text
Session::new(entry, SessionConfig { targets, operation_ids, cache_entries,
                                   cache_bytes, owner })
Session::generate() -> SessionOutput { revision, contract, files, changed_paths,
                                      new_documents, delta, stats }
Session::set_config(config)
Session::write(output, directory)
```

1. Discover the local reference closure and copy its exact bytes into a private
   tree, preserving relative paths. Verify that its compiled closure remains
   completely inside that tree.
2. Serialize the copied documents as **canonical JSON in their original
   filenames**. This makes controlled edits independent of YAML formatting and
   avoids counting a format conversion as an edit. Both the untouched original
   bytes and this prepared input form are fingerprinted separately. These cold
   timings apply to the prepared JSON bytes, rather than the original YAML bytes.
3. Build independent fresh-Session output oracles for the baseline and three
   edits, outside measured intervals. The schema probe adds an optional string
   property to a reachable object; on the split fixture it edits the external
   recursive `Node`. The operation probe adds an unused concrete error response.
   The docs probe changes one selected operation description.
4. Each cycle starts a new Session and empty artifact root, measures a cold
   generate/write and an unchanged warm generate/write, then measures each edit
   and its revert. Rotate the first edit by cycle to reduce always-first bias.
   A measured edit has not previously been cached in that cycle. Reverts must
   reuse the original snapshot.
5. Run an additional configuration probe: remove a target (or change package
   version in a single-target run), verify contract reuse and obsolete-file
   removal, then restore the cached configuration. These samples are retained
   separately from the eight latency series.
6. If Swift is selected, rename its module explicitly. Exactly Swift renders;
   every other target reuses its cache. Check the Swift manifest/module paths,
   obsolete-source removal, contract identity and complete cached revert.

The eight series are `cold`, `warm`, `schema-edit`, `schema-revert`,
`operation-edit`, `operation-revert`, `docs-edit`, and `docs-revert`. Every
warmup cycle executes the same checks; its raw samples are retained separately
and excluded from numerical summaries.

| Metric | Exact scope |
| --- | --- |
| `generate.ms` | `Session::generate`; the cold case also includes `Session::new` |
| `write.ms` | Ownership comparison, staging and changed-file writes via `Session::write` |
| `refresh_ms` | Sum of those two intervals; no untimed verification is included |
| `allocation_calls`, `allocated_bytes` | Process-wide Rust `System` allocator requests during those intervals; includes `alloc`, `alloc_zeroed` and full requested `realloc` sizes |
| `artifact_bytes`, `artifact_files` | Complete emitted artifact set, excluding the ownership manifest; each oracle also records per-file SHA-256/bytes |
| `rss_after_bytes` | Untimed Linux `VmRSS`, nullable elsewhere; neither peak RSS nor a gated metric |

Allocation instrumentation is active for all timing samples. Requested bytes are
not live/retained heap, and exclude C/tree-sitter allocations. Cold means a fresh
Session, not a fresh OS cache or process; setup/oracle work can warm allocator
and filesystem state. The API exposes aggregate generation and write seams, so
the harness does not invent parse/normalize/plan/render phase timings.

The default finite cache is four snapshots / 256 MiB. The selected workload must
fit its baseline plus an edited snapshot; a budget too small for the asserted
reuse fails rather than being described as a successful warm measurement.
Fixed-size recursion measurements do not prove asymptotic complexity or bounded
RSS. The recorded node/edge/artifact counts make expansion visible.

## Mandatory functional gates

These gates always run, including ordinary CI smoke and observational runs:

- Exactly one contract compilation per cold/source-edit snapshot, and one
  render per selected target. This checks shared compilation across the target set.
- Zero compiles/renders and one cache hit on unchanged and reverted snapshots;
  the actual `Arc<Contract>` and `Arc<Vec<OutFile>>` identities must be reused.
- Every artifact must equal its independent fresh-session oracle. Every source
  edit must reach each selected backend. No stale warm/edit/revert results.
- `changed_paths` must equal the complete independently computed artifact diff,
  including removals. Disk contents must match every desired file and contain no
  obsolete artifacts. The ownership manifest must exist.
- No byte-identical artifact or ownership manifest may change its modification
  time, inode or change time on Unix. This covers unrelated files during edits
  as well as all files during unchanged refreshes.
- Docs-only edits must propagate while preserving executable code and unrelated
  artifacts. A self-contained TS/Rust syntax projection preserves literals and
  rejects ambiguous syntax. Python compares the
  two source-bound method docstrings, and Go compares the exact source prose
  comment. Remaining Python/Go manifest data and examples must match; source
  range shifts in example diagnostics are the explicitly permitted metadata
  difference. TypeScript-only runs verify their source-bound TypeDoc comments
  and operation manifest; their generic `http.md` need not change.
  Swift compares only the comment block immediately above the source-bound
  method, checks propagation into its DocC operation reference and preserves all
  other Swift code/metadata. Python/Go Sphinx bindings may change only selected
  symbol descriptions; literal prose replacements preserve other RST content.
- Configuration changes reuse the contract; restoring a configuration reuses its
  snapshot. Removing a target reuses each remaining target's artifact cache with
  zero renders and actually removes its owned obsolete files. The raw cache-hit
  counter is one for whole-snapshot reuse, or one per reused target during a new
  configuration assembly.
- The five-target cold/source-edit deltas are `{compiles:1, renders:5,
  cache_hits:0}`. Removing Swift gives `{0,0,4}`; renaming its module gives
  `{0,1,4}`. Warm/reverted complete snapshots give `{0,0,1}`. Module restoration
  must reuse both original Arcs and leave no stale alias-module files.
- Original source bytes remain unchanged, private edits are reverted, and the
  running binary retains its fingerprint.

The Python comparator revalidates required flags, sample completeness, counters,
phase sums, oracle hashes, artifact sizes and complete change sets. Favorable
timings cannot hide missing or failing functional evidence. The regression tests
exercise these rejection paths and CLI exit statuses with **synthetic test
data**, never presented as performance evidence.

## Attribution and compatibility

Suite reports use `suspect-sdk-session-suite-v2`; raw cases use
`suspect-sdk-session-bench-v2`. They preserve:

- SHA-256 manifests of actual generator/build/template/runtime sources,
  including dirty/new files, captured before and after compilation;
- a frozen executable and its hash; exact Rust/Cargo, C compiler, Python and Git
  versions and executable hashes (including configured Rust compiler wrappers);
  relevant build environment flags and parent/user Cargo configuration hashes;
- original/prepared closure bytes/hashes, selected operations, package identities,
  cache budgets, preparation/schedule/measurement versions and all raw samples;
- declared runner ID/image/CPU policy and observed OS/kernel/CPU/memory/affinity,
  hashed machine identity and available Linux governor/boost settings, checked
  again after execution;
- native and suite process IDs/start/finish intervals; claimed sample duration
  cannot exceed its containing process interval;
- tracked-public provenance. `source_revision` is set only when bytes equal that
  repository's HEAD blob. A dirty working tree is not attributed to its commit.

A source change during compilation makes that attempt **unmeasured**. Once the
binary has been frozen against a stable source fingerprint, later source work
does not alter it. Tool/binary/input/harness drift during measurements fails.

Comparison requires exactly matching runner, toolchain, harness, build flags,
target/operation/configuration set, input closure, prepared form, counts and
private source paths. Keep the same checkout/scratch paths on a pinned runner:
source URIs occur in real output, and varying them changes the workload.
Missing/incompatible cases fail closed. The generator source and binary hashes
are **allowed to differ for a candidate**, because changed generator code is the
thing being measured. All calibration runs must use the same source and binary.
The five-target v2 protocol is incompatible with v1 reports and `policy-v1.json`;
v1 evidence cannot qualify the v2 workload.

## Collect and qualify a sustained p95 baseline

`calibrate.py` runs the complete protocol; `policy-v2.json` defines its explicit
candidate criteria. There is no `--calibrated` switch and no shipped qualified
baseline. `runners/` holds versioned runner declarations pinned to an observed
hardware/power/tool identity (`runners/luke-mac-v2.json` is one such
declaration).

### Exact sampling protocol

| Requirement | Baseline | Candidate |
| --- | --- | --- |
| Independent suite processes | At least **5** | At least **3** |
| Measured cycles in each fixture/target process | At least **200** | Same protocol, at least **200** |
| Warmup cycles per fixture/target process | At least **5**, retained separately | Same |
| Required fixtures | `m2-small`, `split-recursive`, tracked public five operations | Identical |
| Required groups | TypeScript alone and **all five** canonical targets | Identical |
| Build | Release; one stable source and binary across repetitions | Release; one stable candidate source and binary |

Each cycle contains all eight scenarios. Thus each scenario has 200 observations
per process (ten observations in its upper 5% tail); the baseline has five
independent estimates and the candidate three. **Do not pool short runs into a
pretend 200-sample run.** Suite processes run sequentially, with a fresh native
process for each fixture/group, and fresh Sessions per cycle. The existing
fixed edit/revert rotation remains part of the workload identity.

The collector retains every attempted command, PID, start/finish interval,
exit status, run ID, report hash and raw report in
`suspect-sdk-session-collection-v2`. It stops on a failed attempt instead of
discarding it and keeping faster runs. The qualifier rejects reused/overlapping
process evidence, mismatched sample counts, mixed binaries/sources, incomplete
collections and missing functional gates. An advisory workspace lock serializes
collectors using the same work root; it does not prove unrelated work is absent.

### Runner identity, not a label

On the intended **reserved** host, inventory its current state:

```sh
python3 tools/sdk-session-perf/run.py \
  --inspect-runner target/session-runner-inventory.json
```

This only writes an **observational inventory**. Copy `runner.example.json` to
the host's versioned runner declaration and explicitly record:

- an inventoried runner ID, immutable image and CPU/affinity/power policy;
- `kind: dedicated` only for a genuinely reserved, controlled host;
- `expected_identity_sha256` equal to that host inventory's `actual_sha256`.

Qualification requires the observed identity to match that pin, including a
stable hashed machine ID, OS/kernel/architecture/CPU/memory, affinity and
available governor/boost settings. Exact tool versions/executable hashes,
source/config fingerprints, input hashes and private paths must also match.
Changing only `kind` cannot qualify even a sufficiently long collection. This
is local identity/repeatability evidence, not external runner attestation.

### Confidence, drift and noise criteria

For **each** fixture × target group × scenario × metric, compute nearest-rank
p95 separately per process. A two-sided, distribution-free order-statistic
interval uses `B ~ Binomial(n, .95)`: choose the tight ranks `L,U` with
`P(B < L) <= .025` and `P(B >= U) <= .025`, giving `[X_(L), X_(U)]`.
Unbounded endpoints cannot qualify. At 200 samples the ranks are **184 and
197**, with discrete coverage approximately **96.715%** (at least 95%).

Every baseline **and candidate** process must pass:

1. **Precision:** the larger distance from sample p95 to either confidence bound
   is at most `max(absolute_floor, maximum_confidence_relative * run_p95)`.
2. **Within-run drift:** the absolute difference between medians of the first
   and second chronological halves is at most
   `max(absolute_floor, maximum_drift_relative * run_p95)`.
3. **Between-process repeatability:** let `R` be the median of the process p95s
   and `N` their maximum absolute deviation from `R`. Require
   `N <= max(absolute_floor, maximum_noise_relative * R)`.

The relative precision/drift/noise caps are 5% for latency, 2% for allocation
metrics, and 0% for artifact bytes, with these absolute floors:

| Metric | Absolute floor |
| --- | ---: |
| Total refresh | 0.5 ms |
| Generate / write | 0.25 ms each |
| Allocation requests | 100 |
| Requested allocation bytes | 16 KiB |
| Artifact bytes | 1 KiB |

Intervals assume sufficiently independent, stationary cycle samples. The
repeatability/drift screens add evidence but do not prove independence.
Confidence is **per series**, conditional on the observed versioned reference;
it is not a 95% simultaneous/familywise guarantee for the entire matrix.

The initial >10%-beyond-noise policy is implemented as:

```text
R = median(baseline process p95s)
N = max(abs(baseline process p95 - R))
limit = R + 0.10 * R + max(absolute_floor, 2 * N)
```

After both collections qualify, compare each candidate process's confidence
interval with that **fixed baseline limit**:

- `passed`: every candidate upper bound is within its corresponding limit;
- `regressed`: for at least one series, **every** candidate lower bound exceeds
  the limit;
- `inconclusive`: neither condition is established; this is non-passing.

This avoids turning a point estimate that straddles the threshold into a pass.
Raw point estimates and `would_regress` remain visible. Fix interference or
collect a new explicitly versioned protocol to improve precision; retain prior
attempts and do not tune tolerances against the candidate being judged.

### Commands on a reserved, pinned host

Collect a baseline on the reserved, pinned host:

```sh
python3 tools/sdk-session-perf/calibrate.py \
  --role baseline --out target/session-baseline-v2 \
  --suite public --openrouter-root /pinned/openrouter-web \
  --runner /pinned/session-runner-v2.json \
  --repeats 5 --iterations 200 --warmups 5 --offline --require-qualified
```

The collector writes `collection.json`, `policy.json`, `baseline.json`, all raw
suite reports, logs and frozen binaries. `--require-qualified` exits 2 when the
evidence remains observational; it never promotes it by request. The
`suspect-sdk-session-baseline-v2` artifact embeds all calibration reports and
process evidence. Its limits, confidence and qualification are recomputed when
loaded, rejecting a manually edited limit or qualification flag.

Collect and compare an independent candidate:

```sh
python3 tools/sdk-session-perf/calibrate.py \
  --role candidate --out target/session-candidate-v2 \
  --baseline target/session-baseline-v2/baseline.json \
  --policy target/session-baseline-v2/policy.json \
  --suite public --openrouter-root /pinned/openrouter-web \
  --runner /pinned/session-runner-v2.json \
  --repeats 3 --iterations 200 --warmups 5 --offline --gate
```

The lower-level replay interfaces are:

```sh
python3 tools/sdk-session-perf/compare.py baseline \
  --collection target/session-baseline-v2/collection.json \
  --policy target/session-baseline-v2/policy.json --out target/replayed-baseline-v2.json
python3 tools/sdk-session-perf/compare.py compare \
  --baseline target/session-baseline-v2/baseline.json \
  --candidate target/session-candidate-v2/collection.json \
  --gate --out target/replayed-comparison-v2.json
```

Loose `--report` inputs can produce only an observational baseline without
collector evidence. A single candidate report cannot satisfy the three-process
gate. A calibration process cannot be reused as a candidate.

| Result | Meaning / exit status |
| --- | --- |
| `not-measured` | Untimed functional checks; rejected as performance evidence |
| Raw `observational` | Actual samples, no numerical qualification claim |
| Baseline `observational` | Missing, shared-host, under-sampled, noisy, drifting or imprecise evidence; reasons retained |
| Baseline `qualified` | Every collection/identity/functional/statistical requirement passed under its stated policy |
| Compare without `--gate`: `observational` | Diagnostic estimates/decisions only; exit 0 |
| Gated `passed` / `regressed` | Confidence-supported numerical decision; exit 0 / 1 |
| `inconclusive`, `invalid`, `incompatible`, `ineligible` | Non-passing uncertainty or unusable evidence; exit 2 |

Qualification and numerical acceptance are distinct. No baseline updates
automatically. The policy remains a **candidate policy**, and absolute
interactive budgets still require agreement based on actual qualified evidence.

## CI collection

`.github/workflows/sdk-session-performance.yml` runs compact **untimed** five-target
functional smoke and collector/comparator tests on Ubuntu 24.04 with Rust
1.97.1/Python 3.12.12. It uploads the report/logs/frozen binary. This job supplies
no p95 verdict and needs no installed native Swift toolchain: it exercises the
Rust canonical Swift emitter; the existing native Swift gates are separate.

`workflow_dispatch` exposes `mode: smoke | calibrate | compare`. Both numerical
modes require:

- `SDK_SESSION_PERF_ENABLED=true` and a reserved self-hosted
  `sdk-session-perf-v1` runner;
- `SDK_SESSION_PERF_RUNNER_SPEC` pointing to its versioned declaration and, for
  the public suite, `SDK_SESSION_PERF_CORPUS_ROOT` pointing to the pinned tracked
  checkout;
- a v2 runner declaration pinned to its observed identity. The self-hosted CI
  label is not provisioned here; a reserved local host can run the same CLI
  protocol.

`calibrate` runs the collector for five complete 200-cycle suites with five
warmups and `--require-qualified`; it uploads `sdk-session-baseline`, including
`baseline.json`, `policy.json` and all raw collection evidence. It needs no prior
baseline. `compare` requires the explicit baseline run ID/artifact, collects
three independent suites and invokes `--gate` under the artifact's policy.
Workspace/job concurrency is serialized and the long collection has a 360-minute
limit. Missing artifacts, unsuitable identity, insufficient evidence, uncertainty
and noise fail rather than falling back to a passing smoke result. Compact-only
collections remain observational under the default policy's public-input requirement.

The collector/comparator tests (`tools/sdk-session-perf/test_*.py`) cover actual
CLI exit codes, exact binomial coverage, uncertain tails, within-run drift,
collection overlap/reuse, missing identity pins, Swift module-cache counters and
preservation of failed attempts. Collector tests use explicitly synthetic
subprocesses/reports; their qualified fixture results are not evidence about any
real runner.

Canonical native package build/import/codec/HTTP tests remain separate workloads.
This harness does not infer their performance from Session timings. It does not
establish absolute interactive budgets, per-phase compiler internals,
cache-retained heap/peak RSS or recursion growth bounds.

## Importing evidence into `sdk-full` acceptance

`tools/sdk-session-perf/accept.py` imports collected evidence into the xtask
acceptance runs. `cargo run --locked -p xtask -- sdk-full --performance-plan
PLAN` takes a `suspect.sdk.full.performance-plan.v1` file whose `evidence` pins
the manifest below by path and SHA-256, and runs the four `accept.py import`
stages itself (`xtask/src/sdk_full.rs`). `accept.py plan` emits the
`suspect.sdk.m3-m6.performance-plan.v1` stage plan consumed by
`sdk-m3-m6 --performance-plan` (`xtask/src/sdk_m3_m6.rs`). Collection
happens in the original workspace, where real Git/input provenance and fixed
private measurement paths are valid; acceptance executes from xtask's
byte-verified source copy. The adapter performs read-only verification and
imports evidence; it never runs measurements inside the acceptance build/test
workload.

The input is a reviewed, hash-pinned manifest with this exact shape (all paths
absolute; substitute the actual measured file hashes):

```json
{
  "format": "suspect-sdk-session-acceptance-inputs-v1",
  "baseline": {"path": "/absolute/session-baseline-mac-v2/baseline.json", "sha256": "<reviewed SHA-256>"},
  "candidate": {"path": "/absolute/session-candidate-mac-v2/collection.json", "sha256": "<reviewed SHA-256>"},
  "comparison": {"path": "/absolute/session-candidate-mac-v2/comparison.json", "sha256": "<reviewed SHA-256>"},
  "policy": {"path": "/absolute/session-baseline-mac-v2/policy.json", "sha256": "<reviewed SHA-256>"},
  "runner": {"path": "/absolute/tools/sdk-session-perf/runners/luke-mac-v2.json", "sha256": "<reviewed SHA-256>"}
}
```

The adapter verifies more than the verdict tag:

1. Hash the exact imported bytes, parse without duplicate/nonfinite JSON, and
   recompute v2 qualification/confidence/comparison from all embedded raw data.
   Require the pinned comparison to equal recomputation exactly, with
   `gated: true`, `status: passed`, zero point/confidence regressions and no
   candidate qualification reasons.
2. Require the baseline **and candidate** source manifests to match the final
   original tree and actual acceptance execution-copy files. Verify the complete
   selected source census, individual bytes, harness contents, source-file kinds,
   permissions and confined symlink targets. This final-tree acceptance adapter
   does not transplant timing evidence from a different implementation.
3. Match xtask's `source-manifest.json` to `--source-sha256` (default
   `SUSPECT_M3_M6_SOURCE_SHA256`), using xtask's serialization order, and verify
   the separate frozen acceptance CLI against `--cli-sha256` (default
   `SUSPECT_M3_M6_BINARY_SHA256`). The CLI hash is a binding to this acceptance
   invocation, not a claim that it is the benchmark executable.
4. Hash every measured frozen benchmark binary; verify actual tool executable
   bytes, versions/queries and Xcode dispatchers; verify actual Cargo config
   files and runner machine/power identity. Reconstruct session configurations
   and actual argv from the canonical fixture/target definitions. Read-only
   verification temporarily restores recorded build selectors, so xtask's
   private `CARGO_TARGET_DIR` is not falsely treated as a measurement setting.
5. Match original fixture/public bytes to xtask's actual source/input copies.
   Verify public provenance against the original tracked checkout, not the
   private copy's Git tag. Rehash the archived normalized input bytes consumed
   by the native benchmark; final collectors preserve them under each suite's
   `prepared-inputs/` before scratch cleanup.

Only then are original raw reports, policy/declaration, frozen benchmark bytes
and prepared input bytes copied into content-addressed, readonly files below
`{out}/performance/raw/`. Existing identical blobs are verified, never overwritten.
Each stage creates its own previously nonexistent normalized report, and
xtask's final seal covers these reports and copied evidence.

`accept.py plan` validates the numerical pins/verdict and emits the four-stage
`sdk-m3-m6` plan (actual acceptance-file binding is checked again when the stages run):

```sh
python3 tools/sdk-session-perf/accept.py plan \
  --evidence /absolute/performance-inputs.json \
  --evidence-sha256 "$REVIEWED_PERFORMANCE_INPUTS_SHA256" \
  --corpus /Users/luke/github/openrouter-web \
  --out target/sdk-m3-m6-performance-plan.json
```

The plan contains exact stage IDs `performance-small`,
`performance-split-recursive`, `performance-openrouter` and
`performance-compare` (last), the measured Python executable, and
`cwd: {workspace}`. It pins all consumed baseline/config/report/binary/prepared
input files. Each stage runs:

```sh
"$MEASURED_PYTHON" "{workspace}/tools/sdk-session-perf/accept.py" import \
  --stage small \
  --evidence /absolute/performance-inputs.json \
  --evidence-sha256 "$REVIEWED_PERFORMANCE_INPUTS_SHA256" \
  --original "{original}" --snapshot "{workspace}" \
  --corpus /Users/luke/github/openrouter-web \
  --acceptance-root "{out}" --out "{out}/performance/small.json"
```

Reports use `suspect-sdk-session-acceptance-v1`. Collection reports expose
`complete`, `qualification: qualified`, `performance_status: observational`,
and measured counts under `/summary/{cold,warm,sourceChange,configChange}/samples`.
`/summary/warm/{compiles,renders,writes}` must be zero; warm writes also require
empty changed paths and the native byte/mtime/inode/ctime stability proof.
Configuration counts include separately timed target-removal/module-rename
probes, and zero/invalid phase values cannot masquerade as measurements.

Comparison reports expose `complete`, `gated`, `verdict`, `regressions`,
`comparedCases` and `comparedFixtureClasses`. The generated plan binds exact
format/policy/verification assertions and all required RFC 6901 claim pointers.
An unusable or changed import exits 2 with an incomplete report; a qualified tag
without matching source/tools/config/input bytes cannot satisfy acceptance.
