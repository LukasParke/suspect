# `suspect` command reference

<!-- Generated from the clap command tree by crates/suspect-cli/tests/cli_reference.rs; do not edit by hand. Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test cli_reference`. -->

Exit codes: `0` clean, `1` findings or failed checks, `2` usage or environment error, `130` interrupted (long-running servers exit `0` on Ctrl-C).

## `suspect`

```text
OpenAPI/Arazzo/Overlay toolkit

Usage: suspect <COMMAND>

Commands:
  acquire            Acquire a hash-pinned document closure or verify its offline cache
  auth               Manage and verify credentials for the testing flows
  check              Parse documents and report family, syntax errors, `$ref` edges, cycles, and workspace stats
  validate           Validate OpenAPI syntax and semantic requirements, reporting located findings
  admission          Run the generation admission layer over documents without generating
  breaking           Detect consumer-breaking changes between two spec revisions
  docs               Generate API reference docs (HTML, Markdown, or a SvelteKit site)
  stubs              Generate typed server stubs from an OpenAPI document
  arazzo-diff        Detect breaking changes between two Arazzo documents
  overlay-dry-run    Report overlay action target matches without applying
  upgrade            Upgrade Swagger 2.0 documents to OpenAPI 3.1
  rules              Run TS/JS custom rules (Bun sidecar) over documents
  lint               Run spectral-style lint rules over documents
  overlay            Apply, explain, or synthesize (`diff`) Overlay 1.0/1.1 documents
  fmt                Re-emit a document in canonical JSON/YAML form
  stats              Structural counts for a document
  bundle             Bundle a document and its `$ref` closure into one file
  diff               Semantic structural diff between two documents
  drift              Cross-reference recorded traffic against the spec: endpoints the traffic exercised that the spec does not declare, undeclared query parameters, and declared operations the traffic never touched
  har                Export recorded traffic as an HTTP Archive (HAR 1.2) — the interchange format profiling and debugging tools read. Accepts a Suspect Cassette or a Suspect Journal
  bench              Wall-clock micro-benchmark of the pipeline stages on one fixture
  bridge             Run the live contract bridge: watch the spec and emit regeneration plans, evolution proposals, and reconciliation conflicts
  reverse            Extract server framework routes and cross-reference them with the spec: undocumented endpoints, spec-only endpoints, method drift
  stateful           Generate and run stateful dependency-graph test sequences against a live server
  test               Compile an Arazzo document into an executable suite and run it
  load               Run Arazzo workflows as weighted virtual-user scenarios under a phased load profile, with latency/throughput metrics and SLO gates
  fuzz               Fuzz operations with schema-mutating requests against a live server
  why                Trace a validation failure to its origin: source location, git blame, and whether recorded traffic ever passed it
  replay             Re-issue a recorded cassette against an upstream and report drift
  gen                Render documentation or custom template manifests from an OpenAPI document
  watch              Re-run a command whenever documents change under the given roots
  gateway            Serve a spec as a mock, or proxy/validate/record against an upstream, or replay a recorded cassette
  codegen            Generate a source-selected native SDK package
  codegen-terraform  Generate a Terraform provider through explicit lifecycle mappings and the generated Go SDK
  codegen-cli        Generate a Go/Cobra API CLI application over its own embedded generated Go SDK
  codegen-mcp        Generate a TypeScript stdio MCP server over its own embedded generated TypeScript SDK
  codegen-profiles   List the native SDK or application profiles available in this CLI build
  codegen-session    Persistent canonical SDK generation with readonly preview and watch mode
  codegen-compare    Compare wire contracts and native SDK interfaces with migration notes
  release-plan       Recommend a release version and changelog from two spec revisions
  impact             Evaluate recorded traffic against a candidate contract revision
  release-publish    Plan and execute SDK publishing across registries
  release-workflow   Render the release manifest as a tag-triggered CI workflow
  evidence           Print the SDK verification matrix: backend x feature -> evidence
  ci                 Run validate, contract, breaking, codegen, and tests across every project in a workspace as one gate
  contract           Emit or verify a machine-readable contract package
  config             Show the configuration in effect, or with `--explain` every setting and where its value came from
  project            Build and check a suspect project from one manifest
  lsp                Run the language server over stdio
  help               Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## `suspect acquire`

```text
Acquire a hash-pinned document closure or verify its offline cache

Usage: suspect acquire [OPTIONS] <MANIFEST>

Arguments:
  <MANIFEST>  Versioned manifest containing exact retrieval URIs and SHA-256 pins

Options:
      --cache-dir <CACHE_DIR>
          Content-addressed document cache [default: .suspect-cache]
      --offline
          Verify cache bytes without source-file reads or network requests
      --refresh
          Refetch every declared pin and reject byte or redirect drift
      --stale-before <STALE_BEFORE>
          Refetch pins older than this UTC YYYY-MM-DDTHH:MM:SSZ timestamp
      --max-bytes <MAX_BYTES>
          Per-document byte ceiling [default: 67108864]
      --max-total-bytes <MAX_TOTAL_BYTES>
          Total byte ceiling across the declared closure [default: 268435456]
      --timeout-seconds <TIMEOUT_SECONDS>
          End-to-end acquisition deadline in seconds [default: 30]
      --allow-redirect-origin <ALLOW_REDIRECT_ORIGIN>
          Exact destination origin allowed for a manifest-declared cross-origin redirect
      --insecure-test-origin <INSECURE_TEST_ORIGIN>
          Explicit HTTP origin for a numeric-loopback test fixture
      --format <FORMAT>
          Human-readable status or a structured provenance/diagnostic report [default: text] [possible values: text, json]
  -h, --help
          Print help (see more with '--help')
```

## `suspect auth`

```text
Manage and verify credentials for the testing flows

Usage: suspect auth <COMMAND>

Commands:
  check  Verify every configured credential without running a workflow
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

## `suspect auth check`

```text
Verify every configured credential without running a workflow

Usage: suspect auth check [OPTIONS]

Options:
      --credentials <FILE>  Credentials file (default: `test.credentials`, `SUSPECT_CREDENTIALS`, else a discovered `.suspect/credentials.json`)
  -h, --help                Print help
```

## `suspect check`

```text
Parse documents and report family, syntax errors, `$ref` edges, cycles, and workspace stats

Usage: suspect check [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Documents to check

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect validate`

```text
Validate OpenAPI syntax and semantic requirements, reporting located findings

Usage: suspect validate [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  OpenAPI documents to validate

Options:
      --reference-allowlist <REFERENCE_ALLOWLIST>
          JSON array of absolute document paths; deny every unlisted reference load
      --strict-format
          Assert JSON-Schema `format` keywords (RFC 2020-12 makes them annotations by default; this validates declared formats)
      --format <FORMAT>
          Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help
          Print help (see more with '--help')
```

## `suspect admission`

```text
Run the generation admission layer over documents without generating

Usage: suspect admission [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  OpenAPI documents to review

Options:
      --format <FORMAT>  Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help             Print help (see more with '--help')
```

## `suspect breaking`

```text
Detect consumer-breaking changes between two spec revisions

Usage: suspect breaking [OPTIONS] [OLD] [NEW]

Arguments:
  [OLD]  The previous spec revision (file mode only)
  [NEW]  The current spec revision (file mode only)

Options:
      --current <NEW>    The current spec revision in baseline mode; the previous revision comes from git, so only one file is named
      --baseline <REF>   Compare against this git ref (tag, branch, or commit) instead of the `old` file: the "what did we last publish" baseline
      --format <FORMAT>  Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help             Print help (see more with '--help')
```

## `suspect docs`

```text
Generate API reference docs (HTML, Markdown, or a SvelteKit site)

Usage: suspect docs [OPTIONS] <INPUT>

Arguments:
  <INPUT>  The OpenAPI document to document

Options:
      --style <STYLE>  Output style (default: a single HTML file, or `docs.style` from `.suspect.yaml`) [possible values: html, markdown, sveltekit]
  -o, --output <PATH>  Output path: a file for `html`, a directory for `markdown` and `sveltekit`
      --title <TITLE>  Override the document title (defaults to `info.title`)
  -h, --help           Print help (see more with '--help')
```

## `suspect stubs`

```text
Generate typed server stubs from an OpenAPI document

Usage: suspect stubs --backend <BACKEND> --output <DIR> <INPUT>

Arguments:
  <INPUT>  The OpenAPI document

Options:
      --backend <BACKEND>  Target framework: `rust-axum` or `ts-express` [possible values: rust-axum, ts-express]
  -o, --output <DIR>       Output directory [alias: --out]
  -h, --help               Print help (see more with '--help')
```

## `suspect arazzo-diff`

```text
Detect breaking changes between two Arazzo documents

Usage: suspect arazzo-diff [OPTIONS] <OLD> <NEW>

Arguments:
  <OLD>  The previous Arazzo revision
  <NEW>  The current Arazzo revision

Options:
      --format <FORMAT>  Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help             Print help (see more with '--help')
```

## `suspect overlay-dry-run`

```text
Report overlay action target matches without applying

Usage: suspect overlay-dry-run <OVERLAY> <TARGET>

Arguments:
  <OVERLAY>  The Overlay document
  <TARGET>   The target document the overlay applies to

Options:
  -h, --help  Print help
```

## `suspect upgrade`

```text
Upgrade Swagger 2.0 documents to OpenAPI 3.1

Usage: suspect upgrade [OPTIONS] <INPUT>

Arguments:
  <INPUT>  The Swagger 2.0 document to upgrade

Options:
  -o, --output <FILE>  Write the document here instead of stdout
      --to <TO>        Serialization (default: the `--output` extension, else the input's). `--format` is the old `bundle`/`upgrade` spelling [possible values: json, yaml]
  -h, --help           Print help (see more with '--help')
```

## `suspect rules`

```text
Run TS/JS custom rules (Bun sidecar) over documents

Usage: suspect rules <COMMAND>

Commands:
  check  Evaluate TS rules from `.suspect/rules` against documents
  new    Scaffold a new TS rule file
  bench  Measure worker cold start and evaluate latency
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

## `suspect rules check`

```text
Evaluate TS rules from `.suspect/rules` against documents

Usage: suspect rules check [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Documents to evaluate

Options:
      --timeout-ms <TIMEOUT_MS>      Per-evaluate deadline in milliseconds [default: 250]
      --min-severity <MIN_SEVERITY>  Report only findings at or above this severity [default: hint] [possible values: hint, info, warning, error]
      --format <FORMAT>              Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help                         Print help (see more with '--help')
```

## `suspect rules new`

```text
Scaffold a new TS rule file

Usage: suspect rules new [OPTIONS] <NAME>

Arguments:
  <NAME>  Rule name (becomes the file name and rule id)

Options:
      --dir <DIR>  Target directory (default `.suspect/rules`)
  -h, --help       Print help
```

## `suspect rules bench`

```text
Measure worker cold start and evaluate latency

Usage: suspect rules bench [OPTIONS] <PATH>

Arguments:
  <PATH>  Document to evaluate against

Options:
      --iterations <ITERATIONS>  Warm evaluate iterations [default: 5]
      --timeout-ms <TIMEOUT_MS>  Per-evaluate deadline (large specs need more than the 250ms interactive default) [default: 30000]
  -h, --help                     Print help
```

## `suspect lint`

```text
Run spectral-style lint rules over documents

Usage: suspect lint [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Documents to lint

Options:
      --ruleset <RULESET>            Ruleset document (default: built-in spectral ruleset)
      --min-severity <MIN_SEVERITY>  Report only findings at or above this severity (default: hint, or `lint.min_severity` from `.suspect.yaml`) [possible values: hint, info, warning, error]
      --format <FORMAT>              Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help                         Print help (see more with '--help')
```

## `suspect overlay`

```text
Apply, explain, or synthesize (`diff`) Overlay 1.0/1.1 documents

Usage: suspect overlay <COMMAND>

Commands:
  apply    Apply an overlay document to a target document
  explain  Explain what each action selects and changes, in sequence
  diff     Synthesize an Overlay 1.1 document from two document revisions
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

## `suspect overlay apply`

```text
Apply an overlay document to a target document

Usage: suspect overlay apply [OPTIONS] <OVERLAY> <TARGET>

Arguments:
  <OVERLAY>  Overlay 1.0/1.1 document
  <TARGET>   Document the overlay acts on

Options:
  -o, --output <FILE>  Write the document here instead of stdout
      --to <TO>        Serialization (default: the `--output` extension, else the input's). `--format` is the old `bundle`/`upgrade` spelling [possible values: json, yaml]
  -h, --help           Print help (see more with '--help')
```

## `suspect overlay explain`

```text
Explain what each action selects and changes, in sequence

Usage: suspect overlay explain [OPTIONS] <OVERLAY> <TARGET>

Arguments:
  <OVERLAY>  Overlay 1.0/1.1 document
  <TARGET>   Document the overlay acts on

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
      --check            Exit 1 when the result is not clean
  -h, --help             Print help (see more with '--help')
```

## `suspect overlay diff`

```text
Synthesize an Overlay 1.1 document from two document revisions

Usage: suspect overlay diff [OPTIONS] <OLD> <NEW>

Arguments:
  <OLD>  The current document
  <NEW>  The revised document

Options:
  -o, --output <FILE>  Write the document here instead of stdout
      --to <TO>        Serialization (default: the `--output` extension, else the input's). `--format` is the old `bundle`/`upgrade` spelling [possible values: json, yaml]
  -h, --help           Print help (see more with '--help')
```

## `suspect fmt`

```text
Re-emit a document in canonical JSON/YAML form

Usage: suspect fmt [OPTIONS] <INPUT>

Arguments:
  <INPUT>  Input document

Options:
  -o, --output <FILE>  Write the document here instead of stdout
      --to <TO>        Serialization (default: the `--output` extension, else the input's). `--format` is the old `bundle`/`upgrade` spelling [possible values: json, yaml]
  -h, --help           Print help (see more with '--help')
```

## `suspect stats`

```text
Structural counts for a document

Usage: suspect stats [OPTIONS] <PATH>

Arguments:
  <PATH>  Input document

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect bundle`

```text
Bundle a document and its `$ref` closure into one file

Usage: suspect bundle [OPTIONS] <INPUT>

Arguments:
  <INPUT>  Entry document

Options:
      --strategy <STRATEGY>  Bundling strategy [default: keep] [possible values: keep, inline]
  -o, --output <FILE>        Write the document here instead of stdout
      --to <TO>              Serialization (default: the `--output` extension, else the input's). `--format` is the old `bundle`/`upgrade` spelling [possible values: json, yaml]
  -h, --help                 Print help (see more with '--help')
```

## `suspect diff`

```text
Semantic structural diff between two documents

Usage: suspect diff [OPTIONS] <A> <B>

Arguments:
  <A>  Left-hand document
  <B>  Right-hand document

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
      --check            Exit 1 when the result is not clean
  -h, --help             Print help (see more with '--help')
```

## `suspect drift`

```text
Cross-reference recorded traffic against the spec: endpoints the traffic exercised that the spec does not declare, undeclared query parameters, and declared operations the traffic never touched

Usage: suspect drift [OPTIONS] <SPEC> <TRAFFIC>

Arguments:
  <SPEC>     Entry OpenAPI document
  <TRAFFIC>  Traffic file: a Suspect Cassette (gateway `--mode record`) or a Suspect Journal (gateway `--journal`); records are recognized line by line

Options:
      --ignore-param <NAME>  Query parameter names to ignore in gap analysis (repeatable); the CI-passed auth token lives here, for instance
      --check                Exit with status 1 when the traffic addresses undeclared endpoints or parameters (old spelling: `--exit-on-gap`)
      --format <FORMAT>      Output format for the finding list [default: text] [possible values: text, json, sarif, junit]
  -h, --help                 Print help (see more with '--help')
```

## `suspect har`

```text
Export recorded traffic as an HTTP Archive (HAR 1.2) — the interchange format profiling and debugging tools read. Accepts a Suspect Cassette or a Suspect Journal

Usage: suspect har [OPTIONS] <TRAFFIC>

Arguments:
  <TRAFFIC>  Traffic file: a Suspect Cassette or Suspect Journal

Options:
  -o, --output <FILE>  Write the archive here instead of stdout
  -h, --help           Print help
```

## `suspect bench`

```text
Wall-clock micro-benchmark of the pipeline stages on one fixture

Usage: suspect bench [OPTIONS] <FIXTURE>

Arguments:
  <FIXTURE>  Fixture document

Options:
      --iters <ITERS>    Iterations per stage (mean reported) [default: 3]
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect bridge`

```text
Run the live contract bridge: watch the spec and emit regeneration plans, evolution proposals, and reconciliation conflicts

Usage: suspect bridge [OPTIONS] <SPEC>

Arguments:
  <SPEC>  Entry OpenAPI document to watch

Options:
      --interval-ms <INTERVAL_MS>  Polling interval in milliseconds [default: 250]
      --format <FORMAT>            Event stream format: human lines or one JSON object per line [default: text] [possible values: text, ndjson]
  -h, --help                       Print help (see more with '--help')
```

## `suspect reverse`

```text
Extract server framework routes and cross-reference them with the spec: undocumented endpoints, spec-only endpoints, method drift

Usage: suspect reverse [OPTIONS] <SOURCE> <SPEC>

Arguments:
  <SOURCE>  Server source tree or single file with route registrations
  <SPEC>    Entry OpenAPI document

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
      --check            Exit 1 when the result is not clean
  -h, --help             Print help (see more with '--help')
```

## `suspect stateful`

```text
Generate and run stateful dependency-graph test sequences against a live server

Usage: suspect stateful [OPTIONS] <SPEC>

Arguments:
  <SPEC>  Entry OpenAPI document

Options:
      --base-url <BASE_URL>  Base URL prepended to operation paths (default: `test.base_url` from `.suspect.yaml`, `SUSPECT_BASE_URL`, else `http://localhost:8080`)
      --filter <FILTER>      Run only sequences whose target operationId contains this
      --emit                 Print the generated sequences as JSON instead of running them
      --report-file <FILE>   Write the JSON run report here. `--json` is the old `load` spelling
      --junit <FILE>         Write a JUnit XML report here
  -h, --help                 Print help
```

## `suspect test`

```text
Compile an Arazzo document into an executable suite and run it

Usage: suspect test [OPTIONS] <ARAZZO>

Arguments:
  <ARAZZO>  Arazzo document describing the workflows

Options:
      --base-url <BASE_URL>   Base URL prepended to operation paths (default: `test.base_url` from `.suspect.yaml`, `SUSPECT_BASE_URL`, else `http://localhost:8080`)
      --filter <FILTER>       Run only workflows whose id contains this substring
      --input <NAME=VALUE>    Provide a workflow input: `--input name=value` (repeatable). The value is parsed as JSON when it parses, else taken as a string; applies to every workflow that declares the input
      --cassette <CASSETTE>   Run offline against a recorded Suspect Cassette instead of live HTTP
      --format <FORMAT>       Event stream format: human text or one-JSON-per-line ndjson (default: `test.report`, else `text`) [possible values: text, ndjson]
      --message-broker <DIR>  Message broker directory for Arazzo 1.1 AsyncAPI steps: `inbox.jsonl` supplies pre-recorded messages and `outbox.jsonl` collects what the workflow published
      --credentials <FILE>    Credentials file for the security schemes the workflows exercise. Default: `.suspect/credentials.json` discovered by walking up from the Arazzo document; `test.credentials` and `SUSPECT_CREDENTIALS` override it
      --journal <FILE>        Write the journal to this file (append) instead of stdout
      --inputs-file <FILE>    JSON or YAML object file of workflow inputs, applied before `--input` pairs (which override it)
      --report-file <FILE>    Write the JSON run report here. `--json` is the old `load` spelling
      --junit <FILE>          Write a JUnit XML report here
  -h, --help                  Print help (see more with '--help')
```

## `suspect load`

```text
Run Arazzo workflows as weighted virtual-user scenarios under a phased load profile, with latency/throughput metrics and SLO gates

Usage: suspect load [OPTIONS] <INPUT>

Arguments:
  <INPUT>  Load profile (`*.load.yaml`) or an Arazzo document (with --workflow)

Options:
      --base-url <BASE_URL>         Base URL of the system under test; overrides the profile `target`
  -e, --env <ENV>                   Named environment from the profile's `environments`
      --workflow <WORKFLOW_ID>      Profile-less run: workflow id to use as the (only) scenario; repeatable
      --rate <RATE>                 Profile-less run: arrivals per second
      --ramp-to <RAMP_TO>           Profile-less run: ramp the arrival rate linearly to this value
      --vus <VUS>                   Profile-less run: closed-loop concurrent users
      --iterations <ITERATIONS>     Profile-less run: total iterations for --vus (instead of a duration)
      --duration <DURATION>         Profile-less run: phase length (`30s`, `2m`)
      --max-vus <MAX_VUS>           Cap on concurrently running users (arrivals beyond it are skipped)
      --input <KEY=VALUE>           Workflow input binding `key=value` applied to every scenario; repeatable
      --inputs-file <FILE>          JSON or YAML object file of workflow inputs applied to every scenario
      --credentials <FILE>          Credentials file for the security schemes the workflows exercise. Default: `.suspect/credentials.json` discovered by walking up from the Arazzo document; `SUSPECT_CREDENTIALS` overrides. Every virtual user shares one token cache
      --validate-responses          Also validate every response body against the schema the operation declares for its status; a violation counts as a contract failure. Off by default: it costs a schema compile per response on the generator
      --seed <SEED>                 RNG seed for reproducible scenario selection, pools and jitter
      --jitter <JITTER>             Inter-arrival jitter: deterministic spacing or a Poisson process [default: none] [possible values: none, poisson]
      --format <FORMAT>             Event stream format: human lines or one JSON object per line [default: text] [possible values: text, ndjson]
      --report-interval <DURATION>  Seconds between progress snapshots; overrides the profile
      --grace <DURATION>            How long to wait for in-flight users after the last phase
      --timeout <DURATION>          Per-request timeout; overrides the profile
      --http1                       Disable HTTP/2 negotiation
      --insecure                    Accept invalid TLS certificates
      --report-file <FILE>          Write the JSON run report here. `--json` is the old `load` spelling
      --junit <FILE>                Write a JUnit XML report here
      --dry-run                     Resolve and print the plan (phases, scenarios, prerequisites) without sending anything
  -h, --help                        Print help (see more with '--help')
```

## `suspect fuzz`

```text
Fuzz operations with schema-mutating requests against a live server

Usage: suspect fuzz [OPTIONS] <SPEC>

Arguments:
  <SPEC>  Entry OpenAPI document

Options:
      --base-url <BASE_URL>    Base URL prepended to operation paths (default: `test.base_url` from `.suspect.yaml`, `SUSPECT_BASE_URL`, else `http://localhost:8080`)
      --runs <RUNS>            Mutant requests generated per operation [default: 25]
      --filter <FILTER>        Run only operations whose operationId contains this substring
      --journal <FILE>         Write the journal to this file (append) instead of stdout
      --evolved                Grammar-evolved, coverage-guided fuzzing: novel response shapes become seeds the campaign exploits
      --rounds <ROUNDS>        Evolved fuzzing: rounds per operation [default: 8]
      --per-round <PER_ROUND>  Evolved fuzzing: requests per round [default: 12]
      --report-file <FILE>     Write the JSON run report here. `--json` is the old `load` spelling
      --junit <FILE>           Write a JUnit XML report here
  -h, --help                   Print help
```

## `suspect why`

```text
Trace a validation failure to its origin: source location, git blame, and whether recorded traffic ever passed it

Usage: suspect why [OPTIONS] --spec <SPEC> <CONSTRAINT>

Arguments:
  <CONSTRAINT>  The constraint that fired (e.g. `type` or the diagnostic text)

Options:
      --spec <SPEC>          Spec file the failure refers to
      --offset <OFFSET>      Byte offset of the failing constraint in the spec, when known
      --why-line <WHY_LINE>  Line of the failing constraint (1-based), with `--why-col`
      --why-col <WHY_COL>    Column of the failing constraint (1-based), with `--why-line`
      --cassette-dir <DIR>   Directory of recorded cassettes searched for historical traffic (`.jsonl`/`.ndjson`/`.json` files)
      --format <FORMAT>      Output format for the report [default: text] [possible values: text, json]
  -h, --help                 Print help (see more with '--help')
```

## `suspect replay`

```text
Re-issue a recorded cassette against an upstream and report drift

Usage: suspect replay [OPTIONS] --upstream <UPSTREAM> <CASSETTE>

Arguments:
  <CASSETTE>  Recorded Suspect Cassette to replay from

Options:
      --upstream <UPSTREAM>  Upstream base URL the recorded traffic is re-issued against
      --diff                 Print unified diffs of drifted UTF-8 response bodies
      --journal <FILE>       Write the journal to this file (append) instead of stdout
      --report-file <FILE>   Write the JSON run report here. `--json` is the old `load` spelling
      --junit <FILE>         Write a JUnit XML report here
  -h, --help                 Print help
```

## `suspect gen`

```text
Render documentation or custom template manifests from an OpenAPI document

Usage: suspect gen [OPTIONS] <SPEC>

Arguments:
  <SPEC>  Entry OpenAPI document

Options:
      --preset <PRESET>      Shipped documentation preset [possible values: docs-md]
      --manifest <MANIFEST>  Custom generation manifest: `gen.toml`, `gen.yaml` or `gen.json` (templates resolved relative to it)
  -o, --output <DIR>         Output root directory [default: gen-out] [alias: --out]
      --check                Exit 1 when the result is not clean
      --diff                 Also print unified diffs of drifted files; implies `--check`
      --owner <OWNER>        Stable logical owner when sharing an output root across generation streams
      --adopt-identical      Explicitly adopt only byte-identical unowned output
  -h, --help                 Print help
```

## `suspect watch`

```text
Re-run a command whenever documents change under the given roots

Usage: suspect watch <ROOTS>... [-- <COMMAND>...]

Arguments:
  <ROOTS>...    Directories (or files) watched recursively for yaml/yml/json changes
  [COMMAND]...  Command (with arguments) to run and re-run on each change burst

Options:
  -h, --help  Print help
```

## `suspect gateway`

```text
Serve a spec as a mock, or proxy/validate/record against an upstream, or replay a recorded cassette

Usage: suspect gateway [OPTIONS] <SPEC>

Arguments:
  <SPEC>  Entry OpenAPI document (mock/validate/record) or ignored (replay)

Options:
  -p, --port <PORT>                  TCP port to bind on 127.0.0.1 (`0` picks a free port). The bound address is printed on stderr once the gateway can serve. Default: `gateway.port`, else 8080
      --mode <MODE>                  Operating mode. Default: `gateway.mode`, else `mock` [possible values: mock, proxy, validate, record, replay, scenario]
      --scenario <FILE>              Scenario mode: JSON file of scripted steps (`{"steps": [{"method", "path_suffix", "status", "body"}]}`)
      --upstream <URL>               Upstream base URL for proxy/validate/record modes (default: `gateway.upstream`)
      --cassette <CASSETTE>          Cassette path for record output
      --enforce                      Validate mode only: reject invalid requests with 400 instead of forwarding them
      --delay-ms <DELAY_MS>          Fault injection: delay in milliseconds [default: 0]
      --delay-pct <DELAY_PCT>        Fault injection: percent of requests delayed [default: 0]
      --error-status <ERROR_STATUS>  Fault injection: status returned by faulted requests
      --error-pct <ERROR_PCT>        Fault injection: percent of requests faulted [default: 0]
      --journal <FILE>               Write the journal to this file (append) instead of stdout
      --redact-header <NAME>         Header name redacted from journals and cassettes beyond the default denylist (repeatable)
      --redact-json-key <KEY>        JSON body key redacted from journals and cassettes beyond the default denylist (repeatable)
  -h, --help                         Print help (see more with '--help')
```

## `suspect codegen`

```text
Generate a source-selected native SDK package

Usage: suspect codegen [OPTIONS] --package-name <PACKAGE_NAME> [SPEC]

Arguments:
  [SPEC]  Entry OpenAPI document

Options:
      --pins <PINS>
          Immutable pin manifest for cache-only generation
      --cache-dir <CACHE_DIR>
          Cache populated by `suspect acquire`; used with --pins
      --insecure-test-origin <INSECURE_TEST_ORIGIN>
          Numeric-loopback origin recorded by an explicitly acquired test manifest
      --profile <PROFILE>
          Native capability profile; unsupported contracts fail before output. Defaults to `.suspect.yaml`'s `codegen.profile` when omitted [possible values: typescript-http, rust-http, python-http, go-http, swift-http, ruby-http, csharp-http, dart-http, cpp-http, kotlin-http, php-http, java-http]
      --operation-id <OPERATION_ID>
          Exact operationId selector (repeatable); omit to attempt all operations
      --package-name <PACKAGE_NAME>
          Native package identity, independent of the API title
      --package-version <PACKAGE_VERSION>
          Exact native-compatible package SemVer, independent of API info.version. Defaults to `.suspect.yaml`'s `codegen.package_version` when omitted
      --import-name <IMPORT_NAME>
          Native import/module identity, independent of the source API semantics
      --compatibility-profile <COMPATIBILITY_PROFILE>
          Explicit versioned source interpretation (repeatable); ordinary OpenAPI semantics remain the default, independently of native feature support [possible values: legacy-binary-string-v1, oas30-nullable-in-3.1-v1, colon-path-parameters-v1, schemaless-stream-events-v1, all-of-scoped-mode-v1]
      --defaults <FILE>
          Generation options file: a JSON or YAML (`.yaml`/`.yml`) document holding `compatibility_profiles`, `credential_env` and `sdk_defaults` (the same schema `codegen-session` accepts). Flag-declared compatibility profiles are added on top
  -o, --output <DIR>
          Output root for generated artifacts. Defaults to `.suspect.yaml`'s `codegen.out`, then `codegen-out` [alias: --out]
      --check
          Check ownership and drift without writing
      --format <FORMAT>
          Human-readable output or a structured report [default: text] [possible values: text, json]
  -h, --help
          Print help (see more with '--help')
```

## `suspect codegen-terraform`

```text
Generate a Terraform provider through explicit lifecycle mappings and the generated Go SDK

Usage: suspect codegen-terraform [OPTIONS] --mapping <MAPPING> --target-config <TARGET_CONFIG> [SPEC]

Arguments:
  [SPEC]  Entry OpenAPI document providing every API operation and schema

Options:
      --pins <PINS>
          Immutable pin manifest; generation reads its verified cache only
      --cache-dir <CACHE_DIR>
          Cache populated by `suspect acquire` for --pins input
      --insecure-test-origin <INSECURE_TEST_ORIGIN>
          Numeric-loopback origin from an explicitly acquired test manifest
      --mapping <MAPPING>
          Closed, versioned Terraform lifecycle/state mapping (JSON, or YAML by `.yaml`/`.yml` extension)
      --target-config <TARGET_CONFIG>
          Provider/toolchain identity and pinned generated-Go-SDK dependency (JSON, or YAML by extension)
  -o, --output <DIR>
          Output root containing go/ and the dependent terraform/ artifacts [default: terraform-out] [alias: --out]
      --check
          Read-only ownership/drift check
      --format <FORMAT>
          Text output or one structured generation report [default: text] [possible values: text, json]
  -h, --help
          Print help (see more with '--help')
```

## `suspect codegen-cli`

```text
Generate a Go/Cobra API CLI application over its own embedded generated Go SDK

Usage: suspect codegen-cli [OPTIONS] --mapping <MAPPING> --target-config <TARGET_CONFIG> [SPEC]

Arguments:
  [SPEC]  Entry OpenAPI document providing every API operation and schema

Options:
      --pins <PINS>
          Immutable pin manifest; generation reads its verified cache only
      --cache-dir <CACHE_DIR>
          Cache populated by `suspect acquire` for --pins input
      --insecure-test-origin <INSECURE_TEST_ORIGIN>
          Numeric-loopback origin from an explicitly acquired test manifest
      --mapping <MAPPING>
          Closed, versioned application surface mapping (JSON, or YAML by `.yaml`/`.yml` extension)
      --target-config <TARGET_CONFIG>
          Package/toolchain identity, credential policy and runtime bounds (JSON, or YAML by extension)
  -o, --output <DIR>
          Output root owning the application and its embedded generated SDK [default: cli-out for codegen-cli, mcp-out for codegen-mcp] [alias: --out]
      --check
          Read-only ownership/drift check
      --format <FORMAT>
          Text output or one structured generation report [default: text] [possible values: text, json]
  -h, --help
          Print help (see more with '--help')
```

## `suspect codegen-mcp`

```text
Generate a TypeScript stdio MCP server over its own embedded generated TypeScript SDK

Usage: suspect codegen-mcp [OPTIONS] --mapping <MAPPING> --target-config <TARGET_CONFIG> [SPEC]

Arguments:
  [SPEC]  Entry OpenAPI document providing every API operation and schema

Options:
      --pins <PINS>
          Immutable pin manifest; generation reads its verified cache only
      --cache-dir <CACHE_DIR>
          Cache populated by `suspect acquire` for --pins input
      --insecure-test-origin <INSECURE_TEST_ORIGIN>
          Numeric-loopback origin from an explicitly acquired test manifest
      --mapping <MAPPING>
          Closed, versioned application surface mapping (JSON, or YAML by `.yaml`/`.yml` extension)
      --target-config <TARGET_CONFIG>
          Package/toolchain identity, credential policy and runtime bounds (JSON, or YAML by extension)
  -o, --output <DIR>
          Output root owning the application and its embedded generated SDK [default: cli-out for codegen-cli, mcp-out for codegen-mcp] [alias: --out]
      --check
          Read-only ownership/drift check
      --format <FORMAT>
          Text output or one structured generation report [default: text] [possible values: text, json]
  -h, --help
          Print help (see more with '--help')
```

## `suspect codegen-profiles`

```text
List the native SDK or application profiles available in this CLI build

Usage: suspect codegen-profiles [OPTIONS]

Options:
      --kind <KIND>      Profile family: native SDK backends, or application targets [default: sdk] [possible values: sdk, applications]
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect codegen-session`

```text
Persistent canonical SDK generation with readonly preview and watch mode

Usage: suspect codegen-session [OPTIONS] --config <CONFIG>

Options:
      --config <CONFIG>            JSON or YAML (`.yaml`/`.yml`) configuration file; relative source paths resolve from its directory
  -o, --output <DIR>               Owned output root [default: sdk-out] [alias: --out]
      --watch                      Preserve a process/session across input and configuration changes
      --check                      Check disk drift and conflicts without writing
      --preview                    Return complete generated contents as readonly preview artifacts
      --format <FORMAT>            Human-readable status or versioned newline-delimited JSON records (`ndjson`; `json` is accepted) [default: text] [possible values: text, ndjson]
      --interval-ms <INTERVAL_MS>  Content-fingerprint polling interval, in milliseconds. 50ms keeps perceived refresh latency low; a poll hashes only the closure's bytes, so the cost is negligible next to any real change [default: 50]
  -h, --help                       Print help (see more with '--help')
```

## `suspect codegen-compare`

```text
Compare wire contracts and native SDK interfaces with migration notes

Usage: suspect codegen-compare [OPTIONS] --before <BEFORE> --after <AFTER>

Options:
      --before <BEFORE>  Baseline SDK session configuration; its source is relative to this file
      --after <AFTER>    Candidate SDK session configuration; its source is relative to this file
      --format <FORMAT>  Versioned report JSON or human-readable Markdown migration notes [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect release-plan`

```text
Recommend a release version and changelog from two spec revisions

Usage: suspect release-plan [OPTIONS] [OLD] [NEW]

Arguments:
  [OLD]  The previous spec revision (file mode only)
  [NEW]  The current spec revision (file mode only)

Options:
      --current <NEW>    The current spec revision in baseline mode; the previous revision comes from git, so only one file is named
      --baseline <REF>   Plan against this git ref (tag, branch, or commit) instead of the `old` file: the "what did we last release" baseline
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect impact`

```text
Evaluate recorded traffic against a candidate contract revision

Usage: suspect impact [OPTIONS] <OLD> <NEW> <CASSETTE>

Arguments:
  <OLD>       The previous spec revision
  <NEW>       The candidate spec revision
  <CASSETTE>  Recorded Suspect Cassette of production-like traffic

Options:
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect release-publish`

```text
Plan and execute SDK publishing across registries

Usage: suspect release-publish [OPTIONS]

Options:
      --manifest <MANIFEST>  Release manifest: a `suspect.project.json` with a `release` section, or a standalone `release.json` (deprecated). Defaults to `suspect.project.json` when it has `release`, then `release.json`
      --dry-run              Print the plan only (the default)
      --execute              Actually run the publish commands. Publishing is destructive on the registry, so it never happens without this flag
      --resume               Resume: skip targets already recorded as published in the state file
      --state <STATE>        State file recording completed targets [default: .release-state.json]
      --format <FORMAT>      Output format for the report [default: text] [possible values: text, json]
  -h, --help                 Print help (see more with '--help')
```

## `suspect release-workflow`

```text
Render the release manifest as a tag-triggered CI workflow

Usage: suspect release-workflow [OPTIONS]

Options:
      --manifest <MANIFEST>  Release manifest: a `suspect.project.json` with a `release` section, or a standalone `release.json` (deprecated). Defaults to `suspect.project.json` when it has `release`, then `release.json`
  -o, --output <OUTPUT>      Write the workflow here instead of stdout
  -h, --help                 Print help
```

## `suspect evidence`

```text
Print the SDK verification matrix: backend x feature -> evidence

Usage: suspect evidence [OPTIONS]

Options:
      --fragments <FRAGMENTS>  Path to the behavioral fragment directory [default: crates/suspect-codegen/tests/behavioral]
      --format <FORMAT>        Output format for the matrix [default: text] [possible values: text, json]
  -h, --help                   Print help (see more with '--help')
```

## `suspect ci`

```text
Run validate, contract, breaking, codegen, and tests across every project in a workspace as one gate

Usage: suspect ci [OPTIONS] [ROOT]

Arguments:
  [ROOT]  Root to search for projects [default: .]

Options:
      --baseline <REF>   Git ref each project is compared against for breaking changes (default: `diff.baseline` from `.suspect.yaml`, or `SUSPECT_GIT_BASE`)
      --stage <NAME>     Only run these stages (repeatable): validate, lint, contract, breaking, codegen, test
      --skip-tests       Skip contract tests (they may need a live server)
      --build            Build every project before gating it: overlays, publication, validation, contract, docs and SDK generation, then the gate. One command from a clean checkout to a verified workspace
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect contract`

```text
Emit or verify a machine-readable contract package

Usage: suspect contract [OPTIONS] <INPUT>

Arguments:
  <INPUT>  Entry OpenAPI document

Options:
  -o, --output <DIR>    Output directory for the contract package [default: contract] [alias: --out]
      --to <TO>         Description serialization [default: yaml] [possible values: json, yaml]
      --overlay <FILE>  Overlay documents applied in order before emission (repeatable)
      --check           Verify the existing package matches the current source instead of writing; exit 1 when it is stale or missing
  -h, --help            Print help (see more with '--help')
```

## `suspect config`

```text
Show the configuration in effect, or with `--explain` every setting and where its value came from

Usage: suspect config [OPTIONS] [INPUT]
       suspect config <COMMAND>

Commands:
  migrate  Move a manifest's deprecated `lint`, `validate` and `editor` sections into the settings file and point the manifest at it
  help     Print this message or the help of the given subcommand(s)

Arguments:
  [INPUT]  Report the configuration a command on this input would use: the nearest settings file above it, else the one above the working directory

Options:
      --explain          Print every setting with its value and origin (`flag`, `env SUSPECT_X`, `.suspect.yaml:12`, `default`), plus debug variables
      --set-only         With `--explain`: hide settings that hold their default
      --format <FORMAT>  Output format for the report [default: text] [possible values: text, json]
  -h, --help             Print help (see more with '--help')
```

## `suspect config migrate`

```text
Move a manifest's deprecated `lint`, `validate` and `editor` sections into the settings file and point the manifest at it

Usage: suspect config migrate [MANIFEST]

Arguments:
  [MANIFEST]  The manifest to migrate [default: suspect.project.json]

Options:
  -h, --help  Print help
```

## `suspect project`

```text
Build and check a suspect project from one manifest

Usage: suspect project <COMMAND>

Commands:
  init   Write a starter `suspect.project.json` in the given directory
  check  Validate the manifest and every declared input
  build  Execute the build pipeline: overlays → publish → profiles → validate → docs → tests
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

## `suspect project init`

```text
Write a starter `suspect.project.json` in the given directory

Usage: suspect project init [DIR]

Arguments:
  [DIR]  Project directory (defaults to the current directory) [default: .]

Options:
  -h, --help  Print help
```

## `suspect project check`

```text
Validate the manifest and every declared input

Usage: suspect project check [OPTIONS]

Options:
      --manifest <MANIFEST>  Manifest path (defaults to `suspect.project.json`) [default: suspect.project.json]
  -h, --help                 Print help
```

## `suspect project build`

```text
Execute the build pipeline: overlays → publish → profiles → validate → docs → tests

Usage: suspect project build [OPTIONS]

Options:
      --manifest <MANIFEST>  Manifest path (defaults to `suspect.project.json`) [default: suspect.project.json]
      --skip-tests           Skip contract tests (docs/validate still run)
  -h, --help                 Print help
```

## `suspect lsp`

```text
Run the language server over stdio

Usage: suspect lsp

Options:
  -h, --help  Print help
```

