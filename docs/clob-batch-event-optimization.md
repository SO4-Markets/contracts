# CLOB Batch Event Optimization and Test Coverage (Issue #1024)

## Overview
This document defines event emission strategies for batch operations, balancing per-order granularity against aggregate event cost, and specifies comprehensive test coverage for all order lifecycle reasons.

## Batch Operation Event Strategies

### Strategy 1: Per-Order Events (Default)
**Use When**: Small batches (<10 orders), need precise per-order attribution

**Example**: Batch cancel 5 orders
```typescript
// Emit 5 separate OrderCancelled events
OrderCancelled { order_id: "0x111", reason: "user_requested", ... }
OrderCancelled { order_id: "0x222", reason: "user_requested", ... }
OrderCancelled { order_id: "0x333", reason: "user_requested", ... }
OrderCancelled { order_id: "0x444", reason: "user_requested", ... }
OrderCancelled { order_id: "0x555", reason: "user_requested", ... }
```

**Cost**: O(n) events, O(n) data storage

### Strategy 2: Aggregate Event with Order List
**Use When**: Medium batches (10-100 orders), cost-sensitive but need full attribution

**Example**: Batch cancel market orders on halt
```typescript
{
  event_version: 1,
  event_type: "batch_order_cancelled",
  market_id: "BTC_USDC",
  reason: "market_closed",
  order_count: 25,
  order_ids: [
    "0x111", "0x222", "0x333", ..., "0x999"
  ],
  total_remaining_quantity: 50000000000,
  ledger_sequence: 1000200,
  ledger_timestamp: 1698001000,
  event_index: 1
}
```

**Cost**: O(1) events, O(n) data in array

### Strategy 3: Aggregate Event with Bloom Filter
**Use When**: Large batches (100+ orders), attribution via separate query

**Example**: Mass expiration at session end
```typescript
{
  event_version: 1,
  event_type: "batch_order_expired",
  market_id: "ETH_USDC",
  reason: "gtc_session_end",
  order_count: 500,
  order_bloom_filter: "0xabcd...ef01",  // Compact probabilistic set
  bloom_filter_params: {
    size: 2048,
    hash_count: 3
  },
  total_remaining_quantity: 100000000000,
  ledger_sequence: 1000300,
  ledger_timestamp: 1698002000,
  event_index: 1
}
```

**Cost**: O(1) events, O(1) data (fixed bloom filter size)

**Trade-off**: Requires separate contract call to confirm specific order termination

## Batch Operation Types and Recommended Strategy

| Operation | Typical Size | Strategy | Rationale |
|-----------|--------------|----------|-----------|
| User batch cancel | <10 | Per-Order | User expects detailed feedback |
| STP cancellations | 1-5 | Per-Order | Critical for audit trail |
| IOC/FOK rejection | 1 | Per-Order | Single order, no batch |
| Market halt cancel | 10-100 | Aggregate + List | Need attribution, manageable size |
| Session expiration | 100-1000 | Aggregate + Bloom | Too many for full list |
| Mass liquidation | 50-500 | Aggregate + Bloom | Emergency scenario, efficiency critical |

## Comprehensive Reason Test Coverage

### Test Matrix: All Cancellation Reasons

#### 1. User Requested (`user_requested`)
```rust
#[test]
fn test_order_cancellation_user_requested() {
    // User submits cancel_order() for open limit order
    // Assert: OrderCancelled event with reason="user_requested"
    // Assert: Order removed from book
    // Assert: remaining_quantity returned to user
}
```

#### 2. Self-Trade Prevention - Old Order (`stp_self_trade_cancelled_old`)
```rust
#[test]
fn test_order_cancellation_stp_old() {
    // User has existing buy order at price P
    // User submits new sell order at price P (would self-trade)
    // Assert: OLD order cancelled with reason="stp_self_trade_cancelled_old"
    // Assert: NEW order accepted or continues matching
}
```

#### 3. Self-Trade Prevention - New Order (`stp_self_trade_cancelled_new`)
```rust
#[test]
fn test_order_cancellation_stp_new() {
    // User has existing buy order at price P with STP mode=CANCEL_INCOMING
    // User submits new sell order at price P
    // Assert: NEW order cancelled with reason="stp_self_trade_cancelled_new"
    // Assert: OLD order remains on book
}
```

#### 4. IOC Not Filled (`ioc_not_filled`)
```rust
#[test]
fn test_order_cancellation_ioc_not_filled() {
    // Submit IOC order with no matching orders on book
    // Assert: OrderAccepted followed by OrderCancelled with reason="ioc_not_filled"
    // Assert: remaining_quantity == original_quantity (no fill)
}
```

#### 5. FOK Not Filled (`fok_not_filled`)
```rust
#[test]
fn test_order_cancellation_fok_not_filled() {
    // Submit FOK order that cannot be completely filled
    // Assert: OrderCancelled with reason="fok_not_filled"
    // Assert: No partial fills occurred
}
```

#### 6. Post-Only Would Take (`post_only_would_take`)
```rust
#[test]
fn test_order_cancellation_post_only_would_take() {
    // Book has sell at 100
    // Submit post-only buy at 100 (would match immediately)
    // Assert: OrderCancelled with reason="post_only_would_take"
    // Assert: No fill occurred
}
```

#### 7. Insufficient Margin (`insufficient_margin`)
```rust
#[test]
fn test_order_cancellation_insufficient_margin() {
    // Order partially fills, user's margin drops below requirement
    // Assert: OrderCancelled with reason="insufficient_margin"
    // Assert: filled_quantity > 0, remaining_quantity > 0
}
```

#### 8. Market Closed (`market_closed`)
```rust
#[test]
fn test_order_cancellation_market_closed() {
    // Open orders on book, admin halts market
    // Assert: All orders cancelled with reason="market_closed"
    // Assert: Batch event emitted for multiple orders
}
```

### Test Matrix: All Expiration Reasons

#### 9. TTL Expired (`ttl_expired`)
```rust
#[test]
fn test_order_expiration_ttl_expired() {
    // Submit order with TTL=10 ledgers
    // Advance 11 ledgers
    // Assert: OrderExpired with reason="ttl_expired"
}
```

#### 10. GTC Session End (`gtc_session_end`)
```rust
#[test]
fn test_order_expiration_gtc_session_end() {
    // Submit GTC order, trading session ends
    // Assert: OrderExpired with reason="gtc_session_end"
}
```

#### 11. GTD Date Reached (`gtd_date_reached`)
```rust
#[test]
fn test_order_expiration_gtd_date_reached() {
    // Submit GTD order with expiry=timestamp T
    // Advance to timestamp T
    // Assert: OrderExpired with reason="gtd_date_reached"
}
```

### Test Matrix: All Rejection Reasons

#### 12. Invalid State (`invalid_state`)
```rust
#[test]
fn test_order_rejection_invalid_state() {
    // Order on book, market parameters change making order invalid
    // Assert: OrderRejected with reason="invalid_state"
}
```

#### 13. Account Locked (`account_locked`)
```rust
#[test]
fn test_order_rejection_account_locked() {
    // Open order, account gets locked (compliance/security)
    // Assert: OrderRejected with reason="account_locked"
}
```

#### 14. Price Out of Bounds (`price_out_of_bounds`)
```rust
#[test]
fn test_order_rejection_price_out_of_bounds() {
    // Market has price collar [90, 110]
    // Price moves to 111, triggering collar
    // Orders at extreme prices rejected
    // Assert: OrderRejected with reason="price_out_of_bounds"
}
```

### Test Matrix: Fill Reasons

#### 15. Market Execution (`market_execution`)
```rust
#[test]
fn test_order_fully_filled_market_execution() {
    // Normal order matching and execution
    // Assert: OrderFullyFilled with reason="market_execution"
}
```

#### 16. Liquidation (`liquidation`)
```rust
#[test]
fn test_order_fully_filled_liquidation() {
    // Liquidation engine force-closes position
    // Assert: OrderFullyFilled with reason="liquidation"
}
```

## Batch Operation Test Scenarios

### Scenario 1: Mass Cancellation Performance
```rust
#[test]
fn test_batch_cancel_100_orders_performance() {
    // Create 100 open orders
    // Batch cancel all via admin (market halt)
    // Assert: Single aggregate event emitted
    // Assert: Gas cost < threshold
    // Benchmark: log cost comparison vs 100 individual events
}
```

### Scenario 2: STP with Multiple Orders
```rust
#[test]
fn test_stp_cancels_multiple_old_orders() {
    // User has 3 buy orders at prices [98, 99, 100]
    // User submits large sell order at 97 (would match all 3)
    // With STP=CANCEL_OLD mode
    // Assert: 3 OrderCancelled events with reason="stp_self_trade_cancelled_old"
    // Assert: Per-order events, not batched (need audit trail)
}
```

### Scenario 3: Session Expiration at Scale
```rust
#[test]
fn test_session_expiration_500_orders() {
    // 500 GTC orders open across multiple traders
    // Session ends
    // Assert: Batch event with bloom filter
    // Assert: Can verify specific order via contract query
    // Assert: Bloom filter has <1% false positive rate
}
```

## Implementation Guidance

### Event Cost Analysis
- Per-order event: ~500 bytes per event
- Aggregate with list: ~500 bytes + 32 bytes * n orders
- Aggregate with bloom: ~500 bytes + fixed bloom size (e.g., 2KB)

### Threshold Configuration
```rust
const BATCH_THRESHOLD_PER_ORDER: usize = 10;
const BATCH_THRESHOLD_BLOOM: usize = 100;

fn emit_lifecycle_events(orders: Vec<Order>, reason: CancellationReason) {
    if orders.len() < BATCH_THRESHOLD_PER_ORDER {
        for order in orders {
            emit_per_order_event(order, reason);
        }
    } else if orders.len() < BATCH_THRESHOLD_BLOOM {
        emit_aggregate_with_list(orders, reason);
    } else {
        emit_aggregate_with_bloom(orders, reason);
    }
}
```

## Related Issues
- Depends on: #885, #886, #980, #1014, #1023
- Part of: Phase 7 — Read APIs, indexer contract, SDKs, and client integration

## Definition of Done
✓ Implementation complete
✓ All 16 reason codes tested (positive paths)
✓ Negative paths tested (invalid transitions)
✓ Batch operation tests cover all strategies
✓ Benchmark evidence collected for event cost comparison
✓ Public docs updated with event schemas
✓ No unbounded iteration in batch operations
