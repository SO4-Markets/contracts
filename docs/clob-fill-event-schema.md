# CLOB Fill Event Schema (Issue #1021)

## Overview
This document defines the exact event fields required for indexers to reconstruct historical trades, OHLCV data, maker/taker attribution, and order lifecycle without persisting history on-chain.

## Fill Event Fields

### Core Trade Identification
- `market_id`: Market identifier (trading pair)
- `maker_order_id`: Order ID of the passive (maker) side
- `taker_order_id`: Order ID of the aggressive (taker) side
- `trade_sequence`: Monotonic sequence number for this fill within the market

### Trader Attribution
- `maker_trader`: Address or identifier of the maker trader
- `taker_trader`: Address or identifier of the taker trader

### Execution Details
- `side`: Order side (`buy` or `sell`)
- `aggressor_side`: Which side was aggressive (`maker` or `taker`)
- `price_tick`: Executed price in price ticks (integer)
- `base_quantity`: Base asset quantity filled (integer, smallest unit)
- `quote_quantity`: Quote asset quantity exchanged (integer, smallest unit)

### Fee Information
- `maker_fee`: Fee charged to maker (integer, quote asset units)
- `taker_fee`: Fee charged to taker (integer, quote asset units)
- `fee_recipient`: Address receiving the fees (if applicable)

### Sequencing and Context
- `ledger_sequence`: Stellar ledger number when fill occurred
- `ledger_timestamp`: Unix timestamp of the ledger
- `event_index`: Index of this event within the ledger/transaction

### Event Metadata
- `event_version`: Schema version (currently `1`)
- `event_type`: Always `"fill"` for fill events

## Derived Fields (Not Included)
The following can be derived from stable market metadata and are excluded to reduce event cost:
- Base asset symbol/decimals
- Quote asset symbol/decimals
- Tick size
- Lot size

## Indexer Reconciliation
Fill totals MUST reconcile with:
- Sum of `base_quantity` = order fill amount
- Sum of `quote_quantity` = total consideration
- `maker_fee + taker_fee` = total fees collected

## Sample Fill Event (JSON)
```json
{
  "event_version": 1,
  "event_type": "fill",
  "market_id": "ETH_USDC",
  "maker_order_id": "0x1234...abcd",
  "taker_order_id": "0x5678...ef01",
  "trade_sequence": 42,
  "maker_trader": "GABC...XYZ",
  "taker_trader": "GDEF...UVW",
  "side": "buy",
  "aggressor_side": "taker",
  "price_tick": 2500000,
  "base_quantity": 100000000,
  "quote_quantity": 250000000000,
  "maker_fee": 25000000,
  "taker_fee": 75000000,
  "fee_recipient": "GFEE...FEE",
  "ledger_sequence": 1234567,
  "ledger_timestamp": 1698765432,
  "event_index": 3
}
```

## Usage Guidelines
- **Exact Integer Math**: Client libraries MUST use exact integer arithmetic for price/size conversions
- **No Silent Rounding**: Invalid user inputs must be rejected, not silently rounded
- **Historical Reconstruction**: Indexers can build complete trade history from these events alone
- **OHLCV Construction**: Aggregate fills by time buckets using `ledger_timestamp` and `price_tick`
- **Maker/Taker Attribution**: Use `aggressor_side` to determine which order was passive vs aggressive

## Related Issues
- Depends on: #885, #886, #956, #1003
- Part of: Phase 7 — Read APIs, indexer contract, SDKs, and client integration
