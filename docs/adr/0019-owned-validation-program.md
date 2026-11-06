# Generated SDKs validate by running an owned, portable validation program

`OwnedCompiler` (`crates/suspect-schema/src/owned/`) compiles the selected schema closure from the Contract into an `OwnedProgram`: a finite list of typed instructions, each carrying its source document and keyword pointer. Each of the 12 backends emits a dependency-free evaluator for that program and runs it inside its codecs. Codecs validate before building a native value on decode and validate the current value again on encode. No generated package depends on a third-party JSON Schema library, so every language gets the same semantics: exact numbers, the same regex subset, the same work budgets, the same outcomes (ADR-0021) and source-located errors. The cost is 12 evaluators, held together by the shared vectors in `crates/suspect-codegen/tests/fixtures/runtime-contract-v1.json`.

## Considered Options

- Per-language libraries (ajv, jsonschema, gojsonschema, …): they disagree on drafts, keywords, number handling and limits, and add a runtime dependency.
- Embedding the editor's `suspect-schema` evaluator: it borrows its document, is neither `Send` nor `Sync`, and had exactness and annotation-scope defects at the time.
- Optional validation, lean structural decoding by default: proposed and not built; validation is always on.
