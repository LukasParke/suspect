# Python runtime regression gates

`tests/python_runtime_regressions.rs` emits actual Python artifacts and invokes
these independent, expected-correct consumers. Runtime files and generated
metadata are consumed unchanged. `consumer-config.json` contains only consumer
inputs and expected source identities; resource policies come from Rust's public
`CodecConfig`, `JsonLimits`, and `HttpConfig` APIs.

| Review finding | Gate |
| --- | --- |
| R1 | Bounded string-input work for short strings and escape-heavy strings; raw timing samples are observational |
| R2 | Exact padded-exponent JSON helpers, emitted integer model codecs, separate original OpenRouter `ContainerFile` opt-in |
| R3 | Literal/JSON-to-object union fallback, plus fatal JSON resource exhaustion during a branch trial |
| R6 | Per-call conversion budget shared by large JSON copies, multiple extra fields, and JSON descendants |
| R7 | Literal equality exhaustion remains `CodecError(resource)` with original source and instance path |
| R8 | Installed sync/async HTTP clients preserve primary resource/read failures and caller cancellation when closing also fails; standalone close failures are typed/redacted |
| R12 | Exact-fitting integer output-byte limits, including a 5,000-digit integer |
| R13 | Installed sync/async HTTP clients reject invalid ports before transport, with valid endpoint controls |
| R16 | Actual parse/write errors have bounded, control-escaped formatting |

The R1 meter is a `str`-compatible input that records character access and the
distance searched by `find()`. It does not patch the parser or assert private
function calls. A generous linear allowance detects the original repeated
suffix scans deterministically. The separate timing samples have no wall-clock
or timing-ratio pass/fail threshold.

## Run

All native tests are explicitly ignored by ordinary workspace test runs. Run this
one target with the chosen interpreter; use both Python 3.11 and Python 3.14:

```sh
SUSPECT_PYTHON_BIN=/path/to/python3.11 \
  cargo test --locked -p suspect-codegen --test python_runtime_regressions -- \
  --include-ignored --skip original_openrouter
```

HTTP gates build with `SUSPECT_PYTHON_TOOLS` (default
`target/sdk-native-python-tools/bin/python`, which can remain Python 3.11), then
use `uv venv --offline --python "$SUSPECT_PYTHON_BIN"` and install the wheel with
`uv pip install --offline`. The existing build/hatchling tools and cached
httpx 0.28.1 dependencies must be available; missing tools fail explicitly.

The separate, read-only original-source gate is:

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
SUSPECT_PYTHON_BIN=/path/to/python3.11 \
  cargo test --locked -p suspect-codegen --test python_runtime_regressions \
  original_openrouter -- --include-ignored
```

Each test retains a new directory under
`target/sdk-python-runtime-regressions/candidates`. Override the parent with
`SUSPECT_PYTHON_REGRESSION_ARTIFACTS`. Native stdout/stderr, interpreter/package
provenance, wheel build/install logs, source artifacts, and raw scaling samples
remain available there. These directories are separate from the immutable
independent-review snapshots and reports.
