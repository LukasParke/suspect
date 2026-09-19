# Swift shared HTTP protocol and native DX

Updated 2026-09-10. Swift HTTP planning now consumes the admitted
`http_protocol::ProtocolPlan` directly. This supersedes the strict HTTP profile
described in the original [Swift milestone document](SDK-SWIFT.md); that document's
model, exact-number, validation and toolchain contracts remain applicable.

The later [resource-profile handoff](SDK-SWIFT-RESOURCES.md) records verified
`DocumentRelativeServers`, `SchemaResources` and `DynamicSchemaReferences`
adoption, including physical/logical metadata and current/floor native evidence.
The protocol matrices below retain their original results.

## One source-backed pipeline

```rust,ignore
let plan = swift_sdk::plan_sdk(contract, &selected_sources, SwiftConfig::default())?;
let wire = plan.protocol();
let roots = wire.codec_roots();
let files = swift_sdk::emit_sdk(&plan, &PackageConfig::default())?;
```

The native plan retains original operation/parameter/body/response descriptors,
schema use-site roots, HTTP declaration and terminal provenance, and reference
hops. It feeds the real JSON/text/item/header roots into the existing owned
validator and Swift model/codec planner. Byte schemas and mixed multipart
aggregates are metadata and structural rules, not JSON codec roots. HEAD and
body-forbidden response declarations do not force unused body models.

The example stage is `examples::plan_protocol_examples`, including its v2
response-pattern, header, part and item roles. It does not rerun strict HTTP
admission. Executable examples and DocC quickstarts lower validated values into
direct Swift constructors. Binary examples use explicit `Data` recipes;
schema-free JSON uses an explicit `JsonValue` construction. Neither is represented
by a substitute JSON null passed to a byte/aggregate codec.

`SdkPlan::protocol()`, operation `protocol()` / `body()` / `responses()`, and the
native `PlannedBody`, `PlannedMedia`, `PlannedParts`, `PlannedPart`,
`PlannedResponse`, `PlannedHeader`, `PlannedPositionalParts` and `PlannedQueryForm`
descriptors are available for inspection.
`credential_property(&SourceId)` resolves an allocated credential member using
the scheme's source-document-scoped identity. Compatibility capture consumes
these types, including constructor order, missing defaults, part/header types,
media cases, positional constructors, whole-query encodings, response memberships
and credential attachments. Its configuration comes from the canonical
`NativeSnapshot.generation` through `backend::swift_options`. It does not parse
emitted Swift. Model/validation/JSON/number implementations were not changed.

## Native call sites

The following names come from the independent protocol fixture in
`tests/swift_protocol.rs`:

```swift
import GeneratedSDK

let client = Client()
let response = try await client.publicValue()
print(response.data.ok)

let body = Payload(name: "direct",
                   amount: .value(try JsonNumber("1.00e1000")))
_ = try await client.sendBody(SendBodyInput(body: .json(body)))

let events = try await client.events()
for try await event in events.data {
    print(event.data) // SSE data is a string, including JSON-looking text.
    break            // Closes the transfer even while `events` remains alive.
}
```

Inputs whose fields are all optional have a default argument, so empty input
construction is unnecessary. A sole successful response exposes `.data`,
`.status`, `.headers`, `.rawBody`, `.contentType`, `.declaredContentType` and
`.links` directly. The existing `.status200(...)` cases remain usable by current
consumers. Multiple statuses/media are explicit enums. Responses with declared
headers additionally expose `.typedHeaders`.

Multipart constructors put required fields first. File values are finite Data:

```swift
let file = UploadMultipartFilePart(
    value: Data([0, 255, 128]),
    headers: UploadMultipartFileHeaders(xSize: 3),
    filename: "sample.bin"
)
let upload = UploadMultipartBody(file: file, title: HTTPPart("sample"))
_ = try await client.upload(UploadInput(body: upload))
```

## Verified capability surface

The explicit adapter identity is **`swift-http-protocol-v1`**. The generator lists
individual capabilities; it never opts into `Capability::ALL`.

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
their scheme; overrides require HTTPS or loopback HTTP unless `allowHTTP` is set.
Invalid names, enum values, URL characters and path traversal fail before I/O.

Security defaults to the first fully configured alternative, including an
anonymous alternative in its declared position. `securityAlternative` selects an
explicit alternative. AND members are all required. Credentials stay named by
their source schemes; basic credentials take username/password values and use
UTF-8 before base64 encoding.

OAuth/OIDC providers are `@Sendable` async closures receiving
`HTTPCredentialContext`: scheme/requirement provenance, permissions, flow URLs,
scope metadata and discovery/metadata URLs. They return a **complete Authorization
field**, so token type is caller-selected. No acquisition, refresh, discovery,
scope authorization decision or retry is inferred.

### Streaming lifetime and limits

`HTTPEventStream<T>` is a single-consumer AsyncSequence. It validates each parsed
item with the exact source-bound codec. SSE `data` remains a string; neither JSON
inside it nor `[DONE]` has special behavior. Event fields are represented when
applicable, the last valid id persists, malformed retry fields and NUL-containing
ids are ignored, and incomplete SSE events at EOF are not dispatched. JSON lines
accept a final complete JSON value without a newline and reject empty/malformed
lines and record-separator/sentinel shortcuts.

Iterator lifetime leases close streams on early break, exhaustion and failure.
Task cancellation promptly releases real URLSession I/O and remains
`CancellationError`. Explicit `close()` is idempotent. URLSession's delegate uses
demand-driven suspension, a finite queue and a total received-byte ceiling.
Buffer overflow is an explicit error, never dropped events or an unbounded queue.
The native tests observe server-side socket closure on break, cancellation,
timeout, queue overflow and total-limit rejection.

`HTTPTransport.open` supports injected pull-based bytes. A send-only custom
transport has a finite buffered compatibility implementation; overriding `open`
is required to expose real chunks. Custom transports remain responsible for their
timeout/cancellation/redirect contract, and clients recheck returned bounds.

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

Client/request response limits can only lower generated ceilings. Exact JSON,
validation and native conversion retain their existing finite budgets. URLSession
uses ephemeral sessions, disabled cookie/cache/credential persistence, redirect
refusal and one SDK request per operation. Mutable transfer state is contained in
lock-protected Sendable adapters; continuations/user closures run outside locks.

## Explicit boundaries

- Nested/streamed multipart and streaming request producers are not admitted.
  Composite RFC6570 **form response** grouping declines because an
  unambiguous inverse grouping plan is needed. Content-based named form responses
  are supported.
- MIME style encodings that expand one schema value into multiple fields/parts
  (exploded composites and deepObject) retain an explicit grouping refusal.
  A style descriptor for multipart media other than form-data also declines:
  those source fields should be ignored by the shared media planner rather than
  activated as a form-data wire strategy. The adapter does not reconstruct a
  different representation from raw source text.
- Operation `responses` absence (`UndeclaredResponses`) remains a disabled native
  capability. CONNECT tunnels, mutual TLS, undefined query credential/content
  merging and the shared planner's other declared unsupported semantics remain
  explicit refusals.
- URLSession does not expose reliable uncombined repeated Set-Cookie fields.
  Source-typed Set-Cookie declarations decline. Other ambiguous repeated typed
  header fields fail decoding.
- The shared planner's flat/structural rules still apply: undefined cookie
  combinations, nested style values, untyped form/part extras, mixed representation
  choices within a part, transfer encoding and whole-content stream assertions
  have explicit refusal paths.
- Active directional annotations and unsupported native schema layouts retain
  their existing source-linked policy. No schema/model fallback was added.
- Vendor SSE schemas, sentinels, stream request fields, pagination and retry
  extensions are uninterpreted. A 3.1 SSE complete-content schema does not become
  `itemSchema`.
- `CompatibilityProfile::LegacyBinaryStringV1` is an explicit generation opt-in
  for the legacy 3.1/3.2 string/binary marker in byte contexts. Ordinary OAS 3.0's
  binary marker is separately witnessed without that opt-in.

## Actual OpenRouter expansion

Three additional operations from the unchanged actual checkout have native
package, consumer, type, DocC and real URLSession wire witnesses:

- `createCoinbaseCharge`: explicit anonymous security and unspecified bounded
  success bytes, plus its typed 410 error. Its deprecation/never-200 prose is not
  interpreted as a transport rule.
- `downloadContainerFileContent` and `downloadFileContent`: in-memory binary
  responses, admitted with the explicit legacy-binary profile. Original parameter
  constraints and JSON error codecs remain in force.

The source's `uploadFile` and `createOauthToken` still decline for untyped form
extras; `sendChatCompletionRequest` still declines for its nonstandard 3.1 SSE
item convention. No upstream schema was repaired. Independent protocol fixtures
provide the ranges, headers, links, forms and standard item streams absent from
the real corpus. The original five-operation OpenRouter and M2 gates also pass.

## Verification and retained evidence

Native gates use public Swift package imports, warnings as errors, independent
literal wire expectations and real loopback sockets. Generated files are always
produced by `plan_sdk` / `emit_sdk` and the ownership-aware writer.

- Main crate: 11 existing Swift host tests, 2 new protocol host tests and 4 Swift
  compatibility tests pass.
- Current Swift **6.3.3 + SDK 26.5**: original M2, five-operation OpenRouter and
  both review regressions pass; protocol fixture, OAS 3.0 and the three new actual
  OpenRouter operations pass native execution, type probes and DocC.
- Floor Swift **6.0.3 + SDK 15.4**: the same native gates pass. The original failed
  SwiftShims restoration attempt is retained separately and is not counted as a pass.
- Final main-crate protocol gates: **15 native behavioral tests on each
  toolchain**, plus installed SDK/consumer builds, generated example execution,
  positive/eight negative consumer type probes and warning-free DocC conversion.
- Query-amplification regression observed peak RSS growth of **720,896 bytes** on
  current and **753,664 bytes** on floor, under its conservative 24-MiB regression
  threshold. Shared remaining-budget probes observed 163,840 / 65,536 bytes.
  These are process high-water observations, not universal allocation promises.

Early native runs used an isolated Cargo harness pointing at the live production
Swift/protocol/example modules while sibling adapters were mid-migration. It used
the normal owned artifact writer. Main-crate compilation and host/compatibility
checks subsequently passed. The final protocol-specific native checks use the
main crate directly.

Retained roots under `${TMPDIR%/}/opencode` include:

- `swift-protocol-current-debug` — protocol development/repro, 14-case execution,
  positive/eight negative type probes and rendered DocC;
- `swift-protocol-gates/new-openrouter-n19UoD`, `oas30-Y6TOX0` — current additional gates;
- `swift-protocol-floor/protocol-VGlJx3`, `new-openrouter-bBGTZb`, `oas30-XKM7D7` — floor gates;
- `swift-sdk-gates/m2-48BIQF`, `openrouter-QRYK9G`, and review roots — original current gates;
- `swift-protocol-floor-legacy/m2-oFnNOj`, `openrouter-yFkfup`, and review roots — original floor gates;
- `swift-protocol-final-current/protocol-kqaTJ6` and
  `swift-protocol-final-floor/protocol-Js5lvJ` — final main-crate protocol checks
  including exact Unicode HTTP map keys and MIME commentary.

The restored floor extraction was missing SwiftShims headers. The existing
official package was reverified (SHA-256
`764c3d5ba27473494206278c27d1bb0fdc3a8bca35ed902ed030f6699904e903`, trusted Apple
notarization and Swift Open Source signature) and extracted without installation
to `swift-6.0.3-protocol-reexpanded`. No global toolchain or SDK was modified.

Reproduction commands, with the selectors documented in [SDK-SWIFT.md](SDK-SWIFT.md):

```sh
cargo test --locked --offline -p suspect-codegen --test swift_sdk --test swift_protocol
cargo test --locked --offline -p suspect-codegen --test sdk_compatibility swift_
cargo test --locked --offline -p suspect-codegen --test swift_protocol -- --ignored --nocapture
cargo test --locked --offline -p suspect-codegen --test swift_sdk -- --ignored --nocapture
```

For current, use the physical Xcode toolchain's `usr/bin/swift`, `swiftc` and
`docc` with SDK 26.5. For the recovered floor, use
`swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload/usr/bin`
and `/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk`. The new harness
resolves the physical default compiler and explicit SDK for standalone probes,
avoiding `/usr/bin` wrapper lookup failures in symbol-graph extraction.

## Remaining-standard follow-up: custom methods, complete queries and positions

The follow-up adds explicit `CustomMethods`, `QuerystringParameters`,
`QuerystringForm` and `PositionalMultipart` capabilities. They were enabled only
after proposed-capability native wire/descriptor/type/DocC witnesses ran through
the same Swift planning/emission implementation, on current and floor.

### Exact methods

Foundation's `URLRequest.httpMethod` normalizes `get`, `GeT` and `head` to uppercase.
The generated default transport detects this before I/O. On Apple platforms those
requests take a bounded Network-framework HTTP/1.1 path; other requests continue
through URLSession. No method-override header or business proxy convention is used.
The request line retains the actual token, and lowercase `head` can carry a normal
response body. HTTP 205 suppression remains independent of method spelling.

The exact-method path supports content-length, chunked and EOF-delimited bodies,
bounded informational headers/trailers, a whole-transfer deadline, task and stream
cancellation, early-break cleanup, and system TLS hostname/chain verification.
Waiting/failed connections terminate without automatic retry. Tests include a
real untrusted TLS peer and assert a TLS-domain failure, not a missing-listener
failure. This path is HTTP/1.1 with identity content encoding; it does not implement
content decompression, tunnels, implicit proxy configuration or a trust bypass.
Non-Apple transports must provide equivalent case-preserving behavior for these
tokens; the generated fallback explicitly refuses when Network is unavailable.

### Complete queries and ordered parts

```swift
let query = Query(q: "a +雪", exact: .value(try JsonNumber("1.00e+3")))
_ = try await client.wholeJSON(WholeJSONInput(criteria: query, id: "a/b"))

let fields = FormFields(bar: true, foo: "a + b",
                        tags: .value(["a+b", "c d"]))
_ = try await client.wholeForm(WholeFormInput(id: "x/y", fields: fields))
```

Literal witnesses distinguish JSON/text URI encoding from form encoding:
`foo=a+%2B+b`, style-encoded array values such as `tags=c%20d`, and a complete JSON
query with no `criteria=` prefix. The complete-query codec is a real aggregate
JSON schema input. Per-field codecs/limits, structural rules and the shared
remaining URL budget run before transport; the 4-KiB repeated-name fixture checks
amplification without allocating an oversized final query.

Positional bodies expose `part1`, `part2`, optional suffix slots and typed `items`.
They enforce contiguous prefix presence, total cardinality and per-position media,
header and payload codecs. The core false-prefix barrier removes later slots and
forbids remaining items; tests also cover `prefixEncoding` extending beyond
`prefixItems` using the actual indexed `items` schema. No aggregate tuple model,
byte-to-null conversion, inferred part name or inferred Content-Disposition is
introduced. Positional form-data uses the declared required disposition headers.

The MIME style witness corrected an earlier native expectation: OAS 3.2
Appendix E.3 puts names in Content-Disposition and values in part bodies. The
named repeated `tag` fixture now expects body `1`, not `tag=1`. The prior accepted
reports remain retained; the 15-case suite is reused as a regression with this
normative expectation corrected.

### Follow-up evidence

`tests/swift_protocol.rs` adds three descriptor/decline/capture checks and the
`native_remaining_standard_custom_query_positional` gate. Its nine native cases
exercise independent literal wire vectors, real sockets, byte equality, negative
values, resource failures, cancellation/cleanup and TLS rejection. The gate also
builds/imports the package, executes source-validated examples, checks a positive
consumer and seven negative type probes, and converts DocC with warnings as errors.

Pre-advertisement evidence is retained at:

- `swift-next-gates/proposed-xVeEJU` — current native execution and type probes;
- `swift-next-gates/proposed-F6AGGg` — current regenerated, successfully rendered DocC;
- `swift-next-floor/proposed-T5x5pp` — complete floor native/type/DocC pass.

Final public-entry-point results on **both** Swift 6.3.3 + SDK 26.5 and Swift
6.0.3 + SDK 15.4: **9 focused native cases + 15 regression cases passed**, with
SwiftPM package/consumer builds, executable examples, positive-controlled
negative type probes (7 focused / 8 regression), and DocC conversion. Five
protocol host tests and four Swift compatibility tests also passed.

Final retained artifacts:

| Gate | Current | Floor |
| --- | --- | --- |
| Focused standard capabilities | `swift-next-final-current/standard-Tk4dAR` | `swift-next-final-floor/standard-e2Bf6j` |
| Existing 15-case regression | `swift-next-regression-current/protocol-nHDOBY` | `swift-next-regression-floor/protocol-H5j5Pa` |

Run the focused public gates with native selectors as above:

```sh
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test swift_protocol \
  native_remaining_standard_custom_query_positional -- --ignored --nocapture
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test swift_protocol \
  native_protocol_spm_types_wire_stream_lifetimes_and_docs -- --ignored --nocapture
```

DocC initially overflowed its Markdown parser's stack on an unescaped long wire
name. The generator now escapes wire names and source IDs in DocC comments;
executable models/codecs were not changed. Swift 6.0.3 initially hit a debug-type
reconstruction assertion when binding `POSIXErrorCode` from NWError. Converting
that case through its NSError code avoids the problematic local debug type;
no compiler check was disabled.

The follow-up gates used the main crate with `--no-default-features --features
http-protocol` during unrelated Java/default-feature integration. Main subsequently
reported that canonical GenerationOptions integration passes for all 12 targets.
The Swift capture continues to use `backend::swift_options(&snapshot.generation)`.
Earlier core/IR/schema and M2/five-operation reports are retained rather than rerun.

### Main-owned fingerprint integration

Main registered these production files in the shared
`compatibility/provenance.rs` `SWIFT` asset list after the completed native
follow-up, so protocol-only edits affect native runtime fingerprints:

```text
swift_sdk/protocol.rs
swift_sdk/protocol_emit.rs
swift_sdk/protocol_examples.rs
swift_sdk/protocol_metadata.rs
swift_sdk/protocol_positional.rs
swift_sdk/protocol_query.rs
swift_sdk/protocol_runtime.swift
swift_sdk/protocol_parameters.swift
swift_sdk/protocol_parts.swift
swift_sdk/protocol_stream.swift
swift_sdk/protocol_exact.swift
```
