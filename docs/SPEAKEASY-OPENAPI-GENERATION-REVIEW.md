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

## Candidate patterns (not yet landed, ranked)

1. **Naming-collision rules after naming** (`duplicateoperationname`,
   `duplicatemodelnamespace`, `operationmethodnameconflictchecker`).
   Their validator detects method-name collisions *after* naming rules
   (group + method, sanitization), not just duplicate `operationId`s.
   Our per-backend planners resolve collisions internally, but a
   spec-side warning that predicts the collision would surface it at
   authoring time. Requires exporting enough naming logic from the
   engine to be honest — do it with the public admission API (below).
2. **A public admission API on the engine** (`HttpContract` is
   `pub(crate)` today). This unlocks: the LSP `suspect/generationContract`
   request (hover shows the real admission verdict per operation), the
   collision rules above, and CI pre-flight for spec authors. Their
   `cmd/validate` standalone binary is the same idea.
3. **Fragment/overlay test architecture.** One focused, self-describing
   OpenAPI fragment per edge case (22 `primary` fragments: self-recursive
   additionalProperties maps, nullable-datetime allOf, oneOf const int32,
   …), composed per variant/target, with overlays only for genuinely
   target-specific behavior. Our per-backend test documents duplicate a
   lot of fixture YAML; a fragments layer would reduce that and make each
   edge case's intent explicit. Medium effort, benefits every future
   feature.
4. **Toolchain manifests for native gates.** Their compile config
   declares dependency checks (command, version regex, `minVersion`,
   `installDocumentation`). Our opt-in native gates could report
   "missing tool + install instructions" instead of requiring contributors
   to read the ignore message. Small, nice DX win.
5. **MCP/CLI targets.** They generate MCP servers and CLIs from the same
   AST. When suspect grows non-SDK targets, the 12-backend IR makes this
   a new consumer of the same contracts rather than a new pipeline.

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
