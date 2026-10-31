# P4. One editor story

Status: proposal. Depends on: P2 (P2.1 `resolve`, P2.3 schema extension,
P2.7 JSON Schema), P5 (release artifacts). Decided: P2 option A (policy
lives only in `.suspect.yaml`, so the manifest's `editor` section goes away
with it); the `sdk-*` readiness codes are a separate category (P1).

Evidence was gathered at `c7e87fb`. LSP probes drove a freshly built
`target/debug/suspect lsp` over stdio with an empty client capability set,
from a throwaway Python script (not checked in).

## Summary

A user sees the same setting names in VS Code, Zed and `.suspect.yaml`. The
editor finds the matching `suspect` binary itself. Unrelated YAML and JSON
files are left alone. In the code:

- the `suspect.*` settings in `editors/vscode/package.json` are generated
  from the P2 JSON Schema, and a test fails when they drift;
- the server advertises its custom requests and their versions in
  `capabilities.experimental.suspect`, and clients declare which commands
  they own instead of patching the language client;
- the server answers a new `suspect/documentKind` request and stops
  formatting and hovering documents that are not specs;
- VS Code ships one VSIX per platform with the binary inside, and Zed
  downloads the matching release asset and checks it.

## Problem

1. **Three spellings, three schemas, one setting.** `lint.min_severity` is
   written `suspect.lint.minSeverity` in VS Code (`package.json:94`), parsed
   by hand in `suspect-config` (`crates/suspect-config/src/lib.rs:36-47`),
   described again in the LSP's hand-written table
   (`crates/suspect-lsp/src/config_schema.rs:129-324`, whose module doc says
   "The tables are hand-authored"), and read from the client by a fourth
   parser (`crates/suspect-lsp/src/editor_config.rs:106-129`). The client
   section is split across two objects: `minSeverity`, `ruleset`, `design`,
   `strictFormat` go to `EditorConfig`, while `rules`, `design`,
   `recommended`, `ref`, `inlayHints` and `formatting` go to `SuspectConfig`
   (`config_files.rs:214-270`). `lint.design` is read by both.
2. **Settings the CLI never reads.** `lint.recommended`, `ref.max_docs`,
   `inlay_hints.*` and `formatting.sort_keys` are documented `.suspect.yaml`
   keys (`config_schema.rs:233-295`) but are not fields of
   `suspect_config::Settings`. Grepping `crates/suspect-cli/src` for
   `recommended`, `sort_keys` and `max_docs` finds only the `project init`
   template (`commands/project.rs:389-395`). So `suspect lint` and the
   editor apply different rule sets when a project sets
   `lint.recommended: false`.
3. **Editor-only settings with server or CLI twins.** See the table in
   [Settings = config](#settings--config). Examples:
   - `suspect.testBaseUrl` is read only by the extension (`runner.ts:91`).
     The server's own workflow runner reads `suspect.run.baseUrl` from
     `initializationOptions`, then `SUSPECT_BASE_URL`, then
     `http://localhost:8080` (`run_lenses.rs:401-434`, `lib.rs:3133`).
     Neither VS Code nor Zed sends `initializationOptions`, so the server
     always uses the env var or the hard-coded default.
   - `suspect.ci.baseline` feeds `suspect ci --baseline`
     (`testing.ts:136`). The server's `suspect.breakingChanges` command
     reads `SUSPECT_GIT_BASE` instead (`lib.rs:2745`). The extension's
     message about it reads `process.env.SUSPECT_GIT_BASE` of the extension
     host (`serverCommands.ts:106`), which is not the server's environment.
   - `suspect.sdk.compatibilityProfiles` allows one value
     (`package.json:27-29`). `suspect codegen --compatibility-profile`
     accepts five (`docs/CLI.md:801`).
4. **Untyped settings reach the wire.** `liveSdk.ts:180` reads
   `sdk.importNames` (an object, default `{}`) as a string and sends it as
   `importName`. `{}` is truthy, so every render sends an object. The probe
   reproduced this with the same parameters:

   ```text
   suspect/renderSdk {"importName": {}, ...}
   -> {"error": {"code": -32602, "message": "invalid type: map, expected a string"}}
   ```

   Live SDK Preview therefore fails at its first render under default
   settings. This is a defect, not a design question. It is listed as step
   0 below because generated, typed settings stop this class of bug.
5. **Command ownership is a monkey-patch.** The server advertises 15
   `workspace/executeCommand` ids (`lib.rs:802-822`). The extension
   registers palette wrappers for 11 of them under the same ids
   (`serverCommands.ts:56-83`). vscode-languageclient throws on a duplicate
   registration, so `extension.ts:121-126` replaces
   `commands.register` on the client's `workspace/executeCommand` feature to
   filter those 11 ids out. The overlap is between those wrappers and the
   server, not between client-only commands and the server. The 15
   client-only commands (`startGateway`, `genPreset`, `previewSdk`, …) have
   no server twin.
6. **No capability advertisement.** The probe's `initialize` result has
   `experimental: None`. The three custom requests (`suspect/runWorkflow`,
   `suspect/generationContract`, `suspect/renderSdk`, registered at
   `lib.rs:3283-3296`) are found by trying them. `suspect/runWorkflow`
   returns an `InternalError` when the server was built without the
   `live-run` feature (`lib.rs:3125-3129`), and a client cannot know that in
   advance.
7. **The version check is exact and VS Code-only.** `extension.ts:146-160`
   warns when `serverInfo.version` differs from the extension's version as a
   string. A patch release warns. Zed cannot see `serverInfo` and does not
   check.
8. **The server acts on files that are not specs.** The server already
   sniffs the document family (`suspect_low::LowDoc::sniff_family`,
   `crates/suspect-low/src/doc.rs:103-149`). Diagnostics, code lenses,
   inlay hints and semantic tokens are empty for `SpecFamily::Unknown`. But
   formatting and hover still answer. The probe opened `docker-compose.yml`
   and `package.json`:

   ```text
   publishDiagnostics docker-compose.yml []        package.json []
   codeLens / inlayHint / semanticTokens           0 / 0 / 0 for both
   hover docker-compose.yml (1:3)  -> "### `web`\n\n```yaml\nimage: nginx ..."
   formatting package.json         -> 1 edit, rewrites the whole file
   ```

   Zed attaches to every YAML and JSON buffer (`editors/zed/extension.toml`
   `languages = ["YAML", "JSON"]`). VS Code selects every `file` YAML/JSON
   document (`extension.ts:100-103`) and activates on any JSON file
   (`package.json:12-13`). The editor-context menu shows Generate Example,
   Extract and Inline on every file (`package.json:364-380`,
   `when: editorTextFocus && resourceScheme == file`).
9. **No binary acquisition.** VS Code resolves `suspect.basePath`, the
   deprecated `suspect.server.path`, then bare `suspect` on `PATH`
   (`runner.ts:59-89`). On `ENOENT` it shows "Install it or set
   suspect.basePath" (`extension.ts:132-135`). Zed resolves
   `lsp.suspect-lsp.binary.path`, then `Worktree::which("suspect")`
   (`editors/zed/src/lib.rs:52-90`). Neither can fetch a binary.
10. **Nothing is publishable.** `package.json` has `"private": true`, no
    `repository`, `license` or `icon`, no `.vscodeignore`, and no bundler
    (`dist/` is plain `tsc` output that `require`s `node_modules`).
    `editors/zed` has no licence file and no `repository` in
    `extension.toml`. No workflow packages either (`.github/workflows/`
    has `ci`, `docs`, `platform`, `application-targets`,
    `native-evidence`, `sdk-session-performance`).
11. **Client-side config parsing.** `config.ts:30-36` works out the
    effective severity floor for the Set Minimum Severity picker with a
    regex over `.suspect.yaml` (`/^\s*min_severity:\s*(\w+)/m`). That is a
    fifth parser. It ignores nesting, `.suspect.yml`, and parent
    directories.

## Goals and non-goals

Goals:

- One key per setting. Each editor setting maps mechanically onto one
  config key, and the mapping is generated and tested.
- One owner per command id. The ownership is declared in the protocol, not
  patched in.
- Clients check for features instead of guessing, and every client gets the
  version check.
- Editors find a matching binary without user action on the five P5
  platforms.
- The server leaves non-spec YAML/JSON alone in every client.
- The VS Code extension is published to the Marketplace and Open VSX, and
  the Zed extension to the Zed registry, from the release workflow.

Non-goals:

- New editor features: webviews in Zed, Neovim or Helix plugins.
- Changing the server's LSP feature set beyond gating by document kind.
- Designing the config schema itself (P2) or the release workflow (P5).
  This proposal states what it needs from both.
- Auto-updating a binary the user installed themselves.

## Design

### Settings = config

**Rule.** Every config key that changes editor behaviour has exactly one
editor setting: `suspect.` plus the config path with each segment in
camelCase. `lint.min_severity` becomes `suspect.lint.minSeverity`, and
`inlay_hints.refs` becomes `suspect.inlayHints.refs`. The short version
calls these "identical keys". They are not identical: the mapping is a case
conversion, which keeps the existing VS Code names and VS Code's camelCase
convention. The server accepts both spellings from any client (Zed users
already write camelCase, `editors/zed/README.md`).

**Today and target.** Every setting in `package.json`, what the server does
with it today, and the config key it maps to:

| Editor setting (default) | Server field today | Config key today | Target config key |
|---|---|---|---|
| `suspect.lint.minSeverity` (`null`) | `EditorConfig.settings.lint.min_severity` (`editor_config.rs:109`) | `lint.min_severity` | `lint.min_severity` |
| `suspect.lint.ruleset` (`null`) | `EditorConfig.settings.lint.ruleset`, resolved against the config root (`editor_config.rs:112-118`) | `lint.ruleset` | `lint.ruleset` |
| `suspect.lint.design` (`null`) | both `EditorConfig.settings.lint.design` (`:119`) and `SuspectConfig.lint.design` (`config_files.rs:227`) | `lint.design` | `lint.design` |
| `suspect.lint.rules` (`null`) | `SuspectConfig.lint.rules` only (`config_files.rs:218`) | `lint.rules` | `lint.rules` |
| `suspect.lint.recommended` (`null`) | `SuspectConfig.lint.recommended` (`config_files.rs:231`) | none in `suspect-config`; LSP table only | `lint.recommended` (P2.3) |
| `suspect.validate.strictFormat` (`null`) | `EditorConfig.settings.validate.strict_format` (`editor_config.rs:122`) | `validate.strict_format` | `validate.strict_format` |
| `suspect.ref.maxDocs` (`null`) | `SuspectConfig.refs.max_docs` (`config_files.rs:237`) | none in `suspect-config`; LSP table only | `ref.max_docs` (P2.3) |
| `suspect.inlayHints.refs` (`null`) | `SuspectConfig.inlay_hints.ref_targets` (`config_files.rs:247`) | none in `suspect-config` | `editor.inlay_hints.refs` (P2.3 `editor`) |
| `suspect.inlayHints.properties` (`null`) | `SuspectConfig.inlay_hints.properties` (`:253`) | none in `suspect-config` | `editor.inlay_hints.properties` |
| `suspect.formatting.sortKeys` (`null`) | `SuspectConfig.formatting.sort_keys` (`:261`) | none in `suspect-config` | `format.sort_keys` (joins the existing `format` section, so `suspect fmt` reads it too) |
| `suspect.testBaseUrl` (`http://localhost:8080`) | none; server uses init option `suspect.run.baseUrl` or `SUSPECT_BASE_URL` | none | `test.base_url` (P2.3), env `SUSPECT_BASE_URL` |
| `suspect.gatewayPort` (`8080`) | none | none | `gateway.port` (P2.3) |
| `suspect.ci.baseline` (`""`) | none; `suspect.breakingChanges` reads `SUSPECT_GIT_BASE` | none | `ci.baseline` (new; P2 must add a `ci` section), env `SUSPECT_GIT_BASE` |
| `suspect.sdk.packageVersion` (`""`) | none | `codegen.package_version` exists but is not wired to the setting | `codegen.package_version` |
| `suspect.sdk.sessionOutput` (`sdk-out`) | none | `codegen.out` exists, default `codegen-out` | `codegen.out` |
| `suspect.sdk.packageName` (`""`) | none | none (CLI `--package-name`) | `codegen.package_name` |
| `suspect.sdk.importNames` (`{}`) | none | none (CLI `--import-name`) | `codegen.import_names` (profile to name) |
| `suspect.sdk.operationIds` (`[]`) | none | none (CLI `--operation-id`) | `codegen.operation_ids` |
| `suspect.sdk.compatibilityProfiles` (`[]`) | none | none (CLI `--compatibility-profile`) | `codegen.compatibility_profiles` |
| `suspect.sdk.sessionConfig` (`""`) | none | none (it points at a session config file) | none; editor-only (P2.6 may fold the file format) |
| `suspect.basePath` (`suspect`) | none | none | none; editor-only |
| `suspect.server.path` (`""`, deprecated) | none | none | none; editor-only, removed after two releases |

Config keys the server reads with no editor setting today: `extensions`
(`config_files.rs:257`). Config keys no editor setting needs: `format.json`,
`format.yaml`, `docs.*`, `codegen.profile`. Neither group becomes an editor
setting unless the schema marks it (below).

Moving `codegen.out` changes the default the extension shows (`sdk-out` to
`codegen-out`). The generator takes the default from the schema, so the
two cannot disagree again.

**What P4 needs from the P2 schema.** P2.7 generates a JSON Schema for
`.suspect.yaml` from the Rust types. P4 needs one annotation per property:

```json
"min_severity": {
  "type": "string",
  "enum": ["error", "warning", "info", "hint"],
  "description": "Minimum severity to report.",
  "x-suspect-editor": { "scope": "resource" }
}
```

A property with `x-suspect-editor` becomes an editor setting. `scope` is
VS Code's (`window`, `resource`, `machine-overridable`). An optional
`order` sets the position in the Settings UI. Defaults and enum
descriptions come from the schema. A property without the annotation is not
exposed.

**Generator.** The bless pattern in
`crates/suspect-cli/tests/cli_reference.rs` fits, but `package.json` cannot
be rewritten from Rust without reordering it: the workspace `serde_json`
has no `preserve_order` feature (`Cargo.toml:56`), and turning it on
through a dev-dependency would change map order for every crate's tests.
So the work is split in two, and each half stays in its own toolchain:

1. Rust, a bless test: `crates/suspect-config/tests/json_schema.rs`
   (P2.7) writes `editors/vscode/schemas/suspect.schema.json`. The VS Code
   extension needs that file anyway for `yamlValidation` (P2.7).
2. Node: `editors/vscode/scripts/settings.cjs` (new) reads that schema plus
   `editors/vscode/settings.editor.json` (new, hand-written: `basePath`,
   `server.path`, `sdk.sessionConfig`), and builds
   `contributes.configuration`. `npm run settings:write` rewrites
   `package.json` with `JSON.stringify(manifest, null, '\t')`, which keeps
   key order and the existing tab indentation. `test/settings.cjs` (new,
   added to `test:headless`) fails when the result differs from the
   checked-in `package.json`.

```js
// editors/vscode/scripts/settings.cjs (new), the mapping core
const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
function* exposed(schema, path = []) {
  for (const [key, prop] of Object.entries(schema.properties ?? {})) {
    const next = [...path, key];
    if (prop['x-suspect-editor']) yield [next, prop];
    if (prop.type === 'object' && prop.properties) yield* exposed(prop, next);
  }
}
function contribute(schema, editorOnly) {
  const properties = {};
  for (const [path, prop] of exposed(schema)) {
    properties[`suspect.${path.map(camel).join('.')}`] = {
      // null means "unset: defer to .suspect.yaml", the minSeverity rule today
      type: [prop.type, 'null'], default: null,
      ...(prop.enum && { enum: [...prop.enum, null] }),
      markdownDescription: `${prop.description} (\`${path.join('.')}\` in \`.suspect.yaml\`)`,
      scope: prop['x-suspect-editor'].scope,
    };
  }
  return { title: 'Suspect', properties: { ...properties, ...editorOnly } };
}
```

Every config-backed setting gets `type: [T, "null"]` and `default: null`.
Today `minSeverity` already does this. The other eight `null`-default
settings declare a non-null type (`package.json:121-191`), so the Settings
UI shows them as set to an invalid value.

The same script emits `src/settings.generated.ts`: one typed getter per
setting (`lintMinSeverity(): 'error' | … | null`). Calls such as
`config.get<string>('sdk.importNames')` (problem 4) then fail to compile.

**CI.** The existing `vscode-extension` job in `platform.yml` already runs
`npm run test:headless`, so the drift test needs no new job. `cargo test`
already covers the Rust bless test. Both failures print the regenerate
command.

**Server side.** With P2.1, `editor_config::apply_client`, the client half
of `config_files::parse_config` and `read_manifest_policy`'s `editor`
unwrapping (`config_files.rs:291-328`) are deleted. The pulled `suspect`
section becomes one layer of `suspect_config::resolve`, converted from
camelCase by the same schema path list:

```rust
// suspect-config (P2.1 owns resolve; P4 adds the editor layer input)
pub struct Inputs<'a> {
    pub flags: Option<&'a serde_json::Value>,
    /// The client's `suspect` settings section, camelCase or snake_case.
    pub editor: Option<&'a serde_json::Value>,
}
pub fn resolve_with(root: &Path, inputs: Inputs<'_>) -> Result<Resolved, ConfigError>;
```

`Backend::reload_config` (`lib.rs:153-178`) calls `resolve_with` once and
stores one `Resolved` in place of `st.config` and `st.editor_config`.

**Precedence.** In the LSP the editor setting is the per-session explicit
choice, which is the role a flag plays in the CLI. So the order is
`editor > env > file > default`. The short version puts editor settings at
"the highest file layer", below env. That would let a `SUSPECT_LINT_*` value
in a shell profile override a floor the user just picked in the editor, so
this proposal puts the editor layer where flags sit. Today the server does
not read `SUSPECT_*` settings variables at all, so either choice is new
behaviour.

**Explain.** `suspect config --explain` (P2.4) cannot see the editor layer,
because that layer exists only inside the server. A new request,
`suspect/explainConfig`, returns the server's own trace, using P2's
`Resolution` trace (`suspect-config/src/resolve.rs:15-21`), for every key:

```json
{ "source": ".suspect.yaml", "error": null,
  "values": [{ "key": "lint.min_severity", "value": "warning",
               "layer": "editor", "trace": [["editor","warning"],["file:.suspect.yaml:3","error"]] }] }
```

A VS Code status-bar item shows `suspect: .suspect.yaml` (or
`suspect: defaults`, or a warning icon when `EditorConfig.error` is set,
`editor_config.rs:25`). Clicking it opens that report. `config.ts`'s regex
floor (problem 11) is replaced by reading the `lint.min_severity` entry
from the report.

### Command ownership

**Rule.** A command id has one owner.

- Server commands (`suspect.*`) are implemented in `Backend::run_command`
  (`lib.rs:2559`). Generic clients (Zed, Neovim) run them directly from
  lenses and code actions.
- Client commands (`suspect.*`, contributed in `package.json`) are what
  users see in the palette and bind keys to.
- A rich client may own the user-facing side of a server command: a picker,
  progress, or a readable result. It does that by declaring the id, not by
  filtering it.

**Negotiation (new).** The client lists the ids it owns in its initialize
capabilities:

```json
"capabilities": { "experimental": { "suspect": {
  "clientCommands": ["suspect.runWorkflow", "suspect.showRefGraph", "..."],
  "extensionVersion": "0.2.0"
} } }
```

`Backend::initialize` already stores `params.capabilities`
(`lib.rs:702`). It leaves the declared ids out of
`execute_command_provider.commands`. `run_command` still executes them:
VS Code's wrappers call `workspace/executeCommand` directly
(`serverCommands.ts:18`), and the LSP specification does not require a
command to be advertised before a client may call it.

In VS Code a `StaticFeature` fills the capability, and the
`commands.register` override (`extension.ts:121-126`) is deleted:

```ts
// editors/vscode/src/protocol.ts (new)
export const clientCapabilities: StaticFeature = {
  fillClientCapabilities(caps) {
    caps.experimental = { ...caps.experimental,
      suspect: { clientCommands: [...EDITOR_OWNED_COMMANDS], extensionVersion } };
  },
  initialize() {}, getState: () => ({ kind: 'static' }), dispose() {},
};
// startClient(): current.registerFeature(clientCapabilities);
```

**No `suspect.ui.*` rename.** The short version moves client-only commands
to `suspect.ui.*`. Those commands do not collide with anything (problem 5),
and VS Code has no alias mechanism for keybindings: a renamed id in
`keybindings.json` does nothing and reports nothing. The rename would cost
users their bindings and leave the real overlap in place. This proposal
keeps every contributed id. It keeps `_suspect.*` for hidden internal
commands (the existing `_suspect.toggleGateway`). It adds a test that every
`registerCommand` id is contributed in `package.json` or starts with `_`.
If reviewers still want the namespace, see Compatibility for the alias
scheme.

### Capability advertisement

`initialize` sets `capabilities.experimental` (tower-lsp 0.20's
`ServerCapabilities::experimental: Option<Value>`):

```json
"experimental": { "suspect": {
  "protocol": 1,
  "requests": {
    "suspect/generationContract": 1,
    "suspect/renderSdk": 1,
    "suspect/runWorkflow": 1,
    "suspect/documentKind": 1,
    "suspect/explainConfig": 1
  },
  "commands": { "suspect.showRefs": 1, "suspect.changeImpact": 1, "...": 1 }
} }
```

- `protocol` changes only if the envelope changes.
- Each request and command carries an integer version. Bump it when the
  params or the result change incompatibly. Adding an optional field is
  not a bump.
- `commands` lists every server command, including ones the client
  declared as its own. `executeCommandProvider` lists only the ones the
  client should register. Positional arguments are part of the versioned
  contract (for example `suspect.generateExample` takes `uri, line,
  character` while `suspect.changeImpact` takes `uri, offset`,
  `lib.rs:2710-2830`).
- `suspect/runWorkflow` is listed only under `#[cfg(feature =
  "live-run")]`.

One table drives both the registration and the advertisement, so they
cannot drift:

```rust
// crates/suspect-lsp/src/protocol.rs (new)
use tower_lsp::lsp_types::request::Request;
pub struct Versioned { pub name: &'static str, pub version: u32 }
pub const REQUESTS: &[Versioned] = &[
    Versioned { name: <generation_contract::GenerationContractRequest as Request>::METHOD, version: 1 },
    Versioned { name: <render_sdk::RenderSdkRequest as Request>::METHOD, version: 1 },
    #[cfg(feature = "live-run")]
    Versioned { name: <run_lenses::RunWorkflowRequest as Request>::METHOD, version: 1 },
    Versioned { name: <DocumentKindRequest as Request>::METHOD, version: 1 },
    Versioned { name: <ExplainConfigRequest as Request>::METHOD, version: 1 },
];
pub const COMMANDS: &[Versioned] = &[ /* the 15 ids in lib.rs:803-818 */ ];
pub fn experimental() -> serde_json::Value { /* builds the object above */ }
```

`initialize` builds `execute_command_provider.commands` from `COMMANDS`
minus the client's declared ids. A test sends each advertised method and
asserts the answer is not `MethodNotFound`. A test also asserts that every
literal in `run_command`'s match is in `COMMANDS`.

Clients gate features on this object. Example: VS Code disables Live SDK
Preview with "this suspect CLI does not support live SDK preview" when
`requests["suspect/renderSdk"]` is missing.

### Version handshake

The check moves to the server, so every client gets it, including Zed,
which cannot read `serverInfo`. The client sends its version
(`experimental.suspect.extensionVersion` above; Zed sends it through
`language_server_initialization_options`, which `zed_extension_api` 0.6
provides). In `initialized` the server compares major and minor (for `0.x`,
minor only). On a mismatch it sends one `window/showMessage` warning:

```text
suspect 0.3.1 is serving an extension built for 0.2.x. Editor features may not match.
```

A patch difference does not warn. With bundled or downloaded binaries
(below) this only happens when the user points `basePath` or
`binary.path` at another build. `warnOnVersionMismatch`
(`extension.ts:146-160`) is deleted.

### Document detection

**The server decides.** Neither VS Code document selectors nor Zed language
configs can look at file contents. Both clients therefore keep attaching to
YAML and JSON, and the server, which has already parsed every open
document, gates itself. This works for every client, Neovim and Helix
included, while a client-side filter would have to be written once per
editor.

`OpenDoc` (`state.rs:39-44`) gains a `kind: DocKind`, computed once per
parse:

```rust
// crates/suspect-lsp/src/state.rs (new type)
pub enum DocKind { OpenApi, Swagger, Arazzo, Overlay, Load, Config, Fragment, None }
```

- `OpenApi`, `Swagger`, `Arazzo`, `Overlay`, `Load`: from
  `LowDoc::sniff_family()` (`doc.rs:103-149`). It reads a few root keys of
  an already-parsed document, so it costs microseconds.
- `Config`: from `config_schema::kind_of` by file name (`config_schema.rs:46`).
- `Fragment`: an `Unknown` document that is the target of a `$ref`
  edge in the loaded workspace (`Workspace::edges()`,
  `suspect-ref/src/workspace.rs:690`), such as `schemas/Pet.yaml`.
  Navigation and hover must keep working there. The check runs once per
  workspace build, not per request.
- `None`: everything else.

For `DocKind::None` the server returns `None` from `formatting`,
`range_formatting`, `hover`, `completion`, `code_action`,
`document_symbol` and `document_link`. It already returns nothing for
diagnostics, lenses, inlay hints and semantic tokens.

**`suspect/documentKind`** (new) exposes the same value:

```ts
// params                      result
{ uri: string }            ->  { kind: 'openapi' | 'swagger' | 'arazzo' | 'overlay'
                                       | 'load' | 'config' | 'fragment' | 'none',
                                 version?: string }   // "3.1", "1.0", ...
```

For an open document it is a map lookup. For an unopened file it parses
the file with `LowDoc::parse`, which is what `did_open` does anyway. VS Code
calls it on `onDidChangeActiveTextEditor` and sets a context key
`suspect.documentKind`. The editor-context entries then use
`when: suspect.documentKind =~ /^(openapi|fragment)$/` instead of
`resourceScheme == file`.

**Globs.** Add `workspaceContains:**/*.openapi.{yaml,yml,json}` and
`**/*.overlay.{yaml,yml}` to VS Code's activation events next to the
existing Arazzo one. Keep `onLanguage:yaml` and `onLanguage:json`: an
OpenAPI document named `api.json` must still activate, and once the server
gates itself an idle server only costs a process. No file associations
change in either editor.

**Zed.** No language or grammar changes. The README documents the opt-out
for users who do not want the server on JSON:

```json
{ "languages": { "JSON": { "language_servers": ["!suspect-lsp", "..."] } } }
```

### Binary acquisition

**Resolution order, both editors:**

1. The explicit setting (`suspect.basePath` / `suspect.server.path` in
   VS Code, `lsp.suspect-lsp.binary.path` in Zed).
2. The copy that belongs to this extension version: bundled in the VSIX
   (VS Code), or downloaded once into the extension's work directory (Zed).
3. `suspect` on `PATH`. This is used on platforms P5 does not build for, or
   when Zed is offline on first start.
4. Failure, with "Locate binary…" (VS Code opens a file picker and writes
   `suspect.basePath`) and the P5 install command.

The short version puts `PATH` before the managed copy. This proposal puts
it after, because a stale `cargo install` on `PATH` would otherwise always
win and fail the version check. That is the mismatch the handshake exists
to catch.

**Platforms (needs P5).** The five P5.1 targets:

| P5 triple | VS Code `--target` | Zed `current_platform()` |
|---|---|---|
| `aarch64-apple-darwin` | `darwin-arm64` | `(Mac, Aarch64)` |
| `x86_64-apple-darwin` | `darwin-x64` | `(Mac, X8664)` |
| `x86_64-unknown-linux-musl` | `linux-x64`, `alpine-x64` | `(Linux, X8664)` |
| `aarch64-unknown-linux-musl` | `linux-arm64`, `alpine-arm64` | `(Linux, Aarch64)` |
| `x86_64-pc-windows-msvc` | `win32-x64` | `(Windows, X8664)` |

Static musl binaries serve both glibc and Alpine. `win32-arm64` and Zed's
`X86` fall through to `PATH`.

What P4 asks of P5:

- assets named `suspect-<triple>.tar.gz` on every platform, Windows
  included (`zed_extension_api` 0.6 can extract `gzip`, `gzip-tar` and
  `zip` but not `.tar.xz`, which is cargo-dist's Unix default);
- a `suspect-<triple>.tar.gz.sha256` sidecar for each;
- the executable at the archive root (`suspect` or `suspect.exe`);
- tag `v<version>`, where the extension versions equal the workspace
  version (`Cargo.toml:6`, now `0.1.0`; `package.json:5` and
  `editors/zed/extension.toml` agree).

**VS Code: platform-specific VSIXs.** The release job runs `vsce package
--target <t>` once per row above, with the binary copied to
`editors/vscode/bin/`. The Marketplace and Open VSX both serve the matching
VSIX per platform, and a universal VSIX without a binary covers the rest.
`suspectBinary()` (`runner.ts:75-89`) gains step 2:

```ts
function bundledBinary(context: vscode.ExtensionContext): string | undefined {
  const exe = process.platform === 'win32' ? 'suspect.exe' : 'suspect';
  const candidate = vscode.Uri.joinPath(context.extensionUri, 'bin', exe).fsPath;
  return fs.existsSync(candidate) ? candidate : undefined;
}
```

The Testing view, the gateway and SDK generation all spawn
`suspectBinary()`, so they use the same copy. `context.environmentVariableCollection`
prepends `bin/` to `PATH` in integrated terminals, so the existing "Run
Now" action (`serverCommands.ts:146-148`, `terminal.sendText('suspect
contract')`) runs the same build.

The short version proposes a managed download for VS Code too. This
proposal bundles instead, because the Marketplace already delivers the
right binary per platform with its own integrity checks, which removes
first-start network access, proxy handling, checksum code and update logic
from the extension.

**Zed: managed download.** Zed extensions are WebAssembly and cannot ship
a native binary, so Zed downloads. `zed_extension_api` 0.6 (pinned in
`editors/zed/Cargo.toml`, locked at 0.6.0) provides
`github_release_by_tag_name`, `download_file`, `make_file_executable`,
`current_platform`, `http_client::fetch` and
`set_language_server_installation_status`. It has no hashing helper.

```rust
// editors/zed/src/lib.rs, step 2 (new)
fn managed_binary(&mut self, id: &LanguageServerId) -> Result<String> {
    let version = env!("CARGO_PKG_VERSION");
    let dir = format!("suspect-{version}");
    let exe = format!("{dir}/{}", exe_name());
    if std::fs::metadata(&exe).is_ok_and(|m| m.is_file()) {
        return Ok(exe);
    }
    zed::set_language_server_installation_status(id, &LanguageServerInstallationStatus::Downloading);
    let release = zed::github_release_by_tag_name("LukasParke/suspect", &format!("v{version}"))?;
    let asset = format!("suspect-{}.tar.gz", triple()?);
    let url = find(&release, &asset)?;
    let want = fetch_text(&find(&release, &format!("{asset}.sha256"))?)?;
    let bytes = fetch_bytes(&url)?;                         // zed::http_client::fetch
    verify_sha256(&bytes, &want)?;                          // sha2 crate
    unpack_tar_gz(&bytes, &dir)?;                           // flate2 (rust backend) + tar
    zed::make_file_executable(&exe)?;
    remove_other_versions(&dir);
    Ok(exe)
}
```

`download_file` extracts as it downloads, so it cannot check the archive.
The sketch therefore fetches the bytes, hashes them and unpacks them in the
extension with pure-Rust crates that build for `wasm32-wasip1`. This
assumes the extension can read and write its own work directory through
`std::fs`. Zed extensions commonly do this to cache downloads (the
sandbox note in `lib.rs:4-7` is about the host's `PATH` and files). The
first hour of step 8 checks it. If it does not hold, the fallback is
`download_file(url, dir, GzipTar)` over HTTPS with no checksum, recorded as
a known gap.

### Publishing

- **VS Code Marketplace and Open VSX** from the P5 release workflow: one
  `editors` job after the binary matrix. It runs
  `vsce publish --packagePath *.vsix` with `VSCE_PAT` and
  `ovsx publish --packagePath *.vsix` with `OVSX_PAT`, for the five
  platform VSIXs and the universal one. Prerequisites in `package.json`:
  a registered publisher id in place of the placeholder `"suspect"`,
  `repository`, `license`, a 128 px PNG `icon`, `categories`, removing
  `"private": true`, a `.vscodeignore`, and an esbuild bundle so the VSIX
  does not carry `node_modules`.
- **Zed**: the registry is the `zed-industries/extensions` repository. The
  first publication is a PR that adds this repo as a submodule
  (`extensions/suspect`) and an `extensions.toml` entry with
  `path = "editors/zed"` and `version`. Each release is a PR that bumps the
  submodule and the version. The release workflow opens that PR from a
  bot account. Merging stays with Zed's maintainers, so a Zed release can
  lag the CLI by their review time. Prerequisites: a licence file inside
  `editors/zed/`, `repository` in `extension.toml`, and a unique `id`.

## Compatibility and migration

- **Renamed settings** (`testBaseUrl` to `test.baseUrl`, `gatewayPort` to
  `gateway.port`, `sdk.sessionOutput` to `codegen.out`, the other `sdk.*`
  to `codegen.*`, `formatting.sortKeys` to `format.sortKeys`,
  `inlayHints.*` to `editor.inlayHints.*`). For two releases the old keys
  stay in `settings.editor.json` with a `deprecationMessage`. The extension
  reads the new key, then the old one. On activation it offers once per
  workspace to move a set old value. The server accepts the old client
  spellings for the same window and logs one `window/logMessage` line for
  each. In `.suspect.yaml`, `formatting.sort_keys`, `inlay_hints.*` and
  `ref.max_docs` follow P2's deprecation reader.
- **`suspect.server.path`** is removed at the end of the same window. It is
  already deprecated (`package.json:48-53`).
- **Precedence**: editor settings already beat files today
  (`editor_config.rs:7-9`). Env sitting below the editor is new, because
  the server reads no settings env vars today.
- **Commands**: no ids change. If reviewers choose `suspect.ui.*` anyway,
  each old id stays registered for two releases as a hidden alias
  (`commandPalette` `when: false`) that forwards to the new id and shows a
  one-time notice naming the new id for `keybindings.json`.
- **Old clients with a new server**: a client that does not send
  `clientCommands` gets the full command list. Today's monkey-patched
  VS Code client keeps working because it still filters.
- **New clients with an old server**: `experimental.suspect` is missing.
  The client assumes protocol 0, which means today's three requests, no
  `documentKind` and no `explainConfig`, and hides what depends on them.
- **Binary order**: users with `suspect` on `PATH` and no setting switch to
  the bundled or downloaded copy. The release notes say so and name
  `suspect.basePath` and `binary.path` for keeping their own build.
- **Gating**: users who relied on suspect formatting arbitrary JSON lose
  that. This is intended.

## Implementation plan

Each step leaves `main` green. Steps 1-3 need nothing from P2 or P5.

0. **Done (2026-10-30).** ~~Fix the Live SDK Preview `importName` bug.~~ `liveSdk.ts:180` reads
   `sdk.importNames[profile]` the way `extension.ts:407` does. Test: a
   headless case asserting the request params carry a string or nothing.
   1 hour.
1. **Done.** **Protocol table, `experimental.suspect`, client-declared commands.**
   New `crates/suspect-lsp/src/protocol.rs`; `lib.rs` `initialize` and
   `service()`; `editors/vscode/src/protocol.ts` (new), `extension.ts`
   (delete the override). Tests: in `crates/suspect-cli/tests/lsp_session.rs`,
   every advertised request answers, and declared ids are left out of
   `executeCommandProvider`; in `test/ui-surface.cjs`, every
   `registerCommand` id is contributed or starts with `_`. 1 day.
   As built: custom methods are still registered one by one in `service()`
   (each has its own handler type), so the table drives the advertisement
   and `executeCommandProvider`, and the session test checks every
   advertised request against the registration. `suspect/runWorkflow` stays
   registered without `live-run` (it answers with an error) but is not
   advertised. A server older than this handshake still lists the
   extension's ids, which now fails the client start with "command already
   exists"; the extension names that case ("older than this extension")
   instead of a generic start failure.
2. **Done.** **Server-side version handshake.** `initialized` compares
   `extensionVersion`. Zed sends it through
   `language_server_initialization_options`. Delete
   `warnOnVersionMismatch`. Test: a session with `0.0.9` gets one
   `showMessage`, and a patch difference gets none. 0.5 day.
   As built: the server reads `experimental.suspect.extensionVersion`
   (VS Code) first, then `initializationOptions.extensionVersion` (Zed,
   merged into the user's `lsp.suspect-lsp.initialization_options`). A
   version that does not parse is not a mismatch.
3. **Done.** **`DocKind`, gating, `suspect/documentKind`.** `state.rs`, `lib.rs`
   handlers, VS Code context key and `when` clauses, activation globs, Zed
   README opt-out. Tests: an `lsp_session` case that opens
   `docker-compose.yml`, `package.json` and a `$ref`'d fragment and
   asserts no formatting or hover on the first two and hover on the
   fragment; a `documentKind` table test. 1 day.
   As built: `OpenDoc.kind` holds the document's own kind (family or
   config name); `Fragment` is resolved against a `FragmentIndex` cached
   per workspace state (the workspace pointer plus its document count,
   which grows as documents load). Open question 2 is taken with one
   narrowing: a spec at the workspace root does not make the whole tree
   fragments (its `package.json`, `docker-compose.yml` and `.github/`
   would all qualify), so there only `$ref` targets count; a spec in
   `api/` claims `api/` and below. `version` in the result is the family's
   `major.minor`. The context key refreshes on editor switches, on save of
   the active document and once the client starts; a server without the
   request leaves it unset, which hides the three spec-only menu entries.
   The host smoke was not extended (VS Code has no API to read a context
   key).
4. **Settings generation** (after P2.7). `editors/vscode/scripts/settings.cjs`,
   `settings.editor.json`, `schemas/suspect.schema.json`,
   `src/settings.generated.ts`, `test/settings.cjs`; regenerate
   `package.json`; replace `config.get` calls with the typed getters.
   Tests: the drift test, plus the existing `ui-surface.cjs` settings
   assertions rewritten against the schema. 1 day.
   **Done.** The setting table's `editor` column is now the camelCase path
   (`test.baseUrl`), and a new `editor_aliases` column holds the old client
   keys (`testBaseUrl`, `gatewayPort`, `ci.baseline`, `formatting.sortKeys`,
   `inlayHints.refs`, `inlayHints.properties`); `x-suspect-editor` is
   `{key, scope, aliases?}` and sits on canonical properties only.
   `scripts/settings.cjs` emits each alias as a deprecated setting with
   default `null` (so an untouched old key no longer overrides the files, as
   `testBaseUrl`'s `http://localhost:8080` default used to), plus
   `RENAMED_SETTINGS` and `CONFIG_KEYS` in `settings.generated.ts`; getters
   read the new key, then the old ones. `npm run settings:write` rewrites
   both outputs; `test/settings.cjs` also fails when a source file reads a
   `suspect.*` setting by string. With `suspect.test.baseUrl` unset the
   Testing view leaves `--base-url` off, so `suspect test` applies
   `test.base_url` from `.suspect.yaml`. Deviation: P2 did not add
   `codegen.package_name`, `import_names`, `operation_ids` or
   `compatibility_profiles`, and `codegen.out` means something else than
   `sdk.sessionOutput` (an output root for `codegen`, not the session
   preview's prompt default), so all `suspect.sdk.*` settings stay
   editor-only in `settings.editor.json` for now; the `sdk.*` to `codegen.*`
   renames wait for those keys.
5. **Server reads the editor layer through `resolve_with`** (after P2.1).
   Delete `editor_config::apply_client`, the client half of
   `parse_config`, and the manifest `editor` unwrapping. Migrate the
   `editor_config.rs` unit tests to `suspect-config`. Old-spelling
   aliases and log lines. 1 day.
   **Done.** P2 step 4 had already deleted `editor_config.rs` and the
   client half of `parse_config` and moved the pulled section into
   `resolve_found` (editor above env, as open question 1 recommends); the
   manifest `editor` section is P2's deprecated manifest layer, read for
   one more release, so it stays until P2's removal. What this step adds:
   `Layer::from_editor` reads current spellings first and an old client
   key (`editor_aliases`, or a deprecated file spelling reached through
   the snake_case fallback) only when the current key is unset in the same
   section, with a warning `editor suspect.testBaseUrl: deprecated setting;
   use suspect.test.baseUrl`; a `null`/blank old key (VS Code sends every
   contributed key) warns about nothing. The server logs these
   (`window/logMessage`) whenever the files or the client's settings
   change, not only when the effective values change. Tests: the alias
   read in `layer.rs`; `crates/suspect-cli/tests/lsp_editor_settings.rs`
   for the log line. The Zed README documents the new spellings.
6. **Renamed settings migration and `suspect/explainConfig`** with the
   status-bar item; delete `config.ts`'s regex. Tests: a host-smoke case
   (the floor picker shows the layer), a unit test for the alias read.
   0.5-1 day.
   **Done.** `suspect/explainConfig` (`protocol::ExplainConfigRequest`, in
   `REQUESTS` at version 1) takes `{uri?}` and returns
   `Resolved::explain_json()` for that document's configuration (the
   workspace's without one) plus `source` (the settings file's name, else
   the manifest's, else `null`) and `error` (the first rejected value).
   VS Code: `src/explainConfig.ts` (status-bar item `suspect: .suspect.yaml`,
   `suspect: defaults`, or a warning icon with the error in the tooltip;
   refreshed on client start, editor switches, `suspect.*` changes and
   config-file events) and a new `suspect.explainConfig` command ("Explain
   Configuration") that opens the report as Markdown; the floor picker and
   the Project view read `lint.min_severity` and its layer from the report,
   and without a server that offers it fall back to the setting, the
   manifest, then `hint` (no file parsing). The gateway uses the report's
   `gateway.port` for the spec. `src/migrateSettings.ts` offers once per
   workspace to move set old values to their new keys at the same level
   (user, workspace, folder), skipping a level where the new key is
   already set. Tests: `lsp_editor_settings.rs` (editor over
   `.suspect.yaml` with the trace, a rejected value as `error`, camelCase
   and snake_case payloads alike); `ui-surface.cjs` (floor from the
   report, status text, rendering, gating, move planning); the host smoke
   now writes a `.suspect.yaml` and asserts the report row
   `| error | editor setting | .suspect.yaml:2 = hint; editor
   suspect.lint.minSeverity = error |` (run locally, green). The P4 sample
   result shape (`values[].layer`, `trace`) is P2's `explain_json`, which
   also carries `origin`, `root`, `config`, `files`, `warnings`, `errors`.
7. **VS Code bundled binary** (after P5 artifacts). `runner.ts`
   resolution order, "Locate binary…", terminal `PATH`, esbuild bundle,
   `.vscodeignore`, `package.json` publishing fields. Test: host smoke with
   a VSIX built for the runner's platform and no `basePath`. 1 day.
   **Done.** Size first: the release `suspect` for `aarch64-apple-darwin` is
   40.7 MB (35.6 MB stripped); its platform VSIX is 15.1 MB, so bundling
   stays. `runner.ts` `resolveBinary`: a non-default `suspect.basePath` (or
   `suspect.server.path`), then `<extension>/bin/suspect[.exe]`, then
   `suspect` on `PATH`; the bundled copy's `bin/` is prepended to `PATH` in
   integrated terminals (`environmentVariableCollection`), and its mode bit
   is restored if an installer dropped it. A start failure offers "Locate
   binary…" (file picker, writes `suspect.basePath` in user settings; also
   the `suspect.locateBinary` command) and names the P5 installer command.
   `scripts/bundle.cjs` (esbuild 0.28.2, run by `vscode:prepublish` after
   `tsc`) writes `dist/extension.js`; `.vscodeignore` ships only that file
   of `dist/` and no `node_modules`, and packaging passes
   `--no-dependencies` (universal VSIX 518 KB -> 159 KB). tsc's per-module
   `dist/*.js` stay for the headless tests. `scripts/package-target.cjs
   --target <t> --binary <path>` builds `suspect-vscode-<t>-<V>.vsix` with
   the binary at `bin/`; `package-editors.yml` gains a `vscode-platform`
   matrix (the seven targets of the table above, alpine sharing the musl
   builds) that verifies each `suspect-cli-<triple>` archive against its
   `.sha256`, packages, and runs `bin/suspect --version` for `linux-x64`;
   `asset-names` checks the 14 new names. The universal VSIX stays (open
   question 5). Host smoke: `SUSPECT_SMOKE_VSIX=<vsix> node
   test/host-smoke/run.cjs` unpacks the VSIX, runs it with no `basePath` and
   every `PATH` entry holding a `suspect` removed; green locally with a
   `darwin-arm64` VSIX of both the debug and the stripped release build.
   Not in CI (needs a desktop VS Code, as before). The `package.json`
   publishing fields were P5 step 5.
8. **Zed managed download** (after P5 artifacts). `editors/zed/src/lib.rs`,
   new deps `sha2`, `flate2`, `tar`; licence file; `extension.toml`
   `repository`. Tests: unit tests for `triple()`, `verify_sha256` and
   unpacking (host `cargo test`); the `zed-extension` CI job keeps
   building for `wasm32-wasip1`; `zed_smoke.sh` gains a no-`PATH` run.
   1 day.
   **Done, not run against a real release** (none exists yet). The licence
   files and `repository` were P5 step 5. New `editors/zed/src/download.rs`
   holds the pure helpers: `triple(os, arch)` (the table above; others
   fall through to `PATH`), `asset_name` (`suspect-cli-<triple>.tar.gz`,
   `.zip` for Windows per P5's asset contract, so the extension also reads
   zip: a minimal central-directory reader for stored and deflated members
   instead of the `zip` crate), `parse_sha256` (first token, 64 hex
   digits), `verify_sha256` (`sha2`), and `extract_binary` (`flate2` with
   the Rust backend + `tar`, no default features), which takes only the
   `suspect[.exe]` member at depth 1 or 2 and writes nothing else.
   `lib.rs` `managed_binary`: reuse `suspect-<V>/suspect` when present,
   else `github_release_by_tag_name("LukasParke/suspect", "v<V>")`, fetch
   the `.sha256` and the archive with `http_client` (`RedirectPolicy::FollowAll`
   for GitHub's asset redirect), verify, unpack into `suspect-<V>.partial/`
   and rename it into place, `make_file_executable`, delete other
   `suspect-*` directories; installation status is `Downloading`, then
   `None`, or `Failed(<reason and the options>)` when the download and
   `PATH` both fail. The order is `binary.path`, managed, `PATH`. Tests: six
   host unit tests (triples and names, checksums, tar.gz at the root, one
   directory down and too deep, zip stored and deflated); the
   `zed-extension` CI job (`cargo xtask ci zed-extension`) now runs them
   before the `wasm32-wasip1` build. The work-directory `std::fs` access is
   the pattern other Zed extensions use, but has not been exercised in a
   running Zed here. `zed_smoke.sh` gains `ZED_SMOKE_MANAGED=1` (no
   `binary.path`, `suspect` must be absent from `PATH`, expects the process
   from the extension's work directory); it skips while release `v<V>`
   does not exist, so it is a documented manual check.
9. **Publishing jobs** in P5's release workflow: VSIX matrix, `vsce` and
   `ovsx` publishing, Zed registry PR. Account setup (publisher, Open VSX
   namespace, tokens) is outside the estimate. 0.5 day.

Total: about 7-8 days. The suggested-order table in the source document
gives 2-3 days for P4. The difference is mostly steps 3, 7 and 8, which
the short version did not size.

## Testing

- **Unit** (`crates/suspect-lsp`): the `protocol::REQUESTS` and `COMMANDS`
  tables against `run_command`'s match arms; `DocKind` for each family,
  config names and fragments; the version comparison (patch, minor, `0.x`).
- **Integration** (`crates/suspect-cli/tests/lsp_session.rs`, which drives
  the real server): the advertisement round trip; `clientCommands`
  filtering; gating on non-spec files (the probe in Problem 8 as
  assertions); `suspect/documentKind`; `suspect/explainConfig` showing the
  editor layer winning over `.suspect.yaml`; a camelCase and a snake_case
  client payload giving the same result.
- **Config** (`crates/suspect-config/tests/`): the P2.7 schema bless test;
  `resolve_with` precedence `editor > env > file > default`.
- **VS Code headless** (`npm run test:headless`, already in CI):
  `test/settings.cjs` (drift), `ui-surface.cjs` (command ownership, `when`
  clauses), unit tests for binary resolution order with a fake
  `extensionUri`.
- **VS Code host smoke** (`npm run test:host:smoke`): extend `suite.cjs`
  to assert `experimental.suspect` is read, `suspect.documentKind` is set
  for the petstore spec, and the status-bar item names `.suspect.yaml`.
  Today this runs only on a desktop (`run.cjs:7-8`). Add a Linux CI job
  that downloads a pinned VS Code with `@vscode/test-electron` and runs it
  under `xvfb-run`. 0.5 day on top of the plan.
- **Zed**: `cargo test` on the host for the pure helpers, the existing
  WASI build job, and `editors/multiclient/zed_smoke.sh` locally.
- **Neovim**: `nvim_smoke.lua` asserts formatting a non-spec JSON buffer
  returns no edits.

## Alternatives considered

- **Hand-maintained settings with a lint test** that only compares names.
  Lost: types, defaults and enums would still drift, as
  `compatibilityProfiles` shows.
- **Rewriting `package.json` from Rust.** Lost: it needs `serde_json`'s
  `preserve_order`, which would be unified into the workspace build, or a
  hand-written JSON printer.
- **Snake_case editor keys (`suspect.lint.min_severity`).** Lost: it
  breaks every existing setting for spelling alone and goes against
  VS Code's convention. The case mapping is mechanical.
- **`suspect.ui.*` for client commands.** Lost: see Command ownership.
- **Namespacing server commands instead (`suspect.server.*`).** Lost: it
  breaks lens and code-action ids that Zed and Neovim users run, and it
  still needs the client to declare which ids it wraps.
- **Client-side document filtering** (VS Code middleware dropping
  `didOpen`; a custom Zed "OpenAPI" language reusing tree-sitter-yaml with
  `path_suffixes` and `first_line_pattern`). Lost: it has to be written per
  editor. In Zed it would duplicate the grammar and highlight queries and
  take OpenAPI files out of the user's YAML language settings.
  `first_line_pattern` only matches files whose first line is `openapi:`.
- **Managed download in VS Code.** Lost: see Binary acquisition.
- **PATH before the managed copy.** Lost: a stale install wins silently.
  It remains the fallback.
- **Zed `download_file` without a checksum.** Kept only as the fallback if
  the extension cannot use `std::fs` in its work directory.

## Risks

- **VSIX size.** The binary links every codegen backend. The debug build is
  163 MB (`ls -la target/debug/suspect`). No release build was measured. If
  the stripped release binary is too large for comfortable per-platform
  VSIXs, the fallback is the download path for VS Code too. Step 7 starts
  by measuring it.
- **Zed work-directory access** (see Binary acquisition). If it fails, the
  checksum is lost, not the feature.
- **Zed registry latency.** Releases wait on a third-party review, so Zed
  users can run one version behind. The handshake warns them, because the
  downloaded binary matches the extension, not the latest CLI.
- **Gating false negatives.** An OpenAPI fragment that nothing references
  yet (a new `schemas/Foo.yaml`) is `DocKind::None` until the first `$ref`
  to it is saved. Mitigation: treat `Unknown` documents under a directory
  that holds a spec as `Fragment`. This is an open question below.
- **P2 schema shape.** Generation depends on P2.7 emitting nested
  `properties` with `description`, `default`, `enum` and the
  `x-suspect-editor` annotation. If P2 instead emits `$defs` and `$ref`,
  `exposed()` must follow them (about 10 lines).

## Open questions

1. **Editor layer above or below env?** Recommended: above (editor acts as
   the flag layer), for the reason under Precedence.
2. **Should `Fragment` include unreferenced YAML next to a spec?**
   Recommended: yes, for documents in the directory of any loaded spec or
   below it. It is cheap, and the cost of a false positive is a hover on a
   file that probably is a fragment.
3. **Should `extensions` (`config_files.rs:257`) get an editor setting?**
   Recommended: no. It is project policy, so it belongs in `.suspect.yaml`
   only.
4. **Should `suspect.sdk.sessionConfig` stay editor-only?** Recommended:
   yes, until P2.6 decides the session config format. It names a file, not
   a value.
5. **Should the universal VSIX download a binary on supported
   platforms?** Recommended: no. The Marketplace serves the platform VSIX
   to supported platforms, so the universal one only reaches platforms with
   no binary.
6. **Who owns the Zed registry PR?** Recommended: the release workflow
   opens it from a bot account with a fine-grained token scoped to a fork
   of `zed-industries/extensions`, and a maintainer is assigned to follow
   it through review.
