//! Position utilities — per-position PnL, fee calculation, validation, and liquidation check.
//! Mirrors GMX's PositionUtils.sol, PositionStoreUtils.sol, and related helpers.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env};
use gmx_types::{MarketProps, PositionProps, PositionFees, PriceProps};
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION, mul_div_wide, mul_div_wide_up};
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

    // 1. BORROWING FEE — round up so the protocol never under-collects
    let cum_borrow_key = cumulative_borrowing_factor_key(env, &market.market_token, position.is_long);
    let cum_borrow_factor = ds.get_u128(&cum_borrow_key) as i128;
    let borrow_delta = (cum_borrow_factor - position.borrowing_factor).max(0);
    // fee = delta × size_in_tokens / FLOAT_PRECISION  (round up → protocol favor)
    let borrowing_fee_amount = mul_div_wide_up(env, borrow_delta, position.size_in_tokens, FLOAT_PRECISION);

    // 2. FUNDING FEE — round up so the protocol never under-collects
    let funding_key = funding_amount_per_size_key(
        env, &market.market_token, &position.collateral_token, position.is_long
    );
    let latest_funding = ds.get_i128(&funding_key);
    let funding_delta = latest_funding - position.funding_fee_amount_per_size;
    // If delta > 0: position owes funding; if <= 0: position is owed (claimable, fee = 0 here)
    let funding_fee_amount = if funding_delta > 0 {
        // fee in collateral tokens = delta × size_in_usd / FLOAT_PRECISION / collateral_price × TOKEN_PRECISION
        // Each division rounds up to ensure the owed amount is never under-charged
        let fee_usd = mul_div_wide_up(env, funding_delta, position.size_in_usd, FLOAT_PRECISION);
        if collateral_token_price > 0 {
            mul_div_wide_up(env, fee_usd, TOKEN_PRECISION, collateral_token_price)
        } else {
            0
        }
    } else {
        0
    };

    // 3. POSITION FEE (opening/closing fee) — round up so the protocol never under-collects
    let fee_factor_key = position_fee_factor_key(env, &market.market_token, for_positive_impact);
    let fee_factor = ds.get_u128(&fee_factor_key) as i128;
    let position_fee_usd = mul_div_wide_up(env, size_delta_usd, fee_factor, FLOAT_PRECISION);
    let position_fee_amount = if collateral_token_price > 0 {
        mul_div_wide_up(env, position_fee_usd, TOKEN_PRECISION, collateral_token_price)
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
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
        }
    }

    // 2. MAX LEVERAGE check
    let max_lev_key = max_leverage_key(env, &market.market_token);
    let max_leverage = ds.get_u128(&max_lev_key) as i128;
    if max_leverage > 0 && collateral_usd > 0 {
        let effective_leverage = mul_div_wide(env, position.size_in_usd, FLOAT_PRECISION, collateral_usd);
        if effective_leverage > max_leverage {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(2u32));
        }
    }

    // 3. OPEN INTEREST check
    if validate_open_interest(env, data_store, market, position.is_long).is_err() {
        soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(3u32));
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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_types::{MarketProps, PositionProps, PriceProps};
    use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};

    const ONE_TOKEN: i128 = 10_000_000;
    const FP: i128 = FLOAT_PRECISION;

    struct World {
        env:       Env,
        admin:     Address,
        ds:        Address,
        market_tk: Address,
        long_tk:   Address,
        short_tk:  Address,
        index_tk:  Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::controller(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let market_tk = Address::generate(&env);
        let long_tk   = Address::generate(&env);
        let short_tk  = Address::generate(&env);
        let index_tk  = Address::generate(&env);

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
        ds_c.set_address(&admin, &gmx_keys::market_long_token_key(&env, &market_tk),  &long_tk);
        ds_c.set_address(&admin, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

        World { env, admin, ds, market_tk, long_tk, short_tk, index_tk }
    }

    fn make_market(w: &World) -> MarketProps {
        MarketProps {
            market_token: w.market_tk.clone(),
            index_token:  w.index_tk.clone(),
            long_token:   w.long_tk.clone(),
            short_token:  w.short_tk.clone(),
        }
    }

    fn make_position(w: &World, size_usd: i128, collateral: i128, index_price: i128) -> PositionProps {
        let size_in_tokens = gmx_math::mul_div_wide(&w.env, size_usd, TOKEN_PRECISION, index_price);
        PositionProps {
            account:                     w.admin.clone(),
            market:                      w.market_tk.clone(),
            collateral_token:            w.long_tk.clone(),
            size_in_usd:                 size_usd,
            size_in_tokens,
            collateral_amount:           collateral,
            pending_impact_amount:       0,
            borrowing_factor:            0,
            funding_fee_amount_per_size: 0,
            long_claim_fnd_per_size:     0,
            short_claim_fnd_per_size:    0,
            increased_at_time:           1_000,
            decreased_at_time:           0,
            is_long:                     true,
        }
    }

    // ── Task 1: get_position_fees ─────────────────────────────────────────────

    /// With zero fee factors configured, all fee components are zero.
    #[test]
    fn position_fees_are_zero_when_factors_unset() {
        let w = setup();
        let market   = make_market(&w);
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);

        let fees = get_position_fees(
            &w.env, &w.ds, &market, &position,
            2_000 * FP, 1_000 * FP, true,
        );

        assert_eq!(fees.borrowing_fee_amount, 0, "borrowing fee must be 0 with no factor");
        assert_eq!(fees.funding_fee_amount,   0, "funding fee must be 0 with no delta");
        assert_eq!(fees.position_fee_amount,  0, "position fee must be 0 with no factor");
        assert_eq!(fees.total_cost_amount,    0);
    }

    /// Position fee matches the expected bps formula.
    #[test]
    fn position_fee_matches_bps_formula() {
        let w = setup();
        let fee_bps: i128 = 30; // 30 bps
        let fee_factor = fee_bps * FP / 10_000;
        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(&w.admin, &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),  &(fee_factor as u128));

        let market       = make_market(&w);
        let index_price  = 2_000 * FP;
        let size_delta   = 1_000 * FP;
        let position     = make_position(&w, size_delta, ONE_TOKEN * 10, index_price);

        let fees = get_position_fees(
            &w.env, &w.ds, &market, &position,
            index_price, size_delta, true,
        );

        let fee_usd          = gmx_math::mul_div_wide(&w.env, size_delta, fee_factor, FP);
        let expected_fee_tok = gmx_math::mul_div_wide(&w.env, fee_usd, TOKEN_PRECISION, index_price);

        assert!(fees.position_fee_amount > 0,   "position fee must be non-zero");
        assert_eq!(fees.position_fee_amount, expected_fee_tok, "position fee must match formula");
    }

    /// Borrowing fee is proportional to the cumulative factor delta and position size in tokens.
    #[test]
    fn borrowing_fee_proportional_to_cum_factor_delta() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        // Seed a cumulative borrowing factor > position snapshot (0)
        let cum_factor: i128 = FP / 1_000; // small factor
        ds_c.set_u128(&w.admin, &gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true),
            &(cum_factor as u128));

        let market   = make_market(&w);
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);
        // position.borrowing_factor = 0, cum = cum_factor → delta = cum_factor

        let fees = get_position_fees(
            &w.env, &w.ds, &market, &position,
            2_000 * FP, 1_000 * FP, true,
        );

        let expected = gmx_math::mul_div_wide(&w.env, cum_factor, position.size_in_tokens, FP);
        assert_eq!(fees.borrowing_fee_amount, expected, "borrowing fee must match formula");
    }

    // ── Task 1: settle_funding_fees ───────────────────────────────────────────

    /// settle_funding_fees credits claimable amount when position is owed funding.
    #[test]
    fn settle_funding_credits_claimable_when_owed() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        // Set global funding-per-size to negative: position (tracker=0) is owed funding
        let fnd_key = gmx_keys::funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true);
        let funding_per_size: i128 = -(FP / 100_000); // small negative
        ds_c.apply_delta_to_i128(&w.admin, &fnd_key, &funding_per_size);

        let market   = make_market(&w);
        let mut pos  = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);
        // pos.long_claim_fnd_per_size = 0 > funding_per_size → claimable

        settle_funding_fees(&w.env, &w.ds, &w.admin, &market, &mut pos);

        let claim_key = gmx_keys::claimable_funding_amount_key(&w.env, &w.market_tk, &w.long_tk, &w.admin);
        let claimable = ds_c.get_u128(&claim_key);
        assert!(claimable > 0, "claimable funding must be credited when position is owed funding");
    }

    /// After settle_funding_fees, the position's tracker is updated to the current global value.
    #[test]
    fn settle_funding_resets_tracker_to_current_global() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        let fnd_key      = gmx_keys::funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true);
        let global_value: i128 = FP / 50_000;
        ds_c.apply_delta_to_i128(&w.admin, &fnd_key, &global_value);

        let market   = make_market(&w);
        let mut pos  = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);

        settle_funding_fees(&w.env, &w.ds, &w.admin, &market, &mut pos);

        // After settlement the position's funding_fee_amount_per_size must equal the global
        assert_eq!(pos.funding_fee_amount_per_size, global_value,
            "position tracker must be reset to current global after settlement");
    }
}
