//! Integration test for issue #197: market OI cap blocks position increase beyond limit
//!
//! Issue #450: every test here drives the open-interest cap through *real*
//! create_order/execute_order calls — never a synthetic direct storage write —
//! so the cap-aggregation logic (open_interest_for_side sums two separate
//! per-collateral-token storage buckets) is exercised the same way it would be
//! under genuine multi-actor load.
//!
//! Test Scenario:
//!   1. Create ETH/USD market, set max_open_interest_long = 500,000 USD
//!   2. trader1 opens a real long position of 499,000 USD long OI → succeeds
//!   3. trader1 attempts a further 2,000 USD long increase (would push OI to
//!      501,000) → reverts
//!   4. trader1 attempts a further 1,000 USD long increase (exactly at cap:
//!      500,000) → succeeds
//!   5. Short OI cap is independent of long OI cap
//!   6. OI cap of 0 = uncapped (default behaviour)
//!   7. Position decrease is always allowed even when at cap
//!   8. trader1 (collateralized in long_tk) and trader2 (collateralized in
//!      short_tk) both push the *same* long-side OI cap — proving the cap
//!      aggregates both collateral-token storage buckets when driven by real,
//!      independent order flow from two different traders.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_open_interest_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::CreateOrderParams;
use gmx_types::{OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

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
    short_tk: Address,
    index_tk: Address,
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
    // Issue #450: a genuinely distinct short_token, so OI aggregation across
    // the two collateral-token storage buckets can be exercised by real orders
    // instead of one collateral token standing in for both.
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    // Order handler
    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // order_handler needs CONTROLLER to write open interest / position state to
    // data_store and to move collateral through order_vault and market_token.
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    // Setup market
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    // Mint tokens to traders
    StellarAssetClient::new(&env, &long_tk).mint(&trader1, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&env, &long_tk).mint(&trader2, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&env, &short_tk).mint(&trader1, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&env, &short_tk).mint(&trader2, &(10_000 * ONE_TOKEN));

    let world = TestWorld {
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
        short_tk,
        index_tk,
    };

    // Oracle prices: index at $2000, both collateral tokens pegged at $1.
    let oracle_c = OClient::new(&world.env, &world.oracle);
    oracle_c.set_prices_simple(
        &world.keeper,
        &Vec::from_array(
            &world.env,
            [
                TokenPrice {
                    token: world.index_tk.clone(),
                    min: 2_000 * ONE_USD,
                    max: 2_000 * ONE_USD,
                },
                TokenPrice {
                    token: world.long_tk.clone(),
                    min: ONE_USD,
                    max: ONE_USD,
                },
                TokenPrice {
                    token: world.short_tk.clone(),
                    min: ONE_USD,
                    max: ONE_USD,
                },
            ],
        ),
    );

    world
}

/// Open a real MarketIncrease position via create_order/execute_order,
/// collateralized in `collateral_tk`. Panics (failing the test) if execution reverts.
fn open_real_position(
    w: &TestWorld,
    trader: &Address,
    collateral_tk: &Address,
    collateral_amount: i128,
    size_usd: i128,
    is_long: bool,
) {
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    soroban_sdk::token::Client::new(&w.env, collateral_tk).transfer(
        trader,
        &w.ord_vault,
        &collateral_amount,
    );
    let order_key = oh_c.create_order(
        trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: collateral_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: size_usd,
            collateral_delta_amount: collateral_amount,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long,
            expiry_ledger: None,
        },
    );
    oh_c.execute_order(&w.keeper, &order_key);
}

/// Attempt a real MarketIncrease order and return whether execution succeeded.
fn try_real_increase(
    w: &TestWorld,
    trader: &Address,
    collateral_tk: &Address,
    collateral_amount: i128,
    size_usd: i128,
    is_long: bool,
) -> bool {
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    soroban_sdk::token::Client::new(&w.env, collateral_tk).transfer(
        trader,
        &w.ord_vault,
        &collateral_amount,
    );
    let order_key = oh_c.create_order(
        trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: collateral_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: size_usd,
            collateral_delta_amount: collateral_amount,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long,
            expiry_ledger: None,
        },
    );
    oh_c.try_execute_order(&w.keeper, &order_key).is_ok()
}

#[test]
fn oi_cap_exact_boundary_succeeds_one_over_fails() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    // Set max OI long to 500,000 USD
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Real prior order brings long OI to 499,000 USD.
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 499_000 * ONE_USD, true);

    // A further 2,000 USD increase would exceed the cap (501,000 > 500,000).
    let succeeded = try_real_increase(&w, &w.trader1, &w.long_tk, 100 * ONE_TOKEN, 2_000 * ONE_USD, true);
    assert!(!succeeded, "position increase that exceeds OI cap should fail");
}

#[test]
fn oi_cap_exactly_at_cap_succeeds() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Real prior order brings long OI to 499,000 USD.
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 499_000 * ONE_USD, true);

    // Increase by exactly 1,000 USD hits the cap exactly at 500,000.
    let succeeded = try_real_increase(&w, &w.trader1, &w.long_tk, 50 * ONE_TOKEN, 1_000 * ONE_USD, true);
    assert!(succeeded, "position increase that hits cap exactly should succeed");
}

#[test]
fn oi_cap_short_independent_of_long() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    // Set max OI long to 500,000 USD, short to 300,000 USD (independent caps).
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, false),
        &(300_000 * ONE_USD as u128),
    );

    // Max out long OI with a real order.
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 500_000 * ONE_USD, true);

    // A short position should succeed even though long is fully at cap.
    let succeeded = try_real_increase(&w, &w.trader1, &w.long_tk, 100 * ONE_TOKEN, 10_000 * ONE_USD, false);
    assert!(succeeded, "short position should succeed when only long cap is hit");
}

#[test]
fn oi_cap_zero_means_uncapped() {
    let w = setup();
    // OI cap defaults to 0 (uncapped) — max_open_interest_key intentionally left unset.

    // A very large position should succeed with no cap configured.
    let succeeded = try_real_increase(&w, &w.trader1, &w.long_tk, 5_000 * ONE_TOKEN, 1_000_000 * ONE_USD, true);
    assert!(succeeded, "position increase should succeed when OI cap is 0 (uncapped)");
}

#[test]
fn oi_cap_decrease_always_allowed() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    // Set max OI long to 500,000 USD and bring OI to exactly the cap with a real order.
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 500_000 * ONE_USD, true);

    // Decreasing the position should always be allowed, even while at the cap.
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let order_key = oh_c.create_order(
        &w.trader1,
        &CreateOrderParams {
            receiver: w.trader1.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: 10_000 * ONE_USD,
            collateral_delta_amount: 0,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
            expiry_ledger: None,
        },
    );

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "position decrease should always be allowed even at OI cap"
    );
}

// ── Issue #450: real multi-trader, multi-collateral-token cap aggregation ────
//
// open_interest_for_side sums two separate storage buckets — one per
// collateral token — because a position's collateral token isn't tied to its
// long/short direction. These tests drive both buckets through genuinely
// independent order flow from two different traders, so a future change that
// updates only one of the two buckets (or rounding that makes the actual OI
// increment differ from the requested size) would be caught here.

#[test]
fn oi_cap_boundary_via_real_multi_trader_multi_collateral_orders_reverts_over_cap() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // trader1 opens a real long position collateralized in long_tk, bringing
    // long-side OI (long_tk bucket) to just under the cap.
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 499_000 * ONE_USD, true);

    // trader2 opens a real long position collateralized in short_tk (a
    // different collateral token, same market, same direction) that would
    // push the aggregate long OI (long_tk bucket + short_tk bucket) over the
    // cap: 499,000 + 2,000 = 501,000 > 500,000.
    let succeeded = try_real_increase(&w, &w.trader2, &w.short_tk, 100 * ONE_TOKEN, 2_000 * ONE_USD, true);
    assert!(
        !succeeded,
        "second trader's order in a different collateral token must still be blocked \
         once the aggregated long OI across both collateral-token buckets exceeds the cap"
    );
}

#[test]
fn oi_cap_boundary_via_real_multi_trader_multi_collateral_orders_exact_boundary_succeeds() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // trader1 opens a real long position collateralized in long_tk: 499,000 USD.
    open_real_position(&w, &w.trader1, &w.long_tk, 600 * ONE_TOKEN, 499_000 * ONE_USD, true);

    // trader2 opens a real long position collateralized in short_tk that fits
    // exactly at the boundary: 499,000 + 1,000 = 500,000 == cap.
    let succeeded = try_real_increase(&w, &w.trader2, &w.short_tk, 50 * ONE_TOKEN, 1_000 * ONE_USD, true);
    assert!(
        succeeded,
        "second trader's order in a different collateral token that lands exactly on the \
         aggregated cap must succeed"
    );

    // A further increase from either trader, in either collateral token, must
    // now revert — the aggregate is exactly at the cap.
    let over = try_real_increase(&w, &w.trader1, &w.long_tk, 10 * ONE_TOKEN, 1 * ONE_USD, true);
    assert!(!over, "any further long increase once the aggregate cap is hit must fail");
}
