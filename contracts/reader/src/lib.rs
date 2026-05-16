//! Reader — read-only view contract for aggregating protocol state.
//! Mirrors GMX's Reader.sol.
//!
//! Aggregates data across data_store, oracle, and position/market utils
//! into rich structs the frontend consumes without needing multiple calls.
//! All functions are view-only — no writes, no auth.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contractimpl, Address, BytesN, Env,
};
use gmx_types::{
    MarketProps, PositionProps, PositionInfo, PositionFees, PriceProps,
    PoolValueInfo, FundingInfo,
};
use gmx_math::{TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    funding_amount_per_size_key, saved_funding_factor_per_second_key,
};
use gmx_market_utils::{get_pool_value, get_open_interest_for_side};
use gmx_position_utils::{get_position_pnl_usd, get_position_fees, is_liquidatable};
use gmx_pricing_utils::{get_execution_price, get_position_price_impact};

// ─── External clients ─────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[allow(dead_code)]
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
        let ds = DataStoreClient::new(&env, &data_store);
        let index_token = ds.get_address(&market_index_token_key(&env, &market_token))
            .unwrap_or_else(|| panic!("market index token not found"));
        let long_token = ds.get_address(&market_long_token_key(&env, &market_token))
            .unwrap_or_else(|| panic!("market long token not found"));
        let short_token = ds.get_address(&market_short_token_key(&env, &market_token))
            .unwrap_or_else(|| panic!("market short token not found"));
        MarketProps { market_token, index_token, long_token, short_token }
    }

    /// Get the full pool value breakdown for a market at current oracle prices.
    pub fn get_market_pool_value_info(
        env: Env,
        data_store: Address,
        oracle: Address,
        market_token: Address,
        maximize: bool,
    ) -> PoolValueInfo {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let oracle_client = OracleClient::new(&env, &oracle);
        let long_price  = oracle_client.get_primary_price(&market.long_token).mid_price();
        let short_price = oracle_client.get_primary_price(&market.short_token).mid_price();
        let index_price = oracle_client.get_primary_price(&market.index_token).mid_price();
        get_pool_value(&env, &data_store, &market, long_price, short_price, index_price, maximize)
    }

    /// Get open interest for both sides of a market.
    /// Returns (long_oi_usd, short_oi_usd).
    pub fn get_open_interest(
        env: Env,
        data_store: Address,
        market_token: Address,
    ) -> (i128, i128) {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let long_oi  = get_open_interest_for_side(&env, &data_store, &market, true)  as i128;
        let short_oi = get_open_interest_for_side(&env, &data_store, &market, false) as i128;
        (long_oi, short_oi)
    }

    /// Get the aggregate funding state for a market.
    pub fn get_funding_info(
        env: Env,
        data_store: Address,
        market_token: Address,
    ) -> FundingInfo {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token.clone());
        let ds = DataStoreClient::new(&env, &data_store);

        let funding_factor_per_second = ds.get_i128(
            &saved_funding_factor_per_second_key(&env, &market_token)
        );
        // Long side tracks funding in long_token collateral; short in short_token
        let long_funding_amount_per_size = ds.get_i128(
            &funding_amount_per_size_key(&env, &market_token, &market.long_token, true)
        );
        let short_funding_amount_per_size = ds.get_i128(
            &funding_amount_per_size_key(&env, &market_token, &market.short_token, false)
        );

        FundingInfo {
            funding_factor_per_second,
            long_funding_amount_per_size,
            short_funding_amount_per_size,
        }
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
        let market = Self::get_market(env.clone(), data_store.clone(), position.market.clone());
        let oracle_client = OracleClient::new(&env, &oracle);

        let index_price      = oracle_client.get_primary_price(&market.index_token);
        let collateral_price = oracle_client.get_primary_price(&position.collateral_token).mid_price();

        // PnL for the full position size
        let (pnl_usd, uncapped_pnl_usd) = get_position_pnl_usd(
            &env, &position, &index_price, position.size_in_usd,
        );

        // Fees in collateral token units
        let fees: PositionFees = get_position_fees(
            &env, &data_store, &market, &position,
            collateral_price, position.size_in_usd, false,
        );

        // Convert fee amounts (collateral token raw) → USD (FLOAT_PRECISION)
        let borrowing_fee_usd = mul_div_wide(&env, fees.borrowing_fee_amount, collateral_price, TOKEN_PRECISION);
        let funding_fee_usd   = mul_div_wide(&env, fees.funding_fee_amount,   collateral_price, TOKEN_PRECISION);
        let position_fee_usd  = mul_div_wide(&env, fees.position_fee_amount,  collateral_price, TOKEN_PRECISION);

        // Approximate liquidation price:
        // For a long:  liq_price = (size_usd - collateral_usd + fees_usd) / size_in_tokens × TOKEN_PRECISION
        // For a short: liq_price = (size_usd + collateral_usd - fees_usd) / size_in_tokens × TOKEN_PRECISION
        let collateral_usd = mul_div_wide(&env, position.collateral_amount, collateral_price, TOKEN_PRECISION);
        let total_fees_usd = borrowing_fee_usd + funding_fee_usd + position_fee_usd;

        let liquidation_price = if position.size_in_tokens > 0 {
            let numerator = if position.is_long {
                position.size_in_usd - collateral_usd + total_fees_usd
            } else {
                position.size_in_usd + collateral_usd - total_fees_usd
            };
            if numerator > 0 {
                mul_div_wide(&env, numerator, TOKEN_PRECISION, position.size_in_tokens)
            } else {
                0
            }
        } else {
            0
        };

        PositionInfo {
            position,
            pnl_usd,
            uncapped_pnl_usd,
            borrowing_fee_usd,
            funding_fee_usd,
            position_fee_usd,
            liquidation_price,
        }
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
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market.index_token).mid_price();

        let impact_usd = get_position_price_impact(
            &env, &data_store, &market, is_long, size_delta_usd, is_increase, index_price,
        );

        get_execution_price(&env, index_price, size_delta_usd, impact_usd, is_long, is_increase)
    }

    /// Return whether a position is currently liquidatable at oracle prices.
    pub fn is_position_liquidatable(
        env: Env,
        data_store: Address,
        oracle: Address,
        position: PositionProps,
    ) -> bool {
        let market = Self::get_market(env.clone(), data_store.clone(), position.market.clone());
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price      = oracle_client.get_primary_price(&market.index_token);
        let collateral_price = oracle_client.get_primary_price(&position.collateral_token).mid_price();
        is_liquidatable(&env, &data_store, &position, &market, collateral_price, &index_price)
    }
}
