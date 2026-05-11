//! Position utilities — per-position PnL, fee calculation, validation, and liquidation check.
//! Mirrors GMX's PositionUtils.sol, PositionStoreUtils.sol, and related helpers.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PositionFees, PriceProps};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    cumulative_borrowing_factor_key,
    funding_amount_per_size_key,
    position_fee_factor_key,
    min_collateral_factor_key,
    max_leverage_key,
    claimable_funding_amount_key,
};

// ─── Data-store client (same minimal interface used across libs) ───────────────

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
}

// ─── PnL ─────────────────────────────────────────────────────────────────────

/// Unrealised PnL in USD (FLOAT_PRECISION) for a full or partial close.
///
/// `size_delta_usd` — the portion of the position being closed (= position.size_in_usd for full).
pub fn get_position_pnl_usd(
    env: &Env,
    position: &PositionProps,
    index_token_price: &PriceProps,
    size_delta_usd: i128,
) -> (i128, i128) {
    // TODO:
    // 1. Pick the price that maximises PnL for the trader:
    //    - Long position: use max price (higher price = more profit)
    //    - Short position: use min price (lower price = more profit)
    //    price = index_token_price.pick_price_for_pnl(position.is_long, maximize=true)
    //
    // 2. Compute current value of all position tokens:
    //    positionValue = position.size_in_tokens * price / TOKEN_PRECISION
    //    (size_in_tokens is in raw 7-decimal units; price is FLOAT_PRECISION per whole token)
    //
    // 3. Unrealised PnL for the full position:
    //    if is_long:  total_pnl = positionValue - position.size_in_usd
    //    if is_short: total_pnl = position.size_in_usd - positionValue
    //
    // 4. Scale to the delta being closed:
    //    pnl_usd = total_pnl * size_delta_usd / position.size_in_usd
    //    (use mul_div_wide to avoid overflow)
    //
    // 5. Also compute uncapped_pnl_usd (before any pnlFactor cap applied by caller)
    //    For now uncapped = pnl_usd (capping is done in get_pool_value / Reader)
    //
    // Returns (pnl_usd, uncapped_pnl_usd)
    todo!()
}

// ─── Fees ─────────────────────────────────────────────────────────────────────

/// Compute all fees owed by a position for a given size delta.
///
/// Returns `PositionFees` with each component in collateral token raw units.
pub fn get_position_fees(
    env: &Env,
    data_store: &Address,
    market: &MarketProps,
    position: &PositionProps,
    collateral_token_price: i128,   // FLOAT_PRECISION
    size_delta_usd: i128,
    for_positive_impact: bool,
) -> PositionFees {
    // TODO: (mirrors GMX PositionPricingUtils.getPositionFees)
    //
    // 1. BORROWING FEE (in collateral tokens):
    //    cumBorrowFactor = data_store.get_u128(cumulative_borrowing_factor_key(market.market_token, is_long))
    //    delta_factor = cumBorrowFactor - position.borrowing_factor   (if negative → 0)
    //    borrowing_fee_tokens = delta_factor * position.size_in_tokens / FLOAT_PRECISION
    //    (size_in_tokens tracks how many raw tokens back the position;
    //     delta_factor is per token so the result is in collateral raw units)
    //
    // 2. FUNDING FEE (in collateral tokens):
    //    latestFundingAmountPerSize = data_store.get_i128(
    //        funding_amount_per_size_key(market.market_token, collateral_token, is_long))
    //    funding_delta = latestFundingAmountPerSize - position.funding_fee_amount_per_size
    //    if funding_delta > 0 (position owes funding):
    //        funding_fee_tokens = funding_delta * position.size_in_usd / FLOAT_PRECISION
    //    else (position is owed funding, tracked as claimable):
    //        funding_fee_tokens = 0  (the credit is stored in claimable_funding_amount_key)
    //
    // 3. POSITION FEE (opening/closing fee, in collateral tokens):
    //    fee_factor = data_store.get_u128(
    //        position_fee_factor_key(market.market_token, for_positive_impact))
    //    position_fee_usd = size_delta_usd * fee_factor / FLOAT_PRECISION
    //    position_fee_tokens = position_fee_usd * TOKEN_PRECISION / collateral_token_price
    //
    // 4. total_cost_amount = borrowing_fee_tokens + funding_fee_tokens + position_fee_tokens
    //
    // 5. Return PositionFees { borrowing_fee_amount, funding_fee_amount,
    //                          position_fee_amount, total_cost_amount }
    todo!()
}

/// Settle accumulated funding: credit the claimable amount and update position's
/// per-size baseline so the next fee calculation starts clean.
pub fn settle_funding_fees(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    position: &mut PositionProps,
) {
    // TODO: (mirrors GMX PositionUtils.updateFundingAndBorrowingState calls + settlement)
    //
    // 1. For each collateral token (long_token and short_token):
    //    claimable_per_size = latestFundingAmountPerSize - position tracking field:
    //      - long collateral tracker: position.long_claim_fnd_per_size
    //      - short collateral tracker: position.short_claim_fnd_per_size
    //
    // 2. If claimable_per_size > 0 (position is OWED funding):
    //    claimable_amount = claimable_per_size * position.size_in_usd / FLOAT_PRECISION
    //    data_store.apply_delta_to_u128(
    //        caller,
    //        claimable_funding_amount_key(market.market_token, collateral_token, position.account),
    //        claimable_amount as i128
    //    )
    //
    // 3. Update position's per-size tracker to current value so it won't double-count
    todo!()
}

// ─── Validation ───────────────────────────────────────────────────────────────

/// Validate that a position still meets leverage and collateral requirements.
/// Panics if any constraint is violated.
pub fn validate_position(
    env: &Env,
    data_store: &Address,
    position: &PositionProps,
    market: &MarketProps,
    collateral_token_price: i128,
    index_token_price: &PriceProps,
) {
    // TODO: (mirrors GMX PositionUtils.validatePosition)
    //
    // 1. MIN COLLATERAL check:
    //    min_collateral_factor = data_store.get_u128(min_collateral_factor_key(market.market_token))
    //    collateral_usd = position.collateral_amount * collateral_token_price / TOKEN_PRECISION
    //    required_min = position.size_in_usd * min_collateral_factor / FLOAT_PRECISION
    //    if collateral_usd < required_min → panic "min collateral violated"
    //
    // 2. MAX LEVERAGE check:
    //    max_leverage = data_store.get_u128(max_leverage_key(market.market_token))
    //    effective_leverage = position.size_in_usd * FLOAT_PRECISION / collateral_usd
    //    if effective_leverage > max_leverage → panic "max leverage exceeded"
    //
    // 3. OPEN INTEREST check:
    //    Call market_utils::validate_open_interest(env, data_store, market, position.is_long)
    todo!()
}

/// Returns true if the position can be liquidated at current prices.
///
/// A position is liquidatable when remaining collateral after all fees and
/// unrealised loss falls below the minimum collateral factor threshold.
pub fn is_liquidatable(
    env: &Env,
    data_store: &Address,
    position: &PositionProps,
    market: &MarketProps,
    collateral_token_price: i128,
    index_token_price: &PriceProps,
) -> bool {
    // TODO: (mirrors GMX LiquidationUtils.isPositionLiquidatable)
    //
    // 1. Compute all current fees using get_position_fees(for_positive_impact=false)
    //
    // 2. Compute unrealised PnL for FULL position using get_position_pnl_usd
    //    Use price that MINIMISES PnL (worst case for trader):
    //    price = index_token_price.pick_price_for_pnl(is_long, maximize=false)
    //
    // 3. Remaining collateral in USD:
    //    collateral_usd = position.collateral_amount * collateral_token_price / TOKEN_PRECISION
    //    remaining = collateral_usd - fees_usd + pnl_usd
    //    (pnl_usd is negative for a loss, so this reduces collateral)
    //
    // 4. Min required collateral:
    //    min_collateral_factor = data_store.get_u128(min_collateral_factor_key(market))
    //    min_required = position.size_in_usd * min_collateral_factor / FLOAT_PRECISION
    //
    // 5. Return remaining < min_required  (OR remaining < absolute min collateral constant)
    todo!()
}

// ─── Position key ─────────────────────────────────────────────────────────────

/// Compute the data_store key for a position.
/// key = position_key(account, market_token, collateral_token, is_long)
pub fn get_position_key(
    env: &Env,
    account: &Address,
    market_token: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    // TODO:
    // Call gmx_keys::position_key(env, account, market_token, collateral_token, is_long)
    todo!()
}
