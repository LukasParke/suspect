"""Expected-correct public JSON regressions; no generated-runtime patches."""
import json
from pathlib import Path
import statistics
import sys
import time
import unittest

from json_runtime import (
    DUPLICATE_KEY,
    NOT_INTEGER,
    RESOURCE_LIMIT,
    UNSUPPORTED_VALUE,
    JsonError,
    JsonLimits,
    JsonNumber,
    parse_json,
    stringify_json,
)


class ExactNumbers(unittest.TestCase):
    def test_zero_padded_exponents(self):
        # Independent mathematical expectations around the significant-exponent
        # boundary. Padding changes spelling, never the value or integrality.
        for padding in (0, 39, 40, 41, 128, 4096):
            zeros = "0" * padding
            for token, expected in (
                ("1e" + zeros + "0", 1),
                ("10e-" + zeros + "1", 1),
                ("0.1e+" + zeros + "1", 1),
                ("-12500e-" + zeros + "2", -125),
                ("-0e-" + zeros + "9", 0),
            ):
                with self.subTest(padding=padding, prefix=token[:12]):
                    number = parse_json(token.encode("ascii"))
                    self.assertIs(type(number), JsonNumber)
                    self.assertEqual(number.token, token)
                    self.assertTrue(number.is_integer())
                    self.assertEqual(number.to_int(8), expected)
                    self.assertEqual(stringify_json(number), token)
            token = "0.1e-" + zeros + "0"
            with self.subTest(padding=padding, fractional=True):
                number = JsonNumber(token)
                self.assertFalse(number.is_integer())
                with self.assertRaises(JsonError) as caught:
                    number.to_int(8)
                self.assertEqual(caught.exception.kind, NOT_INTEGER)

        # Genuine huge exponents remain symbolic and obey conversion limits.
        huge = JsonNumber("1e" + "9" * 80)
        self.assertTrue(huge.is_integer())
        with self.assertRaises(JsonError) as caught:
            huge.to_int(8)
        self.assertEqual(caught.exception.kind, RESOURCE_LIMIT)
        self.assertFalse(JsonNumber("1e-" + "9" * 80).is_integer())
        self.assertEqual(JsonNumber("0e-" + "9" * 80).to_int(1), 0)

    def test_exact_integer_output_bytes(self):
        for value, expected in (
            (0, "0"), (1, "1"), (7, "7"), (8, "8"), (9, "9"),
            (99, "99"), (999, "999"), (99999, "99999"),
            (-8, "-8"), (-999, "-999"),
        ):
            with self.subTest(value=value):
                fitting = JsonLimits(max_output_bytes=len(expected))
                self.assertEqual(stringify_json(value, fitting), expected)
                self.assertEqual(stringify_json(JsonNumber(expected), fitting), expected)
                with self.assertRaises(JsonError) as caught:
                    stringify_json(value, JsonLimits(max_output_bytes=len(expected) - 1))
                self.assertEqual(caught.exception.kind, RESOURCE_LIMIT)

        # More digits than the interpreter's usual int-to-str cap, without
        # changing that process-global setting or using it as the oracle.
        value = 10**5000 - 1
        expected = "9" * 5000
        self.assertEqual(stringify_json(value, JsonLimits(max_output_bytes=5000)), expected)
        with self.assertRaises(JsonError) as caught:
            stringify_json(value, JsonLimits(max_output_bytes=4999))
        self.assertEqual(caught.exception.kind, RESOURCE_LIMIT)


class MeteredText(str):
    """A str-compatible input recording visible character/search work.

    This instruments the public input, not the generated parser. It charges the
    distance actually searched by find(), so repeated unsuccessful suffix scans
    fail deterministically. Correct parsers can use indexing, bounded searches,
    regular expressions or a native scanner; no private function/count is fixed.
    """
    def __new__(cls, text):
        value = super().__new__(cls, text)
        value.units = 0
        value.allowance = 32 * len(text)
        return value

    def charge(self, amount):
        self.units += amount
        if self.units > self.allowance:
            raise AssertionError(
                f"string scanning exceeded a linear input-work allowance: "
                f"{self.units} > {self.allowance}"
            )

    def __iter__(self):
        for char in super().__iter__():
            self.charge(1)
            yield char

    def __getitem__(self, key):
        value = super().__getitem__(key)
        self.charge(len(value))
        return value

    def find(self, needle, start=0, end=None):
        stop = len(self) if end is None else min(end, len(self))
        found = super().find(needle, start, stop)
        self.charge(max(0, (found + len(needle) if found >= 0 else stop) - start))
        return found


class StringScanning(unittest.TestCase):
    def test_short_strings_and_escapes_stay_within_linear_input_work(self):
        count = 2048
        cases = (
            ('[' + ','.join(['"a"'] * count) + ']', ["a"] * count),
            ('"' + '\\n' * count + '"', "\n" * count),
        )
        for text, expected in cases:
            with self.subTest(bytes=len(text)):
                measured = MeteredText(text)
                self.assertEqual(parse_json(measured, JsonLimits(max_work=16 * len(text))), expected)
                self.assertLessEqual(measured.units, measured.allowance)
                with self.assertRaises(JsonError) as caught:
                    parse_json(text, JsonLimits(max_work=1))
                self.assertEqual(caught.exception.kind, RESOURCE_LIMIT)

    def test_observational_raw_scaling_samples(self):
        # Wall-clock measurements are evidence only: shared CI machines and
        # interpreter versions have no tight timing or ratio assertion here.
        rows = []
        for count in (4096, 16384, 65536):
            text = b'[' + b'"a",' * (count - 1) + b'"a"]'
            samples = []
            for _ in range(3):
                start = time.perf_counter()
                parsed = parse_json(text)
                samples.append(time.perf_counter() - start)
                self.assertEqual(parsed, ["a"] * count)
            rows.append({"bytes": len(text), "values": count,
                         "seconds": samples, "median_seconds": statistics.median(samples)})
        Path("json-scaling-observations.json").write_text(json.dumps(rows, indent=2) + "\n")
        print("JSON_SCALING_OBSERVATIONS=" + json.dumps(rows), flush=True)


class DiagnosticFormatting(unittest.TestCase):
    def test_real_parse_and_write_errors_have_safe_bounded_formatting(self):
        controls = "\n\r\t\x1b\x7f\u0085\u2028\u2029"
        for length in (8192, 65536):
            key = "untrusted" + controls + "x" * length
            encoded_key = json.dumps(key)
            text = ('{' + encoded_key + ':0,' + encoded_key + ':1}').encode("ascii")
            with self.subTest(operation="parse", key_length=length):
                with self.assertRaises(JsonError) as caught:
                    parse_json(text)
                self.assertEqual(caught.exception.kind, DUPLICATE_KEY)
                self.assertIsNotNone(caught.exception.offset)
                self.assert_safe(caught.exception, controls)
            with self.subTest(operation="write", key_length=length):
                with self.assertRaises(JsonError) as caught:
                    stringify_json({key: object()})
                self.assertEqual(caught.exception.kind, UNSUPPORTED_VALUE)
                self.assert_safe(caught.exception, controls)

    def assert_safe(self, error, controls):
        self.assertIsNotNone(error.path)
        rendered = str(error)
        # Generous public diagnostic ceiling, independent of truncation marker
        # or the implementation's smaller internal formatting allowance.
        self.assertLessEqual(len(rendered.encode("utf-8")), 2048)
        self.assertTrue(all(char not in rendered for char in controls), repr(rendered))
        self.assertIn(error.kind, rendered)


if __name__ == "__main__":
    print("NATIVE_PYTHON=" + json.dumps({"executable": sys.executable, "version": sys.version}), flush=True)
    unittest.main()
