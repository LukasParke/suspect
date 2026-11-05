# Owned applicators: additive v2 executable contract

Status: the bounded nine-op implementation and Rust verification are complete,
2026-09-10. Native owners may implement the stable descriptors below. Twelve
focused tests, 32 independent execution vectors, 395 in-scope official cases,
normal-stack depth failure and warnings-denied Clippy pass. Six official cases
have explicit capability deferrals listed below. This is not full conformance.

## Migration and frozen v1

`OwnedCompiler::compile` keeps its existing v1 admission and behavior.
`OwnedCompiler::compile_v2` additionally admits the modern applicators. It emits
v2 only when the selected effective closure needs a new operation; base closures
retain their v1 program representation. Contract owns effective traversal,
source/dialect context and reference targets. No schema identity is synthesized.

| Version | Profile |
| --- | --- |
| `suspect.validation.experimental.v1` | `oas31-jsonschema202012-static-subset` |
| `suspect.validation.experimental.v2` | `oas31-jsonschema202012-static-applicators` |

The `OwnedProgram` envelope, root/node/check/source structure, exact numeric
tokens and existing limit fields stay the same. Existing instruction operands
are unchanged. `OwnedProgram::check()` recognizes the exact version/profile
pairs and rejects every new operation in a v1 envelope. Associated constants
`OwnedProgram::{V1_VERSION,V1_PROFILE,V2_VERSION,V2_PROFILE}` expose these pairs.
`ProgramInstruction::requires_v2()` identifies the new operations.

## New public typed instructions

The Rust enum is still `ProgramInstruction`. JSON `op` and operand field names
are camelCase. Optional targets/counts are serialized as JSON null when absent.
All schema targets are indices into the same finite `nodes` array.

| Rust variant / JSON op | Operands | Original check source |
| --- | --- | --- |
| `If` / `if` | `condition: usize`, `thenTarget: Option<usize>`, `elseTarget: Option<usize>` | `/if` |
| `DependentRequired` / `dependentRequired` | `dependencies: Vec<(String, Vec<String>)>`; JSON ordered `[trigger, names]` pairs | `/dependentRequired` |
| `DependentSchemas` / `dependentSchemas` | `dependencies: Vec<ProgramProperty>`; ordered `{name,target}` entries | `/dependentSchemas` |
| `Contains` / `contains` | `target: usize`, `minimum: Option<String>`, `maximum: Option<String>`; exact minContains/maxContains tokens | `/contains` |
| `PatternProperties` / `patternProperties` | `patterns: Vec<(String, PatternProgram, usize)>`; ordered `[patternText, NFA, target]` triples | `/patternProperties` |
| `AdditionalPropertiesWithPatterns` / `additionalPropertiesWithPatterns` | `declared: Vec<String>`, `target: usize`; patterns come from the same node's PatternProperties instruction | `/additionalProperties` |
| `PropertyNames` / `propertyNames` | `target: usize` | `/propertyNames` |
| `UnevaluatedProperties` / `unevaluatedProperties` | `target: usize` | `/unevaluatedProperties` |
| `UnevaluatedItems` / `unevaluatedItems` | `target: usize` | `/unevaluatedItems` |

The ordinary AdditionalProperties opcode keeps its v1 semantics. A node with
nonempty PatternProperties and additionalProperties must use the new
AdditionalPropertiesWithPatterns opcode. The guard verifies this relationship,
the adjacent declared names, pattern programs, and actual child target locations.
Pattern programs are stored once on PatternProperties, not rediscovered from
raw schemas at execution. Pattern syntax and NFA limits remain the existing
portable regular ECMA-262 Unicode subset.

## Executable rules

All existing type/number/count/equality semantics remain unchanged. Applicability
never implies an instance type. All evaluations have distinct Valid, Invalid,
and noninvertible EvaluationFailure outcomes. No default insertion, field
stripping, coercion or algebraic conditional rewriting occurs.

- **If:** evaluate the condition once as a trial, discarding mismatch findings.
  EvaluationFailure stops the call. Evaluate only the selected then/else target,
  if present. Neither branch sees the other's or the condition's local scope.
  A successful condition contributes its evaluated-location annotations; a
  selected successful branch contributes its own. In a v2 closure, a standalone
  if also executes because its annotations may matter. Orphan then/else do not.
- **DependentRequired:** on objects, visit each ordered trigger; if present
  (including a null-valued member), require every listed name. It does not
  evaluate property values or produce evaluated-property annotations.
- **DependentSchemas:** on objects, evaluate each present trigger's target
  against the whole object in a fresh scope. Never apply it to the trigger value.
- **Contains:** on arrays, trial the target against every element, retaining all
  successful indices. Failures are not suppressed by an earlier match or an
  exceeded maximum. The implicit minimum is 1, the absent maximum is unbounded.
  Explicit bounds are nonnegative mathematical integers under maxNumberBytes.
  Default 1 is semantic, not an invented source numeric operand. Non-arrays pass.
  The contains annotation exists on success of contains itself (at least one
  match, or explicit minimum zero); min/max count failures still reject the
  containing schema. Explicit count findings use sibling minContains/maxContains
  locations; an implicit-minimum finding uses contains.
- **PatternProperties:** on objects, test every pattern for every member and
  apply every matching target to that member value. Overlaps are conjunctive.
- **AdditionalPropertiesWithPatterns:** exclude declared names and any name
  matching any same-node pattern, regardless of whether that pattern's value
  schema validated. Apply the target to the remaining member values.
- **PropertyNames:** evaluate each decoded member name as a string, using the
  complete compiled validator. Findings retain the child assertion source and
  the associated member's instance pointer; no key-token pointer syntax is
  invented. This does not evaluate or annotate that member's value.
- **Unevaluated:** run after all other checks in that schema object. Apply the
  target only to unmarked members/indices and contribute successful evaluations
  to the result. A subschema always starts with a fresh local scope; it cannot
  consume a caller's, sibling's or cousin's sets.

### Scoped evaluated locations

An evaluation returns a Boolean result and sets of evaluated property names and
array indices **at its own instance level**. Failed schemas export empty sets.
Properties/items/pattern/additional/unevaluated applicators mark their immediate
members, not their descendants. Required, dependentRequired and propertyNames
never mark member values. Ref and same-instance applicators propagate successful
sets; anyOf unions all passing branches, oneOf keeps sets only with exactly one
passing branch, not always discards its child's sets. Branches never share mutable
annotation state. These are evaluated-location sets, not a generic annotation
output API. In particular, unevaluated behavior is not property-list flattening.

The exact executable scope algorithm is:

```text
Eval(node, instance):
    spend(node.source); check depth and active (node, instance identity)
    local = empty property/index sets; valid = true
    for check in node.checks:
        spend(check.source)
        (ok, produced) = Apply(check, instance, local)
        valid &= ok                         # do not stop on an ordinary mismatch
        if ok: Merge(local, produced, check.source)
    leave active identity/depth
    return (valid, local if valid else empty)

Merge(destination, source, location):
    for each property name in source, then each index in source:
        spend(location)                     # even if already in destination
        insert into destination
```

`Apply` never passes `local` into a child `Eval`. It is available only to the
current node's unevaluated checks and to these explicit contribution rules:

| Applicator | Produced-set and local-set rule |
| --- | --- |
| Ref | Move the child's returned sets into produced; the common final Merge publishes them only if the child passed. |
| AllOf | Evaluate every child. If any failed, discard all child sets. Otherwise Merge each child's sets into a temporary produced union, then the common Merge publishes that union. |
| AnyOf | Trial every child. If none passed, fail with no sets. Otherwise Merge every passing child's sets into produced, then publish the union. |
| OneOf | Trial every child. With exactly one passing child, Merge that child's sets into produced and publish. With zero or multiple matches, discard all child sets without union work. |
| Not | Always discard the trial's sets; only invert a completed Boolean result. |
| If | Trial the condition. On true, immediately Merge its sets into local; on false discard them. Eval only the selected branch in a fresh scope; its returned sets become produced. Thus a successful condition contributes locally even if then fails, while the whole failing node still exports no sets. |
| DependentSchemas | Eval every present trigger against the whole object. If any triggered child fails, discard all child sets. Otherwise Merge each successful child's sets into produced and publish. |
| Contains | Record all matching indices from all completed item trials. If contains itself passes (a match exists or minContains is zero), Merge those indices directly into local. Count assertions determine the instruction's Boolean result separately; produced is empty. A failing minContains/maxContains does not erase contains' local annotation, but a failing node exports nothing. |
| Properties, PatternProperties, Items, PrefixItems, either AdditionalProperties, either Unevaluated | Record the immediate members/indices to which the keyword applied. Child descendant sets do not move up a level. Publish produced only if every applicable child passed. |
| PropertyNames, Required, DependentRequired | Produce no evaluated-value sets. |

Trial evaluation diverts only mismatch findings. Depth, active identities,
numeric caches and all work/equality counters remain shared and are never reset
or rolled back. EvaluationFailure immediately propagates through every rule.

### Limits and order

Schema entry, each instruction, and collection/branch visits consume the shared
maxEvaluationSteps allowance. NFA transitions consume that same allowance.
Annotation-set merging charges each candidate insertion, including duplicates.
Equality, numeric-byte and depth limits keep their existing meanings. Zero
maxErrors alone means unlimited reporting; the other zero limits allow no work.
Reporting caps never make an incomplete evaluation valid or suppress a failure.

Instruction and tuple-list order is compiled order; array order is index order.
Object visits and annotation-set merges use decoded Unicode-scalar lexical key
order (no Unicode normalization). This is a visit budget, not complete CPU/heap
accounting. Property-name temporary strings have independent instance identity
and do not alter schema/source identity or numeric caches.

More specifically, Properties charges one visit per declared name, Required one
per required name, and Items/PrefixItems one per visited element. AllOf/AnyOf/
OneOf charge one visit before each child. If adds no extra branch visit beyond
the instruction and child entries. DependentRequired charges each trigger and,
when present, each required name; DependentSchemas charges each trigger.
Contains charges each element. PatternProperties charges each object member and
each pattern attempt before the NFA's own charges. AdditionalPropertiesWithPatterns
charges each member and each exclusion-pattern attempt until one matches;
declared names skip pattern attempts. PropertyNames and unevaluated checks charge
each member/index examined, including already evaluated ones for the latter.

Recording an immediate member into a keyword's temporary set is covered by that
visit; it does not add a separate step. The explicit Merge operations above add
one step per candidate. Moving a child set is not a merge. String-byte copying
and set implementation costs are not separately metered by this visit profile;
native implementations may optimize storage while retaining these logical
charges. This prevents the same declared limit from changing merely because a
runtime uses bitsets instead of ordered sets.

## Primary oracles and scope

- [JSON Schema 2020-12 Core §7](https://json-schema.org/draft/2020-12/json-schema-core#section-7): scope, applicability and annotation behavior.
- [Core §10](https://json-schema.org/draft/2020-12/json-schema-core#section-10): composition, conditionals, dependencies and object/array applicators.
- [Core §11](https://json-schema.org/draft/2020-12/json-schema-core#section-11): unevaluated locations and successful annotation propagation.
- [Validation §6](https://json-schema.org/draft/2020-12/json-schema-validation#section-6): exact counts, contains bounds and dependentRequired.
- Official JSON Schema Test Suite cases, with pinned provenance, supplement the
  independent Valid/Invalid/EvaluationFailure fixtures in this tranche.

This does not claim full dialect conformance. Canonical `$id`/`$self` resource
and dynamic-scope metadata remain coordinated with the IR/resource owner; no
static reference or flattened property list substitutes for those semantics.

## Landed fixtures and verification

- Maintained cross-language source fixture:
  `crates/suspect-schema/tests/fixtures/owned-applicators-v2.json` — **32 cases**,
  with exact schemaJson/instanceJson strings, limits, independently specified
  Valid/Invalid/EvaluationFailure outcomes and responsible source pointers.
- Ready-to-execute portable descriptors:
  `target/sdk-schema-applicators-executable-v2.json` — the same **32 cases** with
  their actual checked OwnedProgram values. Source document URIs are the original
  test inputs, retained by Contract even after their loader lifetimes end.
- `target/sdk-schema-applicators-verified-02.log`: **12 focused tests passed**,
  including all 32 vectors, v1 byte equality/inert behavior, v1 rejection of new
  forms, malformed v2 program admission, exact counts, pattern exclusions,
  property-name paths/equality, short circuit, recursion and shared failures.
- `target/sdk-schema-applicators-verified-01.log`: the new official-file runner's
  **11 tests passed, 395 original cases executed**. The exact deferred cases are
  two dynamic/$id groups (four cases) and the Unicode-property-escape pattern
  group (two cases), each checked for its concrete located Unsupported result.
  They are not counted as successful validation. Original expectations and files
  were not patched; fixture provenance is in the new applicator-conformance README.
- `target/sdk-schema-applicators-clippy-02.log`: library and both new test targets
  pass warnings-denied Clippy. Scoped formatting/whitespace checks pass.

The new depth stress initially reproduced a debug-build stack overflow. Opcode
rules now have isolated call frames behind a small dispatcher. The default 512
depth limit reports EvaluationFailure cleanly on a 2 MiB thread stack in **both
v1 and v2**; see `sdk-schema-applicators-depth-01.log` (red), `depth-02.log` and
the final focused suite (green). No depth/work limit was lowered to hide the bug.

The affected existing owned suite recorded **23 passed, 1 integration assertion
failed, 2 existing ignored**. Its failing
`selected_nested_roots_cannot_bypass_an_unsupported_enclosing_schema_resource`
test expects the old Contract `unsupported-schema-resource` diagnostic at the
parent schema. The concurrent IR resource work removed that diagnostic; Owned
still refuses the resource at its real `$id` keyword. Main/resource ownership
must migrate that old expectation with resource admission. This task did not
restore an obsolete IR diagnostic or edit that existing test.

The prior shared native fixture
`crates/suspect-codegen/tests/fixtures/runtime-contract-v1.json` remains untouched:
SHA-256 `49fd80574c704a397d3f513ab886ecfdd90aa93b6f67c1af938935cf99a3a646`.
Earlier native/dialect matrices were not rerun merely for this tranche or recovery.
