# Native scoped-validation adoption

Every one of the twelve SDK runtimes executes all three owned validation program
tiers ([ADR 0020](adr/0020-validation-programs-are-versioned-and-frozen.md)).
The planner picks the oldest envelope the selected closure needs:

| Tier | Version / profile | Compiler | Emitted when |
| --- | --- | --- | --- |
| v1 | `suspect.validation.experimental.v1` / `oas31-jsonschema202012-static-subset` | `OwnedCompiler::compile` | Ordinary static closures |
| v2 | `suspect.validation.experimental.v2` / `oas31-jsonschema202012-static-applicators` | `OwnedCompiler::compile_v2` | A closure uses one of the nine scoped applicators |
| v3 | `suspect.validation.experimental.v3` / `oas31-jsonschema202012-resources-dynamic` | `OwnedCompiler::compile_v3` | A closure needs schema resources or dynamic references |

Ordinary closures keep their v1 bytes; adding a newer tier never changes the
output of SDKs whose schemas do not use it.

## Contract and fixtures

- [Executable v2 semantics](SDK-SCHEMA-APPLICATORS.md), including exact annotation
  contributions, branch isolation, evaluation failures and visit/set-merge costs.
- [Owned v3 resource/dynamic program](SDK-SCHEMA-RESOURCES.md). The public crate
  re-exports `ProgramResource` and `ProgramResourceContext`. `compile`/`compile_v2`
  omit resource metadata; `compile_v3` explicitly emits the v3 envelope.
- `ProgramInstruction::requires_v2()` and the exact `OwnedProgram` version/profile
  constants support admission fences and native dispatch.
- v2 vectors: `crates/suspect-schema/tests/fixtures/owned-applicators-v2.json`
  (32 cases).
- v3 vectors: the unmodified source cases and supplied remote documents in
  `crates/suspect-schema/tests/fixtures/resource-conformance/`.

Native tests compile these source fixtures through the real `compile_v2` /
`compile_v3` APIs. They must not depend on a previously generated witness file.

## Requirements for a native runtime

1. Refuse unsupported instructions at their source before emitting a package.
   Unknown versions, mismatched profiles and malformed operands never become
   valid programs.
2. Implement all nine v2 instructions and scoped evaluated-property/item sets.
   Child scopes are fresh; only the documented successful annotations propagate.
   Numeric/equality/work failures remain noninvertible and shared across trials.
3. For v3, bind dynamic references through actually entered resources with exact
   context-aware cycle identity; never substitute an initial-target static guess.
4. Verify the shared vectors, including exact source/instance locations and
   failure-versus-invalid distinctions, plus native-specific ownership, mutation,
   key-identity, recursion, cancellation or resource controls where needed. Keep
   exact numbers and omission/null behavior.
5. Admit faithful native model/codec representations. Pattern-matched extras may
   not be dropped or treated as forbidden by an old additional-properties model.
   A runtime-validated JSON-value carrier is appropriate where static native
   types cannot express the constraint; it must remain checked on decode/encode.
6. Exercise actual SDK operations using the new schemas, positive/negative native
   consumer types, generated examples and rendered symbol documentation on each
   declared toolchain tier. Model-only APIs retain explicit codec obligations.

## Per-backend tests

Run each selector below separately as `cargo test -p suspect-codegen <selector>`;
`--lib <module>` uses the module path as the test-name filter. Planner and
descriptor cases run by default; native-toolchain cases are `#[ignore]` and run
with `-- --include-ignored`.

| Backend | v2 tests | v3 tests | Native toolchain overrides |
| --- | --- | --- | --- |
| Rust | `--test rust rust_validation_v2::` | `--test rust rust_validation_v3::`, `--test rust rust_protocol_v3::`, `--test rust rust_protocol_resources::` | `SUSPECT_NATIVE_RUST_TOOLCHAIN` |
| Swift | `--test swift`, `--lib swift_sdk::validation::v2_tests` | `--lib swift_sdk::validation::v3_tests`, `--lib swift_sdk::resources_tests` | `SUSPECT_SWIFT_RESOURCE_BASE` |
| Ruby | `--test ruby ruby_schema_v2::` | `--test ruby ruby_schema_v3::`, `--test ruby ruby_document_servers::` | `SUSPECT_RUBY_HOME`, `SUSPECT_RUBY_GEMS` |
| C# | `--lib csharp_sdk::validation_tests` | `--lib csharp_sdk::resources_tests` | `SUSPECT_DOTNET_BIN` |
| Dart | `--lib dart_sdk::validation_tests` | `--lib dart_sdk::v3_tests`, `--lib dart_sdk::v3_sdk_tests` | — |
| Python | `--test python python_schema_v2::`, `--test python python_applicators::` | `--test python python_schema_v3::`, `--test python python_resources::` | `SUSPECT_PYTHON_TOOLS`, `SUSPECT_PYTHON_SCOPED_VERSIONS` |
| Java | `--test java java_schema_v2::` | `--test java java_schema_v3::` | `SUSPECT_MAVEN_BIN` |
| PHP | `--lib php_sdk::tests_v2` | `--lib php_sdk::tests_v3` | `SUSPECT_PHP_BIN`, `SUSPECT_COMPOSER_PHAR`, `SUSPECT_PHPSTAN_PHAR` |
| Kotlin | `--test kotlin kotlin_validation_v2::` | `--test kotlin kotlin_validation_v3::`, `--test kotlin kotlin_validation_resource_integration::` | — |
| C++ | `--lib cpp_sdk::v2_tests` | `--lib cpp_sdk::v3_tests` | `SUSPECT_CPP_CXX`, `SUSPECT_CPP_CMAKE`, `SUSPECT_CPP_DOXYGEN` |
| Go | `--test go go_schema_v2::` | `--test go go_schema_v3::` ([SDK-GO-SCHEMA-V3.md](SDK-GO-SCHEMA-V3.md)) | `SUSPECT_GO_TOOLCHAIN`, `SUSPECT_SPHINX_PYTHON` |
| TypeScript | `--test typescript typescript_applicators::` | `--test typescript typescript_resources::` | `SUSPECT_NODE24_BIN`, `SUSPECT_DOCS_NODE`, `SUSPECT_CHROMIUM` |

All TypeScript cases are native and `#[ignore]`d. Rust and Swift resource
behavior is described further in [SDK-RUST-RESOURCES.md](SDK-RUST-RESOURCES.md)
and [SDK-SWIFT-RESOURCES.md](SDK-SWIFT-RESOURCES.md); Ruby's in
[SDK-RUBY.md](SDK-RUBY.md).
