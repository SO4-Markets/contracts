// Rollback atomicity tests for settlement and fee failures
// Validates that failed transactions don't leave partial state changes
// Issue #975: Build a snapshot/compare harness for rollback testing

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::roles;
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use rollback_harness::RollbackHarness;
use soroban_sdk::{testutils::Address as _, Address, Env, Map, Vec};

#[allow(dead_code)]
struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    user: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let user = Address::generate(&env);

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

    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX Market Token"),
        &soroban_sdk::String::from_str(&env, "GM"),
        &long_tk,
        &short_tk,
    );
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let dep_handler = env.register(DepositHandler, ());
    DepositHandlerClient::new(&env, &dep_handler).initialize(
        &admin,
        &rs,
        &ds,
        &oracle_addr,
        &dep_vault,
    );
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));

    let ord_handler = env.register(OrderHandler, ());
    OrderHandlerClient::new(&env, &ord_handler).initialize(
        &admin,
        &rs,
        &ds,
        &oracle_addr,
        &ord_vault,
    );
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    World {
        env,
        admin,
        keeper,
        user,
        ds,
        oracle: oracle_addr,
        ord_vault,
        dep_handler,
        ord_handler,
        market_tk,
        long_tk,
        short_tk,
    }
}

#[test]
fn snapshot_captures_protocol_state() {
    let world = setup();
    let balances: Map<Address, i128> = Map::new(&world.env);
    let fees: Map<Address, i128> = Map::new(&world.env);
    let liabilities: Map<Address, i128> = Map::new(&world.env);
    let orders: Vec<gmx_types::OrderProps> = Vec::new(&world.env);

    let snapshot =
        RollbackHarness::snapshot_protocol_state(&world.env, balances, fees, liabilities, orders);

    assert_eq!(world.env.ledger().sequence() as u64, snapshot.block_height);
}

#[test]
fn rollback_detects_state_changes() {
    let world = setup();

    let mut balances: Map<Address, i128> = Map::new(&world.env);
    balances.set(world.long_tk.clone(), 1000);

    let fees: Map<Address, i128> = Map::new(&world.env);
    let liabilities: Map<Address, i128> = Map::new(&world.env);
    let orders: Vec<gmx_types::OrderProps> = Vec::new(&world.env);

    let snapshot =
        RollbackHarness::snapshot_protocol_state(&world.env, balances, fees, liabilities, orders);

    // Mutate balances after snapshot
    let mut new_balances: Map<Address, i128> = Map::new(&world.env);
    new_balances.set(world.long_tk.clone(), 900);

    let new_fees: Map<Address, i128> = Map::new(&world.env);
    let new_liabilities: Map<Address, i128> = Map::new(&world.env);

    let validation = RollbackHarness::validate_atomic_rollback(
        &snapshot,
        &world.env,
        &new_balances,
        &new_fees,
        &new_liabilities,
    );

    assert!(!validation.is_valid(), "should detect balance change");
    assert!(
        !validation.balances_match,
        "balance_mismatch should be detected"
    );
}

#[test]
fn rollback_validates_full_revert() {
    let world = setup();

    let mut balances: Map<Address, i128> = Map::new(&world.env);
    balances.set(world.long_tk.clone(), 1000);

    let fees: Map<Address, i128> = Map::new(&world.env);
    let liabilities: Map<Address, i128> = Map::new(&world.env);
    let orders: Vec<gmx_types::OrderProps> = Vec::new(&world.env);

    let snapshot = RollbackHarness::snapshot_protocol_state(
        &world.env,
        balances.clone(),
        fees.clone(),
        liabilities.clone(),
        orders,
    );

    // Restore state exactly
    let validation = RollbackHarness::validate_atomic_rollback(
        &snapshot,
        &world.env,
        &balances,
        &fees,
        &liabilities,
    );

    assert!(validation.is_valid(), "should validate full revert");
    assert!(
        validation.state_fully_reverted,
        "state_fully_reverted should be true"
    );
}

#[test]
fn conservation_law_holds() {
    let world = setup();

    let movements = soroban_sdk::vec![
        &world.env,
        (world.user.clone(), 100i128),
        (world.admin.clone(), -100i128),
    ];

    let result = RollbackHarness::assert_conservation(&world.env, &movements);
    assert!(result.is_ok(), "movements should conserve");
}

#[test]
fn conservation_law_rejects_imbalanced() {
    let world = setup();

    let movements = soroban_sdk::vec![
        &world.env,
        (world.user.clone(), 100i128),
        (world.admin.clone(), -50i128),
    ];

    let result = RollbackHarness::assert_conservation(&world.env, &movements);
    assert!(result.is_err(), "should reject imbalanced movements");
}
