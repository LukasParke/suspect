# Canonical TypeScript directional codec views

The TypeScript model codec planner supports bounded request/response views.
The `typescript-http` profile requests these views and packages their native
types, codecs and docs through the shared CLI, session and editor pipeline.

## Public API

`suspect_codegen::typescript::codecs::plan_codecs_with_views(
contract: Arc<Contract>, roots: &[SchemaId], views: &[ModelView],
config: CodecConfig) -> Result<CodecPlan, Vec<ModelDiagnostic>>` plans
`Neutral`, `Request` and `Response` views simultaneously in one package.
`plan_codecs` is equivalent to `plan_codecs_with_views` with
`[ModelView::Neutral]`.

Every (source, view) pair receives its own collision-allocated symbol, its own
`<Symbol>Codec` export in `model-codecs.ts` typed as `Codec<models.<Symbol>>`,
its own validation root, and its own manifest entry carrying `name`, `model`,
`view`, `file` and source identity. Documentation comments are rendered from
each symbol's actual view, never from a hardcoded `Neutral`.

The installable package derives its documented views from these actual symbols:
`package.json`'s `suspect.modelView` carries the distinct implemented views in
canonical order (for example `Neutral`, `Request+Response`). Package metadata
and documentation describe those selected views.

## Directional semantics: one shared proof

The single applicability authority is
`typescript::directional_annotation_walk` (private to the module, consumed by
both call sites). A required member's presence is relaxed in a view if and
only if the shared walk proves the view's annotation — `readOnly` for
`Request`, `writeOnly` for `Response` — applicable to that exact property
position: the property schema declares the keyword itself, or the annotation
is reachable through an unconditional `$ref` target chain. Because the model
tracer's `object` lowering and the runtime projection both route through this
one walk, model requiredness and runtime requiredness cannot disagree.

Annotations reachable only through applicators (`allOf`, `anyOf`, `oneOf`,
`if`/`then`/`else`, `not`) are never proven. Each unestablishable child
produces a source-linked `directional-annotation-evaluation` error that fails
planning for directional views; the neutral view of the same contract still
plans. This is the recorded normative position: evaluated annotation
applicability would require an annotation-collecting validator, which the
owned program does not implement. Guessing is prohibited.

## Explicit 3.1 request/response validation policy

[JSON Schema Validation §9.4](https://json-schema.org/draft/2020-12/json-schema-validation#section-9.4)
specifies that applicable readOnly/writeOnly occurrences SHOULD behave as true
when any occurrence is true. A false sibling of an unconditional reference does
not cancel its target's true annotation. This combination rule does not establish
applicability through unsupported conditional/composition branches.

OpenAPI 3.1+/3.2 use JSON Schema annotation semantics (JSON Schema Validation
§9.4): read-only and write-only are annotations, and the owning authority may
ignore or reject modifications to read-only values. The implemented policy:

- Request and response views keep every declared wire property. No data is
  stripped, rejected, defaulted, or reordered.
- Only the presence requirement changes, only for proven directional
  annotations, and only at the original object positions. All other compiled
  instructions — instance types, exact numeric bounds, patterns, cardinality,
  `additionalProperties`, composition including `oneOf` exclusivity,
  references, recursion — are retained unchanged with their source
  identities.
- Every provided value still validates. A read-only value supplied in a
  request (or a write-only value in a response) encodes, decodes and
  validates; the authority-accepts-unchanged-value behavior is the modeled
  3.1 stance.
- Both encode and decode of a view codec validate against that view's
  projected program, so the wire contract a producer must satisfy and the one
  a consumer accepts are the same program.

This projection is a requiredness projection. It is deliberately not the OAS
3.0 rule.

## OpenAPI 3.0 stays unsupported

OpenAPI 3.0 normatively reverses the direction in which `required` applies
for `readOnly`/`writeOnly`. The owned validator compiles only the OAS 3.1 /
JSON Schema 2020-12 subset, so a 3.0 directional codec plan fails
`codec-schema-compilation` instead of guessing 3.0 semantics. Faithful 3.0
support requires a 3.0-frontend validator profile first.

## Program artifacts

- The first distinct projected program (canonical view order
  Neutral → Request → Response, independent of caller order) is emitted
  through the shared validation seam as `typescript/validation-program.ts`,
  together with the validator runtimes and `validation.md`.
- Additional distinct projections are emitted as
  `typescript/validation-program-request.ts` and
  `typescript/validation-program-response.ts`; `model-codecs.ts` imports each
  codec's validators from its view's module with deterministic aliases.
- Programs are deduplicated by structural equality. A closure without
  directional annotations emits exactly one validation program no matter how
  many views are requested; requesting directional views never invents
  directional semantics.
- `validation.md` documents the first distinct program; per-view binding is
  documented in `codecs.md` and the docs manifest.

## Implementation boundaries

The projection mutates only `required` instructions in the portable
`OwnedProgram` snapshot and then re-runs its portable invariant checks
(`codec-directional-projection` on failure). It never reparses Contract JSON,
never drops nodes, and never touches non-required instructions. Resource
policies (`CodecConfig`) are unchanged and per-view shared.

HTTP transport and packaging now integrate directional views: the HTTP planner
requests `Request`/`Response` views for annotated closures (recorded as
`directionPolicy: "oas31-required-applicability-v1"` in `http-manifest.json`,
`neutral-equivalent` otherwise), and `typescript::package` derives its
documented views from the plan's symbols. Packaging additionally documents the
explicit 3.1 requiredness policy in the package README and rejects claiming a
neutral-only package for directional plans. Still open and unchanged from the
model plan: release promotion, allOf-carried annotation collection,
unevaluated applicators, and OAS 3.0 directional support.

## Focused verification

`crates/suspect-codegen/tests/typescript_directional.rs` covers located
diagnostics (unestablishable applicator annotations, OAS 3.0 rejection),
determinism and view-order independence, manifest view binding, program
dedupe for unannotated closures, neutral-view equivalence, and native
strict TypeScript consumers (`strict`, `exactOptionalPropertyTypes`,
`noUncheckedIndexedAccess`) exercising encode/decode, absence/null, retained
read-only values, recursion at every projected node, and preserved `oneOf`
exclusivity, plus positive/negative static view assignments.

```sh
# Non-native checks.
cargo test --locked -p suspect-codegen --test typescript_directional
# Native checks (need tsc and Node 22 on PATH).
cargo test --locked -p suspect-codegen --test typescript_directional -- --include-ignored
# With the pinned sibling checkout for the corpus case:
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
  cargo test --locked -p suspect-codegen --test typescript_directional -- --include-ignored --nocapture
```

Regression insurance for the unchanged neutral path:

```sh
cargo test --locked -p suspect-codegen --test typescript_codecs \
  --test typescript_codecs_docs --test typescript_contract -- --include-ignored
```
