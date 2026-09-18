# Golden Native SDK Defaults and Complete OpenAPI Support

Date: 2026-09-14

Status: implementation-plan draft, expanded from the SDK comparison discussion.
**Implementation status (2026-09-15): M1, M3, M4, M5 and M9 complete.
Attribution, pagination detection + native runtimes, OAuth lifecycle runtimes
(with runtime OIDC/metadata discovery on the reference backends), and typed
SSE integration span all twelve backends. Callbacks and webhooks are planned
and emitted (TypeScript and Python). Elimination gates are real and measured
for ten languages (esbuild/graph/launcher for TypeScript; binary and symbol
gates for Go, Rust, Swift, C++, Java, Kotlin, C#, Ruby, PHP) with the audit
published. Cost methodology is `repeated-v1`; the OpenAPI matrix is
code-verified including XML's fail-loud contract; the release checklist is
published. The full default suite is green: 215 test binaries, zero failures
and zero warnings. Callbacks/webhooks and OIDC discovery cover every backend.
The M6 401-triggered auth replay covers every backend. Three dialect
compatibility profiles landed (`oas30-nullable-in-3.1-v1`,
`colon-path-parameters-v1`, `schemaless-stream-events-v1`): against the real
assembled OpenRouter spec, admission rose from 52/90 to **85/90 operations**
(94%), the previously-refused chat-streaming operation now generates, colon
path templates render correctly, and OAS-3.0 nullability decodes — verified
end-to-end on regenerated SDKs. Dart's AOT elimination gate runs under Dart
3.9.4 and caught (now fixed) four real emitted-Dart defects; the remaining
five spec refusals are upstream source defects (an RSS endpoint, an undeclared
second scheme, a duplicated operationId, two multipart-extras operations).**

- **M1 complete.** `sdk_defaults` v1 configuration (env-prefix, the pagination
  grammar with aliases, per-operation mappings and disables, and the OAuth
  section), `ua/v1` attribution planning and emission for all twelve backends
  with per-language `userAgent`/application-identity client options and native
  attribution tests, automatic `<ENV_PREFIX>_API_KEY` resolution (explicit
  `credential_env` mappings still win), and the `http_protocol/pagination`
  detection planner with source-linked single-page explanations.
- **M3 complete.** Pagination runtime emission for all twelve backends, with
  native behavioral tests that compile and execute the emitted helpers where a
  toolchain exists on the host.
- **M4 complete (all twelve backends).** Typed stream planning lives in
  `http_protocol` (`StreamPlan`: OAS 3.2 `itemSchema` with SSE/JSON-lines
  framings and typed item codecs, bounded item bytes, explicit refusals for
  unsupported sequential media) and `http_protocol/stream_plan` compiles
  per-operation typed payload, sentinel, and completion semantics from source
  evidence. Every backend emits typed event iteration for discriminated SSE
  operations: declared payload kinds decode into their models, undeclared
  kinds surface through a typed unknown-event alternative, invalid payloads of
  recognized kinds remain decoding failures, an evidence-declared sentinel
  completes the stream before payload decode while preserving the final usage
  frame, and early break/cancel stops consumption without further reads.
- **M5 complete (all twelve backends).** `http_protocol/oauth` compiles
  per-scheme lifecycle descriptors (declared flows, URLs, scopes, client
  authentication, configured client-id/secret variables, refresh skew,
  revocation/introspection endpoints, discovery). Every backend emits a
  first-party token-lifecycle runtime when a usable scheme is compiled:
  token-store interface with an instance-owned memory store, client-credentials
  acquisition with skew-aware caching and single-flight, explicit and on-demand
  refresh with replacement-token adoption, configured
  revocation/introspection helpers, authorization-code + PKCE transactions
  (entropy: /dev/urandom on unix targets for Rust with typed non-unix
  refusal; std::random_device with a repetition sanity check for C++), and
  full RFC 8628 device-authorization polling with cumulative slow-down; typed
  auth errors never carry token values.
- **Published-SDK compatibility diff (2026-09-16).** The published
  `@openrouter/sdk` 1.2.133 and `openrouter` 1.1.153 (both Speakeasy-generated)
  were diffed mechanically against SDKs generated from the upstream assembled
  spec: `docs/SDK-COMPAT-DIFF-TYPESCRIPT.md` and
  `docs/SDK-COMPAT-DIFF-PYTHON.md`, synthesized into the drop-in plan
  `docs/SDK-COMPAT-DROPIN-PLAN.md`. Session operation selection now accepts
  `METHOD /path` selectors for unnamed operations (an emission-completeness
  fix), and the published-SDK diff surfaced three general compatibility
  profiles to land next (OAS-3.0 nullable in 3.1 documents, colon path
  parameters, schemaless SSE) plus `sdk_defaults.globals`/`naming` policies.
- **M2 gates landed.** `tests/typescript_elimination.rs` gates bundler
  elimination (esbuild 0.28.2 profile measurements), a dependency-free
  import-graph reachability model, and generation-level launcher stability;
  single-operation consumer artifacts grow 0–4 bytes when unrelated operations
  are added and pure type imports erase entirely. The three descriptor-data
  initializer defeats the gates found were fixed with `/* @__PURE__ */`
  annotations and a provably-pure stream-descriptor re-emission (an unrelated
  operation's data no longer appears in any single-operation bundle). Measured
  numbers and per-language mechanisms (with CI-pending toolchain gates) are in
  `docs/SDK-ELIMINATION-AUDIT.md`.
- **Costs.** The native-cost harness records `repeated-v1` run sets (cold /
  warmup / steady) per dimension, computes medians/p90/p99 from raw samples,
  and validation recomputes every stored summary from the retained raw runs;
  `tools/sdk-native-costs/run.py` accepts
  `--cold-runs/--warmups/--steady-runs/--report {raw,summary}`.
- **Matrix and guide.** The normative OpenAPI feature matrix is published with
  honest, code-verified statuses in `docs/SDK-OPENAPI-MATRIX.md`; the
  user-facing guide for this program is `docs/SDK-GOLDEN-DEFAULTS.md`.

Remaining: full XML codec emission
(today the contract is fail-loud with source-linked refusals and
bounded-bytes representation — verified and locked by tests); Swift
whole-module retention is a recorded, understood defeat — per-operation
SwiftPM targets are the documented fix, an M0 composition decision; the M8
matrix rows that need new
protocol capability beyond what landed.

Target repository: `/Users/luke/github/suspect`

Intended repository destination: `docs/SDK-GOLDEN-DEFAULTS-PLAN.md`

Configuration and interface examples below are proposed designs, not interfaces already supported by the current CLI. Existing implementation and historical verification are identified separately.

## 1. Goal and priorities

Generate SDKs that feel hand-designed in each language: easy to install, discover, configure, call, stream, paginate, authenticate, and debug. Best-in-class UX/DX, package and bundle size, startup time, and runtime performance are co-primary goals.

Published OpenRouter SDKs are comparison inputs and useful examples of generated interfaces. Preserve their good decisions and prefer compatible interfaces when quality and cost are otherwise equivalent. Intentional changes are appropriate when they improve the goals above; document their benefit and migration recipe.

### Required outcomes

| Area | Requirement |
| --- | --- |
| OpenAPI | Work toward 100% coverage of the normative features in the versioned OpenAPI support matrix, across all twelve native backends. |
| Credentials | Ordinary client construction automatically uses the configured API-key environment convention. |
| Pagination | Default automatic detection of common limit/offset and cursor patterns using parameter-key and response-shape matching. Simple aliases and manual overrides cover unusual names. |
| OAuth | Every backend supplies native first-party functions for the supported OAuth flows and lifecycle: acquisition, token storage, refresh, rotation, and supported revocation/introspection. |
| SSE | Typed, idiomatic streaming with correct framing, completion, cancellation, backpressure, and bounded memory. |
| Validation | Full standard schema validation is optional, through a developer-supplied standard validation library or compiled validator. It is not a required runtime dependency. |
| Packaging | Verifiable elimination of unused code, lean default imports, minimal initialization, optional feature dependencies, and polished native packages. |
| Attribution | Every generated package is identifiable as suspect-generated. Every outgoing request carries a programmatically built User-Agent composed of the suspect version, the package identity (SDK name, or a client-supplied application identifier), the language and language version, and the source OpenAPI version, in a stable parseable order so raw SDK usage and application adoption are separable in API traffic. |
| Compatibility | A preference that yields to demonstrated improvements in DX, fidelity, size, startup, or runtime behavior. |

Scope: TypeScript/JavaScript, Python, Go, Rust, Swift, Java, Kotlin, C#, Ruby, PHP, Dart, and C++.

## 2. Existing foundations and material changes

The current generator already has a shared source-addressed Contract, an HTTP protocol planner, twelve native emitters, native model/codec machinery, environment-credential bindings, installed-package checks, and a native-cost harness.

Important starting points:

- Environment resolution exists across all twelve backends. Constructor shapes and explicit `FromEnv` factories differ; promote automatic resolution into the ordinary onboarding path.
- The September 11 staged comparison reports all 103 operations in its saved source for every backend. That is operation coverage for that input, rather than complete OpenAPI conformance.
- Current standard SSE handling returns parsed envelopes with string `data`. Add explicit typed payload/completion semantics above the framing implementation.
- Pagination helpers and OAuth acquisition/refresh are new behavior. Existing OAuth descriptors mainly describe flows and pass credentials to caller hooks.
- Current full schema evaluation is coupled to native codecs. Separate structural wire decoding from optional full schema validation.
- Python currently loads the whole protocol-plan JSON on first operation use and builds a shared codec lookup. TypeScript currently serializes parsed SSE envelopes before codec decoding. These are concrete profiling and optimization candidates.
- The native-cost harness currently records single-sample observations. Upgrade measurement methodology before establishing performance budgets.

Historical documents are evidence for their pinned snapshots. Recheck moving implementation and generated artifacts when starting each milestone.

## 3. Architecture: one behavior contract, native implementations

```text
OpenAPI documents + SDK configuration + recognized annotations
                           |
             Existing Contract / HTTP protocol plan
                           |
           Shared, versioned SDK-behavior planning module
                           |
        Native interfaces, codecs, protocols, and optional modules
                           |
          Installed-package DX / conformance / cost gates
```

The SDK-behavior module compiles decisions at generation time:

- Environment-variable bindings and credential defaults.
- Resource naming, input conveniences, response projection, and error metadata.
- Pagination selection, exact field accessors, continuation, and termination.
- SSE payload schemas, request selectors, terminal signals, and event projections.
- OAuth flow descriptors, endpoint bindings, token lifecycle, and storage requirements.
- Retry eligibility, body replayability, and deadline policy.
- Optional validator/schema exports and runtime feature dependencies.
- Client attribution: the user-agent template, identity token selection, and per-language version discovery.

Native adapters consume typed descriptors. Avoid rediscovering these decisions in each emitter or parsing a full OpenAPI document in a generated application's hot path.

Reuse useful existing annotations, including supported Speakeasy pagination and stream annotations, through granular interpretation adapters. A naming annotation must not activate unrelated ignore, retry, or legacy behavior.

Every inferred or overridden decision appears in a generation report with its operation, source locations, selected rule, and policy version. Configuration changes participate in generation fingerprints, caches, native compatibility reports, examples, and documentation.

OpenAPI wire rules remain source-derived. Pagination conventions, environment names, and stream sentinels are explicit SDK defaults or interpretations with their own recorded provenance.

## 4. Simple configuration and useful defaults

Default generation enables common pagination detection and source-supported OAuth helpers. A service normally needs only its existing package configuration and one stable environment prefix or credential mapping.

Illustrative minimal policy configuration:

```json
{
  "sdk_defaults": {
    "version": "v1",
    "env_prefix": "OPENROUTER"
  }
}
```

Illustrative expansion of the defaults:

```json
{
  "sdk_defaults": {
    "version": "v1",
    "env_prefix": "OPENROUTER",
    "pagination": {
      "mode": "auto",
      "patterns": ["limit-offset", "cursor", "page-number", "next-link"],
      "page_size": 100
    },
    "oauth": {
      "mode": "auto",
      "storage": "memory",
      "refresh": "on-demand"
    },
    "validation": {
      "mode": "off"
    }
  }
}
```

The page-size value is a proposed SDK fallback, subject to source bounds and M0 approval. It does not assert a server default. Full schema validation is off by default; required structural decoding remains active.

The policy should permit both shorthand values such as `pagination: "auto"` and expanded objects, normalized to one internal representation. Omitted configuration resolves to the documented golden defaults. An explicit per-operation disable remains available.

Existing `credential_env` mappings remain the precise override for multiple or unusual security schemes. One stable service identity should produce the same environment names across all language packages.

## 5. Automatic pagination

### 5.1 Detection and precedence

Pagination must work automatically for conventional APIs without requiring annotations on every list operation.

Resolution order:

1. Explicit per-operation configuration, including an explicit disable.
2. Supported source pagination annotations or explicitly identified pagination links.
3. Built-in key/shape patterns, extended by configured aliases.
4. Ordinary single-page operation with a source-linked explanation if inference is ambiguous or incomplete.

Matching runs at generation time against resolved, effective parameters and success response schemas. It uses parameter identity/location, compatible types, a collection response, and a complete continuation rule. A parameter named `limit` alone is insufficient.

Use deterministic normalized key comparison for common casing/separator variants. Preserve actual wire names and case. Avoid edit-distance guesses or matching arbitrary substrings. User aliases extend the built-in vocabulary.

Start automatic parameter matching with query parameters on collection-reading operations. Explicit metadata or configuration supports paginated POST/search operations, body-contained pagination, and unusual header-based controls. Unnamed operations can be selected by method/path or source identity.

### 5.2 Built-in patterns

| Pattern | Request candidates | Response evidence |
| --- | --- | --- |
| Limit/offset | Size: `limit`, `page_size`, `pageSize`, `per_page`, `perPage`, `take`, `max_results`, `maxResults`; position: `offset`, `skip`, `start_index`, `startIndex` | A collection; preferably a next offset, `has_more`/`hasMore`, or a total count. |
| Cursor/token | `cursor`, `page_token`, `pageToken`, `continuation_token`, `continuationToken`, `after`, `starting_after`, `startingAfter`; optional size parameter | `next_cursor`, `nextCursor`, `next_page_token`, `nextPageToken`, `next_token`, or an unambiguous continuation-token field. |
| Last-item cursor | `after` or `starting_after`, optional size | Collection plus `has_more`, with an explicit last identifier or a verified item-ID mapping. |
| Page number | `page`/`page_number`/`pageNumber`, plus a size parameter | Next page, total pages, or a documented page-number convention. |
| Next link | A supported body link or RFC 8288 `Link` relation | A uniquely identified `next` link and its termination convention. |

Recognize a top-level array and common array fields such as `data`, `items`, `results`, and `records`, including a bounded set of common envelope paths. Recognize metadata containers such as `pagination`, `meta`, and `page_info` when the full mapping is unambiguous.

Cursor tokens retain their original value/type and remain opaque. A field named `next` can denote a URL, cursor, or page number; type and pattern evidence must disambiguate it.

### 5.3 Slightly different names: aliases and manual matching

Global aliases should be enough when an API consistently uses different vocabulary:

```json
{
  "sdk_defaults": {
    "pagination": {
      "mode": "auto",
      "aliases": {
        "limit": ["batch_size"],
        "offset": ["from_row"],
        "cursor": ["continuation"]
      }
    }
  }
}
```

Operation overrides provide exact matching when request names or response envelopes differ:

```json
{
  "sdk_defaults": {
    "pagination": {
      "mode": "auto",
      "operations": {
        "listInvoices": {
          "pattern": "limit-offset",
          "request": { "limit": "maxRows", "offset": "fromRow" },
          "response": { "items": "/entries", "total": "/paging/count" },
          "initial_offset": 0,
          "advance": "items-returned"
        },
        "listAuditEvents": {
          "pattern": "cursor",
          "request": { "limit": "batchSize", "cursor": "continuation" },
          "response": {
            "items": "/payload/events",
            "next_cursor": "/paging/next"
          },
          "stop": "missing-or-null-cursor"
        },
        "getReport": false
      }
    }
  }
}
```

Request strings name actual query parameters. An expanded locator form supports other parameter locations and request-body pointers. Response pointers are JSON Pointers relative to the decoded response body, not schema-document pointers.

An override may replace only a name or pointer while retaining the rest of an inferred pattern. Validate targets, types, response alternatives, source bounds, and contradictory configuration before emitting helpers. Missing or ambiguous mappings must remain actionable generation findings.

### 5.4 Traversal semantics

- A normal list call obtains one page. Item/page iteration lazily requests subsequent pages, including the already-fetched first page exactly once.
- Expose native item iteration, page iteration, and explicit next-page access. An eager collect-all helper is explicit.
- Preserve the original filters, sort order, headers, selected representation, credentials, and request options. Change only pagination controls.
- Respect caller values first. Use source defaults/bounds for helper defaults; otherwise use the documented SDK convention. An explicit zero-valued offset is a value.
- Prefer server-provided continuation metadata. For conventional offsets without it, advance by items returned unless a declared/configured rule specifies another stride.
- When only limit/offset and an array are known, use the documented empty-page fallback rather than silently assuming a short page means completion. A server can cap the requested limit. More efficient termination applies when effective size or completion metadata is known.
- An empty cursor page with a valid next cursor can continue. Missing, null, and empty-string tokens have explicit pattern rules; an empty string is not universally completion.
- Prefer explicit `next_offset`/cursor/has-more evidence over inferred totals or short-page rules. Contradictory authoritative metadata or non-progress produces a pagination error.
- Memory stays proportional to page size and bounded continuation bookkeeping. Detect repeated/non-advancing continuation values; expose optional page/item limits and resumable checkpoints.
- Default to one request at a time. Early break, cancellation, timeout, or an item limit prevents extra requests. Prefetch is an explicit, bounded option if later measurements justify it.
- A continuation retry repeats the current page request before advancing state. Helpers do not claim snapshot consistency or exactly-once records when the server's collection is changing.
- Next-link resolution follows declared URL and credential-origin policies. A continuation URL does not automatically gain authority to receive another origin's credentials.

Compile request setters, response accessors, and continuation rules into native code. Common pagination must not require a JSONPath library or a runtime schema search.

### 5.5 Acceptance cases

Cover zero-config `limit`/`offset`, cursor/next-cursor, common aliases, user aliases, partial/full manual mappings, disable overrides, nested envelopes, top-level arrays, competing candidates, wrong types, and mixed response alternatives.

Runtime cases include missing totals, server-clamped page sizes, empty pages with continuation, nonzero starts, opaque cursor values, repeated cursors, retries, cancellation during a next-page request, early break, preserved filters, OAuth refresh between pages, and stable memory over long traversals.

## 6. Automatic API-key environment handling

Ordinary construction in each language uses the configured environment policy; a special `FromEnv` factory is an optional explicit spelling.

- A single unambiguous bearer/API-key scheme can use `<ENV_PREFIX>_API_KEY`. Multiple schemes use explicit source-scheme mappings when a convention would be ambiguous.
- Source declarations determine bearer versus header/query/cookie attachment. The scheme's name alone does not determine attachment.
- Explicit credentials or an explicit credential provider take precedence. An explicit auth-disabled value prevents fallback.
- Resolve names once during generation and values per client at construction. Importing or generating an SDK reads no credential values.
- Specify omission, native nullish values, empty strings, and partial credential-object behavior in each golden constructor example. Empty credentials must not accidentally select another identity.
- Missing credentials fail before a protected request with a useful variable/scheme name. Public operations remain usable.
- Keep browser/portable entrypoints independent of unconditional process, Node, filesystem, or IO imports. Applications can inject environment/credential providers where process environment access is unavailable.
- Credential-provider and OAuth-manager interfaces support rotation. Environment snapshots do not become hidden global mutable state.

## 7. Suspect attribution header

Every outgoing HTTP request from a generated SDK carries a User-Agent that identifies both the generator and the client. The value is assembled programmatically from generation-time descriptors plus one runtime-discovered component, so raw SDK usage and identified application adoption are separable in API traffic, and every request is identifiable as suspect-generated.

### 7.1 Value grammar

A fixed, parseable token order in RFC 9110 product/comment form, versioned as `ua/v1`:

```text
suspect/<suspect-version> <identity>/<identity-version> (<language>/<language-version>; openapi/<spec-version>)
```

- `suspect/<suspect-version>` is always the first token: the suspect generator version captured at generation time.
- The identity slot is the SDK package name and version by default. A client-supplied application identifier replaces it, so default traffic and identified application traffic are distinguishable while every request still carries the suspect token.
- `<language>/<language-version>` is discovered at client construction in the platform's native idiom (for example `python/3.12.1`, `go1.22`, or the Node runtime version for TypeScript packages). When the platform cannot report a version, the token degrades to `unknown` rather than being omitted or breaking the format.
- `<spec-version>` is the source document's declared OpenAPI or Swagger version (2.0 through 3.2.x), a generation-time constant.

Examples, default identity then application adoption:

```text
User-Agent: suspect/0.42.1 openrouter-python/1.2.0 (python/3.12.1; openapi/3.1.0)
User-Agent: suspect/0.42.1 acme-chatbot/7 (python/3.12.1; openapi/3.1.0)
```

### 7.2 Behavior

- Constant parts (suspect version, package identity, spec version, template) are compiled into each backend from the shared descriptor. No emitter re-implements formatting, and generated applications never parse the OpenAPI document to produce the header.
- Language version is discovered per client at construction with a graceful `unknown` fallback. Targets without a runtime-discoverable version (for example Rust and C++) use a build-time-detected compiler version or `unknown`; the per-language choice is settled in M0.
- The application identifier is an ordinary client option spelled in each language's idiom. Values are validated as RFC 9110 tokens with a bounded length. An empty value means unset and must not produce a blank token. Documentation notes that the identifier is visible on the wire and must not contain private information.
- An explicit caller-supplied `User-Agent` header wins over the automatic value, and a disable option exists for callers that must suppress it.
- Every SDK-issued request carries the header: operations, pagination pages, SSE streams, and OAuth token/revocation/introspection endpoints, unless overridden or disabled. The header never contains hostnames, paths, user identity, or other environment-derived values.
- The template and its provenance appear in the generation report, generated documentation, and examples. Format changes are compatibility events: versioned, recorded, and migration-noted like other interface changes.
- A machine-readable companion header (JSON metadata in a second header) is deliberately out of scope for `ua/v1`; if added later it must be a separately versioned extension.

## 8. Golden SSE and native iteration

Build one semantic stream plan and implement it with native language primitives.

Shared behavior:

1. Parse UTF-8 incrementally and implement SSE framing, comments, multiline data, field rules, and EOF behavior.
2. Decode declared JSON payloads directly into typed events. Interpret content annotations/stream metadata explicitly; arbitrary SSE text remains representable.
3. Apply configured sentinels at the correct framing stage before JSON decoding. `[DONE]` is an API convention rather than a universal SSE rule.
4. Preserve final usage/completion data and all declared event kinds, including tool and non-text events.
5. Define protocol-declared fatal events and ordinary application events separately; preserve the original event payload in native errors where a fatal-event policy applies.
6. Represent future event types through a typed unknown-event alternative. Invalid payloads for recognized event types remain decoding failures.
7. Expose status/headers/event metadata and an explicit raw-frame interface without retaining every prior event.
8. Bound per-event and in-flight buffers independently of optional total bytes, events, and duration limits.
9. Propagate cancellation and consumer cleanup to the underlying body. Returning a stream leaves its request context alive until consumption ends.
10. Stream-specific helpers set the declared request selector and select the correct response type. A stream request has no unnecessary JSON-or-stream result union.

### Native shapes across all twelve languages

These iteration idioms also guide item/page traversal; buffered pages do not acquire unnecessary stream-disposal requirements.

| Target | Streaming / traversal interface | OAuth/control idiom |
| --- | --- | --- |
| TS/JS | `AsyncIterable<Event>`, `AbortSignal`, Web Streams interoperability | Promise-based helpers, abortable provider calls, shared in-flight refresh promise |
| Python | Sync/async iterators and context managers | Sync and async OAuth clients/stores; async paths avoid blocking the event loop |
| Go | Typed `Next`/`Value`/`Err`/`Close`, context first | Context-aware helpers, native errors, coordinated refresh |
| Rust | `Stream<Item = Result<Event, StreamError>>`, ownership-based cleanup | Native futures/results, runtime-compatible coordination, explicit transport features |
| Swift | Pull-driven `AsyncSequence`, task cancellation | Async/await, actor-safe token state, Sendable policy |
| Java | Closeable iterator and standard `Flow.Publisher` | Sync/`CompletableFuture` helpers, native concurrency and ownership |
| Kotlin | Coroutine-native `Flow<Event>` | Suspending helpers/stores, coroutine-aware refresh coordination |
| C# | `IAsyncEnumerable<Event>`, `CancellationToken` | Task-based helpers, async stores and disposal |
| Ruby | Blocks/`Enumerator` with reliable cleanup | Keyword helpers, native exceptions, appropriate thread/fiber coordination |
| PHP | Closeable iterable/generator | Synchronous typed helpers and stores; persistent stores provide cross-process coordination |
| Dart | Single-subscription `Stream<Event>`, pause/cancel propagation | Future-based helpers/stores, single-flight refresh, portable/IO separation |
| C++ | RAII-owned pull stream, `stop_token` | Native outcomes, explicit ownership, transport-compatible synchronization |

Document request-start semantics for each native form. For example, collecting a cold Kotlin Flow initiates an exchange; repeated collection must not be mistaken for replaying a buffered result.

## 9. First-party native OAuth lifecycle support

### 9.1 Required capabilities in every backend

Each backend must emit native OAuth functions rather than leave the application to hand-assemble token requests. Generate service-bound helpers from source flows and supported metadata, with small explicit configuration for information the description omits.

| Capability | Native functions / behavior |
| --- | --- |
| Authorization code | Create an authorization transaction/URL, generate state and PKCE S256 values, validate the callback transaction, exchange the code for tokens. |
| Client credentials | Acquire a token using supported client authentication, cache it, and reacquire when expired if no refresh token is issued. |
| Device authorization | Start device authorization; poll with cancellation, expiry, `authorization_pending`, and `slow_down` handling; return a typed token set. |
| Refresh | Explicit refresh function and automatic on-demand refresh when supported and needed. |
| Storage | Native token-store interface plus built-in in-memory storage; load, atomically replace, and clear a token set. |
| Rotation | Atomically adopt replacement refresh/access tokens, coordinate concurrent refreshes, and expose application credential/key-provider rotation where applicable. |
| Revocation | Native revocation helper when an endpoint is declared, discovered, or configured; explicit local session clearing. |
| Introspection | Typed introspection helper when supported; preserve its authentication requirements and active/inactive result. |
| OAuth/OIDC metadata | Discover and cache metadata from a declared/configured authority; bind issuer, endpoints, scopes, and supported client authentication. |
| OpenID Connect | Where enabled, use an established verification adapter for ID-token signature/issuer/audience/time/nonce checks and verification-key refresh. |
| Deprecated source flows | Preserve and represent OpenAPI implicit/password flow declarations. Any execution support belongs to explicitly selected compatibility helpers; authorization code with PKCE is the normal interactive default. |

Use standard OAuth endpoint semantics. Honor a declared `refreshUrl`; use the token endpoint for standard refresh when supported and no distinct refresh URL is declared. Revocation, introspection, and other optional endpoints require metadata/configuration because OpenAPI flow objects do not universally supply them.

The current OpenRouter bearer scheme does not by itself describe an OAuth authorization server. OAuth capability is native to every backend, while each generated package binds only the flows its source or explicit configuration supports.

Authorization transactions are bound to their client/session and consumed once. Application navigation, redirect routing, or device-code display integrates with native helpers; importing an SDK or making an API-key call does not start an interactive login.

Public-client profiles use PKCE without embedding a client secret. Confidential-client profiles support declared/configured client authentication. Signing and certificate-based profiles use established native cryptography/transport adapters.

### 9.2 Token storage and refresh correctness

- Default to an instance-owned in-memory token store. Persistence is an explicit choice.
- Supply first-party optional storage adapters for supported OS credential stores, alongside an application-supplied store interface for databases, secret stores, and distributed deployments.
- Browser persistence and OS/IO dependencies use explicit entrypoints. Storage adapters must declare their supported platforms and lifecycle.
- Store the access token, token type, refresh token if present, known expiry, granted scope, and relevant issuer/account/audience identity. Preserve provider extensions where appropriate.
- Partition stored credentials by authorization context; different users, issuers, clients, audiences, or tenants must not share an accidental global token cache.
- Compute expiry from supported token metadata and an injectable clock, with bounded configurable refresh skew. Opaque tokens do not imply JWT expiry claims.
- If a refresh response contains a replacement refresh token, atomically replace the token set. If it omits one, retain the existing refresh token when the protocol permits that behavior.
- Coordinate refresh before contacting the token endpoint. A shared persistent store needs a refresh lease/lock as well as versioned atomic replacement; compare-and-swap after two simultaneous refresh requests is insufficient for rotating tokens.
- Re-read token state after acquiring refresh ownership. Prevent a late response from overwriting newer credentials.
- Define cancellation and ownership for refresh waiters: cancelling one waiter must not incorrectly cancel another caller's active acquisition or leak a refresh task after client shutdown.
- Use on-demand refresh as the normal lifecycle. Background refresh, if added, has explicit scheduling and disposal costs.
- A qualifying invalid-token response may trigger one coordinated refresh and an eligible request replay. Apply the same replayability/idempotency rules as normal retries; delivered stream data prevents transparent restart.
- Revocation, invalid grants, denied consent, malformed responses, storage failures, and unsupported refresh produce typed authentication failures. Errors and default logging exclude token values.
- Rotation means honoring the server's token lifecycle. Generate API-key rotation endpoints only where the API describes them; an API-key string does not imply a remote rotation operation.

### 9.3 Configuration

Source declarations supply flows and URLs. Optional scheme configuration supplies client identity, environment names, endpoint/discovery overrides, and lifecycle settings:

```json
{
  "sdk_defaults": {
    "oauth": {
      "mode": "auto",
      "schemes": {
        "userOAuth": {
          "client_id_env": "EXAMPLE_CLIENT_ID",
          "client_secret_env": "EXAMPLE_CLIENT_SECRET",
          "refresh_skew_seconds": 30
        }
      }
    }
  }
}
```

Client-secret configuration applies only to confidential-client usage. Runtime construction can supply a native token store, clock, credential provider, or existing HTTP transport. Store instances and credential values are runtime inputs, not generated configuration values.

### 9.4 OAuth qualification

Use controlled authorization/token servers to test code/PKCE transactions, client credentials, device polling, refresh with and without replacement tokens, expiry/skew, storage restart, concurrent refresh, shared-store rotation, revocation/introspection, metadata refresh, error paths, cancellation, and retry interactions.

Exercise the public generated functions in every native package. Include a paginated collection that crosses token expiry and an SSE exchange whose credentials expire after the stream starts.

## 10. Optional standard schema validation

Full schema validation must be optional in both runtime behavior and dependency/package reachability.

### Default path

- Generate lean serialization/deserialization and the structural checks needed to construct the advertised native types and follow HTTP framing.
- Keep required presence, nullability, omission, wire names, correct union decoding, and lossless numeric transport where those are part of the native representation.
- Separate these checks from exhaustive schema assertions such as all composition constraints, patterns, ranges, and unevaluated-location rules.
- Do not describe structural decoding as full JSON Schema validation.

### Opt-in interface

- Expose a small native validator interface accepting a schema identity/resource closure, its dialect, validation direction, and the value or supported exact representation.
- Allow a developer-supplied standards-based library or precompiled validation functions. Provide lightweight first-party adapters for selected ecosystem libraries where their supported semantics are verified.
- Allow request-only, response-only, or both, with client defaults and per-operation/per-call controls.
- Emit standard schema resources through optional schema entrypoints/artifacts, preserving reference resolution, `$id`, anchors, dynamic references, and dialect identity. OAS 3.0 schema projection must document its semantic conversion.
- Compile/cache validators once per reachable schema and provider configuration. Providers that support precompilation should permit zero runtime schema compilation.
- Keep validator libraries, schema registries, and full schema documents outside the unvalidated core dependency graph. A disabled boolean alone is insufficient if imports still retain their cost.
- Validation observes the original value by default. Library coercion, stripping, or default insertion requires a separately explicit transformation policy.
- A provider must identify its dialect/vocabulary and exact-number capabilities. Unsupported capabilities or incomplete evaluation produce explicit outcomes, rather than being reported as successful validation.
- Preserve useful schema/instance paths in validation failures. Request failures occur before sending; response validation failures retain HTTP status and request metadata.

Keep generation-time conformance/example checking independent of this runtime choice. Users should be able to generate faithful models and run full contract checks in CI while shipping an SDK client without a mandatory schema-validation engine.

Existing fully checked codecs may remain available through an optional adapter/export if their cost and semantics are useful. Their implementation must not remain an implicit dependency of the default client.

## 11. Ordinary interfaces, request policy, and native interoperability

Golden examples should cover construction, one request, typed failures, streaming, pagination, OAuth, upload/download, cancellation, and model serialization.

Recommended interface decisions to validate in M0:

- Readable resource namespaces, stable names, and simple body arguments for straightforward operations.
- Body-first ordinary results, with a native metadata interface when requested. Preserve actual API envelopes; eliminate unnecessary SDK-only response layers.
- A single well-understood native error family with useful categories, HTTP status, operation, request ID, and causes. Invalid error bodies must not erase the HTTP failure context.
- Forward-compatible response enums/events with an explicit unknown representation. Request types remain source-directed; full source assertions are available through the validator interface.
- Faithful bounded native numbers where possible and compact exact carriers elsewhere. Provide checked conversions and standard arithmetic-library interoperability; guard accidental lossy coercion.
- Native JSON interop through generated converters where feasible. Where a platform serializer cannot preserve the representation, supply a clear, small conversion interface beside the first example.
- Reusable connection pools and clear owned-versus-borrowed transports. Closing an SDK must respect caller-owned transport lifetimes.
- Streaming uploads/downloads using native byte/file interfaces; large binary bodies and long SSE streams have separate limits from buffered JSON.

Proposed request defaults:

- At most two automatic retries, with bounded jitter and `Retry-After` support for eligible 429/transient responses.
- Retry eligibility depends on method/operation semantics, idempotency support, and body replayability. Token exchanges and rotating refresh requests require their own explicit retry treatment.
- Unify the overall attempt budget across transport retry and auth-refresh replay so nested policies cannot multiply attempts unexpectedly.
- Separate connection/headers, stream-idle, and optional total deadlines. Per-call controls remain native and override client defaults.
- Pagination, OAuth, normal calls, and streams use the same cancellation/error-policy interfaces.

Confirm numeric timeout/backoff values using representative workloads during M0; feature implementation must not silently inherit a one-hour legacy retry budget.

## 12. Package architecture and perfect tree shaking

Treat elimination of unused code as a tested design requirement.

### Package requirements

- Export a lean core plus resource, operation, model, OAuth, storage, and validation entrypoints with explicit dependency graphs.
- Statically importing one operation/resource must not retain unrelated operations, codecs, schema validators, OAuth grants, storage adapters, or platform shims.
- Prototype the ordinary root-client interface before freezing it. An all-resources class or factory can keep every referenced method reachable. If the proposed default prevents dead-code elimination, change its composition/import shape to satisfy the goal.
- Resource composition and a separately explicit full-client entrypoint are candidate designs. The normal recommended path must have first-class size and startup budgets.
- Verify public barrel exports and side-effect declarations. Avoid module-level registration, whole-contract initialization, and eager per-client allocation of every operation closure.
- Share reachable codecs where semantics permit, including recursive groups, while keeping documentary provenance and optional schema evaluation outside hot initialization.
- Use native platform transports and standards libraries where they are a good fit. Every additional runtime dependency has a measured, documented purpose.
- Keep consumer dependency ranges compatible with the supported ecosystem; pin build/test tooling independently.
- Publish native metadata, license declarations, exports, typing artifacts, and concise install/auth/first-call instructions. Larger references and provenance have their own artifacts or documentation surfaces.

### Tree-shaking acceptance

Test empty import, core-only, one JSON operation, one resource, one SSE operation, one pager, API-key-only, individual OAuth flows, individual storage adapters, and validation off/on.

Inspect bundler dependency graphs and emitted bytes using at least esbuild and Rollup/Vite on pinned versions. Adding an unrelated operation to the source must not increase a single-operation consumer artifact.

Count asynchronous chunks and copied assets in addition to the entry chunk. Lazy loading changes startup behavior but does not prove that unused code was eliminated.

Scope the elimination guarantee to published static-import/build profiles. Dynamic resource selection has a documented full-surface cost. Pure type imports must erase entirely.

For other languages, apply the equivalent package/feature/linker/AOT/autoload discipline: measure reachable dependencies, compile cost, linked artifacts, class/module initialization, and reflection/trimming compatibility. Language-specific mechanisms share the same unused-feature cost goal.

## 13. Complete OpenAPI coverage program

Build a normative feature matrix for OpenAPI 2.0, 3.0.x, 3.1.x, and 3.2.x. Support older document syntax through generation-time frontends and the shared contract so generated packages can retain modern native defaults.

Record the exact specification patch editions used as test references. Standard support and extension/provider profiles have distinct capability records.

| Feature family | Completion scope |
| --- | --- |
| Documents and references | JSON/YAML, split documents, components, reference siblings, canonical URIs, `$self`, `$id`, anchors, dynamic references, and version/dialect context. |
| Schemas and native models | All normative schema constructs, compositions, recursion, discriminators, directions, presence/null, exact numbers, extras, XML annotations, and optional standard validation integration. |
| Operations | All source operations, unnamed operations, standard/custom methods, tags, deprecation, naming collisions, and source identity. |
| Servers and security | Server variables/relative bases, security overrides and OR/AND alternatives, API keys, HTTP schemes, mutual TLS, OAuth/OIDC, and required transport capabilities. |
| Parameters | Path/query/header/cookie/querystring locations, styles, explode, content-based parameters, encoding, and exact wire names. |
| Media | JSON, text, binary, forms, multipart including positional/nested/streamed encodings, XML, media negotiation, and extension media adapters. |
| Responses | Exact/range/default dispatch, bodyless responses, multiple success representations, typed headers, links, and unexpected response handling. |
| Sequential content | SSE, JSON-lines/other described sequential media, complete-content versus item schemas, item metadata, and binary streams. |
| Callbacks and webhooks | Correct incoming direction; native typed request decoding/response construction and framework adapters, plus the described runtime expressions. |
| Documentation and artifacts | Examples, descriptions, external documentation, native references, package metadata, deterministic regeneration, and compatibility reports. |

Track source parsing, semantic planning, emitted representation, native runtime behavior, optional-validator capability, and installed-package evidence separately for each matrix row and target.

A valid feature that needs a richer native representation should get the best faithful model plus an explicit lossless fallback where necessary. Unknown custom vocabularies/extensions use declared adapters; arbitrary extension semantics cannot be inferred from their names.

Every valid normative feature needs a supported executable path in each language's declared platform/adapter matrix. Platform restrictions must remain visible; for example, browser transport restrictions are not resolved by inventing different HTTP semantics.

Resolve all applicable unsupported/refusal paths before claiming completion. Operation counts alone, retained metadata alone, or raw-body fallbacks alone do not establish complete runtime support for a described feature.

## 14. Measurements and release budgets

| Dimension | Required observations |
| --- | --- |
| Distribution | Archive bytes, installed runtime bytes, schema/docs assets, transitive dependencies. |
| JS consumption | Minified/gzip/Brotli bytes, all chunks/assets, root/resource/operation profiles, unused-code reports. |
| Startup | Module/import time, client construction, metadata initialization, first useful request/event. |
| Runtime | Warm request overhead, codec throughput, allocations, pooling, representative concurrency. |
| SSE | First-event overhead, per-event CPU/allocations, slow-reader behavior, retained memory over long streams. |
| Pagination | Request counts, per-page overhead, early-break behavior, retained memory across many pages. |
| OAuth | Cached-token fast path, refresh latency/coordination, persistence cost, unrelated-flow elimination. |
| Validation | Disabled-path cost, compile/cold cost, warmed validation, provider and schema reachability. |
| Native/IDE | Clean/incremental compile, declarations/type checking, editor responsiveness, linked/AOT artifacts, module/class initialization. |

Use pinned hosts/toolchains, repeated cold runs, explicit warmup, steady-state batches, medians/tails, and allocation/memory measurements. Separate runtime startup from the operation being measured.

Compare the current staged output, published SDK consumer paths where comparable, and minimal native HTTP references. Pin source/operation/payload sets and record semantic differences such as validation and retry policy. Benchmark defaults as experienced by users, plus equivalent-behavior cases.

Establish numerical budgets during M0. Every milestone reports incremental cost and enforces the budgets for its affected profiles. Size and performance are acceptance criteria throughout development.

## 15. Implementation milestones and dependencies

| Milestone | Deliverable and exit condition | Depends on |
| --- | --- | --- |
| M0: golden interfaces and baselines | Executable interface sketches for all twelve languages; versioned feature matrix; repeated cost baselines; proof of a lean, tree-shakable default import/composition design; attribution header grammar with per-language version-discovery choices; approved policy values. | None |
| M1: behavior planning and configuration | Shared descriptors and the attribution template, default/alias/manual matching grammar, source-linked explanation reports, env conventions, cache/compatibility integration, and deterministic configuration tests. | M0 |
| M2: lean runtime and optional validation | Separate structural codecs from full validation; modular runtime/schema/validator exports; native validator seam; root/resource/operation size and startup gates. | M1 |
| M3: automatic pagination | Default limit/offset and cursor inference, aliases and partial/full overrides, native items/pages/next helpers, all traversal and request-count acceptance cases. | M1, M2 |
| M4: typed SSE and stream lifetime | Typed payload/completion planning, efficient framing/decoding, native consumption and metadata, cancellation/backpressure/long-stream memory gates. | M1, M2 |
| M5: OAuth functions and token lifecycle | First-party flows, stores, metadata, refresh/rotation coordination, supported revocation/introspection, native errors and controlled-server evidence. | M1, M2 |
| M6: integrated request and model DX | Unified retries/deadlines/auth replay, response/error interfaces, model interop, uploads/downloads, and integration scenarios spanning pages/streams/OAuth. | M3, M4, M5 |
| M7: all-twelve native qualification | Complete implementation in each backend, installed-package golden examples, supported platform/toolchain checks, optional dependency profiles, attribution presence/override checks, and native cost gates. | M2-M6 |
| M8: full OpenAPI closure | Complete remaining frontend/protocol/schema/callback/webhook/transport rows in the normative matrix, with executable native paths and explicit standard-validation capabilities. | M1 onward; final exit requires M7 |
| M9: release packages and migration | Publishable native packages, polished references/quickstarts, migration notes, compatibility review, and complete conformance/DX/performance qualification. Checklist published: `docs/SDK-RELEASE-CHECKLIST.md` — generation inputs, pre-publish gates, per-backend package metadata, and the compatibility-report release gate; the manual sign-offs that remain (toolchain pins, registry accounts, license/release-readiness promotion, review) are listed in that checklist. | M7, M8 |

Use TypeScript, Python, Go, and Rust as early end-to-end reference implementations for M2-M6. Design all twelve interfaces in M0 and bring the other eight through the same behavioral fixtures once shared decisions stabilize. Completion always includes all twelve backends.

Large conformance gaps discovered in M0 become explicitly dependency-linked work under M8; preserve their visibility while delivering the default-DX improvements.

### Existing implementation seams

- `crates/suspect-codegen/src/backend/options.rs`: canonical generation policy.
- `crates/suspect-codegen/src/credential_env.rs`: existing source-bound env mapping.
- `crates/suspect-codegen/src/http_protocol/`: shared source/wire descriptors.
- `crates/suspect-codegen/src/generation_session.rs` and `compatibility/`: identity and interface changes.
- Native planning, emitters, runtime assets, package emitters, and example generators under `crates/suspect-codegen/src/`.
- `tools/sdk-native-costs/` and `crates/suspect-codegen/tests/sdk_native_measurements.rs`: cost methodology and profile gates.

New SDK-behavior, pagination, and OAuth planning modules should use these existing seams. Generated-source post-processing should not become a second implementation of the SDK contract.

## 16. Final acceptance

- [ ] Ordinary client construction uses native environment credentials in every language.
- [ ] Every request from every generated package carries the versioned suspect attribution User-Agent; application-identifier separation, explicit override, and disable behavior are tested in every backend.
- [ ] Conventional limit/offset and cursor APIs get automatic pagination with zero per-operation configuration.
- [ ] Global aliases, partial overrides, full manual mappings, and explicit disable work and produce traceable reports.
- [ ] Native item/page traversal is lazy, cancellable, replay-aware, and memory-bounded.
- [ ] Every backend supplies the supported native OAuth functions and token-store interface.
- [ ] Token refresh and replacement are correct under concurrent calls and persistent shared storage.
- [ ] Typed SSE consumes sentinels appropriately, preserves completion data, and handles cleanup and long streams correctly.
- [ ] Full standard schema validation is optional, developer-supplied, dialect-aware, and absent from the default runtime dependency graph.
- [ ] Default decoding preserves its advertised native-type and wire guarantees without claiming exhaustive validation.
- [ ] Unused operations, resources, OAuth flows, storage adapters, and validators disappear from supported consumer build profiles.
- [ ] The ordinary recommended client shape passes the agreed startup and size budgets.
- [ ] All normative OpenAPI matrix rows have the required generation/native evidence before the 100% claim is made.
- [ ] Every package has executable installed-consumer quickstarts, native references, supported toolchain metadata, and release/licensing metadata.
- [ ] Compatibility changes have migration recipes and a documented quality or cost benefit.
- [ ] Cold/warm, stream, pagination, OAuth, validation, and native build/IDE budgets pass for the supported profiles.

## 17. Decisions to settle in M0

These decisions refine implementation rather than reopen the required outcomes:

1. The exact lean root/resource composition interface that proves the tree-shaking requirement while keeping onboarding short.
2. Final body-first response/metadata spelling and per-language argument conveniences.
3. Default unknown-enum/event representation and exact-number interoperability details.
4. Final pagination alias vocabulary, fallback page size, and ambiguity diagnostics.
5. First-party persistent token-store platform matrix and the initial standard validator adapters.
6. Numeric timeout/retry/skew settings and measured performance budgets.
7. Attribution header details: identity-slot spelling when an application identifier is present (replace versus retain the SDK name), per-language version-discovery mechanisms and fallbacks, client option names, and the disable/override surface.

## References

Repository context:

- `docs/SDK-CREDENTIAL-ENV.md`
- `docs/SDK-HTTP-PROTOCOL.md`
- `docs/SDK-INTERPRETATION-PROFILES.md`
- `docs/SDK-NATIVE-DX.md`
- `docs/SDK-DX-REVIEW-20260911.md`
- `docs/SDK-NATIVE-COSTS.md`
- `docs/SDK-CAPABILITIES.md`

Primary references:

- [Published versus staged OpenRouter SDK comparison](https://github.com/OpenRouterTeam/openrouter-web/blob/chore/stage-generated-sdks/docs/engineering/sdk-published-vs-staged-2026-09-11.md)
- [Published SDK baseline and pinned sources](https://github.com/OpenRouterTeam/openrouter-web/blob/chore/stage-generated-sdks/docs/engineering/sdk-published-baseline-2026-09-11.md)
- [OpenAPI 3.2 specification](https://spec.openapis.org/oas/v3.2.0.html), especially media/SSE, schemas, security schemes, OAuth flows, callbacks, and webhooks.
- [RFC 6749: OAuth 2.0](https://www.rfc-editor.org/rfc/rfc6749)
- [RFC 7636: PKCE](https://www.rfc-editor.org/rfc/rfc7636)
- [RFC 8628: Device Authorization](https://www.rfc-editor.org/rfc/rfc8628)
- [RFC 8414: Authorization Server Metadata](https://www.rfc-editor.org/rfc/rfc8414)
- [RFC 7009: Token Revocation](https://www.rfc-editor.org/rfc/rfc7009)
- [RFC 7662: Token Introspection](https://www.rfc-editor.org/rfc/rfc7662)
- [RFC 9700: Current OAuth security practice](https://www.rfc-editor.org/rfc/rfc9700), including PKCE and refresh-token rotation.
- [RFC 8288: Web Linking](https://www.rfc-editor.org/rfc/rfc8288)
- [JSON Schema specifications](https://json-schema.org/specification)

Verify the selected specification editions and provider-library capabilities as implementation begins. Normative requirements, SDK conventions, and provider-specific behavior must remain distinguishable in the generated capability evidence.
