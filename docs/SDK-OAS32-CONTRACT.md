# OpenAPI 3.2 HTTP Contract indexing

Updated 2026-09-10. The public entry point is
`suspect_ir::contract::Contract::from_workspace`. It produces owned source
documents, a finite source-addressed schema graph, and typed HTTP views for the
shared protocol planner. Schema assertion admission is documented separately in
[SDK-SCHEMA-DIALECTS.md](SDK-SCHEMA-DIALECTS.md); wire capability admission lives
in [SDK-HTTP-PROTOCOL.md](SDK-HTTP-PROTOCOL.md).

## Public API handoff

| API | Meaning |
| --- | --- |
| `Operation::method() -> HttpMethod<'_>` | Borrowed, `Copy`, case-sensitive HTTP token. `.as_str()` retains the existing call style. |
| `Contract::reference_target(&SourceId) -> Option<&SourceId>` | One direct indexed reference edge. Pass the **reference object** source, not its `$ref` child. Intermediate reference objects remain separate nodes. |
| `Contract::openapi_version_at(&SourceId) -> &str` | The declaring document's version, or the inherited version of a standalone referenced fragment. |
| `ParameterLocation::Querystring` | A single parameter representing the complete query string; its representations are available through `Parameter::content()`. |
| `MediaType::item_schema()` | Per-item schema for sequential content, independently addressable from `schema()`. |
| `MediaType::schema_roots()` | The declared complete-content and per-item schema IDs, suitable for `Contract::reachable_from`. |
| `MediaType::resolved_source()` | Terminal Media Type Object; `source()` remains the original content-map/component reference location. |
| `Contract::media_types()` | Entry-document `components.mediaTypes` declarations. Their `name()` is a component name; a referencing content view keeps its own media type name. |
| `MediaType::prefix_encoding()` / `item_encoding()` | Positional Encoding Objects with real source IDs, including nested encodings and headers. |
| `Encoding::encoding()` / `prefix_encoding()` / `item_encoding()` | Nested encoding declarations. Positional entries use `EncodingObject`, which has source/raw, header, content-type and serialization accessors. |

Further typed metadata includes:

- `ParameterStyle::Cookie`;
- `Server::name()` and `Response::summary()`;
- `Example::data_value()`, `data_value_source()`, `serialized_value()`, and
  `serialized_value_source()`;
- `SecurityScheme::deprecated()` and `oauth2_metadata_url()`;
- `OAuthFlowKind::DeviceAuthorization` and
  `OAuthFlow::device_authorization_url()`;
- `Header::resolved_source()`.

All new fields are interpreted according to their source's OpenAPI feature set.
Unsupported earlier-version declarations remain in raw storage with located
errors, and their 3.2-only typed accessors return `None`/empty collections.

`reference_target` returns `None` for absent, invalid, or unsupported edges.
Consumers distinguish those cases through the raw declaration and diagnostics.
Cycles retain direct edges, so consumers following them must track visited
sources. Schema recursion is valid; a transport reference cycle receives
`HTTP_REFERENCE_CYCLE` because it cannot produce a finite effective HTTP object.

## Method identity and direction

The nine fixed Path Item operation fields map to `GET`, `PUT`, `POST`, `DELETE`,
`OPTIONS`, `HEAD`, `PATCH`, `TRACE`, and (in 3.2) `QUERY`.
`additionalOperations` preserves every valid HTTP token exactly, including
mixed/lowercase tokens and names beginning with `x-`.

An additional token must be nonempty ASCII `tchar` syntax from RFC 9110. It must
not be one of the nine uppercase fixed methods, even if the corresponding fixed
field is absent. HTTP methods are case-sensitive: `GET` is prohibited in the
additional map, while `get` and `GeT` are distinct, valid method identities.
Invalid tokens, duplicate fixed-method spellings, malformed maps and malformed
operation values receive located errors. They never become invented `GET`
operations. The platform-level `suspect_ir::Method` and `IrSpec` continue to have
their separate eight-method API. Equality comparisons between `HttpMethod` and
the platform `Method` remain supported for existing callers.

`operations()`, `webhooks()` and `callbacks()` retain their respective direction.
Custom methods are indexed in all three collections. Callback expressions,
callback parent/name, webhook name and path-item mount source remain available.
An `x-`-prefixed name in a webhook/callback **map** is a real name; extension
filtering applies to objects that define extension fields, such as Paths and
Callback Objects.

## Querystring parameter declarations

The contract enforces OAS 3.2 §4.12:

1. `name` and `in` remain required strings. The name determines `(name, in)`
   override identity even though it is unused in serialization.
2. `content` is required and contains exactly one media entry.
3. `schema`, `style`, `explode` and `allowReserved` must be absent.
   `allowEmptyValue` is valid only for ordinary `in: query` parameters.
4. There is at most one effective querystring parameter, regardless of name.
5. Ordinary query and querystring parameters cannot coexist, including across
   Path Item and Operation declarations.
6. Operation-level parameters override the same path-level `(name, in)` pair.
   A same-name querystring override is valid; a differently named querystring
   leaves two effective parameters and is rejected.

Collection-local duplicates are checked before overrides; coexistence and
querystring cardinality are also checked on the effective parameter list.
Referenced parameters use their resolved identities and preserve their
declaration and terminal source IDs. Content-based and querystring parameters
have no effective style/explode default.

## Structural schema and reference registration

`contract/shape.rs` defines containment at actual HTTP and schema positions.
`contract/walk.rs` uses those shapes for reference traversal, source-version
context and lexical schema registration. Both readers drive the same traversal
from their independently materialized normalized document values.

The indexed schema positions include:

- component schemas;
- parameter/header `schema` and all content-map representations;
- request and response representations, including referenced bodies/responses;
- both Media Type Object `schema` and 3.2 `itemSchema`;
- `components.mediaTypes` and chains of 3.2 Media Type Reference Objects;
- encoding-header schemas beneath named, positional and nested encodings;
- the corresponding positions inside callbacks and webhooks;
- standard schema-valued applicators and direct schema reference targets.

`schema` describes the complete representation. `itemSchema` describes each
stream item. Both can be present, and each has its own schema ID and reference
closure. The index adds no synthetic array/item schemas and does not replace one
field with the other. A missing YAML value or malformed declared schema remains
an indexed source slot and produces `invalid-schema`.

Schema roots in encoding headers are independently registered. They do not
become schema children of an unrelated media schema; the HTTP planner chooses
the actual body/item/header roots it needs.

Examples (`example`, `value`, `dataValue`, `serializedValue`), defaults, enum and
const instances, extensions, and arbitrary schema-like keys inside those values
are leaves. Their `$ref`, `$anchor`, `$id`, `schema` or `itemSchema` keys cannot
load documents or introduce schemas. Ignored non-schema Reference Object
siblings likewise cannot introduce schema roots or references.

Complete OpenAPI document declarations are registered before anchor lookup;
standalone schema documents and explicitly referenced HTTP/schema fragments
receive their corresponding structural context. This keeps forward, recursive,
escaped and split-file anchor references source-addressed without admitting
instance anchors. Declared but unused external references do not expand the
entry's document closure. Ambiguous anchors produce explicit unresolved edges.

## Provenance, metadata and version boundaries

`SourceId` continues to mean canonical **retrieval URI + decoded JSON Pointer**.
Byte spans are stored separately. An operation mounted through a Path Item
reference keeps its actual declaration in `source()` and its entry mount in
`path_item_source()`.

Effective server precedence remains Operation → Path Item → entry document.
Explicit empty server arrays retain their source and select the OAS default
`/`. Effective security preserves operation overrides, empty-array disabling,
OR alternatives and AND members. Implicit security scheme names use the
existing source-document lookup policy; OAS permits implementation-defined
multi-document implicit connections and recommends an entry-document policy.

Referenced OpenAPI documents supply their own versions and dialect defaults.
Standalone fragments inherit their referring context. Reuse of one source under
different OpenAPI feature sets reports `AMBIGUOUS_OPENAPI_CONTEXT` instead of
silently assigning the first mount's version to every use. Lexical schema
dialects remain available even when only a nested schema is selected by a
reference. OAS 3.2's default schema dialect remains
`https://spec.openapis.org/oas/3.1/dialect/base`.

Declaration checks cover the new metadata's string/Boolean/container shapes,
the exclusive Example Object value forms, encoding map/positional exclusivity,
and positional encoding schema presence. `dataValue` may coexist with
`serializedValue` or `externalValue`; it excludes `value`. Serialized examples
remain distinct from schema-ready instances.

Response descriptions are optional in 3.2 and required in 3.0/3.1. An Operation's
`responses` field is optional in 3.1/3.2 and required in 3.0; a present Responses
Object still requires a response. Exact 100–599 statuses, uppercase 1XX–5XX
ranges and `default` keep their original map keys. Encoding objects use
`contentType` when all RFC6570 serialization fields are absent; the form-style
default is exposed only when an explicit serialization field activates it.

## Remaining limitations

- **Canonical resources:** OpenAPI Object `$self` in 3.2 yields a located
  `unsupported-openapi-self` error. References whose source or target depends
  on `$self` or a schema `$id` remain unresolved with diagnostics. `$id` scope
  errors identify the actual declaring schema and `$id` span, including when a
  reference selects only its nested child. Canonical identifier aliases,
  relocation and resource-relative fragments require separate resolver work.
- `$self` on a Schema Object is an unknown annotation, not OpenAPI document
  identity. Earlier-version OpenAPI `$self` declarations are unsupported fields.
- Dynamic/recursive reference scope, custom dialect vocabularies and legacy
  schema keywords retain their existing explicit limitations. Schema assertion
  and dialect-declaration admission are covered by the owned schema compiler.
- Wire serialization, media selection, streaming parsers, SSE event semantics,
  multipart layout applicability, endpoint/authentication policy and native
  adapter capability remain protocol-layer responsibilities. Typed retention
  does not imply every native backend admits these features. URL and media-type
  grammar checks beyond the retained HTTP declaration checks also belong to
  that admission boundary.
- YAML alias occurrence ambiguity, non-JSON values and unsupported Fast-reader
  syntax retain the existing explicit reader failures.

## Verification and integration

`crates/suspect-ir/tests/contract_oas32.rs` adds **20 passing public-seam tests**.
They assert independently specified method tokens, status values, source IDs,
reference hops, split-file schema closures, querystring override/coexistence
rules, version gates, malformed declarations and source-scoped identity
limitations. JSON/YAML and Lossless/Fast cases check exact values, graph edges,
source spans and diagnostics. Examples with adversarial schema/reference keys
remain raw instances.

Commands run:

```sh
cargo test --locked --offline -p suspect-ir --target-dir target/sdk-oas32-build --no-fail-fast
cargo check --locked --offline -p suspect-codegen --target-dir target/sdk-oas32-build
cargo check --locked --offline -p suspect-codegen --all-features --target-dir target/sdk-oas32-build
cargo test --locked --offline -p suspect-codegen --features http-protocol --target-dir target/sdk-oas32-build --test http_protocol -- --nocapture
cargo test --locked --offline -p suspect-codegen --features http-protocol --target-dir target/sdk-oas32-build --test http_admission --test rust_http --test typescript_http --no-fail-fast
```

- Full IR result: **72 passed, 1 failed, 5 existing tests ignored**. The sole
  failure is the old blanket-3.2 rejection expectation in
  `crates/suspect-ir/tests/http_contract.rs:627–630`. Main needs to remove those
  unsupported-method expectations and update the operation count at line 646
  from 2 to 4. The conflict/cycle/callback assertions remain meaningful.
- `suspect-codegen` library check passed.
- The all-features check is blocked at
  `crates/suspect-codegen/src/backend.rs:250`: the Kotlin arm's `Ok(plan.render())`
  expects `Vec<OutFile>`, but `render()` returns
  `Result<Vec<OutFile>, Vec<HttpDiagnostic>>`. Main needs to propagate/map that
  result as the adjacent C#/C++ arms do. No additional parameter/style/flow
  exhaustive-match error was reported by this check.
- Shared HTTP protocol integration: **20 passed**, including streamed item
  schemas, positional encoding header roots and direct reference provenance.
  The initial lockfile-drift blocker cleared on the later locked run.
- Focused HTTP admission/Rust/TypeScript checks: **14 passed, 3 existing tests
  ignored**. The ignored cases are two native Rust runtime tests and the tracked
  OpenRouter corpus test; these results cover the selected planner/emission
  checks rather than claiming those ignored runtime/corpus gates ran.
- Main has integrated the `ParameterLocation::Querystring` exhaustive arms in
  `crates/suspect-codegen/src/http_examples.rs` and `model_naming.rs`.
  `ParameterStyle::Cookie` and `OAuthFlowKind::DeviceAuthorization` are also
  public additions; the default and `http-protocol` configurations compile with
  them. The all-features blocker is recorded above.

## Primary sources

- [OAS 3.2.0 Path Item](https://spec.openapis.org/oas/v3.2.0.html#path-item-object),
  [Parameter](https://spec.openapis.org/oas/v3.2.0.html#parameter-object),
  [Components](https://spec.openapis.org/oas/v3.2.0.html#components-object).
- [OAS 3.2.0 Media Type](https://spec.openapis.org/oas/v3.2.0.html#media-type-object),
  [Encoding](https://spec.openapis.org/oas/v3.2.0.html#encoding-object),
  [Response](https://spec.openapis.org/oas/v3.2.0.html#response-object),
  [Example](https://spec.openapis.org/oas/v3.2.0.html#example-object).
- [OAS 3.2.0 Server](https://spec.openapis.org/oas/v3.2.0.html#server-object),
  [Security Scheme](https://spec.openapis.org/oas/v3.2.0.html#security-scheme-object),
  [OAuth Flows](https://spec.openapis.org/oas/v3.2.0.html#oauth-flows-object).
- [OAS 3.2.0 reference bases](https://spec.openapis.org/oas/v3.2.0.html#establishing-the-base-uri)
  and [implicit connections](https://spec.openapis.org/oas/v3.2.0.html#resolving-implicit-connections).
- [OAS 3.1.2 Operation](https://spec.openapis.org/oas/v3.1.2.html#operation-object)
  and [Response](https://spec.openapis.org/oas/v3.1.2.html#response-object), for
  version-specific required-field rules.
- [RFC 9110 §5.6.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-5.6.2)
  (`token` / `tchar`) and [§9.1](https://www.rfc-editor.org/rfc/rfc9110.html#section-9.1)
  (case-sensitive method identity).
