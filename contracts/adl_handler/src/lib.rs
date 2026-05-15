//! Auto-Deleveraging (ADL) handler — partially close profitable positions
//! when the pool's PnL-to-pool-value ratio exceeds the configured threshold.
//! Mirrors GMX's AdlHandler.sol.
//!
//! Delegates actual position closure to order_handler since positions live there.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env, symbol_short,
};
use gmx_types::{MarketProps, PriceProps, PositionProps};
use gmx_math::{FLOAT_PRECISION, mul_div_wide};
use gmx_keys::{
    roles,
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_pnl_factor_key, max_pnl_factor_for_adl_key,
    position_key,
};
use gmx_market_utils::{get_pool_value, get_pnl};
use gmx_position_utils::get_position_pnl_usd;

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
    AlreadyInitialized = 1,
    Unauthorized       = 3,
    AdlNotRequired     = 4,
    NotProfitable      = 6,
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

#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn execute_adl(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    );
    fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps>;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct AdlHandler;

#[contractimpl]
impl AdlHandler {
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

    /// Check whether ADL is currently required for the given market side.
    ///
    /// Returns true if total trader PnL / pool_value > MAX_PNL_FACTOR_FOR_ADL.
    pub fn is_adl_required(env: Env, market: Address, is_long: bool) -> bool {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price_props = oracle_client.get_primary_price(&market_props.index_token);
        let long_price  = oracle_client.get_primary_price(&market_props.long_token).mid_price();
        let short_price = oracle_client.get_primary_price(&market_props.short_token).mid_price();
        let index_price = index_price_props.mid_price();

        // Minimize pool value (conservative: harder to trigger ADL)
        let pool_info = get_pool_value(
            &env, &data_store, &market_props,
            long_price, short_price, index_price, false,
        );
        if pool_info.pool_value <= 0 {
            return false;
        }

        // Maximize trader PnL (worst case for pool)
        let pnl = get_pnl(&env, &data_store, &market_props, index_price, is_long, true);
        if pnl <= 0 {
            return false;
        }

        let pnl_factor = mul_div_wide(&env, pnl, FLOAT_PRECISION, pool_info.pool_value);
        let max_pnl_factor = DataStoreClient::new(&env, &data_store)
            .get_u128(&max_pnl_factor_for_adl_key(&env, &market, is_long)) as i128;

        if max_pnl_factor == 0 {
            return false; // No limit configured
        }
        pnl_factor > max_pnl_factor
    }

    /// Execute ADL on a specific profitable position.
    ///
    /// Validates ADL is required and the position is profitable, then delegates
    /// the partial close to order_handler (where positions are stored).
    pub fn execute_adl(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    ) {
        keeper.require_auth();

        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::adl_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        // Check ADL is required
        if !AdlHandler::is_adl_required(env.clone(), market.clone(), is_long) {
            panic_with_error!(&env, Error::AdlNotRequired);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let order_handler: Address = env.storage().instance().get(&InstanceKey::OrderHandler).unwrap();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market_props.index_token);

        // Verify the target position is profitable (ADL only closes profitable positions)
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps = match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
            Some(p) => p,
            None => panic!("position not found"),
        };

        let (pnl_usd, _) = get_position_pnl_usd(&env, &position, &index_price, size_delta_usd);
        if pnl_usd <= 0 {
            panic_with_error!(&env, Error::NotProfitable);
        }

        // Delegate to order_handler
        OrderHandlerClient::new(&env, &order_handler)
            .execute_adl(&keeper, &account, &market, &collateral_token, &is_long, &size_delta_usd);

        env.events().publish(
            (symbol_short!("adl_req"),),
            (account, market, is_long, size_delta_usd, pnl_usd),
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
