# TypeScript resources and dynamic references: v3 native adoption

Verified 2026-09-10 against [the shared v3 contract](SDK-SCHEMA-RESOURCES.md).
The native validator, model codecs, HTTP admission, source examples, installed
packages, strict consumer types, rendered documentation and Chromium witnesses
are implemented. Earlier [v2 evidence](SDK-TYPESCRIPT-VALIDATION-V2.md) and the
completed protocol/M2 matrices remain preserved.

## Entry points and admission

The public `typescript::validation::emit`, `plan_models`,
`codecs::plan_codecs[_with_views]`, `http::plan_http` and package APIs remain the
entry points. `CodecPlan::validation_profile()` reports the actual pair:

| Selected source closure | Executable program |
| --- | --- |
| Ordinary base assertions | `suspect.validation.experimental.v1` / `oas31-jsonschema202012-static-subset` |
| Scoped applicators without resource requirements | `suspect.validation.experimental.v2` / `oas31-jsonschema202012-static-applicators` |
| Canonical resources or dynamic references | `suspect.validation.experimental.v3` / `oas31-jsonschema202012-resources-dynamic` |

The native planner explicitly invokes `compile_v3` only for resource requirements
in Contract's effective, candidate-aware schema closure. Other closures use
`compile_v2`, which retains v1 for ordinary assertions. Model-only APIs retain
located `resource-validation-required` and applicator codec obligations.

ExpandedV1 HTTP admission now enumerates `SchemaResources`,
`DynamicSchemaReferences` and separately witnessed `DocumentRelativeServers`.
The library's StrictJson default is preserved. Canonical generation and native
capture still share `backend::typescript_options`.

## Execution, representation and ownership

- V3 retains the checked `resourceContext` and `DynamicRef` operands. Entering a
  nested node enters its indexed resource without evaluating an unselected
  resource root. The outermost actually entered matching binding wins.
- The initial target's resource is not entered before lookup. Pointer, empty and
  static-anchor fallbacks retain their initial target. Unentered candidates are
  inert. Every return, mismatch, failure and trial restores scope.
- Cycle detection includes exact ordered resource-context identity. New distinct
  resource entries and each inspected resource/binding spend the shared work
  allowance. Dynamic targets start with fresh v2 annotation scopes and propagate
  only successful sets. Numeric, equality, depth and work failures remain
  noninvertible.
- Program guards check version/profile pairs, dense v3 node/scope alignment,
  physical containment, canonical/base/alias URI consistency, declaration and
  anchor locations, binding targets and initial-resource agreement. V1/V2 reject
  resource metadata and DynamicRef. Exact literal instance data stays opaque.
  Metadata is an acyclic data-property graph; accessors are rejected without
  invocation before initialization/freezing. URI and anchor checks reject final
  line terminators as well as malformed escapes and names.
- Known native fields retain their types. Dynamic positions use checked
  `JsonValue` carriers. Decode/encode preserve exact numbers, every allowed key,
  absence and null. Encoding validates the current value after mutation.
- V3 codec traces are keyed by source, instance path **and resource context**.
  A failed condition in another binding context cannot overwrite a successful
  static model projection. Dynamic directional annotation applicability remains
  a located refusal; it is not inferred from a fallback binding.

V1/V2 root-specific validators retain their existing sparse static closures.
V3 root-specific validators retain the complete checked finite resource registry,
including candidate targets; only entered resources affect execution.
Source IDs and spans stay physical. Logical names and aliases remain separate
metadata and confer no runtime loading or acquisition behavior.

## Native HTTP and public metadata additions

Relative servers use `ServerPlan.document_base`, the effective physical retrieval
document containing the Server Object. An absent server array defaults from the
entry document; an explicit empty override belongs to the overriding document.
`server.documentURL` remains an explicit caller override.

The native RFC 3986 helper preserves encoded dots/slashes, percent spelling and
path case. Native Fetch URL rewrites are rejected before sending. A capable
explicit transport receives the exact URI. The installed witness sends literal
request targets and physical Host fields through a loopback virtual-host
transport, and exercises native Fetch with an explicit document-base override.
Chromium independently checks relative override resolution and encoded-dot refusal.

Public additions for Main/runner integration:

- `operations.ResourceContext`: readonly logical metadata alongside physical
  `SourceLocation`; `Provenance` exposes its use-site/terminal/reference contexts.
- `operations.ServerPlan.document_base`: physical document location.
- `operations.CredentialContext.effectiveServerURL`: selected API server base for
  relative OAuth/OIDC endpoint metadata. Original endpoint values remain intact.
- Standalone `ValidationTrace` has an optional fourth resource-context signature
  for v3. V1/V2 still invoke observers with exactly three arguments. The signature
  is meaningful within its checked program, not a cross-program schema identity.
- Standalone `ValidationProgram` admits the exact v3 pair and its optional
  `ValidationResourceContext`; resource metadata types are exported by
  `validation.ts`.

`plan_protocol_examples_v3` is selected for actual v3 codec plans. Declared
dynamic examples preserve their physical source and invalidity; native recipes
use checked model expressions. Shared part-example projections are consumed as
entries; grouped form/multipart data is never given a fabricated aggregate codec.

`CodecPlan::interfaces()` captures the checked graph with root-local node and
resource indices, bindings, validation/conversion limits and encoding policy.
Physical/prose relocation stays outside equality. Native model/operation
`Location` fields retain source ownership. Binding changes produce native model
changes through the common comparator.

## Retained evidence

All nine maintained tests in `typescript_resources` have passing focused
evidence. The combined non-ignored attempt retained one test-oracle mismatch:
Contract reports a legacy keyword at its containing SchemaId with the exact
keyword span. The targeted follow-up checks that original location and passes.
Completed native checks were retained rather than replayed for that assertion.

| Gate | Evidence under `target/` |
| --- | --- |
| All **44 unmodified official cases**, closed supplied remote documents, real `compile_v3`; Node **22.23.1 / 24.21.0**, TS **5.5.4 / 5.9.3** | `sdk-typescript-v3-final-01.log`, `sdk-typescript-v3-final-01/official-yIdhDs/` |
| **19 independent native cases**, scope/context cycles, nested entry, static fallbacks, exact resource/lookup budgets, failure controls, malformed metadata and RFC URI vectors; both compilers/runtimes | `sdk-typescript-v3-final-guards-03.log`, `sdk-typescript-v3-final-guards-03/` (adds terminal-line and metadata-accessor controls) |
| Base v1/v2 program/budget/three-argument trace parity and source-linked refusals | `sdk-typescript-v3-base-guards-03.log`, `sdk-typescript-v3-base-guards-03/` |
| Four actual resource/dynamic SDK operations; separate floor/current tarballs installed and executed on both Node versions; negative strict types, examples and TypeDoc | `sdk-typescript-v3-sdk-01.log`, `sdk-typescript-v3-sdk-01/installed-sdk-Yafkd1/` |
| Physical document/server/redirect ownership, absent/empty arrays, overrides, OAuth metadata base and literal encoded paths; installed floor/current packages on both runtimes | `sdk-typescript-physical-servers-03.log`, `sdk-typescript-physical-servers-03/physical-servers-kNznwW/` |
| Chromium **153**: all 44 source cases, dynamic SDK calls, context-aware codecs, exact bytes, mutation/abort/response controls, relative override and encoded-dot refusal | `sdk-typescript-v3-browser-02.log`, `sdk-typescript-v3-browser-02/browser-hYuf6Y/` |
| Typed resource capture, physical/prose relocation, binding changes and invalid declared examples | `sdk-typescript-v3-final-01.log`, `sdk-typescript-v3-final-01/capture-aIav4h/` |
| Focused protocol capture and URL-assembly regression | `sdk-typescript-protocol-compat-07.log`, `sdk-typescript-physical-query-regression-01.log` |
| Clippy for the library and owned v2/v3/capture targets | `sdk-typescript-v3-clippy-01.log` (`-D warnings -A dead_code`; the existing legacy HTTP projection's dead fields are excluded) |

Source fixtures are the unchanged files in
`crates/suspect-schema/tests/fixtures/resource-conformance/`, including their
three supplied remote documents, README hashes and license. Maintained tests do
not depend on the one-off executable report in `target/`.

### Exact selectors

Under `--test typescript_resources`:

1. `v3_metadata_and_dynamic_ops_cannot_enter_older_or_mismatched_envelopes`
2. `source_driven_v3_executes_all_44_official_cases_from_closed_supplied_documents`
3. `native_v3_scope_identity_lookup_budgets_and_malformed_metadata_controls`
4. `resource_selection_keeps_base_profiles_and_source_linked_refusals`
5. `resource_capture_uses_typed_context_graphs_and_keeps_physical_locations_separate`
6. `v3_example_admission_retains_dynamic_invalidity_and_declared_source`
7. `installed_v3_sdk_preserves_dynamic_models_exact_values_examples_and_native_docs` — ignored
8. `physical_document_server_bases_preserve_redirect_provenance_overrides_and_encoded_paths` — ignored
9. `browser_v3_executes_official_sources_dynamic_sdk_calls_and_contextual_codecs` — ignored

Selectors: `SUSPECT_DOCS_NODE`, `SUSPECT_NODE24_BIN`,
`SUSPECT_TYPESCRIPT_V3_MATRIX=1`, `SUSPECT_TYPESCRIPT_V3_ARTIFACTS` (absolute
retained-evidence parent), and optional `SUSPECT_CHROMIUM`. Matrix mode selects
the reviewed installed compiler files under
`crates/suspect-codegen/tools/{typescript-floor,typescript-docs}/node_modules/typescript/bin/tsc`.
Installed witnesses provision isolated compilers using offline `npm ci`.

## Production asset handoff

New production files:

- `crates/suspect-codegen/src/typescript/uri.ts`
- `crates/suspect-codegen/src/typescript/validation-resources.ts`

Both are emitted runtime dependencies and are listed by the language-owned
`typescript::http::source_assets()` inventory. Main's common provenance merges
that inventory once for planned and empty selections.

Changed production files in this tranche, relative to `suspect-codegen/src`:
`typescript.rs`, `typescript/validation.rs`, `typescript/validation.ts`,
`typescript/codecs.rs`, `typescript/codecs.ts`, `typescript/http.rs`,
`typescript/http/{runtime-protocol.ts,security.ts,types.ts,protocol_emit.rs}`,
and `typescript/package.rs`. V2's TypeScript-only capture changes in
`compatibility/native.rs` remain; no other capture function was edited.
Native documentation verification also uses `tools/typescript-docs/build.mjs`.

The earlier non-form-data multipart content-plan issue is closed by the shared
planner fix and the focused installed TypeScript request/response witness in
[the protocol report](SDK-TYPESCRIPT-PROTOCOL.md). Its defensive native guard
remains. Earlier checkpoint reports retain their original status and bytes.
