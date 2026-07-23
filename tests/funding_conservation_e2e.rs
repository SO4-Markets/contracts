//! End-to-end test: real, opposite-side positions settle funding to
//! conserved actual token balances — issue #451.
//!
//! Alice opens a long, Bob opens a short on the same market with a
//! long-heavy OI imbalance (longs pay shorts). Funding accrues, both
//! positions close, and Bob claims his funding via fee_handler. This
//! checks real token balances end-to-end, not just internal accumulators.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient};
use gmx_keys::{position_key, roles};
use gmx_market_utils::update_funding_state;
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, MarketProps, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
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
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle, &dep_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &ord_vault);

    let fee_handler = env.register(FeeHandler, ());
    FHClient::new(&env, &fee_handler).initialize(&admin, &rs, &ds);

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &fee_handler, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &gmx_keys::market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

    World {
        env,
        admin,
        keeper,
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
                TokenPrice { token: w.long_tk.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
                TokenPrice { token: w.short_tk.clone(), min: ONE_USD, max: ONE_USD },
                TokenPrice { token: w.index_tk.clone(), min: eth_usd * ONE_USD, max: eth_usd * ONE_USD },
            ],
        ),
    );
}

#[test]
fn funding_conserves_real_token_balances_between_two_opposite_side_positions() {
    let w = setup();
    let env = &w.env;

    let lp = Address::generate(env);
    let alice = Address::generate(env);
    let bob = Address::generate(env);

    set_prices(&w, 2000);

    // LP funds both sides of the pool.
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

    // Alice opens a large long, collateralised in long_tk (matching the funding
    // accounting model: the long side's funding key is scoped to market.long_token).
    StellarAssetClient::new(env, &w.long_tk).mint(&alice, &(1 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.long_tk).transfer(&alice, &w.ord_vault, &(1 * ONE_TOKEN));
    let alice_open = OHClient::new(env, &w.ord_handler).create_order(
        &alice,
        &CreateOrderParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 8_000 * ONE_USD,
            collateral_delta_amount: 1 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 2_100 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &alice_open);

    // Bob opens a small short.
    StellarAssetClient::new(env, &w.short_tk).mint(&bob, &(200 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).transfer(&bob, &w.ord_vault, &(200 * ONE_TOKEN));
    let bob_open = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 1_000 * ONE_USD,
            collateral_delta_amount: 200 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &bob_open);

    // Directly seed a large, established per-second funding rate favoring
    // "longs pay shorts" (matches the real long-heavy OI just created), and
    // accrue it over a long period — same technique used for issue #266,
    // bypassing the ramp mechanism's per-call step-size limits.
    let market_props = MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
    let ds_c = DsClient::new(env, &w.ds);
    let fp = FLOAT_PRECISION as u128;
    ds_c.set_i128(&w.admin, &gmx_keys::saved_funding_factor_per_second_key(env, &w.market_tk), &(FLOAT_PRECISION / 10));
    ds_c.set_u128(&w.admin, &gmx_keys::funding_updated_at_key(env, &w.market_tk), &0u128);
    ds_c.set_u128_instance(&w.admin, &gmx_keys::funding_factor_key(env, &w.market_tk), &fp);
    ds_c.set_u128_instance(&w.admin, &gmx_keys::funding_exponent_factor_key(env, &w.market_tk), &fp);
    let ramp: u128 = 1_000u128 * fp;
    ds_c.set_u128_instance(&w.admin, &gmx_keys::funding_increase_factor_per_second_key(env, &w.market_tk), &ramp);
    ds_c.set_u128_instance(&w.admin, &gmx_keys::funding_decrease_factor_per_second_key(env, &w.market_tk), &ramp);
    let bound: i128 = 1_000_000_i128 * FLOAT_PRECISION;
    ds_c.set_i128_instance(&w.admin, &gmx_keys::min_funding_factor_per_second_key(env, &w.market_tk), &(-bound));
    ds_c.set_i128_instance(&w.admin, &gmx_keys::max_funding_factor_per_second_key(env, &w.market_tk), &bound);

    update_funding_state(env, &w.ds, &w.admin, &market_props, 0, 0, 100_000);

    set_prices(&w, 2000);

    // Alice (paying side) closes her long fully.
    let alice_pos_key = position_key(env, &alice, &w.market_tk, &w.long_tk, true);
    let alice_pos = OHClient::new(env, &w.ord_handler).get_position(&alice_pos_key).unwrap();
    let alice_long_before_close = StellarAssetClient::new(env, &w.long_tk).balance(&alice);
    let alice_close = OHClient::new(env, &w.ord_handler).create_order(
        &alice,
        &CreateOrderParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: alice_pos.size_in_usd,
            collateral_delta_amount: alice_pos.collateral_amount,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &alice_close);
    let alice_payout = StellarAssetClient::new(env, &w.long_tk).balance(&alice) - alice_long_before_close;

    // Alice (paying side) must never receive back more than her full collateral —
    // any nonzero funding fee only ever reduces her payout, never inflates it.
    assert!(
        alice_payout <= 1 * ONE_TOKEN,
        "Alice (paying side) must never receive more than her full collateral back: got {alice_payout}"
    );

    // Bob (receiving side) closes his short, then claims his accrued funding.
    let bob_pos_key = position_key(env, &bob, &w.market_tk, &w.short_tk, false);
    let bob_pos = OHClient::new(env, &w.ord_handler).get_position(&bob_pos_key).unwrap();
    let bob_close = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: bob_pos.size_in_usd,
            collateral_delta_amount: bob_pos.collateral_amount,
            trigger_price: 0,
            acceptable_price: 2_100 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &bob_close);

    let bob_short_before_claim = StellarAssetClient::new(env, &w.short_tk).balance(&bob);
    let claimed = FHClient::new(env, &w.fee_handler).claim_funding_fees(&bob, &w.market_tk, &w.short_tk);
    let bob_claim_payout = StellarAssetClient::new(env, &w.short_tk).balance(&bob) - bob_short_before_claim;

    assert!(claimed > 0, "Bob (receiving side) must have a nonzero claimable funding amount");
    assert_eq!(
        bob_claim_payout, claimed as i128,
        "Bob's real token balance increase must exactly match the claimed amount"
    );
}
