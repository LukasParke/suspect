# SDK release checklist

The publication runbook for **one generated SDK package**, one backend at a
time. Every command and file name below was verified against this tree on
2026-09-15; package metadata claims come from an empirical twelve-backend
generation run (methodology in [§7](#7-how-this-checklist-was-verified)). It
complements the program plan
([SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md) M9) and the
capability evidence in [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md).

One hard rule governs the program: the pinned byte fixtures across all twelve
backends must stay valid — publishable packages may only change through
regeneration, never through hand edits of generated output.

## 1. Generation inputs

One package = one `(spec, sdk_defaults, TargetConfig)` triple. Everything the
package later does is decided by these inputs; record all three with the
release.

### Source spec

- An OpenAPI 3.0/3.1/3.2 document (2.0 is detected but is not an SDK frontend
  yet — see [SDK-OPENAPI-MATRIX.md](SDK-OPENAPI-MATRIX.md)), loaded through
  the source-addressed `Contract` with its full reference closure.
- The single-target CLI takes `--spec FILE` or a pinned-source `--pins`
  manifest. A frozen-source release must pin (`suspect gen api.yaml
  --manifest FILE` captures pins) so regeneration reproduces the same
  contract bytes; `crates/suspect-codegen/tests/pinned_generation.rs` gates
  pin-driven sessions.
- Session-based generation (`suspect codegen-session --config session.json`)
  takes a JSON configuration with `spec` **or** `pins`, plus `targets`,
  `operation_ids`, `credential_env`, and `sdk_defaults`
  (`crates/suspect-cli/src/commands/codegen_session.rs`).

### `sdk_defaults` (the golden-defaults policy)

- **Available through the session JSON `sdk_defaults` field and the library
  `GenerationOptions.sdk_defaults` — not through `suspect codegen` flags.**
  The single-target `codegen` command forwards only compatibility profiles
  today (`crates/suspect-cli/src/commands/codegen_cmd.rs`); a package with
  pagination helpers, the automatic `<ENV_PREFIX>_API_KEY` convention, or
  OAuth lifecycle code must be generated via a session JSON (or the library
  API `suspect_codegen::backend::generate_with_options`).
- The policy is `sdk_defaults` v1: closed fields, unknown values are errors,
  every accepted value participates in the generation fingerprint. See the
  field reference in [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md) §1.

### `TargetConfig` identity

| Field | Rules (verified per backend) |
| --- | --- |
| `backend` | One of the twelve registered profiles; list with `suspect codegen-profiles` (`Backend::ALL`). |
| `package_name` | TypeScript/JavaScript: npm `@scope/name`. Python: distribution name (import name derived `_`-normalized). Go: **module path**; the Go package name is fixed `sdk`. Rust: crate name. Swift: module name. Java and Kotlin: Maven `group:artifact` (enforced — `backend.rs` refuses anything else). PHP: `vendor/package`. C#: namespace segments from dotted names. Dart and C++: package name. |
| `package_version` | Exact package SemVer, independent of the OpenAPI `info.version`. It feeds the `ua/v1` identity slot, every package manifest, and the compatibility report. |
| `import_name` | Optional native identity override for Python (import name), Swift (module), JVM (package), Ruby/C#/PHP/C++ (namespace). **Rejected** by TypeScript, Rust, Go and Dart — their identity is the package name. |

Identity mistakes are the cheapest kind to catch: run
`suspect codegen --check --format json` before writing, and review the
resulting attribution identity (§2) before publishing, because it is visible
on the wire in every request.

## 2. Pre-publish gates (all exist today)

Run from the repository root. Everything is `cargo test --locked --offline`
unless a gate needs its native toolchain; suites degrade to skipped/ignored
tests when a toolchain is absent, so a green run always states what ran.

### Attribution (`ua/v1`)

- `cargo test -p suspect-codegen --test sdk_attribution` — compiles the
  attribution descriptor and runtime for TypeScript, Python, Go and Rust
  packages (suspect version, SDK identity, spec version, language tag).
- `cargo test -p suspect-codegen --test swift_attribution --test java_attribution
  --test kotlin_attribution --test csharp_attribution --test ruby_attribution
  --test php_attribution --test dart_attribution --test cpp_attribution` —
  the eight remaining per-language attribution suites (options, sentinel
  packages without a descriptor, runtime resolution).
- `cargo test -p suspect-codegen --test sdk_compatibility_attribution` —
  attribution and `sdk_defaults` participate in compatibility capture.

### Pagination (zero-config detection + native behavioral gates)

- Planner/explanations: `cargo test -p suspect-codegen --test sdk_pagination`.
- One native suite per backend, each with behavioral tests that execute the
  emitted helpers where a toolchain exists on the host:
  `typescript_pagination.rs`, `python_pagination.rs`, `go_pagination.rs`,
  `rust_pagination.rs`, `swift_pagination.rs`, `java_pagination.rs`,
  `kotlin_pagination.rs`, `csharp_pagination.rs`, `ruby_pagination.rs`,
  `php_pagination.rs`, `dart_pagination.rs`, `cpp_pagination.rs`
  (`crates/suspect-codegen/tests/`).

### OAuth lifecycle

- Planner: `cargo test -p suspect-codegen --test sdk_oauth_plan`.
- Native suites: `typescript_oauth.rs`, `python_oauth.rs`, `go_oauth.rs`,
  `rust_oauth.rs`, `swift_oauth.rs`, `java_oauth.rs`, `kotlin_oauth.rs`,
  `csharp_oauth.rs`, `ruby_oauth.rs`, `php_oauth.rs`, `dart_oauth.rs`,
  `cpp_oauth.rs` — each runs the emitted token lifecycle natively where the
  toolchain exists on the host.

### Typed SSE

- Plan compilation: `cargo test -p suspect-codegen --test sdk_stream_plan`.
- Native suites: `typescript_stream_typed.rs`, `python_stream_typed.rs`,
  `go_stream_typed.rs`, `rust_stream_typed.rs`, `swift_stream_typed.rs`,
  `java_stream_typed.rs`, `kotlin_stream_typed.rs`, `csharp_stream_typed.rs`,
  `ruby_stream_typed.rs`, `php_stream_typed.rs`, `dart_stream_typed.rs`,
  `cpp_stream_typed.rs`.

### Credential environment (byte-pinned)

- Shared policy suite: `cargo test -p suspect-codegen --test credential_env`.
- One suite per backend, each pinning or recording emitted artifact bytes
  (SHA-256) with explicit no-policy byte-stability tests:
  `typescript_credential_env.rs`, `python_credential_env.rs`,
  `rust_credential_env.rs`, `go_credential_env.rs` (+
  `go_credential_env_canonical.rs`, `go_credential_env_factory_capture.rs`),
  `swift_credential_env.rs` (+ `swift_credential_env_canonical.rs`),
  `java_credential_env.rs`, `kotlin_credential_env.rs`,
  `ruby_credential_env.rs`, `php_credential_env.rs`,
  `dart_credential_env.rs`, `cpp_credential_env.rs`. The C++ suite drives the
  installed native client instead of pinning bytes.
- **C# exception:** there is no `tests/csharp_credential_env.rs`; the C#
  coverage is an in-crate test module run with
  `cargo test -p suspect-codegen --lib`
  (`src/csharp_sdk/credential_env_tests.rs`).

### Elimination gates

Per [SDK-ELIMINATION-AUDIT.md](SDK-ELIMINATION-AUDIT.md) (which records the
measured numbers and the honest defeats):

```sh
cargo test -p suspect-codegen --test typescript_elimination -- --nocapture
cargo test -p suspect-codegen --test swift_elimination     -- --nocapture
cargo test -p suspect-codegen --test cpp_elimination       -- --nocapture
cargo test -p suspect-codegen --test java_elimination      -- --nocapture
cargo test -p suspect-codegen --test kotlin_elimination    -- --nocapture
cargo test -p suspect-codegen --test csharp_elimination    -- --nocapture
cargo test -p suspect-codegen --test ruby_elimination      -- --nocapture
cargo test -p suspect-codegen --test php_elimination       -- --nocapture
```

Go and Rust are gated by the real `go build`/`cargo build` consumer
measurements recorded in the audit; Dart's gate is mechanism-only pending a
CI toolchain. Every per-language gate writes its machine-readable record to
`target/tmp/<language>-elimination-report.json`.

### Native costs

- `cargo test --locked --offline -p suspect-codegen --test sdk_native_measurements`
  — the host/adversarial checks for the `repeated-v1` methodology. The
  actual native collection is the ignored entrypoint, run only with all six
  `SUSPECT_SDK_FULL_*` variables exported by the `sdk-full` runner (see
  [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md)); a release claim about cost
  numbers requires that fresh integrated run.

### Determinism and drift

- `cargo test -p suspect-codegen --test artifact_safety` and
  `--test pinned_generation` — output sets are deterministic and
  source-pin-driven.
- Regenerate the candidate and run the same generation with
  `suspect codegen --check --format json` (or `codegen-session --check`):
  ownership-aware drift must be empty before anything is published.

## 3. Package metadata the emitters already produce

Verified empirically by generating one small package per backend (one JSON
spec, two operations, `sdk_defaults` with `env_prefix` + auto pagination, one
`TargetConfig` per backend). Emitted package metadata per backend:

| Backend (profile) | Package metadata emitted | Notes |
| --- | --- | --- |
| TypeScript/JS (`typescript-http`) | `package.json`, `package-lock.json`, `tsconfig.json`, `tsconfig.docs.json`, `typedoc.json` | ESM `exports` subpaths (`/operations`, `/models`, `/codecs`, `/json`); `files` allowlist; `engines.node >= 22`; `suspect` metadata block with toolchain pins (Node/npm/TypeScript), `status: "prototype"`, `releaseReady: false`. |
| Python (`python-http`) | `pyproject.toml` (+ `py.typed` marker) | hatchling build backend, `requires-python`, dependency pins; `src/<import>/` layout; in-package `manifest.json` records `release_ready: false` and the OpenAPI version. |
| Go (`go-http`) | `go.mod` | Module path = `package_name`, `go 1.23`; no sum file is emitted (zero dependencies — one is generated only if a consumer adds deps). |
| Rust (`rust-http`) | `Cargo.toml` | Feature-gated (`http`, `reqwest-rustls`), exact `=`-pinned optional deps, `rust-version = "1.88"`, `publish = false` by default; `Cargo.lock` is created by the first local build, not emitted. |
| Swift (`swift-http`) | `Package.swift` (+ `sdk-manifest.json`) | tools 6.0, language mode v6, platform floors, DocC bundle; no `Package.resolved` (zero external deps). |
| Java (`java-http`) | `pom.xml` (+ `sdk-manifest.json` resource) | `maven.compiler.release 21`, zero runtime dependencies, pinned plugin versions, javadoc/source plugins bound, reproducible `outputTimestamp`. |
| Kotlin (`kotlin-http`) | `pom.xml` | Kotlin 2.4.20 + coroutines 1.11.0 exactly, JVM release 21, Dokka coverage via `docs/`. |
| C# (`csharp-http`) | `Suspect.csproj` | net8.0, zero `PackageReference`, embedded `protocol-plan.json`/`validation-program.json` resources, docs generation on. |
| Ruby (`ruby-http`) | `release-pack.gemspec` (+ `.yardopts`, `sig/*.rbs`) | required Ruby version, the six stdlib gem dependencies with ranges, file lists. |
| PHP (`php-http`) | `composer.json` | classmap autoload, PHP `^8.3`, ext-json, phpstan dev tool, `suggest ext-curl`, `"license": "proprietary"`. |
| Dart (`dart-http`) | `pubspec.yaml` (+ `CHANGELOG.md`, `analysis_options.yaml`, `dartdoc_options.yaml`) | SDK constraint `>=3.9.4 <4.0.0`; portable `lib/<name>.dart` + `lib/<name>_io.dart` entrypoints. |
| C++ (`cpp-http`) | `CMakeLists.txt` (+ `cmake/<name>Config.cmake.in`, `Doxyfile`, `sdk-manifest.json`) | C++20 (`cxx_std_20`), libcurl adapter option, export config template for `find_package`. |

What is **not** emitted (and must not be invented at publish time): any
registry account data, signing material, or publish scripts. The generator
boundary is explicit — `backend::generate_with_options` performs no IO,
toolchain execution, or publishing
(`crates/suspect-codegen/src/backend.rs`).

## 4. License and provenance in the emitted manifests

Verified against the §3 generation run:

- **License:** the only emitted license declaration is PHP's
  `"license": "proprietary"` in `composer.json`. No other backend's manifest
  or README carries a license field today. Choosing and configuring the
  package license is a manual sign-off (§6); adding license metadata to the
  other eleven emitters is emitter work outside this checklist.
- **Generator version** (`ua/v1` first token): compiled into every package's
  attribution module (verified: Rust `ATTRIBUTION_SUSPECT_VERSION`,
  Go `userAgentSuspectVersion`, Swift `suspectVersion` — all
  `CARGO_PKG_VERSION` at generation time, currently `0.1.0`). The C++ and
  Swift `sdk-manifest.json` files also record it as `"generator"`.
- **Source provenance:** manifests carry the source contract's identity —
  the C++ manifest records `contractDigest` + `openapi` version, the Swift
  manifest `protocolProfile`, the Java manifest `openapiVersion`, the Python
  in-package `manifest.json` `openapi_version`, the TypeScript
  `docs-manifest.json` `openapiVersion` + `validationProfile`. Generated
  doc comments carry `OpenAPI source: <uri>#<json-pointer>` links back to
  the source document.
- **Release readiness flags:** packages are emitted with
  `releaseReady: false` / `status: "prototype"` (TypeScript package.json
  `suspect` block, Go/Python/Rust/C#/TypeScript `http-manifest.json`,
  Python `manifest.json`, Swift manifest "promotion" note). Promotion is a
  deliberate gate, not a default.
- **Toolchain pins visible in manifests** (use these when pinning CI):
  TypeScript `typescript 5.9.3`/`npm@10.9.8`/`node >= 22`; Python
  `hatchling==1.29.0`, `httpx==0.28.1`, `>=3.11`; Rust `rust-version 1.88` +
  exact dependency pins; Kotlin 2.4.20/1.11.0, Java 21 (same for Java-only);
  PHP `^8.3`, phpstan `2.2.13`; Dart `>=3.9.4 <4.0.0`; Ruby `>= 3.3.12`;
  Swift tools 6.0; C++ CMake 3.24 / C++20; Go 1.23. The full maintained
  toolchain matrix is [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md).

## 5. Compatibility report as a release gate

The typed compatibility machinery is
`crates/suspect-codegen/src/compatibility.rs`:

- `compatibility::snapshot_with_options(contract, operation_ids, targets,
  generation)` — capture one side's wire + native surface under exact
  generation options (`sdk_defaults`, `credential_env`, profiles).
- `compatibility::compare_with_options(old, new, operation_ids,
  (old_targets, old_generation), (new_targets, new_generation))` — joint
  selection resolution (renames via method/path, removals/additions) and a
  `CompatibilityReport` with per-finding impact and migration text.
- `compatibility::compare_snapshots(old, new)` — compare retained plans
  without replanning.

The release gate: keep the **previously published** generation's session JSON
(same targets, operation selection, `sdk_defaults`) frozen as `before`;
produce the candidate's session JSON as `after`; run

```sh
suspect codegen-compare --before published.session.json \
                        --after candidate.session.json --format json
```

Exit 0 = proved compatible within its recorded scope; 1 = any
breaking/potentially breaking/unknown finding (triage each, write the
migration note); 2 = input/configuration failure. The same API is exercised
by `tests/sdk_compatibility.rs`, `tests/sdk_compatibility_attribution.rs`,
`tests/sdk_compatibility_context.rs`, `tests/sdk_generation_options.rs`, and
`tests/typescript_protocol_compatibility.rs`, and by
`docs/SDK-COMPATIBILITY-REPORTS.md`.

## 6. Manual sign-offs that remain

These are not automatable from this repository today; each is a human
check with a place to record the decision:

1. **Toolchain pins.** Confirm the publishing host matches the maintained
   tiers in [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md) (Node 22/24,
   Python 3.11/3.14, Rust 1.88/stable, Go 1.23/1.27, Swift 6.0/6.3,
   JDK 21/25, .NET 8/10, Kotlin 2.4.20, Ruby 3.3/4.0, PHP 8.3/8.5,
   Dart 3.9/3.13, Clang 21). CI-pending gates called out in
   [SDK-ELIMINATION-AUDIT.md](SDK-ELIMINATION-AUDIT.md) (Rollup/Vite byte
   gates, Java/Kotlin R8 shrink, Dart AOT, `sdk_defaults`-enabled Go/Rust
   consumer gates) must either run in CI or be explicitly waived.
2. **Registry accounts and publishing mechanics.** npm, PyPI, crates.io, Go
   module tagging, SwiftPM source hosting, RubyGems, NuGet, Maven Central,
   pub.dev, Packagist, and the C++ distribution choice each need an account,
   trusted publishing/2FA, and a first-publish path. Nothing in the repo
   performs publication — the generator emits packages only.
3. **License decision.** Pick the license per package and record it; today
   only PHP's manifest declares one (`proprietary`). This is an emitter
   change if per-package licensing should be configurable.
4. **Release-readiness promotion.** Manifests ship `releaseReady: false` /
   `status: "prototype"`; flipping to a release claim is a reviewed decision
   with the §2 gates green.
5. **Identity review.** `package_name`/`package_version` (the `ua/v1`
   identity slot is visible on every wire request), `env_prefix` (public
   variable names), and `import_name` choices.
6. **Compatibility review.** The §5 report for every changed source, with
   migration notes for each finding, archived beside the published release.
7. **Reproducibility.** A second regeneration from the same frozen inputs
   (pinned source, same options) must be drift-free (`--check`).

## 7. How this checklist was verified

- Every gate command maps to an existing test file (listed with real paths);
  `sdk_attribution`, `typescript_credential_env` (byte-pin test), and
  `swift_attribution` were executed green during verification.
- The §3/§4 inventory came from one throwaway Rust example under
  `crates/suspect-codegen/examples/` that called
  `backend::generate_with_options` for all twelve `Backend`s with a two-spec
  fixture and `sdk_defaults`, writing every emitted file under
  `target/sdk-release-packaging-verify/` (kept out of the tracked tree);
  the example was deleted after the run and no generated byte outside
  `target/` was touched.
- Go module path / Maven coordinates enforcement and `import_name` refusals
  were read from `crates/suspect-codegen/src/backend.rs` (the same rules the
  §3 table states).

## References

- [SDK-GOLDEN-DEFAULTS.md](SDK-GOLDEN-DEFAULTS.md) — user guide.
- [SDK-GOLDEN-DEFAULTS-PLAN.md](SDK-GOLDEN-DEFAULTS-PLAN.md) — program plan,
  M9 definition.
- [SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md) — credential runtime
  semantics.
- [SDK-OPENAPI-MATRIX.md](SDK-OPENAPI-MATRIX.md) — normative feature matrix.
- [SDK-ELIMINATION-AUDIT.md](SDK-ELIMINATION-AUDIT.md) — elimination gates
  and their CI-pending remainder.
- [SDK-NATIVE-COSTS.md](SDK-NATIVE-COSTS.md) — `repeated-v1` methodology and
  maintained toolchain tiers.
- [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md) — capability evidence matrix.
- [SDK-COMPATIBILITY-REPORTS.md](SDK-COMPATIBILITY-REPORTS.md) — the
  comparison report surface behind §5.
