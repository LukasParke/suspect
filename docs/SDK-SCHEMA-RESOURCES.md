# Owned resources and dynamic references: v3 contract

Normative contract of the v3 `resources-dynamic` program. V1 and the published
v2 [applicator contract](SDK-SCHEMA-APPLICATORS.md) remain frozen
([ADR 0020](adr/0020-validation-programs-are-versioned-and-frozen.md)). This is
not full dialect conformance.

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
metadata structs are `ProgramResourceContext` and `ProgramResource`, exported
from the `suspect-schema` crate root.

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

## Fixtures and tests

- `crates/suspect-schema/tests/owned_resources.rs` covers canonical/self/nested
  IDs, physical/retrieval/logical aliases, escaped pointers, outermost
  precedence, unentered candidates, fallback modes, detached/parent-root
  bindings, restored trial scopes, changed-context revisits, ambiguity, resource
  mutation admission, exact numeric failures, recursion and default depth limits.
- `crates/suspect-schema/tests/fixtures/resource-conformance/`: the unmodified
  pinned official dynamicRef file and its remote documents. Every case executes
  through a closed supplied-document provider; there is no runtime acquisition.
  Original URLs, SHA-256 values and license are recorded in its README.
- The dynamic/unevaluated official cases that `compile_v2` refuses execute under
  v3. Frozen `compile_v2` still refuses resource semantics, so its capability
  deferrals and fixtures are unchanged, and v1/v2 envelopes never carry
  `resourceContext`.
- Internal failure values are boxed so a deep distinct-resource chain does not
  overflow a normal stack: the configured **512 depth ceiling** fails explicitly
  on a **2 MiB stack**.

```sh
cargo test -p suspect-schema --test owned_resources
```

The `--ignored` test `published_v2_program_bytes_are_unchanged_with_the_v3_api_present`
recompiles the published v2 vectors and compares them byte-for-byte with a frozen
v2 executable program file, `target/sdk-schema-applicators-executable-v2.json`,
which must be supplied before running it.

## Remaining boundaries

Custom vocabularies/dialects, legacy `$recursiveRef`/`$recursiveAnchor`, the
existing portable regex limitations, assertion-mode formats, and unsupported
OAS 3.0 directional requirements remain explicit refusals. Each native runtime
checks the v3 envelope before executing it
([SDK-SCHEMA-NATIVE-ADOPTION.md](SDK-SCHEMA-NATIVE-ADOPTION.md)).

## Primary rules

[JSON Schema 2020-12 Core](https://json-schema.org/draft/2020-12/json-schema-core)
§§4.3.5, 7.1, 8.2.1–8.2.3 and 9; [OAS 3.2 OpenAPI Object](https://spec.openapis.org/oas/v3.2.0.html#openapi-object)
and Appendix F; [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) §§5.2 and 5.4.
The lower shared resource URI helper is reused through the existing schema URI
seam, retaining its independent tests.
