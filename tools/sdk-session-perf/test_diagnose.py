"""Independent worked observations; synthetic values never qualify a benchmark."""

import unittest

import compare
import diagnose
from test_compare import synthetic_report


def observation(name, generate_ms):
    report = synthetic_report(name, iterations=6)
    case = report["cases"][0]["report"]
    case["configuration"]["schedule"] = "fixed-edit-revert-cycles-v1"
    case["configuration_sha256"] = compare.digest(case["configuration"])
    for sample in case["samples"]:
        sample["generate"]["ms"] = generate_ms
        sample["write"]["ms"] = 1.0
        sample["refresh_ms"] = generate_ms + 1.0
    return report


class DiagnosisTests(unittest.TestCase):
    def test_process_strata_survive_aggregation_and_are_never_a_gate(self):
        result = diagnose.analyze([observation("first", 10.0), observation("second", 30.0)],
                                  metrics=("generate_ms",))
        self.assertFalse(result["gating_eligible"])
        row = next(row for row in result["dimensions"] if row["scenario"] == "cold")
        self.assertEqual(row["pooled"]["samples"], 12)
        self.assertEqual(row["pooled"]["median"], 20.0)
        self.assertEqual(row["pooled"]["p95"], 30.0)
        self.assertEqual([item["summary"]["p95"] for item in row["processes"]], [10.0, 30.0])
        self.assertEqual(row["between_process_p95_spread"], 20.0)
        self.assertEqual(row["positions"][0]["position"], 0)
        self.assertEqual(row["positions"][0]["summary"]["samples"], 12)

    def test_serial_order_is_visible_and_constant_series_has_no_correlation_estimate(self):
        self.assertAlmostEqual(diagnose.lag_correlation([1, 2, 3, 4, 5], 1), 1.0)
        self.assertAlmostEqual(diagnose.lag_correlation([0, 1, 0, 1, 0, 1], 1), -1.0)
        self.assertIsNone(diagnose.lag_correlation([4, 4, 4, 4], 1))

    def test_block_interval_for_constant_processes_is_exact_and_explicitly_exploratory(self):
        result = diagnose.block_interval([[12.0] * 20, [12.0] * 20], 5, 200, 4)
        self.assertEqual((result["lower"], result["upper"]), (12.0, 12.0))
        self.assertEqual(result["block_length"], 5)
        self.assertFalse(result["coverage_established"])
        with self.assertRaises(compare.ReportError):
            diagnose.block_interval([[12.0] * 3], 5, 200, 4)

    def test_incompatible_schedules_are_separate_cohorts(self):
        first = observation("fixed", 10.0)
        second = synthetic_report("rotating", iterations=6)
        result = diagnose.analyze([first, second], metrics=("generate_ms",))
        self.assertEqual(len(result["cohorts"]), 2)
        self.assertEqual(len(result["dimensions"]), 16)
        self.assertTrue(all(len(row["processes"]) == 1 for row in result["dimensions"]))


if __name__ == "__main__":
    unittest.main()
