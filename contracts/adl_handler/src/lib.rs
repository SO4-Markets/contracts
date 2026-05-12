//! Auto-Deleveraging (ADL) handler — partially close profitable positions
//! when the pool's PnL-to-pool-value ratio exceeds the configured threshold.
//! Mirrors GMX's AdlHandler.sol.
//!
//! ADL is triggered when total trader PnL threatens pool solvency.
//! Keepers select a profitable position and reduce it just enough to bring
//! the PnL factor back below the max threshold, crediting the trader fairly.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, Address, BytesN, Env,
};
use gmx_types::{MarketProps, PriceProps};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_pnl_factor_key,
};
use gmx_market_utils::{get_pool_value, get_pnl};
use gmx_decrease_position_utils::{decrease_position, DecreasePositionParams};

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:      &str = "ADMIN";
const ROLE_STORE_KEY: &str = "ROLE_STORE";
const DATA_STORE_KEY: &str = "DATA_STORE";
const ORACLE_KEY:     &str = "ORACLE";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    AdlNotRequired     = 4,
    PositionNotFound   = 5,
    NotProfitable      = 6,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: soroban_sdk::Symbol) -> bool;
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

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct AdlHandler;

#[contractimpl]
impl AdlHandler {
    /// One-time setup.
    pub fn initialize(env: Env, admin: Address, role_store: Address, data_store: Address, oracle: Address) {
        // TODO: panic if already initialized
        //       Store all addresses in instance storage
        todo!()
    }

    /// Check whether ADL is currently required for the given market side.
    ///
    /// Returns true if the PnL factor (total_trader_pnl / pool_value) exceeds
    /// the configured MAX_PNL_FACTOR threshold for that side.
    pub fn is_adl_required(
        env: Env,
        market: Address,
        is_long: bool,
    ) -> bool {
        // TODO: (mirrors GMX AdlUtils.isAdlRequired)
        //
        // 1. Load data_store and oracle from instance storage
        //
        // 2. Load MarketProps from data_store (same index/long/short key pattern)
        //
        // 3. Fetch prices from oracle for index, long, short tokens
        //
        // 4. pool_value = get_pool_value(env, ds, market_props, long_price, short_price,
        //                                index_price, maximize=false)
        //    (use minimize for conservative pool value estimate)
        //
        // 5. pnl = get_pnl(env, ds, market_props, index_price.mid_price(), is_long, maximize=true)
        //    (use maximize=true to get worst-case trader PnL against the pool)
        //
        // 6. if pnl <= 0 → return false (no profitable positions to ADL)
        //
        // 7. pnl_factor = pnl * FLOAT_PRECISION / pool_value
        //    max_pnl_factor = ds.get_u128(max_pnl_factor_key(&env, &market.market_token, is_long))
        //
        // 8. Return pnl_factor > max_pnl_factor as i128
        todo!()
    }

    /// Execute ADL on a specific profitable position to reduce the market's PnL factor.
    ///
    /// The keeper identifies a profitable position and supplies `size_delta_usd` — the
    /// amount to close. The handler validates ADL is needed and that the position is
    /// profitable, then executes a partial decrease.
    pub fn execute_adl(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    ) {
        // TODO: (mirrors GMX AdlHandler.executeAdl)
        //
        // 1. keeper.require_auth()
        //    Require keeper has ADL_KEEPER role in role_store
        //
        // 2. Validate ADL is required:
        //    if !is_adl_required(env, market, is_long) → panic Error::AdlNotRequired
        //
        // 3. Load MarketProps and oracle prices (same pattern as liquidation_handler)
        //
        // 4. Load the target position from order_handler storage (cross-contract or shared)
        //    Validate position.size_in_usd >= size_delta_usd
        //
        // 5. Validate the position is currently PROFITABLE (PnL > 0):
        //    (pnl_usd, _) = get_position_pnl_usd(env, &position, &index_price, size_delta_usd)
        //    if pnl_usd <= 0 → panic Error::NotProfitable
        //    (ADL only targets profitable positions; closing losers doesn't help the pool)
        //
        // 6. Execute partial decrease:
        //    result = decrease_position(env, &DecreasePositionParams {
        //        data_store, caller: &env.current_contract_address(),
        //        account: &account, receiver: &account,
        //        market: &market_props, collateral_token: &collateral_token,
        //        size_delta_usd, acceptable_price: 0,  // no slippage check for ADL
        //        is_long, index_token_price: &index_price,
        //        collateral_price, current_time: env.ledger().timestamp(),
        //    })
        //
        // 7. Validate that post-ADL the PnL factor is now below the threshold.
        //    If still above, the keeper will need to call again with another position.
        //
        // 8. Emit "adl_executed" event with { account, market, size_delta_usd, pnl_usd }
        todo!()
    }
}
