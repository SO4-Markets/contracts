//! Increase position utilities — open or add to a long/short position.
//! Mirrors GMX's IncreasePositionUtils.sol.
//!
//! Flow:
//!   1. Compute execution price (index price ± position price impact).
//!   2. Collect position fees from collateral.
//!   3. Compute new sizeInTokens = sizeDeltaUsd / executionPrice.
//!   4. Update position fields (size, tokens, collateral, trackers).
//!   5. Apply deltas to open interest, collateral sum, pool amounts.
//!   6. Validate leverage and OI limits.
//!   7. Persist updated position.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PositionFees, PriceProps};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    position_key, open_interest_key, open_interest_in_tokens_key, collateral_sum_key,
};
use gmx_market_utils::{
    apply_delta_to_pool_amount, apply_delta_to_open_interest,
    apply_delta_to_open_interest_in_tokens, update_cumulative_borrowing_factor,
    update_funding_state,
};
use gmx_position_utils::{get_position_fees, validate_position, settle_funding_fees};
use gmx_pricing_utils::{get_position_price_impact, get_execution_price, apply_position_impact_value};

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
}

// ─── Params ───────────────────────────────────────────────────────────────────

pub struct IncreasePositionParams<'a> {
    pub data_store:        &'a Address,
    pub caller:            &'a Address,   // handler contract address (has CONTROLLER)
    pub account:           &'a Address,   // position owner
    pub receiver:          &'a Address,   // where excess collateral goes
    pub market:            &'a MarketProps,
    pub collateral_token:  &'a Address,
    pub size_delta_usd:    i128,
    pub collateral_amount: i128,          // raw token units transferred into pool
    pub acceptable_price:  i128,          // FLOAT_PRECISION; 0 = no check
    pub is_long:           bool,
    pub index_token_price: &'a PriceProps,
    pub collateral_price:  i128,          // FLOAT_PRECISION
    pub current_time:      u64,
}

// ─── Main entry ───────────────────────────────────────────────────────────────

/// Open or increase an existing position. Returns the updated PositionProps.
pub fn increase_position(env: &Env, p: &IncreasePositionParams) -> PositionProps {
    // TODO: (mirrors GMX IncreasePositionUtils.increasePosition)
    //
    // 1. LOAD OR CREATE POSITION:
    //    key = position_key(env, p.account, &p.market.market_token, p.collateral_token, p.is_long)
    //    position = data_store.get_bytes32(key) deserialised, OR default zero-position
    //    (in our impl, store PositionProps directly in handler local storage keyed by BytesN<32>)
    //
    // 2. UPDATE MARKET STATE (funding + borrowing) BEFORE modifying position:
    //    update_funding_state(env, ds, caller, market, long_price, short_price, current_time)
    //    update_cumulative_borrowing_factor(env, ds, caller, market, is_long, current_time)
    //
    // 3. SETTLE PENDING FUNDING for this position (credit claimable, reset trackers):
    //    settle_funding_fees(env, ds, caller, market, &mut position)
    //
    // 4. PRICE IMPACT:
    //    impact_usd = get_position_price_impact(env, ds, market, is_long, size_delta_usd, true)
    //    apply_position_impact_value(env, ds, caller, market, impact_usd, index_price)
    //
    // 5. EXECUTION PRICE:
    //    execution_price = get_execution_price(env, index_price, size_delta_usd, impact_usd,
    //                                          is_long, is_increase=true)
    //    if acceptable_price != 0:
    //        if is_long  && execution_price > acceptable_price → panic "price too high"
    //        if !is_long && execution_price < acceptable_price → panic "price too low"
    //
    // 6. NEW SIZE IN TOKENS:
    //    new_size_in_tokens = size_delta_usd * TOKEN_PRECISION / execution_price
    //    (i.e., how many raw index tokens the new size represents at execution price)
    //
    // 7. POSITION FEES:
    //    for_positive_impact = impact_usd >= 0
    //    fees = get_position_fees(env, ds, market, &position, collateral_price,
    //                             size_delta_usd, for_positive_impact)
    //    total_fee_tokens = fees.total_cost_amount
    //
    // 8. UPDATE COLLATERAL:
    //    position.collateral_amount += collateral_amount - total_fee_tokens
    //    (fees deducted from the deposited collateral)
    //    if position.collateral_amount < 0 → panic "insufficient collateral for fees"
    //
    // 9. UPDATE POSITION SIZE & TOKEN TRACKERS:
    //    position.size_in_usd    += size_delta_usd
    //    position.size_in_tokens += new_size_in_tokens
    //    Update borrowing_factor tracker:
    //      position.borrowing_factor = current cumulative_borrowing_factor from ds
    //    Update funding per-size trackers:
    //      position.funding_fee_amount_per_size = current funding_amount_per_size from ds
    //    position.increased_at_time = current_time
    //
    // 10. OPEN INTEREST DELTAS:
    //     apply_delta_to_open_interest(env, ds, caller, market, collateral_token,
    //                                  is_long, +size_delta_usd)
    //     apply_delta_to_open_interest_in_tokens(env, ds, caller, market, collateral_token,
    //                                             is_long, +new_size_in_tokens)
    //
    // 11. COLLATERAL SUM (tracks total collateral per side for liquidation waterfall):
    //     ds.apply_delta_to_u128(caller,
    //         collateral_sum_key(env, &market.market_token, collateral_token, is_long),
    //         collateral_amount as i128
    //     )
    //
    // 12. POOL AMOUNT UPDATE (net fee income goes to pool):
    //     apply_delta_to_pool_amount(env, ds, caller, market, collateral_token,
    //                                total_fee_tokens as i128)
    //     (fees collected from position → credited to the pool as income)
    //
    // 13. VALIDATE POSITION (leverage, min collateral, max OI):
    //     validate_position(env, ds, &position, market, collateral_price, index_token_price)
    //
    // 14. PERSIST POSITION:
    //     Store serialised PositionProps back to handler local storage at `key`
    //     If new position (was zero): add key to POSITION_LIST and ACCOUNT_POSITION_LIST in ds
    //
    // Returns updated PositionProps
    todo!()
}
