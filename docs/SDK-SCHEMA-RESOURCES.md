# Owned resources and dynamic references: v3 contract

Implemented and verified resource-integration tranche, 2026-09-10. V1 and the
published v2 applicator contract remain frozen. The full schema suite passes
206 tests; v3 includes all 44 official dynamicRef cases and the four formerly
deferred dynamic/unevaluated cases. This is not full dialect conformance.

## Admission and wire migration

`OwnedCompiler::compile_v3` explicitly selects canonical-resource/dynamic
admission and a v3 program, including v2's evaluated-applicator behavior.
`compile` and `compile_v2` retain their previous resource/dynamic refusals and
unchanged program semantics. V3 uses:

- version `suspect.validation.experimental.v3`
- profile `oas31-jsonschema202012-resources-dynamic`

The associated `OwnedProgram::V3_VERSION` and `V3_PROFILE` constants expose this
pair. `ProgramInstruction::requires_v3()` identifies DynamicRef; the published
`requires_v2()` classifier retains exactly its nine v2 operations. Existing v1/v2
consumers must continue checking their supported envelope before emission.

`OwnedProgram` adds optional `resourceContext`, omitted entirely for v1/v2.
Existing node/check/source fields and v2 op operands are unchanged. New public
metadata structs are `ProgramResourceContext` and `ProgramResource`; they are
defined/re-exported by the owned module for Main's public crate re-export.

```text
resourceContext = {
  resources: [{
    source: ProgramSource,                 # physical resource boundary
    kind: "document" | "openApiDocument" | "schema",
    canonicalUri: String,
    baseUri: String,                       # fragment-free
    aliases: [String],
    declarationSource: ProgramSource | null,
    dynamicAnchors: [[name, keywordSource, targetNodeIndex], ...]
  }, ...],
  nodeScopes: [[resourceIndex, schemaRootSource, canonicalAddress], ...]
}
```

nodeScopes is aligned one-to-one with nodes. Resource source identity never
becomes its canonical URI. Canonical names, aliases, schema roots, addresses and
candidate bindings are copied from Contract's indexed metadata, not guessed by
the evaluator. Only selected/encounterable binding targets are emitted.

One new named instruction is `ProgramInstruction::DynamicRef`, serialized as:

```text
{ op: "dynamicRef", target: initialTargetNodeIndex,
  initialResource: resourceIndex, anchor: String | null }
```

Source is the original `$dynamicRef` keyword. `anchor` is copied from
`Contract::dynamic_reference`, and is present only when the initial URI used a
plain-name fragment resolving to a dynamic anchor. Empty fragments, pointers,
and ordinary anchors retain a null anchor and use the initial target directly.
The evaluator never converts static `$ref` into dynamic lookup.

## Dynamic evaluation

Entering a schema enters the resource given by its indexed node scope, including
when validation starts at a nested schema and the physical resource root was
not itself evaluated. Only resources actually entered on the current evaluation
path participate; unentered catalogue resources do not override anything.

For dynamicRef with a dynamic anchor name, search entered resources outermost
first and select the first matching indexed dynamic binding. If none matches,
retain the initial target. Resolve neither URIs nor source schemas at runtime.
Repeated entries of the same resource cannot change its immutable bindings, so
the runtime may keep the ordered first distinct active resources. Restore scope
on every return, including invalidity and evaluation failure.

Cycle identity includes node, instance identity and the ordered resource-context
identity: revisiting a node under genuinely different dynamic bindings cannot
be rejected as the old context's cycle. Context interning retains exact identity,
not a collision-prone hash-only approximation. Depth/work failures and recursive
nonprogress remain explicit, noninvertible EvaluationFailure outcomes.

DynamicRef evaluates its selected target in a fresh annotation scope and
propagates only its successful evaluated-location sets, exactly like v2 Ref.
Conditional, branch, pattern and unevaluated semantics are unchanged from v2.
Trials share limits/caches and cannot leak entered resources into sibling trials.

## Resource work and checked admission

V3 preserves v2 instruction/annotation charging and adds one visit when a new
distinct resource enters dynamic scope. Dynamic lookup charges each inspected
resource and each candidate binding inspected, stopping at the first match.
Scope restoration and exact context-cache lookup do not add visits. Resource
metadata does not authorize acquisition or source loading.

The portable guard requires the v3 version/profile for resourceContext or a
DynamicRef instruction. It checks node/resource alignment, physical containment,
canonical/base/alias URI syntax and uniqueness, original declaration/anchor
locations, finite candidate targets and initial-target/resource consistency.
It cannot attest a publicly mutated program against absent original documents;
the compiler additionally checks against the immutable Contract registry.

Bindings are sorted by decoded anchor name; resource records are sorted by
physical SourceId. Node scopes follow node index order. A resource enters scope
when its node is entered, before that node's assertions, even when entry begins
below the resource root. The initial target's resource is **not** prematurely
entered before an override lookup. Declaration candidates from unentered
resources remain inert. Successful and failing returns restore scope; the
interned context cache may be retained across trials, but current scope cannot.

## Verified evidence and handoff fixtures

- `crates/suspect-schema/tests/owned_resources.rs`: 15 ordinary tests plus the
  explicitly run frozen-checkpoint test. Covers canonical/self/nested IDs,
  physical/retrieval/logical aliases, escaped pointers, outermost precedence,
  unentered candidates, fallback modes, detached/parent-root bindings, restored
  trial scopes, changed-context revisits, ambiguity, resource mutation admission,
  exact numeric failures, recursion and default depth limits.
- `crates/suspect-schema/tests/fixtures/resource-conformance/`: unmodified pinned
  official dynamicRef file and three remote documents. All **44 cases execute**
  through a closed supplied-document provider; no runtime acquisition. Original
  URLs, SHA-256 values and license are recorded in its README.
- `target/sdk-schema-resources-executable-v3.json`: **44 actual checked v3
  programs**, `rootTarget` node indices, exact instanceJson strings and original
  Valid/Invalid expectations, ready for cross-native runtime adoption.
- The four previously deferred dynamic/unevaluated cases now execute under v3.
  Frozen compile_v2 still refuses resource semantics, so its existing capability
  deferrals and fixtures remain unchanged.
- The retained **32 published v2 programs** were recompiled from their original
  physical source identities and compared byte-for-byte in canonical JSON form.
  The optional resourceContext field does not appear in old envelopes.

The deep distinct-resource chain initially reproduced a normal-stack overflow.
Internal failure values are now boxed to avoid reserving large URI/source errors
in each recursive debug frame. Outcomes/sources and visit charges are unchanged;
the configured **512 depth ceiling** fails explicitly on a **2 MiB stack**, with
no lowered limits. Red and green evidence remain in
`sdk-schema-resources-verified-01.log` and `sdk-schema-resources-depth-02.log`.

Final checks:

| Gate | Result | Evidence |
| --- | --- | --- |
| Full schema unit/integration/doc suite | **206 passed, 0 failed, 6 ignored** | `target/sdk-schema-resources-full-schema-01.log` |
| Official dynamic fixture export | **44 cases passed** | `target/sdk-schema-resources-fixture-export.log` |
| All-target schema Clippy, warnings denied | **passed** | `target/sdk-schema-resources-clippy-03.log` |
| Frozen 32-case v2 program bytes | **passed** | `target/sdk-schema-resources-verified-01.log` (before the independent depth stress aborted) |
| Retained strict RFC URI/helper and source URI tests | **passed in full suite** | same full-schema log |

The six full-suite ignores are the five existing corpus-dependent acceptance
tests and the separately run retained-checkpoint test. No v1/v2 native matrix was
rerun. Main's reported suspicious-assignment and conditional Clippy findings are
resolved, including the custom macro body that rustfmt does not reformat.

The stale owned resource refusal witness now checks the precise frozen-v1 `$id`
source/span and asserts that Contract's canonical resource is indexed. The old
blanket Contract diagnostic is not recreated.

Preserved hashes:

- native runtime-contract-v1.json:
  `49fd80574c704a397d3f513ab886ecfdd90aa93b6f67c1af938935cf99a3a646`
- published 32-case v2 source fixture:
  `e7ee18bca889072aedd75559b57105dfffbc6c88578bdc437417d4a8cd030967`
- published 32-case v2 executable fixture:
  `0dcae95d213030abd6fc14ebca8d3f4bf6ce9f5bd5382730bfa403f91be53056`

## Remaining boundaries

Custom vocabularies/dialects, legacy `$recursiveRef`/`$recursiveAnchor`, the
existing portable regex limitations, assertion-mode formats, and unsupported
OAS 3.0 directional requirements remain explicit refusals. Native v3 execution
is the next adoption gate; this owner has changed no IR or native files. Main
must add ProgramResource and ProgramResourceContext to the public crate's
existing owned re-export list; their definitions and owned exports have landed.

## Primary rules

[JSON Schema 2020-12 Core](https://json-schema.org/draft/2020-12/json-schema-core)
§§4.3.5, 7.1, 8.2.1–8.2.3 and 9; [OAS 3.2 OpenAPI Object](https://spec.openapis.org/oas/v3.2.0.html#openapi-object)
and Appendix F; [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) §§5.2 and 5.4.
The lower shared resource URI helper is reused through the existing schema URI
seam, retaining its independent tests. No IR/native/acquisition changes belong
to this tranche.
