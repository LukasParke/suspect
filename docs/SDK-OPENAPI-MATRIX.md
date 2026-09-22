# OpenAPI support matrix for SDK generation

Normative feature matrix for the SDK program
([SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md) §13), published with
**honest statuses verified against the current tree** (2026-09-15). The target
is complete coverage of OpenAPI 3.0.x, 3.1.x, and 3.2.x across all twelve
native backends; this table records where that work actually stands today.

Every status below was checked against the repository files named in it.
Where a claim could not be verified from the code, the status says
**unverified** rather than overclaiming.

## How to read the columns

- **Completion scope** is the normative target from the plan; it is not a claim
  of current support.
- **Evidence tier** records the strongest verification class available for the
  row's status:
  - *generation* — exercised through the shared Contract/protocol planner and
    located diagnostics;
  - *native runtime* — exercised against emitted native code paths;
  - *installed-package* — exercised through installed packages and consumers
    (the class used by the [capability matrix](SDK-CAPABILITIES.md) and the
    [env-credential acceptance](SDK-CREDENTIAL-ENV.md)).
- **Status** is the current repo state for the family as a whole, including the
  honest exceptions.

The plan also requires tracking *source parsing, semantic planning, emitted
representation, native runtime behavior, optional-validator capability, and
installed-package evidence separately per row and per target*. This table is
the published rollup; per-target detail lives in the linked capability and
protocol documents.

## Matrix

| Feature family | Completion scope | Evidence tier | Status in this repo |
| --- | --- | --- | --- |
| Documents and references | JSON/YAML, split documents, components, reference siblings, canonical URIs, `$self`, `$id`, anchors, dynamic references, and version/dialect context. | generation | **Implemented** for OAS 3.0/3.1/3.2 frontends: source-addressed local/split inputs, acquired pinned/offline closures, stable URI/pointer identities, reference siblings, `$self`/`$id`/resource-local anchors and dynamic candidates indexed by the Contract, canonical dialect contexts. Runtime dynamic binding still requires its own executable admission profile. The OpenAPI 2.0 family is detected but is **not** an SDK frontend yet. (`docs/SDK-CAPABILITIES.md` "Contract and runtime scope".) |
| Schemas and native models | All normative schema constructs, compositions, recursion, discriminators, directions, presence/null, exact numbers, extras, XML annotations, and optional standard validation integration. | installed-package | **Implemented for the admitted, witnessed set — not a 100%-of-constructs claim.** All twelve profiles emit native models with checked codecs and have installed consumer evidence (exact integers/decimals, explicit absence/null, source wire names, typed extras, finite recursion, bounded unions/intersections, encode-time validation of mutable models). Directions: only TS/JS admits the `oas31-required-applicability-v1` request/response views; other profiles refuse active directional projections explicitly. Owned validation v1/v2 is available with scoped applicators and independent vectors; native v2 adoption is tracked separately. XML annotations are verified **retained but not interpreted**: they are kept as source provenance, echoed only in the documentation artifacts that deliberately keep original schemas, validated for well-formedness when native codecs compile (a malformed annotation fails with `codec-schema-compilation`), and never applied to JSON wire names — `xml.name` does not rename a JSON property and no XML codec is emitted (verified in `crates/suspect-codegen/tests/sdk_xml_media.rs`). Full XML codec emission remains a future capability. (`docs/SDK-CAPABILITIES.md` native profiles + "Values and models", "Validation", "Directions".) |
| Operations | All source operations, unnamed operations, standard/custom methods, tags, deprecation, naming collisions, and source identity. | installed-package | **Implemented**, bounded by admission. Standard GET/POST/PATCH/PUT/DELETE compile into the shared descriptors for all selected operations; OAS 3.2 `additionalOperations` behind a capability, QUERY requires 3.2, TRACE body checks, CONNECT refused with a source-linked diagnostic; unnamed operations behind an explicit capability; duplicate `operationId` is an error; summary/description/deprecated/tags/externalDocs metadata is carried in the descriptors. The staged OpenRouter comparison emits all 103 operations in saved source per backend, and installed consumers exercise the maintained slice — this is operation coverage for those inputs, not complete conformance. (`crates/suspect-codegen/src/http_protocol/planner.rs`; `docs/SDK-CAPABILITIES.md`.) |
| Servers and security | Server variables/relative bases, security overrides and OR/AND alternatives, API keys, HTTP schemes, mutual TLS, OAuth/OIDC, and required transport capabilities. | installed-package | **Implemented** except mutual TLS. Server objects plan variables/enums/defaults, relative bases resolve against the document URL (explicit `documentURL`/`serverURL` for local files), per-call and per-client selection; security plans OR alternatives with AND requirements, per-call alternative selection, anonymous alternatives, and explicit credential hooks; credential kinds are bearer, basic, and API key (header/query/cookie); oauth2/OpenID Connect compile to lifecycle descriptors with caller credential hooks (no acquisition/refresh today — see [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md) §5); `mutualTLS` is **refused** with a source-linked diagnostic (`http-security-mutual-tls`). Environment-credential bindings are qualified for all twelve adapters (`docs/SDK-CREDENTIAL-ENV.md`). (`crates/suspect-codegen/src/http_protocol/servers.rs`, `security.rs`; `docs/SDK-CAPABILITIES.md` "HTTP".) |
| Parameters | Path/query/header/cookie/querystring locations, styles, explode, content-based parameters, encoding, and exact wire names. | installed-package | **Implemented.** Shared descriptors cover path/query/header/cookie parameters with scalar, scalar-array and flat-object serialization, source styles, UTF-8 percent-encoding and `allowReserved` hazards, exact wire names, header parameters passed without invented quoting, declared cookie strategies, content-based parameters (`ParameterSerialization::Content`), and optional empty form-query array omission. Framing collisions, ambiguous delimiters, control characters and required empty composites fail before transport. Per-style/per-location conformance beyond the witnessed profiles is bounded by admission. (`crates/suspect-codegen/src/http_protocol/parameters.rs`; `docs/SDK-CAPABILITIES.md` "HTTP".) |
| Media | JSON, text, binary, forms, multipart including positional/nested/streamed encodings, XML, media negotiation, and extension media adapters. | installed-package | **Implemented** for JSON, text, binary, forms, and multipart. Form/multipart inputs have native fields and per-part codecs with repeated-field arrays, mixed binary aggregates, positional multipart preserving prefix/item order, part metadata (`{data, headers, contentType, filename}`), and multi-media parts requiring `contentType`; response media negotiation selects exact/range/default then most-specific declaration with wildcard narrowing (`contentType`/`mediaType`/`rawContentType` in TypeScript); undeclared success bodies are bounded raw bytes. Streamed multipart and nested transfer encodings are **refused**, not silently degraded. XML media is verified **refused, not decoded**: a declared XML structure — any typed schema, properties map, or `$ref` into one, under `application/xml` or a vendor `+xml` type — fails planning before artifacts with a source-linked binary-policy diagnostic (`http-binary-schema-type`, `http-binary-schema-keyword`, or `http-binary-legacy-marker`), an unrepresentable XML sibling refuses the whole response even next to a JSON declaration, and XML media without a representable schema is admitted only as bounded raw bytes (`Representation::Binary`), never parsed or serialized XML. Mixed JSON/XML negotiation picks JSON for a JSON Content-Type. This is the documented v1 contract: fail-loud, not fail-silent; full XML codec emission is a future capability. Extension media adapters: **unverified**. (`crates/suspect-codegen/src/http_protocol/media.rs`, `bodies.rs`; verified by `crates/suspect-codegen/tests/sdk_xml_media.rs`; `docs/SDK-CAPABILITIES.md` "HTTP".) |
| Responses | Exact/range/default dispatch, bodyless responses, multiple success representations, typed headers, links, and unexpected response handling. | installed-package | **Implemented.** Exact status before range before default; success depends on the actual 200–299 status; HEAD and body-forbidden statuses return no data; multiple success representations dispatch by media; typed response headers plan with codecs and required validation; response links (`LinkPlan`) are planned as source-backed metadata and **trigger no calls**; undeclared bodies are bounded bytes; unexpected responses raise the package's typed error family with HTTP context preserved. (`crates/suspect-codegen/src/http_protocol/model.rs` `ResponsePlan`/`LinkPlan`; `docs/SDK-CAPABILITIES.md` "HTTP".) |
| Sequential content | SSE, JSON-lines/other described sequential media, complete-content versus item schemas, item metadata, and binary streams. | native runtime | **Partial.** Generation planning is landed: OAS 3.2 `itemSchema` with an SSE or JSON-lines framing compiles a `StreamPlan` (framing, typed item codec, bounded item bytes), with explicit refusals for 3.1 complete-content schemas, aggregate schemas, and unsupported sequential media such as RFC 7464 JSON text sequences. Native consumption is emitted per language (AsyncIterable in TypeScript, generators/iterators in Python, Go and Rust stream runtimes, Swift sequences, Kotlin flows, and the other backends' stream helpers), currently yielding parsed SSE field envelopes with string `data` and one JSON value per JSON-lines record. Typed payload/completion semantics — decoded typed events from `data`, sentinel handling, completion/usage preservation — are **planned** (the dedicated stream-plan module `src/http_protocol/stream_plan.rs` does not exist in this tree; the interim `StreamPlan` lives in `crates/suspect-codegen/src/http_protocol/model.rs` and is compiled in `bodies.rs`). (`crates/suspect-codegen/src/http_protocol/bodies.rs`, `model.rs`; per-backend `pagination.rs`/stream modules.) |
| Callbacks and webhooks | Correct incoming direction; native typed request decoding/response construction and framework adapters, plus the described runtime expressions. | generation | **Not supported as client operations; planned.** Callbacks and webhooks are indexed in the Contract, structure-validated (a `callbacks` field must be a map), included in `operationId` uniqueness checks, and then **explicitly excluded from client operation planning** with a source-linked explanation: "callbacks, webhooks and unindexed methods are not client operations" (`http-operation-not-found`). Provider-initiated operations remain in the Contract for future incoming-direction support; no native typed request decoding, response construction, or framework adapters are emitted today. (`crates/suspect-codegen/src/http_protocol/planner.rs` lines 57–78 and 169–178.) |
| Documentation and artifacts | Examples, descriptions, external documentation, native references, package metadata, deterministic regeneration, and compatibility reports. | installed-package | **Implemented.** Each profile emits its native documentation artifacts (TSDoc/TypeDoc, Sphinx docstrings with `py.typed`, Go doc, DocC, Rustdoc with doctests, Javadoc, compiler XML documentation with native reference pages, Dokka, YARD/RBS/Steep, PHPStan/PHPDoc references, dartdoc, Doxygen) plus package metadata and executable examples; examples carry source/synthesized provenance with located invalid-example findings; artifact sets are deterministic with ownership-aware drift checks; `codegen-compare` produces compatibility reports across source snapshots. (`docs/SDK-CAPABILITIES.md` native profiles and "Examples", "Artifacts", "Compatibility".) |

## Honest-status notes

- The statuses above deliberately stop short of the plan's 100% completion
  language. Operation counts, retained metadata, and raw-body fallbacks do not
  establish complete runtime support, and this matrix does not treat them as
  such.
- Rows marked **implemented** are bounded by source-linked admission: an
  unsupported selected feature produces a located diagnostic before artifacts
  are written, never a silent raw-body fallback.
- XML is the concrete, test-verified instance of that rule: neither XML media
  nor `xml` annotations are interpreted today, and both fail loudly (or stay
  explicitly untyped as bounded bytes) rather than appearing supported. The
  current contract is fail-loud, not fail-silent, and full XML codec emission
  is a named future capability.
- The remaining gaps with named milestones: typed SSE payload/completion
  runtime (M4), callbacks/webhooks incoming support (M8), OpenAPI 2.0 SDK
  frontend (M8), mutual TLS transport capability (M8), and per-target matrix
  closure with installed-package evidence for every row (M7/M8).

## Sources

- Plan and scope: [SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md) §13.
- Capability baselines: [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md).
- Credential runtime: [SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md).
- User-facing behavior: [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md).
- Implementation: `crates/suspect-codegen/src/http_protocol/` (planner,
  `model.rs`, `bodies.rs`, `parameters.rs`, `media.rs`, `responses.rs`,
  `servers.rs`, `security.rs`, `pagination.rs`, `oauth.rs`).
