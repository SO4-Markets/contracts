//! Decrease position utilities — partial or full close of a long/short position.
//! Mirrors GMX's DecreasePositionUtils.sol.
//!
//! Flow:
//!   1. Update market funding and borrowing state.
//!   2. Settle claimable funding for this position.
//!   3. Compute price impact and execution price.
//!   4. Realise PnL for the closing slice.
//!   5. Deduct fees from remaining collateral.
//!   6. Update position size, tokens, and trackers.
//!   7. Apply OI deltas and pool updates.
//!   8. Validate (if partial) or remove (if fully closed) position.
//!   9. Transfer output tokens to receiver.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PositionFees, PriceProps, DecreasePositionResult};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    position_key, open_interest_key, open_interest_in_tokens_key, collateral_sum_key,
    claimable_funding_amount_key,
};
use gmx_market_utils::{
    apply_delta_to_pool_amount, apply_delta_to_open_interest,
    apply_delta_to_open_interest_in_tokens, update_cumulative_borrowing_factor,
    update_funding_state,
};
use gmx_position_utils::{
    get_position_pnl_usd, get_position_fees, validate_position, settle_funding_fees,
};
use gmx_pricing_utils::{
    get_position_price_impact, get_execution_price, apply_position_impact_value,
};

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
}

// ─── Params ───────────────────────────────────────────────────────────────────

pub struct DecreasePositionParams<'a> {
    pub data_store:        &'a Address,
    pub caller:            &'a Address,   // handler contract address (has CONTROLLER)
    pub account:           &'a Address,   // position owner
    pub receiver:          &'a Address,   // where output tokens are sent
    pub market:            &'a MarketProps,
    pub collateral_token:  &'a Address,
    pub size_delta_usd:    i128,          // USD value of the slice being closed
    pub acceptable_price:  i128,          // FLOAT_PRECISION; 0 = no slippage check
    pub is_long:           bool,
    pub index_token_price: &'a PriceProps,
    pub collateral_price:  i128,          // FLOAT_PRECISION
    pub current_time:      u64,
}

// ─── Main entry ───────────────────────────────────────────────────────────────

/// Decrease or fully close a position. Returns a `DecreasePositionResult`.
pub fn decrease_position(env: &Env, p: &DecreasePositionParams) -> DecreasePositionResult {
    // TODO: (mirrors GMX DecreasePositionUtils.decreasePosition)
    //
    // 1. LOAD POSITION:
    //    key = position_key(env, p.account, &p.market.market_token, p.collateral_token, p.is_long)
    //    position = load from handler storage at `key`
    //    Panic if position doesn't exist: "position not found"
    //    If p.size_delta_usd > position.size_in_usd → clamp to full close
    //
    // 2. UPDATE MARKET STATE (funding + borrowing) BEFORE modifying position:
    //    update_funding_state(env, ds, caller, market, long_price, short_price, current_time)
    //    update_cumulative_borrowing_factor(env, ds, caller, market, is_long, current_time)
    //
    // 3. SETTLE PENDING FUNDING for this position:
    //    settle_funding_fees(env, ds, caller, market, &mut position)
    //
    // 4. PRICE IMPACT:
    //    impact_usd = get_position_price_impact(env, ds, market, is_long,
    //                                           size_delta_usd, is_increase=false)
    //    apply_position_impact_value(env, ds, caller, market, impact_usd, index_price.mid_price())
    //    For decreases: positive impact = good for trader (closing a size that improved OI balance)
    //
    // 5. EXECUTION PRICE:
    //    execution_price = get_execution_price(env, index_price.mid_price(), size_delta_usd,
    //                                          impact_usd, is_long, is_increase=false)
    //    if acceptable_price != 0:
    //        if is_long  && execution_price < acceptable_price → panic "price too low"
    //        if !is_long && execution_price > acceptable_price → panic "price too high"
    //
    // 6. SIZE DELTA IN TOKENS:
    //    Compute size_delta_in_tokens proportional to position.size_in_tokens:
    //    size_delta_in_tokens = size_delta_usd * position.size_in_tokens / position.size_in_usd
    //    (use mul_div_wide to avoid overflow; this is how many raw index tokens are freed)
    //
    // 7. REALISE PnL:
    //    (pnl_usd, _) = get_position_pnl_usd(env, &position, index_token_price, size_delta_usd)
    //    pnl_token_amount = pnl_usd * TOKEN_PRECISION / collateral_price
    //    (can be negative — a loss reduces collateral)
    //    Pool settlement:
    //      if pnl_usd > 0 (trader profit): pool pays out, apply_delta_to_pool_amount(-pnl_tokens)
    //      if pnl_usd < 0 (trader loss):  pool gains,   apply_delta_to_pool_amount(+loss_tokens)
    //
    // 8. POSITION FEES:
    //    for_positive_impact = impact_usd >= 0
    //    fees = get_position_fees(env, ds, market, &position, collateral_price,
    //                             size_delta_usd, for_positive_impact)
    //    apply_delta_to_pool_amount for fee income (fees go to pool)
    //
    // 9. COMPUTE OUTPUT AMOUNT:
    //    remaining_collateral_after_pnl = position.collateral_amount + pnl_token_amount
    //    output_amount = remaining_collateral_after_pnl - fees.total_cost_amount
    //    For full close: output_amount = all collateral (after pnl & fees)
    //    For partial close: output_amount = collateral_delta (portion proportional to size_delta)
    //    if output_amount < 0 → set to 0 (liquidation scenario; no output)
    //
    // 10. UPDATE POSITION SIZE FIELDS:
    //     position.size_in_usd    -= size_delta_usd
    //     position.size_in_tokens -= size_delta_in_tokens
    //     position.collateral_amount -= output_amount + fees.total_cost_amount - pnl_token_amount
    //     (i.e., remove what was output plus fees collected, adjust for pnl already settled in pool)
    //     Update trackers:
    //       position.borrowing_factor = current cumulative_borrowing_factor from ds
    //       position.funding_fee_amount_per_size = current funding_amount_per_size from ds
    //       position.decreased_at_time = current_time
    //
    // 11. OPEN INTEREST DELTAS:
    //     apply_delta_to_open_interest(env, ds, caller, market, collateral_token,
    //                                  is_long, -size_delta_usd)
    //     apply_delta_to_open_interest_in_tokens(env, ds, caller, market, collateral_token,
    //                                             is_long, -size_delta_in_tokens)
    //
    // 12. COLLATERAL SUM:
    //     ds.apply_delta_to_u128(caller,
    //         collateral_sum_key(env, &market.market_token, collateral_token, is_long),
    //         -(output_amount as i128)
    //     )
    //     (reduce collateral sum by the amount leaving the pool)
    //
    // 13. is_fully_closed = position.size_in_usd == 0
    //     if is_fully_closed:
    //         Remove position key from POSITION_LIST and ACCOUNT_POSITION_LIST in ds
    //         Delete position from handler storage
    //     else:
    //         validate_position(env, ds, &position, market, collateral_price, index_token_price)
    //         Persist updated position to handler storage at `key`
    //
    // 14. TRANSFER OUTPUT TO RECEIVER:
    //     MarketTokenClient::new(env, &market.market_token)
    //         .withdraw_from_pool(caller, collateral_token, receiver, output_amount)
    //     (if output_amount > 0)
    //
    // Returns DecreasePositionResult { execution_price, pnl_usd, output_amount,
    //                                  secondary_output_amount: 0,
    //                                  remaining_collateral: position.collateral_amount,
    //                                  is_fully_closed }
    todo!()
}
