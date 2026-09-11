# suspect

High-performance Rust toolkit for OpenAPI (3.0, 3.1, 3.2; 2.0 read via family
sniffing), Arazzo 1.0, and Overlay 1.0: lossless tree-sitter parsing, native
`$ref` resolution with cycle classification, JSON Schema 2020-12 validation,
Spectral-compatible linting, an LSP server, and a CLI.

Design constraints: parse once, view many; zero-copy scalars; arena-indexed
nodes; lazy memoized resolution. Benchmarks with honest budget verdicts live
in [BENCHMARKS.md](BENCHMARKS.md).

SDK generation uses one source-addressed contract pipeline for **Python, Go,
Swift, Rust, TypeScript/JavaScript, Java, C#, Kotlin, Ruby, PHP, Dart and C++**.
Each experimental profile produces native models, checked codecs, HTTP
operations, packages and documentation. See
[SDK capabilities](docs/SDK-CAPABILITIES.md) and the
[compiler architecture](docs/SDK-GENERATION-PLAN.md).

## LSP — full-featured editor experience

`suspect lsp` is a complete language server for OpenAPI/Arazzo/Overlay files.
The repo ships a ready-to-use VS Code extension in
[`.vscode-suspect/`](.vscode-suspect/) (see its README for setup), and any LSP
client works — Neovim, Helix, Zed, Emacs (lsp-mode/eglot).

**Capabilities**: publishDiagnostics (syntax + 22 semantic checks + 21 lint
rules + Arazzo validation, debounced), goto-definition and references across
`$ref` boundaries (multi-file), hover with resolved target previews,
context-aware completions (`$ref` candidates from the workspace), quick fixes
and `source.fixAll.suspect`, document formatting, semantic highlighting,
`$ref`-target inlay hints, property type hints, document highlights,
selection ranges, rename with workspace-wide `$ref` rewriting
(`prepareRename` guarded), workspace symbols, folding ranges.

## How we compare

A detailed feature-by-feature comparison against **vacuum** (pb33f/daveshanley)
and **Telescope** (sailpoint-oss) is in
[docs/LSP_COMPARISON.md](docs/LSP_COMPARISON.md).

### In action (VS Code + the bundled extension)

Screenshots are reproducible via `docs/capture/capture.sh` (requires VS Code, Xvfb, xdotool, tesseract).

Diagnostics from the validator and linter, with `$ref` inlay hints:

![Diagnostics](docs/images/lsp-diagnostics.png)

Hover over a `$ref` resolves it across files and previews the target:

![Hover](docs/images/lsp-hover.png)

Quick fixes derived from diagnostics (insert missing `operationId`,
`responses`, `description`, contact/license skeletons, …):

![Code actions](docs/images/lsp-code_actions.png)

Problems panel:


Semantic highlighting and property type hints:

![Semantic tokens + inlay hints](docs/images/lsp-semantic_tokens.png)

Workspace-wide rename rewrites the declaration and every `$ref` that points
at it (guarded by `prepareRename`; cross-file edits land in one atomic
`WorkspaceEdit`) — shown here renaming `Pet`:

![Workspace symbols](docs/images/lsp-workspace_symbols.png)

After applying, every `$ref` is rewritten:

![Go to definition](docs/images/lsp-goto_def.png)


## Crates

| Crate | Purpose |
|---|---|
| `suspect-source` | Loading: mmap, encodings (UTF-8/16, BOM), line indexes, URIs |
| `suspect-syntax` | Lossless tree-sitter CSTs (vendored grammars, patched for >32k-line YAML) |
| `suspect-low` | Ordered model, source maps, YAML 1.2 typing, aliases/merge keys, pointers |
| `suspect-ref` | `$ref` engine: workspace graph, cycle census (legal vs illegal), memos |
| `suspect-jsonpath` | RFC 9535 JSONPath over low nodes |
| `suspect-schema` | Source-bound JSON Schema validation and checked owned programs for SDK codecs |
| `suspect-oas` | Typed OpenAPI 3.x views; `$ref`-transparent, cycle-safe |
| `suspect-ir` | Source-addressed SDK `Contract` and indexed platform snapshots |
| `suspect-codegen` | Native SDK models, codecs, HTTP packages, docs, sessions and compatibility |
| `suspect-gen` | Markdown documentation preset and custom manifest/template rendering |
| `suspect-artifact` | Shared ownership-aware output, drift checks and changed-file writes |
| `suspect-overlay` | Overlay 1.0 models + apply engine |
| `suspect-arazzo` | Arazzo 1.0 models, runtime expressions, cross-ref validation |
| `suspect-validate` | Source-located semantic checks with stable diagnostic codes |
| `suspect-lint` | Spectral-compatible rulesets and builtin rules |
| `suspect-lsp` | tower-lsp server: diagnostics, goto-def/references, hover, symbols, completion |
| `suspect-cli` | `suspect` binary: authoring, SDK generation, tests, gateway and editor commands |

## CLI

```console
$ suspect check spec.yaml              # parse + resolve report, cycle census
$ suspect validate spec.yaml           # source-located semantic findings
$ suspect lint spec/ --format json     # Spectral-style linting
$ suspect bundle api.yaml --strategy inline -o bundled.yaml
$ suspect overlay apply overlay.yaml api.yaml -o out.yaml
$ suspect diff old.yaml new.yaml
$ suspect stats api.yaml
$ suspect fmt api.yaml --json
$ suspect bench fixture.yaml           # per-stage timings
$ suspect lsp                          # stdio language server
```

## SDK generation

Generate the shared create/update/list/get example contract:

```sh
suspect codegen crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile typescript-http --package-name @example/widgets \
  --package-version 0.1.0 --out generated
```

Run `suspect codegen-profiles --format json` to discover the profiles available
in your build. Package identity is explicit configuration. Repeat
`--operation-id NAME` to select exact source operations; omitting it attempts
all outgoing operations. Unsupported selected contracts produce source-linked
diagnostics before output. Add `--check --format json` for read-only drift checks.

For multi-target generation, watch and editor previews, use
[`codegen-session`](docs/SDK-INCREMENTAL-GENERATION.md). Compare source snapshots
and generated interfaces with [`codegen-compare`](docs/SDK-COMPATIBILITY-REPORTS.md).
Markdown and custom templates use `suspect gen api.yaml --preset docs-md` or
`suspect gen api.yaml --manifest FILE`; native SDK docs are emitted by each profile.

## Notable engineering

- **Vendored grammars**: `crates/suspect-syntax/grammars/` with a local patch
  widening the YAML scanner's `int16_t` row tracking to `int32_t` — upstream
  tree-sitter-yaml v0.7.2 corrupts parses beyond 32,767 lines; suspect handles
  100k+-line documents (guarded by `crates/suspect-low/tests/large_yaml.rs`).
- **Cycle taxonomy**: `$ref` loops are classified as legal schema recursion or
  illegal (unresolvable) loops with exact cycle paths — never stack overflow,
  never hangs.
- **CI**: `.github/workflows/ci.yml` gates fmt, clippy (`-D warnings`), tests,
  and a bench smoke run.

## VS Code extension

```console
# build the server once
cargo build --release -p suspect-cli

# point the extension at it (or rely on `suspect` being on PATH)
mkdir -p ~/.vscode/extensions/suspect-lsp
cp -r .vscode-suspect/* ~/.vscode/extensions/suspect-lsp/
cd ~/.vscode/extensions/suspect-lsp && npm install vscode-languageclient@9
```

Then set `"suspect.server.path"` to the absolute path of the `suspect`
binary and reload the window. The `.vscode-suspect/README.md` documents the
full configuration surface.
