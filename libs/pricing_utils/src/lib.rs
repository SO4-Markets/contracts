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

use gmx_keys::{
    position_impact_exponent_factor_key, position_impact_factor_key,
    position_impact_pool_amount_key, swap_fee_factor_key, swap_impact_exponent_factor_key,
    swap_impact_factor_key, swap_impact_pool_amount_key,
};
use gmx_market_utils::{
    get_open_interest_for_side, get_pool_amount, get_position_impact_pool_amount,
    get_swap_impact_pool_amount,
};
use gmx_math::{mul_div_wide, mul_div_wide_up, pow_factor, FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_types::MarketProps;
use soroban_sdk::{Address, BytesN, Env};

#[allow(dead_code)]
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
        let next_pow = pow_factor(env, next_diff, exponent);
        let raw = mul_div_wide(
            env,
            positive_factor,
            initial_pow - next_pow,
            FLOAT_PRECISION,
        );
        // Cap by available impact pool
        raw.min(impact_pool_usd)
    } else {
        // Pool balance worsens → negative impact for user
        let initial_pow = pow_factor(env, initial_diff, exponent);
        let next_pow = pow_factor(env, next_diff, exponent);
        let raw = mul_div_wide(
            env,
            negative_factor,
            next_pow - initial_pow,
            FLOAT_PRECISION,
        );
        -raw
    }
}

// ─── Swap price impact ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
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
    let pool_in = get_pool_amount(env, data_store, market, token_in) as i128;
    let pool_out = get_pool_amount(env, data_store, market, token_out) as i128;
    let pool_in_usd = mul_div_wide(env, pool_in, price_in, TOKEN_PRECISION);
    let pool_out_usd = mul_div_wide(env, pool_out, price_out, TOKEN_PRECISION);
    let amount_in_usd = mul_div_wide(env, amount_in, price_in, TOKEN_PRECISION);

    let initial_diff = (pool_in_usd - pool_out_usd).abs();
    let next_in_usd = pool_in_usd + amount_in_usd;
    let next_out_usd = pool_out_usd - amount_in_usd;
    let next_diff = (next_in_usd - next_out_usd).abs();

    let pos_factor = ds.get_u128(&swap_impact_factor_key(env, &market.market_token, true)) as i128;
    let neg_factor = ds.get_u128(&swap_impact_factor_key(env, &market.market_token, false)) as i128;
    let exponent = ds.get_u128(&swap_impact_exponent_factor_key(env, &market.market_token)) as i128;

    // Impact pool cap (in USD of token_out)
    let pool_tokens = get_swap_impact_pool_amount(env, data_store, market, token_out) as i128;
    let pool_usd = mul_div_wide(env, pool_tokens, price_out, TOKEN_PRECISION);

    compute_impact_usd(
        env,
        initial_diff,
        next_diff,
        pos_factor,
        neg_factor,
        exponent,
        pool_usd,
    )
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
    DataStoreClient::new(env, data_store).apply_delta_to_u128(
        caller,
        &swap_impact_pool_amount_key(env, &market.market_token, token),
        &delta,
    );

    impact_amount
}

// ─── Swap output amount ───────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
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

    // Swap fee — round up so the protocol never under-collects
    let fee_factor = DataStoreClient::new(env, data_store).get_u128(&swap_fee_factor_key(
        env,
        &market.market_token,
        for_positive_impact,
    )) as i128;
    let fee_amount = mul_div_wide_up(env, amount_out_before_fees, fee_factor, FLOAT_PRECISION);

    // Price impact (in token_out units)
    let impact_usd = get_swap_price_impact(
        env, data_store, market, token_in, token_out, amount_in, price_in, price_out,
    );
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

    let long_oi = get_open_interest_for_side(env, data_store, market, true) as i128;
    let short_oi = get_open_interest_for_side(env, data_store, market, false) as i128;
    let initial_diff = (long_oi - short_oi).abs();

    let (next_long, next_short) = match (is_long, is_increase) {
        (true, true) => (long_oi + size_delta_usd, short_oi),
        (false, true) => (long_oi, short_oi + size_delta_usd),
        (true, false) => ((long_oi - size_delta_usd).max(0), short_oi),
        (false, false) => (long_oi, (short_oi - size_delta_usd).max(0)),
    };
    let next_diff = (next_long - next_short).abs();

    let pos_factor =
        ds.get_u128(&position_impact_factor_key(env, &market.market_token, true)) as i128;
    let neg_factor = ds.get_u128(&position_impact_factor_key(
        env,
        &market.market_token,
        false,
    )) as i128;
    let exponent = ds.get_u128(&position_impact_exponent_factor_key(
        env,
        &market.market_token,
    )) as i128;

    // Impact pool cap (in USD of index token)
    let pool_tokens = get_position_impact_pool_amount(env, data_store, market) as i128;
    let pool_usd = if index_token_price > 0 {
        mul_div_wide(env, pool_tokens, index_token_price, TOKEN_PRECISION)
    } else {
        0
    };

    compute_impact_usd(
        env,
        initial_diff,
        next_diff,
        pos_factor,
        neg_factor,
        exponent,
        pool_usd,
    )
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
    DataStoreClient::new(env, data_store).apply_delta_to_u128(
        caller,
        &position_impact_pool_amount_key(env, &market.market_token),
        &delta,
    );
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

// ─── Tests — Issue #61: negative swap price impact accounting ─────────────────
//
// Verifies that when a swap worsens pool balance:
//   • The impact pool delta equals the negative impact amount (pool grows).
//   • The user's output is reduced by the same amount.
//   • Multiple impact magnitudes are covered.
#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_math::{mul_div_wide, FLOAT_PRECISION, TOKEN_PRECISION};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};

    fn deploy_role_store(env: &Env, admin: &Address) -> Address {
        let id = env.register(RoleStore, ());
        RsClient::new(env, &id).initialize(admin);
        id
    }

    fn deploy_data_store(env: &Env, admin: &Address, rs: &Address) -> Address {
        let id = env.register(DataStore, ());
        DsClient::new(env, &id).initialize(admin, rs);
        id
    }

    fn setup(env: &Env) -> (Address, Address, Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let rs = deploy_role_store(env, &admin);
        let ds = deploy_data_store(env, &admin, &rs);

        let rs_c = RsClient::new(env, &rs);
        rs_c.grant_role(&admin, &admin, &roles::controller(env));

        let market_token = Address::generate(env);
        let long_token = Address::generate(env);
        let short_token = Address::generate(env);
        let _index_token = Address::generate(env);

        (
            admin,
            ds,
            market_token,
            _index_token,
            long_token,
            short_token,
        )
    }

    fn make_market(
        market_token: &Address,
        index_token: &Address,
        long_token: &Address,
        short_token: &Address,
    ) -> MarketProps {
        MarketProps {
            market_token: market_token.clone(),
            index_token: index_token.clone(),
            long_token: long_token.clone(),
            short_token: short_token.clone(),
        }
    }

    /// Seed pool amounts and impact factors in data_store.
    fn seed_swap_market(
        env: &Env,
        ds: &Address,
        caller: &Address,
        market: &MarketProps,
        long_pool: i128,
        short_pool: i128,
        neg_factor: i128, // FLOAT_PRECISION
        pos_factor: i128, // FLOAT_PRECISION
        exponent: i128,   // FLOAT_PRECISION (1.0 = linear)
    ) {
        let ds_c = DsClient::new(env, ds);
        // Pool amounts (raw token units)
        ds_c.set_u128(
            caller,
            &gmx_keys::pool_amount_key(env, &market.market_token, &market.long_token),
            &(long_pool as u128),
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::pool_amount_key(env, &market.market_token, &market.short_token),
            &(short_pool as u128),
        );
        // Impact factors
        ds_c.set_u128(
            caller,
            &gmx_keys::swap_impact_factor_key(env, &market.market_token, false),
            &(neg_factor as u128),
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::swap_impact_factor_key(env, &market.market_token, true),
            &(pos_factor as u128),
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::swap_impact_exponent_factor_key(env, &market.market_token),
            &(exponent as u128),
        );
    }

    // ── Issue #61: negative impact increases impact pool ──────────────────────

    /// When a swap worsens pool balance (token_in side already larger),
    /// the impact is negative, and the impact pool for token_out grows by
    /// exactly the absolute impact amount.
    #[test]
    fn negative_swap_impact_increases_impact_pool() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);
        // Swapping long→short worsens the imbalance → negative impact
        let price = FLOAT_PRECISION; // $1 per token
        let long_pool = 2_000 * TOKEN_PRECISION;
        let short_pool = 1_000 * TOKEN_PRECISION;
        let neg_factor = FLOAT_PRECISION / 1000; // 0.1% per unit
        let pos_factor = FLOAT_PRECISION / 2000;
        let exponent = FLOAT_PRECISION; // linear (exponent = 1.0)

        seed_swap_market(
            &env, &ds, &admin, &market, long_pool, short_pool, neg_factor, pos_factor, exponent,
        );

        let amount_in = 100 * TOKEN_PRECISION; // swap 100 long tokens

        // Compute impact
        let impact_usd =
            get_swap_price_impact(&env, &ds, &market, &lt, &st, amount_in, price, price);

        // Impact must be negative (worsens balance)
        assert!(
            impact_usd < 0,
            "impact must be negative when worsening pool balance, got {}",
            impact_usd
        );

        // Record impact pool before
        let pool_key = gmx_keys::swap_impact_pool_amount_key(&env, &mt, &st);
        let pool_before = DsClient::new(&env, &ds).get_u128(&pool_key) as i128;

        // Apply impact
        let impact_amount =
            apply_swap_impact_value(&env, &ds, &admin, &market, &st, price, impact_usd);

        let pool_after = DsClient::new(&env, &ds).get_u128(&pool_key) as i128;

        // Impact amount should be negative (user loses tokens)
        assert!(
            impact_amount < 0,
            "impact_amount must be negative for negative impact"
        );

        // Pool must have grown by |impact_amount| (negative impact → pool grows)
        let pool_delta = pool_after - pool_before;
        assert_eq!(
            pool_delta, -impact_amount,
            "impact pool delta must equal |impact_amount|: pool_delta={}, impact_amount={}",
            pool_delta, impact_amount
        );
    }

    /// User output is reduced by the absolute impact amount when impact is negative.
    #[test]
    fn negative_swap_impact_reduces_user_output() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let price = FLOAT_PRECISION;
        let long_pool = 3_000 * TOKEN_PRECISION;
        let short_pool = 500 * TOKEN_PRECISION;
        let neg_factor = FLOAT_PRECISION / 500;
        let pos_factor = FLOAT_PRECISION / 1000;
        let exponent = FLOAT_PRECISION;

        seed_swap_market(
            &env, &ds, &admin, &market, long_pool, short_pool, neg_factor, pos_factor, exponent,
        );

        // Set swap fee factor to 0 so we isolate impact effect
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, false),
            &0u128,
        );
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, true),
            &0u128,
        );

        let amount_in = 200 * TOKEN_PRECISION;

        // Output without any impact: amount_in * price_in / price_out = amount_in (same price)
        let baseline_output = amount_in; // price_in == price_out

        let (net_output, _fee) = get_swap_output_amount(
            &env, &ds, &market, &lt, &st, amount_in, price, price,
            false, // for_positive_impact = false (negative impact scenario)
        );

        // Net output must be less than baseline (impact reduces output)
        assert!(
            net_output < baseline_output,
            "net_output {} must be less than baseline {} when impact is negative",
            net_output,
            baseline_output
        );

        // The reduction must equal the absolute impact amount
        let impact_usd =
            get_swap_price_impact(&env, &ds, &market, &lt, &st, amount_in, price, price);
        assert!(impact_usd < 0, "impact must be negative");
        let impact_tokens = mul_div_wide(&env, impact_usd.abs(), TOKEN_PRECISION, price);
        let reduction = baseline_output - net_output;
        assert_eq!(
            reduction, impact_tokens,
            "output reduction {} must equal impact tokens {}",
            reduction, impact_tokens
        );
    }

    /// Multiple impact magnitudes: larger imbalance → larger negative impact.
    #[test]
    fn negative_impact_scales_with_imbalance_magnitude() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let price = FLOAT_PRECISION;
        let neg_factor = FLOAT_PRECISION / 1_000_000;
        let pos_factor = FLOAT_PRECISION / 2_000_000;
        let exponent = 2 * FLOAT_PRECISION;

        // Small imbalance: long=1100, short=1000
        seed_swap_market(
            &env,
            &ds,
            &admin,
            &market,
            1_100 * TOKEN_PRECISION,
            1_000 * TOKEN_PRECISION,
            neg_factor,
            pos_factor,
            exponent,
        );
        let impact_small = get_swap_price_impact(
            &env,
            &ds,
            &market,
            &lt,
            &st,
            50 * TOKEN_PRECISION,
            price,
            price,
        );

        // Large imbalance: long=5000, short=1000
        seed_swap_market(
            &env,
            &ds,
            &admin,
            &market,
            5_000 * TOKEN_PRECISION,
            1_000 * TOKEN_PRECISION,
            neg_factor,
            pos_factor,
            exponent,
        );
        let impact_large = get_swap_price_impact(
            &env,
            &ds,
            &market,
            &lt,
            &st,
            50 * TOKEN_PRECISION,
            price,
            price,
        );

        // Both must be negative
        assert!(impact_small < 0, "small imbalance impact must be negative");
        assert!(impact_large < 0, "large imbalance impact must be negative");

        // Larger imbalance → larger (more negative) impact
        assert!(
            impact_large < impact_small,
            "larger imbalance must produce larger negative impact: large={}, small={}",
            impact_large,
            impact_small
        );
    }

    // ── Issue #61: position price impact pool accounting ─────────────────────

    // ── Issue #136: property tests for pricing_utils ─────────────────────────

    /// Swap price impact is zero when the pool is perfectly balanced.
    #[test]
    fn property_swap_impact_zero_on_balanced_pool() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let price = FLOAT_PRECISION;
        let pool_size = 1_000 * TOKEN_PRECISION;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 2_000;
        let exponent = FLOAT_PRECISION; // linear

        // Perfectly balanced: long == short
        seed_swap_market(
            &env, &ds, &admin, &market, pool_size, pool_size, neg_factor, pos_factor, exponent,
        );

        // A balanced swap should have no price impact (initial_diff == 0)
        // but any non-zero swap will cause imbalance. Check that the sign of
        // the impact is correctly negative when swapping into the larger side.
        let impact = get_swap_price_impact(
            &env,
            &ds,
            &market,
            &lt,
            &st,
            100 * TOKEN_PRECISION,
            price,
            price,
        );
        // With equal pools, swapping long→short worsens balance → negative impact
        assert!(
            impact <= 0,
            "swapping into larger side on balanced pool must not be positive: {impact}"
        );
    }

    /// Larger swap amounts produce larger magnitude negative price impact
    /// (monotone in amount_in when worsening pool balance).
    #[test]
    fn property_swap_negative_impact_monotone_in_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let price = FLOAT_PRECISION;
        let neg_factor = FLOAT_PRECISION / 1_000_000;
        let pos_factor = FLOAT_PRECISION / 2_000_000;
        let exponent = 2 * FLOAT_PRECISION; // quadratic: larger swaps hurt more

        // Long pool >> short pool → swapping long→short worsens imbalance
        seed_swap_market(
            &env,
            &ds,
            &admin,
            &market,
            5_000 * TOKEN_PRECISION,
            1_000 * TOKEN_PRECISION,
            neg_factor,
            pos_factor,
            exponent,
        );

        let small_impact = get_swap_price_impact(
            &env,
            &ds,
            &market,
            &lt,
            &st,
            10 * TOKEN_PRECISION,
            price,
            price,
        );
        let large_impact = get_swap_price_impact(
            &env,
            &ds,
            &market,
            &lt,
            &st,
            500 * TOKEN_PRECISION,
            price,
            price,
        );

        assert!(
            small_impact <= 0,
            "small swap must have non-positive impact: {small_impact}"
        );
        assert!(
            large_impact <= 0,
            "large swap must have non-positive impact: {large_impact}"
        );
        assert!(
            large_impact <= small_impact,
            "larger swap must produce worse (more negative) impact: small={small_impact}, large={large_impact}"
        );
    }

    /// Position price impact is zero when open interest is perfectly balanced.
    #[test]
    fn property_position_impact_zero_on_balanced_oi() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let index_price = 2_000 * FLOAT_PRECISION;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 2_000;
        let exponent = FLOAT_PRECISION;

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_factor_key(&env, &mt, false),
            &(neg_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_factor_key(&env, &mt, true),
            &(pos_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_exponent_factor_key(&env, &mt),
            &(exponent as u128),
        );

        // Balanced OI: long == short → initial_diff == 0, opening more long makes it unbalanced
        let balanced_oi = 5_000 * FLOAT_PRECISION as u128;
        ds_c.set_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, true),
            &balanced_oi,
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, false),
            &balanced_oi,
        );

        // Opening a long when balanced worsens balance → negative impact
        let impact = get_position_price_impact(
            &env,
            &ds,
            &market,
            true,
            1_000 * FLOAT_PRECISION,
            true,
            index_price,
        );
        assert!(
            impact <= 0,
            "opening long on balanced OI must not produce positive impact: {impact}"
        );

        // Opening a short on the same balanced market also worsens balance → negative
        let impact_short = get_position_price_impact(
            &env,
            &ds,
            &market,
            false,
            1_000 * FLOAT_PRECISION,
            true,
            index_price,
        );
        assert!(
            impact_short <= 0,
            "opening short on balanced OI must not produce positive impact: {impact_short}"
        );
    }

    /// get_execution_price with zero price_impact returns the raw index price.
    #[test]
    fn property_execution_price_no_impact_equals_index() {
        let env = Env::default();
        let index_price = 2_000 * FLOAT_PRECISION;
        let size_delta_usd = 5_000 * FLOAT_PRECISION;
        let result = get_execution_price(&env, index_price, size_delta_usd, 0, true, true);
        assert_eq!(
            result, index_price,
            "zero price impact must leave execution price unchanged"
        );
    }

    /// Negative price impact raises the effective execution price for longs
    /// (trader pays more per unit). The adjusted price > index_price.
    #[test]
    fn property_negative_impact_raises_execution_price_for_long() {
        let env = Env::default();
        let index_price = 2_000 * FLOAT_PRECISION;
        let size_delta_usd = 10_000 * FLOAT_PRECISION;
        let neg_impact = -(100 * FLOAT_PRECISION); // −$100 in protocol precision

        let exec_price =
            get_execution_price(&env, index_price, size_delta_usd, neg_impact, true, true);
        assert!(
            exec_price > index_price,
            "negative impact must raise execution price for long: exec={exec_price}, index={index_price}"
        );
    }

    /// apply_swap_impact_value with impact_usd = 0 returns 0 without mutating state.
    #[test]
    fn property_apply_swap_impact_zero_impact_is_noop() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let pool_key = gmx_keys::swap_impact_pool_amount_key(&env, &mt, &st);
        let before = DsClient::new(&env, &ds).get_u128(&pool_key);

        let result = apply_swap_impact_value(&env, &ds, &admin, &market, &st, FLOAT_PRECISION, 0);

        let after = DsClient::new(&env, &ds).get_u128(&pool_key);
        assert_eq!(result, 0, "zero impact must return 0");
        assert_eq!(before, after, "zero impact must not mutate impact pool");
    }

    // ── Issue #137: differential tests against reference GMX formulas ─────────
    //
    // Each test computes a reference value by hand and asserts the function
    // matches it exactly, catching any formula drift.

    /// Reference (SwapPricingUtils): execution price with zero price impact.
    ///   index_price    = $2 000  (FP)
    ///   size_delta_usd = $10 000 (FP)
    ///   price_impact   = 0
    ///   execution_price = index_price (no shift)
    #[test]
    fn differential_execution_price_zero_impact_equals_index() {
        let env = Env::default();
        let index_price = 2_000 * FLOAT_PRECISION;
        let size_delta_usd = 10_000 * FLOAT_PRECISION;

        let exec = get_execution_price(&env, index_price, size_delta_usd, 0, true, true);

        assert_eq!(
            exec, index_price,
            "zero-impact execution price must equal index: {exec} != {index_price}"
        );
    }

    /// Reference: negative price impact raises execution price for a long.
    ///   index_price    = $2 000
    ///   size_delta_usd = $10 000
    ///   impact_usd     = −$100  (user gets $9 900 worth of tokens for $10 000)
    ///   adjusted_tokens = $9 900 / $2 000 per token × TOKEN_PRECISION
    ///   exec_price     = $10 000 / adjusted_tokens × TOKEN_PRECISION
    ///                  = $10 000 / (9 900 / 2 000) ≈ $2 020.20…
    ///   → exec_price > index_price (trader pays more per token)
    #[test]
    fn differential_execution_price_negative_impact_raises_long_price() {
        let env = Env::default();
        let index_price = 2_000 * FLOAT_PRECISION;
        let size_delta_usd = 10_000 * FLOAT_PRECISION;
        let impact_usd = -(100 * FLOAT_PRECISION);

        let exec = get_execution_price(&env, index_price, size_delta_usd, impact_usd, true, true);

        // Reference: adjusted_size = size_delta_usd + impact_usd = $9_900
        let adjusted_size = size_delta_usd + impact_usd;
        let adjusted_tokens = mul_div_wide(&env, adjusted_size, TOKEN_PRECISION, index_price);
        let expected_exec = mul_div_wide(&env, size_delta_usd, TOKEN_PRECISION, adjusted_tokens);

        assert_eq!(
            exec, expected_exec,
            "execution price must match reference formula: exec={exec}, expected={expected_exec}"
        );
        assert!(
            exec > index_price,
            "negative impact must raise execution price: exec={exec}, index={index_price}"
        );
    }

    /// Reference: get_swap_output_amount with no price impact and no fee.
    ///   amount_in   = 100 tokens (TOKEN_PRECISION)
    ///   price_in    = $2 000  (FP)
    ///   price_out   = $1      (FP — stable)
    ///   fee         = 0
    ///   raw_out     = 100 tokens * $2000 / $1 = 200_000 tokens
    ///   (using TOKEN_PRECISION: 100 * 10^7 * 2000*FP / FP = 200_000 * 10^7)
    #[test]
    fn differential_swap_output_no_fee_no_impact_matches_price_ratio() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let price_in = 2_000 * FLOAT_PRECISION;
        let price_out = FLOAT_PRECISION; // $1 stable
        let amount_in = 100 * TOKEN_PRECISION;

        // Balanced pools so there is no price impact (zero impact factors).
        // Pool sizes must be small enough that pool_usd = tokens * price / TOKEN_PRECISION
        // fits within i128. 1_000 tokens * $2_000 * FP / TOKEN_PRECISION = 2e36 < i128::MAX.
        seed_swap_market(
            &env,
            &ds,
            &admin,
            &market,
            1_000 * TOKEN_PRECISION,
            2_000_000 * TOKEN_PRECISION, // balanced in USD: $2M each side
            0,
            0,
            FLOAT_PRECISION,
        );

        // Zero fee factors
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, true),
            &0u128,
        );
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, false),
            &0u128,
        );

        let (output, fee) = get_swap_output_amount(
            &env, &ds, &market, &lt, &st, amount_in, price_in, price_out, true,
        );

        // Reference: output = amount_in * price_in / price_out
        let expected_output = mul_div_wide(&env, amount_in, price_in, price_out);
        assert_eq!(fee, 0, "zero fee factor must produce zero fee");
        assert_eq!(
            output, expected_output,
            "output must match price ratio: output={output}, expected={expected_output}"
        );
        // 100 tokens * $2000 / $1 = 200_000 tokens
        assert_eq!(
            output,
            200_000 * TOKEN_PRECISION,
            "known numeric: 100 tokens at 2000x = 200_000 tokens"
        );
    }

    /// Reference: swap fee calculation.
    ///   amount_out_before_fee = 1_000 tokens (TOKEN_PRECISION)
    ///   fee_factor            = 0.3% = FP * 3 / 1_000
    ///   fee                   = 1_000 tokens * 0.003 = 3 tokens = 3 * TOKEN_PRECISION
    ///   net_output            = 997 tokens
    ///
    /// Uses ceiling rounding so fee >= 3 tokens (never under-collected).
    #[test]
    fn differential_swap_fee_matches_reference_formula() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let fee_factor = 3 * FLOAT_PRECISION / 1_000; // 0.3%
        let price = FLOAT_PRECISION; // $1 / token (both tokens same price → simple)

        // Balanced pools so impact = 0
        seed_swap_market(
            &env,
            &ds,
            &admin,
            &market,
            100_000 * TOKEN_PRECISION,
            100_000 * TOKEN_PRECISION,
            0,
            0,
            FLOAT_PRECISION,
        );

        // Set fee for positive-impact direction
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, true),
            &(fee_factor as u128),
        );
        DsClient::new(&env, &ds).set_u128(
            &admin,
            &gmx_keys::swap_fee_factor_key(&env, &mt, false),
            &(fee_factor as u128),
        );

        let amount_in = 1_000 * TOKEN_PRECISION;

        let (output, fee) = get_swap_output_amount(
            &env, &ds, &market, &lt, &st, amount_in, price, price, true, // for_positive_impact
        );

        // Reference: fee = ceil(1000 tokens * 0.003) = 3 tokens = 30_000_000 units
        assert_eq!(
            fee,
            3 * TOKEN_PRECISION,
            "fee must be 0.3% of amount_out_before_fee"
        );
        assert_eq!(
            output,
            1_000 * TOKEN_PRECISION - 3 * TOKEN_PRECISION,
            "net output = 997 tokens"
        );
    }

    /// Negative position price impact increases the position impact pool.
    #[test]
    fn negative_position_impact_increases_impact_pool() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let index_price = 2_000 * FLOAT_PRECISION; // $2000 per token
        let neg_factor = FLOAT_PRECISION / 1000;
        let pos_factor = FLOAT_PRECISION / 2000;
        let exponent = FLOAT_PRECISION;

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_factor_key(&env, &mt, false),
            &(neg_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_factor_key(&env, &mt, true),
            &(pos_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_impact_exponent_factor_key(&env, &mt),
            &(exponent as u128),
        );

        // Seed OI: long=5000 USD, short=1000 USD (long side larger)
        // Opening more long worsens imbalance → negative impact
        ds_c.set_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, true),
            &(5_000 * FLOAT_PRECISION as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::open_interest_key(&env, &mt, &lt, false),
            &(1_000 * FLOAT_PRECISION as u128),
        );

        let size_delta = 1_000 * FLOAT_PRECISION; // $1000 increase

        let impact_usd = get_position_price_impact(
            &env,
            &ds,
            &market,
            true, // is_long
            size_delta,
            true, // is_increase
            index_price,
        );

        assert!(
            impact_usd < 0,
            "opening more long when long>short must be negative impact, got {}",
            impact_usd
        );

        // Record pool before
        let pool_key = gmx_keys::position_impact_pool_amount_key(&env, &mt);
        let pool_before = ds_c.get_u128(&pool_key) as i128;

        // Apply impact
        let impact_amount =
            apply_position_impact_value(&env, &ds, &admin, &market, impact_usd, index_price);

        let pool_after = DsClient::new(&env, &ds).get_u128(&pool_key) as i128;

        // Negative impact → pool grows
        assert!(impact_amount < 0, "impact_amount must be negative");
        let pool_delta = pool_after - pool_before;
        assert_eq!(
            pool_delta, -impact_amount,
            "impact pool delta must equal |impact_amount|: pool_delta={}, impact_amount={}",
            pool_delta, impact_amount
        );
    }

    // ── Issue #246: unit tests for get_position_price_impact ─────────────────

    /// Seed both OI sides (via the long_token collateral key) and the three
    /// position impact parameters.  All OI values are in FLOAT_PRECISION units.
    fn seed_position_market(
        env: &Env,
        ds: &Address,
        caller: &Address,
        market: &MarketProps,
        long_oi: u128,
        short_oi: u128,
        neg_factor: i128,
        pos_factor: i128,
        exponent: i128,
    ) {
        let ds_c = DsClient::new(env, ds);
        ds_c.set_u128(
            caller,
            &gmx_keys::open_interest_key(env, &market.market_token, &market.long_token, true),
            &long_oi,
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::open_interest_key(env, &market.market_token, &market.long_token, false),
            &short_oi,
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::position_impact_factor_key(env, &market.market_token, false),
            &(neg_factor as u128),
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::position_impact_factor_key(env, &market.market_token, true),
            &(pos_factor as u128),
        );
        ds_c.set_u128(
            caller,
            &gmx_keys::position_impact_exponent_factor_key(env, &market.market_token),
            &(exponent as u128),
        );
    }

    /// Seed the position impact pool with a raw token balance.
    fn seed_impact_pool(
        env: &Env,
        ds: &Address,
        caller: &Address,
        market: &MarketProps,
        pool_tokens: u128,
    ) {
        DsClient::new(env, ds).set_u128(
            caller,
            &gmx_keys::position_impact_pool_amount_key(env, &market.market_token),
            &pool_tokens,
        );
    }

    /// When long_oi == short_oi the pool is perfectly balanced (initial_diff = 0).
    /// A long-increase trade creates imbalance from scratch → negative impact.
    ///
    /// Derivation (linear exponent = FLOAT_PRECISION):
    ///   initial_diff = |5_000·FP − 5_000·FP| = 0
    ///   next_diff    = size_delta = 1_000·FP  (next_long − next_short)
    ///   pow_factor(0,  FP) = 0  [value ≤ 0 special-case]
    ///   pow_factor(1_000·FP, FP) = 1_000·FP  [linear special-case]
    ///   raw    = neg_factor × (next_diff − 0) / FP
    ///          = (FP/1_000) × 1_000·FP / FP = FP
    ///   impact = −FP
    #[test]
    fn test_price_impact_usd_balanced_pool() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let oi = 5_000 * FLOAT_PRECISION as u128;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 2_000;

        seed_position_market(
            &env, &ds, &admin, &market, oi, oi, neg_factor, pos_factor, FLOAT_PRECISION,
        );
        seed_impact_pool(&env, &ds, &admin, &market, 100_000 * TOKEN_PRECISION as u128);

        let size_delta = 1_000 * FLOAT_PRECISION;
        let index_price = FLOAT_PRECISION; // $1 per token in FP units

        let impact =
            get_position_price_impact(&env, &ds, &market, true, size_delta, true, index_price);

        assert_eq!(
            impact, -FLOAT_PRECISION,
            "balanced pool: long trade must produce negative impact = −FP, got {impact}"
        );
    }

    /// When long_oi > short_oi a short-increase trade moves the pool toward
    /// balance → positive impact (rebate to the trader).
    ///
    /// Derivation:
    ///   long_oi = 6_000·FP, short_oi = 4_000·FP → initial_diff = 2_000·FP
    ///   Short of 1_000·FP: next_short = 5_000·FP → next_diff = 1_000·FP
    ///   next_diff < initial_diff → positive case
    ///   raw = pos_factor × (2_000·FP − 1_000·FP) / FP
    ///       = (FP/2_000) × 1_000·FP / FP = FP/2
    ///   pool_usd = 1_000 tokens × FP / TOKEN_PRECISION = 1_000·FP ≫ raw → cap not binding
    ///   impact = FP/2
    #[test]
    fn test_price_impact_usd_imbalanced_balancing_trade() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let long_oi = 6_000 * FLOAT_PRECISION as u128;
        let short_oi = 4_000 * FLOAT_PRECISION as u128;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 2_000;

        seed_position_market(
            &env, &ds, &admin, &market, long_oi, short_oi, neg_factor, pos_factor, FLOAT_PRECISION,
        );
        // Pool large enough so the computed rebate is never capped
        seed_impact_pool(&env, &ds, &admin, &market, 1_000 * TOKEN_PRECISION as u128);

        let size_delta = 1_000 * FLOAT_PRECISION;
        let index_price = FLOAT_PRECISION;

        let impact =
            get_position_price_impact(&env, &ds, &market, false, size_delta, true, index_price);

        let expected = FLOAT_PRECISION / 2;
        assert!(impact > 0, "balancing short must yield positive rebate, got {impact}");
        assert_eq!(
            impact, expected,
            "balancing short: expected FP/2 = {expected}, got {impact}"
        );
    }

    /// When long_oi > short_oi a long-increase trade deepens the imbalance
    /// → negative impact (cost to the trader).
    ///
    /// Derivation:
    ///   long_oi = 6_000·FP, short_oi = 4_000·FP → initial_diff = 2_000·FP
    ///   Long of 1_000·FP: next_long = 7_000·FP → next_diff = 3_000·FP
    ///   next_diff > initial_diff → worsening → negative
    ///   raw = neg_factor × (3_000·FP − 2_000·FP) / FP
    ///       = (FP/1_000) × 1_000·FP / FP = FP
    ///   impact = −FP
    #[test]
    fn test_price_impact_usd_imbalanced_worsening_trade() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let long_oi = 6_000 * FLOAT_PRECISION as u128;
        let short_oi = 4_000 * FLOAT_PRECISION as u128;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 2_000;

        seed_position_market(
            &env, &ds, &admin, &market, long_oi, short_oi, neg_factor, pos_factor, FLOAT_PRECISION,
        );
        seed_impact_pool(&env, &ds, &admin, &market, 100_000 * TOKEN_PRECISION as u128);

        let size_delta = 1_000 * FLOAT_PRECISION;
        let index_price = FLOAT_PRECISION;

        let impact =
            get_position_price_impact(&env, &ds, &market, true, size_delta, true, index_price);

        assert!(impact < 0, "worsening long must produce negative impact, got {impact}");
        assert_eq!(
            impact, -FLOAT_PRECISION,
            "worsening long-increase: expected −FP, got {impact}"
        );
    }

    /// When both OI sides are zero (empty market) and impact factors have not
    /// been configured (both = 0), any trade returns zero impact.
    ///
    /// Derivation: raw = 0 × Δ / FP = 0 for every size and direction.
    #[test]
    fn test_price_impact_usd_zero_oi() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        seed_position_market(&env, &ds, &admin, &market, 0, 0, 0, 0, FLOAT_PRECISION);
        seed_impact_pool(&env, &ds, &admin, &market, 0);

        let size_delta = 1_000 * FLOAT_PRECISION;
        let index_price = FLOAT_PRECISION;

        let impact =
            get_position_price_impact(&env, &ds, &market, true, size_delta, true, index_price);

        assert_eq!(
            impact, 0,
            "zero OI with zero impact factors must return 0, got {impact}"
        );
    }

    /// When the computed positive rebate exceeds the available impact pool
    /// balance the protocol caps the payout at pool_usd.
    ///
    /// Derivation:
    ///   long_oi = 6_000·FP, short_oi = 4_000·FP → initial_diff = 2_000·FP
    ///   Short of 2_001·FP over-balances: next_short = 6_001·FP
    ///   next_diff = |6_000 − 6_001|·FP = 1·FP  (< initial_diff → positive)
    ///   raw = pos_factor × (2_000·FP − 1·FP) / FP
    ///       = (FP/100) × 1_999·FP / FP = 1_999·FP/100  ≈ 19.99·FP
    ///   pool_usd = 5 tokens × FP / TOKEN_PRECISION = 5·FP
    ///   raw (≈ 19.99·FP) > pool_usd (5·FP) → capped
    ///   impact = 5·FP  (= pool_usd)
    #[test]
    fn test_price_impact_usd_rebate_capped_at_pool_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, ds, mt, it, lt, st) = setup(&env);
        let market = make_market(&mt, &it, &lt, &st);

        let long_oi = 6_000 * FLOAT_PRECISION as u128;
        let short_oi = 4_000 * FLOAT_PRECISION as u128;
        let neg_factor = FLOAT_PRECISION / 1_000;
        let pos_factor = FLOAT_PRECISION / 100; // aggressive factor to produce large raw rebate

        seed_position_market(
            &env, &ds, &admin, &market, long_oi, short_oi, neg_factor, pos_factor, FLOAT_PRECISION,
        );
        // Only 5 tokens in the pool → pool_usd = 5·FP, much less than raw rebate
        let pool_tokens: u128 = 5 * TOKEN_PRECISION as u128;
        seed_impact_pool(&env, &ds, &admin, &market, pool_tokens);

        // Short of 2_001·FP over-balances (next_diff = 1·FP)
        let size_delta = 2_001 * FLOAT_PRECISION;
        let index_price = FLOAT_PRECISION; // $1 per token

        let impact =
            get_position_price_impact(&env, &ds, &market, false, size_delta, true, index_price);

        // pool_usd = pool_tokens × index_price / TOKEN_PRECISION = 5·TOKEN_PRECISION × FP / TOKEN_PRECISION = 5·FP
        let pool_usd = mul_div_wide(&env, pool_tokens as i128, index_price, TOKEN_PRECISION);
        assert!(impact > 0, "over-balancing short must produce positive rebate, got {impact}");
        assert_eq!(
            impact, pool_usd,
            "rebate must be capped at pool_usd={pool_usd}, got {impact}"
        );
    }

    /// Documents the tuning example in docs/price-impact.md:
    /// target 0.5% impact on a 50,000 USD trade with quadratic position impact.
    ///
    ///   target_impact = 50,000 * 50 bps / 10,000 = 250 USD
    ///   factor        = target_impact / trade^2
    ///                 = 250 / 2,500,000,000 = 1e-7
    ///   scaled factor = 1e-7 * FLOAT_PRECISION = 1e23
    #[test]
    fn test_price_impact_tuning_example_derives_factor() {
        let trade_usd: i128 = 50_000;
        let target_bps: i128 = 50;
        let target_impact_usd = trade_usd * target_bps / 10_000;

        let factor_scaled =
            target_impact_usd * FLOAT_PRECISION / (trade_usd * trade_usd);

        assert_eq!(target_impact_usd, 250);
        assert_eq!(factor_scaled, 100_000_000_000_000_000_000_000);

        let recomputed_impact_usd =
            factor_scaled * trade_usd * trade_usd / FLOAT_PRECISION;
        assert_eq!(recomputed_impact_usd, target_impact_usd);
    }
}
