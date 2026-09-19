# Canonical Python model foundation

`python_models::plan_models(&Contract, roots)` builds a source-addressed immutable
symbol/dataclass plan. Its renderer emits `python/models.py`, `json_runtime.py`,
`__init__.py`, a manifest and a guide. This is a model-only foundation: every
selected root keeps a missing-codec obligation, and `release_ready()` is false.

The bounded profile preserves primitives, nullable scalars, arrays, simple
objects and canonical static references. Python dataclasses use keyword-only
construction. A required nullable field has no default; optional absence is the
distinct `UNSET` sentinel. General numbers use the exact `JsonNumber` runtime;
integers use Python `int` with finite source-codec conversion/resource policies.
Schema defaults are not applied. Names derive from source identities, not titles.

Object fields retain wire/native maps. Open objects have private extra storage,
an explicit collision-checking setter, and a read-only mapping view. Pattern
properties retain their admitted extras even when the residual additional policy
is closed. Constraints are not enforced by dataclass
construction; model codecs are required before HTTP or SDK promotion.

References are lowered from the canonical graph. Alias definitions are ordered
after their dependencies; dataclass annotations use postponed evaluation for
recursive fields. Source prose and pointers are encoded as inert Python text.
Package imports and standalone model-module imports are both supported.

The retained `PyDecl`/`PyType` plan also represents literal types, tagged/ordinary
unions, typed extras, false schemas as `typing.Never`, and nullable opaque object
maps. Scoped conditionals/intersections, nullable named objects, mixed tuple/item
domains and other constraints use faithful runtime-checked carriers where a native
field annotation cannot express the assertion. Resource/dynamic closures use
complete-root exact JSON carriers so conversion preserves the active binding
context. Ordinary static alias-only recursion and unsupported legacy resource
forms retain source-linked diagnostics.
Neutral directional annotations are retained; request/response projections are
not implemented for this backend.

Native tests exercise dataclass imports/constructors, annotation resolution,
omission/null, names, recursion, extra-key collisions, inert pydoc HTML and exact
credits fields from the tracked OpenRouter source. The syntax floor is
Python 3.11; focused interpreter results are recorded in `SDK-PROGRESS.md`.
The separate source-codec and HTTP planners now pass installed-wheel/native M2
and five-operation OpenRouter checks, including strict package/consumer mypy.
Model-only plans still retain obligations. Full integrated M3/release status is
recorded in `SDK-PROGRESS.md`.

```sh
OPENROUTER_WEB_ROOT=/Users/luke/github/openrouter-web \
  cargo test --locked -p suspect-codegen --test python_models -- --include-ignored
# Set SUSPECT_PYTHON_BIN to exercise a specific installed interpreter.
```
