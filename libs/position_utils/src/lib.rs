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
    position_key,
};
use gmx_market_utils::validate_open_interest;

// ─── Data-store client (same minimal interface used across libs) ───────────────

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

// ─── PnL ─────────────────────────────────────────────────────────────────────

/// Unrealised PnL in USD (FLOAT_PRECISION) for a full or partial close.
///
/// `size_delta_usd` — the portion of the position being closed (= position.size_in_usd for full).
///
/// Returns (pnl_usd, uncapped_pnl_usd) — same value for now; capping happens in get_pool_value.
pub fn get_position_pnl_usd(
    env: &Env,
    position: &PositionProps,
    index_token_price: &PriceProps,
    size_delta_usd: i128,
) -> (i128, i128) {
    if position.size_in_usd == 0 || position.size_in_tokens == 0 {
        return (0, 0);
    }

    // Pick the price that maximises PnL for the trader:
    //   Long: higher price = more profit → use max
    //   Short: lower price = more profit → use min
    let price = index_token_price.pick_price_for_pnl(position.is_long, true);

    // Current value of all position tokens in USD (FLOAT_PRECISION)
    let position_value = mul_div_wide(env, position.size_in_tokens, price, TOKEN_PRECISION);

    // Unrealised PnL for the full position
    let total_pnl = if position.is_long {
        position_value - position.size_in_usd
    } else {
        position.size_in_usd - position_value
    };

    // Scale to the slice being closed
    let pnl_usd = mul_div_wide(env, total_pnl, size_delta_usd, position.size_in_usd);

    (pnl_usd, pnl_usd)
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
    let ds = DataStoreClient::new(env, data_store);

    // 1. BORROWING FEE
    let cum_borrow_key = cumulative_borrowing_factor_key(env, &market.market_token, position.is_long);
    let cum_borrow_factor = ds.get_u128(&cum_borrow_key) as i128;
    let borrow_delta = (cum_borrow_factor - position.borrowing_factor).max(0);
    // fee = delta × size_in_tokens / FLOAT_PRECISION  (result is raw collateral token units)
    let borrowing_fee_amount = mul_div_wide(env, borrow_delta, position.size_in_tokens, FLOAT_PRECISION);

    // 2. FUNDING FEE
    let funding_key = funding_amount_per_size_key(
        env, &market.market_token, &position.collateral_token, position.is_long
    );
    let latest_funding = ds.get_i128(&funding_key);
    let funding_delta = latest_funding - position.funding_fee_amount_per_size;
    // If delta > 0: position owes funding; if <= 0: position is owed (claimable, fee = 0 here)
    let funding_fee_amount = if funding_delta > 0 {
        // fee in collateral tokens = delta × size_in_usd / FLOAT_PRECISION / collateral_price × TOKEN_PRECISION
        let fee_usd = mul_div_wide(env, funding_delta, position.size_in_usd, FLOAT_PRECISION);
        if collateral_token_price > 0 {
            mul_div_wide(env, fee_usd, TOKEN_PRECISION, collateral_token_price)
        } else {
            0
        }
    } else {
        0
    };

    // 3. POSITION FEE (opening/closing fee)
    let fee_factor_key = position_fee_factor_key(env, &market.market_token, for_positive_impact);
    let fee_factor = ds.get_u128(&fee_factor_key) as i128;
    let position_fee_usd = mul_div_wide(env, size_delta_usd, fee_factor, FLOAT_PRECISION);
    let position_fee_amount = if collateral_token_price > 0 {
        mul_div_wide(env, position_fee_usd, TOKEN_PRECISION, collateral_token_price)
    } else {
        0
    };

    let total_cost_amount = borrowing_fee_amount + funding_fee_amount + position_fee_amount;

    PositionFees {
        borrowing_fee_amount,
        funding_fee_amount,
        position_fee_amount,
        total_cost_amount,
    }
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
    let ds = DataStoreClient::new(env, data_store);

    // For each collateral token side, check if the position is owed funding (negative delta means owed)
    for (collateral_token, tracker) in [
        (&market.long_token,  position.long_claim_fnd_per_size),
        (&market.short_token, position.short_claim_fnd_per_size),
    ] {
        let fnd_key = funding_amount_per_size_key(env, &market.market_token, collateral_token, position.is_long);
        let latest = ds.get_i128(&fnd_key);
        // Negative delta → position is owed funding from the other side
        let claimable_per_size = tracker - latest; // positive if position is owed
        if claimable_per_size > 0 {
            let claimable_amount = mul_div_wide(env, claimable_per_size, position.size_in_usd, FLOAT_PRECISION);
            if claimable_amount > 0 {
                let claim_key = claimable_funding_amount_key(env, &market.market_token, collateral_token, &position.account);
                ds.apply_delta_to_u128(caller, &claim_key, &claimable_amount);
            }
        }
    }

    // Reset trackers to current values so there's no double-counting next time
    let long_fnd_key = funding_amount_per_size_key(env, &market.market_token, &market.long_token, position.is_long);
    let short_fnd_key = funding_amount_per_size_key(env, &market.market_token, &market.short_token, position.is_long);
    position.long_claim_fnd_per_size = ds.get_i128(&long_fnd_key);
    position.short_claim_fnd_per_size = ds.get_i128(&short_fnd_key);

    // Also update the owed-funding tracker (for positions that PAY funding)
    let owned_key = funding_amount_per_size_key(env, &market.market_token, &position.collateral_token, position.is_long);
    position.funding_fee_amount_per_size = ds.get_i128(&owned_key);
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
    _index_token_price: &PriceProps,
) {
    let ds = DataStoreClient::new(env, data_store);

    // Collateral in USD
    let collateral_usd = mul_div_wide(env, position.collateral_amount, collateral_token_price, TOKEN_PRECISION);

    // 1. MIN COLLATERAL check
    let min_col_key = min_collateral_factor_key(env, &market.market_token);
    let min_collateral_factor = ds.get_u128(&min_col_key) as i128;
    if min_collateral_factor > 0 {
        let required_min = mul_div_wide(env, position.size_in_usd, min_collateral_factor, FLOAT_PRECISION);
        if collateral_usd < required_min {
            panic!("min collateral violated");
        }
    }

    // 2. MAX LEVERAGE check
    let max_lev_key = max_leverage_key(env, &market.market_token);
    let max_leverage = ds.get_u128(&max_lev_key) as i128;
    if max_leverage > 0 && collateral_usd > 0 {
        let effective_leverage = mul_div_wide(env, position.size_in_usd, FLOAT_PRECISION, collateral_usd);
        if effective_leverage > max_leverage {
            panic!("max leverage exceeded");
        }
    }

    // 3. OPEN INTEREST check
    if validate_open_interest(env, data_store, market, position.is_long).is_err() {
        panic!("max open interest exceeded");
    }
}

/// Returns true if the position can be liquidated at current prices.
pub fn is_liquidatable(
    env: &Env,
    data_store: &Address,
    position: &PositionProps,
    market: &MarketProps,
    collateral_token_price: i128,
    index_token_price: &PriceProps,
) -> bool {
    if position.size_in_usd == 0 {
        return false;
    }

    // 1. All current fees (worst case: not for positive impact)
    let fees = get_position_fees(
        env, data_store, market, position,
        collateral_token_price, position.size_in_usd, false,
    );

    // 2. Unrealised PnL using price that MINIMISES profit (worst case for trader)
    let worst_price = index_token_price.pick_price_for_pnl(position.is_long, false);
    let worst_price_props = PriceProps { min: worst_price, max: worst_price };
    let (pnl_usd, _) = get_position_pnl_usd(env, position, &worst_price_props, position.size_in_usd);

    // 3. Remaining collateral in USD after fees and PnL
    let collateral_usd = mul_div_wide(env, position.collateral_amount, collateral_token_price, TOKEN_PRECISION);
    let fees_usd = mul_div_wide(env, fees.total_cost_amount, collateral_token_price, TOKEN_PRECISION);
    let remaining = collateral_usd - fees_usd + pnl_usd;

    // 4. Min required collateral
    let ds = DataStoreClient::new(env, data_store);
    let min_col_key = min_collateral_factor_key(env, &market.market_token);
    let min_collateral_factor = ds.get_u128(&min_col_key) as i128;

    if min_collateral_factor == 0 {
        // No limit configured — fall back to: remaining < 0
        return remaining < 0;
    }

    let min_required = mul_div_wide(env, position.size_in_usd, min_collateral_factor, FLOAT_PRECISION);
    remaining < min_required
}

// ─── Position key ─────────────────────────────────────────────────────────────

/// Compute the data_store key for a position.
pub fn get_position_key(
    env: &Env,
    account: &Address,
    market_token: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    position_key(env, account, market_token, collateral_token, is_long)
}
