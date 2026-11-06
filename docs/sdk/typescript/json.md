# Generated TypeScript exact JSON runtime

Canonical model artifacts now include `json.ts`. The generator emits the same
`JsonNumber` class used by model declarations, a lossless parser, and a checked
encoder, with no runtime dependencies. This implements JSON representation and
grammar. It does not implement OpenAPI schema validation, model decoding, or an
HTTP client, and it does not clear model codec obligations.

`parseJson(text, limits)` preserves every number as a privately branded exact
token. `JsonNumber.parse(token)` checks JSON number grammar; `toBigInt` and
`toSafeInteger` perform checked mathematical conversion. Large exponents stay
symbolic unless a requested integral conversion fits the explicit digit budget.
`stringifyJson(value, options)` writes exact numeric tokens, bigint values, and
finite native numbers with safe integer checks. Ordinary `JSON.stringify` on
`JsonNumber` fails explicitly to prevent accidental representation loss.

The parser rejects duplicate decoded keys and preserves names such as
`__proto__` as own data properties. Arrays and objects use iterative traversal;
per-call limits cover text characters, visited values, container depth, and
numeric-token characters. Defaults are 16 Mi UTF-16 code units, 1,000,000 values,
128 open containers, and 4,096 numeric characters. These are configurable
resource limits, not schema constraints or a bound on every underlying JS
allocation. Character counts are not UTF-8 byte counts.

The encoder rejects cycles, array holes, accessors, non-enumerable/extra array
properties, symbol keys, undefined/functions, non-finite numbers, unsafe native
integers, and nonplain object prototypes. Repeated acyclic objects are allowed.
It neither calls application `toJSON` methods nor dispatches exact-number
serialization through instance methods. Private-token extraction also handles
JavaScript subclasses safely. Container admission checks precede descriptor
materialization; own-key enumeration and Proxy traps cannot be eliminated by a
generic JavaScript serializer. Encoding defaults to rejecting undefined. An
explicit `omitUndefinedProperties: true` option permits omission of enumerable
string-named object data properties whose value is undefined. Null is retained;
undefined array elements still fail. Omitted slots consume the node budget and
getters are never invoked. Model codecs select this policy and validate the
resulting wire object against requiredness and all other supported constraints.

## Verification

Six native tests exercise generated artifacts under strict TypeScript and
Node.js, including canonical model interoperability and all four tracked
OpenRouter documents. Each complete Contract document survives a native
parse/encode roundtrip with an exact comparison against its original owned JSON.
That is representation evidence; it does not suppress existing upstream schema
defects or establish schema validity.

Public cases include large integers/decimals/exponents, negative zero, escaped
strings/keys, duplicate keys, caller-defined numeric subclasses, getters,
output/node admission and iterative 5,000-level traversal. Independent review
found and repaired virtual numeric-method dispatch and descriptor expansion
before budget admission. It also replaced a quadratic trailing-zero regex with
a linear scan. Independent probes checked 5,004 exact numeric conversions
against Python Decimal and 30,000 mutated JSON syntax cases against native
JSON.parse; these are differential probes, not complete grammar certification.

```sh
OPENROUTER_WEB_ROOT=/path/to/openrouter-web \
  cargo test --locked -p suspect-codegen --test typescript_json \
  -- --include-ignored
```

Native tools and tracked inputs are mandatory when this opt-in gate is run.
Evidence: `target/typescript-json-audit-tests.log` and
`target/typescript-json-audit/`. Final independent repair checks also pass all
5,004 numeric and 30,000 syntax probes, with zero descriptor inspections in the
unaffordable-container cases. See `repair-results.json` in that audit directory.

## Initial local performance evidence

The initial JSON runtime baseline, before the explicit undefined-omission option
and with every export retained, was 6,770 bytes after Bun
browser minification and 2,591 bytes gzipped. On Node 26.8.1/Bun 1.4.0, the
independent 1,009,274-byte normalization of the tracked OpenRouter document had
median parse/encode times of 11.07/12.90 ms over 20 samples after warmup.
The repaired 4,096-character numeric conversion took 0.758 ms for 1,000 calls.

The source digest and runtime digest are recorded in
`target/typescript-json-runtime-baseline/report.json`, with a rerunnable local
benchmark. These are development measurements for JSON representation only;
they neither establish a minimum possible bundle nor certify complete SDK
performance. Runtime release thresholds and complete SDK measurements remain
subsequent work; the separately emitted schema validator now has its own bounded
conformance gates.
