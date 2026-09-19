# suspect-platform

OpenAPI/Arazzo/Overlay toolkit: language server, contract testing engine,
template generator, and a local API gateway — one binary, one lossless
parser core, everything observable through structured journals.

SDK generation has one contract pipeline with Python, Go, Swift, Rust and
TypeScript/JavaScript profiles. Native models, checked codecs, HTTP clients,
packages and docs share typed language plans. See
[SDK capabilities](docs/SDK-CAPABILITIES.md) and
[current verification](docs/SDK-PROGRESS.md) for the experimental profiles and
pending post-cleanup acceptance.

## Crates

| Crate | Purpose |
|---|---|
| `suspect-syntax/source/low` | Lossless YAML/JSON parse layer (tree-sitter backed) |
| `suspect-oas/arazzo/ref/jsonpath/schema/validate/lint` | Semantic layer: models, `$ref` resolution, validation, spectral-style lint |
| `suspect-ir` | Source-addressed SDK Contract and indexed platform spec snapshots |
| `suspect-journal` | Structured JSONL logs, traffic records, Suspect Cassette v1 format |
| `suspect-rex` | Arazzo runtime-expression parser/evaluator |
| `suspect-test` | Plan compiler + concurrent workflow executor + reporters |
| `suspect-gateway` | Local server: mock / proxy / validate / record / replay |
| `suspect-codegen` | Five native SDK backends, codecs/docs, incremental sessions and compatibility reports |
| `suspect-gen` | Template engine (minijinja), docs-md preset and custom manifests |
| `suspect-artifact` | Ownership-aware output and read-only drift checking |
| `suspect-watch` | Debounced file watcher driving `--watch` modes |
| `suspect-lsp` | Language server: diagnostics, navigation, run lenses, notebook-grade integrations |

## Quick start

```bash
cargo build --release -p suspect-cli
export PATH="$PWD/target/release:$PATH"

# Parse, semantically validate, and lint a spec
suspect check api.yaml
suspect validate api.yaml
suspect lint api.yaml

# Contract testing (offline from a recorded cassette)
suspect test flows.arazzo.yaml --offline --cassette prod.scj

# Live mock of the whole API
suspect gateway api.yaml -p 8080 --mode mock

# Record traffic through the proxy, then replay deterministically
suspect gateway api.yaml -p 9000 --mode record --upstream http://staging:80 --cassette rec.scj
suspect gateway api.yaml -p 9001 --mode replay --cassette rec.scj

# Generate Markdown, custom templates or native HTTP packages
suspect gen api.yaml --preset docs-md --out site
suspect gen api.yaml --manifest templates/gen.toml --out custom-output
suspect codegen api.yaml --profile typescript-http \
  --package-name @example/sdk --package-version 0.0.0 --out sdk
suspect codegen api.yaml --profile rust-http \
  --package-name example-sdk --package-version 0.0.0 --out generated-rust

# Property-based fault hunt against a live deployment
suspect fuzz api.yaml --base-url http://localhost:8080 --runs 50

# Re-run on every save
suspect watch . -- suspect test flows.arazzo.yaml --offline --cassette rec.scj
```

The SDK profiles are `python-http`, `go-http`, `swift-http`, `rust-http` and
`typescript-http`. Repeat `--operation-id NAME` to select exact operations;
omitting selectors attempts the whole outgoing API and reports unsupported
contracts. Add `--check --format json` for read-only ownership/drift inspection.
[`codegen-session`](docs/SDK-INCREMENTAL-GENERATION.md) shares one Contract across
multiple targets and supports watch/preview. [`codegen-compare`](docs/SDK-COMPATIBILITY-REPORTS.md)
reports native-interface and wire changes between snapshots.

## Feature roadmap

The next ten platform features — traffic-to-contract synthesis, stateful
property-based testing, server scaffolding with progressive mock→real
handoff, webhooks/events first-class support, an OWASP API security ruleset,
an auth-flow execution engine, environment parity monitoring, release
engineering, and transactional spec codemods — are documented in
[docs/FEATURE-ROADMAP.md](docs/FEATURE-ROADMAP.md).

## Historical performance measurements

See [docs/PERFORMANCE.md](docs/PERFORMANCE.md) for the full profiling report,
phase-by-phase cost attribution, and the ranked plan of attack.
These measurements are fixture-specific; they do not establish OpenRouter
end-to-end SDK generation or emitted-SDK performance.
Current session measurements and pending Mac calibration are documented in
[SDK session performance](docs/SDK-SESSION-PERFORMANCE.md).

| Path | Budget | Achieved |
|---|---|---|
| Parse + IR cold | < 10 ms | **7.5 ms** |
| Lint p95/file | ≤ 50 ms | **42 ms** |
| Validate p95/file | ≤ 50 ms | **44 ms** |
| Gateway startup | < 1 s | **~880 ms** |
| Executor per step | ≤ 100 µs | **3.6 µs** |
| Gateway MOCK throughput | ≥ 30k req/s | **95–127k** |
| REPLAY throughput | ≥ 50k req/s | **88–146k** |
| Prepared docs rendering | ≥ 400 MiB/s synthetic test floor | **1.6–1.9 GB/s** historical rendering measurement |
| Watch save→rerun | < 150ms | ~10 ms event latency |

Run them: `cargo bench`.

## Editors

- **VS Code** (`editors/vscode`): language features via the bundled LSP,
  Testing-API explorer for Arazzo workflows/steps, an Arazzo *notebook*
  webview (cells = steps, inline runs), gateway status-bar control.
- **Zed** (`editors/zed`): extension registering `suspect lsp` for
  YAML/JSON; codelens-driven runs and diagnostics today.

## Cassette format

Suspect Cassette `.scj`: JSONL — header line (`format`, `version`,
`recorded_at_ms`, `source`) then one entry per exchange with sha256-hashed
bodies (UTF-8 or base64). Append-only, git-diffable, consumed by replay,
offline tests, and drift checks.
