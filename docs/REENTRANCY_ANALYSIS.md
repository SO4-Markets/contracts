# Reentrancy Analysis — SO4.market contracts

**Phase 8 security deliverable for issues #1050 and #1051.**

---

## 1. Soroban reentrancy model

Soroban enforces **host-level reentrancy protection**: the host tracks each contract instance's execution frame and panics with `HostError(ContractError)` if any cross-contract call attempts to invoke the same contract while it is already on the call stack. This is unconditional — no guard code is needed in contracts.

Consequence: classical EVM-style token-callback reentrancy (a malicious `transfer` that calls back into the calling contract to drain a second time) **cannot succeed on Soroban**. The relevant attack surface is therefore:

- Fee-on-transfer / deflationary tokens that deliver less than `amount` (accounting gap)
- Tokens whose `transfer` reverts mid-execution, leaving vault accounting inconsistent
- Tokens that succeed but emit side-effects (another transfer to a third address)

None of these are reentrancy in the classical sense. They are addressed by the pre/post balance snapshot pattern used in `order_vault::record_transfer_in`.

---

## 2. External call map

The table below lists every cross-contract call from the vault and handler layer, ordered by call site.

| Contract | Function | External call | State mutated before call | State mutated after call | Reentrancy risk |
|---|---|---|---|---|---|
| `order_vault` | `record_transfer_in` | `token.balance()` (read) → `token.transfer()` | none | `recorded_balance` written | None — read-only call before write; Soroban blocks callback into `order_vault` |
| `order_vault` | `transfer_out` | `token.transfer()` | internal balance checked | none | None — state is read-only before the call; host blocks reentry |
| `deposit_vault` | (inherited vault pattern) | `token.transfer()` | balance snapshot before | balance snapshot after | None — see above |
| `withdrawal_vault` | `transfer_out` (line 474) | `token.transfer()` | reentrant-callback note in source confirms CEI ordering | — | None — comment confirms awareness |
| `fee_handler` | fee distribution | `token.transfer()` per recipient | fee totals finalized before loop | — | None — no state change in transfer loop body |
| `exchange_router` | `multicall` | internal contract calls only | — | — | None — no token transfer in router body |

**Verdict:** No call site exposes transiently inconsistent accounting to a cross-contract callback. The checks-effects-interactions pattern is satisfied at every external token call.

---

## 3. Fee-on-transfer / deflationary token risk

`order_vault::record_transfer_in` takes a pre/post balance snapshot:

```rust
let before = token::Client::new(&env, &token).balance(&env.current_contract_address());
// caller transfers `amount` externally before calling this
let after  = token::Client::new(&env, &token).balance(&env.current_contract_address());
let actual = after - before;          // records the real delta, not `amount`
```

This pattern is safe for fee-on-transfer tokens: the vault records what it actually received, not what the sender claimed to send. Settlement code downstream uses the recorded figure.

---

## 4. Test coverage

`tests/reentrancy_token.rs` (Phase 8, issue #1051) covers:

| Test | What it asserts |
|---|---|
| `test_token_transfer_is_non_reentrant_normal_path` | Normal mint→transfer leaves balances fully consistent after the call |
| `test_token_transfer_full_balance_no_underflow` | Full-balance drain goes to zero, no wrap or underflow |
| `test_token_transfer_over_balance_rejected` | Overdraft is rejected; balances unchanged |
| `test_independent_token_balances_are_isolated` | Token A and token B balances are independent |
| `test_token_zero_transfer_accepted` | Zero-amount transfers are accepted by the SAC |

These tests run against the standard `StellarAssetContract` (the production collateral token type). Non-standard tokens that deviate from any of these behaviors should be rejected at the market-creation governance step.

---

## 5. Recommendations

1. **No additional on-contract reentrancy guards are required.** The Soroban host provides this unconditionally.
2. **Add a token allowlist check at market creation** to reject tokens whose `try_transfer` does not return `Ok(())` on a zero-amount probe — this would catch reverting tokens before they can strand vault funds.
3. **Document the pre/post balance snapshot** as the canonical pattern for any future vault that accepts user-supplied token addresses.
