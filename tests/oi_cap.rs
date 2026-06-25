//! Integration test for issue #197: market OI cap blocks position increase beyond limit
//!
//! Test Scenario:
//!   1. Create ETH/USD market, set max_open_interest_long = 500,000 USD
//!   2. Open positions totalling exactly 499,000 USD long OI → all succeed
//!   3. Attempt to open a 2,000 USD long position (would push OI to 501,000) → revert
//!   4. Attempt to open a 1,000 USD long position (exactly at cap: 500,000) → succeeds
//!   5. Any further long increase → reverts
//!   6. Short OI cap is independent of long OI cap
//!   7. OI cap of 0 = uncapped (default behaviour)
//!   8. Position decrease is always allowed even when at cap

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_open_interest_key, open_interest_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::CreateOrderParams;
use gmx_types::{OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

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
    }
}

#[test]
fn oi_cap_exact_boundary_succeeds_one_over_fails() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Set max OI long to 500,000 USD
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Simulate existing OI of 499,000 USD
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, true),
        &(499_000 * ONE_USD as u128),
    );

    // Set oracle prices
    oracle_c.set_primary_price(
        &w.keeper,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.keeper,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(200 * ONE_TOKEN),
    );

    // Attempt to increase by 2,000 USD (would exceed cap)
    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 2_000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_err(),
        "Position increase that exceeds OI cap should fail"
    );
}

#[test]
fn oi_cap_exactly_at_cap_succeeds() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Set max OI long to 500,000 USD
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Simulate existing OI of 499,000 USD
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, true),
        &(499_000 * ONE_USD as u128),
    );

    // Set oracle prices
    oracle_c.set_primary_price(
        &w.keeper,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.keeper,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    // Increase by exactly 1,000 USD (hits cap exactly at 500,000)
    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 1_000 * ONE_USD,
        collateral_delta_amount: 50 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "Position increase that hits cap exactly should succeed"
    );
}

#[test]
fn oi_cap_short_independent_of_long() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Set max OI long to 500,000 USD
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Set max OI short to 300,000 USD (different from long)
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, false),
        &(300_000 * ONE_USD as u128),
    );

    // Max out long OI
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Set oracle prices
    oracle_c.set_primary_price(
        &w.keeper,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.keeper,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    // Open short position (should succeed even though long is at cap)
    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 1_900 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: false, // Short position
    });

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "Short position should succeed when only long cap is hit"
    );
}

#[test]
fn oi_cap_zero_means_uncapped() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // OI cap defaults to 0 (uncapped)
    // Don't set max_open_interest_key

    // Set very high existing OI
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, true),
        &(999_000_000 * ONE_USD as u128),
    );

    // Set oracle prices
    oracle_c.set_primary_price(
        &w.keeper,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.keeper,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    // Open large position (should succeed - no cap)
    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 1_000_000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2_100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "Position increase should succeed when OI cap is 0 (uncapped)"
    );
}

#[test]
fn oi_cap_decrease_always_allowed() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Set max OI long to 500,000 USD and set current OI at cap
    ds_c.set_u128(
        &w.admin,
        &max_open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );
    ds_c.set_u128(
        &w.admin,
        &open_interest_key(&w.env, &w.market_tk, true),
        &(500_000 * ONE_USD as u128),
    );

    // Set oracle prices
    oracle_c.set_primary_price(
        &w.keeper,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.keeper,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.trader1,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    // Create decrease order (should always be allowed)
    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.trader1.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 10_000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 1_900 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });

    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "Position decrease should always be allowed even at OI cap"
    );
}
