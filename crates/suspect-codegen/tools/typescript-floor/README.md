# Native TypeScript floor toolchain

This private development tool pins the declared **TypeScript 5.5.4 floor**
compiler beside the package's pinned current 5.9.3. It does not alter generated
packages and none of its dependencies become generated SDK runtime
dependencies. The direct version is pinned in `package.json`; `package-lock.json`
is the exact registry resolution for TypeScript 5.5.4
(`sha512-Mtq29sKDAEYP7aljRgtPOpTvOfbwRWlS6dPRzwjdE+C0R4brX/GUyhHSecbHMFLNBLcJIPt9nl9yG5TZ1weH+Q==`).

The opt-in native gate `crates/suspect-codegen/tests/m2_vertical.rs` uses this
compiler alongside the current toolchain to check the self-contained
TypeScript/Rust installed-package fixture. The compiler binary is
`node_modules/typescript/bin/tsc` inside an installation. Missing required
tools fail the gate; requirements are documented in
[native call sites](../../../../docs/SDK-M2-CALLSITES.md).

For an intentional dependency update, resolve the exact direct dependency and
review the resulting lock:

```sh
npm install --package-lock-only --ignore-scripts \
  --registry=https://registry.npmjs.org \
   --no-audit --no-fund
```

Run the shared native gate with the pinned Node toolchain and Cargo available:

```sh
export SUSPECT_PACKAGE_NODE=/path/to/node-22.23.1
cargo test --locked -p suspect-codegen --test m2_vertical -- --include-ignored
```
