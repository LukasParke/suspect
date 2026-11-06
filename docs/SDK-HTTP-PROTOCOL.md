# Shared HTTP protocol plan

`suspect_codegen::http_protocol` is the source-backed HTTP planning layer for
the canonical SDK pipeline. It consumes the existing immutable `Contract` and
publishes wire decisions, codec inputs, provenance, and diagnostics. Language
adapters continue to allocate native names, compile the existing native codecs,
and emit packages, clients, examples, and documentation.

The protocol plan is always compiled. All twelve backends plan through it. Representing a feature here does **not**
certify any native adapter: each adapter opts in to a capability only after its
own native tests cover it.

### Protocol-aware examples

`examples::plan_protocol_examples(contract, &protocol, config)` consumes the
admitted descriptors directly. It does not call the old strict admission layer.
Its version-2 manifest retains exact response statuses or their original
range/default patterns, header/part/item roles, actual codec roots and both
declaration and terminal example locations. Native renderers can use
`ExampleRole::is_response()` when choosing request/response model views.

Stream item examples belong to `itemSchema`; an entire media example is not
silently treated as one item. SSE `data` remains a string. Byte payloads and
schema-free values require explicit native recipes rather than invented schema
nodes or JSON-null stand-ins. Request-body/parameter binding remains distinct
from part/item binding. OpenAPI3.2 `dataValue` and the OAS3.0 reference-sibling
rules are preserved in source example discovery.

`crates/suspect-codegen/tests/protocol_examples.rs` covers status patterns,
referenced `dataValue` provenance, multipart byte separation, SSE-envelope items,
bodyless responses sharing a request codec, unavailable examples, ignored 3.0
siblings, false positional prefixes and occurrence-aware positional projections.
Native SDK tests remain the acceptance seam for language call sites.

## Public API

```rust,ignore
use suspect_codegen::http_protocol::{self, Capabilities, Capability, ByteLimits};

let capabilities = Capabilities::for_adapter(
    "my-native-http-v1",
    [Capability::AnonymousSecurity, Capability::RelativeServers,
     Capability::DocumentRelativeServers],
).with_limits(ByteLimits::new(8 * 1024 * 1024, 1024 * 1024, 64 * 1024));

let protocol = http_protocol::plan(&contract, &selected_operation_sources, capabilities)
    .into_result()?;

// Feed these exact existing SchemaIds to the normal native codec planner.
let roots = protocol.codec_roots();
for operation in protocol.operations() {
    for parameter in operation.parameters() {
        let schema_id = parameter.codec().schema().id();
        let serialization = parameter.serialization();
        let declaration = parameter.source().use_site();
        let definition = parameter.source().terminal();
    }
}
```

`plan(&Contract, &[SourceId], Capabilities) -> ProtocolPlan` is deterministic
and atomic. `ProtocolPlan::is_admitted()` / `into_result()` guard artifact
construction. On any error, both `operations()` and `codec_roots()` are empty;
`diagnostics()` retains all collected findings. A valid empty selection is an
empty plan. Warnings preserve uninterpreted annotations without activating them.

The descriptors have private fields and read-only accessors, implement
`Serialize`, and have descriptor version `PROTOCOL_PLAN_VERSION == 1`. Serialized
source IDs contain separate retrieval-document and JSON-Pointer fields. These
are structured descriptors, not parsed fragments of emitted source code.

### Descriptor vocabulary

| Descriptor | Public accessors / variants |
| --- | --- |
| `ProtocolPlan` | `version()`, `capabilities()`, `operations()`, `codec_roots()`, `codec_schema_closure()`, `diagnostics()`, `is_admitted()`, `into_result()` |
| `OperationPlan` | `source()`, `path_item()`, `method()`, `path()`, optional `operation_id()` / `summary()` / `description()`, `deprecated()`, `tags()`, `servers()`, `security()`, `parameters()`, optional `body()`, `responses()`, `annotations()` |
| `Method` | Fixed variants plus `Custom(String)`; `as_str()` preserves the exact HTTP token, `is_custom()` identifies additional-method declarations |
| `Provenance` | `use_site()`, `terminal()`, `references()`, optional `use_site_resource()` / `terminal_resource()`, aligned `reference_resources()`; each physical `SourceLocation` exposes `source()` and `span()` |
| `ResourceContext` | physical lookup `source()`, resource-boundary `resource()`, `kind()`, logical `canonical_uri()` / `base_uri()` / `scope_address()`, optional `base_source()` / `schema_root()`, `aliases()`, read-only `resolve_reference(text)` |
| `Diagnostic` | physical `source()`, `related()`, optional `resource_context()`, `code()`, `severity()`, `kind()`, `message()`, `capability()` |
| `Located<T>` | `value()`, `source()`; used for defaults, enums, permissions, URLs, and literal link values |
| `SchemaUse` | `id()` is an actual indexed schema input; `source()` preserves the schema use-site and reference target; `resource_context()` retains its lexical resource metadata |
| `CodecRef` | `schema()`, `input()` (`Json` or `TextScalar`) |
| `ServersPlan` | effective-array `source()`, ordered `candidates()` |
| `ServerPlan` | optional declared `source()` or `default_from()`, `template()`, `variables()`, `description()`, `name()`, physical `document_base()`, `url_base()`, `expand(overrides)`, `resolve_url(document_url, overrides)`, `resolve_document_url(overrides)` |
| `SecurityPlan` | `Undeclared`, `NoAuth`, or `Alternatives`; `alternatives()` returns OR alternatives |
| `SecurityAlternative` | `source()`, `is_anonymous()`, `requirements()` (AND) |
| `CredentialRequirement` | `source()`, `scheme()`, `name()`, `permissions()`, `credential()`, `description()`, `deprecated()`, optional located `deprecated_source()` |
| `Permissions` | `Scopes(Vec<Located<String>>)` or `Roles(Vec<Located<String>>)` |
| `CredentialHook` | `Bearer`, `Basic`, `ApiKey { location, name }`, `OAuth2 { flows, metadata_url }`, `OpenIdConnect { discovery_url }`; `url_base()` is `EffectiveServer` for OAuth/OIDC metadata URLs |
| `ParameterPlan` | `source()`, `name()`, `location()`, `required()`, `deprecated()`, `description()`, `codec()`, `serialization()`, optional `content_media()`, `examples()`, `serialize(codec_valid_value)` |
| `ParameterSerialization` | `Style { style, explode, shape, percent_encoding }` or `Content { media_type, percent_encoding }` |
| `WireShape` | scalar, homogeneous scalar array, or flat object with explicit `AdditionalScalars` policy |
| `BodyPlan` | `source()`, `required()`, `description()`, `media()`, `limits()`, `match_media(concrete_content_type)` |
| `MediaPlan` | `source()`, `media_type()`, `representation()`, `examples()` |
| `Representation` | `Json`, `Text`, `Binary`, `Form`, `Multipart`, `Stream` |
| `PartPlan` | `source()`, optional `name()`, aggregate/property `schema()`, `required()`, `multiplicity()`, `min_items()` / `max_items()`, optional `encoding_source()`, `content_types()`, `representation()`, `headers()` |
| `PartRepresentation` | `Json`, `Text`, `Binary`, or explicit RFC6570 `Style`; JSON/text/style variants carry the actual codec input |
| `MultipartPlan` | named properties plus structural object rules, or positional prefix/items plus array cardinalities |
| `StreamPlan` | `source()`, `framing()`, `item_codec()`, `max_item_bytes()` |
| `ResponsePlan` | `source()`, `status_key()`, `status()`, `description()`, optional `declared_description()` / `summary()`, `media()`, `headers()`, `links()`, `max_body_bytes()` |
| `HeaderPlan` | `source()`, `name()`, `required()`, `deprecated()`, `description()`, `codec()`, `serialization()`, optional `content_media()`, `examples()`, `serialize(codec_valid_value)` |
| `LinkPlan` | `source()`, `name()`, `target()`, `parameters()`, `request_body()`, `description()`, `server()` |
| `ExampleMetadata` | optional located `inline()`, `named()` Example Plans |
| `ExamplePlan` | `source()`, `name()`, optional located `summary()`, `description()`, `value()`, `data_value()`, `serialized_value()`, `external_value()` |

### API notes

`Method` owns custom tokens and is `Clone`, not `Copy`.
`OperationPlan::method()` returns `&Method`, and `Method::as_str()` returns a
borrowed `&str`. Calls such as `operation.method().as_str()`, enum pattern
matches, and comparisons with `Method::Head` work in both operand orders of
borrowed/owned comparisons. Consumers needing an owned method can clone it. Serialization remains a plain
token string for both fixed and custom methods.

For a 3.2 response with no description, the display-oriented `description()` returns empty text
anchored at the actual Response Object; `declared_description()` returns `None`.
No nonexistent `/description` source is created.

`OperationPlan::match_response(actual_status, content_type)` returns a
`ResponseMatch` with the actual `status()`, `is_success()`, selected `response()`
and optional `media()`, and `body_disposition()`. Status selection is **exact >
class > default**, before media selection. A media mismatch on an exact status
does not fall back to its class/default. Success is determined from the actual
200–299 status, including a successful status matched by `default`.

`BodyPlan::match_media(concrete_content_type)` applies the same media precedence
to requests. A caller cannot select a broad byte representation to bypass a more
specific JSON request schema. A wildcard is a declaration matcher, not a concrete
Content-Type to send on the wire. `MediaMatchError` reports invalid/unmatched
types and unsupported charsets.

## Capability contract

`Capabilities::default()` identifies `strict-json-v1`. Empty `enabled` sets use
the same base feature set. `for_adapter(name, capabilities)` names the adapter's
explicit opt-ins; `with(capability)` adds one. `Capability::ALL` is the planner
vocabulary, **not a native-support default**. The normative tests use the separate
adapter identity `normative-fixtures-v1`.

The feature vocabulary is:

- Selection/methods/dialects: `UnnamedOperations`, `AdditionalMethods`, `CustomMethods`,
  `OpenApi30`, `OpenApi32`.
- Servers: `HttpServers`, `RelativeServers`, `DocumentRelativeServers`,
  `MultipleServers`, `ServerVariables`.
- Security: `AnonymousSecurity`, `SecurityAlternatives`, `ConjunctiveSecurity`,
  `HttpBasic`, `ApiKeys`, `OAuth2`, `OpenIdConnect`, `SecurityRoles`.
- Parameters: `ParameterStyles`, `HeaderParameters`, `CookieParameters`,
  `ReservedParameters`, `ContentParameters`, `QuerystringParameters`,
  `QuerystringForm`.
- Responses/media: `RangeResponses`, `DefaultResponses`, `MultipleMediaTypes`,
  `MediaRanges`, `MediaTypeParameters`, `StructuredJsonMedia`, `SchemaFreeJson`,
  `TextBodies`, `BinaryBodies`, `UndeclaredResponseBody`, `ResponseHeaders`,
  `ResponseLinks`, `UndeclaredResponses`.
- Parts/streams: `FormBodies`, `MultipartBodies`, `PartEncodings`,
  `PositionalMultipart`, `ServerSentEvents`, `JsonLines`.
- Executable schema semantics: `SchemaResources`, `DynamicSchemaReferences`.
  These require resource/dynamic support in the selected native codec profile
  (the v3 validation program); Contract indexing alone is not enough.

Unsupported capability errors carry `Diagnostic::capability()` and the precise
source. `DiagnosticKind` distinguishes malformed source, currently unsupported
semantics, adapter capability refusal, and uninterpreted annotation.

### Adapter guidance

An adapter projects an admitted plan into its language plan. Native naming,
collision checks, directional-model policy, codec compilation, transport
limits, documentation, and artifact ownership stay in that same pipeline. Each
additional capability is enabled only after its native tests pass.

Read the original schema IDs from codec references. Use
`operation_id().value()` for the source name and the native symbol allocator for
the generated name. Use declaration and terminal provenance in docs/diagnostics
rather than copying an arbitrary schema's span.

Do not rerun `http_contract::plan` as another admission stage for a
protocol-planned operation. Adapters consume the descriptors directly; there is
no additional SDK pipeline or emitted-code compatibility parser.

The planner applies ordinary OpenAPI rules where the older strict profile used
narrower literal tests (for example case-insensitive media/auth names and
ignored metadata fields). Native tests must cover these rules; a baseline
projection alone does not show that an emitter implements them.

## Semantics that adapters must preserve

### Canonical resources and URL bases

Canonical references use the Contract's public resource catalogue
([ADR 0008](adr/0008-sdks-generate-from-the-source-addressed-contract.md)). A known `$self`, `$id`,
resource-root pointer or anchor can resolve to an HTTP/schema object in another
supplied document. `SourceId` and spans continue to identify its **effective
physical retrieval document**. Requested redirect aliases, logical names and
cache paths never replace that identity.

`Provenance` exposes logical resource context beside each physical use-site,
terminal and reference hop. `ResourceContext::resource()` is the physical resource
boundary; `canonical_uri()` is its logical name; `base_source()` identifies the
actual `$self`/`$id` declaration when one supplied the base. `scope_address()` is
the nearest registered object's resource-relative address, exactly as provided
by Contract. For an unregistered keyword child it is an enclosing scope address,
not an invented logical address for that child.

The different URI/URL roles have deliberately different bases:

| Field / role | Base and behavior |
| --- | --- |
| HTTP/Schema `$ref`, Link `operationRef` | The declaring source's canonical resource base; `Contract::reference_target` / `resolve_resource_reference` return physical target IDs. Missing catalogue/operation discovery is an explicit diagnostic. |
| Description URIs such as `Example.externalValue` | The lexical reference base. `ResourceContext::resolve_reference` resolves the string without looking up, loading or fetching it. |
| Relative `Server.url` | The effective retrieval URL of the document **containing that Server Object**, per OAS 3.2 §4.5. A `$self` URI does not relocate the API. |
| OAuth2/OIDC metadata endpoint URLs | The selected effective server, per OAS 3.2 §4.5.2. `CredentialHook::url_base()` and `OAuthFlow::url_base()` expose `ApiUrlBase::EffectiveServer`. Raw values and sources remain unchanged; no acquisition is performed. |
| Relative links inside CommonMark | Their rendered context, per OAS §4.1.2.2.3, rather than a blanket resource-base rewrite. |

`ServerPlan::document_base()` records the physical document root.
`resolve_document_url(overrides)` uses that retrieval URL; `resolve_url` retains
the explicit caller-provided document-base override. A default `/` from absent
entry servers belongs to the entry document; an explicit empty override belongs
to the overriding declaration's document. Local-file sources need an explicit
HTTP base. Resolution uses the shared strict RFC 3986 helper, preserving encoded
dots/slashes and path spelling rather than a browser URL parser's repairs.

`DocumentRelativeServers` is a new, default-disabled native fence in addition to
`RelativeServers`. Relative declarations and URL templates without a fixed
literal HTTP(S) scheme require it, even when a template's default is absolute.
An adapter opts in only after its native tests cover physical-document and
explicit-override resolution, redirect provenance and encoded-path handling. An absolute
server continues to be independent of the document's logical reference base.

For JSON/text codec inputs, `SchemaResources` gates canonical resource semantics;
`DynamicSchemaReferences` also gates `$dynamicRef`/`$dynamicAnchor`. Consumers must
pair these assertions with the explicit v3 resource/dynamic validation program
([SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md)). The shared planner supplies the actual `codec_roots()` and
Contract's candidate-aware `codec_schema_closure()`. Candidate bindings are a
finite planning superset, **not** generation-time target selections. The native
codec retains dynamic-reference metadata and evaluates the resource stack; it
must not compile the initial target as an unconditional static reference.

These fences are evaluated on the effective codec closure, which ignores OAS
3.0 reference siblings and retains original dialect/context origins. Forbidden/bodyless response payloads do not become codec inputs or
resource-profile requirements. Byte schemas keep their physical/logical metadata
and finite byte bounds without entering a JSON codec or substituting null.
Non-JSON wire-shape proof through a dynamic reference remains an explicit refusal.

Native consumer changes are limited and explicit:

1. For a relative server's default, use `document_base()` /
   `resolve_document_url()`; keep a caller-supplied document-base override
   explicit. Opt in to `DocumentRelativeServers` only after the native transport
   preserves the physical base and encoded path semantics.
2. Feed `codec_roots()` to the selected codec planner and retain
   `codec_schema_closure()` / Contract dynamic-reference metadata for resource
   linking. Opt in to `SchemaResources` / `DynamicSchemaReferences` only with the
   v3 executable program. Do not replace a dynamic use-site
   with its fallback or expand candidate schemas into additional body inputs.
3. Continue using physical `SourceLocation` for diagnostics, compatibility and
   ownership. Use the separate `ResourceContext` for logical reference addresses;
   its URI resolver is metadata-only and supplies no acquisition authority.

### Source and codec identity

- A Schema Object `$ref` retains its **indexed use-site root**, so OAS 3.1
  assertion siblings are not discarded. Terminal identities are provenance, not
  an instruction to substitute the terminal-only codec.
- HTTP references retain original declarations and terminal targets separately.
  Path-item mounts also retain their source even when the operation lives in an
  external component. A source mounted at several paths is refused as ambiguous
  for source-only selection.
- Parameters are validated at both declaration levels before override filtering.
  Valid operation-level `(name, in)` overrides work; same-level duplicates and
  malformed overwritten metadata cannot disappear.
- Referenced fragments use `Contract::openapi_version_at`, including when a 3.1
  entry references a 3.2 document that supplies a standalone fragment's context.
  Ambiguous feature-set reuse is a located refusal, not a first-mount guess.
- `codec_roots()` includes only actual JSON/text inputs: normal parameter/header
  values, JSON/text bodies, JSON/text/style form parts, repeated part **items**,
  and stream items. Byte parts, binary bodies, and mixed multipart aggregates
  are never validated by substituting JSON `null` for bytes.
- A form-urlencoded **querystring parameter** additionally binds its complete
  JSON object through `ParameterPlan::codec()`. That is an actual JSON input,
  with no byte-valued fields; its field codecs remain explicit. This does not
  make a mixed multipart aggregate a JSON codec input.
- Forms/multipart enforce required fields, declared extras policy and supported
  cardinalities structurally. Whole-object assertions needing a JSON aggregate
  are explicitly refused. Untyped extra form/part values currently require
  `additionalProperties: false` or an explicit additional-part schema.
- HEAD and statuses that forbid content keep declaration metadata but do not
  add unused body codecs to the root set. Headers can still require codecs.

### Servers and credentials

Absent/empty effective server arrays select the normative relative `/`, with a
default anchor rather than an invented Server Object. Operation servers override
path-item servers, which override root servers. Variables require string defaults;
enums are nonempty strings and contain the default. Runtime substitutions validate
override names/enum values and the resulting URL. Relative servers resolve against
the URL serving the source document. A local file is not a guessed network origin.
Every generated SDK checks caller overrides and source-declared servers with one
policy ([ADR 0035](adr/0035-one-server-url-policy.md)): a source-declared server keeps
its scheme, so a declared plain `http` server is honored; a caller override is `https`,
or plain `http` only to `localhost`, `127.0.0.1` or `[::1]` unless the caller opts in on
that client (Swift `allowHTTP`; `allowInsecureHttp`/`allow_insecure_http`/
`AllowInsecureHTTP`/`AllowInsecureHttp` elsewhere); no userinfo, query or fragment;
a valid port; a literal `.`/`..` segment in an absolute server URL is refused (only
a relative reference is resolved, removing literal dot segments); percent-encoded
octets are opaque and sent verbatim; malformed escapes, non-ASCII, controls and
backslashes are refused.

Security arrays are OR, member schemes are AND, `{}` is an explicitly anonymous
alternative, `[]` disables inherited security, and absence stays distinguishable.
The current Contract's documented source-document-local component-name policy is
used for multi-document implicit security connections. There is no fallback to a
similarly named scheme in another document. Non-OAuth role names and OAuth/OIDC
scope names remain separate, located values. Flow URLs, scope descriptions, and
OIDC discovery metadata are retained.

Bearer/basic/API-key attachment is explicit. OAuth/OIDC hooks accept caller
credentials; they do not acquire tokens, refresh them, perform discovery, select
flows, infer bearer token types, retry requests, or infer business authorization.
Token acquisition, refresh, discovery and the single 401 replay exist only in
the opt-in `sdk_defaults` OAuth lifecycle, planned separately by
`http_protocol::oauth` ([SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md#5-oauth-lifecycle)).
Conflicting conjunctive attachments are refused rather than overwritten.

### Parameters and media

`GET`, `PUT`, `POST`, `DELETE`, `OPTIONS`, `HEAD`, `PATCH`, `TRACE`, and `QUERY`
retain their fixed spelling. Valid OAS 3.2 `additionalOperations` tokens retain
their exact case, including `get`, `GeT`, and `x-PING`. Uppercase fixed spellings
are forbidden in that map even if the corresponding fixed field is absent.
Malformed maps/operations/tokens are checked before filtered IR views can hide
them. `CustomMethods` is separate from `AdditionalMethods`: transports that
normalize method case cannot opt in based on fixed-method tests. `CONNECT`
tunneling remains an explicit unsupported transport feature.

The planner distinguishes scalar, scalar-array, and flat-object styles. It
supports simple/label/matrix/form/spaceDelimited/pipeDelimited/deepObject, header
values, cookie form values where defined, and OAS 3.2 `style: cookie`. OAS 3.1
`deepObject, explode: false` is undefined; OAS 3.2 ignores that explode flag.
Exploded `form` composites in cookies are refused rather than silently replacing
`&` with `; `. Arrays/objects nested within a style value are refused. Additional
untyped object properties still have a runtime scalar-only wire guard.

OAS 3.2 `in: querystring` requires exactly one content representation and has no
name prefix or style/explode default. JSON and UTF-8 text receive one URI-component
encoding pass. Form-urlencoded content uses the retained `FormPlan`; its output
already incorporates URI encoding and receives no second pass. Examples include
`bar=true&foo=a+%2B+b` for form content, compared with percent-encoded complete JSON
or text content. `content_media()` retains media-reference provenance and the
chosen representation for native adapters.

The name still participates in `(name, in)` override identity. A same-name
operation override replaces the path-item querystring; a different name leaves
two and is rejected. Ordinary query/querystring coexistence is forbidden across
both levels. `schema`, `style`, `explode`, `allowReserved`, and `allowEmptyValue`
cannot appear on a querystring declaration. Adding a query API-key credential to
a complete querystring remains unsupported without a defined credential/content
merge policy. File, multipart, and stream parameter contents remain refusals.

`ParameterPlan::serialize` and `HeaderPlan::serialize` are reference interpreters
of admitted descriptors. Call the normal bound codec first for full schema
validation. Object-key order is lexicographic. They refuse CR/LF/control injection,
undefined empty-composite expansion, nested values, and ambiguous active
delimiters. Optional empty composites can be omitted by the caller; there is no
automatic conversion into a required empty field. Header values are passed without
URI encoding or automatic quoting. Cookie values using the OAS 3.2 cookie style
must already escape characters disallowed by cookie syntax.

`allowReserved` preserves legal reserved characters and valid percent triples.
The caller must pre-escape query/form hazards such as `&`, `=`, `+`, and `#`, and active
value delimiters. For space/pipe/deepObject styles, delimiters themselves are
percent-encoded; values containing those delimiters require an additional
API-defined escape convention, per OAS Appendix E.6. The planner does not invent it.

Media names are parsed case-insensitively, with structured parameters. Concrete
media outrank type wildcards, which outrank `*/*`; more declared matching
parameters break ties at the same range specificity. Charset values are
case-insensitive; other parameter values are not globally lowercased. Invalid
tokens/parameter maps, duplicate parameters, invented suffix wildcards, and
equally specific overlapping declarations are source errors. Runtime dispatch
requires a real Content-Type for declared content and does not sniff it.

JSON and `+json` use JSON codecs; UTF-8 text uses an explicit scalar/text input;
opaque bytes use `BytePolicy`. No declared response content means an unspecified,
bounded byte body—not an assertion that the body is empty. HEAD/1xx/204/205/304 forbid
body decoding independently of the declared media. Required response and part
headers retain their required flag; native decoders must enforce it.

An absent Operation `responses` field is valid in 3.1/3.2 and requires
`UndeclaredResponses`. It produces no invented default response; runtime statuses
remain undeclared. A present but empty/malformed Responses Object still fails.
Response descriptions are optional in 3.2, with summary and absence retained.

Links retain operation identity, parameter/request-body literals or expression
text, descriptions and optional servers. A Link selects no operation implicitly
and triggers no invocation. Objects under a link literal containing `$ref` or
`schema` are instance data, not additional schema roots.

3.2 Example Objects retain `dataValue` separately from `serializedValue` and
`externalValue`, including explicit null. `value` excludes all three;
`serializedValue` and `externalValue` exclude one another, while `dataValue` can
accompany either. Field types, exclusivity, feature-set boundaries, reference
provenance, and reference annotation overrides are checked here. Instance
validation remains in the canonical example/codec pipeline. Schema/reference-like
keys inside example values cannot load documents or contribute codec roots.

### Forms, multipart and streams

Form-urlencoded, multipart, JSON, text and bytes are separate representations.
Part defaults follow their actual schemas. Complex JSON parts bind JSON codecs;
plain scalar parts bind text codecs. Array properties use per-item codecs and
repeated names. In `application/x-www-form-urlencoded` and
`multipart/form-data`, explicit style/explode/allowReserved switches an Encoding
Object to its RFC6570 strategy and ignores `contentType`, as required by the spec.
OAS 3.2 applies named encodings per array item; OAS 3.1 explicit style encoding
retains the whole property representation.

Those three serialization fields are **ignored for other multipart media**, per
OAS 3.1.2 §4.8.15.1.2 and OAS 3.2 §4.15.1.2. Their presence does not select a
Style descriptor or discard `contentType`. The planner carries the enclosing
media context through part planning, retains the actual explicit/default
`content_types()` and `PartRepresentation::{Json, Text, Binary}`, and emits a
located `http-encoding-style-ignored` warning for each ignored field. Part headers,
schema IDs, byte bounds and codec roots retain their original sources. For
positional multipart, an array-valued part stays one JSON part; an ignored explode
flag does not manufacture item grouping.

The literal `ignoredMultipartEncoding` fixture covers both request and
response `multipart/mixed`: explicit JSON string content, default integer text,
whole-array JSON, opaque bytes and JSON-object `itemEncoding`. Negative tests
show ignored fields cannot bypass an incompatible actual content codec or malformed
metadata. The paired 3.1/3.2 controls preserve active form-data/form-urlencoded
behavior; 3.1 non-form-data multipart without supported correlation and 3.1
positional encoding declarations retain their existing located refusals.

Named multipart and OAS 3.2 positional prefix/items retain encoding/header
sources and codec references. File inputs are finite, in-memory bytes; filename
and Content-Disposition policies are native transport concerns, with declared
required headers preserved. Nested multipart/transfer-encoding, mixed JSON/text/
binary `contentType` choices for a single part, and streamed multipart are
currently explicit refusals.

`text/event-stream`, `application/jsonl` and `application/x-ndjson` item streaming
uses standard OAS 3.2 `itemSchema`. SSE's input to the item codec is the **parsed
event envelope** after HTML event-stream framing: `data`, `id`, and `event` are
strings; `retry` is an integer. Multiline data is joined according to that framing.
Neither JSON inside `data` nor a `[DONE]` sentinel is inferred. `contentSchema`
remains an annotation unless the normal schema/compiler policy explicitly
implements it. JSON lines have no record-separator or sentinel shortcut.

A 3.1 SSE `schema` is complete-content metadata and cannot stand in for
`itemSchema`. Whole-content plus item validation, JSON text-sequence framing,
and vendor stream/request-field/sentinel conventions require further verified
semantics. Unknown extensions are located, uninterpreted annotations.

`CompatibilityProfile::LegacyBinaryStringV1` is an explicit opt-in for legacy
OAS 3.1/3.2 `type: string, format: binary` markers **in byte contexts**. The normal
OAS 3.1+ binary schema is unconstrained. This versioned profile introduces no
JSON-null conversion and no streaming convention.

## Native acceptance required before opting in

Every capability needs tests through the public native SDK entry point. Planner
fixtures alone do not replace the following adapter tests:

| Capability family | Required native tests |
| --- | --- |
| Baseline projection | Existing real selected-operation packages/codecs/docs/types/wire gates; declaration/terminal identity and unchanged owned artifacts; case-insensitive media/auth corrections |
| Methods/servers | GET/PUT/POST/DELETE/OPTIONS/HEAD/PATCH/TRACE/QUERY, custom tokens preserving case without transport normalization, fixed-method duplicate refusal, explicit caller server choice, relative document base, variable defaults/enums/overrides, invalid overrides before transport |
| Security | Anonymous/disabled/OR/AND alternatives, case-insensitive bearer/basic attachment, header/query/cookie API keys, scopes versus roles preserved, hooks receiving source/flow metadata, conflicts refused, no implicit acquisition or retries |
| Parameters | Literal normative style vectors, UTF-8 escaping, reserved-value hazards, headers unchanged, cookie rules, JSON content, complete-query JSON/text/form encoding without a name prefix or double encoding, legal overrides and coexistence/duplicate refusals, non-null/flat guards, required and omitted inputs |
| Responses/media | Exact over range over default, actual-status success, no media fallback across statuses, parameters/wildcards/tie refusal, missing or invalid Content-Type, undeclared bounded bytes, HEAD/204 body suppression |
| Typed headers/links | Required header decode failure, non-string scalar/array/object codecs, exact reference provenance, Set-Cookie repeated-field limitations, links rendered as metadata without inferred calls |
| Binary/parts | Byte-for-byte equality including zero/non-UTF8 octets, separate body/part ceilings before allocation, required/extras/cardinality rules, repeated files and item codecs, correct content/disposition/part headers and itemEncoding, no placeholder nulls |
| SSE/JSON lines | Arbitrarily split transport chunks, UTF-8 boundaries, SSE CR/LF/comment/field/multiline rules and numeric retry, item codec failures, `[DONE]` treated as ordinary data, cancellation/close/backpressure, per-item and total transport bounds, JSON-lines framing |
| Every expansion | Native positive/negative consumer typing, checked codec/transport execution, package install/import, executable examples, accurate docs, ownership/compatibility reports, source-linked decline paths |

## Tests and IR integration

The test seam is **public source `Contract` → wire plan**, including the plan's
parameter reference interpreter and response matcher. Fixtures are literal
synthetic normative/adversarial examples in
`crates/suspect-codegen/tests/fixtures/http-protocol-v1.json` (literal parameter
vectors, complete-query vectors and located malformed-declaration cases). They
are not generated snapshots and are not claimed to occur in the OpenRouter
corpus. `crates/suspect-codegen/tests/http_protocol.rs` adds source-to-plan
cases for custom methods, querystrings, media references, indexed stream/header
roots, fragment-version context, redirects and canonical `$self`, split HTTP and
media references, physical versus logical bases, canonical links, OAuth URL base
classification, unresolved logical URI diagnostics, dynamic candidates reached
through nested schemas, forbidden/binary codec-root separation and ignored
multipart encoding fields.

Run the module's tests with:

```sh
cargo test -p suspect-codegen --test shared http_protocol::
```

Native language tests are separate and do not change by editing this document.

The planner consumes these canonical IR interfaces of the source-addressed
Contract
([ADR 0008](adr/0008-sdks-generate-from-the-source-addressed-contract.md)):

1. `Contract::reference_target` follows the real direct reference graph, retaining
   every intermediate source. `openapi_version_at` supplies the declaration's
   version, including inherited standalone-fragment contexts.
2. `Operation::method()` supplies case-preserved `HttpMethod` tokens; querystring
   parameter identities and effective overrides are indexed by the same Contract.
3. Media Type references, `itemSchema`, and positional encoding header schemas are
   actual indexed sources. The fixtures compare planner roots with
   `MediaType::item_schema()`, `schema_roots()`, and `resolved_source()`. An
   unindexed or malformed schema still receives a located error; none is forged.

Known canonical `$self`/resource-URI references are resolved by Contract.
URI-named security/connections requiring missing discovery remain source-linked
refusals. `operationRef` uses canonical lookup
but identifies only existing indexed operations. Nested or streamed multipart,
aggregate-plus-stream validation, and CONNECT tunnels remain explicit refusals
even with `Capability::ALL`.

### Native ignored-encoding test

The TypeScript native test
`typescript_multipart_ignored_encoding::ignored_multipart_styles_preserve_content_in_installed_requests_and_responses`
(`--ignored`; needs pinned Node 22/24 and offline npm with TypeScript 5.5/5.9)
packs and installs the generated package and checks request and response
exchanges, pre-transport request controls, response controls, required headers,
exact integers, and literal JSON/text/binary MIME payloads from the source
fixture. `SUSPECT_PROTOCOL_ARTIFACTS` chooses where its artifacts are retained.
The `http-typescript-multipart-content-plan-required` guard remains a defensive
invariant; the corrected descriptors do not enter the invalid
Style-outside-form-data branch.

### Normative primary sources

- [OAS 3.1.2](https://spec.openapis.org/oas/v3.1.2.html): Server/Server Variable,
  Parameter and Style Examples, URL Percent-Encoding and Appendix E, Media Type,
  Encoding, Responses/Response, Header, Link, Security Scheme/Requirement and
  OAuth Flow objects.
- [OAS 3.2.0](https://spec.openapis.org/oas/v3.2.0.html): version-specific cookie
  and deepObject rules, itemSchema/sequential media, SSE parsed event objects,
  named and positional part encoding, binary maxLength, and credential metadata.
- [OAS 3.2 §4.5](https://spec.openapis.org/oas/v3.2.0.html#server-object) and
  [§4.5.2](https://spec.openapis.org/oas/v3.2.0.html#relative-references-in-api-urls):
  server/API location bases remain distinct from `$self` reference identity;
  [RFC 3986 §5](https://www.rfc-editor.org/rfc/rfc3986#section-5) supplies resolution.
- [RFC6570](https://www.rfc-editor.org/rfc/rfc6570),
  [RFC9110](https://www.rfc-editor.org/rfc/rfc9110),
  [RFC7578](https://www.rfc-editor.org/rfc/rfc7578), and the
  [HTML event-stream algorithm](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream)
  are the cited underlying wire specifications.

The ordinary OAS rules take precedence over vendor extensions and operation-name
heuristics.
