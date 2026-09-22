# Golden SDK defaults — user guide

`sdk_defaults` is the application-selected generation policy that turns a
generated SDK from "a set of operations" into "a client that feels
hand-designed": one stable environment-variable convention for credentials,
automatic pagination helpers, a versioned attribution header on every request,
and an explicit OAuth lifecycle policy. It is **v1** configuration: absent
fields resolve to documented defaults, unknown fields are errors, and every
accepted value participates in the generation fingerprint, compatibility
reports, and captures.

This guide describes what is generated **today**. Where a behavior is still
planned, the text says so explicitly. The implementation plan is
[SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md); the credential
runtime semantics are specified in
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md); the normative OpenAPI support
matrix is [SDK-OPENAPI-MATRIX.md](SDK-OPENAPI-MATRIX.md).

- Applies to: all twelve language packages (TypeScript/JavaScript, Python, Go,
  Rust, Swift, Java, Kotlin, C#, Ruby, PHP, Dart, C++).
- Configuration only: `sdk_defaults` never reads an environment variable, a
  file, or a remote resource at generation time. Values are resolved per client
  at construction, in the generated package.

## 1. The `sdk_defaults` configuration

The policy is a top-level field of the SDK generation configuration (session
JSON; forwarded through generation options like `credential_env`):

```json
{
  "sdk_defaults": {
    "version": "v1",
    "env_prefix": "OPENROUTER",
    "pagination": {
      "mode": "auto",
      "patterns": ["limit-offset", "cursor", "page-number", "next-link"],
      "aliases": {
        "limit": ["batch_size"],
        "offset": ["from_row"],
        "cursor": ["continuation"]
      },
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
          "response": { "items": "/payload/events", "next_cursor": "/paging/next" },
          "stop": "missing-or-null-cursor"
        },
        "getReport": false
      },
      "page_size": 100
    },
    "oauth": {
      "mode": "auto",
      "storage": "memory",
      "refresh": "on-demand",
      "schemes": {
        "userOAuth": {
          "client_id_env": "EXAMPLE_CLIENT_ID",
          "client_secret_env": "EXAMPLE_CLIENT_SECRET",
          "refresh_skew_seconds": 30,
          "revocation_endpoint": "https://auth.example.com/oauth/revoke",
          "introspection_endpoint": "https://auth.example.com/oauth/introspect",
          "discovery_url": "https://auth.example.com/.well-known/openid-configuration"
        }
      }
    }
  }
}
```

### Field reference

| Field | Values | Notes |
| --- | --- | --- |
| `version` | `"v1"` | Required. Closed version of the policy; unknown versions are errors. |
| `env_prefix` | `1..=64` bytes, `[A-Za-z_][A-Za-z0-9_]*` | Enables the automatic `<ENV_PREFIX>_API_KEY` convention. See §2. |
| `pagination` | `"auto"`, `"off"`, or the expanded object | Shorthand strings normalize to the expanded form. See §4. |
| `pagination.mode` | `"auto"` (default) or `"off"` | `off` disables inference; explicit per-operation entries still apply. |
| `pagination.patterns` | list of `limit-offset`, `cursor`, `page-number`, `next-link` | Accepted and fingerprinted; **not yet enforced** — detection currently always uses the built-in set. |
| `pagination.aliases` | `limit`/`offset`/`cursor`/`page` → list of request-key synonyms | Extends the built-in vocabulary. One name cannot be claimed by two roles. |
| `pagination.operations` | per-operation mapping or `false` | Mapping object or explicit disable, keyed by `operationId` (or `METHOD /path` for unnamed operations). |
| `pagination.page_size` | `1..=10000` | Proposed SDK fallback page size. Accepted and fingerprinted; **not yet consumed by emitted helpers**. It never asserts a server default. |
| `oauth` | `"auto"`, `"off"`, or the expanded object | Lifecycle policy for used OAuth 2.0/OIDC schemes. See §5. |
| `oauth.storage` | `"memory"` (only value in v1) | The enum is closed; future stores are explicit configuration. |
| `oauth.refresh` | `"on-demand"` (only value in v1) | Refresh-before-expiry gate; background refresh is future work. |
| `oauth.schemes` | source Security Scheme name → scheme config | `client_id_env`, `client_secret_env` (portable variable names), `refresh_skew_seconds` (`0..=3600`, default 30), `revocation_endpoint`, `introspection_endpoint`, `discovery_url` (absolute http(s) URLs; plain http only for loopback hosts). |

Every mapping is validated at configuration time: request keys must be actual
parameter names of the selected operation, response values must be RFC 6901
JSON Pointers that resolve inside the declared success schema with a
role-appropriate type, and required roles per pattern must be present.
Contradictory or ambiguous configuration is a located generation error, not a
silent guess.

## 2. Automatic API-key environment credentials

With `env_prefix` configured, ordinary client construction resolves
`<ENV_PREFIX>_API_KEY` automatically — no credential argument, no special
factory. For `env_prefix: "OPENROUTER"`, a generated Python client reads
`OPENROUTER_API_KEY` at construction; a TypeScript client reads it from the
runtime environment; Go uses `NewClientFromEnv`; Rust uses `Client::from_env`,
and so on in every package's documented idiom (see
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md) for the exact per-language
constructor forms).

Binding rules:

- The automatic variable binds when the selected operations use **exactly one**
  unambiguous bearer/API-key scheme. Zero or multiple string-credential schemes
  produce no default binding; generation continues without it rather than
  guessing.
- OpenAPI's declaration decides how the value is attached (bearer header,
  named header, query, or cookie). The scheme's *name* never decides
  attachment.
- Explicit `credential_env` mappings always win entirely; the prefix applies
  only when no explicit mapping exists. Multiple or unusual schemes should use
  `credential_env`.

Precedence at runtime, in order:

1. An explicit credential argument is authoritative. Empty/null values, an
   empty credentials object, or missing members are **never** supplemented from
   the environment.
2. An explicit credential provider, where the package exposes one.
3. The configured environment snapshot — taken once at client construction.
   Later environment changes affect newly created clients only. Importing or
   generating an SDK never reads credential values.
4. Nothing: missing/empty environment values remain unavailable credentials.

Missing credentials fail **before HTTP** with a bounded, secret-free error that
may name the variable and scheme but never the value. Public (anonymous)
operations stay usable without any credential, and a client containing both
protected and anonymous operations can serve the latter. Browser TypeScript and
portable Dart never require process/`dart:io` access: environment readers are
injectable, and absent access produces the same unavailable-credential state.

## 3. Attribution: the `ua/v1` User-Agent

Every request issued by a generated package — operations, pagination pages,
and streaming reads — carries a programmatically assembled User-Agent:

```text
suspect/<suspect-version> <identity>/<identity-version> (<language>/<language-version>; openapi/<spec-version>)
```

```text
User-Agent: suspect/0.42.1 openrouter-python/1.2.0 (python/3.12.1; openapi/3.1.0)
User-Agent: suspect/0.42.1 acme-chatbot/7 (python/3.12.1; openapi/3.1.0)
```

- `suspect/<version>` is always the first token: the generator version captured
  at generation time.
- The identity slot is the SDK package name and version by default. A
  client-supplied application identifier **replaces** it, so raw SDK usage and
  identified application adoption are separable in API traffic while every
  request still carries the suspect token.
- The trailing comment names the language/runtime version (discovered at client
  construction) and the source document's declared OpenAPI/Swagger version. The
  format is versioned (`ua/v1`); changes are compatibility events.

### Client options

| Package | Override (wins entirely) | Suppress | Application identity (replaces the SDK token) |
| --- | --- | --- | --- |
| TypeScript/JS | `userAgent: string` | `userAgent: null` | `applicationId: string` |
| Python | `user_agent=...` | `user_agent=None` | `application_id="acme-chatbot/7"` |
| Go | `ClientOptions.UserAgent *string` | non-nil empty string | `ClientOptions.ApplicationID string` |
| Rust | `ClientOptions.user_agent` | explicit empty string | `ClientOptions.application_id` |
| Swift | `ClientOptions.userAgent` | empty string | `ClientOptions.applicationId` |
| Java | `HttpRuntime.Builder.userAgent(value)` | empty string | `applicationId(value)` |
| Kotlin | `ClientOptions.userAgent` | explicit empty | `ClientOptions.applicationId` |
| C# | `ClientOptions.UserAgent` | explicit empty string | `ClientOptions.ApplicationId` |
| Ruby | `user_agent:` keyword | `nil` or empty string | `application_id:` keyword |
| PHP | `ClientOptions(userAgent: ...)` | explicit empty string | `ClientOptions(applicationId: ...)` |
| Dart | `Client(userAgent: ...)` | explicit empty string | `Client(applicationId: ...)` |
| C++ | `ClientOptions.user_agent` | empty value | `ClientOptions.application_id` |

- An application identifier is `<name>` or `<name>/<version>` — RFC 9110
  tokens, at most 128 bytes. Invalid values never reach the wire: packages
  either reject them up front or omit the automatic header.
- An explicit caller-supplied `User-Agent` header (for example a declared
  header parameter) keeps precedence over the automatic value.
- Setting the generator version to empty at generation time disables the
  automatic header for that package entirely.

### What the server sees, and privacy

The header contains only: the generator version, the package/application
identity, the language/runtime version, and the source spec version. It never
contains hostnames, paths, user identity, or other environment-derived values.
The runtime version is discovered natively per platform:
`platform.python_version()` (Python), `runtime.Version()` (Go), `RUBY_VERSION`
(Ruby), `PHP_VERSION` (PHP), `java.version` (Java/Kotlin), `Environment.Version`
(C#), the OS version (Swift), the Node runtime version for TypeScript packages
(`node/<version>`, `unknown` in browsers), and `unknown` where a platform has
no discoverable version (C++, Dart today) rather than breaking the format.

**`applicationId` is visible on the wire.** Choose an identifier you are willing
to send to the API server on every request; do not put private information in
it.

## 4. Automatic pagination

When the generation configuration carries `sdk_defaults` (any section),
list-style operations get native pagination helpers with zero per-operation
configuration. Detection runs at generation time against the resolved query
parameters and the declared success response schemas — never at runtime, and
never by guessing.

### Detection rules

A pattern is selected only with a **complete** request/response continuation
rule. Detection order within one operation: `cursor`, then `limit-offset`, then
`page-number`. Names are matched after deterministic normalization (lowercase,
`_`/`-` removed), so `pageSize`, `page_size`, and `pagesize` are one key; the
emitted helpers keep the actual wire names and case.

| Pattern | Request evidence (query parameters) | Response evidence (declared success schema) |
| --- | --- | --- |
| `cursor` | a string-typed `cursor`, `pageToken`, `continuationToken`, `after`, or `startingAfter` (plus optional size parameter) | a `next_cursor`, `nextCursor`, `next_page_token`, `nextToken`, or `continuation` string field (or an integer `next_offset`) |
| `limit-offset` | an integer `limit`, `pageSize`, `per_page`, `take`, or `max_results` **and** an integer `offset`, `skip`, or `start_index` | a collection: a top-level array or a recognized array field (`data`, `items`, `results`, `records`); optional `total`, `has_more`, `next_offset` |
| `page-number` | an integer `page`/`pageNumber` **plus** a size parameter (never one parameter doing both jobs) | the collection; optional `total`, `total_pages`, `has_more` |
| `next-link` | — (manual mapping only) | — |

Additional rules:

- User `aliases` extend the built-in vocabulary per role; a name claimed by two
  roles is a configuration error.
- Type evidence is required: a `limit` parameter must be integer-typed, a
  cursor string-typed. Wrong types do not match.
- Manual `operations` mappings bind request names against the operation's query
  parameters and response pointers against the declared success schema; a
  pointer that resolves to nothing, or to the wrong type for its role, is a
  located error. Cursor tokens stay opaque and keep their original type.
- Operations that stay ordinary single-page calls get a source-linked
  explanation ("no query parameters to paginate with", "no JSON success
  response schema to inspect", "no complete request/response continuation rule
  matched", or an explicit disable).
- v1 detection covers **query parameters only**. Paginated POST/search
  operations, body-carried pagination, and header controls need a manual
  mapping today.
- `next-link` is admitted as a manual mapping pattern; v1 walkers do not follow
  links, so such helpers stop after the first page.
- `pagination.patterns` and `pagination.page_size` are accepted, validated, and
  fingerprinted, but not yet enforced by the detection planner or consumed by
  emitted helpers (see the field table above).

Precedence: an explicit per-operation entry (mapping **or** `false` disable)
wins; otherwise detection runs with the built-in + alias vocabulary; otherwise
the operation remains a documented single-page call.

### Traversal semantics

- One request in flight. The first page is exactly the direct call's result,
  included once; every later request rebuilds **only** the pagination controls
  and preserves filters, sort order, headers, representation, and credentials.
- Laziness is guaranteed: a request starts only when a not-yet-fetched page is
  required. Early `break`, `close()`, drop, cancellation, or timeout issues no
  further request.
- Caller values win: an explicitly set pagination control is honored for the
  first request; an explicit zero-valued offset is a value, not "unset".
- A repeated identical continuation value fails the walk with a typed
  pagination error instead of looping forever.

### Stop rules and the empty-page fallback

| Pattern | Stops when |
| --- | --- |
| `limit-offset` (advance `items-returned`) | the compiled `has_more` pointer reads `false`, or a page yields **zero items** (the documented fallback when the source has no explicit end marker — a short page continues, an empty page ends the walk) |
| `limit-offset` with a compiled `next_offset` pointer | the pointer reads absent, `null`, or empty |
| `cursor` (`stop: missing-or-null-cursor`) | the continuation pointer reads absent, `null`, or the empty string |
| `page-number` | zero items, `has_more` reading `false`, or the last declared page (`total_pages`) |
| `next-link` / pointer-less cursor mappings | after the first page (no followable pointer in v1) |

An empty cursor page with a valid next cursor **continues**; only the
continuation value itself decides. Server-clamped page sizes are handled by the
items-returned advance (the walk uses what the page actually returned, not the
requested size).

### Native helpers per package

`<op>` is the operation name in each language's native casing. All helpers are
generated only for operations the planner actually paginated.

| Package | Page iteration | Item iteration | Explicit next page |
| --- | --- | --- | --- |
| TypeScript/JS | `<op>Pages(client, input): AsyncIterable<Page>` | `<op>Items(client, input): AsyncIterable<Item>` | `await <op>NextPage(client, input): Promise<Input \| null>` (the rebuilt next-page input, not a page) |
| Python | `iter_<op>_pages(**kwargs)` (sync `Client`; `async def` on `AsyncClient`) | `iter_<op>_items(**kwargs)` (sync + async) | — (iterate pages) |
| Go | `<Op>Pages(ctx, input) *<Op>PageIterator` with `Next() bool` / `Page()` / `Err() error` / `Close()` | `<Op>Items(ctx, input) *<Op>ItemIterator` with `Next()` / `Item()` / `Err()` | `<Op>NextPage(ctx, input) (Input, bool)` |
| Rust | `<op>_pages(&client, input) -> <Op>Pages` with `async fn next(&mut self)` | `<op>_items(&client, input) -> <Op>Items` pull pager | `<op>_next_page(&client, input) -> Result<Option<Input>, Error>` |
| Swift | `<Op>PageSequence` (pull-driven async sequence) | `<Op>ItemSequence` | `<op>NextPage(...)` |
| Java | `<Op>Pages implements Iterator<Page>, AutoCloseable` (`hasNext`/`next`/`close`) | `<Op>Items implements Iterator<Item>, AutoCloseable` | `<Op>NextPage(...)` |
| Kotlin | `<op>Pages(input): Flow<Page>` | `<op>Items(input): Flow<Item>` | suspending `<op>NextPage(input)` |
| C# | `<Op>PagesAsync(input, cancellationToken): IAsyncEnumerable<Page>` | `<Op>Items(input, ...): IAsyncEnumerable<Item>` | `await <Op>NextPageAsync(input)` (+ `<Op>Continuation`) |
| Ruby | `<op>_pages(**kwargs): Enumerator` (with RBS signatures) | `<op>_items(**kwargs): Enumerator` | explicit next-input builder on the client |
| PHP | `<op>Pages(input): Generator` | `<op>Items(input): Generator` | `<op>NextPage(input)` |
| Dart | `<op>Pages(input): Stream<Page>` (single-subscription, lazy) | `<op>Items(input): Stream<Item>` | `<op>NextPage(input)` |
| C++ | `<op>_pages(input)` RAII pager with `next`/`value`/`error` | `<op>_items(input)` item pager | `<op>_next_page(input)` |

Every backend has native behavioral tests that compile and execute the emitted
helpers where a toolchain exists on the host.

## 5. OAuth lifecycle

### What is generated today

`http_protocol` compiles one lifecycle descriptor per **used** OAuth 2.0/OIDC
security scheme. The descriptor carries exactly what the source declares plus
what `sdk_defaults.oauth.schemes` supplements; endpoints are never invented:

- The scheme kind (`oauth2` or OpenID Connect) and its declared flows
  (`implicit`, `password`, `client-credentials`, `authorization-code`,
  OAS 3.2 `device-authorization`), each with its declared authorization/token/
  refresh/device URLs and scopes, in declaration order.
- The token-endpoint client authentication derived per flow
  (`client-secret-basic` for confidential clients with a configured secret;
  none for public clients).
- The configured `client_id_env`/`client_secret_env` variable names,
  `refresh_skew_seconds` (default 30), and — configuration-supplied only, since
  OpenAPI flow objects do not declare them — `revocation_endpoint`,
  `introspection_endpoint`, and `discovery_url`. For OpenID Connect the
  declared `openIdConnectUrl` is the known discovery document; for OAuth2 the
  declared `oauth2MetadataUrl` or the configured `discovery_url`.
- Implicit and password flows are **represented but never executed**;
  authorization-code with PKCE is the normal interactive default and
  client-credentials the normal service default.

Generated packages today describe these flows and pass the descriptors —
source, flow, scope, and discovery metadata — to **caller credential hooks**.
The SDK performs no discovery, acquisition, refresh, or retry on its own. An
OAuth credential is attached exactly like the source declares (for example a
bearer `Authorization` value supplied by your callback).

**Opt-in 401 replay wrapper (unified request policy).** On top of the plain
provider — which keeps today's attach-only semantics and never retries — the
four reference backends (TypeScript, Python, Go, Rust) emit an opt-in
*replaying* variant of the client-credentials provider. When a protected
request answers HTTP 401 (and only 401) with a token the replaying provider
itself attached, it performs exactly **one coordinated refresh** (concurrent
401s share one token request through the same single-flight round) and
exactly **one replay** of the request with the fresh token, preserving method,
URL and body while regenerating headers through the normal attach path. The
second response is surfaced whatever it is: a second 401 reaches the caller
as the declared error, and the overall budget is one refresh plus one replay,
never nested with other retry policies. Attaches for **stream-protected
requirements are never replayed**, because delivered stream data prevents a
transparent restart; a failed refresh surfaces as the typed authentication
failure instead of a replay. Replay is wired per call site: the provider is
passed as the scheme's `auth` member and its transport wrapper as the
client's transport, so existing packages and callers are unaffected unless
they opt in.

### Planned: first-party token lifecycle (M5)

The plan's target runtime — not yet emitted by generated packages — is:

- A native **token store interface** plus an instance-owned in-memory store;
  persistence is always an explicit choice. The store holds the access token,
  token type, refresh token if present, known expiry, granted scope, and
  issuer/audience identity, partitioned per authorization context.
- **Client credentials** with single-flight acquisition, cached until
  `expiry − refresh_skew_seconds` against an injectable clock.
- **On-demand refresh** with rotation semantics: a replacement refresh token in
  a refresh response atomically replaces the token set; an omitted one is
  retained where the protocol permits; concurrent refreshes coordinate before
  contacting the token endpoint.
- **Revocation and introspection** helpers only when the corresponding endpoint
  is configured or discovered.
- Typed authentication errors that never carry token values.

Until that runtime lands, applications implement acquisition/refresh in their
own credential hooks; the descriptors above are the contract those hooks
consume. Status and milestone tracking are in
[SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md).

## 6. Migrating from pre-1.0 generated SDKs

Regenerating a package with `sdk_defaults` changes three observable behaviors.
All are configuration-driven: regenerate without an `sdk_defaults` section (or
with the relevant section removed) to keep the previous behavior.

1. **Generated packages now send a User-Agent by default.** Every request
   carries the `ua/v1` attribution header. Servers and tests that assert on
   `User-Agent` will see the new value. Disable it per client with the
   suppression spelling for your language (`userAgent: null` on TypeScript,
   `user_agent=None` on Python, a non-nil empty `UserAgent` string on Go, …),
   or override it with your own value. A declared User-Agent header parameter
   still wins over the automatic value.
2. **Pagination helpers appear only when `sdk_defaults` is configured.** A
   regeneration with an `sdk_defaults` section may add per-operation
   `<op>Pages`/`<op>Items`/`<op>NextPage`-style helpers and a pagination module
   to packages that previously had none. No-policy generation stays
   byte-identical to the previous output — the helpers are additive files and
   methods, and existing operation methods keep their names and signatures.
   Disable individual operations with `operations: { "<opId>": false }` or
   switch detection off entirely with `pagination: "off"`.
3. **`<ENV_PREFIX>_API_KEY` resolves automatically when `env_prefix` is
   configured.** Ordinary construction now snapshots the composed variable at
   client creation for a single unambiguous bearer/API-key scheme. Existing
   explicit constructors and explicit `credential_env` mappings are unchanged
   and take precedence; an explicit empty/null credential is never supplemented
   from the environment. If you already construct clients with explicit
   credentials, nothing changes; if you relied on factory-only env resolution,
   the ordinary constructor now does it too.

## References

- [SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md) — the program plan
  and milestone status.
- [SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md) — environment-credential
  runtime semantics and per-language constructor forms.
- [SDK-OPENAPI-MATRIX.md](SDK-OPENAPI-MATRIX.md) — the normative OpenAPI
  feature matrix with honest per-row status.
- [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md) — the `repeated-v1` native cost
  methodology.
- [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md) — the twelve-backend capability
  matrix and per-profile evidence.
- [SDK-RELEASE-CHECKLIST.md](SDK-RELEASE-CHECKLIST.md) — the per-package
  publication runbook (generation inputs, pre-publish gates, package
  metadata, compatibility-report gate, manual sign-offs).
