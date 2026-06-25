//! Integration test: full deposit → open long → close long → withdrawal lifecycle.
//!
//! Scenario:
//!   Alice (LP) deposits 1 WETH + 2000 USDC into the ETH/USD market.
//!   Bob opens a 1 ETH long position (2000 USD notional, 200 USDC collateral) at $2000/ETH.
//!   Price advances to $2200/ETH (+$200 PnL for Bob).
//!   Bob closes the full position — receives collateral + PnL back.
//!   Alice withdraws all her GM tokens.
//!
//! Final assertions:
//!   - GM token total supply == 0
//!   - Pool long and short amounts == 0 (net after fees to Alice)
//!   - Bob's collateral token balance reflects the ~200 USD PnL
//!   - All position storage keys cleared

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
    admin: Address,
    keeper: Address,
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
    long_tk: Address,  // WETH
    short_tk: Address, // USDC
    index_tk: Address, // ETH price feed token
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

    let wth_vault = env.register(WithdrawalVault, ());
    WVClient::new(&env, &wth_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // Market token (GM)
    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM"),
    );

    // Underlying tokens
    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    // Handlers
    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);

    let wth_handler = env.register(WithdrawalHandler, ());
    WHClient::new(&env, &wth_handler).initialize(&admin, &rs, &ds, &oracle_addr, &wth_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Grant CONTROLLER to all handlers so they can write to data_store and market_token
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    // Register market in data_store
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    World {
        env,
        admin,
        keeper,
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

fn set_prices(w: &World, eth_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &soroban_sdk::Vec::from_array(
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

#[test]
fn full_lifecycle_deposit_long_close_withdraw() {
    let w = setup();
    let env = &w.env;

    let alice = Address::generate(env);
    let bob = Address::generate(env);

    // ── Mint tokens ───────────────────────────────────────────────────────────
    // Alice: 1 WETH + 2000 USDC for LP deposit
    StellarAssetClient::new(env, &w.long_tk).mint(&alice, &(1 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).mint(&alice, &(2000 * ONE_TOKEN));
    // Bob: 200 USDC as collateral for a long position
    StellarAssetClient::new(env, &w.short_tk).mint(&bob, &(200 * ONE_TOKEN));

    let bob_initial_short = StellarAssetClient::new(env, &w.short_tk).balance(&bob);

    // ── Step 1: Alice deposits into the pool ──────────────────────────────────
    set_prices(&w, 2000);

    let dep_key = DHClient::new(env, &w.dep_handler).create_deposit(
        &alice,
        &CreateDepositParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 1 * ONE_TOKEN,
            short_token_amount: 2000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);

    let alice_gm = MtClient::new(env, &w.market_tk).balance(&alice);
    assert!(alice_gm > 0, "Alice must receive GM tokens after deposit");

    let ds_c = DsClient::new(env, &w.ds);
    let long_pool = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk));
    let short_pool = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    assert_eq!(long_pool, 1 * ONE_TOKEN as u128, "long pool must reflect Alice's deposit");
    assert!(short_pool > 0, "short pool must reflect Alice's deposit");

    // ── Step 2: Bob opens a long position ────────────────────────────────────
    // Bob puts 200 USDC collateral into order_vault and creates a MarketIncrease
    StellarAssetClient::new(env, &w.short_tk).transfer(&bob, &w.ord_vault, &(200 * ONE_TOKEN));

    set_prices(&w, 2000);

    let open_key = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
        receiver: bob.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.short_tk.clone(),
        swap_path: Vec::new(env),
        size_delta_usd: 2000 * ONE_USD,   // 1 ETH notional at $2000
        collateral_delta_amount: 200 * ONE_TOKEN,
        trigger_price: 0,
        acceptable_price: 2100 * ONE_USD, // accept up to $2100
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketIncrease,
        is_long: true,
    });
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &open_key);

    // Position must exist
    let pos_key = position_key(env, &bob, &w.market_tk, &w.short_tk, true);
    let pos = OHClient::new(env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("Bob must have an open position after MarketIncrease");
    assert!(pos.size_in_usd > 0, "position size must be nonzero");

    // ── Step 3: Price advances to $2200, Bob closes for profit ────────────────
    set_prices(&w, 2200);

    let close_key = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
        receiver: bob.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.short_tk.clone(),
        swap_path: Vec::new(env),
        size_delta_usd: pos.size_in_usd, // full close
        collateral_delta_amount: pos.collateral_amount,
        trigger_price: 0,
        acceptable_price: 2100 * ONE_USD, // accept down to $2100
        execution_fee: 0,
        min_output_amount: 0,
        order_type: OrderType::MarketDecrease,
        is_long: true,
    });
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &close_key);

    // Position must be gone
    let pos_after = OHClient::new(env, &w.ord_handler).get_position(&pos_key);
    assert!(pos_after.is_none(), "Bob's position must be cleared after full close");

    // Bob should have received more short tokens than he started with (PnL)
    let bob_final_short = StellarAssetClient::new(env, &w.short_tk).balance(&bob);
    assert!(
        bob_final_short > bob_initial_short,
        "Bob must receive PnL: initial={bob_initial_short}, final={bob_final_short}"
    );

    // ── Step 4: Alice withdraws all GM tokens ─────────────────────────────────
    set_prices(&w, 2200);

    let wth_key = WHClient::new(env, &w.wth_handler).create_withdrawal(
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
    WHClient::new(env, &w.wth_handler).execute_withdrawal(&w.keeper, &wth_key);

    // ── Final assertions ──────────────────────────────────────────────────────
    let gm_supply = MtClient::new(env, &w.market_tk).total_supply();
    assert_eq!(gm_supply, 0, "GM total supply must return to 0 after Alice withdraws");

    // Alice must have received tokens back
    let alice_long_back = StellarAssetClient::new(env, &w.long_tk).balance(&alice);
    let alice_short_back = StellarAssetClient::new(env, &w.short_tk).balance(&alice);
    assert!(alice_long_back > 0 || alice_short_back > 0, "Alice must receive pool tokens back");

    // Pool must be empty
    let long_pool_final = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk));
    let short_pool_final = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    assert_eq!(long_pool_final, 0, "long pool must be zero after full withdrawal");
    assert_eq!(short_pool_final, 0, "short pool must be zero after full withdrawal");
}
