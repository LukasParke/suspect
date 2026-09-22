## Native models, presence and exact numbers

Use the generated data classes with named constructor arguments. Required fixed
string tags are codec-owned getters. Optional inputs have native defaults;
operations with no required inputs can be called with `client.operation()`.

| Source states | Kotlin representation |
| --- | --- |
| Required non-null | Constructor argument `T` |
| Required nullable | Required argument `T?`, including explicit null |
| Optional non-null | `Presence<T> = Presence.Absent` |
| Optional nullable | `Presence<T?> = Presence.Absent` |

`Presence.Present(null)` emits JSON null where the source permits it; Absent
omits the member. Source defaults remain annotations. Unknown permitted JSON
members remain in the typed `additionalProperties` store. `JsonNull` is the null
member of the explicit `JsonValue` domain.

`JsonNumber.of(123L)` and `JsonNumber.parse("9007199254740993.000000000000000001")`
are exact. Tokens retain spelling; equality/hash/order and `isInteger()` are
mathematical. Huge and zero-padded exponents remain symbolic.
`toBigIntegerExact()` checks integrality and limits expansion to 4096 digits;
`toLongExact()` also checks overflow.

`Codecs.<name>.decode/encode` executes the checked source program. Native union
wrappers validate the selected arm and the parent, including oneOf exclusivity.
Each encode revalidates caller-owned mutable collections and snapshots generic
JSON extras. Avoid concurrent mutation during a codec call. Schema invalidity
is `ValidationException`; incomplete resource-limited evaluation is
`EvaluationException`, including inside logical alternatives.

## Source-defined authentication and servers

Credential constructor properties derive from the actual security schemes.
Bearer/API-key values are explicit strings, Basic uses `BasicCredentials`, and
OAuth/OIDC use a suspending `CredentialProvider`. A hook receives
`CredentialContext` with operation identity, the scheme, scopes/roles and the
source-defined OAuth-flow or OIDC-discovery metadata. It returns the complete
Authorization field. Credential acquisition and refresh belong to the caller.

The default OR policy selects the first source alternative for which every AND
credential is supplied. `RequestOptions.securityAlternative` selects an explicit
alternative. Anonymous alternatives attach no credentials. A selected malformed
credential fails before I/O. Basic values use UTF-8 followed by Base64; usernames
cannot contain the colon delimiter. API-key query values use URI component
escaping; API-key cookies require valid, already escaped cookie octets.

`Client()` is sufficient for anonymous operations. `ClientOptions` and
`RequestOptions` expose `serverIndex`, literal `serverVariables`, and `documentUrl`
for relative server resolution. The base is the retrieval URL, not `$self` or
schema `$id`; locally loaded descriptions need an explicit HTTP document URL.
Server enum/default/unknown-variable rules are enforced. The explicit
`serverUrl` convenience override accepts HTTPS or loopback HTTP.

## Parameters and representations

The runtime executes the shared planned parameter styles, explode settings,
content encodings and locations. Values remain source-bound native types.
Headers and cookies have control/delimiter checks. Reserved expansion preserves
valid caller-supplied percent triples once; illegal query/path delimiters require
explicit pre-escaping. Undefined composite values, including ambiguous empty
expansions, have explicit representation errors.

Multiple request media have a sealed native request-body choice. A wildcard
request additionally needs `RequestOptions.requestMedia` containing a concrete
Content-Type. A more-specific source declaration cannot be bypassed by choosing
a wildcard arm. `responseMedia` selects and requires a successful representation;
documented HTTP errors still decode under their own declarations.

Response matching is exact status, then status class, then default. Media
matching uses concrete media, then type wildcard, then `*/*`, with declared
parameters contributing specificity. Actual status is retained, including for
default responses. A success result exposes `.data` directly when all success
arms have the same native payload type. Sealed cases preserve every other
status/media/disposition alternative.

JSON and structured `+json` media use exact codecs. Text uses its declared
scalar codec. Binary media use `ByteArray` and their finite byte policy. A
response with no declared body exposes bounded bytes; HEAD and body-forbidden
statuses use `Unit`. No synthetic JSON value stands in for either case.

Declared response headers have generated typed structures. `ResponseInfo.links`
retains source link metadata and expressions without invoking linked operations.

## Forms and multipart

Named form and multipart bodies have native data classes. Required fields,
property counts, extra-field rules and repeated-item counts come from the shared
structural descriptors. Individual JSON/text values use their real codecs;
binary aggregate bodies never pass through a JSON-null substitute.

Use `Upload(data = bytes, filename = "example.bin")` for binary parts. Fields
with declared part headers have a native wrapper and typed header structure.
Concrete part media choices must match the source declarations. Multipart
boundaries are generated and owned by the runtime. Filename/name/header quoting,
body bounds and malformed incoming framing are checked.

Present empty repeated fields and unsupported response composite grouping are
diagnosed rather than silently changing their representation. Positional
multipart, nested multipart and transfer-encoding profiles need separate native
admission and are not enabled by this package.

## Cold Flow and resource lifetime

An operation with an SSE or JSON-Lines response returns a cold `Flow<Result>`.
Collection starts the exchange. Each emitted status/media case contains its
typed item and actual HTTP metadata. `take`, cancellation, exceptions and normal
completion close the response reader. A rendezvous channel gives backpressure;
the runtime does not launch a detached operation.

SSE uses standard OpenAPI 3.2 `itemSchema` event envelopes. Data remains a string;
multiline data is joined with LF. Comments, unknown fields, invalid id/retry
values and no-data blocks are ignored. Provided event/id/retry fields retain
their native types. No JSON-in-data transformation, sentinel or reconnect policy
is inferred. An unfinished SSE block is not dispatched at EOF. JSON Lines
validates each JSON value, with blank records rejected.

Request `itemSchema` sequences accept `Flow<Item>`. They are collected once into
a finite, bounded request body before any HTTP bytes are sent. Producer failure,
limit exhaustion and cancellation close the producer and prevent a partial POST.
This finite-request profile enforces the total request ceiling as well as each
item's source/runtime ceiling. SSE request fields must be representable by the
standard framing; unsupported fields or framing characters fail explicitly.

The public `StreamingTransport` / `BodyReader` seam supports owned response
streams; finite `Transport` responses can also feed a Flow. Implementations must
cooperate with cancellation and return bounded chunks. The JDK adapter owns
its input streams and cancellable read tasks. It disables redirects, cookies,
implicit proxy routing and automatic credential acquisition. The SDK adds no
retry or pagination loop.

## Errors, deadlines and finite policies

Declared API failures have public operation-specific exception classes and
validated `.data`. `SdkException` separates request representation/validation,
authentication, response validation/media/metadata, transport, timeout and
resource failures. Original causes and bounded raw metadata are available;
messages exclude credentials and payloads.

Configured client/per-call deadlines cover preparation, transport, decoding and
Flow collection. Caller coroutine cancellation, including an outer `withTimeout`,
remains cancellation. CPU conversion uses structured `Dispatchers.Default` work,
so caller timeout schedulers can remain responsive. Cleanup errors cannot replace
a primary resource failure or cancellation. `Client.close()` closes an injected
AutoCloseable transport; Kotlin `use` preserves primary exceptions.

Defaults: 4 MiB JSON/body/request-URL ceilings, depth 128, 4096 numeric-token bytes,
100000 JSON/model/schema/equality/numeric visits, 16 MiB conversion/assertion text
budgets, 1 MiB stream items, 64 KiB transport chunks, 8192 preview bytes and
32 KiB / 128 header fields or values. Request part codecs share their conversion
and validation budget; response items have independent bounded codec sessions.
Limits can be lowered through the public options.

## Reference and verified profile

- [Native operations, models and codecs](docs/reference.md)
- [Source examples, provenance and findings](docs/examples.md)
- [Shared protocol descriptors](docs/protocol.json)
- [Allocated symbols](docs/symbols.json) and [coverage](docs/coverage.json)
- Dokka HTML: `target/dokka/index.html` after `mvn verify`.

Maven produces the main Kotlin/metadata jar, source jar, Dokka jar and executable
examples jar. Examples are lowered to native constructors and executed during
the build. Native byte fixtures are explicit application scaffolding, separate
from declared JSON/text examples. Invalid declarations remain source findings.

This package's declared capability set is recorded in `docs/protocol.json`.
OpenAPI 3.1/3.2 JSON Schema models use the checked static 2020-12 profile.
Native intersections, positional tuples, active directional views, OpenAPI 3.0,
implicit extension profiles and undefined protocol shapes have located admission
diagnostics. Format remains annotation-only. JDK-restricted request headers and
CONNECT tunnel behavior are outside the native transport profile.
