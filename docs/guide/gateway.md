# Gateway, recording and replay

`suspect traffic gateway` serves an OpenAPI document as a mock, or sits in
front of a real server to validate, record or replay its traffic. Recorded
traffic feeds offline contract tests, drift reports and HAR export. Every
flag is in the [command reference](../reference/cli.md#suspect-traffic-gateway).

## Modes

```console
$ suspect traffic gateway api.yaml -p 8080 --mode mock
$ suspect traffic gateway api.yaml -p 9000 --mode proxy    --upstream http://staging:80
$ suspect traffic gateway api.yaml -p 9000 --mode validate --upstream http://staging:80 --enforce
$ suspect traffic gateway api.yaml -p 9000 --mode record   --upstream http://staging:80 --cassette rec.scj
$ suspect traffic gateway api.yaml -p 9001 --mode replay   --cassette rec.scj
$ suspect traffic gateway api.yaml -p 9002 --mode scenario --scenario steps.json
```

| Mode | What it does |
|---|---|
| `mock` | answers from the spec's examples and schemas, with the status and media type each operation declares; `--enforce` rejects a request that breaks the contract the way the operation declares its rejections (`400` when it declares none) |
| `proxy` | forwards to `--upstream` and journals each exchange |
| `validate` | forwards and checks requests and responses against the spec, with the checks `suspect test run` applies; `--enforce` rejects an invalid request with 400 instead of forwarding it |
| `record` | forwards and writes each exchange to `--cassette` |
| `replay` | answers from a recorded cassette; the spec argument is ignored |
| `scenario` | answers a scripted sequence of steps (`{"steps": [{"method", "path_suffix", "status", "body"}]}`) |

A mock reply satisfies the schema it is built from: an `example`,
`default` or `const` is used as written, and otherwise numbers respect
their bounds, strings their `format`, length and (for simple patterns)
`pattern`, arrays their `minItems`, and a recursive schema ends in an empty
list rather than a `null`. Declared response headers are sent with example
values. So a suite run against the mock under `strict` conformance fails
only where the description contradicts itself, such as an `example` that
does not match its schema.

The gateway binds `127.0.0.1`. `-p 0` picks a free port, and the bound
address is printed on stderr once the gateway can serve. Ctrl-C stops it
with exit status `0`.

Defaults come from `.suspect.yaml`, and the flags win over them:

```yaml
gateway:
  port: 8080        # editor setting: suspect.gateway.port
  mode: mock
  upstream: http://staging:80
```

### Fault injection

`--delay-ms` with `--delay-pct` delays a share of requests, and
`--error-status` with `--error-pct` fails a share of them, in every mode but
`replay`. Which requests are hit depends only on the request, so the pattern
is the same on every run. Use it to test client retries and timeouts.

### Journals and redaction

Each exchange is journalled as one JSON line on stdout, or appended to
`--journal FILE`. Credentials are redacted from journals and cassettes by a
default denylist of headers and JSON body keys. `--redact-header NAME` and
`--redact-json-key KEY` (both repeatable) add to it.

## Using recorded traffic

```console
$ suspect test run flows.arazzo.yaml --cassette rec.scj    # contract tests, offline
$ suspect traffic drift api.yaml rec.scj --check            # traffic the spec does not describe
$ suspect traffic har rec.scj -o rec.har                    # HTTP Archive 1.2
$ suspect diff impact old.yaml new.yaml rec.scj             # recorded traffic a revision would break
$ suspect test replay rec.scj --upstream http://staging:80  # re-issue traffic and report drift
```

Offline, `test run --cassette` answers each request with the recording of
that request (method, path and query; not scheme or host), so the order
workflows happen to run in does not matter; see
[contract testing](contract-testing.md#recording-and-offline-runs). The
gateway's own `replay` mode is stateless: the latest recording of a request
always answers it.

`traffic drift` reads a cassette or a journal and reports endpoints and query
parameters the traffic used that the spec does not declare, and declared
operations the traffic never touched. `--check` exits `1` on undeclared
endpoints or parameters. `--ignore-param` skips a parameter, such as the
auth token CI passes. `--format` takes `text`, `json`, `sarif` or `junit`.

## Cassette format

A Suspect Cassette (`.scj`) is JSON Lines. The first line is a header
(`format`, `version`, `recorded_at_ms`, `source`). Each later line is one
exchange, with sha256-hashed bodies stored as UTF-8 or base64. Cassettes are
append-only and diff well in git. Replay, offline tests and drift checks
read them.

## From the editor

In VS Code, `Suspect: Start Mock Gateway` and `Suspect: Stop Mock Gateway`
run the gateway on the open spec at `suspect.gateway.port` (see the
[editors guide](editors.md)).
