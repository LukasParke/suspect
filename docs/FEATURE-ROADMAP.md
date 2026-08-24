# Feature Roadmap — the next ten platform features

Each feature builds on crates that already exist (journal/cassettes, rex,
suspect-ir, jsonpath, gateway, test executor). Nothing here is a greenfield
bet; these are compounding moves on the same core. Status legend:
`planned` → `in progress` → `shipped`.

---

## 1. Traffic-to-contract synthesis `planned`

**Builds on:** gateway record mode, suspect-journal cassettes, rex, test executor.

Reverse-compiles a recorded Suspect Cassette into an Arazzo workflow with
assertions. Captured request/response pairs become steps (`$statusCode`
criteria, body-shape checks via JSONPath); values that look volatile —
incrementing ids, timestamps, request-bound tokens — are parameterized
instead of hard-coded.

```
suspect synthesize rec.scj --spec api.yaml --out flows.arazzo.yaml
```

The output is a normal, editable Arazzo file: review it, tighten the
assertions, commit it as a regression suite. Turns "we have production
traffic" into "we have a contract suite" in one command.

**Acceptance:** cassette from a 3-endpoint session synthesizes a workflow
that replays green against the same mock; volatile fields are parameters.

## 2. Stateful property-based testing `planned`

**Builds on:** suspect-test executor, IrSpec operation graph, fuzz module.

Beyond random single-call fuzzing: model-based exploration of *sequences*.
The resource-dependency graph (POST creates → GET reads by id → PATCH
mutates) drives generation of call chains that feed created ids forward;
failures shrink to minimal reproducible sequences. Hypothesis/stateful-testing
class rigor, driven by our IR rather than hand-written state machines.

**Acceptance:** seeded failure in a demo API shrinks to a 3-step minimal
sequence automatically.

## 3. Example-pair auto-tests `planned`

**Builds on:** IrSpec examples materialization, executor.

Every matching `requestBody.example` + declared response example pair in
the spec compiles to a ready-to-run contract test. Specs that document well
test for free — and the lint ruleset nudges toward examples, closing the
loop.

**Acceptance:** petstore fixture yields one passing test per documented
example pair without any handwritten workflow.

## 4. Server scaffolding + progressive mock→real handoff `planned`

**Builds on:** codegen compiler (part 1), gateway mock synthesis, executor.

Generate *server* skeletons (axum / gin / express) with fully typed handlers
that return synthesized examples until implemented — the same shapes the
gateway mocks from. Teams flip routes from gateway-mock to their server per
endpoint; the contract suite moves with them unchanged. Spec-first
development becomes a gradient instead of a cliff.

**Acceptance:** generated axum skeleton compiles, serves example responses,
and passes the same Arazzo suite the gateway mock passes.

## 5. Webhooks & events first-class `planned`

**Builds on:** IR lift (webhooks/callbacks nodes), codegen emitters.

OpenAPI 3.1 `webhooks`/callbacks lifted in the IR like paths: consumer-side
handler stubs, producer-side dispatch helpers, and JSON-Schema artifacts for
message buses per target language. Event-driven API developers are
underserved everywhere; this makes them first-class here.

**Acceptance:** spec declaring two webhooks emits typed handler interfaces +
dispatch helpers in all three SDK targets.

## 6. OWASP API security ruleset `planned`

**Builds on:** suspect-lint engine, IrSpec security/parameter model.

A lint family covering the OWASP API Top 10 patterns checkable statically:
auth coverage per operation (securitySchemes gaps), mass-assignment hints
(writable request fields overlapping response fields), excessive data
exposure, missing rate-limit/idempotency headers on mutating operations.
Surfaces through the same editor diagnostics and CI gates, plus a posture
report per spec.

**Acceptance:** deliberately vulnerable fixture triggers every rule with
accurate spans; clean fixtures stay silent.

## 7. Auth-flow execution engine `planned`

**Builds on:** executor, gateway, config layer.

Test runner acquires credentials automatically from `securitySchemes`:
OAuth2 client-credentials/device-code executed to token, refresh handled
mid-workflow, api-key/header injection bound to environment config. Removes
the biggest friction point in real-world API testing.

**Acceptance:** workflow against a token-guarded demo API runs unattended,
refreshing once when the token expires mid-run.

## 8. Environment parity monitor `planned`

**Builds on:** replay mode, cassette format, drift comparison.

Extends replay-diff into a standing canary: scheduled replays of one
cassette against prod/staging/local with semantic comparison — status,
schema-validity, field-level deltas — and alerts on drift between
environments or from the contract. The cassette format graduates from test
fixture to continuous cross-environment truth.

**Acceptance:** seeded field removal in staging is detected and reported
with the exact pointer.

## 9. Release engineering `planned`

**Builds on:** breaking-change detection, codegen hashing/diff.

Semantic spec diff → human changelog, semver recommendation (breaking ⇒
major, additive ⇒ minor), and per-SDK release notes ("`Pet.tag` added;
`listPets.limit` now capped"), ready for a release PR. The breaking-change
detector grows into the whole ship step.

**Acceptance:** diff of two fixture versions produces changelog + correct
semver bump + per-SDK notes sections.

## 10. Transactional spec codemods `planned`

**Builds on:** IrSpec, codegen orchestration, watch/journal.

Rename/deprecate/move a field as one atomic operation that rewrites the
spec, regenerates affected docs/SDK sections, flags affected Arazzo
criteria, and emits the migration note. Codemods for the source of truth —
with every derived artifact updated or flagged in the same transaction.

**Acceptance:** renaming `Pet.tag` updates spec + regenerated TS/Rust/Go +
flags the workflow asserting on it, all in one command.
