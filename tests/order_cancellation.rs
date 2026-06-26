#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{
    account_order_list_key, market_index_token_key, market_long_token_key, market_short_token_key,
    order_list_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    keeper: Address,
    user: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    market_tk: Address,
    xlm_tk: Address,
    usdc_tk: Address,
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

    let oracle = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle, &dep_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &ord_vault);

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "XLM/USDC Market"),
        &soroban_sdk::String::from_str(&env, "GM-XLM"),
    );

    let xlm_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let usdc_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = xlm_tk.clone();

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &xlm_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &usdc_tk);

    World {
        env,
        keeper,
        user,
        ds,
        oracle,
        ord_vault,
        dep_handler,
        ord_handler,
        market_tk,
        xlm_tk,
        usdc_tk,
        index_tk,
    }
}

fn set_prices(w: &World, xlm_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice {
                    token: w.xlm_tk.clone(),
                    min: xlm_usd * ONE_USD,
                    max: xlm_usd * ONE_USD,
                },
                TokenPrice {
                    token: w.usdc_tk.clone(),
                    min: ONE_USD,
                    max: ONE_USD,
                },
                TokenPrice {
                    token: w.index_tk.clone(),
                    min: xlm_usd * ONE_USD,
                    max: xlm_usd * ONE_USD,
                },
            ],
        ),
    );
}

fn seed_pool(w: &World) {
    let lp = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.xlm_tk).mint(&lp, &(1_000 * ONE_TOKEN));
    StellarAssetClient::new(&w.env, &w.usdc_tk).mint(&lp, &(100_000 * ONE_TOKEN));
    set_prices(w, 1);
    let key = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.xlm_tk.clone(),
            initial_short_token: w.usdc_tk.clone(),
            long_token_amount: 1_000 * ONE_TOKEN,
            short_token_amount: 100_000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &key);
}

fn create_long_increase_order(
    w: &World,
    collateral_amount: i128,
    execution_fee: i128,
    order_type: OrderType,
    trigger_price: i128,
) -> soroban_sdk::BytesN<32> {
    StellarAssetClient::new(&w.env, &w.usdc_tk).transfer(&w.user, &w.ord_vault, &collateral_amount);
    if execution_fee > 0 {
        StellarAssetClient::new(&w.env, &w.xlm_tk).transfer(&w.user, &w.ord_vault, &execution_fee);
    }

    OHClient::new(&w.env, &w.ord_handler).create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.usdc_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 5_000 * ONE_USD,
            collateral_delta_amount: collateral_amount,
            trigger_price,
            acceptable_price: 2 * ONE_USD,
            execution_fee,
            min_output_amount: 0,
            order_type,
            is_long: true,
        },
    )
}

#[test]
fn user_cancel_returns_exact_collateral_and_execution_fee() {
    let w = setup();
    let collateral_amount = 1_000 * ONE_TOKEN;
    let execution_fee = ONE_TOKEN / 2;

    StellarAssetClient::new(&w.env, &w.usdc_tk).mint(&w.user, &collateral_amount);
    StellarAssetClient::new(&w.env, &w.xlm_tk).mint(&w.user, &execution_fee);

    let usdc_before = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);
    let xlm_before = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.user);

    let key = create_long_increase_order(
        &w,
        collateral_amount,
        execution_fee,
        OrderType::MarketIncrease,
        0,
    );

    let ds_c = DsClient::new(&w.env, &w.ds);
    assert!(ds_c.contains_bytes32(&order_list_key(&w.env), &key));
    assert!(ds_c.contains_bytes32(&account_order_list_key(&w.env, &w.user), &key));

    OHClient::new(&w.env, &w.ord_handler).cancel_order(&w.user, &key);

    let usdc_after = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);
    let xlm_after = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.user);

    assert_eq!(usdc_after, usdc_before, "user must receive the exact USDC deposit back");
    assert_eq!(xlm_after, xlm_before, "user must receive the exact XLM execution fee back");
    assert!(
        OHClient::new(&w.env, &w.ord_handler).get_order(&key).is_none(),
        "order entry must be removed after cancel"
    );
    assert!(
        !ds_c.contains_bytes32(&order_list_key(&w.env), &key),
        "global order list must not contain the cancelled order"
    );
    assert!(
        !ds_c.contains_bytes32(&account_order_list_key(&w.env, &w.user), &key),
        "account order list must not contain the cancelled order"
    );
}

#[test]
fn keeper_failed_execution_cancels_and_pays_keeper_fee() {
    let w = setup();
    let collateral_amount = 1_000 * ONE_TOKEN;
    let execution_fee = ONE_TOKEN / 2;

    seed_pool(&w);
    set_prices(&w, 1);

    StellarAssetClient::new(&w.env, &w.usdc_tk).mint(&w.user, &collateral_amount);
    StellarAssetClient::new(&w.env, &w.xlm_tk).mint(&w.user, &execution_fee);

    let user_usdc_before = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);
    let user_xlm_before = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.user);
    let keeper_xlm_before = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.keeper);

    let key = create_long_increase_order(
        &w,
        collateral_amount,
        execution_fee,
        OrderType::LimitIncrease,
        ONE_USD / 2,
    );

    let result = OHClient::new(&w.env, &w.ord_handler).try_execute_order(&w.keeper, &key);
    assert!(result.is_err(), "keeper execution must fail before cancellation");

    OHClient::new(&w.env, &w.ord_handler).cancel_order(&w.keeper, &key);

    let user_usdc_after = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);
    let user_xlm_after = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.user);
    let keeper_xlm_after = StellarAssetClient::new(&w.env, &w.xlm_tk).balance(&w.keeper);
    let ds_c = DsClient::new(&w.env, &w.ds);

    assert_eq!(
        user_usdc_after,
        user_usdc_before,
        "user collateral refund must match the exact USDC deposit"
    );
    assert_eq!(
        user_xlm_after,
        user_xlm_before - execution_fee,
        "user should pay only the execution fee on keeper-triggered cancellation"
    );
    assert_eq!(
        keeper_xlm_after,
        keeper_xlm_before + execution_fee,
        "keeper must receive the full execution fee after failed execution"
    );
    assert!(
        OHClient::new(&w.env, &w.ord_handler).get_order(&key).is_none(),
        "order entry must be removed after keeper cancellation"
    );
    assert!(
        !ds_c.contains_bytes32(&order_list_key(&w.env), &key),
        "global order list must be cleared after failed execution cancellation"
    );
    assert!(
        !ds_c.contains_bytes32(&account_order_list_key(&w.env, &w.user), &key),
        "account order list must be cleared after failed execution cancellation"
    );
}

#[test]
fn cancel_refunds_partial_collateral_without_rounding_loss() {
    let w = setup();
    let collateral_amount = 123_456_789i128;

    StellarAssetClient::new(&w.env, &w.usdc_tk).mint(&w.user, &collateral_amount);
    let usdc_before = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);

    let key = create_long_increase_order(
        &w,
        collateral_amount,
        0,
        OrderType::MarketIncrease,
        0,
    );

    OHClient::new(&w.env, &w.ord_handler).cancel_order(&w.user, &key);

    let usdc_after = StellarAssetClient::new(&w.env, &w.usdc_tk).balance(&w.user);
    assert_eq!(
        usdc_after,
        usdc_before,
        "partial collateral refund must match the exact deposited amount"
    );
}
