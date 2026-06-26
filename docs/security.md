# Sandwich Attack Resistance

This document analyses the sandwich attack surface of SO4.market's order execution model and describes the layered mitigations already present in the codebase.

---

## Threat Model

A **sandwich attack** occurs when an adversary:

1. Observes a pending transaction (e.g., a market increase order in the mempool).
2. Frontruns it with a trade that moves the price in a direction that hurts the victim.
3. Executes the victim's transaction at the degraded price.
4. Backtracks immediately to capture the spread.

On Stellar, transactions are not ordered by gas price — the network uses a parallel throughput model with ledger-close ordering. This removes the classic Ethereum MEV frontrunning vector. However, a malicious or colluding **keeper** controlling the order-execution queue faces an analogous risk: by selectively executing orders out of order (or pairing them with price oracle updates), it could manufacture artificial slippage against users.

The primary adversarial assumptions for SO4.market are:

- **Adversarial keeper**: a keeper that can choose *when* to submit execution transactions within a valid oracle freshness window.
- **Stale oracle data**: a keeper that submits a price outdated enough to be disadvantageous but still within tolerance.
- **Position cherry-picking**: a keeper that skips unprofitable-to-execute orders and batches favourable ones.

---

## Attack Vectors and Mitigations

### 1. Price Manipulation via `acceptable_price`

**Vector**: A keeper executes a `MarketIncrease` order after the index price has moved against the user, filling them at a worse price than they expected.

**Mitigation**: Every order stores an `acceptable_price` set by the user at creation time. `execute_order` enforces this bound at execution:

- For increase orders the execution price must not exceed `acceptable_price` (long) or must be at least `acceptable_price` (short).
- The check is performed inside `increase_position` / `get_execution_price` in `libs/pricing_utils/src/lib.rs`, which panics with `PriceTooHigh` or `PriceTooLow` if the bound is violated.

**Effect**: The user defines their own maximum slippage tolerance. No keeper action can fill an order at a price the user explicitly rejected. This is the primary sandwich defence.

---

### 2. Price Impact Model

**Vector**: A keeper submits a large order just before the victim's order to shift the effective pool price.

**Mitigation**: SO4.market implements a price impact model (see `docs/price-impact.md` and `libs/pricing_utils/src/lib.rs`) that:

- Charges a positive impact fee to trades that increase pool imbalance (e.g., adding to an already-dominant long side).
- Pays a negative impact (rebate) to trades that restore balance.
- Stores accumulated fees in a price-impact pool that accrues to LPs, not to the manipulating keeper.

**Effect**: Any attacker who frontruns with an imbalancing trade pays a price-impact penalty proportional to the imbalance they created. This makes sandwiching economically costly relative to the gain from slippage.

---

### 3. Execution Fee Disincentive (issue #294)

**Vector**: A keeper selectively executes only orders where the execution fee exactly covers the keeper's cost, skipping orders with a low fee. This is not a sandwich attack directly, but allows a keeper to time executions opportunistically.

**Mitigation**: Issue #294 introduces a configurable global minimum execution fee validated at `create_order` time. Orders that do not meet the minimum are rejected at creation — they never enter the execution queue. This prevents users from accidentally submitting underpaid orders that could be held and executed at a strategically chosen moment.

**Effect**: All live orders in the queue carry at least the protocol minimum fee. Keepers cannot exploit the fee delta to cherry-pick timing.

---

### 4. Keeper Liveness Monitoring (issue #249)

**Vector**: A keeper withholds execution of orders until a favourable oracle price window opens, effectively timing the fill.

**Mitigation**: `execute_order` calls `record_keeper_activity` after every successful execution, stamping the `ORDER_KEEPER` role's last-activity timestamp. `flag_stale_keeper` in `order_handler` can be called by any participant if the heartbeat has not been updated within `keeper_heartbeat_timeout`. A stale keeper can be replaced via the role management flow in `role_store`.

**Effect**: Prolonged order withholding is observable on-chain and triggers an automatic staleness flag, enabling keeper rotation.

---

## Risk Acceptance

The combination of `acceptable_price` slippage bounds, price impact fees, minimum execution fee enforcement, and keeper liveness monitoring collectively limits the attack surface to within user-defined tolerances. No additional on-chain code is required at this time.

Residual risk: a colluding oracle and keeper pair could submit a stale-but-valid price update simultaneously with a targeted execution. This is mitigated at the oracle level by the circuit breaker and price deviation limits documented in `docs/oracle-risk.md`.

---

## Related Docs

- `docs/oracle-risk.md` — oracle price deviation limits and circuit breaker
- `docs/price-impact.md` — price impact fee model
- `docs/SECURITY_REVIEW.md` — admin key custody and upgrade authority audit
- `docs/keeper-execution-flow.md` — full keeper lifecycle
