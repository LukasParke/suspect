# Go resource/dynamic schema-v3 adoption

Completed native adoption of the stable contract in
[SDK-SCHEMA-RESOURCES.md](SDK-SCHEMA-RESOURCES.md), after the
[Go scoped-v2 checkpoint](SDK-GO-SCHEMA-V2.md). Native evidence uses Go **1.23.12**
and **1.27.1**, with original physical source ownership retained.

## API and profile choice

The existing public Go generation entrypoints remain the entrypoints:

- `go_validation::emit` accepts the checked exact v1, v2 and v3 version/profile
  pairs. `OwnedProgram::check()` runs before any artifact is returned. Old
  envelopes reject resource context and DynamicRef; malformed context, targets,
  aliases and bindings remain located refusals.
- `go_codecs::plan_codecs` selects **`compile_v3` explicitly only when the
  effective closure needs canonical-resource/dynamic semantics**. Other closures
  still use `compile_v2`, which preserves their existing v1 or v2 programs.
  Base programs do not acquire a `resourceContext` field.
- `CodecPlan::validation_program()` (introduced in Go v2 adoption) remains the
  typed source for version/profile/resource metadata. No new public profile
  selector is required for canonical backend dispatch.
- The Go HTTP planner admits `SchemaResources` and `DynamicSchemaReferences`
  after the native profile witnesses. It uses `codec_schema_closure()` for
  candidate-aware admission checks and keeps `codec_roots()` as the actual codec
  input identities. V3 examples use `plan_protocol_examples_v3`; v1/v2 examples
  retain their respective existing helpers.
- `DocumentRelativeServers` has its independent physical-base witness in
  `go_schema_v2::native_physical_document_base_redirect_and_encoded_path_witness`.

Public native validation still uses `Validate`, `ValidationError` and
`ValidationFinding`. No URI loader, acquisition hook or dynamic-target selector
is exposed or invoked at runtime. The exact pair is:

```text
suspect.validation.experimental.v3
oas31-jsonschema202012-resources-dynamic
```

## Executable resource scope

The generated runtime consumes the compiler's resource table, aligned node-scope
tuples and indexed dynamic bindings. Schema entry uses the node's indexed
resource, including a nested entry whose resource root was never evaluated.

Only actually entered resources participate. The outermost matching entered
resource wins. The fallback resource is not entered before lookup; pointer,
empty-fragment and ordinary-anchor fallbacks have no dynamic name and retain
their initial target. Static Ref remains static.

The ordered first-distinct active resource list is restored on every return.
Context IDs are interned by the exact `(previous context, resource index)` pair;
cycle detection includes that context alongside node and instance identity.
Neither sibling trials nor subsequent calls inherit active resources.

V3 reuses the verified v2 annotation evaluator. A selected dynamic target gets a
fresh annotation scope and contributes only successful sets. Evaluation failure
is noninvertible. New resource entries and every inspected resource/binding are
charged exactly as specified; scope restoration and context-cache lookup add no
visits. Metadata is immutable and grants no acquisition authority.

## Native models and codec boundaries

Local named object fields retain their native structs and constructors. A dynamic
reference without a fixed local object representation uses a checked JSON-value
carrier with explicit nullable/absence states. The initial target is never used
as a generation-time type guess.

The complete selected v3 graph validates once at the public codec boundary.
Nested conversion does **not** restart validation at a child schema as if it were
an independent root: that would discard the caller's dynamic context and could
incorrectly apply a narrower fallback. Independently invoking the child's public
codec still establishes that child's own proper root context.

Mutable encoding, exact numbers, native field checks, omission/null invariants,
conversion budgets and JSON cycle protection remain active. The native witness
explicitly proves that an integer accepted by an outer override can be decoded
and encoded in its record, while the same dynamic-use codec independently accepts
its string fallback and rejects the integer.

## Source examples and documentation

V3 examples come from Main's bounded source-driven helper. Native construction
continues to use retained Go model descriptors. Ordinary JSON/text input records
have a single member binding with their validated entry index and container.

`OperationExamples.validated_aggregates` now supplies complete grouping for
source-declared JSON forms and multipart. Go uses that grouping for repeated
items, dynamic named extras, optional absence, empty arrays and positional tails.
It never fabricates a whole-object codec or turns JSON values into file contents.
A required empty repeated field can still fail the native wire representation;
the example preserves that exact value rather than inventing a nonempty one.

## Exact maintained selectors and results

All selectors below are in `crates/suspect-codegen/tests/go_schema_v3.rs` unless
another target is shown. Each native selector passed on both supported Go tiers.

| Selector | Result / scope |
| --- | --- |
| `official_44_unmodified_resource_cases_execute_with_closed_supplied_documents` | **44 official cases passed** per tier. The maintained test compiles the original dynamicRef source file and all three supplied remote documents through `compile_v3`; no target report is loaded. |
| `malformed_v3_contexts_never_emit_and_old_envelopes_reject_resources` | Eight context/binding/target/URI mutations refused, plus v1/v2 resource and dynamic-op fences. |
| `independent_dynamic_scope_branch_cycle_budget_and_annotation_controls` | **16 independent controls passed** per tier: outermost precedence, inert candidates, fallback kinds, nested entry, restoration, changed-context reentry, exact resource/binding work, numeric failure and annotation isolation/propagation. |
| `native_codec_boundaries_preserve_outer_dynamic_context_and_base_profile_selection` | Context-aware decode/mutable encode and independent-root fallback behavior passed; ordinary programs retain v1/v2 and absent resource metadata. |
| `installed_resource_sdk_keeps_typed_fields_dynamic_wire_checks_and_examples` | Installed SDK, native constructors/types, request/response checks, strict recursive tree, exact integers, encoded wire paths, source examples and rendered docs passed. SDK consumer runs with `-race`; Sphinx runs with warnings as errors. |
| `native_distinct_resource_depth_and_concurrent_contexts_are_bounded` | 550-resource chain reaches the explicit 512 depth failure on a normal native stack; later and concurrent contexts remain isolated. |
| `go_examples_aggregate::declared_form_and_positional_groups_keep_items_extras_and_absence` | Grouped native form/MIME wire recipes preserve distinct repeated values, extras, optional absence, empty arrays and positional tails on both tiers. |

The original unchanged m3 host selector is
`m3_native_docs::examples_bind_real_slots_and_report_missing_required_values`.
The earlier Go v2 source/codec/context results and the v1/base/protocol evidence
retain their completed scope; those matrices were not replayed for restarts or
format/lint-only changes.

Reproduction:

```sh
SUSPECT_SPHINX_PYTHON=/path/to/sphinx-python \
  cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test go_schema_v3 -- --include-ignored --nocapture

cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test go_examples_aggregate -- --ignored --nocapture
```

The default native test runs both toolchains. `SUSPECT_GO_TOOLCHAIN` can select
one explicitly. The scoped quality command is:

```sh
cargo clippy --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --lib -- -D warnings -A dead_code
```

It passed. The allowance is for the retained unrelated strict-adapter dead-code
warnings; the owned map-keys iteration finding was repaired.

## Final production asset handoff

New generated v3 production asset:

- **`go/validation_resources.go`**, from
  `crates/suspect-codegen/src/go_validation/resources.go`.

Existing generated assets whose v3 content/profile can change:

- `go/validation.go`, `go/validation_scoped.go`, `go/validation_program.json`;
- `go/models.go`, `go/codec_runtime.go`, `go/codecs.go`, `go/codec-plan.json`;
- Go HTTP runtime, operation, manifest, source-binding, README and example/docs
  artifacts driven by those retained plans.

The two v2 additions remain `go/validation_scoped.go` and
`go/validation_scoped_pattern.go`. Shared registration/options/provenance files
were not edited by this owner. The Go-only compatibility capture retains the
actual validation profile and native representations from typed plans.

## Remaining boundaries

This is the checked resource/dynamic profile, not full dialect conformance.
Custom vocabularies/dialects, legacy recursive-reference forms, the existing
portable regex/format limitations and unsupported directional profiles retain
located refusals. Dynamic and heterogeneous values can be broader native JSON
carriers whose source codec is mandatory; static Go types do not prove arbitrary
schema predicates. Runtime acquisition, business defaults, retries and schema
rewriting are not inferred.
