# Remaining original SDK plan — active work map

Current completed evidence and active owners are recorded in
[SDK-FULL-PLAN-STATUS.md](SDK-FULL-PLAN-STATUS.md).

User instruction, **2026-09-10**: implement the remaining languages and other
remaining items of the original plan end to end. This resumes M4/M5 and full-plan
carryovers after the accepted hackathon checkpoint. The single canonical SDK
pipeline and removal of legacy implementations remain in force.

Entry snapshot: `target/sdk-full-plan-entry-20260910/` contains the actual
uncommitted source archive, hashes and status (2,218 entries). Preserve it and
the earlier immutable reports. The verified five-profile demo remains a
regression baseline, not proof of later-language or protocol support.

## Language delivery

Every target needs retained native descriptors, exact values/presence, checked
codecs, real clients, source-linked diagnostics, package installation, native
documentation, executable samples, positive/negative consumer types and independent
wire/resource tests. Start from the existing drafts and repair them through native
checks. All twelve backends must ultimately share the expanded contract gates.

| Target | Current tranche | Retained native admission scope |
| --- | --- | --- |
| Java | Native base/protocol/v2/v3 verified; aggregate integration | JDK21/25, Maven/Javadoc, sync/async, M2 + actual operations, scoped/resource consumers |
| C# | Native base/protocol/v2/v3 verified; aggregate integration | .NET8/10, NuGet/XML docs, Task/cancellation, positional and scoped/resource consumers |
| Kotlin | Native base/protocol/v2/v3 verified; aggregate integration | JDK21/25, data/sealed/checked carriers, coroutines, Dokka and physical/resource consumers |
| Ruby | Native base/protocol/v2/v3 verified; aggregate integration | 3.3.12/4.0.6 keyword models, exact codecs, gems/YARD/RBS/Steep and resource consumers |
| PHP | Native base/protocol/v2/v3 verified; aggregate integration | 8.3.32/8.5.8 typed models, Composer/PHPStan/PHPDoc and resource consumers |
| Dart | Native base/protocol/v2/v3 verified; aggregate integration | 3.9.4/3.13.3 VM/JS/browser, pub/dartdoc and contextual/resource consumers |
| C++ | Native base/protocol/v2/v3 verified; aggregate integration | Declared C++20/libcurl profile, value/variant/checked carriers, RAII, installed CMake/Doxygen |

All twelve SDK features are now enabled by default, after the individual native
profile gates. Main owns shared registrations, Backend/TargetConfig, CLI/editor,
compatibility adapters and broad acceptance. A compilation-only draft is not an
admitted SDK profile. Legacy emitters or compatibility projections are not restored.

## Shared protocol and semantic work

- [ ] Source-aware capability planning that admits new shapes only for verified adapters.
- [ ] Exact/range/default response precedence, no declared content, binary/text/structured
  JSON media, content dispatch and typed response headers/links.
- [ ] Parameter locations/styles/explode/allowReserved, content parameters, objects,
  headers/cookies and OpenAPI3.2 querystring semantics.
- [ ] No-auth/optional/OR/AND security, bearer/basic/API keys/OAuth scopes and explicit
  credential hooks; effective servers, variables and overrides.
- [ ] Forms, multipart encoding/file bodies and explicit request-media choices.
- [ ] Source-backed streaming/events, OpenAPI3.2 itemSchema and distinct callbacks/
  webhooks; independently verified optional extension profiles where needed.
- [ ] Expanded native composition/presence/intersections, heterogeneous literals,
  typed/pattern extras, tuples, unevaluated/dynamic schema semantics and directional
  codecs; correct declared OpenAPI3.0/3.1 profiles and explicit3.2 coverage.
- [x] Pinned remote acquisition/offline dependency cache, separate refresh and
  compilation, complete source/provenance and missing/stale-resource handling.
- [ ] Full package/docs/examples/type/wire/cancellation/resource/name matrices for
  each newly admitted language and protocol capability.
- [ ] Updated per-language compatibility records and CLI/editor/acceptance routing.

Shared protocol/resource and native scoped-schema implementations have advanced
through their independent witnesses. The remaining checkboxes include final
integrated acceptance and the last located findings; they are not a claim that
the already verified individual matrices need restarting. Current receipts are
indexed in [SDK-SCHEMA-NATIVE-ADOPTION.md](SDK-SCHEMA-NATIVE-ADOPTION.md) and the
live Main checkpoint.

Use ordinary specifications plus independent normative/adversarial vectors. The
four tracked OpenRouter inputs remain read-only. Upstream source defects remain
visible; do not repair them silently or infer auth/stream/retry/pagination behavior
from names. XML and opaque payloads require faithful source-backed representations.

## Remaining maintenance/release work

- [x] Customer-facing DX: readable inline names, convenient simple calls, public
  error/helper exports and complete native-construction quickstarts. The user's
  quality question and actual package inspection are captured in
  [SDK-DX-ASSESSMENT.md](SDK-DX-ASSESSMENT.md); passing native gates alone does not
  establish this usability bar. [DEMO-README.md](../DEMO-README.md) now provides
  detailed, native-checked usage/DX for all twelve SDKs, an independent JavaScript
  consumer, a 5–10 minute offline run order, typed errors, absence/null handling
  and a source-edit/compatibility walkthrough. The prepared candidate's thirteen
  positive and thirteen typed-error consumers pass; final integrated acceptance
  remains tracked separately below.
- [ ] Complete current-source capability/coverage reports and native migration notes.
- [ ] Broader source-change invalidation and interactive preview/performance checks.
- [ ] CPU/wall/I/O and workload-order/correlation-aware measurement methodology,
  then prospectively agreed numerical budgets and fresh baseline/candidate evidence.
- [ ] Native SDK build/import/codec/request/size measurements, beyond generation timing.
- [ ] Full integrated native/quality/security/source-integrity gates and independent reviews.

The earlier Mac timing baseline is retained as unqualified observational evidence;
it cannot be relabeled passing. No calibration job is running. Performance work
resumes through controls/methodology, rather than another identical long collection.

The original archived plan is available in the entry archives. Its legacy migration
items were explicitly superseded by the user's greenfield decision; its contract,
runtime, packaging, documentation and verification requirements remain applicable.

## Stretch goal — Terraform Provider generation

User-added **2026-09-10**. Follow the core SDK/protocol deliverables with a separate
Terraform Provider output target, **built on the generated Go SDK** (subsequent
explicit user requirement). The canonical contract retains mapping/source identity;
every API call uses the Go SDK so its improvements flow through the dependency.
This is an additional artifact target, not another SDK programming language.

- [x] Generate a Go provider using the Terraform Plugin Framework and a pinned
  generated Go SDK module dependency, with resources
  and data sources selected through explicit, versioned lifecycle mappings.
- [x] Map create/read/update/delete, identity/import, request/response paths and
  Terraform state/schema behavior explicitly; retain source-linked diagnostics.
- [x] Preserve Terraform unknown/null states, computed/sensitive/write-only
  attributes, drift/refresh and replacement semantics under that mapping profile.
- [x] Emit installable provider packages, native documentation and runnable HCL
  examples; verify init/validate/plan/apply/refresh/import/destroy against controlled
  API fixtures and the declared Terraform/toolchain versions.

The bounded lifecycle-v1 implementation has its original native lifecycle and
supplemental receipts, focused fixes on both Go tiers, and clean independent
Standards/Spec rechecks at
`target/sdk-main-final-integration-20260910-01/review-terraform-01/fixes-01/`.
The [reviewed local walkthrough](../examples/sdk-demo-all/terraform-reviewed/README.md)
uses a newly built provider from that fixed output and passes ten actual local
Terraform steps. These implementation/native milestones do not close the
remaining original-plan integrated SDK/editor/cost/performance gates.

See [the stretch-goal scope](SDK-TERRAFORM-STRETCH.md). The core SDK acceptance
inventory continues to describe the SDK deliverables independently.
