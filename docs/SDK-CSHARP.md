# C# native SDK — shared HTTP protocol

The default `csharp-sdk` backend targets **.NET 8 / C# 12**, with .NET 8.0.424 and
10.0.400 native gates. Generation uses the canonical `http_protocol::ProtocolPlan`
directly. Only its actual JSON/text/item/header codec roots enter the existing
owned schema compiler; bytes and multipart aggregates are not replaced by JSON.

## Native calls

```csharp
using Example.Sdk;

using var client = new Client(
    new Credentials { ApiKey = token },
    new ClientOptions { Timeout = TimeSpan.FromSeconds(30) });

var credits = await client.GetCreditsAsync(cancellationToken);
Console.WriteLine(credits.Data.Data.TotalCredits.Token);

await client.CreateKeysAsync(
    new CreateKeysInput {
        Body = new CreateKeysRequest { Name = "My key" }
    },
    cancellationToken: cancellationToken);
```

Names above correspond to the tested public OpenRouter operations; namespace and
package identity are target configuration. No-input overloads omit empty input
objects. A single successful response has a concrete result with typed `Data`,
`Metadata` and actual `Status`. Source status/media alternatives use closed nested
classes. Public API exceptions expose the source-typed error body.

The generated README includes a complete source-bound first-request function with
native constructors, explicit credentials, cancellation, timeout and public typed
error handling. The identical `examples/Quickstart.cs` runs against an **installed
NuGet package**. Byte recipes are explicitly labeled native fixtures.

## Implemented protocol families

### Credentials

- Undeclared security, `security: []`, anonymous alternatives, source-ordered OR
  selection and AND requirements remain distinct.
- `RequestOptions.SecurityAlternative` explicitly selects an alternative. Otherwise
  the first alternative whose credentials are all supplied is used; an anonymous
  alternative is complete without credentials.
- Bearer, ASCII Basic, and header/query/cookie API keys have source-allocated native
  properties. The name `apiKey` does not imply an API-key header: an HTTP bearer
  declaration still sends `Authorization: Bearer ...`.
- OAuth/OIDC use an `AuthorizationProvider` delegate. It receives `CredentialContext`
  with operation/requirement/scheme identities, scopes versus roles, permission
  sources, flow URLs/scope descriptions and discovery metadata. It returns an
  explicit `AuthorizationValue(scheme, parameter)`.
- Without an `sdk_defaults` OAuth policy, no discovery, acquisition, refresh, flow
  selection, bearer inference or business permission check is performed; the opt-in
  token lifecycle is described in [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md).
  Conflicting attachments fail before sending.

### Servers, methods and parameters

- Effective server choices, relative URLs, source variable defaults/enums and explicit
  overrides are retained. `ServerIndex`, `ServerVariables`, `DocumentUrl` and
  `ServerUrl` are client/per-call options. A local source file is never a guessed
  network origin. `Client.Operations` exposes source server metadata.
- `DocumentRelativeServers` uses the shared `ServerPlan.document_base`: the
  effective physical retrieval document containing the Server Object. An implicit
  `/` belongs to the entry document; explicit empty server arrays belong to their
  overriding declaration. Redirect-request aliases and logical `$self`/`$id` names
  stay metadata. Local files require an explicit HTTP(S) document override.
- `ServerInfo.DocumentUrl`, `DocumentSource`, `Source`, `Resource`, `UrlBase` and
  `ResolveUrl` expose those distinctions. `OperationInfo` retains both physical
  source sites and aligned logical reference contexts. `ResourceInfo` reads the
  indexed canonical/base/scope addresses directly rather than constructing them.
- Relative resolution removes only literal RFC 3986 dot segments. Encoded dots,
  encoded slashes, path case and repeated internal slashes reach HttpClient intact.
  OAuth/OIDC hooks receive `CredentialContext.ServerUrl` plus `EffectiveServer`
  URL-base classification; endpoint strings remain raw metadata.
- Standard GET/PUT/POST/DELETE/OPTIONS/HEAD/PATCH/TRACE/QUERY and supported custom
  method tokens are sent through the native transport. Case-distinct custom spellings
  of known methods (for example `head` or `GeT`) are refused: HttpClient's known-method
  normalization does not provide their required case-sensitive semantics.
- Scalar, array and flat-object simple/label/matrix/form/spaceDelimited/pipeDelimited/
  deepObject styles, headers, cookies, content parameters and OAS 3.2 querystring
  JSON/text/form values have checked native serialization.
- UTF-8, decoded names, percent triples, `allowReserved` hazards, active delimiters,
  cookie escaping and undefined empty-composite expansion are checked. Object-key
  ordering follows Unicode scalar order. Numeric parameters preserve exact tokens.

### Responses, headers and Links

- Exact status outranks class range, which outranks default. A media mismatch cannot
  fall back to another status declaration. Success follows the **actual** 200–299
  status, including a default match.
- Concrete media outrank type wildcards then `*/*`; matching declared parameters
  break ties. Charset comparisons are case-insensitive; other parameter values are
  not globally lowercased. Actual content is never sniffed.
- JSON/`+json`, schema-free JSON, UTF-8 scalar text and bounded `byte[]` are distinct.
  Explicit request media choices cannot use a broad byte alternative to bypass a
  more specific schema.
- HEAD/1xx/204/304 produce `HttpNoContent`; JSON null is a separate value. Missing
  response content declarations mean bounded unspecified bytes. Missing response
  declarations leave actual statuses undeclared.
- Typed response headers enforce requiredness and run their bound codecs. Scalar,
  array and typed flat-object forms are supported. Repeated values remain separate;
  Set-Cookie array values are not comma-folded.
- `ResponseLinkInfo` retains target, literals/expressions, source and optional server
  metadata without evaluating expressions, selecting an operation or making a call.

### Forms and named multipart

- Native records represent form fields and named multipart inputs/outputs. JSON/text
  part codecs use their actual indexed roots; repeated arrays use per-item codecs.
- Required fields, declared extras and object/array cardinalities are checked
  structurally. File values are bounded in-memory `byte[]`, with explicit filename,
  concrete media and typed part-header properties.
- Source defaults and Encoding Object rules follow the enclosing media type's essence,
  including OAS 3.2 per-item encoding. RFC6570 fields are active for URL-encoded
  forms and `multipart/form-data`; an individual part's `contentType` does not
  determine applicability. Within form-data, `allowReserved` has no percent-encoding
  effect, but its explicit presence (including `false`) selects the active RFC6570
  strategy and its `contentType`-ignore rule. Binary parts retain their byte representation.
- Multipart parsing bounds headers and parts, checks disposition/media, preserves raw
  bytes and recognizes complete boundary delimiters. Text parts use the MIME default
  `text/plain` when appropriate; data is not inferred from filenames.

### Finite positional multipart

- OAS 3.2 `prefixEncoding` / `itemEncoding` uses an ordered native record with
  `Item1`, `Item2`, … and a typed `Items` list when the source admits remaining
  values. Each slot has its actual JSON/text codec or byte policy and typed headers.
- Optional prefix slots are `Optional<PartType>`; absent suffixes stay absent.
  A later slot or tail cannot skip an absent earlier position. A present zero-byte
  part, a zero-part MIME body, and an absent request body remain distinct.
- `minItems`, `maxItems`, closed tails and the shared compiler's false-prefix
  barrier are enforced before transport and during response decode. Unreachable
  slots after a false prefix are not generated. The native part-count ceiling is 1024.
- Each payload and declared header is validated with its source schema. Bytes never
  become JSON-null stand-ins. Positional parts are emitted in order without invented
  names. `multipart/form-data` requires the source-declared Content-Disposition header;
  other multipart types permit absent dispositions and headerless text parts.
- MIME parsing supports bounded preamble/epilogue and transport padding, folded
  headers, empty header blocks and binary data containing non-delimiter boundary
  prefixes. Source part-media/defaults and required headers remain enforced.
- Active positional RFC6570 style expansion in `multipart/form-data` has no source
  field name and is explicitly declined. For other multipart media, `style`,
  `explode` and `allowReserved` are ignored with source-located warnings; explicit
  or default JSON/text/byte content, whole-array parts and typed headers retain
  their native profile. Malformed metadata and incompatible actual content codecs
  still fail at their original source spans. Positional array-valued parts stay
  whole; named-array 3.1/3.2 item rules do not supply a positional grouping rule.
  Undefined response/group inverses retain their located refusals.

### SSE and JSON lines

- Standard OAS 3.2 `itemSchema` binds `HttpStream<T>`, implementing one-pass
  `IAsyncEnumerable<T>` and `IAsyncDisposable`.
- HTML event-stream framing handles BOM, CR/LF/CRLF, comments, multiline data,
  persistent event ids and integer retry metadata. SSE uses HTML's UTF-8 replacement
  decoding. The item codec sees an envelope: `data` remains a string.
- JSON lines use strict UTF-8 JSON and preserve numeric tokens, including an
  unterminated final JSON line. Blank lines are not invented JSON values.
- Neither JSON within SSE data nor `[DONE]`, reconnection, retry, pagination or
  workflow semantics are inferred. EOF does not dispatch an unterminated SSE event.
- Backpressure is iterator-driven. Total, item and buffer ceilings are finite.
  Cancellation and whole-call deadlines cover iteration; codec errors, early break,
  explicit disposal and unconsumed-response disposal release the transport.

## Values and failure boundaries

| Source domain | Native type |
| --- | --- |
| Required non-null / nullable | `required T` / `required T?` |
| Optional non-null / nullable | `Optional<T>` / `Optional<T?>` |
| Number / mathematical integer | `JsonNumber` / `JsonInteger` |
| Object / array | Mutable sealed record / `List<T>` |
| Literal set / union | Native literal enum / closed class with sealed typed arms |
| Unconstrained JSON | Checked `JsonElement` |
| Bytes / body forbidden by HTTP | `byte[]` / `HttpNoContent` |

Default `Optional<T>` is absent. `Optional<T?>.Present(null)` is explicit JSON null.
Source-required singleton tags initialize their sole native enum member; they are
not C# `required` properties. Other source defaults do not insert missing fields.

Codecs preserve exact number spellings, negative zero and leading-zero exponents,
using symbolic `BigInteger` math instead of `double`/`decimal`. Mathematical
integrality, bounds, divisibility and structural equality keep booleans separate
from numbers. `JsonInteger.ToBigInteger` expands only inside its digit ceiling.

Decode validates before conversion. Encode revalidates current mutable values,
selected union arms and parent constraints. Invalid UTF-8/Unicode, duplicate decoded
keys, undefined JSON elements, cycles and extra-key collisions have typed failures.

`SdkException.Kind` distinguishes authentication, request validation/representation,
transport, timeout, disposed resources, resource limits, response decoding,
unexpected responses and declared API errors. `CodecError` retains source/instance
pointers. Caller cancellation remains `OperationCanceledException`. Captures are
bounded copies excluded from default exception formatting. Cleanup preserves the
primary failure; late responses after cancellation are observed and closed.

The SDK-owned HttpClient disables redirects, cookies, proxy discovery, decompression
and response draining. Injected clients remain caller-owned and require empty
DefaultRequestHeaders; their handler policies remain explicitly application-owned.

Defaults: 8 MiB body/total stream, 1 MiB part/item, 64 KiB headers, 4096-byte capture,
30-second whole-call deadline, 128 JSON/conversion depth, 4096-byte numeric operands,
100,000 validation/equality visits and 1,000,000 conversion work/copy units.

### Scoped v2 schema validation

Ordinary static closures use `OwnedCompiler::compile_v2`. A closure requiring the new
applicators emits `suspect.validation.experimental.v2` with the exact
`oas31-jsonschema202012-static-applicators` profile. A base closure retains its v1
program and native runtime bytes.

The C# evaluator implements all nine instructions from
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md): conditional branches,
dependent required names and schemas, exact `contains` counts, property patterns,
pattern-aware additional properties, property names and both unevaluated rules.

- Each subschema starts with fresh evaluated-property/item sets. Only the specified
  successful contributions propagate; failed alternatives and unrelated scopes
  cannot leak annotations. Conditions and `contains` retain their separately
  specified local contributions even when a selected branch or count fails.
- Trials share depth, recursion identities and numeric/equality/work budgets.
  Evaluation failures cannot become validity through `not`, alternatives or a
  reporting cap. The public codec exposes the first mismatch while completing the
  required evaluation. Zero work/equality/depth limits permit no corresponding work.
- Object visits and annotation merges use decoded Unicode-scalar key order without
  normalization. Every merge candidate is charged, including duplicates; NFA
  transitions use the same evaluation budget. Key strings have independent instance
  identity from their associated values.
- Declared object fields keep their types, requiredness and omission/null states.
  Patterned extras use `Dictionary<string, JsonElement>` even with
  `additionalProperties: false`. Every matching pattern still applies, and
  unmatched values follow their actual additional/unevaluated rule.
- Within a v2 closure, conditional/intersection domains that lack a faithful static
  type use source-checked `JsonElement` codecs. Heterogeneous prefix arrays use
  `List<JsonElement>`. Constructors alone do not prove these constraints: both
  generated codecs and SDK calls validate current mutable fields, extras and lists.
- Scoped packages use `examples::plan_protocol_examples_v2`; C# additionally checks
  each accepted entry against the SDK compiler's actual limits before lowering
  native constructors. Declared origins, invalid findings and bounded synthesis
  remain shared with the canonical example planner.

The separate `ScopedValidationRuntime.cs` and `ValidationProgramGuard.cs` assets
are emitted only for v2. The native guard checks the entire finite descriptor,
source/target relationships, ordering, operands and NFA bounds before execution.
Unknown versions, profile mismatches and malformed instructions fail explicitly.

### Resource scopes and dynamic references (v3)

Resource-bearing codec closures explicitly use `OwnedCompiler::compile_v3` and
the exact pair `suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic`. `SchemaResources` and
`DynamicSchemaReferences` are enabled. The existing `plan_sdk`, `plan_sdk_with_options`, generated `Codecs`
and `Client` entrypoints select the checked profile; canonical dispatch needs no
new public entrypoint. Ordinary closures keep their established v1/v2 programs.

- Native validation enters each node's indexed resource, including a nested entry
  whose resource root is never evaluated. Outermost actually entered matching
  bindings win. The initial fallback resource is entered only after lookup;
  pointer, empty-fragment and ordinary-anchor references keep their static targets.
- Candidate schemas belong to the finite validation closure. They do not become
  HTTP body inputs or statically selected fallback types. Resource scopes restore
  on success, invalidity, failures and branch trials. Recursive cycle identity
  includes the exact ordered resource context, interned with equality-checked keys.
- Dynamic targets start fresh annotation scopes and propagate successful sets.
  V2 instruction/annotation costs remain, plus one visit per new distinct active
  resource and per scanned resource/binding. Work, numeric, depth and cycle failures
  remain noninvertible.
- Declared object fields, exact numbers and required/optional construction remain
  native. Dynamic-reference leaves and context-sensitive unions use `JsonElement`;
  standalone union trials would lose the enclosing resource scope. For those
  dependency cones, null-domain representations are conservative and complete-root
  codecs check actual membership. Optional JSON null stays distinct from absence.
- The immutable program and native reference docs retain physical resource and
  declaration sources, canonical/base/alias URIs and aligned node scopes. Logical
  identifiers never replace physical source IDs or authorize runtime acquisition.
  API server bases remain independently tied to their physical retrieval documents.
- `examples::plan_protocol_examples_v3` supplies bounded source discovery without
  following dynamic fallback annotations. Emitted values are additionally checked
  under the C# SDK's actual compiler limits before native constructor lowering.

The new `ResourceScope.cs` and `ResourceProgramGuard.cs` assets are composed with
the frozen scoped evaluator/guard by `csharp_sdk/resources.rs`. Each fixed template
extension requires exactly one matching seam. V3 packages emit
`ResourceValidationRuntime.g.cs` and `ResourceStaticGuard.g.cs` alongside those
extensions. V1/V2 continue emitting their original assets. V3 admission verifies
the exact envelope, physical containment, canonical/alias syntax and identity,
node-scope alignment, original declaration/anchor sources, finite targets and
initial-resource agreement before evaluation.

## OpenAPI 3.0

`csharp-http-protocol-v1` opts in to OpenAPI 3.0 documents. Per
[ADR 0016](adr/0016-closed-dialect-set.md), 3.0 Schema Objects are lowered onto
the same owned validation instructions as 3.1/3.2, and model planning reads
them through the shared `schema_view` dialect view, so native types and
validation agree:

- `nullable: true` adds null to the same-object `type` only. `type: string,
  nullable: true` becomes `string?`, a nullable object becomes `Model?`, and a
  `$ref` to a nullable schema is nullable through its target. An `enum` must
  still list `null`; `nullable` without a same-object `type` has no effect.
- Boolean `exclusiveMinimum`/`exclusiveMaximum` make the adjacent
  `minimum`/`maximum` exclusive (the finding keeps the bound's source).
- A Reference Object applies only its `$ref`: siblings such as `nullable`,
  `type` or `description` neither refuse nor change the alias.
- Direction-dependent `required` (a required `readOnly`/`writeOnly` member)
  stays a located refusal, like every active directional annotation here.

`tests/fixtures/oas30/petstore-nullable.openapi.yaml` is the plan/emit witness
and the installed .NET 8/10 consumer fixture.

## Explicit limits and compatibility profiles

Source-located refusals remain for streamed multipart, request
streams, nested multipart, undefined composite inverse response styles, untyped
extra header values, active directional projections, custom vocabularies/dialects
and legacy recursive-reference keywords. Portable pattern syntax remains the
bounded regular Unicode subset. Base v1-only closures retain their existing native
intersection/tuple/negation and mixed-compound-literal representation refusals;
the scoped v2 carriers above have explicit codec obligations. Basic credentials
and multipart filenames use documented ASCII profiles. Source-relative URLs
require a real HTTP(S) document base.

Legacy OAS 3.1 `type: string, format: binary` markers require explicit
`ProtocolOptions.compatibility_profiles = [LegacyBinaryStringV1]` through
`plan_sdk_with_options`. The profile applies only in byte contexts and is recorded
in the protocol plan. Default generation does not activate it.

## Public Rust planning API

### Explicit runtime environment credentials

`csharp_sdk::protocol::ProtocolOptions.credential_env` optionally accepts the shared
`credential_env::CredentialEnv` v1 policy. The adapter binds it after protocol
admission, retains `SdkPlan::credential_env()`, and captures only its typed
`semantic_descriptor()` in `NativeSnapshot.credential_env`. Generated metadata
contains variable names and source bindings, never generator-process values.

For an OpenRouter policy mapping `apiKey` to `OPENROUTER_API_KEY`, configure package
`OpenRouter.SDK` and namespace `OpenRouter`. Configured packages expose the compiled
factory:

```csharp
using OpenRouter;

using var client = Client.FromEnvironment();
await using var response = await client.GetCurrentKeyAsync(cancellationToken);
```

The exact factory signature is
`Client.FromEnvironment(ClientOptions? options = null, HttpClient? httpClient = null)`.
It reads mapped variables at creation, filters them through the existing declared
attachment rules, and preserves the source-default server. Environment changes
affect later clients. Missing/empty/unavailable values remain missing: anonymous
operations remain usable, and a protected call fails before HTTP with a bounded,
secret-free `SdkException` of kind `Authentication`.

The explicit constructor `new Client(Credentials, ClientOptions?, HttpClient?)`
uses the entire explicit argument; it never fills empty/null values or missing
members from the environment. An explicit null Credentials argument retains its
existing `RequestRepresentation` constructor failure. `new Client()` remains the
anonymous-client form. Injected HttpClient instances remain caller-owned, and
Task/cancellation/disposal semantics retain their normal client behavior.

V1 permits bearer and header/query/cookie API-key strings. Mappings to Basic,
OAuth/OIDC or unknown/ambiguous declarations fail through the shared binder.
Configured packages use use-site credential identity so aliases to the same
terminal declaration do not leak environment defaults into one another. Native
environment access that is security- or platform-unavailable becomes a missing
value without exposing exception text. No .env loading or runtime acquisition is
performed.

The factory and `CredentialEnvironment.cs` helper are emitted only with a bound
policy. Unconfigured packages retain their existing explicit construction and
output bytes. The actual-source native test selects only `getCurrentKey` and
`getCredits`, loads their original schemas/examples, and injects a controlled
HttpClient handler at the source HTTPS URL; it makes no live account calls.

```text
SdkConfig { name: String, version: String, namespace: String }
plan_sdk(Arc<Contract>, &[SourceId], SdkConfig)
  -> Result<SdkPlan, Vec<HttpDiagnostic>>
plan_sdk_with_options(Arc<Contract>, &[SourceId], SdkConfig, ProtocolOptions)
  -> Result<SdkPlan, Vec<HttpDiagnostic>>

SdkPlan::render()               -> Result<Vec<OutFile>, Vec<HttpDiagnostic>>
SdkPlan::package()/config()     -> &SdkConfig
SdkPlan::contract()             -> &Arc<Contract>
SdkPlan::protocol()             -> &http_protocol::ProtocolPlan
SdkPlan::operations()           -> &[PlannedOperation]
SdkPlan::models()               -> &models::ModelPlan
SdkPlan::credential_bindings()  -> &[protocol::PlannedCredential]
SdkPlan::credentials()          -> &BTreeMap<String, String>
SdkPlan::program()              -> &OwnedProgram
SdkPlan::examples()             -> &ExamplePlan
```

Credential map keys are terminal scheme source identities. `PlannedOperation.wire`,
`PlannedParameter.wire`, `PlannedBody.wire`, `PlannedResponse.wire` retain immutable
shared protocol descriptors. Native media, parts, headers, result/error names,
no-content alternatives and single-success choices are retained alongside them.
`PlannedMedia::schema()` is optional: bytes are not codec roots.

`ModelPlan::field_initialization`, `is_deprecated`, `native_type`, `codec_name`,
`names`, `declarations`, `is_nullable` and `is_json_carrier` are shared emitter/compatibility
seams. Compatibility captures actual types, erased aliases, codecs, constructors,
get/set/init access, credential allocations, part/header records and media/stream
alternatives. There is no parsing of emitted C# to rediscover the API.

Canonical `backend::generate_with_options` and `compatibility::snapshot_with_options`
use the same `GenerationOptions.compatibility_profiles` through
`backend::csharp_options`. Snapshot capture preserves `snapshot.generation` and
passes it to `plan_sdk_with_options`; TargetConfig's package fields are unchanged.
`PlannedMedia.positional` retains `PlannedPositionalParts` (aggregate source,
ordered prefix, optional typed items, min/max counts). Compatibility records its
ordered fields, closed tail, native construction/access and part/header symbols.
V2/V3 native descriptors additionally record `sourceValidation` (version/profile,
complete decode and current-value encode obligations, scoped evaluation and JSON
carrier identity) and patterned-extra validation policy. The capture uses the same
public planner and canonical generation options as SDK emission.
V3 also records outermost-entered binding, context-sensitive carrier/null-domain
obligations, ordered-context cycle identity and validation-only candidate policy.

## Package and verification

Every artifact path begins with `csharp/`. The NuGet source project targets net8.0,
enables nullable analysis/warnings-as-errors/XML docs, and has no third-party runtime
dependencies. `protocol-plan.json` and `validation-program.json` are embedded.
`http-manifest.json` and `docs/reference.json` bind the native interface to sources.
Version-2 examples preserve response-pattern/header/part/item roles and findings.

```sh
CARGO_TARGET_DIR=target/sdk-csharp-cargo cargo test -p suspect-codegen \
  --test csharp csharp_protocol:: -- --include-ignored --nocapture --test-threads=1
CARGO_TARGET_DIR=target/sdk-csharp-cargo cargo test -p suspect-codegen \
  --test csharp -- --include-ignored --nocapture --test-threads=1 \
  csharp_sdk:: csharp_integration::
```

The expanded native matrix uses the literal shared protocol vectors and an
independent loopback server, with .NET 8/10 package installation, negative types,
native XML/reference binding checks, byte fixtures and stream lifetime controls.
The base fixture, five actual OpenRouter operations, shared 17 validator
cases and CLR descriptor probes remain regression gates. Real operation
tests also cover `createCoinbaseCharge`, `downloadContainerFileContent` and
`downloadFileContent` (downloads use the explicit legacy binary profile).

Protocol capability support is distinct from whole-API or release-readiness claims.

### Verification

Native tests are `#[ignore]`d and need the .NET SDKs. Every native command honors
`SUSPECT_DOTNET_BIN` (falling back to the mise-installed `dotnet`), and each tier's
`global.json` pins the exact SDK (8.0.424 or 10.0.400) with `rollForward: disable`.
`csharp_positional` covers finite positional multipart, including the case-distinct
known-method refusal. Scoped v2 validation, physical document servers and v3
resources have library-native selectors:

```text
csharp_sdk::validation_tests::native_scoped_source_vectors
csharp_sdk::validation_tests::native_scoped_limits_and_admission
csharp_sdk::validation_tests::native_scoped_sdk_packages
```

`csharp_sdk::server_tests::native_document_relative_servers` covers
`DocumentRelativeServers`.

```text
csharp_sdk::resources_tests::native_resource_source_vectors
csharp_sdk::resources_tests::native_resource_scope_and_admission
csharp_sdk::resources_tests::native_resource_sdk_packages
```

These compile their source fixtures through the real `compile_v2`/`compile_v3`
and run against both SDK tiers with installed NuGet consumers and executable
examples. For a base closure, the v1 program and generated package files are
byte-identical under either compiler.
