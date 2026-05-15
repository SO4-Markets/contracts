//! Pricing utilities — price impact and execution price for swaps and positions.
//! Mirrors GMX's SwapPricingUtils.sol and PositionPricingUtils.sol.
//!
//! Price impact formula (both swap and position):
//!   initialDiff = |sideA_usd - sideB_usd|
//!   nextDiff    = |sideA_usd ± delta - sideB_usd ∓ delta|
//!   if nextDiff < initialDiff → positive impact: factor × (initialDiff^exp - nextDiff^exp)
//!   if nextDiff > initialDiff → negative impact: factor × (nextDiff^exp - initialDiff^exp)
//!   Positive impact is capped by the available impact pool amount.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::MarketProps;
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide, pow_factor};
use gmx_keys::{
    swap_impact_factor_key, swap_impact_exponent_factor_key,
    position_impact_factor_key, position_impact_exponent_factor_key,
    swap_impact_pool_amount_key, position_impact_pool_amount_key,
    swap_fee_factor_key,
};
use gmx_market_utils::{
    get_pool_amount, get_open_interest_for_side,
    get_swap_impact_pool_amount, get_position_impact_pool_amount,
};

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
}

// ─── Internal: core impact formula ───────────────────────────────────────────

/// Compute signed price impact USD given before/after imbalance values and factors.
///
/// next_diff < initial_diff → positive impact (caps at pool)
/// next_diff > initial_diff → negative impact
fn compute_impact_usd(
    env: &Env,
    initial_diff: i128,
    next_diff: i128,
    positive_factor: i128,
    negative_factor: i128,
    exponent: i128,
    impact_pool_usd: i128,
) -> i128 {
    if initial_diff == next_diff {
        return 0;
    }

    if next_diff < initial_diff {
        // Pool balance improves → positive impact for user
        let initial_pow = pow_factor(env, initial_diff, exponent);
        let next_pow    = pow_factor(env, next_diff, exponent);
        let raw = mul_div_wide(env, positive_factor, initial_pow - next_pow, FLOAT_PRECISION);
        // Cap by available impact pool
        raw.min(impact_pool_usd)
    } else {
        // Pool balance worsens → negative impact for user
        let initial_pow = pow_factor(env, initial_diff, exponent);
        let next_pow    = pow_factor(env, next_diff, exponent);
        let raw = mul_div_wide(env, negative_factor, next_pow - initial_pow, FLOAT_PRECISION);
        -raw
    }
}

// ─── Swap price impact ────────────────────────────────────────────────────────

/// Compute price impact USD for a swap of `amount_in` of `token_in` → `token_out`.
///
/// Returns signed impact_usd (positive = good for user; negative = bad for user).
pub fn get_swap_price_impact(
    env: &Env,
    data_store: &Address,
    market: &MarketProps,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
    price_in: i128,
    price_out: i128,
) -> i128 {
    let ds = DataStoreClient::new(env, data_store);

    // Pool amounts in USD (FLOAT_PRECISION)
    let pool_in  = get_pool_amount(env, data_store, market, token_in)  as i128;
    let pool_out = get_pool_amount(env, data_store, market, token_out) as i128;
    let pool_in_usd  = mul_div_wide(env, pool_in,  price_in,  TOKEN_PRECISION);
    let pool_out_usd = mul_div_wide(env, pool_out, price_out, TOKEN_PRECISION);
    let amount_in_usd = mul_div_wide(env, amount_in, price_in, TOKEN_PRECISION);

    let initial_diff = (pool_in_usd - pool_out_usd).abs();
    let next_in_usd  = pool_in_usd  + amount_in_usd;
    let next_out_usd = pool_out_usd - amount_in_usd;
    let next_diff    = (next_in_usd - next_out_usd).abs();

    let pos_factor  = ds.get_u128(&swap_impact_factor_key(env, &market.market_token, true))  as i128;
    let neg_factor  = ds.get_u128(&swap_impact_factor_key(env, &market.market_token, false)) as i128;
    let exponent    = ds.get_u128(&swap_impact_exponent_factor_key(env, &market.market_token)) as i128;

    // Impact pool cap (in USD of token_out)
    let pool_tokens = get_swap_impact_pool_amount(env, data_store, market, token_out) as i128;
    let pool_usd    = mul_div_wide(env, pool_tokens, price_out, TOKEN_PRECISION);

    compute_impact_usd(env, initial_diff, next_diff, pos_factor, neg_factor, exponent, pool_usd)
}

/// Apply the computed swap impact to the impact pool in data_store.
///
/// Positive impact reduces the pool (paid to user); negative adds to it.
/// Returns the impact amount in token units.
pub fn apply_swap_impact_value(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    token: &Address,
    token_price: i128,
    impact_usd: i128,
) -> i128 {
    if impact_usd == 0 || token_price == 0 {
        return 0;
    }
    // Convert USD impact to token amount
    let impact_amount = mul_div_wide(env, impact_usd, TOKEN_PRECISION, token_price);

    // Positive impact → paid from pool (reduce pool); negative → paid into pool (increase pool)
    let delta = -impact_amount;
    DataStoreClient::new(env, data_store)
        .apply_delta_to_u128(caller, &swap_impact_pool_amount_key(env, &market.market_token, token), &delta);

    impact_amount
}

// ─── Swap output amount ───────────────────────────────────────────────────────

/// Compute the net output amount of `token_out` for a swap, after fees and impact.
///
/// Returns (net_output_amount, fee_amount) both in token_out raw units.
pub fn get_swap_output_amount(
    env: &Env,
    data_store: &Address,
    market: &MarketProps,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
    price_in: i128,
    price_out: i128,
    for_positive_impact: bool,
) -> (i128, i128) {
    if price_out == 0 {
        return (0, 0);
    }

    // Raw output before fees (price conversion)
    let amount_out_before_fees = mul_div_wide(env, amount_in, price_in, price_out);

    // Swap fee
    let fee_factor = DataStoreClient::new(env, data_store)
        .get_u128(&swap_fee_factor_key(env, &market.market_token, for_positive_impact)) as i128;
    let fee_amount = mul_div_wide(env, amount_out_before_fees, fee_factor, FLOAT_PRECISION);

    // Price impact (in token_out units)
    let impact_usd = get_swap_price_impact(env, data_store, market, token_in, token_out, amount_in, price_in, price_out);
    let impact_amount = if price_out > 0 {
        mul_div_wide(env, impact_usd, TOKEN_PRECISION, price_out)
    } else {
        0
    };

    let net_output = (amount_out_before_fees - fee_amount + impact_amount).max(0);
    (net_output, fee_amount)
}

// ─── Position price impact ────────────────────────────────────────────────────

/// Compute price impact USD for opening/closing a position of size `size_delta_usd`.
///
/// Uses open interest imbalance as the "virtual balance" (instead of pool amounts).
pub fn get_position_price_impact(
    env: &Env,
    data_store: &Address,
    market: &MarketProps,
    is_long: bool,
    size_delta_usd: i128,
    is_increase: bool,
    index_token_price: i128,
) -> i128 {
    let ds = DataStoreClient::new(env, data_store);

    let long_oi  = get_open_interest_for_side(env, data_store, market, true)  as i128;
    let short_oi = get_open_interest_for_side(env, data_store, market, false) as i128;
    let initial_diff = (long_oi - short_oi).abs();

    let (next_long, next_short) = match (is_long, is_increase) {
        (true,  true)  => (long_oi  + size_delta_usd, short_oi),
        (false, true)  => (long_oi,  short_oi + size_delta_usd),
        (true,  false) => ((long_oi  - size_delta_usd).max(0), short_oi),
        (false, false) => (long_oi,  (short_oi - size_delta_usd).max(0)),
    };
    let next_diff = (next_long - next_short).abs();

    let pos_factor = ds.get_u128(&position_impact_factor_key(env, &market.market_token, true))  as i128;
    let neg_factor = ds.get_u128(&position_impact_factor_key(env, &market.market_token, false)) as i128;
    let exponent   = ds.get_u128(&position_impact_exponent_factor_key(env, &market.market_token)) as i128;

    // Impact pool cap (in USD of index token)
    let pool_tokens = get_position_impact_pool_amount(env, data_store, market) as i128;
    let pool_usd    = if index_token_price > 0 {
        mul_div_wide(env, pool_tokens, index_token_price, TOKEN_PRECISION)
    } else {
        0
    };

    compute_impact_usd(env, initial_diff, next_diff, pos_factor, neg_factor, exponent, pool_usd)
}

/// Apply position price impact to the impact pool.
///
/// Returns impact_amount in index token raw units.
pub fn apply_position_impact_value(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    impact_usd: i128,
    index_token_price: i128,
) -> i128 {
    if impact_usd == 0 || index_token_price == 0 {
        return 0;
    }
    let impact_amount = mul_div_wide(env, impact_usd, TOKEN_PRECISION, index_token_price);
    let delta = -impact_amount; // positive impact → pool shrinks; negative → pool grows
    DataStoreClient::new(env, data_store)
        .apply_delta_to_u128(caller, &position_impact_pool_amount_key(env, &market.market_token), &delta);
    impact_amount
}

// ─── Execution price ──────────────────────────────────────────────────────────

/// Compute the execution price for a position change after applying price impact.
///
/// Returns the adjusted price in FLOAT_PRECISION (USD per whole token).
pub fn get_execution_price(
    env: &Env,
    index_price: i128,
    size_delta_usd: i128,
    price_impact_usd: i128,
    _is_long: bool,
    _is_increase: bool,
) -> i128 {
    if size_delta_usd == 0 || index_price == 0 {
        return index_price;
    }

    // Adjusted size after price impact
    let adjusted_size = size_delta_usd + price_impact_usd;
    if adjusted_size <= 0 {
        return index_price;
    }

    // Tokens you effectively get for adjusted_size at index_price
    // adjusted_tokens (raw 7-decimal units)
    let adjusted_tokens = mul_div_wide(env, adjusted_size, TOKEN_PRECISION, index_price);
    if adjusted_tokens == 0 {
        return index_price;
    }

    // execution_price = size_delta_usd (USD) / adjusted_tokens (raw) × TOKEN_PRECISION
    // = size_delta_usd × TOKEN_PRECISION / adjusted_tokens  → FLOAT_PRECISION per whole token
    mul_div_wide(env, size_delta_usd, TOKEN_PRECISION, adjusted_tokens)
}
