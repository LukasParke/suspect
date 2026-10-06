# Validating suspect's LSP beyond VS Code

VS Code is one client. It sends a rich capability set: dynamic
registration, pull diagnostics, semantic-token deltas, workspace
configuration. Other editors send far less, and every capability the
server branches on is a path VS Code never exercises. This directory
stages the multi-client validation.

## What each client uniquely exercises

| client | what it sends that VS Code does not | what it omits |
|---|---|---|
| minimal (harness) | — | everything: no dynamic registration, no pull diagnostics, no configuration — the floor every server must answer on |
| Neovim | its own cancel-during-flight timing; `vim.lsp` semantics | no pull diagnostics by default; no workspace/configuration until 0.11+ |
| Zed | a different initialize shape; no dynamic registration | no `workspace/configuration` (server must not block on it); no pull diagnostics |
| Helix | UTF-8 position encodings negotiation | no dynamic registration, no configuration |
| Emacs (lsp-mode/eglot) | eglot: no semantic tokens by default; very conservative caps | everything optional |

## The staged validations

### 1. Minimal-capability conformance (always runs)

`crates/suspect-cli/tests/lsp_session.rs::a_minimal_client_gets_everything`
drives the real server with a capability set stripped to nothing — the
floor that Zed, Helix, and eglot all sit near. It asserts the whole
advertised surface answers and produces content, because a server that
gates behaviour on a capability VS Code sends is a server those editors
silently lose features in.

Run: `cargo test -p suspect-cli --test lsp_session a_minimal_client`

### 2. Neovim headless smoke (`nvim_smoke.lua`)

Drives the real editor: attach, hover, documentSymbol, semanticTokens,
diagnostics — through `vim.lsp`'s own client. Skipped when `nvim` is not
on PATH.

    ./editors/multiclient/run_nvim.sh

Needs once per machine: `brew install neovim`.

### 3. Zed smoke (`zed_smoke.sh`)

Zed is installed on this machine but never attached during earlier
testing. The smoke script builds an isolated fixture with project-local
`.zed/settings.json` — never touching global Zed settings, which hold
user secrets — launches Zed on it, and verifies from the process table
and Zed's log that the server attached and answered.

    ./editors/multiclient/zed_smoke.sh

### 4. Capability-matrix notes

The harness captures capability sets as fixtures
(`crates/suspect-cli/tests/fixtures/vscode-capabilities.json` is a live
capture). Add a captured `initialize` from each IDE as
`zed-capabilities.json`, `nvim-capabilities.json`, … and the session
suite can replay each editor's exact handshake — the same method that
caught the burst deadlock, applied to client diversity.

## What has actually been validated, and when

- **VS Code 1.140** — full suite, live editor, hover/lint/markdown
  verified visually by the user.
- **Minimal caps** — the conformance test above.
- **Zed** — attach smoke (this machine).
- **Neovim / Helix / Emacs** — staged, not run on this machine (not
  installed). The harness replay path (§4) is the substitute until they
  are.
