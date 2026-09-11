# Rust runtime environment credentials — v1

Rust implements the explicit policy in
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md). Both focused native gates passed on
Rust **1.97.1** and **1.88.0**, using installed Cargo packages and controlled
transports. Actual OpenRouter `getCurrentKey` and `getCredits` source schemas are
covered; these checks make no real account requests.

## Configuration and binding

Canonical session configuration stores variable names only:

```json
{
  "credential_env": {
    "version": "v1",
    "schemes": { "apiKey": "OPENROUTER_API_KEY" }
  }
}
```

For direct native planning, set
`rust_http::HttpConfig::credential_env: Option<credential_env::CredentialEnv>`.
The default is `None`. After protocol admission, Rust calls the shared
`credential_env::plan` and retains its result on
`HttpPlan::credential_env() -> Option<&CredentialEnvPlan>`.

OpenRouter's scheme named `apiKey` is **HTTP bearer**. Supply a plain token in
`OPENROUTER_API_KEY`; the scheme name does not imply an API-key header. The shared
binder chooses the actual used declaration, deduplicates requirements, and
refuses ambiguous/unselected names and Basic/OAuth/OIDC mappings. Runtime values
are never read by the generator.

## Compiled native helpers

These helpers exist only for a successfully bound generation policy:

| Helper | Arguments | Result | Feature |
| --- | --- | --- | --- |
| `Client::from_env()` | None | `Result<Client<reqwest_transport::ReqwestTransport>, reqwest::Error>` | `reqwest-rustls` |
| `Client::with_transport_from_env(transport)` | Owned `T`; operation calls require `T: http::Transport` | `Client<T>` | `http` |
| `Credentials::from_env()` | None | `Credentials` | `http` |

Each factory snapshots mapped variables through `std::env::var` when called.
A variable shared by several bindings is read once per snapshot. Bindings use
their physical scheme **use-site** identity with the existing
`with_source_bearer` / `with_source_api_key` machinery.

Missing, empty and unavailable/non-Unicode values stay absent. Other nonempty
strings pass through existing attachment validation. Protected calls report
source-linked `SdkErrorKind::RequestValidation` before HTTP for missing or invalid
credentials; error display/debug contains neither the value nor the raw auth
header. Normal credential/HTTP byte ceilings remain in force.

`Client::from_env()` returns a construction error only if reqwest initialization
fails. Missing credentials are deferred to protected operations so anonymous
operations remain usable. Existing OR, AND, explicit alternative choice and
anonymous behavior is preserved. `Credentials::from_env().select_alternative(i)`
lets a caller select an existing source alternative explicitly.

Existing constructors remain authoritative for their whole argument:

```rust,ignore
Client::with_reqwest(Credentials::api_key(explicit_token))?;
Client::with_transport(transport, Credentials::new());
```

They never fill an empty value, an empty credentials object or missing members
from the environment. Rust's explicit credential argument is non-nullable;
passing `None` fails compilation. New helper names are reserved only for
configured packages; colliding operation/scheme names follow the native allocator.

## OpenRouter live-program form

Generate a clean package named **`openrouter`** with the policy above. The compiled
default read-only call is `get_current_key_default()`; management mode can call
`get_credits_default()` explicitly.

```rust,no_run
use openrouter::Client;

async fn current_key() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::from_env()?;
    let response = client.get_current_key_default().await?.into_response();
    println!("HTTP {}: {}", response.status, response.data.data.label);
    Ok(())
}
```

Run this inside the caller's Tokio runtime with the package's `reqwest-rustls`
feature. Source-default URLs are `https://openrouter.ai/api/v1/key` and
`https://openrouter.ai/api/v1/credits`; no server override is required. Generation,
imports and per-request execution do not read mapped variables. No `.env` loading,
acquisition, refresh or automatic retry is added. This is a standard-library/native
Rust facility; browser, WASM and no-std environment access are not claimed targets.

## Output, capture and byte retention

Configured packages add the private support file `rust/src/credential_env.rs`, a
feature-gated module declaration and policy documentation. The source manifest's
optional `credentialEnv` member retains the bound plan and physical provenance.
Rust capture sets `NativeSnapshot.credential_env` from the retained plan's typed
**`semantic_descriptor()`**, excluding physical provenance from interface equality.

The pre-change **36-file** unconfigured artifact checkpoint lives in
`tests/fixtures/rust-credential-env-no-policy.json`. Every hash matches, including
`http-manifest.json`, constructors, docs and runtimes. Main's unchanged ordinary
`plan_http_v3` / v2 full-artifact and generation/session/capture bridge also passes.

## Focused verification

Target: `--test rust_credential_env`.

Native selectors, verified on both toolchains:

- `installed_rust_credential_env_snapshots_and_explicit_credentials_obey_source_auth`
- `installed_rust_openrouter_env_current_key_uses_source_default_https`

The first runs seven isolated modes: positive, missing, empty, one-available
alternative, creation-time snapshot, invalid value and non-Unicode unavailability.
It covers explicit value/empty/partial credentials, OR/AND, explicit choice,
anonymous calls, bearer/header/query/cookie attachment and helper collisions.
It passes **20 SDK Rustdoc tests** and **two null-argument compile-fail tests**.

The OpenRouter gate uses unmodified tracked source and two controlled process
modes. It passes **34 SDK Rustdoc tests**, two null-argument controls, and native
decode of source-declared response examples. The reqwest factory is constructed
and type-checked; all requests use a controlled transport at the source HTTPS URLs.

Ordinary host selectors:

- `rust_no_policy_artifacts_match_pre_env_checkpoint`
- `rust_credential_env_is_source_bound_and_reserves_only_configured_helpers`
- `rust_credential_env_generation_does_not_capture_generator_values`

```sh
cargo test --locked --offline -p suspect-codegen --test rust_credential_env \
  -- --include-ignored --nocapture --test-threads=1

SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0 cargo test --locked --offline \
  -p suspect-codegen --test rust_credential_env \
  -- --ignored --nocapture --test-threads=1
```

`SUSPECT_RUST_CREDENTIAL_ENV_TARGET` selects a native cache;
`OPENROUTER_WEB_ROOT` locates the tracked corpus. Attempts and command logs are
retained. Environment mutation occurs only in a standalone, single-threaded test
consumer before any executor/reqwest construction. Generated SDK code is Rust 2024
and contains no unsafe code or environment mutation.

Native proof used reduced `http-protocol` features during parallel adapter edits.
Final full-default-feature Clippy with warnings denied passed. Old native matrices
were not replayed.

## Receipt and owner handoff

Receipt: `target/sdk-rust-credential-env-20260911-01/native-receipt-01.json`

SHA-256: `bb527fb74b370c06337b77326fd45b3abe6cb0b95f60ca0e5a08e27ac5dd86c7`.

The receipt contains four native attempts, compiled helper/consumer hashes, copied
command logs, corpus hash, no-policy checkpoint and owned production hashes.
Main owns opening canonical Rust readiness and checking policy capture/Session.
The docs owner can use the compiled helper contract with the fresh pinned candidate.

Owned production assets, relative to `crates/suspect-codegen/src/`:

```text
rust_http.rs
rust_http/plan.rs
rust_http/emit.rs
rust_http/emit/credential_env.rs       new emission module
rust_http/emit/docs.rs
compatibility/native.rs              rust() only
```

The new emission module is registered in `rust_http::source_assets()` for Main's
central provenance integration. Maintained support inputs are
`tests/rust_credential_env.rs`, `tests/fixtures/rust-credential-env-consumer.rs`,
and the no-policy checkpoint.
