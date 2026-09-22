# Native scoped-validation adoption

The shared v2 applicator engine is implemented. The original-plan next gate is
native execution and model/codec admission in each of the twelve SDK runtimes.
All earlier v1 language/protocol reports retain their original scope and bytes.

## Authoritative contract and fixtures

- [Executable semantics](SDK-SCHEMA-APPLICATORS.md), including exact annotation
  contributions, branch isolation, evaluation failures and visit/set-merge costs.
- `OwnedCompiler::compile` retains v1 admission.
- `OwnedCompiler::compile_v2` admits the nine new operations and emits v2 only
  when they are needed. Ordinary base closures retain v1 representation.
- `ProgramInstruction::requires_v2()` and the exact `OwnedProgram` version/profile
  constants support admission fences and native dispatch.
- Maintained independent vectors:
  `crates/suspect-schema/tests/fixtures/owned-applicators-v2.json` (32 cases).
- Frozen executable witness programs:
  `target/sdk-schema-applicators-executable-v2.json`.

Maintained native tests must compile the source fixture through the real
`compile_v2` API. A test must not depend on an earlier `target/` witness report
being present. The frozen program fixture is useful for independent one-off
execution and review; it does not replace the maintained source-driven seam.

## Per-runtime completion

1. Preserve v1 guards and execution. Before v2 support, refuse unwitnessed
   instructions at their source before emitting a package. Unknown versions,
   mismatched profiles and malformed operands never become valid programs.
2. Implement all nine instructions and scoped evaluated-property/item sets.
   Child scopes are fresh; only the documented successful annotations propagate.
   Numeric/equality/work failures remain noninvertible and shared across trials.
3. Verify the 32 independent vectors, including exact source/instance locations
   and failure-versus-invalid distinctions. Add native-specific ownership,
   mutation, key-identity, recursion, cancellation or resource controls where
   needed. Keep exact numbers and omission/null behavior.
4. Admit faithful native model/codec representations. Pattern-matched extras may
   not be dropped or treated as forbidden by an old additional-properties model.
   A runtime-validated JSON-value carrier is appropriate where static native
   types cannot express the constraint; it must remain checked on decode/encode.
5. Exercise actual SDK operations using the new schemas, positive/negative native
   consumer types, generated examples and rendered symbol documentation on each
   declared toolchain tier. Enable `compile_v2` in the native planner only after
   these witnesses pass; model-only APIs retain explicit codec obligations.
6. Retain new source/tool/package/log evidence, update typed compatibility records
   and give Main the complete new production asset list. Reuse unaffected v1
   matrices; do not restart them solely because the server restarted.

Each existing language owner keeps ownership of its validator, model/codec
admission, tests, documentation and compatibility capture. Main owns shared
registry/options/provenance/CLI/editor/acceptance. Shared IR/schema owners retain
their modules; native owners consume those public seams.

## Resource and dynamic scope follow-up

[Canonical resource indexing](SDK-CONTRACT-RESOURCES.md) and the explicit
[owned v3 resource/dynamic program](SDK-SCHEMA-RESOURCES.md) are implemented and
verified. The public crate re-exports `ProgramResource` and
`ProgramResourceContext`. `compile`/`compile_v2` retain their semantics and omit
resource metadata; `compile_v3` explicitly emits the new envelope.

Each existing native owner now adopts that profile after its v2 proof. Maintained
tests compile the44 unmodified source cases and supplied remote documents in
`crates/suspect-schema/tests/fixtures/resource-conformance/` through actual
`compile_v3`. The executable file under `target/` remains a one-off witness.
Dynamic binding must use actually entered resources and exact context-aware
cycle identity; it must never become an initial-target static guess. Native
model/codec, installed SDK, type/example/doc, work/ownership and malformed-program
checks remain required before enabling resource capabilities.

## Completed native v2 receipts

These are bounded native-profile results on both declared toolchain tiers. The
final integrated runner remains a separate current-source gate.

| Native target | Authoritative receipt / guide |
| --- | --- |
| Rust | `docs/SDK-RUST-VALIDATION-V2.md` —10 tests,395 official cases, installed consumers and existing-v1 parity |
| Swift | `docs/SDK-SWIFT-VALIDATION-V2.md` —54 runtime cases,10 installed SDK cases, typing/examples/DocC |
| Ruby | `target/sdk-ruby-schema-v2-verification/report.json` —32 shared +13 controls,21 guards, installed gem/types/YARD |
| C# | `target/sdk-csharp-schema-v2-evidence-20260910/report.json` —538 runtime/SDK checks per tier, installed NuGet/examples/types |
| Dart | `target/sdk-dart-v2-20260910/report.json` —59 cases,237 budget assertions, installed VM/JS/browser SDK/types/dartdoc |
| Python | `target/sdk-python-scoped-completion-01/completion.json` —32 shared +13 controls,20 guards,12 installed operations, typing/Sphinx |
| Java | `target/sdk-java-validation-v2/REPORT.md` —32 shared +8 controls,22 guards, installed Maven/types/Javadoc |
| PHP | `target/sdk-php-validation-v2-verified-20260910-01/report.json` —32 shared +12 controls, installed Composer/types/examples |
| Kotlin | `target/sdk-kotlin-applicators-verified/REPORT.md` —32 shared +32 controls, installed JVM consumers and Dokka on JDK21/25 |
| C++ | `target/sdk-cpp-v2-verification-20260910/report.json` —32 shared +12 controls, installed CMake/Doxygen/guide SDK and independent libcurl exchanges |
| Go | `docs/SDK-GO-SCHEMA-V2.md` and `target/sdk-go-native-schema-evidence-20260910/receipts.json`; source vectors, installed SDK/types/docs and context/race witnesses |
| TypeScript | `docs/SDK-TYPESCRIPT-VALIDATION-V2.md` and `target/sdk-typescript-v2-final-01.log` —32 source vectors, installed floor/current Node/TS consumers, native docs/browser |

All twelve native v2 handoffs are now integrated. Reuse earlier completed native
matrices rather than restarting them to update this index.

## Completed native resource/dynamic v3 receipts

All twelve adapters now have bounded native v3 proof. Ordinary v1/v2 programs
retain their meanings, and resource/dynamic execution uses the explicit v3
envelope. Canonical integration is checked separately from each native handoff.

| Native target | Authoritative receipt |
| --- | --- |
| Ruby | `target/sdk-ruby-schema-v3-verification/report.json`; physical servers: `target/sdk-ruby-document-servers-verification/report.json` |
| Dart | `target/sdk-dart-v3-20260910/report.json`; physical servers: `target/sdk-dart-document-base-20260910/report.json` |
| Python | `target/sdk-python-resources-completion-01/completion.json`, including final physical/credential-base witnesses |
| PHP | `target/sdk-php-validation-v3-verified-20260910-01/report.json`, with separately retained physical-server proof |
| Kotlin | `target/sdk-kotlin-resources-verified/REPORT.md`, including JDK21/25 physical-server and installed SDK/Dokka checks |
| C# | `target/sdk-csharp-schema-v3-evidence-20260910/report.json` —453 runtime/SDK checks per .NET tier and public/installed package parity |
| C++ | `target/sdk-cpp-v3-verification-20260910/report.json` and its source/runtime inventories; physical servers have a separate retained report |
| Go | `docs/SDK-GO-SCHEMA-V3.md` and `target/sdk-go-native-schema-evidence-20260910/receipts.json`, including contextual codecs, installed SDK/race/docs and grouped examples |
| Java | `target/sdk-java-validation-v3/REPORT.md`; native resource/SDK gates and explicit canonical `plan_sdk_with_protocol_v3` |
| TypeScript | `target/sdk-typescript-v3-acceptance-01.json` and `docs/SDK-TYPESCRIPT-VALIDATION-V3.md`, including installed Node/TS, typed capture, TypeDoc and Chromium |
| Rust | `docs/SDK-RUST-RESOURCES.md` and `target/sdk-rust-resources-20260910-01/evidence.json`; five native resource/SDK/physical-server selectors on Rust1.88/1.97.1, explicit `plan_http_v3` |
| Swift | `docs/SDK-SWIFT-RESOURCES.md` and `target/sdk-swift-resources-20260910/`; 93 runtime cases per tier, installed SDK/type/wire/DocC and physical-server gates, existing `plan_sdk` |

Go's receipt explicitly distinguishes retained raw files from earlier managed
stdout that became unavailable after restart. C# likewise records missing outer
Cargo stdout as observed completion summaries while retaining native commands and
packages. Rust's initial current-tier official/scope runs likewise retain owner
completion summaries; eight other native command logs are retained and hashed.
Those entries are completion receipts, not complete raw transcripts.
The required fresh integrated run supplies its own sealed command/log evidence.

Main's Swift receipt verifies all 78 emitted validation-runtime files against the
current include bytes and the exact maintained SDK header, plus 22 retained native
command logs: `target/sdk-main-final-integration-20260910-01/swift-v3-runtime-parity-03.json`.
Its generation/session/capture bridge now passes for eleven adapters in
`canonical-resources-eleven-01.log` in the same directory. Rust's canonical v3
transition is awaiting the ordinary HTTP-manifest parity and matching capture
follow-up; the concrete red is `rust-v3-canonical-byte-01.log`. These individual
receipts do not replace the final frozen twelve-target run.
