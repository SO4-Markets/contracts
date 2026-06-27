//! Integration test suite: Auto-Deleveraging (ADL) risk flow
//!
//! Tests the full ADL lifecycle from pool seeding through profitable position
//! detection to partial closure via the AdlHandler.
//!
//! Scenario background:
//!   ADL is triggered when the aggregate trader PnL as a fraction of pool value
//!   exceeds `max_pnl_factor_for_adl`. The AdlHandler gates execution behind the
//!   `ADL_KEEPER` role and verifies the target position is profitable before
//!   delegating to order_handler.
//!
//! Test matrix:
//!   1. `adl_not_required_when_max_pnl_factor_is_zero`
//!      No cap configured → `is_adl_required` returns false regardless of PnL.
//!
//!   2. `adl_required_when_pnl_factor_exceeds_threshold`
//!      Price rallies sharply after position open; with a very low ADL threshold,
//!      `is_adl_required` returns true.
//!
//!   3. `adl_executes_and_reduces_position_size`
//!      `execute_adl` via AdlHandler reduces position size (partial close).
//!      Position still exists (partial, not full close).
//!
//!   4. `adl_reverts_when_pnl_factor_below_threshold`
//!      High threshold → `execute_adl` panics with `AdlNotRequired`.
//!
//!   5. `adl_reverts_when_position_is_unprofitable`
//!      Price drops → position at a loss → `is_adl_required` false even with low threshold.
//!
//!   6. `adl_requires_adl_keeper_role`
//!      Non-ADL-keeper address → `execute_adl` panics with `Unauthorized`.

#![cfg(test)]

use adl_handler::{AdlHandler, AdlHandlerClient};
use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_pnl_factor_for_adl_key, position_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateDepositParams, CreateOrderParams, OrderType, TokenPrice};
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
    adl_keeper: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    dep_vault: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    adl_handler: Address,
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
    let adl_keeper = Address::generate(&env);

    // Role store
    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
    rs_c.grant_role(&admin, &adl_keeper, &roles::adl_keeper(&env));

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

    // Market token (LP token / pool custodian)
    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "ADL Test Market"),
        &soroban_sdk::String::from_str(&env, "GM-ADL"),
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

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    let adl_handler_addr = env.register(AdlHandler, ());
    AdlHandlerClient::new(&env, &adl_handler_addr).initialize(
        &admin,
        &rs,
        &ds,
        &oracle_addr,
        &ord_handler,
    );

    // Grant CONTROLLER to all handlers and the market token
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &adl_handler_addr, &roles::controller(&env));
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    // Register market tokens in data store
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    // Market config: 10 bps fee, 1 % min collateral factor, 100× max leverage
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
        adl_keeper,
        rs,
        ds,
        oracle: oracle_addr,
        dep_vault,
        ord_vault,
        dep_handler,
        ord_handler,
        adl_handler: adl_handler_addr,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
}

/// Set oracle prices (index_usd expressed as a plain number, e.g. 2_000).
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
                    min: ONE_USD, // short_tk is a stablecoin
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

/// Deposit `long_amt` long_tk into the pool to provide deep liquidity.
fn seed_pool(w: &TestWorld, long_amt: i128) {
    let lp = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &long_amt);
    let key = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: long_amt,
            short_token_amount: 0,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &key);
}

/// Open a long position for `trader` with the given collateral and size.
fn open_long(w: &TestWorld, trader: &Address, collateral: i128, size_usd: i128) {
    StellarAssetClient::new(&w.env, &w.long_tk).mint(trader, &collateral);
    soroban_sdk::token::Client::new(&w.env, &w.long_tk).transfer(
        trader,
        &w.ord_vault,
        &collateral,
    );
    let key = OHClient::new(&w.env, &w.ord_handler).create_order(
        trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: size_usd,
            collateral_delta_amount: collateral,
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

// ─── Test 1 ───────────────────────────────────────────────────────────────────

/// When `max_pnl_factor_for_adl` is zero (not configured), `is_adl_required`
/// must return false regardless of the actual PnL ratio.
#[test]
fn adl_not_required_when_max_pnl_factor_is_zero() {
    let w = setup();
    let entry_price = 1_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * ONE_USD);

    // Price doubles — position has significant profit.
    set_prices(&w, entry_price * 2);

    // No threshold configured (max_pnl_factor == 0) → ADL is disabled.
    let is_required =
        AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true);
    assert!(
        !is_required,
        "ADL must not be required when max_pnl_factor_for_adl is 0 (disabled)"
    );
}

// ─── Test 2 ───────────────────────────────────────────────────────────────────

/// With a very low ADL threshold, a large price move makes `is_adl_required`
/// return true.
#[test]
fn adl_required_when_pnl_factor_exceeds_threshold() {
    let w = setup();
    let entry_price = 1_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * ONE_USD);

    // Price doubles → large profitable PnL.
    let rally_price = entry_price * 2;
    set_prices(&w, rally_price);

    // Set a tiny threshold so the current PnL ratio exceeds it.
    let low_threshold: u128 = FLOAT_PRECISION as u128 / 1_000_000; // ~0.0001 %
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &low_threshold,
    );

    let is_required =
        AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true);
    assert!(
        is_required,
        "ADL must be required when PnL factor exceeds the configured threshold"
    );
}

// ─── Test 3 ───────────────────────────────────────────────────────────────────

/// `execute_adl` partially closes the profitable position through AdlHandler.
/// Position size must decrease and the position must still exist (partial close).
#[test]
fn adl_executes_and_reduces_position_size() {
    let w = setup();
    let entry_price = 1_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    let size_usd = 10_000 * ONE_USD;
    open_long(&w, &trader, 5 * ONE_TOKEN, size_usd);

    // Record size before ADL.
    let pos_key = position_key(&w.env, &trader, &w.market_tk, &w.long_tk, true);
    let pos_before = OHClient::new(&w.env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("position must exist before ADL");
    let size_before = pos_before.size_in_usd;
    assert!(size_before > 0, "position size must be positive before ADL");

    // Price doubles → substantial profit.
    let rally_price = entry_price * 2;
    set_prices(&w, rally_price);

    // Set a low threshold.
    let low_threshold: u128 = FLOAT_PRECISION as u128 / 1_000_000;
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &low_threshold,
    );

    // Execute ADL on 25 % of the position.
    let adl_size = size_usd / 4;
    AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
        &w.adl_keeper,
        &trader,
        &w.market_tk,
        &w.long_tk,
        &true,
        &adl_size,
    );

    // Position must still exist (partial close, not fully liquidated).
    let pos_after = OHClient::new(&w.env, &w.ord_handler)
        .get_position(&pos_key)
        .expect("position must still exist after partial ADL");

    assert!(
        pos_after.size_in_usd < size_before,
        "ADL must reduce position size: before={size_before}, after={}",
        pos_after.size_in_usd
    );
}

// ─── Test 4 ───────────────────────────────────────────────────────────────────

/// `execute_adl` must panic with `AdlNotRequired` when the PnL ratio does not
/// exceed the configured threshold.
#[test]
#[should_panic]
fn adl_reverts_when_pnl_factor_below_threshold() {
    let w = setup();
    let entry_price = 1_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    open_long(&w, &trader, 5 * ONE_TOKEN, 5_000 * ONE_USD);

    // Price rises only modestly → small PnL ratio.
    let modest_rally = 1_100i128; // +10 %
    set_prices(&w, modest_rally);

    // High threshold — PnL ratio will not exceed it.
    let high_threshold: u128 = FLOAT_PRECISION as u128; // 100 % — essentially unreachable
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &high_threshold,
    );

    // Must panic with AdlNotRequired.
    AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
        &w.adl_keeper,
        &trader,
        &w.market_tk,
        &w.long_tk,
        &true,
        &(500 * ONE_USD),
    );
}

// ─── Test 5 ───────────────────────────────────────────────────────────────────

/// When the position is at a loss (price below entry), `is_adl_required` must
/// return false even with the strictest threshold.
#[test]
fn adl_not_required_when_position_is_unprofitable() {
    let w = setup();
    let entry_price = 2_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * ONE_USD);

    // Price crashes → long position is deeply underwater.
    let crash_price = 500i128;
    set_prices(&w, crash_price);

    // Even with the strictest threshold (1 unit), unprofitable PnL → no ADL.
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &1u128,
    );

    let is_required =
        AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true);
    assert!(
        !is_required,
        "ADL must not be required when the position is at a loss"
    );
}

// ─── Test 6 ───────────────────────────────────────────────────────────────────

/// A caller without the `ADL_KEEPER` role must be rejected by `execute_adl`.
#[test]
#[should_panic]
fn adl_requires_adl_keeper_role() {
    let w = setup();
    let entry_price = 1_000i128;

    set_prices(&w, entry_price);
    seed_pool(&w, 200 * ONE_TOKEN);
    set_prices(&w, entry_price);

    let trader = Address::generate(&w.env);
    open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * ONE_USD);

    // Price doubles.
    set_prices(&w, entry_price * 2);

    // Set a low threshold so ADL is technically required.
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
        &1u128,
    );

    // An impostor without ADL_KEEPER role must be rejected.
    let impostor = Address::generate(&w.env);
    AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
        &impostor,
        &trader,
        &w.market_tk,
        &w.long_tk,
        &true,
        &(1_000 * ONE_USD),
    );
}
