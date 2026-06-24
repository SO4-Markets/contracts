# Implementation Plan: SO4 Markets Protocol Architecture

This document defines the architectural decisions, storage layouts, and contract responsibility matrix for the SO4 Markets protocol on Stellar/Soroban.

---

## 1. Canonical Storage Model (Issue #2)

### Decision
All request types (Deposits, Withdrawals, Orders) and Positions live in **handler-local persistent storage** within their respective contracts rather than in the shared `data_store`.

- **Deposits** are stored in the persistent storage of `deposit_handler`.
- **Withdrawals** are stored in the persistent storage of `withdrawal_handler`.
- **Orders** are stored in the persistent storage of `order_handler`.
- **Positions** are stored in the persistent storage of `order_handler`.

### Rationale

1. **Storage Rent (TTL) Isolation**
   Soroban requires contracts to pay rent (TTL) for persistent storage. Placing user-specific transient requests (deposits, withdrawals, orders) and long-lived positions in their local handler contracts isolates their TTL management. It prevents a single global `data_store` from becoming a bottleneck or a source of shared rent eviction risk.

2. **Access Control & Encapsulation**
   Positions and orders represent critical financial states. Storing them inside `order_handler` enforces that only the authorized `order_handler` logic (via `create_order`, `execute_order`, etc.) can mutate position records. If stored in a shared `data_store`, any contract granted the `CONTROLLER` role would have raw write access to position states, increasing the attack surface.

3. **Avoid Serialization Overhead**
   Storing complex Rust structs (like `PositionProps` or `OrderProps`) in a centralized key-value database like `data_store` requires cross-contract serialization/deserialization into raw bytes or maps. Local storage allows direct, type-safe serialization within the contract's own storage namespace, significantly reducing CPU instruction usage and transaction size.

### Implementation Verification
No core contracts store requests or positions in a way that contradicts this decision.
- `deposit_handler` uses `LocalKey::Deposit(key)` in its own persistent storage.
- `withdrawal_handler` uses `LocalKey::Withdrawal(key)` in its own persistent storage.
- `order_handler` uses `OrderStorageKey::Order(key)` and `PositionStorageKey::Position(key)` in its own persistent storage.
- View and settlement contracts (e.g. `liquidation_handler`, `adl_handler`, and `reader`) query the `order_handler` via cross-contract view calls (`get_position`) to read position details rather than querying the `data_store`.

---

## 2. Contract Responsibility Matrix (Issue #4)

Below is the responsibility mapping of every contract under `contracts/*` within the protocol graph.

### Core Storage & Roles

#### `role_store`
- **Description**: Access control registry mapping accounts to roles.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::Initialized`, and persistent `(account, role) -> bool`.
- **Initialization Args**: `admin: Address`
- **Roles Checked**: Admin authorization required for granting/revoking roles.
- **Callers**: All contracts validating roles (via `has_role`).
- **Callees**: None.
- **Emitted Events**: `RoleGranted`, `RoleRevoked` (implied).
- **Upgrade Policy**: Upgradeable (Admin).

#### `data_store`
- **Description**: Universal typed key-value database for market metadata, configs, and parameters.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, and arbitrary `BytesN<32> -> Value` mappings.
- **Initialization Args**: `admin: Address, role_store: Address`
- **Roles Checked**: `CONTROLLER` role required for all state-mutating (write) calls.
- **Callers**: Handlers, factories, and utilities.
- **Callees**: `role_store`.
- **Emitted Events**: None.
- **Upgrade Policy**: Upgradeable (Admin).

#### `oracle`
- **Description**: Keeper-fed oracle that stores and signs token prices per ledger.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, and temporary price records.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, passphrase: Bytes`
- **Roles Checked**: `price_keeper` / `order_keeper` role required to set prices.
- **Callers**: Handlers and readers.
- **Callees**: `role_store`, `data_store`.
- **Emitted Events**: `prices_set` (implied).
- **Upgrade Policy**: Upgradeable (Admin).

---

### Markets & Tokens

#### `market_factory`
- **Description**: Deterministically deploys LP token contracts (`market_token`) and registers them.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::MarketTokenWasmHash`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address`
- **Roles Checked**: `MARKET_KEEPER` required to create markets; Admin required to set WASM hash.
- **Callers**: Keepers/Admin.
- **Callees**: Deploys and initializes `market_token`. Writes market props to `data_store`.
- **Emitted Events**: `wasm_set`, `mkt_new`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `market_token`
- **Description**: SEP-41 compliant LP token representing pool ownership.
- **State Owned**: Decimals, name, symbol, balances, allowances.
- **Initialization Args**: `admin: Address, role_store: Address, decimal: u32, name: String, symbol: String`
- **Roles Checked**: `CONTROLLER` required to mint/burn LP tokens.
- **Callers**: `market_factory`, handlers.
- **Callees**: `role_store`.
- **Emitted Events**: Standard SEP-41 `Transfer`, `Mint`, `Burn`, `Approval`.
- **Upgrade Policy**: Immutable.

---

### Vaults (Custodian Contracts)

#### `deposit_vault`
- **Description**: Holds long/short tokens between deposit creation and execution/cancellation.
- **State Owned**: `InstanceKey::RoleStore`, and persistent `TokenBalance(Address)` snapshots.
- **Initialization Args**: `admin: Address, role_store: Address`
- **Roles Checked**: `CONTROLLER` required for `transfer_out`.
- **Callers**: `deposit_handler`.
- **Callees**: SEP-41 token contracts, `role_store`.
- **Emitted Events**: None.
- **Upgrade Policy**: Immutable.

#### `withdrawal_vault`
- **Description**: Holds LP tokens between withdrawal creation and execution/cancellation.
- **State Owned**: `InstanceKey::RoleStore`, and persistent `TokenBalance(Address)` snapshots.
- **Initialization Args**: `admin: Address, role_store: Address`
- **Roles Checked**: `CONTROLLER` required for `transfer_out`.
- **Callers**: `withdrawal_handler`.
- **Callees**: `market_token`, `role_store`.
- **Emitted Events**: None.
- **Upgrade Policy**: Immutable.

#### `order_vault`
- **Description**: Holds collateral during pending order lifecycle.
- **State Owned**: `InstanceKey::RoleStore`, and persistent `TokenBalance(Address)` snapshots.
- **Initialization Args**: `admin: Address, role_store: Address`
- **Roles Checked**: `CONTROLLER` required for `transfer_out`.
- **Callers**: `order_handler`.
- **Callees**: SEP-41 token contracts, `role_store`.
- **Emitted Events**: None.
- **Upgrade Policy**: Immutable.

---

### Lifecycle Handlers

#### `deposit_handler`
- **Description**: Manages two-step deposit lifecycle (create -> keeper execute/cancel).
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::Oracle`, `InstanceKey::DepositVault`, and persistent `LocalKey::Deposit(nonce)` props.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, oracle: Address, deposit_vault: Address`
- **Roles Checked**: `ORDER_KEEPER` required to execute/cancel.
- **Callers**: `exchange_router`, keepers.
- **Callees**: `deposit_vault`, `market_token`, `data_store`, `oracle`, `role_store`.
- **Emitted Events**: `dep_req`, `dep_exec`, `dep_fail`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `withdrawal_handler`
- **Description**: Manages two-step withdrawal lifecycle (create -> keeper execute/cancel).
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::Oracle`, `InstanceKey::WithdrawalVault`, and persistent `LocalKey::Withdrawal(nonce)` props.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, oracle: Address, withdrawal_vault: Address`
- **Roles Checked**: `ORDER_KEEPER` required to execute/cancel.
- **Callers**: `exchange_router`, keepers.
- **Callees**: `withdrawal_vault`, `market_token`, `data_store`, `oracle`, `role_store`.
- **Emitted Events**: `wd_req`, `wd_exec`, `wd_fail`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `order_handler`
- **Description**: Manages full order lifecycle, position updates, and execution routing.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::Oracle`, `InstanceKey::OrderVault`, persistent `OrderStorageKey::Order(nonce)`, and persistent `PositionStorageKey::Position(key)`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, oracle: Address, order_vault: Address`
- **Roles Checked**: `ORDER_KEEPER` required to execute/cancel orders. `LIQUIDATION_KEEPER` / `ADL_KEEPER` / `CONTROLLER` required for liquidation and ADL.
- **Callers**: `exchange_router`, keepers, `liquidation_handler`, `adl_handler`.
- **Callees**: `order_vault`, `data_store`, `oracle`, `role_store`, and libraries (`increase_position_utils`, `decrease_position_utils`, etc.).
- **Emitted Events**: `ord_req`, `ord_exec`, `ord_fail`, `pos_update`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `liquidation_handler`
- **Description**: Validates position health and triggers forced closing of underwater positions.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::Oracle`, `InstanceKey::OrderHandler`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, oracle: Address, order_handler: Address`
- **Roles Checked**: `LIQUIDATION_KEEPER` required to liquidate.
- **Callers**: Keepers.
- **Callees**: `order_handler` (for position view and liquidation execution), `data_store`, `oracle`, `role_store`.
- **Emitted Events**: `liq_req`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `adl_handler`
- **Description**: Deleveraging handler that closes profitable positions when the pool PnL threshold is breached.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::Oracle`, `InstanceKey::OrderHandler`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, oracle: Address, order_handler: Address`
- **Roles Checked**: `ADL_KEEPER` required to execute deleveraging.
- **Callers**: Keepers.
- **Callees**: `order_handler` (for position view and ADL execution), `data_store`, `oracle`, `role_store`.
- **Emitted Events**: `adl_req`.
- **Upgrade Policy**: Upgradeable (Admin).

#### `fee_handler`
- **Description**: Claims accumulated protocol fees and manages fee distribution.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address`
- **Roles Checked**: `FEE_KEEPER` / Admin required to claim fees.
- **Callers**: Keepers/Admin.
- **Callees**: `data_store`, `role_store`, SEP-41 token contracts.
- **Emitted Events**: `fees_claimed` (implied).
- **Upgrade Policy**: Upgradeable (Admin).

#### `referral_storage`
- **Description**: Stores referral codes, trader-referrer links, and tier rebate configs.
- **State Owned**: `InstanceKey::Admin`, `ReferralKey::CodeOwner`, `ReferralKey::TraderCode`, `ReferralKey::ReferrerTier`, `ReferralKey::TierConfig`.
- **Initialization Args**: `admin: Address`
- **Roles Checked**: Admin auth required to configure tiers and referrer settings.
- **Callers**: Users (code registration), frontend/handlers (discounts).
- **Callees**: None.
- **Emitted Events**: `CodeRegistered`, `TraderCodeSet`.
- **Upgrade Policy**: Upgradeable (Admin).

---

### Periphery & Aggregation

#### `reader`
- **Description**: Stateless read-only aggregate queries for UI and keepers.
- **State Owned**: None (Stateless).
- **Initialization Args**: None.
- **Roles Checked**: None (Public view).
- **Callers**: Client UI, keepers.
- **Callees**: `data_store`, `oracle` (via read-only calls).
- **Emitted Events**: None.
- **Upgrade Policy**: Upgradeable (Admin).

#### `exchange_router`
- **Description**: Main entry point for user interactions. Supports atomic multicall.
- **State Owned**: `InstanceKey::Admin`, `InstanceKey::RoleStore`, `InstanceKey::DataStore`, `InstanceKey::DepositHandler`, `InstanceKey::WithdrawalHandler`, `InstanceKey::OrderHandler`.
- **Initialization Args**: `admin: Address, role_store: Address, data_store: Address, deposit_handler: Address, withdrawal_handler: Address, order_handler: Address`
- **Roles Checked**: None (Public).
- **Callers**: Users, frontends.
- **Callees**: `deposit_handler`, `withdrawal_handler`, `order_handler`, SEP-41 token contracts.
- **Emitted Events**: None.
- **Upgrade Policy**: Upgradeable (Admin).

---

## 3. Scope and Deviations (Laying Groundwork for Issue #1 & #3)
*(Detailed launch scope definitions and Solidity-to-Soroban deviations to be finalized by the secondary contributor).*
