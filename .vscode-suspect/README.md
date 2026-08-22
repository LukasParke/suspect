# suspect-lsp (VS Code extension)

Bundles a VS Code language-client that launches the `suspect` LSP server for
OpenAPI (2/3.x), Arazzo, and Overlay documents.

## Install

1. Build the server once from the workspace root:

   ```console
   cargo build --release -p suspect-cli
   ```

2. Copy this directory into your VS Code extensions folder and install its
   only runtime dependency:

   ```console
   mkdir -p ~/.vscode/extensions/suspect-lsp
   cp -r .vscode-suspect/* ~/.vscode/extensions/suspect-lsp/
   cd ~/.vscode/extensions/suspect-lsp && npm install vscode-languageclient@9
   ```

3. Point the client at the server binary — add to your user `settings.json`:

   ```json
   { "suspect.server.path": "/absolute/path/to/target/release/suspect" }
   ```

4. Reload the window. Opening any `.yaml`/`.json` file starts the server.

## Features

- Diagnostics: syntax errors, 22 semantic checks (`oas-*` codes), 21
  Spectral-compatible lint rules, Arazzo validation — debounced and merged.
- Navigation: goto-definition / references across `$ref` boundaries
  (multi-file workspaces), document highlights, selection ranges.
- Hover: resolved `$ref` target previews with source excerpts.
- Completions: context-aware object keys plus `$ref` candidates collected
  from every loaded document.
- Quick fixes: insert missing `operationId`/`responses`/`description`,
  contact/license skeletons, strip trailing slashes — plus
  `source.fixAll.suspect`.
- Rename: component declarations with workspace-wide `$ref` rewriting,
  guarded by `prepareRename` (component sections only in v1).
- Semantic highlighting, `$ref`-target + property-type inlay hints, document
  symbols, folding ranges, workspace symbols, document formatting.

## Commands

| Command | Description |
|---|---|
| `suspect.restart` | Restart the language server |

## Settings

| Setting | Default | Description |
|---|---|---|
| `suspect.server.path` | `"suspect"` | Server binary path |
| `suspect.diagnostics.enable` | `true` | Publish diagnostics |
