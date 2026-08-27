# Market Token (GM) Valuation

GM tokens are SO4.market's liquidity-provider (LP) shares. Each GM token represents a proportional claim on the assets backing a specific market pool. This document explains how GM token prices are calculated, what drives them, and how minting and burning are priced.

---

## NAV Formula

The price of a GM token at any moment is the pool's **net asset value (NAV)** divided by the total GM supply:

```
GM price = pool_value / GM supply
```

All values are denominated in USD at FLOAT_PRECISION (10^30). Token amounts use TOKEN_PRECISION (10^7, Stellar's 7-decimal convention).

---

## Pool Value Components

`get_pool_value` in `libs/market_utils/src/lib.rs` sums four components:

```
pool_value = long_token_usd + short_token_usd + impact_pool_usd − net_pnl
```

| Component | Description |
|---|---|
| `long_token_usd` | Long-token pool amount × long-token price |
| `short_token_usd` | Short-token pool amount × short-token price |
| `impact_pool_usd` | Price-impact pool (accrues from impact fees) × index-token price |
| `net_pnl` | Unrealized PnL of all open positions (positive means traders profit, which reduces LP value) |

`net_pnl = long_pnl + short_pnl`, where:

```
long_pnl  = (oi_tokens_long  × index_price / TOKEN_PRECISION) − oi_usd_long
short_pnl = oi_usd_short − (oi_tokens_short × index_price / TOKEN_PRECISION)
```

A positive `long_pnl` (longs in profit) reduces `pool_value`. A positive `short_pnl` (shorts in profit) also reduces `pool_value`. LPs absorb unrealized trader gains.

---

## Minting and Burning Price

`get_market_token_price` in `libs/market_utils/src/lib.rs` computes the per-token price:

```rust
// Returns price in FLOAT_PRECISION units ($1 = FLOAT_PRECISION)
pub fn get_market_token_price(..., maximize: bool) -> i128 {
    let supply = market_token.total_supply();
    if supply <= 0 { return FLOAT_PRECISION; }   // first deposit always at $1

    let info = get_pool_value(..., maximize, ...);
    mul_div_wide(env, info.pool_value, TOKEN_PRECISION, supply)
}
```

The `maximize` parameter is reserved for future min/max price selection (e.g., valuing long tokens at their highest bid for deposits, or their lowest ask for withdrawals). It has no effect in the current implementation — `pool_value` uses the oracle's primary price regardless of `maximize`. This conservative simplification avoids sandwich opportunities during the initial launch phase.

**First deposit:** When GM supply is zero the price is initialised to exactly `FLOAT_PRECISION` ($1) so the first LP sets the baseline NAV.

---

## Worked Example

### Setup

| Parameter | Value |
|---|---|
| Long token | ETH, pool amount = 5 ETH |
| Short token | USDC, pool amount = 10,000 USDC |
| ETH price | $2,000 |
| USDC price | $1 |
| GM supply | 10,000 GM tokens |

Open position: 1 ETH long, entered when ETH was $1,500 (OI in USD = $1,500; OI in tokens = 1 ETH).

### Step 1 — Pool token value

```
long_token_usd  = 5 ETH  × $2,000 = $10,000
short_token_usd = 10,000 × $1     = $10,000
impact_pool_usd = $0  (no accumulated price impact)

total backing   = $20,000
```

### Step 2 — Unrealized PnL

```
long_pnl = (1 ETH × $2,000) − $1,500 = $2,000 − $1,500 = +$500
           (longs are $500 in profit)
short_pnl = $0  (no short positions)
net_pnl = +$500
```

### Step 3 — Pool value

```
pool_value = $20,000 − $500 = $19,500
```

The $500 unrealized gain for the long trader reduces the LP pool by the same amount.

### Step 4 — GM price

```
GM price = $19,500 / 10,000 GM = $1.95 per GM
```

This arithmetic matches the `get_market_token_price` formula:
```
price = pool_value × TOKEN_PRECISION / supply
      = ($19,500 × FP) × 10^7 / (10,000 × 10^7)
      = $1.95 × FP        (i.e. $1.95 in FLOAT_PRECISION units)
```

*Arithmetic derived from `get_pool_value` and `get_market_token_price` in `libs/market_utils/src/lib.rs`; verified by inspection against the inline PnL formulas.*

---

## Key Relationships

- **Deposits add liquidity symmetrically**: depositing tokens at fair value mints GM at the current NAV price, leaving existing LPs unaffected.
- **Traders in profit → GM price falls**: open winning positions reduce `pool_value`, lowering the NAV every LP holds.
- **Traders at a loss → GM price rises**: unrealized losses increase `pool_value`, benefiting LPs.
- **Impact pool appreciation**: fees collected into the price-impact pool add to `pool_value` and slowly accrue to LPs.

---

## Further Reading

- `libs/market_utils/src/lib.rs` — `get_pool_value`, `get_market_token_price`
- `docs/price-impact.md` — how price impact fees accumulate in the impact pool
- `docs/borrowing-fees.md` — borrowing fee accrual (simplified in current implementation)
