# TypeScript protocol slice: next source-backed expansion

Source analysis begun 2026-09-08, with implementation context updated for the
five-profile pipeline. Corpus facts use openrouter-web revision `db378a2a…`
and the four inputs in [SDK-SESSION-HANDOFF.md](SDK-SESSION-HANDOFF.md#pinned-corpus). Sections
marked **Proposal** are engineering recommendations, not researched facts and
not implemented support.

## 1. Current compiler boundary

- `crates/suspect-codegen/src/http_contract.rs` admits the shared JSON,
  exact-status, static-server and bearer profile. It retains body/response
  media source identities for codecs and documentation. Other media, no-content
  responses, ranges/defaults and typed response headers/links are rejected.
- All five native profiles consume that admission. Their language plans bind
  actual models/codecs to operation inputs, success variants and typed errors.
- Admitted real-source HTTP operations are `getCredits`, `createKeys`,
  `updateKeys`, `listContainerFiles` and `getContainerFile`. The complete current
  matrix is in [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md).

## 2. Corpus census: which real operations exercise each feature

Public YAML line numbers; management/temporal pointed by path.

| Feature | Real operations and pointers |
| --- | --- |
| Exact statuses only | All responses; e.g. `/credits` `getCredits` (30216–30295) declares 200/400/401/403/500. No `default`, no `NXX` ranges exist in any of the four inputs. `5XX` appears only inside `x-speakeasy-retries.statusCodes` (40825) — a vendor extension, not a Responses Object. |
| Multi-media response (JSON + event-stream on one 200) | `sendChatCompletionRequest` (json 29477, event-stream 29478), `createEmbeddings` (31016), `createMessages` (35410), `createImages` (33514), `createRerank` (38561), `createResponses` (38755). |
| Binary/opaque response | `downloadContainerFileContent` (octet-stream, 30014), `downloadFileContent` (31948), `listVideosContent` (video/mp4, 39584); all declare `schema: {type: string, format: binary}`. Management `/models` (4046) 200 declares `application/rss+xml` with a string schema (4426–4431). |
| No-content response | `createCoinbaseCharge` `'200'` with only a description (30295–30297), its sole success status. Temporal `setBenchmarkWorkflowVisibility` POST 404 has no `content` (`packages/temporal/benchmarks.openapi.json`). |
| Request media negotiation | `createAudioTranscriptions` declares two request media: `application/json` (28276) and `multipart/form-data` (28285). `createOauthToken` declares only `application/x-www-form-urlencoded` (36417); `uploadFile` only multipart (31572). |
| Response headers / links / default / ranges | **No operation in any input declares them.** Management `headers:` matches (e.g. 10725) are schema properties, not Response Object headers. |
| Streaming (SSE) | The six operations above declare `text/event-stream` with a complete-content `schema` (e.g. `ChatStreamingResponse`) — not the OAS 3.2 `itemSchema` streamed-items form. `x-speakeasy-sse-sentinel: '[DONE]'` (6×) and `x-speakeasy-stream-request-field: 'stream'` (29678, 35540, 38951) are vendor extensions with no normative meaning. |
| Pagination | `x-speakeasy-pagination` appears 20×; it is vendor extension data. No normative pagination contract is declared. Out of scope here. |

## 3. Source defects vs unsupported compiler semantics

**Source defects (spec-valid or malformed upstream content; do not repair
silently):**

- `createRerank`/`createEmbeddings` declare `text/event-stream` whose schema
  self-describes as unused — "Not used for rerank/embeddings — does not
  support streaming" (38563–38568, 31018–31021) with example `'data: [DONE]'`.
  A declared media the source says never streams is contradictory input.
- `createCoinbaseCharge` declares a `200` the description says "will never
  return" (30296). Declared-success-without-schema is valid OAS; the
  never-returns fact is prose only.
- `x-speakeasy-sse-sentinel` / `-stream-request-field` / `-pagination` /
  `-retries` are extension metadata. Ordinary specifications must not require
  them. The accepted plan permits an explicit, versioned and independently
  verified compatibility profile for extension semantics; current standard-only
  admission does not activate these extensions or infer their transformations.
- Known example defects persist (models response missing `total_count`/`links`;
  PATCH-key example missing `external_user`).

**Unsupported compiler semantics (correct strict rejection today; expansion
requires normative vectors first):** event-stream framing, multipart
`encoding` maps, form-urlencoded request encoding, response headers/links,
default/range status precedence, no-content and binary responses. None of
these may be admitted because a source name suggests them.

## 4. Shared HTTP interface changes (Proposal)

Goal: let one backend expand without silently widening any other backend's
admission. The strict JSON/exact-status profile stays the baseline; every new
shape is an explicit capability each of the five backends must declare.

1. `http_contract::Response` → `media: ResponseMedia` where
   `ResponseMedia::Json(SchemaId) | ResponseMedia::Opaque { media_type:
   String } | ResponseMedia::None`. `status` stays `u16` exact; `default`
   and ranges are **not** added until independent normative vectors exist
   (§5). `Opaque` requires a string/`format: binary` schema or no schema;
   decoded value is raw bytes, never a guessed model.
2. Response matching becomes (status, media) pair matching. Admitting several
   media on one status does not imply content negotiation beyond exact
   Content-Type equality; anything else stays `UnexpectedResponse`.
3. Capability policy stays explicit per backend: a `BackendCapabilities`
   table in `http_contract` (JSON responses, opaque bytes, no-content,
   request-media kinds) that each emitter must satisfy; unsupported shapes
   still produce the existing source-linked diagnostics, with event-stream
   requests rejected by a dedicated `http-streaming-unsupported` diagnostic
    instead of the generic media diagnostic. Other backends keep their bounded
    capability sets until their own gates exist; sharing a shape does not
    establish native support.
4. Request body: `Body::media` becomes the declared media list; the TS input
   type is a caller-selected media choice (§6). Multipart and form encodings
   are separate jobs with their own codecs; `encoding` maps remain rejected.

## 5. Where the corpus lacks vectors (independent fixtures required)

No input exercises: response `default`, status classes (`2XX`), Response
Object `headers`, `links`, OAS 3.2 `itemSchema` streaming. If precedence
(exact > class > default; OAS 3.2 Responses Object) is ever implemented, it
must start from a synthetic, clearly labeled normative/adversarial fixture
set (exact-over-class, class-without-exact, default-only, default ignored
when exact exists, malformed range keys) — never presented as corpus-derived.

## 6. Example native TypeScript callsites (Proposal — no support claimed)

```ts
// Status narrowing (already implemented, createKeys):
const created = await createKeys(client, { body: { name: "CI key" } });
if (created.status === 201) { const key = created.data.key; }
// created: CreateKeysSuccess, with the actual allocated source model.

// Multi-media response, exact Content-Type match (sendChatCompletionRequest):
// ApiResponse<Models.ChatResult, 200, "application/json">
// Event-stream alternative stays a compile-time-admitted *declaration*
// only after streaming gates exist; today it must be a diagnostic, not a type.

// No-content response (createCoinbaseCharge '200', schema-free):
declare const chargeInput: CreateCoinbaseChargeInput;
const r = await createCoinbaseCharge(client, chargeInput);
if (r.status === 200) { /* a proposed void data representation */ }
// Proposed r: ApiResponse<void, 200, "none">. Source body-less declarations
// require an explicit response-consumption policy and bounded wire tests.

// Opaque bytes (downloadFileContent 200 octet-stream, bounded by maxResponseBytes):
declare const downloadInput: DownloadFileContentInput;
const file = await downloadFileContent(client, downloadInput);
const bytes: Uint8Array = file.data; // no decoding, no text assumption

// Request media negotiation (createAudioTranscriptions, two declared media):
declare const jsonTranscription: Models.STTRequest;
await createAudioTranscriptions(client, {
  body: { contentType: "application/json", value: jsonTranscription },
});
// multipart alternative arrives only with the multipart encoding job.
```

## 7. Implementation jobs (Proposal — independent, each with its own gates)

1. **Shared shapes + capability table** (§4.1–4.3): `ResponseMedia`, (status,
   media) matching, `BackendCapabilities`; all existing five-profile plans and
   tests unchanged in behavior.
2. **No-content responses** (TS first, then Rust): exercised by
   `createCoinbaseCharge` 200 and temporal `setBenchmarkWorkflowVisibility`
   404; success/error unions admit `ApiResponse<void, S>`.
3. **Opaque byte responses** (TS): `downloadContainerFileContent`,
   `downloadFileContent`, `listVideosContent`; bytes pass through the
   existing `maxResponseBytes` bound; no streaming claim.
4. **Form-urlencoded request encoding** (TS): `createOauthToken` with
   `TokenExchangeRequest`; RFC 3986 escaping identical to the query policy.
5. **Multipart request encoding** (TS): `createAudioTranscriptions`; binary
   file part + scalar fields; `encoding` maps still rejected with
   diagnostics.
6. **Diagnostic refinement**: event-stream on any response →
    `http-streaming-unsupported` with the media's source pointer. Standard-only
    mode does not activate `x-speakeasy-*`; an explicitly versioned compatibility
    profile requires its own framing, request-flag and sentinel gates.
7. **Independent precedence fixtures** (only before any default/range job):
   the §5 vector set, validated against the Responses Object text.

Jobs 2–6 each land behind `BackendCapabilities`; additional backends adopt them
only with their own wire/docs gates. No job infers behavior from operation names.

## 8. Minimal strict wire/docs test matrix (Proposal)

| Case | Wire expectation | Docs expectation |
| --- | --- | --- |
| Declared exact 200 JSON | status/body/headers echoed; codec validates | status, media, model listed |
| Declared 4xx JSON | branded `DeclaredApiError`, `response.status` exact | error model per status |
| Undeclared status (e.g. 403 on an op without it) | `UnexpectedResponseError` + bounded capture | "other statuses fail" stated |
| Declared 200 with second media | matches only on exact Content-Type essence | both media listed |
| Binary 200 | `Uint8Array` equals sent bytes; limit truncation errors | "raw bytes, bounded" stated |
| No-content declared status | explicit bounded empty/unspecified-body policy; no fabricated model | policy and proposed void representation stated |
| Form-urlencoded request | body bytes match RFC 3986 encoding exactly | encoding documented |
| Multipart request | exact boundary, part order, binary part | encoding documented |
| Event-stream declared | emission-time `http-streaming-unsupported`; nothing emitted | operation absent, not "supported" |
| Default/2XX fixtures (synthetic) | exact wins over class, class over default | fixture-labeled, not corpus |

## 9. Non-claims

This document adds no feature support. All §6 signatures and §7 jobs are
proposals. Streaming framing, pagination, and header typing remain
unimplemented and must keep source-linked diagnostics until their normative
vectors and gates exist. No builds, tests, or gates were run for this brief.
