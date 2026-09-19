# Swift scoped validation v2

Swift implements the additive evaluated-applicator contract in
[SDK-SCHEMA-APPLICATORS.md](SDK-SCHEMA-APPLICATORS.md). The public SDK path uses
`OwnedCompiler::compile_v2` over the Contract's effective schema closure, checks
the resulting program, lowers its typed instructions, and caches the admitted
Swift table before returning an SDK plan. Unsupported/malformed programs fail
with original source locations before artifacts are available.

## Versions and admission

| Program | Native profile |
| --- | --- |
| `suspect.validation.experimental.v1` | `oas31-jsonschema202012-static-subset` |
| `suspect.validation.experimental.v2` | `oas31-jsonschema202012-static-applicators` |

Base closures retain their exact v1 program. Annotation collection is disabled
for v1, including its additional-property/item and logical-trial semantics.
`plan_protocol_examples_v2` is selected only for a v2 program; v1 keeps
`plan_protocol_examples`. Both share native direct-constructor lowering.

The initial integration checkpoint used a keyword-linked v2 admission refusal
while the Swift evaluator was under construction. That refusal was removed after
the runtime vectors passed on current and floor. Emission is now fallible and
admitted once during planning, rather than assuming every future instruction is
supported. At this checkpoint the concurrently introduced v3/resource/dynamic
profile was explicitly refused. The subsequent verified adoption is documented
in [SDK-SWIFT-RESOURCES.md](SDK-SWIFT-RESOURCES.md); `DynamicRef` is never
downgraded to a static reference.

Native limits retain the previous policy: depth at most 512, numeric operands at
most 65,536 source bytes, and counters no larger than Int32.max. Program checking
also verifies exact version/profile pairs, source/target identities, pattern
programs, operands, and the required unevaluated-check ordering.

## Nine new instructions

| Instruction | Swift execution |
| --- | --- |
| If | Trial the condition exactly once. Execute only the selected then/else branch. A passing condition contributes locally, independently of a failing selected branch. |
| DependentRequired | On objects, presence—including null or false—activates required-name checks. It does not annotate values. |
| DependentSchemas | Evaluate each active target against the whole object in a fresh scope. Publish child annotations only when all activated schemas pass. |
| Contains | Trial every element, retain matching indices, and compare exact cardinality tokens. Later failures survive earlier matches or an exceeded maximum. |
| PatternProperties | Apply every matching NFA/schema pair. Overlapping patterns are conjunctive, including overlap with a declared property. |
| AdditionalPropertiesWithPatterns | Exclude declared names and any same-node pattern match, independently of that pattern's value-schema result. |
| PropertyNames | Evaluate decoded names as string instances; retain the responsible schema source and the member instance path. Never annotate member values. |
| UnevaluatedProperties | Check locally unmarked property values after all other checks. |
| UnevaluatedItems | Check locally unmarked array indices after all other checks. |

The emitter consumes `ProgramInstruction` operands and actual target indices;
the runtime does not interpret source schemas or reconstruct them from generated
text. Pattern-property NFAs remain the portable checked pattern programs.

### Annotation scopes and failure propagation

Each schema evaluation owns property/index sets at its **own instance level**.
Child containers cannot mark their parents. A failed schema returns empty sets.
Ref and same-instance applicators transfer only successful results. AnyOf merges
every passing branch; oneOf publishes only exactly one passing branch; not
discards its trial annotations. Siblings and cousins never inherit each other's
sets. Required, dependentRequired and propertyNames produce no evaluated-value
annotations.

If's successful condition contributes to its parent scope before the branch, but
does not seed the branch. Contains' matching indices contribute independently of
the adjacent minContains/maxContains result. These distinctions avoid converting
an ordinary invalid result into an unrelated later numeric/resource failure.

Trials restore mismatch reporting only. Depth, active identities, evaluation,
equality and numeric work remain shared. EvaluationFailure propagates through all
branches and negation; ordinary mismatches do not stop remaining conjunctive work.
The public throwing API retains its first-mismatch behavior.

Property identities use `JsonKey`/UTF-8 bytes. Canonically equivalent Swift strings
remain different JSON names. Count tokens retain symbolic decimal exponents;
contains equality/integrality never narrows through Double or NSNumber. Pattern
matching streams Unicode scalars instead of allocating a complete scalar array
before observing the work limit.

### Normal-stack resource failure

The depth stress reproduced a Swift debug-build stack overflow before the allowed
512-level guard. Splitting opcode bodies alone still left the large enum-dispatch
frame on the recursive stack. Rule selection now **returns before** recursive
execution; rule-specific frames retain only their own operands. A 1,024-level
instance on a 2 MiB native thread now reports EvaluationFailure at the configured
depth on both toolchains. No stack-size requirement was raised and no evaluation
limit/compiler check was disabled. Red and green artifacts are retained.

## Faithful native models and codecs

Stable object properties, scalar types, nullable/presence fields and explicit
union payloads retain their existing native types. Pattern-dependent extra keys
use `JsonObject<JsonValue>`; the owning source codec validates every overlapping
pattern, name rule, additional-property rule and unevaluated assertion.

When v2 layouts cannot have one unconditional native declaration, the planner
emits a named **checked carrier**:

```swift
let object = JsonValue.object(try JsonObject([
    ("tag", JsonValue.string("a")),
    ("payload", JsonValue.number(2))
]))
var value = ConditionalCarrier(value: object)
let bytes = try ConditionalCarrier.codec.encode(value)
// Wire: {"payload":2,"tag":"a"}; there is no synthetic "value" member.

value.value = .array([.string("else branch")])
_ = try ConditionalCarrier.codec.encode(value) // Revalidates current state.
```

Checked reference/intersection/conditional carriers retain the complete exact
JSON instance. V2 heterogeneous prefix/item arrays can use a checked `[JsonValue]`
carrier. These are typed source-bound model declarations, not an unchecked Any
fallback. Their mutable `value` is converted without field loss, then validated
through the original schema on every encode and decode.

Nullable source domains remain explicit. The full source codec can consume
Nullable; the non-null carrier's codec rejects a null hidden inside its value
case. Optional wrappers still distinguish missing from present null. Native
Codable integration continues to require SDKJSONEncoder/SDKJSONDecoder.

Compatibility model capture records the checked carrier's value type, mutable
member, constructor, full-value wire representation and source-codec contract.
No emitted-source parsing is used.

### Remaining model/profile boundaries

- V3 canonical-resource/dynamic-reference execution uses the separately verified
  [native resource profile](SDK-SWIFT-RESOURCES.md).
- Active directional annotations retain their existing source-linked policy.
- Multiple non-null primitive `type` alternatives, untyped object/array literal
  domains, and directly constructible false-schema model roots retain explicit
  model-layout refusals. False schemas within checked applicators remain valid
  executable assertions.
- This tranche adds evaluated-location sets, not a public general annotation
  reporting API or full JSON Schema dialect conformance.

## Native evidence

The runtime oracle combines the shared 32 literal normative/adversarial cases
with 21 Swift-specific literal cases. Expected results are declared in fixtures,
not calculated by the Rust evaluator. Tests cover exact source pointers,
condition/branch isolation, ref propagation, overlaps, decoded key identities,
contains precision/counts, unmasked resource failure and active recursion.

| Gate | Result on current and floor |
| --- | --- |
| Scoped evaluator | 53 literal cases + normal-stack depth stress: **54 passed** |
| Installed public SDK | **10 codec/model/HTTP-boundary cases passed** |
| Native typing | Successful control plus **7 required failures** |
| Packaging and documentation | SwiftPM generated-package and independent-consumer tests; DocC with warnings as errors |
| V1 validation regression | Existing shared corpus and budget cases passed; original fixture unchanged |

Host checks verify all nine opcodes, original source identity, checked-carrier
descriptors, exact v1 program parity, and source-linked malformed/resource-profile
refusals: **3 integration checks + 1 checked-emission unit check passed**.
The four Swift compatibility checks and the aggregate default-feature
`cargo check -p suspect-codegen` also pass.

The public SDK fixture covers mutable condition/dependency checks, patterns with
closed extras, exact Unicode keys, checked JSON/array carriers, nullable carriers,
anyOf value preservation, Codable adapters, request validation before transport,
and response validation after transport. Guides use native constructors and the
new v2-aware shared example plan.

### Exact test selectors

```sh
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --lib \
  swift_sdk::validation::v2_tests::native_evaluated_applicator_vectors \
  -- --ignored --nocapture

cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test swift_validation_v2 \
  native_installed_v2_codecs_types_and_docs -- --ignored --nocapture

cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --lib \
  swift_sdk::validation::tests::native_shared_runtime_contract_vectors \
  -- --ignored --nocapture

cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test swift_validation_v2
```

Use the `SUSPECT_SWIFT_BIN`, `SUSPECT_SWIFTC_BIN`, `SUSPECT_SWIFT_DOCC_BIN` and
`SUSPECT_SWIFT_SDKROOT` selectors described in [SDK-SWIFT.md](SDK-SWIFT.md).
Current is Swift 6.3.3 + SDK 26.5. Floor is the retained/re-extracted official
Swift 6.0.3 toolchain + `/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk`.
`SUSPECT_SWIFT_V2_ROOT` selects a fresh artifact parent. Each gate retains its
source inputs, checked-program snapshots, generated sources, builds and command
logs. Earlier completed protocol/M2/OpenRouter reports are not rerun by this gate.

Retained roots under `${TMPDIR%/}/opencode`:

- `swift-v2-gates/runtime-v2-ixyCtQ` — current 54-case runtime pass;
- `swift-v2-floor/runtime-v2-xMrYkB` — floor 54-case runtime pass;
- `swift-validation-ObFjJb` / `swift-validation-auOOXq` — current/floor v1 runtime regressions;
- `swift-v2-gates/sdk-v2-J4RS6V` — initial current installed SDK/type/DocC pass;
- `swift-v2-current-final/sdk-v2-VuFiA8` — final current installed SDK/type/DocC pass with the v2-aware shared example helper;
- `swift-v2-floor/sdk-v2-Mq8IXB` — floor installed SDK/type/DocC pass with v2 examples;
- `swift-v2-gates/runtime-v2-62EVHr` and `runtime-v2-gT9nc8` — retained depth-stress red attempts.

## Production asset handoff

This tranche modifies these production sources (the protocol runtime files and
shared core/IR/registry are outside the change):

```text
swift_sdk.rs
swift_sdk/validation.rs
swift_sdk/validation.swift
swift_sdk/models.rs
swift_sdk/emit.rs
swift_sdk/protocol_examples.rs
compatibility/swift_models.rs
```

New test-only sources are `tests/swift_validation_v2.rs`,
`swift_sdk/validation_v2.rs`, `swift_sdk/validation_v2_support.rs`, and
`swift_sdk/validation_v2_native.swift`. They are not production runtime assets.
