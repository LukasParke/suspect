# C++20 SDK

The C++ backend emits native value models, exact codecs, an injectable synchronous
HTTP client, a libcurl adapter, and an installable CMake package. Its public Rust
entry point is `suspect_codegen::cpp_sdk::plan_sdk`, behind the `cpp-sdk` feature.
`SdkPlan::render()` returns the complete artifact set rooted at **`cpp/`**.

The current implementation consumes `http_protocol::ProtocolPlan`. Its expanded
profile keeps JSON/text codec roots, byte policies, form/multipart aggregate rules,
typed headers and streamed item roots separate. Enable `cpp-sdk`
when building without the workspace's default features.

## Generate and install

Supply an owned canonical `Contract`, the exact selected operation sources, and
`SdkConfig`. Package identity is separate from service behavior:

```rust
use suspect_codegen::cpp_sdk::{SdkConfig, plan_sdk};

let plan = plan_sdk(contract, &selected_sources, SdkConfig {
    name: "generated_sdk".into(),
    namespace: "generated_sdk".into(),
    version: "0.1.0".into(),
    ..SdkConfig::default()
})?;
suspect_codegen::write_files(&plan.render()?, &output_directory)?;
```

The returned files include public headers, compiled implementation files, CMake
install/export metadata, Doxygen configuration, a source reference, manifests,
checked example values and an executable native-constructor quickstart.

```sh
cmake -S generated/cpp -B build-cpp \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$PWD/install-cpp" \
  -DSUSPECT_SDK_BUILD_DOCS=ON
cmake --build build-cpp
ctest --test-dir build-cpp --output-on-failure
cmake --build build-cpp --target sdk_docs
cmake --install build-cpp
```

An independent consumer needs the installed package, rather than generated source
directories in its include path:

```cmake
cmake_minimum_required(VERSION 3.24)
project(Consumer LANGUAGES CXX)
find_package(generated_sdk 0.1.0 CONFIG REQUIRED)
add_executable(app main.cpp)
target_link_libraries(app PRIVATE generated_sdk::generated_sdk)
```

Configure that consumer with `-DCMAKE_PREFIX_PATH=/path/to/install-cpp`. The
exported target carries its C++20, threading and libcurl requirements. A core-only
package uses `-DSUSPECT_SDK_WITH_CURL=OFF` and an injected `Transport`; its installed
target has no libcurl dependency. Documentation is a build-time dependency and
can be disabled with `-DSUSPECT_SDK_BUILD_DOCS=OFF`.

## Ordinary OpenRouter calls

These are the actual public names for the five-operation OpenRouter selection.
The scheme named `apiKey` declares HTTP bearer authentication, so its token becomes
`Authorization: Bearer ...`. The application supplies the token. Management-key
guidance from the original descriptions remains documentation, rather than a
new credential type invented by the generator.

```cpp
#include <generated_sdk/sdk.hpp>
#include <iostream>
using namespace generated_sdk;

int read_credits(std::string token) {
    Credentials credentials;
    credentials.api_key = std::move(token);
    auto connected = Client::with_curl(std::move(credentials));
    if (!connected) {
        std::cerr << connected.error().message;
        return 1;
    }

    auto result = connected.value().get_credits();
    if (!result) {
        if (auto denied = std::get_if<GetCreditsStatus401>(&result.error())) {
            std::cerr << denied->data.error.message;
        } else if (auto error = std::get_if<SdkError>(&result.error())) {
            std::cerr << error->message;
        }
        return 2;
    }
    const auto& response = std::get<GetCreditsStatus200>(result.value());
    std::cout << response.data.data.total_credits.token() << '\n';
    return 0;
}
```

No-input operations can omit their input argument. Inputs with source-optional
fields still offer their named input type. The outer response wrapper's `.data`
holds the model; the inner `.data` above is an actual OpenRouter JSON member.

Requests use direct constructors, including readable operation-role names for
inline request bodies:

```cpp
CreateKeysBody body("Native Test Key");
body.limit = JsonNumber::parse("50.25").value(); // fixed checked literal
body.limit_reset = Null{};
auto created = client.create_keys(CreateKeysInput(std::move(body)));

UpdateKeysBody update;
update.name = "Updated Native Key";
update.limit = JsonNumber::parse("75.50").value();
update.limit_reset = Null{};
auto changed = client.update_keys(UpdateKeysInput("fixture-hash", update));
```

Check numeric parse results for application-supplied text. Integral conveniences
such as `JsonNumber(50)` and `JsonInteger(2)` avoid parsing when the caller already
has an ordinary integral C++ value. Floating-point constructors are deleted.

The generated README's constructor call site is the same text compiled into
`examples/client.cpp`. Running that executable without arguments uses its checked
fixture transport; supplying a token explicitly performs one request against the
source-declared server. Declared examples and synthesized examples are labeled,
and invalid source examples remain findings in `docs/coverage.json`.

## Values and validation

| Schema states | C++ representation |
| --- | --- |
| Required non-null | `T`, supplied to the constructor |
| Required nullable | `Nullable<T>` = `std::variant<Null,T>` |
| Optional non-null | `Presence<T>` = `std::optional<T>` |
| Optional nullable | `Presence<Nullable<T>>` |

`std::nullopt` means omission; `Null{}` means explicit JSON null. Defaults are not
inserted. Required singleton string tags are initialized from their proved source
literals: for example, the base fixture's `StandardPayload("plain")` constructor supplies `kind: "standard"`.
Enums, object fields, arrays and union alternatives retain native types.

Open extras use an exact `JsonValue` map; typed `additionalProperties` uses its
actual native value type; closed objects expose no extra map. A declared-key
collision is an error even when that declared member is absent. Recursive edges
use deep-copy `Box<T>` ownership, including recursive variants and nullable
objects. A moved-from Box remains an explicit model error.

Named codecs expose `decode`, `encode` and `to_json`. Every encode revalidates the
current mutable model. Union encoding checks both the selected arm and the parent
schema. `oneOf` checks exclusivity; `anyOf` decodes the first valid source arm.
Incomplete evaluation cannot be hidden by another successful union arm or inverted
by `not`.

The exact runtime preserves number tokens and mathematical integer semantics,
including `1.0`, `1e3`, negative zero, very large exponents and zero-padded exponent
spellings. Coefficients and exponents remain symbolic; exponent magnitude never
causes zero-padding allocation. `.to_int64()` performs a checked conversion.
JSON equality separates numbers from Booleans, compares numbers mathematically,
and compares Unicode scalar sequences without normalization. Duplicate decoded
keys, invalid UTF-8, unpaired surrogates and trailing JSON input are rejected.

The generator checks `OwnedProgram` before emission and lowers its instructions
to immutable native tables. The executor covers the complete current checked
instruction set, including the portable Unicode pattern NFA. It does not interpret
OpenAPI documents or substitute a platform regex implementation at runtime.
`emit_validation_runtime` exposes this checked layer independently of the narrower
native model-layout admission.

Errors distinguish invalid JSON, completed schema invalidity, evaluation failure,
native representation failure and resource exhaustion. They retain the original
document, JSON Pointer, byte range, instance pointer and parser offset where
applicable. HTTP failures additionally carry the stable operation identity.

## Transport and lifetime

The declared adapter profile is **C++20, HTTP/1.1, libcurl >=7.85**, with TLS,
asynchronous DNS and thread-safe initialization. The client is synchronous and
owns `std::shared_ptr<const Transport>`; copies retain that transport. All curl
easy/multi handles, header lists, callback state and initialization lifetimes are
RAII-owned. Each call uses a fresh exchange and performs one transport attempt.

`Transport::open` returns an owning `HttpExchange` with a pull-based `ResponseBody`.
Existing finite adapters can implement `send`; its default open adapter splits
the bounded result into bounded chunks. The libcurl adapter instead waits only
for the response head, pauses receipt when its finite window fills, and resumes
when the consumer asks for more bytes. `ItemStream<T>` transfers this lease into
a move-only range cursor, so breaking iteration closes the socket independently
of the outer response's lifetime.

- TLS 1.2 or newer, peer verification and hostname verification are mandatory.
  `CurlOptions` allows explicit CA bundle/directory configuration.
- Redirects, ambient proxies, netrc, cookie persistence, referer generation,
  decompression and automatic retries/replay are disabled.
- Server declarations/overrides support HTTP and HTTPS. URL/query construction
  uses exact RFC3986 escaping and incremental size checks.
- `CallOptions::stop` accepts a native stop token. `Cancellation` requests stop on
  destruction. Cancellation is checked throughout codecs and validation and at
  most every 25ms while waiting for libcurl I/O.
- Timeouts cover request validation through response decoding and every stream
  pull. Buffered calls release I/O before returning; stream responses own their
  transfer until exhaustion, close, error or destruction.
- Injected adapters receive the same cancellation/deadline and receive-time caps.
  The client checks returned status, headers and body too. Adapter exceptions
  retain their original `exception_ptr` cause.

Use an application-owned `std::jthread` for background calls and retain an immutable
input and a Client copy until the call completes. A custom adapter must implement
the documented concurrent-call and cancellation contract; arbitrary application
code cannot be preempted by the SDK.

Default ceilings are 8 MiB for each request URL/body and response body, 64 KiB
for captures and cumulative headers, 256 response header fields, 128 JSON/native
levels and 32 MiB of shared JSON/conversion byte-and-node work. Validation defaults
to 128 levels, 4,096-byte numeric operands, 100,000 evaluation visits, 100,000
equality visits and 100,000 numeric work units. All trial branches share budgets;
an entire client call also shares its request/response codec work. The runtime
bounds work and retained data, rather than promising a fixed process RSS.

Byte ceilings can only be lowered by client/call options. Timeouts must be positive,
at most one day, and no greater than the enclosing client timeout. Responses and
failures retain a separately bounded body capture with explicit truncation,
including incomplete transfers.

Default part limits are 8 MiB and 1,024 parts; item frames are limited to 1 MiB,
with a 64 KiB receive window (caller minimum 16 KiB). Request and stream-item
validation/conversion share one context: no counter is reset between items.
Idle streams check their continuing deadline on the next pull; stop requests
close idle libcurl transfers without another read.

## Native descriptors and profile boundaries

Compatibility consumers use the retained plans directly:

- `ModelSymbol`: source, public name, actual C++ type, native definition, shape,
  nullable view, codec name and constructor;
- `Field`, `ConstructorParameter`, `TagInitializer`: wire/native names, types,
  requiredness, exact argument order and source-backed tag defaults;
- `PlannedOperation`, `PlannedParameter`, `PlannedBody`, `PlannedResponse`: actual
  methods, input/result/error types, response variants, codecs and source identities;
- credentials, the checked validation program and the shared example plan.

Model type references are fully qualified internally, so a source model named
`Failure` cannot accidentally bind to the runtime's private failure carrier.
Human-facing constructors retain names such as `WidgetInput` and `CreateKeysBody`.
Overlong identifiers retain a readable prefix/suffix plus a deterministic digest;
original wire names remain separate. Long literal strings are emitted in bounded
C++ string fragments. Compiled literal nesting beyond 256 or numeric tokens beyond
65,536 bytes are rejected at their original source before emission, independently
of the smaller per-call operand policy.

The protocol profile supports OpenAPI 3.0/3.1/3.2 source declarations for:

- Anonymous, OR and AND security; bearer, Basic, header/query/cookie API keys,
  and caller-owned OAuth2/OIDC providers. Providers receive source metadata,
  scopes/roles and cancellation/deadline controls; acquisition remains application
  code unless an `sdk_defaults` OAuth policy opts in to the generated token lifecycle
  ([SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md)).
- Multiple and relative servers with variables/defaults/enums; exact method tokens,
  including OpenAPI 3.2 QUERY and case-sensitive additionalOperations.
- Defined scalar/flat-object/array parameter styles, content parameters, reserved
  expansion guards, header/cookie parameters and complete-querystring JSON/text/form content.
- Exact/range/default response precedence and actual-status success classification;
  JSON, `+json`, UTF-8 scalar text, bytes, media wildcards/parameters and source-free JSON.
- HTTP-forbidden body handling, bounded undeclared response bytes, typed response
  headers and inert source Link metadata.
- Native form and named multipart aggregates, per-part codecs/headers, structural
  rules, explicit byte data and safe boundary/name/filename handling.
- OpenAPI 3.2 SSE envelopes and JSON-lines itemSchema, with shared call/codec budgets,
  finite receive/item buffers and RAII pull/range iteration. Request streams use
  finite native item vectors. JSON-in-data, sentinels, retries and pages are not inferred.

The native `legacy_binary_strings` option explicitly enables the versioned interpretation
of 3.1/3.2 `type: string, format: binary` markers in binary media. The admitted actual
OpenRouter expansion includes `downloadFileContent`, `downloadContainerFileContent`
and `createCoinbaseCharge` (including its anonymous security and content-absent
declared success). `createOauthToken` retains the shared `http-form-untyped-extras`
refusal: its source leaves extra form-field encoding undefined. No input was repaired.
Public backend and compatibility entrypoints use canonical
`GenerationOptions.compatibility_profiles` with `LegacyBinaryStringV1`; an empty
set keeps standard interpretation. C++ capture reads the same value from
`NativeSnapshot.generation`.

Active directional views retain located model-admission refusals. General v1
native model intersections/tuples keep their earlier boundary; within an admitted
scoped v2 closure, non-static views use an explicitly checked JSON-value carrier.
Positional or
streamed multipart, ambiguous untyped header extras and flattened response-form
object ownership need separate profiles. The checked validator independently
supports tuple/intersection instructions. Unsupported models never become arbitrary
JSON or strings. Native protocol details are retained in `docs/protocol-plan.json`.
All nine shared v2 scoped-applicator operations and the checked v3 resource/dynamic
profile have native execution and source-driven tests. Unsupported program
versions cannot become static-reference guesses.

### Native protocol usage

The generated request content wrappers retain their actual allocated names.
For the independent protocol fixture, ordinary call sites are:

```cpp
auto result = client.media(MediaInput(
    MediaBodyApplicationJsonContent(Record("hello"))));

CallOptions selection;
selection.security_alternative = 1; // source-declared AND alternative
auto authorized = client.secure(SecureInput{}, selection);

CallOptions controls;
Cancellation cancellation;
controls.stop = cancellation.token();
controls.timeout = std::chrono::seconds(10);
auto response = client.events(EventsInput("normal"), controls);
if (response) {
    auto& items = std::get<EventsStatus200>(response.value()).data;
    for (const auto& item : items) {
        if (!item) break; // explicit SdkError outcome
        std::cout << item.value().data; // SSE data remains a string
        break; // cursor destruction closes the transfer
    }
}
```

These paths are exercised by the installed native consumer in
`cpp_sdk/tests/native_protocol.cpp`. The fixture is synthetic and independently
checks normative wire bytes; it is not presented as an OpenRouter stream contract.

## Credentials and validation profiles

### Explicit environment credentials

`SdkConfig::credential_env` accepts the shared optional `CredentialEnv` policy.
The policy is bound through `credential_env::plan` after HTTP admission and is
available through `SdkPlan::credential_env()`. Only mapped variable **names** are
generated. With no policy, the existing package output and constructors retain
their prior bytes and behavior.

For a package/namespace named `openrouter`, configured with source scheme
`apiKey` → `OPENROUTER_API_KEY`, the compiled helpers are:

```text
static Result<Client, TransportError> Client::from_env(
    ClientOptions options = {}, CurlOptions curl = {});

static Client Client::from_env_with_transport(
    std::shared_ptr<const Transport> transport, ClientOptions options = {});
```

`from_env` requires the default libcurl-enabled build. The supplied-transport
companion also works in a core-only C++20 build. Both copy `std::getenv` values at
factory invocation, never at import/generation or per request. The copied value
is bounded to 8,192 bytes per mapped string. Missing, empty, over-limit or
attachment-incompatible values stay unavailable; protected operations fail with
the existing secret-free `SdkError::Kind::RequestValidation` before HTTP. Mixed
anonymous/protected clients remain usable for anonymous operations. Curl setup
errors return `TransportError`; allocation failures retain the standard C++
allocation-error policy.

Explicit `Client(transport, credentials, options)` and `Client::with_curl(...)`
remain authoritative for the whole credential argument. An empty credentials
object, absent `std::nullopt` member or empty string is never filled from env.
Configured aliases to the same terminal scheme retain separate declaration-bound
fields and variable snapshots. OR/AND and explicit alternative selection retain
their existing semantics. Source-default servers, RAII/moves, stop and deadlines
are unchanged.

The live-default call is `ready.value().get_current_key()` after checking
`auto ready = openrouter::Client::from_env()`. It uses the actual source default
`https://openrouter.ai/api/v1/key`. The native tests also compile the optional
`get_credits()` call against `/credits`; they make no real account calls.

Focused gates are in `tests/cpp_credential_env.rs`:

- `native_openrouter_credential_env_snapshot_and_explicit_precedence`
- `native_credential_env_security_and_portable_controls`
- `actual_openrouter_credential_env_generation_contract`
- `credential_env_binds_native_declarations_and_reserves_only_configured_helpers`

Native capture stores only the retained policy's `semantic_descriptor()` in `NativeSnapshot::credential_env`.
Physical provenance remains in the configured package's `docs/credential-env.json`.

### Scoped validation v2

The C++ runtime implements all nine scoped operations from
[the executable applicator contract](SDK-SCHEMA-APPLICATORS.md). The planner uses
`OwnedCompiler::compile_v2`, with v1 programs retained for ordinary closures.
Unknown version/profile pairs and malformed descriptors remain refused before
package emission; resource/dynamic closures use the separate checked v3 profile.

Each schema evaluation owns fresh evaluated-property/item sets. Successful
same-instance branches propagate only their specified sets; failed branches
export none. Conditions and contains retain their independently specified local
annotations. Every required visit and set insertion, including duplicate merges,
spends the shared evaluation budget. Errors and exhausted budgets cannot be
inverted into success by not, if, or an already-passing alternative.

Objects retain named native fields and constructors. Patterned extras use
`std::map<std::string, JsonValue>` and are validated against **every** matching
pattern plus the whole-object additional/unevaluated policies. Closed additional
properties do not erase matching pattern keys. Some non-static conditional,
intersection, and tuple views use `Shape::ValidatedJson`: an exact value carrier
with an explicit named-codec obligation. This is reflected in native compatibility
records as `validated-json` and `wholeObjectPatternValidation`.

For the scoped SDK fixture:

```cpp
Ledger ledger(JsonInteger(1));
ledger.extra.emplace("s-label", JsonValue("yes")); // pattern-matched string
ledger.extra.emplace("other", JsonValue(JsonInteger(2))); // additional integer
auto result = client.ledger(LedgerInput(ledger)); // full scoped preflight

ledger.extra["other"] = JsonValue("invalid");
auto encoded = LedgerCodec::encode(ledger); // explicit validation error
```

Canonical `plan_protocol_examples_v2` supplies validated values, origins and
findings; C++ supplies native constructors and executable lowering. Copying a
recursive `Box` retains independent ownership, and moved-from values remain codec
errors. Resource and cancellation failures unwind active identities and scopes.

The native tests compile the original **32-case source fixture** using the real
`compile_v2` API. They cover exact visit/duplicate-merge thresholds, escaped and
non-normalized key identity, symbolic contains counts, mutation, interruption,
and a 512-depth failure on an explicitly 2 MiB thread stack in a debug build.
The SDK test builds and installs a fresh CMake package, executes generated guides,
renders Doxygen, compiles invalid consumer programs, and exercises real libcurl
exchanges across the scoped SDK operations.

### Physical document server bases

`DocumentRelativeServers` is admitted. Its installed native test exercises real
libcurl exchanges across two physical origins, redirected document identities,
logical `$self` addresses, referenced path items, inherited/explicit-empty defaults, variables,
explicit document/server overrides, and encoded dot/slash/case preservation.
OAuth/OIDC hooks receive `CredentialRequest::effective_server_url` and
`metadata_url_base`; relative metadata endpoints remain unacquired. Diagnostics
continue pointing to physical documents and spans. The shared protocol record
retains the separate logical `ResourceContext` metadata and candidate-aware
`codec_schema_closure()` without turning candidate schemas into body inputs.

### Checked resource/dynamic v3

The existing `plan_sdk` entrypoint selects `OwnedCompiler::compile_v3` only for
resource-bearing codec closures. Ordinary selected closures retain their v1/v2
programs. `SchemaResources` and `DynamicSchemaReferences` are admitted; no
separate generation option or acquisition path is introduced. Examples use canonical `plan_protocol_examples_v3`.

`ProgramResourceContext` records resources and aligned node scopes. Native
evaluation enters the node's actual indexed resource, even for nested entry,
and never evaluates an unrelated resource root to obtain its bindings. The
outermost actually entered matching resource wins dynamic lookup. The initial
fallback is not entered early; pointer/empty/static-anchor fallbacks stay static.
Unentered candidates remain inert. Return/trial/unwind restores resources, and
cycle identity includes an exact interned ordered resource context. Resource and
binding scans charge the same shared visit budget as schema evaluation.

Dynamic-reference values and v3 union views have an explicitly checked
`JsonValue` representation, retaining all native codec/HTTP validation. Named
object fields stay typed. This avoids selecting a fallback type or validating
an isolated union arm without its caller's resource scope. Physical sources and
logical resource metadata remain separate throughout planning, docs and failures.

The 44 official cases and supplied remote documents are unmodified source files
under `crates/suspect-schema/tests/fixtures/resource-conformance/`, compiled via
the real `compile_v3` API. No maintained test reads a frozen target program.
Independent controls cover outermost/unentered bindings, premature fallback,
detached entry, branch restore, context-sensitive cycles, noninvertible failures,
exact resource/binding budgets, malformed metadata, cancellation/deadlines and a
550-resource chain with a 512-depth failure on a 2 MiB debug thread stack.
The installed SDK gate checks CMake, Doxygen, executable guides, three negative
consumers, six real libcurl exchanges and streamed-item resource scopes.

## Verification

Native gates are explicit ignored Rust tests in
[`crates/suspect-codegen/tests/cpp_sdk.rs`](../crates/suspect-codegen/tests/cpp_sdk.rs)
and [`crates/suspect-codegen/tests/cpp_protocol.rs`](../crates/suspect-codegen/tests/cpp_protocol.rs).
They fail when required tools or the actual OpenRouter checkout are unavailable.
Native tool selection uses `SUSPECT_CPP_CXX`, `SUSPECT_CPP_CMAKE` and
`SUSPECT_CPP_DOXYGEN`.

```sh
cargo test --locked -p suspect-codegen --features cpp-sdk --test cpp cpp_sdk:: \
  --target-dir target/sdk-cpp-cargo

SUSPECT_CPP_CXX=/usr/bin/clang++ \
SUSPECT_CPP_CMAKE=/absolute/path/to/cmake \
SUSPECT_CPP_DOXYGEN=/absolute/path/to/doxygen \
cargo test --locked -p suspect-codegen --features cpp-sdk --test cpp \
  --target-dir target/sdk-cpp-cargo -- --ignored --nocapture --test-threads=1
```

For a specific expanded gate, set the same native tool variables and run:

```sh
cargo test --locked -p suspect-codegen --no-default-features \
  --features cpp-sdk --test cpp \
  native_rich_protocol_installed_wire_and_streams \
  --target-dir target/sdk-cpp-cargo -- --ignored --nocapture
```

The native cases exercise the base fixture, five operations in the
actual OpenRouter description, independent hand-authored wire bytes, installed
consumers, positive-controlled negative type compilation, Doxygen symbol indexes,
the shared 17-case runtime corpus, hostile names/prose, typed extras, recursion,
mutable encodes, resource limits and real cancellation/TLS/transport behavior.

Tested native tooling on arm64 macOS: Apple Clang 21.0.0 in C++20 mode, CMake
3.31.6, system libcurl 8.7.1 with SecureTransport/LibreSSL, and Doxygen 1.18.0.
No additional compiler or operating-system support is implied by this toolchain
record.

The expanded protocol consumer also checks split UTF-8/CRLF/BOM framing, empty SSE
data, ID persistence/reset, invalid ID/retry handling, `[DONE]` as ordinary data,
discarded pending SSE at EOF, strict JSON-lines, bounded captures/chunks/empty
polls, exceptions, moved ownership, multipart boundaries/preamble/epilogue and
release on pre-decode failures. Wire records are independent fixture assertions;
OpenRouter descriptions are read-only and exercised against local servers.

Exact library selectors (`--lib`, `--ignored --nocapture`):

- `cpp_sdk::v2_tests::native_v2_independent_32`
- `cpp_sdk::v2_tests::native_v2_scope_resource_edges`
- `cpp_sdk::v2_tests::native_v2_sdk_operations`
- `cpp_sdk::server_tests::native_document_relative_servers`
- `cpp_sdk::v3_tests::native_v3_official_dynamic_ref_44`
- `cpp_sdk::v3_tests::native_v3_scope_and_resource_controls`
- `cpp_sdk::v3_tests::native_v3_sdk_operations`
