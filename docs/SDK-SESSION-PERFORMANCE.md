# SDK Session performance

**Hackathon disposition, 2026-09-10:** the user confirmed that the deliverable is
generation and the output SDKs. Latency remains observational and is non-blocking
for the explicitly named `hackathon-demo` acceptance profile. The original
calibrated profile remains strict; its failed qualification is preserved.

The replacement Mac baseline completed five suites and **48,000 measured
refreshes in 3 hours 27 minutes**. All work/size stability checks passed, but
all 144 timing dimensions failed at least one noise/confidence/drift condition.
An independent calculation reproduced all 677 overlapping qualification reasons;
these are not SDK regressions, and no candidate ran. The launchd job is removed.
Evidence: `target/session-baseline-mac-v2-restart01/` and
`target/sdk-calibration-restart01/analysis-diagnosis.md`.
The procedures below remain available for a future prospectively controlled run.

## Full-plan timing controls (2026-09-10)

The resumed full-plan work adds **prospective attribution and workload-order
controls**, with the original numerical thresholds preserved:

- `run.py` and `calibrate.py` accept `--attribution`. The native benchmark records
  process user/system CPU time, block IO operations, page faults, context switches
  and lifetime process peak RSS immediately outside each timed phase. Missing or
  inconsistent counters fail validation. These counters do not identify a thermal
  cause, physical IO bytes, or a per-refresh peak.
- `--schedule fixed` pins each scenario to one position in every cycle. The default
  `rotating` retains the historical schedule. The schedules have different workload
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
  --input target/session-baseline-mac-v2-restart01/collection.json \
  --out target/NEW-diagnosis.json
```

Use `--metric refresh_ms --bootstrap-replicates 1000 --block-length 5` to request
the explicitly labeled exploratory interval. Comparing different block lengths
is a sensitivity analysis, not a way to choose a passing confidence bound.

The v2 harness covers all **five canonical HTTP backends: TypeScript, Rust,
Python, Go and Swift**, through the persistent `Session` API. Functional checks
can run without timing. Performance reports remain **observational** until a
separate collector/qualifier verifies repeated execution, pinned runner identity,
sample counts, quantile confidence, stationarity and repeatability.

This implements the measurement/regression seam from
[the generation plan](SDK-GENERATION-PLAN.md#speed-measure-the-complete-workflow).
The shipped policy is the plan's **candidate** “investigate >10% beyond measured
noise” policy. It is not an approved absolute interactive-latency budget or an M6
completion claim.

## Run it

From the repository root, with Rust/Cargo, a C compiler and Python 3.10+.
While integration is changing, use a focused **untimed** check:

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

Once Main's integration edits settle, collect observations explicitly:

```sh
python3 tools/sdk-session-perf/run.py \
  --out target/sdk-session-five-observation \
  --suite public --openrouter-root /path/to/openrouter-web \
  --iterations 5 --warmups 1 --offline
```

`compact` runs the small M2 vertical and split-recursive M2 closure. `public`
adds the **tracked** `projects/docs/openapi/openapi.yaml`, selecting `getCredits`,
`createKeys`, `updateKeys`, `listContainerFiles`, and `getContainerFile`. This
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
The new five-target v2 protocol is incompatible with archived v1 reports. The
four-backend observations below and `policy-v1.json` remain historical evidence;
they cannot qualify the expanded workload.

## Collect and qualify a sustained p95 baseline

`calibrate.py` runs the complete protocol; `policy-v2.json` defines its explicit
candidate criteria. There is no `--calibrated` switch and no shipped qualified
baseline. The user has explicitly reserved **this Mac after greenfield cleanup,
builds and reviews finish**. `runners/luke-mac-v2.json` records that decision and
pins observed hardware/power/tool identity. Qualification remains pending real
measurements; Main must explicitly declare ready before collection starts.

### Exact sampling protocol

| Requirement | Baseline | Candidate |
| --- | --- | --- |
| Independent suite processes | At least **5** | At least **3** |
| Measured cycles in each fixture/target process | At least **200** | Same protocol, at least **200** |
| Warmup cycles per fixture/target process | At least **5**, retained separately | Same |
| Required fixtures | M2, split-recursive, tracked public five operations | Identical |
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

The plan's initial >10%-beyond-noise policy is implemented as:

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

### Main's commands, after integration settles on a reserved, pinned host

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
automatically. The policy remains a **candidate policy**, and absolute M6
interactive budgets still require agreement based on actual qualified evidence.

## CI collection and current functional evidence

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
  label is not provisioned here; the user-reserved Mac can run the same CLI
  protocol locally once Main declares ready.

`calibrate` runs the collector for five complete 200-cycle suites with five
warmups and `--require-qualified`; it uploads `sdk-session-baseline`, including
`baseline.json`, `policy.json` and all raw collection evidence. It needs no prior
baseline. `compare` requires the explicit baseline run ID/artifact, collects
three independent suites and invokes `--gate` under the artifact's policy.
Workspace/job concurrency is serialized and the long collection has a 360-minute
limit. Missing artifacts, unsuitable identity, insufficient evidence, uncertainty
and noise fail rather than falling back to a passing smoke result. Compact-only
collections remain observational under the default policy's public-input requirement.

On 2026-09-09 the focused five-backend split-recursive check completed in
`target/sdk-session-five-functional-02/report.json`: eight untimed cold/warm/edit/
revert checks plus target removal/restoration and Swift module rename/restoration.
Cold used one compile/five renders; warm/reverts used zero compiles/renders;
target removal reused four target caches; the module rename rendered Swift once
and reused the other four. Fresh-output, byte/metadata stability and obsolete-file
checks passed. The v2 raw-report validator accepted this report. Its performance
status is **not-measured**; it adds no observations to the historical table.

The 21 collector/comparator tests pass, including actual CLI exit codes, exact
binomial coverage, uncertain tails, within-run drift, collection overlap/reuse,
missing identity pins, Swift module-cache counters and preservation of failed
attempts. Collector tests use explicitly synthetic subprocesses/reports; their
qualified fixture results are not evidence about any real runner. Local runner
inventory was exercised and remained observational. Full five-target measurements
await Main's integration settling and actual runner qualification.

Canonical native package build/import/codec/HTTP tests remain separate workloads.
This harness does not infer their performance from Session timings. It does not
yet establish absolute interactive budgets, per-phase compiler internals,
cache-retained heap/peak RSS, recursion growth bounds or full M6 exit evidence.

## Reserved Mac readiness and planning estimate

User decision, 2026-09-09: **“Calibrate this Mac” after builds/reviews finish**.
Reservation file: `tools/sdk-session-perf/runners/luke-mac-v2.json`.
The paired `runners/luke-mac-inventory-20260909.json` is an observational record:

- macOS **26.6.2 / 25G83**, Darwin 25.6.0, Apple M5 Pro, **18 physical/logical
  CPUs**, **64 GiB** RAM;
- ordinary macOS scheduling; explicit CPU affinity is unavailable (`null`);
- **AC Power**, observed `powermode 0`, unchanged power settings. No privileged
  settings or calibration were started by the inventory probe;
- Rust/Cargo 1.97.1, Python 3.14.7, Apple Git 2.50.1 and Apple Clang 21.0.0.
  Rust/Cargo are resolved independently through rustup. Both `/usr/bin` Xcode
  dispatchers and the actual selected Git/Clang executable bytes are pinned;
- actual machine/power identity SHA-256
  `6fe3b27d006b8fc3cec896331405c89baed9d1839976d3d04fc778dc14bc0033`;
  tool identity SHA-256
  `ec60636a689cc32d5af44d722cd0745faf4069dbe8ab0c67bc530d719fbfaaee`.

Recommended protocol remains **5 baseline + 3 candidate suites, 200 measured
cycles and 5 warmups per case**. The previous four-target data sums to about
9.327 s across all eight scenario medians and six cases: approximately 31.1 min
per 200-cycle suite, or **4.15 hours for 5+3 suites**, counting timed intervals
only. The existing five-target focused debug process completed in about **2.40 s**
including setup/oracles/functional probes; it is not a throughput benchmark and
does not predict the public-input tail. Allow roughly **5–7 hours total** as a
planning window for added Swift/docs work, verification I/O and variation. This
is an estimate, not new five-target timing evidence or an absolute budget.

After Main freezes the final greenfield tree and explicitly declares ready:

```sh
# Optional nonprivileged awake assertion; no pmset/governor configuration change.
/usr/bin/caffeinate -is python3 tools/sdk-session-perf/calibrate.py \
  --role baseline --out target/session-baseline-mac-v2 \
  --suite public --openrouter-root /Users/luke/github/openrouter-web \
  --runner tools/sdk-session-perf/runners/luke-mac-v2.json \
  --repeats 5 --iterations 200 --warmups 5 --offline --require-qualified

/usr/bin/caffeinate -is python3 tools/sdk-session-perf/calibrate.py \
  --role candidate --out target/session-candidate-mac-v2 \
  --baseline target/session-baseline-mac-v2/baseline.json \
  --policy target/session-baseline-mac-v2/policy.json \
  --suite public --openrouter-root /Users/luke/github/openrouter-web \
  --runner tools/sdk-session-perf/runners/luke-mac-v2.json \
  --repeats 3 --iterations 200 --warmups 5 --offline --gate
```

Keep the same original checkout/scratch paths and final source/tool/config/input
bytes through both collections and acceptance. New output directories preserve
each attempt. Source drift, tool/power pin drift, wide confidence intervals,
noise or a regression remain failures; reservation does not bypass any check.

## Original-workspace evidence bridge for final acceptance

`tools/sdk-session-perf/accept.py` addresses xtask's **unborn private Git tree and
private corpus copies**. Collection happens in the frozen original workspace,
where real Git/input provenance and fixed private measurement paths are valid.
The adapter subsequently executes from xtask's byte-verified source copy. It
performs read-only verification and imports evidence; it never runs measurements
inside the acceptance build/test workload.

Main supplies a reviewed, hash-pinned manifest with this exact shape (all paths
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
3. Match xtask's `source-manifest.json` to `SUSPECT_M3_M6_SOURCE_SHA256`, using
   xtask's serialization order, and verify the separate frozen acceptance CLI
   against `SUSPECT_M3_M6_BINARY_SHA256`. The CLI hash is a binding to this
   acceptance invocation, not a claim that it is the benchmark executable.
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
Each stage creates its own previously nonexistent normalized report. xtask's
final seal covers these reports and copied evidence. Existing target reports and
historical observations remain unchanged; no legacy/M0 failed-stage lineage is
required by this bridge.

After Main records the real pin-manifest hash, produce the complete four-stage
plan (this validates numerical pins/verdict; actual acceptance-file binding is
checked again when the stages run):

```sh
python3 tools/sdk-session-perf/accept.py plan \
  --evidence /absolute/performance-inputs.json \
  --evidence-sha256 "$REVIEWED_PERFORMANCE_INPUTS_SHA256" \
  --corpus /Users/luke/github/openrouter-web \
  --out target/sdk-m3-m6-performance-plan.json
```

The generated plan uses `suspect.sdk.m3-m6.performance-plan.v1`, exact stage IDs
`performance-small`, `performance-split-recursive`, `performance-openrouter`,
`performance-compare` (last), the measured Python executable, and
`cwd: {workspace}`. It pins all consumed baseline/config/report/binary/prepared
input files. Main supplies this plan to its current `sdk-m3-m6` entrypoint; Main
owns changes to xtask acceptance requirements during greenfield cleanup.

The individual command shape, already emitted by `plan`, is:

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

## Historical four-backend observations (v1, immutable)

The following evidence is the original TS/Rust/Python/Go result, including its
original API/tooling verification. It is not re-labelled as five-target or v2
evidence. The raw reports, 240 observations and v1 policy remain unchanged.

On 2026-09-09, `target/sdk-session-perf-observed-04/report.json` completed all six
fixture/target combinations on a shared Apple M5 Pro development host, using
Rust 1.97.1 and a release binary. **240 measured refreshes**, 48 separate warmup
refreshes and 12 configuration probes passed the functional checks. All 120
measured warm/revert refreshes performed zero compiles/renders, with zero
redundant rewrites. All changed outputs matched fresh-session oracles and disk.

These are **median refresh milliseconds from five samples per series**, not
calibrated p95 evidence:

| Fixture / targets | Cold | Warm | Schema edit | Operation edit | Docs edit | Baseline artifact bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| M2 / TypeScript | 109.29 | 7.26 | 32.46 | 77.00 | 24.73 | 372,975 |
| M2 / all four | 382.66 | 23.84 | 110.74 | 237.62 | 89.59 | 1,223,604 |
| Split-recursive / TypeScript | 119.72 | 6.12 | 37.84 | 39.79 | 29.43 | 156,235 |
| Split-recursive / all four | 411.71 | 41.43 | 154.91 | 159.18 | 99.85 | 516,455 |
| Public five / TypeScript | 679.65 | 17.49 | 612.56 | 642.57 | 595.70 | 985,512 |
| Public five / all four | 861.71 | 45.56 | 651.13 | 708.78 | 608.19 | 3,503,385 |

The raw report also retains per-phase allocation counts/requested bytes, every
individual latency sample, per-file artifact hashes and all source/tool/config
identities. Median warm allocated bytes for the all-target M2, split and public
cases were 3,117,942, 1,676,220 and 9,553,730 respectively; these are allocator
requests, not retained memory. RSS is explicitly unavailable on this macOS run.

Native benchmark binary SHA-256:
`2ec09d54f973c9dbac4305f9a5115c34f65a1767d788f8ef2869149ba6a34bf1`.
Generator/build source-manifest SHA-256:
`a2195c36352d2c232eadd99825e7bd430e1f328b9d8cbb3ea66369ffaf9f2c72`.
The public input was tracked-HEAD at
`db378a2a90d0167b9dca4f98b52074c54d249e1f`, 1,432,512 bytes, SHA-256
`bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821`.

Targeted example type-check/Clippy and workflow YAML parsing succeeded. Clippy
reported library warnings outside this benchmark; no example warnings were
reported. The comparator's 12 adversarial tests pass, including real CLI exit
codes, noisy/ineligible/incompatible evidence, false release-profile claims,
stale-output counters and regression injection. The synthetic test numbers are
not included in the measurement table.

The baseline CLI accepted that real report and wrote
`target/sdk-session-perf-observational-baseline.json` with status `observational`
and `gating_eligible: false`. An actual `compare --gate` invocation refused it
with exit 2 / `ineligible`; `target/sdk-session-perf-gate-refusal.json` retains
the dedicated-runner, sample-count and independent-run reasons. This verifies
the measured-report/comparator boundary without promoting smoke-size samples.

Later attribution-enhanced attempts `sdk-session-perf-observed-05` and `-06`
stopped before measurement because the shared source tree changed during their
builds; their `failure.json` files preserve that outcome. They contribute no
timing evidence. Rustup tool resolution was checked separately against the
actual Rust/Cargo executable paths and SHA-256 hashes. Re-run the final suite
when integration edits have settled; `-04` is the complete stable-build result
cited above.

The host is not a calibrated dedicated runner. A numerical gate, measured
absolute budgets and Swift Session performance remain unestablished.
