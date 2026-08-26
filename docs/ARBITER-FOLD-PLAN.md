# Folding Arbiter into suspect — Architecture Plan

Companion to `docs/CODE-FOLD-PLAN.md` (charte.rs fold, code → spec) and
`docs/TS-RULE-RUNTIME.md`. This plan folds the **third leg**: Arbiter's
traffic plane (exact capture, replay, TLS interception, credential gateway,
traffic→spec inference) into suspect, under the same alignment rule —
**suspect primitives win**.

## 0. What each side contributes

| Arbiter brings | suspect brings |
|---|---|
| Byte-exact capture (sha256-addressed bodies, spill-to-disk, redaction-before-persistence, fail-closed secret scan) | The semantic core: IrSpec, STG, validate, lint, semantic diff |
| SSE terminal-state evidence + WebSocket stream capture | Contract evaluation in the data path (verdicts per exchange) |
| TLS interception (CA + per-host leaf certs + CONNECT MITM) | Arazzo executor, stateful test generation, evolved fuzzing |
| Credential gateway (opaque tokens, server-side injection) | Consumer-impact analysis, causal debugger, evolution engine |
| Traffic→spec synthesis (schema inference, path-param detection, security detection) | TS rule runtime, LSP, deterministic codegen |
| Replay verdicts: exact-byte / semantic-JSON / semantic-SSE | Quality scoring, playground, docs |
| HAR export, LLM provider fingerprinting | 22-crate workspace discipline: benches, goldens, clippy −D |

The strategic fit: suspect today evaluates contracts against **parsed views**
of traffic. Arbiter guarantees **the bytes themselves**. After the fold,
every contract verdict, every diff, every evolution proposal is anchored to
content-addressed ground truth — and suspect gains the ability to observe
*any* HTTPS client (TLS MITM) rather than only proxies it controls.

### Non-goals

- No changes to the frozen bundle interchange format (Arbiter's v1/v2
  contract with the published TS package and OpenRouter CI).
- No absorption of Arbiter's CLI surface, Scalar docs server, or SQLite
  store (suspect has its own surfaces; bundles replace SQLite as the
  interchange, journal JSONL remains the streaming format).
- Arbiter-the-repo continues to exist for the TS/npm ecosystem; **the bundle
  format is the permanent bridge** between the two codebases.

---

## 1. The central design: one exchange record, three consumers

Today two exchange models coexist:

- **Arbiter `CapturedExchange`**: byte-fidelity bodies (`sha256` +
  `BodyStorage::{InlineBase64, Blob}`), `StreamState` (SSE terminal
  evidence), `WsStream`, `TunnelInfo`/`TlsExchangeInfo`, `LlmMeta`,
  `ValidationSummary`, strict sequence, digest-excluded timing provenance.
- **suspect-journal `TrafficRecord`**: parsed views, correlation id,
  contract `verdict`, JSONL streaming.

**Decision: `CapturedExchange` is the base; suspect's contract fields join
it as an extension.** The unified record lives in the new `suspect-capture`:

```rust
pub struct Exchange {
    // — Arbiter v2 wire fields, byte-compatible (frozen) —
    pub captured: CapturedExchange,
    // — suspect extensions (additive, skip-if-absent; v1 records
    //   serialize byte-identically per Arbiter's AMEND-13 rule) —
    pub contract: Option<ContractEvaluation>,  // verdict, violations, matched op
    pub correlation: Option<Correlation>,      // arazzo workflow/step linkage
}
```

Consumers:
1. **Journal** (JSONL) streams `Exchange` — v1 `TrafficRecord` gets a
   load-adapter; old journals remain readable.
2. **Bundles** (content-addressed, sha256 digests) — read/write unchanged;
   suspect-authored bundles load in Arbiter/TS and vice versa. This is the
   interoperability anchor and the OpenRouter CI contract.
3. **Analysis engines** — consumer impact, evolution, causal debugger,
   semantic replay all consume `Exchange` instead of `RecordedExchange`
   (the placeholder type in `consumer_impact` is deleted).

---

## 2. Crate architecture

### New crates (from Arbiter's `rust/src` modules)

| Crate | Ported from | Contents |
|---|---|---|
| `suspect-capture` | `types.rs`, `capture/`, `bundle/`, `redaction.rs`, `secret_scan.rs`, `headers.rs`, `sse.rs`, `ws/`, `json.rs` | `Exchange`, `CaptureSession` (embeddable reverse-proxy recorder), bundle load/write/validate/sanitize, redaction policy, fail-closed secret scanning, incremental SSE parser with terminal markers, WS pump, stable key-sorted JSON, HAR derivation |
| `suspect-replay` | `replay/` | Replay engine: status-only / exact-byte / semantic-JSON / semantic-SSE comparison; strongest-match request scoring |
| `suspect-infer` | `infer.rs`, `generate_spec.rs` | Schema inference from JSON values, path-parameter detection across requests, security-scheme detection, OpenAPI 3.1 synthesis from traffic JSONL/bundles |
| `suspect-tls` *(feature-gated)* | `tls/`, `server/tls_downstream.rs` | Root-CA generate/load, dynamic per-host leaf certs, CONNECT tunnel interception, passthrough globs, ALPN h2+http/1.1 |
| `suspect-credential` | `gateway/` | Opaque short-lived tokens, policy validation, credential-provider commands, server-side injection |
| `suspect-llm` *(feature-gated)* | `llm/` | Provider fingerprinting (8 providers), usage-token folding, shape fingerprints + drift grouping |

### Merged into existing crates

| Arbiter module | Destination | Notes |
|---|---|---|
| `mock/` (example gen, capture-replay matcher, fault injector, template) | `suspect-gateway::mock` | Fault injection becomes available in mock AND proxy modes; capture-replay (byte-exact strongest-match) joins spec-driven mock as a second mock source |
| `validation/` | `suspect-gateway::validate` | suspect-validate stays the rule authority; Arbiter's violation keywords (`unknown-path`, `method-not-defined`, `schema-violation`…) merge into the verdict model |
| `diff.rs` (presence-level) | `suspect-codegen::diff` | Kept as the presence pre-filter feeding the semantic diff |
| `storage/` (SQLite) | dropped | Bundles are the interchange; journal JSONL streams; SQLite only if a consumer demands it later |
| `server/`, `tui/`, `cli/`, `config/` | dropped (suspect surfaces) | Gateway gains a `/flows` API only if the TUI phase lands |

### Discarded from suspect

- `suspect-gateway`'s hand-rolled recorder (replaced by `CaptureSession`)
- `consumer_impact::RecordedExchange` (replaced by `Exchange`)
- journal `TrafficRecord` v1 write path (read-adapter kept)

---

## 3. Gateway convergence: one dispatch pipeline

`suspect-gateway` remains the single server surface. Post-fold request path:

```text
client ──▶ [TLS MITM ─ optional] ──▶ [credential gate ─ optional]
          ──▶ [fault injector ─ optional]
          ──▶ route dispatch
                ├─ mock: spec examples │ capture-replay (strongest match)
                └─ proxy/validate: forward upstream
          ──▶ CaptureSession records exact bytes (request + response + SSE/WS)
          ──▶ suspect-validate evaluates contract → verdict + violations
          ──▶ journal.append(Exchange { captured, contract, correlation })
          ──▶ respond (byte-exact to client)
```

Mode matrix after convergence (all existing suspect flags keep working):

| Mode | Engine | Addition from Arbiter |
|---|---|---|
| mock | spec examples (existing) | `--from-capture DIR`: byte-exact replay, strongest-match scoring; `--fault-*` flags |
| proxy | forward + record | `CaptureSession` replaces the recorder; TLS MITM available |
| validate | forward + contract eval | verdicts embedded in `Exchange.contract`; violation keywords unified |
| record | capture | byte-exact bundles as first-class output (`--bundle out/`) alongside journal |
| replay | replay | semantic comparison modes; bundle input |
| gateway | — | new: credential-gate mode in front of any upstream |

---

## 4. Traffic→spec synthesis (the evolution engine, completed)

The R1 feature (traffic-informed spec evolution) currently detects
*undocumented fields*. With `suspect-infer` it becomes full synthesis:

```text
suspect spec sync --from-bundle ./captures --spec openapi.yaml
  1. suspect-capture loads the bundle (byte-exact exchanges)
  2. suspect-infer synthesizes: unseen endpoints, inferred schemas
     (per-value types, enums, formats, required-ness), security schemes
  3. suspect-ir::evolution folds in undocumented-field observations
     (frequency-ranked, confidence-scored)
  4. lower both sides to IrSpec; suspect-codegen::diff computes the
     BREAKING/ADDITIVE/COSMETIC delta between spec-truth and traffic-truth
  5. output: a reviewable spec patch + drift report (semver recommendation)
```

Provenance chains to the exact captured exchanges back every proposal —
the same evidence discipline as the charte fold (source spans) and the
causal debugger (git blame).

---

## 5. Consumer impact + replay convergence

`analyze_impact(old_ir, new_ir, &[Exchange])` replaces the synthetic
`RecordedExchange`:

- request bodies are the **actual bytes** consumers sent (not synthesized)
- consumer fingerprints from real headers (user-agent, client-id, source IP)
- per-exchange verdicts under BOTH spec versions by replaying each captured
  request against the new spec's expectations (`suspect-replay` semantic
  mode) — "3 of 12 consumers break; here are the exact captured requests"

New capability unlocked: **record today, diff tomorrow**. `suspect gateway
--mode record --bundle captures/` against prod-like traffic, then any spec
change is evaluated against weeks of real traffic before merge.

---

## 6. Arazzo × capture

- **Replay steps**: a workflow step gains `x-suspect-replay: {bundle,
  exchange: {match: {...}}}` — execute a recorded request, assert
  semantic-JSON equality (or exact-byte). Deterministic regression tests
  from production traffic, no live server needed.
- **Capture runs**: `suspect test --bundle out/` journals the workflow run
  as a bundle — CI goldens for API behavior, the same role Arbiter's
  PLAN.md defines for provider contracts, extended from LLM APIs to any
  OpenAPI.

---

## 7. Frozen compatibility guarantees (inherited from Arbiter, now suspect's)

1. **Bundle format**: byte-identical manifests + NDJSON exchange encoding,
   same sha256 content addressing, digest excludes timing provenance.
   Suspect-authored bundles load in Arbiter TS/Rust and vice versa. Verified
   by porting Arbiter's `tests/roundtrip.rs` (409 unit tests + roundtrip
   suite are the porting contract, same role as charte's goldens).
2. **Redaction semantics**: same default header set, same sensitive-name
   pattern, same `__redacted__` query placeholder.
3. **Secret scanning**: same pattern set, fail-closed; findings never
   contain the full secret.
4. **SSE terminal markers**: `[DONE]`, `message_stop`, `error`,
   `response.completed|failed|incomplete`.
5. **Journal v1**: read-adapter; old journals stay queryable (causal
   debugger history preserved).

---

## 8. Phases and acceptance gates

| Phase | Deliverable | Gate |
|---|---|---|
| A | `suspect-capture`: Exchange, CaptureSession, bundle I/O + validation + sanitize, redaction, secret scan, SSE, WS, HAR derivation | Arbiter's unit tests ported green (409-test contract); roundtrip vs Arbiter-written fixture bundles byte-identical; clippy −D |
| B | `suspect-replay` + journal v2 `Exchange` + v1 read-adapter; `consumer_impact` on real exchanges | replay verdict modes green; old journal fixtures load; impact test rewritten against captured bytes |
| C | Gateway convergence: CaptureSession as record engine, verdicts in `Exchange.contract`, fault injection (mock+proxy), capture-replay mock source | gateway integration matrix (mode × feature) green; byte-exact echo test through proxy; validate-mode violations carry unified keywords |
| D | `suspect-infer` + `suspect spec sync --from-bundle` (evolution + inference + semantic diff) | synthesized spec from a recorded Plex bundle diffs against the real spec with only known gaps flagged; proposals carry exchange provenance |
| E | `suspect-tls` + `suspect-credential` (feature-gated `tls`, `credentials`) | curl through MITM proxy with generated CA: intercepted exchange captured byte-exact; untrusted client with opaque token reaches upstream, real credential never in capture |
| F | HAR export command, `suspect-llm` fingerprinting, TUI (optional) | HAR opens in a standard inspector; fingerprint parity with Arbiter on fixture traffic |

Every phase: fmt, clippy −D warnings, full workspace tests, determinism
(double-run byte-identical bundles), bench deltas recorded
(`suspect bench capture` — proxy overhead %, capture throughput, bundle
write; Arbiter's published baseline: 83–96ms startup, 1–4% proxy overhead
at c=16).

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Arbiter is pre-cutover (TS published, Rust port staged) | the Rust port's 409-test suite + roundtrip fixtures are the porting contract; bundle format is versioned and frozen |
| rustls/CA dependency weight | `suspect-tls` behind a cargo feature; core capture/replay/infer build without it |
| Exchange model migration breaks journal consumers | v2 records are additive (`skip_serializing_if`); v1 read-adapter; causal debugger queries both |
| Two mock engines during transition | parity tests first (same spec → same responses from both engines), then delete Arbiter's standalone server/mock modules |
| Credential-gateway security surface | port Arbiter's policy validation + tests unchanged; tokens opaque and short-lived; real credentials never serialized into bundles or journal (redaction-before-persistence is fail-closed) |
| Workspace bloat (22 → 27 crates) | feature-gated niche crates (`suspect-tls`, `suspect-llm`); CLI stays single binary |

---

## 10. Success criteria

1. Every Arbiter unit + roundtrip test passes inside suspect's workspace;
   suspect-authored bundles are byte-identical to Arbiter-authored ones.
2. `suspect gateway --mode validate` journals carry byte-exact bodies +
   contract verdicts in one record; causal debugger resolves a failure to
   spec line + git commit + the captured exchange.
3. `suspect spec sync --from-bundle` produces a reviewable, semver-
   classified patch from recorded traffic alone.
4. Consumer impact runs on real captured request bodies with per-consumer
   remediation.
5. An Arazzo workflow can bind a step to a captured exchange and assert
   semantic equality — deterministic regression tests from traffic.
6. TLS MITM + credential gateway round-trip with real clients (curl), with
   upstream credentials provably absent from every persisted artifact.
7. All existing suspect gates hold: deterministic outputs, benches recorded,
   clippy −D, workspace tests green.
