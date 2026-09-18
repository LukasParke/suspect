# Source-aware SDK compatibility reports

M6 comparison input is an owned source-addressed `Contract` plus the actual
selected native language plans. `suspect_codegen::compatibility` compares typed
descriptors and the resolved HTTP/schema graph directly.

## Integration API

The library API is exposed through `suspect codegen-compare`, using the same
twelve-backend registry and configurations as `codegen-session`. See
[the CLI workflow](SDK-INCREMENTAL-GENERATION.md#native-and-wire-compatibility).

```rust,ignore
use suspect_codegen::compatibility;

let report = compatibility::compare(
    old_contract,       // Arc<Contract>
    new_contract,       // Arc<Contract>
    &operation_ids,     // &[String]; empty means all outgoing operations
    &target_configs,    // &[backend::TargetConfig]
)?;

let json = serde_json::to_value(&report)?;
let migration_notes = report.migration_notes();
let compatible_within_scope = report.is_proven_compatible();
```

`compare_with_targets(old, new, ids, old_targets, new_targets)` also compares
package names, package versions, effective Python/Swift import identities, and
target additions/removals. Each backend may appear once in each target list.
An empty target list is an explicit wire-only comparison.

For repeated comparisons, retain `CompatibilitySnapshot`s:

```rust,ignore
let before = compatibility::snapshot(old_contract, &old_ids, &old_targets)?;
let after = compatibility::snapshot(new_contract, &new_ids, &new_targets)?;
let report = compatibility::compare_snapshots(&before, &after);
```

Snapshot comparison reuses the captured native descriptors without replanning.
Capture calls the native default-policy planners and package admission functions;
some existing planners/package functions render in memory. It does not execute
native toolchains, fetch sources, write packages, or publish. A snapshot retains
its `Arc<Contract>`; it is not a serialized replacement for the canonical graph.
Native records and complete reports are serializable.

Use `compare`/`compare_with_targets` when selected operationIds may have disappeared.
They resolve selection jointly: an ID found on either side remains in scope.
Unique unchanged method/path pairs recognize ID renames after matching unchanged
IDs. A removed selected operation produces a report. An ID absent from both
inputs, an ambiguous explicitly selected ID, or duplicate target configuration
returns `CompatibilityError`. `snapshot` instead requires each explicitly
selected ID to exist in that individual snapshot. Different snapshot selections
represent real generated-surface additions/removals.

## Report contract

`format` is `suspect-sdk-compatibility-v1`.

- `before` / `after`: entry URI, OpenAPI version, selected operations, generator
  version, and SHA-256 fingerprints of every owned closure document's normalized
  JSON. Formatting bytes and source spans are not semantic matching keys.
- `wire`: HTTP/schema findings, with independent request/response direction.
- `native`: one record per backend, including both target configurations,
  actual operation/model symbol snapshots, native declaration changes, plan
  diagnostics, and a target-specific summary.
- `summary`: counts of compatible changes, breaking changes, potentially
  breaking changes, and unknowns. Unknowns prevent `is_proven_compatible()`.
- `scope`: the proof and representation limits applying to this report.

A finding has a stable `code`, `impact`, human-readable `message` and `migration`,
before/after operationIds, original source URI/pointer/span, structured before/after
evidence, and proof reasoning. `schema_deltas` point to changed keywords in their
original documents, including external schema targets behind unchanged `$ref`
text. Property names and instance keys named `description`, `default`, or
`examples` are preserved; annotation filtering applies only to schema positions.

`migration_notes()` is a compact Markdown view. JSON retains full native
declarations and fine-grained schema evidence. Package-only findings have no
invented OpenAPI location.

### Impact meanings

| Impact | Meaning |
| --- | --- |
| `compatible` | A sufficient rule proves compatibility within the stated profile, or this is metadata/additive evidence |
| `breaking` | An endpoint/public binding was removed or changed, or an explicit transport presence requirement changed incompatibly |
| `potentially-breaking` | A relevant native shape/domain changed and compatibility has not been proved; no schema satisfiability/counterexample proof is implied |
| `unknown` | A required proof/descriptor is unavailable, a planner refused the target, or comparison limits were reached |

Planner failure is not an empty successful API: its original diagnostic becomes
an unknown, and the unavailable native surface is not misreported as wholesale
operation/model deletion. Known wire/package deltas still survive.

## Wire variance and proof boundaries

The baseline is an existing client against a new server contract:

- **Requests:** old admitted values must be included in the new input domain.
  Loosening a string bound or allowing null can be compatible.
- **Responses:** new admitted values must be included in the old output domain.
  Tightening the same bound can be compatible; allowing new null values can
  invalidate old consumers.

These are independent from upgrading a generated package. A wire-compatible
response narrowing can still remove a Rust/Swift enum case, Go constructor, or
Python/TypeScript exported type used by source consumers.

The wire engine compares method/path, effective servers, security alternatives
(OR) and conjunctive scheme/scope requirements (AND), parameter serialization,
required presence, request/response media, response headers and schemas. Exact
statuses take precedence over range responses and then `default` over the finite
100–599 HTTP status domain. A new exact status covered by an equivalent old
`default` is not automatically a new output possibility. Transitioning to/from
no declared response content is flagged separately; an empty content map is not
silently treated as safe deletion of a media alternative.

OpenAPI3.2 `itemSchema` has its own request/response variance and resolved schema
closure. Changing a referenced item schema cannot disappear behind unchanged
media or reference text; malformed item declarations remain unknown. Media Type
Object references retain their mount locations while comparison reads effective
terminal metadata.

Relative server identity includes its effective physical HTTP retrieval base,
including the implicit `/` default and explicit empty server arrays. `$self`,
schema `$id`, requested redirect aliases and cache paths do not relocate a server.
Absolute server templates are independent of retrieval relocation. Local-file
inputs retain the requirement for a caller-supplied HTTP document URL; a network
host is not invented from a file path. Template-dependent base comparisons remain
conservative.

Named and positional multipart/form encoding metadata, including referenced
part headers and nested item encodings, require a protocol-specific proof and
remain explicit unknowns in this structural comparison profile. Equal reference
text does not waive that boundary.

Supported schema proofs are deliberately sufficient rather than complete:

- Source-independent structural equality follows **resolved static references**,
  checks dialects, and visits recursive graph pairs without expanding them.
  Source relocation/model renaming can preserve wire equality.
- Canonical resource references use Contract's logical-resource resolution while
  retaining physical sources in deltas. Paired reference-only wrappers are aligned
  before target inclusion, so a mixed-dialect closure compares corresponding
  target dialects. Cross-dialect assertion equivalence and dynamic binding remain
  outside this static proof profile.
- Simple conjunctions cover type/null domain inclusion, each const/enum
  restriction, object requiredness/properties/additional properties, array items,
  unique items, and monotone scalar/cardinality bounds.
- Unequal numeric bounds are ordered only when exact signed 128-bit integer
  parsing succeeds. Decimals, exponents and larger integers decline the numeric
  implication proof rather than using floating point. Identical exact values
  still support structural equality.
- Changed composition (`oneOf`, `anyOf`, `allOf`, negation/conditionals), patterns,
  extension-dependent assertions, cross-dialect semantics, and dynamic-reference
  scope remain unknown when no structural equality proof applies. Adding a
  `oneOf` branch is not assumed to widen its domain: overlap can instead invalidate
  values that previously matched exactly one branch.
- Inclusion through a changed recursive cycle remains unknown. Each schema use
  has a finite comparison work budget and inclusion depth limit. Exhausting a
  limit yields unknown, never compatible.

Schema defaults and documentation do not become invented runtime behavior.
Unchanged unsupported declarations can still produce contract/native diagnostics;
“no textual change” is not a substitute for a usable target plan.

## Native-plan coverage and owner accessors

Native records use format `suspect-native-interface-v1`. Native shapes preserve
allocated names and typed wrappers/variants; source comments, descriptions and
byte spans are excluded from declaration equality. Model correspondence follows
paired HTTP uses and canonical reference edges, retaining both original addresses.
Changes to unknown/inaccessible portions are never inferred from code text.

Source relocation alone does not change a declaration: TypeScript `Expr::At`
addresses remain provenance rather than native type identity. Allocated names in
reference types and initializers remain significant. Moving a source can change
collision allocation, and those native renames are still reported even when the
wire graph is equivalent. Wire names/locations in field/parameter records remain
available as bindings; changing only those bindings does not invent a native
signature change when the actual member, type and presence are unchanged.

| Target | Captured input | Explicit remaining descriptor gaps |
| --- | --- | --- |
| Go | Actual allocated operation/input-constructor, parameter/body fields and fluent setters, concrete response types, closed response-union membership and credential constructors; typed model declarations including field/constructor metadata, presence/null types and unions/literals | Runtime API/ABI outside the recorded declaration profile |
| Rust | `HttpPlan` operation symbols and typed `interface()` bindings; typed model declarations, fields, wrappers, variants and `Field::init`; scoped codec profile | Runtime API/ABI outside the recorded declaration profile |
| Python | Actual planned operation/parameter names, public operations/module exports, retained collision-allocated status classes, schema/model bindings; `PyDecl`/`PyType` dataclasses, fixed values, nullable/Unset, unions and contextual codec profile | Runtime API/ABI outside the recorded declaration profile |
| TS/JS | Typed HTTP bindings; actual `ModelSymbol::expression()` trees including directional fields, primitives, literals, references, arrays, unions/intersections; typed codec/resource graphs and limits | General TypeScript assignability beyond equal descriptors is not proved |
| Swift | `SdkPlan` operation/parameter names; actual typed model declarations, fields, initializer defaults/order, literal/union cases, and allocated codec symbols; typed input constructors and exact-status response enums | General Swift assignability and runtime API/ABI beyond the descriptor profile are not proved |
| Java | Actual package/client, immutable models/accessors/builders, input/media/part constructors, response variants, credential hooks and source validation profile | Runtime/ABI and general JVM assignability beyond recorded descriptors |
| C# | Native namespaces, model members/constructors, Task/input/result types, source validation and contextual JSON/pattern-extra obligations | Runtime/ABI outside the recorded declaration profile |
| Kotlin | Native packages, data/sealed/checked-JSON model shapes, coroutine inputs/results, constructor defaults, credential context and validation profile | Runtime/ABI and general Kotlin assignability beyond recorded descriptors |
| Ruby | Keyword constructors, native model/operation names, scoped extras, checked dynamic carriers and codec profile | Runtime behavior and dynamic Ruby assignability outside the descriptor profile |
| PHP | Actual namespaces, typed/PHPDoc models and constructors, allocated codecs, media/part/header/result classes and resource-aware carrier obligations | Runtime behavior beyond the recorded native interface |
| Dart | Package/library identity, null-safe model/codec/operation declarations, contextual carriers and complete codec policy | General Dart assignability and runtime/ABI beyond recorded descriptors |
| C++ | Actual namespace, native value/variant/model/codec declarations, operation constructors/results and checked resource profile | ABI and toolchain/library compatibility outside the declared native profile |

Swift is registered as `Backend::SwiftHttp` (`swift-http`). Its module name is
`TargetConfig.import_name.unwrap_or(package_name)`; the Swift backend validates
package/module identifiers and the exact version through `PackageConfig`
admission. An explicit import equal to the default does not invent a source
change; an invalid module name produces an unavailable target with its diagnostic.

The selected targets' native accessors are integrated. Rust constructor parameter
order and `Default` availability follow the actual `Init` variants, including
source-defined singleton initialization; Python concrete response names are read
from `PlannedResponse`, not recomputed from operationId. Native self-comparison
uses the same selected planner/options as canonical generation. Dynamic wire
equivalence remains unknown even when the native descriptors compare equal.

Go HTTP names also come directly from the completed naming plan:
`input_constructor`, parameter/body `setter_name` and body `field_name`, and
`PlannedResponse::type_name`. Constructor argument order includes only required
parameters followed by a required body. A constructor is not reconstructed as
`New{input_type}`: a model or another package declaration can reserve that name.
Likewise, new optional input fields can displace existing fluent setters even
when the wire change is compatible. The record keeps the actual concrete
response type, closed success/error interface, and value versus pointer method
receiver. The private seal is the profile's fixed `is{allocated-interface}`
method; API error interfaces additionally embed `error`. That rule is covered
by the fingerprinted Go emitter, rather than guessed from operationId.

Swift records actual declarations as role `model`, and each allocated `Codecs`
property as role `codec`. A single source can own both: `Thing.codec` consumes the
non-null declaration, while `Codecs.thing` can consume `Nullable<Thing>`. Matching
uses source correspondence **and role** so component reuse, nullable domains and
record order cannot confuse the two interfaces. Scalar/ref type bindings appear
in codec/operation signatures; unused planning name hints are not reported as
exported model types.

Swift constructor ordering comes from `Declaration::constructor_fields`, with
defaults from `Field::initializer`: missing optional fields, exact source string
singletons, or required arguments. Additional-property maps follow with their
actual empty-map default. Both `Nullable`/`Presence` and intrinsic `JsonValue`/
`JsonNull` domains retain their native representations, including `Indirect`
layout wrappers and indirect union enums. Literal cases, variant associated
types, declaration codecs and independently allocated namespace codecs all remain
in the native record. HTTP records use the actual success/error enum names and
their exact `statusNNN` cases; the non-emitted `error_variant` planning
label is excluded.

Go's model-level literal/variant suffixes now come from global allocation that
also reserves constructors and runtime constants; the existing typed descriptor
adapter retains those allocated suffixes. The HTTP namespace guard still turns
any future intrinsic inconsistency into a source-linked planner refusal. Such
refusals remain unknown in compatibility reports. A wire-only result can be
compatible while a refused native target
is unknown.

## Provenance and verification

Each native record stores the generator version, default-policy profile and a
SHA-256 fingerprint of the explicit transitive planning/emission asset list in
`compatibility/provenance.rs`. Paths in `fingerprinted_assets` are relative to
`crates/suspect-codegen/src`; entries are sorted and deduplicated before hashing.
Names and file contents are length-delimited in the hash input.

Language-owned `source_assets()` inventories are merged into that same hash;
capture functions do not append a second, differently framed digest. Empty and
planned selections retain the same asset bytes and list. The public integration
test recomputes each declared fingerprint from its actual source files across the
compiled registry.

The list covers every current selected profile's JSON, exact numeric/pattern
validation, model/presence, codec and HTTP runtime, along with its Rust planning
and code/documentation emission modules. It includes shared HTTP/example planning
and documentation, Python/Go's shared native naming helpers, Go's reused Rust
codec policy, shared schema validation defaults, TypeScript's naming filter, and
the reviewed TypeScript package lock used by package emission. This
captures embedded package/docs/example templates as well as runtime files; a
change to a template is not hidden behind an unchanged HTTP runtime file.

`runtime_version: null` means that runtimes are bundled with the generator and
have no independently declared release version. The asset list is the exact
fingerprint scope: other canonical/schema compiler internals, the dependency
resolver and installed external toolchains are outside it. It does not prove
runtime ABI.
Changed provenance produces an unknown that requires the native conformance
gates. `compare` plans both inputs with the current generator; retaining snapshots
is necessary to compare historical native surfaces rather than regenerating both
sides under today's naming rules.

`crates/suspect-codegen/tests/sdk_compatibility.rs` exercises canonical public
contracts: unchanged entry with changed external targets, docs-only edits,
per-language operation/model renames, null/presence and input/output variance,
operation removal, method/path/security changes, native refusal, contradictory
literal intersections, composition/pattern/large-number uncertainty, recursive
references, status/default coverage, package identity, and serializable native
provenance. Native cases cover completed no-change/docs-only proofs, unchanged
names under source relocation, collision-driven native renames, Rust singleton
constructors and `Default`, Python keyword/status-class allocation, TypeScript
directional field expressions, Go constructor/response/setter collisions and
closed response method membership, and Swift initializer/default ordering,
model-versus-codec roles under shared-component reuse, nullable codec signatures,
module admission, and all-target field/null/union changes. These checks do not derive their oracle from
emitted code or compare the implementation to itself.

```sh
cargo test -p suspect-codegen --test sdk_compatibility --locked
```

Current integrated verification is recorded in [SDK-PROGRESS.md](SDK-PROGRESS.md).
