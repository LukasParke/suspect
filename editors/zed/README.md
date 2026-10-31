# suspect for Zed

OpenAPI/Arazzo/Overlay language support for [Zed](https://zed.dev), backed by
the `suspect` language server (`suspect lsp`). The extension registers a
`suspect-lsp` language server for **YAML** and **JSON** buffers; YAML/JSON
language grammars themselves come from Zed's built-ins.

## The server binary

Nothing to install on macOS (Apple silicon and Intel), Linux (x86-64 and
arm64) and Windows (x86-64). The extension resolves the server binary in
this order:

1. `lsp.suspect-lsp.binary.path` in your Zed settings (with optional
   `binary.arguments`, default `["lsp"]`, and `binary.env`);
2. the `suspect` release build matching this extension's version: on first
   start the extension downloads `suspect-cli-<triple>.tar.gz` (`.zip` on
   Windows) from the GitHub release `v<version>`, checks it against the
   release's `.sha256` file, and unpacks the binary into its work directory
   (`suspect-<version>/`); later starts reuse it, and a new extension
   version removes the old copy;
3. an executable named `suspect` on the worktree's shell `PATH` (resolved by
   Zed on the host): other platforms, or no network on first start.

If none is found the language server fails to start with an error naming
the options. To keep your own build, set `binary.path`:

```json
{
  "lsp": {
    "suspect-lsp": {
      "binary": { "path": "/path/to/suspect", "arguments": ["lsp"] }
    }
  }
}
```

## Non-spec YAML and JSON

Zed starts the server for every YAML and JSON buffer, because a language
config cannot look at file contents. The server decides per document: on
files that are not OpenAPI, Swagger, Arazzo, Overlay, load profiles,
suspect config or fragments of a loaded spec (a `$ref` target, or a file in
a spec's directory below the workspace root), it returns no hover,
formatting, completion, code actions, symbols or links, and no
diagnostics. `docker-compose.yml` and `package.json` are left alone.

To keep the server off JSON buffers entirely:

```json
{ "languages": { "JSON": { "language_servers": ["!suspect-lsp", "..."] } } }
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
The keys the server reads are the `.suspect.yaml` keys, in camelCase (the
VS Code `suspect.*` settings) or snake_case:

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
        "editor": { "inlayHints": { "refs": true, "properties": true } },
        "format": { "sortKeys": false },
        "test": { "baseUrl": "http://localhost:8080" }
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
  `suspect/generationContract`, `suspect/renderSdk` and
  `suspect/documentKind`, each listed with a version in the server's
  `capabilities.experimental.suspect`. Zed has no UI for
  custom requests, so these are only reachable from other clients today.

## Pending

- Preview panels (the `suspect.renderPreview` lens and SDK previews): the Zed
  extension API does not expose webviews.
- Bundled prebuilt server binaries per platform (currently the host must
  provide `suspect` itself).
