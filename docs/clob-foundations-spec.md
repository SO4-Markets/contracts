# CLOB Foundations Specification

This document defines the consensus-critical conventions for market identity, units, price
conversion and events. Contract and SDK code must reference these definitions. The code lives in
`libs/clob_settlement_core/src/` (`market_id.rs`, `units.rs`, `conversion.rs`, `events.rs`).

Covered issues: #880 (market identity), #881 (typed units), #882 (tick and lot conversions), #885
(event schema).

---

## 1. Market identity (#880)

A market is identified by its **unordered** asset pair and a version.

```text
MarketId = sha256( "SO4:clob:market-id:v1" || version_be_u32 || xdr(lo) || xdr(hi) )
```

- `lo <= hi` under Soroban `Address` ordering. `xdr(a)` is the XDR encoding of the address as a
  Soroban value.
- `(base, quote)` and `(quote, base)` derive the same id. Reversed market creation is therefore a
  duplicate and must be rejected by the registry.
- `base == quote` is rejected.
- Base and quote orientation is stored in the market definition, not in the id.
- Native XLM is referenced by its Stellar Asset Contract address, like every other asset.

## 2. Units (#881)

| Type | Inner width | Meaning |
|------|-------------|---------|
| `BaseAtoms`, `QuoteAtoms` | `i128`, non-negative | Token amounts in smallest unit |
| `BaseLots`, `QuoteLots` | `u64` | Count of lots (`lot_size` atoms each) |
| `Tick` | `u64` | Price level index |
| `SequenceNumber` | `u64` | Per-market order sequence, used as the order id |

Rules:

- No `From` impl exists between units. Converting between them requires a named function.
- Arithmetic is checked and returns `Option`.
- Token decimals must be at most `MAX_TOKEN_DECIMALS = 18`. This keeps `10^18 * 10^18` below
  `i128::MAX`.
- Atoms-to-lots conversion floors and returns the dust, or converts exactly and rejects dust.
  It returns `None` when the lot count does not fit in `u64`.

## 3. Price and conversions (#882)

```text
scaled_price = tick * tick_size                       (exact)
quote_atoms  = base_atoms * scaled_price / 10^8        (rounded)
```

- `tick_size` is a positive `u64` in quote atoms per base atom, scaled by `PRICE_SCALE = 10^8`.
- Lot-to-atom conversion is exact. Rounding happens only at the price step.
- Every divide takes an explicit `Rounding`. There is no default.

| Rounding | Use when |
|----------|----------|
| `Up` | The trader pays quote in: buy-side payments and reserves |
| `Down` | The protocol pays quote out: sell-side proceeds |

Both directions differ by at most one quote atom. No path uses floating point.

## 4. Event schema (#885)

Every record carries `market: MarketId` and `index: EventIndex`.

- `EventIndex` starts at 1 and increases by exactly 1 per market. A gap or duplicate means the
  stream is corrupt, and clients must re-sync from a snapshot.
- Orders are identified by the `SequenceNumber` assigned at acceptance. The same id appears on
  every later event for that order.
- Cancel and expiry remove all remaining lots.
- For a closed order: `accepted = filled + canceled + expired`.

| Topic | Payload |
|-------|---------|
| `deposit` | trader, asset, amount |
| `withdrawal` | trader, asset, amount |
| `order_accepted` | order, trader, side, price, lots, client_id |
| `order_rested` | order, remaining |
| `order_reduced` | order, reduced_by, remaining |
| `order_canceled` | order, canceled |
| `order_expired` | order, expired |
| `fill` | maker_order, taker_order, maker, taker, taker_side, price, lots, quote, maker_remaining, taker_remaining |
| `fee_accrued` | asset, kind (`Execution`, `Position`, `Referral`), amount |
| `fee_withdrawn` | asset, recipient, amount |
| `market_status_changed` | previous, next (`Open`, `Paused`, `Closed`) |
| `config_changed` | field (`TickSize`, `LotSize`, `FeeRate`), previous, next |

Topic strings are wire identifiers. Renaming one is a breaking change for indexers.

Open for spec owners: the `MarketStatus` and `ConfigField` variants above are the initial set. Confirm
them before SDK codegen.
