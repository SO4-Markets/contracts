# CLOB market configuration validation

**Issue:** #900. **Code:** `libs/clob_settlement_core/src/market_config.rs`.

Market creation and every permitted configuration update run the same check,
`MarketConfig::validate`. An update runs `MarketConfig::validate_update(current, next)`. That
first rejects any change to an immutable field, then calls `validate` on `next`. An update
therefore cannot produce a configuration that creation would reject.

## Fields

| Field | Mutable | Rule | Error |
|-------|---------|------|-------|
| `base`, `quote` | No | Distinct token contracts | `IdenticalTokens` |
| `lot_size` | No | `> 0` | `LotSizeZero` |
| `tick_size` | No | `> 0` | `TickSizeZero` |
| `max_tick` | No | `> 0`, and `max_tick * tick_size` fits `u64` | `MaxTickZero`, `TickRangeOverflow` |
| `fees.taker_fee_rate` | No | `0..=MAX_FEE_RATE` | `TakerFeeOutOfRange` |
| `fees.maker_fee_rate` | No | `0..=MAX_FEE_RATE` | `MakerFeeOutOfRange` |
| `min_order_lots` | Yes | `> 0`; `min_order_lots * lot_size` fits `u64`; the minimum order at tick 1 credits the seller at least one quote atom | `MinOrderLotsZero`, `MinOrderSizeOverflow`, `MinOrderBelowQuoteGranularity` |
| `max_open_orders_per_trader` | Yes | `1..=MAX_OPEN_ORDERS_PER_TRADER_LIMIT` (1,024, provisional until benchmarked) | `MaxOpenOrdersZero`, `MaxOpenOrdersAboveLimit` |
| `status` | Yes | `Active`, `CancelOnly`, or `Halted` | None. Every status is valid at creation and after an update. |

Changing an immutable field returns `ImmutableFieldChanged(field)`, which names the field. The
immutable set follows spec section 3 and section 4 of `docs/clob-state-ownership.md`: pair
identity, the price grid, and the fee rates are fixed. Changing any of them means creating a new
market.

The minimum order is stored in lots, so it is always on the lot grid. The quote-granularity rule
rejects a market whose cheapest possible fill (minimum size at tick 1) would round the seller's
credit down to zero (spec section 4.2).

`MarketConfigError` discriminants are stable `u32` values, so a contract can return them as
error codes.
