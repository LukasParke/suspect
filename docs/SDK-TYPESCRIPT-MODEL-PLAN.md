# TypeScript model planning

`suspect_codegen::typescript::plan_models(&contract, roots, views)` builds the
immutable model plan used by TypeScript/JavaScript codecs and the
[`typescript-http` profile](SDK-TYPESCRIPT-HTTP.md). It selects canonical
`SchemaId` roots and `Neutral`, `Request` or `Response` views, follows resolved
references and allocates symbols before lowering.

The public plan exposes symbols, typed expressions, source IDs, views, file paths
and source-linked diagnostics. Code and documentation use that same allocation.
Recursive references retain finite source identities; unrepresentable recursion
or conflicting native representations fail planning.

## Representation boundaries

| Source contract | Native model |
| --- | --- |
| Boolean schemas | `true` → exact `JsonValue` domain; `false` → `never` |
| Presence/null | Requiredness and nullable values stay separate; consumers use `exactOptionalPropertyTypes` |
| Objects | Exact wire names, named fields, open/closed shapes and typed maps; named fields plus typed extras require a bounded representation proof |
| Composition | Explicit unions/intersections; `oneOf` exclusivity and other assertions are runtime obligations |
| Scoped applicators | Checked v2 codec obligations; conditional/dependency/unevaluated scopes are not flattened |
| Patterns and tuples | Named fields retain types; pattern extras and heterogeneous prefix items use a lossless `JsonValue` carrier checked on decode/encode |
| Literals | Supported exact enum/const values retain their identity; incompatible numeric representations are rejected |
| Integers | Unbounded values use `bigint`; proven safe bounded values can use `number` |
| General numbers | Exact `JsonNumber` with the shared JSON runtime |
| References/names | Stable source URI/pointer identity, guarded recursion and deterministic collision allocation |
| Dynamic references | Checked v3 `JsonValue` carrier at dynamic positions, indexed candidate bindings and separate logical/physical metadata |
| Directions | Retain every field and apply bounded, dialect-aware requiredness rules; neutral models preserve declared requiredness |

Integer normalization uses decimal digits/exponents without floating-point
conversion. Mathematically integral spellings such as `1.0` and `1e3` remain
exact. Native model assignability alone does not prove JSON validity, ranges,
closed-object exactness or union exclusivity.

OpenAPI 3.0 `nullable` modifies an explicit type; OpenAPI 3.1+ retains it as an
annotation. Model planning preserves these distinctions, while codec admission
requires the supported owned-validator dialect. The nine scoped v2 operations
are implemented by the [native codec](SDK-TYPESCRIPT-VALIDATION-V2.md); unsupported
dialect features and unproven compositions remain explicit planner errors.

## Models, codecs and HTTP

The model-only API retains `model-codec-unimplemented` obligations for selected
roots, so a nonempty model-only plan is not release-ready. An `Error` blocks
rendering; an `Annotation` retains metadata without inventing validation.

[`typescript::codecs`](SDK-TYPESCRIPT-CODECS.md) consumes the typed model
expressions and a checked owned program to implement exact decode/encode and
validation of mutated values. [`plan_codecs_with_views`](SDK-TYPESCRIPT-DIRECTIONAL.md)
supplies bounded request/response runtime views. HTTP/package planning reuses
these layers through the same CLI, session and editor profile.

## Documentation artifacts

`ModelPlan::render()` returns `OutFile` artifacts under `typescript/`, including
`models.ts`, `models.md`, `docs-manifest.json`, `typedoc.json`,
`tsconfig.docs.json` and `docs-readme.md`, plus shared exact-JSON support.

Native comments and Markdown record model/field names, wire keys, source
URI/pointers, views, descriptions and outstanding obligations. Source prose is
escaped before entering comments or rendered documentation; it cannot introduce
executable examples, TSDoc tags or active HTML.

The private [TypeDoc tool](../crates/suspect-codegen/tools/typescript-docs/README.md)
renders actual declarations and audits native HTML with parse5. It checks
planned symbol identity, original source bindings, descriptions, obligations,
links and anchors. Missing descriptions are reported rather than fabricated.
Tool versions and native evidence are recorded with the package/docs gates.

## Verification

`crates/suspect-codegen/tests/typescript_contract.rs` exercises OpenAPI → Contract
→ plan → artifacts → native consumers. Strict positive/negative consumers cover
presence/null, Boolean schemas, intersections, guarded recursion, exact numbers,
names and directional annotations. The tracked public OpenRouter case selects
`ORAnthropicNullableCaller` and `AnthropicImageBlockParam`, checking null and all
declared caller/image branches.

`typescript_docs.rs` adds hostile-prose and native symbol/source/link checks;
codec/directional/package suites verify the higher layers.

```sh
cargo test --locked -p suspect-codegen --test typescript_contract --test typescript_docs
# Native cases require the pinned TypeScript/Node/TypeDoc tools and tracked corpus.
cargo test --locked -p suspect-codegen --test typescript_contract --test typescript_docs \
  -- --include-ignored
```

Current profile boundaries are in [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md).
