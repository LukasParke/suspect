# Swift SDK: selected-operation M3 slice

This document retains the original slice and toolchain evidence. Later additive
coverage is documented in [Swift HTTP protocol](SDK-SWIFT-PROTOCOL.md),
[scoped validation v2](SDK-SWIFT-VALIDATION-V2.md), and
[resources/dynamic references and physical server bases](SDK-SWIFT-RESOURCES.md).

The Swift backend emits a working Swift 6 package from the canonical contract.
Swift is part of the **2026-09-09 M3 priority**, alongside Python, Go and Rust.
The verification inputs are the original
[`canonical.openapi.yaml`](../crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml)
and these five operations in the actual `openrouter-web` checkout:

- `getCredits`
- `createKeys`
- `updateKeys`
- `listContainerFiles`
- `getContainerFile`

## Entry points and artifacts

```sh
suspect codegen crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile swift-http --package-name GeneratedSDK --package-version 0.1.0 \
  --out generated-swift
```

The CLI/session backend roots the package at `generated-swift/swift/`. Add
`--check --format json` for read-only drift inspection. Multi-target generation
and watch use [the shared session configuration](SDK-INCREMENTAL-GENERATION.md).

`suspect_codegen::swift_sdk::plan_sdk(contract, selected_sources, config)` performs
shared HTTP admission, owned validation compilation and native model planning.
`emit_sdk(&plan, &PackageConfig)` returns a complete artifact set. Admission
failures have original source URIs, JSON Pointers and byte spans; they return
before any artifacts are available to the writer.

The package contains:

- `Package.swift`, an importable library and compiled example tests;
- value models, exact JSON/model codecs, the portable validation executor,
  async operations and the URLSession/injectable transport;
- a DocC catalog with installation, compiled call sites, operation reference,
  reachable model schemas, constraints and runtime policy;
- `sdk-manifest.json`, `validation-program.json` and `example-coverage.json`,
  recording symbols, original sources, policy and declared/synthesized origins.

Package identity is separate from API semantics. The manifest uses
`swift-tools-version: 6.0` and Swift 6 language mode. Its Apple deployment floor
is macOS 13, iOS/tvOS 16 and watchOS 9. Verified toolchains are **Apple Swift
6.3.3** (current, `/usr/bin/swift`) and the official **Swift 6.0.3 release**
(6.0-family floor, paired with the existing macOS 15.4 SDK), on arm64 macOS.
No third-party runtime dependency is required. The original unpatched Swift 6.0
release's SDK failure evidence is retained separately below. Other operating
systems have not been executed in this gate.

## Native usage

These names are emitted from the unchanged M2 fixture:

```swift
import GeneratedSDK

let client = Client(credentials: Credentials(apiKey: runtimeToken))
var body = WidgetInput(name: "alpha")
body.amount = .value(try JsonNumber("9007199254740993.000000000000000001"))

switch try await client.createWidget(CreateWidgetInput(body: body)) {
case .status200(let response):
    print(response.data.amount.raw)
}
```

All models and operation inputs have value semantics. Struct properties are
mutable; encoding revalidates their **current** values. String literals become
native enums, and declared unions become indirect enums with typed payloads.
No union branch is selected merely because its tag looks right: the compiled
source assertions are evaluated. `oneOf` requires exactly one match; `anyOf`
decodes to the first valid alternative in source order. Encoding a selected
variant checks that variant as well as the enclosing union.

| Source field | Swift representation |
| --- | --- |
| Required, non-null | `T` |
| Required, nullable | `Nullable<T>`: `.null`, `.value(T)` |
| Optional, non-null | `OptionalField<T>`: `.missing`, `.value(T)` |
| Optional, nullable | `Presence<T>`: `.missing`, `.null`, `.value(T)` |

Arbitrary JSON and null-only schemas carry null in their own `JsonValue` or
`JsonNull` domain. Optional fields in those domains retain a missing/present
wrapper. No default is inserted. Single-valued string tags can have a
source-backed initializer default. Recursive fields use indirect value enums;
required object-layout feedback edges receive `Indirect<T>`.

Open objects retain every undeclared property in a typed `JsonObject<T>`.
Closed objects reject extras. A caller cannot overwrite a declared field by
placing its wire name in `additionalProperties`, even when that field is missing.
Wire names and native names are distinct, with deterministic collision allocation.

## Exact JSON and Codable boundary

```swift
let encoded = try WidgetInput.codec.encode(body)
let decoded = try SDKJSONDecoder().decode(WidgetInput.self, from: encoded)
let roundTrip = try SDKJSONEncoder().encode(decoded)
```

`JsonNumber` retains the original validated number token. `JsonInteger` asserts
mathematical integrality without narrowing and offers checked `int64Value()`.
Integer literals are supported; floating-point literals are not implicitly
accepted as exact JSON numbers. Exponents remain symbolic decimal integers, so
large exponent magnitudes do not cause zero-padding allocations.

Generated models conform to `Codable` through a **source-preserving SDK
boundary**. Foundation `JSONDecoder` and `JSONEncoder` are explicitly rejected
by those adapters. Their generic containers cannot recover arbitrary original
number tokens. The SDK decoder/encoder and source-bound `ModelCodec` implement
the actual conversion; there is no `NSNumber`/`Double` JSON intermediate.

Number token spellings and Unicode scalar sequences survive round trips.
Whitespace, object order and string escape spellings may change. JSON object
keys compare by UTF-8 bytes, so `é` and `e\u0301` remain distinct names despite
Swift String's canonical-equivalence equality. Duplicate *decoded* keys,
invalid UTF-8, unpaired surrogates, invalid numbers and trailing input are
rejected. Native `Equatable` follows the native value types; schema equality is
implemented separately with exact mathematical number comparison.

## Portable validation and resource policy

The generator uses `OwnedCompiler` and checks its `OwnedProgram` before lowering
the instructions into immutable Swift tables. The Swift executor implements
the complete checked program instruction set: types, static references,
object/array applicators, composition, exact bounds/divisibility, cardinality,
literal equality, uniqueness and the shared portable pattern NFA. It does not
interpret source schemas at runtime or substitute Foundation regular expressions.

The executor distinguishes completed `invalid` from `evaluationFailure`.
Recursive nonproductive references and exhausted evaluation/equality/numeric
budgets remain failures inside `anyOf`, `oneOf` and `not`. Diagnostics retain the
responsible source keyword and instance pointer. The public throwing API reports
the first mismatch; logical evaluation still observes all necessary branches.

Default limits are:

| Boundary | Default |
| --- | --- |
| Response, assembled request URL, serialized request body | 8 MiB each |
| Error raw-body capture | 8 MiB; configurable at generation |
| Raw response headers | 256 fields, 64 KiB total |
| JSON input/output | 8 MiB; HTTP codecs use generated byte ceilings |
| JSON/conversion depth and node count | 128 levels, 100,000 nodes |
| JSON number token | 4,096 bytes per codec call |
| Validation depth / operand bytes | 128 / 4,096 |
| Validation visits / equality visits / numeric work | 100,000 each |

`JsonLimits` can be passed to direct codecs. Generation rejects unsupported
validation metadata limits and non-positive or greater-than-`Int32.max` HTTP
ceilings. Collection sizes are checked before native conversion allocation or
object-key sorting; path/query construction is bounded incrementally. Limits
bound the SDK's work and output, rather than promising a fixed process RSS.

## HTTP and concurrency

The shared `http_contract` determines methods, paths, effective servers,
security, parameter serialization, body requiredness and response schemas.
The admitted profile has one canonical static HTTPS server, one bearer scheme,
simple string path parameters, form scalar/scalar-array queries,
`application/json` bodies/responses and exact status codes.

`Client` and all its stored configuration/models are `Sendable`. An injected
`HTTPTransport` must also be `Sendable`. The default `URLSessionTransport` uses
an ephemeral session for each transfer, disables cookie/cache/credential storage,
rejects redirects and bounds response accumulation in delegate callbacks,
including responses without Content-Length. It invalidates its session on
completion, error, size rejection or cancellation. No operation retries or
pagination loops are inferred.

The one `@unchecked Sendable` type is the private URLSession delegate state
holder. Its mutable state is protected by a lock; continuations and external
callbacks execute outside that lock. Cancellation arriving before continuation
or task installation is retained. The caller receives native
`CancellationError`, including cancellation of in-flight real socket I/O.

`ClientOptions` configures the server override, whole-transfer timeout and lower
response ceiling. `RequestOptions` supplies a per-call timeout and may further
lower that ceiling. Cleartext overrides are restricted to loopback fixtures.
Custom transports must implement cancellation, timeouts, redirect and byte-limit
policy; the client checks cancellation and verifies returned status/header/body
bounds as well.

Successful alternatives return an operation-specific result enum. Declared
errors throw an operation-specific `APIError` enum containing the source-typed
payload. `SDKError` covers request validation/representation, malformed or
unexpected responses, limits, timeout and transport failures. Raw success data
and capped error captures are accessible, with truncation explicit. Error
descriptions do not print credentials or response bodies.

## Verified gates

The tests are in
[`tests/swift_sdk.rs`](../crates/suspect-codegen/tests/swift_sdk.rs) and the private
Swift validation module. Required native gates fail if their toolchain/corpus is
missing; their `ignore` annotations make native verification explicit.

The Swift 6.3.3 execution on 2026-09-09: **13/13 integration tests passed**, including
both native package/consumer/DocC gates, and **1/1 shared native validation gate
passed**. Standalone type probes use `swiftc`, first require a successful positive
control, and then require five real type-check failures.

The **6.0.3 + SDK 15.4 floor run also passed both native SDK gates and the shared
native validation gate**. This is the tested 6.0 patch tier; the package's
declared tools/language baseline remains 6.0.

```sh
export CARGO_TARGET_DIR="${TMPDIR%/}/opencode/swift-integration-cargo"
cargo test --locked -p suspect-codegen --test swift_sdk
cargo test --locked -p suspect-codegen --test swift_sdk -- --ignored --nocapture
cargo test --locked -p suspect-codegen --lib \
  swift_sdk::validation::tests::native_shared_runtime_contract_vectors \
  -- --ignored --nocapture
```

### Original Swift 6.0 release probe (retained)

The exact official **6.0** release was downloaded from
[download.swift.org](https://download.swift.org/swift-6.0-release/xcode/swift-6.0-RELEASE/swift-6.0-RELEASE-osx.pkg)
and extracted with `pkgutil --expand-full` into isolated temporary tooling.
The [official release catalog](https://www.swift.org/api/v1/install/releases.json)
identifies `swift-6.0-RELEASE`, dated 2024-09-16, with Xcode 16. Package identity
is `org.swift.600202409101a`, version `6.0.20240910101`.

Verification:

- Downloaded package SHA-256:
  `2b53b7ceaadded915213a7155131e0f14bef8e766859f88c68d128822058d159`.
  This is a locally recorded artifact identity. The adjacent `.pkg.sha256` URL
  returned 404; no separately published checksum is claimed.
- `pkgutil --check-signature` reported a trusted Apple notarization and timestamp
  `2024-09-13 18:45:03 +0000`, signed by
  `Developer ID Installer: Swift Open Source (V9AUD2URP3)`, matching the
  [official macOS signing instructions](https://www.swift.org/install/macos/package_installer/).
- `codesign --verify --strict` accepted the extracted `swift-frontend`.
- The extracted binaries report
  `Apple Swift version 6.0 (swift-6.0-RELEASE)`.
- A Foundation-free executable compiled the unchanged `number.swift` with that
  compiler and passed exact comparison, signed-zero, divisibility and 4,000-digit
  zero-padded exponent probes. This is a numeric/compiler smoke check, not the
  complete native validation or SDK gate.

**Original-release SDK blocker:** both installed SDK candidates fail even a small
standalone `import Foundation` program under this compiler, targeting macOS 13.

| Existing SDK | Observed Swift 6.0 result |
| --- | --- |
| `/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk` | Compiler signal 6 during Darwin `MandatorySILLinker`; `POSIXErrorCode.init(rawValue:)` deserialization mismatch through `_errno`, built from the SDK swiftinterface into a fresh isolated module cache |
| `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk` | The same Darwin function deserialization failure through `_DarwinFoundation1`, using a separate fresh cache |

The existing 15.4 SDK identifies build `24E241`; its Foundation interface reports
Apple Swift 6.1. Its installed file receipt is
`com.apple.pkg.CLTools_SDK_macOS_LMOS`, version `26.6.0.0.1781586411`.
The actual M2 SwiftPM gate was then run with explicit Swift 6.0 `SWIFT_EXEC` and
the 15.4 SDK. It reached generated source compilation and reproduced the same
Darwin SDK crash. Native M2 execution, five-operation execution, standalone type
checks, full portable validation and DocC promotion under the original unpatched
6.0 release were blocked by that prerequisite. The subsequent 6.0.3 floor results
below are separate evidence.

The declared tools/language baseline remains 6.0. No SDK files or installer
scripts were modified or activated globally. These original 6.0 logs remain a
record of the observed failure; the patch-tier verification below does not
reinterpret that failure as a pass.

Tooling and complete failure logs are retained locally under
`${TMPDIR%/}/opencode/swift-6.0-toolchain/`:
`PROVENANCE.md`, `foundation-sdk15.4.log`, `foundation-sdk26.5.log`, and
`m2-floor.log`. The generated failed-floor fixture is `gates/m2-tJUkcW`.

#### Bounded matching-SDK inventory

A follow-up inventory found Xcode **26.6** in `/Applications/Xcode.app` and no
Xcode 16 bundle in either application directory. Available SDKs remain 15.4
(`24E241`) and 26.5 (`25F70`); no existing SDK 15.0/15.1 was found in the Xcode,
CommandLineTools, user Developer or approved tooling roots.

The official direct disk-image endpoints for
[CLT 16.0](https://download.developer.apple.com/Developer_Tools/Command_Line_Tools_for_Xcode_16/Command_Line_Tools_for_Xcode_16.dmg)
and
[CLT 16.1](https://download.developer.apple.com/Developer_Tools/Command_Line_Tools_for_Xcode_16.1/Command_Line_Tools_for_Xcode_16.1.dmg)
both redirected to `https://developer.apple.com/unauthorized/`. Their final
response was an HTML page, not an available SDK disk image. A bounded check of
Apple's macOS-14 merged software-update catalog returned 404. SDK acquisition
stopped at this boundary without credentials, third-party SDK mirrors or a full
Xcode download.

### Verified Swift 6.0 patch tier: 6.0.3

The latest official 6.0 patch release,
[Swift 6.0.3](https://download.swift.org/swift-6.0.3-release/xcode/swift-6.0.3-RELEASE/swift-6.0.3-RELEASE-osx.pkg),
was acquired and extracted into a separate isolated tooling directory.

- Package SHA-256:
  `764c3d5ba27473494206278c27d1bb0fdc3a8bca35ed902ed030f6699904e903`.
- `pkgutil --check-signature`: trusted Apple notarization, trusted timestamp
  `2024-12-11 23:08:18 +0000`, and the documented
  `Developer ID Installer: Swift Open Source (V9AUD2URP3)` identity.
- `codesign --verify --strict` accepted the extracted `swift-frontend`.
- Bundle `org.swift.603202412101a`, version `6.0.3.20241210101`.
- Driver and compiler report `Apple Swift version 6.0.3 (swift-6.0.3-RELEASE)`.
- Tiny Foundation and Darwin programs compiled and ran first, using
  `-swift-version 6`, target `arm64-apple-macosx13.0`, SDK 15.4 and separate fresh
  module caches.
- Both native SDK gates then passed with this exact compiler/SDK pairing:
  M2 and the five actual OpenRouter operations, independent consumers, actual
  loopback URLSession wire checks, positive-controlled negative type checks, and
  DocC from the same 6.0.3 toolchain.
- The full shared native validation corpus and adversarial budget gate passed.

The first 6.0.3 test invocation passed its XCTest cases but then failed in
SwiftPM's separate Swift Testing discovery helper while loading the newer
Xcode's private `XCTestCore` framework. All tests in these gates are XCTest.
The harness now uses the documented `--disable-swift-testing` selector to run
that framework explicitly. No test case, compiler diagnostic, validator check
or generated SDK behavior is disabled. Fresh full gates passed in this mode.
Integration scratch paths use `build/sdk` and `build/consumer`, avoiding
overlapping source/build path prefixes in older Swift debug-info paths.

Successful floor artifacts are retained under
`${TMPDIR%/}/opencode/swift-6.0.3-toolchain/`:

- `gates/m2-lgvm9X` — original M2 package, consumer, type probes and DocC archive;
- `gates/openrouter-zjAYTJ` — five-operation package, consumer and DocC archive;
- `sdk-floor-gates.log`, `validation-floor-gate.log`, and `PROVENANCE.md`;
- initial helper failures in `initial-sdk-gates-swift-testing-helper.log` and
  `initial-validation-swift-testing-helper.log`.

The native validation fixture is retained at
`${TMPDIR%/}/opencode/swift-validation-TIItqG`. The original `swift-6.0-toolchain`
directory and its failure reproductions remain available.

Native harness selectors (per invocation):

| Selector | Meaning |
| --- | --- |
| `SUSPECT_SWIFT_BIN` | Swift driver / SwiftPM executable |
| `SUSPECT_SWIFTC_BIN` | Matching compiler; defaults to the driver's sibling `swiftc` |
| `SUSPECT_SWIFT_SDKROOT` | SDK path passed explicitly to SwiftPM (`--sdk`), standalone `swiftc` (`-sdk`), and their `SDKROOT` environment |
| `SUSPECT_SWIFT_DOCC_BIN` | Matching DocC executable; defaults to `xcrun docc` |
| `SUSPECT_SWIFT_GATE_ROOT` | Isolated integration artifact/build directory |

The harness sets `SWIFT_EXEC` only on native subprocesses to the selected
compiler. Both the integration gates and portable validation unit gate honor
the compiler/SDK selectors. For the verified floor pairing:

```sh
floor="${TMPDIR%/}/opencode/swift-6.0.3-toolchain"
toolchain="$floor/expanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"
export SUSPECT_SWIFT_BIN="$toolchain/usr/bin/swift"
export SUSPECT_SWIFTC_BIN="$toolchain/usr/bin/swiftc"
export SUSPECT_SWIFT_DOCC_BIN="$toolchain/usr/bin/docc"
export SUSPECT_SWIFT_SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
export SUSPECT_SWIFT_GATE_ROOT="$floor/gates"
cargo test --locked -p suspect-codegen --test swift_sdk -- --ignored --nocapture
cargo test --locked -p suspect-codegen --lib \
  swift_sdk::validation::tests::native_shared_runtime_contract_vectors \
  -- --ignored --nocapture
```

Native verification performs:

1. SwiftPM build/test of the generated package with warnings as errors, including
   shared source-validated example round trips.
2. A separate SwiftPM consumer importing the generated library, with no repaired
   generated source.
3. All four original M2 operations through independently specified recording
   transports, plus positive/negative compile-time presence/union/input checks.
4. All five actual OpenRouter operations through both recording transports and
   real loopback URLSession exchanges. The independent TCP fixture checks
   method, path/query escaping, headers and exact request body bytes.
5. Real URLSession redirect refusal, chunked response limits, timeout, in-flight
   cancellation and bounded declared/undeclared error captures.
6. Exact JSON, mutable encode validation, source-tagged unions, recursion,
   required/null/presence, extra-property collision and malformed response checks.
7. The shared `runtime-contract-v1.json` validation corpus, plus nonproductive
   references inside unions/negation, Unicode equality, huge cardinalities and
   budget exhaustion. This independently checks the validator beyond the native
   model planner's admitted shapes.
8. SwiftPM public symbol graph extraction and DocC conversion with warnings as
   errors; the rendered `.doccarchive/index.html` is required.

Host checks cover source-linked rejection, package identity, naming reservations,
repeat emission and an operation-description-only edit. That edit changes just
the operation doc comments and operation reference page; executable text,
models, codecs, validation tables and package identity remain byte-identical.
The shared generation session/artifact writer owns the broader M6 lifecycle.
Within the crate, `SdkPlan::models()` exposes immutable typed declarations,
native names/type bindings, codec symbols, field requiredness, typed initializer
defaults, constructor order and union/literal cases for compatibility analysis.
This interface uses planned descriptors rather than parsing emitted Swift.

### Independent review regressions

Two native package/client regressions lock down the final Swift review findings:

- **Example locals:** required parameters named `client`, `body`, `input`,
  `_parameter0` and `request_body` generate and execute a real `Examples` call.
  The regression reproduced `String has no member 'echoNames'` before the fix.
  Example locals now use an independent indexed namespace and a separate request
  body local; the native public input labels are preserved and exercised by a
  second direct client call. Guides and compiled examples share this lowering.
- **Query amplification:** 10,000 short array values with a 4-KiB wire name
  previously materialized 40,960,000 bytes of repeated keys before the 8-MiB
  guard. The native client regression observed 53,723,136 bytes of peak RSS
  growth before the fix, even though the request eventually failed.
  Serialization now checks minimum repeated-key/separator cost before allocating
  pairs, sizes each percent-encoded value against the shared remaining budget,
  and sizes comma-joined arrays without retaining an encoded-string array.
  The query budget accounts for the final server/path prefix and the question
  mark, then remains shared across parameters.

The regressions passed on both Swift 6.3.3 and the verified Swift 6.0.3 + SDK 15.4
pairing. The observed amplification-case peak RSS increases were **851,968 bytes**
on current and **753,664 bytes** on the floor. The prior-parameter budget case
grew by 262,144 and 49,152 bytes respectively. These are isolated process
high-water-mark measurements after warm-up, not claims of exact allocation counts
or a general performance budget. The test's conservative 24-MiB amplification
threshold rejected the pre-fix behavior. It also checks that oversized requests
never reach transport, that a ~3-MiB JSON array cannot allocate its ~9-MiB URI
expansion, and that escaping, order, exact numeric tokens and optional empty
arrays keep their existing wire behavior.

Run just these package regressions with the native selectors above:

```sh
cargo test --locked -p suspect-codegen --test swift_sdk native_review_ \
  -- --ignored --nocapture
```

`SUSPECT_SWIFT_REVIEW_ROOT` optionally selects a stable isolated artifact directory
for incremental repro runs. The local red/current evidence is under
`${TMPDIR%/}/opencode/swift-review-current/`; floor evidence is under
`${TMPDIR%/}/opencode/swift-review-floor/`. The original M2/five-operation gates
remain their prior independent evidence; the focused review checks exercise
these changes without repeating the full native matrix.

## Original slice boundaries (historical)

The original gate verified a selected-operation JSON HTTP profile. Its unsupported schema
layouts fail before emission, including general intersections/ref assertion
siblings, tuple models, multiple non-null `type` array members, untyped
conditional layouts and boolean-false model values. The supported `allOf` proof
admits a reference carrier with redundant overlays or strengthened requiredness
of already-declared fields. The portable executor independently supports its
full instruction set even when a native model layout is rejected.

At that checkpoint, active `readOnly`/`writeOnly`, unsupported dialects/vocabularies, dynamic refs,
other HTTP media/security shapes, typed response headers/links, range/default
statuses, uploads and streamed API media were
outside that slice and produced source-linked admission findings. Unconstrained
JSON legitimately uses `JsonValue`; unsupported schemas do not fall back to it.
Service workflows are not inferred from operation or property names.

The 6.0 family floor is verified at patch release 6.0.3. Additional-platform
runs remain separate release-matrix evidence. Native rendered docs and executable examples are present; this gate
does not certify a numerical performance budget or full OpenAPI coverage.
