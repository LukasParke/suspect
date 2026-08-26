# Folding charte.rs into suspect — Architecture Plan

> Sibling plans: `docs/ARBITER-FOLD-PLAN.md` (traffic plane: capture,
> replay, TLS, credentials, inference) and `docs/TS-RULE-RUNTIME.md`
> (TS custom rules on a Bun sidecar).

## 0. Goal

Merge charte.rs (tree-sitter fact extraction from 14 web-framework sources
across 6 languages) into suspect, aligned on **suspect primitives**, and wire
code-level fact extraction end-to-end into the LSP, Lint, Validation, and Test
engine. The product outcome: **OpenAPI rules lint the code where the API is
written** — a rule violation in the spec surfaces as a diagnostic on the
handler that implements it, and an undocumented behavior in the code surfaces
as a diagnostic on the spec. Retain suspect's quality bar (determinism,
benchmarks, clippy-clean, golden tests) and charte.rs's depth (provenance,
confidence, fixture-first adapters).

### Non-goals

- No behavioral changes to existing suspect crates' public APIs beyond
  additive extensions.
- charte's CLI, sdk/mcp/serve/context_pack/ledger commands are not ported
  (suspect has its own surfaces; candidates for later separate features).
- No runtime traffic capture (that is Arbiter's domain).

---

## 1. The central design: dual-source union model

Today suspect treats the spec as the only ground truth. After the fold there
are two evidence sources for every API element:

```text
  openapi.yaml (spec truth)          src/** (code truth)
        │                                  │
        ▼                                  ▼
   IrSpec (existing)  ←──────────  facts → IrSpec + Provenance
        │                                  │        (suspect-code)
        └────────────┬─────────────────────┘
                     ▼
            UNION evaluation graph
                     │
        ┌────────────┼────────────────┐
        ▼            ▼                ▼
     Lint         Validate        Test engine
  (projected)   (projected)    (handler coverage)
```

**Key simplification over charte.rs:** charte has its own parallel IR
(`ApiDocument`) and renderer. In the fold, the IR is *suspect's* `IrSpec`.
Code facts lower into an `IrSpec` plus a `ProvenanceIndex` — so every
downstream consumer (validate, lint, diff, codegen, gateway, test) works on
code-derived specs **with zero changes**, exactly like `IrSpec::from_workspace`
already does for YAML. The renderer survives only as an emitter for
`suspect code gen` output and for lossless YAML insertion in code actions.

### The killer mechanism: union evaluation + projection

The lint engine already evaluates JSONPath `given` expressions against a
document. The fold adds a second document kind — **fact-space** — the
deterministic facts JSON (charte's facts-output format, schema-versioned).
Rules declare a target: `spec` (default, existing behavior) or `facts`.

Findings are then **projected**:

| Finding origin | Evaluation document | Projection target |
|---|---|---|
| Spectral rule fires on spec element | spec LowDoc (unchanged) | code spans via ProvenanceIndex (method, path) → `Vec<SourceSpan>` |
| Rule fires on fact-space element | facts JSON | directly via the fact's `SourceSpan` |
| Union rule (element in code, missing in spec) | union graph | **on the code**: "returned 404 is undocumented — add to spec"; and on the spec: "documented 404 not produced by any handler" |

This is the reconciler stance: neither side "wins". Every existing
Spectral-ecosystem rule instantly becomes a code linter with no rule-engine
changes — only the reporting layer gains projection. New rule dimensions
become possible because fact-space carries signals the spec cannot
(middleware evidence, handler response construction, security metadata).

---

## 2. Crate plan

### New: `suspect-code` (the folded charte.rs)

Single crate, module tree mirroring charte's layout so the port is
mechanically diff-able against the source:

```text
crates/suspect-code/
  src/
    lib.rs            error (thiserror), schema_version, re-exports
    span.rs           SourceSpan (file, bytes, lines, node_kind, reason)
    confidence.rs     Exact | Inferred | Partial | Unknown
    facts.rs          ExtractionFact enum — ported verbatim in shape:
                      Route, RouteConstraint, Security, Handler, Middleware,
                      Parameter, RequestBody, Response, Schema, Tag,
                      Description, Deprecation
                      + FactIdAllocator (deterministic zero-padded ids)
    workspace.rs      discovery: ignore rules, manifests, language detect
    parser/           tree-sitter registry, ParsedFile, parse diagnostics,
                      content-hash incremental cache
    adapters/         mod.rs (FrameworkAdapter trait, AdapterContext,
                      registry) + rust/{axum,actix}, typescript/{express,
                      fastify,hono,nextjs,nestjs,sveltekit}, python/{fastapi,
                      flask}, go/{mux,chi}, java/{jaxrs,spring},
                      csharp/aspnet
    lower.rs          facts → suspect_ir::IrSpec  (+ ProvenanceIndex)
    provenance.rs     ProvenanceIndex: (method, path) → Vec<SpanEntry>
                      ranked by confidence; (schema name) → Vec<SpanEntry>
    render/           OpenAPI 3.1 emitter (JSON/YAML) for `suspect code gen`
                      and code-action insertion snippets
    diagnostics.rs    CHARTE_RS_* codes renamed SUSPECT_CODE_*; stable codes
    facts_json.rs     facts → deterministic JSON document (fact-space) for
                      the lint engine
  tests/              golden_* per adapter (ported from charte, see §4)
  test-data/          fixtures ported verbatim from charte
```

Conventions aligned to suspect: `#![deny(missing_docs)]`, thiserror,
serde with stable ordering (`BTreeMap`), deterministic fact IDs, byte-offset
provenance everywhere, `#[must_use]`, workspace lint level. Tree-sitter
grammars become workspace dependencies (6 grammar crates + tree-sitter core —
charte already pins compatible versions).

### Modified crates (additive)

| Crate | Change |
|---|---|
| `suspect-ir` | `IrSpec::from_code(facts) -> (IrSpec, ProvenanceIndex)` constructor; no changes to existing types |
| `suspect-lint` | `RuleTarget { Spec, Facts }` on compiled rules; `project(findings, provenance) -> Vec<ProjectedFinding>`; union evaluation mode |
| `suspect-lsp` | State gains `code: CodeWorkspace` (parsed trees, facts, provenance); diagnostics push into code documents; cross-resolvers (§5) |
| `suspect-validate` | `--with-code`: cross-checks spec responses/params/security against facts; violations carry projection spans |
| `suspect-test` | plan/step provenance (`x-suspect-handler`); handler-coverage report (handlers with zero test coverage) |
| `suspect-cli` | `suspect code gen|check|sync|lint`; `--project` flags on `lint`/`check` |
| `suspect-codegen` | `diff_specs(spec_ir, code_ir)` — already exists, no change; `sync` composes it |

---

## 3. The port contract (retaining charte's quality)

**Goldens are the contract.** charte's fixture-first discipline transfers:

1. Port `test-data/` fixtures verbatim (anonymized real-world projects per
   framework, including unsupported-pattern cases).
2. Port `tests/golden_*.rs` per adapter: expected facts, diagnostics, and
   OpenAPI JSON committed as snapshots.
3. An adapter is *ported* only when its goldens pass byte-identically
   (modulo the `CHARTE_RS_` → `SUSPECT_CODE_` code rename) and its fixture
   README documents supported surface + limitations.

**Port order** (by value to suspect's audience, goldens gating each):

| Wave | Adapters | Rationale |
|---|---|---|
| 1 | core (facts/span/confidence/workspace/parser) + rust/axum, rust/actix | suspect's home ecosystem; exercises the full pipeline |
| 2 | typescript/express, python/fastapi, python/flask | highest-population frameworks |
| 3 | typescript/fastify, hono, nestjs, nextjs, sveltekit | TS breadth |
| 4 | go/mux, go/chi (module-graph branch) | charte's reference adapter; hardest infra |
| 5 | java/spring, jaxrs, csharp/aspnet | enterprise coverage |

**Discarded from charte** (suspect-primitives alignment): `ApiDocument` IR
(replaced by lowering into `IrSpec`), CLI surface, sdk/mcp/serve/
context_pack/ledger/ingest commands, its lint command (superseded by
suspect-lint with fact-space), coverage command (superseded by R10 quality
scoring, which gains a new *code-agreement* dimension fed by facts).

---

## 4. The killer feature, end to end

Scenario: repo with `openapi.yaml` + ruleset.yaml + axum handlers.

```text
1. suspect-lsp opens the workspace
   ├─ parses spec (existing LowDoc pipeline, <10ms)
   └─ suspect-code extracts facts from src/** (tree-sitter, incremental)

2. Ruleset defines (existing Spectral syntax, new optional target field):
   rules:
     info-description:        { given: $.info, then: {field: description} }
     operation-description:   { given: $.paths[*][*], then: {field: description} }
     responses-documented:    { given: $.paths[*][*].responses, ... }
   union rules (new, evaluated on spec ∪ facts):
     handler-returns-undocumented-status
     spec-declares-unimplemented-operation

3. Evaluation
   ├─ spec rules → findings on spec LowDoc (existing)
   ├─ fact rules → findings on facts JSON (same JSONPath engine)
   └─ union rules → findings on the merged graph

4. Projection (new, the fold's payoff)
   finding on GET /pets ──ProvenanceIndex──▶ src/routes/pets.rs:42
   finding on fact (response 404, handler X) ──SourceSpan──▶ src/routes/pets.rs:58

5. LSP publishes diagnostics INTO the code editor:
   src/routes/pets.rs:42  [warning] operation-description:
     GET /pets lacks a description (rule: operation-description)

6. Code actions:
   ├─ "Add description" → inserts doc comment at handler (template)
   └─ "Document in spec" → lossless YAML insert into openapi.yaml
      at the correct byte position (suspect-low CST edit)
```

Navigation closes the loop: `textDocument/definition` on
`/paths/~1pets/get` → handler span; hover on the handler → operation
summary, lint status, spec fragment; `documentLink` in code → spec.

---

## 5. LSP integration (multi-root)

`State` gains `code: CodeWorkspace`:

- `trees: HashMap<Uri, (tree_sitter::Tree, content_hash)>` — incremental
  reparse on change (tree-sitter `Tree::edit`), full reparse on miss
- `facts: Vec<ExtractionFact>` — re-extract per changed file only
- `provenance: ProvenanceIndex` — rebuilt incrementally
- `spec_ir` — existing; refreshed on spec change (150ms debounce, unchanged)

New capabilities:

| Feature | Mechanism |
|---|---|
| Code diagnostics (projected lint/validate) | debounce batch → `publishDiagnostics` per code Uri |
| definition spec→code, hover code→spec | ProvenanceIndex lookups |
| documentLink in code files | handler → spec Location |
| code actions | add-description (doc comment insert), document-in-spec (lossless YAML insert via suspect-low), generate-spec-from-code |
| rename handler ↔ operationId | existing rename engine + provenance (spec-side rewrite already exists for component keys) |
| semantic tokens (code) | route attributes/decorators highlighted (adapter-supplied token ranges) |

Performance budget (suspect bar): keystroke → incremental parse + re-extract
+ index update + projection recompute ≤ 5ms for a handler file; full-workspace
cold extraction ≤ 1s on suspect-repo-size (204 files / 2.3MB); provenance
lookup O(1).

---

## 6. Validation & test engine wiring

- `suspect check --with-code spec.yaml`: existing battery + cross-checks —
  responses declared but never constructed (axum `Result`/`HttpResponse`
  signals), parameters declared but never read, security schemes declared
  but no middleware evidence, operations declared with no handler. Each
  violation carries both spec and code spans.
- `suspect code sync spec.yaml`: three-way report — code-only (undocumented),
  spec-only (unimplemented), both-present (run `suspect-codegen::diff` between
  spec-IR and code-IR → BREAKING/ADDITIVE/COSMETIC drift with semver). This
  reuses the R5 engine verbatim: drift detection between two IrSpecs.
- Test engine: compiled `StepPlan` gains optional handler provenance; step
  failures report "assertion failed at step 2 (GET /pets/{id}) — handler
  src/routes/pets.rs:42". New report: **handler coverage** — handlers with
  zero Arazzo/fuzz coverage (joins R4 stateful sequences with facts).
- Gateway: mock routes annotated with handler spans in the startup listing;
  validate-mode violations projected to code in the journal.

---

## 7. Phases and acceptance gates

| Phase | Deliverable | Gate |
|---|---|---|
| P0 | `suspect-code` skeleton: span/confidence/facts/parser/workspace + axum + actix adapters, lowering, provenance | charte axum+actix goldens byte-identical; `suspect code gen` on suspect's own gateway source round-trips; bench: extraction ≤3ms/file; clippy clean |
| P1 | CLI `code gen|check|sync`; `diff_specs(spec_ir, code_ir)` drift report | sync on a deliberately drifted fixture reports exact BREAKING set; corpus bench updated |
| P2 | Lint projection: RuleTarget::Facts, union evaluation, ProvenanceIndex projection | every existing spectral-default rule produces projected findings on a fixture repo; union rules fire on both sides of drift |
| P2.5 | TS/JS custom rules on a Bun sidecar worker — see `docs/TS-RULE-RUNTIME.md` | protocol bench budgets met; conformance suite green; union TS rules report at handler spans |
| P3 | LSP: CodeWorkspace, projected diagnostics, hover/def/links, code actions | driven verification: edit handler → diagnostic appears in editor at handler span within debounce window; navigation round-trips |
| P4 | validate `--with-code`; test-engine provenance + handler coverage; gateway projection | `check --with-code` catches seeded spec/code lies in fixtures; failing step reports handler span |
| P5 | Remaining adapter waves (express/fastapi/flask → TS breadth → go → java/csharp) | each: goldens byte-identical + fixture README |

Every phase: `cargo fmt`, clippy `-D warnings`, full workspace tests,
determinism test (double-run byte-identical), bench deltas recorded.

---

## 8. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Tree-sitter grammar deps grow build | workspace pins charte's known-good versions; optional cargo features per language for embedders, all-on for the CLI/LSP |
| Mounted/nested routers → ambiguous provenance (one route, many declarations) | ProvenanceIndex keeps ALL spans per operation ranked by confidence (charte's mount semantics); diagnostics attach to primary span, hover shows all |
| Spec-less repos | facts pipeline works standalone; hover still shows extracted operations; "generate spec" code action is the on-ramp |
| Two-truth conflicts annoy users | reconciler stance: drift is diagnostics on both sides with configurable severity; `sync --baseline` can freeze an accepted-drift list (journal-backed) |
| Golden drift during port | goldens are copied FIRST and are the definition of done; any intentional deviation (error-code rename) is a mechanical sed recorded in the port log |
| LSP state bloat | content-hash tree cache; facts for non-open code files can be evicted; index rebuild is incremental |

---

## 9. Success criteria

1. `suspect code gen` on charte's full fixture suite: byte-identical goldens
   for every ported adapter.
2. A Spectral rule written for any spec produces a correct diagnostic inside
   the handler source that implements the violating operation — zero rule
   changes, zero code annotations.
3. `suspect code sync` classifies seeded spec/code drift with correct
   BREAKING/ADDITIVE/COSMETIC labels and semver recommendation.
4. LSP: hover/definition/documentLink round-trips spec↔code; projected
   diagnostics appear at handler spans within one debounce window.
5. Failing Arazzo step reports the implementing handler's file:line.
6. All suspect quality gates hold: deterministic outputs, clippy `-D`,
   100%+ test-count growth from ported goldens, benches recorded per phase.
