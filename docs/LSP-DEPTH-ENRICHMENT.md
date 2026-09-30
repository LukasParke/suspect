# Depth enrichment plan — the thin and at-risk surface

Companion to [DEPTH-ANALYSIS.md](DEPTH-ANALYSIS.md), which measured where
depth is missing. This document plans the work, with the **LSP first**,
because it is the largest thin surface *and* the one with the clearest
strategic position.

State at the time of writing: the five depth-plan items are built
(config unification, the deleted inliner, executed `dependsOn`, published
SDK evidence, the shared contract runtime). What remains is depth, not
breadth.

---

## Part 1 — The LSP: a platform front-end, not a YAML highlighter

### Why this is the right bet

Every OpenAPI language server in the market is a *linter with syntax
highlighting*: parse, resolve a `$ref`, emit diagnostics. That is a solved,
commodity problem. We are the only toolchain that also owns:

- a **JSON Schema 2020-12 compiler** (so "does this instance conform" is a
  first-class operation, not a guess),
- a **twelve-backend code generator** (so "generate this operation" is
  real),
- a **contract-test runner** (so "run this workflow" is real),
- an **Overlay engine** (so "show me what this overlay does" is real),
- and now a **contract runtime and a project build graph**.

So the strategic move is not "a better OpenAPI LSP". It is **the editor
front-end to the whole toolchain**. A competitor's hover shows you a
schema. Ours can show you the resolved type, generate the client, run the
test, apply the overlay, and tell you what breaks if you change the field —
in the editor, at keystroke latency. That is a defensible position because
it is structural, not incremental.

### L1. A per-node semantic model (the foundation — do this first)

**Gap today:** hover, completion, code actions, inlay hints and semantic
tokens are each written against the CST with their own ad-hoc reasoning
about "what does this position mean". Adding a feature means writing
another such heuristic, and two features can disagree.

**Do:** one `NodeMeaning` resolved per position, cached per document
generation, answering: which OpenAPI object am I inside (Operation,
Parameter, Schema, Response, PathItem, Callback, …)? which logical path
(`/paths/~1pets/get/parameters/0/schema`)? which spec/dialect section
applies (OAS 3.0 vs 3.1 vs 3.2 vs the 2020-12 dialect)? what is the
resolved `$ref` target? what references *this* node?

Every existing feature becomes a query against that model rather than its
own walk, and the model is directly testable — which is the depth the
feature layer currently lacks. This is the single highest-leverage change
in this plan: it converts 26k lines of heuristics into a testable core
plus thin adapters.

**Evidence:** a test that, for every node kind in the OAS 3.2 grammar,
asserts the model's answer; a test that every current feature's output is
unchanged after the migration.

### L2. Schema-aware completion

**Gap today:** completions are keyword- and context-shaped.

**Do:** proposals ranked by what the contract actually offers —
sibling property names from the enclosing schema; `$ref` targets ranked by
reference count, path distance and type compatibility with the position;
enum and example values for a constrained parameter; `operationId`s for
`$sourceDescriptions.*`; channel addresses for Arazzo `channelPath`;
`components/schemas` names at any schema position. A completion that
suggests a schema you already use twice, in the right dialect, is not a
list — it is advice.

### L3. Refactors that move meaning, not text

**Gap today:** rename rewrites references; nothing else restructures.

**Do,** in order of value:
1. **Extract to component** — pull an inline schema into
   `components/schemas` and rewrite the reference, atomically, with the
   build graph's regeneration queued.
2. **Inline component** — the inverse, at the reference site.
3. **Move operation between tags / promote a parameter to a reusable
   component.**
4. **Apply overlay** — preview an Overlay's actions against the current
   document in a diff, then apply. The engine already exists; only the
   preview surface is missing.

These are the codemods the roadmap named, delivered as editor operations
where they belong. Each is a transactional multi-file edit, which the
lossless CST already supports.

### L4. Change-impact awareness (the differentiator)

**Gap today:** the server knows the current document's diagnostics and
nothing about consequences.

**Do:** when a definition is about to change, answer in the editor:
which operations transitively use it, which Arazzo criteria assert on it,
which generated SDK files it will move, whether `suspect ci` would fail,
which recorded cassettes contain matching data. We have every input:
the reference graph, the Arazzo plan, the artifact ownership map, the
`impact` report. No competitor can compute this, because no competitor
owns the runner, the generator and the traffic.

This is the feature to build the product's story around.

### L5. Protocol conformance as a first-class test suite

**Gap today:** per-feature unit tests; no LSP-level harness.

**Do:** a conformance suite that drives a real `tower-lsp` server over
JSON-RPC — initialize, didOpen, completion, hover, definition, code
action, execute command — asserting protocol-level responses. Includes
cancellation, partial-result and error-path cases. If we claim a complete
LSP 3.17 surface, the evidence should be protocol-shaped.

### L6. The editor as the runner

**Gap today:** run lenses exist for Arazzo; the rest of the toolchain is
CLI-only.

**Do:** expose the platform as LSP commands, reusing the existing
`executeCommand` plumbing — run this workflow, run this operation's
example as a contract test, generate this operation into the project's SDK
target, verify the contract package, show what this overlay changes, gate
this service. Progress through `window/workDoneProgress`, results as
structured data the client renders. The commands are thin wrappers over
functions that already exist; the work is surfacing them, which is exactly
why nobody else can do it.

### L7. Latency as a published number

**Gap today:** microsecond executor and parser budgets are measured;
editor latency is not.

**Do:** measure didOpen→diagnostics, didChange→semantic tokens, and
completion at document scale (a 5k-line, 1k-operation document), publish
the p50/p95 in CI as a budget with a regression gate. Editor performance
is a correctness property users feel immediately, and a measured claim
beats an anecdote.

---

## Part 2 — The remaining thin surfaces

### T1. SDK codegen evidence (from risk 1)

**Now:** `suspect evidence` + the scheduled job publish the *declared*
matrix; actual run results are in the job log.

**Next:** fold the run results back into the matrix so the artifact
answers "verified on date X against toolchain Y" per backend, and alert
when a claim loses its evidence. Then attack the underlying weakness:
coverage that exists but is never executed because a toolchain is absent
should be visible in the matrix as `declared, unverified` rather than
silently green.

### T2. Lint depth (from the risk 2 fix)

**Gap:** the ruleset is a thin Spectral subset; rules are tested one at a
time, so rule *interaction* is unverified, and a document that trips a
dozen rules may produce an unreadable or unstable diagnostic set.

**Next:** a shared fixture corpus where every rule fires alongside
neighbours, with a test that the diagnostic *set* is stable (no duplicate
codes for one node, no ordering nondeterminism, no cascade noise), plus
custom-function loading and external `extends` for parity with Spectral.

### T3. Gateway operations (from the thin list)

**Gap:** a demo operationally. No rate limiting, TLS, graceful shutdown,
body ceiling, or request-id propagation into the cassette.

**Next:** the four that change behaviour under load or failure — body
ceiling, request-id correlation, graceful drain, and keyed (rather than
percentage) fault injection so a specific test id can be faulted
deterministically. Then TLS, which is a configuration story more than a
code story.

### T4. Arazzo and Overlay owned models (from the thin list)

**Gap:** the models are views. Load → modify → save is not first-class for
Arazzo the way Overlay synthesis now is.

**Next:** owned, round-trippable models for both (like `suspect-overlay`'s
`Value`), which unlocks Arazzo codemods and a `suspect arazzo upgrade`
for 1.0 → 1.1 — a real need now that we support both.

### T5. Message broker adapters (from the thin list)

**Gap:** AsyncAPI execution works over a file broker and a loopback; no
real broker.

**Next:** one adapter against the `MessageTransport` seam with a real
conformance fixture (publish/subscribe, correlation, timeout, redelivery),
selected by `suspect test --message-broker`. Pick the transport by what
users actually run — the honest first question is which broker dominates
our users' stacks, not which is easiest.

### T6. Schema-engine adoption (from the shallow-adoption risk)

**Gap:** the compiler is deep; its *defaults* are not. The response path
now resolves refs natively (this cycle), but `compile_v2`'s scoped
applicators remain opt-in and native codegen adoption is tracked, not done.

**Next:** make the deep path the only path — the owned-program compiler
with native adoption as the default, and the v1 inliner removed rather
than bypassed. Depth is not real while two engines exist.

---

## Part 3 — Sequencing

**This cycle (highest leverage, each small):**
L1 (semantic model) → L7 (latency budgets) → T1 (evidence results folded
back) → T2 (lint interaction corpus). L1 unblocks most of the LSP work;
L7 makes the rest trustworthy; T1 and T2 close the two remaining named
risks.

**Next cycle:** L2 (schema-aware completion) → L5 (protocol conformance)
→ L4 (change impact) → T3 (gateway operations). L4 is the flagship
feature and should land once L1 makes it cheap.

**After:** L3 (refactors), L6 (editor as runner), T4, T5, T6 — the larger
surfaces, in the order they become possible rather than the order they
sound impressive.

## What would make me wrong

- If editor latency is not a real adoption driver for this audience, L7
  matters less and L4 matters more — the flagship feature, not the
  measurement, is the bet.
- If users live in their own CI and rarely in the editor, L6 is
  low-leverage regardless of how good it is, and the effort belongs in T2
  and T3.
- If the twelve backends are not actually differentiated in practice, T1's
  underlying risk is overstated and the codegen breadth should be reduced
  rather than evidenced more thoroughly.

Those are the three assumptions this plan rests on. Each is checkable
cheaply — usage data on commands, editor session length, backend
downloads — and each should be checked before the next cycle commits a
quarter to it.
