# SDK generation progress

Updated **2026-09-10**. The hackathon deliverable is **generation and the output
SDKs**, with latency reporting observational by the user's latest decision.

## Current result

**The native SDK and iteration-tool demo scope is verified.** Five canonical
backends produce Python, Go, Swift, Rust and TypeScript/JavaScript packages.
Source fidelity, exact codecs, native clients/types/errors, docs/examples and
iteration behavior are covered by the sealed acceptance run.

| Scope | Result |
| --- | --- |
| M3 native backends | Verified: all native acceptance criteria pass |
| Hackathon generation/SDK/iteration scope | **208/208 relevant criteria pass** |
| Original full M3/M6 profile | **208/212**; four numerical M6 criteria remain unmet |
| Latency calibration | Five suites completed; timing noise prevented qualification |
| M4/M5 languages and protocol expansion | Paused/future work |

Run the [demo](SDK-DEMO.md) using `examples/sdk-demo.json`. The current CLI paths
are `codegen`, `codegen-session` and `codegen-compare`; `gen` provides Markdown
documentation and generic custom manifests. Legacy SDK implementations are removed.

## Verified generator and SDK behavior

- One owned, source-addressed Contract with exact values, references and HTTP metadata.
- Typed native plans, reusable component models, exact JSON/schema codecs and
  encode-time validation of mutable values.
- Installed native consumers for the independent M2 fixture and five real
  OpenRouter credits/key/container-file operations.
- Positive/negative types, request/response bytes, security/resource controls,
  cancellation, transport injection, native docs and executable examples.
- One Contract per accepted generation snapshot, finite content-addressed cache,
  target-only config reuse, cached reverts, missing-reference recovery, ownership
  protection and zero unchanged rewrites.
- CLI/editor watch, read-only previews and source-aware native/wire comparison.

The complete native matrix covers Python 3.11/3.14, Go 1.23/1.27, Swift 6.0.3 with
SDK 15.4 and Swift 6.3.3 with SDK 26.5, Rust 1.88/current, TS 5.5/5.9 and Node
22/24/browser consumers. Declared feature bounds remain in
[SDK-CAPABILITIES.md](SDK-CAPABILITIES.md).

## Evidence

- Sealed native report: `target/sdk-greenfield-native-verified-03/report.json`.
  SHA-256: `66684555f64e2721e893c71f035de6801390fcfa28ce1afb33559d5db07b84b5`.
- Its seal SHA-256:
  `05afcafb4e57a2854c6affb345d59cdb41a6bffe6e1fdc5743a183b9203f313b`.
- Earlier failed attempts `sdk-greenfield-native-verified-01` and `-02` remain
  immutable and record the tooling/isolation corrections.
- Review evidence: `target/sdk-m3-independent-runtime-review/` and
  `target/sdk-m3-m6-review-source/`. All final code-review findings were resolved.
- Current-scope assessment: `target/sdk-hackathon-demo-accepted/report.json`.
  This links the unchanged native report and applies the explicit demo scope;
  it does not rewrite the historical full-profile outcome.

## Performance disposition

`target/session-baseline-mac-v2-restart01/` contains five complete baseline suites,
**48,000 measured refreshes** collected over approximately **3 hours 27 minutes**.
Allocation and artifact-size checks were stable. All 144 timing dimensions failed
at least one qualification condition under the proposed policy; the independent
audit reproduced the result. The 677 overlapping reasons are qualification
failures, not SDK regressions. No candidate collection was started.

User decision: keep these timings observational for the hackathon. No calibration
job remains running. Original numerical M6 qualification requires prospectively
improved controls or an agreed policy tested on fresh held-out measurements.
See `target/sdk-calibration-restart01/analysis-diagnosis.md` and
[performance tooling](SDK-SESSION-PERFORMANCE.md).

## Remaining full-plan work

Java, C#, Kotlin, Ruby, PHP, Dart and C++ remain unverified paused drafts.
Broader media/uploads/streaming/security/dialect support remains M4/M5 work.
There is no full-protocol or production-release claim.

The tracked `openrouter-web` corpus remains read-only; its independent source
validation findings remain visible. The implementation is still in the existing
uncommitted checkout. [The handoff](SDK-SESSION-HANDOFF.md) records paths and state.
