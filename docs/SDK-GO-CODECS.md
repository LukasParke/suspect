# Source-bound Go codecs

`go_codecs::plan_codecs(Arc<Contract>, roots, CodecConfig)` returns an immutable
`CodecPlan` with `models()` and `render()`. The plan consumes retained
`GoDescriptors`, compiles source assertions with `OwnedCompiler`, and emits native
models, a typed codec registry, descriptor metadata and exact runtimes.

```go
user, err := sdk.Codecs.User.Decode(data)
if err != nil {
    return err
}
wire, err := sdk.Codecs.User.Encode(user)
```

`User` is illustrative; registry entries use the allocated native model names.
Each entry is a `Codec[T]` with `Decode`, `DecodeValue`, `Encode` and `EncodeValue`.
Generated descriptors and a native `reflect.Type` registry drive conversion.
Reflection operates on those retained native plans, never on parsed code text.

## Semantics

- Exact JSON validation precedes native construction; native conversion is
  followed by validation during encoding. Mutable invalid models fail explicitly.
- Exact `Number`/`Integer` tokens and proven native integer ranges avoid floating
  point conversion. Mathematical integer checks preserve decimal/exponent meaning.
- `Nullable[T]`, `Optional[T]` and `Presence[T]` retain required-null,
  absent/present and absent/null/value states. Contradictory wrapper states fail
  conversion rather than discarding a payload or a null flag.
- Objects, typed extras, arrays/maps, source-known literal tags, native union
  wrappers and recursive references follow actual model descriptors. `oneOf`
  exclusivity remains a source validation rule.
- Defined model/literal `encoding/json` adapters delegate to source codecs.
  Standalone presence wrappers still require their source model context; aliases
  retain underlying Go behavior and need an explicit codec for schema validation.
- Conversion/validation share finite per-call work/depth/equality budgets.
  Incomplete evaluation is a resource failure, distinct from an invalid value.
  `CodecError` retains source and instance paths and supports error unwrapping.

The model manifest records `sourceCodecs: true` for codec output and removes the
discharged model-only codec obligations. It does not certify an entire SDK release.

## Native verification

`tests/go_codecs.rs` exercises the representative M2 models through native Go:
exact numbers, presence, tagged/recursive values, JSON adapters, mutated models,
extra collisions and inconsistent wrappers. The HTTP and actual OpenRouter gates
exercise the codecs from independent module consumers. Go 1.23 is the declared
language floor; pinned native runs use Go 1.23.12 and the current toolchain.

```sh
SUSPECT_GO_TOOLCHAIN=go1.23.12 \
  cargo test --locked -p suspect-codegen --test go_codecs -- --include-ignored
```

General unproved intersections, unsupported applicators/representations and HTTP
directional projections remain explicit diagnostics. See [model planning](SDK-GO-MODEL-PLAN.md),
[HTTP](SDK-GO-HTTP.md), and [current acceptance](SDK-PROGRESS.md).
