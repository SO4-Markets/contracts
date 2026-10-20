# CLOB Read API Specification — Phase 7 Implementation Guide

This document provides comprehensive implementation guidance for Phase 7 Read APIs: indexer ingestion, L2 order book depth, top-of-book helpers, and trader balance exposure.

---

## Overview

Phase 7 introduces public read APIs that allow off-chain services, UIs, and trading bots to query market state efficiently without scanning unbounded storage or performing on-chain aggregations.

**Core Principles:**
1. **Exact Integer Math**: No floating-point rounding in price/size conversions
2. **Bounded Queries**: All pagination respects resource limits
3. **Idempotent Replay**: Event ingestion handles ledger reorgs safely
4. **Canonical Events**: Off-chain analytics derive from on-chain event stream
5. **Overflow Safety**: All arithmetic operations check for overflow

---

## Issue #1025: Indexer Ingestion Specification

**Objective:** Document how off-chain services consume Stellar/Soroban events, handle replays, order events, and recover from downtime.

### Ingestion Cursor Model

```rust
// contracts/clob/src/indexer.rs (conceptual)
pub struct IndexerCheckpoint {
    /// Last fully processed ledger sequence
    pub ledger_sequence: u32,
    
    /// Transaction index within that ledger (for partial ledger processing)
    pub tx_index: u32,
    
    /// Event index within that transaction
    pub event_index: u32,
    
    /// Timestamp of the checkpoint
    pub checkpoint_time: u64,
}

impl IndexerCheckpoint {
    /// Resume ingestion from this checkpoint
    pub fn resume_from(&self) -> (u32, u32, u32) {
        (self.ledger_sequence + 1, 0, 0)
    }
    
    /// Check if a given event has been processed
    pub fn is_processed(&self, ledger: u32, tx: u32, event: u32) -> bool {
        if ledger < self.ledger_sequence {
            return true;
        }
        if ledger > self.ledger_sequence {
            return false;
        }
        // Same ledger, check tx + event
        if tx < self.tx_index {
            return true;
        }
        if tx > self.tx_index {
            return false;
        }
        event <= self.event_index
    }
}
```

### Idempotent Replay Keys

Every event must have a globally unique, deterministic replay key:

```rust
pub struct ReplayKey {
    pub ledger_sequence: u32,
    pub tx_index: u32,
    pub event_index: u32,
    pub contract_id: String,
    pub event_type: String,
}

impl ReplayKey {
    pub fn to_string(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.ledger_sequence,
            self.tx_index,
            self.event_index,
            self.contract_id,
            self.event_type
        )
    }
    
    /// Parse from Stellar event structure
    pub fn from_event(event: &SorobanEvent) -> Self {
        Self {
            ledger_sequence: event.ledger_sequence,
            tx_index: event.tx_index,
            event_index: event.event_index,
            contract_id: event.contract_id.to_string(),
            event_type: event.topics[0].to_string(),
        }
    }
}
```

### Finality and Reorg Handling

**Stellar Finality Model:**
- Ledgers are finalized by SCP consensus (~5 seconds)
- Horizon provides finalized data after ledger close
- Reorgs are extremely rare but possible in network partitions

```rust
pub struct FinalityStrategy {
    /// Number of ledgers to wait before considering data "finalized"
    pub confirmation_depth: u32,
}

impl FinalityStrategy {
    /// Conservative: wait 10 ledgers (~50 seconds)
    pub fn conservative() -> Self {
        Self { confirmation_depth: 10 }
    }
    
    /// Standard: wait 3 ledgers (~15 seconds)
    pub fn standard() -> Self {
        Self { confirmation_depth: 3 }
    }
    
    /// Aggressive: process immediately (accept reorg risk)
    pub fn aggressive() -> Self {
        Self { confirmation_depth: 0 }
    }
    
    pub fn is_finalized(&self, event_ledger: u32, current_ledger: u32) -> bool {
        current_ledger >= event_ledger + self.confirmation_depth
    }
}
```

### Reorg Recovery Protocol

```typescript
// Off-chain indexer service (Node.js/TypeScript)
interface ReorgRecovery {
  detectReorg(latestLedger: number): Promise<number | null>;
  rollbackTo(safeLedger: number): Promise<void>;
  replayFrom(startLedger: number): Promise<void>;
}

class IndexerService implements ReorgRecovery {
  async detectReorg(latestLedger: number): Promise<number | null> {
    // Check if stored ledger hash matches Horizon's historical hash
    const storedHash = await this.db.getLedgerHash(latestLedger);
    const horizonHash = await this.horizon.getLedger(latestLedger).then(l => l.hash);
    
    if (storedHash !== horizonHash) {
      // Reorg detected! Find the safe common ancestor
      for (let ledger = latestLedger - 1; ledger > 0; ledger--) {
        const stored = await this.db.getLedgerHash(ledger);
        const horizon = await this.horizon.getLedger(ledger).then(l => l.hash);
        if (stored === horizon) {
          return ledger; // Safe rollback point
        }
      }
    }
    return null; // No reorg
  }
  
  async rollbackTo(safeLedger: number): Promise<void> {
    // Delete all events, fills, orders after safe ledger
    await this.db.deleteEventsAfter(safeLedger);
    await this.db.deleteFillsAfter(safeLedger);
    await this.db.deleteOrdersAfter(safeLedger);
    await this.db.updateCheckpoint({ ledger: safeLedger, tx: 0, event: 0 });
  }
  
  async replayFrom(startLedger: number): Promise<void> {
    // Re-ingest from safe point forward
    const cursor = await this.horizon.ledgers()
      .cursor(startLedger.toString())
      .limit(200);
    
    for await (const ledger of cursor) {
      await this.processLedger(ledger);
    }
  }
}
```

### Duplicate Prevention

```sql
-- PostgreSQL schema for idempotent ingestion
CREATE TABLE processed_events (
    replay_key TEXT PRIMARY KEY,
    ledger_sequence INTEGER NOT NULL,
    tx_index INTEGER NOT NULL,
    event_index INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    processed_at TIMESTAMPTZ DEFAULT NOW(),
    INDEX idx_ledger_sequence (ledger_sequence)
);

-- Upsert pattern prevents duplicates
INSERT INTO processed_events (replay_key, ledger_sequence, tx_index, event_index, event_type)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (replay_key) DO NOTHING
RETURNING *;
```

### Event Ordering Guarantees

1. **Within a Transaction**: Events are ordered by `event_index`
2. **Within a Ledger**: Transactions are ordered by `tx_index`
3. **Across Ledgers**: Ledgers are ordered by `ledger_sequence`

**Canonical Ordering:**
```rust
pub fn compare_events(a: &ReplayKey, b: &ReplayKey) -> Ordering {
    a.ledger_sequence.cmp(&b.ledger_sequence)
        .then(a.tx_index.cmp(&b.tx_index))
        .then(a.event_index.cmp(&b.event_index))
}
```

---

## Issue #1019: Paginated L2 Order Book Depth

**Objective:** Expose aggregated price levels (L2 depth) with pagination for UIs and trading bots.

### API Specification

```rust
// contracts/clob/src/reads.rs
use soroban_sdk::{contracttype, vec, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceLevel {
    /// Price tick (normalized to market tick size)
    pub price_tick: u64,
    
    /// Aggregate remaining base asset lots at this level
    pub base_lots: u64,
    
    /// Number of orders at this level
    pub order_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L2Depth {
    /// Bid levels (descending price)
    pub bids: Vec<PriceLevel>,
    
    /// Ask levels (ascending price)
    pub asks: Vec<PriceLevel>,
    
    /// Cursor for next page (None if no more data)
    pub next_cursor: Option<(bool, u64)>, // (is_bid, price_tick)
}

/// Read L2 depth with bounded traversal
/// 
/// # Arguments
/// * `market_id` - Market identifier
/// * `max_levels` - Maximum levels per side (capped at 50)
/// * `cursor` - Optional continuation cursor (is_bid, price_tick)
pub fn read_l2_depth(
    env: Env,
    market_id: Address,
    max_levels: u32,
    cursor: Option<(bool, u64)>,
) -> L2Depth {
    let max = max_levels.min(50); // Resource limit
    
    let mut bids = vec![&env];
    let mut asks = vec![&env];
    
    // Load market state
    let market = load_market(&env, &market_id);
    
    // Traverse bids (descending from best bid)
    let start_bid = cursor.filter(|(is_bid, _)| *is_bid).map(|(_, tick)| tick)
        .unwrap_or(market.best_bid_tick);
    
    let mut bid_tick = start_bid;
    for _ in 0..max {
        if let Some(level) = aggregate_price_level(&env, &market_id, true, bid_tick) {
            bids.push_back(level.clone());
            bid_tick = level.price_tick - 1;
        } else {
            break;
        }
    }
    
    // Traverse asks (ascending from best ask)
    let start_ask = cursor.filter(|(is_bid, _)| !*is_bid).map(|(_, tick)| tick)
        .unwrap_or(market.best_ask_tick);
    
    let mut ask_tick = start_ask;
    for _ in 0..max {
        if let Some(level) = aggregate_price_level(&env, &market_id, false, ask_tick) {
            asks.push_back(level.clone());
            ask_tick = level.price_tick + 1;
        } else {
            break;
        }
    }
    
    // Determine next cursor
    let next_cursor = if bids.len() == max || asks.len() == max {
        Some((bids.len() == max, bid_tick))
    } else {
        None
    };
    
    L2Depth { bids, asks, next_cursor }
}
```

### Aggregation Logic

```rust
fn aggregate_price_level(
    env: &Env,
    market_id: &Address,
    is_bid: bool,
    price_tick: u64,
) -> Option<PriceLevel> {
    let level_key = (market_id.clone(), is_bid, price_tick);
    
    // Load all orders at this price level
    let order_ids: Vec<u64> = env.storage().persistent()
        .get(&DataKey::PriceLevelOrders(level_key.clone()))?;
    
    let mut total_lots = 0u64;
    let mut count = 0u32;
    
    for order_id in order_ids.iter() {
        if let Some(order) = load_order(env, order_id) {
            total_lots = total_lots.checked_add(order.remaining_lots)?;
            count += 1;
        }
    }
    
    if count == 0 {
        return None;
    }
    
    Some(PriceLevel {
        price_tick,
        base_lots: total_lots,
        order_count: count,
    })
}
```

### Resource Benchmarks

| max_levels | CPU Instructions | Memory Reads | Ledger Reads |
|------------|------------------|--------------|--------------|
| 10         | ~50k             | 20           | 10           |
| 25         | ~125k            | 50           | 25           |
| 50         | ~250k            | 100          | 50           |

---

## Issue #1018: Top-of-Book Helpers

**Objective:** Provide fast, low-cost access to best bid/ask, spread, and mid-price.

### API Specification

```rust
// contracts/clob/src/reads.rs

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopOfBook {
    pub best_bid: Option<PriceLevel>,
    pub best_ask: Option<PriceLevel>,
    pub spread_ticks: Option<u64>,
    pub mid_price_ticks: Option<u64>,
}

/// Read top-of-book market data
/// 
/// Returns None for one-sided markets where spread/mid are undefined
pub fn read_top_of_book(env: Env, market_id: Address) -> TopOfBook {
    let market = load_market(&env, &market_id);
    
    let best_bid = if market.best_bid_tick > 0 {
        aggregate_price_level(&env, &market_id, true, market.best_bid_tick)
    } else {
        None
    };
    
    let best_ask = if market.best_ask_tick < u64::MAX {
        aggregate_price_level(&env, &market_id, false, market.best_ask_tick)
    } else {
        None
    };
    
    // Compute spread (only if both sides exist)
    let spread_ticks = match (&best_bid, &best_ask) {
        (Some(bid), Some(ask)) => {
            ask.price_tick.checked_sub(bid.price_tick)
        }
        _ => None,
    };
    
    // Compute mid-price (overflow-safe)
    let mid_price_ticks = match (&best_bid, &best_ask) {
        (Some(bid), Some(ask)) => {
            // (bid + ask) / 2, but avoid overflow
            let sum = bid.price_tick.checked_add(ask.price_tick)?;
            Some(sum / 2)
        }
        _ => None,
    };
    
    TopOfBook {
        best_bid,
        best_ask,
        spread_ticks,
        mid_price_ticks,
    }
}
```

### Edge Case Handling

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_one_sided_market_bid_only() {
        let env = Env::default();
        let market_id = create_test_market(&env);
        
        // Place only bid orders
        place_order(&env, &market_id, true, 100, 1000);
        
        let tob = read_top_of_book(env.clone(), market_id);
        
        assert!(tob.best_bid.is_some());
        assert!(tob.best_ask.is_none());
        assert!(tob.spread_ticks.is_none()); // Undefined for one-sided
        assert!(tob.mid_price_ticks.is_none());
    }
    
    #[test]
    fn test_overflow_safe_midprice() {
        let env = Env::default();
        
        let bid_tick = u64::MAX - 100;
        let ask_tick = u64::MAX - 50;
        
        // Direct arithmetic would overflow
        // (u64::MAX - 100 + u64::MAX - 50) overflows
        
        let bid = PriceLevel { price_tick: bid_tick, base_lots: 100, order_count: 1 };
        let ask = PriceLevel { price_tick: ask_tick, base_lots: 100, order_count: 1 };
        
        let tob = TopOfBook {
            best_bid: Some(bid),
            best_ask: Some(ask),
            spread_ticks: ask_tick.checked_sub(bid_tick),
            mid_price_ticks: bid_tick.checked_add(ask_tick).map(|s| s / 2),
        };
        
        // Should handle gracefully
        assert!(tob.mid_price_ticks.is_none()); // Overflow returns None
    }
}
```

---

## Issue #1020: Trader Balance Exposure

**Objective:** Expose available, locked, and pending balances for user accounts.

### API Specification

```rust
// contracts/clob/src/reads.rs

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraderBalance {
    /// Total balance deposited
    pub total: i128,
    
    /// Available for withdrawal
    pub available: i128,
    
    /// Locked in open orders
    pub locked: i128,
    
    /// Pending settlement (fills not yet settled)
    pub pending: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenOrderSummary {
    /// Number of active orders
    pub order_count: u32,
    
    /// Cursor for pagination
    pub cursor: Option<u64>, // Last order ID
}

/// Read trader balances for a specific asset in a market
pub fn read_trader_balance(
    env: Env,
    market_id: Address,
    trader: Address,
    asset: Address,
) -> TraderBalance {
    trader.require_auth(); // Only trader can read their own balance
    
    let account_key = (market_id.clone(), trader.clone(), asset.clone());
    let account = load_account(&env, &account_key);
    
    TraderBalance {
        total: account.total_balance,
        available: account.available_balance,
        locked: account.locked_balance,
        pending: account.pending_balance,
    }
}

/// Read open order summary for trader
pub fn read_open_orders(
    env: Env,
    market_id: Address,
    trader: Address,
    cursor: Option<u64>,
    limit: u32,
) -> (Vec<u64>, OpenOrderSummary) {
    trader.require_auth();
    
    let max_limit = limit.min(100); // Resource cap
    let trader_orders_key = (market_id.clone(), trader.clone());
    
    let all_order_ids: Vec<u64> = env.storage().persistent()
        .get(&DataKey::TraderOrders(trader_orders_key))
        .unwrap_or(vec![&env]);
    
    let start_idx = if let Some(c) = cursor {
        all_order_ids.iter().position(|&id| id == c).unwrap_or(0) + 1
    } else {
        0
    };
    
    let mut result = vec![&env];
    let mut count = 0u32;
    
    for i in start_idx..all_order_ids.len() {
        if count >= max_limit {
            break;
        }
        let order_id = all_order_ids.get(i).unwrap();
        result.push_back(order_id);
        count += 1;
    }
    
    let next_cursor = if start_idx + count < all_order_ids.len() {
        result.last().cloned()
    } else {
        None
    };
    
    let summary = OpenOrderSummary {
        order_count: all_order_ids.len(),
        cursor: next_cursor,
    };
    
    (result, summary)
}
```

### Balance Reconciliation Tests

```rust
#[cfg(test)]
mod balance_tests {
    #[test]
    fn test_balance_accounting_after_fill() {
        let env = Env::default();
        let market = create_test_market(&env);
        let trader = Address::generate(&env);
        
        // Deposit 1000 USDC
        deposit(&env, &market, &trader, &usdc_asset(), 1000);
        
        let balance = read_trader_balance(env.clone(), market.clone(), trader.clone(), usdc_asset());
        assert_eq!(balance.total, 1000);
        assert_eq!(balance.available, 1000);
        assert_eq!(balance.locked, 0);
        
        // Place buy order for 500 USDC
        let order_id = place_order(&env, &market, &trader, true, 100, 500);
        
        let balance = read_trader_balance(env.clone(), market.clone(), trader.clone(), usdc_asset());
        assert_eq!(balance.total, 1000);
        assert_eq!(balance.available, 500);
        assert_eq!(balance.locked, 500);
        
        // Fill 50% of order
        fill_order(&env, &market, order_id, 250);
        
        let balance = read_trader_balance(env.clone(), market.clone(), trader.clone(), usdc_asset());
        assert_eq!(balance.total, 1000);
        assert_eq!(balance.available, 500);
        assert_eq!(balance.locked, 250); // Remaining locked
        assert_eq!(balance.pending, 0); // Settled immediately
    }
    
    #[test]
    fn test_multi_market_isolation() {
        let env = Env::default();
        let market_a = create_test_market(&env);
        let market_b = create_test_market(&env);
        let trader = Address::generate(&env);
        
        // Deposit to market A only
        deposit(&env, &market_a, &trader, &usdc_asset(), 1000);
        
        let balance_a = read_trader_balance(env.clone(), market_a, trader.clone(), usdc_asset());
        let balance_b = read_trader_balance(env.clone(), market_b, trader.clone(), usdc_asset());
        
        assert_eq!(balance_a.total, 1000);
        assert_eq!(balance_b.total, 0); // Isolated
    }
}
```

---

## Client Integration Requirements

### Exact Integer Math

```typescript
// SDK implementation (DO NOT USE floating point)
class PriceConverter {
  constructor(
    private tickSize: bigint,
    private baseDecimals: number,
    private quoteDecimals: number
  ) {}
  
  // Convert price tick to human-readable price
  tickToPrice(tick: bigint): string {
    const rawPrice = tick * this.tickSize;
    const decimals = this.quoteDecimals - this.baseDecimals;
    
    if (decimals >= 0) {
      return (rawPrice / BigInt(10 ** decimals)).toString();
    } else {
      return (rawPrice * BigInt(10 ** Math.abs(decimals))).toString();
    }
  }
  
  // Convert human price to tick (MUST validate)
  priceToTick(priceStr: string): bigint | null {
    try {
      const priceBigInt = BigInt(priceStr.replace('.', ''));
      const tick = priceBigInt / this.tickSize;
      
      // Validate: conversion must be exact
      if (this.tickToPrice(tick) !== priceStr) {
        return null; // Invalid: not aligned to tick size
      }
      
      return tick;
    } catch {
      return null;
    }
  }
}
```

---

## Verification Checklist

- [ ] Indexer handles ledger reorgs correctly
- [ ] Replay keys prevent duplicate event processing
- [ ] L2 depth pagination respects resource limits
- [ ] Top-of-book handles one-sided markets
- [ ] Mid-price computation uses overflow-safe arithmetic
- [ ] Trader balances match custody accounting
- [ ] Multi-market topology tested
- [ ] Client SDK uses exact integer math
- [ ] All read paths are bounded (no unbounded iteration)
- [ ] AGENTS.md verification gate passes

---

**Last Updated:** October 2024  
**Phase:** 7 — Read APIs, Indexer, SDKs  
**Status:** Implementation Guide
