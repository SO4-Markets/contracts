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
    symbol_short, panic_with_error, token,
};
use gmx_types::{MarketProps, OrderProps, OrderType, PriceProps};
use gmx_keys::{
    roles,
    order_key, order_list_key, account_order_list_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
};
use gmx_increase_position_utils::{IncreasePositionParams, increase_position};
use gmx_decrease_position_utils::{DecreasePositionParams, decrease_position};
use gmx_swap_utils::swap_with_path;
use gmx_position_utils::is_liquidatable;
use gmx_types::PositionProps;

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    OrderVault,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
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
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn increment_nonce(env: Env, caller: Address) -> u64;
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

// ─── Position storage key (must match increase/decrease position utils) ───────

/// Positions are stored in this contract's persistent storage under this key.
/// The #[contracttype] XDR encoding must match the one in increase/decrease_position_utils.
#[contracttype]
pub enum PositionStorageKey {
    Position(BytesN<32>),
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
    pub market:                     Address,  // market_token address
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
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
        env.storage().instance().set(&InstanceKey::Oracle, &oracle);
        env.storage().instance().set(&InstanceKey::OrderVault, &order_vault);
    }

    /// Create a new order and pull collateral into the order vault.
    ///
    /// For increase/swap order types: caller must have already transferred
    /// collateral to the order_vault; we call record_transfer_in to snapshot it.
    /// Returns the order key.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        caller.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault).unwrap();
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        // Record collateral arrival for increase/swap orders
        let is_increase_or_swap = matches!(
            params.order_type,
            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease |
            OrderType::MarketSwap     | OrderType::LimitSwap
        );
        let collateral_delta_amount = if is_increase_or_swap {
            let received = OrderVaultClient::new(&env, &order_vault)
                .record_transfer_in(&params.initial_collateral_token);
            received.max(0)
        } else {
            params.collateral_delta_amount
        };

        // Generate unique key
        let nonce = ds.increment_nonce(&handler);
        let key = order_key(&env, nonce);

        let order = OrderProps {
            account:                  caller.clone(),
            receiver:                 params.receiver,
            market:                   params.market.clone(),
            initial_collateral_token: params.initial_collateral_token,
            swap_path:                params.swap_path,
            size_delta_usd:           params.size_delta_usd,
            collateral_delta_amount,
            trigger_price:            params.trigger_price,
            acceptable_price:         params.acceptable_price,
            execution_fee:            params.execution_fee,
            min_output_amount:        params.min_output_amount,
            order_type:               params.order_type,
            is_long:                  params.is_long,
            updated_at_time:          env.ledger().timestamp(),
        };

        env.storage().persistent().set(&OrderStorageKey::Order(key.clone()), &order);

        ds.add_bytes32_to_set(&handler, &order_list_key(&env), &key);
        ds.add_bytes32_to_set(&handler, &account_order_list_key(&env, &caller), &key);

        env.events().publish((symbol_short!("ord_crt"),), (key.clone(), caller, params.market));
        key
    }

    /// Execute a pending order (called by keeper).
    pub fn execute_order(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let handler = env.current_contract_address();

        // Load order
        let order: OrderProps = env.storage().persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        // Check frozen
        let is_frozen: bool = env.storage().persistent()
            .get(&OrderStorageKey::OrderFrozen(key.clone()))
            .unwrap_or(false);
        if is_frozen {
            panic_with_error!(&env, Error::OrderFrozen);
        }

        // Load market props
        let market = load_market_props(&env, &data_store, &order.market);

        // Fetch oracle prices
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price   = oracle_client.get_primary_price(&market.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&order.initial_collateral_token)
            .mid_price();

        // Trigger price checks for non-market orders
        match order.order_type {
            OrderType::LimitIncrease => {
                // Enter at or below trigger price
                if index_price.min > order.trigger_price {
                    panic_with_error!(&env, Error::UnsatisfiedTrigger);
                }
            }
            OrderType::LimitDecrease => {
                // Exit at or above trigger price
                if index_price.max < order.trigger_price {
                    panic_with_error!(&env, Error::UnsatisfiedTrigger);
                }
            }
            OrderType::StopLossDecrease => {
                // Exit if price drops to/below trigger
                if index_price.min > order.trigger_price {
                    panic_with_error!(&env, Error::UnsatisfiedTrigger);
                }
            }
            _ => {}
        }

        // Dispatch by order type
        match order.order_type {
            OrderType::MarketSwap | OrderType::LimitSwap => {
                // Transfer collateral from vault to first market in path
                let first_market = order.swap_path.get(0)
                    .unwrap_or_else(|| panic!("empty swap path"));
                OrderVaultClient::new(&env, &order_vault).transfer_out(
                    &handler,
                    &order.initial_collateral_token,
                    &first_market,
                    &order.collateral_delta_amount,
                );
                let (_token_out, amount_out) = swap_with_path(
                    &env, &data_store, &handler, &oracle,
                    &order.initial_collateral_token,
                    order.collateral_delta_amount,
                    &order.swap_path,
                    &order.receiver,
                );
                if amount_out < order.min_output_amount {
                    panic!("min output not met");
                }
            }

            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease => {
                // Transfer collateral from vault into the market pool
                OrderVaultClient::new(&env, &order_vault).transfer_out(
                    &handler,
                    &order.initial_collateral_token,
                    &market.market_token,
                    &order.collateral_delta_amount,
                );
                increase_position(&env, &IncreasePositionParams {
                    data_store:        &data_store,
                    caller:            &handler,
                    account:           &order.account,
                    receiver:          &order.receiver,
                    market:            &market,
                    collateral_token:  &order.initial_collateral_token,
                    size_delta_usd:    order.size_delta_usd,
                    collateral_amount: order.collateral_delta_amount,
                    acceptable_price:  order.acceptable_price,
                    is_long:           order.is_long,
                    index_token_price: &index_price,
                    collateral_price,
                    current_time:      env.ledger().timestamp(),
                });
            }

            OrderType::MarketDecrease | OrderType::LimitDecrease |
            OrderType::StopLossDecrease | OrderType::Liquidation => {
                decrease_position(&env, &DecreasePositionParams {
                    data_store:        &data_store,
                    caller:            &handler,
                    account:           &order.account,
                    receiver:          &order.receiver,
                    market:            &market,
                    collateral_token:  &order.initial_collateral_token,
                    size_delta_usd:    order.size_delta_usd,
                    acceptable_price:  order.acceptable_price,
                    is_long:           order.is_long,
                    index_token_price: &index_price,
                    collateral_price,
                    current_time:      env.ledger().timestamp(),
                });
            }
        }

        // Remove order
        remove_order(&env, &data_store, &handler, &key, &order.account);

        env.events().publish((symbol_short!("ord_exe"),), (key, order.account));
    }

    /// Cancel a pending order and refund collateral to the account.
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault).unwrap();
        let role_store: Address  = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        let handler = env.current_contract_address();

        let order: OrderProps = env.storage().persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        let is_keeper = RoleStoreClient::new(&env, &role_store)
            .has_role(&caller, &roles::order_keeper(&env));
        if caller != order.account && !is_keeper {
            panic_with_error!(&env, Error::Unauthorized);
        }

        // Refund collateral for increase/swap order types
        let needs_refund = matches!(
            order.order_type,
            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease |
            OrderType::MarketSwap     | OrderType::LimitSwap
        );
        if needs_refund && order.collateral_delta_amount > 0 {
            OrderVaultClient::new(&env, &order_vault).transfer_out(
                &handler,
                &order.initial_collateral_token,
                &order.account,
                &order.collateral_delta_amount,
            );
        }

        remove_order(&env, &data_store, &handler, &key, &order.account);

        env.events().publish((symbol_short!("ord_can"),), (key, order.account));
    }

    /// Update a pending order's trigger/acceptable price or size delta.
    pub fn update_order(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        size_delta_usd: i128,
        acceptable_price: i128,
        trigger_price: i128,
        min_output_amount: i128,
    ) {
        caller.require_auth();

        let mut order: OrderProps = env.storage().persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        if caller != order.account {
            panic_with_error!(&env, Error::Unauthorized);
        }

        order.size_delta_usd    = size_delta_usd;
        order.acceptable_price  = acceptable_price;
        order.trigger_price     = trigger_price;
        order.min_output_amount = min_output_amount;
        order.updated_at_time   = env.ledger().timestamp();

        env.storage().persistent().set(&OrderStorageKey::Order(key.clone()), &order);

        // Clear frozen flag if set (order is being updated = re-enabled)
        env.storage().persistent().remove(&OrderStorageKey::OrderFrozen(key.clone()));

        env.events().publish((symbol_short!("ord_upd"),), (key, caller));
    }

    /// Freeze an order that cannot currently be executed.
    pub fn freeze_order(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let _order: OrderProps = env.storage().persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        env.storage().persistent().set(&OrderStorageKey::OrderFrozen(key.clone()), &true);

        env.events().publish((symbol_short!("ord_frz"),), key);
    }

    /// Return a stored order by key, or None if not found.
    pub fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps> {
        env.storage().persistent().get(&OrderStorageKey::Order(key))
    }

    /// Return a stored position by its position_key (sha256 hash), or None.
    /// Used by liquidation_handler and adl_handler to check position health.
    pub fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps> {
        env.storage().persistent().get(&PositionStorageKey::Position(key))
    }

    /// Force-liquidate a position. Called by the liquidation_handler after role/health checks.
    ///
    /// Positions live in order_handler storage, so liquidation must run here.
    pub fn liquidate_position(
        env: Env,
        keeper: Address,  // must have LIQUIDATION_KEEPER role
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) {
        keeper.require_auth();
        require_liquidation_keeper(&env, &keeper);

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let handler = env.current_contract_address();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price      = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client.get_primary_price(&collateral_token).mid_price();

        // Load position to get size
        use gmx_keys::position_key;
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps = env.storage().persistent()
            .get(&PositionStorageKey::Position(pk.clone()))
            .unwrap_or_else(|| panic!("position not found"));

        // Validate liquidatability
        if !gmx_position_utils::is_liquidatable(
            &env, &data_store, &position, &market_props, collateral_price, &index_price,
        ) {
            panic!("position not liquidatable");
        }

        let result = decrease_position(&env, &DecreasePositionParams {
            data_store:        &data_store,
            caller:            &handler,
            account:           &account,
            receiver:          &account,
            market:            &market_props,
            collateral_token:  &collateral_token,
            size_delta_usd:    position.size_in_usd,
            acceptable_price:  0,
            is_long,
            index_token_price: &index_price,
            collateral_price,
            current_time:      env.ledger().timestamp(),
        });

        env.events().publish(
            (symbol_short!("liq_exe"),),
            (account, market, result.pnl_usd, result.execution_price),
        );
    }

    /// Partially close a profitable position for ADL. Called by adl_handler after checks.
    pub fn execute_adl(
        env: Env,
        keeper: Address,  // must have ADL_KEEPER role
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    ) {
        keeper.require_auth();
        require_adl_keeper(&env, &keeper);

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let handler = env.current_contract_address();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price      = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client.get_primary_price(&collateral_token).mid_price();

        let result = decrease_position(&env, &DecreasePositionParams {
            data_store:        &data_store,
            caller:            &handler,
            account:           &account,
            receiver:          &account,
            market:            &market_props,
            collateral_token:  &collateral_token,
            size_delta_usd,
            acceptable_price:  0,
            is_long,
            index_token_price: &index_price,
            collateral_price,
            current_time:      env.ledger().timestamp(),
        });

        env.events().publish(
            (symbol_short!("adl_exe"),),
            (account, market, size_delta_usd, result.pnl_usd),
        );
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn require_order_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::order_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_liquidation_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::liquidation_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_adl_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::adl_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds.get_address(&market_index_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market index token not found"));
    let long_token = ds.get_address(&market_long_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market long token not found"));
    let short_token = ds.get_address(&market_short_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market short token not found"));
    MarketProps {
        market_token: market_token.clone(),
        index_token,
        long_token,
        short_token,
    }
}

fn remove_order(env: &Env, data_store: &Address, caller: &Address, key: &BytesN<32>, account: &Address) {
    env.storage().persistent().remove(&OrderStorageKey::Order(key.clone()));
    env.storage().persistent().remove(&OrderStorageKey::OrderFrozen(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(caller, &order_list_key(env), key);
    ds.remove_bytes32_from_set(caller, &account_order_list_key(env, account), key);
}
