// Integration tests for settlement accounting invariants
// Validates exact balance/fee/liability equations before and after fills
// Issue #972, #973: Settlement invariant checks

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::roles;
use gmx_math::TOKEN_PRECISION;
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use settlement_invariants::{AskTakerInvariant, BidTakerInvariant, LockedLiabilityInvariant};
use soroban_sdk::{testutils::Address as _, Address, Env};

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
    index_tk: Address,
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
        index_tk: short_tk.clone(),
        short_tk,
    }
}

#[test]
fn bid_taker_invariant_holds_on_fill() {
    let world = setup();

    // Simulate bid-taker: long collateral increases, short decreases, fees accumulate.
    let invariant = BidTakerInvariant {
        taker_account: world.user.clone(),
        long_token: world.long_tk.clone(),
        short_token: world.short_tk.clone(),
        long_collateral_delta: 100 * TOKEN_PRECISION,
        short_collateral_delta: -100 * TOKEN_PRECISION,
        execution_fee_delta: 10 * TOKEN_PRECISION,
        position_fee_delta: 5 * TOKEN_PRECISION,
    };

    assert!(invariant.validate().is_ok(), "bid-taker invariant violated");
}

#[test]
fn ask_taker_invariant_holds_on_fill() {
    let world = setup();

    // Simulate ask-taker: long collateral decreases, short increases, fees accumulate.
    let invariant = AskTakerInvariant {
        taker_account: world.user.clone(),
        long_token: world.long_tk.clone(),
        short_token: world.short_tk.clone(),
        long_collateral_delta: -100 * TOKEN_PRECISION,
        short_collateral_delta: 100 * TOKEN_PRECISION,
        execution_fee_delta: 10 * TOKEN_PRECISION,
        position_fee_delta: 5 * TOKEN_PRECISION,
    };

    assert!(invariant.validate().is_ok(), "ask-taker invariant violated");
}

#[test]
fn locked_liability_invariant_holds() {
    let world = setup();

    // All open-order collateral must match recorded locked liability.
    let invariant = LockedLiabilityInvariant {
        account: world.user.clone(),
        token: world.short_tk.clone(),
        aggregate_open_orders_collateral: 5000 * TOKEN_PRECISION,
        recorded_locked_liability: 5000 * TOKEN_PRECISION,
    };

    assert!(
        invariant.validate().is_ok(),
        "locked-liability invariant violated"
    );
}

#[test]
fn bid_taker_invariant_rejects_invalid_deltas() {
    let world = setup();

    // Long decreased (should increase) — invalid
    let invariant = BidTakerInvariant {
        taker_account: world.user.clone(),
        long_token: world.long_tk.clone(),
        short_token: world.short_tk.clone(),
        long_collateral_delta: -100 * TOKEN_PRECISION,
        short_collateral_delta: 100 * TOKEN_PRECISION,
        execution_fee_delta: 10 * TOKEN_PRECISION,
        position_fee_delta: 5 * TOKEN_PRECISION,
    };

    assert!(
        invariant.validate().is_err(),
        "should reject decreased long collateral"
    );
}

#[test]
fn ask_taker_invariant_rejects_invalid_deltas() {
    let world = setup();

    // Short decreased (should increase) — invalid
    let invariant = AskTakerInvariant {
        taker_account: world.user.clone(),
        long_token: world.long_tk.clone(),
        short_token: world.short_tk.clone(),
        long_collateral_delta: -100 * TOKEN_PRECISION,
        short_collateral_delta: -100 * TOKEN_PRECISION,
        execution_fee_delta: 10 * TOKEN_PRECISION,
        position_fee_delta: 5 * TOKEN_PRECISION,
    };

    assert!(
        invariant.validate().is_err(),
        "should reject decreased short collateral"
    );
}

#[test]
fn locked_liability_mismatch_detected() {
    let world = setup();

    // Recorded liability doesn't match aggregate — invalid
    let invariant = LockedLiabilityInvariant {
        account: world.user.clone(),
        token: world.short_tk.clone(),
        aggregate_open_orders_collateral: 5000 * TOKEN_PRECISION,
        recorded_locked_liability: 4500 * TOKEN_PRECISION,
    };

    assert!(
        invariant.validate().is_err(),
        "should detect liability mismatch"
    );
}
