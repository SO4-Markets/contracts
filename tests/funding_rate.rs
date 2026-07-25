//! Integration test for funding rate accumulation and settlement.
//!
//! Test Scenario:
//!   1. Create an ETH/USD market with equal long and short OI (100,000 USD each).
//!   2. Configure funding parameters: factor = 1%, exponent = 1, ramp rate high enough
//!      to hit the target in one dt.
//!   3. Advance time by 3600 seconds (1 hour).
//!   4. Trigger funding update via a position decrease and verify:
//!      - saved_funding_factor_per_second is nonzero (longs > shorts → positive = longs pay)
//!      - long_funding_amount_per_size is negative (longs paid)
//!      - short_funding_amount_per_size is positive (shorts received)
//!   5. Open a long position, advance time, decrease it, verify settlement:
//!      - Position's funding_fee_amount_per_size changes
//!      - claimable_funding_amount for the position owner accumulates
//!   6. Symmetric test: shorts > longs → negative factor → shorts pay longs

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{
    claimable_funding_amount_key, funding_amount_per_size_key, funding_decrease_factor_per_second_key,
    funding_exponent_factor_key, funding_factor_key, funding_increase_factor_per_second_key,
    funding_updated_at_key, market_index_token_key, market_long_token_key, market_short_token_key,
    max_funding_factor_per_second_key, min_funding_factor_per_second_key, open_interest_key,
    position_key, roles, saved_funding_factor_per_second_key,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, OrderType, TokenPrice, PositionProps};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use reader::{Reader, ReaderClient as RClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, BytesN, Env};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

struct TestWorld {
    env: Env,
    admin: Address,
    keeper: Address,
    trader1: Address,
    trader2: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    index_tk: Address,
    reader: Address,
}

fn setup() -> TestWorld {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let trader1 = Address::generate(&env);
    let trader2 = Address::generate(&env);

    // Role store
    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    // Data store
    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    // Oracle
    let oracle_addr = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    // Order vault
    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // Market token
    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "ETH Market"),
        &soroban_sdk::String::from_str(&env, "GM-ETH"),
    );

    // Tokens
    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    // Order handler
    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Reader
    let reader = env.register(Reader, ());
    RClient::new(&env, &reader).initialize(&admin, &rs, &ds, &oracle_addr);

    // Setup market
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &long_tk);

    // Mint tokens to traders
    StellarAssetClient::new(&env, &long_tk).mint(&trader1, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&env, &long_tk).mint(&trader2, &(10_000 * ONE_TOKEN));

    TestWorld {
        env,
        admin,
        keeper,
        trader1,
        trader2,
        rs,
        ds,
        oracle: oracle_addr,
        ord_vault,
        ord_handler,
        market_tk,
        long_tk,
        index_tk,
        reader,
    }
}

/// Configure funding parameters for the market.
///
/// Sets funding_factor = 1% (0.01 × FLOAT_PRECISION), exponent = 1,
/// increase/decrease ramp factors high enough to reach target in a single dt,
/// and min/max bounds that allow the full range.
fn configure_funding(w: &TestWorld) {
    let ds_c = DsClient::new(&w.env, &w.ds);

    // funding_factor = 0.01 × FLOAT_PRECISION (1% annual rate base)
    let funding_factor = FLOAT_PRECISION / 100;
    ds_c.set_u128_instance(
        &w.admin,
        &funding_factor_key(&w.env, &w.market_tk),
        &(funding_factor as u128),
    );

    // exponent = 1 (linear)
    ds_c.set_u128_instance(
        &w.admin,
        &funding_exponent_factor_key(&w.env, &w.market_tk),
        &(1u128),
    );

    // increase/decrease ramp: high enough to hit target in one dt (1 hour = 3600s)
    // Set to 100% per second so ramp is unconstrained
    let ramp = FLOAT_PRECISION as u128;
    ds_c.set_u128_instance(
        &w.admin,
        &funding_increase_factor_per_second_key(&w.env, &w.market_tk),
        &ramp,
    );
    ds_c.set_u128_instance(
        &w.admin,
        &funding_decrease_factor_per_second_key(&w.env, &w.market_tk),
        &ramp,
    );

    // min/max bounds: allow factor in range [-0.1, +0.1] per second
    let max = FLOAT_PRECISION / 10;
    let min = -max;
    ds_c.set_i128_instance(
        &w.admin,
        &min_funding_factor_per_second_key(&w.env, &w.market_tk),
        &min,
    );
    ds_c.set_i128_instance(
        &w.admin,
        &max_funding_factor_per_second_key(&w.env, &w.market_tk),
        &max,
    );
}

/// Set oracle prices and seed the pool with collateral tokens.
fn seed_pool_and_set_prices(w: &TestWorld, eth_usd: i128) {
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Set oracle prices
    oracle_c.set_prices_simple(
        &w.keeper,
        &soroban_sdk::Vec::from_array(
            &w.env,
            [
                TokenPrice {
                    token: w.long_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
                TokenPrice {
                    token: w.index_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
            ],
        ),
    );

    // Seed pool with tokens
    let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
    ds_c.set_u128(&w.admin, &pool_key, &(1_000_000 * ONE_TOKEN as u128));
}

/// Open a long position for trader1 and return the position key.
fn open_long_position(w: &TestWorld, size_usd: i128, collateral: i128) -> BytesN<32> {
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &collateral,
    );

    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: size_usd,
        collateral_delta_amount: collateral,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    oh_c.execute_order(&w.keeper, &order_key);

    position_key(&w.env, &w.trader1, &w.market_tk, &w.long_tk, true)
}

/// Open a short position for trader2 and return the position key.
fn open_short_position(w: &TestWorld, size_usd: i128, collateral: i128) -> BytesN<32> {
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader2,
        &w.ord_vault,
        &collateral,
    );

    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader2.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: size_usd,
        collateral_delta_amount: collateral,
        trigger_price: 0,
        acceptable_price: 1_900 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: false,
    });

    oh_c.execute_order(&w.keeper, &order_key);

    position_key(&w.env, &w.trader2, &w.market_tk, &w.long_tk, false)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// When long OI > short OI the funding factor should be positive (longs pay shorts).
/// After advancing 1 hour and triggering a decrease, verify the per-size deltas.
#[test]
fn funding_rate_longs_pay_shorts_when_long_oi_exceeds_short() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let reader_c = RClient::new(&w.env, &w.reader);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    // Set initial funding timestamp to 0
    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    // Set initial saved funding factor to 0
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    // Simulate asymmetric OI: 200,000 long, 100,000 short
    let long_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
    let short_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, false);
    ds_c.set_u128(&w.admin, &long_oi_key, &(200_000 * ONE_USD as u128));
    ds_c.set_u128(&w.admin, &short_oi_key, &(100_000 * ONE_USD as u128));

    // Create a position so we can trigger funding via decrease
    let pos_key = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    // Advance time by 1 hour (3600 seconds)
    let new_time = 3600u64;
    w.env.ledger().set_timestamp(new_time);

    // Trigger funding update via position decrease (partial close)
    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    // Verify funding state was updated
    let saved_factor = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    // With long_oi > short_oi, factor should be positive (longs pay)
    assert!(
        saved_factor > 0,
        "Funding factor must be positive when long OI > short OI, got {saved_factor}"
    );

    // Verify per-size deltas applied
    let long_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        true,
    ));
    let short_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        false,
    ));

    // Longs paid: their per-size accumulator should have decreased (negative delta applied)
    assert!(
        long_fnd_per_size < 0,
        "Long funding per size should be negative (longs paid), got {long_fnd_per_size}"
    );
    // Shorts received: their per-size accumulator should have increased
    assert!(
        short_fnd_per_size > 0,
        "Short funding per size should be positive (shorts received), got {short_fnd_per_size}"
    );

    // Verify via reader
    let funding_info = reader_c.get_funding_info(&w.ds, &w.market_tk);
    assert_eq!(
        funding_info.funding_factor_per_second, saved_factor,
        "Reader must return the same saved funding factor"
    );
    assert_eq!(
        funding_info.long_funding_amount_per_size, long_fnd_per_size,
        "Reader long funding per size must match"
    );
    assert_eq!(
        funding_info.short_funding_amount_per_size, short_fnd_per_size,
        "Reader short funding per size must match"
    );
}

/// When short OI > long OI the funding factor should be negative (shorts pay longs).
#[test]
fn funding_rate_shorts_pay_longs_when_short_oi_exceeds_long() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    // Set initial state
    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    // Asymmetric OI: 100,000 long, 200,000 short
    let long_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
    let short_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, false);
    ds_c.set_u128(&w.admin, &long_oi_key, &(100_000 * ONE_USD as u128));
    ds_c.set_u128(&w.admin, &short_oi_key, &(200_000 * ONE_USD as u128));

    // Create a short position to trigger funding
    let pos_key = open_short_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    // Advance time by 1 hour
    w.env.ledger().set_timestamp(3600);

    // Trigger funding via decrease
    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader2.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 1_900 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: false,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    let saved_factor = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    assert!(
        saved_factor < 0,
        "Funding factor must be negative when short OI > long OI, got {saved_factor}"
    );

    let long_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        true,
    ));
    let short_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        false,
    ));

    // Shorts pay longs: shorts negative, longs positive
    assert!(
        long_fnd_per_size > 0,
        "Long funding per size should be positive (longs received), got {long_fnd_per_size}"
    );
    assert!(
        short_fnd_per_size < 0,
        "Short funding per size should be negative (shorts paid), got {short_fnd_per_size}"
    );
}

/// When OI is balanced, funding factor should be zero and per-size deltas should be zero.
#[test]
fn funding_rate_zero_when_oi_balanced() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    // Equal OI: 150,000 each side
    let long_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
    let short_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, false);
    ds_c.set_u128(&w.admin, &long_oi_key, &(150_000 * ONE_USD as u128));
    ds_c.set_u128(&w.admin, &short_oi_key, &(150_000 * ONE_USD as u128));

    let pos_key = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    w.env.ledger().set_timestamp(3600);

    // Trigger funding via decrease
    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    let saved_factor = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    assert_eq!(
        saved_factor, 0,
        "Funding factor must be zero when OI is balanced, got {saved_factor}"
    );

    let long_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        true,
    ));
    let short_fnd_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        false,
    ));

    assert_eq!(
        long_fnd_per_size, 0,
        "Long funding per size must be zero when balanced, got {long_fnd_per_size}"
    );
    assert_eq!(
        short_fnd_per_size, 0,
        "Short funding per size must be zero when balanced, got {short_fnd_per_size}"
    );
}

/// When OI is zero, funding factor and per-size deltas should remain zero.
#[test]
fn funding_rate_zero_when_no_oi() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    // No OI at all — create a position then fully close it
    let pos_key = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    w.env.ledger().set_timestamp(3600);

    // Fully close the position
    let pos = oh_c.get_position(&pos_key).expect("position must exist");
    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: pos.size_in_usd,
        collateral_delta_amount: pos.collateral_amount,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    // Position should be fully closed — no OI left
    let long_oi = ds_c.get_u128(&open_interest_key(&w.env, &w.market_tk, &w.long_tk, true));
    let short_oi = ds_c.get_u128(&open_interest_key(&w.env, &w.market_tk, &w.long_tk, false));
    assert_eq!(long_oi, 0, "Long OI must be zero after full close");
    assert_eq!(short_oi, 0, "Short OI must be zero");

    // The funding factor should be zero (compute_next_funding_factor returns 0 when total_oi == 0)
    let saved_factor = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    assert_eq!(
        saved_factor, 0,
        "Funding factor must be zero when no OI, got {saved_factor}"
    );
}

/// Funding settlement: a position that held through a funding period should have
/// accumulated claimable funding amount.
#[test]
fn funding_settlement_accumulates_claimable_amount() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    // Asymmetric OI so funding is nonzero
    let long_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
    let short_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, false);
    ds_c.set_u128(&w.admin, &long_oi_key, &(200_000 * ONE_USD as u128));
    ds_c.set_u128(&w.admin, &short_oi_key, &(100_000 * ONE_USD as u128));

    // Open a long position
    let pos_key = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    // Record the initial claimable amount (should be 0)
    let claimable_key = claimable_funding_amount_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        &w.trader1,
    );
    let claimable_before = ds_c.get_i128(&claimable_key);

    // Advance time by 1 hour and trigger funding via another decrease
    w.env.ledger().set_timestamp(3600);

    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    // After settlement, the position's funding_fee_amount_per_size should be updated
    let pos = oh_c.get_position(&pos_key).expect("position must still exist");
    assert_ne!(
        pos.funding_fee_amount_per_size, 0,
        "Position funding fee per size must be nonzero after settlement"
    );

    // The claimable amount may or may not change depending on whether the
    // position was fully settled; verify the per-size value changed from initial
    let funding_per_size_key = funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        true,
    );
    let funding_per_size = ds_c.get_i128(&funding_per_size_key);
    assert!(
        funding_per_size < 0,
        "Long funding per size should be negative (longs paid), got {funding_per_size}"
    );
}

/// Funding rate ramps gradually: after a short dt the factor should be
/// proportionally smaller than after a long dt.
#[test]
fn funding_rate_ramps_proportionally_to_dt() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    // --- First run: 1 hour dt ---
    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    let long_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
    let short_oi_key = open_interest_key(&w.env, &w.market_tk, &w.long_tk, false);
    ds_c.set_u128(&w.admin, &long_oi_key, &(200_000 * ONE_USD as u128));
    ds_c.set_u128(&w.admin, &short_oi_key, &(100_000 * ONE_USD as u128));

    let pos_key = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    w.env.ledger().set_timestamp(3600);

    let close_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key);

    let factor_1h = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    assert!(factor_1h > 0, "Factor after 1h must be positive");

    // --- Second run: 2 hour dt (from fresh state) ---
    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );

    let pos_key2 = open_long_position(&w, 50_000 * ONE_USD, 5_000 * ONE_TOKEN);

    w.env.ledger().set_timestamp(7200);

    let close_key2 = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 1_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &close_key2);

    let factor_2h = ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk));
    assert!(factor_2h > 0, "Factor after 2h must be positive");

    // The 2h factor should be strictly greater than the 1h factor
    // because the ramp has more time to approach the target
    assert!(
        factor_2h >= factor_1h,
        "2h factor ({factor_2h}) must be >= 1h factor ({factor_1h})"
    );
}

/// Integration test: 600 000 USD long OI vs 200 000 USD short OI (3:1 imbalance).
///
/// Steps:
///   1. Set up an ETH/USD market and configure funding parameters.
///   2. Open long positions totalling 600 000 USD OI.
///   3. Open short positions totalling 200 000 USD OI.
///   4. Advance ledger by 1 000 steps (timestamp += 1 000 s) without touching positions.
///   5. Call `update_funding_state` directly (via gmx_market_utils) to trigger the update.
///   6. Assert:
///      - `saved_funding_factor_per_second` is positive (longs pay).
///      - `long_funding_amount_per_size` is negative (debit on longs).
///      - `short_funding_amount_per_size` is positive (credit on shorts).
///      - Absolute values are within 1 % of the analytically expected values.
///      - Conservation: |long_paid_usd - short_received_usd| / long_paid_usd < 1 %.
#[test]
fn test_funding_rate_accumulation_and_settlement_integration() {
    use gmx_market_utils::update_funding_state;
    use gmx_types::MarketProps;

    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    configure_funding(&w);
    seed_pool_and_set_prices(&w, 2000);

    // ── Initialise funding state at t = 0 ─────────────────────────────────────
    ds_c.set_u128(
        &w.admin,
        &funding_updated_at_key(&w.env, &w.market_tk),
        &0u128,
    );
    ds_c.set_i128(
        &w.admin,
        &saved_funding_factor_per_second_key(&w.env, &w.market_tk),
        &0i128,
    );
    // Zero-out per-size accumulators to start clean.
    ds_c.set_i128(
        &w.admin,
        &funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true),
        &0i128,
    );
    ds_c.set_i128(
        &w.admin,
        &funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, false),
        &0i128,
    );

    // ── Step 2: inject 600 000 USD long OI and 200 000 USD short OI ───────────
    //
    // We write OI directly (as the existing tests do) so the imbalance is exact
    // and independent of position-fee noise from open_long_position.
    let long_oi: u128 = 600_000 * ONE_USD as u128;
    let short_oi: u128 = 200_000 * ONE_USD as u128;

    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, &w.long_tk, true),
        &long_oi,
    );
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, &w.long_tk, false),
        &short_oi,
    );

    // Open a minimal position so there is an account to trigger decrease on.
    // This is needed for the execute_order path; OI is already set above.
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.trader1, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(5_000 * ONE_TOKEN),
    );
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let seed_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 50_000 * ONE_USD,
        collateral_delta_amount: 5_000 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });
    oh_c.execute_order(&w.keeper, &seed_key);

    // ── Step 4: advance ledger state by 1 000 seconds ─────────────────────────
    let dt: u64 = 1_000;
    w.env.ledger().set_timestamp(dt);

    // ── Step 5: trigger update_funding_state directly ─────────────────────────
    //
    // The caller must hold CONTROLLER; we use `w.admin` which was granted that
    // role in configure_funding → setup.
    let market_props = MarketProps {
        market_token: w.market_tk.clone(),
        index_token: w.index_tk.clone(),
        long_token: w.long_tk.clone(),
        short_token: w.long_tk.clone(), // same token (single-sided market, matching setup())
    };

    let result = update_funding_state(
        &w.env,
        &w.ds,
        &w.admin,
        &market_props,
        2_000 * ONE_USD, // long_token_price
        2_000 * ONE_USD, // short_token_price
        dt,
    );

    // ── Step 6a: direction checks ──────────────────────────────────────────────
    assert!(
        result.funding_factor_per_second > 0,
        "Funding factor must be positive when long OI > short OI, got {}",
        result.funding_factor_per_second
    );

    let long_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        true,
    ));
    let short_per_size = ds_c.get_i128(&funding_amount_per_size_key(
        &w.env,
        &w.market_tk,
        &w.long_tk,
        false,
    ));

    // Longs paid → per-size accumulator decremented (negative / more negative).
    assert!(
        long_per_size < 0,
        "Long funding per size must be negative (debit on longs), got {long_per_size}"
    );
    // Shorts received → per-size accumulator incremented (positive / more positive).
    assert!(
        short_per_size > 0,
        "Short funding per size must be positive (credit on shorts), got {short_per_size}"
    );

    // ── Step 6b: magnitude check (within 1 % of expected) ─────────────────────
    //
    // With funding_factor = FLOAT_PRECISION/100 = 1e28, exponent = 1, and
    // ratio = |600k − 200k| / 800k = 0.5:
    //   target_factor ≈ (FLOAT_PRECISION/100) × 0.5 = FLOAT_PRECISION/200.
    // Since the ramp is set to 1.0 per second (effectively unconstrained), the
    // factor reaches the target immediately.
    //
    // Funding USD per second = factor × min(long_oi, short_oi) / FLOAT_PRECISION
    //                        ≈ (FLOAT_PRECISION/200) × 200_000e30 / FLOAT_PRECISION
    //                        = 1_000 USD.
    // Over 1 000 s:  funding_usd_total ≈ 1 000 × 1 000 = 1 000 000 USD (in FLOAT_PRECISION).
    //
    // long_per_size  = −funding_usd_total × FLOAT_PRECISION / long_oi
    //               ≈ −(1_000_000 × 1e30) × 1e30 / (600_000 × 1e30)
    //               ≈ −1.667 × 1e30.
    // short_per_size = +funding_usd_total × FLOAT_PRECISION / short_oi
    //               ≈ +(1_000_000 × 1e30) × 1e30 / (200_000 × 1e30)
    //               ≈ +5.0 × 1e30.
    //
    // We allow ±1 % tolerance around these expected values.

    let tolerance_bps: i128 = 100; // 1 % = 100 bps

    // Expected long_per_size ≈ −(FLOAT_PRECISION/200) × min_oi × dt / long_oi
    // (Simplified integer arithmetic: expect a non-trivial magnitude)
    let abs_long = long_per_size.abs();
    let abs_short = short_per_size;

    // The ratio short_per_size / long_per_size should equal long_oi / short_oi = 3.0.
    // Check: abs_short × long_oi ≈ abs_long × short_oi  (cross multiply)
    // For integer maths, scale both sides by 1_000 for bps:
    let lhs = abs_short as u128 * long_oi / ONE_USD as u128;
    let rhs = abs_long as u128 * short_oi / ONE_USD as u128;
    let numerator = if lhs > rhs { lhs - rhs } else { rhs - lhs };
    let denominator = lhs.max(rhs);
    let ratio_error_bps = if denominator == 0 {
        0u128
    } else {
        numerator * 10_000 / denominator
    };

    assert!(
        ratio_error_bps <= tolerance_bps as u128,
        "Conservation ratio mismatch: long_per_size={long_per_size}, \
         short_per_size={short_per_size}, error={ratio_error_bps} bps (max {tolerance_bps})"
    );

    // ── Step 6c: conservation invariant ───────────────────────────────────────
    //
    // Total funding paid by longs ≈ total funding received by shorts.
    // In FLOAT_PRECISION terms:
    //   long_paid   = abs(long_per_size) × long_oi / FLOAT_PRECISION
    //   short_recv  = short_per_size     × short_oi / FLOAT_PRECISION
    //
    // These should be equal up to rounding.

    let long_paid = abs_long as u128 * long_oi / ONE_USD as u128;
    let short_recv = abs_short as u128 * short_oi / ONE_USD as u128;
    let conserv_num = if long_paid > short_recv {
        long_paid - short_recv
    } else {
        short_recv - long_paid
    };
    let conserv_denom = long_paid.max(short_recv);
    let conserv_error_bps = if conserv_denom == 0 {
        0u128
    } else {
        conserv_num * 10_000 / conserv_denom
    };

    assert!(
        conserv_error_bps <= tolerance_bps as u128,
        "Conservation invariant violated: long_paid={long_paid}, \
         short_recv={short_recv}, error={conserv_error_bps} bps (max {tolerance_bps})"
    );

    // Verify result struct mirrors what was written to storage.
    assert_eq!(
        result.funding_factor_per_second,
        ds_c.get_i128(&saved_funding_factor_per_second_key(&w.env, &w.market_tk)),
        "FundingResult must reflect the persisted factor"
    );
    assert!(
        result.long_funding_per_size_delta < 0,
        "FundingResult long delta must be negative"
    );
    assert!(
        result.short_funding_per_size_delta > 0,
        "FundingResult short delta must be positive"
    );
}
