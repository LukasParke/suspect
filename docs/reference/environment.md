# Environment variables

<!-- Generated from the suspect_config::schema tables (`env` column and `DEBUG_VARS`) by crates/suspect-cli/tests/reference_docs.rs; do not edit by hand. Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`. -->

Every `SUSPECT_*` variable the `suspect` binary reads. A test (`crates/suspect-config/tests/env_registry.rs`) fails when the code reads one that is not listed here.

## Settings

Each variable sets one [configuration key](config.md). It overrides `.suspect.yaml` and is overridden by an editor setting or a flag; `suspect config --explain` names it as the origin.

| Variable | Key | Type | Default | Description |
|---|---|---|---|---|
| `SUSPECT_BASE_URL` | `test.base_url` | URL | `http://localhost:8080` | Base URL requests are sent to. |
| `SUSPECT_CREDENTIALS` | `test.credentials` | path |  | Credentials file for the security schemes tests exercise. Default: `.suspect/credentials.json` discovered upward. |
| `SUSPECT_GIT_BASE` | `diff.baseline` | string |  | Git ref compared against for breaking changes (`suspect ci`, the editor's breaking-changes command; the editor falls back to `HEAD~1`). |
| `SUSPECT_LINT_MIN_SEVERITY` | `lint.min_severity` | string: `error`, `warning`, `info`, `hint` | `hint` | Minimum severity to report. Findings below this floor are suppressed. |
| `SUSPECT_LINT_RULESET` | `lint.ruleset` | path |  | Custom ruleset document, relative to the file that sets it. |
| `SUSPECT_STRICT_FORMAT` | `validate.strict_format` | boolean | `false` | Assert `format` keywords. The 2020-12 default is annotation-only, so this catches example drift that annotation-only mode would let through. |

## Debugging

These change diagnostic output only; `suspect config --explain` lists the ones that are set.

| Variable | Prints |
|---|---|
| `SUSPECT_PROFILE` | lint/validate phase timings on stderr |
| `SUSPECT_GW_PROFILE` | gateway request timings on stderr |
| `SUSPECT_BUILD_PROFILE` | project build and contract stage timings on stderr |
| `SUSPECT_CONTRACT_TRACE` | contract compiler trace on stderr |
| `SUSPECT_RULES_DEBUG` | TypeScript rule host and worker trace on stderr |
| `SUSPECT_HLDBG` | semantic highlighting trace on stderr (language server) |
