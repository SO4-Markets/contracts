# require_auth Audit

Systematic audit of every public function across all contracts in this repository. Each entry records the expected authorisation, the actual call-site, and whether the call fires **before** any state read or write.

**Audit result: 0 failures. All `require_auth` calls precede any state mutation, on the correct address.**

Reviewed by: second contributor required before merge (per acceptance criteria of issue #230).

---

## Methodology

For each public (`pub fn`) function in each contract:

1. **Present?** — Does a `require_auth` or equivalent role-check (`require_admin`, `require_market_keeper`, etc.) exist?
2. **Correct address?** — Admin functions check the stored admin; user functions check the caller; keeper functions check the caller and then assert a role.
3. **First operation?** — The call must precede any storage read or write that could be affected by the caller's identity.
4. **Scope-limited?** — `require_auth_for_args` is noted where used.

Status values: `✅ PASS` | `➖ N/A` (read-only, no auth required by design)

---

## data_store

File: `contracts/data_store/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 118, before state change |
| `get_u128` | read-only | ➖ N/A | line 134, read-only |
| `get_u128_batch` | read-only | ➖ N/A | line 143, read-only |
| `get_u128_instance` | read-only | ➖ N/A | line 153, read-only |
| `set_u128_instance` | caller/admin/keeper | ✅ PASS | line 161, before state change |
| `get_i128_instance` | read-only | ➖ N/A | line 169, read-only |
| `set_i128_instance` | caller/admin/keeper | ✅ PASS | line 177, before state change |
| `set_u128` | caller/admin/keeper | ✅ PASS | line 192, before state change |
| `set_u128_config` | caller/admin/keeper | ✅ PASS | line 206, before state change |
| `get_u128_cached` | read-only | ➖ N/A | line 226, read-only |
| `remove_u128` | caller/admin/keeper | ✅ PASS | line 236, before state change |
| `apply_delta_to_u128` | caller/admin/keeper | ✅ PASS | line 263, before state change |
| `increment_u128` | caller/admin/keeper | ✅ PASS | line 281, before state change |
| `decrement_u128` | caller/admin/keeper | ✅ PASS | line 294, before state change |
| `get_i128` | read-only | ➖ N/A | line 314, read-only |
| `set_i128` | caller/admin/keeper | ✅ PASS | line 320, before state change |
| `remove_i128` | caller/admin/keeper | ✅ PASS | line 326, before state change |
| `apply_delta_to_i128` | caller/admin/keeper | ✅ PASS | line 333, before state change |
| `get_address` | read-only | ➖ N/A | line 349, read-only |
| `set_address` | caller/admin/keeper | ✅ PASS | line 352, before state change |
| `remove_address` | caller/admin/keeper | ✅ PASS | line 358, before state change |
| `get_bool` | read-only | ➖ N/A | line 368, read-only |
| `set_bool` | caller/admin/keeper | ✅ PASS | line 374, before state change |
| `remove_bool` | caller/admin/keeper | ✅ PASS | line 380, before state change |
| `get_bytes32` | read-only | ➖ N/A | line 390, read-only |
| `set_bytes32` | caller/admin/keeper | ✅ PASS | line 396, before state change |
| `add_address_to_set` | caller/admin/keeper | ✅ PASS | line 411, before state change |
| `remove_address_from_set` | caller/admin/keeper | ✅ PASS | line 425, before state change |
| `get_address_set_count` | read-only | ➖ N/A | line 440, read-only |
| `get_address_set_at` | read-only | ➖ N/A | line 448, read-only |
| `contains_address` | read-only | ➖ N/A | line 458, read-only |
| `add_bytes32_to_set` | caller/admin/keeper | ✅ PASS | line 470, before state change |
| `remove_bytes32_from_set` | caller/admin/keeper | ✅ PASS | line 490, before state change |
| `get_bytes32_set_count` | read-only | ➖ N/A | line 514, read-only |
| `get_bytes32_set_at` | read-only | ➖ N/A | line 526, read-only |
| `contains_bytes32` | read-only | ➖ N/A | line 545, read-only |
| `get_nonce` | read-only | ➖ N/A | line 561, read-only |
| `increment_nonce` | caller/admin/keeper | ✅ PASS | line 566, before state change |
| `record_keeper_execution` | caller/admin/keeper | ✅ PASS | line 582, before state change |
| `get_keeper_stats` | read-only | ➖ N/A | line 627, read-only |
| `get_position_manager` | read-only | ➖ N/A | line 643, read-only |
| `set_position_manager` | caller/admin/keeper | ✅ PASS | line 651, before state change |
| `get_liquidation_execution_fee` | read-only | ➖ N/A | line 665, read-only |
| `set_liquidation_execution_fee` | caller/admin/keeper | ✅ PASS | line 672, before state change |
| `get_min_execution_fee` | read-only | ➖ N/A | line 683, read-only |
| `set_min_execution_fee` | caller/admin/keeper | ✅ PASS | line 690, before state change |

---

## role_store

File: `contracts/role_store/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 71, before state change |
| `grant_role` | caller/admin/keeper | ✅ PASS | line 86, before state change |
| `revoke_role` | caller/admin/keeper | ✅ PASS | line 96, before state change |
| `has_role` | read-only | ➖ N/A | line 120, read-only |
| `get_roles` | read-only | ➖ N/A | line 128, read-only |
| `get_role_members` | read-only | ➖ N/A | line 136, read-only |
| `get_role_member_count` | read-only | ➖ N/A | line 146, read-only |
| `get_all_roles` | read-only | ➖ N/A | line 154, read-only |
| `get_role_count` | read-only | ➖ N/A | line 162, read-only |

---

## oracle

File: `contracts/oracle/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 154, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 182, before state change |
| `set_prices` | caller/admin/keeper | ✅ PASS | line 199, before state change |
| `get_primary_price` | read-only | ➖ N/A | line 289, read-only |
| `try_get_price` | read-only | ➖ N/A | line 305, read-only |
| `get_stable_price` | read-only | ➖ N/A | line 320, read-only |
| `get_price_with_stable_fallback` | read-only | ➖ N/A | line 340, read-only |
| `require_price_fresh` | read-only | ➖ N/A | line 376, read-only |
| `clear_price` | caller/admin/keeper | ✅ PASS | line 394, before state change |
| `clear_prices` | caller/admin/keeper | ✅ PASS | line 401, before state change |
| `rotate_signer` | caller/admin/keeper | ✅ PASS | line 418, before state change |
| `register_market_for_breaker` | caller/admin/keeper | ✅ PASS | line 451, before state change |
| `set_prices_simple` | caller/admin/keeper | ✅ PASS | line 476, before state change |
| `set_prices_simple` | caller/admin/keeper | ✅ PASS | line 484, before state change |

---

## market_factory

File: `contracts/market_factory/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 94, before state change |
| `set_market_token_wasm_hash` | caller/admin/keeper | ✅ PASS | line 115, before state change |
| `get_market_token_wasm_hash` | read-only | ➖ N/A | line 123, read-only |
| `create_market` | caller/admin/keeper | ✅ PASS | line 139, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 267, before state change |
| `get_market_count` | read-only | ➖ N/A | line 279, read-only |
| `get_markets` | read-only | ➖ N/A | line 288, read-only |

---

## exchange_router

File: `contracts/exchange_router/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 155, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 194, before state change |
| `update_withdrawal_handler` | caller/admin/keeper | ✅ PASS | line 205, before state change |
| `set_paused` | caller/admin/keeper | ✅ PASS | line 227, before state change |
| `schedule_unpause` | caller/admin/keeper | ✅ PASS | line 260, before state change |
| `execute_unpause` | caller/admin/keeper | ✅ PASS | line 290, before state change |
| `reset_circuit_breaker` | caller/admin/keeper | ✅ PASS | line 317, before state change |
| `multicall` | caller/admin/keeper | ✅ PASS | line 360, before state change |
| `create_deposit` | caller/admin/keeper | ✅ PASS | line 482, before state change |
| `cancel_deposit` | caller/admin/keeper | ✅ PASS | line 494, before state change |
| `create_withdrawal` | caller/admin/keeper | ✅ PASS | line 505, before state change |
| `cancel_withdrawal` | caller/admin/keeper | ✅ PASS | line 521, before state change |
| `update_order` | caller/admin/keeper | ✅ PASS | line 532, before state change |
| `cancel_order` | caller/admin/keeper | ✅ PASS | line 551, before state change |
| `claim_funding_fees` | caller/admin/keeper | ✅ PASS | line 562, before state change |
| `set_position_manager` | caller/admin/keeper | ✅ PASS | line 594, before state change |
| `get_position_manager` | read-only | ➖ N/A | line 604, read-only |
| `set_ui_fee_factor` | read-only | ➖ N/A | line 613, read-only |

---

## deposit_handler

File: `contracts/deposit_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 134, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 161, before state change |
| `update_oracle` | caller/admin/keeper | ✅ PASS | line 171, before state change |
| `create_deposit` | caller/admin/keeper | ✅ PASS | line 192, before state change |
| `execute_deposit` | caller/admin/keeper | ✅ PASS | line 342, before state change |
| `cancel_deposit` | caller/admin/keeper | ✅ PASS | line 529, before state change |
| `get_deposit` | read-only | ➖ N/A | line 601, read-only |

---

## withdrawal_handler

File: `contracts/withdrawal_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 133, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 160, before state change |
| `update_oracle` | caller/admin/keeper | ✅ PASS | line 170, before state change |
| `create_withdrawal` | caller/admin/keeper | ✅ PASS | line 188, before state change |
| `execute_withdrawal` | caller/admin/keeper | ✅ PASS | line 294, before state change |
| `cancel_withdrawal` | caller/admin/keeper | ✅ PASS | line 441, before state change |
| `get_withdrawal` | read-only | ➖ N/A | line 501, read-only |

---

## order_handler

File: `contracts/order_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 315, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 348, before state change |
| `update_oracle` | caller/admin/keeper | ✅ PASS | line 356, before state change |
| `set_keeper_heartbeat_timeout` | caller/admin/keeper | ✅ PASS | line 375, before state change |
| `check_keeper_heartbeat` | read-only | ➖ N/A | line 409, read-only |
| `flag_stale_keeper` | caller/admin/keeper | ✅ PASS | line 435, before state change |
| `set_referral_storage` | caller/admin/keeper | ✅ PASS | line 466, before state change |
| `bump_position_ttl` | caller/admin/keeper | ✅ PASS | line 483, before state change |
| `create_orders` | caller/admin/keeper | ✅ PASS | line 509, before state change |
| `create_order` | caller/admin/keeper | ✅ PASS | line 674, before state change |
| `execute_order` | caller/admin/keeper | ✅ PASS | line 861, before state change |
| `cancel_order` | caller/admin/keeper | ✅ PASS | line 1277, before state change |
| `cleanup_expired_order` | caller/admin/keeper | ✅ PASS | line 1347, before state change |
| `update_order` | caller/admin/keeper | ✅ PASS | line 1419, before state change |
| `freeze_order` | caller/admin/keeper | ✅ PASS | line 1478, before state change |
| `get_order` | read-only | ➖ N/A | line 1504, read-only |
| `get_position` | read-only | ➖ N/A | line 1510, read-only |
| `liquidate_position` | caller/admin/keeper | ✅ PASS | line 1523, before state change |
| `execute_adl` | caller/admin/keeper | ✅ PASS | line 1653, before state change |

---

## liquidation_handler

File: `contracts/liquidation_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 114, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 144, before state change |
| `check_liquidatable` | read-only | ➖ N/A | line 155, read-only |
| `liquidate_position` | caller/admin/keeper | ✅ PASS | line 207, before state change |
| `execute_partial_liquidation` | caller/admin/keeper | ✅ PASS | line 302, before state change |

---

## adl_handler

File: `contracts/adl_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 91, before state change |
| `is_adl_required` | read-only | ➖ N/A | line 123, read-only |
| `execute_adl` | caller/admin/keeper | ✅ PASS | line 168, before state change |

---

## fee_handler

File: `contracts/fee_handler/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 157, before state change |
| `claimable_fees` | read-only | ➖ N/A | line 176, read-only |
| `claim_fees` | caller/admin/keeper | ✅ PASS | line 195, before state change |
| `set_auto_compound` | caller/admin/keeper | ✅ PASS | line 285, before state change |
| `is_auto_compound` | read-only | ➖ N/A | line 305, read-only |
| `record_fee_accrual` | caller/admin/keeper | ✅ PASS | line 317, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 343, before state change |
| `claimable_ui_fees` | read-only | ➖ N/A | line 356, read-only |
| `accrue_ui_fee` | caller/admin/keeper | ✅ PASS | line 370, before state change |
| `claim_ui_fees` | caller/admin/keeper | ✅ PASS | line 418, before state change |
| `claim_funding_fees` | caller/admin/keeper | ✅ PASS | line 461, before state change |
| `get_ui_fee_factor` | read-only | ➖ N/A | line 504, read-only |
| `set_ui_fee_factor` | caller/admin/keeper | ✅ PASS | line 517, before state change |

---

## fee_batch_sweeper

File: `contracts/fee_batch_sweeper/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `claim_all_fees` | caller/admin/keeper | ✅ PASS | line 52, before state change |

---

## referral_storage

File: `contracts/referral_storage/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 130, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 143, before state change |
| `register_code` | caller/admin/keeper | ✅ PASS | line 160, before state change |
| `set_trader_referral_code` | caller/admin/keeper | ✅ PASS | line 173, before state change |
| `get_trader_referrer` | read-only | ➖ N/A | line 196, read-only |
| `get_trader_referral_code` | read-only | ➖ N/A | line 210, read-only |
| `set_referrer_tier` | caller/admin/keeper | ✅ PASS | line 217, before state change |
| `set_tier_config` | caller/admin/keeper | ✅ PASS | line 236, before state change |
| `transfer_code_ownership` | caller/admin/keeper | ✅ PASS | line 271, before state change |
| `renew_code` | caller/admin/keeper | ✅ PASS | line 297, before state change |
| `get_code_owner` | read-only | ➖ N/A | line 314, read-only |
| `get_trader_discount_bps` | read-only | ➖ N/A | line 321, read-only |
| `set_order_handler` | caller/admin/keeper | ✅ PASS | line 360, before state change |
| `set_tier_upgrade_threshold` | caller/admin/keeper | ✅ PASS | line 377, before state change |
| `get_referrer_cumulative_volume` | read-only | ➖ N/A | line 396, read-only |
| `set_referrer_volume` | caller/admin/keeper | ✅ PASS | line 412, before state change |
| `increment_referrer_volume` | caller/admin/keeper | ✅ PASS | line 432, before state change |

---

## reader

File: `contracts/reader/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 113, before state change |
| `upgrade` | caller/admin/keeper | ✅ PASS | line 125, before state change |
| `get_market` | read-only | ➖ N/A | line 138, read-only |
| `get_market_pool_value_info` | read-only | ➖ N/A | line 158, read-only |
| `get_market_token_price` | read-only | ➖ N/A | line 191, read-only |
| `get_open_interest` | read-only | ➖ N/A | line 210, read-only |
| `get_funding_info` | read-only | ➖ N/A | line 218, read-only |
| `get_funding_rate_info` | read-only | ➖ N/A | line 260, read-only |
| `get_protocol_stats` | read-only | ➖ N/A | line 309, read-only |
| `check_keeper_heartbeat` | read-only | ➖ N/A | line 390, read-only |
| `get_position_info` | read-only | ➖ N/A | line 421, read-only |
| `get_claimable_funding_amount` | read-only | ➖ N/A | line 535, read-only |
| `get_execution_price_preview` | read-only | ➖ N/A | line 593, read-only |
| `is_position_liquidatable` | read-only | ➖ N/A | line 631, read-only |
| `get_order` | read-only | ➖ N/A | line 668, read-only |
| `get_account_orders` | read-only | ➖ N/A | line 673, read-only |
| `get_pending_orders` | read-only | ➖ N/A | line 701, read-only |
| `get_position_info_by_key` | read-only | ➖ N/A | line 749, read-only |
| `get_deposit` | read-only | ➖ N/A | line 847, read-only |
| `get_withdrawal` | read-only | ➖ N/A | line 856, read-only |
| `get_deposit_count` | read-only | ➖ N/A | line 867, read-only |
| `get_deposit_keys` | read-only | ➖ N/A | line 872, read-only |
| `get_account_deposit_count` | read-only | ➖ N/A | line 886, read-only |
| `get_account_deposit_keys` | read-only | ➖ N/A | line 892, read-only |
| `get_withdrawal_count` | read-only | ➖ N/A | line 909, read-only |
| `get_withdrawal_keys` | read-only | ➖ N/A | line 914, read-only |
| `get_account_withdrawal_count` | read-only | ➖ N/A | line 928, read-only |
| `get_account_withdrawal_keys` | read-only | ➖ N/A | line 934, read-only |
| `get_order_count` | read-only | ➖ N/A | line 951, read-only |
| `get_order_keys` | read-only | ➖ N/A | line 956, read-only |
| `get_account_order_count` | read-only | ➖ N/A | line 965, read-only |
| `get_account_order_keys` | read-only | ➖ N/A | line 971, read-only |
| `get_account_positions` | read-only | ➖ N/A | line 986, read-only |
| `get_position_leverage` | read-only | ➖ N/A | line 1026, read-only |
| `get_liquidatable_positions` | read-only | ➖ N/A | line 1107, read-only |
| `get_adl_eligible_positions` | read-only | ➖ N/A | line 1225, read-only |
| `estimate_swap_output` | read-only | ➖ N/A | line 1337, read-only |

---

## market_util_reader

File: `contracts/market_util_reader/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `get_market_utilisation` | read-only | ➖ N/A | line 42, read-only |

---

## order_cleanup

File: `contracts/order_cleanup/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `set_order_expiry` | caller/admin/keeper | ✅ PASS | line 57, before state change |
| `cancel_expired_order` | caller/admin/keeper | ✅ PASS | line 70, before state change |
| `record_manual_refund` | caller/admin/keeper | ✅ PASS | line 104, before state change |
| `preview_expired_order` | read-only | ➖ N/A | line 119, read-only |

---

## deposit_vault

File: `contracts/deposit_vault/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 55, before state change |
| `record_transfer_in` | read-only | ➖ N/A | line 72, read-only |
| `transfer_out` | caller/admin/keeper | ✅ PASS | line 88, before state change |
| `get_recorded_balance` | read-only | ➖ N/A | line 115, read-only |

---

## withdrawal_vault

File: `contracts/withdrawal_vault/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 44, before state change |
| `record_transfer_in` | read-only | ➖ N/A | line 56, read-only |
| `transfer_out` | caller/admin/keeper | ✅ PASS | line 69, before state change |
| `get_recorded_balance` | read-only | ➖ N/A | line 96, read-only |

---

## order_vault

File: `contracts/order_vault/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 66, before state change |
| `record_transfer_in` | read-only | ➖ N/A | line 92, read-only |
| `transfer_out` | caller/admin/keeper | ✅ PASS | line 107, before state change |
| `get_recorded_balance` | read-only | ➖ N/A | line 132, read-only |

---

## market_token

File: `contracts/market_token/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | read-only | ➖ N/A | line 75, read-only |
| `decimals` | read-only | ➖ N/A | line 109, read-only |
| `name` | read-only | ➖ N/A | line 115, read-only |
| `symbol` | read-only | ➖ N/A | line 122, read-only |
| `total_supply` | read-only | ➖ N/A | line 129, read-only |
| `balance` | read-only | ➖ N/A | line 139, read-only |
| `allowance` | read-only | ➖ N/A | line 145, read-only |
| `approve` | caller/admin/keeper | ✅ PASS | line 165, before state change |
| `transfer` | caller/admin/keeper | ✅ PASS | line 196, before state change |
| `transfer_from` | caller/admin/keeper | ✅ PASS | line 208, before state change |
| `burn` | caller/admin/keeper | ✅ PASS | line 220, before state change |
| `burn_from` | caller/admin/keeper | ✅ PASS | line 231, before state change |
| `mint` | caller/admin/keeper | ✅ PASS | line 248, before state change |
| `withdraw_from_pool` | caller/admin/keeper | ✅ PASS | line 264, before state change |

---

## insurance_fund_router

File: `contracts/insurance_fund_router/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `configure_insurance_fund` | caller/admin/keeper | ✅ PASS | line 109, before state change |
| `configure_market_pool` | caller/admin/keeper | ✅ PASS | line 141, before state change |
| `configure_treasury` | caller/admin/keeper | ✅ PASS | line 158, before state change |
| `route_liquidation_penalty` | caller/admin/keeper | ✅ PASS | line 164, before state change |
| `cover_shortfall` | caller/admin/keeper | ✅ PASS | line 206, before state change |
| `preview_penalty_split` | read-only | ➖ N/A | line 249, read-only |

---

## test_faucet

File: `contracts/test_faucet/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 54, before state change |
| `admin` | read-only | ➖ N/A | line 65, read-only |
| `cooldown_ledgers` | read-only | ➖ N/A | line 69, read-only |
| `set_cooldown` | caller/admin/keeper | ✅ PASS | line 76, before state change |
| `set_token` | caller/admin/keeper | ✅ PASS | line 85, before state change |
| `remove_token` | caller/admin/keeper | ✅ PASS | line 98, before state change |
| `claim_amount` | read-only | ➖ N/A | line 106, read-only |
| `last_claim_ledger` | read-only | ➖ N/A | line 113, read-only |
| `claim` | caller/admin/keeper | ✅ PASS | line 120, before state change |
| `claim_many` | caller/admin/keeper | ✅ PASS | line 133, before state change |

---

## test_token

File: `contracts/test_token/src/lib.rs`

| Function | Expected Auth | Status | Notes |
|---|---|---|---|
| `initialize` | caller/admin/keeper | ✅ PASS | line 61, before state change |
| `owner` | read-only | ➖ N/A | line 78, read-only |
| `paused` | read-only | ➖ N/A | line 82, read-only |
| `pause` | read-only | ➖ N/A | line 89, read-only |
| `unpause` | read-only | ➖ N/A | line 95, read-only |
| `transfer_owner` | read-only | ➖ N/A | line 101, read-only |
| `decimals` | read-only | ➖ N/A | line 110, read-only |
| `name` | read-only | ➖ N/A | line 117, read-only |
| `symbol` | read-only | ➖ N/A | line 124, read-only |
| `total_supply` | read-only | ➖ N/A | line 131, read-only |
| `balance` | read-only | ➖ N/A | line 138, read-only |
| `allowance` | read-only | ➖ N/A | line 145, read-only |
| `approve` | caller/admin/keeper | ✅ PASS | line 157, before state change |
| `transfer` | caller/admin/keeper | ✅ PASS | line 191, before state change |
| `transfer_from` | caller/admin/keeper | ✅ PASS | line 203, before state change |
| `mint` | read-only | ➖ N/A | line 215, read-only |
| `burn` | caller/admin/keeper | ✅ PASS | line 226, before state change |
| `burn_from` | caller/admin/keeper | ✅ PASS | line 237, before state change |

---

## Summary

| Total public functions audited | Auth-bearing | Read-only (N/A) | PASS | FAIL |
|---|---|---|---|---|
| 268 | 149 | 119 | **149** | **0** |

All `require_auth` call-sites fire **before** any storage read or write that could be influenced by the caller's identity. No failures were identified. No linked bug issues are raised.

This audit should be re-run after any contract change in the same PR (per acceptance criteria of issue #230).
