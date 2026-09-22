# SDK elimination audit

Audit version: **`suspect-sdk-elimination-audit-v3`**, 2026-09-16.

This document records, per backend, (1) the mechanism that makes one operation's
artifacts independent of unrelated operations, codecs, schema validators, OAuth
grants, storage adapters and platform shims, and (2) the acceptance gate that
currently proves it. Every number below was measured by a command run in this
session; anything not measured here is marked **not yet gated**. The
TypeScript gates live in
`crates/suspect-codegen/tests/typescript_elimination.rs`; the per-language
M2 gates added in this session live next to it
(`swift_elimination.rs`, `java_elimination.rs`, `kotlin_elimination.rs`,
`csharp_elimination.rs`, `ruby_elimination.rs`, `php_elimination.rs`,
`cpp_elimination.rs`, `dart_elimination.rs`); a
machine-readable record of the last run of each gate is written to
`target/tmp/<language>-elimination-report.json`.

## How to read the gates

- **Bundler gate (TypeScript):** each consumer profile is bundled twice with
  the repo-pinned esbuild (once against the canonical fixture, once against the
  same fixture plus one unrelated operation). Retention is asserted with
  string-literal markers (identifiers are minified away, wire literals are
  not), the resolved module graph is asserted from the esbuild metafile, and
  the plan's additivity property — *adding an unrelated operation must not
  increase a single-operation consumer artifact* — is asserted on artifact
  bytes with a 64-byte slack (measured deltas are 0–4 bytes).
- **Graph gate (TypeScript, fallback):** a dependency-free import-graph model
  of a *perfect* tree shaker (name propagation over the emitted ESM graph,
  pruning unused binding imports, honoring `import type` and re-export
  edges). It asserts module-level binding reachability and is always computed
  and recorded; where it disagrees with the bundler, the divergence is a
  recorded defeat, not a weakened assertion.
- **Launcher gate (all TypeScript profiles):** generation-level byte stability —
  every shared runtime/infrastructure file must be byte-identical when only an
  unrelated operation is added, and only a documented set of per-contract
  artifacts (operations module, model, validation-program, docs, manifests,
  examples) may differ.
- **Native toolchain gates (Go, Rust):** real `go build` / `cargo build`
  consumer artifacts measured in this session.
- **Linked-binary gates (Swift, C++):** real toolchain consumer executables
  (SwiftPM debug/release; CMake Release with and without function sections +
  linker dead stripping), `nm` symbol counts and raw-bytes markers. Where the
  linked artifact retains what a consumer never references, the gate asserts
  the measured retention and records it as a defeat.
- **Consumer-unit gates (Java, Kotlin):** real `mvn install` artifacts, a
  `javac`/`kotlin-maven-plugin`-compiled single-operation consumer, and
  `javap -v -p` constant-pool analysis: the consumer's own compiled unit must
  reference exactly one operation, and the compiled unit must be byte-identical
  across generations. The one-artifact composition shape of the emitted jar is
  measured and documented alongside. The poms' dependency declarations are
  asserted (Java: zero beyond the JDK; Kotlin: the documented stdlib +
  coroutines set) — the no-transitive-retention guarantee.
- **Trim gate (C#):** real `dotnet pack` + NuGet consumer +
  `PublishTrimmed` self-contained publish; the rewritten SDK assembly is
  inspected with System.Reflection.Metadata to measure which operation methods
  survive trimming.
- **Load-graph gates (Ruby, PHP):** real interpreter runs measuring the actual
  require/autoload reachability (`$LOADED_FEATURES` under Ruby 3.3,
  `get_included_files()` + Composer classmap under PHP 8.3), with a functional
  single-op/codec round trip. Where a file groups unrelated operations, the
  file-granular retention is measured and recorded as a defeat.
- **AOT-snapshot gate (Dart):** real `dart analyze --fatal-infos` +
  `dart compile exe` consumer binaries with raw-bytes markers over the
  snapshot (sizes, not symbol counts — Dart AOT snapshots are data with a
  constant exported-symbol surface). The structural half of the gate
  (launcher byte-identity, zero-dependency pubspec) runs unconditionally; on
  2026-09-16 the AOT half was blocked by emission defects and its outcomes
  were measured on locally repaired copies instead (recorded below, never
  fixed in this measurement task).

Fixture: one limit/offset-paginated list, one discriminated SSE stream, one
plain JSON read, one OAuth2 client-credentials operation, one device-flow
operation, all under an API-key document policy, generated with
`sdk_defaults` (pagination + OAuth schemes configured); the extended variant
adds one unrelated `listGizmos` operation with a distinctive
`"zeta-quantum"` enum literal. All per-language gates use the identical
fixture document so measurements stay cross-language comparable.

## TypeScript (`typescript-http`)

Mechanism: ESM subpath exports (`/operations`, `/models`, `/codecs`, `/json`,
`/oauth`), conditional emission (pagination only under `sdk_defaults`, OAuth
only for executable schemes, typed streams only for discriminated SSE), and an
emission shape designed for tree shaking: per-operation `Source`/`Wire`/
`Descriptor` constants, per-operation codec tables referencing the
`model-codecs` namespace by property access, `/* @__PURE__ */`-initialized lazy
codec and validator bindings, and per-root validation programs whose node
tables are pure object literals.

Gate: `consumer_profiles_eliminate_unrelated_surfaces` — **bundler gate ran**
(esbuild **0.28.2**, the version pinned in
`crates/suspect-codegen/tools/typescript-http-bundle/package.json`, invoked
from that toolchain's install). Rollup/Vite are **not installed on this
machine** (`npx --no-install rollup --version` fails); a Rollup or Vite gate
is **CI-pending**. Generated package size numbers below are raw minified ESM
bytes (gzip-6 in parentheses).

| Profile | Base (B) | + unrelated op (B) | Δ | Modules | gzip base / ext (B) |
| --- | --- | --- | --- | --- | --- |
| empty import | 38 | 38 | 0 | 1 | 58 / 58 |
| core-only `createClient` (via `/operations`) | 229 257 | 246 586 | +17 329 (by design) | 20 | 40 700 / 41 614 |
| core-only `createClient` (via package root) | 229 257 | 246 586 | +17 329 (by design) | 22 | 40 696 / 41 615 |
| one JSON operation (`getGadget` from `/operations`) | 150 707 | 150 707 | **0** | 20 | 36 476 / 36 479 |
| one pager (`listWidgetsItems` from `/operations`) | 162 007 | 162 011 | **+4** | 20 | 37 413 / 37 419 |
| one OAuth provider (`createClientCredentialsProvider` from `/oauth`) | 7 355 | 7 355 | **0** | 2 | 2 836 / 2 836 |
| one codec (`GadgetCodec` from `/codecs`) | 52 917 | 52 917 | **0** | 9 | 15 697 / 15 697 |
| type-only imports (`import type`) | 0 | 0 | 0 | 1 | 20 / 20 |
| plain imports used only in type positions | 0 | 0 | 0 | 1 | 20 / 20 |
| all operations (namespace capture baseline) | 247 314 | 264 734 | +17 420 (by design) | 20 | 45 667 / 46 593 |

Verified eliminations (marker absence in both generations):

- One JSON operation retains its own function, guard, wire and codec closure,
  and sheds the other four operations (`createBanner`, `listLicenses` absent),
  the pagination walker code (`PaginationError` absent) and every OAuth grant
  code string (`client_credentials`, `authorization_code`,
  `code_challenge_method`, `S256`, the device-code grant URI all absent).
- The single-operation artifacts above contain none of the unrelated
  operation's markers (`listGizmos`, `zeta-quantum`) — including its codec and
  validation-program nodes — so the unrelated operation contributes **zero
  bytes** (0/0/+4/0/0 measured deltas).
- One pager retains exactly its own operation plus the walker library and its
  descriptor entry; the other three operations shed.
- One OAuth provider (7 355 B total) keeps the client-credentials grant, the
  shared on-demand refresh helper and the single compiled `MemoryTokenStore`
  adapter (v1 admits exactly one storage adapter), and sheds the
  authorization-code PKCE machinery and the device-authorization grant
  entirely; the resolved graph is exactly the entry plus `oauth.ts`.
- One codec pulls the codec runtime, the validation runtime and its own
  model's validator closure (`standard`/`compact` literals prove the right
  validator), with no operations module, no pagination, no OAuth and no HTTP
  runtime shims in the graph.
- Pure type imports erase entirely: both `import type` and plain imports used
  only in type positions bundle to **0 bytes** with only the entry module in
  the resolved graph.
- The generated package emits no dynamic imports, so each bundle is exactly
  one chunk; asynchronous-chunk and copied-asset accounting is trivially
  empty (asserted: metafile `outputs` length is 1 for every profile).

Verified by the launcher gate: when only the unrelated operation is added,
all 21 shared infrastructure files (`runtime.ts`, all eight `http/*.ts`
shims, `json.ts`, `codecs.ts`, `validation.ts`, `pattern.ts`,
`validation-resources.ts`, `uri.ts`, `pagination.ts`, `oauth.ts`,
`source/index.ts`, `package.json`, `package-lock.json`, `tsconfig.json`) are
byte-identical, and exactly the 14 per-contract artifacts differ
(`operations.ts`, `models.ts`, `model-codecs.ts`, `validation-program.ts`,
`models.md`, `codecs.md`, `validation.md`, `http.md`, `docs-readme.md`,
`docs-manifest.json`, `http-manifest.json`, `examples.json`, `examples.md`,
`examples/validated.ts`).

### Recorded tree-shaking defeats (measured, not fixed)

All grant/walker/operation/codec **code** sheds correctly. Three top-level
descriptor-**data** initializers are not provably side-effect-free for
esbuild, so an unused import or re-export of their module degrades into a
side-effect import and the dead data is retained:

1. `typescript/pagination.ts` — `export const paginationDescriptors =
   Object.freeze({...})`. Every `operations.ts` consumer (even one using zero
   pagination bindings) retains the full descriptor map of all paginated
   operations (`"limit-offset"` marker). Walker code sheds.
2. `typescript/oauth.ts` — `const schemes = {...}` +
   `export const oauthSchemes = Object.freeze(schemes)`. Compiled scheme
   descriptors (all flows' URLs, source spans) are retained by any
   `operations.ts` consumer and by `/oauth` consumers importing a single
   provider. Provider/grant/store code sheds (the 7 355 B provider bundle
   proves the code sheds).
3. `typescript/operations.ts` — the typed stream-events descriptor clone
   `const XEventsDescriptor = { ...XDescriptor, responseCodecs: { ... } }`
   uses object spreads, which esbuild cannot prove pure; the clone (and
   transitively the operation's full `Source`/`Wire` data) is retained as
   dead data by every `operations.ts` consumer (`streamChat`,
   `text/event-stream` markers).

Emitter-side fix (not applied here — this is a measurement/gate task):
annotate these top-level initializers `/* @__PURE__ */`, exactly like the
codec (`createLazyCodec`) and validator (`createLazyValidator`) bindings that
already shed correctly. The dependency-free graph model records the same
divergence (`graphModelEvaluation*` in the run report: `oauth.ts` is
evaluation-reachable from `operations.ts` through its re-export line).

## Go (`go-http`)

Mechanism: per-package compilation with zero dependencies; each selected
operation is emitted as its own functions, input types and codec bindings in
`operations.go`/`models.go`/`codecs.go`, and the Go linker eliminates
unreferenced package-level symbols at link time.

Gate: verified with real `go build` runs in this session (go1.27.1
darwin/arm64, generated with the `suspect codegen --profile go-http` CLI on a
two-operation fixture: `listWidgets`, `getGadget`; extended variant adds
`listGizmos`):

- Consumer calling only `GetGadget` against the one-operation SDK:
  **7 201 026 B** binary.
- Consumer calling both operations against the all-operations SDK:
  **7 239 138 B** binary (the 38 112 B difference is `listWidgets`' actually
  used code and data).
- Additivity: the single-operation consumer binary is **byte-identical**
  (`go build -trimpath -ldflags "-buildid="`) whether the SDK source contains
  two or three operations, and contains zero occurrences of the unrelated
  operation's markers (`listGizmos`, `zeta-quantum`; `grep -c` = 0). Without
  `-trimpath`, the sizes are identical and the bytes differ only in embedded
  source-path and build metadata.

Not yet gated: `go build` consumer gates against an `sdk_defaults`-enabled
fixture (pagination walkers and OAuth lifecycle modules — the CLI `codegen`
command has no `sdk_defaults` input, so this session's Go fixture exercised
the plain protocol surface); go build gates in CI.

## Rust (`rust-http`)

Mechanism: cargo feature gates on the generated crate —
`[features] default = []` (model-only), `http = ["dep:url"]`,
`reqwest-rustls = ["http", "dep:reqwest", ...]`; `lib.rs` gates the client
surface with `#[cfg(feature = "http")] pub mod operations;`, so the
operations module does not exist at compile time under the default features,
and every dependency (`serde`, `serde_json`, `url`, `reqwest`) is optional.

Gate: verified with real `cargo build` runs in this session (rustc 1.97.1):

- Model-only consumer (`GadgetCodec` round-trip) builds **with zero
  dependencies** (`cargo build --offline` succeeds; the SDK compiles nothing
  beyond its own sources): binary **1 381 664 B** debug / **582 864 B**
  release, with zero HTTP markers (`api.elimination.test`: 0, `get_gadget`:
  0).
- HTTP consumer (`reqwest-rustls`, calls `get_gadget`): builds; binary
  **15 925 440 B** (debug).
- Additivity: the same HTTP consumer against the single-operation SDK
  generated from the extended source: **15 926 752 B** (+1 312 B of
  debug/metadata noise; `zeta-quantum`, `list_gizmos` markers absent). The
  generation-level file set shows the shared infrastructure (`src/http/`,
  `support.rs`, `json.rs`, `Cargo.toml`) byte-identical; only `models.rs`,
  `codecs.rs`, `validation.rs` and the per-operation file differ (the
  per-operation file's diff is the embedded source-path string of the
  different fixture file, an environment artifact).
- Consumer using only `getGadget` against the two-operation SDK:
  **15 942 400 B**, with zero `list_widgets` and zero `zeta-quantum`
  markers — unreferenced operation code is dropped at link granularity.

Not yet gated: `cargo build` consumer gates in CI against an
`sdk_defaults`-enabled fixture (pagination walkers under the `http` feature).

## Swift (`swift-http`)

Mechanism: AOT compilation with `swiftc`; SwiftPM builds the emitted package
as one static library, and executables link it with the Darwin linker.
Conditional emission follows the shared plan shape: per-operation files in
`Operations.swift`/`Client.swift`, pagination only under `sdk_defaults`
(`Pagination.swift`), OAuth only for executable schemes (`OAuth.swift`),
streams only for SSE operations (`HTTPStreams.swift`, `StreamEvents.swift`).

Gate: `swift_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/swift_elimination.rs` — real `swift build`
consumer executables (Swift 6.3.3, darwin/arm64), measured debug and release,
with `nm` symbol counts and raw-bytes marker checks:

| Consumer | Config | Base (B) | + unrelated op (B) | Δ | nm symbols base / ext |
| --- | --- | --- | --- | --- | --- |
| one operation (`getGadget`) | debug | 2 761 312 | 2 849 792 | **+88 480** | 8 702 / 8 987 |
| one operation (`getGadget`) | release | 1 811 272 | 1 895 272 | **+84 000** | 6 395 / 6 650 |
| codec-only (`Codecs.gadget`) | debug | 2 758 464 | 2 846 992 | **+88 528** | 8 670 / 8 955 |
| codec-only (`Codecs.gadget`) | release | 1 808 824 | 1 892 792 | **+83 968** | 6 370 / 6 625 |

### Recorded elimination defeat (measured, not fixed)

The plan's additivity property **does not hold** for Swift at link
granularity, and the gate asserts the measured retention instead:

- Every consumer artifact retains every selected operation, the pagination
  walker code (`PaginationError` marker) and the OAuth grant code
  (`client_credentials`, the device-code grant URI) — including the
  codec-only consumer that never references `Client`, in debug, in
  release/whole-module, and with an explicit linker pass: a release codec-only
  build with `-Xlinker -dead_strip` measures **byte-identical** to the plain
  release build (1 808 824 B, `listWidgets` still retained), so linker dead
  stripping is not the differentiator.
- The unrelated operation contributes 83 968–88 528 B to every artifact. In
  the extended one-operation consumer the `Client.listGizmos` async function,
  its `ListGizmosHTTP.metadata` lazy-global accessors and the
  `ModelCodec<UnrelatedGizmo>` generic metadata/value-witness records are all
  retained.
- Mechanism (from disassembly): Swift's reflective metadata records and lazy
  per-operation globals root the module's public API — the same
  composition-shape category as the TypeScript `createClient`/descriptor-map
  defeats, but covering the whole module rather than three data tables.
- Emitter-side fix (not applied here — this is a measurement/gate task):
  emit one SwiftPM target (or product) per operation so the static-archive
  member is the elimination unit, mirroring the Go per-operation functions
  that the Go linker sheds today.

The test pins these outcomes: if a future toolchain or emitter change makes
unreferenced operations shed, the gate fails loudly and forces this audit to
be updated with the improved measurement.

## Kotlin (`kotlin-http`)

Mechanism: JVM bytecode plus per-operation classes; on Android, R8/ProGuard
shrinks unreferenced classes and members; on the JVM, unreferenced classes are
never loaded. Conditional emission per operation/stream/oauth module.

Gate: `kotlin_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/kotlin_elimination.rs` — real `mvn install`
builds (Temurin 21.0.12+1, Maven 3.9.16, Kotlin 2.4.20) plus a
single-operation consumer compiled by `kotlin-maven-plugin` against the
installed artifact:

- Consumer-unit gate: the consumer (only `client.getGadget(...)` inside one
  coroutine) compiles to a **880 B** `EliminationGateKt.class` that is
  **byte-identical** whether the SDK source contains five or six operations.
  Its `javap -v -p` constant pool references `getGadget` and references none
  of `listWidgets`/`streamChat`/`createBanner`/`listLicenses` and no
  `ListGizmos` type — method-level elimination measured in the consumer's own
  compiled unit.
- Dependency gate: the emitted `pom.xml` declares **exactly**
  `kotlin-stdlib` 2.4.20 and `kotlinx-coroutines-core-jvm` 1.11.0 — no other
  runtime dependency exists, so nothing transitive can be retained.
- Artifact composition shape (documented, not fixed): the emitted package is
  one artifact; the jar grows 287 → 309 classes and ships the unrelated
  operation's four classes (`ListGizmosInput`,
  `ListGizmosInput$...`/`ListGizmosResult`, `ListGizmosResult$Status200`,
  `ListGizmosApiException`).

Not yet gated: R8/ProGuard consumer shrink measurement (Android); the JVM
never-loads-unreferenced-classes property is implied by the consumer-unit
gate above plus the documented dependency set, but an R8 size gate is
CI-pending.

## Java (`java-http`)

Mechanism: immutable per-operation client classes with Maven packaging;
unreferenced classes are never loaded on the JVM, and R8/ProGuard performs
the Android-side elimination.

Gate: `java_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/java_elimination.rs` — real `mvn install`
builds (Temurin 21.0.12+1, Maven 3.9.16) plus a `javac`-compiled
single-operation consumer:

- Consumer-unit gate: the consumer (only
  `client.getGadget(Client.GetGadgetInput.builder("g").build())`) compiles to
  a **1 529 B** `OneOperation.class` that is **byte-identical** whether the
  SDK source contains five or six operations. Its `javap -v -p` constant pool
  references `getGadget` and references none of
  `listWidgets`/`streamChat`/`createBanner`/`listLicenses` and no
  `ListGizmos` type.
- Dependency gate: the emitted `pom.xml` declares **zero** `<dependencies>` —
  the runtime surface is the JDK alone, which is the hard elimination
  guarantee: no transitive retention is possible at all.
- Artifact composition shape (documented, not fixed): the emitted package is
  one artifact; the jar grows 157 → 166 classes and ships the unrelated
  operation's four classes (`Client$ListGizmosInput`,
  `Client$ListGizmosInput$Builder`, `Client$ListGizmosStatus200`,
  `ListGizmosResponse`).

Not yet gated: R8/ProGuard consumer shrink measurement (Android); CI-pending.

## C++ (`cpp-http`)

Mechanism: per-operation translation units compiled with
`-ffunction-sections -fdata-sections`, letting the linker garbage-collect
unreferenced sections (`--gc-sections` for ld/lld, `-dead_strip` for ld64 on
Darwin); static-archive member granularity sheds whole members (OAuth,
pagination, streaming) for consumers that do not reference them.

Gate: `cpp_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/cpp_elimination.rs` — real `cmake` + Apple
clang 21.0.0 consumer builds, measured plain Release and Release with
`-ffunction-sections -fdata-sections` + `-Wl,-dead_strip`:

| Consumer | Config | Base (B) | + unrelated op (B) | Δ | nm symbols base / ext |
| --- | --- | --- | --- | --- | --- |
| one operation (`get_gadget`) | release | 720 536 | 744 696 | +24 160 | 1 397 / 1 446 |
| one operation (`get_gadget`) | dead-strip | 449 384 | 465 896 | +16 512 | 778 / 778 (**identical**) |
| codec-only (`decode_4`/`encode_4`) | release | 16 840 | 16 840 | **0** | 2 / 2 |
| codec-only (`decode_4`/`encode_4`) | dead-strip | 16 840 | 16 840 | **0** | 2 / 2 |

- **Measured elimination (codec-only):** a consumer that only takes the
  address of one codec pair links a **16 840 B / 2-symbol** executable that
  is **byte-identical** across generations and contains none of the
  unrelated operation's code or data markers — every operation method, the
  OAuth grants, the pagination walker and the stream machinery shed
  completely, in both configurations.
- **Measured elimination (one-operation, dead-strip):** with function
  sections and dead stripping, every unreferenced operation's code sheds:
  the extended one-operation consumer's symbol table is **identical**
  (778 symbols) to the base generation's, and the unrelated operation's
  method (`list_gizmos`), operation id (`listGizmos`) and every other
  operation's markers are absent. Static-archive member granularity already
  sheds the OAuth/pagination members even without dead stripping.
- **Recorded data-granular defeat:** the unrelated schema's codec *data*
  (`zeta-quantum`, `UnrelatedGizmo`) rides in the shared models/program
  descriptor tables and costs **16 512 B** in the dead-strip one-operation
  consumer (the dead-strip analogue of the TypeScript descriptor-map
  defeats). Emitter-side fix (not applied here): split the schema
  descriptor tables per schema root so per-table granularity matches
  per-operation granularity.
- **Recorded retention defeat (plain Release):** without function sections
  the whole `client.cpp` member is one link unit, so every operation method
  is retained (the unrelated operation costs 24 160 B); the
  `-ffunction-sections` configuration is therefore the package's documented
  elimination configuration.

## C# (`csharp-http`)

Mechanism: per-operation Task-client classes; .NET IL trimming
(`dotnet publish -p:PublishTrimmed=true`) removes unreferenced types and
members for self-contained deployments.

Gate: `csharp_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/csharp_elimination.rs` — real `dotnet pack`
(dotnet SDK 8.0.424, osx-arm64), a NuGet-referencing single-operation
consumer, System.Reflection.Metadata inspection, and a trimmed
self-contained publish:

- Dependency gate: the emitted `Suspect.csproj` declares **zero**
  `<PackageReference>` items — no transitive retention is possible.
- Consumer-unit gate: the consumer (only
  `await client.GetGadgetAsync(...)`) builds against the packed SDK; its
  compiled assembly references `GetGadgetAsync` and references none of
  `ListWidgetsAsync`/`StreamChatAsync`/`CreateBannerAsync`/
  `ListLicensesAsync` and no `ListGizmos` type.
- **Trim gate (measured elimination):** `dotnet publish -c Release -r
  osx-arm64 --self-contained -p:PublishTrimmed=true` rewrites the SDK
  assembly, and the rewritten `Client` type retains exactly
  `.ctor`×2 / `Dispose` / `GetGadgetAsync` / `.cctor` — every unrelated
  operation method is **trimmed away**. Publish-directory sizes:
  **21 714 112 B** (base) → **21 732 032 B** (extended), a **+17 920 B**
  delta within the gate's 64 KiB slack; the residual growth is the embedded
  protocol/validation resource data and docs, not retained code.
- Artifact composition shape (documented, not fixed): without trimming, the
  SDK assembly's `Client` carries every operation (measured: all five
  operation methods plus `ListGizmosAsync` in the extended artifact).

Recorded caveat: the trimmer reports `IL2104` for the SDK assembly (the
reflection-shaped HTTP/JSON/Validation runtimes are not trim-annotated); the
consumer suppresses the warning and the trim measurement stands. Emitter-side
follow-up (not applied here): `[IsTrimmable]`/DynamicallyAccessedMembers
annotations so the SDK can trim warning-free.

## Dart (`dart-http`)

Mechanism: per-operation Future clients; `dart compile exe` (Dart AOT)
compiles each emitted package as one library and tree-shakes unreferenced
functions, classes and constants from the snapshot. The emitted pubspec
declares **zero** dependencies (the hard no-transitive-retention guarantee),
and conditional emission follows the shared plan shape: pagination only under
`sdk_defaults` (`lib/src/pagination.dart`), OAuth only for executable schemes
(`lib/src/oauth.dart`), typed streams only for SSE operations
(`lib/src/stream_events.dart`).

Gate: `crates/suspect-codegen/tests/dart_elimination.rs` (Dart 3.9.4 via
mise), two layers:

- **Structural gate (runs, green):** the launcher byte-identity property
  holds exactly — when the unrelated operation is added, only the 8
  per-contract files differ (`doc/API.md`, `doc/validation-program.json`,
  `example/source_examples.dart`, `lib/src/client.dart`,
  `lib/src/models.dart`, `lib/src/program.dart`, `lib/src/protocol.dart`,
  `sdk-manifest.json`) and all 25 shared runtime parts are byte-identical;
  the unrelated operation's markers (`listGizmos`, `UnrelatedGizmo`,
  `zeta-quantum`) never appear in a shared runtime file; one client file
  carries every selected operation's methods (composition shape: AOT
  function-level shedding inside it is the elimination mechanism, the
  Java/PHP one-class analogue); and the zero-dependency pubspec resolves
  with `dart pub get --offline` with no hosted package in the graph.
- **AOT consumer gate (`#[ignore]`d — blocked, see below):** `dart analyze
  --fatal-infos` plus a `dart compile exe` consumer matrix — one-operation
  (`getGadget`), codec-only (`gadgetCodec`) and a calling all-operations
  baseline — against both generations, with artifact sizes, raw-bytes
  markers and a size-determinism probe. Its assertions are pinned to the
  measured outcomes below.

### Emission defects blocking the AOT gate (measured, not fixed)

The Dart 3.9.4 toolchain is newly available on this machine, so this is the
first session to run the emitted bytes under a real Dart analyzer. Result:
**`dart analyze` reports six hard errors on every emitted dart-http package**
(`dart pub get --offline` itself succeeds — the pubspec is
zero-dependency). No dart-http package currently compiles, so the AOT
consumer measurements cannot run on the emitted bytes. The six errors come
from four emitter/runtime defects:

1. `src/dart_sdk/transport.dart` (static runtime) — the `_ClientBase`
   constructor takes `userAgent`/`applicationId` as plain parameters but
   never initializes the `final` fields `_userAgent`/`_applicationId`
   (`final_not_initialized_constructor` at `lib/src/transport.dart:201`;
   the fields are read by the attribution header code). **Universal: this
   alone breaks every package the backend emits, with or without
   `sdk_defaults`.** One-token fix (not applied): `this._userAgent,
   this._applicationId` in the constructor parameter list.
2. `src/dart_sdk/stream_events.rs` — the sealed event base class emits only
   `const StreamChatEvent._()` while each event subclass constructor invokes
   the unnamed super constructor (`undefined_constructor_in_initializer`,
   three sites in `lib/src/stream_events.dart`). Fix (not applied):
   `: super._()` on the subclass initializers.
3. `src/dart_sdk/oauth.rs` — the `OAuthTransport` typedef is declared
   synchronous while both endpoint transports (`oauth_transport_io.dart`,
   `oauth_transport_stub.dart`) are `async`, so `_defaultTransport`'s
   tear-off does not type-check (`return_of_invalid_type` at
   `lib/src/oauth.dart:126`). Fix (not applied): `Future<...>`-wrap the
   typedef; every call site already awaits.
4. `src/dart_sdk/stream_events.rs` — the typed-events core emits a
   `StreamChatEvent? held;` local that is dead when the operation declares
   no sentinel token, and the package's `analysis_options.yaml` upgrades
   unused locals to errors (`unused_local_variable` at
   `lib/src/client.dart:249`).

All four defects live in runtime/emitter files that this measurement task
must not touch, so they are recorded here instead. Once fixed, remove the
`#[ignore]` on `dart_consumers_eliminate_unrelated_operations` — its
assertions are already pinned to the measured outcomes below, and a failure
there forces this audit to be re-pinned.

### Measured AOT behavior (Dart 3.9.4, on locally repaired copies)

Because the four defects are mechanical, the same two generated packages
were repaired by hand in throwaway copies under
`target/sdk-dart-elimination/repair{a,b}` (never in the repo, never in the
bytes the committed gate asserts on), where both analyze clean under
`--fatal-infos`, and the full consumer matrix was compiled and measured
there (full record: `target/tmp/dart-elimination-repaired-probe.json`):

| Consumer | Base (B) | + unrelated op (B) | Δ | Unrelated markers |
| --- | --- | --- | --- | --- |
| one operation (`getGadget`) | 6 235 216 | 6 235 216 | **0** | code absent; `zeta-quantum`/`UnrelatedGizmo` data retained |
| codec-only (`gadgetCodec`) | 5 348 752 | 5 348 752 | **0** | none at all |
| all operations (calling baseline) | 6 366 544 | 6 399 376 | +32 832 (by design) | all retained |

- **Measured code elimination:** every unreferenced operation's methods,
  the pagination walker (`PaginationException`), the OAuth grant code
  (`client_credentials`, the device-code grant URI) and the typed-stream
  machinery shed completely from the single-operation artifacts — including
  their name strings. Method-level elimination inside the one-client-file
  composition shape works exactly as the mechanism claims.
- **Recorded data-granular defeat:** the one-operation consumer validates
  responses through the shared program tables — the single
  `_validationNodes` list and `_validationRoots` map in `program.dart`,
  indexed at runtime by `validation.dart` via `ModelCodec.fromJson` →
  `requireValid` — so the unrelated schema's data (`zeta-quantum`,
  `UnrelatedGizmo` pointers) ships even though its code sheds. Measured
  size-neutral: the 0 B delta shows snapshot pool slack absorbed the extra
  table data. The codec-only consumer never reaches the validation path and
  sheds the tables entirely (measured full isolation: no unrelated marker
  whatsoever). Emitter-side fix (not applied): split the program/codec
  tables per schema root, exactly like the TypeScript/C++/PHP/Ruby
  defeats below.
- **OAuth is caller-composition:** the generated client never references the
  grant/lifecycle code internally (callers pass `Credentials(oauth: ...)`
  providers), so the grants shed from every profile — including the calling
  all-operations baseline. Stronger than the TypeScript `oauth.ts` scheme
  table, which rides along as data.
- **Tear-offs do not pin code:** a tear-off-only baseline measured
  5 414 416 B — 820 800 B smaller than one real call — and retained zero
  operation markers (Dart AOT sheds never-invoked closures), so the
  committed baseline calls every operation through statically reachable,
  runtime-unreachable branches instead.
- **Measurement mechanics:** `dart compile exe` is size-deterministic but
  **not** byte-deterministic across compiles (same input: equal size,
  different sha256), and `nm` symbol counts are constant (621) for every
  profile — the snapshot is data with a fixed exported-symbol surface — so
  cross-generation comparison uses sizes and raw-bytes markers, never
  hashes. The committed gate asserts exactly that.

## PHP (`php-http`)

Mechanism: no ahead-of-time dead-code elimination exists for interpreted PHP;
elimination is achieved at emission (per-operation classes are only emitted
for selected operations) and at load time (Composer classmap autoload never
loads a class that is not referenced).

Gate: `php_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/php_elimination.rs` — real `composer
dump-autoload` + `php -r` runs (PHP 8.3.32, Composer 2.10.3):

- Classmap gate: the generated `autoload_classmap.php` maps **104 classes**
  (base) → **110 classes** (extended); the unrelated model class
  (`UnrelatedGizmo`) is mapped in the extended artifact — documented
  artifact-level composition.
- Codec consumer gate (measured elimination): a functional consumer that
  only decodes a `Gadget` (`EliminationSdk\Gadget::fromJson`, round-trip
  asserted) loads **9 package files / 155 942 B** (base). `Client.php`,
  `OAuth.php`, `Stream.php`, `Transport.php`, `Http.php` and `Parts.php`
  never load, and `class_exists('EliminationSdk\Client', false)` is false —
  the whole client surface (every operation, the pagination walker, the OAuth
  lifecycle) is eliminated at autoload granularity for this consumer shape.
- Client consumer gate (composition shape): touching
  `EliminationSdk\Client` loads exactly **1 file / 31 366 B** (base →
  34 918 B extended) — the one client class carries every operation's
  methods, so the unrelated operation's methods load with it (the
  `listGizmos` marker is present in the extended loaded set).
- Recorded file-granular defeat: `Codecs.php` groups every schema's codec
  data, so the extended codec consumer's loaded set grows to **171 096 B**
  and carries the unrelated schema's `"zeta-quantum"` marker. Emitter-side
  fix (not applied here): split the codec/shape tables per schema root so
  per-file granularity matches per-operation granularity, exactly like the
  TypeScript `operations.ts` descriptor defeats below.

## Ruby (`ruby-http`)

Mechanism: no dead-code elimination exists for interpreted Ruby; elimination
is achieved at emission (per-operation modules only for selected operations)
and at load time (`require` of only the referenced files).

Gate: `ruby_consumers_eliminate_unrelated_operations` in
`crates/suspect-codegen/tests/ruby_elimination.rs` — real Ruby 3.3.12 runs
measuring `$LOADED_FEATURES`:

- Gemspec gate: the emitted gemspec declares **exactly** the six documented
  standard-library gem dependencies (`net-http`, `uri`, `openssl`,
  `timeout`, `base64`, `securerandom`) — no transitive retention beyond
  them.
- Full-gem consumer: `require "elimination_fixture"` + `Client.new` loads
  **19 modules / 253 523 B** (base → 258 048 B extended). The gem entry
  requires a fixed module list ending in `client.rb`, so this consumer shape
  loads every operation's code (composition-shape finding: no per-operation
  require targets exist), and the extended consumer loads the unrelated
  operation's markers.
- Codec consumer gate (measured elimination): a functional consumer that
  requires only `policy`/`json`/guards/`validation*`/`program`/`codecs`/
  `models` and decodes a `Gadget` through `Codecs::Gadget` (round-trip
  asserted) loads **11 modules / 100 575 B** (base). `client.rb`, `oauth.rb`,
  `streams.rb`, `http.rb`, `payload.rb` and `wire.rb` never load — the whole
  client surface (every operation, the pagination walker, the OAuth
  lifecycle) is eliminated at require granularity for this consumer shape.
- Recorded file-granular defeat: `models.rb`/`codecs.rb` group every schema's
  shape/codec data, so the extended codec consumer's loaded set grows to
  **103 561 B** and carries the unrelated schema's `"zeta-quantum"` marker.
  Emitter-side fix (not applied here): split the shape/codec tables per
  schema root so per-file granularity matches per-operation granularity.

## Gate summary

| Backend | Mechanism | Gate run this session | CI-pending |
| --- | --- | --- | --- |
| TypeScript | ESM subpath exports + conditional emission + pure-annotated lazy bindings | esbuild 0.28.2 bundler gate + graph gate + launcher gate (all green) | Rollup/Vite byte gates |
| Go | link-time dead-code elimination | `go build` consumer sizes + byte-identical additivity | sdk_defaults-enabled fixture gates |
| Rust | cargo feature gates (`http`), model-only default | `cargo build` consumer builds (zero-dep model-only) | sdk_defaults-enabled fixture gates |
| Swift | AOT + linker `-dead_strip` | `swift build` debug/release binaries: **retention defeat recorded** (84–88 KB per unrelated op, even codec-only with `-dead_strip`) | per-operation-target emission re-measurement |
| Kotlin | per-operation classes + R8/JVM | `javap` consumer-unit gate (880 B unit, byte-identical) + pom dependency gate (stdlib+coroutines only) | R8 shrink measurement |
| Java | per-operation classes + R8/JVM | `javap` consumer-unit gate (1 529 B unit, byte-identical) + zero-dependency pom gate | R8 shrink measurement |
| C# | IL trimming | `PublishTrimmed` gate: unrelated ops trimmed from `Client`; consumer refs gate; zero-PackageReference gate; +17 920 B publish delta | trim-annotated SDK (IL2104) re-measurement |
| Dart | AOT tree shaking + zero-dependency pubspec | structural emission gate green (launcher byte-identity: 8 per-contract files, 25 shared parts identical; zero-dep offline resolution); AOT consumer gate **blocked** — emitted package fails `dart analyze` (4 emission defects recorded, 1 universal); AOT behavior measured on repaired copies: **0 B additivity** for one-op/codec-only, function-level code shedding, validation-program data retention (size-neutral) recorded | emission-defect fixes, then unignore `dart_consumers_eliminate_unrelated_operations` |
| C++ | `--gc-sections` / `-dead_strip` | `cmake`/`clang++` consumer binaries: codec-only sheds everything (byte-identical, 16 840 B); dead-strip sheds all unrelated operation code (identical 778-symbol table); schema codec-data retention (+16 512 B) recorded | — |
| PHP | conditional emission + autoload | Composer classmap gate + `get_included_files()` reachability (codec consumer sheds the whole client surface) | — |
| Ruby | conditional emission + require | gemspec dependency gate + `$LOADED_FEATURES` reachability (codec consumer sheds the whole client surface) | — |

Reproduce the gates with:

```sh
cargo test -p suspect-codegen --test typescript_elimination -- --nocapture
cargo test -p suspect-codegen --test swift_elimination -- --nocapture
cargo test -p suspect-codegen --test cpp_elimination -- --nocapture
cargo test -p suspect-codegen --test java_elimination -- --nocapture
cargo test -p suspect-codegen --test kotlin_elimination -- --nocapture
cargo test -p suspect-codegen --test csharp_elimination -- --nocapture
cargo test -p suspect-codegen --test ruby_elimination -- --nocapture
cargo test -p suspect-codegen --test php_elimination -- --nocapture
cargo test -p suspect-codegen --test dart_elimination -- --nocapture
```

The per-language gates retain their native artifacts under
`target/sdk-<language>-elimination/` (generated packages, consumer projects,
command logs) and write the machine-readable measurement records to
`target/tmp/<language>-elimination-report.json`. The TypeScript gates write
`target/tmp/typescript-elimination-report.json` with the bundled consumer
entries, per-profile bytes, gzip sizes, resolved module graphs, graph-model
reachability sets and the unrelated-operation markers.
