# TypeScript scoped validation: v2 native adoption

Verified on 2026-09-10. The TypeScript validator implements all nine operations
in [the shared v2 contract](SDK-SCHEMA-APPLICATORS.md). `compile_v2` supplies the
checked native codec program; ordinary base closures retain their v1 program.
Standalone model rendering retains `applicator-validation-required` obligations.

## Native behavior

- Each child starts with fresh evaluated-property/item sets. Only the specified
  successful annotations propagate through refs, compositions and dependencies.
  `if` and `contains` retain their specified local contributions before adjacent
  assertions; failed enclosing schemas export no annotations.
- Trials share numeric/equality/work/depth limits and preserve noninvertible
  evaluation failure. Object visits and set merges use Unicode-scalar key order.
  Duplicate merge candidates still consume work.
- All matching property patterns apply. Pattern-aware additional properties
  exclude matched names even when a matching value schema fails. Property-name
  validation has independent key identity and ordinary escaped instance paths.
- Known fields keep native types. Pattern-matched extras and heterogeneous
  prefix items use checked `JsonValue` carriers. Every allowed key and exact
  number survives conversion; absence and null stay distinct. Encoding validates
  the current wire value after mutation.
- Public emission and runtime initialization check exact version/profile pairs,
  instruction operands, targets, source relationships and limits. V2 instructions
  cannot enter v1 envelopes. Root-specific emission retains every reachable
  conditional, dependency, pattern, contains and unevaluated target.

HTTP planning uses `examples::plan_protocol_examples_v2`. Declared examples keep
their canonical slot and provenance; invalid examples produce located findings.
Native values, first requests and validated examples compile with the package.
TypeDoc renders the actual SDK/model/codec declarations and scoped/mutability
contract, including anonymous clients and structural JSON-value carriers.

`CodecPlan::validation_profile()` returns the actual executable version/profile
pair. `CodecPlan::interfaces()` retains each allocated model's checked codec
graph, conversion/validation limits and encoding policy. Compatibility capture
uses these typed records alongside native declarations. Root-local graph indices
exclude physical source/prose changes; exact literal instance objects retain
their own `source`, `description`, `schema` and `span` keys. `Location` stays on
the containing native model/operation record.

## Immutable acceptance checkpoint

`target/sdk-typescript-v2-final-01.log`: **7 passed, 0 failed**.
Artifacts are retained under `target/sdk-typescript-v2-final-01/`:

- `vectors-lADx1X`: all **32 unmodified source-driven cases**, compiled through
  real `compile_v2`, with original source/instance-location checks and root-slice
  parity; TypeScript **5.5.4 / 5.9.3**, Node **22.23.1 / 24.21.0**.
- `controls-jjZP80`: ten independent native budget/identity/base-v1 controls,
  accessor/observer/mutation and malformed-program checks on both compilers and
  runtimes.
- `installed-sdk-JNDkoG`: four actual operations covering all nine new ops;
  separate locally packed and installed compiler-floor/current tarballs,
  executable SDK calls and examples on both Node versions, meaningful negative
  strict consumer types, and rendered TypeDoc symbol/source/link coverage.
- `browser-AgeQE7`: Chromium **153** executes all 32 vectors and actual Fetch SDK
  calls, exact request bytes, response rejection, mutation, absence/null and abort
  controls.
- `capture-XjnPpn`: typed native snapshot. Located invalid-example and
  relocation/prose/literal/constraint-mutation comparisons passed too.

Earlier red attempts and the initial v2 admission-fence proof are retained.
The completed v1 protocol/M2/native matrices were not repeated for this tranche.

## Exact maintained selectors

All are in `--test typescript_applicators`:

1. `v2_instructions_cannot_enter_v1_or_unknown_program_envelopes`
2. `source_driven_v2_vectors_preserve_scope_failures_and_original_locations`
3. `scoped_work_identity_and_mutation_controls_are_native_failures_not_mismatches`
4. `scoped_sdk_capture_tracks_checked_graphs_and_preserves_literal_instance_data`
5. `scoped_http_examples_keep_valid_declared_values_and_locate_invalid_ones`
6. `installed_scoped_sdk_preserves_models_values_examples_and_docs` — ignored
7. `browser_scoped_sdk_executes_source_vectors_and_native_http_controls` — ignored

```sh
SUSPECT_DOCS_NODE=/path/to/node22 \
SUSPECT_NODE24_BIN=/path/to/node24 \
SUSPECT_TYPESCRIPT_V2_MATRIX=1 \
SUSPECT_TYPESCRIPT_V2_ARTIFACTS=/absolute/path/to/new-evidence-directory \
cargo test -p suspect-codegen --no-default-features --features http-protocol \
  --test typescript_applicators -- --include-ignored --nocapture
```

`SUSPECT_TYPESCRIPT_V2_MATRIX=1` selects the reviewed installed compiler files at
`tools/typescript-floor/node_modules/typescript/bin/tsc` (5.5.4) and
`tools/typescript-docs/node_modules/typescript/bin/tsc` (5.9.3), relative to
`crates/suspect-codegen`. Without it, the validator-only tests use `tsc` on PATH.
The installed-package witness provisions its isolated compilers with offline
`npm ci`. `SUSPECT_CHROMIUM` overrides the default macOS Chrome executable.
Evidence directories receive unique suffixes and are retained even on failure.

Production changes use existing assets: `typescript.rs`,
`typescript/{validation.rs,validation.ts,codecs.rs,http.rs,package.rs}`, and only
the TypeScript capture function in `compatibility/native.rs`. The native docs
checker is `tools/typescript-docs/build.mjs`. No new production runtime module was
introduced for v2. The language-owned HTTP `source_assets()` inventory is merged
by Main's canonical provenance hash, with no supplemental capture hash.
