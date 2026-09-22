# Go shared HTTP protocol adoption

The Go SDK now consumes the admitted `http_protocol::ProtocolPlan` directly.
`go_http::plan_http` compiles its actual `codec_roots()`, allocates native names,
and produces the client, models/codecs, metadata, documentation and examples in
one pipeline. It calls `examples::plan_protocol_examples`; the old
`http_contract::plan` is not an admission or example-planning stage for Go.

The adapter identity is **`go-net-http-protocol-v1`**. Opt-ins are an explicit
list in `go_http/planning.rs`, rather than `Capability::ALL`. Shared descriptor
semantics are documented in [SDK-HTTP-PROTOCOL.md](SDK-HTTP-PROTOCOL.md).

The optional source-bound runtime environment factory is documented in
[SDK-GO-CREDENTIAL-ENV.md](SDK-GO-CREDENTIAL-ENV.md), including its compiled
`NewClientFromEnv` API, creation-time snapshot semantics and no-policy byte parity.

## Native API

Existing operation, required-input constructor and exact-status wrapper names
remain available. For example, a generated OpenRouter consumer can write:

```go
client, err := sdk.NewClient(sdk.ApiKey(token), sdk.ClientOptions{
    Transport: httpClient, // optional standard net/http-compatible Doer
})
if err != nil { return err }
defer client.CloseIdleConnections()

credits, err := client.GetCreditsData(ctx)
if err != nil { return err }
fmt.Println(credits.Data.TotalCredits.String())
```

- Operations with no required inputs accept zero or one typed input. Optional
  inputs still use the allocated `New…Input().With…` API.
- A single unambiguous successful representation also gets an allocated
  `…Data` method. The ordinary method retains actual status, typed status/media
  alternatives, raw headers, `DecodedHeaders` and link metadata.
- Result and declared API-error interfaces support `Close`. Buffered responses
  need no cleanup; closing a response carrying an iterator releases its stream.
- Multiple request media have a closed native body choice and a constructor for
  each representation. A wildcard choice requires a concrete Content-Type.
  Dispatch verifies the most-specific declaration before encoding, preventing a
  broad byte alternative from bypassing a JSON schema.
- `Part[T]` holds native `Data`, `ContentType`, `Filename` and `http.Header`.
  `NewPart`, `WithContentType`, `WithFilename` and `WithHeader` are ordinary native
  constructors/setters. File data is finite `[]byte`; a filename never opens a file.
- Forms and multipart use generated native aggregate structs. Required fields,
  extras, array multiplicity/cardinality and per-part codecs are checked before
  transport. Mixed binary aggregates are never sent through a JSON codec.

Generated README callsites and `examples/validated/main.go` construct native
models, closed-union alternatives, typed parts and finite item slices. The
ExamplePlan retains source/synthesized origins for real codec values. Native byte
demonstrations are explicitly labeled **empty in-memory byte recipes**, and
schema-free JSON recipes are labeled separately; source example strings, filenames
and JSON null are never converted into file content. Fixture HTTP calls require
an explicit `-server-url`; running the example without it checks codecs locally.

## Protocol surface

| Area | Go behavior and acceptance witnesses |
| --- | --- |
| Methods | Fixed methods through QUERY and case-preserved custom methods. Native tests include `GeT`, plus GET/PUT/POST/DELETE/OPTIONS/HEAD/PATCH/TRACE/QUERY. |
| Servers | Ordered candidates, `ServerIndex`, literal variable defaults/enums/overrides, HTTP/HTTPS and relative document URLs. `DocumentURL` supplies an HTTP base for local specs; `ServerURL` is an explicit absolute override retaining its path prefix. Invalid choices fail before transport. |
| Security | Undeclared/disabled/anonymous, OR alternatives and AND requirements. Bearer, basic, header/query/cookie API keys, explicit caller authorization and OAuth/OIDC hooks. `SecurityAlternative` selects an OR member; otherwise the first complete member in source order is used. |
| Credential metadata | Hook requests include operation/scheme provenance, distinct scopes and roles, flow URLs and scope descriptions, OAuth metadata URL and OIDC discovery URL. Caller hooks own token policy; the SDK performs no acquisition, discovery or refresh. Conflicting attachments fail before transport. |
| Parameters | Scalar, homogeneous scalar-array and flat-object simple/label/matrix/form/spaceDelimited/pipeDelimited/deepObject styles, 3.2 cookie style, JSON/text content parameters and `allowReserved`. The native tests consume the core's 30 independent literal vectors plus rejection cases. |
| Complete querystring | OAS 3.2 JSON and text receive one component-encoding pass. Form querystrings use the actual FormPlan and per-field codecs with no second URI-encoding pass. |
| Response selection | Exact status > class > default, then concrete media > type wildcard > any wildcard, with declared parameters breaking ties. Actual 2xx status determines success, including a default match. Exact-status media mismatches do not fall back to another status declaration. |
| Content | JSON and `+json`, schema-free JSON, UTF-8 text/scalars, bounded bytes, media parameters and wildcards. Missing/invalid/duplicate Content-Type fails when content is declared. Undeclared response content is bounded bytes. |
| No-body responses | HEAD and 1xx/204/205/304 suppress reading/decoding even when content is declared. Unused body schemas are not codec roots. Headers still validate. |
| Headers and links | Required and optional scalar/array/flat-object or content-based header codecs. Repeated Set-Cookie cannot be comma-folded. Links retain source/target/literal-expression metadata without calls or schema-root discovery inside literals. |
| Forms/parts | Content-based and RFC6570 form encoding; named and OAS 3.2 positional multipart, repeated files, per-item codecs, typed extras, per-part headers and finite in-memory byte parts. Both requests and admitted 3.2 responses are supported. MIME style names/bodies are structured values: `&`, `=`, spaces and Unicode are not accidentally split as a query string. |
| Streams | OAS 3.2 `itemSchema` SSE and JSON-lines responses use `Stream[T]`. Finite typed item slices encode request streams. SSE passes parsed event envelopes to the item codec: string data/id/event and integer retry. JSON inside data, sentinels and retry/reconnect behavior are not inferred. |

### Stream lifetime

```go
stream, err := client.EventsData(ctx)
if err != nil { return err }
defer stream.Close()
for stream.Next() {
    event := stream.Value()
    fmt.Println(event.Data)
}
return stream.Err()
```

Iteration has one consumer, with bounded read-ahead and no background read loop.
Cancellation, timeout, early close, EOF, framing/codec errors and budget failures
release the body and timeout resources. `Close` can interrupt a blocked `Next`.
The SDK timeout covers injected transports and the entire response/iterator
lifetime. Earlier caller deadlines remain authoritative.

SSE supports transport splits inside UTF-8, CR/LF/CRLF framing, the leading BOM,
comments, ignored fields/values, multiline data, empty data lines and decimal
retry values. An unterminated final event is discarded. JSON lines support LF,
CRLF and a final record without a trailing newline; blank or malformed lines
remain errors.

## Bounds and errors

Generation defaults:

| Policy | Default |
| --- | ---: |
| Assembled request URL/body | 8 MiB |
| Response body / total stream wire bytes | 8 MiB |
| Individual part payload | 1 MiB |
| Form fields / MIME parts, including repeated expansions | 4096 |
| Stream item | 64 KiB |
| Diagnostic capture | 4096 bytes, clamped to response ceiling |

`HttpConfig` can configure generation ceilings; they must be positive and fit
a portable signed 32-bit integer. Client response/part/item limits may lower
generated ceilings. Declared 3.2 binary `maxLength` further limits body/part
bytes. MIME header/disposition overhead consumes the aggregate budget rather
than the part payload ceiling. Repeated-name expansion and part work are bounded
before their potentially large intermediate allocations.

Public `*SDKError` exposes `Kind`, `Operation`, `Source`, `Status`, `Headers`,
`RawCapture`, `Truncated`, `Cause`, `Unwrap` and `ResourceLimited`. Declared errors
are typed response wrappers usable with `errors.As`. Normal formatting omits
captures, credentials and causes. Cancellation/deadline causes remain accessible
through `errors.Is` in both header and body phases.

The default `net/http` transport ignores ambient proxies, disables automatic
decompression and follows no redirects. The SDK adds no retries, pagination or
linked calls. Request bodies have no replay factory. Standard transport
connection-recovery behavior and any injected transport policy remain explicit
transport concerns.

## Located refusals

Shared admission continues to reject undefined/nonrepresented semantics, such as
nested parameter styles, ambiguous empty composites or delimiter data, unsupported
charsets, nested/streamed multipart, untyped part extras, unsupported whole-part
aggregate assertions and vendor stream conventions. Go's existing model/codec
admission still governs schema assertions and neutral readOnly/writeOnly policy.
For example, reference assertion siblings needing an intersection representation
are a source-linked model refusal; no terminal-only codec is substituted.

Go additionally refuses source request parameters named Content-Length,
Transfer-Encoding or Trailer, because `net/http` owns those framing fields.
Host uses the standard request's `Host` property. No admitted parameter is silently
discarded to accommodate native framing.

OAS 3.0's binary string marker is supported as its declared byte representation.
For OAS 3.1/3.2, `type: string, format: binary` in a byte context requires the
explicit `CompatibilityProfile::LegacyBinaryStringV1` in
`HttpConfig.compatibility_profiles`. Ordinary unconstrained binary schemas need
no compatibility opt-in.

## Rust plan API changes

These are the integration points for tooling and compatibility consumers:

| API | Current contract |
| --- | --- |
| `HttpPlan::protocol()` | Borrow the one admitted shared ProtocolPlan, including warnings/provenance and exact codec roots. |
| `HttpPlan::config()` | Borrow the finite native generation configuration. |
| `PlannedOperation::wire()` | Public shared OperationPlan; the old strict Operation projection is gone. |
| `PlannedOperation` | Adds `optional_input` and allocated optional `data_method`. Existing name fields remain. |
| `PlannedParameter::schema()` | Still the actual `&SchemaId`; `wire()` returns the shared ParameterPlan. |
| `PlannedBody` | Adds `native_type`, `media()` and `is_choice()`. `schema()` is now `Option<&SchemaId>` for a single real JSON/text/item codec input. |
| `PlannedMedia` | Shared MediaPlan, native type, optional aggregate plan, and choice names when part of a closed request-media choice. |
| `PlannedResponse::status()` | Returns shared `ResponseStatus` (Exact/Range/Default). Read the original key through `wire().status_key()`. |
| `PlannedResponse::schema()` | Now `Option<&SchemaId>`; bytes, forbidden bodies and structural aggregates have no JSON codec input. |
| `PlannedResponse` | One native status/media variant, with response/media indices, `forbidden_body`, `native_type`, typed header names/plans, `media()`, `can_succeed()` and `can_fail()`. Pattern responses can have both memberships. |
| `HttpConfig` | Adds `max_part_bytes`, `max_parts`, `max_stream_item_bytes`, and explicit `compatibility_profiles`. |

`go/http-manifest.json` has format `suspect-go-http-v2` and embeds the serialized
shared plan. Native operation/body/response names come from the same retained
plans as emission. The Go capture function in `compatibility/native.rs` records
these typed descriptions, including both default-response memberships and close
semantics. `go doc` extracts declarations only for human documentation; it is
never parsed to recover compiler metadata.

## Verification

`tests/go_protocol.rs` installs independently generated modules into separate
consumer modules and runs native positive and negative typing, transport/codec,
wire, boundary, cancellation and cleanup witnesses on **Go 1.23.12 and 1.27.1**.
It also runs the packaged executable examples and native `go doc`.

The actual OpenRouter corpus gate selects the original five operations plus
`getKey`, `deleteKeys`, `listProviders`, `listOauthJwks`,
`downloadContainerFileContent`, `downloadFileContent` and `createAudioSpeech`.
The three byte-returning operations require the explicit legacy-binary profile
in this source. The independent wire fixtures retain exact number tokens and
non-UTF-8/zero byte equality. `uploadFile` and `createOauthToken` retain located
untyped-part-extra refusals, and 3.1 chat SSE cannot acquire inferred item/sentinel
semantics. The corpus used is `openrouter-web` commit
`db378a2a90d0167b9dca4f98b52074c54d249e1f`.
The OpenAPI file SHA-256 is
`bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821`.

Completed native evidence:

| Gate | Go 1.23.12 | Go 1.27.1 |
| --- | ---: | ---: |
| Existing `go_http` | 2 passed | 2 passed |
| Existing `go_http_names` | 5 passed | 5 passed |
| Existing `go_runtime_regressions` | 9 passed | 9 passed |
| New protocol planning/native families | 9 passed | 9 passed |

The Go 1.27.1 consolidated run passed **26 Rust tests** including the documentation
gate. The minimum-floor results include the final focused rerun of the cookie
vectors. Sphinx **8.2.3** built with warnings as errors against **Go 1.23.12** and
verified **302 documented symbols**. The ordinary default-feature
`suspect-codegen` library check also passed.

The pre-existing `go_http`, `go_http_names` and all nine
`go_runtime_regressions` tests remain required. Reproduce the selected Go gates:

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web SUSPECT_GO_TOOLCHAIN=go1.23.12 \
  cargo test --locked --offline -p suspect-codegen \
  --test go_http --test go_http_names --test go_runtime_regressions --test go_protocol \
  -- --include-ignored --nocapture

# Repeat the same gate with SUSPECT_GO_TOOLCHAIN=go1.27.1.
```

An initial temporary harness compiled these same working-tree Go module sources
while other adapters were mid-edit. Subsequent checks use the ordinary
`suspect-codegen` crate. This evidence covers the admitted selected-operation
surface; broad multi-backend integration belongs to the main session.
