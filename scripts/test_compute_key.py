#!/usr/bin/env python3
"""Cross-check tests for compute_key.py against committed key_vectors.json (#1168).

Run:  python3 scripts/test_compute_key.py -v

The JSON file was seeded from the same encoding that libs/keys/src/lib.rs
uses (SHA-256 of length-prefixed components). A divergence between this file
and compute_key.py means the script will silently write to a key that no
contract reads — the exact failure mode described in issue #1168.

The Rust side is cross-checked via libs/keys/tests/key_vectors.rs, which
reads the same JSON and asserts byte equality inside cargo test.
"""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compute_key as ck  # noqa: E402

_VECTORS_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "key_vectors.json")


def _load_vectors():
    with open(_VECTORS_PATH) as f:
        data = json.load(f)
    return data["_meta"], data["vectors"]


META, VECTORS = _load_vectors()

MARKET  = META["MARKET"]
TOKEN_A = META["TOKEN_A"]
TOKEN_B = META["TOKEN_B"]
PNL_HEX = META["PNL_HEX"]


class KeyVectorTests(unittest.TestCase):
    """Assert that compute_key.py reproduces every committed test vector."""

    def _check(self, key: str, got: str):
        expected = VECTORS[key]
        self.assertEqual(
            got,
            expected,
            f"\nKey '{key}' mismatch:\n  got      {got}\n  expected {expected}\n"
            "Update scripts/key_vectors.json if libs/keys changed intentionally.",
        )

    # ── role keys ──────────────────────────────────────────────────────────────

    def test_role_role_admin(self):
        self._check("role:ROLE_ADMIN", ck.role_key("ROLE_ADMIN"))

    def test_role_controller(self):
        self._check("role:CONTROLLER", ck.role_key("CONTROLLER"))

    def test_role_market_keeper(self):
        self._check("role:MARKET_KEEPER", ck.role_key("MARKET_KEEPER"))

    def test_role_order_keeper(self):
        self._check("role:ORDER_KEEPER", ck.role_key("ORDER_KEEPER"))

    def test_role_liquidation_keeper(self):
        self._check("role:LIQUIDATION_KEEPER", ck.role_key("LIQUIDATION_KEEPER"))

    def test_role_adl_keeper(self):
        self._check("role:ADL_KEEPER", ck.role_key("ADL_KEEPER"))

    def test_role_fee_keeper(self):
        self._check("role:FEE_KEEPER", ck.role_key("FEE_KEEPER"))

    # ── constant keys ─────────────────────────────────────────────────────────

    def test_max_pnl_factor_for_traders(self):
        self._check("max_pnl_factor_for_traders", ck.max_pnl_factor_for_traders_key())

    def test_max_pnl_factor_for_deposits(self):
        self._check("max_pnl_factor_for_deposits", ck.max_pnl_factor_for_deposits_key())

    def test_max_pnl_factor_for_withdrawals(self):
        self._check("max_pnl_factor_for_withdrawals", ck.max_pnl_factor_for_withdrawals_key())

    def test_market_type_default(self):
        self._check("market_type_default", ck.market_type_default())

    # ── pool / market address keys ────────────────────────────────────────────

    def test_max_pool_amount_market_token_a(self):
        self._check("max_pool_amount:market,token_a", ck.max_pool_amount_key(MARKET, TOKEN_A))

    def test_max_pool_amount_market_token_b(self):
        self._check("max_pool_amount:market,token_b", ck.max_pool_amount_key(MARKET, TOKEN_B))

    def test_min_collateral_factor(self):
        self._check("min_collateral_factor:market", ck.min_collateral_factor_key(MARKET))

    def test_max_leverage(self):
        self._check("max_leverage:market", ck.max_leverage_key(MARKET))

    def test_funding_factor(self):
        self._check("funding_factor:market", ck.funding_factor_key(MARKET))

    def test_funding_exponent_factor(self):
        self._check("funding_exponent_factor:market", ck.funding_exponent_factor_key(MARKET))

    def test_funding_increase_factor_per_second(self):
        self._check(
            "funding_increase_factor_per_second:market",
            ck.funding_increase_factor_per_second_key(MARKET),
        )

    def test_funding_decrease_factor_per_second(self):
        self._check(
            "funding_decrease_factor_per_second:market",
            ck.funding_decrease_factor_per_second_key(MARKET),
        )

    def test_min_funding_factor_per_second(self):
        self._check(
            "min_funding_factor_per_second:market",
            ck.min_funding_factor_per_second_key(MARKET),
        )

    def test_max_funding_factor_per_second(self):
        self._check(
            "max_funding_factor_per_second:market",
            ck.max_funding_factor_per_second_key(MARKET),
        )

    def test_swap_impact_exponent_factor(self):
        self._check(
            "swap_impact_exponent_factor:market",
            ck.swap_impact_exponent_factor_key(MARKET),
        )

    def test_position_impact_exponent_factor(self):
        self._check(
            "position_impact_exponent_factor:market",
            ck.position_impact_exponent_factor_key(MARKET),
        )

    def test_min_market_tokens_for_first_deposit(self):
        self._check(
            "min_market_tokens_for_first_deposit:market",
            ck.min_market_tokens_for_first_deposit_key(MARKET),
        )

    def test_stable_price_token_a(self):
        self._check("stable_price:token_a", ck.stable_price_key(TOKEN_A))

    # ── bool-parameterised keys ───────────────────────────────────────────────

    def test_max_open_interest_long(self):
        self._check("max_open_interest:market,long", ck.max_open_interest_key(MARKET, True))

    def test_max_open_interest_short(self):
        self._check("max_open_interest:market,short", ck.max_open_interest_key(MARKET, False))

    def test_borrowing_factor_long(self):
        self._check("borrowing_factor:market,long", ck.borrowing_factor_key(MARKET, True))

    def test_borrowing_factor_short(self):
        self._check("borrowing_factor:market,short", ck.borrowing_factor_key(MARKET, False))

    def test_borrowing_exponent_factor_long(self):
        self._check(
            "borrowing_exponent_factor:market,long",
            ck.borrowing_exponent_factor_key(MARKET, True),
        )

    def test_borrowing_exponent_factor_short(self):
        self._check(
            "borrowing_exponent_factor:market,short",
            ck.borrowing_exponent_factor_key(MARKET, False),
        )

    def test_max_pnl_factor_for_adl_long(self):
        self._check(
            "max_pnl_factor_for_adl:market,long", ck.max_pnl_factor_for_adl_key(MARKET, True)
        )

    def test_max_pnl_factor_for_adl_short(self):
        self._check(
            "max_pnl_factor_for_adl:market,short", ck.max_pnl_factor_for_adl_key(MARKET, False)
        )

    def test_max_pnl_factor_long(self):
        self._check("max_pnl_factor:long", ck.max_pnl_factor_key(PNL_HEX, MARKET, True))

    def test_max_pnl_factor_short(self):
        self._check("max_pnl_factor:short", ck.max_pnl_factor_key(PNL_HEX, MARKET, False))

    def test_position_fee_factor_pos(self):
        self._check("position_fee_factor:market,pos", ck.position_fee_factor_key(MARKET, True))

    def test_position_fee_factor_neg(self):
        self._check("position_fee_factor:market,neg", ck.position_fee_factor_key(MARKET, False))

    def test_swap_fee_factor_pos(self):
        self._check("swap_fee_factor:market,pos", ck.swap_fee_factor_key(MARKET, True))

    def test_swap_fee_factor_neg(self):
        self._check("swap_fee_factor:market,neg", ck.swap_fee_factor_key(MARKET, False))

    def test_swap_impact_factor_pos(self):
        self._check("swap_impact_factor:market,pos", ck.swap_impact_factor_key(MARKET, True))

    def test_swap_impact_factor_neg(self):
        self._check("swap_impact_factor:market,neg", ck.swap_impact_factor_key(MARKET, False))

    def test_position_impact_factor_pos(self):
        self._check(
            "position_impact_factor:market,pos", ck.position_impact_factor_key(MARKET, True)
        )

    def test_position_impact_factor_neg(self):
        self._check(
            "position_impact_factor:market,neg", ck.position_impact_factor_key(MARKET, False)
        )

    # ── keeper public key ─────────────────────────────────────────────────────

    def test_keeper_public_key_index_0(self):
        self._check("keeper_public_key:0", ck.keeper_public_key_key(0))

    def test_keeper_public_key_index_1(self):
        self._check("keeper_public_key:1", ck.keeper_public_key_key(1))

    # ── coverage guard ────────────────────────────────────────────────────────

    def test_all_vectors_are_covered(self):
        """Fail if key_vectors.json has entries not exercised by any test above."""
        tested = {
            "role:ROLE_ADMIN", "role:CONTROLLER", "role:MARKET_KEEPER",
            "role:ORDER_KEEPER", "role:LIQUIDATION_KEEPER", "role:ADL_KEEPER",
            "role:FEE_KEEPER",
            "max_pnl_factor_for_traders", "max_pnl_factor_for_deposits",
            "max_pnl_factor_for_withdrawals", "market_type_default",
            "max_pool_amount:market,token_a", "max_pool_amount:market,token_b",
            "min_collateral_factor:market", "max_leverage:market",
            "funding_factor:market", "funding_exponent_factor:market",
            "funding_increase_factor_per_second:market",
            "funding_decrease_factor_per_second:market",
            "min_funding_factor_per_second:market", "max_funding_factor_per_second:market",
            "swap_impact_exponent_factor:market", "position_impact_exponent_factor:market",
            "min_market_tokens_for_first_deposit:market", "stable_price:token_a",
            "max_open_interest:market,long", "max_open_interest:market,short",
            "borrowing_factor:market,long", "borrowing_factor:market,short",
            "borrowing_exponent_factor:market,long", "borrowing_exponent_factor:market,short",
            "max_pnl_factor_for_adl:market,long", "max_pnl_factor_for_adl:market,short",
            "max_pnl_factor:long", "max_pnl_factor:short",
            "position_fee_factor:market,pos", "position_fee_factor:market,neg",
            "swap_fee_factor:market,pos", "swap_fee_factor:market,neg",
            "swap_impact_factor:market,pos", "swap_impact_factor:market,neg",
            "position_impact_factor:market,pos", "position_impact_factor:market,neg",
            "keeper_public_key:0", "keeper_public_key:1",
        }
        missing = set(VECTORS.keys()) - tested
        self.assertFalse(
            missing,
            f"These vectors in key_vectors.json have no test: {sorted(missing)}\n"
            "Add a test or remove the vector.",
        )


if __name__ == "__main__":
    unittest.main()
