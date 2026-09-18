# M3 / M6 exit evidence

**Status: M3 verified; hackathon demo scope accepted.** The user confirmed on
2026-09-10 that the deliverable is generation and the output SDKs. All 208 relevant
criteria pass in `target/sdk-greenfield-native-verified-03/report.json`.
That immutable report retains its original full-profile outcome, 208/212 with
four numerical performance criteria unmet. The completed Mac baseline is
observational after timing qualification failed; no candidate was run.

This document defines the evidence profiles for
[the revised scope](SDK-TODAY-M3-M6.md): Python, Go, Swift and Rust native SDKs,
TS/JS regressions, and M6 sessions/editor/compatibility/performance. The runner's
own isolated checks are not milestone acceptance evidence. Both `releaseReady`
and `sdkReleaseReady` stay `false`, including after this scoped run succeeds.

## Entry point

The registered module and dispatch in `xtask/src/main.rs` are:

```rust
mod sdk_m3_m6;
// In the existing task match:
"sdk-m3-m6" => sdk_m3_m6::run(rest),
```

The API is `pub(super) fn run(raw: &[String]) -> anyhow::Result<()>`.
`Args::parse` requires `--source` and `--out`. **`--demo`** selects the explicit
208-gate `hackathon-demo` profile. Without it, `full-calibrated` retains all 212
gates and accepts an optional `--performance-plan`. The two options cannot be mixed.
Run after Main freezes integration.

```sh
cargo run --locked -p xtask -- sdk-m3-m6 \
  --source /Users/luke/github/openrouter-web \
  --out target/sdk-demo-verified-NEW --demo
```

`--out` must be new, with an existing parent under this workspace's `target/`.
Execution scratch is reserved separately under the caller's `${TMPDIR}/opencode`
as `sdk-<report-name>-<output-path-hash-prefix>.work/`. Its canonical ancestors
must contain neither a Cargo manifest nor Git metadata; an unsafe scratch parent
fails before work begins. Each attempt retains its report and external scratch.
Reports and inventories record `acceptanceProfile`, `numericalPerformanceRequired`
and the exact required stages. Demo reports list the four numerical stages as
deferred, while native/functional failures still block acceptance.

For **full-calibrated** acceptance, add
`--performance-plan /absolute/plan.json` when usable numerical evidence is available. A missing,
unreadable, malformed or incorrectly pinned plan records all four required
performance rows as **unmet** and continues native, editor and quality checks.
M3 can complete independently; full-profile M6 and that command remain incomplete
(nonzero exit) until all required performance evidence is met.

## Provenance and completion rules

- **Actual implementation snapshot:** the runner captures every tracked and
  nonignored untracked file, including staged/unstaged contents and deleted-path
  records. `source-manifest.json` records SHA-256, byte count, file kind,
  permissions and relative symlink targets. HEAD, NUL-delimited status/index
  records and both binary Git patches are additional provenance. The complete
  `source/` snapshot contains the uncommitted implementation; HEAD alone is not
  a reconstruction recipe. Nonrelocatable source symlinks fail explicitly.
- **Execution copy:** builds and tests use a byte-verified external `{work}/source/`
  copy. Build directories, generated packages and native temporary roots are
  outside the repository. Extracted Cargo packages therefore discover their own
  workspace instead of an outer Suspect manifest. No scratch Git repository or
  invented commit is created. The editor has its own copy. Original source and
  snapshot hashes are checked again at the end.
- **Frozen CLI:** a fresh CLI is built from that execution copy into a private
  Cargo target directory, copied to `bin/suspect` and hashed. The five-target
  session flow and real editor helper use that executable. Existing CLI process
  tests embed `CARGO_BIN_EXE_suspect`; the runner installs the same frozen bytes
  at that private compiled path before executing those harnesses and verifies
  the identity before broad workspace checks.
- **Commands:** `checks/<id>.json` and stdout/stderr logs are create-once.
  Commands retain resolved executable, executable hash, argument array, cwd,
  exact cleared-and-rebuilt environment, exit code, elapsed time and log hashes.
  Native package build/install/type/doc/archive commands have their own rows.
  Consumer suites retain named harness results and their source snapshots;
  successful subprocess output that an existing test discards is not presented
  as a separate native subprocess transcript.
- **Tools/configuration:** version/config commands cover both runtime tiers,
  Rust compiler/Cargo/Rustdoc executable paths, Python build/type/doc packages,
  pinned npm toolchains, Swift and Chromium. DocC identity uses its actual binary
  digest and supported `--help` interface, alongside selected Swift toolchain
  metadata; no unsupported `docc --version` probe is used. SDK identity comes from
  the selected SDK's actual `SDKSettings.json`, copied and hashed under
  `tool-metadata/`, including its own `Version` field. Installed Python tooling and
  npm compiler/runtime package files are hashed. Applicable home configuration
  presence/digests are recorded without copying their contents. The report
  includes generation configuration, lockfiles and generated package sources;
  wheel, npm and Rust archives are retained under `native-artifacts/`.
- **Criterion versus exit:** `success` preserves raw exit-zero status.
  `criterionMet` additionally evaluates the named requirement. For example,
  the expected readonly drift preview exits 1 and can satisfy its criterion.
  Missing executables, timeouts, absent suites, zero-test suites, filtered tests,
  ignored native tests, Node skips, incomplete performance reports and missing
  measurements cannot satisfy required gates. Native harnesses include ignored
  tests and use one test thread. The shared Swift library gate uses `--show-output`
  to require its exact named passing test as well as a complete suite result.
- **Broad versus focused results:** the default broad workspace run retains its
  normal ignored-test accounting. Required M3 native suites are separately
  executed with no omissions. A broad green result cannot substitute for them.
- **Immutable result:** final `report.json` enumerates required identities and
  unmet gates even if an earlier phase aborted. `seal.json` inventories evidence
  bytes and symlink targets, `seal.sha256` anchors that inventory, and the report
  tree becomes readonly. Completion requires a complete report **and a verified
  seal**. An interrupted directory or an unsealed report is incomplete evidence.
  The external work/cache tree is separate from the immutable evidence tree;
  `invocation.json.workDirectory` is its authoritative location.

The four tracked OpenRouter files are read from the original checkout, verified
with Git and the exact hashes below, then consumed from private copies. The
original `/Users/luke/github/openrouter-web` remains read-only.

| Input | Tracked relative path | SHA-256 |
| --- | --- | --- |
| Public | `projects/docs/openapi/openapi.yaml` | `bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821` |
| Management | `openrouter-management.openapi.yaml` | `c1ed00af257808f00c2a070b28ace135954f9cc1f97111a48364a35eae0505f2` |
| Provider | `projects/docs/assets/provider-monitor-schema-v2.openapi.json` | `5e8d14ed38af861e2c0d4827367d7f00d1a55722a5f62199e821ecc84241a355` |
| Temporal | `packages/temporal/benchmarks.openapi.json` | `e0fea9dc837eb24a3d82e8d5334fffd154d83cf22c44782b9651a36416b54ea7` |

### Independent source validation

The current greenfield runner uses the actual source/tool/input snapshots as its
prerequisites. Full-document OpenRouter validation remains a separate command:
the public `exclusiveMinimum` error and management validation/lint findings keep
their real failure outcomes there. Native SDK success for selected operations
does not establish full-corpus acceptance; this report records
`currentFullCorpusAcceptance: null`.

## Runtime selection

Defaults are explicit and recorded in `invocation.json` and each command. An
override must name a real installed tool satisfying the recorded version gate.

| Purpose | Selector / value |
| --- | --- |
| Python floor | `SUSPECT_PYTHON_FLOOR_BIN=/Users/luke/.local/share/uv/python/cpython-3.11-macos-aarch64-none/bin/python3.11` |
| Python current | `SUSPECT_PYTHON_CURRENT_BIN=/opt/homebrew/opt/python@3.14/bin/python3.14` |
| Python build/type/docs tools | `SUSPECT_PYTHON_TOOLS=$PWD/target/sdk-native-python-tools/bin/python` |
| Go floor / current | Stage-local `GOTOOLCHAIN=go1.23.12` / `local`, together with the matching `SUSPECT_GO_TOOLCHAIN` |
| Rust floor / current | Stage-local `RUSTUP_TOOLCHAIN=1.88.0` / `stable`, together with `SUSPECT_NATIVE_RUST_TOOLCHAIN` |
| Node 22 | `SUSPECT_DOCS_NODE=/Users/luke/.local/share/mise/installs/node/22.23.1/bin/node` |
| Node 24 | `SUSPECT_NODE24_BIN=/Users/luke/.local/share/mise/installs/node/24.21.0/bin/node` |
| Swift current 6.3.3 | Actual implementations resolved by `xcrun --find swift` / `swiftc`; `SUSPECT_SWIFT_BIN` / `SUSPECT_SWIFTC_BIN` may select another installed implementation |
| Swift current SDK | Automatically resolved by `xcrun --sdk macosx --show-sdk-path` for the selected toolchain; optional explicit `SUSPECT_SWIFT_SDKROOT` override |
| Swift 6.0.3 floor | `SUSPECT_SWIFT_FLOOR_ROOT` selects the extracted payload described below; `SUSPECT_SWIFT_FLOOR_BIN` can override its driver |
| Swift floor SDK | `SUSPECT_SWIFT_FLOOR_SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk` |
| Browser regression | `SUSPECT_CHROMIUM=/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` |

The default Swift floor payload is
`${TMPDIR%/}/opencode/swift-6.0.3-toolchain/expanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload`.
It is resolved from the **caller's** TMPDIR before child tmp paths are replaced.
Its `usr/bin/swift`, `swiftc` and `docc` supply the floor. Driver overrides default
to sibling compiler/DocC executables; `SUSPECT_SWIFTC_FLOOR_BIN` and
`SUSPECT_SWIFT_FLOOR_DOCC_BIN` override those individually. Current-tier SDK/DocC
overrides are `SUSPECT_SWIFT_SDKROOT` and `SUSPECT_SWIFT_DOCC_BIN`.

System `/usr/bin/swift` and `/usr/bin/swiftc` selectors are resolved to the actual
Xcode toolchain before use. `SWIFT_EXEC` must name that implementation: SwiftPM
locates `swift-symbolgraph-extract` beside it. The runner hashes that companion
and refuses an incomplete toolchain. Current DocC is likewise resolved with
`xcrun --find docc`, or uses the explicit implementation override.
`tool-selection.json` retains the resolved selectors and their discovery results;
`invocation.json` retains the originally requested selectors.

Current SDK discovery sets `DEVELOPER_DIR` to the developer directory containing
the selected Swift compiler. Driver/compiler directories must agree, and an
automatically discovered SDK must belong to that developer directory. The
`tool-swift-current-sdk-path` row retains the retrieval argv, environment, result
and logs. `tool-swift-current-sdk` snapshots the actual SDK settings/version.
The verified default is currently Xcode's `MacOSX26.5.sdk`, with metadata version
**26.5**; the floor remains independently selected SDK **15.4**. An explicit
SDK override is validated as a macOS SDK and recorded as an override rather than
attributed to automatic discovery.

Both tiers set the actual harness selectors `SUSPECT_SWIFT_BIN`,
`SUSPECT_SWIFTC_BIN`, `SUSPECT_SWIFT_SDKROOT`, `SDKROOT`, and matching DocC.
The pinned `{swift-sdk}` path is supplied to every current standalone compiler,
SwiftPM integration/library harness and direct package build. No SDK environment
variable is needed for the default current tier.
Each uses new external `{work}/swift/<tier>/` gate, tmp, module-cache and package-cache
paths. Direct package builds pass `SWIFT_EXEC`, `--sdk`, and
explicit scratch/cache paths. The floor frontend and SDK settings are hashed.
The independently verified pairings are documented in [SDK-SWIFT.md](SDK-SWIFT.md).

The Python tooling venv must already contain build 1.4.0, hatchling 1.29.0,
httpx 0.28.1, mypy 1.19.1 and Sphinx 8.2.3. A compatibility link in the execution
copy supports existing helpers that locate this venv relative to their crate.
It does not install packages into the user's Python interpreter. Tools are
hashed after preparation and checked for mutation at the end.

### One-time cache/component preparation

Prepare the pinned caches before freezing a new acceptance attempt. Main verified
fresh offline installs after filling the docs/editor caches and installed the
official Rust 1.88 Clippy component. Reproduce setup as needed:

```sh
export PATH="/Users/luke/.local/share/mise/installs/node/22.23.1/bin:$PATH"
rustup component add --toolchain 1.88.0 clippy
for directory in crates/suspect-codegen/tools/typescript-docs crates/suspect-codegen/tools/typescript-floor crates/suspect-codegen/tools/typescript-http-bundle editors/vscode; do
  npm --prefix "$directory" ci --ignore-scripts --no-audit --no-fund
done
```

The acceptance runner still installs offline and treats failures as unmet gates.
If preparation fails, `tool-config-inputs` names the failing install checks and
their stderr logs; missing inventory directories include their exact paths.

Rust harnesses are **built with stable**, then executed directly under the
native tier environment. This ensures that even an older helper which calls
bare `cargo` really uses Rust 1.88 in the floor run. Recompiling the host harness
on each tier is unnecessary. TS package/TypeDoc gates use the pinned Node 22,
npm 10.9.8, TypeScript 5.5.4/5.9.3 and TypeDoc toolchains. Shared TS runtime
vectors additionally execute with Node 24 on `PATH`.
Swift also builds the **entire codegen library test harness** with stable and
executes it under each Swift tier with `--include-ignored --show-output`.
The `swift_sdk::validation::tests::native_shared_runtime_contract_vectors` result
must be present and passing; zero ignored/filtered tests are required.

## Mandatory evidence matrix

Every listed family is required; exact expanded identities and argv are saved
in `report.json.requiredStages`. Missing future integration suites are failures.
The same code supplies a no-execution inventory, including source/provenance
checks and the required performance slots:

```sh
cargo run --locked -p xtask -- sdk-m3-m6 --list-stages --demo
cargo run --locked -p xtask -- sdk-m3-m6 --list-stages
```

Its `stageCount`, `milestoneCounts` and `stages` come from the same requirement
selection used by execution. Demo acceptance contains **208 unique gates**:
**205 M3** and **49 M6 iteration** memberships, including **46 shared gates**.
The full calibrated inventory contains **212 unique gates**: **205 M3** and
**53 M6** memberships, including **46 shared gates**. These counts include all
source/provenance checks and four mandatory performance slots; they are not
counts of passing tests or completed acceptance. The milestone counts overlap
where a shared gate serves both M3 and M6.

| Stage family | Required coverage |
| --- | --- |
| Source/input/tool integrity | Full dirty-tree snapshot, tracked four-input hashes, tool/config fingerprints and final integrity |
| `library-*` | `generation_session`, `sdk_compatibility`, `artifact_safety`, `http_admission`, `sdk_examples`, `sdk_workflow_contract` |
| `cli-*` | Process suites `codegen_session`, `codegen_compare`, `sdk`, `rust_sdk`, `generation_ownership`, `entry_loading`; same frozen CLI bytes |
| `five-op-*` | Frozen CLI readonly preview, generation and drift check for all five targets from one retained configuration |
| `package-*` | Python wheel/source mypy and floor/current venv install/import/dependency checks; Go floor/current build/vet/doc/module inventory; Rust floor/current models/HTTP/all-features/docs/archive; TS install/build/archive; Swift floor/current native package builds |
| Python floor/current | `python_json`, `python_validation`, `python_models`, `python_codecs`, `python_http`, `python_runtime_regressions` |
| Go floor/current | `go_json`, `go_validation`, `go_models`, `go_codecs`, `go_http`, `go_http_names`, `go_runtime_regressions` |
| `floor/current-wave_a_openrouter` | Installed/native Python and Go consumers call all five actual operations against independent HTTP fixtures |
| `floor/current-m3_native_docs` | Source-bound native symbols, installed samples, Sphinx link/inert-prose checks and real native type/doc coverage |
| `names-python_http_names` | Python native name/argument/status collision regressions; Go naming regressions run in both Go tiers |
| Rust floor/current | `rust_json`, `rust_validation`, `rust_contract`, `rust_codecs`, `rust_codecs_json`, `rust_http`, `rust_http_runtime`, `rust_http_openrouter`, `rust_http_security`, `rust_http_docs`, `rust_serde` |
| Representative M2 floor/current | `m2_vertical`, `sdk_example_packages` with the original shared contract, independent consumers, negative types and native docs |
| `floor/current-swift_sdk` | Swift 6.0.3 + SDK 15.4 and 6.3.3 integration suites: original M2 fixture, five actual operations, negative types and matching DocC |
| `floor/current-swift-shared-runtime` | Full library suites with the specifically required shared native validation test on both Swift tiers |
| TS/JS native regression | `typescript_contract`, `typescript_http`, `typescript_http_runtime`, `typescript_http_docs`, `typescript_http_bundle`, `typescript_package`, `typescript_m2_toolchains`, `typescript_directional_package`, `typescript_http_directional`, `typescript_codecs_docs`, `m2_browser` |
| `node22/node24-*` | `typescript_json`, `typescript_validation`, `typescript_codecs_runtime`; Rust/Python/Go validation suites also execute `runtime-contract-v1.json` |
| Editor | Private `npm ci`, extension compile, real CLI generation/session helpers including `generation-native-profiles.cjs`, and session/UI protocol tests; complete TAP result with zero skips |
| Performance | Small, split-recursive and five-op OpenRouter cold/warm/source-change/config-change measurements, plus versioned regression comparison |
| Workspace quality | `cargo test --workspace --locked`; all-target Clippy with `-D warnings`; warnings-denied workspace Rustdoc; `cargo fmt --all -- --check`; unstaged and staged `git diff --check` |

The five actual operation selectors are `getCredits`, `createKeys`,
`updateKeys`, `listContainerFiles` and `getContainerFile`. The package build rows
prove packaging/tool execution; the named native consumer suites prove wire and
type behavior. `cargo package --no-verify` is an archive check alongside the
separate required native compilation and consumer gates.

## Main-wired performance plan

The five-backend v2 API is documented in
[SDK-SESSION-PERFORMANCE.md](SDK-SESSION-PERFORMANCE.md). `run.py` records observed
measurements; functional-only runs are `not-measured`. `calibrate.py` qualification
requires a reserved, identity-pinned host, at least five baseline and three
independent candidate suite processes, 200 measured cycles and five warmups per
fixture/group process, plus the v2 confidence, drift and repeatability checks.
The completed five-suite Mac baseline did not qualify because of timing noise.
Under full acceptance, omitting the plan preserves that unmet result while
collecting other evidence. Demo acceptance does not require this numerical plan.

The numerical plan/adapter remains Main-wired. It must preserve the original v2
reports and real `compare --gate` verdict. An observational exit-zero comparison
cannot complete full-profile numerical M6. Performance commands have a six-hour deadline; other
commands retain their 30-minute limit.

Plan schema:

```json
{
  "format": "suspect.sdk.m3-m6.performance-plan.v1",
  "inputs": [
    { "path": "/absolute/reviewed-baseline.json", "sha256": "<reviewed lowercase SHA-256>" }
  ],
  "stages": [
    {
      "id": "performance-small",
      "program": "cargo",
      "args": ["<actual v2 collector/adapter argv>"],
      "cwd": "{workspace}",
      "environment": {},
      "report": "{out}/performance/small.json",
      "assertions": { "/format": "<actual versioned format>", "/complete": true },
      "claims": {
        "complete": "/complete",
        "measurementStatus": "/performance_status",
        "warmCompiles": "/summary/warm/compiles",
        "warmRenders": "/summary/warm/renders",
        "warmWrites": "/summary/warm/writes",
        "coldSamples": "/summary/cold/samples",
        "warmSamples": "/summary/warm/samples",
        "sourceChangeSamples": "/summary/sourceChange/samples",
        "configChangeSamples": "/summary/configChange/samples"
      }
    }
  ]
}
```

This shows one stage's shape, not a runnable complete plan. Supply **exactly**
`performance-small`, `performance-split-recursive`, `performance-openrouter`
and `performance-compare`, with comparison last. All baseline/config inputs
outside the source snapshot need absolute paths and verified hashes. Their
bytes and the plan itself are retained under the report.

The runner expands `{workspace}` to the actual execution snapshot, `{work}` to
scratch space and `{out}` to the immutable-evidence directory. Runtime tokens
such as `{python-floor}`, `{python-current}`, `{node22}` and `{node24}` are also
available. Commands use argument arrays and a fixed cwd, with no shell expansion.
The environment includes `SUSPECT_M3_M6_SOURCE_SHA256` and
`SUSPECT_M3_M6_BINARY_SHA256` for binding tool output to this invocation.

Each command must create its previously nonexistent report under
`{out}/performance/`. `assertions` map RFC 6901 JSON pointers to exact expected
values, including the actual format. The `claims` pointers map the performance
tool's schema to mandatory obligations:

- Collection: `complete` is true and `measurementStatus` is `observational`;
  `warmCompiles`, `warmRenders`, `warmWrites` are exactly zero; all four scenario
  sample counts are positive integers. Untimed placeholders do not qualify.
- Comparison: `complete` is true; `regressions` is zero or an empty array;
  `comparedCases` is an integer at least three; `gated` is true and `verdict` is
  `passed`. Map the latter claims to the real v2 comparison's `gated` and `status`
  fields. Its assertions must select the reviewed versioned budgets and complete
  fixture/metric comparison policy.
- A skip/missing-tool marker, absent pointer, nonzero command exit or changed
  baseline/report digest fails the gate.

Main must bind these pointers to actual measured aggregates and the comparator's
real verdict. The v2 raw suites/collections do not have this example's normalized
summary shape. A source-controlled adapter may normalize them, retaining their
raw outputs and preserving every failed measurement/comparison. The collector
requires its own pinned source/scratch paths and tracked public provenance;
private corpus byte copies alone do not satisfy its Git provenance checks.
The execution copy has no Git metadata. Direct v2 `run.py` expects Git attribution,
so the bridge must use the retained original revision/source manifest faithfully.
It must not synthesize a commit or substitute a clean HEAD checkout. `{work}` and
`{workspace}` now point outside the repository; derive them from the supplied
tokens or `invocation.json.workDirectory`, never by appending `.work` to `--out`.

## Isolation/tool regression evidence

The first full attempt remains sealed at
`target/sdk-greenfield-native-verified-01/report.json`; its failed outcomes and
old work tree are retained. The subsequent targeted Cargo 1.88 fixture first
reproduced outer-workspace inheritance, then passed standalone checking,
packaging, extraction and example execution with the external layout and no
manifest repair. Its passing logs are under
`${TMPDIR}/opencode/sdk-isolation-proof-khYqRc/` (the red control is `U04hjK`).
The tiny current-Swift symbol-graph probe and both DocC help/identity probes pass
under `${TMPDIR}/opencode/sdk-swift-tool-proof-FuNIBj/`.

The second full run is sealed at
`target/sdk-greenfield-native-verified-02/report.json`, with 205 of its 210 gates
met and source integrity passing. Its one non-performance failure exposed the
missing explicit SDK in standalone Swift typechecking. The strengthened tiny
probe reproduced that failure at `${TMPDIR}/opencode/sdk-swift-tool-proof-EYpwKR/`.
With automatic discovery and no caller SDK override, standalone Foundation/
standard-library typechecking and symbol-graph extraction both pass at
`${TMPDIR}/opencode/sdk-swift-tool-proof-09cPF3/`. Its current SDK metadata is
`macosx26.5` / version `26.5`. These are targeted runner regressions, not another
full native acceptance run or numerical qualification.

## Preserved full-profile performance follow-up

1. All five backends and CLI routing are integrated. The Swift facade prefixes
   artifacts with `swift/`; the frozen session still discovers its real manifest.
   Final evidence must include both Swift tiers and the shared library-vector gates.
2. `m3_native_docs`, the naming suites, both runtime-regression suites and the new
   editor profiles test are mandatory. The Python names helper currently uses
   the tooling interpreter; its standalone row is not labelled a Python 3.14
   execution. The floor/current JSON/models/codecs/HTTP and five-op rows select
   the consumer interpreter explicitly.
3. Qualified numerical evidence still needs the reserved host, real v2
   baseline/candidate collections and the manual plan
   adapter with pinned inputs. Until then, required M6 performance gates stay unmet.
4. After Main freezes integration, run the matrix and verify the final seal.
   Promote only the individual milestones whose required evidence is complete.
   Keep attempts intact and choose a new output name for another attempt.
