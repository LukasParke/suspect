# Explicit runtime environment credentials — v1

Generated SDKs read credentials from the environment only when generation
opts in; the rationale is [ADR 0026](adr/0026-environment-credentials-are-opt-in.md).
There are two opt-ins: an explicit `credential_env` mapping (below) or the
`sdk_defaults.env_prefix` convention described in
[SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md), which binds `<ENV_PREFIX>_API_KEY`
when exactly one bearer/API-key scheme is used. An explicit `credential_env`
mapping takes precedence over `env_prefix`.

## Generation configuration

The SDK session JSON has an optional top-level field:

```json
{
  "credential_env": {
    "version": "v1",
    "schemes": { "apiKey": "OPENROUTER_API_KEY" }
  }
}
```

`schemes` keys are source Security Requirement / Security Scheme names. Values
are **environment variable names**, never credential values. OpenAPI supplies the
attachment semantics: the current OpenRouter scheme named `apiKey` is HTTP bearer.
Package identities remain explicit target configuration; titles, URLs, operation
names and extensions supply no inferred environment-variable convention.

V1 admits HTTP bearer tokens and API-key strings (header/query/cookie). Basic,
OAuth/OIDC credential structures and other attachment shapes are refused by this
initial policy. This is configuration-only; no extension metadata or `.env` files
are interpreted. A mapping requires a nonempty map, at most 64 entries, nonempty
bounded source names, and portable variable names matching
`[A-Za-z_][A-Za-z0-9_]{0,127}`. Duplicate JSON map keys and unknown versions/fields
are errors.

## Shared module and interface

`suspect_codegen::credential_env` owns syntax, source binding and typed metadata:

```rust
CredentialEnv { version: CredentialEnvVersion::V1, schemes: BTreeMap<String, String> }
CredentialEnv::v1(schemes)
credential_env::plan(&Contract, &ProtocolPlan, Option<&CredentialEnv>)
    -> Result<Option<CredentialEnvPlan>, Vec<HttpDiagnostic>>
```

The binder consumes the native adapter's **already admitted** canonical protocol
plan. A configured name must identify exactly one used Security Scheme
declaration (`requirement.scheme().use_site().source()`) across the selected
operations. Repeated requirements for that declaration deduplicate. Identically
named declarations in different documents are ambiguous, including aliases to a
common terminal; unknown/unselected names and unsupported credential kinds fail
before artifacts. No source graph or security semantics are reconstructed.

`CredentialEnvPlan::bindings()` returns sorted immutable bindings with
`name()`, `variable()`, `kind()` and `scheme()` getters. The scheme retains typed
use-site/terminal/reference provenance. `semantic_descriptor()` returns typed
version/name/variable/kind metadata that excludes physical provenance from
interface equality. The module performs no environment access or I/O.

Each native planning configuration adds
`credential_env: Option<credential_env::CredentialEnv>` (default `None`), retains
the bound plan, and exposes `credential_env() -> Option<&CredentialEnvPlan>`.
Canonical generation options forward the policy to every adapter; each adapter
calls the shared binder after its real protocol admission and consumes its
typed result in its existing emitter. No generated-source post-processing or
duplicate binder is introduced.

## Runtime semantics

1. Configured omitted-credential construction, or a clearly named native env
   factory where the existing constructor requires credentials, snapshots mapped
   variables at **client creation**. Import/generation never reads their values.
   A later environment change affects a newly created client, not an existing one.
2. An explicit credential argument is authoritative for the entire argument.
   Empty/null values, an empty credentials object, or missing members are never
   supplemented from environment variables. Existing native type/null errors
   remain errors; a whole-null argument is not converted into an empty object.
   Existing explicit constructors remain.
3. Missing/empty/unavailable environment values remain unavailable credentials.
   Existing security selection handles OR/AND, explicit alternative selection,
   disabled security and anonymous alternatives. An anonymous operation can run
   without environment access being available; one missing unused alternative
   cannot prevent a satisfiable alternative from running.
4. Missing credentials fail **before HTTP**, with a bounded, secret-free error.
   Failure may be deferred to the protected operation so a client containing both
   protected and anonymous operations remains usable for the latter. Errors may
   name the configured variable/scheme, but never its value or raw auth header.
5. Only strings usable by the existing declared attachment are supplied to the
   existing native credentials machinery. No acquisition, role/scope invention,
   refresh, retry, server rewrite or per-call environment lookup is added.
6. Browser TypeScript/JavaScript and portable Dart retain explicit-credential
   operation without unconditional Node/process or `dart:io` imports. Missing
   runtime environment access produces the same unavailable-credential state;
   protected calls fail clearly before transport and anonymous calls remain valid.

Adapters allocate/reserve new helper names through their maintained native naming
rules. Unconfigured generation keeps its existing constructor behavior and
ordinary output bytes. New env helpers and constructor branches are emitted only
for a configured, successfully bound policy. Source-default fixed HTTPS servers
already belong to the protocol plan and must be used when the caller omits an
override. Multiple/relative/variable servers keep their existing semantics.

## Canonical configuration and compatibility

The optional policy belongs to `GenerationOptions`, so complete/target cache keys,
revision identity, input/config reports, and compatibility metadata include it.
Serialization omits an absent policy to retain existing configuration bytes.
Native capture records the bound semantic descriptor; physical binding metadata
belongs in source/provenance output. A policy change is a native client-default
change, not a change to OpenAPI wire declarations or an interpretation profile.
Environment values are absent from every generated artifact and fingerprint.

### Qualified native forms

All twelve registered adapters emit a configured environment client form. The
real CLI reports the explicit names-only policy, produces identical
files/revisions under different generator environments, and restores every
ordinary output byte when the policy is disabled.

| Adapter | Actual configured API | Toolchains and notes |
| --- | --- | --- |
| TS / JS | `createClient(options: ClientOptions = {})` | TS5.5.4/5.9.3, Node22.23.1/24.21.0 and Chrome153; own `auth` property truly omitted; explicit undefined/null retain their native behavior |
| Python | `Client()` / `AsyncClient()` | Python 3.11.15/3.14.7; omitted `auth` only; existing httpx transport keywords remain |
| Go | `NewClientFromEnv(options ...ClientOptions) (*Client, error)` | Go1.23.12/1.27.1; zero or one options value; ordinary explicit constructor remains authoritative |
| Rust | `Client::from_env()`, `Client::with_transport_from_env(T)`, `Credentials::from_env()` | Rust1.88.0/1.97.1; reqwest factory returns its native initialization Result; supplied-transport and credential factories use the `http` feature |
| Swift | `Client()` / `Client.fromEnvironment(transport:options:)` | Swift6.0.3/SDK15.4 and Swift6.3.3/SDK26.5; nonthrowing Sendable construction; explicit whole nil is a compile-time error |
| Java | `Client.fromEnv()`, `fromEnv(HttpClient)`, `fromEnv(HttpClient, Function<String,String>)` | JDK21/25; injected transport is caller-owned; null accessor stays unavailable |
| Kotlin | `Client(transport:options:)` / `Client.fromEnv(transport:options:environment:)` | JDK21/25; optional `CredentialEnvironment` reader; existing transport lifetime rules remain |
| PHP | `Client::fromEnv(Transport, ClientOptions)` with both arguments defaulted | PHP8.3.32/8.5.8; disabled `getenv` yields missing credentials; existing explicit constructor is authoritative |
| Dart | `Client(transport: IoTransport())` | Dart3.9.4/3.13.3; non-nullable `Credentials` with a typed omission sentinel; optional injected `environment` reader; VM uses cached/read-only platform environment, browser uses unavailable stub |
| Ruby | `Client.new` / `Client.open` with `auth:` truly omitted | Ruby3.3.12/4.0.6; explicit nil retains existing `ArgumentError` without reading ENV |
| C# | `Client.FromEnvironment(ClientOptions? options = null, HttpClient? httpClient = null)` | SDK8.0.424/10.0.400; injected HttpClient is caller-owned; parameterless ordinary constructor remains anonymous |
| C++ | `Client::from_env(ClientOptions = {}, CurlOptions = {})`; `Client::from_env_with_transport(shared_ptr<const Transport>, ClientOptions = {})` | C++20/libcurl factory returns `Result<Client, TransportError>`; supplied-transport factory returns `Client` and works in core-only builds |

These APIs exist only in packages generated with an environment policy.
Configured empty-selection capture follows actual native admission rather than
skipping planning: it reports a located refusal, and retains the ordinary
`EmptySelection` meaning without a policy.

## Verification

Checks exercise public configuration/plan/generation/Session/
capture interfaces and actual native SDK clients. The shared binder has located
positive/reference/ambiguity/unsupported-kind controls. Canonical checks cover
no-policy byte retention, policy edit/revert cache identity, source-linked capture,
and absence of a generator-process canary value from artifacts.

Each affected native runtime needs a bounded positive environment read; absent and
empty values; explicit value/empty/null/missing-member precedence; OR/AND/anonymous
controls; creation-time snapshot; source-default server capture; and appropriate
browser/portable checks. Native commands use controlled test processes/transports,
not real account credentials.

The shared binder tests are `cargo test -p suspect-codegen --test shared`.
Each backend has `--test <lang>_credential_env` (C# uses the
`csharp_sdk::credential_env_tests` library module); generation-only cases run by
default, and the native-toolchain cases are `#[ignore]` and run with `--ignored`.
