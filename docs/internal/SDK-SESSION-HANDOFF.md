# SDK session handoff — hackathon demo

**Current active integration/recovery state:** read
[SDK-FULL-PLAN-STATUS.md](SDK-FULL-PLAN-STATUS.md) first. It records completed
full-plan evidence, current owners and resumed work after server interruptions.

Updated **2026-09-10**. Latest user decision: **“the hackathon demo is the
generation and the output sdks.”** Latency is observational for the demo;
additional calibration is not part of the current task.

## Latest steering — full original plan resumed

The user subsequently requested implementing the remaining languages and other
original-plan items end to end. Read `SDK-FULL-PLAN-WORK.md` first for the new
active work map. The hackathon checkpoint below remains verified history; its
paused-language/no-active-work statements are superseded. Preserve the new source
entry snapshot `target/sdk-full-plan-entry-20260910/` and all prior evidence.
Legacy SDK paths remain removed. Native language work and shared protocol/schema
expansion resume; numerical performance needs improved controls before fresh gates.

## Current state

- Five canonical profiles: Python, Go, Swift, Rust and TypeScript/JavaScript.
- One Contract-backed pipeline; legacy SDK emitters/templates, flags and old
  comparison/projection APIs are removed.
- Native packages, types, exact codecs, HTTP behavior, errors, cancellation,
  source-linked docs/examples and iteration tools are verified for the selected
  JSON HTTP profile. Broader protocol/dialect support remains explicit future work.
- M3 is verified. M6's session/watch/preview/compatibility tooling is delivered for
  the hackathon. The original strict numerical M6 exit remains deferred.
- `xtask sdk-m3-m6 --demo` explicitly selects 208 native/functional criteria;
  default full-calibrated acceptance retains all 212. Demo reports cannot claim
  calibrated latency. See [the demo guide](SDK-DEMO.md).
- Java, C#, Kotlin, Ruby, PHP, Dart and C++ are paused drafts.
- No calibration job or implementation subagent remains running.

## Authoritative evidence

- `target/sdk-greenfield-native-verified-03/report.json`: **208/212** under the
  original full profile; every M3 and iteration criterion passed. Only the four
  numerical performance rows were unmet.
- Report SHA-256:
  `66684555f64e2721e893c71f035de6801390fcfa28ce1afb33559d5db07b84b5`.
- Seal SHA-256:
  `05afcafb4e57a2854c6affb345d59cdb41a6bffe6e1fdc5743a183b9203f313b`.
- `target/sdk-hackathon-demo-accepted/report.json`: scope assessment linking the
  unchanged sealed run to the user-approved **208/208 demo** criteria.
- Reviews and preserved fixes: `target/sdk-m3-independent-runtime-review/` and
  `target/sdk-m3-m6-review-source/`. All final code-review findings were resolved.
- Earlier failed attempts `sdk-greenfield-native-verified-01` and `-02` remain
  immutable. They record corrected tool-cache, isolation and Swift SDK selection
  issues, rather than current SDK failures.

The native matrix covers Python 3.11/3.14, Go 1.23/1.27, Rust 1.88/current,
Swift 6.0.3 + SDK15.4 and 6.3.3 + SDK26.5, TS5.5/5.9 and Node22/24/browser.
Source/tool/input fingerprints, package archives and native logs are in the report.

## Performance disposition

The original `target/session-baseline-mac-v2/` campaign was interrupted by a server
restart and lost its outer execution ledger. Its completed/partial measurements
remain preserved and were not reused as qualified evidence.

`target/session-baseline-mac-v2-restart01/` completed five independent suites:
**48,000 measured refreshes in about 3 hours 27 minutes**. Allocation and artifact
size checks were stable; timing noise prevented qualification. An independent
audit reproduced all 677 overlapping qualification reasons across the 144 timing
dimensions. These are not SDK regressions. No candidate run started.

Analysis: `target/sdk-calibration-restart01/analysis-diagnosis.md`,
`analysis-qualification-audit.json` and `observations.json`. The launchd job was
removed. User chose the generation/SDK demo scope afterward, so do not restart
calibration automatically. Tools remain available for a future prospectively
agreed measurement policy/control setup.

## Checkout and scope constraints

- Repository: `/Users/luke/github/suspect`, branch `codex/sdk-contract-foundation`.
- The implementation remains extensively uncommitted; HEAD alone lacks this work.
  Preserve the current checkout. Do not reset/clean or replace it with HEAD.
- Source archives: `target/sdk-m3-m6-entry/worktree.tar.gz` and the sealed report's
  `source/`. Historical M0–M2 reports/binaries remain under `target/sdk-m0-m2*`.
- No package publishing, merging or upstream source edits are authorized.
- `openrouter-web/knip.json` has unrelated user/concurrent work; preserve it.

## Demo and implementation paths

- Demo config: `examples/sdk-demo.json` (expects `openrouter-web` beside Suspect).
- Demo guide: [SDK-DEMO.md](SDK-DEMO.md).
- Generator: `crates/suspect-codegen/src/backend.rs` and language modules.
- Contract/reference graph: `crates/suspect-ir/src/contract/`.
- Checked schema programs: `crates/suspect-schema/src/owned/`.
- Sessions/compatibility: `generation_session.rs`, `compatibility.rs` and children.
- CLI: `codegen`, `codegen-session`, `codegen-compare`; docs/custom templates use `gen`.
- Editor: `editors/vscode/`.
- Acceptance: `xtask/src/sdk_m3_m6.rs`; use explicit `--demo` for hackathon scope.
- Performance: `tools/sdk-session-perf/`; no active numerical qualification claim.

## Pinned corpus

Read-only source checkout: `/Users/luke/github/openrouter-web`, revision
`db378a2a90d0167b9dca4f98b52074c54d249e1f` at the recorded checkpoint.

| Input | Tracked path | SHA-256 |
| --- | --- | --- |
| Public | `projects/docs/openapi/openapi.yaml` | `bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821` |
| Management | `openrouter-management.openapi.yaml` | `c1ed00af257808f00c2a070b28ace135954f9cc1f97111a48364a35eae0505f2` |
| Provider | `projects/docs/assets/provider-monitor-schema-v2.openapi.json` | `5e8d14ed38af861e2c0d4827367d7f00d1a55722a5f62199e821ecc84241a355` |
| Temporal | `packages/temporal/benchmarks.openapi.json` | `e0fea9dc837eb24a3d82e8d5334fffd154d83cf22c44782b9651a36416b54ea7` |

The five demonstrated operations are `getCredits`, `createKeys`, `updateKeys`,
`listContainerFiles` and `getContainerFile`. Independent M2 and shared-runtime
fixtures live under `crates/suspect-codegen/tests/fixtures/`.
Full-document source validation findings remain distinct from selected SDK
profile acceptance; the public and management defects are retained in reports.
