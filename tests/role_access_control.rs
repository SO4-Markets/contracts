//! Integration test for issue #196: role store access control — unauthorized keeper is rejected
//!
//! Test Scenario:
//!   Unauthorized Calls (must all revert):
//!     - order_handler::execute_order called by address without ORDER_KEEPER → revert
//!     - liquidation_handler::liquidate without LIQUIDATION_KEEPER → revert
//!     - adl_handler::execute_adl without ADL_KEEPER → revert
//!     - fee_handler::claim_fees without FEE_KEEPER → revert
//!     - data_store::set_u128 called directly without CONTROLLER → revert
//!     - role_store::grant_role called by non-admin → revert
//!   
//!   Authorized Calls (must succeed after role grant):
//!     - Grant ORDER_KEEPER to keeper account
//!     - execute_order by that account → succeeds
//!   
//!   Role Revocation:
//!     - Revoke ORDER_KEEPER
//!     - execute_order → reverts again

#![cfg(test)]

use adl_handler::{AdlHandler, AdlHandlerClient as AHClient};
use data_store::{DataStore, DataStoreClient as DsClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_pnl_factor_for_adl_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, OrderType, TokenPrice};
use liquidation_handler::{LiquidationHandler, LiquidationHandlerClient as LHClient};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, BytesN, Env};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

struct TestWorld {
    env: Env,
    admin: Address,
    unauthorized_user: Address,
    keeper: Address,
    liq_keeper: Address,
    adl_keeper: Address,
    fee_keeper: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    ord_handler: Address,
    liq_handler: Address,
    adl_handler: Address,
    fee_handler: Address,
    market_tk: Address,
    long_tk: Address,
    index_tk: Address,
}

fn setup() -> TestWorld {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);
    let keeper = Address::generate(&env);
    let liq_keeper = Address::generate(&env);
    let adl_keeper = Address::generate(&env);
    let fee_keeper = Address::generate(&env);

    // Role store
    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));

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

    // Liquidation handler
    let liq_handler = env.register(LiquidationHandler, ());
    LHClient::new(&env, &liq_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_handler);

    // ADL handler
    let adl_handler = env.register(AdlHandler, ());
    AHClient::new(&env, &adl_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_handler);

    // Fee handler
    let fee_handler = env.register(FeeHandler, ());
    FHClient::new(&env, &fee_handler).initialize(&admin, &rs, &ds, &market_tk);

    // Setup market
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &long_tk);

    // Mint tokens
    StellarAssetClient::new(&env, &long_tk).mint(&unauthorized_user, &(1000 * ONE_TOKEN));

    TestWorld {
        env,
        admin,
        unauthorized_user,
        keeper,
        liq_keeper,
        adl_keeper,
        fee_keeper,
        rs,
        ds,
        oracle: oracle_addr,
        ord_vault,
        ord_handler,
        liq_handler,
        adl_handler,
        fee_handler,
        market_tk,
        long_tk,
        index_tk,
    }
}

#[test]
fn unauthorized_execute_order_reverts() {
    let w = setup();
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Setup oracle prices
    oracle_c.set_primary_price(
        &w.admin,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.admin,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral and create order
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.unauthorized_user,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.unauthorized_user.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 1000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    // Unauthorized user tries to execute - should fail
    let result = oh_c.try_execute_order(&w.unauthorized_user, &order_key);
    assert!(
        result.is_err(),
        "Execute order without ORDER_KEEPER role should fail"
    );
}

#[test]
fn unauthorized_liquidation_reverts() {
    let w = setup();
    let lh_c = LHClient::new(&w.env, &w.liq_handler);

    // Try to liquidate without LIQUIDATION_KEEPER role
    let dummy_position_key = BytesN::from_array(&w.env, &[0u8; 32]);
    let result = lh_c.try_liquidate_position(&w.unauthorized_user, &dummy_position_key);
    
    assert!(
        result.is_err(),
        "Liquidate without LIQUIDATION_KEEPER role should fail"
    );
}

#[test]
fn unauthorized_adl_reverts() {
    let w = setup();
    let ah_c = AHClient::new(&w.env, &w.adl_handler);
    let ds_c = DsClient::new(&w.env, &w.ds);

    // Set ADL threshold
    ds_c.set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &(90_000_000_000_000_000_000_000_000_000 as u128), // 0.9 * 10^30
    );

    // Try to execute ADL without ADL_KEEPER role
    let dummy_position_key = BytesN::from_array(&w.env, &[0u8; 32]);
    let result = ah_c.try_execute_adl(&w.unauthorized_user, &w.market_tk, &true, &dummy_position_key);
    
    assert!(
        result.is_err(),
        "Execute ADL without ADL_KEEPER role should fail"
    );
}

#[test]
fn unauthorized_claim_fees_reverts() {
    let w = setup();
    let fh_c = FHClient::new(&w.env, &w.fee_handler);

    // Try to claim fees without FEE_KEEPER role
    let result = fh_c.try_claim_fees(&w.unauthorized_user, &w.market_tk, &w.long_tk);
    
    assert!(
        result.is_err(),
        "Claim fees without FEE_KEEPER role should fail"
    );
}

#[test]
fn unauthorized_data_store_set_reverts() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);

    // Try to set data without CONTROLLER role
    let dummy_key = soroban_sdk::Bytes::from_slice(&w.env, b"test_key");
    let result = ds_c.try_set_u128(&w.unauthorized_user, &dummy_key, &12345u128);
    
    assert!(
        result.is_err(),
        "Data store set without CONTROLLER role should fail"
    );
}

#[test]
fn unauthorized_grant_role_reverts() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);

    // Try to grant role without admin privileges
    let result = rs_c.try_grant_role(
        &w.unauthorized_user,
        &w.unauthorized_user,
        &roles::order_keeper(&w.env),
    );
    
    assert!(
        result.is_err(),
        "Grant role without admin privileges should fail"
    );
}

#[test]
fn authorized_execute_order_succeeds_after_grant() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Grant ORDER_KEEPER role to keeper
    rs_c.grant_role(&w.admin, &w.keeper, &roles::order_keeper(&w.env));

    // Setup oracle prices
    oracle_c.set_primary_price(
        &w.admin,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.admin,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral and create order
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.unauthorized_user,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.unauthorized_user.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 1000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    // Keeper with proper role executes - should succeed
    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_ok(),
        "Execute order with ORDER_KEEPER role should succeed"
    );
}

#[test]
fn role_revocation_prevents_access() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);

    // Grant ORDER_KEEPER role to keeper
    rs_c.grant_role(&w.admin, &w.keeper, &roles::order_keeper(&w.env));

    // Setup oracle prices
    oracle_c.set_primary_price(
        &w.admin,
        &w.index_tk,
        &TokenPrice {
            min: 2000 * ONE_USD,
            max: 2000 * ONE_USD,
        },
    );
    oracle_c.set_primary_price(
        &w.admin,
        &w.long_tk,
        &TokenPrice {
            min: ONE_USD,
            max: ONE_USD,
        },
    );

    // Transfer collateral and create order
    StellarAssetClient::new(&w.env, &w.long_tk).transfer(
        &w.unauthorized_user,
        &w.ord_vault,
        &(100 * ONE_TOKEN),
    );

    let order_key = oh_c.create_order(&CreateOrderParams {
        receiver: w.unauthorized_user.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.long_tk.clone(),
        swap_path: soroban_sdk::Vec::new(&w.env),
        size_delta_usd: 1000 * ONE_USD,
        collateral_delta_amount: 100 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2100 * ONE_USD,
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });

    // Revoke ORDER_KEEPER role
    rs_c.revoke_role(&w.admin, &w.keeper, &roles::order_keeper(&w.env));

    // Keeper tries to execute after revocation - should fail
    let result = oh_c.try_execute_order(&w.keeper, &order_key);
    assert!(
        result.is_err(),
        "Execute order after role revocation should fail"
    );
}
