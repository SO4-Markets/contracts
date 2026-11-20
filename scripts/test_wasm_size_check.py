"""Tests for wasm-size-check.py.

Covers the three branches identified in the issue:
  1. No base directory supplied  → sizes are reported, exit 0.
  2. Contract grows within budget (< BLOCK_THRESHOLD) → ✅ / 🟡, exit 0.
  3. Contract exceeds BLOCK_THRESHOLD → 🔴 BLOCK row, exit 1.

Also covers:
  4. Test contracts (test_faucet, test_token) are excluded.
  5. GITHUB_STEP_SUMMARY is written when the env variable is set.
  6. A contract present in HEAD but absent from BASE shows "—" baseline.
"""

import importlib.util
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

# Load the module under test.  It lives in the same directory as this test.
_SCRIPT = Path(__file__).parent / "wasm-size-check.py"
_spec = importlib.util.spec_from_file_location("wasm_size_check", _SCRIPT)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)

contract_sizes = _mod.contract_sizes
report_row = _mod.report_row
main = _mod.main
WARN_THRESHOLD = _mod.WARN_THRESHOLD
BLOCK_THRESHOLD = _mod.BLOCK_THRESHOLD
TEST_CONTRACTS = _mod.TEST_CONTRACTS


def _make_wasm_dir(contract_sizes_map: dict) -> str:
    """Write stub .wasm files of the given byte sizes, return the directory."""
    d = tempfile.mkdtemp()
    for name, size in contract_sizes_map.items():
        path = os.path.join(d, f"{name}.wasm")
        with open(path, "wb") as f:
            f.write(b"\x00" * size)
    return d


class TestContractSizes(unittest.TestCase):

    def test_reads_wasm_files(self):
        d = _make_wasm_dir({"clob": 1000, "oracle": 2000})
        sizes = contract_sizes(d)
        self.assertEqual(sizes, {"clob": 1000, "oracle": 2000})

    def test_excludes_test_contracts(self):
        d = _make_wasm_dir({"clob": 1000, "test_faucet": 500, "test_token": 300})
        sizes = contract_sizes(d)
        self.assertNotIn("test_faucet", sizes)
        self.assertNotIn("test_token", sizes)
        self.assertIn("clob", sizes)

    def test_ignores_non_wasm_files(self):
        d = _make_wasm_dir({"clob": 1000})
        # plant a non-.wasm file
        with open(os.path.join(d, "README.md"), "w") as f:
            f.write("ignored")
        sizes = contract_sizes(d)
        self.assertEqual(list(sizes.keys()), ["clob"])


class TestReportRow(unittest.TestCase):

    def test_no_base(self):
        row = report_row("clob", 1000, None)
        self.assertIn("—", row)
        self.assertIn("1,000", row)

    def test_ok_no_growth(self):
        row = report_row("clob", 1000, 1000)
        self.assertIn("✅", row)

    def test_warn_threshold(self):
        base = 1000
        head = int(base * (1 + WARN_THRESHOLD + 0.01))  # just above warn
        row = report_row("clob", head, base)
        self.assertIn("🟡 WARN", row)

    def test_block_threshold(self):
        base = 1000
        head = int(base * (1 + BLOCK_THRESHOLD + 0.01))  # just above block
        row = report_row("clob", head, base)
        self.assertIn("🔴 BLOCK", row)

    def test_negative_growth_is_ok(self):
        row = report_row("clob", 800, 1000)
        self.assertIn("✅", row)


class TestMain(unittest.TestCase):

    def _run(self, argv):
        """Run main() with captured stdout, return (exit_code, stdout)."""
        buf = io.StringIO()
        with patch("sys.stdout", buf):
            code = main(argv)
        return code, buf.getvalue()

    def test_no_base_exits_0(self):
        head = _make_wasm_dir({"clob": 1000})
        code, out = self._run(["wasm-size-check.py", head])
        self.assertEqual(code, 0)
        self.assertIn("clob", out)

    def test_no_growth_exits_0(self):
        head = _make_wasm_dir({"clob": 1000})
        base = _make_wasm_dir({"clob": 1000})
        code, out = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 0)
        self.assertIn("✅", out)

    def test_growth_under_block_threshold_exits_0(self):
        base_size = 100_000
        head_size = int(base_size * (1 + BLOCK_THRESHOLD - 0.01))  # just under
        head = _make_wasm_dir({"clob": head_size})
        base = _make_wasm_dir({"clob": base_size})
        code, _ = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 0)

    def test_growth_over_block_threshold_exits_1(self):
        base_size = 100_000
        head_size = int(base_size * (1 + BLOCK_THRESHOLD + 0.01))  # just over
        head = _make_wasm_dir({"clob": head_size})
        base = _make_wasm_dir({"clob": base_size})
        code, out = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 1)
        self.assertIn("🔴 BLOCK", out)

    def test_new_contract_in_head_shows_dash_base(self):
        head = _make_wasm_dir({"clob": 1000, "new_contract": 500})
        base = _make_wasm_dir({"clob": 1000})
        code, out = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 0)
        # new_contract has no base → should show dash in base column
        self.assertIn("new_contract", out)
        self.assertIn("—", out)

    def test_test_contracts_excluded_from_budget(self):
        # Even if test_faucet grows massively, exit should be 0.
        base_size = 100_000
        head_size = base_size * 10
        head = _make_wasm_dir({"test_faucet": head_size})
        base = _make_wasm_dir({"test_faucet": base_size})
        code, out = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 0)
        self.assertNotIn("test_faucet", out)

    def test_github_step_summary_written(self):
        head = _make_wasm_dir({"clob": 1000})
        base = _make_wasm_dir({"clob": 1000})
        with tempfile.NamedTemporaryFile(mode="w", suffix=".md", delete=False) as f:
            summary_path = f.name
        try:
            with patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": summary_path}):
                buf = io.StringIO()
                with patch("sys.stdout", buf):
                    main(["wasm-size-check.py", head, base])
            content = Path(summary_path).read_text()
            self.assertIn("clob", content)
        finally:
            os.unlink(summary_path)

    def test_multiple_contracts_one_blocked(self):
        # Only the blocked contract causes exit 1.
        base = _make_wasm_dir({"clob": 100_000, "oracle": 50_000})
        head = _make_wasm_dir({
            "clob":   int(100_000 * (1 + BLOCK_THRESHOLD + 0.01)),  # blocked
            "oracle": 50_000,                                         # fine
        })
        code, out = self._run(["wasm-size-check.py", head, base])
        self.assertEqual(code, 1)
        self.assertIn("🔴 BLOCK", out)
        self.assertIn("✅", out)


if __name__ == "__main__":
    unittest.main()
