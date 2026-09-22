# Native TypeScript floor toolchain

This private development tool pins the declared **TypeScript 5.5.4 floor**
compiler beside the package's pinned current 5.9.3. It does not alter generated
packages and none of its dependencies become generated SDK runtime
dependencies. The direct version is pinned in `package.json`; `package-lock.json`
is the exact registry resolution for TypeScript 5.5.4
(`sha512-Mtq29sKDAEYP7aljRgtPOpTvOfbwRWlS6dPRzwjdE+C0R4brX/GUyhHSecbHMFLNBLcJIPt9nl9yG5TZ1weH+Q==`).

The opt-in native gate `crates/suspect-codegen/tests/typescript_m2_toolchains.rs`
copies this `package.json`/`package-lock.json` into a temporary directory and
runs `npm ci --ignore-scripts` there, so the repository tree stays clean. The
compiler binary is `node_modules/typescript/bin/tsc` inside that install. Missing
required tools (pinned Node 22.23.1, npm, an explicit Node 24 binary, the
tracked OpenRouter checkout) fail the gate; they never skip it.

For an intentional dependency update, resolve the exact direct dependency and
review the resulting lock:

```sh
npm install --package-lock-only --ignore-scripts \
  --registry=https://registry.npmjs.org \
   --no-audit --no-fund
```

Run the gate (Node 22 pinned, Node 24 explicit additional path):

```sh
export OPENROUTER_WEB_ROOT=/Users/luke/github/openrouter-web
export SUSPECT_DOCS_NODE=/Users/luke/.local/share/mise/installs/node/22.23.1/bin/node
export SUSPECT_NODE24_BIN=/path/to/node-24
cargo test --locked -p suspect-codegen --test typescript_m2_toolchains -- --include-ignored
```

The independent `tests/m2_browser.rs` gate runs the generated ESM in isolated
headless Chromium with a real loopback server, exact response numbers and abort.
