# Original JSON Schema applicator conformance fixtures

Unmodified files from the official JSON Schema Test Suite, revision
`f6fd52a0a95472e079cbfc6ef7f089702b80e045`, retrieved 2026-09-10 from
`https://raw.githubusercontent.com/json-schema-org/JSON-Schema-Test-Suite/`.
The original MIT LICENSE is included. Exact URLs and SHA-256 hashes are recorded
in `target/sdk-schema-applicators-oracle-sources.json`.

`owned_applicator_conformance.rs` evaluates every case in these six files through
Contract, `OwnedCompiler::compile_v2`, `program().check()`, and actual validation.
Each original schema is a real standalone schema document behind an OpenAPI ref;
`#` and `#/$defs/...` references are not rewritten or re-rooted into components.

Five other files already present in `tests/conformance/draft2020-12` are reused
without modification. Exactly two named groups (four cases) use embedded `$id`
resources and `$dynamicRef`: those assert the concrete current capability refusal
and are counted separately. One additional named patternProperties group (two
cases) uses Unicode property escapes outside the existing portable NFA profile;
it asserts that concrete source-linked refusal. These are not silently skipped
or reported passing. All remaining 395 cases must compile, pass checked admission, complete evaluation,
and match the original validity expectation. This is not full draft conformance.
