# Canonical Rust HTTP packages

`rust_http::plan_http(Arc<Contract>, &[SourceId], HttpConfig)` plans selected
outgoing operations. `rust_http::emit_http(&plan, &PackageConfig)` produces a
private Cargo package under `rust/`, with native models/codecs, async operations,
Rustdoc, a package guide and a source/symbol HTTP manifest. Native acceptance
passes for this bounded profile; emission does not certify a complete SDK.

## Generate and consume

```sh
cargo run --locked -p suspect-cli -- codegen \
  ../openrouter-web/projects/docs/openapi/openapi.yaml \
  --profile rust-http \
  --operation-id getCredits --operation-id createKeys --operation-id updateKeys \
  --operation-id listContainerFiles --operation-id getContainerFile \
  --package-name openrouter-sdk --package-version 0.0.0 \
  --out target/openrouter-rust-package
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
`Authorization: Bearer …`, not a guessed API-key header. No credentials are
discovered from environment variables or embedded into generated artifacts.

## Native interface

- Operation modules and client methods use allocated snake-case operation IDs.
  Input types use Pascal case, e.g. `GetCredits`, `CreateKeys`, `UpdateKeys`.
  The manifest retains every original wire name and source identity.
- Required parameters followed by a required body become constructor arguments.
  `with_<member>` supplies optional inputs; absence does not apply schema defaults.
  Models use the existing `Nullable`, `Presence`, exact `JsonInteger` and
  `JsonNumber` representations. Encoding validates mutable models again.
- A free function and a borrowed-client method execute the same operation.
  Results contain status-specific success variants and native model bodies.
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

## Admitted profile and boundaries

HTTP declaration admission is shared by the five SDK profiles through `http_contract`.
The bounded profile supports a static HTTPS source server, one HTTP bearer
requirement, GET/POST/PUT/PATCH/DELETE, required simple string paths, non-null form
scalar/scalar-array queries with either explode value, JSON request bodies and
exact-status JSON responses. Encoders use RFC3986 UTF-8 escaping and exact numeric
tokens. Required empty query arrays fail representation before transport;
optional empty arrays omit. Null query encoding and pagination are not inferred.

Rust still requires neutral directional equivalence: readOnly/writeOnly must be
absent or false in its selected closure. Other media, ranges/default responses,
typed headers/links, alternative security, server variables, streams and
unimplemented schema representations remain source-linked planner errors.

Rust strings contain Unicode scalar values; lone UTF-16 surrogate representation
remains a documented cross-language gap. URL-normalizing path values such as dot
segments fail explicitly before sending. The separate byte, depth, evaluation
and conversion policies are not complete CPU or allocator accounting. Model-only
planning still records its codec obligations, and no complete language SDK or
milestone is promoted by this slice.

## Verification

The native seams are maintained in `tests/rust_http.rs`,
`tests/rust_http_runtime.rs`, `tests/rust_http_openrouter.rs`, CLI
`tests/rust_sdk.rs`, and the editor's real helper-to-CLI test. Native checks cover
generation, drift, model-only/custom/reqwest builds, Rustdoc and private packing
for credits, keys and container-file groups. The integrated gate is documented
in [SDK-M3-M6-EXIT.md](SDK-M3-M6-EXIT.md).

Results are recorded in [SDK-PROGRESS.md](SDK-PROGRESS.md) and the
[session handoff](SDK-SESSION-HANDOFF.md). Generated-package consumers,
Rustdoc/doctests and installed native HTTP checks pass on Rust 1.88.0 and the
current compiler, including the recommended reqwest/rustls profile.
