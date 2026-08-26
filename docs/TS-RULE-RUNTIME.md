# TS/JS Custom Rules on a Bun Sidecar — Design

Amendment to `docs/CODE-FOLD-PLAN.md` (§ P2/P2.5). Adds a third rule source —
user-authored TypeScript rules — alongside native Rust rules (suspect-lint)
and the fact-space/union evaluation model. Inspired by Telescope's custom-rule
DX (`defineRule`, visitor pattern, `ctx.report`/`ctx.locate`, zod schemas,
config-registered rule files), redesigned on suspect primitives.

---

## 1. Topology: long-lived Bun sidecar worker

**Decision: suspect spawns a persistent `bun` worker process; TS rules execute
there; heavy operations call back into Rust.**

Rejected alternatives, with evidence:

| Option | Verdict | Why |
|---|---|---|
| Embed Bun in the suspect process | Rejected | Bun 1.4 (Aug 2026) completed the Zig→Rust rewrite but ships as a runtime binary, not an embeddable library. There is no official Rust-host embedding API. Embedding raw JavaScriptCore instead would lose Bun's TS execution and APIs — the two things we want. |
| Subprocess per evaluation | Rejected for LSP | Bun cold start (~30–100ms with TS transpile) per keystroke destroys the latency budget. Acceptable only for one-shot CI mode. |
| V8/deno_core embed | Rejected | Different runtime than the user's toolchain; loses "run rules with the same engine you run your app" coherence; heavier embed. |
| **Sidecar worker** | **Chosen** | The esbuild/tsserver model: spawn once, keep warm, stream work over stdio. Crash isolation (a hung rule never stalls the LSP), hot rule reload, zero amortized startup, and `bun:ffi` gives the worker direct, fast calls into a suspect cdylib for Rust-backed operations. Bun 1.4's native JSONL API fits the wire protocol. |

Trust model (same as ESLint plugins / Spectral JS rules): custom rules execute
arbitrary code and are **opt-in via explicit config**. The LSP prompts before
first activation per workspace; `--no-custom-rules` and a server setting
hard-disable; rules never auto-activate from untrusted workspaces.

---

## 2. Worker protocol

Transport: length-prefixed NDJSON frames over the child's stdio. Versioned at
`hello`; capability-negotiated.

```text
→ hello  {protocol: 1, sdk: "1.0.0", cwd, workspace_root}
← ready  {rules: [{id, name, targets, given[], visitors[], usesZod, file}]}
           # Rust learns each rule's `given` selectors WITHOUT running it,
           # and can pre-evaluate selection natively (fast path).

→ evaluate {run_id, doc_kind: spec|facts|union, timeout_ms,
             selections: [{rule_id, nodes: [{pointer, value, span?}]}],
             context: {doc_uri, spec_summary?}}
← finding {run_id, rule_id, pointer, message, severity,
            span?, code_action?, fix?}          # streamed, batched per run
← done    {run_id, ms, findings: n}

→ native  {call_id, fn: "resolveRef"|"provenance"|"workspaceSchema"
                  |"locateRange"|"stgTypeOf"|"readWorkspaceFile", args}
← native_result {call_id, ok, value}             # FFI-backed when available

→ reload  {changed: [rule_file_paths]}           # hot reload, re-import
← ready   {...}                                  # re-declared rules

→ ping / ← pong                                  # watchdog liveness
```

Host-side enforcement:

- **Watchdog**: per-`evaluate` deadline (default 250ms LSP / 5s CLI). On
  breach: kill worker tree, disable the offending rule for the session,
  publish `rule-timeout` diagnostic, restart worker. Fail-open, never
  fail-silent.
- **Determinism**: findings sorted host-side by `(rule_id, pointer)`. The
  `ctx` surface exposes no clock, no randomness, no ambient network. File
  reads only via `ctx.readWorkspaceFile` (workspace-root-jailed).
- **Memory**: worker holds only open-document slices with LRU eviction;
  256MB cap, restart on breach.

---

## 3. The SDK — where the DX is won

`@suspect/rules-sdk` npm package. Two rule shapes, both first-class:

### 3.1 Point rules (the 95% case — Spectral-shaped, ESLint-comfortable)

Rust evaluates the `given` selector (suspect-lint fast path) and ships only
the matched nodes. The rule body is a typed function per node.

```ts
// .suspect/rules/operation-summary.ts
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: {
    id: "operation-summary",
    targets: ["spec"],              // "spec" | "facts" | "union"
    description: "Operations need a one-line summary (≤ 120 chars)",
    type: "problem",                // "problem" | "suggestion" | "hint"
  },
  given: r.operation,               // selector DSL — compiled Rust-side
  check(op, ctx) {
    if (!op.summary) {
      ctx.report({
        message: "Operation is missing a summary",
        at: op,                     // span resolved host-side from pointer
        severity: "warning",
        fix: { kind: "insert-doc", template: "summary" },
      });
    } else if (op.summary.length > 120) {
      ctx.report({ message: "Summary exceeds 120 chars", at: op.summary });
    }
  },
});
```

Contrast with Telescope's equivalent: no `joinPointer`/`splitPointer`
plumbing, no `getValueAtPointer`, no `ctx.project.docs.get` — typed child
access (`op.summary`, `op.responses.ok`, `op.parameters.path.id`) and
`at: node` span resolution are built in. `given` selection means the rule
never touches the document root.

### 3.2 Walk rules (cross-node — ESLint-style visitors)

```ts
export default defineRule({
  meta: { id: "unique-operation-ids", targets: ["union"] },
  visitors: {
    onDocument(_doc, ctx) { ctx.state.ids = new Map(); },
    Operation(op, ctx) {
      const prev = ctx.state.ids.get(op.operationId);
      if (prev) {
        ctx.report({
          message: `Duplicate operationId "${op.operationId}" (also at ${prev})`,
          at: op.operationIdNode ?? op,
        });
      } else {
        ctx.state.ids.set(op.operationId, op.pointer);
      }
    },
  },
});
```

The walker runs TS-side over the shipped document slice; `ctx.state` is
per-run shared state; node wrappers are typed. This shape enables the rules
point-selection cannot express (duplication, consistency, cross-references).

### 3.3 Typed document model

Discriminated unions mirroring the OpenAPI model *and* the fact model:

- Spec nodes: `OperationNode`, `ParameterNode`, `ResponseNode`,
  `SchemaNode`, `PathItemNode`, `SecuritySchemeNode`… each carries
  `.pointer`, `.span` (byte range into the lossless CST — exact source
  location), `.parent()`, `.at(subpointer)`, `.text()`.
- Fact nodes (fold integration): `FactRouteNode`, `FactResponseNode`,
  `FactParameterNode`, `FactSchemaNode`… each carries `.confidence`
  (`exact | inferred | partial | unknown`), `.adapterId`, `.handler`,
  `.sourceSpan` (the handler's file + byte range). A TS rule walking
  fact-space can therefore **report findings directly inside handler code** —
  TS rules are first-class citizens of the code-linting story.

### 3.4 Selector DSL

```ts
r.operation            // $.paths[*][*]        (excludes path items)
r.operationIn("paths") // namespaced variants
r.parameter            // all parameter objects, resolved+inline
r.response
r.schema               // component schemas
r.pathItem
r.fact.route | r.fact.response | r.fact.parameter | r.fact.schema
r.custom("$.paths[*].get.responses[*]")   // raw JSONPath escape hatch
```

Selectors are declared in the rule's `ready` metadata, so the host
pre-evaluates them natively and never ships the whole document for point
rules.

### 3.5 Zod schemas (Telescope-inspired, three validation modes)

```ts
// .suspect/schemas/pet.ts
import { defineSchema } from "@suspect/rules-sdk";

export default defineSchema("Pet", (z) =>
  z.object({
    id: z.number().int(),
    name: z.string().min(1),
    status: z.enum(["available", "pending", "sold"]),
  }),
);
```

Registered schemas validate: (a) spec component schemas structurally
(STG→JSON-Schema lowering fed to zod), (b) **fact-space inferred shapes** —
code truth vs the zod contract produces drift findings at handler spans,
(c) example payloads in the spec. Zod issues map to findings with pointer
paths and configurable severity.

---

## 4. Rust rule functions with TS bindings — the honest split

One semantic source, two runtimes, conformance-gated:

| Class | Implementation | Call path | Examples |
|---|---|---|---|
| **Leaf functions** (pure, per-node) | Rust (suspect-lint `functions`) **and** TS mirror in the SDK | in-process TS sync calls | `casing`, `matches`, `defined`, `truthy`, `lengthBetween`, `isDateTime`, `enumValues` |
| **Native-only functions** (stateful/heavy — Rust-authoritative) | Rust only, behind `bun:ffi` (dlopen `libsuspect_rules_sdk`) with protocol fallback | `ctx.*` async calls, O(rule) not O(node) | `ctx.resolveRef`, `ctx.provenance(op)` (spec↔code spans), `ctx.workspaceSchema(name)`, `ctx.stgTypeOf(node)`, `ctx.locateRange`, `ctx.validateAgainstMetaSchema` |

Why mirror the leaf functions instead of FFI-everything: an FFI call costs
~1–3µs; the leaf function bodies cost ~100ns. FFI-per-node would make the
bindings the bottleneck. Mirrors keep the hot loop pure TS; the conformance
suite (shared fixture corpus, CI asserts Rust and TS outputs are identical)
keeps the mirror honest. Native-only functions need Rust state (workspace IR,
provenance index, STG) and are called rarely — FFI/protocol cost amortizes
to noise.

Every function has one doc table: rustdoc ↔ TSDoc generated from the same
source of truth, so the SDK docs can never drift from the native behavior.

---

## 5. Config and discovery

Spectral rulesets stay the entry point; custom rules extend them:

```yaml
# .suspect/config.yaml   (or `customRules:` inside an existing ruleset)
rules:
  operation-summary:
    import: ./rules/operation-summary.ts
    severity: error
  team-no-raw-string-responses:
    import: ./rules/no-raw-strings.ts
    severity: warn
schemas:
  pet: { import: ./schemas/pet.ts, validate: [components.schemas.Pet] }
customRules:
  dir: .suspect/rules        # auto-discovery alternative to explicit imports
runtime:
  bun: auto                  # auto | require | off  (auto = use if ≥1.4 on PATH)
  timeoutMs: 250
```

Scaffold + testing DX (beyond Telescope):

```bash
suspect rules new operation-summary   # writes template rule + config entry
```

```ts
// rules/operation-summary.test.ts — unit-test your rule with bun test
import { testRule } from "@suspect/rules-sdk/testing";
import rule from "./operation-summary.ts";

testRule(rule, {
  given: { "/pets": { get: { operationId: "listPets" } } },
}, (f) => {
  f.expectsFinding({ message: "Operation is missing a summary" });
});
```

Rule authors get in-editor unit tests, typed nodes, and scaffolded rules —
the loop Telescope leaves to manual fixture runs.

---

## 6. Integration with the fold

- `targets: ["facts" | "union"]` gives TS rules the code-linting story for
  free: walk `FactResponseNode`s → "handler returns raw string" reported at
  the handler span.
- TS findings, native findings, and projected findings share one LSP
  pipeline (same debounce, same `publishDiagnostics`), one ordering, one
  severity model.
- `fix` intents from TS rules (`insert-doc`, `document-in-spec`) map onto
  the fold's code actions; the host executes them (lossless YAML insert /
  doc-comment insert) — TS proposes, Rust disposes (keeps CST edits safe).
- The worker is a *consumer* of the fold's ProvenanceIndex via
  `ctx.provenance` — a TS rule can annotate a spec finding with the handler
  location without reimplementing any index logic.

---

## 7. Performance budget (suspect bar)

| Path | Budget |
|---|---|
| Sidecar cold start | ≤ 100ms (CLI one-shot; amortized zero in LSP) |
| `evaluate` IPC overhead | ≤ 1.5ms round trip |
| Worker time, ≤50 point rules on one file's given-subtrees | ≤ 5ms |
| Full stripe-spec lint, 100 rules | ≤ 250ms worker time (Rust selection dominates) |
| Hot reload | ≤ 50ms for changed rule files |
| Per-node round trips | **Zero** — batched frames, streamed NDJSON findings |

Bench plan: `suspect bench rules-ts` — cold start, per-evaluate overhead,
rules/sec at N rules × M nodes, hot-reload latency; recorded per release
like the existing feature benches.

---

## 8. Phasing

| Phase | Deliverable | Gate |
|---|---|---|
| P2.5a | Worker + protocol + `hello/ready/evaluate` lifecycle; point rules only; no FFI | round-trip bench meets §7; watchdog kill/restart test; determinism (sorted findings) test |
| P2.5b | SDK v1: `defineRule`, typed spec nodes, selector DSL, leaf-function mirrors + **conformance suite** | conformance green across fixture corpus; `testRule` harness works in `bun test` |
| P2.5c | Walk rules, zod schemas, fact-space/union targets, `fix` intents | union TS rule reports at handler spans in the LSP; zod drift test on seeded fixture |
| P2.5d | `bun:ffi` native functions (`resolveRef`, `provenance`, `stgTypeOf`) with protocol fallback | FFI path ≤ 50µs/call; fallback verified when cdylib absent |
| P2.5e | `suspect rules new` scaffold, docs generation (rustdoc ↔ TSDoc), npm packaging | docs build from single source of truth |

---

## 9. Risks

| Risk | Mitigation |
|---|---|
| Bun 1.4 is fresh (Rust rewrite just landed) | minimum-version gate; versioned protocol with capability negotiation; worker is replaceable (protocol is the contract, not Bun) |
| Arbitrary code execution | explicit config opt-in; LSP trust prompt; `--no-custom-rules`; workspace-jailed file reads; no network surface in `ctx` |
| FFI instability across Bun versions | FFI is an optimization, not a dependency — protocol fallback always works |
| Mirror drift (Rust vs TS leaf functions) | conformance CI gate over shared fixtures; native-only classification for anything stateful |
| Rule abuse (infinite loops, huge allocations) | watchdog kill + rule disable + worker restart; memory cap; findings capped per run |
