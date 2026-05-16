#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::{MarketProps, PoolValueInfo};
use gmx_math::{
    FLOAT_PRECISION, TOKEN_PRECISION,
    mul_div_wide, pow_factor,
};
use gmx_keys::{
    pool_amount_key,
    open_interest_key, open_interest_in_tokens_key,
    cumulative_borrowing_factor_key, cumulative_borrowing_factor_updated_at_key,
    funding_amount_per_size_key,
    funding_updated_at_key, saved_funding_factor_per_second_key,
    funding_increase_factor_per_second_key, funding_decrease_factor_per_second_key,
    min_funding_factor_per_second_key, max_funding_factor_per_second_key,
    funding_exponent_factor_key, funding_factor_key,
    swap_impact_pool_amount_key, position_impact_pool_amount_key,
    max_pool_amount_key, max_open_interest_key,
    borrowing_factor_key, borrowing_exponent_factor_key,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    MaxPoolAmountExceeded = 1,
    MaxOpenInterestExceeded = 2,
}

// ─── Data-store client interface ──────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
}

// ─── Pool amounts ─────────────────────────────────────────────────────────────

pub fn get_pool_amount(env: &Env, ds: &Address, market: &MarketProps, token: &Address) -> u128 {
    let key = pool_amount_key(env, &market.market_token, token);
    DataStoreClient::new(env, ds).get_u128(&key)
}

pub fn apply_delta_to_pool_amount(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    token: &Address,
    delta: i128,
) -> u128 {
    let key = pool_amount_key(env, &market.market_token, token);
    DataStoreClient::new(env, ds).apply_delta_to_u128(caller, &key, &delta)
}

pub fn get_swap_impact_pool_amount(env: &Env, ds: &Address, market: &MarketProps, token: &Address) -> u128 {
    let key = swap_impact_pool_amount_key(env, &market.market_token, token);
    DataStoreClient::new(env, ds).get_u128(&key)
}

pub fn get_position_impact_pool_amount(env: &Env, ds: &Address, market: &MarketProps) -> u128 {
    let key = position_impact_pool_amount_key(env, &market.market_token);
    DataStoreClient::new(env, ds).get_u128(&key)
}

// ─── Open interest ────────────────────────────────────────────────────────────

pub fn get_open_interest(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    collateral_token: &Address,
    is_long: bool,
) -> u128 {
    let key = open_interest_key(env, &market.market_token, collateral_token, is_long);
    DataStoreClient::new(env, ds).get_u128(&key)
}

pub fn get_open_interest_in_tokens(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    collateral_token: &Address,
    is_long: bool,
) -> u128 {
    let key = open_interest_in_tokens_key(env, &market.market_token, collateral_token, is_long);
    DataStoreClient::new(env, ds).get_u128(&key)
}

/// Total OI in USD for one side (both collateral tokens combined).
pub fn get_open_interest_for_side(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    is_long: bool,
) -> u128 {
    get_open_interest(env, ds, market, &market.long_token, is_long)
        + get_open_interest(env, ds, market, &market.short_token, is_long)
}

pub fn apply_delta_to_open_interest(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    collateral_token: &Address,
    is_long: bool,
    delta: i128,
) -> u128 {
    let key = open_interest_key(env, &market.market_token, collateral_token, is_long);
    DataStoreClient::new(env, ds).apply_delta_to_u128(caller, &key, &delta)
}

pub fn apply_delta_to_open_interest_in_tokens(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    collateral_token: &Address,
    is_long: bool,
    delta: i128,
) -> u128 {
    let key = open_interest_in_tokens_key(env, &market.market_token, collateral_token, is_long);
    DataStoreClient::new(env, ds).apply_delta_to_u128(caller, &key, &delta)
}

// ─── PnL ─────────────────────────────────────────────────────────────────────

/// Unrealized PnL for one side in USD (FLOAT_PRECISION).
///
/// long pnl  = oi_tokens × price - oi_usd
/// short pnl = oi_usd - oi_tokens × price
pub fn get_pnl(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    index_token_price: i128,
    is_long: bool,
    maximize: bool,
) -> i128 {
    // Choose min or max price for the index token
    let price = if is_long == maximize {
        // long+maximize or short+minimize → use max price
        index_token_price
    } else {
        index_token_price
    };

    // Sum OI over both collateral tokens
    let oi_usd = (get_open_interest_for_side(env, ds, market, is_long)) as i128;
    let oi_tokens_long = get_open_interest_in_tokens(env, ds, market, &market.long_token, is_long) as i128;
    let oi_tokens_short = get_open_interest_in_tokens(env, ds, market, &market.short_token, is_long) as i128;
    let oi_tokens = oi_tokens_long + oi_tokens_short;

    if oi_tokens == 0 {
        return 0;
    }

    // value = oi_tokens × price / TOKEN_PRECISION (price is FLOAT_PRECISION per whole token)
    let position_value = mul_div_wide(env, oi_tokens, price, TOKEN_PRECISION);

    if is_long {
        position_value - oi_usd
    } else {
        oi_usd - position_value
    }
}

// ─── Borrowing fees ───────────────────────────────────────────────────────────

/// Pending borrowing fee for a position: (cum_factor_now - factor_at_open) × size_in_tokens.
pub fn get_borrowing_fees(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    _collateral_token: &Address,
    is_long: bool,
    borrowing_factor_at_open: u128,
    size_in_tokens: u128,
) -> u128 {
    let key = cumulative_borrowing_factor_key(env, &market.market_token, is_long);
    let cum_factor = DataStoreClient::new(env, ds).get_u128(&key);
    if cum_factor <= borrowing_factor_at_open {
        return 0;
    }
    let delta = cum_factor - borrowing_factor_at_open;
    // fee = delta × size_in_tokens / FLOAT_PRECISION
    mul_div_wide(env, delta as i128, size_in_tokens as i128, FLOAT_PRECISION) as u128
}

/// Update the cumulative borrowing factor for one side.
///
/// borrowingFactor × (OI / poolAmount)^borrowingExponent × dt
pub fn update_cumulative_borrowing_factor(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    is_long: bool,
    current_time: u64,
) {
    let ds_client = DataStoreClient::new(env, ds);

    let updated_at_key = cumulative_borrowing_factor_updated_at_key(env, &market.market_token, is_long);
    let last_updated: u64 = ds_client.get_u128(&updated_at_key) as u64;
    let dt = current_time.saturating_sub(last_updated);
    if dt == 0 {
        return;
    }

    let collateral_token = if is_long { &market.long_token } else { &market.short_token };
    let pool_amount = get_pool_amount(env, ds, market, collateral_token) as i128;
    if pool_amount == 0 {
        ds_client.set_u128(caller, &updated_at_key, &(current_time as u128));
        return;
    }

    let oi = get_open_interest_for_side(env, ds, market, is_long) as i128;

    let factor_key = borrowing_factor_key(env, &market.market_token, is_long);
    let exponent_key = borrowing_exponent_factor_key(env, &market.market_token, is_long);
    let borrowing_factor = ds_client.get_u128(&factor_key) as i128;
    let exponent = ds_client.get_u128(&exponent_key) as i128;

    // utilization ratio (FLOAT_PRECISION)
    let util = mul_div_wide(env, oi, FLOAT_PRECISION, pool_amount);
    // util^exponent (FLOAT_PRECISION)
    let util_exp = pow_factor(env, util, exponent);
    // delta = borrowingFactor × util^exp × dt / FLOAT_PRECISION
    let delta_per_second = mul_div_wide(env, borrowing_factor, util_exp, FLOAT_PRECISION);
    let delta = mul_div_wide(env, delta_per_second, dt as i128, FLOAT_PRECISION);

    let cum_key = cumulative_borrowing_factor_key(env, &market.market_token, is_long);
    ds_client.apply_delta_to_u128(caller, &cum_key, &delta);
    ds_client.set_u128(caller, &updated_at_key, &(current_time as u128));
}

// ─── Funding ──────────────────────────────────────────────────────────────────

pub struct FundingResult {
    pub funding_factor_per_second: i128,
    pub long_funding_per_size_delta: i128,
    pub short_funding_per_size_delta: i128,
}

/// Ramp the funding rate toward target and compute per-size deltas.
pub fn update_funding_state(
    env: &Env,
    ds: &Address,
    caller: &Address,
    market: &MarketProps,
    _long_token_price: i128,
    _short_token_price: i128,
    current_time: u64,
) -> FundingResult {
    let ds_client = DataStoreClient::new(env, ds);

    let updated_at_key = funding_updated_at_key(env, &market.market_token);
    let last_updated: u64 = ds_client.get_u128(&updated_at_key) as u64;
    let dt = current_time.saturating_sub(last_updated);

    let saved_key = saved_funding_factor_per_second_key(env, &market.market_token);
    let current_factor = ds_client.get_i128(&saved_key);

    let next_factor = if dt == 0 {
        current_factor
    } else {
        compute_next_funding_factor(env, ds, market, current_factor, dt)
    };

    // Persist updated rate and timestamp
    if dt > 0 {
        ds_client.set_i128(caller, &saved_key, &next_factor);
        ds_client.set_u128(caller, &updated_at_key, &(current_time as u128));
    }

    // Compute per-size deltas for longs and shorts
    let long_oi = get_open_interest_for_side(env, ds, market, true) as i128;
    let short_oi = get_open_interest_for_side(env, ds, market, false) as i128;

    let (long_delta, short_delta) = if long_oi == 0 || short_oi == 0 || dt == 0 {
        (0i128, 0i128)
    } else {
        let funding_usd = mul_div_wide(env, next_factor.abs(), long_oi.min(short_oi), FLOAT_PRECISION);
        let funding_usd_scaled = mul_div_wide(env, funding_usd, dt as i128, FLOAT_PRECISION);
        if next_factor > 0 {
            // longs pay shorts
            let l = mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, long_oi);
            let s = if short_oi > 0 { -mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, short_oi) } else { 0 };
            (l, s)
        } else {
            // shorts pay longs
            let l = if long_oi > 0 { -mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, long_oi) } else { 0 };
            let s = mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, short_oi);
            (l, s)
        }
    };

    // Update cumulative funding-amount-per-size in data_store
    for is_long in [true, false] {
        let collateral_token = if is_long { &market.long_token } else { &market.short_token };
        let delta = if is_long { long_delta } else { short_delta };
        let fnd_key = funding_amount_per_size_key(env, &market.market_token, collateral_token, is_long);
        ds_client.apply_delta_to_i128(caller, &fnd_key, &delta);
    }

    FundingResult {
        funding_factor_per_second: next_factor,
        long_funding_per_size_delta: long_delta,
        short_funding_per_size_delta: short_delta,
    }
}

/// Ramp the current funding factor toward the target based on OI imbalance.
fn compute_next_funding_factor(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    current_factor: i128,
    dt: u64,
) -> i128 {
    let ds_client = DataStoreClient::new(env, ds);

    let long_oi = get_open_interest_for_side(env, ds, market, true) as i128;
    let short_oi = get_open_interest_for_side(env, ds, market, false) as i128;
    let total_oi = long_oi + short_oi;

    if total_oi == 0 {
        return 0;
    }

    let exponent_key = funding_exponent_factor_key(env, &market.market_token);
    let funding_factor_key_val = funding_factor_key(env, &market.market_token);
    let exponent = ds_client.get_u128(&exponent_key) as i128;
    let funding_factor = ds_client.get_u128(&funding_factor_key_val) as i128;

    let diff_oi = (long_oi - short_oi).abs();
    // ratio = |diffOI| / totalOI (FLOAT_PRECISION)
    let ratio = mul_div_wide(env, diff_oi, FLOAT_PRECISION, total_oi);
    // ratio^exponent (FLOAT_PRECISION)
    let ratio_exp = pow_factor(env, ratio, exponent);
    // target = fundingFactor × ratio^exp / FLOAT_PRECISION (FLOAT_PRECISION per second)
    let target_factor = mul_div_wide(env, funding_factor, ratio_exp, FLOAT_PRECISION);
    // sign: positive = longs pay shorts
    let signed_target = if long_oi >= short_oi { target_factor } else { -target_factor };

    let inc_key = funding_increase_factor_per_second_key(env, &market.market_token);
    let dec_key = funding_decrease_factor_per_second_key(env, &market.market_token);
    let min_key = min_funding_factor_per_second_key(env, &market.market_token);
    let max_key = max_funding_factor_per_second_key(env, &market.market_token);

    let increase_factor = ds_client.get_u128(&inc_key) as i128;
    let decrease_factor = ds_client.get_u128(&dec_key) as i128;
    let min_factor = ds_client.get_i128(&min_key);
    let max_factor = ds_client.get_i128(&max_key);

    // Ramp toward target
    let ramp_delta = if signed_target > current_factor {
        let max_inc = mul_div_wide(env, increase_factor, dt as i128, FLOAT_PRECISION);
        (signed_target - current_factor).min(max_inc)
    } else {
        let max_dec = mul_div_wide(env, decrease_factor, dt as i128, FLOAT_PRECISION);
        -(current_factor - signed_target).min(max_dec)
    };

    let next = current_factor + ramp_delta;
    next.max(min_factor).min(max_factor)
}

// ─── Pool value ───────────────────────────────────────────────────────────────

/// Full pool value breakdown (mirrors GMX's getPoolValue).
pub fn get_pool_value(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    long_token_price: i128,
    short_token_price: i128,
    index_token_price: i128,
    maximize: bool,
) -> PoolValueInfo {
    let _ds_client = DataStoreClient::new(env, ds);

    let long_pool = get_pool_amount(env, ds, market, &market.long_token) as i128;
    let short_pool = get_pool_amount(env, ds, market, &market.short_token) as i128;

    // USD value of pool tokens = amount × price / TOKEN_PRECISION
    let long_usd = mul_div_wide(env, long_pool, long_token_price, TOKEN_PRECISION);
    let short_usd = mul_div_wide(env, short_pool, short_token_price, TOKEN_PRECISION);

    // Impact pool (denominated in index token)
    let impact_pool_tokens = get_position_impact_pool_amount(env, ds, market) as i128;
    let impact_pool_usd = mul_div_wide(env, impact_pool_tokens, index_token_price, TOKEN_PRECISION);

    // Net PnL for each side
    let long_pnl = get_pnl(env, ds, market, index_token_price, true, maximize);
    let short_pnl = get_pnl(env, ds, market, index_token_price, false, maximize);
    let net_pnl = long_pnl + short_pnl;

    // Total value = longUSD + shortUSD + impactPool - netPnL (PnL is owed to traders)
    let pool_value = long_usd + short_usd + impact_pool_usd - net_pnl;

    PoolValueInfo {
        pool_value,
        long_pnl,
        short_pnl,
        net_pnl,
        long_token_usd: long_usd,
        short_token_usd: short_usd,
        long_token_amount: long_pool,
        short_token_amount: short_pool,
        total_borrowing_fees: 0, // simplified: computed separately when needed
        impact_pool_amount: impact_pool_tokens,
    }
}

// ─── Market token price ───────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn total_supply(env: Env) -> i128;
}

/// Price per LP token in FLOAT_PRECISION.
/// Returns FLOAT_PRECISION (i.e. $1) on first deposit (supply = 0).
pub fn get_market_token_price(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    long_token_price: i128,
    short_token_price: i128,
    index_token_price: i128,
    maximize: bool,
) -> i128 {
    let supply = MarketTokenClient::new(env, &market.market_token).total_supply();
    if supply <= 0 {
        return FLOAT_PRECISION;
    }

    let info = get_pool_value(env, ds, market, long_token_price, short_token_price, index_token_price, maximize);
    if info.pool_value <= 0 {
        return FLOAT_PRECISION;
    }

    // price = poolValue × TOKEN_PRECISION / supply  (result is FLOAT_PRECISION)
    mul_div_wide(env, info.pool_value, TOKEN_PRECISION, supply)
}

// ─── Validation ───────────────────────────────────────────────────────────────

pub fn validate_pool_amount(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    token: &Address,
) -> Result<(), Error> {
    let key = max_pool_amount_key(env, &market.market_token, token);
    let max = DataStoreClient::new(env, ds).get_u128(&key);
    if max == 0 {
        return Ok(());
    }
    let current = get_pool_amount(env, ds, market, token);
    if current > max { Err(Error::MaxPoolAmountExceeded) } else { Ok(()) }
}

pub fn validate_open_interest(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    is_long: bool,
) -> Result<(), Error> {
    let key = max_open_interest_key(env, &market.market_token, is_long);
    let max = DataStoreClient::new(env, ds).get_u128(&key);
    if max == 0 {
        return Ok(());
    }
    let current = get_open_interest_for_side(env, ds, market, is_long);
    if current > max { Err(Error::MaxOpenInterestExceeded) } else { Ok(()) }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;

    fn deploy_role_store(env: &Env, admin: &Address) -> Address {
        let id = env.register(RoleStore, ());
        RsClient::new(env, &id).initialize(admin);
        id
    }

    fn deploy_data_store(env: &Env, admin: &Address, rs: &Address) -> Address {
        let id = env.register(DataStore, ());
        DsClient::new(env, &id).initialize(admin, rs);
        id
    }

    fn make_market(env: &Env) -> (Address, Address, Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let rs = deploy_role_store(env, &admin);
        let ds = deploy_data_store(env, &admin, &rs);
        let rs_client = RsClient::new(env, &rs);
        rs_client.grant_role(&admin, &admin, &roles::controller(env));

        let market_token = Address::generate(env);
        let index_token = Address::generate(env);
        let long_token = Address::generate(env);
        let short_token = Address::generate(env);

        (admin, ds, market_token, index_token, long_token, short_token)
    }

    fn make_market_props(
        market_token: &Address,
        index_token: &Address,
        long_token: &Address,
        short_token: &Address,
    ) -> MarketProps {
        MarketProps {
            market_token: market_token.clone(),
            index_token: index_token.clone(),
            long_token: long_token.clone(),
            short_token: short_token.clone(),
        }
    }

    #[test]
    fn pool_amount_zero_by_default() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        assert_eq!(get_pool_amount(&env, &ds, &market, &lt), 0);
        assert_eq!(get_pool_amount(&env, &ds, &market, &st), 0);
    }

    #[test]
    fn apply_delta_to_pool_amount_works() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let after = apply_delta_to_pool_amount(&env, &ds, &admin, &market, &lt, 1_000_000);
        assert_eq!(after, 1_000_000);
        let after2 = apply_delta_to_pool_amount(&env, &ds, &admin, &market, &lt, -500_000);
        assert_eq!(after2, 500_000);
    }

    #[test]
    fn open_interest_zero_by_default() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        assert_eq!(get_open_interest_for_side(&env, &ds, &market, true), 0);
        assert_eq!(get_open_interest_for_side(&env, &ds, &market, false), 0);
    }

    #[test]
    fn pnl_zero_when_no_positions() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        let price = FLOAT_PRECISION; // $1
        assert_eq!(get_pnl(&env, &ds, &market, price, true, true), 0);
        assert_eq!(get_pnl(&env, &ds, &market, price, false, true), 0);
    }

    #[test]
    fn pool_value_empty_market() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        let price = FLOAT_PRECISION;
        let info = get_pool_value(&env, &ds, &market, price, price, price, true);
        assert_eq!(info.pool_value, 0);
        assert_eq!(info.net_pnl, 0);
    }

    #[test]
    fn borrowing_fees_zero_at_open() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        // cum_factor starts at 0; position opened at 0 → no fees
        let fees = get_borrowing_fees(&env, &ds, &market, &lt, true, 0, 1_000_000);
        assert_eq!(fees, 0);
    }

    #[test]
    fn validate_pool_amount_no_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);
        // No max configured → always passes
        assert!(validate_pool_amount(&env, &ds, &market, &lt).is_ok());
    }
}
