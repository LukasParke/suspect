# OpenAPI requirements for faithful SDK generation

Research date: 2026-09-08. This note distinguishes specification facts from proposed product policy. Sources are the OpenAPI Initiative's specifications, the JSON Schema project's specifications and test suite, and documentation-tool owners. Official pages were fetched successfully; initial sandbox DNS restrictions were resolved through approved read-only network access. Swift's documentation page required JavaScript, so its official source repository README was used instead.

## Existing schema engine: SDK integration assessment

Initial read-only evaluation, 2026-09-08. It recommended an owned validation
program compiled from the resolved contract registry. That bounded
[owned validation boundary](SDK-OWNED-VALIDATION.md) now supplies the SDK codecs;
the source-bound compiler remains a separate API. The assessment below records
the pre-repair implementation.

**Implementation update:** the subsequent bounded
[exact numbers and equality repair](SDK-SCHEMA-NUMERICS.md) resolves the numeric
failures listed below through the public API, with explicit evaluation limits,
official conformance cases, actual OpenRouter acceptance and local benchmarks.
The examples in this assessment remain a record of the pre-repair baseline;
they do not describe the current numeric implementation.

The evaluation exercised the real public `Compiler::compile` and
`Schema::validate` interfaces, using the pinned tracked OpenRouter input and
independent normative vectors. No `suspect-schema` code was changed during that
initial evaluation. Current behavior is verified by the maintained schema,
owned-program and native codec suites.

Three blocker families were reproduced:

1. **Constraints and exact values.** The actual OpenRouter
   `ChatRequest.properties.messages` accepts `[]` despite `minItems: 1`;
   `ChatChoice.properties.index` accepts `1e-400` despite `type: integer`.
   Independent cases also accept `9223372036854775808` under `maximum: 0`,
   accept `9007199254740993` under `maximum: 9007199254740992.0`, accept
   `0.070000000001` under `multipleOf: 0.01`, and merge enum values
   `9007199254740992` / `9007199254740993`. A valid schema bound
   `maximum: 18446744073709551616` fails compilation as “must be a number.”
   The compiler's keyword dispatch omits `minItems` and other cardinality
   checks; numbers materialize as `i64`/`f64`, wide integer instances can skip
   numeric checks, enum equality uses `f64`, and `multipleOf` uses an epsilon.
   See [compilation](../crates/suspect-schema/src/compile.rs),
   [execution](../crates/suspect-schema/src/exec.rs),
   [numeric checks](../crates/suspect-schema/src/keywords/numeric.rs), and
   [equality](../crates/suspect-schema/src/keywords/types.rs).
2. **Annotation scope.** For `{"a":1}`, the schema
   `{"anyOf":[{"properties":{"a":true},"required":["missing"]},true],"unevaluatedProperties":false}`
   incorrectly succeeds: a failed branch leaves annotations visible.
   `{"allOf":[{"properties":{"a":true}},{"unevaluatedProperties":false}]}`
   also incorrectly succeeds because a sibling branch sees its cousin's
   annotations. The valid outer-`unevaluatedProperties` control and a simple
   overlapping `oneOf` rejection behave correctly. The shared instance masks
   and error-only branch diversion in [execution](../crates/suspect-schema/src/exec.rs)
   need evaluation-scope isolation, not just additional union type syntax.
3. **Resolution and dialect context.** Compiling the tracked
   `AnthropicImageBlockParam` subtree rejects its valid URL-image input because
   `#/components/schemas/AnthropicUrlImageSource` is resolved from the component
   subtree. A copied document-root closure plus a root `$ref` succeeds.
   External references report “external schema resolution not configured,”
   and the public configuration has no resolver or dialect selector.
   `$schema` is explicitly treated as an annotation; 3.0 nullable behavior is
   not selectable. See [compiler entrypoint](../crates/suspect-schema/src/compile.rs),
   [references](../crates/suspect-schema/src/keywords/refs.rs), and
   [configuration](../crates/suspect-schema/src/config.rs).

The proposed seam accepts a resolved schema registry, root schema identities,
and explicit dialect/validation policy, returning an immutable, owned program
plus source-linked diagnostics. Store exact numeric values, stable reference
edges and keyword provenance; reject unsupported required capabilities before
release. Execution takes an instance view and keeps annotation masks, error
limits and dynamic scope local to each evaluation/branch. An owned program can
be shared across threads, while each validation owns its mutable state.

The current program stores borrowed `NodeRef`/string values, `Rc` programs and
a lazy `RefCell` cache, so it is neither `Send` nor `Sync`. The public API does
accept schema and instance values from separate live `LowDoc`s, as the probe
demonstrated; a shared lifetime is not a same-document identity requirement.
The integration problem is preserving the schema's full resolved context and
exact semantics without reparsing/recompiling it per target or worker.
Reuse useful keyword implementations and tests behind the new seam, repair the
reproduced behavior, and let language codecs/docs consume the same semantic
decisions. Do not serialize the owned contract back into synthetic source text
as the long-term SDK runtime interface.

## What “fully sourced from OpenAPI” can mean

**Recommendation:** API-specific facts must originate in the OpenAPI description and its referenced resources: operations, wire names, schemas, media types, authentication requirements, errors, examples, and prose. Package names, target language/toolchain versions, release versions, and formatting are generation settings. They must not become a second source of API semantics.

The standard describes API contracts, but it does not prescribe one idiomatic SDK surface or a universal pagination iterator. OpenAPI 3.2 can model pagination linksets in bodies and HTTP `Link` headers, while Link Objects connect operations using runtime expressions. Neither is a general declaration of cursor selection, page accumulation, stopping conditions, and retry policy for arbitrary APIs. [OAS 3.2 Link Object](https://spec.openapis.org/oas/v3.2.0.html#link-object), [Modeling Link Headers](https://spec.openapis.org/oas/v3.2.0.html#modeling-link-headers).

**Recommendation:** support every fully described low-level operation without metadata guesses. Add convenience methods only when the standard supplies enough information or a documented, schema-validated extension in the OpenAPI source declares the missing behavior. Do not infer pagination, idempotency, retry safety, or business behavior from names such as `list`, `cursor`, or `next`. The `x-` extension mechanism is standard, but each extension's meaning is application-specific. [OAS Specification Extensions](https://spec.openapis.org/oas/v3.2.0.html#specification-extensions).

## Version-aware semantics

| Specification | Verified facts | Consequence for the generator |
|---|---|---|
| [Swagger/OpenAPI 2.0](https://spec.openapis.org/oas/v2.0.html) | A separate published input format exists, with its own operation/parameter/body definitions. | Treat support as a separate frontend and conversion profile, with diagnostics for information that cannot be represented in the current model. Do not silently call every 2.0 conversion lossless. |
| [OpenAPI 3.0.4](https://spec.openapis.org/oas/v3.0.4.html#schema-object) | Schema Objects use a restricted JSON Schema subset. `type` is one string; unsupported JSON Schema keywords are explicitly unsupported. `nullable: true` only takes effect when `type` appears in the same Schema Object, and other constraints can still exclude null. `readOnly`/`writeOnly` alter the direction in which `required` applies. | Normalize through a 3.0 frontend, retaining original version/provenance. A blanket replacement of every `nullable` with an unconstrained null union is incorrect. |
| [OpenAPI 3.1.2](https://spec.openapis.org/oas/v3.1.2.html#schema-object) | Schema Objects support JSON Schema Draft 2020-12 and dialect selection. Schema `$ref` follows JSON Schema semantics. | Preserve explicit null types, boolean schemas, constraints, schema-resource identity, and dialect. Do not apply 3.0 Reference Object or nullable rules to schema `$ref`. |
| [OpenAPI 3.2.0](https://spec.openapis.org/oas/v3.2.0.html) | Includes `querystring` parameters, the `query` operation and `additionalOperations`, streamed-item schemas, and discriminator `defaultMapping`. Schema Objects continue to use the OAS dialect based on JSON Schema 2020-12. | Make these explicit capabilities in the parser, semantic model, wire layer, target backends, and documentation. Parsing the version string alone is not support. |

The 3.2 OAS dialect identifier remains `https://spec.openapis.org/oas/3.1/dialect/base`; constructing a new dialect URI from the OpenAPI version would be wrong. A schema resource's `$schema` overrides the document's `jsonSchemaDialect`; tooling must support the OAS dialect and may support additional dialects. [OAS 3.2 JSON Schema Keywords](https://spec.openapis.org/oas/v3.2.0.html#json-schema-keywords), [Specifying Schema Dialects](https://spec.openapis.org/oas/v3.2.0.html#specifying-schema-dialects).

**Recommendation:** publish an input-version × feature × target capability matrix. Stable support means checked semantics, codecs, generated package, and docs; accepted syntax is a separate measure. Unsupported dialects or wire behavior need actionable diagnostics rather than silent widening or dropped operations.

## Type fidelity: preserve information before choosing syntax

### Presence and null are independent

JSON Schema `required` checks whether a property name exists. Omitting `required` behaves as an empty list. Null is a separate instance type; requiring a property does not prohibit null, and permitting null does not make a property optional. [JSON Schema Validation §6.1.1 and §6.5.3](https://json-schema.org/draft/2020-12/json-schema-validation).

| Required property? | Null allowed? | States to preserve |
|---|---|---|
| Yes | No | Value |
| Yes | Yes | Value, explicit null |
| No | No | Absent, value |
| No | Yes | Absent, explicit null, value |

**Recommendation:** make presence and nullability separate semantic fields and separate codec concerns. Use a small idiomatic presence abstraction where the target's ordinary optional type cannot preserve all states through serialization. Test both encoding and decoding for every row, including nested objects, arrays, and maps. Do not let a target serializer's default omission policy erase explicit null.

In JSON Schema 2020-12, `default` is an annotation, not an instruction to mutate an instance or make a required property optional. OpenAPI 3.0 describes default differently, as what an input consumer assumes if a value is not provided, and additionally requires type conformity. [JSON Schema Validation §9.2](https://json-schema.org/draft/2020-12/json-schema-validation#section-9.2), [OAS 3.0 Schema Object](https://spec.openapis.org/oas/v3.0.4.html#schema-object).

**Recommendation:** document defaults without automatically inserting them into requests. Any opt-in default application must have explicit, version-aware rules and tests.

### Request/response views need version-aware policy

OpenAPI 3.0 says a `readOnly` property should not be sent in requests and that its `required` constraint applies only to responses; `writeOnly` reverses that direction. A property cannot be both. [OAS 3.0 Schema Object](https://spec.openapis.org/oas/v3.0.4.html#schema-object).

OpenAPI 3.1/3.2 use JSON Schema annotation semantics. The owning authority may ignore or reject modifications to read-only values. OpenAPI 3.2 explicitly discusses permitting an unchanged read-only field in a PUT representation and warns that its behavior differs from 3.0. [OAS 3.2 Validating readOnly and writeOnly](https://spec.openapis.org/oas/v3.2.0.html#validating-readonly-and-writeonly), [JSON Schema Validation §9.4](https://json-schema.org/draft/2020-12/json-schema-validation#section-9.4).

**Recommendation:** retain the canonical schema plus direction annotations; derive ergonomic request/response views with a documented policy for each dialect. Avoid treating a convenient request-view projection as if it were the unmodified normative schema. A universal “remove every readOnly field and its required entry” transformation is not faithful across these versions.

### Composition is validation logic

`allOf` means every subschema validates; `anyOf` means at least one; `oneOf` means exactly one. Subschemas validate independently. Composition does not imply class inheritance. A discriminator is a dispatch hint and must not change the validation result. An `allOf` discriminator on a parent does not cause validation to search for or validate child schemas. OpenAPI 3.2 requires `defaultMapping` when the discriminating property is optional. [JSON Schema Core §10.2.1](https://json-schema.org/draft/2020-12/json-schema-core#section-10.2.1), [OAS 3.2 Composition](https://spec.openapis.org/oas/v3.2.0.html#composition-and-inheritance-polymorphism), [Discriminator Object](https://spec.openapis.org/oas/v3.2.0.html#discriminator-object).

**Recommendation:** distinguish semantic intersections, exclusive alternatives, and inclusive alternatives in the intermediate representation. Tagged unions are excellent when supported by the source schema, but `anyOf` cannot always become one tagged alternative, and flattening `allOf` objects requires a semantics-preserving proof. A discriminator can accelerate selection without skipping constraints that establish correctness.

`additionalProperties` applies relative to `properties` and `patternProperties` in the same schema object. Omitting it permits additional values. `unevaluatedProperties` instead depends on successful evaluations across applicable schemas and references. Therefore merging the property lists of closed `allOf` members can change what the original schema accepts. [JSON Schema Core §10.3.2 and §11](https://json-schema.org/draft/2020-12/json-schema-core), [OAS 3.0 additionalProperties default](https://spec.openapis.org/oas/v3.0.4.html#schema-object).

**Recommendation:** explicitly model open objects, typed additional properties, closed objects, pattern-based properties, tuples, conditional constraints, and unevaluated constraints. Retain valid unknown fields when an open object permits them. Decide separately whether an invalid response should be rejected or made available through a documented raw-response path; silently discarding information is not fidelity.

### Numeric and format mappings require care

JSON Schema imposes no precision or size bound on JSON numbers. A schema `integer` is mathematically integral: JSON representations `1` and `1.0` can both be integers. `format` is non-validating by default; its applicability does not implicitly constrain the instance's type. [JSON Schema Validation §4.2](https://json-schema.org/draft/2020-12/json-schema-validation#section-4.2), [OAS 3.2 Data Types](https://spec.openapis.org/oas/v3.2.0.html#data-types), [Data Type Format](https://spec.openapis.org/oas/v3.2.0.html#data-type-format).

**Recommendation:** do not map every integer to a fixed-width integer or every number to a binary float without a precision policy. Use constraints to select safe native types; retain lossless numeric representations where necessary. Preserve wire values for timestamps, decimals, unknown format strings, and encoded binary unless a documented conversion round-trips the source contract. Strict enum checking and forward-compatible unknown-enum handling are distinct policies; an unknown case is a robustness feature, not evidence that the declared enum permits that value.

### References are a graph, not textual substitution

JSON Schema `$id` establishes resource identity and base URI; `$anchor` creates named fragments; `$dynamicRef` uses dynamic scope. Schema `$ref` permits adjacent schema keywords. A resolved schema URI is an identifier and need not be a downloadable URL. OpenAPI Reference Objects have different sibling rules from Schema Objects containing `$ref`. [JSON Schema Core §8.2–8.3](https://json-schema.org/draft/2020-12/json-schema-core), [OAS 3.2 Reference Object](https://spec.openapis.org/oas/v3.2.0.html#reference-object).

**Recommendation:** use a canonical schema graph with resource URI, pointer/anchor, dialect, original span, and stable identity. Resolve recursive edges without infinitely expanding them. Preserve provenance through normalization and language lowering so every diagnostic and generated API symbol can link to its source. Cache resolved resources and semantic nodes by content and configuration; do not refetch every URI or duplicate complete schemas per operation.

## HTTP behavior is part of the type contract

Parameter serialization depends on location, style, explode, reserved-character handling, schema/content, and media type. `content` parameters permit one media-type entry. `deepObject` only defines objects with scalar properties; nested arrays and objects are not defined by that style. OpenAPI 3.2's `querystring` parameter describes the whole query string and cannot coexist with individual query parameters for the same operation. Its `cookie` style has cookie-specific separators and no automatic percent encoding. [OAS 3.2 Parameter Object](https://spec.openapis.org/oas/v3.2.0.html#parameter-object).

Request and response media-type maps use their most specific applicable entry. Form and multipart encodings carry behavior that JSON serializers alone cannot implement. OpenAPI 3.2 also distinguishes complete content schemas from item-by-item streaming and positional multipart encodings. [Request Body Object](https://spec.openapis.org/oas/v3.2.0.html#request-body-object), [Media Type Object](https://spec.openapis.org/oas/v3.2.0.html#media-type-object), [Encoding Object](https://spec.openapis.org/oas/v3.2.0.html#encoding-object).

Responses can be specified by exact status, status class, and default; an exact code wins over its class. Descriptions need not enumerate every possible status. A response also has headers, media types, and links. [Responses Object](https://spec.openapis.org/oas/v3.2.0.html#responses-object), [Response Object](https://spec.openapis.org/oas/v3.2.0.html#response-object).

Security requirements are alternatives across array entries (OR), with every scheme inside an entry required together (AND). An operation can override top-level security, an empty requirement permits anonymous access, and an empty operation security array removes the top-level requirement. Security schemes describe HTTP, API key, OAuth2, OpenID Connect, and mutual TLS mechanisms; 3.2 includes device authorization flow. [Security Requirement Object](https://spec.openapis.org/oas/v3.2.0.html#security-requirement-object), [Security Scheme Object](https://spec.openapis.org/oas/v3.2.0.html#security-scheme-object).

**Recommendations:**

- Create a language-neutral wire plan for every operation, with target runtimes implementing the same reviewed rules. A correct generated method signature is insufficient if a list, cookie, multipart field, status, or security combination travels incorrectly.
- Expose typed documented errors together with status, headers, and access to the raw body; retain an explicit unexpected-response error. Preserve all declared successful responses, including no-content responses. Keep transport failures separate from API error payloads.
- Build credential-provider and custom-transport hooks around declared security schemes. Do not invent credentials, auto-select an incompatible alternative, or equate documenting an OAuth flow with implementing an interactive authorization application.
- Run cross-language wire fixtures against a recording server: exact method/path/query/headers/body; auth alternatives/conjunctions; response status/media selection; binary and multipart; cancellation; malformed data; and streaming frames split at arbitrary byte boundaries.

## Streaming and higher-level scope

OpenAPI 3.2 defines sequential media types including JSONL, NDJSON, JSON text sequences, SSE, and multipart/mixed. `schema` applies to complete content; `itemSchema` applies independently to each stream item. SSE must be parsed according to the event-stream specification before schema application, including multiline data and ignored fields/comments. [Complete vs Streaming Content](https://spec.openapis.org/oas/v3.2.0.html#complete-vs-streaming-content), [Special Considerations for Server-Sent Events](https://spec.openapis.org/oas/v3.2.0.html#special-considerations-for-server-sent-events).

**Recommendation:** treat streaming as typed incremental codecs and native iteration with cancellation and bounded buffering. Do not buffer an unbounded stream to make a JSON-array API work. Older OpenAPI input may describe a streaming media type without independently typed items; require sufficient schema metadata or a documented in-spec extension for a stronger generated item type.

OpenAPI has callback definitions tied to an operation and independent incoming webhook definitions. These describe requests the API provider can initiate, which has a different direction from ordinary SDK calls. [Callback Object](https://spec.openapis.org/oas/v3.2.0.html#callback-object), [OpenAPI Object webhooks](https://spec.openapis.org/oas/v3.2.0.html#openapi-object).

**Recommendation:** scope webhook payload types, parsing/validation helpers, and docs as additional outputs. Signature verification needs an explicitly described algorithm and inputs; the presence of a webhook schema alone does not supply those facts. Polling, pagination, retries, resumability, and webhook verification each need their own declared contract rather than one generic “advanced SDK” switch.

## Language-native documentation and release acceptance

The following table proposes target-specific release gates. The linked sources establish the documentation mechanisms; the package, typing, and test gates are recommendations, not claims that a language standard mandates them. No language-adoption ranking was researched.

| Target | Proposed package and API acceptance | Native documentation output |
|---|---|---|
| TypeScript / JavaScript | Installable package; strict TypeScript consumer fixtures; runtime tests in each claimed JS environment; declarations shipped and checked; cancellation and streaming API reviewed. | TSDoc-style source comments and [TypeDoc](https://typedoc.org/) HTML/JSON from real exports, plus runnable TS and JS quickstarts. TypeDoc documents exported symbols. |
| Python | Installable wheel/source distribution; import smoke test; typed consumer fixtures; sync/async policy explicit; presence sentinels survive model serialization. | Docstrings plus [Sphinx autodoc](https://www.sphinx-doc.org/en/master/usage/extensions/autodoc.html) reference and executable examples. Autodoc extracts documentation from docstrings. |
| Go | Module consumption; formatting, build, tests, vet; context cancellation; explicit errors; safe zero/presence/null behavior. | Package/symbol comments following [Go doc comments](https://go.dev/doc/comment), package examples and browsable API reference. Go says every exported name should have a doc comment. |
| Rust | Crate consumption; build/test/lint; typed errors; ownership and async policy reviewed; explicit presence handling where `Option` alone loses wire states. | Rustdoc comments, [`cargo doc`](https://doc.rust-lang.org/cargo/commands/cargo-doc.html), and checked documentation examples. |
| Java | Consumable Maven artifact; selected JDK matrix; null/presence and async interfaces reviewed; compile and consumer tests. | [Javadoc](https://docs.oracle.com/en/java/javase/25/docs/specs/man/javadoc.html) reference, package overview, and compiled examples. Javadoc generates API HTML from Java source and documentation comments. |
| C# | Consumable NuGet package; nullable-reference checking; cancellation; typed API/transport errors; target framework matrix. | Compiler-generated [XML documentation comments](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/xmldoc/) shipped with the library, plus a rendered reference. The compiler can report undocumented public members when documentation generation is enabled. |
| Kotlin | Consumable Gradle/Maven artifact; Kotlin consumer tests; deliberate coroutine and Java interoperability policy. | KDoc with [Dokka](https://kotlinlang.org/docs/dokka-introduction.html); Dokka understands KDoc and Javadoc and emits reference documentation. |
| Swift | Consumable Swift package; platform/toolchain matrix; async/throws and cancellation; explicit absent/null/value serialization. | Source documentation and [Swift-DocC](https://github.com/swiftlang/swift-docc) reference/tutorials through the SwiftPM documentation plugin. |
| Ruby | Consumable gem; runtime tests; declared type-checking approach and limitations; keyword and absence semantics reviewed. | [YARD](https://yardoc.org/) source documentation and rendered reference, plus executable examples. |
| PHP | Consumable Composer package; native types plus documented analyzer profile; runtime and consumer tests; explicit object/map and null handling. | Docblocks and [phpDocumentor](https://docs.phpdoc.org/) reference, plus runnable examples. |
| C++ | Consumable CMake package; compiler/platform matrix; ownership, transport, and error policy; ABI/dependency commitments explicit. | Structured comments and [Doxygen](https://www.doxygen.nl/manual/docblocks.html) reference, plus compiled examples. |

**Current delivery decision (2026-09-09):** establish native parity across Python,
Go, Swift, Rust and TypeScript/JavaScript. Other ecosystem drafts are paused.
This is an engineering sequence, not a researched popularity ranking. Each
profile must pass native package, type, codec, docs and consumer gates.
JavaScript shares the TypeScript implementation but has explicit runtime and
documentation requirements. See [SDK-GENERATION-PLAN.md](SDK-GENERATION-PLAN.md).

Every language's docs should include installation, configuration/authentication, first successful call, operations, request/response models, field presence/null rules, errors, uploads/downloads, and any generated streaming/pagination features. Render preserved source descriptions and examples against the language's final symbol table so renamed parameters, escaped keywords, and model splits match the actual API. Include standard metadata such as deprecations and external documentation links. Examples should be validated against the source schema, compiled or type-checked against the generated package, and run with a controlled fake transport where useful. When the OpenAPI input lacks business prose or a valid usable example, report that gap; generated boilerplate is not evidence that the missing information exists.

## Validation and performance implications

The JSON Schema project's [official test suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite) is language-agnostic and covers specified validator behavior, including Draft 2020-12. Its maintainers explicitly distinguish it from a schema-authoring style guide. It does not by itself test OpenAPI HTTP serialization or generated SDK ergonomics.

**Recommendations:**

1. Reuse applicable official schema conformance cases and add an SDK-specific corpus for presence, unions/intersections, schema graphs/dialects, numbers, names, direction, encoding, errors, and docs. Pin the test-suite revision used by releases.
2. Check both accepted and rejected consumer programs where the language has static checking. Pair those with codec round trips and reference schema validation; compile success cannot prove runtime contract fidelity.
3. Define fidelity outcomes as “native static representation,” “native type plus codec/validation,” or “unsupported with diagnostic.” Do not describe a lossy `any`, generic dictionary, or unchecked fallback as complete type support. JSON Schema constraints such as arbitrary regexes and conditional requirements need runtime validation or a clearly reported limitation when they cannot be expressed statically.
4. Benchmark correctness-preserving stages separately: parse/resolve, semantic normalization, target lowering, emission, formatting, documentation build, and package verification. Record cold and warm runs, per-language and many-language runs, large schemas, recursive graphs, wall time, peak memory, and generated size. Avoid performance promises until these measurements exist.
5. Resolve and normalize once, share immutable semantic data, cache dependency-aware stages, lower independent targets concurrently, and write only changed artifacts. Key caches by source/resource content, dialect, compiler/backend versions, and generation policy. These are architecture recommendations; speedups require measurement and invalidation tests.
