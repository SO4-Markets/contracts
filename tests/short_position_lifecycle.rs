//! Integration test: short position lifecycle where collateral and PnL are
//! denominated entirely in the short token (USDC), exercised through the real
//! `execute_order` flow (issue #264).
//!
//! Scenario (winning short):
//!   1. Alice (LP) deposits 20,000 USDC into the ETH/USD market's short side.
//!   2. Bob opens a 5 ETH short position (10,000 USD notional) at $2000/ETH,
//!      with 1,000 USDC collateral.
//!   3. Price moves to $1,800/ETH — the short is $1,000 in unrealised profit.
//!   4. Bob closes the full position via a real MarketDecrease `execute_order`.
//!   5. Bob receives 1,000 USDC collateral + 1,000 USDC PnL = 2,000 USDC.
//!
//! Assertions:
//!   - USDC pool decreases by exactly 1,000 (the PnL paid out).
//!   - Short OI (for the USDC collateral bucket) decreases to 0.
//!   - Position is removed from storage.
//!   - No ETH (long token) pool balance changes — a short only ever touches USDC.
//!
//! A second test covers the losing-short case: price rises, so Bob loses his
//! USDC collateral and the pool gains it.

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

/// LP deposits `usdc_amount` (whole USDC units) into the short side of the market only.
fn lp_deposit_short_only(w: &World, alice: &Address, usdc_amount: i128) {
    StellarAssetClient::new(&w.env, &w.short_tk).mint(alice, &(usdc_amount * ONE_TOKEN));

    let dep_key = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        alice,
        &CreateDepositParams {
            receiver: alice.clone(),
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
}

#[test]
fn short_position_lifecycle_winning_short_pays_pnl_from_pool() {
    let w = setup();
    let env = &w.env;
    let ds_c = DsClient::new(env, &w.ds);

    let alice = Address::generate(env);
    let bob = Address::generate(env);

    // ── Step 1: Alice (LP) deposits 20,000 USDC into the short side ──────────
    set_prices(&w, 2000);
    lp_deposit_short_only(&w, &alice, 20_000);

    let long_pool_before_open = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk));
    let short_pool_after_deposit = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    assert_eq!(long_pool_before_open, 0, "no long-side liquidity was ever deposited");
    assert!(short_pool_after_deposit > 0, "short pool must reflect Alice's USDC deposit");

    // ── Step 2: Bob opens a 5 ETH short (10,000 USD notional) with 1,000 USDC collateral
    StellarAssetClient::new(env, &w.short_tk).mint(&bob, &(1_000 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).transfer(&bob, &w.ord_vault, &(1_000 * ONE_TOKEN));

    set_prices(&w, 2000);

    let open_key = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 10_000 * ONE_USD, // 5 ETH notional at $2000
            collateral_delta_amount: 1_000 * ONE_TOKEN,
            trigger_price: 0,
            acceptable_price: 1_900 * ONE_USD, // shorts accept a price no lower than this
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &open_key);

    // Position must exist
    let pos_key = position_key(env, &bob, &w.market_tk, &w.short_tk, false);
    let pos = OHClient::new(env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("Bob must have an open short position after MarketIncrease");
    assert!(pos.size_in_usd > 0, "position size must be nonzero");
    assert!(!pos.is_long, "position must be recorded as short");

    let short_oi_key = open_interest_key(env, &w.market_tk, &w.short_tk, false);
    let short_oi_after_open = ds_c.get_u128(&short_oi_key);
    assert!(short_oi_after_open > 0, "short OI must reflect the newly opened position");

    let long_pool_after_open = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk));
    assert_eq!(long_pool_after_open, 0, "opening a short must never touch the ETH pool");

    // ── Step 3: Price falls to $1,800 — the short is $1,000 in unrealised profit
    set_prices(&w, 1800);

    let short_pool_before_close = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    let bob_usdc_before_close = StellarAssetClient::new(env, &w.short_tk).balance(&bob);

    // ── Step 4: Bob closes the full position ──────────────────────────────────
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
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &close_key);

    // ── Assertions ─────────────────────────────────────────────────────────────

    // Position removed from storage.
    let pos_after = OHClient::new(env, &w.ord_handler).get_position(&pos_key);
    assert!(pos_after.is_none(), "Bob's position must be cleared after full close");

    // Short OI decreases to 0.
    let short_oi_after_close = ds_c.get_u128(&short_oi_key);
    assert_eq!(short_oi_after_close, 0, "short OI must return to 0 after full close");

    // USDC pool decreases by exactly 1,000 (the PnL paid out).
    let short_pool_after_close = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    let pool_decrease = short_pool_before_close - short_pool_after_close;
    assert_eq!(
        pool_decrease,
        (1_000 * ONE_TOKEN) as u128,
        "USDC pool must decrease by exactly the 1,000 USDC PnL paid out to Bob"
    );

    // No ETH pool balance changes — a short uses only USDC.
    let long_pool_after_close = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk));
    assert_eq!(long_pool_after_close, 0, "closing a short must never touch the ETH pool");

    // Bob receives 1,000 USDC collateral + 1,000 USDC PnL = 2,000 USDC.
    let bob_usdc_after_close = StellarAssetClient::new(env, &w.short_tk).balance(&bob);
    let bob_payout = bob_usdc_after_close - bob_usdc_before_close;
    assert_eq!(
        bob_payout,
        2_000 * ONE_TOKEN,
        "Bob must receive his 1,000 USDC collateral plus 1,000 USDC PnL"
    );
}

#[test]
fn short_position_lifecycle_losing_short_forfeits_collateral_to_pool() {
    let w = setup();
    let env = &w.env;
    let ds_c = DsClient::new(env, &w.ds);

    let alice = Address::generate(env);
    let bob = Address::generate(env);

    // ── Step 1: Alice (LP) deposits 20,000 USDC into the short side ──────────
    set_prices(&w, 2000);
    lp_deposit_short_only(&w, &alice, 20_000);

    // ── Step 2: Bob opens a 5 ETH short (10,000 USD notional) with 1,000 USDC collateral
    StellarAssetClient::new(env, &w.short_tk).mint(&bob, &(1_000 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).transfer(&bob, &w.ord_vault, &(1_000 * ONE_TOKEN));

    set_prices(&w, 2000);

    let open_key = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 10_000 * ONE_USD,
            collateral_delta_amount: 1_000 * ONE_TOKEN,
            trigger_price: 0,
            // Opening a short: acceptable_price is the minimum entry price Bob will accept.
            acceptable_price: 1_900 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &open_key);

    let pos_key = position_key(env, &bob, &w.market_tk, &w.short_tk, false);
    let pos = OHClient::new(env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("Bob must have an open short position after MarketIncrease");

    let short_oi_key = open_interest_key(env, &w.market_tk, &w.short_tk, false);
    assert!(ds_c.get_u128(&short_oi_key) > 0, "short OI must reflect the newly opened position");

    // ── Step 3: Price RISES to $2,200 — the short is now $1,000 in unrealised loss
    set_prices(&w, 2200);

    let short_pool_before_close = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    let bob_usdc_before_close = StellarAssetClient::new(env, &w.short_tk).balance(&bob);

    // ── Step 4: Bob closes the full position, forfeiting his collateral ──────
    let close_key = OHClient::new(env, &w.ord_handler).create_order(
        &bob,
        &CreateOrderParams {
            receiver: bob.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: pos.size_in_usd,
            collateral_delta_amount: pos.collateral_amount,
            trigger_price: 0,
            // Bob is stopping his loss: he must accept a price up to (and including)
            // the current, worse $2,200 execution price.
            acceptable_price: 2_300 * ONE_USD,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: false,
            expiry_ledger: None,
        },
    );
    OHClient::new(env, &w.ord_handler).execute_order(&w.keeper, &close_key);

    // ── Assertions ─────────────────────────────────────────────────────────────

    // Position removed from storage.
    let pos_after = OHClient::new(env, &w.ord_handler).get_position(&pos_key);
    assert!(pos_after.is_none(), "Bob's position must be cleared after full close");

    // Short OI decreases to 0.
    assert_eq!(ds_c.get_u128(&short_oi_key), 0, "short OI must return to 0 after full close");

    // USDC pool GAINS Bob's forfeited collateral (his $1,000 loss).
    let short_pool_after_close = ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.short_tk));
    let pool_increase = short_pool_after_close - short_pool_before_close;
    assert_eq!(
        pool_increase,
        (1_000 * ONE_TOKEN) as u128,
        "USDC pool must gain exactly the 1,000 USDC Bob lost"
    );

    // No ETH pool balance changes — a short uses only USDC.
    assert_eq!(
        ds_c.get_u128(&pool_amount_key(env, &w.market_tk, &w.long_tk)),
        0,
        "closing a short must never touch the ETH pool"
    );

    // Bob receives nothing back — his entire 1,000 USDC collateral is forfeited.
    let bob_usdc_after_close = StellarAssetClient::new(env, &w.short_tk).balance(&bob);
    assert_eq!(
        bob_usdc_after_close, bob_usdc_before_close,
        "Bob must receive no payout after losing his full collateral"
    );
}
