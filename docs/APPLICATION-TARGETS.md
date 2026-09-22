# Application targets — Go API CLI and TypeScript stdio MCP server

Two generators in `suspect-codegen` emit *applications* rather than SDK
languages: a Go/Cobra command-line program (`suspect codegen-cli`) and a
TypeScript stdio Model Context Protocol server (`suspect codegen-mcp`). Each
sits on top of a canonical generated Suspect SDK, which it embeds.

Every command line in this document was executed against the fixture contracts
in `crates/suspect-codegen/tests/fixtures/{api-cli-v1,mcp-v1}/`. Output blocks
are transcripts, edited only to elide long artifact lists (marked `…`) and to
replace scratch directories with `$WORK`. Steps that need a toolchain or
network are marked **environment-dependent**.

## Contents

- [Model](#model)
- [The two suspect commands](#the-two-suspect-commands)
- [Walkthrough: Go API CLI](#walkthrough-go-api-cli)
- [Walkthrough: TypeScript MCP server](#walkthrough-typescript-mcp-server)
- [CLI mapping reference](#cli-mapping-reference)
- [CLI target configuration reference](#cli-target-configuration-reference)
- [MCP mapping reference](#mcp-mapping-reference)
- [MCP target configuration reference](#mcp-target-configuration-reference)
- [Numeric representation](#numeric-representation)
- [Optional presence semantics](#optional-presence-semantics)
- [Generated CLI runtime](#generated-cli-runtime)
- [Generated MCP server runtime](#generated-mcp-server-runtime)
- [Application surface manifests](#application-surface-manifests)
- [Ownership, `--check` and drift](#ownership---check-and-drift)
- [Pinned inputs](#pinned-inputs)
- [Refusal codes](#refusal-codes)
- [Supported scope](#supported-scope)
- [Deferred scope](#deferred-scope)
- [Known limitations](#known-limitations)
- [Verification and CI](#verification-and-ci)

## Model

**One owned root per application.** An output root holds the application *and*
its embedded generated SDK as a single artifact set: one Go module
(`internal/sdk/` plus `internal/cli/` plus `cmd/<binary>/`), or one private npm
package (`typescript/` plus `server/`). The embedded SDK is not a dependency —
the Go SDK's own `go.mod` and the TypeScript SDK's own `package.json`,
lockfile, build config and examples are dropped, so the root installs and
builds as exactly one artifact. Each root is registered under its own stable
owner (`suspect-cli-app:lifecycle-v1`, `suspect-mcp-app:lifecycle-v1`), so
neither application can adopt, overwrite or prune the other's files.

**Closed mapping and configuration.** Two JSON documents drive generation, and
both are *closed*: `deny_unknown_fields` everywhere, no defaults, no inferred
names. The mapping explicitly allowlists which source operations are exposed
and allocates every public name (command paths and flags; tool names and input
properties). The target configuration carries package/toolchain identity, the
credential-environment policy and the finite runtime bounds. Because nothing
defaults, no exposed operation, public name, request body, confirmation policy
or runtime bound can be concealed by omission.

**Generation refuses; the emitted program does not discover.** HTTP
serialization, authentication, schema validation and exact JSON handling stay
in the canonical SDK and its generated codecs. Anything the application layer
cannot carry exactly — a non-JSON media type, a parameter that is not a scalar,
a schema with no unambiguous tool projection, a colliding or reserved public
name, an unverified toolchain pin — fails during generation with a diagnostic
located at both its mapping pointer and its contract source. It never fails at
the emitted program's first use.

## The two suspect commands

```
suspect codegen-cli [SPEC] --mapping <FILE> --target-config <FILE> [OPTIONS]
suspect codegen-mcp [SPEC] --mapping <FILE> --target-config <FILE> [OPTIONS]
```

| Argument / option | Meaning |
| --- | --- |
| `SPEC` (positional) | Entry OpenAPI document. Required unless `--pins` is given; mutually exclusive with it. |
| `--pins <FILE>` | Immutable pin manifest; generation reads its verified cache only. |
| `--cache-dir <DIR>` | Cache populated by `suspect acquire`. Requires `--pins`; defaults to `.suspect-cache`. |
| `--insecure-test-origin <ORIGIN>` | Numeric-loopback origin from an explicitly acquired test manifest. Repeatable; requires `--pins`. |
| `--mapping <FILE>` | Closed, versioned application surface mapping JSON. Required. |
| `--target-config <FILE>` | Identity, credential policy and runtime bounds JSON. Required. |
| `-o`, `--out <DIR>` | Output root. Defaults to `cli-out` for `codegen-cli`, `mcp-out` for `codegen-mcp`. |
| `--check` | Read-only ownership/drift check. Never writes. |
| `--format <text\|json>` | Text diagnostics (default) or one structured generation report. |

Mapping and target configuration are read and parsed **before** the input
document is opened, so a malformed mapping never reaches the network, the pin
cache or the output root.

Exit status is `0` only for status `generated` or `current`; `1` for `drift`,
`conflict` and `failed`.

| | `codegen-cli` | `codegen-mcp` |
| --- | --- | --- |
| Mapping profile | `suspect.application.cli.v1` | `suspect.application.mcp.v1` |
| Surface manifest format | `suspect.application.cli.surface.v1` | `suspect.application.mcp.surface.v1` |
| Report format | `suspect.application.cli.generation.v1` | `suspect.application.mcp.generation.v1` |
| Output-root owner | `suspect-cli-app:lifecycle-v1` | `suspect-mcp-app:lifecycle-v1` |
| Target-specific refusal-code prefix | `cli-`, `application-cli-` | `mcp-`, `application-mcp-` |

Those prefixes cover only the codes a target produces on its own. Refusals
from the *shared* application layer are deliberately target-neutral and carry
no target prefix. There are exactly four, emitted verbatim by both commands
because both resolve selectors, bind the resolved operation and relativize
manifest documents through the same shared code:
`application-operation-missing`, `application-operation-ambiguous`,
`application-path-item-unplanned` and
`application-document-outside-entry-tree`. See
[Refusal codes](#refusal-codes) for what each one means.

### Discovery

Application profiles are listed separately from SDK language profiles:

```console
$ suspect codegen-profiles --kind applications
codegen-cli	suspect.application.cli.v1	suspect.application.cli.surface.v1	suspect-cli-app:lifecycle-v1
codegen-mcp	suspect.application.mcp.v1	suspect.application.mcp.surface.v1	suspect-mcp-app:lifecycle-v1
```

`--format json` emits `"format": "suspect.application.profiles.v1"` with one
entry per target carrying `command`, `profile` (the mapping profile the target
accepts, reported once), `surfaceFormat`, `owner`, `requires`
(`["--mapping","--target-config"]`) and `experimental: true`.

## Walkthrough: Go API CLI

The contract is `crates/suspect-codegen/tests/fixtures/api-cli-v1/openapi.json`
(copied to `$WORK/widgets.openapi.json`): an API-key-secured widget control
plane with `getWidget`, `createWidget` and `purgeWidgets`.

### 1. Mapping — `$WORK/cli-mapping.json`

```json
{
  "format": "suspect.application.cli.v1",
  "description": "Explicitly mapped widget control-plane operations.",
  "commands": [
    {
      "selector": "getWidget",
      "path": ["widgets", "get"],
      "summary": "Read one widget",
      "description": "Reads exactly one widget and writes its exact JSON representation.",
      "parameters": {
        "id": { "flag": "id", "description": "Exact widget identifier." },
        "verbose": { "flag": "verbose", "description": "Request the verbose projection." },
        "limit": { "flag": "limit", "description": "Exact maximum number of related records." },
        "label": { "flag": "label", "description": "Exact label filter." },
        "trace": { "flag": "trace", "description": "Caller-supplied trace identifier." }
      },
      "body": { "kind": "none" },
      "confirmation": { "kind": "not_required" }
    },
    {
      "selector": "POST /widgets",
      "path": ["widgets", "create"],
      "summary": "Create one widget",
      "description": "Sends one finite JSON document read from a file or standard input.",
      "parameters": {},
      "body": { "kind": "json_document" },
      "confirmation": { "kind": "not_required" }
    },
    {
      "selector": "purgeWidgets",
      "path": ["widgets", "purge"],
      "summary": "Delete every widget in one scope",
      "description": "Removes every widget in the named scope; this cannot be undone.",
      "parameters": {
        "scope": { "flag": "scope", "description": "Exact scope name to purge." }
      },
      "body": { "kind": "none" },
      "confirmation": { "kind": "required", "token": "purge-widgets" }
    }
  ]
}
```

### 2. Target configuration — `$WORK/cli-target.json`

```json
{
  "module_path": "example.com/widget-cli",
  "binary_name": "widgetctl",
  "version": "1.4.0",
  "go_version": "1.24.0",
  "go_toolchain": "go1.27.1",
  "credential_env": { "version": "v1", "schemes": { "apiKey": "WIDGET_API_KEY" } },
  "runtime": { "request_deadline_ms": 30000, "max_input_bytes": 1048576 },
  "output": { "format": "compact_json" }
}
```

### 3. Generate

```console
$ suspect codegen-cli $WORK/widgets.openapi.json \
    --mapping $WORK/cli-mapping.json \
    --target-config $WORK/cli-target.json \
    --out $WORK/widgetctl --format json
{
  "artifacts": [
    "application-surface.json",
    "cmd/widgetctl/main.go",
    "go.mod",
    "go.sum",
    "internal/cli/commands.go",
    "internal/cli/runtime.go",
    "internal/sdk/codecs.go",
    …
  ],
  "diagnostics": [],
  "format": "suspect.application.cli.generation.v1",
  "operations": [
    { "document": "file://…/widgets.openapi.json", "nativeMethod": "PurgeWidgets",
      "operationId": "purgeWidgets", "pointer": "/paths/~1widgets/delete" },
    …
  ],
  "out": "…/widgetctl",
  "owner": "suspect-cli-app:lifecycle-v1",
  "profile": "suspect.application.cli.v1",
  "status": "generated",
  "surfaceFormat": "suspect.application.cli.surface.v1"
}
```

42 artifacts, exit status `0`. The text form of the same run prints
`codegen-cli generated: 3 mapped SDK operations, 42 planned artifacts
(suspect.application.cli.v1)`.

### 4. Build — environment-dependent (Go toolchain)

The emitted module's pinned metadata is complete on its own, so it builds under
the default `-mod=readonly` with no repair:

```console
$ cd $WORK/widgetctl
$ GOWORK=off GOFLAGS=-mod=readonly go build ./...
$ GOWORK=off GOFLAGS=-mod=readonly go build -o widgetctl ./cmd/widgetctl
```

Verified with `go1.27.1 darwin/arm64`. The `-mod=readonly` setting is
deliberate: under `-mod=mod` the toolchain would silently repair a missing
`require` line or `go.sum` row, so an incomplete pinned dependency set would
pass here and fail for everybody else.

### 5. Help and completion need no credentials and no network

```console
$ ./widgetctl --help
Explicitly mapped widget control-plane operations.

Machine output is exact JSON written to standard output; every diagnostic is
written to standard error. Numbers keep their exact source tokens.

Exit codes:
  exit 0  the request succeeded, or the command needed no request
  exit 1  runtime failure: transport, deadline, cancellation, codec or output
  exit 2  usage failure: the invocation was refused before anything was sent
  exit 3  documented API failure: the API returned a response the contract
          declares as a failure; its exact document is on standard output

Credentials come only from these environment variables. The embedded SDK
reads them when an API command runs, never during help or completion:
  WIDGET_API_KEY supplies the source security scheme "apiKey"

Every request uses a 30000ms deadline and stops on SIGINT or SIGTERM. No request is
retried and no pagination is traversed.

Usage:
  widgetctl [flags]
  widgetctl [command]
…
```

`./widgetctl completion bash` and `./widgetctl --version`
(`widgetctl version 1.4.0`) also run with the credential variable unset.

### 6. Run against a loopback API — environment-dependent (python3)

`crates/suspect-codegen/tests/fixtures/api-cli-v1/loopback.py` binds
`127.0.0.1` on an ephemeral port, prints the port, and serves exactly the
fixture's operations. `$PORT` below is that port.

```console
$ WIDGET_API_KEY=loopback-key ./widgetctl widgets get \
    --id w-1 --limit 9007199254740993 --verbose=false --label "" --trace t-9 \
    --server-url http://127.0.0.1:$PORT/v1
{"active":false,"amount":9007199254740993,"echo":{"apiKey":"loopback-key","body":null,"query":"verbose=false&limit=9007199254740993&label=","trace":"t-9"},"id":"w-1"}
```

Exit `0`. Four things are visible in that one line: the large integer survives
in both directions as its exact token; `--verbose=false` reaches the wire as
`verbose=false` rather than being dropped; `--label ""` reaches it as `label=`;
and the credential came from the environment, never from a flag.

A request body arrives by file or standard input:

```console
$ echo -n '{"name":"alpha","amount":9007199254740993,"active":false,"note":null,"tags":[]}' \
    | WIDGET_API_KEY=loopback-key ./widgetctl widgets create --body-file - \
      --server-url http://127.0.0.1:$PORT/v1
{"active":false,"amount":9007199254740993,"echo":{…,"body":"{\"active\":false,\"amount\":9007199254740993,\"name\":\"alpha\",\"note\":null,\"tags\":[]}",…},"id":"w-created","note":null}
```

The echoed `body` is the exact bytes the generated codec put on the wire: the
large integer keeps its token, and `false`, `null` and `[]` all survive as
themselves.

### 7. Exit-code categories, observed

```console
$ echo -n '{"name":"deny","amount":1}' | ./widgetctl widgets create --body-file - …
{"code":422,"message":"rejected"}
POST /widgets (createWidget) returned documented failure status 422
# exit 3 — the documented failure document is on stdout, the diagnostic on stderr

$ ./widgetctl widgets purge --scope staging …
refusing to send DELETE /widgets (purgeWidgets) without confirmation: pass --confirm purge-widgets, or --confirm - to read the token from standard input
DELETE /widgets (purgeWidgets) requires explicit confirmation
# exit 2 — nothing was sent, stdout is empty

$ ./widgetctl widgets purge --scope staging --confirm purge-widgets …
# exit 0, no output (the 204 response declares no document)

$ echo purge-widgets | ./widgetctl widgets purge --scope staging --confirm - …
# exit 0 — the token may come from the first line of standard input

$ ./widgetctl widgets get --id w-1 --limit 1.5 …
--limit is not an exact integer for GET /widgets/{id} (getWidget): json syntax: numeric token is not an integer
# exit 2 — rejected by the SDK's exact integer tokenizer before anything was sent

$ ./widgetctl widgets get --id w-1 --server-url http://127.0.0.1:1/v1
GET /widgets/{id} (getWidget) failed: SDK transport failure
# exit 1
```

### 8. Refusals, observed

A mapping that names operations no exact command surface can carry:

```console
$ suspect codegen-cli $WORK/widgets.openapi.json --mapping $WORK/cli-mapping-refused.json …
…/widgets.openapi.json:123:18 [cli-unsupported-media] response "200" needs either no content or exactly one concrete JSON media with a generated codec (/paths/~1blobs~1{id}/get/responses/200; mapping /commands/0)
…/widgets.openapi.json:136:11 [cli-parameter-unrepresentable] source parameter "id" cannot be carried by one exact flag: a flag carries only a native string, boolean, integer or number scalar (/paths/~1matrices~1{id}/get/parameters/0; mapping /commands/1/parameters/id)
codegen-cli failed: 0 mapped SDK operations, 0 planned artifacts (suspect.application.cli.v1)
```

Exit `1`, and the output root is never created. Each diagnostic carries the
contract document, line, column and JSON pointer, plus the pointer into the
caller's own mapping. Selector resolution runs first, so an unresolvable
selector short-circuits the rest:

```console
…/widgets.openapi.json:1:1 [application-operation-ambiguous] selector "dup" must identify exactly one outgoing Contract operation (found 2) (; mapping /commands/2/selector)
```

## Walkthrough: TypeScript MCP server

The contract is `crates/suspect-codegen/tests/fixtures/mcp-v1/openapi.json` —
the same widget control plane plus operations that are deliberately
unprojectable.

### 1. Mapping — `$WORK/mcp-mapping.json`

```json
{
  "format": "suspect.application.mcp.v1",
  "description": "Explicitly mapped widget control-plane operations exposed as MCP tools.",
  "tools": [
    {
      "selector": "getWidget",
      "name": "read_widget",
      "title": "Read widget",
      "description": "Reads exactly one widget and returns its exact JSON representation.",
      "annotations": {
        "read_only": true, "destructive": false,
        "idempotent": true, "open_world": true
      },
      "input": {
        "parameters": {
          "id": { "property": "id", "description": "Exact widget identifier." },
          "verbose": { "property": "verbose", "description": "Request the verbose projection." },
          "limit": { "property": "limit", "description": "Exact maximum number of related records." },
          "label": { "property": "label", "description": "Exact label filter." },
          "trace": { "property": "trace", "description": "Caller-supplied trace identifier." }
        },
        "body": { "kind": "none" }
      }
    },
    {
      "selector": "POST /widgets",
      "name": "create_widget",
      "title": "Create widget",
      "description": "Creates exactly one widget from a finite JSON document.",
      "annotations": {
        "read_only": false, "destructive": false,
        "idempotent": false, "open_world": true
      },
      "input": {
        "parameters": {},
        "body": {
          "kind": "json_value",
          "property": "widget",
          "description": "The complete widget document to create."
        }
      }
    },
    {
      "selector": "purgeWidgets",
      "name": "purge_widgets",
      "title": "Purge widgets",
      "description": "Removes every widget in the named scope; this cannot be undone.",
      "annotations": {
        "read_only": false, "destructive": true,
        "idempotent": true, "open_world": true
      },
      "input": {
        "parameters": {
          "scope": { "property": "scope", "description": "Exact scope name to purge." }
        },
        "body": { "kind": "none" }
      }
    }
  ]
}
```

### 2. Target configuration — `$WORK/mcp-target.json`

```json
{
  "package_name": "widget-mcp-server",
  "bin_name": "widget-mcp",
  "server_name": "widget-control-plane",
  "version": "1.4.0",
  "node_version": "26.8.1",
  "node_minimum_major": 22,
  "npm_version": "11.19.0",
  "typescript_version": "5.9.3",
  "node_types_version": "26.6.2",
  "mcp_server_version": "2.0.0",
  "mcp_client_version": "2.0.0",
  "credential_env": { "version": "v1", "schemes": { "apiKey": "WIDGET_API_KEY" } },
  "server_url_env": "WIDGET_SERVER_URL",
  "runtime": {
    "call_deadline_ms": 30000,
    "max_input_bytes": 262144,
    "max_result_bytes": 1048576
  },
  "logs": { "policy": "calls" }
}
```

### 3. Generate

```console
$ suspect codegen-mcp $WORK/widgets.openapi.json \
    --mapping $WORK/mcp-mapping.json \
    --target-config $WORK/mcp-target.json \
    --out $WORK/widget-mcp
codegen-mcp generated: 3 mapped SDK operations, 34 planned artifacts (suspect.application.mcp.v1)

$ ls $WORK/widget-mcp
application-surface.json  package-lock.json  package.json  README.md  server  tsconfig.json  typescript
```

`server/main.ts` holds the tool registrations, `server/runtime.ts` the
tool-call runtime, `typescript/` the embedded canonical SDK sources, and
`README.md` a generated guide covering build, run, client configuration,
credentials and the value surface.

### 4. Install and build — environment-dependent (Node, npm registry access)

```console
$ cd $WORK/widget-mcp
$ npm ci --ignore-scripts
$ npm run build

> widget-mcp-server@1.4.0 build
> tsc --project tsconfig.json

$ ls dist/server/main.js
dist/server/main.js
```

Verified with Node `v26.8.1` and npm `11.19.0`. `npm ci` resolves only the
emitted `package-lock.json`, which carries the reviewed registry resolutions
and integrity digests for `@modelcontextprotocol/server@2.0.0`,
`typescript@5.9.3` and `@types/node@26.6.2`. Neither install nor build needs a
credential, runs a lifecycle hook, or contacts the API. **This step requires
npm registry access**; it is the only step in this document that does.

### 5. Drive it with the official MCP client — environment-dependent

The acceptance harness is the official client package
(`@modelcontextprotocol/client@2.0.0`) installed beside the application; the
script used here is
`crates/suspect-codegen/tests/fixtures/mcp-v1/client.mjs`. It launches the
built entry point over stdio with `WIDGET_SERVER_URL` and `WIDGET_API_KEY` in
the child environment — never in a tool argument — and pins protocol
negotiation to one fixed era.

```console
$ node client.mjs $WORK/widget-mcp/dist/server/main.js http://127.0.0.1:$PORT/v1 loopback-key
```

Discovery is deterministic — the same order and the same schemas on repeat
calls:

```json
{ "names": ["read_widget", "create_widget", "purge_widgets"],
  "repeated": ["read_widget", "create_widget", "purge_widgets"] }
```

`read_widget`'s projected input schema, as the client actually received it:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["id"],
  "properties": {
    "id": { "type": "string", "description": "Exact widget identifier." },
    "label": { "type": "string", "description": "Exact label filter." },
    "verbose": { "type": "boolean", "description": "Request the verbose projection." },
    "trace": { "type": "string", "description": "Caller-supplied trace identifier." },
    "limit": {
      "type": "string",
      "pattern": "^-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?$",
      "description": "Exact maximum number of related records. Supplied as a string holding one exact JSON number token, for example \"-12\" or \"9007199254740993\". The generated codec converts the token to a native bigint integer, so a large or high-precision value is never routed through a floating-point number."
    }
  }
}
```

A real call with `{ id: "w-1", verbose: false, limit: "9007199254740993",
label: "", trace: "t-9" }` returned:

```json
{
  "ok": true,
  "status": 200,
  "mediaType": "application/json",
  "document": {
    "id": "w-1",
    "amount": "9007199254740993",
    "active": false,
    "note": null,
    "echo": {
      "query": "verbose=false&limit=9007199254740993&label=",
      "trace": "t-9",
      "apiKey": "loopback-key",
      "body": null
    }
  }
}
```

with the `content` text block carrying the API's exact document,
`{"id":"w-1","amount":9007199254740993,…}` — numbers as real JSON numbers in
the text, as exact token strings in `structuredContent.document`.

The same session also observed: omitted optional properties staying omitted
(`echo.query` is `""`, `echo.trace` is `null`); a declared upstream 404
returning `isError: true` with `{"ok":false,"status":404,"mediaType":"application/json","document":{…}}`;
a `204` returning `{"ok":true,"status":204,"mediaType":null}` with the text
block `DELETE /widgets (purgeWidgets) returned HTTP 204 with no declared
response document.`; a codec-rejected argument returning
`{"ok":false,"failure":"invalid-argument"}`; and client-side cancellation
reaching the running upstream call. Every server log line arrived on stderr,
in the form `read_widget GET /widgets/{id} (getWidget) -> HTTP 200`.

### 6. Refusals, observed

```console
$ suspect codegen-mcp $WORK/widgets.openapi.json --mapping $WORK/mcp-mapping-refused.json …
…:57:20 [mcp-unsupported-schema] the declared request body has no exact tool input projection: model Node is recursive, and a recursive schema has no finite tool input projection (/components/schemas/Node/properties/child; mapping /tools/3/input/body)
…:132:18 [mcp-unsupported-media] response "200" needs either no content or exactly one concrete JSON media with a generated codec (/paths/~1blobs~1{id}/get/responses/200; mapping /tools/0)
…:176:45 [mcp-unsupported-schema] the declared request body has no exact tool input projection: arbitrary JSON has no tool input projection: its numbers would already be rounded by the client's own JSON parser before the server could preserve them (/paths/~1loose/post/requestBody/content/application~1json/schema/properties/anything; mapping /tools/2/input/body)
…:151:41 [mcp-unsupported-schema] the declared request body has no exact tool input projection: this schema admits both a number and a string, so one JSON string at the tool boundary would have two meanings; expose a single concrete type instead (/paths/~1selectors/post/requestBody/content/application~1json/schema/properties/pick; mapping /tools/1/input/body)
codegen-mcp failed: 0 mapped SDK operations, 0 planned artifacts (suspect.application.mcp.v1)
```

## CLI mapping reference

Profile `suspect.application.cli.v1`. Closed: unknown fields and unknown
variants are errors.

| Field | Type | Semantics |
| --- | --- | --- |
| `format` | string | Must equal `suspect.application.cli.v1`. Any other value → `cli-mapping-version`. |
| `description` | string | Long help for the root command. |
| `commands` | array | The explicit allowlist of exposed operations, in command-tree order. |

### `commands[]`

| Field | Type | Semantics |
| --- | --- | --- |
| `selector` | string | Exact `operationId`, or the exact `METHOD /path` form (case-sensitive HTTP method token, one space, the exact path template with its leading slash). No normalization, trimming or prefix matching. Only outgoing client operations are searched — never webhooks or callbacks. |
| `path` | array of string | Command path under the root: 1..=4 segments. Each segment is 1..=32 characters of lower-case letters, digits and single interior dashes, with no leading/trailing/doubled dash. |
| `summary` | string | One-line help (`Short`). |
| `description` | string | Long help (`Long`); the target operation is appended automatically. |
| `parameters` | object | Flag names keyed by **exact source parameter name**. Every parameter of the selected operation needs exactly one entry; an unmapped parameter is `cli-parameter-unmapped`, an unknown key is `cli-parameter-unknown`. |
| `body` | object | `{"kind":"none"}` or `{"kind":"json_document"}`. Must match whether the operation declares a request body, or `cli-body-policy`. |
| `confirmation` | object | `{"kind":"not_required"}` or `{"kind":"required","token":"<exact token>"}`. |

### `commands[].parameters.<source name>`

| Field | Type | Semantics |
| --- | --- | --- |
| `flag` | string | Long flag name, no leading dashes, same token rule as a path segment. |
| `description` | string | Flag usage text. Requiredness and the exact-token note are appended automatically. |

Reserved command segments: `help`, `completion`, `__complete`,
`__completeNoDesc`. Reserved flag names: `help`, `body`, `body-file`,
`confirm`, `server-url`, `version`. A command path that also prefixes another
mapped command cannot run an operation (`cli-command-collision`) — a group is
never also a leaf.

## CLI target configuration reference

| Field | Type | Semantics |
| --- | --- | --- |
| `module_path` | string | Go module path of the single emitted module, e.g. `example.com/team/ctl`. The embedded SDK becomes `<module_path>/internal/sdk`. |
| `binary_name` | string | Executable name, root command name and `cmd/<name>/` directory. 1..=64 characters, must start with a lower-case letter, then lower-case letters, digits and single interior dashes. |
| `version` | string | Exact SemVer reported by `--version`. Version *requirements* are rejected. |
| `go_version` | string | `go` directive: `X.Y` or `X.Y.Z` of ASCII digits. |
| `go_toolchain` | string | `toolchain` directive: `goX.Y` or `goX.Y.Z`. |
| `credential_env` | object or `null` | Explicit source-scheme → environment **variable name** policy. `null` selects an anonymous application. |
| `runtime.request_deadline_ms` | integer | Whole-request deadline, 1..=3600000. |
| `runtime.max_input_bytes` | integer | Ceiling on a body read from a file or stdin, checked before any decode, 1..=8388608. |
| `output.format` | string | `compact_json` or `pretty_json`. |

`credential_env` is `{"version":"v1","schemes":{<source scheme name>: <ENV VAR
NAME>}}` with 1..=64 entries. Values are variable *names* — 1..=128 ASCII bytes
of letters, digits and underscores starting with a letter or underscore — never
credential values. Generation never reads a variable. Each configured name must
identify exactly one used source Security Scheme declaration across the
selected operations; otherwise the SDK's own `sdk-credential-env-unbound`
finding surfaces at mapping pointer `/commands`.

Callers of the emitted binary cannot raise any runtime bound: they are
constants compiled into `internal/cli/commands.go`.

## MCP mapping reference

Profile `suspect.application.mcp.v1`. Closed, as above.

| Field | Type | Semantics |
| --- | --- | --- |
| `format` | string | Must equal `suspect.application.mcp.v1`, or `mcp-mapping-version`. |
| `description` | string | Prose describing the exposed surface; used by the generated guide and the surface manifest. |
| `tools` | array | The explicit allowlist, in tool-listing order. That order is what `tools/list` returns. |

### `tools[]`

| Field | Type | Semantics |
| --- | --- | --- |
| `selector` | string | Same exact selector grammar as the CLI mapping. |
| `name` | string | Stable public tool name: 1..=64 characters of lower-case letters, digits, underscores and single interior dashes, starting with a lower-case letter. |
| `title` | string | Human-readable display title (also `annotations.title`). |
| `description` | string | Tool description shown during discovery. |
| `annotations` | object | Advisory hints — see below. |
| `input.parameters` | object | Public property names keyed by **exact source parameter name**; exactly one entry per source parameter. |
| `input.body` | object | `{"kind":"none"}` or `{"kind":"json_value","property":"<name>","description":"<text>"}`. |

`annotations` requires all four booleans explicitly: `read_only`,
`destructive`, `idempotent`, `open_world`. They are projected as
`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`. **They
are hints only**: they change nothing the emitted server does, and a client
must not rely on them. Nothing in the server consults them.

`input.parameters.<source name>` carries `property` (1..=64 characters of
letters, digits and underscores, starting with a lower-case letter) and
`description` (prefixed onto the projected schema's own representation note).

Reserved tool names: `completion`, `elicitation`, `initialize`, `logging`,
`mcp`, `notifications`, `ping`, `prompts`, `resources`, `roots`, `sampling`,
`tasks`, `tools`.

There is no filesystem-path input form. A request body arrives as one projected
JSON value under `input.body.property`.

## MCP target configuration reference

| Field | Type | Semantics |
| --- | --- | --- |
| `package_name` | string | npm identity of the single emitted private package. |
| `bin_name` | string | Executable published under `bin`. Same token rule as `binary_name`. |
| `server_name` | string | MCP server identity reported during initialization. 1..=128 bytes, no control characters. |
| `version` | string | Exact SemVer of both the package and the reported server version. |
| `node_version` | string | Exact reviewed Node version `X.Y.Z`, recorded for review and documentation. |
| `node_minimum_major` | integer | Minimum Node major written into `engines.node`. Must be ≥ 22 — the floor the emitted application actually requires: the stricter of the official MCP SDK's own minimum (20) and the embedded canonical TypeScript SDK's own `engines.node` (`>=22`), since that SDK is compiled into the application rather than depended on. Derived in code, not restated. |
| `npm_version` | string | Exact `X.Y.Z` written into `packageManager`. |
| `typescript_version` | string | Must equal `5.9.3`. |
| `node_types_version` | string | Must equal `26.6.2`. The official SDK's declarations reference `node:stream` and `Buffer`, so the package cannot compile without them. |
| `mcp_server_version` | string | Must equal `2.0.0`. |
| `mcp_client_version` | string | Must equal `2.0.0`. Acceptance-testing tool only; the emitted package never depends on it. |
| `credential_env` | object or `null` | As for the CLI. `null` selects an anonymous server, which is **refused** when the selected operations require credentials (`mcp-credentials-unconfigured`). |
| `server_url_env` | string or `null` | Environment variable name that may supply an absolute base URL override. `null` forbids any override. Never a credential and never a tool input. |
| `runtime.call_deadline_ms` | integer | Whole-call deadline, 1..=3600000, combined with the request's own cancellation signal. |
| `runtime.max_input_bytes` | integer | Ceiling on one call's encoded arguments, checked before any codec runs, 1..=8388608. |
| `runtime.max_result_bytes` | integer | Ceiling on one call's exact result document, 1..=8388608. |
| `logs.policy` | string | `off` (nothing, not even a failure), `failures` (one line per failed call) or `calls` (one line per outcome). Every policy writes to stderr only, and none logs an argument or document value. |

The four version pins are compared for exact equality against the
registry-verified constants in `crates/suspect-codegen/src/mcp.rs`; any other
value is `mcp-toolchain-pin` rather than a generation against an unverified
dependency.

## Numeric representation

Both targets keep every schema-declared number as its **exact source token**.
Neither ever routes a number through a float, and neither uses ordinary JSON
serialization for an SDK model: the generated codecs are the only encoder and
decoder. The emitted MCP server contains no `JSON.parse(` or `JSON.stringify(`
call over a model — `server/runtime.ts` uses the SDK's own `parseJson` /
`stringifyJson`.

The canonical example is `9007199254740993` (2⁵³ + 1), which
`JSON.parse`/`JSON.stringify` and every IEEE-754 double round to
`9007199254740992`.

**API CLI.** A numeric flag is a *text* flag parsed by the SDK's exact
tokenizer (`ParseInteger` / `ParseNumber`), so `--limit 9007199254740993`
reaches the wire as `limit=9007199254740993`. A non-integer token for an
integer position is a usage failure before anything is sent (`--limit 1.5` →
exit 2). Machine output is the codec's exact bytes; `pretty_json` only inserts
whitespace *between* already-encoded tokens (`json.Indent`), so no number is
ever re-encoded. Flag help for numeric positions says `Accepts one exact
integer token.` / `Accepts one exact number token.`

**MCP.** A tool's arguments reach the server through the *client's* own JSON
parser, so a real JSON number would already have been rounded before the server
saw it. Every schema-declared numeric position is therefore projected as a
**string holding one exact JSON number token**, in both directions:

```
"pattern": "^-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?$"
```

That is the representation grammar only — the JSON number grammar without
leading zeros or a leading `+`. No declared numeric assertion is ever copied
onto the string: `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`
and `multipleOf` never appear on it. The generated codec enforces them, along
with integrality, pattern and format, and attributes the rejection to the tool
input property it came from. Results travel back the same way: the codec
re-encodes the declared response exactly, the `content` text block is that
exact document, and `structuredContent.document` carries each number as its
original token string (`"amount": "9007199254740993"`).

## Optional presence semantics

Omission is a distinct wire state from `false`, `0`, `""` and `null`
everywhere.

**API CLI.** Presence is decided by Cobra's `Changed`, never by the value.
`--verbose=false` sets the parameter to `false`; omitting `--verbose` leaves it
absent. `--label ""` sends `label=`; omitting `--label` sends nothing. In the
observed run, `--verbose=false --limit 0 --label ""` produced the query
`label=&limit=0&verbose=false`, while the same command with all three omitted
produced the empty query string. A missing *required* flag is a usage failure
(exit 2) and no request is sent.

**MCP.** Presence is decided by `Object.hasOwn(args, "<property>")` on the
argument object. An omitted optional property is never turned into a zero,
`false`, an empty string or `null`, and an explicit `null` is accepted only
where the source schema admits one. In the observed run,
`read_widget({id:"w-1"})` produced `echo.query === ""` and `echo.trace ===
null` upstream — nothing was defaulted.

## Generated CLI runtime

### Root and per-command flags

| Flag | Scope | Meaning |
| --- | --- | --- |
| `--server-url <URL>` | persistent (root) | Absolute base URL override, including its path prefix. Empty selects the source server. |
| `--version`, `-v` | root | Exact `version` from the target configuration. |
| `--help`, `-h` | every command | Complete offline; constructs no credentials and contacts no network. |
| `--<mapped flag>` | per command | One per source parameter. Boolean parameters are boolean flags (`--flag=false` is a real value); string, integer and number parameters are text flags. |
| `--body-file <PATH>` | commands with `body.kind = "json_document"` | One finite JSON document. `-` reads standard input. There is **no** inline-value form. |
| `--confirm <TOKEN>` | commands with `confirmation.kind = "required"` | Must match the mapped token exactly. `-` reads the token from the first line of standard input. |

The generated binary has no output-format flag: `output.format` is fixed at
generation time. Standard input serves exactly one purpose per invocation —
`--confirm -` together with `--body-file -` is refused as a usage failure.
Confirmation is checked *before* any flag is read, so an unconfirmed invocation
never consumes the caller's standard input.

### Exit codes

| Code | Category | Meaning |
| --- | --- | --- |
| 0 | success | The request succeeded, or the command needed no request (help, a bare group or root invocation). |
| 1 | runtime failure | Transport, deadline, cancellation, codec or output failure. No machine output. |
| 2 | usage failure | The invocation was refused before anything was sent: a missing required flag, an unparsable exact token, an oversized or unreadable body, a failed confirmation, an unknown command or flag. |
| 3 | documented API failure | The API returned a response the contract declares as a failure. Its exact document is already on standard output; the classification line is on standard error. |

An unrecognized command or subcommand is exit 2, never a silently successful
help dump; a *bare* group or root invocation is a help request and exits 0.

### Streams, deadlines and cancellation

Machine output — exactly one JSON document per command, followed by a newline —
goes to standard output. Every diagnostic goes to standard error. Each request
is bounded by `request_deadline_ms` and observes a context cancelled by SIGINT
and SIGTERM; an exceeded deadline and a cancellation are each named explicitly
in the exit-1 message. Nothing is retried and no pagination is traversed.
Failure text keeps the SDK's own secret-free wording.

## Generated MCP server runtime

### Transport and streams

Stdio only. Standard output carries MCP protocol traffic exclusively; the
entry point calls `keepStandardOutputForProtocol()`, which redirects
`console.log`, `.info`, `.debug`, `.dir` and `.table` to standard error, so a
dependency that writes to the console cannot corrupt the session. Every
diagnostic and every log line goes to standard error.

### Tool input schema projection

Each tool input schema is JSON Schema 2020-12 with
`"additionalProperties": false` and an explicit `required` list, so an
undeclared argument is refused rather than silently dropped. The projection
walks exactly the native representation the generated TypeScript SDK already
admitted — never a re-read source schema — so a tool surface can never claim
something the codecs do not implement. Two rules govern it:

1. **The projected schema never admits a JSON number anywhere.** See
   [Numeric representation](#numeric-representation).
2. **A projected position never admits one JSON value with two meanings.**
   String-or-number unions, arbitrary JSON (`Any`), intersections and recursion
   are refused during generation rather than coerced.

Objects, arrays, booleans, `null`, closed string enumerations (kept as an
`anyOf` of `{"type":"string","const":…}`) and unions dispatched by JSON value
kind all project. Projection is bounded at 4096 schema nodes. An object whose
source schema also admits undeclared properties is explicitly closed, and the
projected schema *says so* in its own `description`, e.g. `Only the properties
declared here are exposed. The source schema also admits undeclared
properties, which this tool surface does not carry: …`.

### Result envelope

A successful or declared-failure call returns `content: [{type:"text", text}]`
plus `structuredContent`:

| Field | Meaning |
| --- | --- |
| `ok` | `true` for a success response, `false` for a declared failure. |
| `status` | The actual HTTP status. |
| `mediaType` | The matched media type, or `null` when the response carries no document. |
| `document` | Present only when the response declares a document: the exact document with every number as its token string. |

The `text` block is the API's exact JSON document, or — when HTTP itself
suppresses the body — `<METHOD> <path> (<operationId>) returned HTTP <status>
with no declared response document.` A declared failure additionally sets
`isError: true`.

An *execution* failure returns a sanitized refusal envelope instead:

```json
{ "ok": false, "failure": "<category>" }
```

with `isError: true` and a one-line `text` explanation. Categories:

| Category | Cause |
| --- | --- |
| `input-too-large` | Encoded arguments exceed `max_input_bytes`, measured before any codec runs. |
| `invalid-argument` | The argument phase rejected a value: it does not fit the projected surface, or the generated codec rejected a source assertion the projection deliberately does not carry. Nothing was sent. |
| `cancelled` | The request's own signal aborted the call. |
| `deadline` | The call exceeded `call_deadline_ms`. |
| `upstream-failure` | The SDK failed in a way the contract does not declare. Only the SDK's own enumerated failure `kind` is echoed, matched against `^[a-z][a-z-]{0,39}$`. |
| `undeclared-response` | The API returned a status the contract does not declare. |
| `result-encoding` | The response document could not be re-encoded exactly. |
| `result-too-large` | The exact result document exceeds `max_result_bytes`. |

No caught exception message, cause chain, stack, header, URL or credential
value is ever echoed.

**Invalid argument versus upstream failure** is a deliberate two-phase split,
not a classification of a caught value. `bindArguments` runs first and sends
nothing; only `call` reaches the API. A rejected argument is therefore always
the client's to correct and can never be reported as a condition at the API.
Note that the official server SDK validates arguments against the published
schema *before* the handler runs, so a violation of the *projected* schema (a
number token that fails the pattern, an undeclared property) comes back as the
SDK's own `Input validation error: …` with no `structuredContent`; a violation
of a *source* assertion the projection does not carry (for example
integrality) reaches the handler and comes back as
`{"ok":false,"failure":"invalid-argument"}`.

### Cancellation

The request's own `AbortSignal` and a timer for `call_deadline_ms` are combined
into one signal handed to the SDK. An already-aborted incoming signal aborts
immediately. Both the timer and the listener are cleaned up in a `finally`
block. Cancellation was observed reaching a running upstream call in the
acceptance session.

### Environment

The only environment variable the *application* reads is `server_url_env`, and
only as an absolute base URL override; unset or empty means the contract's own
server selection applies. Credentials are read exclusively by the generated
SDK's own factory when the process starts. No tool input schema mentions a
credential, no tool argument can supply one, and an incoming MCP client's own
credentials are never forwarded to the API.

A client registration therefore looks like:

```json
{
  "mcpServers": {
    "widget-control-plane": {
      "command": "node",
      "args": ["/absolute/path/to/dist/server/main.js"],
      "env": {
        "WIDGET_API_KEY": "<value>",
        "WIDGET_SERVER_URL": "<absolute base URL>"
      }
    }
  }
}
```

The generated `README.md` contains this block with the configured variable
names filled in and placeholder values only — a generated guide never contains
a credential.

## Application surface manifests

Both targets emit `application-surface.json` at the output root: a
deterministic record of the admitted surface for review and compatibility
comparison. It carries **no build time and no generator path**.

**Document path semantics.** Every contract document the manifest names is
recorded as a path *relative to the entry document's directory* — the entry
itself as its bare filename, a document in a subdirectory as
`nested/identifier.json`. A document **outside** that directory tree has no
relative form, so it is **refused during generation** with
`application-document-outside-entry-tree` rather than recorded as an absolute
`file:///…` URI. This is what makes the manifest a comparable review artifact:
two generations from identical inputs on two different machines produce
byte-identical manifests, which an absolute generator path would break. The
refusal is located at both the offending contract source and the mapping
pointer that reached it, and it fires for both targets through one shared
implementation (`suspect_codegen::application::DocumentScope`). The common
cause is a shared `$ref` library above the entry document — e.g. a Parameter
Object in `../shared/params.json`; the fix is to move it inside the entry tree
or to expose an operation that does not reach it.

`suspect.application.cli.surface.v1` records `binary`, `module`, `version`,
`outputFormat`, `runtime`, `credentialEnv`, `credentialEnvFactory`,
`cobraVersion` and one entry per command with its `path`, `name`, `summary`,
`operationId`, `httpMethod`, `httpPath`, `nativeMethod`, `source`,
`confirmation`, `confirmationFlag`, `flags` (each with `flag`, `parameter`,
`in`, `required`, `representation`, `model`, `schema`), `body` and `responses`.

`suspect.application.mcp.surface.v1` records `transport` (`"stdio"`),
`numberRepresentation` (`"exact_json_number_token"`), `server`, `package`,
`sdk`, `runtime` (including `logPolicy` and `logDestination: "stderr"`),
`credentialEnv`, `credentialEnvBindings`, `serverUrlEnv`, the explicit
`resources: false`, `prompts: false`, `retries: false`,
`paginationTraversal: false`, and one entry per tool with its `name`, `title`,
`description`, `operationId`, `httpMethod`, `httpPath`, `nativeFunction`,
`source`, `annotations`, `input` (each with `property`, `parameter`, `in`,
`required`, `representation`, `mediaType`, `member`, `model`, `codec`,
`schema`) and `responses` (with `successStatuses` / `failureStatuses`).

Input/flag `representation` values:

| CLI | MCP |
| --- | --- |
| `string`, `boolean`, `exact_integer`, `exact_number`; body `exact_json_document` | `string`, `boolean`, `null`, `exact_number_token`, `array`, `object`, `union`; body `exact_json_value` |

Response `representation` is `no_content` or `exact_json_document` for both.

Generation is byte-identical for the same inputs — asserted by
`emission_is_byte_identical_for_the_same_inputs` in both test suites.

## Ownership, `--check` and drift

Emission and the ownership classification both complete **before any write**,
so a planning or ownership refusal leaves the output root exactly as it was.

| Status | Meaning | Exit |
| --- | --- | --- |
| `generated` | The root was written. | 0 |
| `current` | Every owned artifact already matches. | 0 |
| `drift` | `--check` only: the root differs from the desired set. | 1 |
| `conflict` | An owned artifact was edited after its recorded generation, or a path is owned by a different owner. Nothing is written. | 1 |
| `failed` | Input, mapping, planning or package refusals. Nothing is written. | 1 |

```console
$ suspect codegen-cli … --out $WORK/widgetctl --check
codegen-cli current: 3 mapped SDK operations, 42 planned artifacts (suspect.application.cli.v1)
# exit 0

$ suspect codegen-cli … --target-config $WORK/cli-target-drift.json --out $WORK/widgetctl --check
codegen-cli drift: 3 mapped SDK operations, 42 planned artifacts (suspect.application.cli.v1)
# exit 1, nothing written
```

A hand edit to an owned artifact is preserved, not overwritten:

```console
$ printf '// hand edit\n' >> $WORK/widgetctl/cmd/widgetctl/main.go
$ suspect codegen-cli … --out $WORK/widgetctl
…/cmd/widgetctl/main.go:1:1 [application-cli-artifact-conflict] owned output was edited after its recorded generation; preserving user changes (; mapping )
codegen-cli conflict: 3 mapped SDK operations, 42 planned artifacts (suspect.application.cli.v1)
# exit 1; the edit is still there afterwards
```

The two owners never cross:

```console
$ suspect codegen-mcp … --out $WORK/widgetctl
…/widgetctl/application-surface.json:1:1 [application-mcp-artifact-conflict] owned by `suspect-cli-app:lifecycle-v1`; owner `suspect-mcp-app:lifecycle-v1` cannot overwrite or adopt it (; mapping )
codegen-mcp conflict: 3 mapped SDK operations, 34 planned artifacts (suspect.application.mcp.v1)
# exit 1
```

Unowned files in the root are left alone: after `npm ci` and `npm run build`
created `node_modules/` and `dist/`, `codegen-mcp --check` still reported
`current`.

## Pinned inputs

`--pins` replaces the positional spec with an immutable pin manifest, and
generation then reads only the verified cache — no entry file and no network:

```console
$ suspect acquire $WORK/pins.json --cache-dir $WORK/cache --format json
{ …, "networkRequests": 0, "status": "acquired" }

$ rm $WORK/widgets.openapi.json      # the entry document is gone
$ suspect codegen-cli --pins $WORK/pins.json --cache-dir $WORK/cache \
    --mapping $WORK/cli-mapping.json --target-config $WORK/cli-target.json \
    --out $WORK/pinned-out
codegen-cli generated: 3 mapped SDK operations, 42 planned artifacts (suspect.application.cli.v1)
# exit 0
```

`--cache-dir` and `--insecure-test-origin` both require `--pins`.
`--insecure-test-origin` admits a numeric-loopback origin from an explicitly
acquired test manifest and exists for acceptance fixtures only. Acquisition
failures keep the acquisition layer's own diagnostic codes and point at the
manifest and its `/resources/<index>` pointer.

## Refusal codes

### Shared application layer (`crates/suspect-codegen/src/application.rs`)

Both commands emit these verbatim: they carry **no target prefix**, because
both targets select operations and relativize manifest documents through this
one shared implementation.

| Code | Meaning | Typical fix |
| --- | --- | --- |
| `application-operation-missing` | The selector identifies zero outgoing operations. | Check the exact `operationId` spelling, or use the exact `METHOD /path` template including path-parameter braces. |
| `application-operation-ambiguous` | The selector identifies more than one. | Two declarations reuse one `operationId`; select by `METHOD /path` instead, or fix the source. |
| `application-document-outside-entry-tree` | A document the surface manifest would name lies outside the entry document's directory tree, so it has no machine-independent relative form. | Move the referenced document inside the entry tree, or expose an operation that does not reach it. Usually a shared `$ref` library above the entry, such as a Parameter Object in `../shared/params.json`. |
| `application-path-item-unplanned` | The selector resolved, but the canonical SDK plan does not carry that operation at the resolved source. In practice: a Path Item reached through a `$ref`, which `codegen-cli` cannot bind. | Declare the operation in the entry document's own `paths`. See [Known limitations](#known-limitations) — `codegen-mcp` does support referenced Path Items. |

Selector resolution runs before any binding, so the two selector codes
short-circuit the rest of the mapping. The document-scope check runs after
binding, once the exact set of manifest-visible documents is known, and is
located at both the offending contract source and the mapping pointer that
reached it.

### `codegen-cli`

| Code | Meaning | Typical fix |
| --- | --- | --- |
| `cli-mapping-version` | `format` is not `suspect.application.cli.v1`. | Set the exact profile string. |
| `cli-package-identity` | `go_version` / `go_toolchain` is not an exact directive, or the SDK package could not be emitted under `module_path`. | Use `X.Y[.Z]` and `goX.Y[.Z]`; check `module_path`. |
| `cli-binary-name` | `binary_name` violates the token rule. | 1..=64 lower-case letters, digits and single interior dashes, starting with a letter. |
| `cli-runtime-bounds` | `request_deadline_ms` or `max_input_bytes` is out of range. | 1..=3600000 and 1..=8388608. |
| `cli-command-name` | A `path` has 0 or >4 segments, or a segment violates the token rule. | Shorten the path; use lower-case tokens. |
| `cli-command-reserved` | A segment is `help`, `completion`, `__complete` or `__completeNoDesc`. | Rename the command. |
| `cli-command-collision` | Two commands claim one path, or a leaf path also prefixes another command. | Rename, or move the operation under a deeper path. |
| `cli-flag-name` | A `flag` violates the token rule. | Use a lower-case token ≤32 characters. |
| `cli-flag-reserved` | A flag is `help`, `body`, `body-file`, `confirm`, `server-url` or `version`. | Rename the flag. |
| `cli-flag-collision` | Two parameters of one command map to one flag. | Give each its own flag. |
| `cli-parameter-unmapped` | A source parameter has no mapped flag. | Add an entry keyed by the exact source parameter name. |
| `cli-parameter-unknown` | A `parameters` key is not a parameter of the selected operation. | Remove it, or fix the key spelling. |
| `cli-parameter-unrepresentable` | The parameter's native carrier is not a string, boolean, integer or number scalar (an object, array, record, union or cyclic alias). | Expose an operation with scalar parameters; a structured parameter cannot be carried by one flag exactly. |
| `cli-body-policy` | `body.kind` disagrees with whether the operation declares a request body. | Use `json_document` when it does, `none` when it does not. |
| `cli-unsupported-media` | A request body or a response does not have exactly one concrete JSON media with a generated codec, and is not an HTTP-forbidden-body status. | Expose a JSON operation; see [Known limitations](#known-limitations). |

### `codegen-mcp`

| Code | Meaning | Typical fix |
| --- | --- | --- |
| `mcp-mapping-version` | `format` is not `suspect.application.mcp.v1`. | Set the exact profile string. |
| `mcp-package-identity` | The SDK package could not be emitted under `package_name`. | Use a valid npm package name. |
| `mcp-bin-name` | `bin_name` violates the token rule. | As for `binary_name`. |
| `mcp-server-identity` | `server_name` is empty, >128 bytes or has control characters; or `server_url_env` is not a portable variable name. | Fix the name. |
| `mcp-runtime-bounds` | `call_deadline_ms`, `max_input_bytes` or `max_result_bytes` is out of range. | 1..=3600000 and 1..=8388608. |
| `mcp-toolchain-pin` | A version pin is not the registry-verified value, `node_minimum_major` is below the floor the application actually requires (22), or `node_version` / `npm_version` is not exact `X.Y.Z`. | Match the constants in `src/mcp.rs`. |
| `mcp-credentials-unconfigured` | The selected operations require credentials but `credential_env` is `null`. | Configure `credential_env`; a tool input can never carry a credential. |
| `mcp-tool-name` | A tool name violates the token rule. | 1..=64 characters, lower-case letters, digits, underscores, single interior dashes, starting with a letter. |
| `mcp-tool-reserved` | The name is an MCP protocol method family (see the reserved list). | Rename the tool. |
| `mcp-tool-collision` | Two tools claim one name. | Rename one. |
| `mcp-property-name` | An input property violates the token rule. | Letters, digits and underscores, starting with a lower-case letter. |
| `mcp-property-collision` | Two inputs of one tool claim one property, including the body property. | Rename one. |
| `mcp-parameter-unmapped` | A source parameter has no mapped property. | Add an entry keyed by the exact source parameter name. |
| `mcp-parameter-unknown` | A `parameters` key is not a parameter of the selected operation. | Remove or fix it. |
| `mcp-unsupported-schema` | No generated model, or no exact tool input projection: arbitrary JSON, an uninhabited schema, an intersection, recursion, a string-or-number union, two different array or object representations in one union, a model outside the SDK's closure, or a projection over 4096 nodes. | Expose a single concrete schema for that position. |
| `mcp-body-policy` | `input.body.kind` disagrees with whether the operation declares a request body. | Use `json_value` when it does, `none` when it does not. |
| `mcp-unsupported-media` | A request body or a body-carrying response does not have exactly one concrete JSON media with a generated codec. | Expose a JSON operation. |

### Command layer (`crates/suspect-cli/src/commands/application_cmd.rs`)

Prefixed per target — `application-cli-…` and `application-mcp-…` — so one
report never mixes two targets' refusals under one code.

| Code suffix | Meaning |
| --- | --- |
| `mapping-input` | The mapping file could not be read, or is not valid JSON for the closed profile. Reports the file, line and column. |
| `target-input` | The target configuration file could not be read or parsed. |
| `input` | Neither a spec nor `--pins` was usable, or the input document could not be normalized, opened or lifted into a contract. |
| `artifacts` | The output root could not be classified or written. |
| `artifact-conflict` | One owned path conflicts: edited after its recorded generation, or owned by a different owner. One diagnostic per conflicting path. |

### Pass-through SDK findings

Findings from the canonical SDK planners keep their own codes and are reported
at mapping pointer `/commands` or `/tools`. Two observed examples:

```
[sdk-credential-env-unbound] credential_env.schemes["apiKey"] must identify exactly one used source Security Scheme declaration; found 0 (; mapping /commands)
[http-parameter-shape-unsupported] parameter style requires a scalar, a scalar array, or a flat scalar object; nullable/untyped/composed representations need an explicit wire policy (…; mapping /tools)
```

## Supported scope

| Area | API CLI | MCP server |
| --- | --- | --- |
| Transport | HTTPS/HTTP via the generated SDK | stdio only |
| Operation selection | Explicit mapping allowlist | Explicit mapping allowlist |
| Parameter locations | path, query, querystring, header, cookie — scalar carriers only | path, query, querystring, header, cookie — any exactly projectable schema |
| Request body | Exactly one concrete JSON media, by `--body-file` or stdin | Exactly one concrete JSON media, as one projected JSON value |
| Responses | Exactly one concrete JSON media, or an HTTP-forbidden-body status | Same |
| Exact numbers | Exact tokenizers and codecs | Exact token strings both ways |
| Optional presence | `Changed`-based | `Object.hasOwn`-based |
| Credentials | Environment variables read by the SDK factory | Same; never a tool input |
| Base URL override | `--server-url` | `server_url_env` |
| Destructive guard | Mapping `confirmation` + `--confirm` | Advisory `destructive` hint only |
| Bounds | Request deadline, max input bytes | Call deadline, max input bytes, max result bytes |
| Cancellation | SIGINT / SIGTERM | The request's own `AbortSignal` |
| Discovery | Cobra help and shell completion, offline | Deterministic `tools/list` |
| Review artifact | `application-surface.json` | `application-surface.json` |
| Pinned dependency | Cobra 1.10.2 | `@modelcontextprotocol/server` 2.0.0, TypeScript 5.9.3, `@types/node` 26.6.2 |

## Deferred scope

Explicitly **not** generated by this bounded profile:

- **Remote MCP transport and authentication.** Stdio only. No HTTP or SSE
  transport, no OAuth, and no forwarding of an incoming client's credentials.
- **MCP resources and prompts.** Tools only. The surface manifest records
  `resources: false` and `prompts: false`.
- **Automatic all-operation exposure.** There is no "expose everything" mode;
  a mapping must name each operation.
- **Automatic retries.** No request is ever retried by either application.
- **Pagination traversal.** No application follows a next-page link or
  aggregates pages.
- **Arbitrary filesystem paths as inputs.** The CLI's only path input is
  `--body-file` (with `-` for stdin); no MCP tool argument is ever a path.
- **Other application languages.** Go for the CLI and TypeScript for the MCP
  server. No other language, and no extension of the SDK language enum.
- **Generalized shared-SDK target graphs.** Each output root owns its own
  embedded SDK copy; multiple applications over one shared generated SDK is
  deferred, as is any change to existing SDK session configuration or
  serialization.
- **Inline body values, multipart and streaming bodies, non-JSON media.**

## Known limitations

These are real, current bounds — recorded so a reviewer does not have to
rediscover them.

**The media profile is exactly-one-JSON-media-or-none, and the no-media case is
conservative.** A response is admitted only if HTTP itself forbids its body
(204, 304 and the like → `no_content`) or it declares exactly one concrete JSON
media with a generated codec. A response declared with *no* `content` at all on
a status that permits a body is **refused**, not treated as empty — both
targets behave identically here. Observed on a `200` whose only member is
`description`:

```console
$ suspect codegen-cli $WORK/nomedia.openapi.json …
…/nomedia.openapi.json:12:18 [cli-unsupported-media] response "200" needs either no content or exactly one concrete JSON media with a generated codec (/paths/~1ping/get/responses/200; mapping /commands/0)

$ suspect codegen-mcp $WORK/nomedia.openapi.json …
…/nomedia.openapi.json:12:18 [mcp-unsupported-media] response "200" needs either no content or exactly one concrete JSON media with a generated codec (/paths/~1ping/get/responses/200; mapping /tools/0)
```

The conservative choice is deliberate — guessing "probably empty" would make
the application claim a representation the codecs do not implement — but it
does mean an under-specified contract must be tightened before it can be
exposed.

**Numeric `const` / `enum` widen to unconstrained token strings in the MCP
projection.** Because numeric positions are carried as strings, a numeric
literal cannot keep its `const`: the projected string carries only the generic
number pattern, with the declared value named in prose. A numeric enumeration
becomes an `anyOf` of *N* identically-shaped unconstrained token strings.
Observed for a body declaring `"version": {"const": 7}` and
`"tier": {"enum": [1,2,3]}`:

```json
"version": {
  "type": "string",
  "pattern": "^-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?$",
  "description": "… The source declares exactly the value 7; any spelling of that same number is accepted."
},
"tier": { "anyOf": [ {…"the value 1"…}, {…"the value 2"…}, {…"the value 3"…} ] }
```

The practical consequence: a client's own schema validation will **not** reject
`"version": "8"`. The generated codec enforces the original constraint at the
tool boundary, so the call is refused — as
`{"ok":false,"failure":"invalid-argument"}`, attributed to that property —
rather than reaching the API. The constraint is enforced; it is just enforced
one layer later than a client-side validator would, and it is not
self-describing in the published schema beyond the prose. String enumerations
are unaffected: they keep their exact `const` members.

**`codegen-cli` does not support a Path Item reached through a `$ref`; the two
targets differ here.** With
`"/things/{id}": {"$ref": "nested/paths.json#/things"}` in the entry document,
the contract resolves the selector to the operation's own declaration in the
referenced document, but the canonical Go SDK plan does not record it at that
source, so `codegen-cli` cannot bind it and refuses at generation with
`application-path-item-unplanned`. Run against the committed fixture
`crates/suspect-codegen/tests/fixtures/application-scope/`, whose
`api/referenced-path.json` is exactly that entry (`$FIXTURE` below is that
directory):

```console
$ suspect codegen-cli $FIXTURE/api/referenced-path.json \
    --mapping $WORK/cli-mapping.json --target-config $WORK/cli-target-anon.json \
    --out $WORK/out
file://$FIXTURE/api/nested/paths.json:4:12 [application-path-item-unplanned] selector "getThingByReference" resolves to the operation declared at file://$FIXTURE/api/nested/paths.json /things/get, which the canonical SDK plan does not carry at that source; a Path Item reached through a `$ref` is outside this application target's bounded profile - declare the operation in the entry document's own `paths` instead (/things/get; mapping /commands/0/selector)
codegen-cli failed: 0 mapped SDK operations, 0 planned artifacts (suspect.application.cli.v1)
```

The mapping selects `getThingByReference` onto the command path
`["things", "get"]` with its one `id` parameter — the same shape the
`api_cli` test suite uses for this fixture.

`codegen-mcp` **does** support it: its canonical SDK plan records the operation
at the referenced source, so the tool binds normally and the referenced
document is recorded relative to the entry directory like any other (subject
to the document-scope rule above). This asymmetry is a property of the two
canonical SDK planners, not of the mapping vocabulary; the fix for the CLI is
to declare the operation in the entry document's own `paths`.

**Not every `type` + `enum` combination projects.** An integer parameter
declared as `{"type": "integer", "enum": [1,2,3]}` becomes a native
*intersection*, which has no unambiguous tool projection and is refused with
`mcp-unsupported-schema`. A parameter declared as a bare `{"enum": [1,2,3]}` is
refused even earlier, by the SDK's own parameter-shape policy
(`http-parameter-shape-unsupported`).

**Objects open in the source are closed in the projection.** An OpenAPI 3.1
object without `additionalProperties: false` admits arbitrary JSON, whose
numbers could not stay exact. The projection closes the object and records the
reason in the projected `description`, so the narrowing is visible in the
published schema — but a client cannot send an undeclared member that the API
itself would accept.

**Advisory annotations are advisory.** `read_only`, `destructive`,
`idempotent` and `open_world` are declared per tool and projected as hints;
nothing in the emitted server consults them. Unlike the CLI's `confirmation`
policy, they impose no guard. A destructive MCP tool is gated only by whatever
the client does with the hint.

**Numeric-token measurement uses ordinary JSON on the way in.** The
`max_input_bytes` check measures `JSON.stringify(args)` before any codec runs.
Those are the client's own parsed values, which never contain an exact-number
wrapper, so no token can be lost by that measurement — but the byte count is of
the re-serialized arguments, not of the client's original bytes.

**`--version` takes an exact SemVer, not a requirement.** Both targets reject
version *ranges*; `version` is the literal string reported by the application.

## Verification and CI

`.github/workflows/application-targets.yml` runs four explicit jobs. It
deliberately runs only the portable native application acceptance suites, never
all corpus-dependent ignored tests.

| Job | Command | Needs |
| --- | --- | --- |
| `planning` | `cargo test -p suspect-codegen --test application --test api_cli --test mcp` | Rust only |
| `native-go-cli` | `cargo test -p suspect-codegen --test api_cli -- --include-ignored` | Go stable, python3 |
| `native-mcp-server` | `cargo test -p suspect-codegen --test mcp -- --include-ignored` | Node 22 and Node 26 (matrix), npm registry access, python3 |
| `commands` | `cargo test -p suspect-cli --test application_codegen` | Rust only |

The two native jobs build real artifacts. `native-go-cli` runs
`go build ./...`, `go test ./...` and the built binary against the python3
loopback fixture under `-mod=readonly`, asserting that the emitted `go.mod` and
`go.sum` survive byte-for-byte. `native-mcp-server` runs `npm ci` against the
emitted lockfile, `tsc`, and the built server over stdio driven by the official
`@modelcontextprotocol/client@2.0.0`. **`native-mcp-server` requires npm
registry access** and will fail on an isolated runner; that is an environmental
dependency, not a defect in the generated package.

`native-mcp-server` is a matrix whose low end is the emitted application's
runtime floor, **Node 22**, which is also the floor the emitted
`engines.node` declares. The application depends on the official MCP SDK
(whose own minimum is 20) *and* embeds the canonical Suspect TypeScript SDK as
compiled-in source (whose `engines.node` is `>=22`), so the real floor is the
stricter of the two. `mcp::NODE_MINIMUM_MAJOR` is derived from
`typescript::package::NODE_MINIMUM_MAJOR` rather than restated, so the two can
never drift into an application declaring a floor it cannot honour. CI
therefore builds and runs at the declared floor (22) and at the current
release (26). Both python steps pin `3.12.12` exactly: the loopback fixtures
are the exactness oracle for both native suites.

Observed locally on darwin/arm64 with go1.27.1, Node v26.8.1, npm 11.19.0 and
Python 3.14.7:

```
tests/api_cli.rs      20 passed (including native_cli_builds_and_drives_a_loopback_api)
tests/application.rs   7 passed
tests/mcp.rs          24 passed (including native_mcp_server_builds_and_serves_the_official_client)
suspect-cli tests/application_codegen.rs  9 passed
```
