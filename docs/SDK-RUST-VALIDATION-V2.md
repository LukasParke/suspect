# Rust scoped-applicator validation v2

Rust now has an additive native implementation of the checked v2 contract in
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md). It uses the actual
`OwnedCompiler::compile_v2` program, original schema IDs and portable NFAs.

## Public entry points

```rust,ignore
use suspect_codegen::{rust_models, rust_codecs, rust_http, rust_validation};
use suspect_schema::{Config, OwnedCompiler};

let compiled = OwnedCompiler::new(Config::default())
    .compile_v2(contract.clone(), &schema_roots)?;
let validation_files = rust_validation::emit(&compiled.program())?;

let models = rust_models::plan_models_v2(&contract, &schema_roots);
let codecs = rust_codecs::plan_codecs_v2(
    contract.clone(), &schema_roots, Default::default(),
)?;
let version = codecs.validation_version();
let profile = codecs.validation_profile();

let http = rust_http::plan_http_v2(
    contract, &operation_sources, Default::default(),
)?;
```

The existing `plan_models`, `plan_codecs` and `plan_http` entry points retain
their v1 admission. `plan_http_v2` uses `plan_codecs_v2` and Main's
`examples::plan_protocol_examples_v2`; v2 HTTP examples do not pass through the
v1 schema compiler. Source example roles, declared provenance and invalid
findings remain intact, and native input construction still belongs to Rust.

`rust_validation::emit` first calls `OwnedProgram::check`. It recognizes the
exact v1/v2 version/profile pairs, checks native metadata/depth limits, and emits
all nine v2 instructions as typed native operations. Invalid versions, targets,
source identities, counts, patterns and instruction order fail before artifacts.
V1/v2 envelopes still refuse resource metadata and `DynamicRef`. The separate
[native v3 resource profile](SDK-RUST-RESOURCES.md) is now implemented through
explicit v3 model/codec/HTTP entry points; it does not change v2 admission.

## Native semantics

- **Conditionals:** the condition is one isolated trial. Its mismatches are
  discarded; its evaluation failure propagates. Only the selected then/else
  branch runs. A successful condition contributes annotations independently of
  a failing then branch, but never supplies the branch's initial scope.
- **Dependencies:** null-valued members are present. `dependentRequired` checks
  names and produces no evaluated-property annotations. Every triggered
  `dependentSchemas` target evaluates the whole object in a fresh scope.
- **Contains:** every item trial finishes or returns evaluation failure, even
  after a match or an exceeded maximum. Matched indices are retained. The
  implicit minimum 1 has no invented numeric operand. Explicit minimum/maximum
  tokens use exact arithmetic and retain their sibling keyword sources. Contains
  annotations are independent of minContains/maxContains assertion failure.
- **Patterns:** every matching property pattern applies, including overlaps with
  named properties. Pattern-aware additionalProperties excludes names matched by
  an adjacent pattern even when its value schema failed. The runtime uses the
  compiled NFA, never a second regular-expression parser.
- **Property names:** decoded names are temporary JSON strings evaluated through
  the complete validator. Findings use the associated member's escaped instance
  path; names do not mark property values as evaluated.
- **Unevaluated locations:** these checks run after other checks at the same
  schema node. Each evaluation starts empty and tracks only its own instance
  level. Properties/items mark immediate members, not descendants. Successful
  references and same-instance applicators propagate sets. anyOf merges every
  passing branch, oneOf only its unique passing branch, and not exports none.
  Failed schemas export empty sets. Siblings and cousins cannot consume one
  another's local annotations.

Schema entries, checks, collection visits, NFA transitions and annotation merge
insertions spend the shared evaluation allowance. Duplicate merge candidates
are charged too. Invalid findings do not stop later checks, and a finding cap
does not hide a later resource failure. Codec sessions preserve budgets across
root checks, branch selection and equality, but never preserve annotation sets
between calls.

The public outcomes remain `Valid`, `Invalid(Vec<ValidationFinding>)`, and
`EvaluationFailure(ValidationFinding)`. Codecs preserve the distinction between
`CodecError::Invalid`, `EvaluationFailure`, JSON and conversion errors. Sources
and decoded instance paths survive conditional, pattern, name and reference
contexts.

## Native carriers and mutable encoding

`plan_models_v2` retains source constraints as codec obligations. Models alone
are candidates, not proof that a value is schema-valid.

- Ordinary named properties and unconditional required constructor arguments
  retain their native types. Conditional requiredness is not flattened into
  unconditional fields.
- Patterned extras use exact `JsonValue` independently of unmatched
  additionalProperties. A matching key is representable even when
  additionalProperties is false; unmatched keys still fail the whole-root codec.
- Objects with only patterns use `BTreeMap<String, JsonValue>`. Decoded Unicode
  keys are not normalized, and keys such as `__proto__`, `constructor`,
  `_extra_fields`, and `a/b~` have no special runtime meaning.
- Prefix arrays preserve heterogeneous values as `Vec<JsonValue>` rather than
  imposing the tail's item type on prefix positions. Homogeneous arrays keep
  their existing native item types.
- Untyped conditionals and intersections use a faithful exact JSON carrier
  instead of inventing an object/scalar type. The finite Rust nullability proof
  accounts for the selected null branch; an incomplete proof is a planning error.
- Ref assertion siblings, conditional constraints, dependencies, contains,
  property names and unevaluated rules are checked on the complete source root
  during decode and again during mutable encoding. A broader carrier never
  removes an assertion from the program.

Existing unsupported native representations and schema profiles remain
source-linked planner errors. Exact numbers do not go through f64; no default,
coercion or field stripping is introduced.

## Frozen v1 and native depth

`rust_validation/runtime.rs`, `number.rs`, and `pattern.rs` remain the existing
v1/runtime arithmetic assets. V2 selects the separate `runtime_v2.rs` asset;
it does not run both evaluators. A base closure compiled with `compile_v2`
retains a v1 program and emits identical validation files. Base v1/v2 codec
packages have a byte-for-byte comparison gate as well.

The admitted 512-depth policy remains unchanged. The v2 recursive evaluator
uses boxed internal outcomes and a separate entry guard so debug-build stack
frames do not grow with the large instruction/error union. The public result
types remain unboxed. Native tests use an explicit 2 MiB thread stack and test
both productive recursion and the exact 512/513 boundary; exceeding the policy
must produce an evaluation failure rather than abort the process.

## Test seams and commands

`crates/suspect-codegen/tests/rust_validation_v2.rs` uses:

1. The schema owner's independent shared v2 fixture plus Rust-specific literal
   valid/invalid/resource vectors in `fixtures/rust-validation-v2.json`.
2. **395 executable official JSON Schema Test Suite cases**, using unmodified
   standalone schema roots. Six dynamic-reference/Unicode-property-escape cases
   assert the documented source-level refusal rather than being counted as native
   passes.
3. Real private Cargo archives extracted into fresh installed consumers, native
   models/codecs, meaningful compile-fail consumers and built Rustdoc.
4. Mutation, source/path identity, recursive scopes, exact counts, overlapping
   patterns, Unicode/colliding keys, shared sessions and finite resource failures.
5. An additive HTTP sample gate that compiles native constructor docs and executes
   examples through the actual v2 codecs.

```sh
cargo test --locked --offline -p suspect-codegen --test rust_validation_v2 \
  -- --include-ignored --nocapture --test-threads=1

SUSPECT_NATIVE_RUST_TOOLCHAIN=1.88.0 cargo test --locked --offline \
  -p suspect-codegen --test rust_validation_v2 \
  -- --include-ignored --nocapture --test-threads=1
```

`SUSPECT_RUST_V2_TARGET` can select an isolated native build cache. Attempts,
including successful packages and consumers, are retained for inspection; the
gate prints each location. The implementation uses the live new program API,
including when an isolated Rust-only harness is needed during other owners'
aggregate edits. The previously completed Rust HTTP protocol matrix is separate
evidence and is not repeated for this tranche.

## Production asset handoff

- `crates/suspect-codegen/src/rust_validation.rs`
- `crates/suspect-codegen/src/rust_validation/runtime_v2.rs` (new)
- `crates/suspect-codegen/src/rust_models.rs`
- `crates/suspect-codegen/src/rust_models/applicators.rs` (new)
- `crates/suspect-codegen/src/rust_codecs.rs`
- `crates/suspect-codegen/src/rust_http.rs`
- `crates/suspect-codegen/src/rust_http/plan.rs`
- `crates/suspect-codegen/src/compatibility/native.rs` — Rust capture only;
  supplemental hashing removed in favor of Main's shared provenance framing.

The shared program, IR/schema engine, Python-model annotation change and other
native adapters are owned by their existing sessions. The language-owned
`rust_http::source_assets()` inventory includes the two new production modules;
Main's provenance module owns sorted/deduplicated length-delimited hashing for
both planned and empty snapshots.

## Completed verification — 2026-09-10

- **10/10 v2 integration tests passed** on current **Rust 1.97.1** and native
  **Rust 1.88.0**, with no ignored tests in the executed tranche.
- The installed validation program gate executed the shared independent
  fixtures and additional Rust adversarial vectors; no expected result was
  derived from the source evaluator.
- **395 official executable cases passed**; six specifically identified
  dynamic/NFA cases verified source-level refusal.
- Native packages, installed positive/negative codec consumers, mutable encoding,
  zero-budget failures, built Rustdoc, and v2 HTTP constructor examples passed.
- The 2 MiB-stack gate passed productive recursion and exact 512/513-depth
  behavior. Its original overflowing attempt and assembly diagnostics remain
  retained; the fix changes only the v2 runtime's internal outcome storage.
- **5/5 focused v1 validation regression tests passed**, including exact numbers,
  patterns, logical failure propagation, shared sessions and the original depth
  ceiling. Base v1/v2 program and codec artifact bytes compare equal.
- Live `cargo check --locked --offline -p suspect-codegen` passed after the
  aggregate owners' intervening edits cleared.

The live new schema/program API was used throughout. During aggregate edits,
native execution used the existing isolated Rust module harness, not the frozen
v1 dependency snapshot. The completed HTTP protocol matrix was not repeated.

### Exact focused selectors

All are in `--test rust_validation_v2`:

- `v2_program_admission_is_source_linked_and_rejects_malformed_variants`
- `unused_v2_features_keep_v1_emission_bytes`
- `independent_v2_programs_run_through_installed_native_validation`
- `official_applicators_run_against_native_v2_with_unmodified_schema_roots`
- `v2_recursive_depth_and_codec_sessions_have_finite_isolated_scopes`
- `v2_model_carriers_retain_constraints_and_base_codecs_keep_their_bytes`
- `installed_v2_codecs_validate_mutable_native_carriers_and_negative_consumers`
- `installed_v2_codec_resource_failures_are_never_invalid_or_representation_success`
- `scoped_http_examples_bind_the_v2_program_and_preserve_invalid_source_findings`
- `scoped_http_native_constructor_examples_compile_and_execute`

Native attempts print their retained locations. Completed run logs in the
session's OpenCode shell directory are:

- `sh_08d329767001d0buO4TjW4fFyP.out` — complete current-toolchain v2 tranche.
- `sh_08d385d16001e4oXYrSWolHrZ2.out` — complete Rust 1.88 native v2 tranche.
- `sh_08d33c3af0019JExXjJbJACHJh.out` — focused v1 validation regressions.
