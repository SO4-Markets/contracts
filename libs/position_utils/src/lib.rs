//! Position utilities — per-position PnL, fee calculation, validation, and liquidation check.
//! Mirrors GMX's PositionUtils.sol, PositionStoreUtils.sol, and related helpers.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    claimable_funding_amount_key, cumulative_borrowing_factor_key, funding_amount_per_size_key,
    max_leverage_key, min_collateral_factor_key, position_fee_factor_key, position_key,
};
use gmx_market_utils::validate_open_interest;
use gmx_math::{mul_div_wide, mul_div_wide_up, FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_types::{MarketProps, PositionFees, PositionProps, PriceProps};
use soroban_sdk::{Address, BytesN, Env};

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
    collateral_token_price: i128, // FLOAT_PRECISION
    size_delta_usd: i128,
    for_positive_impact: bool,
) -> PositionFees {
    let ds = DataStoreClient::new(env, data_store);

    // 1. BORROWING FEE — round up so the protocol never under-collects
    let cum_borrow_key =
        cumulative_borrowing_factor_key(env, &market.market_token, position.is_long);
    let cum_borrow_factor = ds.get_u128(&cum_borrow_key) as i128;
    let borrow_delta = (cum_borrow_factor - position.borrowing_factor).max(0);
    // fee = delta × size_in_tokens / FLOAT_PRECISION  (round up → protocol favor)
    let borrowing_fee_amount =
        mul_div_wide_up(env, borrow_delta, position.size_in_tokens, FLOAT_PRECISION);

    // 2. FUNDING FEE — round up so the protocol never under-collects
    let funding_key = funding_amount_per_size_key(
        env,
        &market.market_token,
        &position.collateral_token,
        position.is_long,
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
        mul_div_wide_up(
            env,
            position_fee_usd,
            TOKEN_PRECISION,
            collateral_token_price,
        )
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
        (&market.long_token, position.long_claim_fnd_per_size),
        (&market.short_token, position.short_claim_fnd_per_size),
    ] {
        let fnd_key = funding_amount_per_size_key(
            env,
            &market.market_token,
            collateral_token,
            position.is_long,
        );
        let latest = ds.get_i128(&fnd_key);
        // Negative delta → position is owed funding from the other side
        let claimable_per_size = tracker - latest; // positive if position is owed
        if claimable_per_size > 0 {
            // Round DOWN (floor): credit the position the floor amount so the pool
            // never pays out more than it mathematically owes.
            let claimable_amount = mul_div_wide(
                env,
                claimable_per_size,
                position.size_in_usd,
                FLOAT_PRECISION,
            );
            if claimable_amount > 0 {
                let claim_key = claimable_funding_amount_key(
                    env,
                    &market.market_token,
                    collateral_token,
                    &position.account,
                );
                ds.apply_delta_to_u128(caller, &claim_key, &claimable_amount);
            }
        }
    }

    // Reset trackers to current values so there's no double-counting next time
    let long_fnd_key = funding_amount_per_size_key(
        env,
        &market.market_token,
        &market.long_token,
        position.is_long,
    );
    let short_fnd_key = funding_amount_per_size_key(
        env,
        &market.market_token,
        &market.short_token,
        position.is_long,
    );
    position.long_claim_fnd_per_size = ds.get_i128(&long_fnd_key);
    position.short_claim_fnd_per_size = ds.get_i128(&short_fnd_key);

    // Also update the owed-funding tracker (for positions that PAY funding)
    let owned_key = funding_amount_per_size_key(
        env,
        &market.market_token,
        &position.collateral_token,
        position.is_long,
    );
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
    let collateral_usd = mul_div_wide(
        env,
        position.collateral_amount,
        collateral_token_price,
        TOKEN_PRECISION,
    );

    // 1. MIN COLLATERAL check
    let min_col_key = min_collateral_factor_key(env, &market.market_token);
    let min_collateral_factor = ds.get_u128(&min_col_key) as i128;
    if min_collateral_factor > 0 {
        let required_min = mul_div_wide(
            env,
            position.size_in_usd,
            min_collateral_factor,
            FLOAT_PRECISION,
        );
        if collateral_usd < required_min {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
        }
    }

    // 2. MAX LEVERAGE check
    let max_lev_key = max_leverage_key(env, &market.market_token);
    let max_leverage = ds.get_u128(&max_lev_key) as i128;
    if max_leverage > 0 && collateral_usd > 0 {
        let effective_leverage =
            mul_div_wide(env, position.size_in_usd, FLOAT_PRECISION, collateral_usd);
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
        env,
        data_store,
        market,
        position,
        collateral_token_price,
        position.size_in_usd,
        false,
    );

    // 2. Unrealised PnL using price that MINIMISES profit (worst case for trader)
    let worst_price = index_token_price.pick_price_for_pnl(position.is_long, false);
    let worst_price_props = PriceProps {
        min: worst_price,
        max: worst_price,
    };
    let (pnl_usd, _) =
        get_position_pnl_usd(env, position, &worst_price_props, position.size_in_usd);

    // 3. Remaining collateral in USD after fees and PnL
    let collateral_usd = mul_div_wide(
        env,
        position.collateral_amount,
        collateral_token_price,
        TOKEN_PRECISION,
    );
    let fees_usd = mul_div_wide(
        env,
        fees.total_cost_amount,
        collateral_token_price,
        TOKEN_PRECISION,
    );
    // net_collateral excludes PnL — used for the min_collateral_factor adequacy check.
    // PnL should not mask a collateral shortfall: a profitable unrealised gain does
    // not mean the deposited collateral is sufficient to absorb liquidation costs.
    let net_collateral = collateral_usd - fees_usd;
    let remaining = net_collateral + pnl_usd;

    // 4. Min required collateral
    let ds = DataStoreClient::new(env, data_store);
    let min_col_key = min_collateral_factor_key(env, &market.market_token);
    let min_collateral_factor = ds.get_u128(&min_col_key) as i128;

    if min_collateral_factor == 0 {
        // No limit configured — fall back to: remaining < 0
        return remaining < 0;
    }

    let min_required = mul_div_wide(
        env,
        position.size_in_usd,
        min_collateral_factor,
        FLOAT_PRECISION,
    );
    // Use net_collateral (no PnL) so unrealised gains cannot hide a collateral shortfall.
    net_collateral < min_required
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
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
    use gmx_types::{MarketProps, PositionProps, PriceProps};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};

    const ONE_TOKEN: i128 = 10_000_000;
    const FP: i128 = FLOAT_PRECISION;

    struct World {
        env: Env,
        admin: Address,
        ds: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
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
        let long_tk = Address::generate(&env);
        let short_tk = Address::generate(&env);
        let index_tk = Address::generate(&env);

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &admin,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_short_token_key(&env, &market_tk),
            &short_tk,
        );

        World {
            env,
            admin,
            ds,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn make_market(w: &World) -> MarketProps {
        // Issue #248: build via the shared constructor instead of a per-field literal.
        MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk)
    }

    fn make_position(
        w: &World,
        size_usd: i128,
        collateral: i128,
        index_price: i128,
    ) -> PositionProps {
        let size_in_tokens = gmx_math::mul_div_wide(&w.env, size_usd, TOKEN_PRECISION, index_price);
        PositionProps {
            account: w.admin.clone(),
            market: w.market_tk.clone(),
            collateral_token: w.long_tk.clone(),
            size_in_usd: size_usd,
            size_in_tokens,
            collateral_amount: collateral,
            pending_impact_amount: 0,
            borrowing_factor: 0,
            funding_fee_amount_per_size: 0,
            long_claim_fnd_per_size: 0,
            short_claim_fnd_per_size: 0,
            increased_at_time: 1_000,
            decreased_at_time: 0,
            is_long: true,
        }
    }

    // ── Task 1: get_position_fees ─────────────────────────────────────────────

    /// With zero fee factors configured, all fee components are zero.
    #[test]
    fn position_fees_are_zero_when_factors_unset() {
        let w = setup();
        let market = make_market(&w);
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            2_000 * FP,
            1_000 * FP,
            true,
        );

        assert_eq!(
            fees.borrowing_fee_amount, 0,
            "borrowing fee must be 0 with no factor"
        );
        assert_eq!(
            fees.funding_fee_amount, 0,
            "funding fee must be 0 with no delta"
        );
        assert_eq!(
            fees.position_fee_amount, 0,
            "position fee must be 0 with no factor"
        );
        assert_eq!(fees.total_cost_amount, 0);
    }

    /// Position fee matches the expected bps formula.
    #[test]
    fn position_fee_matches_bps_formula() {
        let w = setup();
        let fee_bps: i128 = 30; // 30 bps
        let fee_factor = fee_bps * FP / 10_000;
        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );

        let market = make_market(&w);
        let index_price = 2_000 * FP;
        let size_delta = 1_000 * FP;
        let position = make_position(&w, size_delta, ONE_TOKEN * 10, index_price);

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            index_price,
            size_delta,
            true,
        );

        let fee_usd = gmx_math::mul_div_wide(&w.env, size_delta, fee_factor, FP);
        let expected_fee_tok =
            gmx_math::mul_div_wide(&w.env, fee_usd, TOKEN_PRECISION, index_price);

        assert!(
            fees.position_fee_amount > 0,
            "position fee must be non-zero"
        );
        assert_eq!(
            fees.position_fee_amount, expected_fee_tok,
            "position fee must match formula"
        );
    }

    /// Borrowing fee is proportional to the cumulative factor delta and position size in tokens.
    #[test]
    fn borrowing_fee_proportional_to_cum_factor_delta() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        // Seed a cumulative borrowing factor > position snapshot (0)
        let cum_factor: i128 = FP / 1_000; // small factor
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true),
            &(cum_factor as u128),
        );

        let market = make_market(&w);
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);
        // position.borrowing_factor = 0, cum = cum_factor → delta = cum_factor

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            2_000 * FP,
            1_000 * FP,
            true,
        );

        let expected = gmx_math::mul_div_wide(&w.env, cum_factor, position.size_in_tokens, FP);
        assert_eq!(
            fees.borrowing_fee_amount, expected,
            "borrowing fee must match formula"
        );
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

        let market = make_market(&w);
        let mut pos = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);
        // pos.long_claim_fnd_per_size = 0 > funding_per_size → claimable

        settle_funding_fees(&w.env, &w.ds, &w.admin, &market, &mut pos);

        let claim_key =
            gmx_keys::claimable_funding_amount_key(&w.env, &w.market_tk, &w.long_tk, &w.admin);
        let claimable = ds_c.get_u128(&claim_key);
        assert!(
            claimable > 0,
            "claimable funding must be credited when position is owed funding"
        );
    }

    /// After settle_funding_fees, the position's tracker is updated to the current global value.
    #[test]
    fn settle_funding_resets_tracker_to_current_global() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        let fnd_key = gmx_keys::funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true);
        let global_value: i128 = FP / 50_000;
        ds_c.apply_delta_to_i128(&w.admin, &fnd_key, &global_value);

        let market = make_market(&w);
        let mut pos = make_position(&w, 1_000 * FP, ONE_TOKEN * 10, 2_000 * FP);

        settle_funding_fees(&w.env, &w.ds, &w.admin, &market, &mut pos);

        // After settlement the position's funding_fee_amount_per_size must equal the global
        assert_eq!(
            pos.funding_fee_amount_per_size, global_value,
            "position tracker must be reset to current global after settlement"
        );
    }

    // ── Issue #136: property tests for position_utils ─────────────────────────

    /// get_position_pnl_usd returns 0 when the position has zero size.
    #[test]
    fn property_pnl_zero_for_zero_size_position() {
        let w = setup();
        let mut pos = make_position(&w, 0, 0, 2_000 * FP);
        pos.size_in_tokens = 0;
        let price_props = PriceProps {
            min: 2_000 * FP,
            max: 2_000 * FP,
        };
        let (pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, 0);
        assert_eq!(pnl, 0, "zero-size position must have zero PnL");
    }

    /// A long position has zero PnL when the current price equals the entry price.
    #[test]
    fn property_long_pnl_zero_at_entry_price() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let size_usd = 1_000 * FP;
        let pos = make_position(&w, size_usd, ONE_TOKEN * 10, entry_price);
        let price_props = PriceProps {
            min: entry_price,
            max: entry_price,
        };
        let (pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);
        // pnl = tokens * price / TOKEN_PRECISION - size_in_usd
        //     = (size_usd / entry_price * TOKEN_PRECISION) * entry_price / TOKEN_PRECISION - size_usd
        //     ≈ 0  (up to rounding)
        let tol = 1i128;
        assert!(
            pnl.abs() <= tol,
            "long PnL must be ~0 at entry price; got {pnl}"
        );
    }

    /// A long position has negative PnL when the price drops below entry.
    #[test]
    fn property_long_pnl_negative_when_price_falls() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let exit_price = 1_000 * FP; // halved
        let size_usd = 1_000 * FP;
        let pos = make_position(&w, size_usd, ONE_TOKEN * 10, entry_price);
        let price_props = PriceProps {
            min: exit_price,
            max: exit_price,
        };
        let (pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);
        assert!(
            pnl < 0,
            "long position must have negative PnL when price falls; got {pnl}"
        );
    }

    /// A long position has positive PnL when the price rises above entry.
    #[test]
    fn property_long_pnl_positive_when_price_rises() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let exit_price = 4_000 * FP; // doubled
        let size_usd = 1_000 * FP;
        let pos = make_position(&w, size_usd, ONE_TOKEN * 10, entry_price);
        let price_props = PriceProps {
            min: exit_price,
            max: exit_price,
        };
        let (pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);
        assert!(
            pnl > 0,
            "long position must have positive PnL when price rises; got {pnl}"
        );
    }

    /// All fee components are non-negative (no underflow into negative fees).
    #[test]
    fn property_position_fees_never_negative() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        // Seed non-zero fee factors
        let fee_factor: i128 = FP / 1_000; // 0.1%
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );

        let cum_factor: i128 = FP / 500;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true),
            &(cum_factor as u128),
        );

        let market = make_market(&w);
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 5, 2_000 * FP);

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            2_000 * FP,
            1_000 * FP,
            true,
        );

        assert!(
            fees.borrowing_fee_amount >= 0,
            "borrowing fee must not be negative: {}",
            fees.borrowing_fee_amount
        );
        assert!(
            fees.funding_fee_amount >= 0,
            "funding fee must not be negative: {}",
            fees.funding_fee_amount
        );
        assert!(
            fees.position_fee_amount >= 0,
            "position fee must not be negative: {}",
            fees.position_fee_amount
        );
        assert!(
            fees.total_cost_amount >= 0,
            "total cost must not be negative: {}",
            fees.total_cost_amount
        );
    }

    /// total_cost_amount == borrowing + funding + position fees (no hidden component).
    #[test]
    fn property_total_cost_is_sum_of_components() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        let fee_factor: i128 = FP / 2_000;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );

        let market = make_market(&w);
        let position = make_position(&w, 2_000 * FP, ONE_TOKEN * 8, 2_000 * FP);

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            2_000 * FP,
            1_000 * FP,
            true,
        );

        assert_eq!(
            fees.total_cost_amount,
            fees.borrowing_fee_amount + fees.funding_fee_amount + fees.position_fee_amount,
            "total_cost must equal sum of components"
        );
    }

    /// Partial close PnL scales linearly with size_delta (property: proportionality).
    /// Closing 50% of a position should yield 50% of the full PnL (within rounding).
    #[test]
    fn property_partial_close_pnl_proportional_to_size() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let exit_price = 3_000 * FP;
        let size_usd = 1_000 * FP;
        let pos = make_position(&w, size_usd, ONE_TOKEN * 10, entry_price);
        let price_props = PriceProps {
            min: exit_price,
            max: exit_price,
        };

        let (full_pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);
        let (half_pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd / 2);

        // half_pnl should be ~50% of full_pnl (within 1 unit rounding)
        let expected_half = full_pnl / 2;
        assert!(
            (half_pnl - expected_half).abs() <= 1,
            "partial close PnL must be proportional: full={full_pnl}, half={half_pnl}, expected_half={expected_half}"
        );
    }

    /// is_liquidatable returns false for a well-collateralised position at entry price.
    #[test]
    fn property_healthy_position_is_not_liquidatable() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let entry_p = 2_000 * FP;
        let market = make_market(&w);

        // Set min collateral factor (1%)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &((FP / 100) as u128),
        );

        // Large collateral relative to size → very healthy
        let position = make_position(&w, 1_000 * FP, ONE_TOKEN * 100, entry_p);
        let price_props = PriceProps {
            min: entry_p,
            max: entry_p,
        };

        assert!(
            !is_liquidatable(&w.env, &w.ds, &position, &market, entry_p, &price_props),
            "well-collateralised position at entry price must not be liquidatable"
        );
    }

    // ── Issue #83: min collateral factor edge-case tests ─────────────────────
    //
    // Done: Missing config key, zero value, and very high value each produce safe
    //       and documented behavior. Tests cover all three.

    /// When min_collateral_factor_key is never set (DataStore returns 0 for any
    /// unset key), is_liquidatable falls back to the `remaining < 0` check.
    #[test]
    fn is_liquidatable_missing_key_falls_back_to_remaining_check() {
        let w = setup();
        let market = make_market(&w);
        let entry_p = 2_000 * FP;

        // Deeply leveraged long: 1 token collateral, $10 000 notional
        let position = make_position(&w, 10_000 * FP, ONE_TOKEN, entry_p);

        // Price crashes 90% → remaining = collateral_usd + pnl < 0
        let crash_price = 200 * FP;
        let crash_props = PriceProps {
            min: crash_price,
            max: crash_price,
        };
        assert!(
            is_liquidatable(&w.env, &w.ds, &position, &market, crash_price, &crash_props),
            "leveraged long with crashed price and no factor configured must be liquidatable"
        );

        // Well-collateralised long at entry price → remaining > 0 → not liquidatable
        let safe_position = make_position(&w, 1_000 * FP, ONE_TOKEN * 100, entry_p);
        let entry_props = PriceProps {
            min: entry_p,
            max: entry_p,
        };
        assert!(
            !is_liquidatable(
                &w.env,
                &w.ds,
                &safe_position,
                &market,
                entry_p,
                &entry_props
            ),
            "well-collateralised position with no factor configured must not be liquidatable"
        );
    }

    /// Explicitly setting min_collateral_factor to 0 produces the same behaviour as
    /// never setting it — the code path is identical (DataStore returns 0 for both).
    #[test]
    fn is_liquidatable_zero_factor_explicit_same_as_missing() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = make_market(&w);
        let entry_p = 2_000 * FP;

        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &0u128,
        );

        let position = make_position(&w, 10_000 * FP, ONE_TOKEN, entry_p);
        let crash_price = 200 * FP;
        let crash_props = PriceProps {
            min: crash_price,
            max: crash_price,
        };
        assert!(
            is_liquidatable(&w.env, &w.ds, &position, &market, crash_price, &crash_props),
            "factor=0 explicit must behave the same as missing key (remaining < 0)"
        );

        let safe_position = make_position(&w, 1_000 * FP, ONE_TOKEN * 100, entry_p);
        let entry_props = PriceProps {
            min: entry_p,
            max: entry_p,
        };
        assert!(
            !is_liquidatable(
                &w.env,
                &w.ds,
                &safe_position,
                &market,
                entry_p,
                &entry_props
            ),
            "factor=0 explicit: healthy position must not be liquidatable"
        );
    }

    /// A very high min_collateral_factor (100% = FLOAT_PRECISION) makes positions
    /// with collateral_usd < size_in_usd liquidatable. The same position passes
    /// with a low factor.
    #[test]
    fn is_liquidatable_very_high_factor_triggers_liquidation() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = make_market(&w);
        let price = 2_000 * FP;
        let props = PriceProps {
            min: price,
            max: price,
        };

        // Position: $10 000 notional, 1 token collateral → collateral_usd = $2 000
        let position = make_position(&w, 10_000 * FP, ONE_TOKEN, price);

        // 100% factor: min_required = size_in_usd = $10 000 > collateral_usd $2 000 → liquidatable
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &(FP as u128),
        );
        assert!(
            is_liquidatable(&w.env, &w.ds, &position, &market, price, &props),
            "100% factor: position with collateral_usd < size_in_usd must be liquidatable"
        );

        // 10% factor: min_required = $1 000 < collateral_usd $2 000 → not liquidatable
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &((FP / 10) as u128),
        );
        assert!(
            !is_liquidatable(&w.env, &w.ds, &position, &market, price, &props),
            "10% factor: position with 20% collateral ratio must not be liquidatable"
        );
    }

    /// validate_position does not panic when min_collateral_factor is 0 (key unset).
    #[test]
    fn validate_position_zero_factor_does_not_panic() {
        let w = setup();
        let market = make_market(&w);
        let price = 2_000 * FP;
        let props = PriceProps {
            min: price,
            max: price,
        };
        // Minimal collateral relative to position — would fail with a strict factor,
        // but factor=0 skips the min-collateral check entirely.
        let position = make_position(&w, 10_000 * FP, ONE_TOKEN, price);
        // Should not panic
        validate_position(&w.env, &w.ds, &position, &market, price, &props);
    }

    /// validate_position panics when factor is set high enough that the position's
    /// collateral falls below the required minimum.
    #[test]
    #[should_panic]
    fn validate_position_factor_enforced_when_set() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = make_market(&w);
        let price = 2_000 * FP;
        let props = PriceProps {
            min: price,
            max: price,
        };

        // 100% factor: collateral must be >= size_in_usd
        // Position: $10 000 size, 1 token @ $2 000 = $2 000 collateral → insufficient
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &(FP as u128),
        );
        let position = make_position(&w, 10_000 * FP, ONE_TOKEN, price);
        validate_position(&w.env, &w.ds, &position, &market, price, &props);
    }

    // ── Issue #137: differential tests against reference GMX formulas ─────────
    //
    // Each test pins a specific numeric result against a manually derived
    // reference value to catch formula drift.

    /// Reference (PositionUtils.sol): position PnL for full long close.
    ///   size_in_usd    = $2 000   (FP units)
    ///   size_in_tokens = 1 token  (TOKEN_PRECISION units)
    ///   exit_price     = $3 000
    ///   position_value = 1 token × $3 000 = $3 000
    ///   pnl            = $3 000 − $2 000 = $1 000
    #[test]
    fn differential_position_pnl_long_profit_matches_reference() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let exit_price = 3_000 * FP;
        let size_usd = 2_000 * FP;

        let pos = make_position(&w, size_usd, ONE_TOKEN, entry_price);
        let price_props = PriceProps {
            min: exit_price,
            max: exit_price,
        };

        let (pnl, uncapped) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);

        let expected_pnl = 1_000 * FP; // $1 000 profit
        assert_eq!(
            pnl, expected_pnl,
            "full close PnL must match reference: {pnl} != {expected_pnl}"
        );
        assert_eq!(
            uncapped, expected_pnl,
            "uncapped PnL must equal pnl (no capping at this level)"
        );
    }

    /// Reference: long loss when price falls below entry.
    ///   size_in_usd    = $4 000
    ///   size_in_tokens = 2 tokens  (at $2 000 entry)
    ///   exit_price     = $1 000
    ///   position_value = 2 × $1 000 = $2 000
    ///   pnl            = $2 000 − $4 000 = −$2 000
    #[test]
    fn differential_position_pnl_long_loss_matches_reference() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let exit_price = 1_000 * FP;
        let size_usd = 4_000 * FP;

        let pos = make_position(&w, size_usd, 2 * ONE_TOKEN, entry_price);
        let price_props = PriceProps {
            min: exit_price,
            max: exit_price,
        };

        let (pnl, _) = get_position_pnl_usd(&w.env, &pos, &price_props, size_usd);

        let expected_pnl = -2_000 * FP;
        assert_eq!(
            pnl, expected_pnl,
            "long loss PnL must match reference: {pnl} != {expected_pnl}"
        );
    }

    /// Reference: position opening fee.
    ///   size_delta_usd  = $1 000 (FP)
    ///   fee_factor      = 30 bps = 0.003 × FP
    ///   collateral_price = $2 000 (FP per whole token)
    ///   fee_usd         = $1 000 × 0.003 = $3
    ///   fee_tokens      = $3 / $2 000 per token × TOKEN_PRECISION
    ///                   = 3 * FP / (2_000 * FP) * TOKEN_PRECISION
    ///                   = 3 * TOKEN_PRECISION / 2_000 = 15_000 units (≈ 0.0015 tokens)
    #[test]
    fn differential_position_fee_matches_reference_formula() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        let fee_bps: i128 = 30;
        let fee_factor: i128 = fee_bps * FP / 10_000; // 30 bps in FP units
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );

        let collateral_price = 2_000 * FP;
        let size_delta_usd = 1_000 * FP;
        let market = make_market(&w);
        let position = make_position(&w, size_delta_usd, ONE_TOKEN * 10, collateral_price);

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            collateral_price,
            size_delta_usd,
            true,
        );

        // Reference: fee_usd = size_delta × fee_factor / FP = $3
        // fee_tokens = fee_usd × TOKEN_PRECISION / collateral_price
        //            = 3*FP × TOKEN_PRECISION / (2000*FP) = 3 * TOKEN_PRECISION / 2000 = 15_000
        let fee_usd = gmx_math::mul_div_wide(&w.env, size_delta_usd, fee_factor, FP);
        let expected_fee =
            gmx_math::mul_div_wide(&w.env, fee_usd, TOKEN_PRECISION, collateral_price);

        assert_eq!(
            fees.position_fee_amount, expected_fee,
            "position fee must match reference formula: got {}, expected {}",
            fees.position_fee_amount, expected_fee
        );
        assert_eq!(
            fees.position_fee_amount, 15_000,
            "known numeric: 30bps of $1000 / $2000 = 0.0015 tokens = 15_000 units"
        );
    }

    /// Reference: borrowing fee delta = (cum_now − cum_at_open) × size_in_tokens / FP
    ///   cum_now      = FP / 5   (20%)
    ///   cum_at_open  = 0
    ///   size_tokens  = 3 tokens
    ///   fee = 20% × 3 tokens = 0.6 tokens = 6_000_000 units
    #[test]
    fn differential_borrowing_fee_matches_reference_formula() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);

        let cum_factor: i128 = FP / 5; // 20%
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true),
            &(cum_factor as u128),
        );

        let size_tokens = 3 * ONE_TOKEN; // 3 tokens
        let size_usd = 6_000 * FP; // 3 tokens @ $2 000
        let market = make_market(&w);
        let mut position = make_position(&w, size_usd, size_tokens, 2_000 * FP);
        position.borrowing_factor = 0; // opened when cum_factor was 0

        let fees = get_position_fees(
            &w.env,
            &w.ds,
            &market,
            &position,
            2_000 * FP,
            size_usd,
            true,
        );

        // Reference: (FP/5 - 0) * 3 tokens / FP = 0.6 tokens = 6_000_000 units
        let expected_borrow = gmx_math::mul_div_wide(&w.env, cum_factor, size_tokens, FP);
        assert_eq!(
            fees.borrowing_fee_amount, expected_borrow,
            "borrowing fee must match reference: got {}, expected {}",
            fees.borrowing_fee_amount, expected_borrow
        );
        assert_eq!(
            fees.borrowing_fee_amount, 6_000_000,
            "known numeric: 20% of 3 tokens = 0.6 tokens = 6_000_000 units"
        );
    }

    /// Bug #2 — PnL must not mask a collateral shortfall in the min_collateral_factor check.
    ///
    /// Setup: size=$10 000, collateral=$500 gross, pending borrowing fees≈$420
    ///   → net_collateral = $500 - $420 = $80
    ///   → min_collateral_factor = 1% → min_required = $100
    ///   → net_collateral ($80) < min_required ($100) → LIQUIDATABLE
    ///
    /// The position also has +$100 unrealised PnL. Before this fix, the old code
    /// computed `remaining = $80 + $100 = $180 ≥ $100` and returned false (not
    /// liquidatable). The fixed code must return true.
    #[test]
    fn pnl_does_not_mask_collateral_shortfall_below_min_factor() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = make_market(&w);

        // 1% min collateral factor
        let min_factor = FP / 100;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &(min_factor as u128),
        );

        // Position: $10 000 notional, collateral_price = $2 000/token
        // We want collateral_usd ≈ $500 → collateral_amount ≈ 0.25 tokens
        let collateral_price = 2_000 * FP;
        let size_usd = 10_000 * FP;
        // 0.25 tokens × $2 000 = $500 gross collateral
        let collateral_tokens = ONE_TOKEN / 4;

        // Set a large cumulative borrowing factor so fees ≈ $420
        // We need: borrowing_fee_amount × collateral_price / TOKEN_PRECISION ≈ $420 * FP
        // borrowing_fee_amount = delta × size_in_tokens / FP
        // size_in_tokens = size_usd / entry_price × TOKEN_PRECISION = 10_000*FP / (2_000*FP) × TOKEN_PRECISION = 5 tokens
        // We want fee_amount_tokens such that fee_amount_tokens × 2_000*FP / TOKEN_PRECISION ≈ 420*FP
        // fee_amount_tokens ≈ 420*FP × TOKEN_PRECISION / (2_000*FP) = 210 * ONE_TOKEN / 100 = 2_100_000
        // delta × 5 tokens / FP = 2_100_000 → delta = 2_100_000 × FP / (5 × TOKEN_PRECISION)
        //                                            = 2_100_000 × FP / 50_000_000 = 42 * FP / 1_000
        let size_in_tokens = 5 * ONE_TOKEN; // 5 tokens
        let delta = 42 * FP / 1_000;        // cumulative borrowing factor delta
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true),
            &(delta as u128),
        );

        let mut position = make_position(&w, size_usd, collateral_tokens, collateral_price);
        position.size_in_tokens = size_in_tokens;
        position.borrowing_factor = 0; // opened at cum=0

        // Price advances 2% → unrealised PnL ≈ +$200 (should NOT save the position)
        let current_price = 2_040 * FP; // slight profit
        let price_props = PriceProps {
            min: current_price,
            max: current_price,
        };

        // Fixed: net_collateral < min_required → must be liquidatable
        assert!(
            is_liquidatable(&w.env, &w.ds, &position, &market, collateral_price, &price_props),
            "position with net_collateral below min_collateral_factor must be liquidatable even with positive PnL"
        );
    }

    /// Preservation: a position that is healthy on net collateral (after fees)
    /// must not be liquidatable regardless of PnL.
    #[test]
    fn healthy_net_collateral_not_liquidatable_regardless_of_pnl() {
        let w = setup();
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = make_market(&w);

        // 1% min collateral factor
        let min_factor = FP / 100;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::min_collateral_factor_key(&w.env, &w.market_tk),
            &(min_factor as u128),
        );

        let collateral_price = 2_000 * FP;
        let size_usd = 10_000 * FP;
        // Large collateral: 2 tokens = $4 000 gross; net well above $100 min_required
        let position = make_position(&w, size_usd, 2 * ONE_TOKEN, collateral_price);

        // Even with negative PnL (price drops slightly) — collateral is still healthy
        let lower_price = 1_900 * FP;
        let price_props = PriceProps {
            min: lower_price,
            max: lower_price,
        };

        assert!(
            !is_liquidatable(&w.env, &w.ds, &position, &market, collateral_price, &price_props),
            "well-collateralised position with healthy net collateral must not be liquidatable"
        );
    }
}
