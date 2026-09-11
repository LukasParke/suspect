# Owned SDK schema dialect admission

Updated 2026-09-10. This is the capability boundary for
`OwnedCompiler::compile(Arc<Contract>, roots)`, `OwnedSchema::program().check()`,
and owned JSON-value evaluation. It expands the initial scope recorded in
[SDK-OWNED-VALIDATION.md](SDK-OWNED-VALIDATION.md). It does **not** claim full
JSON Schema dialect conformance or end-to-end OpenAPI support by every native
SDK backend.

## Stable instruction boundary

The immutable Contract remains the source of schema identities, references,
values and spans. Normalization does not serialize/reparse schemas, insert
defaults, mutate Contract, invent schemas, or expand recursive reference graphs.
Every emitted node still names an indexed Contract schema. Every check names an
actual source value.

The existing envelope remains:

```json
{
  "version": "suspect.validation.experimental.v1",
  "profile": "oas31-jsonschema202012-static-subset"
}
```

**No `ProgramInstruction` variants, op tags, operands, or limit fields were
added.** The profile describes executable instruction semantics; admitted
OAS 3.0 input is normalized to those same semantics. Existing valid v1 programs
and their source locations remain admitted. The checked-program guard adds one
source-location case: an exclusive `bound` may originate at `minimum` or
`maximum`, reflecting the original numeric operand of an OAS 3.0 bound.
Modern `exclusiveMinimum`/`exclusiveMaximum` locations retain their prior rules.
Duplicate sources, mislocated bounds, invalid numeric tokens and broken graph
targets still fail admission.

`Valid`, `Invalid`, and `EvaluationFailure` remain distinct. Numeric byte limits,
equality work, recursion depth/cycles, and shared evaluation work retain the
existing semantics. `not`, `allOf`, `anyOf`, and `oneOf` cannot hide or invert
evaluation failures. These are limits on the compiled evaluation, not complete
CPU/heap accounting or an assertion about source-interpreter visit counts.

## Admitted source dialects

| Input | Admission |
| --- | --- |
| OAS 3.0.x | Version-specific Schema Object rules below, lowered to the existing static subset |
| OAS 3.1.x standard base dialect | Existing 2020-12 static subset plus the explicitly inert forms below |
| OAS 3.2.x standard base dialect | Same assertion vocabulary; version-aware OAS annotation declarations |
| Explicit JSON Schema 2020-12 | Same static subset, including explicit null, Boolean schemas and schema `$ref` siblings |
| Other/custom dialects | Located `Unsupported` error; vocabulary semantics are not guessed |

The two admitted URI dialects are
`https://spec.openapis.org/oas/3.1/dialect/base` and
`https://json-schema.org/draft/2020-12/schema`, with an optional empty fragment.
**OAS 3.2 still uses the OAS 3.1 base dialect URI.**
`https://spec.openapis.org/oas/3.2/dialect/base` is not an admitted dialect.

A schema resource's `$schema` overrides `jsonSchemaDialect`. Referenced OpenAPI
documents have their own defaults; standalone referenced schemas use the
context retained by Contract. Invalid URI/string declarations and unsupported
effective dialects report the original `$schema` or `jsonSchemaDialect` source,
including external files and inherited declarations. A property map, example,
default, or property *named* `$schema` is not a dialect declaration.

`$schema` on a nested non-resource schema is invalid: selecting that schema as
an SDK root does not create a schema resource. `$id`-established resources remain
explicitly refused because their canonical identity/base-URI indexing is absent
from this Contract boundary.

## Fully enforced assertions within the admitted subset

| Area | Behavior |
| --- | --- |
| Types | Exact JSON types and mathematical integers; modern type arrays/Boolean schemas; OAS 3.0 type/nullable normalization |
| Numbers | Exact inclusive/exclusive bounds and positive `multipleOf`, including huge integers, tiny fractions and symbolic exponents |
| Literal equality | `enum`, modern `const`, `uniqueItems`; structural object/array equality and exact numbers |
| Counts | Nonnegative mathematical string/array/object cardinalities, including bounds beyond machine integers; Unicode character counts |
| Objects | `properties`, `required`, `additionalProperties`; decoded names and local additional-property scope |
| Arrays | `items`, modern `prefixItems`; positional applicability never implies required array length |
| Composition | Independent `allOf`, inclusive `anyOf`, exactly-one `oneOf`, and `not` |
| References | Contract-indexed static local/external/recursive references; modern anchors; dialect-specific sibling rules |
| Patterns | Existing bounded portable NFA; OAS 3.0 admits only its ECMA-262 5.1-compatible portion described below |

Applicability does not imply a type. An untyped `minimum` does not exclude null;
an untyped `required` does not exclude scalars. `required` checks presence, not
non-nullness. Local `additionalProperties` cannot be expanded with the properties
of another `allOf` member. An impossible enum/type intersection remains an
unsatisfiable schema; compilation does not invent another allowed value.

### OAS 3.0 normalization and declaration restrictions

- `type` must be one of the six non-null type names (including mathematical
  `integer`), written as one string. Type arrays and `type: null` are invalid.
- `nullable` must be Boolean. `true` adds null only to the type check on the
  **same Schema Object**. It does not add an unconstrained null branch outside
  enum, `not`, references or composition. Without a same-object type it has no
  effect, and `nullable: false` alone does not prohibit null.
- `exclusiveMinimum` and `exclusiveMaximum` are Boolean modifiers of the
  corresponding same-object numeric bound. The bound keeps its exact numeric
  token and `minimum`/`maximum` source. A modifier without a bound has nothing
  to modify; no default bound or Boolean-to-number conversion is invented.
- `items` must be an object and must be present when the same object's `type`
  is `array`. Tuples and Boolean `items` are invalid. Properties, composition
  members and `not` require Schema/Reference Objects. Literal
  `additionalProperties: true/false` remains valid.
- A Reference Object applies **only** its `$ref`. All siblings are ignored,
  including malformed/unsupported siblings and their schema subtrees and
  Contract diagnostics. Ignored subtrees do not expand the compiled closure.
  Explicitly selecting such a subtree independently still subjects it to
  admission. A 3.0 reference must target an OAS 3.0 Schema Object, not a Boolean
  schema or a different declared dialect requiring conversion.
- `required` arrays must be nonempty with unique strings. `allOf`, `anyOf`, and
  `oneOf` arrays must be nonempty in **both** dialect families.
- `enum` must be an array. Wright-00 §5.20, which OAS 3.0 incorporates, says
  **SHOULD**, not MUST, for nonempty/unique entries. The modern wording does too.
  Empty enums remain false assertions; duplicate and heterogeneous entries keep
  their exact equality semantics. These recommendations are not promoted to
  mandatory declaration errors or eager equality-budget checks.
- `default` must conform to a same-object declared type, including nullable.
  Integer conformity uses exact arithmetic and reports numeric budget exhaustion
  at the default's source. Other constraints do not become extra default
  admission rules. Defaults never populate absent instance properties.
- `readOnly` and `writeOnly` cannot both be true. Their effect on **required**
  depends on request/response direction. The neutral compiler refuses affected
  requirements, including conservative in-place reference/composition cases,
  until an explicit directional validation view exists. It does not make those
  requirements unconditional or remove them for all uses.
- Keywords outside the 3.0 fixed vocabulary require `x-` extension names.
  Modern/legacy assertions such as `const`, `contains`, `prefixItems`, `$defs`,
  `$schema`, or `dependencies` are located errors, not modernized no-ops.

OAS 3.0 cites ECMA-262 5.1, whose regex character unit is UTF-16, while the
existing portable NFA consumes Unicode scalars. Positive BMP character sets
with compatible escapes share results on well-formed Unicode and are admitted.
Dot, complement/astral matches, `\u{...}`, legacy-only escapes, and `\s`/`\S`
with version-dependent whitespace behavior are refused as requiring a separate
pattern profile. This avoids silently giving `^.$` modern single-code-point
behavior on astral text. Modern portable patterns retain their prior behavior
and limitations; see [SDK-OWNED-VALIDATION.md](SDK-OWNED-VALIDATION.md).

## Modern annotations and explicitly inert forms

Modern defaults, examples, format/content metadata, read/write annotations and
unknown individual keywords remain non-validating. `format_assertion: true`
with a valid `format` declaration is explicitly unsupported. Malformed format
declarations are invalid even in assertion mode.

The following known forms have no assertion to lower and are admitted after
declaration checks:

- `minContains`/`maxContains` without adjacent `contains`;
- empty `patternProperties` and `dependentSchemas` maps;
- `dependentRequired` entries containing only the trigger itself or no names;
- `if` without `then` or `else`, and `then`/`else` without adjacent `if`;
- Boolean-true `unevaluatedProperties`/`unevaluatedItems`.

Retained schema-valued children still undergo the selected Contract closure's
declaration/capability admission; this includes `$defs`, `contentSchema`, and
unused conditional children. Unsupported child schemas can therefore refuse
compilation even though they would not execute. This is an explicit static
profile limitation, not a claim that the standard applies their assertions to
the containing instance. String-encoded content is never decoded/validated as
part of the enclosing JSON value's validation.

OAS `discriminator`, `xml`, and `externalDocs` declarations have their fixed
field shapes checked. In OAS 3.2, `discriminator.defaultMapping` is a string
annotation and `xml.nodeType` admits `element`, `attribute`, `text`, `cdata` or
`none`; `nodeType` excludes `attribute` and `wrapped` even when those are false.
These 3.2-only fields are not treated as defined 3.0/3.1 fields. Under explicit
plain JSON Schema 2020-12, OAS-only keywords are arbitrary annotations instead.

A discriminator does not select an otherwise-invalid branch or resolve an
overlapping `oneOf`. This compiler does not implement discriminator-based codec
dispatch, mapping-target validation, the OAS requirement for defaultMapping
when the discriminating property is optional, or XML serialization. Those need
native/application capability checks. It retains their original annotations.

In contrast, OAS 3.2's **OpenAPI Object** `$self` changes document identity and
reference bases. It is refused at `/$self` pending canonical Contract indexing.
A `$self` property on a Schema Object, including a standalone schema document,
is only an unknown annotation. The word alone does not give it OAS document
semantics. Querystring parameters, additional operations, item streaming and
other 3.2 HTTP features also require their own wire-layer admission.

## Refused features and the concrete missing operations

| Feature | Required capability; why existing instructions are insufficient |
| --- | --- |
| Active `if`/`then`/`else` | One condition trial and only the selected branch. Eager `allOf`/`anyOf` rewrites evaluate skipped branches and expose spurious budget/cycle failures; synthetic helper schema addresses would lose provenance. |
| Nontrivial `dependentRequired` | Object-only presence trigger and required names. Null is present; `properties` visits values and cannot enforce this on the containing object. |
| Nonempty `dependentSchemas` | Object-only presence trigger applying a target to the entire object, with shared failure/work accounting. |
| `contains` with min/max counts | Array-only per-item trials and exact successful-match counts. `not(items(not S))` fails non-array applicability and requires unsupported helper identities; full contains also contributes evaluated-item annotations. |
| Nonempty `patternProperties` | Pattern-to-value application, overlapping matches, and same-object pattern exclusions for additionalProperties. |
| `propertyNames` | Evaluation against property-name strings with source/instance paths; properties only evaluates member values. |
| Nontrivial unevaluated applicators | Scoped successful-evaluation property/item sets, branch isolation and annotation propagation across references. |
| `$id`, OAS document `$self` | Canonical resource/base-URI identities and indexed resolution in Contract first; a new execution opcode alone is insufficient. |
| Dynamic/recursive references and anchors | Dialect-specific resource and dynamic-scope indexing plus runtime scope-aware resolution. |
| Custom dialects, `$vocabulary` | Explicit meta-schema/vocabulary admission. The SDK profile also declines ordinary schema documents containing `$vocabulary`; it does not claim the Core rule requires rejecting them. |
| Legacy `definitions`/`dependencies`/`additionalItems` | Explicit legacy traversal/dialect normalization; no accidental legacy semantics in 2020-12. |
| Assertion-mode formats | A declared portable format assertion policy and matching implementations. |
| OAS 3.0 directional required | Explicit request/response validation view preserving original required/property provenance. |

These are `Unsupported` outcomes with located, concrete capability messages,
not ignored validation assertions. Malformed supported/known declarations are
`Invalid`; exact-number admission exhaustion is `ResourceLimit`.

## Phase 2: implemented native views in the five backends

Rust, Python, Go, Swift and TypeScript now apply the dialect view in model
planning and codec-root selection. This implements the representable 3.0 subset;
it is no longer a blanket 3.0 dialect guard. Main's operation-role naming hints
remain integrated in each allocator, and the existing native-name tests pass.

The shared `crates/suspect-codegen/src/schema_view.rs` is registered from the
owned `typescript.rs` rather than modifying Main's `lib.rs`. Other backends can
use these **`pub(crate)`** helpers at `crate::typescript::schema_view`:

| Helper | Contract |
| --- | --- |
| `raw(Schema) -> Cow<Value>` | Borrows ordinary schema data; exposes only the original `$ref` on a 3.0 Reference Object. It does not inline a target or mutate Contract. |
| `reference_only(Schema)` | Per-schema dialect decision, including external documents. |
| `closure(&Contract, roots)` | Finite, deterministic containment/reference closure, excluding ignored 3.0 sibling subtrees. |
| `diagnostic_applies(...)` | Filters raw Contract diagnostics against that effective closure; ignored ref siblings cannot poison an admitted model. |
| `null_allowed(&Contract, id) -> Result<bool, Problem>` | Intersects type/nullable, enum/const, references and composition, including exactly-one null matching. Incomplete/cyclic/depth/work-limited proof is a located refusal, not a non-null conclusion. |
| `accepts_literal(Schema, &Value)` | Local literal/type intersection, including same-object 3.0 nullable; enum/composition remain separate constraints. |
| `integer_interval(Schema)` | Exact sufficient integer-range proof combining inclusive bounds and dialect-correct exclusivity. No floating-point conversion. |
| `description(Schema)` | Ignores 3.0 ref-sibling prose while retaining original raw source separately. |
| `problems(&Contract, closure)` | Early model-only refusals for invalid 3.0 type forms and directional required needing an unsupported validation profile. |

Rust, Python and Go use the shared nullability proof. Swift continues to use
the admitted owned validator to establish nullability. TypeScript retains its
source-located union/intersection AST and its existing additional-property and
intersection representation proofs, now fed the dialect view. Ignored null-only
type/enum/const hints cannot remove a 3.0 reference branch from native unions.
Python constructor defaults likewise cannot come from ignored `const` siblings.

Rust and TypeScript now use the exact range proof for native integer selection,
including `-1 < integer < 256` becoming Rust `u8` and a safe TypeScript `number`.
The proof handles integral endpoints representable in i128; fractional or larger
endpoints retain an exact carrier instead of being rounded. Rust also retains
its prior u128-capable inclusive proof. Python `int`, Go `Integer`, and Swift
`JsonInteger` remain lossless carriers; no unsupported fixed-width conversions
were added to their runtimes.

Python/Go codec root collection and Swift's validation-root/directional preflight
now use the effective closure instead of selecting every raw ignored child as
an independent validation root. Swift's changes in `swift_sdk.rs` are confined
to that model/validation planning. HTTP wire admission, runtime implementations,
package emission, shared naming, compatibility adapters and portable opcodes
were not changed by this phase.

### Native coverage and remaining representation limits

The new `crates/suspect-codegen/tests/sdk_dialect_models.rs` compiles and runs
real consumers in **all five toolchains** for OAS 3.0.4 and OAS 3.1.2. Both inputs
also reference an external OAS 3.0 document. Cases cover:

- required nullable values, optional absence and explicit null round trips;
- nullable intersected with enum, exact wide integers and exclusive bounds;
- invalid scalar mutations rejected on encoding as well as decoding;
- ignored invalid/unsupported/ref/direction/const/prose siblings and subtrees;
- reference alternatives whose ignored null-only hints must not erase a branch;
- exactly-one rejection for overlapping number/integer alternatives;
- preserved source IDs and Main's readable native names.

TypeScript additionally compiles positive/negative consumer types and executes
a modern `$ref` + `type: string` intersection whose referenced schema permits
null: the sibling still excludes null. Rust/Python/Go/Swift continue to refuse
general unimplemented modern ref intersections at source. Rust/Go/Swift retain
their bounded transparent-allOf layouts; Python still refuses general allOf and
nullable named-object layouts that require a separate non-null declaration.
Other existing literal/tuple/intersection representation limits remain explicit.

OAS 3.0 directional required remains a located model-and-codec refusal for all
views until an admitted directional validation profile is supplied. TypeScript's
explicit modern Request/Response annotation policy remains supported and keeps
its existing codec diagnostic classification for the 3.0 refusal. No modern
policy is silently borrowed for 3.0. The owned validator's resource, pattern,
contains, conditional, dynamic and unevaluated limits above still apply.

Actual native evidence:

- `target/sdk-schema-phase2-native.log`: Rust, Python, TypeScript/Node and Swift
  consumers passed both dialect inputs. Its initial Go test used the wrong
  wrapper field (`Null` instead of `Nullable.IsValue`).
- `target/sdk-schema-phase2-go-followup.log`: corrected Go consumer passed both
  inputs, including mutable encoding, absence/null and exact values.
- `target/sdk-schema-phase2-ts-followup.log`: both dialect inputs plus the
  additional modern ref-intersection consumer passed strict tsc and Node.
- Installed/current tools: Rust/Cargo 1.97.1, Python 3.14.7, Go 1.27.1,
  TypeScript 5.9.3 / Node 26.8.1, Swift 6.3.3.
- Installed floor checks: generated Rust consumers **and their 13 doctests**
  passed on Rust **1.88.0** for both inputs (`sdk-schema-phase2-rust-floor-*.log`);
  both Python consumers passed on **Python 3.11**
  (`sdk-schema-phase2-python-floor-*.log`). No uninstalled-toolchain floor claim.

Native test functions are marked toolchain-required in the maintained Rust
test harness, consistent with existing suites. The evidence above explicitly
runs them; it is not an emission-only or ignored-test success claim.

Final focused regression evidence:

- `target/sdk-schema-phase2-regression-final.log`: **39 passed, 0 failed**,
  across native model, naming, TypeScript directional/intersection, Swift SDK
  and new dialect host suites. The 30 toolchain/corpus-required tests in that
  command remain marked ignored; the new native consumers were explicitly run
  separately as documented above. All four Main-owned naming tests passed.
- `target/sdk-schema-phase2-proof-final.log`: the additional incomplete
  nullability proof regression passed for cycles, active conditionals and dynamic
  references in Rust, Python and Go. The current codegen checkout compiled.
- Scoped rustfmt and whitespace checks passed. Concurrent docs/IR edit-window
  build failures were resolved before these final checks.

## Initial native planner handoff (superseded by phase 2 above)

Owned admission is a validation gate, not proof of a faithful native layout.
Before admitting OAS 3.0 packages in each backend:

1. Use each source schema's `Schema::dialect()`, including external documents.
   A modern entry can reference a 3.0 schema. Entry-version-only checks are
   insufficient. Until a target is verified, retain a located dialect guard.
2. Apply the 3.0 ref-only rule in **every** raw-schema consumer: closure planning,
   nullability, enum/type/composition analysis, additional properties, docs and
   direction/view projection. Rust `null_allowed`, Python `nullable`, and Go
   nullability currently intersect ref siblings. TypeScript has a ref-only
   lowering path, but auxiliary analyses also need the rule. Swift's validated
   nullability is useful, while its raw ref-sibling layout admission needs the
   dialect distinction.
3. Same-object nullable changes only type admission. Retain enum/composition
   restrictions and all four required/optional × null/non-null states.
4. Consume normalized bound flags; never interpret a 3.0 Boolean exclusive flag
   as a numeric bound or ignore its corresponding numeric constraint.
5. Keep the explicit directional-required refusal until a source-backed
   request/response view is implemented. Modern readOnly/writeOnly annotations
   do not normatively relax required in the same way as 3.0.
6. OAS 3.2 annotations are not permission for discriminator dispatch, XML,
   canonical `$self` resolution, or new HTTP behavior. Gate each native feature
   separately. The present change requires no new runtime opcode handlers.

Public APIs and signatures are unchanged. Main's existing nested test at
`crates/suspect-schema/tests/owned/contract.rs:187–199` expects every 3.0 schema
to be unsupported; its simple string fixture now needs positive/negative
evaluation assertions. That nested file is outside this task's literal
`tests/owned*.rs` edit ownership and was not masked or modified.
Main subsequently updated that obsolete assertion and verified the focused
follow-up in `target/sdk-schema-contract-followup.log`.

## Verification and primary oracles

- Initial owned baseline: **24 passed, 2 existing acceptance tests ignored** in
  `target/sdk-schema-baseline.log`.
- New independent dialect/adversarial tests exercise public compilation,
  `program().check()`, original source/span presence and evaluation. They cover
  nullable/absence, exact arithmetic, exclusive bounds, enum recommendations,
  overlap, ignored siblings, external/recursive identity, declaration errors,
  scope and noninvertible resource failures.
- `owned_normative.rs` runs **340 original cases from 15 unmodified official
  files** through each of OAS 3.1, OAS 3.2 and explicit JSON Schema 2020-12:
  **1,020 evaluations**, all passing. Upstream revision and license remain in
  `crates/suspect-schema/tests/conformance/README.md`.
- `target/sdk-schema-normative-tests.log` records the first passing 20-test
  dialect run and all three official-file runs. The final
  `target/sdk-schema-final-tests.log` records **21 passing dialect tests**,
  the three passing official-file runs, and the full suite below.
- Existing owned regression: **23 passed, 1 obsolete blanket-3.0 assertion
  failed, 2 ignored** in `target/sdk-schema-owned-regression.log`. This is a
  reported integration task, not a passing full-suite claim.
- Full `cargo test --locked --offline -p suspect-schema --target-dir
  target/sdk-schema-build --no-fail-fast -- --nocapture`: **159 passed, 1 failed,
  5 existing acceptance tests ignored** across unit, integration and doc tests.
  The sole failure is the obsolete nested blanket-3.0 rejection above. All
  targets compiled; `--no-fail-fast` let every remaining suite finish. The
  intentional `should_panic` conformance test passed and is not a second failure.
- Scoped `rustfmt --check` and `git diff --check` passed. No performance
  calibrations, baseline expectations, upstream fixtures, or native language
  files were edited. Native package/runtime acceptance remains Main's gate.
- Final `cargo check --locked --offline -p suspect-schema --target-dir
  target/sdk-schema-build` passed in `target/sdk-schema-final-check.log`.

Primary sources, used as the oracle rather than implementation-generated
expectations:

1. [OAS 3.0.4 Schema Object](https://spec.openapis.org/oas/v3.0.4.html#schema-object),
   [Reference Object](https://spec.openapis.org/oas/v3.0.4.html#reference-object),
   [Data Types](https://spec.openapis.org/oas/v3.0.4.html#data-types).
2. [Wright-00 Validation §§4–6](https://www.ietf.org/archive/id/draft-wright-json-schema-validation-00.txt):
   missing keywords, Boolean exclusivity, required/enum/composition declarations.
3. [JSON Schema 2020-12 Core](https://json-schema.org/draft/2020-12/json-schema-core):
   §§7–8 scope/identifiers, §10 applicability/conditionals, §11 unevaluated locations.
4. [JSON Schema 2020-12 Validation](https://json-schema.org/draft/2020-12/json-schema-validation):
   §§4, 6 exact numbers/equality/counts/dependencies; §§7–9 annotations.
5. [OAS 3.1.2 Schema Object](https://spec.openapis.org/oas/v3.1.2.html#schema-object).
6. [OAS 3.2.0 Schema Object](https://spec.openapis.org/oas/v3.2.0.html#schema-object),
   [Discriminator](https://spec.openapis.org/oas/v3.2.0.html#discriminator-object),
   [XML](https://spec.openapis.org/oas/v3.2.0.html#xml-object),
   [OpenAPI Object / `$self`](https://spec.openapis.org/oas/v3.2.0.html#openapi-object).
7. [ECMA-262 5.1 regular expressions](https://262.ecma-international.org/5.1/#sec-15.10)
   and the [official JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite).
