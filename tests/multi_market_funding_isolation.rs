//! Integration test: funding state for two independent markets does not
//! bleed across markets — issue #266.
//!
//! Scenario: ETH/USD is long-heavy (80% long OI), BTC/USD is balanced
//! (50/50 OI). After advancing time and calling `update_funding_state` for
//! both markets, ETH/USD accumulates significant funding while BTC/USD
//! stays near zero, and each market's data_store keys remain independent.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{funding_amount_per_size_key, open_interest_key, roles};
use gmx_math::FLOAT_PRECISION;
use gmx_market_utils::update_funding_state;
use gmx_types::MarketProps;
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

/// Seed a market's funding config. `saved_factor` is the current (already
/// established) per-second funding rate — a long-heavy market is given a
/// realistic nonzero rate directly (matching an already-running market that
/// has ramped up over many prior ticks); a balanced market is left at 0.
fn setup_funding_params(env: &Env, ds: &Address, admin: &Address, market: &Address, saved_factor: i128) {
    let ds_c = DsClient::new(env, ds);
    let fp = FLOAT_PRECISION as u128;
    ds_c.set_i128(admin, &gmx_keys::saved_funding_factor_per_second_key(env, market), &saved_factor);
    ds_c.set_u128(admin, &gmx_keys::funding_updated_at_key(env, market), &0u128);
    ds_c.set_u128_instance(admin, &gmx_keys::funding_factor_key(env, market), &fp);
    ds_c.set_u128_instance(admin, &gmx_keys::funding_exponent_factor_key(env, market), &fp);
    let ramp: u128 = 1_000u128 * fp;
    ds_c.set_u128_instance(admin, &gmx_keys::funding_increase_factor_per_second_key(env, market), &ramp);
    ds_c.set_u128_instance(admin, &gmx_keys::funding_decrease_factor_per_second_key(env, market), &ramp);
    let bound: i128 = 1_000_000_i128 * FLOAT_PRECISION;
    ds_c.set_i128_instance(admin, &gmx_keys::min_funding_factor_per_second_key(env, market), &(-bound));
    ds_c.set_i128_instance(admin, &gmx_keys::max_funding_factor_per_second_key(env, market), &bound);
}

#[test]
fn eth_funding_accumulates_while_btc_funding_stays_near_zero() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let rs = env.register(RoleStore, ());
    RsClient::new(&env, &rs).initialize(&admin);
    RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::controller(&env));

    let ds = env.register(DataStore, ());
    let ds_c = DsClient::new(&env, &ds);
    ds_c.initialize(&admin, &rs);

    // ── Two independent markets ──────────────────────────────────────────────
    let eth_market = Address::generate(&env);
    let eth_long = Address::generate(&env);
    let eth_short = Address::generate(&env);
    let eth_index = Address::generate(&env);

    let btc_market = Address::generate(&env);
    let btc_long = Address::generate(&env);
    let btc_short = Address::generate(&env);
    let btc_index = Address::generate(&env);

    // ETH/USD already has an established, meaningfully large per-second funding
    // rate (as a long-heavy market would after ramping up over many prior ticks);
    // BTC/USD (balanced) starts at 0.
    setup_funding_params(&env, &ds, &admin, &eth_market, FLOAT_PRECISION / 10);
    setup_funding_params(&env, &ds, &admin, &btc_market, 0);

    // ETH/USD: long-heavy (80% long OI, 20% short OI out of 1,000,000 USD notional).
    // OI is stored as FLOAT_PRECISION-scaled USD, matching production usage.
    ds_c.apply_delta_to_u128(&admin, &open_interest_key(&env, &eth_market, &eth_long, true), &(800_000_i128 * FLOAT_PRECISION));
    ds_c.apply_delta_to_u128(&admin, &open_interest_key(&env, &eth_market, &eth_short, false), &(200_000_i128 * FLOAT_PRECISION));

    // BTC/USD: balanced (50/50 OI).
    ds_c.apply_delta_to_u128(&admin, &open_interest_key(&env, &btc_market, &btc_long, true), &(500_000_i128 * FLOAT_PRECISION));
    ds_c.apply_delta_to_u128(&admin, &open_interest_key(&env, &btc_market, &btc_short, false), &(500_000_i128 * FLOAT_PRECISION));

    let eth_props = MarketProps::new(&eth_market, &eth_index, &eth_long, &eth_short);
    let btc_props = MarketProps::new(&btc_market, &btc_index, &btc_long, &btc_short);

    // ── Advance 100 ledgers, update funding state for both markets ─────────────
    let current_time: u64 = 100;
    update_funding_state(&env, &ds, &admin, &eth_props, 0, 0, current_time);
    update_funding_state(&env, &ds, &admin, &btc_props, 0, 0, current_time);

    let eth_long_fnd = ds_c.get_i128(&funding_amount_per_size_key(&env, &eth_market, &eth_long, true));
    let btc_long_fnd = ds_c.get_i128(&funding_amount_per_size_key(&env, &btc_market, &btc_long, true));

    assert!(
        eth_long_fnd.abs() > 0,
        "long-heavy ETH/USD must accumulate a nonzero funding_amount_per_size, got {eth_long_fnd}"
    );
    assert_eq!(
        btc_long_fnd, 0,
        "balanced BTC/USD must remain at ~0 funding_amount_per_size, got {btc_long_fnd}"
    );

    // ── No cross-market state leakage: each market's keys are independently scoped ──
    let eth_oi = ds_c.get_u128(&open_interest_key(&env, &eth_market, &eth_long, true));
    let btc_oi = ds_c.get_u128(&open_interest_key(&env, &btc_market, &btc_long, true));
    assert_eq!(eth_oi, (800_000_i128 * FLOAT_PRECISION) as u128, "ETH long OI must be untouched by BTC's update");
    assert_eq!(btc_oi, (500_000_i128 * FLOAT_PRECISION) as u128, "BTC long OI must be untouched by ETH's update");

    // Raw storage check: BTC's own long-funding key must not equal ETH's key
    // (proves keys are market-scoped, not colliding on any shared bucket).
    assert_ne!(
        funding_amount_per_size_key(&env, &eth_market, &eth_long, true),
        funding_amount_per_size_key(&env, &btc_market, &btc_long, true),
        "ETH and BTC markets must never share a funding storage key"
    );
}
