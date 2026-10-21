#!/usr/bin/env python3
"""
test_gen_auth_audit.py — Regression coverage for gen_auth_audit.py's --check mode.

Issue #356: docs/auth-audit.md previously certified two functions
(set_account_principal_delta / get_account_principal_delta) that did not exist
anywhere in contracts/data_store/src/lib.rs, and mis-stated get_position_manager
as an auth-checked "✅ PASS" read when it is actually an unrestricted public read.
gen_auth_audit.py --check was added to catch exactly this class of drift, but
had no automated test of its own — this fills that gap by exercising
validate_against_source() against a fabricated source/doc pair with both a
phantom (documented-but-nonexistent) entry and a missing (undocumented-but-real)
entry, then confirms a freshly generated doc round-trips clean.

Usage:
    python3 scripts/test_gen_auth_audit.py
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_PATH = Path(__file__).resolve().parent / "gen_auth_audit.py"


def _load_gen_auth_audit():
    spec = importlib.util.spec_from_file_location("gen_auth_audit", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    # dataclasses' field-type resolution looks the module up via
    # sys.modules[cls.__module__], so it must be registered before exec.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


gen_auth_audit = _load_gen_auth_audit()

FAKE_SOURCE = """
#![no_std]

#[contractimpl]
impl FakeContract {
    pub fn get_position_manager(env: Env, owner: Address, market: Address) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Addr(owner))
    }

    pub fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::U128(key), &value);
        value
    }
}
"""


class GenAuthAuditTests(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmpdir.cleanup)
        root = Path(self.tmpdir.name)

        crate_dir = root / "contracts" / "fake_contract" / "src"
        crate_dir.mkdir(parents=True)
        (crate_dir / "lib.rs").write_text(FAKE_SOURCE)

        self.output_path = root / "docs" / "auth-audit.md"
        self.output_path.parent.mkdir(parents=True)

        self.patches = [
            patch.object(gen_auth_audit, "CONTRACTS_DIR", root / "contracts"),
            patch.object(gen_auth_audit, "OUTPUT_PATH", self.output_path),
            patch.object(
                gen_auth_audit,
                "CONTRACTS",
                [("fake_contract", "fake_contract")],
            ),
        ]
        for p in self.patches:
            p.start()
            self.addCleanup(p.stop)

    def test_freshly_generated_doc_has_no_discrepancies(self):
        """A doc generated straight from source must validate clean (the
        property --check relies on to gate CI)."""
        self.output_path.write_text(gen_auth_audit.generate_markdown())
        self.assertEqual(gen_auth_audit.validate_against_source(), [])

    def test_phantom_documented_function_is_detected(self):
        """A documented function that does not exist in source (#356's exact
        failure mode: set_account_principal_delta / get_account_principal_delta)
        must be reported as a discrepancy."""
        doc = gen_auth_audit.generate_markdown()
        doc = doc.replace(
            "| `set_u128` |",
            "| `set_account_principal_delta` |\n| `set_u128` |",
        )
        self.output_path.write_text(doc)

        discrepancies = gen_auth_audit.validate_against_source()
        self.assertTrue(
            any("set_account_principal_delta" in d and "does NOT exist" in d for d in discrepancies),
            f"expected a phantom-function discrepancy, got: {discrepancies}",
        )

    def test_undocumented_real_function_is_detected(self):
        """A real pub fn missing from the doc entirely must be reported —
        the other direction of drift --check guards against."""
        doc = gen_auth_audit.generate_markdown()
        doc = "\n".join(
            line for line in doc.splitlines() if "get_position_manager" not in line
        )
        self.output_path.write_text(doc)

        discrepancies = gen_auth_audit.validate_against_source()
        self.assertTrue(
            any("get_position_manager" in d and "NOT documented" in d for d in discrepancies),
            f"expected a missing-function discrepancy, got: {discrepancies}",
        )

    def test_get_position_manager_classified_as_read_only_not_pass(self):
        """get_position_manager takes no auth-check action on its own body, so
        it must classify as read-only (N/A), never a caller-checked PASS —
        the specific mis-statement #356 flagged."""
        fns = gen_auth_audit.extract_pub_fns(FAKE_SOURCE)
        by_name = {fn.name: fn for fn in fns}
        status, _expected = gen_auth_audit.classify_fn(by_name["get_position_manager"])
        self.assertEqual(status, "➖ N/A")


if __name__ == "__main__":
    unittest.main()
