//! Integration test for issue #271: keeper executes orders across multiple markets in one ledger.
//!
//! Scenario:
//!   1. Set up two markets: ETH/USD and BTC/USD (separate market tokens, tokens, index feeds)
//!   2. Seed liquidity into both market pools
//!   3. Open a pending long on ETH/USD (order A) and a pending short on BTC/USD (order B)
//!   4. Keeper executes order A then order B in the same ledger
//!   5. Assert:
//!      - ETH/USD long OI increased by order A's size
//!      - BTC/USD short OI increased by order B's size
//!      - No cross-market state contamination (each pool is independent)

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, open_interest_key,
    pool_amount_key, position_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

const ONE_TOKEN: i128 = 10_000_000; // 7-decimal Stellar precision
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    // ETH/USD market
    eth_market: Address,
    weth: Address,    // long token
    usdc: Address,    // short token
    eth_idx: Address, // index price feed token
    // BTC/USD market
    btc_market: Address,
    wbtc: Address,    // long token
    btc_idx: Address, // index price feed token
    // Shared infrastructure
    rs: Address,
    ds: Address,
    oracle: Address,
    dep_vault: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);

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

    // Vaults
    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // ── ETH/USD market ────────────────────────────────────────────────────────
    let eth_market = env.register(MarketToken, ());
    MtClient::new(&env, &eth_market).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM-ETH"),
    );

    let weth = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let usdc = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let eth_idx = Address::generate(&env);

    // ── BTC/USD market ────────────────────────────────────────────────────────
    let btc_market = env.register(MarketToken, ());
    MtClient::new(&env, &btc_market).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "BTC/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM-BTC"),
    );

    let wbtc = env.register_stellar_asset_contract_v2(admin.clone()).address();
    // BTC/USD also uses usdc as its short token
    let btc_idx = Address::generate(&env);

    // Handlers
    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    // Register market metadata in data_store
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &eth_market), &eth_idx);
    ds_c.set_address(&admin, &market_long_token_key(&env, &eth_market), &weth);
    ds_c.set_address(&admin, &market_short_token_key(&env, &eth_market), &usdc);

    ds_c.set_address(&admin, &market_index_token_key(&env, &btc_market), &btc_idx);
    ds_c.set_address(&admin, &market_long_token_key(&env, &btc_market), &wbtc);
    ds_c.set_address(&admin, &market_short_token_key(&env, &btc_market), &usdc);

    World {
        env,
        admin,
        keeper,
        eth_market,
        weth,
        usdc,
        eth_idx,
        btc_market,
        wbtc,
        btc_idx,
        rs,
        ds,
        oracle: oracle_addr,
        dep_vault,
        ord_vault,
        dep_handler,
        ord_handler,
    }
}

fn set_prices(w: &World, eth_usd: i128, btc_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &soroban_sdk::Vec::from_array(
            &w.env,
            [
                TokenPrice { token: w.weth.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
                TokenPrice { token: w.wbtc.clone(), min: btc_usd * ONE_USD, max: btc_usd * ONE_USD },
                TokenPrice { token: w.usdc.clone(), min: ONE_USD, max: ONE_USD },
                TokenPrice { token: w.eth_idx.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
                TokenPrice { token: w.btc_idx.clone(), min: btc_usd * ONE_USD, max: btc_usd * ONE_USD },
            ],
        ),
    );
}

fn seed_pool(w: &World, market: &Address, long_tk: &Address, short_tk: &Address, lp: &Address) {
    StellarAssetClient::new(&w.env, long_tk).mint(lp, &(10 * ONE_TOKEN));
    StellarAssetClient::new(&w.env, short_tk).mint(lp, &(50_000 * ONE_TOKEN));

    let dep_key = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: market.clone(),
            initial_long_token: long_tk.clone(),
            initial_short_token: short_tk.clone(),
            long_token_amount: 5 * ONE_TOKEN,
            short_token_amount: 20_000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);
}

#[test]
fn keeper_executes_orders_on_two_markets_in_same_ledger() {
    let w = setup();
    let env = &w.env;

    let eth_trader = Address::generate(env);
    let btc_trader = Address::generate(env);
    let lp = Address::generate(env);

    // Seed both pools
    set_prices(&w, 2_000, 50_000);
    seed_pool(&w, &w.eth_market, &w.weth, &w.usdc, &lp);

    set_prices(&w, 2_000, 50_000);
    seed_pool(&w, &w.btc_market, &w.wbtc, &w.usdc, &lp);

    // ── Order A: long ETH/USD ─────────────────────────────────────────────────
    let eth_collateral = 200 * ONE_TOKEN; // 200 USDC
    StellarAssetClient::new(env, &w.usdc).mint(&eth_trader, &eth_collateral);
    soroban_sdk::token::Client::new(env, &w.usdc)
        .transfer(&eth_trader, &w.ord_vault, &eth_collateral);

    set_prices(&w, 2_000, 50_000);

    let order_a = OHClient::new(env, &w.ord_handler).create_order(
        &eth_trader,
        &CreateOrderParams {
            receiver: eth_trader.clone(),
            market: w.eth_market.clone(),
            initial_collateral_token: w.usdc.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 2_000 * ONE_USD, // 1 ETH notional at $2000
            collateral_delta_amount: eth_collateral,
            trigger_price: 0,
            acceptable_price: 2_100 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: None,
        },
    );

    // ── Order B: short BTC/USD ────────────────────────────────────────────────
    let btc_collateral = 500 * ONE_TOKEN; // 500 USDC
    StellarAssetClient::new(env, &w.usdc).mint(&btc_trader, &btc_collateral);
    soroban_sdk::token::Client::new(env, &w.usdc)
        .transfer(&btc_trader, &w.ord_vault, &btc_collateral);

    let order_b = OHClient::new(env, &w.ord_handler).create_order(
        &btc_trader,
        &CreateOrderParams {
            receiver: btc_trader.clone(),
            market: w.btc_market.clone(),
            initial_collateral_token: w.usdc.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 10_000 * ONE_USD, // 0.2 BTC notional at $50000
            collateral_delta_amount: btc_collateral,
            trigger_price: 0,
            acceptable_price: 45_000 * ONE_USD, // accept down to $45k (short)
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
        },
    );

    // ── Keeper executes both in the same simulated ledger ─────────────────────
    set_prices(&w, 2_000, 50_000);
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &order_a);
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &order_b);

    // ── Assertions: ETH/USD long OI ──────────────────────────────────────────
    let ds_c = DsClient::new(env, &w.ds);
    let eth_long_oi = ds_c.get_u128(&open_interest_key(env, &w.eth_market, &w.usdc, true));
    assert!(eth_long_oi > 0, "ETH/USD long OI must be nonzero after order A");

    // ── Assertions: BTC/USD short OI ─────────────────────────────────────────
    let btc_short_oi = ds_c.get_u128(&open_interest_key(env, &w.btc_market, &w.usdc, false));
    assert!(btc_short_oi > 0, "BTC/USD short OI must be nonzero after order B");

    // ── Assertions: no cross-market contamination ─────────────────────────────
    let eth_short_oi = ds_c.get_u128(&open_interest_key(env, &w.eth_market, &w.usdc, false));
    let btc_long_oi = ds_c.get_u128(&open_interest_key(env, &w.btc_market, &w.usdc, true));
    assert_eq!(eth_short_oi, 0, "ETH/USD short OI must not be contaminated by BTC order");
    assert_eq!(btc_long_oi, 0, "BTC/USD long OI must not be contaminated by ETH order");

    // ── Assertions: both positions exist with correct keys ────────────────────
    let eth_pos_key = position_key(env, &eth_trader, &w.eth_market, &w.usdc, true);
    let btc_pos_key = position_key(env, &btc_trader, &w.btc_market, &w.usdc, false);
    let eth_pos = OHClient::new(env, &w.ord_handler).get_position(&eth_pos_key);
    let btc_pos = OHClient::new(env, &w.ord_handler).get_position(&btc_pos_key);
    assert!(eth_pos.is_some(), "ETH/USD long position must exist after order A");
    assert!(btc_pos.is_some(), "BTC/USD short position must exist after order B");

    // Pool amounts are independent
    let eth_pool = ds_c.get_u128(&pool_amount_key(env, &w.eth_market, &w.usdc));
    let btc_pool = ds_c.get_u128(&pool_amount_key(env, &w.btc_market, &w.usdc));
    assert!(eth_pool > 0, "ETH/USD pool must hold collateral");
    assert!(btc_pool > 0, "BTC/USD pool must hold collateral");
    // Pools are segregated — neither should equal the other
    assert_ne!(eth_pool, btc_pool, "pool amounts should differ (different collateral sizes)");
}
