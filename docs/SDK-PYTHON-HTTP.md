# Python HTTP SDK — current M3 slice

`suspect_codegen::python_http` consumes the shared strict HTTP admission and
the native Python codec plan. Selection is by canonical operation source.
Package configuration supplies identity; it does not supply API semantics.

## Generate

```sh
suspect codegen crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml \
  --profile python-http --package-name example-sdk --package-version 0.1.0 \
  --out generated-python
```

The package is written under `generated-python/python/`. Add `--check --format json`
for read-only drift inspection. [Session configuration](SDK-INCREMENTAL-GENERATION.md)
supports multiple profiles and an explicit Python `import_name`.

## Planning and packaging

```rust
python_http::plan_http(Arc<Contract>, selected: &[SourceId], HttpConfig)
    -> Result<HttpPlan, Vec<HttpDiagnostic>>
python_http::emit_http(&HttpPlan, &PackageConfig { name, version, import_name })
    -> Result<Vec<OutFile>, PackageError>
```

The immutable plan exposes its contract, operations, codec/model plan,
schema-to-native symbol map and shared `ExamplePlan`. Operations retain
allocated method names, keyword parameter names, success/error aliases and
concrete status classes. Documentation and examples reuse those names.
Codec lowering retains typed model descriptors; no documentation step parses
emitted Python to rediscover an API or maintains a second signature table.

Distribution name, exact SemVer input (also required to satisfy Python PEP 440)
and import identifier are checked before
emission, including explicit reserved import names. The package uses Python
3.11+, hatchling 1.29.0 and httpx 0.28.1. Output paths are rooted at `python/`:

- `pyproject.toml`, `README.md`.
- `src/<import>/__init__.py`, `py.typed`, `_client.py`, `_runtime.py`.
- `src/<import>/models.py`, `model_codecs.py` and exact JSON/validation/codec
  runtime artifacts from the codec plan.
- `src/<import>/http-manifest.json` and `examples.json`.
- `docs/{index,api,operations,examples}.rst`, `docs/conf.py`, pinned
  `docs/requirements.txt` and `docs/source-bindings.json`.
- `examples/validated.py`.

The documentation child returns full root-relative paths to the package
assembler, including the configured import name in every source binding.

## Actual native surface

Import `Client`, `AsyncClient`, `models`, `codecs`, `UNSET`, `Unset`, `SdkError`
and `ApiError` from the configured package. Concrete status classes and
operation success/error aliases live in `<import>._client`; model codec
instances are available through `codecs.<Model>Codec`.

Clients take explicit `auth={source_scheme_name: token}`, an optional
`server_url` and an optional httpx `BaseTransport`/`AsyncBaseTransport`.
Operations use required keyword-only arguments and optional `UNSET` arguments.
Declared null is `None`; absence does not apply a source default. Every input
uses its source codec, including revalidation of mutable model values.

Successful declared statuses return immutable status classes with validated
`data`, a header tuple and exact `Literal` status. Declared non-2xx responses
raise concrete `ApiError` subclasses with validated data. Unexpected statuses,
media, malformed responses, transport failures and response limits become
`SdkError`; input codec errors can propagate directly. Ordinary HTTP error
formatting hides captures and causes; explicit attributes retain diagnostics.

The default raw httpx transports ignore environment proxies and add no retries,
redirect handling, cookie persistence or decompression. Custom transports own
their policy. SDK-created transports close on client/context-manager exit;
injected transports remain caller-owned. Responses always close. Async calls
and body reads stay on the caller task so cancellation propagates normally.

Request URL/body and response capture have finite budgets. Response limits are
independent of Content-Length and may be lowered by callers. Path values and
source-defined form scalar/array queries use RFC 3986 escaping and the source
explode setting. Source server prefixes are retained.

## Browsable source-bound documentation

From the generated package, with the wheel installed and Sphinx 8.2.3 available:

```sh
python -m sphinx -W --keep-going -b html docs docs/_build/html
```

`conf.py` provides a Sphinx directive that imports real package objects and
uses `inspect.signature` and native annotations. This covers constructor,
operation, model-field and codec signatures, including sync/async differences,
keyword-only arguments, aliases and the `UNSET` default. Docstrings are literal
nodes; source descriptions and schema JSON are literal blocks. Source text
cannot introduce RST directives, roles, includes or active HTML.

Public symbols have crosslinked pages/anchors and original document/pointer
bindings. The inventory includes models, fields, extra-property APIs, codecs,
HTTP operations, success/error unions, concrete statuses and runtime helpers.
Operation metadata retains parameter/media/schema identities and security-use
and definition sources. Schema constraints and annotations remain inspectable
as original JSON. `coverage.json` in the built HTML directory records observed
native signatures and documentation coverage. Missing planned documentation
or broken crosslinks fail the strict build; native source dependencies also
invalidate incremental Sphinx builds.

## Executable samples and gates

`examples/validated.py` decodes, encodes and decodes every available shared
example through the installed native codecs. It emits statically typed sync
and async calls from the same validated values, binding by the canonical wire
slot rather than a parameter name. Missing required examples omit that
callsite with an explicit reason; optional unavailable values stay absent.
Declared/synthesized origins and located invalid-example findings are retained.

```sh
python examples/validated.py
python -m mypy --strict examples/validated.py
python examples/validated.py --server-url http://127.0.0.1:8080/api/v1 --token fixture-token
python examples/validated.py --server-url http://127.0.0.1:8080/api/v1 --token fixture-token --async-client
```

The default invocation only exercises codecs. HTTP execution requires an
explicit fixture URL/token and expects declared successful responses.
Callsite availability means that required schema-valid examples exist;
native representation, transport and API failures still propagate normally.

`crates/suspect-codegen/tests/m3_native_docs.rs` adds pure source/coverage/slot
checks and opt-in native gates: private wheel installation, strict mypy of the
generated sample, native-signature inventory checks, hostile-prose Sphinx
builds, unchanged incremental rebuilds, negative coverage/type probes and
sync/async sample execution against an independent create/update/list/get
recording server. Run the Python native gate with:

```sh
SUSPECT_PYTHON_BIN=python3.11 cargo test -p suspect-codegen --test m3_native_docs installed_python_docs_and_generated_samples_are_native_executable_artifacts -- --ignored
```

`SUSPECT_PYTHON_TOOLS` selects the build/mypy/Sphinx interpreter; the default is
`target/sdk-native-python-tools/bin/python`. The native gate requires `uv` and
cached wheel dependencies. Missing tools fail an explicitly invoked native
gate. Existing installed HTTP/runtime gates remain in `tests/python_http.rs`.

The same test file's `tracked_five_operation_docs_and_example_artifacts_build_natively`
gate loads the actual five tracked OpenRouter operations, checks shared example
provenance across Python/Go, builds both documentation inventories, typechecks
the Python sample and executes both native codec samples. Select that source
with `OPENROUTER_WEB_ROOT` or `SUSPECT_OPENROUTER_YAML` and invoke the gate with
`--ignored`. Its HTTP wire acceptance remains in `tests/wave_a_openrouter.rs`.

This documents the admitted M3 slice. Directional annotations and other
unproved HTTP/schema profiles remain source-linked planner rejections. Sample
availability and native gate results are separate from a complete SDK release
claim.
