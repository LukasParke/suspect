# Drop-in replacement review: suspect SDKs vs the published OpenRouter SDKs

Date: 2026-09-16

Inputs: the published npm `@openrouter/sdk` 1.2.133 and PyPI `openrouter` 1.1.153
(both Speakeasy-generated from the upstream generator's own configuration),
diffed mechanically against SDKs generated at this revision from
`openrouter-web/packages/sdk-generation/openapi-assembled.json` (with one
documented local repair: the upstream export drops
`components.securitySchemes` while declaring document security — restored as
the HTTP-bearer `apiKey` scheme the published SDKs implement).

Detailed per-language diffs: `SDK-COMPAT-DIFF-TYPESCRIPT.md` and
`SDK-COMPAT-DIFF-PYTHON.md`. This document is the synthesis: what it would take
to make suspect's generated SDKs drop-in replacements, using **general
generation policies only** — no OpenRouter-specific logic.

## Surface census

- Published TypeScript: 115 methods over 30 resource classes + `callModel`.
- Published Python: 124 sync + 124 async methods over 30 resources.
- Ours: 52 of the 90 spec operations admitted and emitted (45/42 visible
  functions per language; the rest of the published surface splits into
  refused admissions, spec-absent areas, and generator-config-only surface).

## Verdict categories

Every diff verdict maps to exactly one of:

1. **Compatibility profile** — a versioned, general interpretation of real
   upstream-spec dialect used in the wild. These unlock operations; they are
   not OpenRouter-specific because the dialect patterns are common.
2. **`sdk_defaults` policy** — declared interface conventions. A consumer
   targeting drop-in parity sets the same policy values any other API would.
3. **Source arbitration** — the upstream assembled export disagrees with the
   published generator's own source of truth. No generator policy can fix an
   input; these need upstream spec regeneration or a live-wire arbitration
   decision.
4. **Deliberate divergence** — suspect's design is the improvement; keep,
   document, and provide the migration recipe.

## Findings and the policies that close them

### P0 — wire correctness (a drop-in replacement must decode what the published SDK decodes)

| Finding | Class | Resolution |
| --- | --- | --- |
| OAS 3.0 `nullable: true` appears throughout the 3.1 source; published emits nullable types; ours refuses 32 operations and — worse — for admitted ones emits non-null codecs where the wire documents `null` (e.g. `ByokKey.allowed_api_key_hashes`) | Compatibility profile | New `oas30-nullable-in-3.1-v1` profile: interpret `nullable: true` as a null-type union. Same shape as the existing `legacy-binary-string-v1` (which already clears the 3 `format: binary` markers — verified). |
| Colon path templates (`/keys/:hash`) — OAS requires `{hash}`; published handles them; 27 operations refuse | Compatibility profile | `colon-path-parameters-v1`: accept `:name` segments when a declared parameter of that name exists in the same item; refuse when the name is undeclared (never silently guess). |
| Schemaless SSE (6 streaming operations: no itemSchema) — published streams text events; ours refuses (`http-stream-item-schema-required`) so zero operations stream today | Compatibility profile | `schemaless-stream-events-v1`: admit schemaless `text/event-stream` as text-frame events (the typed-stream machinery already handles declared itemSchemas; schemaless lowers to text payloads + unknown-kind alternative). |
| Published request/response types carry a `{"result": …}` wrapper on ≥11 list endpoints that the assembled spec does not declare; our list models would decode the wrong shape if the wire carries it | Source arbitration | Needs live-wire arbitration or upstream spec regeneration. Neither generator can guess; the failure mode is decode-time and typed on our side. |
| Published surface includes parameters the OpenAPI source does not declare (`HTTP-Referer`/`X-OpenRouter-Title`/`X-OpenRouter-Categories` on all 115 methods) — they live in the published generator's config | Policy | New `sdk_defaults.globals`: declare shared parameters (name, in, wire name, required) applied to every operation. Generalizes a real Speakeasy/Stainless pattern. |
| Extension-gated parameters dropped by the assembler (`group_by`, `workspace_id`, benchmark/file-list filters) | Source arbitration | Upstream export regeneration; generator-side: `sdk_defaults.globals`/parameter config can restore specific members, but per-operation gating is source truth. |

### P1 — interface conventions (drop-in ergonomics; all general policy values)

| Finding | Policy | Mechanism |
| --- | --- | --- |
| Composition: `client.credits.getCredits(request?)` vs our `getCredits(client, input?)` / `createClient().credits…` | Offer resource-nested composition grouped by source tag (tags are already carried on the wire plan) as a `sdk_defaults.composition: "resources" | "functions"` policy; keep `createClient` binding credentials once. The plan's M0 decision list named exactly this. |
| Input type naming: `GetCreditsRequest` vs `GetCreditsInput` | `naming.input_suffix` policy value. |
| Param naming: published uses camelCase SDK names with declared wire mappings (`httpReferer` → `HTTP-Referer`); ours uses wire names verbatim | `naming.parameters: {wire: sdkName}` map — the wire mapping stays source-derived, the SDK-native spelling is declared policy. |
| Python optionality: both sides use `UNSET`+`None` (no `NOT_GIVEN`) — verified equal; keep. | Already aligned. |
| Response surface: published returns the bare envelope (`{data: …}`) for single-success operations; ours returns body-first with status/media metadata accessors (`ApiResponse<…>`) | Keep ours (plan §4 improvement); document the migration recipe (`.data` accessor). Verdict: deliberate divergence. |
| Pagination: published Python has NO auto-pagination (manual offset/limit); published TS has Page helpers; ours emits typed lazy pagers on every detected list operation | Keep ours; the published shape is a subset. Verdict: deliberate divergence. |
| Retry policy: published retries 5XX/connection errors with exponential backoff (Python: 5 retries 500 ms→60 s ×1.5); ours never retries | `sdk_defaults.retry` policy (the plan §10 default: ≤2 automatic retries, bounded jitter, `Retry-After`, replay-budget unification with auth replay). |
| Error hierarchy: published has `APIError`/`APIConnectionError`/status subclasses + `SDKValidationError`; ours has one typed `SdkError` family with declared `ApiError` guards | Keep ours (single well-understood family, plan §10); document mapping. |
| Enum forward-compat: published `unrecognized` enum fallbacks | Add a `naming.enums: "unrecognized-fallback"` policy for response enums (plan §10 already specifies typed-unknown for events; extend to enums). |
| Datetime: published TS `Date` (rfcdate/constdatetime), Python `datetime`; ours exact JSON carriers | `naming.datetimes: "native" | "exact"` policy. |

### P2 — cosmetic

Docs-tone differences, README quickstart ordering, package metadata
(`releaseReady: false` flags) — cosmetic, covered by the release checklist.

## Drop-in recipe (general policies only)

1. ~~Land the three compatibility profiles (nullable-in-3.1, colon paths,
   schemaless SSE).~~ **Landed 2026-09-16** (`tests/sdk_compat_profiles.rs`):
   admission against the assembled spec rose 52/90 → **85/90**; verified
   end-to-end (`/keys/{hash}` rendering, chat-streaming generation, OAS-3.0
   nullability decoding). Remaining 5 refusals are upstream source defects
   (an RSS endpoint, an undeclared second scheme, a duplicated operationId,
   two multipart-extras operations).
2. Land `sdk_defaults.globals` + `naming` policies (composition, input suffix,
   param naming with wire maps, retry, enum fallbacks, datetimes). A
   drop-in-targeting consumer sets the documented policy values; no generator
   logic knows OpenRouter.
3. Arbitrate the source drift upstream (the `result` wrapper, extension-gated
   parameters): regenerate the assembled export with the securitySchemes and
   parameter gating included, or decide against the published generator's
   source and keep the spec's. Wire arbitration decides the list-envelope
   question.
4. Keep and document the deliberate divergences (body-first responses, typed
   pagination, single error family, no global mutable state) — these are the
   plan's §1 improvements with migration recipes.

Generator-side, none of these require recognizing OpenRouter: profiles key on
spec dialect patterns, policies on declared configuration, arbitration on the
input document.
