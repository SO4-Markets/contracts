//! Order handler — create, execute, cancel, update, and freeze orders.
//! Mirrors GMX's OrderHandler.sol.
//!
//! Supported order types (OrderType enum in gmx_types):
//!   MarketSwap, LimitSwap            → routed to swap_utils
//!   MarketIncrease, LimitIncrease    → routed to increase_position_utils
//!   MarketDecrease, LimitDecrease,
//!   StopLossDecrease, Liquidation    → routed to decrease_position_utils
//!
//! Two-step lifecycle (same as deposit/withdrawal):
//!   create_order  → pulls collateral into order_vault, stores OrderProps
//!   execute_order → keeper calls with fresh oracle prices, dispatches by type
//!   cancel_order  → refunds collateral from order_vault to account
//!   update_order  → modify trigger_price / acceptable_price / size before execution
//!   freeze_order  → mark order as frozen (keeper-side circuit breaker)
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Vec,
    symbol_short, panic_with_error,
};
use gmx_types::{MarketProps, OrderProps, OrderType, PriceProps};
use gmx_keys::{
    order_key, order_list_key, account_order_list_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:       &str = "ADMIN";
const ROLE_STORE_KEY:  &str = "ROLE_STORE";
const DATA_STORE_KEY:  &str = "DATA_STORE";
const ORACLE_KEY:      &str = "ORACLE";
const ORDER_VAULT_KEY: &str = "ORDER_VAULT";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
pub enum Error {
    AlreadyInitialized    = 1,
    NotInitialized        = 2,
    Unauthorized          = 3,
    OrderNotFound         = 4,
    InvalidOrderType      = 5,
    UnsatisfiedTrigger    = 6,
    PriceTooHigh          = 7,
    PriceTooLow           = 8,
    OrderFrozen           = 9,
}

// ─── External contract clients ────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: soroban_sdk::Symbol) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn increment_nonce(env: Env, caller: Address) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[soroban_sdk::contractclient(name = "OrderVaultClient")]
trait IOrderVault {
    fn record_transfer_in(env: Env, token: Address) -> i128;
    fn transfer_out(env: Env, caller: Address, token: Address, receiver: Address, amount: i128);
}

// ─── Order-frozen flag (stored alongside OrderProps) ──────────────────────────

#[contracttype]
pub enum OrderStorageKey {
    Order(BytesN<32>),
    OrderFrozen(BytesN<32>),
}

// ─── Create params (mirrors GMX BaseOrderUtils.CreateOrderParams) ─────────────

#[contracttype]
pub struct CreateOrderParams {
    pub receiver:                   Address,
    pub market:                     Address,
    pub initial_collateral_token:   Address,
    pub swap_path:                  Vec<Address>,
    pub size_delta_usd:             i128,
    pub collateral_delta_amount:    i128,
    pub trigger_price:              i128,   // FLOAT_PRECISION; 0 for market orders
    pub acceptable_price:           i128,   // FLOAT_PRECISION; 0 = no slippage check
    pub execution_fee:              i128,
    pub min_output_amount:          i128,
    pub order_type:                 OrderType,
    pub is_long:                    bool,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct OrderHandler;

#[contractimpl]
impl OrderHandler {
    /// One-time setup.
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        order_vault: Address,
    ) {
        // TODO: panic if already initialized (ADMIN_KEY exists in instance storage)
        //       Store all five addresses in instance storage under their respective keys
        todo!()
    }

    /// Create a new order and pull collateral into the order vault.
    ///
    /// For increase orders: caller transfers `collateral_delta_amount` of
    ///   `initial_collateral_token` into order_vault before this call,
    ///   then we call order_vault.record_transfer_in() to snapshot it.
    /// For decrease orders: no collateral transfer needed (position already has it).
    /// Returns the order key (BytesN<32>) that keepers use for execution.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        // TODO: (mirrors GMX OrderHandler.createOrder)
        //
        // 1. caller.require_auth()
        //    require role ORDER_KEEPER or just any authenticated caller (GMX allows anyone)
        //
        // 2. Load data_store and order_vault from instance storage
        //
        // 3. For increase/swap order types: record collateral arrival
        //    received = order_vault_client.record_transfer_in(initial_collateral_token)
        //    Validate received >= params.collateral_delta_amount
        //
        // 4. Generate key:
        //    nonce = ds_client.increment_nonce(&caller)
        //    key   = order_key(&env, nonce)
        //
        // 5. Build OrderProps:
        //    OrderProps {
        //        account: caller.clone(),
        //        receiver: params.receiver,
        //        market: params.market,
        //        initial_collateral_token: params.initial_collateral_token,
        //        swap_path: params.swap_path,
        //        size_delta_usd: params.size_delta_usd,
        //        collateral_delta_amount: received (for increase) or params.collateral_delta_amount,
        //        trigger_price: params.trigger_price,
        //        acceptable_price: params.acceptable_price,
        //        execution_fee: params.execution_fee,
        //        min_output_amount: params.min_output_amount,
        //        order_type: params.order_type,
        //        is_long: params.is_long,
        //        updated_at_time: env.ledger().timestamp(),
        //    }
        //
        // 6. Persist order in handler's own persistent storage at OrderStorageKey::Order(key)
        //
        // 7. Index in data_store:
        //    ds_client.add_bytes32_to_set(&caller, order_list_key(&env), key)
        //    ds_client.add_bytes32_to_set(&caller, account_order_list_key(&env, &caller), key)
        //
        // 8. Emit "order_created" event
        //
        // Returns key
        todo!()
    }

    /// Execute a pending order (called by keeper).
    ///
    /// Routes to the appropriate utils based on order type:
    ///   - swap orders   → swap_utils::swap_with_path
    ///   - increase      → increase_position_utils::increase_position
    ///   - decrease      → decrease_position_utils::decrease_position
    pub fn execute_order(env: Env, keeper: Address, key: BytesN<32>) {
        // TODO: (mirrors GMX OrderHandler.executeOrder)
        //
        // 1. keeper.require_auth()
        //    Require keeper has ORDER_KEEPER role
        //
        // 2. Load order from handler persistent storage by OrderStorageKey::Order(key)
        //    Panic if not found: Error::OrderNotFound
        //    Panic if frozen: Error::OrderFrozen
        //
        // 3. Load market props from data_store (same load_market_props pattern as deposit handler)
        //    index_token = ds.get_address(market_index_token_key(&env, &order.market))
        //    long_token  = ds.get_address(market_long_token_key(&env, &order.market))
        //    short_token = ds.get_address(market_short_token_key(&env, &order.market))
        //    market = MarketProps { market_token: order.market, index_token, long_token, short_token }
        //
        // 4. Fetch oracle prices:
        //    index_price     = oracle.get_primary_price(index_token)
        //    long_price      = oracle.get_primary_price(long_token)
        //    short_price     = oracle.get_primary_price(short_token)
        //    collateral_price = oracle.get_primary_price(initial_collateral_token).mid_price()
        //
        // 5. TRIGGER PRICE CHECK (for limit and stop-loss orders):
        //    LimitIncrease:    index_price.min <= order.trigger_price (entry at or below trigger)
        //    LimitDecrease:    index_price.max >= order.trigger_price (exit at or above trigger)
        //    StopLossDecrease: index_price.min <= order.trigger_price (exit if price drops below)
        //    MarketSwap / MarketIncrease / MarketDecrease: no trigger check needed
        //    Panic Error::UnsatisfiedTrigger if not met
        //
        // 6. DISPATCH by order.order_type:
        //
        //    OrderType::MarketSwap | OrderType::LimitSwap:
        //      // Transfer collateral from vault to first market pool, then multi-hop swap
        //      order_vault_client.transfer_out(&caller, initial_collateral_token,
        //                                      &path[0].market_token_addr, collateral_delta_amount)
        //      swap_utils::swap_with_path(env, ds, caller, oracle, initial_collateral_token,
        //                                 collateral_delta_amount, swap_path, receiver)
        //      Validate output >= order.min_output_amount → panic "min output not met"
        //
        //    OrderType::MarketIncrease | OrderType::LimitIncrease:
        //      // Transfer collateral from vault to market pool so the increase handler finds it
        //      order_vault_client.transfer_out(&caller, initial_collateral_token,
        //                                      &market.market_token, collateral_delta_amount)
        //      result = increase_position_utils::increase_position(env, IncreasePositionParams {
        //          data_store, caller, account: order.account, receiver: order.receiver,
        //          market, collateral_token: initial_collateral_token,
        //          size_delta_usd, collateral_amount: collateral_delta_amount,
        //          acceptable_price, is_long, index_token_price: index_price,
        //          collateral_price, current_time,
        //      })
        //
        //    OrderType::MarketDecrease | OrderType::LimitDecrease |
        //    OrderType::StopLossDecrease | OrderType::Liquidation:
        //      result = decrease_position_utils::decrease_position(env, DecreasePositionParams {
        //          data_store, caller, account: order.account, receiver: order.receiver,
        //          market, collateral_token: initial_collateral_token,
        //          size_delta_usd, acceptable_price, is_long, index_token_price: index_price,
        //          collateral_price, current_time,
        //      })
        //
        // 7. Remove order from storage and indexes after successful execution:
        //    Remove from handler persistent storage
        //    ds.remove_bytes32_from_set(order_list_key, key)
        //    ds.remove_bytes32_from_set(account_order_list_key, key)
        //
        // 8. Emit "order_executed" event
        todo!()
    }

    /// Cancel a pending order and refund collateral to the account.
    ///
    /// Only the order's original account can cancel (or CONTROLLER with a force flag).
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        // TODO: (mirrors GMX OrderHandler.cancelOrder)
        //
        // 1. caller.require_auth()
        // 2. Load order → panic if not found
        // 3. Validate caller == order.account OR caller has CONTROLLER role
        //
        // 4. For increase/swap order types (collateral is in vault):
        //    order_vault_client.transfer_out(&caller, initial_collateral_token,
        //                                    &order.account, collateral_delta_amount)
        //
        // 5. Remove order from handler storage and data_store indexes (same as execute)
        //
        // 6. Emit "order_cancelled" event
        todo!()
    }

    /// Update a pending order's trigger or acceptable price, or size delta.
    ///
    /// Only the order's account can call this.
    pub fn update_order(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        size_delta_usd: i128,
        acceptable_price: i128,
        trigger_price: i128,
        min_output_amount: i128,
    ) {
        // TODO: (mirrors GMX OrderHandler.updateOrder)
        //
        // 1. caller.require_auth()
        // 2. Load order → panic if not found
        // 3. Validate caller == order.account
        //
        // 4. Update mutable fields on OrderProps:
        //    order.size_delta_usd    = size_delta_usd
        //    order.acceptable_price  = acceptable_price
        //    order.trigger_price     = trigger_price
        //    order.min_output_amount = min_output_amount
        //    order.updated_at_time   = env.ledger().timestamp()
        //
        // 5. Persist updated order back to handler storage
        //
        // 6. Emit "order_updated" event
        todo!()
    }

    /// Freeze an order that cannot currently be executed (e.g. oracle failure).
    ///
    /// Keepers call this to prevent repeated execution attempts.
    /// Frozen orders can still be cancelled by the user.
    pub fn freeze_order(env: Env, keeper: Address, key: BytesN<32>) {
        // TODO: (mirrors GMX OrderHandler.freezeOrder)
        //
        // 1. keeper.require_auth()
        //    Require keeper has ORDER_KEEPER role
        //
        // 2. Load order → panic if not found
        //
        // 3. Set frozen flag:
        //    env.storage().persistent().set(&OrderStorageKey::OrderFrozen(key), &true)
        //
        // 4. Emit "order_frozen" event
        todo!()
    }

    /// Return a stored order by key, or None if not found.
    pub fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps> {
        // TODO: env.storage().persistent().get(&OrderStorageKey::Order(key))
        todo!()
    }
}
