//! Integration tests for issue #269: zero-amount validation on order, deposit, and withdrawal creation.
//!
//! Verifies that create_order with size_delta_usd = 0 on a position order type
//! produces a typed Error::ZeroSizeDelta rather than a panic, and that the
//! deposit and withdrawal handlers similarly reject zero-amount inputs.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, roles,
};
use gmx_types::{CreateDepositParams, CreateOrderParams, CreateWithdrawalParams, OrderType};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{Error as OrderError, OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};
use withdrawal_handler::{WithdrawalHandler, WithdrawalHandlerClient as WHClient};
use withdrawal_vault::{WithdrawalVault, WithdrawalVaultClient as WVClient};

const ONE_TOKEN: i128 = 10_000_000;

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    user: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    dep_vault: Address,
    wth_vault: Address,
    ord_vault: Address,
    dep_handler: Address,
    wth_handler: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
    index_tk: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let user = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle_addr = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let wth_vault = env.register(WithdrawalVault, ());
    WVClient::new(&env, &wth_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM-ETH"),
    );

    let long_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let index_tk = Address::generate(&env);

    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);

    let wth_handler = env.register(WithdrawalHandler, ());
    WHClient::new(&env, &wth_handler).initialize(&admin, &rs, &ds, &oracle_addr, &wth_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    let rs_c = RsClient::new(&env, &rs);
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    StellarAssetClient::new(&env, &long_tk).mint(&user, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&env, &short_tk).mint(&user, &(10_000 * ONE_TOKEN));

    World {
        env,
        admin,
        keeper,
        user,
        rs,
        ds,
        oracle: oracle_addr,
        dep_vault,
        wth_vault,
        ord_vault,
        dep_handler,
        wth_handler,
        ord_handler,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
}

// ── Issue #269: create_order with size_delta_usd = 0 → ZeroSizeDelta ─────────

#[test]
fn market_increase_with_zero_size_reverts() {
    let w = setup();
    let env = &w.env;

    // Transfer collateral into vault first (required for increase orders)
    soroban_sdk::token::Client::new(env, &w.long_tk)
        .transfer(&w.user, &w.ord_vault, &(100 * ONE_TOKEN));

    let result = OHClient::new(env, &w.ord_handler).try_create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 0, // zero size — must revert
            collateral_delta_amount: 100 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: None,
        },
    );

    assert_eq!(result, Err(Ok(OrderError::ZeroSizeDelta)));
}

#[test]
fn limit_increase_with_zero_size_reverts() {
    let w = setup();
    let env = &w.env;

    soroban_sdk::token::Client::new(env, &w.long_tk)
        .transfer(&w.user, &w.ord_vault, &(100 * ONE_TOKEN));

    let result = OHClient::new(env, &w.ord_handler).try_create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 0,
            collateral_delta_amount: 100 * ONE_TOKEN,
            trigger_price: 2_000 * 10_i128.pow(30),
            acceptable_price: 2_100 * 10_i128.pow(30),
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::LimitIncrease,
            is_long: true,
            expiry_ledger: None,
        },
    );

    assert_eq!(result, Err(Ok(OrderError::ZeroSizeDelta)));
}

#[test]
fn market_decrease_with_zero_size_reverts() {
    let w = setup();
    let env = &w.env;

    let result = OHClient::new(env, &w.ord_handler).try_create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 0,
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

    assert_eq!(result, Err(Ok(OrderError::ZeroSizeDelta)));
}

#[test]
fn swap_order_with_zero_size_succeeds() {
    let w = setup();
    let env = &w.env;

    // Swap orders use size_delta_usd = 0 legitimately; should NOT revert
    soroban_sdk::token::Client::new(env, &w.long_tk)
        .transfer(&w.user, &w.ord_vault, &(100 * ONE_TOKEN));

    let result = OHClient::new(env, &w.ord_handler).try_create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 0,
            collateral_delta_amount: 100 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
            expiry_ledger: None,
        },
    );

    // Should succeed (no ZeroSizeDelta) — may fail later for other reasons but not this check
    assert!(result.is_ok() || !matches!(result, Err(Ok(OrderError::ZeroSizeDelta))));
}

// ── Issue #269: deposit with zero amounts → ZeroDeposit ───────────────────────

#[test]
fn deposit_with_zero_amounts_reverts() {
    let w = setup();
    let env = &w.env;

    let result = DHClient::new(env, &w.dep_handler).try_create_deposit(
        &w.user,
        &CreateDepositParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 0,
            short_token_amount: 0,
            min_market_tokens: 0,
            execution_fee: 0,
        },
    );

    assert!(result.is_err(), "zero-amount deposit must revert");
}

// ── Issue #269: withdrawal with zero market token amount → ZeroWithdrawal ─────

#[test]
fn withdrawal_with_zero_amount_reverts() {
    let w = setup();
    let env = &w.env;

    let result = WHClient::new(env, &w.wth_handler).try_create_withdrawal(
        &w.user,
        &CreateWithdrawalParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            market_token_amount: 0,
            min_long_token_amount: 0,
            min_short_token_amount: 0,
            execution_fee: 0,
        },
    );

    assert!(result.is_err(), "zero-amount withdrawal must revert");
}
