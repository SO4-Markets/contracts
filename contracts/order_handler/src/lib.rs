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
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env,
    symbol_short, panic_with_error,
};
use gmx_types::{MarketProps, OrderProps, OrderType, PriceProps};
pub use gmx_types::CreateOrderParams;
use gmx_keys::{
    roles,
    order_key, order_list_key, account_order_list_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
};
use gmx_increase_position_utils::{IncreasePositionParams, increase_position};
use gmx_decrease_position_utils::{DecreasePositionParams, decrease_position};
use gmx_swap_utils::swap_with_path;
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
    /// Increase/swap orders require collateral to have been transferred to
    /// order_vault (via exchange_router SendTokens) before calling create_order.
    /// record_transfer_in returned zero, meaning no collateral arrived.
    ZeroCollateral        = 10,
}

// ─── External contract clients ────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn increment_nonce(env: Env, caller: Address) -> u64;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[allow(dead_code)]
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

    /// Create a new order and record collateral in the order vault.
    ///
    /// # Collateral model (canonical — issue #47)
    ///
    /// **Chosen path:** the exchange_router is responsible for transferring
    /// collateral from the caller to order_vault BEFORE invoking create_order.
    /// order_handler then calls `record_transfer_in` to snapshot the delta.
    ///
    /// **Why this model:**
    /// - The router owns the auth context for the caller's token approval.
    /// - Keeping the pull inside the router makes the vault a passive custodian
    ///   with no token-approval dependencies of its own.
    /// - Handlers never hold approvals, so they cannot silently double-pull.
    ///
    /// **Invariant enforced here:**
    /// - For increase/swap orders: `record_transfer_in` delta MUST be > 0.
    ///   A zero delta means tokens were not pre-sent; the transaction reverts
    ///   with `ZeroCollateral` before any state is written.
    /// - For decrease/stop-loss/liquidation orders: no collateral is deposited;
    ///   `collateral_delta_amount` comes from params (typically the existing position size).
    ///
    /// **Multicall sequence the router enforces for increase/swap orders:**
    /// ```text
    /// multicall([
    ///   SendTokens { token, receiver: order_vault, amount },   // 1. push collateral
    ///   CreateOrder { params },                                 // 2. snapshot + store order
    /// ])
    /// ```
    ///
    /// Returns the order key.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        caller.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        // Determine whether this order type requires upfront collateral in the vault.
        // Increase and swap orders pull from the vault; decrease orders do not deposit.
        let is_increase_or_swap = matches!(
            params.order_type,
            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease |
            OrderType::MarketSwap     | OrderType::LimitSwap
        );

        // Snapshot vault balance and derive received amount (canonical model — issue #47).
        // Reverts with ZeroCollateral if caller skipped the SendTokens pre-step.
        let collateral_delta_amount = if is_increase_or_swap {
            let received = OrderVaultClient::new(&env, &order_vault)
                .record_transfer_in(&params.initial_collateral_token);
            if received <= 0 {
                panic_with_error!(&env, Error::ZeroCollateral);
            }
            received
        } else {
            // Decrease/liquidation orders: no collateral deposit required.
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

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
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
            OrderType::LimitIncrease if index_price.min > order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            OrderType::LimitDecrease if index_price.max < order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            OrderType::StopLossDecrease if index_price.min > order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            _ => {}
        }

        // Dispatch by order type
        match order.order_type {
            OrderType::MarketSwap | OrderType::LimitSwap => {
                // Transfer collateral from vault to first market in path
                let first_market = order.swap_path.get(0)
                    .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidOrderType));
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
                    panic_with_error!(&env, Error::PriceTooLow);
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

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let role_store: Address  = env.storage().instance().get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
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

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
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
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        // Validate liquidatability
        if !gmx_position_utils::is_liquidatable(
            &env, &data_store, &position, &market_props, collateral_price, &index_price,
        ) {
            panic_with_error!(&env, Error::InvalidOrderType);
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

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
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
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::order_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_liquidation_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::liquidation_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_adl_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::adl_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds.get_address(&market_index_token_key(env, market_token))
        .expect("market index token not found");
    let long_token = ds.get_address(&market_long_token_key(env, market_token))
        .expect("market long token not found");
    let short_token = ds.get_address(&market_short_token_key(env, market_token))
        .expect("market short token not found");
    MarketProps { 
        market_token: market_token.clone(), 
        index_token, 
        long_token, 
        short_token 
    }
}

fn remove_order(env: &Env, data_store: &Address, caller: &Address, key: &BytesN<32>, account: &Address) {
    env.storage().persistent().remove(&OrderStorageKey::Order(key.clone()));
    env.storage().persistent().remove(&OrderStorageKey::OrderFrozen(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(caller, &order_list_key(env), key);
    ds.remove_bytes32_from_set(caller, &account_order_list_key(env, account), key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env, Vec};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use gmx_keys::{roles, position_key};
    use gmx_types::TokenPrice;

    /// 1 whole token at 7-decimal Stellar precision.
    const COLLATERAL: i128 = 1_000_0000;

    struct World {
        env:       Env,
        admin:     Address,
        keeper:    Address,
        user:      Address,
        rs:        Address,
        ds:        Address,
        oracle:    Address,
        vault:     Address,
        handler:   Address,
        market_tk: Address,
        long_tk:   Address,
        short_tk:  Address,
        index_tk:  Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin  = Address::generate(&env);
        let keeper = Address::generate(&env);
        let user   = Address::generate(&env);

        // — Role store —
        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin,  &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        // — Data store —
        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        // — Oracle —
        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        // — Order vault —
        let vault = env.register(OrderVault, ());
        OVClient::new(&env, &vault).initialize(&admin, &rs);

        // — Market token (LP + pool custodian) —
        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin, &rs, &7u32,
            &soroban_sdk::String::from_str(&env, "SO4 Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        // — Order handler —
        let handler = env.register(OrderHandler, ());
        OrderHandlerClient::new(&env, &handler).initialize(
            &admin, &rs, &ds, &oracle_addr, &vault,
        );

        // Grant handler CONTROLLER: needed for DS writes and vault.transfer_out
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        // — Underlying tokens (SEP-41 stellar assets support minting in tests) —
        let long_tk  = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let index_tk = Address::generate(&env);

        // Register market in DataStore
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(&handler, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
        ds_c.set_address(&handler, &gmx_keys::market_long_token_key (&env, &market_tk), &long_tk);
        ds_c.set_address(&handler, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

        World { env, admin, keeper, user, rs, ds, oracle: oracle_addr, vault, handler, market_tk, long_tk, short_tk, index_tk }
    }

    /// Set oracle prices: long_tk = $2000, short_tk = $1, index_tk = $2000.
    fn set_prices(w: &World) {
        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(&w.keeper, &Vec::from_array(&w.env, [
            TokenPrice { token: w.long_tk.clone(),  min: 2000 * fp, max: 2000 * fp },
            TokenPrice { token: w.short_tk.clone(), min: fp,        max: fp },
            TokenPrice { token: w.index_tk.clone(), min: 2000 * fp, max: 2000 * fp },
        ]));
    }

    /// Fund the vault with COLLATERAL of long_tk then call create_order.
    /// Returns (client, order_key).
    fn create_increase_order(
        w: &World,
        order_type: OrderType,
        trigger_price: i128,
    ) -> (OrderHandlerClient, BytesN<32>) {
        // Simulates the SendTokens step that the exchange_router multicall does
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.vault, &COLLATERAL);

        let hc = OrderHandlerClient::new(&w.env, &w.handler);
        let key = hc.create_order(&w.user, &CreateOrderParams {
            receiver:                  w.user.clone(),
            market:                    w.market_tk.clone(),
            initial_collateral_token:  w.long_tk.clone(),
            swap_path:                 Vec::new(&w.env),
            size_delta_usd:            2000 * gmx_math::FLOAT_PRECISION,
            collateral_delta_amount:   COLLATERAL,
            trigger_price,
            acceptable_price:          0,  // 0 = no slippage check
            execution_fee:             0,
            min_output_amount:         0,
            order_type,
            is_long:                   true,
        });
        (hc, key)
    }

    // ── Issue #47: collateral model guard ────────────────────────────────────

    /// Creating an increase/swap order without pre-depositing collateral in the
    /// vault must revert with ZeroCollateral (canonical model — issue #47).
    #[test]
    #[should_panic]
    fn create_order_without_collateral_reverts() {
        let w = setup();
        // Vault has no tokens → record_transfer_in returns 0 → ZeroCollateral
        OrderHandlerClient::new(&w.env, &w.handler).create_order(&w.user, &CreateOrderParams {
            receiver:                  w.user.clone(),
            market:                    w.market_tk.clone(),
            initial_collateral_token:  w.long_tk.clone(),
            swap_path:                 Vec::new(&w.env),
            size_delta_usd:            2000 * gmx_math::FLOAT_PRECISION,
            collateral_delta_amount:   COLLATERAL,
            trigger_price:             0,
            acceptable_price:          0,
            execution_fee:             0,
            min_output_amount:         0,
            order_type:                OrderType::MarketIncrease,
            is_long:                   true,
        });
    }

    /// Vault's recorded balance must equal its on-chain balance immediately after
    /// create_order (balance snapshot invariant — issue #47).
    #[test]
    fn vault_balance_invariant_holds_after_create() {
        let w = setup();
        let (_hc, _key) = create_increase_order(&w, OrderType::MarketIncrease, 0);

        let ov = OVClient::new(&w.env, &w.vault);
        let recorded = ov.get_recorded_balance(&w.long_tk);
        let on_chain = soroban_sdk::token::Client::new(&w.env, &w.long_tk)
            .balance(&w.vault);
        assert_eq!(recorded, on_chain,   "vault recorded ≠ on-chain balance");
        assert_eq!(recorded, COLLATERAL, "vault should hold exactly the deposited collateral");
    }

    /// cancel_order must refund the full collateral to the account and update the
    /// vault's recorded balance to zero.
    #[test]
    fn cancel_order_refunds_collateral_to_user() {
        let w = setup();
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);

        let before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);
        hc.cancel_order(&w.user, &key);
        let after  = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);

        assert_eq!(after - before, COLLATERAL, "user should receive full collateral refund");
        assert!(hc.get_order(&key).is_none(), "order must be removed after cancel");
        assert_eq!(
            OVClient::new(&w.env, &w.vault).get_recorded_balance(&w.long_tk),
            0,
            "vault recorded balance must be zero after refund"
        );
    }

    /// Submitting a second create_order for the same token without new collateral
    /// arriving at the vault must revert (double-snapshot guard — issue #47).
    #[test]
    #[should_panic]
    fn double_create_order_without_new_deposit_reverts() {
        let w = setup();
        // First order consumes the vault snapshot
        let _  = create_increase_order(&w, OrderType::MarketIncrease, 0);

        // Second order — no new tokens deposited; record_transfer_in delta = 0 → ZeroCollateral
        OrderHandlerClient::new(&w.env, &w.handler).create_order(&w.user, &CreateOrderParams {
            receiver:                  w.user.clone(),
            market:                    w.market_tk.clone(),
            initial_collateral_token:  w.long_tk.clone(),
            swap_path:                 Vec::new(&w.env),
            size_delta_usd:            2000 * gmx_math::FLOAT_PRECISION,
            collateral_delta_amount:   COLLATERAL,
            trigger_price:             0,
            acceptable_price:          0,
            execution_fee:             0,
            min_output_amount:         0,
            order_type:                OrderType::MarketIncrease,
            is_long:                   true,
        });
    }

    // ── Issue #49: limit increase order lifecycle ─────────────────────────────

    /// LimitIncrease where oracle price > trigger_price must revert with
    /// UnsatisfiedTrigger before any state mutation occurs.
    #[test]
    #[should_panic]
    fn limit_increase_unsatisfied_trigger_reverts() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        // trigger = $1 000; oracle index price = $2 000 → 2000 > 1000 → reverts
        let (hc, key) = create_increase_order(&w, OrderType::LimitIncrease, 1000 * fp);
        set_prices(&w);
        hc.execute_order(&w.keeper, &key);
    }

    /// LimitIncrease where oracle price ≤ trigger_price must succeed:
    ///   • position created with correct fields,
    ///   • open interest increased in DataStore,
    ///   • order key removed from storage.
    #[test]
    fn limit_increase_satisfied_trigger_creates_position() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        // trigger = $3 000; oracle index = $2 000 → 2000 ≤ 3000 → satisfied
        let (hc, key) = create_increase_order(&w, OrderType::LimitIncrease, 3000 * fp);
        set_prices(&w);

        hc.execute_order(&w.keeper, &key);

        // 1. Order must be removed from handler storage
        assert!(hc.get_order(&key).is_none(), "order should be removed after execution");

        // 2. Position must exist and have correct fields
        let pk       = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        let position = hc.get_position(&pk)
            .expect("position must exist after limit increase execution");

        assert!(position.size_in_usd > 0,          "position size_in_usd must be positive");
        assert_eq!(position.market, w.market_tk,   "position market must match order market");
        assert_eq!(position.is_long, true,          "position must be long");
        assert_eq!(position.collateral_token, w.long_tk, "collateral token must match");

        // 3. Long open interest in DataStore must have increased
        let ds_c    = DsClient::new(&w.env, &w.ds);
        let long_oi = ds_c.get_u128(
            &gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true)
        );
        assert!(long_oi > 0, "long open interest must increase after limit increase execution");
    }
}
