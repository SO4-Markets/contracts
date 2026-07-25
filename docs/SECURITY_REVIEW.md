# SECURITY REVIEW: Upgrade Admin Risk and Role Store

This security review provides an in-depth audit of the admin key custody, role access control, upgrade authority chain, and the single-transaction-drain risk profile for the **SO4.market** protocol. 

---

## 1. Executive Summary

The SO4.market protocol utilizes a robust, permissioned access-control design governed by the `role_store` contract. However, the concentration of administrative privileges presents high-value targets for key compromise. 

This audit reveals a **critical divergence** between the protocol documentation (`README.md`) and the Rust source code:
* **Documentation Claim:** Older docs implied broad protocol upgradeability.
* **Rust Reality:** Seven contracts currently implement the `upgrade` function in Rust: `deposit_handler`, `order_handler`, `liquidation_handler`, `fee_handler`, `referral_storage`, `reader`, and `exchange_router`.
* **Security Finding:** Core infrastructure and vault contracts—including `role_store`, `data_store`, `oracle`, and the custodial vaults—are **functionally immutable**. This is a favorable security characteristic because it limits the Wasm upgrade attack vector away from the raw storage and custody layer.

To mitigate administrative key compromise and single-point-of-failure risks, this report details operational and design practices, including **Stellar's native account-level multi-signature system**, while providing a rationale for deferring complex on-chain timelock smart contracts.

---

## 2. Access Control & Role Store Privileges

Access control is managed globally by `role_store/src/lib.rs`. There are three primary tiers of authorization:

### 2.1 The `ROLE_ADMIN` Privilege
* **Ownership:** Granted to the initial `admin` address at deployment during `role_store::initialize()`.
* **Capability:** Can call `grant_role` and `revoke_role` for **any** role, including granting `ROLE_ADMIN` to new addresses.
* **Revocation Guard:** The contract implements an explicit safety check to prevent revoking the last remaining `ROLE_ADMIN` holder:
  ```rust
  if role == admin_role {
      let members = env.storage().persistent().get(&RoleKey::RoleMembers(admin_role.clone()));
      if members.len() <= 1 {
          panic_with_error!(&env, Error::LastAdmin);
      }
  }
  ```
  *Verdict:* This successfully prevents administrative self-lockout.

### 2.2 The `CONTROLLER` Privilege
* **Ownership:** Granted to other protocol smart contracts (e.g. handlers) by `ROLE_ADMIN`.
* **Capability:** The `CONTROLLER` role is the primary gatekeeper for the shared state database `data_store`. A controller can overwrite any value in `data_store` (e.g., prices, configurations, pool balances) and withdraw from custodians.

### 2.3 Keeper Roles
* **`ORDER_KEEPER` / `MARKET_KEEPER`:** Authorised to execute or cancel pending deposits, withdrawals, and orders.
* **`LIQUIDATION_KEEPER` / `ADL_KEEPER`:** Authorised to forcibly close under-collateralised positions or deleverage profitable positions.
* **`FEE_KEEPER`:** Authorised to claim accumulated fees.

---

## 3. Upgrade Authority Chain & Immutability Analysis

Wasm upgrades in Soroban are executed using `env.deployer().update_current_contract_wasm(new_wasm_hash)`.

### 3.1 The Stored Admin Mismatch
In upgradeable contracts, the upgrade authorization is gated by an `admin: Address` stored in the contract's local **instance storage** (`InstanceKey::Admin`), which is initialized during deployment.
```rust
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    let admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
    admin.require_auth();
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}
```
* **Decoupled Auth:** The handler's upgrade authority is completely separate from `role_store` roles. It relies entirely on the direct signature of the stored `admin` address.
* **No `set_admin` Endpoint:** There is no function inside the handler contracts to update `InstanceKey::Admin` after initialization. If the private key corresponding to `InstanceKey::Admin` is lost, the contract is locked forever against upgrades.

### 3.2 Audit of Upgrade Implementations
A workspace-wide code audit confirms that only the contracts listed below implement Wasm upgrade capabilities:

| Contract | Implements `pub fn upgrade` | Upgrade Authority | State Impact |
|---|---|---|---|
| `order_handler` | **✅ Yes** | Local `Admin` Address | High (Position storage, trade execution) |
| `deposit_handler` | **✅ Yes** | Local `Admin` Address | Medium (LP minting, deposit queues) |
| `liquidation_handler` | **✅ Yes** | Local `Admin` Address | Medium (Liquidation routing) |
| `fee_handler` | **✅ Yes** | Local `Admin` Address | Medium (Fee claiming/config) |
| `referral_storage` | **✅ Yes** | Local `Admin` Address | Medium (Referral mappings and tiers) |
| `reader` | **✅ Yes** | Local `Admin` Address | Low (View aggregation) |
| `exchange_router` | **✅ Yes** | Local `Admin` Address | High (User entrypoint and pause control) |
| `role_store` | ❌ No (Immutable) | — | None (Cannot swap ACL rules) |
| `data_store` | ❌ No (Immutable) | — | None (Cannot swap database KV structure) |
| `oracle` | ❌ No (Immutable) | — | None (Price feeds are stable) |
| `market_factory` | ❌ No (Immutable) | — | None |
| `deposit_vault` | ❌ No (Immutable) | — | None (Custodian vault is immutable) |
| `withdrawal_vault`| ❌ No (Immutable) | — | None (Custodian vault is immutable) |
| `order_vault` | ❌ No (Immutable) | — | None (Custodian vault is immutable) |
| `withdrawal_handler`| ❌ No (Immutable) | — | None |
| `adl_handler` | ❌ No (Immutable) | — | None |

> [!TIP]
> **Audit Conclusion on Immutability:**
> Leaving vault and core storage contracts immutable is an industry best-practice. It guarantees that funds stored in `deposit_vault`, `withdrawal_vault`, and `order_vault` cannot be stolen via a malicious Wasm upgrade, even if the handler's admin key is compromised, because the vault contracts themselves can never be upgraded to expose raw transfer functions.

---

## 4. Compromised Admin Threat Scenarios (Single-Transaction Drain)

### 4.1 Scenario A: Compromised `ROLE_ADMIN` Private Key
If an attacker compromises the private key of the `ROLE_ADMIN` account, they can perform a complete protocol takeover in a single ledger block:
1. **Grant Controller:** The attacker grants `CONTROLLER` role to their own malicious contract address using `role_store::grant_role`.
2. **Database Takeover:** Since `data_store` allows any `CONTROLLER` to write arbitrary values, the attacker uses `set_u128` or `set_address` to overwrite:
   * Pool balances (`pool_amount_key`): Artificially reducing it to block withdrawals or increasing it to distort LP token value.
   * Vault addresses or market tokens.
3. **Execute Poisoned Operations:** They can grant keeper roles to their own bot addresses to execute arbitrates, force liquidations of healthy positions, or sweep fees.

### 4.2 Scenario B: Compromised Handler `Admin` Key
If the private key of the `Admin` address stored inside `order_handler` is compromised:
1. **Malicious Wasm Swap:** The attacker uploads a malicious Wasm bytecode blob containing a backdoor that bypasses collateral verification or lets them write arbitrary positions.
2. **Wasm Upgrade:** The attacker calls `order_handler::upgrade` with the malicious Wasm hash.
3. **Vault & Pool Drain:** Since `order_handler` holds `CONTROLLER` privileges on `role_store` (granted during deploy), the backdoored `order_handler` can call `transfer_out` directly on `order_vault` or withdraw funds from market pools, completely draining the funds in a single transaction.

---

## 5. Mitigation Strategy: Native Stellar Multisig

Rather than deploying complex, custom multisig smart contracts (which introduce an additional unaudited threat surface and high CPU gas consumption), SO4.market utilizes **Stellar's native, protocol-level multi-signature account model**.

### 5.1 Stellar native multi-signature design
Every Stellar account can have multiple signing keys with custom weights and operation thresholds:
* **Low Threshold:** Used for minor operations (e.g. `set_prices` for keepers).
* **Medium Threshold:** Used for standard transactions (e.g. trade execution, LP actions).
* **High Threshold:** Used for administrative operations (e.g. `grant_role` or Wasm `upgrade`).

### 5.2 Recommended Production Configuration
For mainnet deployments, both the `ROLE_ADMIN` address (in `role_store`) and the local `Admin` address (in handlers) should be configured as Stellar multisig accounts with a **3-of-5 threshold configuration**:

* **Signers:** Five independent hardware security modules (HSMs) or cold keys.
* **Weights:** Each key is assigned a weight of `1`.
* **High Threshold Setting:** Set to `3`. Any administrative transaction (such as a contract upgrade or a role change) will automatically revert on-chain unless signed by at least 3 of the 5 keys.

This is native to Stellar’s transaction envelopes. When a contract executes `admin.require_auth()`, the Soroban host environment automatically validates that the transaction signature gathers sufficient weight, providing institutional-grade multisig with **zero smart contract gas overhead**.

---

## 6. Timelock Analysis & Rationale for Deferral

An on-chain timelock requires a delayed execution queue where administrative proposals (e.g. upgrading a contract or changing a fee parameter) must be registered and wait for a cooldown period (e.g., 48 hours) before execution, allowing LPs to withdraw if they disagree.

### Rationale for Deferring On-Chain Timelock Smart Contracts:
1. **Native Stellar Threshold Capabilities:** A 3-of-5 cold-multisig configuration provides sufficient operational delay and security guarantees for launch.
2. **CPU & Memory Limitations:** Soroban transactions are constrained by strict CPU instruction and memory limits. Passing every administrative write through an intermediary timelock contract significantly increases transaction complexity and gas costs.
3. **Contract Size Limit:** Implementing a complete proposal, voting, queuing, and execution governance system in Rust easily exceeds the 64KB Wasm contract size limit without major structural partitioning.
4. **Development Deferral:** Operational timelocks (such as off-chain multi-sig signing policies that require a 48-hour review period before signing) can be enforced legally and operationally at launch, deferring the development of complex on-chain timelock wrappers to a future governance phase.

---

## 7. LP Mint/Burn and Pool Value Arithmetic (Issue #148)

A security review of the LP pricing, pool value, and redemption math in `libs/market_utils`, `deposit_handler`, and `withdrawal_handler` was conducted to identify manipulation vectors.

### 7.1 First-Deposit LP Pricing
* **Observation:** In `get_market_token_price`, if `total_supply <= 0`, the price defaults to `FLOAT_PRECISION` ($1 per LP token).
* **Security Posture:** Safe. Because pool reserves are tracked internally via `DataStore` (`pool_amount_key`) rather than by querying the raw `balance()` of the token contract, an attacker **cannot** execute the classic Uniswap V2 "first depositor" attack (where a user deposits 1 wei, donates 1,000,000 tokens to the contract, and heavily inflates the LP share price). Direct token donations to the vault do not inflate `pool_value`.

### 7.2 Pool Value Calculation and PnL Cap
* **Observation:** `get_pool_value` calculates `pool_value = long_usd + short_usd + impact_pool_usd - net_pnl`. There is currently no explicit fraction cap on `net_pnl`.
* **Edge Case / Nonstandard Behavior:** If aggregate trader PnL is overwhelmingly positive and exceeds total pool assets, `pool_value` becomes `<= 0`. `get_market_token_price` handles this by resetting the LP price to `FLOAT_PRECISION` ($1). 
* **Assumption Documented:** If a market is completely bankrupted by winning traders, the LP token price resets to $1 rather than converging to zero. Subsequent deposits into the bankrupted pool will mint LP tokens at the fresh $1 rate. While this prevents divide-by-zero panics, market creators should be aware that a bankrupt pool effectively "reboots" its LP pricing.

### 7.3 LP Redemption Math
* **Observation:** In `execute_withdrawal`, the payout is calculated pro-rata: `long_out = pool_amount * lp_amount / total_supply`.
* **Security Posture:** Safe. The withdrawal math relies strictly on the internal `pool_amount` ledger rather than the actual vault balance. This ensures that LPs can only withdraw their fair share of the actively tracked pool reserves, ignoring any externally airdropped tokens that might sit in the vault. Furthermore, the `min_long_token_amount` and `min_short_token_amount` slippage parameters rigorously enforce user-defined minimums, preventing keeper front-running during withdrawals.
