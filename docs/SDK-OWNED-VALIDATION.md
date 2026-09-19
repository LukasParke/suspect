# Owned validation boundary

The owned validator compiles selected schema IDs from an `Arc<Contract>` into
an immutable program. It retains the source contract once, indexes static
references as finite edges, and allocates evaluation state separately for each
call. It does not retain `LowDoc` lifetimes, expand recursive schemas into
trees, or serialize and reparse schema fragments.

`OwnedCompiler::compile(contract, roots)` returns either a complete supported
program or located compilation errors. Unknown requested roots are errors.
Unsupported features elsewhere in the contract do not disable a selected
supported closure. `OwnedSchema::validate(root, &Value)` accepts only roots
selected at compilation and returns one of three explicit outcomes:

- `Valid`: every assertion in the supported program passed.
- `Invalid(findings)`: evaluation completed and assertions rejected the input.
- `EvaluationFailure(finding)`: validity could not be determined. Numeric or
  equality resource limits, excessive recursion, a recursive cycle without
  instance progress, and an unselected root ID produce this outcome.

Composition cannot suppress or invert evaluation failures. Reporting limits
cap mismatch findings without turning incomplete evaluation into validity.
Every finding preserves the original document URI and schema keyword pointer,
including failures reached through references to external documents.

`Config::max_evaluation_steps` defaults to 100,000 and counts schema entries,
keyword checks, and collection/branch visits across the entire call. It bounds
repeated evaluation of a finite reference graph independently of recursion
depth and equality comparisons. It is a visit budget, not a bound on byte
work; string lengths and numeric representations still matter. The existing
numeric-byte and equality-work limits remain separate.

## Initial supported subset

The initial dialects are the standard OAS 3.1 base dialect and JSON Schema
2020-12. This is a declared subset, not a claim of full dialect conformance.

| Area | Supported behavior |
| --- | --- |
| Shape | Boolean schemas, all JSON types, mathematical integers, type unions, explicit null |
| Numbers | Exact inclusive/exclusive bounds and `multipleOf`; symbolic exponents and existing numeric budgets |
| Equality | `enum`, `const`, and `uniqueItems`, using the same exact structural equality and work budgets as the source-bound validator |
| Sizes | Nonnegative mathematical cardinalities for strings, arrays, and objects, including bounds beyond machine integers |
| Patterns | Bounded Thompson NFAs for a regular ECMA-262 Unicode subset, with normalized scalar ranges, strict anchors, ASCII `\w`/`\d`, and shared evaluation work |
| Objects | `properties`, `additionalProperties`, and `required`; applicability never implies object type |
| Arrays | `items` and `prefixItems`; applicability never implies array type |
| Composition | `allOf`, `anyOf`, `oneOf`, and `not` |
| References | Strict URI-reference syntax followed by Contract-indexed static local, external, recursive and anchored `$ref` edges, including sibling assertions |
| Annotations | Retained source prose, examples, defaults, content annotations and OAS metadata; annotation-mode formats; unknown modern annotations |

OAS 3.0 and custom dialects are explicitly unsupported. Dynamic references,
embedded `$id` resources, custom vocabularies, pattern properties, lookarounds,
backreferences, Unicode property escapes, conditionals,
dependency applicators, `contains`, and unevaluated applicators reject
compilation in this initial program. Format assertion mode also rejects a
closure that uses `format`. These are explicit capability limits rather than
approximations or successful no-ops.

Existing `ExactNumber`, pre-factored divisors and count bounds supply numeric
semantics. One structural-equality algorithm adapts both source nodes and
owned JSON values; neither adapter converts numbers through floating point.

The source-bound `Compiler` uses this same portable pattern program for
`pattern`, `patternProperties`, and sibling `additionalProperties` exclusions.
Pattern transitions and property-name matching consume the call's shared
`max_evaluation_steps` allowance. Valid ECMA-262 constructs outside the
declared regular subset, such as lookarounds, backreferences, word boundaries,
and Unicode property escapes, fail compilation explicitly; they are never
silently delegated to Rust regex semantics.

Pattern admission caps source size at 4,096 bytes, nesting at 64, repeat
endpoints at 1,024, states at 8,192, expansion visits at 32,768 and aggregate
character ranges at 65,536. Both source compilation and portable-program
admission enforce bounds before expensive expansion or cloning. Matching
streams Unicode scalars with reusable O(states) scratch. Input boundaries,
queue attempts, state processing and range comparisons all consume the shared
evaluation allowance; backtracking is not used.

The generated TypeScript executor uses the same program and work accounting.
All four actual OpenRouter chat pattern schemas have source-bound and owned
coverage. Native tests compare admitted expressions with Node's Unicode RegExp;
an independent review compared 411 admitted patterns against 19 strings without
an accepted semantic mismatch. The complete official pattern file is not a
passing conformance claim: it includes unsupported Unicode property escapes.

One representation boundary remains explicit: JavaScript strings and the
generated JSON parser can retain lone UTF-16 surrogates, while this pattern
profile operates on Unicode scalars. A pattern applied to such a string returns
a source-located `evaluationFailure`; it does not report ordinary invalidity or
throw outside validation. Full cross-language surrogate representation remains
unresolved.

## Acceptance evidence

The public tests exercise source-lifetime independence, concurrent validation,
strict root IDs, external recursive identity, exact arithmetic, scoped
unsupported features, and evaluation failures inside logical branches. A
dedicated OpenRouter test uses the tracked `ChatChoice.index` and
`ChatRequest.messages` schemas with independently specified valid and invalid
instances. Unrelated upstream defects must not be hidden or rewritten to
compile these selected closures.

The owned test module also runs the 176 pinned official `enum`, `const`, and
`uniqueItems` cases through Contract and OwnedCompiler. Compilation failures
and incomplete evaluation always fail those tests, including cases expected
to be invalid. See `crates/suspect-schema/tests/conformance/README.md` for the
official suite revision and license.

The five native SDK profiles compile their selected codec closures through
this owned boundary before emission. Model-only APIs retain explicit codec
obligations. Native runtime details are documented for
[TypeScript](SDK-TYPESCRIPT-CODECS.md), [Rust](SDK-RUST-MODEL-PLAN.md#validated-model-codecs),
[Python](SDK-PYTHON-VALIDATION.md), [Go](SDK-GO-VALIDATION.md) and
[Swift](SDK-SWIFT.md#portable-validation-and-resource-policy).

## Experimental portable program

`OwnedSchema::program()` snapshots the already compiled instructions into a
typed, owned `OwnedProgram` with `serde::Serialize`. Its wire discriminator is
`suspect.validation.experimental.v1`, with profile
`oas31-jsonschema202012-static-subset`. This is an experimental runtime input,
not a stable serialization ABI or a second general JSON Schema format.

The snapshot does not discover keywords or reinterpret raw schema semantics.
It copies the compiled finite node order and selected root indices. Each
`ProgramNode` has its original document URI, escaped JSON Pointer, and ordered
`ProgramCheck` list. Each check has its own source identity and a flattened
camelCase `op` tag. References use node indices, including recursive and
external references. Annotation prose and examples remain available from the
retained Contract for documentation generation; they are not executable checks.

| Instruction | Portable operands |
| --- | --- |
| `always` | Boolean `value` |
| `type` | `types` array of standard JSON type names in canonical order |
| `ref` / `not` | Child node `target` |
| `properties` | `properties` array of decoded `{name, target}` entries |
| `additionalProperties` | Sorted decoded `declared` names and child `target` |
| `required` | Decoded `names` in declaration order |
| `items` | Child `target` and zero-based `start`, accounting for `prefixItems` |
| `prefixItems` / `allOf` / `anyOf` / `oneOf` | Child `targets` in declaration order |
| `bound` | Exact numeric token string `value`, `maximum`, and `exclusive` |
| `multipleOf` | Validated positive numeric token string `value` |
| `count` | Exact numeric token string `value`, `maximum`, and `target` kind (`string`, `array`, or `object`) |
| `enum` / `const` | Retained JSON literal `values` / `value`, with arbitrary-precision numbers |
| `uniqueItems` | No operands |
| `pattern` | Versioned Thompson NFA with finite `match`, `char`, `split`, `jump`, `start`, and `end` states |

Numeric instructions retain the validated Contract numeric token; this can
already include a normalized exponent spelling from JSON/YAML loading. The
exporter does not convert numbers through floating point. Cardinality keeps
that token alongside the executor's optimized machine-size bound, so an
astronomical bound is never reduced to a platform-specific sentinel in the
portable output. Enum and const copy their compiled literal operand from the
same retained source address used by the owned executor. Their JSON numbers
must also be loaded losslessly by a consuming runtime; ordinary JavaScript
`JSON.parse` would lose information for some valid operands.

`limits` carries `maxDepth`, `maxErrors`, `maxNumberBytes`, `maxEqualitySteps`,
and `maxEvaluationSteps`. Their zero values have the same meaning as the owned
runtime: only `maxErrors: 0` means unlimited; the other zero values allow no
corresponding work. Work budgets survive logical trials. Every schema's checks
are conjunctive; properties and cardinalities never imply a type, and defaults
never supply a missing value. Unknown versions/profiles/instructions and
incomplete evaluation must produce explicit failures in consumers.

Because portable snapshot fields are public, emitters call
`OwnedProgram::check()` before accepting a potentially modified snapshot.
This common host guard checks the version/profile, absolute fragment-free
document URIs, escaped keyword/source identities, unique nodes and roots,
finite graph targets, type/name uniqueness, applicator child locations, and
adjacent `items`/`prefixItems` and `additionalProperties`/`properties`
consistency. Exact numeric token syntax and the configured operand limit are
admitted before arithmetic; the existing `ExactNumber` implementation decides
positivity, zero and mathematical integrality. For example, `-0e9` is valid
nonnegative cardinality, while `1e-400` is fractional and a zero divisor is
invalid. Exponents remain symbolic. No separate language-emitter arithmetic
interpretation is needed for this host check.

`ProgramCheckError` identifies the responsible source and the failed invariant;
global version/profile errors have no source. Language-specific representation
limits, such as JavaScript safe integers for graph metadata, remain the
emitter's responsibility. This guard does not attest that a modified bound
still matches its original OpenAPI document, and deliberately does not
pre-apply numeric/equality budgets to unvisited enum or const literals.

Public compile-to-program tests cover deterministic recursive indices, every
instruction family, decoded external identity after all source lifetimes end,
all limit fields, exact operands beyond binary64/machine integers, and failure
to export unsupported, invalid, or unknown roots. A second tracked OpenRouter
test inspects the actual compiled `ChatChoice.index`, `ChatRequest.messages`,
and `ChatMessages.oneOf` closure. Export itself does not prove a generated
language runtime implements these instructions; that requires consumer tests.

## Native Rust consumer

`suspect_codegen::rust_validation::emit(&OwnedProgram)` emits a dependency-free
Rust runtime after the common program check plus explicit native resource
admission. Generated `validation::validate(root_node_index, &JsonValue)` returns
`Valid`, `Invalid(findings)` or `EvaluationFailure(finding)`. Root indices, exact
numeric values, portable patterns, source addresses and instance pointers remain
explicit. Unknown roots and exhausted work never pass through logical negation
or an alternative branch.

Native admission rejects depth policies above 512 and counters beyond u32
capacity rather than clamping them. Coefficient division has a separate shared
numeric-work allowance equal to `max_evaluation_steps`. Codec conversion uses
private `validate_at`/`equal_at` sessions so branch/literal trials share counters
and carry their own source/instance locations, not the last evaluated root.

Native stress verification found and repaired recursive debug-frame growth.
Scalar formatting/arithmetic/pattern work and individual recursive opcode work
now live outside the shared dispatch frame. All nine recursive instruction
families complete at the 512 ceiling and fail explicitly at 513 on default Rust
threads, with `RUST_MIN_STACK` removed. This is not universal CPU/heap accounting
or a guarantee for caller-selected undersized thread stacks.

`tests/rust_validation.rs` contains checked-program admission, differential
numeric/applicator/pattern outcomes, logical resource failure, source/path and
tracked OpenRouter closure consumers. Combined native Rust verification is
recorded in `target/sdk-glm-rust-native-tests.log`. Rust string representation
retains the existing lone-surrogate boundary described in [Rust JSON](SDK-RUST-JSON.md).
