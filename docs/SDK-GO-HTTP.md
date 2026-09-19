# Go HTTP SDK — current M3 slice

`suspect_codegen::go_http` plans source-selected Go packages using the standard
library HTTP transport and shared strict `http_contract` admission. The native
plan owns operation, constructor, field, setter and status-type names. Models
and codecs retain native typed descriptors; documentation and examples reuse
those descriptors and the shared `ExamplePlan`.

## Generate

```sh
suspect codegen crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile go-http --package-name example.com/example-sdk --package-version 0.1.0 \
  --out generated-go
```

The CLI package name supplies the Go module path; the native package is `sdk`
under `generated-go/go/`. Add `--check --format json` for read-only drift
inspection, or use [session configuration](SDK-INCREMENTAL-GENERATION.md) for
multi-target generation and watch.

## Planning API

```rust
plan_http(Arc<Contract>, selected: &[SourceId], HttpConfig::default())
    -> Result<HttpPlan, Vec<HttpDiagnostic>>
emit_http(&HttpPlan, &PackageConfig) -> Result<Vec<OutFile>, Vec<String>>
```

- Selection is by canonical source identity.
- `HttpConfig` carries the shared `go_codecs::CodecConfig` and positive byte
  ceilings for responses and assembled requests/bodies.
- Directional `readOnly`/`writeOnly` annotations anywhere in the neutral codec
  closure are explicitly unsupported initially and produce
  `http-directional-codec-unsupported` source-linked diagnostics.
- The codec closure is planned by `go_codecs::plan_codecs`. Documentation
  inventories are derived from its retained model descriptors and fields.
- `PlannedOperation` exposes allocated input constructors, parameters and their
  optional setters, request-body names and concrete per-status response types.
  Documentation does not maintain parallel method signatures.

## Native surface

- A `Client` with reusable configuration and a `Doer` transport seam
  (`*http.Client` satisfies it).
- Every method takes `ctx context.Context` as its first argument; contexts
  bound dial, headers and the full body read. Requests are never run in the
  background or detached.
- Native input structs per operation with required-input constructors and
  allocated optional setters. Optional operation inputs use `Optional`; model
  fields use the planned `Optional`, `Nullable` and `Presence` distinctions.
- Success is typed per exact declared status; errors are typed per declared
  error status (`<Op>ApiError`) plus transport-level failures.
- The SDK installs no retry policy. Standard net/http connection recovery
  remains transport policy; generated requests clear `GetBody`, so request
  bodies have no replay factory.
- The default transport suppresses ambient proxy environment variables and
  redirect following, and leaves decompression off: codecs see exact raw bytes.
- Responses and diagnostic captures are bounded independently of
  Content-Length. Ordinary HTTP error formatting omits captures and causes;
  `SDKError` fields and `Unwrap` deliberately expose diagnostics. Concrete API
  errors embed `APIResponse[Model]` with status, headers and validated data.
- Inputs are validated (declared constraints and schema mutations) before the
  request is assembled; no SDK defaults or auth are inferred from names.

## Packaging

`PackageConfig { module_path, package_name, version }` is independent of API
semantics. This profile currently admits package name `sdk`, a checked module
path and an exact SemVer. `go.mod` is assembled structurally with a Go 1.23
floor. Every emitted path is rooted at `go/`:

- Native model/codec/JSON/validation files, `http_runtime.go`, `operations.go`,
  `go.mod` and `README.md`.
- `http-manifest.json`, `examples.json` and model/codec source metadata.
- `doc.go`, `docs/{index,api,operations,examples,native}.rst`, `docs/conf.py`,
  pinned `docs/requirements.txt` and `docs/source-bindings.json`.
- `examples/validated/main.go`, importing the configured module path.

The documentation child returns generation-root-relative paths for direct
package assembly. It also exposes `operation_comment(plan, op)`, allowing the
parent emitter to attach the same source description, input/response links and
canonical location directly to native client methods.

## Browsable documentation and source coverage

```sh
go doc -all .
python -m sphinx -W --keep-going -b html docs docs/_build/html
```

The Sphinx 8.2.3 configuration calls the selected native `go doc` tool at
documentation-build time. Native declarations are its signature authority;
the Rust generator never parses emitted Go or copies a signature table.
Source descriptions, schemas and native tool output render as literal blocks,
so source prose cannot become RST directives or active HTML.

Crosslinked anchors cover every planned model, field, constructor, extra-field
API, literal, union alternative, typed codec entry, HTTP operation, input,
optional setter and status wrapper. Runtime declarations and helpers have
native documentation, and the complete native package reference exposes the
tool's full public output. `Codecs.<Model>` entries link to the real anonymous
registry declaration and their source-bound model/codec type.

`docs/source-bindings.json` carries the native names, artifact paths,
document/pointer identities, crosslinks, parameter/body/media bindings and
security-use/definition sources. Built `coverage.json` records documented
symbols and successful native queries. Strict Sphinx builds reject missing
symbols and broken links, including after incremental source changes.

## Executable samples and native gates

The packaged sample revalidates every available declared/synthesized value
through its source codec. It compiles typed operation calls using the plan's
allocated constructors and setters. Binding uses the canonical parameter or
media container, preserving distinct slots with equal wire names. Missing
required examples omit that callsite with a recorded reason; optional missing
values remain absent. Example diagnostics and provenance stay inspectable.

```sh
go test ./...
go run ./examples/validated
go run ./examples/validated -server-url http://127.0.0.1:8080/api/v1 -token fixture-token
```

The default sample only executes codecs. Explicit fixture flags enable HTTP
calls under a caller-owned 30-second context deadline; declared successful
responses are expected and errors propagate normally. Callsite availability
means that required schema-valid input examples exist, rather than a promise
that arbitrary transport or service calls succeed.

`crates/suspect-codegen/tests/m3_native_docs.rs` contains pure source/coverage/
example-binding checks and an ignored native gate. The native gate compiles
the actual package and sample, builds/link-checks Sphinx against `go doc`,
checks hostile prose and every planned HTML anchor, checks incremental builds,
executes the generated sample against an independent create/update/list/get
recording server, and rejects mistyped consumer inputs and missing docs.

```sh
SUSPECT_GO_TOOLCHAIN=go1.23.12 cargo test -p suspect-codegen --test m3_native_docs go_docs_and_generated_samples_are_native_executable_artifacts -- --ignored
SUSPECT_GO_TOOLCHAIN=go1.27.1 cargo test -p suspect-codegen --test m3_native_docs go_docs_and_generated_samples_are_native_executable_artifacts -- --ignored
```

`SUSPECT_GO_BIN` selects the Go executable; `SUSPECT_PYTHON_TOOLS` selects the
Sphinx interpreter (default `target/sdk-native-python-tools/bin/python`).
Missing tools fail explicitly invoked native gates. Existing independent
consumer/cancellation tests remain in `tests/go_http.rs`.

`tracked_five_operation_docs_and_example_artifacts_build_natively` in the same
test file additionally builds Python/Go documentation and compiles/executes
their codec samples for the five actual tracked OpenRouter operations. It
checks identical shared-example provenance. Select the corpus through
`OPENROUTER_WEB_ROOT` or `SUSPECT_OPENROUTER_YAML` and use `--ignored`; the
independent five-operation HTTP consumers remain in `tests/wave_a_openrouter.rs`.

This is documentation and executable-example support for the admitted M3
HTTP slice. Broader schema/HTTP features and a complete release remain subject
to their own source-fidelity and installed-native gates.
