# Exact schema numbers and equality

Implementation and local evidence, 2026-09-08. This is a bounded repair of
`suspect-schema` through `Compiler::compile` and `Schema::validate`, not a claim
of complete JSON Schema or SDK conformance.

## Behavior

Numeric bounds and `multipleOf` now operate on exact decimal values. Neither
compilation nor evaluation converts them to `i64` or `f64`. Adjacent wide
integers remain distinct, decimal divisibility requires an exact integer
quotient, and schema bounds larger than machine integers compile successfully.
`integer` uses LowDoc's mathematical integrality check, including exponent
forms that overflow or underflow binary floating point.

`enum`, `const`, and the newly executed `uniqueItems` share structural equality:
exact numbers, decoded Unicode strings and object keys, ordered arrays, and
unordered objects. For example, `1`, `1.0`, and `10e-1` compare equal;
`9007199254740992` and `9007199254740993` do not. An empty `enum` compiles and
accepts no values; the specification recommends a nonempty enum but does not
require it. Invalid quoted strings and ambiguous duplicate decoded object keys
produce an evaluation failure. Equality follows the existing LowDoc alias and
merge expansion; this change does not redesign YAML merge semantics.

YAML signed decimal, hexadecimal and octal integer spellings retain their
mathematical values, as do `.125`, `1.`, and exponent forms. Nonfinite YAML
values cannot act as schema numeric bounds or divisors, and cause explicit
evaluation failures when a type or numeric/equality check needs their numeric
value. This is not a general JSON-domain preflight on every input to a `true`
schema.

## Representation and limits

[number.rs](../crates/suspect-schema/src/number.rs) owns a sign, normalized
decimal coefficient, arbitrary integer exponent, and source spelling. The
coefficient has no leading or trailing zeroes; zero has one canonical value.
Comparison aligns decimal orders and compares only written coefficient digits.
Divisibility removes the divisor coefficient's powers of two and five, then
uses exact integer remainder. No operation expands a symbolic exponent into
that many decimal digits. Divisors are factored once during compilation.

`num-bigint` is a compiler/validator dependency. It does **not** become a
dependency of generated SDK packages. The numeric value is owned and reusable
by a future owned validation program; the surrounding schema program still
borrows LowDoc nodes and uses `Rc`/`RefCell`.

The [configuration](../crates/suspect-schema/src/config.rs) makes limits explicit:

| Limit | Default | Behavior when exceeded |
| --- | ---: | --- |
| `max_number_bytes` | 4,096 source bytes per exact operand | `CompileError::ResourceLimit` for an eagerly compiled numeric keyword; evaluation failure for instance/equality operands or lazy compilation |
| `max_equality_steps` | 100,000 compared node pairs per validation | Evaluation failure; trial schema branches share the allowance |
| `max_depth` | 512 | Equality's explicit work stack checks this depth without recursive equality calls |

Zero numeric/equality allowances disable those operations; they do not mean
unlimited. `uniqueItems` currently compares pairs, so large arrays of distinct
items can exhaust the equality allowance. That result means validity was not
determined. It is not an assertion that the array contains a duplicate.
Canonical hashing is a potential optimization after preserving these semantics.

`SchemaErrorKind::Evaluation` distinguishes these failures from ordinary
`Invalid` assertions. Evaluation failures survive `not`, `if`, `anyOf`,
`oneOf`, error caps, and cached lazy references. An evaluation failure takes
precedence in `validate_first` and in a capped error list. Consumers should
report inability to evaluate, not a proven schema mismatch. This slice does
not migrate every older engine diagnostic to this distinction.

## Verification

The primary acceptance test loads the actual pinned OpenRouter public schema
and compiles `components.schemas.ChatChoice.properties.index`. It rejects
`1e-400` and accepts `0` and `1e400`. The exact snapshot SHA-256 is
`bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821`, from tracked
`projects/docs/openapi/openapi.yaml` at OpenRouter commit
`db378a2a90d0167b9dca4f98b52074c54d249e1f`.

The [public behavior tests](../crates/suspect-schema/tests/numeric.rs) also cover
wide bounds, exact decimal multiples, nested equality, escaped text, YAML
values, thousand-digit exponents, and limits inside logical/lazy reference
branches. The OpenRouter test is opt-in because the upstream checkout is not
vendored; it has been explicitly executed. A missing snapshot is an error,
not a successful skip.

The 294 cases in nine unmodified official Draft 2020-12 files for bounds,
divisibility, type, enum, const and uniqueItems execute through the same public API. Their
[provenance and license](../crates/suspect-schema/tests/conformance/README.md)
are retained. An independent review also ran 20,000 deterministic coefficient
and exponent cases against `i128` rational cross-products, remainder and
checked addressable-integer conversion, with no discrepancies. That temporary
harness is `/private/tmp/suspect-number-independent-20260908/check.rs`.

```sh
cargo test -p suspect-schema --locked
cargo clippy -p suspect-schema --all-targets --locked -- -D warnings
OPENROUTER_PUBLIC_SCHEMA=/absolute/path/to/pinned/input.yaml \
  cargo test -p suspect-schema --test numeric --locked -- --ignored --nocapture
cargo bench -p suspect-schema --bench numeric --locked
```

## Local runtime evidence

Release microbenchmarks use the public compiler/validator interface, with
LowDoc parsing outside the measurement. Compilation and validation have
separate measurements; all benchmark instances first pass a correctness check.

| Case | Compile | Validate |
| --- | ---: | ---: |
| Ordinary decimal with type, bounds and `multipleOf` | 6.13 µs | 2.19 µs |
| Wide integer with type, bounds and `multipleOf` | 6.18 µs | 1.86 µs |
| Exponent 400 | 7.95 µs | 2.47 µs |
| Exponent written with 1,001 digits | 11.37 µs | 12.13 µs |

These are local Criterion estimates from an active development machine, not
an SDK generation throughput target or a controlled before/after comparison.
They show that exponent magnitude does not trigger expansion. The benchmark
source is [benches/numeric.rs](../crates/suspect-schema/benches/numeric.rs);
temporary estimates and environment metadata are recorded in
`target/schema-numeric-evidence/benchmark.json`.

Remaining integration work includes compiling from the owned contract registry,
full dialect/reference context, complete branch annotation isolation, and
language-specific codecs and native documentation. Fixing these numeric cases
does not promote any generated SDK target to complete support.
