# Native SDK cost observations

`crates/suspect-codegen/tests/sdk_native_measurements.rs` supplies the maintained
native-cost entrypoint required by [SDK-FULL-EXIT.md](SDK-FULL-EXIT.md#native-cost-integration-contract).
Its ignored `native_build_import_codec_request_costs` test dispatches
`tools/sdk-native-costs/run.py collect`. Explicit execution requires the full
environment and fails on missing inputs, tools, measurements, or native witnesses.

The collector executes **92 run sets**: the twelve targets' 23 maintained
`toolchain_tiers` × `build`, `import`, `codec`, and `request`. Each set is one
repeated-run measurement of that dimension under the versioned `repeated-v1`
methodology (see [What the numbers mean](#what-the-numbers-mean)). TypeScript
and JavaScript share one backend; C++ has one declared tier. The actual target
configuration comes from the runner's JSON. Unknown targets, changed tiers and
duplicate dimensions require an explicit collector update.

## Runner API

The six required environment variables are exactly the integration contract:

| Variable | Value |
| --- | --- |
| `SUSPECT_SDK_FULL_PACKAGES` | Absolute immutable CLI-emitted `generated/` directory |
| `SUSPECT_SDK_FULL_NATIVE_ROOT` | Absolute private installed `native/` consumer directory |
| `SUSPECT_SDK_FULL_TARGETS` | Absolute maintained `configs/targets.json` |
| `SUSPECT_SDK_FULL_SOURCE_SHA256` | Actual frozen source-census fingerprint |
| `SUSPECT_SDK_FULL_BINARY_SHA256` | Actual frozen CLI SHA-256 |
| `SUSPECT_SDK_FULL_MEASUREMENTS` | Absolute **nonexistent** output directory with an existing parent |

`sdk-full` supplies these to the compiled suite and runs it with
`--include-ignored --show-output --test-threads=1`. No dispatcher arguments or
feature changes are needed. The host test requires installed Python 3.11+;
`SUSPECT_PYTHON_CURRENT_BIN` selects its interpreter, otherwise `python3` is used.

The collector consumes the runner's real installed layouts: `floor`/`current`
directories, C++ `declared`, installed Maven snapshots, NuGet packages, npm
packages, gems, Composer vendor trees, extracted Cargo/Go/Swift/Dart packages,
and the `python-floor`/`python-current` wheel venvs beside `native/`.

It reads `full-tool-selection.json` beside `generated/` when supplied. This is
important for floor selectors that the full runner retains in its replacement
table rather than in the suite-wide child environment. Explicit `SUSPECT_*`
overrides use the same names as the full runner:

- Node: `SUSPECT_DOCS_NODE`, `SUSPECT_NODE24_BIN`; the execution workspace's
  locked `typescript-floor`/`typescript-docs` compiler installations.
- Python: `SUSPECT_PYTHON_FLOOR_BIN`, `SUSPECT_PYTHON_CURRENT_BIN`,
  `SUSPECT_PYTHON_TOOLS`; actual installed-wheel interpreters for runtime work.
- Rust: installed `1.88.0`/`stable`, resolved with `rustup which`, then executed
  through the actual Cargo/rustc payloads and matching `RUSTUP_TOOLCHAIN`.
- Go: runner-resolved `go-floor-bin`/`go-current-bin` implementations. Standalone
  fallback finds the already-installed floor payload and current `go` on PATH.
  `GOTOOLCHAIN=local` prevents toolchain acquisition.
- Swift: `SUSPECT_SWIFT[_FLOOR]_BIN`, `SUSPECT_SWIFTC[_FLOOR]_BIN`,
  `SUSPECT_SWIFT[_FLOOR]_SDKROOT`, `SUSPECT_SWIFT_FLOOR_ROOT`. Current default
  implementations are resolved with `xcrun`; SDKSettings versions are checked.
- JVM: `SUSPECT_JAVA_FLOOR_HOME`, `SUSPECT_JAVA_CURRENT_HOME`,
  `SUSPECT_MAVEN_BIN`. Kotlin uses the emitted 2.4.20/1.11.0 dependency pins and
  a private copy of the runner's warm Kotlin Maven repository.
- .NET: `SUSPECT_DOTNET_BIN`; each private consumer pins SDK
  8.0.424/10.0.400 with `global.json` and `rollForward: disable`.
- Ruby: `SUSPECT_RUBY_FLOOR_HOME`, `SUSPECT_RUBY_CURRENT_HOME`, with the actual
  installed SDK gem and matching default gem ABI directory.
- PHP: `SUSPECT_PHP_FLOOR_BIN`, `SUSPECT_PHP_CURRENT_BIN`,
  `SUSPECT_COMPOSER_PHAR`.
- Dart: `SUSPECT_DART_FLOOR_BIN`, `SUSPECT_DART_CURRENT_BIN`.
- C++: `SUSPECT_CPP_CXX`, `SUSPECT_CPP_CMAKE`, and the selected macOS SDK.

Missing tools/cache entries fail explicitly. Commands use offline package-manager
settings, private build/cache/home outputs, and no tool activation or downloads.
Ambient proxy, Python injection, Node options and Java-agent variables are excluded
from native subprocess environments. Every version/setup command is retained.

## What the numbers mean

Every measured phase records a full **`repeated-v1` run set**: by default
**5 retained cold runs, then 2 excluded warmup runs, then a 20-run steady-state
batch**. Each run's `nanoseconds` is an actual monotonic subprocess wall
observation (`time.perf_counter_ns`), measured immediately around
launching/waiting for the native command. Build and import use one iteration
per run; codec performs **16 decode/encode pairs** per run; request performs
**three sequential SDK calls** per run. The run-set order is fixed — cold runs
first, then the excluded warmups, then the steady batch — and preparation is
never part of a measured run. There is no adaptive calibration.

The run-set counts are CLI policy on `collect`:
`--cold-runs N` (retained cold runs per measured phase, ≥ 1),
`--warmups W` (excluded runs before the steady batch, ≥ 0), and
`--steady-runs M` (steady-state runs, ≥ 1). Every row records its declared
`runs` counts, its `methodology` (`repeated-v1`), and a `summary` computed
**only from that row's retained raw steady-state samples**: `min`, `max`,
`median`, `p90`, `p99`, `mean`, and population `stddev` over the raw nanosecond
durations.

Codec and request wall times include process/VM startup, fixture constants,
first-use initialization/JIT work, typed assertions, output, and cleanup.
Dividing one run's wall time by the iteration count yields an amortized
observation of that stated boundary, not an isolated steady-state operation
latency. Build observations include dependency compilation where the native
build requires it; dependency *sources* and compiler/plugin installations are
warm and offline. Setup, dependency resolution, installed-consumer compilation,
and server startup are recorded separately from measured commands.

| Language / maintained tiers | Measured build boundary | Import boundary |
| --- | --- | --- |
| TS/JS: Node 22 + TS 5.5, Node 24 + TS 5.9 | Compile the fresh emitted package and declarations with the selected `tsc` | New Node process loading the installed public ESM entry and its transitive SDK modules |
| Rust: 1.88.0, stable | Release SDK build with `reqwest-rustls`, private empty Cargo target | New linked native consumer process |
| Python: 3.11, 3.14 | Wheel build with that interpreter and warm installed build/hatchling modules | New installed-wheel interpreter importing the package, native models and codecs; bytecode writes disabled |
| Go: 1.23.12, 1.27.1 | Native SDK archive build, private compiler cache | New linked executable including package initialization |
| Swift: 6.0.3/SDK 15.4, 6.3.3/SDK 26.5 | Release SwiftPM module/object build | New linked executable and public model type access |
| Java: JDK 21.0.12.1, 25.0.4.1 | `javac --release 21` over all emitted main sources | New JVM, installed client class and allocated codec holder initialization |
| C#: SDK 8.0.424/net8.0, 10.0.400/net10.0 | Release build of the emitted net8.0 SDK | New selected .NET runtime and consumer assembly load; runtime SDK DLL bytes match the installed NuGet DLL |
| Kotlin: 2.4.20/JDK 21 and JDK 25 | Emitted Maven `compile` lifecycle | New JVM, actual installed jar, public client and codec initialization |
| Ruby: 3.3.12, 4.0.6 | Native `.gem` archive construction | New VM, installed-gem activation and actual `require` |
| PHP: 8.3.32, 8.5.8 | Composer authoritative classmap/autoloader generation over a new package copy | New VM, installed Composer autoloader and public Client/Codecs class loads |
| Dart: 3.9.4, 3.13.3 | AOT compile of a typed public-IO consumer against the fresh package | New AOT linked consumer and public codec type access |
| C++: Apple Clang 21/C++20/libcurl 8.7.1 | Release static SDK library build using exported CMake configuration | New linked native consumer process |

The import phase for linked languages describes consumer startup; lazy codec
initialization occurs when the codec/request phase first uses it. PHP's build
boundary is autoloader construction, and Ruby/Python use their native package
archive builders. These boundaries are deliberately explicit because languages
have different compilation and loading models.

## Native work and independent wire checks

Allocated operation, model, codec, constructor and response symbols come from the
actual package's JSON HTTP/source/native documentation manifests. The collector
does not parse generated source code to recover symbols. Native compiled drivers
refer to public generated model and codec types; Python/Ruby/PHP also check their
real model classes and public APIs.

Codec samples repeatedly decode the unmodified `credits` string from
`crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json`, inspect
the typed `total_credits` and `total_usage` values, and encode the native model.
They require preservation of `100.50000000000000001` and `25.75`, count real
encoded bytes, and fail on a precision loss or unexecuted loop.

Request samples use the SDK's real public `getCredits` operation and native HTTP
adapter: Fetch, reqwest/rustls, httpx, net/http, URLSession, JDK HTTP, .NET HTTP,
Ruby HTTP, PHP CurlTransport, Dart IoTransport, or C++ libcurl. A collector-owned
numeric-loopback server supplies the independent response. It checks:

- Exactly three `GET /api/v1/credits` HTTP/1.1 exchanges to its ephemeral port.
- Exactly one `Authorization: Bearer sdk-native-costs-loopback-only` and
  `Accept: application/json` header.
- An empty request body, absent request content type, and no persisted cookie
  after responses deliberately set one.
- Exact response bytes and the SDK's typed decoded number tokens.

Every request connection is closed. Server socket deadlines, bounded headers and
bodies, explicit socket shutdown and a finite thread join cover failure paths.
Native commands run in their own process groups with finite deadlines; timeout or
interruption terminates that owned group and preserves partial stdout/stderr.
No sample contacts the real OpenRouter API.

## Evidence and sizes

`report.json` is written once with `format: suspect.sdk.native-costs.v1`,
`methodology: repeated-v1`, runner fingerprints, `complete`, and
`measurements`. A successful report has exactly the 92 required dimensions.
Every row has the integration contract's `language`, `tier`, `phase`,
`packageManifestSha256`, positive `artifactBytes`, its declared `runs` counts,
a steady-state `summary`, and its full run set as `samples`. Each sample is one
run: it records its `runKind` (`cold`, `warmup`, or `steady`), positive
`nanoseconds`/`iterations`, actual `exitCode`, the absolute executable and
actual argv, `programSha256`, and confined relative stdout/stderr paths with
their SHA-256 digests.

Additional retained evidence:

- `commands/`: all version, preparation and sample subprocess records, exact
  environment/cwd/deadlines, and raw stdout/stderr including failures.
- `bindings/`: selected source-addressed allocated symbols and metadata hashes.
- `installed/` and `input-inventory.json`: emitted/installed byte linkage and
  immutable input censuses. A runner `generated-manifest.json`, when present,
  must match the complete emitted file census. Frozen CLI bytes are checked when
  exposed by `SUSPECT_TEST_BINARY` or the runner's `bin/suspect` layout.
- `work/`: fresh package copies, actual typed driver source, private build
  outputs, compiler inputs and installed-consumer linkage.
- `artifacts/`: explicit file/byte/hash inventories used to recompute every
  `artifactBytes` field. Build rows count their named native build payload;
  runtime rows count the loaded SDK/consumer payload or linked executable.
  System interpreter/VM/OS libraries are outside these payload sizes. Kotlin's
  runtime inventory includes its explicit stdlib/coroutines jars.
- `wire/` and `fixtures/`: independent raw fixture bytes and checked request
  records, response digests and server termination proof.
- `observations.jsonl`: flushed incremental rows/failures. `candidate-report.json`
  retains the report before final self-verification. Failed attempts keep their
  sources, outputs and logs; output paths cannot be reused or resumed.

Validation checks dimensions, exact counts, package/tool/input integrity, raw-log
confinement/hashes, subprocess/sample attribution, actual artifact byte counts,
native completion witnesses and request records. Stored summaries are never
trusted: validation re-counts each row's retained run kinds against its declared
`runs` policy and **recomputes the steady-state statistics from the retained raw
samples**, rejecting any row whose `summary` disagrees with its own raw data. A
row or report that declares a different `methodology` is refused. Historical
single-sample receipts without a `methodology` field still verify and summarize
as their own one-run set. Source/CLI fingerprints remain
the full runner's frozen-census authority; the collector separately fingerprints
its own sources, drivers and interpreter. A killed process without a final report
is incomplete evidence.

These are raw observations with explicit cost/size boundaries. The published
medians and tail percentiles describe the retained steady-state samples on the
collecting host; they establish no performance qualification, calibrated-host
status or regression threshold.

## Checks and development observations

```sh
# Host/adversarial checks; the native integration entrypoint remains ignored.
cargo test --locked --offline -p suspect-codegen --test sdk_native_measurements

# Direct stdlib checks.
python3 -B -m unittest discover -s tools/sdk-native-costs -p 'test_*.py' -v

# With all six required variables exported by the runner:
python3 -B tools/sdk-native-costs/run.py collect
python3 -B tools/sdk-native-costs/run.py validate

# Adjust the run-set policy or emit a derived summary view beside the raw evidence:
python3 -B tools/sdk-native-costs/run.py collect --cold-runs 3 --warmups 1 --steady-runs 10
python3 -B tools/sdk-native-costs/run.py collect --report summary

# Isolated development investigation, still requiring the maintained target JSON:
python3 -B tools/sdk-native-costs/run.py collect --only python
python3 -B tools/sdk-native-costs/run.py validate --allow-incomplete
```

With `--report summary`, the complete raw document is retained as
`raw-evidence.json` and `report.json` becomes a derived `reportView: summary`
document that carries `rawReport`/`rawReportSha256`. Validation re-verifies the
raw document and rejects any summary row that disagrees with it; aggregates are
never the only copy of the evidence.

A development subset retains `complete: false`, lists the missing dimensions,
and exits nonzero from `collect`. `validate --allow-incomplete` verifies existing
partial observations; it does not promote them to acceptance. Each new attempt
requires a new `SUSPECT_SDK_FULL_MEASUREMENTS` directory.

The host checks cover malformed/missing configuration, all 92 dimensions,
allocated-name metadata, immutable-source/installed-root drift, artifact/log/tool
tampering, traversal/symlink escapes, failed or fake exit-zero samples, missing
encode work, wrong precision/counts, wire auth/body/cookie errors, deadlines and
cleanup. Their synthetic `UNIT` report fixtures are never native measurements.

### Retained development validation, 2026-09-10

Forty real observations were collected against **new canonical-name packages
emitted by an existing source-attributed, frozen five-backend CLI**. These
receipts predate `repeated-v1`; validation still accepts them and summarizes
each row as its own one-run set, while any row that declares a `methodology`
must be a complete, self-consistent `repeated-v1` run set. Their identity
and original source-census linkage are retained in
`target/sdk-native-costs-validation-20260910-03/historical-cli-provenance.json`.
These exercise the collector; they are not the fresh twelve-backend full-runner
acceptance required for the current implementation.

| Retained report | Successful native rows |
| --- | --- |
| `target/sdk-native-costs-validation-20260910-03/observations-01/report.json` | 24: TypeScript, Python, Go; both tiers, all four phases |
| `target/sdk-native-costs-observations-20260910-04/report/report.json` | 12: Rust on both tiers and current Swift, all four phases |
| `target/sdk-native-costs-observations-20260910-05/report/report.json` | 4: recovered Swift floor, all four phases |

The Rust/Swift collector survived a harness-server restart and completed its
existing attempt. Completed observations were retained rather than re-collected.
The first Swift-floor attempt remains an explicit failure: the original
`swift-6.0.3-toolchain/expanded` payload lacks `SwiftShims`. Only that unfinished
tier was subsequently run with the already-restored toolchain documented in
[SDK-SWIFT-PROTOCOL.md](SDK-SWIFT-PROTOCOL.md#verification-and-retained-evidence).
For this host, select its actual payload before the full runner starts:

```sh
export SUSPECT_SWIFT_FLOOR_ROOT="${TMPDIR%/}/opencode/swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"
export SUSPECT_SWIFT_FLOOR_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/swift"
export SUSPECT_SWIFTC_FLOOR_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/swiftc"
export SUSPECT_SWIFT_FLOOR_DOCC_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/docc"
export SUSPECT_SWIFT_FLOOR_SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
```

Each partial report preserves `complete: false` and its exact missing dimensions.
The remaining Java/C#/Kotlin/Ruby/PHP/Dart/C++ recipes are implemented for the full
runner's canonical packages and installed roots; their 52 native cost dimensions
still require that fresh integrated invocation. The earlier all-feature CLI build
attempt in `target/sdk-native-costs-validation-20260910-02/` captured in-progress
Java/C++ migration errors and is retained as a failed historical attempt. Main
subsequently reported canonical-options, all-twelve feature capture and CLI tests
passing; that earlier build failure is not a claim about Main's current tree.
