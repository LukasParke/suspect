# Native Ruby SDK backend

## Runtime credential environment defaults — v1

Ruby implements the shared [credential-env contract](SDK-CREDENTIAL-ENV.md).
Configure **variable names** in the SDK session:

```json
{"credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}}
```

`RubyConfig::credential_env` defaults to `None`. `plan_sdk` binds a configured
policy with `credential_env::plan` after HTTP admission and retains it behind
`SdkPlan::credential_env()`. Ruby's compatibility capture records the bound
`semantic_descriptor()`, keeping physical provenance out of interface equality.

For the verified `openrouter` gem/require and `OpenRouter` namespace:

```ruby
require 'openrouter'

OpenRouter::Client.open do |client|
  response = client.get_current_key
  puts response.status
  puts response.data.data.label
end
```

Both `Client.new` and `Client.open` snapshot mapped variables only when `auth:`
is **truly omitted**. `get_current_key` is the source `GET /key`; `get_credits` is
the optional management-key call. With no server override, their source URLs are
`https://openrouter.ai/api/v1/key` and `/credits`. `transport:` retains the existing
injectable interface for controlled execution.

- The complete explicit `auth:` argument wins. `{}`, missing members, empty
  strings and null values are never supplemented from the environment.
- Explicit `auth: nil` keeps the established `ArgumentError`; explicit bearer
  null/empty values also keep the existing constructor validation. An empty hash
  or absent required member produces the normal protected-operation
  `RequestError` before HTTP. Explicit API-key strings keep their existing
  attachment validation.
- Missing, empty, invalid, oversized or unavailable environment strings stay
  missing. Anonymous operations remain usable. The original OR/AND and explicit
  `security:` selection rules apply; no alternative is selected by availability.
- Import and generation read no credential values. An existing client keeps its
  creation-time copy after the environment changes; newly created clients see
  the updated values. Reader errors and their potentially secret-bearing causes
  are discarded, and protected-operation failures remain secret-free.

Configured packages add `lib/openrouter/credential_env.rb` and
`lib/openrouter/credential-env.json`, an entrypoint require and a README section.
The runtime uses a private `Internal::CredentialEnvConstructor` module with
private binding/sentinel constants. It adds only the already-reserved
`initialize` method, so source operations such as `snapshot` and `env_bindings`
retain their names. The JSON support file contains bound variable names and
physical provenance. No new public factory or signature is required.

### Credential-env tests

`tests/ruby_credential_env.rs` selectors run on Ruby 3.3.12 and 4.0.6:

- `bound_policy_uses_admitted_source_names_and_keeps_no_policy_bytes` — runs
  real Sessions through no policy → configured → disabled, restoring the exact
  original files/revision after disabling. Configured output adds exactly
  `credential_env.rb` and `credential-env.json`; only the entrypoint require and
  README may change among existing files. Two physical fixture locations retain
  their actual URIs, pointers and spans. Two separate controlled Rust generator
  processes run the Session sequence under different ambient credential values;
  output bytes/revisions must agree and neither value may appear in an artifact.
  This needs no Ruby installation.
- `credential_env_refusals_are_shared_and_follow_protocol_admission`
- `canonical_credential_env_capture_is_semantic_and_keeps_credential_surface` —
  source relocation preserves equality, absent policy is omitted, and a
  variable-name change is a native client-default change with no wire delta.
- `installed_credential_env_omission_precedence_choices_and_snapshot`
- `installed_openrouter_env_uses_source_default_https_and_real_key_schemas` —
  compares no-policy/configured artifacts from one loaded Contract, then runs
  controlled synthetic and actual-source OpenRouter exchanges, explicit
  precedence, OR/AND/anonymous, unavailable `ENV`, creation snapshots, gem
  installation, RBS, Steep and YARD. It needs `OPENROUTER_WEB_ROOT` containing
  `projects/docs/openapi/openapi.yaml`, plus the selected Ruby installation and
  tool gems. The `SUSPECT_RUBY_ENV_HOST_PROBE_*` variables are internal to the
  host test, not runner inputs.

The installed selectors are `#[ignore]`; no account requests are made.

## Resource/dynamic v3

`SchemaResources` and `DynamicSchemaReferences` are enabled by default.
The existing `ruby_sdk::plan_sdk` selects `OwnedCompiler::compile_v3` from indexed
Contract resource/scope/dynamic metadata. Its exact pair is:

- `suspect.validation.experimental.v3`
- `oas31-jsonschema202012-resources-dynamic`

Ordinary closures continue through `compile_v2`, retaining their established
v1/v2 envelopes and byte identity. There is no separate public SDK dispatcher.
`RubyConfig::{schema_resources,dynamic_schema_references}` default to true; explicit
false values retain source-linked capability refusals. The
`document_relative_servers` option also defaults to true.

### Checked resources, evaluation and native values

`resource_guard.rb` validates the resource registry, aligned node scopes, physical
containment, canonical/base/alias URI syntax and uniqueness, canonical addresses,
declaration and anchor locations, sorted bindings, finite targets, and each
dynamic instruction's initial target/resource/name agreement. Both native program
entrypoints reject malformed metadata, mixed version/profile pairs and unknown
fields before evaluation. V1/v2 reject resource metadata and the new opcode.

`validation_v3.rb` activates each node's indexed resource, including nested codec
entries whose resource root is never evaluated. It looks up dynamic bindings in
outermost-first entered-resource order before entering a fallback, and restores
scope on every return and trial. Cycle identity includes the exact ordered
resource context. Dynamic targets receive fresh annotation scopes and propagate
only successful evaluated-location sets. Resource entry and every inspected
resource/binding consume the shared work budget. Cycles and budget/depth failures
remain noninvertible `EvaluationFailure` outcomes.

Named fields keep keyword models. `ModelShape::Dynamic` retains the initial
target, initial resource, optional anchor and candidate indices, while storing
exact JSON values at dynamic-reference sites. Candidates stay validation
dependencies; they do not require standalone native model carriers. V3 union
carriers use source-validated JSON to avoid context-free branch re-trials.
All parent/operation codecs execute the full original program. Physical source
URIs/pointers remain codec and finding identities; logical names are metadata.

The canonical `examples::plan_protocol_examples_v3` supplies declared examples,
invalid findings and bounded synthesis. Native construction starts through the
actual enclosing codec, preserving outer bindings when nested constructors would
otherwise use a different standalone fallback. Runtime validation performs no
document acquisition or raw-schema interpretation. Legacy recursive references,
custom dialects/vocabularies, directional-codec constraints and the established
format/regex limits retain their explicit refusals.

### v3 tests

`tests/ruby_schema_v3.rs` provides these exact selectors:

- `source_driven_official_v3_resources_scopes_and_guards` — compiles the original
  44 official cases in `suspect-schema/tests/fixtures/resource-conformance/`, with
  supplied remote documents through a closed provider, using real `compile_v3`.
  It also executes the four official dynamic/unevaluated cases, 32 independent
  scope/branch/fallback/annotation/budget/depth controls and two v1/v2 compatibility
  controls: **82 outcomes total**, plus **38 malformed programs through two
  entrypoints**. It does not depend on a generated witness file.
- `resource_native_descriptors_and_profile_selection_preserve_physical_identity`
  — named/dynamic carriers, candidate dependencies, physical/logical provenance,
  source-linked capability and unsupported-profile refusals, default-versus-opt-in
  emission, ordinary v1/v2 byte identity, and canonical generation/compatibility.
- `installed_resource_sdk_models_types_examples_and_wire` — builds and locally
  installs the gem, verifies emitted/installed byte identity, typed keyword models,
  incompatible outer-versus-fallback bindings, mutable exact JSON, absent/null,
  unentered candidates, real request/response failures, RBS, positive/eight
  negative Steep consumers, YARD HTML, three offline example exchanges and the
  installed quickstart.

```sh
cargo test --locked -p suspect-codegen --no-default-features --features ruby-sdk \
  --test ruby --target-dir target/sdk-ruby-schema-v3-cargo -- \
  --include-ignored --test-threads=1

SUSPECT_RUBY_HOME="$HOME/.local/share/mise/installs/ruby/4.0.6" \
SUSPECT_RUBY_GEMS="$PWD/target/sdk-ruby-tools/gems-ruby4" \
cargo test --locked -p suspect-codegen --no-default-features --features ruby-sdk \
  --test ruby --target-dir target/sdk-ruby-schema-v3-cargo -- \
  --include-ignored --test-threads=1
```

The compatibility adapter retains typed dynamic metadata and the existing
`ruby-http-protocol-v1` identity. The v3 runtime assets are
`ruby_sdk/resource_guard.rb` and `ruby_sdk/validation_v3.rb`, both in
`ruby_sdk::source_assets()`.

## Physical server documents

`DocumentRelativeServers` is enabled by default. `ServerPlan.document_base`
supplies the effective physical retrieval document, including redirect
identity. An absent inherited
server array uses the entry document; an explicit empty override uses the
overriding document. Neither an operation's unrelated physical file nor `$self`
or `$id` is substituted as that server base. Caller `document_url:` remains an
explicit override. Ruby's RFC3986 resolution preserves encoded dots/slashes,
hex case, and non-dot path spelling through the actual HTTP request.

Credential contexts and OAuth flows expose `url_base` and `server_url`.
Relative OAuth/OIDC endpoint strings remain unchanged metadata with an
`effective-server` base; without an `sdk_defaults.oauth` policy the SDK performs
no discovery or acquisition. Logical
resource context stays beside physical provenance in the retained protocol plan.
`codec_schema_closure()` supplies compilation dependencies while `codec_roots()`
continues to describe actual codec inputs.

Focused target: `tests/ruby_document_servers.rs`:

- `physical_base_metadata_is_separate_and_schema_resource_fences_stay_closed`
- `installed_gem_uses_effective_physical_document_server_bases`

The schema-resource fence checks use explicit false options.

## Scoped schema v2

Ruby uses `OwnedCompiler::compile_v2` for closures that need the scoped
applicators. Ordinary closures retain the exact
`suspect.validation.experimental.v1` / `oas31-jsonschema202012-static-subset`
program bytes. Only closures requiring the scoped applicators emit
`suspect.validation.experimental.v2` / `oas31-jsonschema202012-static-applicators`;
the v3 section above extends the supported pairs
([ADR 0020](adr/0020-validation-programs-are-versioned-and-frozen.md)).

All nine instructions are implemented: `if`, `dependentRequired`,
`dependentSchemas`, `contains`, `patternProperties`,
`additionalPropertiesWithPatterns`, `propertyNames`, `unevaluatedProperties`,
and `unevaluatedItems`. Every subschema has a fresh local property/index scope;
only the documented successful contributions propagate. Set merges charge every
candidate insertion, including duplicates, in Unicode-scalar/index order.
Conditions, selected branches, contains counts and failed unions preserve the
exact contribution rules in [SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md).
Depth, NFA, numeric/equality and shared work failures remain noninvertible.

`program_guard.rb` validates the complete portable envelope before execution:
versions/profiles, opcodes, operand kinds, target/source identities, unique names,
pattern graphs/ranges, exact numeric bounds, adjacent pattern exclusions and
unevaluated-check ordering. Unknown fields and instructions cannot hide inside an
inactive instance kind or branch. Explicitly supplied programs are checked without
mutating/freezing the caller's containers; shipped metadata is deeply immutable.

### Native models and examples

Named object fields keep their keyword constructors and typed attributes.
Pattern/unevaluated extras use `ExtraFields::Scoped` and remain exact JSON values
in `extra_fields`; the complete source program checks every matching pattern,
additional schema and unevaluated rule. Overlaps are conjunctive and cannot be
flattened into one additional-properties type. `PatternExtra` retains the pattern,
real child schema index and source identity. Where an untyped or combined schema
has no proven native carrier, `RefinedJson` retains its full checked JSON domain.
No field or pattern-matched extra is stripped on decode/encode.

The canonical `examples::plan_protocol_examples_v2` supplies v2 declared values,
provenance, invalid findings and bounded synthesis. Ruby's existing typed native
construction trees render these examples and the quickstart. V1 examples continue
through `plan_protocol_examples`. No secondary HTTP admission or schema rewrite
is used. The compatibility adapter records scoped extra rules and actual codec
methods plus the program version/profile.

The v2 runtime assets are `ruby_sdk/program_guard.rb` and
`ruby_sdk/validation_v2.rb`, both in `ruby_sdk::source_assets()`.

### v2 tests

`tests/ruby_schema_v2.rs` compiles the maintained
`suspect-schema/tests/fixtures/owned-applicators-v2.json` source fixture through
the real `compile_v2` API.

- `source_driven_v2_instructions_scopes_budgets_and_program_guards` — 32 maintained
  cases plus 13 additional merge-cost, source/path, recursion, failure and v1
  byte-parity checks; malformed-program mutation gates.
- `scoped_native_descriptors_retain_fields_patterns_and_real_program_identity` —
  typed fields/extras, declared invalid-example findings and exact v1 program bytes.
- `installed_scoped_sdk_models_types_examples_and_wire` — real gem build/local
  install, mutable native models, exact decimals, omission/null, checked pattern
  extras, independent HTTP requests, RBS, positive/seven negative Steep consumers,
  generated executable examples and YARD HTML.

```sh
cargo test --locked -p suspect-codegen --test ruby \
  --target-dir target/sdk-ruby-schema-v2-cargo -- \
  --include-ignored --test-threads=1

SUSPECT_RUBY_HOME="$HOME/.local/share/mise/installs/ruby/4.0.6" \
SUSPECT_RUBY_GEMS="$PWD/target/sdk-ruby-tools/gems-ruby4" \
cargo test --locked -p suspect-codegen --test ruby \
  --target-dir target/sdk-ruby-schema-v2-cargo -- \
  --include-ignored --test-threads=1
```

These source-driven tests do not depend on a frozen-program witness.
Resource/dynamic-scope v3 is outside this nine-operation set; it is never
guessed as static reference behavior. The canonical provenance module consumes
`source_assets()` for both empty and planned snapshots.

## HTTP protocol

Ruby consumes **`http_protocol::plan` directly**. Native
`Models`, codecs, keyword calls, public exceptions, gems, YARD, RBS and examples
remain one pipeline. `examples::plan_protocol_examples` supplies version-2
response-pattern/header/part/item roles and actual source codec roots.

The adapter identity is `ruby-http-protocol-v1`. Its native tests cover:

  case-insensitive bearer/Basic, header/query/cookie API keys, and caller-provided
  OAuth/OIDC attachment hooks. `CredentialContext` exposes source, permissions,
  scopes/roles, typed flow metadata and discovery/metadata URLs. No acquisition,
  scopes/roles, typed flow metadata and discovery/metadata URLs. Without an
  `sdk_defaults.oauth` policy no acquisition, discovery, refresh, token-type
  selection or retry is automatic; the opt-in OAuth lifecycle is described in
  [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md).
- Multiple and relative servers, declared variable defaults/enums and explicit
  overrides. Local file inputs require a supplied HTTP `document_url` for relative
  resolution. Standard methods, QUERY and case-preserved custom method tokens.
- Scalar, array and flat-object style serialization; header/cookie, reserved and
  content parameters; whole-query JSON/text/form encoding. The native tests use
  the 30 literal normative vectors over actual loopback HTTP.
- Exact/range/default status precedence with actual-status success; JSON, `+json`,
  UTF-8 text scalars, bytes, wildcard/media-parameter matching, and bounded
  undeclared response content. HEAD/1xx/204/205/304 use `NO_CONTENT`, separately
  from JSON null and `Bytes`.
- Native typed response/part header records and immutable Link metadata. Repeated
  raw `Set-Cookie` values stay separate; the core's unsupported typed comma-folded
  Set-Cookie declaration is declined with its original source.
- Generated keyword form/named-multipart records, repeated actual byte parts,
  source-bound part/header codecs, required/extras/cardinality checks and finite
  positional multipart. Mixed byte-bearing aggregates never enter JSON Schema
  validation with substitute null values.
- Standard OAS 3.2 SSE-envelope and JSON-lines item schemas, native closeable
  Enumerators, arbitrary transport chunk boundaries, backpressure, early close,
  cancellation and deadlines that remain active while paused. SSE `data` stays
  text, `[DONE]` is ordinary data, and retry/id metadata triggers no reconnect loop.
  Finite request item sequences are validated and bounded before dispatch.
- OAS 3.0 normalized nullable/reference-only semantics and native binary contexts.
  Legacy OAS 3.1 binary-string markers require the explicit
  `RubyConfig::legacy_binary_strings` compatibility profile; they are never inferred.

### Native protocol API

```ruby
api = ExampleSDK::Client.new(
  auth: {'basic' => ExampleSDK::BasicCredential.new(username: user, password: password)},
  server: 'tenant', server_variables: {'tenant' => 'customer'}
)
# Explicit source choices are keyword options on calls:
# content_type:, accept:, security:, server:, server_variables:, document_url:

file = ExampleSDK::BytePart.new(
  bytes: supplied_octets, filename: 'report.bin', content_type: 'application/octet-stream'
)
# Put file in the source-generated keyword body model.
```

Sequential response data is an `ItemStream < Enumerator`. `each` ensures cleanup;
after `next`, explicitly `close` in an ensure block. JSON numbers retain exact
tokens through item codecs. Response data, `typed_headers` and `links` are public;
operation errors remain public classes and range/default errors keep actual status.

### Immutable Rust descriptors

- `SdkPlan::protocol()` retains the complete shared `ProtocolPlan`;
  `records()` exposes native form/part/header records and `schema_index(id)` looks
  up a real codec carrier. `source_assets()` publishes the complete native asset
- `PlannedOperation::wire` retains servers, security, method and all provenance.
  `PlannedParameter::{value_type,wire,serialization(),schema_index()}` exposes the
  native type plus the canonical serialization; its codec index is optional for
  native form records.
- Bodies and responses expose `media: Vec<PlannedMedia>`. Each media retains its
  exact `MediaPlan` and `NativeType`; no first-media projection stands in for the
  complete interface. Responses retain `status`, `status_key`, success/error class
  names, typed headers, `headers_class` and `body_forbidden`.
- `NativeType` distinguishes `Codec`, `Json`, `Scalar`, `Bytes`, `NoContent`,
  `Record`, `Array`, `Part`, `Stream` and `Union`. `NativeRecord` retains field wire/
  native names, requiredness, additional-field types and the actual media/header
  structural binding. The compatibility adapter consumes these descriptors.

Protocol tests are in `tests/ruby_sdk.rs` and `src/ruby_sdk/tests/protocol*`,
`streaming.rb`, and `openrouter_protocol.rb`. Extra actual OpenRouter tests
cover `downloadContainerFileContent`, `downloadFileContent` and `deleteKeys`, with
all 256 octet values supplied by the independent byte fixture. The two download
operations require the explicit legacy-binary compatibility profile in their
unchanged tracked source.

## Package overview

The `ruby-sdk` feature enables `suspect_codegen::ruby_sdk`. Its complete package
root is **`ruby/`**: keyword models and calls, checked codecs, native exceptions,
an injectable HTTP client, gem metadata, YARD documentation, RBS signatures and
executable source-derived examples.

## Everyday use

Package identity is configurable. For a package generated as `ruby_native_gate`
with namespace `RubyNativeGate`, the five-operation OpenRouter selection exposes:

```ruby
require 'ruby_native_gate'

RubyNativeGate::Client.open(auth: {'apiKey' => supplied_token}) do |client|
  credits = client.get_credits
  puts credits.data.data.total_credits.token

  request = RubyNativeGate::Models::CreateKeysRequest.new(name: 'Native Test Key')
  created = client.create_keys(body: request)
  puts created.status

  patch = RubyNativeGate::Models::UpdateKeysRequest.new(
    limit: RubyNativeGate::JsonNumber.new('75.50'),
    limit_reset: nil
  )
  client.update_keys(hash_value: 'fixture-hash', body: patch)
end
```

`hash_value` is the allocated Ruby name of wire parameter `hash`, preserving
Ruby's inherited `hash` method. The outer `response.data` is the SDK result; the
inner `data` is a real source-declared JSON member. Methods without API inputs
take no empty input object. A single declared success returns its concrete
response class directly.

Model names come from component names or operation roles (`CreateKeysRequest`,
`UpdateKeysRequest`). Full source pointers stay in metadata. The generated README
leads with a complete native-model request. `examples/quickstart.rb` contains the
same request and supports injected transport; the offline example runner executes
it alongside the complete source-example set.

## Presence, models and exact values

- Required fields are required constructor keywords. Optional fields use the
  singleton `UNSET`; `nil` means JSON null and must satisfy the source schema.
- JSON Schema defaults are never inserted. Required singleton tags have checked
  constructor conveniences, for example `StandardPayload.new(text: 'plain')`.
- Object union arms are ordinary generated classes. Scalar/literal arms use
  native Ruby values. References retain both their own source-bound codec and
  the terminal native carrier.
- Objects retain closed, open or typed additional-property behavior through
  `extra_fields`. Arrays retain positional-prefix and item validation.
- Fields and collections are mutable. Constructors and **every encode** validate
  the complete source program, including nested mutations. Both the selected
  native union arm and its parent must accept the value.
- All decoded numbers use immutable `JsonNumber` tokens. Numeric inputs also
  accept Ruby integers within the token ceiling. Floats are rejected rather than
  silently rounded. Exponents remain symbolic; padded exponent spellings,
  integrality, comparison, equality and divisibility use exact arithmetic.
- `to_i(max_digits:)` is an explicit bounded expansion. `to_f` is explicitly lossy.
  The JSON domain distinguishes numeric values from booleans, validates UTF-8 and
  Unicode escapes, and rejects duplicate decoded keys and cyclic containers.

RBS expresses native kinds, nullable/optional fields, literal tags, union carriers,
operation arguments and concrete results. The portable runtime enforces bounds,
patterns, mathematical integrality, tuple constraints and exclusive unions.
In-place recursive validation cycles retain evaluation-failure behavior; their
uninhabited signature aliases use RBS bottom. Productive object recursion keeps
normal model references.

## Authentication, failures and transport

Auth is an explicit string-keyed hash using the **declared security-scheme name**.
The demonstrated OpenRouter `apiKey` scheme declares HTTP bearer authentication,
so it produces an `Authorization: Bearer …` header. Without a policy the SDK does
not read environment credentials or acquire/refresh tokens; environment
credentials are opt-in through `credential_env` or `sdk_defaults.env_prefix`
([ADR 0026](adr/0026-environment-credentials-are-opt-in.md)), and token lifecycle
and pagination helpers through `sdk_defaults`
([ADR 0011](adr/0011-behavior-comes-only-from-the-spec.md)). Source descriptions,
including management-key requirements, remain documentation rather than invented
key types.

Declared, validated API errors have public operation/status-specific classes,
such as `CreateWidgetStatus422 < CreateWidgetApiError`. `RequestError`,
`ResponseError`, `TransportError`, `TimeoutError`, `CancelledError` and
`ResourceLimitError` identify other failure categories. Original Ruby causes and
source/instance locations remain available. Messages and inspect output omit
credentials and payloads; bounded captures are explicit attributes.

A borrowed transport implements:

```ruby
def exchange(request:, context:)
  context.check!
  yield WireResponse.new(status: 200, headers: headers, body: chunks)
end
```

It yields exactly one response. Client consumes bounded chunks and closes the
response on every exit, before returning decoded data. `Client.open` closes its
owned adapter; injected transports remain caller-owned. Calls after close fail
before dispatch.

The default adapter uses Net::HTTP, verifies TLS peers and explicitly disables
ambient proxies, redirects, decompression and automatic retries. Each exchange
owns one connection. Its per-connection bounded line reader prevents unbounded
header/chunk-framing buffering before response dispatch. HTTP framing, body,
header, URL, chunk-count, JSON depth/work, conversion, equality and validation
budgets are finite. Logical trials share their budgets and cannot swallow
evaluation failure.

`timeout:` is a total monotonic deadline covering encoding, HTTP, consumption and
decoding (default 30 seconds; positive and at most 86400). `CancellationToken`
interrupts blocking Ruby I/O. Custom native extensions must permit Ruby thread
interrupts during blocking operations. Per-call response/capture settings can
only lower client ceilings.

## Refusals

Other dialects, active directional annotations, unsupported instructions,
unproved structural intersections, unsupported media/security/parameter shapes
and invalid package identities fail with located diagnostics before artifacts.
Runtime URI handling explicitly rejects unsafe spellings and dot-segment path
arguments. Runtime server overrides accept HTTPS or loopback HTTP unless
`allow_insecure_http: true` is passed; declared servers keep their scheme
([ADR 0035](adr/0035-one-server-url-policy.md)). No retry or
business behavior is inferred from operation names.

## Native verification

Tests: `crates/suspect-codegen/tests/ruby_sdk.rs`, behind `ruby-sdk`.
Handwritten consumers live under `crates/suspect-codegen/src/ruby_sdk/tests/`.
Each native gate emits a fresh package, builds a real `.gem`, installs locally,
and runs against the installed bytes. Package source is never repaired by tests.

Toolchains installed without activation:

- Ruby 3.3.12: `~/.local/share/mise/installs/ruby/3.3.12`
- Ruby 4.0.6: `~/.local/share/mise/installs/ruby/4.0.6`
- Isolated tools (the tests' default `SUSPECT_RUBY_GEMS`):
  `target/sdk-ruby-tools/gems`, and `target/sdk-ruby-tools/gems-ruby4` for Ruby 4
- YARD 0.9.37, Redcarpet 3.6.1, RBS 3.9.5, Steep 1.10.0.

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
  cargo test --locked -p suspect-codegen --features ruby-sdk \
  --test ruby --target-dir target/sdk-ruby-cargo -- \
  --include-ignored --test-threads=1

SUSPECT_RUBY_HOME="$HOME/.local/share/mise/installs/ruby/4.0.6" \
SUSPECT_RUBY_GEMS="$PWD/target/sdk-ruby-tools/gems-ruby4" \
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
  cargo test --locked -p suspect-codegen --features ruby-sdk \
  --test ruby --target-dir target/sdk-ruby-cargo -- \
  --include-ignored --test-threads=1
```

The matrix covers four baseline operations; `getCredits`, `createKeys`,
`updateKeys`, `listContainerFiles`, `getContainerFile` from the actual tracked
OpenRouter source; all 17 shared `runtime-contract-v1.json` schemas; seven negative
Steep consumers; exact-value/presence/mutation/recursion/name/prose adversaries;
shared branch/copy/equality/numeric budgets; and real/injected transport security,
framing, timeout, cancellation and cleanup failures. OpenRouter wire responses
come from the hand-authored `openrouter-five-responses.json` fixture.

## Rust adapter accessors

```rust,ignore
let plan = ruby_sdk::plan_sdk(contract, &selected_sources, RubyConfig::default())?;
let files = ruby_sdk::emit_sdk(&plan, &PackageConfig {
    name: "example-sdk".into(),
    version: "0.1.0".into(),
    require_name: "example_sdk".into(),
    namespace: "ExampleSDK".into(),
})?;
```

The plan exposes immutable borrowed data:

- `SdkPlan::{contract, config, program, models, operations, examples,
  native_examples, schema_source}`. `render()` uses the default package identity.
- `ModelPlan::symbols()` yields `&ModelSymbol`; `symbol(schema_index)` and
  `source_symbol(&SchemaId)` find exact carriers; `carrier(index)` resolves aliases.
- `ModelSymbol::{source, schema_index, name, type_name, signature_uninhabited,
  description, shape}`. `name` identifies the codec constant and, for object
  shapes, the class under `Models`; `type_name` is the RBS alias under `Types`.
- `ModelShape` retains `Json`, `RefinedJson`, exact `Literal` values, `Never`,
  `Scalar`, `Alias`, `Object`, `Array` and `Union`. Object fields retain native/wire
  names, source, index, requiredness and singleton literal; extras are
  `Closed`, `Json` or `Typed(index)`. Unions retain ordered branches/exclusivity.
- `PlannedOperation` retains source, operation ID, native method, wire method/path,
  server, description, all security identities, parameters, body, responses and
  its public API-error class. Parameter fields retain keyword/wire name/source/
  requiredness/index; `schema()`, `location()`, `style()`, `explode()` and `array()`
  expose the typed wire descriptor.
- `PlannedBody` and `PlannedResponse` retain source, terminal media source, media
  type and schema index; bodies retain requiredness; responses retain exact status
  and native class name.
- `NativeExample` retains operation source, entry index, schema index and a typed
  `SampleValue` construction tree. Public names, docs and samples use these plans;
  compatibility adapters need not inspect emitted Ruby text.
