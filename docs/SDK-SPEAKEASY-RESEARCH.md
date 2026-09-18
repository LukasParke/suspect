# Speakeasy / OpenRouter SDK research

Research date: **2026-09-10**. Primary-source inspection of the official OpenRouter TypeScript and Python repositories and current Speakeasy documentation.

## Bottom line

OpenRouter's Speakeasy-generated SDKs provide a substantial consumer-facing baseline: resource namespaces, conventional imports and model names for the three operations examined, native HTTP integrations, configurable retries/timeouts, structured exceptions, and extensive generated endpoint/model documentation. Their behavior is a combination of **Speakeasy generation, OpenRouter's specification and overlays, generation configuration, and custom code**. Treating all of it as an unconfigured generator default would misattribute both strengths and problems. [T-config] [P-config] [T-workflow] [P-workflow] [T-root]

The clearest precision distinction is concrete: OpenRouter's credits totals become TypeScript `number` and Python `float`. The input contract specifies `number` / `double`; this is not evidence that Speakeasy mishandles general OpenAPI or cannot generate decimal types. Speakeasy explicitly documents decimal support. Both SDKs also already distinguish omitted and explicitly null update fields and perform runtime validation. [T-credits-model] [P-credits-model] [credits-schema] [T-update-model] [P-update-model] [S-ts]

For comparison with our canonical SDKs, the coordinator supplied this local status: five verified targets (TypeScript/JavaScript, Python, Go, Rust, Swift), bounded JSON/bearer scope, four native integrations, and seven remaining targets in progress. Local source-provenance, exact-numeric, presence, mutable-validation, resource, and native-gate claims remain the main session's verification responsibility. The external evidence supports a **breadth/documentation advantage for the inspected OpenRouter packages**, alongside a potentially stronger, narrower semantic guarantee in our implementation. It does not establish a numerical quality ranking, performance result, or overall security winner.

## 1. Reproducible source snapshot

GitHub's public commits API supplied these default-branch heads. Versions below come from repository manifests, not search snippets or an independently checked package-registry release.

| Repository | Pinned commit | Commit time (UTC) | Manifest version |
| --- | --- | --- | --- |
| `OpenRouterTeam/typescript-sdk` | `9078199a74dc5d35714ad641a0dce2756164166f` | 2026-09-09 21:29:11 | `@openrouter/sdk` **1.2.116** |
| `OpenRouterTeam/python-sdk` | `c6a371052886819c6f6905a304168f5bfae745b3` | 2026-09-09 21:29:02 | `openrouter` **1.1.136** |

Sources: [T-commit], [P-commit], [T-package], [P-package]. Both workflows pin Speakeasy CLI **1.787.0**; generated runtime metadata reports generator **2.914.0** and OpenAPI document version **1.0.0**. These are distinct version numbers. [T-workflow] [P-workflow] [T-options] [P-version]

Public commit archives were inspected outside the working repository. Findings and examples are **source-verified**, not authenticated API executions or a build/test of published distributions. Live vendor documentation is dated by this retrieval; repository citations are commit-pinned.

## 2. Same three operations: real public call sites

All three operations require an OpenRouter **management key**, according to their operation descriptions. Configure that key as `OPENROUTER_API_KEY` in these examples. The SDK supplies the bearer header. [T-credits-op] [T-keys] [P-credits] [P-keys]

| OpenAPI operation / HTTP route | TypeScript public method | Python public method |
| --- | --- | --- |
| `getCredits` — `GET /credits`, success 200 | `sdk.credits.getCredits()` | `sdk.credits.get_credits()` |
| `createKeys` — `POST /keys`, success 201 | `sdk.apiKeys.create({ requestBody: … })` | `sdk.api_keys.create(name=…, …)` |
| `updateKeys` — `PATCH /keys/{hash}`, success 200 | `sdk.apiKeys.update({ hash, requestBody: … })` | `sdk.api_keys.update(hash=…, …)` |

Sources: [T-credits], [T-keys], [T-create-op], [T-update-op], [P-credits], [P-keys]. These are resource-namespaced operations, not root `sdk.getCredits()` / `sdk.createKeys()` methods. Python additionally provides `get_credits_async`, `create_async`, and `update_async` on the same resources. Both root clients lazily instantiate resource clients. [T-root] [P-root]

### TypeScript / JavaScript

```typescript
import { OpenRouter } from "@openrouter/sdk";

const sdk = new OpenRouter({
  apiKey: process.env["OPENROUTER_API_KEY"],
});

const credits = await sdk.credits.getCredits();
const purchased: number = credits.data.totalCredits;
const used: number = credits.data.totalUsage;

const created = await sdk.apiKeys.create({
  requestBody: { name: "sdk-demo", limit: 10 },
});

const updated = await sdk.apiKeys.update({
  hash: created.data.hash,
  requestBody: { name: "sdk-demo-renamed", limit: null },
});
// created.key contains the one-time plaintext key; updated.data is metadata.
// limit: null is explicit JSON null; omitting limit leaves it absent.
```

The nested `requestBody` is required at this pin, including for **create**. `CreateKeysRequest`, `CreateKeysRequestBody`, `UpdateKeysRequest`, `UpdateKeysRequestBody`, `GetCreditsResponse`, and `GetCreditsData` are actual public model names. The public type-import entry point is `@openrouter/sdk/models/operations`. No manual serialization or request-model construction is needed for these calls. [T-create-model] [T-update-model] [T-credits-model] [T-package]

### Python

```python
import os
from openrouter import OpenRouter

with OpenRouter(api_key=os.environ["OPENROUTER_API_KEY"]) as sdk:
    credits = sdk.credits.get_credits()
    purchased: float = credits.data.total_credits
    used: float = credits.data.total_usage

    created = sdk.api_keys.create(name="sdk-demo", limit=10)
    updated = sdk.api_keys.update(
        hash=created.data.hash,
        name="sdk-demo-renamed",
        limit=None,
    )
    # created.key is the one-time plaintext key.
    # None is explicit JSON null here; omission uses UNSET.
```

Python's keyword-only convenience methods construct `operations.CreateKeysRequestBody` / `UpdateKeysRequestBody` internally. Models and matching `TypedDict` forms exist for structured inputs, but consumers do not need to import them for these operations. Async consumption uses `async with OpenRouter(...)` and `await sdk.credits.get_credits_async()` / `await sdk.api_keys.create_async(...)` / `await sdk.api_keys.update_async(...)`. [P-keys] [P-credits] [P-update-model] [P-root]

## 3. Auth, native clients, options, errors, and operational behavior

### Construction and authentication

| Concern | TypeScript at the pin | Python at the pin |
| --- | --- | --- |
| Constructor | `new OpenRouter(options?)` | `OpenRouter(api_key=…, …)` |
| Credential input | `apiKey?: string \| (() => Promise<string>)` | `api_key`: optional string or synchronous callable returning an optional string |
| Environment fallback | `OPENROUTER_API_KEY`; environment parsing is memoized | `OPENROUTER_API_KEY` when no security object was supplied |
| Wire authentication | `Authorization: Bearer …`; preserves an already-prefixed token | Same bearer-prefix behavior |
| Default API URL | `https://openrouter.ai/api/v1` | Same |
| URL override | `serverURL`, `server: "production"`; also `OPENROUTER_BASE_URL` | `server_url`, `server`, `url_params`; the inspected configuration does not show a corresponding base-URL environment fallback |
| Native/custom transport | Exported `HTTPClient`, wrapping native `fetch`; `httpClient` constructor option | HTTPX sync and async clients; `client` and `async_client` constructor options |
| Attribution headers | `httpReferer`, `appTitle`, `appCategories` | `http_referer`, `x_open_router_title`, `x_open_router_categories` |
| Resource lifecycle | Resource getters reuse options | Sync/async context managers and finalizer; caller-supplied HTTP clients are not closed by the respective context-manager exit |

Sources: [T-options], [T-native], [T-http], [T-security], [T-env], [P-root], [P-options], [P-http], [P-security]. Neither constructor verifies that a credential is a management key. That authorization is an API requirement. A credential callback is not itself evidence of an automatic OAuth refresh flow in these bearer-configured OpenRouter clients.

### Retries and timeouts: the actual defaults matter

- **All three selected operations, including POST and PATCH, default to retrying `5XX` and transport failures/timeouts.** The embedded backoff configuration is `initialInterval=500 ms`, `maxInterval=60,000 ms`, `exponent=1.5`, `maxElapsedTime=3,600,000 ms`, with connection retries enabled. `429` is a modeled error for create/update, but is **not** in their default retry status list. This comes from OpenRouter's `x-speakeasy-retries` configuration. The adjacent `x-retry-strategy.maxAttempts: 3` is not the policy used by these generated retry loops. [retry-schema] [T-credits-op] [T-create-op] [T-update-op] [P-credits] [P-keys]
- **Overrides:** TypeScript constructor `retryConfig`, per-call `retries`, and per-call `retryCodes`; disable with `{ strategy: "none" }`. Python constructor `retry_config`, per-call `retries`; explicit `None` disables retries, while `UNSET` inherits defaults. Python's actual `utils.RetryConfig` also has `status_codes_override`, even though the generic Speakeasy retry guide says runtime status-code overrides are unavailable. Prefer package source for exact APIs. [T-retry] [P-retry] [P-credits] [S-retry]
- **Backoff implementations differ:** TypeScript computes `initialInterval * attemptIndex ** exponent + jitter`; Python computes `initial_interval * exponent ** retries + jitter`. Both inspect `Retry-After` and `retry-after-ms`. TypeScript caps the computed/header delay at `maxInterval`; Python returns a positive server-specified delay before applying its normal cap. Both check elapsed budget after a failed attempt, so `maxElapsedTime` is not a strict whole-call deadline. [T-retry] [P-retry]
- **TypeScript timeout/cancel:** constructor and per-call `timeoutMs`; no SDK timeout signal by default. A positive timeout creates a fresh `AbortSignal.timeout` for each HTTP attempt. A supplied per-call `signal` takes precedence, so it replaces the SDK's `timeoutMs` mechanism. `RequestOptions` flattens native `RequestInit` options; `fetchOptions` still exists but is deprecated. Abort is recognized as non-retryable, although the backoff sleep itself does not listen to the abort signal. [T-http] [T-retry]
- **Python timeout/cancel:** constructor and per-call `timeout_ms` are converted to HTTPX seconds. Omission delegates to the HTTP client's timeout configuration via `httpx.USE_CLIENT_DEFAULT`. These are transport timeouts, not a whole-retry-loop deadline. There is no cancellation-token argument on these methods; async task cancellation is the native mechanism. The async path awaits HTTPX and `asyncio.sleep`, and catches `Exception`, not `BaseException`. [P-http] [P-retry] [P-credits]

Source-shaped control recipes (using the clients above):

```typescript
// Per-call timeout, with retries explicitly disabled.
await sdk.credits.getCredits(undefined, {
  timeoutMs: 10_000,
  retries: { strategy: "none" },
});

// Native cancellation; pass the controller's signal at the top level.
const controller = new AbortController();
const pending = sdk.credits.getCredits(undefined, {
  signal: controller.signal,
  retries: { strategy: "none" },
});
controller.abort();
await pending; // rejects; handle RequestAbortedError
```

```python
# Per-call HTTP timeout, with retries explicitly disabled.
credits = sdk.credits.get_credits(timeout_ms=10_000, retries=None)

# Native whole-call async deadline, inside an async function/context.
import asyncio
credits = await asyncio.wait_for(
    sdk.credits.get_credits_async(retries=None),
    timeout=10,
)
```

These illustrate control APIs; they were not executed against OpenRouter. [T-http] [P-http] [P-retry]

### Current error surface

Both expose the following explicitly modeled HTTP errors for the selected operations:

| Operation | HTTP status → generated exception class |
| --- | --- |
| Credits | 401 → `UnauthorizedResponseError`; 403 → `ForbiddenResponseError`; 500 → `InternalServerResponseError` |
| Create key | 400 → `BadRequestResponseError`; 401 → `UnauthorizedResponseError`; 403 → `ForbiddenResponseError`; 429 → `TooManyRequestsResponseError`; 500 → `InternalServerResponseError` |
| Update key | 400 → `BadRequestResponseError`; 401 → `UnauthorizedResponseError`; 404 → `NotFoundResponseError`; 429 → `TooManyRequestsResponseError`; 500 → `InternalServerResponseError` |

Other HTTP failures/unmatched responses fall back to `OpenRouterDefaultError`; `OpenRouterError` is the HTTP-error base class. TypeScript imports are from `@openrouter/sdk/models/errors`; Python uses `from openrouter import errors`. TypeScript exposes `statusCode`, `body`, `headers`, `contentType`, `rawResponse`; Python exposes `status_code`, `body`, `headers`, `raw_response`. These are HTTP errors, not an exhaustive superclass for input/transport failures. [T-credits-op] [T-create-op] [T-update-op] [T-matchers] [T-error] [P-credits] [P-keys] [P-error]

TypeScript class methods unwrap results and reject on error; standalone functions such as `creditsGetCredits(core)` instead expose a typed `{ ok, value/error }` result. Input parsing uses `SDKValidationError`; bad response parsing uses `ResponseValidationError`; transport errors are classified into `ConnectionError`, `RequestAbortedError`, `RequestTimeoutError`, `InvalidRequestError`, and `UnexpectedClientError`. Python constructs Pydantic request models, wraps response decode/validation failures as `errors.ResponseValidationError`, and propagates HTTPX transport exceptions through its default hooks/retry machinery. Consequently generic vendor examples using `SDKError`/`SDKException` are not the exact OpenRouter exception names. [T-credits] [T-credits-op] [T-http] [T-credit-doc] [P-keys] [P-validation] [P-http]

### Debugging

TypeScript accepts `debugLogger: console`; Python accepts `debug_logger=logging.getLogger("openrouter")` with DEBUG logging configured. Both support `OPENROUTER_DEBUG`. At these pins the environment check is truthiness-based: TypeScript uses `z.coerce.boolean()`, Python tests the nonempty environment string. The string `"false"` therefore does not disable this setting. [T-env] [T-http] [P-logger] [P-readme]

Logging is opt-in by default. TypeScript's logger enumerates header values and JSON bodies, and its README explicitly says that tokens can appear in debug logs. Python logs HTTPX header objects and full non-streaming response text; there is no SDK-level body redaction in that path, including a create-key response's plaintext `key`. Streaming response bodies are represented by a marker in these logging paths. These are specific, inspectable logging behaviors, not evidence of a general security ranking. [T-readme] [T-http] [P-http]

## 4. Numeric fidelity, presence, validation, and naming

The credits model mapping is literal:

```typescript
// src/models/operations/getcredits.ts
type GetCreditsData = { totalCredits: number; totalUsage: number };
// Wire fields use z.number(), then remap total_credits / total_usage.
// JSON helpers use JSON.parse, not an exact-decimal JSON representation.
```

```python
# src/openrouter/operations/getcredits.py
class GetCreditsData(BaseModel):
    total_credits: float
    total_usage: float
```

Both generated response fields are required. These types do not preserve arbitrary JSON numeric lexemes or promise arbitrary decimal precision. The upstream credits schema explicitly uses `format: double`; this is a host-number mapping consistent with that declared format. Speakeasy's TypeScript design documentation separately supports `format: decimal` via `decimal.js` and large integers via `format: bigint`, and its language matrix lists decimal/bigint support. **A fair claim is “our chosen exact-number representation offers stronger preservation for these fields,” not “Speakeasy cannot handle precise numbers.”** [T-credits-model] [P-credits-model] [credits-schema] [S-ts] [S-maturity]

Presence handling is also already present: TS update `limit?: number | null | undefined`, with a nullable/optional outbound schema; Python `limit: OptionalNullable[float] = UNSET`, with serialization checking explicitly-set nullable fields. Both can represent “absent” versus “explicit null” for this PATCH. TS validates inputs on each call and parses responses through Zod; Python constructs Pydantic request models and validates decoded responses. A superiority claim about mutation revalidation, exact schema-resource identity, or full JSON Schema semantics needs matched adversarial tests, not the observation that Speakeasy uses Zod/Pydantic. [T-update-model] [P-update-model] [T-credits-op] [P-keys] [P-validation]

The three selected operations have relatively readable public names, and object literals/keyword arguments hide model plumbing at the call site. This does not establish uniformly polished naming: the same TypeScript package contains `SendChatCompletionRequestRequest`. Python offers snake_case fields and corresponding TypedDicts; TS uses camelCase mapping. These are partly configurable presentation choices. [T-create-model] [T-update-model] [T-chat-model] [P-update-model] [T-config] [P-config]

## 5. What documentation and native recipes actually ship?

### Quantitative inventory, not a quality benchmark

Counts below are file counts in the pinned archives: `docs/**/*.mdx` and `docs/sdks/**/README.mdx`. They do not measure completeness, correctness, compilation, or semantic coverage.

| Inventory item | TypeScript | Python |
| --- | ---: | ---: |
| Resource-reference `README.mdx` files | 29 | 29 |
| All MDX documentation files, including model pages | 1,683 | 1,895 |
| Selected operations inspected in source | 3 | 3 sync + 3 async variants |

Sources: [T-doc-tree], [P-doc-tree], [T-credits], [T-keys], [P-credits], [P-keys].

- **Root README:** TS includes installation for npm/pnpm/Bun/Yarn, ESM-only/CommonJS guidance, a streaming chat example, pagination, file-upload recipes using native file APIs, debugging, and development instructions. Python includes uv/pip/Poetry, Python ≥3.10, script usage, sync/async examples, Pydantic IDE guidance, web search, pagination, file uploads, resource lifecycle, debugging, and development instructions. PyPI is configured to use `README-PYPI.md`. [T-readme] [P-readme] [P-package]
- **Generated `USAGE.md`:** TS currently demonstrates `analytics.getUserActivity()`; Python demonstrates `analytics.get_user_activity()` and its async variant. They are short usage snippets, not comprehensive native-client manuals. [T-usage] [P-usage]
- **Endpoint docs:** `docs/sdks/credits/README.mdx` and `docs/sdks/apikeys/README.mdx` contain operation descriptions, examples, parameter/response links, and status/error tables. TS also documents per-operation standalone functions and their result-style error handling. Models have separate linked field/type pages. Both repos configure Mintlify documentation and publish SDK code samples through their Speakeasy workflows. [T-credit-doc] [P-credit-doc] [T-key-doc] [P-key-doc] [T-config] [P-config] [T-workflow] [P-workflow]
- **Native/runtime guidance and examples:** TS has `RUNTIMES.md` covering Fetch/Streams/async iterables and recommended compiler libraries, `FUNCTIONS.md`, and an examples directory with a Next.js app, embeddings, reasoning, multimodal/tools, and analytics examples. Python has the context-manager recipes above and an OAuth PKCE example. These are valuable packaging/DX assets; their existence is not evidence that all samples currently compile or execute. [T-runtimes] [T-functions] [T-example-tree] [P-example-tree]
- **Published native SDK docs:** OpenRouter's current TS/Python SDK landing pages provide installation/quickstart and link resource/model API references. Speakeasy itself documents generated README/reference support, code-sample overlays, and integrations; these are distinct from OpenRouter's curated landing content. [OR-ts] [OR-python] [S-features] [S-samples]

### Concrete documentation gaps at these pins

1. Both READMEs contain `<!-- No … -->` markers suppressing generated authentication, retries, error handling, server selection, and custom-HTTP-client sections. Thus “Speakeasy has comprehensive runtime recipes” does not mean all those recipes appear in **these** top-level READMEs. [T-readme] [P-readme]
2. The generated TS credits parameter table marks `request` required, while its own example omits it and the method signature is `request?`. It also presents `options.fetchOptions`, which current source deprecates in favor of flattened options. [T-credit-doc] [T-credits] [T-http]
3. Python's README streaming examples assign `res` but iterate `event_stream`, an undefined variable in the shown snippets. This is a directly visible copy/paste defect, not a runtime experiment. [P-readme]
4. TS's README/landing quickstart passes chat fields directly to `chat.send({ messages, … })`; pinned source requires `{ chatRequest: … }`. The root source also still contains a custom `callModel` region while the README directs users to migrate to `@openrouter/agent`. Source and curated documentation should therefore be checked together, rather than treating either as an infallible generated contract. [T-readme] [OR-ts] [T-chat] [T-chat-model] [T-root]

## 6. Generator defaults versus OpenRouter decisions

- The inspected operation/model files carry **“Code generated by Speakeasy … DO NOT EDIT”** headers; TS additionally includes `@generated-id`. The repos retain `.speakeasy/in.openapi.yaml`, `out.openapi.yaml`, `gen.yaml`, workflow files, and locks. The TS workflow lock includes source revision/blob digests and a code-samples digest. Speakeasy therefore does have generation/source provenance; any comparison with our per-resource/per-field provenance needs to identify the different granularity. [T-credits-model] [P-credits-model] [T-workflow] [P-workflow] [T-lock]
- Each workflow applies **seven overlays**. Both include open enums, RSS-response removal, app headers, allOf simplification, boolean query parameters, and a deprecated beta-response alias. TS also applies `nullable-model-fields`; Python applies `fix-nullable-pagination`. These are concrete OpenRouter choices, not evidence that all generated SDKs have those policies. [T-workflow] [P-workflow]
- The header overlay injects globals and path-level headers and overrides public header names. Method names such as key `create` / `update` are also present as `x-speakeasy-name-override` extensions in the resulting specification. TS config selects `maxMethodParams: 0`, camelCase models, ESM, flat responses, strict mode, and Zod v4; Python selects `flattenRequests: true`, `maxMethodParams: 999`, `asyncMode: both`, and response schema validation. These settings explain much of the actual call-site difference. [header-overlay] [key-schema-create] [key-schema-update] [T-config] [P-config]
- The root TS SDK includes explicit `#region` custom-code imports/body for `callModel`, whose helper source lacks a generated-file header and builds an OpenRouter-specific result/tool wrapper. Do not attribute that convenience layer wholesale to default Speakeasy output. The three credits/key methods examined use ordinary generated resource methods. [T-root] [T-call-model] [T-credits] [T-keys]
- Speakeasy documents customization through the OpenAPI document, overlays/extensions, `gen.yaml`, hooks, and custom code preserved through three-way merging. Hook-generation availability is documented for Business/Enterprise; SDK contract-test generation is separately documented as an Enterprise add-on. These concern the builder's product/configuration, not a runtime service requirement for consumers of the generated packages. [S-custom] [S-hooks] [S-tests]

## 7. Current language support and broader capabilities

**Verified current maturity table:** TypeScript, Python, Go, Java, C#, PHP, and Ruby are **seven GA SDK language targets**. The first five have feature-support level GA; PHP and Ruby have level 1. Terraform, MCP TypeScript, Postman, and CLI are separately listed artifact targets, not extra SDK programming languages. [S-maturity]

Speakeasy's current docs landing page explicitly labels **Rust and C++ “Coming Soon.”** **Swift status is unresolved by the retrieved maturity documentation:** it is absent from that table, and the OAuth page has a Swift row marked under construction for the password flow. Indexed marketing mentions Swift, but the SDK product page and attempted Swift-methodology URLs returned 404 on retrieval. Search snippets were not promoted into a verified support claim. In particular, this report does **not** establish that Swift is unsupported. [S-home] [S-maturity] [S-oauth]

Official capability documentation establishes the following, with configuration/language qualifications:

| Area | What official documentation actually supports |
| --- | --- |
| Authentication | Basic, API-key, and bearer schemes from OpenAPI; OAuth flows with scheme/configuration requirements, and hooks/custom security for other flows. OAuth subfeatures have language-specific support tables. |
| Pagination | `x-speakeasy-pagination` supports offset/limit, cursor, and next-URL styles. OpenRouter's README concretely demonstrates BYOK pagination: TS async iteration, Python `.next()`. |
| Streaming | SSE modeled using `text/event-stream`; support documented for TS, Python, Go, Java, C#, PHP, Ruby. TS/Python support inferred streaming overloads. Sentinel handling is configurable. |
| Documentation/customization | Generated references/examples, `x-codeSamples` overlays, naming/grouping/configuration, lifecycle hooks, preserved custom edits. |
| OpenAPI input | Quickstart advertises OpenAPI 3.0, 3.1, and JSON Schema. The feature matrix is explicitly non-exhaustive and distinguishes implemented/partial/missing features. |
| Contract-test generation | Beta; documented for TS, Python, Go, Java, Ruby, C#; **successful scenarios only**, not generated error assertions. Both OpenRouter configs set `generateTests: false`, which does not mean the repositories contain no custom tests. |

Sources: [S-auth], [S-oauth], [S-pagination], [T-readme], [P-readme], [S-sse], [S-samples], [S-custom], [S-quickstart], [S-maturity], [S-tests], [T-config], [P-config]. This is vendor documentation plus inspection of two concrete outputs, not a controlled cross-generator JSON Schema/OpenAPI conformance suite.

## 8. Evidence-bounded comparison for the main answer

| Dimension | Supported assessment |
| --- | --- |
| Everyday consumer DX | OpenRouter's resource APIs and simple literals/keywords are a strong baseline. The selected public model names are approachable. Compare our actual call sites before claiming equal polish. |
| Documentation and distribution | Extensive generated references, examples, normal package-manager installs, runtime docs, and publication workflows support a breadth advantage over the coordinator's reported thin README/coverage. The concrete drift above prevents an “all examples are correct” claim. |
| Auth and native integration | Bearer setup, credential callbacks, Fetch/HTTPX injection, timeouts, retries, and Python context management are already provided. Our bearer/native gates can prove bounded behavior; they do not establish a general security advantage. |
| Numeric fidelity | The inspected credits totals use host floating-point types. A source-proven exact-number representation can offer a real stronger guarantee for the same wire values. Speakeasy's configurable decimal support must be acknowledged. |
| Presence and validation | Both systems have relevant mechanisms. Our distinctive claim must be the tested semantic guarantees, including mutation/resource handling, rather than “Speakeasy lacks null/absence or runtime validation.” |
| Provenance and conformance | Speakeasy includes source/config snapshots, digests, generated markers, and feature/test-generation documentation. Our finer-grained provenance and native/semantic gates can be differentiators if demonstrated on matching contracts. No whole-generator pass/fail ranking was measured here. |
| Language/feature breadth | Seven documented GA Speakeasy SDK targets are not comparable to “five locally verified bounded targets plus seven in progress” as a single quality score. Keep target maturity, operation coverage, protocol coverage, and executed native gates separate. |

The practical takeaway is to retain our demonstrated semantic guarantees while closing the visible consumer-DX/documentation gaps. These sources support that prioritization more strongly than a blanket claim that either generator produces universally better SDKs.

## Sources

Repository links below are pinned to the snapshots in §1; Speakeasy/OpenRouter website links were retrieved on the research date.

[T-commit]: https://github.com/OpenRouterTeam/typescript-sdk/commit/9078199a74dc5d35714ad641a0dce2756164166f
[P-commit]: https://github.com/OpenRouterTeam/python-sdk/commit/c6a371052886819c6f6905a304168f5bfae745b3
[T-package]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/package.json
[P-package]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/pyproject.toml
[T-config]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/gen.yaml
[P-config]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/.speakeasy/gen.yaml
[T-workflow]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/workflow.yaml
[P-workflow]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/.speakeasy/workflow.yaml
[T-lock]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/workflow.lock
[T-root]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/sdk.ts
[P-root]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/sdk.py
[T-options]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/config.ts
[P-options]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/sdkconfiguration.py
[P-version]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/_version.py
[T-credits]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/credits.ts
[T-keys]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/apikeys.ts
[T-credits-op]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/funcs/creditsGetCredits.ts
[T-create-op]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/funcs/apiKeysCreate.ts
[T-update-op]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/funcs/apiKeysUpdate.ts
[P-credits]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/credits.py
[P-keys]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/api_keys.py
[T-credits-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/models/operations/getcredits.ts#L56-L141
[P-credits-model]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/operations/getcredits.py#L140-L164
[T-create-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/models/operations/createkeys.ts
[T-update-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/models/operations/updatekeys.ts
[P-update-model]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/operations/updatekeys.py#L100-L154
[T-http]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/sdks.ts
[T-native]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/http.ts
[P-http]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/basesdk.py
[T-security]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/security.ts
[P-security]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/utils/security.py
[T-env]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/env.ts
[T-retry]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/retries.ts
[P-retry]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/utils/retries.py
[T-matchers]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/lib/matchers.ts
[T-error]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/models/errors/openroutererror.ts
[P-error]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/errors/openroutererror.py
[P-validation]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/utils/unmarshal_json_response.py
[P-logger]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/utils/logger.py
[T-readme]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/README.md
[P-readme]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/README.md
[T-usage]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/USAGE.md
[P-usage]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/USAGE.md
[T-credit-doc]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/docs/sdks/credits/README.mdx
[P-credit-doc]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/docs/sdks/credits/README.mdx
[T-key-doc]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/docs/sdks/apikeys/README.mdx
[P-key-doc]: https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/docs/sdks/apikeys/README.mdx
[T-doc-tree]: https://github.com/OpenRouterTeam/typescript-sdk/tree/9078199a74dc5d35714ad641a0dce2756164166f/docs
[P-doc-tree]: https://github.com/OpenRouterTeam/python-sdk/tree/c6a371052886819c6f6905a304168f5bfae745b3/docs
[T-runtimes]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/RUNTIMES.md
[T-functions]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/FUNCTIONS.md
[T-example-tree]: https://github.com/OpenRouterTeam/typescript-sdk/tree/9078199a74dc5d35714ad641a0dce2756164166f/examples
[P-example-tree]: https://github.com/OpenRouterTeam/python-sdk/tree/c6a371052886819c6f6905a304168f5bfae745b3/examples
[T-chat]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/chat.ts
[T-chat-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/models/operations/sendchatcompletionrequest.ts
[T-call-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/funcs/call-model.ts
[credits-schema]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/out.openapi.yaml#L31013-L31092
[retry-schema]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/out.openapi.yaml#L41988-L42002
[header-overlay]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/overlays/add-headers.overlay.yaml
[key-schema-create]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/out.openapi.yaml#L35580-L35591
[key-schema-update]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/.speakeasy/out.openapi.yaml#L36270-L36281
[OR-ts]: https://openrouter.ai/docs/sdks/typescript
[OR-python]: https://openrouter.ai/docs/sdks/python
[S-home]: https://www.speakeasy.com/docs
[S-maturity]: https://www.speakeasy.com/docs/sdks/languages/maturity
[S-quickstart]: https://www.speakeasy.com/docs/sdks/create-client-sdks
[S-ts]: https://www.speakeasy.com/docs/sdks/languages/typescript/methodology-ts
[S-auth]: https://www.speakeasy.com/docs/sdks/customize/authentication/overview
[S-oauth]: https://www.speakeasy.com/docs/sdks/customize/authentication/oauth
[S-retry]: https://www.speakeasy.com/docs/sdks/customize/runtime/retries
[S-pagination]: https://www.speakeasy.com/docs/sdks/customize/runtime/pagination
[S-sse]: https://www.speakeasy.com/docs/sdks/customize/runtime/server-sent-events
[S-custom]: https://www.speakeasy.com/docs/sdks/customize/basics
[S-hooks]: https://www.speakeasy.com/docs/sdks/customize/code/sdk-hooks
[S-samples]: https://www.speakeasy.com/docs/sdks/sdk-docs/code-samples/generate-code-samples
[S-features]: https://www.speakeasy.com/docs/sdks/languages/typescript/feature-support
[S-tests]: https://www.speakeasy.com/docs/sdks/sdk-contract-testing
