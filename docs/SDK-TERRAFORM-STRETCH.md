# Stretch goal: Terraform Provider generation

Added by the user **2026-09-10**, after resuming the original SDK plan.

## Goal

Generate an installable Terraform Provider **built on the generated Go SDK**.
Use Go and the Terraform Plugin Framework, with native provider schemas,
resources/data sources, documentation and HCL examples. The shared OpenAPI
Contract provides source bindings for the provider's lifecycle mapping plan.

User requirement, 2026-09-10: the provider consumes the Go SDK so it benefits
from SDK improvements. All API calls go through generated Go SDK operations.
Authentication, serialization, checked codecs, validation, transport and response
handling belong to that dependency. Provider code owns Terraform schema/state,
planning, lifecycle and diagnostics, and passes Terraform's context to SDK calls.

This is a stretch deliverable after the core SDK language/protocol work. It is
tracked as a separate artifact target rather than a thirteenth SDK backend.

## Inputs and semantics

OpenAPI continues to own HTTP, request/response and schema semantics. Terraform
lifecycle semantics require an explicit, versioned mapping profile:

- Resource/data-source identity and selected operations.
- Create/read/update/delete bindings, request inputs and response-to-state paths.
- Stable ID, import format and read-after-import behavior.
- Required/optional/computed attributes, unknown/null handling, sensitive and
  write-only state behavior.
- Refresh/drift, replacement, missing-resource and partial-failure behavior.
- Explicit polling or retry behavior where a resource lifecycle requires it.

These mappings must not be inferred from operation names. Package identity and
provider/toolchain versions remain build configuration. The generated provider
declares and pins its Go SDK module dependency. Regeneration and SDK upgrades
are verified together so improvements flow through without a second HTTP runtime.

## End-to-end acceptance

Verify emitted provider packages and a separate Terraform consumer through
`init`, `validate`, `plan`, `apply`, `refresh`, `import` and `destroy` against
controlled API fixtures through the Go SDK's injectable transport. Include drift,
unknown/null values, failed/partial
operations and cancellation. Generated documentation and HCL examples must match
the actual provider schema and pass the same native verification workflow.

## Implemented artifact boundary

`suspect_codegen::terraform` is an independent artifact target. It is feature
gated by the existing `http-protocol` feature and adds no SDK language/backend.

```rust
parse_mapping(&str) -> Result<MappingProfile, serde_json::Error>
plan_provider(Arc<Contract>, MappingProfile, TargetConfig)
    -> Result<ProviderPlan, Vec<Diagnostic>>
emit_provider(&ProviderPlan) -> Vec<OutFile>
```

`ProviderPlan::sdk_plan()` exposes the actual canonical `go_http::HttpPlan`.
`mapping()` and `config()` expose the admitted lifecycle and package identities.
Admission completes before any desired artifacts are returned to the writer.
`Diagnostic` retains `code`, `message`, `mapping_pointer`, physical OpenAPI
`source`, and its byte range `at`. The emitter does no I/O. Call the existing
ownership-aware artifact writer with a stable Terraform-specific owner.

### CLI

Main's separately registered command uses the same parse/plan/emit boundary:

```sh
suspect codegen-terraform \
  crates/suspect-codegen/tests/fixtures/terraform-v1/openapi.json \
  --mapping crates/suspect-codegen/tests/fixtures/terraform-v1/mapping.json \
  --target-config crates/suspect-codegen/tests/fixtures/terraform-v1/target.json \
  --out generated-terraform --format json

# Read-only ownership/drift check, including absent/current/user-edited outputs.
suspect codegen-terraform \
  crates/suspect-codegen/tests/fixtures/terraform-v1/openapi.json \
  --mapping crates/suspect-codegen/tests/fixtures/terraform-v1/mapping.json \
  --target-config crates/suspect-codegen/tests/fixtures/terraform-v1/target.json \
  --out generated-terraform --check --format json
```

Use `--pins <manifest> --cache-dir <directory>` instead of the positional source
for canonical cache-only pinned acquisition inputs; existing test-origin policy
also applies. Mapping and target configuration remain separate closed JSON
documents. The writer owner is `suspect-terraform:lifecycle-v1`. Invalid mapping
and target configuration fail before writes. Reused operation IDs retain each
lifecycle use site's exact mapping pointer, including ambiguous/missing IDs.

Main verified all three real CLI process tests and four established SDK CLI
option tests in
`target/sdk-main-final-integration-20260910-01/terraform-cli-integration-05.log`.
The process checks include generation after both original source files have
been deleted, physical source IDs, corrupt pinned-cache refusal, ownership
conflicts, and the unchanged twelve-language SDK registry. Main's all-target,
all-feature CLI Clippy check is `cli-clippy-terraform-01.log` in that directory.

The complete desired set contains:

- `go/**`: the **unchanged canonical Go SDK emission** for the mapping's selected
  operation closure, including its codecs, documentation and source metadata.
- `terraform/go.mod`, `go.sum`, `main.go`, and `provider/*.go`: an independently
  installable Plugin Framework provider module. All resource/data-source calls
  use the allocated SDK method, input, field and exact-status wrapper names.
- `terraform/lifecycle-mapping.json` and `source-bindings.json`: the closed
  mapping, package pins, original physical sources, actual native bindings and
  SHA-256 inventory of the generated SDK dependency.
- `terraform/docs/**` and `examples/{resources,data-sources}/<type>/main.tf`:
  schema documentation and independent runnable Terraform consumers.

The provider's module pins the generated SDK version **and its actual Go `h1`
module/checksum identity**. It contains no local `replace` directive. The
Framework dependency closure is retained in `terraform/dependencies.{mod,sum}`
in the generator source; it came from actual Go dependency resolution.
Regeneration recomputes SDK checksums from canonical emitted bytes using Go's
`sumdb/dirhash.Hash1` definition. A different published SDK artifact therefore
requires the corresponding version/configuration update; it is not silently
accepted under an existing checksum.

Package admission also checks Terraform's source-address component rules within
the target's lowercase-ASCII, dotted-hostname subset. Namespace/type parts do not
permit dots, underscores, edge dashes, or consecutive dashes; host labels have
their separate IDNA hyphen restrictions. Source type must equal `provider_name`.
Actual generated SDK Go package paths (including its native example) must not
collide with the provider's root or `provider` subpackage import paths. Separate,
sibling and non-colliding nested SDK module paths remain supported.

## Closed lifecycle-v1 profile

The format discriminator is **`suspect.terraform.lifecycle.v1`**. Serde rejects
unknown fields and enum variants. No retry, polling, import splitting, or
operation-name inference is enabled implicitly.

The first bounded profile supports:

- 1–64 explicitly named resource/data-source types, each with 1–256 flat
  Terraform string/bool attributes. Inputs and outputs bind **one property of a
  closed native Go record**, or an explicitly located SDK scalar parameter.
  Native aliases and string/bool literals keep their actual allocated Go types.
- One exact successful status/representation per lifecycle operation. Mutations
  use a required single concrete JSON request record, if there is a body. State
  responses are single concrete JSON records. Delete success is SDK no-content.
- Explicit `create`, `read`, `update`, and `delete` operation IDs. Every required
  native SDK input is mapped. Every configured API attribute has a create and
  update binding, or an explicit replacement rule. Every state-bearing response
  maps all non-write-only API attributes.
- Required, optional, computed and optional-computed modes. Optional-computed
  input bindings require an optional SDK field with null-to-omission mapping;
  unconfigured values are supplied by the SDK response. No API defaults are
  copied into Terraform. Unsupported numeric, collection, dynamic-value,
  multi-media, composite identity and nested paths produce located refusals.
- Anonymous APIs or one exact source-declared bearer scheme. Provider `endpoint`
  and, for bearer mappings, sensitive `token` arguments are explicit. They are
  passed to the SDK's `ClientOptions` and allocated credential constructor.
  Unknown provider configuration is a diagnostic, never an invented credential
  or server. `NewWithTransport` exposes the SDK's own `Doer` injection seam.

### State and lifecycle rules

| Concern | Explicit v1 behavior |
| --- | --- |
| Unknown | Planning preserves unknowns. An unresolved API input or trigger at apply fails before the SDK request. Terraform-computed output values come from the response. |
| Null | Each input chooses `reject`, `omit`, or `send_null`, validated against the actual native optional/nullable carrier. Response omission and JSON null both map to Terraform null. |
| Identity/import | A non-sensitive computed string attribute receives a required non-null response ID. The ID must be nonempty and stable on update/refresh. `opaque_string` import preserves the exact text; read and delete need only that identity parameter. |
| Replacement | `requires_replace` emits the Framework's typed `RequiresReplace` plan modifier. Stable identity uses `UseStateForUnknown`. |
| Refresh/drift | `authoritative_read` maps remote fields back to state. A planned configuration difference becomes Terraform drift repair. Successes that disagree with known plan values retain authoritative state and produce a diagnostic. |
| Missing | Only explicitly listed, declared and successfully decoded SDK error types remove resource state on read or make delete idempotent. Deferred-validation SDK streams cannot be mapped as missing in v1. Data-source missing is an error. Transport/codec failures and their wrapped causes are not reclassified as missing. |
| Sensitive | Framework `Sensitive` redacts normal CLI output; the value still exists in state. |
| Write-only | Framework `WriteOnly` plus `Sensitive`, required/optional only. Values are taken from configuration, passed through SDK typed inputs and set to null in returned state. A configured secret needs its explicit Terraform-only version trigger. |
| Rotation | Update `trigger_changed` sends the write-only input only when its configured state-only trigger changes. An unknown trigger or null/unknown secret on rotation fails without a request. Changing the secret alone creates no Terraform diff. |
| Partial failure | `mapped_partial_otherwise_preserve` binds explicit error-status response state. Failed creates with identity are saved and Terraform taints them. Partial updates save actual returned fields while retaining prior trigger values. Other errors preserve prior state (none on create); uncertain side effects require refresh/import. |
| Retry/polling | Both enums admit only `none`. Each lifecycle invocation has one SDK call; Terraform context, cancellation, checked decoding and resource cleanup stay on that call. Every returned SDK API-error response is closed through its actual allocated SDK interface, including partial/missing/unmapped errors. Unmapped streams are closed without provider-side parsing or iteration. |

The provider reads no OpenAPI schema at runtime and carries no HTTP client,
request serializer, codec, response parser, or API validation implementation.
Admission uses the canonical Go plan's retained typed descriptors, not a second
schema interpreter or a parser for emitted Go.

## Maintained fixture and acceptance

All maintained inputs are in
`crates/suspect-codegen/tests/fixtures/terraform-v1/`:

- `openapi.json` and `schemas.json`: split source identity, colliding native
  field/method names, exact status types and intentionally misleading operation
  IDs (`read-item` creates; `erase` updates; `observe` deletes).
- `mapping.json`: resource/data-source lifecycle, drift, replacement, opaque
  import, nullable/omitted input, sensitive fingerprint, write-only secret and
  trigger, partial create/update and missing behavior.
- `target.json`: provider `registry.terraform.io/suspect/fixture` version
  **0.1.0**, SDK `example.com/lifecycle-sdk` **v0.4.2**, Framework **1.15.1** /
  Plugin Go **0.27.0**, Go language **1.23.0** / toolchain **go1.23.12**, Terraform
  **1.15.8**. These are fixture build identities, not publication claims.
- `provider_test.go` and `negative.go`: independent native Framework/SDK
  consumers. Test overlays remain outside the production desired artifact set.

The native selector invokes `tools/sdk-terraform-acceptance.py` with a fresh
approved temporary root. It installs the exact generated SDK ZIP through an
isolated local Go module proxy, verifies module and binary dependency linkage,
and installs provider binaries through a Terraform filesystem mirror. The
mirror configuration has no remote installation fallback. The native toolchain
matrix is Go **1.23.12** / **1.27.1**, using the real Terraform **1.15.8** binary.

```sh
cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test terraform

# The evidence path must not already exist. Native roots/logs are retained.
SUSPECT_TERRAFORM_EVIDENCE=/absolute/new/evidence/path \
  cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test terraform \
  real_terraform_lifecycle_through_pinned_generated_sdk -- --ignored --nocapture
```

### Completed owner acceptance

Evidence is retained at `target/sdk-terraform-stretch-20260910-01/`:

| Gate | Result | Receipt |
| --- | --- | --- |
| Full real provider lifecycle, Go 1.23.12 / 1.27.1, Terraform 1.15.8 | **204 commands, 330 checks passed; 82 recorded API exchanges** | `native-02/report.json` |
| Native Framework types, unknown/null, checked SDK rejection, context/body cancellation, write-only triggers, partial state and validated missing errors | `go test -race`, `go vet`, native Go docs and negative consumer compilation passed on both tiers | `native-02/commands/` |
| Terraform runtime/schema/docs/HCL | `init`, `validate`, `plan`, `apply`, `refresh`, `import`, `destroy`; drift, replacement, data sources, unknown resolution, secret exclusion from saved plan/state, failed/partial operations, and actual SIGINT cancellation passed on both tiers | `native-02/go1.23.12/` and `go1.27.1/` |
| Supplemental optional-computed / anonymous data-only / SDK upgrade controls | **12 commands, 10 checks passed** on both tiers; new SDK v0.4.3 changes source security and the allocated read method, and the provider binds that actual interface | `variants-02/report.json` |
| Unknown secret and unknown trigger | Focused native update controls reject both without an API call; prior state remains known | `variants-02/commands/` |
| Current-source host admission and ownership | **9 passed**, two explicit native selectors left ignored in this host-only run | `host-04.log` |
| Scoped Clippy and Rustdoc | Passed; Clippy allows only the existing unrelated strict-adapter `dead_code` warnings; Rustdoc denies warnings | `clippy-02.log`, `rustdoc-01.log` |
| Positive output after mapping-location/anonymous-admission fixes | All **48** emitted SDK/provider files byte-identical to the completed full native matrix (34 SDK + 14 provider assets) | `post-diagnostic-artifact-parity.json` |

The full native matrix remains completed; the supplemental selectors exercise
new profile controls rather than replaying it. Reproduce supplemental controls:

```sh
SUSPECT_TERRAFORM_EVIDENCE=/absolute/new/variant/evidence/path \
  cargo test --locked --offline -p suspect-codegen --no-default-features \
  --features http-protocol --test terraform \
  native_optional_computed_anonymous_and_sdk_upgrade_controls -- --ignored --nocapture
```

`tools/sdk-terraform-variants.py` uses fresh unpublished SDK module-cache
namespaces and reuses only the pinned public dependency/toolchain cache. This
keeps fresh physical source identities separate from previously accepted SDK
checksums. A current-source base emission in that receipt proves byte parity
with the completed Terraform matrix; ownership metadata differs only because
the comparison deliberately uses a distinct logical writer owner.

Failed attempts remain intact: `host-01.log` is the initial closure-typing
failure; `native-01/` records actual dependency provisioning and the strict
complete-lock failure before the checked dependency assets were emitted;
`variants-launch-01.log` records the initial refusal of canonical `security: []`,
subsequently fixed and checked in both host and native anonymous controls.
Native binaries, SDK ZIPs, module sums, tool digests, raw command logs, saved
Terraform plans/state, wire receipts and isolated roots remain preserved.

Main's independent Terraform Standards/Spec reviews and full-workspace
integration are tracked separately under
`target/sdk-main-final-integration-20260910-01/review-terraform-01/`. Earlier
common-code reviews do not constitute a review of this new artifact target.

### Independent review followups

The initial owner handoff and its 779-file checksum inventory remain immutable.
The new reviews found native boundaries outside the first fixture's coverage:

- **Provider source address:** dotted namespaces and malformed native host/type
  components previously passed package admission. The located
  `terraform-package` refusal at `/config/provider_address` is now checked by
  24 independent actual Terraform parser controls, including valid custom
  hostname/namespace/type combinations. Evidence:
  `target/sdk-terraform-address-admission-20260910-01/`. That address-only
  candidate preserves all 48 ordinary emitted files byte-for-byte.
- **SDK error ownership/deferred validation:** v1 now refuses lazy SDK stream
  representations in `missing` mappings with
  `terraform-missing-response-deferred` at the exact mapping element and media
  source. Returned API errors are scoped and closed separately from SDK
  transport/codec failures; nested causes cannot masquerade as missing responses.
  Fresh host/native red/green evidence is under
  `target/sdk-terraform-error-ownership-20260910-01/`.
- **Go import collision:** an SDK module named exactly like the emitted provider
  subpackage previously passed admission and failed native import lookup. The
  `terraform-package` refusal at `/config/sdk/module_path` now protects actual
  generated package paths. Focused checked-module lookup controls and legitimate
  separate/sibling/nested layouts are under
  `target/sdk-terraform-module-collision-20260910-01/`.

The error-lifetime repair changes generated lifecycle error paths. Its focused
native checks supplement the retained full Terraform matrix; package-admission
repairs alone do not require that matrix to be replayed.

The completed followup receipts are the address `native-01/report.json`
(24 actual Terraform address controls), error-ownership
`native-green-02/report.json` (both Go tiers, exact SDK ZIP/h1/no replacement,
including unmapped 409 **and 503**, cancellation, identity early-return,
transport-cause distinction and buffered partial/missing validation), and
module-collision `native-01/report.json` (28 commands/20 checks on both tiers).
Combined host/Clippy/format and current-source output deltas are in
`target/sdk-terraform-review-fixes-20260910-01/`: 46 of 48 ordinary files are
unchanged, including every SDK, package, HCL and documentation byte. Only the two
generated lifecycle Go files change for SDK API-error ownership handling.
Independent review severities remain separate: stream handling is **Spec P1 /
Standards P2**, address admission is **Standards P2**, and the package collision
is **Spec P2**. Main owns the fresh independent rechecks of these corrections.

## Primary references

Consulted 2026-09-10; package versions are deliberately pinned rather than
silently selecting the documentation site's latest version:

- [Framework write-only arguments](https://developer.hashicorp.com/terraform/plugin/framework/v1.15.x/resources/write-only-arguments): configuration-only values, Terraform >=1.11, explicit rotation triggers.
- [Framework create](https://developer.hashicorp.com/terraform/plugin/framework/v1.15.x/resources/create), [update](https://developer.hashicorp.com/terraform/plugin/framework/v1.15.x/resources/update), [plan modification](https://developer.hashicorp.com/terraform/plugin/framework/v1.15.x/resources/plan-modification), and [import](https://developer.hashicorp.com/terraform/plugin/framework/v1.15.x/resources/import): plan/state consistency, taint, replacement, authoritative partial state and import read requirements.
- [Framework v1.15.1 source](https://github.com/hashicorp/terraform-plugin-framework/tree/v1.15.1) and its [exact module manifest](https://proxy.golang.org/github.com/hashicorp/terraform-plugin-framework/@v/v1.15.1.mod).
- [Go module dirhash](https://github.com/golang/mod/blob/v0.24.0/sumdb/dirhash/hash.go): native checksum algorithm, independently checked by Go dependency installation.

Retained primary source bytes and SHA-256 identities are under the evidence
root's `research/` directory.
