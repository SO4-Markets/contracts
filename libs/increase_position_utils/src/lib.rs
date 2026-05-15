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

use soroban_sdk::{contracttype, Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PriceProps};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide};
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

// ─── Position storage key used within the calling contract ────────────────────

#[contracttype]
enum PositionKey {
    Position(BytesN<32>),
}

// ─── Params ───────────────────────────────────────────────────────────────────

pub struct IncreasePositionParams<'a> {
    pub data_store:        &'a Address,
    pub caller:            &'a Address,   // handler contract address (has CONTROLLER)
    pub account:           &'a Address,   // position owner
    pub receiver:          &'a Address,   // where excess collateral goes (unused here, for symmetry)
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
///
/// Positions are stored in the **calling contract's** persistent storage
/// (typically order_handler) keyed by position_key(account, market, collateral, is_long).
pub fn increase_position(env: &Env, p: &IncreasePositionParams) -> PositionProps {
    let pos_key = position_key(env, p.account, &p.market.market_token, p.collateral_token, p.is_long);
    let storage_key = PositionKey::Position(pos_key.clone());

    // 1. Load or create position
    let is_new = !env.storage().persistent().has(&storage_key);
    let mut position: PositionProps = env.storage().persistent()
        .get(&storage_key)
        .unwrap_or_else(|| PositionProps {
            account:                   p.account.clone(),
            market:                    p.market.market_token.clone(),
            collateral_token:          p.collateral_token.clone(),
            size_in_usd:               0,
            size_in_tokens:            0,
            collateral_amount:         0,
            pending_impact_amount:     0,
            borrowing_factor:          0,
            funding_fee_amount_per_size: 0,
            long_claim_fnd_per_size:   0,
            short_claim_fnd_per_size:  0,
            increased_at_time:         0,
            decreased_at_time:         0,
            is_long:                   p.is_long,
        });

    // 2. Update market funding + borrowing state before modifying position
    let index_price = p.index_token_price.mid_price();
    update_funding_state(env, p.data_store, p.caller, p.market, index_price, index_price, p.current_time);
    update_cumulative_borrowing_factor(env, p.data_store, p.caller, p.market, p.is_long, p.current_time);

    // 3. Settle any pending funding owed to this position
    settle_funding_fees(env, p.data_store, p.caller, p.market, &mut position);

    // 4. Price impact
    let impact_usd = get_position_price_impact(
        env, p.data_store, p.market,
        p.is_long, p.size_delta_usd, true,
        index_price,
    );
    apply_position_impact_value(env, p.data_store, p.caller, p.market, impact_usd, index_price);

    // 5. Execution price
    let execution_price = get_execution_price(env, index_price, p.size_delta_usd, impact_usd, p.is_long, true);
    if p.acceptable_price != 0 {
        if p.is_long && execution_price > p.acceptable_price {
            panic!("execution price too high for long");
        }
        if !p.is_long && execution_price < p.acceptable_price {
            panic!("execution price too low for short");
        }
    }

    // 6. New size in tokens = size_delta_usd / execution_price (in raw 7-decimal units)
    let new_size_in_tokens = if execution_price > 0 {
        mul_div_wide(env, p.size_delta_usd, TOKEN_PRECISION, execution_price)
    } else {
        0
    };

    // 7. Position fees (deducted from collateral)
    let for_positive_impact = impact_usd >= 0;
    let fees = get_position_fees(
        env, p.data_store, p.market, &position,
        p.collateral_price, p.size_delta_usd, for_positive_impact,
    );

    // 8. Update collateral: add deposited, subtract fees
    position.collateral_amount += p.collateral_amount - fees.total_cost_amount;
    if position.collateral_amount < 0 {
        panic!("insufficient collateral for fees");
    }

    // 9. Update position size and funding/borrowing trackers
    position.size_in_usd    += p.size_delta_usd;
    position.size_in_tokens += new_size_in_tokens;
    position.increased_at_time = p.current_time;

    // Sync borrowing factor to current cumulative value
    let cum_borrow_key = cumulative_borrowing_factor_key(env, &p.market.market_token, p.is_long);
    position.borrowing_factor = DataStoreClient::new(env, p.data_store).get_u128(&cum_borrow_key) as i128;

    // Sync funding per-size tracker
    let fnd_key = funding_amount_per_size_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    position.funding_fee_amount_per_size = DataStoreClient::new(env, p.data_store).get_i128(&fnd_key);

    // 10. Open interest deltas
    apply_delta_to_open_interest(env, p.data_store, p.caller, p.market, p.collateral_token, p.is_long, p.size_delta_usd);
    apply_delta_to_open_interest_in_tokens(env, p.data_store, p.caller, p.market, p.collateral_token, p.is_long, new_size_in_tokens);

    // 11. Collateral sum
    let col_sum_key = collateral_sum_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    DataStoreClient::new(env, p.data_store)
        .apply_delta_to_u128(p.caller, &col_sum_key, &(p.collateral_amount));

    // 12. Pool gets the fee income
    apply_delta_to_pool_amount(env, p.data_store, p.caller, p.market, p.collateral_token, fees.total_cost_amount);

    // 13. Validate position (leverage, min collateral, max OI)
    validate_position(env, p.data_store, &position, p.market, p.collateral_price, p.index_token_price);

    // 14. Persist
    env.storage().persistent().set(&storage_key, &position);

    // If brand-new position, add to the tracking sets
    if is_new {
        let ds = DataStoreClient::new(env, p.data_store);
        ds.add_bytes32_to_set(p.caller, &position_list_key(env), &pos_key);
        ds.add_bytes32_to_set(p.caller, &account_position_list_key(env, p.account), &pos_key);
    }

    env.events().publish(
        (soroban_sdk::symbol_short!("pos_inc"),),
        (pos_key, p.account.clone(), p.size_delta_usd, execution_price),
    );

    position
}
