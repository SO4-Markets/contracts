# CLOB-138: Hot Consensus State vs Indexer-Only Historical Data

## Overview

Trade history, completed-order records, and analytics **must not accumulate
indefinitely in contract storage**. Only state required for future consensus
execution is kept on-chain. Everything else is published through contract events
and delegated to off-chain indexers.

This document records the boundary, the invariants each contract must maintain,
and the data-availability assumptions indexers may rely on.

---

## Hot Consensus State (kept on-chain)

These entries are required for the protocol to execute correctly on future
transactions. They must be persisted and their TTL actively maintained
(see `clob-storage-classification.md` for tier assignments).

| Entry | Contract | Purpose |
|---|---|---|
| Open resting order | `data_store` | Required to match, cancel, or partially fill |
| Trader balance (`available`, `locked`) | `data_store` | Required to validate margin, fill, and withdrawal |
| Active price level / FIFO queue node | `data_store` | Required for order matching |
| Market configuration | `market_factory` | Required for every execution path |
| Protocol fee buckets | `fee_handler` | Required for fee accounting |
| Referrer cumulative volume (`u128`) | `referral_storage` | Required for tier/rebate logic on next trade |
| Market registry index | `market_factory` | Required for routing |

### Invariants

1. **No on-chain array of fills or trades.** A `Vec<Fill>` or `Vec<Trade>` appended
   per execution is unbounded — it must never be stored.
2. **No on-chain array of completed orders.** Once an order is fully filled or
   cancelled, its resting-order entry is removed from storage, not migrated to a
   history array.
3. **Compact counters only.** Per-market or per-referrer running totals (`u128`)
   are permitted when protocol logic depends on them (e.g. `cumulative_volume`
   for referral tiers). They must not grow in proportion to trade count.
4. **Terminal order fields are deleted.** When an order transitions to a terminal
   state (fully filled, cancelled, expired), its `data_store` entry is either
   removed or allowed to archive via TTL non-renewal (Tier 3 — see
   `clob-storage-classification.md`).

---

## Indexer-Only Historical Data (published via events, never stored)

Indexers reconstruct the full trading history from contract events alone.
No historical fact that is indexer-only should ever be written to on-chain
storage.

| Data | Event | Schema reference |
|---|---|---|
| Individual fills (price, qty, fees, trader ids) | `fill` | `clob-fill-event-schema.md` |
| Order opened / amended / cancelled | `order_opened`, `order_amended`, `order_cancelled` | `clob-order-lifecycle-events.md` |
| Completed trade (maker + taker summary) | `trade` | `clob-fill-event-schema.md` |
| OHLCV / market statistics | Derived from `fill` events | Indexer responsibility |
| Maker/taker attribution history | Derived from `fill` events | Indexer responsibility |

### Indexer data-availability assumptions

- Every fill emits a `fill` event in the same transaction that modifies balances.
  An indexer that processes every ledger from market genesis has a complete
  and consistent trade history.
- Event data is sufficient to reconstruct cumulative per-trader volume,
  total market volume, and OHLCV candles at any granularity.
- Indexers must not rely on querying on-chain storage for completed-order
  records — those entries may have been removed or archived.

---

## State-Cleanup Rules

These rules apply when an order transitions to a terminal state:

1. **Fully filled order**: remove the resting-order entry from `data_store`.
   Emit a `fill` event (and a `trade` event if applicable) before removal.
2. **Cancelled order**: remove the resting-order entry. Emit `order_cancelled`.
3. **Expired order**: the `order_cleanup` contract removes the entry and emits
   `order_expired`. The keeper is compensated by the escrowed `execution_fee`.
4. **Zero-balance account**: do not extend TTL. The entry archives via Soroban
   rent non-renewal. No removal call is required; no history entry is written.

Cleanup **must not** break cancellation or query of live orders.
Specifically, `order_cleanup` batch operations must filter out live orders
before removal, and must never remove an order whose TTL has not expired and
whose status is not terminal.

---

## References

- `clob-storage-classification.md` — 3-Tier TTL strategy
- `clob-fill-event-schema.md` — Fill event field definitions (issue #1021)
- `clob-order-lifecycle-events.md` — Order lifecycle event sequence
- `clob-batch-event-optimization.md` — Batch event cost analysis
- `ttl-strategy.md` — TTL extension thresholds and renewal rules
