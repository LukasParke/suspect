# Full SDK plan — live integration checkpoint

Updated **2026-09-11**. Full-plan implementation is active after the verified
hackathon checkpoint. Preserve the existing dirty checkout and all recorded
attempts; the implementation remains uncommitted.

## User decisions

- Implement the remaining original SDK languages and original-plan items end to end.
- Evaluate real developer experience against Speakeasy; readable public names,
  native construction and useful onboarding are part of the work.
- Add Terraform Provider generation as a **stretch goal**, built on the generated
  **Go SDK** so API-client improvements flow through its dependency. Provider code
  handles Terraform lifecycle/state/schema mapping. See `SDK-TERRAFORM-STRETCH.md`.
- The primary demo deliverable is now a polished local **webpage** showcasing all
  twelve SDK languages, with short examples and buttons that run the accepted
  native consumers live and display their results.

## Current Main checkpoint — shared integration continuation

Main integration now belongs to `ses_f72dcea72ffe3CL9Uf6IC2rP9u`. The existing
language, schema, protocol, editor and acceptance owners retain their areas.
Fresh Main evidence is under `target/sdk-main-final-integration-20260910-01/`,
including an exact pre-edit common-file snapshot. Earlier evidence is intact.

### Latest user priority — polished native-SDK demo webpage

**Current live result: all twelve native SDKs are confirmed HTTP200 with decoded
results.** The page is running at **http://127.0.0.1:8765**, observed PID70427,
using the guarded `./demo-web.sh` launcher and approved repair registry. Main
read-only verification of all twelve current-server job receipts passed in
`target/sdk-main-dart-tls-repair-20260911-01/actual-live-closure-01.json`, SHA
`9ad8ac6cf96c4be18dcc8d92e3cbc59915d10cc63d67c95e6b3c7dacbb22f109`.
Each records completed state, HTTP200, decoded=true, exit0, no truncation and the
full scalar confirmation. These are current live receipts, not imported success
markers. Dart binds secure manifest5eff… and Kotlin binds2ff…; both complete
source/execution/owner pin chains were reverified. Main made no new SDK/account
request and did not start the new process or inspect/change a key. The private
rotation decision and process-start actor are not inferred from these receipts.

**Recorded initial live failures:** the webpage's bounded technical check and the
parent's visual review passed, but subsequent user-triggered account requests
failed for **Dart** (`UnexpectedResponseException`) and **Kotlin** (`TRANSPORT`).
The other ten languages were reported working by the user. The parent owns the
two-language red/green loop at
`target/sdk-web-live-diagnosis-20260911-01/repro.py`; its `result-01.json` records
both real failures. Earlier zero-submission and controlled52 receipts retain their
earlier scope and do not establish current all-twelve account success.

**Repair history:** Kotlin's full-contract replacement is confirmed live
HTTP200. Dart's first Host-only replacement still returned400. Main identified a
second defect: its custom connection factory opened plain TCP even for HTTPS,
bypassing HttpClient's normal TLS selection. Earlier authenticated Dart attempts
may therefore have transmitted the credential without TLS; the parent notified
the user and recommended rotation. Dart execution was blocked server-side while
the native owner proved the secure fix with actual-SDK local TLS tests on both
tiers, including untrusted-CA/wrong-host rejection and encrypted cancellation.

The secure Dart-only producer is now
`target/sdk-main-dart-tls-repair-20260911-01/`, CLI `fb1f8eb4…`, with only
`lib/openrouter_io.dart` changed to `04bd75a2…`; the other28 Dart files are equal
to the previous cohort. Main verified the new private full-contract caller and
approved manifest
`5effd8a1ece8fb78f2d05c8b8fe51cd09f13b430a5fb9538343ec2f9342710bd`
in `dart-tls-adoption-01.json` (SHA `a2a9688f…`). The web owner hot-adopted it on
the same8765/PID58959 without restart or key transfer. Evidence:
`target/sdk-demo-web-20260911-05-repairs/DART-TLS-ACTIVE-01.json`.
The later current-server Dart job `39f84699586241b1b1e654958c11ea4e` is now
confirmed HTTP200/full decoded output under that exact secure revision. Its
receipt SHA is `e2b66d97d1d1a2dfc41c763442f390cf84f78029ea855c3e8425f716533f0961`.
Existing results supplied this closure without another Main request. Older Dart
binaries and the Host-only77a manifest remain historical and
must not be used for authenticated fallback.

The existing Dart/Kotlin owners and sole web owner handled these failures.
Dart's safe unauthenticated differential isolated missing Host framing: clearing
HttpClient's headers gives400, restoring only the URI-derived Host gives401. Its
actual-SDK regression reproduced HTTP400 and passes with the Host restoration on
both Dart tiers. Kotlin's cause is JDK HTTP/2 `:status` metadata entering ordinary
header validation and cancelling the response subscription; the bounded JDK-only
metadata filter retains TLS/HTTP2 and has a native diagnostic live200 proof.

The single coordinated repair cohort is now generated at
`target/sdk-main-user-live-repair-20260911-01/`: CLI
`6056ec28346fceda1a38a5c958bad5c63619e9b6402d5da9fb0b4bdc3f6e211f`,
source manifest `3add550686b8f5bf0218e69533d7b8817906f8b348689b51be0417ad2f34d3be`,
package pins `be358f3240d5fe20e98d9ab4b45ce1b6af679ba61f847d01251cb2cf168e50e7`.
Only the three proved transport templates were overlaid on accepted31c2; generation
produced611 files, changing Dart's IO file, Kotlin's two transport files and the
ownership manifest. The other ten SDKs remain byte-identical. A relocated Cargo
cache's missing tree-sitter output failed before generation; its cache/logs remain
preserved, and the same frozen candidate built successfully in a fresh target.

Kotlin's original full-contract caller has now been compiled against its fixed SDK;
Main verified all26 generated files equal this cohort and approved its eight pinned
execution artifacts in `kotlin-adoption-01.json`. Minimal diagnostic success records
are not used as final web confirmations: repair callers retain usage/freeTier/
management, and the strict web validator remains. The first Host-only Dart caller
was recorded in `dart-adoption-01.json`: all29 package files
match the6056 cohort, with35 source pins and two execution pins verified. Its
strict repair manifest is
`target/sdk-dart-live-diagnosis-20260911/replacement-01/replacement-manifest.json`,
SHA `77a765d3209e41dbf56545c5321e6c8706d867ea5e5188bad85e48b482cd1892`.
That first pair was adopted through a user-authorized same-port normal-ENV
transition, without RAM-key extraction. Kotlin passed live; residual Dart400
exposed the second TLS defect. The subsequent secure5eff replacement and current
live closure above supersede this Host-only attempt. Every intermediate receipt
and binary remains preserved.

**Deadline update:** the user has a demo in approximately one hour. The parent's
target is a working webpage within approximately 20 minutes, leaving the remaining
time for browser checks and rehearsal. The sole web owner should hand over the
first working URL immediately once twelve cards, token input, per-card background
native execution and UI results work. Main's critical path is the bounded technical
readiness check: accepted manifests/fixed dispatch, read-only GET `/key`, RAM-only
credentials/redaction, capture limits, cleanup and fail-closed HTTP results. Parent
owns visual review. Documentation sealing, SDK rebuilds/full matrices, broad source
freezes and numerical qualification do not gate this first working page.
The additive Main priority record is
`target/sdk-credential-env-integration-20260911-01/web-deadline-priority-01.md`.

The parent created exactly one web owner:
**`ses_f6f40ae87ffeE8xDWJ81zNTmA9`**. Its exclusive new implementation paths are
`demo-web.sh` and `tools/sdk-demo-web/**`, with fresh
`target/sdk-demo-web-20260911-*` evidence. The existing docs owner retains the
sealed native-env edition; native production, prepared programs and prior
README/script/seal paths remain their established owners' work.

The webpage is the primary presentation. It runs from a Python server bound
to **127.0.0.1**, default port **8765**, with a polished responsive frontend:
twelve language examples of three or four lines, Copy Source, per-SDK Run and
Run All, an asynchronous queue with progress/cancellation, and UI results showing
actual status, decoded booleans and exact usage. Runs launch the fixed, hash-pinned
native argv from the accepted manifests and use source-default read-only GET
`/key`. SDK generation, installation and the completed 52-case native matrix are
already established by the baseline below.

Credentials come from the server environment or one password input held only in
RAM. They stay out of arguments, logs and storage. The local API has origin/nonce
guards and an allowlist of native programs, with no arbitrary-command interface.
Timeouts, cancellation, cleanup and whole-stream truncation/redaction retain the
accepted runner's behavior. Agent/backend/browser staging uses controlled probes;
real API execution is user-click initiated. The parent owns visual review of the
actual page; Main owns independent technical readiness after the web handoff.

Main's prepared acceptance plan is
`target/sdk-credential-env-integration-20260911-01/web-readiness-plan-01.md`;
the durable assignment is `web-primary-assignment-01.json` alongside it.
Web implementation, bounded technical readiness, parent visual review and the
current twelve-native live confirmations are complete at their recorded scopes. Ruby F3,
TypeScript F4, SDK-full/editor/cost work and strict performance qualification
continue separately; numerical qualification does not hold this web delivery.

#### Accepted native execution and example baseline

**The original automatic-env edition remains the accepted preparation reference:**
[LIVE-ENV-DEMO-README.md](../LIVE-ENV-DEMO-README.md). Current authenticated execution
uses the webpage's guarded repair registry. The sealed older Dart executable has
the subsequently discovered TLS defect and must not be used for authenticated
fallback; its historical controlled receipts are preserved.
All twelve native SDK programs are prebuilt; JavaScript is an optional thirteenth
consumer of the TypeScript SDK. Each client uses its configured native environment
convenience, with runtime `OPENROUTER_API_KEY` or a hidden prompt and the source
GET `/key` default. Main accepted the final seal and all 52 controlled outcomes in
`target/sdk-credential-env-integration-20260911-01/live-env-delivery-owner-receipt-01.json`.
The original live, F10, reference and Terraform entries retain their seals.

The accepted baseline provides **live, single-run demonstrations for each of the twelve SDKs**,
using a real token supplied at runtime, with actual status/decoded confirmation
after short readable examples. The default is the source-declared `getCurrentKey`
GET `/key`, suitable for a normal token; `getCredits` is an explicit management-key
mode. Live programs use the OpenAPI HTTPS server without a caller URL override.
Staging uses controlled test mode; no token or live-success claim is fabricated.

The same docs owner `ses_f72144e30ffeca1RzXZzDgjbw3` owns new
`LIVE-DEMO-README.md`, `demo-live.sh`, `tools/sdk-demo-live/**`,
`examples/sdk-demo-live/**` and `target/sdk-demo-live-20260911-01/**`. Native
consumers are prepared in advance: stage commands perform no build or dependency
download. Runtime `OPENROUTER_API_KEY` or a secure prompt supplies credentials;
tokens do not enter arguments, generated files or logs. Existing sealed offline
and reviewed Terraform deliveries remain intact.

New clean-name DX paths are `DEMO-DX-README.md`, `examples/sdk-demo-branded.json`,
`examples/sdk-demo-branded/**` and `target/sdk-demo-dx-20260911-01/**`. Package
identities are explicit configuration: `@openrouter/sdk`, Python/Rust/Ruby/Dart/
C++ `openrouter`, Go `github.com/openrouter/sdk-go`, Swift `OpenRouterSDK` with
module `OpenRouter`, Java `ai.openrouter:openrouter-sdk` / `ai.openrouter.sdk`,
C# `OpenRouter.SDK` / `OpenRouter`, Kotlin `ai.openrouter:openrouter-kotlin` /
`ai.openrouter.kotlin`, and PHP `openrouter/sdk` / `OpenRouter`. Every package is
local and unpublished; preparation resolves actual local artifacts.

Main's `target/sdk-main-live-admission-20260911-01/` proves both selected live reads
generate for all twelve clean identities with the frozen `5d3cecd1…` CLI:
**one Contract compile, twelve renders, 434 desired artifacts plus ownership**.
`REPORT.json`, `session.json` and `package-manifest.json` are pinned and have been
forwarded to the docs owner and both new DX reviewers. This is generation proof;
native/live execution and automatic native environment loading are separate gates.
The working live edition may use explicitly documented wrapper-supplied runtime
credentials while the new native convenience is implemented.

The docs owner's actual live candidate is now
`target/sdk-demo-live-20260911-01/candidate-02/`: **592 desired artifacts plus
ownership**, selecting `getCurrentKey` and the original five operations. All
twelve native builds and 39 controlled checks (13 consumers × key/credits/401)
pass. This six-operation scope is distinct from Main's supplemental two-read
admission snapshot. `LIVE-DEMO-README.md` and `demo-live.sh` now provide source
excerpts, real read-only execution, secure prompt/environment handling, status
confirmation, deadlines/cancellation and nonzero failures. **Final live delivery-02
is accepted:** [LIVE-DEMO-README.md](../LIVE-DEMO-README.md) and
`./demo-live.sh all` are the accepted original human entry. Main independently verified all
1,276 seal hashes, 1,245 protected files with metadata, 593 package files and 39
actual controlled transcripts/wire exchanges in
`target/sdk-credential-env-integration-20260911-01/live-delivery-owner-receipt-01.json`.
The parent separately checked preflight: twelve of twelve ready. No real token or
account API execution is claimed.

The wrapper's capture-limit token-prefix defect is repaired and independently
verified for stdout and stderr. Entire truncated streams are discarded before
persistence, printing or confirmation parsing; a zero-exit child cannot qualify
truncated output as success. The original red proof and intermediate seal remain
intact. `live-redaction-green-01/REPORT.json` pins the independent recheck; final
runner SHA-256 is `578b79a4a34bf5a55966e217c96d3322ac31955ad9bf2015de09d9bcb85d3ee7`.
`target/sdk-demo-live-20260911-01/HANDOFF-02.md` records the fresh accepted seal.
The frozen `5d3cecd1…` packages use explicitly documented wrapper-supplied runtime
credentials; native environment helpers belong to the next configured edition.

The optional **[live v2 observability addendum](../examples/sdk-demo-live/observability/README.md)**
and `./demo-live-v2.sh all` are also verified. Only the TypeScript/Python callers
are replaced; the existing prebuilt SDKs and accepted redaction-safe runner are
reused. Main checked 680 new seal hashes, 3,113 protected files with metadata and
16 actual controlled failure transcripts in `live-observability-owner-receipt-01.json`.
Malformed 200/401 responses retain SDK status/category; transport errors retain
null status. The initial Python hook setup made an unintended read-only request
with an invalid canary token and received 401. That attempt is preserved and
explicitly excluded from the accepted controlled/no-egress proof; the corrected
HTTPTransport hook blocks socket connection APIs. No real credential or account
success is claimed. The original live entry and its seal remain intact.

#### Explicit environment policy implementation

The canonical contract is recorded in [SDK-CREDENTIAL-ENV.md](SDK-CREDENTIAL-ENV.md):

```json
"credential_env": {
  "version": "v1",
  "schemes": { "apiKey": "OPENROUTER_API_KEY" }
}
```

The shared `credential_env` module binds variable names to an already admitted,
uniquely identified source scheme, with typed provenance and relocation-independent
semantic capture. It never reads credentials at generation/import time. Native
clients snapshot mapped values at construction; explicit credentials are
authoritative without member-wise fallback; missing credentials fail before HTTP
while OR/AND/anonymous behavior remains intact. V1 is configuration-only and
admits bearer/API-key strings. Browser and portable explicit-auth paths remain.

Main owns shared planning/configuration/cache/CLI/provenance/capture integration.
All twelve **existing** native owners have bounded implementation/native-test
assignments in `target/sdk-credential-env-integration-20260911-01/owner-assignments-01.json`.
Seven public controls now pass: source binding/deduplication, syntax/duplicate-key
limits, native-policy versus wire-interpretation reporting, unknown/unused/nonstring
hook refusals, referenced-source/relocation fidelity, cross-document ambiguity,
and config-edit/cache-revert identity. `GenerationOptions` and session JSON carry
the optional policy. The real CLI policy/compare/no-write control passes.
**Python, Java, PHP, Dart, Ruby and C# now have completed native proofs on both
declared tiers and focused canonical capture proofs.** Main opened their readiness
entries and passed `canonical-six-01` (eight shared/integration tests): generation
equals Session output, policy edits reuse the Contract and rerender targets,
semantic capture is typed, wire interpretation is unchanged, and A/ordinary
reverts reuse their original artifact Arcs.

C++'s completed native receipt additionally proves both libcurl and core-only
factories, 31-file no-policy parity and source-default controlled reads. Main
verified 54 source entries, 28 runtime entries, 20 native command results and the
installed client header/library in `cpp-native-owner-receipt-03.json`; two earlier
receipt-reader mistakes are retained. C++ readiness is open, its new emitter asset
is inventoried, and `canonical-seven-01` passes the seven-target canonical bridge.
Main then inspected the completed TypeScript/Go/Rust/Swift/Kotlin native handoffs,
verified 338 further source/artifact/log entries and pinned 50 receipt files in
`native-final-five-owner-receipt-01.json`. **All twelve readiness entries are now
open.** `all-twelve-bridge-01` passes all eight shared/env tests and both canonical
resource/ordinary-Rust tests. Raw policy forwarding reaches every native adapter.
Main also found and verified the complete final TypeScript existing-emitter/capture
index already present in `acceptance-01.json` and `source-checkpoint-01/`:
all ten source entries match the current and corrected-CLI source, including the
four existing emitters and exact TypeScript capture function. Its six native-gate
log hashes remain valid in `typescript-final-source-owner-receipt-01.json`.

Rust's ordinary HTTP-manifest/capture repair is now accepted. Main switched the
canonical backend to `plan_http_v3`, matching the retained native capture, and
removed the Rust exclusion from the resource bridge. The full-file ordinary V2/V3
equality guard remains intact and passes. Resource generation/Session/capture now
cover all twelve adapters; dynamic wire-equivalence reporting stays conservative.

Final owner indices:

| Adapter | Completed evidence |
| --- | --- |
| Python | `target/sdk-python-credential-env-final-20260911-01/completion.json` |
| Java | `target/sdk-java-credential-env/REPORT.md`, `production-assets.json`, `support-files.json` |
| PHP | `target/sdk-php-credential-env-20260911-01/report.json`; `target/sdk-php-credential-env-canonical-20260911-01/report.json` |
| Dart | Original `target/sdk-dart-credential-env-20260911/`; corrected constructor `target/sdk-dart-credential-env-null-20260911/report.json` and independent `standards.md` |
| Ruby | `target/sdk-ruby-credential-env-20260911-01/report.json`, `HANDOFF.md` |
| C# | `target/sdk-csharp-credential-env-evidence-20260911/report.json` |
| C++ | `target/sdk-cpp-credential-env-verification-20260911/report.json` |
| TypeScript | `target/sdk-typescript-credential-env-20260911/handoff.md` |
| Go | `target/sdk-go-credential-env-20260911-01/completion-01.md`, `artifact-inventory-01.json` |
| Rust | `target/sdk-rust-credential-env-20260911-01/native-receipt-01.json`, `handoff.md` |
| Swift | `target/sdk-swift-credential-env-20260911/native-handoff.json` |
| Kotlin | `target/sdk-kotlin-credential-env-verified/HANDOFF.md`, `native-receipt.json` |

Main independently verified 393 completed source/evidence entries and pinned 25
final receipt files for the first six adapters in
`native-six-final-owner-receipt-02.json`. The earlier reader attempt preserves its
mistaken assumption that a final inventory omitted historical failed commands;
C# correctly retains Clippy-01's failure alongside the successful Clippy-02.

Dart's preserved no-policy maps contain **25** artifacts; the earlier 26-file
prose count was incorrect. Its four Chrome pages are a separate browser-script
invocation. Ruby's historical 56-artifact raw parity remains accepted as immutable
receipt-only proof at the original source paths. Main approved the Ruby/runner
contract for a mandatory self-contained current no-policy/configured/disabled
gate, exact artifact deltas, relocated provenance and controlled ambient-value
independence across generator subprocesses. Its host-only implementation passes
three checks in `target/sdk-ruby-credential-env-runner-20260911-01/`; the same five
selectors remain, and the optional historical baseline input is retired.
Historical expected bytes and native matrices remain intact.

The configured CLI process check exposed and repaired a missing policy field in
successful session output. `cli-defaults-process-01/{red,green}.log` retains the
actual missing-field failure and passing `credentialEnv` names-only output, two
generator-process canaries, complete artifact/revision equality and disabled-helper
cleanup. Absent-policy reports retain their prior shape.

A fresh env-enabled CLI is built at
`target/sdk-main-env-candidate-20260911-01/bin/suspect`, SHA-256
`f971ead49e99a15e205325b186b2a7effb640e412da31f8c24e100983c59f496`.
Its immutable private source contains 1,157 files, unchanged during the build, and
the actual binary advertises exactly twelve SDK profiles plus the separate
Terraform command. Fresh same-six-operation clean-name generation/capture passes
under `live-generation-01/completion-02/`: **610 configured artifacts plus
ownership**, twelve Planned captures with six operations each, typed policy,
native-only variable changes, and two generator processes with identical files
and revisions. **All 593 unconfigured files equal the original live package pins.**
The first reader used the wrong compatibility-report nesting; its four successful
CLI processes and package outputs were retained and not replayed.

#### Independent env review and corrective source delta

`review-env-01/` freezes 129 changed source/test/support files, nine context
documents and the 1,157-file build context. Both independent axes completed:

- **Standards P2:** configured Dart widened the existing non-nullable credential
  argument and accepted explicit null. The Dart owner restored the non-nullable
  parameter with the private omission sentinel and updated capture/docs. Both
  Dart tiers pass targeted VM/JS checks and two separate Chrome pages; 25-file
  no-policy parity remains intact. Main verified 42 evidence entries. The original
  Standards reviewer independently rechecked the repair and is **clean** in
  `review-env-01/standards-dart-recheck-01.md`. This closes the pinned source/package
  repair; the older `f971ead49…` nullable packages remain historical.
- **Spec P2:** Go's allocated `NewClientFromEnv` factory was omitted from native
  capture. An additive source model renames it to `NewClientFromEnv2`, but the old
  comparison reported compatibility despite an actual old-consumer compile
  failure. The existing Go owner's capture/signature repair is complete in
  `target/sdk-go-env-factory-capture-20260911-01/`. Its actual CLI now reports the
  located rename as **Breaking** and exits 1. Main verified 657 evidence entries,
  all 212 emitted-artifact parity entries, 74 original-review package files, two
  unchanged ordinary captures and four exact configured-capture projections in
  `go-factory-owner-receipt-01.json`. Three new host controls and the existing
  canonical control pass. The same Spec reviewer independently rechecked the
  correction and is **clean** in `review-env-01/fixes-01/spec.md`.

`review-env-01/findings-and-dart-closure-01.json` pins both original reports and the
Dart-specific recheck without changing either axis's severity. Both combined
source rechecks are now **clean** in `review-env-01/fixes-01/{standards,spec}.md`:
13 inventory deltas, seven context documents and 1,161 pinned build inputs. Spec's
three fresh independent comparisons also verify all-twelve configured empty
refusals and unconfigured `EmptySelection`, alongside the original Go collision.
Its inventory-reader failure involving a non-build Python cache file remains
preserved separately. The final automatic-env live delivery now uses the corrected
candidate and is independently accepted below.
The sole docs owner delivered the additive `LIVE-ENV-DEMO-README.md`,
`demo-live-env.sh`, environment example/tool paths and `environment-01/` staging
paths with the corrected CLI and exact package pins below.

The corrected candidate is now
`target/sdk-main-env-candidate-20260911-02/bin/suspect`, SHA-256
`31c2fe23c760f191fdb8546cc10d2d78973935d3a728cd8875dfb66f45975261`.
Its 1,161-file source manifest has SHA-256
`35bbf1114bee47cab2b5c72460eec878130c0a48e0ef723d3e8bb9ae8ab0d1be`.
The actual same-six-operation CLI proof is
`live-generation-01/completion-02/REPORT.json`: **611 configured package files**,
**593 ordinary files equal to the original live edition**, two independent
generator environments with identical artifacts/revisions, all twelve native
captures Planned, names-only policy and native-only variable changes. The new
binary reproduces the located Breaking Go factory collision and the corrected
configured/ordinary empty-capture distinction.

All eleven non-Dart package subtrees are byte-identical to the first env cohort.
Only Dart's constructor, README and separately emitted credential guide change,
plus the root ownership manifest. Package-pins SHA-256 is
`7d27ae8229b1d3bddcef76cab66da5f7b4ba738cf33847b514fb2e2a5ec22e27`.
Main checked 1,211 protected prior files with metadata. The first verifier omitted
the standalone Dart guide from its expected set; its failed attempt and all three
successful generation runs were preserved and reused. The sole docs owner has the
exact replacement pins and eleven-language parity receipt for valid preparation
reuse. `review-env-01/fixes-01/closure-and-replacement-cli-01.json` links both clean
reviews and this actual CLI/package boundary. Focused current quality is complete:
**20 host tests passed**, zero failed, three separately proven Dart native opt-ins
ignored; scoped all-feature Clippy and codegen Rustdoc pass with warnings denied,
and workspace formatting passes. Source bytes remain stable. The accepted env
source/CLI boundary is recorded in `ENV-ACCEPTANCE.json`, SHA-256
`841ff131324377c5d551dba7aa527fb6a34efd66cebc100836cefc5044574c4a`.
The sole docs owner used this accepted source boundary for final controlled native
preparation and sealing. Final whole-workspace acceptance retains its later source
freeze.

The completed provisional preparation now has an independent Main receipt:
`env-provisional-native-owner-receipt-01.json`. Main verified **1,541 entries**,
the ten prepared native consumers plus JavaScript, all **44 actual outcomes** and
**33 controlled GET exchanges**. Eleven missing-environment cases fail with zero
HTTP. Every completed package/source subtree matches the corrected candidate,
allowing exact reuse of that preparation. Fresh Go and corrected Dart first-attempt
builds then passed their eight key/credits/401/missing-env checks.

**Final automatic-env delivery is accepted.** The new human README has SHA-256
`680457b1b4a4b1794e4d26d47d2bb77b72eb06554c431bfb079b1beca7fcf13b`.
Main independently verified **3,179 seal hashes**, **4,377 protected files with
metadata**, all **1,541** previously accepted provisional entries, all **611**
corrected package files, and the actual **52 outcomes**: 44 reused plus eight new
Go/Dart cases, **39 controlled GET exchanges**, and **13 missing-env zero-HTTP
cases**. Fourteen retained documentation command records validate 13 exact
four-line excerpts; all 23 README links resolve. The recorded new-edition preflight
is 12/12 ready. Main verified its transcript without executing it again.

The adapter's routing/prompt simulations remain explicitly labeled. Main also
checked the unchanged critical base-runner functions, Python handler AST equality
and TypeScript/JavaScript guard equivalence. The 16 earlier F10 cases remain
inherited evidence outside the 52-case total; the failed invalid-canary outbound
401 setup remains disclosed and excluded. Actual ENV `live-runs/` is absent;
no real-token live success is claimed. The final receipt is
`live-env-delivery-owner-receipt-01.json`, SHA-256
`3ba90129fd219ac246e0d5b028fe9123d9e22e911a91985248ec57305558c3c9`.
Owner handoff: `target/sdk-demo-live-20260911-01/environment-01/HANDOFF-01.md`.

The later Go formatting-only notification is already contained in the accepted
source: current `compatibility/native.rs` equals the candidate's `85dd97c0…`
bytes. `go-format-closeout-reconciliation-01.json` records that receipt at its
existing scope; it introduces no post-acceptance source delta.

Main also repaired a shared empty-selection edge: a configured native capture must
retain the actual native admission refusal rather than return `EmptySelection`
early. `empty-capture-01/red-02.log` reproduces `EmptySelection` versus generation's
located `http-no-operations`; `green.log` passes the small shared-branch repair.
Unconfigured empty capture retains its prior meaning. The maintained
`credential_env` target now has nine host tests; both combined reviews and
Runner 17 cover this delta.

#### Current host quality scope

`quality-env-candidate-01/` checks the immutable env source plus the completed
Go V3 test-only Clippy fix. Workspace/all-target/all-feature **Clippy passes with
warnings denied**, and workspace/all-feature **Rustdoc passes with warnings denied**.
The full test run records **1,641 passed, 12 failed and 395 ignored** across 258
harness summaries. All twelve failures were execution-copy setup: four copied
read-only fixtures, three missing pinned TS compiler cases and five omitted root
rules-fixture cases. `quality-setup-closure-01/` reruns only those twelve exact
selectors with normal execution-copy permissions, complete fixtures and pinned
Node22/TS5.9; **all twelve pass** and source bytes remain unchanged. The first
logs are preserved. One Main-owned formatting chain was repaired separately.
These receipts qualify the frozen env scope; later review fixes retain their own
focused/current quality boundary.

Original acceptance continues; numerical collection remains unlaunched during
native work. None of these new helpers is claimed present in frozen live CLI
`5d3cecd1…`.

The earlier shared ignored-multipart/TypeScript gap is now complete:
`target/sdk-http-protocol-ignored-encoding-20260910/HANDOFF.md` records 46 shared
checks, 20 example controls and the installed TypeScript 5.5.4/5.9.3 × Node
22.23.1/24.21.0 witness (11 exchanges, 11 request and nine response controls per
combination). Main accepted and forwarded its exact native selector to the
existing runner owner. The defensive content-plan guard remains; old matrices
were not replayed. The separate source-only release handoff at
`target/sdk-http-protocol-ignored-encoding-release-20260910-01/` is also verified:
seal `d4c4e9e5…`, 40-file source aggregate `056188db…`, 16 native wire/result files
and 37 managed package files. Main checked 114 entries in
`ignored-encoding-release-owner-receipt-01.json`. Its earlier source capture has
36 files still equal to the current checkout; three later TypeScript env emitter
changes match the accepted corrected CLI, and the added env documentation paragraph
is reconciled separately. Runner 18 already registers the exact native selector
once. This remains the existing bounded native prerequisite closure, with no new
runtime or whole-workspace acceptance inferred from the source seal.
Rust's canonical ordinary-manifest bridge and Swift aggregate
proof are now accepted. Main verified the existing Swift aggregate handoff and
both completed native tiers in `swift-aggregate-owner-receipt-01/`: four consumer
tests, one generated example and DocC per tier, eight raw native command logs,
and unchanged test/fixture/driver/support bytes. The primary Swift module's later
optional-env additions are reconciled in a retained diff. The exact ignored
library selector was handed to the runner for both tiers; no native replay was
needed. Runner 17 now registers it on both tiers; the former floor/selector
handoff blocker is closed. The actual compiled-library census remains required.

#### Independent DX assessments

The parent assigned two read-only reviewers:

- `ses_f71b2c181ffefMM5xShcPIrnJp` owns
  `docs/SDK-DX-REVIEW-20260911.md`, assessing actual all-twelve native DX.
- `ses_f71b1c877ffeo4XEwiZuk35VrX` owns
  `docs/SDK-CURRENT-SDK-COMPARISON-20260911.md`, comparing pinned current official
  OpenRouter TypeScript/Go/Python packages and repository heads with matched
  controlled consumers.

They retain baseline/current/configuration distinctions and receive fresh
candidate manifests. Their work does not hold up the runnable live edition.

The current official-SDK comparison is complete in
`docs/SDK-CURRENT-SDK-COMPARISON-20260911.md`, with evidence at
`target/sdk-current-comparison-20260911-01/`. It pins official TypeScript 1.2.117,
Go 0.7.130 and Python 1.1.137 against the explicitly scoped live candidate.
Chat/vendor-SSE and untyped upload admission gaps are recorded for prioritization;
they do not authorize inferred semantics or an additional implementation tranche.
The all-twelve review is also complete in `docs/SDK-DX-REVIEW-20260911.md`, with
evidence at `target/sdk-dx-review-20260911-01/` and `-02-env/`. Its factual findings
retain exact snapshot distinctions. The existing Ruby owner has the reproduced
no-input generated-README bug; the TypeScript owner is assessing the implicit
exact-number coercion guard and compatibility impact. The sole docs owner has an
additive two-consumer observability repair for lost SDK status/category on invalid
responses. These bounded follow-ups preserve the sealed working live programs.
The known truncation/redaction issue is recorded as resolved against delivery-02.

### Completed September 11 offline/reference demo deliverable

The user requested a local, step-by-step README explaining usage patterns and DX
for **each of the twelve SDKs**, ready for the demo on **2026-09-11 morning**.
The sole docs/demo owner is **`ses_f72144e30ffeca1RzXZzDgjbw3`**, owning new
`DEMO-README.md`, `examples/sdk-demo-all.json`, `tools/sdk-demo/**` and
`examples/sdk-demo-all/**`. Its evidence starts at
`target/sdk-demo-readme-20260911-01/`. Default requests use offline/loopback
fixtures; only an explicitly chosen live credits read may use a user-supplied
token. Existing demo/config/report artifacts stay historical.

Main's fresh source-pinned demo CLI is ready at
`target/sdk-main-demo-candidate-20260911-02/bin/suspect`, SHA-256
`8e567dc987b7cff03d37b328a1a1b746ae764bf20aa54bc14465d127f211b499`.
The retained `REPORT.json` verifies a byte-stable private build, actual default
twelve-profile discovery and Terraform registration. Attempt 01 retains a
build-input inventory miss for the five transitive `rules-runtime` source files;
attempt 02 includes their exact bytes. The docs owner has the new binary and owns
the fresh prepared packages and walkthrough. Its actual package roots are
`target/sdk-demo-readme-20260911-01/candidate-02/packages/`: twelve renders from one
Contract compile, with 565 artifacts. `package-pins.json` records their bytes and
metadata; `pins.json` binds all four tracked upstream inputs, CLI and config.
`DEMO-README.md` is written: a 5–10 minute run order, preparation/day-of commands,
and detailed usage/DX chapters for all twelve SDKs plus independent JavaScript.
All thirteen prepared native consumers pass both the
exact-number/construct/omitted-null loopback flow and typed 401 checks. Fourteen
native excerpts and 136 local links are checked; the optional live-read block was
exercised only through an injected Fetch oracle. The ordinary Terraform appendix
has a separate ten-command local workflow and 48-file native artifact parity.
The completed handoff is `target/sdk-demo-readme-20260911-01/HANDOFF-01.md`.
The root README SHA-256 is
`09c99c8401748a981089a2f496613289c27add17601bfd21ef83c78b143b820a`.
Main verified all 480 delivery-seal files, all 566 package hashes/metadata, and
the actual 26 successful consumer transcripts with 65 local requests in
`demo-delivery-owner-receipt-02.json`. Attempt 01 preserves a receipt-reader
mistake requiring an optional timeout field on older completed command records;
the corrected receipt records that field's absence explicitly. The README is
ready and delivered independently of the remaining full-plan gates.

The optional **[reviewed Terraform addendum](../examples/sdk-demo-all/terraform-reviewed/README.md)**
is also ready. Its prepared root is
`target/sdk-demo-readme-20260911-01/candidate-02/terraform-reviewed-01/`; the
additive seal is `candidate-02/delivery-02/`. Main verified all 192 new sealed
files, 1,051 protected prior files with metadata, 48 current owner-matching
artifacts, the 34-file SDK ZIP/installed-byte linkage and its independently
computed `h1`, and ten successful native commands/eight local exchanges in
`demo-reviewed-terraform-owner-receipt-01.json`. The original root README and
delivery-01 remain unchanged.

The original all-twelve comparison's five Ruby source-relocation false positives
are repaired by a typed credential capture projection. The fresh comparison CLI
`target/sdk-ruby-credential-compatibility-20260911-02/bin/suspect` has SHA-256
`32d24ad5310b3e633c9378e54afd34e7be3bad868e4bd2353ab1fe84fc933599`.
Its exact replay plans all twelve profiles and preserves the two real potential
changes (wire constraint and TypeScript declaration), with zero Ruby changes and
zero unknowns. The separately pinned `candidate-02/comparison-final-02/bin/suspect`
now has a full 565-artifact plus ownership-manifest byte-parity receipt. The README
again uses the all-twelve comparison. `candidate-02/preservation-fix-01/result.json`
records two repeated regenerate/compare flows preserving 16 original and earlier
new receipt files' hashes, mtimes and inodes; subsequent summaries use fresh paths.
The original seven-finding report and interim TypeScript-only reports remain
historical, including the interim `e7807e44…` CLI. Ruby's final handoff includes
22 focused passes and preserves literal OAuth scope names resembling descriptor
metadata. Final capture/test bytes already match the late-review snapshot.
README delivery is prioritized independently of strict numerical qualification;
original acceptance continues.

### Integration and acceptance

**Common closeout:** both independent axes are clean at
`review-01/fixes-04/{standards,spec}.md`, after the canonical IR dual-role scope
repair. The retained common run has 43 passing checks. The sole Terraform owner
is **`ses_f7249c553ffeiEu2jQF3uZ8zO9`**, recorded in `terraform-owner-01.json`, with
new artifact-module/test/docs ownership and a pinned generated-Go-SDK-only API
boundary. No competing Terraform owner exists.

Both axes are also **clean** at `review-late-integration-01/{standards,spec}.md`:
six changed files
and 49 pinned context files covering package diagnostic identity, Ruby credential
capture, the eleven-adapter canonical bridge, and the two reference-test repairs.
The reviewers verified the final Ruby source/test/CLI/replay hashes and the exact
source-fidelity controls. This review records the Rust ordinary-manifest bridge as
a known pending item;
its eventual promotion receives a fresh source delta.

- **Terraform CLI:** all three real process checks and four established SDK
  option checks pass in `terraform-cli-integration-05.log`. The separate
  `codegen-terraform` command preserves exact SDK dependency identity, located
  mapping diagnostics, artifact ownership and cache-only generation after the
  originals are deleted. Corrupted private cache bytes fail digest verification.
  CLI all-target/all-feature Clippy passes in `cli-clippy-terraform-01.log`.
- **Terraform native milestone:** the owner reports 204 real commands and 330
  strict checks on Go 1.23.12/1.27.1 with Terraform 1.15.8 in
  `target/sdk-terraform-stretch-20260910-01/native-02/report.json`. Supplemental
  `variants-02/report.json` passes 12 commands and 10 checks for optional-computed,
  unknown-value refusal, anonymous data-only providers and a real allocated-method
  SDK upgrade. Nine host checks, scoped Clippy and warnings-denied Rustdoc pass.
  All 48 base artifacts retain their native-02 bytes. Both existing reviewers
  completed the exact Terraform/CLI snapshot in `review-terraform-01/`;
  `supplement-01/` pins these newer receipts and its documentation-only delta.
  Main verified all 779 sealed evidence files and 21 owned source files in
  `terraform-final-owner-receipt-01.json`. Standards has two confirmed P2s: dotted
  provider namespaces are admitted before Terraform rejects HCL, and lazy error
  streams can remove missing-resource state without validation and leak their
  owned body. The sole Terraform owner has both repairs and the actual native
  counterexamples in `standards-evidence-02/` and `standards-evidence-03/`.
  Spec independently confirmed the same stream problem as **P1**, preserving
  Standards' **P2** ranking on its own axis; its unmapped error control is 503,
  alongside Standards' 409. Spec also found a package-admission **P2**: an SDK
  module equal to the provider's `/provider` import path creates an ambiguous Go
  import. All three are assigned to the same owner. Address admission now passes
  24 native Terraform parser controls in
  `target/sdk-terraform-address-admission-20260910-01/`, with all 48 ordinary
  artifacts byte-identical. The module-collision repair passes 28 commands and
  20 checks on both Go tiers, including native ambiguity controls and real valid
  nested/sibling module linkage. Final error-ownership `native-green-02` passes
  both tiers with exact SDK ZIP/`h1`/no-replace linkage, including independent 503,
  cancellation, partial-identity early return and nested transport-cause controls.
  The combined owner handoff is
  `target/sdk-terraform-review-fixes-20260910-01/HANDOFF-01.md`: 12 host passes and
  scoped quality checks; only the two provider lifecycle Go artifacts differ from
  the original 48-file candidate. Main verified 928 source/evidence entries,
  including the original 779 sealed files. Both independent rechecks are **clean**
  at `review-terraform-01/fixes-01/{standards,spec}.md` (64 files; 11 changes/seven
  additions). Fresh independent counterexamples verify lazy error cleanup and
  buffered validation, original/reverse module collisions, a legitimate nested
  checksummed SDK dependency, and native Terraform address/HCL acceptance.
  `review-closure-01.json` pins both clean axes and their independent evidence,
  preserving original Standards P2/P2 and Spec P1/P2 rankings. The same docs owner
  delivered the [reviewed Terraform addendum](../examples/sdk-demo-all/terraform-reviewed/README.md)
  and passed its
  bounded ten-step local workflow with a newly built Go 1.23.12 provider, SHA-256
  `077219ba674e0ca062b45c114fa21b1115aa1322cba9d4467c97419f23a5c51f`.
  Its fixed-CLI output matches all 48 final owner artifacts; exact SDK v0.4.2
  ZIP/`h1`/installed-file/binary linkage is checked without replacement directives.
  The additive delivery-02 seal and 21 local links/two shell blocks are verified.
  Original README/delivery-01 files, SDK packages and consumers remain pinned in
  place.
  `terraform-review-findings-01.json` pins both
  final reports and records three unique findings with their original axis ranks.
- **Fresh review CLI:** `target/sdk-main-review-candidate-20260911-01/bin/suspect`,
  SHA-256 `5d3cecd1bc0d5bd6bdda4b086ff7038977a7bb4939cefca213235e2d225e84cb`,
  is built from the original frozen demo source plus the two Terraform fixes and
  final Ruby capture. Five real CLI checks pass in `terraform-review-cli-green-01/`:
  all 48 artifacts equal the final owner's output, check preserves metadata, and
  all three review defects have located pre-artifact refusals. The real twelve-SDK
  preview in `review-cli-sdk-parity-01/` proves all 565 desired SDK files match the
  prepared demo and all 566 package/ownership files retain bytes, mtimes and inodes.
- **Final native handoffs:** Rust's resource guide and immutable receipt now
  verify five native selectors on both toolchains. Main verified all twelve
  production hashes and eight retained command-log hashes in
  `rust-v3-source-receipt-01.json`. Swift's resource guide and retained current/floor
  native package logs also record completed v3 proof. Main verified 78 emitted
  Swift validation-runtime files and 22 native command logs. The eleven-adapter
  resource generation/session/capture bridge passed in
  `canonical-resources-eleven-01.log`. Its subsequent all-twelve closure, canonical
  Rust v3 dispatch and retained ordinary V2 artifact equality pass in the new
  `all-twelve-bridge-01/` receipt above; `rust-v3-canonical-byte-01.log` remains the
  original manifest-difference red witness.
- **Earlier workspace scope (before env):** workspace Rustdoc with all features and warnings denied
  passes in `workspace-rustdoc-02.log`; earlier receipts remain preserved.
  Staged/unstaged whitespace checks pass. The first
  workspace test attempt retained 13 failing assertions and hit the 120-second
  harness deadline (`workspace-tests-01.log` / `workspace-failures-01.json`), so it
  is incomplete. Main's shared admission, import identity and scoped-model
  follow-ups pass. The complete no-timeout `workspace-tests-02.log` run has
  **1,583 passed, 3 failed and 365 ignored** across 240 harness summaries.
  `workspace-repair-closure-03.json` records all thirteen prior failures resolved
  (twelve unchanged names plus the renamed directional/intersection control).
  The two reference provider/transport failures are now resolved in
  `target/sdk-ref-canonical-assertions-20260910-01/DISPOSITION.md`: 34 maintained
  pinned-suite passes, targeted Clippy/format/whitespace checks, and only two test
  files changed. The transport control proves that unprovided remote references
  cause no load attempt, then explicitly opens the same URI and verifies the
  denied attempt is recorded without HTTP. Located valid/invalid resource and
  anchor controls cover both readers. The Rust v3 ordinary-manifest bridge is the
  last remaining failure from workspace run 02 and is now resolved by the all-twelve
  bridge above; the historical log retains all three original failures.
  Warnings-denied **workspace Clippy now passes** in `workspace-clippy-05.log`,
  including all targets/features and the repaired Go v2 test conversions. All
  1,092 recorded source files stayed stable during that check. Latest staged and
  unstaged whitespace checks pass. **Workspace formatting now passes** in
  `workspace-fmt-04.log`, after the final Terraform-owned assertion formatting.
  `post-review-host-01.log` passes all 18 final Terraform/Ruby host checks under
  all codegen features; five native opt-ins retain their separately executed
  receipts. The recorded source files stayed stable during this integration run.
- **Runner readiness:** `target/sdk-full-runner-check-20260910-18/HANDOFF.md`
  records 924 stages, 920 functional requirements, 296 frozen harness invocations,
  33 library declarations, 50 native selections and 41 passing host tests. All six
  finalized Go v3 witnesses and the separate aggregate witness are registered on
  both exact Go tiers with pinned Sphinx selectors. The six final Ruby credential
  controls now have an explicit frozen host suite/execution proof, without a
  native-tier environment. All twelve env adapters now have required native
  coverage with their exact tier/internal-matrix contracts. Thirteen newly frozen
  targets retain the 66-name inventory and now require the ninth shared host name
  and sixth Dart suite name, including shared
  policy/Session/capture and CLI process controls. The shared/TypeScript
  ignored-multipart controls remain registered. Prior definitions and 1,037 earlier
  evidence paths/binaries are preserved. Main independently verified 1,085 pins
  and the exact two-stage Go host addition in `runner-18-owner-receipt-01.json`;
  `runner-17-owner-receipt-01.json` retains the earlier Swift/Dart/shared closure.
  `runner-12-owner-receipt-01.json` retains the earlier 611 verified -12 hashes.
  The Go and Ruby inventory gaps are closed. Final source freeze, HTTP descriptor/native
  closeout, both current twelve-profile editor scenarios, all 92 native-cost rows
  and the four strictly qualified numerical gates remain outstanding. No numerical
  collection is running during native work. The accepted Swift aggregate
  floor/current selector, ninth shared empty-capture host check and new Dart
  constructor-only selector are registered. The Go capture repair's three new
  host selectors now run once with a separate execution proof and fresh private
  capture-artifact parent. Runner 18's historical pending-review text is retained;
  the later independent reviews and `ENV-ACCEPTANCE.json` above close that env
  boundary. The compiled-library census stays fail-closed. Final
  integrated native execution requires its own freshly frozen harnesses and raw
  stdout/stderr; owner completion notifications are not execution substitutes.

### Earlier Main integration entries

These chronological entries retain their original checkpoint scope; the current
closeout above supersedes earlier open/future dispositions.

- **All twelve SDK features are enabled by default.** PHP's completed advanced
  protocol report has nine native gates on each of8.3.32/8.5.8 and thirteen
  verified canonical integration checks:
  `target/sdk-php-advanced-20260910-01/report.json`. Java's six integration findings
  are also repaired:15 canonical checks pass, with completed JDK21/25 protocol
  evidence in `target/sdk-java-protocol/REPORT.md`. Their v2 work is separate.
- Shared `examples::plan_protocol_examples_v2` explicitly uses `compile_v2` and
  reuses canonical slot discovery, source provenance and finite validation/
  synthesis budgets. Its two new checks plus the twelve existing protocol-example
  checks and three context checks pass in `scoped-example-check-01.log`. All native
  owners received the API handoff; default example planning remains v1.
- Main registered the new protocol/resource and current scoped-runtime production
  assets. Source inventories are being reconciled with final owner handoffs.
  Dart's superseded local example draft is preserved by its owner and its stale
  provenance include was removed; Dart now uses the shared v2 example seam.
- New wire witnesses reproduced missed itemSchema target changes, malformed item
  schemas and physical relative-server relocation. Static-resource target changes
  also exposed reference-wrapper misalignment in mixed-dialect inclusion. Red
  evidence is `wire-resource-red-01.log`. The fixes pass all eight context cases,
  all29 existing compatibility cases, four twelve-backend interpretation checks,
  Java's15 and PHP's13 integration checks: **69 passed**, with one previously
  verified Java native reflection opt-in left ignored, in
  `wire-options-integration-01.log`.
- Acceptance owner `ses_f739ce66effeDLHhy3mAbuOCZB` is integrating exact v2 native
  selections and the editor host contract, with fresh runner-only evidence `-04`.
  The finalized editor contract is
  `target/sdk-editor-native-host-integration/HANDOFF.md`:53 new guards and an
  isolated selection probe passed. Both complete twelve-profile lifecycle and
  command scenarios remain required. Full acceptance begins after source
  integration settles.
- The current default CLI passed11 process checks for options, sessions, pinned
  inputs and comparisons in `cli-twelve-defaults-01.log`. Its frozen binary's
  actual profile discovery verifies exactly twelve profiles:
  `cli-profiles-evidence.json`.
- Swift and Ruby completed native scoped v2 on both toolchain tiers. Reuse
  `docs/SDK-SWIFT-VALIDATION-V2.md` and
  `target/sdk-ruby-schema-v2-verification/report.json`; their complete runtime,
  installed-package/type/codec/example/doc witnesses are separate from unchanged
  v1 protocol matrices. Both preserve v1 bytes and refuse unwitnessed v3 programs.
- Rust, C#, Dart, Python, Java and PHP also completed native v2. Their receipts are
  indexed in `SDK-SCHEMA-NATIVE-ADOPTION.md`. Main selected Rust's explicitly
  additive `plan_http_v2` entrypoint for canonical generation; its native capture
  is coordinated with the Rust owner. Existing native v1 entrypoints stay intact.
- Owned resources/dynamic v3 is verified:206 schema tests,44 official dynamicRef
  cases and four deferred dynamic/unevaluated cases; published v2 bytes stay
  unchanged. Main added the requested `ProgramResource`/`ProgramResourceContext`
  public exports. All twelve existing native owners have the stable v3 handoff;
  they finish their v2 proofs first. Main's explicit `plan_protocol_examples_v3`
  helper is implemented, with source-driven checks in progress.
- The independent common reviews each reported two P2 findings. Original reports
  are `review-01/{standards,spec}.md`. All four have actual immutable-CLI red
  witnesses under `review-01/cli-red-02/`. Fixes cover complete media-reference
  diagnostic chains, context-aware item metadata, empty positional declarations,
  and validated/provenance-preserving form aggregate examples. The current check
  passes **39 checks** in `review-01/fixes-check-01.log`:19 protocol examples,
  thirteen compatibility contexts and seven canonical configuration/provenance/
  Rust-v2 checks. A fresh readonly review snapshot follows these fixes.
- Full-runner integration is host-verified at
  `target/sdk-full-runner-check-20260910-04/HANDOFF.md`:766 stages/762 functional/
  217 frozen harness invocations. Its new editor scenarios, full native matrix,
  92 cost rows and numerical qualification remain pending; later v3 native
  declarations are being added by the existing runner owner.
- The runner's latest follow-up is
  `target/sdk-full-runner-check-20260910-06/HANDOFF.md`:818 stages/814 functional/
  243 harness invocations, with no missing suite/named/support source at that
  checkpoint. All92 cost rows and four numerical gates remain required.
- Standards' `review-01/fixes-02/standards.md` is clean. The positional reuse
  follow-up has retained9-versus3 red output and27 passing example/configuration
  checks, using actual prefix/tail identities separate from schema sources.
  Spec's latest remaining dual-role HTTP/schema diagnostic witness is reproduced
  in `fixes-02/dual-role-red-01.log`; the narrowly role-aware fix is under the
  current `dual-role-and-nine-resources-green-01.log` check.
- The six initially completed resource adapters passed their canonical
  generation/capture bridge in `canonical-resources-six-01.log`. Go, C++ and C#
  now have final v3 receipts and their production assets are registered; their
  public APIs retain the existing dispatch signatures. The bridge now includes
  these nine adapters. Native evidence indices retain their stated raw-log limits.
- Go's original four-slot/source-entry example-binding regression passes in
  `example-bindings-followup-01.log`. The focused native Go docs/reflection/sample
  gate passes on1.23.12 in `go-native-docs-floor-01.log`; current-tier follow-up is
  running. These exercise Main's changed helper, not a restarted full Go matrix.
- Both focused Go native documentation/reflection/sample tiers now pass; their
  retained fixture roots and honest log scope are indexed in
  `go-native-docs-receipt-01.json`.
- The latest shared review counterexample remains **open**: an OAS3.0 Reference
  Object used as both a schema and a Response must ignore its `$schema` sibling.
  Main now uses canonical applicability for the exact HTTP role too, but the
  source still exposes a genuine IR `invalid-reference`/missing-scope defect.
  `review-01/fixes-03/dual-role-ignored-sibling-green-01.log` retains that failure;
  fourteen other contexts and all eight configuration/provenance checks pass.
  Existing IR owner `ses_f73ec7f9fffeghYNqDw7Np3YCr` is repairing that source-scope
  cause. Both reviewers retain the open disposition; invalid references are not
  hidden. Terraform's single reserved owner starts after the actual recheck clears.
- TypeScript completed native v2/v3; its existing expanded configuration is
  already shared by Backend/capture. Its reported ignored-style/content-plan
  defect is assigned to the existing protocol owner, with the native guard
  retained until the affected descriptor and native witness are repaired.
- The latest runner-only checkpoint is
  `target/sdk-full-runner-check-20260910-08/HANDOFF.md`:828 stages/824 functional/
  248 invocations, including final TypeScript matrices and Java aggregate cases.
  Source checks and41 host tests pass; real full acceptance is still pending.
- Remaining gates: native v2 completion and the explicit resource/dynamic profile,
  current-source native/docs/quality/independent review, the full92 native-cost
  observations, strictly qualified numerical performance, and Terraform stretch.
  No performance calibration is running during native builds.

## Completed evidence, without repeating it

The original five-target report and demo assessment remain immutable. They verify
their pinned bounded slice, not the actively changing full-plan checkout.

New native base-profile matrices completed:

| Target | Verified native tools | Evidence |
| --- | --- | --- |
| Ruby | 3.3.12 / 4.0.6 | `target/sdk-ruby-verification/report.json` |
| C# | .NET 8.0.424 / 10.0.400 | `target/sdk-csharp-verification-20260910/report.json` |
| Dart | 3.9.4 / 3.13.3, including compiled JS | `target/sdk-dart-verified-20260910/report.json` |
| C++ | Declared C++20/libcurl profile | `target/sdk-cpp-verification-20260910/report.json` |
| Kotlin | JDK21/25; Kotlin2.4.20/coroutines1.11/Dokka2.2 | `target/sdk-kotlin-json-verified/REPORT.md` |
| PHP | 8.3.32 / 8.5.8 | `target/sdk-php-verified-20260910-02/report.json` |
| Java | JDK21.0.12.1 / 25.0.4.1; Maven3.9.16 | `target/sdk-java-verification/REPORT.md` |

All twelve backends now have native **base-profile** evidence. Each matrix includes
native packages/consumers, M2, five actual OpenRouter operations, shared vectors,
types, wire/resource controls and native docs. These are **base-profile** results;
advanced protocol admission needs additional native evidence.

Other completed work:

- Pinned acquisition/provider library: 61 tests and native CLI-routine evidence.
  `target/sdk-acquire-integration.md` and `target/sdk-acquire-evidence.md`.
- Public acquisition → offline codegen/session/watch/compare flow passed, with a
  real loopback-acquired schema, original entry deleted and server stopped:
  `target/sdk-pinned-cli-integration-02.log`.
- Pinned sessions passed 3 cache/logical-identity tests plus 8 existing session
  regressions: `target/sdk-pinned-generation-integration-02.log`.
- Shared HTTP protocol's original 20 tests and all 31 expanded source-backed
  seam tests passed; Method comparison recursion was corrected and tested.
  `target/sdk-http-protocol-phase2-seam-01.log` records the expanded matrix.
- OAS3.2 Contract indexing: 20 new tests and Lossless/Fast parity. See
  `docs/SDK-OAS32-CONTRACT.md`. Main updated the obsolete QUERY/COPY refusal test;
  its focused follow-up passed.
- Owned schema dialect work: 21 new dialect tests, 1,020 normative evaluations;
  Main fixed and verified the old blanket 3.0 refusal assertion.
- Five native model/codec dialect consumers passed for representable 3.0.4/3.1.2
  and mixed external dialects. `target/sdk-schema-phase2-handoff.md` records exact
  tool/floor scope. Shared helpers now live at `crate::schema_view`.
- Source-role model names passed four public-plan/native-constructor checks and
  existing Rust/TS/Go/Python model regressions. New examples use names such as
  `CreateKeysRequest`; the original `target/sdk-demo/` packages stay historical.
- Python public operations module, intentional root helpers, and native task
  guides passed installed-wheel/mypy/Sphinx/wire gates on 3.11/3.14:
  `target/sdk-python-dx-result.json`.
- Ruby canonical backend/session/compatibility tests passed:
  `target/sdk-ruby-canonical-integration.log`.
- Timing tooling: 35 harness checks passed; process CPU/IO/fault/scheduler
  attribution and fixed-order controls implemented. Eight real attribution smoke
  samples passed raw-report validation; 288 historical dimensions were analyzed
  read-only. These are observational, not a qualified latency baseline.
- DX comparison is saved in `SDK-DX-ASSESSMENT.md` and the commit-pinned
  `SDK-SPEAKEASY-RESEARCH.md`.

Independent shared-core review found and reproduced redirect-base, ambiguous
dialect, fragment-version, multipart false-prefix and example-provenance bugs.
Main has implemented fixes and retained their red/green witnesses under
`target/sdk-core-review-20260910/`. The first fix snapshot is `fixes-v1/`.
Eleven protocol/example regressions, two new
dialect-context regressions, the 21 dialect tests, 1,020 normative evaluations,
and the full IR/schema suites pass. The original review reports remain unchanged.
All-target, warnings-denied IR/schema Clippy also passes after these fixes.

The context follow-up is now at `fixes-v3/`: reused HTTP fragments propagate all
new contexts, and structural schema positions determine dialect ancestors.
Eight context regressions, full IR/schema tests and warnings-denied Clippy pass.
Both scoped reviews are clean: `standards-final.md` and
`spec-http-context-final.md` (fourteen independent Spec rechecks passed).
Prior snapshots/reports and red witnesses remain preserved.

Expanded native protocol results are now recorded for Go, Rust, Ruby, TypeScript,
C#, Kotlin, Dart and C++, plus Swift's completed standard-capability follow-up. Swift passed nine
focused native cases plus fifteen regression cases on both supported toolchains,
including types, SwiftPM, DocC, TLS, wire and cleanup. Main registered its eleven
new protocol files in compatibility provenance. TypeScript is completing further
standard capabilities; C# is adding finite positional multipart.
Each language guide records its actual toolchains,
admitted operations, native tests and explicit source-linked boundaries.

## Earlier integration checkpoints

- `Backend::ALL` drives CLI profile choices. Ruby is registered by default.
- C#, Dart, C++, Kotlin and PHP have feature-gated Backend adapters and shared
  package-config functions in `backend.rs`; their compatibility adapters are
  actively being completed before default promotion.
- Default codegen features now include `ruby-sdk`, `csharp-sdk`, `java-sdk`,
  `kotlin-sdk`, `dart-sdk`, `cpp-sdk` and `http-protocol`. Each optional SDK feature
  declares its protocol dependency. Java's 15 canonical integration tests and 4,535 native
  metadata assertions passed; Java is now in protocol expansion.
- `generation_session::Input` supports `File` and cache-only `Pinned` inputs.
  Pinned cache verification precedes cache hits; manifest/provider fingerprints
  determine logical revision identity. Cache paths never become schema SourceIds.
- CLI `acquire`, `codegen --pins`, session `pins` configuration and pinned
  `codegen-compare` are wired and the public process flow has passed.
- Core protocol adoption of indexed 3.2 querystring/custom methods and streaming/
  positional encoding roots is complete. Native protocol implementation is active
  per language; shared planner support is not native SDK acceptance.
- Python's compatibility capture must include public exports/module identities;
  its protocol owner is assigned that update. Provenance now fingerprints the
  Python docs/native-example emitters.
- C# compatibility/native descriptor integration passed 11 tests and is now enabled
  by default. Dart/PHP adapters are saved with tests awaiting the aggregate build;
  C++ passed its 15 source-subset integration checks. Their native package evidence
  remains unchanged.
- Main fixed Kotlin's fallible `render()` propagation and reported simple Clippy
  findings in IR shape/walk and session manifest verification. Broad quality checks
  follow integration; avoid collecting performance timings amid native builds.
- The corrected QUERY/COPY IR follow-up and all-target IR Clippy now pass.
- `Contract::effective_schema_closure`, `Schema::ignores_ref_siblings`, and
  `Contract::schema_diagnostic_applies` now own shared traversal/applicability.
  Ambiguous context cannot be suppressed as an ignored 3.0 keyword sibling.
  Redirected entry/dependency identities use effective Workspace handle URIs.
- CLI profile discovery (`codegen-profiles`) and editor support for discovered
  profiles, native import overrides and pinned-manifest identity are implemented.
  Editor TypeScript compilation and the two new real CLI/profile identity tests
  passed. The broader editor run exposed obsolete refusals and missing discovery
  replies in UI mocks; the editor owner is repairing them with positive/negative
  protocol witnesses.
- The new real CLI/editor profile-discovery, Ruby namespace generation/drift,
  and pinned-manifest identity tests passed: `target/sdk-full-plan-editor-profile-tests.log`.
- Main's broader compatibility attempt exposed three Go snapshot regressions
  (source/prose in native equality and changed exact-status representation).
  The Go capture was corrected to retain actual native interface descriptors;
  all 29 established-target compatibility tests passed in
  `target/sdk-five-protocol-compatibility.log` after that correction.
- Shared protocol-aware examples and response-body suppression passed five tests
  in the byte-exact source-subset harness. Main added the RFC9110 HTTP205 witness,
  observed it fail, then fixed codec-root and response-dispatch suppression.
  Evidence: `target/sdk-protocol-205-red.log` and
  `target/sdk-protocol-examples-check-06.log`. The latest snapshot includes the
  owner's Method comparison fix. Earlier attempts remain preserved.
- Canonical `GenerationOptions` now carries explicit versioned interpretation
  across backend, CLI, session cache/revision and compatibility capture. See
  `SDK-INTERPRETATION-PROFILES.md`. Four option checks, eight session regressions
  and twelve protocol/example tests passed for the established five backends:
  `target/sdk-generation-options-regressions-02.log`. Real CLI generation/options,
  sessions, comparisons and pinned-input tests also passed:
  `target/sdk-cli-options-integration-01.log`.
- The all-feature attempt passed all four interpretation checks across twelve
  backends plus C++ (17), Dart (12), Kotlin (16), Ruby (3), and C# (10 host)
  integrations. Java/PHP retained six/two integration failures, assigned to their
  existing owners: `target/sdk-full-options-integrations-01.log`. Existing native
  metadata opt-ins were not repeated by this host integration run.
- The same integration fixed unavailable Example Object references being treated
  as missing wire schemas. Located example findings and dependency recovery are
  retained; missing wire schemas remain blocking. The red witness is
  `target/sdk-core-review-20260910/example-availability-red.log`.
- The next original-plan schema tranche is active: owned conditional/dependency/
  pattern/contains/unevaluated applicators, and canonical `$id`/`$self` plus
  dynamic-reference context indexing. Existing v1 native evidence remains pinned.
- Kotlin completed its JDK21/25 rich/profile matrices and sixteen canonical
  integration checks. Dart completed current/floor VM/Node, strict analysis,
  dartdoc, examples and a real Chrome153 consumer. C++ completed nine installed
  CMake/Doxygen packages with 89 wire exchanges. Their asset closures are now
  registered and these three language features are enabled by default.
- The editor's 122 complete process/mock checks and 101 protocol/UI checks pass.
  A real installed VSIX on graphical VS Code1.137.0 also passed 15 lifecycle and
  seven focused command checks, preserved all63 owned SDK files' bytes/metadata,
  and retained fourteen screenshots: `target/sdk-editor-protocol-native-host-01/`.
  That immutable CLI/VSIX evidence advertised eight profiles at its snapshot.
- Python's 75-request protocol/lifetime/doc checks passed on3.11/3.14, but its
  original final report retained one null-only-model mypy blocker. Main fixed
  annotations and aliases in `python_models.rs`; generated model/codec, strict
  mypy, three negative controls, omission/null, alias identity, oneOf and mutable
  encode checks now pass on both interpreters in
  `target/sdk-python-null-models-{floor,current}-06.log`. The owner then regenerated
  the66-operation SDK, installed fresh wheels, passed full-package strict mypy on
  both interpreters and rendered1,049 Sphinx symbols with warnings denied. The new
  completion is `target/sdk-python-protocol-completion-01/`; the old blocker report
  is intact. This closes Python's v1 protocol gate, not schema-v2 acceptance.
- Those focused null-model checks used exact current model/codec source with a
  frozen v1 schema dependency while v2 was unbuildable. Source/attempt manifests
  are in `target/sdk-python-null-harness-01/`. A complete last-reviewed v1 schema
  snapshot for isolated protocol fixes is now compiled and pinned at
  `target/sdk-schema-v1-full-plan-snapshot-01/`; it does not establish v2 support.
- Main reproduced and repaired two wire-comparison context errors: ignored3.0
  reference siblings creating false unknowns, and malformed unselected operations
  on a selected path tainting comparisons. Red real-CLI evidence is retained in
  `target/sdk-compatibility-context-red-{01,02}/`. The three new context tests are
  awaiting aggregate compilation; no green result is claimed yet.
- The new schema-v2 instruction set currently requires Rust/Swift typed-emitter
  adoption. Both existing owners are assigned source-linked admission fences
  followed by native v2 interpretation. Current default/all-feature build blockers
  are recorded in `target/sdk-compatibility-context-green-01.log` (not a passing run).
- The native cost collector has27 host checks and40 real phase observations across
  five languages/both tiers, with160 codec round trips and30 loopback SDK requests.
  The full92-row fresh run still needs the other52 dimensions. See
  `docs/SDK-NATIVE-COSTS.md`; use the restored Swift floor toolchain specified there.

## Active ownership / recovery sessions

Resume these existing sessions after interruption, preserving completed checks.
Do not start duplicate owners. Completed source-acquisition, schema-dialect and
IR-indexing work does not need to be restarted.

| Work | Session | Owned area |
| --- | --- | --- |
| Java protocols (registration/compatibility completed) | `ses_f742beae0ffeOv7erDs00JyGLy` | Java backend/tests/docs and `compatibility/java.rs` |
| C# finite positional multipart follow-up | `ses_f742beae0ffdvBDl4bsjJLnHbx` | Broad protocols/selector/base/compatibility passed; C# backend/tests/docs and adapter |
| Kotlin protocols — completed | `ses_f742beac2ffehMN0DeR948kJkr` | `target/sdk-kotlin-protocol-verified/REPORT.md`; resume for v2/new findings |
| PHP protocols; first verify 11 saved integration tests in isolation | `ses_f742beac1ffd4fPwu2PXw6R9wK` | PHP backend/tests/docs and `compatibility/php.rs` |
| Dart protocols — completed | `ses_f742beac1ffcAvJ8LodYZuDAA2` | `target/sdk-dart-protocol-20260910/report.json`; resume for v2/new findings |
| C++ protocols — completed | `ses_f742beaafffee7oviEec2NpiYf` | `target/sdk-cpp-protocol-verification-20260910/report.json`; resume for v2/new findings |
| Ruby protocols — completed | `ses_f742beac1ffeaEHFKdf2YZjfoz` | `target/sdk-ruby-protocol-verification/report.json`; resume only for findings |
| Shared protocol / new IR adoption — completed | `ses_f74241921ffeeFkn68meD60cAn` | `http_protocol*` and its tests/docs; resume only for findings |
| TypeScript native compatibility / remaining standard protocols | `ses_f73d087bfffeFGg0Qfhsv2OQKL` | Initial native matrix passed; TS capture function and TS-owned HTTP/package/tests/docs |
| Go protocols — completed | `ses_f73cc3d90ffea31f659Zum1rON` | Go HTTP/runtime and protocol tests/docs; resume only for findings |
| Rust scoped validation v2 | `ses_f73cc3d90ffdYk64FlMfNGe4rQ` | Rust validation/emitter/runtime and necessary model/codec admission; completed protocol evidence retained |
| Swift scoped validation v2 | `ses_f73cc3d89ffejhQElxcPpk9cDC` | Swift validation/emitter/runtime and necessary model/codec admission; completed protocol evidence retained |
| Python protocols — completed | `ses_f73f82130ffen8pLYsl1meVCR4` | Full-package typing completion in `sdk-python-protocol-completion-01/`; resume for v2/new findings |
| Full acceptance runner — follow-up completed | `ses_f739ce66effeDLHhy3mAbuOCZB` | 655 stages/651 functional/172 harness invocations; native-host input contract remains to integrate |
| Native SDK cost collector — implemented | `ses_f735ad0f9ffedC7SHS1oWpyReX` | 40 observations verified; integrated92-row run remains Main/full-runner work |
| Shared-core standards review — clean | `ses_f73981171ffeoB6A6ovypgsD5w` | `target/sdk-core-review-20260910/standards-final.md` |
| Shared-core specification follow-up — clean | `ses_f73981171ffdlWHh5znrzZwXAI` | `target/sdk-core-review-20260910/spec-http-context-final.md` |
| Editor native-host input generalization | `ses_f7340c274ffe1qSTEfMu0Dols6` | Completed graphical evidence retained; making strict fresh CLI/tool pins usable by full runner |
| Owned schema applicators | `ses_f7425fe23fferD70sGijTqihyi` | `suspect-schema/src/owned/**`, new normative/adversarial tests/fixture and docs |
| Canonical resource/dynamic indexing | `ses_f73ec7f9fffeghYNqDw7Np3YCr` | `suspect-ir/src/contract/**`, necessary reference APIs, tests/docs |

Native protocol owners may update only their own capture function in
`compatibility/native.rs`. New-language compatibility owners may register only
their own cfg module and dispatch arm. Main owns shared registry/default features,
provenance, CLI, sessions, editor integration, acceptance and performance tooling.

The acceptance-runner work is now delegated to its listed owner; Main should not
edit those xtask files while it is active. Shared-core reviews use the exact
54-file dirty-entry comparison captured in `target/sdk-core-review-20260910/`
(no commits represented), independently on standards and specification axes.

Java registration is explicitly delegated to its owner: Maven `group:artifact`,
Java package from `import_name` or group, canonical client class `Client`.
An optional Maven group override will separate coordinates from the Java package
while preserving the original library defaults. Its native base gates (seven
planning tests and four package gates on each JDK) are complete and should not
be restarted merely to resume integration.

Aggregate builds can be temporarily broken during active native/schema migrations.
A real CLI build passed in `target/sdk-full-plan-profile-cli-build-02.log`; the
canonical options/CLI integration also passed in
`target/sdk-cli-options-integration-01.log`. Do not repair another active owner's
intermediate code or rerun completed native base matrices merely because the
server restarted.

## Constraints and next work

### Immediate Main continuation after restart

Latest continuation supersedes the earlier active-owner list below:

- Shared scoped applicators v2 completed: nine operations, twelve focused tests,
 32 independent vectors and395 in-scope official cases. `compile` keeps v1;
  `compile_v2` opts into the new instructions and leaves base-program bytes alone.
  See `SDK-SCHEMA-APPLICATORS.md` and `SDK-SCHEMA-NATIVE-ADOPTION.md`.
- All twelve existing native owners now have v2 adoption work. Java/PHP and the
  TypeScript protocol follow-up finish their current evidence first. Native owners
  preserve completed v1 matrices and add source-driven32-case/new-SDK witnesses.
  Rust and Swift already restored compilation with explicit admission fences.
- IR canonical resources completed:18 new tests,94 full-IR passes and29 context
  checks. `SDK-CONTRACT-RESOURCES.md` is the public API handoff. The schema owner
  (`ses_f7425fe23fferD70sGijTqihyi`) is integrating resources/dynamic scope under an
  additive explicit executable profile, preserving the frozen v2 contract. The
  shared protocol owner (`ses_f74241921ffeeFkn68meD60cAn`) is consuming resource
  provenance/base APIs separately from native capability enablement.
- C# finite positional multipart completed:355 checks and22 loopback requests on
  both .NET SDKs, plus five focused tests. Main registered its two new assets.
- Main's three new compatibility-context tests and all29 existing compatibility
  tests PASS in `target/sdk-main-context-and-docs-integration-03.log`. That combined
  run remains failed because a separate Go example-binding regression is open.
- Go's ordinary referenced JSON body is recorded twice in example bindings, and
  neither body record includes its validated entry index. Python's same fixture
  passes. Full reproduction: `target/sdk-example-bindings-regression-detail-01.log`.
  Go owner `ses_f73cc3d90ffea31f659Zum1rON` is repairing this; keep the existing
  count/source-entry assertions. The impossible optional parameter stays omitted.
- The reusable real VS Code host input/report contract is still being generalized
  by `ses_f7340c274ffe1qSTEfMu0Dols6` for the full runner. Its completed15+7 graphical
  checks remain intact. Runner integration follows that concrete handoff.
- Source-linked v2 native gates, final integrated native/quality/cost acceptance,
  qualified numerical performance and the Terraform stretch remain outstanding.

- Completed native matrices stay completed. Only the nine currently active rows
  (Java, PHP, TypeScript follow-up, C# follow-up, owned schema-v2, IR resources,
  Rust-v2, Swift-v2 and editor input generalization) were resumed after the latest
  user-reported restart. No new duplicate owner was created.
- Main's `sdk_compatibility_context` fixes are implemented in
  `compatibility/schema.rs` and `compatibility/wire.rs`. The current check is
  `target/sdk-main-context-and-docs-integration-03.log`, covering three new context
  tests plus the29 existing compatibility cases. Earlier attempts are retained.
- `m3_native_docs.rs` now uses optional JSON schema accessors, Go's
  `can_succeed`/`can_fail` status membership and actual variadic input signatures.
  Its target compiles in the same pending check; native reflection/doc execution
  still needs a justified focused run after that compile succeeds.
- `python_null_models.rs` has passed both interpreters in the exact-source
  subset with a frozen v1 schema. The current-crate version is required by the
  full runner; do not confuse its normal ignored result with native execution.
- Remaining native runtimes must adopt the schema-v2 instruction/annotation
  contract after the schema owner publishes it. Rust/Swift are the first active
  adopters; the other ten owners retain their v1 protocol evidence meanwhile.
- `sdk-full`'s latest runner-only evidence is
  `target/sdk-full-runner-check-20260910-03/REPORT.json`:35 tests, Clippy, inventories
  and source checks passed. Full native acceptance and all92 cost rows remain
  pending. Historical inventories/reports were preserved.
- Main registered Kotlin, Dart, C++ and Swift protocol assets and enabled the
  completed Kotlin/Dart/C++ language features. The default registry now contains
  eleven languages; PHP remains opt-in until its active integration completes.

### Continuing constraints

- Keep `/Users/luke/github/openrouter-web` read-only, including unrelated `knip.json`.
- No commits, resets/clean operations, package publishing, upstream edits or merges.
- Preserve `target/sdk-full-plan-entry-20260910/` and earlier sealed reports,
  frozen binaries and interrupted calibration attempts.
- Complete new-language registration/compatibility/provenance and native protocol
  capability witnesses; keep unsupported semantics source-linked and explicit.
- Extend editor/session examples and the integrated native acceptance inventory.
- Run independent standards/spec review and final source/tool/input-pinned checks
  against the actual dirty implementation snapshot.
- Numerical performance still requires fresh qualified evidence under prospectively
  controlled methodology. The failed Mac baseline is not relabeled passing.
- Terraform remains a stretch goal after the core work.
