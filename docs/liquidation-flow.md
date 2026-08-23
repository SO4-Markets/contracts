# Liquidation Flow

## Overview

A position becomes eligible for liquidation when its health factor drops below 1.0 — meaning the remaining collateral no longer covers the outstanding notional multiplied by the configured minimum reserve factor. Three parties interact during liquidation: the liquidation keeper (executor), the insurance fund, and the position owner.

---

## 1. Eligibility Formula

A position is liquidatable when (in the normal, configured case):

```
net_collateral_usd + pnl_usd < size_in_usd × min_collateral_factor
```

where `net_collateral_usd = collateral_usd - fees_usd` — current fees (borrowing +
funding + position fee, computed worst-case) are subtracted from collateral, and
unrealised PnL (`pnl_usd`) is folded into `remaining` (`remaining = net_collateral + pnl_usd`)
before the comparison.

Where:
- `collateral_usd` — current mark-to-market value of the position's collateral, in USD at `FLOAT_PRECISION`
- `fees_usd` — all currently-accrued fees on the position, in USD
- `pnl_usd` — unrealised profit/loss of the position, in USD
- `size_in_usd` — total notional size of the position
- `min_collateral_factor` — per-market configuration stored under `min_collateral_factor_key(market)` in `data_store`

If `min_collateral_factor` is unset (0, the fallback case), the check evaluates
`remaining < 0` (`net_collateral_usd + pnl_usd < 0`). Both primary and fallback
branches evaluate `remaining` (including PnL).

The `is_liquidatable` helper in `libs/position_utils` encodes this check and is the sole gate called by `LiquidationHandler::check_liquidatable`. If the check passes (position is healthy) the liquidation reverts with `NotLiquidatable`.

### Why the factor matters

`min_collateral_factor` is typically 1% (`FLOAT_PRECISION / 100`). A $10,000 notional position needs at least $100 of net collateral (after fees and PnL). As fees accrue or as unrealised PnL moves against the position, available collateral erodes — once it falls below this threshold the position can be forcibly closed to prevent bad debt from accumulating in the pool.

---

## 2. Liquidation Trigger

Any account holding the `LIQUIDATION_KEEPER` role may call:

```
LiquidationHandler::liquidate_position(
    keeper,
    account,
    market,
    collateral_token,
    is_long,
)
```

The function takes no price parameters — it fetches the current index price and
collateral price itself from the oracle internally before checking liquidatability.

The contract executes the following steps:

1. Verifies `keeper` holds the `LIQUIDATION_KEEPER` role (via `role_store.has_role`).
2. Fetches current prices from the oracle and calls `is_liquidatable` — reverts if the position is still healthy.
3. Delegates to `order_handler.liquidate_position` to execute the close and distribute remaining collateral.
4. Emits a `liq_done` event carrying the keeper, account, market, side, execution price, keeper execution fee, and realised PnL (issue #437).

---

## 3. Collateral Split

> **Not yet implemented (tracked in issue #213).** The percentage-based split
> described below — `liquidation_fee_factor_key`, `max_liquidation_fee_factor_key`,
> `liquidation_keeper_fee_factor_key`, and the insurance fund address — does not
> exist in `order_handler::liquidate_position` today. The only fee actually
> charged is the flat `liquidation_execution_fee_key` amount (issue #416),
> deducted from the position's collateral and paid entirely to the keeper; the
> full remainder goes to the position owner. A separate `insurance_fund_router`
> contract exists with the primitives this split would need
> (`route_liquidation_penalty`, `cover_shortfall`), but its own `INTEGRATION.md`
> documents wiring it into liquidation as distinct future work, not yet done.

After the position is closed, remaining gross collateral is distributed in priority order:

```
gross_collateral  (closing collateral after PnL settlement)
        │
        ├─── keeper_fee  ──────────────────► liquidation keeper (caller)
        │
        ├─── liquidation_fee  ─────────────► insurance fund address
        │
        └─── remainder
                 │
                 ├─ remainder > 0  ─────────► position owner
                 └─ remainder ≤ 0  ─────────► pool absorbs the shortfall
```

If `gross_collateral < keeper_fee + liquidation_fee` the fees are capped at the available balance and the position owner receives nothing.

### Fee parameters

| Parameter | Storage key | Description |
|-----------|-------------|-------------|
| Liquidation fee | `liquidation_fee_factor_key(market)` | Fraction of gross collateral retained by the insurance fund (`FLOAT_PRECISION`) |
| Max liquidation fee | `max_liquidation_fee_factor_key(market)` | USD ceiling on the insurance fund portion |
| Keeper fee factor | `liquidation_keeper_fee_factor_key(market)` | Fraction of the insurance fee forwarded to the executing keeper |

---

## 4. Worked Example

**Setup:**
- Position size: $10,000 notional
- Collateral: 2 tokens of long_token @ $100 each = $200
- `min_collateral_factor` = 1% ($100 required)
- `liquidation_execution_fee_key` = $10.00 (flat keeper execution fee)

**Health check before price move:**
```
required_collateral = $10,000 × 0.01 = $100
collateral_usd      = $200
$200 ≥ $100  →  position is healthy, cannot be liquidated
```

**After price drops to $45/token:**
```
collateral_usd      = 2 × $45 = $90
required_collateral = $10,000 × 0.01 = $100
$90 < $100  →  position is liquidatable
```

**Collateral distribution (current implementation):**
```
gross_collateral = $90.00
keeper_fee       = min($10.00, $90.00) = $10.00 → keeper wallet
remainder        = $90.00 - $10.00 = $80.00     → position owner
```

---

## 5. Comparison: Liquidation vs. ADL

| | Liquidation | Auto-Deleveraging (ADL) |
|---|---|---|
| **Trigger** | Individual position health factor < 1 | `total trader PnL / pool_value` (FLOAT_PRECISION-scaled ratio) exceeds the per-market-per-side threshold stored under `max_pnl_factor_for_adl_key(market, is_long)` |
| **Executor role** | `LIQUIDATION_KEEPER` | `ADL_KEEPER` |
| **Position selection** | Any single position below the health threshold | Highest-profit positions first (most impact on pool) |
| **Fee charged** | Flat keeper execution fee (`liquidation_execution_fee_key`) | None |
| **Outcome** | Position fully closed | Position partially or fully reduced |
| **Primary purpose** | Prevent bad debt on under-collateralised positions | Rebalance pool PnL when profitable OI grows too large |

ADL targets profitable positions — unlike liquidation, it is triggered at the market level when the pool's ability to pay out all winners is at risk. It does not charge a keeper or insurance fee; the reduction in size is the mechanism.
