# Dart SDK backend

Native OpenAPI 3.0/3.1/3.2 protocol implementation, installable pub packages, strict
native types, typed compatibility capture and executable guides. Native consumers
are tested on **Dart 3.9.4 (floor) and 3.13.3 (current), macOS arm64**, as
compiled JavaScript under Node, and in **Chrome 153**.

**Scoped validation v2** (all nine applicator instructions) and
**resource/dynamic validation v3** are admitted. Ordinary closures retain their
v1/v2 programs ([ADR 0020](adr/0020-validation-programs-are-versioned-and-frozen.md)).

## Package and ordinary usage

Every artifact is rooted under `dart/`. A package contains:

- `lib/<package>.dart`: null-safe native models, exact JSON, checked codecs,
  typed HTTP metadata/results/errors, `Future` and lazy `Stream` operations.
- `lib/<package>_io.dart`: optional dependency-free VM adapter.
- `pubspec.yaml`, strict analyzer settings, dartdoc settings, README/changelog,
  `doc/API.md`, checked validation program and source/protocol manifest.
- Native-constructor `example/quickstart.dart` and executable
  `example/source_examples.dart`. The latter exercises schema-valid values without
  networking; byte and credential recipes remain explicit obligations.

No-input calls use `client.getCredits()`. A sole success status returns its
concrete result directly. For the actual OpenRouter selection, native request
names include `CreateKeysBody` and `UpdateKeysBody`:

```dart
import 'dart:io' show Platform;
import 'package:generated_sdk/generated_sdk_io.dart';

Future<void> main() async {
  final client = Client(
    transport: IoTransport(),
    credentials: Credentials(apiKey: Platform.environment['OPENROUTER_API_KEY']),
  );
  final cancellation = CancellationToken();
  try {
    final response = await client.getCredits(
      cancellation: cancellation,
      timeout: const Duration(seconds: 20),
    );
    print(response.data.data.totalCredits.token);
    final created = await client.createKeys(
      body: CreateKeysBody(name: 'Native Test Key'),
    );
    await client.updateKeys(
      hash: created.data.data.hash,
      body: UpdateKeysBody(limit: const Present(null)),
    );
  } finally {
    await client.close();
  }
}
```

The outer `.data` is a response wrapper; a second `.data` remains when it is an
actual wire field. No empty input struct, codec-first string construction or
unsafe cast is needed for an ordinary request.

## Explicit environment credential defaults

The optional versioned policy in
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md) is supported on both Dart tiers:

```json
{"credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}}
```

`DartConfig.credential_env` stores variable names only. The shared binder runs
after protocol admission; `Plan.credential_env()` exposes its typed result.
Native capture stores `semantic_descriptor()` in `NativeSnapshot.credential_env`,
so physical source relocation does not change the client-default policy.
The actual OpenRouter scheme named `apiKey` remains HTTP bearer.

For a policy-configured package named `openrouter`, the compiled live form is:

```dart
import 'package:openrouter/openrouter_io.dart';

Future<void> main() async {
  final client = Client(transport: IoTransport());
  try {
    final response = await client.getCurrentKey();
    print('HTTP ${response.status}; usage ${response.data.data.usage.token}');
  } finally {
    await client.close();
  }
}
```

The default request is source-declared `GET /key` at
`https://openrouter.ai/api/v1/key`. `getCredits()` is also compiled and tested,
as an explicit management-key operation. Native verification captures these
source-default HTTPS URLs through a controlled transport and makes no account
requests. Existing explicit `Credentials(apiKey: token)` calls still work.

Only configured clients gain these constructor semantics:

- Omitted `credentials:` snapshots the mapped environment values at creation.
- An explicit `Credentials` object is a whole override, including
  `const Credentials()`, empty strings, null members and missing members. No
  member is filled from environment defaults. The parameter is non-nullable:
  literal `credentials: null` is a static type error; a dynamic null raises
  `TypeError` before any environment lookup.
- `environment: String? Function(String)?` optionally injects a portable lookup.
  For example, `environment: (name) => values[name]` over a mutable
  `Map<String, String>`. Existing clients retain their snapshot; new clients
  observe new map values. Each distinct mapped variable is read once per client.
- The conditional VM helper reads Dart's cached, read-only `Platform.environment`
  view inside the creation-time function. No module/import or per-request lookup
  occurs. Mutable in-process configuration uses the injection seam.
- Browser/compiled-JS builds select a stub that returns unavailable values, even
  when Node has process environment variables. Explicit credentials and injected
  lookups work in the portable library.
- Missing/empty/unavailable, oversized or unusable environment values remain
  missing. Anonymous operations stay usable. Protected calls lacking a complete
  alternative throw `ConfigurationException` before transport, with bounded,
  secret-free messages. Existing OR/AND and explicit alternative rules apply.

The public `Credentials` constructor remains source-allocated. New implementation
symbols are private; configured helpers do not reserve or steal source model
names. Source-required deprecated types remain publicly marked deprecated; the
configured generator scopes internal codec/example lint handling to those
necessary references. Unconfigured output keeps its original bytes.

Native tests cover installed VM process-env controls, compiled JS/portable
controls, the real OpenRouter source schemas, compiled VM/JS source examples,
zero-warning dartdoc and Chrome pages. The constructor's non-nullable
`credentials` signature (literal null rejected statically, dynamic null raising
`TypeError` with zero env reads/HTTP) has the focused selector
`native_credential_env_constructor_compatibility`.

Exact native selectors in `--test dart` are
`native_credential_env_controls` and `native_openrouter_credential_env`, with
`-- --exact --ignored --nocapture`. `SUSPECT_DART_BIN`,
`SUSPECT_DART_REPO_ROOT` and `SUSPECT_DART_GATE_ROOT` select the usual native tools
and output root. The source gate accepts `OPENROUTER_WEB_ROOT` for the actual
source checkout. Support files are conditional `environment.dart`,
`environment_io.dart` and `environment_stub.dart`; the emitted README and
`doc/CREDENTIAL-ENV.md` describe the configured mapping and platform behavior.

## Native values and validation

Required native values use `required T` / `required T?`. Optional values use
`Presence<T>` / `Presence<T?>`; `Absent()` and `Present(null)` encode differently.
Defaults from schemas are not inserted. Constant getters are projected only when
the complete checked field schema accepts the literal; contradictory type/const
fields stay compilable and their codecs reject invalid values.

`JsonNumber` / `JsonInteger` retain original numeric spellings, including
`9007199254740993.000000000000000001`, `1.0` and `1e+000008`. Mathematical comparison
and divisibility use symbolic decimal exponents and `BigInt`, without expanding
huge powers of ten. `toBigInt(maxDigits:)` bounds explicit expansion. No general
JSON number passes through `double`, `num`, `jsonDecode` or a host JSON parser.

Objects have mutable native fields and preserved typed/open extra fields. Every
encode revalidates models, nested collections, extra-key collisions and cycles.
Tagged unions are sealed interfaces with the actual branch classes; other unions
have explicit branch wrappers. Non-native intersections, tuples and heterogeneous
literals have checked exact-JSON wrappers. Static aliases/references and recursive
objects keep source identity.

JSON strings and keys admit Unicode scalars. Invalid UTF-8, unpaired UTF-16
surrogates and duplicate decoded keys reject; valid pairs and distinct Unicode
normalization forms remain distinct. Findings retain original schema/keyword and
escaped instance pointers. Completed mismatch and incomplete evaluation are
different outcomes. Branch trials share root validation work; multipart/form part
conversion also shares one request-wide conversion/evaluation budget.

### OpenAPI 3.0 sources

OAS 3.0 Schema Objects are lowered onto the same checked program and native
models ([ADR 0016](adr/0016-closed-dialect-set.md)). `nullable: true` widens only
a same-object `type` (`type: string, nullable: true` is `String?`; a nullable
object is `Owner?`); it cannot bypass an `enum`/`const` or composition, and
without a same-object `type` it changes nothing. Boolean `exclusiveMinimum` /
`exclusiveMaximum` make the adjacent `minimum`/`maximum` exclusive. A Reference
Object applies only its `$ref`: sibling `type`, `nullable`, `readOnly` or
`description` neither shape the native type nor document it, so a `$ref` to a
nullable schema is nullable through its target. 3.0 `type: string, format: binary`
octet bodies are native `Uint8List`. `OpenApi30` is recorded in the manifest
capability set only for sources that declare OAS 3.0, so 3.1/3.2 package bytes
are unchanged. Directional `readOnly`/`writeOnly` (and therefore 3.0
direction-dependent `required`) remain located refusals.
`dart_sdk::native_oas30_nullable_bounds_and_reference_siblings` installs and runs
a consumer of `tests/fixtures/oas30/nullable-bounds-refs.openapi.yaml`.

## Scoped validation v2

The HTTP planner first uses `OwnedCompiler::compile_v2`. A closure that needs only v1
keeps its v1 program, executor and native representation. A closure requiring
modern applicators selects `validation_v2.dart` and the exact
`suspect.validation.experimental.v2` /
`oas31-jsonschema202012-static-applicators` pair. Typed operands, targets, source
locations, patterns and profiles pass `OwnedProgram::check()` before lowering.
Unknown/mismatched envelopes are refused. V1/v2 envelopes reject resource/dynamic
instructions; resource closures use the v3 profile below.

The nine new instructions implement:

- `If`: one condition trial and only the selected then/else branch.
- `DependentRequired`: presence triggers, including present JSON null.
- `DependentSchemas`: the complete object in a fresh child scope.
- `Contains`: every item trial, exact explicit counts and the semantic default
  minimum one, without an invented numeric operand.
- `PatternProperties`: every matching pattern applies conjunctively.
- `AdditionalPropertiesWithPatterns`: declared/matched names are excluded based
  on their names, independently of whether their value validators pass.
- `PropertyNames`: full validation of decoded names at their member pointers.
- `UnevaluatedProperties` and `UnevaluatedItems`: only unmarked locations in the
  current schema object's scope, after its other checks.

Children start with fresh evaluated-property/item sets. Successful refs and
same-instance compositions propagate sets; `anyOf` merges every passing branch,
`oneOf` only a sole passing branch, and `not` discards its child's sets. A passing
condition or successful contains annotation contributes locally even when a
selected branch or a separate contains count assertion fails. Invalid complete
schemas export empty sets. No property-list flattening substitutes for scope.

Node/check/collection/branch visits, NFA transitions and merge candidates use the
shared evaluation allowance. Duplicate merge candidates cost a visit. Object
visits and merges use decoded Unicode-scalar lexical order, including
supplementary keys. Trials retain shared equality, numeric-byte, depth, active
identity and work limits. Failure remains noninvertible even after a passing
branch or an exhausted reporting cap. Exact counts retain `1.0`, `-0.0` and huge
symbolic exponents without float conversion or exponent expansion.

### Faithful models, examples and capture

Typed object constructors keep their declared fields. Pattern/scoped extras use
`DartExtras::Checked` and native `Map<String, JsonValue>` when a single static
extra-value type cannot represent them. Matched fields survive
`additionalProperties: false`, and a pattern-only required field is not narrowed
to the additional-properties schema. Complete-object codecs validate decoded
values and revalidate mutable fields/maps before every encode or HTTP send.

Nullable typed values retain Dart null and explicit `Presence` semantics.
Checked exact-JSON wrappers and their v2 aliases consistently carry `JsonNull`
inside the wrapper. `ModelPlan::uses_native_null` distinguishes that representation
from a nullable Dart reference; semantic nullability remains on `DartModel`.
Typed compatibility records describe these carriers and v2 codec obligations,
without using compiler indices as interface identity.

V2 selections use the shared `examples::plan_protocol_examples_v2` helper, with
bounded discovery/synthesis, original Example Object/dataValue provenance and
located invalid-example findings. Values are lowered through actual native
constructors and codecs. The generated README includes the scoped guide inline
so dartdoc renders it; `doc/SCOPED-VALIDATION.md` is also shipped in the package.

The executable rules are documented in
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md), and native adoption
requirements in [SDK-SCHEMA-NATIVE-ADOPTION.md](SDK-SCHEMA-NATIVE-ADOPTION.md).

### V2 tests and selectors

The vector gate reads the maintained
`crates/suspect-schema/tests/fixtures/owned-applicators-v2.json`, builds real
Contracts and calls `compile_v2`. It checks every finding's original
document/schema/instance location, independently derived step counts and
budget-sweep assertions. Added controls cover exact counts, shared equality and
numeric failures, key identity/order, recursion, error caps and
successful-annotation isolation.

The SDK gate covers `scopedEcho`, `optionalPatch`, `mixedExtras` and `scopedRows`.
It verifies real socket exchanges including the exact README quickstart,
negative-type controls, source examples on VM and JS, lazy per-item
validation/cleanup, and zero-warning dartdoc with rendered symbol checks. Hosted
archive SHA-256, offline pub resolution and every installed file's bytes are
verified before running consumers.

Exact native test selectors:

```sh
SUSPECT_DART_REPO_ROOT="$PWD" cargo test -p suspect-codegen \
  --no-default-features --features dart-sdk --lib \
  dart_sdk::validation_tests::native_v2_source_vectors \
  -- --exact --ignored --nocapture

SUSPECT_DART_REPO_ROOT="$PWD" cargo test -p suspect-codegen \
  --no-default-features --features dart-sdk --lib \
  dart_sdk::sdk_v2_tests::native_v2_sdk_operations \
  -- --exact --ignored --nocapture
```

`SUSPECT_DART_BIN` selects the toolchain. The default (floor) is
`target/sdk-dart-tools/dart-sdk/bin/dart`; current is
`target/sdk-dart-tools/current-3.13.3/dart-sdk/bin/dart`. Use absolute paths.
`SUSPECT_DART_GATE_ROOT` selects the output parent, with a unique
subdirectory for each gate. The optional browser harness is
`tests/dart_support/browser.mjs`; pass an output root and a manifest of exact
compiled script paths/expected completion markers. It uses installed Chrome
and Node built-ins, with an isolated profile and loopback server.

## Canonical resources and dynamic references: v3

The existing `plan_sdk` / `plan_sdk_with_profiles` entrypoints prefer the static
compiler. If static admission fails, they explicitly try `compile_v3` and require
its checked `suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic` pair. `SchemaResources` and
`DynamicSchemaReferences` are enabled.
Ordinary v1/v2 program, executor and model bytes stay on their established paths;
there is no new canonical-dispatch API or mandatory generation option.

The executor enters each node's indexed resource, including nested entrypoints,
without evaluating a resource root to establish scope. Dynamic lookup selects
the outermost actually entered matching binding before entering a fallback.
Pointer, empty-fragment and static-anchor fallbacks do not override. Unentered
catalogue candidates remain inert. Returns, failed trials and failures restore
scope; cycle keys include node, instance identity and the exact ordered context.
Each new distinct resource, scanned resource and inspected binding spends shared
work. Targets begin fresh annotation scopes and publish successful annotations.
No schema loading, URI lookup or acquisition happens during validation.

`validation_v3.dart` supplies node-entry and dynamic-dispatch overrides for the
frozen v2 engine. V3 composition changes only the private base-class declaration;
every ordinary applicator, equality and numeric rule is reused verbatim. V1/v2
emission does not acquire that superclass, resource tables or dispatch hooks.
Typed guards verify the envelope, resource/node alignment, physical containment,
logical aliases/addresses, bindings and initial targets before source emission.

`DartShape::Contextual` represents a dynamic field as exact `JsonValue`. Its
fallback type is not used as a universal native type: an outer binding may turn
a string into an integer or a different object. Enclosing native objects remain
typed, and their complete codecs validate current values in the actual resource
scope. Standalone field codecs use their own indexed entry scope. Typed capture
records these context-bound carriers and the v3 codec profile. Findings and
ownership retain physical SourceIds, including redirected schema documents;
logical `$id`/`$self`/anchor identities remain separate metadata.

V3 examples use the shared `plan_protocol_examples_v3`, retaining bounded generation,
original examples and validation against the complete source schema. The emitted
README renders the resource guide inline; `doc/RESOURCE-VALIDATION.md` and the
full checked program are also included in the installed package.

Native v3 tests on **3.9.4 and 3.13.3**:

- **44 unmodified official cases** from the maintained
  `resource-conformance/` fixture plus independent controls covering resource
  costs, budget sweeps, exact finding locations and post-return scope
  restoration. The supplied remote documents use a closed provider and the real
  `compile_v3` seam.
- Installed SDK: strict versus loose recursive resources, dynamic integer
  override of a string fallback, redirected/logical static references, per-item
  streams, independent socket exchanges, negative-type controls and compiled
  VM/JS source examples.
- The exact README quickstart runs against the fixture server. Strict package
  and consumer analysis, zero-warning dartdoc and rendered contextual-field
  symbols are checked on both tiers. Every installed package file is compared
  with emitted bytes, including after offline pub resolution.
- Chrome runs both tiers' vector, SDK and example JavaScript artifacts.

Exact library-test selectors are
`dart_sdk::v3_tests::native_v3_source_resources` and
`dart_sdk::v3_sdk_tests::native_v3_sdk_operations`, using the same
`-- --exact --ignored --nocapture` and `SUSPECT_DART_*` controls as v2. The
stable executable contract is [SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md).

## Protocol behavior

All source decisions come from `http_protocol::ProtocolPlan`; Dart does not
reinterpret raw HTTP declarations or fabricate schemas for opaque data.

| Concern | Native behavior |
| --- | --- |
| Security | Absent/disabled/anonymous, OR alternatives, AND requirements; bearer, Basic, header/query/cookie API keys |
| OAuth/OIDC | Caller `CredentialProvider` hooks receive scopes/roles, scheme/operation source and flow/discovery metadata; return explicit authorization values. An `sdk_defaults` OAuth policy opts in to a generated token lifecycle |
| Servers | Effective choices, relative URLs, literal variable substitution, declared defaults/enums and explicit overrides |
| Methods | Standard methods, `QUERY`, exact case-sensitive custom tokens from `additionalOperations` |
| Parameters | Simple/label/matrix/form/space/pipe/deepObject, headers, cookies, complete JSON/text content, reserved expansion and 3.2 whole-querystring JSON/text/form |
| Status matching | Exact → class range → default; actual status determines success/error |
| Media matching | Concrete → type wildcard → any wildcard, then declared parameter specificity; no fallback to a less-specific status on media failure |
| Payloads | Exact JSON/+json, UTF-8 scalar text, bounded bytes, schema-free JSON, undeclared bounded bytes and explicit `NoBody` for HTTP-forbidden content |
| Headers/links | Source-typed headers and immutable link metadata; no linked operation is inferred or invoked |
| Forms/multipart | Typed native fields/bytes/part headers, repeated parts, part codecs, structural rules, bounded framing and boundary-collision checks |
| Streams | Standard 3.2 SSE envelope and JSON-lines `itemSchema` as lazy, backpressured native streams |

`Credentials` members come from allocated security names. String credentials are
used for bearer/API keys, `BasicCredentials` for Basic, and callbacks for
OAuth/OIDC. The default selects the first source-ordered complete alternative;
`securityAlternative:` chooses explicitly. Attachments are staged before sending,
and conflicting headers/query/cookies reject instead of overwriting each other.
A source scheme named `apiKey` can still be HTTP bearer. Unless the package was
generated with `credential_env` or `sdk_defaults` (see
[SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md)), the application supplies
environment lookup, token acquisition and refresh.

`ServerSelection(index:, variables:, documentUrl:)` selects a server. Relative
servers resolve against the HTTP document URL, not schema `$id`/API naming
conventions. A local source file requires a supplied HTTP document URL.
`Client(server: Uri.parse(...))` is an explicit override. Malformed escapes,
userinfo/query/fragment server values and unstable path representations reject.

`DocumentRelativeServers` is enabled. The default base
comes from `ServerPlan::document_base()`: the effective physical retrieval
document containing the Server Object. An absent default belongs to the entry
document; an explicit empty override belongs to its declaring document.
`ServerInfo.documentBase` retains that root separately from `source`, while
logical resource addresses remain in the typed protocol provenance/manifest.
`ServerSelection.documentUrl` remains an explicit caller override.

The emitted URI implementation resolves literal dot segments using RFC 3986 and
retains escaped dots, escaped slashes, duplicate slashes and path case. This is
needed because Dart's default `Uri.parse` removes escaped-dot segments. The
transport still exposes `Uri`; its path/string and `removeFragment` views retain
the exact HTTP target. OAuth/OIDC hooks expose `CredentialInfo.urlBase` and
`CredentialRequest.serverUrl`, the selected effective server before appending
the operation path. Metadata endpoints remain original strings for the caller.

`dart_sdk::document_tests::native_document_relative_servers` runs installed
VM/JS consumers on both tiers and in Chrome, with real exchanges across two
independent origins. It covers redirect aliases, different `$self` names,
inherited/default/empty/relative/variable servers, literal versus escaped
dots/slashes, file-source overrides and OAuth/OIDC bases.

Response alternatives and multipart/request content choices have allocated sealed
native types. A default response can produce a success or an API exception, with
the actual HTTP code retained. HEAD/1xx/204/205/304 have `NoBody`; empty bytes and
JSON null remain different values. Binary schemas and mixed multipart aggregates
are not evaluated using a substitute JSON value.

### Streaming

Stream operations return lazy `Stream<AllocatedStatusType>` values. Each record's
`.data` is the native `itemSchema` value. HTTP starts only when listened to. A
listener pause stops item production and applies input backpressure; listener
cancellation, caller cancellation, deadline and client close release the body and
connection. Late transport responses are closed and late errors observed.

SSE processing follows the standard parsed-envelope mapping: multiline `data`
remains a string, `retry` is an exact integer, comments/unknown fields are ignored,
invalid id/retry fields are ignored, and absent fields stay absent. It performs no
JSON-in-data parsing, sentinel handling or reconnection. `[DONE]` is ordinary data.
JSON lines are individually parsed and validated with exact number tokens. SSE
EOF does not invent a missing blank-line dispatch.

## Transport and finite resources

The portable `HttpTransport` supplies status, repeated headers, a byte stream and
an idempotent release callback. Core checks apply to injected adapters too.
Clients own their transport. Closing is memoized before callbacks, including
re-entrant cancellation observers. Raw captures are immutable bounded prefixes
on successful and failed responses; truncation is explicit.

The VM adapter normally uses one `HttpClient`/connection task per exchange. It
disables redirects, proxies, automatic auth, decompression and shared cookie
state while retaining standard TLS verification. Native testing found that
`HttpClient` uppercases method strings. Case-sensitive custom methods therefore
use an independently bounded HTTP/1.1 socket path, with standard TLS, exact tokens,
strict framing, interim/chunked handling and cancellation before response headers.

Defaults: request/response/part limits 8 MiB, raw capture/headers 64 KiB, stream
item 1 MiB and incoming stream chunk buffer 2 MiB. Native parts cap physical count
at 16,384 before assembly. JSON/conversion depth is 128 and their work ceilings are
32 Mi visits/string units. Owned evaluation/equality defaults are 100,000 visits,
4,096 numeric-token bytes and 100 findings. Client byte limits may only decrease;
capture must fit response. The default whole-exchange deadline is 30 seconds,
with positive overrides up to one day.

These are per-call bounds, not a process-memory/performance benchmark. HTTP
transport libraries have their own initial buffering; the core checks retained
headers/body/stream values independently. Exceptions distinguish API, unexpected
status, media, payload, transport, cancellation, deadline and resource failures.
Automatic messages omit credentials, URLs and body contents.

## Explicit generation profiles

`plan_sdk(...)` uses an empty profile set. The additive API is:

```rust
dart_sdk::plan_sdk_with_profiles(
    contract,
    &selected,
    config,
    &generation.compatibility_profiles,
)?;
```

`GenerationOptions.compatibility_profiles` is handled by the backend/session
pipeline and retained in native snapshots. Only explicitly requested profiles are
applied to Dart's capability fence. `LegacyBinaryStringV1` admits the legacy
string/binary marker in binary media/parts; actual native `Uint8List` bytes are
used. It does not change JSON media into bytes. The default still refuses that
nonstandard marker in 3.1/3.2. Both runtimes have installed-package byte and JSON
control tests.

## Admission boundaries

Unimplemented/undefined declarations retain located diagnostics: unnamed
operations, directional codecs (including OAS 3.0 direction-dependent `required`), undefined style/explode/type combinations,
arbitrary untyped form extras, JSON cookie content without an escaping policy,
positional/nested multipart, form/multipart response parsing, request item streams,
aggregate stream assertions, JSON-sequence framing and mixed stream/error-content
profiles. Source `itemSchema` is required for 3.2 streaming. Native plans cannot
inherit capabilities merely because the shared planner recognizes them.

## Typed Rust and compatibility surface

`Plan` retains the canonical Contract, admitted protocol, checked `OwnedProgram`,
compiled schema, native model graph and v2 examples. Rendering is deterministic
and filesystem-independent. `PlannedOperation`, `PlannedParameter`, `PlannedBody`,
`PlannedStatus`, `PlannedMedia`, `PlannedPayload`, aggregate/part/header and
credential descriptors expose actual allocated names and source bindings.

`compatibility/dart.rs` consumes those types directly. It records real typedefs
and erased signatures, fixed getters/presence/defaults, parent membership,
constructors, codecs, native status/media/part classes, typed headers,
Future/Stream returns, security alternatives and client options. Internal program
indices never become interface identities.

## Verification

The advanced tests build an archive from emitted bytes, install it through an
independent loopback pub server into an isolated cache, prove offline resolution,
and compile consumers against those installed bytes. They run strict analysis,
positive/negative type controls, source/quickstart executables and native dartdoc.
Independent wire fixtures assert methods, query/header/body bytes and observe
peer EOF on cancellation. The JavaScript consumer executes the same portable
auth/media/form/framing/backpressure tests under Node and Chrome.

To run the protocol suite:

```sh
SUSPECT_DART_REPO_ROOT="$PWD" \
  cargo test -p suspect-codegen --features dart-sdk \
  --test dart dart_protocol:: -- --include-ignored --nocapture --test-threads=1
```

Set `SUSPECT_DART_BIN` to
`target/sdk-dart-tools/current-3.13.3/dart-sdk/bin/dart` for current-runtime checks.

Primary protocol references: [OpenAPI 3.2.0](https://spec.openapis.org/oas/v3.2.0.html),
especially sequential media/SSE mapping and encoding-by-name; [WHATWG SSE](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream);
[Dart HttpClient](https://api.dart.dev/dart-io/HttpClient-class.html),
[SecureSocket](https://api.dart.dev/dart-io/SecureSocket-class.html), and
[SDK archives](https://dart.dev/get-dart/archive).

## DX comparison scope

Readable operation-role model names, direct native construction, explicit
credential/server controls, sealed content choices and executable guides are the
Dart DX priorities. Flat operation naming, explicit presence/exact-number types,
some long nested names and validated wrappers for non-native schema compositions
remain tradeoffs. Published vendor SDKs are a breadth/usability reference, not a
parity target ([ADR 0030](adr/0030-no-drop-in-parity-with-published-sdks.md)).
