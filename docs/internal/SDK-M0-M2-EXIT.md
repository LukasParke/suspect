# Historical M0–M2 exit checkpoint

This page indexes the completed original milestone report and its evidence.
It is historical context for the current five-profile SDK pipeline. The approved
native interface baseline remains in [SDK-M0-M2-DX-APPROVAL.md](SDK-M0-M2-DX-APPROVAL.md).

**Completed 2026-09-09:** all **24 consolidated criteria pass** in
`target/sdk-m0-m2-verified/report.json`. The comparator-only post-review
hardening also passes all **six supplemental checks** in
`target/sdk-m0-m2-post-review-verified/report.json` (linked by the original
report's digest). Current post-cleanup integration is pending; see
[SDK-PROGRESS.md](SDK-PROGRESS.md) and [SDK-M3-M6-EXIT.md](SDK-M3-M6-EXIT.md).

The original criteria, command outcomes, known-failure classifications and
source/tool fingerprints are stored in the report. The failure inventory is
indexed in [SDK-M0-KNOWN-FAILURES.md](SDK-M0-KNOWN-FAILURES.md); workflow samples
are summarized in [SDK-M0-WORKFLOW.md](SDK-M0-WORKFLOW.md).

## Archived evidence

| Gate | Result |
| --- | --- |
| Workspace tests/doctests | 968 passed, 146 groups; 153 opt-in tests omitted by the default run |
| Contract/independent YAML oracle | 56 checks, six suites; all four tracked inputs plus independent vectors |
| Ownership | Seven checks, including real OpenRouter byte/mtime/inode stability |
| Examples | 12 checks across source planning, installed native execution and docs-only regressions |
| Shared native vertical | TS/JS + Rust pass on current and Rust 1.88 floor; TS 5.5.4/5.9.3, actual TypeDoc/Rustdoc and negative types |
| TS additional native checks | 36 checks in nine suites; includes directional packages, Node 22/24 and package/bundle checks |
| Rust additional native checks | 29 current checks plus four floor checks; exact codecs, HTTP, docs, security and optional Serde |
| Browser | Isolated Chromium executes actual getCredits and pre-aborted SDK calls |
| Entry points | 13 CLI checks, 18 pinned comparison/runner checks and two real editor-helper flows |
| Quality | All-target Clippy, warnings-denied Rustdoc, formatting and patch whitespace pass |
| Full corpus | 87 pass / 11 known fail / 98 stages; all 42 canonical stages pass; integrity checks pass |

Counts overlap and are not a unique-coverage total. The separate full-corpus
acceptance result was false in this historical run. Its classification was
recorded separately from raw command success. The first consolidated attempt
failed because an ownership test lacked its corpus-path variable; the corrected
historical verifier and all seven ownership checks passed. These counts must
not be substituted for fresh post-cleanup results.

Frozen CLI: `target/sdk-m0-m2-verified/bin/suspect`.
SHA-256: `4b53c87c79ce86388be8de0814275f07354b11198eddbc440e84ebfc27112417`.
Independent review reports and dispositions: `target/sdk-m0-m2-review-source/`.
