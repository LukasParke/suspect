# Contributing

## Layout

- `crates/` — the Rust workspace; `crates/suspect-cli` builds the `suspect`
  binary (CLI and `suspect lsp`).
- `editors/vscode`, `editors/zed` — editor clients for `suspect lsp`;
  `editors/multiclient` holds Neovim/Zed smoke scripts.
- `rules-runtime/` — the TypeScript SDK and Bun worker for custom rules.
- `xtask/` — corpus fetching and SDK verification tasks.
- `docs/` — see [docs/README.md](docs/README.md) for the index.

## Checks

CI runs the same commands; run them before opening a pull request.

```sh
# Rust
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check                      # cargo install cargo-deny

# Relative links in tracked Markdown (lychee 0.24.2, offline; see lychee.toml)
cargo xtask ci links

# VS Code extension
cd editors/vscode && npm ci && npm run compile && npm run test:headless
# ...and in a real VS Code window (desktop session; macOS app path by default,
# SUSPECT_VSCODE_BIN overrides it)
cargo build -p suspect-cli && (cd editors/vscode && npm run test:host:smoke)

# Zed extension
cd editors/zed && rustup target add wasm32-wasip1 && cargo build --target wasm32-wasip1

# Custom rules runtime
cd rules-runtime && bun test
```

The corpus suite needs the real-world specs:
`cargo run -p xtask -- fetch-corpus && cargo test -p suspect-cli --test corpus_suite`.

## Generated files

- `docs/CLI.md` is rendered from the clap command tree. After changing a
  command or flag, regenerate it:
  `SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`.
- `editors/vscode/dist` and `node_modules` are build output and are not
  committed.

## Native SDK gates

Behavioral gates that compile and run generated SDKs need real toolchains
(Go, JDK/Maven, .NET, Swift, Dart, Node, Ruby, PHP, Python). They are opt-in
(`#[ignore]`) or skip with a message naming what is missing.
`crates/suspect-codegen/src/toolchain.rs` declares each tool, its minimum
version and its `SUSPECT_*_BIN` override; run
`cargo test -p suspect-codegen -- --ignored` to include them.

## Conventions

- Exit codes: `0` clean, `1` findings or failed checks, `2` usage or
  environment error.
- Settings belong in `crates/suspect-config` so the CLI and the language
  server read them the same way.

## License

By contributing you agree that your contributions are licensed under either
of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
