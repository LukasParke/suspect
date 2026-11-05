# Speakeasy `openapi-generation` review — patterns for the suspect engine

A review of [speakeasy-api/openapi-generation](https://github.com/speakeasy-api/openapi-generation)
(commit series of September 2026, AGPL-3.0), the Go generator behind
Speakeasy's SDKs and — since the September 2026 open-source release —
Google's Gemini client libraries. It produces SDKs (Go, Python,
TypeScript, Java, C#, PHP, Ruby, Unity), CLIs, Terraform providers,
Postman collections, and MCP servers from OpenAPI documents.

**Licensing note.** The generator is AGPL-3.0. This review and the
changes it landed in this repository share *architecture and ideas*
only: no source, template text, or test fixtures were copied or
mechanically translated. Ideas and interfaces are not copyrightable;
code is. Keep it that way — suspect's engine must remain free of AGPL
lineage.

## Their architecture in one paragraph

`cmd/generate` loads and validates the OpenAPI document, builds a
target-neutral SDK AST (`internal/ast/`, ~1.1 MB of Go), then hands it
to the selected target: a directory of TypeScript files
(`config.ts`, `main.ts`, `examples.ts`, `features.ts`) that declare
configuration fields, feature support with versions, compile commands,
and the templating logic. Rendered output passes a per-target formatting
pipeline and optional compile checks. Test coverage comes from focused
OpenAPI fragments composed into specs, runtime tests of generated SDKs
against local `httpbin` + test-service containers, and tracked review
SDKs (`zSDKs/`) whose diffs make template changes reviewable.

## What suspect already matches (no action)

| Their pattern | Our counterpart |
|---|---|
| Target-neutral AST before templates | `suspect-ir` contracts + `http_contract` admission layer, shared by all 12 backends |
| `gen.yaml` config with `configVersion` | `sdk_defaults` v1 policy config, compatibility profiles (versioned, kebab-case) |
| `zSDKs/` tracked review SDKs; deterministic regeneration regardless of license election | Pinned byte fixtures across all 12 backends; byte-stability hashes |
| Pre-generation validation gates | `suspect-validate` battery + `http_contract` admission diagnostics (source-located, fail-loud before artifacts) |
| Pagination/retries/SSE/webhooks/OAuth as generator features | M3–M6 golden-defaults program: same feature set, planned in the engine rather than templated |
| Runtime tests of generated SDKs against stubbed services | Per-backend behavioral tests (opt-in `#[ignore]`, CI-gated) |

The structural agreement is strong: both engines put a strict
admission/validation layer in front of generation and keep per-language
logic out of the core. Notably, the OpenRouter published SDKs we diffed
in the compatibility work were Speakeasy output — the drop-in plan we
produced against it validated our feature set against theirs.

## Adopted (landed with this review)

### 1. The `Rule` interface: every diagnostic carries fix guidance

Their validation rules are structs with `ID()`, `Category()`,
`Summary()`, `Description()`, `HowToFix()`, `Link()`,
`DefaultSeverity()`, and `Versions()` — a linter whose every finding
answers "what is this?" and "how do I fix it?".

Adopted: `suspect_validate::Diagnostic` now carries stable `summary`
and `how_to_fix` fields, resolved per code from a central table
(`guidance.rs`) covering all ~70 battery codes. The CLI `validate`
output appends a `how to fix:` line; the LSP rides the guidance in the
diagnostic `data` payload, where quick-fix providers can consume it.

### 2. Missing-error-response as a generation-quality rule

Their `generator-missing-error-response` (Hint) flags operations whose
`responses` carry neither `default` nor a 4XX/5XX entry — "makes client
error handling predictable and consistent".

Adopted: `oas-operation-no-error-response` (Info) in the operations
check group, with the same semantics and guidance.

### 3. The feature matrix as a verified artifact, not documentation

Their `features.ts` per target declares every supported feature with
the version that shipped it — a machine-readable capability matrix that
docs and READMEs render from.

Adopted: `crates/suspect-codegen/src/features.rs` — a feature registry
(7 features × 12 backends) where every claim names the acceptance test
file that proves it. The sync test (`tests/feature_manifest.rs`)
verifies claims against evidence in both directions (a claim without
evidence fails; an evidence file without a claim fails) and regenerates
`docs/SDK-FEATURES.md` byte-for-byte (`SUSPECT_REGEN_FEATURES=1`).

First-day findings from the manifest itself: C# implements
environment-variable credentials but has no acceptance test; Go emits
the ua/v1 attribution constants but has no attribution test. Both are
recorded as unclaimed (test debt) rather than claimed — the matrix
cannot be flattered.

## Candidate patterns — status

1. **Naming-collision rules after naming** — LANDED as backend-independent
   cross-convention analysis in the admission API: operationIds and
   component schema names that agree after separator-and-case folding are
   flagged (`naming-method-collision` / `naming-model-collision`, advice)
   because they collapse to the same identifier under camel, Pascal, and
   snake conventions alike. Identical operationIds remain the hard
   `DUPLICATE_OPERATION_ID` refusal. Per-backend group+method collision
   checking (their richer form) still belongs behind the public admission
   API once group-based naming exists.
2. **A public admission API on the engine** — LANDED:
   `suspect_codegen::admission::{review, review_selected}` returns one
   `AdmissionReport` (per-operation verdicts, located findings with kind/
   summary/how_to_fix) composing shared admission, incoming admission,
   contract limitations, and naming analysis. Consumers: the
   `suspect admission` CLI pre-flight command (CI-ready, JSON or text) and
   the LSP `suspect/generationContract` custom request (live-document
   verdicts in LSP coordinates).
3. **Fragment/overlay test architecture** — LANDED for admission:
   `tests/fragments/` holds one focused, self-describing OpenAPI document
   per tricky construct with an enforced `# INVARIANT:` header;
   `tests/fragments.rs` pins each fragment's admission verdict and fails
   on unregistered fragments. Extending the fragment convention to
   per-backend behavioral suites remains open.
4. **Toolchain manifests for native gates** — LANDED:
   `suspect_codegen::toolchain` declares per-tool version command,
   extraction regex, minimum version, and install guidance;
   `probe_status`/`guidance` replace hand-rolled checks (wired into the Go
   and Java gates). Remaining gates adopt incrementally.
5. **MCP/CLI targets** — not attempted: a new emitter is a program-scale
   effort, not a pattern adoption. When suspect grows non-SDK targets, the
   12-backend IR makes this a new consumer of the same contracts.

## Rejected for suspect

- **TypeScript-in-the-loop target configuration.** Their target contract
  is TS code executed by the generator. It is a proven plugin boundary,
  but it adds a JS runtime to generation; our Rust-native backends are
  single-binary, AOT-friendly, and already share admission through
  `http_contract`. Adopting the *interface shape* (config + feature
  declarations per target) without the runtime is what the feature
  manifest does.
- **Telemetry** (`internal/analytics`) and the private snapshot
  companion. Suspect's determinism guarantees are enforced by pinned
  bytes in-repo; adding phone-home or private infrastructure would
  weaken that property.
- **License-election machinery.** Relevant to their commercial model,
  not to a engine that emits unlicensed, consumer-owned output.
