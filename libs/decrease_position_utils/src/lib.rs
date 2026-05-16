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
#![allow(deprecated)]

use soroban_sdk::{contracttype, Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PriceProps, DecreasePositionResult};
use gmx_math::{TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    position_key, position_list_key, account_position_list_key,
    cumulative_borrowing_factor_key, funding_amount_per_size_key,
    collateral_sum_key,
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

#[allow(dead_code)]
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

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn withdraw_from_pool(env: Env, caller: Address, pool_token: Address, receiver: Address, amount: i128);
}

// ─── Position storage key ──────────────────────────────────────────────────────

#[contracttype]
enum PositionKey {
    Position(BytesN<32>),
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
    let pos_key = position_key(env, p.account, &p.market.market_token, p.collateral_token, p.is_long);
    let storage_key = PositionKey::Position(pos_key.clone());

    // 1. Load position
    let mut position: PositionProps = env.storage().persistent()
        .get(&storage_key)
        .unwrap_or_else(|| panic!("position not found"));

    // Clamp size_delta to full close if needed
    let size_delta_usd = p.size_delta_usd.min(position.size_in_usd);

    // 2. Update market funding + borrowing state
    let index_price_mid = p.index_token_price.mid_price();
    update_funding_state(env, p.data_store, p.caller, p.market, index_price_mid, index_price_mid, p.current_time);
    update_cumulative_borrowing_factor(env, p.data_store, p.caller, p.market, p.is_long, p.current_time);

    // 3. Settle pending funding for this position
    settle_funding_fees(env, p.data_store, p.caller, p.market, &mut position);

    // 4. Price impact (decrease: is_increase = false)
    let impact_usd = get_position_price_impact(
        env, p.data_store, p.market,
        p.is_long, size_delta_usd, false,
        index_price_mid,
    );
    apply_position_impact_value(env, p.data_store, p.caller, p.market, impact_usd, index_price_mid);

    // 5. Execution price
    let execution_price = get_execution_price(env, index_price_mid, size_delta_usd, impact_usd, p.is_long, false);
    if p.acceptable_price != 0 {
        if p.is_long  && execution_price < p.acceptable_price {
            panic!("execution price too low for long decrease");
        }
        if !p.is_long && execution_price > p.acceptable_price {
            panic!("execution price too high for short decrease");
        }
    }

    // 6. Size delta in tokens (proportional to position)
    let size_delta_in_tokens = if position.size_in_usd > 0 {
        mul_div_wide(env, size_delta_usd, position.size_in_tokens, position.size_in_usd)
    } else {
        0
    };

    // 7. Realise PnL for the closing slice
    let (pnl_usd, _) = get_position_pnl_usd(env, &position, p.index_token_price, size_delta_usd);
    let pnl_token_amount = if p.collateral_price > 0 {
        mul_div_wide(env, pnl_usd, TOKEN_PRECISION, p.collateral_price)
    } else {
        0
    };

    // Settle PnL with the pool:
    //   trader profit → pool shrinks (pool pays trader)
    //   trader loss   → pool grows  (trader pays pool)
    if pnl_token_amount > 0 {
        apply_delta_to_pool_amount(env, p.data_store, p.caller, p.market, p.collateral_token, -pnl_token_amount);
    } else if pnl_token_amount < 0 {
        apply_delta_to_pool_amount(env, p.data_store, p.caller, p.market, p.collateral_token, -pnl_token_amount); // negative delta = pool grows
    }

    // 8. Position fees
    let for_positive_impact = impact_usd >= 0;
    let fees = get_position_fees(
        env, p.data_store, p.market, &position,
        p.collateral_price, size_delta_usd, for_positive_impact,
    );
    // Fee income goes to pool
    apply_delta_to_pool_amount(env, p.data_store, p.caller, p.market, p.collateral_token, fees.total_cost_amount);

    // 9. Compute output amount
    // For a partial close, we return the collateral proportional to the size delta
    let collateral_delta = if position.size_in_usd > 0 {
        mul_div_wide(env, position.collateral_amount, size_delta_usd, position.size_in_usd)
    } else {
        position.collateral_amount
    };

    let raw_output = collateral_delta + pnl_token_amount - fees.total_cost_amount;
    let output_amount = raw_output.max(0);

    // 10. Update position size fields
    position.size_in_usd    -= size_delta_usd;
    position.size_in_tokens -= size_delta_in_tokens;
    position.collateral_amount -= collateral_delta;
    position.decreased_at_time = p.current_time;

    // Sync trackers
    let cum_borrow_key = cumulative_borrowing_factor_key(env, &p.market.market_token, p.is_long);
    position.borrowing_factor = DataStoreClient::new(env, p.data_store).get_u128(&cum_borrow_key) as i128;

    let fnd_key = funding_amount_per_size_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    position.funding_fee_amount_per_size = DataStoreClient::new(env, p.data_store).get_i128(&fnd_key);

    // 11. Open interest deltas
    apply_delta_to_open_interest(env, p.data_store, p.caller, p.market, p.collateral_token, p.is_long, -size_delta_usd);
    apply_delta_to_open_interest_in_tokens(env, p.data_store, p.caller, p.market, p.collateral_token, p.is_long, -size_delta_in_tokens);

    // 12. Collateral sum
    let col_sum_key = collateral_sum_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    DataStoreClient::new(env, p.data_store)
        .apply_delta_to_u128(p.caller, &col_sum_key, &(-output_amount));

    // 13. Persist or remove position
    let is_fully_closed = position.size_in_usd == 0;
    let remaining_collateral = position.collateral_amount;

    if is_fully_closed {
        env.storage().persistent().remove(&storage_key);
        let ds = DataStoreClient::new(env, p.data_store);
        ds.remove_bytes32_from_set(p.caller, &position_list_key(env), &pos_key);
        ds.remove_bytes32_from_set(p.caller, &account_position_list_key(env, p.account), &pos_key);
    } else {
        validate_position(env, p.data_store, &position, p.market, p.collateral_price, p.index_token_price);
        env.storage().persistent().set(&storage_key, &position);
    }

    // 14. Transfer output to receiver
    if output_amount > 0 {
        MarketTokenClient::new(env, &p.market.market_token)
            .withdraw_from_pool(p.caller, p.collateral_token, p.receiver, &output_amount);
    }

    env.events().publish(
        (soroban_sdk::symbol_short!("pos_dec"),),
        (pos_key, p.account.clone(), size_delta_usd, execution_price, pnl_usd),
    );

    DecreasePositionResult {
        execution_price,
        pnl_usd,
        output_amount,
        secondary_output_amount: 0,
        remaining_collateral,
        is_fully_closed,
    }
}
