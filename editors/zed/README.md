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

1. `lsp.suspect-lsp.binary.path` in your Zed settings (with optional
   `binary.arguments`, default `["lsp"]`, and `binary.env`);
2. an executable named `suspect` on the worktree's shell `PATH` (resolved by
   Zed on the host; the extension itself runs in a WASI sandbox and cannot
   read `PATH` or the filesystem directly).

If neither is found the language server fails to start with an error naming
both options.

```json
{
  "lsp": {
    "suspect-lsp": {
      "binary": { "path": "/path/to/suspect", "arguments": ["lsp"] }
    }
  }
}
```

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

CI builds the same WASI artifact (`cargo build --manifest-path
editors/zed/Cargo.toml --target wasm32-wasip1`) in the `zed` job of the
`platform` workflow; the extension crate is intentionally standalone (its own `[workspace]`) so it never touches the
repository root lockfile.

## Configuration

Everything under `lsp.suspect-lsp.settings` is forwarded to the server as
workspace configuration under the `suspect` section (it is wrapped for you;
settings already nested under a `suspect` key are passed through unchanged).
The keys the server reads mirror `.suspect.yaml` and the VS Code `suspect.*`
settings:

```json
{
  "lsp": {
    "suspect-lsp": {
      "settings": {
        "lint": {
          "minSeverity": "warning",
          "rules": { "operation-tags": "error" },
          "design": "off",
          "recommended": true,
          "ruleset": "rules/custom.yaml"
        },
        "validate": { "strictFormat": false },
        "ref": { "maxDocs": 500 },
        "inlayHints": { "refTargets": true, "properties": true },
        "formatting": { "sortKeys": false }
      }
    }
  }
}
```

`lint.minSeverity` accepts `error`, `warning`, `info` (or `information`) and
`hint`; leave it unset to defer to the committed `.suspect.yaml`. Settings
from the editor layer over the committed `.suspect.yaml` /
`suspect.project.json` files.

The extension sends `extensionVersion` in the server's initialization options,
merged into anything you set under `lsp.suspect-lsp.initialization_options`.
The server compares it with its own version and shows one warning when the
major or minor release differs (for example a `binary.path` pointing at an
older build).

## What works

- Full LSP feature set over stdio (`suspect lsp`): diagnostics (validation +
  linting), `$ref` navigation, rename, completions, hover, quick fixes, inlay
  hints, semantic highlighting, code lenses.
- Workspace configuration forwarding (see above), re-sent on every settings
  change.
- Server commands over `workspace/executeCommand` (e.g. `suspect.showRefs`,
  `suspect.addOperationId`, `suspect.generateExample`,
  `suspect.extractSchema`, `suspect.inlineSchema`), reachable from code
  actions and lenses.
- Custom requests the server implements: `suspect/runWorkflow`,
  `suspect/generationContract` and `suspect/renderSdk`. Zed has no UI for
  custom requests, so these are only reachable from other clients today.

## Pending

- Preview panels (the `suspect.renderPreview` lens and SDK previews): the Zed
  extension API does not expose webviews.
- Bundled prebuilt server binaries per platform (currently the host must
  provide `suspect` itself).
