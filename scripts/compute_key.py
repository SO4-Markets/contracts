#!/usr/bin/env python3
"""Compute gmx_keys-style BytesN<32> hashes for SO4.market data_store.

Replicates the length-prefix SHA256 encoding from libs/keys/src/lib.rs.

Usage:
    python3 scripts/compute_key.py <key_type> [args...]

Key types:
    max_pool_amount <market> <token>
    max_open_interest <market> <is_long>
    min_collateral_factor <market>
    max_leverage <market>
    position_fee_factor <market> <for_positive_impact>
    swap_fee_factor <market> <for_positive_impact>
    borrowing_factor <market> <is_long>
    borrowing_exponent_factor <market> <is_long>
    funding_factor <market>
    funding_exponent_factor <market>
    funding_increase_factor_per_second <market>
    funding_decrease_factor_per_second <market>
    min_funding_factor_per_second <market>
    max_funding_factor_per_second <market>
    swap_impact_factor <market> <is_positive>
    swap_impact_exponent_factor <market>
    position_impact_factor <market> <is_positive>
    position_impact_exponent_factor <market>
    max_pnl_factor <pnl_type_hex> <market> <is_long>
    max_pnl_factor_for_traders
    max_pnl_factor_for_deposits
    max_pnl_factor_for_withdrawals
    max_pnl_factor_for_adl <market> <is_long>
    min_market_tokens_for_first_deposit <market>
    stable_price <token>
    market_type_default

Boolean args: true/1/yes/long = True; false/0/no/short = False
"""

import hashlib
import struct
import sys


# ── Encoding helpers ──────────────────────────────────────────────────────────

def _push_str(s: str) -> bytes:
    """2-byte BE length prefix + UTF-8 bytes."""
    b = s.encode("utf-8")
    return struct.pack(">H", len(b)) + b


def _push_addr(addr: str) -> bytes:
    """2-byte BE length prefix + strkey bytes (the G.../C... string)."""
    b = addr.encode("utf-8")
    return struct.pack(">H", len(b)) + b


def _push_bool(v: bool) -> bytes:
    return b"\x01" if v else b"\x00"


def _push_raw32(hex_str: str) -> bytes:
    """Raw 32 bytes from a 64-char hex string — NO length prefix."""
    raw = bytes.fromhex(hex_str)
    if len(raw) != 32:
        raise ValueError(f"expected 32 bytes, got {len(raw)}: {hex_str!r}")
    return raw


def _sha256(*parts: bytes) -> str:
    h = hashlib.sha256()
    for p in parts:
        h.update(p)
    return h.hexdigest()


def _parse_bool(s: str) -> bool:
    return s.lower() in ("true", "1", "yes", "long")


# ── Key functions (mirror libs/keys/src/lib.rs) ───────────────────────────────

def market_type_default() -> str:
    """sha256(b'DEFAULT') — the standard market_type discriminant."""
    return hashlib.sha256(b"DEFAULT").hexdigest()


def max_pool_amount_key(market: str, token: str) -> str:
    return _sha256(_push_str("MAX_POOL_AMOUNT"), _push_addr(market), _push_addr(token))


def max_open_interest_key(market: str, is_long: bool) -> str:
    return _sha256(_push_str("MAX_OPEN_INTEREST"), _push_addr(market), _push_bool(is_long))


def min_collateral_factor_key(market: str) -> str:
    return _sha256(_push_str("MIN_COLLATERAL_FACTOR"), _push_addr(market))


def max_leverage_key(market: str) -> str:
    return _sha256(_push_str("MAX_LEVERAGE"), _push_addr(market))


def position_fee_factor_key(market: str, for_positive_impact: bool) -> str:
    return _sha256(
        _push_str("POSITION_FEE_FACTOR"), _push_addr(market), _push_bool(for_positive_impact)
    )


def swap_fee_factor_key(market: str, for_positive_impact: bool) -> str:
    return _sha256(
        _push_str("SWAP_FEE_FACTOR"), _push_addr(market), _push_bool(for_positive_impact)
    )


def borrowing_factor_key(market: str, is_long: bool) -> str:
    return _sha256(_push_str("BORROWING_FACTOR"), _push_addr(market), _push_bool(is_long))


def borrowing_exponent_factor_key(market: str, is_long: bool) -> str:
    return _sha256(
        _push_str("BORROWING_EXPONENT_FACTOR"), _push_addr(market), _push_bool(is_long)
    )


def funding_factor_key(market: str) -> str:
    return _sha256(_push_str("FUNDING_FACTOR"), _push_addr(market))


def funding_exponent_factor_key(market: str) -> str:
    return _sha256(_push_str("FUNDING_EXPONENT_FACTOR"), _push_addr(market))


def funding_increase_factor_per_second_key(market: str) -> str:
    return _sha256(_push_str("FUNDING_INCREASE_FACTOR_PER_SECOND"), _push_addr(market))


def funding_decrease_factor_per_second_key(market: str) -> str:
    return _sha256(_push_str("FUNDING_DECREASE_FACTOR_PER_SECOND"), _push_addr(market))


def min_funding_factor_per_second_key(market: str) -> str:
    return _sha256(_push_str("MIN_FUNDING_FACTOR_PER_SECOND"), _push_addr(market))


def max_funding_factor_per_second_key(market: str) -> str:
    return _sha256(_push_str("MAX_FUNDING_FACTOR_PER_SECOND"), _push_addr(market))


def swap_impact_factor_key(market: str, is_positive: bool) -> str:
    return _sha256(_push_str("SWAP_IMPACT_FACTOR"), _push_addr(market), _push_bool(is_positive))


def swap_impact_exponent_factor_key(market: str) -> str:
    return _sha256(_push_str("SWAP_IMPACT_EXPONENT_FACTOR"), _push_addr(market))


def position_impact_factor_key(market: str, is_positive: bool) -> str:
    return _sha256(
        _push_str("POSITION_IMPACT_FACTOR"), _push_addr(market), _push_bool(is_positive)
    )


def position_impact_exponent_factor_key(market: str) -> str:
    return _sha256(_push_str("POSITION_IMPACT_EXPONENT_FACTOR"), _push_addr(market))


def max_pnl_factor_for_traders_key() -> str:
    return _sha256(_push_str("MAX_PNL_FACTOR_FOR_TRADERS"))


def max_pnl_factor_for_deposits_key() -> str:
    return _sha256(_push_str("MAX_PNL_FACTOR_FOR_DEPOSITS"))


def max_pnl_factor_for_withdrawals_key() -> str:
    return _sha256(_push_str("MAX_PNL_FACTOR_FOR_WITHDRAWALS"))


def max_pnl_factor_key(pnl_type_hex: str, market: str, is_long: bool) -> str:
    return _sha256(
        _push_str("MAX_PNL_FACTOR"),
        _push_raw32(pnl_type_hex),
        _push_addr(market),
        _push_bool(is_long),
    )


def max_pnl_factor_for_adl_key(market: str, is_long: bool) -> str:
    return _sha256(
        _push_str("MAX_PNL_FACTOR_FOR_ADL"), _push_addr(market), _push_bool(is_long)
    )


def min_market_tokens_for_first_deposit_key(market: str) -> str:
    return _sha256(_push_str("MIN_MARKET_TOKENS_FOR_FIRST_DEPOSIT"), _push_addr(market))


def stable_price_key(token: str) -> str:
    return _sha256(_push_str("STABLE_PRICE"), _push_addr(token))


# ── CLI dispatch ──────────────────────────────────────────────────────────────

_DISPATCH = {
    "market_type_default": (0, lambda args: market_type_default()),
    "max_pool_amount": (2, lambda a: max_pool_amount_key(a[0], a[1])),
    "max_open_interest": (2, lambda a: max_open_interest_key(a[0], _parse_bool(a[1]))),
    "min_collateral_factor": (1, lambda a: min_collateral_factor_key(a[0])),
    "max_leverage": (1, lambda a: max_leverage_key(a[0])),
    "position_fee_factor": (2, lambda a: position_fee_factor_key(a[0], _parse_bool(a[1]))),
    "swap_fee_factor": (2, lambda a: swap_fee_factor_key(a[0], _parse_bool(a[1]))),
    "borrowing_factor": (2, lambda a: borrowing_factor_key(a[0], _parse_bool(a[1]))),
    "borrowing_exponent_factor": (2, lambda a: borrowing_exponent_factor_key(a[0], _parse_bool(a[1]))),
    "funding_factor": (1, lambda a: funding_factor_key(a[0])),
    "funding_exponent_factor": (1, lambda a: funding_exponent_factor_key(a[0])),
    "funding_increase_factor_per_second": (1, lambda a: funding_increase_factor_per_second_key(a[0])),
    "funding_decrease_factor_per_second": (1, lambda a: funding_decrease_factor_per_second_key(a[0])),
    "min_funding_factor_per_second": (1, lambda a: min_funding_factor_per_second_key(a[0])),
    "max_funding_factor_per_second": (1, lambda a: max_funding_factor_per_second_key(a[0])),
    "swap_impact_factor": (2, lambda a: swap_impact_factor_key(a[0], _parse_bool(a[1]))),
    "swap_impact_exponent_factor": (1, lambda a: swap_impact_exponent_factor_key(a[0])),
    "position_impact_factor": (2, lambda a: position_impact_factor_key(a[0], _parse_bool(a[1]))),
    "position_impact_exponent_factor": (1, lambda a: position_impact_exponent_factor_key(a[0])),
    "max_pnl_factor_for_traders": (0, lambda _: max_pnl_factor_for_traders_key()),
    "max_pnl_factor_for_deposits": (0, lambda _: max_pnl_factor_for_deposits_key()),
    "max_pnl_factor_for_withdrawals": (0, lambda _: max_pnl_factor_for_withdrawals_key()),
    "max_pnl_factor": (3, lambda a: max_pnl_factor_key(a[0], a[1], _parse_bool(a[2]))),
    "max_pnl_factor_for_adl": (2, lambda a: max_pnl_factor_for_adl_key(a[0], _parse_bool(a[1]))),
    "min_market_tokens_for_first_deposit": (1, lambda a: min_market_tokens_for_first_deposit_key(a[0])),
    "stable_price": (1, lambda a: stable_price_key(a[0])),
}


def main():
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)

    key_type = sys.argv[1]
    args = sys.argv[2:]

    if key_type not in _DISPATCH:
        print(f"Unknown key type: {key_type!r}", file=sys.stderr)
        print(f"Available: {', '.join(sorted(_DISPATCH))}", file=sys.stderr)
        sys.exit(1)

    expected_args, fn = _DISPATCH[key_type]
    if len(args) != expected_args:
        print(f"{key_type} expects {expected_args} arg(s), got {len(args)}", file=sys.stderr)
        sys.exit(1)

    print(fn(args))


if __name__ == "__main__":
    main()
