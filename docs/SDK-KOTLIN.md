# Kotlin/JVM SDK

The Kotlin backend emits native Kotlin data/sealed models, exact JSON codecs,
coroutine operations and an installable Maven module. It consumes the canonical
contract and checked portable validation program. Generated code has no Java-SDK
dependency. Its native HTTP profile consumes the shared OpenAPI 3.1/3.2 protocol
descriptors, including source-defined authentication, media, multipart and item
streams. Models use the checked static JSON Schema 2020-12 profile.

## Compiler interface and artifacts

Enable `suspect-codegen`'s `kotlin-sdk,http-protocol` features and call:

```rust
suspect_codegen::kotlin_sdk::plan_sdk(contract, &selected, SdkConfig {
    group_id: "com.example".into(),
    artifact_id: "openrouter-sdk".into(),
    version: "0.1.0".into(),
    package_name: "example.sdk".into(),
    credential_env: None,
})?.render()
```

`contract` is an `Arc<Contract>` and `selected` contains canonical operation
`SourceId`s. Packaging configuration does not change API semantics. The shared
artifact writer receives paths beneath **`kotlin/`**:

- `src/main/kotlin/<package>/`: models, codecs, client and bounded protocol runtimes.
- `src/main/resources/<package>/validation.json`: checked `OwnedProgram`.
- `src/main/resources/<package>/protocol.json`: retained shared protocol plan.
- `src/test/kotlin/<package>/`: executable constructor examples and quickstart.
- `pom.xml`, `README.md`, native reference and source/example coverage manifests.

`Plan::operations()` retains allocated input/result/error names, constructors,
parameter names and types, body/codec bindings, and every status constructor.
`Plan::models()` exposes typed `Shape`, `Field`, `Additional` and `Symbol`
descriptors, including nullable types, fixed tags, union alternatives and actual
codec symbols. `Plan::program()`, `examples()` and `contract()` retain their
canonical inputs. Compatibility consumers do not need to parse emitted Kotlin.

`plan_sdk_with_profiles(contract, selected, config, &profiles)` is the additive
explicit-interpretation API. `plan_sdk` delegates with an empty profile set.
The shared backend's `GenerationOptions.compatibility_profiles` flows into this
API and into native compatibility snapshots and session cache identity.

## Native calls and public errors

These are the actual names from the five-operation OpenRouter acceptance package
using package `example.sdk`. A sole successful payload is available directly as
`result.data`; exact status cases remain available for exhaustive matching.
No-input operations have default inputs, so `getCredits()` needs no empty object.

```kotlin
import example.sdk.*

suspend fun createKey(client: Client) {
    val created = client.createKeys(
        CreateKeysInput(body = CreateKeysBody(name = "Native Test Key"))
    )
    println(created.response.status) // 201, as declared by this operation
    val requiredNullable: String? = created.data.data.updatedAt
    println(requiredNullable)
}
```

The inner `data` is a real OpenRouter JSON-envelope property. The outer `data`
belongs to the SDK result containing HTTP metadata. Models have readable names
such as `CreateKeysBody`, rather than source-pointer-shaped names. The exact
pointer remains in the descriptor and documentation.

```kotlin
import example.sdk.*

suspend fun credits(client: Client) {
    try {
        val response = client.getCredits()
        println(response.data.data.totalCredits.token)
    } catch (error: GetCreditsApiException.Status401) {
        println(error.data.error.message)
    } catch (error: SdkException) {
        println(error.kind)
        throw error
    }
}
```

Every documented error status has a public nested `StatusNNN` with a validated
payload, bounded response information, operation ID and original source. An
invalid documented error body becomes response validation failure. Custom
transport exceptions remain transport failures with their original cause.

## Authentication, presence and exact values

Credentials are explicit application inputs:

```kotlin
import example.sdk.*

suspend fun withCredentials(token: String) {
    Client(Credentials(apiKey = token)).use { client ->
        client.getCredits()
    }
}
```

The source scheme named `apiKey` is HTTP **bearer**, so it produces
`Authorization: Bearer <token>`. No API-key header convention is inferred from
that name. The SDK does not acquire/refresh credentials or read the environment.
The generated quickstart explicitly reads `API_TOKEN` as application scaffolding.
OpenRouter's management-key guidance remains in operation documentation.

### Explicit environment defaults

`SdkConfig.credential_env` accepts the shared v1 policy. The canonical session
form is `credential_env: {"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}`.
Only the variable name enters the generation plan; source security defines the
attachment. `Plan::credential_env()` exposes the bound declarations, and native
compatibility captures their provenance-free semantic descriptor.

For configured Kotlin packages, `Client()` snapshots the mapped process
environment at creation. The explicit factory is `Client.fromEnv(transport,
options, environment)`, with defaults for all arguments and a
`CredentialEnvironment { name -> String? }` reader seam. Whole explicit
`Credentials` are authoritative: null/empty/missing members are not supplemented.
Kotlin rejects a null whole credential object. Unavailable or attachment-invalid
environment values stay missing; protected operations fail before HTTP, while
anonymous operations remain usable. Reader exceptions are treated as unavailable;
coroutine cancellation retains its existing behavior.

The branded live package is `ai.openrouter:openrouter-kotlin` in namespace
`ai.openrouter.kotlin`. Its normal-token entry point is `client.getCurrentKey()`
(source `GET https://openrouter.ai/api/v1/key`); management-key credits remain an
explicit optional operation. The environment gate uses the actual source schema
and controlled transport responses, never a real account request.

Environment helpers and constructor overloads are emitted only for a configured
policy. Unconfigured packages retain their existing emitted bytes and explicit
constructor defaults.

| Source states | Native type |
| --- | --- |
| Required non-null | Required constructor argument `T` |
| Required nullable | Required constructor argument `T?` |
| Optional non-null | `Presence<T> = Presence.Absent` |
| Optional nullable | `Presence<T?> = Presence.Absent` |

```kotlin
import example.sdk.*

suspend fun presence(client: Client, hash: String) {
    client.updateKeys(UpdateKeysInput(hash = hash, body = UpdateKeysBody()))
    client.updateKeys(UpdateKeysInput(
        hash = hash,
        body = UpdateKeysBody(limit = Presence.Present(null))
    ))
    client.updateKeys(UpdateKeysInput(
        hash = hash,
        body = UpdateKeysBody(limit = Presence.Present(JsonNumber.parse("75.50")))
    ))
}
```

These encode `{}`, `{"limit":null}` and `{"limit":75.50}`. Source defaults are
not inserted. Required string const/singleton-enum tags are codec-owned getters,
so `StandardPayload(text = "plain")` supplies its fixed `kind` without permitting
a contradictory constructor argument.

Numbers use `JsonNumber.of(Long)` or exact `JsonNumber.parse(token)`.
`token` preserves spelling; equality/hash/order are mathematical. Integer checks
accept `1.0`, `1e3` and zero-padded exponents. Exponents remain symbolic, including
values beyond BigDecimal's scale range. `toBigIntegerExact()` bounds expansion to
4096 digits; `toLongExact()` also checks overflow. Neither conversion truncates
fractions. General JSON uses the explicit sealed `JsonValue` domain.

## Validation, cancellation and resource ownership

Codecs validate before native decoding and after native encoding. Selected union
arms are validated independently of the parent, and `oneOf` exclusivity is still
checked. Inclusive unions select the first matching native arm on decode.
Unknown source-permitted members survive through the extra-field store, including
typed extras and distinct composed/decomposed Unicode keys.

Data-class collections can be caller-owned and mutable. Each encode revalidates
them; returned JSON snapshots recursively detach generic JSON extras. Collection
sizes are admitted before Kotlin's preallocating `map`/`associate` operations.
Callers must not mutate a model while its codec is executing.

One codec session shares schema visits, equality pairs, numeric work and
conversion/text budgets across all logical trials and native traversal.
`ValidationException` means completed invalidity; `EvaluationException` means
incomplete evaluation and cannot become success under `anyOf` or `not`.
Nonproductive references fail explicitly. The executor implements the checked
portable NFA instead of substituting JVM regex semantics for schema patterns.

Finite client methods are `suspend`; operations with item-stream responses return
cold `Flow<Result>`. CPU preparation/decoding runs in a structured
`Dispatchers.Default` context so a caller's single-thread timeout scheduler stays
responsive. Configured deadlines cover preparation, HTTP and decoding, becoming
`FailureKind.TIMEOUT`. Caller cancellation and outer `withTimeout` remain native
coroutine cancellation. The JDK future and body subscription are cancelled too.

Collecting a Flow starts its request. The collector owns the response reader;
normal completion, `take(1)`, cancellation and failures close it. A rendezvous
channel provides backpressure. SSE uses the standard 3.2 event envelope:
`data`/`event`/`id` strings and integer `retry`. Data lines join with LF; unknown
fields, comments and invalid id/retry values are ignored. No nested JSON, sentinel
or reconnect policy is inferred. JSON Lines validates each JSON value; blank
records fail. Unfinished SSE blocks are discarded at EOF.

Request `itemSchema` bodies accept `Flow<Item>`. This finite-request profile
collects the producer once under both item and total byte ceilings before I/O.
Producer failure or cancellation prevents a partial request. Part codecs share
the operation's model/evaluation budgets; response items have bounded independent
codec sessions.

`Transport` is a public suspending interface. It must cooperate with cancellation,
bound I/O and release response resources before returning. The client rechecks
returned bytes/headers. A failing transport `finally` cannot replace cancellation.
`Client.close()` closes an `AutoCloseable` transport. Kotlin `use` preserves the
primary failure if close also fails; standalone close failures have safe SDK
messages and accessible causes.

`StreamingTransport.open()` returns `StreamingResponse` with an owned
`BodyReader`. Reader implementations must cooperate with coroutine cancellation
and return finite chunks. The JDK adapter closes streams and cancels read tasks,
including a response that arrives concurrently with cancellation.

Default finite policies:

- 4 MiB JSON/HTTP body and per-URL/request-body ceilings; JSON depth 128,
  100000 values and 4096 bytes per numeric token.
- 100000 native/schema/equality/numeric work units per respective budget;
  16 MiB native-conversion and assertion-text budgets.
- 8192 preview bytes, clamped to the response ceiling; 32 KiB / 128 header
  fields or values, counting empty header lists.
- 1 MiB per stream item and 64 KiB per transport chunk, with lowerable options.

The JDK adapter retains at most the body ceiling plus one sentinel byte, then
cancels oversized input. It disables redirects, cookies, implicit proxies and
automatic credential acquisition. The SDK has no retry or pagination loop.
Malformed UTF-8, duplicate JSON keys, wrong media/charset/encoding, unexpected
statuses, invalid headers and resource failures retain classified outcomes.

## Native package and documentation gates

The generated module runs constructor/codec and coroutine examples during
`mvn verify`. `mvn install` installs Kotlin metadata plus the main, sources,
Dokka `javadoc`, and executable `examples` jars. The README quickstart is lowered
from validated values into native constructors; an installed consumer compiles
and executes that exact snippet with a controlled response.

Dokka runs with `failOnWarning`. The separate coverage gate checks rendered KDoc
pages against actual allocated operation/model/property/codec/union/status
descriptors. Primary-constructor properties carry direct KDoc, rather than
depending on Dokka's class-level `@property` propagation. Synthetic enum
`entries`/`values`/`valueOf` do not have authored KDoc, so Dokka's blanket
`reportUndocumented` setting is disabled; explicit surface coverage remains a
required gate. Example provenance and invalid source examples remain visible.

## Toolchain pins and boundaries

Pins verified on 2026-09-10 against primary repositories and resolved by Maven:

| Component | Pin / actual runtime |
| --- | --- |
| Kotlin compiler and stdlib | **2.4.20** |
| kotlinx-coroutines-core-jvm | **1.11.0** |
| Dokka Maven plugin | **2.2.0** |
| Maven | **3.9.16** |
| JVM floor/current | **21 / 25**, Temurin **21.0.12.1+1 / 25.0.4.1+1** |

Primary availability: [Kotlin releases](https://kotlinlang.org/docs/releases.html),
[compiler artifact](https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-maven-plugin/2.4.20/),
[stdlib artifact](https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/2.4.20/),
[coroutines artifact](https://repo.maven.apache.org/maven2/org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.11.0/),
[coroutines changelog](https://github.com/Kotlin/kotlinx.coroutines/blob/master/CHANGES.md),
[Dokka Maven documentation](https://kotlinlang.org/docs/dokka-maven.html) and
[Dokka artifact](https://repo.maven.apache.org/maven2/org/jetbrains/dokka/dokka-maven-plugin/2.2.0/).
Availability records and actual JVM versions are in `target/sdk-kotlin-tools/`.
Kotlin compilation is in-process; the module does not require a compiler daemon.
Executable Maven examples run in a separate bounded JVM, so Maven does not try
to interrupt the application's shared coroutine dispatcher during goal cleanup.

Native intersections, tuple/prefixItems models, structural ref/union siblings,
active directional annotations and OpenAPI 3.0 produce source-linked diagnostics.
The standalone portable validator has broader instruction coverage than native
model representation, including tuples and unconstrained composition.

## Rich HTTP surface

| Protocol descriptor | Kotlin surface and policy |
| --- | --- |
| Anonymous / OR / AND security | First complete source alternative, or explicit `RequestOptions.securityAlternative` |
| Bearer, Basic, API keys | Source-named `Credentials` fields; `BasicCredentials`; header/query/cookie attachment |
| OAuth/OIDC | Suspending `CredentialProvider` returns a complete Authorization value; `CredentialContext` retains scopes/roles, source and flow/discovery metadata |
| Server choices and variables | `serverIndex`, `serverVariables`, `documentUrl` in client/per-call options; literal defaults/enums and relative retrieval-base resolution |
| Parameters | Source style/explode/content/allowReserved and header/cookie rules; querystring content has no parameter-name prefix |
| Multiple request media | Sealed body choice; `requestMedia` supplies a concrete wildcard representation |
| Responses | Exact → range → default matching with actual status; concrete → type wildcard → `*/*` media precedence and declared media parameters |
| JSON / +json / text / bytes | Checked native model, scalar, or `ByteArray`; schema-free JSON uses `JsonValue` |
| No declared response body | Bounded raw bytes; HEAD/1xx/204/205/304 have `Unit` and skip body consumption |
| Response headers and links | Generated typed header structures; `ResponseInfo.links` retains source expressions without following them |
| Form / named multipart | Native body/part constructors, `Upload` bytes, typed part headers, source cardinalities and extra-field rules |
| SSE / JSON Lines | Cold `Flow<Result>` with typed `itemSchema` values, structured lifetime and finite item buffers |

Relative servers loaded from local files need an explicit HTTP `documentUrl`.
`serverUrl` is a direct environment override. `responseMedia` selects and requires
a successful representation; declared errors retain their own source media.
Reserved expansion preserves valid `%NN` sequences once and rejects active
query/path/style delimiter hazards. Empty or ambiguous composites have explicit
representation errors. Runtime errors preserve causes and bounded metadata.

These constructors are compiled and executed by the rich-protocol installed
consumer (`example.protocol` is that fixture's package):

```kotlin
import example.protocol.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

suspend fun uploadBytes(client: Client, bytes: ByteArray): Reply =
    client.upload(UploadInput(UploadRequestMultipartBody(
        file = UploadRequestMultipartBodyFilePart(
            value = Upload(data = bytes, filename = "example.bin"),
            headers = UploadRequestMultipartBodyFileHeaders(JsonNumber.of(7L))
        ),
        title = "Example",
        meta = UploadBodyMeta(flag = true)
    ))).data

fun eventData(client: Client): Flow<String> =
    client.events().map { it.data.data }
```

Positional/nested multipart, CONNECT tunnels, JDK-restricted request headers,
untyped response-header extras and composite response-form grouping outside the
implemented inverse profile have located native admission diagnostics. Undefined
compatibility interpretations are never silently enabled.

The explicit `legacy-binary-string-v1` profile interprets `type: string,
format: binary` as bytes only in binary media/parts. It does not alter JSON media:
the same marker there remains a Kotlin string and receives ordinary string
validation. Additional unsupported byte-schema assertions still decline through
the shared planner. `capabilities()` includes no compatibility profile by default.

## Reproduction

```sh
cargo test -p suspect-codegen --features kotlin-sdk,http-protocol \
  --test kotlin_sdk --test kotlin_integration --test kotlin_protocol \
  --target-dir target/sdk-kotlin-cargo

cargo test -p suspect-codegen --features kotlin-sdk,http-protocol \
  --test kotlin_sdk --test kotlin_protocol \
  --target-dir target/sdk-kotlin-cargo -- --include-ignored --nocapture --test-threads=1
```

Native tests use isolated `target/sdk-kotlin-native/` packages/consumers/logs and
`target/sdk-kotlin-maven/`. Rich protocol gates use `target/sdk-kotlin-protocol/`.
Override tool discovery with `SUSPECT_KOTLIN_MAVEN`
and `SUSPECT_KOTLIN_JAVA_HOME`; omit the Java override to exercise both installed
JDKs. `OPENROUTER_WEB_ROOT` selects the read-only corpus checkout.

The required matrix is the unchanged M2 fixture; five actual OpenRouter
operations (`getCredits`, `createKeys`, `updateKeys`, `getContainerFile`,
`listContainerFiles`) with independent response fixtures; all 17 shared runtime
vectors; and the independent hostile-name/composition/budget package. Each SDK
package has an installed Kotlin consumer, positive-controlled negative types,
executable source examples and native documentation coverage. Tests retain their
artifacts on success and failure. Source integrity is checked against the pinned
OpenRouter SHA-256; the upstream document is never rewritten.

## Preserved JSON baseline — 2026-09-10

All four native gates passed on both JDKs. Each SDK gate installed the Maven
module, executed an independent consumer, compiled positive-controlled negative
types, ran constructor/coroutine examples and checked actual rendered KDoc pages.

| Gate | Evidence directory under `target/sdk-kotlin-native/` | Per-JDK evidence |
| --- | --- | --- |
| M2 | `NativeM2-C9ddOU` | 20 wire exchanges; 6 negative type cases; 156 documented symbols |
| Five actual OpenRouter operations | `NativeOpenRouter-Af4W3O` | 6 wire exchanges; 4 negative type cases; 519 documented symbols |
| Adversarial models/names/budgets | `NativeAdversarial-8Q2Flt` | 4 negative type cases; 168 documented symbols |
| Shared runtime corpus | `Runtime-L9mb0d` | 17 vectors: 40 valid / 41 invalid instances, plus resource controls |

All three SDK README quickstarts passed through installed consumers. The exact
Kotlin blocks in this document also compiled on both JDKs; their controlled
create/credits/error/presence calls passed in `target/sdk-kotlin-docs-check/`.
`target/sdk-kotlin-json-verified/report.json` records logs, rendered coverage and
hashes of the verified source artifacts. Earlier failed/partial attempts remain
separate and are not counted as passing evidence.

### Current protocol and integration gates

`capabilities()` selects **`kotlin-jvm-protocol-v1`**. `Plan::protocol()` and
`docs/protocol.json` retain the actual shared descriptors and enabled profile
choices. Examples use `plan_protocol_examples` and its v2 body/part/header/item
roles; binary aggregates never enter JSON model or validation roots.

The 13 original compatibility/session tests passed in the byte-exact target-only
harness. Extended tests cover rich wrappers, controls, streams and explicit
profile capture/cache behavior. Current protocol evidence and source manifests
are recorded in `target/sdk-kotlin-protocol-verified/REPORT.md`.

The full native suite still contains the base and rich protocol gates.
`fresh_generation_preserves_expected_m2_native_bindings` compares fresh emissions
and independent native binding expectations. The one-off historical adoption
probe was moved to `target/sdk-kotlin-tools/historical-probes/`; it is not part of
the current `--include-ignored` suite. Tests do not read or write the preserved
`sdk-kotlin-json-verified` reports.

## Scoped validation adoption

For non-resource closures the default SDK planner uses the verified source-driven
`OwnedCompiler::compile_v2` seam. The additive `validation::plan_validation_v2`
and `plan_sdk_v2` entry points expose the same scoped profile. The lower-level
`validation::plan_validation` retains v1 admission. Exact version/profile pairs
are checked before emission; resource/dynamic closures use the separately verified
V3 profile below. Base closures still produce v1 programs and the original
validation-runtime bytes.

The v2 runtime implements the nine operations in
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md), including fresh child scopes,
successful-annotation propagation, exact merge/visit costs, decoded Unicode-scalar
key ordering and noninvertible evaluation failures. Numeric/equality/work state
is shared across trials. The default contains minimum is not a source numeric
operand. Report caps never turn incomplete evaluation into success.

Pattern extras use the explicit `Additional::Scoped` projection and remain in
`additionalProperties: Map<String, JsonValue>`. `additionalProperties: false`
does not close or discard keys matched by a source pattern. All matching value
schemas and the whole parent schema still run on both codec directions.

`Shape::CheckedJson` represents constraints that cannot be faithfully expressed
as static Kotlin fields. It emits an allocated data class with a `value: JsonValue`
constructor/property and a source-bound codec, rather than flattening composition
or losing tuple members. Constructors/copy can hold values that have not yet been
validated; SDK operations perform checked encoding, and model-only use has an
explicit codec obligation. Decoding detaches generic JSON snapshots.

These are actual constructor/codec names in the scoped acceptance package:

```kotlin
import example.scoped.sdk.*

fun scopedPattern(): PatternRecord {
    val value = PatternRecord(
        label = "scoped guide",
        additionalProperties = mapOf("n_even" to JsonNumber.of(4L))
    )
    Codecs.patternRecord.encode(value)
    return value
}

fun scopedCarrier(): ConditionalCarrier =
    Codecs.conditionalCarrier.decodeJson(JsonString("scoped guide"))
```

Maintained gates are in `tests/kotlin_validation_v2.rs`. They compile the 32
independent source cases from `suspect-schema/tests/fixtures/owned-applicators-v2.json`
through the real v2 compiler, then execute the native kernel under each case's
limits and check source/instance locations. Additional controls cover exact work
thresholds, duplicate merges, property-name identity, recursion, Unicode ordering,
count bounds and reporting/equality failures. The SDK gate verifies actual
operations, installed consumers, negative types, constructor examples and Dokka
on JDK 21/25. These tests have no dependency on previous target witnesses.

```sh
cargo test -p suspect-codegen --test kotlin_validation_v2 \
  -- --include-ignored --nocapture --test-threads=1
```

## Resource/dynamic validation and physical server bases

`plan_sdk_v3` and `validation::plan_validation_v3` explicitly compile the checked
V3 resource profile. The default SDK planner selects this verified profile only
when its actual codec closure has canonical-resource/dynamic requirements; other
closures remain on V1/V2. Its exact version/profile pair and aligned resource/node
tables are rechecked before native execution. Dynamic references enter only the
selected target's indexed resource after lookup; the outermost actually entered
matching resource wins. Pointer/empty/static-anchor fallbacks stay static.
Resource state restores across returns and trials, and cycle identity includes
the exact ordered context. The runtime never retrieves schemas.

Dynamic slots use checked JSON carriers so an override cannot be misrepresented
as its fallback's static Kotlin type. For example, the resource acceptance SDK
has a numeric array whose dynamic item binding can be overridden by a string
resource; both use the same lossless carrier and different complete codecs.

```kotlin
import example.resources.sdk.*

fun dynamicStrings(): List<NumbersItem> {
    val values = listOf(NumbersItem(value = JsonString("dynamic guide")))
    Codecs.strings.encode(values)
    return values
}
```

The physical-document server witness uses supplied requested/effective document
aliases, distinct `$self` identities, referenced Path Items, absent versus empty
server arrays, local-file explicit overrides, encoded dots/slashes and significant
empty path segments. `CredentialContext.effectiveServer` supplies the selected
base for caller-owned relative OAuth/OIDC metadata URLs. Logical IDs do not
relocate HTTP endpoints.

Maintained V3 cases come from the unmodified official `resource-conformance`
fixtures through a closed provider and real `compile_v3`. Exact native selectors:

```sh
cargo test -p suspect-codegen --test kotlin_validation_v3 \
  native_resource_44_vectors -- --ignored --nocapture
cargo test -p suspect-codegen --test kotlin_validation_v3 \
  native_resource_sdk_operations -- --ignored --nocapture
cargo test -p suspect-codegen --test kotlin_protocol_documents \
  native_physical_document_servers -- --ignored --nocapture
```
