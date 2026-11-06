# Changelog

All notable changes to suspect are listed here. Versions follow the policy in
CONTRIBUTING.md ("Releasing"): before 1.0, a release that removes or renames a
command, flag, config key or exit code bumps the minor version.

## [0.1.0] - Unreleased

First release. The history before it has no tag to diff against, so this
entry is a summary rather than a generated list.

### Install

- Prebuilt `suspect` binaries for macOS (arm64, x86_64), Linux (glibc and
  static musl, x86_64 and arm64) and Windows (x86_64, tier 2), with shell and
  PowerShell installers and a Homebrew formula (`LukasParke/tap/suspect`).
- The VS Code extension on the Marketplace and Open VSX, and the Zed
  extension in the Zed registry.
- The `LukasParke/suspect/.github/actions/suspect` GitHub Action downloads
  the release binary instead of building from source.

### Features

- Lossless tree-sitter parsing of OpenAPI 3.0, 3.1 and 3.2 (2.0 read through
  family sniffing), Arazzo 1.0 and 1.1, and Overlay 1.0 and 1.1.
- `$ref` resolution across files with cycle classification; JSON Schema
  2020-12 validation; a Spectral-compatible linter with OpenAPI, security and
  design rule packs; TypeScript custom rules on a Bun worker.
- `suspect lsp`: diagnostics, hover, completion, go-to-definition and
  references across `$ref`s, rename, formatting, semantic highlighting, inlay
  hints and quick fixes, with VS Code and Zed clients.
- SDK generation for twelve languages (TypeScript/JavaScript, Python, Go,
  Rust, Swift, Java, Kotlin, C#, Ruby, PHP, Dart, C++), documentation
  generation (HTML, Markdown, SvelteKit) and application targets (Go CLIs,
  TypeScript MCP servers).
- Arazzo workflow testing against live servers or recorded cassettes,
  contract and load testing, a mock gateway with recording, replay and drift
  analysis.
- `suspect check` runs every check the editor runs (syntax, `$ref`s,
  OpenAPI, Swagger 2.0, Arazzo, Overlay, load profiles, lint, markdown,
  vendor extensions, configuration files, TypeScript rules, and SDK
  readiness when codegen is configured) under one policy, and exits 1 on
  an error-severity finding; `validate` and `lint` are filters of it
  (`--only`). Inline (`# suspect-disable`, `x-suspect-disable`) and
  baseline (`--baseline`, `check.baseline`) suppression. Text, JSON, SARIF
  and JUnit output. The parse census `check` used to print is in `stats`.
  Finding JSON is `{"findings": [...], "suppressed": [...]}` with
  lowercase severities, and lint rules that duplicate a validation code
  report under that code (the old lint code still configures it).
- `suspect ci` as a workspace gate (one `check` stage) with text and JSON
  output (JUnit for load runs); Swagger 2.0 to 3.1 upgrade; semantic and
  breaking-change diff; Overlay apply, explain and synthesis.
