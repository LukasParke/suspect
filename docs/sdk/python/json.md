# Exact JSON for generated Python SDKs

Status: experimental representation layer. Eight native runtime gates pass,
including a 10,000-case rational oracle and all four normalized tracked inputs.
The runtime and bounded model consumers pass Python 3.11 and 3.14. This does not
certify Python model codecs or a complete SDK.

The generated Python JSON runtime (`python/json_runtime.py`) is a
dependency-free, standard-library-only module that parses and writes one
exact JSON value. It preserves every valid JSON number token exactly,
including negative zero, exponent spelling, fractions, and integers beyond
native ranges. Numbers never route through `float`, `Decimal`, or
`str(float)`. It is representation-only: it does not validate an OpenAPI
schema, select a union branch, apply defaults, coerce values, or convert
models. Source-aware model codecs remain a required subsequent layer.

## Public API

```python
class JsonNumber:                       # immutable, validated exact token
    def __init__(self, token: str): ...
    @classmethod
    def parse(cls, token: str) -> "JsonNumber": ...
    @property
    def token(self) -> str: ...         # exact source spelling
    def is_integer(self) -> bool: ...   # symbolic, no exponent-sized allocation
    def to_int(self, max_digits: int = 4096) -> int: ...

from typing import Union
JsonValue = Union[None, bool, str, JsonNumber, list, dict]

def parse_json(text_or_bytes, limits: JsonLimits | None = None): ...
def stringify_json(value, limits: JsonLimits | None = None) -> str: ...
```

`JsonNumber(token)` and `JsonNumber.parse(token)` are the same validating
constructor; non-`str` input raises `TypeError`, malformed spellings raise
`JsonError` with kind `syntax`. Equality and hashing are spelling equality:
`1.0` and `1` are distinct representations, which is required to preserve
wire identity. `is_integer` answers mathematically (comparing the mantissa's
trailing zero count against the effective exponent) and works on
`1e999999999` without allocating an exponent-sized value. `to_int` is an
exact chunked decimal conversion — chunks of nine digits, so the global
`sys.set_int_max_str_digits` limit never changes behavior and no
`str(float)` path exists. It raises `not_integer` for non-integral values
and `resource_limit` when the exact result would exceed `max_digits`
(capped at `10**9`, so the oversized-exponent bound below always dominates).

Generated model modules import the runtime as `import json_runtime as
_json` and consume `_json.JsonNumber`.

## Limits and work accounting

`JsonLimits(max_input_bytes, max_output_bytes, max_depth, max_work)` defaults
to 8 MiB / 8 MiB / 128 / 32M work units, mirroring the Rust runtime. Input
size is checked before decoding or allocating, nesting depth before entering
a container, and output bytes before every append. Work is charged
incrementally while scanning, decoding, and writing. This counter is not a
complete CPU or memory accounting mechanism: UTF-8 admission, allocator
overhead, and dict hashing are bounded indirectly by the byte and depth
limits. As in Rust, the requested depth is hard-capped at 256 so recursive
descent can never approach the interpreter recursion limit; the module
neither reads nor changes `sys.setrecursionlimit` or
`sys.set_int_max_str_digits`. Configuring `max_depth > 256` fails with
`resource_limit` before any work starts.

## Errors

`JsonError` has a stable `kind` (`syntax`, `invalid_utf8`, `duplicate_key`,
`resource_limit`, `not_integer`, `unsupported_value`, `cycle`), an `offset`
(byte offset for bytes input, character offset for `str` input, `None` for
whole-operation failures), and a JSONPath-like `path` (for example
`$.a[2].b`) for structural failures such as duplicate keys. Messages are
fixed and bounded; errors never retain or echo the input document or
property values.

Rejected inputs: malformed JSON grammar, trailing content, a UTF-8 BOM,
NaN/Infinity spellings, malformed UTF-8 in bytes input, duplicate *decoded*
object keys (escape spelling is normalized first), lone surrogate escapes,
unpaired or invalid surrogate pairs, raw control characters in strings, and
raw lone surrogate characters in `str` input.

## Unicode scalar policy

Python `str` can contain lone UTF-16 surrogates; Rust strings cannot. This
runtime adopts the Rust scalar-only policy so both runtimes represent the
same values: valid escaped high/low surrogate pairs decode to one Unicode
scalar, while lone surrogate escapes, raw lone surrogates in input strings,
and surrogate output are explicit located errors — never silent
`\ufffd`-style replacement. This keeps the documented cross-language gap
explicit instead of hiding it in one runtime.

## Encoding contract

`stringify_json` accepts exactly `None`, `bool` (checked before `int`),
exact `int`, exact `str`, `JsonNumber`, `list`, and `dict` with exact `str`
keys. `float`, `Decimal`, `IntEnum`, `str`/`JsonNumber` subclasses, tuples,
sets, and custom types raise `unsupported_value` — there is no dispatch
through untrusted `__str__`, `__index__`, or hook methods. Shared
substructures encode repeatedly; true cycles raise `cycle` with the current
path. Large exact integers render through the same chunked decimal
conversion, bounded by the output budget before conversion starts. Object
keys keep insertion order (Python dicts are ordered); the Rust runtime
sorts keys, so cross-language consumers must normalize key order (the
tracked-corpus roundtrip test does exactly that and labels the
serde_json-value comparison as its normalization oracle). Escapes follow
the RFC 8259 table; non-ASCII scalars emit as raw UTF-8.

## Emission and ownership

The template lives at
`crates/suspect-codegen/src/python_json/runtime.py`;
`suspect_codegen::python_json::runtime_source()` is its single source of
truth and `emit()` produces `python/json_runtime.py`. The model planner
consumes `runtime_source()` so every generated
package embeds the same audited text. Native acceptance is opt-in:
`crates/suspect-codegen/tests/python_json.rs` runs real Python consumers
over independent grammar/value vectors, exact wide numbers, absent/null key
distinction, duplicate escaped keys, controls/surrogates/UTF-8, cycles,
resource budgets, and the tracked OpenRouter corpus roundtrip; missing
tools or inputs fail loudly and never skip silently.
