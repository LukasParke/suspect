# suspect for Zed

OpenAPI/Arazzo/Overlay language support for [Zed](https://zed.dev), backed by
the `suspect` language server (`suspect lsp`). The extension registers a
`suspect-lsp` language server for **YAML** and **JSON** buffers; YAML/JSON
language grammars themselves come from Zed's built-ins.

## Prerequisites

The `suspect` CLI must be installed somewhere Zed can find it:

```sh
# from the repository root
cargo install --path crates/suspect-cli
```

The extension resolves the server binary in this order:

1. the `SUSPECT_PATH` environment variable (verbatim path override);
2. an executable named `suspect` found on `PATH`.

## Install (as a dev extension)

```sh
cd editors/zed
zed ext install .
```

(or from inside Zed: *Extensions → Install Dev Extension → select
`editors/zed`*).

## Build from source

The extension compiles to WebAssembly. Host `cargo check` works for editor
tooling, but packaging requires the WASI target:

```sh
rustup target add wasm32-wasip1
cd editors/zed
cargo build --target wasm32-wasip1
```

CI runs the host-target equivalent (`cargo check --manifest-path
editors/zed/Cargo.toml`) in the `zed-check` workflow job; the extension crate
is intentionally standalone (its own `[workspace]`) so it never touches the
repository root lockfile.

## Configuration

Per-project Zed settings (`lsp` section):

```json
{
  "lsp": {
    "suspect-lsp": {
      "settings": {
        "basePath": "http://127.0.0.1:8080",
        "testBaseUrl": "http://127.0.0.1:8080"
      }
    }
  }
}
```

These are forwarded to the server as workspace configuration under the
`suspect` section: `{ "suspect": { "basePath", "testBaseUrl" } }`.
`basePath` anchors `$url`-style external references and hover base URLs;
`testBaseUrl` is the default base URL for Arazzo test runs started from the
editor.

## What works

- Full LSP feature set over stdio (`suspect lsp`): diagnostics (validation +
  Spectral-style linting), `$ref` navigation, rename, completions, hover,
  quick fixes, inlay hints, semantic highlighting.
- Workspace configuration forwarding for `basePath` / `testBaseUrl`.
- Tasks: Arazzo workflow runs stream back through the journal event channel;
  custom LSP methods (`suspect/runWorkflow`, `suspect/gatewayStatus`,
  `suspect/renderPreview`) are served by the CLI subprocess.

## Pending

- Webview-based preview panels (`renderPreview` UI surface) — Zed extension
  API does not expose webviews yet; the LSP method already works, only the
  visual panel is missing.
- Bundled prebuilt server binaries per-platform download (currently the host
  must provide `suspect` itself).
