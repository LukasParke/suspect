# Swift SDK: HTTP client

This document retains the original slice and toolchain evidence. Later additive
coverage is documented in [Swift HTTP protocol](protocol.md),
[scoped validation v2](validation-v2.md), and
[resources/dynamic references and physical server bases](resources.md).

The Swift backend emits a working Swift 6 package from the canonical contract.
Swift is part of the **2026-09-09 M3 priority**, alongside Python, Go and Rust.
The verification inputs are the original
[`canonical.openapi.yaml`](../../../crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml)
and these five operations in the actual `openrouter-web` checkout:

- `getCredits`
- `createKeys`
- `updateKeys`
- `listContainerFiles`
- `getContainerFile`

## Entry points and artifacts

```sh
suspect gen sdk crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile swift-http --package-name GeneratedSDK --package-version 0.1.0 \
  --output generated-swift
```

The CLI/session backend roots the package at `generated-swift/swift/`. Add
`--check --format json` for read-only drift inspection. Multi-target generation
and watch use [the shared session configuration](../../guide/sdk-incremental-generation.md).

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
6.3.3** and the official **Swift 6.0.3 release** (6.0-family floor, paired with
the macOS 15.4 SDK), on arm64 macOS. No third-party runtime dependency is
required. Other operating systems are not exercised by the native tests.

## Native usage

These names are emitted from the canonical fixture:

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

Swift HTTP planning consumes the shared `http_protocol::ProtocolPlan`
([SDK-HTTP-PROTOCOL.md](../../design/http-protocol.md)). The explicit adapter identity is
**`swift-http-protocol-v1`**; the generator lists individual capabilities and
never opts into `Capability::ALL`.

| Area | Native behavior |
| --- | --- |
| Versions and methods | OAS 3.0, 3.1 and 3.2; named and unnamed operations; fixed methods and exact case-sensitive `additionalOperations` tokens, including `get`, `GeT`, `head`, `COPY`, `MiXeD`, `x-PING` and `pOsT`. |
| Servers | Ordered effective choices, relative URLs, default `/`, variable defaults/enums and explicit overrides. Relative local-file descriptions require a caller-supplied HTTP document URL. |
| Security | Undeclared, disabled, anonymous, OR and AND requirements; case-insensitive bearer/basic scheme names; header/query/cookie API keys; distinct roles/scopes; OAuth/OIDC caller hooks. Conflicting attachments decline before artifacts or fail before transport. |
| Parameters | Scalar, scalar-array and flat-object simple/label/matrix/form/spaceDelimited/pipeDelimited/deepObject; headers, cookie form and OAS 3.2 cookie style; JSON/plain-text content; reserved expansion and UTF-8 escaping. Nested/null/ambiguous wire values are guarded. Object key order and identity use UTF-8 bytes. |
| Whole query | OAS 3.2 JSON and UTF-8 text with one URI-component encoding pass and no name prefix; form-urlencoded from the typed native object through the aggregate and field codecs, with no second encoding pass. Empty-present and missing queries remain distinct. |
| Request media | JSON and structured `+json`, schema-free JSON, UTF-8 scalar text, finite bytes, form-urlencoded and named multipart. Concrete Content-Type must select the same typed body case; wildcard bytes cannot bypass a more specific JSON schema. |
| Responses | Exact > class > default before media matching. Concrete media > type wildcard > `*/*`, then matching parameter count. Success is determined from the actual 200–299 status. Media mismatch does not fall through to another status declaration; content is not sniffed. |
| Body suppression | HEAD, 1xx, 204, 205 and 304 expose no decoded/raw body. No declared content otherwise means bounded unspecified bytes, not an empty-body assertion. |
| Headers and links | Required typed scalar/array/flat-object/content headers, unchanged raw fields, precise header failure sources, and link literals/expressions/targets/provenance as metadata. Links trigger no calls. |
| Parts | Separate aggregate and per-part ceilings; required/extras/property counts; named repeated fields; typed positional prefix slots and remaining items; item cardinality/codecs; binary equality including zero/non-UTF8 octets; typed part headers; explicit media choices; non-expansive MIME style values; bounded request/response parsing. Preamble/epilogue are ignored. |
| Streams | Standard OAS 3.2 `itemSchema`, parsed SSE envelopes, `application/jsonl` and `application/x-ndjson`; arbitrary chunk and UTF-8 splits, CR/LF framing, comments, multiline data, numeric retry, id handling, codec errors and bounded captures. |

### Server and credential policy

`ClientOptions` and `RequestOptions` select a server index, variables and document
URL. A full `serverURL` override is also available. Declared HTTP servers retain
their scheme; overrides require HTTPS or loopback HTTP unless `allowHTTP` is set
([ADR 0035](../../adr/0035-one-server-url-policy.md)). Literal dot segments are refused.
Invalid names, enum values, URL characters and path traversal fail before I/O.

Security defaults to the first fully configured alternative, including an
anonymous alternative in its declared position. `securityAlternative` selects an
explicit alternative. AND members are all required. Credentials stay named by
their source schemes; basic credentials take username/password values and use
UTF-8 before base64 encoding.

OAuth/OIDC providers are `@Sendable` async closures receiving
`HTTPCredentialContext`: scheme/requirement provenance, permissions, flow URLs,
scope metadata and discovery/metadata URLs. They return a **complete Authorization
field**, so token type is caller-selected. A generated token lifecycle is added
only through an `sdk_defaults` OAuth policy
([golden defaults](../../guide/sdk-generation.md#5-oauth-lifecycle)).

### Stream limits

`HTTPEventStream<T>` is a single-consumer AsyncSequence that validates each parsed
item with the exact source-bound codec. Early break, exhaustion, failure,
cancellation and idempotent `close()` release the transfer; URLSession's delegate
uses demand-driven suspension, a finite queue and a total received-byte ceiling,
and overflow is an explicit error.

| Generation policy | Default |
| --- | --- |
| Assembled request URL and serialized request body | 8 MiB each |
| Received response / complete stream | 8 MiB |
| Each in-memory part | 8 MiB |
| Each parsed/framed stream item | 1 MiB |
| Queued stream transport bytes | 64 KiB |
| Raw failure capture | 8 MiB, independently configurable |
| Headers | 256 fields / 64 KiB |
| Timeout | 60 seconds, positive finite values up to one day |

### Transport

`Client` and all its stored configuration/models are `Sendable`. An injected
`HTTPTransport` must also be `Sendable`. The default `URLSessionTransport` uses
an ephemeral session for each transfer, disables cookie/cache/credential storage,
rejects redirects and bounds response accumulation in delegate callbacks,
including responses without Content-Length. It invalidates its session on
completion, error, size rejection or cancellation. No operation retries
([ADR 0025](../../adr/0025-one-http-exchange-per-call.md)); pagination is generated only
through an `sdk_defaults` policy.

The one `@unchecked Sendable` type is the private URLSession delegate state
holder. Its mutable state is protected by a lock; continuations and external
callbacks execute outside that lock. Cancellation arriving before continuation
or task installation is retained. The caller receives native
`CancellationError`, including cancellation of in-flight real socket I/O.

`ClientOptions` configures the server override, whole-transfer timeout and lower
response ceiling. `RequestOptions` supplies a per-call timeout and may further
lower that ceiling.
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
[`tests/swift_sdk.rs`](../../../crates/suspect-codegen/tests/swift_sdk.rs) and the private
Swift validation module. Required native gates fail if their toolchain/corpus is
missing; their `ignore` annotations make native verification explicit.
Standalone type probes use `swiftc`, first require a successful positive
control, and then require five real type-check failures.

```sh
export CARGO_TARGET_DIR="${TMPDIR%/}/suspect-swift-integration-cargo"
cargo test --locked -p suspect-codegen --test swift swift_sdk::
cargo test --locked -p suspect-codegen --test swift swift_sdk:: -- --ignored --nocapture
cargo test --locked -p suspect-codegen --lib \
  swift_sdk::validation::tests::native_shared_runtime_contract_vectors \
  -- --ignored --nocapture
```

### Toolchain floor

The declared tools/language baseline is 6.0; the tested 6.0 patch tier is
**6.0.3** with the macOS 15.4 SDK. The original unpatched Swift 6.0 release is
not usable with the available macOS 15.4 and 26.5 SDKs: even a standalone
`import Foundation` program crashes the compiler during Darwin SDK module
deserialization.

All tests in these gates are XCTest. The harness passes
`--disable-swift-testing` so SwiftPM does not run its separate Swift Testing
discovery helper, which fails on 6.0.3 while loading a newer Xcode's private
`XCTestCore` framework. Integration scratch paths use `build/sdk` and
`build/consumer`, avoiding overlapping source/build path prefixes in older Swift
debug-info paths.

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
the compiler/SDK selectors. For example, with an extracted 6.0.3 toolchain and
the 15.4 SDK:

```sh
floor="${TMPDIR%/}/suspect-swift-6.0.3-toolchain"
toolchain="$floor/expanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"
export SUSPECT_SWIFT_BIN="$toolchain/usr/bin/swift"
export SUSPECT_SWIFTC_BIN="$toolchain/usr/bin/swiftc"
export SUSPECT_SWIFT_DOCC_BIN="$toolchain/usr/bin/docc"
export SUSPECT_SWIFT_SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
export SUSPECT_SWIFT_GATE_ROOT="$floor/gates"
cargo test --locked -p suspect-codegen --test swift swift_sdk:: -- --ignored --nocapture
cargo test --locked -p suspect-codegen --lib \
  swift_sdk::validation::tests::native_shared_runtime_contract_vectors \
  -- --ignored --nocapture
```

Native verification performs:

1. SwiftPM build/test of the generated package with warnings as errors, including
   shared source-validated example round trips.
2. A separate SwiftPM consumer importing the generated library, with no repaired
   generated source.
3. All four canonical-fixture operations through independently specified recording
   transports, plus positive/negative compile-time presence/union/input checks.
4. All five OpenRouter operations through both recording transports and
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
The shared generation session/artifact writer owns the broader artifact lifecycle.
Within the crate, `SdkPlan::models()` exposes immutable typed declarations,
native names/type bindings, codec symbols, field requiredness, typed initializer
defaults, constructor order and union/literal cases for compatibility analysis.
This interface uses planned descriptors rather than parsing emitted Swift.

### Native regressions

Two native package/client regressions cover example naming and query budgeting:

- **Example locals:** required parameters named `client`, `body`, `input`,
  `_parameter0` and `request_body` generate and execute a real `Examples` call.
  Example locals use an independent indexed namespace and a separate request
  body local; the native public input labels are preserved and exercised by a
  second direct client call. Guides and compiled examples share this lowering.
- **Query amplification:** 10,000 short array values with a 4-KiB wire name
  would materialize ~40 MB of repeated keys if serialized before the 8-MiB
  guard. Serialization checks minimum repeated-key/separator cost before allocating
  pairs, sizes each percent-encoded value against the shared remaining budget,
  and sizes comma-joined arrays without retaining an encoded-string array.
  The query budget accounts for the final server/path prefix and the question
  mark, then remains shared across parameters.

The test bounds isolated process peak RSS growth with a conservative 24-MiB
amplification threshold; this is a regression guard, not an allocation count or
a performance budget. It also checks that oversized requests
never reach transport, that a ~3-MiB JSON array cannot allocate its ~9-MiB URI
expansion, and that escaping, order, exact numeric tokens and optional empty
arrays keep their existing wire behavior.

Run just these package regressions with the native selectors above:

```sh
cargo test --locked -p suspect-codegen --test swift native_review_ \
  -- --ignored --nocapture
```

`SUSPECT_SWIFT_REVIEW_ROOT` optionally selects a stable isolated artifact directory
for incremental repro runs.

## Limits

Under the v1 validation program, unsupported schema layouts fail before
emission, including general
intersections/ref assertion siblings, tuple models, multiple non-null `type`
array members, untyped conditional layouts and boolean-false model values
([ADR 0010](../../adr/0010-refuse-what-cannot-be-represented.md)). The supported
`allOf` proof admits a reference carrier with redundant overlays or strengthened
requiredness of already-declared fields. Programs v2 and later carry tuple and
untyped-assertion shapes as checked `JsonValue` values instead. The portable executor independently
supports its full instruction set even when a native model layout is rejected.

Unconstrained JSON legitimately uses `JsonValue`; unsupported schemas do not
fall back to it. Service workflows are not inferred from operation or property
names. The native tests do not certify a numerical performance budget or full
OpenAPI coverage.
