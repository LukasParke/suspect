# Arazzo-driven load testing — Architecture Plan

Companion to `docs/FEATURE-ROADMAP.md` and `docs/ARBITER-FOLD-PLAN.md`.
This plan adds Artillery-style load generation (`suspect load`) on top of
the existing Arazzo contract-test executor, so the **same Arazzo workflow**
that runs as a contract test also runs as a weighted user scenario under a
phased arrival profile, with latency/throughput/error metrics and SLO gates.

Status: `shipped` — phases A–E landed 2026-10-10 (see §14 for what each
phase delivered and §18 for deferred items); user guide in
[LOAD-TESTING.md](LOAD-TESTING.md). Nothing here was a greenfield bet: the
plan compiler,
executor, rex evaluator, journal, and gateway already exist; this is a
second *execution mode* over them.

Rebased 2026-10-13 onto a `main` whose executor had meanwhile gained
credential injection (`.suspect/credentials.json`, OAuth grants),
Arazzo 1.1 message steps, step timeouts, step-level `dependsOn`
scheduling and response-schema validation. The observer seam carries all
of them (built-in failures surface through `ExecObserver::on_check_failed`);
`RunOpts` gained `auth` (one token cache shared by every VU) and
`validate_responses`; `run_workflow` takes `&Plan` so the component
schemas travel with it. `suspect load` reads credentials exactly like
`suspect test` (`--credentials`) and schema-checks responses only with
`--validate-responses`. Sections below describe the design as planned
against the earlier base.

Step-level `dependsOn` resolution is shared by `suspect test` and
`suspect load` (both run through `run_workflow`). The plan compiler
normalizes `$workflows.<this>.steps.<id>` to the local step `<id>`,
turns `$workflows.<other>.steps.<id>` into an implied workflow
dependency (so `<other>` runs first, and as a once-per-run prerequisite
under load) plus a runtime check that `<id>` passed there, drops
`$sourceDescriptions.…` step references like their workflow-level
counterparts, and rejects unknown workflows/steps, malformed entries,
and self-dependencies at compile time. A step that still cannot become
ready (a cycle, or a failed or absent prerequisite step) fails the run
with a `depends-on` diagnosis instead of being counted as skipped. Before
this, the qualified form was never resolved: the step silently never
ran and the run still reported `0 failed`.

`onFailure: goto` (fixed 2026-10-14, also pre-existing on main): a goto
now re-runs the workflow from its target step onward in document order,
so the failed step is retried after the recovery steps and completed
steps after the target re-run against the fresh state. Jumps are capped
at `MAX_GOTO_JUMPS` (16) per workflow run, after which the step fails with
an `on-failure-goto` diagnosis. Before this, the "invalidate dependents"
loop could never remove anything, the failed step was dropped instead of
retried, and a goto that kept failing back into itself looped forever.

---

## 0. What exists today (review)

### Pipeline

```text
Arazzo + OpenAPI workspace --compile_plan--> Plan --run_plan--> RunSummary
   (suspect-arazzo views)   (suspect-test::plan)  (suspect-test::exec)   + TestEvent stream
                                                        │
                                              HttpClient (trait) ── LiveTransport (CLI, reqwest)
                                                                 ── CannedTransport / ReplayTransport
```

| Layer | File | What it gives us for load |
|---|---|---|
| Plan compiler | `crates/suspect-test/src/plan.rs` | `Plan`/`WfPlan`/`StepPlan` are `Clone`; ops resolved to canonical `method + path` via `IrSpec`; params/body/outputs kept as `Rex` for late evaluation; criteria pre-compiled to `CriterionKind`. `Arc<Plan>` can be shared across thousands of tasks with zero recompilation. |
| Executor | `crates/suspect-test/src/exec.rs` | `run_step` already does everything one VU request needs: param placement, `${…}` template substitution, body rendering, criteria evaluation, output capture. Measured at 3.6 µs/step overhead (`README.platform.md`). |
| Transport | `crates/suspect-cli/src/commands/http.rs` | `LiveTransport` (reqwest) is the only live client; `HttpClient` is object-safe and `Send + Sync`. |
| Reporters | `crates/suspect-test/src/reporters.rs` | console / JUnit / NDJSON over the event log. Pure functions; easy to add siblings. |
| Journal | `crates/suspect-journal` | `RunSummaryRecord`, `TrafficRecord`, sinks (stdout/file/vec). `Record` is an additive tagged enum. |
| Gateway | `crates/suspect-gateway` | Mock mode sustains 95–127k req/s locally; proxy has fault injection (`--delay-ms/--delay-pct/--error-status/--error-pct`). A deterministic, self-contained load target. |
| Concurrency precedent | `crates/suspect-cli/src/commands/fuzz.rs` | `JoinSet` + `IN_FLIGHT` cap over `Arc<LiveTransport>`. |

### What blocks load generation right now

1. **Executor is one-shot and borrowing.** `run_plan` drives one future per
   workflow with a hand-rolled `poll_fn` because the futures borrow `&Plan`
   and `&dyn HttpClient`; nothing is `'static`, so nothing can be
   `tokio::spawn`ed across the multi-thread runtime. `run_workflow` and
   `run_step` are private.
2. **No input binding API.** `WfPlan.inputs` comes only from the Arazzo
   `parameters` block. `workflow_inputs` says runtime-expression values "are
   bound by the caller at execution time instead" — but no binding path
   exists (the CLI has no `--input`). Per-VU data (CSV rows, generated
   values, setup-workflow outputs) needs exactly this.
3. **Observation is a chatty event stream.** Every step emits 4–6
   `TestEvent`s with owned `String`s through a bounded(256) channel that
   applies backpressure to the executor. Fine for a dozen steps; wrong for
   50k req/s. Timing is `duration_ms: u64` — too coarse for p99 work.
4. **Transport defaults.** reqwest built with `default-features = false`
   (no `http2`, no `cookies`), hard-coded 30 s timeout, one shared client.
5. **Arazzo features parsed but not executed:** `dependsOn`, workflow-level
   `outputs`, `$workflows.<wf>.…` expressions, `retry`/`end` actions,
   `onSuccess` goto. Setup/teardown flows need the first three.

None of these are architectural problems — they are the seams the load
mode has to open, and opening them improves the contract-test path too.

---

## 1. Goal, non-goals, and the Artillery mapping

**Goal:** `suspect load profile.yaml` runs weighted Arazzo workflows as
virtual users under phased arrival rates, reports Artillery-grade metrics
(request rate, response time percentiles, status codes, VU lifecycle,
error classes), and fails CI on SLO breaches — while still evaluating every
response against the workflow's `successCriteria`, so the headline number
is *contract* error rate under load, not just HTTP error rate.

### Concept mapping

| Artillery | suspect equivalent | Source of truth |
|---|---|---|
| `config.target` | `target` (overridable `--base-url`) | profile |
| `config.phases[]` (`arrivalRate`, `rampTo`, `arrivalCount`, `pause`, `maxVusers`, `name`) | `phases[]` — same four kinds, same field names | profile |
| `config.environments` + `-e` | `environments:` map + `--env` | profile |
| `config.payload` (CSV → variables) | `data:` → workflow `inputs` (`order: random\|sequence\|unique`) | profile |
| `config.variables` | `inputs:` literal/array pools per scenario | profile |
| `config.ensure` | `ensure:` thresholds/conditions/`maxErrorRate` | profile |
| `config.http.timeout`, `pool`, `maxSockets` | `http:` block (`timeout`, `http2`, `per_vu_connections`, `cookies`) | profile |
| `before` / `after` | `before: <workflowId>` / `after: <workflowId>`; Arazzo `dependsOn` honoured | profile + Arazzo |
| `scenarios[].flow` | an Arazzo **workflow** (steps = requests) | Arazzo |
| `scenarios[].weight`, `name` | `scenarios[]: {workflow, weight, name}` | profile |
| `flow[].get/post/…` | step `operationId`/`operationPath` resolved through the OpenAPI contract | Arazzo |
| `capture: {json, as}` | step `outputs: {name: $response.body#/…}` | Arazzo |
| `expect` plugin | step `successCriteria` (default 2xx) | Arazzo |
| `think: 2` | step `x-suspect-think: 2s` **or** profile-level `think:` per scenario | Arazzo ext. / profile |
| `loop` | out of scope v1 (scenario-level `iterations` covers most cases) | — |
| `$uuid`, `$testId`, `$environment` | `$inputs.__vu`, `$inputs.__run`, `$inputs.__env` injected per VU | runtime |
| periodic 10 s report + final | same cadence; `--report-interval` | runtime |
| `--output report.json` + `artillery report` | `--json out.json` (versioned schema); HTML later | runtime |
| `run-fargate` / `run-lambda` | out of scope (single-node; design keeps aggregation mergeable) | — |

### Why not transpile to Artillery?

An `Arazzo → Artillery YAML` exporter would be cheap, but it loses the three
things that make this worth doing: contract-resolved requests (path
templates, parameter placement from the OpenAPI document), contract
assertions on every response, and Rust throughput (Artillery saturates at a
few hundred req/s per Node worker; suspect's gateway bench shows the
executor is not the bottleneck). **Decision: native runner only.** No
exporter is planned.

### Non-goals (v1)

- Distributed/multi-node load (keep aggregates mergeable; no coordinator).
- Browser/WebSocket/Socket.IO engines.
- HTML report rendering (JSON first; HTML via `suspect-gen` template later).
- Artillery YAML import *or* export.

---

## 2. Where the load profile lives

Two candidates; the recommendation is both, with a clear split:

| Concern | Lives in | Why |
|---|---|---|
| What a user does (requests, assertions, captured values, think time between *specific* steps) | **Arazzo document** (+ `x-suspect-think` step extension) | Portable, lint/LSP-aware, already the contract-test artifact. Arazzo 1.0 permits `x-` extensions on every object. |
| How many users, how fast, with what data, against which target, under what SLOs | **Load profile file** (`*.load.yaml`) referencing workflows by id | Load shape changes per environment/run; keeping it out of Arazzo keeps the workflow reusable as a contract test unchanged. Mirrors Artillery's `config`/`scenarios` split. |

Profile-less quick runs are also supported from flags alone
(`suspect load flows.arazzo.yaml --workflow checkout --rate 50 --duration 60s`).

### Profile schema (v1)

```yaml
# checkout.load.yaml
suspectLoad: "1.0"                     # root discriminator, like `openapi:` / `arazzo:` / `overlay:`
arazzo: ./flows.arazzo.yaml            # relative to this file
target: http://localhost:8080          # --base-url overrides
environments:
  staging: { target: https://staging.example.com }

phases:
  - { name: warmup,  duration: 30s, arrivalRate: 5 }
  - { name: ramp,    duration: 2m,  arrivalRate: 5, rampTo: 100 }
  - { name: sustain, duration: 5m,  arrivalRate: 100, maxVusers: 500 }
  - { pause: 10s }
  - { name: spike,   duration: 20s, arrivalCount: 2000 }
  # closed-loop phase (k6-style): N long-lived VUs looping until the phase ends
  - { name: soak,    duration: 10m, vus: 50 }

before: seed-catalog                   # workflowId, run once; outputs → $workflows.seed-catalog.outputs.*
after:  purge-catalog

scenarios:
  - name: browse
    workflow: browse-and-search
    weight: 6
    think: { between_steps: 500ms..2s }   # uniform jitter; step x-suspect-think overrides
  - name: checkout
    workflow: create-order
    weight: 3
    inputs:
      currency: [USD, EUR, GBP]          # pool → random pick per VU
    data:                                # CSV/NDJSON rows → inputs, one row per VU
      path: ./users.csv
      fields: [email, password]
      order: unique                      # random | sequence | unique (fail when exhausted)
  - name: admin-report
    workflow: nightly-report
    weight: 1
    iterations: 3                        # VU repeats the workflow 3× (new step state each time)

http:
  timeout: 10s
  http2: true
  per_vu_connections: false              # true = new pool + cookie jar per VU (realistic TLS/session load)
  cookies: true

ensure:
  maxErrorRate: 1                        # % of requests failing transport OR criteria
  thresholds:
    - http.response_time.p95: 250        # ms
    - http.response_time.p99: 800
    - scenario.checkout.step.place-order.response_time.p99: 1200
  conditions:
    - expression: http.request_rate > 90 and vusers.failed == 0
      strict: true                       # false = warn only

report:
  interval: 10s
  json: ./load-report.json
  junit: ./load-junit.xml
```

Validation (`suspect validate checkout.load.yaml`): workflow ids exist in
the Arazzo doc; `before`/`after` are not also scenarios; weights > 0; phase
kinds mutually exclusive; `ensure` metric names resolve; data files exist and
`fields` match the referenced workflow's `inputs` schema (missing required
inputs are an error, extra fields a warning).

---

## 3. Architecture

### Placement

A new module tree inside `suspect-test` (crate description extended to
"…planner, executor, load generator, reporters, and transports"):

```text
crates/suspect-test/src/
  exec.rs            # refactored: observer seam, owned/spawnable VU run, input binding
  load/
    mod.rs           # pub API: LoadProfile, run_load, LoadSummary
    profile.rs       # schema, YAML load, validation
    schedule.rs      # phase → arrival stream; maxVusers; closed-loop
    vu.rs            # one virtual user: pick scenario, bind inputs, run, think
    data.rs          # CSV/NDJSON feeders (random/sequence/unique), input pools
    metrics.rs       # Sample, Aggregator, windows, histograms
    ensure.rs        # SLO evaluation over the final/interval aggregates
    report.rs        # console (interval + final), JSON schema v1, JUnit
crates/suspect-cli/src/commands/load.rs   # `suspect load`
```

Rationale: `fuzz`, `stateful`, and `evolved_fuzz` already live in
`suspect-test` as "strategies over the executor"; load is another one and
needs the executor's internals during the refactor. If `load/` grows an HTML
renderer or distribution later, it splits to `suspect-load` with no API
change (everything is behind `suspect_test::load`).

New dependencies: `hdrhistogram` (pure Rust, the standard for latency
percentiles, mergeable — keeps the door open for multi-node) and `csv`
(quoting/escaping done right; the repo otherwise hand-rolls small parsers
but CSV edge cases are not worth it). The profile is parsed with the
existing `suspect-low` parser — no YAML dependency — so it gets
source-located diagnostics and LSP support for free (see §12), read through
a small typed view in `load/profile.rs` the way `suspect-arazzo::model`
reads Arazzo. `rand` and `tokio` are already present. reqwest gains the
`http2` and `cookies` features.

### Data flow at runtime

```text
LoadProfile ──▶ Scheduler (phases) ──arrival──▶ spawn VU task
                                                   │  pick scenario by weight (seeded RNG)
                                                   │  bind inputs (pools / data row / before-outputs / __vu)
                                                   ▼
                                     run_workflow_owned(Arc<Plan>, wf_idx, inputs, http, observer)
                                                   │  per step: build → send → criteria → outputs → think
                                                   ▼
                                     LoadRecorder (ExecObserver) ──Sample──▶ Aggregator task
                                                                                 │  cumulative + interval window
                                                                                 ├─▶ console every `interval`
                                                                                 ├─▶ journal NDJSON snapshots
                                                                                 └─▶ final: JSON, JUnit, ensure → exit code
```

---

## 4. Executor refactor (`exec.rs`)

These changes are the foundation and are independently valuable for
`suspect test`.

### 4.1 Observer seam

Replace the hard-wired `mpsc::Sender<TestEvent>` with a trait; the current
behaviour becomes one implementation.

```rust
#[async_trait]
pub trait ExecObserver: Send + Sync {
    async fn on_workflow_started(&self, wf: &WfPlan, vu: VuId) {}
    async fn on_request(&self, cx: &StepCx<'_>, req: &HttpRequest) {}
    async fn on_response(&self, cx: &StepCx<'_>, sample: &StepSample) {}   // always fires, incl. transport errors
    async fn on_criterion(&self, cx: &StepCx<'_>, crit: &CriterionPlan, result: Result<(), (String, String)>) {}
    async fn on_output(&self, cx: &StepCx<'_>, key: &str, value: &serde_json::Value) {}
    async fn on_workflow_done(&self, wf: &WfPlan, vu: VuId, outcome: &WfOutcome) {}
}

pub struct StepSample {
    pub status: Option<u16>,              // None on transport failure
    pub latency: Duration,                // µs precision; send → last body byte
    pub ttfb: Option<Duration>,           // send → headers (LiveTransport fills it)
    pub bytes_out: usize, pub bytes_in: usize,
    pub outcome: StepVerdict,             // Passed | CriteriaFailed(n) | Transport(ErrorClass)
}
pub enum ErrorClass { Timeout, Connect, Tls, Reset, Other }
```

- `EventObserver(mpsc::Sender<TestEvent>)` reproduces today's stream
  byte-for-byte (NDJSON goldens in `tests.rs` and the VS Code runner that
  parses it must not change). `TestEvent::ResponseGot` keeps `duration_ms`
  and additionally gains `duration_us`.
- `LoadRecorder` implements only `on_response` and `on_workflow_done`, is
  non-blocking (`try_send` into a large bounded channel; drops are counted
  and reported as `metrics.dropped`), never applies backpressure to VUs.
- `ErrorClass` is derived in `LiveTransport` from reqwest error kinds so
  the aggregator can report `errors.ETIMEDOUT`-style breakdowns.

### 4.2 Spawnable, owned workflow runs

```rust
pub struct RunOpts { pub base_url: Arc<str>, pub inputs: InputMap, pub think: ThinkPolicy, pub vu: VuId }

pub async fn run_workflow_owned(
    plan: Arc<Plan>, wf_idx: usize, opts: RunOpts,
    http: Arc<dyn HttpClient>, observer: Arc<dyn ExecObserver>,
) -> WfOutcome   // { ok, passed, failed, skipped, outputs: Map, duration }
```

`run_plan` becomes a thin wrapper: for each workflow, `run_workflow_owned`
with inputs from `WfPlan.inputs` + defaults, observer = `EventObserver`.
The `poll_fn` driver goes away (workflows become `JoinSet` tasks), which
also removes the "deliberately non-Send" caveat in `commands/test.rs` and
`platform_bench.rs`.

### 4.3 Input binding

Precedence (lowest → highest): schema `default` → Arazzo `parameters`
literal → caller-supplied `RunOpts.inputs`. Expose on the CLI as
`suspect test --input key=value --inputs file.json` (fixes the latent gap
regardless of load). Reserved runtime inputs `__vu` (u64), `__run`
(uuid-ish string), `__env` are injected by the load runner.

### 4.4 Setup/teardown and cross-workflow outputs

- Honour `dependsOn`: topologically run prerequisite workflows before the
  dependent one (contract mode: within `run_plan`; load mode: once, before
  arrivals begin).
- Capture workflow-level `outputs` into `WfOutcome.outputs`.
- Add `Rex::Workflows { workflow, step: Option<String>, key }` to
  `suspect-rex` and a `workflows_outputs` field on `RexCtx`, so scenarios
  can read `$workflows.seed-catalog.outputs.token`. `suspect-arazzo`'s
  validator already walks these references.

### 4.5 Think time

`StepPlan.think: Option<ThinkSpec>` compiled from `x-suspect-think`
(`2s`, `500ms..2s`). `ThinkPolicy::Ignore` (default for `suspect test`) or
`ThinkPolicy::Honour { default: Option<Range<Duration>> }` (load). Sleeps
happen *after* a step's outputs are captured and are not counted in step
latency.

---

## 5. Scheduler (`load/schedule.rs`)

Open-model arrivals, as in Artillery: a VU is created on schedule
regardless of whether earlier VUs finished.

| Phase | Arrival schedule |
|---|---|
| `arrivalRate: r` | one arrival every `1/r` s (`jitter: none`) or exponential inter-arrivals with mean `1/r` (`jitter: poisson`, default `none` to match Artillery) |
| `arrivalRate: r, rampTo: R` | instantaneous rate `r(t) = r + (R−r)·t/duration`; next arrival at `t + 1/r(t)` (continuous ramp — strictly smoother than Artillery's per-second steps; documented) |
| `arrivalCount: n` | `n` arrivals spaced `duration/n` |
| `pause: d` | no arrivals; in-flight VUs continue |
| `vus: n` (closed loop; in v1) | `n` long-lived tasks each looping the scenario picker until the phase ends; `iterations` on the phase caps total loops instead of time |

- `maxVusers`: a `tokio::sync::Semaphore`; an arrival with no permit is
  **skipped** and counted as `vusers.skipped` (Artillery semantics — it
  does not queue, which would distort the open model).
- Clock: `tokio::time::Instant` throughout so tests can `pause()`/`advance()`.
- Drain: after the last phase, wait for in-flight VUs up to
  `--grace 30s`, then abort and count the rest as `vusers.aborted`.
- Ctrl-C: first signal stops arrivals and drains; second aborts. Final
  report still prints (partial, flagged `interrupted: true`).
- Seeded RNG (`--seed`) for scenario selection, pools, jitter, and
  `order: random` data rows → reproducible runs.

---

## 6. Virtual user (`load/vu.rs`)

Per arrival:

1. Pick a scenario by weight (cumulative-weight table, one RNG draw).
2. Bind inputs: scenario `inputs` pools → one `data` row → `before`
   outputs (as `$workflows.<id>.outputs.*`) → `__vu/__run/__env`.
3. Acquire a client: shared `Arc<dyn HttpClient>` by default;
   `per_vu_connections: true` builds a fresh `LiveTransport` (own pool,
   own cookie jar) and drops it on completion.
4. `run_workflow_owned` `iterations` times; stop the VU on the first
   failed iteration unless `continue_on_failure: true`.
5. Report `vusers.completed` / `vusers.failed` and
   `vusers.session_length` (histogram).

A VU failing a criterion is a *failed VU* and counts toward
`maxErrorRate` at the request level (that step) — the remaining steps are
skipped exactly as in contract mode (`onFailure: goto` still honoured,
with the same `MAX_GOTO_JUMPS` bound per iteration).

---

## 7. Metrics (`load/metrics.rs`)

Compact `Sample` (~64 B) per step response → single aggregator task.
Budget: 50k samples/s is ~3 MB/s; one task keeps up. Measured, not assumed
(see §13 benches).

```rust
pub struct Sample {
    pub t: Instant, pub scenario: u16, pub step: u16, pub op: u16,  // indices into the plan
    pub status: Option<u16>, pub latency_us: u32, pub ttfb_us: u32,
    pub bytes_in: u32, pub bytes_out: u32, pub verdict: StepVerdict,
}
```

The aggregator keeps **cumulative** and **interval** (reset every
`report.interval`) views of:

| Metric (name as reported) | Structure |
|---|---|
| `http.requests`, `http.responses`, `http.request_rate` | counters; rate = Δ/interval |
| `http.codes.<code>` | `BTreeMap<u16, u64>` |
| `http.response_time.{min,max,mean,median,p75,p90,p95,p99,p99.9}` | `hdrhistogram` (1 µs → 60 s, 3 sig figs) |
| `http.ttfb.*`, `http.downloaded_bytes`, `http.uploaded_bytes` | histogram / counters |
| `errors.<class>` (`timeout`, `connect`, `tls`, `reset`, `other`) | counters |
| `contract.failures`, `contract.failures.<scenario>.<step>` | counters — the suspect-specific headline |
| `vusers.created`, `.created_by_name.<scenario>`, `.completed`, `.failed`, `.skipped`, `.aborted` | counters |
| `vusers.session_length.*` | histogram |
| `scenario.<name>.step.<id>.response_time.*`, `.codes.*` | per-step histograms (bounded by plan size) |
| `op.<METHOD> <path>.response_time.*` | per-operation rollup across scenarios |
| `metrics.dropped` | counter (channel full) |

Histograms are mergeable (`hdrhistogram::Histogram::add`), so a future
multi-node mode only needs to ship serialized histograms to a coordinator.

---

## 8. Reporting (`load/report.rs`)

- **Console, interval**: Artillery-style block every `report.interval` —
  phase name, elapsed, `vusers.created/completed/failed`, request rate,
  codes, response-time quantiles, top error classes, contract failures.
  Width-aligned like `reporters::console`.
- **Console, final**: cumulative version plus a per-scenario and
  per-step table, then the `ensure` verdicts (`✓ p95 212ms < 250ms`,
  `✗ maxErrorRate 2.3% > 1%`).
- **JSON** (`report.json: {version: 1, run, profile, phases[], intervals[], aggregate, ensure[]}`)
  — the machine artifact; schema documented alongside the cassette format.
- **NDJSON / journal**: each interval snapshot is emitted as a new
  `Record::LoadSnapshot(LoadSnapshotRecord)` (additive variant), and the
  run closes with the existing `RunSummaryRecord { run_kind: "load", … }`
  where `passed/failed` = ensure checks. `--report ndjson` streams the same
  records to stdout for the VS Code extension.
- **JUnit**: one `<testcase>` per `ensure` check, so CI dashboards show
  *which* SLO broke. Reuses the escaping/shape of `reporters::junit`.
- **Exit codes**: `0` all ensure checks pass (or none declared and no
  transport failures), `1` an `ensure` check with `strict: true` failed
  (default strict), `2` configuration/compile error, `130` interrupted
  before any phase completed.

---

## 9. `ensure` evaluation (`load/ensure.rs`)

Evaluated once over the **cumulative** aggregate at the end (Artillery
semantics). Supported forms:

- `maxErrorRate: <pct>` — `(transport failures + criteria failures) / http.requests`.
- `thresholds: [{<metric>: <max>}]` — any `*.response_time.pN`,
  `*.rate`, `*.codes.*`, `vusers.*`, `contract.*` name from §7; value is a
  maximum (ms for time metrics).
- `conditions: [{expression, strict}]` — a small boolean expression
  grammar: metric names, numeric literals, `< <= > >= == !=`, `and`,
  `or`, `not`, parentheses. Hand-rolled recursive-descent parser (the repo
  style), with source-located errors at validation time.
- Per-phase checks are **not** in v1 (interval metrics are reported, not
  gated); noted as a follow-up.

---

## 10. CLI surface

```console
$ suspect load checkout.load.yaml                       # full profile
$ suspect load checkout.load.yaml -e staging --seed 7
$ suspect load checkout.load.yaml --base-url http://127.0.0.1:8080 --json out.json --junit out.xml
$ suspect load flows.arazzo.yaml --workflow create-order --rate 50 --duration 2m --max-vus 200
$ suspect load flows.arazzo.yaml --workflow browse --vus 20 --iterations 100      # closed loop
$ suspect load checkout.load.yaml --dry-run           # validate + print the resolved schedule, send nothing
$ suspect load checkout.load.yaml --no-assert         # skip successCriteria (pure HTTP load)
```

Flags mirror profile keys; a flag overrides the profile. `--report
text|ndjson`, `--report-interval`, `--grace`, `--timeout`, `--http1`
(disable h2), `--insecure` (TLS `danger_accept_invalid_certs`), `--input`.

The `Command::Load` variant follows the existing clap pattern in
`crates/suspect-cli/src/lib.rs`; `commands/load.rs` mirrors
`commands/test.rs` (workspace load → compile → runtime → reporters →
journal `run_summary`).

---

## 11. Transport changes (`commands/http.rs`)

- `LiveTransport::builder()` with `timeout`, `http2` (ALPN; reqwest
  `http2` feature), `cookie_store`, `pool_max_idle_per_host`,
  `tcp_nodelay`, `danger_accept_invalid_certs`, `user_agent`
  (`suspect-load/<version>`).
- Fill `StepSample.ttfb` by timing `send()` vs `bytes()` separately;
  record `bytes_in` from the body length.
- Map reqwest errors to `ErrorClass` (`is_timeout`, `is_connect`,
  TLS/`reset` via source inspection).
- A `DelayTransport<T>` decorator in `suspect-test::transports` (fixed or
  jittered `tokio::time::sleep` before delegating) for deterministic tests
  of the scheduler and metrics without a socket.

---

## 12. Validation and editor integration

- Parse the profile with `suspect-low` so diagnostics carry byte ranges;
  add `SpecFamily::Load10` to `LowDoc::sniff_family` keyed on the
  `suspectLoad` root field (same shape as the `arazzo`/`overlay` branches).
  `suspect validate` and the LSP then report profile
  errors at the right line (unknown workflow id, bad duration literal,
  unresolvable `ensure` metric), and `$ref`-style goto-definition from
  `workflow: create-order` into the Arazzo document is a small addition to
  the existing cross-file navigation.
- `suspect lint`: a rule warning when a workflow referenced by a load
  scenario has no `successCriteria` beyond the 2xx default (weak contract
  signal under load), and one when `x-suspect-think` appears in a
  workflow never referenced by a profile (dead config).
- VS Code: `runner.ts` already parses `suspect test --report ndjson`; a
  "Run as load test" lens reuses it to stream `LoadSnapshot` records into
  a status bar / output channel. Optional, after the CLI lands.

---

## 13. Testing and benches

Unit/integration (all in-process, deterministic, `tokio::time::pause`):

| Test | Asserts |
|---|---|
| `schedule::constant_rate_emits_r_per_second` | `arrivalRate: 10, duration: 5s` → 50 arrivals ±0 under paused time |
| `schedule::ramp_is_linear` | arrivals in each fifth of the window grow monotonically; total matches the integral |
| `schedule::arrival_count_spacing` / `pause_emits_nothing` | counts and gaps |
| `schedule::max_vusers_skips_not_queues` | with a `DelayTransport` longer than the inter-arrival gap, `vusers.skipped > 0` and concurrency never exceeds the cap |
| `vu::weights_distribute` | 10k picks with seed → within 2 % of 6/3/1 |
| `vu::inputs_precedence_and_reserved_keys` | defaults < parameters < data row < pools; `__vu` unique per VU |
| `data::unique_order_exhaustion_fails_fast` | third VU on a 2-row file errors with a clear message |
| `exec::observer_event_stream_unchanged` | NDJSON golden from `tests.rs` byte-identical through `EventObserver` |
| `exec::depends_on_runs_prerequisites_once` and `workflows_outputs_visible` | `$workflows.seed.outputs.token` resolves in scenario steps |
| `metrics::histogram_percentiles_and_merge` | known distribution → p50/p95/p99 exact within histogram precision; merge of two windows equals the cumulative |
| `ensure::expression_grammar` | precedence, errors with byte ranges, strict vs non-strict exit mapping |
| `report::console_and_json_goldens` | fixed seed + `CannedTransport` → golden files (same discipline as the reporters tests) |
| `cli::load_against_gateway_mock` | spin `suspect-gateway` mock on port 0, run a 3-phase profile for 3 s, assert `http.responses == http.requests`, codes all 2xx, ensure passes |
| `cli::load_detects_fault_injection` | same, with `--error-pct 20 --delay-ms 300` on the gateway → `maxErrorRate: 1` fails, exit 1, JUnit shows the breached check |

Benches (`crates/suspect-cli/examples/feature_bench.rs` + `cargo bench`):

| Budget | Target |
|---|---|
| Load-generator overhead per request (no network: `CannedTransport`) | ≤ 15 µs (executor 3.6 µs + sample + scheduling) |
| Sustained rate against local gateway mock, 8 cores | ≥ 20k req/s with p99 generator-side overhead < 1 ms |
| Aggregator ingest | ≥ 200k samples/s single task |
| Memory at 5k concurrent VUs | < 200 MB RSS |

Budgets are recorded in `README.platform.md` with the same honest
"budget / achieved" table as the existing rows.

---

## 14. Phases and gates

Status per phase (2026-10-10): **A ✓ B ✓ C ✓ D ✓ (retry action deferred) E ✓ F partial** (`Load10` family, `check` label and LSP profile diagnostics shipped; lint rules and the VS Code lens are deferred).

| Phase | Deliverable | Gate |
|---|---|---|
| A — executor seams | `ExecObserver`, `run_workflow_owned`, input binding (`--input`), µs timing, `ErrorClass`, `DelayTransport`; `run_plan` rewritten on top | existing `suspect-test` + CLI tests green; NDJSON golden byte-identical; `platform_bench` executor budget unchanged |
| B — profile + scheduler + VU | `load::profile` (`suspect-low` typed view + validation, `SpecFamily::Load10`), `schedule` (all four open-loop kinds + closed-loop `vus:`), `vu`, `data`; `suspect load --dry-run`; flag-only quick run | schedule/weights/inputs tests incl. a closed-loop test (`vus: 4, duration: 2s` under paused time → exactly 4 concurrent, loops stop at the boundary); `--dry-run` prints resolved phases and scenario table |
| C — metrics + reports + ensure | `metrics`, `report` (console/JSON/JUnit/journal), `ensure`; exit codes | goldens; `load_against_gateway_mock` and `load_detects_fault_injection` green |
| D — Arazzo completeness | `dependsOn`, workflow outputs, `$workflows.…` rex, `x-suspect-think`, `before`/`after`, `retry` action | cross-workflow tests; contract mode also gains these |
| E — transport + perf | reqwest builder (h2, cookies, per-VU), ttfb, benches in `feature_bench.rs`, `README.platform.md` rows | bench budgets met or honestly recorded |
| F — editor/lint (optional) | LSP publishes the Phase B profile diagnostics + goto-definition into the Arazzo doc, lint rules, VS Code lens | diagnostics land on the right byte range; lens streams snapshots |

Every phase: `cargo fmt`, `clippy -D warnings`, workspace tests, docs build
(`#![deny(missing_docs)]` holds in `suspect-test`), determinism (same seed →
same JSON report modulo timestamps and wall-clock-dependent fields).

---

## 15. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Coordinated omission (slow responses hide the load the generator *should* have applied) | Open-model arrivals by design; report `vusers.skipped` loudly; expose `--jitter poisson`; document that closed-loop `vus:` phases are subject to it |
| Generator becomes the bottleneck and the report blames the server | Generator-side overhead is a bench budget; report `metrics.dropped`; `--dry-run` + gateway-mock self-test documented as the calibration step |
| Observer refactor perturbs contract-test output consumed by the VS Code extension | Byte-identical NDJSON golden is a Phase A gate |
| Per-VU clients exhaust ephemeral ports / file descriptors | Default shared pool; `per_vu_connections` logs an fd/ulimit hint; `maxVusers` recommended in validation when it is on |
| Credentials in load profiles | Profile supports `${ENV_VAR}` in `inputs`/headers resolved at load time; journal snapshots never include inputs; `Redactor` applies if traffic records are ever enabled |
| Histogram memory with many per-step histograms | Per-step histograms are bounded by plan size (steps × scenarios); 3-sig-fig HDR at 60 s range is ~ 100 KB each; cap and warn above 512 |
| Scope creep toward a general load tool | Scenarios are *only* Arazzo workflows; no ad-hoc URL steps. If someone wants that, they write a one-step Arazzo workflow |

---

## 16. Decisions

Resolved by the owner (2026-10-10):

| # | Decision | Outcome |
|---|---|---|
| 1 | Native runner vs. transpile to Artillery | **Native runner only.** No exporter, no importer (§1 non-goals). |
| 2 | Profile parser | **`suspect-low`** + typed view in `load/profile.rs`; `SpecFamily::Load10` keyed on `suspectLoad`. No `serde_yaml`. |
| 3 | Closed-loop `vus:` phases | **In v1** (Phase B), alongside the four Artillery open-loop kinds. |

Defaulting to the recommendation unless overridden:

| # | Decision | Default |
|---|---|---|
| 4 | Think-time in `suspect test` | **Ignored** (`ThinkPolicy::Ignore`); contract runs stay fast and deterministic. `suspect load` honours it. |
| 5 | Journal record for interval snapshots | **Typed `Record::LoadSnapshot` variant**; the enum is additive and consumers filter by `kind`. |

---

## 17. Success criteria

1. One Arazzo document serves both `suspect test` (unchanged output) and
   `suspect load` (phased, weighted, SLO-gated) with no edits.
   **Met** — `examples/load/flows.arazzo.yaml`; event-stream golden guards
   `suspect test` output.
2. A 3-phase profile against `suspect gateway --mode mock` runs green;
   the same profile against the gateway with `--error-pct 20` fails
   `maxErrorRate` with a JUnit testcase naming the breached check.
   **Met** — `crates/suspect-cli/tests/load.rs`.
3. Reports expose contract-failure rate separately from transport/HTTP
   error rate. **Met** — `contract.*` vs `errors.*` vs `http.error_rate`.
4. Generator overhead and sustained-rate budgets are met and recorded in
   `README.platform.md`. **Met** — ~4 µs CPU/request, 43k req/s, 64M
   samples/s aggregator.
5. All existing gates hold: clippy −D, workspace tests, docs, determinism.
   **Met** for every touched crate; one pre-existing `HEAD` failure
   (`suspect-cli` fuzz unit test, macOS `/var` canonicalization) is
   unrelated and unchanged.

## 18. Deferred / follow-ups

- Arazzo `retry` / `end` actions and `onSuccess` goto in the executor.
- Lint rules from §12 (weak criteria under load; orphaned
  `x-suspect-think`) and the VS Code "Run as load test" lens.
- Per-phase `ensure` (interval metrics are reported, not gated).
- HTML report via a `suspect-gen` template over the JSON report.
- Recorder batching per user: today a saturated host drops samples
  (reported as `metrics.dropped`); batching would raise the ceiling.
- Multi-node runs: histograms and windows are mergeable; a coordinator
  and transport are not written.
- `${ENV_VAR}` interpolation in profile `inputs` (security-scheme
  credentials are covered by the shared credentials file, which already
  interpolates `${VAR}`; `--input` flags and `--inputs` cover the rest).
