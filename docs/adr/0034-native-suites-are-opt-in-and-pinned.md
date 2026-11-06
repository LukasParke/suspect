# Native SDK suites are opt-in, run nightly with real toolchains, and fail when inputs are missing

Each backend's acceptance suites build and run the generated package with the real language toolchain (javac, dotnet, swift, …), so they are `#[ignore]`d: `cargo test --workspace` and the pull-request gate (`ci.yml`) stay hermetic and fast, and `native-evidence.yml` runs each backend's grouped suite binary nightly. An `#[ignore]` gate someone asked for never passes by skipping: a missing tool or input (`toolchain::require`, `toolchain::pinned_node`, `test_inputs::openrouter_spec`) fails it with the message naming what to install or set. Default-suite tests may skip a missing tool (`toolchain::optional`), and `SUSPECT_REQUIRE_TOOLS` turns those skips into failures for the tools CI installs. Toolchain lookup never depends on one developer's machine, and per-suite packages (npm tools, the Python tools venv, Ruby gems, PHP phars) come from one script, `tools/native-setup.sh`. Suites over a real third-party API description read the `OPENROUTER_WEB_ROOT` checkout, which the nightly job checks out at `toolchain::OPENROUTER_SPEC_COMMIT`.

## Considered Options

- Run the native suites on every pull request: rejected for cost and flakiness (twelve toolchains, several minutes each, network-dependent installs).
- Let `#[ignore]` gates skip when a tool is missing: rejected; a requested gate that passes without running is false evidence.

## Consequences

A native regression shows up within a day rather than on the pull request that caused it. Running a gate locally needs its toolchain and inputs installed; the failure says how. Cross-backend suites (`tests/groups/shared.rs`) get their own nightly entry, since a suite no job runs decays silently. The OpenRouter checkout is private, so the nightly job needs the `OPENROUTER_WEB_TOKEN` secret.
