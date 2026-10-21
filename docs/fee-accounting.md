# Fee accounting

The settlement core uses `FEE_RATE_SCALE` fixed-point rates. Each market has two
rates, both in `[0, MAX_FEE_RATE]`:

- `taker_fee_rate`, charged to the taker on each fill;
- `maker_fee_rate`, charged to the maker on each fill.

Rates are non-negative. Maker rebates are out of scope for v1, and a rate of
zero charges nothing.

## Formulas

Every fee is rounded up, so a non-zero obligation is never under-collected.
With `Up(x)` the ceiling of `x` (protocol spec section 4.2):

```text
fee(q, r)          = Up(q * r / FEE_RATE_SCALE)
maker_fee(fill)    = fee(fill_quote, maker_fee_rate)
taker_fee(fill)    = fee(T + fill_quote, taker_rate) - fee(T, taker_rate)
```

`T` is the quote the same taker already filled earlier in the transaction, and
is zero for its first fill. The taker fee is therefore charged on the running
total. The fees of consecutive fills sum to the fee of their total, so splitting
a fill across several makers never changes what the taker pays. The maker fee
has no running total: a maker is filled at most once per transaction.

## Fee rounding and minimum fee policy (CLOB-092)

### Rounding direction

All fees are rounded **up** (ceiling). This ensures the protocol never
under-collects fees. The rounding formula is:

```text
fee(quote_amount, rate) = ceiling(quote_amount * rate / FEE_RATE_SCALE)
                        = floor((quote_amount * rate + FEE_RATE_SCALE - 1) / FEE_RATE_SCALE)
```

### Zero-value fee behavior

Fees may round to zero in the following cases:

1. **Zero rate**: If `rate = 0`, the fee is always zero, regardless of the quote amount.
   This represents a zero-fee market.

2. **Zero amount**: If `quote_amount = 0`, the fee is always zero, regardless of the rate.
   An order with zero remaining quote has no fee obligation.

3. **Tiny fills**: When `quote_amount * rate < FEE_RATE_SCALE`, the multiplication produces
   a numerator less than the scale. After rounding up, the fee is still zero because the
   product is zero. This occurs only when both `quote_amount` and `rate` are zero.

4. **Sub-unit amounts**: When `0 < quote_amount * rate < FEE_RATE_SCALE`, the fee rounds up
   to **1 atom**. This is the minimum non-zero fee.

### Minimum fee: ZERO

**v1 has no minimum fee per fill.** Fills whose calculated fee rounds to zero pay zero.
This policy is acceptable because:

- **No insolvency**: An order that can fill at all has locked enough quote to cover the
  rounded-up fee. The fee formula is used both for locking (at order placement) and for
  settlement (at fill time), with the same rounding direction. A valid order always has
  sufficient locked quote to pay its fee.

- **No fragmentation exploits**: The cumulative taker fee formula prevents splitting a fill
  from reducing the total fee. The fee for consecutive fills of amounts `q1, q2, ..., qn`
  equals the fee for one fill of `q1 + q2 + ... + qn`. Splitting cannot game the fee.

- **Dust prevention at order placement**: Lot size and minimum notional rules (enforced
  during order placement validation) prevent excessive dust trading. Fee logic does not
  duplicate these checks.

### Examples of smallest valid trades

Assume `FEE_RATE_SCALE = 100_000_000` (fixed-point scale with 8 decimal places).

| Quote amount | Taker rate (0.1%) | Maker rate (0.05%) | Taker fee | Maker fee | Notes |
|--------------|-------------------|--------------------|-----------|-----------| ------|
| 0            | 100_000           | 50_000             | 0         | 0         | Zero amount always charges zero |
| 1            | 100_000           | 50_000             | 1         | 1         | Rounds up from 0.000001 and 0.0000005 |
| 999          | 100_000           | 50_000             | 1         | 1         | Still rounds to 1 atom |
| 1_000        | 100_000           | 50_000             | 1         | 1         | Exactly 0.001 quote, no rounding |
| 1_001        | 100_000           | 50_000             | 2         | 1         | Rounds up to 2 for taker |
| 1_000_000    | 100_000           | 50_000             | 1_000     | 500       | Documented test vector |
| 1_000        | 0                 | 0                  | 0         | 0         | Zero-fee market |
| 1_000_000    | 1                 | 1                  | 1         | 1         | Minimal rate rounds to 1 |

### Deterministic behavior

Fee calculation is deterministic: the same quote amount and rate always produce the same fee.
No hidden state, trader history (except cumulative taker fee within a transaction), or market
conditions affect the result. The calculation uses checked integer arithmetic with well-defined
overflow behavior (operations that would overflow return an error and abort the transaction).

## Settlement legs

A fill moves fees out of the trader quote legs and into protocol liabilities:

| Side  | Quote leg, seller is maker    | Quote leg, buyer is maker     |
|-------|-------------------------------|-------------------------------|
| Maker | `+quote - maker_fee`          | `-(quote + maker_fee)`        |
| Taker | `-(quote + taker_fee)`        | `+quote - taker_fee`          |

Each fee is recorded as a `fee_liabilities` entry, so the quote legs and the
liabilities sum to zero. A zero fee records no entry, so a zero-fee market has
no hidden deductions.

Protocol fees are liabilities keyed by token in `FeeAccumulator`. They are not
part of `TraderAccount.available_balance`, `locked_balance`, or `total_balance`,
so withdrawal code cannot spend them. Conservation includes both owners.

## Bid reservation

A resting bid can later fill as a maker, so its lock cannot assume the taker
role. A bid for quote `q` locks:

```text
required_bid_reservation(q) = q + max(taker_fee(q), maker_fee(q))
```

## Tiers

Version one resolves either the market-wide rate or one bounded, already
authorized `FeeTier` in O(1). A tier replaces only the taker rate, and it is
validated against `MAX_FEE_RATE`. No historical-volume scan is performed. The
resolved rate is applied while the fill is settled, so later tier changes do
not alter completed fills.

## Test vectors

These vectors are checked in `libs/clob_settlement_core/src/lib.rs`. Any SDK
preview must produce the same numbers.

| Quote   | Taker rate | Maker rate | Taker fee | Maker fee |
|---------|------------|------------|-----------|-----------|
| 1000000 | 100000     | 50000      | 1000      | 500       |
| 1000    | 100000     | 200000     | 1         | 2         |

Split fills with `taker_rate = 1`: three fills of one quote atom each pay a
total taker fee of `1`, the same as one fill of three atoms.
