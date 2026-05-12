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

// ─── Swap price impact ────────────────────────────────────────────────────────

/// Compute price impact USD for a swap of `amount_in` of `token_in` → `token_out`.
///
/// Returns signed impact_usd (positive = good for user, paid from pool;
///                             negative = bad for user, paid into pool).
pub fn get_swap_price_impact(
    env: &Env,
    data_store: &Address,
    market: &MarketProps,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
    price_in: i128,   // FLOAT_PRECISION
    price_out: i128,  // FLOAT_PRECISION
) -> i128 {
    // TODO: (mirrors GMX SwapPricingUtils.getPriceImpactUsd)
    //
    // 1. Load current pool amounts (in raw token units):
    //    pool_in  = get_pool_amount(env, ds, market, token_in)   as i128
    //    pool_out = get_pool_amount(env, ds, market, token_out)  as i128
    //
    // 2. Convert to USD (FLOAT_PRECISION):
    //    pool_in_usd  = pool_in  * price_in  / TOKEN_PRECISION
    //    pool_out_usd = pool_out * price_out / TOKEN_PRECISION
    //    amount_in_usd = amount_in * price_in / TOKEN_PRECISION
    //
    // 3. Virtual balance BEFORE and AFTER the swap:
    //    initial_diff = abs(pool_in_usd - pool_out_usd)
    //    next_in_usd  = pool_in_usd  + amount_in_usd
    //    next_out_usd = pool_out_usd - amount_in_usd  (swap drains out side)
    //    next_diff    = abs(next_in_usd - next_out_usd)
    //
    // 4. Load impact factors:
    //    positive_factor  = ds.get_u128(swap_impact_factor_key(market.market_token, true))
    //    negative_factor  = ds.get_u128(swap_impact_factor_key(market.market_token, false))
    //    exponent         = ds.get_u128(swap_impact_exponent_factor_key(market.market_token))
    //
    // 5. Compute impact (use pow_factor for the exponentiation):
    //    if next_diff < initial_diff:  // pool balance improves
    //        impact_usd = positive_factor * (initial_diff^exp - next_diff^exp) / FLOAT_PRECISION
    //        impact_usd = min(impact_usd, impact_pool_usd)  // cap by available pool
    //    else:                         // pool balance worsens
    //        impact_usd = -(negative_factor * (next_diff^exp - initial_diff^exp) / FLOAT_PRECISION)
    //
    // 6. Get impact_pool_usd for cap:
    //    impact_pool_tokens = get_swap_impact_pool_amount(env, ds, market, token_out)
    //    impact_pool_usd = impact_pool_tokens * price_out / TOKEN_PRECISION
    //
    // Returns signed impact_usd
    todo!()
}

/// Apply the computed swap impact to the impact pool in data_store.
///
/// Positive impact reduces the pool (paid out to user); negative adds to it.
pub fn apply_swap_impact_value(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    token: &Address,
    token_price: i128,
    impact_usd: i128,
) -> i128 {
    // TODO:
    // 1. Convert impact_usd → token amount:
    //    impact_amount = impact_usd * TOKEN_PRECISION / token_price
    //    (negative impact → positive pool delta; positive impact → negative pool delta)
    //
    // 2. Apply delta to impact pool:
    //    ds.apply_delta_to_u128(
    //        caller,
    //        swap_impact_pool_amount_key(market.market_token, token),
    //        -impact_amount  // positive impact removes from pool, negative adds to it
    //    )
    //
    // 3. Returns impact_amount (the actual token quantity transferred)
    todo!()
}

// ─── Swap output amount ───────────────────────────────────────────────────────

/// Compute the net output amount of `token_out` for a swap, after fees and impact.
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
    // TODO: (mirrors GMX SwapUtils._swap output calculation)
    //
    // 1. Compute raw output before fees:
    //    amount_out_before_fees = amount_in * price_in / price_out
    //    (simple price conversion using mul_div_wide)
    //
    // 2. Load swap fee factor:
    //    fee_factor = ds.get_u128(swap_fee_factor_key(market.market_token, for_positive_impact))
    //    fee_amount = amount_out_before_fees * fee_factor / FLOAT_PRECISION
    //
    // 3. Compute price impact:
    //    impact_usd = get_swap_price_impact(...)
    //    impact_amount = impact_usd * TOKEN_PRECISION / price_out
    //    (positive impact adds tokens; negative reduces output)
    //
    // 4. net_output = amount_out_before_fees - fee_amount + impact_amount
    //
    // Returns (net_output_amount, fee_amount)
    todo!()
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
) -> i128 {
    // TODO: (mirrors GMX PositionPricingUtils.getPriceImpactUsd)
    //
    // 1. Load current open interest for each side:
    //    long_oi  = get_open_interest_for_side(env, ds, market, true)   as i128
    //    short_oi = get_open_interest_for_side(env, ds, market, false)  as i128
    //    initial_diff = abs(long_oi - short_oi)
    //
    // 2. Compute next OI after the position change:
    //    if is_increase && is_long:  next_long  = long_oi  + size_delta_usd
    //    if is_increase && !is_long: next_short = short_oi + size_delta_usd
    //    if !is_increase && is_long: next_long  = long_oi  - size_delta_usd
    //    etc.
    //    next_diff = abs(next_long - next_short)
    //
    // 3. Load impact factors:
    //    positive_factor = ds.get_u128(position_impact_factor_key(market.market_token, true))
    //    negative_factor = ds.get_u128(position_impact_factor_key(market.market_token, false))
    //    exponent        = ds.get_u128(position_impact_exponent_factor_key(market.market_token))
    //
    // 4. Same formula as swap:
    //    next_diff < initial_diff → positive impact (capped by impact pool)
    //    next_diff > initial_diff → negative impact
    //
    // 5. Impact pool cap (in USD):
    //    impact_pool_tokens = get_position_impact_pool_amount(env, ds, market)
    //    impact_pool_usd    = impact_pool_tokens * index_token_price / TOKEN_PRECISION
    //    (caller must pass index_token_price or compute inside; for simplicity take as param)
    //
    // Note: For decrease orders, the impact direction is reversed:
    //   a decrease that improves OI balance pays the trader from impact pool.
    todo!()
}

/// Apply position price impact to the impact pool.
pub fn apply_position_impact_value(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    impact_usd: i128,
    index_token_price: i128,
) -> i128 {
    // TODO:
    // 1. impact_amount = impact_usd * TOKEN_PRECISION / index_token_price
    //    (negative impact → tokens flow INTO impact pool)
    //    (positive impact → tokens flow OUT of impact pool to trader)
    //
    // 2. ds.apply_delta_to_u128(
    //        caller,
    //        position_impact_pool_amount_key(market.market_token),
    //        -impact_amount
    //    )
    //
    // Returns impact_amount
    todo!()
}

// ─── Execution price ──────────────────────────────────────────────────────────

/// Compute the execution price for a position change after applying price impact.
///
/// Returns the adjusted price in FLOAT_PRECISION.
pub fn get_execution_price(
    env: &Env,
    index_price: i128,
    size_delta_usd: i128,
    price_impact_usd: i128,
    is_long: bool,
    is_increase: bool,
) -> i128 {
    // TODO: (mirrors GMX PositionPricingUtils.getExecutionPrice)
    //
    // Intuition: price impact changes how many tokens you actually get.
    //   Negative impact → you "lose" some of the size → effective price is worse.
    //   Positive impact → you gain extra tokens → effective price is better.
    //
    // 1. Adjusted size in USD = size_delta_usd + price_impact_usd
    //    (price_impact_usd is positive if good for trader, negative if bad)
    //    For an increase: negative impact makes the effective entry price higher.
    //    For a decrease: negative impact makes the effective exit price lower.
    //
    // 2. Tokens received for adjusted_size at index_price:
    //    adjusted_tokens = adjusted_size * TOKEN_PRECISION / index_price  (FLOAT_PRECISION)
    //
    // 3. Execution price = size_delta_usd / adjusted_tokens * TOKEN_PRECISION
    //    (i.e., how many USD per WHOLE token you effectively paid/received)
    //    execution_price = mul_div_wide(env, size_delta_usd, TOKEN_PRECISION, adjusted_tokens)
    //
    // 4. Acceptable price check (done by caller, not here):
    //    - Long increase:  execution_price <= acceptable_price
    //    - Long decrease:  execution_price >= acceptable_price
    //    - Short increase: execution_price >= acceptable_price
    //    - Short decrease: execution_price <= acceptable_price
    //
    // Returns execution_price
    todo!()
}
