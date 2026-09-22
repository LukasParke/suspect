# __PACKAGE__

Native, null-safe SDK generated from OpenAPI __OPENAPI__. **Dart >=3.9.4 <4.0.0**,
with zero pub dependencies. Import `__PACKAGE__.dart` for the portable API or
`__PACKAGE___io.dart` for the VM transport.

## Install and make a first request

```yaml
dependencies:
  __PACKAGE__: __VERSION__
```

The following is the shipped `example/quickstart.dart`, using actual allocated
names and source-validated native values. `API_TOKEN` and `API_SERVER` are explicit
application-selected environment variables. Missing byte/credential examples are
reported in `sdk-manifest.json`; filenames and JSON null are never substituted
for file data.

```dart
__QUICKSTART__
```

## Native values

Construct models with named arguments. Required fields use `required T` or
`required T?`. Optional fields use `Presence<T>`: `const Absent()` omits a field,
`Present(value)` supplies it, and `const Present<String?>(null)` supplies JSON
null. Source defaults are annotations, not automatic request values.

`JsonNumber` and `JsonInteger` retain numeric token spellings. Their JSON scanner,
writer, comparison and divisibility never use `double` or `jsonDecode`. Exponents
are symbolic decimal `BigInt`s, including leading-zero exponent digits.

```dart
final amount = JsonNumber.parse('9007199254740993.000000000000000001');
final count = JsonInteger.parse('1e+000008');
print(amount.token);
print(count.toBigInt(maxDigits: 32));
```

Models and collections are revalidated on every encode. Required constant getters
are emitted only when the compiled field schema accepts the constant. Tagged
unions use sealed interfaces and actual native branch classes. Other unions have
explicit wrappers. Intersections, tuples and heterogeneous literals can use named
validated exact-JSON wrappers. Extra fields, source refs, null and absence remain
distinct. Malformed UTF-8, unpaired surrogates and duplicate decoded keys reject.

## Methods, parameters and servers

- No-input operations are normal `client.method()` calls. Ordinary responses use
  `Future` and a sole success status returns its concrete status class directly.
- Parameters use source-native types and named arguments. Simple/label/matrix,
  form/space/pipe/deepObject, header/cookie and content serializations come from
  the shared typed protocol plan. `allowReserved` requires caller pre-escaping
  of active URI/style delimiters; ambiguous values are rejected.
- OpenAPI 3.2 querystring parameters describe the whole query. JSON/text content
  is encoded once, and form-urlencoded content retains its complete field map.
- `ServerSelection(index:, variables:, documentUrl:)` chooses a declared server.
  Defaults/enums are applied only to declared server variables. Relative servers
  use the HTTP document URL; local specifications require an explicit base.
  `Client(server: Uri.parse(...))` supplies an explicit absolute override.
- Standard and declared custom method tokens are preserved. The VM adapter uses
  a bounded exact HTTP/1.1 path when `HttpClient` would uppercase a custom token.

## Authentication

`Credentials` properties are named from actual security schemes. String fields
serve bearer and API-key schemes; `BasicCredentials(username, password)` serves
HTTP Basic. API keys attach to their declared header, query or cookie name.

Security alternatives are OR; each alternative's requirements are AND. The
default is the first source-ordered complete alternative. A declared anonymous
alternative attaches no credentials. Use `securityAlternative:` to choose an
alternative explicitly. An absent/empty security declaration makes no automatic
use of configured credentials. Conflicting attachments fail before sending.

OAuth/OIDC properties accept `CredentialProvider` callbacks. Each receives a
`CredentialRequest` with cancellation, URL, operation source, required scopes or
roles, and immutable source metadata (flows/discovery URLs). Return an explicit
`AuthorizationCredential`, or `null` when unavailable. The SDK does not acquire,
refresh or introspect tokens. `AuthorizationCredential.bearer(token)` is an
explicit attachment convenience. Automatic request/error strings redact values.

## Response status and content choices

Exact status wins over a class range, which wins over `default`. The actual HTTP
status determines success versus a typed API exception, including for `default`.
A media mismatch never falls through to another status declaration.

Concrete media wins over `type/*`, then `*/*`; declared parameter specificity is
retained. JSON/+json goes through exact codecs, UTF-8 text through its scalar
codec, and binary/wildcard content through bounded `Uint8List`. Undeclared response
content remains bounded bytes. HEAD/1xx/204/205/304 content absence uses `NoBody`,
which is distinct from JSON null and empty binary data.

Multiple representations have sealed, source-allocated content alternatives.
Request alternatives carry a native value and their concrete `contentType`.
Wildcard requests require an explicit type; choosing a broad representation cannot
bypass a more-specific declared schema. Typed response headers live in `.headers`;
raw repeated headers remain in `.response.headers`. `.links` contains source link
metadata and never invokes another operation.

## Forms and multipart

Form/multipart request models have named, typed fields and explicit presence.
File fields use `Uint8List`. Repeated parts use native lists; custom part headers
use generated header classes. Part wrappers expose explicit content type and
filename metadata where needed. Per-part codecs and byte policies are applied,
then required/property/item counts are checked structurally. A mixed byte
aggregate is never sent to a JSON codec with placeholder values.

Form fields follow their declared content/style encoding. Named multipart uses
bounded framing, validated disposition/header values and collision-checked
boundaries. Mutable bytes and models validate again on encode. Conversion work is
shared across parts, and physical part count is capped at 16,384 before assembly.

## Streams, cancellation and lifetime

Source-backed SSE and JSON-lines operations return **lazy single-subscription
`Stream`** values of typed status/item records. No request starts until listening.
Consume each record's `.data` for the native `itemSchema` value and `.response`
for bounded raw metadata. Pausing the listener applies backpressure; cancelling
the subscription, calling a token's `cancel()`, reaching a deadline or closing the
client releases the body subscription and native connection. Late responses are
released, and late errors have handlers.

SSE fields are mapped to the standard OpenAPI 3.2 event envelope: `data`, `event`
and `id` are strings; `retry` is an exact integer. Comments/unknown fields and
invalid id/retry fields are ignored, multiline data is combined, and absent
envelope fields remain absent. JSON in `data` is not implicitly decoded;
`[DONE]` has no special meaning. JSON-lines records use the exact JSON parser.
Each item is validated independently. There is no inferred sentinel, retry,
pagination loop or automatic reconnection.

Always close an owned client in `finally`. The default whole-exchange timeout is
30 seconds; pass a positive per-call `timeout` of at most one day. A completed call
detaches from a reusable caller token. If importing `dart:async` unprefixed, hide
its `TimeoutException` or qualify the SDK to distinguish the two types.

## Failure and resource information

Declared non-2xx responses have operation/status-specific `ApiException` types
with typed data. Unexpected status, media, payload, transport, cancellation,
deadline and closed-client failures are distinct public types. `CodecException`
retains source/instance pointers and separates ordinary mismatch from incomplete
evaluation, conversion, JSON and resource failures.

`.response.body` is an immutable bounded prefix on successes and failures;
`.response.truncated` reports omitted or incomplete bytes. Full binary payloads
use the separately bounded `.data` value. Inspect raw fields explicitly.

Request/response/capture/header/stream-buffer ceilings may only be lowered by
client options. Parser, writer, conversion, evaluation, equality and numeric work
have independent finite budgets. Stream item/chunk/total-byte limits apply before
unbounded buffering. The precise generated policy is source/config dependent.

## Admission and documentation

Generation profiles are explicit. The default planner does not treat
`type: string, format: binary` as bytes in OpenAPI 3.1/3.2. A caller can request
`LegacyBinaryStringV1` through generation options; it affects only binary
media/parts. JSON media still carries a JSON string. The manifest records every
requested profile and the core's source-linked compatibility warning.

Known undefined/unimplemented profiles fail with source-located diagnostics:
OpenAPI 3.0, directional codecs, unnamed operations, undefined style combinations,
arbitrary untyped form extras, JSON cookie content without an escaping policy,
positional/nested multipart, response form/multipart parsing, request item streams,
aggregate stream schemas, JSON-sequence framing and mixed streaming success/error
representations. A source `itemSchema` is required for standard 3.2 streams.

```sh
dart pub get
dart analyze --fatal-infos
dart compile exe example/source_examples.dart -o source-examples
./source-examples
dart compile exe example/quickstart.dart -o quickstart
dart doc --validate-links
```

`doc/API.md`, native dartdoc and `sdk-manifest.json` retain actual allocated
symbols. The manifest contains the typed protocol plan and version-2 example
roles/origins/findings. `doc/validation-program.json` is the checked instruction
program used by the codecs. Examples requiring opaque bytes or external
credentials remain explicit native-example obligations.
