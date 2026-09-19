# SDK Compatibility Diff — TypeScript (Published `@openrouter/sdk` 1.2.133 vs generated `@openrouter/sdk-compat`)

Review document. Every claim below was read directly from the two trees:

- **PUBLISHED**: `/tmp/openrouter-published/package` — npm tarball `@openrouter/sdk` 1.2.133.
- **OURS**: `/Users/luke/github/suspect/target/sdk-compat-full/typescript` — generated from `/tmp/openrouter-assembled-repaired.json` (the upstream assembled spec with a documented local `securitySchemes` repair), admission inventory `/tmp/admission-inventory.json`.

## 0. Source identification (corrects two assumptions in the tasking)

1. **The published tarball is Speakeasy-generated, not Stainless-generated.** Evidence: `esm/_speakeasy/` metadata directory; `esm/funcs/`, `esm/hooks/`, `esm/core.d.ts`, `esm/lib/` layout; `esm/types/{unrecognized,rfcdate,constdatetime,fp,blobs,enums}.d.ts`; `SDK_METADATA` in `esm/lib/config.d.ts` reads `speakeasy-sdk/typescript 1.2.133 2.914.0 1.0.0 @openrouter/sdk`; `zod ^3.25 || ^4` is the only runtime dependency. No `x-stainless-*` (or `x-speakeasy-*`) request headers exist anywhere in `esm/**/*.js` (grep-verified); the only generator telemetry is the overridable `user-agent` header (`esm/lib/sdks.js:145`).
2. **Our generated package has no `incoming.ts` and no webhook surface.** There is no `incoming.ts` in the tree (file list verified) and no webhook verb/verification code in any generated module. The published side has none either — the only "webhook" hits are `models/observabilitywebhookdestination.*` (a *model* for observability destinations, not a webhook-verification API). Webhook comparison is therefore: **absent on both sides**.

The assembled spec is OpenAPI **3.1.0**, 70 paths, 66 operationId-bearing operations (2 duplicate `getCurrentKey`, see §d). The published SDK's generator consumed a different (richer/newer) OpenRouter document; several drift points are flagged below where they change wire-visible behavior.

---

## a. Executive summary

The two SDKs expose the same upstream API through structurally incompatible facades. Ranked deltas that matter for drop-in compatibility:

| # | Delta | Severity |
|---|-------|----------|
| 1 | **74 of 115 published methods have no counterpart on our side.** 39 are blocked by generator admission refusals (38 refused spec operations; 2 endpoints back 2 published methods each), 10 were admitted in the inventory but never emitted (all path-keyed, i.e. no `operationId` in the assembled spec), and 25 belong to API areas entirely absent from the assembled spec (SCIM, vault, containers, session cost, OAuth JWKS/token-exchange, `callModel`). | **P0** |
| 2 | **Response envelope drift on list endpoints.** The published SDK declares a `{"result": <model>}` wire wrapper for at least 11 list operations (`GetAppRankingsResponse = { result: models.AppRankingsResponse }`, `ListBYOKKeysResponse = { result: ... }`, `ListFilesResponse`, `ListGuardrailsResponse`, `ListKeyAssignments/MemberAssignments(×2)`, `ListObservabilityDestinationsResponse`, `ListOrganizationMembersResponse`, `ListPresetsResponse`). Our side has **zero** `result` wire properties (grep: 0 in `models.ts`); the assembled spec 200-response for e.g. `/api/v1/byok` is a bare `$ref` to `ListBYOKKeysResponse`. If the live wire carries `result`, our list ops decode the wrong shape. Needs live-wire arbitration. | **P0 (potential)** |
| 3 | **Input fields present in the published SDK are absent from ours** on the same operations (published generator read a richer spec): `groupBy`/`workspaceId` on activity; 4 of 9 benchmark filters; 5 of 8 file-list parameters (`provider`, `after`, `afterId`, `beforeId`, `order`) and `provider` on file delete/get/download; `workspaceId` on guardrails list. | **P0** |
| 4 | **Composition**: nested resource classes (`client.credits.getCredits()`) vs flat functions bound by `createClient` (`client.getCredits()`). | **P1** |
| 5 | **Parameter naming**: published uses SDK-native camelCase fields with a `$Outbound` wire mapping layer (e.g. `creatorUserId` → wire `creator_user_id`); ours exposes wire names directly (`api_key_hash`, `start_date`, `file_id`). | **P1** |
| 6 | **Globals**: every published request carries `httpReferer`/`appTitle`/`appCategories` (wire: `HTTP-Referer`, `X-OpenRouter-Title`, `X-OpenRouter-Categories`) with client-level defaults and env fallback; ours has none (headers are not in the assembled spec at all). | **P1** |
| 7 | **Numeric policy**: ours maps 64-bit integers to `bigint` and decimal-typed values to exact `JsonNumber`; published uses plain `number` everywhere. | **P1** |
| 8 | **Retry/timeout policy**: published ships default exponential-backoff retries (`5XX` codes) and per-call timeouts; ours does neither by design (no retries, caller `AbortSignal` only). | **P1** |
| 9 | **Streaming**: published `chat.send`/`responses.send`/`beta.responses.send`/`images.generate` expose `EventStream<...>` overloads; our side has the machinery (`http/streams.ts` SSE + JSON-lines) but **zero** streaming operations (all were refused: `http-stream-item-schema-required`). | **P1** |
| 10 | **Error typing**: published throws 21 typed error classes (per-status, `SDKValidationError`, `ResponseValidationError`, client-error family) and internally returns a `Result` monad; ours throws one `ApiError` (carrying the full `ApiResponse`) plus a 7-kind `SdkError` family, with per-operation `is<Op>ApiError` guards. | **P1/P2** |
| 11 | **Model views**: published camelCase + `Date`/`RFCDate` + `Unrecognized` enum fallback + unknown fields stripped (zod); ours wire-name properties + raw strings + `(string & ("a"|"b"))` open enums + `[key: string]: JsonValue` extras preserved + nullable-dropped static views. | **P1/P2** |

What matches well: the HTTP surface where both sides generated (method/verb/paths), the bearer auth wire form (`Authorization: Bearer <token>`), the `{data}` envelope for single-success read/write endpoints that both specs declared, declared-error narrowing (theirs per-status classes, ours per-op guards), and the byte/binary download surface.

Counts: **115 published methods compared — 14 MATCH, 27 NAME-ONLY-DIFF, 74 MISSING-OUR-SIDE, 1 EXTRA-OUR-SIDE** (plus 3 our-side pagination helpers and the `createClient` bundle as surface additions).

---

## b. Client construction & lifecycle

### Published — `new OpenRouter(SDKOptions)` (`esm/lib/config.d.ts`, `esm/lib/sdks.d.ts`)

```ts
export type SDKOptions = {
    apiKey?: string | (() => Promise<string>) | undefined; // env: OPENROUTER_API_KEY
    httpReferer?: string | undefined;   // SDK global → wire header "HTTP-Referer"
    appTitle?: string | undefined;      // SDK global → wire header "X-OpenRouter-Title"
    appCategories?: string | undefined; // SDK global → wire header "X-OpenRouter-Categories"
    httpClient?: HTTPClient;            // injectable transport (below)
    server?: keyof typeof ServerList | undefined;      // "production"
    serverURL?: string | undefined;     // env: OPENROUTER_BASE_URL
    userAgent?: string | undefined;     // default SDK_METADATA.userAgent
    retryConfig?: RetryConfig;
    timeoutMs?: number;
    debugLogger?: Logger;
};
export declare const ServerList: { readonly production: "https://openrouter.ai/api/v1" };
```

- **Transport injection**: `httpClient?: HTTPClient` — `new HTTPClient({ fetcher?: Fetcher })`, `Fetcher = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>` (`esm/lib/http.d.ts`). `HTTPClient` also carries `addHook`/`removeHook` for `"beforeRequest" | "requestError" | "response"` and `clone()`.
- **SDK-level hooks** (separate from HTTPClient hooks): `SDKHooks` with `sdkInit`, `beforeCreateRequest`, `beforeRequest`, `afterSuccess`, `afterError` (`esm/hooks/types.d.ts`).
- **Env auto-configuration** (`esm/lib/env.d.ts`): `OPENROUTER_API_KEY`, `OPENROUTER_HTTP_REFERER`, `OPENROUTER_APP_TITLE`, `OPENROUTER_APP_CATEGORIES`, `OPENROUTER_DEBUG`, `OPENROUTER_BASE_URL` fill options when unset (`fillGlobals`).
- **Retries** (default in every generated func, e.g. `esm/funcs/creditsGetCredits.js:48-61`):
  ```js
  retryConfig: options?.retries || client._options.retryConfig ||
    { strategy: "backoff",
      backoff: { initialInterval: 500, maxInterval: 60000, exponent: 1.5, maxElapsedTime: 3600000 },
      retryConnectionErrors: true },
  retryCodes: options?.retryCodes || ["5XX"],
  ```
  Per-call override via `RequestOptions.retries` / `retryCodes`. `RetryConfig = { strategy: "none" } | { strategy: "backoff", backoff?, retryConnectionErrors? }` (`esm/lib/retries.d.ts`).
- **Timeouts**: client-level `timeoutMs`, per-call `RequestOptions.timeoutMs` (ms, implemented as `AbortSignal.timeout`); an explicit `fetchOptions.signal` wins (`esm/lib/sdks.js:151-183`).
- **Per-call `RequestOptions`**: `timeoutMs`, `retries`, `retryCodes`, `serverURL`, plus flattened `RequestInit` (headers, signal, …) — `fetchOptions` is deprecated but still accepted (`esm/lib/sdks.d.ts:8-34`).
- **User agent**: `headers.set(conf.uaHeader ?? "user-agent", conf.userAgent ?? SDK_METADATA.userAgent)` where `SDK_METADATA.userAgent = "speakeasy-sdk/typescript 1.2.133 2.914.0 1.0.0 @openrouter/sdk"`. No other telemetry headers.

### Ours — `createClient(ClientOptions)` (`http/types.ts`, `runtime.ts`, `operations.ts`)

```ts
export interface Credentials { readonly "apiKey": Credential<string>; }
export type ClientCredentials = Partial<Credentials> & (Pick<Credentials, "apiKey">);
export interface ClientOptions extends RuntimeClientOptions<"apiKey", ClientCredentials> { readonly auth: ClientCredentials; }
// RuntimeClientOptions (http/types.ts):
export interface ClientOptions<Auth extends object = ...> {
    readonly auth?: Auth;              // { apiKey: string | (ctx) => string }  (bearer scheme named "apiKey")
    readonly serverURL?: string;
    readonly server?: ServerChoice;    // { index?, name?, variables?, documentURL? } — server-array selection
    readonly fetch?: Fetch;            // plain Fetch replacement, no wrapper class
    readonly maxResponseBytes?: number; readonly maxRequestBytes?: number; readonly maxPartBytes?: number;
    readonly maxStreamItemBytes?: number; readonly maxStreamBufferBytes?: number; readonly maxStreamItems?: number;
    readonly maxErrorCaptureBytes?: number;
    readonly userAgent?: string | null;   // null suppresses entirely
    readonly applicationId?: string;      // replaces the SDK-identity token in the UA
}
export interface CallOptions {
    readonly signal?: AbortSignal;        // cancellation
    readonly server?: ServerChoice;       // per-call server override
    readonly securityAlternative?: number; // per-call security alternative
}
```

- **Transport injection**: `fetch?: Fetch` on options; `runtime.ts:192-204` uses `client.fetch ?? globalThis.fetch`, forces `redirect: 'error'` and `credentials: 'omit'`, and — when using the native global fetch — runs a representability preflight that rejects URLs/headers/methods native Fetch would normalize.
- **Base URL**: single server candidate `https://openrouter.ai/api/v1` from the spec's `servers` array; selection via `serverURL` option or `server`/`call.server` (`http/security.ts` `serverURL`). No environment-variable fallback, no named `ServerList`.
- **Retries/timeouts**: none. `executeOperation` is one exchange ("no retries" per `runtime.ts:106`); cancellation is the caller's `AbortSignal` (`CallOptions.signal`); finite byte budgets (`max*Bytes`, request 8 MiB, response 8 MiB, part 1 MiB, stream item 64 KiB / buffer 128 KiB / 100 000 items — descriptor `limits`).
- **Auth wire form**: scheme name `apiKey`, `kind: "bearer"` → `Authorization: Bearer <value>` with an RFC 6750 token validation (`http/security.ts:81-85`). Static value or a `CredentialProvider` callback (no acquisition/retry). The scheme itself is the **documented local repair** in the assembled spec: `components/securitySchemes/apiKey/description` = *"Repaired locally for the compatibility diff: the upstream assembled export drops securitySchemes while declaring security requirements."*
- **User agent**: `resolveUserAgent` (`runtime.ts:47-55`) → `suspect/<suspect_version> <sdk_name>/<sdk_version> (<language>/<node>; openapi/<spec_version>)`, overridable (`userAgent`) or suppressible (`userAgent: null`), with `applicationId` spliced in as `<name>/<version>`.

### Option-name mapping table

| Concept | Published (`SDKOptions` / `RequestOptions`) | Ours (`ClientOptions` / `CallOptions`) |
|---|---|---|
| Credential | `apiKey?: string \| (() => Promise<string>)` | `auth.apiKey: string \| ((ctx: CredentialContext) => string \| Promise<string>)` |
| Transport | `httpClient?: HTTPClient` (+ `HTTPClientOptions.fetcher`) | `fetch?: Fetch` (direct) |
| Hooks | `HTTPClient.addHook(...)` + `SDKHooks` (5 lifecycle hooks) | none |
| Base URL | `serverURL` (or `server: "production"`, env `OPENROUTER_BASE_URL`) | `serverURL` (or `server`/`call.server` `ServerChoice`) |
| Retries | `retryConfig` + per-call `retries`, `retryCodes`; default backoff 500 ms→60 s, 1.5×, 1 h cap, `5XX` | none |
| Timeout | `timeoutMs` (client + per call) | caller `AbortSignal` only |
| Globals | `httpReferer`, `appTitle`, `appCategories` (+ env) | absent |
| UA | `userAgent` (string) | `userAgent: string \| null`, `applicationId` |
| Limits | none (unbounded by default) | six `max*Bytes` ceilings |
| Debug | `debugLogger?: Logger` | none |

---

## c. Composition

**Published**: `class OpenRouter extends ClientSDK` with 30 lazy resource getters (`analytics`, `apiKeys`, `benchmarks`, `beta` → `beta.responses`, `byok`, `chat`, `classifications`, `containers`, `credits`, `datasets`, `embeddings`, `endpoints`, `files`, `generations`, `guardrails`, `images`, `models`, `oAuth`, `observability`, `organization`, `presets`, `providers`, `rerank`, `responses`, `scim`, `stt`, `tts`, `vault`, `videoGeneration`, `workspaces`) plus a root `callModel(...)`. Each resource class extends `ClientSDK`, methods take `(request: operations.XRequest, options?: RequestOptions)` and `unwrapAsync` the internal `Result` (throwing typed errors).

**Ours**: 42 top-level `export function <opId>(client: ClientOptions, input: <Op>Input, call?: CallOptions): Promise<<Op>Success>` in `operations.ts`, each delegating to `executeOperation(<opId>Descriptor, input, client, call)`; `createClient(options)` returns a bound bundle `{ getUserActivity: (input?, call?) => getUserActivity(client, input, call), ... }` (42 entries). Errors are **thrown** (`ApiError` / `SdkError` family) — the returned promise resolves only on a declared 2xx; there is no `Result` monad and no `Promise` subclass.

### Side-by-side call examples

1. **Read with envelope — getCredits**

   Published (`esm/sdk/credits.d.ts`, `esm/models/operations/getcredits.d.ts`):
   ```ts
   const res = await openrouter.credits.getCredits({ appTitle: "my-app" }); // request?: GetCreditsRequest | undefined
   res.data.totalCredits;   // GetCreditsResponse = { data: GetCreditsData }  — body envelope preserved
   ```
   Ours (`operations.ts:431-436`, `models.ts:9146`):
   ```ts
   const res = await getCredits(client);            // input: GetCreditsInput = {}
   res.data.data.total_credits;                     // ApiResponse<GetCreditsResponse200, 200, "application/json">
   // res.data = the decoded body { data: { total_credits: JsonNumber, total_usage: JsonNumber } }
   // res.status / res.contentType / res.mediaType / res.headers / res.typedHeaders / res.links
   ```
   Note the extra `data` hop on our side: our `Success` type is the **ApiResponse record** (status/media/headers + `data` = decoded body); the published method returns the decoded body directly. Same applies to the returned value of every operation.

2. **Write — create API key** (published) vs nearest our-side write (workspace)
   Published create-keys exists; ours **does not** (see §d: admitted but not emitted), so the write-op comparison uses `createWorkspace`:
   ```ts
   // Published
   const ws = await openrouter.workspaces.create({
     httpReferer: "https://app.example",      // globals trio on every request
     createWorkspaceRequest: { name: "eng", slug: "eng", description: null }, // nested body under a named field
   });
   // Ours
   const res = await createWorkspace(client, {
     body: { name: "eng", slug: "eng", description: undefined }, // bare `body` member; wire property names inside
   }, { signal: AbortSignal.timeout(5_000) });
   ```
   Shape deltas on this operation: published wraps the body under `createWorkspaceRequest:` and accepts `description?: string | null | undefined`; ours uses `body?:` and the static type is `"description"?: string` (the spec's `nullable: true` is not surfaced as `| null` — see §f).

3. **Paginated list**
   Published (auto-pagination, 19 methods):
   ```ts
   const it = await openrouter.organization.listMembers({ limit: 100 }); // Promise<PageIterator<ListOrganizationMembersResponse, { offset: number }>>
   for await (const page of it) { page.result.data; }   // or drive `it.next()` manually; `~next` holds page state
   ```
   Ours (only one paginated op emitted):
   ```ts
   for await (const page of listOrganizationMembersPages(client, { limit: 100n })) { page.data.data; }
   for await (const member of listOrganizationMembersItems(client, {})) { member.email; }
   const nextInput = await listOrganizationMembersNextPage(client, { offset: 100n }); // ListOrganizationMembersInput | null
   ```
   No `PageIterator` type, no `~next`; the walker rebuilds inputs from a compiled `PaginationDescriptor` (`pagination.ts:231-233`: pattern `limit-offset`, items pointer `/data`, total `/total_count`, `initialOffset: 0`). A repeated continuation value throws `PaginationError` instead of looping.

---

## d. Method inventory (every published method × ours)

Verdict legend: **MATCH** = published method name equals our function name (composition aside); **NAME-ONLY-DIFF** = same upstream operation, different accessor name; **MISSING-OUR-SIDE** = no our-side counterpart, with cause; **EXTRA-OUR-SIDE** = our function with no published counterpart. `http-extension-uninterpreted` appears on every refused operation and is abbreviated `+ext`; other codes are quoted in full.

### analytics (3)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `analytics.getUserActivity` | GET /api/v1/activity | `getUserActivity` | **MATCH** |
| `analytics.getAnalyticsMeta` | GET /api/v1/analytics/meta | `getAnalyticsMeta` | **MATCH** |
| `analytics.queryAnalytics` | POST /api/v1/analytics/query | `queryAnalytics` | **MATCH** |

### apiKeys (6)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `apiKeys.list` | GET /api/v1/keys | `list` | **MATCH** |
| `apiKeys.create` | POST /api/v1/keys | — | **MISSING** — admitted (`POST /api/v1/keys: true`) but path-keyed (no operationId) → not emitted |
| `apiKeys.get` | GET /api/v1/keys/:hash | — | **MISSING** — refused: `http-path-parameters` +ext |
| `apiKeys.delete` | DELETE /api/v1/keys/:hash | — | **MISSING** — refused: `http-path-parameters` +ext |
| `apiKeys.update` | PATCH /api/v1/keys/:hash | — | **MISSING** — refused: `http-path-parameters` +ext |
| `apiKeys.getCurrentKeyMetadata` | GET /api/v1/key (and duplicate GET /api/v1/auth/key) | — | **MISSING** — refused: `http-operation-id-duplicate` +ext (both spec paths carry operationId `getCurrentKey`) |

### benchmarks (1) / classifications (1) / credits (1)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `benchmarks.getBenchmarks` | GET /api/v1/benchmarks | `getBenchmarks` | **MATCH** |
| `classifications.getTaskClassifications` | GET /api/v1/classifications/task | `getTaskClassifications` | **MATCH** |
| `credits.getCredits` | GET /api/v1/credits | `getCredits` | **MATCH** |

### beta (1 nested) / chat (1) / responses (1) / rerank (1) / embeddings (2) / images (3)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `beta.responses.send` | POST /api/v1/responses | — | **MISSING** — refused: `http-stream-item-schema-required` +ext (same spec operation as `responses.send`) |
| `chat.send` | POST /api/v1/messages | — | **MISSING** — refused: `http-stream-item-schema-required` +ext (two inventory entries: path-keyed `POST /api/v1/messages` and operationId `sendChatCompletionRequest`) |
| `responses.send` | POST /api/v1/responses | — | **MISSING** — refused: `http-stream-item-schema-required` +ext |
| `rerank.rerank` | POST /api/v1/rerank | — | **MISSING** — refused: `http-stream-item-schema-required` +ext |
| `embeddings.generate` | POST /api/v1/embeddings | — | **MISSING** — refused: `http-stream-item-schema-required` +ext |
| `embeddings.listModels` | GET /api/v1/embeddings/models | — | **MISSING** — admitted but path-keyed → not emitted |
| `images.generate` | POST /api/v1/images | — | **MISSING** — refused: `http-stream-item-schema-required` +ext |
| `images.listModels` | GET /api/v1/images/models | `listImageModels` | **NAME-ONLY-DIFF** |
| `images.listModelEndpoints` | GET /api/v1/images/models/{author}/{slug}/endpoints | `listImageModelEndpoints` | **NAME-ONLY-DIFF** |

### byok (5) / endpoints (2) / files (5) / generations (3)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `byok.list` | GET /api/v1/byok | `listBYOKKeys` | **NAME-ONLY-DIFF** |
| `byok.create` | POST /api/v1/byok | `createBYOKKey` | **NAME-ONLY-DIFF** |
| `byok.get` | GET /api/v1/byok/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `byok.delete` | DELETE /api/v1/byok/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `byok.update` | PATCH /api/v1/byok/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `endpoints.list` | GET /api/v1/models/{author}/{slug}/endpoints | `listEndpoints` | **NAME-ONLY-DIFF** |
| `endpoints.listZdrEndpoints` | GET /api/v1/endpoints/zdr | — | **MISSING** — admitted but path-keyed → not emitted |
| `files.list` | GET /api/v1/files | `listFiles` | **NAME-ONLY-DIFF** |
| `files.upload` | POST /api/v1/files | — | **MISSING** — refused: `http-form-untyped-extras`, `http-compatibility-profile` +ext |
| `files.delete` | DELETE /api/v1/files/{file_id} | `deleteFile` | **NAME-ONLY-DIFF** |
| `files.retrieve` | GET /api/v1/files/{file_id} | `getFileMetadata` | **NAME-ONLY-DIFF** |
| `files.download` | GET /api/v1/files/{file_id}/content | `downloadFileContent` | **NAME-ONLY-DIFF** |
| `generations.getGeneration` | GET /api/v1/generation | — | **MISSING** — admitted but path-keyed → not emitted |
| `generations.listGenerationContent` | GET /api/v1/generation/content | — | **MISSING** — admitted but path-keyed → not emitted |
| `generations.submitFeedback` | POST /api/v1/generation/feedback | `submitGenerationFeedback` | **NAME-ONLY-DIFF** |

### guardrails (13)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `guardrails.list` | GET /api/v1/guardrails | `listGuardrails` | **NAME-ONLY-DIFF** |
| `guardrails.create` | POST /api/v1/guardrails | `createGuardrail` | **NAME-ONLY-DIFF** |
| `guardrails.get` | GET /api/v1/guardrails/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `guardrails.update` | PATCH /api/v1/guardrails/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `guardrails.delete` | DELETE /api/v1/guardrails/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `guardrails.listGuardrailKeyAssignments` | GET /api/v1/guardrails/{id}/assignments/keys | `listGuardrailKeyAssignments` | **MATCH** |
| `guardrails.bulkAssignKeys` | POST /api/v1/guardrails/{id}/assignments/keys | `bulkAssignKeysToGuardrail` | **NAME-ONLY-DIFF** |
| `guardrails.bulkUnassignKeys` | POST /api/v1/guardrails/{id}/assignments/keys/remove | `bulkUnassignKeysFromGuardrail` | **NAME-ONLY-DIFF** |
| `guardrails.listGuardrailMemberAssignments` | GET /api/v1/guardrails/{id}/assignments/members | `listGuardrailMemberAssignments` | **MATCH** |
| `guardrails.bulkAssignMembers` | POST /api/v1/guardrails/{id}/assignments/members | `bulkAssignMembersToGuardrail` | **NAME-ONLY-DIFF** |
| `guardrails.bulkUnassignMembers` | POST /api/v1/guardrails/{id}/assignments/members/remove | `bulkUnassignMembersFromGuardrail` | **NAME-ONLY-DIFF** |
| `guardrails.listKeyAssignments` | GET /api/v1/guardrails/assignments/keys | `listKeyAssignments` | **MATCH** |
| `guardrails.listMemberAssignments` | GET /api/v1/guardrails/assignments/members | `listMemberAssignments` | **MATCH** |

Note the mixed path-parameter spellings in the assembled spec itself: `:id` colon-style on the refused operations vs `{id}` brace-style on the admitted ones (`http-path-parameters` is precisely the colon-style refusal).

### models (4) / observability (5) / organization (1) / presets (7) / providers (1)

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `models.get` | GET /api/v1/model/:author/:slug | — | **MISSING** — refused: `http-path-parameters` +ext |
| `models.list` | GET /api/v1/models | — | **MISSING** — refused: `http-nullable-annotation` ×15, `http-binary-schema-type` +ext |
| `models.count` | GET /api/v1/models/count | — | **MISSING** — admitted but path-keyed → not emitted |
| `models.listForUser` | GET /api/v1/models/user | — | **MISSING** — refused: `http-nullable-annotation`, `http-security-scheme-unresolved` +ext |
| `observability.list` | GET /api/v1/observability/destinations | `listObservabilityDestinations` | **NAME-ONLY-DIFF** |
| `observability.create` | POST /api/v1/observability/destinations | `createObservabilityDestination` | **NAME-ONLY-DIFF** |
| `observability.get` | GET /api/v1/observability/destinations/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `observability.update` | PATCH /api/v1/observability/destinations/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `observability.delete` | DELETE /api/v1/observability/destinations/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `organization.listMembers` | GET /api/v1/organization/members | `listOrganizationMembers` | **NAME-ONLY-DIFF** |
| `presets.list` | GET /api/v1/presets | `listPresets` | **NAME-ONLY-DIFF** |
| `presets.get` | GET /api/v1/presets/:slug | — | **MISSING** — refused: `http-path-parameters` +ext |
| `presets.createPresetsChatCompletions` | POST /api/v1/presets/:slug/chat/completions | — | **MISSING** — refused: `http-path-parameters` +ext |
| `presets.createPresetsMessages` | POST /api/v1/presets/:slug/messages | — | **MISSING** — refused: `http-path-parameters` +ext |
| `presets.createPresetsResponses` | POST /api/v1/presets/:slug/responses | — | **MISSING** — refused: `http-path-parameters` +ext |
| `presets.listVersions` | GET /api/v1/presets/:slug/versions | — | **MISSING** — refused: `http-path-parameters`, `http-nullable-annotation` +ext |
| `presets.getVersion` | GET /api/v1/presets/:slug/versions/:version | — | **MISSING** — refused: `http-path-parameters` +ext |
| `providers.list` | GET /api/v1/providers | `listProviders` | **NAME-ONLY-DIFF** |

### scim (8) / vault (7) / containers (4) / datasets (3) — absent from the assembled spec

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `scim.listMappings` | GET /scim/… (JWS-protected SCIM API) | — | **MISSING** — no such path in `/tmp/openrouter-assembled-repaired.json` |
| `scim.create` / `scim.delete` / `scim.read` / `scim.update` / `scim.listGroups` / `scim.createSyncJob` / `scim.getSyncJob` | SCIM group/sync endpoints | — | **MISSING** — absent from assembled spec (7 methods) |
| `vault.listVaultSecrets` / `storeVaultSecret` / `deleteVaultSecret` / `listInternVaultSecrets` / `storeInternVaultSecret` / `deleteInternVaultSecret` / `copyVaultSecretsToIntern` | vault secret endpoints | — | **MISSING** — absent from assembled spec (7 methods) |
| `containers.listContainerFiles` / `getContainerFile` / `downloadContainerFileContent` / `promoteContainerFile` | container file endpoints | — | **MISSING** — absent from assembled spec (4 methods) |
| `datasets.getSessionCost` | session-cost endpoint | — | **MISSING** — absent from assembled spec |
| `oAuth.createAuthorizationUrl` | client-side URL builder (no HTTP) | — | **MISSING** — not an HTTP operation; absent from spec by construction |
| `oAuth.createSHA256CodeChallenge` | client-side PKCE helper (no HTTP) | — | **MISSING** — as above |
| `oAuth.listOauthJwks` | JWKS endpoint | — | **MISSING** — absent from assembled spec |
| `oAuth.createOauthToken` | RFC 8693 token exchange | — | **MISSING** — absent from assembled spec |
| `callModel` (root) | composite agent call over POST /api/v1/chat/completions | — | **MISSING** — client-side composite; upstream has moved it to `@openrouter/agent` (published README §"Migrating callModel") |

### stt (2) / tts (1) / videoGeneration (4) / workspaces (12) / oAuth exchange

| Published | HTTP | Ours | Verdict |
|---|---|---|---|
| `stt.createTranscription` | POST /api/v1/audio/transcriptions | — | **MISSING** — refused: `http-form-untyped-extras`, `http-compatibility-profile` +ext |
| `stt.createTranscriptionMultipart` | POST /api/v1/audio/transcriptions | — | **MISSING** — same refused spec operation (second published variant) |
| `tts.createSpeech` | POST /api/v1/audio/speech | — | **MISSING** — admitted but path-keyed → not emitted |
| `videoGeneration.generate` | POST /api/v1/videos | — | **MISSING** — admitted but path-keyed → not emitted |
| `videoGeneration.getGeneration` | GET /api/v1/videos/:jobId | — | **MISSING** — refused: `http-path-parameters` +ext |
| `videoGeneration.getVideoContent` | GET /api/v1/videos/:jobId/content | — | **MISSING** — refused: `http-path-parameters`, `http-nullable-annotation`, `http-compatibility-profile` +ext |
| `videoGeneration.listVideosModels` | GET /api/v1/videos/models | — | **MISSING** — admitted but path-keyed → not emitted |
| `workspaces.list` | GET /api/v1/workspaces | `listWorkspaces` | **NAME-ONLY-DIFF** |
| `workspaces.create` | POST /api/v1/workspaces | `createWorkspace` | **NAME-ONLY-DIFF** |
| `workspaces.get` | GET /api/v1/workspaces/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `workspaces.update` | PATCH /api/v1/workspaces/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `workspaces.delete` | DELETE /api/v1/workspaces/:id | — | **MISSING** — refused: `http-path-parameters` +ext |
| `workspaces.listMembers` | GET /api/v1/workspaces/{id}/members | `listWorkspaceMembers` | **NAME-ONLY-DIFF** |
| `workspaces.bulkAddMembers` | POST /api/v1/workspaces/{id}/members/add | `bulkAddWorkspaceMembers` | **NAME-ONLY-DIFF** |
| `workspaces.bulkRemoveMembers` | POST /api/v1/workspaces/{id}/members/remove | `bulkRemoveWorkspaceMembers` | **NAME-ONLY-DIFF** |
| `workspaces.listBudgets` | GET /api/v1/workspaces/{id}/budgets | `listWorkspaceBudgets` | **NAME-ONLY-DIFF** |
| `workspaces.getBudget` | GET /api/v1/workspaces/{id}/budgets/:interval | — | **MISSING** — refused: `http-path-parameters` +ext |
| `workspaces.setBudget` | PUT /api/v1/workspaces/{id}/budgets/:interval (opId `upsertWorkspaceBudget`) | — | **MISSING** — refused: `http-path-parameters` +ext |
| `workspaces.deleteBudget` | DELETE /api/v1/workspaces/{id}/budgets/:interval | — | **MISSING** — refused: `http-path-parameters` +ext |
| `oAuth.exchangeAuthCodeForAPIKey` | POST /api/v1/auth/keys | `exchangeAuthCodeForAPIKey` | **MATCH** |
| `oAuth.createAuthCode` | POST /api/v1/auth/keys/code | — | **MISSING** — admitted but path-keyed → not emitted |

### EXTRA-OUR-SIDE

| Ours | HTTP | Published counterpart |
|---|---|---|
| `createCoinbaseCharge` (success `ApiResponse<Uint8Array, 200, null>`, declared error 410) | POST /api/v1/credits/coinbase | **none** — no method, no model, no reference in the published tarball (grep-verified). The operation exists in the assembled spec (`/api/v1/credits/coinbase`, admitted) but the published SDK does not expose it. |
| `listOrganizationMembersPages` / `listOrganizationMembersItems` / `listOrganizationMembersNextPage` | — | pagination surface shape differs (theirs: method returns `PageIterator` directly) |
| `createClient` bundle | — | composition difference (§c) |

**Counts**: 115 published methods = **14 MATCH + 27 NAME-ONLY-DIFF + 74 MISSING-OUR-SIDE** (39 refused-by-generator via 38 refused spec operations — `POST /api/v1/responses` and `POST /api/v1/audio/transcriptions` each back two methods — + 10 admitted-but-not-emitted + 25 absent-from-assembled-spec) + **1 EXTRA-OUR-SIDE** operation. Our side: 42 operation functions, 42 `is<Op>ApiError` guards, 3 pagination helpers, `createClient`.

---

## e. Parameter shapes

Conventions, verified from both trees:

- **Theirs**: request types are SDK-native camelCase records. Each `operations.XRequest` embeds the body under a named wrapper field (`requestBody`, `chatRequest`, `createWorkspaceRequest`, `createBYOKKeyRequest`, `id`, `fileId`, …) **plus the globals trio** `httpReferer?`, `appTitle?`, `appCategories?` (with a parallel `XGlobals` type). Outbound serialization runs through a `$Outbound` zod schema + `remap$` that converts to wire names (`creatorUserId` → `creator_user_id`, `expiresAt: Date` → `expires_at: string`, `httpReferer` → header `HTTP-Referer`, `appTitle` → `X-OpenRouter-Title`, `appCategories` → `X-OpenRouter-Categories`; chat/responses also emit `X-OpenRouter-Metadata`). Per-request values override client-level globals: `payload["HTTP-Referer"] ?? client._options.httpReferer` (all 115 methods carry this pattern — verified in `esm/funcs/*.js`).
- **Ours**: `<Op>Input` interfaces use **wire names directly** (verified: `runtime.ts:124-147` reads `data[descriptor.parameterMembers[index]]` where `parameterMembers` are the spec's wire names, and substitutes them into `descriptor.wire.path` / query string / headers verbatim). Request bodies are a bare optional `body?: Models.X` member (`taggedBody: false` for all 42 — no wrapper field, no media-choice tagging). Unknown input members are rejected (`'undeclared operation input member'`, `runtime.ts:116`). Optional empty form-style query arrays are skipped (compat convenience, `runtime.ts:135`).

### Published globals — affected operations

The trio is present on **every** published request type (`XRequest` + `XGlobals` in all 90 `esm/models/operations/*.d.ts`) — i.e. all 115 methods — with client defaults and `OPENROUTER_HTTP_REFERER` / `OPENROUTER_APP_TITLE` / `OPENROUTER_APP_CATEGORIES` env fallback. These headers are **not in the assembled OpenAPI source**; they are published-generator configuration ("SDK globals"). Our side has none of them.

### Per-operation comparison (42 generated ops)

Field rows: `ours` (wire name) / `theirs` (camelCase). Types are SDK-native on both sides; `opt` = optional member. Where the field sets match and only casing/type differ, one row covers both.

| Op | Ours fields (wire) | Theirs fields (camelCase) | Deltas beyond naming |
|---|---|---|---|
| `getUserActivity` | `date?` string, `api_key_hash?` string, `user_id?` string | `date?`, `apiKeyHash?`, `userId?`, **`groupBy?`**, **`workspaceId?`** | 2 params missing ours (assembled spec has 3 params; published source had 5) |
| `getAnalyticsMeta` | — | — + globals trio | none |
| `queryAnalytics` | `body?: Models.QueryAnalyticsRequest` | `request: { httpReferer?, appTitle?, appCategories?, requestBody: QueryAnalyticsRequestBody }` (required positional request) | ours: input fully optional (body optional per spec); theirs: request required; inner model names wire vs camel |
| `exchangeAuthCodeForAPIKey` | `body?: Models.ExchangeAuthCodeForApiKeyRequest` | `requestBody: ExchangeAuthCodeForAPIKeyRequestBody` | wrapper vs `body`; theirs response `{ key: string; userId: string \| null }` vs ours typed 200 model |
| `getBenchmarks` | `source?`, `task_type?`, `arena?`, `category?`, `max_results?` (bigint) | `source?`, `taskType?`, **`benchmarkType?`**, **`includeRunConfig?`**, **`searchEngine?`**, **`searchSurface?`**, `arena?`, `category?`, `maxResults?` | 4 params missing ours |
| `listBYOKKeys` | `offset?` **bigint**, `limit?`, `workspace_id?`, `provider?` | `offset?` **number \| null**, `limit?`, `workspaceId?`, `provider?` | int64 policy: `bigint` vs `number \| null` |
| `createBYOKKey` | `body?: Models.CreateByokKeyRequest` | `createBYOKKeyRequest: models.CreateBYOKKeyRequest` | wrapper vs `body` |
| `getTaskClassifications` | `window?` | `window?` | name-identical |
| `getCredits` | — | — + globals | none |
| `createCoinbaseCharge` | — (success = raw `Uint8Array`, `mediaType: null`) | *n/a (extra)* | — |
| `getAppRankings` | `category?`, `subcategory?`, `sort?`, `start_date?`, `end_date?`, `limit?`, `offset?` bigint | `category?`, `subcategory?`, `sort?`, `startDate?`, `endDate?`, `limit?`, `offset?` number \| null | int64 policy |
| `getRankingsDaily` | `start_date?`, `end_date?`, `period?`, `modality?`, `context_bucket?`, `category?`, `language_type?` | `startDate?`, `endDate?`, `period?`, `modality?`, `contextBucket?`, `category?`, `languageType?` | casing only |
| `listFiles` | `limit?`, `cursor?`, `workspace_id?` | `limit?`, `cursor?`, `workspaceId?`, **`provider?`**, **`after?`**, **`afterId?`**, **`beforeId?`**, **`order?`** | 5 params missing ours |
| `deleteFile` | `file_id` (required), `workspace_id?` | `fileId` (required), `workspaceId?`, **`provider?`** | 1 param missing ours |
| `getFileMetadata` | `file_id`, `workspace_id?` | `fileId`, `workspaceId?`, **`provider?`** | 1 param missing ours |
| `downloadFileContent` | `file_id`, `workspace_id?` | `fileId`, `workspaceId?`, **`provider?`** | 1 param missing ours |
| `submitGenerationFeedback` | `body?: Models.SubmitGenerationFeedbackRequest_971496da3f4566ed` | `submitGenerationFeedbackRequest: models.SubmitGenerationFeedbackRequest` | wrapper vs `body` |
| `listGuardrails` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?`, **`workspaceId?`** | 1 param missing ours; int64 policy |
| `createGuardrail` | `body?: Models.CreateGuardrailRequest_476b25993979e06b` | `createGuardrailRequest: models.CreateGuardrailRequest` | wrapper vs `body` |
| `listKeyAssignments` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?` | int64 policy |
| `listMemberAssignments` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?` | int64 policy |
| `listGuardrailKeyAssignments` | `id` (required), `offset?` bigint, `limit?` | `id` (required), `offset?` number \| null, `limit?` | int64 policy |
| `bulkAssignKeysToGuardrail` | `id` (required), `body?` | `id`, `bulkAssignKeysRequest` | wrapper vs `body` |
| `bulkUnassignKeysFromGuardrail` | `id`, `body?` | `id`, `bulkUnassignKeysRequest` | wrapper vs `body` |
| `listGuardrailMemberAssignments` | `id`, `offset?`, `limit?` | `id`, `offset?`, `limit?` | int64 policy |
| `bulkAssignMembersToGuardrail` | `id`, `body?` | `id`, `bulkAssignMembersRequest` | wrapper vs `body` |
| `bulkUnassignMembersFromGuardrail` | `id`, `body?` | `id`, `bulkUnassignMembersRequest` | wrapper vs `body` |
| `listImageModels` | — | — + globals | none |
| `listImageModelEndpoints` | `author` (req), `slug` (req) | `author`, `slug` | none |
| `list` (API keys) | `include_disabled?`, `offset?` bigint, `workspace_id?` | `includeDisabled?`, `offset?` number \| null, `workspaceId?` | int64 policy |
| `listEndpoints` | `author`, `slug` | `author`, `slug` | none |
| `listObservabilityDestinations` | `offset?` bigint, `limit?`, `workspace_id?` | `offset?` number \| null, `limit?`, `workspaceId?` | int64 policy |
| `createObservabilityDestination` | `body?: …_e84978c55791aaa8` | `createObservabilityDestinationRequest` | wrapper vs `body` |
| `listOrganizationMembers` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?` | int64 policy |
| `listPresets` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?` | int64 policy |
| `listProviders` | — | — + globals | none |
| `listWorkspaces` | `offset?` bigint, `limit?` | `offset?` number \| null, `limit?` | int64 policy |
| `createWorkspace` | `body?: Models.CreateWorkspaceRequest_11ea5e8f54e58697` | `createWorkspaceRequest: models.CreateWorkspaceRequest` | wrapper vs `body`; nullability/number diffs — see §f model 3 |
| `listWorkspaceBudgets` | `id` (required) | `workspaceRef` (required) | **different member name for the same `{id}` path parameter** |
| `listWorkspaceMembers` | `id`, `offset?`, `limit?` | `id`, `offset?`, `limit?` | int64 policy |
| `bulkAddWorkspaceMembers` | `id`, `body?: …_56f9794190f2ec65` | `id`, `bulkAddWorkspaceMembersRequest` | wrapper vs `body` |
| `bulkRemoveWorkspaceMembers` | `id`, `body?: …_4abf9893791751c0` | `id`, `bulkRemoveWorkspaceMembersRequest` | wrapper vs `body` |

Additional mechanical notes:

- **Body-wrapper naming on ours** is `body` everywhere; published wrapper names track the spec's `requestBody` field naming (`requestBody`, `chatRequest`, `responsesRequest`, `imageGenerationRequest`, per-resource names like `createKeysRequestBody`).
- **Response-typed 408**: only ours declares `QueryAnalyticsResponse408` as a declared error; the published `queryAnalytics` throws typed classes — equivalent semantics, different surface.
- **Overload-based stream discrimination** (published `chat.send`/`responses.send`/`beta.responses.send`/`images.generate` have three overloads narrowing `chatRequest.stream`/`responsesRequest.stream`/`imageGenerationRequest.stream` to `false`/`true`/absent) has no our-side equivalent — those operations were refused.

---

## f. Response & model shapes

### Envelope preservation

- **Ours**: `<Op>Success = ApiResponse<Models.<Op>Response<status>, <status>, "<media>">`. `ApiResponse` (`http/types.ts:98-110`) carries `status`, `contentType`, `mediaType` (the matched declaration, used to narrow media unions), `rawContentType`, `headers` (raw `Headers`), `typedHeaders` (decoded per declared headers), `links` (declared Link metadata), and `data` (the decoded body — for declared errors, a frozen detached snapshot via `errorSnapshot`). The decoded body keeps whatever envelope the source declared: `{ data: … }` (`GetCreditsResponse200`, `GetAnalyticsMetaResponse200`, `ListEndpointsResponse200`), `{ data: [], meta }` (`AppRankingsResponse`), `{ data, key }` (create-keys shape in `models.ts`), bare arrays/models where the spec said so. **No `result`-wrapped envelope exists anywhere in `models.ts`** (0 occurrences of a `result` wire property).
- **Theirs**: single-success operations return the parsed body directly (no wrapper, no status record): `GetCreditsResponse = { data: GetCreditsData }`, `CreateKeysResponse = { data, key }`, `ExchangeAuthCodeForAPIKeyResponse = { key, userId }`, and — for the endpoints where their source wrapped it — `XResponse = { result: models.X }` (11+ list endpoints, §a-2). Errors never come back as values from resource methods: the internal `funcs` return `APIPromise<Result<Success, Errors>>` and the resource method `unwrapAsync`s it, throwing the typed error class.
- **Multi-variant responses**: ours — status/media-level union inside `<Op>Success` (one branch per declared 2xx status × media; e.g. `CreateCoinbaseChargeSuccess = ApiResponse<Uint8Array, 200, null>` for a non-JSON success). Theirs — discriminated at the call-signature level via stream overloads (`SendChatCompletionRequestResponse = models.ChatResult | EventStream<models.ChatStreamChunk>`), not via tagged status unions. Neither side emits "tagged unions" keyed by status at runtime; ours narrows by `response.status`/`response.mediaType` on the record.

### Field-by-field: 5 representative models

Notation: `theirs` / `ours`.

1. **GetCreditsData ↔ `Models.GetCreditsResponse200`** (`getcredits.d.ts` / `models.ts:9146`)
   - `totalCredits: number` / `"total_credits": JsonNumber` — wire name vs camelCase; plain JS number vs exact-decimal `JsonNumber`.
   - `totalUsage: number` / `"total_usage": JsonNumber` — same.
   - Envelope: both sides keep `data` in the body type; ours additionally wraps in `ApiResponse`.
   - Unknown keys: theirs strips (zod object default — `GetCreditsData$inboundSchema` is `z.object({total_credits: z.number(), …}).transform(remap$…)`); ours preserves via `[key: string]: JsonValue` index signatures on every object view.

2. **ListOrganizationMembersResponse ↔ `Models.ListOrganizationMembersResponse200`** (`operations/listorganizationmembers.d.ts` / `models.ts:11060`)
   - Envelope: theirs `{ result: ListOrganizationMembersResponseBody }`; ours `{ "data": [ …members ] }` — **the `result` wrapper is missing on our side** (source drift, §a-2).
   - Fields: `email: string` / `"email": string`; `id` / `"id"`; `firstName: string | null` / `"first_name": string` (**theirs carries null, ours does not** — the assembled spec's member schema has no nullable here); `lastName` / `"last_name"` same; `role: Role` (OpenEnum) / `"role": (string & ("org:admin" | "org:member"))`.
   - Role enum: theirs `OpenEnum<typeof Role>` = union `| Unrecognized<string>` (branded fallback value at runtime); ours an intersection with `string` — type-level permissive, but no runtime brand and no separate "unrecognized" class of value.

3. **CreateWorkspaceRequest ↔ `Models.CreateWorkspaceRequest_11ea5e8f54e58697` → `…_3956f1bc17564f93`** (`models/createworkspacerequest.d.ts` / `models.ts:1401`)
   - Required: `name: string; slug: string` / `"name": string; "slug": string` — match (wire vs camel).
   - Optional-and-nullable: theirs `defaultImageModel?: string | null | undefined`, `defaultProviderSort?`, `defaultTextModel?`, `description?`, `ioLoggingApiKeyIds?: Array<number> | null | undefined`; ours `"default_image_model"?: string`, `"default_provider_sort"?: string`, `"default_text_model"?: string`, `"description"?: string`, `"io_logging_api_key_ids"?: bigint[]` — **the assembled spec's `nullable: true` (OAS-3.0 keyword in this 3.1 document) is not surfaced as `| null` in our static view** ("Nullability is represented independently" per the generated doc comment; null is a codec/runtime obligation, and the compiled codec for these fields is the bare scalar/array without a null branch — `model-codecs.ts` symbol 36), while nulls *inside enum value lists* are surfaced (`headquarters?: (string & (…)) | null` because the spec enum literally contains `null`).
   - Numbers: `ioLoggingSamplingRate?: number` / `"io_logging_sampling_rate"?: JsonNumber`; `Array<number>` / `bigint[]` for api-key ids (int64 policy).

4. **BYOKKey ↔ `Models.BYOKKey`** (`models/byokkey.d.ts` / `models.ts:234`)
   - `allowedApiKeyHashes: Array<string> | null` / `"allowed_api_key_hashes": (string)[]` — theirs nullable array, ours required non-null (spec drift: the assembled schema declares no nullability).
   - `allowedModels` / `"allowed_models"`, `allowedUserIds` / `"allowed_user_ids"` — same pattern.
   - `createdAt: string` / `"created_at": string` — **both raw strings here**; but on `CreateKeysData.expiresAt` the published side converts to `Date` (inbound) / ISO string (outbound). Ours never converts: every date-time/date is a raw `string` (grep: no `Date` and no `RFCDate` in `models.ts`; query date params are `string`).
   - `disabled`, `id`, `label`, `provider` (`BYOKProviderSlug`) — match modulo casing. Ours `"provider": BYOKProviderSlug` where `BYOKProviderSlug = string & ("ai21" | … | "z-ai")` (112-slug intersection, `models.ts:354`); theirs `BYOKProviderSlug` is an OpenEnum over the same slugs + `Unrecognized` fallback.
   - Present theirs-only: `isByokOnly`, `isRequired`, `name?: string | null | undefined` (ours has `"name"?` without null), `sortOrder: number` / ours `"sort_order": bigint`, `workspaceId: string | null` / ours `"workspace_id": string` (required, non-null). Present ours-only: `is_fallback`/`isFallback` match — no, `is_fallback` exists both sides; ours-only: none material.
   - Nullability deltas on this single model: 4 fields where theirs is `| null` and ours is not (spec drift).

5. **AppRankingsResponse ↔ `Models.AppRankingsResponse`** (`models/apprankingsresponse.d.ts` / `models.ts:204`)
   - Envelope: theirs `{ result: models.AppRankingsResponse }` at the operation level, then `{ data: Array<AppRankingsItem>, meta: RankingsDailyMeta }`; ours directly `{ "data": (AppRankingsItem)[], "meta": RankingsDailyMeta, [key: string]: JsonValue }`.
   - `data` array of `AppRankingsItem` both sides (inner field casing differs per item model); `meta: RankingsDailyMeta` both sides.
   - Same `result`-wrapper drift as model 2.

### Their `unrecognized` pattern vs ours

- Theirs (`esm/types/unrecognized.d.ts`, `esm/types/enums.d.ts`): `Unrecognized<T> = T & { [__brand]: "unrecognized" }`; `OpenEnum<T> = T[keyof T] | Unrecognized<string>` — the parsed value keeps unknown enum strings with a brand distinguishing them from declared members; `startCountingUnrecognized()` exists for diagnostics. Closed enums are plain unions (`ClosedEnum<T>`).
- Ours: every open enum (documented `x-extended-enum`-style open vocabulary) is rendered `(string & ("a" | "b" | …))` — statically open, no brand, no runtime distinction, and unknown values pass codec validation only because the string intersection admits them. Closed enums in ours are bare unions (e.g. `"role"` renders open, matching the spec's open flag). There is no `unrecognized()` helper and no counting utility.

### Numeric / datetime / unknown-fallback summary

| Aspect | Published | Ours |
|---|---|---|
| int64 integers | `number` | `bigint` (13 exported query/int64 types; e.g. `ListWorkspacesQueryOffset = bigint`) |
| decimals / unspecified numbers | `number` | `JsonNumber` (exact string-backed decimal) |
| date-time (response) | `Date` (`CreateKeysData.expiresAt?: Date`) | `string` |
| date-time (request) | accept `Date`, serialize ISO (`expires_at?: string` in `$Outbound`) | `string` |
| date-only | `RFCDate` class (`esm/types/rfcdate.d.ts`) | `string` |
| unknown object keys | stripped at parse (zod object default) | preserved, typed `JsonValue` (index signatures) |
| constant date-time / template values | `constDateTime(val)` zod factory (`esm/types/constdatetime.d.ts`) | literal type in the model view |
| binary / blobs | `Blob`-based types (`esm/types/blobs.d.ts`) | finite in-memory `Uint8Array` / `ReadonlyBytes` |

---

## g. Pagination, streaming, errors, webhooks — side by side

### Pagination

- **Theirs**: list methods return `Promise<PageIterator<V, PageState>>` where `PageIterator<V, S> = V & { next: Paginator<V>; [Symbol.asyncIterator]: () => AsyncIterableIterator<V>; "~next"?: S }` (`esm/types/operations.d.ts`) — the page object is itself the success body, extended with `next()` (manual driving) and async iteration; helpers `createPageIterator`, `haltIterator`, `unwrapResultIterator`, `URL_OVERRIDE`. **19 auto-paginated methods**: `byok.list`, `datasets.getAppRankings`, `datasets.getSessionCost`, `embeddings.listModels`, `files.list`, `guardrails.{list, listGuardrailKeyAssignments, listGuardrailMemberAssignments, listKeyAssignments, listMemberAssignments}`, `models.{list, listForUser}`, `organization.listMembers`, `presets.{list, listVersions}`, `scim.{listGroups, listMappings}`, `workspaces.{list, listMembers}` (grep `PageIterator<` over `esm/sdk/*.d.ts`). Offset-based ones show the state type inline, e.g. `PageIterator<…, { offset: number }>`; files uses `{ cursor: string }`.
- **Ours**: generic walker (`pagination.ts`) — `PaginationDescriptor` patterns `limit-offset | cursor | page-number | next-link`, advance modes `items-returned | next-offset`, RFC-6901 response pointers, stop rules (hasMore=false / zero items / absent cursor / last page), `initialLimit` fallback hook, `PaginationError` loop guard, `walkPages` / `walkItems` / `nextPageInput`. **Emitted for exactly one operation**: `listOrganizationMembers` (`operations.ts:2090-2122`, `paginationDescriptors` has a single entry). The other 18 published paginated methods have no our-side counterpart (operation missing), and even where our operations exist (`listFiles`, `listBYOKKeys`, `listPresets`, `listWorkspaces`, `listGuardrails`, …) **no page/item/next helpers were generated** — the generator emitted pagination only for `listOrganizationMembers`, so e.g. `listPresets` must be driven by hand via `offset`.

### Streaming

- **Theirs**: `EventStream<T> extends ReadableStream<T>` with `[Symbol.asyncIterator]` and SSE parsing (`SseMessage<T>`, `esm/lib/event-streams.d.ts`); chat overloads return `Promise<operations.SendChatCompletionRequestResponse>` where the response is `models.ChatResult | EventStream<models.ChatStreamChunk>` (chunk carries `error?`, `openrouterMetadata?`, `serviceTier?`, `systemFingerprint?`); responses API returns `models.OpenResponsesResult | EventStream<models.StreamEvents>` (event models incl. `streameventsresponsecompleted/failed/incomplete`); images has the same dual overloads. Plus the whole agent toolkit: `callModel`, `ModelResult`, `Tool`/`ToolType`, tool executor/orchestrator, `chat-compat`/`anthropic-compat` libs, reusable-stream, stream-type-guards.
- **Ours**: `http/streams.ts` implements `streamItems` (SSE `server-sent-events` and JSON-lines framing, pull-driven, per-item limits) and `encodeItems`; `StreamPlan`/`Representation` types support both framings. **But no generated operation declares a stream representation** (grep `server-sent-events` in `operations.ts`: 0) — every streaming-capable endpoint was refused (`http-stream-item-schema-required`). Net: zero streaming surface despite functional machinery.

### Errors

| | Published | Ours |
|---|---|---|
| Base API error | `OpenRouterError extends Error` with `statusCode`, `body`, `headers`, `contentType`, `rawResponse` | `ApiError extends Error` with `kind: 'api-error'`, `operationSource`, `responseSource`, `response: ApiResponse<ReadonlyResponseData<T>, S, M, H, L, D>` (status/headers/typed data) |
| Per-status classes | 19: `BadRequestResponseError`, `UnauthorizedResponseError`, `PaymentRequiredResponseError`, `ForbiddenResponseError`, `NotFoundResponseError`, `RequestTimeoutResponseError`, `PayloadTooLargeResponseError`, `UnprocessableEntityResponseError`, `TooManyRequestsResponseError`, `InternalServerResponseError`, `BadGatewayResponseError`, `ServiceUnavailableResponseError`, `EdgeNetworkTimeoutResponseError`, `ProviderOverloadedResponseError`, `ConflictResponseError`, `GoneResponseError`, `GatewayTimeoutResponseError`, `OAuthErrorResponse`, `OpenRouterDefaultError` — each with typed `data` (`UnauthorizedResponseErrorData` etc.), `data$` raw value, and an inbound zod schema | none — declared errors are per-operation type unions `<Op>ApiError = DeclaredApiError<Models.XResponse400, 400, "application/json"> \| …` discriminated by `status`/`mediaType`, plus runtime guard `is<Op>ApiError` (branded by operation source key, lookalikes rejected) |
| Validation failures | `SDKValidationError` (rawValue, rawMessage, `pretty()`); `ResponseValidationError extends OpenRouterError` | `ResponseDecodingError` (kind `'response-decoding'`, status/contentType/headers/rawCapture) inside the `SdkError` family |
| Client/transport failures | `HTTPClientError` + `ConnectionError`, `RequestTimeoutError`, `RequestAbortedError`, `InvalidRequestError`, `UnexpectedClientError` | `RuntimeError` with `SdkFailureKind` = `request-validation \| request-representation \| transport \| cancelled \| resource-limit \| unexpected-response \| response-decoding`; undeclared statuses/media → `UnexpectedResponseError` with bounded `rawCapture`/`truncated` |
| Control flow | `Result<T, E>` monad + `APIPromise` at the func layer; thrown at resource layer (`unwrapAsync`) | thrown at the operation layer; success promise otherwise |
| Guards | `instanceof` (incl. custom `Symbol.hasInstance` on `SDKValidationError`) | `isSdkError(error, operationSource?)`, `isDeclaredApiError(error, source)`, `is<Op>ApiError`, `isPaginationError` |

### Webhooks

- **Theirs**: none (no webhook verification API generated; only observability *webhook destination* data models).
- **Ours**: none — no `incoming.ts` was emitted; the tree has no webhook code.

---

## h. Verdict — drop-in compatibility blockers and the general generator policies that would close them

| # | Sev | Blocker (observed) | General generator policy change | Class |
|---|-----|--------------------|--------------------------------|-------|
| 1 | P0 | 39 published methods unimplemented because admission refuses the operations — dominant code `http-path-parameters` (colon-style `:id` path params), plus `http-nullable-annotation`, `http-binary-schema-type`, `http-compatibility-profile`, `http-form-untyped-extras`, `http-stream-item-schema-required`, `http-security-scheme-unresolved`, `http-operation-id-duplicate` | **Colon path params**: accept and normalize `:param` path templates to RFC-6570 `{param}` at assembly; **compat profile: OAS-3.0 `nullable` in 3.1 documents** (map to `type: [T, "null"]` before admission); **binary schema type** acceptance (`type: string, format: binary` under non-body locations); **schemaless SSE items** (allow stream items without an item schema, validated as raw frames); **untyped form extras** (additionalProperties-less form bodies with a captured-extras escape); **duplicate operationId disambiguation** (synthesize stable unique ids); **securityScheme resolution fallback** (synthesize bearer-from-name when requirements reference missing schemes — currently done ad hoc in the assembled spec's local repair) | NEW-CAPABILITY (each code maps to an admission-profile rule) |
| 2 | P0 | 10 admitted operations not emitted because they carry no `operationId` (path-keyed inventory entries) | Operation naming fallback: synthesize `operationId` from method+path when absent, so admitted operations are always emitted | NEW-CAPABILITY |
| 3 | P0 | 25 published methods target API areas absent from the assembled spec (scim ×8, vault ×7, containers ×4, session-cost, JWKS, token exchange, `callModel`) | Spec-assembly completeness pass: reconcile the assembled document against the vendor's published SDK surface (or accept multiple source documents per package) | CONFIG-DEFAULT-CHANGE (assembly input policy) |
| 4 | P0 | Response envelope drift: published declares `{"result": …}` wire wrappers on ≥11 list endpoints; ours has none (0 `result` wire properties) | Envelope-reconciliation pass in assembly: detect wrapper-vs-component divergence between source documents; treat the wire-observed envelope as authoritative and encode the wrapper in response plans | CONFIG-DEFAULT-CHANGE (assembly), requires live-wire arbitration first |
| 5 | P0 | Input members lost vs published on shared operations (`group_by`, `workspace_id` on activity; 4 benchmark filters; `provider`/`after`/`after_id`/`before_id`/`order` on files; `workspace_id` on guardrails list) | Assembly must retain every declared parameter (the losses correlate with `http-extension-uninterpreted` drops — parameters gated behind uninterpreted vendor extensions are currently discarded) | CONFIG-DEFAULT-CHANGE (extension handling) |
| 6 | P1 | Resource-nested composition vs flat functions | **Resource-nested composition**: emit `class`/namespace accessors mirroring the vendor SDK (or a compat facade layer over flat functions) | NEW-CAPABILITY |
| 7 | P1 | Input type naming/ergonomics: `<Op>Request` + named body wrapper fields vs `<Op>Input` + bare `body` | **Request-suffix input naming** with per-spec wrapper-field names (`requestBody`, `chatRequest`, …) instead of a universal `body` | CONFIG-DEFAULT-CHANGE |
| 8 | P1 | Parameter naming: wire names exposed (`api_key_hash`, `start_date`) vs camelCase with `$Outbound` wire mapping | **camelCase SDK-native param naming with wire mapping**: generate camelCase members plus an outbound remap layer (the inverse of today's direct-wire convention) | NEW-CAPABILITY |
| 9 | P1 | SDK globals (`HTTP-Referer` / `X-OpenRouter-Title` / `X-OpenRouter-Categories`) absent — not in the OpenAPI source | **SDK globals configuration**: first-class generator config for non-spec headers (client-level default + per-request override + env fallback), applied to all operations | NEW-CAPABILITY |
| 10 | P1 | int64 → `bigint` and decimals → `JsonNumber` vs published `number` | Numeric compat profile: map int64/decimal to `number` (with documented precision loss) or `number \| JsonNumber` under a compatibility switch | CONFIG-DEFAULT-CHANGE |
| 11 | P1 | Unknown response/object keys preserved as `JsonValue` vs published stripping them | Choose per direction: for a published-SDK-compat profile, strip undeclared properties at decode (or keep ours — a strictness advantage, not a blocker, but a visible behavioral difference) | CONFIG-DEFAULT-CHANGE |
| 12 | P1 | No retries/timeouts vs published default backoff + `timeoutMs` | Optional retry/timeout module (policy object on client/call options; default off preserves today's behavior) | ALREADY-EXISTS (transport hook point; policy machinery is new) |
| 13 | P1 | Error surface granularity: per-status error classes, `pretty()` validation errors, client-error taxonomy vs single `ApiError` + `RuntimeError` kinds | **Error-class generation**: optional per-status error classes (and `SDKValidationError`-equivalent with `pretty()`) derived from declared error responses | NEW-CAPABILITY |
| 14 | P1 | Streaming operations absent (schemaless SSE) | See #1 (schemaless SSE items); plus overload-style stream discrimination (flag-parameter overloads returning item iterators) | NEW-CAPABILITY |
| 15 | P2 | `unrecognized` enum fallback brand vs bare string intersections | **Unrecognized-enum fallbacks**: brand open-enum fallback values so application code can distinguish declared vs undeclared members | NEW-CAPABILITY |
| 16 | P2 | Datetime as raw strings vs `Date`/`RFCDate` | **Datetime as native `Date`** (decode date-time to `Date`, date-only to an `RFCDate`-equivalent; keep wire mapping for encode) | NEW-CAPABILITY |
| 17 | P2 | `nullable: true` not surfaced as `| null` in static model views (while null-in-enum is) | Surface nullability in the static type (`T \| null`) whenever the source says nullable — closes the gap and subsumes refusal code `http-nullable-annotation` | CONFIG-DEFAULT-CHANGE |
| 18 | P2 | No `HTTPClient`-style hook surfaces (`beforeRequest`/`afterSuccess`/`afterError`, request-error hooks) or `debugLogger` | Lifecycle hook module around the injected `fetch` (wraps the same injection point that exists today) | ALREADY-EXISTS (injection point) / NEW-CAPABILITY (hook API) |
| 19 | P2 | Different telemetry/UA conventions (`speakeasy-sdk/…` vs `suspect/… ua/v1`), no env auto-config (`OPENROUTER_*`), no `debugLogger` | Config-pluggable attribution template + optional env-alias table | CONFIG-DEFAULT-CHANGE |
| 20 | P2 | Cosmetic: `PageIterator`-style page objects vs our `<op>Pages/Items/NextPage` triple; `~next` page state; `X-Globals` types; subpath exports (`/models/errors`, `/models/operations`, `./types`) | Emitter cosmetics under a compat profile | CONFIG-DEFAULT-CHANGE |
| 21 | — | `callModel` / tool-orchestration / `ModelResult` agent toolkit | Out of scope for an HTTP-surface SDK (upstream itself split it into `@openrouter/agent`) | — |

### Bottom line

- The two SDKs agree on wire-level facts wherever both sides generated an operation: method, path shape, bearer header, declared media types, and (excepting the `result`-wrapper question) the decoded body shapes for the 42 shared operations.
- Drop-in **call-site** compatibility is ~12% (14/115 methods callable by the same name through the same nesting); drop-in **capability** compatibility is ~36% (41/115 methods exist at all under different names).
- The largest gaps are admission-profile (path-param style, nullability, streaming schemas, form extras) and source-assembly (missing API areas, lost parameters, missing `result` envelopes) rather than runtime deficiencies: the runtime already has streaming, pagination, media, and codec machinery that the admitted operations do not exercise.
