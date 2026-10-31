# Editors

`suspect lsp` is a language server for OpenAPI, Swagger 2.0, Arazzo, Overlay,
load profiles and suspect's own config files, over stdio. The repository ships
a VS Code extension and a Zed extension, and any LSP client works. The full
capability list is the [LSP feature matrix](../reference/lsp-features.md).

The server reports the same findings as `suspect check`, with the same codes
and severities, because both read the same `.suspect.yaml`. The
[linting guide](linting.md) covers the settings and suppression comments
(the editor ignores the CI baseline).

## Which files the server works on

Clients start the server for every YAML and JSON file. The server decides per
document. Files that are not an API description, an Arazzo or Overlay
document, a load profile, a suspect config file, or a fragment of a loaded
spec (a `$ref` target, or a file in a spec's directory below the workspace
root) get no diagnostics, hover, formatting, completion, code actions,
symbols or links. `docker-compose.yml` and `package.json` are left alone.

## In action (VS Code)

Every screenshot below is drawn from real server output for
[`docs/demo/petstore.yaml`](../demo/petstore.yaml). Regenerate them with
`cargo run --example showcase -p suspect-lsp && python3 docs/capture/render.py`
(needs Chrome and Pillow).

Diagnostics from the validator and the Spectral-compatible linter, in the
editor and the Problems panel:

![Diagnostics](../images/lsp-diagnostics.png)

Hover over a `$ref` resolves it (across files, too) and previews the target:

![Hover](../images/lsp-hover.png)

Quick fixes derived from diagnostics, plus `source.fixAll.suspect`:

![Code actions](../images/lsp-code_actions.png)

Completions inside a `$ref` offer the components declared across the
workspace:

![Completion](../images/lsp-completion.png)

Go to definition peeks at the `$ref` target:

![Go to definition](../images/lsp-goto_def.png)

Semantic highlighting; the legend lists the token types in this file:

![Semantic tokens](../images/lsp-semantic_tokens.png)

Inlay hints name each `$ref` target and mark required properties:

![Inlay hints](../images/lsp-inlay_hints.png)

Workspace symbols cover paths, operations and components:

![Workspace symbols](../images/lsp-workspace_symbols.png)

Workspace-wide rename rewrites the declaration and every `$ref` that points
at it (guarded by `prepareRename`; cross-file edits land in one atomic
`WorkspaceEdit`) — shown here renaming `Pet`:

![Rename](../images/lsp-rename.png)

## VS Code

The extension lives in [`editors/vscode/`](../../editors/vscode/). Build it
with `cd editors/vscode && npm ci && npm run compile`, then run it from the
extension development host or package it with `npm run package`.

It finds the `suspect` binary in this order:

1. `suspect.basePath`: a directory containing `suspect`, the executable
   itself, or bare `suspect`. The older `suspect.server.path` is used only
   while `suspect.basePath` is left at its default.
2. The binary bundled in a platform VSIX (`bin/suspect`), so a stale
   `cargo install` on `PATH` does not win over the build the extension was
   made for.
3. `suspect` on `PATH`.

`Suspect: Locate Binary…` picks one by hand. The server compares the
extension's version with its own and shows one warning when the major or
minor version differs.

Settings use the `.suspect.yaml` keys in camelCase under `suspect.`:
`suspect.lint.minSeverity`, `suspect.lint.rules`, `suspect.validate.strictFormat`,
`suspect.editor.inlayHints.refs`, `suspect.test.baseUrl`, `suspect.gateway.port`,
`suspect.codegen.packageName` and so on. An editor setting is layered over
`.suspect.yaml`, and one left empty defers to the file. Older spellings
(`suspect.inlayHints.refs`, `suspect.testBaseUrl`, `suspect.gatewayPort`,
`suspect.sdk.*`, `suspect.ci.baseline`) still work, and the server logs the
replacement. `Suspect: Explain Configuration` shows every effective setting
and the layer it came from, editor settings included (what `suspect config
--explain` shows, plus the layer the CLI cannot see), and `Suspect: Open
Configuration` opens
`.suspect.yaml`. `.suspect.yaml` and `suspect.project.json` are validated
against the checked-in JSON Schemas.

Beyond the language features, the extension adds:

- a Testing view for Arazzo workflows and an Arazzo notebook
  (`Suspect: Run Workflow`, `Suspect: Open Workflow as Notebook`);
- the mock gateway (`Suspect: Start Mock Gateway`, `Suspect: Stop Mock
  Gateway`; see the [gateway guide](gateway.md));
- SDK, docs and template generation, SDK previews, drift checks and watch
  (`Suspect: Generate SDK, Docs or Custom Templates`, `Suspect: Live SDK
  Preview`; see [SDK editor preview](sdk-editor-preview.md));
- `Suspect: Breaking Changes (vs base)` against `diff.baseline` (default
  `HEAD~1`), `Suspect: Change Impact at Cursor`, `Suspect: Show $ref Graph`
  and `Suspect: Contract Coverage`;
- `Suspect: Set Minimum Finding Severity`, which writes
  `suspect.lint.minSeverity`.

## Zed

The extension lives in [`editors/zed/`](../../editors/zed/README.md). It
registers `suspect-lsp` for YAML and JSON buffers. On first start it
downloads the release build that matches its version, checks it against the
release's `.sha256` file and reuses it afterwards; `lsp.suspect-lsp.binary.path`
points it at your own build instead. Install it as a dev extension with
`zed ext install editors/zed`, or from *Extensions → Install Dev Extension*.

Settings under `lsp.suspect-lsp.settings` are forwarded to the server as the
`suspect` section, in camelCase or snake_case:

```json
{
  "lsp": {
    "suspect-lsp": {
      "settings": {
        "lint": { "minSeverity": "warning", "rules": { "operation-tags": "error" } },
        "test": { "baseUrl": "http://localhost:8080" }
      }
    }
  }
}
```

Zed has no webviews, so the preview panels are not available. To keep the
server off JSON buffers, add `"!suspect-lsp"` to the JSON language's
`language_servers`.

## Other clients

Run `suspect lsp` over stdio. Neovim, Helix and Emacs (lsp-mode or eglot)
need nothing else; the server works with a minimal capability set and does
not block on `workspace/configuration`. Settings come from `.suspect.yaml`,
or from `workspace/configuration` under the `suspect` section when the
client sends it. Smoke configurations for other clients are in
[`editors/multiclient/`](../../editors/multiclient/README.md).

Neovim, for example:

```lua
vim.lsp.config("suspect", {
  cmd = { "suspect", "lsp" },
  filetypes = { "yaml", "json" },
  root_markers = { ".suspect.yaml", "suspect.project.json", ".git" },
})
vim.lsp.enable("suspect")
```
