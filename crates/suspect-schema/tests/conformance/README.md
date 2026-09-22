# JSON Schema conformance fixtures

These unmodified JSON files are from the official
[JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite),
commit `f6fd52a0a95472e079cbfc6ef7f089702b80e045`, retrieved 2026-09-08.
The upstream MIT license is included in `LICENSE`.

`draft2020-12/` currently contains 20 cardinality, string-length, contains,
numeric, type, enum, const, uniqueItems and unevaluated-properties/items files.
The public `Compiler::compile` → `Schema::validate` tests in `../conformance.rs`
execute every case in each included file. There are no case skips or patched
expectations. This limited selection is not a claim of full draft conformance.

OpenRouter remains the primary SDK workload. These independent normative
vectors supplement the tracked OpenRouter tests in `../cardinality.rs` and `../numeric.rs`; they
exercise cases absent from that workload, such as mathematically integral
decimal bounds and type-applicability rules.
