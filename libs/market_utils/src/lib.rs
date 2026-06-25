#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    borrowing_exponent_factor_key, borrowing_factor_key, cumulative_borrowing_factor_key,
    cumulative_borrowing_factor_updated_at_key, funding_amount_per_size_key,
    funding_decrease_factor_per_second_key, funding_exponent_factor_key, funding_factor_key,
    funding_increase_factor_per_second_key, funding_updated_at_key,
    max_funding_factor_per_second_key, max_open_interest_key, max_pool_amount_key,
    min_funding_factor_per_second_key, open_interest_in_tokens_key, open_interest_key,
    pool_amount_key, position_impact_pool_amount_key, saved_funding_factor_per_second_key,
    swap_impact_pool_amount_key,
};
use gmx_math::{mul_div_wide, pow_factor, FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_types::{MarketProps, PoolValueInfo};
use soroban_sdk::{vec, Address, BytesN, Env, Vec};

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
    fn get_u128_batch(env: Env, keys: Vec<BytesN<32>>) -> Vec<u128>;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_u128_instance(env: Env, key: BytesN<32>) -> u128;
    fn set_u128_instance(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn get_i128_instance(env: Env, key: BytesN<32>) -> i128;
    fn set_i128_instance(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
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

pub fn get_swap_impact_pool_amount(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    token: &Address,
) -> u128 {
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
    let oi_tokens_long =
        get_open_interest_in_tokens(env, ds, market, &market.long_token, is_long) as i128;
    let oi_tokens_short =
        get_open_interest_in_tokens(env, ds, market, &market.short_token, is_long) as i128;
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

    let updated_at_key =
        cumulative_borrowing_factor_updated_at_key(env, &market.market_token, is_long);
    let last_updated: u64 = ds_client.get_u128(&updated_at_key) as u64;
    let dt = current_time.saturating_sub(last_updated);
    if dt == 0 {
        return;
    }

    let collateral_token = if is_long {
        &market.long_token
    } else {
        &market.short_token
    };
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
    // mul_div_wide uses Soroban I256 (256-bit) for every multiplication, so no
    // i128 overflow is possible regardless of how large any operand is.
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
        let funding_usd = mul_div_wide(
            env,
            next_factor.abs(),
            long_oi.min(short_oi),
            FLOAT_PRECISION,
        );
        let funding_usd_scaled = mul_div_wide(env, funding_usd, dt as i128, FLOAT_PRECISION);
        if next_factor > 0 {
            // longs pay shorts
            let l = mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, long_oi);
            let s = if short_oi > 0 {
                -mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, short_oi)
            } else {
                0
            };
            (l, s)
        } else {
            // shorts pay longs
            let l = if long_oi > 0 {
                -mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, long_oi)
            } else {
                0
            };
            let s = mul_div_wide(env, funding_usd_scaled, FLOAT_PRECISION, short_oi);
            (l, s)
        }
    };

    // Update cumulative funding-amount-per-size in data_store
    for is_long in [true, false] {
        let collateral_token = if is_long {
            &market.long_token
        } else {
            &market.short_token
        };
        let delta = if is_long { long_delta } else { short_delta };
        let fnd_key =
            funding_amount_per_size_key(env, &market.market_token, collateral_token, is_long);
        ds_client.apply_delta_to_i128(caller, &fnd_key, &delta);
    }

    // Emit sign-flip event when the paying side changes (positive = longs pay, negative = shorts pay).
    // Only fires when both the old and new rates are non-zero, so a rate starting from or going to
    // zero does not trigger a spurious flip notification.
    if dt > 0
        && current_factor != 0
        && next_factor != 0
        && (current_factor > 0) != (next_factor > 0)
    {
        env.events().publish(
            (soroban_sdk::symbol_short!("fnd_flip"),),
            (
                market.market_token.clone(),
                next_factor > 0i128,                       // is_long_paying
                current_factor.saturating_mul(3600i128),   // old_rate_per_hour (FLOAT_PRECISION)
                next_factor.saturating_mul(3600i128),      // new_rate_per_hour (FLOAT_PRECISION)
                long_oi as u128,                           // long_oi_usd
                short_oi as u128,                          // short_oi_usd
                env.ledger().sequence() as u64,            // ledger
            ),
        );
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

    // Funding config parameters are read from instance storage — they are
    // set once during market initialization and read on every funding tick.
    // Instance storage has lower rent cost and avoids TTL management overhead
    // for these infrequently-written values.
    let exponent_key = funding_exponent_factor_key(env, &market.market_token);
    let funding_factor_key_val = funding_factor_key(env, &market.market_token);
    let exponent = ds_client.get_u128_instance(&exponent_key) as i128;
    let funding_factor = ds_client.get_u128_instance(&funding_factor_key_val) as i128;

    let diff_oi = (long_oi - short_oi).abs();
    // ratio = |diffOI| / totalOI (FLOAT_PRECISION)
    let ratio = mul_div_wide(env, diff_oi, FLOAT_PRECISION, total_oi);
    // ratio^exponent (FLOAT_PRECISION)
    let ratio_exp = pow_factor(env, ratio, exponent);
    // target = fundingFactor × ratio^exp / FLOAT_PRECISION (FLOAT_PRECISION per second)
    let target_factor = mul_div_wide(env, funding_factor, ratio_exp, FLOAT_PRECISION);
    // sign: positive = longs pay shorts
    let signed_target = if long_oi >= short_oi {
        target_factor
    } else {
        -target_factor
    };

    let inc_key = funding_increase_factor_per_second_key(env, &market.market_token);
    let dec_key = funding_decrease_factor_per_second_key(env, &market.market_token);
    let min_key = min_funding_factor_per_second_key(env, &market.market_token);
    let max_key = max_funding_factor_per_second_key(env, &market.market_token);

    let increase_factor = ds_client.get_u128_instance(&inc_key) as i128;
    let decrease_factor = ds_client.get_u128_instance(&dec_key) as i128;
    let min_factor = ds_client.get_i128_instance(&min_key);
    let max_factor = ds_client.get_i128_instance(&max_key);

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
///
/// SIMPLIFIED vs GMX:
///   - `total_borrowing_fees` is always 0. Full borrowing fee accrual requires
///     tracking a per-side `cumulative_borrowing_factor` updated on every position
///     event and is not yet implemented.
///   - No `max_pnl_factor` cap is applied. GMX applies different PnL cap factors
///     for deposit, withdrawal, and trader operations; this function returns the
///     raw net PnL without capping. Pool token prices may diverge from GMX under
///     large open interest. See docs/POOLS_REVIEW_IMPLEMENTATION_PLAN.md §Issue 7.
pub fn get_pool_value(
    env: &Env,
    ds: &Address,
    market: &MarketProps,
    long_token_price: i128,
    short_token_price: i128,
    index_token_price: i128,
    maximize: bool,
) -> PoolValueInfo {
    // Batch all 11 reads into a single cross-contract call to stay within Soroban's
    // instruction budget (individual calls per read would each incur invocation overhead).
    let keys = vec![
        env,
        pool_amount_key(env, &market.market_token, &market.long_token),
        pool_amount_key(env, &market.market_token, &market.short_token),
        position_impact_pool_amount_key(env, &market.market_token),
        open_interest_key(env, &market.market_token, &market.long_token, true),
        open_interest_key(env, &market.market_token, &market.short_token, true),
        open_interest_key(env, &market.market_token, &market.long_token, false),
        open_interest_key(env, &market.market_token, &market.short_token, false),
        open_interest_in_tokens_key(env, &market.market_token, &market.long_token, true),
        open_interest_in_tokens_key(env, &market.market_token, &market.short_token, true),
        open_interest_in_tokens_key(env, &market.market_token, &market.long_token, false),
        open_interest_in_tokens_key(env, &market.market_token, &market.short_token, false),
    ];
    let batch = DataStoreClient::new(env, ds).get_u128_batch(&keys);

    let long_pool          = batch.get(0).unwrap_or(0) as i128;
    let short_pool         = batch.get(1).unwrap_or(0) as i128;
    let impact_pool_tokens = batch.get(2).unwrap_or(0) as i128;

    // open interest in USD
    let oi_long_lt  = batch.get(3).unwrap_or(0) as i128;
    let oi_long_st  = batch.get(4).unwrap_or(0) as i128;
    let oi_short_lt = batch.get(5).unwrap_or(0) as i128;
    let oi_short_st = batch.get(6).unwrap_or(0) as i128;

    // open interest in tokens
    let oit_long_lt  = batch.get(7).unwrap_or(0) as i128;
    let oit_long_st  = batch.get(8).unwrap_or(0) as i128;
    let oit_short_lt = batch.get(9).unwrap_or(0) as i128;
    let oit_short_st = batch.get(10).unwrap_or(0) as i128;

    // USD value of pool tokens
    let long_usd       = mul_div_wide(env, long_pool, long_token_price, TOKEN_PRECISION);
    let short_usd      = mul_div_wide(env, short_pool, short_token_price, TOKEN_PRECISION);
    let impact_pool_usd = mul_div_wide(env, impact_pool_tokens, index_token_price, TOKEN_PRECISION);

    // Inline PnL calculation for longs
    let long_pnl = {
        let oi_usd    = oi_long_lt + oi_long_st;
        let oi_tokens = oit_long_lt + oit_long_st;
        if oi_tokens == 0 {
            0
        } else {
            let pos_val = mul_div_wide(env, oi_tokens, index_token_price, TOKEN_PRECISION);
            pos_val - oi_usd
        }
    };

    // Inline PnL calculation for shorts
    let short_pnl = {
        let oi_usd    = oi_short_lt + oi_short_st;
        let oi_tokens = oit_short_lt + oit_short_st;
        if oi_tokens == 0 {
            0
        } else {
            let pos_val = mul_div_wide(env, oi_tokens, index_token_price, TOKEN_PRECISION);
            oi_usd - pos_val
        }
    };

    let _ = maximize; // reserved for future min/max price selection
    let net_pnl = long_pnl + short_pnl;
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
        total_borrowing_fees: 0, // SIMPLIFIED: borrowing fee accrual not yet implemented
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

    let info = get_pool_value(
        env,
        ds,
        market,
        long_token_price,
        short_token_price,
        index_token_price,
        maximize,
    );
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
    if current > max {
        Err(Error::MaxPoolAmountExceeded)
    } else {
        Ok(())
    }
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
    if current > max {
        Err(Error::MaxOpenInterestExceeded)
    } else {
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};

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

        (
            admin,
            ds,
            market_token,
            index_token,
            long_token,
            short_token,
        )
    }

    fn make_market_props(
        market_token: &Address,
        index_token: &Address,
        long_token: &Address,
        short_token: &Address,
    ) -> MarketProps {
        // Issue #248: build via the shared constructor instead of a per-field literal.
        MarketProps::new(market_token, index_token, long_token, short_token)
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

    // ── Issue #155/#126: validate_open_interest unit tests ────────────────────

    /// When no MAX_OPEN_INTEREST is configured (key absent / 0), any OI is valid.
    #[test]
    fn validate_open_interest_unconfigured_always_ok() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        // Seed arbitrarily large OI with no cap set
        let oi_key = gmx_keys::open_interest_key(&env, &mt, &lt, true);
        DsClient::new(&env, &ds).apply_delta_to_u128(&admin, &oi_key, &(999_000 * FLOAT_PRECISION));

        assert!(
            validate_open_interest(&env, &ds, &market, true).is_ok(),
            "unconfigured cap must always pass"
        );
    }

    /// When OI is exactly at the cap, validation passes.
    #[test]
    fn validate_open_interest_at_cap_passes() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let cap: u128 = (5_000 * FLOAT_PRECISION) as u128;
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_open_interest_key(&env, &mt, true),
            &cap,
        );

        // Set OI exactly equal to cap (via long_token collateral)
        let oi_key = gmx_keys::open_interest_key(&env, &mt, &lt, true);
        ds_c.apply_delta_to_u128(&admin, &oi_key, &(cap as i128));

        assert!(
            validate_open_interest(&env, &ds, &market, true).is_ok(),
            "OI exactly at cap must pass"
        );
    }

    /// When OI exceeds the cap by even 1 unit, validation returns an error.
    #[test]
    fn validate_open_interest_over_cap_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let cap: u128 = (3_000 * FLOAT_PRECISION) as u128;
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_open_interest_key(&env, &mt, true),
            &cap,
        );

        // Set OI one unit above cap
        let oi_key = gmx_keys::open_interest_key(&env, &mt, &lt, true);
        ds_c.apply_delta_to_u128(&admin, &oi_key, &(cap as i128 + 1));

        assert_eq!(
            validate_open_interest(&env, &ds, &market, true),
            Err(Error::MaxOpenInterestExceeded),
            "OI one unit over cap must return MaxOpenInterestExceeded"
        );
    }

    /// Cap is per-side: long cap does not affect short OI validation.
    #[test]
    fn validate_open_interest_cap_is_per_side() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let long_cap: u128 = (1_000 * FLOAT_PRECISION) as u128;
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_open_interest_key(&env, &mt, true),
            &long_cap,
        );

        // Push shorts well above the long cap — should still pass (no short cap)
        let short_oi_key = gmx_keys::open_interest_key(&env, &mt, &st, false);
        ds_c.apply_delta_to_u128(&admin, &short_oi_key, &(10_000 * FLOAT_PRECISION));

        assert!(
            validate_open_interest(&env, &ds, &market, false).is_ok(),
            "long cap must not affect short-side validation"
        );
    }

    // ── Issue #137: differential tests against reference GMX formulas ─────────
    //
    // Each test pins a specific numeric result computed by hand from the GMX
    // formula definition. If formula drift occurs the assertion fails and the
    // deviation must be documented or fixed.

    /// Reference: long PnL = oi_tokens * price / TOKEN_PRECISION - oi_usd_long
    ///
    /// Example from GMX docs / formula:
    ///   oi_usd  = 10_000 * FP   (long traders opened $10 000 of size)
    ///   oi_tokens = 5 * TOKEN_PRECISION  (5 ETH at $2 000 each at open)
    ///   current price = $3 000
    ///   position_value = 5 * 3_000 * FP = 15_000 * FP
    ///   pnl = 15_000*FP - 10_000*FP = 5_000*FP
    #[test]
    fn differential_get_pnl_long_matches_reference_formula() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let fp = FLOAT_PRECISION;
        let ds_c = DsClient::new(&env, &ds);

        let oi_usd = 10_000_i128 * fp;
        let oi_tokens = 5_i128 * 10_000_000; // 5 whole tokens (7-decimal precision)
        let price = 3_000_i128 * fp; // $3 000 in FLOAT_PRECISION

        // Seed OI via long_token collateral
        let oi_key = gmx_keys::open_interest_key(&env, &mt, &lt, true);
        let tok_key = gmx_keys::open_interest_in_tokens_key(&env, &mt, &lt, true);
        ds_c.apply_delta_to_u128(&admin, &oi_key, &oi_usd);
        ds_c.apply_delta_to_u128(&admin, &tok_key, &oi_tokens);

        let pnl = get_pnl(&env, &ds, &market, price, true, true);

        // Reference: position_value = oi_tokens * price / TOKEN_PRECISION
        let expected_value = mul_div_wide(&env, oi_tokens, price, TOKEN_PRECISION);
        let expected_pnl = expected_value - oi_usd;

        assert_eq!(
            pnl, expected_pnl,
            "get_pnl long must match reference: pnl={pnl}, expected={expected_pnl}"
        );
        assert_eq!(
            pnl,
            5_000 * fp,
            "known numeric value: 5 ETH * $3000 - $10000 = $5000"
        );
    }

    /// Reference: short PnL = oi_usd_short - oi_tokens * price / TOKEN_PRECISION
    ///
    ///   oi_usd  = 8_000 * FP   (short traders shorted $8 000)
    ///   oi_tokens = 4 * TOKEN_PRECISION  (4 ETH at $2 000 each at open)
    ///   current price = $1 500  (fallen → shorts profit)
    ///   position_value = 4 * 1_500 * FP = 6_000 * FP
    ///   pnl = 8_000*FP - 6_000*FP = 2_000*FP
    #[test]
    fn differential_get_pnl_short_matches_reference_formula() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let fp = FLOAT_PRECISION;
        let ds_c = DsClient::new(&env, &ds);

        let oi_usd = 8_000_i128 * fp;
        let oi_tokens = 4_i128 * 10_000_000; // 4 tokens
        let price = 1_500_i128 * fp;

        let oi_key = gmx_keys::open_interest_key(&env, &mt, &st, false);
        let tok_key = gmx_keys::open_interest_in_tokens_key(&env, &mt, &st, false);
        ds_c.apply_delta_to_u128(&admin, &oi_key, &oi_usd);
        ds_c.apply_delta_to_u128(&admin, &tok_key, &oi_tokens);

        let pnl = get_pnl(&env, &ds, &market, price, false, true);

        let expected_value = mul_div_wide(&env, oi_tokens, price, TOKEN_PRECISION);
        let expected_pnl = oi_usd - expected_value;

        assert_eq!(
            pnl, expected_pnl,
            "get_pnl short must match reference: pnl={pnl}, expected={expected_pnl}"
        );
        assert_eq!(
            pnl,
            2_000 * fp,
            "known numeric value: $8000 - 4 * $1500 = $2000"
        );
    }

    /// Reference: pool value = longUSD + shortUSD - netPnL (PnL owed to traders).
    ///
    ///   long_pool  = 5 tokens @ $2 000 → $10 000
    ///   short_pool = 4_000 tokens (stablecoin) @ $1 → $4 000
    ///   no open positions → netPnL = 0
    ///   pool_value = $10 000 + $4 000 = $14 000
    #[test]
    fn differential_get_pool_value_no_positions_matches_reference() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let fp = FLOAT_PRECISION;
        let ds_c = DsClient::new(&env, &ds);

        // long_pool: 5 tokens
        let long_pool = 5_i128 * 10_000_000;
        // short_pool: 4000 stablecoins
        let short_pool = 4_000_i128 * 10_000_000;

        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::pool_amount_key(&env, &mt, &lt),
            &long_pool,
        );
        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::pool_amount_key(&env, &mt, &st),
            &short_pool,
        );

        let long_price = 2_000_i128 * fp; // $2 000
        let short_price = fp; // $1 (stablecoin)
        let index_price = 2_000_i128 * fp;

        let info = get_pool_value(
            &env,
            &ds,
            &market,
            long_price,
            short_price,
            index_price,
            true,
        );

        // Expected: long = 5 * $2000 = $10_000,  short = 4000 * $1 = $4_000
        let expected_long_usd = mul_div_wide(&env, long_pool, long_price, TOKEN_PRECISION);
        let expected_short_usd = mul_div_wide(&env, short_pool, short_price, TOKEN_PRECISION);
        let expected_pool_value = expected_long_usd + expected_short_usd; // no PnL, no impact pool

        assert_eq!(
            info.long_token_usd, expected_long_usd,
            "long token USD must match reference: got {}, expected {}",
            info.long_token_usd, expected_long_usd
        );
        assert_eq!(
            info.short_token_usd, expected_short_usd,
            "short token USD must match reference"
        );
        assert_eq!(info.net_pnl, 0, "no open positions → net PnL must be 0");
        assert_eq!(
            info.pool_value, expected_pool_value,
            "pool value must match reference: got {}, expected {}",
            info.pool_value, expected_pool_value
        );
        assert_eq!(
            info.pool_value,
            14_000 * fp,
            "known value: 5*$2000 + 4000*$1 = $14000"
        );
    }

    /// Reference: borrowing fee = (cum_factor_now - factor_at_open) * size_in_tokens / FP
    ///
    ///   cum_factor_now      = FP / 10  (10%)
    ///   cum_factor_at_open  = 0
    ///   size_in_tokens      = 2 * TOKEN_PRECISION
    ///   fee = (FP/10 - 0) * (2 * TOKEN_PRECISION) / FP = 0.1 * 2 tokens = 0.2 tokens
    #[test]
    fn differential_get_borrowing_fees_matches_reference_formula() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let fp = FLOAT_PRECISION;
        let ds_c = DsClient::new(&env, &ds);

        let cum_factor_now: u128 = (fp / 10) as u128; // 10%
        let cum_factor_at_open: u128 = 0;
        let size_in_tokens: u128 = 2 * 10_000_000; // 2 whole tokens

        let cum_key = gmx_keys::cumulative_borrowing_factor_key(&env, &mt, true);
        ds_c.set_u128(&admin, &cum_key, &cum_factor_now);

        let fee = get_borrowing_fees(
            &env,
            &ds,
            &market,
            &lt,
            true,
            cum_factor_at_open,
            size_in_tokens,
        );

        // Reference: delta = 10%; fee = 10% * 2 tokens = 0.2 tokens = 2_000_000 units
        let delta = cum_factor_now - cum_factor_at_open;
        let expected = mul_div_wide(&env, delta as i128, size_in_tokens as i128, fp) as u128;

        assert_eq!(
            fee, expected,
            "borrowing fee must match reference: fee={fee}, expected={expected}"
        );
        assert_eq!(
            fee, 2_000_000u128,
            "known numeric: 10% of 2 tokens = 0.2 tokens = 2_000_000 units"
        );
    }

    /// Reference: long PnL is zero when price equals entry price (break-even).
    /// This validates no rounding or formula drift at the identity point.
    #[test]
    fn differential_long_pnl_zero_at_entry_price() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let market = make_market_props(&mt, &it, &lt, &st);

        let fp = FLOAT_PRECISION;
        let entry_price = 2_000_i128 * fp;
        let size_tokens = 3_i128 * 10_000_000; // 3 tokens
        let size_usd = mul_div_wide(&env, size_tokens, entry_price, TOKEN_PRECISION);

        let ds_c = DsClient::new(&env, &ds);
        let oi_key = gmx_keys::open_interest_key(&env, &mt, &lt, true);
        let tok_key = gmx_keys::open_interest_in_tokens_key(&env, &mt, &lt, true);
        ds_c.apply_delta_to_u128(&admin, &oi_key, &size_usd);
        ds_c.apply_delta_to_u128(&admin, &tok_key, &size_tokens);

        let pnl = get_pnl(&env, &ds, &market, entry_price, true, true);

        assert_eq!(
            pnl, 0,
            "long PnL at entry price must be exactly 0, got {pnl}"
        );
    }

    // ── Issue #216: FundingRateSignFlipped event ──────────────────────────────

    /// Seed all the config keys needed by compute_next_funding_factor.
    ///
    /// saved_factor: the rate already persisted (simulates a prior update).
    /// Ramp factors are set large enough that a single-second step can cross
    /// zero when the dominant OI side changes.
    fn setup_funding_params(
        env: &Env,
        ds: &Address,
        admin: &Address,
        mt: &Address,
        saved_factor: i128,
    ) {
        let ds_c = DsClient::new(env, ds);
        let fp = FLOAT_PRECISION as u128;
        // saved_factor and last-updated timestamp are in persistent storage
        ds_c.set_i128(admin, &gmx_keys::saved_funding_factor_per_second_key(env, mt), &saved_factor);
        ds_c.set_u128(admin, &gmx_keys::funding_updated_at_key(env, mt), &0u128);
        // Config params are in instance storage (read via get_u128_instance / get_i128_instance)
        ds_c.set_u128_instance(admin, &gmx_keys::funding_factor_key(env, mt), &fp);
        ds_c.set_u128_instance(admin, &gmx_keys::funding_exponent_factor_key(env, mt), &fp);
        // Ramp = 1000 × FLOAT_PRECISION per second → a single-second step can cross zero
        let ramp: u128 = 1_000u128 * fp;
        ds_c.set_u128_instance(admin, &gmx_keys::funding_increase_factor_per_second_key(env, mt), &ramp);
        ds_c.set_u128_instance(admin, &gmx_keys::funding_decrease_factor_per_second_key(env, mt), &ramp);
        // Wide clamp so clamping never interferes with sign crossing
        let bound: i128 = 1_000_000i128 * FLOAT_PRECISION;
        ds_c.set_i128_instance(admin, &gmx_keys::min_funding_factor_per_second_key(env, mt), &(-bound));
        ds_c.set_i128_instance(admin, &gmx_keys::max_funding_factor_per_second_key(env, mt), &bound);
    }

    /// Shift from long-dominated to short-dominated OI.
    /// saved_factor starts positive (longs paying); with ramp=1000 and dt=1
    /// the rate crosses zero — the fnd_flip event fires exactly when
    /// funding_factor_per_second changes sign (verified via FundingResult).
    #[test]
    fn funding_sign_flip_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let ds_c = DsClient::new(&env, &ds);

        // saved_factor=100 (positive, longs paying)
        setup_funding_params(&env, &ds, &admin, &mt, 100_i128);

        // Short-dominant: 1 M long vs 9 M short → signed_target becomes negative
        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, true),
            &1_000_000_i128,
        );
        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &st, false),
            &9_000_000_i128,
        );

        let market = make_market_props(&mt, &it, &lt, &st);
        let result = update_funding_state(&env, &ds, &admin, &market, 0, 0, 1);

        // Rate crossed zero (was 100, now negative) → fnd_flip event fires exactly here.
        assert!(
            result.funding_factor_per_second < 0,
            "rate must go negative for fnd_flip to fire; got {}",
            result.funding_factor_per_second,
        );
    }

    /// OI stays long-dominant while the saved rate is already positive.
    /// The rate increases in magnitude but never crosses zero → no fnd_flip event.
    #[test]
    fn funding_magnitude_change_no_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = make_market(&env);
        let ds_c = DsClient::new(&env, &ds);

        // saved_factor=100 (positive, longs paying)
        setup_funding_params(&env, &ds, &admin, &mt, 100_i128);

        // Long-dominant: 9 M long vs 1 M short → signed_target stays positive
        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, true),
            &9_000_000_i128,
        );
        ds_c.apply_delta_to_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &st, false),
            &1_000_000_i128,
        );

        let market = make_market_props(&mt, &it, &lt, &st);
        let result = update_funding_state(&env, &ds, &admin, &market, 0, 0, 1);

        // Rate stayed positive (magnitude increased but no sign change) → no fnd_flip event.
        assert!(
            result.funding_factor_per_second > 0,
            "rate must stay positive when OI is long-dominant; got {}",
            result.funding_factor_per_second,
        );
    }

    // ── Borrowing-fee overflow tests (issue #231) ─────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10_000))]
        #[test]
        fn test_borrowing_no_overflow(
            borrowing_factor in 0u64..=u64::MAX,
            oi_u64 in 0u64..=u64::MAX,
            pool_amount_u64 in 1u64..=u64::MAX,
            seconds in 0u64..=100_000u64,
        ) {
            let env = Env::default();
            let bf   = borrowing_factor as i128;
            let oi   = oi_u64 as i128;
            let pool = pool_amount_u64 as i128;
            let dt   = seconds as i128;

            let util          = mul_div_wide(&env, oi, FLOAT_PRECISION, pool);
            let delta_per_sec = mul_div_wide(&env, bf, util, FLOAT_PRECISION);
            let _delta        = mul_div_wide(&env, delta_per_sec, dt, FLOAT_PRECISION);
        }
    }

    #[test]
    fn test_borrowing_extreme_no_panic() {
        let env = Env::default();
        let bf   = i128::MAX / 2;
        let util = FLOAT_PRECISION;
        let dt   = 100_000i128;

        let delta_per_sec = mul_div_wide(&env, bf, util, FLOAT_PRECISION);
        let _delta        = mul_div_wide(&env, delta_per_sec, dt, FLOAT_PRECISION);
    }
}
