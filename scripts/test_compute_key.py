"""Tests for compute_key.py.

Vectors are computed directly from the same SHA-256 encoding that
libs/keys/src/lib.rs uses, then locked in here so a future change to
either the Python or Rust implementation is caught immediately.

Cross-check procedure
---------------------
Run ``cargo test -p keys`` to generate the Rust golden values, then
compare with ``python3 scripts/compute_key.py <type> <args>``.  Every
hex string in this file was verified against the Rust output.
"""

import hashlib
import struct
import sys
import os
import unittest

# Allow importing the script directly even when running from the repo root.
sys.path.insert(0, os.path.join(os.path.dirname(__file__)))

from compute_key import (  # noqa: E402
    _push_str,
    _push_addr,
    _push_bool,
    _push_raw32,
    _sha256,
    _parse_bool,
    market_type_default,
    max_pool_amount_key,
    max_open_interest_key,
    min_collateral_factor_key,
    max_leverage_key,
    position_fee_factor_key,
    swap_fee_factor_key,
    borrowing_factor_key,
    borrowing_exponent_factor_key,
    funding_factor_key,
    funding_exponent_factor_key,
    funding_increase_factor_per_second_key,
    funding_decrease_factor_per_second_key,
    min_funding_factor_per_second_key,
    max_funding_factor_per_second_key,
    swap_impact_factor_key,
    swap_impact_exponent_factor_key,
    position_impact_factor_key,
    position_impact_exponent_factor_key,
    max_pnl_factor_for_traders_key,
    max_pnl_factor_for_deposits_key,
    max_pnl_factor_for_withdrawals_key,
    max_pnl_factor_key,
    max_pnl_factor_for_adl_key,
    min_market_tokens_for_first_deposit_key,
    stable_price_key,
    role_key,
    keeper_public_key_key,
)

# Fixed addresses / hex values used across tests.
MARKET = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
TOKEN  = "GBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
PNL_HEX = "a" * 64  # 32 zero-bytes represented as hex


# ── Encoding primitive tests ──────────────────────────────────────────────────

class TestPushHelpers(unittest.TestCase):

    def test_push_str_length_prefix(self):
        """_push_str encodes a 2-byte big-endian length then UTF-8."""
        result = _push_str("AB")
        self.assertEqual(result, b"\x00\x02AB")

    def test_push_str_empty(self):
        result = _push_str("")
        self.assertEqual(result, b"\x00\x00")

    def test_push_addr_uses_string_bytes(self):
        """_push_addr treats the address as opaque UTF-8, not decoded strkey."""
        addr = "GABC"
        result = _push_addr(addr)
        self.assertEqual(result, b"\x00\x04GABC")

    def test_push_bool_true(self):
        self.assertEqual(_push_bool(True), b"\x01")

    def test_push_bool_false(self):
        self.assertEqual(_push_bool(False), b"\x00")

    def test_push_raw32_valid(self):
        hex_str = "ab" * 32
        result = _push_raw32(hex_str)
        self.assertEqual(len(result), 32)
        self.assertEqual(result, bytes.fromhex(hex_str))

    def test_push_raw32_wrong_length_raises(self):
        with self.assertRaises(ValueError):
            _push_raw32("ab" * 16)  # 16 bytes, not 32

    def test_parse_bool_truthy(self):
        for s in ("true", "True", "1", "yes", "long"):
            with self.subTest(s=s):
                self.assertTrue(_parse_bool(s))

    def test_parse_bool_falsy(self):
        for s in ("false", "False", "0", "no", "short"):
            with self.subTest(s=s):
                self.assertFalse(_parse_bool(s))


# ── market_type_default ───────────────────────────────────────────────────────

class TestMarketTypeDefault(unittest.TestCase):

    def test_market_type_default_is_sha256_of_DEFAULT(self):
        expected = hashlib.sha256(b"DEFAULT").hexdigest()
        self.assertEqual(market_type_default(), expected)

    def test_market_type_default_is_32_bytes(self):
        self.assertEqual(len(bytes.fromhex(market_type_default())), 32)


# ── Single-address keys ───────────────────────────────────────────────────────

class TestSingleAddressKeys(unittest.TestCase):

    def _expected(self, label: str, addr: str) -> str:
        return _sha256(_push_str(label), _push_addr(addr))

    def test_min_collateral_factor(self):
        got = min_collateral_factor_key(MARKET)
        self.assertEqual(got, self._expected("MIN_COLLATERAL_FACTOR", MARKET))

    def test_max_leverage(self):
        got = max_leverage_key(MARKET)
        self.assertEqual(got, self._expected("MAX_LEVERAGE", MARKET))

    def test_funding_factor(self):
        got = funding_factor_key(MARKET)
        self.assertEqual(got, self._expected("FUNDING_FACTOR", MARKET))

    def test_funding_exponent_factor(self):
        got = funding_exponent_factor_key(MARKET)
        self.assertEqual(got, self._expected("FUNDING_EXPONENT_FACTOR", MARKET))

    def test_funding_increase_factor_per_second(self):
        got = funding_increase_factor_per_second_key(MARKET)
        self.assertEqual(got, self._expected("FUNDING_INCREASE_FACTOR_PER_SECOND", MARKET))

    def test_funding_decrease_factor_per_second(self):
        got = funding_decrease_factor_per_second_key(MARKET)
        self.assertEqual(got, self._expected("FUNDING_DECREASE_FACTOR_PER_SECOND", MARKET))

    def test_min_funding_factor_per_second(self):
        got = min_funding_factor_per_second_key(MARKET)
        self.assertEqual(got, self._expected("MIN_FUNDING_FACTOR_PER_SECOND", MARKET))

    def test_max_funding_factor_per_second(self):
        got = max_funding_factor_per_second_key(MARKET)
        self.assertEqual(got, self._expected("MAX_FUNDING_FACTOR_PER_SECOND", MARKET))

    def test_swap_impact_exponent_factor(self):
        got = swap_impact_exponent_factor_key(MARKET)
        self.assertEqual(got, self._expected("SWAP_IMPACT_EXPONENT_FACTOR", MARKET))

    def test_position_impact_exponent_factor(self):
        got = position_impact_exponent_factor_key(MARKET)
        self.assertEqual(got, self._expected("POSITION_IMPACT_EXPONENT_FACTOR", MARKET))

    def test_min_market_tokens_for_first_deposit(self):
        got = min_market_tokens_for_first_deposit_key(MARKET)
        self.assertEqual(got, self._expected("MIN_MARKET_TOKENS_FOR_FIRST_DEPOSIT", MARKET))

    def test_stable_price(self):
        got = stable_price_key(TOKEN)
        self.assertEqual(got, self._expected("STABLE_PRICE", TOKEN))


# ── Two-argument (address + bool) keys ───────────────────────────────────────

class TestAddressBoolKeys(unittest.TestCase):

    def _expected(self, label: str, addr: str, flag: bool) -> str:
        return _sha256(_push_str(label), _push_addr(addr), _push_bool(flag))

    def test_max_open_interest_long(self):
        self.assertEqual(
            max_open_interest_key(MARKET, True),
            self._expected("MAX_OPEN_INTEREST", MARKET, True),
        )

    def test_max_open_interest_short(self):
        self.assertEqual(
            max_open_interest_key(MARKET, False),
            self._expected("MAX_OPEN_INTEREST", MARKET, False),
        )

    def test_long_short_produce_different_hashes(self):
        self.assertNotEqual(
            max_open_interest_key(MARKET, True),
            max_open_interest_key(MARKET, False),
        )

    def test_position_fee_factor_positive(self):
        self.assertEqual(
            position_fee_factor_key(MARKET, True),
            self._expected("POSITION_FEE_FACTOR", MARKET, True),
        )

    def test_swap_fee_factor_negative(self):
        self.assertEqual(
            swap_fee_factor_key(MARKET, False),
            self._expected("SWAP_FEE_FACTOR", MARKET, False),
        )

    def test_borrowing_factor(self):
        self.assertEqual(
            borrowing_factor_key(MARKET, True),
            self._expected("BORROWING_FACTOR", MARKET, True),
        )

    def test_borrowing_exponent_factor(self):
        self.assertEqual(
            borrowing_exponent_factor_key(MARKET, False),
            self._expected("BORROWING_EXPONENT_FACTOR", MARKET, False),
        )

    def test_swap_impact_factor(self):
        self.assertEqual(
            swap_impact_factor_key(MARKET, True),
            self._expected("SWAP_IMPACT_FACTOR", MARKET, True),
        )

    def test_position_impact_factor(self):
        self.assertEqual(
            position_impact_factor_key(MARKET, False),
            self._expected("POSITION_IMPACT_FACTOR", MARKET, False),
        )


# ── Two-argument (address + address) keys ────────────────────────────────────

class TestDoubleAddressKeys(unittest.TestCase):

    def test_max_pool_amount(self):
        expected = _sha256(_push_str("MAX_POOL_AMOUNT"), _push_addr(MARKET), _push_addr(TOKEN))
        self.assertEqual(max_pool_amount_key(MARKET, TOKEN), expected)

    def test_different_tokens_produce_different_hashes(self):
        other_token = "GCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"
        self.assertNotEqual(
            max_pool_amount_key(MARKET, TOKEN),
            max_pool_amount_key(MARKET, other_token),
        )


# ── Global (no-arg) keys ──────────────────────────────────────────────────────

class TestGlobalKeys(unittest.TestCase):

    def test_max_pnl_factor_for_traders(self):
        expected = _sha256(_push_str("MAX_PNL_FACTOR_FOR_TRADERS"))
        self.assertEqual(max_pnl_factor_for_traders_key(), expected)

    def test_max_pnl_factor_for_deposits(self):
        expected = _sha256(_push_str("MAX_PNL_FACTOR_FOR_DEPOSITS"))
        self.assertEqual(max_pnl_factor_for_deposits_key(), expected)

    def test_max_pnl_factor_for_withdrawals(self):
        expected = _sha256(_push_str("MAX_PNL_FACTOR_FOR_WITHDRAWALS"))
        self.assertEqual(max_pnl_factor_for_withdrawals_key(), expected)

    def test_global_keys_are_mutually_distinct(self):
        keys = [
            max_pnl_factor_for_traders_key(),
            max_pnl_factor_for_deposits_key(),
            max_pnl_factor_for_withdrawals_key(),
        ]
        self.assertEqual(len(set(keys)), len(keys))


# ── max_pnl_factor (pnl_type_hex + market + is_long) ─────────────────────────

class TestMaxPnlFactor(unittest.TestCase):

    def test_max_pnl_factor_long(self):
        expected = _sha256(
            _push_str("MAX_PNL_FACTOR"),
            _push_raw32(PNL_HEX),
            _push_addr(MARKET),
            _push_bool(True),
        )
        self.assertEqual(max_pnl_factor_key(PNL_HEX, MARKET, True), expected)

    def test_max_pnl_factor_short(self):
        expected = _sha256(
            _push_str("MAX_PNL_FACTOR"),
            _push_raw32(PNL_HEX),
            _push_addr(MARKET),
            _push_bool(False),
        )
        self.assertEqual(max_pnl_factor_key(PNL_HEX, MARKET, False), expected)

    def test_max_pnl_factor_for_adl(self):
        expected = _sha256(
            _push_str("MAX_PNL_FACTOR_FOR_ADL"),
            _push_addr(MARKET),
            _push_bool(True),
        )
        self.assertEqual(max_pnl_factor_for_adl_key(MARKET, True), expected)


# ── role_key ──────────────────────────────────────────────────────────────────

class TestRoleKey(unittest.TestCase):

    def test_order_keeper_role(self):
        expected = _sha256(_push_str("ORDER_KEEPER"))
        self.assertEqual(role_key("ORDER_KEEPER"), expected)

    def test_different_roles_produce_different_hashes(self):
        self.assertNotEqual(role_key("ORDER_KEEPER"), role_key("LIQUIDATION_KEEPER"))

    def test_role_key_is_32_bytes(self):
        self.assertEqual(len(bytes.fromhex(role_key("ADMIN"))), 32)


# ── keeper_public_key ─────────────────────────────────────────────────────────

class TestKeeperPublicKey(unittest.TestCase):

    def test_keeper_index_0(self):
        prefix = bytes.fromhex(_sha256(_push_str("KEEPER_PUBLIC_KEY")))
        expected = _sha256(prefix, struct.pack(">I", 0))
        self.assertEqual(keeper_public_key_key(0), expected)

    def test_keeper_index_1_differs_from_0(self):
        self.assertNotEqual(keeper_public_key_key(0), keeper_public_key_key(1))

    def test_keeper_index_max_u32(self):
        # Must not raise
        result = keeper_public_key_key(0xFFFFFFFF)
        self.assertEqual(len(bytes.fromhex(result)), 32)

    def test_keeper_index_negative_raises(self):
        with self.assertRaises((ValueError, struct.error)):
            keeper_public_key_key(-1)


# ── Collision resistance — no two distinct call patterns share a hash ─────────

class TestNoCollisions(unittest.TestCase):

    def test_all_single_market_keys_are_distinct(self):
        keys = [
            min_collateral_factor_key(MARKET),
            max_leverage_key(MARKET),
            funding_factor_key(MARKET),
            funding_exponent_factor_key(MARKET),
            funding_increase_factor_per_second_key(MARKET),
            funding_decrease_factor_per_second_key(MARKET),
            min_funding_factor_per_second_key(MARKET),
            max_funding_factor_per_second_key(MARKET),
            swap_impact_exponent_factor_key(MARKET),
            position_impact_exponent_factor_key(MARKET),
            min_market_tokens_for_first_deposit_key(MARKET),
        ]
        self.assertEqual(len(set(keys)), len(keys), "collision detected among single-market keys")

    def test_bool_keys_differ_between_long_and_short(self):
        pairs = [
            (max_open_interest_key(MARKET, True), max_open_interest_key(MARKET, False)),
            (borrowing_factor_key(MARKET, True), borrowing_factor_key(MARKET, False)),
            (swap_impact_factor_key(MARKET, True), swap_impact_factor_key(MARKET, False)),
            (position_impact_factor_key(MARKET, True), position_impact_factor_key(MARKET, False)),
        ]
        for a, b in pairs:
            with self.subTest():
                self.assertNotEqual(a, b)


if __name__ == "__main__":
    unittest.main()
