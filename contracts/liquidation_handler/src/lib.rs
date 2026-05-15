//! Liquidation handler — forcibly close under-collateralised positions.
//! Mirrors GMX's LiquidationHandler.sol.
//!
//! This handler validates the keeper's role and position health, then delegates
//! the actual close to `order_handler::liquidate_position` since positions are
//! stored in order_handler's persistent storage.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env, symbol_short,
};
use gmx_types::{MarketProps, PriceProps, PositionProps};
use gmx_keys::{
    roles,
    market_index_token_key, market_long_token_key, market_short_token_key,
    position_key,
};
use gmx_position_utils::is_liquidatable;

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    OrderHandler,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized  = 1,
    Unauthorized        = 3,
    NotLiquidatable     = 5,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

/// Minimal interface to order_handler for position storage access.
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn liquidate_position(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    );
    fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps>;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct LiquidationHandler;

#[contractimpl]
impl LiquidationHandler {
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
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
        env.storage().instance().set(&InstanceKey::OrderHandler, &order_handler);
    }

    /// Check if a position is currently liquidatable.
    pub fn check_liquidatable(
        env: Env,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) -> bool {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let order_handler: Address = env.storage().instance().get(&InstanceKey::OrderHandler).unwrap();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price      = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client.get_primary_price(&collateral_token).mid_price();

        // Read position from order_handler via a view call
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps = match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
            Some(p) => p,
            None => return false,
        };

        is_liquidatable(&env, &data_store, &position, &market_props, collateral_price, &index_price)
    }

    /// Liquidate a position that is below the minimum collateral threshold.
    ///
    /// Validates health then delegates the actual close to order_handler (where positions live).
    pub fn liquidate_position(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) {
        keeper.require_auth();

        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::liquidation_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let order_handler: Address = env.storage().instance().get(&InstanceKey::OrderHandler).unwrap();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price      = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client.get_primary_price(&collateral_token).mid_price();

        // Verify position is actually liquidatable before delegating
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps = match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
            Some(p) => p,
            None => panic!("position not found"),
        };

        if !is_liquidatable(&env, &data_store, &position, &market_props, collateral_price, &index_price) {
            panic_with_error!(&env, Error::NotLiquidatable);
        }

        // Delegate execution to order_handler (positions live there)
        OrderHandlerClient::new(&env, &order_handler)
            .liquidate_position(&keeper, &account, &market, &collateral_token, &is_long);

        env.events().publish(
            (symbol_short!("liq_req"),),
            (account, market, is_long),
        );
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds.get_address(&market_index_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market index token not found"));
    let long_token = ds.get_address(&market_long_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market long token not found"));
    let short_token = ds.get_address(&market_short_token_key(env, market_token))
        .unwrap_or_else(|| panic!("market short token not found"));
    MarketProps { market_token: market_token.clone(), index_token, long_token, short_token }
}
