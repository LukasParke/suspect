# Ruby runtime and protocol guide

`__NAMESPACE__::Client` exposes source-derived keyword methods. Native JSON
models and form/part/header records live in `Models`; source-bound JSON/text
codecs live in `Codecs`. Constructors and every outgoing encoding validate mutable
values again. Required keywords, explicit `UNSET`, JSON null, literal tags, exact
numbers and union arms keep their distinct meanings.

## Credentials and servers

`auth:` is a source-scheme-name hash. Bearer and API keys accept strings; Basic
accepts `BasicCredential.new(username:, password:)`. OAuth/OIDC attachments use
`AuthorizationCredential.new(scheme:, token:)`, supplied directly or through
`credential_provider.call(context)`. The immutable context retains name, source,
scheme, permissions and authorization/discovery metadata. `url_base` is
`effective-server`; `server_url` retains the complete selected server URL,
including its trailing slash. Endpoint strings retain their source spelling.
No token type, login,
discovery, refresh or retry operation is inferred.

Security alternatives are explicit indices when there is more than one choice.
All schemes in the chosen alternative apply conjunctively; `{}` is anonymous and
an empty effective security array disables inherited auth. Conflicting attachments
fail before dispatch. Role names remain separate from OAuth scopes.

Multiple server candidates require an index or declared name. Variable defaults
are the declared Server Variable defaults; unknown overrides and enum violations
fail. Relative servers require the HTTP document retrieval URL (`document_url:`)
when the source was loaded from a local file. A `$self`/schema identifier is not a
network origin. `server_url:` is an explicit caller override.

For remotely supplied documents the default base is the effective physical
retrieval document recorded with the server, including redirects. An absent
inherited server array uses the entry document; an explicit empty override uses
the overriding document. Encoded dots/slashes, percent-escape case and empty path
segments keep their literal spelling through the HTTP request.

## Media, fields and bytes

Use `content_type:` for a request with several media alternatives or wildcard
media. Wildcards match a concrete media type and cannot bypass a more-specific
schema. `accept:` expresses a concrete response preference. Matching a status
never falls back to a range/default merely because its media did not match.

`Bytes.new(string)` is immutable in-memory octets. No file path is opened and no
JSON value stands in for binary data. `Part.new(data:, headers:, content_type:,
filename:)` attaches optional MIME metadata to a typed value; `BytePart` accepts
`bytes:`. Multipart body models enforce required members, extras and cardinality
without evaluating their byte-bearing aggregate as JSON. Required declared part
headers are validated with their own real codecs.

Text scalar bodies and headers use exact lexical conversion. Header values are
not URI-encoded. Composite values must be flat and have an unambiguous delimiter
representation; callers must pre-escape reserved-expansion hazards. Empty
composites are not silently converted into absent or empty required fields.

Responses retain actual status, raw headers, `typed_headers`, media and link
metadata. No declared response content yields bounded `Bytes`. HTTP-forbidden
content yields `NO_CONTENT`, distinct from JSON null. Links never select or call
another operation.

## Stream lifetime

SSE and JSON lines require standard OpenAPI 3.2 `itemSchema`. Their data is an
`ItemStream`, a native Enumerator with `close`. `each` closes on completion,
exceptions and early block exit. After `next`, use an ensure block to close it.
The transport reads on demand through a single bounded chunk handoff. The total
monotonic deadline remains active while iteration is paused, and cancellation
interrupts blocked Ruby I/O. A forgotten stream still has a finite deadline.

SSE framing handles split UTF-8, CR/LF/CRLF, comments, multiline data, ignored
unknown/invalid fields and numeric retry. It uses the HTML UTF-8 replacement rule
for malformed event-stream bytes. `data` remains a string, `[DONE]` has no special
meaning, and retry/id metadata does not initiate reconnection. JSON lines use the
strict exact JSON parser; blank lines and malformed JSON fail. No record-separator
or sentinel convention is inferred.

Finite request streams accept Enumerable native items and are validated and
buffered within request/item/count ceilings before dispatch. They are sent once.

## Errors and resource limits

Public operation/status errors carry validated declared data. Other categories
are `RequestError`, `ResponseError`, `TransportError`, `TimeoutError`,
`CancelledError` and `ResourceLimitError`. Causes retain native hook/codec errors.
Messages and inspect hide credentials and payloads; bounded captures are explicit.

The default total deadline is 30 seconds. It includes encoding, HTTP consumption
and decoding; operations accept `timeout:` and `cancellation:`. Request/response,
part/item, framing/header, URL, count, JSON, conversion and schema/equality work
ceilings are finite. Logical trials share budgets and cannot hide incomplete
evaluation. `JsonNumber` preserves exact decimal tokens with symbolic exponents;
`to_i(max_digits:)` is bounded and `to_f` is explicitly lossy.

The injectable transport is `exchange(request:, context:) { |response| ... }`.
Client consumes and closes each yielded response. Injected transports are
borrowed; client-owned adapters close on `Client.open` exit. Net::HTTP uses one
connection per exchange, verifies TLS, preserves method spelling and disables
ambient proxies, automatic redirects, decompression, authentication and retries.

## Scoped schema validation

The compiler selects the validation format from the actual schema closure.
Ordinary schemas retain the v1 program format. Conditional branches, dependent
requirements/schemas, contains counts, pattern properties, property-name schemas
and unevaluated members/items use the checked scoped-applicator v2 profile.
Each child schema has an independent local evaluated-location scope. Only the
documented successful annotations propagate; failed branches do not make extras
valid. Work spent on branch trials and annotation merges is shared and finite.

Named properties stay native keyword fields. Pattern-matched and unevaluated
extras remain exact JSON values in `extra_fields`; every matching pattern and the
complete parent schema are checked on construction and every encode. Matching a
pattern does not discard that value or exempt it from overlapping patterns.
JSON-value carriers for refined/combined schemas remain source-validated rather
than becoming unchecked hashes. `UNSET`, null, exact decimals and native model
mutation keep their ordinary codec behavior.

Declared examples use the same scoped compiler as the SDK, retaining invalid
findings and original provenance. Unsupported program versions, malformed graph
targets, numeric operands or pattern programs fail before evaluation, including
inside otherwise inactive branches. The v2 profile does not infer dynamic/resource
binding behavior from static references.

## Resource-scoped schemas

Canonical resources and dynamic references use the explicit checked v3 pair
`suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic`. V1 and v2 keep their established
envelopes and reject resource metadata or dynamic instructions. Physical source
URIs/pointers remain the identities used by codecs and errors; `$self`, `$id`,
canonical aliases and dynamic bindings are separate metadata. Validation performs
no acquisition, URI resolution or raw-schema interpretation.

Each entered node activates its indexed resource, even when a codec starts below
the resource root. The outermost actually entered matching dynamic binding wins.
Unentered candidates are inert; pointer, empty-fragment and static-anchor
fallbacks retain their exact initial target. Every return and logical trial
restores the caller's scope. Cycle identity includes the exact ordered resource
context, and dynamic targets start with a fresh evaluated-location scope.
Resource entry and each inspected resource/binding consume the shared work budget;
depth/work/recursive failures remain `EvaluationFailure`.

Named object fields retain keyword models. Dynamic-reference sites and v3 union
carriers hold the exact JSON value domain, with `JsonNumber` for decoded numbers.
Their codec validates the complete original source program rather than selecting
a candidate native type during generation or re-trying a branch without its
outer context. A nested model checked alone can legitimately use a different
binding from its enclosing model. Use the enclosing `Codecs` entrypoint for that
value; generated v3 examples use the same rooted decode/materialization path.
