# Local SDK demo helpers

Start with the repository-root [DEMO-README.md](../../DEMO-README.md).

All commands run from `/Users/luke/github/suspect`. Set `DEMO` to the prepared
`target/sdk-demo-readme-20260911-01/candidate-02` absolute path.

| Helper | Purpose |
| --- | --- |
| `demo.py --root "$DEMO" freeze --bin PATH --receipt PATH` | New-root-only CLI/source/config freeze; verifies four tracked OpenRouter inputs against HEAD. |
| `demo.py --root "$DEMO" freeze-comparison --bin PATH --receipt PATH` | Additively pin the compatibility-repair CLI and verify all generated SDK bytes against the original package pins. |
| `demo.py --root "$DEMO" generate/check/preview/regenerate` | Real canonical twelve-target generation, read-only checks and unchanged metadata verification. Choose one action. |
| `demo.py --root "$DEMO" verify-pins/provenance` | Verify retained inputs/binary or print the actual getCredits source bindings. Choose one action. |
| `native.py --root "$DEMO" build LANGUAGE` | Preparation: new private package/install/consumer attempt with pinned tools and offline dependency resolution. `all` selects the twelve targets. |
| `native.py --root "$DEMO" run LANGUAGE` | Execute the already prepared consumer against independent loopback responses; no build/install. `all` also runs JavaScript. |
| `native.py --root "$DEMO" error LANGUAGE` | The same read with a declared 401, exercising the actual typed error handler. |
| `native.py --root "$DEMO" where LANGUAGE` | Print exact package/install/working directory/executable/tool paths. |
| `iteration.py --root "$DEMO" --label LABEL prepare/change/restore/compare/watch/watch-check` | Separate private source copies; native model-constraint edit; compact view of actual CLI watch records. Choose one action. Compare uses the final additively pinned fixed CLI and all twelve native records, with a fresh `comparison-final-02/run-NN/` receipt each time. |
| `stream.py --root "$DEMO" prepare/run` | Standard-only OAS 3.2 SSE fixture, installed TypeScript package, no OpenRouter streaming inference. Choose one action. |
| `terraform.py --root "$DEMO" prepare` | Verify new CLI provider emission against the 48 sealed native fixture files; copy the pinned provider to an isolated filesystem mirror. |
| `terraform.py --root "$DEMO" run --label LABEL` | Fresh Terraform consumer, real init/plan/apply/refresh/import/destroy, controlled API server and retained state/wire logs. |
| `check_readme.py --root "$DEMO"` | Local links, exact excerpts, installed snippet pins, and optional-live read exercised with injected Fetch bytes. |
| `seal.py --root "$DEMO"` | New `delivery-NN/` inventory of completed evidence and execution-file hashes; no native replay. |

`build` is explicit preparation. `run` uses only prepared executables/imports.
Generated desired files stay in `packages/`; tool writes go to private native
copies. Attempt directories and command logs are append-only. The scripts do not
publish packages, run the full acceptance matrix, or collect numerical performance.
The initial package inventory and zero-rewrite receipt remain immutable;
subsequent generation/regeneration receipts use fresh `generation-receipts/` paths.
