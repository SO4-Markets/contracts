// Integration tests for multi-hop swap routing — issue #190.
//
// Scenario: WETH → (market1: WETH/USDC) → USDC → (market2: WBTC/USDC) → WBTC
//
// Verifies:
//   - Two-hop MarketSwap delivers token_out to receiver
//   - market1 WETH pool increases; market2 WBTC pool decreases
//   - min_output_amount guard reverts if output is too small
//   - Duplicate market in swap_path reverts

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::roles;
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_types::{CreateDepositParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{CreateOrderParams, OrderHandler, OrderHandlerClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Env, Vec,
};

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    user: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    dep_vault: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    market1_tk: Address,
    weth: Address,
    usdc: Address,
    market2_tk: Address,
    wbtc: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();

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

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let dep_handler = env.register(DepositHandler, ());
    DepositHandlerClient::new(&env, &dep_handler)
        .initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));

    let ord_handler = env.register(OrderHandler, ());
    OrderHandlerClient::new(&env, &ord_handler)
        .initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let weth = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let usdc = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let wbtc = env.register_stellar_asset_contract_v2(admin.clone()).address();

    // Market 1: WETH/USDC — long=WETH, short=USDC
    let market1_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market1_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "WETH-USDC Market"),
        &soroban_sdk::String::from_str(&env, "GM1"),
    );
    rs_c.grant_role(&admin, &market1_tk, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market1_tk), &weth);
    ds_c.set_address(&admin, &gmx_keys::market_long_token_key(&env, &market1_tk), &weth);
    ds_c.set_address(&admin, &gmx_keys::market_short_token_key(&env, &market1_tk), &usdc);

    // Market 2: WBTC/USDC — long=WBTC, short=USDC
    let market2_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market2_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "WBTC-USDC Market"),
        &soroban_sdk::String::from_str(&env, "GM2"),
    );
    rs_c.grant_role(&admin, &market2_tk, &roles::controller(&env));

    ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market2_tk), &wbtc);
    ds_c.set_address(&admin, &gmx_keys::market_long_token_key(&env, &market2_tk), &wbtc);
    ds_c.set_address(&admin, &gmx_keys::market_short_token_key(&env, &market2_tk), &usdc);

    World {
        env,
        admin,
        keeper,
        user,
        rs,
        ds,
        oracle: oracle_addr,
        dep_vault,
        ord_vault,
        dep_handler,
        ord_handler,
        market1_tk,
        weth,
        usdc,
        market2_tk,
        wbtc,
    }
}

fn set_prices(w: &World) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice { token: w.weth.clone(), min: 2000 * FLOAT_PRECISION, max: 2000 * FLOAT_PRECISION },
                TokenPrice { token: w.usdc.clone(), min: FLOAT_PRECISION, max: FLOAT_PRECISION },
                TokenPrice { token: w.wbtc.clone(), min: 30000 * FLOAT_PRECISION, max: 30000 * FLOAT_PRECISION },
            ],
        ),
    );
}

fn seed_market1(w: &World) {
    let lp = Address::generate(&w.env);
    // 1000 WETH ($2M) + 2,000,000 USDC ($2M) — balanced by USD value
    StellarAssetClient::new(&w.env, &w.weth).mint(&lp, &(1000 * TOKEN_PRECISION));
    StellarAssetClient::new(&w.env, &w.usdc).mint(&lp, &(2_000_000 * TOKEN_PRECISION));
    let k = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market1_tk.clone(),
            initial_long_token: w.weth.clone(),
            initial_short_token: w.usdc.clone(),
            long_token_amount: 1000 * TOKEN_PRECISION,
            short_token_amount: 2_000_000 * TOKEN_PRECISION,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

fn seed_market2(w: &World) {
    let lp = Address::generate(&w.env);
    // 100 WBTC ($3M) + 3,000,000 USDC ($3M) — balanced by USD value
    StellarAssetClient::new(&w.env, &w.wbtc).mint(&lp, &(100 * TOKEN_PRECISION));
    StellarAssetClient::new(&w.env, &w.usdc).mint(&lp, &(3_000_000 * TOKEN_PRECISION));
    let k = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market2_tk.clone(),
            initial_long_token: w.wbtc.clone(),
            initial_short_token: w.usdc.clone(),
            long_token_amount: 100 * TOKEN_PRECISION,
            short_token_amount: 3_000_000 * TOKEN_PRECISION,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Two-hop WETH → USDC → WBTC swap delivers WBTC to the receiver.
/// Verifies pool state changes: market1 WETH pool grew, market2 WBTC pool shrank.
#[test]
fn two_hop_swap_weth_to_wbtc() {
    let w = setup();
    set_prices(&w);
    seed_market1(&w);
    seed_market2(&w);

    let collateral = 10 * TOKEN_PRECISION; // 10 WETH ≈ $20,000
    StellarAssetClient::new(&w.env, &w.weth).mint(&w.ord_vault, &collateral);

    let weth_pool_before = DsClient::new(&w.env, &w.ds)
        .get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market1_tk, &w.weth));
    let wbtc_pool_before = DsClient::new(&w.env, &w.ds)
        .get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market2_tk, &w.wbtc));

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market1_tk.clone(),
            initial_collateral_token: w.weth.clone(),
            swap_path: Vec::from_array(&w.env, [w.market1_tk.clone(), w.market2_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );

    set_prices(&w);
    hc.execute_order(&w.keeper, &key);

    // User receives WBTC
    let wbtc_received = soroban_sdk::token::Client::new(&w.env, &w.wbtc).balance(&w.user);
    assert!(wbtc_received > 0, "user should receive WBTC after two-hop swap");

    // market1 WETH pool grew (WETH went in)
    let weth_pool_after = DsClient::new(&w.env, &w.ds)
        .get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market1_tk, &w.weth));
    assert!(
        weth_pool_after > weth_pool_before,
        "market1 WETH pool must increase after receiving WETH input"
    );

    // market2 WBTC pool shrank (WBTC went out)
    let wbtc_pool_after = DsClient::new(&w.env, &w.ds)
        .get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market2_tk, &w.wbtc));
    assert!(
        wbtc_pool_after < wbtc_pool_before,
        "market2 WBTC pool must decrease after sending WBTC output"
    );

    // Order is removed after execution
    assert!(hc.get_order(&key).is_none(), "order must be removed after execution");
}

/// min_output_amount guard: if swap output < min_output_amount, execution reverts.
#[test]
#[should_panic]
fn two_hop_swap_min_output_not_met_reverts() {
    let w = setup();
    set_prices(&w);
    seed_market1(&w);
    seed_market2(&w);

    let collateral = TOKEN_PRECISION; // 1 WETH ≈ $2,000 → ≈ 0.0000667 WBTC
    StellarAssetClient::new(&w.env, &w.weth).mint(&w.ord_vault, &collateral);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market1_tk.clone(),
            initial_collateral_token: w.weth.clone(),
            swap_path: Vec::from_array(&w.env, [w.market1_tk.clone(), w.market2_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 1_000 * TOKEN_PRECISION, // unreachably high
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );

    set_prices(&w);
    hc.execute_order(&w.keeper, &key);
}

/// Duplicate market in swap_path is rejected during execution.
#[test]
#[should_panic]
fn swap_duplicate_market_in_path_reverts() {
    let w = setup();
    set_prices(&w);
    seed_market1(&w);

    let collateral = TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.weth).mint(&w.ord_vault, &collateral);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market1_tk.clone(),
            initial_collateral_token: w.weth.clone(),
            swap_path: Vec::from_array(&w.env, [w.market1_tk.clone(), w.market1_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );

    set_prices(&w);
    hc.execute_order(&w.keeper, &key); // must panic: duplicate market
}

/// Single-hop WETH → USDC swap succeeds and delivers USDC to receiver.
#[test]
fn single_hop_swap_weth_to_usdc() {
    let w = setup();
    set_prices(&w);
    seed_market1(&w);

    let collateral = 5 * TOKEN_PRECISION; // 5 WETH
    StellarAssetClient::new(&w.env, &w.weth).mint(&w.ord_vault, &collateral);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market1_tk.clone(),
            initial_collateral_token: w.weth.clone(),
            swap_path: Vec::from_array(&w.env, [w.market1_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );

    set_prices(&w);
    hc.execute_order(&w.keeper, &key);

    let usdc_received = soroban_sdk::token::Client::new(&w.env, &w.usdc).balance(&w.user);
    assert!(usdc_received > 0, "user should receive USDC after single-hop swap");
}
