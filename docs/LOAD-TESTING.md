# Load testing with `suspect load`

`suspect load` runs Arazzo workflows as weighted virtual-user scenarios
under a phased arrival profile — the Artillery model — and reports
request rate, latency percentiles, status codes, user lifecycle and
error classes. Every response is still checked against the workflow's
`successCriteria`, so the headline number is *contract* failure rate
under load, not just HTTP errors. SLO gates (`ensure`) decide the exit
code. Design and rationale: [LOAD-TESTING-PLAN.md](LOAD-TESTING-PLAN.md).

## Quick start

```console
$ suspect gateway examples/load/petstore.openapi.yaml -p 8080 --mode mock
$ suspect load examples/load/petstore.load.yaml --dry-run      # resolve, send nothing
$ suspect load examples/load/petstore.load.yaml -e smoke       # 5s smoke environment
$ suspect load examples/load/petstore.load.yaml --json out/report.json --junit out/load.xml
```

Profile-less, straight from an Arazzo document:

```console
$ suspect load flows.arazzo.yaml --workflow browse --rate 50 --duration 2m --max-vus 300
$ suspect load flows.arazzo.yaml --workflow checkout --vus 20 --iterations 500
```

The bundled example (`examples/load/`) has a Petstore spec, three
workflows (`sign-in`, `browse`, `create-and-cleanup`) and a six-phase
profile. The same `flows.arazzo.yaml` runs unchanged under `suspect test`.

## Split of concerns

| What a user does | How many users, how fast, with what data, under what SLOs |
|---|---|
| **Arazzo document**: steps, `successCriteria` (= Artillery `expect`), `outputs` (= `capture`), `dependsOn`, `x-suspect-think` | **Load profile** (`*.load.yaml`): phases, weighted scenarios, input pools, data files, `before`/`after`, `http`, `ensure`, `report`, `environments` |

## Profile reference

```yaml
suspectLoad: "1.0"                     # root discriminator
arazzo: ./flows.arazzo.yaml            # relative to this file
target: http://localhost:8080          # --base-url overrides
environments:                          # -e NAME swaps target and/or phases
  staging: { target: https://staging.example.com, phases: [...] }

phases:                                # run in order
  - { name: warmup,  duration: 30s, arrivalRate: 5 }            # N new users/s
  - { name: ramp,    duration: 2m,  arrivalRate: 5, rampTo: 100 }  # linear ramp
  - { name: sustain, duration: 5m,  arrivalRate: 100, maxVusers: 500 }  # cap: extra arrivals are skipped, not queued
  - { pause: 10s }                                               # no arrivals
  - { name: spike,   duration: 20s, arrivalCount: 2000 }         # fixed total, evenly spread
  - { name: soak,    duration: 10m, vus: 50 }                    # closed loop: 50 users looping
  - { name: batch,   duration: 1h,  vus: 8, iterations: 1000 }   # ends at 1000 loops or 1h

before: seed-catalog                   # workflow run once before arrivals
after:  purge-catalog                  # workflow run once after the drain

scenarios:
  - name: browse
    workflow: browse-and-search        # Arazzo workflowId
    weight: 6                          # share = weight / total weight
    think: { between_steps: 500ms..2s } # default pause after each non-final step
  - name: checkout
    workflow: create-order
    weight: 3
    inputs:
      currency: [USD, EUR, GBP]        # pool → one value per user
      region: eu                       # literal
    data:                              # one row per user → inputs
      path: ./users.csv                # CSV (needs fields) or .ndjson/.jsonl
      fields: [email, password]
      order: unique                    # random (default) | sequence | unique (fail when exhausted)
      skipHeader: true
    iterations: 3                      # repeat the workflow per user
    continue_on_failure: false

http:
  timeout: 10s
  http2: true
  per_vu_connections: false            # true = fresh pool + cookie jar per user
  cookies: true
  insecure: false

ensure:                                # evaluated over the whole run
  maxErrorRate: 1                      # % of requests failing transport OR criteria
  thresholds:                          # metric <= max (ms for time metrics)
    - http.response_time.p95: 250
    - scenario.checkout.step.place-order.response_time.p99: 1200
  conditions:
    - expression: http.request_rate > 90 and vusers.failed == 0
      strict: true                     # false = warn only

report:
  interval: 10s
  json: ./load-report.json
  junit: ./load-junit.xml
seed: 7                                # reproducible picks/jitter (default: time-based)
grace: 30s                             # wait for in-flight users after the last phase
```

Durations accept `250ms`, `30s`, `2m`, `1.5h`, or a bare number of
seconds.

### Workflow prerequisites

`before:` and every `dependsOn` of a scenario workflow run **once**, in
dependency order, before arrivals begin; their outputs are readable by
every user as `$workflows.<id>.outputs.<name>` (or
`$workflows.<id>.steps.<step>.outputs.<name>`). A failing prerequisite
aborts the run with exit 2 — nothing is load-tested against a broken
setup.

### Per-user inputs

Precedence (low → high): workflow input schema `default` → Arazzo
`parameters` → profile `inputs` literal/pool → `data` row → `--input`
flags. Reserved inputs: `$inputs.__vu` (user number), `__run`, `__env`,
`__scenario`, `__iteration`.

Runtime expressions inside `requestBody.payload` are resolved per
request: a whole-leaf expression (`qty: $inputs.qty`) keeps its JSON
type; embedded `{$inputs.x}` / `${$inputs.x}` render as text.

### Think time

`x-suspect-think: 500ms..2s` on a step pauses after that step (uniform
jitter, seeded). The scenario `think` is the default for steps without
the extension. `suspect test` ignores think time.

### Credentials and contract checks

`suspect load` injects credentials exactly like `suspect test`: for every
security scheme an operation requires, the configured credential from
`--credentials FILE`, else `SUSPECT_CREDENTIALS`, else
`.suspect/credentials.json` found by walking up from the Arazzo
document. Hooks and every virtual user share one token cache, so an
OAuth client-credentials token is acquired once and refreshed before it
expires rather than per user. A failed token acquisition counts as an
`errors.other` transport failure for that step.

`successCriteria` always decide a step's verdict. `--validate-responses`
additionally checks each response body against the schema the operation
declares for its status (as `suspect test` always does); a violation
counts as a contract failure. It is off by default because it compiles a
schema per response on the generator.

## Metrics

| Name | Meaning |
|---|---|
| `http.requests`, `http.responses`, `http.request_rate` | sent, received, per second |
| `http.codes.<code>`, `http.codes.2xx|4xx|5xx` | by status |
| `http.response_time.{min,max,mean,median,p75,p90,p95,p99,p999}` | ms, request → last body byte |
| `http.ttfb.*` | ms, request → headers |
| `http.error_rate` | % of requests failing transport **or** criteria |
| `http.downloaded_bytes`, `http.uploaded_bytes` | body bytes |
| `errors.{timeout,connect,tls,reset,other,total}` | transport failures |
| `contract.failures`, `contract.failure_rate` | responses failing `successCriteria` (or, with `--validate-responses`, their declared schema) |
| `vusers.{created,completed,failed,skipped,aborted}` | lifecycle; `skipped` = `maxVusers` hit, `aborted` = still running at the grace deadline |
| `vusers.session_length.*` | ms per user |
| `scenario.<name>.vusers.{created,completed,failed}` | per scenario |
| `scenario.<name>.step.<id>.{requests,contract_failures,codes.<c>,errors.<class>,response_time.*}` | per step |
| `metrics.dropped` | samples lost to a full recorder queue — the generator, not the server, is overloaded |

Arrivals are **open-loop**: a user is created on schedule whether or
not earlier users finished. Slow responses therefore show up as more
concurrent users (and `vusers.skipped` once a cap is hit), never as a
quietly reduced load. `--jitter poisson` replaces even spacing with
exponential inter-arrival times.

## `ensure` expressions

```text
expr    = or ;  or = and ("or" and)* ;  and = unary ("and" unary)*
unary   = "not" unary | cmp
cmp     = operand (("<"|"<="|">"|">="|"=="|"!=") operand)?
operand = number | metric | "(" expr ")"
```

Counters with no entries (`http.codes.503` when none occurred) read as
`0`; unknown metric names fail the check with an explanation.

## Output

- **Text**: an Artillery-style block every `report.interval`, then a
  final report with scenario shares, phases, hooks, cumulative metrics,
  a per-step table and `ensure` verdicts.
- **`--report ndjson`**: journal records — `meta` (phase starts),
  `load_snapshot` (interval + cumulative metrics), `log` notes, the full
  JSON report, and a `run_summary` whose `passed`/`failed` count ensure
  checks.
- **`--json FILE`**: versioned report (`version: 1`) with `run`,
  `scenarios`, `phases`, `hooks`, `intervals[]`, `aggregate`, `ensure`.
- **`--junit FILE`**: one `<testcase>` per ensure check; strict failures
  are `<failure>`, non-strict ones `<skipped>`.

Exit codes: `0` every strict check passed · `1` a strict check failed ·
`2` configuration/compile error or failed prerequisite · `130`
interrupted before any traffic. Ctrl-C stops arrivals and drains;
a second Ctrl-C aborts.

## Calibrating the generator

Run the profile against `suspect gateway --mode mock` first. The mock
answers in microseconds, so anything the report attributes to latency
there is generator or host overhead. Watch `metrics.dropped` and
`errors.connect`: both mean the load generator or the host's socket
limits, not the service, are the bottleneck. Measured on an 18-core
Apple laptop with the gateway co-located: ~4 µs CPU per request
in-process and 43k req/s sustained over sockets with zero drops.

## Editor support

`*.load.yaml` files are a recognized document family (`Load10`):
`suspect check` labels them and the LSP publishes profile diagnostics
(bad durations, phase shapes, `ensure` syntax, missing data files, and
unknown workflow ids when the Arazzo document is in the workspace) at
the offending line.
