# Liquidation Flow

## Overview

A position becomes eligible for liquidation when its health factor drops below 1.0 — meaning the remaining collateral no longer covers the outstanding notional multiplied by the configured minimum reserve factor. Three parties interact during liquidation: the liquidation keeper (executor), the insurance fund, and the position owner.

---

## 1. Eligibility Formula

A position is liquidatable when:

```
collateral_usd < size_in_usd × min_collateral_factor
```

Where:
- `collateral_usd` — current mark-to-market value of the position's collateral, in USD at `FLOAT_PRECISION`
- `size_in_usd` — total notional size of the position
- `min_collateral_factor` — per-market configuration stored under `min_collateral_factor_key(market)` in `data_store`

The `is_liquidatable` helper in `libs/position_utils` encodes this check and is the sole gate called by `LiquidationHandler::check_liquidatable`. If the check passes (position is healthy) the liquidation reverts with `PositionNotLiquidatable`.

### Why the factor matters

`min_collateral_factor` is typically 1% (`FLOAT_PRECISION / 100`). A $10,000 notional position needs at least $100 of collateral. As unrealised PnL moves against the position, available collateral erodes — once it falls below this threshold the position can be forcibly closed to prevent bad debt from accumulating in the pool.

---

## 2. Liquidation Trigger

Any account holding the `LIQUIDATION_KEEPER` role may call:

```
LiquidationHandler::liquidate_position(
    caller,
    account,
    market,
    collateral_token,
    is_long,
    index_token_price,
    long_token_price,
    short_token_price,
)
```

The contract executes the following steps:

1. Verifies `caller` holds the `LIQUIDATION_KEEPER` role (via `role_store.has_role`).
2. Calls `is_liquidatable` with the supplied prices — reverts if the position is still healthy.
3. Delegates to `order_handler.liquidate_position` to execute the close and distribute remaining collateral.
4. Emits a `liq_done` event carrying the position key and keeper address.

---

## 3. Collateral Split

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
- `min_collateral_factor` = 1%
- Liquidation fee factor = 5% of gross collateral
- Keeper fee factor = 20% of liquidation fee

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

**Collateral distribution:**
```
gross_collateral = $90.00
liquidation_fee  = $90.00 × 5%  = $4.50   → insurance fund
keeper_fee       = $4.50  × 20% = $0.90   → keeper wallet
remainder        = $90.00 - $4.50 = $85.50 → position owner
```

---

## 5. Comparison: Liquidation vs. ADL

| | Liquidation | Auto-Deleveraging (ADL) |
|---|---|---|
| **Trigger** | Individual position health factor < 1 | Pool-level profit-to-reserve ratio exceeds `adl_threshold_bps` |
| **Executor role** | `LIQUIDATION_KEEPER` | `ADL_KEEPER` |
| **Position selection** | Any single position below the health threshold | Highest-profit positions first (most impact on pool) |
| **Fee charged** | Keeper fee + insurance fee | None |
| **Outcome** | Position fully closed | Position partially or fully reduced |
| **Primary purpose** | Prevent bad debt on under-collateralised positions | Rebalance pool PnL when profitable OI grows too large |

ADL targets profitable positions — unlike liquidation, it is triggered at the market level when the pool's ability to pay out all winners is at risk. It does not charge a keeper or insurance fee; the reduction in size is the mechanism.
