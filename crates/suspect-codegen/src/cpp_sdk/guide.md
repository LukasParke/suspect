
## Values, presence and exact numbers

Models are native value structs. Constructors require the source-required members;
proved singleton string tags are initialized automatically. Optional members start
absent, and source defaults are annotations rather than inserted business values.

| Source member | Native type | Assignment |
| --- | --- | --- |
| Required non-null | `T` | required constructor argument |
| Required nullable | `Nullable<T>` (`std::variant<Null,T>`) | `Null{}` or a value; never missing |
| Optional non-null | `Presence<T>` (`std::optional<T>`) | `std::nullopt` or a value |
| Optional nullable | `Presence<Nullable<T>>` | `std::nullopt`, `Null{}`, or a value |

`JsonNumber(42)` and `JsonInteger(42)` accept ordinary integral C++ values. Exact
decimals use `JsonNumber::parse("9007199254740993.000000000000000001")`, returning
`Result<JsonNumber, CodecError>`. Check the result before accessing `.value()` for
application-supplied tokens. `JsonInteger::parse("1.0")` accepts mathematical
integers, and `.to_int64()` performs a checked conversion. Floating-point inputs
are not implicit exact numbers. Exponents remain symbolic, including extremely
large exponents and zero-padded exponent spellings.

The model's named codec exposes `decode`, `encode` and `to_json`. These are useful
for storage and tests; ordinary client calls accept native models directly.
Encoding validates current mutable values, including strings, enums, extra keys
and the selected union arm plus its parent assertions. `oneOf` requires exactly
one valid branch; `anyOf` decoding selects the first valid source branch. Native
variants have indexed `alternative_0`, `alternative_1`, etc. factories, so even
alternatives with the same underlying C++ type have unambiguous construction.

Recursive edges use deep-copy `Box<T>`. Copies have independent ownership, and a
moved-from Box is an explicit codec error. JSON object key order and whitespace
may change; number tokens and Unicode scalar sequences are retained. Duplicate
decoded keys, invalid UTF-8 and unpaired surrogates are errors. Unconstrained JSON
and open-object extras legitimately use `JsonValue`; unsupported declared model
layouts are rejected during generation.

## Errors and response metadata

Operations return `Result<OperationSuccess, OperationError>`. Successful statuses
form a native variant of distinct response wrappers. Errors form a variant of
`SdkError` and each source-declared error-status wrapper. Use `std::get_if` or
`std::visit` to inspect a known status. A typed response has `.data` and `.response`
metadata: status, normalized media essence, ordered headers, bounded body capture
and a truncation flag. A wire field named `data` remains its own model member.

`SdkError::kind` separates configuration, request validation/representation,
transport, resource limits, unexpected responses, response decoding, cancellation
and timeout. `.codec` retains the exact source keyword, instance pointer, byte
range and parser offset. `.cause` preserves exceptions from an injected adapter.
An invalid declared error body is a decoding failure, not a valid typed API error.
Messages do not print credentials or raw response bodies.

`Result::value()` and `.error()` require the matching arm, like `std::variant`;
incorrect use throws `std::bad_variant_access`. Standard allocation failures use
`std::bad_alloc`. Internal unwinding requires C++ exceptions to be enabled.

## Timeout, cancellation and transport ownership

`ClientOptions` and `CallOptions` set positive timeouts and lower byte ceilings.
Per-call timeouts can only lower the client timeout; the maximum is one day.
Timeouts span request validation, URL/body construction, DNS/TLS, response receipt,
validation and decoding. `CallOptions::stop` accepts a `std::stop_token` from a
`std::stop_source`, `std::jthread` or the scope-owned `Cancellation` helper.
Destroying `Cancellation` requests stop. Run synchronous operations on an
application-owned `std::jthread` when background work is needed; retain the Client
and immutable input for that call's lifetime.

Client copies retain a `std::shared_ptr<const Transport>`. The recommended
`CurlTransport` owns fresh easy/multi handles and header lists for each exchange,
and pairs libcurl initialization/cleanup through shared RAII state. It uses
HTTP/1.1, TLS 1.2 or newer, verified peer certificates and verified hostnames.
`CurlOptions::ca_bundle` and `ca_directory` allow explicit CA trust configuration.
The linked libcurl must be at least 7.85 with TLS, asynchronous DNS and thread-safe
initialization. The loop polls at most every 25ms and closes native I/O on every
exit. Cancellation cannot preempt arbitrary code inside a user-provided adapter.

Redirect following, automatic replay/retries, cookie storage, netrc, environment
proxies, referer generation and content decompression are disabled. No credential
is discovered from the environment. HTTP/HTTPS server selection and overrides are
explicit. Source path/query serializers use exact RFC3986 escapes; server paths
normalize dot segments while operation path parameters reject ambiguous ones.

`Transport::send` remains the finite-response seam. `Transport::open` exposes
headers and a move-only `ResponseBody` for incremental reads. The default open
adapter uses a bounded send result; libcurl overrides it with a demand-driven
multi/easy transfer and a capped receive window. Implementations own their I/O,
are safe for concurrent const calls, and enforce cancellation, deadlines and
cumulative header/body caps. The client verifies returned metadata and chunks.
Partial captures are bounded and marked truncated. Each operation makes one attempt.

## Authentication, servers and representation choices

Credentials have their source-allocated native fields. Bearer and API-key fields
accept strings. Basic fields accept `BasicCredentials(username, password)`.
OAuth2/OIDC fields accept a `CredentialProvider`: a callback returning an explicit
`Authorization(scheme, value)` result. The callback receives original scheme,
flow/discovery metadata, scopes or roles, operation identity, stop token and
deadline. The SDK never retrieves a token, refreshes it or treats roles as scopes.
`CredentialRequest::effective_server_url` and `metadata_url_base` identify the
selected server as the base for relative OAuth/OIDC endpoint metadata.

Security alternatives are OR; members of an alternative are AND. The default
chooses the first alternative whose credentials are supplied, including an
explicit anonymous alternative. `CallOptions::security_alternative` selects one
explicitly. Attachment conflicts are errors before transport. Cookie and query
API keys use their declared locations; cookie persistence remains disabled.

`ClientOptions::server_index` and `server_variables` select/expand a declared
server, and CallOptions can override them. Defaults and enums come from the
source; unknown variables and invalid enum values fail. Relative servers use the
HTTP document retrieval URL, or an explicit `document_url` for local files.
The physical document is retained by `ServerPlan::document_base()`, including
redirected retrievals and defaults inherited through referenced path items.
Logical `$self`/`$id` addresses remain separate metadata. RFC3986 resolution
preserves encoded dots/slashes, percent-triplet case and repeated path separators.
`server_url` is an explicit complete override. HTTP method token case is retained,
including OpenAPI 3.2 QUERY and additionalOperations.

Request bodies with several media types use a native variant of named content
wrappers. Wildcard wrappers require a concrete Content-Type. Selecting a wildcard
cannot bypass a more specific schema. Responses match exact status, then class
range, then default, followed by media specificity and declared parameters.
The actual status determines success/error membership: a default response can
belong to either result arm. A successful `response_media` request is enforced
after source matching, while declared error representations remain decodable.

JSON and structured `+json` media use exact JSON codecs. Text media use their
native scalar and source codec. Binary media use `Bytes` (`std::vector<uint8_t>`),
not a JSON string or null. HEAD and HTTP content-forbidden statuses carry `Unit`;
responses without declared content carry bounded bytes when HTTP permits a body.
Typed response headers have a generated header struct. Links are source metadata
in `.response.links`, with no automatic navigation or operation selection.

## Form, multipart and item streams

Form and named multipart bodies have generated aggregate value structs. Their
constructors require source-required fields, and codecs validate whole-aggregate
presence/cardinality plus each real part/header codec. Multipart part wrappers
carry `.data`, optional `.filename`, `.content_type` selection and typed headers.
Byte parts contain actual in-memory Bytes. Boundaries are checked against payload
collisions; names and filenames are encoded as safe MIME quoted parameters.
OpenAPI 3.2 applies named encoding to each top-level array item. The shared plan
retains the distinct 3.1 whole-value style behavior where applicable.

OpenAPI 3.2 itemSchema SSE/JSON-lines responses use `ItemStream<T>`. `next()` returns
`Result<Presence<T>, SdkError>`: absent means EOF. The stream is move-only and owns
the live transfer, codec context and remaining budgets. A range-for cursor takes
the stream lease; breaking the loop releases the socket even when the response
object remains in scope. Explicit close, errors, exhaustion and stop requests also
close the body. No later transport chunk is requested until needed.

SSE parsing follows HTML framing: split UTF-8/CRLF boundaries, replacement decoding,
BOM, comments/unknown fields, multiline data, valid ID/retry values and blank-line
dispatch. The last valid ID persists until reset by an empty ID. Empty data
dispatches; events with no data and a final unterminated SSE event are discarded.
An empty event type resets that optional field; retry is retained for its event.
Data remains a string; `[DONE]` is ordinary data. Retry is metadata, not an SDK
retry instruction. JSON-lines use strict exact JSON and accept a final
record without LF. Request item streams are finite native vectors, encoded under
the same whole-call budget with per-item limits.

The deadline continues while a stream is idle and is checked before each pull;
an active read checks it while waiting. Stop requests close an idle libcurl
transfer synchronously. The stream does not run a background polling thread.

JSON/conversion work and schema/equality/numeric work are shared across each
codec or complete client call, including all union trials. Evaluation exhaustion
remains a failure inside unions and negation. Source schema constraints execute
from checked immutable `OwnedProgram` tables, including the portable Unicode
pattern NFA, without interpreting source schemas or using platform regex syntax.

## Source profile and documentation

This profile consumes the shared OpenAPI 3.0/3.1/3.2 protocol plan. `format` remains
annotation-only for JSON models. The optional `legacy_binary_strings` configuration
explicitly admits the versioned 3.1/3.2 binary-string compatibility profile; it
does not change JSON strings or infer legacy streaming semantics.

Active directional views retain their located model-admission boundary; ordinary
v1 closures retain the earlier static intersection/tuple boundary. Positional/streamed multipart, ambiguous
untyped header-object extras, ambiguous flattened response-form object ownership,
transport-owned framing/MIME headers and legacy SSE schema/sentinel inference
receive located profile refusals. Server authorities use ASCII/IDNA spelling;
unsupported caller URL spellings fail before I/O. Undefined escaping
combinations fail with their source location rather than guessing a wire value.
The scoped v2 static profile implements `if`/`then`/`else`, dependentRequired,
dependentSchemas, contains with exact min/maxContains, patternProperties and
pattern-aware additionalProperties, propertyNames, unevaluatedProperties and
unevaluatedItems. Every child schema starts with fresh evaluated-location sets;
only the specified successful annotations propagate. Invalid branches discard
their annotations, while evaluation failures immediately stop the call.

Pattern-matched extras use an exact `JsonValue` map. They participate in every
matching pattern and the whole object's remaining policies, including when
additionalProperties is false. Declared properties remain typed and are checked
against matching patterns too. Some conditional/intersection/tuple views use a
checked `JsonValue` carrier where no faithful static layout is available; their
named codec and all HTTP operations validate the full source schema on decode
and every mutable encode. Constructing a value does not prove schema validity.

The planner uses `OwnedCompiler::compile_v2` for ordinary closures, retaining v1
when scoped applicators are unnecessary. Resource-bearing codec closures select
`compile_v3` explicitly. Scoped/resource examples use the corresponding canonical
`plan_protocol_examples_v2` / `plan_protocol_examples_v3` helper and the same
native constructor lowering. Unknown or mismatched envelopes remain refusals.

### Resource and dynamic execution

V3 consumes checked resource records and aligned node scopes. Physical source
IDs/spans remain diagnostics and ownership; canonical/base URIs and aliases are
metadata. A nested schema entry enters its indexed resource without evaluating
that resource's root. Dynamic lookup searches actually entered resources
outermost first and never enters the initial fallback before that lookup.
Unentered candidates are inert; pointer/empty/static-anchor fallback descriptors
remain static. Evaluation performs no URI resolution or document acquisition.

Every exit/trial restores the ordered resource context. Cycles use node, instance
identity and an exact interned ordered context, so a changed context is not a
false cycle. Resource entries and each scanned resource/binding consume shared
work; selected target evaluation has a fresh annotation scope. Failures remain
noninvertible through all v2 applicators and release RAII state on unwind.

Dynamic-reference values and v3 union views use checked exact JSON carriers;
choosing the initial fallback's native type would be unsound. Ordinary named
object fields remain native and mutable encodes validate the complete source.

Doxygen reads the actual public C++ headers and produces HTML plus XML symbol
indexes. Original prose is rendered as inert text, preserving guidance without
treating provider-relative Markdown links as C++ symbol references. Detailed
source pointers, constructor/type/codec bindings and example findings are in
`docs/reference.md` and `docs/coverage.json`. Invalid declared examples remain
findings; synthesized examples have a separate origin. `sdk-manifest.json` records
the generator, profile, configuration and contract identity.
