# IDE testing and validation ladder

Ordered by cost/benefit. Each rung below the top is running today; the
rungs above are the staged next steps.

## Running now

| # | validation | what it catches | how |
|---|---|---|---|
| 1 | Harness session suite (44 tests) | protocol-level behaviour against the real binary: every feature, content asserted, wedges caught under burst | `cargo test -p suspect-cli --test lsp_session` |
| 2 | Load bench suite (14 scenarios) | perf regressions and wedges under editor-shaped load, incl. cancel storms and file switching | `cargo test -p suspect-cli --test lsp_bench` |
| 3 | Minimal-capability conformance | features that go dark for lean clients (Zed/Helix/eglot) — hover without workspace/configuration, etc. | `a_minimal_client_gets_everything` in the session suite |
| 4 | Extension manifest contract | contributions VS Code validates silently at load: scopes shape, menus referencing commands, view/container consistency | `node --test editors/vscode/test/manifest.cjs` |
| 5 | Zed attach smoke | a real second editor's handshake, from the process table and Zed's log | `editors/multiclient/zed_smoke.sh` |

## Staged next steps

| # | validation | what it adds | effort |
|---|---|---|---|
| 6 | **Neovim headless smoke** — attach, hover, symbols, pushed diagnostics through `vim.lsp` itself | a third client's framing and cancellation timing; lean-caps behaviour in a real editor | ready (`run_nvim.sh`); needs `brew install neovim` |
| 7 | **Per-editor handshake fixtures** — capture `initialize` from Zed/Neovim/Helix/eglot and replay each in the session suite | every editor's exact capability set driving the whole burst suite — the method that caught the burst deadlock, pointed at client diversity | capture scripts + fixture files; suite already supports replay |
| 8 | **Extension-host integration tests** (`@vscode/test-electron`) — run the real extension inside real VS Code, assert views/commands/status behaviour | the layer nothing else reaches: TreeView population, status-bar item, command wiring, diagnostics surfacing in the Problems panel | scaffold + first test for the new Overview view; needs the VS Code download it performs |
| 9 | **Position-encoding conformance** — drive the suite with `positionEncodings: ["utf-8"]` and `["utf-16"]` variants | off-by-one column bugs for non-ASCII spec text, which VS Code never surfaces but Helix negotiates | fixture handshakes; assertions on exact positions over multi-byte content |
| 10 | **Cancellation soak** — a client that cancels mid-flight aggressively (Neovim's pattern) while asserting the survivors | the cancel path under real-client timing, not just our own storm | nvim smoke extension or harness scenario |
| 11 | **Arazzo notebook + custom editor smoke** — open the notebook in real VS Code, run a step | the customEditors surface, which no protocol test can cover | test-electron rung |
| 12 | **Upgrade matrix** — run the session suite against the two newest VS Code insiders versions | regressions in the client (the 1.140 wedge class) | CI matrix entries |

## Where each rung lives

- Session/bench/conformance: `crates/suspect-cli/tests/`
- Manifest contract: `editors/vscode/test/manifest.cjs`
- Zed/Neovim smokes: `editors/multiclient/`
- Everything else lands in those same places as it's built.
