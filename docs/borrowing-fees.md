# Borrowing Fees — Derivation, Rust Pseudocode, and Worked Example

> **Issue #287.** This document explains the borrowing-fee model used by the SO4
> perpetuals protocol. It targets new contributors and auditors who need to
> verify that the on-chain implementation matches the economic intent.

---

## 1. Concepts

| Term | Definition |
|------|-----------|
| `borrowing_factor` | Per-market, per-side base rate stored in `data_store` under `borrowing_factor_key(market, is_long)`. Units: FLOAT_PRECISION per second per utilisation unit. |
| `borrowing_factor_per_second` | The _instantaneous_ rate at which the `cumulative_borrowing_factor` grows, computed from `borrowing_factor × utilisation^exponent`. |
| `cumulative_borrowing_factor` | A monotonically increasing accumulator stored under `cumulative_borrowing_factor_key(market, is_long)`. Grows each time order execution calls `update_cumulative_borrowing_factor`. Units: FLOAT_PRECISION. |
| `position.borrowing_factor` | The snapshot of `cumulative_borrowing_factor` taken when the position was last touched (opened or increased). |

---

## 2. How `cumulative_borrowing_factor` Grows

Source: [`libs/market_utils/src/lib.rs`](../libs/market_utils/src/lib.rs) — `update_cumulative_borrowing_factor`.

```
utilisation      = open_interest / pool_amount               (FLOAT_PRECISION)
util_exp         = utilisation ^ borrowing_exponent_factor   (FLOAT_PRECISION)
delta_per_second = borrowing_factor × util_exp / FLOAT_PRECISION
delta            = delta_per_second × elapsed_seconds / FLOAT_PRECISION

cumulative_borrowing_factor += delta
```

Key properties:
- When `open_interest = 0` → `utilisation = 0` → `delta = 0`. No fee accrues.
- When `open_interest = pool_amount` → `utilisation = FLOAT_PRECISION` → maximum rate.
- The function is a no-op when `elapsed_seconds = 0` (idempotent within a ledger).

---

## 3. How a Position's Borrowing Fee Is Settled

Source: [`libs/position_utils/src/lib.rs`](../libs/position_utils/src/lib.rs) — `get_position_fees`.

```
borrow_delta        = cumulative_borrowing_factor − position.borrowing_factor
                      (clamped to 0 if negative)

borrowing_fee_amount = ceil(borrow_delta × position.size_in_tokens / FLOAT_PRECISION)
```

The fee is expressed in **collateral token units** (not USD). It is settled at close
or decrease time and deducted from the output amount before tokens are returned to the trader.

`position.borrowing_factor` is updated to `cumulative_borrowing_factor` every time the position
is touched, so the next settlement only charges for the _new_ accumulation since the last touch.

---

## 4. Rust Pseudocode

### `update_cumulative_borrowing_factor`

```rust
fn update_cumulative_borrowing_factor(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    is_long: bool,
    current_time: u64,
) {
    let last_updated: u64 = ds.get_u128(updated_at_key(market, is_long)) as u64;
    let dt = current_time.saturating_sub(last_updated);
    if dt == 0 { return; }

    let pool_amount = get_pool_amount(env, ds, market, collateral_token) as i128;
    if pool_amount == 0 {
        ds.set_u128(caller, updated_at_key(market, is_long), current_time as u128);
        return;
    }

    let oi             = get_open_interest_for_side(env, ds, market, is_long) as i128;
    let borrowing_f    = ds.get_u128(borrowing_factor_key(market, is_long)) as i128;
    let exponent       = ds.get_u128(borrowing_exponent_key(market, is_long)) as i128;

    let util           = mul_div_wide(env, oi, FLOAT_PRECISION, pool_amount);
    let util_exp       = pow_factor(env, util, exponent);
    let delta_per_sec  = mul_div_wide(env, borrowing_f, util_exp, FLOAT_PRECISION);
    let delta          = mul_div_wide(env, delta_per_sec, dt as i128, FLOAT_PRECISION);

    ds.apply_delta_to_u128(caller, cumulative_borrowing_factor_key(market, is_long), delta);
    ds.set_u128(caller, updated_at_key(market, is_long), current_time as u128);
}
```

### `get_borrowing_fees_for_position`

```rust
fn get_borrowing_fee_amount(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    position: &PositionProps,
) -> i128 {
    let cum = ds.get_u128(cumulative_borrowing_factor_key(market, position.is_long)) as i128;
    let borrow_delta = (cum - position.borrowing_factor).max(0);
    // Round up: protocol never under-collects
    mul_div_wide_up(env, borrow_delta, position.size_in_tokens, FLOAT_PRECISION)
}
```

---

## 5. Worked Example

**Market setup:**
- Index token: ETH/USD
- Pool collateral: 10 000 USD of long-side tokens (pool_amount = 10 000 × TOKEN_PRECISION)
- Long open interest: 5 000 USD → utilisation = 0.50 (50 %)
- `borrowing_factor` = 1 × 10⁻⁷ per second (≈ 0.00001 % / second)
- `borrowing_exponent_factor` = FLOAT_PRECISION (exponent = 1 → linear)
- FLOAT_PRECISION = 10³⁰, TOKEN_PRECISION = 10⁷

**Step 1 — delta per second:**
```
util           = 5_000 / 10_000 = 0.5  →  0.5 × FLOAT_PRECISION  (in fixed-point)
util_exp       = util^1 = 0.5 × FLOAT_PRECISION
delta_per_sec  = borrowing_factor × util_exp / FLOAT_PRECISION
               = 1e-7 × 0.5 = 5e-8  (per second, FLOAT_PRECISION units)
```

**Step 2 — 1 000 ledgers elapsed at ~5 s/ledger = 5 000 seconds:**
```
delta = delta_per_sec × 5_000 = 5e-8 × 5_000 = 2.5e-4
cumulative_borrowing_factor += 2.5e-4 × FLOAT_PRECISION
```

**Step 3 — Position: 1 000 USD long, opened when cumulative = 0:**
```
position.size_in_tokens = 1_000 / index_price_per_token
                        (assume ETH = $2 000 → 0.5 ETH = 5e6 token units at TOKEN_PRECISION)
borrow_delta   = cumulative − 0 = 2.5e-4 × FLOAT_PRECISION
fee_amount     = ceil(2.5e-4 × FLOAT_PRECISION × size_in_tokens / FLOAT_PRECISION)
               = ceil(2.5e-4 × 5_000_000)
               = ceil(1_250)
               = 1 250 token units (≈ $0.000175 USD at TOKEN_PRECISION)
```

At TOKEN_PRECISION = 10⁷ that is 0.000 125 0 ETH ≈ $0.25 USD at $2 000/ETH.

---

## 6. Edge Cases

### OI = 0 (no borrowing fee)
`util = 0 → delta_per_sec = 0 → cumulative does not grow`. Positions opened or held
when OI is zero accumulate zero borrowing fee regardless of how long they are held.

### OI = pool (100 % utilisation, maximum rate)
`util = FLOAT_PRECISION → util_exp = FLOAT_PRECISION` (for exponent = 1). The rate equals
`borrowing_factor × dt` — the uncapped maximum. At higher exponents the fee accelerates
super-linearly as utilisation approaches 100 %.

### Partial position decrease
`update_cumulative_borrowing_factor` is called at the start of every order execution, so
the accumulator is current before fees are computed. `get_position_fees` receives the
`size_delta_usd` for the decrease; the borrowing fee is proportional to the _closing_ slice:

```
borrow_delta  = cumulative − position.borrowing_factor
fee_for_delta = ceil(borrow_delta × (size_in_tokens × size_delta_usd / size_in_usd)
                     / FLOAT_PRECISION)
```

After settlement, `position.borrowing_factor` is reset to `cumulative` so subsequent
decreases only charge for new accumulation. The remaining open portion continues to
accrue at the same rate.

---

## 7. Cross-References

| File | Relevant lines |
|------|---------------|
| `libs/market_utils/src/lib.rs` | `update_cumulative_borrowing_factor` |
| `libs/position_utils/src/lib.rs` | `get_position_fees` — borrowing fee calculation |
| `libs/decrease_position_utils/src/lib.rs` | calls `get_position_fees`, settles fees into pool |
| `libs/keys/src/lib.rs` | `cumulative_borrowing_factor_key`, `borrowing_factor_key`, `borrowing_exponent_factor_key` |
