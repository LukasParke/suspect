# Canonical Rust HTTP packages

`rust_http::plan_http(Arc<Contract>, &[SourceId], HttpConfig)` plans selected
outgoing operations. `rust_http::emit_http(&plan, &PackageConfig)` produces a
private Cargo package under `rust/`, with native models/codecs, async operations,
Rustdoc, a package guide and a source/symbol HTTP manifest.

## Generate and consume

```sh
cargo run --locked -p suspect-cli -- gen sdk \
  ../openrouter-web/projects/docs/openapi/openapi.yaml \
  --profile rust-http \
  --operation-id getCredits --operation-id createKeys --operation-id updateKeys \
  --operation-id listContainerFiles --operation-id getContainerFile \
  --package-name openrouter-sdk --package-version 0.0.0 \
  --output target/openrouter-rust-package
```

Add `--check --format json` for read-only ownership/drift inspection. Missing,
modified or obsolete owned files are findings; user-edited files remain conflicts.
Selectors are exact operation IDs. Omitting selectors attempts every outgoing
operation and reports unsupported selected contracts before output. Package name
and SemVer are explicit packaging configuration, independent of API title/version.
The compiler invokes no native package manager and performs no publication.

The editor's canonical Rust choice uses the same argv/process helper, prompts for
Cargo identity and exact selectors, and opens the generated package guide.

```toml
[dependencies]
openrouter-sdk = { path = "./target/openrouter-rust-package/rust", features = ["reqwest-rustls"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
```

```rust,no_run
use openrouter_sdk::{Client, Credentials};
use openrouter_sdk::operations::get_credits::{GetCredits, GetCreditsSuccess};

async fn credits(token: String) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::with_reqwest(Credentials::api_key(token))?;
    let GetCreditsSuccess::Status200(response) =
        client.get_credits(GetCredits::new()).await?;
    println!("{}", response.data.data.total_credits.as_str());
    Ok(())
}
```

`token` is application-supplied. The source scheme named `apiKey` is an HTTP
bearer scheme; the generated constructor follows that declaration. It produces
`Authorization: Bearer …`, not a guessed API-key header. Credentials are never
embedded into generated artifacts. Environment credentials are opt-in: only
when the generation config sets a `credential_env` policy or
`sdk_defaults.env_prefix` does the package emit `Credentials::from_env()`,
`Client::from_env()` and `Client::with_transport_from_env()`, which snapshot the
mapped variables once at client creation
([ADR 0026](../../adr/0026-environment-credentials-are-opt-in.md),
[SDK-CREDENTIAL-ENV.md](../../reference/credential-env.md)). Without a policy no variable is
read.

## Native interface

- Operation modules and client methods use allocated snake-case operation IDs.
  Input types use Pascal case, e.g. `GetCredits`, `CreateKeys`, `UpdateKeys`.
  The manifest retains every original wire name and source identity.
- Required parameters followed by a required body become constructor arguments.
  `with_<member>` supplies optional inputs; absence does not apply schema defaults.
  When every input is optional, a no-input `<operation>_default` helper omits
  them all.
  Models use the existing `Nullable`, `Presence`, exact `JsonInteger` and
  `JsonNumber` representations. Encoding validates mutable models again.
- A free function and a borrowed-client method execute the same operation.
  Results contain status-specific success variants and native model bodies.
  A sole-success enum implements `Deref<Target = ApiResponse<...>>`,
  `into_response()` and `into_data()`; multiple-success operations keep explicit
  variants.
  `OperationError::Api(Box<OperationApiError>)` contains declared error variants;
  `OperationError::Sdk(Box<SdkError>)` contains runtime failures. Boxing keeps large
  response models out of the size of every returned `Result`.
- `ApiResponse<T>` preserves actual status, selected media, raw header bytes and
  decoded data. Invalid declared errors remain decoding/evaluation failures.
  Unexpected and invalid responses retain bounded raw capture and truncation.
- Native docs and compiled examples bind the same allocated models, codecs,
  operations, status/media alternatives and source addresses as implementation.
  Package renaming re-renders codec examples from the typed plan; source prose is
  never rewritten by replacing strings in already-emitted Rust.

## Features and transport

| Feature | Interface and dependencies |
| --- | --- |
| Defaults | Models, exact JSON, validation and codecs; no external dependencies |
| `http` | Generic `Transport` / `ResponseBody` async seam; pinned `url = 2.5.8` |
| `reqwest-rustls` | Recommended reqwest 0.12.28 adapter, default features disabled, rustls TLS; caller-owned Tokio runtime |
| `serde-json` | Optional `serialize`/`deserialize` functions on each codec for `#[serde(with = ...)]`, running the validated codec ([ADR 0018](../../adr/0018-no-serde-derives-in-generated-rust.md)) |

`Client::with_transport` accepts a native transport. `ClientOptions` holds an
explicit server override and lower response/capture limits. The source HTTPS
server and its path prefix are the defaults; an explicit local HTTP server is
available to consumers and tests. Request URL/body and actual streamed response
bytes are bounded. Content-Length is not used as a substitute for counting bytes.

`ReqwestTransport::from_builder` permits pool/TLS customization while enforcing
disabled proxy routing/discovery, redirects, retries, decompression and referer generation. It accepts a
builder so these policies can be applied before construction. A caller-supplied
custom transport is responsible for its own HTTP implementation and cancellation.

The SDK owns each exchange/body in the operation future. Dropping that future
cancels by ownership; callers can use `tokio::time::timeout` or their executor's
future selection. The SDK starts no executor or detached request task. Connection
pool internals remain the selected adapter's responsibility.

## Protocol profile and boundaries

HTTP admission consumes the shared `http_protocol::ProtocolPlan`
([SDK-HTTP-PROTOCOL.md](../../design/http-protocol.md)); the native profile is
`rust-http-protocol-v1`, and its explicitly enumerated capabilities are
published by `rust_http::native_capabilities()`. New shared capabilities are
never enabled automatically.

`HttpConfig` holds the codec policy plus `max_request_bytes`,
`max_response_bytes`, `max_part_bytes`, `max_stream_item_bytes`,
`max_chunk_bytes`, `max_header_bytes` and `compatibility_profiles`. Use
`..Default::default()` when specifying individual overrides. Defaults are 8 MiB
request/response/part/chunk, 1 MiB stream item and 64 KiB headers. Generation
limits must be positive and portable to 32-bit `usize`; caller limits can only
lower them.

Response selection is **exact status → class range → default**, followed by
media selection: concrete media outrank type ranges and `*/*`, with declared
parameter count breaking specificity ties. A media mismatch never falls through
to another status rule. Actual 200–299 status determines success, including for
default responses.

`CompatibilityProfile::LegacyBinaryStringV1` is an explicit opt-in for legacy
3.1/3.2 `type: string, format: binary` byte markers. Ordinary 3.1+ byte schemas
are unconstrained; standard 3.0 binary markers are supported directly.

Rust still requires neutral directional equivalence: readOnly/writeOnly must be
absent or false in its selected closure (`http-directional-codec-unsupported`).
Rust strings contain Unicode scalar values; lone UTF-16 surrogate representation
remains a documented cross-language gap. URL-normalizing path values such as dot
segments fail explicitly before sending. The separate byte, depth, evaluation
and conversion policies are not complete CPU or allocator accounting.

## Verification

The native seams are maintained in `tests/rust_http.rs`,
`tests/rust_http_runtime.rs`, `tests/rust_http_openrouter.rs`, CLI
`tests/rust_sdk.rs`, and the editor's real helper-to-CLI test. Native checks cover
generation, drift, model-only/custom/reqwest builds, Rustdoc and private packing
for credits, keys and container-file groups. The integrated gate is documented
in [SDK-M3-M6-EXIT.md](../../internal/SDK-M3-M6-EXIT.md).

Results are recorded in [SDK-PROGRESS.md](../../internal/SDK-PROGRESS.md) and the
[session handoff](../../internal/SDK-SESSION-HANDOFF.md). Generated-package consumers,
Rustdoc/doctests and installed native HTTP checks pass on Rust 1.88.0 and the
current compiler, including the recommended reqwest/rustls profile.
