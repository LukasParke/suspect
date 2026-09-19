# Python portable validation

`python_validation::emit(&OwnedProgram)` accepts a checked, versioned program
compiled by `OwnedCompiler`. It dispatches the exact v1, v2 or v3 version/profile.
Every profile emits `validation.py`, `validation_number.py` and
`validation_program.json`; the exact JSON runtime is supplied by `python_json`.
The runtime executes compiled instructions with retained source addresses.

`validate(root_node, value)` starts a fresh `ValidationSession`. Codec branch
trials use a shared session so trial failures cannot reset evaluation/equality
budgets. `ValidationError.kind` distinguishes `invalid` from
`evaluation_failure`; errors carry source and instance paths.

Evaluation uses an explicit generator driver, including recursive/composed
schemas, without changing Python's global recursion limit. Decimal arithmetic
compares symbolic coefficients/exponents and exact divisibility with finite numeric
work. The portable pattern executor preserves the compiler's strict-anchor profile.

Program metadata is checked before emission. The Python profile caps schema depth
at 512 and numeric operand length at 65,536 bytes; JSON/model conversion have their
own smaller configurable ceilings.

The independent versioned vectors in
`crates/suspect-codegen/tests/fixtures/runtime-contract-v1.json` cover 17 schema
cases: exact numbers/integrality/bounds/divisibility, numeric structural equality,
exclusive/inclusive unions and sibling assertions, negation, presence/extras,
uniqueness, Unicode cardinality, strict patterns, tuples and Boolean schemas.
Expected results are hand-authored, including an explicit distinction between
invalid input and incomplete evaluation. Python, Go, Rust and TypeScript run these
same vectors; backend-specific resource/reference tests supplement them.

```sh
cargo test --locked -p suspect-codegen --test python_validation -- --include-ignored
```

`SUSPECT_PYTHON_BIN` selects the native interpreter. Full JSON Schema coverage is
bounded by successful owned-program compilation and each emitter's profile.

## Scoped v2 execution

The additive v2 runtime implements all nine instructions from
[the applicator contract](SDK-SCHEMA-APPLICATORS.md). Each schema evaluation owns
fresh evaluated-property/item sets. Successful same-instance applicators merge
their permitted sets; failed schemas export none. Immediate members, decoded
Unicode key order, trial findings and every duplicate merge candidate follow the
published visit-cost rules. Numeric, equality, depth and work failures remain
noninvertible across all branch trials.

V2 additionally emits `validation_v1.py` and `validation_guard.py`. The original
v1 evaluator and numeric helper remain byte-identical. The v2 native guard checks
envelopes, operands, source/target identities, patterns, adjacency and execution
order. Sessions own checked metadata independently of caller mutation.

The maintained `python_applicators` target compiles the original 32 source vectors
through `compile_v2`. Both Python tiers pass those vectors, 13 additional
scope/work/Unicode/recursion cases, 20 malformed-program controls and normal-stack
depth checks. Frozen completion: `target/sdk-python-scoped-completion-01/`.

## Indexed resource/dynamic v3 execution

V3 implements [the owned resource contract](SDK-SCHEMA-RESOURCES.md). It adds
`validation_v2.py` and `validation_resource_guard.py`, retaining the v1/v2
templates unchanged. The v3 entry point checks the resource registry, aliases,
physical containment, canonical addresses, aligned node scopes and DynamicRef
operands before execution.

Each node enters its indexed resource, including nested entry points. Dynamic
lookup scans only actually entered resources, outermost first; it never enters
the initial fallback before lookup. Static refs and empty/pointer/static-anchor
fallbacks retain their indexed targets. Every return/trial restores scope. The
cycle key includes the exact ordered resource tuple, node and instance identity.
Resource entry and lookup add exactly the published visit charges. Selected
targets use fresh v2 annotation scopes. Runtime validation loads no source URI.

The maintained `python_resources` target compiles the unmodified 44 official
dynamicRef cases and supplied remote documents through a closed provider and the
real `compile_v3` API. Both native interpreters pass all 44 cases, 17 independent
scope/context-cycle/work/depth cases, 27 malformed-program controls and strict
runtime typing. See `target/sdk-python-resources-runtime-03.log` and the complete
[native resource adoption record](SDK-PYTHON-RESOURCES.md).

```sh
cargo test --locked -p suspect-codegen --no-default-features --features http-protocol \
  --test python_resources \
  python_v3_executes_official_sources_dynamic_scopes_guards_and_exact_budgets \
  -- --exact --ignored
```
