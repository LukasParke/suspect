# Go portable validation

`go_validation::emit(&OwnedProgram)` checks portable program metadata and emits
`validation.go`, `validation_number.go`, `validation_pattern.go` and
`validation_program.json`. The program carries original schema source identities;
the executor interprets typed instructions and retains exact JSON literal bytes.

`Validate(root_node, value)` starts a fresh validation session. Source codecs share
one session across branch trials, retaining evaluation/equality/numeric budgets.
Typed `ValidationError` distinguishes `invalid` from `evaluation_failure` and
includes the original source and instance path.

Exact decimal arithmetic uses arbitrary-precision coefficients/exponents with
finite work checks. Large exponents are compared symbolically, preserving
integrality, range and divisibility without floating-point saturation. Portable
pattern execution uses the compiler's checked NFA and strict-anchor semantics.

The profile caps depth at 512, numeric operands at 65,536 bytes, and work counters
to the portable native integer range. JSON parsing and model conversion have their
own configurable budgets.

`tests/go_validation.rs` runs the same 17 independent schema cases in
`tests/fixtures/runtime-contract-v1.json` as Python, TypeScript and Rust. Invalid
vectors must produce invalid outcomes; an evaluation/resource failure cannot pass
as expected invalid input.

```sh
SUSPECT_GO_TOOLCHAIN=go1.23.12 \
  cargo test --locked -p suspect-codegen --test go_validation -- --include-ignored
```

Complete schema-language support remains bounded by checked owned-program
compilation. This executor is separate from native model representation admission.
