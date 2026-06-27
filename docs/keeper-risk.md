# Keeper Front-Running Risk Assessment

**Issue:** [#296](https://github.com/SO4-Markets/contracts/issues/296)  
**Scope:** Discretionary execution power of ORDER_KEEPER, LIQUIDATION_KEEPER, and ADL_KEEPER roles and the risks that arise from selective or delayed order execution.

---

## Background

SO4.market uses a **two-step execution model**: users create requests on-chain (no price embedded), and a permissioned keeper later executes them with a fresh oracle price. This eliminates the classic "keeper sees the user's limit price and front-runs at a better rate" attack. However, keepers still retain discretion over **which** pending requests to execute and **when**.

---

## Risk 1 — Order Execution Ordering

**Description:** A keeper observes multiple pending orders and chooses to execute the profitable subset first (e.g., a long-increase when the keeper knows the price will rise, or a liquidation when the position has become maximally underwater).

**Severity:** LOW  
**Likelihood:** LOW

**Why low?**
- Every order has an `acceptable_price` set by the user at creation time. An increase order reverts if the execution price is worse than `acceptable_price`; a decrease order similarly. A keeper cannot profitably reorder fills to extract value because the user's price floor/ceiling is enforced on-chain.
- Oracle prices are ledger-scoped: the keeper submits prices in the same ledger as execution, so the price is fixed at submission time — the keeper cannot choose a historical favorable price.
- The keeper does not earn a position-proportional reward; it earns a flat `execution_fee` set by the user, which cannot be arbitrarily increased by reordering.

**Recommended Mitigation (implemented):** `acceptable_price` on all order types is the primary guard. No FIFO queue is enforced on-chain because Soroban transactions are commutative under fixed oracle prices; adding an on-chain queue would increase ledger entry cost without meaningful additional protection.

**Residual risk:** A keeper could delay a user's limit order until the limit condition changes (e.g., the price moves back above a limit-buy's trigger). This is bounded by the user's ability to cancel and re-submit.

---

## Risk 2 — Liquidation Cherry-Picking

**Description:** A LIQUIDATION_KEEPER delays liquidating a position until it accumulates maximum bad debt or until it can claim the maximum liquidation execution fee.

**Severity:** MEDIUM  
**Likelihood:** LOW

**Why medium severity?**
- Delayed liquidation of an underwater position can cause bad debt to accumulate in the pool, socialising losses to LPs.
- The liquidation keeper's fee is a flat `execution_fee` embedded in the liquidation order, not proportional to position size, so there is limited direct financial incentive to delay (the keeper earns the same fee whether it acts early or late).
- The `min_collateral_factor` guard sets the threshold below which positions become liquidatable. Positions are not liquidatable until they cross this threshold, so early keepers have no advantage over later keepers within the window.

**Mitigations in place:**
1. Multiple-keeper competition: anyone granted `LIQUIDATION_KEEPER` can liquidate any eligible position. If one keeper is slow or malicious, others will act to earn the fee.
2. `liquidate_position` reverts if the position is not actually underwater (`health_check` must fail). A keeper cannot liquidate a healthy position regardless of intent.
3. The execution fee is user-set and fixed, providing no extra marginal revenue for delayed execution.

**Recommended Additional Mitigation:**
- Off-chain monitoring (see Keeper Monitoring section below) should alert operators when any position's collateral ratio crosses 110% of the liquidation threshold for more than N ledgers without execution.
- Consider a keeper heartbeat timeout: if no liquidation is executed within a configurable window, the LIQUIDATION_KEEPER role can be revoked and re-assigned. The `last_keeper_activity` key in DataStore already tracks keeper activity for ORDER_KEEPER.

---

## Risk 3 — ADL Selection

**Description:** An ADL_KEEPER selects which profitable positions to partially close for Auto-Deleveraging, potentially preferring positions held by the keeper's own accounts.

**Severity:** MEDIUM  
**Likelihood:** LOW

**Why medium severity?**
- ADL forces partial closure of profitable positions when the insurance fund is insufficient. Targeting specific accounts (e.g., competitors) is a form of economic censorship.
- ADL selection is currently keeper-discretionary: `execute_adl` accepts an arbitrary `account`, `market`, `collateral_token`, and `is_long`. There is no on-chain enforcement that the "most profitable" position is selected first.

**Mitigations in place:**
1. ADL is only possible when the ADL condition is met (protocol checks open interest vs. reserve); keepers cannot trigger ADL on a healthy market.
2. `adl_handler` verifies `adl_conditions_are_met` before calling `order_handler.execute_adl`; the selection is constrained to positions that actually need ADL.
3. Multiple ADL keepers can act; a biased keeper's self-serving selection will be corrected when another keeper selects the largest profitable position.

**Recommended Additional Mitigation:**
- Publish an ADL selection algorithm as part of the off-chain keeper specification so the community can verify conformance.
- Emit an `adl_exe` event with the selected position's key and size; off-chain monitors can verify the largest eligible position was chosen.

---

## Risk 4 — Deposit/Withdrawal Timing

**Description:** A keeper delays executing a withdrawal until the pool loses value (reducing the LP token value and thus the amount received), or delays a deposit until the pool gains value (reducing the LP tokens minted).

**Severity:** LOW  
**Likelihood:** VERY LOW

**Why low?**
- Both `execute_deposit` and `execute_withdrawal` use the **current oracle price** at execution time. The output (LP tokens minted, pool tokens received) is calculated at execution time, not at creation time.
- A delayed withdrawal results in the withdrawer receiving *current* pool value, not a stale committed value. If the pool drops, the withdrawer loses value — this is normal LP market risk, not keeper manipulation, because any rational withdrawer can cancel and re-submit.
- Withdrawal slippage protection: `min_long_token_amount` and `min_short_token_amount` on withdrawals cause reverts if the output is below user-specified minimums. A keeper delaying until the pool shrinks would simply cause the withdrawal to revert, and the user can cancel.
- Deposit slippage protection: `min_market_tokens` on deposits causes reverts if the minted LP is below a minimum.

**Recommended Mitigation (already implemented):** User-specified slippage parameters on all requests. No additional on-chain change is needed; education of LP users about setting appropriate min amounts is the primary lever.

---

## Summary

| Risk | Severity | Likelihood | Primary Mitigation |
|------|----------|------------|-------------------|
| Order execution reordering | Low | Low | `acceptable_price` enforced on-chain |
| Liquidation cherry-picking / delay | Medium | Low | Multiple-keeper competition; flat fee structure |
| ADL position selection bias | Medium | Low | Multiple ADL keepers; ADL condition check |
| Deposit/withdrawal timing | Low | Very Low | User-set slippage minima; cancellable requests |

---

## Implemented Mitigations Summary

| Mitigation | Status | Where |
|-----------|--------|-------|
| `acceptable_price` on all orders | ✅ Implemented | `order_handler::execute_order` |
| `min_market_tokens` on deposits | ✅ Implemented | `deposit_handler::execute_deposit` |
| `min_long/short_token_amount` on withdrawals | ✅ Implemented | `withdrawal_handler::execute_withdrawal` |
| `min_output_amount` on swap orders | ✅ Implemented | `order_handler::execute_order` (swap path) |
| Multiple keepers supported (role-based, not singleton) | ✅ Implemented | `role_store` — any account with the role can act |
| Position health check before liquidation | ✅ Implemented | `liquidation_handler::liquidate_position` |
| ADL condition check before ADL execution | ✅ Implemented | `adl_handler` |
| Keeper activity monitoring key | ✅ Implemented | `last_keeper_activity_key` in `gmx_keys` |
| Circuit breaker / keeper heartbeat timeout | ✅ Implemented | `keeper_heartbeat_timeout_key`, `last_keeper_activity_key` in `order_handler` |

---

## Keeper Monitoring Guide

Operators running or auditing keepers should monitor the following:

### Order Keeper
- **Pending orders queue depth**: alert if > N orders are unexecuted for > M ledgers.
- **Cancelled vs executed ratio**: a high cancellation rate on limit orders may indicate price staleness or keeper avoidance of certain order types.
- **`last_keeper_activity` key in DataStore**: if the last activity ledger is > `keeper_heartbeat_timeout` ledgers ago, the keeper is considered inactive and orders can revert with `KeeperInactive`.

### Liquidation Keeper
- **Positions near the liquidation threshold**: monitor positions with collateral ratio ≤ 120% of `min_collateral_factor`; alert if any such position goes unexecuted for > 10 ledgers.
- **Insurance fund balance**: track `insurance_fund_balance_key` in DataStore; a falling balance with active ADL is a sign of sustained bad debt.

### ADL Keeper
- **ADL condition flag**: check `is_adl_enabled_key` per market; when `true`, the keeper should execute ADL within 2–3 ledgers.
- **Largest profitable position**: off-chain keepers should always target the position with the highest PnL first to minimise total ADL events.

### General
- **Keeper role revocation**: build tooling to detect when a keeper's wallet has been compromised and revoke via `role_store.revoke_role` before damage is done.
- **Transaction simulation before broadcast**: simulate every keeper execution with Soroban RPC `simulateTransaction` and skip submission if the simulation fails, to avoid burning fees on reverts.
