# Documentation

Start with the [project README](../README.md) for install, a tour and the
command map, then the [command reference](reference/cli.md). Every command
shares one settings file, `.suspect.yaml` (run `suspect config --explain` to
see what is in effect and where each value came from).

Each directory holds one kind of page:

| Directory | Holds |
|---|---|
| [`guide/`](#guide) | task-oriented pages |
| [`reference/`](#reference) | lookup pages; some are generated |
| [`sdk/<lang>/`](#sdk-backends) | per-backend notes |
| [`design/`](#design) | current architecture and contract notes |
| [`proposals/`](#proposals) | the toolchain unification proposals |
| [`internal/`](#internal) | plans, exit evidence, dated reviews and research (frozen) |

## Guide

- [Checking and linting](guide/linting.md): `check`, rule settings, suppression comments, baselines, custom rules
- [Editors](guide/editors.md): VS Code, Zed and other LSP clients
- [Gateway, recording and replay](guide/gateway.md)
- [Documentation generation](guide/docs-generation.md): `gen docs` and custom templates
- [Golden SDK defaults — user guide](guide/sdk-generation.md)
- [Canonical incremental SDK generation](guide/sdk-incremental-generation.md)
- [SDK editor preview and watch](guide/sdk-editor-preview.md)
- [Source-aware SDK compatibility reports](guide/sdk-compatibility-reports.md)
- [Explicit SDK interpretation profiles](guide/sdk-interpretation-profiles.md)
- [Large contract generation](guide/sdk-large-contracts.md)
- [SDK release checklist](guide/sdk-release-checklist.md)
- [Application targets — Go API CLI and TypeScript stdio MCP server](guide/application-targets.md)
- [Load testing with suspect test load](guide/load-testing.md)
- [Test credentials](guide/test-credentials.md)

## Reference

- [suspect command reference](reference/cli.md) (generated)
- [Diagnostic codes](reference/diagnostics.md) (generated): every code, its check, default severity and aliases
- [Configuration reference](reference/config.md) (generated): every `.suspect.yaml` and `suspect.project.json` key, with its JSON Schema
- [Environment variables](reference/environment.md) (generated)
- [LSP Feature Matrix — Full LSP 3.17 Specification Coverage](reference/lsp-features.md)
- [SDK generation capabilities](reference/sdk-capabilities.md)
- [Generated-SDK feature matrix](reference/sdk-features.md) (generated)
- [OpenAPI support matrix for SDK generation](reference/openapi-matrix.md)
- [Explicit runtime environment credentials — v1](reference/credential-env.md)
- [Generated artifact ownership](reference/artifact-ownership.md)

## SDK backends

Per-language notes: models, codecs, HTTP protocol, validation and
credentials. Each directory's `README.md` is its overview.

- [TypeScript / JavaScript](sdk/typescript/README.md)
- [Python](sdk/python/README.md)
- [Go](sdk/go/README.md)
- [Rust](sdk/rust/README.md)
- [Swift](sdk/swift/README.md)
- [Java](sdk/java/README.md)
- [Kotlin](sdk/kotlin/README.md)
- [C#](sdk/csharp/README.md)
- [Ruby](sdk/ruby/README.md)
- [PHP](sdk/php/README.md)
- [Dart](sdk/dart/README.md)
- [C++](sdk/cpp/README.md)
- [Terraform provider](sdk/terraform/README.md)

## Design

Architecture and contract design for the shared pipeline. Code comments and
tests cite these pages, so they are kept current.

- [SDK Contract Readers (M1 parity seam)](design/contract-readers.md)
- [Contract resources and dynamic-reference metadata](design/contract-resources.md)
- [SDK elimination audit](design/elimination-audit.md)
- [Shared HTTP protocol plan](design/http-protocol.md)
- [Native TypeScript/JavaScript and Rust SDK interfaces](design/native-dx.md)
- [OpenAPI 3.2 HTTP Contract indexing](design/oas32-contract.md)
- [Owned validation boundary](design/owned-validation.md)
- [Pinned remote acquisition and offline closure compilation](design/pinned-closure.md)
- [Remaining native SDK interfaces](design/remaining-native-dx.md)
- [Owned applicators: additive v2 executable contract](design/schema-applicators.md)
- [Owned SDK schema dialect admission](design/schema-dialects.md)
- [Native scoped-validation adoption](design/schema-native-adoption.md)
- [Exact schema numbers and equality](design/schema-numerics.md)
- [Owned resources and dynamic references: v3 contract](design/schema-resources.md)
- [Standalone schema URI references](design/schema-uris.md)
- [Validated SDK example planning](design/sdk-examples.md)
- [SDK Session performance](design/sdk-session-performance.md)
- [TS/JS Custom Rules on a Bun Sidecar — Design](design/ts-rule-runtime.md)
- [TypeScript protocol slice: next source-backed expansion](design/typescript-protocol-next.md)

## Proposals

- [Toolchain unification proposals](proposals/README.md): the index, the
  contracts between proposals and the order; P1 to P8 are linked from it.

## Internal

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
