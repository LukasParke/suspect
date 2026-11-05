# Validated SDK example planning

Native packages share `examples::plan_protocol_examples(Arc<Contract>,
&ProtocolPlan, ExampleConfig)`. It consumes the already admitted HTTP protocol
slots instead of running another narrower HTTP admission pass. Every entry
carries operation, wire-slot and schema identity, exact normalized JSON,
declared/synthesized origin and the original declared value's location.

The original `plan_examples(Arc<Contract>, &[SourceId], ExampleConfig)` API keeps
its bounded v1 JSON HTTP profile. Both entrypoints use v1 owned validation.
Native adapters that have completed scoped-applicator admission select
**`plan_protocol_examples_v2`** with the same arguments. This explicitly uses
`OwnedCompiler::compile_v2` while retaining the same discovery, provenance and
finite synthesis policy. Base schemas retain their v1 values and origins.
**`plan_protocol_examples_v3`** explicitly selects `compile_v3` for an admitted
resource/dynamic native profile. Its validity checks use actual entered resource
scope; annotation discovery follows static references and never guesses that a
dynamic reference's initial fallback supplied an active declaration.

Inline container examples and named Example Objects override schema annotations.
Named references use the canonical reference graph; `$ref` inside an instance
remains ordinary data. Static schema references can contribute `example` and
`examples` annotations. Equal values at different declaration sites keep their
distinct provenance. No external example URL is fetched.

The owned validator decides validity. Invalid declarations retain located
findings and are never silently repaired or relabeled. When needed, synthesis
tries source literals/enums, primitive bounds, required object fields, bounded
arrays and composition candidates. Each candidate and emitted value must pass
its actual schema. Recursive required structures, unproven contextual directions,
complex synthesis and exhausted work produce explicit unavailable findings.
Incomplete evaluation is never classified as an invalid instance.

Protocol slots cover schema-bound parameters, request/response media, typed
headers, named/positional parts and per-item stream schemas. Their containers
retain the real use-site binding through references, and declared examples keep
their terminal value source. HEAD and body-forbidden statuses do not gain payload
examples even when the same schema is used by a request. Binary and schema-free
payloads remain explicit native fixture obligations; JSON null is not a byte
placeholder. OpenAPI3.2 `dataValue` remains distinct from serialized/external
example representations.

Complete declared form/multipart JSON examples are validated before projection
to native part-codec inputs. Their whole values and original locations are kept
in `OperationExamples.validated_aggregates` and the optional `validatedAggregates`
manifest field. These are example-only schema roots, not additional native JSON
codecs. The first valid aggregate supplies member/item values with their exact
child source pointers; omitted optional members remain absent. Other valid
aggregate declarations retain their own provenance, and invalid declarations
retain findings before independent bounded fallback synthesis.

Positional bindings carry `ExamplePartPosition::Prefix(index)` or `Items`
independently of schema identity. A reused `items` schema does not merge prefix
encodings or populate an absent later prefix/tail. The optional `partPosition`
manifest field records this wire occurrence while `schema` retains its actual
codec/source identity.

An aggregate containing a native byte representation is explicitly unavailable
for this JSON example path at its declared location. No filename, string or JSON
null is substituted for bytes. Native recipes can use the complete validated
aggregate for group construction, including empty arrays, named extras and
positional tails. The manifest omits `validatedAggregates` when empty, preserving
ordinary JSON/v1 artifact bytes.

`max_work` counts plan-wide visits, validation attempts and copied JSON bytes;
declared values are checked by a counting sink before cloning. Candidate, string,
depth and per-validation budgets are finite. Profile ceilings are depth 64,
64 candidates, 10 million work units, one million evaluation steps, 65,536 string
characters and 128 declared candidates per slot. Larger policies are rejected.

Packages emit shared `examples.json` provenance and native source-bound samples
that validate planned data again and construct available typed inputs. The exact
package paths, execution commands and native lowering are in each language guide;
for the original TS/JS and Rust packages:

- TS/JS: `npm run build`, then `node dist/examples/validated.js`.
- Rust: `cargo run --example validated --features http`.

Samples make codec calls; actual client exchanges are exercised by the shared
native HTTP consumers. Customized runtime budgets still apply. The public
`sdk_examples` and installed `sdk_example_packages` suites exercise independent
fixtures plus the five tracked OpenRouter operations, preserving the real invalid
PATCH-key response example as a source finding.
