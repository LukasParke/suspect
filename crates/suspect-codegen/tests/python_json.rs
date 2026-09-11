//! Native consumers of the exact JSON runtime emitted for generated Python
//! packages. Vectors are independent expectations, not emitter mirrors.

use std::process::Command;

fn run_python(
    files: &[suspect_codegen::OutFile],
    consumer: &str,
    input: Option<&str>,
) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(files, directory.path()).unwrap();
    let root = directory.path().join("python");
    std::fs::write(root.join("consumer.py"), consumer).unwrap();
    if let Some(input) = input {
        std::fs::write(root.join("input.json"), input).unwrap();
    }
    let output =
        Command::new(std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()))
            .current_dir(&root)
            .arg("consumer.py")
            .output()
            .expect("requires native Python 3");
    if !output.status.success() {
        let retained = directory.keep();
        panic!(
            "native fixture retained at {}\n{}{}",
            retained.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    directory
}

fn execute(consumer: &str, input: Option<&str>) -> tempfile::TempDir {
    run_python(&suspect_codegen::python_json::emit(), consumer, input)
}

#[test]
#[ignore = "requires native Python 3"]
fn exact_json_grammar_and_value_vectors() {
    execute(
        r##"import json_runtime as _json
from json_runtime import JsonError, JsonNumber, parse_json, stringify_json

assert _json.JsonNumber is JsonNumber

# Exact number spellings survive, including negative zero and huge exponents.
values = parse_json('[-0, -0.0, 1e999999999, 0.30000000000000004e-10, 123456789012345678901234567890]')
assert [v.token for v in values] == [
    '-0', '-0.0', '1e999999999', '0.30000000000000004e-10',
    '123456789012345678901234567890',
]
assert isinstance(values[0], JsonNumber)

# Object key presence is distinguishable from a null value.
present = parse_json('{"a": null}')
assert list(present) == ['a'] and present['a'] is None
assert 'a' not in parse_json('{}')

# Valid escaped surrogate pairs decode to one scalar.
assert parse_json('"\\ud83d\\ude00"') == '\U0001F600'
assert parse_json('"caf\\u00e9"') == 'caf\u00e9'
assert parse_json('"caf\\u00E9"') == 'caf\u00e9'
assert parse_json('"\\uD83D\\uDE00"') == '\U0001F600'

def expect(kind, text, *, offset=None, path=None):
    try:
        parse_json(text)
    except JsonError as error:
        assert error.kind == kind, (text, error.kind, error)
        if offset is not None:
            assert error.offset == offset, (text, error.offset)
        if path is not None:
            assert error.path == path, (text, error.path)
    else:
        raise AssertionError(f'accepted: {text}')

# Duplicate decoded keys (escape-normalized) are rejected with a path.
expect(_json.DUPLICATE_KEY, '{"a": 1, "\\u0061": 2}', path='$["a"]')
expect(_json.DUPLICATE_KEY, '{"\\u0061": 1, "a": 2}', path='$["a"]')

# Grammar failures carry stable kinds and offsets.
expect(_json.SYNTAX, '[1, 2,]')
expect(_json.SYNTAX, '1 2')
expect(_json.SYNTAX, '[NaN, Infinity]')
expect(_json.SYNTAX, "'text'")
expect(_json.SYNTAX, '{"a" 1}')
expect(_json.SYNTAX, '"a\tb"')
expect(_json.SYNTAX, '"\\ud800"')
expect(_json.SYNTAX, '"\\uD800"')
expect(_json.SYNTAX, '"\\udc00"')
expect(_json.SYNTAX, '"\\ud800\\u0041"')

# str input with a raw lone surrogate is rejected, not silently replaced.
try:
    parse_json('"a\ud800b"')
except JsonError as error:
    assert error.kind == _json.SYNTAX, error.kind
else:
    raise AssertionError('accepted raw lone surrogate')

# bytes input must be well-formed UTF-8, with a located byte offset.
try:
    parse_json(b'"a\xffb"')
except JsonError as error:
    assert error.kind == _json.INVALID_UTF8, error.kind
    assert error.offset == 2, error.offset
else:
    raise AssertionError('accepted malformed UTF-8')

# Escapes normalize to the same decoded string.
assert parse_json(r'"A"') == parse_json('"\\u0041"') == 'A'
print('grammar vectors ok')
"##,
        None,
    );
}

#[test]
#[ignore = "requires native Python 3"]
fn json_number_is_symbolic_and_bounded_exact() {
    execute(
        r##"import json_runtime as _json
from json_runtime import JsonError, JsonNumber

def expect_int(token, expected, max_digits=4096):
    number = JsonNumber.parse(token)
    assert number.token == token, (token, number.token)
    assert number.to_int(max_digits) == expected, token

def expect_failure(token, kind, max_digits=4096):
    try:
        JsonNumber.parse(token).to_int(max_digits)
    except JsonError as error:
        assert error.kind == kind, (token, error.kind)
    else:
        raise AssertionError(f'converted: {token}')

# Wide integers convert exactly, beyond double precision.
expect_int('9007199254740993', 9007199254740993)
expect_int(
    '123456789012345678901234567890123456789',
    123456789012345678901234567890123456789,
)
expect_int('-9007199254740993', -9007199254740993)

# Exponent scaling is exact and never routed through float.
expect_int('10e-1', 1)
expect_int('1500e-2', 15)
expect_int('0e0', 0)
expect_int('-0', 0)
expect_int('1e6', 1000000)

# Symbolic integrality needs no exponent-sized allocation.
assert JsonNumber.parse('1e999999999').is_integer() is True
assert JsonNumber.parse('-1e999999999').is_integer() is True
assert JsonNumber.parse('1e-999999999').is_integer() is False
assert JsonNumber.parse('3.0').is_integer() is True
assert JsonNumber.parse('3.5').is_integer() is False
assert JsonNumber.parse('0.0').is_integer() is True
assert JsonNumber.parse('1500e-3').is_integer() is False
assert JsonNumber.parse('1500e-2').is_integer() is True

# Non-integral and over-budget conversions fail with stable kinds.
expect_failure('15e-1', _json.NOT_INTEGER)
expect_failure('0.1', _json.NOT_INTEGER)
expect_failure('1e1000000', _json.RESOURCE_LIMIT)
expect_failure('9' * 5000, _json.RESOURCE_LIMIT)
expect_failure('9' * 4100, _json.RESOURCE_LIMIT)
assert JsonNumber.parse('9' * 4096).to_int() == int('9' * 4096)

# Exact conversion is independent of str(float) and of the global
# int<->str digit limit; chunked conversion must round-trip.
token = '123456789' * 455  # 4095 digits, under the default budget
assert JsonNumber.parse(token).to_int() == int(token)
print('number conversions ok')
"##,
        None,
    );
}

#[test]
#[ignore = "requires native Python 3"]
fn stringify_accepts_exact_values_and_rejects_ambiguous_ones() {
    execute(
        r##"import decimal
import enum
import json_runtime as _json
from json_runtime import JsonError, JsonNumber, stringify_json

def expect_failure(value, kind, limits=None):
    try:
        stringify_json(value, limits)
    except JsonError as error:
        assert error.kind == kind, (error.kind, error)
    else:
        raise AssertionError(f'encoded: {value!r}')

assert stringify_json(None) == 'null'
assert stringify_json(True) == 'true'
assert stringify_json(False) == 'false'
assert stringify_json(12345) == '12345'
assert stringify_json(-0) == '0'
assert stringify_json([True, None, 1, 'x']) == '[true,null,1,"x"]'

# Key order is insertion order and escapes match the RFC table.
assert stringify_json({'b': 1, 'a': 2}) == '{"b":1,"a":2}'
assert (
    stringify_json('\x00\x1f"\\\u00e9 \U0001F600')
    == '"\\u0000\\u001f\\"\\\\\u00e9 \U0001F600"'
)

# Number spelling is preserved verbatim.
assert stringify_json(JsonNumber.parse('-0.0')) == '-0.0'
assert stringify_json(JsonNumber.parse('1e999999999')) == '1e999999999'

# bool is not an int and int is not bool on the wire.
expect_failure({True: 1}, _json.UNSUPPORTED_VALUE)

# float, Decimal, unknown subclasses, and hooks are rejected, not dispatched.
expect_failure(decimal.Decimal('1.5'), _json.UNSUPPORTED_VALUE)
expect_failure(float('nan'), _json.UNSUPPORTED_VALUE)
expect_failure((1, 2), _json.UNSUPPORTED_VALUE)
expect_failure({1, 2}, _json.UNSUPPORTED_VALUE)
expect_failure({'k': object()}, _json.UNSUPPORTED_VALUE)

class Width(str):
    pass

class FakeNumber(JsonNumber):
    def __init__(self):
        pass

class Level(enum.IntEnum):
    LOW = 7

expect_failure(Width('x'), _json.UNSUPPORTED_VALUE)
expect_failure(Level.LOW, _json.UNSUPPORTED_VALUE)
expect_failure(FakeNumber(), _json.UNSUPPORTED_VALUE)
expect_failure({'a': 1.5}, _json.UNSUPPORTED_VALUE)
expect_failure([1, [2.0]], _json.UNSUPPORTED_VALUE)

# Dict keys must be exactly str.
expect_failure({1: 'a'}, _json.UNSUPPORTED_VALUE)
expect_failure({None: 'a'}, _json.UNSUPPORTED_VALUE)

# Cycles are detected with a located path, not an unbounded walk.
cyclic = {'a': []}
cyclic['a'].append(cyclic)
expect_failure(cyclic, _json.CYCLE)
self_list = []
self_list.append(self_list)
expect_failure(self_list, _json.CYCLE)
shared = [1]
assert stringify_json([shared, shared]) == '[[1],[1]]', 'shared references are not cycles'

# Lone surrogates cannot be emitted; the scalars-only policy is explicit.
expect_failure('a\ud800b', _json.UNSUPPORTED_VALUE)
expect_failure({'k\udfff': 1}, _json.UNSUPPORTED_VALUE)

# Output is bounded before appends grow it.
tiny = _json.JsonLimits(max_output_bytes=4)
expect_failure([1, 2], _json.RESOURCE_LIMIT, tiny)
expect_failure('abcdef', _json.RESOURCE_LIMIT, tiny)
assert stringify_json([1], tiny) == '[1]'
print('stringify policy ok')
"##,
        None,
    );
}

#[test]
#[ignore = "requires native Python 3"]
fn limits_bound_input_depth_work_and_ceiling_without_global_state() {
    execute(
        r##"import json_runtime as _json
from json_runtime import JsonError, JsonLimits, parse_json

def expect_limit(text, limits, *, raw=None):
    try:
        parse_json(raw if raw is not None else text, limits)
    except JsonError as error:
        assert error.kind == _json.RESOURCE_LIMIT, (error.kind, error)
    else:
        raise AssertionError(f'accepted under limit: {text!r}')

# Input byte budgets fail before parsing does work.
expect_limit('12345', JsonLimits(max_input_bytes=4))
assert parse_json('1234', JsonLimits(max_input_bytes=4)).to_int() == 1234

# Depth counts containers; 128 nesting fits the default limits.
deep_ok = '[' * 128 + '0' + ']' * 128
nested = parse_json(deep_ok)
depth = 0
cursor = nested
while isinstance(cursor, list):
    depth += 1
    cursor = cursor[0]
assert depth == 128, depth
expect_limit('[' * 129 + ']' * 129, JsonLimits())

# 256 is the hard ceiling regardless of configuration; no global
# recursionlimit or int digit limit is consulted or changed.
assert parse_json('[' * 256 + ']' * 256, JsonLimits(max_depth=256)) is not None
expect_limit('[' * 257 + ']' * 257, JsonLimits(max_depth=256))
expect_limit('[]', JsonLimits(max_depth=300))

# Work budgets bound scanning of whitespace, strings, and numbers.
expect_limit('   [1]', JsonLimits(max_work=2))
expect_limit('"' + 'a' * 100 + '"', JsonLimits(max_work=10))
expect_limit('123456789012345678901234567890', JsonLimits(max_work=5))

# Limits validate their own shape.
try:
    JsonLimits(max_depth=-1)
except ValueError:
    pass
else:
    raise AssertionError('accepted negative limit')

import sys
assert sys.getrecursionlimit() >= 300, 'runtime must not lower the recursion limit'
print('limits ok')
"##,
        None,
    );
}

#[test]
#[ignore = "requires native Python 3"]
fn json_number_survives_type_identity_checks_in_encoding() {
    execute(
        r##"import json_runtime as _json
from json_runtime import JsonError, JsonNumber, stringify_json

# The model worker passes JsonNumber instances through type identity, not
# subclass dispatch; numbers encode by exact token, strings by exact value.
assert stringify_json({'price': JsonNumber.parse('1.25e3')}) == '{"price":1.25e3}'
assert stringify_json([JsonNumber.parse('-0')]) == '[-0]'

# JsonNumber equality is spelling equality, hashable for model dicts.
assert JsonNumber.parse('1.0') == JsonNumber.parse('1.0')
assert JsonNumber.parse('1.0') != JsonNumber.parse('1')
assert hash(JsonNumber.parse('1.0')) == hash(JsonNumber.parse('1.0'))
assert JsonNumber.parse('1.0').token == '1.0'
assert repr(JsonNumber.parse('-0')).startswith('JsonNumber(')

# parse() is the documented alias of the validating constructor.
assert isinstance(JsonNumber.parse('12e1'), JsonNumber)
for bad in ['1.', '.5', '1e', '+1', '01', '1.2.3', 'NaN', 'Infinity', '']:
    try:
        JsonNumber.parse(bad)
    except JsonError as error:
        assert error.kind == _json.SYNTAX, (bad, error.kind)
    else:
        raise AssertionError(f'accepted token: {bad!r}')
try:
    JsonNumber.parse('1 2')
except JsonError as error:
    assert error.kind == _json.SYNTAX, error.kind
else:
    raise AssertionError('accepted trailing characters')
try:
    JsonNumber(1.5)
except TypeError:
    pass
else:
    raise AssertionError('accepted non-str token')
print('number type contract ok')
"##,
        None,
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT and native Python 3; missing inputs fail"]
fn tracked_openrouter_contract_documents_roundtrip_through_python_runtime() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .expect("set OPENROUTER_WEB_ROOT to the source checkout");
    for relative in [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ] {
        let path = std::path::Path::new(&root).join(relative);
        let workspace = std::sync::Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        let contract = suspect_ir::contract::Contract::from_workspace(
            &workspace,
            &suspect_source::Uri::from_path(&path).unwrap(),
        )
        .expect("tracked OpenRouter contract input must load");
        let original = contract.document(contract.entry()).unwrap();
        let text = serde_json::to_string(original).unwrap();
        let directory = execute(
            r##"import json_runtime as _json
with open('input.json', 'rb') as handle:
    document = _json.parse_json(handle.read())
with open('output.json', 'wb') as handle:
    handle.write(_json.stringify_json(document).encode('utf-8'))
"##,
            Some(&text),
        );
        let encoded = std::fs::read(directory.path().join("python/output.json")).unwrap();
        // Normalization oracle: serde_json::Value equality ignores object key
        // order (the Python writer keeps insertion order, the Rust writer
        // sorts), and numbers are re-parsed to serde_json numbers. The
        // tracked source documents stay read-only.
        let roundtripped: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            &roundtripped, original,
            "{relative}: python runtime changed the contract document"
        );
    }
}

#[test]
#[ignore = "requires native Python 3"]
fn exact_numeric_conversions_match_an_independent_rational_oracle() {
    execute(
        r#"
import random
from fractions import Fraction
from json_runtime import JsonNumber, JsonError, NOT_INTEGER
randomizer = random.Random(8259)
for _ in range(10000):
    coefficient = randomizer.randrange(-1000000, 1000001)
    fraction = randomizer.randrange(0, 7)
    exponent = randomizer.randrange(-40, 41)
    digits = str(abs(coefficient)).rjust(fraction + 1, '0')
    mantissa = digits if fraction == 0 else digits[:-fraction] + '.' + digits[-fraction:]
    token = ('-' if coefficient < 0 else '') + mantissa + 'e' + str(exponent)
    expected = Fraction(coefficient, 10 ** fraction) * (Fraction(10 ** exponent) if exponent >= 0 else Fraction(1, 10 ** -exponent))
    number = JsonNumber.parse(token)
    assert number.is_integer() == (expected.denominator == 1), token
    if expected.denominator == 1:
        assert number.to_int() == expected.numerator, token
    else:
        try: number.to_int()
        except JsonError as error: assert error.kind == NOT_INTEGER, token
        else: raise AssertionError(token)
"#,
        None,
    );
}

#[test]
#[ignore = "requires native Python 3"]
fn byte_budgets_and_byte_input_locations_use_utf8_bytes() {
    execute(
        r#"
from json_runtime import JsonError, JsonLimits, parse_json, RESOURCE_LIMIT, SYNTAX
for value, limit in [('"éé"', 4), (memoryview(bytearray(b'1234')).cast('I'), 3)]:
    try: parse_json(value, JsonLimits(max_input_bytes=limit))
    except JsonError as error: assert error.kind == RESOURCE_LIMIT
    else: raise AssertionError('byte cap bypassed')
try: parse_json('["é",]'.encode('utf-8'))
except JsonError as error:
    assert error.kind == SYNTAX
    assert error.offset == len('["é",'.encode('utf-8')), error.offset
else: raise AssertionError('invalid trailing comma accepted')
"#,
        None,
    );
}
