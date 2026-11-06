# Rust physical resources and checked dynamic validation v3

Rust implements the checked resource contract in
[SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md), plus the shared HTTP physical
document-base contract. Native tests cover validation, installed SDKs,
source-backed examples, mutable codecs, real reqwest requests and independent
controls on Rust **1.97.1** and **1.88.0**.

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
requirement or inherited native resource scope selects the v3 branch;
ordinary inputs retain `rust-http-protocol-v1`. Canonical `rust-http` backend
dispatch and Rust compatibility capture both call `plan_http_v3` and records `plan.protocol().capabilities().adapter()`, yielding
the actual ordinary or resource profile.

| Capability | Native profile and tests |
| --- | --- |
| `DocumentRelativeServers` | Base and v3 HTTP profiles; separate installed physical-base/reqwest test. |
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
actual root after conversion. The installed SDK test demonstrates the same generic
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
transport would normalize their path; this is the one transport-imposed exception
to [ADR 0035](adr/0035-one-server-url-policy.md)'s opaque-octet rule. Local files require a serving-document URL
or absolute server override.

## Native tests

All fixtures are compiled from source through real `OwnedCompiler::compile_v3`;
native tests do not consume a precompiled executable program file.

| Target | Native test (`--ignored`) |
| --- | --- |
| `rust_validation_v3` | `installed_rust_v3_executes_all_official_dynamic_ref_source_fixtures` |
| `rust_validation_v3` | `installed_rust_v3_scope_cycles_fallbacks_and_work_failures_are_independent` |
| `rust_validation_v3` | `installed_rust_v3_default_depth_scope_restore_and_annotation_isolation` |
| `rust_protocol_v3` | `installed_rust_v3_sdk_preserves_dynamic_models_examples_and_real_wire` |
| `rust_protocol_resources` | `installed_rust_relative_servers_use_effective_physical_documents` |

The official test executes every case from the unmodified pinned files in
`crates/suspect-schema/tests/fixtures/resource-conformance/`, supplying the
remote documents through a closed provider. It also executes the
dynamic/unevaluated cases that v2 defers, from their existing source fixtures.

Independent controls cover outermost precedence, unentered candidates, static
fallback modes, nested entries (including an id-less ancestor `$dynamicAnchor`),
changed-context and nonprogress cycles, exact resource/binding charges, fresh
annotations and every return outcome. The native 512/513 distinct-resource
boundary is checked on an explicit **2 MiB stack**.

The installed SDK test checks:

- Model-only package compilation and its Rustdoc examples with warnings denied.
- Built all-feature Rustdoc and executable source-backed examples.
- Primary and low-budget `.crate` archives installed into a fresh consumer.
- Native consumer tests and compile-fail type controls.
- Actual reqwest requests, with invalid inputs and resource failures proven not
  to send extra requests; physical sources, aliases and URL spelling checked.

Ordinary host tests cover malformed programs, profile/byte preservation,
candidate-aware source diagnostics, id-less nested admission and capability
fences. In `rust_protocol_v3`,
`rust_v3_http_profile_preserves_all_ordinary_artifacts` and
`rust_v3_http_profile_capture_tracks_selected_resource_admission` cover base
v1/v2 HTTP packages, unselected resources, metadata-only byte operations,
explicit legacy interpretation and byte ceilings, resource-required capture,
inherited id-less resource scopes and the actual captured profile.
`sdk_generation_options::canonical_rust_generation_session_and_capture_select_the_verified_scoped_profile`
checks canonical generation, sessions and capture against the same profile.

```sh
cargo test --locked --offline -p suspect-codegen \
  --test rust -- --include-ignored --nocapture --test-threads=1 \
  rust_validation_v3:: rust_protocol_v3:: rust_protocol_resources::
```

For a single test, use its target and exact name with `-- --exact --ignored`.
`SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0` selects the floor for emitted packages.
`SUSPECT_RUST_V3_TARGET`, `SUSPECT_RUST_V3_HTTP_TARGET` and
`SUSPECT_RUST_RESOURCES_TARGET` isolate native build caches. Each native test
appends its commands, stdout, stderr and status to a `commands.log` in its work
directory.

## Source files

Paths below are relative to `crates/suspect-codegen/src/`:

- `rust_validation.rs` and `rust_validation/runtime_v3.rs` (the separate v3
  runtime).
- `rust_models.rs` and `rust_models/resources.rs` (profile/carrier admission).
- `rust_codecs.rs`.
- `rust_http.rs`, `rust_http/plan.rs`, `rust_http/emit/descriptors.rs`,
  `rust_http/runtime/descriptors.rs`, `rust_http/runtime/servers.rs` and
  `rust_http/emit/docs.rs` (generated resource guide).
- `compatibility/native.rs` (Rust capture section).

The modules are registered in `rust_http::source_assets()`, so they participate
in the generator's provenance hash.
