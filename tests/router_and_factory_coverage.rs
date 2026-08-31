//! Integration coverage for exchange_router and market_factory — issue #610.
//!
//! Before this file, the `tests/` integration crate did not declare
//! exchange-router, market-factory, insurance-fund-router, fee-batch-sweeper,
//! or referral-storage as dependencies at all, so no integration test could
//! reference the two contracts real users/frontends actually call in
//! production: exchange_router (the documented single user-facing entry
//! point) and market_factory (the only contract that deploys a market
//! end-to-end). Every other lifecycle test in this crate opens positions and
//! moves funds by calling order_handler/deposit_handler/withdrawal_handler
//! directly and seeds market registration straight into data_store, bypassing
//! both contracts entirely.
//!
//! This file adds one test per gap:
//!   - `market_factory_create_market_deploys_real_market_token`: exercises
//!     market_factory::create_market's actual successful-deploy path,
//!     including a real market_token WASM upload (not the native-Rust
//!     `env.register` shortcut every other test in this crate uses for LP
//!     tokens).
//!   - `exchange_router_multicall_opens_position`: exercises
//!     exchange_router::multicall's SendTokens + CreateOrder actions to open
//!     a real position, routing through the router instead of calling
//!     order_handler directly.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use exchange_router::{ExchangeRouter, ExchangeRouterClient, RouterAction, SendTokensParams};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, position_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, MarketProps, OrderType, TokenPrice};
use market_factory::{MarketFactory, MarketFactoryClient};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

const ONE_TOKEN: i128 = 10_000_000; // 7-decimal Stellar precision
const ONE_USD: i128 = FLOAT_PRECISION;

// Real compiled market_token WASM — built via `stellar contract build
// --package market-token` before running this test. market_factory's own
// unit tests explicitly avoid this (see the comment on
// create_market_duplicate_token_triple_panics in
// contracts/market_factory/src/lib.rs), so this is the first place in the
// workspace that exercises create_market's real deploy path.
mod market_token_wasm {
    soroban_sdk::contractimport!(file = "../target/wasm32v1-none/release/market_token.wasm");
}

#[test]
fn market_factory_create_market_deploys_real_market_token() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &admin, &roles::market_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let factory = env.register(MarketFactory, ());
    let factory_client = MarketFactoryClient::new(&env, &factory);
    factory_client.initialize(&admin, &rs, &ds);
    rs_c.grant_role(&admin, &factory, &roles::controller(&env));

    let wasm_hash = env.deployer().upload_contract_wasm(market_token_wasm::WASM);
    factory_client.set_market_token_wasm_hash(&admin, &wasm_hash);

    // create_market queries decimals() on each token via a real SEP-41
    // token::Client call, so all three must be real token contracts, not
    // bare placeholder addresses.
    let index_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let market_type = soroban_sdk::BytesN::from_array(&env, &[1u8; 32]);

    assert_eq!(factory_client.get_market_count(), 0);

    let market: MarketProps =
        factory_client.create_market(&admin, &index_tk, &long_tk, &short_tk, &market_type);

    assert_eq!(market.index_token, index_tk);
    assert_eq!(market.long_token, long_tk);
    assert_eq!(market.short_token, short_tk);

    // The deployed market_token must be a real, callable contract — not a
    // placeholder address — confirmed by reading back its SEP-41 metadata.
    let mt_client = MtClient::new(&env, &market.market_token);
    assert_eq!(mt_client.total_supply(), 0);

    assert_eq!(factory_client.get_market_count(), 1);
    let markets = factory_client.get_markets(&0, &10);
    assert_eq!(markets.len(), 1);
    assert_eq!(markets.get(0).unwrap(), market.market_token);

    // create_market's own registration into data_store must also be visible
    // to the same keys handlers rely on to reconstruct MarketProps.
    let ds_c = DsClient::new(&env, &ds);
    assert_eq!(
        ds_c.get_address(&market_index_token_key(&env, &market.market_token)),
        Some(index_tk)
    );
}

#[test]
fn exchange_router_multicall_opens_position() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let trader = Address::generate(&env);

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

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "SO4 Market"),
        &soroban_sdk::String::from_str(&env, "GM"),
        &long_tk,
        &short_tk,
    );

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Deposit/withdrawal handlers aren't exercised by this test but
    // exchange_router::initialize requires an address for each; a dummy
    // handler address is fine since multicall only dispatches to the
    // handler(s) the actions in this call actually target.
    let dummy_handler = Address::generate(&env);
    let router = env.register(ExchangeRouter, ());
    ExchangeRouterClient::new(&env, &router).initialize(
        &admin,
        &rs,
        &ds,
        &dummy_handler,
        &dummy_handler,
        &ord_handler,
        &dummy_handler,
    );

    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &router, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    OClient::new(&env, &oracle_addr).set_prices_simple(
        &keeper,
        &Vec::from_array(
            &env,
            [
                TokenPrice { token: long_tk.clone(), min: 2_000 * ONE_USD, max: 2_000 * ONE_USD },
                TokenPrice { token: short_tk.clone(), min: ONE_USD, max: ONE_USD },
                TokenPrice { token: index_tk.clone(), min: 2_000 * ONE_USD, max: 2_000 * ONE_USD },
            ],
        ),
    );

    let collateral = 2 * ONE_TOKEN; // 2 tokens ≈ $4 000 → 1x leverage on a $2 000 open
    StellarAssetClient::new(&env, &long_tk).mint(&trader, &collateral);

    // Route the deposit + order creation through the real router entrypoint
    // (RouterAction::SendTokens + RouterAction::CreateOrder), not a direct
    // order_vault/order_handler call.
    let router_client = ExchangeRouterClient::new(&env, &router);
    let keys = router_client.multicall(
        &trader,
        &Vec::from_array(
            &env,
            [
                RouterAction::SendTokens(SendTokensParams {
                    token: long_tk.clone(),
                    receiver: ord_vault.clone(),
                    amount: collateral,
                }),
                RouterAction::CreateOrder(CreateOrderParams {
                    receiver: trader.clone(),
                    market: market_tk.clone(),
                    initial_collateral_token: long_tk.clone(),
                    swap_path: Vec::new(&env),
                    size_delta_usd: 2_000 * ONE_USD,
                    collateral_delta_amount: collateral,
                    trigger_price: 0,
                    acceptable_price: 0,
                    execution_fee: 0,
                    min_output_amount: 0,
                    order_type: OrderType::MarketIncrease,
                    is_long: true,
                    expiry_ledger: None,
                    on_behalf_of: None,
                }),
            ],
        ),
    );

    let order_key = keys.get(1).unwrap();
    let hc = OHClient::new(&env, &ord_handler);
    assert!(
        hc.get_order(&order_key).is_some(),
        "order created via multicall must exist in order_handler"
    );

    hc.execute_order(&keeper, &order_key);

    let pos_key = position_key(&env, &trader, &market_tk, &long_tk, true);
    let position = hc
        .get_position(&pos_key)
        .expect("position must exist after executing the order created via multicall");
    assert_eq!(position.size_in_usd, 2_000 * ONE_USD);
}
