# Java SDK: native HTTP protocols and exact checked models

The `java-sdk` feature implements native immutable models/builders, exact checked
codecs, the portable `OwnedProgram` validator, synchronous and
`CompletableFuture` HTTP, Maven packages, Javadoc and executable source-bound
examples. Generated artifacts are rooted under **`java/`**.

The accepted base uses the original M2 fixture, five actual OpenRouter
operations, independent hand-authored response bytes and all 17 shared runtime
validation vectors. Toolchains are JDK **21.0.12.1** and **25.0.4.1**, with Maven
**3.9.16**. Current evidence and exact source hashes are indexed in
`target/sdk-java-verification/REPORT.md`.

The HTTP protocol phase adds ordinary OAS 3.0/3.1/3.2 auth/server/parameter,
status/media/header/link, native bytes, form/multipart and closeable SSE/JSON-lines
support. Its independent byte consumers, stream controls, typed compatibility
records and new actual OpenRouter operations are indexed in
`target/sdk-java-protocol/REPORT.md`. Base and canonical reflection evidence remain
in their original reports and are reused.

Scoped schema-v2 adoption is source-driven through
`tests/java_schema_v2.rs`. The runtime covers all nine instructions, scoped
successful-evaluation annotations, exact counts, fresh branch/key identities and
noninvertible failures. Evidence is retained in
`target/sdk-java-validation-v2/REPORT.md` separately from the HTTP phase.

The explicit resource/dynamic V3 profile is verified through
`tests/java_schema_v3.rs`: all 44 original official dynamic-reference cases,
independent scope/context/budget/malformed controls and installed SDK operations
on JDK 21/25. Its records are in `target/sdk-java-validation-v3/REPORT.md`.

## Consumer interface

These are actual names from the five-operation OpenRouter package configured
with `package = "com.example.generated"` and `api_name = "OpenRouter"`:

```java
import com.example.generated.*;
import com.example.generated.OpenRouter.*;
import static com.example.generated.JsonRuntime.*;

try (var client = new OpenRouter(HttpRuntime.Options.builder()
        .credential("apiKey", token)
        .timeout(java.time.Duration.ofSeconds(30))
        .build())) {
    var credits = client.getCredits();
    JsonNumber total = credits.data().data().totalCredits();

    var body = CreateKeysRequest.builder("Native Test Key")
            .limit(JsonNumber.parse("50.25"))
            .limitReset(null)
            .build();
    var created = client.createKeys(CreateKeysInput.builder(body).build());

    var update = UpdateKeysRequest.builder()
            .name("Updated Native Key")
            .limit(JsonNumber.parse("75.50"))
            .build();
    var pending = client.updateKeysAsync(
            UpdateKeysInput.builder("fixture-hash", update).build());
    var updated = pending.join();
}
```

The actual source scheme named `apiKey` is HTTP bearer. Its header is
`Authorization: Bearer <token>`. The source's management-key requirement remains
in the operation documentation. The SDK applies supplied credentials; it does
not acquire a token or infer permissions from its spelling.

A single declared success returns its concrete result directly. Multiple
successes use a sealed result interface. Declared errors are source/status-bound
`SdkException` subclasses, such as `OpenRouter.GetCreditsStatus401`, exposing
typed `data()` and immutable `headers()`.

The outer `data()` is the HTTP response payload. The inner `data()` in the
credits example is an actual OpenRouter wire field.

### Values and presence

- Required, non-singleton fields are builder-factory arguments. Optional setters
  have an `omitField()` counterpart. Required singleton `const`/`enum` fields are
  supplied from their source-proved value; defaults are never inserted.
- `Presence<T>` distinguishes absent from present. Ordinary nullable scalar and
  object values use Java null. Free JSON uses `JsonNull.INSTANCE`. A nullable
  union retains an explicit typed arm, including a null-only arm.
- Object builds, union constructors and operation-input builds validate and
  deeply snapshot mutable input containers. Published getters and nested
  containers are immutable. Every encode validates again.
- Finite literals have nominal types and named constants, including mixed and
  compound literals. A decoded numeric literal retains its original spelling.
- `oneOf` checks the selected native arm and parent exclusivity. `anyOf` decoding
  picks the first matching typed view and retains its complete wire data.
- Static aliases and recursive objects/unions retain their source-specific
  codec bindings. JSON numbers and mathematical integers use exact
  `JsonRuntime.JsonNumber`, with symbolic exponents and explicit bounded integer
  conversion. No float or bounded decimal intermediate is used.

`Model.CODEC` provides `decode(String)`, `decode(byte[])`, `decodeValue`, `encode`,
`encodeValue`, `snapshot` and `withLimits`. Source holder classes also expose
`decode(String)` and `encode(value)` conveniences. Internal conversion methods
are private/package-private; public codecs always perform checked calls.

Scoped v2 objects keep declared native fields while preserving pattern-matched
extras as immutable JSON values. Each matching pattern and the correct
additional-properties rule is checked on construction, decode and encode.
Required keys not declared as fixed fields use `putAdditionalProperty` and are
checked at build time. Where intersections, heterogeneous prefix items or
constrained unions cannot be expressed faithfully as one native static type,
the source retains a `ModelCodec<JsonValue>` holder. Callers use its checked
`decode`/`encode` operations; generated HTTP boundaries enforce that obligation
automatically. Ordinary v1 declarations keep their established representations.

### Resources, cancellation and injection

Each codec call shares its JSON, conversion/byte-work, validation-visit and
equality budgets across all nested values, generic JSON and union trials.
Builder snapshots use one such session for write, validation and read.
Exhaustion remains distinct from ordinary schema invalidity.

Default JSON limits are 8 MiB input/output, nesting 128 and 32 MiB work. Native
conversion/byte work has a separate 32 MiB allowance, with 100,000 schema visits
and 100,000 equality pairs. Schema numeric operands have the compiled program's
4,096-byte limit; standalone exact JSON tokens allow up to 65,536 bytes under a
finite work limit. Exponent magnitude does not cause decimal expansion.
The emitted validation metadata is admitted against its own 8 MiB byte ceiling,
depth 128 and conservative 32 MiB loading-work allowance before artifacts are
returned. Declared-property membership tables are immutable, cached data.

HTTP options expose request, response, URL and capture ceilings plus codec
limits. URL expansion is checked incrementally, including repeated form-array
query keys. The response subscriber bounds bytes before accumulation and
cancels on overflow. Failure captures are explicit defensive copies; exception
messages omit bodies, credentials and raw transport messages. Codec failures
retain original `schemaSource()` and `instancePath()`.

The configured deadline includes encoding, transport, body completion and
decoding. Streaming responses retain it until EOF or close, including item
conversion and zero-demand Flow subscriptions. It also applies to injected
`java.net.http.HttpClient` instances.
`future.cancel(true)` propagates to work and HTTP subscriptions. Sync
interruption preserves the interruption flag and cancels the exchange. Cleanup
exceptions cannot replace primary resource failures or cancellation.

`close()` releases owned HTTP, worker and deadline resources and terminates
in-flight calls. Injected clients retain caller ownership and must disable
redirects, cookies and implicit authentication. SDK-owned transports also
disable environment proxy selection. The SDK adds no retries or pagination
loops.

### Native protocol surface

- Security preserves anonymous/OR/AND choices. Options provide bearer
  `credential`, UTF-8 `basic`, header/query/cookie `apiKey`, and caller-owned
  `authorization` hooks for OAuth2/OIDC. `CredentialContext` exposes original
  flow/discovery/source metadata and scopes/roles. No login, token refresh or
  permission inference is built in.
- `RequestOptions` adds per-call server URL, source index/name, variable,
  document URL, security alternative, Accept and timeout controls. Relative
  servers resolve against retrieval URLs; file specs require an explicit HTTP
  document URL. Custom method tokens retain exact case, including custom `head`.
- Parameter styles/content/reserved expansion, headers/cookies and OAS 3.2
  whole-query content retain shared normative serialization. Empty composites
  and undefined escaping profiles fail explicitly.
- Actual responses use exact/range/default precedence and most-specific media
  matching. JSON/+json, text scalars, bytes and wildcard/parameterized media have
  distinct codecs. `Bytes` owns immutable octets, `NoContent` represents
  HTTP-forbidden content, and `ResponseBody<T>` distinguishes body-forbidden
  range/default matches from content (including JSON null).
- Generated status objects own streaming bodies and expose `typedHeaders()` and
  metadata-only `links()`. All response/error values are closeable.
- Forms, named multipart and bounded positional multipart have typed immutable
  builders, typed extra/tail fields, native byte parts and metadata-bearing part
  wrappers. Required/cardinality rules and per-part/header codecs are checked
  without making byte or aggregate schemas into fake JSON roots.
- `EventStream<T>` supports a closeable iterator, async
  `CompletableFuture<Presence<T>>` pull and `Flow.Publisher<EventStream.Item<T>>`.
  Null items remain distinct from EOF. SSE parses standard envelopes and keeps
  data as text; JSON lines use strict exact JSON. Byte/work bounds, backpressure,
  cancellation, early close and deadlines are explicit. Request streams are
  finite native lists. No vendor event JSON, sentinel, retry or pagination policy
  is inferred.

Full generated recipes and precise runtime policies are maintained in
`crates/suspect-codegen/src/java_sdk/readme-runtime.md`; the emitter includes them
in every package README. Source-specific call examples and native construction
recipes are emitted in `docs/api.md` and `SdkExamples`.

### Explicit credential-environment policy

`ProtocolConfig.credential_env` accepts an optional shared
`credential_env::CredentialEnv`. It defaults to `None`. Java binds the policy
after protocol admission and exposes `SdkPlan::credential_env()` as an immutable
`CredentialEnvPlan`; generation stores variable names and source bindings only.
The canonical session form is:

```json
{"credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}}
```

Configured packages add these compiled factory overloads:

```java
Client.fromEnv();
Client.fromEnv(java.net.http.HttpClient transport);
Client.fromEnv(java.net.http.HttpClient transport,
               java.util.function.Function<String, String> environment);
```

The process-environment overloads use `System.getenv` at factory creation. Each
distinct mapped variable is read once, bounded to the existing credential size,
and passed only if usable by its declared bearer/API-key attachment. Missing,
empty, invalid or unavailable values stay missing. The accessor overload also
supports unavailable environment access: null return, null accessor or an accessor
exception is missing, with no process-environment fallback. Supplied transports
are non-null, caller-owned and retain the normal JDK transport constraints.

The existing `new Client(HttpRuntime.Options)` constructor is authoritative for
the whole explicit credential set. No member is filled from env, including an
empty set or a partial set. Empty/null explicit credential setters retain their
existing argument validation. Unsatisfied operation security produces secret-free
`SdkException.kind() == "missing-credential"` before transport; anonymous and
satisfiable OR alternatives remain usable. RequestOptions retains explicit
per-call server, deadline and security-alternative controls.

Factories, helper branches, policy files and their documentation are emitted only
when a successfully bound policy exists. The no-policy whole-artifact fixture
retains its sealed 44-file hash. Typed capture records `semantic_descriptor()`
without physical provenance; `credential-env.json` retains the source binding.
Helper names participate in configured-only native name reservation.

The branded actual-source witness uses `ai.openrouter.sdk` and Maven
`ai.openrouter:openrouter-sdk:0.1.0`, with `getCurrentKey` (`GET /key`) as the live
default and `getCredits` as explicit management mode. Native tests intercept the
original `https://openrouter.ai/api/v1` server through a controlled JDK transport.
The compiled support program and exact receipts are indexed in
`target/sdk-java-credential-env/REPORT.md`; no real account call is part of those
tests. Maintained selectors are `java_credential_env::native_credential_env_controls`
and `java_credential_env::native_credential_env_openrouter` on JDK 21/25.

## Maven, documentation and examples

```sh
mvn -B package
mvn -B install
java -ea -cp target/generated-sdk-0.1.0.jar com.example.generated.SdkExamples
```

The emitted POM pins compiler, jar, source, Javadoc and install plugins. It uses
warnings-denied Java compilation and warnings-denied Javadoc with structural
doclint. Maven attaches the binary, source and Javadoc jars. Runtime
dependencies: **none**.

Generated documentation includes:

- A complete `README.md` with the exact compiled `examples/GettingStarted.java`
  source, authentication, errors, timeout, injection and presence recipes.
- Native Javadoc for model fields/builders/literals, codec bindings, operation
  inputs/methods and status variants, with original descriptions and identities.
- `docs/api.md`, `docs/examples.md`, `examples.json`, and `doc-coverage.json`.
- `SdkExamples`: native constructor/builder expressions for validated source
  examples, offline codec verification and typed sync/async call helpers.

Declared and synthesized examples remain labeled separately. Invalid source
examples remain findings. Coverage reports retain unavailable examples explicitly.
The acceptance cases require every selected operation input and retained native
example to be available, compile, and execute. Native documentation coverage
checks the actual rendered symbols and field/codec anchors.

## Library API for Main integration

The Java-owned public entry point remains:

```rust
java_sdk::plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig { package, version, api_name },
    roots: &[SchemaId],
) -> Result<SdkPlan, Vec<HttpDiagnostic>>
```

For explicit build identity:

```rust
java_sdk::plan_sdk_with_maven(
    contract, selected, config, roots,
    MavenConfig { group_id, artifact_id, java_release },
    // defaults: None (Java package), generated-sdk, 21
)
```

`plan_sdk_with_protocol(contract, selected, config, roots, maven, ProtocolConfig)`
additionally accepts `http_protocol::ByteLimits` and an explicit set of
`http_protocol::CompatibilityProfile` values. Existing planning entry points
delegate with the default protocol policy. The retained `SdkPlan::protocol()` is
the rich shared plan; `codec_roots()` includes actual JSON, header, part and item
bindings while native bytes/aggregates remain separate.

`plan_sdk_with_protocol_v3` has the same arguments and is the explicit
resource-aware entrypoint. It returns an already-admitted ordinary V1/V2 plan
unchanged, including its artifact policy. Resource-requiring declarations use
`compile_v3`, the indexed candidate-aware schema closure and
`plan_protocol_examples_v3`. `validation::plan_validation_v3` explicitly compiles
a standalone V3 program. Existing direct SDK/validation entrypoints retain V1/V2
behavior and resource refusals.

`SdkPlan::render()` returns `Result<Vec<OutFile>, Vec<HttpDiagnostic>>`. The
immutable plan exposes `models()`, `operations()`, `program()`, `package()`,
`maven()`, `maven_group_id()`, `contract()`, `protocol()`, `examples()`, `native_examples()` and `openapi_version()`.
Planning does not expose an emission-only `release_ready` claim.

### Canonical registration and comparison

`Backend::JavaHttp` is registered under feature `java-sdk`, with profile
`java-http`, directory `java`, and owner `suspect-sdk:java-http`.
`TargetConfig.package_name` is **Maven `group:artifact`**;
`TargetConfig.import_name` is the Java package and defaults to the Maven group.
The canonical client class is **`Client`**. These identities are independent:

```rust
TargetConfig {
    backend: Backend::JavaHttp,
    package_name: "example.widgets:thing-sdk".into(),
    package_version: "1.0.0".into(),
    import_name: Some("example.sdk".into()),
}
```

The backend maps these values through `backend::java_config` into
`PackageConfig` and `MavenConfig`, then calls `plan_sdk_with_protocol_v3` with no extra
model roots and `backend::java_options(&generation)`. Compatibility capture uses
`backend::java_options(&snapshot.generation)` and the same retained
declarations, including builder factories/setters/omitters, literal constants,
union constructors, source-bound codecs, nested client input/status types,
sync/async/no-input methods and control/credential signatures. Descriptors use
actual qualified names and exclude validation graph indices and source-text
parsing. It also retains sealed media variants, header/part/aggregate builders,
byte and stream types, range/default wrappers and ownership controls.
`group_id: None` remains equivalent to an explicit same-group choice for all
sources/resources except the manifest recording that the option was supplied.
Phase 3 adds protocol artifacts, so the original 68-file phase-2 hash remains
historical evidence rather than an invariant on the expanded package.

`tests/java_integration.rs` covers registry/generation, compatibility deltas,
session warm/configuration reuse, byte preservation and a JVM reflection check
of the new canonical package. Evidence is in
`target/sdk-java-integration/REPORT.md`. The accepted JDK/runtime matrix is reused.

### Exact retained compatibility descriptors

`java_sdk::models` is public:

- `JavaModelPlan::symbols()`, `symbol(&SchemaId)`, `names()`, `fields()`,
  `codec(&SchemaId)`, `native_type(&SchemaId)` and `render_type(&JavaType)`.
- `JavaSymbol::{source, name, description, nullable, declaration, codec}`.
- `JavaDeclaration::{Object { fields, extras, constructor }, Alias(JavaType),
  Union { exclusive, variants }, Literals { values }}`.
- `JavaField`: `name`, `wire`, `ty`, `required`, `nullable`, `source`,
  `description`, `fixed`, `omit_method`. Optional presence is a field concern;
  `ty` is the supplied value type.
- `JavaConstructor`: factory/constructor `name` and ordered `arguments`.
  `JavaArgument`: `name`, `ty`, `source`, `nullable`.
- `JavaVariant`: `name`, `source`, `ty`, `constructor`.
- `JavaLiteral`: allocated constant `name` and original `value`.
- `JavaCodecBinding`: `source`, `holder`, `field` (`CODEC`), `root`, `native_type`.
- `JavaType::{String, Boolean, Number, Json, Null, Never, Named(SchemaId),
  Nullable(Box<_>), List(Box<_>)}`. Alias resolution is retained in the graph;
  `native_type` supplies the actual rendered type.

`java_sdk::http` is public. `SdkPlan::operations()` returns `&[JavaOperation]`:

- Operation: `source`, `operation_id`, `method_name`, `async_method_name`,
  `input_type`, `constructor`, `success_type`, `http_method`, `path`, `description`,
  `parameters`, `body`, `responses`, and rich `wire: OperationPlan`.
- Parameter: `source`, `native_name`, `native_type`, `required`, `schema`, and
  `wire: ParameterPlan` with original serialization/security/source metadata.
- Body: `source`, `native_name`, `native_type`, `required`, `choice_type`, `value`,
  `media`, and `wire: BodyPlan`.
- Response: `source`, `variant_name`, `error_variant_name`, `native_type`, `value`,
  `choice_type`, `media`, `headers`, and `wire: ResponsePlan`. `can_succeed()` and
  `can_fail()` use the matcher; actual status determines the emitted variant.

`java_sdk::protocol` retains `JavaValue`, `JavaMedia`, `JavaHeaders`, `JavaHeader`,
`JavaAggregate`, `JavaPart` and `AggregateRules`. Media and part values expose
their actual native representation and optional JSON schema identity. Their
descriptor pointers are private runtime addresses, never native API identities.

Inputs and status/result variants are nested under the configured client class.
Compose their full Java identity as `api_name + "." + allocated_name`.
Standalone model/literal/alias holders live in the configured package. Numeric
and JSON native types are nested under `JsonRuntime`. These descriptors are the
adapter interface; compatibility tooling does not need to parse Java source.

V3 capture uses the same explicit planner and effective generation options as the
canonical backend. Dynamic fields retain immutable JSON-value types and checked
codec obligations. Schema resource canonical/base/alias URIs and dynamic binding
metadata remain distinct from physical source locations; no logical URI is
substituted into a codec's `source()` identity.

### Resource/dynamic execution

The exact V3 pair is `suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic`. `ValidationResources.java` checks the
finite resource/node catalogue, URI aliases, physical containment, declarations,
bindings and initial-target consistency before execution. A nested entry activates
its indexed resource without evaluating the resource's root schema. Dynamic lookup
uses the outermost actually entered matching resource and never enters a fallback
before selection. Context is restored on every return/trial; cycles include node,
instance and exact interned ordered-resource identity. Lookup/entry visits share
the ordinary budget, and target annotation scopes are fresh. No runtime schema
resolution, acquisition or dynamic-annotation inference occurs.

`SchemaResources` and `DynamicSchemaReferences` are enabled in the verified
`capabilities_v3` path; the existing capability function remains V1/V2. The
separately verified `DocumentRelativeServers` path continues using physical
retrieval URLs rather than `$self`/`$id`.

## Verification commands and evidence

```sh
cargo test -p suspect-codegen --features java-sdk --test java_sdk

JAVA_HOME="$HOME/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS" \
SUSPECT_MAVEN_BIN="$HOME/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn" \
cargo test -p suspect-codegen --features java-sdk --test java_sdk \
  -- --ignored --nocapture --test-threads=1
```

The current-JDK run uses `temurin-25.0.4+101.0.LTS` with the same command. These
set `JAVA_HOME` per process; no global tool activation is required. Native tests
retain emitted packages, installed-jar consumers, all logs and failed attempts
under `target/sdk-java-native/`. Maven's local cache is isolated under
`target/sdk-java-maven-cache/java/repository`.

The accepted native base was verified during the shared-protocol migration using
the source-linked, target-only harness at `target/sdk-java-base-verification/java/`:
**7 planning tests and 4 native package gates on each JDK**, plus warnings-denied
Clippy in that closure. Its historical canonical build blockers remain recorded
in `target/sdk-java-verification/REPORT.md`.

The subsequent canonical registration phase passes **15 integration tests** with
the normal default-feature command:

```sh
cargo test -p suspect-codegen --features java-sdk --test java_integration
```

A separate new JDK 21 package/metadata gate builds and installs the canonical
`Client` package at independent Maven coordinates, then checks its compatibility
descriptors against JVM public members, generic types, constructors, literal
values, sealed memberships and source-codec bindings. It reuses the accepted
runtime matrix. Its evidence is in `target/sdk-java-integration/REPORT.md`.

The new protocol-only native gates are:

```sh
JAVA_HOME="$HOME/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS" \
cargo test -p suspect-codegen --no-default-features --features java-sdk \
  --test java_protocol -- --ignored --nocapture --test-threads=1
```

Use the JDK 25 home for the other protocol tier. `SUSPECT_JAVA_PROTOCOL_GATES`
can narrow ongoing verification to `examples,wire,controls,edges,multipart,lifetime,types,normative`.
Every native attempt is retained under `target/sdk-java-protocol/java/`. A
source-linked Java-only harness against Main's reviewed v1 schema snapshot is
available under `target/sdk-java-protocol/v1-isolation/` during shared schema
work; its records explicitly identify the isolated dependency. It is not evidence
of canonical v2/v3 schema admission.

## Boundaries and DX assessment

The protocol profile is deliberately bounded. CONNECT tunnel semantics, generic
typed Set-Cookie, ambiguous raw scalar extras, undeclared multipart encoding
conventions and unbounded positional multipart retain source-linked refusals.
Legacy OAS 3.1 binary markers require explicit `legacy-binary-string-v1`; OAS 3.0
native binary and ordinary OAS 3.1/3.2 byte declarations require no such profile.

The validator implements complete v1, scoped v2 and indexed-resource v3 instruction profiles.
`OwnedCompiler::compile_v2` preserves ordinary v1 program bytes; scoped examples
use the shared `plan_protocol_examples_v2` seam. Native intersections/tuples in
scoped closures use checked immutable JSON carriers. Resource/dynamic admission
is explicit through the new V3 APIs and canonical dispatch; existing direct
V1/V2 entrypoints preserve their refusals. Directional projections, legacy
recursive-reference keywords and custom dialect/vocabulary semantics remain
source-linked boundaries.
Safe static-refinement siblings and OAS 3.0 ref/nullable/exclusive-bound context
are retained. Format is annotation-only in the declared profile.

Compared with the Speakeasy assessment in `SDK-DX-ASSESSMENT.md`, this Java slice
improves inline names (`CreateKeysRequest`, `GetCreditsResponse`), constructs
examples through native builders, provides a no-argument credits call, and
returns a sole success directly. Remaining consumer friction includes flat
operation names, operation-input wrappers, a custom exact-number type, generic
scheme-name credential configuration, and source URI/pointer prose rather than
a polished hosted source-link experience. Automatic auth workflows, hosted
documentation and distribution maturity remain application/product work. Native
correctness evidence alone does not establish overall Speakeasy parity or full
M4/M5 release acceptance.
