# Price impact parameters and tuning guide

SO4 uses the same core price-impact shape for swaps and positions:

```text
initial_diff = abs(side_a_usd - side_b_usd)
next_diff    = abs(next_side_a_usd - next_side_b_usd)

if next_diff < initial_diff:
    impact_usd = positive_factor * (initial_diff^exponent - next_diff^exponent)
if next_diff > initial_diff:
    impact_usd = -negative_factor * (next_diff^exponent - initial_diff^exponent)
```

All factors and USD values use `FLOAT_PRECISION = 10^30`. In code, the
division by `FLOAT_PRECISION` is applied after multiplying by the configured
factor. A positive result is a trader rebate; a negative result is a trader
cost.

The current implementation has one exponent curve, not a hard-coded branch
that switches at a fixed trade size. Operators should still think about it in
two regions:

- Near the current balance, small changes behave roughly like a linear marginal
  cost around the current `initial_diff`.
- As a trade becomes large relative to the existing imbalance, the exponent
  dominates. With `position_impact_exponent_factor = 2e30`, impact grows
  quadratically with the imbalance change.

There is no separate crossover constant. The practical crossover is the trade
size at which the exponent term becomes more important than the local linear
approximation for the current market state.

## Parameters in this repository

The keys are generated in `libs/keys/src/lib.rs` and consumed by
`libs/pricing_utils/src/lib.rs`.

| Area | Factor keys | Exponent key | Balance metric |
|---|---|---|---|
| Swaps | `swap_impact_factor_key(market, is_positive)` | `swap_impact_exponent_factor_key(market)` | Pool amount imbalance between input and output token USD values |
| Positions | `position_impact_factor_key(market, is_positive)` | `position_impact_exponent_factor_key(market)` | Long/short open-interest imbalance |

`scripts/configure_market.sh` currently seeds these defaults:

| Parameter | Default | Human meaning |
|---|---:|---|
| `SWAP_IMPACT_POS` | `200000000000000000000000` | `2e23` |
| `SWAP_IMPACT_NEG` | `400000000000000000000000` | `4e23` |
| `SWAP_IMPACT_EXP` | `1000000000000000000000000000000` | `1.0`, linear |
| `POS_IMPACT_POS` | `100000000000000000000000` | `1e23` |
| `POS_IMPACT_NEG` | `200000000000000000000000` | `2e23` |
| `POS_IMPACT_EXP` | `2000000000000000000000000000000` | `2.0`, quadratic |

Use separate positive and negative factors deliberately. A higher negative
factor charges imbalance-worsening trades more than the protocol rebates
balancing trades.

## Impact pool

Negative impact is not paid directly to LPs. It is converted into token units
and added to the relevant impact pool:

- swap impact uses `swap_impact_pool_amount_key(market, token_out)`;
- position impact uses `position_impact_pool_amount_key(market)`.

Positive impact rebates are drawn from the same pool. Rebates are capped by the
available pool value:

```text
positive_impact_usd = min(raw_positive_impact_usd, impact_pool_usd)
```

If the pool is empty, a balancing trade can still compute a positive raw impact
but receives no rebate. The trade may improve market balance without receiving
an impact payment.

## Tuning formula

For a balanced position market and a quadratic exponent, a worsening trade of
size `trade_usd` has:

```text
target_impact_usd = trade_usd * target_impact_bps / 10_000
factor_fraction   = target_impact_usd / trade_usd^2
factor_scaled     = factor_fraction * 1e30
```

Worked example: cap the impact at `0.5%` on a `50,000 USD` trade in a
`1,000,000 USD` market.

```text
target_impact_usd = 50,000 * 50 / 10,000 = 250
factor_fraction   = 250 / 50,000^2
                  = 250 / 2,500,000,000
                  = 0.0000001
factor_scaled     = 0.0000001 * 1e30
                  = 100000000000000000000000
                  = 1e23
```

The market size matters operationally because the trade is `5%` of a
`1,000,000 USD` market, but the current formula uses the long/short OI
imbalance directly. It does not normalize by total market liquidity.

## Suggested starting values for ETH/USD

For a new ETH/USD position market targeting about `0.5%` negative impact on a
`50,000 USD` imbalance-worsening trade:

| Parameter | Suggested value | Reason |
|---|---:|---|
| `position_impact_exponent_factor` | `2000000000000000000000000000000` | Quadratic curve. Large imbalance changes become progressively more expensive. |
| `position_impact_factor` negative | `100000000000000000000000` | Derived above for `0.5%` on `50,000 USD`. |
| `position_impact_factor` positive | `50000000000000000000000` | Starts rebates at half the negative charge so the pool can accumulate before paying large rebates. |

The repository's current default negative position factor is `2e23`, which
would target about `1.0%` on the same balanced-market example. That is more
protective for LPs but more expensive for traders.

For swaps, the seeded exponent is linear. In a linear curve:

```text
factor_scaled = target_impact_usd * 1e30 / trade_usd
```

The default swap values use a larger negative factor than positive factor for
the same reason: worsening pool balance should fund the impact pool faster than
balancing trades drain it.

## Edge cases

Very small or low-liquidity markets can produce high impact for trades that
look small in absolute USD terms. Since the formula does not divide by total
liquidity, operators should tune factors against realistic OI and pool-size
scenarios before opening a market.

An empty impact pool means positive impact is capped to zero. A trader who
improves OI balance may receive no rebate until previous negative-impact trades
have funded the pool.

Large quadratic inputs can approach arithmetic limits faster than linear inputs.
When tuning with `exponent = 2e30`, run representative values through the unit
tests or a simulation before deploying parameters.

## Code references

- `compute_impact_usd` in `libs/pricing_utils/src/lib.rs` applies the signed
  factor/exponent formula.
- `get_swap_price_impact` compares token pool USD balances and caps positive
  impact by the swap impact pool.
- `get_position_price_impact` compares long/short OI and caps positive impact
  by the position impact pool.
- `apply_swap_impact_value` and `apply_position_impact_value` convert the USD
  impact into token units and update the impact pool.
