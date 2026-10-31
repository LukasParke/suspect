# suspect

High-performance Rust toolkit for OpenAPI (3.0, 3.1, 3.2; 2.0 read via family
sniffing), Arazzo 1.0, and Overlay 1.0: lossless tree-sitter parsing, native
`$ref` resolution with cycle classification, JSON Schema 2020-12 validation,
Spectral-compatible linting, an LSP server, and a CLI.

Design constraints: parse once, view many; zero-copy scalars; arena-indexed
nodes; lazy memoized resolution. Benchmarks with honest budget verdicts live
in [BENCHMARKS.md](BENCHMARKS.md).

SDK generation uses one source-addressed contract pipeline for twelve native
backends — **TypeScript/JavaScript, Python, Go, Rust, Swift, Java, Kotlin, C#,
Ruby, PHP, Dart and C++** — producing native models, checked codecs, HTTP
operations, packages, golden-defaults behavior (attribution, pagination,
OAuth, env-credential defaults) and documentation.
Start with the [SDK generation guide](docs/guide/sdk-generation.md); the
[SDK capabilities](docs/reference/sdk-capabilities.md) page compares the
backends.

## Install

Prebuilt binaries for macOS, Linux (glibc and static musl) and Windows are
attached to every [release](https://github.com/LukasParke/suspect/releases)
(Windows is tier 2 until one release has passed its smoke tests):

```console
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/LukasParke/suspect/releases/latest/download/suspect-cli-installer.sh | sh
# Homebrew
brew install LukasParke/tap/suspect
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/LukasParke/suspect/releases/latest/download/suspect-cli-installer.ps1 | iex"
# From source (needs Rust)
cargo install --locked --git https://github.com/LukasParke/suspect suspect-cli
suspect --version
```

The installers put `suspect` in `~/.cargo/bin`, the same place `cargo
install` uses. Each archive (`suspect-cli-<target>.tar.gz`, `.zip` on
Windows) has a `.sha256` file next to it, and `gh attestation verify <file>
--repo LukasParke/suspect` checks its build provenance.

In GitHub Actions, `LukasParke/suspect/.github/actions/suspect@v<version>`
downloads the release binary and can run `check`, `test` or `gen`
([`action.yml`](.github/actions/suspect/action.yml)).

## Tour

Check a spec. `check` runs every check the editor runs and exits 1 on errors;
`validate` and `lint` are filtered views of it:

```console
$ suspect check api.yaml
api.yaml:8:7 [WARNING] oas-operation-missing-operationId: get operation is missing an operationId
how to fix: Add an `operationId` so generated SDK methods carry a stable name.
$ suspect check api.yaml --format sarif --baseline .suspect-baseline.json
```

Settings live in one file, `.suspect.yaml`, read by every command, CI and the
editor; `suspect config --explain` shows each value and where it came from.
Findings can be suppressed inline or recorded in a baseline
([linting guide](docs/guide/linting.md)).

Open the same spec in an editor. `suspect lsp` serves diagnostics, `$ref`
navigation across files, hover, completion, rename, quick fixes, formatting,
semantic highlighting and inlay hints to VS Code, Zed, Neovim, Helix and
Emacs ([editors guide](docs/guide/editors.md)):

![Diagnostics in VS Code](docs/images/lsp-diagnostics.png)

Generate an SDK, docs or your own templates:

```console
$ suspect gen sdk crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
    --profile typescript-http --package-name @example/widgets \
    --package-version 0.1.0 -o generated
$ suspect gen docs api.yaml --style markdown -o docs/api
```

`suspect gen profiles` lists the twelve backends. `--check` turns any
generator into a read-only drift check
([SDK generation](docs/guide/sdk-generation.md),
[docs generation](docs/guide/docs-generation.md)).

Run Arazzo workflows as contract tests or load tests, live or against
recorded traffic:

```console
$ suspect test run flows.arazzo.yaml --base-url http://localhost:8080 --input petName=Rex
$ suspect test load profile.load.yaml
$ suspect traffic gateway api.yaml -p 9000 --mode record --upstream http://staging:80 --cassette rec.scj
$ suspect test run flows.arazzo.yaml --cassette rec.scj
```

See [load testing](docs/guide/load-testing.md), the runnable example in
[`examples/load/`](examples/load/), and the [gateway guide](docs/guide/gateway.md).

## Command map

```text
suspect check         every check the editor runs; exit 1 on errors
suspect validate      check, structure and semantics only
suspect lint          check, lint, markdown and TypeScript rules only
suspect fmt | bundle | upgrade | stats
suspect overlay       apply | explain | diff
suspect diff          structural | breaking | impact | sdk | routes
suspect gen           docs | sdk | stubs | contract | terraform | cli | mcp
                      template | session | bridge | profiles | admission
suspect test          run | stateful | fuzz | load | replay
suspect traffic       gateway | drift | har
suspect release       plan | publish | workflow
suspect project       init | check | build
suspect ci            every project under a root, as one gate
suspect why           trace a validation failure to its origin
suspect rules         check | new | bench   (TypeScript rules on Bun)
suspect config | auth | acquire | watch | lsp
suspect dev           bench | evidence
```

Every command and flag is in the generated
[command reference](docs/reference/cli.md). The old flat names (`breaking`,
`codegen`, `gateway`, ...) still work with a deprecation warning; the
reference ends with the rename table. Exit codes are shared: `0` clean, `1`
findings or failed checks, `2` usage or environment error, `130`
interrupted.

## Documentation

- [Documentation index](docs/README.md): guides, references, per-language SDK
  notes and design records.
- Guides: [linting](docs/guide/linting.md), [editors](docs/guide/editors.md),
  [gateway](docs/guide/gateway.md), [docs generation](docs/guide/docs-generation.md),
  [SDK generation](docs/guide/sdk-generation.md), [load testing](docs/guide/load-testing.md).
- Reference: [CLI](docs/reference/cli.md), [LSP features](docs/reference/lsp-features.md),
  [SDK features](docs/reference/sdk-features.md).
- [How suspect compares](docs/internal/LSP_COMPARISON.md) with vacuum and
  Telescope.
- Plans: the [toolchain unification proposals](docs/proposals/README.md).
  Earlier plans, including the
  [feature roadmap](docs/internal/FEATURE-ROADMAP.md), are kept in
  [`docs/internal/`](docs/internal/README.md).
- [CONTRIBUTING.md](CONTRIBUTING.md): development setup, the crate map and
  `cargo xtask ci`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. See [CONTRIBUTING.md](CONTRIBUTING.md)
for development setup.
