# Configuration reference

<!-- Generated from the suspect_config::schema setting tables by crates/suspect-cli/tests/reference_docs.rs; do not edit by hand. Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`. -->

suspect reads two files. `.suspect.yaml` holds the settings every command, CI and the editor share. `suspect.project.json` describes what a project builds. Both are validated against checked-in JSON Schemas generated from the same tables: [`suspect.schema.json`](../../editors/vscode/schemas/suspect.schema.json) and [`suspect.project.schema.json`](../../editors/vscode/schemas/suspect.project.schema.json). Point an editor's YAML or JSON schema setting at them; the VS Code extension does this already.

Precedence, lowest first: the default, the project manifest (deprecated policy keys), `.suspect.yaml` (over its `extends` parent), the environment variable, then an editor setting or a command-line flag. `lint.rules` merges per rule across layers. Paths are relative to the file that sets them. `suspect config --explain` lists every key with its value and where it came from. Older spellings still work and print a warning naming the current key. Environment variables are also listed in the [environment reference](environment.md).

## `.suspect.yaml`

Workspace-wide settings. Every suspect command, the language server and CI read this file, so the editor, a shell and CI see identical findings.

### `extends`

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `extends` | path |  | A parent settings file, relative to this one. Its values apply under this file's; one level only. |  |  |  |  |

### `lint`

Which findings are reported, and from where.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `lint.ruleset` | path |  | Custom ruleset document, relative to the file that sets it. | `SUSPECT_LINT_RULESET` | `suspect.lint.ruleset` | `suspect lint --ruleset` |  |
| `lint.min_severity` | string: `error`, `warning`, `info`, `hint` | `hint` | Minimum severity to report. Findings below this floor are suppressed. | `SUSPECT_LINT_MIN_SEVERITY` | `suspect.lint.minSeverity` | `suspect lint --min-severity` |  |
| `lint.design` | string: `error`, `warning`, `info`, `hint`, `off` |  | Severity for design-class rules: findings that flag the API's design rather than the document's accuracy. A project documenting an API it does not own sets `off` (or `info` to keep them as notes); `lint.rules` wins per rule. |  | `suspect.lint.design` |  |  |
| `lint.rules` | map of string: `error`, `warning`, `info`, `hint`, `off` | `{}` | Rule id to severity (`error`, `warning`, `info`, `hint`, `off`), overriding the severity a rule would otherwise produce. Merged per rule across files. |  | `suspect.lint.rules` |  |  |
| `lint.recommended` | boolean | `true` | Apply the recommended (built-in) lint rule set. With `false` and no `lint.ruleset`, no lint rules run. |  | `suspect.lint.recommended` |  |  |

### `validate`

How strictly documents are validated.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `validate.strict_format` | boolean | `false` | Assert `format` keywords. The 2020-12 default is annotation-only, so this catches example drift that annotation-only mode would let through. | `SUSPECT_STRICT_FORMAT` | `suspect.validate.strictFormat` | `suspect validate --strict-format` |  |
| `validate.reference_allowlist` | path |  | File listing the documents `$ref`s may reach; others are reported. |  |  | `suspect validate --reference-allowlist` |  |

### `format`

Formatter output.

Older spelling: `formatting`.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `format.json` | boolean | `false` | Emit JSON regardless of the input extension. |  |  | `suspect fmt --to` |  |
| `format.yaml` | boolean | `false` | Emit YAML regardless of the input extension. |  |  | `suspect fmt --to` |  |
| `format.sort_keys` | boolean | `false` | Reorder object keys into the canonical OpenAPI order when formatting. Defaults to `true` when a `format` section exists, else `false`. |  | `suspect.format.sortKeys` (was `suspect.formatting.sortKeys`) |  | `formatting.sort_keys`, `formatting.sortKeys` |

### `docs`

Reference documentation generation.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `docs.style` | string: `html`, `markdown`, `sveltekit` |  | Output style for the reference site. |  |  | `suspect gen docs --style` |  |
| `docs.output` | path |  | Output root for generated documentation. |  |  | `suspect gen docs --output` | `docs.out` |

### `codegen`

Defaults applied to SDK generation.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `codegen.out` | path |  | Default output root, relative to the file that sets it. |  |  | `suspect gen sdk --output` |  |
| `codegen.profile` | string |  | Default native profile id. |  |  | `suspect gen sdk --profile` |  |
| `codegen.package_name` | string |  | Native package identity, independent of the API title. The editor's SDK commands and Live SDK Preview start from it. |  | `suspect.codegen.packageName` (was `suspect.sdk.packageName`) |  |  |
| `codegen.package_version` | string |  | Default package SemVer, independent of `info.version`. Swift requires a stable version (no prerelease or build metadata). |  | `suspect.codegen.packageVersion` (was `suspect.sdk.packageVersion`) | `suspect gen sdk --package-version` |  |
| `codegen.import_names` | map of string | `{}` | SDK profile to native import/module/namespace name, for example `{python-http: widgets_sdk}`. A profile without an entry derives its name from the package name. |  | `suspect.codegen.importNames` (was `suspect.sdk.importNames`) |  |  |
| `codegen.operation_ids` | list of strings | `[]` | Exact `operationId`s to generate. Empty attempts every operation; an unsupported one fails rather than being dropped. |  | `suspect.codegen.operationIds` (was `suspect.sdk.operationIds`) |  |  |
| `codegen.compatibility_profiles` | list of names: `legacy-binary-string-v1`, `oas30-nullable-in-3.1-v1`, `colon-path-parameters-v1`, `schemaless-stream-events-v1`, `all-of-scoped-mode-v1` | `[]` | Versioned source interpretations to opt into (`suspect gen sdk --compatibility-profile`). Empty keeps ordinary OpenAPI semantics. |  | `suspect.codegen.compatibilityProfiles` (was `suspect.sdk.compatibilityProfiles`) |  |  |
| `codegen.session.config` | path |  | Session config (JSON) naming the spec and targets. Unset: the editor asks for one. |  | `suspect.codegen.session.config` (was `suspect.sdk.sessionConfig`) |  |  |
| `codegen.session.out` | string | `sdk-out` | Initial output root for session preview/watch/drift checks, relative to the session config's directory (not `codegen.out`). |  | `suspect.codegen.session.out` (was `suspect.sdk.sessionOutput`) |  |  |

### `ref`

Reference-workspace tuning.

Older spelling: `refs`.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `ref.max_docs` | integer | `500` | Maximum documents loaded in one ref workspace. |  | `suspect.ref.maxDocs` |  | `ref.maxDocs`, `refs.max_docs`, `refs.maxDocs` |

### `editor`

Editor-only behaviour.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `editor.inlay_hints.refs` | boolean | `true` | Show resolved `$ref` target inlay hints. |  | `suspect.editor.inlayHints.refs` (was `suspect.inlayHints.refs`) |  | `inlay_hints.refs`, `inlay_hints.ref_targets`, `editor.inlay_hints.ref_targets` |
| `editor.inlay_hints.properties` | boolean | `true` | Show property type-annotation inlay hints. |  | `suspect.editor.inlayHints.properties` (was `suspect.inlayHints.properties`) |  | `inlay_hints.properties` |

### `test`

Contract tests, fuzzing and load runs.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `test.base_url` | URL | `http://localhost:8080` | Base URL requests are sent to. | `SUSPECT_BASE_URL` | `suspect.test.baseUrl` (was `suspect.testBaseUrl`) | `suspect test run --base-url`, `suspect test fuzz --base-url`, `suspect test stateful --base-url`, `suspect test load --base-url` | `run.base_url` |
| `test.credentials` | path |  | Credentials file for the security schemes tests exercise. Default: `.suspect/credentials.json` discovered upward. | `SUSPECT_CREDENTIALS` |  | `suspect test run --credentials`, `suspect auth check --credentials` |  |
| `test.report` | string: `text`, `ndjson` | `text` | Event stream format for `suspect test`. |  |  | `suspect test run --format` |  |

### `gateway`

The mock/proxy gateway.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `gateway.port` | port | `8080` | Port the gateway listens on. |  | `suspect.gateway.port` (was `suspect.gatewayPort`) | `suspect traffic gateway --port` |  |
| `gateway.mode` | string: `mock`, `proxy`, `validate`, `record`, `replay`, `scenario` | `mock` | Operating mode. |  |  | `suspect traffic gateway --mode` |  |
| `gateway.upstream` | URL |  | Upstream base URL for proxy, validate and record modes. |  |  | `suspect traffic gateway --upstream` |  |

### `diff`

Change detection against git history.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `diff.baseline` | string |  | Git ref compared against for breaking changes (`suspect ci`, the editor's breaking-changes command; the editor falls back to `HEAD~1`). | `SUSPECT_GIT_BASE` | `suspect.diff.baseline` (was `suspect.ci.baseline`) | `suspect ci --baseline` |  |

### `check`

The `suspect check` gate.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `check.baseline` | path |  | Findings baseline file: findings recorded there do not fail `check`, `validate`, `lint` or the `ci` check stage. Write one with `suspect check --write-baseline`. The editor ignores it. |  |  | `suspect check --baseline` |  |

### `rules`

TypeScript rule plugins.

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `rules.enabled` | boolean | `false` | Run TypeScript rule plugins in the editor and `check`. |  |  |  |  |
| `rules.roots` | list of strings | `[".suspect/rules"]` | Directories searched for rule plugins, relative to the file that sets them. |  |  |  |  |

### `extensions`

| Key | Type | Default | Description | Env | Editor setting | Flags | Older spellings |
|---|---|---|---|---|---|---|---|
| `extensions` | section |  | Vendor extensions: ordering anchors (`before`, `after`), `contexts` and a JSON Schema (`schema`, inline or a path) per `x-` key. |  |  |  |  |

## `suspect.project.json`

Project manifest: the published spec, its profiles, SDK targets, documentation and contract tests.

Settings that moved to `.suspect.yaml` (`lint`, `validate`, `editor`) are still read here for one release, with a warning; `suspect config migrate` moves them.

| Field | Type | Default | Description |
|---|---|---|---|
| `version` | integer |  | Manifest schema version. Must be 1. |
| `name` | string | the directory name | Project name. |
| `entry` | path |  | Entry OpenAPI document, relative to the manifest directory. |
| `config` | path |  | The settings file for this project (`.suspect.yaml`), relative to the manifest. Lint, validate and editor settings live there. |
| `overlays` | list of strings | `[]` | Overlay documents applied in order to produce the published spec. |
| `publish` | section |  | Where the published (post-overlay) spec is written. |
| `publish.output` | path | `build/spec.yaml` | Where the published spec is written, relative to the manifest. |
| `publish.profiles` | map of list of strings | `{}` | Named publication profiles: each is an extra overlay list applied on top of the published spec (for example `public` strips internals). |
| `contract` | section |  | The contract package for this project. |
| `contract.output` | path |  | Where the contract package is written. |
| `lint` | section |  | Deprecated: lint policy belongs in `.suspect.yaml`. Still applied under that file for one release; `suspect config migrate` moves it. |
| `lint.min_severity` | string: `error`, `warning`, `info`, `hint` |  | Deprecated; see `lint.min_severity` in `.suspect.yaml`. |
| `lint.rules` | map of string: `error`, `warning`, `info`, `hint`, `off` |  | Deprecated; see `lint.rules` in `.suspect.yaml`. |
| `lint.design` | string: `error`, `warning`, `info`, `hint`, `off` |  | Deprecated; see `lint.design` in `.suspect.yaml`. |
| `lint.ruleset` | path |  | Deprecated; see `lint.ruleset` in `.suspect.yaml`. |
| `lint.recommended` | boolean |  | Deprecated; see `lint.recommended` in `.suspect.yaml`. |
| `validate` | section |  | Deprecated: validation settings belong in `.suspect.yaml`. |
| `validate.strict_format` | boolean |  | Deprecated; see `validate.strict_format` in `.suspect.yaml`. |
| `editor` | section |  | Deprecated: editor settings belong in `.suspect.yaml` (`editor`, `ref`, `format`). |
| `editor.inlay_hints` | section |  | Deprecated; see `editor.inlay_hints`. |
| `editor.inlay_hints.refs` | boolean |  | Deprecated; see `editor.inlay_hints.refs`. |
| `editor.inlay_hints.properties` | boolean |  | Deprecated; see `editor.inlay_hints.properties`. |
| `editor.ref` | section |  | Deprecated; see `ref`. |
| `editor.ref.max_docs` | integer |  | Deprecated; see `ref.max_docs`. |
| `editor.formatting` | section |  | Deprecated; see `format`. |
| `editor.formatting.sort_keys` | boolean |  | Deprecated; see `format.sort_keys`. |
| `docs` | section |  | Documentation target. |
| `docs.style` | string: `html`, `markdown`, `sveltekit` |  | Output style. |
| `docs.output` | path |  | Output root for generated documentation. |
| `codegen` | list of SDK targets | `[]` | SDK generation targets built from the published spec. |
| `release` | section |  | SDK release: the version every target carries and how each is published (`release publish`, `release workflow`). Replaces a standalone `release.json`. |
| `tests` | section |  | Contract-test targets: Arazzo documents run against `base_url`, or offline from `cassette`. |
| `tests.arazzo` | list of strings | `[]` | Arazzo documents to compile and run. |
| `tests.base_url` | URL |  | Base URL for this project's suites. `SUSPECT_BASE_URL` overrides it; it overrides `test.base_url` in `.suspect.yaml`. |
| `tests.cassette` | path |  | Run offline against this cassette instead of live HTTP. |
| `tests.message_broker` | path |  | Message broker directory for Arazzo 1.1 AsyncAPI steps. |
| `tests.credentials` | path |  | Credentials file for the security schemes the suites exercise. |
| `tests.inputs` | section |  | Workflow inputs supplied to every declared suite: name to value. |

### `codegen` targets

Each entry of the manifest's `codegen` list.

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | the profile id | Target name, used in progress output. |
| `profile` | string |  | The native profile id (`typescript-http`, `python-http`, ...). |
| `package_name` | string |  | Native package identity. |
| `package_version` | string |  | Package SemVer. |
| `out` | path | `sdk` | Output root, relative to the manifest. |
| `operation_id` | list of strings | `[]` | Exact operationId selectors; empty selects all outgoing operations. |
| `import_name` | string |  | Explicit import/module/namespace identity where the profile needs one. |
| `check` | boolean | `false` | When true, only ownership/drift is checked; nothing is written. |
| `compatibility_profiles` | list of strings |  | Interpretation profiles applied to the source (`--compatibility-profile`). |
| `credential_env` | section |  | Credential environment variable mapping (`gen sdk --defaults` schema). |
| `sdk_defaults` | section |  | SDK defaults: pagination, OAuth lifecycle (`gen sdk --defaults` schema). |
| `options` | section |  | Generation options in the `gen sdk --defaults` schema (`compatibility_profiles`, `credential_env`, `sdk_defaults`). A key may be set here or beside it, not both. |

### `release` fields

| Field | Type | Default | Description |
|---|---|---|---|
| `version` | string |  | The release version every target must carry. |
| `tag` | string |  | The tag that triggers the release workflow; `v` plus the version when unset. |
| `targets` | list of strings |  | SDK publish targets, in manifest order. |

### `release.targets` entries

| Field | Type | Default | Description |
|---|---|---|---|
| `backend` | string |  | Backend id (`typescript`, `python`, ...). |
| `directory` | path |  | Generated package directory, relative to the manifest. |
| `package` | string |  | Native package identity. |
| `registry` | string |  | Registry id (`npm`, `pypi`, `maven-central`, ...). |
| `publish` | string |  | Exact publish command, run in `directory`. |
| `version` | string |  | This target's version, when it differs from the release version. |
| `version_skew_reason` | string |  | Why this target's version differs from the release version. |
| `depends_on` | list of strings |  | Backends that must publish first. |

