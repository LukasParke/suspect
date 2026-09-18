# Hackathon demo: OpenAPI to native SDKs

User-approved focus, 2026-09-10: **the generation workflow and the output SDKs**.
The demo includes the five native backends, source fidelity, executable examples,
documentation and iteration tools. Latency measurements are observational.

## Generate five SDKs from the real specification

From the repository root, with `openrouter-web` checked out beside `suspect`:

```sh
cargo build --locked -p suspect-cli --bin suspect
./target/debug/suspect codegen-session \
  --config examples/sdk-demo.json --out target/sdk-demo --format json
```

The configuration selects the five verified credits/key/container-file operations.
Its source path is relative to the configuration file; adjust `spec` for another
checkout location. Package identities are configuration, independent of API semantics.

| Output | Show in the demo |
| --- | --- |
| `typescript/` | Native types/unions, exact codecs, fetch operations, ESM/declarations and TypeDoc metadata |
| `python/` | Keyword-only models, UNSET/null, sync/async clients, wheel metadata and Sphinx docs |
| `go/` | Context-aware methods, typed input/result/error shapes, presence wrappers and standard-library HTTP |
| `rust/` | Enums/value models, builders, exact codecs, optional transport features and Rustdoc |
| `swift/` | Value models/enums, explicit presence, async clients, SwiftPM and DocC |

Open a model, its codec, and an operation that reuses it. The source manifests
connect the emitted symbols to the original OpenAPI document and JSON Pointer.

## Show reproducible output and preview

```sh
./target/debug/suspect codegen-session \
  --config examples/sdk-demo.json --out target/sdk-demo --check --format json

./target/debug/suspect codegen-session \
  --config examples/sdk-demo.json --out target/sdk-demo --preview --format json
```

The check reports `current` with no changed artifacts. Preview returns artifact
contents without writes. The editor also offers read-only diffs and a persistent
watch session. Use a private specification copy when demonstrating live edits.

## Run the generated examples

The examples validate source/synthesized values through the emitted codecs.
Their default execution does not contact the API.

- **TypeScript/JS:** in `typescript/`, run `npm ci`, `npm run build`, then
  `node dist/examples/validated.js`.
- **Python:** install `python/` into an isolated environment, then run
  `python examples/validated.py` from that package directory.
- **Go:** in `go/`, run `go run ./examples/validated`.
- **Rust:** in `rust/`, run `cargo run --features http --example validated`.
- **Swift:** in `swift/`, run `swift test --disable-swift-testing`.

Installed consumers and independent loopback fixtures already verify actual
requests, response handling, types, cancellation and documentation. The packages
also include source-backed call sites for those interfaces.

## Acceptance

```sh
cargo run --locked -p xtask -- sdk-m3-m6 --list-stages --demo
```

The `hackathon-demo` profile retains **208 generation/native/iteration gates**.
A fresh full run uses `--demo --source /path/to/openrouter-web --out target/NEW`.
The frozen `target/sdk-greenfield-native-verified-03/report.json` already records
all 208 as passing. Its original full-profile outcome remains unchanged.

The five-suite latency baseline completed but was too noisy for the proposed
strict p95 policy. Those measurements and failed qualification are preserved;
strict numerical acceptance is deferred from the demo. Full-plan performance
work remains available through the separately scoped calibrated acceptance path.
