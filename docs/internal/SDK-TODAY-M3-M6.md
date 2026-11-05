# Hackathon scope: generation and output SDKs

Latest user decision, **2026-09-10**: “the hackathon demo is the generation and
the output sdks.” This adopts observational latency reporting for the demo.
The earlier sequencing decision brought M6 forward and included Swift and Rust
in M3 alongside Python/Go, with TypeScript/JavaScript as the regression baseline.

## Demo deliverables

- One canonical Contract-backed pipeline for Python, Go, Swift, Rust and TS/JS.
- Source-bound models/codecs, exact values, absence/null, references, recursion,
  native unions, mutable encode validation and explicit unsupported semantics.
- Installed native packages, independent create/update/list/get consumers and
  the five actual OpenRouter operation consumers.
- Native types, errors, cancellation, injectable transports, bounded runtime work,
  source-linked documentation and executable examples.
- Persistent sessions, correct source/configuration invalidation, changed-file
  writes, watch/preview and native/wire compatibility reports.
- Observed cold/warm/edit performance plus deterministic cache/work/output gates.

## Accepted evidence

The sealed `target/sdk-greenfield-native-verified-03/report.json` records all
**208 demo-relevant criteria as passing**. M3 is verified. M6's iteration tools
are delivered for this demo scope. The original full-profile result is retained:
208/212, with four numerical performance criteria unmet.

The complete replacement calibration collected five suites and 48,000 measured
refreshes. Work/size measurements were stable, but timing variability prevented
qualification under the proposed policy. No candidate collection ran. These
results remain observational, and no calibration job is running.

`xtask sdk-m3-m6 --demo` makes this acceptance scope explicit. Full calibrated
acceptance remains a separate path; the original numerical M6 exit is deferred.
Java, C#, Kotlin, Ruby, PHP, Dart and C++ remain paused drafts.

See [the runnable demo](SDK-DEMO.md), [progress](SDK-PROGRESS.md) and
[acceptance details](SDK-M3-M6-EXIT.md).
