# Swift environment credentials — v1

Native implementation and scoped current/floor proof for
[SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md). This is an opt-in generated-client
default. The policy contains environment **variable names**, never their values.

## Generation and capture

The existing `swift_sdk::SwiftConfig` adds:

```rust,ignore
credential_env: Option<crate::credential_env::CredentialEnv>
```

Its default is `None`. After the existing shared and Swift protocol admission,
`plan_sdk` calls `credential_env::plan` and retains its `Option<CredentialEnvPlan>`.
`SdkPlan::credential_env()` returns the bound immutable plan. Source use-site and
terminal identities remain distinct, including a requirement bound through a
referenced Security Scheme. The binder supplies the common syntax, unknown-name,
ambiguity and unsupported-kind refusals before artifacts.

Swift's capture function sets `NativeSnapshot.credential_env` from
`semantic_descriptor()`. It does not put physical Provenance into interface
equality. The fixed native 8,192-byte value policy is documented beside that
capture and fingerprinted with the implementation. Main owns canonical option
forwarding, readiness, cache/Session identity and comparison classification.

For OpenRouter the explicit policy is:

```json
{"credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}}
```

The source scheme named `apiKey` declares HTTP bearer attachment. Neither its name
nor the operation name implies an authentication convention.

## Actual compiled Swift API

Only a configured package emits these construction paths:

```swift
public init(
    transport: any HTTPTransport = URLSessionTransport(),
    options: ClientOptions = .init()
)

public static func fromEnvironment(
    transport: any HTTPTransport = URLSessionTransport(),
    options: ClientOptions = .init()
) -> Client
```

Both are **nonthrowing**. `Client()` uses the omitted-credentials initializer.
The factory also supports an injected Sendable transport and normal options:

```swift
let client = Client.fromEnvironment(
    transport: transport,
    options: ClientOptions(timeout: 15)
)
```

The explicit initializer keeps its original parameter types:

```swift
public init(
    credentials: Credentials,
    transport: any HTTPTransport = URLSessionTransport(),
    options: ClientOptions = .init()
)
```

An explicit `Credentials` value is authoritative as a whole. For example,
`Client(credentials: Credentials(apiKey: nil))`, `Credentials(apiKey: "")`,
`Credentials()`, and a credentials value containing only another scheme do not
load missing members from the environment. The whole credential argument is
nonoptional: **`Client(credentials: nil)` is a compile-time error**. The factory
does not accept a `credentials:` argument.

The fixed helper name `fromEnvironment` is reserved in the client-method allocator
only for configured plans. A conflicting source operation receives its normal
allocated suffix. The environment access is qualified as
`Foundation.ProcessInfo.processInfo.environment`, so a source model named
`ProcessInfo` cannot shadow it. The `Client` remains an immutable Sendable value;
its existing transport/task ownership is retained.

## Snapshot, availability and errors

`ProcessInfo.environment` is read once inside the construction-time factory.
Generated module initialization, generation and operations do not read it. The
mapped strings become the client's credential value. Changing the environment
affects a newly created client, not an existing client or a task using that client.

- Missing and empty values remain unavailable.
- Values over **8,192 UTF-8 bytes** remain unavailable. The reader examines at
  most 8,193 bytes for this check, then applies the existing attachment rules.
- Bearer values use the native printable, non-whitespace ASCII guard.
- Header API keys pass the existing header guard; query/cookie strings retain
  their source-defined URI-component encoding.
- An unusable or missing variable does not poison another satisfiable security
  alternative. OR/AND, explicit selection, disabled security and anonymous
  alternatives still use the existing protocol machinery.

A protected operation with unsatisfied credentials throws `SDKError` with
`kind == .requestRepresentation`, a fixed secret-free JsonError and the original
operation source **before transport**. Its status is absent and its response
headers/body are empty. Client creation succeeds so a mixed protected/anonymous
SDK remains usable for anonymous operations.

Swift uses Foundation's process environment on its existing Apple-platform
baseline. The declared verified tiers are Swift **6.3.3 + SDK 26.5** and Swift
**6.0.3 + SDK 15.4**. There is no additional portable/browser dependency.

## Compiled OpenRouter call site for the docs owner

The native proof generates the clean package/module **`OpenRouter`**, version
`1.0.0`, from the actual source checkout. It selects the original `getCurrentKey`
and `getCredits` operations without editing their schemas or servers.

```swift
import OpenRouter

let client = Client() // Configured OPENROUTER_API_KEY snapshot happens here.
let result = try await client.getCurrentKey()
print(result.status)
print(result.data.data.isManagementKey)
```

The default request is source-declared **GET
`https://openrouter.ai/api/v1/key`**. `try await client.getCredits()` is the explicit
management-key mode and uses the source `/credits` declaration. These helpers do
not set a server override. The proof uses a controlled transport, checks the
actual HTTPS URLs and decoded source models, and makes no account request.

## Maintained native checks

```text
--test swift_credential_env native_credential_env_precedence_snapshot_and_docs
--test swift_credential_env native_openrouter_current_key_environment_defaults
```

Use `cargo test --locked --offline -p suspect-codegen --no-default-features
--features http-protocol`, append a selector above, then
`-- --ignored --exact --nocapture`.

The first gate runs **9 native cases**: positive omitted/factory construction,
missing/empty values and anonymous use, explicit whole-value precedence, OR/AND
and explicit alternatives, header/query/cookie attachment, creation-time snapshot
across task handoff, unusable values, exact 8,192-byte/Unicode boundaries, and
factory/source-name collisions. The second runs **3 actual OpenRouter-source
cases**. Both gates build and test the generated package and an independent
consumer, typecheck successful controls, require negative probes for whole `nil`
and wrong argument forms, and convert DocC with warnings as errors.

Both gates pass on both declared tiers: **12 native consumer cases + 2 generated
example tests + 4 negative type probes per tier**, plus positive type controls
and DocC. The generator-process canary and source-binding/refusal host controls
also pass. The unconfigured fixture matches **all 27 pre-change artifact hashes**
in `tests/fixtures/swift-credential-env-no-policy.json`.

### Receipts and support contract

Receipts: `target/sdk-swift-credential-env-20260911/`:

- `native-handoff.json` is the finalized machine-readable API/selector/support
  contract, with production/support hashes and four completed native roots.
  Current/floor generated artifacts are byte-identical for each package, and
  both retained consumer fixtures match their maintained source bytes.

- `native-current-01.log` contains the passing actual OpenRouter gate and the
  retained first synthetic-test helper-name compile failure.
- `native-current-02.log` is the completed synthetic gate after the test-only
  helper rename; the generated client implementation already compiled.
- `native-floor-01.log` records both floor gates passing.
- `host-01.log` records four host checks, including exact no-policy artifact
  parity and absence of a generator-process canary from every artifact.
- `swift-quality-01.json` records zero Swift-owned warning/error diagnostics.
- `rustdoc-01.log` records warnings-denied Rustdoc success;
  `capture-format-01.json` verifies the Swift capture's scoped formatting.

Retained artifact roots under `${TMPDIR%/}/opencode`:

| Gate | Current | Floor |
| --- | --- | --- |
| Environment controls | `swift-credential-env-current/synthetic-5gofVw` | `swift-credential-env-floor/synthetic-nYkKzj` |
| Actual OpenRouter | `swift-credential-env-current/openrouter-75CUui` | `swift-credential-env-floor/openrouter-YELBte` |

Each contains the generated package, bound/semantic policy records, artifact
hashes, native command logs, independent consumer, typing controls and DocC
archive. OpenRouter roots also retain the original source path/hash. Toolchain
selection uses existing `SUSPECT_SWIFT_BIN`, `SUSPECT_SWIFTC_BIN`,
`SUSPECT_SWIFT_DOCC_BIN`, and `SUSPECT_SWIFT_SDKROOT`. Optional
`SUSPECT_SWIFT_CREDENTIAL_ENV_ROOT` selects the fresh artifact parent.
`OPENROUTER_WEB_ROOT` selects the actual source checkout; the harness supplies
controlled test values for `OPENROUTER_API_KEY`.

## Production and test assets

Production edits:

```text
crates/suspect-codegen/src/swift_sdk.rs
crates/suspect-codegen/src/swift_sdk/protocol_emit.rs
crates/suspect-codegen/src/swift_sdk/emit.rs
crates/suspect-codegen/src/compatibility/native.rs::swift
```

Helpers are emitted into configured `Client.swift` files. There is no new
standalone production runtime asset. New test-only files are
`tests/swift_credential_env.rs`, `tests/fixtures/swift-credential-env-no-policy.json`,
`swift_sdk/credential_env_native.swift`, and `swift_sdk/credential_env_openrouter.swift`.
They reuse `swift_sdk/validation_v2_support.rs` for physical toolchain selection,
retained command logs, typechecking and DocC. Existing v1/v2/v3, HTTP and aggregate
native matrices retain their completed evidence.
