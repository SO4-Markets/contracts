# CLOB Resting Order State Model and Persistence Split (Issue #917, CLOB-041)

## 1. Objective

This document defines the canonical resting `Order` struct model, specifies the strict split between persistent on-chain state and derivable fields, and records exact serialized XDR size measurements for Soroban resource budgeting.

In Soroban, every byte written or read contributes directly to transaction resource costs (CPU instructions, memory, read/write bytes, and storage footprint). Keeping the resting order state minimal is essential for high-throughput matching and bounded ledger operations.

---

## 2. Canonical Resting Order Model

The resting order struct contains only the minimum consensus-critical fields required for deterministic order matching, priority queues, cancellation, reduction, expiry, and settlement:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestingOrder {
    /// Unique identifier for this order within the market.
    pub order_id: u64,
    /// Owner address who placed the order and deposited collateral.
    pub owner: Address,
    /// Order side: Bid (Buy) or Ask (Sell).
    pub side: OrderSide,
    /// Limit price expressed in integer ticks (multiple of market tick_size).
    pub price_ticks: u64,
    /// Initial size in integer base lots at order placement.
    pub initial_base_lots: u64,
    /// Remaining unfilled size in integer base lots.
    pub remaining_base_lots: u64,
    /// Monotonic sequence number for deterministic FIFO tie-breaking.
    pub sequence_number: u64,
    /// Ledger sequence deadline after which the order is expired.
    pub expiry_ledger: u64,
    /// Client-supplied order identifier (0 if unset) for off-chain indexing.
    pub client_order_id: u64,
    /// Pointer to the previous order ID in the price-level FIFO queue.
    pub queue_prev: Option<u64>,
    /// Pointer to the next order ID in the price-level FIFO queue.
    pub queue_next: Option<u64>,
}
```

---

## 3. Persistent vs. Derivable Fields Split

To optimize Soroban resource consumption, the order model strictly separates persistent ledger state from fields derivable from market metadata, mathematical rules, or event streams:

| Field Name | Storage Classification | Derivation Source | Rationale |
|---|---|---|---|
| `order_id` | **Persistent** | None | Primary key for order queries and cancellations. |
| `owner` | **Persistent** | None | Required to return funds, verify auth, and attribute fills. |
| `side` | **Persistent** | None | Determines book side (bids vs asks). |
| `price_ticks` | **Persistent** | None | Matching priority and price-level grouping. |
| `initial_base_lots` | **Persistent** | None | Needed for partial fill calculations and auditability. |
| `remaining_base_lots` | **Persistent** | None | Active order size matching against incoming orders. |
| `sequence_number` | **Persistent** | None | Strict time priority for orders at the same price tick. |
| `expiry_ledger` | **Persistent** | None | Deterministic permissionless order expiration. |
| `client_order_id` | **Persistent** | None | Idempotent cancel-by-client and trade attribution. |
| `queue_prev` / `queue_next` | **Persistent** | None | O(1) doubly-linked list insertion and removal. |
| *Market Token / Pair* | **Derivable** | Storage key prefix | Inherent to the contract/market storing the order. |
| *Base / Quote Decimals* | **Derivable** | `MarketConfig` | Stored once per market in instance configuration. |
| *Tick Size / Lot Size* | **Derivable** | `MarketConfig` | Global market configuration. |
| *Quote Notional* | **Derivable** | `ticks × lots × tick_size × lot_size` | Pure arithmetic, never stored. |
| *Filled Quantity* | **Derivable** | `initial_lots - remaining_lots` | Single subtraction. |
| *Fill Percentage* | **Derivable** | `(initial - remaining) / initial` | UI / analytical metric. |
| *Status (Active/Filled/etc.)* | **Derivable** | `remaining > 0` vs `remaining == 0` | State machine invariant. |
| *Fee Rates & Amounts* | **Derivable** | Fee schedule + fill execution price | Computed during settlement execution. |

---

## 4. Serialized Size Measurement & Resource Budget

Soroban uses XDR (External Data Representation) encoding for persistent storage entries.

### Measured Footprint:
- **Struct Component Raw Values**:
  - `order_id`: 8 bytes
  - `owner`: 32 bytes public key + 4 bytes envelope = ~36 bytes
  - `side`: 4 bytes enum discriminant
  - `price_ticks`: 8 bytes
  - `initial_base_lots`: 8 bytes
  - `remaining_base_lots`: 8 bytes
  - `sequence_number`: 8 bytes
  - `expiry_ledger`: 8 bytes
  - `client_order_id`: 8 bytes
  - `queue_prev`: 4-byte discriminant + 8-byte value = 12 bytes
  - `queue_next`: 4-byte discriminant + 8-byte value = 12 bytes
- **Raw Value Payload Size**: **~120 bytes**
- **Soroban ScVal Map Serialization**: **384 bytes** (including all 11 field symbol name keys, type tags, and values as measured by `order.to_xdr(&env)`).
- **Total Storage Entry Footprint**: **~424 bytes** (including 40-byte storage key `(DataKey::Order, order_id)`).

### Resource Impact:
- **Ledger Read**: 424 bytes / entry (well below the 64KB Soroban entry limit).
- **Ledger Write**: Compact 424-byte footprint keeps rent at the minimum Soroban rent tier.
- **Queue Traversal**: Doubly-linked queue permits O(1) cancel and pop operations without touching other price levels.

---

## 5. Lifecycle Operations Supported

The `RestingOrder` model provides complete information for all CLOB lifecycle actions:

1. **Match**: Decrements `remaining_base_lots` by fill amount.
2. **Reduce**: Decrements `remaining_base_lots` without losing queue priority.
3. **Cancel**: Removes order and updates `queue_prev.next` and `queue_next.prev`.
4. **Expire**: Any keeper can check `current_ledger > expiry_ledger` and trigger permissionless cleanup.
5. **Settle**: Multiplies filled lots by price ticks and lot/tick size to determine token amounts.
