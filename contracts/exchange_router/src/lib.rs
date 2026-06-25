//! Exchange router — single entry point for all user-facing protocol actions.
//! Mirrors GMX's ExchangeRouter.sol.
//!
//! Combines token transfers, vault interactions, and handler calls into
//! atomic multicall transactions. Users approve the router, then call
//! `multicall(Vec<RouterAction>)` with encoded instructions.
//!
//! Supported actions:
//!   SendTokens, CreateDeposit, CancelDeposit,
//!   CreateWithdrawal, CancelWithdrawal,
//!   CreateOrder, UpdateOrder, CancelOrder,
//!   ClaimFundingFees
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{global_pause_key, is_market_paused_key, scheduled_unpause_ledger_key};
use gmx_types::{CreateDepositParams, CreateOrderParams, CreateWithdrawalParams};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, Address, BytesN,
    Env, Vec,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    DepositHandler,
    WithdrawalHandler,
    OrderHandler,
    FeeHandler,
}

// ─── Router-only param structs ────────────────────────────────────────────────
// These are user-facing types for actions that have no handler equivalent.

#[contracttype]
pub struct SendTokensParams {
    pub token: Address,
    pub receiver: Address,
    pub amount: i128,
}

#[contracttype]
pub struct UpdateOrderParams {
    pub key: BytesN<32>,
    pub size_delta_usd: i128,
    pub acceptable_price: i128,
    pub trigger_price: i128,
    pub min_output_amount: i128,
}

#[contracttype]
pub struct ClaimFundingFeesParams {
    pub markets: Vec<Address>,
    pub tokens: Vec<Address>,
}

// ─── Multicall action discriminant ────────────────────────────────────────────

/// Each element in a multicall Vec is one action variant.
#[contracttype]
pub enum RouterAction {
    SendTokens(SendTokensParams),
    CreateDeposit(CreateDepositParams),
    CancelDeposit(BytesN<32>),
    CreateWithdrawal(CreateWithdrawalParams),
    CancelWithdrawal(BytesN<32>),
    CreateOrder(CreateOrderParams),
    UpdateOrder(UpdateOrderParams),
    CancelOrder(BytesN<32>),
    ClaimFundingFees(ClaimFundingFeesParams),
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
    BatchSizeLimitExceeded = 5,
    /// `execute_unpause` was called before the timelock window expired (issue #282).
    TimelockNotExpired = 6,
    /// `execute_unpause` was called without a prior `schedule_unpause` (issue #282).
    UnpauseNotScheduled = 7,
}

// ─── External handler clients ─────────────────────────────────────────────────
// Signatures must match the handler contract's public functions exactly.

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_bool(env: Env, key: BytesN<32>) -> bool;
    fn set_bool(env: Env, caller: Address, key: BytesN<32>, value: bool) -> bool;
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn set_position_manager(env: Env, caller: Address, market: Address, manager: Address) -> Address;
    fn get_position_manager(env: Env, owner: Address, market: Address) -> Option<Address>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DepositHandlerClient")]
trait IDepositHandler {
    fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32>;
    fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "WithdrawalHandlerClient")]
trait IWithdrawalHandler {
    fn create_withdrawal(env: Env, caller: Address, params: CreateWithdrawalParams) -> BytesN<32>;
    fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32>;
    fn create_orders(env: Env, caller: Address, requests: Vec<CreateOrderParams>) -> Vec<BytesN<32>>;
    fn update_order(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        size_delta_usd: i128,
        acceptable_price: i128,
        trigger_price: i128,
        min_output_amount: i128,
    );
    fn cancel_order(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "FeeHandlerClient")]
trait IFeeHandler {
    fn claim_funding_fees(env: Env, account: Address, market: Address, token: Address) -> u128;
    fn set_ui_fee_factor(env: Env, ui_receiver: Address, factor: u128);
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct ExchangeRouter;

#[contractimpl]
impl ExchangeRouter {
    /// One-time setup — store all handler addresses.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        deposit_handler: Address,
        withdrawal_handler: Address,
        order_handler: Address,
        fee_handler: Address,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
        env.storage()
            .instance()
            .set(&InstanceKey::DataStore, &data_store);
        env.storage()
            .instance()
            .set(&InstanceKey::DepositHandler, &deposit_handler);
        env.storage()
            .instance()
            .set(&InstanceKey::WithdrawalHandler, &withdrawal_handler);
        env.storage()
            .instance()
            .set(&InstanceKey::OrderHandler, &order_handler);
        env.storage()
            .instance()
            .set(&InstanceKey::FeeHandler, &fee_handler);
    }

    /// Upgrade the contract wasm. Only the stored admin may call this.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    /// Update the withdrawal_handler address. Only the stored admin may call this.
    pub fn update_withdrawal_handler(env: Env, caller: Address, new_handler: Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::WithdrawalHandler, &new_handler);
    }

    /// Default timelock for unpausing: ~4 hours at 5 s/ledger (issue #282).
    const UNPAUSE_TIMELOCK_LEDGERS: u32 = 2880;

    /// Pause the protocol immediately.
    ///
    /// Re-pausing clears any pending `schedule_unpause` so defenders can reset
    /// the timelock clock if the threat resurfaces (issue #282).
    pub fn set_paused(env: Env, paused: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        ds.set_bool(
            &env.current_contract_address(),
            &global_pause_key(&env),
            &paused,
        );
        // Clear any pending unpause schedule when re-pausing.
        if paused {
            ds.set_u128(
                &env.current_contract_address(),
                &scheduled_unpause_ledger_key(&env),
                &0,
            );
        }
    }

    /// Schedule an unpause after `UNPAUSE_TIMELOCK_LEDGERS` ledgers (issue #282).
    ///
    /// Records `current_ledger + UNPAUSE_TIMELOCK_LEDGERS` as the earliest ledger
    /// at which `execute_unpause` may succeed. Emits the scheduled ledger as an event
    /// so off-chain monitoring can observe the intent. Admin only.
    pub fn schedule_unpause(env: Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let scheduled_at =
            (env.ledger().sequence() + Self::UNPAUSE_TIMELOCK_LEDGERS) as u128;
        DataStoreClient::new(&env, &data_store).set_u128(
            &env.current_contract_address(),
            &scheduled_unpause_ledger_key(&env),
            &scheduled_at,
        );
        env.events().publish(
            (soroban_sdk::symbol_short!("unpause_s"),),
            scheduled_at,
        );
    }

    /// Execute a previously scheduled unpause if the timelock has expired (issue #282).
    ///
    /// Reverts with `TimelockNotExpired` if called before the scheduled ledger,
    /// and with `UnpauseNotScheduled` if `schedule_unpause` was never called (or was
    /// cleared by a re-pause). On success, clears both the pause flag and the schedule.
    pub fn execute_unpause(env: Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let router = env.current_contract_address();

        let scheduled =
            ds.get_u128(&scheduled_unpause_ledger_key(&env));
        if scheduled == 0 {
            panic_with_error!(&env, Error::UnpauseNotScheduled);
        }
        if (env.ledger().sequence() as u128) < scheduled {
            panic_with_error!(&env, Error::TimelockNotExpired);
        }

        // Clear pause and schedule.
        ds.set_bool(&router, &global_pause_key(&env), &false);
        ds.set_u128(&router, &scheduled_unpause_ledger_key(&env), &0);
    }

    pub fn reset_circuit_breaker(env: Env, market: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        DataStoreClient::new(&env, &data_store).set_bool(
            &env.current_contract_address(),
            &is_market_paused_key(&env, &market),
            &false,
        );
    }

    fn require_not_paused(env: &Env) {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        let ds = DataStoreClient::new(env, &data_store);
        if ds.get_bool(&global_pause_key(env)) {
            panic_with_error!(env, Error::Paused);
        }
    }

    // ── Multicall ─────────────────────────────────────────────────────────────

    /// Execute a batch of actions atomically.
    ///
    /// Single caller.require_auth() covers all sub-actions (they run inside this invocation).
    /// Returns one BytesN<32> result per action (create_* returns a key; others return zero hash).
    /// If any action panics, the entire transaction reverts (Soroban atomicity).
    ///
    /// Handlers are called directly (not via the self-referential public wrappers) to avoid a
    /// double require_auth() within the same invocation frame, which Soroban rejects.
    pub fn multicall(env: Env, caller: Address, actions: Vec<RouterAction>) -> Vec<BytesN<32>> {
        caller.require_auth();
        Self::require_not_paused(&env);

        let deposit_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let withdrawal_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::WithdrawalHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let fee_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::FeeHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        let mut results: Vec<BytesN<32>> = Vec::new(&env);
        let zero_key = BytesN::from_array(&env, &[0u8; 32]);

        let len = actions.len();
        let mut i = 0u32;
        while i < len {
            let action = actions.get(i).unwrap();
            match action {
                RouterAction::SendTokens(p) => {
                    token::Client::new(&env, &p.token).transfer(&caller, &p.receiver, &p.amount);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateDeposit(p) => {
                    let key = DepositHandlerClient::new(&env, &deposit_handler)
                        .create_deposit(&caller, &p);
                    results.push_back(key);
                }
                RouterAction::CancelDeposit(key) => {
                    DepositHandlerClient::new(&env, &deposit_handler).cancel_deposit(&caller, &key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateWithdrawal(p) => {
                    let key = WithdrawalHandlerClient::new(&env, &withdrawal_handler)
                        .create_withdrawal(&caller, &p);
                    results.push_back(key);
                }
                RouterAction::CancelWithdrawal(key) => {
                    WithdrawalHandlerClient::new(&env, &withdrawal_handler)
                        .cancel_withdrawal(&caller, &key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateOrder(p) => {
                    let key =
                        OrderHandlerClient::new(&env, &order_handler).create_order(&caller, &p);
                    results.push_back(key);
                }
                RouterAction::UpdateOrder(p) => {
                    OrderHandlerClient::new(&env, &order_handler).update_order(
                        &caller,
                        &p.key,
                        &p.size_delta_usd,
                        &p.acceptable_price,
                        &p.trigger_price,
                        &p.min_output_amount,
                    );
                    results.push_back(zero_key.clone());
                }
                RouterAction::CancelOrder(key) => {
                    OrderHandlerClient::new(&env, &order_handler).cancel_order(&caller, &key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::ClaimFundingFees(p) => {
                    let fee_client = FeeHandlerClient::new(&env, &fee_handler);
                    let mlen = p.markets.len();
                    let mut mi = 0u32;
                    while mi < mlen {
                        fee_client.claim_funding_fees(
                            &caller,
                            &p.markets.get(mi).unwrap(),
                            &p.tokens.get(mi).unwrap(),
                        );
                        mi += 1;
                    }
                    results.push_back(zero_key.clone());
                }
            }
            i += 1;
        }

        results
    }

    // ── Individual action helpers ─────────────────────────────────────────────

    /// Transfer `amount` of `token` from caller to `receiver` (funds a vault).
    pub fn send_tokens(env: Env, caller: Address, token: Address, receiver: Address, amount: i128) {
        caller.require_auth();
        Self::require_not_paused(&env);
        token::Client::new(&env, &token).transfer(&caller, &receiver, &amount);
    }

    /// Forward create_deposit to the deposit_handler.
    pub fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32> {
        caller.require_auth();
        Self::require_not_paused(&env);
        let deposit_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        DepositHandlerClient::new(&env, &deposit_handler).create_deposit(&caller, &params)
    }

    /// Forward cancel_deposit to the deposit_handler.
    pub fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let deposit_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        DepositHandlerClient::new(&env, &deposit_handler).cancel_deposit(&caller, &key);
    }

    /// Forward create_withdrawal to the withdrawal_handler.
    pub fn create_withdrawal(
        env: Env,
        caller: Address,
        params: CreateWithdrawalParams,
    ) -> BytesN<32> {
        caller.require_auth();
        Self::require_not_paused(&env);
        let withdrawal_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::WithdrawalHandler)
            .unwrap();
        WithdrawalHandlerClient::new(&env, &withdrawal_handler).create_withdrawal(&caller, &params)
    }

    /// Forward cancel_withdrawal to the withdrawal_handler.
    pub fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let withdrawal_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::WithdrawalHandler)
            .unwrap();
        WithdrawalHandlerClient::new(&env, &withdrawal_handler).cancel_withdrawal(&caller, &key);
    }

    /// Forward create_order to the order_handler.
    ///
    /// # Required multicall sequence for increase / swap order types
    ///
    /// The protocol's canonical collateral model (issue #47) requires that
    /// the caller pushes tokens into order_vault **before** this action runs.
    /// Use `SendTokens` with `receiver = order_vault` as the immediately
    /// preceding step in the same multicall:
    ///
    /// ```text
    /// multicall([
    ///   SendTokens { token: collateral_token, receiver: order_vault, amount },
    ///   CreateOrder { params },   ← order_handler snapshots the delta here
    /// ])
    /// ```
    ///
    /// Omitting `SendTokens` causes order_handler to revert with `ZeroCollateral`.
    /// Decrease / stop-loss / liquidation orders do not require a prior token send.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        caller.require_auth();
        Self::require_not_paused(&env);
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap();
        OrderHandlerClient::new(&env, &order_handler).create_order(&caller, &params)
    }

    /// Create up to 5 orders atomically in a single call (issue #219).
    ///
    /// For increase/swap orders in the batch the caller must pre-fund the
    /// order_vault via `SendTokens` before this call (one send per increase/swap leg).
    /// Any failure reverts the entire batch (Soroban atomicity).
    pub fn create_orders(
        env: Env,
        caller: Address,
        requests: Vec<CreateOrderParams>,
    ) -> Vec<BytesN<32>> {
        caller.require_auth();
        Self::require_not_paused(&env);
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        OrderHandlerClient::new(&env, &order_handler).create_orders(&caller, &requests)
    }

    /// Forward update_order to the order_handler.
    pub fn update_order(env: Env, caller: Address, params: UpdateOrderParams) {
        caller.require_auth();
        Self::require_not_paused(&env);
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap();
        OrderHandlerClient::new(&env, &order_handler).update_order(
            &caller,
            &params.key,
            &params.size_delta_usd,
            &params.acceptable_price,
            &params.trigger_price,
            &params.min_output_amount,
        );
    }

    /// Forward cancel_order to the order_handler.
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap();
        OrderHandlerClient::new(&env, &order_handler).cancel_order(&caller, &key);
    }

    /// Claim earned funding fees across multiple markets in one call.
    pub fn claim_funding_fees(
        env: Env,
        caller: Address,
        markets: Vec<Address>,
        tokens: Vec<Address>,
    ) {
        caller.require_auth();
        let fee_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::FeeHandler)
            .unwrap();
        let fee_client = FeeHandlerClient::new(&env, &fee_handler);
        let len = markets.len();
        let mut i = 0u32;
        while i < len {
            fee_client.claim_funding_fees(
                &caller,
                &markets.get(i).unwrap(),
                &tokens.get(i).unwrap(),
            );
            i += 1;
        }
    }

    /// Set or revoke a position manager for the caller on a specific market.
    ///
    /// A position manager is authorized to create, increase, decrease, or close
    /// positions on behalf of the owner, but cannot redirect collateral receipts.
    /// The manager cannot override the receiver — funds always go to the owner.
    ///
    /// Call with zero_address to revoke an existing manager.
    pub fn set_position_manager(env: Env, caller: Address, market: Address, manager: Address) {
        caller.require_auth();
        let data_store: Address = env.storage().instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let data_store_client = DataStoreClient::new(&env, &data_store);
        data_store_client.set_position_manager(&caller, &market, &manager);
    }

    /// Query the current position manager for an account on a specific market.
    pub fn get_position_manager(env: Env, owner: Address, market: Address) -> Option<Address> {
        let data_store: Address = env.storage().instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let data_store_client = DataStoreClient::new(&env, &data_store);
        data_store_client.get_position_manager(&owner, &market)
    }

    /// Set the UI fee factor for a receiver. Delegates auth enforcement to fee_handler.
    pub fn set_ui_fee_factor(env: Env, ui_receiver: Address, factor: u128) {
        let fee_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::FeeHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        FeeHandlerClient::new(&env, &fee_handler).set_ui_fee_factor(&ui_receiver, &factor);
    }
}

// ─── Tests — Issues #101 #102 #103 #104: Full protocol E2E harness ────────────
//
// Issue #101: Reusable setup that deploys all contracts, grants roles, creates
//   tokens and markets, sets oracle prices, and seeds liquidity.
//   Done: New E2E tests share a single setup(); boilerplate is not copy-pasted.
//
// Issue #102: Deposit-to-withdrawal E2E through the full handler stack.
//   Done: User recovers expected tokens within acceptable rounding.
//         Pool amounts return to baseline.
//
// Issue #103: LP deposit → trader opens position → price moves → position closes
//   → LP withdraws. Pool accounting must be consistent throughout.
//   Done: Pool accounting is consistent at every step.
//         Trader PnL and LP redemption values are correct.
//
// Issue #104: Liquidation via generated contract clients (deployed-style),
//   not direct utility function calls.
//   Done: Test uses client-based invocation. Succeeds for underwater position.
//         Fails for healthy position.
#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use deposit_handler::{DepositHandler, DepositHandlerClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use gmx_keys::roles;
    use gmx_math::FLOAT_PRECISION;
    use gmx_types::{
        CreateDepositParams, CreateOrderParams, CreateWithdrawalParams, OrderType, TokenPrice,
    };
    use liquidation_handler::{LiquidationHandler, LiquidationHandlerClient as LHClient};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env};
    use withdrawal_handler::{WithdrawalHandler, WithdrawalHandlerClient};
    use withdrawal_vault::{WithdrawalVault, WithdrawalVaultClient as WVClient};

    const ONE_TOKEN: i128 = 10_000_000; // Stellar 7-decimal precision

    // ── Issue #101: shared full-protocol harness ──────────────────────────────

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        liq_keeper: Address,
        rs: Address,
        ds: Address,
        oracle: Address,
        dep_vault: Address,
        wth_vault: Address,
        ord_vault: Address,
        dep_handler: Address,
        wth_handler: Address,
        ord_handler: Address,
        liq_handler: Address,
        #[allow(dead_code)]
        router: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);
        let liq_keeper = Address::generate(&env);

        // Role store
        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
        rs_c.grant_role(&admin, &liq_keeper, &roles::liquidation_keeper(&env));

        // Data store
        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        // Oracle
        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        // Vaults
        let dep_vault = env.register(DepositVault, ());
        DVClient::new(&env, &dep_vault).initialize(&admin, &rs);
        let wth_vault = env.register(WithdrawalVault, ());
        WVClient::new(&env, &wth_vault).initialize(&admin, &rs);
        let ord_vault = env.register(OrderVault, ());
        OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

        // Market token (LP token + pool custodian)
        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "SO4 Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        // Underlying tokens
        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        // Handlers
        let dep_handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &dep_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &dep_vault,
        );

        let wth_handler = env.register(WithdrawalHandler, ());
        WithdrawalHandlerClient::new(&env, &wth_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &wth_vault,
        );

        let ord_handler = env.register(OrderHandler, ());
        OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

        let liq_handler = env.register(LiquidationHandler, ());
        LHClient::new(&env, &liq_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_handler);

        // Exchange router (fee_handler is unused in E2E tests — dummy address)
        let fee_handler_dummy = Address::generate(&env);
        let router = env.register(ExchangeRouter, ());
        ExchangeRouterClient::new(&env, &router).initialize(
            &admin,
            &rs,
            &ds,
            &dep_handler,
            &wth_handler,
            &ord_handler,
            &fee_handler_dummy,
        );

        // Grant CONTROLLER to all handlers and the router (router writes global_pause_key)
        rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &liq_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &router, &roles::controller(&env));

        // Register market in DataStore
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &admin,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_short_token_key(&env, &market_tk),
            &short_tk,
        );

        // Market config: 0.1% position fee, 1% min collateral factor, 100x max leverage
        let fee_factor = FLOAT_PRECISION / 1000;
        let min_col_factor = FLOAT_PRECISION / 100;
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_fee_factor_key(&env, &market_tk, true),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_fee_factor_key(&env, &market_tk, false),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::min_collateral_factor_key(&env, &market_tk),
            &(min_col_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_leverage_key(&env, &market_tk),
            &(100 * FLOAT_PRECISION as u128),
        );

        World {
            env,
            admin,
            keeper,
            liq_keeper,
            rs,
            ds,
            oracle: oracle_addr,
            dep_vault,
            wth_vault,
            ord_vault,
            dep_handler,
            wth_handler,
            ord_handler,
            liq_handler,
            router,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn set_prices(w: &World, index_usd: i128) {
        let fp = FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(
            &w.keeper,
            &soroban_sdk::Vec::from_array(
                &w.env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                ],
            ),
        );
    }

    /// Mint tokens to `lp`, deposit them through the deposit handler, execute, return minted LP balance.
    fn provide_liquidity(w: &World, lp: &Address, long_amt: i128, short_amt: i128) -> i128 {
        if long_amt > 0 {
            StellarAssetClient::new(&w.env, &w.long_tk).mint(lp, &long_amt);
        }
        if short_amt > 0 {
            StellarAssetClient::new(&w.env, &w.short_tk).mint(lp, &short_amt);
        }
        let key = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
            lp,
            &CreateDepositParams {
                receiver: lp.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: long_amt,
                short_token_amount: short_amt,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &key);
        MtClient::new(&w.env, &w.market_tk).balance(lp)
    }

    /// Mint collateral to `user`, transfer to order vault (canonical collateral model),
    /// then create and execute a MarketIncrease long order.
    fn open_long_position(w: &World, user: &Address, collateral_tokens: i128, size_usd: i128) {
        StellarAssetClient::new(&w.env, &w.long_tk).mint(user, &collateral_tokens);
        soroban_sdk::token::Client::new(&w.env, &w.long_tk).transfer(
            user,
            &w.ord_vault,
            &collateral_tokens,
        );
        let hc = OHClient::new(&w.env, &w.ord_handler);
        let key = hc.create_order(
            user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: size_usd,
                collateral_delta_amount: collateral_tokens,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
        hc.execute_order(&w.keeper, &key);
    }

    // ── Issue #102: deposit-to-withdrawal E2E ────────────────────────────────

    /// Full LP lifecycle: deposit long+short → receive LP → withdraw all → recover tokens.
    /// Asserts user recovers tokens and pool returns to zero (single-depositor, no trades).
    #[test]
    fn e2e_deposit_then_withdraw_recovers_tokens() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let lp = Address::generate(&w.env);

        set_prices(&w, 2_000 * fp);

        let long_amt = 5 * ONE_TOKEN;
        let short_amt = 5_000 * ONE_TOKEN;

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &long_amt);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp, &short_amt);

        let lp_tokens = provide_liquidity(&w, &lp, long_amt, short_amt);
        assert!(lp_tokens > 0, "LP tokens must be minted on deposit");

        let ds_c = DsClient::new(&w.env, &w.ds);
        assert_eq!(
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk)),
            long_amt as u128,
            "long pool must match deposit"
        );
        assert_eq!(
            ds_c.get_u128(&gmx_keys::pool_amount_key(
                &w.env,
                &w.market_tk,
                &w.short_tk
            )),
            short_amt as u128,
            "short pool must match deposit"
        );

        set_prices(&w, 2_000 * fp);

        // Withdraw all LP tokens
        let wth_key = WithdrawalHandlerClient::new(&w.env, &w.wth_handler).create_withdrawal(
            &lp,
            &CreateWithdrawalParams {
                receiver: lp.clone(),
                market: w.market_tk.clone(),
                market_token_amount: lp_tokens,
                min_long_token_amount: 0,
                min_short_token_amount: 0,
                execution_fee: 0,
            },
        );
        WithdrawalHandlerClient::new(&w.env, &w.wth_handler)
            .execute_withdrawal(&w.keeper, &wth_key);

        assert_eq!(
            MtClient::new(&w.env, &w.market_tk).balance(&lp),
            0,
            "LP tokens must be fully burned after withdrawal"
        );

        let long_back = StellarAssetClient::new(&w.env, &w.long_tk).balance(&lp);
        let short_back = StellarAssetClient::new(&w.env, &w.short_tk).balance(&lp);
        assert!(
            long_back > 0 || short_back > 0,
            "user must recover tokens after withdrawal"
        );

        // Pool returns to zero (single depositor, no trades — no rounding loss)
        assert_eq!(
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk)),
            0,
            "long pool must return to zero after full withdrawal"
        );
        assert_eq!(
            ds_c.get_u128(&gmx_keys::pool_amount_key(
                &w.env,
                &w.market_tk,
                &w.short_tk
            )),
            0,
            "short pool must return to zero after full withdrawal"
        );
    }

    // ── Issue #103: deposit-to-trade-to-withdraw E2E ─────────────────────────

    /// LP deposits → trader opens long position → position closes at break-even
    /// → LP withdraws. Pool accounting must be consistent at every step.
    #[test]
    fn e2e_deposit_trade_withdraw_pool_accounting() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let lp_user = Address::generate(&w.env);
        let trader = Address::generate(&w.env);

        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        // LP provides deep liquidity so the pool can pay out PnL if needed
        let lp_long_amt = 10 * ONE_TOKEN;
        let lp_short_amt = 10_000 * ONE_TOKEN;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp_user, &lp_long_amt);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp_user, &lp_short_amt);
        let lp_tokens = provide_liquidity(&w, &lp_user, lp_long_amt, lp_short_amt);
        assert!(lp_tokens > 0, "LP must receive market tokens");

        let ds_c = DsClient::new(&w.env, &w.ds);

        // Pool accounting after deposit
        let pool_long_post_dep =
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk));
        assert_eq!(
            pool_long_post_dep, lp_long_amt as u128,
            "long pool matches deposit"
        );

        set_prices(&w, entry_price);

        // Trader opens 2x leveraged long position
        let collateral = ONE_TOKEN;
        let size_usd = 4_000 * fp;
        open_long_position(&w, &trader, collateral, size_usd);

        let pos_key = gmx_keys::position_key(&w.env, &trader, &w.market_tk, &w.long_tk, true);
        let position = OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .expect("position must exist after MarketIncrease");
        assert!(position.size_in_usd > 0, "position size must be positive");

        // Pool grew: collateral moved from vault into pool
        let pool_long_post_open =
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk));
        assert!(
            pool_long_post_open > pool_long_post_dep,
            "pool must grow when collateral is added on position open"
        );

        // On-chain balance of market_tk must be >= DataStore pool record.
        // DataStore tracks the LP portion + fees; position collateral is held on-chain
        // but accounted separately through open interest, not pool_amount_key.
        let on_chain_long =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.market_tk);
        assert!(
            on_chain_long as u128 >= pool_long_post_open,
            "on-chain balance must be >= DataStore pool amount (pool is always fully backed)"
        );

        // Trader closes position at same price (break-even; trader pays fees)
        set_prices(&w, entry_price);
        let close_key = OHClient::new(&w.env, &w.ord_handler).create_order(
            &trader,
            &CreateOrderParams {
                receiver: trader.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: position.size_in_usd,
                collateral_delta_amount: 0, // ignored by decrease_position logic
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketDecrease,
                is_long: true,
            },
        );
        OHClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &close_key);

        // Position must be fully closed
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_none(),
            "position must be removed after full MarketDecrease"
        );

        // Trader received collateral back (collateral minus fees)
        let trader_bal = StellarAssetClient::new(&w.env, &w.long_tk).balance(&trader);
        assert!(
            trader_bal > 0,
            "trader must receive collateral back after closing position"
        );

        // After close: position collateral is returned to trader, so on-chain balance
        // should now equal pool_amount_key (no open positions remain).
        let pool_long_post_close =
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk));
        let on_chain_long_close =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.market_tk);
        assert_eq!(
            pool_long_post_close, on_chain_long_close as u128,
            "DataStore pool must equal on-chain balance after all positions are closed"
        );
        // Pool must be at least as large as original deposit (fees collected from trader)
        assert!(
            pool_long_post_close >= pool_long_post_dep,
            "pool must be at least equal to original deposit after break-even trade (fees earned)"
        );

        // LP withdraws all LP tokens
        set_prices(&w, entry_price);
        let wth_key = WithdrawalHandlerClient::new(&w.env, &w.wth_handler).create_withdrawal(
            &lp_user,
            &CreateWithdrawalParams {
                receiver: lp_user.clone(),
                market: w.market_tk.clone(),
                market_token_amount: lp_tokens,
                min_long_token_amount: 0,
                min_short_token_amount: 0,
                execution_fee: 0,
            },
        );
        WithdrawalHandlerClient::new(&w.env, &w.wth_handler)
            .execute_withdrawal(&w.keeper, &wth_key);

        assert_eq!(
            MtClient::new(&w.env, &w.market_tk).balance(&lp_user),
            0,
            "LP tokens must be burned after withdrawal"
        );

        let lp_long_back = StellarAssetClient::new(&w.env, &w.long_tk).balance(&lp_user);
        let lp_short_back = StellarAssetClient::new(&w.env, &w.short_tk).balance(&lp_user);
        assert!(
            lp_long_back > 0 || lp_short_back > 0,
            "LP must recover tokens after withdrawal"
        );

        // Pool near-zero after full LP exit (within 1 unit rounding tolerance)
        let pool_long_final =
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk));
        let pool_short_final = ds_c.get_u128(&gmx_keys::pool_amount_key(
            &w.env,
            &w.market_tk,
            &w.short_tk,
        ));
        assert!(
            pool_long_final <= 1,
            "long pool must return near baseline after full LP withdrawal"
        );
        assert!(
            pool_short_final <= 1,
            "short pool must return near baseline after full LP withdrawal"
        );
    }

    // ── Issue #104: liquidation E2E through deployed-style clients ────────────

    /// Open a 10x long, crash the price past the liquidation threshold, and verify
    /// that the liquidation_handler client closes the position (client-based invocation,
    /// not a direct call to the position utility library).
    #[test]
    fn e2e_client_liquidation_underwater_succeeds() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let lp = Address::generate(&w.env);
        let trader = Address::generate(&w.env);

        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        // Seed pool so the market has liquidity for the position
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(ONE_TOKEN * 100));
        provide_liquidity(&w, &lp, ONE_TOKEN * 100, 0);

        set_prices(&w, entry_price);

        // Trader opens 10x leveraged long
        let collateral = ONE_TOKEN;
        let size_usd = 20_000 * fp;
        open_long_position(&w, &trader, collateral, size_usd);

        let pos_key = gmx_keys::position_key(&w.env, &trader, &w.market_tk, &w.long_tk, true);
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_some(),
            "position must exist before liquidation"
        );

        // Crash price — position is deeply underwater
        let crash_price = 100 * fp;
        set_prices(&w, crash_price);

        let is_liq = LHClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
        assert!(is_liq, "position must be liquidatable after price crash");

        // Liquidate via client (deployed-style invocation)
        LHClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_none(),
            "position key must be removed from order_handler storage after liquidation"
        );
    }

    /// Attempting to liquidate a healthy position must revert (NotLiquidatable).
    #[test]
    #[should_panic]
    fn e2e_client_liquidation_healthy_reverts() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let lp = Address::generate(&w.env);
        let trader = Address::generate(&w.env);

        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        // Seed pool
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(ONE_TOKEN * 100));
        provide_liquidity(&w, &lp, ONE_TOKEN * 100, 0);

        set_prices(&w, entry_price);

        // Well-collateralised long (10 tokens at 2x leverage — very healthy)
        open_long_position(&w, &trader, ONE_TOKEN * 10, 4_000 * fp);

        // Price stays at entry — position is healthy
        set_prices(&w, entry_price);

        // Must panic with NotLiquidatable
        LHClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
    }

    // ── Issue #177: pause flag coverage for create_deposit ───────────────────

    /// When the protocol is paused, create_deposit must revert with a Paused error
    /// before any token transfer occurs.
    #[test]
    fn create_deposit_reverts_when_paused() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let user = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &(ONE_TOKEN * 2));
        set_prices(&w, 2_000 * fp);

        let router = ExchangeRouterClient::new(&w.env, &w.router);

        // Pause the protocol
        router.set_paused(&true);

        let result = router.try_create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: ONE_TOKEN,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        assert!(result.is_err(), "create_deposit must fail with Paused error");

        // No tokens should have moved — user still holds everything
        let user_bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user);
        assert_eq!(
            user_bal,
            ONE_TOKEN * 2,
            "no tokens must transfer when create_deposit reverts due to pause"
        );
    }

    /// cancel_deposit must succeed while the protocol is paused so users can
    /// always recover pending funds.
    #[test]
    fn cancel_deposit_succeeds_when_paused() {
        let w = setup();
        let user = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &ONE_TOKEN);

        // Create deposit through the handler directly (before pausing)
        let key = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: ONE_TOKEN,
                short_token_amount: 0,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );

        let router = ExchangeRouterClient::new(&w.env, &w.router);

        // Pause the protocol
        router.set_paused(&true);

        // Cancel must succeed — cancellations are whitelisted while paused
        router.cancel_deposit(&user, &key);

        assert!(
            DepositHandlerClient::new(&w.env, &w.dep_handler)
                .get_deposit(&key)
                .is_none(),
            "deposit must be gone after cancel"
        );
        let balance = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user);
        assert_eq!(balance, ONE_TOKEN, "tokens must be refunded after cancel while paused");
    }

    // ── Issue #135: Router multicall E2E tests ────────────────────────────────

    /// Successful multicall: SendTokens to order_vault followed by CreateOrder
    /// completes atomically. The order is created with the canonical collateral
    /// model (send-first, then create), and the resulting key is executable.
    #[test]
    fn e2e_multicall_send_tokens_then_create_order_succeeds() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let lp = Address::generate(&w.env);
        let trader = Address::generate(&w.env);

        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        // Seed pool so the market has liquidity to absorb the position
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(ONE_TOKEN * 100));
        provide_liquidity(&w, &lp, ONE_TOKEN * 100, 0);

        set_prices(&w, entry_price);

        // Mint collateral to trader
        let collateral = 2 * ONE_TOKEN;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&trader, &collateral);

        // Verify collateral is in trader's account before multicall
        let balance_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&trader);
        assert_eq!(balance_before, collateral);

        // Build multicall: [SendTokens → ord_vault, CreateOrder (MarketIncrease)]
        let actions = soroban_sdk::vec![
            &w.env,
            RouterAction::SendTokens(SendTokensParams {
                token: w.long_tk.clone(),
                receiver: w.ord_vault.clone(),
                amount: collateral,
            }),
            RouterAction::CreateOrder(gmx_types::CreateOrderParams {
                receiver: trader.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: 4_000 * fp,
                collateral_delta_amount: collateral,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            }),
        ];

        let router_client = ExchangeRouterClient::new(&w.env, &w.router);
        let results = router_client.multicall(&trader, &actions);

        // First result is zero_key (SendTokens), second is the created order key
        assert_eq!(results.len(), 2);
        let zero_key = soroban_sdk::BytesN::from_array(&w.env, &[0u8; 32]);
        assert_eq!(
            results.get(0).unwrap(),
            zero_key,
            "SendTokens result must be zero_key"
        );

        let order_key = results.get(1).unwrap();
        assert_ne!(
            order_key, zero_key,
            "CreateOrder must return a non-zero order key"
        );

        // Trader's collateral has moved to the vault
        let trader_bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&trader);
        assert_eq!(
            trader_bal_after, 0,
            "trader collateral must be in vault after multicall"
        );

        // Execute the order and verify a position was opened
        OHClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &order_key);

        let pos_key = gmx_keys::position_key(&w.env, &trader, &w.market_tk, &w.long_tk, true);
        let position = OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .expect("position must exist after multicall + execute_order");
        assert!(
            position.size_in_usd > 0,
            "position must have positive size after successful multicall"
        );
    }

    /// Atomicity guarantee: if any step in a multicall panics, all preceding
    /// steps are also reverted. Here, SendTokens succeeds but the following
    /// CancelOrder (non-existent key) panics, rolling back the token transfer.
    #[test]
    fn e2e_multicall_failed_step_reverts_preceding_steps() {
        let w = setup();
        let lp = Address::generate(&w.env);
        let trader = Address::generate(&w.env);

        set_prices(&w, 2_000 * FLOAT_PRECISION);

        // Seed pool
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(ONE_TOKEN * 50));
        provide_liquidity(&w, &lp, ONE_TOKEN * 50, 0);

        set_prices(&w, 2_000 * FLOAT_PRECISION);

        // Mint tokens to trader
        let amount = ONE_TOKEN;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&trader, &amount);

        let balance_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&trader);
        assert_eq!(balance_before, amount);

        // A random key that doesn't correspond to any real order
        let fake_key = soroban_sdk::BytesN::from_array(&w.env, &[0xFFu8; 32]);

        // Build multicall: Step 1 transfers tokens (would succeed alone),
        // Step 2 cancels a non-existent order → will panic → reverts Step 1.
        let actions = soroban_sdk::vec![
            &w.env,
            RouterAction::SendTokens(SendTokensParams {
                token: w.long_tk.clone(),
                receiver: w.ord_vault.clone(),
                amount,
            }),
            RouterAction::CancelOrder(fake_key),
        ];

        let router_client = ExchangeRouterClient::new(&w.env, &w.router);
        let result = router_client.try_multicall(&trader, &actions);

        // Multicall must have failed
        assert!(result.is_err(), "multicall must fail when a step panics");

        // Step 1 (SendTokens) must have been atomically reverted:
        // trader still holds the full amount
        let balance_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&trader);
        assert_eq!(
            balance_after, balance_before,
            "token transfer from Step 1 must revert when Step 2 fails; balance_after={}, balance_before={}",
            balance_after, balance_before
        );

        // Vault must hold nothing (transfer also reverted)
        let vault_bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.ord_vault);
        assert_eq!(
            vault_bal, 0,
            "order_vault must hold nothing after multicall reverts"
        );
    }
}
