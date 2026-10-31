# Contributing

## Layout

- `crates/` — the Rust workspace; `crates/suspect-cli` builds the `suspect`
  binary (CLI and `suspect lsp`).
- `editors/vscode`, `editors/zed` — editor clients for `suspect lsp`;
  `editors/multiclient` holds Neovim/Zed smoke scripts.
- `rules-runtime/` — the TypeScript SDK and Bun worker for custom rules.
- `xtask/` — `cargo xtask ci`, corpus fetching and SDK verification tasks.
- `docs/` — see [docs/README.md](docs/README.md) for the index.

## Checks

`cargo xtask ci` runs every job CI runs, with the same commands; `cargo xtask
ci --list` shows them, `cargo xtask ci clippy test` runs a subset, and a CI
failure names the job to rerun locally. Jobs whose tool is missing fail with
an install hint; `--skip-missing` reports them as skipped instead.

`cargo xtask ci fmt clippy test` is the quick loop. The full run includes
`bench-smoke`, `corpus-validation` (downloads real-world specs),
`platform-smoke`, the editor extensions, `rules-runtime` and `links` (lychee
0.24.2, offline; see `lychee.toml`). `rust-toolchain.toml` pins the Rust
version CI uses.

The real-host VS Code smoke needs a desktop session and stays manual (macOS
app path by default, `SUSPECT_VSCODE_BIN` overrides it):

```sh
cargo build -p suspect-cli && (cd editors/vscode && npm run test:host:smoke)
```

## Generated files

- `docs/CLI.md` is rendered from the clap command tree. After changing a
  command or flag, regenerate it:
  `SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`.
  `cargo xtask bless` regenerates every generated doc (`docs/CLI.md`,
  `docs/SDK-FEATURES.md`) in one step.
- `editors/vscode/dist` and `node_modules` are build output and are not
  committed.

## Native SDK gates

Tests that compile and run generated SDKs need real toolchains (Go, JDK/Maven,
.NET, Swift, Dart, Node, Ruby, PHP, Python). The rule:

- A test in the default suite may skip when a tool is missing. It prints one
  `skipped:` line naming the tool and how to install it.
- A test marked `#[ignore]` is an explicit gate. When you run it
  (`cargo test -p suspect-codegen -- --ignored`), a missing tool or input
  fails it. No gate passes by skipping.
- CI sets `SUSPECT_REQUIRE_TOOLS` for the tools it installs, so a skip there
  is a failure.

Use `suspect_codegen::toolchain::optional` in default-suite tests and
`toolchain::require` in `#[ignore]` gates; `toolchain.rs` declares each tool,
its minimum version and its `SUSPECT_*_BIN` override. Gates that read the
OpenRouter specification need `OPENROUTER_WEB_ROOT=/path/to/openrouter-web`
and get it from `suspect_codegen::test_inputs`.

The corpus suite skips without `corpus/` (`cargo xtask fetch-corpus`), unless
`SUSPECT_REQUIRE_CORPUS=1`, which `cargo xtask ci corpus-validation` sets.

## Conventions

- Exit codes: `0` clean, `1` findings or failed checks, `2` usage or
  environment error.
- Settings belong in `crates/suspect-config` so the CLI and the language
  server read them the same way.

## License

By contributing you agree that your contributions are licensed under either
of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
