# suspect-platform

OpenAPI/Arazzo/Overlay toolkit: language server, contract testing engine,
template generator, and a local API gateway — one binary, one lossless
parser core, everything observable through structured journals.

## Crates

| Crate | Purpose |
|---|---|
| `suspect-syntax/source/low` | Lossless YAML/JSON parse layer (tree-sitter backed) |
| `suspect-oas/arazzo/ref/jsonpath/schema/validate/lint` | Semantic layer: models, `$ref` resolution, validation, spectral-style lint |
| `suspect-ir` | Normalized indexed spec snapshot shared by gen/test/gateway/LSP |
| `suspect-journal` | Structured JSONL logs, traffic records, Suspect Cassette v1 format |
| `suspect-rex` | Arazzo runtime-expression parser/evaluator |
| `suspect-test` | Plan compiler + concurrent workflow executor + reporters |
| `suspect-gateway` | Local server: mock / proxy / validate / record / replay |
| `suspect-gen` | Template engine (minijinja) + docs/TS-SDK/Rust-SDK presets |
| `suspect-watch` | Debounced file watcher driving `--watch` modes |
| `suspect-lsp` | Language server: diagnostics, navigation, run lenses, notebook-grade integrations |

## Quick start

```bash
cargo build --release -p suspect-cli
export PATH="$PWD/target/release:$PATH"

# Lint + validate a spec
suspect check api.yaml && suspect lint api.yaml

# Contract testing (offline from a recorded cassette)
suspect test flows.arazzo.yaml --offline --cassette prod.scj

# Live mock of the whole API
suspect gateway api.yaml -p 8080 --mode mock

# Record traffic through the proxy, then replay deterministically
suspect gateway api.yaml -p 9000 --mode record --upstream http://staging:80 --cassette rec.scj
suspect gateway api.yaml -p 9001 --mode replay --cassette rec.scj

# Generate docs / SDKs
suspect gen api.yaml --preset docs-md --out site
suspect gen api.yaml --preset ts-sdk  --out sdk/ts
suspect gen api.yaml --preset rust-sdk --out sdk/rust

# Property-based fault hunt against a live deployment
suspect fuzz api.yaml --base-url http://localhost:8080 --runs 50

# Re-run on every save
suspect watch . -- suspect test flows.arazzo.yaml --offline --cassette rec.scj
```

## Feature roadmap

The next ten platform features — traffic-to-contract synthesis, stateful
property-based testing, server scaffolding with progressive mock→real
handoff, webhooks/events first-class support, an OWASP API security ruleset,
an auth-flow execution engine, environment parity monitoring, release
engineering, and transactional spec codemods — are documented in
[docs/FEATURE-ROADMAP.md](docs/FEATURE-ROADMAP.md).

## Editors

- **VS Code** (`editors/vscode`): language features via the bundled LSP,
  Testing-API explorer for Arazzo workflows/steps, an Arazzo *notebook*
  webview (cells = steps, inline runs), gateway status-bar control.
- **Zed** (`editors/zed`): extension registering `suspect lsp` for
  YAML/JSON; codelens-driven runs and diagnostics today.

## Performance budgets (enforced by in-repo criterion benches)

| Path | Budget |
|---|---|
| Parse + IR cold | < 10 ms |
| Mock gateway added latency p99 | ≤ 1 ms vs raw hyper |
| Replay throughput | ≥ 50k req/s |
| Gen render p95/file | < 1 ms |
| Watch save→rerun | < 150 ms |

Run them: `cargo bench`.

## Cassette format

Suspect Cassette `.scj`: JSONL — header line (`format`, `version`,
`recorded_at_ms`, `source`) then one entry per exchange with sha256-hashed
bodies (UTF-8 or base64). Append-only, git-diffable, consumed by replay,
offline tests, and drift checks.
