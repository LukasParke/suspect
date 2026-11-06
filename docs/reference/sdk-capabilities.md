# SDK generation capabilities

Matrix version: **`suspect-sdk-capabilities-v5`**.

SDK generation uses one pipeline: source-addressed `Contract` → typed native
plans → models, checked codecs, HTTP clients, packages and documentation →
ownership-aware artifacts. `gen sdk`, `gen session`, editor generation and
`diff sdk` use the same twelve-backend registry. All twelve language
features are now enabled by default; `gen profiles --format json` reports the
exact profiles compiled into the selected binary.

All profiles are **experimental and bounded**. Each has native base-profile
package/type/wire/docs evidence. Expanded protocol and scoped-schema capabilities
require their own native witnesses. [The live integration checkpoint](../internal/SDK-FULL-PLAN-STATUS.md)
records those results and the outstanding full-plan gates.

The earlier sealed five-language matrix verified all 208 criteria in its
[demo scope](../internal/SDK-DEMO.md). It remains a historical regression baseline. Fresh
twelve-language integrated acceptance, native costs and strict numerical M6
qualification are still required for the full plan.

## Native profiles

| Language | `gen sdk --profile` | Package/docs | Verified native scope |
| --- | --- | --- | --- |
| Python | `python-http` | Wheel-source package, `py.typed`, docstrings and Sphinx | Installed sync/async consumers of the canonical fixture and real-source operations, strict package/consumer mypy, examples and docs on Python 3.11/3.14 |
| Go | `go-http` | Standard-library module, Go doc and Sphinx | Independent consumers of the canonical fixture and real-source operations, native type controls, examples and docs on Go 1.23.12/1.27.1 |
| Swift | `swift-http` | Swift Package Manager, native comments and DocC | Canonical-fixture and real-source HTTP, exact codecs, cancellation, types, docs and shared vectors on Swift 6.3.3 and 6.0.3 + macOS SDK 15.4 |
| Rust | `rust-http` | Cargo package, Rustdoc and doctests | Installed consumers of the canonical fixture and real-source operations, codecs, native types, wire/security/docs tests on Rust 1.88 and current; dependency-free model defaults, opt-in custom HTTP and reqwest/rustls |
| TypeScript / JavaScript | `typescript-http` | ESM package with declarations, TSDoc and TypeDoc | Installed TS/JS consumers, strict positive/negative types, Node/browser and native docs; exact codecs and bounded request/response views |
| Java | `java-http` | Maven packages and Javadoc | Installed JDK21/25 consumers, immutable models, sync/CompletableFuture calls, exact codecs, native types/docs and source metadata |
| C# | `csharp-http` | NuGet, compiler XML documentation and native reference pages | .NET8/10 consumers, Task/cancellation, exact codecs, wire/resource controls, protocol and finite positional multipart tests |
| Kotlin | `kotlin-http` | Maven and Dokka | JDK21/25, Kotlin2.4.20/coroutines1.11/Dokka2.2; installed typed coroutine consumers and rich protocol matrices |
| Ruby | `ruby-http` | Gems, YARD, RBS and Steep | Ruby3.3.12/4.0.6 installed keyword-model consumers, exact codecs, expanded protocols and native docs/types |
| PHP | `php-http` | Composer, PHPStan and PHPDoc/reference | PHP8.3.32/8.5.8 installed typed consumers, expanded native protocol tests per tier and canonical integration checks |
| Dart | `dart-http` | pub packages and dartdoc | Dart3.9.4/3.13.3 VM and compiled JavaScript, strict analysis, native docs, hosted-pub installation and a real Chrome consumer |
| C++ | `cpp-http` | CMake, libcurl and Doxygen | Declared C++20/libcurl profile; installed CMake consumers, RAII/resource controls, exact codecs and independent protocol exchanges |

The real-source HTTP tests select `getCredits`, `createKeys`, `updateKeys`,
`listContainerFiles` and `getContainerFile` from the pinned OpenRouter public
specification. The independent shared canonical fixture
(`crates/suspect-codegen/tests/fixtures/m2/`) exercises create/update/list/get,
presence/null, exact numbers, unions, recursion and strict wire behavior.

The seven additional language base reports and subsequent protocol reports are
indexed in [the live checkpoint](../internal/SDK-FULL-PLAN-STATUS.md). Their profile promotion
does not claim completion of active schema-v2/native-dynamic work. Source-linked
admission remains specific to each adapter and executable profile.

## Contract and runtime scope

“Bounded” means admission succeeds only for an implemented representation and
validation profile. Unsupported selected features produce source-linked
diagnostics before artifacts are written.

| Area | Current scope |
| --- | --- |
| Input | Source-addressed local/split and explicitly acquired pinned/offline closures, exact values, stable physical URI/pointer identities and source spans; independent Lossless/Fast value parity checks |
| Dialects | Canonical OAS3.0/3.1/3.2 context and dialect-aware owned compilation; native acceptance is recorded per target. Ignored3.0 reference siblings cannot change the effective closure. OpenAPI2.0 family detection is not an SDK frontend |
| Values and models | Explicit absence/null, exact integer/decimal representations, source wire names, typed extras, finite recursive graphs, bounded native unions/intersections and encode-time validation of mutable models. `xml` annotations are retained as source provenance but never interpreted: they do not rename JSON properties or change codecs (`tests/sdk_xml_media.rs`) |
| Validation | Checked owned programs retain exact operands, source identity and finite evaluation policies. Three frozen, additive program envelopes ([ADR 0020](../adr/0020-validation-programs-are-versioned-and-frozen.md)): v1 static subset (`compile`), v2 scoped applicators (`compile_v2`) and v3 resources/dynamic references (`compile_v3`); a newer envelope is emitted only when the closure needs it. Incomplete evaluation cannot become successful validation |
| Resources | Contract indexes `$self`, `$id`, resource-local anchors and dynamic candidates without acquisition authority. Physical SourceIds remain separate from logical URIs. Runtime dynamic binding executes only through the explicit v3 program ([SDK-SCHEMA-RESOURCES.md](../design/schema-resources.md)) |
| Directions | TS/JS supports `oas31-required-applicability-v1` request/response views. Supplied fields remain validated and retained. Other HTTP profiles reject active directional projections explicitly |
| HTTP | Shared source-backed descriptors cover standard/custom methods, server choices, OR/AND security and explicit credential hooks, opt-in `sdk_defaults` OAuth token lifecycle and pagination ([SDK-GOLDEN-DEFAULTS.md](../guide/sdk-generation.md)), parameter encodings, exact/range/default dispatch, bodyless/binary/text/JSON media, headers/links, forms, multipart and item streams. Native adapters enumerate tested capabilities and refuse the remainder. XML media with a declared structure is refused with a source-linked diagnostic; XML media without a representable schema is carried only as bounded raw bytes. No XML codec is emitted |
| Interpretation | Ordinary declarations use standard semantics. Departures are explicit, versioned [compatibility profiles](#interpretation-profiles) carried through generation, sessions, CLI/editor and comparisons |
| Examples | Shared source/synthesized provenance, located invalid-example findings and schema-valid values lowered through the actual native symbol/codec plan. Missing required examples have explicit reasons |
| Artifacts | Deterministic complete file sets, owner conflicts, read-only drift checks, zero unchanged rewrites and removal of unchanged obsolete owned files; atomic replacement is per file |
| Iteration | One shared Contract per accepted snapshot, SHA-256 source/configuration identity, finite in-process caching, target-specific config reuse, missing-reference recovery, watch/check/preview |
| Compatibility | Separate source-aware native/wire findings through all twelve adapters, actual typed descriptors, generator/runtime asset provenance, explicit interpretation options and conservative uncertainty |

Schema fidelity includes runtime behavior. Model-only library plans deliberately
retain codec obligations; the profile's codec/HTTP plan supplies checked runtime
conversion. A supported neutral model does not imply every directional or HTTP
use is supported.

## Interpretation profiles

Ordinary OpenAPI semantics are the default. A non-standard idiom is accepted
only through a named, versioned compatibility profile
([ADR 0012](../adr/0012-dialect-quirks-need-a-compatibility-profile.md)). The set
is closed; an unknown name fails configuration before emission:

| Profile | Meaning |
| --- | --- |
| `legacy-binary-string-v1` | In OAS 3.1/3.2 binary media or parts, `type: string, format: binary` means raw bytes. JSON media keep their string models. |
| `oas30-nullable-in-3.1-v1` | The OAS 3.0 `nullable` keyword on 3.1/3.2 schemas adds or removes `"null"` from the same-object type. |
| `colon-path-parameters-v1` | A `/keys/:hash` segment becomes `/keys/{hash}` when a required path parameter of that name is declared; otherwise it is still refused. |
| `schemaless-stream-events-v1` | A `text/event-stream` response with no event structure is admitted as an untyped frame stream. |
| `all-of-scoped-mode-v1` | Schemas with `allOf` intersections use the scoped (checked) model/codec path instead of the unscoped projection. |

Select profiles with the repeatable `codegen --compatibility-profile NAME` flag
or a top-level `compatibility_profiles` array in a `codegen-session`
configuration (omitted means none). `codegen-profiles --format json` lists them
under `compatibilityProfiles`; discovery does not activate them. Profiles
participate in revision and cache keys, and a profile change between two
compared snapshots is reported as `wire-interpretation-profile-changed` and
`native-interpretation-profile-changed` unknowns even when native types match.

## Commands

```sh
suspect gen sdk crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile typescript-http --package-name @example/widgets \
  --package-version 0.1.0 --output generated
```

Package name/version are required configuration. Repeat `--operation-id NAME`
for exact source selection; omit it to attempt all outgoing operations. Add
`--check --format json` for read-only ownership/drift checking. Generation does
not invoke native package managers.

`--import-name` configures Python/Swift identities, JVM packages and native
Ruby/C#/PHP/C++ namespaces. TypeScript, Rust and Dart derive their import identity
from the package name; the canonical Go target uses package `sdk` within its
configured module. Those four targets reject an inapplicable import override.

Use [`gen session`](../guide/sdk-incremental-generation.md) for a multi-target JSON
configuration, watch and previews, and [`diff sdk`](../guide/sdk-compatibility-reports.md)
for snapshot comparison. `suspect gen` renders the `docs-md` preset or custom
manifests using the platform `IrSpec`; SDK packages and native docs consume
`Contract` through the native backends.

## Detailed boundaries

- [Current progress](../internal/SDK-PROGRESS.md), [generation plan](../internal/SDK-GENERATION-PLAN.md)
  and [full-plan exit gate](../internal/SDK-FULL-EXIT.md).
- Native HTTP: [Python](../sdk/python/http.md), [Go](../sdk/go/http.md),
  [Swift](../sdk/swift/README.md), [Rust](../sdk/rust/http.md),
  [TypeScript/JavaScript](../sdk/typescript/http.md).
- Additional native profiles: [Java](../sdk/java/README.md), [C#](../sdk/csharp/README.md),
  [Kotlin](../sdk/kotlin/README.md), [Ruby](../sdk/ruby/README.md), [PHP](../sdk/php/README.md),
  [Dart](../sdk/dart/README.md) and [C++](../sdk/cpp/README.md).
- [Native schema adoption](../design/schema-native-adoption.md),
  [canonical resources](../design/contract-resources.md) and
  [explicit interpretation profiles](../guide/sdk-interpretation-profiles.md).
- [Contract readers](../design/contract-readers.md), [owned validation](../design/owned-validation.md),
  [examples](../design/sdk-examples.md) and [artifact ownership](artifact-ownership.md).
- [Editor preview](../guide/sdk-editor-preview.md) and
  [session performance](../design/sdk-session-performance.md). The completed Mac baseline
  remains observational after timing-noise qualification failed.
- [Historical M0–M2 evidence](../internal/SDK-M0-M2-EXIT.md) and
  OpenRouter acceptance. Archived counts describe
  their pinned binaries and inputs, not current acceptance.
