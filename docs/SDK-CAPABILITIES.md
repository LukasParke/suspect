# SDK generation capabilities

SDK generation uses one pipeline: source-addressed `Contract` → typed native
plans → models, checked codecs, HTTP clients, packages and documentation →
ownership-aware artifacts. `codegen`, `codegen-session`, editor generation and
`codegen-compare` share the same backend registry.

All profiles are **experimental and bounded**. A selected contract must pass
the target's admission checks before artifacts are produced. Support in the
shared contract model does not imply support in every native adapter.

## Native profiles

All twelve profiles are enabled by the default build. Use
`suspect codegen-profiles --format json` to inspect the actual compiled registry.

| Language | Profile | Package and documentation | Details |
| --- | --- | --- | --- |
| Python | `python-http` | Python package, `py.typed`, docstrings and Sphinx | [Python HTTP](SDK-PYTHON-HTTP.md) |
| Go | `go-http` | Standard-library module, Go doc and Sphinx | [Go HTTP](SDK-GO-HTTP.md) |
| Swift | `swift-http` | Swift Package Manager and DocC | [Swift](SDK-SWIFT.md) |
| Rust | `rust-http` | Cargo package, Rustdoc and doctests | [Rust HTTP](SDK-RUST-HTTP.md) |
| TypeScript / JavaScript | `typescript-http` | ESM package, declarations, TSDoc and TypeDoc | [TypeScript HTTP](SDK-TYPESCRIPT-HTTP.md) |
| Java | `java-http` | Maven and Javadoc | [Java](SDK-JAVA.md) |
| C# | `csharp-http` | NuGet, compiler XML documentation and native reference pages | [C#](SDK-CSHARP.md) |
| Kotlin | `kotlin-http` | Maven and Dokka | [Kotlin](SDK-KOTLIN.md) |
| Ruby | `ruby-http` | Gems, YARD, RBS and Steep | [Ruby](SDK-RUBY.md) |
| PHP | `php-http` | Composer, PHPStan and PHPDoc/reference | [PHP](SDK-PHP.md) |
| Dart | `dart-http` | pub packages and dartdoc | [Dart](SDK-DART.md) |
| C++ | `cpp-http` | CMake, libcurl and Doxygen | [C++](SDK-CPP.md) |

Package identity is explicit configuration. `--import-name` supplies native
module/package/namespace overrides for Python, Swift, JVM languages, Ruby, C#,
PHP and C++. TypeScript, Rust and Dart derive import identity from the package
name; Go uses package `sdk` within its configured module.

## Contract and runtime scope

| Area | Shared capabilities |
| --- | --- |
| Input | Local/split documents and explicitly acquired pinned offline closures; physical source URI, pointer and span provenance |
| Dialects | OpenAPI 3.0, 3.1 and 3.2 context with dialect-aware owned schema compilation; OpenAPI 2.0 family detection is not an SDK frontend |
| Values and models | Explicit absence/null, exact numeric values, source wire names, typed extras, recursive graphs, unions/intersections and encode-time validation |
| Validation | Checked owned programs, exact operands, finite evaluation budgets, scoped applicators and resource/dynamic profiles admitted per native adapter |
| HTTP | Source-backed methods, servers, security, parameter encodings, status dispatch, JSON/text/binary/bodyless media, forms, multipart and item streams |
| Interpretation | Standard declarations by default; explicit `legacy-binary-string-v1` compatibility profile for legacy binary markers |
| Examples | Source/synthesized provenance, located invalid-example findings and schema-valid values lowered through native symbol/codec plans |
| Artifacts | Deterministic file sets, ownership conflicts, read-only drift checks, unchanged-file preservation and removal of unedited obsolete owned files |
| Iteration | Shared Contract per accepted snapshot, source/configuration fingerprints, bounded caching, target-specific reuse and watch/check/preview |
| Compatibility | Separate source-aware wire/native findings, typed descriptors, asset provenance and explicit uncertainty |

Native profiles reject unsupported selected features with source-linked
diagnostics. Consult the language guides for exact admission boundaries.

## Generate a package

The in-repository create/update/list/get fixture provides a self-contained start:

```sh
suspect codegen crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile typescript-http --package-name @example/widgets \
  --package-version 0.1.0 --out generated
```

Repeat `--operation-id NAME` to select exact source operations; omit selectors
to attempt all outgoing operations. Add `--check --format json` for read-only
ownership/drift checking. Generation does not invoke native package managers.

## Further reading

- [Compiler architecture and migration](SDK-GENERATION-PLAN.md)
- [Multi-target sessions](SDK-INCREMENTAL-GENERATION.md),
  [editor preview](SDK-EDITOR-PREVIEW.md) and
  [compatibility reports](SDK-COMPATIBILITY-REPORTS.md)
- [Contract readers](SDK-CONTRACT-READERS.md),
  [contract resources](SDK-CONTRACT-RESOURCES.md) and
  [pinned acquisition](SDK-PINNED-CLOSURE-DESIGN.md)
- [Owned validation](SDK-OWNED-VALIDATION.md),
  [schema dialects](SDK-SCHEMA-DIALECTS.md) and
  [HTTP protocol](SDK-HTTP-PROTOCOL.md)
- [Examples](SDK-EXAMPLES.md), [artifact ownership](SDK-ARTIFACT-OWNERSHIP.md)
  and [session measurement](SDK-SESSION-PERFORMANCE.md)
