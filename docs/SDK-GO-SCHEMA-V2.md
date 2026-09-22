# Go scoped schema-v2 adoption

Go validation executes the stable nine-op contract in
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md). This is additive to the
previous Go protocol/base acceptance scope.

## Public APIs and selection

- `go_validation::emit(&OwnedProgram)` accepts checked v1 and scoped-applicator
  v2 programs. It runs `OwnedProgram::check()` and enforces the native resource
  bounds before returning artifacts. Unknown/mismatched profiles, malformed
  operands and unverified resource/dynamic v3 programs are refused.
- `go_codecs::plan_codecs` uses `OwnedCompiler::compile_v2`. The compiler retains
  the exact v1 program representation when the selected closure needs no new
  operation. The native runtime dispatches v1 to its existing evaluator.
- `CodecPlan::validation_program()` exposes the actual checked program, including
  its version/profile and physical source roots, for typed tooling and evidence.
- The generated `codec-plan.json` records its validation version/profile. The
  native codec loader checks that profile and every model's source/root against
  the embedded validation program.
- Public Go `Validate(root, Value)` retains `*ValidationError` and its
  `invalid` versus `evaluation_failure` distinction. V2 mismatch reports expose
  ordered `ValidationError.Findings []ValidationFinding`; each finding has
  `Source`, `InstancePath` and `Message`. `MaxErrors == 0` means unlimited reports.
  Reporting limits never hide later evaluation failures.
- The Go HTTP planner selects `plan_protocol_examples_v2` for an actual v2 codec
  program, and retains `plan_protocol_examples` for v1 programs. The native
  compatibility capture records the typed validation profile and error names.

## Execution contract

All nine instructions are implemented: `if`, `dependentRequired`,
`dependentSchemas`, `contains`, `patternProperties`,
`additionalPropertiesWithPatterns`, `propertyNames`, `unevaluatedProperties`
and `unevaluatedItems`.

Each evaluation owns fresh property/index sets. Ref and successful same-instance
applicators propagate sets; member applicators mark only immediate members.
AnyOf includes every passing branch; oneOf publishes only its unique passing
branch. Failed schemas export no sets. Trial evaluation diverts mismatch reports
while retaining the same depth, active-identity, work and equality allowances.

Passing `if` condition annotations enter the current local scope independently of
the selected branch's outcome. Contains examines every element, retains all
matches, and contributes its successful annotation independently of min/max
count validity. The implicit minimum of one is not invented as a numeric operand.
Explicit count errors retain the real minContains/maxContains source locations.

Map visits and set merges are decoded Unicode lexical order, without
normalization. Each attempted set insertion is charged, including duplicates.
The v2 NFA preserves the contract's position/enqueue/pop/range-test schedule;
equality retains a single shared allowance and pending-comparison preflight.
Names used as temporary string instances have separate identities. Active
container identity detects cycles while permitting shared acyclic values.

Exact number tokens remain exact on parsing, validation, native conversion and
mutable encoding. No branch failure can be inverted into validity by `not`,
condition selection, prior matches, an exceeded contains maximum or an error cap.

## Native representations

Explicit object schemas retain their named Go fields, ordinary required-field
constructors and absence/null wrappers. Pattern-matched extras remain available
through `SetExtra` and `Extra()` even with `additionalProperties: false`.
Pattern extras use checked `Value` carriers because different and overlapping
patterns can constrain a key differently. Declared names cannot be shadowed.

Unions/intersections that have no faithful single static shape use a checked JSON
value carrier. Scoped prefix-item arrays use `[]Value` rather than incorrectly
applying the tail `items` type to every position. Conditional nullability is
proved for the null instance where possible; incomplete finite analysis retains
a nullable carrier and full codec checks rather than inventing a non-null type.
Boolean-false/uninhabited scoped subschemas can have a runtime-checked carrier;
their codec still rejects every value.

Decoding preserves pattern extras and exact numeric tokens. Both `Encode` and
`EncodeValue`, as well as `encoding/json` adapters, revalidate mutable models.
Model-only plans continue to carry explicit codec obligations and do not certify
validated construction on their own.

### Remaining representational boundaries

- The checked carrier is intentionally broader than the validated domain for
  heterogeneous composition, untyped conditional constraints, overlapping
  pattern domains and uninhabited schemas. Its source codec remains mandatory.
- Static Go types do not encode cross-property dependencies, contains counts,
  evaluated-location sets or arbitrary schema predicates.
- Base-only representation limits retain their earlier scope; this tranche does
  not turn every prior model refusal into a new static type.
- Unsupported dialect/vocabulary, resource/dynamic and directional HTTP profiles
  retain located refusals. `SchemaResources` and `DynamicSchemaReferences` remain
  disabled in Go HTTP, and owned-v3 execution is not claimed.

## Example-binding regression

An ordinary JSON/text request body previously produced separate construction and
selection records, neither carrying its actual ExamplePlan entry index. It now
has one member binding containing `entry`, physical `container`, schema, origin,
construction and concrete media selection. Closed media choices follow the same
rule. Byte, aggregate and part recipes remain explicit where no whole-body JSON
entry exists. The existing m3 slot count, omission and provenance assertions were
kept intact, including the established missing-required-example reason.

## Required resource-handoff adapter changes

Go consumes `ServerPlan::document_base()` for default relative resolution.
Caller `DocumentURL` remains an explicit override. Encoded dot/slash path segments
are preserved instead of being treated as decoded traversal segments. The native
redirect witness uses different requested, effective physical and logical `$self`
addresses, and checks exact URI bytes on both Go toolchains.

`Server.DocumentBase` is physical metadata. `CredentialRequest.ServerURL` exposes
the selected effective API server to caller OAuth/OIDC hooks; `URLBase` on
`SecurityRequirement` and `OAuthFlow` preserves the shared effective-server rule
while URLs themselves remain raw metadata. These hooks acquire nothing.
Logical resource contexts remain distinct in the embedded shared ProtocolPlan;
they do not replace physical source ownership or diagnostic identities.

## Maintained native witnesses

The maintained tests compile source schemas through the real `compile_v2` API.
They do not load the frozen `target/sdk-schema-applicators-executable-v2.json`.

| Exact selector in `tests/go_schema_v2.rs` | Evidence |
| --- | --- |
| `ordinary_media_examples_have_one_entry_binding_and_rich_recipes_remain_explicit` | Single JSON/text/choice entry bindings; byte and part recipes remain distinct. |
| `checked_program_fences_and_base_v1_program_identity` | Unknown/mismatched/v1-with-v2 envelopes and invalid targets refused; base program equality and v1 codec selection. |
| `scoped_32_source_vectors_execute_in_installed_consumers` | All 32 independent source vectors plus native work/report/key-identity controls; exact outcome, physical document, source pointer and instance pointer. Includes exact NFA merge thresholds and reporting-cap controls. |
| `scoped_models_codecs_and_sdk_operations_preserve_native_data` | Installed SDK, named record fields, overlapping pattern extras, nullable/absent states, exact multipleOf/numbers, mutable encode, carriers, positive/negative native consumer types, source examples, native docs and optional Sphinx. |
| `scoped_native_recursion_depth_and_call_isolation` | `-race` on both toolchains: normal-stack depth refusal, cyclic input, shared acyclic input and concurrent per-call scopes. |
| `native_physical_document_base_redirect_and_encoded_path_witness` | Effective physical retrieval versus requested/logical addresses, caller override, encoded path spelling, caller OAuth URL-base metadata and schema-resource refusal. |

The existing host selector
`m3_native_docs::examples_bind_real_slots_and_report_missing_required_values`
also passes after the binding fix. It was the original regression loop, not
replaced by a weaker assertion.

Native toolchains: **Go 1.23.12 and Go 1.27.1**. Scoped SDK examples run from the
generated package; consumers install it in an independent module via `replace`.
The synthetic SDK's record and sequence operations exercise all nine operations
through the actual Go HTTP planner, codecs and native transport entry point.
No OpenRouter operation is labeled a v2 witness merely because it appeared in an
earlier, unaffected protocol/base corpus gate.

Commands (add `SUSPECT_GO_TOOLCHAIN` to select one tier):

```sh
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test m3_native_docs \
  examples_bind_real_slots_and_report_missing_required_values -- --nocapture

SUSPECT_SPHINX_PYTHON=/path/to/sphinx-python \
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test go_schema_v2 \
  -- --include-ignored --nocapture
```

The minimal feature selection kept this Go gate runnable while other native
adapters and the shared owned-v3 implementation were mid-edit. Earlier completed
v1/base/protocol matrices were not restarted for this adoption.

## Production asset handoff

New generated production assets:

- `go/validation_scoped.go`
- `go/validation_scoped_pattern.go`

Changed generated assets/surfaces:

- `go/validation.go`: guarded v1/v2 dispatch, new typed operands and findings.
- `go/validation_program.json`: exact selected v1/v2 checked program.
- `go/models.go`: faithful scoped representations and pattern extras.
- `go/codec_runtime.go`, `go/codecs.go`, `go/codec-plan.json`: profile-bound codecs.
- Go HTTP operation/runtime/metadata/docs/example artifacts inherit those typed
  plans, body-binding metadata and physical-document-base changes.

Owned generator source additions are `go_validation/scoped.go` and
`go_validation/scoped_pattern.go`. Generator edits are confined to Go validators,
models/codecs, the affected Go HTTP integration and the Go-only compatibility
capture. Shared registry/options/provenance ownership remains with Main.
