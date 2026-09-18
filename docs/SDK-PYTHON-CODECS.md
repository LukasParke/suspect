# Source-bound Python codecs

`python_codecs::plan_codecs(Arc<Contract>, roots, CodecConfig)` builds native
models, compiles their source schemas and emits the exact JSON, validation and
conversion runtimes. It consumes retained `PyDecl`/`PyType` descriptors from
`python_models`; model source text is never parsed to recover types.

The existing entry point selects `compile_v3` only when the indexed effective
closure needs resource/dynamic semantics. Other closures use `compile_v2`, which
retains v1 programs for ordinary base schemas. `CodecPlan::validation_program()`
exposes the actual typed checked program for documentation and compatibility.

## Generated surface

```python
from generated_sdk import models, codecs

# User is illustrative; each exported name comes from the native model plan.
user: models.User = codecs.UserCodec.decode(b'{"id":"u1"}')
wire: str = codecs.UserCodec.encode(user)
```

`<Model>Codec` is a value of `ModelCodec[models.<Model>]`, emitted in
`model_codecs.py`. Its methods are `decode`, `decode_value`, `encode` and
`encode_value`. HTTP wheel packages expose that module as `codecs`.

The package includes `models.py`, `model_codecs.py`, `codec_runtime.py`,
`codec-plan.json`, `json_runtime.py`, `validation.py`, `validation_number.py` and
`validation_program.json`. The descriptor includes every field's native/wire
names, requiredness, native representation, source address and validation root.

## Behavior

- Decode parses exact JSON, validates the source schema, then constructs native
  values. Encode converts the native model and validates again, including mutations.
- Python `int` excludes `bool`; decimal/general numeric values use exact
  `JsonNumber`. Native integer materialization and parsing have finite limits.
- Keyword-only dataclasses distinguish absent `UNSET` from present `None` and
  present values. Required fields have no invented default. Fixed required literal
  tags can be initialized from their source constant.
- Literals, ordinary/tagged unions, recursive static references and typed extra
  storage are supported within the model planner's admitted profile. `oneOf`
  validation rejects overlap even when a native union could hold the value.
- Branch trials share validation/conversion budgets. Resource exhaustion is an
  evaluation failure, never evidence that a branch is invalid. Conversion tracks
  depth/work and detects native object cycles.
- `CodecError` identifies invalid-schema values, native conversion failures or
  resource exhaustion and retains source/instance context. Exact JSON parsing has
  its own typed syntax/resource errors.

Model-only planning retains codec obligations. Codec rendering records
`source_codecs: true` and `model_only: false` in the model manifest. Complete SDK
release acceptance is recorded separately.

## Boundaries and verification

Scoped assertions use the native types that can faithfully carry their values.
Pattern extras remain exact JSON values even with `additionalProperties: false`;
every matching pattern and residual-value assertion still runs. Conditional and
intersection shapes, mixed tuple/contains arrays and nullable objects may use
runtime-checked JSON carriers. No annotation set is replaced by property-list
flattening, and no extra value is dropped.

Resource/dynamic closures use complete-root JSON carriers, with local scalar,
list or dictionary annotations where proved. Nested conversion does not restart
dynamic validation in a detached child context. Both encode and decode check the
complete root with its actual resource stack. Model-only APIs retain codec
obligations. Null-only aliases keep their `TYPE_CHECKING` spelling and runtime
`NoneType` identity; null annotations use `None`.

Alias-only cycles in the ordinary static native representation, unsupported
dialects/vocabularies, legacy recursive references and Python HTTP contextual
readOnly/writeOnly projection remain located refusals. Every codec root requires
successful owned-schema compilation under its selected profile.

Native tests cover exact values, absence/null, literals/tags, recursive models,
typed extras, mutable encode validation, exhausted budgets, cycles and actual
OpenRouter credits. HTTP tests additionally build/install the wheel and run strict
mypy over the generated package and native consumers.

```sh
OPENROUTER_WEB_ROOT=/Users/luke/github/openrouter-web \
  cargo test --locked -p suspect-codegen --test python_codecs -- --include-ignored
```

`SUSPECT_PYTHON_BIN` selects the interpreter. The declared floor is Python 3.11;
current integrated evidence and remaining gates are in [progress](SDK-PROGRESS.md).

New scoped and resource evidence is recorded in
`target/sdk-python-scoped-completion-01/` and
[Python resource adoption](SDK-PYTHON-RESOURCES.md). These additive gates reuse the
completed ordinary HTTP/wire matrices.
