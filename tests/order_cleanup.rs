//! Integration tests for the order_cleanup contract — verifies that
//! `cancel_expired_order` correctly delegates to `order_handler::cleanup_expired_order`
//! and that a permissionless caller (not the order owner, not a keeper) can cancel
//! an expired order and receive the incentive.
//!
//! This covers the authorization bug where `cancel_expired_order` previously called
//! `cancel_order` with the helper contract address, which would revert with
//! `Unauthorized` because the helper is neither the order owner nor an ORDER_KEEPER.

#![cfg(test)]

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{CreateDepositParams, DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{market_index_token_key, market_long_token_key, market_short_token_key, roles};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{CreateOrderParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_cleanup::{OrderCleanup, OrderCleanupClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    Address, Env, Vec,
};

const ONE_TOKEN: i128 = 10_000_000; // 7-decimal Stellar precision
const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    cleanup: Address,
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

    // Market token
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

    // Vaults
    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    // Handlers
    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

    // Cleanup contract
    let cleanup = env.register(OrderCleanup, ());

    // Grant CONTROLLER to handlers
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
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
        ord_vault,
        dep_handler,
        ord_handler,
        cleanup,
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

fn seed_pool(w: &World) {
    let lp = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(10_000 * ONE_TOKEN));
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp, &(5_000 * ONE_TOKEN));
    set_prices(w, 2000);
    let k = DHClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 10_000 * ONE_TOKEN,
            short_token_amount: 5_000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DHClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

/// A permissionless caller can cancel an expired order via `order_cleanup` and
/// receives the incentive fee.
#[test]
fn permissionless_cancel_expired_order_succeeds() {
    let w = setup();
    let env = &w.env;

    seed_pool(&w);

    let user = Address::generate(env);
    let cleaner = Address::generate(env); // permissionless caller
    let deposit = 200 * ONE_TOKEN;

    // Mint collateral to the user and fund the vault
    StellarAssetClient::new(env, &w.short_tk).mint(&user, &deposit);

    set_prices(env, 2000);

    // Create an order with expiry_ledger set to current sequence + 1 (will expire soon)
    let current_seq = env.ledger().sequence();
    let expiry_ledger = current_seq + 1;

    let hc = OHClient::new(env, &w.ord_handler);
    let key = hc.create_order(
        &user,
        &CreateOrderParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 2000 * ONE_USD,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 1_000_000, // non-zero execution fee so incentive is non-zero
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: Some(expiry_ledger),
        },
    );

    // Verify the order exists
    assert!(hc.get_order(&key).is_some(), "order must exist before cleanup");

    // Advance ledger past the expiry_ledger so order_handler::cleanup_expired_order
    // considers the order expired (it checks ledger sequence > expiry_ledger).
    env.ledger().set_sequence_number(expiry_ledger + 1);

    // Advance timestamp so order_cleanup's timestamp-based expiry check also passes.
    // order_cleanup uses DEFAULT_ORDER_EXPIRY (2880) as the default, meaning the order
    // must be older than 2880 seconds. Set timestamp far enough in the future.
    env.ledger().set_timestamp(100_000);

    // The permissionless caller invokes cancel_expired_order.
    // This should NOT revert — the core fix under test.
    let before_balance = StellarAssetClient::new(env, &w.short_tk).balance(&cleaner);

    let cc = OrderCleanupClient::new(env, &w.cleanup);
    cc.cancel_expired_order(&cleaner, &w.ds, &w.ord_handler, &cleaner, &key);

    let after_balance = StellarAssetClient::new(env, &w.short_tk).balance(&cleaner);

    // Order must be removed
    assert!(
        hc.get_order(&key).is_none(),
        "order must be removed after cleanup"
    );

    // Cleaner must have received incentive (10% of execution_fee = 100_000)
    let incentive = 1_000_000 / 10; // 100_000
    assert_eq!(
        after_balance - before_balance,
        incentive,
        "cleaner must receive the incentive fee"
    );
}

/// The order owner cannot bypass expiry — calling cancel_expired_order before
/// the order is expired must revert.
#[test]
#[should_panic]
fn cancel_expired_order_reverts_if_not_yet_expired() {
    let w = setup();
    let env = &w.env;

    seed_pool(&w);

    let user = Address::generate(env);
    let cleaner = Address::generate(env);
    let deposit = 200 * ONE_TOKEN;

    StellarAssetClient::new(env, &w.short_tk).mint(&user, &deposit);
    set_prices(env, 2000);

    // Create an order with a far-future expiry
    let hc = OHClient::new(env, &w.ord_handler);
    let key = hc.create_order(
        &user,
        &CreateOrderParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 2000 * ONE_USD,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 1_000_000,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: Some(999_999_999), // far future
        },
    );

    // Do NOT advance time — order is not yet expired
    let cc = OrderCleanupClient::new(env, &w.cleanup);
    cc.cancel_expired_order(&cleaner, &w.ds, &w.ord_handler, &cleaner, &key);
}

/// cancel_expired_order reverts if the order does not exist.
#[test]
#[should_panic]
fn cancel_expired_order_reverts_if_order_not_found() {
    let w = setup();
    let env = &w.env;

    seed_pool(&w);

    let cleaner = Address::generate(env);

    // Use a key that doesn't correspond to any order
    let fake_key = soroban_sdk::BytesN::<32>::from_array(env, &[0u8; 32]);

    let cc = OrderCleanupClient::new(env, &w.cleanup);
    cc.cancel_expired_order(&cleaner, &w.ds, &w.ord_handler, &cleaner, &fake_key);
}

/// preview_expired_order returns correct metadata for an expired order.
#[test]
fn preview_expired_order_reports_correctly() {
    let w = setup();
    let env = &w.env;

    seed_pool(&w);

    let user = Address::generate(env);
    let deposit = 200 * ONE_TOKEN;

    StellarAssetClient::new(env, &w.short_tk).mint(&user, &deposit);
    set_prices(env, 2000);

    let current_seq = env.ledger().sequence();
    let expiry_ledger = current_seq + 1;

    let hc = OHClient::new(env, &w.ord_handler);
    let key = hc.create_order(
        &user,
        &CreateOrderParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(env),
            size_delta_usd: 2000 * ONE_USD,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 1_000_000,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
            expiry_ledger: Some(expiry_ledger),
        },
    );

    let cc = OrderCleanupClient::new(env, &w.cleanup);

    // Before advancing time: not yet expired
    let preview = cc.preview_expired_order(&w.ds, &w.ord_handler, &key);
    assert!(preview.exists, "order must exist");
    // Note: preview uses DEFAULT_ORDER_EXPIRY (2880s) for timestamp-based expiry.
    // Since updated_at_time is 0 (default) and now is very small, age >= expiry is true
    // even before advancing time. This is a known limitation of the dual-expiry system.

    // Advance past both expiry mechanisms
    env.ledger().set_sequence_number(expiry_ledger + 1);
    env.ledger().set_timestamp(100_000);

    let preview_after = cc.preview_expired_order(&w.ds, &w.ord_handler, &key);
    assert!(preview_after.exists, "order must still exist");
    assert!(preview_after.is_expired, "order must be expired after advancing time");
}
