# TypeScript / JavaScript HTTP protocol adapter

The TypeScript adapter consumes the admitted, immutable
`http_protocol::ProtocolPlan` and the existing native model/codec plan. It does
not rerun `http_contract::plan`, reconstruct schemas from generated code, acquire
credentials, or infer vendor streaming conventions.

## Profiles and entry point

```rust,ignore
use suspect_codegen::typescript::{http, package};

let plan = http::plan_http(contract, &selected, http::HttpConfig::expanded())?;
let files = package::emit_http(&plan, &package::PackageConfig {
    name: "@example/client".into(),
    version: "0.1.0".into(),
})?;
```

`HttpConfig::default()` retains the **StrictJson** admission profile. It uses the
same verified protocol runtime as **ExpandedV1**, which is explicitly selected
with `HttpConfig::expanded()`. The expanded capability list is enumerated in the
adapter; it does not use `Capability::ALL`.
The canonical backend selects ExpandedV1 through
`backend::typescript_options(&GenerationOptions)`, shared by generation and
native compatibility capture. `with_profile` and `with_compatibility_profile`
are public adapter setters; the library's StrictJson default is preserved.

The OAS 3.1+ legacy string/binary marker is separate:
`HttpConfig { legacy_binary_string: true, ..HttpConfig::expanded() }` enables
`LegacyBinaryStringV1` only in byte contexts. It is off by default. Ordinary JSON
media continue to use the source JSON model codec.

## Native API

The original five OpenRouter operation exports, model/codec identities, plain
body literals, exact numbers and operation-specific error guards are preserved.
Functions and bound methods can omit the input object when all inputs are
optional. An anonymous client can be constructed with `createClient()`.

```ts
const client = createClient({ auth: { apiKey: token } });
const credits = await client.getCredits();
await client.createKeys({ body: { name: "CI" } });
await client.updateKeys({ hash: key, body: { limit: null } });
```

Single-media bodies are native values. Multi-media bodies explicitly select a
concrete Content-Type. Wildcards have an additional source-range discriminator:

```ts
await client.sendMedia({
  body: { contentType: "application/json", data: { name: "native" } },
});
await client.sendMedia({
  body: {
    mediaType: "application/*",
    contentType: "application/pdf",
    data: new Uint8Array([0, 255, 128]),
  },
});
```

These second examples are from the normative synthetic fixture, not operations
claimed to exist in OpenRouter. The `mediaType` discriminator prevents wildcard
branches from weakening concrete request typing and supports response-union
narrowing. Runtime matching independently prevents bypassing a more-specific
codec. `rawContentType` retains the complete received field.

Response `headers` retains Fetch's raw Headers. `typedHeaders` enforces declared
requiredness and scalar/array/flat-object codecs. `links` and `operationMetadata`
are inert, source-backed metadata. Status selection is exact > range > default;
success uses the actual status. Unspecified content is bounded Uint8Array data;
HEAD and body-forbidden statuses suppress decoding.
Fixed bodyless statuses are separate native union members, so excluding both
204 and 205 narrows default/range success data to its declared body type. Required
typed headers remain available and validated on bodyless responses.

### Credentials and servers

An explicitly configured [credential-env v1 policy](credential-env.md)
adds creation-time defaults to `createClient()` / `createClient({})`. Only an
omitted own auth property enables lookup. Unconfigured output retains its original
bytes and constructor behavior; explicit auth is never supplemented from env.

- Source alternatives are OR and member schemes are AND. The first complete
  alternative is used unless `call.securityAlternative` explicitly selects one.
- Bearer and API keys use strings. Basic uses `{username, password}`; non-ASCII
  credentials require explicit `encoding: "utf-8" | "latin1"`.
- OAuth2/OIDC uses a complete `{authorization: "Scheme credential"}` or a
  caller callback returning it. Callbacks receive source/flow/discovery metadata,
  located scopes versus roles, and AbortSignal. Without an `sdk_defaults`
  OAuth policy ([golden defaults](../../guide/sdk-generation.md#5-oauth-lifecycle))
  there is no acquisition, discovery, refresh, fallback retry, or inferred
  Bearer token type.
- `server: {index}` or `{name}` selects a declared server. `variables` validates
  defaults, names and enums. Relative local-file servers need an explicit HTTP
  `documentURL` or `serverURL`. Per-call server choices override client choices.
- Relative server defaults use the effective physical document containing the
  Server Object (`document_base`), independently of logical `$self`/`$id` names.
  Absent arrays default from the entry document; empty overrides use their own
  document. Native Fetch URL rewrites fail before sending; capable explicit
  transports retain encoded dot/slash spelling. Credential callbacks receive
  `effectiveServerURL` for relative OAuth/OIDC endpoint metadata.

### Parts and streams

Forms and named multipart retain required fields, extras policy and cardinality.
Typed extra values use an `additionalFields` bag, qualified when necessary.
Repeated parts use per-item codecs. Binary aggregates never enter a JSON codec
with placeholder nulls. Multipart values needing metadata use
`{data, headers, contentType, filename}`; multiple content choices require an
explicit contentType. Raw byte parts use the filename `blob`. Filenames cause no
filesystem access. Positional multipart preserves prefix/item codecs and order.

RFC6570-style **named multipart** maps to physical MIME fields. Scalar values
have one named part; exploded arrays repeat the name; exploded objects use their
property names; deepObject uses literal bracketed part names; joined arrays and
objects use literal comma/space/pipe delimiters. URI percent-encoding and
`name=value` query-text payloads are not applied to MIME fields. OAS 3.1 retains
whole-property style codecs; OAS 3.2 applies encoding to each outer-array item.
The inverse mapping validates every item and required header. Conflicting
physical names, ambiguous active delimiters, inconsistent metadata across an
expanded logical value, framing injection and absent required fields fail.

OAS 3.2 whole-query JSON/text/form content has no parameter-name prefix. JSON and
text receive one component-encoding pass; form output is not encoded a second
time. Form query inputs use the actual aggregate JSON model plus per-field
codecs. Ordinary query slots, duplicate whole-query slots and query API-key
attachment cannot be merged into that complete content.

Custom HTTP method tokens retain their exact case and spelling. Native Fetch
normalization of tokens such as `get`, `GeT` and `head` is refused before sending;
a capable explicit transport receives the original token. Lowercase `head` is
not treated as HEAD, and 205 responses suppress content alongside other
HTTP-body-forbidden statuses.

OAS 3.2 itemSchema produces native AsyncIterable SSE/JSON Lines response data.
SSE data remains a string, including JSON-looking text and `[DONE]`; retry is an
integer. Comments, unknown fields, CR/LF framing, multiline data, split UTF-8,
invalid retry/id values and incomplete EOF events follow the framing rules.
JSON Lines has no sentinel shortcut. Request sequences accept Iterable or
AsyncIterable and are bounded/validated before HTTP transmission.

Iterators are pull-driven. Abort, early return, EOF, codec failure and resource
failure cancel/unlock readers and remove caller listeners. Consumers must consume,
return/break, or abort an unused stream. Late responses from an uncooperative
transport are cancelled too.

### Finite policy and explicit refusals

Generated defaults are 8 MiB per request/response, 1 MiB per part, 64 KiB per
stream item, 128 KiB per retained transport chunk, and 100,000 stream items.
Caller limits can only reduce them. Multipart also limits physical part count
(10,000), part header count (100), and part header bytes (64 KiB).

Native Fetch cannot send TRACE or browser Cookie headers. TRACE requires a
capable explicit Fetch-shaped transport; browser Cookie is refused before
transport. CORS/Fetch limits response-header visibility. Required unavailable
headers fail decoding. Repeated Set-Cookie cannot be folded into a scalar header.

Expanded positional values or repeated composite items that would require
invented physical item-group boundaries return
`http-typescript-multipart-style-grouping` at the Encoding Object. Named scalar,
array and flat-object physical expansion is implemented. Core refusals for ambiguous styles,
nested/streamed multipart, transfer encodings, JSON sequences and legacy/vendor
stream semantics remain source-located. This is not blanket full-protocol support.

## Typed plans and compatibility integration

- `HttpPlan::protocol()` → the complete admitted ProtocolPlan.
- `PlannedOperation::protocol()` → complete source operation descriptors.
- `PlannedParameter::protocol()` → original serialization and codec references.
- `PlannedOperation::interface()` → actual native signatures, media/body/part,
  typed-header/stream/result and credential types, constructed by the same typed
  emitter helpers. Prose, document locations and spans are excluded; opaque link
  instance values retain their own `source`/`schema`/`description` members.
- `PlannedOperation::input_optional()` → native default-input policy.
- `HttpPlan::first_request()` → a typed `FirstRequest` recipe, lowered from model
  expressions and validated example entries. `source(package)` renders it.
- `http::source_assets()` → the exact compile-time HTTP emitter/runtime asset
  closure for compatibility provenance. Common provenance merges this inventory
  into its sorted, deduplicated, length-delimited hash for every snapshot,
  including empty selections.

Legacy public `.body` and `.responses` fields are **baseline JSON projections**;
compatibility consumes `interface()` for multiple media, parts, streams,
bodyless responses, ranges/defaults and no-input signatures. Source `Location`
is retained separately. Parameter native names remain explicit.
`http-manifest.json` is emitted directly from these plans and includes the full
protocol, native input types, codec bindings, `inputOptional`, provenance,
credential/server choices and limits.

`examples::plan_protocol_examples_v2` and `plan_protocol_examples_v3` are
selected from the actual native codec profile. The v2 example roles use
`role.is_response()` for direction. Native examples use typed literals and
JsonNumber/bigint only where the model requires them; ordinary inputs do not
start with JSON codec plumbing. The package build compiles both validated values
and `examples/first-request.ts`.

Scoped native schema validation, faithful pattern extras/prefix items, checked
codec capture and installed/browser evidence are recorded in
[TypeScript validation v2](validation-v2.md). Codec graphs come
directly from checked programs and retain opaque literal data independently of
source/prose correspondence.

The [v3 native resource profile](validation-v3.md) adds checked
canonical resource/dynamic execution, source-aware model/codec admission and
context-aware conversion traces. ExpandedV1 enumerates the verified
SchemaResources, DynamicSchemaReferences and DocumentRelativeServers capabilities.
Ordinary closures retain their established v1/v2 programs.

## Tests

Tests are at the public Contract → TypeScript HTTP/package → native consumer
seam. Literal vectors come from the independent normative protocol fixture;
they are not snapshots derived from emitter text.

- `tests/typescript_protocol.rs`: protocol planning plus `--ignored` installed
  ESM packages on Node 22/24 with strict TypeScript 5.5/5.9 declarations and
  executable validated examples — literal parameter vectors, whole-query bytes,
  case-exact custom methods, credentials/servers, media/header/link dispatch,
  default/range/bodyless discriminants, binary/form/named/positional multipart,
  streaming/cancellation and explicit legacy byte profiles. The browser Fetch
  test needs an isolated Chromium; set `SUSPECT_CHROMIUM` if it is not at the
  macOS default, and `SUSPECT_NODE24_BIN` to add a second Node runtime.
- `tests/typescript_protocol_compatibility.rs`: rich compatibility capture;
  relocated/prose-only sources stay stable while native
  part/header/stream/signature/credential/status/link literal mutations are
  detected.
- `tests/typescript_multipart_ignored_encoding.rs`: ignored multipart encoding
  styles (below).

```sh
cargo test --locked -p suspect-codegen --test typescript typescript_protocol::
cargo test --locked -p suspect-codegen --test typescript typescript_protocol:: -- --ignored --nocapture
cargo test --locked -p suspect-codegen --test typescript typescript_protocol_compatibility:: -- --ignored
cargo test --locked -p suspect-codegen --test typescript typescript_multipart_ignored_encoding:: -- --ignored
```

Outside `multipart/form-data`, ignored
style/explode/allowReserved fields preserve JSON/text/binary content plans,
content types, required headers and actual codec roots. The defensive
`http-typescript-multipart-content-plan-required` invariant remains; correct
descriptors do not enter that Style-outside-form-data branch.

`typescript_multipart_ignored_encoding::ignored_multipart_styles_preserve_content_in_installed_requests_and_responses`
uses the unchanged `ignoredMultipartEncoding` source fixture. It verifies literal
request/response MIME bytes for an explicitly JSON-encoded string, default integer
text, a whole JSON array, raw bytes and JSON object itemEncoding. It also checks
required headers, exact integers, five indexed codec roots, all eight located
ignored-field warnings, a four-part response with an empty array/absent tail,
11 pre-transport request controls and nine response-decoding controls.

It runs with separate TypeScript **5.5 / 5.9** tarballs installed and executed
on Node **22 / 24**; strict positive/negative consumers and generated example
execution run in each installed package.
