# Contract resources and dynamic-reference metadata

2026-09-10 — implemented resource-indexing handoff. Canonical OAS 3.2 `$self`,
2020-12 `$id`, resource-local anchors, direct targets and dynamic-reference
metadata are available through the public Contract seam. Dynamic execution is
the owned compiler/runtime's next integration step.
This supersedes the Contract-level `$self`/`$id` limitations in
`SDK-OAS32-CONTRACT.md`; owned/native capability admission remains separate.

This tranche preserves the core fixes recorded in
`target/sdk-core-review-20260910/fixes-v3/DISPOSITION.md`, including effective
3.0 reference-only closure, context-aware HTTP reuse, schema-position ancestry,
dialect declaration origins, and effective retrieval identities after redirects.
A pre-edit snapshot is retained under
`target/sdk-contract-resources-20260910/before/`.

## Stable API for the owned schema compiler

The following immutable API is implemented. `ResourceId` is an
alias of `SourceId`: a physical source boundary, independently of its URI names.

```rust
Contract::resources() -> impl Iterator<Item = &Resource>
Contract::resource(id: &ResourceId) -> Option<&Resource>
Contract::resource_scope(source: &SourceId) -> Option<&ResourceScope>
Contract::resolve_resource_reference(source: &SourceId, reference: &str)
    -> Result<SourceId, ResourceResolutionError>
Contract::reference_target(reference_object: &SourceId) -> Option<&SourceId>
Contract::dynamic_reference(schema: &SchemaId) -> Option<&DynamicReference>
```

- `Resource`: `source()`, `kind()`, `canonical_uri()`, `base_uri()`,
  `declaration_source()`, `aliases()`, `anchors()`.
- `ResourceKind`: `Document`, `OpenApiDocument`, `Schema`.
- `ResourceScope`: `resource()`, `schema_root()`, `base_uri()`, `base_source()`,
  `address()`. `schema_root()` distinguishes an embedded schema root without
  `$id` from its OpenAPI/HTTP document's URI and fragment-resolution boundary.
- `SchemaAnchor`: `name()`, `source()` (the keyword), `target()` (the schema),
  `resource()`, `kind()`; `AnchorKind::{Static, Dynamic}`.
- `DynamicReference`: `source()` (the `$dynamicRef` keyword),
  `initial_target()`, `initial_resource()`, `dynamic_anchor()`, `candidates()`.
- Existing `schema_contexts`, dialect/default/version declaration origins,
  `is_schema_position`, and static reference-object provenance remain available.

`resource_scope` returns `None` for malformed, unsupported or conflicting
scopes, with located Contract diagnostics. URI lookup is fallible and reports
duplicates rather than selecting the first declaration.
For an unregistered descendant such as a keyword value, `resource_scope` returns
the nearest registered ancestor's scope; its `address()` belongs to that
registered object/schema. Schema IDs returned by Contract have their own exact
registered addresses.

URI target identity and schema admission are separate. A physical retrieval
URI/JSON Pointer can identify a source value whose schema dialect is unsupported
or malformed. Such target nodes and their dialect origins remain indexed for
located admission errors. Unknown-dialect raw reference traversal uses an already
known enclosing base only when it does not cross an unimplemented `$id`;
canonical identifiers/anchors of unknown vocabularies are not guessed.

### Dynamic semantics contract

The initial target is the result of ordinary URI-reference resolution.
`dynamic_anchor()` is set **only** when the reference uses a plain-name fragment
whose initial resolution is a matching `$dynamicAnchor`. A JSON Pointer, an
empty fragment, or an ordinary `$anchor` remains a static fallback, including
when its target happens to carry another dynamic anchor.

`candidates()` retains matching dynamic declarations by source resource. It is
metadata for the owned compiler to combine with the evaluation's resource
stack, not a preselected runtime target. Runtime evaluation must use the
outermost matching resource in dynamic scope, retaining the initial target
when no overriding resource applies. Static `$ref` never performs this override.
Dynamic references remain distinguishable from static references throughout
the Contract graph; their initial target must not be lowered as an unconditional
static reference by a consumer that does not support dynamic scope.

`Schema::references()` includes a distinct `$dynamicRef` edge for graph planning;
its target is the initial fallback. `Contract::reference_target()` continues to
expose only the ordinary `$ref` edge, including when both keywords are present.
`effective_schema_closure()` adds candidate binding schemas only from resources
encountered by the selected closure, iterating to a fixed point. This is a finite
planning superset rather than a runtime binding decision. Candidate schemas and
their dependencies are materialized under their original source/context even
when the resource was entered through a different nested schema.

## Identity and loading boundary

- `SourceId` always keeps the effective physical retrieval URI and decoded JSON
  Pointer. `$self`, `$id`, requested redirect aliases and cache paths do not
  overwrite that source identity.
- OAS 3.2 `$self` selects a document's logical identifier and reference base.
  Modern schema `$id` establishes a schema resource and resolves relative to
  the enclosing resource/document base. Resource-root pointers and resource-local
  anchors resolve back to original physical sources.
- Retrieval aliases remain usable and distinct from canonical identifiers.
  Multiple physical resources claiming one logical URI are located errors,
  including identical bytes hosted under different retrieval identities.
- `$id` accepts an absent or empty fragment, normalizing the empty fragment away;
  a nonempty fragment is invalid. `$self` retains its full identifier, while its
  reference base is fragment-free. Both its canonical name and base are accepted
  aliases. Duplicate base/alias claims are errors as well as duplicate canonical
  identifiers. Anchors are unique within each resource; the same name in distinct
  resources is valid. Reusing a name through `$anchor` and `$dynamicAnchor` in one
  resource is diagnosed rather than assigning unspecified precedence.
- URI registration grants no acquisition authority. Resource discovery uses
  authorized, already supplied/loaded document bytes; ordinary dependency reads
  continue through Workspace's existing provider and allowlist. There is no
  network or filesystem fallback from a closed provider and no allowlist mutation.
- URI logic is exposed as `suspect_ref::resource_uri` and re-exported as
  `suspect_ir::contract::resource_uri`, using the audited strict RFC 3986 algorithm in
  `suspect-schema/src/resources/uri.rs`. The schema owner can re-export that
  lower-layer helper to consolidate the borrowed compiler's implementation
  without an IR↔schema dependency cycle or a new direct schema→ref dependency.
  Schema files were not edited by this owner.

### Catalogue and cache behavior

`Workspace::available_document_uris()` enumerates already loaded or supplied
retrieval names after the existing requested/effective allowlist checks. It
performs no I/O. Contract uses these names for registration-only catalogue
discovery when resolving a logical name outside that retrieval namespace.
Every matching declaration is considered; an already registered name cannot
silently defeat another supplied declaration due to discovery order.

Catalogue parsing uses the lossless workspace sidecar and may populate its
existing parsed-document cache. It follows no candidate references and promotes
only matching documents into Contract's retained closure. The selected reader
then materializes promoted values; unsupported Fast syntax still fails explicitly.
Unselected malformed documents do not invalidate the entry or expand its semantic
reference closure. Catalogue cache keys retain object kind, feature version,
dialect and the authorized document-name set. They are independent of reference
evaluation and do not change Workspace's generic reference memoization.

An ordinary authorized retrieval name loads its own pinned bytes before a
same-named `$id` from another source can be trusted. Closed-provider misses never
read an existing filesystem file or invoke acquisition. For local workspaces
without a provider, ordinary unresolved file dependencies retain the existing
Workspace loading policy. Arbitrary filesystem directories are not scanned for
identifiers. Nonstandard embedding formats need a known structural root or a
typed reference mount before nested schema declarations can be discovered.

### Deliberate limits

- Runtime dynamic binding, evaluation and native adapter execution remain with
  the schema/native owners. The contract performs no instance-time selection.
- New/custom dialects, `$vocabulary`, and legacy `$recursiveRef`/
  `$recursiveAnchor` retain explicit unsupported diagnostics. Core 2020-12 and
  the OAS 3.1 base dialect used by OAS 3.2 have concrete resource semantics.
- Retrieval aliases and document-root pointers crossing `$id` boundaries are
  accepted compatibility addresses. Portable descriptions should use the nearest
  canonical `$id`, as advised by Core §9.2.1 and OAS Appendix F.
- URI resolution uses the generic RFC algorithm and scheme/host case folding;
  encoded dots/slashes, path case, user information and query spelling are
  preserved. URL-parser repairs and optional normalization are not used to merge
  logical resource identities.

## Verification and remaining schema-owner integration

Evidence is preserved under `target/sdk-contract-resources-20260910/`:

| Gate | Result | Evidence |
| --- | --- | --- |
| New `contract_resources.rs` | **18 passed**, both readers; includes YAML/JSON and provider fixtures | `ir-after-context-fix.log` |
| Full IR | **94 passed**, 5 existing ignores | `ir-after-context-fix.log` |
| Existing owned dialect/context witnesses | **29 passed** | `context-after-fix.log` |
| Full schema | **190 passed**, 1 failed, 5 existing ignores | `schema-after-context-fix.log` |
| Shared URI helper | **3 passed**, including all 42 RFC 3986 §5.4 normal/abnormal vectors | `uri-helper-tests.log` |
| All-target IR/ref Clippy, warnings denied | **passed** | `clippy-final.log` |

The full schema failure is the retired blanket-resource refusal witness at
`crates/suspect-schema/tests/owned/contract.rs:293–325`. It expects both an
`Unsupported` error at the containing resource and a Contract
`unsupported-schema-resource` diagnostic. Known `$id` is now implemented in
Contract; owned admission currently emits its own located refusal at `$id`.
The schema owner needs to update that admission/test as it consumes this API.
The remaining schema tests, including v1 byte/budget compatibility, v2 static
applicator conformance, numeric/source fidelity and HTTP-fragment context
witnesses passed. No SDK native reruns were performed in this tranche.

The initial full run exposed an overly strict target-scope guard that hid
unsupported-dialect target nodes and their original declarations. It was fixed
and the original context witnesses passed without changing their expectations.
Both the first full output (`full-ir-schema-initial.log`) and subsequent evidence
remain intact. Two older IR `$self`/`$id` refusal tests were updated to assert real
canonical scope, reference targets and preserved physical sources.

Primary commands:

```sh
cargo test --locked --offline -p suspect-ir --target-dir target/sdk-contract-resources-build --no-fail-fast
cargo test --locked --offline -p suspect-schema --target-dir target/sdk-contract-resources-build --no-fail-fast
cargo test --locked --offline -p suspect-ref --target-dir target/sdk-contract-resources-build --lib resource_uri::tests
cargo clippy --locked --offline -p suspect-ir -p suspect-ref --all-targets --target-dir target/sdk-contract-resources-build -- -D warnings
```

Primary rules: [JSON Schema 2020-12 Core](https://json-schema.org/draft/2020-12/json-schema-core)
§§4.3.5, 7.1, 8.1.1, 8.2.1–8.2.3, 9.1–9.3;
[OAS 3.2](https://spec.openapis.org/oas/v3.2.0.html) OpenAPI Object `$self`, Schema
Object dialect rules and Appendix F; [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
§§5.2, 5.4 and 6.2.2.1.
