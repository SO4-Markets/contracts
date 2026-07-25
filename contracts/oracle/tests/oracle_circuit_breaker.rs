// Integration tests for oracle price-manipulation circuit breaker — issue #265.
//
// The circuit breaker trips when a new price deviates from the previously
// stored price by more than `circuit_breaker_factor_key` basis points.
// When tripped, `is_market_paused_key` is set to true for every market whose
// index / long / short token matches the submitted token.
//
// Verifies:
//   - 0% deviation: market remains unpaused
//   - Deviation exactly equal to the threshold (1500 bps = 15%): NOT tripped
//   - Deviation one bps above the threshold (1501 bps): tripped → market paused
//   - Multiple markets: only the market whose token matches is paused

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::roles;
use gmx_math::FLOAT_PRECISION;
use gmx_types::TokenPrice;
use oracle::{Oracle, OracleClient as OClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Bytes, Env, Vec,
};

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    ds: Address,
    oracle: Address,
    market: Address,
    index_tk: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle_addr = env.register(Oracle, ());
    let passphrase = Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    // Oracle must hold CONTROLLER so it can call ds.set_bool when tripping the breaker
    rs_c.grant_role(&admin, &oracle_addr, &roles::controller(&env));

    // Register a fake market and its index token in data_store
    let market = Address::generate(&env);
    let index_tk = Address::generate(&env);
    let ds_c = DsClient::new(&env, &ds);

    // Add market to the global market list
    ds_c.add_address_to_set(&admin, &gmx_keys::market_list_key(&env), &market);
    // Map market → index token (the oracle checks index_token_key, long_token_key, short_token_key)
    ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market), &index_tk);

    World { env, admin, keeper, ds, oracle: oracle_addr, market, index_tk }
}

fn set_initial_price(w: &World, price: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(&w.env, [TokenPrice { token: w.index_tk.clone(), min: price, max: price }]),
    );
}

fn set_price(w: &World, price: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(&w.env, [TokenPrice { token: w.index_tk.clone(), min: price, max: price }]),
    );
}

fn is_paused(w: &World) -> bool {
    DsClient::new(&w.env, &w.ds)
        .get_bool(&gmx_keys::is_market_paused_key(&w.env, &w.market))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// No prior price → circuit breaker has nothing to compare against, no trip.
#[test]
fn no_prior_price_does_not_trip_breaker() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market),
        &(1500u128), // 15% threshold in bps
    );

    set_price(&w, 2_000 * fp);
    assert!(!is_paused(&w), "first submission must never trip the circuit breaker");
}

/// Zero deviation (same price twice): never trips regardless of threshold.
#[test]
fn zero_deviation_does_not_trip_breaker() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market),
        &(1500u128),
    );

    set_initial_price(&w, 2_000 * fp);
    // Advance ledger so the temporary price survives into the next call
    w.env.ledger().set_sequence_number(10);
    set_price(&w, 2_000 * fp); // 0% deviation

    assert!(!is_paused(&w), "zero deviation must not trip the circuit breaker");
}

/// Deviation exactly at the threshold (1500 bps = 15%): NOT tripped.
/// check_circuit_breaker uses strict `>`, not `>=`, so equality is safe.
#[test]
fn deviation_at_threshold_does_not_trip() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let threshold_bps: u128 = 1500; // 15%
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market),
        &threshold_bps,
    );

    let base = 10_000 * fp; // use a large base to get exact bps arithmetic
    set_initial_price(&w, base);
    w.env.ledger().set_sequence_number(10);

    // 15% move: new = base * 11500 / 10000 → deviation_bps = 1500 exactly
    let new_price = base + (base * 1500) / 10_000;
    set_price(&w, new_price);

    assert!(!is_paused(&w), "deviation equal to threshold must NOT trip the circuit breaker");
}

/// Deviation one bps above the threshold (1501 bps): MUST trip.
#[test]
fn deviation_above_threshold_trips_breaker() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let threshold_bps: u128 = 1500; // 15%
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market),
        &threshold_bps,
    );

    let base = 10_000 * fp;
    set_initial_price(&w, base);
    w.env.ledger().set_sequence_number(10);

    // 1501 bps move: new = base * 11501 / 10000
    let new_price = base + (base * 1501) / 10_000;
    set_price(&w, new_price);

    assert!(is_paused(&w), "deviation above threshold (1501 bps) must trip the circuit breaker");
}

/// Price drop (negative deviation) of >15% also trips the breaker.
#[test]
fn downward_deviation_above_threshold_trips_breaker() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market),
        &1500u128,
    );

    let base = 10_000 * fp;
    set_initial_price(&w, base);
    w.env.ledger().set_sequence_number(10);

    // 20% drop
    let new_price = base - (base * 2000) / 10_000;
    set_price(&w, new_price);

    assert!(is_paused(&w), "downward price manipulation beyond threshold must pause the market");
}

/// When threshold is 0 (not configured), the circuit breaker is disabled.
#[test]
fn zero_threshold_disables_circuit_breaker() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    // threshold not set → defaults to 0 → disabled

    let base = 2_000 * fp;
    set_initial_price(&w, base);
    w.env.ledger().set_sequence_number(10);

    // Extreme move: 50% → would trip if threshold were set
    let new_price = base * 3;
    set_price(&w, new_price);

    assert!(!is_paused(&w), "zero threshold must disable the circuit breaker entirely");
}

/// Only the market whose token matches is paused; an unrelated market is unaffected.
#[test]
fn only_matching_market_is_paused() {
    let w = setup();
    let fp = FLOAT_PRECISION;

    // Register a second market with a DIFFERENT index token
    let other_market = Address::generate(&w.env);
    let other_index_tk = Address::generate(&w.env);
    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.add_address_to_set(&w.admin, &gmx_keys::market_list_key(&w.env), &other_market);
    ds_c.set_address(
        &w.admin,
        &gmx_keys::market_index_token_key(&w.env, &other_market),
        &other_index_tk,
    );

    // Set threshold for both markets
    ds_c.set_u128(&w.admin, &gmx_keys::circuit_breaker_factor_key(&w.env, &w.market), &1500u128);
    ds_c.set_u128(&w.admin, &gmx_keys::circuit_breaker_factor_key(&w.env, &other_market), &1500u128);

    let base = 10_000 * fp;
    set_initial_price(&w, base); // sets price for w.index_tk
    w.env.ledger().set_sequence_number(10);

    let new_price = base + (base * 1501) / 10_000;
    set_price(&w, new_price); // manipulates w.index_tk → should pause w.market only

    assert!(is_paused(&w), "matching market must be paused");
    assert!(
        !ds_c.get_bool(&gmx_keys::is_market_paused_key(&w.env, &other_market)),
        "unrelated market must remain unpaused"
    );
}
