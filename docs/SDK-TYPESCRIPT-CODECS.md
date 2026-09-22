# Generated TypeScript model codecs

`suspect_codegen::typescript::codecs::plan_codecs` now connects the canonical
model plan to exact JSON parsing, owned schema validation and typed conversion.
The API accepts one `Arc<Contract>`, selected schema roots and `CodecConfig`.
It returns an immutable `CodecPlan` only when both representation planning and
schema compilation succeed. It does not reinterpret rendered TypeScript or
reparse raw OpenAPI in a second backend.

`plan_codecs` selects **neutral OAS 3.1 / JSON Schema 2020-12 model closures
admitted by the owned validator**. `plan_codecs_with_views` also supports
[bounded request/response views](SDK-TYPESCRIPT-DIRECTIONAL.md). The
[`typescript-http` profile](SDK-TYPESCRIPT-HTTP.md) integrates these codecs
through CLI, sessions and editor generation. Model-only `plan_models` rendering
retains its own missing-codec status; complete dialect and SDK release gates
remain open.

The additive [v2 scoped-validator adoption](SDK-TYPESCRIPT-VALIDATION-V2.md)
implements conditional/dependency/contains/pattern/property-name/unevaluated
instructions. Native admission uses `compile_v2`, preserving v1 programs for
ordinary base closures. Pattern-matched extras and heterogeneous prefix items
use checked JSON-value carriers; decode and mutation-time encode enforce all
applicable source constraints. Typed compatibility records retain the actual
checked per-view program and conversion limits via `CodecPlan::interfaces()`.

[V3 resources and dynamic references](SDK-TYPESCRIPT-VALIDATION-V3.md) are selected
with explicit `compile_v3` only when required by the effective source closure.
Dynamic positions retain checked `JsonValue` carriers. V3 conversion traces also
retain exact resource context, so a failed condition under different bindings
cannot overwrite a passing projection. Base v1/v2 programs and trace calls retain
their established behavior.

## Generated API

Every public model symbol, including referenced targets, has a codec export in
`model-codecs.ts` and is selected as a root of the compiled validation program.
The public generic interface is `Codec<T>`. Runtime support types and located
`ModelCodecError` failures are exported from the same module.

```ts
import { ORAnthropicNullableCallerCodec } from './model-codecs.js';

const caller = ORAnthropicNullableCallerCodec.decode('{"type":"direct"}');
const text = ORAnthropicNullableCallerCodec.encode(caller);
const absentCaller = ORAnthropicNullableCallerCodec.decode('null');
```

Decode parses once, validates the root once and then converts through the typed
model expression plan. Only completed schema traces from a valid root may
select schema alternatives. Inline type/literal alternatives use representation
checks, without invented schema identities. If multiple validated alternatives
can represent the instance, deterministic model-plan order selects the first
convertible one. Conversion or evaluation exhaustion is never suppressed by
trying another branch.

Intersections merge typed field/item/map projections recursively. Generic
passthrough values are neutral in that merge. Every permitted extra property is
retained; missing fields and null stay distinct. The planner rejects overlapping
numeric representations it cannot reconcile, before returning artifacts.

Named fields may coexist with typed additional properties only when a bounded
representation proof shows their decoded values fit the native index-signature
type. Extra-property schema constraints remain separate from named-field
constraints. The proof handles scalar/literal, union/intersection, array,
object and reference representations conservatively, including optional map
fields and TypeScript weak-object rules. Unproven compatibility is a located
error. This establishes decoded representation compatibility, not complete
soundness after arbitrary mutation through TypeScript aliases; encode validation
remains authoritative for the resulting wire value.

Encode writes the exact JSON representation, reparses its wire value, then
validates once. Enumerable string-named object data properties with undefined
are omission candidates; requiredness is checked against the resulting object.
Null is preserved, undefined array elements fail, getters are never invoked,
and omitted slots still consume the JSON work allowance. Typed callers with
`exactOptionalPropertyTypes` must omit optional fields instead of assigning
undefined; the runtime policy also handles JavaScript callers.

Native bounded integers use `number`, unbounded integers use `bigint`, and
general numbers retain `JsonNumber`. Independent JSON, validation and conversion
budgets prevent incomplete work from masquerading as schema success. These are
visit/depth/representation limits, not a total allocation or byte-work bound.
Generated programs and JavaScript builtins are trusted application code.

## Verification

Six public planner checks include native compilation/execution of generated
codecs for recursive models, exact decimal/integer literals, safe integer
bounds, presence/null, refined unions and disjoint intersections. Actual tracked
OpenRouter nullable caller, image-source discrimination and chat-choice index
pass typed encode/decode consumers. A separate tracked-source check confirms
the invalid `upscale_factor/exclusiveMinimum` declaration still prevents codec
artifacts and reports its original source span.

Five independent native runtime fixtures cover conversion behavior, one-call
validation/branch traces, nested merge rules, limits and actual tracked
OpenRouter caller/index inputs. These explicit conversion fixtures complement
the generated planner tests; they do not substitute for them.

The combined model/codec/intersection/JSON run passes 32 checks, including all
required tracked OpenRouter inputs:
`target/typescript-model-codecs-integrated-tests.log`. The source validator and
generated schema validator retain separate normative/adversarial conformance
gates. Complete workspace checks and native package/docs gates are recorded at
their own checkpoints in [SDK-PROGRESS.md](SDK-PROGRESS.md).

Additional-property admission and portable pattern validation now pass the
actual tracked `ChatRequest` and `ChatResult` codec consumers, including
`ImageGenerationServerToolConfig`, `RouterParams`, `SubagentNestedTool` and
`TraceConfig`. Independent review's 29 valid-wire witnesses agree with admission
expectations; all 12 admitted outputs compile natively and five additionally
round-trip through generated codecs. Unsupported or unproven representation
combinations still fail planning. The combined pinned Node 22 checkpoint passes
78 checks across 13 TypeScript suites, including native codecs, HTTP, packages
and docs: `target/typescript-http-patterns-node22-native-checkpoint.log`.

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
  cargo test --locked -p suspect-codegen \
  --test typescript_codecs --test typescript_codecs_runtime \
  -- --include-ignored
```

Native documentation must resolve the actual generic codec interface, its
documented encode/decode methods, and each codec's model/source identity. A
TypeDoc alias that merely renders a name without its signatures does not pass.
Codec artifact manifests keep `releaseReady: false` until the complete SDK
gates are fulfilled. Functional checks alone do not measure full generated-SDK
bundle and transport performance.

## Reachable programs and initialization

Generated validation nodes and conversion expressions are shared immutable
constants. Each codec references only its reachable compiled closure, retaining
original indices, source identities, recursion and branch traces. The public
codec module stays a barrel with the same typed exports. Bundlers can discard
unreferenced roots and expressions without recompiling source schemas.

Root validators and codec conversion tables initialize lazily and memoize their
first use. Ordinary unbundled imports still construct shared static literals
once; they do not eagerly build every root's validator caches. Sparse tables
currently allocate through their highest reachable compiler index. Eliminating
that remaining index-dependent overhead requires separate measurement and work.

With pinned Node 22.23.1/esbuild 0.28.2, the installed three-operation OpenRouter
fixture's selected `BadRequestResponseCodec` bundle fell from 155,256 to 35,689
minified bytes (14,252 to 9,704 gzip). The fixture's getCredits bundle fell from
165,608 to 69,397 bytes (17,688 to 14,321 gzip). Importing all three operations
measured 174,511 bytes / 19,275 gzip at that checkpoint; an equivalent before measurement
was not retained, so no all-operation reduction is claimed. Source node sharing
avoids duplicating node bodies; per-root selection metadata still adds bytes.
Evidence: `target/typescript-http-bundle-report.json`. Timings are observational
local samples without a regression threshold or complete SDK performance claim.

## Installable ESM package

`typescript::package::emit(&plan, &PackageConfig { name, version })` adds a
private npm package around the admitted codec artifacts. Npm identity and exact
version are validated; the emitted lock retains the reviewed TypeScript 5.9.3
registry integrity. Builds use pinned Node 22.23.1/npm 10.9.8, ES2022 and
NodeNext. There are no runtime dependencies or implicit installation/publication
hooks. `models` and `codecs` namespaces at the root, plus `/models`, `/codecs` and
`/json` subpaths, keep every model addressable when source names collide with
support APIs. The codec subpath exposes the generic `Codec<T>` interface; the
package root also exports the runtime `ModelCodec<T>` type.

Both native package fixtures build, locally pack and install real tarballs into
isolated JavaScript and strict TypeScript consumers. They cover hostile symbol
collisions and actual tracked OpenRouter caller/image models, execute the
compiler-checked README helper, verify packed source/docs bytes and source-map
targets, and reject access to unexported runtime subpaths. Npm deliberately omits
`package-lock.json` from tarballs: retain the generated source directory and its
lock for reproducible builds. Native HTML generation is a separate verified
gate, not an effect of package emission.
