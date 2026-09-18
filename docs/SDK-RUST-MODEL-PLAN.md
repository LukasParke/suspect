# Canonical Rust model planning

`suspect_codegen::rust_models::plan_models(&Contract, roots)` is an experimental,
neutral model backend. It consumes the source-addressed Contract directly.
The [Rust HTTP profile](SDK-RUST-HTTP.md) reuses its typed plan with checked
codecs and selected operations through CLI/editor generation. The model-only
API retains its explicit codec obligations.

The immutable `ModelPlan` contains allocated public symbols, representation
roles, canonical source identities, source spans, source-linked diagnostics and
planned declarations. Code and documentation render from that same snapshot.
Rendering never re-reads OpenAPI or re-allocates identifiers. The existing
ownership-aware artifact writer publishes the resulting `OutFile` values.

The generated package contains `Cargo.toml`, `src/lib.rs`, `src/models.rs`,
`src/support.rs`, `src/json.rs` and `README.md`. Every planned declaration has Rustdoc and a
README entry with its exact original schema and source address. Descriptions
are escaped as prose: source Markdown does not become an executable Rustdoc
example or an inferred Rust symbol link.

## Implemented representations

| Source construct | Rust model candidate |
| --- | --- |
| Required non-null property | Required constructor argument and field |
| Required nullable property | `Nullable<T>` with `Null` and `Value(T)` |
| Optional non-null property | `Option<T>` |
| Optional nullable property | `Presence<T>` with `Absent`, `Null`, `Value(T)` |
| Nullable named schema | Complete alias plus separately named non-null value declaration |
| Boolean true schema | Exact JSON value domain, including null |
| Boolean false schema / empty enum | Uninhabited type |
| String / Boolean | `String` / `bool` |
| General JSON number | Opaque, exact `JsonNumber` token |
| Unbounded integer | Opaque, mathematically integral `JsonInteger` token |
| Integer with proven finite bounds | Smallest fitting native signed/unsigned integer, up to 128 bits |
| Scalar enum / const | Enum variants with an exact `wire_json()` literal accessor |
| Declared oneOf / anyOf members | Payload enum containing the non-null alternatives |
| Array / typed map without declared fields | `Vec<T>` / `BTreeMap<String, T>` |
| Named fields plus typed additional properties | Native fields plus a separately typed private extra map |
| Proven reference-carrier allOf | Redundant overlays or required-presence strengthening of existing fields |
| Recursive object/union layout | `Box` on deterministic cycle-closing layout edges |

Null is factored out before presence is added. Optional nullable properties do
not accidentally become `Presence<Nullable<T>>`. Collection indirection is
recognized, so recursive values inside vectors and maps do not acquire an
unnecessary box. Alias-only recursion is rejected, including alias cycles
through collections, because Rust cannot express recursively expanding aliases.

Constructors initialize optional fields as absent and require all other fields,
except source-defined singleton enum/const values. Singleton discriminator tags
can therefore be constructed without asking callers to repeat a known constant.
Examples and defaults do not invent constructor values. Discriminator mappings
only name already-declared reference alternatives; they do not create branches
or imply schema validation.

Field names are allocated as Rust identifiers, while exact wire names remain in
the plan, docs and extra-field guards. Unknown fields use a private map of the
declared additional-property type (`JsonValue` for unconstrained extras).
`insert_extra` rejects declared wire names; the public iterator does not expose
unchecked mutation. Struct literals cannot bypass private storage. Numeric
values in arrays and objects keep exact tokens.

Neutral models retain readOnly/writeOnly and other annotations in source docs.
Directional request/response models are not implemented in this slice.

## Exact-number runtime

The generated model package has no external dependencies. It declares Rust 2024
and Rust 1.88 as its minimum compiler version. Native floor/current evidence is
recorded in [SDK-PROGRESS.md](SDK-PROGRESS.md) and the linked milestone reports.

The runtime scanner implements JSON numeric grammar directly. It rejects
whitespace, leading plus signs/zeroes, incomplete fractions or exponents, NaN,
Infinity and non-ASCII digits. It retains valid original spelling, including
negative zero, tiny decimals and arbitrarily long exponents.

`JsonInteger` checks mathematical integrality by decimal scale and trailing
zeroes. Thus `100e-2` and `1e400` succeed, while `1e-400` fails. Exponents are
scanned symbolically; values never expand to an exponent-sized allocation.
Checked `to_i128()` / `to_u128()` conversions return `None` outside the target
range. Negative spellings of mathematical zero convert to unsigned zero.

Time and storage are O(token length). There is no application input-size limit
in these model helpers; callers retain their normal input/resource limits.
There is no float conversion, arithmetic API or mathematical equality API.
Token equality is deliberately not presented as number equality.

The generated root also exports `parse_json`, `parse_json_bytes`,
`stringify_json`, `JsonLimits`, `JsonError` and `JsonErrorKind`. These operate
on the exact generic JSON domain with finite bytes/depth/scanning-work policies;
they do not serialize native models or establish schema validity. See
[Rust JSON](SDK-RUST-JSON.md) for Unicode and resource boundaries. Models live
in `models`, while runtime support lives at the crate root; source names such
as `JsonError` remain usable through qualified imports. Actual collisions within
the model namespace retain deterministic allocation. Zero-argument model constructors also
have an equivalent `Default` implementation; schema defaults are not applied.

## Explicit blockers

The model-only API retains a `model-codec-unimplemented` obligation for each
selected root; nonempty model plans do not return `release_ready() == true`.
The separate codec plan below discharges those representation obligations with
checked runtime validation, without certifying an HTTP SDK. Model-only native
type checking proves construction/layout, not scalar constraints or exclusivity.
- The model-only renderer provides no Serde derives, implicit untagged-union
  validation, HTTP transport, or native model serialization. Its exact JSON
  representation helpers remain separate from schema validation.
- General intersections beyond the bounded reference-carrier proof,
  conditionals, pattern/unevaluated applicators, untyped constraints,
  undeclared required keys and multiple non-null type-array alternatives remain
  unsupported by model lowering. Exclusively folded overlays are not promoted
  independently; explicitly selected or referenced overlays keep normal admission.
- Numeric const/enum intersections block emission until exact mathematical
  equality lowering exists. `const: 1` intersected with `enum: [1.0]` must not
  become empty through token equality.
- Object/array literal enums and assertion siblings of references/unions are
  unsupported. Unknown roots and canonical contract errors also block emission.

`DiagnosticKind::Error` prevents `render()`. Codec obligations allow reviewable
model artifacts, but always block release. Annotation findings preserve source
information without adding validation rules.

## Validated model codecs

`rust_codecs::plan_codecs(Arc<Contract>, roots, CodecConfig)` reuses immutable
typed model descriptors and the checked `OwnedProgram`; it never parses emitted
Rust to rediscover a second type system. It emits a complete standalone Cargo
package with no dependency on Suspect or third-party runtime crates.

Every public model has `codecs::<Symbol>Codec` methods `decode(&str)`,
`decode_bytes(&[u8])`, `decode_value(JsonValue)`, `encode(&Model)` and
`encode_value(&Model)`. Decode validates before conversion. Encode validates
both the selected native union branch and the complete wire value, including
mutated models. `CodecError` distinguishes JSON representation, completed
invalidity, incomplete evaluation and conversion failures.

Requiredness, null/absence, exact numbers, literal equality, typed extras and
recursive layout remain explicit. Inclusive unions choose the first validating
source branch; exclusive unions require exactly one. Branch trials share
evaluation/equality budgets and retain actual nested instance/source paths.
Schema literals are planned exact values, initialized independently of transport
input limits; repeated trials do not reparse them as request documents.

`CodecConfig` contains the owned schema policy, JSON input/output/work limits,
and native conversion depth/work limits. These are bounded-resource controls,
not complete CPU/heap accounting. The JSON and conversion depth ceilings are
256. Native evaluation has its own explicit admission policy described in
[owned validation](SDK-OWNED-VALIDATION.md).

Native codec consumers cover tracked `ORAnthropicNullableCaller`,
`AnthropicImageBlockParam`, `ChatChoice.index`,
`ImageGenerationServerToolConfig`, `ChatRequest` and `ChatResult`, along with
adversarial presence, union, numeric, mutation, naming and budget cases.
See `tests/rust_codecs.rs` and `tests/rust_validation.rs`. Codec Rustdoc/doctests
use the same model symbols, qualify standard-library names, and share the model
renderer’s inert-prose/HTTP-link handling. Rust HTTP is implemented by the separate
bounded HTTP profile; directional projections remain open. This is not whole-corpus
or complete SDK certification.

## Native evidence

`crates/suspect-codegen/tests/rust_contract.rs` contains seven end-to-end cases:
two ordinary planning/error tests and five explicit native package cases. The
native harness runs a separate consumer library, so `compile_fail` examples
execute as actual doctests. It also tests generated-package doctests and builds
Rustdoc with broken links denied; native compilation denies warnings.

Coverage includes independent presence/null, false/empty schemas, singleton
construction, exact wire-name guards, private extra storage, mutual recursion,
collection indirection, reserved/colliding/Unicode names, inert untrusted prose,
JSON number grammar, negative zero, huge exponents, integral decimals, i128/u128
range limits and exact values stored in generic JSON objects.

The tracked OpenRouter public YAML is exercised directly for
`ORAnthropicNullableCaller` and `AnthropicImageBlockParam`, including every
caller branch, the null member, URL and base64 source payloads, MIME literals,
required source fields and source-defined tags. This is a bounded model-closure
check; it is not full-corpus Rust support or runtime/HTTP acceptance.

The `rust_codecs_json` and `rust_json` suites add native exact-JSON behavior,
actual generated-package imports, executable examples, Rustdoc/Clippy and
dependency-tree checks. All four tracked OpenRouter documents round-trip after
Contract normalization, with independent serde_json JSON grammar/value
comparison. This is not an independent YAML normalization oracle or a claim
that the Rust model planner admits every schema in those documents.

Verified commands:

```sh
cargo test --offline -p suspect-codegen --test rust_contract -- --include-ignored --nocapture
cargo test --offline -p suspect-codegen
cargo clippy --offline -p suspect-codegen --all-targets -- -D warnings
```
