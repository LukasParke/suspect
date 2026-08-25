# Ten Revolutionary Features for the Suspect Platform

The platform already has something no other OpenAPI toolchain has: a single
semantic core that drives parsing, validation, code generation, contract
testing, gateway serving, and traffic capture — all from one lossless parse
of the specification at sub-10ms latency on production-scale APIs.

These ten features exploit that foundation to build capabilities that don't
exist anywhere else in the ecosystem.

---

## 1. Traffic-Informed Spec Evolution

The gateway watches real API traffic continuously. When it observes a
response field, query parameter, or header that the spec doesn't document,
it doesn't just journal it — it proposes a spec amendment.

### How it works

1. Gateway proxies live traffic through the validate pipeline.
2. Every response body is parsed against the operation's declared response
   schema from the IR.
3. Fields present in responses but absent from schemas are collected as
   "undocumented observations" with frequency counts.
4. After a configurable observation window, the system produces a diff:
   a proposed `openapi.yaml` patch adding the observed fields with inferred
   types (from the observed value shapes).
5. The developer reviews the proposal as a standard git diff — accept,
   modify, or reject each field individually.

### Why it's revolutionary

Every other tool treats the spec as immutable truth. This makes it a living
document that converges toward reality instead of drifting away from it.

### What it leverages

- Gateway validate mode (already built) captures full request/response pairs
- IR knows exactly which fields are documented vs undocumented
- The CST preserves byte offsets, so proposed amendments can be inserted at
  the correct position in the YAML source without breaking formatting
- Journal records provide frequency data to rank proposals by confidence

---

## 2. Causal Contract Debugger

When a test fails, most tools say "expected X, got Y." This debugger traces
the *causal chain*: which schema constraint fired, where in the spec source
it lives, who introduced it (git blame), when, whether any historical
traffic ever passed this check, and what changed since.

### How it works

1. A validation error identifies a specific constraint node in the IR.
2. Each IR node carries its byte offset into the original YAML file (from
   the lossless CST).
3. Byte offset → line/column → `git blame` → commit SHA + author + date.
4. The cassette history is searched: has any recorded response ever passed
   this constraint? If yes, when did it start failing?
5. The output is a timeline: "This constraint was added by commit `abc123`
   on 2024-03-15. It first failed in traffic recorded on 2024-06-22."

### Why it's revolutionary

No other tool connects runtime failures back to the exact spec source
location, let alone to version-control history and historical traffic.

### What it leverages

- Lossless CST preserves byte offsets through every transformation
- IR nodes carry their provenance chain
- Cassette format stores timestamps per entry
- Git integration is trivial from Rust

---

## 3. Grammar-Evolved Generative Fuzzing

Current fuzzing sends random mutants. This feature learns the *input
grammar* from successful responses, then uses coverage-guided evolution to
generate increasingly targeted test cases — AFL/libFuzzer philosophy applied
to HTTP APIs.

### How it works

1. Start with schema-derived seed inputs.
2. Send requests; classify responses by shape (status × top-level keys ×
   value-type distribution). Novel shapes earn coverage points.
3. Mutations that produce novel response shapes are kept as new seeds;
   others are discarded.
4. Over iterations, the fuzzer explores the input space systematically:
   boundary values → type confusions → injection patterns → state
   interactions.
5. Any 5xx or unexpected schema violation is reported as a crash with the
   minimal reproducing input.

### Why it's revolutionary

Existing API fuzzers (Schemathesis, Dredd) generate inputs randomly within
schema constraints. They don't learn from responses or guide exploration
toward interesting behavior. This turns the fuzzer from a smoke test into
a research tool.

### What it leverages

- The rex evaluator understands Arazzo runtime expressions natively
- The executor supports concurrent step execution
- The journal captures structured response metadata for coverage tracking
- The fast IR (<10ms) means the fuzzer can re-analyze between rounds

---

## 4. Stateful Dependency-Graph Testing

Auto-discovers which operations create/read/update/delete which resource
types from the spec semantics (POST /users creates User; GET /users/{id}
reads User), builds a dependency DAG, and generates test suites that set up
required state before exercising dependent operations.

### How it works

1. Analyze path templates and parameter names across all operations to
   infer resource types (`/users/{userId}/posts/{postId}` implies
   posts depend on users).
2. Build a DAG: operations that return 201 with a `Location` header or
   body containing an id field create resources.
3. Generate test sequences: create prerequisites first, then exercise
   target operations using captured ids via `$response.body#/id`.
4. On failure, shrink the sequence to the minimal reproducing chain.
5. Teardown: DELETE operations clean up created resources.

### Why it's revolutionary

Schemathesis tests individual operations in isolation. This tests *stateful
interactions* — the hardest bugs to catch are the ones where resource B
breaks because resource A was mutated. Auto-generating these sequences from
the spec eliminates hundreds of lines of manual test setup code.

### What it leverages

- The IR already indexes all operations by method+path
- Path templates expose parameter names that map to resource ids
- The executor chains outputs between steps via `$response.body#/id`
- The shrinking algorithm uses the existing criterion evaluation engine

---

## 5. Wire-Format-Aware Semantic Diffing

Text diffs and JSON structural diffs can't distinguish between a breaking
change and a cosmetic refactoring. This feature diffs at the Semantic Type
Graph level, understanding wire-format transforms.

### How it works

1. Parse two spec versions into separate STGs.
2. Match components by semantic identity (not name — handle renames via
   heuristic matching on field sets and types).
3. For each matched pair, compute a delta: added/removed/changed fields,
   constraint tightening/loosening, enum value changes, nullability flips.
4. Classify each delta as BREAKING / ADDITIVE / COSMETIC based on whether
   it changes the wire format consumers observe.
5. Output: a report showing only semantically meaningful changes, with
   breaking changes highlighted and consumer impact estimated from
   recorded traffic.

### Why it's revolutionary

Renaming a JSON key with a custom serializer is NOT breaking, but every
text-based diff tool flags it. Changing a discriminator value from
`"created"` to `"new"` IS breaking, but a structural diff might miss it if
only the string literal changed. Only a semantic-level diff gets this right.

### What it leverages

- The STG already lifts schemas into typed nodes with refinements
- Constraint comparisons (tighter vs looser) are type-theory operations
- Recorded cassettes provide ground truth about which fields consumers
  actually read

---

## 6. Live Contract Bridge

Continuous bidirectional synchronization between the running server and the
specification. The gateway watches real responses; when undocumented
behavior appears, it proposes spec amendments. When the spec changes, mocks
and SDKs regenerate instantly.

### How it works

1. **Observation loop**: gateway validates live traffic → detects
   undocumented fields/responses → accumulates evidence → proposes spec
   patches (feature #1 above).
2. **Propagation loop**: approved spec changes trigger instant
   regeneration of mock routes, SDK types, documentation pages, and
   validator rules via the existing watch infrastructure.
3. **Reconciliation**: conflicts (spec says X, server returns Y) are
   surfaced as diagnostics in the LSP with quick-fix actions to update
   either side.

### Why it's revolutionary

This creates a self-healing development loop where the specification
converges toward reality and reality converges toward the specification.
No manual sync steps, no drift, no stale docs.

### What it leverages

- Watch mode (already built) provides file-system eventing
- Gateway validate mode (already built) provides behavioral observation
- LSP diagnostics (already built) surface conflicts in-editor
- Codegen hashing (already built) enables incremental regeneration
- The entire pipeline runs at <10ms per iteration on stripe-sized specs

---

## 7. Consumer Impact Analysis Across Versions

Given two spec versions and a recorded traffic cassette, compute exactly
which recorded consumers break under the new version, what migration steps
they need, and quantify the blast radius.

### How it works

1. Load both spec versions into separate IRs.
2. Compute the semantic diff (feature #5): which fields were removed,
   renamed, retyped, had constraints tightened.
3. Replay the cassette against BOTH IRs: which recorded exchanges pass the
   old spec but fail the new one?
4. Group violations by consumer fingerprint (user-agent, client-id header,
   source IP range) to identify affected consumers.
5. Output: per-consumer impact report with specific remediation steps.

### Why it's revolutionary

"Will this change break my customers?" is currently answered by hope and
manual testing. With this feature, you get a quantified answer: "3 of 12
consumers send requests that will fail; here are the exact requests and
what they need to change."

### What it leverages

- Cassette entries record full request headers including identifying info
- The dual-IR approach lets us evaluate the same request against both versions
- The STG semantic diff classifies each difference by severity
- The journal format tracks consumer fingerprints across sessions

---

## 8. Implementation-to-Spec Reverse Engineering

Parse server framework route registrations (axum macros, Express route
definitions, Gin handler groups) and cross-reference against the spec to
detect undocumented endpoints, missing parameters, or response-shape
mismatches.

### How it works

1. Parse the server source tree for known framework patterns:
   - Rust: `#[get("/path")]`, `.route("/path", post(handler))`
   - TypeScript: `app.get('/path', handler)`
   - Go: `r.HandleFunc("/path", handler)`
2. Extract route paths, HTTP methods, and handler function signatures.
3. Map handler parameter types to expected request schemas (via serde/
   zod/struct-tag annotations).
4. Diff against the spec: endpoints implemented but not documented;
   parameters accepted but not declared; response types that don't match
   declared schemas.
5. Propose spec updates for undocumented endpoints.

### Why it's revolutionary

Every other tool works FROM the spec. This works FROM the implementation,
closing the gap from the other direction. Combined with the forward
codegen pipeline, it creates a bidirectional bridge between specification
and implementation.

### What it leverages

- Tree-sitter parsers already exist for Rust, TypeScript, and Go
- The STG compiler can synthesize schema definitions from native type
  declarations (structs/interfaces)
- The diagnostic infrastructure surfaces mismatches as editor warnings

---

## 9. Interactive Contract Playground

A REPL/scratchpad where developers construct requests visually, see live
validation results against the spec, inspect generated type representations
in any target language, and export working requests as Arazzo test steps.

### How it works

1. The LSP serves a webview (VS Code extension) or opens a browser tab.
2. The playground loads the current spec's IR and presents available
   operations as a searchable palette.
3. Selecting an operation shows its parameters with inline validation,
   auto-completion from enums/examples, and type hints from the STG.
4. Executing a request shows the response alongside:
   - Contract validation verdicts (per-field)
   - Generated type representations in TS/Rust/Go
   - Schema conformance score
   - Similar historical requests from cassettes
5. "Save as test step" exports the request + assertions as an Arazzo step
   appended to a workflow file.

### Why it's revolutionary

Postman/Insomnia require manual import and lose spec context. Swagger UI
is read-only. This is a *live editing environment* connected to the spec,
with instant feedback from real servers and generated code.

### What it leverages

- The LSP already serves spec analysis at keystroke latency
- The executor handles real HTTP dispatch with criteria evaluation
- The STG compiler generates type representations on demand
- The journal captures results for reproducibility
- The VS Code extension provides the webview container

---

## 10. Specification Quality Scoring with Actionable Feedback

Not just lint findings — a holistic quality score per endpoint, per schema,
and per spec, with prioritized, actionable recommendations ranked by
developer impact.

### How it works

1. Score dimensions: documentation completeness (descriptions, examples),
   consistency (naming conventions, pagination patterns), constraint
   density (how well inputs are validated), error documentation (are error
   responses documented?), deprecation hygiene.
2. Each dimension is scored 0–100 with specific improvement actions.
3. Actions are prioritized by: number of affected consumers (from
   cassettes) × effort to fix (estimated from edit distance).
4. Trend tracking over time: is quality improving or degrading?
5. Integration: scores appear in CI as a non-blocking metric with trend
   arrows, preventing gradual quality decay.

### Why it's revolutionary

Lint rules are binary: pass or fail. Quality scoring is continuous and
contextual — it tells you WHICH improvements matter MOST and tracks whether
you're making progress. It turns API governance from policing into coaching.

### What it leverages

- The lint engine provides the rule evaluation framework
- The IR provides completeness metrics (documented vs undocumented)
- Cassette data provides consumer-impact weighting
- The journal tracks quality trends over time
- CI integration surfaces trends before they become problems

---

## Summary Matrix

| # | Feature | New capability | Builds on | Effort |
|---|---|---|---|---|
| 1 | Traffic-Informed Spec Evolution | Living spec converges toward reality | gateway + IR + CST | Medium |
| 2 | Causal Contract Debugger | Failure→spec-source→git-blame timeline | CST offsets + cassettes + git | Low |
| 3 | Grammar-Evolved Fuzzing | Coverage-guided API input exploration | fuzz module + executor + journal | Medium |
| 4 | Stateful Dependency-Graph Testing | Auto-generated stateful test sequences | IR + executor + path analysis | Medium |
| 5 | Wire-Format-Aware Semantic Diffing | Breaking vs cosmetic classification | STG compiler + dual-IR comparison | Low |
| 6 | Live Contract Bridge | Self-healing spec↔implementation loop | gateway + watch + codegen + LSP | High |
| 7 | Consumer Impact Analysis | Quantified blast radius per version change | dual-IR + cassettes + STG diff | Medium |
| 8 | Handler-to-Spec Reverse Engineering | Detect undocumented server endpoints | tree-sitter parsers + STG synthesis | High |
| 9 | Interactive Contract Playground | Visual REPL for API exploration + test authoring | LSP webview + executor + STG | High |
| 10 | Quality Scoring with Actionable Feedback | Continuous governance, not binary policing | lint + IR + journal + cassettes | Low |

## Recommended Sequencing

Quick wins (days): #5 (semantic diffing), #10 (quality scoring),
#2 (causal debugger)

Medium-term (weeks): #1 (traffic-informed evolution), #3 (grammar-evolved
fuzzing), #4 (stateful testing)

Long-term (month+): #6 (live contract bridge), #7 (consumer impact),
#8 (reverse engineering), #9 (playground)

Each feature compounds: semantic diffing (#5) enables consumer impact
analysis (#7); causal debugging (#2) enriches the playground (#9);
traffic-informed evolution (#1) feeds the live contract bridge (#6).
