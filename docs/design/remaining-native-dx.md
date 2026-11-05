# Remaining native SDK interfaces

The user approved these interface baselines on 2026-09-09 while authorizing
implementation of the original M3–M6 milestones. OpenAPI remains the complete
semantic input. Names shown below are source-derived examples, not new service
conventions.

The verified checkpoint covers Python, Go, Swift and Rust with the TS/JS
baseline. Java/C#/Kotlin/Ruby/PHP/Dart/C++ are in active implementation after the
full-plan instruction on 2026-09-10. The rows below are approved interface
baselines; verification and customer-facing polish are independent requirements.

| Target | Approved baseline |
| --- | --- |
| Python | Keyword-only typed arguments/dataclasses, UNSET distinct from None, sync/async clients and context managers, explicit typed exceptions and transport protocols |
| Go | context.Context first, typed operation inputs/results, native errors and presence wrappers, standard net/http transport seam |
| Java | Native immutable/value models and builders, typed failures, explicit presence, sync/CompletableFuture clients |
| Kotlin | Data/sealed models, named/default arguments where source-optional, explicit presence, coroutine call sites |
| C# | Nullable-aware models, Task, CancellationToken, HttpClient seam and explicit presence/failure values |
| Swift | Value models/enums, explicit absence/null, async/await, Sendable policy and URLSession seam |
| Ruby | Keyword calls/models, explicit unset/null and native exceptions/transport |
| PHP | Typed models/enums, presence and interoperable transport with PHPStan-compatible types |
| Dart | Null-safe typed models, explicit presence, Future/Stream and injectable HTTP |
| C++ | Value types/variants, explicit presence/errors, RAII and declared compiler/HTTP profiles |

Each implementation must prove installed-package imports, strict native types,
source-bound codecs, actual wire behavior and native documentation. Upload and
stream methods are exposed only after their source-backed wire profiles pass.
No pagination, retry, auth acquisition or business default is inferred from a
name. Version/toolchain pins and actual verified boundaries are recorded with
each target's native gates before promotion.
