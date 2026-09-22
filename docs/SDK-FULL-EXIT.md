# Full SDK plan acceptance

`xtask sdk-full` is the maintained integrated runner for **twelve SDK languages**
and the original plan's shared contract, acquisition, dialect, protocol, native
package, iteration, quality and measurement work. Run it after integration settles.
The implementation checks and follow-ups in
`target/sdk-full-runner-check-20260910-01/`,
`target/sdk-full-runner-check-20260910-02/`,
`target/sdk-full-runner-check-20260910-03/`,
`target/sdk-full-runner-check-20260910-04/`,
`target/sdk-full-runner-check-20260910-05/`,
`target/sdk-full-runner-check-20260910-06/`,
`target/sdk-full-runner-check-20260910-07/`,
`target/sdk-full-runner-check-20260910-08/`,
`target/sdk-full-runner-check-20260910-09/`,
`target/sdk-full-runner-check-20260910-10/`,
`target/sdk-full-runner-check-20260910-11/`,
`target/sdk-full-runner-check-20260910-12/`,
`target/sdk-full-runner-check-20260910-13/`,
`target/sdk-full-runner-check-20260910-14/`,
`target/sdk-full-runner-check-20260910-15/`,
`target/sdk-full-runner-check-20260910-16/`,
`target/sdk-full-runner-check-20260910-17/` and
`target/sdk-full-runner-check-20260910-18/` exercise the runner; they are not native
acceptance reports. Earlier source snapshots, reports and binaries retain their
recorded scope and identities.

The historical `sdk-m3-m6` command retains its five-backend scope, its 208-gate
demo and 212-gate strict inventories, and its original performance policy. Its
source changes for this integration expose existing mechanics to the new module;
they do not expand historical reports or turn the completed native **base**
matrices into protocol acceptance.

## Entry point and outcomes

Registration is in `xtask/src/main.rs`; the independent module is
`xtask/src/sdk_full.rs`. API:

```rust
pub(super) fn run(raw: &[String]) -> anyhow::Result<()>
```

```sh
cargo run --locked --offline -p xtask -- sdk-full \
  --source /Users/luke/github/openrouter-web \
  --editor-host-tools /absolute/editor-host-tools.json \
  --out target/sdk-full-runner-NEW

# Explicitly accept functional evidence while keeping numerical work pending:
cargo run --locked --offline -p xtask -- sdk-full \
  --source /Users/luke/github/openrouter-web \
  --editor-host-tools /absolute/editor-host-tools.json \
  --out target/sdk-full-runner-functional-NEW --functional-only

# No native generation, Cargo build, tool activation or measurement:
cargo run --locked --offline -p xtask -- sdk-full --list-stages
cargo run --locked --offline -p xtask -- sdk-full --list-stages --functional-only
cargo run --locked --offline -p xtask -- sdk-full --check-syntax
```

`--out` must be a nonexistent directory with an existing parent under this
workspace's `target/`. Attempts never resume into or overwrite an existing report
or scratch directory. Scratch is separately reserved beneath the caller's
`${TMPDIR%/}/opencode/`, using the report name and absolute-output-path hash. Its
canonical ancestors must contain neither `Cargo.toml` nor Git metadata.

`--check-syntax` validates the maintained inventory, matches the compiled runner's
embedded source bytes to its three current source files, and checks required
suite/doc/native-host source paths and exact named native declarations. It reports
`missingSuiteSources`, `missingNamedWitnesses` and `missingSupportSources`, and
exits nonzero for missing prerequisites or a stale runner. It does not assert that
the actively edited aggregate codegen crate builds or that native tests passed.

| Result | Meaning |
| --- | --- |
| Strict, every required gate met | `complete: true`, `profileComplete: true`, exit 0 |
| Strict, no usable qualified performance plan | Four numerical rows remain unmet; `complete: false`, exit nonzero |
| Functional-only, functional gates met | `profileComplete: true`, **`complete: false`**, `status: functional-passed-numerical-pending`, exit 0 |
| Any required functional/native gate missing or failed | `profileComplete: false`, exit nonzero in either profile |
| Interrupted or unsealed attempt | Incomplete evidence |

Functional-only removes **only** the four numerical requirements from the exit
decision. They remain visible in the inventory and report. It does not label the
original plan complete or the host calibrated. `releaseReady` and
`sdkReleaseReady` remain false; selected-operation SDK acceptance does not establish
whole-corpus validity or a publishing/distribution release.

Terraform is a separate stretch artifact built on the generated **Go SDK**. It is
not a thirteenth SDK language and is not in the core SDK gate inventory.

## Fixed stage inventory

The inventory comes from the same functions used by execution. At this checkpoint
it contains **924 unique stages**, **920 functional requirements** and the four
original numerical M6 requirements. It includes **296 frozen-harness invocations**
and their separate named-execution proofs. These are requirement counts, not
passing tests or completed language claims. `--list-stages` provides exact argv,
environment selectors, criteria, targets and source operation selection.

The prior **643/639/166** and **655/651/172** inventories and their source/binary
snapshots remain historical. The current inventory adds source-driven native v2,
core applicator/resource tests, a complete frozen-library test partition, explicit
default-feature twelve-profile discovery, and both real installed editor scenarios.
`libraryNativeSelections` and `editorNativeContract` expose the maintained exact
names, tiers, claims and screenshots in the machine-readable inventory.

There is no `--stage`, caller-supplied command list, assertion map, or generic JSON
pass-through gate. Missing future suites remain required. A successful build that
only emits a test executable does not count as executing that suite.

| Family | Required execution/evidence |
| --- | --- |
| Source/provenance | Actual dirty-tree census/copy, HEAD/status/index/binary diffs, embedded runner-source identity, four tracked input hashes, final source/tool/input integrity |
| Tools/builds | Explicit installed native floor/current tools, private copied caches, fresh full-feature CLI build, separately frozen test binaries |
| Default profiles | Separate default-feature CLI build/freeze/discovery must advertise exactly all twelve profiles; explicit all-feature native builds remain required |
| `twelve-five-op-*` | Real CLI preview, generation and unchanged check from one twelve-target session; all twelve actual package roots are required |
| `cli-twelve-*`, `twelve-profile-coverage` | Exactly twelve discovered backend/directory identities and twelve retained native compatibility snapshots; no unknown/breaking self-comparison |
| `package-*`, `install-*`, `installed-*-evidence` | Native archives, builds, installs/resolution, executable installed consumers, native docs, compiler/member/source linkage and retained artifacts |
| Baseline five languages | All existing M3 model/JSON/validation/codec/HTTP/package/type/docs/browser/wire/cancellation/resource gates; both native runtime tiers and shared Swift validation |
| New-language base | `java_sdk`, `csharp_sdk`, `kotlin_sdk`, `ruby_sdk`, `php_sdk`, `dart_sdk`, `cpp_sdk`, including ignored native package tests |
| Canonical integration | All seven `*_integration` suites; twelve-backend `sdk_generation_options`; `typescript_protocol_compatibility`, `sdk_compatibility_context` and exact Ruby credential-capture controls |
| Protocols | All twelve languages: eleven `*_protocol` suites plus exact expanded-protocol witnesses in Ruby's per-tier `ruby_sdk` suite; required named native execution |
| Core acquisition | `pinned_acquisition`, `pinned_provider`, `pinned_transport`, including real loopback/TLS/hostile-environment controls |
| Core contract/dialects | IR contract/readers/references/`pinned_contract`/scalars/HTTP/`contract_oas32`/`contract_resources`/`contract_dual_role_scope`; owned dialect/normative/`owned_context_regressions`/numeric/reference/URI/budget, `owned_applicators`, `owned_applicator_conformance` and `owned_resources`; source-linked declaration checks |
| Scoped native v2 | All twelve language inventories below; maintained 32-case source fixture through `compile_v2`, native scope/budget guards and installed SDK/type/example/doc witnesses |
| Models/examples/DX | Existing names, directional/intersection, source/example and native quickstart suites; floor/current `sdk_dialect_models` and `python_null_models`; shared `http_protocol` and `protocol_examples` |
| `twelve-pinned-*` | Acquire the pinned local public-input copy, delete that private entry, then run a twelve-target cache-only watch and compatibility comparison; warm compile/render/write counts must be zero |
| CLI/session | Frozen process suites for generation, ownership, entry loading, sessions, compatibility, acquisition/pinned generation and validation, including `sdk_protocol_options` |
| Editor | Existing compiled protocol/UI/process suites and the maintained 13-test matrix, plus closed native-host input guards and both real installed VSIX lifecycle/commands scenarios with all twelve expected and observed profile IDs |
| Native costs | Maintained `sdk_native_measurements` suite plus all 92 actual per-language/tier build/import/codec/request rows, package sizes, command/log/tool/source identities; see the integration contract below |
| Measurement tooling | Actual Python collector/comparator/adapter regression tests, with nonempty complete unittest output; synthetic test fixtures are not measurements |
| Quality | Offline locked workspace tests, all-feature/all-target warnings-denied Clippy, warnings-denied Rustdoc, formatting, staged/unstaged whitespace checks |
| Numerical M6 | `performance-small`, `performance-split-recursive`, `performance-openrouter`, `performance-compare`; fixed, provenance-verifying `accept.py` import |

The five real public operation selectors are `getCredits`, `createKeys`,
`updateKeys`, `listContainerFiles`, and `getContainerFile`. Original M2, shared
runtime/protocol vectors and independent native wire/type/resource cases remain
separate required suites. Original OpenRouter validation findings remain visible
in validation tests; `currentFullCorpusAcceptance` is not inferred from selected
SDK operations.

Whole integration harnesses use `--include-ignored --show-output --test-threads=1`
and require a nonempty, complete libtest summary and named passing results. Early
skip messages, absent tools, ignored native cases and zero tests fail. Exact
library selections use the explicit census accounting below rather than treating
filtered tests as executed. `pinned_transport::environment_child` has separate
subprocess accounting: its two real
parent tests invoke it themselves with incompatible hostile environments. The
runner requires the parent suite and that exact ignored subprocess-entry row;
it does not run the helper bare without its required environment.

The required `core-suspect-ir-contract_dual_role_scope` harness and its separate
`-execution` proof run all ten exact maintained dual-role regression witnesses.
Coverage includes both readers, equivalent JSON/YAML, reversed HTTP/schema
registration, single-role metadata, malformed references and genuine ambiguity.
The fixed suite is part of the IR binary build/freeze inventory and source-name
readiness check. Its owner receipt documents coherent resource/base/address
identity with an optional schema-root hint; invalid scopes and conflicting
identities remain failures. Main's exact compatibility-context witness passed
unmodified, as recorded in
`target/sdk-ir-dual-role-20260910-01/DISPOSITION.md`.

C# base and protocol suites internally pin both SDKs with
`global.json`/`rollForward: disable` and run `net8.0`/`net10.0` consumers. Python's
protocol suite internally installs and executes both 3.11 and 3.14. C++ has one
declared verified C++20/libcurl profile, rather than an invented second platform.

Ruby **3.3.12 / 4.0.6** each executes the complete `ruby_sdk` harness once, through
`floor-ruby_sdk` and `current-ruby_sdk`. The four existing native base witnesses
remain mandatory. The same invocation additionally requires these exact passing
native protocol tests:

```text
native_expanded_protocol_wire_parts_streams_and_types
native_additional_openrouter_binary_and_delete_operations
native_oas30_nullable_reference_siblings_and_binary
```

Together they cover expanded wire/parts/streams/types, additional actual
OpenRouter binary/delete operations and OpenAPI 3.0 nullable/reference-sibling/
binary semantics. Ruby has no separate `ruby_protocol` harness requirement or
empty compatibility shim. A base-only result cannot satisfy either combined
Ruby gate; losing any base or protocol witness fails its execution proof.

Ruby's ordinary `ruby_compatibility_credentials` target runs once through
`core-suspect-codegen-ruby_compatibility_credentials` and its execution proof. All
six exact host controls are required:

```text
source_relocation_alone_preserves_native_credential_equality_and_locations
relocated_oauth_oidc_and_api_keys_keep_typed_values_and_url_bases
credential_names_types_attachments_alternatives_and_permissions_still_change
oauth_flows_endpoints_and_oidc_discovery_changes_survive_capture
effective_server_base_changes_remain_real_wire_changes
request_name_tightening_remains_wire_change_without_credential_noise
```

These exercise public Contract/native capture/comparison. Typed credential and
scope projections separate physical provenance from native equality while keeping
Locations, actual URL/base semantics and arbitrary literal scope values. Genuine
credential, endpoint and request-constraint changes remain observable. The owner
receipt at `target/sdk-ruby-credential-compatibility-20260911-02/report.json`
records six focused controls, fifteen common-context tests and the existing Ruby
constructor control passing. Its all-twelve CLI replay removes five false Ruby
changes while retaining the two real potentially-breaking wire/TypeScript
findings; the expected replay exit is 1. That replay is separate evidence, and
its frozen binary is not substituted for the runner's fresh source/CLI build.

Python's floor/current `python_null_models` suites require the exact native test
`null_only_fields_aliases_containers_and_unions_preserve_native_typing_and_values`.
They receive the selected `SUSPECT_PYTHON_BIN`, `SUSPECT_PYTHON_TOOLS` and a private
artifact directory. Null-only values/aliases, containers, union typing, runtime
values and positive-controlled negative mypy consumers remain required while
Main integrates the typing fix and schema applicator v2 work.

Both Swift protocol tiers require these exact passing native witnesses:

```text
native_protocol_spm_types_wire_stream_lifetimes_and_docs
native_remaining_standard_custom_query_positional
```

The complete suite still executes with all native opt-ins. The additional witness
covers the remaining standard/custom/query/positional cases in that suite; it is
not substituted by the original protocol test's passing result.

## Source-driven scoped validation and library partition

| Language | Maintained v2 harness | Execution |
| --- | --- | --- |
| TypeScript | `typescript_applicators` | One explicit internal Node22/24 × TS5.5/5.9 matrix; all seven source/profile/scope/capture/example/installed-SDK/Chromium witnesses |
| Rust | `rust_validation_v2` | Rust 1.88/stable; all six native vector/official/depth/codec/resource/HTTP-example witnesses |
| Python | `python_applicators`, `python_schema_v2` | One matrix invocation each; runtime and installed-SDK witnesses execute both Python versions from the pinned PATH/UV selectors, with `SUSPECT_PYTHON_SCOPED_VERSIONS=3.11,3.14` |
| Go | `go_schema_v2` | Floor/current Go selectors; six exact source-binding/fence/vector/SDK/recursion/document-base witnesses; `SUSPECT_SPHINX_PYTHON` binds the pinned Sphinx interpreter so rendered docs execute |
| Ruby | `ruby_schema_v2` | Floor/current Ruby selectors; runtime, retained descriptors and installed SDK witnesses |
| Swift | `swift_validation_v2` and library `swift_sdk::validation::v2_tests::native_evaluated_applicator_vectors` | Both declared compiler/SDK tiers |
| Java | `java_schema_v2` | JDK21/25; `native_schema_v2_vectors` and `native_schema_v2_sdk_operations` |
| Kotlin | `kotlin_validation_v2` | JDK21/25 selected through `SUSPECT_KOTLIN_JAVA_HOME` |
| C# | Library `csharp_sdk::validation_tests::{native_scoped_source_vectors,native_scoped_limits_and_admission,native_scoped_sdk_packages}` | Each selection runs its internal exact .NET8/.NET10 matrix once |
| Dart | Library `dart_sdk::validation_tests::native_v2_source_vectors` and `dart_sdk::sdk_v2_tests::native_v2_sdk_operations` | Both Dart tiers; `SUSPECT_DART_REPO_ROOT` is the actual execution copy, plus explicit compiler and retained gate root |
| C++ | Library `cpp_sdk::v2_tests::{native_v2_independent_32,native_v2_scope_resource_edges,native_v2_sdk_operations}` | One declared C++20/libcurl profile |
| PHP | Library `php_sdk::tests_v2::{native_v2_source_vectors,native_v2_sdk_packages_models_codecs,native_v2_scope_resource_controls}` | Both PHP tiers with PHP/Composer/PHPStan selectors |

The shared source fixture is
`crates/suspect-schema/tests/fixtures/owned-applicators-v2.json`; its version, 32
unique cases, source JSON and expected outcomes are verified and hash-pinned.
Native gates compile these sources through the real `compile_v2` API. A stored
executable program or earlier native report cannot replace that source seam.

Core `owned_resources` additionally has a backward-compatibility test that
recompiles those source schemas at their original logical URIs and compares the
published v2 program bytes. Only that core test receives a copied, immutable
`target/sdk-schema-applicators-executable-v2.json`, pinned to
`0dcae95d213030abd6fc14ebca8d3f4bf6ce9f5bd5382730bfa403f91be53056`.
The original is read-only; its new evidence copy is
`inputs/core/published-applicators-v2.json`. It is not a native execution oracle.
The resource/dynamic core tests do not establish native v3 capability flags;
native v1/v2 requirements and located v3 fences keep their own scope. Dart's
separately witnessed `DocumentRelativeServers` capability requires
`dart_sdk::document_tests::native_document_relative_servers` on both Dart tiers,
with the same explicit execution-copy/compiler/gate-root selectors. Both PHP
protocol tiers additionally require
`native_document_relative_servers_keep_physical_bases`. These named native
witnesses, rather than shared HTTP/resource host results, justify their scope.
C# also requires `csharp_sdk::server_tests::native_document_relative_servers`
once, with its internal exact .NET8/.NET10 matrix and `SUSPECT_DOTNET_BIN`.
C++ requires `cpp_sdk::server_tests::native_document_relative_servers` once in
its declared C++20/libcurl environment. Go requires
`go_schema_v2::native_physical_document_base_redirect_and_encoded_path_witness`
on both Go tiers. These retain each owner’s physical-retrieval/redirect/logical-URI
and encoded-path distinctions; they are not inferred from shared resource tests.
Ruby's separate `ruby_document_servers` suite runs on both tiers and requires
`physical_base_metadata_is_separate_and_schema_resource_fences_stay_closed` plus
`installed_gem_uses_effective_physical_document_server_bases`.

Dart's separate V3 runtime and installed SDK proofs are required on both tiers:
`dart_sdk::v3_tests::native_v3_source_resources` and
`dart_sdk::v3_sdk_tests::native_v3_sdk_operations`. Both use the actual execution
copy via `SUSPECT_DART_REPO_ROOT` and the declared compiler/gate-root selectors.
Dart's finalized public entrypoints retain their signatures, prefer `compile_v2`
for ordinary closures and use explicit `compile_v3` for admitted resource closures;
its final native package-byte parity and old-version preservation receipts remain
separate from this runner-only integration.
Further native V3 SDK/operation gates need their finalized owner contracts;
unassigned new ignored library tests fail the census rather than disappearing
from acceptance. Go's finalized contextual-codec, installed SDK, depth/concurrency
and aggregate native suites are now required as described below; the earlier
partial official-case receipt retains its narrower historical scope.
Java's fixed V3 suite `java_schema_v3` is required on JDK21/25 with
`native_schema_v3_resource_conformance` and `native_schema_v3_sdk_operations`.
The finalized owner receipt uses explicit `plan_sdk_with_protocol_v3`,
`validation::plan_validation_v3` and `protocol::capabilities_v3`; canonical
dispatch/capture use the explicit V3 entrypoint and backend Java options. Ordinary
V1/V2 plan/artifact equality has its own host witness in the complete suite.
The focused `java_aggregate_examples` suite additionally runs on JDK21/25 with
`native_declared_aggregate_examples`, plus exact grouping/provenance and source
arity-refusal tests. It requires the changed `validated_aggregates` /
`ExampleEntry.part_position` seam: empty optional groups, dynamic extras, repeated
values, mixed text/JSON occurrences, sparse prefixes/tails and byte findings.
The owner's retained proof is 31 offline examples and six real example calls per
tier; the runner selects each JDK through `JAVA_HOME` / `SUSPECT_JAVA_HOME`, uses
the pinned `SUSPECT_MAVEN_BIN` and a private copy of the declared Maven cache.
Ruby's finalized `ruby_schema_v3` suite is also required on both Ruby tiers, with
these exact witnesses:

```text
source_driven_official_v3_resources_scopes_and_guards
installed_resource_sdk_models_types_examples_and_wire
resource_native_descriptors_and_profile_selection_preserve_physical_identity
```

Its native source/guard and installed SDK gates, together with the ordinary
descriptor/profile-selection test, cover the promoted existing public entrypoint.
They do not replace Ruby's base, protocol or V2 requirements.
C++'s finalized public V3 proof requires these three library selections once each
in its declared C++20/libcurl environment:

```text
cpp_sdk::v3_tests::native_v3_official_dynamic_ref_44
cpp_sdk::v3_tests::native_v3_scope_and_resource_controls
cpp_sdk::v3_tests::native_v3_sdk_operations
```

The existing public `plan_sdk` selects explicit `compile_v3` for actual resource
closures and keeps ordinary V1/V2 selection. Its source-driven closed-provider,
scope/budget/malformed, installed operation/type/example/doc proofs remain
distinct from the preserved V2 and physical-document-base gates.
Kotlin's finalized resource and document-server suites run under each explicit
JDK tier: `kotlin_validation_v3::{native_resource_44_vectors,native_resource_sdk_operations}`
and `kotlin_protocol_documents::native_physical_document_servers` (plus its
ordinary physical/logical-context witness). `kotlin_validation_integration` and
`kotlin_validation_resource_integration` are required host integration suites.
The existing canonical `plan_sdk_with_profiles` dispatch selects necessary V3
closures while preserving ordinary V1/V2; no new shared dispatcher is assumed.
PHP's finalized V3 library selections are required on both existing PHP tiers:
`php_sdk::tests_v3::{native_v3_source_fixtures,native_v3_scope_resource_controls,native_v3_sdk_packages_models_codecs}`.
They retain the explicit PHP/Composer/PHPStan selectors, source-driven official
and independent resource controls, and installed public SDK proofs. Ordinary
successful `compile_v2` paths and full V1/V2 artifacts keep their existing scope.
Python's finalized `python_resources`, `python_schema_v3` and
`python_document_servers` suites each run once because their native tests execute
both 3.11 and 3.14 internally. Required names are
`python_v3_executes_official_sources_dynamic_scopes_guards_and_exact_budgets`,
`python_resource_admission_keeps_ordinary_programs_and_typed_capture`,
`installed_python_v3_dynamic_operations_native_examples_types_and_sphinx`, and
`installed_python_physical_document_servers_preserve_redirects_overrides_and_encoded_paths`.
The existing planner dispatch and ordinary V1/V2 paths remain source-driven;
Main's separate floor/current null-only regression stays required.
C#'s finalized V3 library selections run once each because every test internally
executes both SDK8.0.424 and SDK10.0.400 with exact `global.json` pins and
`rollForward: disable`:

```text
csharp_sdk::resources_tests::native_resource_source_vectors
csharp_sdk::resources_tests::native_resource_scope_and_admission
csharp_sdk::resources_tests::native_resource_sdk_packages
```

All native commands use `SUSPECT_DOTNET_BIN`. The existing public planner/codec/
client APIs explicitly select V3 for resource-bearing closures and retain ordinary
V1/V2 programs. These source-driven runtime, scope/admission and installed SDK
proofs are distinct from the earlier C# V2 and physical-server requirements.

The separate `matrix-csharp_positional` suite and execution proof require all
seven maintained positional projection, source guard, options/capture and native
package witnesses:

```text
positional_projection_uses_only_reachable_part_and_header_codecs
undefined_positional_styles_are_located_refusals
ignored_positional_styles_preserve_native_content_profile
ignored_positional_styles_cannot_hide_invalid_content_or_metadata
documented_httpclient_method_case_refusal_is_source_located
canonical_generation_options_are_identical_in_generation_and_snapshot_capture
native_positional_multipart_matrix
```

Its one native selection internally installs and executes both SDK8.0.424 and
SDK10.0.400, using `SUSPECT_DOTNET_BIN` and exact `global.json` pins. The repaired
host seams retain the active `multipart/form-data` source/span refusal, consume
the shared `ignoredMultipartEncoding` fixture, require identical content-only
codec roots/program/generated C# sources, and preserve actual-content and
malformed-metadata refusals. Generated-source identity is a host requirement;
the existing installed MIME/type/docs/cleanup matrix remains a separate named
native requirement within the same complete harness.

TypeScript's finalized `typescript_resources` suite runs once as
`matrix-typescript_resources`, requiring all nine exact witnesses:

```text
v3_metadata_and_dynamic_ops_cannot_enter_older_or_mismatched_envelopes
source_driven_v3_executes_all_44_official_cases_from_closed_supplied_documents
native_v3_scope_identity_lookup_budgets_and_malformed_metadata_controls
resource_selection_keeps_base_profiles_and_source_linked_refusals
resource_capture_uses_typed_context_graphs_and_keeps_physical_locations_separate
v3_example_admission_retains_dynamic_invalidity_and_declared_source
installed_v3_sdk_preserves_dynamic_models_exact_values_examples_and_native_docs
physical_document_server_bases_preserve_redirect_provenance_overrides_and_encoded_paths
browser_v3_executes_official_sources_dynamic_sdk_calls_and_contextual_codecs
```

Both TypeScript suites bind `SUSPECT_DOCS_NODE={node22}`,
`SUSPECT_NODE24_BIN={node24}`, the pinned `SUSPECT_CHROMIUM`, and their respective
`SUSPECT_TYPESCRIPT_V2_MATRIX=1` / `SUSPECT_TYPESCRIPT_V3_MATRIX=1` selectors. Their
`*_ARTIFACTS` roots are separate retained private directories under
`{work}/native/typescript/matrix/`. The maintained helpers execute both Node and
compiler tiers internally, including separate installed compiler-tier packages.
V2's former `floor-typescript_applicators` / `current-typescript_applicators`
invocations and execution proofs are superseded by `matrix-typescript_applicators`
and its execution proof. All seven finalized V2 witnesses are mandatory, including
native browser execution; the prior three required names are included. Historical
checkpoint inventories retain their original identities.

Existing public TypeScript entrypoints select `compile_v3` only for a
resource-bearing source closure, keeping ordinary V1/V2 programs. Its separate
physical-server and contextual-codec tests are required. The earlier shared
ignored-multipart content-descriptor prerequisite is now closed by Main's accepted
shared fix and the existing TypeScript owner's installed native receipt. The
defensive `http-typescript-multipart-content-plan-required` guard remains.

### Ignored multipart Encoding native and source gates

`matrix-typescript_multipart_ignored_encoding` and its execution proof require
`ignored_multipart_styles_preserve_content_in_installed_requests_and_responses`.
The complete one-test harness runs once: it internally installs TypeScript
5.5.4/5.9.3 packages and executes each on Node22.23.1/24.21.0. It binds
`SUSPECT_DOCS_NODE={node22}`, `SUSPECT_NODE24_BIN={node24}` and
`SUSPECT_PROTOCOL_ARTIFACTS={work}/native/typescript/matrix/ignored-encoding`.
The artifact parent belongs to the new attempt; the helper retains distinct
native package/consumer/wire directories beneath it. Offline npm/Cargo caches,
Node22's npm CLI and the reviewed TypeScript floor manifests are prerequisites.

The existing `http_protocol` suite additionally requires these four exact source
controls from the same accepted handoff:

```text
ignored_multipart_style_fields_preserve_explicit_and_default_content_plans
ignored_multipart_style_cannot_bypass_the_actual_content_codec_or_metadata_validation
active_form_styles_keep_their_oas31_whole_property_and_oas32_item_semantics
older_oas_versions_do_not_gain_invented_positional_or_named_mixed_encodings
```

They use `fixtures/http-protocol-v1.json#ignoredMultipartEncoding` under
`crates/suspect-codegen/tests`. The private shared `PartContext` keeps actual
declared/default Json/Text/Binary content when Encoding fields are ignored for other
multipart media, while form-urlencoded/form-data retain their active rules. Public APIs,
physical warning/source spans, required headers, whole-array positional parts and
malformed-metadata/content refusals retain their contracts.

`target/sdk-typescript-ignored-encoding-20260910/acceptance-01.json` records the
existing native proof: 11 real exchanges, 11 request controls and nine response
controls per compiler/runtime combination. Shared 46/46, example 20/20 and library
Clippy receipts are in `target/sdk-http-protocol-ignored-encoding-20260910/`.
The bounded 40-file source seal and reconciled documentation digest remain in
`target/sdk-http-protocol-ignored-encoding-release-20260910-01/`; this source-only
capture is not a new runtime or full-workspace acceptance report. Runner13 and the
separate later-release intake retain their historical bytes. The new runner
registration executes fresh frozen source when Main launches integrated acceptance.

### Complete frozen-library accounting

The full-feature `suspect_codegen` library binary is frozen once. Two `--list`
commands inventory all compiled tests and its ignored native tests. The latter
set must equal the maintained 33-name library-native catalog exactly: an absent
declared witness or newly ignored unassigned test blocks acceptance.
The received credential-env native selectors for all twelve adapters are assigned
below. Additional native/helper selectors still require an explicit final owner
handoff; the census continues to reject every unassigned ignored library test.

`library-codegen-host` executes every nonignored library test once, checking its
named passing set, zero filters, and the exact ignored native set. The 50 native
selections then run under their own language/tier environments:

```text
<frozen library> --include-ignored --exact <maintained full test name> \
  --show-output --test-threads=1
```

Each selection must execute exactly its named test, with one pass, zero ignored,
zero failures, and exactly `all_census_tests - 1` filtered tests. The final
`library-codegen-coverage` gate requires every assigned selection's execution proof.
Thus Dart floor tests cannot inherit only the current/Swift environment, C#'s
internal two-tier matrix is not duplicated per external tier, and C++ receives its
declared environment. Both original Swift v1 native-vector stage IDs remain with
their exact floor/current selectors. The historical M3/M6 runner is unchanged.

### Final Rust and Swift resource witnesses

The finalized Rust resource suites run on both existing Rust 1.88/stable tiers.
All five native names and their five maintained ordinary source/profile guards
are required across the three complete harnesses:

| Harness | Required native witnesses |
| --- | --- |
| `rust_validation_v3` | `installed_rust_v3_executes_all_official_dynamic_ref_source_fixtures`, `installed_rust_v3_scope_cycles_fallbacks_and_work_failures_are_independent`, `installed_rust_v3_default_depth_scope_restore_and_annotation_isolation` |
| `rust_protocol_v3` | `installed_rust_v3_sdk_preserves_dynamic_models_examples_and_real_wire` |
| `rust_protocol_resources` | `installed_rust_relative_servers_use_effective_physical_documents` |

`SUSPECT_RUST_V3_TARGET`, `SUSPECT_RUST_V3_HTTP_TARGET` and
`SUSPECT_RUST_RESOURCES_TARGET` select separate private per-tier Cargo targets.
The existing `SUSPECT_NATIVE_RUST_TOOLCHAIN` / `RUSTUP_TOOLCHAIN` selections apply.
Official and deferred dynamic/unevaluated fixtures are compiled from maintained
source with the real compiler and a closed provider. Runtime/model helper sources,
the six source fixture files and resource documentation are readiness prerequisites.
The owner evidence records all five native witnesses on both toolchains; ordinary
model/codec byte fences retain their own scope. Main's earlier
`rust-v3-canonical-byte-01.log` remains the original red receipt. The later
`all-twelve-bridge-01` passes strict ordinary Rust V2/V3 full-file parity and the
all-twelve resource generation/Session/capture bridge; canonical Rust dispatch now
uses `plan_http_v3` with aligned native capture.

Swift adds these exact library selections on both existing Swift/compiler/SDK
tiers, each with `SUSPECT_SWIFT_V3_ROOT={work}/native/swift/<tier>/v3`:

```text
swift_sdk::validation::v3_tests::native_resource_dynamic_source_vectors
swift_sdk::resources_tests::native_installed_v3_resources_codecs_types_wire_and_docs
swift_sdk::resources_tests::native_installed_physical_document_servers
```

The test-only V3 support helper and both native Swift consumer sources are
required. The owner records 93 runtime checks per tier plus installed SDK/type/
wire/example/DocC and physical-server proofs. The public `plan_sdk` API is retained.
Main's all-twelve bridge includes Swift generation, cold/warm Session
identity/work checks and typed capture in the existing `sdk_generation_options`
suite. Swift's declared-aggregate handoff is now accepted on both published tiers.
The exact library selection
`swift_sdk::aggregate_examples_tests::native_installed_declared_aggregate_examples`
runs under `floor-swift-v3-aggregate-examples` and
`current-swift-v3-aggregate-examples`, each with its execution proof and the existing
Swift/SWIFTC/DocC/SDKROOT plus private `SUSPECT_SWIFT_V3_ROOT` environment. Its native
fixture and driver/support hashes match both retained proofs: four consumer tests,
one generated example and DocC per tier. The older primary `swift_sdk.rs` digest
is explicitly reconciled to its subsequent configured-only credential-env changes
in Main's `swift-aggregate-owner-receipt-01` receipt; default None behavior and the
native fixture/driver remain unchanged. The compiled census still requires the
exact complete ignored set and is not satisfied merely by this registration.

Both existing `go_models` tiers additionally require
`native_pattern_properties_validate_matching_values_without_stripping_extras`,
`pattern_properties_use_scoped_codecs_and_retain_model_obligations` and
`unimplemented_go_shapes_block_artifacts_with_source_locations`. The native
pattern gate receives the selected Go toolchain and preserves unmatched/null
extras, exact integers, mutable-encode checks and physical source/instance paths.

### Final Go resource and aggregate witnesses

The complete `go_schema_v3` harness runs on both Go **1.23.12 / 1.27.1**, requiring
all six exact source/native witnesses:

```text
official_44_unmodified_resource_cases_execute_with_closed_supplied_documents
malformed_v3_contexts_never_emit_and_old_envelopes_reject_resources
independent_dynamic_scope_branch_cycle_budget_and_annotation_controls
native_codec_boundaries_preserve_outer_dynamic_context_and_base_profile_selection
installed_resource_sdk_keeps_typed_fields_dynamic_wire_checks_and_examples
native_distinct_resource_depth_and_concurrent_contexts_are_bounded
```

The separate `go_examples_aggregate` harness also runs on both tiers and requires
`declared_form_and_positional_groups_keep_items_extras_and_absence`. Each of the
four harness invocations has a separate named-execution proof. Both
`SUSPECT_GO_TOOLCHAIN` and `GOTOOLCHAIN` select the exact tier explicitly, so the
helpers' default two-toolchain loops execute only the selected tier per invocation.
`SUSPECT_SPHINX_PYTHON={python-tools}` enables the maintained rendered-doc gate.
The helper intentionally selects Go1.23.12 within Sphinx; direct `go doc`, installed
consumers and native SDK/codecs use each selected tier. Sphinx reflection is not
claimed as a separate current-toolchain proof.

The finalized owner contract covers the 44 unmodified official cases, 16
independent controls, outer-context codec decode/mutable encode and independent-
root fallback, installed SDK/type/wire/example/Sphinx work, `-race`, the 550-resource
chain with the 512-depth failure, concurrent contexts and grouped form/positional
recipes. Existing public Go entrypoints explicitly compile V3 only for resource
closures and preserve ordinary V1/V2 programs. The three native-witnessed resource/
dynamic/physical-server capabilities are admitted. Source readiness requires
`go_validation/resources.go` (emitted as `go/validation_resources.go`) and the
retained V2 `scoped.go` / `scoped_pattern.go` assets. Existing m3 binding assertions
remain part of their complete required suites.

`target/sdk-go-native-schema-evidence-20260910/receipts.json` explicitly records
missing earlier managed Cargo logs as `retainedFile: null`, using observed
completion notifications; depth/grouped/Clippy logs are retained. Those receipts
identify the finalized selector/API contract. Integrated execution captures new
frozen-harness stdout/stderr in its own attempt; old binaries/logs are not accepted
as execution substitutes, and missing child output is never reconstructed.

### Credential environment V1 native receipts

Dart's complete `dart_credential_env` suite runs on both 3.9.4/3.13.3 tiers and
requires all six maintained names:

```text
no_policy_output_bytes_are_preserved
policy_binds_actual_source_names_and_keeps_semantics_relocation_independent
canonical_policy_capture_and_session_identity_are_source_bound
native_credential_env_controls
native_credential_env_constructor_compatibility
native_openrouter_credential_env
```

Each invocation gets the existing `SUSPECT_DART_BIN`, execution-copy
`SUSPECT_DART_REPO_ROOT` and private `SUSPECT_DART_GATE_ROOT`, plus the pinned Node22
PATH for the JS consumers. `OPENROUTER_WEB_ROOT={out}/inputs` selects the read-only
pinned source copy for actual `getCurrentKey` and `getCredits` schemas. These
selectors exercise installed VM/JS controls, examples and zero-warning dartdoc;
the four Chrome153 pages are a separate retained owner browser receipt. Source
readiness requires the shared bound-plan module, five Dart environment assets,
three native consumers, test support and the complete no-policy hash fixture.
The preserved pre-policy map and fixture are identical and have **25** entries;
the final owner index corrects the initial prose count of 26. The additional
canonical capture/Session test was added during registration and is now required.

The configured Dart API is `Client(transport: IoTransport())` from
`package:openrouter/openrouter_io.dart`, with
`Credentials credentials = const _OmittedCredentials()`. Only the private typed
omission marker loads environment defaults. Explicit `Credentials` objects pass
through unchanged, including empty/partial objects and null-valued members.
Literal null is a native analyzer/VM/JS compile error; a dynamic-null argument
raises native `_TypeError` before any environment read or HTTP call, without a
null-to-empty conversion. Portable mutable lookup
uses `environment: (name) => values[name]`, captured at client creation. I/O reads
`Platform.environment` inside that lookup; JS/browser defaults report missing.
Protected calls with no complete alternative fail `ConfigurationException` before
HTTP with bounded secret-free diagnostics; anonymous remains usable. Actual
source-default URLs are checked through controlled transports, without account calls.

The new `native_credential_env_constructor_compatibility` selector is required on
both existing Dart tiers. Its bounded installed proof and two separate Chrome
constructor pages are recorded in
`target/sdk-dart-credential-env-null-20260911/{RECHECK.md,report.json}`. It uses the
existing tool/repo/output selectors and support files. The earlier nullable
configured API receipts/packages describe incorrect historical behavior and are
not relabeled as corrected. Main now reports the assigned independent Dart recheck
clean; that review disposition is separate from the runner's registration and
host verification.

C# requires these exact library selectors once each, because each internally runs
SDK8.0.424 and SDK10.0.400 with `rollForward: disable`:

```text
csharp_sdk::credential_env_tests::native_environment_credentials
csharp_sdk::credential_env_tests::native_openrouter_environment_client
```

Both bind `SUSPECT_DOTNET_BIN={dotnet}` and `OPENROUTER_WEB_ROOT={out}/inputs`.
The library host partition includes the five ordinary absent-policy, declaration/
semantic, isolated-generator-canary, canonical-capture and naming tests. Required support sources include
`credential_env.rs`, `CredentialEnvironment.cs`, both native consumers and their
source fixture. The owner receipt records 88 checks/18 controlled requests per tier
for controls, and 10 checks/5 controlled requests per tier for actual OpenRouter
schemas; all 27 no-policy package files match the retained pre-factory bytes.

C#'s configured factory is `Client.FromEnvironment(ClientOptions? options = null,
HttpClient? httpClient = null)` in `OpenRouter.SDK` / namespace `OpenRouter`.
Supplied `HttpClient` stays caller-owned. Existing explicit credentials win as a
whole; explicit null keeps the existing `SdkException(RequestRepresentation)`.
Parameterless `new Client()` stays anonymous. The factory snapshots mapped process
variables at creation; protected missing-auth calls fail bounded secret-free
`SdkException(Authentication)` before HTTP. Cancellation/disposal and source auth
alternatives retain their contracts.

Ruby's complete `ruby_credential_env` suite runs on 3.3.12/4.0.6 with these five
finalized names:

```text
bound_policy_uses_admitted_source_names_and_keeps_no_policy_bytes
credential_env_refusals_are_shared_and_follow_protocol_admission
installed_credential_env_omission_precedence_choices_and_snapshot
installed_openrouter_env_uses_source_default_https_and_real_key_schemas
canonical_credential_env_capture_is_semantic_and_keeps_credential_surface
```

Each tier keeps its existing Ruby/gem selectors and the pinned OpenRouter source
copy. The declared generator canary is explicitly supplied through
`SUSPECT_RUBY_GENERATOR_CANARY`, `RUBY_ENV_BEARER` and `OPENROUTER_API_KEY`; native
child commands replace/remove these controlled values before runtime checks.
The new runtime `ruby_sdk/credential_env.rb` and three source/native fixture files
are prerequisites. The owner records 29 synthetic plus four actual-source
controlled exchanges per tier, installed-byte identity, RBS/Steep/YARD and canonical
semantic capture passing. Existing typed credential/literal-scope controls remain.

Configured `OpenRouter::Client.new` / `.open` use creation-time environment defaults
only when `auth:` is truly omitted. Explicit nil retains `ArgumentError`; empty or
incomplete maps keep protected-operation `RequestError` before transport. Whole
explicit precedence, copied client snapshots, anonymous/source choices and private
helper names are retained. Source defaults are exercised through controlled HTTPS
transports; no account request is part of this native gate.

Ruby's sealed 56-file historical no-policy parity remains a separate owner receipt.
Those bytes retain their original physical URIs and are not a runner prerequisite.
The owner-defined maintained gate compares current no-policy/configured/disabled
plans, including relocated typed provenance and ambient-value independence in
separate controlled generator processes: no environment support without policy or
after disabling it, exactly two conditional support files, and unchanged existing
files except the entry require/README. The runner supplies no ambient
`SUSPECT_RUBY_CREDENTIAL_ENV_BASELINE`, copies no historical baseline into the
execution tree and never labels the sealed comparison as freshly rerun.
The strengthened host-only contract passed with the legacy baseline variable
pointing at an absent directory: three host tests passed and two native tests
stayed ignored. Its exact source delta and logs are preserved in
`target/sdk-ruby-credential-env-runner-20260911-01/`.

These final native/canonical receipts permit the exact registrations above.
Dart's final index/capture/Session checks, Ruby's canonical capture and C#'s
canonical/lint/Rustdoc closeout are green per their receipts; Main now reports the
all-twelve canonical environment/resource bridge and strict ordinary Rust parity
passing. Remaining native selector registrations and Main's final freeze retain their
own scope. No canonical/shared source is edited by the runner, and other provisional
environment tests gain no implicit acceptance or assignment.

### Complete twelve-adapter credential-env inventory

Main accepted the remaining published native receipts and opened all-twelve
canonical ENV readiness. `all-twelve-bridge-01` passed the then-eight shared policy tests
plus the all-twelve resource and strict ordinary Rust parity checks. The runner
requires the complete `credential_env` shared suite, `go_credential_env_canonical`,
`go_credential_env_factory_capture`, `swift_credential_env_canonical`, and the CLI `credential_env_codegen` process
suite in its frozen inventory, in addition to every native suite below.
The shared suite now additionally requires the ninth exact host selector
`empty_operation_capture_cannot_skip_configured_native_admission`. For a configured
policy, empty capture must execute actual native admission and retain the
`http-no-operations` refusal; unconfigured `EmptySelection` semantics remain.
Main's `empty-capture-01/{red-02.log,green.log}` records that source-bound delta.
The prior eight-test bridge keeps its historical scope.

The source-only Go factory capture suite runs once through
`core-suspect-codegen-go_credential_env_factory_capture` and its execution proof,
requiring all three exact controls:

```text
additive_model_collision_reports_source_corresponding_environment_factory_break
unselected_collision_and_absent_policy_keep_the_existing_surface_compatible
environment_factory_signature_is_part_of_native_constructor_comparison
```

It verifies the allocated factory symbol and variadic signature as source-paired
native construction surface: a selected model collision must report the located
`NewClientFromEnv` → `NewClientFromEnv2` break, while unselected collisions and
absent policy preserve compatibility. The existing single-name
`go_credential_env_canonical` target remains required. Fresh host captures use
`SUSPECT_GO_FACTORY_CAPTURE_EVIDENCE={work}/native/go/credential-env-factory-capture`.
The owner's final `target/sdk-go-env-factory-capture-20260911-01/INDEX.json` records
212 unchanged emitted entries and the fresh CLI's expected exit 1 with one breaking
factory rename and one potentially-breaking constructor change. This is a capture-
only repair, not native/runtime execution. Independent Spec recheck and Main's
combined source/CLI qualification remain pending their own receipts.

| Adapter / harness | Required native selectors | Execution |
| --- | --- | --- |
| TypeScript / `typescript_credential_env` | `installed_environment_defaults_preserve_creation_snapshot_explicit_auth_and_security_choices`; `installed_openrouter_current_key_uses_source_bearer_env_and_default_https`; `browser_environment_absence_preserves_explicit_and_anonymous_source_clients` | One complete six-test suite; installed tests internally build TS5.5.4/5.9.3 and execute each on Node22/24; Chromium153 is an actual separate native test in this harness |
| Python / `python_credential_env` | `installed_python_credential_env_snapshots_and_explicit_auth_precedence`; `installed_openrouter_python_credential_env_current_key_and_optional_credits` | One complete five-test suite; each native test installs/executes both Python3.11/3.14 internally with wheel/mypy/Sphinx checks |
| Go / `go_credential_env` | `native_env_factory_snapshots_and_keeps_whole_explicit_credentials_authoritative`; `native_allocated_factory_handles_symbol_collision_and_bounded_env_input`; `native_actual_openrouter_key_and_credits_use_env_snapshot_and_source_https` | Seven named tests per explicit Go1.23.12/1.27.1 tier |
| Rust / `rust_credential_env` | `installed_rust_credential_env_snapshots_and_explicit_credentials_obey_source_auth`; `installed_rust_openrouter_env_current_key_uses_source_default_https` | Five named tests per Rust1.88/stable tier |
| Swift / `swift_credential_env` | `native_credential_env_precedence_snapshot_and_docs`; `native_openrouter_current_key_environment_defaults` | Six named tests per existing compiler/SDK tier |
| Java / `java_credential_env` | `native_credential_env_controls`; `native_credential_env_openrouter` | Six named tests per JDK21/25 tier, all seven control modes; no recovery-mode filter |
| Kotlin / `kotlin_credential_env` | `native_environment_controls`; `native_openrouter_current_key_environment` | Eight named tests per explicit JDK21/25 tier; `SUSPECT_KOTLIN_JAVA_HOME` selects one tier inside the helper |
| PHP / `php_credential_env` | `native_env_snapshot_explicit_auth_and_unavailable_platform`; `native_openrouter_current_key_factory_uses_source_https` | Six named tests per PHP8.3.32/8.5.8 tier with explicit Composer/PHPStan |
| C++ / `cpp_credential_env` | `native_openrouter_credential_env_snapshot_and_explicit_precedence`; `native_credential_env_security_and_portable_controls` | One complete four-test declared C++20/libcurl harness; includes its proved ignored source-generation/no-policy/canary control |
| Dart | The six-name `dart_credential_env` contract above | Both Dart tiers; Main reports independent recheck clean for the bounded constructor repair; separate Chrome receipts retain their scope |
| Ruby | The five-name `ruby_credential_env` contract above | Both Ruby tiers; current portable parity gate, no historical-baseline input |
| C# | The two exact library selectors above | Each internally runs both .NET SDK tiers once |

TypeScript uses `SUSPECT_DOCS_NODE={node22}`, `SUSPECT_NODE24_BIN={node24}`,
`SUSPECT_CHROMIUM={chromium}` and a fresh
`SUSPECT_CREDENTIAL_ENV_ARTIFACTS={work}/native/typescript/matrix/credential-env`.
Python uses the already pinned execution-copy Python tools, offline UV cache and
installed interpreter selections; the helper's internal two-version loop is not
duplicated externally.

Go selects both `GOTOOLCHAIN` and `SUSPECT_GO_TOOLCHAIN` explicitly, with
`SUSPECT_GO_CREDENTIAL_ENV_EVIDENCE={work}/native/go/<tier>/credential-env` and
pinned Sphinx Python. Its Sphinx helper's Go1.23.12 policy remains explicit.
Rust uses a separate `SUSPECT_RUST_CREDENTIAL_ENV_TARGET` per tier; Swift uses
`SUSPECT_SWIFT_CREDENTIAL_ENV_ROOT` per tier and all existing compiler/SDK/DocC
selectors. Java pins Maven and `SUSPECT_OPENROUTER_OPENAPI` to the copied input;
Kotlin pins Maven, its copied repository and one JDK. PHP pins its selected
interpreter/Composer/PHPStan; C++ pins CXX/CMake/Doxygen and a controlled generator
canary. Every actual OpenRouter environment witness uses the pinned input copy.

All published host/no-policy/canonical names in these harnesses are mandatory in
`expected_native_tests`, not just the native rows above. New runtime/test helpers,
baseline fixtures and the shared policy guide are source-readiness prerequisites.
Existing owner receipts establish the callable APIs and scoped native evidence;
the integrated runner still requires fresh frozen execution and its own transcripts.
The Go no-policy test retains its existing Terraform fixture byte check; Terraform
remains a separate artifact/native evidence track, not a thirteenth SDK gate.

## Actual package identities

Every canonical package uses version `0.0.0`. The runner checks the actual emitted
manifest and retains `generated/`, `generated-manifest.json`, `package-roots.json`
and each native installation's artifacts and linkage inventory.

| Language / CLI profile | Package identity | Native import identity | Manifest under its language root |
| --- | --- | --- | --- |
| TypeScript / `typescript-http` | `@suspect-fixtures/sdk-full` | Package exports | `package.json` |
| Rust / `rust-http` | `sdk-full` | `sdk_full` | `Cargo.toml` |
| Python / `python-http` | `sdk-full` | `sdk_full` | `pyproject.toml` |
| Go / `go-http` | `example.com/sdk-full` | `sdk` | `go.mod` |
| Swift / `swift-http` | `SdkFull` | `SdkFull` | `Package.swift` |
| Java / `java-http` | `com.example.generated:sdk-full` | `com.example.generated` | `pom.xml` |
| C# / `csharp-http` | `Suspect.SdkFull` | `Suspect.SdkFull` | `Suspect.csproj` |
| Kotlin / `kotlin-http` | `com.example:sdk-full` | `example.sdk` | `pom.xml` |
| Ruby / `ruby-http` | `sdk-full` | require `sdk_full`, namespace `SdkFull` | `sdk-full.gemspec` |
| PHP / `php-http` | `example/sdk-full` | `Example\SdkFull` | `composer.json` |
| Dart / `dart-http` | `sdk_full` | `package:sdk_full/sdk_full.dart` | `pubspec.yaml` |
| C++ / `cpp-http` | `sdk_full` | `sdk_full` | `CMakeLists.txt` |

These use the configuration rules in the actual `SDK-<LANG>.md` guides. Java and
Kotlin receive Maven coordinates, PHP receives Composer vendor/package syntax,
and Dart/C++ receive legal native identifiers. Swift's real `swift/Package.swift`
root and every other language prefix must be present.

Canonical installations include wheel/venv, npm tarball, extracted Cargo crate,
Go archive/module replacement, Swift archive/SwiftPM dependency, byte-matched
installed Maven jars, NuGet PackageReference, installed gem, Composer ZIP,
archived local pub dependency and exported CMake package consumers. The required
Dart native suite additionally exercises a real loopback hosted-pub repository,
archive download, cache installation and offline resolution. These are different
recorded installation seams; a local dependency is not reported as publishing.

Native docs are actual TypeDoc, Rustdoc, Sphinx, DocC, Javadoc, compiler XML/member
bindings, Dokka, YARD/RBS, PHPDoc/reference, dartdoc and Doxygen outputs. The runner
requires emitted source bindings and rendered outputs, preserves native coverage
files, and runs the native owners' full symbol/type/example/wire suites. Maven
installed jars are copied and matched immediately before another tier can replace
the shared private cache coordinate. Installed source-based packages are compared
to their actual CLI-emitted bytes.

## Tool setup and isolation

The installed tool versions below are prerequisites, not downloads performed by
acceptance. Overrides name real installed tools that still satisfy the version
gates. Native child selectors are set by maintained per-stage environments.

### Existing five-language selectors

| Selector | Default |
| --- | --- |
| `SUSPECT_PYTHON_FLOOR_BIN` | `/Users/luke/.local/share/uv/python/cpython-3.11-macos-aarch64-none/bin/python3.11` |
| `SUSPECT_PYTHON_CURRENT_BIN` | `/opt/homebrew/opt/python@3.14/bin/python3.14` |
| `SUSPECT_PYTHON_TOOLS` | `<original>/target/sdk-native-python-tools/bin/python` |
| `SUSPECT_DOCS_NODE` | `/Users/luke/.local/share/mise/installs/node/22.23.1/bin/node` |
| `SUSPECT_NODE24_BIN` | `/Users/luke/.local/share/mise/installs/node/24.21.0/bin/node` |
| Rust | Installed `1.88.0` / `stable`; matching `RUSTUP_TOOLCHAIN` and `SUSPECT_NATIVE_RUST_TOOLCHAIN` |
| Go | Installed `go1.23.12` / current `go1.27.1`; matching `GOTOOLCHAIN` and `SUSPECT_GO_TOOLCHAIN` |
| `SUSPECT_SWIFT_BIN`, `SUSPECT_SWIFTC_BIN`, `SUSPECT_SWIFT_DOCC_BIN` | Actual current 6.3.3 implementations resolved through `xcrun`; symbol-graph companion required |
| `SUSPECT_SWIFT_SDKROOT` | Selected current toolchain's actual macOS SDK, ordinarily 26.5 |
| `SUSPECT_SWIFT_FLOOR_ROOT` | `${TMPDIR%/}/opencode/swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload` |
| `SUSPECT_SWIFT_FLOOR_BIN`, `SUSPECT_SWIFTC_FLOOR_BIN`, `SUSPECT_SWIFT_FLOOR_DOCC_BIN` | Matching `usr/bin/swift`, `swiftc`, `docc` below that payload |
| `SUSPECT_SWIFT_FLOOR_SDKROOT` | `/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk` |
| `SUSPECT_CHROMIUM` | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` |

The tooling venv requires build 1.4.0, hatchling 1.29.0, httpx 0.28.1, mypy 1.19.1
and Sphinx 8.2.3. npm 10.9.8 and the source-locked TypeScript 5.5.4/5.9.3,
TypeDoc and esbuild tools install offline in the execution copy. Protocol TS floor
and current rows select their corresponding compiler and Node on `PATH`.
The acquisition/native archive fixtures also pin `/usr/bin/curl`, the selected
`openssl`, `tar` and `unzip` implementations through actual version commands.

### Recovered Swift 6.0.3 floor identity

The full runner resolves its default from the **caller's** TMPDIR before child
environments receive private scratch paths. It uses the restored payload recorded
in [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md#retained-development-validation-2026-09-10)
and [SDK-SWIFT-PROTOCOL.md](SDK-SWIFT-PROTOCOL.md#verification-and-retained-evidence):

```sh
export SUSPECT_SWIFT_FLOOR_ROOT="${TMPDIR%/}/opencode/swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"
export SUSPECT_SWIFT_FLOOR_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/swift"
export SUSPECT_SWIFTC_FLOOR_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/swiftc"
export SUSPECT_SWIFT_FLOOR_DOCC_BIN="$SUSPECT_SWIFT_FLOOR_ROOT/usr/bin/docc"
export SUSPECT_SWIFT_FLOOR_SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
```

Explicit root/driver/compiler/DocC/SDK selectors take precedence. A driver override
supplies the default sibling compiler and DocC paths. The full runner inventories
the selected actual payload and SDK metadata for each new attempt.

The older `swift-6.0.3-toolchain/expanded` installation lacks `SwiftShims`; its
failed attempt and earlier tool identities remain preserved. The restored payload
is a distinct selection for new full-runner evidence. The historical `sdk-m3-m6`
default, policies and manifests are not reinterpreted as this recovered toolchain.

### New-language selectors

Here `<mise>` means `$HOME/.local/share/mise/installs` as discovered **before**
the runner replaces child HOME. `<original>` is the actual Suspect checkout.

| Full-runner selector | Default / native child selector |
| --- | --- |
| `SUSPECT_JAVA_FLOOR_HOME` | `<mise>/java/temurin-21.0.12+101.0.LTS`; `JAVA_HOME`, `SUSPECT_JAVA_HOME`, `SUSPECT_KOTLIN_JAVA_HOME` |
| `SUSPECT_JAVA_CURRENT_HOME` | `<mise>/java/temurin-25.0.4+101.0.LTS`; same tier-local selectors |
| `SUSPECT_MAVEN_BIN` | `<mise>/maven/3.9.16/apache-maven-3.9.16/bin/mvn`; also `SUSPECT_KOTLIN_MAVEN` |
| `SUSPECT_DOTNET_BIN` | `$HOME/.local/share/mise/dotnet-root/dotnet`, SDKs 8.0.424 and 10.0.400 |
| `SUSPECT_RUBY_FLOOR_HOME` / `SUSPECT_RUBY_CURRENT_HOME` | `<mise>/ruby/3.3.12` / `<mise>/ruby/4.0.6`; mapped to `SUSPECT_RUBY_HOME` |
| `SUSPECT_RUBY_FLOOR_GEMS` / `SUSPECT_RUBY_CURRENT_GEMS` | `<original>/target/sdk-ruby-tools/gems` / `gems-ruby4`; mapped to `SUSPECT_RUBY_GEMS`, with the matching default gem ABI directory |
| Ruby tool gems | YARD 0.9.37, Redcarpet 3.6.1, RBS 3.9.5, Steep 1.10.0 |
| `SUSPECT_PHP_FLOOR_BIN` / `SUSPECT_PHP_CURRENT_BIN` | `<original>/target/sdk-php-tools/php-8.3.32/php` / `php-8.5.8/php`; mapped to `SUSPECT_PHP_BIN` |
| `SUSPECT_COMPOSER_PHAR` | `<original>/target/sdk-php-tools/composer-2.10.3.phar` |
| `SUSPECT_PHPSTAN_PHAR` | `<original>/target/sdk-php-tools/phpstan-2.2.13.phar` |
| `SUSPECT_DART_FLOOR_BIN` / `SUSPECT_DART_CURRENT_BIN` | `<original>/target/sdk-dart-tools/dart-sdk/bin/dart` / `current-3.13.3/dart-sdk/bin/dart`; mapped to `SUSPECT_DART_BIN` |
| `SUSPECT_CPP_CXX` | `/usr/bin/clang++`, Apple Clang 21.0.0; declared C++20/libcurl 8.7.1 profile |
| `SUSPECT_CPP_CMAKE` | `<mise>/cmake/3.31.6/cmake-3.31.6-macos-universal/CMake.app/Contents/bin/cmake`; sibling `ctest` |
| `SUSPECT_CPP_DOXYGEN` | `<original>/target/sdk-cpp-tools/doxygen/doxygen-1.18.0/doxygen` |

Kotlin's emitted Maven metadata pins Kotlin 2.4.20, coroutines 1.11.0 and Dokka
2.2.0. Complete Maven installation and native docs are required, not a compiler
version string standing in for those dependencies. Kotlin's default promotion
does not remove the full runner's explicit seven-language feature selection or
the required native JDK21/JDK25 and integration gates.

### Private cache bindings

| Optional cache-seed selector | Default |
| --- | --- |
| `SUSPECT_FULL_CARGO_CACHE` | Original `$CARGO_HOME/registry`, or `$HOME/.cargo/registry` |
| `SUSPECT_FULL_GO_CACHE` | `$HOME/go/pkg/mod` |
| `SUSPECT_FULL_NPM_CACHE` | Original `$npm_config_cache`, or `$HOME/.npm` |
| `SUSPECT_FULL_UV_CACHE` | Original `$UV_CACHE_DIR`, or `$HOME/.cache/uv` |
| `SUSPECT_FULL_JAVA_MAVEN_CACHE` | `<original>/target/sdk-java-maven-cache/java/repository` |
| `SUSPECT_FULL_KOTLIN_MAVEN_CACHE` | `<original>/target/sdk-kotlin-maven` |

Caches are byte-inventoried and copied to fresh external scratch, not linked as
writable original inputs. `cache-inputs/*.json` records their seed bytes. Child
HOME/XDG/Cargo/npm/uv/Go/Maven/.NET/NuGet/Composer/pub homes and build outputs are
private. Cargo's private home has no ambient configuration; native extracted
packages and TMPDIR are outside an ancestor Suspect workspace. Go disables
ambient goenv/workspaces; Cargo/npm/uv/Maven/Composer use offline/no-download
settings. The actual local-hosted Dart and acquisition fixtures can still use
their explicit loopback sockets.

The private execution tree provides the compatibility Python-tooling link and
copied Composer/PHPStan files required by older helpers' relative paths. Their
original tool payloads are fingerprinted, not modified. Compiler/runtime/library
payloads, including directory-linked .NET/JDK contents, actual executables,
SDK settings, npm/Python tools and consumed configuration are checked for drift.

## Real installed VS Code host acceptance

The finalized entrypoint is
`editors/vscode/test/native-host/run.cjs`, with its closed
`pins.schema.json` and `contract.cjs` verification. The owner handoff is preserved
at `target/sdk-editor-native-host-integration/HANDOFF.md`. Full acceptance requires
both **lifecycle** and **commands**; a selection probe cannot satisfy either.

Supply `--editor-host-tools /absolute/editor-host-tools.json`. This full-runner
input intentionally exposes only installed app/tool pins:

```json
{
  "format": "suspect.sdk.full.editor-tools.v1",
  "vscode": {
    "executable": "/absolute/Visual Studio Code.app/Contents/MacOS/Code",
    "executableSha256": "<lowercase SHA-256>",
    "archive": "/absolute/VSCode-platform.zip",
    "archiveSha256": "<lowercase SHA-256>",
    "version": "1.137.0",
    "commit": "<40 lowercase hexadecimal characters>",
    "platform": "darwin-arm64"
  },
  "tools": {
    "directory": "/absolute/native-host-tools",
    "lockSha256": "<SHA-256 of directory/package-lock.json>"
  }
}
```

Objects reject unknown fields. The caller cannot choose a CLI, editor source,
VSIX, profile subset, scenario, mode or replacement command in this document.
Missing/unusable tool pins leave required editor gates unmet in both acceptance
profiles. A tool-only configuration for the installed host is retained in
`target/sdk-full-runner-check-20260910-04/editor-host-tools.json`; its contents
contain app/tool pins only, with no historical CLI or VSIX identity.

The app/archive and installed tools must already exist. The tool directory needs
locked `@vscode/test-electron`, `@vscode/vsce` and `playwright-core`. The witnessed
versions are 3.1.0, 3.9.2 and 1.63.0; installed versions must match the pinned lock.
App/tool payload trees and selected files are hashed and rechecked.

After compiling the actual private editor copy, the runner:

1. Executes the maintained native-host input guard tests with complete TAP results.
2. Calls **`contract.cjs.sourceIdentity`** on that compiled copy and compares its
   exported check/screenshot contract with the runner's maintained lists. The hash
   formula is the owner's ordered source inventory, not a new Rust approximation.
3. Stages exact runtime/manifest bytes, runs offline production-only `npm ci`
   against the copied npm cache, and packages a new self-contained VSIX using the
   pinned `vsce`. It never publishes an extension.
4. Creates `editor-native/pins.json` in the owner's
   `suspect.editor.native-host.pins.v1` format, with this run's frozen
   `bin/suspect` hash, all twelve profile IDs, compiled editor identity, actual
   app/tool pins and newly packaged VSIX. Historical eight-profile pins cannot
   enter this generated document.
5. Launches the source-copy native runner twice, forcing `SUSPECT_NATIVE_MODE=run`
   and `SUSPECT_NATIVE_SCENARIO=lifecycle` / `commands`, with distinct absent
   `editor-native/<scenario>/` outputs. `SUSPECT_TEST_BINARY` is always the frozen
   CLI. A short, existing external `${TMPDIR%/}/opencode` parent is supplied for
   the native runner's unique `n-*` profile roots and macOS IPC-length guard.

The host calls use cleared/rebuilt environments. Unrelated `SUSPECT_NATIVE_*`
variables, notably the native Rust selector, are removed before the five host
inputs and forced mode are added. The host owns its private HOME/XDG/TMP/profile
directories; Node/system utilities remain on PATH. The supplied fresh VSIX is
installed locally with `--do-not-include-pack-dependencies`. Offline dependency
cache misses fail preparation rather than falling back to network packaging.

Each authoritative `report.json` must be `suspect.editor.native-host.run.v2`,
`mode: run`, the exact requested scenario, `status: passed`, and exit 0. The full
runner independently checks:

- Exactly twelve **expected, queried and native-observed** backend IDs.
- This invocation's CLI/source/pins/VSIX/app/tool identities and source-copy
  harness hashes; raw `native-report.json` identity and matching observations.
- The exact ordered 15 lifecycle / 7 command check names and all eight boolean
  claims from the finalized contract. Missing, extra, renamed, skipped, string
  truth values, legacy reports and `selectionProbeOnly` are rejected.
- Native host exit 0, no signal/timeout, and all recorded subprocesses' real
  executable hashes, argv and exit status.
- Ten lifecycle / four command screenshots with the exact names, renderer
  viewport provenance, actual PNG bytes, hashes and output containment.
- Complete dynamic owned-output before/after inventories, actual current file
  census/hashes/sizes, ownership manifest and canonical TypeScript model. The
  complete owned tree is copied into sealed evidence; no historical 63-file
  threshold is used.
- Distinct native attempt IDs and isolated profiles for the two scenarios.

`editorNativeContract` in the stage inventory/report records exact strings and
screenshots. `editor-native-integrity` requires both full scenario proofs and
unchanged app/tool payloads. Native profiles and all output attempts remain
preserved separately from the final readonly report.

## Sealed evidence contract

- Snapshot selection is `git ls-files --cached --others --exclude-standard -z`,
  including actual staged, unstaged, untracked and deleted paths. The source
  manifest records kinds, hashes, sizes, modes and confined relocatable symlinks.
  Embedded bytes for `sdk_full.rs`, `sdk_m3_m6.rs` and `main.rs` must match the
  running verifier. HEAD alone is never presented as this dirty implementation.
- Build/test execution uses a byte-verified external source copy. The CLI is
  frozen under `bin/suspect`; required harnesses are individually frozen under
  `bin/tests/<crate>/<suite>`. Their Cargo JSON artifact attribution, hashes and
  source fingerprint are retained. A later Cargo build cannot replace those
  copies. The baked-in CLI process-test path receives the same frozen CLI bytes.
- Each command records exact executable/hash, argv, cwd, cleared-and-rebuilt
  environment, real exit status, elapsed wall time and stdout/stderr hashes.
  Expected drift exit 1 can meet a semantic criterion without relabeling raw
  `success` as true. Missing executables, timeouts and fake exit-zero/no-output
  tools fail their native/output proofs.
- `native-artifacts/<language>/<tier>/inventory.json` records canonical package
  archives, actual installations, rendered docs and their source/member linkage.
  `native-outputs.json` additionally indexes retained native suite sources,
  consumers, archives, coverage and command logs. Discarded successful subprocess
  output in an older native test is not invented as a separate transcript.
- Final checks compare original and execution source bytes, original four input
  hashes, generated sources, frozen binaries and tool payloads. Every missing
  required stage is explicitly recorded as unmet, including after an early abort.
- `report.json` is create-once. `seal.json` inventories all report bytes/link
  targets; `seal.sha256` anchors it. The evidence tree becomes readonly and the
  runner verifies its final seal and exact file census before returning success.
  Scratch is preserved separately; its authoritative path is in `invocation.json`.

## Strict numerical M6

The full runner uses the existing, source-controlled
`tools/sdk-session-perf/accept.py import` implementation. That adapter recomputes
qualification and comparison from actual pinned raw evidence and verifies current
source/snapshot, tools, inputs, prepared sources, benchmark binaries, build
configuration and runner identity. Acceptance itself does **not** collect timings
amid native builds.

The full-runner plan intentionally has only this schema:

```json
{
  "format": "suspect.sdk.full.performance-plan.v1",
  "evidence": {
    "path": "/absolute/performance-inputs.json",
    "sha256": "<reviewed lowercase SHA-256>"
  }
}
```

The evidence file is the existing
`suspect-sdk-session-acceptance-inputs-v1` pin manifest containing exactly
`baseline`, `candidate`, `comparison`, `policy` and `runner` pins, as documented in
[SDK-SESSION-PERFORMANCE.md](SDK-SESSION-PERFORMANCE.md#original-workspace-evidence-bridge-for-final-acceptance).
Supply it with `--performance-plan /absolute/full-plan.json`.

The runner fixes all four import commands and semantic assertions. Plans cannot
supply executables, shell scripts, environments, stages or replacement claim
pointers. Import reports must be newly created inside this attempt's
`performance/`; source/CLI hashes are passed explicitly. Incomplete, observational,
ineligible, noisy, regressed or changed evidence cannot satisfy numerical M6.

This preserves the original **five-backend v2 numerical workload and policy**;
`originalM6NumericalScope` records that fact. Native twelve-language cost
observations are a different required workload below. Historical failed Mac
qualification and earlier reports are never rewritten or renamed calibrated.

## Remaining Main/native-owner coordination

The runner is deliberately fail-closed while owners finish. `--check-syntax`
produces the current exact missing-suite list; current native base reports remain
their independent completed checkpoint.

1. Main integration belongs to `ses_f72dcea72ffe3CL9Uf6IC2rP9u`; xtask stays with
   this runner owner. Default discovery now has all twelve profiles. The runner
   independently rechecks a fresh default-feature CLI as well as the explicit
   all-feature CLI/test build; changing-source integration and final quality/review
   must settle before the expensive combined acceptance run.
2. Finish the twelve-language native protocol matrix, with Ruby's delivered base
   and expanded protocol witnesses in `ruby_sdk` and separate `*_protocol` suites
   for the other eleven languages. Required existing witnesses are enumerated in
   `expected_native_tests`;
   new protocol suites need a passing `native_*` or `installed_*` test and a
   complete suite. Renaming an existing mandatory witness requires maintaining
   the inventory rather than silently dropping it.
3. Main is coordinating removal of the historical-target-dependent
   `kotlin_sdk::shared_protocol_adoption_preserves_verified_native_artifacts`
   checkpoint test from the maintained path. The full runner executes the current
   complete `kotlin_sdk` suite and does not import or modify the historical
   `target/sdk-kotlin-json-verified/report.json` or its follow-up evidence.
4. C#'s finalized native protocol/resource selectors honor `SUSPECT_DOTNET_BIN`.
   The internally two-tier library gates are each invoked once, with actual tool
   identity still pinned. Earlier missing outer Cargo stdout remains a documented
   completion summary in the owner's receipt, not a reconstructed transcript.
5. The native cost collector implementation is delivered by
   `ses_f735ad0f9ffedC7SHS1oWpyReX`; its maintained suite is
   `crates/suspect-codegen/tests/sdk_native_measurements.rs`, with required gates
   `core-suspect-codegen-sdk_native_measurements`, its `-execution` proof, and
   `native-costs-evidence`. Its 27 host checks and 40 real development observations
   across five languages/both tiers are recorded in
   [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md). Full acceptance still requires the
   fresh complete **92-row** report for all twelve current packages and tool/input
   identities under the contract below.
6. Editor owner `ses_f7340c274ffe1qSTEfMu0Dols6` finalized the reusable host
   contract. Both full scenarios are integrated above; the 53 input-guard tests,
   controlled eight-profile selection probe and earlier graphical matrices remain
   their historical scopes. They do not substitute for a new twelve-profile,
   current-source installed-host run.
7. TypeScript's finalized V2/V3 selectors, Java's focused aggregate-example
   selector and all ten IR dual-role scope witnesses are integrated. Main reports
   the shared fixes/39 checks and Go docs/reflection on both tiers green. The IR
   owner reports 104 full IR passes, eight schema-context passes and all-target IR
   Clippy passing, with Main's exact compatibility witness passing unmodified.
   Main's latest common/canonical Standards and Spec review axes are both clean,
   with 43 shared checks. Final native/canonical bridge work remains with the
   existing owners before stabilized full acceptance.
8. Recent PHP/C++ host expectation repairs and Dart's test-support consolidation
   are covered by their existing complete suites and exact native selectors.
   C#'s repaired positional suite is explicitly frozen and required above. The
   C#'s focused library/positional Clippy gate now passes. Its latest broad
   test-target receipt reports only Kotlin, Python and Go test lints; the earlier
   Terraform findings were repaired by that owner. These focused receipts retain
   their own scope while Main completes workspace quality.
9. Final Rust/Swift V3 native inventories, the full Go V3/aggregate suites and the
   Go pattern witness are registered above. Main's all-twelve resource/ENV bridge
   and strict ordinary Rust parity are green. Swift's accepted aggregate
   floor/current selection is now assigned; the accepted
   TypeScript ignored-multipart native/source handoff is now registered. Completed
   owner matrices are not inventory substitutes for the eventual fresh integrated run.
10. Ruby's finalized credential-metadata repair has explicit six-control host
    capture coverage above. The owner reports all 18 SDK/runtime assets identical
    to the accepted native inventory; the source change is confined to Ruby capture.
    Its preserved all-twelve replay retains genuine wire/TypeScript findings and
    zero unknowns. Original interim/red comparison reports keep their recorded scope.
11. Current user-environment/live-DX/native-helper work remains with the existing
    twelve language owners. All twelve published credential-env native contracts
    are now registered above. Unassigned additions still fail the library census.
    Main supplies the final source
    freeze before integrated SDK, editor, cost or numerical execution.
12. Main's frozen ENV Clippy/Rustdoc checks passed. Its original workspace attempt
    recorded 1,641 passed, 12 setup failures and 395 ignored across 258 harnesses;
    the later `quality-setup-closure-01` receipt closes all twelve identified setup
    failures with targeted reruns. The original attempt retains its partial scope;
    successful suites were not replayed. Main reports Dart's independent P2 recheck
    clean; Go's capture P2 remains pending its reviewer. F10 overlay verification
    is a separate controller receipt, and full
    SDK/native/editor/cost/numerical acceptance remains unstarted.

### Native cost integration contract

The compiled native suite receives these explicit locations:

- `SUSPECT_SDK_FULL_PACKAGES`: immutable actual `generated/` package roots;
- `SUSPECT_SDK_FULL_NATIVE_ROOT`: the actual private installed consumer roots;
- `SUSPECT_SDK_FULL_TARGETS`: maintained twelve-target configuration;
- `SUSPECT_SDK_FULL_SOURCE_SHA256`, `SUSPECT_SDK_FULL_BINARY_SHA256`: actual source
  census and frozen CLI identities;
- `SUSPECT_SDK_FULL_MEASUREMENTS`: fresh external `native-costs/` output directory.

It must execute real native build/import/codec/request work against these packages
and independent fixtures, preserving sample subprocesses and raw logs. The runner
requires `native-costs/report.json` with `format: suspect.sdk.native-costs.v1`,
`complete: true`, `sourceFingerprint`, `cliSha256`, and `measurements`.

There must be exactly one row per maintained language × `toolchain_tiers` entry ×
phase (`build`, `import`, `codec`, `request`). Each row contains:

```text
language, tier, phase, packageManifestSha256, artifactBytes (>0),
samples: [
  { nanoseconds (>0), iterations (>0), exitCode: 0,
    command: [absolute executable, ...actual args], programSha256,
    stdout: relative log path, stdoutSha256,
    stderr: relative log path, stderrSha256 }
]
```

Raw logs must be confined to that root and match their digests; executable bytes
and actual package manifests must match too. These observations establish native
cost boundaries and sizes, not calibrated p95 or a new regression threshold.
Partial development observations cannot satisfy this gate in either profile.
The four phase assertions and original numerical M6 policy remain the same.
