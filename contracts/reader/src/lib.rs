//! Reader — read-only view contract for aggregating protocol state.
//! Mirrors GMX's Reader.sol.
//!
//! Aggregates data across data_store, oracle, and position/market utils
//! into rich structs the frontend consumes without needing multiple calls.
//! All functions are view-only — no writes, no auth.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contractimpl, Address, BytesN, Env, Vec,
};
use gmx_types::{
    MarketProps, PositionProps, PositionInfo, PositionFees, PriceProps,
    PoolValueInfo, FundingInfo,
};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    funding_amount_per_size_key, saved_funding_factor_per_second_key,
};
use gmx_market_utils::{get_pool_value, get_open_interest_for_side, get_pnl};
use gmx_position_utils::{get_position_pnl_usd, get_position_fees, is_liquidatable};
use gmx_pricing_utils::get_execution_price;

// ─── External clients ─────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct Reader;

#[contractimpl]
impl Reader {
    // ── Market views ─────────────────────────────────────────────────────────

    /// Load full MarketProps for a given market_token address from data_store.
    pub fn get_market(env: Env, data_store: Address, market_token: Address) -> MarketProps {
        // TODO:
        // index_token = ds.get_address(market_index_token_key(&env, &market_token)).unwrap()
        // long_token  = ds.get_address(market_long_token_key(&env, &market_token)).unwrap()
        // short_token = ds.get_address(market_short_token_key(&env, &market_token)).unwrap()
        // Return MarketProps { market_token, index_token, long_token, short_token }
        todo!()
    }

    /// Get the full pool value breakdown for a market at current oracle prices.
    pub fn get_market_pool_value_info(
        env: Env,
        data_store: Address,
        oracle: Address,
        market_token: Address,
        maximize: bool,
    ) -> PoolValueInfo {
        // TODO:
        // 1. market = get_market(env, data_store, market_token)
        // 2. Fetch oracle prices for long, short, index tokens
        // 3. Return get_pool_value(env, ds, market, long_price, short_price, index_price, maximize)
        todo!()
    }

    /// Get open interest for both sides of a market.
    /// Returns (long_oi_usd, short_oi_usd).
    pub fn get_open_interest(
        env: Env,
        data_store: Address,
        market_token: Address,
    ) -> (i128, i128) {
        // TODO:
        // market = get_market(env, data_store, market_token)
        // long_oi  = get_open_interest_for_side(env, ds, market, true)  as i128
        // short_oi = get_open_interest_for_side(env, ds, market, false) as i128
        // Return (long_oi, short_oi)
        todo!()
    }

    /// Get the aggregate funding state for a market.
    pub fn get_funding_info(
        env: Env,
        data_store: Address,
        market_token: Address,
    ) -> FundingInfo {
        // TODO:
        // 1. key_factor = saved_funding_factor_per_second_key(&env, &market_token)
        //    funding_factor_per_second = ds.get_i128(key_factor)
        // 2. long_key  = funding_amount_per_size_key(&env, &market_token, true)
        //    short_key = funding_amount_per_size_key(&env, &market_token, false)
        //    long_funding_amount_per_size  = ds.get_i128(long_key)
        //    short_funding_amount_per_size = ds.get_i128(short_key)
        // 3. Return FundingInfo { funding_factor_per_second,
        //                         long_funding_amount_per_size,
        //                         short_funding_amount_per_size }
        todo!()
    }

    // ── Position views ────────────────────────────────────────────────────────

    /// Get a single position enriched with PnL, fees, and liquidation price.
    ///
    /// `position` must be supplied by caller (loaded from order_handler storage
    /// via cross-contract, or passed directly from the frontend's cached data).
    pub fn get_position_info(
        env: Env,
        data_store: Address,
        oracle: Address,
        position: PositionProps,
    ) -> PositionInfo {
        // TODO: (mirrors GMX Reader.getPositionInfo)
        //
        // 1. Load MarketProps for position.market from data_store
        //
        // 2. Fetch prices:
        //    index_price      = oracle.get_primary_price(market.index_token)
        //    collateral_price = oracle.get_primary_price(position.collateral_token).mid_price()
        //
        // 3. Compute PnL:
        //    (pnl_usd, uncapped_pnl_usd) = get_position_pnl_usd(
        //        env, &position, &index_price, position.size_in_usd)
        //
        // 4. Compute fees:
        //    fees = get_position_fees(env, ds, market, position, collateral_price,
        //                             position.size_in_usd, for_positive_impact=false)
        //    borrowing_fee_usd  = fees.borrowing_fee_amount * collateral_price / TOKEN_PRECISION
        //    funding_fee_usd    = fees.funding_fee_amount   * collateral_price / TOKEN_PRECISION
        //    position_fee_usd   = fees.position_fee_amount  * collateral_price / TOKEN_PRECISION
        //
        // 5. Compute liquidation price (approximate):
        //    Solve for the index_price at which remaining_collateral == min_collateral_required:
        //    For a long: liq_price = (position.size_in_usd - collateral_usd + fees_usd)
        //                            / position.size_in_tokens * TOKEN_PRECISION
        //    For a short: liq_price = (position.size_in_usd + collateral_usd - fees_usd)
        //                             / position.size_in_tokens * TOKEN_PRECISION
        //    (simplified; GMX uses a more precise iterative solve)
        //
        // 6. Return PositionInfo { position, pnl_usd, uncapped_pnl_usd,
        //                          borrowing_fee_usd, funding_fee_usd, position_fee_usd,
        //                          liquidation_price }
        todo!()
    }

    /// Compute the execution price a user would get for a given size and order direction.
    ///
    /// Useful for the UI to preview slippage before placing an order.
    pub fn get_execution_price_preview(
        env: Env,
        data_store: Address,
        oracle: Address,
        market_token: Address,
        is_long: bool,
        is_increase: bool,
        size_delta_usd: i128,
    ) -> i128 {
        // TODO:
        // 1. market = get_market(env, data_store, market_token)
        // 2. index_price = oracle.get_primary_price(market.index_token).mid_price()
        // 3. impact_usd = get_position_price_impact(env, ds, market, is_long,
        //                                           size_delta_usd, is_increase)
        // 4. Return get_execution_price(env, index_price, size_delta_usd, impact_usd,
        //                               is_long, is_increase)
        todo!()
    }

    /// Return whether a position is currently liquidatable at oracle prices.
    pub fn is_position_liquidatable(
        env: Env,
        data_store: Address,
        oracle: Address,
        position: PositionProps,
    ) -> bool {
        // TODO:
        // 1. market = get_market(env, data_store, position.market)
        // 2. index_price = oracle.get_primary_price(market.index_token)
        //    collateral_price = oracle.get_primary_price(position.collateral_token).mid_price()
        // 3. Return is_liquidatable(env, ds, position, market, collateral_price, index_price)
        todo!()
    }
}
