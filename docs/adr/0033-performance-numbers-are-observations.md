# SDK performance numbers are observations, not gates, unless qualified on a dedicated runner

Hosted CI runs the session benchmark as a functional check only (`--functional-only`). Timings and native costs are recorded as raw observations with no threshold. A regression verdict requires a qualified baseline (at least 5 runs of 200 samples, exact-binomial p95 confidence intervals, drift and noise limits) collected on a self-hosted runner whose declared identity matches its measured inventory (`.github/workflows/sdk-session-performance.yml`, `tools/sdk-session-perf/`). A candidate passes only when every confidence upper bound is under the baseline limit, and `inconclusive` fails. Shared hosted runners are too noisy for a simple threshold, which would both pass and fail spuriously.

## Consequences

No qualified baseline exists yet (the 2026-09-10 attempt failed qualification on noise), so the repo has no latency budget or regression gate today. Precision is improved by collecting under a new versioned protocol, never by loosening tolerances against the candidate being judged.
