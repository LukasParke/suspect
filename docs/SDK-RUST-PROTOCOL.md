# Expanded Rust HTTP protocol SDK

`rust_http::plan_http(Arc<Contract>, &[SourceId], HttpConfig)` now consumes
`http_protocol::ProtocolPlan` directly. The native profile is
**`rust-http-protocol-v1`**. Its explicitly enumerated capabilities are published
by `rust_http::native_capabilities()`; new shared capabilities are never enabled
automatically.

This supersedes the strict HTTP admission section of [SDK-RUST-HTTP.md](SDK-RUST-HTTP.md).
The existing Rust models, exact JSON codecs, owned validation, package identity,
and artifact ownership pipeline supply the native implementation. Examples use
`examples::plan_protocol_examples` and its v2 roles over the same admitted plan.

## Public plan API

```rust,ignore
use suspect_codegen::rust_http::{self, HttpConfig, PackageConfig};

let plan = rust_http::plan_http(contract.clone(), &selected_sources, HttpConfig::default())?;
for operation in plan.operations() {
    let source = &operation.source; // Actual indexed operation SourceId.
    let mount = operation.wire().path_item();
    let parameters = operation.parameters();
    let responses = operation.responses();
    let native_bindings = operation.interface();
}
let files = rust_http::emit_http(&plan, &PackageConfig {
    name: "example-sdk".into(),
    version: "0.0.0".into(),
})?;
```

| API | Meaning |
| --- | --- |
| `HttpPlan::protocol()` | The admitted shared descriptors, capability/profile list, real codec roots, and located warnings. |
| `HttpPlan::symbols()` / `codecs()` | Actual `SchemaId` → native model names and the existing source-bound codec plan. |
| `HttpPlan::credentials()` | Source-addressed scheme constructors. |
| `PlannedOperation::wire()` | Method token, mount/provenance, servers, security, parameters, bodies, responses, and annotations. |
| `PlannedOperation::{parameters,body,responses,credentials}()` | Native allocated bindings alongside their immutable shared wire descriptors. |
| `PlannedOperation::default_function_name` | Allocated helper when all input members are optional. |
| `PlannedOperation::interface()` | Structured native constructor, payload, header, credential-constructor, and response-union bindings for compatibility capture. |
| `Payload` | Model with its original schema ID, exact JSON, text, bytes, no content, typed stream items, or native structural parts. |
| `PlannedAggregate` / `PlannedPart` | Native field/type names, multiplicity, declared headers, and actual per-part/item codec identities. |
| `rust_http::source_assets()` | Complete compile-time Rust HTTP plan/emitter/runtime asset closure for provenance. |

An operation mounted through a Path Item reference keeps its actual indexed
operation in `PlannedOperation::source`; the shared provenance retains both the
mount and definition. JSON Schema `$ref` use-site roots remain codec inputs, so
assertion siblings are preserved. Byte schemas and mixed multipart aggregates
are not JSON codec roots.

Rust compatibility capture uses `operation.interface()` directly. It includes
allocated credential constructors, body/media choices, typed headers and part
structures, stream payloads, no-input helpers, and sole-success accessors. Its
runtime fingerprint includes every new Rust HTTP module via `source_assets`,
in addition to the common/model/codec provenance already recorded.
Source addresses, byte spans and prose stay in the typed wire plan and manifest;
they do not manufacture native signature changes after source relocation or
documentation edits.

## Native use

Generated packages retain empty default features:

| Feature | Dependencies and use |
| --- | --- |
| Default | Models, exact JSON, validation, codecs; no required dependencies. |
| `http` | Native async `Transport` / `ResponseBody` seam and pinned `url`. |
| `reqwest-rustls` | Pinned reqwest 0.12.28 with rustls; caller-owned Tokio runtime. |
| `serde-json` | Existing optional exact serde_json codec integration. |

```rust,no_run
use openrouter_sdk::{Client, Credentials};

async fn credits(token: String) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::with_reqwest(Credentials::api_key(token))?;
    let response = client.get_credits_default().await?.into_response();
    println!("{}", response.data.data.total_credits.as_str());
    Ok(())
}
```

The original `get_credits(GetCredits::new())` call and `Status200` match remain
valid. No-input `_default` helpers omit all optional inputs. A sole-success enum
implements `Deref<Target = ApiResponse<...>>`, `into_response()`, and `into_data()`;
multiple-success operations keep explicit variants. API and SDK errors retain
their boxed native enum payloads.

Generated Rustdoc includes a compile-checked generic callsite and a
reqwest-gated **native first-request constructor** sample for each operation.
Validated model examples are lowered from typed model declarations. Finite parts
use native aggregate and `Part::new` / `Part::with_headers` constructors; file
bytes are caller parameters. Required inputs that lack a verified construction
recipe remain explicitly typed caller parameters. `examples/validated.rs`
executes the actual JSON/text/header/part/item codecs; v2 example diagnostics
retain byte/framing recipe obligations without fabricating JSON values.

## Implemented execution

### Methods, servers, and credentials

- Standard methods, OAS 3.2 QUERY, and case-preserved `additionalOperations`
  tokens reach the native transport. Lowercase `head` is distinct from HEAD.
- Ordered server candidates, defaults, variable enums/overrides, and relative
  document bases are implemented. `ClientOptions::server_index`,
  `server_variables`, and `document_url` select them. Local-file descriptions
  need an explicit HTTP serving-document URL for relative servers. An absolute
  `server_url` override is exclusive with candidate/variable selection.
- Source-declared HTTP servers are supported. Absolute HTTP overrides remain
  loopback-only, preserving the previous native override policy. URL userinfo,
  query/fragment injection, malformed percent escapes, and route-changing
  normalization fail before transport.
- Undeclared/disabled security, anonymous alternatives, OR alternatives and AND
  members are implemented. The first fully supplied alternative is selected,
  or callers use `Credentials::select_alternative(index)`.
- Bearer, UTF-8 basic credentials, and API keys in headers/query/cookies have
  explicit attachments. Query keys are URI encoded. Cookie keys preserve valid
  cookie octets; values needing escaping must be supplied already escaped.
  Generated scheme constructors bind the original scheme source. Named setters
  express caller policy; `bearer_token` is a lookup of those named entries.
- OAuth/OIDC accept a complete caller-selected Authorization value or a
  `CredentialProvider`. Hooks receive located requirements, scopes versus roles,
  reference provenance, flow endpoints, OAuth metadata and OIDC discovery URLs.
  They perform no implicit discovery, acquisition, refresh or retries. Conflicting
  credential/parameter attachments fail explicitly.

### Parameters and responses

Simple, label, matrix, form, space-delimited, pipe-delimited, deep-object and 3.2
cookie styles support their admitted scalar, scalar-array and flat-object
shapes. Native codecs validate each value first. UTF-8 escaping, exact numeric
tokens, JSON/text content parameters, reserved expansion, unquoted header
values, and cookie escaping follow the descriptors. Required empty composites
fail representation; optional empty style composites omit. Ambiguous active
delimiters and header/cookie controls are rejected.

Complete 3.2 querystrings support JSON, UTF-8 strings, and form-urlencoded
objects. Their source parameter name is not emitted. Form querystrings use the
actual complete-object codec plus the planned field strategies and receive no
second URI-encoding pass. Ordinary query parameters and query credentials
cannot be merged into a complete querystring implicitly.

Response selection is **exact status → class range → default**, followed by
media selection. A media mismatch never falls through to another status rule.
Actual 200–299 status determines success, including default responses. Absence
of `responses` is admitted as an entirely undeclared response surface.

Concrete JSON/+json, UTF-8 text scalars, opaque bytes, type/any media ranges,
and structured media parameters have native bindings. Concrete media outrank
type ranges and `*/*`, with declared parameter count breaking specificity ties.
Missing, duplicated or malformed Content-Type cannot select declared content.
HEAD/1xx/204/205/304 bodies are dropped without polling or decoding. A response
with no declared content otherwise retains bounded bytes.

`ApiResponse<T, H>` exposes actual status, source-selected and actual media,
raw duplicate header bytes, `typed_headers`, links, and data. Required headers
are enforced through their bound codecs. Simple non-string scalar, array and
flat-object headers and JSON/text content headers are decoded. Repeated scalar
fields, notably Set-Cookie, are an explicit typed-decoding failure; raw fields
remain available in the bounded error. Links are immutable source metadata and
cause no further calls.

### Forms, multipart, and streams

Forms and named/positional multipart have native aggregate structs. Byte parts
are `Part<Vec<u8>, H>`; text/JSON parts use their actual bound native models.
Repeated properties use per-item values/codecs. Required fields, permitted
extras, property counts, repeated-item cardinalities and positional order are
checked structurally. Required part headers are encoded/decoded through their
own codecs. Boundary selection checks collisions; MIME framing, filenames and
Content-Disposition names are validated. Preamble/epilogue are ignored on input.

Encoding `style`, `explode`, and `allowReserved` select RFC6570 behavior only for
form-urlencoded and `multipart/form-data`. Other multipart media retain their
explicit/default JSON, text or byte content codecs and report each ignored field
at its source. Positional arrays remain one JSON part when their encoding's
`explode` is ignored. Active positional form-data styling retains Rust's located
`http-rust-positional-style-unsupported` refusal.

OAS 3.2 SSE and JSON lines use **`itemSchema`**. Responses return
`ItemStream<Model>` with `next().await -> Option<Result<Model, SdkError>>`,
`close()`, and `is_closed()`. They retain one bounded chunk and one bounded
item, pull no later chunk until needed, and release the body on completion,
error, close or drop. A pending operation owns its response body as well.

SSE applies UTF-8/CR/LF/BOM/comment/field framing, joins multiline data, ignores
unknown fields and invalid retry/ID values, and supplies the parsed event
envelope to the source codec. `data`, `id`, and `event` remain strings; `retry`
is an exact nonnegative integer. `[DONE]` is ordinary data. JSON inside `data`,
contentSchema evaluation, reconnection and Last-Event-ID request policy are not
inferred. JSON lines have no sentinel or record-separator shortcut. Finite
request streams use `Vec<Model>` and the same source item codecs.

## Resource and compatibility policy

An explicit `HttpConfig::credential_env` policy adds creation-time environment
factories while retaining the existing explicit credential constructors. See
[SDK-RUST-CREDENTIAL-ENV.md](SDK-RUST-CREDENTIAL-ENV.md) for the compiled helper
signatures, source binding, native proofs and no-policy artifact checkpoint.

`HttpConfig` retains its codec, request and response policy and adds
`max_part_bytes`, `max_stream_item_bytes`, `max_chunk_bytes`,
`max_header_bytes`, and `compatibility_profiles`. Use `..Default::default()`
when specifying individual overrides. Defaults are 8 MiB request/response/part/
chunk, 1 MiB stream item, and 64 KiB headers. Generation limits are positive and
portable to 32-bit `usize`; caller limits can only lower them.

The runtime counts actual response bytes independently of Content-Length,
checks chunks before retaining/copying them, bounds error capture and body
polls, checks byte parts before copying, and proves multipart assembly size
before allocating the final body. Exact JSON/evaluation/conversion limits remain
independent. A custom transport bounds its own initial chunk/header allocation;
these policies are not complete allocator or CPU accounting.

The reqwest adapter retains disabled redirects, retries, ambient proxy routing,
referer generation and automatic decompression. It creates no SDK task or
executor. Request debug hides URL/header/body values; SDK error display/debug
hides credentials and captures while preserving explicit causes and metadata.

`CompatibilityProfile::LegacyBinaryStringV1` is an explicit opt-in for legacy
3.1/3.2 `type: string, format: binary` byte markers. Ordinary 3.1+ byte schemas are
unconstrained; standard 3.0 binary markers are supported directly.

The shared planner's refusals remain in force: whole-stream plus item assertions,
3.1 SSE schema-as-item conventions, streamed/nested multipart, unsupported
transfer/content encodings, mixed per-part representations, untyped form extras,
undefined parameter combinations and ambiguous media maps. Rust additionally
declines positional RFC6570 part expansion with no variable name. Neutral Rust
codecs still decline active readOnly/writeOnly projections and unsupported
native schema representations. Unknown extensions remain located annotations.

## Native gates and corpus coverage

The native acceptance seam is the public plan/emitter API → real Cargo package
→ installed consumer. `tests/rust_protocol.rs` supplies independently authored
wire/status/media/part/stream failures, the shared literal parameter vectors,
native positive/negative typing, model-only/rootless packages, executable
examples, Rustdoc, raw TCP requests, and actual reqwest stream cancellation.

`tests/rust_http_openrouter.rs` retains the original five-operation installed
consumer: `getCredits`, `createKeys`, `updateKeys`, `listContainerFiles`, and
`getContainerFile`. Its expanded installed TCP consumer uses the **unmodified
tracked source** for:

- `downloadContainerFileContent` and `downloadFileContent`, with the explicit
  legacy binary profile and byte-for-byte zero/non-UTF-8 responses;
- `createCoinbaseCharge`, preserving its source `security: []`, no-input call,
  and typed 410 response;
- `deleteFile`, preserving DELETE and the source's negotiated response union.

The actual `uploadFile` remains declined because its multipart aggregate leaves
additional properties untyped. Its filename-like example is not treated as file
bytes. Normative fixtures independently exercise native uploads and part codecs.

Run the Rust gates on the current toolchain and then with native Cargo 1.88:

```sh
cargo test --locked --offline -p suspect-codegen \
  --test rust_protocol --test rust_http --test rust_http_runtime \
  --test rust_http_security --test rust_http_docs --test rust_http_openrouter \
  --no-fail-fast -- --include-ignored --test-threads=1

SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0 cargo test --locked --offline -p suspect-codegen \
  --test rust_protocol --test rust_http --test rust_http_runtime \
  --test rust_http_security --test rust_http_docs --test rust_http_openrouter \
  --no-fail-fast -- --include-ignored --test-threads=1
```

During parallel adapter migration, the Rust modules and these same public-seam
tests can be compiled in an isolated source harness. That checks the actual Rust
plan, codecs, owned writer and runtime while other adapters are mid-edit.

### Verified checkpoint — 2026-09-10

| Gate | Result |
| --- | --- |
| Full Rust integration batch through the isolated canonical-module harness | **17 passed**, no ignored tests, on Rust **1.97.1** and native Cargo/Rust **1.88.0**. |
| Final expanded installed consumer, including multipart RFC6570 item decoding, header whitespace preservation and stream cleanup | **Passed on both toolchains**, including positive/negative consumers, private `.crate` installation, Rustdoc, executable examples and real reqwest sockets. |
| Original five-operation OpenRouter installed consumer | **Passed on both toolchains**. |
| Four additional actual OpenRouter operations with explicit legacy-byte profile | **Passed on both toolchains**; `uploadFile` retains its located untyped-extras refusal. |
| Integrated default-feature `cargo check -p suspect-codegen` | **Passed**; remaining dead-code warnings are in other adapters/the retired strict module. |
| Integrated `tests/rust_protocol.rs` ordinary tests | **5 passed**; the two native gates were explicitly run above. Includes the additional docs/relocation-stable native-interface regression. |
| Integrated native Rustdoc manifest/prose gate | **Passed** after compatibility projection changes. |
| Integrated Rust CLI ownership, refusal and native all-feature Cargo checks | **3 passed**, no ignored tests. |
| Integrated wire-name/native-signature compatibility regression | **Passed**. |
| Integrated multi-language compatibility sweep | **26 passed, 3 Go-related failures**: docs-only/relocated body projections and the old Go response-status descriptor expectation. |
| Integrated generation-session sweep | **7 passed, 1 failed**: unresolved example-reference admission expectation. |
| Rust-owned file formatting | **Passed**. |

The joint M2 vertical gate reaches a **TypeScript TypeDoc blocker before its Rust
stage**: `ServerChoice`, `CredentialRequirement`, `Located`, `ServerPlan`,
`Provenance`, `CredentialValue`, and `SourceLocation` are referenced but absent
from that documentation export inventory. The retained log is
`sh_08c8c1e17002mVhO4dizJ73g8T.out` in the session's OpenCode shell output directory.
Rust's existing native operation/type/wire/resource/cancellation gates and both
OpenRouter installed-consumer groups passed separately.

The broader integration sweep also identified the Go native-interface
source/span projection and an old generation-session expectation for an
unresolved external example reference. Those are Main/other-adapter follow-ups;
the latter now receives located `http-reference-unresolved` admission at
`examples/external/$ref`. Rust native interface snapshots no longer include
prose, source offsets or retrieval locations in signature comparisons.

## Physical-document and native v3 adoption — 2026-09-10

`DocumentRelativeServers` is now separately witnessed on Rust **1.97.1** and
**1.88.0**. Relative servers use the shared `ServerPlan::document_base()` emitted
as native `Server::document_base`, including effective retrieval URLs after
redirects. Requested aliases and logical `$self`/`$id` names remain metadata.

- Absent entry `servers` defaults to `/` at the entry document, including for
  referenced operations. Explicit empty overrides belong to their declaration's
  physical document. `ClientOptions::document_url` takes precedence.
- Local-file declarations require an explicit HTTP document URL or absolute
  server override. Physical sources remain the diagnostic identities.
- Actual reqwest requests preserve encoded slashes such as `%2f` and ordinary
  relative dot navigation. The pinned URL transport normalizes encoded dot
  segments, so Rust explicitly refuses `%2e`, `%2e%2e`, `.%2e`, and `%2e.`
  (case-insensitively) before parsing/joining a server or document-base URL.
- Native `Provenance` includes `use_site_resource`, `terminal_resource`, and
  aligned `reference_resources`. `Operation::provenance` exposes mount/terminal
  context. `ResourceContext` keeps canonical/base URI names, aliases, schema
  roots and physical resource/source identities separate.
- `Server::url_base()` is `ApiUrlBase::ServerDocument`; credential-kind and OAuth
  flow URL-base accessors expose `EffectiveServer`. OAuth/OIDC endpoints remain
  raw located metadata and cause no acquisition.

Exact native witness:

```sh
cargo test --locked --offline -p suspect-codegen --test rust_protocol_resources \
  installed_rust_relative_servers_use_effective_physical_documents \
  -- --exact --ignored --nocapture
```

The maintained fixture uses two distinct loopback origins, requested/effective/
logical URI separation, referenced operations, explicit/absent/empty servers,
local-file refusal/override, OAuth/OIDC metadata, and pre-transport normalization
controls. It packages and installs the SDK before its real socket requests.

The additive **`rust_http::plan_http_v3`** selects resource/dynamic-capable codecs
and `examples::plan_protocol_examples_v3` when the selected inputs require them.
Ordinary inputs preserve the complete v2 HTTP artifact set, including manifest
adapter/capabilities and examples. The resource capability profile is
**`rust-http-resources-v3`**, exposed by `native_capabilities_v3()`.
`SchemaResources` and `DynamicSchemaReferences` are enabled only there, after
the checked native/installed SDK witnesses. The existing v1/v2 entry points keep
their resource fences. Ordinary codec closures retain their established v1/v2
programs. Full semantics, production assets and exact selectors are in
[SDK-RUST-RESOURCES.md](SDK-RUST-RESOURCES.md).

### Focused workspace integration repair — ignored multipart styling

The workspace preflight exposed an obsolete refusal in
`rust_protocol::legacy_byte_profiles_and_unimplemented_conventions_are_explicit`:
`multipart/mixed` correctly yields content codecs and an ignored-field warning.
The repaired host test checks those native bindings, exact codec roots and warning
provenance, compares the native interface with the previously witnessed content
plan, and retains the actual positional form-data refusal with its mandatory
disposition headers. Explicit legacy-binary opt-in and unsupported stream controls
remain enforced. Malformed ignored fields and incompatible content codecs have
additional physical-source/span assertions.

The changed host selector passes **1 test**. The new focused native selector
`installed_native_multipart_ignored_styles_preserve_content` passes on Rust
**1.97.1 and 1.88.0**, with **two installed consumer tests and six Rustdoc tests**
per toolchain. It uses the unmodified shared `ignoredMultipartEncoding` source,
the real generated operation and a literal native `Transport`. An independent MIME
oracle verifies explicit JSON string content, default integer text, unsplit JSON
arrays, opaque bytes, JSON itemEncoding and required part headers in both
directions. Incompatible request media fail before transport; an invalid response
retains its actual schema source.

```sh
cargo test --locked --offline -p suspect-codegen --test rust_protocol \
  legacy_byte_profiles_and_unimplemented_conventions_are_explicit -- --exact

cargo test --locked --offline -p suspect-codegen --test rust_protocol \
  installed_native_multipart_ignored_styles_preserve_content \
  -- --exact --ignored --nocapture
```

Use `SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0` for the floor and
`SUSPECT_RUST_IGNORED_MULTIPART_TARGET` for an isolated native cache. Focused Clippy
with warnings denied and rustfmt pass. This repair changes the owned test and
documentation; it needs no production/capability change or unaffected matrix replay.
Evidence, red/green logs and retained command logs are indexed in
`target/sdk-rust-resources-20260910-01/ignored-multipart-integration.json`, SHA-256
`b648979ce6cbfe18ac668411dfec33d9e65b1d6228e55ff7ad570d93ee867c19`.
