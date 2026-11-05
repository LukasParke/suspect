# Large contract generation

Omitting CLI operation selectors, or setting session `operation_ids` to `[]`, attempts every outgoing operation. Unsupported selected declarations produce source-linked diagnostics. A successful full-scope run includes the selected operation set in each native package's metadata.

## Source semantics

Generation consumes an explicit contract. For SSE, OpenAPI 3.2 `itemSchema` describes parsed event envelopes; `data` is a string. `contentMediaType` and `contentSchema` can describe JSON carried inside that string without implicitly decoding it. Application-level sentinels, retries, and pagination require their own behavior beyond standard framing.

Malformed or ambiguous source declarations should be clarified explicitly, with the original input and a change ledger retained. Backend model support preserves the original schema assertions: ordinary `allOf` and numeric enum codecs can use checked exact-value carriers even when their validation program only needs v1 instructions. Native-field ergonomics and schema validation are separate capabilities.

## Native size boundaries

Large schemas exercise compiler and metadata limits separately from request-serving budgets:

| Surface | Strategy |
| --- | --- |
| TypeScript metadata declarations | Explicit property types refer to each operation's wire constant with `typeof`, preserving exact types without serializing the whole metadata object as one inferred declaration. |
| Kotlin codec classes | Groups of at most 128 models distribute getters and conversion functions across files. The public `Codecs.member` surface inherits its getters through sealed, stateless groups. |
| Kotlin executable examples | Individual values and fixture calls use separate methods, keeping the coroutine entrypoint below the JVM method-size ceiling. |
| Kotlin static JSON metadata | Up to 16 MiB, 16,384 validation nodes, and 1,000,000 metadata values. Public payload JSON retains its 4 MiB and 100,000-value ceilings. |
| Java HTTP metadata | A separate strict JSON loader permits up to 32 MiB of generated metadata. Public request/response JSON retains its 8 MiB ceiling. |
| Java and C# package text | A finite 256 MiB emitted-source package ceiling accounts for models, source metadata, examples, and documentation. |

The static loaders retain strict UTF-8/JSON handling. Per-call validation work, numeric, depth, and HTTP limits remain independently enforced.

## Regression witnesses

- `tests/native_model_admission.rs` exercises ordinary and nested intersections across registered backends, Go numeric membership and both codec directions, and TypeScript integer examples inside nullable object intersections.
- `tests/kotlin_program_scale.rs` loads a 6,000-node validation program and compiles thousands of codecs while checking inherited public access and source assertions.
- `tests/java_metadata_budget.rs` distinguishes generated metadata loading from public payload limits and checks strict parsing.
- `dart_sdk::mixed_stream_tests` checks actual JSON/SSE/no-content/error dispatch, raw sentinel data, finite-body bounds, and response ownership.
- `tests/python_codecs.rs` includes a source model named `Model`, so its generated `ModelCodec` value cannot shadow construction of later codecs.

Native witnesses are explicitly selected with `--include-ignored` and require their named toolchains. Full API coverage and passing native checks establish a staging checkpoint; package manifests continue to identify the SDK profiles as experimental.
