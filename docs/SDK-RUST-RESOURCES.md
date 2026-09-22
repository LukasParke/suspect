# Rust physical resources and checked dynamic validation v3

Rust implements the checked resource contract in
[SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md), plus the shared HTTP physical
document-base contract. Native validation, installed SDKs, source-backed examples,
mutable codecs, real reqwest requests and independent controls pass on Rust
**1.97.1** and **1.88.0**.

## Explicit APIs and capability boundary

```rust,ignore
use suspect_codegen::{rust_models, rust_codecs, rust_http, rust_validation};
use suspect_schema::{Config, OwnedCompiler};

let compiled = OwnedCompiler::new(Config::default())
    .compile_v3(contract.clone(), &schema_roots)?;
let validation_files = rust_validation::emit(&compiled.program())?;
let models = rust_models::plan_models_v3(&contract, &schema_roots);
let codecs = rust_codecs::plan_codecs_v3(
    contract.clone(), &schema_roots, Default::default(),
)?;
let http = rust_http::plan_http_v3(
    contract, &operation_sources, Default::default(),
)?;
```

The exact v3 pair is:

- `suspect.validation.experimental.v3`
- `oas31-jsonschema202012-resources-dynamic`

`rust_validation::emit` calls `OwnedProgram::check` and independently recognizes
the three native version/profile pairs. Malformed resource metadata, node scopes,
targets or envelopes fail before artifacts. Policies above the 512-depth ceiling
or portable 32-bit counter/index capacity are refused explicitly.

`plan_models`, `plan_codecs` and `plan_http` retain v1 admission; their `_v2`
counterparts retain v2 admission. `_v3` planning uses the established v1/v2 path
for ordinary closures. `plan_http_v3` preserves **all** ordinary v2 HTTP artifact
bytes, including adapter/capability metadata in `http-manifest.json`, examples and
caller configuration. Resource admission includes a selected nested entry below
an id-less ancestor `$dynamicAnchor`; an unrelated sibling anchor does not promote
a base closure.

The HTTP selector starts with the baseline protocol. A typed resource-capability
requirement or inherited native resource scope selects the verified v3 branch;
ordinary inputs retain `rust-http-protocol-v1`. Planned Rust compatibility capture
calls `plan_http_v3` and records `plan.protocol().capabilities().adapter()`, yielding
the actual ordinary or resource profile.

| Capability | Native profile and proof |
| --- | --- |
| `DocumentRelativeServers` | Base and v3 HTTP profiles; separate installed physical-base/reqwest witness. |
| `SchemaResources` | `native_capabilities_v3()` / `rust-http-resources-v3`; checked v3 programs, aliases/scopes and installed codecs. |
| `DynamicSchemaReferences` | Same explicit v3 profile; official and independent native dynamic-scope execution, followed by actual SDK request/response validation. |

The resource branch uses `plan_protocol_examples_v3`; ordinary inputs retain the
v2 example planner alongside their ordinary codecs. Static annotation discovery
does not traverse dynamic fallbacks. Candidate-aware `codec_schema_closure()` is
used for directional refusal checks; candidate bindings are not added as HTTP
codec inputs. Unsupported readOnly/writeOnly projection remains source-linked.

## Native execution

`rust_validation/runtime_v3.rs` is a separate emitted runtime. V1/v2 runtime and
exact-number/NFA assets are preserved.

- Enter each node's indexed physical resource, even for a selected nested entry;
  do not evaluate a resource root just to establish context.
- Search outermost actually entered distinct resources for a dynamic binding.
  Keep the initial target when none matches. Do not enter the fallback before
  lookup. Unentered candidates remain inert.
- Pointer, empty-fragment and ordinary-anchor dynamicRef operands have no dynamic
  anchor name and use their checked initial target. Static Ref stays static.
- Cycle identity includes node, instance identity and an exactly interned ordered
  resource context. Changed-context revisits are not old-context cycles.
- Restore scope after every valid, invalid and failed return, including trials
  and reused codec validation sessions. Target annotations start fresh; only
  successful target annotations propagate. Existing scoped-applicator rules apply.
- Depth, work, exact numeric and recursive-nonprogress failures remain
  `EvaluationFailure`, including under conditionals, alternatives and negation.
- Charge one shared evaluation step for every new distinct resource entry and
  every scanned resource/binding. Nonmatching bindings cost work too. Restoration
  and context-cache lookup add no visits.

Runtime lookup uses checked indices only and has no URI resolver, source loader
or acquisition API. `validation::resources()` and `node_scopes()` expose immutable
`Resource`, `DynamicBinding`, `NodeScope` and physical `Source` metadata. Canonical
names, aliases and schema-root addresses do not replace diagnostic identity.

## Model and codec behavior

Source-local static types and required fields keep native constructors. Dynamic
references use exact JSON carriers. Context-dependent unions also use exact JSON
carriers, because a converter-side standalone union trial would discard the
calling resource stack and could choose the wrong fallback. Null is retained
unless source-local assertions prove it impossible independently of the binding.

Whole-root decode validates before conversion; mutable encoding validates the
actual root after conversion. The installed witness demonstrates the same generic
object accepting string values standalone, integer values under one resource and
null under another. Exact `9007199254740993` survives codecs and real wire traffic.
The string fallback cannot narrow the integer/null carrier or bypass validation.

Native outcomes and codec/HTTP error distinctions remain explicit. The installed
SDK rejects invalid mutations before transport, rejects invalid response bodies,
and reports resource failures in both directions from a separately generated
low-budget package. Physical schema keyword and escaped instance paths survive.

## HTTP physical bases

Relative servers use emitted `Server::document_base`, copied from the shared
physical retrieval decision. An absent entry server belongs to the entry document;
an explicit empty override belongs to its declaring document. A caller's
`document_url` takes precedence. Logical `$self` and `$id` are never API URL bases.

The reqwest/URL stack preserves encoded slashes and ordinary relative `.`/`..`.
Rust explicitly refuses encoded dot segments before parsing/joining because this
transport would normalize their path. Local files require a serving-document URL
or absolute server override. See [SDK-RUST-PROTOCOL.md](SDK-RUST-PROTOCOL.md) for
native provenance fields and OAuth/OIDC URL-base metadata.

## Maintained witnesses

All fixtures are compiled from source through real `OwnedCompiler::compile_v3`.
The maintained native test does not consume the one-off executable handoff JSON.

| Target | Exact native selector |
| --- | --- |
| `rust_validation_v3` | `installed_rust_v3_executes_all_official_dynamic_ref_source_fixtures` |
| `rust_validation_v3` | `installed_rust_v3_scope_cycles_fallbacks_and_work_failures_are_independent` |
| `rust_validation_v3` | `installed_rust_v3_default_depth_scope_restore_and_annotation_isolation` |
| `rust_protocol_v3` | `installed_rust_v3_sdk_preserves_dynamic_models_examples_and_real_wire` |
| `rust_protocol_resources` | `installed_rust_relative_servers_use_effective_physical_documents` |

The official gate executes all **44** cases from the unmodified pinned files in
`crates/suspect-schema/tests/fixtures/resource-conformance/`, supplying all three
remote documents through a closed provider. It also executes the four previously
deferred dynamic/unevaluated cases from their existing source fixtures: **48 total**.

Independent controls cover outermost precedence, unentered candidates, static
fallback modes, nested entries, changed-context and nonprogress cycles, exact
resource/binding charges, fresh annotations and every return outcome. The native
512/513 distinct-resource boundary passes on an explicit **2 MiB stack**.

The installed SDK gate checks:

- Model-only package compilation and **26 Rustdoc examples** with warnings denied.
- Built all-feature Rustdoc and **five executable source-backed examples**.
- Primary and low-budget `.crate` archives installed into a fresh consumer.
- **Three native consumer tests** and **three compile-fail type controls**.
- Four actual reqwest requests, with invalid inputs and resource failures proven
  not to send extra requests; physical sources, aliases and URL spelling checked.

Ordinary host selectors cover malformed programs, profile/byte preservation,
candidate-aware source diagnostics, id-less nested admission and capability fences.

```sh
cargo test --locked --offline -p suspect-codegen \
  --test rust_validation_v3 --test rust_protocol_v3 --test rust_protocol_resources \
  -- --include-ignored --nocapture --test-threads=1
```

For a single witness, use its target and exact selector with `-- --exact --ignored`.
`SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0` selects the floor for emitted packages.
`SUSPECT_RUST_V3_TARGET`, `SUSPECT_RUST_V3_HTTP_TARGET` and
`SUSPECT_RUST_RESOURCES_TARGET` isolate native build caches. Every native attempt
is retained; newer attempts include command/stdout/stderr/status in `commands.log`.

## Verified evidence — 2026-09-10

All five native selectors passed on **both toolchains**, in focused invocations.
The completed v1/v2 and expanded HTTP matrices were not replayed. The Rust-only
source harness was used when another adapter was mid-edit; it copies the actual
live Rust modules and uses the actual live schema/IR libraries.

The stable result record is
`target/sdk-rust-resources-20260910-01/evidence.json`, SHA-256
`04ed97e2ab94980ef460dadc2ebd07cca5a812657a54c452ba7564e6873bbf0c`.
It records all ten native attempts, retained package/consumer paths, eight copied
command logs, final production hashes and four frozen-runtime byte comparisons.
The initial current-toolchain runs predate per-attempt command logging; their
completed tool transcripts and retained consumers are identified explicitly.

Original tool-run identifiers:

| Evidence | Log |
| --- | --- |
| Current v3 48-case fixture and independent scope/fallback gates | `sh_08d6022a6001SeoF0dhll5Kk2E.out` |
| Rust 1.88 v3 fixtures/scope controls and physical-base gate | `sh_08d6a6971001IH4RrvgvwZewAE.out` |
| Current final physical-base gate | `sh_08d6a69710023T6G6o7T4YrCqB.out` |
| Current installed SDK, examples/candidate controls, base byte fences | `sh_08d6ec80e0015d0eZU5gZbVqsq.out` |
| Rust 1.88 installed SDK | `sh_08d711841002sUA2HqES6AHi8L.out` |
| Current default-depth, session restoration and annotation gate | `sh_08d711841001BEq3nZIqKPEAxB.out` |
| Rust 1.88 default-depth/session/annotation gate | `sh_08d7332df002X5bzhuFx0gdjD7.out` |
| Id-less nested-entry admission regression, green | `sh_08d745609001peqkAgxvuYYWTc.out` |

The nested-entry review found and fixed one profile-selection edge: an ancestor's
id-less dynamic anchor had been missed by the v3 selector. Its red tool transcript
is preserved in `nested-admission-red-transcript.log` beside the evidence record.
Selection now uses indexed anchor ancestry; the unchanged native evaluator
already passed nested-entry execution.

### Final owned quality

Warnings-denied Clippy passes for the live default-feature `suspect-codegen`
library and all three focused Rust test targets. The six Rust-owned findings from
Main's quality handoff are repaired. Evidence: `clippy-green.log` beside the result
record (tool run `sh_08d9c4ee6001aAMl2P6vnyUJ8F`). Scoped rustfmt checks pass,
including an independently formatted Rust capture section; the shared capture
file is handed back to Main for whole-file formatting. These cleanup changes
did not replay native matrices.

The final integrated ordinary run passes **5 tests**, with the five native gates
already executed separately. `ordinary-green.log` and `cleanup.json` record this
check and verify that every final production hash still matches the immutable
evidence snapshot. Cleanup record SHA-256:
`0ec403478b3f8fb4091ca80f6470c7f7375764f823a395da3b485c9a3421a330`.

## Production asset handoff

Paths below are relative to `crates/suspect-codegen/src/`:

- `rust_validation.rs`
- `rust_validation/runtime_v3.rs` — new separate runtime.
- `rust_models.rs`
- `rust_models/resources.rs` — new profile/carrier admission helper.
- `rust_codecs.rs`
- `rust_http.rs`
- `rust_http/plan.rs`
- `rust_http/emit/descriptors.rs`
- `rust_http/runtime/descriptors.rs`
- `rust_http/runtime/servers.rs`
- `rust_http/emit/docs.rs` — generated resource guide and owned lint repairs.
- `rust_http/emit/bodies.rs` — owned lint repair, same emitted expression.
- `compatibility/native.rs` — Rust capture section only; source-selected v3/ordinary bridge.

The new modules are registered in language-owned `rust_http::source_assets()`;
Main's centralized provenance hash already includes that inventory. Schema, IR,
shared HTTP/example implementation and other-language ownership stay with their
existing sessions. Main owns canonical backend dispatch; the explicit v3 API above
is the handoff for advancing that dispatch and its matching Rust capture/profile.

## Supplemental ordinary-HTTP profile fix

Main's unchanged
`sdk_generation_options::canonical_rust_generation_session_and_capture_select_the_verified_scoped_profile`
now passes its complete HTTP artifact comparison and canonical generation/session/
capture checks. Two additional host tests in `rust_protocol_v3` pass:

- `rust_v3_http_profile_preserves_all_ordinary_artifacts`
- `rust_v3_http_profile_capture_tracks_selected_resource_admission`

They cover base v1/v2 HTTP packages, unselected resources, metadata-only byte
operations, explicit legacy interpretation and byte ceilings, resource-required
capture, inherited id-less resource scopes and the actual captured profile.
Warnings-denied Clippy and scoped rustfmt pass. The accepted native matrices were
not replayed.

Only `rust_http/plan.rs` and the Rust capture function changed in production.
The original evidence, cleanup record and Main receipt retain their original
hashes. Supplemental disposition:
`target/sdk-rust-resources-20260910-01/http-profile-selection-supplement-01.json`
SHA-256: `973c57c338a6bb3c935b46143d3fc01739a8e3d8d5ca77580d37b54f0b5ed454`.
It records the two replacement source hashes and verifies all other accepted
production/runtime assets. Main can now promote its `Backend::RustHttp` call to
`plan_http_v3` and include Rust in the all-resource canonical bridge; Rust capture
already uses the matching API and selected profile.
