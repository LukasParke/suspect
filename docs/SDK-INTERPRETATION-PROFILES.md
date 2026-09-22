# Explicit SDK interpretation profiles

Ordinary OpenAPI semantics are the default. The canonical backend, CLI,
generation sessions and compatibility reports share `GenerationOptions`.
Package identity remains in the four-field `TargetConfig`; it does not select API
semantics. Native adapters continue to admit only their verified capabilities.

## Legacy binary strings, version 1

`legacy-binary-string-v1` interprets an OAS 3.1/3.2
`{ "type": "string", "format": "binary" }` declaration as raw bytes **only in
binary media or binary part contexts**. These markers occur in the tracked
OpenRouter download operations. Standard 3.1/3.2 generation refuses that
interpretation unless requested explicitly.

JSON media keep their real string models and codecs. The profile adds no JSON
null placeholder, base64 conversion, streaming convention, request flag, retry,
pagination or authentication behavior. It does not rewrite the input document.
OAS 3.0's own binary rules remain dialect semantics.

```sh
suspect codegen openapi.yaml \
  --profile go-http \
  --operation-id downloadContainerFileContent \
  --package-name example.com/openrouter-sdk --package-version 0.1.0 \
  --compatibility-profile legacy-binary-string-v1 \
  --out generated
```

The repeatable CLI flag uses a closed, versioned enum. Unsupported names fail
configuration before emission. `codegen-profiles --format json` advertises
`compatibilityProfiles` separately from native backend profiles. Discovery does
not activate them.

## Sessions and libraries

Add a top-level field to a session configuration when needed:

```json
{
  "spec": "openapi.yaml",
  "operation_ids": ["downloadContainerFileContent"],
  "compatibility_profiles": ["legacy-binary-string-v1"],
  "targets": [{
    "backend": "go-http",
    "package_name": "example.com/openrouter-sdk",
    "package_version": "0.1.0"
  }]
}
```

Omitting `compatibility_profiles` means an empty set. The same configuration is
used for finite generation, watch, check, preview and pinned offline inputs.
Interpretation participates in both revision and per-target cache keys. Changing
it reuses the immutable Contract but replans the affected targets. Removing an
opt-in cannot reuse an incompatible admitted package. Cached reverts retain the
original revision and artifacts.

Library entry points:

- `backend::generate_with_options(contract, selected, target, options)`;
- `SessionConfig::generation`;
- `compatibility::snapshot_with_options(...)` and `compare_with_options(...)`.

Existing generation/snapshot/compare wrappers use empty options. Canonical
TypeScript generation selects the verified `HttpConfig::expanded()` capability
profile; this is independent of optional interpretation. The language-library
`HttpConfig::default()` retains its explicit strict admission profile.

## Compatibility and evidence

Reports retain each side's options. A profile transition produces explicit
`wire-interpretation-profile-changed` and
`native-interpretation-profile-changed` unknowns, including when a particular
JSON operation has identical native types. Source equality alone does not prove
equivalent interpretation. Comparing the same admitted profile preserves normal
wire/native comparison behavior.

`sdk_generation_options` verifies canonical admission/capture, JSON-versus-byte
bindings, cache invalidation, unchanged reuse, removal of an opt-in, profile
transitions and closed configuration names. `sdk_protocol_options` exercises the
real CLI, readonly session previews, comparison and unnamed operations. Native
byte behavior is separately covered by each adapter's protocol matrix; these
configuration tests do not replace those native checks.

Current integration evidence:

- `target/sdk-generation-options-regressions-02.log`: four new option checks,
  eight session regressions and twelve protocol/example tests passed against the
  five established backends.
- `target/sdk-cli-options-integration-01.log`: real CLI option, SDK generation,
  session, comparison and pinned-input tests passed, with the existing costly
  native installation opt-in retained for integrated acceptance.
- `target/sdk-full-options-integrations-01.log`: the four option checks also
  passed across **all twelve** compiled backends. C++, Dart, Kotlin, Ruby and
  C# host integration suites passed. That broader attempt retained Java/PHP
  adapter-regression failures being resolved by their protocol owners.
- `target/sdk-main-final-integration-20260910-01/wire-options-integration-01.log`:
  all twelve default backends' four interpretation checks pass alongside69 total
  host integration checks, including repaired Java/PHP adapters. The current
  CLI's eleven process checks and exact twelve-profile discovery are retained in
  the same evidence root. Native scoped-schema adoption is recorded separately.

Additional-language option integration is verified with its own compiled
features before default promotion.
