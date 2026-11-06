# Native SDK suites are opt-in, run nightly with real toolchains, and use pinned inputs

Each backend's acceptance suites build and run the generated package with the real language toolchain (javac, dotnet, swift, …), so they are `#[ignore]`d: `cargo test --workspace` and the pull-request gate (`ci.yml`) stay hermetic and fast, and `native-evidence.yml` runs every backend's suites nightly on one runner per backend. To keep an opt-in gate honest, that job sets `SUSPECT_REQUIRE_NATIVE=1`, which turns a missing or broken toolchain into a failure instead of a skip, and the toolchain lookup (`suspect_codegen::toolchain`) never depends on one developer's machine. Suites that exercise a real third-party API description pin the exact revision by SHA-256 (`toolchain::openrouter_spec`) and skip when it is unavailable, so upstream edits cannot fail them, and per-suite packages (npm tools, the Python tools venv, Ruby gems, PHP phars) come from one script, `tools/native-setup.sh`.

## Considered Options

- Run the native suites on every pull request: rejected for cost and flakiness (twelve toolchains, several minutes each, network-dependent installs).
- Test against the latest upstream OpenRouter description: rejected; it changed under the suites and failed them on drift that says nothing about the generator.

## Consequences

A native regression shows up within a day rather than on the pull request that caused it. Local runs need the toolchain installed; `toolchain::require` prints what is missing and how to install it. Cross-backend suites (`tests/groups/shared.rs`) get their own nightly job, since a suite no job runs decays silently. The pinned OpenRouter revision lives in a private repository, so the nightly jobs check it out only when the `OPENROUTER_WEB_TOKEN` secret is set; without it those suites skip.
