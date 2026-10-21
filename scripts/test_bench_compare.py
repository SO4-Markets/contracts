#!/usr/bin/env python3
"""Unit tests for scripts/bench_compare.py (#1064). Run: python3 scripts/test_bench_compare.py -v"""
import copy
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench_compare as bc  # noqa: E402


def doc(scenarios, **extra):
    d = {"schema": bc.SCHEMA_VERSION, "scenarios": scenarios}
    d.update(extra)
    return d


THRESHOLDS = {
    "instructions": {"pct": 2.0, "abs": 1000},
    "write_entries": {"pct": 0.0, "abs": 0},
}


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.base = doc({"s": {"instructions": 100_000, "write_entries": 5}}, thresholds=THRESHOLDS)

    def run_compare(self, cur, base=None, root="."):
        return bc.compare(base or self.base, doc(cur), root)

    def test_identical_run_passes(self):
        rows, failures = self.run_compare({"s": {"instructions": 100_000, "write_entries": 5}})
        self.assertEqual(failures, [])
        self.assertTrue(all(r[4] == "same" for r in rows))

    def test_increase_within_tolerance_passes(self):
        # +1.5% and +1500 instructions: above abs (1000) but under pct (2%).
        _, failures = self.run_compare({"s": {"instructions": 101_500, "write_entries": 5}})
        self.assertEqual(failures, [])

    def test_regression_past_both_thresholds_fails(self):
        _, failures = self.run_compare({"s": {"instructions": 103_000, "write_entries": 5}})
        self.assertEqual(len(failures), 1)
        self.assertIn("instructions", failures[0])

    def test_any_extra_ledger_write_fails(self):
        _, failures = self.run_compare({"s": {"instructions": 100_000, "write_entries": 6}})
        self.assertEqual(len(failures), 1)
        self.assertIn("write_entries", failures[0])

    def test_improvement_passes_and_is_reported(self):
        rows, failures = self.run_compare({"s": {"instructions": 90_000, "write_entries": 4}})
        self.assertEqual(failures, [])
        self.assertEqual({r[4] for r in rows}, {"improved"})

    def test_missing_scenario_fails(self):
        _, failures = self.run_compare({})
        self.assertTrue(any("was not run" in f for f in failures))

    def test_new_scenario_without_baseline_fails(self):
        _, failures = self.run_compare({
            "s": {"instructions": 100_000, "write_entries": 5},
            "t": {"instructions": 1, "write_entries": 1},
        })
        self.assertTrue(any("has no baseline" in f for f in failures))

    def test_schema_mismatch_is_a_usage_error(self):
        with self.assertRaises(bc.GateError):
            bc.compare(self.base, {"schema": 99, "scenarios": {}})

    def test_markdown_lists_before_after(self):
        rows, failures = self.run_compare({"s": {"instructions": 103_000, "write_entries": 5}})
        md = bc.render_markdown(rows, failures, self.base)
        self.assertIn("| `s` | instructions | 100000 | 103000 | +3.00% | REGRESSION |", md)
        self.assertIn("Gate failed", md)


class BoundedConstantTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        os.makedirs(os.path.join(self.root, "src"))
        self.base = doc({}, bounded_constants={"MAX_BATCH": {"file": "src/lib.rs", "value": 5}})

    def tearDown(self):
        self.tmp.cleanup()

    def write_src(self, text):
        with open(os.path.join(self.root, "src", "lib.rs"), "w") as f:
            f.write(text)

    def test_unchanged_or_lowered_constant_passes(self):
        for value in ("5", "3"):
            self.write_src(f"pub const MAX_BATCH: u32 = {value};\n")
            _, failures = bc.compare(self.base, doc({}), self.root)
            self.assertEqual(failures, [], value)

    def test_raised_constant_fails(self):
        self.write_src("pub const MAX_BATCH: u32 = 1_0;\n")
        _, failures = bc.compare(self.base, doc({}), self.root)
        self.assertEqual(len(failures), 1)
        self.assertIn("raised 5 -> 10", failures[0])

    def test_missing_constant_fails(self):
        self.write_src("pub const OTHER: u32 = 5;\n")
        _, failures = bc.compare(self.base, doc({}), self.root)
        self.assertIn("not found", failures[0])


class UpdateAndRationaleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        os.makedirs(os.path.join(self.root, "src"))
        with open(os.path.join(self.root, "src", "lib.rs"), "w") as f:
            f.write("const MAX_BATCH: u32 = 8;\n")
        self.base = doc(
            {"s": {"instructions": 1}},
            meta={"rationale": "initial baseline", "updated": "2026-01-01"},
            thresholds=THRESHOLDS,
            bounded_constants={"MAX_BATCH": {"file": "src/lib.rs", "value": 5}},
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_update_requires_rationale(self):
        for rationale in ("", "   ", "short"):
            with self.assertRaises(bc.GateError):
                bc.update_baseline(self.base, doc({"s": {"instructions": 2}}), rationale, self.root)

    def test_update_records_rationale_and_refreshes_constants(self):
        new = bc.update_baseline(
            self.base, doc({"s": {"instructions": 2}}), "raised batch to 8 after stress run",
            self.root, today="2026-10-16",
        )
        self.assertEqual(new["meta"], {"rationale": "raised batch to 8 after stress run", "updated": "2026-10-16"})
        self.assertEqual(new["bounded_constants"]["MAX_BATCH"]["value"], 8)
        self.assertEqual(new["scenarios"], {"s": {"instructions": 2}})
        self.assertEqual(new["thresholds"], THRESHOLDS)

    def test_changed_evidence_needs_new_rationale(self):
        head = copy.deepcopy(self.base)
        head["scenarios"]["s"]["instructions"] = 2
        self.assertEqual(len(bc.check_rationale(self.base, head)), 1)
        head["meta"]["rationale"] = "explained change"
        self.assertEqual(bc.check_rationale(self.base, head), [])

    def test_unchanged_baseline_passes_rationale_check(self):
        self.assertEqual(bc.check_rationale(self.base, copy.deepcopy(self.base)), [])

    def test_new_baseline_must_have_rationale(self):
        self.assertEqual(bc.check_rationale(None, self.base), [])
        self.assertEqual(len(bc.check_rationale(None, doc({}))), 1)


class CliTests(unittest.TestCase):
    def test_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            base, cur = os.path.join(d, "base.json"), os.path.join(d, "cur.json")
            bc.write_json(base, doc({"s": {"instructions": 100}}))
            bc.write_json(cur, doc({"s": {"instructions": 100}}))
            self.assertEqual(bc.main(["compare", "--baseline", base, "--current", cur, "--root", d]), 0)
            bc.write_json(cur, doc({"s": {"instructions": 200}}))
            self.assertEqual(bc.main(["compare", "--baseline", base, "--current", cur, "--root", d]), 1)
            self.assertEqual(bc.main(["compare", "--baseline", os.path.join(d, "nope.json"),
                                      "--current", cur]), 2)


if __name__ == "__main__":
    unittest.main()
