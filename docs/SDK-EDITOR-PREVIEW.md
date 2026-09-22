# SDK editor preview and watch

The VS Code extension uses the canonical `suspect codegen-session` command for
SDK previews, saved-file watching, and disk drift checks. These actions use the
same config and ownership checks as the CLI.

## Generate a single package

**Suspect: Generate SDK, Docs or Custom Templates** first runs
`suspect codegen-profiles --format json` against the selected CLI. Its validated
`suspect.sdk.profiles.v1` inventory supplies the SDK choices and descriptions;
Markdown documentation and custom template manifests are also available. The
editor recognizes these twelve profiles and displays those compiled into that
CLI:

| Picker profile | CLI profile | Explicit package identity |
| --- | --- | --- |
| TypeScript / JavaScript HTTP SDK | `typescript-http` | npm package name |
| Rust HTTP SDK | `rust-http` | Cargo package name |
| Python HTTP SDK | `python-http` | Python distribution name; the default import package replaces `-` with `_` |
| Go HTTP SDK | `go-http` | Go module path, such as `example.com/team/sdk`; the generated package is named `sdk` |
| Swift HTTP SDK | `swift-http` | Swift package identifier, such as `ExampleSDK`; the default importable module has the same name |
| Ruby HTTP SDK | `ruby-http` | Gem name; the require path replaces `-` and `.` with `_` |
| C# HTTP SDK | `csharp-http` | NuGet package ID, such as `Example.Widgets` |
| Java HTTP SDK | `java-http` | Maven `group:artifact` coordinates |
| Kotlin HTTP SDK | `kotlin-http` | Maven `group:artifact` coordinates |
| PHP HTTP SDK | `php-http` | Composer `vendor/package` name |
| Dart HTTP SDK | `dart-http` | Lowercase pub package name |
| C++ HTTP SDK | `cpp-http` | CMake target/include identifier |

The verified default CLI inventory includes TypeScript, Rust, Python, Go, Swift,
Ruby, C#, and Java. Additional profiles appear when advertised by a matching CLI
build. The action uses the same executable for discovery and generation, even
if the binary setting changes while a picker is open.

The picker prompts for package identity, exact package SemVer, and operation IDs
as a JSON string array. It invokes `suspect codegen SPEC --profile PROFILE` with
`--package-name`, `--package-version`, repeated exact `--operation-id` arguments,
and JSON diagnostics. Output is under `gen-out/PROFILE/LANGUAGE/` in the selected
spec's workspace folder, and the action opens that package's `README.md`.
`suspect.sdk.packageName`, `packageVersion`, and `operationIds` supply initial
prompt values.

The **Markdown documentation** choice invokes `suspect gen SPEC --preset docs-md`.
The **Custom template manifest** choice asks for a TOML manifest and invokes
`suspect gen SPEC --manifest FILE`. Both write under `gen-out/CHOICE/` in the
selected spec's workspace and open a produced document. SDK choices use the
required `codegen --profile` interface with explicit package identity.

`suspect.sdk.importNames` supplies per-profile native import/module/namespace
overrides through the single-package CLI's `--import-name` option. For example:

```json
{
  "suspect.sdk.importNames": {
    "python-http": "custom_widgets_sdk",
    "swift-http": "WidgetsClient",
    "ruby-http": "WidgetsSDK",
    "csharp-http": "Example.Widgets",
    "java-http": "com.example.widgets"
  },
  "suspect.sdk.compatibilityProfiles": []
}
```

Python's prompt describes its default derived import and the override setting.
Go's module path and fixed `sdk` package name are separately identified. Package
and import names are validated by the canonical backend, independently of API
title/version.

Swift package and module names must be valid, non-keyword Swift identifiers.
The CLI validates reserved module names as well. Swift requires exact stable
SemVer without prerelease or build metadata. The picker shows the default module
name; `sdk.importNames["swift-http"]` or a session target's `import_name` supplies
a custom Swift module name.
Canonical Swift output is at `swift/README.md`, `swift/Package.swift`, and
`swift/Sources/MODULE/`. See [SDK-SWIFT.md](SDK-SWIFT.md) for the native SDK gates.

### Explicit compatibility

`suspect.sdk.compatibilityProfiles` defaults to `[]`, preserving ordinary OpenAPI
interpretation. To explicitly interpret legacy OAS 3.1+ `type: string, format:
binary` markers as bytes in binary contexts, set it to
`["legacy-binary-string-v1"]`. Each configured value becomes its own
`--compatibility-profile VALUE` argument alongside any `--import-name` and exact
operation selectors.

The setting uses a closed enum validated by the editor. Unknown or malformed
values produce an error before package prompts or generation. Discovery's
`compatibilityProfiles` catalog and descriptions do not enable any option;
source descriptions do not supply configuration. The CLI records the explicit
selection in its JSON report and validates the selected native capabilities.

## Configure a session

Create a JSON config such as `configs/suspect-sdk.json`:

```json
{
  "spec": "../openapi/api.json",
  "targets": [
    {
      "backend": "typescript-http",
      "package_name": "@example/pet-sdk",
      "package_version": "1.0.0"
    },
    {
      "backend": "python-http",
      "package_name": "pet-sdk",
      "package_version": "1.0.0",
      "import_name": "pet_sdk"
    },
    {
      "backend": "go-http",
      "package_name": "example.com/pet-sdk",
      "package_version": "1.0.0"
    },
    {
      "backend": "swift-http",
      "package_name": "PetSDK",
      "package_version": "1.0.0",
      "import_name": "PetClient"
    },
    {
      "backend": "rust-http",
      "package_name": "pet-sdk",
      "package_version": "1.0.0"
    }
  ],
  "operation_ids": ["getPet"],
  "compatibility_profiles": [],
  "owner": "pet-sdk",
  "cache_entries": 4,
  "cache_bytes": 134217728
}
```

Select the targets needed for your project. Package identity and exact operation
selectors come from this config; API titles, descriptions, and versions do not
supply them. The canonical CLI validates backend capabilities and returns its
source-bound diagnostics for unsupported operations. `operation_ids: []` attempts
all outgoing operations.

Session preview/check/watch pass the saved config to the CLI intact. A session's
top-level `compatibility_profiles` is authoritative; the single-package picker
setting does not override it. Set `compatibility_profiles` to
`["legacy-binary-string-v1"]` for an explicit session opt-in. The CLI includes
these choices in revision/cache identity and reports them as
`compatibilityProfiles`. A saved change can produce a new plan in the same watch
process, and reverting to a cached configuration reuses its earlier plan.

Optional workspace settings:

```json
{
  "suspect.basePath": "/absolute/path/to/suspect",
  "suspect.sdk.sessionConfig": "configs/suspect-sdk.json",
  "suspect.sdk.sessionOutput": "../generated"
}
```

- `sessionConfig` is absolute or relative to the selected workspace folder. When
  empty, the action opens a JSON file picker.
- `spec` is relative to the **config directory**, or absolute. The editor and CLI
  resolve a **lexical absolute retrieval path** without following entry symlinks;
  its parent is the displayed source root and the base of relative references.
  For example, when `selected/api.json` links to `physical/api.json`, its
  `./model.json` reference still resolves under `selected/`, not `physical/`.
  An unreadable or malformed config is reported by the canonical CLI, with the
  source root explicitly unresolved until a successful plan; watch can recover
  after the config is fixed.
- `sessionOutput` is the initial output-directory prompt value, relative to the
  **config directory**, or absolute. Choose the same output root and owner as
  regular generation. The directory can be missing.
- The extension passes absolute config/output paths and sets the CLI working
  directory to the config directory. Output reports and virtual document URIs
  retain the config, source, and output identities, including across multi-root
  workspaces and changes to the config's source entry.

## Commands

| Command palette action | Canonical CLI options | Behavior |
| --- | --- | --- |
| **Suspect: Preview SDK Changes** | `--preview --format json` | One read-only plan; choose an artifact to compare with its current disk bytes. |
| **Suspect: Check SDK Drift** | `--check --format json` | One read-only ownership/drift check, with status and diagnostics in **Suspect SDK** output. |
| **Suspect: Watch SDK Preview** | `--watch --preview --format json` | One persistent session watching saved config/source/reference inputs; opens the first artifact picker and updates the selected diff. |
| **Suspect: Show Latest SDK Preview** | No subprocess | Select another artifact from the latest complete plan, or show the latest diagnostics. Also available by clicking the SDK status item. |
| **Suspect: Stop SDK Watch** | Terminates the active session | Stops watch or an in-flight preview/check and invalidates its virtual documents. |

All three CLI actions also pass `--config FILE --out DIR`. `--preview` implies
read-only checking in the CLI. To write the plan explicitly outside the preview,
use the same config and root without the read-only flags:

```sh
suspect codegen-session --config configs/suspect-sdk.json --out generated --format json
```

The CLI's relative `--out` follows the shell's working directory; the editor
resolves its output prompt against the config directory before spawning it.

### Reading a preview

Both sides of the diff are read-only `suspect-sdk-preview:` text documents. The
left side is a bounded snapshot of the current file **on disk**; a missing file
is empty. The right side is the exact generated text from the CLI. Planned
removals have an empty right side. The picker puts disk drift/ownership changes
first and also exposes unchanged desired files.

The SDK status item shows the latest generation and explicit status (`current`,
`drift`, `write-conflict`, or `planning-error`). **Suspect SDK** output includes
source/output roots, reported compatibility profiles, success,
compile/render/cache-hit deltas and totals, and canonical diagnostics with their
source/pointer/range fields intact. Ownership
conflicts remain visible while generated text is available for comparison.

Watch reads **saved bytes**. Unsaved source or generated-file buffers are not
silently saved or substituted. The CLI emits a record when the revision, status,
drift, diagnostics, or planning activity changes; unchanged polling does not
continually append reports. A cached A→B→A revert repeats A's content revision
with a new generation number and refreshes the diff with zero compiles/renders.
Cache counters may therefore become visible on a later emitted generation.

During watch, one editor file watcher also observes the selected output file.
Its disk-side snapshot refreshes on create/change/delete even when another user
edit leaves the CLI's conflict status and changed-path list unchanged.

## Lifetime and bounded data

- Profile discovery has a **5-second** deadline and **256 KiB** stdout/stderr
  buffers. A stuck process is terminated with SIGKILL at the deadline, including
  one that ignores SIGTERM. Invalid/oversized output, duplicate or unknown
  profiles, and mismatched native directories fail before a menu is offered.
  Descriptions are bounded plain text; they never become commands or paths.
- Starting another SDK action terminates and awaits the prior CLI process. Rapid
  replacement requests cannot launch competing sessions. Cancellation of a
  preview/check notification, the Stop command, extension deactivation/disposal,
  or relevant extension-setting changes terminate the process. Termination uses
  SIGTERM, escalating to SIGKILL after one second if necessary.
- Only the latest artifact set and one open diff pair are retained. The selected
  diff updates in place during watch. A replaced source identity, obsolete
  artifact, planning/transport failure, cancellation, or stop invalidates prior
  virtual documents. A picker that outlives a generation uses the latest plan
  rather than its old captured contents.
- Stream records and individual current-file snapshots are limited to **16 MiB**;
  config reads to **4 MiB**; retained stderr to **8 KiB**; the latest output report
  to **64 KiB**. Path/artifact/diagnostic collections are limited to **4,096**
  entries each. Oversized or malformed protocol output terminates the process
  with an explicit error instead of presenting a truncated SDK.
- Artifact paths must be portable, relative paths. Traversal, command/file URIs,
  duplicate artifact names, and symlink descendants under the selected output
  root are refused. Disk snapshots must be regular UTF-8 text files.
- The executable and arguments are passed directly to `spawn` with `shell:
  false`. Generated files and source prose are text, displayed through a content
  provider and `vscode.diff`; they are never evaluated or run by the preview.

## Protocol and verification

`editors/vscode/src/generation.ts` consumes `suspect.sdk.session.v1`. Watch stdout
is newline-delimited JSON. One-shot output may be compact or pretty JSON. Every
record must carry `success`, an explicit `status`, increasing `generation`,
`changedArtifacts`, `newDocuments`, `delta`, `stats`, and `diagnostics`.
`delta`/`stats` use `compiles`, `renders`, and `cache_hits`. Preview records contain
the **complete** `artifacts: [{path, content}]` set unless planning failed. The
CLI's `config`, `source`, and `output` fields retain the requested lexical path
identities. An initial planning error can explicitly report `source: null`.
The opaque `revision` fingerprint identifies the accepted source/configuration
snapshot and is shown in the output report. Repeated revisions are accepted when
`generation` advances. The helper also accepts `written` for an explicitly
writable CLI session.

When supplied, `compatibilityProfiles` must be an array of known versioned
choices, including the empty array for ordinary semantics. The editor retains
and displays it. Session diagnostics use nested
`source: {document, pointer}` and byte `range: {start, end}`; single-package
`codegen` diagnostics also include `file`, `pointer`, `line`, and `col`. These
canonical fields are retained without translating their locations. A
single-package report for an unnamed operation retains `operationId: null`
alongside its source method/path.

Local checks use pinned Node **22.23.1**:

```sh
npm --prefix editors/vscode run compile
npm --prefix editors/vscode run test:generation:protocol
```

The protocol tests exercise real byte streams at the spawn seam, including
Unicode fragmentation, incompatible/bounded output, exact argv, cancellation,
and termination escalation. `test/generation-discovery.cjs` also uses real
`execFile` child processes to verify UTF-8 inventory handling, buffer limits, and
termination of a SIGTERM-ignoring CLI. The UI-seam tests exercise the actual
registered commands and content provider with mocked VS Code/process APIs and
real temp files: all twelve native-profile picker routes, full/reduced/empty
inventories, malformed and malicious discovery, explicit import/compatibility
options, live diffs, removals/errors/recovery, late pickers/results, lexical
symlink identity, cached reverts, overlapping requests, cancellation/disposal,
and bounded diagnostics.

The existing real-CLI TypeScript/Rust generation regressions are retained in
`test/generation.cjs`. `test/generation-native-profiles.cjs` checks Python/Go
package and import/module identities, source diagnostics, drift/ownership, and a
custom Python session import name. It also checks Swift CLI emission and a
read-only custom-module preview by inspecting package artifacts. Expanded
anonymous/204 operations are positive generation cases; a genuinely undeclared
security scheme supplies negative source-location and no-overwrite assertions.
`test/generation-profiles.cjs` checks an expanded read-only plan for every
advertised native profile and Ruby generation with an explicit namespace.
`test/generation-compatibility.cjs` verifies explicit TypeScript/Python binary
compatibility through real argv and reports, default refusal without writes,
custom import identity, unnamed operation provenance, and saved session-option
revisions with invalid-config recovery and cached reverts.
`test/generation-native-session.cjs` exercises
read-only preview/check, warm ownership drift, source edits, errors/recovery,
symlink-relative references with distinct physical/lexical neighbors, cached
A→B→A reverts, and stop against the actual session CLI. Run these with a freshly
built matching binary and Node 22.23.1 on `PATH`:

```sh
SUSPECT_TEST_BINARY=/absolute/path/to/suspect npm --prefix editors/vscode run test:generation
```

The earlier **52-check** integration and **49-check** protocol/UI runs remain
historical evidence. The accepted process/mocked-UI verification (2026-09-10),
with Node **22.23.1**,
passed compilation, **101 protocol/UI checks**, and **122 full checks** with
zero failures or skips. The full run covers all eight profiles advertised by
the fresh default CLI, including Ruby, C#, and Java; UI-seam coverage exercises
all twelve recognized profiles. The fixtures inspect generated packages without
executing their SDK source.

Fresh local evidence:

- `target/sdk-editor-protocol-compile-01.log`
- `target/sdk-editor-protocol-mocked-01.log`
- `target/sdk-editor-protocol-full-02.log`
- `target/sdk-editor-protocol-inventory-01.json`
- `target/sdk-editor-protocol-cli-identity-01.json` records the verified CLI
  copy and SHA-256; the copy is `target/sdk-editor-protocol-suspect-01`.

The fresh binary came from the successful public CLI test build recorded in
`target/sdk-cli-options-integration-01.log`. Reproduction records for the fixed
discovery deadline/binary-selection bugs and earlier failures are retained under
`target/sdk-editor-protocol-*`.

### Graphical VSIX / extension-host verification

The real graphical gate passed on **VS Code 1.137.0**, official macOS arm64 build
`645f29cc3176500b4b5762ba887cf2a7f0ffdf2c`, using a locally packaged and installed
VSIX in isolated user-data/extensions/home/workspace directories. The archive
SHA-256 was
`16ee5cddb1ea19234e1f2516da07d57e07d7cab6ab45a5515dab077656cbc65e`;
the app's code signature also verified. The host used Electron **42.10.0** and
Node **24.18.1**. The launcher/tools used Node **22.23.1**.

The test runner imports the real `vscode` API, invokes production registered
commands, reads actual provider-backed documents and diff tabs, and observes
the real CLI child processes. Renderer automation operates actual Quick Picks,
Input Boxes and the progress Cancel button, and captures the native Electron
workbench through its loopback CDP endpoint.

- **15 lifecycle checks passed**, exit 0: installed activation and LSP startup;
  the eight-profile discovery picker; a real read-only diff with lexical roots;
  changed/reverted/removed/error/recovery snapshots through one canonical watch
  PID; typing refusal; Stop and dialog/progress cancellation; Python generation
  and its opened README; and final watch shutdown with the host.
- **7 focused command checks passed**, exit 0: three startup prerequisites and
  successful registered one-shot Preview, Show Latest, current Check and drift
  Check actions. Both checks opened no generated diff.
- All **63 baseline owned files** retained their bytes, size, mtime and inode in
  each run. Fourteen rendered screenshots are retained. The frozen CLI still
  advertises eight profiles; native host evidence does not expand that inventory.

Native startup reproduced and fixed three editor defects:

1. An existing executable with a versioned/renamed filename was treated as a
   directory by `suspect.basePath`. Existing regular executable files now retain
   their selected path.
2. Explicit `TransportKind.stdio` made the language client append `--stdio`,
   which `suspect lsp` does not accept. Executable-default stdio uses exact argv.
3. The LSP and editor both registered `suspect.runWorkflow`, aborting client
   initialization. The editor owns that handler, accepts code-lens URI strings,
   and allows the other server commands to register normally.

Evidence root: `target/sdk-editor-protocol-native-host-01/`.

- `result.json` — exact app/CLI/tool/VSIX/source/test pins and result index.
- `native-run-05.log`, `attempt-tQ24xA/native-report.json` — lifecycle results.
- `native-commands-02.log`, `attempt-ddXZy3/native-report.json` — focused command
  results.
- `attempt-tQ24xA/04-native-live-diff-B.png` — rendered A/B diff and read-only
  typing refusal.
- `attempt-tQ24xA/08-native-progress-cancellation.png` — actual progress Cancel
  control.
- `attempt-ddXZy3/commands-03-drift-check.png` — rendered native drift report.
- `editor-compile-03.log` — compilation after the native startup fixes.

Earlier attempts, including the macOS IPC-path limit, executable-resolution and
LSP startup failures, are retained. The accepted **122** process/mocked-UI and
**101** protocol/UI matrices were preserved without rerunning them during this
native follow-up. See `editors/vscode/test/native-host/README.md` for exact commands,
tool pins and fixture details.

Remaining native coverage: OS file chooser selection, multi-root folder
selection, remote workspaces, and Windows/Linux hosts. The macOS logs retain
configuration-scope warnings and late teardown messages after the host's
terminate notification; live Stop invalidation, process cleanup, disk preservation
and exit-code checks passed. The native results make no warning-free-log claim.

### Maintained native-host invocation contract

The reusable gate now takes explicit immutable inputs rather than consulting the
historical evidence above. `editors/vscode/test/native-host/pins.schema.json`
defines `suspect.editor.native-host.pins.v1`: CLI hash and exact expected backend
IDs, VS Code executable/archive hashes and version/commit, installed tool directory
and lock hash, editor source directory/digest, and an optional supplied VSIX/hash.

Call `node editors/vscode/test/native-host/run.cjs` with all of:

- `SUSPECT_TEST_BINARY` — explicit absolute CLI path.
- `SUSPECT_NATIVE_PINS` — absolute pins JSON path.
- `SUSPECT_NATIVE_OUT` — fresh absent output directory, with an existing parent.
- `SUSPECT_NATIVE_SCRATCH` — short external scratch parent.
- `SUSPECT_NATIVE_SCENARIO` — `lifecycle` or `commands`.

The authoritative output is `report.json`, format
`suspect.editor.native-host.run.v2`. Full acceptance requires `mode: "run"`, both
scenarios, the complete exact twelve-backend set, all declared native checks and
claims, verified screenshots, unchanged pinned inputs/owned files, and host exit
0 without timeout. Unknown/incomplete inputs and stale/partial/skipped reports
fail. No old CLI/report/profile is selected implicitly.

For offline execution, provide a prebuilt self-contained VSIX, installed locked
tools and the pinned extracted app plus archive. `env_clear` is supported with an
explicit Node/system `PATH`; profiles and HOME/XDG/TMP paths are isolated by the
runner. Omitting the VSIX selects source packaging and requires npm dependency
availability. See `test/native-host/README.md` for details.

This contract update passed **53 new guard tests** and a single real
`selection-probe` under `env -i`, using explicit fresh copies of the accepted
eight-profile CLI/VSIX. It captured the actual native picker and exited 0 without
SDK baseline generation. That explicit probe mode cannot satisfy either native
acceptance scenario. The completed native and process/mock matrices were not
rerun for this update. Integration instructions and probe evidence are in
`target/sdk-editor-native-host-integration/HANDOFF.md` and
`selection-probe-01/report.json`.
