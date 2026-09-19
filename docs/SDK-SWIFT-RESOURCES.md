# Swift resources, dynamic references and physical server bases

Swift's additive resource profile consumes the checked owned contract in
[SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md), alongside the resource-aware
HTTP descriptors in [SDK-HTTP-PROTOCOL.md](SDK-HTTP-PROTOCOL.md).

## Public entry point and envelope

The Rust entry point remains `swift_sdk::plan_sdk(Contract, selected, SwiftConfig)`.
Canonical `Backend::SwiftHttp` dispatch and the existing configuration API apply.
The planner first tries `OwnedCompiler::compile_v2`, preserving the established
v1/v2 envelopes for ordinary closures. A closure requiring resource admission is
compiled explicitly with `compile_v3`. The admitted v3 pair is:

```text
suspect.validation.experimental.v3
oas31-jsonschema202012-resources-dynamic
```

Validation emission checks the envelope, native limits and `OwnedProgram::check`
before any package artifacts are exposed. It lowers the typed `DynamicRef`
operands and optional `resource_context` directly. Every resource record retains
its physical source, kind, canonical/base URI, aliases, declaration source and
indexed dynamic anchors. `nodeScopes` remains aligned with the node table and
retains the schema-root source and canonical address. No raw schema or URI is
interpreted by native validation.

`ProtocolPlan::codec_schema_closure()` supplies the candidate-aware closure used
for compiler admission and directional checks. `codec_roots()` continues to mean
actual codec inputs; a candidate binding is not promoted into an HTTP input.
V3 examples use `plan_protocol_examples_v3`. The native renderer constructs values
from that bounded plan and does not substitute fallback annotations or retry a
failed shared example evaluation as ordinary invalidity.

`SchemaResources` and `DynamicSchemaReferences` are now promoted after the native
runtime, installed SDK/type/wire/example and documentation gates passed on both
toolchains. `DocumentRelativeServers` has its separate native proof below.

## Native dynamic scope

Each evaluation enters its node's indexed resource, including an entry below an
unevaluated resource root. Only actually entered resources participate. Lookup
scans them outermost first and selects the first exact matching dynamic anchor;
unentered catalogue candidates are inert. The initial fallback resource is not
entered before lookup. Pointer, empty-fragment and ordinary-anchor fallbacks keep
their initial target, and a static Ref never performs dynamic lookup.

The per-call session records ordered first-distinct active resources. Exact
`(previous context ID, resource index)` transitions are interned in an
equality-checked map. The cycle key includes node, exact instance path and context
ID. It is not a hash-only approximation or the old node/instance pair. Successful,
invalid and throwing returns restore active resources and the current context.
Trials retain the shared limits/cache and restore their scope.

Dynamic targets start with fresh annotation sets and propagate only successful
sets, as in v2 Ref. Each newly entered distinct resource and every inspected
resource/binding costs one shared evaluation visit. Numeric, equality, depth and
v2 annotation budgets keep their existing meanings. Evaluation failures remain
noninvertible through anyOf, oneOf, not and conditionals.

The default core depth ceiling of 512 is verified on a **2 MiB native thread**
against a 550-resource chain. It reports an explicit depth failure at the original
`N512` source on both toolchains. No limit or compiler check was disabled.

## Models and source codecs

Static resource schemas retain typed fields, exact numeric types and existing
nullable/presence wrappers. A reverse walk of the admitted instruction graph
identifies models whose conversion could depend on a dynamic binding. Those
values use named, mutable, source-bound checked carriers:

```swift
let value = Choice(value: .object(try JsonObject([
    ("selected", JsonValue.null)
])))
let bytes = try Choice.codec.encode(value)
// {"selected":null}; no synthetic "value" member is added.
```

The carrier's `JsonValue` is the complete wire instance. Its source codec validates
both decoding and current mutable encoding. Dynamic conversion does not narrow
the fallback's shape or nullability, or re-trial a union outside its parent
resource context. Missing keys, explicit nulls, Unicode key bytes and original
number tokens remain distinct. SourceCodable uses SDKJSONEncoder/SDKJSONDecoder.

A layout can be shared by multiple source-bound codecs. A direct call to a nested
source codec starts a fresh resource context, which can differ from the context
entered through an owning reference. The installed witness makes this observable:
the owning detached-entry codec accepts an integer selected by the outer resource,
while the directly exposed nested codec accepts its string fallback. HTTP calls
always retain the owning source codec, including its reference path and resource
entries. Their conversion never substitutes the nested codec's isolated result.

## Physical-document server adoption

`DocumentRelativeServers` is verified on current and floor. Native `HTTPServer`
now exposes `documentBase` and `urlBase`. Its default base is the effective
**physical retrieval document** of the actual declaration, including inherited
servers, external Path Item declarations, defaults and Link servers. An explicit
client/request `documentURL` can override that base. A local file has no invented
HTTP origin. `$self`, `$id` and requested retrieval aliases do not relocate an API.

Resolution folds scheme/host case, removes literal dot segments, and retains
encoded dot/slash segments, path case and repeated separators. Native overrides
reject invalid percent escapes, raw Unicode/whitespace repairs, and fragmentful
document bases. Explicit `serverURL` overrides retain their existing transport
policy.

`HTTPProvenance` exposes `useSiteResource`, `terminalResource` and aligned
`referenceResources`, separately from physical source locations.
`HTTPResourceContext` contains physical source/resource/schema-root locations plus
logical canonical/base URI, scope address, aliases and declaration source. These
fields grant no acquisition authority.

New `HTTPURLBase` values distinguish `.serverDocument` from `.effectiveServer`.
OAuth/OIDC providers receive the selected `HTTPCredentialContext.serverURL` and
the explicit effective-server URL base; flow, metadata and discovery strings keep
their original sources. The SDK performs no token acquisition or URL fetching.

## Maintained native evidence

The runtime test compiles the unmodified pinned source groups in
`crates/suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json` and
the three original supplied remote documents through a closed DocumentProvider
and real `compile_v3`. It has no dependency on the one-off executable file under
`target/`. Expected outcomes come from the official test booleans and independent
literal controls, never from the Rust evaluator.

| Gate | Swift 6.3.3 / SDK 26.5 | Swift 6.0.3 / SDK 15.4 |
| --- | --- | --- |
| V3 evaluator | **93 passed** | **93 passed** |
| Installed V3 SDK | **7 consumer tests + 7 actual wire calls** | **7 consumer tests + 7 actual wire calls** |
| V3 typing | Positive control + **9 required failures** | Positive control + **9 required failures** |
| V3 generated examples / SwiftPM / DocC | Passed, warnings as errors | Passed, warnings as errors |
| Physical servers | **6 consumer tests + 12 exact wire calls** | **6 consumer tests + 12 exact wire calls** |
| Server metadata typing | Positive control + **2 required failures** | Positive control + **2 required failures** |
| Physical-server SwiftPM / DocC | Passed, warnings as errors | Passed, warnings as errors |
| Declared aggregate follow-up | **4 consumer tests**, generated examples and DocC | **4 consumer tests**, generated examples and DocC |

The 93 runtime cases comprise 44 official dynamicRef cases, four formerly deferred
dynamic/unevaluated cases and 45 independent controls. Controls cover fresh target
scopes, outermost precedence, unentered candidates, changed-context cycles,
successful/failed/thrown scope restoration, detached entry, fallback modes,
retrieval aliases, escaped physical pointers, exact budgets, noninvertible numeric
failures and normal-stack depth failure. Checked-emission mutations cover old or
unknown envelopes, missing/misaligned metadata, invalid scopes, aliases, bindings,
sources and initial-resource targets.

The installed V3 fixture includes recursive strict trees, context-sensitive
nullability and unions, detached parent-resource binding, typed static resources,
escaped aliases and physical error locations, mutable/Codable codecs, request
failure before transport, response failure after transport, and compiled direct
examples. The socket fixture verifies exact numeric request bytes.

The physical-server fixture verifies inherited and external declaration bases,
explicit document/server/variable overrides, local-file rejection and explicit
base adoption, encoded path identity, defaults, Link server metadata and the
effective OAuth/OIDC server. The Rustdoc `APIResponse<T>` quoting fix is included.

### Complete declared aggregates

The native constructor renderer now consumes
`OperationExamples.validated_aggregates` for form and multipart grouping. It
retains distinct repeated values, empty arrays, declared dynamic extra names,
optional missing members, and positional prefixes/tails. A declared optional
request body is constructed when its validated aggregate is available. It does
not replace an empty array with a synthesized element. A required repeated array
whose empty value has no wire member produces no executable call example.

Aggregate declarations remain example-only inputs: no aggregate JSON codec or
placeholder for native bytes is added. Each constructed part still uses its real
source codec. The coverage file adds `validatedAggregates` only when nonempty and
retains original declarations plus invalid/unavailable findings. Ordinary JSON
coverage keeps its existing shape. Four focused installed tests exercise exact
form bytes, named repeated/extras grouping, ordered tails and an empty positional
array through the public Client and a capturing transport on both toolchains.

### Host and quality checks

- **19** Swift SDK/protocol/v2 integration checks and **6** Swift unit checks pass.
- The **4** Swift compatibility checks pass.
- Aggregate default-feature Cargo check and warnings-denied Rustdoc pass.
- The three requested Clippy fixes are complete. The scoped Clippy receipt
  records **zero Swift warning/error diagnostics**, including the maintained
  Swift tests. Other owners' diagnostics remain in the common log.
- Scoped rustfmt checks pass. `compatibility/native.rs` was patched only inside
  its Swift capture: form-field/hook matches, planner call and media-schema
  closure. A read-only rustfmt comparison verifies that function's formatting;
  its descriptor values and configuration forwarding are preserved.

Host and quality receipts are in `target/sdk-swift-resources-20260910/`, notably
`host-tests-02.log`, `host-unit-03.log`, `compatibility-01.log`,
`aggregate-check-01.log`, `rustdoc-01.log`, `swift-quality-02.json` and
`swift-capture-format.json`.

### Exact native selectors

Use `cargo test --locked --offline -p suspect-codegen --no-default-features
--features http-protocol` with:

```text
--lib swift_sdk::validation::v3_tests::native_resource_dynamic_source_vectors
--lib swift_sdk::resources_tests::native_installed_v3_resources_codecs_types_wire_and_docs
--lib swift_sdk::resources_tests::native_installed_physical_document_servers
--lib swift_sdk::aggregate_examples_tests::native_installed_declared_aggregate_examples
```

Append `-- --ignored --nocapture`. `SUSPECT_SWIFT_V3_ROOT` chooses a fresh artifact
parent. Toolchain selection uses the existing `SUSPECT_SWIFT_BIN`,
`SUSPECT_SWIFTC_BIN`, `SUSPECT_SWIFT_DOCC_BIN` and `SUSPECT_SWIFT_SDKROOT` variables
documented in [SDK-SWIFT.md](SDK-SWIFT.md).

Retained roots under `${TMPDIR%/}/opencode`:

- `swift-v3-gates/runtime-v3-mrdCsA` — current 93-case runtime proof.
- `swift-v3-floor/runtime-v3-N4OIHL` — floor 93-case runtime proof.
- `swift-v3-gates/sdk-v3-oupATH` — current installed V3 SDK/type/wire/DocC.
- `swift-v3-floor/sdk-v3-JkJpv7` — floor installed V3 SDK/type/wire/DocC.
- `swift-v3-gates/document-servers-qUcvHF` — current physical-server gate.
- `swift-v3-floor/document-servers-Q5DRHQ` — floor physical-server gate.
- `swift-v3-public-current/sdk-v3-MxqnYF` and
  `swift-v3-public-current/document-servers-yhXtqL` — final public-entry current proofs.
- `swift-v3-public-floor/sdk-v3-ShZp6X` and
  `swift-v3-public-floor/document-servers-l7n107` — final public-entry floor proofs.
- `swift-aggregate-current/aggregate-examples-4Re9UR` — current aggregate follow-up.
- `swift-aggregate-floor/aggregate-examples-OXAhJd` — floor aggregate follow-up.
- `swift-v3-gates/sdk-v3-ooY2F8` — retained red consumer constructor-order attempt.
- `swift-v3-gates/document-servers-gApNMg` — retained red socket harness attempt;
  accepted Darwin sockets inherited O_NONBLOCK, corrected with bounded blocking reads.

Each root retains generated sources, checked programs, command logs, independent
consumer builds, typing diagnostics, DocC output and applicable literal wire
captures. Completed v1/v2 and protocol matrices retain their previous evidence.

## Production asset handoff

Changed production paths relative to `crates/suspect-codegen/src/`:

```text
swift_sdk.rs
swift_sdk/validation.rs
swift_sdk/validation.swift
swift_sdk/models.rs
swift_sdk/emit.rs
swift_sdk/protocol.rs
swift_sdk/protocol_emit.rs
swift_sdk/protocol_metadata.rs
swift_sdk/protocol_runtime.swift
swift_sdk/protocol_examples.rs
```

These are existing registered production assets. New test-only sources are
`swift_sdk/validation_v3.rs`, `swift_sdk/validation_v3_support.rs`,
`swift_sdk/resources_tests.rs`, `swift_sdk/validation_v3_native.swift` and
`swift_sdk/protocol_resources_native.swift`, `swift_sdk/aggregate_examples_tests.rs`
and `swift_sdk/aggregate_examples_native.swift`. They are not runtime assets.
`compatibility/native.rs` has the separately requested Swift-capture rustfmt-only
hunks. Existing Swift test support/imports and the protocol test harness received
lint-only cleanup.

## Boundaries

Custom dialects/vocabularies, legacy recursive-reference keywords, current
portable regex restrictions, assertion-mode formats and active directional
annotations retain source-linked refusals. Static-only model layouts outside the
established profile still fail during planning. This is the checked v3 contract,
not full JSON Schema/OpenAPI conformance or a runtime resource-acquisition API.
