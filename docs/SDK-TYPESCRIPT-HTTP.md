# TypeScript HTTP planning and runtime

`typescript::http::plan_http(Arc<Contract>, &[SourceId], HttpConfig)` returns an
immutable plan only after every selected operation fits its implemented
profile. `typescript::package::emit_http(&plan, &PackageConfig)` wraps its
artifacts in a private, installable ESM package with zero runtime dependencies.
These are experimental capabilities; complete SDK release gates remain open.

The canonical `codegen` CLI exposes this profile without a custom Rust integration:

```sh
cargo run --locked -p suspect-cli -- codegen \
  crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile typescript-http \
  --package-name @example/widgets --package-version 0.0.0 \
  --out sdk-out
```

Add `--check --format json` for a read-only CI report. Missing, modified,
obsolete or conflicting owned output fails the check. Invalid or ambiguous
selectors and unsupported selected contracts fail before writing any artifacts.
Omitting `--operation-id` selects all outgoing operations; unsupported ones
are reported rather than silently omitted. Success describes the selected
profile, not full-document validation. Run `suspect validate` separately to
detect defects outside the selected operation closures. Generated output lives
under `sdk-out/typescript/`; package building is explicit and no package manager
or publication command is invoked by `suspect codegen`.

The CLI-to-package native gate uses source fixtures directly, installs the
emitted tarball, compiles a strict TypeScript consumer, calls a localhost fixture
server, checks exact decimals, and builds/validates TypeDoc. The editor uses the
same profile for generation and the shared session command for watch/preview.
See [native call sites](SDK-M2-CALLSITES.md) and
[editor integration](SDK-EDITOR-PREVIEW.md).

## Admitted contract

The profile supports static HTTPS source servers, one HTTP bearer requirement,
required string path parameters with simple serialization, and non-null scalar
or scalar-array query parameters with `style=form`, either `explode` value and
`allowReserved=false`. JSON request bodies and exact-status JSON responses are
supported. GET, POST, PATCH, PUT and DELETE are admitted. HEAD, OPTIONS, TRACE,
security alternatives, server variables, other parameter encodings/media,
response ranges/defaults, headers and links produce source-located errors.
Multipart/form bodies and streaming/SSE remain required work.

Optional query values are omitted when absent; empty scalar strings remain
present. Optional empty arrays emit no pairs. Required empty arrays fail with
`request-representation` before transport because the form expansion would omit
the mandatory parameter. Nullable/object/nested-array/ambiguous union queries,
`allowReserved=true`, `allowEmptyValue=true`, and content parameters remain
explicitly unsupported. The tracked `/keys` operation named `list` has a nullable
offset and is not admitted by guessing a null encoding. Pagination is not inferred.

The HTTP planner plans the selected closure's model codecs through
`plan_codecs_with_views`. A closure without directional annotations plans one
neutral view; `http-manifest.json` records `directionPolicy` as
`neutral-equivalent` and model names stay neutral. A closure with boolean
`readOnly`/`writeOnly` annotations plans `Request` and `Response` views with
`directionPolicy: "oas31-required-applicability-v1"`: an explicit OpenAPI 3.1
context requiredness policy, not unmodified neutral validation and not OAS 3.0
normative behavior. Only required presence proven directional for the view is
relaxed; supplied read-only and write-only properties stay present and are
validated, nothing is stripped or defaulted, and unproven applicability is a
source-linked planning error rather than a silent fallback. Generated
descriptors bind directly to allocated model symbols and codec values. No
emitter parses schema text to discover another type system.

The tracked OpenRouter `getCredits`, `createKeys`, `updateKeys`,
`listContainerFiles` and `getContainerFile` operations pass generation,
installed native consumers and native docs. Container-file queries preserve
exact numeric values and RFC3986 bytes, including reserved characters and
Unicode. Source path/query names remain unchanged on the wire; native input
collisions and Object-prototype names are qualified by location.
This is five selected operations, not certification of all 103 public operations.
The tracked public corpus declares no `readOnly` or `writeOnly` annotations, so
these operations plan as `neutral-equivalent`; directional behavior is covered
by independent fixtures, and no directional corpus support is claimed.
Upstream invalid declarations and source examples remain visible; the key-update
response example still lacks required `external_user`.

## Generated interface and transport policy

Generated free functions and `createClient` accept the exact source-defined
credential keys. A spec declaring `apiKey` produces a credentials type keyed by
`apiKey`. Runtime validation requires the own credential property and RFC 6750
bearer syntax; errors do not echo token contents. The source server prefix,
including OpenRouter's `/api/v1`, is preserved.

Path serialization follows RFC 3986, including encoding `!'()*`. Dot segments,
malformed UTF-16 and values subject to unsafe URL normalization fail before a
request. Calls use injected fetch, explicit caller cancellation,
`redirect: 'error'`, `credentials: 'omit'` and no retries. Schema defaults do
not populate missing input; codecs retain exact decimal/integer semantics.

The default response cap is 8 MiB. A caller can lower the generated cap but
cannot raise it. Incremental reads enforce actual body bytes independently of
Content-Length, decode UTF-8 strictly, and cancel/release readers on failures
and abort races. Response media parameters are parsed strictly. A malformed or
schema-invalid declared error body is a decoding failure, not a valid API error.
Incomplete schema evaluation becomes an explicit resource/evaluation failure.

Success results use readonly `ApiResponse<Model, Status, Media>` wrappers with
mutable decoded body objects. Declared API errors freeze their body and expose
`ReadonlyResponseData<T>`, including nested arrays/objects while preserving
exact `JsonNumber` values. Operation-specific error guards bind identity through
private WeakMaps; mutating public metadata cannot forge another operation's
error. Every response branch preserves its actual model/status/media tuple.

## Package, docs and evidence

The package root exposes `createClient` and the `operations`, `models` and
`codecs` namespaces. `/operations`, `/models`, `/codecs` and `/json` subpaths
preserve module identities. Node 22.23.1, npm 10.9.8, TypeScript 5.9.3 and
TypeDoc 0.28.15 are pinned. Source Markdown and native TypeDoc entrypoints derive
from the same plan; `http-manifest.json` records exact selected source identities.

Native recording-server tests cover each declared status of these three
operations, two-origin redirect rejection, pre/header/body abort, stalled and
rejected streams, lying Content-Length, split UTF-8, truncation, caps, exact
request bytes, no retries and error mutation/forgery. Installed tarball consumers
make a real localhost credits request and compile/execute the generated README
helper while retaining exact decimals.

The TypeDoc gate checks parameter/Promise signatures, error guard predicates,
each exact response tuple, source prose and identity, and real HTML links and
anchors. Altered prose, missing exports/manifests, wrong Promise bindings,
swapped error bodies and wrong guard targets fail the gate. Hostile prose must
remain inert in rendered documentation.

The GLM continuation checkpoint passes 21 checks across the five HTTP/package
suites, with all native/corpus cases enabled: `target/sdk-glm-http-native-tests.log`.
Negative request tests count transport entry before assertions and require
request-side failure kinds, so transport assertion failures cannot masquerade
as request validation. Native docs resolve written model aliases through the
TypeScript checker before allowing TypeDoc's intrinsic alias erasure.
Earlier evidence remains in `target/typescript-http-patterns-node22-native-checkpoint.log`
and the separate integration/package logs. These functional checks do not
establish ideal bundle sizes or complete transport performance.
