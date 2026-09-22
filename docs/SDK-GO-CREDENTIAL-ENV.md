# Go runtime environment credentials — v1

Go HTTP generation supports the explicit policy in
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md). The source Security Scheme
determines attachment; configuration supplies **variable names**:

```json
{
  "credential_env": {
    "version": "v1",
    "schemes": { "apiKey": "OPENROUTER_API_KEY" }
  }
}
```

OpenRouter's source scheme `apiKey` is HTTP bearer. Its configured factory reads
`OPENROUTER_API_KEY` and supplies that string through the existing bearer
credential machinery.

## Public Go API

```go
func NewClientFromEnv(options ...ClientOptions) (*Client, error)
```

Call with zero options to use the source server and ordinary transport defaults,
or one `ClientOptions` to supply a `Doer`, timeout, server selection or override:

```go
import sdk "github.com/openrouter/sdk-go"

client, err := sdk.NewClientFromEnv()
if err != nil { return err }
defer client.CloseIdleConnections()

result, err := client.GetCurrentKey(ctx)
if err != nil { return err }
defer result.Close()
current := result.(sdk.GetCurrentKeyStatus200)
fmt.Println(current.Status, current.Data.Data.Label)
```

`ctx` is caller-owned. The source-default request is
`GET https://openrouter.ai/api/v1/key`. `GetCredits(ctx)` is the separate
management-key call; its success variant is `GetCreditsStatus200` and its exact
credit value is `response.Data.Data.TotalCredits.String()`.

The factory is allocated through the existing Go package-name allocator. If a
selected model or credential constructor already owns `NewClientFromEnv`, the
helper receives the next available name, such as `NewClientFromEnv2`. Consumers
of compiler metadata use `HttpPlan::credential_env_factory()`; generated README,
examples, `go doc`, Sphinx and manifests use that same name.

### Snapshot and precedence

- `os.LookupEnv` runs only in the configured factory, once per mapped binding at
  client creation. Generation and package initialization never read credentials.
- Each client retains its creation-time values. Changes to the process environment
  affect a subsequent factory call.
- Absent, empty, invalid-attachment or over-limit strings remain missing
  credentials. Values are checked with the existing bearer/API-key predicates and
  bounded by `HttpConfig.max_request_bytes` before being retained.
- The existing `NewClient(credentials Credentials, options ClientOptions)` is
  wholly explicit. Empty credentials, an empty string or missing members never
  load or merge environment values. Go rejects `nil` for this value-typed
  credential argument and for its string-valued token setters at compilation.
- Security resolution retains source-order OR, complete AND, explicit
  `SecurityAlternative`, disabled security and anonymous alternatives. A mixed
  anonymous/protected SDK can construct a client without credentials and execute
  its anonymous operations.
- A protected operation with no complete credential alternative returns
  `*SDKError` with `Kind == "request-validation"` before invoking `Doer.Do`.
  Normal formatting is `SDK request-validation failure`; source locations remain
  available on the error, and formatting includes no credential value or header.
- More than one options value returns `*SDKError` with
  `Kind == "request-representation"`. Ordinary options validation is delegated to
  `NewClient`.

This is standard-library Go environment access. The declared native tiers are
Go **1.23.12** and **1.27.1**. The factory delegates context, transport and response
ownership to the established client. It adds no `.env` discovery, acquisition,
refresh, retry or per-request environment lookup.

## Compiler and artifact contract

`go_http::HttpConfig.credential_env` is
`Option<crate::credential_env::CredentialEnv>`, defaulting to `None`.
`go_http::plan_http` invokes the shared binder after protocol admission and retains
the admitted `CredentialEnvPlan`. Public getters are:

```rust
HttpPlan::credential_env() -> Option<&CredentialEnvPlan>
HttpPlan::credential_env_factory() -> Option<&str>
```

V1 supports bearer and header/query/cookie API-key strings. Unsupported kinds and
invalid, unknown, unselected or ambiguous source names are refused by the shared
binder before emission. Go capture assigns the retained plan's
`semantic_descriptor()` to `NativeSnapshot.credential_env`; physical provenance
remains in source-binding artifacts.

Configured emission adds:

- `go/http_environment.go`: allocated native factory; the only added environment
  import/read site.
- `go/credential-env.json`: factory name, versioned semantic descriptor and typed
  source bindings, containing variable names only.

Configured metadata/documentation also appears in `go/http-manifest.json`,
`go/README.md`, `go/docs/index.rst`, `go/docs/api.rst` and
`go/docs/source-bindings.json`. `go/examples/validated/main.go` uses the factory
when a fixture `-server-url` is supplied without `-token`. An explicitly supplied
`-token`, including `-token=`, instead selects the explicit constructor. The
example's default invocation still performs local codec checks only.

No-policy generation emits the established artifacts and names byte-for-byte.
The pre-change witness covers 34 synthetic Go SDK files and all 48 Terraform
fixture artifacts, including its 34-file Go SDK dependency. Inventory hashes bind
each path, byte length and SHA-256:

| Inventory | SHA-256 |
| --- | --- |
| Synthetic SDK | `0724e1c4c26a8861ae87b3d5a9028ee1502feca87f7f317c061be9b3c3f62336` |
| Terraform provider and SDK | `eb880b0850550fa8b69d2852e988414699376e750b7b55486e3688078a50dcf6` |
| Terraform's Go SDK only | `2a4ea978860555651e64491ef0aee742fd551e2837ae519f0a7cceb9ae374d63` |

## Maintained verification

`crates/suspect-codegen/tests/go_credential_env.rs` contains these selectors:

1. `no_policy_retains_pre_change_sdk_and_terraform_bytes`
2. `policy_binds_actual_sources_and_allocates_factory_without_changing_explicit_api`
3. `unsupported_env_attachment_is_refused_by_shared_binder_before_artifacts`
4. `generator_environment_values_do_not_enter_configured_or_unconfigured_artifacts`
5. `native_env_factory_snapshots_and_keeps_whole_explicit_credentials_authoritative`
6. `native_allocated_factory_handles_symbol_collision_and_bounded_env_input`
7. `native_actual_openrouter_key_and_credits_use_env_snapshot_and_source_https`

The native selectors compile installed consumers, negative type controls,
generated packages and example programs on both Go tiers. The OpenRouter selector
loads the original indexed YAML, binds actual `getCurrentKey` and `getCredits`
schemas, and captures their source-default HTTPS URLs through a controlled
`Doer`. Exact response numbers remain lossless. No account requests are made by
these tests.

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
cargo test --locked --offline -p suspect-codegen \
  --no-default-features --features http-protocol \
  --test go_credential_env -- --include-ignored --nocapture
```

`SUSPECT_GO_TOOLCHAIN` selects one native tier when set; otherwise both run.
`SUSPECT_SPHINX_PYTHON` enables warnings-denied Sphinx builds using Go 1.23.12.
`SUSPECT_GO_CREDENTIAL_ENV_EVIDENCE` optionally retains fresh per-case packages,
consumer binaries, hashes and command receipts. An existing case directory is
refused so previous evidence is preserved. Tests always regenerate from original
sources; retained `target/` outputs are not test dependencies.
