//! Integration test suite: Liquidation Flow End-to-End
//!
//! Exercises the full liquidation path from position creation through
//! under-collateralisation detection to forced closure via LiquidationHandler.
//!
//! Scenario background:
//!   A position is liquidatable when its remaining collateral (after unrealised
//!   losses) falls below `min_collateral_factor × size_in_usd`. The
//!   LiquidationHandler verifies health via `is_liquidatable` before delegating
//!   the forced close to `order_handler::liquidate_position`.
//!
//! Test matrix:
//!   1. `check_liquidatable_returns_false_for_healthy_long`
//!      Healthy long (well-collateralised at entry price) → `check_liquidatable`
//!      returns false.
//!
//!   2. `check_liquidatable_returns_true_after_crash`
//!      Price crashes below the liquidation threshold → `check_liquidatable`
//!      returns true.
//!
//!   3. `liquidation_of_underwater_long_removes_position`
//!      Full lifecycle: open long, crash price, `liquidate_position` via
//!      LiquidationHandler → position storage key is removed.
//!
//!   4. `liquidation_of_underwater_short_removes_position`
//!      Symmetric short-side test: open short, pump price, liquidate → key removed.
//!
//!   5. `liquidation_of_healthy_long_reverts`
//!      Attempting to liquidate a healthy long position must panic with
//!      `NotLiquidatable`.
//!
//!   6. `liquidation_requires_liquidation_keeper_role`
//!      A caller without the `LIQUIDATION_KEEPER` role must be rejected.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, pool_amount_key,
    position_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, OrderType, TokenPrice};
use liquidation_handler::{LiquidationHandler, LiquidationHandlerClient as LiqClient};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

const ONE_TOKEN: i128 = 10_000_000; // 10^7 (Stellar 7-decimal precision)
const ONE_USD: i128 = FLOAT_PRECISION;

// ─── Test world ────────────────────────────────────────────────────────────────

struct TestWorld {
    env: Env,
    admin: Address,
    keeper: Address,
    liq_keeper: Address,
    trader: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    ord_handler: Address,
    liq_handler: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
    index_tk: Address,
}

fn setup() -> TestWorld {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let liq_keeper = Address::generate(&env);
    let trader = Address::generate(&env);

    // Role store
    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
    rs_c.grant_role(&admin, &liq_keeper, &roles::liquidation_keeper(&env));

    // Data store
    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    // Oracle
    let oracle_addr = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    // Order vault
    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // Market token (LP + pool custodian)
    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "Liq Test Market"),
        &soroban_sdk::String::from_str(&env, "GM-LIQ"),
    );

    // Underlying tokens
    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    // Order handler
    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Liquidation handler
    let liq_handler_addr = env.register(LiquidationHandler, ());
    LiqClient::new(&env, &liq_handler_addr).initialize(
        &admin,
        &rs,
        &ds,
        &oracle_addr,
        &ord_handler,
    );

    // Grant CONTROLLER to handlers and market token
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &liq_handler_addr, &roles::controller(&env));
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    // Register market tokens in data store
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    // Market config: 10 bps position fee, 1 % min collateral factor, 100× max leverage
    let fee_factor = FLOAT_PRECISION / 1_000;
    let min_col_factor = FLOAT_PRECISION / 100;
    ds_c.set_u128(
        &admin,
        &gmx_keys::position_fee_factor_key(&env, &market_tk, true),
        &(fee_factor as u128),
    );
    ds_c.set_u128(
        &admin,
        &gmx_keys::position_fee_factor_key(&env, &market_tk, false),
        &(fee_factor as u128),
    );
    ds_c.set_u128(
        &admin,
        &gmx_keys::min_collateral_factor_key(&env, &market_tk),
        &(min_col_factor as u128),
    );
    ds_c.set_u128(
        &admin,
        &gmx_keys::max_leverage_key(&env, &market_tk),
        &(100 * FLOAT_PRECISION as u128),
    );

    TestWorld {
        env,
        admin,
        keeper,
        liq_keeper,
        trader,
        rs,
        ds,
        oracle: oracle_addr,
        ord_vault,
        ord_handler,
        liq_handler: liq_handler_addr,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
}

/// Set oracle prices. `index_usd` is a plain number (e.g. 2_000); prices are
/// scaled by FLOAT_PRECISION internally.
fn set_prices(w: &TestWorld, index_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice {
                    token: w.long_tk.clone(),
                    min: index_usd * ONE_USD,
                    max: index_usd * ONE_USD,
                },
                TokenPrice {
                    token: w.short_tk.clone(),
                    min: ONE_USD, // stablecoin at $1
                    max: ONE_USD,
                },
                TokenPrice {
                    token: w.index_tk.clone(),
                    min: index_usd * ONE_USD,
                    max: index_usd * ONE_USD,
                },
            ],
        ),
    );
}

/// Seed the pool with enough long_tk collateral so that positions can be opened
/// and PnL can be paid out on decrease / liquidation.
fn seed_pool(w: &TestWorld, long_amount: i128) {
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &long_amount);
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
        &(long_amount as u128),
    );
}

/// Seed the pool with short_tk collateral.
fn seed_short_pool(w: &TestWorld, short_amount: i128) {
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.market_tk, &short_amount);
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &pool_amount_key(&w.env, &w.market_tk, &w.short_tk),
        &(short_amount as u128),
    );
}

/// Open a long position for `w.trader` using `long_tk` as collateral.
fn open_long_position(w: &TestWorld, collateral_tokens: i128, size_usd: i128) {
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral_tokens);
    let key = OHClient::new(&w.env, &w.ord_handler).create_order(
        &w.trader,
        &CreateOrderParams {
            receiver: w.trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: size_usd,
            collateral_delta_amount: collateral_tokens,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    OHClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
}

/// Open a short position for `w.trader` using `short_tk` as collateral.
fn open_short_position(w: &TestWorld, collateral_tokens: i128, size_usd: i128) {
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.ord_vault, &collateral_tokens);
    let key = OHClient::new(&w.env, &w.ord_handler).create_order(
        &w.trader,
        &CreateOrderParams {
            receiver: w.trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: size_usd,
            collateral_delta_amount: collateral_tokens,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
        },
    );
    OHClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
}

// ─── Test 1 ───────────────────────────────────────────────────────────────────

/// A well-collateralised long position (small leverage, price unchanged) must
/// NOT be flagged as liquidatable by `check_liquidatable`.
#[test]
fn check_liquidatable_returns_false_for_healthy_long() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 1_000_000 * ONE_TOKEN);

    // 10 tokens at $2 000 each = $20 000 collateral; 0.5× leverage
    let collateral = 10 * ONE_TOKEN;
    let size_usd = 10_000 * ONE_USD; // size < collateral_value → very healthy
    open_long_position(&w, collateral, size_usd);

    // Price stays the same.
    set_prices(&w, entry_price);

    let is_liq = LiqClient::new(&w.env, &w.liq_handler).check_liquidatable(
        &w.trader,
        &w.market_tk,
        &w.long_tk,
        &true,
    );
    assert!(
        !is_liq,
        "healthy position must NOT be flagged as liquidatable"
    );
}

// ─── Test 2 ───────────────────────────────────────────────────────────────────

/// After a severe price crash the long position falls below the minimum
/// collateral factor and `check_liquidatable` returns true.
#[test]
fn check_liquidatable_returns_true_after_crash() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 1_000_000 * ONE_TOKEN);

    // 1 token collateral ($2 000), $20 000 size → 10× leverage.
    let collateral = 1 * ONE_TOKEN;
    let size_usd = 20_000 * ONE_USD;
    open_long_position(&w, collateral, size_usd);

    // Price crashes to $100 — deeply underwater.
    let crash_price = 100i128;
    set_prices(&w, crash_price);

    let is_liq = LiqClient::new(&w.env, &w.liq_handler).check_liquidatable(
        &w.trader,
        &w.market_tk,
        &w.long_tk,
        &true,
    );
    assert!(
        is_liq,
        "position must be flagged as liquidatable after severe price crash"
    );
}

// ─── Test 3 ───────────────────────────────────────────────────────────────────

/// Full end-to-end liquidation of an underwater long position:
///   1. Open long at $2 000.
///   2. Crash price to $100.
///   3. `liquidate_position` via LiquidationHandler.
///   4. Position storage key must be removed.
#[test]
fn liquidation_of_underwater_long_removes_position() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 1_000_000 * ONE_TOKEN);

    let collateral = 1 * ONE_TOKEN; // $2 000 at entry
    let size_usd = 20_000 * ONE_USD; // 10× leverage
    open_long_position(&w, collateral, size_usd);

    // Verify position exists.
    let pos_key = position_key(&w.env, &w.trader, &w.market_tk, &w.long_tk, true);
    assert!(
        OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .is_some(),
        "long position must exist before liquidation"
    );

    // Crash price.
    set_prices(&w, 100i128);

    // Confirm liquidatable.
    assert!(
        LiqClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.trader,
            &w.market_tk,
            &w.long_tk,
            &true,
        ),
        "position must be liquidatable after crash"
    );

    // Execute liquidation.
    LiqClient::new(&w.env, &w.liq_handler).liquidate_position(
        &w.liq_keeper,
        &w.trader,
        &w.market_tk,
        &w.long_tk,
        &true,
    );

    // Position key must be cleared.
    assert!(
        OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .is_none(),
        "long position key must be removed after liquidation"
    );
}

// ─── Test 4 ───────────────────────────────────────────────────────────────────

/// Full end-to-end liquidation of an underwater short position:
///   1. Open short at $2 000 using short_tk ($1 stablecoin) as collateral.
///   2. Pump index price to $10 000 — short is deeply underwater.
///   3. `liquidate_position` via LiquidationHandler.
///   4. Position storage key must be removed.
#[test]
fn liquidation_of_underwater_short_removes_position() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_short_pool(&w, 1_000_000 * ONE_TOKEN);

    // 1 short_tk = $1 at stablecoin price; $10 size = 10× leverage.
    let collateral = 1 * ONE_TOKEN;
    let size_usd = 10 * ONE_USD;
    open_short_position(&w, collateral, size_usd);

    // Verify position exists.
    let pos_key = position_key(&w.env, &w.trader, &w.market_tk, &w.short_tk, false);
    assert!(
        OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .is_some(),
        "short position must exist before liquidation"
    );

    // Pump index price — short is now deeply underwater.
    set_prices(&w, 10_000i128);

    // Confirm liquidatable.
    assert!(
        LiqClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.trader,
            &w.market_tk,
            &w.short_tk,
            &false,
        ),
        "short position must be liquidatable after pump"
    );

    // Execute liquidation.
    LiqClient::new(&w.env, &w.liq_handler).liquidate_position(
        &w.liq_keeper,
        &w.trader,
        &w.market_tk,
        &w.short_tk,
        &false,
    );

    // Position key must be cleared.
    assert!(
        OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .is_none(),
        "short position key must be removed after liquidation"
    );
}

// ─── Test 5 ───────────────────────────────────────────────────────────────────

/// Attempting to liquidate a healthy long position must panic with
/// `NotLiquidatable`.
#[test]
#[should_panic]
fn liquidation_of_healthy_long_reverts() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 1_000_000 * ONE_TOKEN);

    // Very well-collateralised: 10 tokens ($20 000) for a $5 000 position.
    let collateral = 10 * ONE_TOKEN;
    let size_usd = 5_000 * ONE_USD;
    open_long_position(&w, collateral, size_usd);

    // Price stays the same — position is healthy.
    set_prices(&w, entry_price);

    // Must panic with NotLiquidatable.
    LiqClient::new(&w.env, &w.liq_handler).liquidate_position(
        &w.liq_keeper,
        &w.trader,
        &w.market_tk,
        &w.long_tk,
        &true,
    );
}

// ─── Test 6 ───────────────────────────────────────────────────────────────────

/// A caller without the `LIQUIDATION_KEEPER` role must be rejected.
#[test]
#[should_panic]
fn liquidation_requires_liquidation_keeper_role() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 1_000_000 * ONE_TOKEN);

    let collateral = 1 * ONE_TOKEN;
    let size_usd = 20_000 * ONE_USD;
    open_long_position(&w, collateral, size_usd);

    // Price crashes so the position is liquidatable.
    set_prices(&w, 100i128);

    // An address without LIQUIDATION_KEEPER must be rejected.
    let impostor = Address::generate(&w.env);
    LiqClient::new(&w.env, &w.liq_handler).liquidate_position(
        &impostor,
        &w.trader,
        &w.market_tk,
        &w.long_tk,
        &true,
    );
}
