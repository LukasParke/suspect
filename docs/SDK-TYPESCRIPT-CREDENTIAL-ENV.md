# TypeScript runtime environment credentials — v1

This implements the explicit, source-bound policy in
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md). Generation records variable names;
the generated client snapshots values at construction.

## Configuration and actual client API

```rust,ignore
use suspect_codegen::{credential_env::CredentialEnv, typescript::http};

let config = http::HttpConfig {
    credential_env: Some(CredentialEnv::v1([
        ("apiKey".into(), "OPENROUTER_API_KEY".into()),
    ].into())),
    ..http::HttpConfig::expanded()
};
let plan = http::plan_http(contract, &selected, config)?;
let bound = plan.credential_env();
```

The canonical session option is
`credential_env: {version: "v1", schemes: {apiKey: "OPENROUTER_API_KEY"}}`.
Main owns canonical forwarding, readiness and Session/cache checks. The native
plan binds through the shared `credential_env::plan` after protocol admission.
Unknown/unselected/ambiguous declarations and unsupported mapped credential
kinds are source-linked planning errors. V1 maps bearer/API-key strings only.

For a configured package, the actual generated factory is:

```ts
createClient(options: ClientOptions = {})
```

`ClientOptions.auth` is optional in configured generation. An explicitly supplied
auth object retains its source-named native credential types. For the actual
OpenRouter source, `apiKey` is **HTTP bearer**, not an inferred API-key header.

```ts
import { createClient, operations } from '@example/credential-client';

const client = createClient(); // snapshots OPENROUTER_API_KEY here
const current = await client.getCurrentKey();
console.log(current.status, current.data.data.is_management_key);

// A caller-supplied whole-argument override:
const explicit = createClient({ auth: { apiKey: token } });

// getCredits is an explicit management-key mode, not the default live request.
```

The native source witness uses GET `https://openrouter.ai/api/v1/key`, its actual
schema and declared response example. `current.data.data.limit_remaining` is an
exact `JsonNumber | null`; `rate_limit.requests` is `bigint`. Package names are
explicit configuration; the test package is private `@example/credential-client`.

## Precedence, timing and platforms

| Input/state | Behavior |
| --- | --- |
| `createClient()` / `createClient({})` | Snapshot mapped variables once because the own auth property is omitted. |
| Explicit auth object/value | Authoritative for the whole argument; no environment filling or lookup. |
| JS `auth: undefined`, `{}`, missing/undefined members | Retain missing credentials; protected operations reject before HTTP. |
| JS empty credential string or invalid credential value | Existing source attachment validation rejects before HTTP. |
| JS `auth: null` / non-object auth | Existing explicit constructor raises a generic `TypeError`; no fallback. |
| Missing/empty/unavailable variable | No credential is supplied for that binding. Other satisfiable OR alternatives remain usable. |
| Environment changes after construction | Existing clients keep their snapshot; new clients read the new values. |
| Browser without process environment access | Omitted-env protected calls fail before HTTP; explicit credentials and anonymous operations work. |

Missing credentials use the existing source-bound `SdkError` with
`kind === 'request-validation'`; `operations.isSdkError(error)` identifies it.
Errors are bounded and contain no credential value. OR/AND, explicit alternative
selection, anonymous alternatives and disabled security use the existing resolver.
Source-default servers and explicit overrides retain their existing behavior.

Generation and import perform no environment access. Standalone operation
functions continue using explicit client options and never read the environment.
Explicit credential providers retain their existing callback semantics. No `.env`
file, Node import, ambient process declaration, acquisition or retry is added.

The conditional helper is `http/credential-env.ts` (`dist/http/credential-env.js`
after build). It imports the existing portable data-object helper and accesses
optional `globalThis.process.env` only inside client creation. Each mapped variable
is read at most once per snapshot. Its private import alias is allocated around
actual source symbols; a source operation named `__suspectCredentialEnv` is covered
by native compilation and execution.

## Metadata and byte preservation

`HttpPlan::credential_env()` retains the immutable `CredentialEnvPlan`, including
source declaration provenance. `http-manifest.json.credentialEnv` contains the
bound source metadata and variable names. Native capture writes the typed
`semantic_descriptor()` into `NativeSnapshot.credential_env`; physical provenance
is excluded from semantic equality.

Unconfigured generation keeps the old constructor and emits no environment
helper/import/branch. All **37** output files of the retained no-policy fixture
match the pre-change SHA-256 map in
`tests/fixtures/typescript-credential-env-no-policy-v1.json`. The original files
and source input remain at
`target/sdk-typescript-credential-env-no-policy-before-01/`.
Controlled generator-process canaries are absent from generated artifacts.

## Native evidence and exact selectors

All selectors below are under `--test typescript_credential_env`:

| Selector | Passing evidence under `target/` |
| --- | --- |
| `no_policy_http_package_keeps_its_baseline_bytes` | `sdk-typescript-credential-env-host-01.log` |
| `environment_plan_retains_source_binding_and_semantic_metadata` | same log |
| `environment_binding_refuses_invalid_unbound_and_unsupported_policies` | same log |
| `installed_environment_defaults_preserve_creation_snapshot_explicit_auth_and_security_choices` | `sdk-typescript-credential-env-native-02.log` |
| `installed_openrouter_current_key_uses_source_bearer_env_and_default_https` | `sdk-typescript-credential-env-openrouter-01.log` |
| `browser_environment_absence_preserves_explicit_and_anonymous_source_clients` | `sdk-typescript-credential-env-browser-01.log` |

The last three tests are ignored unless explicitly selected. Installed tests
build separate TypeScript **5.5.4 / 5.9.3** tarballs and execute each on Node
**22.23.1 / 24.21.0**, with strict positive/negative consumers, generated examples
and TypeDoc. The browser witness uses Chromium **153**. All account-facing URLs
are captured by controlled transports; these results are not live account claims.

Native selectors use `SUSPECT_DOCS_NODE`, `SUSPECT_NODE24_BIN`, optional
`OPENROUTER_WEB_ROOT`, and `SUSPECT_CHROMIUM`. `SUSPECT_CREDENTIAL_ENV_ARTIFACTS`
retains uniquely named package, lock, consumer and result directories. Both
compiler tiers use isolated offline npm installs. Earlier seals, full matrices
and the completed ignored-multipart closeout retain their original evidence.

## Source asset handoff

New production asset, relative to `suspect-codegen/src`:

- `typescript/http/credential-env.ts`
- SHA-256: `a000274989adbcaac30529c9b88cccf6b130260e90a9717bd784154b59786a3b`

Modified existing assets: `typescript/http.rs`,
`typescript/http/{protocol_emit.rs,interface.rs}`, and `typescript/package.rs`.
Only `compatibility/native.rs::typescript` is changed for capture. The language's
`source_assets()` inventory includes the new helper for Main's canonical framing.
The shared backend/options, registry and other captures remain Main/owner work.
