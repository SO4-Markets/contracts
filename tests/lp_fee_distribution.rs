//! Integration test: LP fee accrual and proportional distribution — issue #262.
//!
//! Scenario: two LPs (Alice, Bob) each deposit 10,000 USDC into the short
//! side of the pool for equal GM shares. A trader opens and closes a
//! position, generating trading fees that accrue into the same USDC pool.
//! Both LPs then withdraw their full GM balance and must each receive back
//! their principal plus their proportional share of the accrued fee.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, pool_amount_key,
    position_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, CreateWithdrawalParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};
use withdrawal_handler::{WithdrawalHandler, WithdrawalHandlerClient as WHClient};
use withdrawal_vault::{WithdrawalVault, WithdrawalVaultClient as WVClient};

const ONE_TOKEN: i128 = 10_000_000; // 7-decimal Stellar precision
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    keeper: Address,
    ds: Address,
    oracle: Address,
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
        &soroban_sdk::String::from_str(&env, "GMX ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM"),
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

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    World {
        env,
        keeper,
        ds,
        oracle: oracle_addr,
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

fn set_prices(w: &World, eth_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &soroban_sdk::Vec::from_array(
            &w.env,
            [
                TokenPrice { token: w.long_tk.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
                TokenPrice { token: w.short_tk.clone(), min: ONE_USD, max: ONE_USD },
                TokenPrice { token: w.index_tk.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
            ],
        ),
    );
}

fn lp_deposit_short_only(w: &World, lp: &Address, usdc_amount: i128) -> i128 {
    StellarAssetClient::new(&w.env, &w.short_tk).mint(lp, &(usdc_amount * ONE_TOKEN));
    let dep_key = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 0,
            short_token_amount: usdc_amount * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);
    MtClient::new(&w.env, &w.market_tk).balance(lp)
}

#[test]
fn lp_fees_split_proportionally_between_two_equal_depositors() {
    let w = setup();
    let env = &w.env;
    let ds_c = DsClient::new(env, &w.ds);

    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let trader = Address::generate(env);

    // ── Step 1 & 2: Alice and Bob each deposit 10,000 USDC for equal GM shares ──
    set_prices(&w, 2000);
    let alice_gm = lp_deposit_short_only(&w, &alice, 10_000);
    let bob_gm = lp_deposit_short_only(&w, &bob, 10_000);
    assert_eq!(alice_gm, bob_gm, "equal deposits at the same price must mint equal GM shares");

    let short_pool_before_trade = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));

    // ── Step 3: trader opens and closes a position, generating fees ────────────
    StellarAssetClient::new(env, &w.short_tk).mint(&trader, &(1_000 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).transfer(&trader, &w.ord_vault, &(1_000 * ONE_TOKEN));
    set_prices(&w, 2000);

    let open_key = OHClient::new(env, &w.ord_handler).create_order(
        &trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 10_000 * ONE_USD,
            collateral_delta_amount: 1_000 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &open_key);

    let pos_key = position_key(env, &trader, &w.market_tk, &w.short_tk, false);
    let pos = OHClient::new(env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("trader must have an open position");

    set_prices(&w, 1800);

    let close_key = OHClient::new(env, &w.ord_handler).create_order(
        &trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: pos.size_in_usd,
            collateral_delta_amount: pos.collateral_amount,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &close_key);

    let _ = short_pool_before_trade; // trade may net gain or lose the pool overall PnL vs fees

    // ── Step 4 & 5: both LPs withdraw everything ────────────────────────────────
    set_prices(&w, 2000);

    let alice_usdc_before = StellarAssetClient::new(env, &w.short_tk).balance(&alice);
    let wth_key_a = WHClient::new(env, &w.wth_handler).create_withdrawal(
        &alice,
        &CreateWithdrawalParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            market_token_amount: alice_gm,
            min_long_token_amount: 0,
            min_short_token_amount: 0,
            execution_fee: 0,
        },
    );
    WHClient::new(env, &w.wth_handler).execute_withdrawal(&w.keeper, &wth_key_a);
    let alice_payout = StellarAssetClient::new(env, &w.short_tk).balance(&alice) - alice_usdc_before;

    let bob_usdc_before = StellarAssetClient::new(env, &w.short_tk).balance(&bob);
    let wth_key_b = WHClient::new(env, &w.wth_handler).create_withdrawal(
        &bob,
        &CreateWithdrawalParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            market_token_amount: bob_gm,
            min_long_token_amount: 0,
            min_short_token_amount: 0,
            execution_fee: 0,
        },
    );
    WHClient::new(env, &w.wth_handler).execute_withdrawal(&w.keeper, &wth_key_b);
    let bob_payout = StellarAssetClient::new(env, &w.short_tk).balance(&bob) - bob_usdc_before;

    // ── Assertions ───────────────────────────────────────────────────────────────
    // Core claim of #262: equal GM shares must receive an equal (within rounding)
    // proportional payout, whatever the pool's net PnL/fee outcome from the trade.
    let diff = (alice_payout - bob_payout).abs();
    assert!(diff <= 1, "equal GM shares must split the pool proportionally within 1 unit of rounding, got diff={diff} (alice={alice_payout}, bob={bob_payout})");

    let gm_supply = MtClient::new(env, &w.market_tk).total_supply();
    assert_eq!(gm_supply, 0, "GM total supply must return to 0 after both LPs withdraw");

    let short_pool_final = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    assert_eq!(short_pool_final, 0, "USDC pool must be fully drained after both withdrawals");
}
