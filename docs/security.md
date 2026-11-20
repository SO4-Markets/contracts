# Security Audit: Reentrancy Analysis

**Issue:** [#295](https://github.com/SO4-Markets/contracts/issues/295)  
**Scope:** Checks-Effects-Interactions (CEI) ordering for all handlers that perform external token transfers.

## Background

Soroban cross-contract calls are **synchronous** and **part of the same ledger transaction**. Storage writes made before an external call are visible to any re-entrant callee within that same transaction. A malicious contract receiving tokens could call back into the protocol before cleanup is complete. While all current protocol tokens are admin-controlled, defence-in-depth demands that CEI is enforced regardless.

## Pattern reference

**Safe (CEI):**
1. Validate all inputs
2. Update all internal state (pool amounts, positions, storage cleanup)
3. Perform external token transfer **last**

**Unsafe:**
1. Transfer tokens externally
2. Update state (window for re-entry between steps 1 and 2)

---

## Handler audit

### `deposit_handler::execute_deposit`

| Step | Action | Type |
|------|--------|------|
| 1 | Load deposit record | State read |
| 2 | Query oracle prices | External (read-only) |
| 3 | Check vault balance ≥ recorded amount | External (read-only) |
| 4 | Compute LP mint amount | Arithmetic |
| 5 | `vault.transfer_out` → pool (long token) | External transfer |
| 6 | `apply_delta_to_pool_amount` (long) | State write |
| 7 | `vault.transfer_out` → pool (short token) | External transfer |
| 8 | `apply_delta_to_pool_amount` (short) | State write |
| 9 | `market_token.mint` → receiver | External transfer |
| 10 | `remove_deposit` (delete record + index sets) | State write |

**Finding:** Steps 5–8 interleave external calls with state writes (not strict CEI).

**Mitigation in place:**
- Step 3 (vault balance check) uses `get_recorded_balance` which tracks the vault's internal accounting. After step 5, the vault recorded balance drops to 0, so any re-entrant call to `execute_deposit` with the same key would fail the balance check.
- The deposit record persists through step 10 only; a re-entrant `execute_deposit` with the same key would still find the record but fail the vault balance guard.
- Keepers (`ORDER_KEEPER` role) are trusted — the caller cannot be an arbitrary user.

**Risk:** LOW. The vault balance guard prevents double-spend even under re-entry. Protocol-controlled tokens eliminate the callback vector in practice.

**Recommendation:** For defence-in-depth, move `remove_deposit` (step 10) before the vault transfers (steps 5–7) and update pool amounts before external transfers. Blocked by the DataStore ledger-entry budget; monitor for future re-ordering opportunity.

---

### `withdrawal_handler::execute_withdrawal`

| Step | Action | Type |
|------|--------|------|
| 1 | Load withdrawal record | State read |
| 2 | Compute pro-rata long/short output amounts | Arithmetic |
| 3 | Check output ≥ minimums | Validation |
| 4 | `vault.transfer_out` LP tokens → handler | External transfer |
| 5 | `market_token.burn` LP from handler | External state write |
| 6 | **`remove_withdrawal`** (delete record + index sets) | **State write (CEI fix #295)** |
| 7 | `apply_delta_to_pool_amount` (long, negative) | State write |
| 8 | `market_token.withdraw_from_pool` (long) → receiver | External transfer |
| 9 | `apply_delta_to_pool_amount` (short, negative) | State write |
| 10 | `market_token.withdraw_from_pool` (short) → receiver | External transfer |

**Fix applied (issue #295):** `remove_withdrawal` is now called at step 6, immediately after the LP burn and before any pool transfer to the receiver. A re-entrant callback on the receiving contract will find the withdrawal record gone and panic with `WithdrawalNotFound`, completely closing the re-entry window.

Pool amount state updates (`apply_delta_to_pool_amount`) are also reordered to precede their corresponding `withdraw_from_pool` calls, ensuring all DataStore state is consistent before each external transfer.

**Risk:** NONE — record deleted and pool amounts updated before any tokens leave the pool.

---

### `order_handler::execute_order` — MarketIncrease / LimitIncrease / StopIncrease

| Step | Action | Type |
|------|--------|------|
| 1 | Load order record | State read |
| 2 | Fetch oracle prices | External (read-only) |
| 3 | `vault.transfer_out` collateral → pool | External transfer |
| 4 | `increase_position` (updates DataStore OI, pool amounts, position record) | State write |
| 5 | Remove order (delete record + index sets) | State write |

**Finding:** Collateral transfer (step 3) precedes state updates (step 4). This is not strict CEI.

**Mitigation in place:**
- `vault.record_transfer_in` (called in `create_order`) snapshots the vault delta. The order cannot be created without collateral already in the vault, and the vault transfers to the pool (step 3) deplete the vault before state updates.
- Execution is restricted to `ORDER_KEEPER` — orders cannot be self-executed by users.
- The order record is present until step 5; a re-entrant `execute_order` on the same key would re-enter `increase_position` with already-transferred collateral. The collateral would not be doubled because `record_transfer_in` is only called at order creation.

**Risk:** LOW (keeper-gated + collateral pre-snapshot prevents double-spend).

---

### `order_handler::execute_order` — MarketDecrease / LimitDecrease / StopLossDecrease

| Step | Action | Type |
|------|--------|------|
| 1 | Load order record | State read |
| 2 | Fetch oracle prices | External (read-only) |
| 3 | `decrease_position` (updates position state, pays out PnL via pool transfer) | External + State |
| 4 | Remove order | State write |

**Finding:** Inside `decrease_position`, the PnL payment (external transfer) and state cleanup are interleaved. The order record persists through step 4.

**Mitigation in place:**
- Position state is updated inside `decrease_position` (size reduced / position closed in DataStore) before the external PnL transfer. A re-entrant call via the receiving contract would find the position already reduced, yielding zero or negative PnL — no double-payout.
- Execution is `ORDER_KEEPER` restricted.

**Risk:** LOW. Position state is reduced before the external transfer; re-entry cannot extract more PnL.

---

### `order_handler::execute_order` — MarketSwap / LimitSwap

| Step | Action | Type |
|------|--------|------|
| 1 | Load order record | State read |
| 2 | Fetch oracle prices | External (read-only) |
| 3 | `vault.transfer_out` collateral → first swap market | External transfer |
| 4 | `swap_with_path` (per hop: state updates + pool transfers interleaved) | External + State |
| 5 | Remove order | State write |

**Finding:** Pool state updates and external transfers are interleaved per hop within `swap_with_path`. Swap path length is now validated at order creation time (see issue #300 fix).

**Mitigation:** Duplicate market detection in `swap_with_path` prevents repeated pool mutation. Oracle prices are pre-fetched once before the hop loop (see issue #298 fix), removing oracle as a re-entry vector.

**Risk:** LOW.

---

### `liquidation_handler::liquidate_position`

Delegates to `order_handler::liquidate_position` after health check. No direct token transfers in this handler; all token flows occur inside `order_handler`. Same mitigations as the decrease-order path apply.

**Risk:** LOW.

---

### `fee_handler::claim_fees`

| Step | Action | Type |
|------|--------|------|
| 1 | Resolve the trusted fee recipient (market override, else protocol recipient) | State read |
| 2 | Read claimable amount from DataStore | State read |
| 3 | Compute capped transfer amount | Arithmetic |
| 4 | **Zero out claimable amount in DataStore** | **State write (first)** |
| 5 | `market_token.withdraw_from_pool` → configured recipient | External transfer |

**Finding:** ✅ **Fully CEI compliant.** The claimable balance is zeroed (step 4) before the external transfer (step 5). A re-entrant `claim_fees` call would read zero and return early.

**Destination:** `claim_fees(keeper, market, token)` takes no destination argument, and neither does
`fee_batch_sweeper::claim_all_fees`. Protocol fees are paid only to the recipient held in
`fee_handler`'s own storage:

| Setting | Entrypoint | Auth | Event |
|---|---|---|---|
| Protocol recipient (treasury) | `set_fee_receiver(receiver)` | stored admin | `fee_rcv` `FeeReceiverSet { market: None, previous, receiver }` |
| Per-market override (issuer / partner) | `set_market_fee_receiver(market, Some(receiver) \| None)` | stored admin | `fee_rcv` `FeeReceiverSet { market: Some(market), previous, receiver }` |

Reads: `get_fee_receiver(market)` (the recipient that will actually be paid),
`get_protocol_fee_receiver()`, `get_market_fee_receiver(market)`.

- With no recipient configured, `claim_fees` reverts with `FeeReceiverNotSet` (#7) and fees stay claimable.
- A recipient equal to the market pool or to `fee_handler` is rejected with `InvalidFeeReceiver` (#8),
  both when it is set and when a claim resolves it. Paying the pool itself would decrement
  `pool_amount` while the tokens never leave the pool.
- Accrual is unchanged: settlement credits `claimable_fee_amount(market, token)` in DataStore, a key
  derived from the market only. No order or create-order parameter (`receiver`, `swap_path`,
  `on_behalf_of`, …) influences where protocol fees accrue or where they are paid.
- v1 pays each market to a single recipient. Splits are not supported.

**Shared quote tokens (per-market attribution):** several markets may accrue fees in the same
token (e.g. every `*/USDC` market). The topology is market-scoped for both custody and ledger:

- Custody: each market's fees sit in that market's own `market_token` pool; markets never share a
  custody balance.
- Ledger: fees accrue to `claimable_fee_amount(market, token)` and are backed by
  `pool_amount(market, token)`. Both keys include the market, so two markets sharing a token write
  different entries and cannot overwrite each other. There is no separate token-level fee ledger,
  so the same tokens are never recorded twice.
- Token-level reads and withdrawals aggregate per-market entries rather than replacing them:

| Entrypoint | Auth | Behaviour |
|---|---|---|
| `claimable_fees_by_token(token, markets)` | none (view) | `TokenFeeAttribution { token, total, markets: [MarketFeeAmount { market, amount }] }` |
| `claim_token_fees(keeper, token, markets)` | `FEE_KEEPER` | Claims each market exactly as `claim_fees` does (own pool, own `pool_amount`, own recipient, own `fee_clm` event), then emits `tkfee_clm` `TokenFeesClaimed { token, total, market_count }`. Atomic: one failing market reverts the whole call. |

- Both reject a market listed twice (`DuplicateMarket`, #10), so a market's ledger cannot be counted
  twice in `total`. Both accept at most `MAX_TOKEN_FEE_MARKETS` (20) markets (`TooManyMarkets`, #9),
  and `total` uses checked addition (`FeeOverflow`, #11).
- `reader::get_protocol_stats` rejects duplicate markets for the same reason (`DuplicateMarket`, #6).

**Treasury reconciliation (global total vs sub-ledgers):** `total_claimable_fee_amount(token)` is the
protocol's global fee liability in `token`. It always equals the sum of
`claimable_fee_amount(market, token)` over every market:

- Accrual: every fee path (`increase_position_utils`, `decrease_position_utils`, `swap_utils`,
  `fee_handler::record_fee_accrual`) moves the per-market entry and the global total in the same
  call. The three settlement paths share one implementation,
  `gmx_market_utils::apply_delta_to_claimable_fee_amount`.
- Claims: `claim_fees` and `claim_token_fees` release the transferred amount from the global total
  in the same call as the per-market entry. A claim larger than the global total reverts with
  `FeeLedgerMismatch` (#12), so a per-market entry the global ledger does not cover can never be
  withdrawn.
- Aggregate bound: `claim_token_fees` also checks that its total withdrawal is at most the listed
  markets' summed accrued balances and at most the global total (`ExceedsAccruedFees`, #13).

| Entrypoint | Auth | Behaviour |
|---|---|---|
| `total_claimable_fees(token)` | none (view) | The global total. |
| `reconcile_token_fees(token, markets)` | none (view) | `FeeReconciliation { token, ledger_total, markets_total, reconciled, markets: [MarketFeeReconciliation { market, claimable, pool_amount, custody, backed }] }`. `backed` is `claimable <= pool_amount && claimable <= custody`; `reconciled` holds when every listed market is backed and `markets_total == ledger_total`. Pass every market that accrues `token`: a missing market shows up as `ledger_total > markets_total`. |
| `backfill_fee_total(token, markets)` | stored admin | Upgrade migration: adds each listed `(market, token)` ledger accrued before the global total existed, at most once per pair (`fee_bkfl` `FeeTotalBackfilled { market, token, amount, total }`). |

All three reject duplicate markets and accept at most `MAX_TOKEN_FEE_MARKETS` (20).

**Risk:** NONE — correct CEI ordering; destination is trusted configuration only.

---

## Summary

| Handler | CEI compliant? | Risk | Key mitigation |
|---------|---------------|------|----------------|
| `deposit_handler::execute_deposit` | Partial | Low | Vault balance guard prevents double-spend |
| `withdrawal_handler::execute_withdrawal` | ✅ Fixed (#295) | None | Record deleted + pool amounts updated before any transfer |
| `order_handler::execute_order` (increase) | Partial | Low | Keeper-gated; collateral pre-snapshotted |
| `order_handler::execute_order` (decrease) | Partial | Low | Position state reduced before PnL transfer |
| `order_handler::execute_order` (swap) | Partial | Low | Duplicate market guard; oracle pre-fetched |
| `liquidation_handler::liquidate_position` | — | Low | Delegates to order_handler (same mitigations) |
| `fee_handler::claim_fees` | ✅ Full | None | State zeroed before transfer |

## Outstanding recommendations

1. **`deposit_handler::execute_deposit`** — consider moving `remove_deposit` before vault transfers once the ledger-entry budget allows it.
2. **All handlers** — add an explicit reentrancy guard (`Executing` flag in instance storage) as additional defence when budget permits. Set the flag at function entry; clear at exit.
3. **Ongoing** — any new handler added to the protocol must be reviewed against this document before merging.
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
