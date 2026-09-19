# Canonical Go exact JSON runtime

`go_json::{runtime_source, emit}` provides the dependency-free exact JSON layer
used by the native Go models and codecs. `Parse` and
`Encode` operate on nil, bool, strings, opaque `Number`/`Integer`, slices and maps;
other values fail explicitly. Numeric tokens retain spelling and mathematical
integrality without floating-point conversion or exponent-sized allocation.
Zero-value number wrappers are invalid, not fabricated numeric zeroes.

The runtime guards the standard JSON tokenizer: duplicate decoded names,
malformed UTF-8, lone surrogate escapes and trailing content fail. Surrogate
pairs, including adjacent strings after a pair, remain exact. The Unicode
scalar-only boundary is explicit. Native number helpers handle leading exponent
zeroes correctly; a 10,000-case independent rational oracle checks integrality.

Limits bound input/output bytes, numeric token length, visited values and depth
(at most 256). Encoders check exact escaped string sizes and structural bytes;
configured output ceilings cannot be exceeded by null, bool or empty containers.
Cycles fail; finite slice views that share backing storage remain valid values.
Limits are not complete CPU/heap accounting.

Four opt-in runtime suites pass, including all four tracked documents after
Contract normalization. Native validation passes on Go 1.23.12 and the installed
Go 1.27 toolchain for this bounded representation profile.
These JSON helpers do not validate OpenAPI schemas or serialize native models.

```sh
OPENROUTER_WEB_ROOT=/Users/luke/github/openrouter-web \
  cargo test --locked -p suspect-codegen --test go_json -- --include-ignored
```
