# Enumerable Market Registry — Implementation Guide

**Issue:** #015 — Implement an enumerable market registry

**Status:** Complete — Bounded enumeration, O(1) lookup, pagination support, soft-delete TTL

---

## Overview

Clients need a canonical way to discover markets without scanning contract history. The registry provides:

- **Bounded enumeration**: Pages of exactly 100 markets max per read
- **O(1) lookup**: Direct address-based metadata retrieval  
- **Pagination support**: Clients iterate through markets one page at a time
- **Soft-delete TTL**: Markets can be marked deleted at a specific ledger; clients filter by activity
- **No unbounded iteration**: Storage patterns prevent full-list scans

---

## Architecture

### Storage Pattern

```
Market Metadata Storage:
  Key: MarketId (Address)
  Value: MarketMetadata
    - id: Address (market contract address)
    - base: Address (base token)
    - quote: Address (quote token)
    - created_ledger: u32 (creation time)
    - deleted_ledger: Option<u32> (soft-delete time)

Market Index Pagination:
  Key: PageNumber (u32)
  Value: Vec<Address> (up to 100 market IDs per page)
```

**Lookup:** O(1) — `storage[market_id]`  
**Enumeration:** O(1 + page_size) — load page index + metadata for markets on page

### Bounded Page Size

```rust
pub const MARKETS_PER_PAGE: usize = 100;
```

A single `enumerate_markets(page)` call:
1. Loads one page of up to 100 market IDs from paginated storage
2. Loads metadata for each ID (100 individual O(1) reads)
3. Returns page with `has_next` flag

Total cost: bounded, never unbounded.

---

## API Specification

### Enumeration: `enumerate_markets(page, include_deleted)`

```rust
pub fn enumerate_markets(
    env: Env,
    page: u32,
    include_deleted: bool,
) -> Result<MarketRegistryPage, &'static str>
```

**Arguments:**
- `page` — Page number (0-based). Maximum page is `ceil(total_markets / 100) - 1`
- `include_deleted` — If false, filter out markets with `deleted_ledger` at or before current ledger

**Returns:**
```rust
pub struct MarketRegistryPage {
    pub markets: Vec<MarketRegistry>,  // Up to 100 markets
    pub page: u32,                      // Current page number
    pub total_markets: u32,             // Total active markets (if include_deleted=false)
    pub has_next: bool,                 // Whether page+1 exists
}
```

**Resource Guarantees:**
- CPU: O(100) — max 100 markets per page
- Memory: O(100) — fixed allocation
- Storage reads: O(1 + page_size) — one index lookup + individual metadata

**Example:** Discover all markets in 3 pages (250 markets total)
```
Page 0: markets[0..100], has_next = true
Page 1: markets[100..200], has_next = true
Page 2: markets[200..250], has_next = false
```

### Lookup: `lookup_market(market_id)`

```rust
pub fn lookup_market(
    env: Env,
    market_id: Address,
) -> Result<Option<MarketRegistry>, &'static str>
```

**Arguments:**
- `market_id` — Market contract address

**Returns:**
```rust
pub struct MarketRegistry {
    pub id: Address,                    // Market contract address
    pub base: Address,                  // Base token
    pub quote: Address,                 // Quote token
    pub created_ledger: u32,            // Creation ledger
    pub deleted_ledger: Option<u32>,    // Deletion ledger (if closed)
}
```

**Resource Guarantees:**
- CPU: O(1)
- Memory: O(1)
- Storage reads: 1

---

## Soft-Delete and TTL Behavior

Markets are never hard-deleted; instead, they are marked with a `deleted_ledger` timestamp:

```rust
impl MarketMetadata {
    pub fn is_active(&self, current_ledger: u32) -> bool {
        match self.deleted_ledger {
            None => true,
            Some(del_ledger) => current_ledger < del_ledger,
        }
    }
}
```

**Timeline example:**
- Ledger 100: Market created (`created_ledger = 100`)
- Ledger 500: Market marked for closure (`deleted_ledger = 500`)
- Ledgers 100–499: Market is active (`is_active(ledger) = true`)
- Ledgers 500+: Market is inactive (`is_active(ledger) = false`)

**Why TTL instead of immediate deletion?**
- Allows ledger reorgs: if a deletion is reorg'd out, the market remains accessible
- Supports indexer recovery: indexers can reconcile their state at any ledger
- Audit trail: the deletion timestamp is permanently recorded
- Off-chain discovery: clients query at a specific ledger and see the market state at that time

---

## Storage Implementation Details

### Pages Layout

```
Storage:
  `markets:count` → total number of markets ever registered
  `markets:page:0` → [market_id_0, market_id_1, ..., market_id_99]  (100 max)
  `markets:page:1` → [market_id_100, market_id_101, ..., market_id_199]
  ...
  `markets:meta:{market_id}` → MarketMetadata
```

### Index Bounds Checking

When loading page `N`:
- Valid if `N * MARKETS_PER_PAGE < total_markets`
- Invalid page returns `Err(PageOutOfRange)`

### Filtered Enumeration

When `include_deleted = false`:
- Load page of market IDs
- For each ID, load metadata
- Filter by `current_ledger < deleted_ledger`
- Clients see only active markets in returned count

---

## Acceptance Criteria — Verified

✓ **Registry lookup is O(1) or bounded**
  - Direct address-based lookup: O(1)
  - Paginated enumeration: O(1) + O(100) per page

✓ **Enumeration never reads an unbounded list**
  - Maximum 100 markets per page via `MARKETS_PER_PAGE` constant
  - Page-based pagination prevents full scans

✓ **Tests cover multiple pages and deleted/closed markets**
  - `test_multiple_pages`: 250 markets across 3 pages
  - `test_deleted_market_soft_delete_ttl`: Soft-delete with ledger TTL
  - `test_closed_market_not_enumerated`: Filtering of deleted markets
  - `test_pagination_with_deletions_snapshot`: Mixed active/deleted across pages

✓ **Metadata exposed for SDKs**
  - `base`, `quote` tokens included in `MarketRegistry`
  - `created_ledger`, `deleted_ledger` for audit trail
  - All data types are `#[contracttype]` for Soroban serialization

✓ **TTL behavior defined**
  - `deleted_ledger: Option<u32>` marks closure time
  - `is_active(ledger)` checks against TTL
  - Clients filter by activity when enumerating

---

## Test Coverage

### Unit Tests (market_registry.rs in lib)

- `test_market_metadata_is_active` — TTL boundary conditions
- `test_pagination_cursor_defaults` — Cursor initialization
- `test_market_page_properties` — Page structure validation
- `test_registry_entry_struct` — Entry type correctness
- `test_bounded_page_size` — Constant verification

### Integration Tests (tests/market_registry.rs)

- `test_single_market_is_active` — Active market behavior
- `test_deleted_market_soft_delete_ttl` — Soft-delete timeline
- `test_multiple_markets_same_creation` — Bulk market handling
- `test_markets_with_staggered_deletion` — Partial deletion
- `test_pagination_boundary` — Exactly-one-page scenario
- `test_multiple_pages` — 2.5 pages (250 markets) enumeration
- `test_closed_market_not_enumerated` — Deletion filtering
- `test_market_discovery_without_history_scan` — Bounded discovery guarantee
- `test_deleted_market_timeline` — TTL state transitions
- `test_max_markets_per_page_boundary` — Edge cases at 100
- `test_empty_registry` — Zero-market edge case
- `test_o1_lookup_by_id` — Direct lookup verification
- `test_concurrent_markets_same_epoch` — Bulk creation
- `test_pagination_with_deletions_snapshot` — Deletions across pages

---

## Usage Examples

### Discover All Markets

```rust
// Client code
let mut page_num = 0;
let mut all_markets = Vec::new();

loop {
    let page = enumerate_markets(env, page_num, false)?;  // Active only
    all_markets.extend(page.markets.iter());
    
    if !page.has_next {
        break;
    }
    page_num += 1;
}

println!("Found {} active markets", all_markets.len());
```

### Check if Market Exists

```rust
match lookup_market(env, market_id)? {
    Some(market) => println!("Market: {} / {}", market.base, market.quote),
    None => println!("Market not found"),
}
```

### Filter by Creation Time

```rust
let cutoff_ledger = 10_000;
let page = enumerate_markets(env, 0, true)?;  // Include deleted

for market in page.markets {
    if market.created_ledger >= cutoff_ledger {
        println!("Recent: {}", market.id);
    }
}
```

---

## SDK Integration

### Exact Integer Math

Clients use exact integer types for token amounts and prices:

```typescript
// No floating point — use bigint throughout
const market = await registry.lookup(marketId);
const baseDecimals = 6;   // USDC
const quoteDecimals = 18; // EUR (hypothetical)

const tickSize = 100_000_000n; // Scaled by PRICE_SCALE
const price = (tickSize * 4_5n) / (10n ** (quoteDecimals - baseDecimals));
```

### Canonical Market References

Clients store and compare markets by ID (Address), never by name:

```typescript
type MarketId = string; // Address, canonical identifier
type Pair = { base: TokenAddress, quote: TokenAddress };

// Registry lookup is immutable per market ID
const pairFromId = async (id: MarketId) => {
  const meta = await registry.lookup(id);
  return { base: meta.base, quote: meta.quote };
};
```

---

## Definition of Done

✓ Market registry types defined with bounded structures  
✓ `enumerate_markets` API with pagination  
✓ `lookup_market` API for O(1) access  
✓ Soft-delete via `deleted_ledger` TTL  
✓ Public types in reader API (`MarketRegistry`, `MarketRegistryPage`)  
✓ Unit tests for metadata and pagination  
✓ Integration tests for multiple pages and deletions  
✓ Documentation covering API, storage, and TTL behavior  
✓ No new unbounded iteration patterns  
✓ Resource bounds verified (O(1) lookup, O(page_size) enumeration)

---

## Related Issues

- **#015** — This implementation
- **#900** — Market configuration validation (uses MarketConfig)
- **#893** — Registry topology decision
- **#905** — Custody scope (multi-market balances not double-counted per registry)
- **#891** — Contract upgradeability (registry and markets can upgrade independently)

---

## Storage Benchmarks

| Operation | CPU | Memory | Storage Reads |
|-----------|-----|--------|---------------|
| `lookup_market` | O(1) | O(1) | 1 |
| `enumerate_markets` (empty page) | O(1) | O(100) | 1 |
| `enumerate_markets` (page 0, 100 markets) | O(100) | O(100) | 101 |
| Discover all 250 markets (3 pages) | O(300) | O(100) | 303 |
| Add market | O(1) | O(1) | 2 (if paginating) |
| Delete market | O(1) | O(1) | 1 (TTL write) |

---

**Last Updated:** October 2026  
**Phase:** 1 — Market creation, custody, balances  
**Scope:** Issue #015 - Complete
