//! Integration test suite: Keeper Execution Window Boundary
//!
//! Covers the keeper heartbeat mechanism introduced in issue #249.
//! A keeper's liveness is tracked by recording the ledger sequence at which it
//! last executed an order (`last_keeper_activity_key`). The gap between that
//! ledger and the current ledger is compared to a configurable timeout
//! (default: 2 880 ledgers ≈ 4 h at ~5 s/ledger). If the gap exceeds the
//! timeout the keeper is considered *stale*, the admin can flag it via
//! `flag_stale_keeper`, and the role can then be revoked immediately.
//!
//! Test matrix:
//!   1. `keeper_is_live_immediately_after_execution`
//!      Execute an order at ledger L; heartbeat shows 0 ledgers elapsed → not stale.
//!
//!   2. `keeper_becomes_stale_exactly_at_timeout_boundary`
//!      Execute at L; advance to L + timeout → still live.
//!      Advance to L + timeout + 1 → stale.  (Off-by-one boundary test.)
//!
//!   3. `custom_timeout_is_honoured`
//!      Set timeout to 100 ledgers. Execute at L.
//!      At L + 100 → live. At L + 101 → stale.
//!
//!   4. `flag_and_revoke_stale_keeper_lifecycle`
//!      Execute → advance past window → `flag_stale_keeper` succeeds and emits
//!      the `KeeperHeartbeatMissed` event → role is revocable without waiting.
//!
//!   5. `flag_stale_keeper_reverts_when_keeper_is_live`
//!      Calling `flag_stale_keeper` while the keeper is within its window must panic.
//!
//!   6. `non_admin_cannot_flag_stale_keeper`
//!      A non-admin address calling `flag_stale_keeper` must panic (Unauthorized).

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, pool_amount_key, roles,
};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

const ONE_TOKEN: i128 = 10_000_000; // 10^7 (Stellar 7-decimal precision)
const ONE_USD: i128 = FLOAT_PRECISION;

// ─── Test world ────────────────────────────────────────────────────────────────

struct TestWorld {
    env: Env,
    admin: Address,
    keeper: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    index_tk: Address,
}

fn setup() -> TestWorld {
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

    // Order vault
    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // Market token
    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "ETH Market"),
        &soroban_sdk::String::from_str(&env, "GM-ETH"),
    );

    // Underlying tokens
    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    // Order handler
    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Grant CONTROLLER to handlers
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    // Register market in data store
    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &long_tk);

    // Market config
    let fee_factor = FLOAT_PRECISION / 1_000; // 0.1 %
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
        &gmx_keys::max_leverage_key(&env, &market_tk),
        &(100 * FLOAT_PRECISION as u128),
    );
    ds_c.set_u128(
        &admin,
        &gmx_keys::min_collateral_factor_key(&env, &market_tk),
        &(FLOAT_PRECISION as u128 / 100), // 1 %
    );

    TestWorld {
        env,
        admin,
        keeper,
        rs,
        ds,
        oracle: oracle_addr,
        ord_vault,
        ord_handler,
        market_tk,
        long_tk,
        index_tk,
    }
}

/// Set oracle prices (index and long token at same price, expressed as USD × FLOAT_PRECISION).
fn set_prices(w: &TestWorld, eth_usd: i128) {
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
                    token: w.index_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
            ],
        ),
    );
}

/// Seed pool liquidity and return to allow opens.
fn seed_pool(w: &TestWorld) {
    let ds_c = DsClient::new(&w.env, &w.ds);
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(1_000_000 * ONE_TOKEN));
    ds_c.set_u128(
        &w.admin,
        &pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
        &(1_000_000 * ONE_TOKEN as u128),
    );
}

/// Open a small long position and execute it; returns the order handler client.
///
/// Used to trigger `record_keeper_activity` so the heartbeat ledger is stamped.
fn execute_order_at_current_ledger(w: &TestWorld) -> OHClient {
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let trader = Address::generate(&w.env);
    let collateral = 5 * ONE_TOKEN;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&trader, &collateral);
    soroban_sdk::token::Client::new(&w.env, &w.long_tk).transfer(
        &trader,
        &w.ord_vault,
        &collateral,
    );
    let key = oh_c.create_order(
        &trader,
        &CreateOrderParams {
            receiver: trader.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: soroban_sdk::Vec::new(&w.env),
            size_delta_usd: 2_000 * ONE_USD,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    oh_c.execute_order(&w.keeper, &key);
    oh_c
}

// ─── Test 1 ───────────────────────────────────────────────────────────────────

/// Immediately after executing an order at ledger L, the heartbeat shows
/// `ledgers_since_last_activity == 0` and `is_stale == false`.
#[test]
fn keeper_is_live_immediately_after_execution() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    // Execute an order at a deterministic ledger sequence.
    let exec_ledger: u32 = 500;
    w.env.ledger().set_sequence_number(exec_ledger);

    let oh_c = execute_order_at_current_ledger(&w);

    // Immediately check: gap should be 0, not stale.
    let status = oh_c.check_keeper_heartbeat(&w.ds, &roles::order_keeper(&w.env));

    assert_eq!(
        status.last_active_ledger, exec_ledger as u64,
        "last_active_ledger must equal the ledger at execution"
    );
    assert_eq!(
        status.ledgers_since_last_activity, 0,
        "gap must be 0 immediately after execution"
    );
    assert!(
        !status.is_stale,
        "keeper must NOT be stale immediately after execution"
    );
}

// ─── Test 2 ───────────────────────────────────────────────────────────────────

/// The staleness boundary is strict: at exactly `timeout` ledgers elapsed the
/// keeper is still live; at `timeout + 1` it is stale.
///
/// Default timeout = 2 880 ledgers (`DEFAULT_KEEPER_HEARTBEAT_TIMEOUT`).
#[test]
fn keeper_becomes_stale_exactly_at_timeout_boundary() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    let exec_ledger: u32 = 1_000;
    w.env.ledger().set_sequence_number(exec_ledger);
    let oh_c = execute_order_at_current_ledger(&w);

    let order_keeper_role = roles::order_keeper(&w.env);
    // Default heartbeat timeout is 2 880 ledgers.
    let default_timeout: u32 = 2_880;

    // ── At exactly the timeout boundary: still live ───────────────────────────
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + default_timeout);
    let status_at_boundary = oh_c.check_keeper_heartbeat(&w.ds, &order_keeper_role);
    assert!(
        !status_at_boundary.is_stale,
        "keeper must NOT be stale at exactly the boundary (ledgers_since={})",
        status_at_boundary.ledgers_since_last_activity
    );

    // ── One ledger beyond boundary: stale ─────────────────────────────────────
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + default_timeout + 1);
    let status_past_boundary = oh_c.check_keeper_heartbeat(&w.ds, &order_keeper_role);
    assert!(
        status_past_boundary.is_stale,
        "keeper MUST be stale one ledger past the boundary (ledgers_since={})",
        status_past_boundary.ledgers_since_last_activity
    );
    assert_eq!(
        status_past_boundary.last_active_ledger, exec_ledger as u64,
        "last_active_ledger must not change when keeper is silent"
    );
}

// ─── Test 3 ───────────────────────────────────────────────────────────────────

/// A custom heartbeat timeout overrides the default.
/// Setting it to 100 ledgers: at L + 100 → live; at L + 101 → stale.
#[test]
fn custom_timeout_is_honoured() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    let order_keeper_role = roles::order_keeper(&w.env);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    // Tighten the window to 100 ledgers.
    let custom_timeout: u64 = 100;
    oh_c.set_keeper_heartbeat_timeout(&w.admin, &order_keeper_role, &custom_timeout);

    let exec_ledger: u32 = 2_000;
    w.env.ledger().set_sequence_number(exec_ledger);
    execute_order_at_current_ledger(&w);

    // ── 50 ledgers later: within window ───────────────────────────────────────
    w.env.ledger().set_sequence_number(exec_ledger + 50);
    assert!(
        !oh_c
            .check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale,
        "keeper must be live 50 ledgers after execution with 100-ledger timeout"
    );

    // ── At exactly 100 ledgers: still live ────────────────────────────────────
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + custom_timeout as u32);
    assert!(
        !oh_c
            .check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale,
        "keeper must still be live at the exact boundary with custom timeout"
    );

    // ── At 101 ledgers: stale ─────────────────────────────────────────────────
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + custom_timeout as u32 + 1);
    assert!(
        oh_c.check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale,
        "keeper must be stale one ledger past custom timeout"
    );
}

// ─── Test 4 ───────────────────────────────────────────────────────────────────

/// Full lifecycle:
///   execute → advance past window → `flag_stale_keeper` succeeds →
///   role remains until explicitly revoked → revoke succeeds immediately.
#[test]
fn flag_and_revoke_stale_keeper_lifecycle() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    let order_keeper_role = roles::order_keeper(&w.env);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let rs_c = RsClient::new(&w.env, &w.rs);

    // Execute at ledger 1 000 → heartbeat stamped.
    let exec_ledger: u32 = 1_000;
    w.env.ledger().set_sequence_number(exec_ledger);
    execute_order_at_current_ledger(&w);

    // Verify keeper has the role.
    assert!(
        rs_c.has_role(&w.keeper, &order_keeper_role),
        "keeper must hold ORDER_KEEPER role before being flagged"
    );

    // Advance past the default 2 880-ledger window.
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + 2_880 + 1);

    // Confirm staleness before flagging.
    assert!(
        oh_c.check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale,
        "keeper must be stale before flag_stale_keeper is called"
    );

    // Admin flags the stale keeper. This must succeed and emit KeeperHeartbeatMissed.
    oh_c.flag_stale_keeper(&w.admin, &w.keeper, &order_keeper_role);

    // Role is NOT automatically revoked by flag_stale_keeper; admin must revoke explicitly.
    assert!(
        rs_c.has_role(&w.keeper, &order_keeper_role),
        "role must still exist after flagging (revocation is separate)"
    );

    // Revoke the role — must succeed immediately (no timelock).
    rs_c.revoke_role(&w.admin, &w.keeper, &order_keeper_role);
    assert!(
        !rs_c.has_role(&w.keeper, &order_keeper_role),
        "role must be gone after explicit revocation"
    );
}

// ─── Test 5 ───────────────────────────────────────────────────────────────────

/// `flag_stale_keeper` must panic with `KeeperNotStale` when the keeper is
/// still within its heartbeat window.
#[test]
#[should_panic]
fn flag_stale_keeper_reverts_when_keeper_is_live() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    let order_keeper_role = roles::order_keeper(&w.env);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    // Execute at ledger 3 000.
    w.env.ledger().set_sequence_number(3_000);
    execute_order_at_current_ledger(&w);

    // Attempt to flag while still live (gap = 0) → must panic.
    oh_c.flag_stale_keeper(&w.admin, &w.keeper, &order_keeper_role);
}

// ─── Test 6 ───────────────────────────────────────────────────────────────────

/// A non-admin caller must be rejected by `flag_stale_keeper` (Unauthorized).
#[test]
#[should_panic]
fn non_admin_cannot_flag_stale_keeper() {
    let w = setup();

    set_prices(&w, 2_000);
    seed_pool(&w);
    set_prices(&w, 2_000);

    let order_keeper_role = roles::order_keeper(&w.env);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);

    // Execute and advance past the window.
    let exec_ledger: u32 = 500;
    w.env.ledger().set_sequence_number(exec_ledger);
    execute_order_at_current_ledger(&w);
    w.env
        .ledger()
        .set_sequence_number(exec_ledger + 2_880 + 1);

    assert!(
        oh_c.check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale,
        "keeper must be stale for the non-admin test to be meaningful"
    );

    // A random address (not the admin) must be rejected.
    let impostor = Address::generate(&w.env);
    oh_c.flag_stale_keeper(&impostor, &w.keeper, &order_keeper_role);
}
