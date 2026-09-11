# Python SDK: public results and executable task guides

This describes Python onboarding and public exports. Generated HTTP packages start
with native request construction, explicit bearer credentials, and task guides.
The model names come from the shared source-role allocator; their original
source identities remain in metadata.

## Public API

For the configured import package, the root intentionally exports:

- `Client`, `AsyncClient`, `models`, `codecs`, and **`operations`**.
- `UNSET`, `Unset`, **`JsonNumber`**, and **`JsonValue`**.
- `ApiError`, `SdkError`, **`CodecError`**, **`JsonError`**, and
  **`ValidationError`**.

Both the package root and `operations.py` have explicit `__all__` lists and
type-checker-visible exports. Every concrete status class and every operation's
success/API-error alias is defined in `operations.py`. Examples use either
`from <package> import operations` or direct imports from
`<package>.operations`.

The former `_client` status/alias attributes remain imports of the **same
objects**. Methods still return/raise those exact classes. Their defining
`__module__` is now `<package>.operations`; this intentional introspection
change needs to be represented in native compatibility snapshots.

Operation success/API-error aliases express type unions. A concrete non-2xx
status class or the root `ApiError` base is the appropriate exception handler;
an arbitrary union alias is not an `except` target.

`JsonNumber` is the existing exact-number class, re-exported at the root.
Integer schemas construct Python `int`, including mathematical-integer JSON
spellings such as `1.20e2`. General `number` schemas retain `JsonNumber`, even
for integer-looking tokens. Unconstrained JSON literals use `int` for integer
tokens and `JsonNumber` for decimal/exponent/negative-zero tokens. The model and
codec descriptors determine these choices. `Decimal(value.token)` is an
explicit interoperability step; the guides demonstrate it without binary
floating-point conversion.

## Generated customer documentation

The package README and Sphinx entry point now cover:

1. Installation and a complete first request with explicit credentials.
2. Authentication using the exact source scheme name and source server.
3. Reusable synchronous clients and async calls with caller-owned cancellation.
4. Concrete API exceptions, input validation, and SDK failures.
5. Omission, explicit `UNSET`, source-admitted null, and supplied values.
6. Native models, exact numbers, and Decimal interoperability.
7. Injected httpx transports, timeout semantics, response budgets, and ownership.

The first-request choice prefers an operation with a constructible request
body and a declared success status. For the five tracked OpenRouter operations
this is `create_keys`, using `models.CreateKeysRequest(...)` and
`auth={"apiKey": token}`. The source's management-key guidance remains visible.
The application supplies credentials; runtime environment lookup is not added.

The ordinary examples are emitted as:

```text
examples/quickstart.py
examples/async_client.py
examples/errors.py
examples/presence.py
examples/numbers_example.py
examples/transport.py
```

README Python blocks are the exact files included by the Sphinx task pages.
`docs/operations.rst` presents methods, input types, requiredness and status
classes with links to the native reference. Detailed wire metadata remains in
`docs/source-bindings.json`. The Sphinx directive imports installed package
objects for signatures; it does not derive declarations from emitted source.

Source descriptions are inert literal text. Python strings, source names,
JSON pointers, and Unicode line separators are escaped at their output
boundaries. Fields literally named `properties` or `items` are handled as
fields from `PyDecl`, never as schema keywords during native construction.

## Native examples and provenance

`python_http/native_examples.rs` lowers accepted `ExamplePlan` values using the
retained `PyDecl`/`PyType` descriptors. It handles dataclasses, nested objects,
references, literals, lists/maps, typed extra-property setters, nullable values,
exact numbers, and native union alternatives. Required constant tag fields are
omitted from constructor arguments because their dataclasses use `init=False`.

One `OwnedSchema` is compiled per documentation render using the codec's
semantic configuration and model/branch roots. Union selection uses the same
first-valid-branch rule as the decoder, including branch assertions. An
incomplete evaluation cannot be treated as a failed branch and skipped.

The lowering has finite limits: 64 descriptor/value nesting levels, one million
charged rendering work units, 65,536 bytes per construction, and 4,096 digits
for expanded native integers. Unsupported representations, integer expansion
limits, and incomplete evaluations are recorded with source identities and
reasons. Missing required source examples remain a separate availability case.

The metadata distinguishes:

- `available`: required source-valid slot values exist.
- `nativeAvailable`: the bound inputs also have bounded native constructions.
- `entries[].native`: construction availability, selected branch sources,
  validation function name, or a located failure reason.
- `recipes.quickstart` and `recipes.presence`: the actual selected operation,
  source example, field/slot, and any derived presence variants.

Presence variants start with an accepted request example, change only the
selected field's presence/null state, and validate the resulting whole request.
Their origin is recorded as `derived-presence`, distinct from the unchanged
source example. For `update_keys`, the guide demonstrates omitted `limit`,
`limit=None`, and the source's supplied value.

The existing `examples/validated.py` keeps every source codec
decode/encode/decode round trip. It additionally checks:

```text
codec.encode(native_construction).encode("utf-8")
    == codec.encode(codec.decode(source_example)).encode("utf-8")
```

This compares native encoding with the source codec's native view, allowing
the codec's documented integer normalization while preserving exact general
number tokens. All available ordinary sync/async callsites use constructors
and literals. Optional fixture HTTP execution still requires an explicit URL
and token; the default validation script performs no HTTP calls.

## Native acceptance seam

`crates/suspect-codegen/tests/python_quickstart.rs` contains source-bound
availability checks and opt-in installed-package gates:

```sh
cargo test --locked -p suspect-codegen --test python_quickstart -- --include-ignored
```

The native gates use `target/sdk-native-python-tools/bin/python` with httpx
0.28.1, mypy 1.19.1, Sphinx 8.2.3, build 1.4.0, and hatchling 1.29.0. Each
candidate wheel is installed into separate Python 3.11 and 3.14 environments
with `uv`, using cached dependencies. Artifacts and command logs remain under
`target/sdk-python-dx-*`.

The checks cover the independent M2 contract, a constructor/quoting/union/extra
property fixture, and the actual five-operation OpenRouter selection. They
typecheck the generated package, all guide modules and extracted README blocks
with strict mypy; execute the exact native/decoded byte comparisons; build
Sphinx with warnings as errors against the installed wheel; and run real
generated clients with authenticated MockTransport requests. The fixtures also
check source-name collisions, public/legacy class identity, finite unavailable
cases, and SDK-owned versus caller-owned transport closing.

### Verified evidence

The complete `python_quickstart` suite passed: **4 tests, 0 failures**, including
the opt-in native gates. Each native candidate passed installed-wheel execution,
strict package/guide/README mypy checks, native-versus-decoded encoding checks,
authenticated MockTransport calls, and warning-free Sphinx builds. Interpreters
were **Python 3.11.15** and **Python 3.14.7**.

| Candidate | Operations | Native source examples | Exact README snippets | Retained evidence |
| --- | ---: | ---: | ---: | --- |
| M2 | 4 | 17 | 6 | `target/sdk-python-dx-m2-HzVN5P/evidence.json` |
| Constructor/quoting/union fixture | 1 | 6 | 6 | `target/sdk-python-dx-edge-Kafxsa/evidence.json` |
| Tracked OpenRouter selection | 5 | 39 | 6 | `target/sdk-python-dx-openrouter-HHs1yw/evidence.json` |

All four M2 calls and all five real-source calls have native constructions.
The wire probes observed 15, 11 and 19 requests respectively on each interpreter,
including the quickstart, sync/async operation examples, presence variants,
failure recipes and injected-transport call. Each directory retains command
stdout/stderr, the generated package, installed environments and built Sphinx
coverage. The generated packages were not manually repaired.

The final source-binding review also passed the existing installed Python docs
gate, the public status/parameter collision gate, and the HTTP cleanup/primary
failure/cancellation regression. Their logs are
`target/sdk-python-dx-existing-{docs,names,lifetime}.log`. The reported Clippy
single-character `push_str` finding was corrected; scoped Rust formatting checks
pass.

## Integration required in the main session

- Add `python_quickstart` to the Python native acceptance stages. Its tests
  already exercise both 3.11 and 3.14; one invocation covers the pair.
- Add `python_http/docs.rs` and `python_http/native_examples.rs` to the Python
  provenance asset closure in `compatibility/provenance.rs`.
- Capture the intentional root exports, `<import>.operations` result module,
  and concrete class module identities in Python native compatibility
  snapshots. The HTTP manifest now supplies `publicExports`,
  `operationsModule`, per-operation `resultModule`, and per-response `module`.
- Treat source-role model renames separately from these additive exports. The
  shared naming work preserves source IDs but changes public model spellings.
- Regenerate and re-run the integrated acceptance package before updating its
  frozen report. Existing response lifetime, capture, credential, redirect and
  cancellation checks remain part of that acceptance.

The improvement addresses concrete Python onboarding friction from the
comparison with Speakeasy. Resource namespaces, flattened request conveniences,
protocol breadth and production distribution are separate dimensions of SDK
quality; these documentation/export gates do not establish overall product
parity.
