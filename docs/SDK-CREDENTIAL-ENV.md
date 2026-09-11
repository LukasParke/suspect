# Explicit runtime environment credentials — v1

Credential environment policy is explicit generation configuration. It maps
source security scheme names to **environment variable names**, never values:

```json
{
  "credential_env": {
    "version": "v1",
    "schemes": { "apiKey": "EXAMPLE_API_TOKEN" }
  }
}
```

Add this optional top-level field to a session configuration. OpenAPI supplies
the credential attachment semantics: a scheme named `apiKey` may be an HTTP
bearer scheme, rather than an API-key header. Package names, titles, URLs and
operation IDs do not imply an environment convention.

## Binding and admission

V1 supports HTTP bearer tokens and API-key strings in headers, query parameters
or cookies. Other credential shapes are refused. No `.env` files or extension
metadata are interpreted. The map must contain 1–64 entries with bounded source
names and portable variable names matching `[A-Za-z_][A-Za-z0-9_]{0,127}`.
Duplicate JSON keys and unknown fields/versions are errors.

`suspect_codegen::credential_env` owns syntax, source binding and typed metadata:

```text
CredentialEnv::v1(schemes)
credential_env::plan(&Contract, &ProtocolPlan, Option<&CredentialEnv>)
    -> Result<Option<CredentialEnvPlan>, Vec<HttpDiagnostic>>
```

The binder consumes the adapter's already admitted protocol plan. A configured
name must identify exactly one used security scheme declaration across the
selected operations. Repeated requirements deduplicate; identically named
declarations in different documents are ambiguous. Unknown/unselected names and
unsupported kinds fail before artifacts.

`CredentialEnvPlan::bindings()` exposes sorted immutable bindings with
`name()`, `variable()`, `kind()` and `scheme()` getters. Typed scheme provenance
retains the use site, terminal and references. `semantic_descriptor()` excludes
physical provenance from interface equality. Binding performs no I/O or
environment access.

## Runtime semantics

- Configured omitted-credential construction, or a named native factory,
  snapshots mapped variables **at client creation**. Generation and import do
  not read credential values. Later changes affect newly created clients.
- An explicit credential argument is authoritative for the entire argument.
  Empty/null values, empty credentials objects and missing members are never
  supplemented from the environment. Existing native type/null errors remain.
- Missing, empty or unavailable values remain unavailable credentials. Existing
  OR/AND security selection and anonymous alternatives continue to apply.
- Missing required credentials fail before HTTP with bounded errors that can
  identify a scheme/variable but cannot include its secret value or auth header.
  Failure can be deferred to a protected operation so anonymous calls still work.
- The policy supplies existing credential machinery only. It does not add
  acquisition, refresh, role inference, retries or server rewrites.
- Browser TypeScript/JavaScript and portable Dart work without unconditional
  process or `dart:io` imports. Explicit credentials and anonymous calls remain
  usable when environment access is unavailable.

Unconfigured generation retains its ordinary output and constructor behavior.
Configured helper symbols use the same native name allocator as other APIs.
The source protocol plan retains authority over server selection.

## Native forms

The generated package's own documentation records its allocated symbols and
available transports. Representative configured forms are:

| Adapter | Configured API |
| --- | --- |
| TypeScript / JavaScript | `createClient()` with `auth` omitted |
| Python | `Client()` / `AsyncClient()` with `auth` omitted |
| Go | `NewClientFromEnv(options ...ClientOptions)` |
| Rust | `Client::from_env()`, `Client::with_transport_from_env(T)`, `Credentials::from_env()` |
| Swift | `Client()` / `Client.fromEnvironment(transport:options:)` |
| Java | `Client.fromEnv()` and explicit transport/accessor overloads |
| Kotlin | `Client(transport:options:)` / `Client.fromEnv(transport:options:environment:)` |
| PHP | `Client::fromEnv(Transport, ClientOptions)` with defaulted arguments |
| Dart | `Client(transport: IoTransport())` with credentials omitted |
| Ruby | `Client.new` / `Client.open` with `auth:` omitted |
| C# | `Client.FromEnvironment(ClientOptions?, HttpClient?)` |
| C++ | `Client::from_env(...)` / `Client::from_env_with_transport(...)` |

## Sessions, comparison and tests

The optional policy belongs to `GenerationOptions`. Source/configuration
revisions, cache identities, configuration reports and native compatibility
metadata include its names-only descriptor. An absent policy is omitted during
serialization. A policy edit changes native client defaults, not OpenAPI wire
declarations. Credential values never enter generated artifacts or fingerprints.

The `credential_env`, `sdk_generation_options` and CLI `credential_env_codegen`
suites cover located binding failures, policy edit/revert identity, source-linked
capture and generator-environment independence. Native adapter suites exercise
creation snapshots, explicit precedence, absent/empty/unavailable values,
security alternatives and controlled transport behavior.
