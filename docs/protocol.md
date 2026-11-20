# SO4 CLOB Protocol Specification

This document defines the consensus-critical behavior of the SO4 central limit order book (CLOB).
Contract and SDK code must follow these definitions. Where code and this document disagree, this
document wins and the code is a bug. The specification is split into two parts: trading semantics
(sections 1 to 8) and invariants with bounded matching (sections 9 to 11).

Conventions use the **MUST**, **MUST NOT**, and **MAY** keywords as in RFC 2119.

---

## 1. Scope

The v1 exchange is a **fully collateralized spot central limit order book** on Soroban. Every order
is backed by tokens locked in the trader's account before it can rest or match. Matching follows
price-time priority, in the style of Stellar's offers and Phoenix's FIFO matching. Storage and
execution are adapted to Soroban resource limits: every transaction performs a bounded amount of
work (section 10).

## 2. Non-goals for v1

The following are explicitly out of scope. They MUST NOT be implemented as part of v1 and no
behavior in v1 may depend on them.

- Leverage, margin, and borrowing.
- Perpetual contracts and funding rates.
- Liquidations.
- Synthetic positions and oracle-priced settlement.
- Market orders. Every v1 order is a limit order.
- Stop, post-only, immediate-or-cancel, and fill-or-kill orders. v1 time-in-force is limited to
  GTC and GTD (section 5.3).
- In-place order amendment. A trader changes an order by cancelling it and placing a new one.
- Maker rebates (negative maker fees) and historical-volume fee tiers. v1 maker and taker fees
  are defined in `docs/fee-accounting.md`. Each fee MUST be applied as part of its fill, in a way
  that preserves asset conservation (invariant I1).

## 3. Markets and parameters

A **market** is an ordered pair `(base, quote)` of two distinct token contracts. The base token is
what is bought and sold. The quote token is the currency each price is denominated in. Native XLM
is referenced by its Stellar Asset Contract address, like every other token.

Each market has the following parameters, set once at creation from trusted configuration and
immutable afterwards:

| Parameter   | Type         | Meaning                                                 |
| ----------- | ------------ | ------------------------------------------------------- |
| `lot_size`  | `u64`, `> 0` | Base atoms per lot. All order sizes are whole lots.     |
| `tick_size` | `u64`, `> 0` | Price increment, scaled by `PRICE_SCALE` (section 4.1). |

Constants shared by all markets:

| Constant          | Value                   | Meaning                                                                                                           |
| ----------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `PRICE_SCALE`     | `10^8`                  | Denominator for scaled prices. Defined in `clob_settlement_core/src/lib.rs`.                                      |
| `MAX_TICK`        | implementation constant | Largest valid tick index. Set by benchmark harness; defined in `clob_settlement_core/src/lib.rs`.                 |
| `MAX_MATCH_STEPS` | implementation constant | Work budget per transaction (section 10). Set by benchmark harness; defined in `clob_settlement_core/src/lib.rs`. |

## 4. Units, price, and rounding

### 4.1 Units and price

- **Atoms** are the smallest unit of a token. Base amounts are measured in base atoms and quote
  amounts in quote atoms.
- **Lots** are the unit of order size. `base_atoms = base_lots * lot_size`. This conversion is
  exact and checked.
- A **tick** is a price index in `1..=MAX_TICK`. Ticks are the only prices an order can carry.
- The **scaled price** is `scaled_price = tick * tick_size`, computed with checked `u64`
  multiplication. An order whose scaled price overflows is rejected.
- The **price** is the number of quote atoms per base atom, `scaled_price / PRICE_SCALE`.

For a base amount `b` in atoms at scaled price `s`, the quote amount is:

```text
quote(b, s) = b * s / PRICE_SCALE            (exact rational value)
```

All intermediate products use checked `i128` arithmetic. An overflow rejects the operation. It never
wraps and never saturates.

### 4.2 Rounding

Every division in the protocol takes an explicit rounding mode. There is no default. Two functions
are used:

- `Up(x)` is the ceiling of the exact rational value `x`.
- `Down(x)` is the floor of the exact rational value `x`.

The rules below are binding. They are chosen so that every token atom is accounted for.

| Quantity                                              | Rounding                                   | Reason                                   |
| ----------------------------------------------------- | ------------------------------------------ | ---------------------------------------- |
| Quote locked by a bid for a remaining amount `r` lots | `Up(quote(r * lot_size, s))`               | The trader never under-reserves.         |
| Base locked by an ask for a remaining amount `r` lots | exact, `r * lot_size`                      | Base is never divided.                   |
| Quote paid to the seller for a fill of `f` lots       | `Down(quote(f * lot_size, s))`             | The protocol never over-pays a seller.   |
| Quote debited from the buyer for a fill               | difference of two `Up` locks (section 7.2) | Buyer debit always covers seller credit. |

The difference between the buyer's debit and the seller's credit is a **rounding residue**. It is
less than one quote atom per fill. The residue is credited to the market's protocol dust account
(section 9.1), so no value is created or destroyed.

## 5. Orders

### 5.1 Fields

Each order has:

| Field            | Type                          | Meaning                                                                                              |
| ---------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------- |
| `id`             | `u64`                         | Per-market sequence number. Strictly increasing and never reused. This is the order's time priority. |
| `owner`          | `Address`                     | The account that placed the order.                                                                   |
| `side`           | `Bid` or `Ask`                | `Bid` buys base with quote. `Ask` sells base for quote.                                              |
| `tick`           | `u64`                         | Limit price.                                                                                         |
| `remaining_lots` | `u64`                         | Unfilled size. Always `> 0` while the order rests.                                                   |
| `locked`         | `i128`                        | Tokens currently locked for this order (section 5.2).                                                |
| `tif`            | `GTC` or `GTD(expiry_ledger)` | Time in force (section 5.3).                                                                         |

### 5.2 Locking

An order locks its full notional before it is matched.

- A bid with `remaining_lots = r` locks `Up(quote(r * lot_size, scaled_price))` quote atoms.
- An ask with `remaining_lots = r` locks `r * lot_size` base atoms.

Locks are taken from the trader's available balance. The lock is **all or nothing**. If the
available balance is insufficient, the order is rejected and nothing changes. A partially locked
order never exists.

A bid's `locked` is always equal to `Up(quote(remaining_lots * lot_size, scaled_price))`. This
equality is the basis for the fill debit in section 7.2.

### 5.3 Time in force

- **GTC** orders rest until they are fully filled or cancelled.
- **GTD(expiry_ledger)** orders rest until `expiry_ledger`. They are inactive when the current
  ledger sequence is greater than or equal to `expiry_ledger`.

A GTD order MUST be placed with `expiry_ledger > current_ledger`. A GTC order carries no expiry.

### 5.4 Placement procedure

A new order is processed in this order:

1. Validate: `lots >= 1`, `tick` in range, `tif` valid, and the caller is authorized as `owner`.
2. Lock the full notional (section 5.2). Reject on insufficient balance.
3. Match against the opposite side of the book (section 6).
4. Rest the remainder only if all of the following hold:
   - `remaining_lots > 0`,
   - the order is active under its time in force,
   - matching ended because no crossing liquidity was left, not because of a bound or a self-trade
     (sections 6.9 and 10).
5. Otherwise release the remainder's lock to the owner's available balance.

A transaction creates at most one resting order: the taker's remainder.

## 6. Matching

### 6.1 Priority

An incoming order is the **taker**. An order already resting on the book is a **maker**.

- **Price priority**: the best price matches first. For an incoming bid this is the lowest ask tick.
  For an incoming ask it is the highest bid tick.
- **Time priority**: within one tick, the lowest `id` matches first. Each tick is a FIFO queue.

Priority is total. Two resting orders never have the same `(tick, id)`.

### 6.2 Crossing

- An incoming bid at tick `t` crosses a resting ask at tick `q` if and only if `t >= q`.
- An incoming ask at tick `t` crosses a resting bid at tick `p` if and only if `t <= p`.

Matching stops at the first resting order that does not cross.

### 6.3 Execution price

Every fill executes at the **maker's tick**. The taker receives any price improvement. For example,
an incoming bid at tick 12 that matches a resting ask at tick 10 executes at tick 10.

### 6.4 Fill size and partial fills

The fill size is `f = min(taker.remaining_lots, maker.remaining_lots)`.

- A partially filled maker keeps its `id`, so it keeps its place in the queue.
- A partially filled taker continues matching against the next crossing maker.
- A maker whose `remaining_lots` reaches zero is removed from the book immediately. A zero-size
  order never rests.

### 6.5 Equal prices

Orders at the same tick are filled in ascending `id` order. A later order at that tick never
overtakes an earlier one. Equal-price orders are not merged.

### 6.6 Zero remainder

When the taker's `remaining_lots` reaches zero:

- no order rests,
- the taker's lock ends at `Up(quote(0, s)) = 0` for a bid, so no quote is released,
- any remaining base lock for an ask is zero, because the ask locked exactly its base.

No zero-size order is ever stored, emitted as resting, or returned as active.

### 6.7 Matching loop

Matching repeats the following until one of the stop conditions holds:

1. Select the highest-priority maker on the opposite side (section 6.1).
2. If there is none, or it does not cross the taker (section 6.2), stop with **no liquidity**.
3. If the maker belongs to the same owner as the taker, stop with **self-trade** (section 6.9).
4. If the maker is expired (section 8.2), remove it and continue.
5. Otherwise fill `f` lots at the maker's tick (section 7 settles it), and continue.

Each iteration counts against the work budget (section 10).

### 6.8 Expired makers

A resting GTD maker whose expiry has passed is inactive. It MUST NOT be filled. Matching removes
it, releases its lock to its owner, and continues. Each removal counts as one step of the work
budget.

### 6.9 Self-trade

If the next crossing maker belongs to the taker's owner, matching stops and the taker's remainder
is released. The maker is not touched and no fill is produced. This is **cancel-newest**: the
incoming order yields to the trader's own resting order. The stop emits a `SelfTradeStop` event.

## 7. Settlement

### 7.1 Settlement timing

Fills settle **immediately**, within the transaction that produces them. There is no pending
settlement state in v1. Every fill moves tokens between locked and available balances of the
accounts involved.

### 7.2 Fill legs

For a fill of `f` lots at scaled price `s` between a bid `B` and an ask `A`, let `b = f * lot_size`:

**Base leg.** The seller's locked base decreases by `b`. The buyer's available base increases by
`b`.

**Quote leg.**

```text
lock_before = Up(quote(remaining_before(B) * lot_size, s))
lock_after  = Up(quote(remaining_after(B)  * lot_size, s))
debit       = lock_before - lock_after              (taken from B's locked quote)
credit      = Down(quote(b, s))                     (added to A's available quote)
residue     = debit - credit                        (added to the protocol dust account)
```

`remaining_before(B) = remaining_after(B) + f`. The buyer's locked quote is reduced by `debit`,
and the buyer's `locked` field is updated to `lock_after`.

**Residue is never negative.** For real numbers `x, y >= 0`, `ceil(x + y) >= ceil(x) + floor(y)`.
Set `x = quote(remaining_after(B) * lot_size, s)` and `y = quote(b, s)`. Then `ceil(x) = lock_after`,
`floor(y) = credit`, and `ceil(x + y) = lock_before`. So `residue >= 0`.

Because each debit is a difference of consecutive locks, the debits of a bid plus the release of its
final lock (on cancel or expiry) add up to the lock it took in section 5.2. No fill can debit more
than the bid has locked.

### 7.3 Ask-side accounting

An ask's lock is exact base atoms, so its fill reduces `locked` by `b` and no quote rounding applies
on the base side. Its quote proceeds are the credit in section 7.2.

## 8. Cancellation and expiry

### 8.1 Cancellation

The owner of an order MAY cancel it at any time, with the owner's authorization. Cancellation:

- removes the order from the book,
- releases its remaining lock to the owner's available balance (quote for a bid, base for an ask),
- does not reuse the order's `id`,
- has no effect on any other order.

An order is cancelled entirely or not at all. No partial cancellation exists in v1.

### 8.2 Expiry

A GTD order is **expired** once the current ledger sequence is greater than or equal to its
`expiry_ledger`. Expiry is evaluated lazily:

- matching removes expired makers it encounters (section 6.8),
- any caller MAY remove an expired order directly. The order's lock is released to its owner, not to
  the caller.

Removing an expired order emits an event, and the removal does not change any fill.

---

## 9. Invariants

The following invariants MUST hold after every successful transaction. A transaction that would
violate one MUST revert.

| ID  | Invariant                                                                                                                                                                                                                                      | Planned test category                                                                                             |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| I1  | **Asset conservation.** For each token, the sum of every account's available and locked balances, plus the protocol dust account and any fee account, is unchanged by trading. Deposits and withdrawals change it by exactly the amount moved. | Property tests over random sequences of placements, fills, cancels, and expiries, checked after each operation.   |
| I2  | **Solvency.** The contract's token balance is at least the sum of all account totals and the protocol dust account.                                                                                                                            | Solvency check in every integration test, plus malicious-token and reentrancy tests.                              |
| I3  | **Lock consistency.** Each order's `locked` equals the value given in section 5.2. Each account's locked balance equals the sum of its orders' locks, and `available + locked = total` with no negative balance.                               | Unit tests for each operation, and property tests that recompute locks from scratch.                              |
| I4  | **No crossed resting book.** When both sides have orders, the best bid tick is strictly less than the best ask tick.                                                                                                                           | Adversarial sequences that include bounded stops and self-trade stops, checked after each transaction.            |
| I5  | **Price-time priority.** A fill never happens at a worse tick than an available better tick. Within a tick, fills happen in ascending `id`.                                                                                                    | Ordering tests with equal-price orders inserted in varying sequences.                                             |
| I6  | **Execution at maker price.** Every fill's tick equals the maker's tick.                                                                                                                                                                       | Positive tests with price improvement on the taker side.                                                          |
| I7  | **Sequence uniqueness.** Order `id`s are strictly increasing per market and never reused, including after cancels and bounded stops.                                                                                                           | Sequence tests across mixed placements and cancels.                                                               |
| I8  | **No zero-size orders.** No stored order has `remaining_lots = 0`.                                                                                                                                                                             | Post-condition assertions in matching and cancel tests.                                                           |
| I9  | **Bounded work.** A transaction performs at most `MAX_MATCH_STEPS` steps (section 10). No operation iterates over an unbounded collection.                                                                                                     | Worst-case fixtures that exhaust the budget, and resource benchmarks once the harness exists.                     |
| I10 | **Residue bounds.** Each fill's rounding residue lies in `[0, 1)` quote atoms, and the protocol dust account never decreases.                                                                                                                  | Property tests over random ticks, tick sizes, and sizes, including the boundaries of `MAX_TICK` and maximum lots. |
| I11 | **Expired orders do not fill.** No fill involves a GTD order whose `expiry_ledger` is at or before the current ledger.                                                                                                                         | Tests that advance the ledger across an expiry boundary.                                                          |
| I12 | **No self-trade.** No fill occurs between two orders with the same owner.                                                                                                                                                                      | Self-trade fixtures on both sides of the book.                                                                    |

## 10. Bounded matching

### 10.1 Work budget

Every transaction has a budget of `MAX_MATCH_STEPS` steps. Steps are charged as follows:

- each fill costs one step,
- each expired maker removed during matching costs one step,
- each price level examined while searching for the next populated level costs one step per tick
  examined.

The implementation MUST find the next populated level in bounded time. It MAY use an ordered index
or a bitmap, and any scan over empty ticks is charged against the same budget.

### 10.2 Bounded stop

If the budget is exhausted while crossing liquidity still exists:

1. Matching stops. The fills completed so far are final.
2. The taker's remainder is **not** rested. Resting it would cross the book and violate I4.
3. The taker's remainder lock is released to the owner's available balance.
4. A `BoundedStop` event is emitted with the taker's filled and released amounts.

The stop is safe. A bounded stop leaves the book uncrossed, because the only order that could cross
was the taker, and the taker is never rested.

A bounded stop is not a failure. The trader's transaction succeeds with a partial result. To
complete the order, the trader places a new order for the remaining size. The new order gets a new
`id` (I7). It follows the same priority rules, so it does not gain priority over orders that were
already resting.

### 10.3 Determinism

Given the same book state, ledger sequence, and transaction, matching produces the same fills, the
same stop reason, and the same residues. Nothing else in the environment affects the result.

### 10.4 Worked examples

The examples below use `lot_size = 1`, so lots and base atoms are equal. They use
`tick_size = PRICE_SCALE`, so a tick `t` is a price of `t` quote atoms per base atom. The budget
`MAX_MATCH_STEPS` is set to 2 for these examples only. The value of the real constant is set by the
benchmark harness.

**Example A: bounded stop with partial consumption.**

Resting state at tick 10, in FIFO order: ask `id 1` for 2 lots, ask `id 2` for 2 lots, ask `id 3`
for 2 lots. There are no bids.

A trader places bid `id 4` at tick 10 for 6 lots.

| Step | Action                                 | Taker remaining | Taker lock | Debit          | Credit to seller    | Residue |
| ---- | -------------------------------------- | --------------- | ---------- | -------------- | ------------------- | ------- |
| Lock | Lock `Up(6 * 10) = 60`                 | 6               | 60         |                |                     |         |
| 1    | Fill `id 1`, 2 lots at tick 10         | 4               | `40`       | `60 - 40 = 20` | `Down(2 * 10) = 20` | 0       |
| 2    | Fill `id 2`, 2 lots at tick 10         | 2               | `20`       | `40 - 20 = 20` | `Down(2 * 10) = 20` | 0       |
| Stop | Budget exhausted, `id 3` still crosses | 2               | released   | `20` released  |                     |         |

Result: 4 lots filled at tick 10. The trader pays 40 quote, receives 4 base, and gets 20 quote
released. Ask `id 3` (2 lots) still rests at tick 10. No bid rests, so the book is not crossed.

The trader resubmits the remaining 2 lots as bid `id 5` at tick 10. It locks 20 and fills `id 3`
completely. Its remainder is zero, so nothing rests.

**Example B: rounding residue on a single fill.**

Set `tick_size = 150_000_000`, so the scaled price for tick 3 is `s = 3 * 150_000_000`, a price of
`4.5` quote atoms per base atom. Resting ask `id 1` is 3 lots at tick 3. A bid `id 2` at tick 3 for
3 lots arrives.

- Lock: `quote = 3 * 4.5 = 13.5`, so `Up(13.5) = 14`.
- Fill of 3 lots at tick 3: `lock_before = 14`, `lock_after = Up(0) = 0`, `debit = 14`.
- Seller credit: `Down(13.5) = 13`.
- Residue: `14 - 13 = 1`, credited to the protocol dust account.

Conservation holds: the buyer's quote falls by 14, the seller gains 13, and the dust account gains 1.

**Example C: the same bid split across two fills.**

Same market as Example B. Resting asks at tick 3, in FIFO order: `id 1` for 1 lot, `id 2` for
2 lots. A bid `id 3` at tick 3 for 3 lots arrives. It locks 14.

| Fill   | Lots | Lock before | Lock after        | Debit | Credit          | Residue |
| ------ | ---- | ----------- | ----------------- | ----- | --------------- | ------- |
| `id 1` | 1    | 14          | `Up(2 * 4.5) = 9` | `5`   | `Down(4.5) = 4` | 1       |
| `id 2` | 2    | 9           | `0`               | `9`   | `Down(9) = 9`   | 0       |

Totals: the buyer is debited 14, the seller is credited 13, and the dust account gains 1. This
matches Example B, so the split does not change the outcome. That is the point of the rule in
section 7.2: rounding depends only on the locks, not on how the fills are split.

**Example D: self-trade stop.**

Trader X rests ask `id 1` at tick 10 for 2 lots. Trader X then places bid `id 2` at tick 10 for
2 lots. The first crossing maker belongs to X, so matching stops with a self-trade stop:

- no fill is produced, and `id 1` is untouched,
- the bid's lock of `Up(2 * 10) = 20` is released,
- the bid is not rested.

## 11. Open decisions

These choices are made in this document, so they are binding for v1. Each one changes behavior and
should be confirmed in review before implementation starts.

1. **Self-trade policy.** v1 uses cancel-newest (section 6.9). The alternative is cancel-oldest,
   which would cancel the resting order and continue matching.
2. **Bounded-stop recovery.** v1 releases the taker's remainder and relies on the trader to
   resubmit (section 10.2). The alternative is a permissionless continuation call that resumes
   matching in a later transaction.
3. **Residue destination.** v1 credits the rounding residue to the protocol dust account (section
   7.2). The alternative is to refund it to the buyer, which would require a per-fill refund
   transfer.
4. **`MAX_MATCH_STEPS` and `MAX_TICK`.** Both are implementation constants. Their values are set by
   the benchmark harness and the contract's resource limits, not by this document.

## 12. Governance and resource-bound constants (Phase 0, #013)

All resource-bound constants and market parameters are defined in `clob_settlement_core/src/lib.rs` and validated through runtime guards. This section documents the governance policy for these constants.

### 12.1 Consensus-critical constants

The following constants are consensus-critical. Every contract and SDK implementation must use identical definitions. Changes to these constants require protocol governance.

- `PRICE_SCALE = 10^8`: Fixed-point denominator for scaled prices (section 4.1). Immutable, set globally.
- `MAX_MATCH_STEPS`: Maximum work budget per transaction (section 10). Set by benchmark harness.
- `MAX_TICK`: Maximum valid tick index (section 4.1). Set by benchmark harness.

All constants are implemented as `pub const` in `clob_settlement_core/src/lib.rs` and must be referenced directly by contract and SDK code, never recomputed.

### 12.2 Bounded-work constants

The following constants bound work in matching, batch operations, and queries. They prevent unbounded loops and ensure Soroban resource exhaustion cannot occur.

**Matching bounds:**

- `MAX_MATCHES_PER_CALL = 100` (placeholder): Maximum fills per matching call. Each fill is O(1) work. When exhausted, matching stops and the client must resubmit the remainder as a new order. Rationale: Soroban transactions have finite CPU and ledger write budgets; fills dominate the work.
- `MAX_LEVELS_CROSSED_PER_CALL = 100` (placeholder): Maximum price levels examined during one matching traversal. When exhausted, matching stops at the next non-empty level and client must resubmit. Rationale: Finding the next non-empty level on a sparse book requires scanning empty ticks. Without a bound, an adversarial order book could force quadratic time. Implementation must use an ordered index or bitmap to scan levels in O(1) per level.

**Batch and query bounds:**

- `MAX_BATCH_SIZE = 50` (placeholder): Maximum orders per batch operation (e.g., batch cancel, batch place). Batch operations are not atomic; exceeding the limit requires multiple transactions. Rationale: Each order operation (create, cancel, expiry) requires ledger reads/writes.
- `MAX_DEPTH_QUERY_SIZE = 100` (placeholder): Maximum orders returned in a depth query (market data snapshot). Clients paginate through large books. Rationale: Depth queries scan the book; bounding them keeps query costs predictable.
- `MAX_ORDER_PAYLOAD_BYTES = 256` (placeholder): Maximum size of a single order's encoding (owner, side, tick, size, tif, etc.). Rationale: Soroban transactions have a hard size limit. Bounding per-order size ensures reasonable batch sizes fit within transaction budgets.

### 12.3 Runtime guards

The following functions validate resource-bound parameters at runtime:

- `validate_tick(tick: u64)`: Rejects ticks that are zero or exceed `MAX_TICK`. Called when an order is placed.
- `validate_lot_size(lot_size: u64)`: Rejects zero lot size. Called at market creation.
- `validate_tick_size(tick_size: u64)`: Rejects zero tick size. Called at market creation.
- `validate_match_steps(steps: u32)`: Rejects step counts exceeding `MAX_MATCH_STEPS`. Called during matching budget checks.
- `validate_matches_per_call(count: u32)`: Rejects fill counts exceeding `MAX_MATCHES_PER_CALL`. Called during matching.
- `validate_levels_crossed_per_call(count: u32)`: Rejects level counts exceeding `MAX_LEVELS_CROSSED_PER_CALL`. Called during book traversal.
- `validate_batch_size(size: u32)`: Rejects batch sizes exceeding `MAX_BATCH_SIZE`. Called at batch operation acceptance.
- `validate_depth_query_size(size: u32)`: Rejects query sizes exceeding `MAX_DEPTH_QUERY_SIZE`. Called when returning market data.
- `validate_order_payload_size(size: u32)`: Rejects order payloads exceeding `MAX_ORDER_PAYLOAD_BYTES`. Called at order creation.

Guards use compile-time constants for bounds, ensuring no unbounded configuration is possible. Violations are rejected with deterministic error messages before any state change.

### 12.4 Continuation and recovery

When a bound is exceeded, matching and batch operations **do not fail**. Instead:

- **Matching:** Stops deterministically at the bound. Already-completed fills are final. Client must resubmit a new order for the remainder.
- **Batch:** Stops after processing as many orders as possible up to the bound. Processed orders are final; client retries unprocessed orders.
- **Queries:** Return results up to the bound. Client uses continuation tokens to fetch the next page.

This design ensures bounded resource use without sacrificing liveness or completeness.

### 12.5 Trusted configuration and verification

Market creation consumes trusted configuration (lot size, tick size) from governance. These values must be validated at market creation and immutable thereafter. No caller-supplied infrastructure addresses are used; all resource budgets are fixed at the protocol level.

- Positive-path tests verify that valid values within bounds are accepted by all guards.
- Negative-path tests verify that out-of-bounds and invalid values are rejected with appropriate errors.
- See `clob_settlement_core/src/lib.rs` tests modules `resource_bound_tests` and `bounded_work_tests` for the full suite.
