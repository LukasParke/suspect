# Documentation

Start with the [project README](../README.md) for install and a tour, then
the [command reference](CLI.md). Every command shares one settings file,
`.suspect.yaml` (run `suspect config` to see what is in effect).

## Using suspect

### Command line and editors

- [suspect command reference](CLI.md)
- [LSP Feature Matrix — Full LSP 3.17 Specification Coverage](LSP_FEATURES.md)
- [TS/JS Custom Rules on a Bun Sidecar — Design](TS-RULE-RUNTIME.md)

### Testing and load

- [Load testing with suspect test load](LOAD-TESTING.md)
- [Test credentials](TEST-CREDENTIALS.md)

### SDK and application generation

- [SDK generation capabilities](SDK-CAPABILITIES.md)
- [Generated-SDK feature matrix](SDK-FEATURES.md)
- [Golden SDK defaults — user guide](SDK-GOLDEN-DEFAULTS.md)
- [Explicit runtime environment credentials — v1](SDK-CREDENTIAL-ENV.md)
- [Canonical incremental SDK generation](SDK-INCREMENTAL-GENERATION.md)
- [SDK editor preview and watch](SDK-EDITOR-PREVIEW.md)
- [Source-aware SDK compatibility reports](SDK-COMPATIBILITY-REPORTS.md)
- [Generated artifact ownership](SDK-ARTIFACT-OWNERSHIP.md)
- [Explicit SDK interpretation profiles](SDK-INTERPRETATION-PROFILES.md)
- [Large contract generation](SDK-LARGE-CONTRACTS.md)
- [Validated SDK example planning](SDK-EXAMPLES.md)
- [OpenAPI support matrix for SDK generation](SDK-OPENAPI-MATRIX.md)
- [SDK release checklist](SDK-RELEASE-CHECKLIST.md)
- [Stretch goal: Terraform Provider generation](SDK-TERRAFORM-STRETCH.md)
- [Application targets — Go API CLI and TypeScript stdio MCP server](APPLICATION-TARGETS.md)

### Performance

- [SDK Session performance](SDK-SESSION-PERFORMANCE.md)

## SDK backends by language

Per-language notes: models, codecs, HTTP protocol, validation and credentials.

- **TypeScript / JavaScript**: [codecs](SDK-TYPESCRIPT-CODECS.md), [credential-env](SDK-TYPESCRIPT-CREDENTIAL-ENV.md), [directional](SDK-TYPESCRIPT-DIRECTIONAL.md), [http](SDK-TYPESCRIPT-HTTP.md), [json](SDK-TYPESCRIPT-JSON.md), [model-plan](SDK-TYPESCRIPT-MODEL-PLAN.md), [protocol](SDK-TYPESCRIPT-PROTOCOL.md), [validation-v2](SDK-TYPESCRIPT-VALIDATION-V2.md), [validation-v3](SDK-TYPESCRIPT-VALIDATION-V3.md)
- **Python**: [codecs](SDK-PYTHON-CODECS.md), [dx](SDK-PYTHON-DX.md), [http](SDK-PYTHON-HTTP.md), [json](SDK-PYTHON-JSON.md), [model-plan](SDK-PYTHON-MODEL-PLAN.md), [protocol](SDK-PYTHON-PROTOCOL.md), [resources](SDK-PYTHON-RESOURCES.md), [validation](SDK-PYTHON-VALIDATION.md)
- **Go**: [codecs](SDK-GO-CODECS.md), [credential-env](SDK-GO-CREDENTIAL-ENV.md), [http](SDK-GO-HTTP.md), [json](SDK-GO-JSON.md), [model-plan](SDK-GO-MODEL-PLAN.md), [protocol](SDK-GO-PROTOCOL.md), [schema-v2](SDK-GO-SCHEMA-V2.md), [schema-v3](SDK-GO-SCHEMA-V3.md), [validation](SDK-GO-VALIDATION.md)
- **Rust**: [credential-env](SDK-RUST-CREDENTIAL-ENV.md), [http](SDK-RUST-HTTP.md), [json](SDK-RUST-JSON.md), [model-plan](SDK-RUST-MODEL-PLAN.md), [protocol](SDK-RUST-PROTOCOL.md), [resources](SDK-RUST-RESOURCES.md), [serde](SDK-RUST-SERDE.md), [validation-v2](SDK-RUST-VALIDATION-V2.md)
- **Swift**: [credential-env](SDK-SWIFT-CREDENTIAL-ENV.md), [protocol](SDK-SWIFT-PROTOCOL.md), [resources](SDK-SWIFT-RESOURCES.md), [validation-v2](SDK-SWIFT-VALIDATION-V2.md), [overview](SDK-SWIFT.md)
- **Java**: [overview](SDK-JAVA.md)
- **Kotlin**: [overview](SDK-KOTLIN.md)
- **C#**: [overview](SDK-CSHARP.md)
- **Ruby**: [overview](SDK-RUBY.md)
- **PHP**: [overview](SDK-PHP.md)
- **Dart**: [overview](SDK-DART.md)
- **C++**: [overview](SDK-CPP.md)

## Design notes

Architecture and contract design for the shared SDK pipeline.

- [SDK Contract Readers (M1 parity seam)](SDK-CONTRACT-READERS.md)
- [Contract resources and dynamic-reference metadata](SDK-CONTRACT-RESOURCES.md)
- [Shared HTTP protocol plan](SDK-HTTP-PROTOCOL.md)
- [Native TypeScript/JavaScript and Rust SDK interfaces](SDK-NATIVE-DX.md)
- [OpenAPI 3.2 HTTP Contract indexing](SDK-OAS32-CONTRACT.md)
- [Owned validation boundary](SDK-OWNED-VALIDATION.md)
- [Pinned remote acquisition and offline closure compilation](SDK-PINNED-CLOSURE-DESIGN.md)
- [TypeScript protocol slice: next source-backed expansion](SDK-PROTOCOL-NEXT.md)
- [Remaining native SDK interfaces](SDK-REMAINING-DX.md)
- [Owned applicators: additive v2 executable contract](SDK-SCHEMA-APPLICATORS.md)
- [Owned SDK schema dialect admission](SDK-SCHEMA-DIALECTS.md)
- [Native scoped-validation adoption](SDK-SCHEMA-NATIVE-ADOPTION.md)
- [Exact schema numbers and equality](SDK-SCHEMA-NUMERICS.md)
- [Owned resources and dynamic references: v3 contract](SDK-SCHEMA-RESOURCES.md)
- [Standalone schema URI references](SDK-SCHEMA-URIS.md)
- [SDK elimination audit](SDK-ELIMINATION-AUDIT.md)

## Internal: plans, reviews and history

Plans, milestone exit evidence, dated reviews and research, under
[`internal/`](internal/README.md). Kept for context; not user
documentation and not link-checked. The current alignment plan is the
[toolchain unification proposal](proposals/README.md).

- [Folding Arbiter into suspect — Architecture Plan](internal/ARBITER-FOLD-PLAN.md)
- [Folding charte.rs into suspect — Architecture Plan](internal/CODE-FOLD-PLAN.md)
- [Depth analysis — where suspect is deep, where it is shallow, and what the next iteration should be](internal/DEPTH-ANALYSIS.md)
- [Feature Roadmap — the next ten platform features](internal/FEATURE-ROADMAP.md)
- [Arazzo-driven load testing — Architecture Plan](internal/LOAD-TESTING-PLAN.md)
- [Depth enrichment plan — the thin and at-risk surface](internal/LSP-DEPTH-ENRICHMENT.md)
- [LSP Competitive Analysis: suspect vs vacuum vs Telescope](internal/LSP_COMPARISON.md)
- [Historical performance profiling report](internal/PERFORMANCE.md)
- [Ten Revolutionary Features for the Suspect Platform](internal/REVOLUTIONARY-FEATURES.md)
- [SDK Compatibility Diff — Python: published openrouter 1.1.153 vs generated suspect SDK](internal/SDK-COMPAT-DIFF-PYTHON.md)
- [SDK Compatibility Diff — TypeScript (Published @openrouter/sdk 1.2.133 vs generated @openrouter/sdk-compat)](internal/SDK-COMPAT-DIFF-TYPESCRIPT.md)
- [Drop-in replacement review: suspect SDKs vs the published OpenRouter SDKs](internal/SDK-COMPAT-DROPIN-PLAN.md)
- [Our generated SDKs versus OpenRouter’s current TypeScript, Go and Python SDKs](internal/SDK-CURRENT-SDK-COMPARISON-20260911.md)
- [Hackathon demo: OpenAPI to native SDKs](internal/SDK-DEMO.md)
- [SDK quality and developer experience](internal/SDK-DX-ASSESSMENT.md)
- [Generated SDK DX review — 11 September 2026](internal/SDK-DX-REVIEW-20260911.md)
- [Full SDK plan acceptance](internal/SDK-FULL-EXIT.md)
- [Full SDK plan — live integration checkpoint](internal/SDK-FULL-PLAN-STATUS.md)
- [Remaining original SDK plan — active work map](internal/SDK-FULL-PLAN-WORK.md)
- [OpenAPI-driven SDK generation plan](internal/SDK-GENERATION-PLAN.md)
- [Golden Native SDK Defaults and Complete OpenAPI Support](internal/SDK-GOLDEN-DEFAULTS-PLAN.md)
- [Historical M0 failure baseline](internal/SDK-M0-KNOWN-FAILURES.md)
- [M0–M2 native interface approval](internal/SDK-M0-M2-DX-APPROVAL.md)
- [Historical M0–M2 exit checkpoint](internal/SDK-M0-M2-EXIT.md)
- [Historical M0–M2 workflow measurements](internal/SDK-M0-WORKFLOW.md)
- [M2 native slice call sites](internal/SDK-M2-CALLSITES.md)
- [M3 / M6 exit evidence](internal/SDK-M3-M6-EXIT.md)
- [Native SDK cost observations](internal/SDK-NATIVE-COSTS.md)
- [OpenAPI requirements for faithful SDK generation](internal/SDK-OPENAPI-RESEARCH.md)
- [SDK generation progress](internal/SDK-PROGRESS.md)
- [SDK session handoff — hackathon demo](internal/SDK-SESSION-HANDOFF.md)
- [Speakeasy / OpenRouter SDK research](internal/SDK-SPEAKEASY-RESEARCH.md)
- [Hackathon scope: generation and output SDKs](internal/SDK-TODAY-M3-M6.md)
- [Speakeasy openapi-generation review — patterns for the suspect engine](internal/SPEAKEASY-OPENAPI-GENERATION-REVIEW.md)
