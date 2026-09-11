# Python HTTP protocol adapter

Python HTTP generation now consumes the shared
[`http_protocol::ProtocolPlan`](SDK-HTTP-PROTOCOL.md). There is one admission,
native model/codec plan, package, and runtime. The adapter does not project the
operation back through `http_contract::plan`.

## Planning and configuration

`python_http::plan_http` retains its `Contract`, selected `SourceId`s and
`HttpConfig` inputs. `HttpConfig` now also exposes:

- `capabilities`: the explicit adapter policy, including optional versioned
  compatibility profiles.
- `max_part_bytes`, `max_stream_item_bytes`, and `max_parts`.
- The existing codec, request-byte and response-byte policies.

`python_http::capabilities()` identifies `python-http-protocol-v1` and enumerates
the implemented native families. Requested capabilities outside this adapter's
implementation are located refusals. `HttpPlan::protocol()` exposes the admitted
shared plan; `groups()` exposes native header/form/part carriers. Every JSON/text
codec input is an actual source schema root. Binary schemas and mixed multipart
aggregates are not passed to JSON validation with placeholder values.

The Python lowering retains method names and source-role model names. Rich
responses expose their `ResponseStatus` selector and optional schema rather than
inventing a concrete status or JSON schema for bytes. `exact_status()` is available
for exact declarations.

Canonical backend generation and Python compatibility capture both obtain their
HTTP configuration through `backend::python_options(GenerationOptions)`. Explicit
compatibility profiles therefore use the adapter's capability fence and default
resource policy consistently. `NativeSnapshot.generation` records those choices;
they remain separate from the four-field package `TargetConfig`. The canonical
profile test checks standard-mode refusal and explicit-profile admission at both
the generation and compatibility seams. The real binary download witness below
provides the independent native evidence for `LegacyBinaryStringV1`.
The canonical-options test passes in
`target/sdk-python-protocol-canonical-options-02.log`.

## Installed public surface

The package still exports `Client`, `AsyncClient`, `models`, `codecs`, `operations`,
the absence/numeric types, and classified exceptions. Additional root exports are:

```text
BasicAuth, Authorization, CredentialRequest, OAuthFlow,
AuthValue, Credential, CredentialProvider, Part, Link,
SyncStream, AsyncStream
```

Status classes and success/API-error aliases remain in `<import>.operations`.
Former `_client` imports remain object-identical compatibility aliases. The
`__module__` identity of concrete result classes is the public operations module.
Form bodies, typed part wrappers and declared-header groups also live there.

An operation's synchronous and asynchronous return aliases are identical for
buffered representations. Stream-bearing responses have separate sync/async
classes and aliases so mypy can distinguish `Iterator[T]` from `AsyncIterator[T]`.
Actual status determines return versus API exception, including when `default`
matches a 2xx status. Exact statuses retain `Literal` annotations; ranges and
default retain the actual `int` status. Missing `responses` does not invent a
success: every response remains an undeclared-status SDK error.

Results expose native `data`, the raw `headers` tuple, actual `content_type`,
`links`, and a generated `typed_headers` carrier when headers are declared.
Declared API exceptions expose the corresponding values. Normal HTTP exception
formatting excludes credentials, captures, headers and causes.

## Credentials and servers

### Optional environment defaults

`HttpConfig.credential_env` accepts the shared versioned
[`CredentialEnv` policy](SDK-CREDENTIAL-ENV.md). Python binds it after protocol
admission and exposes the immutable result through `HttpPlan::credential_env()`.
Only configured packages add the private `_credential_env.py` helper and explicit
public constructor overrides. Unconfigured package bytes and constructors retain
their previous behavior.

With `{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}`, the
source-backed OpenRouter package supports:

```python
from openrouter_sdk import Client, AsyncClient

with Client() as client:
    response = client.get_current_key()

async def current_key() -> None:
    async with AsyncClient() as client:
        response = await client.get_current_key()
        print(response.status)
```

The private omitted-auth sentinel distinguishes omission from explicit `auth=None`
or `auth={}`. Mapped variables are read once at construction. An explicit auth
argument wins as a whole; empty/null/UNSET values and missing members are never
filled from the environment. Missing, empty or inaccessible environment variables
remain unavailable. Existing OR/AND/anonymous selection and explicit
`auth_alternative` semantics run at operation time; missing credentials raise a
secret-free `SdkError` before HTTP. Anonymous operations remain usable.

The source named `apiKey` is HTTP bearer; its name does not imply API-key header
attachment. Only variable names are emitted. Basic/OAuth/OIDC mappings are rejected
by the shared binder. No generation/import/per-request variable lookup occurs.
`get_credits()` remains an explicit management-key operation; the SDK does not infer
management permissions. Source-default HTTPS URLs apply without a server override.
Controlled tests inject `httpx.MockTransport`; the live examples above are ordinary
application calls.

Both native Python tiers pass the bounded `python_credential_env` witnesses:
creation snapshots, explicit argument precedence, mixed security, source-default
servers, installed-package typing and Sphinx. The real source witnesses use
`getCurrentKey` and `getCredits`. Completion and no-policy byte parity are recorded
in `target/sdk-python-credential-env-completion-20260911-01/COMPLETION.md`.

### Explicit credentials

Clients accept explicit `auth: Mapping[str, Credential]`. Values are interpreted
by the selected source scheme:

- Bearer: token string, with the declared HTTP scheme matched case-insensitively.
- Basic: `BasicAuth(username=..., password=..., encoding="utf-8")`; Latin-1 is an
  explicit alternative. Controls and username colons are refused.
- API key: string attached at the declared header/query/cookie name.
- OAuth/OIDC: `Authorization("<scheme> <credentials>")`, or a caller callback
  returning that explicit value. The SDK does not infer bearer from OAuth.

`CredentialRequest` carries operation/use/definition sources, roles versus scopes,
OAuth flow URLs/scope metadata, and OIDC/authorization-server metadata URLs.
`effective_server_url` supplies the resolved HTTP server base for the retained
literal OAuth/OIDC endpoint URLs, including caller overrides. It is independent
of the scheme's physical source and logical resource address.
Callbacks may be async on `AsyncClient` and execute on the caller task. There is
no SDK acquisition, discovery, refresh, retry, or local business-permission check.

Security alternatives are OR; requirements within an alternative are AND. The
first fully available alternative is selected. An anonymous alternative attaches
no credentials. `auth_alternative` chooses a source alternative explicitly.
Disabled/undeclared security is distinct metadata and attaches nothing. Conflicting
conjunctive or expanded parameter/credential attachments are refused.

`server` chooses a declared server index or OAS 3.2 name. `server_variables`
supplies literal overrides with unknown-name and enum checks. Relative URLs use
the URL serving the server's source document. Local-file sources need an explicit
`document_url` or absolute `server_url`. Userinfo, invalid ports, unresolved braces,
and route-changing parameter segments are rejected before transport. Custom OAS
3.2 HTTP method tokens retain their exact case, including through httpx.

## Parameters, status selection and media

The native wire interpreter implements the admitted simple/label/matrix/form,
space/pipe-delimited, deep-object, header and cookie strategies. Content-based
JSON/text parameters and complete OAS 3.2 querystrings use their own descriptors.
Form querystrings are encoded once, rather than being URI-escaped a second time.
Source codecs run before serialization. Flat shape, empty required composites,
reserved delimiters, control characters and header/cookie representation are
separate wire checks. Header text has an explicit ASCII/HTAB wire policy; no
header-specific charset or quoting convention is guessed.

Response dispatch selects **exact status > class range > default**, then
**concrete media > type wildcard > any wildcard**, with declared parameter
specificity breaking ties. A media mismatch cannot fall back across statuses.
Runtime Content-Type is required for declared content and is parsed with duplicate
parameter/quoted-string checks. Text/framing is UTF-8; unsupported charsets are
refused. JSON and structured `+json` use exact codecs or the exact JSON domain.
Text, raw bytes and schema-free JSON retain distinct native carriers.

No declared response content means bounded bytes. HEAD and HTTP content-forbidden
statuses, including 204/205/304, produce `None` without attempting body decoding.
Their declared metadata and typed-header obligations remain available. Required
headers are enforced; arrays/objects/content headers use their own source codecs.
Links remain metadata and perform no invocation or expression evaluation.

## Forms and multipart

Generated keyword-only carriers contain typed JSON/text values and actual bytes.
Repeated fields use lists with per-item codecs and source cardinalities. Typed
part subclasses expose declared headers; `Part` also carries a concrete media
choice, filename and explicit extra headers.

The runtime validates required fields, additional-part policy and structural
cardinalities. Whole-object assertions requiring a fabricated JSON aggregate are
refused by the shared plan. URL-encoded content fields and explicit RFC6570 styles
are distinct strategies. OAS 3.2 repeated style fields use per-item encoding.

Multipart framing uses a finite, collision-checked boundary. Name/filename values
are UTF-8 quoted strings with controls rejected. Part bytes are preserved, including
zero and non-UTF8 octets. Per-part body, part-count and whole-body limits apply.
Named multipart and form responses are decoded through the corresponding typed
carriers; MIME boundary prefixes inside file data are not treated as delimiters.

## Item streams and lifetime

Standard OAS 3.2 `itemSchema` drives `SyncStream[T]` and `AsyncStream[T]`.
Neither the field named `data` nor a vendor extension selects a nested JSON or
sentinel interpretation.

- SSE uses HTML line/comment/field/multiline rules, one leading BOM, UTF-8
  replacement decoding, valid id/retry state, and integer retry values. Unfinished
  events at EOF are discarded. `[DONE]` is ordinary string data.
- JSON lines uses LF, permits a terminating CR, rejects blank/non-JSON records,
  and permits an unterminated final JSON line. UTF-8 and numbers remain exact.
- Pulling an item performs its source codec check. Item and total byte budgets
  remain finite across arbitrarily split transport chunks.
- Request item iterables are source-validated and framed into a bounded in-memory
  request; async iterables stay on the caller task and close on termination/failure.

Use `with response.data` / `async with response.data` around an early-stoppable
loop, or call `close` / `aclose`. Plain Python loop break does not notify an arbitrary
iterator. Exhaustion, errors and cancellation close the response. Client exit also
closes outstanding responses. SDK-owned transports close with the client; injected
transports remain caller-owned. Primary failures and cancellation survive cleanup
errors. The async documentation deadline includes stream consumption.

## Documentation, provenance and compatibility

The native-constructor README/Sphinx guides are preserved. Protocol examples use
Main's `examples::plan_protocol_examples_v2`, or `plan_protocol_examples_v3` for a
resource-aware codec closure, and the shared roles for headers, parts and items.
Each accepted JSON/text value is checked against native encoding and the
codec-decoded source example. Byte recipes require caller bytes; no JSON null or
filename stands in for file contents. New guides cover typed forms/parts and
sync/async item consumption. Sphinx imports actual objects, including inherited
dataclass annotations, for the native reference.

`http-manifest.json` is now `suspect-python-http-v2`. It includes `publicExports`,
`operationsModule`, per-operation `resultModule`, actual class `__module__`, async
types, native groups, protocol metadata and limits. `protocol-plan.json` binds
runtime codecs/classes to the admitted descriptors. Python's native compatibility
capture includes the public module/export identities and richer interfaces;
located literal Link values remain literal data during comparison. The Python
asset inventory supplies the full adapter/shared-protocol source closure to
Main's canonical sorted, deduplicated, length-framed provenance hash. Python's
capture has no supplemental hash loop. The typed validation record reports the
actual version/profile for the selected codec plan.

## Native witnesses and remaining refusals

`tests/python_protocol.rs` uses independent normative style/querystring vectors,
literal wire/byte/status/header/security fixtures, real installed wheels, strict
mypy, warning-as-error Sphinx, and source example verification. Its runtime cases
exercise forms, typed headers, binary round trips, stream chunk boundaries,
backpressure, early close and cancellation/cleanup.

The real-source gate selects `downloadContainerFileContent`, `downloadFileContent`,
`listModelsCount` and `listOauthJwks` without editing the OpenRouter source. The two
downloads require the explicit `LegacyBinaryStringV1` compatibility profile because
their 3.1 schemas use `type: string, format: binary`. The default-policy refusal is
also checked. That profile changes byte interpretation only, not authentication or
streaming behavior.

Positional/nested/streamed multipart, ambiguous null/style conventions, untyped
form/part extras, combined whole-content/item assertions and vendor SSE semantics
remain explicit refusals. The ordinary shared schema/model admission still applies.
These are bounded capability and native-conformance witnesses, not a claim that
every OpenRouter operation or arbitrary schema can be generated.

```sh
cargo test --locked -p suspect-codegen --no-default-features --features http-protocol \
  --test python_protocol -- --include-ignored
```

Passed and in-progress candidate logs are retained under
`target/sdk-python-protocol-*`; final promotion must use the completed native
matrix and affected existing Python regression gates.

### Current native evidence and integration blockers

The 66-operation synthetic package passed installed consumers on **Python 3.11.15
and 3.14.7**: **75 wire requests per interpreter**, native encoding comparisons
for **112 source examples**, strict typing of **20 consumer/guide/README files**,
and Sphinx `-W` with **1,049 planned symbols**. Its nine README Python blocks are
the executable guide files; form, multipart and stream guide functions also run
against MockTransport. Evidence: `target/sdk-python-protocol-native-Uo70E4/`.

The extra real four-operation package passed its complete installed matrix,
including full-package strict mypy, on both interpreters: six native wire requests,
27 source examples and 311 documented symbols per interpreter. Evidence:
`target/sdk-python-protocol-openrouter-V7Jfac/`.

The affected DX suite (M2, adversarial constructors, original five OpenRouter
operations) and the public naming-collision gate passed again after the protocol
migration: `target/sdk-python-protocol-baseline-02.log`. Existing installed HTTP
wire/types and cleanup/primary-failure/cancellation/invalid-port checks also passed:
`target/sdk-python-protocol-existing-http-02.log` and
`target/sdk-python-protocol-cleanup-{311,314}.log`.

**Historical blocker, subsequently resolved:** full-package mypy for the
synthetic package is blocked by a model-owner annotation: the source null-only
`Whole1QuerystringIgnoredName.flag` is emitted as `types.NoneType | Unset`.
Mypy requires a valid `None` type annotation. The current witness is
`target/sdk-python-protocol-native-Uo70E4/python/src/protocol_python_sdk/models.py:1011`.
HTTP runtime files passed that same strict package check; this one failure was in
the model emitter. Main's null-annotation fix and the accepted fresh package
completion resolved it on both tiers:
`target/sdk-python-protocol-completion-01/COMPLETION.md`. The original blocker
report remains unchanged. No generated code was repaired to conceal it.

The original combined `m3_native_docs` attempt could not compile its Go-only
helper after Go's protocol API changes (`body.schema`, `response.schema`, and
`response.status`, lines 806/825/830). This is recorded in
`target/sdk-python-protocol-existing-docs.log`; it is distinct from the passing
Python Sphinx builds above. That historical log retains its original scope;
current all-language integration belongs to Main.

## Scoped/resource validation and physical server bases

The existing planner now admits witnessed v2 and v3 schema execution. Ordinary
base closures keep their v1 programs; scoped closures keep v2; indexed resource
declarations or dynamic references select v3 explicitly. Actual JSON/text roots
remain separate from the candidate-aware schema closure. Binary schemas retain
metadata and byte limits without becoming JSON inputs.

`DocumentRelativeServers`, `SchemaResources` and `DynamicSchemaReferences` are
supported after their independent native gates. Server resolution uses
`ServerPlan::document_base()` from the effective physical retrieval document,
including redirected/external declarations and explicit empty overrides. The
logical `$self`/`$id` address is never an HTTP server base. Caller `document_url`
and `server_url` overrides remain explicit. RFC 3986 resolution preserves encoded
dot/slash spelling and empty path segments through the installed httpx transport.
OAuth/OIDC metadata remains relative to the effective server and triggers no
acquisition.

The physical-base selector is
`python_document_servers::installed_python_physical_document_servers_preserve_redirects_overrides_and_encoded_paths`:
11 requests, six malformed/base controls and seven OAuth/OIDC credential-context
checks on each tier, with strict package/consumer typing and Sphinx. See
`target/sdk-python-document-servers-03.log`.

The frozen v2 receipt is `target/sdk-python-scoped-completion-01/COMPLETION.md`.
The subsequent [v3 native adoption record](SDK-PYTHON-RESOURCES.md) includes exact
official-source, SDK, native-guide, mypy and Sphinx selectors and asset inventory.
