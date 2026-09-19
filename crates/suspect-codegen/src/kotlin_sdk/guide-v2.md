
## Scoped JSON Schema validation

This package's checked program uses
`suspect.validation.experimental.v2` /
`oas31-jsonschema202012-static-applicators`. It executes `if`/`then`/`else`,
`dependentRequired`, `dependentSchemas`, `contains` with exact min/max counts,
`patternProperties`, pattern-aware `additionalProperties`, `propertyNames`,
`unevaluatedProperties` and `unevaluatedItems`.

Every child starts a fresh evaluated-location scope. Successful references and
same-instance applicators contribute evaluated names/indices under the compiled
rules. Failed branches do not leak annotations. AnyOf unions all passing arms;
oneOf contributes only with exactly one passing arm; not discards its trial's
annotations. A passing if condition contributes locally even if then fails.
Contains retains its matched-index annotation independently of a failing count
assertion. Ordinary mismatches do not stop later checks; evaluation failures
propagate through every trial and cannot be inverted or suppressed.

Annotation merges, including duplicate candidate insertions, spend the shared
schema-work budget. Pattern transitions use that same allowance. Object visits
and annotation merges follow decoded Unicode-scalar lexical order without
normalizing keys. Exact numeric/count limits and the existing native conversion,
equality and arithmetic policies remain finite. The implicit contains minimum
does not manufacture a numeric source operand.

Known fields retain their native constructors and Presence wrappers. Objects
with pattern-dependent extras retain them in `additionalProperties` as exact
`JsonValue` entries, even when the source has `additionalProperties: false`.
The complete source codec applies overlapping patterns and additional/unevaluated
rules; the storage map is not permission to bypass those assertions.

When a faithful field projection is unavailable, an allocated schema-specific
data class exposes `value: JsonValue`. This checked JSON carrier retains the
complete value, including tuple positions and composition-dependent fields.
Constructors and data-class copies can hold caller-owned values; both codec
directions validate the whole source schema, and SDK calls use those codecs.
Model-only callers should explicitly use the source-bound codec. Successful
decode snapshots detach mutable generic JSON collections.

The scoped runtime is emitted only for a v2 program. Ordinary closures retain
the original v1 runtime and program representation. Dynamic/resource-scope
validation remains a separate, explicitly unimplemented native profile.
