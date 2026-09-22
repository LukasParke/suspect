# Python native resource/dynamic adoption

Python executes the exact `suspect.validation.experimental.v3` /
`oas31-jsonschema202012-resources-dynamic` pair from the
[owned resource contract](SDK-SCHEMA-RESOURCES.md).

Completed receipt: `target/sdk-python-resources-completion-01/COMPLETION.md` and
`completion.json`, including source/input/evidence hashes and the final production
asset inventory. Scoped formatting and Clippy pass in
`sdk-python-resources-format-final.log` and
`sdk-python-resources-clippy-strict-final.log` (under `target/`).

## Admission and native interface

Existing `python_models::plan_models`, `python_codecs::plan_codecs`,
`python_http::plan_http` and `python_validation::emit` entry points remain in use.
The indexed effective closure determines whether resource semantics are needed.
Only those closures call `OwnedCompiler::compile_v3`; ordinary base/scoped
closures retain `compile_v2` and their original v1/v2 program bytes. HTTP examples
use Main's `plan_protocol_examples_v3` for the same selected profile. Native
example lowering checks values through `compile_v3` before rendering expressions.

Resource-aware native models are complete-root exact JSON carriers. Local
constraints may provide `int`, `JsonNumber`, `str`, `bool`, `None`, dictionary or
list annotations. Static annotations never guess a dynamic fallback type.
Conversion owns mutable values and runs the root source codec on every decode
and encode. It does not revalidate a nested dynamic child outside the parent's
binding context. Model-only APIs keep explicit codec obligations.

`CodecPlan::validation_program()` exposes the actual checked typed program.
HTTP manifests and Python compatibility descriptors record its version/profile.
There is no new canonical backend option or alternate public planning API.
The additive native `CredentialRequest.effective_server_url: str | None` field
supplies the actual resolved HTTP server to credential hooks. OAuth/OIDC endpoint
URLs remain literal; the source document and logical resource URI do not replace
their effective-server base. Its default preserves existing manual constructors.
The public adapter inventory includes `SchemaResources`,
`DynamicSchemaReferences` and the separately witnessed `DocumentRelativeServers`.

## Runtime and guards

- Nodes enter indexed resources, including nested entry points whose resource
  root is never evaluated. Only actually entered resources participate.
- Outermost matching bindings win. The fallback resource is not entered before
  lookup. Static refs and pointer/empty/static-anchor fallbacks remain static.
- Returns and trials restore scope. Cycle keys include exact ordered resource
  tuples; the same node/value under a genuinely different context may proceed.
- Dynamic targets start fresh annotation scopes and propagate successful sets.
  V2 annotation and branch rules remain unchanged.
- New resource entry and each scanned resource/binding consume the published
  shared visit budget. Numeric/equality/work/depth failures cannot be inverted.
- Guards check complete envelopes, resource URI/alias identity, physical
  containment, declaration/anchor locations, aligned scopes and finite targets.
  Sessions snapshot caller metadata. Validation performs no source acquisition.

V1 `runtime.py`/`number.py` and v2 `runtime_v2.py`/`guard.py` remain byte-identical
to the frozen v2 receipt. V3 adds `runtime_v3.py` and `resource_guard.py`; emitted
packages reuse the frozen evaluators under private companion module names.
The existing conversion runtime also remains unchanged.

## Native proof

Both **Python 3.11.15 and 3.14.7** pass:

1. **44 unmodified official dynamicRef cases**, compiled from maintained source
   fixtures and original remote documents through a closed provider and the real
   `compile_v3` API; **17 independent scope/context/work/depth controls** and
   **27 malformed-program controls**. Native runtime strict mypy passes.
   Evidence: `target/sdk-python-resources-runtime-vF9TSe/` and
   `target/sdk-python-resources-runtime-03.log`.
2. **Eight installed SDK operations, 29 model aliases and 14 source examples**
   exercising strict recursive trees, dynamic override
   of a differently typed fallback, nested resource entry, pointer/static/empty
   fallbacks, exact numbers, dynamic contains/unevaluated arrays, changed-context
   recursion and actual binary bytes. Native guides send the generated source
   constructions through sync and async clients.
3. Positive/negative installed consumer typing, full-package strict mypy,
   native example encode-byte equivalence and warning-as-error Sphinx using each
   interpreter: 22 strictly typed package files and 342 documented symbols.
   Each tier executes 32 successful requests including generated guides, 12
   invalid-input/mutation controls and six invalid HTTP responses.
   Final evidence: `target/sdk-python-resources-sdk-tWlbaY/` and
   `target/sdk-python-resources-sdk-final.log`.
4. Typed compatibility capture and ordinary v1/v2 program-byte controls:
   `target/sdk-python-resources-sdk-KvxFr5/`.
5. Main's unchanged null-only model regression on both native interpreters:
   `target/sdk-python-resources-null-{311,314}.log`. Affected ordinary model/codec
   planning checks pass in `target/sdk-python-resources-model-regressions.log`.
6. The final physical-server/credential-context witness passes both native tiers,
   package/consumer mypy and Sphinx: 11 requests, six malformed/base controls and
   seven OAuth/OIDC context checks per tier. Literal endpoint URLs and their
   actual effective-server base are independently asserted, including overrides.
   Evidence: `target/sdk-python-document-servers-gvKk4x/` and
   `target/sdk-python-document-servers-03.log`.

Maintained exact selectors, using
`cargo test --locked -p suspect-codegen --no-default-features --features http-protocol`:

```text
--test python_resources python_v3_executes_official_sources_dynamic_scopes_guards_and_exact_budgets -- --exact --ignored
--test python_schema_v3 python_resource_admission_keeps_ordinary_programs_and_typed_capture -- --exact
--test python_schema_v3 installed_python_v3_dynamic_operations_native_examples_types_and_sphinx -- --exact --ignored
--test python_document_servers installed_python_physical_document_servers_preserve_redirects_overrides_and_encoded_paths -- --exact --ignored
```

The official source fixtures and supplied documents are maintained inputs. The
earlier frozen executable witness under `target/` is not a test dependency.
The prior v1 wire/base/typing and v2 matrices retain their own reports and scope;
they were reused for this additive adoption.

## Production inventory

New files across v2/resource/physical-base adoption:

```text
python_validation/runtime_v2.py
python_validation/guard.py
python_validation/runtime_v3.py
python_validation/resource_guard.py
python_http/urls.py
```

Existing production edits are restricted to `python_validation.rs`,
`python_models.rs`, `python_codecs.rs`, `python_http.rs`, Python HTTP
`emit.rs`, `docs.rs`, `native_examples.rs`, `runtime.py`, `auth.py`, `types.py`, and only `fn python` in
`compatibility/native.rs`. No other capture or native-model helper was changed.
`python_http::source_assets()` supplies all added assets and shared protocol
resource/example inputs to Main's canonical framed provenance hash.

Custom vocabularies/dialects, legacy recursive-reference forms, unsupported
patterns, assertion-mode formats and HTTP directional projections retain their
existing source-linked boundaries. This is the bounded native v3 profile.
