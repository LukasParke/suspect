# Canonical Go model foundation

`go_models::plan_models(&Contract, roots)` allocates native names, source/wire
maps, model representations and documentation in one immutable plan. Rendering
produces a standalone `package sdk` module under `go/`, embedding the exact JSON
runtime. Every model-only root retains codec obligations; `release_ready()` is
false. The [Go codecs](SDK-GO-CODECS.md) and [HTTP profile](SDK-GO-HTTP.md) reuse
this plan in the SDK pipeline.

The bounded model profile has native structs, scalar literals, arrays/maps,
static references, recursive indirection and separately typed extra storage.
`Nullable`, `Optional` and `Presence` retain null/absence distinctions; required
fields become constructor arguments. Runtime namespaces and generated
constructors participate in name allocation. Extra setters reject declared wire
keys, and shallow extra-map snapshots cannot replace stored entries. Nested
maps/slices/pointers retain normal Go aliasing; source codecs validate their
current contents on each encode.

Model-only structs, scalar-enum types and presence wrappers explicitly refuse
`encoding/json` serialization. Source-codec output adds validated adapters for
defined model/literal types. Aliases retain
their underlying Go behavior; no ordinary `encoding/json` call is advertised as
source-schema validation. Scalar constraints and exclusive union membership are
codec obligations in the model-only plan. Native union interfaces have typed
variant wrappers and source-bound codec trials. Unsupported applicators and unproven intersections
remain located errors, rather than string or arbitrary-JSON substitutes.

Native consumers exercise construction, omission/null, exact numbers, recursion,
typed extras, enum constants, explicit serialization failure and go doc. A
tracked OpenRouter credits closure has native exact-number fields. The declared
syntax profile is Go 1.23; native consumers pass Go 1.23.12 and Go 1.27.
The separate codec/HTTP planners now pass native M2 and five-operation OpenRouter
consumer gates. Full integrated native/docs/release status remains in `SDK-PROGRESS.md`.

```sh
OPENROUTER_WEB_ROOT=/Users/luke/github/openrouter-web \
  cargo test --locked -p suspect-codegen --test go_models -- --include-ignored
```
