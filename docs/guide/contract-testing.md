# Contract testing and Arazzo workflows

`suspect test run` runs Arazzo workflows against a server and holds every
reply to the OpenAPI description the workflow names. A step passes when
its reply is one the description declares *and* the step's own success
criteria hold. The same description drives a mock for consumers, generated
stateful sequences, a fuzzer and offline replay, so each of them judges a
reply the same way. Every flag is in the
[command reference](../reference/cli.md#suspect-test-run).

## Quick start

The bundled example ([`examples/contract-testing/`](../../examples/contract-testing/))
has a description and a suite. From the repository root:

```console
$ suspect traffic gateway examples/contract-testing/petstore.openapi.yaml -p 8080 --mode mock --enforce &
$ suspect test run examples/contract-testing/flows.arazzo.yaml --base-url http://127.0.0.1:8080
...
suspect test report
===================
lifecycle ... PASS (1 ms)
  POST http://127.0.0.1:8080/pets -> 201
  GET http://127.0.0.1:8080/pets/1 -> 200
  PUT http://127.0.0.1:8080/pets/1 -> 200
  DELETE http://127.0.0.1:8080/pets/1 -> 204
browse ...... PASS (1 ms)
  GET http://127.0.0.1:8080/pets?limit=20 -> 200
  GET http://127.0.0.1:8080/pets/pet_42 -> 200
6 passed, 0 failed, 0 skipped in 3 ms
```

Against your own service, point `--base-url` at it. The exit code is `0`
when every step passed, `1` when any failed and `2` when nothing could run.

A failure names the check, what was wanted and what arrived:

```text
list ... FAIL (1 ms)
  GET http://127.0.0.1:8080/pets -> 200
  FAIL response-schema: expected a body matching the declared schema, got /items/0/id: value has type integer, expected `string`; /items/0: required property `name` is missing
```

## What a run checks

For every reply, in this order:

| Check | Fails when |
|---|---|
| `response-status` | the status is not one the operation declares: no exact code, no range (`4XX`) and no `default` |
| `response-header` | a response header declared `required` is missing, or a declared header does not satisfy its schema |
| `response-content-type` | the `Content-Type` is not one the response declares (`application/*` and `*/*` cover what they say; parameters such as `charset` are ignored) |
| `response-schema` | a JSON body (`application/json` or any `+json` type) does not satisfy the schema of its media type |
| the step's `successCriteria` | any criterion does not hold; a step without criteria requires a `2xx` |

The contract checks run whether or not the step has criteria, and a
criterion that expects an error does not excuse the error's body: a `404`
the suite asks for still has to be the `404` the description declares.

`✓ response 200 matches contract` is printed only when a body was compared
with a schema. A response with no content, no schema or a non-JSON media
type is declared but not schema-checked, and nothing is claimed for it.

### Conformance levels

`--conformance` (or `test.conformance` in `.suspect.yaml`) selects how much
of that applies:

| Level | Applies |
|---|---|
| `strict` (default) | everything above |
| `schema` | the body schema of declared responses only; a status or content type the description omits is not a finding. For descriptions that document the success path only |
| `off` | success criteria only |

```yaml
# .suspect.yaml
test:
  base_url: http://localhost:8080
  conformance: schema
```

### What the description may contain

- **OpenAPI 3.0, 3.1 and 3.2.** A 3.0 document is read in its own dialect:
  `nullable: true` admits `null` (also in the `nullable` + `allOf: [$ref]`
  idiom), and a boolean `exclusiveMinimum`/`exclusiveMaximum` beside
  `minimum`/`maximum` is an exclusive bound.
- **`readOnly` and `writeOnly`.** A response may omit a required
  `writeOnly` property and a request may omit a required `readOnly` one.
  Nothing else about the property changes.
- **Inline and shared declarations.** A schema written inline is checked
  like a component. A response, parameter, request body, header or path
  item behind a `$ref` is followed.
- **Several files.** A reference into another file (`$ref:
  './schemas/pet.yaml'`) is loaded and checked like a local one, recursion
  included. A reference that cannot be loaded fails the step that needs it;
  it is never skipped.

## Writing a suite

A suite is an [Arazzo](https://spec.openapis.org/arazzo/latest.html)
document. `sourceDescriptions` name the OpenAPI descriptions; each step
calls one operation of one of them.

```yaml
arazzo: 1.0.1
info: { title: Pet journeys, version: "1.0" }
sourceDescriptions:
  - { name: petstore, url: petstore.openapi.yaml, type: openapi }
workflows:
  - workflowId: lifecycle
    inputs:
      type: object
      properties:
        petName: { type: string, default: Rex }
    steps:
      - stepId: create
        operationId: createPet
        requestBody:
          contentType: application/json
          payload: { name: $inputs.petName }
        successCriteria:
          - condition: $statusCode == 201
        outputs:
          petId: $response.body#/id
      - stepId: read-back
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: $steps.create.outputs.petId }
```

- **The operation** is `operationId` (qualify it as
  `$sourceDescriptions.<name>.<operationId>` when two descriptions share an
  id) or `operationPath` (`$sourceDescriptions.<name>#/paths/~1pets/get`, or
  the literal `GET /pets`). Under `strict`, an `operationPath` no
  description declares fails with the check `operation`.
- **Inputs** come from `--input name=value` (repeatable; the value is JSON
  when it parses), `--inputs-file`, then the workflow's `parameters`, then
  the `default` of its `inputs` schema.
- **Outputs** are runtime expressions captured after a passing step and
  read by later steps (`$steps.<id>.outputs.<name>`) and by other workflows
  (`$workflows.<id>.outputs.<name>`, with `dependsOn`).
- **Order.** Workflows run concurrently; `dependsOn` makes one wait for
  another. Steps of a workflow run one at a time, in document order, after
  the steps they depend on or read outputs from.
- **Requests** ask for the media types the operation declares (`Accept`),
  JSON first, and send `application/json` bodies unless `contentType` says
  otherwise. Credentials for the operation's security schemes are injected:
  see [test credentials](test-credentials.md).

### Success criteria

A criterion is a `simple` condition unless it declares a `type`.

| Form | Example |
|---|---|
| comparison | `$statusCode == 201`, `$response.body#/total >= 1`, `$response.header.X-Total > 5` |
| logic and grouping | `$statusCode == 200 \|\| $statusCode == 201`, `!($response.body#/state == 'failed')`, `a && (b \|\| c)` |
| against run state | `$response.body#/id == $steps.create.outputs.petId`, `$response.body#/name == $inputs.petName` |
| existence | `$response.body#/id != null`, or the bare expression `$response.body#/id` |
| regex on a context | `context: $response.body#/id`, `condition: '^pet_[0-9]+$'`, `type: regex` |
| JSONPath on a context | `context: $response.body`, `condition: $.items[?@.status == 'available']`, `type: jsonpath` |

The operators are `==`, `!=`, `<`, `<=`, `>`, `>=`, `!`, `&&`, `||` and
parentheses. Literals are `true`, `false`, `null`, numbers and
single-quoted strings (`''` is a quote). String comparison is
case-insensitive, as Arazzo requires. A header value that spells a number
compares as that number. An expression may be written `{$statusCode}`.

A JSONPath criterion (RFC 9535) holds when the query selects at least one
node. A query that is one filter on the root, such as Arazzo's own example
`$[?count(@.pets) > 0]`, is also tried with the context itself as the
filter's current node, so it works when the context is an object.

When a condition does not hold, the failure shows what its expressions
were:

```text
FAIL $statusCode == 200 && $response.header.X-State == 'done': expected $statusCode == 200 && $response.header.X-State == 'done', got $statusCode = 503, $response.header.X-State = <missing>
```

### Actions

`onFailure` and `onSuccess` list actions. The first action whose `criteria`
hold applies (an action without criteria always does).

```yaml
- stepId: wait-for-job
  operationId: showJob
  successCriteria:
    - condition: $response.body#/state == 'done'
  onFailure:
    - name: still-running
      type: retry
      retryAfter: 0.5      # seconds
      retryLimit: 20
      criteria:
        - condition: $statusCode == 200
    - { name: give-up, type: end }
```

| Action | Effect |
|---|---|
| `retry` (`onFailure`) | the step runs again after `retryAfter` seconds, up to `retryLimit` times (once when omitted). A `Retry-After` response header in seconds overrides `retryAfter`. The step is judged by its last attempt. An exhausted retry is passed over for the next action |
| `goto` with `stepId` | the workflow continues at that step; steps the jump passes over are not run. A workflow follows at most 16 gotos |
| `end` | the workflow ends: failed after `onFailure`, passed after `onSuccess` |

A workflow's `failureActions` and `successActions` apply to every step,
after the step's own; a step's action of the same name replaces the
workflow's. Reusable actions (`reference:
$components.failureActions.<name>`) are resolved.

A request body may carry `replacements`: each sets the value at a JSON
Pointer `target` of a JSON `payload`, and its `value` may be a runtime
expression.

## Running suites

```console
$ suspect test run flows.arazzo.yaml                      # test.base_url, else http://localhost:8080
$ suspect test run tests/ other.arazzo.yaml               # every *.arazzo.{yaml,yml,json} under tests/, as one gate
$ suspect test run flows.arazzo.yaml --filter checkout    # workflows whose id contains "checkout"
$ suspect test run flows.arazzo.yaml --input petName=Tom --inputs-file inputs.yaml
$ suspect test run flows.arazzo.yaml --conformance schema
```

Settings come from `.suspect.yaml` next to the suite (`suspect config
--explain` shows each value and where it came from); flags win.

### Reports

| Output | How |
|---|---|
| text (default) | progress lines, then the report above |
| NDJSON | `--format ndjson`: one event per line, ending with `run_done` and the run summary |
| JUnit XML | `--junit out/junit.xml`: one `<testsuite>` per workflow, one `<testcase>` per step, failures with every failed check and the requests sent; steps not reached are `<skipped>` |
| JSON | `--report-file out/report.json`: counts and one entry per step (`suite`, `name`, `status`, `failures`, `requests`) |

`suspect test stateful`, `fuzz` and `replay` take `--junit` and
`--report-file` too, with one case per sequence, operation and exchange.

## Recording and offline runs

Record a run through the gateway, then repeat it without the service:

```console
$ suspect traffic gateway api.yaml -p 9000 --mode record --upstream http://staging:80 --cassette rec.scj &
$ suspect test run flows.arazzo.yaml --base-url http://127.0.0.1:9000
$ suspect test run flows.arazzo.yaml --cassette rec.scj          # offline
```

Offline, each request is answered by the recording of that request: the
method, path and query are matched, the scheme and host are not, so a
cassette recorded against one host replays against any base URL. A request
sent twice walks its recordings in order (read, delete, read again), and a
recording with the same request body is preferred. A request the cassette
does not hold fails its step with a `transport` failure that names it.

Credentials are redacted from cassettes when they are recorded, so a
cassette can be committed. The replies in it are still held to the
contract: after the description changes, an offline run shows which
recorded replies no longer conform, and
`suspect diff impact old.yaml new.yaml rec.scj` says the same without
running the suite.

More on recording, redaction and the cassette format is in the
[gateway guide](gateway.md).

## Tests generated from the description

Neither needs a suite; both hold replies to the contract at the same
`--conformance` level.

```console
$ suspect test stateful api.yaml --base-url http://localhost:8080
$ suspect test fuzz api.yaml --base-url http://localhost:8080 --runs 50
```

- `test stateful` derives create → use → delete sequences from the paths
  (`POST /pets` creates what `GET /pets/{petId}` reads) and fails a
  sequence on a non-`2xx` status or a reply that breaks its contract.
- `test fuzz` sends each operation requests with one field replaced by a
  pathological value. A `5xx` or a transport failure is a **crash**; any
  other reply the operation does not declare is a contract **violation**.
  An API that rejects bad input with a `400` it never documents shows up
  here; `--conformance schema` leaves undeclared statuses out.

## The consumer's side

The provider proves it answers as described; a consumer has to send what
the description allows.

```console
$ suspect traffic gateway api.yaml -p 8080 --mode mock --enforce --journal traffic.jsonl
# run the consumer's own tests against http://127.0.0.1:8080
$ suspect traffic drift api.yaml traffic.jsonl --check
```

- The mock answers from the description's examples and schemas, in the
  media type and status the description declares. With `--enforce` it
  rejects a request that breaks the contract (a missing or ill-typed
  parameter, a body that fails its schema) the way the operation declares
  its rejections: its `400`, `422`, another `4xx` or `default` response, or
  a `400` problem document when it declares none.
- `traffic drift --check` exits `1` when the consumer called endpoints or
  sent query parameters the description does not declare.
- `suspect diff breaking old.yaml new.yaml` and `suspect diff impact`
  tell a provider what a change would break before it ships.

## In CI

```yaml
# .github/workflows/contract.yml
jobs:
  contract:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: docker compose up -d --wait api        # the service under test
      - uses: LukasParke/suspect/.github/actions/suspect@v0.1.0
        with:
          command: test
          arazzo: tests/contract
          base-url: http://127.0.0.1:8080
          junit: out/contract.xml
      - uses: actions/upload-artifact@v4
        if: always()
        with: { name: contract-report, path: out/contract.xml }
```

The action downloads the release binary. Before a release exists, or to
test an unreleased revision, reference the action by branch or commit and
set `version: source`: it builds the CLI with cargo from that revision
(a few minutes; the runner needs a Rust toolchain).

A repository with several projects declares its suites in
`suspect.project.json` and runs them with the rest of its checks:

```json
{ "tests": { "arazzo": ["tests/flows.arazzo.yaml"], "base_url": "http://127.0.0.1:8080" } }
```

```console
$ suspect ci --stage test
```

## In the editor

The VS Code Testing view lists the workflows and steps of every Arazzo
document and runs them with the same binary and settings; a failed check
is shown on the criterion or step that failed. See the
[editors guide](editors.md).

## Limits

- Bodies are schema-checked when they are JSON. XML, form and binary
  bodies are checked for status and content type only.
- A `goto` to a `workflowId`, a step that runs a workflow (`workflowId`
  on a step), `type: xpath` criteria, `$url` and `replacements` on a
  payload that is not JSON are not run: the suite fails to compile and
  says which. Order workflows with `dependsOn` and read their outputs
  through `$workflows.<id>.outputs` instead.
- Array- and object-typed parameters and headers are not decoded, so they
  are not schema-checked.
