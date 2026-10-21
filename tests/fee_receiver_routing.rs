//! End-to-end: protocol fees accrued by real order settlement are paid only to
//! the trusted recipient configured on `fee_handler`, never to any address an
//! order names.
//!
//! An order whose `receiver` is an attacker-chosen address is executed with a
//! non-zero position fee. The fee accrues to `claimable_fee_amount(market,
//! token)` in lockstep with `pool_amount`, and a FEE_KEEPER sweep then pays it
//! to the configured treasury (or the market override) — the order's receiver,
//! the account and the keeper all receive nothing.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient};
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
    keeper: Address,
    fee_keeper: Address,
    treasury: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    fee_handler: Address,
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

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM"),
        &long_tk,
        &short_tk,
    );

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
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(
        &admin,
        &gmx_keys::market_index_token_key(&env, &market_tk),
        &index_tk,
    );
    ds_c.set_address(
        &admin,
        &gmx_keys::market_long_token_key(&env, &market_tk),
        &long_tk,
    );
    ds_c.set_address(
        &admin,
        &gmx_keys::market_short_token_key(&env, &market_tk),
        &short_tk,
    );
    // 10 bps position fee on both impact sides so execution accrues protocol fees.
    let fee_factor = (FLOAT_PRECISION / 1_000) as u128;
    for positive in [true, false] {
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_fee_factor_key(&env, &market_tk, positive),
            &fee_factor,
        );
    }

    World {
        env,
        keeper,
        fee_keeper,
        treasury,
        ds,
        oracle,
        ord_vault,
        dep_handler,
        ord_handler,
        fee_handler,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
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

/// Seed liquidity, then execute a long whose order names `order_receiver`.
fn open_long_naming_receiver(w: &World, trader: &Address, order_receiver: &Address) {
    let env = &w.env;
    set_prices(w, 2000);

    let lp = Address::generate(env);
    StellarAssetClient::new(env, &w.long_tk).mint(&lp, &(10 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).mint(&lp, &(50_000 * ONE_TOKEN));
    let dep_key = DHClient::new(env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 10 * ONE_TOKEN,
            short_token_amount: 50_000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);

    StellarAssetClient::new(env, &w.long_tk).mint(trader, &ONE_TOKEN);
    StellarAssetClient::new(env, &w.long_tk).transfer(trader, &w.ord_vault, &ONE_TOKEN);
    let key = OHClient::new(env, &w.ord_handler).create_order(
        trader,
        &CreateOrderParams {
            receiver: order_receiver.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 8_000 * ONE_USD,
            collateral_delta_amount: ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 2_100 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: None,
            on_behalf_of: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &key);
}

#[test]
fn order_receiver_cannot_redirect_protocol_fees() {
    let w = setup();
    let env = &w.env;
    let trader = Address::generate(env);
    let attacker = Address::generate(env);
    let fh = FHClient::new(env, &w.fee_handler);
    let ds_c = DsClient::new(env, &w.ds);
    let tk = TokenClient::new(env, &w.long_tk);

    open_long_naming_receiver(&w, &trader, &attacker);

    // Settlement accrued the fee to trusted (market, token) state and backed it
    // with pool accounting in the same execution.
    let claimable = fh.claimable_fees(&w.market_tk, &w.long_tk);
    // 8_000 USD * 10 bps = 8 USD at 2_000 USD/token = 0.004 token.
    assert_eq!(claimable, (ONE_TOKEN / 250) as u128);
    let pool_key = gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk);
    let pool_before = ds_c.get_u128(&pool_key);
    assert!(pool_before >= claimable);

    let attacker_before = tk.balance(&attacker);
    let trader_before = tk.balance(&trader);

    let claimed = fh.claim_fees(&w.fee_keeper, &w.market_tk, &w.long_tk);

    assert_eq!(claimed, claimable);
    assert_eq!(tk.balance(&w.treasury), claimable as i128);
    assert_eq!(tk.balance(&attacker), attacker_before);
    assert_eq!(tk.balance(&trader), trader_before);
    assert_eq!(tk.balance(&w.fee_keeper), 0);
    assert_eq!(fh.claimable_fees(&w.market_tk, &w.long_tk), 0);
    assert_eq!(ds_c.get_u128(&pool_key), pool_before - claimable);
}

#[test]
fn market_override_receives_settled_fees_instead_of_treasury() {
    let w = setup();
    let env = &w.env;
    let trader = Address::generate(env);
    let partner = Address::generate(env);
    let fh = FHClient::new(env, &w.fee_handler);
    let tk = TokenClient::new(env, &w.long_tk);

    fh.set_market_fee_receiver(&w.market_tk, &Some(partner.clone()));
    // The order names the treasury itself as its receiver; that must not
    // pull the fee back to the protocol either.
    open_long_naming_receiver(&w, &trader, &w.treasury);

    let claimable = fh.claimable_fees(&w.market_tk, &w.long_tk);
    assert!(claimable > 0);
    let treasury_before = tk.balance(&w.treasury);

    fh.claim_fees(&w.fee_keeper, &w.market_tk, &w.long_tk);

    assert_eq!(tk.balance(&partner), claimable as i128);
    assert_eq!(tk.balance(&w.treasury), treasury_before);
}
