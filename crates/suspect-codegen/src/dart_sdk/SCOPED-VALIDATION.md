# Scoped schema validation

This selection uses the checked `suspect.validation.experimental.v2` /
`oas31-jsonschema202012-static-applicators` profile. Packages whose closure needs
only v1 retain the original executor, resource policy and emitted representation.

The validator executes conditionals (`if`/`then`/`else`), `dependentRequired`,
`dependentSchemas`, `contains` with exact minimum/maximum counts,
`patternProperties`, pattern-aware `additionalProperties`, `propertyNames`,
`unevaluatedProperties` and `unevaluatedItems`.

Each child evaluation starts with fresh property/item annotation sets. Only
successful same-instance annotations propagate through refs and compositions.
`anyOf` collects every passing branch; `oneOf` collects only a sole passing branch;
`not` discards annotations. Passing `if` and `contains` annotations contribute
locally even if a selected branch or a separate contains count assertion fails.
An invalid complete schema exports no evaluated locations.

Every encode checks current native values, including mutable fields and extra
maps. Pattern-matched fields are preserved as exact `JsonValue` carriers when a
single static extra-field type cannot describe them. All matching patterns apply,
including patterns overlapping declared properties. The additional-properties
schema applies only to names excluded by neither declarations nor patterns.

Null is a present property for dependency checks. Optional typed members retain
`Absent()` versus `Present(null)`. Checked exact-JSON wrappers and their aliases
use `JsonNull` inside the wrapper consistently; a semantic nullable schema does
not imply a nullable Dart reference when its representation is such a carrier.
Conditional, tuple and unevaluated constraints are validated without coercion,
default insertion, field stripping or flattening branch scopes.

Schema entry, checks, declared collection/branch visits, NFA transitions and
annotation-set merge candidates spend the shared evaluation allowance. Duplicate
merge candidates cost a visit. Object visits and merges follow decoded
Unicode-scalar lexical order, without Unicode normalization. Temporary
property-name strings retain independent instance identity. Equality, numeric
bytes, recursion and depth failures cannot be inverted by a conditional or
negation, or hidden by a previous successful branch or a reporting cap.

Counts are nonnegative mathematical integers, preserving tokens such as `1.0`,
`-0.0` and huge symbolic exponents. The default contains minimum of one is not a
source numeric operand. These limits count logical work, not every byte of CPU
or heap allocation. Parsing, native conversion and HTTP/stream byte bounds apply
in addition to checked validation.

Declared media/parameter examples, referenced Example Objects, 3.2 `dataValue`
and schema examples are validated against the complete selected slot schema.
The shared `plan_protocol_examples_v2` helper has bounded discovery, synthesis
and validation, with original declared provenance and explicit findings for
invalid/unavailable values. Every candidate must pass its complete slot schema
before it becomes a native example; no conditional is rewritten or assumed. Run the
shipped `example/source_examples.dart` to exercise the emitted native codecs.

Only the exact witnessed static version/profile pairs are admitted. Dynamic
resource-scope programs need a separately implemented and verified executor.
