# P1. One finding model and one check pass

Status: proposal. Depends on: P2.1 (`suspect_config::resolve`) for the final
policy input. Steps 1-4 below do not need it. Decided: the `sdk-*` readiness
codes stay a separate category, shown only when codegen is configured
(2026-10-29). Severity names are parsed by one function (already fixed; this
proposal moves that function, it does not reopen it).


Fixed since this was written (2026-10-30, see the
[index](README.md#already-fixed)): `rules check` reporting `off` findings as warnings, and `ci` forcing `strict_format` off. The Problem section keeps
them as evidence.
## Summary

For a user: `suspect check` runs every check the editor runs and exits 1 on
errors. A problem has one code and one severity in the terminal, in SARIF and
in the editor. `validate` and `lint` become filtered views of `check`.
Findings can be suppressed inline (`# suspect-disable` in YAML,
`x-suspect-disable` in YAML or JSON) or recorded in a baseline file so CI fails
only on new findings.

In the code: a new leaf crate `suspect-diag` holds `Finding`, `Severity`, the
severity parser, rule metadata and the text/JSON/SARIF/JUnit renderers. A new
crate `suspect-check` runs every check and applies policy once. `suspect-lsp`
and `suspect-cli` both call it. The LSP keeps its scheduling and caches, and
the LSP-only checks move out of `suspect-lsp` so the CLI runs them too.

## Problem

### 1. Many finding types, converted in many places

There are 35 finding or diagnostic structs and 10 severity or kind enums in the
toolchain. I found them with
`grep -rnE "(struct|enum) \w*(Diagnostic|Finding|Severity|Problem|Violation)"`
and by reading each hit. I left out the three `ValidationFinding` structs in
`suspect-codegen/src/rust_validation/runtime*.rs`, because those files are
copied into generated SDKs with `include_str!` (`rust_validation.rs:142-146`).

Twelve of them feed spec checking. These are what this proposal unifies:

| Type | Location | Severity | Range | Pointer |
|---|---|---|---|---|
| `suspect_validate::Diagnostic` | `suspect-validate/src/diagnostic.rs:33` | own `Severity` (Error/Warning/Info) | yes | no |
| `suspect_lint::Finding` | `suspect-lint/src/engine.rs:14` | own `Severity` (+ `Off`) | yes | yes (`path: Pointer`) |
| `ArazzoDiagnostic` | `suspect-arazzo/src/validate.rs:8` | none (LSP hard-codes ERROR) | yes | no |
| `OverlayDiagnostic` | `suspect-overlay/src/model.rs:171` | none | **no** | no |
| `RefDiagnostic` | `suspect-ref/src/edges.rs:48` | none, no code | yes | yes |
| `ProfileDiagnostic` | `suspect-test/src/load/profile.rs:23` | `error: bool` | yes | no |
| `TsFinding` | `suspect-rules/src/host.rs:19` | `Option<String>` | optional | yes (`String`) |
| `SyntaxError` | `suspect-syntax/src/doc.rs:177` | none | yes | no |
| `config_schema::Problem` | `suspect-lsp/src/config_schema.rs:725` | none (LSP uses WARNING) | yes | no |
| `markdown::LintFinding` | `suspect-lsp/src/markdown.rs:450` | none (LSP uses HINT) | yes | no |
| `AdmissionFinding` | `suspect-codegen/src/admission.rs:94` | `FindingKind` | yes | no |
| `ContractDiagnostic` | `suspect-ir/src/contract/mod.rs:106` | `ContractSeverity` | yes | no |

The vendor-extension check builds LSP `Diagnostic`s directly, with no type of
its own (`suspect-lsp/src/diagnostics.rs:228-293`).

The other 23 are out of scope (see Non-goals):

- 10 codegen-internal types in `suspect-codegen`: `terraform::Diagnostic`,
  `HttpDiagnostic`, two `ModelDiagnostic`s, `BackendDiagnostic`,
  `schema_view::Problem`, `PlanFinding`, `ExampleDiagnostic`,
  `application::Diagnostic`, `http_protocol::Diagnostic`.
- Traffic and runtime types: `suspect_journal::Violation`,
  `suspect_runtime::Violation`, `OwnedFinding`.
- CLI and LSP report types: `BreakFinding`, `QueryParamGap`, `CoverageGap`,
  three private `Diagnostic` structs in `terraform_cmd.rs`, `sdk.rs` and
  `application_cmd.rs`, `RenderDiagnostic`, `ContractFinding`.
- The wire type `suspect_rules::protocol::FindingBody`.
- `suspect_cli::output::Finding` (`output.rs:63`), which is the CLI's
  existing merge type.

The short version says `output.rs` converts the producer types. It does not.
`output.rs` only defines `Finding` and `Severity`. Each command converts on
its own:

- `commands/validate.rs:133-156`, which also appends `how to fix` to the
  message text;
- `commands/lint.rs:100-119`;
- `commands/rules.rs:34-55`;
- `commands/check.rs:69-107`;
- `commands/admission.rs:41-107`;
- `commands/breaking.rs:164`;
- `commands/arazzo_diff.rs:238`.

The LSP converts separately in `diagnostics.rs:22-446`, with one mapper per
producer.

There are still four severity parsers. The CLI and config use
`suspect_config::parse_severity` (`severity.rs:40`). Lint rulesets use
`suspect_lint::Severity::from_text` (`rule.rs:35`), which accepts `warn` but
not `warning`. CLI lint overrides and LSP policy use
`suspect_lint::Severity::from_policy` (`rule.rs:50`, called from
`commands/lint.rs:73`, `diagnostics.rs:173` and `config_files.rs:765`).
Spectral numbers go through `from_number` (`rule.rs:59`).

### 2. The CLI and the LSP run different checks

`compute_diagnostics_raw` (`suspect-lsp/src/diagnostics.rs:374-403`) runs
these checks: syntax, `.suspect.yaml` schema, OAS 3.x validation, lint,
Swagger 2.0, Arazzo, markdown, vendor-extension schemas and load profiles.

| Check | LSP | `validate` | `lint` | `ci` | `rules check` |
|---|---|---|---|---|---|
| syntax | yes (code `syntax`) | yes (code `syntax-error`) | no | via validate | no |
| `$ref` resolution | yes (inside validate) | yes | no | via validate | no |
| OAS 3.x semantic | yes | yes | no | yes (`strict_format` forced off, `ci.rs:458`) | no |
| Swagger 2.0 semantic | yes (`validate_swagger_low`) | **no**: `validation-input: document is not OpenAPI 3.x (family: Oas2)` | no | no | no |
| Arazzo | yes (`validate_arazzo`) | **no**: same `validation-input` error | no | no | no |
| Overlay structure (`validate_overlay`) | **no** | no | no | no | no |
| load profile | yes | no | no | no | no |
| Spectral-style lint | yes | no | yes | yes | no |
| markdown (`md-*`) | yes | no | no | no | no |
| vendor-extension schemas | yes | no | no | no | no |
| `.suspect.yaml` schema | yes (code `suspect.config`) | no | no | no | no |
| TS custom rules | no | no | no | no | yes |
| SDK admission | on request only (`suspect/generationContract`) | no | no | codegen stage | no |

`validate_overlay` (`suspect-overlay/src/model.rs:181`) has no caller outside
`suspect-overlay/tests/overlay.rs`. An overlay with `actions: []` gets no
`overlay-empty-actions` finding anywhere. It gets only the lint hint
`overlay-info-description` (reproduced below). `Workspace::ref_diagnostics`
(malformed `$ref` values) is called only from `suspect-ref/tests`.

Policy differs as well:

- In the LSP, `lint.rules.<code>` applies to every code, including validate
  codes (`config_files.rs:750-781`). In the CLI it applies only to lint
  findings (`commands/lint.rs:152-166`), and `validate` ignores it.
- `validate` applies no severity floor. The LSP applies the floor to
  everything (`lib.rs` push path, `filter_at_least`).
- An unknown severity in `lint.rules` is an error in `suspect lint`
  (`commands/lint.rs:88-93`). In the LSP it is ignored without a message
  (`config_files.rs:779`, `diagnostics.rs:173`).
- `suspect rules check` reports a TS finding with severity `off` as a warning
  (`commands/rules.rs:19-24`: `off` maps to `None`, and `None` becomes
  `Warning`).

### 3. One problem, several codes and severities

Reproduced on `docs/demo/petstore.yaml` with `target/debug/suspect` built from
this branch. The LSP rows come from a stdio probe that sends
`initialize`/`didOpen` and prints `publishDiagnostics`
(`/tmp/p1-scripts/lsp_probe.py`, not committed).

Missing `operationId` (line 8 col 7, and again at 18:7):

| Path | Code | Severity |
|---|---|---|
| `validate` / LSP | `oas-operation-missing-operationId` | warning |
| `validate` / LSP | `sdk-operation-missing-id` | info |
| `lint` / LSP (`suspect-lint`) | `operation-operationId` | error |

Undeclared path parameter `{petId}`, reported at two different positions:

| Path | Code | Severity | Position |
|---|---|---|---|
| `validate` / LSP | `oas-path-param-not-declared` | error | 17:5 (path item) |
| `lint` / LSP | `path-params` | error | 18:7 (operation) |

A second spec, `/tmp/p1-repro/overlap.yaml`, shows more one-to-one pairs. Each
pair is reported at the same position:

| validate code (severity) | lint code (severity) |
|---|---|
| `oas-license-missing-url` (warning) | `license-url` (warning) |
| `oas-path-trailing-slash` (warning) | `path-keys-no-trailing-slash` (warning) |
| `oas-tag-undeclared` (warning) | `operation-tag-defined` (error) |
| `oas-duplicate-operation-id` (error) | `operation-operationId-unique` (error) |

Arazzo in the LSP (`/tmp/p1-repro/flow.arazzo.yaml`, a step with no
operation): `arazzo-step-missing-operation` (validate, error) and
`arazzo-step-operation` (lint, error), both at 9:9. Swagger 2.0 in the LSP:
`swagger-path-param-undeclared` and `path-params`.

The code already treats some of these pairs as one problem. `actions.rs:268`
matches `"oas-path-trailing-slash" | "path-keys-no-trailing-slash"`, and
`actions.rs:332` matches `"oas-license-missing-url" | "license-url"`. Those
are aliases that exist only inside the code-action table.

There is also overlap inside lint. `info-contact` (warning) and
`oas3-api-contact` (info) both fire at 3:3 on petstore.

### 4. `suspect check` is a parse report

`commands/check.rs:143-171` prints family, syntax error count, `$ref` edges
and the cycle census. It exits 1 only on syntax or IO errors.

```text
$ suspect check docs/demo/petstore.yaml; echo $?
docs/demo/petstore.yaml
  family:           Oas31
  ...
0
```

`validate` exits 1 on the same file, because `oas-path-param-not-declared` is
an error. `check --format sarif` prints the JSON report, not SARIF
(`check.rs:164`). In text mode the findings themselves are never printed;
only the counts are.

### 5. No suppression

Nothing in `crates/` or `editors/` reads a disable comment or extension. I
grepped for `suspect-disable`, `x-suspect-disable`, `spectral-disable` and
`nolint` and got no matches. `suspect-cli/src/baseline.rs` resolves a git
revision for `breaking`. `ci --baseline` is that git ref
(`commands/ci.rs:26`, `515`). `silence.rs` redirects stdout and stderr.

### 6. Admission is not an editor diagnostic

`compute_diagnostics_raw` never calls admission. Admission findings reach the
editor only through `suspect/generationContract`
(`suspect-lsp/src/generation_contract.rs`), and the CLI through
`suspect admission`.

## Goals and non-goals

Goals:

- One `Finding` and one `Severity` type for every spec check. One severity
  parser.
- One function that runs every check, used by the CLI, `ci` and the LSP.
  Parity is tested.
- Every code is registered once, with an owner, a default severity, a
  summary and how-to-fix text. Overlapping codes are aliases of one
  canonical code. The finding is emitted once, and an override on either name
  applies to it.
- Inline suppression and a findings baseline.
- `suspect check` runs everything. `validate` and `lint` are filters of it.
- LSP latency and caches stay as they are.

Non-goals:

- Unifying codegen-internal, runtime-validation or traffic types (the 23
  listed above). They are not spec checks. They keep their types and may
  render through `suspect-diag` later.
- Changing what any check detects, apart from wiring Overlay structure and
  malformed `$ref` diagnostics, which are written but never run.
- Merging overlapping rules whose semantics differ (for example
  `operation-4xx-response` and `oas-operation-no-error-response`). They are
  listed as candidates with a recommendation, not aliased.
- The command tree and flag grammar (P3). This proposal only changes
  `check`, `validate`, `lint` and `ci` stages.
- Config resolution (P2). P1 consumes whatever `suspect_config` returns.

## Design

### Crate graph

```text
suspect-source
   ^
suspect-diag            (new, leaf: suspect-source, serde, serde_json)
   ^
   |-- suspect-config   (severity.rs moves here -> suspect-diag; re-exported)
   |-- suspect-validate, suspect-lint, suspect-arazzo, suspect-overlay,
   |   suspect-test, suspect-rules, suspect-codegen   (export rule tables)
   ^
suspect-check           (new)
   ^
   |-- suspect-lsp
   |-- suspect-cli      (already depends on suspect-lsp; adds suspect-check)
```

`suspect-diag` depends only on `suspect-source` (for `Uri` and `LineIndex`),
`serde` and `serde_json`. It must not depend on `suspect-low`,
`suspect-syntax`, `tower-lsp`, `clap` or any check crate. The reason is that
`suspect-config`, `suspect-rules` and `suspect-codegen` all need it, and a
leaf crate cannot form a cycle with any of them. Pointers are stored as RFC
6901 text. That is the form SARIF and the baseline file need anyway.

`suspect_config::severity` moves into `suspect_diag::severity`. For one
release, `suspect-config` re-exports `SeverityName` and `parse_severity` with
`#[deprecated]`. `suspect_lint::Severity::from_policy` is deleted. Ruleset
parsing keeps `from_text`/`from_number`, because those parse the Spectral
ruleset vocabulary, not policy. After parsing they map onto
`suspect_diag::Level`.

`suspect-check` depends on `suspect-diag`, `suspect-config`, `suspect-source`,
`suspect-low`, `suspect-syntax`, `suspect-ref`, `suspect-oas`,
`suspect-validate`, `suspect-lint`, `suspect-arazzo`, `suspect-overlay`,
`suspect-schema`, `suspect-test` (load profiles) and `suspect-rules`. It also
depends on `suspect-codegen` (admission) behind a default-on `sdk` feature.
`suspect-lsp` already depends on all of these except `suspect-rules`
(checked with the `[dependencies]` sections of each `Cargo.toml`). Only
`suspect-lsp` and `suspect-cli` depend on `suspect-check`. No producer crate
may depend on it. A test in `suspect-check` reads `cargo metadata` and
asserts this.

The orchestrator gets its own crate rather than a module in an existing one:

- `suspect-validate` would have to depend on lint, rules, test and codegen.
- `suspect-lsp` is the wrong owner for code the CLI also runs, even though
  the CLI already links it.

Code that moves out of `suspect-lsp` so the CLI can run it:

| From | To | Why |
|---|---|---|
| `suspect-lsp/src/markdown.rs` (865 lines, depends on low/syntax only) | `suspect-lint/src/markdown.rs` | `md-*` codes are lint codes. `semantic.rs` keeps using it through `suspect_lint::markdown`. |
| `config_schema::problems` and its schema table | `suspect-config/src/schema.rs` | Config validation belongs with the config types (and with P2.7). LSP hover and completion stay in `suspect-lsp` and call it. |
| `diagnostics::extension_diagnostics` and `extensions_config.rs` | `suspect-check/src/checks/extensions.rs` | The check only needs `suspect-schema` and `suspect-overlay::Value`. `keys::Context` is only ever passed as `Unknown`. |
| `navigation::node_at` | `suspect_low::LowDoc::node_at` | Needed by `config_schema` (after the move) and by the pointer and suppression lookup below. |

### `suspect-diag` types

```rust
// crates/suspect-diag/src/lib.rs (new)
use std::borrow::Cow;
use std::ops::Range;
use suspect_source::Uri;

/// Ordered so `>=` is "at least as severe".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Hint, Info, Warning, Error }

/// A configured level: a severity, or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level { Off, On(Severity) }

/// The one parser (moved from suspect-config/src/severity.rs:40).
pub fn parse_level(name: &str) -> Option<Level>;

/// Which check produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckId {
    Syntax, Ref, Validate, Swagger, Arazzo, Overlay, LoadProfile,
    Lint, Markdown, Extension, Config, Rules, Admission,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub code: Cow<'static, str>,
    pub severity: Severity,
    pub message: String,
    pub document: Uri,
    /// Byte range in `document`; `None` for document-level findings.
    pub range: Option<Range<usize>>,
    /// RFC 6901 pointer to the offending node, when known.
    pub pointer: Option<Box<str>>,
    pub check: CheckId,
    pub related: Vec<Related>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Related { pub document: Uri, pub range: Range<usize>, pub message: String }

/// Static metadata for one code.
#[derive(Debug, Clone, Copy)]
pub struct RuleInfo {
    pub code: &'static str,
    pub check: CheckId,
    pub default: Severity,
    pub class: RuleClass,
    pub summary: &'static str,
    pub how_to_fix: &'static str,
    /// Other codes that report this same problem.
    pub aliases: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleClass { Document, Design, Sdk }
```

This differs from the short version in four ways:

- `check: CheckId` replaces `source: Source`. `Source` already names
  `suspect_source::Source`, and the value is "which check", not a file.
- There is no `fix` field. Code actions are derived from code plus range
  (`suspect-lsp/src/actions.rs:265`), and only `TsFinding.fix` carries a fix
  today, as opaque JSON. Add the field when TS fixes are wired.
- `summary` and `how_to_fix` live in `RuleInfo`, not in each finding.
  `suspect_validate::Diagnostic` copies them into every instance today.
- `Severity` is ordered ascending, matching `output::Severity`
  (`output.rs:9-22`), so the floor is a plain `>=`.

Renderers in `suspect_diag::render` take findings and a small lookup trait:

```rust
pub trait Sources {
    fn line_col(&self, doc: &Uri, offset: usize) -> Option<(u32, u32)>; // 0-based
    fn display_path(&self, doc: &Uri) -> String;
}
pub fn text(findings: &[Finding], src: &dyn Sources, out: &mut dyn std::io::Write) -> std::io::Result<()>;
pub fn json(findings: &[Finding], src: &dyn Sources) -> serde_json::Value;
pub fn sarif(findings: &[Finding], src: &dyn Sources, registry: &dyn RuleLookup) -> serde_json::Value;
pub fn junit(findings: &[Finding], src: &dyn Sources) -> String;
```

`text` keeps today's `file:line:col [SEVERITY] code: message` line
(`output.rs:84-94`). `sarif` is `suspect-cli/src/sarif.rs` moved here, with
three additions: `endLine`/`endColumn`, rule `shortDescription` and `help`
from `RuleInfo` (today it uses the code as the description, `sarif.rs:27`),
and `partialFingerprints` from the baseline key. The LSP conversion stays in
`suspect-lsp` (`to_lsp(&Finding, &LowDoc) -> lsp_types::Diagnostic`, using
`state::lsp_range`), so `tower-lsp` stays out of `suspect-diag`.

### Rule registry and aliases

Codes are defined in these places today:

- **validate**: string literals in `suspect-validate/src/checks/*.rs`, with
  summary and fix text in `guidance.rs:22`/`103`. There are 74 codes: 3 `sdk-*`,
  11 `swagger-*`, `invalid-ref`, `unresolved-ref`, `ref-outside-allowlist`,
  and the rest `oas-*`.
- **lint**: `Rule::new(code, ..)` in `suspect-lint/src/packs/{oas,security,
  overlay_arazzo}.rs`, 53 codes. `security.rs:103` marks the security pack
  `Category::Design`.
- **arazzo**: string literals in `suspect-arazzo/src/validate.rs`, 34 codes.
- **load profile**: `suspect-test/src/load/profile.rs` (`load-*`).
- **overlay**: 3 codes in `suspect-overlay/src/model.rs`.
- **markdown**: 5 `md-*` codes in `suspect-lsp/src/markdown.rs`.
- **admission**: `suspect-codegen/src/admission.rs:34-60` (`naming-*`,
  `http-*`).
- **command-local**: `syntax-error`, `validation-input` and `io-error` in the
  CLI; `syntax`, `suspect.config` and `extension-schema` in the LSP.

Each check crate exports a static table built from the data it already has:

```rust
// suspect-validate/src/lib.rs (new)
pub fn rules() -> &'static [suspect_diag::RuleInfo];
// suspect-lint: builtin packs; Linter::rules_info() for a compiled ruleset
pub fn builtin_rules() -> &'static [suspect_diag::RuleInfo];
```

`suspect_check::Registry` merges the tables. It also takes dynamic entries
from a compiled custom ruleset (`Linter`) and from TS rule metadata
(`RuleHost::rules()`, `suspect-rules/src/host.rs:141`).

```rust
// crates/suspect-check/src/registry.rs (new)
pub struct Registry { by_code: HashMap<Box<str>, Entry>, alias_to: HashMap<Box<str>, Vec<Box<str>>> }
impl Registry {
    pub fn builtin() -> Self;
    pub fn with_dynamic(self, rules: impl IntoIterator<Item = OwnedRuleInfo>) -> Result<Self, RegistryError>;
    /// Canonical code(s) for a code or alias.
    pub fn canonical(&self, code: &str) -> &[Box<str>];
    pub fn info(&self, canonical: &str) -> Option<&Entry>;
}
```

The syntax code becomes one code, `syntax-error`, for both CLI and LSP. The
LSP's `syntax` stays as an alias for one release.

Alias table v1 (each pair was reproduced at the same position above):

| Canonical (owner) | Alias (lint) | Canonical default |
|---|---|---|
| `oas-operation-missing-operationId` (validate) | `operation-operationId` | warning |
| `oas-duplicate-operation-id` (validate) | `operation-operationId-unique` | error |
| `oas-path-trailing-slash` (validate) | `path-keys-no-trailing-slash` | warning |
| `oas-license-missing-url` (validate) | `license-url` | warning |
| `oas-tag-undeclared` (validate) | `operation-tag-defined` | warning |
| `arazzo-step-missing-operation`, `arazzo-step-multiple-targets` (arazzo) | `arazzo-step-operation` | error |
| `syntax-error` (syntax) | `syntax` (old LSP code) | error |

The validate code is canonical because validate resolves `$ref`s, knows the
family, carries how-to-fix text, and has the LSP code actions
(`actions.rs:271`).

Candidates for v2. These overlap, but the positions or semantics differ, so
they need a decision per pair:

- `path-params` vs `oas-path-param-not-declared` (17:5 vs 18:7). Spectral's
  `path-params` also covers unused and duplicate parameters.
- `path-params` vs `swagger-path-param-undeclared`.
- `operation-4xx-response` (warning, 4XX only) vs
  `oas-operation-no-error-response` (info, default or 4XX/5XX).
- `info-contact` vs `oas3-api-contact`. Both are lint codes, so this is a
  lint pack fix, not an alias.

Rules for aliases:

- An alias may point at several canonical codes, as `arazzo-step-operation`
  does. An override on the alias applies to all its targets. An override on
  the canonical code wins over an override on an alias.
- Aliases apply only to builtin rule definitions. A custom ruleset that keeps
  the name `operation-operationId` but replaces `given`/`then` (as in
  `suspect-lint/src/ruleset/tests.rs:26`) defines a different check. That
  rule keeps its own code and is not merged. This needs a new
  `Rule::builtin: bool`, set by `Rule::new` and cleared by the ruleset
  compiler when it replaces `given` or `then`. A severity-only override in a
  ruleset counts as an override of the canonical code.
- TS rule ids and custom ruleset codes may not equal a builtin code or alias.
  A collision is a config error: exit 2 in the CLI, and a diagnostic on the
  rule file in the LSP.

Overrides stay keyed by code under `lint.rules` (`suspect-config/src/lib.rs:61`).
Keys are resolved through the registry, so `lint.rules.operation-operationId:
off` and `lint.rules.oas-operation-missing-operationId: off` do the same thing.
`lint.rules` now officially covers every check. That is what the LSP already
does (`config_files.rs:750`). An unknown key gets a warning that names the
nearest code, and the warning never fails the run. Renaming the section to a
top-level `rules:` belongs to P2's schema work.

### The check pass

```rust
// crates/suspect-check/src/lib.rs (new)
pub struct Policy {
    pub checks: CheckSet,                       // --only; default all
    pub floor: Severity,                        // lint.min_severity / editor minSeverity
    pub overrides: BTreeMap<Box<str>, Level>,   // lint.rules, keyed as written
    pub design: Option<Level>,                  // lint.design
    pub recommended: bool,                      // lint.recommended (LSP key today)
    pub sdk: bool,                              // sdk-* + admission, see below
    pub strict_format: bool,                    // validate.strict_format
    pub linter: Arc<suspect_lint::Linter>,      // compiled once per policy
    pub extensions: ExtensionConfig,
    pub registry: Arc<Registry>,
}

pub struct Context<'a> { pub workspace: Option<&'a Arc<Workspace>>, pub policy: &'a Policy }

/// Every built-in check for one document, before policy. Sync; CPU only.
pub fn check_document(cx: &Context<'_>, doc: &LowDoc) -> Vec<Finding>;
/// TS rules for one document (Bun sidecar). Async.
pub async fn check_rules(host: &mut RuleHost, doc: &LowDoc) -> Result<Vec<Finding>, RulesError>;
/// Canonicalize, dedupe, override, suppress, floor. Pure.
pub fn apply_policy(findings: Vec<Finding>, doc: &LowDoc, policy: &Policy) -> Vec<Finding>;
/// CLI/CI entry: load, check, apply policy, then the baseline.
pub fn run(inputs: &[PathBuf], policy: &Policy, baseline: Option<&Baseline>) -> Report;
```

`check_document` routes by `doc.sniff_family()`, the way
`compute_diagnostics_raw` does today:

- syntax (`LowDoc::syntax_errors`);
- config schema (`suspect_config::schema::problems`);
- OAS 3.x (`validate_openapi_with`, which includes `$ref` checks);
- Swagger (`validate_swagger_low`);
- Arazzo (`validate_arazzo`);
- Overlay (`validate_overlay`, newly wired);
- load profile (`read_profile_lenient` + `check_against_plan`);
- lint (`policy.linter.run`, skipped when `!recommended` and no custom
  ruleset);
- markdown;
- extensions;
- admission when `policy.sdk`.

The CLI validate path stops at the first syntax error and skips semantic
checks (`commands/validate.rs:103-118`). The new function keeps that
short-circuit for the semantic checks, so a broken file does not produce
cascades. Lint still runs, because the LSP runs it today.

`OverlayDiagnostic` gains `range: Range<usize>`, because it has none today
(`suspect-overlay/src/model.rs:171-177`). `OverlayDoc` keeps only the root
`NodeRef` and parsed `ActionView`s (`model.rs:6-28`). So `ActionView` gains
`node: NodeRef<'d>` for its action item, and the root-level
`overlay-empty-actions` uses the `actions` value's range.

`apply_policy` runs in this order. The order is fixed and tested:

1. **Canonicalize.** Replace each code with its canonical code. A finding
   from an alias also takes the canonical default severity, so
   `operation-operationId` stops being an error.
2. **Dedupe.** Key on `(canonical code, document, pointer or range.start)`.
   Keep the finding from the owner check, else the first one. This does not
   suppress the alias rule up front: on Swagger 2.0, lint's
   `operation-operationId` has no validate counterpart, so it must survive.
3. **Overrides.** Use the override for the canonical code, else for an alias,
   else `lint.design` for `RuleClass::Design`, else keep the severity.
   `Level::Off` drops the finding.
4. **Category gates.** `RuleClass::Sdk` and `CheckId::Admission` findings are
   dropped unless `policy.sdk`. `lint.recommended = false` drops builtin-pack
   findings.
5. **Suppressions** (inline and `x-suspect-disable`, below).
6. **Floor.**

The baseline is applied after `apply_policy`, in the CLI only.

`policy.sdk` is on when codegen is configured. That means the resolved
config has a `codegen:` section with any key set (`CodegenSettings`,
`suspect-config/src/lib.rs:91`) or the manifest declares SDK targets.
`check --sdk` and `--no-sdk` override it. This puts the 2026-10-29 decision
into code. It also gives the editor admission refusals as diagnostics. They
use the `http-*` and `naming-*` codes and are shown only when `sdk` is on.

### LSP integration

`compute_diagnostics` (`diagnostics.rs:359`) becomes a thin wrapper:

```rust
let raw = suspect_check::check_document(&cx, &doc.low);
let kept = suspect_check::apply_policy(raw, &doc.low, &policy);
kept.iter().map(|f| to_lsp(f, &doc.low, &policy.registry)).collect()
```

The same wrapper serves the push path (`lib.rs:258`), the pull paths
(`pull.rs:101`, `134`), code actions (`actions.rs:113`) and the latency probe
(`latency.rs:79`). `to_lsp` maps `CheckId::{Lint, Markdown, Rules}` to source
`suspect-lint`, as `SOURCE_LINT` does today, and everything else to `suspect`.
It puts `summary`/`how_to_fix` in `data` for every code that has them. Today
only validate codes get them (`diagnostics.rs:127-134`).
`filter_at_least` and `config_files::apply_config` are deleted. Their work is
now steps 3, 4 and 6.

These parts must keep working unchanged:

- `State::cache_epoch() -> (content_epoch, policy_epoch)` (`state.rs:143`,
  `287`). It is captured when a computation starts, and `diag_cache` and
  `ws_diag_cache` key on it. Suppression comments and `x-suspect-disable` are
  document content, so the content epoch and per-document version already
  cover them.
- The `policy_epoch` bump (`lib.rs:171-175`). Today it compares
  `SuspectConfig` and `EditorConfig`. After the switch the server builds an
  `Arc<Policy>` at the same point and stores it in `State`, and the epoch
  bumps when the policy's inputs change. `Policy` implements `PartialEq` over
  its inputs: the config values plus a content hash of the ruleset file and
  TS rule files, not the compiled `Linter`.
- `republish_after_config_change` clearing `ws_doc_diag` explicitly
  (`lib.rs:195`), because its key `(identity, version)` does not include the
  policy.
- Off-lock computation on `spawn_blocking` and the generation check before
  publishing (`lib.rs:243-294`). `check_document` is sync and CPU-only for
  this reason.
- Result ids hash the post-policy set (`pull::diagnostics_result_id`). The
  set is still post-policy, so nothing changes.

One improvement comes with the switch. `lint_diagnostics` reads and compiles
a custom ruleset on every run (`diagnostics.rs:142-167`, `load_ruleset`).
`Policy.linter` compiles it once per policy epoch.

TS rules in the editor are opt-in through `rules.enabled` (P2.3 adds the key;
the default is off until P2). The server keeps one `RuleHost`, starts it
lazily, and runs `check_rules` after `check_document` on the same debounce.
It merges both sets before `apply_policy`. A timeout or worker error drops
the TS part for that run and logs it. It never blocks the built-in findings.
`RuleHost::reload` (`host.rs:308`) runs on a watched change to
`.suspect/rules/**`, and that change bumps `policy_epoch`.

### Suppression

**Inline (YAML only).**

```yaml
paths:
  /legacy:
    # suspect-disable oas-operation-missing-operationId -- owned by partner team
    get:
      responses: { "200": { description: ok } }
```

`# suspect-disable <code>[ <code>...] [-- reason]` suppresses the listed
codes, or aliases, for findings whose range starts inside the node that
begins on the next non-blank, non-comment line. That node is the pair or
sequence item together with its subtree.

The short version had a `suspect-disable-next-line` that suppresses one line.
I changed this because findings anchor on the value mapping, not on the key.
The petstore operation finding is at 8:7, which is the `summary:` line under
`get:`, so a line-based comment would have to sit somewhere no one would put
it.

`# suspect-disable-file <code>...` in the first comment block suppresses the
codes for the whole document.

Comments are CST nodes (`SyntaxKind::Comment`, `suspect-syntax/src/lib.rs:67`).
`suspect_check::suppress` collects them by walking `doc.inner().root()
.descendants()`. It finds the target node with
`descendant_for_byte_range` (the probe `navigation::node_at` already uses,
`suspect-lsp/src/navigation.rs:20-45`, moved to `suspect-low`).

**Extension (YAML and JSON).**

```json
{ "get": { "x-suspect-disable": ["oas-operation-missing-operationId"], "responses": {} } }
```

`x-suspect-disable: [codes]` on a mapping suppresses those codes for findings
inside that mapping. The suspect toolchain already uses `x-suspect-*` (for
example `x-suspect-cyclic`). The contract compiler skips `x-` keys
(`suspect-ir/src/contract/compile.rs:195`), so codegen output does not change.
The extension check gets a builtin schema for the key (a string array), so a
typo shows up as `extension-schema`.

Both forms obey these rules:

- `syntax-error` cannot be suppressed.
- An unknown code in a suppression gives a `suppression-unknown-code`
  warning.
- A suppression that matched nothing gives `suppression-unused` at hint
  level. It is off by default and turned on with
  `lint.rules.suppression-unused: warning`.
- Suppressed findings are counted, and `--format json` lists them under
  `suppressed` so a reviewer can audit them.

### Findings baseline

A key is computed per finding:

```text
key = canonical code + "\0" + document path relative to the baseline file + "\0" + pointer
```

The pointer comes from the producer when it has one: lint `Finding.path`
(`engine.rs:25`), `TsFinding.pointer` and `RefDiagnostic.path`. Otherwise
`apply_policy` computes it as `doc.node_at(range.start)` followed by
`path_from_root()` (`suspect-low/src/node.rs:381`, `Display` gives RFC 6901,
`pointer.rs:146`). A document-level finding uses the pointer `""`.

The key has no message hash, which differs from the short version. Messages
change wording between releases. The CLI even appends how-to-fix text to them
today (`commands/validate.rs:146-151`). A hash would invalidate every
baseline entry on a wording change. Instead, the file stores a count per key,
and a finding is new when the current count for its key exceeds the stored
count.

```json
{
  "version": 1,
  "tool": "suspect",
  "findings": [
    { "code": "oas-operation-missing-operationId",
      "document": "docs/demo/petstore.yaml",
      "pointer": "/paths/~1pets/get",
      "count": 1 }
  ]
}
```

Entries are sorted by `(document, pointer, code)` and the file ends with a
newline, so diffs stay small.

CLI:

- `suspect check --write-baseline .suspect-baseline.json` writes the file
  from the current post-policy findings and exits 0.
- `suspect check --baseline .suspect-baseline.json` drops baselined findings
  before the exit code is computed. In text output it prints `N baselined
  finding(s) hidden` on stderr.
- `check.baseline` in `.suspect.yaml` sets the default, so `ci` uses it
  without a flag. `ci --baseline` already means a git ref for `breaking`
  (`ci.rs:26`), so `ci` gets no new flag.
- SARIF output sets `partialFingerprints["suspect/v1"] = key`, and
  `baselineState` to `new` or `unchanged` when a baseline is in use.
- An unreadable or malformed baseline file exits 2.

The LSP does not apply the baseline. Editors show every finding, because the
person editing the code is the one who can fix it.

### Command surface

| Before | After |
|---|---|
| `suspect check FILE` = parse report, exit 0 on semantic errors | `suspect check FILE...` = every check, exit 1 on any error after policy and baseline |
| `suspect validate FILE` | `suspect check --only syntax,ref,validate,swagger,arazzo,overlay,load-profile,config,extension`; `validate` stays as a command that sets this filter |
| `suspect lint FILE` | `suspect check --only lint,markdown,rules`; `lint` stays as a command that sets this filter |
| `suspect rules check FILE` | still exists; also `check --only rules` |
| `suspect admission FILE` | unchanged; also `check --sdk --only admission` |
| `suspect ci` stages `validate`, `lint` | one stage `check`; `--stages validate,lint` maps to `check` with the matching `--only` |
| census (edges, cycles, workspace) in `check` | `suspect stats` gains `ref_edges`, `legal_cycles`, `illegal_cycles`, `workspace_docs`, `workspace_edges` |

`--only` takes `CheckId` names in kebab case. `check --parse-only` is
shorthand for `--only syntax,ref`. All the filtered views share
`apply_policy`, so `validate` gains the floor and `lint.rules` (the LSP's
behaviour), and `ci`'s validate stage gains `strict_format`.

If `.suspect/rules` contains rules and Bun is not available, `check` (but not
`--only` without `rules`) emits a `rules-runtime-unavailable` warning and
continues. `rules check` keeps exit 2.

Exit codes: 0 clean; 1 when an error-severity finding remains; 2 for usage,
config or baseline errors. This matches P3's planned enum.

## Compatibility and migration

- **`suspect check` changes meaning.** It now exits 1 on files that used to
  pass, and the JSON shape changes from `Vec<FileReport>` to the findings
  report. There is no alias, because the name is reused. The release notes
  say so. The census moves to `stats`. The only in-repo consumers I found are
  the `check_file` unit tests (`suspect-cli/tests/cli.rs:76-100`), `README.md:134`
  and `docs/LOAD-TESTING.md:218`, and all three are updated in the same PR.
- **Codes change for aliased lint rules.** `suspect lint` and SARIF now show
  `oas-operation-missing-operationId` instead of `operation-operationId`.
  JSON output adds `"alias_of"` for one release so scripts can match either
  name. GitHub code scanning will close the old alert and open one under the
  new rule id once.
- **Severities change for aliased rules.** On petstore, `suspect lint` exits
  0 instead of 1, because the canonical missing-operationId default is
  warning. A project that wants the old severity sets
  `lint.rules.operation-operationId: error`. The alias still works.
- **`lint.rules` and the floor now apply in `validate` and `ci`.** That
  matches what the editor already shows. An unknown severity name is exit 2
  everywhere, including the LSP, which shows it as a `suspect.config`
  diagnostic instead of ignoring it.
- **New findings appear.** The CLI gains Arazzo, Swagger, load-profile,
  markdown, extension and config checks. Everyone gains Overlay structure
  findings. `md-*` findings default to hint, so the default floor of hint
  shows them and they never fail a run.
- **LSP.** The code `syntax` becomes `syntax-error`. VS Code users see no
  other change apart from the deduplication.
- **Rust API.** `suspect_config::{SeverityName, parse_severity}` are
  deprecated re-exports for one release. `suspect_lint::Severity::from_policy`
  is removed. The crates are internal (P5.3), so no external users are
  expected.

## Implementation plan

Each step leaves `cargo xtask ci` (or today's CI jobs) green and is
releasable on its own.

1. **`suspect-diag` crate; severity moves.** New crate with `Severity`,
   `Level`, `parse_level`, `Finding`, `CheckId`, `RuleInfo`, and the `text`,
   `json` and `sarif` renderers (ported from `suspect-cli/src/sarif.rs`).
   `suspect-config/src/severity.rs` re-exports from it.
   `suspect_cli::output::Finding` converts to and from `suspect_diag::Finding`,
   so command output is byte-identical. Tests: the severity tests move, the
   SARIF 2.1.0 test moves, and a golden text test runs on petstore.
   Estimate: 1 day.
   **Done.** Deviations: `Finding` has an extra `position: Option<Position>`
   for producers that know a line but no byte offset (`breaking`, CLI
   findings converted from `output::Finding`), and `CheckId` has a
   `Command` variant for command-local findings outside the check pass.
   CLI JSON output keeps its own serializer (`"severity": "Warning"`), because
   `render::json` uses the lowercase names and would change the bytes; it
   switches in step 5. SARIF `endLine`/`endColumn` and
   `partialFingerprints` are not added yet (they change output; step 8).
   `SeverityName`/`parse_severity` are kept as wrappers over `parse_level`
   but not yet `#[deprecated]`, since the LSP's `editor_config.rs` (being
   rewritten by P2) still calls them. Goldens:
   `crates/suspect-cli/tests/finding_output.rs` covers text, JSON and SARIF
   for `validate` and `lint`.
2. **Rule tables and the registry.** `rules()` in validate (from
   `guidance.rs` plus default severities), lint (from packs, plus
   `Rule::builtin`), arazzo, overlay, load profile, admission.
   `suspect_check::Registry` gets no aliases yet. Delete `from_policy`.
   Tests: every code a fixture corpus emits is registered, no code is
   registered twice, every `RuleInfo` has a summary. Estimate: 1 day.
   **Done.** The `suspect-check` crate is created here with only
   `registry.rs` (`Registry::builtin`, `from_tables`, `canonical`, `info`,
   `rules`; it implements `render::RuleLookup`). Tables:
   `suspect_validate::rules()` (74 codes; `swagger-*` are
   `CheckId::Swagger`, `sdk-*` are `RuleClass::Sdk`),
   `suspect_lint::builtin_rules()`, `suspect_arazzo::rules()`,
   `suspect_test::load::rules()`, `suspect_codegen::admission::rules()`.
   Deviations: the Overlay table lives in `suspect-check`, because
   `suspect-overlay` is a publishable foundation crate (P5) and cannot
   depend on the unpublished `suspect-diag`; it moves when `suspect-diag`
   joins the public set. `canonical` returns `Option<&str>` (no
   multi-target aliases until step 4). `no-$ref-siblings` is defined twice
   in the OAS pack (error on 3.0, warning on 3.1+) and is registered once
   from its first definition. Admission lists only the 18 codes with
   guidance in `admission.rs`; the ~200 open-ended `http-*` codes of the
   HTTP admission layer and the contract compiler's codes need a table
   before step 9, so the corpus test does not run admission. Every ruleset
   rule clears `Rule::builtin` (a ruleset rule always has its own `then`,
   so a severity-only override cannot be written yet). Three validate
   codes had no guidance (`oas-schema-instance-invalid`,
   `oas-tag-parent-cycle`, `oas-tag-parent-unknown`); they now have a
   summary and how-to-fix, so `validate` appends a `how to fix:` line for
   them. The corpus test adds five broken fixtures under
   `crates/suspect-check/tests/fixtures/`, because the repository had no
   Swagger, Overlay or failing Arazzo/load-profile fixture files. The
   crate-graph test (`tests/graph.rs`) lands here too. The
   `overlay-action-missing-description` (overlay) /
   `overlay-action-description` (lint) pair is an alias candidate once
   step 3 wires `validate_overlay`.
3. **`suspect-check` with the LSP battery, LSP switched.** Move
   `markdown.rs` to `suspect-lint`, `config_schema::problems` to
   `suspect-config`, the extension check to `suspect-check`, and `node_at`
   to `suspect-low`. Implement `check_document` and `apply_policy` (steps 3,
   4 and 6 only). Switch every `compute_diagnostics` caller. Wire
   `validate_overlay` (with ranges). Tests: an LSP golden test over
   `suspect-cli/tests/lsp_session.rs` fixtures shows the same set as before,
   except for the new overlay findings; the `diagnostics.rs` unit tests move.
   Estimate: 1.5 days.
   **Done.** `suspect_check::{check_document, apply_policy, Policy,
   CheckSet, Context}`; the per-producer mappers are
   `suspect_check::checks::*`. `suspect_lsp::diagnostics` is now
   `compute_diagnostics` (builds `Policy::from_settings`, runs both, maps
   with `to_lsp`), `findings` and `to_lsp`; `compute_diagnostics_raw`,
   `filter_at_least`, the per-producer `*_diagnostics` functions and
   `config_files::apply_config` are gone, and every caller (push, both
   pulls, fix-all, latency probe, bench, showcase) goes through
   `compute_diagnostics`. Golden:
   `crates/suspect-cli/tests/lsp_diagnostics_golden.rs`
   (`tests/fixtures/golden/lsp-diagnostics.txt`, the `lsp_session.rs`
   workspace plus `docs/demo/petstore.yaml` and the registry's broken
   fixtures under a hint floor); recorded before the switch, the only diff
   after it is three Overlay lines (`overlay-empty-actions`,
   `overlay-update-with-remove`, `overlay-action-missing-description`).
   The `diagnostics.rs` tests moved to `suspect-check/tests/check_document.rs`
   and `policy.rs`; the floor tests are `apply_policy` tests. `LowDoc::node_at`
   is in `suspect-low` (`navigation::node_at` forwards to it).
   `markdown.rs` is `suspect_lint::markdown` (with a `rules()` table; the
   LSP no longer depends on pulldown-cmark). Configuration validation is
   `suspect_config::schema::problems` (`FileKind`, `kind_of`, `kind_for`,
   `field`, `children`, `closest`, `problems`, `document_problems`;
   `Problem` gains `code`, and the manifest-policy warning is a problem
   too); LSP hover and completion stay in `config_schema.rs`.
   `OverlayDiagnostic` gains `range` and `ActionView` gains `node`.
   Deviations: only the schema half of `extensions_config.rs` moved
   (`suspect_check::checks::extensions`: the builtin schema table, the
   `extensions.<key>.schema`/`contexts` parser, the check); `before`/`after`
   ordering stays in the LSP with `keys::Context`, because the formatter
   uses it. No syntax short-circuit yet: the editor has always run every
   check on a broken file, and the golden requires the same set; the CLI's
   short-circuit is step 5's (a `CheckSet` or policy flag). The LSP builds
   a `Policy` per computation from the document's `Settings` rather than
   storing an `Arc<Policy>` in `State`; `ResolvedCache`, `cache_epoch`
   and `policy_epoch` are unchanged, and a compiled ruleset is cached per
   process by path, modification time and length, so it is compiled once
   and an edited ruleset still applies without a configuration reload.
   `policy.sdk` defaults to on (today's editor behaviour) until step 9
   detects codegen. Fix guidance now rides in `data` for every registered
   code that has it, tagged with the check name. The syntax code stays
   `syntax` until step 4.
4. **Aliases, dedupe, canonical severity.** Alias table v1, step 1 and 2 of
   `apply_policy`, `syntax` to `syntax-error`. Tests: petstore produces
   exactly one finding per operation for missing `operationId` in the LSP
   path; an override through an alias and through the canonical code (the
   canonical one wins); a redefined custom rule keeps its code; Swagger
   keeps the lint finding. Estimate: 1 day.
   **Done.** `suspect_check::registry::ALIASES` holds table v1 plus the
   Overlay pair `overlay-action-missing-description` (canonical, overlay
   check) / `overlay-action-description` (lint), which the LSP golden
   shows at the same range on the action mapping. `Registry` gains
   `with_aliases`, `alias_targets` and `resolve`; `canonical` returns
   `None` for an alias. The registry also gains `syntax-error`,
   `suspect.config`, `config-manifest-policy`, `extension-schema` and the
   `md-*` table. `apply_policy` steps 1-2: a builtin lint finding with an
   alias code takes the canonical code and default severity; a
   multi-target alias takes the target reported at the same position,
   else keeps its own code; overrides look up the canonical code, then
   its aliases. Deviation: dedupe merges only findings that reached one
   canonical code from different codes (key: canonical code, document,
   range start, else pointer), so repeats of one code at one spot (two
   undeclared parameters on one path item) are all kept. Tests:
   `crates/suspect-check/tests/aliases.rs` (petstore once per operation,
   alias vs canonical override, redefined custom rule, Swagger keeps the
   lint finding, multi-target, overlay pair, `syntax`) and the `policy.rs`
   unit tests; the LSP golden asserts the petstore case on the editor path.
   The regenerated golden drops 129 `operation-tag-defined`, 2
   `operation-operationId`, 2 `arazzo-step-operation` and 1
   `overlay-action-description` duplicates, renames the two Swagger
   `operation-operationId` findings (error -> warning) and `syntax`. The
   CLI is unchanged until step 5, so `finding_output.rs` goldens do not
   move. `docs/capture/showcase_data.json` (written by the `showcase`
   example) still shows the old codes.
5. **CLI on `suspect-check`.** New `check` command, `--only`,
   `--parse-only`, `--sdk`/`--no-sdk`; `validate` and `lint` become filters;
   `ci` gets a `check` stage; `stats` gains the census; JUnit renderer; docs
   (`docs/CLI.md` regenerated by `tests/cli_reference.rs`, README,
   LOAD-TESTING). Tests: `suspect-cli/tests/check.rs` covers exit codes and
   filters, and `check_parity.rs` runs the CLI and the LSP harness
   (`tests/support/editor.rs`) on the same fixtures and compares
   `(code, severity, range)`. Estimate: 1.5 days.
6. **TS rules in `check` and (opt-in) in the LSP.** `check_rules`, registry
   collisions, `rules-runtime-unavailable`, the LSP `RuleHost` lifecycle and
   epoch bump. Tests: bun-gated integration test in `suspect-check/tests`,
   plus an LSP test with a stub host. Estimate: 1 day.
7. **Inline and extension suppression.** `suspect_check::suppress`, the
   `x-suspect-disable` builtin schema, unknown-code and unused-suppression
   findings, and JSON `suppressed`. Tests: YAML comment on a pair, on a
   sequence item, file-level, in JSON via the extension; alias names in
   suppressions; `syntax-error` is not suppressible. Estimate: 1 day.
8. **Baseline.** `--write-baseline`, `--baseline`, `check.baseline`, SARIF
   `partialFingerprints`/`baselineState`. Tests: write then check exits 0;
   adding one finding exits 1 and shows only that one; moving an operation to
   a different line keeps it baselined; renaming the path does not.
   Estimate: 0.5 day.
9. **SDK category and admission as diagnostics.** `policy.sdk` detection,
   admission findings in `check_document` behind the `sdk` feature. Tests:
   `sdk-operation-missing-id` is absent without `codegen:` and present with
   it; refusals appear in the editor when configured. Estimate: 0.5 day.

Total: about 9 days. The short version estimated 4-5 days. The difference is
the LSP-only checks that have to move (step 3), the parity harness (step 5)
and TS rules in the editor (step 6).

## Testing

- **Unit** (`suspect-diag`): parser spellings and rejections, `Severity`
  order, the renderers against golden files, and a SARIF schema check.
- **Unit** (`suspect-check`): `apply_policy` order, one test per step. Alias
  resolution and multi-target aliases. Dedupe keeps the owner. Suppression
  scopes. Baseline counting.
- **Registry completeness** (`suspect-check/tests/registry.rs`): run every
  check over `crates/*/tests/fixtures` and the OpenAPI corpus used by
  `suspect-cli/tests/corpus_suite.rs`, and assert every emitted code is
  registered.
- **Crate graph** (`suspect-check/tests/graph.rs`): `cargo metadata` shows
  `suspect-diag` with only its allowed deps, and only `suspect-lsp` and
  `suspect-cli` depending on `suspect-check`.
- **Parity** (`suspect-cli/tests/check_parity.rs`): for each fixture, CLI
  `check --format json` equals the LSP harness's published diagnostics after
  mapping to `(code, severity, start, end)`.
- **Dedupe regression**: `docs/demo/petstore.yaml` gives exactly one finding
  per operation for missing `operationId` in `check`, `lint`, `validate` and
  the LSP, and no `sdk-operation-missing-id` without `codegen:`.
- **Editor host smoke**: extend `npm run test:host:smoke`
  (`editors/vscode`) to open petstore and assert one missing-operationId
  problem per operation, and that a `# suspect-disable` line removes it.
- **CI**: no new job. The bun-gated TS test runs in the existing
  `rules-runtime` job (`.github/workflows/platform.yml:248`). `cargo deny` covers the two new crates.

## Alternatives considered

- **Put the orchestrator in `suspect-lsp`.** The CLI already depends on it,
  so no new crate would be needed. It lost because CLI-run checks would
  live in a crate built around `tower-lsp`, and `suspect-lsp` is already the
  largest crate.
- **Put it in `suspect-validate`.** Validate would depend on lint, rules,
  test and codegen, and every consumer of plain OAS validation would pay for
  that.
- **Drop alias findings by ownership before running** (disable
  `operation-operationId` whenever validate runs). Simpler, but wrong on
  Swagger 2.0, where validate has no missing-operationId code. It also
  breaks `lint` used alone. Post-hoc dedupe on canonical code plus location
  handles both.
- **Make lint codes canonical** (Spectral names). Familiar to Spectral users,
  but lint does not resolve `$ref`s across documents, has no how-to-fix
  text, and has no code actions. Aliases keep the Spectral names working
  in config and suppressions.
- **Baseline keyed on line numbers.** Every edit above a finding would make
  it "new". Pointers survive reformatting and reordering.
- **Baseline with a message hash** (the short version). Wording changes would
  invalidate baselines. Counting per key is enough.
- **Line-based `suspect-disable-next-line`.** Findings anchor on value
  mappings, not keys (petstore 8:7), so the comment would have to sit inside
  the node. Node scope is what users expect.
- **A `fix` field on `Finding` now.** No built-in producer creates fixes.
  `actions.rs` derives them from codes. Deferred until TS fixes are wired.

## Risks

- **Lint users see different codes and lower severities.** Some pipelines
  that failed on `operation-operationId` will pass. Mitigation: release
  notes, `alias_of` in JSON, and the override recipe. SARIF alert churn
  happens once.
- **Editor latency.** The new work per run is canonicalization, dedupe,
  suppression scanning and pointer computation for findings without a
  pointer. Pointer computation is O(depth) per finding, and suppression is
  one walk over comment nodes. The LSP bench (`suspect-cli/tests/lsp_bench.rs`)
  on the 63k-line spec must stay within 5% of today. The bench gates step 3
  and step 7. Compiling the ruleset once per policy removes per-run work, so
  the net change should be small.
- **New findings in the CLI** (Arazzo, markdown, extensions) may fail
  existing CI runs on Arazzo documents. These are errors the editor already
  shows. Mitigation: the baseline lands one step later; projects can write
  one on upgrade.
- **TS rules in the editor** add a Bun process per server. Opt-in until P2
  decides the default.
- **Canonicalization hides a real difference.** For example,
  `operation-tag-defined` may match cases `oas-tag-undeclared` does not.
  Mitigation: v1 aliases only pairs reproduced at the same position, and
  the dedupe test corpus asserts that no alias fires where its canonical
  code does not, on the corpus.

## Open questions

1. **Canonical default for missing `operationId`: warning (validate) or error
   (lint)?** Recommended: warning. OpenAPI makes `operationId` optional, and
   SDK naming needs are covered by the separate `sdk-*` and admission
   category.
2. **Should `md-*` markdown findings be in the default `check` run?**
   Recommended: yes, at hint, which never fails a run and matches the
   editor.
3. **File-level suppression: keep `# suspect-disable-file`?** Recommended:
   yes. Generated or vendored specs need it, and it costs one extra comment
   pattern.
4. **Where is `x-suspect-disable` honoured inside schemas?** Recommended: on
   any mapping. Inside `properties` it is then a property name, which is
   legal but unlikely. Document the limitation rather than special-case
   schema positions.
5. **Bun missing while `.suspect/rules` exists: warning or exit 2 in
   `check`?** Recommended: a `rules-runtime-unavailable` warning finding.
   `lint` never ran TS rules before, so exit 2 would break working setups.
   `rules check` keeps exit 2.
6. **Should the editor dim baselined findings?** Recommended: no. The editor
   ignores the baseline. Revisit if users ask.
7. **Do the v2 alias candidates (`path-params`, error-response rules) get
   decided in this project?** Recommended: separately, one PR per pair, with
   a corpus diff attached.
