## Native values and presence

Objects use `Model.builder(requiredValues...).optionalField(value).build()`.
Required singleton `const`/`enum` fields are supplied from their source-proved
value; schema defaults are never inserted. Each build validates and deeply
snapshots lists, additional properties and union payloads. Later builder or
input-list mutations cannot alter a built model. Every encode validates again.

`Presence<T>` separates omission from presence. Ordinary nullable scalars and
objects use Java null; the free JSON domain uses `JsonNull.INSTANCE`. A union
keeps its explicit typed arm, including a null-only arm. Getters and nested
containers are immutable. Optional setters have an `omitField()` counterpart.

Finite literals have nominal native types and named constants. Decoding retains
alternate exact numeric spellings. `oneOf` validates both the selected native
arm and parent exclusivity. `anyOf` chooses the first valid typed view on decode;
its complete wire data, including additional properties, is retained. Static
references keep source-specific codecs and recursive object/union identity.

## Exact numbers and validation

Numbers use `JsonRuntime.JsonNumber.parse(token)` or `JsonNumber.of(long)`.
Bounds, integrality, equality and divisibility remain exact, including huge and
zero-padded exponents. `exactIntegerValue(maxDigits)` is an explicit bounded
conversion. Codecs never use a binary float or bounded decimal intermediate.

Every source holder exposes `CODEC`, `decode(String)` and `encode(value)`.
`CODEC` also supports byte decoding, immutable JSON-value conversion, `snapshot`
and `withLimits`. One call shares finite JSON work, conversion/byte work, schema
visits and equality allowances through every nested value and branch trial.
Exhaustion never counts as an invalid alternative that another arm can hide.

The default ceilings are 8 MiB JSON input/output, depth 128, 32 MiB JSON work,
32 MiB conversion/byte work, 100,000 validation visits and 100,000 equality
pairs. Schema numeric operands have the checked program's 4,096-byte ceiling;
standalone exact JSON numbers permit at most 65,536 source bytes and finite
numeric work. Limits bound work/representation size, not mathematical exponent
magnitude. Different recursion layers consume depth independently.

## HTTP, cancellation and ownership

Each operation has sync and `CompletableFuture` methods. A sole declared success
status returns its concrete typed result directly. Multiple successes expose a
sealed result interface. The outer `data()` is HTTP response data; an inner
`data()` exists only when the wire schema declares that field.

Use `HttpRuntime.Options.builder().timeout(Duration.ofSeconds(10))` to set a
whole-call deadline, including preparation, response-body completion and codecs.
For streaming responses it continues through item decoding until EOF or close.
Async `future.cancel(true)` terminates preparation, transport and body work.
Interrupting a synchronous caller preserves the interruption flag and cancels
the operation. Closing the client cancels its active calls and removes owned
timer/worker/HTTP resources; the client is `AutoCloseable`.

Inject a standard `java.net.http.HttpClient` with `.httpClient(transport)`.
It must disable redirects, cookies and implicit authentication. It remains
caller-owned, while the SDK still enforces its own deadlines and cancellation.
SDK-created transports disable redirect following, cookie persistence and
environment proxy selection. No SDK retries or pagination loops are inferred.

`maxResponseBytes`, `maxRequestBytes`, `maxUrlBytes`, `maxHeaderBytes`,
`maxStreamBufferBytes`, `maxCaptureBytes` and `codecLimits` make resource policies
explicit. URL expansion is checked while
percent encoding, including repeated array query names. Response bytes are
bounded in the HTTP body subscriber before accumulation. The deadline is
cancelled when the call or owned stream finishes. Cleanup exceptions cannot replace a primary
resource failure or cancellation.

Every request identifies suspect as the generator through the automatic
`User-Agent` header: `suspect/<generator> <identity> (java/<runtime version>;
openapi/<spec>)`, where the identity is the SDK package or a caller-supplied
application. `userAgent(value)` fully overrides it (an empty string suppresses
the header entirely) and `applicationId(name)` replaces the SDK identity token
with an RFC 9110 `<name>[/<version>]` application identifier; invalid
identifiers simply omit the automatic header. A declared or caller-supplied
`User-Agent` header keeps precedence over the automatic value.

## Authentication and errors

Credentials are keyed by the **exact OpenAPI security scheme name**:

| Source declaration | Options builder method |
| --- | --- |
| HTTP bearer | `credential(sourceScheme, token)` |
| HTTP Basic | `basic(sourceScheme, username, password)` (explicit UTF-8 policy) |
| Header/query/cookie API key | `apiKey(sourceScheme, value)` |
| OAuth2 / OpenID Connect | `authorization(sourceScheme, context -> attachment)` |

An attachment is `HttpRuntime.Authorization.of(httpScheme, credential)`.
The hook receives original source/flow/discovery metadata, declared permissions,
and whether they are scopes or roles. Applications own token acquisition and
refresh. The source security alternatives are OR choices; every requirement in
the chosen alternative is attached together. The first fully configured source
alternative is used by default, including an anonymous alternative. Set
`securityAlternative(index)` on options or `RequestOptions` for an explicit
choice. Anonymous operations attach no configured credential. Attachment conflicts
are rejected before sending.

The name `apiKey` does not imply API-key
serialization: if its declaration is HTTP bearer, the header is
`Authorization: Bearer <token>`. Environment lookup is application code. Token
acquisition, refresh and login flows are not inferred from names or prose.
API-specific requirements, such as a management key, remain in operation docs.

Declared non-2xx responses throw their status-specific `SdkException` subclass
with typed `data()` and immutable `headers()`. Catch the concrete variant when
handling a known API failure, or inspect `kind()` and `status()` on `SdkException`
for transport, timeout, cancellation, unexpected response/media/encoding,
invalid request/response and resource failures. Messages exclude credentials,
response data and underlying transport messages. `capture()` is an explicit,
defensive copy bounded by `maxCaptureBytes`; `truncated()` reports truncation or
incomplete capture. Codec failures retain `schemaSource()` and `instancePath()`.

## Servers, parameters and response selection

`RequestOptions.builder()` provides per-call `serverUrl`, `documentUrl`,
`serverIndex`, `serverName`, `serverVariable`, `securityAlternative`, `accept` and
`timeout` choices. A per-call server name/index overrides the default source
selection. Explicit URLs are absolute HTTP(S) bases. Relative source servers
resolve against their document's retrieval URL; local files need an explicit
HTTP `documentUrl`. Variables use literal substitution with source defaults,
enum checks and rejection of unknown names. Unicode DNS names use JDK IDN;
transitional IDNA deviations require an explicit ASCII host spelling.

Parameters retain standard path/query/header/cookie styles, explode behavior,
content encodings and allowReserved rules. OAS 3.2 `querystring` content replaces
the whole query, including form content without double encoding. Empty composite
parameters must be omitted where optional; undefined delimiter conventions are
not invented. Standard and custom method tokens retain their case. SDK-owned
transport uses a bounded HTTP/1.1 path for custom tokens that case-fold to HEAD,
because JDK HttpClient otherwise suppresses their body. Injected transports must
honor case-sensitive custom method semantics.

Responses choose exact status before range before default, then the most specific
matching media declaration (including declared parameters). Actual status decides
success vs API failure. JSON, structured `+json`, exact text scalars, native bytes,
wildcards, forms and multipart retain distinct codecs. A wildcard cannot bypass
a more specific JSON declaration. Response headers have generated immutable
`typedHeaders()` models. `links()` exposes source metadata and calls no operation.

`Bytes` is immutable octet content; `toByteArray()` returns a defensive copy.
`NoContent.INSTANCE` represents HEAD/1xx/204/205/304 bodies. Range/default results
use `ResponseBody<T>` when actual status might forbid content: `isPresent()` and
`value()` distinguish that case from valid content, including a JSON null.
Responses without content declarations retain bounded native bytes.

## Forms and multipart

Source form and multipart objects have typed immutable builders. Multipart fields
use part wrappers with `value()`, `contentType()`, optional `filename()` metadata,
and source-typed `headers()` where declared. Filenames are never read as paths.
Byte parts take `Bytes`, never a JSON null stand-in. Explicit finite positional
multipart exposes prefix fields and typed `addItem` tails; holes are rejected.
Named extras use `putAdditionalPart(name, value)` and are retained without hiding
declared fields. Required names, aggregate bounds, repeated-part bounds, per-part
codecs and byte ceilings are enforced on construction and transport.

Declared complete aggregate examples preserve every supplied value, optional
empty repeated group, named extra and positional tail. Missing optional members
stay absent. Positional occurrences retain their own encoding even when they
reuse one schema. Whole aggregates are example-validation values, not invented
native JSON codec roots; byte aggregate examples retain explicit unavailable
findings and labeled native-octet fallback recipes.

OAS 3.1 explicit array styles encode the whole array. OAS 3.2 applies field
encoding per array item. MIME framing has bounded header/part parsing, quoted
boundaries and metadata, repeated fields, preamble/epilogue handling and literal
byte payloads. Unsupported content/transfer encodings are explicit failures.

## Streaming SSE and JSON lines

Response item streams are `EventStream<T>`: `Iterable<T>`, `AutoCloseable` and
`Flow.Publisher<EventStream.Item<T>>`. Own the response or stream with
try-with-resources, especially when leaving a loop early. Only one consumption
mode may be selected:

- Blocking iterator: `for (var item : response.data()) { ... }`.
- Async pull: `nextAsync()` returns `CompletableFuture<Presence<T>>`; absence is
  EOF, while a present Java null remains a valid nullable item.
- Flow: demand controls item decoding; `Item<T>` is a non-null wrapper whose
  `value()` may be Java null. Cancellation releases the subscription. Terminal
  callbacks are serialized with item callbacks; deadlines also apply at zero demand.

SSE items are parsed envelopes: data remains text, valid IDs persist, and retry
fields accept ASCII digits with exact integer normalization. UTF-8 replacement,
CR/LF/CRLF, comments, blank blocks and incomplete EOF blocks follow the standard
event-stream rules. A vendor sentinel such as `[DONE]` remains data. The SDK does
not parse event data as JSON from annotations, reconnect, retry or paginate.
JSON lines use strict UTF-8 and exact JSON per line; blank lines are invalid and
a final nonempty unterminated line is accepted. Parsing and item codecs share the
configured per-item budgets. Total bytes, queued bytes and item bytes are bounded.
Request item streams are finite checked native lists encoded within byte ceilings.

## Admitted boundary

This package consumes the shared bounded OpenAPI 3.0/3.1/3.2 protocol plan and its
actual JSON/header/part/item codec roots. OAS 3.0 nullable, exclusive bounds and
reference-sibling rules retain their original context, including external
documents. OAS 3.0 binary is native; the nonstandard OAS 3.1 binary-string marker
requires explicit `legacy-binary-string-v1` generation policy. The profile never
activates because of a service name. Unsupported semantics fail with source-linked
findings; the Java capability list does not silently expand when shared planning grows.

The validator executes v1, scoped v2 and explicitly admitted resource/dynamic v3
programs. Conditions, dependencies,
contains, pattern properties, property-name checks and unevaluated properties/items
use fresh child scopes and propagate only the documented successful evaluated
locations. All branch trials share work/equality/depth limits; exhaustion cannot
be inverted into a match. Ordinary base closures retain v1 program bytes and
execution. Format remains annotation-only.

Declared fields retain native types, immutable builders, fixed literals and
absence/null behavior. Pattern-matched extras are retained in an immutable
`Map<String, JsonValue>` and checked against every matching pattern plus the
appropriate additional-properties rule. Required dynamic keys are supplied using
`putAdditionalProperty`; builds check their presence. Scoped intersections, tuples
and constrained union views use source-checked `JsonValue` carriers where a static
Java value type would lose information. Their codec holder's `CODEC`, `decode`
and `encode` enforce the full schema, and operation inputs/responses use that
binding automatically. These carriers never bypass schema checks.

V3 uses the compiler's immutable resource catalogue and aligned node scopes.
Entering a nested schema enters its indexed resource without evaluating the
resource root. Dynamic references search the outermost actually entered matching
binding; unused candidates stay inactive. Pointer, empty-fragment and static-anchor
fallbacks keep their initial target. Every return/trial restores context, and cycle
identity includes the exact ordered resource context. Target annotation scopes are
fresh and failures remain noninvertible. Runtime evaluation acquires no documents
and resolves no schema URIs. Physical source IDs remain diagnostic/ownership
identities; logical URIs remain separate metadata. Dynamic-use values have checked
JSON carriers, so an initial fallback never becomes a guessed static native type.

The canonical Java backend uses the explicit resource-aware planner when needed;
ordinary admitted closures keep their established V1/V2 programs. Direct callers
can use `plan_sdk_with_protocol_v3` / `plan_validation_v3`; existing direct planning
entrypoints retain V1/V2 admission.

Directional projections, custom dialect/vocabulary semantics and legacy recursive
reference keywords remain source-linked boundaries. CONNECT tunnels, generic typed Set-Cookie, ambiguous untyped
scalar header/part extras, and unbounded positional multipart receive located
refusals. Finite positional multipart is limited to 512 prefix positions and
maxItems at most 4096.
Native model/input/header/aggregate factories are limited to 200 required
arguments and 512 fields, with source-linked refusal before JVM arity overflow.

Native build/install, documentation, consumer and runtime tests are separate
verification gates; generation alone is not a release certification.
