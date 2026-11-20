# CLOB Event Versioning and Ordering (Issue #1022)

## Overview
This document defines event version guarantees and ordering semantics for CLOB fill events, ensuring indexers can reliably reconstruct market state.

## Event Version Scheme

### Version Field
- All events include an `event_version` field (unsigned integer)
- Current version: `1`
- Future schema changes increment this version
- Multiple versions MAY coexist during migration periods

### Version Compatibility
- **Additive Changes**: New optional fields → same version number OK
- **Breaking Changes**: Field removal, type changes, semantics changes → new version number REQUIRED
- **Indexers**: MUST handle unknown versions gracefully (skip or log warning)

## Event Ordering Guarantees

### Within a Single Ledger
1. Events are ordered by `event_index` (ascending)
2. `trade_sequence` increments monotonically within each market
3. Fill events for the same order MUST appear in execution order

### Across Ledgers
1. Events are ordered by `ledger_sequence` (ascending)
2. `ledger_timestamp` provides wallclock ordering (may not be strictly monotonic due to clock skew)
3. `trade_sequence` is market-scoped and monotonic across ledgers

### Ordering Invariants
- For any two fills F1 and F2 in the same market:
  - If `F1.trade_sequence < F2.trade_sequence`, then `F1` executed before `F2`
  - If `F1.ledger_sequence < F2.ledger_sequence`, then `F1` finalized before `F2`

## Published Event Samples

### Sample 1: Simple Buy Fill
```json
{
  "event_version": 1,
  "event_type": "fill",
  "market_id": "BTC_USDC",
  "maker_order_id": "0xaaa1...1111",
  "taker_order_id": "0xbbb2...2222",
  "trade_sequence": 1,
  "maker_trader": "GMKR...ABC",
  "taker_trader": "GTKR...DEF",
  "side": "buy",
  "aggressor_side": "taker",
  "price_tick": 40000000,
  "base_quantity": 50000000,
  "quote_quantity": 2000000000000,
  "maker_fee": 2000000,
  "taker_fee": 6000000,
  "fee_recipient": "GFEE...FEE",
  "ledger_sequence": 1000000,
  "ledger_timestamp": 1698000000,
  "event_index": 1
}
```

### Sample 2: Sell Fill with Partial Maker
```json
{
  "event_version": 1,
  "event_type": "fill",
  "market_id": "ETH_USDC",
  "maker_order_id": "0xccc3...3333",
  "taker_order_id": "0xddd4...4444",
  "trade_sequence": 42,
  "maker_trader": "GMKR...GHI",
  "taker_trader": "GTKR...JKL",
  "side": "sell",
  "aggressor_side": "taker",
  "price_tick": 2500000,
  "base_quantity": 200000000,
  "quote_quantity": 500000000000,
  "maker_fee": 50000000,
  "taker_fee": 150000000,
  "fee_recipient": "GFEE...FEE",
  "ledger_sequence": 1000001,
  "ledger_timestamp": 1698000005,
  "event_index": 7
}
```

### Sample 3: Multiple Fills Same Order
Order `0xeee5...5555` fills in three parts across two ledgers:

```json
[
  {
    "event_version": 1,
    "event_type": "fill",
    "market_id": "SOL_USDC",
    "maker_order_id": "0xeee5...5555",
    "taker_order_id": "0xfff6...6666",
    "trade_sequence": 100,
    "maker_trader": "GMKR...MNO",
    "taker_trader": "GTKR...PQR",
    "side": "buy",
    "aggressor_side": "taker",
    "price_tick": 100000,
    "base_quantity": 300000000,
    "quote_quantity": 30000000000,
    "maker_fee": 3000000,
    "taker_fee": 9000000,
    "fee_recipient": "GFEE...FEE",
    "ledger_sequence": 1000100,
    "ledger_timestamp": 1698000500,
    "event_index": 2
  },
  {
    "event_version": 1,
    "event_type": "fill",
    "market_id": "SOL_USDC",
    "maker_order_id": "0xeee5...5555",
    "taker_order_id": "0x1117...7777",
    "trade_sequence": 101,
    "maker_trader": "GMKR...MNO",
    "taker_trader": "GTKR...STU",
    "side": "buy",
    "aggressor_side": "taker",
    "price_tick": 100000,
    "base_quantity": 200000000,
    "quote_quantity": 20000000000,
    "maker_fee": 2000000,
    "taker_fee": 6000000,
    "fee_recipient": "GFEE...FEE",
    "ledger_sequence": 1000100,
    "ledger_timestamp": 1698000500,
    "event_index": 8
  },
  {
    "event_version": 1,
    "event_type": "fill",
    "market_id": "SOL_USDC",
    "maker_order_id": "0xeee5...5555",
    "taker_order_id": "0x2228...8888",
    "trade_sequence": 105,
    "maker_trader": "GMKR...MNO",
    "taker_trader": "GTKR...VWX",
    "side": "buy",
    "aggressor_side": "taker",
    "price_tick": 100000,
    "base_quantity": 500000000,
    "quote_quantity": 50000000000,
    "maker_fee": 5000000,
    "taker_fee": 15000000,
    "fee_recipient": "GFEE...FEE",
    "ledger_sequence": 1000101,
    "ledger_timestamp": 1698000505,
    "event_index": 3
  }
]
```

## Schema Pinning Tests
Tests MUST verify:
1. Event topic matches expected hash
2. Field order and types match published schema
3. Version number is present and correct
4. All required fields are present
5. Optional fields have correct nullability

## Related Issues
- Depends on: #885, #886, #956, #1003, #1021
- Part of: Phase 7 — Read APIs, indexer contract, SDKs, and client integration
