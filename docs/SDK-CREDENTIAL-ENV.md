# Explicit runtime environment credentials — v1

User-directed DX work, 2026-09-11. Main integration owner:
`ses_f72dcea72ffe3CL9Uf6IC2rP9u`. Work stays local and uncommitted.

## Generation configuration

The SDK session JSON has a new optional top-level field:

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
Main owns canonical option forwarding and CLI/session/capture integration. Native
owners call the shared binder after their real protocol admission and consume its
typed result in their existing emitter. No generated-source post-processing or
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

All twelve registered adapters have completed native proof and pass Main's
configured generation/capture/Session bridge
(`target/sdk-credential-env-integration-20260911-01/all-twelve-bridge-01/`).
The real CLI also reports the explicit names-only policy, produces identical
files/revisions under two controlled generator environments, and restores every
ordinary output byte when the policy is disabled (`cli-defaults-process-01/`).

| Adapter | Actual configured API | Platform/ownership detail |
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

The completed native receipts include source-default `/key` and optional `/credits`
checks through controlled transports. These APIs require newly generated packages
with the explicit policy; the frozen live demo CLI `5d3cecd1…` predates them.
The first frozen env review CLI is
`target/sdk-main-env-candidate-20260911-01/bin/suspect`, SHA-256
`f971ead49e99a15e205325b186b2a7effb640e412da31f8c24e100983c59f496`.
It pins 1,157 workspace inputs. New live-package/consumer preparation has its own
evidence and delivery; the existing staged live edition remains separately sealed.

Independent review found two issues in this first candidate. Dart's configured
parameter had been widened to nullable; its corrected, independently rechecked
source retains non-nullable `Credentials`, the private omission sentinel, static
literal-null rejection and dynamic `TypeError` before environment lookup. The old
candidate packages retain the earlier behavior until replaced. Go's completed
capture repair records the actual allocated factory symbol and its native
signature; the original additive-collision witness now reports a located breaking
rename while generated Go bytes remain equal. Its owner proof is at
`target/sdk-go-env-factory-capture-20260911-01/`. Both combined independent rechecks
are now clean in
`target/sdk-credential-env-integration-20260911-01/review-env-01/fixes-01/`;
the original review reports remain pinned.

Configured empty-selection capture also follows actual native admission rather
than skipping planning. The ninth shared host control verifies its located
refusal and retains the old `EmptySelection` meaning without a policy.

The corrected CLI is
`target/sdk-main-env-candidate-20260911-02/bin/suspect`, SHA-256
`31c2fe23c760f191fdb8546cc10d2d78973935d3a728cd8875dfb66f45975261`.
Its immutable 1,161-file source includes the exact Go/Dart repairs and shared
empty-capture correction. `live-generation-01/completion-02/REPORT.json` verifies
the actual all-twelve six-operation generation/capture, two generator-environment
canaries, 593 unchanged ordinary files and the original Go collision on this
binary. Eleven configured language packages equal the first env cohort; Dart's
constructor and two credential guides are corrected. New prebuilt env consumers
use this package boundary, with their controlled execution and final live seal
recorded separately. `ENV-ACCEPTANCE.json` records both clean independent reviews,
20 passing focused host checks, warnings-denied Clippy/Rustdoc, successful
formatting and stable source bytes for this corrected integration.

The resulting **[automatic-env live edition](../LIVE-ENV-DEMO-README.md)** is now
independently accepted: `./demo-live-env.sh all` runs the twelve prebuilt native
consumers, with optional per-language/JavaScript selection. Main verified the
3,179-file delivery seal, 611 corrected package files and all 52 controlled
preparation outcomes, including the eight fresh Go/Dart cases and 13 missing-env
zero-HTTP cases. The accepted receipt is
`target/sdk-credential-env-integration-20260911-01/live-env-delivery-owner-receipt-01.json`.
Real-token live execution remains a runtime user action; the README distinguishes
the controlled proof and the preserved F10 failed-canary disclosure.

## Existing native owners

Each owner keeps its adapter, capture function, native tests/docs and production
asset inventory. Native configuration field and bound-plan getter are the common
interface above; actual generated helper names are handed off only after native
compilation. The following are requested native forms, not unverified demo code.

| Target | Existing owner | Requested client form |
| --- | --- | --- |
| TS / JS | `ses_f73d087bfffeFGg0Qfhsv2OQKL` | `createClient()`; omitted `auth` only |
| Python | `ses_f73f82130ffen8pLYsl1meVCR4` | `Client()` / `AsyncClient()`; private omission sentinel |
| Go | `ses_f73cc3d90ffea31f659Zum1rON` | `NewClientFromEnv(...)`; existing explicit constructor |
| Rust | `ses_f73cc3d90ffdYk64FlMfNGe4rQ` | `Client::from_env()`; native custom-transport companion as needed |
| Swift | `ses_f73cc3d89ffejhQElxcPpk9cDC` | omitted-credential initializer or native env factory |
| Java | `ses_f742beae0ffeOv7erDs00JyGLy` | native omitted-credential overload / `Client.fromEnv()` |
| C# | `ses_f742beae0ffdvBDl4bsjJLnHbx` | native omitted-credential overload / `Client.FromEnvironment()` |
| Kotlin | `ses_f742beac2ffehMN0DeR948kJkr` | native omitted-credential overload / `Client.fromEnv()` |
| Ruby | `ses_f742beac1ffeaEHFKdf2YZjfoz` | `Client.new` / `Client.open`; omitted `auth:` only |
| PHP | `ses_f742beac1ffd4fPwu2PXw6R9wK` | native omitted-credential constructor / `Client::fromEnv()` |
| Dart | `ses_f742beac1ffcAvJ8LodYZuDAA2` | omitted auth with native supplied transport; conditional env access |
| C++ | `ses_f742beaafffee7oviEec2NpiYf` | `Client::from_env()`; existing explicit constructors |

Main alone changes shared configuration, registry, provenance and comparison code.
All native work is delegated to the existing owners; no duplicate owner is created.

## Verification at the agreed interfaces

User-requested checks exercise public configuration/plan/generation/Session/
capture interfaces and actual native SDK clients. The shared binder has located
positive/reference/ambiguity/unsupported-kind controls. Canonical checks cover
no-policy byte retention, policy edit/revert cache identity, source-linked capture,
and absence of a generator-process canary value from artifacts.

Each affected native runtime needs a bounded positive environment read; absent and
empty values; explicit value/empty/null/missing-member precedence; OR/AND/anonymous
controls; creation-time snapshot; source-default server capture; and appropriate
browser/portable checks. Native commands use controlled test processes/transports,
not real account credentials. Existing full matrices remain their own evidence.

## Live-demo priority and paths

The sole docs/demo owner remains `ses_f72144e30ffeca1RzXZzDgjbw3`. New DX paths are
`DEMO-DX-README.md`, `examples/sdk-demo-branded.json`,
`examples/sdk-demo-branded/**`, and `target/sdk-demo-dx-20260911-01/**`.
New live paths are `LIVE-DEMO-README.md`, `demo-live.sh`,
`tools/sdk-demo-live/**`, `examples/sdk-demo-live/**`, and
`target/sdk-demo-live-20260911-01/**`.

The live default is the source-declared **`getCurrentKey` GET `/key`**, suitable for
a normal user token. `getCredits` is an explicit management-key mode. All programs
use the source HTTPS server, print actual status/decoded confirmation, and perform
read-only requests. Runtime environment or secure prompt supplies the token;
staging uses explicit controlled test mode and cannot claim live success.

The accepted automatic-env entry is `LIVE-ENV-DEMO-README.md` /
`demo-live-env.sh`, backed by `environment-01/candidate-02/` and its explicit
references to byte-identical provisional preparations. The original `5d3cecd1…`
live edition retains its documented wrapper-supplied credentials. Existing
README/package/delivery-01/delivery-02, F10 and Terraform reviewed artifacts remain
sealed at their original paths.
