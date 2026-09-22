# Real VS Code extension-host SDK gate

This gate installs a locally packaged VSIX into an isolated official VS Code
desktop application. The test driver imports the host's real `vscode` API and
executes the production commands. Playwright attaches to that application's
loopback CDP endpoint to operate real Quick Pick/Input Box/progress controls and
capture the rendered workbench. It does not substitute a `vscode` module or CLI.

## Immutable integration contract

The maintained runner requires explicit immutable inputs. It does not read an
older report, select an implicit CLI, or reuse a historical output/profile path.

```sh
SUSPECT_TEST_BINARY=/absolute/pinned/suspect \
SUSPECT_NATIVE_PINS=/absolute/pins.json \
SUSPECT_NATIVE_OUT=/absolute/fresh-output/lifecycle \
SUSPECT_NATIVE_SCRATCH=/absolute/short/external/parent \
SUSPECT_NATIVE_SCENARIO=lifecycle \
  node /absolute/repo/editors/vscode/test/native-host/run.cjs
```

Run `commands` with a different absent output directory for the second scenario.
The output parent and scratch parent must exist. Scratch must be external to the
source checkout and short enough for macOS IPC. No argv options are accepted.
Unknown `SUSPECT_NATIVE_*` variables, including the former
`SUSPECT_NATIVE_EVIDENCE`/`SUSPECT_NATIVE_VSIX`, are errors.

The closed `pins.schema.json` requires:

- `format: "suspect.editor.native-host.pins.v1"`.
- `cli.sha256` and exact `cli.expectedProfiles` IDs; `SUSPECT_TEST_BINARY` supplies
  the path. Comparison is set equality, not a minimum count. The full SDK runner
  must require all twelve known IDs; controlled subset tests still require the
  TypeScript/Rust/Python backends used by these fixtures.
- `vscode.executable`, `executableSha256`, `archive`, `archiveSha256`, exact
  `version`, 40-character `commit`, and host-matching `platform`.
- `tools.directory` and SHA-256 `tools.lockSha256` for its `package-lock.json`.
  The directory must contain installed `@vscode/test-electron`, `@vscode/vsce`
  and `playwright-core` matching the lock. Installed tool-tree hashes are also
  captured and checked after the run.
- `extension.sourceDirectory` and `sourceSha256`; optional `extension.vsix` is
  an object with `path` and `sha256`. Every supplied path is absolute and every
  SHA-256 digest is lowercase hex.

Compute the source pin after compiling the actual execution copy:

```sh
node -e 'require(process.argv[1]).sourceIdentity(process.argv[2]).then(x => console.log(x.sha256))' \
  /absolute/repo/editors/vscode/test/native-host/contract.cjs \
  /absolute/repo/editors/vscode
```

`sourceIdentity` hashes the ordered relative-path/hash inventory of
`package.json`, `package-lock.json`, `src/`, `dist/`, and `media/`. The supplied
VSIX must match all runtime files and runtime manifest fields in that source.
When no VSIX is supplied, the runner stages those exact source bytes, installs
production npm dependencies and packages a local VSIX.

For offline execution, provision the app/archive, installed tool directory and
a self-contained VSIX before calling the runner. Install uses the local VSIX
with `--do-not-include-pack-dependencies`. The source-packaging path invokes
`npm ci` with a fresh private cache and needs npm dependency availability; use a
supplied VSIX for the fully pre-provisioned path.

`env_clear` callers must supply `PATH` with Node and standard system utilities
in addition to the explicit inputs. The runner supplies private HOME/XDG/TMP/npm
cache directories and removes foreign VS Code IPC/portable overrides.

## Report and consumer rules

`SUSPECT_NATIVE_OUT/report.json` is the authoritative result, format
`suspect.editor.native-host.run.v2`. It records immutable input/source/tool pins,
actual CLI inventory, resolved executable hashes/argv, VSIX identity, native
observations, exact required check names/claims, screenshot hashes, host exit,
and a dynamically inventoried canonical ownership baseline.

Native observations use `suspect.editor.native-host.observations.v2` and are
retained as `native-report.json`. A raw native observation file is not a complete
runner verdict. `contract.cjs` exports the required ordered check and screenshot
lists. Missing/extra/duplicate/renamed/skipped checks, missing PNG evidence, input
mutation, bad exit or timeout fail the runner.

- Exit **0**: all checks/claims for the requested mode passed.
- Exit **2**: invocation/pin/path/hash/source/tool/inventory input failure.
- Exit **1**: packaging/native execution/assertion/evidence failure.

Rejections before a fresh output is claimed emit a structured stderr error with
format `suspect.editor.native-host.error.v1`; they create no fallback directory.
Failures after claiming output retain a failed report and partial evidence.

Optional `SUSPECT_NATIVE_MODE=selection-probe` performs only a real installed-host
profile-picker selection/cancellation probe. It skips no required checks: it has
its own single named check and `selectionProbeOnly` claim, no baseline generation,
and cannot satisfy either full native scenario. Full acceptance must require
`mode: "run"`, both scenarios, all twelve expected/observed backend IDs, all exact
required checks and every listed claim equal to boolean `true`.

Input guards can be run independently of the native matrices:

```sh
npm --prefix editors/vscode run test:generation:host:inputs
```

The integration handoff and controlled probe are recorded in
`target/sdk-editor-native-host-integration/`. The runner itself has no dependency
on that directory.

## Accepted historical graphical environment

The 2026-09-10 run used macOS arm64 and official **VS Code 1.137.0**:

- Commit: `645f29cc3176500b4b5762ba887cf2a7f0ffdf2c`
- Archive SHA-256: `16ee5cddb1ea19234e1f2516da07d57e07d7cab6ab45a5515dab077656cbc65e`
- `Contents/MacOS/Code` SHA-256: `72f6b27260b64923278468d94a0ac83f0ce32038803e0b6c05072eddf7d046a9`
- Extension host: Electron **42.10.0**, Node **24.18.1**, Chromium **148.0.7778.280**
- Launcher: Node **22.23.1**
- External tools: `@vscode/test-electron@3.1.0`, `@vscode/vsce@3.9.2`,
  `playwright-core@1.63.0`

The app was downloaded from the commit-pinned URL recorded in
`target/sdk-editor-protocol-native-host-01/official-vscode-pin.json`. Its archive
hash matched Microsoft's update metadata, and `codesign --verify --deep --strict`
passed. The archive, extracted app, tool installation/lockfile and all isolated
profiles are retained in the approved external scratch area.

The historical frozen CLI is `target/sdk-editor-protocol-suspect-01`, SHA-256
`26b0269f3926360743c587e19ca1448f33d77a0058c35350b170ed06876758e3`.
It advertises **eight** native profiles. That accepted run showed all eight picker
entries, exercised a TypeScript/Rust watch, and generated a Python package through
the actual UI. The earlier twelve-profile mocked picker matrix remains separate
evidence.

## Preserved scenario behavior

Both maintained scenarios create fresh user-data/extensions/home/workspace
directories, stage or copy a matching VSIX, and install it using the official
Code CLI. The development extension is a separate minimal test driver;
production code is loaded from the installed VSIX and all runtime-file hashes
are checked.

The baseline SDK is written by the canonical CLI with an explicit owner.
The native test changes source/config files and compares generated disk files by
hash, size, mtime and inode. The cancellation fixture is a regular OpenAPI entry
with a blocked FIFO **reference**, which keeps the real CLI in reference IO long
enough to click the actual Cancel button. The FIFO entry-file refusal from the
earlier attempt is retained as failed evidence.

## Accepted historical coverage and evidence

Successful lifecycle run: **15 checks**, exit 0,
`target/sdk-editor-protocol-native-host-01/attempt-tQ24xA/`.

- Installed VSIX activation, registered production commands, completed LSP command
  registration, exact `suspect lsp` argv and one stable LSP process.
- Actual discovery picker entries, picker cancellation, package identity and
  operation-selector dialogs, import override, generation and opened README.
- Native `TabInputTextDiff` and provider-backed `TextDocument` objects with lexical
  config/source/output URI identities, including an entry symlink.
- Real typing refusal in the generated side; both documents remain clean.
- One canonical watch PID through changed/reverted/removed/error/recovered
  snapshots, with actual native document-change events.
- Stop invalidation, output-dialog cancellation, native progress cancellation,
  and final watch termination when the extension host shuts down.

Successful focused command run: **7 checks**, exit 0,
`target/sdk-editor-protocol-native-host-01/attempt-ddXZy3/`. Three are startup
prerequisites shared with the lifecycle run; four cover the additional registered
Preview/Show Latest/Check actions. The current/drift checks show their native
status/output and open no diff.

Both runs preserved all **63** baseline owned files, including the ownership
manifest. They retained **14 rendered screenshots** in total. `result.json` in
the evidence root indexes the results, source/test hashes, VSIX hash, executable
pins and tool integrities. Each attempt includes commands, launch arguments,
native reports, input fixtures, virtual-document snapshots, disk manifests,
stdout/stderr, and copied VS Code logs.

Useful screenshots:

- `attempt-tQ24xA/04-native-live-diff-B.png` — real A/B diff and typing refusal.
- `attempt-tQ24xA/08-native-progress-cancellation.png` — real Cancel button.
- `attempt-tQ24xA/09-native-generated-python-readme.png` — selected package README.
- `attempt-ddXZy3/commands-03-drift-check.png` — native drift report and roots.

## Remaining coverage boundaries

Native OS file chooser selection, multi-root folder selection, remote workspaces,
and Windows/Linux desktop hosts remain outside these two macOS runs. Multi-root
behavior retains its earlier mocked evidence. The host logged configuration-scope
warnings for resource-qualified reads of window-scoped SDK settings; native
per-folder override behavior is not claimed here.

The retained logs include Node/VS Code deprecation messages and late provider/
closed-channel messages after the host's terminate notification. Live-flow
assertions, explicit Stop invalidation, clean process exit and watch termination
passed; these results do not claim a warning-free VS Code log.
