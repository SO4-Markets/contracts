//! Liquidation handler — forcibly close under-collateralised positions.
//! Mirrors GMX's LiquidationHandler.sol.
//!
//! Keepers call `liquidate_position` when `position_utils::is_liquidatable` returns true.
//! Internally it dispatches a full-close DecreaseOrder with OrderType::Liquidation,
//! routing proceeds to the position's receiver and any remainder to the insurance fund.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, Address, BytesN, Env, panic_with_error,
};
use gmx_types::{MarketProps, PriceProps};
use gmx_keys::{market_index_token_key, market_long_token_key, market_short_token_key};
use gmx_position_utils::is_liquidatable;
use gmx_decrease_position_utils::{decrease_position, DecreasePositionParams};

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:      &str = "ADMIN";
const ROLE_STORE_KEY: &str = "ROLE_STORE";
const DATA_STORE_KEY: &str = "DATA_STORE";
const ORACLE_KEY:     &str = "ORACLE";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
pub enum Error {
    AlreadyInitialized  = 1,
    NotInitialized      = 2,
    Unauthorized        = 3,
    PositionNotFound    = 4,
    NotLiquidatable     = 5,
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
pub struct LiquidationHandler;

#[contractimpl]
impl LiquidationHandler {
    /// One-time setup.
    pub fn initialize(env: Env, admin: Address, role_store: Address, data_store: Address, oracle: Address) {
        // TODO: panic if already initialized (ADMIN_KEY in instance storage)
        //       Store admin, role_store, data_store, oracle in instance storage
        todo!()
    }

    /// Liquidate a position that is below the minimum collateral threshold.
    ///
    /// The keeper supplies the account address and position identifiers.
    /// This call validates liquidatability then executes a full decrease.
    pub fn liquidate_position(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) {
        // TODO: (mirrors GMX LiquidationHandler.liquidatePosition)
        //
        // 1. keeper.require_auth()
        //    Require keeper has LIQUIDATION_KEEPER role in role_store
        //
        // 2. Load data_store and oracle from instance storage
        //
        // 3. Load MarketProps from data_store:
        //    index_token = ds.get_address(market_index_token_key(&env, &market)).unwrap()
        //    long_token  = ds.get_address(market_long_token_key(&env, &market)).unwrap()
        //    short_token = ds.get_address(market_short_token_key(&env, &market)).unwrap()
        //    market_props = MarketProps { market_token: market, index_token, long_token, short_token }
        //
        // 4. Fetch oracle prices:
        //    index_price      = oracle.get_primary_price(index_token)
        //    collateral_price = oracle.get_primary_price(collateral_token).mid_price()
        //
        // 5. Load position from order_handler's storage (or a shared position store).
        //    NOTE: In the full impl, position storage lives in order_handler.
        //    The liquidation handler must either:
        //      a) Be co-deployed with order_handler and share storage (same contract), OR
        //      b) Call order_handler via cross-contract to read/close the position.
        //    Preferred: call order_handler.execute_order with a synthetic Liquidation order key.
        //    Alternatively, grant liquidation_handler CONTROLLER so it can call decrease_position
        //    directly via a cross-contract invoke on order_handler.
        //
        // 6. CHECK: is_liquidatable(env, ds, &position, &market_props, collateral_price, index_price)
        //    if !liquidatable → panic Error::NotLiquidatable
        //
        // 7. Execute full close as Liquidation type:
        //    result = decrease_position(env, &DecreasePositionParams {
        //        data_store, caller: &env.current_contract_address(),
        //        account: &account, receiver: &account,
        //        market: &market_props, collateral_token: &collateral_token,
        //        size_delta_usd: position.size_in_usd,  // full close
        //        acceptable_price: 0,                    // no slippage check for liquidations
        //        is_long, index_token_price: &index_price,
        //        collateral_price, current_time: env.ledger().timestamp(),
        //    })
        //
        // 8. If result.output_amount == 0 (collateral fully consumed by losses/fees):
        //    No transfer needed. The position is gone.
        //    Optionally mint a small "liquidation fee" to keeper from the insurance/fee pool.
        //
        // 9. Emit "position_liquidated" event with { account, market, pnl_usd, execution_price }
        todo!()
    }
}
