# CLOB Order Lifecycle Reason Codes and Terminal Events (Issue #1023)

## Overview
This document defines reason codes and terminal events for complete order lifecycle tracking, enabling indexers to maintain an off-chain open-order view even when terminal state is deleted on-chain.

## Order Lifecycle States

### Non-Terminal States
1. **Accepted**: Order validated and accepted by the matching engine
2. **Resting**: Order is open and resting on the order book
3. **Partial Fill**: Order has been partially filled but remains open

### Terminal States
4. **Fully Filled**: Order completely filled
5. **Cancelled**: Order cancelled by user or system
6. **Expired**: Order expired based on time-in-force
7. **Rejected**: Order rejected (post-acceptance, e.g., insufficient funds after partial fill)

## Reason Codes

### Cancellation Reasons (`cancelled`)
- `user_requested`: User explicitly cancelled the order
- `stp_self_trade_cancelled_old`: Self-trade prevention cancelled the older order
- `stp_self_trade_cancelled_new`: Self-trade prevention cancelled the incoming order
- `ioc_not_filled`: Immediate-or-cancel order not immediately fillable
- `fok_not_filled`: Fill-or-kill order cannot be completely filled
- `post_only_would_take`: Post-only order would match immediately (taken off book)
- `insufficient_margin`: Insufficient margin after partial fill
- `market_closed`: Market halted or closed

### Expiration Reasons (`expired`)
- `ttl_expired`: Time-to-live reached
- `gtc_session_end`: Good-til-cancelled order expired at session end
- `gtd_date_reached`: Good-til-date reached specified date

### Rejection Reasons (`rejected`)
- `invalid_state`: Order state became invalid
- `account_locked`: Account locked during order lifetime
- `price_out_of_bounds`: Price moved out of acceptable bounds (for market protection)

### Fill Reasons (`fully_filled`)
- `market_execution`: Filled through normal market execution
- `liquidation`: Filled as part of liquidation

## Order Lifecycle Events

### 1. OrderAccepted
Emitted when an order is accepted into the matching engine.

```typescript
{
  event_version: 1,
  event_type: "order_accepted",
  market_id: string,
  order_id: string,
  trader: string,
  side: "buy" | "sell",
  order_type: "limit" | "market" | "stop_limit" | "stop_market",
  time_in_force: "gtc" | "ioc" | "fok" | "post_only",
  price_tick: number | null,  // null for market orders
  quantity: number,
  remaining_quantity: number,  // Initially equals quantity
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

### 2. OrderPartialFill
Emitted after each partial fill (in addition to the fill event).

```typescript
{
  event_version: 1,
  event_type: "order_partial_fill",
  market_id: string,
  order_id: string,
  trader: string,
  filled_quantity: number,
  remaining_quantity: number,  // After this fill
  fill_count: number,  // Total number of fills so far
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

### 3. OrderFullyFilled (Terminal)
Emitted when an order is completely filled.

```typescript
{
  event_version: 1,
  event_type: "order_fully_filled",
  market_id: string,
  order_id: string,
  trader: string,
  reason: "market_execution" | "liquidation",
  total_filled_quantity: number,
  total_fill_count: number,
  average_fill_price_tick: number,
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

### 4. OrderCancelled (Terminal)
Emitted when an order is cancelled.

```typescript
{
  event_version: 1,
  event_type: "order_cancelled",
  market_id: string,
  order_id: string,
  trader: string,
  reason: "user_requested" | "stp_self_trade_cancelled_old" | "stp_self_trade_cancelled_new" | 
          "ioc_not_filled" | "fok_not_filled" | "post_only_would_take" | 
          "insufficient_margin" | "market_closed",
  remaining_quantity: number,  // Quantity that was NOT filled
  filled_quantity: number,  // Quantity that WAS filled before cancellation
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

### 5. OrderExpired (Terminal)
Emitted when an order expires.

```typescript
{
  event_version: 1,
  event_type: "order_expired",
  market_id: string,
  order_id: string,
  trader: string,
  reason: "ttl_expired" | "gtc_session_end" | "gtd_date_reached",
  remaining_quantity: number,
  filled_quantity: number,
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

### 6. OrderRejected (Terminal)
Emitted when an order is rejected after acceptance.

```typescript
{
  event_version: 1,
  event_type: "order_rejected",
  market_id: string,
  order_id: string,
  trader: string,
  reason: "invalid_state" | "account_locked" | "price_out_of_bounds",
  remaining_quantity: number,
  filled_quantity: number,
  ledger_sequence: number,
  ledger_timestamp: number,
  event_index: number
}
```

## Lifecycle Invariants

### Deterministic Transitions
1. Every order starts with exactly one `OrderAccepted` event
2. Every order ends with exactly one terminal event:
   - `OrderFullyFilled` XOR `OrderCancelled` XOR `OrderExpired` XOR `OrderRejected`
3. Zero or more `OrderPartialFill` events occur between acceptance and termination
4. `remaining_quantity` MUST monotonically decrease (never increase)
5. `filled_quantity + remaining_quantity = original_quantity` at all times

### Terminal Event Guarantee
No terminal order is deleted from on-chain state without first emitting a terminal lifecycle event.

## Sample Lifecycle Sequences

### Scenario 1: Limit Order Fully Filled
```
OrderAccepted (remaining: 1000)
  → Fill (300) → OrderPartialFill (remaining: 700)
  → Fill (400) → OrderPartialFill (remaining: 300)
  → Fill (300) → OrderFullyFilled (total: 1000)
```

### Scenario 2: IOC Order Cancelled
```
OrderAccepted (remaining: 500)
  → OrderCancelled (reason: ioc_not_filled, remaining: 500, filled: 0)
```

### Scenario 3: Post-Only Rejected
```
OrderAccepted (remaining: 200)
  → OrderCancelled (reason: post_only_would_take, remaining: 200, filled: 0)
```

### Scenario 4: Partial Fill Then User Cancel
```
OrderAccepted (remaining: 1000)
  → Fill (400) → OrderPartialFill (remaining: 600)
  → OrderCancelled (reason: user_requested, remaining: 600, filled: 400)
```

### Scenario 5: Order Expires After Partial Fill
```
OrderAccepted (remaining: 2000)
  → Fill (500) → OrderPartialFill (remaining: 1500)
  → Fill (300) → OrderPartialFill (remaining: 1200)
  → OrderExpired (reason: ttl_expired, remaining: 1200, filled: 800)
```

## Related Issues
- Depends on: #885, #886, #980, #1014
- Part of: Phase 7 — Read APIs, indexer contract, SDKs, and client integration
