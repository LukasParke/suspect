# PHP SDK: exact native models and bounded HTTP protocols

The PHP backend emits a complete Composer package under **`php/`**. It targets
PHP 8.3+ with typed mutable models, string enums, explicit omission, exact JSON
numbers, checked codecs, typed media/part/header carriers and injectable buffered
and pull-streaming HTTP adapters. Native tests
exercise PHP **8.3.32 / 8.5.8**, Composer **2.10.3** and PHPStan **2.2.13** on
macOS arm64. Tool origins and hashes are recorded in
`target/sdk-php-tools/provenance.json`.

## Generate and integrate

The protocol backend uses the `php-sdk` and `http-protocol` features:

```rust
use suspect_codegen::php_sdk::{PhpConfig, plan_sdk};

let plan = plan_sdk(contract, &selected_operations, PhpConfig {
    package_name: "example/openrouter".into(),
    package_version: "0.1.0".into(),
    namespace: "Example\\OpenRouter".into(),
    ..Default::default()
})?;
let files = plan.render(); // deterministic OutFile set, rooted at php/
```

Planning consumes the canonical owned `Contract`, `http_protocol` admission and a
checked `OwnedProgram`. It rejects unsupported declarations before returning
artifacts. Package identity and finite resource policy are target configuration;
wire behavior comes from the selected OpenAPI declarations. `Backend::PhpHttp`,
`backend::php_config`, sessions and compatibility capture use the same planner,
package policy and explicit `GenerationOptions`. `TargetConfig` retains its four
fields: backend, package name, package version and optional native import name.

Standard generation uses an empty compatibility-profile set. The canonical CLI's
`--compatibility-profile legacy-binary-string-v1` is an explicit versioned choice;
the library equivalent is:

```rust
use suspect_codegen::{backend::GenerationOptions, http_protocol::CompatibilityProfile};

let options = GenerationOptions {
    compatibility_profiles: [CompatibilityProfile::LegacyBinaryStringV1]
        .into_iter().collect(),
};
let plan = suspect_codegen::php_sdk::protocol::plan_sdk(
    contract, &selected_operations, config,
    options.apply_to(suspect_codegen::php_sdk::protocol::capabilities()),
)?;
```

This profile interprets the legacy binary string marker only in admitted binary
media/parts. JSON strings retain their JSON meaning. An installed native consumer
sends and receives `\x00\xffnative` as actual bytes; ordinary generation refuses
that legacy binary marker. Profiles are carried by session/cache/comparison
records, including `NativeSnapshot.generation`.

The retained plan exposes:

- `contract()`, `config()`, `models()`, `operations()`, `program()`, `examples()`;
- model `Node` / `Shape` / `Field` descriptors with original schema identities,
  allocated names, native/PHPDoc types, explicit constructor parameter order and
  constant initializers;
- allocated `CodecSymbols` (`decode`, `encode`, `from_value`, `to_value`);
- operation inputs, request media selectors, exact/range/default success/error
  wrappers, typed response headers, part/body classes, item schemas and operation
  error bases.

These descriptors drive code, examples and references. Compatibility consumers
can use them directly without parsing generated PHP.

## Explicit environment credential defaults

`PhpConfig::credential_env` accepts the shared, versioned variable-name policy:

```rust
use suspect_codegen::credential_env::CredentialEnv;

let config = PhpConfig {
    package_name: "openrouter/sdk".into(),
    package_version: "0.1.0".into(),
    namespace: "OpenRouter".into(),
    credential_env: Some(CredentialEnv::v1(
        [("apiKey".into(), "OPENROUTER_API_KEY".into())].into_iter().collect(),
    )),
    ..Default::default()
};
```

The canonical session field is
`"credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}`.
Only variable names enter generation. The PHP planner binds them through the
already admitted protocol, exposes `credential_env()`, and capture records its
semantic descriptor independently of physical provenance. Policy v1 supports
bearer and API-key strings; Basic/OAuth/OIDC mappings are located refusals.

Configured packages expose this factory on PHP 8.3+:

```php
public static function fromEnv(
    Transport $transport = new CurlTransport(),
    ClientOptions $options = new ClientOptions(),
): self;
```

For the branded OpenRouter package with `getCurrentKey` selected:

```php
require __DIR__ . '/vendor/autoload.php';
$client = OpenRouter\Client::fromEnv();
$key = $client->getCurrentKey(); // source GET /key, source-default HTTPS server
echo $key->response->status, PHP_EOL;
```

The factory snapshots each mapped `getenv` variable once at invocation. Missing,
empty, unusable, oversized or unavailable values remain missing credentials.
Operation auth resolution preserves OR/AND, explicit selection and anonymous
alternatives; protected calls with missing credentials fail before HTTP with a
secret-free `SdkError` of kind `credentials`. `getCredits` remains an explicitly
chosen management-key operation, with no inferred key privilege or mode switch.

The existing `new Client($credentials, $transport, $options)` constructor performs
no environment lookup. Explicit empty maps and missing members are authoritative;
null/`Absent` constructor arguments retain native type errors, and invalid explicit
tokens retain existing credential validation. The factory has no credential
parameter. PHP with `getenv` disabled can still use anonymous and explicit-credential
clients. No `.env` file is loaded, and later environment edits affect new clients
only. Unconfigured packages retain their ordinary bytes and constructor behavior;
factory names are reserved only when a policy is configured.

The credential-env native receipt is sealed at
`target/sdk-php-credential-env-20260911-01/native-receipt.json`: both PHP tiers passed
the installed environment-control and real-source OpenRouter gates, including
disabled `getenv`, explicit precedence and controlled source-default HTTPS calls.
All 27 no-policy artifacts matched their pre-change bytes. The opened canonical
generation/capture and policy-reversion check is recorded separately in
`target/sdk-php-credential-env-canonical-20260911-01/report.json`; native matrices
were reused for that host-only integration follow-up.

## Ordinary PHP calls

These names are generated for the five tracked public OpenRouter operations.
Use the namespace configured for your package:

```php
<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

use Example\OpenRouter as Sdk;

$token = getenv('OPENROUTER_API_KEY'); // application-owned credential lookup
if ($token === false || $token === '') {
    throw new RuntimeException('Set OPENROUTER_API_KEY');
}
$client = new Sdk\Client(new Sdk\Credentials(['apiKey' => $token]));

$credits = $client->getCredits();
echo $credits->body->data->totalCredits->token, PHP_EOL;

$created = $client->createKeys(new Sdk\CreateKeysInput(
    body: new Sdk\CreateKeysBody(
        name: 'Native Test Key',
        limit: Sdk\JsonNumber::fromString('50.25'),
        limitReset: null,
    ),
));
echo $created->response->status, PHP_EOL; // source-declared 201
```

`apiKey` is the **source scheme name**. Its declaration is HTTP bearer, producing
`Authorization: Bearer …`. The client uses only explicit credentials. Management
key guidance remains source prose; the SDK does not claim to verify key privileges
or acquire/refresh credentials.

No-required-input operations allow an omitted input. A sole success status returns
its concrete wrapper directly; several successful statuses produce a native union.
`ClientInterface` supplies an application dependency-injection seam. Source wire
names remain separate from readable allocated PHP names (`container_id` →
`containerId`, `total_credits` → `totalCredits`). Reserved words and collisions have
deterministic suffixes, preserved in the reference.

## Presence, enums and mutable models

| Source states | Native property |
| --- | --- |
| Required non-null | `T`, required constructor argument |
| Required nullable | `T|null`, required constructor argument |
| Optional non-null | `T|Absent`, default `Absent::Value` |
| Optional nullable | `T|null|Absent`, default `Absent::Value` |

For the actual key-update body, absence and explicit null remain different JSON:

```php
$patch = new Sdk\UpdateKeysBody();
$patch->toJson();                         // {}
$patch->limit = null;
$patch->toJson();                         // {"limit":null}
$patch->limit = Sdk\JsonNumber::fromString('75.50');
$patch->toJson();                         // {"limit":75.50}
$patch->limit = Sdk\Absent::Value;
$patch->toJson();                         // {}
```

String enums are native PHP enums. Source-required single-value string tags are
readonly properties initialized by the constructor: M2 uses
`new StandardPayload(text: 'plain')`, whose codec emits `kind: "standard"`.
Other model fields are mutable. Every encoder validates the current model again,
including nested values, integer constraints, extras, the chosen native union
branch and its parent schema. A mutated `Long` union carrier cannot silently encode
as a different `Short` carrier.

Open objects expose typed or `JsonValue` extras through `$extra`. Declared wire
names cannot also occur in that store. Closed objects reject unknown members.
Productive recursive object models are supported; cyclic native object instances
fail conversion. Scoped v2 closures retain declared object fields and use checked
`JsonValue` carriers for conditional/intersection shapes that cannot have a faithful
static PHP type. Scoped positional arrays use `list<JsonValue>` where necessary.
Pattern-matched extras remain lossless JSON values and are revalidated against all
matching patterns plus the source additional-member policy.

PHPStan checks generic list/map elements and public union types. The generated
configuration is **level max**, with defensive runtime checking enabled through
`treatPhpDocTypesAsCertain: false`; no error baseline or ignore list is emitted.
Callers should use `declare(strict_types=1)` for PHP scalar arguments.

## Exact values and checked codecs

`JsonNumber` retains the original JSON numeric token and a symbolic normalized
coefficient/exponent. It never parses an API number through a float. General
numbers and unbounded integers use this exact carrier; integer integrality and
numeric assertions are checked at the codec boundary. Native integer conversion
is explicit and range-checked:

```php
$n = Sdk\JsonNumber::fromString('9007199254740993.000000000000000001');
$n->token;
$n->compare(Sdk\JsonNumber::fromInt(1));
$integer = Sdk\JsonNumber::fromString('10e-000001')->toInt(); // 1
$decimal = $n->toDecimalString(maxBytes: 128);
```

`JsonValue` represents the schema's actual arbitrary JSON domain. Its immutable
factories distinguish objects from lists and booleans from numbers; arbitrary JSON
null is `JsonValue::null()`. The parser rejects duplicate decoded object names,
invalid UTF-8, unpaired surrogates and malformed numeric grammar. Numeric member
names survive PHP's integer-key normalization.

Models/enums expose `fromJson()` and `toJson()`. Every schema also has allocated
`Codecs::decodeX`, `encodeX`, `fromX(JsonValue)` and `toX` methods. Use these checked
entry points for exact model serialization. JSON parsing/writing and conversion
are bounded; generic JSON descendants and repeated aliases spend the same
conversion budgets as named fields.

`emit_validation(&OwnedProgram, &PhpConfig)` separately emits the checked portable
validator plus exact JSON runtime. It covers the verified **v1** instruction
families, including portable pattern NFAs, tuples, exact enum/const equality and
composition, and the additive **v2 scoped-applicator** profile.
This standalone validator can express constraints beyond the admitted model
carriers. `Validator::validate(root, value)` throws a source/path-bearing
`ValidationError`: `invalid` means completed invalidity, while
`evaluation_failure` means incomplete evaluation. The first mismatch is reported;
later checks still execute so an evaluation failure dominates earlier mismatches
or successful union arms. Branch trials share schema, equality and numeric work.
The HTTP planner uses `OwnedCompiler::compile_v2` and the canonical
`plan_protocol_examples_v2` helper for scoped source admission, and uses explicit
`compile_v3` / `plan_protocol_examples_v3` when canonical resource/dynamic
semantics are needed. Ordinary base/scoped closures retain their v1/v2 program
bytes and profiles. Unknown versions, mismatched profiles and malformed operands
fail with original source identity before PHP artifacts are returned.

### Scoped v2 rules

Native v2 implements `if`, `dependentRequired`, `dependentSchemas`, `contains`,
`patternProperties`, pattern-aware `additionalProperties`, `propertyNames`,
`unevaluatedProperties` and `unevaluatedItems` according to
[the shared executable contract](SDK-SCHEMA-APPLICATORS.md).

Every child evaluation starts with fresh evaluated-property/index sets. Successful
same-instance applicators propagate only the specified sets; failed schemas export
none. All matching patterns apply. Property-name checks validate decoded keys at
the member's instance pointer without marking the value. Condition and contains
annotations follow their independent contribution rules even when a selected
branch or contains count fails locally. Set merges charge every candidate,
including duplicates, to shared work limits. Trials never reset work, equality,
numeric, recursion or cancellation state, and evaluation failures remain
noninvertible.

Model-only callers retain explicit codec obligations: public decoders/encoders and
model `fromJson()`/`toJson()` methods revalidate the complete source schema. Dynamic
pattern extras are not dropped or mistaken for forbidden undeclared fields.
Compatibility records retain the pattern dispatch policy, typed fields,
`JsonValue` carriers and validation version/profile without execution indices.

## Transport, errors and lifetime

The default `CurlTransport` uses ext-curl/libcurl 7.85+. `Transport` is a small
interoperable seam:

```php
interface Transport {
    public function send(HttpRequest $request): HttpResponse;
}
```

Custom adapters receive method, fully encoded URL, explicit headers/body, remaining
timeout, response/header/capture ceilings and `check()`. They preserve repeated
headers and raw content encoding, check cancellation/deadline while reading, and
release owned resources before returning or throwing. Framework/PSR-18 bridges
need the underlying framework's bounded-read and timeout facilities. The client
also checks a completed custom response and detects a late return; it cannot
preempt arbitrary blocking application code.

The default adapter:

- sends once with source-selected explicit credentials and TLS verification;
- disables redirects, ambient proxies/netrc, cookie storage and decompression;
- bounds aggregate headers and streamed body accumulation, retaining a separate
  bounded failure prefix with `truncated` state;
- accepts identity content encoding and dispatches the selected declared media
  type, preserving media parameters and duplicate header fields;
- closes each connection/handle on success, timeout, cancellation and failure;
- preserves monotonic whole-call deadlines through preparation, transport and
  decoding; generated work observes a cooperative `CancellationToken`.

`ClientOptions` sets client-wide limits and an explicit server override (HTTPS or
HTTP loopback fixtures), or a source `serverIndex`, `serverVariables` and relative
`serverBaseUrl`. `RequestOptions` can lower timeout/response/capture limits and
select a security alternative explicitly.
Documented API statuses throw generated operation-specific subclasses of
`ApiError`, carrying a typed `$body` and response metadata. Other `SdkError` kinds
include `request_validation`, `response_validation`, `transport`, `timeout`,
`cancelled`, `resource_limit`, `unexpected_status`, `unexpected_media`,
`unexpected_encoding`, `credentials` and `configuration`. Their structured
operation/source identities, bounded captures and causes remain accessible;
default formatting omits payloads and causes.

## Security, parameters and response dispatch

- Security supports anonymous, OR and AND alternatives; bearer, Basic and
  header/query/cookie API keys; and caller-owned OAuth/OIDC authorization hooks.
  The first complete source alternative wins unless the caller selects an index.
  A source anonymous alternative can therefore win even when credentials exist.
- Credential strings keep their bearer-token meaning. Other mechanisms use
  `BasicCredential`, `ApiKeyCredential`, `AuthorizationCredential`, or an
  OAuth/OIDC closure receiving `CredentialRequest` with operation, scheme,
  permissions and immutable source metadata. Acquisition and refresh stay with
  the caller. Conflicting source credential/parameter bindings are located
  admission failures.
- Relative/multiple/variable servers preserve source defaults, enums and selection
  order. Relative file fixtures need an explicit absolute base. Resolved relative
  paths preserve meaningful double slashes. `DocumentRelativeServers` uses the
  effective physical retrieval document from `ServerPlan::document_base()`,
  including redirect aliases and external Server Objects. `$self`/`$id` logical
  names do not relocate the API. Encoded dot/slash spelling is preserved through
  an explicit cURL request target. Caller OAuth/OIDC hooks receive the selected
  effective `CredentialRequest::$serverUrl` alongside unchanged relative URL metadata.
- Standard HTTP methods, OAS 3.2 QUERY and case-sensitive custom method tokens are
  retained. Simple, label, matrix, form, space/pipe-delimited and deep-object
  parameters use the source shape, explode and encoding policy. Header/cookie,
  reserved expansion, content parameters and whole-query form objects are supported
  within the admitted unambiguous profiles.
- Exact status precedes range, then default. Actual status determines success or
  error. Media mismatch never falls back to a less specific status declaration.
  Concrete, type-wildcard and any-wildcard media retain specificity and declared
  parameters. JSON, `+json`, schema-free JSON, text and bytes remain distinct.
- Undeclared response content is bounded `Bytes`; an undeclared response status
  is a classified `SdkError` with a bounded capture. HEAD, 1xx, 204, 205 and 304
  use `NoBody::Value` and reject forbidden content.
- Results and declared errors expose `body`, `response`, typed `headers` and
  `list<Link>`. Link metadata is caller guidance, with no automatic invocation.
  `ApiError::$response` has native `HttpResponse|StreamResponse` storage;
  generated constructors and PHPDoc retain the concrete response kind.

## Forms, multipart and item streams

Forms use allocated native body classes and checked per-field codecs. Named
multipart uses separate aggregate and part classes: binary fields hold `Bytes`,
and parts retain filename, content type, typed declared headers and explicit extra
headers. Aggregate requiredness, additional members, property counts and repeated
item counts are checked structurally. MIME parsing preserves binary boundary-like
prefixes and ignores preamble/epilogue comments. Binary aggregates never enter a
JSON codec through placeholder null values.

OAS 3.2 `itemSchema` generates `iterable<T>` requests and `ItemStream<T>` responses.
Request iterables are consumed once under finite item/request ceilings before
transport. Response cURL streaming queues one native chunk and pauses/resumes
reading. SSE exposes the parsed envelope: multiline data is joined with LF,
comments/unknown fields and invalid id/retry values are ignored, and retry remains
an exact integer. UTF-8 follows the SSE replacement rule. Data strings are not
implicitly parsed as JSON, and `[DONE]` remains ordinary data.

JSON-lines validates each record independently, including a final record without
LF. BOMs, malformed JSON/UTF-8 and invalid item values fail. Streams enforce line,
item, total-byte, item-count, capture, cancellation and deadline bounds. Retaining
a response or iterator requires explicit cleanup:

```php
$response = $client->events(); // generated for a source operationId of events
try {
    foreach ($response->body as $event) {
        // Use the source-typed envelope. Its data field is a string.
        break;
    }
} finally {
    $response->body->close();
}
```

`StreamTransport::open(HttpRequest): StreamResponse` and `BodyReader` are the
custom-adapter seams. Readers return a nonempty chunk or null at EOF and close
idempotently. Exhaustion, early iterator disposal, schema/framing failure,
cancellation and deadlines release owned readers. Cleanup failures preserve a
primary failure.

## Package and documentation artifacts

- Composer manifest, classmap autoloading and pinned PHPStan development dependency.
- Native model/operation/runtime PHPDoc and a browsable `docs/index.html` reference
  linking actual constructors, fields, enum cases, codecs, statuses and sources.
- `docs/coverage.json` includes constructor/field/native/PHPDoc coverage for all
  protocol classes and complete-call recipe availability. `docs/examples.json`
  retains v2 request/response, header, part and item roles with provenance.
- Executable native constructor/codec examples, mock-backed operation calls and a
  complete first-request guide with explicit real-byte scaffolding. README and
  `examples/quickstart.php` use the same generator; `examples/client.php` includes
  all available complete calls. Native checks typecheck and execute them and
  validate the browsable reference against runtime reflection.

Invalid declared examples remain findings. Synthesized values are explicitly
labeled. Source prose is inert in PHP comments/literals and escaped in HTML.

## Finite defaults and current boundary

| Policy | Default |
| --- | ---: |
| Request URL / body | 8 MiB each |
| Response body | 8 MiB |
| Aggregate headers | 64 KiB |
| Failure body capture | 16 KiB, lowerable to zero |
| JSON/native conversion depth | 128 |
| JSON/native conversion node visits | 100,000 |
| Native conversion bytes | 32 MiB |
| JSON numeric token | 65,536 bytes |
| Schema numeric operands | OwnedProgram policy (4,096 bytes by default) |
| Schema visits, equality, numeric work | Shared finite OwnedProgram-derived policy |

The default protocol adapter supports the verified OpenAPI **3.1 / 3.2** surfaces
above. OpenAPI 3.0, active directional model projections, unnamed operations,
positional multipart, and ambiguous composite response form/part styles remain
source refusals. Portable v3 resource/dynamic execution is separately verified. Native
capabilities come from the PHP adapter; a versioned compatibility profile cannot
enable unwitnessed native features.

## Reproduce native verification

```sh
SUSPECT_PHP_BIN=$PWD/target/sdk-php-tools/php-8.3.32/php \
cargo test -p suspect-codegen --no-default-features \
  --features php-sdk,http-protocol --test php_protocol -- \
  --include-ignored --test-threads=1
```

Repeat with `php-8.5.8/php`. `SUSPECT_PHPSTAN_PHAR` can select the isolated PHPStan
PHAR. Missing required tools/source fail these opt-in gates.

Each advanced native case retains a new directory in `target/sdk-php-protocol/`, with emitted
sources, Composer archives, a separate installed consumer, native logs and
`commands.jsonl`. Installed source bytes are compared to the complete emitted
artifact set. Independent advanced loopback records cover parameter styles,
whole-query forms, standard/custom methods, auth, real bytes, request items and
early stream close. The historical base evidence in `target/sdk-php-native/`
covers M2 and the five original
OpenRouter operations (`getCredits`, `createKeys`, `updateKeys`,
`listContainerFiles`, `getContainerFile`). Additional gates exercise negative
PHPStan cases, reflection/PHPDoc/reference links, source prose, all 17 shared
OwnedProgram schemas, 10,000 independent exact-rational cases and hostile shared
resource budgets. No generated file is manually repaired.

### Preserved base checkpoint — 2026-09-10

Both PHP **8.3.32** and **8.5.8** completed **10 tests: 10 passed, 0 failed,
0 ignored**. Native execution also requires clean runtime diagnostics. The
preserved report, tool hashes, owned-source archive, commands and artifact paths
are in **`target/sdk-php-verified-20260910-02/report.json`**.

| Installed package | Operations | Source-indexed codecs | Object constructors | Status wrappers | Validated native examples |
| --- | ---: | ---: | ---: | ---: | ---: |
| M2 | 4 | 46 | 8 | 9 | 17 |
| Five actual OpenRouter operations | 5 | 189 | 39 | 31 | 39 |

Each package contains 21 emitted artifacts. The shared validation gate covers
17 schemas / 81 independently specified instances; each runtime also executes
10,000 independent rational arithmetic cases. The adversarial gate covers
nullable collection PHPDoc, mutable union carriers, parent constraints,
source-prose injection and all four presence states. Resource checks include
nonproductive references, branch budgets and unvisited enum/uniqueness operands.

Warnings-denied Rustdoc and owned-file formatting/whitespace checks pass.
Warnings-denied Clippy encountered concurrent shared-file findings in
`suspect-ir/src/contract/{shape,walk}.rs` and
`suspect-codegen/src/generation_session.rs`; their original output is retained
in the report and those files remain with the main integration owner.

### Expanded protocol checkpoint — 2026-09-10

PHP **8.3.32** and **8.5.8** each passed **9/9 advanced native gates** using the
actual codegen crate with `php-sdk,http-protocol`. Every case was archived with
Composer, installed into an independent consumer, checked with PHPStan max, and
exercised through native PHP. The new report is
**`target/sdk-php-advanced-20260910-01/report.json`**. The base report remains the
evidence for the completed base/numeric matrices.

The nine advanced gates cover:

1. Explicit `LegacyBinaryStringV1` native byte identity and ordinary-profile refusal.
2. Independent cURL auth/header/byte requests and stream connection release.
3. Form/named-multipart request bytes, typed part headers and negative PHPStan cases.
4. Parameter/media/status precedence, QUERY/custom/standard wire methods and no-body rules.
5. Installed package, native types and injected protocol exchanges.
6. Single-pass request iterables, independent SSE/JSON-lines wire bytes and finite preparation.
7. Independent form/MIME response bytes, structural bounds and typed metadata.
8. Anonymous/OR/AND auth, caller OAuth/OIDC metadata, roles, links and server choices.
9. Split-byte item framing, UTF-8 behavior, early close, faults, cancellation and deadlines.

All generated codec/client/quickstart examples executed. Reflection checked
protocol constructors, native field types, generic PHPDoc, concrete API-error
documentation and local reference links. The PHP integration checks additionally
verify iterable direction, body/part/header descriptors and located v2 refusal.
The original credential descriptor and namespace diagnostic regressions are fixed.
Main's completed all-12-target `GenerationOptions` checks remain recorded in
`target/sdk-full-options-integrations-01.log`.

### Native validator v2 checkpoint — 2026-09-10

New evidence is retained under
**`target/sdk-php-validation-v2-verified-20260910-01/report.json`**. PHP **8.3.32**
and **8.5.8** each passed the three native v2 gates:

| Gate | Evidence |
| --- | --- |
| `native_v2_source_vectors` | All 32 maintained source fixtures compiled through `compile_v2`; exact Valid/Invalid/EvaluationFailure, source and instance paths; installed Composer corpus; PHPStan max |
| `native_v2_scope_resource_controls` | 12 independent merge-cost, duplicate-candidate, key-identity, equality, recursion/depth and failure vectors; cooperative stop/reuse and released instance ownership |
| `native_v2_sdk_packages_models_codecs` | Three real generated operations, typed mutable/constant fields, null versus omission, overlapping pattern extras, checked JSON/positional carriers, positive/negative consumers, native examples and reflection/reference checks |

The regular `v1_program_bytes_and_unwitnessed_v3_are_preserved` check verifies
byte equality for base programs/emission through `compile` versus `compile_v2`,
version/profile guards, and an original-source `$dynamicRef` refusal. Completed
base, numeric, HTTP and all-12-options matrices were reused rather than restarted.

```sh
SUSPECT_PHP_BIN=$PWD/target/sdk-php-tools/php-8.3.32/php \
cargo test -p suspect-codegen --no-default-features \
  --features php-sdk,http-protocol --lib php_sdk::tests_v2:: -- \
  --include-ignored --test-threads=1
```

Use `php-8.5.8/php` for the current tier. `SUSPECT_COMPOSER_PHAR` and
`SUSPECT_PHPSTAN_PHAR` override the verified default PHAR paths. Maintained gates
load `crates/suspect-schema/tests/fixtures/owned-applicators-v2.json` directly;
they have no dependency on a historical `target/` witness report.

The separately focused
`php_protocol::native_document_relative_servers_keep_physical_bases` gate passed
on both PHP tiers before `DocumentRelativeServers` was enabled. It uses supplied
requested/effective/logical document identities and two independent loopback
origins, checking raw encoded targets, an external server's physical base,
explicit empty server overrides, caller document-base/full-URL overrides and the
OAuth/OIDC effective-server metadata base. Its fresh logs are
`target/sdk-php-integration-checks/document-base-83-04.log` and
`target/sdk-php-integration-checks/document-base-85-05.log`.

### Native resource/dynamic v3 checkpoint — 2026-09-10

The existing `plan_sdk`, backend and standalone `emit_validation` entrypoints
support the checked v3 pair
`suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic`. The planner first tries the established
v2 admission, retaining ordinary v1/v2 envelopes, and uses explicit `compile_v3`
for source resources/dynamic semantics. No new target configuration field or
special public entrypoint is required.

`SchemaResources` and `DynamicSchemaReferences` are enabled after native proof.
The runtime enters the node's indexed resource even for a nested entry, searches
actually entered resources outermost first, and never enters the initial fallback
before lookup. Pointer/empty/static-anchor fallbacks stay static. Unentered
catalogue candidates are inert. Returns and trials restore the resource stack;
cycle keys include exact ordered resource-context identity. Each new distinct
resource and inspected resource/binding consumes the shared work budget. Dynamic
targets start with fresh annotation scope and propagate only successful sets.

Logical canonical/base/alias URIs remain metadata, separate from physical source
ownership. Validation performs no URI resolution, acquisition or source loading.
The native model surface retains declared typed fields. V3 union and dynamic
values use checked `JsonValue` carriers where choosing a native branch outside
the caller's resource context would be unsound. Model-only users must call the
codec for their intended source root; a base model's own codec cannot replace a
stricter resource's codec.

New immutable evidence is under
**`target/sdk-php-validation-v3-verified-20260910-01/report.json`**. The maintained
`php_sdk::tests_v3` suite compiles unmodified source fixtures through `compile_v3`:

| Gate | Evidence on PHP 8.3.32 and 8.5.8 |
| --- | --- |
| `native_v3_source_fixtures` | All 44 official dynamic-reference cases and original supplied remotes from `resource-conformance/`; installed Composer corpus and PHPStan max |
| `native_v3_scope_resource_controls` | 19 independent outermost/unentered/nested-entry/fallback/context-cycle/visit/depth/failure controls, cancellation/ownership checks, 8 malformed programs and old-envelope refusals |
| `native_v3_sdk_packages_models_codecs` | Resource-aware operations, deep strict-tree override, nested-root binding, dynamic union carrier, exact numeric wire bytes, positive/negative types, native examples and reference/reflection |

`ordinary_v1_v2_programs_and_entrypoints_keep_their_bytes` compares ordinary v1/v2
program and full artifact bytes across the v3-capable path. The original base,
10,000-case arithmetic, v2 and HTTP reports keep their original scope and bytes.

```sh
SUSPECT_PHP_BIN=$PWD/target/sdk-php-tools/php-8.3.32/php \
cargo test -p suspect-codegen --no-default-features \
  --features php-sdk,http-protocol --lib php_sdk::tests_v3:: -- \
  --include-ignored --test-threads=1
```

The same PHP/Composer/PHPStan selectors apply. Native tests do not depend on
`target/sdk-schema-resources-executable-v3.json`; it remains a historical export.
