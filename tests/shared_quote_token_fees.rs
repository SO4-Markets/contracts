//! End-to-end: two markets that share a quote token accrue protocol fees in
//! that token through real order settlement without overwriting each other.
//!
//! Both markets are ETH/USD with the same long and short (quote) tokens. Each is
//! its own `market_token` contract holding its own custody, and each settles a
//! short with quote-token collateral, so both fee ledgers are denominated in
//! the same token. The fees stay attributed to `claimable_fee_amount(market,
//! token)` per market, the token-level view sums them without double counting,
//! and a token-level sweep pays each market's share out of that market's own
//! pool.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient, MarketFeeAmount};
use gmx_keys::roles;
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    fee_keeper: Address,
    treasury: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    fee_handler: Address,
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
    let fee_keeper = Address::generate(&env);
    let treasury = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
    rs_c.grant_role(&admin, &fee_keeper, &roles::fee_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle, &dep_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &ord_vault);

    let fee_handler = env.register(FeeHandler, ());
    FHClient::new(&env, &fee_handler).initialize(&admin, &rs, &ds);
    FHClient::new(&env, &fee_handler).set_fee_receiver(&treasury);

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &fee_handler, &roles::controller(&env));

    World {
        env,
        admin,
        keeper,
        fee_keeper,
        treasury,
        rs,
        ds,
        oracle,
        ord_vault,
        dep_handler,
        ord_handler,
        fee_handler,
        long_tk,
        short_tk,
        index_tk,
    }
}

/// Register an ETH/USD market over the shared long/short tokens with a 10 bps
/// position fee, seeded with its own liquidity.
fn add_market(w: &World, symbol: &str) -> Address {
    let env = &w.env;
    let market = env.register(MarketToken, ());
    MtClient::new(env, &market).initialize(
        &w.admin,
        &w.rs,
        &7u32,
        &soroban_sdk::String::from_str(env, "GMX ETH/USD Market"),
        &soroban_sdk::String::from_str(env, symbol),
        &w.long_tk,
        &w.short_tk,
    );
    RsClient::new(env, &w.rs).grant_role(&w.admin, &market, &roles::controller(env));

    let ds_c = DsClient::new(env, &w.ds);
    ds_c.set_address(
        &w.admin,
        &gmx_keys::market_index_token_key(env, &market),
        &w.index_tk,
    );
    ds_c.set_address(
        &w.admin,
        &gmx_keys::market_long_token_key(env, &market),
        &w.long_tk,
    );
    ds_c.set_address(
        &w.admin,
        &gmx_keys::market_short_token_key(env, &market),
        &w.short_tk,
    );
    let fee_factor = (FLOAT_PRECISION / 1_000) as u128;
    for positive in [true, false] {
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(env, &market, positive),
            &fee_factor,
        );
    }

    set_prices(w, 2000);
    let lp = Address::generate(env);
    StellarAssetClient::new(env, &w.long_tk).mint(&lp, &(10 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).mint(&lp, &(50_000 * ONE_TOKEN));
    let dep_key = DHClient::new(env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: market.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 10 * ONE_TOKEN,
            short_token_amount: 50_000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);
    market
}

fn set_prices(w: &World, eth_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice {
                    token: w.long_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
                TokenPrice {
                    token: w.short_tk.clone(),
                    min: ONE_USD,
                    max: ONE_USD,
                },
                TokenPrice {
                    token: w.index_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
            ],
        ),
    );
}

/// Execute a short of `size_usd` in `market` with quote-token collateral, so
/// its position fee accrues in the shared quote token.
fn open_short(w: &World, market: &Address, size_usd: i128) {
    let env = &w.env;
    set_prices(w, 2000);
    let trader = Address::generate(env);
    let collateral = 1_000 * ONE_TOKEN;
    StellarAssetClient::new(env, &w.short_tk).mint(&trader, &collateral);
    StellarAssetClient::new(env, &w.short_tk).transfer(&trader, &w.ord_vault, &collateral);
    let key = OHClient::new(env, &w.ord_handler).create_order(
        &trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: market.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: size_usd * ONE_USD,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
            on_behalf_of: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &key);
}

fn pool_amount(w: &World, market: &Address) -> u128 {
    DsClient::new(&w.env, &w.ds).get_u128(&gmx_keys::pool_amount_key(&w.env, market, &w.short_tk))
}

fn pair(w: &World, a: &Address, b: &Address) -> Vec<Address> {
    Vec::from_array(&w.env, [a.clone(), b.clone()])
}

#[test]
fn markets_sharing_quote_token_accrue_fees_without_overwriting() {
    let w = setup();
    let fh = FHClient::new(&w.env, &w.fee_handler);
    let market_a = add_market(&w, "GMA");
    let market_b = add_market(&w, "GMB");

    // 8_000 USD * 10 bps = 8 USD of quote token at 1 USD/token.
    open_short(&w, &market_a, 8_000);
    let fee_a = fh.claimable_fees(&market_a, &w.short_tk);
    assert_eq!(fee_a, (8 * ONE_TOKEN) as u128);
    assert_eq!(fh.claimable_fees(&market_b, &w.short_tk), 0);

    // A later settlement in market B accrues to B only; A's ledger is untouched.
    open_short(&w, &market_b, 4_000);
    let fee_b = fh.claimable_fees(&market_b, &w.short_tk);
    assert_eq!(fee_b, (4 * ONE_TOKEN) as u128);
    assert_eq!(fh.claimable_fees(&market_a, &w.short_tk), fee_a);

    // The token-level view attributes each share and sums them once.
    let view = fh.claimable_fees_by_token(&w.short_tk, &pair(&w, &market_a, &market_b));
    assert_eq!(view.total, fee_a + fee_b);
    assert_eq!(
        view.markets,
        Vec::from_array(
            &w.env,
            [
                MarketFeeAmount {
                    market: market_a.clone(),
                    amount: fee_a,
                },
                MarketFeeAmount {
                    market: market_b.clone(),
                    amount: fee_b,
                },
            ],
        )
    );
}

#[test]
fn token_level_sweep_pays_each_market_from_its_own_pool() {
    let w = setup();
    let fh = FHClient::new(&w.env, &w.fee_handler);
    let quote = TokenClient::new(&w.env, &w.short_tk);
    let market_a = add_market(&w, "GMA");
    let market_b = add_market(&w, "GMB");
    let partner = Address::generate(&w.env);
    fh.set_market_fee_receiver(&market_b, &Some(partner.clone()));

    open_short(&w, &market_a, 8_000);
    open_short(&w, &market_b, 4_000);
    let fee_a = fh.claimable_fees(&market_a, &w.short_tk);
    let fee_b = fh.claimable_fees(&market_b, &w.short_tk);
    let (pool_a, pool_b) = (pool_amount(&w, &market_a), pool_amount(&w, &market_b));
    let (custody_a, custody_b) = (quote.balance(&market_a), quote.balance(&market_b));

    let result = fh.claim_token_fees(&w.fee_keeper, &w.short_tk, &pair(&w, &market_a, &market_b));

    assert_eq!(result.total, fee_a + fee_b);
    assert_eq!(quote.balance(&w.treasury), fee_a as i128);
    assert_eq!(quote.balance(&partner), fee_b as i128);
    assert_eq!(quote.balance(&market_a), custody_a - fee_a as i128);
    assert_eq!(quote.balance(&market_b), custody_b - fee_b as i128);
    assert_eq!(pool_amount(&w, &market_a), pool_a - fee_a);
    assert_eq!(pool_amount(&w, &market_b), pool_b - fee_b);
    assert_eq!(fh.claimable_fees(&market_a, &w.short_tk), 0);
    assert_eq!(fh.claimable_fees(&market_b, &w.short_tk), 0);

    // Listing a market twice cannot double count its ledger.
    open_short(&w, &market_a, 8_000);
    assert!(fh
        .try_claim_token_fees(&w.fee_keeper, &w.short_tk, &pair(&w, &market_a, &market_a))
        .is_err());
    assert_eq!(fh.claimable_fees(&market_a, &w.short_tk), fee_a);
    assert_eq!(fh.total_claimable_fees(&w.short_tk), fee_a);
}

/// Settlement keeps the global fee total for the shared token equal to the
/// sum of the per-market ledgers, and the reconciliation API agrees with
/// custody and pool accounting before and after the token-level sweep.
#[test]
fn global_fee_total_reconciles_with_markets_through_settlement_and_sweep() {
    let w = setup();
    let fh = FHClient::new(&w.env, &w.fee_handler);
    let market_a = add_market(&w, "GMA");
    let market_b = add_market(&w, "GMB");
    let both = pair(&w, &market_a, &market_b);

    open_short(&w, &market_a, 8_000);
    open_short(&w, &market_b, 4_000);
    let fee_a = fh.claimable_fees(&market_a, &w.short_tk);
    let fee_b = fh.claimable_fees(&market_b, &w.short_tk);
    assert!(fee_a > 0 && fee_b > 0);

    assert_eq!(fh.total_claimable_fees(&w.short_tk), fee_a + fee_b);
    let rec = fh.reconcile_token_fees(&w.short_tk, &both);
    assert!(rec.reconciled);
    assert_eq!(rec.ledger_total, fee_a + fee_b);
    assert_eq!(rec.markets_total, fee_a + fee_b);
    for row in rec.markets.iter() {
        assert!(row.backed);
        assert!(row.pool_amount >= row.claimable);
    }

    // Omitting a market shows the shortfall instead of a false balance.
    let partial =
        fh.reconcile_token_fees(&w.short_tk, &Vec::from_array(&w.env, [market_a.clone()]));
    assert!(!partial.reconciled);
    assert_eq!(partial.ledger_total - partial.markets_total, fee_b);

    let swept = fh.claim_token_fees(&w.fee_keeper, &w.short_tk, &both);
    assert_eq!(swept.total, fee_a + fee_b);
    assert_eq!(fh.total_claimable_fees(&w.short_tk), 0);
    let rec = fh.reconcile_token_fees(&w.short_tk, &both);
    assert!(rec.reconciled);
    assert_eq!((rec.ledger_total, rec.markets_total), (0, 0));
}
