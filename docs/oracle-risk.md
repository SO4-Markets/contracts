# Oracle Compromise Risk: Maximum Loss in One Ledger

> Issue #250 — risk-modelling task. This document quantifies the worst-case loss
> if the oracle signer key is compromised for a single ledger, and evaluates
> mitigations with their cost and complexity. It is a modelling document, not a
> code change.

---

## 1. Threat Model

The `oracle` contract is a **single point of failure**. Prices are submitted by a
keeper holding `ORDER_KEEPER` and are verified against a registered ed25519
public key (`oracle::set_prices`) or, in the simplified path, accepted directly
(`oracle::set_prices_simple`). Either way, the **integrity of every market price
reduces to the secrecy of one signer key**.

If that key is compromised, the attacker can submit arbitrary `(min, max)` prices
for any token for the duration of their control. The relevant question for
treasury and circuit-breaker design is: **how much value can leave the protocol
in a single ledger before any human can respond?**

Assumptions used throughout:

| Symbol | Meaning |
| ------ | ------- |
| `L`    | Total open **long** open interest across all markets (USD) |
| `S`    | Total open **short** open interest across all markets (USD) |
| `P`    | Total pool value (TVL) across all markets (USD) |
| `C`    | Total trader collateral backing open positions (USD) |

All positions are marked to the attacker-supplied price within the same ledger,
so the attacker both **sets the price** and **closes positions at that price**.

---

## 2. Worst-Case Single-Ledger Attack

### 2.1 Profit extraction via price manipulation then position close

The protocol realises trader PnL out of the pool. For a long position:

```
pnl_usd = size_in_tokens × (close_price − entry_price)
```

`get_position_pnl_usd` (in `position_utils`) computes this directly from
`size_in_tokens` and the oracle price, and `decrease_position` pays it out of the
market pool. There is **no per-position cap on favourable price movement** — the
only ceiling is the pool itself and the `MAX_PNL_FACTOR_FOR_TRADERS` clamp applied
to *pool value*, not to an individual close.

Attack sequence (single ledger):

1. Open (or already hold via a colluding account) a long position of size `L`.
2. Set the index price to an arbitrarily large value (e.g. ETH = 1,000,000 USD).
3. Close the position. PnL is bounded only by what the pool can pay.

**Maximum extracted this way:**

```
steal_price ≈ min( P , L × manipulation_factor )
```

Because `manipulation_factor` is unbounded (the attacker controls the price), the
binding constraint is **pool depth `P`**. In the limit the attacker drains the
entire pool: `steal_price → P`.

> The mirror attack works on shorts by setting the price to near-zero
> (`min` price → 1, the smallest value `set_prices_simple` accepts after the
> `tp.min > 0` guard), giving shorts unbounded PnL. Net: **either direction
> drains `P`.**

### 2.2 Profit extraction via forced liquidations

`is_liquidatable` (in `position_utils`) compares remaining collateral against the
maintenance requirement at the oracle price. By moving the price against open
positions, the attacker can mark **all opposing OI** as liquidatable in one
ledger.

- Forcing liquidation of longs: set price low → up to `L` of positions liquidated.
- Forcing liquidation of shorts: set price high → up to `S` of positions liquidated.

Liquidation seizes remaining collateral into the pool, so this path **harms
traders** (loss bounded by their collateral `C`) rather than the LP pool. The
attacker profits only if they hold the opposing side and close into the move, in
which case 2.1 already captures the gain. The independent liquidation damage to
honest traders is bounded by:

```
liquidation_loss ≤ C
```

### 2.3 Combined upper bound

```
max_single_ledger_loss ≤ P + C
```

- **`P`** — the entire LP pool, extractable via §2.1 (price manipulation + close).
- **`C`** — honest-trader collateral, seizable via §2.2 (forced liquidation).

In practice `P` dominates: the pool is the deep, liquid target, and §2.1 alone is
sufficient to drain it. **Treat `P` (total TVL) as the headline single-ledger
loss figure.** `get_protocol_stats` (issue #251) reports `P` directly as
`total_pool_value_usd`, which is the number a circuit breaker should threshold
against.

---

## 3. Pool Drain via Price Manipulation (worked example)

Setting ETH to 1,000,000 USD when the true price is 2,000 USD (a 500× move):

- Every long position has effectively **infinite** PnL relative to its collateral.
- `decrease_position` pays PnL out of the pool until the pool is exhausted.
- The attacker extracts `min(total_pool_value, long_oi × price_manipulation_factor)`.
- With an unconstrained `price_manipulation_factor`, the result collapses to
  `total_pool_value` — **the pool is fully drained.**

The only existing brake is `MAX_PNL_FACTOR_FOR_TRADERS`, which caps *total*
recognised trader PnL as a fraction of pool value during pool-value computation.
It limits how much of the pool is attributed to PnL in a single valuation, but it
does **not** reject an out-of-band price, so a determined attacker re-running the
close across markets or ledgers still converges on draining `P`.

---

## 4. Recommended Mitigations

Evaluated against cost and complexity. Ordered by recommended priority.

### 4.1 Price deviation cap / circuit breaker — **highest priority** (see #203)

Reject any submitted price that deviates from the last accepted price by more than
a configured percentage (e.g. ±10% per ledger).

- **Effect:** Hard-caps the per-ledger `manipulation_factor`. A 500× move becomes
  impossible; the most an attacker can shift price is the cap, bounding §2.1 loss
  to roughly `L × deviation_cap` rather than `P`.
- **Cost:** Low. One stored "last price + timestamp" per token and a comparison in
  `set_prices`. No new contracts.
- **Complexity:** Low. Needs a sane bootstrap (first price, and a path for
  legitimate large gaps after downtime — likely an admin-gated override).
- **Trade-off:** Can stall execution during genuine high-volatility moves; pair
  with an admin reset.

### 4.2 Multi-signer median oracle — **high value, higher cost** (see #280)

Require *m-of-n* signers and take the median submitted price.

- **Effect:** Raises attack cost from one compromised key to a quorum (e.g. 2-of-3).
  A single leaked key can no longer move price at all.
- **Cost:** Medium. Store *n* public keys, verify *m* signatures, compute a median.
- **Complexity:** Medium. Signer-set rotation, liveness if a signer is down, and
  agreement on rounding for the median.

### 4.3 Time-delayed price execution — **medium**

Apply submitted prices only after a 1-ledger (or N-second) delay.

- **Effect:** Opens a human-response window between a malicious submission and its
  use, so a watchtower can pause the protocol (`global_pause_key`) before the bad
  price executes.
- **Cost:** Low-medium. A pending-price buffer and an activation check.
- **Complexity:** Medium. Adds latency to *all* execution and needs careful
  interaction with order trigger checks (a 1-ledger-old price changes fill logic).

### 4.4 Per-market oracle isolation — **defence in depth**

Use a distinct signer per market (or per token), so compromising one market's
signer does not affect others.

- **Effect:** Caps blast radius to a single market's `P_market` instead of total `P`.
- **Cost:** Medium. Key-management overhead scales with market count.
- **Complexity:** Low on-chain (key lookup is already per-token-capable), higher
  operationally.

---

## 5. Summary

| Metric | Value |
| ------ | ----- |
| Worst-case single-ledger loss | `≈ P` (total pool value), up to `P + C` including trader collateral |
| Binding constraint | **Pool depth `P`**, not OI |
| Cheapest effective mitigation | Price deviation cap (#203) |
| Strongest mitigation | Multi-signer median oracle (#280) |
| Recommended stack | Deviation cap **+** multi-signer, with per-market isolation as defence-in-depth |

The single most impactful change is the **price deviation cap (#203)**: it is
cheap, requires no new contracts, and directly converts the unbounded §2.1 drain
into a small per-ledger bound. The **multi-signer median oracle (#280)** then
removes the single-key root cause. A circuit breaker should threshold against
`ProtocolStats.total_pool_value_usd` from the reader (#251), since that is the
quantity an attacker is ultimately trying to remove.
